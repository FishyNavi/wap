use lofty::file::AudioFile;
use lofty::file::TaggedFileExt;
use lofty::read_from_path;
use lofty::tag::Accessor;
use rand::seq::SliceRandom;
use ratatui::crossterm::event::{self, Event as TermEvent, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Gauge, List, ListItem, ListState, Paragraph};
use ratatui::{DefaultTerminal, Frame};
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
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::thread;
use std::time::Duration;
#[derive(Debug, Clone, Copy, PartialEq)]
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
    warnings: Vec<String>,
}
impl Queue {
    fn new() -> Result<Self, Box<dyn Error>> {
        let sink_handle = rodio::DeviceSinkBuilder::open_default_sink()
            .map_err(|e| format!("cannot open audio device: {e:?}"))?;
        let player = rodio::Player::connect_new(&sink_handle.mixer());
        player.pause();

        Ok(Self {
            queue: Vec::new(),
            in_player: VecDeque::new(),
            sink_handle,
            track_list: HashMap::new(),
            player,
            next_id: 0,
            queue_mode: QueueModes::Shuffle,
            warnings: Vec::new(),
        })
    }
    fn open(&self, track_id: usize) -> Result<Decoder<BufReader<File>>, Box<dyn Error>> {
        let data = self
            .track_list
            .get(&(track_id))
            .ok_or_else(|| format!("no track with id {track_id}"))?;
        let file = File::open(&data.path).map_err(|e| format!("{}: {e}", data.path))?;
        Ok(Decoder::try_from(file)?)
    }
    fn status(&self) -> Status {
        Status {
            current: self.in_player.front().copied(),
            position: self.player.get_pos(),
            paused: self.player.is_paused(),
            volume: self.player.volume(),
            mode: self.queue_mode,
        }
    }
    fn take_warnings(&mut self) -> Vec<String> {
        std::mem::take(&mut self.warnings)
    }

    // fn get_track_data(&self) -> Result<TrackData, Box<dyn Error>> {
    //     Ok(self
    //         .track_list
    //         .get(self.in_player.front().ok_or("id not found")?)
    //         .cloned()
    //         .ok_or("not such id")?)
    // }
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
            EventPayload::TogglePause => {
                if self.player.is_paused() {
                    self.player.play()
                } else {
                    self.player.pause()
                }
            }
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
            let repeat_id = if self.queue_mode == QueueModes::OnRepeat {
                self.in_player.back().copied()
            } else {
                None
            };

            let track_id = match repeat_id {
                Some(id) => id,
                None => {
                    if self.next_id >= self.queue.len() {
                        self.next_id = 0;
                    }
                    let id = self.queue[self.next_id];
                    self.next_id += 1;
                    id
                }
            };

            if let Err(e) = self.enqueue(track_id) {
                if self.warnings.len() < 5 {
                    self.warnings
                        .push(format!("skipping track {track_id}: {e}"));
                }
                failures += 1;
                if repeat_id.is_some() {
                    break;
                }
            }
        }
    }
}
struct Resources {
    conn: Connection,
    skipped: usize,
}
const AUDIO_EXTENSIONS: &[&str] = &["mp3", "flac", "ogg", "oga", "opus", "wav", "m4a", "aac"];
impl Resources {
    fn new() -> Result<Self, Box<dyn Error>> {
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
        // fix this maybe
        for entry in fs::read_dir(dir)? {
            let Ok(entry) = entry else {
                self.skipped += 1;
                continue;
            };
            let path = entry.path();
            if entry.path().is_dir() {
                if self.check_dir(&entry.path()).is_err() {
                    self.skipped += 1;
                };
            } else if Self::is_audio(&path) {
                self.conn.execute(
                    "INSERT INTO db (path) VALUES (?1)",
                    params![path.display().to_string()],
                )?;
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
        let rows: Vec<(i64, String)> = {
            let mut stmt = self.conn.prepare("SELECT id, path FROM db")?;
            stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect::<Result<_, _>>()?
        };

        let db_tx = self.conn.transaction()?;
        for (_, path) in rows {
            match read_from_path(&path) {
                Ok(tagged_file) => {
                    // for item in tagged_file.primary_tag().unwrap().items() {
                    //     println!("{:?}", item)
                    // }
                    let duration = tagged_file.properties().duration().as_millis() as i64;
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
#[allow(dead_code)]
#[derive(Debug, Clone)]
struct TrackTag {
    title: String,
    artist: String,
    album: String,
    genre: String,
}
// impl TrackTag {
//     fn new(title: String, artist: String, album: String, genre: String) -> Self {
//         Self {
//             title,
//             artist,
//             album,
//             genre,
//         }
//     }
// }
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
// impl Event {
//     fn get_id(buffer: &mut String, stdin: &io::Stdin) -> Option<usize> {
//         buffer.clear();
//         if stdin.read_line(buffer).ok()? == 0 {
//             return None;
//         }
//         let id = buffer.trim().parse::<usize>().unwrap_or_else(|_| {
//             println!("Invalid id");
//             0
//         });
//         Some(id)
//     }
// }
enum Response {
    Empty,
    Exit,
    // TrackData { data: TrackData },
}
#[allow(dead_code)]
enum EventPayload {
    Update,
    Enqueue { id: usize },
    Shuffle,
    TogglePause,
    ChangeVolume { value: f32 },
    SetVolume { value: f32 },
    NextSong,
    PreviousSong,
    PlaySong { id: usize },
    GetTrackInfo,
    ChangeQueueMode { mode: QueueModes },
    Exit,
}
struct State {
    queue: Queue,
    resources: Resources,
}
impl State {
    fn new(dir: &Path) -> Result<Self, Box<dyn Error>> {
        let queue = Queue::new()?;
        let mut resources = Resources::new()?;
        resources.index(dir)?;
        Ok(Self { queue, resources })
    }

    fn handle_event(&mut self, event: Event) -> Result<Response, Box<dyn Error>> {
        match event.payload {
            EventPayload::Exit => return Ok(Response::Exit),
            EventPayload::Update => self.queue.update(),
            other => self.queue.handle_event(other)?,
        }
        Ok(Response::Empty)
    }

    fn load_tracks(&mut self) -> Result<(), Box<dyn Error>> {
        let tracks = self.resources.fetch_db()?;
        self.queue.set_track_list(tracks);
        Ok(())
    }
}
const TICK: Duration = Duration::from_millis(100);
fn backend_main(dir: PathBuf, rx: Receiver<Event>, tx: Sender<UiMessage>) {
    let init = || -> Result<State, Box<dyn Error>> {
        let mut state = State::new(&dir)?;
        state.load_tracks()?;
        state.queue.shuffle();
        Ok(state)
    };
    let mut state = match init() {
        Ok(s) => s,
        Err(e) => {
            let _ = tx.send(UiMessage::Fatal(e.to_string()));
            return;
        }
    };

    let send = |m: UiMessage| tx.send(m).is_ok();

    let library: Vec<TrackData> = state.queue.track_list.values().cloned().collect();
    if !send(UiMessage::Library(library)) {
        return;
    }
    if state.resources.skipped > 0 {
        let n = state.resources.skipped;
        send(UiMessage::Notice(format!("{n} file(s) could not be read")));
    }

    loop {
        match rx.recv_timeout(TICK) {
            Ok(event) => match state.handle_event(event) {
                Ok(Response::Exit) => break,
                Ok(Response::Empty) => {}
                Err(e) => {
                    if !send(UiMessage::Notice(e.to_string())) {
                        break;
                    }
                }
            },
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }

        state.queue.update();
        for w in state.queue.take_warnings() {
            send(UiMessage::Notice(w));
        }
        if !send(UiMessage::Status(state.queue.status())) {
            break;
        }
    }
}

// struct App {
//     state: Arc<Mutex<State>>,
// }
// impl App {
//     fn new() -> Result<Self, Box<dyn Error>> {
//         let state = Arc::new(Mutex::new(State::new()?));

//         Ok(Self { state })
//     }
//     fn poll(&self) {
//         let clone = Arc::clone(&self.state);

//         thread::spawn(move || {
//             loop {
//                 {
//                     let mut state = clone.lock().unwrap_or_else(|p| p.into_inner());
//                     if let Err(e) = state.handle_event(Event {
//                         payload: EventPayload::Update,
//                     }) {
//                         eprintln!("update error: {e}");
//                     }
//                 }
//                 sleep(Duration::from_millis(100)); // maybe change
//             }
//         });
//     }
//     fn parse_command(line: &str) -> Result<EventPayload, String> {
//         let mut parts = line.split_whitespace();
//         let cmd = parts.next().ok_or("empty command")?;
//         let mut arg = || -> Result<usize, String> {
//             parts
//                 .next()
//                 .ok_or("missing argument")?
//                 .parse::<usize>()
//                 .map_err(|e| format!("bad number: {e}"))
//         };
//         Ok(match cmd {
//             "exit" => EventPayload::Exit,
//             "play" => EventPayload::PlaySong { id: arg()? },
//             "enqu" => EventPayload::Enqueue { id: arg()? },
//             "next" => EventPayload::NextSong,
//             "prev" => EventPayload::PreviousSong,
//             "vol" => EventPayload::SetVolume {
//                 value: (arg()? as f32 / 100.0).clamp(0.0, 1.0),
//             },
//             "paus" => EventPayload::Pause,
//             "resu" => EventPayload::Unpause,
//             "shuf" => EventPayload::Shuffle,
//             "info" => EventPayload::GetTrackInfo,
//             "mode" => EventPayload::ChangeQueueMode {
//                 mode: if arg()? == 0 {
//                     QueueModes::Shuffle
//                 } else {
//                     QueueModes::OnRepeat
//                 },
//             },
//             other => return Err(format!("unknown command: {other}")),
//         })
//     }
//     fn run(&mut self) -> Result<(), Box<dyn Error>> {
//         {
//             let mut state = self.state.lock().unwrap();
//             state.load_tracks()?;
//             state.handle_event(Event {
//                 payload: EventPayload::Shuffle,
//             })?;
//         }
//         let mut buffer = String::new();

//         self.poll();

//         loop {
//             buffer.clear();
//             if io::stdin().read_line(&mut buffer)? == 0 {
//                 break;
//             }
//             let line = buffer.trim();
//             if line.is_empty() {
//                 continue;
//             }
//             let payload = match Self::parse_command(line) {
//                 Ok(p) => p,
//                 Err(e) => {
//                     eprintln!("{e}");
//                     continue;
//                 }
//             };
//             let result = self.state.lock().unwrap().handle_event(Event { payload });
//             match result {
//                 Ok(Response::Exit) => break,
//                 Ok(Response::TrackData { data }) => println!("{data:?}"),
//                 Ok(Response::Empty) => {}
//                 Err(e) => eprintln!("error: {e}"),
//             }
//         }
//         Ok(())
//     }
// }

// ================================================================================================
//                                             UI
// ================================================================================================
fn fmt_dur(d: Duration) -> String {
    format!("{}:{:02}", d.as_secs() / 60, d.as_secs() % 60)
}

fn label(t: &TrackData) -> String {
    match &t.tag {
        Some(tag) if tag.title != "Unknown Title" => format!("{} — {}", tag.artist, tag.title),
        _ => Path::new(&t.path)
            .file_stem()
            .and_then(OsStr::to_str)
            .unwrap_or("?")
            .to_string(),
    }
}
#[derive(Debug, Clone, Copy)]
struct Status {
    current: Option<usize>,
    position: Duration,
    paused: bool,
    volume: f32,
    mode: QueueModes,
}
enum UiMessage {
    Library(Vec<TrackData>),
    Status(Status),
    Notice(String),
    Fatal(String),
}
struct Ui {
    tx: Sender<Event>,
    rx: Receiver<UiMessage>,
    library: Vec<TrackData>,
    status: Status,
    list: ListState,
    message: Option<String>,
    fatal: Option<String>,
    running: bool,
}

impl Ui {
    fn new(tx: Sender<Event>, rx: Receiver<UiMessage>) -> Self {
        Self {
            tx,
            rx,
            library: Vec::new(),
            status: Status {
                current: None,
                position: Duration::ZERO,
                paused: true,
                volume: 1.0,
                mode: QueueModes::Shuffle,
            },
            list: ListState::default(),
            message: Some("Scanning library…".into()),
            fatal: None,
            running: true,
        }
    }

    fn run(&mut self, terminal: &mut DefaultTerminal) -> io::Result<()> {
        while self.running {
            self.drain();
            terminal.draw(|f| self.draw(f))?;
            if event::poll(Duration::from_millis(50))? {
                if let TermEvent::Key(key) = event::read()? {
                    if key.kind == KeyEventKind::Press {
                        self.on_key(key.code, key.modifiers);
                    }
                }
            }
        }
        Ok(())
    }
    fn drain(&mut self) {
        loop {
            match self.rx.try_recv() {
                Ok(UiMessage::Library(mut tracks)) => {
                    tracks.sort_by_cached_key(|t| label(t).to_lowercase());
                    self.list
                        .select(if tracks.is_empty() { None } else { Some(0) });
                    self.library = tracks;
                    self.message = None;
                }
                Ok(UiMessage::Status(s)) => self.status = s,
                Ok(UiMessage::Notice(m)) => self.message = Some(m),
                Ok(UiMessage::Fatal(m)) => {
                    self.fatal = Some(m);
                    self.running = false;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.running = false;
                    break;
                }
            }
        }
    }

    fn send(&mut self, payload: EventPayload) {
        if self.tx.send(Event { payload }).is_err() {
            self.running = false;
        }
    }

    fn on_key(&mut self, code: KeyCode, mods: KeyModifiers) {
        self.message = None;
        let selected = self
            .list
            .selected()
            .and_then(|i| self.library.get(i))
            .map(|t| t.id);

        match code {
            KeyCode::Char('c') if mods.contains(KeyModifiers::CONTROL) => self.quit(),
            KeyCode::Char('q') | KeyCode::Esc => self.quit(),
            KeyCode::Up | KeyCode::Char('k') => self.list.select_previous(),
            KeyCode::Down | KeyCode::Char('j') => self.list.select_next(),
            KeyCode::Enter => {
                if let Some(id) = selected {
                    self.send(EventPayload::PlaySong { id });
                }
            }
            KeyCode::Char('e') => {
                if let Some(id) = selected {
                    self.send(EventPayload::Enqueue { id });
                }
            }
            KeyCode::Char(' ') => self.send(EventPayload::TogglePause),
            KeyCode::Char('n') => self.send(EventPayload::NextSong),
            KeyCode::Char('p') => self.send(EventPayload::PreviousSong),
            KeyCode::Char('+') | KeyCode::Char('=') => {
                self.send(EventPayload::ChangeVolume { value: 0.05 })
            }
            KeyCode::Char('-') => self.send(EventPayload::ChangeVolume { value: -0.05 }),
            KeyCode::Char('s') => self.send(EventPayload::Shuffle),
            KeyCode::Char('r') => {
                let mode = match self.status.mode {
                    QueueModes::Shuffle => QueueModes::OnRepeat,
                    QueueModes::OnRepeat => QueueModes::Shuffle,
                };
                self.send(EventPayload::ChangeQueueMode { mode });
            }
            _ => {}
        }
    }

    fn quit(&mut self) {
        self.send(EventPayload::Exit);
        self.running = false;
    }

    fn draw(&mut self, f: &mut Frame) {
        let [list_area, now_area, help_area] = Layout::vertical([
            Constraint::Min(3),
            Constraint::Length(4),
            Constraint::Length(1),
        ])
        .areas(f.area());

        let items: Vec<ListItem> = self
            .library
            .iter()
            .map(|t| {
                let playing = Some(t.id) == self.status.current;
                let dur = t.duration.map_or("--:--".to_string(), fmt_dur);
                let style = if playing {
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };
                ListItem::new(format!(
                    "{} {:>4}  {}  [{}]",
                    if playing { "▶" } else { " " },
                    t.id,
                    label(t),
                    dur
                ))
                .style(style)
            })
            .collect();
        let list = List::new(items)
            .block(Block::bordered().title(format!(" Library ({}) ", self.library.len())))
            .highlight_style(Style::default().add_modifier(Modifier::REVERSED));
        f.render_stateful_widget(list, list_area, &mut self.list);

        let block = Block::bordered().title(" Now playing ");
        let inner = block.inner(now_area);
        f.render_widget(block, now_area);
        let [info, bar] = Layout::vertical([Constraint::Length(1); 2]).areas(inner);

        let current = self
            .library
            .iter()
            .find(|t| Some(t.id) == self.status.current);
        let name = current.map_or("—".to_string(), label);
        let dur = current.and_then(|t| t.duration);
        let icon = match (current, self.status.paused) {
            (None, _) => "■",
            (_, true) => "⏸",
            (_, false) => "▶",
        };
        let mode = match self.status.mode {
            QueueModes::Shuffle => "shuffle",
            QueueModes::OnRepeat => "repeat",
        };
        f.render_widget(
            Paragraph::new(format!(
                "{icon} {name}    vol {}%   {mode}",
                (self.status.volume * 100.0).round() as u32
            )),
            info,
        );

        let ratio = dur.filter(|d| !d.is_zero()).map_or(0.0, |d| {
            (self.status.position.as_secs_f64() / d.as_secs_f64()).clamp(0.0, 1.0)
        });
        let text = format!(
            "{} / {}",
            fmt_dur(self.status.position),
            dur.map_or("--:--".to_string(), fmt_dur)
        );
        f.render_widget(
            Gauge::default()
                .gauge_style(Style::default().fg(Color::Cyan))
                .ratio(ratio)
                .label(text),
            bar,
        );
        let (text, color) = match &self.message {
            Some(m) => (m.as_str(), Color::Red),
            None => (
                "↑↓ move  Enter play  e enqueue  Space pause  n/p next/prev  +/- vol  s shuffle  r repeat  q quit",
                Color::DarkGray,
            ),
        };
        f.render_widget(
            Paragraph::new(text).style(Style::default().fg(color)),
            help_area,
        );
    }
}
fn main() -> Result<(), Box<dyn Error>> {
    let dir = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));

    let (cmd_tx, cmd_rx) = mpsc::channel::<Event>();
    let (ui_tx, ui_rx) = mpsc::channel::<UiMessage>();
    let backend = thread::spawn(move || backend_main(dir, cmd_rx, ui_tx));

    let mut ui = Ui::new(cmd_tx, ui_rx);
    let mut terminal = ratatui::init();
    let result = ui.run(&mut terminal);
    ratatui::restore();
    let _ = ui.tx.send(Event {
        payload: EventPayload::Exit,
    });
    let panicked = backend.join().is_err();

    result?;
    if let Some(msg) = ui.fatal {
        return Err(msg.into());
    }
    if panicked {
        return Err("backend thread panicked".into());
    }
    Ok(())
}
