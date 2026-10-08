# wavelet

wavelet is a terminal music player for YouTube Music and internet radio. It
plays without ads or a Google account, uses vim keys, and follows the Omarchy
theme or the colors of the album that is playing.

Search YouTube Music, press Enter, and wavelet keeps playing a radio of
similar songs. Search radio-browser.info for a station and it plays live, with
the song title when the station sends one. Media keys and `playerctl` work
through MPRIS.

## Install

wavelet needs `mpv`, `yt-dlp`, and, for the spectrum, `pw-record` from
PipeWire. It runs on Linux.

```sh
cargo install --path .
```

Run `wavelet`.

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
| `q` | Stop and quit |

## Configuration

| Path | Contents |
|---|---|
| `~/.config/wavelet/config.toml` | Layout, theme, audio device, volume, spectrum. Written with commented defaults on first run. |
| `~/.config/wavelet/keymap.toml` | Key overrides, such as `"x" = "toggle-pause"`. Press `?` for the action names. |
| `~/.local/share/wavelet/library.json` | Favorite songs and stations |
| `~/.local/state/wavelet/session.json` | The queue and volume, restored on start without playing |
| `$XDG_RUNTIME_DIR/wavelet/mpv.sock` | mpv's control socket |

A bad value in either file shows a warning and falls back to the default for
that key only. On first start, wavelet imports favorite stations from Radio
Atlas (`~/.local/share/radio-atlas/`) once.

Set `WAVELET_PROTOCOL=halfblocks` to draw album art with text cells. tmux needs
it, because a detached session does not answer the terminal's graphics query.

## Development

`cargo test` runs the reducer, parser, and store tests with no network or mpv.
These tools drive the real program into a PipeWire null sink, so nothing is
audible:

- `tools/with_null_sink.sh cargo test --test player_live -- --ignored` runs the
  player against a real mpv.
- `tools/e2e.sh` drives the TUI in a private tmux server with temporary config
  and data directories.
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
