#!/usr/bin/env bash
# Does a runtime parameter rename (patch 1) reach a CLAP host? Builds a plugin on the vendored nice-plug (a copy of the
# Phase 0 rename spike: setting Slot 64 above 0.9 renames Slot 1 to "Osc 1 · Cutoff" and asks the host to rescan) and a
# small host (clack-host) that processes on its own audio thread, as a real host does, services request_callback on
# its main thread and reads clap_plugin_params.get_info before and after.
#
#   vendor/tests/rename-check/run.sh
#
# Runs the host twice. With the trigger it must see a parameter whose name changed and a
# clap_host_params.rescan(INFO | TEXT) call; without it (the negative control) it must see neither.
# No window is opened and no display is used. Exit status 0 when both runs behaved.
set -Eeuo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
unset DISPLAY WAYLAND_DISPLAY
export CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-3}

flock -s /tmp/enth-quiet.lock cargo build --release --locked --manifest-path "$here/plugin/Cargo.toml"
flock -s /tmp/enth-quiet.lock cargo build --release --locked --manifest-path "$here/host/Cargo.toml"
plugin=$here/plugin/target/release/libspike_np_rename.so
host=$here/host/target/release/rename-host

echo "== with the trigger"
"$host" "$plugin"
echo "== without the trigger (negative control)"
"$host" "$plugin" --no-trigger
