#!/usr/bin/env bash
# Measures what a file dropped on enth's editor does inside REAPER's floating FX window, under Xvfb, never on the real
# display (patch 11's open check; a measurement, not a pass/fail test beyond "the run finished").
#
#   vendor/tests/drop-probe/reaper-into.sh [clap|vst3]
#
# Needs, in $CARGO_TARGET_DIR (else target/): the REAPER sandbox and bundles that `cargo xtask reaper-test --package
# enth-plugin` leaves (reaper-test/home, bundled/), and drop-probe built by run.sh (drop-probe/fork/debug/drop-probe).
# It adds the plugin to a track, shows its window, and runs `drop-probe --into` against that window from inside REAPER
# (xtask/reaper/common.lua supplies the helpers). Prints drop-probe's report and the script's log; exit 0 when the run
# finished, 1 otherwise.
set -Eeuo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
root=$(git -C "$here" rev-parse --show-toplevel)
format=${1:-clap}
case $format in
  clap) fx="CLAPi: enth (enth)" ;;
  vst3) fx="VST3i: enth (enth)" ;;
  *) echo "usage: $0 [clap|vst3]" >&2; exit 2 ;;
esac
target=${CARGO_TARGET_DIR:-$root/target}
home=$target/reaper-test/home
probe=$target/drop-probe/fork/debug/drop-probe
for need in "$home/reaper" "$probe"; do
  [ -x "$need" ] || { echo "reaper-into: $need is missing (run cargo xtask reaper-test, and drop-probe/run.sh)" >&2; exit 2; }
done
out=$target/drop-probe/reaper-into-$format
rm -rf "$out"
mkdir -p "$out"
unset DISPLAY WAYLAND_DISPLAY

cd "$home"
flock -s /tmp/enth-quiet.lock xvfb-run -a -s "-screen 0 1920x1080x24" \
  env GDK_BACKEND=x11 ENTH_OUT="$out" ENTH_LUA_DIR="$root/xtask/reaper" ENTH_EMPTY_PROJECT="$root/xtask/reaper/empty.rpp" \
      ENTH_LOG=drop-probe-into.log "ENTH_FX=$fx" ENTH_TAG="$format" ENTH_PHASE=drop DROP_PROBE="$probe" \
  timeout -k 5 240 "$home/reaper" -nosplash -newinst "$here/reaper-into.lua" \
  > "$out/reaper.stdout.txt" 2>&1 < /dev/null || true

echo "== script log ($out/drop-probe-into.log)"
cat "$out/drop-probe-into.log" 2>/dev/null || true
echo "== drop-probe --into ($fx)"
cat "$out/drop-probe-into-$format.txt" 2>/dev/null || true
if grep -q $'^data\tfinished' "$out/drop-probe-into.log" 2>/dev/null && ! grep -q $'^check\t[^\t]*\tFAIL' "$out/drop-probe-into.log"; then
  echo "reaper-into: finished"
  exit 0
fi
echo "reaper-into: did not finish cleanly; see $out/reaper.stdout.txt" >&2
exit 1
