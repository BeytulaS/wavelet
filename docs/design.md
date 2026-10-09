# wavelet v1: candidate 1, "the player is already a daemon"

## Problem

wavelet plays YouTube Music and internet radio through an mpv child process, with a ratatui UI ported from Myx. The prototype proved every backend fact but grew into one `App` struct where search results doubled as the queue, async results were matched by ad hoc strings (`radio_pending`, `inflight`, `file_started`), and render wrote back through `Cell`s. v1 must not block a later daemon that owns playback while the TUI, `wavelet ctl`, and the Omarchy bar plugin act as clients. So the player core must run with no ratatui types and talk in messages a socket can carry.

Constraints carried in from grounding:

- mpv 0.41 on this machine answers `loadfile` with `{"data":{"playlist_entry_id":N},"request_id":R}`, and `start-file` and `end-file` carry the same `playlist_entry_id`. A failed load emits `start-file` then `end-file reason=error`. A replaced file emits `end-file reason=stop`. `time-pos` carries no entry id. (Measured with a probe against `mpv --idle` this session.)
- The Unix socket path limit is 108 bytes. The probe failed on a long scratch path, so the mpv socket must live under `$XDG_RUNTIME_DIR`.
- yt-dlp takes about 1.5 s (spec, measured). The spec's cold-start target of under 2 s requires resolution to start before Enter.
- Radio Atlas favorites in `state.json` use `uuid, name, url, homepage, favicon, countryCode, tags (comma string), codec, bitrate`. `url-overrides.json` maps uuid to url. (Read this session.)

## Usage (caller's view)

The player is a library with one handle. The TUI, MPRIS, and later the daemon's socket clients all use it the same way.

```rust
// main.rs
let player = player::spawn(PlayerOpts { restore, audio_device, runtime_dir, store })?;
mpris::spawn(player.clone());
tui::run(cfg, issues, player)?;
```

Clients send intent by value and subscribe to snapshots:

```rust
// tui/input.rs: Enter on a YouTube result. The item travels by value; no index survives.
Action::Activate => match ui.browse.selected() {
    Some(Playable::Track(t)) => ui.player.send(Command::PlayTrack(t)),
    Some(Playable::Station(s)) => ui.player.send(Command::PlayStations {
        stations: ui.browse.active().stations(), start: s.id.clone(),
    }),
    None => {}
},
Action::Next => ui.player.send(Command::Next),

// tui/mod.rs: subscribe once; every change arrives as an Arc<Snapshot>.
player.subscribe(Box::new(move |out| tx.send(Msg::Player(out.clone())).is_ok()));

// mpris.rs: same handle, same commands.
mp.connect_play_pause(move |_| h.send(Command::TogglePause));
```

Tests drive the core with no threads, no mpv, and no network:

```rust
let (mut core, _) = Core::new(restore_empty());
let fx = core.step(Input::Command(Command::PlayTrack(track("aaaaaaaaaaa"))), t0);
assert_eq!(fx, vec![resolve("aaaaaaaaaaa", Urgency::Now), refill(ticket(1), "aaaaaaaaaaa")]);
let fx = core.step(Input::Resolved { id: vid("bbbbbbbbbbb"), result: Ok(url()) }, t0);
assert!(fx.is_empty());                       // a resolve for something else only fills the cache
assert_eq!(core.snapshot(t0).now.unwrap().status, Status::Resolving);
```

## Shape

### Module map (sketch/src)

| File | Owns |
|---|---|
| `domain.rs` | `VideoId`, `StationId`, `Track`, `Station`, `Playable` (enum of `Arc`s), `ItemKey`, `Volume`, `StreamUrl`, `Ticket<K>` |
| `player/api.rs` | The protocol: `Command`, `Out`, `Snapshot`, `ClockView`, `Notice`. Serde-ready for the daemon socket. |
| `player/core.rs` | `Core` reducer: `new`, `step`, `snapshot`. `Playback` state machine, `Clock`, `StreamCache`, `Library`, retry table, `reconcile`. Pure. |
| `player/queue.rs` | `Queue` with private cursor, stable `EntryId`s, radio refill bookkeeping. Pure. |
| `player/shell.rs` | Player thread: inbox loop, effect runner, derived snapshot publish, `PlayerHandle`, and the mpv adapter (`Mpv::send`, `Mpv::translate`). |
| `player/store.rs` | library.json, session.json, Radio Atlas import. File schemas private. Atomic writes. |
| `sources/{ytm,radio_browser,ytdlp}.rs` | Blocking fetches plus pure parsers to domain types. `ytdlp::Lanes` runs resolves. |
| `mpris.rs` | MPRIS thread with a local executor. |
| `config.rs` | config.toml and keymap.toml, per-key fallback with `Issue`s. |
| `tui/mod.rs` | `Ui`, `Msg`, the loop, Myx draw cadence, toasts. |
| `tui/input.rs` | `Action`, `Keymap` table, `KeyState::feed`, `dispatch` (the one place actions become commands). |
| `tui/{browse,look,art,spectrum,draw}.rs` | Results lists with search tickets; theme modes and fade with omarchy watch; art fetch and identity check; pw-record capture; rendering returning `FrameOut`. |
| `tui/paint/*` | Myx files ported unchanged (copied into the sketch so it type-checks). |

The sketch passes `cargo check` with `todo!()` bodies (run this session, 0 errors).

### Data first

The queue is not the results list. `Queue` holds `Vec<Entry { id: EntryId, item: Playable, from_radio }>` with a private `cursor`. `EntryId`s are never reused, so a client holding one hits the entry it meant or nothing. Every queue mutation is a method that keeps the cursor valid (`replace`, `insert_next`, `advance`, `retreat`, `jump`, `remove`), which deletes the prototype's `usize::MAX/2` class of bug (per model-the-domain).

`Playback` splits by media kind before stage, because the kinds have different capabilities (per type-system-discipline):

```rust
enum Playback {
    Idle,
    Track   { entry, track: Arc<Track>,     stage: TrackStage },
    Station { entry, station: Arc<Station>, stage: StationStage },
}
enum TrackStage   { Resolving { attempt, start }, Loading { load, attempt }, Playing { load, clock, duration } }
enum StationStage { Connecting { load, attempt },                OnAir   { load, listened: Clock, icy } }
enum Clock { Running { from, at }, Held(Duration) }
```

`Resolving` holds no URL and no `LoadId`, so a load without a URL cannot be written. Only `TrackStage::Playing` has a seekable clock and a duration, and `Command::Seek` is handled by one `if let` on that variant. Pause lives in `Clock` alone. Position at any instant is computed from the anchor.

### One staleness rule

Every async request carries a `Ticket<K>`. The slot that issued it stores it, and a result applies only when its ticket equals the slot's current one. Replacing the slot (new search, new queue, new load) replaces the ticket. `K` brands the slot, so a `SearchId` cannot satisfy a `LoadId` slot. Concretely:

- mpv loads use `LoadId` as the IPC `request_id`. The adapter binds mpv's `playlist_entry_id` to it from the reply, so every `EngineEvent` names its load. A stale `end-file` cannot skip the new song, and old-file `time-pos` events are dropped without a `file_started` flag.
- Radio refills use `RefillId`, stored in `Queue`'s radio state and cleared by `replace`.
- Searches use `SearchId` per tab in `tui/browse.rs`.
- Resolves are the one exception, keyed by `VideoId`. A URL is useful to the cache whoever asked, so a stale resolve fills the cache and `resolved()` starts a load only if the current `Resolving` stage names that id.

### Actors and ownership

std threads and `mpsc` channels, no tokio. Each thread with state owns it outright, and workers hold nothing (per separate-before-serializing-shared-state).

| Thread | Owns | Talks to |
|---|---|---|
| player | `Core`, `Mpv` (entry-id map, position throttle), subscriber sinks | inbox `Inbox::{Core(Input), Mpv(MpvRaw), Subscribe}` |
| mpv reader | nothing; parses lines | player inbox |
| yt-dlp `Now` threads, one `Ahead` worker | nothing | player inbox |
| refill, click, search, art fetches | nothing | player or TUI inbox |
| main (TUI) | `Ui`: browse lists, keys, look, art, toasts, latest `Arc<Snapshot>` | TUI inbox `Msg` |
| capture | writes `VisBands` | the only shared state: a 128-float `Mutex`, `try_lock` on draw |
| mpris | the mpris-server object | `PlayerHandle`; snapshots via `async-channel` |

Dedupe of in-flight resolves lives in `StreamCache` (core), not in the lanes. A `Now` request for an id already in flight at `Ahead` re-emits, so a play never waits behind prewarms.

### Effects and derived work

`step` returns `Vec<Effect>` (`Engine`, `Resolve`, `Refill`, `CountClick`, `SaveLibrary`, `SaveSession`, `Notify`). After every input, `reconcile` derives background work from state: prefetch the next upcoming track, request a refill when 3 or fewer remain, and save the session when it differs from the last saved one and 2 s have passed. Each branch is a no-op when its work is done or in flight, so it runs every step safely (per make-operations-idempotent). The shell publishes by building a snapshot and comparing it with the last one (`PartialEq`), so no field carries a dirty flag. `SetFavorite { on }` is absolute rather than a toggle, so a retried command converges. `EngineEvent::Restarted` (mpv died and was respawned) reloads the current item at its last position.

### Rendering

The TUI holds the latest `Arc<Snapshot>` and its receive `Instant`. `draw::frame(f, &ui, now) -> FrameOut` borrows `&Ui` and returns measurements (list rows, scroll offset, hit rects) instead of writing through `Cell`s. The mpv adapter throttles `time-pos` to 1 Hz or discontinuities, and the TUI extrapolates the clock between snapshots. The loop ports Myx's `should_draw` (16 ms on input, 33 ms while the spectrum, a fade, a toast, or a spinner animates, 500 ms idle) and its `ArtRepaint` wipe-then-draw.

### Interface depth

The player's public surface is `spawn`, `PlayerHandle::{send, subscribe}`, `Command`, and `Out`. Behind it sit retry, prefetch, refill, cache expiry, URL urgency, every staleness check, mpv's event-binding quirks, debounced persistence, and crash recovery. The core's surface is three functions. The mpv adapter's is two (`send`, `translate`). Callers never coordinate multiple calls to finish one operation.

### Traces

"User presses n" to `loadfile` (three files):
`tui/input.rs` `dispatch(Action::Next)` → `PlayerHandle::send(Command::Next)` (`player/shell.rs`) → `Shell::run` → `Core::step` → `command` → `Queue::advance` → `start_current` (cache hit) → `Effect::Engine(Load)` (`player/core.rs`) → `Shell::run_effect` → `Mpv::send` writes `{"command":["loadfile",url,"replace"],"request_id":<LoadId>}` (`player/shell.rs`).

Enter on a YouTube search result to first audio:

1. `tui/mod.rs` `run` → `Ui::handle(Msg::Term)` → `tui/input.rs` `on_key` → `KeyState::feed` → `Fed::Fire { Activate }` → `dispatch` → `tui/browse.rs` `Browse::selected` → `PlayerHandle::send(Command::PlayTrack(track))`.
2. `player/shell.rs` `Shell::run` → `player/core.rs` `Core::step` → `command(PlayTrack)` → `player/queue.rs` `Queue::replace([track], 0, Some(seed))` → `start_current`. With a cache hit from the search prewarm, this emits `Engine(Load)` and jumps to step 4. Otherwise `StreamCache::want(id, Now)` emits `Resolve { Now }` and the stage is `Resolving { First }`. `reconcile` → `Queue::refill_wanted` → `Refill { ticket, seed }`.
3. `player/shell.rs` `run_effect` → `sources/ytdlp.rs` `Lanes::request(id, Now)` → thread `resolve` → `parse_stream_url` → `Inbox::Core(Input::Resolved)`. In parallel, `sources/ytm.rs` `radio` → `Input::Refilled`.
4. `Core::step(Resolved)` → `resolved` (id matches the `Resolving` stage) → `Effect::Engine(Load { load, url })`, stage `Loading` → `Mpv::send` writes `loadfile`.
5. mpv reader → `Inbox::Mpv(Reply)` → `Mpv::translate` binds entry id to `LoadId`. Then `StartFile`, then `PlaybackRestart` → `EngineEvent::Began { load }` → `Core::engine` → `TrackStage::Playing`. `reconcile` emits `Resolve { next, Ahead }`. `publish_if_changed` → the TUI sink → `Msg::Player` → `Ui::handle` → `Art::want`.

Track ends to the next track playing:
mpv `end-file reason=eof` → reader → `Inbox::Mpv(EndFile)` → `player/shell.rs` `Mpv::translate` → `EngineEvent::Ended { load, Eof }` → `player/core.rs` `ended` (live-load check) → `skip` → `Queue::advance` → `start_current` (URL prefetched at the previous `Began`) → `Engine(Load)` → `Mpv::send` → `Began` → `Playing`. `reconcile` prefetches the next one and refills at 3 or fewer.

## Synthesis decision

Three runners sketched independently (two on Opus, one on Sonnet), and an Opus judge scored them on six criteria: candidate 1 at 26, candidate 2 at 25, candidate 3 at 26. This design is candidate 1. All three agreed on the large decisions, which is why they are settled here without further argument: std threads and channels with no tokio, Playback split by media kind before stage, queue entries addressed by `EntryId`, mpv events correlated through `playlist_entry_id`, resolves keyed by `VideoId` into a cache, search owned by the TUI, and policy derived after every input instead of fired from event arms.

The open fork was where the core runs. Candidate 3 kept the reducer on the UI thread and rendered from `&State`. It is less code today. Candidate 1 won because its weak points are local fixes, while candidate 3's touch its contracts: a dirty flag every mutation must remember, a mutex shared between the mpv writer and reader, and the internal `Queue` doubling as the session file format. The daemon in the spec's next phase also makes the player thread a cost paid once, now, instead of a refactor later.

Grafted from candidate 2:

- `mpv::attribute(&mut Attribution, &Value) -> Option<EngineEvent>` is a pure function on the reader thread, tested on recorded mpv lines. First audio is the first `time-pos > 0` after our `start-file`, as the prototype measured. This replaces candidate 1's `playback-restart` signal.
- `Attempt { First, Retry }` with `next() -> Option<Attempt>` is the whole retry policy, applied in one `fail_current`. The URL cache keeps a `Failed { until }` entry so a dead prefetch does not respawn yt-dlp on every step.

Grafted from candidate 3:

- An explicit "queue ran dry while a refill is in flight" state, and `Fetch::Dry` so an empty refill does not loop.
- The snapshot's `now` is the kind-split enum, not a flattened `Status` plus `ClockView`.
- `tests/layering.rs` greps `player/`, `sources/`, and `domain.rs` for `ratatui`, `crossterm`, and `tui::` imports and fails if any appear.

Rejected:

- Candidate 1's `Urgency` lanes and the duplicate yt-dlp run on upgrade. An in-flight set with a cap of five does the job.
- Candidate 1 reading pause and volume back from mpv's echo. The core is the single source of truth for both, as in candidates 2 and 3.
- Candidate 2's content-keyed staleness for searches and radio. Tickets stored in the slot they guard are simpler to reason about.
- Candidate 3's whole-second position in state. The snapshot carries the clock anchor and clients extrapolate.

Answers to the open questions, chosen as defaults:

- First audio is `time-pos > 0` after `start-file` for our load (verified by the prototype's timing log).
- A restored session restores queue, cursor, and volume, and waits for Space.
- `b` restarts the current track when its position is past 3 s, and goes to the previous entry otherwise.
- A station that fails twice stops with a toast. A station list never auto-advances, because a listener who picked one station wants that station.
- A YouTube track that fails twice is skipped with a toast.
- The Radio Atlas import runs once on first start and shows a toast with the count.
- Removing the current entry skips to its follower, then removes it.
- Radio refill seeds from the current track, as in the prototype, which gave varied results over 52 skips.
- When a YouTube search returns, the core pre-resolves the top three results (`Command::Warm`), within the in-flight cap. This is the only way to reach the 2 s cold-start target.
- `q` closes the TUI and playback continues in the daemon (docs/daemon.md). `Q` stops both.

## Tradeoffs accepted

- We accept a short-lived OS thread per resolve, fetch, and search in exchange for no async runtime under the reducer, the shell, or the parsers. Request rates are human-scale. mpris-server, the only async dependency, gets one thread with `futures_lite::block_on`.
- We accept two state owners (player thread, TUI thread) and a channel hop per command in exchange for the daemon move being a transport swap. The TUI already cannot read `Core` internals.
- We accept rebuilding and comparing a full snapshot after every input (a `Vec` of `Arc` clones, a few times a second) in exchange for no dirty flags and no missed redraws.
- We accept that pause and volume update on mpv's echo, not optimistically, in exchange for mpv being the only source of truth for both.
- We accept a duplicate yt-dlp run when a play upgrades an in-flight prewarm, in exchange for plays never queuing behind prewarms.
- We accept the mpv adapter living in `player/shell.rs` (a larger file) in exchange for the three-file trace from key to `loadfile`.
- We accept search living in the TUI, not the player. Searching is view state, and the player stays small. `wavelet ctl search` would call `sources/` directly.
- We accept no shuffle or repeat in v1. `Queue` keeps the cursor private, so an order permutation can be added inside it without touching callers.
- We accept `config.rs` importing TUI enums (`Action`, `ThemeMode`, `LayoutKind`). The player never imports `config`.

## Alternatives considered

- **tokio everywhere** (async shell, `tokio::process` for mpv and yt-dlp, async reqwest, MPRIS on the same runtime). It hides nothing more from callers. It exposes async to every adapter and test, and it pays a runtime for about five concurrent operations. It wins only if the daemon later serves hundreds of socket clients.
- **One thread, core as a field of the UI** (the prototype's shape, render borrows `&Core`). Its interface is shallower to write and has one fewer hop. But nothing stops the TUI from reading core internals that a socket cannot carry, so the daemon move becomes a refactor across every render function.
- **The spec's single `Playback` enum** (`Idle | Resolving | Buffering | Playing | Paused | Failed` around a `Playable`). It keeps a runtime check on the item kind for every seek and duplicates fields between `Playing` and `Paused`. `Failed` as a resting state invites "what now" branches. The retry table in `ended` moves straight on instead.
- **Untyped generation counters** (prototype `search_gen`, `radio_pending: Option<String>`). This is the same idea as tickets without the brand. One shared counter type lets a search generation satisfy a radio check.

## Implementation reconciliation

Where v1 differs from the sketch above, and why.

- **Library plus binary.** `src/lib.rs` holds every module and `src/main.rs` only wires them. Integration tests (`tests/layering.rs`, `tests/player_live.rs`) can then use the player, and units not yet written stayed out of the build instead of carrying `todo!()` warnings past the clippy gate.
- **mpv attribution.** `player/mpv.rs` holds the pure `attribute(&mut Attribution, &Value)`. The reader thread runs it and sends `Input::Engine` straight to the inbox, so `Inbox::Mpv` and `translate` are gone. The mpv child, its writer, and respawn stay in `shell.rs`. The inbox gained `MpvGone(generation)`, so a reader of an already replaced mpv cannot trigger a second respawn, and `Shutdown(ack)`, so `q` waits until mpv has quit.
- **`Began` carries `duration` and `icy`.** The recorded transcript (`fixtures/mpv_transcript.jsonl`) shows `duration` before the first `time-pos > 0`, and stations send ICY titles while connecting. No pre-audio stage has room for them, so the attribution holds them and hands them over with `Began`.
- **First audio.** The transcript shows `playback-restart` before the first `time-pos > 0`, so v1 uses the prototype's rule (`time-pos > 0` after our `start-file`). This settles the first open question below.
- **`Reached { attempt, at }` in `Playing` and `OnAir`.** The live test found that a station whose stream ends right after connecting reconnected forever, because reaching `OnAir` forgot the attempt. A failure within 30 s (`STABLE_AFTER`) of first audio now counts against the attempt that got there; after 30 s a drop starts over at `First`.
- **`Fetch::InFlight { ticket, seed }`.** The refill slot remembers its seed, so `Dry { at }` names the track the empty answer was for, even if the user skipped while the refill ran.
- **Resolves.** Plays ignore the cap and negative entries. A play for an id already in flight waits for that resolve instead of starting a second one. `RESOLVE_CAP` is 5 and `FAILED_BACKOFF` is 60 s.
- **Commands.** `Prewarm` is `Warm`, as the synthesis named it. `SetPaused(bool)` was added for MPRIS Play and Pause. `EngineCmd::Seek` is absolute, because the core computes relative seeks against its own clock and re-anchors it at once.
- **Snapshot favorites** are one `items: Vec<Playable>` (tracks, then stations), which is what the Favorites tab lists.
- **Queue view.** `gq` toggles a fourth list in `Browse` (`Tab::Queue`) that every layout shows in its list panel. Enter jumps and `dd` removes there. Removing the current entry while stopped does not start its follower.
- **Keys.** A bound digit fires its binding (`1` `2` `3` switch sources); unbound digits build a count (`5j`).
- **Art repaint.** v1 keeps the prototype's single force-repaint flag instead of the `ArtRepaint` wipe-then-draw enum. It is what the user approved on screen.
- **Children die with wavelet.** mpv and pw-record get `PR_SET_PDEATHSIG` (`libc`), so a crash cannot leave an idle mpv behind.
- **Boundary cleaning.** ICY titles lose control characters before they reach the terminal.
- **radio-browser** uses the `all.api.radio-browser.info` name, as the prototype did, without a DNS pick.
- **MPRIS** runs under `futures_lite::future::block_on` with zbus's default `async-io` backend. Verified with `busctl` calls equivalent to playerctl's (playerctl is not installed here). This settles the second open question.
- **Tradeoffs above that the synthesis superseded.** Pause and volume do not update on mpv's echo, and a play does not start a duplicate yt-dlp run.
- **Not in v1.** Mouse input, custom theme files (`~/.config/wavelet/themes/<name>.toml`), the `transparent` and `mouse` config keys, the art disk cache, and Lyrics.

## Open questions and risks

- Is `playback-restart` the right first-audio signal for HTTP streams? The prototype measured `time-pos > 0` after `start-file`. If `playback-restart` fires before audible audio, `translate` should switch to that rule. One timing log run settles it.
- `mpris_server::Player` is `!Send`. Does it run cleanly under `futures_lite::block_on` with zbus's default `async-io` backend, or does it need `LocalExecutor`?
- Should removing the current entry (`dd`) start its follower, as sketched, or let the current track finish first?
- Should `b` restart the current track after 3 s, as most players do?
- Should a restored session auto-play on launch? The sketch restores the queue and volume but stays idle until play.
- When a station fails twice, should the queue skip to the next station in the list, or stop? Skipping matches the brief. It may surprise someone who picked one favorite.
- Should the Radio Atlas import stay silent and run once with a toast, as sketched, or show the "offer" prompt the spec mentions?
- Over the future socket, can clients live with about 1 ms of jitter from stamping the clock at receive time, or do snapshots need wall-clock anchors?

## Next implementation step

Fill in `domain.rs`, `player/queue.rs`, and `player/core.rs` with the tests listed in their `mod tests` blocks. These are pure, so they run without mpv or a network, and they pin the state machine before any I/O exists.
