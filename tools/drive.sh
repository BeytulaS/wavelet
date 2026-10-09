#!/usr/bin/env bash
# usage: tools/drive.sh <outprefix> <keys-after-play...>
# Plays "daft punk around the world" into a null sink and captures the screen.
# "@" in the key list captures <outprefix>-<n>.ansi and .png into $OUTDIR
# (default /tmp/wavelet-captures). QUERY and WAIT override the search and the
# seconds to wait for audio. Config, data, state, and the runtime dir are
# private, so the drive never meets a real wavelet daemon.
set -u
OUT=$1; shift
S=${OUTDIR:-/tmp/wavelet-captures}; mkdir -p "$S"
root=$(cd "$(dirname "$0")/.." && pwd); BIN=$root/target/release/wavelet
xdg=$(mktemp -d)
uid=$(id -u); rt=/run/user/$uid/wl-drive-$$; mkdir -p "$rt"
envs="XDG_CONFIG_HOME=$xdg/config XDG_DATA_HOME=$xdg/data XDG_STATE_HOME=$xdg/state \
XDG_RUNTIME_DIR=$rt PIPEWIRE_RUNTIME_DIR=/run/user/$uid PULSE_RUNTIME_PATH=/run/user/$uid/pulse \
DBUS_SESSION_BUS_ADDRESS=unix:path=$rt/no-bus WAVELET_PROTOCOL=halfblocks WAVELET_AUDIO_DEVICE=pipewire/wavelet_test"
tm="tmux -L wavelet-drive"
mod=$(pactl load-module module-null-sink sink_name=wavelet_test sink_properties=device.description=wavelet_test)
trap 'env $envs "$BIN" ctl quit; $tm kill-server 2>/dev/null; pactl unload-module $mod; rm -rf "$xdg" "$rt"' EXIT
$tm new-session -d -s wdrive -x 160 -y 45 "env $envs $BIN"
sleep 1.5
$tm send-keys -t wdrive / ; sleep 0.3; $tm send-keys -t wdrive -l "${QUERY:-daft punk around the world}"; $tm send-keys -t wdrive Enter
sleep 2; $tm send-keys -t wdrive Enter; sleep "${WAIT:-9}"
for k in "$@"; do
  if [ "$k" = "@" ]; then n=$((${n:-0}+1)); $tm capture-pane -p -e -N -t wdrive > "$S/$OUT-$n.ansi" 2>/dev/null; python3 -I "$(dirname "$0")/ansi2png.py" "$S/$OUT-$n.ansi" "$S/$OUT-$n.png"; echo "captured $S/$OUT-$n"
  else $tm send-keys -t wdrive "$k"; sleep 2; fi
done
$tm send-keys -t wdrive q; sleep 1
