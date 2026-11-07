use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum QueueModes {
    OnRepeat,
    Shuffle,
}
pub struct Event {
    pub payload: EventPayload,
    // id time etc
}

#[allow(dead_code)]
pub enum EventPayload {
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
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct TrackTag {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub genre: String,
}
#[derive(Debug, Clone, Copy)]
pub struct Status {
    pub current: Option<usize>,
    pub position: Duration,
    pub paused: bool,
    pub volume: f32,
    pub mode: QueueModes,
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
pub struct TrackData {
    pub id: usize,
    pub path: String,
    pub duration: Option<Duration>,
    pub tag: Option<TrackTag>,
}
pub enum Response {
    Empty,
    Exit,
    // TrackData { data: TrackData },
}
pub enum UiMessage {
    Library(Vec<TrackData>),
    Status(Status),
    Notice(String),
    Fatal(String),
}
