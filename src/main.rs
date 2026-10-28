use rodio::{Decoder, MixerDeviceSink};
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
    fn handle_event(&mut self, event: i16) {
        match event {
            1 => self.play(),
            2 => {
                self.player
                    .set_volume((self.player.volume() + 0.05).min(1.0).max(0.0));
            }
            3 => {
                self.player
                    .set_volume((self.player.volume() - 0.05).min(1.0).max(0.0));
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
    fn handle_event(&mut self, event: i16) -> bool {
        match event {
            0 => println!("its a zero"), // debug
            1..10 => self.queue.handle_event(event),
            10 => return false,
            _ => {}
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
            let input = input.trim().parse::<i16>().unwrap_or(0);
            if !self.state.handle_event(input) {
                break;
            }
        }
    }
}
fn main() {
    let mut app = App::new();
    app.run();
}
