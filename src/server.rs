use crate::player::{Queue, TICK};
use crate::resources::Resources;
use crate::types::{Event, EventPayload, Response, TrackData, UiMessage};

use std::error::Error;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
struct State {
    queue: Queue,
    resources: Resources,
}
impl State {
    fn new(dir: &Path) -> Result<Self, Box<dyn Error>> {
        let queue = Queue::new()?;
        let mut resources = Resources::new(Path::new("."))?;
        resources.index(dir)?;
        Ok(Self { queue, resources })
    }

    fn handle_event(&mut self, event: Event) -> Result<Response, Box<dyn Error>> {
        match event.payload {
            EventPayload::Exit => return Ok(Response::Exit),
            EventPayload::Update => self.queue.update()?,
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

pub fn backend_main(dir: PathBuf, rx: Receiver<Event>, tx: Sender<UiMessage>) {
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

        let e = state.queue.update();
        match e {
            Ok(()) => {}
            Err(e) => {
                if !send(UiMessage::Notice(e.to_string())) {
                    break;
                }
            }
        }
        for w in state.queue.take_warnings() {
            send(UiMessage::Notice(w));
        }
        if !send(UiMessage::Status(state.queue.status())) {
            break;
        }
    }
}
