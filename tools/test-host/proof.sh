#!/usr/bin/env bash
# Proves every enth-test-host mode on a plugin (spike-shell by default) together with its negative control:
# each check must pass on the plugin as it is and must FAIL once a fault is injected, or it proves nothing.
# docs/spikes/test-host.md ("Proof on spike-shell") explains each line. The exit status is the verdict:
# 0 means every command exited with the status it was expected to.
#
#   tools/test-host/proof.sh [--plugin FILE] [--host BIN] [--out DIR] [--secs N] [--churn N] [--no-editor] [--no-timing]
#
# Prerequisites (this script builds nothing):
#   cargo xtask bundle spike-shell --release          # target/bundled/spike-shell.clap
#   cargo build --release -p enth-test-host           # target/release/enth-test-host
# P1b lane INT retired spike-shell from the workspace, so its bundle now comes from a checkout before that (main
# aa0fec0 or earlier: `cargo xtask bundle spike-shell --release` there), passed with --plugin. The controls below are
# written for spike-shell (it does not render the engine's patches, for one), so they do not carry over to enth-plugin.
#
# Timings (process-stats) run under the exclusive build lock, `flock -x /tmp/enth-quiet.lock`, so nothing builds
# meanwhile; --no-timing skips them. To hold the lock for the whole run instead of per command:
#   flock -x /tmp/enth-quiet.lock env ENTH_PROOF_LOCKED=1 tools/test-host/proof.sh
# The editor variant of process-stats needs xvfb-run and is skipped without it.
# No window ever opens on the real display: DISPLAY and WAYLAND_DISPLAY are removed, and the one command that
# opens a window runs under xvfb-run.
set -Eeuo pipefail
unset DISPLAY WAYLAND_DISPLAY

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
plugin=$root/target/bundled/spike-shell.clap
host=$root/target/release/enth-test-host
out=$root/target/test-host-proof
secs=20
churn_cycles=200
editor=1
timing=1
while [ $# -gt 0 ]; do
  case "$1" in
    --plugin) plugin=$2; shift 2 ;;
    --host) host=$2; shift 2 ;;
    --out) out=$2; shift 2 ;;
    --secs) secs=$2; shift 2 ;;
    --churn) churn_cycles=$2; shift 2 ;;
    --no-editor) editor=0; shift ;;
    --no-timing) timing=0; shift ;;
    *) echo "usage: $0 [--plugin FILE] [--host BIN] [--out DIR] [--secs N] [--churn N] [--no-editor] [--no-timing]" >&2; exit 2 ;;
  esac
done
[ -x "$host" ] || { echo "proof: $host is not built (cargo build --release -p enth-test-host)" >&2; exit 2; }
[ -f "$plugin" ] || { echo "proof: $plugin is not built (cargo xtask bundle spike-shell --release)" >&2; exit 2; }
mkdir -p "$out"
golden=$root/tools/test-host/golden
chord="C4@0+1.5:0.8, E4@0+1.5:0.8, G4@0+1.5:0.8"
failures=0
checked=0

# expect <exit status wanted> <name> <command words...>: run the host, keep its JSON, compare the exit status.
run() {
  local want=$1 name=$2
  shift 2
  local got=0
  checked=$((checked + 1))
  "$@" --json-out "$out/$name.json" >/dev/null 2>"$out/$name.err" || got=$?
  if [ "$got" -eq "$want" ]; then
    printf 'ok      %-40s exit %s\n' "$name" "$got"
  else
    printf 'FAILED  %-40s exit %s, wanted %s (see %s)\n' "$name" "$got" "$want" "$out/$name.err"
    failures=$((failures + 1))
  fi
}
h() { "$host" "$@"; }

# why <name> <python expression over the report d>: the failing run failed for the reason the fault was injected for
# (a control that fails for some other reason would prove nothing).
why() {
  local name=$1 expr=$2
  checked=$((checked + 1))
  if python3 -I -c "import json,sys; d=json.load(open(sys.argv[1])); sys.exit(0 if ($expr) else 1)" "$out/$name.json" 2>/dev/null; then
    printf 'ok      %-40s because %s\n' "$name" "$expr"
  else
    printf 'FAILED  %-40s expected: %s\n' "$name" "$expr"
    failures=$((failures + 1))
  fi
}

echo "proof: host $host"
echo "proof: plugin $plugin"
echo "proof: uptime $(uptime)"
echo

echo "== 1. --state: a patch document loaded into the plugin's persisted field"
run 0 state-patch h render "$plugin" --notes "$chord" --dur 0.6 --state "$golden/spike-shell.patch.json" --save-state "$out/state-patch.state" --check-roundtrip
run 1 state-wrong-field h render "$plugin" --notes "$chord" --dur 0.6 --state "$golden/spike-shell.patch.json" --state-field not-a-field --save-state "$out/state-wrong-field.state" --check-roundtrip
why state-wrong-field "d['state']['patch_round_trip']['saved_field_present'] is False"
printf '%s\n' 'this is not json' >"$out/not-json.txt"
run 2 state-not-json h render "$plugin" --notes "$chord" --dur 0.6 --state "$out/not-json.txt" --state-format patch
run 0 state-blob h render "$plugin" --notes "$chord" --dur 0.6 --state "$golden/spike-shell.state"

echo "== 2. cli-equal: engine golden cases through the plugin"
rm -rf "$out/own-expected"
mkdir -p "$out/own-expected"
run 0 cli-equal-block-independent h cli-equal "$plugin" --no-expected
run 0 cli-equal-write-own h cli-equal "$plugin" --no-expected --write-expected "$out/own-expected"
run 0 cli-equal-own-expected h cli-equal "$plugin" --expected "$out/own-expected"
run 1 cli-equal-flip-bit h cli-equal "$plugin" --expected "$out/own-expected" --inject flip-bit
why cli-equal-flip-bit "d['differs'] == d['compared'] > 0 and d['equal'] == 0"
run 1 cli-equal-engine-hashes h cli-equal "$plugin"
why cli-equal-engine-hashes "d['differs'] > 0 and d['equal'] == 0 and d['block_dependent'] == 0"

echo "== 3. churn: instance lifetimes"
run 0 churn h churn "$plugin" -n "$churn_cycles"
run 1 churn-leak-16kb h churn "$plugin" -n "$churn_cycles" --inject leak-kb=16
why churn-leak-16kb "d['checks'] == {'pss': False, 'threads': True, 'fds': True}"
run 1 churn-fd h churn "$plugin" -n "$churn_cycles" --inject fd
why churn-fd "d['checks']['fds'] is False and d['checks']['pss'] and d['checks']['threads']"
run 1 churn-thread h churn "$plugin" -n "$churn_cycles" --inject thread
why churn-thread "d['checks']['threads'] is False and d['checks']['fds']"

echo "== 4. bursts: event floods (INPUT_EVENT_CAPACITY)"
run 0 bursts-params-first h bursts "$plugin" --order params-first --capacity 1024 --counts 16,64,256,1024
run 1 bursts-drop-after-40 h bursts "$plugin" --order params-first --capacity 1024 --counts 16,64,256,1024 --inject drop-after=40
why bursts-drop-after-40 "any('not all applied' in f for t in d['trials'] for f in t['failures'])"
# A real finding, not a control: spike-shell loses a note event that sits before a parameter event on the same frame.
run 1 bursts-notes-first-finding h bursts "$plugin" --order notes-first --capacity 1024 --counts 16,64,256,1024
why bursts-notes-first-finding "any(t['note_end']['unended'] > 0 for t in d['trials'])"

echo "== 5. note-end: NOTE_END completeness and timing"
run 0 note-end h note-end "$plugin"
run 1 note-end-drop-every-3rd h note-end "$plugin" --inject drop-end=3
why note-end-drop-every-3rd "all(b['note_end']['unended'] > 0 for b in d['blocks'])"
run 1 note-end-70-frames-late h note-end "$plugin" --inject late-end=70
why note-end-70-frames-late "any(b['note_end']['outside_block'] > 0 for b in d['blocks'])"

if [ "$timing" -eq 1 ]; then
  echo "== 6. process-stats: timing under the exclusive build lock (${secs} s per run)"
  if [ -n "${ENTH_PROOF_LOCKED:-}" ]; then
    timed() { "$@"; }   # the caller holds the lock for the whole run
  else
    timed() { flock -x -w 900 /tmp/enth-quiet.lock "$@"; }
  fi
  run 0 process-stats-rr10 timed "$host" process-stats "$plugin" --secs "$secs" --warmup 5 --rt 10 --require-rt
  run 0 process-stats-normal-priority timed "$host" process-stats "$plugin" --secs "$secs" --warmup 5 --rt none
  run 1 process-stats-rt99-refused timed "$host" process-stats "$plugin" --secs 3 --warmup 1 --rt 99 --require-rt
  why process-stats-rt99-refused "any('real-time' in f for f in d['failures']) and not d['real_time']"
  run 1 process-stats-impossible-budget timed "$host" process-stats "$plugin" --secs 3 --warmup 1 --max-p99-us 1
  why process-stats-impossible-budget "any('p99 ' in f for f in d['failures'])"
  run 1 process-stats-injected-tail timed "$host" process-stats "$plugin" --secs "$secs" --warmup 5 --inject busy-us=400,every=200 --max-p999-us 333
  why process-stats-injected-tail "any('p99.9' in f for f in d['failures'])"
  run 0 process-stats-4-instances-2-threads timed "$host" process-stats "$plugin" --secs "$secs" --warmup 5 --instances 4 --threads 2
  if [ "$editor" -eq 1 ] && command -v xvfb-run >/dev/null; then
    run 0 process-stats-editor-open timed xvfb-run -a -s "-screen 0 1280x800x24" "$host" process-stats "$plugin" --secs "$secs" --warmup 5 --rt 10 --editor
  fi
fi

echo "== 7. latency: parameter event to audible change"
run 0 latency-gain h latency "$plugin" --param "Macro 1" --from 1.0 --to 0.2 --max-onset-ms 0.5 --max-audible-ms 2
run 0 latency-cutoff h latency "$plugin" --param "Slot 1" --from 0.9 --to 0.2
run 1 latency-inert-param h latency "$plugin" --param "Macro 2" --to 0.9
why latency-inert-param "any('no audible change' in f for f in d['failures'])"
run 1 latency-event-3-blocks-late h latency "$plugin" --param "Macro 1" --from 1.0 --to 0.2 --inject delay-blocks=3 --max-onset-ms 1
why latency-event-3-blocks-late "all(o['onset_frames'] == 192 for o in d['offsets'])"

echo
if [ "$failures" -eq 0 ]; then
  echo "PROOF PASSED: $checked commands exited as expected; reports are in $out"
else
  echo "PROOF FAILED: $failures of $checked commands did not exit as expected" >&2
  exit 1
fi
