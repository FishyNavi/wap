use lofty::file::AudioFile;
use lofty::file::TaggedFileExt;
use lofty::read_from_path;
use lofty::tag::Accessor;
use rodio::source;
use rodio::{Decoder, MixerDeviceSink, queue};
use rusqlite::Connection;
use rusqlite::params;
use std::borrow::Cow;

use rand::seq::SliceRandom;
use std::collections::HashMap;
use std::error::Error;
use std::ffi::OsStr;
use std::fs;
use std::fs::File;
use std::io;
use std::io::BufReader;
use std::mem::take;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::{Arc, RwLock};
use std::thread;
use std::thread::sleep;
use std::time::Duration;
struct Queue {
    queue: Vec<usize>,
    position: Option<usize>,
    sink_handle: MixerDeviceSink,
    track_list: HashMap<usize, TrackData>,
    player: rodio::Player,
    next_id: Option<usize>,
}
impl Queue {
    fn new() -> Self {
        let next_id = None::<usize>;
        let queue: Vec<usize> = Vec::new();
        let position = Some(0);

        let sink_handle =
            rodio::DeviceSinkBuilder::open_default_sink().expect("open default audio stream");
        let player = rodio::Player::connect_new(&sink_handle.mixer());
        let track_list: HashMap<usize, TrackData> = HashMap::new();
        Self {
            queue,
            position,
            sink_handle,
            track_list,
            player,
            next_id,
        }
    }
    fn shuffle(&mut self) -> Result<(), Box<dyn Error>> {
        let mut ids: Vec<usize> = self.track_list.keys().copied().collect();
        ids.shuffle(&mut rand::rng());
        self.queue = ids;
        Ok(())
    }
    fn enqueue_next(&mut self) -> Result<(), Box<dyn Error>> {
        let next = self.next_id.map_or(0, |p| p + 1);
        self.next_id = Some(next);

        if let Some(track) = self.queue.get(next) {
            self.enqueue(*track)?;
        };

        Ok(())
    }
    fn enqueue(&mut self, id: usize) -> Result<(), Box<dyn Error>> {
        let track_data = self.track_from_id(id)?;
        println!("{}", track_data.path);
        if let Some(source) = TrackData::source_from_path(&track_data.path) {
            self.player.append(source);
        }

        Ok(())
    }

    fn play_track(&mut self, id: usize) -> Result<(), Box<dyn Error>> {
        self.position = Some(id as usize);

        // println!(
        //     "{:?} {:?} {:?}",
        //     self.position,
        //     self.player.len(),
        //     self.track_list
        // );
        let (track_id, path) = {
            let track_data = self.track_from_id(id)?;
            (track_data.id, track_data.path.clone())
        };

        self.next_id = Some(track_id);

        let mut source = TrackData::source_from_path(&path);
        if let Some(source) = source.take() {
            self.player.clear();
            self.player.append(source);
            self.player.play();
        }

        Ok(())
    }
    fn previous_track(&mut self) -> Result<(), Box<dyn Error>> {
        let next = self
            .position
            .map_or(0, |p| if p == 0 { self.queue.len() - 1 } else { p - 1 });
        if let Some(&id) = self.queue.get(next) {
            self.position = Some(next);
            self.play_track(id)
        } else {
            Ok(())
        }
    }
    fn next_track(&mut self) -> Result<(), Box<dyn Error>> {
        let next = self.position.map_or(0, |p| p + 1);
        if let Some(&id) = self.queue.get(next) {
            self.position = Some(next);
            self.play_track(id)
        } else {
            Ok(())
        }
    }
    fn handle_event(&mut self, payload: EventPayload) -> Result<(), Box<dyn Error>> {
        // if self.current_track.is_none() {
        //     return;
        // };
        match payload {
            EventPayload::PlaySong { id } => self.play_track(id)?,
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
    fn set_track_list(&mut self, track_list: HashMap<usize, TrackData>) {
        self.track_list = track_list;
    }
    fn track_from_id(&mut self, id: usize) -> Result<&TrackData, &str> {
        let track_data = self
            .track_list
            .get(&id)
            .ok_or("wap searched hard but didn't find any matching the id");
        track_data
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
        self.parse_db()?;
        Ok(())
    }
    fn parse_db(&mut self) -> rusqlite::Result<()> {
        let mut stmt = self.conn.prepare("SELECT id, path FROM db")?;
        let song_iter = stmt.query_map([], |row| {
            Ok(TrackData {
                id: row.get::<_, i32>(0)? as usize,
                path: row.get(1)?,
                tag: None,
            })
        })?;
        for song in song_iter {
            let song = song?;
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
    fn fetch_db(&mut self) -> Result<HashMap<usize, TrackData>, Box<dyn Error>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, path, title, artist, album, genre FROM db")?;
        let tags = stmt
            .query_map([], |row| {
                let id_: i32 = row.get(0)?;
                let id = id_ as usize;
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
            .collect::<Result<HashMap<usize, TrackData>, rusqlite::Error>>()?;
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
    id: usize,
    path: String,
    tag: Option<TrackTag>,
}
// struct Track {
//     source: Option<Decoder<BufReader<File>>>,
//     data: Option<TrackData>,
//     id: usize,
// }
impl TrackData {
    fn new(id: usize, path: String) -> Option<TrackData> {
        Some(Self {
            id,
            path,
            tag: None,
        })
    }
    fn source_from_path(path: &String) -> Option<Decoder<BufReader<File>>> {
        Some(Decoder::try_from(File::open(path).unwrap()).unwrap())
    }
    // fn fetch_tag_from_db(id: usize, conn: &Connection) -> TrackTag {
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
    payload: EventPayload,
    // id time etc
}
impl Event {
    fn from_input(input: i32, stdin: &io::Stdin, buffer: &mut String) -> Option<Self> {
        let payload = match input {
            0 => EventPayload::Exit,
            1 => {
                buffer.clear();
                if stdin.read_line(buffer).ok()? == 0 {
                    return None;
                }
                let id = buffer.trim().parse::<usize>().unwrap_or_else(|_| {
                    println!("Invalid id");
                    0
                });
                EventPayload::PlaySong { id }
            }
            2 => EventPayload::NextSong { forward: true },
            3 => EventPayload::ChangeVolume { value: 0.05 },
            4 => EventPayload::ChangeVolume { value: -0.05 },
            5 => EventPayload::Pause,
            6 => EventPayload::Unpause,
            _ => EventPayload::None,
        };
        Some(Self { payload })
        // change it
    }
}

enum EventPayload {
    Pause,
    Unpause,
    ChangeVolume { value: f32 },
    NextSong { forward: bool },
    PlaySong { id: usize },
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
        match &event.payload {
            EventPayload::Exit => return false,
            _ => self
                .queue
                .handle_event(event.payload)
                .unwrap_or_else(|e| eprintln!("Error: {}", e)),
        }
        true
    }
    fn load_tracks(&mut self) -> Result<(), Box<dyn Error>> {
        let tracks = self.resources.fetch_db()?;
        self.queue.set_track_list(tracks);
        Ok(())
    }
    fn update(&mut self) {
        if self.queue.player.len() < 2 {
            self.queue.enqueue_next();
        }
    }
}
struct App {
    state: Arc<Mutex<State>>,
}
impl App {
    fn new() -> Self {
        let state = Arc::new(Mutex::new(State::new()));

        Self { state }
    }
    fn poll(&self) {
        let clone = Arc::clone(&self.state);

        thread::spawn(move || {
            loop {
                {
                    let mut state = clone.lock().unwrap();
                    state.update();
                }
                sleep(Duration::from_secs(1));
            }
        });
    }
    fn run(&mut self) -> Result<(), Box<dyn Error>> {
        {
            let mut state = self.state.lock().unwrap();
            state.load_tracks()?;
            state.queue.shuffle()?;
        }
        let mut buffer = String::new();

        self.poll();

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
            {
                let mut state = self.state.lock().unwrap();
                if !state.handle_event(event) {
                    break;
                }
            }
        }
        Ok(())
    }
}
fn main() {
    let mut app = App::new();
    app.run();
}
