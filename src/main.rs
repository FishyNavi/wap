use lofty::file::AudioFile;
use lofty::file::TaggedFileExt;
use lofty::read_from_path;
use lofty::tag::Accessor;
use rodio::{Decoder, MixerDeviceSink, queue};
use rusqlite::Connection;
use rusqlite::params;
use std::borrow::Cow;

use std::collections::HashMap;
use std::error::Error;
use std::ffi::OsStr;
use std::fs;
use std::fs::File;
use std::io;
use std::io::BufReader;

use std::path::Path;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
struct Queue {
    queue: Arc<Vec<Track>>,
    sink_handle: MixerDeviceSink,
    track_list: HashMap<i32, TrackData>,
    player: rodio::Player,
    current_track: Option<Track>,
    next_track: Option<Track>,
    previous_track: Option<Track>,
}
impl Queue {
    fn new() -> Self {
        // let current_track = Track::new(1, None).unwrap();
        let current_track = None;
        let queue: Arc<Vec<Track>> = Arc::new(Vec::new());

        let sink_handle =
            rodio::DeviceSinkBuilder::open_default_sink().expect("open default audio stream");
        let player = rodio::Player::connect_new(&sink_handle.mixer());
        let track_list: HashMap<i32, TrackData> = HashMap::new();
        Self {
            queue,
            sink_handle,
            track_list,
            player,
            current_track,
            next_track: None,
            previous_track: None,
        }
    }
    fn play(&mut self, id: i32) -> Result<(), Box<dyn Error>> {
        // if let Some(track) = self.current_track.as_mut() {
        //     if track.id == id {
        //         self.player.clear();
        //     } else {
        //         let track = self.track_from_id(id)?;
        //         self.current_track = Some(track);
        //     }
        // } else {
        //     let track = self.track_from_id(id)?;
        //     self.current_track = Some(track);
        // }
        if self.current_track.is_some() {
            println!(
                "id {} track id {}",
                id,
                self.current_track.as_ref().unwrap().id
            );
        }

        let track = self.track_from_id(id)?;
        self.current_track = Some(track);

        if let Some(track) = self.current_track.as_mut() {
            if let Some(source) = track.source.take() {
                self.player.clear();
                self.player.append(source);
                self.player.play();
            }
        }

        Ok(())
        // self.player.append(self.current_track.source);
    }
    fn handle_event(&mut self, payload: EventPayload) -> Result<(), Box<dyn Error>> {
        // if self.current_track.is_none() {
        //     return;
        // };
        match payload {
            EventPayload::PlaySong { id } => self.play(id)?,
            EventPayload::ChangeVolume { value } => {
                self.player
                    .set_volume((self.player.volume() + value).min(1.0).max(0.0));
            }
            EventPayload::Pause => self.player.pause(),
            EventPayload::Unpause => self.player.play(),

            _ => {}
        }
        Ok(())
    }
    fn set_track_list(&mut self, track_list: HashMap<i32, TrackData>) {
        self.track_list = track_list;
        println!("{:?}", self.track_list);
    }
    fn track_from_id(&mut self, id: i32) -> Result<Track, &str> {
        let track_data = self
            .track_list
            .get(&id)
            .map(|data| {
                let source = Track::source_from_path(&data.path);
                Track {
                    id,
                    source,
                    data: Some(data.clone()),
                }
            })
            .ok_or("wap searched hard but didn't find any matching the id");
        track_data
    }
    // fn queue_track(track: ) {

    // }
}
struct Resources {
    conn: Connection,
}
impl Resources {
    fn new() -> Self {
        let conn = Connection::open_in_memory().expect("");
        Self { conn }
    }
    fn ext(path: &PathBuf) -> Option<&str> {
        path.extension().and_then(OsStr::to_str)
    }
    fn check_dir(&mut self, dir: &Path) -> Result<(), Box<dyn Error>> {
        // fix this maybe
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            if entry.path().is_dir() {
                Self::check_dir(self, &entry.path())?;
            } else {
                let path = entry.path();

                if matches!(Self::ext(&path), Some("mp3")) {
                    self.conn.execute(
                        "INSERT INTO db (path) VALUES (?1)",
                        params![path.display().to_string()],
                    )?;
                }
            }
        }
        Ok(())
    }
    fn index(&mut self, dir: &Path) -> Result<(), Box<dyn Error>> {
        self.conn.execute(
            "
            CREATE TABLE IF NOT EXISTS db (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                path TEXT NOT NULL,
                title TEXT,
                artist TEXT,
                album TEXT,
                genre TEXT
            )
            ",
            [],
        )?;

        self.check_dir(dir)?;
        println!("Populated the db with paths");
        self.parse_db()?;
        println!("Db parsed!");
        Ok(())
    }
    fn parse_db(&mut self) -> rusqlite::Result<()> {
        let mut stmt = self.conn.prepare("SELECT id, path FROM db")?;
        let song_iter = stmt.query_map([], |row| {
            Ok(TrackData {
                id: row.get(0)?,
                path: row.get(1)?,
                tag: None,
            })
        })?;
        for song in song_iter {
            let song = song?;
            println!("Found song {:?}", song);
            let path: String = song.path;

            match read_from_path(&path) {
                Ok(tagged_file) => {
                    // for item in tagged_file.primary_tag().unwrap().items() {
                    //     println!("{:?}", item)
                    // }
                    let duration = tagged_file.properties().duration();
                    let bitrate = tagged_file.properties().audio_bitrate();
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
                        self.conn.execute(
                            "UPDATE db SET title = ?1, artist = ?2, album = ?3, genre = ?4 WHERE path = ?5 ",
                            params![title, artist, album, genre, &path],
                        )?;
                        // TODO: add more tags

                        // println!("{} {} {} {}", title, artist, album, genre)
                    }
                }
                Err(e) => {}
            }
        }

        Ok(())
    }
    fn fetch_db(&mut self) -> Result<HashMap<i32, TrackData>, Box<dyn Error>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, path, title, artist, album, genre FROM db")?;
        let tags = stmt
            .query_map([], |row| {
                let id = row.get(0)?;
                Ok((
                    id,
                    TrackData {
                        id: id,
                        path: row.get(1)?,
                        tag: Some(TrackTag {
                            title: row.get(2)?,
                            artist: row.get(3)?,
                            album: row.get(4)?,
                            genre: row.get(5)?,
                        }),
                    },
                ))
            })
            .expect("No tag")
            .collect::<Result<HashMap<i32, TrackData>, rusqlite::Error>>()?;
        Ok(tags)
    }
}
#[derive(Debug, Clone)]
struct TrackTag {
    title: String,
    artist: String,
    album: String,
    genre: String,
}
impl TrackTag {
    fn new(title: String, artist: String, album: String, genre: String) -> Self {
        Self {
            title,
            artist,
            album,
            genre,
        }
    }
}
#[derive(Debug, Clone)]
struct TrackData {
    id: i32,
    path: String,
    tag: Option<TrackTag>,
}
struct Track {
    source: Option<Decoder<BufReader<File>>>,
    data: Option<TrackData>,
    id: i32,
}
impl Track {
    fn new(id: i32, data: TrackData) -> Option<Track> {
        let source = Some(Decoder::try_from(File::open(data.path).unwrap()).unwrap());
        Some(Self {
            source,
            id,
            data: None,
        })
    }
    fn source_from_path(path: &String) -> Option<Decoder<BufReader<File>>> {
        Some(Decoder::try_from(File::open(path).unwrap()).unwrap())
    }
    // fn fetch_tag_from_db(id: i32, conn: &Connection) -> TrackTag {
    //     let track_tag = conn.query_row(
    //         "
    //             SELECT title, artist, album, genre FROM db WHERE id = ?1
    //         ",
    //         [id],
    //         |row| {
    //             Ok(TrackTag {
    //                 title: row.get(0)?,
    //                 artist: row.get(1)?,
    //                 album: row.get(2)?,
    //                 genre: row.get(3)?,
    //             })
    //         },
    //     );
    //     track_tag.expect("No tag")
    // }
}

struct Event {
    queue_event: bool,
    payload: EventPayload,
    // id time etc
}
impl Event {
    fn from_input(input: i32, stdin: &io::Stdin, buffer: &mut String) -> Option<Self> {
        let (queue_event, payload) = match input {
            0 => (false, EventPayload::Exit),
            1 => {
                buffer.clear();
                if stdin.read_line(buffer).ok()? == 0 {
                    return None;
                }
                let id = buffer.trim().parse::<i32>().unwrap_or_else(|_| {
                    println!("Invalid id");
                    0
                });
                println!("meow");
                (true, EventPayload::PlaySong { id })
            }
            2 => (true, EventPayload::NextSong { forward: true }),
            3 => (true, EventPayload::ChangeVolume { value: 0.05 }),
            4 => (true, EventPayload::ChangeVolume { value: -0.05 }),
            5 => (true, EventPayload::Pause),
            6 => (true, EventPayload::Unpause),
            _ => (false, EventPayload::None),
        };
        Some(Self {
            queue_event,
            payload,
        })
        // change it
    }
}

enum EventPayload {
    Pause,
    Unpause,
    ChangeVolume { value: f32 },
    NextSong { forward: bool },
    PlaySong { id: i32 },
    Exit,
    None, // temp ig
}
struct State {
    queue: Queue,
    resources: Resources,
    paused: bool,
}
impl State {
    fn new() -> Self {
        let queue = Queue::new();
        let mut resources = Resources::new();
        let e = resources.index(Path::new("."));
        // eprintln!("{:?}", e);

        let paused = false;
        Self {
            queue,
            resources,
            paused,
        }
    }
    fn handle_event(&mut self, event: Event) -> bool {
        match (&event.queue_event, &event.payload) {
            (true, _) => self
                .queue
                .handle_event(event.payload)
                .unwrap_or_else(|e| eprintln!("Error: {}", e)),
            (false, EventPayload::Exit) => return false,
            (_, _) => {}
        }
        true
    }
    fn load_tracks(&mut self) -> Result<(), Box<dyn Error>> {
        let tracks = self.resources.fetch_db()?;
        self.queue.set_track_list(tracks);
        Ok(())
    }
}
struct App {
    state: State,
}
impl App {
    fn new() -> Self {
        let state = State::new();

        Self { state }
    }
    fn run(&mut self) -> Result<(), Box<dyn Error>> {
        self.state.load_tracks()?;
        let mut buffer = String::new();
        loop {
            buffer.clear();
            io::stdin()
                .read_line(&mut buffer)
                .expect("Failed to read line");
            let input = buffer.trim().parse::<i32>().unwrap_or(0);
            let event = match Event::from_input(input, &io::stdin(), &mut buffer) {
                Some(evnt) => evnt,
                None => continue,
            };
            if !self.state.handle_event(event) {
                break;
            }
        }
        Ok(())
    }
}
fn main() {
    let mut app = App::new();
    app.run();
}
