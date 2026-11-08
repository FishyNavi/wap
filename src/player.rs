use rand::seq::SliceRandom;
use rodio::{Decoder, MixerDeviceSink};
use std::collections::HashMap;
use std::collections::VecDeque;
use std::error::Error;
use std::fs::File;
use std::io::BufReader;
use std::time::Duration;
use std::usize;

use crate::types::OnRepeat;
use crate::types::Response;
use crate::types::{EventPayload, Status, TrackData};
pub struct Queue {
    pub queue: Vec<usize>,
    pub user_queue: VecDeque<usize>,
    pub current_id: Option<usize>,
    pub current_idx: usize,
    #[allow(unused)]
    pub sink_handle: MixerDeviceSink,
    pub track_list: HashMap<usize, TrackData>,
    pub player: rodio::Player,
    pub next_idx: usize,
    pub on_repeat: OnRepeat,
    pub shuffle: bool,
    pub warnings: Vec<String>,
}
pub const TICK: Duration = Duration::from_millis(100);
impl Queue {
    pub fn new() -> Result<Self, Box<dyn Error>> {
        let mut sink_handle = rodio::DeviceSinkBuilder::open_default_sink()
            .map_err(|e| format!("cannot open audio device: {e:?}"))?;
        sink_handle.log_on_drop(false);
        let player = rodio::Player::connect_new(&sink_handle.mixer());
        player.pause();

        Ok(Self {
            queue: Vec::new(),
            user_queue: VecDeque::new(),
            current_id: None,
            current_idx: 0,
            sink_handle,
            track_list: HashMap::new(),
            player,
            next_idx: 0,
            on_repeat: OnRepeat::Yes,
            shuffle: true,
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
    pub fn status(&self) -> Status {
        Status {
            current: self.current_id,
            position: self.player.get_pos(),
            paused: self.player.is_paused(),
            volume: self.player.volume(),
            on_repeat: self.on_repeat,
            shuffle: self.shuffle,
        }
    }
    pub fn take_warnings(&mut self) -> Vec<String> {
        std::mem::take(&mut self.warnings)
    }
    fn play_from_user_queue(&mut self) -> Result<(), Box<dyn Error>> {
        let id = self.user_queue.pop_front().ok_or("No such id")?;
        self.current_id = Some(id);
        let source = self.open(id)?;
        self.player.clear();
        self.player.append(source);
        self.player.play();
        Ok(())
    }

    // fn get_track_data(&self) -> Result<TrackData, Box<dyn Error>> {
    //     Ok(self
    //         .track_list
    //         .get(self.queue.front().ok_or("id not found")?)
    //         .cloned()
    //         .ok_or("not such id")?)
    // }
    pub fn shuffle(&mut self) {
        let mut ids: Vec<usize> = self.track_list.keys().copied().collect();
        ids.shuffle(&mut rand::rng());
        self.queue = ids;
        self.next_idx = if self.queue.len() > 1 { 1 } else { 0 };
        self.current_idx = 0;
    }
    fn play_next(&mut self, id: usize) {
        self.user_queue.push_front(id);
    }
    fn enqueue(&mut self, id: usize) {
        self.user_queue.push_back(id);
    }
    fn skip(&mut self) {
        self.user_queue.pop_front();
    }
    fn index_of(&self, track_id: usize) -> Option<usize> {
        self.queue.iter().position(|&t| t == track_id)
    }

    fn play_id(&mut self, track_id: usize) -> Result<(), Box<dyn Error>> {
        let source = self.open(track_id)?;
        self.player.clear();
        self.current_idx = self.index_of(track_id).unwrap_or(self.current_idx);
        self.current_id = Some(track_id);
        self.player.append(source);
        self.player.play();
        Ok(())
    }
    fn update_next_idx(&mut self) {
        let len = self.queue.len();
        if len == 0 {
            self.current_idx = 0;
            self.next_idx = 0;
            return;
        }

        self.current_idx = self.next_idx;
        if self.current_idx >= len {
            if self.on_repeat == OnRepeat::No {
                let _ = self.handle_event(EventPayload::ReachedEnd);
            }
            self.current_idx = 0;
        }

        self.next_idx = (self.current_idx + 1) % len;
    }

    fn step(&mut self) -> Result<(), Box<dyn Error>> {
        // let idx = match self.on_repeat {
        //     OnRepeat::Single => self.current_idx,
        //     _ => self.current_idx,
        // };

        let track_id = if self.user_queue.len() > 0 {
            self.user_queue
                .pop_front()
                .ok_or("yeah uh something broke")?
        } else {
            let len = self.queue.len();
            if len == 0 {
                return Err("queue is empty".into());
            }
            match self.on_repeat {
                OnRepeat::Single => {}
                _ => self.update_next_idx(),
            }
            *self
                .queue
                .get(self.current_idx)
                .ok_or("queue index out of bounds")?
        };
        self.current_id = Some(track_id);
        let src = self.open(track_id)?;
        self.player.append(src);
        self.player.play();
        Ok(())
    }
    pub fn update(&mut self) -> Result<(), Box<dyn Error>> {
        if self.player.len() < 2 {
            if let Some(id) = self.current_id {
                if let Some(d) = self.track_list.get(&id).and_then(|t| t.duration) {
                    if d.saturating_sub(self.player.get_pos()) < TICK {
                        self.step()?;
                    }
                }
            }
        }

        Ok(())
    }
    fn next_track(&mut self, forward: bool) -> Result<(), Box<dyn Error>> {
        let len = self.queue.len();
        if len == 0 {
            return Err("queue is empty".into());
        }
        if self.user_queue.len() > 0 && forward {
            return Ok(self.play_from_user_queue()?);
        }
        if self.next_idx >= self.queue.len() {
            self.next_idx = 0;
        }

        let id = if forward {
            self.queue.get(self.next_idx).ok_or("next_track broke 1")?
        } else {
            self.next_idx = if self.player.get_pos() <= Duration::from_secs(3) {
                self.previous_idx()
            } else {
                self.current_idx
            };
            self.queue.get(self.next_idx).ok_or("next_track broke 2")?
        };

        self.play_id(*id)?;
        self.update_next_idx();
        Ok(())
    }
    fn previous_idx(&mut self) -> usize {
        let len = self.queue.len() - 1;
        if self.current_idx == 0 {
            len
        } else {
            self.current_idx - 1
        }
    }
    pub fn handle_event(&mut self, payload: EventPayload) -> Result<Response, Box<dyn Error>> {
        match payload {
            EventPayload::PlaySong { id } => self.play_id(id)?,
            EventPayload::ChangeVolume { value } => {
                self.player
                    .set_volume((self.player.volume() + value).clamp(0.0, 1.0));
            }
            EventPayload::SetVolume { value } => self.player.set_volume(value),
            EventPayload::TogglePause => {
                if self.player.is_paused() {
                    if self.player.len() > 0 {
                        self.player.play();
                    } else {
                        self.play_id(1)?;
                    }
                } else {
                    self.player.pause();
                }
            }
            EventPayload::Enqueue { id } => self.enqueue(id),
            EventPayload::PlayNext { id } => self.play_next(id),
            EventPayload::NextSong => self.next_track(true)?,
            EventPayload::PreviousSong => self.next_track(false)?,
            EventPayload::Shuffle => {
                self.shuffle = true;
                self.shuffle();
                return Ok(Response::QueueUpdate);
            }
            EventPayload::Unshuffle => {
                self.shuffle = false;
                let mut ids: Vec<usize> = self.track_list.keys().copied().collect();
                ids.sort();
                self.queue = ids;
                self.next_idx = if self.queue.len() > 1 { 1 } else { 0 };

                return Ok(Response::QueueUpdate);
            }
            EventPayload::ChangeRepeatMode { mode } => self.on_repeat = mode,
            EventPayload::ReachedEnd => self.player.pause(),

            _ => {}
        }
        Ok(Response::Empty)
    }
    pub fn set_track_list(&mut self, track_list: HashMap<usize, TrackData>) {
        self.track_list = track_list;
        self.shuffle();
    }
    pub fn fuzzy_search(&self, keyword: &str, fuzzy: bool) -> Vec<TrackData> {
        if fuzzy {
            use rust_fuzzy_search::fuzzy_search_threshold;

            let tracks: Vec<String> = self
                .track_list
                .iter()
                .map(|(id, data)| {
                    let tag = data.tag.clone().unwrap_or_default();
                    format!("[{}] {} {} {}", id, tag.title, tag.artist, tag.album)
                })
                .collect();
            let list: Vec<&str> = tracks.iter().map(|s| s.as_str()).collect();

            let res = fuzzy_search_threshold(&keyword, &list, 0.8);

            res.iter()
                .filter_map(|(val, _)| {
                    let id = val
                        .split_whitespace()
                        .next()?
                        .trim_matches(|c| c == '[' || c == ']')
                        .parse::<usize>()
                        .ok()?;
                    self.track_list.get(&id).cloned()
                })
                .collect()
        } else {
            let keyword = keyword.to_lowercase();

            self.track_list
                .iter()
                .filter_map(|(id, data)| {
                    let tag = data.tag.as_ref()?;
                    let matches = tag.title.to_lowercase().contains(&keyword); // add matching for title, album and genre separately
                    if matches {
                        self.track_list.get(&id).cloned()
                    } else {
                        None
                    }
                })
                .collect()
        }
    }
}

// impl TrackData {
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
// }
