# Daemon, `wavelet ctl`, and the bar widget

Playback moves out of the TUI into a long-running daemon, so music keeps playing after the terminal closes and the Omarchy bar can control it. The player thread, `PlayerHandle`, and the serde types in `player/api.rs` were built for this. The move is a transport change, not a redesign of the player.

## Processes

```
wavelet            TUI client. Starts the daemon if no socket answers.
wavelet daemon     Owns mpv, the player thread, MPRIS, the session, and status.json.
wavelet ctl ...    One command per run. Used by the bar widget, keybinds, and scripts.
wavelet plugin install
                   Writes the bar widget into ~/.config/omarchy/plugins/bsmail.wavelet/.
```

All runtime files live in `runtime_dir()` (`$XDG_RUNTIME_DIR/wavelet`).

| File | Owner | Purpose |
|---|---|---|
| `daemon.lock` | daemon | `flock` held for the daemon's life. A second daemon exits 0 at once. |
| `wavelet.sock` | daemon | Client socket. Removed and re-bound only by the lock holder. |
| `mpv.sock` | daemon | mpv IPC. Only the lock holder spawns mpv, so two instances cannot clobber it. |
| `status.json` | daemon | Bar view, written by temp file and rename. |

The daemon's stderr goes to `$XDG_STATE_HOME/wavelet/daemon.log` (append) when a client starts it.

## Wire protocol

Newline-delimited JSON over `wavelet.sock`, serde's default enum tagging.

- Daemon to client: first `ServerLine::Hello { build }`, then `ServerLine::Out(Out)` lines. The first `Out` is always a `Snapshot`. Every connection is subscribed.
- Client to daemon: `api::Command` lines. `Command::Shutdown` stops the daemon.

```rust
// src/daemon/wire.rs
#[derive(Serialize, Deserialize)]
pub enum ServerLine { Hello { build: BuildId }, Out(Out) }

/// Identity of the executable file: "<dev>:<inode>:<mtime_ns>" of current_exe(),
/// read once at startup. A reinstall changes it.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct BuildId(String);
```

A client whose own `BuildId` differs from the daemon's sends `Shutdown`, waits for EOF, and starts a fresh daemon once. `ctl quit` skips the check. The restarted daemon restores the session paused, as any start does.

Each connection gets a reader thread (lines to `PlayerHandle::send`) and a writer thread fed by `sync_channel(64)`. The subscription sink only `try_send`s the `Out`. A full channel drops that client, so a stalled client never blocks the player thread. Serialization happens on the writer thread.

The daemon stops on `Command::Shutdown` from any client, and on SIGTERM, SIGINT, or SIGHUP. Stopping runs `PlayerHandle::shutdown`, writes `status.json` with `state: "off"`, removes `wavelet.sock`, and exits 0.

## status.json

A projection of `Snapshot` that leaves out the clock, so it changes only on real events. The daemon rewrites the file only when the value differs from the last write.

```json
{
  "state": "playing",
  "kind": "station",
  "title": "Daft Punk - One More Time",
  "subtitle": "Radio The Voice",
  "station_id": "9617a958-0601-11e8-ae97-52543be04c81",
  "volume": 60,
  "stations": [{ "id": "9617a958-...", "name": "Radio The Voice" }],
  "up_next": [{ "entry": 12, "title": "Aerodynamic", "subtitle": "Daft Punk" }]
}
```

| Field | Meaning |
|---|---|
| `state` | `"off"` (daemon stopped), `"idle"` (nothing loaded), `"loading"` (Resolving, Loading, Connecting), `"playing"`, `"paused"` |
| `kind` | `"track"`, `"station"`, or `null` |
| `title` | Track title. For a station, the ICY title when present, else the station name. Empty when nothing is current. |
| `subtitle` | Track artists joined with ", ". For a station with an ICY title, the station name; otherwise empty. |
| `station_id` | The current station's id, else `null`. |
| `volume` | 0 to 130. |
| `stations` | Favorite stations, in favorites order. |
| `up_next` | Up to 10 queue rows after the cursor. `entry` is the `EntryId` number. |

When `now` is `None` but the queue has a cursor (a restored session), `state` is `"idle"` and `title`, `subtitle`, and `kind` describe the cursor's item.

## `wavelet ctl`

| Command | Effect |
|---|---|
| `status` | Prints the status JSON. Never starts the daemon; prints `{"state":"off"}` when none answers. |
| `toggle` | If nothing is current, the queue is empty, and a favorite station exists, plays the first favorite station. Otherwise `TogglePause`. |
| `play`, `pause` | `SetPaused(false)`, `SetPaused(true)` |
| `next`, `prev` | `Next`, `Prev` |
| `volume N`, `volume +N`, `volume -N` | `Volume(At)` or `Volume(By)` |
| `station ID` | `PlayStations` with every favorite station and `start = ID`, so next and prev cycle favorites. Exit 1 if ID is not a favorite. |
| `jump ENTRY` | `Jump(EntryId)` |
| `quit` | `Shutdown`. Does not start a daemon. |

Every command except `status` and `quit` starts the daemon when none answers. Exit codes are 0 for success, 1 for a daemon or lookup error, and 2 for usage. Errors go to stderr.

## TUI

- The TUI connects with the same start-if-needed path and renders from the snapshot stream, as today.
- `q` closes the TUI and leaves playback running. `Q` (action `quit-all`) sends `Shutdown`, then closes.
- If the daemon's stream ends, the TUI exits with "the wavelet daemon stopped" on stderr.
- MPRIS, session restore, the Radio Atlas import, and `audio_device` and `volume` from `config.toml` move to the daemon. The TUI keeps layout, theme, images, keys, and the spectrum.

## Bar widget

`plugin/bsmail.wavelet/` holds `manifest.json` and `BarWidget.qml`. `wavelet plugin install` embeds both with `include_str!`, replaces `@WAVELET_BIN@` in the QML with `current_exe()`, writes them to `~/.config/omarchy/plugins/bsmail.wavelet/`, then runs `omarchy-shell -q shell rescanPlugins` and `omarchy-shell -q shell putBarWidget bsmail.wavelet '{"section":"right","after":"akshar.radio-atlas"}'`. The full path matters because the bar's PATH has no `~/.cargo/bin`.

| Input on the icon | Result |
|---|---|
| Hover | Tooltip with title, subtitle, and volume |
| Left click | `omarchy-launch-or-focus-tui --app-id=org.omarchy.wavelet @WAVELET_BIN@` |
| Middle click | `ctl toggle` |
| Scroll | `ctl volume +5` or `-5`, coalesced while a call runs |
| Right click | Popup card |

The popup has the title and subtitle, prev, play or pause, and next buttons, a volume slider, two tabs (Stations, Up next), and an "Open wavelet" button. A station row click runs `ctl station ID`. An up-next row click runs `ctl jump ENTRY`. The widget reads `status.json` with `FileView` and treats a missing file as `"off"`.
