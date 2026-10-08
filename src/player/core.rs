//! The player core: one reducer, no I/O.
//!
//!   Core::new(restore)            -> (Core, Vec<Effect>)
//!   core.step(input, now)         -> Vec<Effect>
//!   core.snapshot(now)            -> Snapshot
//!
//! The shell (shell.rs) feeds inputs, runs effects, and publishes snapshots.
//! Tests do the same with no mpv and no network.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::domain::{ItemKey, LoadId, Playable, RefillId, Station, StationId, StreamUrl, Tickets, Track, VideoId, Volume};
use crate::player::api::{Command, Level, Notice, SeekTo, Snapshot};
use crate::player::queue::{EntryId, Queue};

/// Everything that can happen to the core.
#[derive(Debug)]
pub enum Input {
    Command(Command),
    Engine(EngineEvent),
    /// yt-dlp finished. Keyed by id, not ticket: a URL is useful to the cache
    /// whoever asked for it, so a stale resolve is never wasted.
    Resolved { id: VideoId, result: Result<StreamUrl, String> },
    Refilled { ticket: RefillId, result: Result<Vec<Arc<Track>>, String> },
    /// Once a second from the shell. Drives debounced session saves.
    Tick,
}

/// mpv, already attributed by `mpv::attribute`. Every per-file event carries
/// the `LoadId` it belongs to, bound from mpv's `playlist_entry_id`. Pause and
/// volume are absent on purpose: the core owns both and mpv only follows.
#[derive(Debug, Clone, PartialEq)]
pub enum EngineEvent {
    /// First `time-pos > 0` after our `start-file`: audio is flowing.
    Began { load: LoadId },
    /// Throttled to whole-second steps by the attribution.
    Position { load: LoadId, pos: Duration },
    Duration { load: LoadId, duration: Duration },
    Icy { load: LoadId, title: Option<String> },
    /// `end-file` with reason eof or error. `stop` only follows our own
    /// `loadfile replace` and is dropped before it gets here.
    Ended { load: LoadId, reason: EndReason },
    /// mpv died and the shell spawned a fresh one. All LoadIds are dead.
    Restarted,
}

#[derive(Debug, Clone, PartialEq)]
pub enum EndReason {
    Eof,
    Error(String),
}

/// What the core wants done. The shell runs these in order.
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    Engine(EngineCmd),
    /// Run yt-dlp for this id. The cache dedupes, so the shell just spawns.
    Resolve(VideoId),
    Refill { ticket: RefillId, seed: VideoId },
    /// radio-browser `/json/url/<uuid>` click count.
    CountClick(StationId),
    SaveLibrary(Library),
    SaveSession(Session),
    Notify(Notice),
}

#[derive(Debug, Clone, PartialEq)]
pub enum EngineCmd {
    /// Also unpauses: anything newly loaded plays.
    Load { load: LoadId, url: url::Url, start: Option<Duration> },
    SetPause(bool),
    Seek(SeekTo),
    SetVolume(Volume),
    Stop,
    Quit,
}

/// Split by media kind first, because the kinds have different capabilities:
/// only `Track` has a seekable clock and a duration; only a track passes through
/// `Resolving`, which holds no URL and no LoadId. A seek on a station or a load
/// with no URL has nowhere to be written.
#[derive(Debug, Clone, PartialEq)]
pub enum Playback {
    Idle,
    /// The queue ran out while a radio refill is in flight. `refilled` resumes
    /// from here; an empty refill falls back to `Idle`.
    AwaitingRefill,
    Track { entry: EntryId, track: Arc<Track>, stage: TrackStage },
    Station { entry: EntryId, station: Arc<Station>, stage: StationStage },
}

#[derive(Debug, Clone, PartialEq)]
pub enum TrackStage {
    Resolving { attempt: Attempt, start: Option<Duration> },
    Loading { load: LoadId, attempt: Attempt, start: Option<Duration> },
    Playing { load: LoadId, attempt: Attempt, clock: Clock, duration: Option<Duration> },
}

#[derive(Debug, Clone, PartialEq)]
pub enum StationStage {
    Connecting { load: LoadId, attempt: Attempt },
    OnAir { load: LoadId, listened: Clock, icy: Option<String> },
}

/// The whole retry policy: one retry per item, then give up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Attempt {
    First,
    Retry,
}

impl Attempt {
    pub fn next(self) -> Option<Attempt> {
        todo!()
    }
}

/// Position as an anchor, so pause lives in exactly one place and the position
/// at any instant is computed, not stored.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Clock {
    Running { from: Duration, at: Instant },
    Held(Duration),
}

/// Resolves past this many in flight are not started for warming or prefetch.
/// A play always resolves.
pub const RESOLVE_CAP: usize = 5;
/// A failed resolve is not retried by warming or prefetch for this long.
pub const FAILED_BACKOFF: Duration = Duration::from_secs(60);

#[derive(Debug, Clone)]
enum Cached {
    Ready(StreamUrl),
    /// Negative entry, so a dead prefetch does not respawn yt-dlp every step.
    Failed { until: Instant },
}

/// URL cache plus the in-flight set. Owned by the core so the shell never dedupes.
#[derive(Debug, Default)]
pub struct StreamCache {
    entries: HashMap<VideoId, Cached>,
    in_flight: HashSet<VideoId>,
}

/// Favorites. Persisted as library.json by the shell.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Library {
    pub tracks: Vec<Arc<Track>>,
    pub stations: Vec<Arc<Station>>,
    pub radio_atlas_imported: bool,
}

impl Library {
    /// Idempotent. Returns whether anything changed.
    pub fn set(&mut self, item: &Playable, on: bool) -> bool {
        let _ = (item, on);
        todo!()
    }
    /// Idempotent merge by StationId; existing entries win. Returns how many were added.
    pub fn merge_stations(&mut self, stations: Vec<Arc<Station>>) -> usize {
        let _ = stations;
        todo!()
    }
    pub fn contains(&self, key: &ItemKey) -> bool {
        let _ = key;
        todo!()
    }
}

/// What session.json holds, as domain values. `store.rs` owns the file schema.
#[derive(Debug, Clone, PartialEq)]
pub struct Session {
    pub queue: Vec<Playable>,
    pub cursor: Option<usize>,
    pub radio_seed: Option<VideoId>,
    pub volume: Volume,
    pub position: Option<Duration>,
}

pub struct Restore {
    pub library: Library,
    pub session: Option<Session>,
    /// From config.toml, used only when no session exists.
    pub initial_volume: Volume,
}

pub struct Core {
    queue: Queue,
    playback: Playback,
    streams: StreamCache,
    library: Library,
    volume: Volume,
    tickets: Tickets,
    /// Last session written and when. Saves are derived by comparing, not flagged.
    saved_session: Option<(Session, Instant)>,
    /// Position to resume at when the user presses play after a restore.
    resume_at: Option<Duration>,
}

pub const SESSION_DEBOUNCE: Duration = Duration::from_secs(2);
/// `b` past this point restarts the track instead of going back.
pub const RESTART_AFTER: Duration = Duration::from_secs(3);

impl Core {
    pub fn new(restore: Restore) -> (Core, Vec<Effect>) {
        let _ = restore;
        todo!()
    }

    pub fn step(&mut self, input: Input, now: Instant) -> Vec<Effect> {
        let _ = (input, now);
        todo!()
    }

    pub fn snapshot(&self, now: Instant) -> Snapshot {
        let _ = now;
        todo!()
    }
}

#[cfg(test)]
mod tests {}
