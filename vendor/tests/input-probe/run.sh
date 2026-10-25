#!/usr/bin/env bash
# Runs the input probes under Xvfb, never on the real display (fork patches 12 and 13).
#
#   vendor/tests/input-probe/run.sh                     both probes
#   vendor/tests/input-probe/run.sh tap                 tap-probe only (patch 12)
#   vendor/tests/input-probe/run.sh paste               paste-probe only (patch 13)
#   vendor/tests/input-probe/run.sh paste --control-at REV
#                                                       the paste control is vendor/egui-baseview at git revision REV
#
# tap-probe runs in both layouts (editor created inside the host window, and created without a parent and reparented,
# the CLAP way), with the pad's keyboard claim and without it (`--no-claim`, the negative control for the claim: Space
# must keep reaching the host); all four must pass (exit 0). The arrival stamps have no build without the patch to
# compare with (upstream has no such API): each run compares them with the frame time, which is what upstream gives.
#
# paste-probe runs on the fork (exit 0) and, as the negative control, built against upstream egui-baseview 0.9.0,
# unpacked from its .crate in cargo's registry cache with the sha256 vendor/README.md records (or against REV's tree
# with --control-at; the commit before patch 13 gives the fork without it). The control must fail (exit 1): its Paste
# button reads nothing and pastes nothing.
#
# Builds under the shared build lock with at most 3 jobs, into $CARGO_TARGET_DIR/input-probe if that is set, else
# target/ here; needs xvfb-run. Exit status 0 when every run ended as expected.
set -Eeuo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
which=all
control_rev=
while [ $# -gt 0 ]; do
  case $1 in
    tap|paste|all) which=$1 ;;
    --control-at) control_rev=${2:?--control-at needs a revision}; shift ;;
    *) echo "usage: $0 [tap|paste|all] [--control-at REV]" >&2; exit 2 ;;
  esac
  shift
done
unset DISPLAY WAYLAND_DISPLAY
export CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-3}
out=${CARGO_TARGET_DIR:+$CARGO_TARGET_DIR/input-probe}
out=${out:-$here/target}
mkdir -p "$out"

flock -s /tmp/enth-quiet.lock cargo build --manifest-path "$here/Cargo.toml" --locked --target-dir "$out/fork"

ok=true
run() {
  local variant=$1 probe=$2 want=$3
  shift 3
  echo "== $probe ($variant) $*"
  local status=0
  xvfb-run -a -s "-screen 0 1920x1080x24" "$out/$variant/debug/$probe" "$@" || status=$?
  if [ "$status" -eq "$want" ]; then
    echo "-- $probe ($variant) $*: exit $status, as expected"
  else
    echo "-- $probe ($variant) $*: exit $status, expected $want"
    ok=false
  fi
}

if [ "$which" != paste ]; then
  run fork tap-probe 0
  run fork tap-probe 0 --reparent
  echo "== negative control for the claim: the pad takes egui's focus but does not claim"
  run fork tap-probe 0 --no-claim
  run fork tap-probe 0 --no-claim --reparent
fi

if [ "$which" != tap ]; then
  # The control's egui-baseview (only paste-probe is built against it: tap-probe needs patch 12's API).
  control_src=$out/control-src
  rm -rf "$control_src"
  mkdir -p "$control_src"
  if [ -n "$control_rev" ]; then
    git -C "$(git -C "$here" rev-parse --show-toplevel)" archive "$control_rev" vendor/egui-baseview | tar -x -C "$control_src"
    control_dir=$control_src/vendor/egui-baseview
    control_name="vendor/egui-baseview at $control_rev"
  else
    sha=de6df8bd2f1c6b481ab6c029f741980d1a7b409053178fb9459053ae457828fe
    crate=$(ls "${CARGO_HOME:-$HOME/.cargo}"/registry/cache/*/egui-baseview-0.9.0.crate 2>/dev/null | head -n 1 || true)
    if [ -z "$crate" ]; then
      echo "input-probe: egui-baseview-0.9.0.crate is not in cargo's registry cache (run cargo fetch)" >&2
      exit 2
    fi
    if [ "$(sha256sum "$crate" | cut -d' ' -f1)" != "$sha" ]; then
      echo "input-probe: $crate does not have the sha256 vendor/README.md records" >&2
      exit 2
    fi
    tar -xzf "$crate" -C "$control_src"
    control_dir=$control_src/egui-baseview-0.9.0
    control_name="upstream egui-baseview 0.9.0"
  fi
  # A [patch] given on the command line takes precedence over the manifest's.
  flock -s /tmp/enth-quiet.lock cargo build --manifest-path "$here/Cargo.toml" --locked --target-dir "$out/control" \
    --bin paste-probe --config "patch.crates-io.egui-baseview.path=\"$control_dir\""

  run fork paste-probe 0
  echo "== negative control for the paste: $control_name"
  run control paste-probe 1
fi

if $ok; then
  echo "input-probe: every run ended as expected"
  exit 0
fi
echo "input-probe: FAILED"
exit 1
