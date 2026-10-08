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
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Command {
    /// Enter on a YouTube result: queue becomes `[track]`, autoplay radio seeded by it.
    PlayTrack(Arc<Track>),
    /// Enter on a station: queue becomes the whole list, cursor on `start`, no radio.
    PlayStations { stations: Vec<Arc<Station>>, start: StationId },
    /// `a`: insert right after the cursor. Starts it if nothing is playing.
    PlayNext(Playable),
    Next,
    Prev,
    Jump(EntryId),
    Remove(EntryId),
    TogglePause,
    /// Ignored unless a track is playing. Stations have no seek; see `core::Playback`.
    Seek(SeekTo),
    Volume(VolumeTo),
    /// Absolute, so a double-delivered or retried command converges.
    SetFavorite { item: Playable, on: bool },
    /// Resolve these ahead of time (top search results). Lowest priority.
    Prewarm(Vec<VideoId>),
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
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Out {
    Snapshot(Arc<Snapshot>),
    Notice(Notice),
}

/// Everything a client needs to draw the player. Rebuilt after every input and
/// published only when it differs from the last one (derived, never flagged dirty).
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct Snapshot {
    pub now: Option<NowPlaying>,
    pub queue: Vec<QueueRow>,
    /// Index into `queue` of the current entry. A view-only convenience; commands use `EntryId`.
    pub cursor: Option<usize>,
    pub volume: Volume,
    pub radio: RadioStatus,
    pub favorites: Favorites,
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct QueueRow {
    pub id: EntryId,
    pub item: Playable,
    pub from_radio: bool,
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct NowPlaying {
    pub entry: EntryId,
    pub item: Playable,
    pub status: Status,
    pub clock: ClockView,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum Status {
    /// Track only: waiting on yt-dlp.
    Resolving,
    /// loadfile sent, no audio yet.
    Buffering,
    Playing,
    Paused,
}

/// Position sampled when the snapshot was built. Clients extrapolate with their
/// own receive time while `running`, so the progress bar moves at 30 fps from a
/// 1 Hz snapshot stream and a socket client needs no shared clock.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub enum ClockView {
    Track { pos: Duration, duration: Option<Duration>, running: bool },
    Live { listened: Duration, running: bool, icy_title: Option<String> },
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub enum RadioStatus {
    Off,
    Following { seed: VideoId, fetching: bool },
}

#[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize)]
pub struct Favorites {
    pub tracks: Vec<Arc<Track>>,
    pub stations: Vec<Arc<Station>>,
}

impl Favorites {
    pub fn contains(&self, key: &ItemKey) -> bool {
        todo!("linear scan; favorites are tens of items")
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
