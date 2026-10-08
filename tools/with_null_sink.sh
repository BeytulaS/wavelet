#!/usr/bin/env bash
# usage: tools/with_null_sink.sh <command...>
# Runs the command with a PipeWire null sink loaded, so nothing is audible.
# The sink is unloaded on exit, even on failure. The command sees
# WAVELET_TEST_AUDIO_DEVICE and WAVELET_AUDIO_DEVICE set to the sink.
set -u
mod=$(pactl load-module module-null-sink sink_name=wavelet_test sink_properties=device.description=wavelet_test) || exit 1
trap 'pactl unload-module "$mod"' EXIT
export WAVELET_TEST_AUDIO_DEVICE=pipewire/wavelet_test WAVELET_AUDIO_DEVICE=pipewire/wavelet_test
"$@"
