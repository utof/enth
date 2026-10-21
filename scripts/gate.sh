#!/usr/bin/env bash
# enth merge gates. Judge them by the exit status and nothing else: 0 means every check passed.
#
#   scripts/gate.sh lane <worktree>   Fast, for a lane before it reports: fmt, workspace clippy (-D warnings),
#                                     layers, wasm-check, and `cargo test` of the crates the change touches plus
#                                     every crate that depends on them (`cargo xtask affected`, against the merge
#                                     base with main; ENTH_GATE_BASE=<rev> names another base). When the change
#                                     touches content/lessons/, also `cargo xtask lesson-check` (every pack) and
#                                     `lesson-check --cross --solve` on the packs it touches.
#   scripts/gate.sh full <worktree>   Everything, for the coordinator before a merge: the lane checks with the
#                                     whole workspace's tests, plus cargo-deny (bans, licenses, sources), golden,
#                                     host-render and validate on the default plugin (`cargo xtask host-render`,
#                                     `cargo xtask validate`; see xtask/src/plugin.rs for the default).
#
# Not part of either: `cargo xtask reaper-test` (headless REAPER under Xvfb, a local gate), `cargo xtask bench`
# (valgrind), `golden --wasm`, and the advisories check (it needs the network; CI runs it as an allowed failure).
#
# Rules this script keeps (P1a's gate lesson):
#   - `set -euo pipefail`, and no pipe whose first stage's status matters: every step's output goes to a log file
#     through a redirect, so `cmd | tail && echo ok` (which reports tail's status) cannot happen here;
#   - a failing step prints its last 60 lines and the script exits with that step's status, so the first failure
#     is the last thing on screen;
#   - "GATE PASSED" is printed last, and only when every step passed;
#   - both modes print the commit they checked, with "+dirty" when the working tree differs from it.
#
# Builds run under a shared lock (`flock -s /tmp/enth-quiet.lock`): a timing measurement holds it exclusively and
# nothing builds while it runs. At most 3 build jobs (CARGO_BUILD_JOBS), warnings are errors (CARGO_BUILD_WARNINGS),
# and no display variable reaches any child. Logs: <worktree>/target/gate-logs/. Foreground only: a cold full gate
# takes several minutes, so give it a timeout above that (the first run in a fresh worktree is the slow one).
set -Eeuo pipefail

self=$(readlink -f -- "${BASH_SOURCE[0]}")
usage() {
  echo "usage: $0 lane|full <worktree>" >&2
  exit 2
}
mode=${1:-}
tree=${2:-}
case "$mode" in
  lane | full) ;;
  *) usage ;;
esac
[ -n "$tree" ] || usage
[ -d "$tree" ] || { echo "gate: $tree is not a directory" >&2; exit 2; }
root=$(cd "$tree" && git rev-parse --show-toplevel)
cd "$root"
[ -f xtask/layers.toml ] || { echo "gate: $root is not an enth worktree (no xtask/layers.toml)" >&2; exit 2; }

if [ -z "${ENTH_GATE_LOCKED:-}" ]; then
  export ENTH_GATE_LOCKED=1
  exec flock -s /tmp/enth-quiet.lock "$self" "$mode" "$root"
fi

export CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-3}
export CARGO_BUILD_WARNINGS=deny
export CARGO_TERM_COLOR=never
unset DISPLAY WAYLAND_DISPLAY

trap 'rc=$?; echo "GATE FAILED ($mode): the script itself failed at line $LINENO (exit $rc)" >&2' ERR

sha=$(git rev-parse --short=12 HEAD)
dirty=""
if [ -n "$(git status --porcelain)" ]; then dirty="+dirty"; fi
echo "gate $mode: $root @ $sha$dirty ($(git rev-parse --abbrev-ref HEAD))"

logdir=$root/target/gate-logs
mkdir -p "$logdir"
count=0

# step <name> <command...>: run it with its output in a log; on failure show the log's tail and exit with its status.
step() {
  local name=$1
  shift
  count=$((count + 1))
  local log=$logdir/$mode-$count-$name.log
  local started=$SECONDS
  echo "== [$name] $*"
  if "$@" >"$log" 2>&1; then
    echo "   ok ($((SECONDS - started)) s)"
  else
    local rc=$?
    echo "   FAILED (exit $rc, $((SECONDS - started)) s); the last 60 lines of $log:"
    tail -n 60 "$log"
    echo "GATE FAILED ($mode): $name (exit $rc) @ $sha$dirty" >&2
    exit "$rc"
  fi
}

# The `test result:` lines of the last step's log, summed. Display only: the step's own status already decided.
test_totals() {
  awk '/^test result/ { p += $4; f += $6 } END { print "   tests passed:", p + 0, "failed:", f + 0 }' "$logdir/$mode-$count-test.log"
}

# Cheap checks first, so the first failure comes early.
step metadata cargo metadata --format-version 1 --locked
step fmt cargo fmt --all --check
step layers cargo xtask check-layers
step clippy cargo clippy --workspace --all-targets --locked -- -D warnings
step wasm-check cargo xtask wasm-check

if [ "$mode" = lane ]; then
  if [ -n "${ENTH_GATE_BASE:-}" ]; then
    affected=$(cargo xtask affected --base "$ENTH_GATE_BASE")
  else
    affected=$(cargo xtask affected)
  fi
  if [ -z "$affected" ]; then
    echo "== [test] no Rust crate is affected by this change: nothing to test"
  else
    names=()
    packages=()
    for name in $affected; do
      names+=("$name")
      packages+=(-p "$name")
    done
    echo "== crates to test: ${names[*]}"
    step test cargo test --locked "${packages[@]}"
    test_totals
  fi
  # R2 lane LCK: a change under content/lessons/ (against the same base as `affected`: committed, uncommitted and
  # untracked) runs `lesson-check`: the files and the playthrough of every pack, then `--cross --solve` on each pack
  # the change touches (docs/learn.md, "`lesson-check` and the authoring loop").
  lesson_base=${ENTH_GATE_BASE:-main}
  lesson_base=$(git merge-base HEAD "$lesson_base" 2>/dev/null || git merge-base HEAD origin/main)
  lesson_changes=$({ git diff --name-only "$lesson_base" -- content/lessons; git ls-files --others --exclude-standard -- content/lessons; } | sort -u)
  if [ -n "$lesson_changes" ]; then
    lesson_packs=()
    for pack in $(printf '%s\n' "$lesson_changes" | awk -F/ 'NF > 3 { print $3 }' | sort -u); do
      if [ -f "content/lessons/$pack/pack.toml" ]; then lesson_packs+=("$pack"); fi
    done
    step lesson-check cargo xtask lesson-check
    if [ "${#lesson_packs[@]}" -gt 0 ]; then
      step lesson-solve cargo xtask lesson-check --cross --solve "${lesson_packs[@]}"
    fi
  fi
else
  step deny cargo deny --locked check bans licenses sources
  step golden cargo xtask golden
  step test cargo test --workspace --locked
  test_totals
  step host-render cargo xtask host-render
  step validate cargo xtask validate
fi

echo "GATE PASSED ($mode) @ $sha$dirty in $SECONDS s"
