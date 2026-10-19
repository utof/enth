#!/usr/bin/env bash
# Opens and closes the editor of an instrumented spike-shell 200 times under Xvfb (never the real display) with
# `enth-test-host editor-cycles`, once against the vendored fork and once against stock crates.io nice-plug (the
# negative control), and checks what the fork patches promise:
#
#   fork:  editor_closed() calls == number of editor opens (patch 2)
#          anonymous PSS growth after close, worst editor run minus the baseline, at most 26 KB per cycle (patch 3; the stock
#          figure is about 86). The anonymous part is what a leak adds. The test host's headline `pss_kb` also counts the
#          shared libraries' pages (Mesa, llvmpipe), which the kernel reclaims and re-faults on a loaded machine: one run in
#          this lane's measurements showed 108 KB per cycle in `pss_kb` and 17.8 in the anonymous part. Both are printed.
#          the thread count after every close is constant
#   stock: editor_closed() is never called, and the PSS slope is above 26 KB per cycle. If it were not, the fork's
#          numbers would prove nothing.
#
#   vendor/tests/editor-cycles/run.sh [cycles]        default 200; each build is a full release build of egui (4 minutes cold)
#
# The plugin (src/) is a copy of spikes/spike-shell with counters for the editor lifecycle; the spikes are frozen, so
# the copy lives here. Reports: target/ under this directory (fork.json, stock.json). Exit status 0 when every check held.
set -Eeuo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
root=$(cd "$here/../../.." && pwd)
cycles=${1:-200}
max_kb=26
unset DISPLAY WAYLAND_DISPLAY
export CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-3}

out=$here/target
mkdir -p "$out"

( cd "$root" && flock -s /tmp/enth-quiet.lock cargo build --release -p enth-test-host --locked )
host=$root/target/release/enth-test-host
for variant in fork stock; do
  flock -s /tmp/enth-quiet.lock cargo build --release --locked --manifest-path "$here/$variant/Cargo.toml"
  cp "$here/$variant/target/release/libspike_shell.so" "$out/$variant.clap"
done

for variant in fork stock; do
  echo "== editor-cycles $variant ($cycles cycles)"
  repeat=1
  [ "$variant" = fork ] && repeat=2
  flock -s /tmp/enth-quiet.lock xvfb-run -a -s "-screen 0 1920x1080x24" \
    "$host" editor-cycles "$out/$variant.clap" -n "$cycles" --mode both --repeat "$repeat" --json-out "$out/$variant.json" \
    > "$out/$variant.log" 2>&1
done

python3 -I - "$out" "$cycles" "$max_kb" <<'PY'
import json, re, sys
out, cycles, max_kb = sys.argv[1], int(sys.argv[2]), float(sys.argv[3])
ok = True

def fit(series, warmup=10):
    ys = series[warmup:]
    n = len(ys)
    mx, my = (n - 1) / 2, sum(ys) / n
    return sum((x - mx) * (y - my) for x, y in enumerate(ys)) / sum((x - mx) ** 2 for x in range(n))

def check(label, good, detail):
    global ok
    print(("PASS " if good else "FAIL ") + label + ": " + detail)
    ok = ok and good

for variant in ("fork", "stock"):
    d = json.load(open(f"{out}/{variant}.json"))
    drops = [m for s in d["stderr_samples"] for m in re.findall(r"drop builds=(\d+) editor_closed_calls=(\d+)", s)]
    edit = [(int(b), int(c)) for b, c in drops if int(b) > 0]
    editor_runs = [r for r in d["runs"] if r["mode"] == "editor"]
    base_runs = [r for r in d["runs"] if r["mode"] == "baseline"]
    anon = [fit(r["series"]["pss_anon_kb"]) for r in editor_runs]
    base_anon = sum(fit(r["series"]["pss_anon_kb"]) for r in base_runs) / len(base_runs)
    slope = max(anon) - base_anon
    threads_closed = sorted({t for r in editor_runs for t in r["series"]["threads_closed"]})
    drawn = [r["summary"]["opens_drawn_in_time"] for r in editor_runs]
    print(f"{variant}: pss_kb slopes (test host) editor {[round(x, 2) for x in d['comparison']['editor_pss_growth_kb_per_cycle']]} "
          f"baseline {[round(x, 2) for x in d['comparison']['baseline_pss_growth_kb_per_cycle']]} KB/cycle; "
          f"anonymous slopes editor {[round(x, 2) for x in anon]} baseline {base_anon:.2f}, worst editor minus baseline {slope:.2f}, "
          f"threads after close {threads_closed}, drawn in time {drawn}, builds/closed {edit}")
    if variant == "fork":
        check("fork: editor_closed() calls equal the builds", bool(edit) and all(b == c == cycles for b, c in edit), f"{edit}")
        check("fork: anonymous PSS growth", slope <= max_kb, f"{slope:.2f} KB per cycle against the {max_kb:.0f} KB limit")
        check("fork: threads constant after close", len(threads_closed) == 1 and all(r["summary"]["threads_constant_after_warmup"] for r in editor_runs), f"{threads_closed}")
        check("fork: every first frame drawn", all(x == cycles for x in drawn), f"{drawn}")
    else:
        check("control: stock never calls editor_closed()", bool(edit) and all(c == 0 for _, c in edit), f"{edit}")
        check("control: stock leaks above the limit", slope > max_kb, f"{slope:.2f} KB per cycle")
sys.exit(0 if ok else 1)
PY
