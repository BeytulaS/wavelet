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

use crate::domain::{
    ItemKey, LoadId, Playable, RefillId, Station, StationId, StreamUrl, Tickets, Track, VideoId, Volume,
};
use crate::player::api::{
    ClockView, Command, Favorites, Level, Notice, Now, QueueRow, RadioStatus, SeekTo, Snapshot, StationView, TrackView,
    VolumeTo,
};
use crate::player::queue::{EntryId, Fetch, Queue, Removed};

/// Everything that can happen to the core.
#[derive(Debug)]
pub enum Input {
    Command(Command),
    Engine(EngineEvent),
    /// yt-dlp finished. Keyed by id, not ticket: a URL is useful to the cache
    /// whoever asked for it, so a stale resolve is never wasted.
    Resolved {
        id: VideoId,
        result: Result<StreamUrl, String>,
    },
    Refilled {
        ticket: RefillId,
        result: Result<Vec<Arc<Track>>, String>,
    },
    /// Once a second from the shell. Drives debounced session saves.
    Tick,
}

/// mpv, already attributed by `mpv::attribute`. Every per-file event carries
/// the `LoadId` it belongs to, bound from mpv's `playlist_entry_id`. Pause and
/// volume are absent on purpose: the core owns both and mpv only follows.
#[derive(Debug, Clone, PartialEq)]
pub enum EngineEvent {
    /// First `time-pos > 0` after our `start-file`: audio is flowing. Carries
    /// the duration and ICY title mpv reported before that, which no stage
    /// before `Playing`/`OnAir` has room for.
    Began {
        load: LoadId,
        duration: Option<Duration>,
        icy: Option<String>,
    },
    /// Throttled to whole-second steps by the attribution.
    Position {
        load: LoadId,
        pos: Duration,
    },
    Duration {
        load: LoadId,
        duration: Duration,
    },
    Icy {
        load: LoadId,
        title: Option<String>,
    },
    /// `end-file` with reason eof or error. `stop` only follows our own
    /// `loadfile replace` and is dropped before it gets here.
    Ended {
        load: LoadId,
        reason: EndReason,
    },
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
    Refill {
        ticket: RefillId,
        seed: VideoId,
    },
    /// radio-browser `/json/url/<uuid>` click count.
    CountClick(StationId),
    SaveLibrary(Library),
    SaveSession(Session),
    Notify(Notice),
}

#[derive(Debug, Clone, PartialEq)]
pub enum EngineCmd {
    /// Also unpauses: anything newly loaded plays.
    Load {
        load: LoadId,
        url: url::Url,
        start: Option<Duration>,
    },
    SetPause(bool),
    /// Absolute; the core computes relative seeks against its own clock.
    Seek(Duration),
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
    Track {
        entry: EntryId,
        track: Arc<Track>,
        stage: TrackStage,
    },
    Station {
        entry: EntryId,
        station: Arc<Station>,
        stage: StationStage,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum TrackStage {
    Resolving { attempt: Attempt, start: Option<Duration> },
    Loading { load: LoadId, attempt: Attempt, start: Option<Duration> },
    Playing { load: LoadId, clock: Clock, duration: Option<Duration> },
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
        match self {
            Attempt::First => Some(Attempt::Retry),
            Attempt::Retry => None,
        }
    }
}

/// Position as an anchor, so pause lives in exactly one place and the position
/// at any instant is computed, not stored.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Clock {
    Running { from: Duration, at: Instant },
    Held(Duration),
}

impl Clock {
    pub fn at(&self, now: Instant) -> Duration {
        match *self {
            Clock::Running { from, at } => from + now.saturating_duration_since(at),
            Clock::Held(d) => d,
        }
    }
    pub fn running(&self) -> bool {
        matches!(self, Clock::Running { .. })
    }
    fn toggled(self, now: Instant) -> Clock {
        match self {
            Clock::Running { .. } => Clock::Held(self.at(now)),
            Clock::Held(from) => Clock::Running { from, at: now },
        }
    }
    fn reanchor(self, pos: Duration, now: Instant) -> Clock {
        match self {
            Clock::Running { .. } => Clock::Running { from: pos, at: now },
            Clock::Held(_) => Clock::Held(pos),
        }
    }
    fn view(&self, now: Instant) -> ClockView {
        ClockView { pos: self.at(now), running: self.running() }
    }
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
    Failed {
        until: Instant,
    },
}

/// URL cache plus the in-flight set. Owned by the core so the shell never dedupes.
#[derive(Debug, Default)]
pub struct StreamCache {
    entries: HashMap<VideoId, Cached>,
    in_flight: HashSet<VideoId>,
}

impl StreamCache {
    pub fn get(&self, id: &VideoId, now: Instant) -> Option<&StreamUrl> {
        match self.entries.get(id) {
            Some(Cached::Ready(u)) if u.usable_until > now => Some(u),
            _ => None,
        }
    }

    /// Someone is waiting on this id. Ignores the cap and negative entries.
    pub fn want(&mut self, id: &VideoId, now: Instant) -> Option<Effect> {
        if self.get(id, now).is_some() || !self.in_flight.insert(id.clone()) {
            return None;
        }
        Some(Effect::Resolve(id.clone()))
    }

    /// Ahead of need. Respects the cap and negative entries.
    pub fn warm(&mut self, id: &VideoId, now: Instant) -> Option<Effect> {
        let failed = matches!(self.entries.get(id), Some(Cached::Failed { until }) if *until > now);
        if failed || self.in_flight.len() >= RESOLVE_CAP {
            return None;
        }
        self.want(id, now)
    }

    pub fn landed(&mut self, id: &VideoId, result: &Result<StreamUrl, String>, now: Instant) {
        self.in_flight.remove(id);
        self.entries.retain(|_, c| match c {
            Cached::Ready(u) => u.usable_until > now,
            Cached::Failed { until } => *until > now,
        });
        let entry = match result {
            Ok(u) => Cached::Ready(u.clone()),
            Err(_) => Cached::Failed { until: now + FAILED_BACKOFF },
        };
        self.entries.insert(id.clone(), entry);
    }

    pub fn evict(&mut self, id: &VideoId) {
        self.entries.remove(id);
    }
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
        if self.contains(&item.key()) == on {
            return false;
        }
        match (item, on) {
            (Playable::Track(t), true) => self.tracks.push(t.clone()),
            (Playable::Station(s), true) => self.stations.push(s.clone()),
            (Playable::Track(t), false) => self.tracks.retain(|x| x.id != t.id),
            (Playable::Station(s), false) => self.stations.retain(|x| x.id != s.id),
        }
        true
    }

    /// Idempotent merge by StationId; existing entries win. Returns how many were added.
    pub fn merge_stations(&mut self, stations: Vec<Arc<Station>>) -> usize {
        let before = self.stations.len();
        for s in stations {
            if !self.contains(&ItemKey::Station(s.id.clone())) {
                self.stations.push(s);
            }
        }
        self.stations.len() - before
    }

    pub fn contains(&self, key: &ItemKey) -> bool {
        match key {
            ItemKey::Video(id) => self.tracks.iter().any(|t| &t.id == id),
            ItemKey::Station(id) => self.stations.iter().any(|s| &s.id == id),
        }
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
    /// The last saved session without its position, and when it was saved.
    /// Saves are derived by comparing; position alone never triggers one.
    saved: (Session, Option<Instant>),
    /// Position to resume at when the user presses play after a restore.
    resume_at: Option<Duration>,
}

pub const SESSION_DEBOUNCE: Duration = Duration::from_secs(2);
/// `b` past this point restarts the track instead of going back.
pub const RESTART_AFTER: Duration = Duration::from_secs(3);

fn notice(level: Level, text: impl Into<String>) -> Effect {
    Effect::Notify(Notice { level, text: text.into() })
}

impl Core {
    pub fn new(restore: Restore) -> (Core, Vec<Effect>) {
        let mut queue = Queue::default();
        let (volume, resume_at) = match restore.session {
            Some(s) => {
                queue.restore(s.queue, s.cursor, s.radio_seed);
                (s.volume, s.position)
            }
            None => (restore.initial_volume, None),
        };
        let mut core = Core {
            queue,
            playback: Playback::Idle,
            streams: StreamCache::default(),
            library: restore.library,
            volume,
            tickets: Tickets::default(),
            saved: (Session { queue: vec![], cursor: None, radio_seed: None, volume, position: None }, None),
            resume_at,
        };
        core.saved.0 = core.session_key();
        (core, vec![Effect::Engine(EngineCmd::SetVolume(volume))])
    }

    pub fn step(&mut self, input: Input, now: Instant) -> Vec<Effect> {
        let mut fx = Vec::new();
        match input {
            Input::Command(cmd) => self.command(cmd, now, &mut fx),
            Input::Engine(ev) => self.engine(ev, now, &mut fx),
            Input::Resolved { id, result } => self.resolved(id, result, now, &mut fx),
            Input::Refilled { ticket, result } => self.refilled(ticket, result, now, &mut fx),
            Input::Tick => {}
        }
        self.reconcile(now, &mut fx);
        fx
    }

    pub fn snapshot(&self, now: Instant) -> Snapshot {
        let now_view = match &self.playback {
            Playback::Idle | Playback::AwaitingRefill => None,
            Playback::Track { entry, track, stage } => Some(Now::Track {
                entry: *entry,
                track: track.clone(),
                stage: match stage {
                    TrackStage::Resolving { .. } => TrackView::Resolving,
                    TrackStage::Loading { .. } => TrackView::Loading,
                    TrackStage::Playing { clock, duration, .. } => {
                        TrackView::Playing { clock: clock.view(now), duration: *duration }
                    }
                },
            }),
            Playback::Station { entry, station, stage } => Some(Now::Station {
                entry: *entry,
                station: station.clone(),
                stage: match stage {
                    StationStage::Connecting { .. } => StationView::Connecting,
                    StationStage::OnAir { listened, icy, .. } => {
                        StationView::OnAir { listened: listened.view(now), icy: icy.clone() }
                    }
                },
            }),
        };
        let radio = match (self.queue.radio_seed(), self.queue.fetch()) {
            (Some(seed), Some(fetch)) => RadioStatus::Following {
                seed: seed.clone(),
                fetching: matches!(fetch, Fetch::InFlight { .. }),
                dry: matches!(fetch, Fetch::Dry { .. }),
            },
            _ => RadioStatus::Off,
        };
        Snapshot {
            now: now_view,
            queue: self
                .queue
                .entries()
                .iter()
                .map(|e| QueueRow { id: e.id, item: e.item.clone(), from_radio: e.from_radio })
                .collect(),
            cursor: self.queue.cursor_index(),
            volume: self.volume,
            radio,
            favorites: Favorites { tracks: self.library.tracks.clone(), stations: self.library.stations.clone() },
        }
    }

    fn audible(&self) -> bool {
        matches!(
            self.playback,
            Playback::Track { stage: TrackStage::Playing { .. }, .. }
                | Playback::Station { stage: StationStage::OnAir { .. }, .. }
        )
    }

    fn command(&mut self, cmd: Command, now: Instant, fx: &mut Vec<Effect>) {
        match cmd {
            Command::PlayTrack(track) => {
                let seed = track.id.clone();
                self.queue.replace(vec![Playable::Track(track)], 0, Some(seed));
                self.start_current(None, now, fx);
            }
            Command::PlayStations { stations, start } => {
                let at = stations.iter().position(|s| s.id == start).unwrap_or(0);
                self.queue.replace(stations.into_iter().map(Playable::Station).collect(), at, None);
                self.start_current(None, now, fx);
            }
            Command::PlayNext(item) => {
                fx.push(notice(Level::Info, format!("plays next: {}", item.title())));
                let id = self.queue.insert_next(item);
                if matches!(self.playback, Playback::Idle | Playback::AwaitingRefill) {
                    self.queue.jump(id);
                    self.start_current(None, now, fx);
                }
            }
            Command::Next => {
                if !matches!(self.playback, Playback::Idle) {
                    self.skip(false, now, fx);
                } else if self.queue.advance().is_some() {
                    self.start_current(None, now, fx);
                }
            }
            Command::Prev => {
                if let Playback::Track { stage: TrackStage::Playing { clock, .. }, .. } = &mut self.playback
                    && clock.at(now) > RESTART_AFTER
                {
                    *clock = clock.reanchor(Duration::ZERO, now);
                    fx.push(Effect::Engine(EngineCmd::Seek(Duration::ZERO)));
                } else if self.queue.retreat().is_some() {
                    self.start_current(None, now, fx);
                }
            }
            Command::Jump(id) => {
                if self.queue.jump(id).is_some() {
                    self.start_current(None, now, fx);
                }
            }
            Command::Remove(id) => {
                let was_active = !matches!(self.playback, Playback::Idle);
                if self.queue.remove(id) == Removed::Current && was_active {
                    self.start_current(None, now, fx);
                }
            }
            Command::TogglePause => self.toggle_pause(now, fx),
            Command::SetPaused(p) => {
                let running = match &self.playback {
                    Playback::Track { stage: TrackStage::Playing { clock, .. }, .. } => clock.running(),
                    Playback::Station { stage: StationStage::OnAir { listened, .. }, .. } => listened.running(),
                    Playback::Idle => false,
                    _ => return,
                };
                if running == p {
                    self.toggle_pause(now, fx);
                }
            }
            Command::Seek(to) => {
                // The kind split makes a track's Playing stage the only place a seek can land.
                if let Playback::Track { stage: TrackStage::Playing { clock, duration, .. }, .. } = &mut self.playback {
                    let cur = clock.at(now);
                    let target = match to {
                        SeekTo::By(s) if s < 0 => cur.saturating_sub(Duration::from_secs(s.unsigned_abs().into())),
                        SeekTo::By(s) => cur + Duration::from_secs(s.unsigned_abs().into()),
                        SeekTo::At(d) => d,
                    };
                    let target = duration.map_or(target, |d| target.min(d));
                    *clock = clock.reanchor(target, now);
                    fx.push(Effect::Engine(EngineCmd::Seek(target)));
                }
            }
            Command::Volume(to) => {
                self.volume = match to {
                    VolumeTo::By(d) => self.volume.nudge(d),
                    VolumeTo::At(v) => v,
                };
                fx.push(Effect::Engine(EngineCmd::SetVolume(self.volume)));
            }
            Command::SetFavorite { item, on } => {
                if self.library.set(&item, on) {
                    fx.push(Effect::SaveLibrary(self.library.clone()));
                }
            }
            Command::Warm(ids) => {
                for id in ids {
                    fx.extend(self.streams.warm(&id, now));
                }
            }
            Command::Shutdown => {
                self.save_session(now, fx);
                fx.push(Effect::Engine(EngineCmd::Quit));
            }
        }
    }

    fn toggle_pause(&mut self, now: Instant, fx: &mut Vec<Effect>) {
        let clock = match &mut self.playback {
            Playback::Track { stage: TrackStage::Playing { clock, .. }, .. } => clock,
            Playback::Station { stage: StationStage::OnAir { listened, .. }, .. } => listened,
            Playback::Idle if self.queue.current().is_some() => {
                let start = self.resume_at.take();
                self.start_current(start, now, fx);
                return;
            }
            _ => return,
        };
        *clock = clock.toggled(now);
        fx.push(Effect::Engine(EngineCmd::SetPause(!clock.running())));
    }

    /// Begin playing whatever the queue cursor points at. The single entry point
    /// for "something new should play"; every command funnels here.
    fn start_current(&mut self, start: Option<Duration>, now: Instant, fx: &mut Vec<Effect>) {
        self.resume_at = None;
        let Some(entry) = self.queue.current().cloned() else {
            self.stop(fx);
            return;
        };
        self.playback = match entry.item {
            Playable::Track(track) => {
                let stage = match self.streams.get(&track.id, now) {
                    Some(u) => {
                        let load = self.tickets.issue();
                        fx.push(Effect::Engine(EngineCmd::Load { load, url: u.url.clone(), start }));
                        TrackStage::Loading { load, attempt: Attempt::First, start }
                    }
                    None => {
                        fx.extend(self.streams.want(&track.id, now));
                        TrackStage::Resolving { attempt: Attempt::First, start }
                    }
                };
                Playback::Track { entry: entry.id, track, stage }
            }
            Playable::Station(station) => {
                let load = self.tickets.issue();
                fx.push(Effect::Engine(EngineCmd::Load { load, url: station.stream.clone(), start: None }));
                fx.push(Effect::CountClick(station.id.clone()));
                let stage = StationStage::Connecting { load, attempt: Attempt::First };
                Playback::Station { entry: entry.id, station, stage }
            }
        };
    }

    fn stop(&mut self, fx: &mut Vec<Effect>) {
        if matches!(self.playback, Playback::Track { .. } | Playback::Station { .. }) {
            fx.push(Effect::Engine(EngineCmd::Stop));
        }
        self.playback = Playback::Idle;
    }

    /// Advance and start. At the end of the queue, wait for a radio refill that
    /// is due or in flight; otherwise stop if `stop_at_end`, or keep playing.
    fn skip(&mut self, stop_at_end: bool, now: Instant, fx: &mut Vec<Effect>) {
        if self.queue.advance().is_some() {
            self.start_current(None, now, fx);
        } else if matches!(self.queue.fetch(), Some(Fetch::InFlight { .. })) || self.queue.refill_wanted().is_some() {
            self.stop(fx);
            self.playback = Playback::AwaitingRefill;
        } else if stop_at_end {
            self.stop(fx);
        }
    }

    fn resolved(&mut self, id: VideoId, result: Result<StreamUrl, String>, now: Instant, fx: &mut Vec<Effect>) {
        self.streams.landed(&id, &result, now);
        let Playback::Track { track, stage, .. } = &mut self.playback else { return };
        let TrackStage::Resolving { attempt, start } = *stage else { return };
        if track.id != id {
            return;
        }
        match result {
            Ok(u) => {
                let load = self.tickets.issue();
                fx.push(Effect::Engine(EngineCmd::Load { load, url: u.url, start }));
                *stage = TrackStage::Loading { load, attempt, start };
            }
            Err(e) => self.fail_current(e, now, fx),
        }
    }

    /// The one place a failure is handled: retry per `Attempt`, then skip a
    /// track or stop a station. A station list never auto-advances, because a
    /// listener who picked one station wants that station.
    fn fail_current(&mut self, reason: String, now: Instant, fx: &mut Vec<Effect>) {
        match self.playback.clone() {
            Playback::Track { entry, track, stage } => {
                let (attempt, start) = match stage {
                    TrackStage::Resolving { attempt, start } | TrackStage::Loading { attempt, start, .. } => {
                        (attempt, start)
                    }
                    TrackStage::Playing { clock, .. } => (Attempt::First, Some(clock.at(now))),
                };
                match attempt.next() {
                    Some(attempt) => {
                        // YouTube URLs are bound to the requesting IP; a VPN hop kills them.
                        self.streams.evict(&track.id);
                        fx.extend(self.streams.want(&track.id, now));
                        let stage = TrackStage::Resolving { attempt, start };
                        self.playback = Playback::Track { entry, track, stage };
                    }
                    None => {
                        fx.push(notice(Level::Error, format!("skipped {}: {reason}", track.title)));
                        self.skip(true, now, fx);
                    }
                }
            }
            Playback::Station { entry, station, stage } => {
                let attempt = match stage {
                    StationStage::Connecting { attempt, .. } => attempt,
                    StationStage::OnAir { .. } => Attempt::First,
                };
                match attempt.next() {
                    Some(attempt) => {
                        let load = self.tickets.issue();
                        fx.push(Effect::Engine(EngineCmd::Load { load, url: station.stream.clone(), start: None }));
                        let stage = StationStage::Connecting { load, attempt };
                        self.playback = Playback::Station { entry, station, stage };
                    }
                    None => {
                        fx.push(notice(Level::Error, format!("{} is off the air: {reason}", station.name)));
                        self.stop(fx);
                    }
                }
            }
            Playback::Idle | Playback::AwaitingRefill => {}
        }
    }

    fn refilled(
        &mut self,
        ticket: RefillId,
        result: Result<Vec<Arc<Track>>, String>,
        now: Instant,
        fx: &mut Vec<Effect>,
    ) {
        let (tracks, err) = match result {
            Ok(t) => (Some(t), None),
            Err(e) => (None, Some(e)),
        };
        let Some(added) = self.queue.refill_landed(ticket, tracks) else { return };
        if let Some(e) = err {
            fx.push(notice(Level::Warn, format!("radio refill failed: {e}")));
        }
        if self.playback == Playback::AwaitingRefill {
            if added > 0 {
                self.skip(true, now, fx);
            } else {
                self.playback = Playback::Idle;
            }
        }
    }

    fn engine(&mut self, ev: EngineEvent, now: Instant, fx: &mut Vec<Effect>) {
        match (ev, &mut self.playback) {
            (EngineEvent::Began { load, duration, .. }, Playback::Track { track, stage, .. }) => {
                if let TrackStage::Loading { load: l, start, .. } = *stage
                    && l == load
                {
                    let clock = Clock::Running { from: start.unwrap_or_default(), at: now };
                    *stage = TrackStage::Playing { load, clock, duration: duration.or(track.duration) };
                }
            }
            (EngineEvent::Began { load, icy, .. }, Playback::Station { stage, .. }) => {
                if let StationStage::Connecting { load: l, .. } = *stage
                    && l == load
                {
                    *stage =
                        StationStage::OnAir { load, listened: Clock::Running { from: Duration::ZERO, at: now }, icy };
                }
            }
            (
                EngineEvent::Position { load, pos },
                Playback::Track { stage: TrackStage::Playing { load: l, clock, .. }, .. },
            ) if *l == load => {
                *clock = clock.reanchor(pos, now);
            }
            (
                EngineEvent::Duration { load, duration },
                Playback::Track { stage: TrackStage::Playing { load: l, duration: d, .. }, .. },
            ) if *l == load => *d = Some(duration),
            (
                EngineEvent::Icy { load, title },
                Playback::Station { stage: StationStage::OnAir { load: l, icy, .. }, .. },
            ) if *l == load => {
                *icy = title;
            }
            (EngineEvent::Ended { load, reason }, _) => self.ended(load, reason, now, fx),
            (EngineEvent::Restarted, _) => self.restarted(now, fx),
            _ => {}
        }
    }

    fn ended(&mut self, load: LoadId, reason: EndReason, now: Instant, fx: &mut Vec<Effect>) {
        // Only the live LoadId of the current stage gets past this; an end-file
        // for an older load is stale by construction.
        let live = match &self.playback {
            Playback::Track {
                stage: TrackStage::Loading { load: l, .. } | TrackStage::Playing { load: l, .. },
                ..
            }
            | Playback::Station {
                stage: StationStage::Connecting { load: l, .. } | StationStage::OnAir { load: l, .. },
                ..
            } => *l,
            _ => return,
        };
        if live != load {
            return;
        }
        match (reason, &self.playback) {
            (EndReason::Eof, Playback::Track { .. }) => self.skip(true, now, fx),
            (EndReason::Eof, _) => self.fail_current("the stream ended".into(), now, fx),
            (EndReason::Error(e), _) => self.fail_current(e, now, fx),
        }
    }

    /// mpv was respawned. Idempotent: reload whatever should be audible.
    fn restarted(&mut self, now: Instant, fx: &mut Vec<Effect>) {
        fx.push(Effect::Engine(EngineCmd::SetVolume(self.volume)));
        let start = match &self.playback {
            Playback::Track { stage: TrackStage::Playing { clock, .. }, .. } => Some(clock.at(now)),
            Playback::Track { stage: TrackStage::Loading { start, .. }, .. } => *start,
            Playback::Station { .. } => None,
            Playback::Track { stage: TrackStage::Resolving { .. }, .. } | Playback::Idle | Playback::AwaitingRefill => {
                return;
            }
        };
        self.start_current(start, now, fx);
    }

    /// Derived background work. Runs after every step; each branch is a no-op
    /// when its work is already done or in flight, so running it twice is harmless.
    fn reconcile(&mut self, now: Instant, fx: &mut Vec<Effect>) {
        if self.audible()
            && let Some(Playable::Track(next)) = self.queue.upcoming().next().map(|e| &e.item)
        {
            let id = next.id.clone();
            fx.extend(self.streams.warm(&id, now));
        }
        if !matches!(self.playback, Playback::Idle)
            && let Some(seed) = self.queue.refill_wanted()
        {
            let ticket = self.tickets.issue();
            self.queue.refill_started(ticket, seed.clone());
            fx.push(Effect::Refill { ticket, seed });
        }
        let debounced = self.saved.1.is_none_or(|at| now.saturating_duration_since(at) >= SESSION_DEBOUNCE);
        if debounced && self.session_key() != self.saved.0 {
            self.save_session(now, fx);
        }
    }

    fn save_session(&mut self, now: Instant, fx: &mut Vec<Effect>) {
        let position = match &self.playback {
            Playback::Track { stage: TrackStage::Playing { clock, .. }, .. } => Some(clock.at(now)),
            Playback::Idle => self.resume_at,
            _ => None,
        };
        let key = self.session_key();
        fx.push(Effect::SaveSession(Session { position, ..key.clone() }));
        self.saved = (key, Some(now));
    }

    fn session_key(&self) -> Session {
        Session {
            queue: self.queue.entries().iter().map(|e| e.item.clone()).collect(),
            cursor: self.queue.cursor_index(),
            radio_seed: self.queue.radio_seed().cloned(),
            volume: self.volume,
            position: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::player::queue::tests::{station, track, vid};

    fn core() -> Core {
        Core::new(Restore { library: Library::default(), session: None, initial_volume: Volume::new(70) }).0
    }

    fn id(c: char) -> VideoId {
        VideoId::parse(&vid(c)).expect("test id")
    }

    fn url(c: char, t0: Instant) -> Result<StreamUrl, String> {
        let url = url::Url::parse(&format!("https://media.test/{c}")).expect("test url");
        Ok(StreamUrl { url, usable_until: t0 + Duration::from_secs(3600) })
    }

    fn cmd(core: &mut Core, c: Command, now: Instant) -> Vec<Effect> {
        quiet(core.step(Input::Command(c), now))
    }

    fn input(core: &mut Core, i: Input, now: Instant) -> Vec<Effect> {
        quiet(core.step(i, now))
    }

    /// Drops session saves, which every queue change emits and no test here is about.
    fn quiet(fx: Vec<Effect>) -> Vec<Effect> {
        fx.into_iter().filter(|e| !matches!(e, Effect::SaveSession(_))).collect()
    }

    fn load_of(fx: &[Effect]) -> LoadId {
        fx.iter()
            .find_map(|e| match e {
                Effect::Engine(EngineCmd::Load { load, .. }) => Some(*load),
                _ => None,
            })
            .expect("a Load effect")
    }

    fn loaded_url(fx: &[Effect]) -> Option<String> {
        fx.iter().find_map(|e| match e {
            Effect::Engine(EngineCmd::Load { url, .. }) => Some(url.to_string()),
            _ => None,
        })
    }

    fn refill_ticket(fx: &[Effect]) -> Option<RefillId> {
        fx.iter().find_map(|e| match e {
            Effect::Refill { ticket, .. } => Some(*ticket),
            _ => None,
        })
    }

    fn began(load: LoadId) -> Input {
        Input::Engine(EngineEvent::Began { load, duration: None, icy: None })
    }

    fn now_title(core: &Core, t: Instant) -> Option<String> {
        core.snapshot(t).now.map(|n| match n {
            Now::Track { track, .. } => track.title.clone(),
            Now::Station { station, .. } => station.name.clone(),
        })
    }

    /// Plays `a` with radio refill `rest`, through to first audio. Returns the load.
    fn playing(core: &mut Core, rest: &[char], t0: Instant) -> LoadId {
        let fx = cmd(core, Command::PlayTrack(track(&vid('a'))), t0);
        let ticket = refill_ticket(&fx).expect("refill on play");
        let tracks = rest.iter().map(|c| track(&vid(*c))).collect();
        input(core, Input::Refilled { ticket, result: Ok(tracks) }, t0);
        let fx = input(core, Input::Resolved { id: id('a'), result: url('a', t0) }, t0);
        let load = load_of(&fx);
        input(core, began(load), t0);
        load
    }

    #[test]
    fn play_track_with_a_cold_cache_resolves_and_refills() {
        let t0 = Instant::now();
        let mut c = core();
        let fx = cmd(&mut c, Command::PlayTrack(track(&vid('a'))), t0);
        assert_eq!(fx.len(), 2, "{fx:?}");
        assert_eq!(fx[0], Effect::Resolve(id('a')));
        assert!(matches!(&fx[1], Effect::Refill { seed, .. } if *seed == id('a')));
        assert!(matches!(c.snapshot(t0).now, Some(Now::Track { stage: TrackView::Resolving, .. })));
    }

    #[test]
    fn a_resolve_for_another_track_only_fills_the_cache() {
        let t0 = Instant::now();
        let mut c = core();
        cmd(&mut c, Command::PlayTrack(track(&vid('a'))), t0);
        let fx = input(&mut c, Input::Resolved { id: id('b'), result: url('b', t0) }, t0);
        assert!(fx.is_empty(), "{fx:?}");
        assert!(matches!(c.snapshot(t0).now, Some(Now::Track { stage: TrackView::Resolving, .. })));
        let fx = cmd(&mut c, Command::PlayTrack(track(&vid('b'))), t0);
        assert_eq!(loaded_url(&fx).as_deref(), Some("https://media.test/b"), "cache hit loads at once");
        assert!(!fx.contains(&Effect::Resolve(id('b'))));
    }

    #[test]
    fn an_end_file_for_an_old_load_is_ignored() {
        let t0 = Instant::now();
        let mut c = core();
        let old = playing(&mut c, &['b', 'c', 'd', 'e'], t0);
        input(&mut c, Input::Resolved { id: id('b'), result: url('b', t0) }, t0);
        let fx = cmd(&mut c, Command::Next, t0);
        let new = load_of(&fx);
        for reason in [EndReason::Eof, EndReason::Error("stop".into())] {
            let fx = input(&mut c, Input::Engine(EngineEvent::Ended { load: old, reason }), t0);
            assert!(fx.is_empty(), "{fx:?}");
        }
        assert_eq!(now_title(&c, t0), Some(format!("title {}", vid('b'))));
        input(&mut c, began(new), t0);
        assert!(matches!(c.snapshot(t0).now, Some(Now::Track { stage: TrackView::Playing { .. }, .. })));
    }

    #[test]
    fn the_next_track_is_prefetched_only_after_first_audio() {
        let t0 = Instant::now();
        let mut c = core();
        let fx = cmd(&mut c, Command::PlayTrack(track(&vid('a'))), t0);
        let ticket = refill_ticket(&fx).expect("refill");
        let fx = input(&mut c, Input::Refilled { ticket, result: Ok(vec![track(&vid('b'))]) }, t0);
        assert!(!fx.contains(&Effect::Resolve(id('b'))), "resolving: no prefetch yet");
        let fx = input(&mut c, Input::Resolved { id: id('a'), result: url('a', t0) }, t0);
        assert!(!fx.contains(&Effect::Resolve(id('b'))), "loading: no prefetch yet");
        let fx = input(&mut c, began(load_of(&fx)), t0);
        assert!(fx.contains(&Effect::Resolve(id('b'))), "{fx:?}");
    }

    #[test]
    fn refill_runs_at_three_left_and_an_empty_refill_does_not_loop() {
        let t0 = Instant::now();
        let mut c = core();
        playing(&mut c, &['b', 'c', 'd', 'e'], t0);
        let fx = input(&mut c, Input::Tick, t0);
        assert_eq!(refill_ticket(&fx), None, "4 upcoming");
        input(&mut c, Input::Resolved { id: id('b'), result: url('b', t0) }, t0);
        let fx = cmd(&mut c, Command::Next, t0);
        let ticket = refill_ticket(&fx).expect("3 upcoming triggers a refill");
        let fx = input(&mut c, Input::Refilled { ticket, result: Ok(vec![track(&vid('c'))]) }, t0);
        assert_eq!(refill_ticket(&fx), None, "{fx:?}");
        for _ in 0..3 {
            assert_eq!(refill_ticket(&input(&mut c, Input::Tick, t0)), None);
        }
        assert!(matches!(c.snapshot(t0).radio, RadioStatus::Following { dry: true, .. }));
    }

    #[test]
    fn the_queue_running_dry_waits_for_the_refill_in_flight() {
        let t0 = Instant::now();
        let mut c = core();
        let fx = cmd(&mut c, Command::PlayTrack(track(&vid('a'))), t0);
        let ticket = refill_ticket(&fx).expect("refill");
        let load = load_of(&input(&mut c, Input::Resolved { id: id('a'), result: url('a', t0) }, t0));
        input(&mut c, began(load), t0);
        input(&mut c, Input::Engine(EngineEvent::Ended { load, reason: EndReason::Eof }), t0);
        assert_eq!(c.snapshot(t0).now, None);
        let fx = input(&mut c, Input::Refilled { ticket, result: Ok(vec![track(&vid('b'))]) }, t0);
        assert!(fx.contains(&Effect::Resolve(id('b'))), "{fx:?}");
        assert_eq!(now_title(&c, t0), Some(format!("title {}", vid('b'))));
    }

    #[test]
    fn a_track_load_error_retries_once_then_skips() {
        let t0 = Instant::now();
        let mut c = core();
        playing(&mut c, &['b', 'c', 'd', 'e'], t0);
        input(&mut c, Input::Resolved { id: id('b'), result: url('b', t0) }, t0);
        let l1 = load_of(&cmd(&mut c, Command::Next, t0));
        let fx =
            input(&mut c, Input::Engine(EngineEvent::Ended { load: l1, reason: EndReason::Error("403".into()) }), t0);
        assert!(fx.contains(&Effect::Resolve(id('b'))), "evicts and re-resolves: {fx:?}");
        let l2 = load_of(&input(&mut c, Input::Resolved { id: id('b'), result: url('b', t0) }, t0));
        let fx =
            input(&mut c, Input::Engine(EngineEvent::Ended { load: l2, reason: EndReason::Error("403".into()) }), t0);
        assert!(fx.iter().any(|e| matches!(e, Effect::Notify(n) if n.level == Level::Error)), "{fx:?}");
        assert!(fx.contains(&Effect::Resolve(id('c'))), "skips to c: {fx:?}");
        assert_eq!(now_title(&c, t0), Some(format!("title {}", vid('c'))));
    }

    #[test]
    fn a_resolve_error_retries_once_then_skips() {
        let t0 = Instant::now();
        let mut c = core();
        cmd(&mut c, Command::PlayTrack(track(&vid('a'))), t0);
        cmd(&mut c, Command::PlayNext(Playable::Track(track(&vid('b')))), t0);
        let fx = input(&mut c, Input::Resolved { id: id('a'), result: Err("gone".into()) }, t0);
        assert_eq!(fx, vec![Effect::Resolve(id('a'))]);
        let fx = input(&mut c, Input::Resolved { id: id('a'), result: Err("gone".into()) }, t0);
        assert!(fx.contains(&Effect::Resolve(id('b'))), "{fx:?}");
    }

    #[test]
    fn a_station_that_fails_twice_stops_without_advancing() {
        let t0 = Instant::now();
        let mut c = core();
        let (s1, s2) = (station(1), station(2));
        let fx = cmd(&mut c, Command::PlayStations { stations: vec![s1.clone(), s2], start: s1.id.clone() }, t0);
        assert!(fx.contains(&Effect::CountClick(s1.id.clone())));
        let l1 = load_of(&fx);
        let err = |load| Input::Engine(EngineEvent::Ended { load, reason: EndReason::Error("refused".into()) });
        let fx = input(&mut c, err(l1), t0);
        assert_eq!(loaded_url(&fx).as_deref(), Some("http://radio.test/1"), "one retry of the same station");
        let fx = input(&mut c, err(load_of(&fx)), t0);
        assert!(fx.contains(&Effect::Engine(EngineCmd::Stop)), "{fx:?}");
        assert!(loaded_url(&fx).is_none(), "never advances to station 2");
        assert_eq!(c.snapshot(t0).now, None);
    }

    #[test]
    fn prev_restarts_past_three_seconds_and_goes_back_before() {
        let t0 = Instant::now();
        let mut c = core();
        playing(&mut c, &['b', 'c', 'd', 'e'], t0);
        input(&mut c, Input::Resolved { id: id('b'), result: url('b', t0) }, t0);
        let lb = load_of(&cmd(&mut c, Command::Next, t0));
        input(&mut c, began(lb), t0);
        let fx = cmd(&mut c, Command::Prev, t0 + Duration::from_secs(5));
        assert_eq!(fx, vec![Effect::Engine(EngineCmd::Seek(Duration::ZERO))]);
        assert_eq!(now_title(&c, t0), Some(format!("title {}", vid('b'))));
        let fx = cmd(&mut c, Command::Prev, t0 + Duration::from_secs(6));
        assert_eq!(loaded_url(&fx).as_deref(), Some("https://media.test/a"));
    }

    #[test]
    fn play_next_does_not_interrupt_and_plays_after_the_current_entry() {
        let t0 = Instant::now();
        let mut c = core();
        playing(&mut c, &['b', 'c'], t0);
        let fx = cmd(&mut c, Command::PlayNext(Playable::Track(track(&vid('x')))), t0);
        assert!(loaded_url(&fx).is_none(), "{fx:?}");
        assert!(fx.contains(&Effect::Resolve(id('x'))), "it is next, so it is prefetched: {fx:?}");
        assert_eq!(now_title(&c, t0), Some(format!("title {}", vid('a'))));
        cmd(&mut c, Command::Next, t0);
        assert_eq!(now_title(&c, t0), Some(format!("title {}", vid('x'))));
    }

    #[test]
    fn removing_the_current_entry_starts_its_follower() {
        let t0 = Instant::now();
        let mut c = core();
        playing(&mut c, &['b', 'c'], t0);
        let current = c.snapshot(t0).queue[0].id;
        cmd(&mut c, Command::Remove(current), t0);
        let snap = c.snapshot(t0);
        assert_eq!(now_title(&c, t0), Some(format!("title {}", vid('b'))));
        assert!(snap.queue.iter().all(|r| r.id != current));
    }

    /// A station's `StationStage` has no clock to seek and `Command::Seek`
    /// matches only `TrackStage::Playing`, so a station seek cannot be written.
    #[test]
    fn seek_moves_a_playing_track_and_does_nothing_on_a_station() {
        let t0 = Instant::now();
        let mut c = core();
        playing(&mut c, &[], t0);
        let fx = cmd(&mut c, Command::Seek(SeekTo::By(5)), t0 + Duration::from_secs(10));
        assert_eq!(fx, vec![Effect::Engine(EngineCmd::Seek(Duration::from_secs(15)))]);

        let s = station(1);
        let load = load_of(&cmd(&mut c, Command::PlayStations { stations: vec![s.clone()], start: s.id.clone() }, t0));
        input(&mut c, began(load), t0);
        assert!(cmd(&mut c, Command::Seek(SeekTo::By(5)), t0).is_empty());
        assert!(cmd(&mut c, Command::Seek(SeekTo::At(Duration::from_secs(30))), t0).is_empty());
    }

    #[test]
    fn warming_stops_at_five_in_flight_and_skips_recent_failures() {
        let t0 = Instant::now();
        let mut c = core();
        let ids: Vec<VideoId> = "abcdefg".chars().map(id).collect();
        let fx = cmd(&mut c, Command::Warm(ids.clone()), t0);
        assert_eq!(fx.len(), RESOLVE_CAP, "{fx:?}");
        input(&mut c, Input::Resolved { id: id('a'), result: Err("nope".into()) }, t0);
        let fx = cmd(&mut c, Command::Warm(ids), t0);
        assert_eq!(fx, vec![Effect::Resolve(id('f'))], "a negative entry is not warmed again");
        let fx = cmd(&mut c, Command::PlayTrack(track(&vid('a'))), t0);
        assert!(fx.contains(&Effect::Resolve(id('a'))), "a play ignores the cap and the negative entry");
    }

    #[test]
    fn pause_lives_in_the_core_clock() {
        let t0 = Instant::now();
        let mut c = core();
        playing(&mut c, &[], t0);
        let fx = cmd(&mut c, Command::TogglePause, t0 + Duration::from_secs(4));
        assert_eq!(fx, vec![Effect::Engine(EngineCmd::SetPause(true))]);
        let Some(Now::Track { stage: TrackView::Playing { clock, .. }, .. }) =
            c.snapshot(t0 + Duration::from_secs(9)).now
        else {
            panic!("playing")
        };
        assert_eq!(clock, ClockView { pos: Duration::from_secs(4), running: false });
        assert!(cmd(&mut c, Command::SetPaused(true), t0).is_empty(), "absolute pause converges");
    }

    #[test]
    fn a_restored_session_waits_for_play_and_resumes_at_its_position() {
        let t0 = Instant::now();
        let session = Session {
            queue: vec![Playable::Track(track(&vid('a')))],
            cursor: Some(0),
            radio_seed: None,
            volume: Volume::new(40),
            position: Some(Duration::from_secs(42)),
        };
        let (mut c, fx) =
            Core::new(Restore { library: Library::default(), session: Some(session), initial_volume: Volume::new(70) });
        assert_eq!(fx, vec![Effect::Engine(EngineCmd::SetVolume(Volume::new(40)))]);
        assert!(c.step(Input::Tick, t0).is_empty(), "no autoplay and no save of an unchanged session");
        cmd(&mut c, Command::TogglePause, t0);
        let fx = input(&mut c, Input::Resolved { id: id('a'), result: url('a', t0) }, t0);
        assert!(
            fx.iter().any(|e| matches!(e, Effect::Engine(EngineCmd::Load { start: Some(s), .. }) if s.as_secs() == 42))
        );
    }

    #[test]
    fn session_saves_are_debounced_and_shutdown_saves_the_position() {
        let t0 = Instant::now();
        let mut c = core();
        let saves = |fx: &[Effect]| fx.iter().filter(|e| matches!(e, Effect::SaveSession(_))).count();
        assert_eq!(saves(&c.step(Input::Command(Command::PlayTrack(track(&vid('a')))), t0)), 1);
        assert_eq!(saves(&c.step(Input::Command(Command::Volume(VolumeTo::By(5))), t0)), 0, "within 2 s");
        assert_eq!(saves(&c.step(Input::Tick, t0 + SESSION_DEBOUNCE)), 1, "the pending change lands later");
        assert_eq!(saves(&c.step(Input::Tick, t0 + SESSION_DEBOUNCE * 3)), 0, "nothing changed");
        let fx = c.step(Input::Command(Command::Shutdown), t0 + SESSION_DEBOUNCE * 3);
        assert_eq!(saves(&fx), 1);
        assert_eq!(fx.last(), Some(&Effect::Engine(EngineCmd::Quit)));
    }

    #[test]
    fn set_favorite_is_absolute() {
        let t0 = Instant::now();
        let mut c = core();
        let item = Playable::Station(station(3));
        assert_eq!(cmd(&mut c, Command::SetFavorite { item: item.clone(), on: true }, t0).len(), 1);
        assert!(cmd(&mut c, Command::SetFavorite { item: item.clone(), on: true }, t0).is_empty());
        assert!(c.snapshot(t0).favorites.contains(&item.key()));
    }

    #[test]
    fn restarted_reloads_the_current_track_at_its_position() {
        let t0 = Instant::now();
        let mut c = core();
        playing(&mut c, &[], t0);
        let fx = input(&mut c, Input::Engine(EngineEvent::Restarted), t0 + Duration::from_secs(30));
        assert!(
            fx.iter().any(|e| matches!(e, Effect::Engine(EngineCmd::Load { start: Some(s), .. }) if s.as_secs() == 30))
        );
    }
}
