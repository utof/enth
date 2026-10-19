#!/usr/bin/env bash
# Runs the keyboard-focus probe under Xvfb, never on the real display. Exit status: 0 when every check passed.
#
#   vendor/tests/focus-probe/run.sh            all four runs: while-typing and on-click, each direct and reparented
#   vendor/tests/focus-probe/run.sh while-typing --reparent     one run (arguments go to focus-probe)
#
# `on-click` is upstream's behaviour: the negative control. It passes when the probe sees the Space key swallowed.
# Builds under the shared build lock with at most 3 jobs; needs xvfb-run.
set -Eeuo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
unset DISPLAY WAYLAND_DISPLAY
export CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-3}

flock -s /tmp/enth-quiet.lock cargo build --manifest-path "$here/Cargo.toml" --locked
probe=$here/target/debug/focus-probe

run() {
  echo "== focus-probe $*"
  xvfb-run -a -s "-screen 0 1920x1080x24" "$probe" "$@"
}

if [ $# -gt 0 ]; then
  run "$@"
else
  run while-typing
  run while-typing --reparent
  run on-click
  run on-click --reparent
fi
