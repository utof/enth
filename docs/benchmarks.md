# Benchmarks and measurement protocol

**Measure what users feel, not just averages.** A plugin that averages 1% CPU but occasionally takes 3 ms on the audio thread causes dropouts; an editor that idles at 0% but leaks 2 MB per open/close bloats a long session. This doc defines what we measure, how, and the pass/fail thresholds.

Three areas: the **UI stack** (decided in Phase 0), the **engine** (tracked forever), and the **agent loop** (from Phase 1).

## Two tiers of GUI measurement

2026-10-19: the NVIDIA 595-series X driver crashed Xorg (and the DAW with it) after short-lived X clients ran on the real display. That matches a known driver bug that plugin editors, which open and close windows constantly, would trigger. So GUI measurements come in two tiers, and agents run only the first.

| Tier | Where | Who | What it measures |
| --- | --- | --- | --- |
| Automated | Xvfb + Mesa software rendering (`xvfb-run`), headless REAPER, the test host | Agents, CI | CPU, wake-ups, PSS, leaks, threads, open time, keyboard, in-host HiDPI, screenshots |
| Owner-attended | The real display and the real GPU, work saved, ≥ 10 s between launches | The owner | Frame pacing, GPU %, VRAM, first-open behaviour, crash exposure, behaviour under the real window manager |

Never automate rapid editor open/close on the real NVIDIA display.

## What to watch beyond CPU, GPU, RAM and startup

| Dimension | Why it matters | Metric |
| --- | --- | --- |
| Worst-case audio block time | Dropouts come from spikes, not averages | p99 and p99.9 `process()` time vs the block deadline, under real-time priority (max is information; [Real-time measurement](#real-time-measurement-p1b)) |
| Dropouts (xruns) while the GUI is busy | A UI that steals CPU or locks causes clicks | xrun count over 10 min with editors animating, attributed by an editor-open vs editor-closed A/B (never read as an absolute zero) |
| Cost with the editor **closed** | Most instances in a project are closed | GUI-thread CPU ≈ 0, no timers |
| Cost when **occluded or minimised** | Hidden editors shouldn't keep drawing | CPU ≤ idle value, ideally 0 |
| Memory per instance (PSS, not RSS) | RSS double-counts shared libraries | Marginal PSS per instance, closed and open |
| Scaling | Fixed per-instance costs multiply | 1 / 10 / 50 instances |
| Leaks | Long sessions open editors hundreds of times | PSS growth and thread count after 200 open/close cycles, against a host-only baseline (REAPER's own ReaEQ is flat) |
| Frame time and jank | Dragging a knob must feel instant | CPU time per frame p50/p99; achieved fps vs requested (baseview caps near 30 fps); frames > 50 ms |
| Wake-ups per second | An idle editor should sleep | `pidstat -w` on the GUI thread, open and idle (egui-baseview polls at 64/s today) |
| First-open reliability | A blank first open ruins trust (nice-plug #95) | Editor draws within 1 s on 20 of 20 opens, on the real display |
| Knob → sound latency | Feel | Time from param event to the first audio block reflecting it |
| Automation → UI latency | Knobs must follow host automation | UI lag behind a host ramp |
| Scope/meter throughput | Visual feedback for learning | Updates/s delivered without drops |
| Power on battery | Laptops | Watts in each state |
| Editor open time | Feel | `gui.create` → first presented frame |
| Plugin scan / load time | Hosts scan many plugins | Load without GUI |
| Install footprint and runtime dependencies | e.g. a WebView depends on the system's WebKitGTK | Bundle size; system libraries required |
| Host integration | Many bugs live here | Checklist below |
| Accessibility | Inclusion | Screen reader (Orca) and keyboard-only use |

## The reference panel (identical in every UI arm)

- 64 knobs bound to real host parameters, with value labels.
- A modulation-matrix list of 16 rows.
- 3 draggable module cards in a lane (drag to reorder).
- A modulation ring animating on 8 knobs (driven by an LFO's value from the audio thread).
- 2 scopes: an oscilloscope (2048 samples) and a 512-bin spectrum, fed from the audio thread through a lock-free ring at 60 Hz, drawn at ≤ 30 fps while baseview 0.4.1 caps near 30 fps.
- A lesson pane: ~1,500 words with headings, bold text, one image and a scrolling list.
- Window 1100×768 logical pixels; test at scale 1.0 and 2.0.

**Repaint gating (egui):** repaint only on input, on a parameter change, or when a *visible* scope has new data; stop entirely when the editor is closed. Scopes ask for the next frame after `1 / motion.scope_hz` = 1/60 s, only while visible and fed: under baseview 0.4.1 a 60 Hz request gives about 30 fps (28.6–29.6 measured), and a 1/30 s request lands on about 20 fps (19.5), so the token stays at 60 until baseview's pacing changes ([spikes/ui.md](spikes/ui.md)). **2026-10-20 (P1b lane INT):** the product editor (lane ED2) asks for every live frame at `motion.animated_hz`, 1/30 s, the scope included; `motion.scope_hz` is not used by it. Its exact-delay tests check 0 frames idle and 33.33 ms while data changes; what baseview makes of that on the real display (about 20 fps by the Phase 0 measurement) is an open check for owner build 1 ([phase-1b.md](phase-1b.md#open-checks)). Measured 2026-10-19 under Xvfb: 0 frames while idle, about 26 frames/s while host automation plays (research 02) and 28.6–29.6 fps with a chord playing in the reference panel. Name the GUI thread (`prctl(PR_SET_NAME)` inside `build()`) and log `CLOCK_THREAD_CPUTIME_ID` per window of frames, to attribute CPU per editor.

## Procedure

1. **Automated tier.** Headless REAPER under Xvfb (Dummy Audio, driven by `cargo xtask reaper-test`-style Lua scripts), 48 kHz, 64-sample buffer, 8 instances each playing a held chord. Editor states: closed → open idle → open animating (→ covered → minimised, owner tier only: under Xvfb a covering window does not stop drawing, because there is no compositor). 120 s per state. The REAPER copy used for automation is a throwaway, never your own config.
2. Repeat with 1, 10 and 50 instances (editors closed), then 10 editors open. Scaling to 50 instances and watts move to S1.
3. In `tools/test-host` (clack-host): 200 scripted open/close cycles (`enth-test-host editor-cycles`, Xvfb only; it refuses a real display), with threads and PSS recorded per cycle against a no-editor baseline; editor open time; knob → sound latency; process-time distribution. For leak attribution use `valgrind --leak-check=full` (about 100 s for 6 cycles): heaptrack segfaults as soon as the plugin opens a window, and REAPER crashes under it ([spikes/test-host.md](spikes/test-host.md)).
4. Repeat a subset in a second host: Carla (VST3 only; the plugin's VST3 build). CLAP poly-mod needs Bitwig (trial). Ardour hosts VST3 only.
5. Run the host-integration checklist.
6. **Owner-attended tier.** On the real display, with work saved: first-open reliability (20 opens in REAPER), idle and animating cost on the real GPU (`nvidia-smi pmon -s um -d 1`, accounting for the compositor's own GPU and CPU use), covered and minimised, glow against wgpu, and the same again with `prime-select on-demand` if the X driver crashes.

### Tools (Linux)

```bash
xvfb-run -a -s "-screen 0 1920x1080x24" <command that opens a window>   # all automated GUI work
pidstat -t -p "$(pidof reaper)" 1 120 > cpu.txt                 # per-thread CPU (add -w for wake-ups)
smem -P reaper -c "pid pss uss" ; cat /proc/<pid>/smaps_rollup   # PSS / USS
nvidia-smi dmon -s um -d 1                                       # GPU SM % and memory
perf stat -e task-clock,context-switches -p <pid> -- sleep 60    # overall cost; blocked while kernel.perf_event_paranoid is 4
perf record -g -p <pid> -- sleep 30                              # where the GUI spends time; same limit
valgrind --tool=callgrind <engine bench>                         # instruction counts: load-independent
heaptrack <test-host> …                                          # allocations / leaks
sudo powertop                                                    # watts on battery
clap-validator validate ~/.clap/enth.clap                        # correctness
```

REAPER's *Performance Meter* (View menu) shows real-time CPU and xruns per track.

## UI thresholds

The thresholds are the Phase 0 defaults; lane Z filled in the measured values and the G0 status on 2026-10-19 (lane U2's measurements were made by lanes U1 and H: [spikes/ui.md](spikes/ui.md), [spikes/test-host.md](spikes/test-host.md)). The tier column says who measures: **A** automated (Xvfb), **O** owner-attended (real GPU). Automated numbers ran under Xvfb with Mesa software rendering on a shared, loaded laptop: frame counts, idle behaviour, CPU before the GPU, threads and leaks transfer to a real GPU; drawing cost and memory under llvmpipe don't. Status is *passed*, *pending owner* or *failed*.

| Metric | Pass | Tier | Measured, Phase 0 | G0 status |
| --- | --- | --- | --- | --- |
| Extra xruns attributable to the GUI (10 min, 8 instances, 64-sample buffer), by editor-open vs editor-closed A/B | 0 | A, O | Not measured: Dummy Audio only, no 8-instance run | Pending owner (real audio device) |
| Real-time CPU increase from opening an editor | < 1% | A | Idle: none measurable (REAPER 7.7–9.9 % with the editor open, 7.9–9.9 % never opened). Animating: dominated by llvmpipe under Xvfb | Passed when idle; animating pending owner |
| GUI CPU, editor closed | ≈ 0% (no timers) | A | 0: no GUI thread exists while closed | Passed |
| GUI CPU, open and idle | < 1% of one core per editor; report wake-ups/s | A | 0.05–0.19 %, 0 frames in 22 s (6 runs); 64 wake-ups/s (egui-baseview's notifier poll) | Passed |
| GUI CPU, open and animating (scopes ≤ 30 fps) | < 5% of one core per editor (borderline at 20–30 fps: about 3.5–6%) | A, O | CPU before the GPU: 1.16–1.41 ms per frame, 3.5–4.2 % at 30 fps (headless). Under Xvfb 24–31 %, which is software GL | Pending owner (real GPU); the CPU side passes |
| GUI CPU, occluded or minimised | ≤ idle (gate only on "closed" until the owner has checked a real window manager) | O | Not measured (no compositor under Xvfb); the panel schedules nothing when egui reports it minimised or occluded | Pending owner |
| CPU time per frame p99 | < 16.7 ms; no frame > 50 ms during a knob drag; achieved fps reported | A | Headless p99 1.12–2.30 ms, max 4.90 ms (scale 1 and 2); `ui()` p50 0.55–0.67 ms in REAPER; 6.69 M / 7.64 M instructions per idle / animating frame (callgrind); 28.6–29.6 fps achieved for 60 requested | Passed |
| Marginal PSS per open editor | < 30 MB | A | +71 MB under Xvfb, almost all Mesa's llvmpipe | Pending owner (real GPU); the Xvfb figure is not a GPU number |
| Marginal PSS per closed instance (above DSP state) | < 5 MB | A | ≤ 3.2 MB: the whole test-host process with the plugin loaded and playing, editor never opened | Passed |
| PSS growth after 200 open/close cycles | No growth beyond the host-only baseline; threads constant. The unfixed baseview leak is ≈70–160 KB per cycle, so the strict "< 2 MB" is a goal once it is fixed upstream | A | Editor minus baseline **76.9 KB per cycle** (≈15 MB per 200; baseline 0.2 KB). About 61 KB of it is one xkb keymap baseview 0.4.1 never frees; with that one line fixed, **16.1 KB per cycle** (≈3.2 MB per 200). Threads 3 closed and 30 open, constant over 200 cycles | **Failed** (baseview, fix drafted for upstream); threads passed |
| Editor open time | < 150 ms warm, < 400 ms cold | A, O | Test host, create → show: median 53.1 ms, p95 69 ms, max 87 ms (release; debug median 70.6 ms, max 131 ms). REAPER, `build()` → first frame: 44–67 ms | Passed (A); real display pending owner |
| First open | Draws within 1 s on 20 of 20 opens | O | Under Xvfb: 200 of 200 in the test host; 22 of 22 in REAPER (research 02) | Pending owner |
| GPU per open editor | < 5% SM, < 100 MB VRAM | O | Not measured | Pending owner |
| Knob → sound latency | ≤ 1 audio block + 1 frame | A | Not measured | Not measured; owned by lane TH in P1b (test host, [phase-1b.md](phase-1b.md)) |
| Host-integration checklist | All pass in REAPER and one other Linux host (Carla, VST3) | A, O | REAPER under Xvfb: open, close and reopen without a crash; typing into a field works; at scale 2 REAPER clips the editor (baseview #359); Space no longer reaches the transport after a click in the editor. Carla: editor not tried | **Failed** (HiDPI and Space, both baseview); the rest pending owner |

2026-10-19 (P1b research): a prototype of the generated editor (cards, knobs and segmented controls built from `ParamSpec`s, no macro bar, matrix or spectrum yet) costs 4.55 M instructions for an idle frame and 4.90 M for an animating one, against the spike panel's 6.69 M and 7.64 M; the real editor should land near 7–8 M. With the vendored baseview's xkb fix, the leak gate is at most 26 KB per open/close cycle (16.1 KB measured plus 10 of margin; stock baseview would need 87 KB). See the [ADR-0002 note](adr/0002-ui-toolkit.md#note--2026-10-19-p1b-research-layout-as-data-stable-ids-keyboard-focus).

### Host-integration checklist

- [ ] Opens, closes and reopens without crashing or leaking
- [ ] Resizes (host-driven and plugin-driven); correct at HiDPI scale 2.0, tested **in the host** with `Xft.dpi: 192` on the root window (REAPER currently sizes its FX window for scale 1 while baseview draws at scale 2, so the editor is clipped)
- [ ] Keyboard, three separate checks: typing into a value field works (passes today); spacebar still reaches the host for transport after clicking an empty editor area (fails today under Xvfb: clicking the editor moves keys to the plugin); focus returns to the host after clicking the host. Expect the focus items to fail until baseview's focus policy is fixed, and log them rather than treating them as surprises
- [ ] Copy/paste in text fields
- [ ] Drag-and-drop inside the editor
- [ ] Tooltips and context menus appear in the right place
- [ ] Mouse wheel and fine-drag (Shift) on knobs
- [ ] Host automation moves the knobs; knob moves record as automation
- [ ] Works when the host runs the plugin bridged/sandboxed (REAPER option)

## Decision rules for the UI toolkit (Gate G0)

2026-10-19: there is one arm, **egui rendered with glow**. vizia is stale and iced spins a core while idle, so neither is built; the WebView arm is cut ([ADR-0002](adr/0002-ui-toolkit.md)).

1. Reject egui (and open a new research session, rather than keep a second toolkit warm) if it causes GUI-attributable xruns, a crash or hang in REAPER, or fails the PSS or CPU thresholds after reasonable tuning.
2. Keep **glow** unless the owner-attended real-GPU run shows wgpu clearly better on open time, fps and VRAM. wgpu creates a device per editor and adds 16–21 MB of PSS under software rendering.
3. If the real-GPU first-open test fails, or the NVIDIA crash recurs with plugin editors, decide about a CPU-rendered fallback (softbuffer or tiny-skia) before accepting the toolkit.

Applied at G0 (2026-10-19): rule 1 did not trigger. There was no GUI-caused crash or hang in REAPER, and the CPU thresholds pass on the CPU side; the PSS growth fails only through baseview's keymap leak, which sits below egui and every other toolkit on baseview. Rules 2 and 3 wait for the owner's real-GPU run ([spikes/owner-checklist.md](spikes/owner-checklist.md), B2, C1 and D3). ADR-0002 is accepted on that basis, with the reopen conditions listed there.

## Engine benchmarks (tracked from Phase 1, in CI)

The measured column is the Phase 0 engine spike (`spikes/spike-engine`, [spikes/engine.md](spikes/engine.md)): 2 s renders at 48 kHz, block 64, one core, release, timings at load 6–10 (read wall-clock as ±20 %; instruction counts are exact).

| Metric | Tool | Target | Measured, Phase 0 |
| --- | --- | --- | --- |
| Offline render speed | `enth bench`, `hyperfine` | G0 floor ≥ 100× realtime per core (met). From P1: ≥ 1,000× for the 1-voice template and ≥ 100× for an 8-voice P1-like patch, on the scalar path; G1 adds the 16-voice budget ([roadmap](roadmap.md#p1--minimal-synth--cli-v0-46-weeks)). Report min and median with the load average | 1 voice: 1,993× best, 1,783× median (wasmtime 1,357×). 16 voices: 129× / 122× scalar, 245× / 213× f32x8. 8-voice P1-like with reverb: 152× / 149× scalar, 231× / 217× f32x8 |
| CPU per voice at 48 kHz | **Instruction counts** (callgrind, gungraun), per module | Regression gate: an absolute `hard_limits` budget per benchmark (measured + 10 %) that fails the run when exceeded; it needs no stored baseline, so it works in CI. Re-measure budgets with the golden hashes after a toolchain, `libm` or `wide` bump. Criterion and wall-clock time are information only: with no code change Criterion reported two false "improvements" (−13%, −36%) under load | Per 2 s render (budget in brackets): 1 voice 8.15 M (9.0 M); 16 voices scalar 122.4 M (135 M), f32x4 65.1 M (72 M), f32x8 62.9 M (69.5 M); P1-like 8 voices scalar 112.4 M (124 M), f32x8 62.3 M (69 M). That is 80–85 instructions per sample and voice for the simple voice, 146 for the P1-like one. Passes locally and on GitHub's ubuntu-24.04 runner |
| Subnormals | A deterministic test: render a 40 s release tail and count subnormal values | 0. Instruction counts under-report the denormal penalty (4.1× more instructions against 33.5× more wall time in the spike), so this test is half of the gate | 0 subnormal output samples and state values over 40 s tails (scalar and lanes, 12 random patches). Negative control without flushing: 203,962 subnormal samples, voices never end |
| p99 and p99.9 `process()` time at 64 samples | test host `process-stats`, under rtkit `SCHED_RR` | p99 and p99.9 < 25% of the block deadline (333 µs of 1.33 ms at 48 kHz); max is information | Engine only, 2026-10-19 ([details](#real-time-measurement-p1b)): p99 125–146 µs, p99.9 166–186 µs (12–14 %). Plugin level: not measured yet (P1b, lane RT) |
| Plan swap time | test host `process-stats` | Swap-block p99 < 25% of the deadline with a structural edit every 250 ms; compile off the audio thread | Engine only, 2026-10-19: with the state copy, instant-swap block p99 797 µs (max 997 µs, 75 %); with the move, p99 139 µs (max 149 µs). A crossfading swap still copies (p99 1,052 µs). Compiling the G1 patch takes about 0.9 ms on the main thread |
| Allocations on the audio thread | `assert_no_alloc` in engine tests (debug builds only); nice-plug's `assert_process_allocs` in plugin debug builds | 0 | 0: `assert_no_alloc` around every `process()` block (scalar, x4, x8; 16 random patches), negative control caught; clap-validator's process tests pass with `assert_process_allocs` on |
| Real-time safety | RTSan, one nightly run (nightly Rust; the runtime ships in rust-std) | No findings. It catches `malloc` but misses an uncontended `std::sync::Mutex`, so `enth-engine` also has no `Mutex` or `RwLock` | One manual run (rustc 1.101.0-nightly 2026-10-17): no findings; an injected allocation is reported, an uncontended `Mutex` is not |
| Determinism | golden tests in CI | Hashes identical across platforms, wasm engines and build flags (Linux, wasm32 under wasmtime and node, Windows; aarch64 if runners exist); a feature summary with tolerances is only the diagnostic when a hash differs | 1,010 hashes identical on Linux x86-64, Windows x86-64 (CI) and `wasm32-wasip1` under wasmtime (with and without simd128), across debug, release, `target-cpu=native`, FMA, LTO, lanes and FTZ/DAZ. aarch64: manual CI job only. node/V8 not re-run (research 03 ran it on an earlier engine) |

## Real-time measurement (P1b)

2026-10-19, from the P1b research (report 03). The numbers are engine-only (`enth-engine` standing in for the plugin; there is no `enth-plugin` yet): paced 64-frame blocks at 48 kHz, one G1 instance, on this laptop at a **load average of 7 to 26** because other builds were running, so wall-clock numbers are upper bounds. Re-run the G1 numbers on a quiet machine before the gate, with the environment snapshot recorded.

### Real-time priority first

A normal-priority thread measures the scheduler, not the plugin. One G1 instance, 64 frames, 60 s, paced:

| Thread | Deadline misses (of 45,000 blocks) | p99.9 `process()` | Wake-up lateness, max |
| --- | --- | --- | --- |
| `SCHED_OTHER` | 980–2,299 (2–5 %) | 179–663 µs | up to 95 ms |
| `SCHED_RR` 10, via rtkit | **3** | **166–185 µs** (12–14 % of the 1,333 µs deadline) | 1.5–1.9 ms |

The engine's own CPU time is the same in both, 152–187 µs at p99.9. The misses were scheduler noise, not plugin CPU.

- **No `ulimit -r` is needed.** rtkit already grants this user `SCHED_RR` up to priority 20. Lower `RLIMIT_RTTIME` to at most 200 ms, call `org.freedesktop.RealtimeKit1.MakeThreadRealtimeWithPID`, and the thread becomes `SCHED_RR|SCHED_RESET_ON_FORK`. A runaway RR thread is then killed by the limit instead of freezing the desktop (16 G1 instances on one thread were killed this way). rtkit allows 25 real-time threads per user and PipeWire already holds four, so test runs use few threads. `SCHED_FIFO` would need a system change and is not needed.
- **Pinning without isolation makes `SCHED_OTHER` worse** (2,299 misses). Use RR without pinning, or RR pinned for slightly tighter tails.
- **Max is not a gate.** Max under RR (1.1–1.4 ms) is a property of this laptop: interrupts, SMIs and frequency changes charged to the thread, because `IRQ_TIME_ACCOUNTING` is off. Report max, p99.99, lateness, misses and max(wall − cpu) as information.

### The G1 row

The `process()` row of Gate G1 ([roadmap.md](roadmap.md#p1--minimal-synth--cli-v0-46-weeks)) is measured like this:

- **Conditions:** the real `enth-plugin` release bundle; the G1 patch with a held 16-note chord; 64 frames at 48 kHz; `enth-test-host process-stats` with rtkit RR 10; one instance; 120 s after a 10 s warm-up; the environment snapshot recorded (load, governor, quantum, kernel, preempt mode).
- **Pass:** p99 under 25 % of the deadline (333 µs) **and** p99.9 under 25 %. With a structural edit every 250 ms, the p99 of instant-swap blocks and of the blocks during a crossfade is under 25 % too.
- **Today** (the engine standing in): steady p99 125–146 µs and p99.9 166–186 µs pass. Swap blocks on main fail (below).
- **Guards that need no wall clock**, because wall-clock time is information only: a test that an instant swap moves the state (the new plan's reverb buffers are the old plan's), and gungraun budgets for three spiky blocks (an instant swap, a crossfade, 16 note-ons). Instruction counts under-state a memory copy, so the move test, not the budgets, guards against a copy.

### The plan swap

The cause is memory bandwidth, not page faults (0 in the swap block): the reverb's six delay rings are 1.6 MB, and copying them takes 148–249 µs alone, 2–4× more when paced because caches are cold after a sleep and the cores clock down. A normal block costs 57–93 µs. Paced under RR with a plan sent every 250 ms (30 s each):

| Swap | Swap block, copy (main) | Swap block, with the move (prototype) |
| --- | --- | --- |
| Instant (nothing added or removed) | p50 328, p99 797, **max 997 µs** (75 % of the deadline) | p50 103, p99 139, **max 149 µs** |
| Crossfading (a module added or removed) | p50 354, p99 1,052, **max 1,057 µs** (79 %); crossfade blocks p50 174, max 294 µs | Unchanged: both plans run, so the copy stays |

P1b adopts the move ([design.md §3](design.md#3-engine-and-real-time-safety)). A prototype that blends the two voice sums before one effect chain would bring the crossfading swap to p99 189, max 195 µs, but it changes the crossfade definition; it is not adopted now ([phase-1b.md](phase-1b.md#open-checks)). Compiling the G1 patch on the main thread takes about 0.9 ms (p50; the reverb's preparation dominates), which is fine.

### Multiple instances

One G1 instance costs about 100 µs per 64-frame block when paced, about 7.5 % of a core. That is 1.5–1.8× the offline figure of 4.3 % (taken at load 7), because of cold caches and frequency scaling. Each thread below has its own independent timer, unlike REAPER's synchronised worker pool.

| Instances × threads (RR) | Callback p50 | Max | Misses |
| --- | --- | --- | --- |
| 8 × 1 | 788 µs (59 % of the deadline) | 1,214 µs (91 %) | 0 in 30 s |
| 16 × 1 | — | — | Cannot keep up: killed by the `RLIMIT_RTTIME` limit |
| 16 × 4 | 423–426 µs | 600–931 µs | 0–1 per thread |

Eight instances fit on one thread. Sixteen need several threads, which hosts provide. Information gate: 16 instances on 4 RR threads, each thread's p99 under 50 % (measured 472–477 µs, 35–36 %). With 8 RT threads, some saw wake-ups 10–35 ms late (cause not isolated).

### In a host, and xruns

- **REAPER's anticipative FX.** `process()` runs on anticipative-FX worker threads unless the track sets `I_PERFFLAGS` bit 2 (`SetMediaTrackInfo_Value(tr, "I_PERFFLAGS", 2)`), so it is not on the device deadline. Xrun checks set that flag. REAPER exposes no per-call plugin timing, so the plugin measures itself: an `rt-stats` cargo feature, off by default, with a fixed log-bucketed atomic histogram of `process()` wall time, dumped as JSON at `deactivate()` on the main thread. The same histogram works in REAPER, Carla, Bitwig and the test host.
- **A private PipeWire daemon for agents:** no hardware, quantum 64, its own socket, and `rtportal.enabled = false`; `pw-jack -r` then gives a silent 64-frame JACK graph without touching the owner's PipeWire. Without that setting, PipeWire 1.0.5's `module-rt` silently fails through xdg-desktop-portal on this machine and the thread stays `SCHED_OTHER`. Its Dummy-Driver overruns raise no JACK xrun callbacks: count them with `pw-top -b` (ERR) and cycles completed against expected.
- **Headless REAPER over JACK did not connect**, so a REAPER xrun test stays owner-attended. The owner's PipeWire config forces min-quantum 1024; `pw-metadata -n settings 0 clock.force-quantum 64` bypasses the config's limits (documented), and `clock.force-quantum 0` puts it back.
- **Editor against audio, under Xvfb** (llvmpipe is a worst-case CPU competitor). With an RR audio thread, opening an animating 1100×768 editor made no measurable difference: 0 and 2 misses closed, 4 and 3 open, per 45,000 blocks. With `SCHED_OTHER` the misses roughly doubled (1,024–1,146 closed, 1,947–2,114 open, per 22,500). So GUI-caused xruns come from the scheduling class, from locks shared with the audio thread and from GPU-driver latency, not from editor CPU as such. Only the last needs the real GPU, which is the owner tier. At 2–4 misses a minute a 60 s A/B cannot resolve a difference: use interleaved runs of 10 minutes.
- **Allocation checks.** `assert_process_allocs` wraps nice-plug's whole process wrapper in debug builds only, so keep it on for debug bundles under the test host and clap-validator. An RTSan-built test host (nightly, date pinned, linked with `--export-dynamic`) that wraps the plugin call in `__rtsan_realtime_enter` and `__rtsan_realtime_exit` catches `malloc` in a plain stable-built plugin. It misses an uncontended `std::sync::Mutex` and a `thread::sleep`, and the entry symbol is internal, not part of Clang's public API.

## Measurement hygiene

Every timing taken in Phase 0 ran at a load average of 2–30 on a shared laptop, so treat wall-clock numbers as upper bounds and instruction counts as the reference. The machine itself is described in [spikes/environment.md](spikes/environment.md).

- Measure on a rebooted machine with heavy applications closed, the repo and `target/` on ext4, and nothing else building.
2026-10-20 (P1b lane INT), on `enth-plugin` with the vendored fork: `cargo xtask leak-test` measures the editor at 12.31 and 12.45 KB per open/close cycle over 200 cycles (baseline 0.0; the gate is 26 KB), 3 threads after every close, 200 of 200 first frames drawn under Xvfb; instance churn 0.54 KB per cycle with no thread or descriptor left ([plugin.md](plugin.md#leak-gates)). The "PSS growth after 200 open/close cycles" row's failure was baseview's keymap; with the fix it passes the P1b gate. The real-display rows go to owner build 1 ([p1b-owner.md](p1b-owner.md)).

- Record an **environment snapshot with every result**: `uptime` load average, free RAM and swap, the PRIME mode, the preempt mode, the PipeWire quantum, and the kernel and NVIDIA driver versions.
- Report timings as min and median, never a single run. Compare configurations interleaved, not one after the other.
- Attribute xruns by an editor-open vs editor-closed A/B, not as an absolute zero.

## Agent-loop benchmarks (from Phase 1)

Defined in [agent-cli.md](agent-cli.md#evaluating-the-cli-like-a-product): solve rate, renders per solve, tokens per solve, invalid-command rate, on `content/agent-evals`.

## Results template

Copy into `docs/spikes/ui.md`. One toolkit, two tiers; every row carries the environment snapshot.

| Metric | egui/glow, Xvfb (A) | egui/glow, real GPU (O) | Threshold | Notes |
| --- | --- | --- | --- | --- |
| GUI CPU, closed | | | ≈ 0% | |
| GUI CPU, open idle | | | < 1% | |
| Wake-ups/s, open idle | | | report | |
| GUI CPU, open animating (≤ 30 fps) | | | < 5% | |
| Achieved fps vs requested | | | report | |
| GUI CPU, occluded | | | ≤ idle | |
| xruns (10 min), open vs closed A/B | | | 0 extra | |
| PSS per open editor | | | < 30 MB | |
| PSS growth per cycle / threads after N cycles, vs host-only baseline | | | no growth beyond baseline | |
| CPU time per frame p99 | | | < 16.7 ms | |
| Editor open (warm / cold) | | | 150 / 400 ms | |
| First open draws (of 20) | | | 20 / 20 | |
| GPU SM % / VRAM | | | 5% / 100 MB | |
| Keyboard: field typing / Space after click / focus back to host | | | all pass | |
| HiDPI in host (scale 1 and 2) | | | correct | |
| Checklist (REAPER / Carla) | | | all pass | |
