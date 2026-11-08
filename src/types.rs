use std::time::Duration;
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OnRepeat {
    No,
    Yes,
    Single,
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
    ChangeRepeatMode { mode: OnRepeat },
    Exit,
    ReachedEnd,
}
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct TrackTag {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub genre: String,
}
impl Default for TrackTag {
    fn default() -> Self {
        Self {
            title: "Unknown Title".to_string(),
            artist: "Unknown Artist".to_string(),
            album: "".to_string(),
            genre: "".to_string(),
        }
    }
}
#[derive(Debug, Clone, Copy)]
pub struct Status {
    pub current: Option<usize>,
    pub position: Duration,
    pub paused: bool,
    pub volume: f32,
    pub on_repeat: OnRepeat,
    pub shuffle: bool,
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
