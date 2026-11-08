use lofty::file::{AudioFile, TaggedFileExt};
use lofty::read_from_path;
use lofty::tag::Accessor;
use rusqlite::{Connection, params};
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::types::{TrackData, TrackTag};
pub struct Resources {
    pub conn: Connection,
    pub skipped: usize,
}
const AUDIO_EXTENSIONS: &[&str] = &["mp3", "flac", "ogg", "oga", "opus", "wav", "m4a", "aac"];
impl Resources {
    pub fn new(db_path: &Path) -> Result<Self, Box<dyn Error>> {
        // Ok(Self {
        //     conn: Connection::open(db_path)?,
        //     skipped: 0,
        // })
        let conn = Connection::open_in_memory()?;

        conn.execute(
            "
            CREATE TABLE IF NOT EXISTS db (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                path TEXT NOT NULL UNIQUE,
                duration INTEGER,
                title TEXT,
                artist TEXT,
                album TEXT,
                genre TEXT,
                modified INTEGER
            )
            ",
            [],
        )?;
        Ok(Self { conn, skipped: 0 })
    }
    // fn ext(path: &PathBuf) -> Option<&str> {
    //     path.extension().and_then(OsStr::to_str)
    // }

    fn is_audio(path: &Path) -> bool {
        path.extension()
            .and_then(OsStr::to_str)
            .is_some_and(|e| AUDIO_EXTENSIONS.iter().any(|x| e.eq_ignore_ascii_case(x)))
    }
    fn check_dir(dir: &Path, out: &mut Vec<PathBuf>, skipped: &mut usize) {
        let Ok(entries) = fs::read_dir(dir) else {
            *skipped += 1;
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            match entry.file_type() {
                Ok(t) if t.is_dir() => Self::check_dir(&path, out, skipped),
                Ok(_) if Resources::is_audio(&path) => out.push(path),
                _ => {}
            }
        }
    }
    // fn check_dir(&mut self, dir: &Path) {
    //     // fix this maybe. delete entries with path outside of scanned dir
    //     for entry in fs::read_dir(dir)? {
    //         let Ok(entry) = entry else {
    //             self.skipped += 1;
    //             continue;
    //         };
    //         let path = entry.path();
    //         if entry.file_type().is_ok_and(|t| t.is_dir()) {
    //             if self.check_dir(&entry.path()) {
    //                 self.skipped += 1;
    //             };
    //         } else if Self::is_audio(&path) {
    //             self.conn.execute(
    //                 "INSERT OR IGNORE INTO db (path) VALUES (?1)",
    //                 params![path.display().to_string()],
    //             );
    //         }
    //     }
    // }
    pub fn index(&mut self, path: &Path) -> Result<(), Box<dyn Error>> {
        let dir = fs::canonicalize(path)?;

        let mut files = Vec::new();
        let mut skipped = 0;
        Self::check_dir(dir.as_path(), &mut files, &mut skipped);
        let known = Self::get_known(&self.conn)?;
        let mut seen = HashSet::new();
        let db_tx = self.conn.transaction()?;
        for p in &files {
            let path = p.display().to_string();
            seen.insert(path.clone());

            let Ok(mtime) = fs::metadata(&path).and_then(|m| m.modified()) else {
                skipped += 1;
                continue;
            };
            let mtime = mtime
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64;
            if known.get(&path).and_then(|k| k.1) == Some(mtime) {
                continue;
            }
            match read_from_path(&path) {
                Ok(file) => {
                    let duration = file.properties().duration().as_millis() as i64;
                    let tag = file.primary_tag();
                    let title = tag
                        .and_then(|t| t.title())
                        .unwrap_or(Cow::Borrowed("Unknown Title"));
                    let artist = tag
                        .and_then(|t| t.artist())
                        .unwrap_or(Cow::Borrowed("Unknown Artist"));
                    let album = tag
                        .and_then(|t| t.album())
                        .unwrap_or(Cow::Borrowed("Unknown Album"));
                    let genre = tag
                        .and_then(|t| t.genre())
                        .unwrap_or(Cow::Borrowed("Unknown Genre"));
                    db_tx.execute(
                        "INSERT INTO db (path, duration, title, artist, album, genre, modified)
                            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                            ON CONFLICT(path) DO UPDATE SET
                                duration = excluded.duration, title = excluded.title,
                                artist = excluded.artist, album = excluded.album,
                                genre = excluded.genre, modified = excluded.modified",
                        params![path, duration, title, artist, album, genre, mtime],
                    )?;
                }
                Err(_) => skipped += 1,
            }
        }
        for (path, (id, _)) in &known {
            if Path::new(path).starts_with(&dir) && !seen.contains(path) {
                db_tx.execute("DELETE FROM db WHERE id = ?1", params![id])?;
            }
        }

        db_tx.commit()?;
        self.skipped += skipped;
        Ok(())
    }
    fn get_known(conn: &Connection) -> rusqlite::Result<HashMap<String, (i64, Option<i64>)>> {
        let mut stmt = conn.prepare("SELECT path, id, modified FROM db")?;
        stmt.query_map([], |r| Ok((r.get(0)?, (r.get(1)?, r.get(2)?))))?
            .collect::<Result<_, _>>()
    }
    pub fn fetch_db(&mut self) -> Result<HashMap<usize, TrackData>, Box<dyn Error>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, path, duration, COALESCE(title,'Unknown Title'), COALESCE(artist,'Unknown Artist'), COALESCE(album,'Unknown Album'), COALESCE(genre,'Unknown Genre') FROM db")?;
        let tags = stmt
            .query_map([], |row| {
                let id_: i64 = row.get(0)?;
                let id = id_ as usize;
                let duration: Option<i64> = row.get(2)?;
                Ok((
                    id,
                    TrackData {
                        id: id,
                        path: row.get(1)?,
                        duration: duration.map(|ms| Duration::from_millis(ms as u64)),
                        tag: Some(TrackTag {
                            title: row.get(3)?,
                            artist: row.get(4)?,
                            album: row.get(5)?,
                            genre: row.get(6)?,
                        }),
                    },
                ))
            })?
            .collect::<Result<HashMap<usize, TrackData>, rusqlite::Error>>()?;
        Ok(tags)
    }
}
