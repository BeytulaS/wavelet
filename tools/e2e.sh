#!/usr/bin/env bash
# Drive the release binary in a private tmux server, with temp XDG dirs, a
# private runtime dir (so it never meets a real wavelet daemon), no session
# bus, and a PipeWire null sink, so nothing is audible and no real config is
# touched.
#
#   tools/e2e.sh start [fresh]   start wavelet (fresh: wipe the temp XDG dirs first)
#   tools/e2e.sh keys <keys...>  tmux send-keys; "-l text" sends literal text
#   tools/e2e.sh cap [file]      plain-text capture (with ANSI to <file> if given)
#   tools/e2e.sh ctl <args...>   wavelet ctl against the test daemon
#   tools/e2e.sh stop            press q, ctl quit, unload the sink
#   tools/e2e.sh procs           list mpv / pw-record processes started for wavelet
set -u
root=$(cd "$(dirname "$0")/.." && pwd)
state=${WAVELET_E2E_DIR:-/tmp/wavelet-e2e}
uid=$(id -u)
rt=/run/user/$uid/wl-e2e
tm="tmux -L wavelet-e2e"
envs="XDG_CONFIG_HOME=$state/xdg/config XDG_DATA_HOME=$state/xdg/data XDG_STATE_HOME=$state/xdg/state \
XDG_RUNTIME_DIR=$rt PIPEWIRE_RUNTIME_DIR=/run/user/$uid PULSE_RUNTIME_PATH=/run/user/$uid/pulse \
DBUS_SESSION_BUS_ADDRESS=unix:path=$rt/no-bus WAVELET_PROTOCOL=halfblocks WAVELET_AUDIO_DEVICE=pipewire/wavelet_test"
bin=$root/target/release/wavelet
case "${1:-}" in
start)
  [ "${2:-}" = fresh ] && rm -rf "$state/xdg"
  mkdir -p "$state/xdg/config" "$state/xdg/data" "$state/xdg/state" "$rt"
  if [ ! -s "$state/sink" ]; then
    pactl load-module module-null-sink sink_name=wavelet_test sink_properties=device.description=wavelet_test >"$state/sink" || exit 1
  fi
  $tm kill-server 2>/dev/null
  $tm new-session -d -s wv -x 160 -y 45 "env $envs $bin 2>$state/stderr; echo EXIT=\$? >>$state/stderr; sleep 600"
  ;;
keys)
  shift
  $tm send-keys -t wv "$@"
  ;;
cap)
  if [ -n "${2:-}" ]; then $tm capture-pane -p -e -N -t wv >"$2"; fi
  $tm capture-pane -p -t wv
  ;;
ctl)
  shift
  env $envs "$bin" ctl "$@"
  ;;
stop)
  $tm send-keys -t wv q 2>/dev/null
  sleep 1.5
  env $envs "$bin" ctl quit
  cat "$state/stderr" 2>/dev/null
  $tm kill-server 2>/dev/null
  if [ -s "$state/sink" ]; then pactl unload-module "$(cat "$state/sink")" && : >"$state/sink"; fi
  rm -rf "$rt"
  ;;
procs)
  pgrep -af "audio-client-name=wavelet .*$rt/|pw-record.*--target wavelet( |$)" || echo "none"
  ;;
*)
  sed -n 2,14p "$0"
  exit 2
  ;;
esac
