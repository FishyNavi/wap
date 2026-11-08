use crate::player::{Queue, TICK};
use crate::resources::{self, Resources};
use crate::types::{Event, EventPayload, Response, TrackData, UiMessage};

use std::error::Error;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
struct State {
    queue: Queue,
    resources: Resources,
    search_results: Vec<TrackData>,
}
impl State {
    fn new() -> Result<Self, Box<dyn Error>> {
        let queue = Queue::new()?;
        let resources = Resources::new(Path::new("music.db"))?;
        Ok(Self {
            queue,
            resources,
            search_results: vec![],
        })
    }

    fn handle_event(&mut self, event: Event) -> Result<Response, Box<dyn Error>> {
        match event.payload {
            EventPayload::Exit => return Ok(Response::Exit),
            EventPayload::TracksLoaded { tracks } => {
                self.queue.set_track_list(tracks.clone());
                return Ok(Response::UpdateLibrary);
            }
            EventPayload::FuzzySearch(keyword) => {
                self.search_results = self.queue.fuzzy_search(keyword.as_str(), true);
                return Ok(Response::SearchResults);
            }
            other => return Ok(self.queue.handle_event(other)?),
        }
    }

    fn load_tracks(&mut self) -> Result<(), Box<dyn Error>> {
        let tracks = self.resources.fetch_db()?;
        self.queue.set_track_list(tracks);
        Ok(())
    }
}

pub fn backend_main(dir: PathBuf, ui_rx: Receiver<Event>, tx: Sender<UiMessage>) {
    let init = || -> Result<State, Box<dyn Error>> {
        let mut state = State::new()?;
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

    let (event_tx, rx) = std::sync::mpsc::channel();
    let fwd_tx = event_tx.clone();

    std::thread::spawn(move || {
        while let Ok(event) = ui_rx.recv() {
            if fwd_tx.send(event).is_err() {
                break;
            }
        }
    });
    let bg_dir = dir.clone();
    let bg_tx = event_tx.clone();

    std::thread::spawn(move || {
        let update_dur = std::time::Duration::from_secs(10);
        loop {
            if let Ok(mut resources) = Resources::new(Path::new("music.db")) {
                if resources.index(&bg_dir).is_ok() {
                    if let Ok(tracks) = resources.fetch_db() {
                        let _ = bg_tx.send(Event {
                            payload: EventPayload::TracksLoaded { tracks },
                        });
                    }
                }
            }
            std::thread::sleep(update_dur);
        }
    });
    if state.resources.skipped > 0 {
        let n = state.resources.skipped;
        send(UiMessage::Notice(format!("{n} file(s) could not be read")));
    }

    loop {
        match rx.recv_timeout(TICK) {
            Ok(event) => match state.handle_event(event) {
                Ok(Response::Exit) => break,
                Ok(Response::Empty) => {}
                Ok(Response::UpdateLibrary) => {
                    let library: Vec<TrackData> =
                        state.queue.track_list.values().cloned().collect();
                    if !send(UiMessage::Library(library)) {
                        return;
                    }
                }
                Ok(Response::SearchResults) => {
                    if !send(UiMessage::SearchResults(state.search_results.clone())) {
                        return;
                    }
                }
                Ok(Response::QueueUpdate) => {
                    if !send(UiMessage::QueueUpdate(state.queue.queue.clone())) {
                        return;
                    }
                }
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
