#!/usr/bin/env bash
# Drops files on the vendored egui-baseview through XDND under Xvfb, never on the real display (fork patch 11).
#
#   vendor/tests/drop-probe/run.sh                     the fork, then the negative control
#   vendor/tests/drop-probe/run.sh --control-at REV    the control is vendor/egui-baseview at git revision REV instead
#
# The fork runs in both layouts (editor created inside the host window, and created without a parent and reparented,
# the CLAP way) and must pass every check (drop-probe exit 0). The negative control is the same probe built against
# upstream egui-baseview 0.9.0, unpacked from its .crate in cargo's registry cache with the sha256 vendor/README.md
# records (as vendor/check-pristine.sh does), or against REV's tree with --control-at (the commit before patch 11 gives
# the fork without it). It must fail its [egui] checks and pass its [x11] ones (exit 1): baseview takes the drop either
# way, and only the patch hands it to egui.
#
# Builds under the shared build lock with at most 3 jobs, into $CARGO_TARGET_DIR/drop-probe if that is set, else
# target/ here; needs xvfb-run. Exit status 0 when the fork passed and the control failed as expected.
set -Eeuo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
control_rev=
while [ $# -gt 0 ]; do
  case $1 in
    --control-at) control_rev=${2:?--control-at needs a revision}; shift ;;
    *) echo "usage: $0 [--control-at REV]" >&2; exit 2 ;;
  esac
  shift
done
unset DISPLAY WAYLAND_DISPLAY
export CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-3}
out=${CARGO_TARGET_DIR:+$CARGO_TARGET_DIR/drop-probe}
out=${out:-$here/target}
mkdir -p "$out"

# The control's egui-baseview.
control_src=$out/control-src
rm -rf "$control_src"
mkdir -p "$control_src"
if [ -n "$control_rev" ]; then
  git -C "$here" archive "$control_rev" vendor/egui-baseview | tar -x -C "$control_src"
  control_dir=$control_src/vendor/egui-baseview
  control_name="vendor/egui-baseview at $control_rev"
else
  sha=de6df8bd2f1c6b481ab6c029f741980d1a7b409053178fb9459053ae457828fe
  crate=$(ls "${CARGO_HOME:-$HOME/.cargo}"/registry/cache/*/egui-baseview-0.9.0.crate 2>/dev/null | head -n 1 || true)
  if [ -z "$crate" ]; then
    echo "drop-probe: egui-baseview-0.9.0.crate is not in cargo's registry cache (run cargo fetch)" >&2
    exit 2
  fi
  if [ "$(sha256sum "$crate" | cut -d' ' -f1)" != "$sha" ]; then
    echo "drop-probe: $crate does not have the sha256 vendor/README.md records" >&2
    exit 2
  fi
  tar -xzf "$crate" -C "$control_src"
  control_dir=$control_src/egui-baseview-0.9.0
  control_name="upstream egui-baseview 0.9.0"
fi

flock -s /tmp/enth-quiet.lock cargo build --manifest-path "$here/Cargo.toml" --locked --target-dir "$out/fork"
# A [patch] given on the command line takes precedence over the manifest's.
flock -s /tmp/enth-quiet.lock cargo build --manifest-path "$here/Cargo.toml" --locked --target-dir "$out/control" \
  --config "patch.crates-io.egui-baseview.path=\"$control_dir\""

run() {
  local variant=$1 want=$2
  shift 2
  echo "== drop-probe $variant $*"
  local status=0
  xvfb-run -a -s "-screen 0 1920x1080x24" "$out/$variant/debug/drop-probe" "$@" || status=$?
  if [ "$status" -eq "$want" ]; then
    echo "-- $variant $*: exit $status, as expected"
  else
    echo "-- $variant $*: exit $status, expected $want"
    ok=false
  fi
}

ok=true
run fork 0
run fork 0 --reparent
echo "== negative control: $control_name"
run control 1
run control 1 --reparent
if $ok; then
  echo "drop-probe: the fork passed every check, and the control lost the drops in egui only"
  exit 0
fi
echo "drop-probe: FAILED"
exit 1
