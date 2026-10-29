use lofty::file::AudioFile;
use lofty::file::TaggedFileExt;
use lofty::read_from_path;
use lofty::tag;
use lofty::tag::Accessor;
use rodio::{Decoder, MixerDeviceSink, queue};
use rusqlite::Connection;
use rusqlite::params;
use std::borrow::Cow;
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
    queue: Arc<RwLock<Vec<i16>>>,
    sink_handle: MixerDeviceSink,
    songs: Vec<Track>,
    player: rodio::Player,
    current_track: TrackData,
    next_track: Option<TrackData>,
    previous_track: Option<TrackData>,
}
impl Queue {
    fn new() -> Self {
        let current_track = TrackData::new(1).unwrap();
        let queue: Arc<RwLock<Vec<i16>>> = Arc::new(RwLock::new(Vec::new()));

        let sink_handle =
            rodio::DeviceSinkBuilder::open_default_sink().expect("open default audio stream");
        let player = rodio::Player::connect_new(&sink_handle.mixer());
        let songs: Vec<Track> = vec![];
        Self {
            queue,
            sink_handle,
            songs,
            player,
            current_track,
            next_track: None,
            previous_track: None,
        }
    }
    fn play(&mut self) {
        if let Some(source) = self.current_track.source.take() {
            self.player.append(source);
        }
        // self.player.append(self.current_track.source);
    }
    fn handle_event(&mut self, payload: EventPayload) {
        match payload {
            EventPayload::PlaySong { id } => self.play(),
            EventPayload::ChangeVolume { value } => {
                self.player
                    .set_volume((self.player.volume() + value).min(1.0).max(0.0));
            }
            EventPayload::Pause => self.player.pause(),
            EventPayload::Unpause => self.player.play(),

            _ => {}
        }
    }
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
            Ok(Track {
                id: row.get(0)?,
                path: row.get(1)?,
                tags: None,
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
}
#[derive(Debug, Clone)]
struct Track {
    id: i32,
    path: String,
    tags: Option<i32>,
}
struct TrackData {
    name: String,
    source: Option<Decoder<BufReader<File>>>,
    id: i16,
}
impl TrackData {
    fn new(id: i16) -> Option<TrackData> {
        let name = "test".to_string();
        let path = "song.mp3";
        let source = Some(Decoder::try_from(File::open(path).unwrap()).unwrap());
        Some(Self {
            name: name,
            source,
            id,
        })
    }
}

struct Event {
    queue_event: bool,
    payload: EventPayload,
    // id time etc
}
impl Event {
    fn from_input(input: i32) -> Self {
        let (queue_event, payload) = match input {
            0 => (false, EventPayload::Exit),
            1 => (true, EventPayload::PlaySong { id: 123 }),
            2 => (true, EventPayload::NextSong { next: true }),
            3 => (true, EventPayload::ChangeVolume { value: 0.05 }),
            4 => (true, EventPayload::ChangeVolume { value: -0.05 }),
            5 => (true, EventPayload::Pause),
            6 => (true, EventPayload::Unpause),
            _ => (false, EventPayload::None),
        };
        Self {
            queue_event,
            payload,
        }
    }
}

enum EventPayload {
    Pause,
    Unpause,
    ChangeVolume { value: f32 },
    NextSong { next: bool },
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
        eprintln!("{:?}", e);
        let paused = false;
        Self {
            queue,
            resources,
            paused,
        }
    }
    fn handle_event(&mut self, event: Event) -> bool {
        match (&event.queue_event, &event.payload) {
            (true, _) => self.queue.handle_event(event.payload),
            (false, EventPayload::Exit) => return false,
            (_, _) => {}
        }
        true
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
    fn run(&mut self) {
        loop {
            let mut input = String::new();
            io::stdin()
                .read_line(&mut input)
                .expect("Failed to read line");
            let input = input.trim().parse::<i32>().unwrap_or(0);
            let event = Event::from_input(input);
            if !self.state.handle_event(event) {
                break;
            }
        }
    }
}
fn main() {
    let mut app = App::new();
    app.run();
}
