//! The player's public protocol: what a client sends (`Command`) and what it
//! receives (`Out`). No ratatui, no mpv, no JSON shapes from upstream services.
//! Everything derives serde so the later daemon can put these on its socket as
//! newline-delimited JSON with no translation layer.

use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::domain::{ItemKey, Playable, Station, StationId, Track, VideoId, Volume};
use crate::player::queue::EntryId;

/// Intent, not mechanism. Commands carry items by value or by stable id, never
/// an index into some list the client happens to be showing.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub enum Command {
    /// Enter on a YouTube result: queue becomes `[track]`, autoplay radio seeded by it.
    PlayTrack(Arc<Track>),
    /// Enter on a station: queue becomes the whole list, cursor on `start`, no radio.
    PlayStations {
        stations: Vec<Arc<Station>>,
        start: StationId,
    },
    /// `a`: insert right after the cursor. Starts it if nothing is playing.
    PlayNext(Playable),
    Next,
    Prev,
    Jump(EntryId),
    Remove(EntryId),
    TogglePause,
    SetPaused(bool),
    /// Ignored unless a track is playing. Stations have no seek; see `core::Playback`.
    Seek(SeekTo),
    Volume(VolumeTo),
    /// Absolute, so a double-delivered or retried command converges.
    SetFavorite {
        item: Playable,
        on: bool,
    },
    /// Resolve these ahead of time (top search results), within the in-flight cap.
    Warm(Vec<VideoId>),
    Shutdown,
}

#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
pub enum SeekTo {
    By(i32),
    At(Duration),
}

#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
pub enum VolumeTo {
    By(i8),
    At(Volume),
}

/// What subscribers receive. A new subscriber gets one `Snapshot` immediately.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub enum Out {
    Snapshot(Arc<Snapshot>),
    Notice(Notice),
}

/// Everything a client needs to draw the player. Rebuilt after every input and
/// published only when it differs from the last one (derived, never flagged dirty).
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct Snapshot {
    /// `None` when idle, and while the queue ran dry waiting on a radio refill.
    pub now: Option<Now>,
    pub queue: Vec<QueueRow>,
    /// Index into `queue` of the current entry. A view-only convenience; commands use `EntryId`.
    pub cursor: Option<usize>,
    pub volume: Volume,
    pub radio: RadioStatus,
    pub favorites: Favorites,
}

impl Snapshot {
    pub fn empty() -> Snapshot {
        Snapshot {
            now: None,
            queue: vec![],
            cursor: None,
            volume: Volume::new(0),
            radio: RadioStatus::Off,
            favorites: Favorites::default(),
        }
    }
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct QueueRow {
    pub id: EntryId,
    pub item: Playable,
    pub from_radio: bool,
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub enum Now {
    Track { entry: EntryId, track: Arc<Track>, stage: TrackView },
    Station { entry: EntryId, station: Arc<Station>, stage: StationView },
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub enum TrackView {
    Resolving,
    /// loadfile sent, no audio yet.
    Loading,
    Playing {
        clock: ClockView,
        duration: Option<Duration>,
    },
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub enum StationView {
    Connecting,
    OnAir { listened: ClockView, icy: Option<String> },
}

/// Position sampled when the snapshot was built. Clients extrapolate from their
/// own receive time while `running`, so a 1 Hz snapshot stream draws a smooth
/// bar and a socket client needs no shared clock.
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
pub struct ClockView {
    pub pos: Duration,
    pub running: bool,
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub enum RadioStatus {
    Off,
    /// `dry`: the last refill added nothing, so no refill runs until the current track changes.
    Following {
        seed: VideoId,
        fetching: bool,
        dry: bool,
    },
}

#[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize)]
pub struct Favorites {
    /// Each in the order they were favorited.
    pub tracks: Vec<Arc<Track>>,
    pub stations: Vec<Arc<Station>>,
}

impl Favorites {
    pub fn contains(&self, key: &ItemKey) -> bool {
        match key {
            ItemKey::Video(id) => self.tracks.iter().any(|t| &t.id == id),
            ItemKey::Station(id) => self.stations.iter().any(|s| &s.id == id),
        }
    }
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Notice {
    pub level: Level,
    pub text: String,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum Level {
    Info,
    Warn,
    Error,
}
