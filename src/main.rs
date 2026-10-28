use rodio::{Decoder, MixerDeviceSink, queue};
use std::fs::File;
use std::io;
use std::io::BufReader;
use std::sync::{Arc, RwLock};
struct Queue {
    queue: Arc<RwLock<Vec<i16>>>,
    sink_handle: MixerDeviceSink,
    player: rodio::Player,
    current_track: Track,
    next_track: Option<Track>,
    previous_track: Option<Track>,
}
impl Queue {
    fn new() -> Self {
        let current_track = Track::new(1).unwrap();
        let queue: Arc<RwLock<Vec<i16>>> = Arc::new(RwLock::new(Vec::new()));

        let sink_handle =
            rodio::DeviceSinkBuilder::open_default_sink().expect("open default audio stream");
        let player = rodio::Player::connect_new(&sink_handle.mixer());
        Self {
            queue,
            sink_handle,
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

            _ => {}
        }
    }
}
struct Track {
    name: String,
    source: Option<Decoder<BufReader<File>>>,
    id: i16,
}
impl Track {
    fn new(id: i16) -> Option<Track> {
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
    ChangeVolume { value: f32 },
    NextSong { next: bool },
    PlaySong { id: i32 },
    Exit,
    None, // temp ig
}
struct State {
    queue: Queue,
    paused: bool,
}
impl State {
    fn new() -> Self {
        let queue = Queue::new();
        let paused = false;
        Self { queue, paused }
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
