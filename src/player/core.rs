//! The player core: one reducer, no I/O.
//!
//!   Core::new(restore)            -> (Core, Vec<Effect>)
//!   core.step(input, now)         -> Vec<Effect>
//!   core.snapshot(now)            -> Snapshot
//!
//! That is the whole surface. The shell (shell.rs) feeds inputs, runs effects,
//! and publishes snapshots. Tests do the same with no mpv and no network.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::domain::{
    ItemKey, LoadId, Playable, RefillId, Station, StationId, StreamUrl, Tickets, Track, VideoId, Volume,
};
use crate::player::api::{
    ClockView, Command, Favorites, Level, NowPlaying, Notice, QueueRow, RadioStatus, SeekTo, Snapshot, Status,
    VolumeTo,
};
use crate::player::queue::{EntryId, Queue, Removed};

// ------------------------------------------------------------------ inputs and effects

/// Everything that can happen to the core.
#[derive(Debug)]
pub enum Input {
    Command(Command),
    Engine(EngineEvent),
    /// yt-dlp finished. Keyed by id, not ticket: a URL is useful to the cache
    /// whoever asked for it, so a "stale" resolve is never wasted.
    Resolved { id: VideoId, result: Result<StreamUrl, String> },
    Refilled { ticket: RefillId, result: Result<Vec<Arc<Track>>, String> },
    /// Once a second from the shell. Drives debounced session saves.
    Tick,
}

/// mpv, already translated by `shell::Mpv::translate`. Every per-file event
/// carries the `LoadId` it belongs to; the adapter bound it from mpv's
/// `playlist_entry_id` (loadfile reply, start-file, end-file all carry it on mpv 0.41).
#[derive(Debug, Clone, PartialEq)]
pub enum EngineEvent {
    /// First `playback-restart` after `start-file`: audio is flowing.
    Began { load: LoadId },
    /// Throttled by the adapter to one per second or on a discontinuity.
    Position { load: LoadId, pos: Duration },
    Duration { load: LoadId, duration: Duration },
    Icy { load: LoadId, title: Option<String> },
    /// `end-file` with reason eof or error. The adapter swallows `stop`, which only
    /// our own `loadfile replace` causes.
    Ended { load: LoadId, reason: EndReason },
    Paused(bool),
    Volume(Volume),
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
    Resolve { id: VideoId, urgency: Urgency },
    Refill { ticket: RefillId, seed: VideoId },
    /// radio-browser `/json/url/<uuid>` click count.
    CountClick(StationId),
    SaveLibrary(Library),
    SaveSession(Session),
    Notify(Notice),
}

#[derive(Debug, Clone, PartialEq)]
pub enum EngineCmd {
    Load { load: LoadId, url: url::Url, start: Option<Duration> },
    SetPause(bool),
    Seek(SeekTo),
    SetVolume(Volume),
    Stop,
    Quit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Urgency {
    /// Someone is waiting on it. The shell starts it immediately.
    Now,
    /// Next in queue or a prewarm. One background lane.
    Ahead,
}

// ------------------------------------------------------------------ playback state machine

/// Split by media kind first, because the kinds have different capabilities:
/// only `Track` has a seekable clock and a duration; only a track passes through
/// `Resolving`, which holds no URL and no LoadId. A seek on a station or a load
/// with no URL has nowhere to be written.
#[derive(Debug, Clone, PartialEq)]
pub enum Playback {
    Idle,
    Track { entry: EntryId, track: Arc<Track>, stage: TrackStage },
    Station { entry: EntryId, station: Arc<Station>, stage: StationStage },
}

#[derive(Debug, Clone, PartialEq)]
pub enum TrackStage {
    Resolving { attempt: Attempt, start: Option<Duration> },
    Loading { load: LoadId, attempt: Attempt },
    Playing { load: LoadId, clock: Clock, duration: Option<Duration> },
}

#[derive(Debug, Clone, PartialEq)]
pub enum StationStage {
    Connecting { load: LoadId, attempt: Attempt },
    OnAir { load: LoadId, listened: Clock, icy: Option<String> },
}

/// One retry per item. `Retry` failing means skip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Attempt {
    First,
    Retry,
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
    pub fn hold(self, now: Instant) -> Clock {
        Clock::Held(self.at(now))
    }
    pub fn run(self, now: Instant) -> Clock {
        Clock::Running { from: self.at(now), at: now }
    }
    pub fn reanchor(self, pos: Duration, now: Instant) -> Clock {
        match self {
            Clock::Running { .. } => Clock::Running { from: pos, at: now },
            Clock::Held(_) => Clock::Held(pos),
        }
    }
}

// ------------------------------------------------------------------ owned sub-states

/// URL cache. Entries past `usable_until` are treated as absent and pruned on insert.
#[derive(Debug, Default)]
pub struct StreamCache {
    urls: HashMap<VideoId, StreamUrl>,
    /// Requests out right now, at the most urgent level asked. Owned here so the
    /// shell never dedupes and a reconcile that runs every step emits each Resolve once.
    in_flight: HashMap<VideoId, Urgency>,
}

impl StreamCache {
    pub fn get(&self, id: &VideoId, now: Instant) -> Option<&StreamUrl> {
        self.urls.get(id).filter(|u| u.usable_until > now)
    }
    /// Returns the effect to emit, or None if cached or already in flight.
    pub fn want(&mut self, id: &VideoId, urgency: Urgency, now: Instant) -> Option<Effect> {
        // TODO:
        //   cached                         -> None
        //   in flight at Now               -> None
        //   in flight at Ahead, asked Now  -> upgrade to Now and emit again. The Ahead lane may
        //                                     be busy with prewarms; a second yt-dlp run is cheaper
        //                                     than a user waiting behind them. First result wins.
        //   in flight at Ahead, asked Ahead-> None
        //   absent                         -> insert, emit
        todo!()
    }
    pub fn landed(&mut self, id: &VideoId, result: &Result<StreamUrl, String>, now: Instant) {
        todo!("in_flight.remove; on Ok insert and prune expired")
    }
    pub fn evict(&mut self, id: &VideoId) {
        self.urls.remove(id);
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
        todo!()
    }
    /// Idempotent merge by StationId; existing entries win.
    pub fn merge_stations(&mut self, stations: Vec<Arc<Station>>) {
        todo!()
    }
    pub fn contains(&self, key: &ItemKey) -> bool {
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

// ------------------------------------------------------------------ the core

pub struct Core {
    queue: Queue,
    playback: Playback,
    streams: StreamCache,
    library: Library,
    /// Mirror of mpv's volume. Updated from `EngineEvent::Volume`, never optimistically.
    volume: Volume,
    tickets: Tickets,
    /// Last session written and when. Saves are derived by comparing, not flagged.
    saved_session: Option<(Session, Instant)>,
    /// Position to resume at when the user presses play after a restore.
    resume_at: Option<Duration>,
}

pub const SESSION_DEBOUNCE: Duration = Duration::from_secs(2);

impl Core {
    pub fn new(restore: Restore) -> (Core, Vec<Effect>) {
        // TODO:
        //   queue.restore(session.queue, session.cursor, session.radio_seed)
        //   volume = session.volume or initial_volume
        //   playback = Idle (restore never auto-plays)
        //   effects: [Engine(SetVolume(volume))]
        //            + [SaveLibrary] if the library was just migrated (radio_atlas_imported flipped by main)
        todo!()
    }

    pub fn step(&mut self, input: Input, now: Instant) -> Vec<Effect> {
        let mut fx = Vec::new();
        match input {
            Input::Command(cmd) => self.command(cmd, now, &mut fx),
            Input::Engine(ev) => self.engine(ev, now, &mut fx),
            Input::Resolved { id, result } => self.resolved(id, result, now, &mut fx),
            Input::Refilled { ticket, result } => self.refilled(ticket, result, &mut fx),
            Input::Tick => {}
        }
        self.reconcile(now, &mut fx);
        fx
    }

    pub fn snapshot(&self, now: Instant) -> Snapshot {
        // TODO: map Playback -> Option<NowPlaying>:
        //   Track/Resolving  -> Status::Resolving, ClockView::Track{pos: 0, duration: track.duration, running: false}
        //   Track/Loading    -> Buffering
        //   Track/Playing    -> Playing|Paused by clock.running(), pos = clock.at(now), duration
        //   Station/Connecting -> Buffering, ClockView::Live{listened: 0, ..}
        //   Station/OnAir    -> Playing|Paused, listened = clock.at(now), icy
        // queue rows from queue.entries(), cursor from queue.cursor_index()
        // radio from queue.radio_seed() + queue.refill_pending()
        // favorites from library (Arc clones)
        let _ = (ClockView::Track { pos: Duration::ZERO, duration: None, running: false }, Status::Playing);
        let _: (Option<NowPlaying>, Vec<QueueRow>, RadioStatus, Favorites);
        todo!()
    }

    // -------------------------------------------------------------- commands

    fn command(&mut self, cmd: Command, now: Instant, fx: &mut Vec<Effect>) {
        match cmd {
            Command::PlayTrack(track) => {
                // queue = [track], radio seeded by it; then start the current entry.
                let seed = track.id.clone();
                self.queue.replace(vec![Playable::Track(track)], 0, Some(seed));
                self.start_current(None, now, fx);
            }
            Command::PlayStations { stations, start } => {
                // TODO: start = position of `start` in stations (0 if absent);
                //       queue.replace(stations as Playable, start, None); start_current
                todo!()
            }
            Command::PlayNext(item) => {
                self.queue.insert_next(item);
                // TODO: if playback is Idle and nothing was current, jump to it and start.
                fx.push(notice(Level::Info, "plays next"));
            }
            Command::Next => {
                if self.queue.advance().is_some() {
                    self.start_current(None, now, fx);
                }
            }
            Command::Prev => {
                // TODO: if a track has played > 3 s, restart it (Seek At 0) instead of retreating.
                if self.queue.retreat().is_some() {
                    self.start_current(None, now, fx);
                }
            }
            Command::Jump(id) => {
                if self.queue.jump(id).is_some() {
                    self.start_current(None, now, fx);
                }
            }
            Command::Remove(id) => match self.queue.remove(id) {
                // Removing what plays starts its follower, or stops at the end.
                Removed::Current => self.start_current(None, now, fx),
                Removed::Other | Removed::NotFound => {}
            },
            Command::TogglePause => self.toggle_pause(now, fx),
            Command::Seek(to) => {
                // The type split makes this the only place a seek can be issued.
                if let Playback::Track { stage: TrackStage::Playing { .. }, .. } = self.playback {
                    fx.push(Effect::Engine(EngineCmd::Seek(to)));
                }
            }
            Command::Volume(to) => {
                let v = match to {
                    VolumeTo::By(d) => self.volume.nudge(d),
                    VolumeTo::At(v) => v,
                };
                fx.push(Effect::Engine(EngineCmd::SetVolume(v)));
            }
            Command::SetFavorite { item, on } => {
                if self.library.set(&item, on) {
                    fx.push(Effect::SaveLibrary(self.library.clone()));
                }
            }
            Command::Prewarm(ids) => {
                for id in ids {
                    fx.extend(self.streams.want(&id, Urgency::Ahead, now));
                }
            }
            Command::Shutdown => {
                // Save first so a crash during mpv quit still leaves a session.
                fx.push(Effect::SaveSession(self.session(now)));
                fx.push(Effect::Engine(EngineCmd::Quit));
            }
        }
    }

    fn toggle_pause(&mut self, now: Instant, fx: &mut Vec<Effect>) {
        // TODO:
        //   Idle with a current entry (session restore) -> start_current(self.resume_at.take())
        //   Track Playing / Station OnAir -> Engine(SetPause(clock.running()))   // mpv echoes Paused(..)
        //   Resolving / Loading / Connecting -> ignore (nothing to pause yet)
        todo!()
    }

    /// Begin playing whatever the queue cursor points at. The single entry point
    /// for "something new should play"; every command above funnels here.
    fn start_current(&mut self, start: Option<Duration>, now: Instant, fx: &mut Vec<Effect>) {
        let Some(entry) = self.queue.current().cloned() else {
            if self.playback != Playback::Idle {
                fx.push(Effect::Engine(EngineCmd::Stop));
            }
            self.playback = Playback::Idle;
            return;
        };
        match entry.item {
            Playable::Track(track) => {
                // Cache hit: straight to Loading, which is the <300 ms skip path.
                if let Some(url) = self.streams.get(&track.id, now).map(|u| u.url.clone()) {
                    let load = self.tickets.next();
                    fx.push(Effect::Engine(EngineCmd::Load { load, url, start }));
                    self.playback =
                        Playback::Track { entry: entry.id, track, stage: TrackStage::Loading { load, attempt: Attempt::First } };
                } else {
                    fx.extend(self.streams.want(&track.id, Urgency::Now, now));
                    self.playback = Playback::Track {
                        entry: entry.id,
                        track,
                        stage: TrackStage::Resolving { attempt: Attempt::First, start },
                    };
                }
            }
            Playable::Station(station) => {
                let load = self.tickets.next();
                fx.push(Effect::Engine(EngineCmd::Load { load, url: station.stream.clone(), start: None }));
                fx.push(Effect::CountClick(station.id.clone()));
                self.playback = Playback::Station {
                    entry: entry.id,
                    station,
                    stage: StationStage::Connecting { load, attempt: Attempt::First },
                };
            }
        }
    }

    // -------------------------------------------------------------- async results

    fn resolved(&mut self, id: VideoId, result: Result<StreamUrl, String>, now: Instant, fx: &mut Vec<Effect>) {
        self.streams.landed(&id, &result, now);
        // Identity check, no tickets: is the current playback waiting on this id?
        let Playback::Track { track, stage: TrackStage::Resolving { attempt, start }, .. } = &self.playback else {
            return;
        };
        if track.id != id {
            return;
        }
        // TODO:
        //   Ok(url)  -> load = tickets.next(); Engine(Load{load, url, start}); stage = Loading{load, attempt}
        //   Err(e) if attempt == First -> streams.want(id, Now) again; stage = Resolving{Retry, start}
        //   Err(e)   -> Notify(Error, "couldn't play <title>: <e>"); skip()
        let _ = (attempt, start);
        todo!()
    }

    fn refilled(&mut self, ticket: RefillId, result: Result<Vec<Arc<Track>>, String>, fx: &mut Vec<Effect>) {
        // Queue::refill_landed does the ticket check; a refill for a replaced queue returns 0.
        // TODO:
        //   Ok(tracks) -> n = queue.refill_landed(ticket, Some(tracks)); if n > 0 Notify(Info, "radio: +n")
        //   Err(e)     -> queue.refill_landed(ticket, None); Notify(Warn, "radio refill failed")
        //   If playback is Idle because the queue had run dry while the refill was in flight
        //   (auto-advance found nothing), advance and start_current now.
        todo!()
    }

    // -------------------------------------------------------------- engine events

    fn engine(&mut self, ev: EngineEvent, now: Instant, fx: &mut Vec<Effect>) {
        match ev {
            EngineEvent::Volume(v) => self.volume = v,
            EngineEvent::Paused(p) => {
                // TODO: in Track Playing / Station OnAir: clock = if p { hold } else { run }
            }
            EngineEvent::Restarted => {
                // TODO: idempotent recovery. Every LoadId is dead.
                //   fx Engine(SetVolume(self.volume))
                //   Track (any stage) -> start_current(Some(last known pos)) with attempt reset
                //   Station -> start_current(None)
                //   Idle -> nothing
            }
            EngineEvent::Began { load } => {
                // TODO: only if `load` matches the current Loading/Connecting stage:
                //   Track Loading{load}     -> Playing{load, clock: Running{from: 0 or start, at: now}, duration: track.duration}
                //   Station Connecting{load}-> OnAir{load, listened: Running{0, now}, icy: None}
                //   anything else           -> stale; drop
            }
            EngineEvent::Position { load, pos } => {
                // TODO: Track Playing{load == this} -> clock = clock.reanchor(pos, now)
                //       Stations ignore position; listened time is wall clock.
            }
            EngineEvent::Duration { load, duration } => {
                // TODO: Track Playing{load == this} -> duration = Some(duration)
            }
            EngineEvent::Icy { load, title } => {
                // TODO: Station OnAir{load == this} -> icy = title
            }
            EngineEvent::Ended { load, reason } => self.ended(load, reason, now, fx),
        }
    }

    fn ended(&mut self, load: LoadId, reason: EndReason, now: Instant, fx: &mut Vec<Effect>) {
        // Stale end-files (an old load) are dropped by this match: only the live
        // LoadId of the current stage gets through.
        let live = match &self.playback {
            Playback::Track { stage: TrackStage::Loading { load: l, .. } | TrackStage::Playing { load: l, .. }, .. }
            | Playback::Station { stage: StationStage::Connecting { load: l, .. } | StationStage::OnAir { load: l, .. }, .. } => *l,
            _ => return,
        };
        if live != load {
            return;
        }
        // TODO: the retry table.
        //   (Track, Eof)                         -> auto-advance: skip()
        //   (Track Loading{First}, Error)        -> streams.evict(id); want(id, Now);
        //                                           stage = Resolving{Retry, start: None}
        //                                           (YouTube URLs are IP-bound; a VPN hop kills them)
        //   (Track Playing{..}, Error)           -> same, but start = Some(clock.at(now)) so it resumes in place
        //   (Track Loading{Retry}, Error)        -> Notify(Error, "skipped <title>"); skip()
        //   (Station Connecting{First}, Error)   -> reload same URL once: new LoadId, Connecting{Retry}
        //   (Station *, Eof) | (Station OnAir, Error) -> same as Connecting{First} (stream dropped)
        //   (Station Connecting{Retry}, Error)   -> Notify(Error, ..); skip()
        let _ = (reason, now, fx);
        todo!()
    }

    /// Advance and start, or stop at the end of the queue.
    /// At the end with radio refill in flight, stays Idle; `refilled` resumes.
    fn skip(&mut self, now: Instant, fx: &mut Vec<Effect>) {
        if self.queue.advance().is_some() {
            self.start_current(None, now, fx);
        } else {
            fx.push(Effect::Engine(EngineCmd::Stop));
            self.playback = Playback::Idle;
        }
    }

    // -------------------------------------------------------------- reconcile

    /// Derived background work. Runs after every step; each branch is a no-op
    /// when its work is already done or in flight, so running it twice is harmless.
    fn reconcile(&mut self, now: Instant, fx: &mut Vec<Effect>) {
        // 1. Prefetch: once the current item is past Resolving, make sure the next
        //    upcoming *track* has a usable URL. Skip stations; they need nothing.
        //    TODO: if !matches!(playback, Track{stage: Resolving{..}, ..}) {
        //              if let Some(Track t) = queue.upcoming().find_map(track) .next only {
        //                  fx.extend(streams.want(&t.id, Urgency::Ahead, now)) } }
        //    Expiry is covered: `want` treats an expired URL as absent.
        //
        // 2. Radio refill.
        //    TODO: if let Some(w) = queue.refill_wanted() {
        //              let ticket = tickets.next(); queue.refill_started(ticket);
        //              fx.push(Effect::Refill{ticket, seed: w.seed}) }
        //
        // 3. Session save, debounced and derived.
        //    TODO: let s = self.session(now);
        //          if saved_session.map_or(true, |(old, at)| old != s && now - at >= SESSION_DEBOUNCE) {
        //              fx.push(SaveSession(s.clone())); saved_session = Some((s, now)) }
        //    Position is excluded from the comparison except on Shutdown, or every
        //    second would write the file.
        let _ = (now, fx);
        todo!()
    }

    fn session(&self, now: Instant) -> Session {
        todo!("queue items, cursor, radio seed, volume, position of a playing track")
    }
}

fn notice(level: Level, text: impl Into<String>) -> Effect {
    Effect::Notify(Notice { level, text: text.into() })
}

#[cfg(test)]
mod tests {
    // Driven through `step` only, asserting on returned effects and `snapshot()`.
    // Fixtures: a few `Arc<Track>`/`Arc<Station>` builders and a fake `StreamUrl`.
    //
    // play_track_with_cold_cache_emits_urgent_resolve_and_refill
    // play_track_with_cached_url_loads_immediately_without_resolve
    // expired_cached_url_is_treated_as_missing
    // resolve_for_a_track_no_longer_current_only_fills_cache
    //   catches: the prototype class of "late resolve starts the wrong song".
    // ended_with_stale_load_id_is_ignored
    //   catches: end-file(stop/error) of the previous file skipping the new one.
    // load_error_first_attempt_evicts_and_reresolves_once
    // load_error_on_retry_skips_and_notifies
    // eof_advances_and_starts_next
    // eof_at_end_with_refill_in_flight_waits_then_resumes_on_refill
    // seek_on_station_emits_nothing
    // seek_while_resolving_emits_nothing
    // began_moves_loading_to_playing_and_triggers_prefetch_of_next_track
    // refill_for_replaced_queue_is_dropped
    // play_stations_starts_at_named_station_and_counts_click
    // set_favorite_twice_saves_once
    // restarted_reloads_current_track_at_last_position
    // session_save_is_debounced_and_skips_identical_sessions
}
