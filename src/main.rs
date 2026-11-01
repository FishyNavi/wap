use lofty::file::AudioFile;
use lofty::file::TaggedFileExt;
use lofty::id3::v2::ChannelType::BackLeft;
use lofty::mpeg::Emphasis;
use lofty::read_from_path;
use lofty::tag::Accessor;
use rand::seq::SliceRandom;
use rodio::{Decoder, MixerDeviceSink};
use rusqlite::Connection;
use rusqlite::params;
use std::borrow::Cow;
use std::collections::HashMap;
use std::collections::VecDeque;
use std::error::Error;
use std::ffi::OsStr;
use std::fs;
use std::fs::File;
use std::io;
use std::io::BufReader;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::thread;
use std::thread::sleep;
use std::time::Duration;
#[derive(PartialEq)]
enum QueueModes {
    OnRepeat,
    Shuffle,
}

struct Queue {
    queue: Vec<usize>,
    in_player: VecDeque<usize>,
    #[allow(unused)]
    sink_handle: MixerDeviceSink,
    track_list: HashMap<usize, TrackData>,
    player: rodio::Player,
    next_id: usize,
    queue_mode: QueueModes,
}
impl Queue {
    fn new() -> Self {
        let next_id = 0;
        let queue: Vec<usize> = Vec::new();
        let in_player: VecDeque<usize> = VecDeque::new();

        let sink_handle =
            rodio::DeviceSinkBuilder::open_default_sink().expect("open default audio stream");
        let player = rodio::Player::connect_new(&sink_handle.mixer());
        player.pause();
        let track_list: HashMap<usize, TrackData> = HashMap::new();
        Self {
            queue,
            in_player,
            sink_handle,
            track_list,
            player,
            next_id,
            queue_mode: QueueModes::Shuffle,
        }
    }
    fn open(&self, track_id: usize) -> Result<Decoder<BufReader<File>>, Box<dyn Error>> {
        let data = self
            .track_list
            .get(&(track_id))
            .ok_or_else(|| format!("no track with id {track_id}"))?;
        let file = File::open(&data.path).map_err(|e| format!("{}: {e}", data.path))?;
        Ok(Decoder::try_from(file)?)
    }
    fn get_track_data(&self) -> Result<TrackData, Box<dyn Error>> {
        Ok(self
            .track_list
            .get(self.in_player.front().ok_or("id not found")?)
            .cloned()
            .ok_or("not such id")?)
    }
    fn shuffle(&mut self) {
        let mut ids: Vec<usize> = self.track_list.keys().copied().collect();
        ids.shuffle(&mut rand::rng());
        self.queue = ids;
        self.next_id = self
            .in_player
            .back()
            .and_then(|&t| self.index_of(t))
            .map_or(0, |i| i + 1);
    }

    // fn get_next_id(&mut self) -> usize {
    //     let previous = self.next_id.unwrap_or(0);
    //     let next = self.next_id.map_or(0, |p| p + 1);
    //     self.next_id = next;

    //     if let Some(track) = self.queue.get(next) {
    //         *track
    //     } else {
    //         previous
    //     }
    // }
    fn enqueue(&mut self, id: usize) -> Result<(), Box<dyn Error>> {
        let source = self.open(id)?;
        self.player.append(source);
        self.in_player.push_back(id);

        Ok(())
    }
    fn index_of(&self, track_id: usize) -> Option<usize> {
        self.queue.iter().position(|&t| t == track_id)
    }

    fn play_id(&mut self, track_id: usize) -> Result<(), Box<dyn Error>> {
        // println!(
        //     "{:?} {:?} {:?}",
        //     self.position,
        //     self.player.len(),
        //     self.track_list
        // );
        let source = self.open(track_id)?;
        let id = match self.index_of(track_id) {
            Some(i) => i,
            None => {
                self.queue.push(track_id);
                self.queue.len() - 1
            }
        };
        self.in_player.clear();
        self.player.clear();
        self.next_id = id + 1;
        self.in_player.push_back(track_id);
        self.player.append(source);
        self.player.play();
        Ok(())
    }
    // fn previous_track(&mut self) -> Result<(), Box<dyn Error>> {
    //     let next = self
    //         .position
    //         .map_or(0, |p| if p == 0 { self.queue.len() - 1 } else { p - 1 });
    //     if let Some(&id) = self.queue.get(next) {
    //         self.position = Some(next);
    //         self.play_id(id)
    //     } else {
    //         Ok(())
    //     }
    // }
    // fn previous_track(&mut self) -> Result<(), Box<dyn Error>> {
    //     let id = match self.position() {
    //         None | Some(0) => self.queue.len() - 1,
    //         Some(p) => p - 1,
    //     };
    //     self.play_id(id)
    // }
    // fn next_track(&mut self) -> Result<(), Box<dyn Error>> {
    //     if self.queue.is_empty() {
    //         println!("empty queue");
    //         return Ok(());
    //     }
    //     let id = self.position().map_or(0, |p| p + 1) % self.queue.len();

    //     self.play_id(id)
    // }
    fn next_track(&mut self, forward: bool) -> Result<(), Box<dyn Error>> {
        let len = self.queue.len();
        if len == 0 {
            return Err("queue is empty".into());
        } else if self.player.len() > 1 && forward {
            self.player.skip_one();
            self.in_player.pop_front();
            return Ok(());
        }

        let cur = self
            .in_player
            .front()
            .copied()
            .and_then(|t| self.index_of(t));
        let idx = match (cur, forward) {
            (Some(i), true) => (i + 1) % len,
            (Some(i), false) => (i + len - 1) % len,
            (None, true) => 0,
            (None, false) => len - 1,
        };
        self.play_id(self.queue[idx])
    }
    fn handle_event(&mut self, payload: EventPayload) -> Result<(), Box<dyn Error>> {
        // if self.current_track.is_none() {
        //     return;
        // };
        match payload {
            EventPayload::PlaySong { id } => self.play_id(id)?,
            EventPayload::ChangeVolume { value } => {
                self.player
                    .set_volume((self.player.volume() + value).min(1.0).max(0.0));
            }
            EventPayload::SetVolume { value } => self.player.set_volume(value),
            EventPayload::Pause => self.player.pause(),
            EventPayload::Unpause => self.player.play(),
            EventPayload::Enqueue { id } => self.enqueue(id)?,
            EventPayload::NextSong => self.next_track(true)?,
            EventPayload::PreviousSong => self.next_track(false)?,
            EventPayload::Shuffle => self.shuffle(),
            EventPayload::ChangeQueueMode { mode } => self.queue_mode = mode,
            _ => {}
        }
        Ok(())
    }
    fn set_track_list(&mut self, track_list: HashMap<usize, TrackData>) {
        self.track_list = track_list;
    }
    fn update(&mut self) {
        while self.in_player.len() > self.player.len() {
            self.in_player.pop_front();
        }
        if self.player.len() >= 2 {
            return;
        }
        if let Some(cur) = self.in_player.front() {
            if let Some(d) = self.track_list.get(cur).and_then(|t| t.duration) {
                if d.saturating_sub(self.player.get_pos()) > Duration::from_millis(200) {
                    return;
                }
            }
        }
        let mut failures = 0;
        while self.player.len() < 2 && failures < self.queue.len() {
            if self.next_id >= self.queue.len() {
                self.next_id = 0;
            }
            let track_id = self.queue[self.next_id];
            self.next_id += 1;
            if let Err(e) = self.enqueue(track_id) {
                eprintln!("skipping track {track_id}: {e}");
                failures += 1;
            }
            if self.queue_mode == QueueModes::OnRepeat {
                self.next_id -= 1;
            }
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
                if let Err(e) = Self::check_dir(self, &entry.path()) {
                    println!("{e}");
                }
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
                duration INTEGER,
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
                duration: None,
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
                    let duration = tagged_file.properties().duration().as_millis() as i64;
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
                            "UPDATE db SET duration = ?1, title = ?2, artist = ?3, album = ?4, genre = ?5 WHERE path = ?6 ",
                            params![duration, title, artist, album, genre, &path],
                        )?;
                        // TODO: add more tags

                        // println!("{} {} {} {}", title, artist, album, genre)
                    }
                }
                Err(e) => {
                    eprintln!("{e}, Error reading from path: {path}")
                }
            }
        }

        Ok(())
    }
    fn fetch_db(&mut self) -> Result<HashMap<usize, TrackData>, Box<dyn Error>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, path, duration, COALESCE(title,'Unknown Title'), COALESCE(artist,'Unknown Artist'), COALESCE(album,'Unknown Album'), COALESCE(genre,'Unknown Genre') FROM db")?;
        let tags = stmt
            .query_map([], |row| {
                let id_: i32 = row.get(0)?;
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
    duration: Option<Duration>,
    tag: Option<TrackTag>,
}
// struct Track {
//     source: Option<Decoder<BufReader<File>>>,
//     data: Option<TrackData>,
//     id: usize,
// }
impl TrackData {
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
    fn get_id(buffer: &mut String, stdin: &io::Stdin) -> Option<usize> {
        buffer.clear();
        if stdin.read_line(buffer).ok()? == 0 {
            return None;
        }
        let id = buffer.trim().parse::<usize>().unwrap_or_else(|_| {
            println!("Invalid id");
            0
        });
        Some(id)
    }
}
enum Response {
    Empty,
    Exit,
    TrackData { data: TrackData },
}
enum EventPayload {
    Update,
    Enqueue { id: usize },
    Shuffle,
    Pause,
    Unpause,
    ChangeVolume { value: f32 },
    SetVolume { value: f32 },
    NextSong,
    PreviousSong,
    PlaySong { id: usize },
    GetTrackInfo,
    ChangeQueueMode { mode: QueueModes },
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

    fn handle_event(&mut self, event: Event) -> Result<Response, Box<dyn Error>> {
        match event.payload {
            EventPayload::Exit => return Ok(Response::Exit),
            EventPayload::Update => self.queue.update(),
            EventPayload::GetTrackInfo => {
                return Ok(Response::TrackData {
                    data: self.queue.get_track_data()?,
                });
            }
            other => self.queue.handle_event(other)?,
        }
        Ok(Response::Empty)
    }
    fn load_tracks(&mut self) -> Result<(), Box<dyn Error>> {
        let tracks = self.resources.fetch_db()?;
        self.queue.set_track_list(tracks);
        Ok(())
    }
    fn update(&mut self) {
        self.queue.update();
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
                    let mut state = clone.lock().unwrap_or_else(|p| p.into_inner());
                    if let Err(e) = state.handle_event(Event {
                        payload: EventPayload::Update,
                    }) {
                        eprintln!("update error: {e}");
                    }
                }
                sleep(Duration::from_millis(100)); // maybe change
            }
        });
    }
    fn parse_command(line: &str) -> Result<EventPayload, String> {
        let mut parts = line.split_whitespace();
        let cmd = parts.next().ok_or("empty command")?;
        let mut arg = || -> Result<usize, String> {
            parts
                .next()
                .ok_or("missing argument")?
                .parse::<usize>()
                .map_err(|e| format!("bad number: {e}"))
        };
        Ok(match cmd {
            "exit" => EventPayload::Exit,
            "play" => EventPayload::PlaySong { id: arg()? },
            "enqu" => EventPayload::Enqueue { id: arg()? },
            "next" => EventPayload::NextSong,
            "prev" => EventPayload::PreviousSong,
            "vol" => EventPayload::SetVolume {
                value: (arg()? as f32 / 100.0).clamp(0.0, 1.0),
            },
            "paus" => EventPayload::Pause,
            "resu" => EventPayload::Unpause,
            "shuf" => EventPayload::Shuffle,
            "info" => EventPayload::GetTrackInfo,
            "mode" => EventPayload::ChangeQueueMode {
                mode: if arg()? == 0 {
                    QueueModes::Shuffle
                } else {
                    QueueModes::OnRepeat
                },
            },
            other => return Err(format!("unknown command: {other}")),
        })
    }
    fn run(&mut self) -> Result<(), Box<dyn Error>> {
        {
            let mut state = self.state.lock().unwrap();
            state.load_tracks()?;
            state.handle_event(Event {
                payload: EventPayload::Shuffle,
            })?;
        }
        let mut buffer = String::new();

        self.poll();

        loop {
            buffer.clear();
            if io::stdin().read_line(&mut buffer)? == 0 {
                break;
            }
            let line = buffer.trim();
            if line.is_empty() {
                continue;
            }
            let payload = match Self::parse_command(line) {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("{e}");
                    continue;
                }
            };
            let result = self.state.lock().unwrap().handle_event(Event { payload });
            match result {
                Ok(Response::Exit) => break,
                Ok(Response::TrackData { data }) => println!("{data:?}"),
                Ok(Response::Empty) => {}
                Err(e) => eprintln!("error: {e}"),
            }
        }
        Ok(())
    }
}
fn main() -> Result<(), Box<dyn Error>> {
    let mut app = App::new();
    app.run()
}
