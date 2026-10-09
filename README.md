# wavelet

wavelet is a terminal music player for YouTube Music and internet radio. It
plays without ads or a Google account, uses vim keys, and follows the Omarchy
theme or the colors of the album that is playing.

Search YouTube Music, press Enter, and wavelet keeps playing a radio of
similar songs. Search radio-browser.info for a station and it plays live, with
the song title when the station sends one. Media keys and `playerctl` work
through MPRIS.

Playback runs in a background daemon, so music keeps playing after you close
the terminal. `wavelet ctl` and the Omarchy bar widget control the same
daemon.

## Install

wavelet needs `mpv`, `yt-dlp`, and, for the spectrum, `pw-record` from
PipeWire. It runs on Linux.

```sh
cargo install --path .
```

Run `wavelet`. The first run starts the daemon.

## Keys

| Keys | Action |
|---|---|
| `j` `k` `gg` `G` `Ctrl-d` `Ctrl-u` | Move (counts work: `5j`) |
| `Tab` `1` `2` `3` | YouTube, Radio, Favorites |
| `gq` | Queue view (Enter jumps, `dd` removes) |
| `/` | Search (Enter runs, Esc cancels) |
| `Enter` | Play the selected row |
| `Space` | Play or pause |
| `n` `b` | Next; restart or previous |
| `H` `L` | Seek 5 s back or forward (songs only) |
| `-` `+` | Volume |
| `a` | Play the selection next |
| `f` | Favorite or unfavorite |
| `w` `t` `v` | Cycle layout, cycle theme, toggle the spectrum |
| `?` | Show every key |
| `q` | Close wavelet; playback continues |
| `Q` | Stop playback and close |

## Daemon and `wavelet ctl`

`wavelet` starts `wavelet daemon` when none is running. The daemon owns mpv,
MPRIS, and the saved session. When you install a new build, the next
`wavelet` or `wavelet ctl` replaces the old daemon, and the session comes back
paused.

`wavelet ctl` sends one command per run, for keybinds and scripts:

| Command | Effect |
|---|---|
| `status` | Print the bar status as JSON. Prints `{"state":"off"}` when no daemon runs. |
| `toggle` | Play or pause. With nothing queued, play the first favorite station. |
| `play`, `pause`, `next`, `prev` | As named |
| `volume N`, `volume +N`, `volume -N` | Set or change the volume (0 to 130) |
| `station ID` | Play a favorite station; next and prev cycle the favorites |
| `jump ENTRY` | Play a queue entry by its number in `status` |
| `quit` | Stop the daemon |

Every command except `status` and `quit` starts the daemon when none runs.
Exit codes are 0 for success, 1 for a daemon or lookup error, and 2 for
usage.

`wavelet plugin install` adds the wavelet widget to the Omarchy bar, next to
Radio Atlas. Hover shows what is playing, middle click plays or pauses, scroll
changes the volume, and right click opens a card with stations and the queue.

## Configuration

| Path | Contents |
|---|---|
| `~/.config/wavelet/config.toml` | Layout, theme, audio device, volume, spectrum. Written with commented defaults on first run. |
| `~/.config/wavelet/keymap.toml` | Key overrides, such as `"x" = "toggle-pause"`. Press `?` for the action names. |
| `~/.local/share/wavelet/library.json` | Favorite songs and stations |
| `~/.local/state/wavelet/session.json` | The queue and volume, restored on start without playing |
| `~/.local/state/wavelet/daemon.log` | The daemon's log, when `wavelet` started it |
| `$XDG_RUNTIME_DIR/wavelet/` | `wavelet.sock` (clients), `mpv.sock`, `status.json` (the bar), `daemon.lock` |

A bad value in either file shows a warning and falls back to the default for
that key only. On first start, wavelet imports favorite stations from Radio
Atlas (`~/.local/share/radio-atlas/`) once.

Album art uses the terminal's image protocol: sixel in foot, kitty in kitty and
ghostty. Inside herdr it falls back to text cells, because herdr shows images
only when its own terminal speaks kitty. Set `images = "kitty"` in
`config.toml` if herdr runs in kitty or ghostty. The `WAVELET_PROTOCOL`
environment variable overrides the config. Detached tmux sessions need
`WAVELET_PROTOCOL=halfblocks`, because they do not answer the graphics query.

## Development

`cargo test` runs the reducer, parser, and store tests with no network, and
`tests/daemon.rs`, which starts the daemon (with an idle mpv) under private
XDG directories.
These tools drive the real program into a PipeWire null sink, so nothing is
audible:

- `tools/with_null_sink.sh cargo test --test player_live -- --ignored` runs the
  player against a real mpv.
- `tools/e2e.sh` drives the TUI in a private tmux server with temporary config
  and data directories and a private runtime directory, so it starts its own
  daemon. `tools/e2e.sh ctl status` talks to that daemon; `stop` quits it.
- `tools/drive.sh` captures screens as PNG through `tools/ansi2png.py`.
- `tools/mutate_core.py` breaks core behaviors one at a time and checks that a
  test fails for each.

`docs/design.md` describes the architecture.

## Credits

The theme system, color extraction, gradients, cross-fade, cover handling,
and spectrum analyzer are adapted from [Myx](https://github.com/HaseebKhalid1507/Myx)
by Haseeb Khalid (MIT). The FFT band analysis comes from
[spotify-player](https://github.com/aome510/spotify-player) by Thang Pham (MIT)
by way of Myx. See `NOTICE`. Streams are resolved by
[yt-dlp](https://github.com/yt-dlp/yt-dlp) and played by [mpv](https://mpv.io).
Station data comes from [radio-browser.info](https://www.radio-browser.info).
