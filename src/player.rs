use rand::seq::SliceRandom;
use rodio::{Decoder, MixerDeviceSink};
use std::collections::HashMap;
use std::collections::VecDeque;
use std::error::Error;
use std::fs::File;
use std::io::BufReader;
use std::time::Duration;

use crate::types::{EventPayload, QueueModes, Status, TrackData};
pub struct Queue {
    pub queue: Vec<usize>,
    pub user_queue: VecDeque<usize>,
    pub history: Vec<usize>,
    pub current_idx: usize,
    #[allow(unused)]
    pub sink_handle: MixerDeviceSink,
    pub track_list: HashMap<usize, TrackData>,
    pub player: rodio::Player,
    pub next_idx: usize,
    pub queue_mode: QueueModes,
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
            history: Vec::new(),
            current_idx: 0,
            sink_handle,
            track_list: HashMap::new(),
            player,
            next_idx: 0,
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
    pub fn status(&self) -> Status {
        Status {
            current: self.user_queue.front().copied(),
            position: self.player.get_pos(),
            paused: self.player.is_paused(),
            volume: self.player.volume(),
            mode: self.queue_mode,
        }
    }
    pub fn take_warnings(&mut self) -> Vec<String> {
        std::mem::take(&mut self.warnings)
    }
    fn play_from_user_queue(&mut self) -> Result<(), Box<dyn Error>> {
        let id = self.user_queue.pop_front().ok_or("No such id")?;
        let source = self.open(id)?;
        self.player.clear();
        self.player.append(source);
        self.player.play();
        if self.player.len() >= 0 {
            println!("skipped idk"); // temp
        }
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
        self.next_idx = 1;
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
        let idx = match self.index_of(track_id) {
            Some(i) => i,
            None => {
                self.queue.push(track_id);
                self.queue.len() - 1
            }
        };
        self.player.clear();

        self.update_next_idx();
        self.player.append(source);
        self.player.play();
        Ok(())
    }
    fn update_next_idx(&mut self) {
        self.current_idx = self.next_idx;
        let len = self.queue.len();
        self.next_idx = if self.next_idx >= len {
            0
        } else {
            self.next_idx + 1
        };
    }
    fn step(&mut self) -> Result<(), Box<dyn Error>> {
        let idx = match self.queue_mode {
            QueueModes::OnRepeat => self.current_idx,
            _ => self.next_idx,
        };
        let source = if self.user_queue.len() > 0 {
            let id = self
                .user_queue
                .pop_front()
                .ok_or("yeah uh something broke")?;
            self.open(id)?
        } else {
            let len = self.queue.len();
            if len == 0 {
                return Err("queue is empty".into());
            }
            let src = self.open(idx)?;
            self.update_next_idx();
            src
        };
        self.player.append(source);
        Ok(())
    }
    pub fn update(&mut self) -> Result<(), Box<dyn Error>> {
        if let Some(d) = self
            .track_list
            .get(&self.current_idx)
            .and_then(|t| t.duration)
        {
            if d.saturating_sub(self.player.get_pos()) < TICK {
                self.step()?;
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
            self.update_next_idx();
        }
        let id = self.queue.get(self.next_idx).unwrap();
        self.play_id(*id)?;
        Ok(())
    }
    pub fn handle_event(&mut self, payload: EventPayload) -> Result<(), Box<dyn Error>> {
        match payload {
            EventPayload::PlaySong { id } => self.play_id(id)?,
            EventPayload::ChangeVolume { value } => {
                self.player
                    .set_volume((self.player.volume() + value).clamp(0.0, 1.0));
            }
            EventPayload::SetVolume { value } => self.player.set_volume(value),
            EventPayload::TogglePause => {
                if self.player.is_paused() {
                    self.player.play()
                } else {
                    self.player.pause()
                }
            }
            EventPayload::Enqueue { id } => self.play_next(id),
            EventPayload::NextSong => self.next_track(true)?,
            EventPayload::PreviousSong => self.next_track(false)?,
            EventPayload::Shuffle => self.shuffle(),
            EventPayload::ChangeQueueMode { mode } => self.queue_mode = mode,
            _ => {}
        }
        Ok(())
    }
    pub fn set_track_list(&mut self, track_list: HashMap<usize, TrackData>) {
        self.track_list = track_list;
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
