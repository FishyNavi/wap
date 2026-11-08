use lofty::file::{AudioFile, TaggedFileExt};
use lofty::read_from_path;
use lofty::tag::Accessor;
use rusqlite::{Connection, params};
use std::borrow::Cow;
use std::collections::HashMap;
use std::error::Error;
use std::ffi::OsStr;
use std::fs;
use std::path::Path;
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
        Ok(Self {
            conn: Connection::open_in_memory()?,
            skipped: 0,
        })
    }
    // fn ext(path: &PathBuf) -> Option<&str> {
    //     path.extension().and_then(OsStr::to_str)
    // }

    fn is_audio(path: &Path) -> bool {
        path.extension()
            .and_then(OsStr::to_str)
            .is_some_and(|e| AUDIO_EXTENSIONS.iter().any(|x| e.eq_ignore_ascii_case(x)))
    }

    fn check_dir(&mut self, dir: &Path) -> Result<(), Box<dyn Error>> {
        // fix this maybe. delete entries with path outside of scanned dir
        for entry in fs::read_dir(dir)? {
            let Ok(entry) = entry else {
                self.skipped += 1;
                continue;
            };
            let path = entry.path();
            if entry.file_type().is_ok_and(|t| t.is_dir()) {
                if self.check_dir(&entry.path()).is_err() {
                    self.skipped += 1;
                };
            } else if Self::is_audio(&path) {
                self.conn.execute(
                    "INSERT OR IGNORE INTO db (path) VALUES (?1)",
                    params![path.display().to_string()],
                )?;
            }
        }
        Ok(())
    }
    pub fn index(&mut self, path: &Path) -> Result<(), Box<dyn Error>> {
        let dir = fs::canonicalize(path)?;
        self.conn.execute(
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

        self.conn.execute_batch("BEGIN")?;
        let scanned = self.check_dir(dir.as_path());
        scanned.and(self.conn.execute_batch("COMMIT").map_err(Into::into));
        self.parse_db()?;
        Ok(())
    }
    fn parse_db(&mut self) -> rusqlite::Result<()> {
        let rows: Vec<(i64, String, Option<i64>)> = {
            let mut stmt = self.conn.prepare("SELECT id, path, modified FROM db")?;
            stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
                .collect::<Result<_, _>>()?
        };

        let db_tx = self.conn.transaction()?;
        for (id, path, modified) in rows {
            let current = match fs::metadata(&path).and_then(|m| m.modified()) {
                Ok(t) => t
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs() as i64,
                Err(_) => {
                    db_tx.execute("DELETE FROM db WHERE id = ?1", params![id])?;
                    continue;
                }
            };
            if modified == Some(current) {
                continue;
            }
            match read_from_path(&path) {
                Ok(tagged_file) => {
                    // for item in tagged_file.primary_tag().unwrap().items() {
                    //     println!("{:?}", item)
                    // }
                    let duration = tagged_file.properties().duration().as_millis() as i64; // handle 0 durations
                    db_tx.execute(
                        "UPDATE db SET duration = ?1 WHERE path = ?2",
                        params![duration, path],
                    )?;

                    // let bitrate = tagged_file.properties().audio_bitrate();
                    if let Some(tag) = tagged_file.primary_tag() {
                        let title = tag
                            .title()
                            .unwrap_or_else(|| Cow::Borrowed("Unknown Title"));
                        let artist = tag
                            .artist()
                            .unwrap_or_else(|| Cow::Borrowed("Unknown Artist"));
                        let album = tag
                            .album()
                            .unwrap_or_else(|| Cow::Borrowed("Unknown Album"));
                        let genre = tag
                            .genre()
                            .unwrap_or_else(|| Cow::Borrowed("Unknown Genre"));
                        db_tx.execute(
                            "UPDATE db SET duration = ?1, title = ?2, artist = ?3, album = ?4, genre = ?5 WHERE path = ?6 ",
                            params![duration, title, artist, album, genre, &path],
                        )?;
                        // TODO: add more tags

                        // println!("{} {} {} {}", title, artist, album, genre)
                    }
                }
                Err(_) => {
                    self.skipped += 1;
                }
            }
        }
        db_tx.commit()?;
        Ok(())
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
