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

**Repaint gating (egui):** repaint only on input, on a parameter change, or when a *visible* scope has new data; stop entirely when the editor is closed. Scopes ask for the next frame after `1 / motion.scope_hz` = 1/60 s, only while visible and fed: under baseview 0.4.1 a 60 Hz request gives about 30 fps (28.6–29.6 measured), and a 1/30 s request lands on about 20 fps (19.5), so the token stays at 60 until baseview's pacing changes ([spikes/ui.md](spikes/ui.md)). **2026-10-20 (P1b lane INT):** the product editor (lane ED2) asks for every live frame at `motion.animated_hz`, 1/30 s, the scope included; `motion.scope_hz` is not used by it. Its exact-delay tests check 0 frames idle and 33.33 ms while data changes; what baseview makes of that on the real display (about 20 fps by the Phase 0 measurement) is an open check for owner build 1 ([phase-1b.md](phase-1b.md#open-checks)). **2026-10-20 (R1/S1 lane AM):** the editor now asks for live frames at `motion.scope_hz`, 1/60 s (the exact-delay tests expect 16.67 ms), which gave about 30 fps under baseview 0.4.1 in Phase 0; the plugin's `editor-counts` feature counts the frames really drawn and the live frame rate (`live_fps`) for owner check 1.3 ([plugin.md](plugin.md#leak-gates)). Its editor frame cost is in [editor.md](editor.md#frame-cost): the worst case is 9.58 M of the proposed 10 M. Measured 2026-10-19 under Xvfb: 0 frames while idle, about 26 frames/s while host automation plays (research 02) and 28.6–29.6 fps with a chord playing in the reference panel. Name the GUI thread (`prctl(PR_SET_NAME)` inside `build()`) and log `CLOCK_THREAD_CPUTIME_ID` per window of frames, to attribute CPU per editor.

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
| p99 and p99.9 `process()` time at 64 samples | test host `process-stats`, under rtkit `SCHED_RR` | p99 and p99.9 < 25% of the block deadline (333 µs of 1.33 ms at 48 kHz); max is information | Engine only, 2026-10-19 ([details](#real-time-measurement-p1b)): p99 125–146 µs, p99.9 166–186 µs (12–14 %). Plugin, release bundle, 2026-10-20 ([lane RT](#lane-rt-the-plugin-measured-2026-10-20)): p99 112–123 µs, p99.9 125–134 µs (8–10 %), at load 2–6. G2, S2a's reference patch, 2026-10-27 ([the G2 row](#the-g2-row-s2a-lane-g2b-2026-10-27)): p99 628–642 µs (47–48 %) at 8 voices and 356 µs (26.7 %) at 4, at load 20–33 (G1 162–171 µs in the same session) |
| Plan swap time | test host `process-stats` | Swap-block p99 < 25% of the deadline with a structural edit every 250 ms; compile off the audio thread | Engine only, 2026-10-19: with the state copy, instant-swap block p99 797 µs (max 997 µs, 75 %); with the move, p99 139 µs (max 149 µs). A crossfading swap still copies (p99 1,052 µs). Compiling the G1 patch takes about 0.9 ms on the main thread. Plugin, 2026-10-20 ([lane RT](#lane-rt-the-plugin-measured-2026-10-20)): instant-swap block p99 107–148 µs, crossfade blocks p99 180 µs, the crossfading swap block p99 337–339 µs (max 521 µs, reported); gungraun budgets in `benches/swap.rs`. G2 ([the G2 row](#the-g2-row-s2a-lane-g2b-2026-10-27)), at load 20–33: crossfading swap block p99 1,420–1,423 µs at 8 voices (G1 656–691 µs in the same session), 851 µs at 4 voices; gungraun budgets in `benches/g2.rs` |
| Allocations on the audio thread | `assert_no_alloc` in engine tests (debug builds only); nice-plug's `assert_process_allocs` in plugin debug builds | 0 | 0: `assert_no_alloc` around every `process()` block (scalar, x4, x8; 16 random patches), negative control caught; clap-validator's process tests pass with `assert_process_allocs` on |
| Real-time safety | RTSan, one nightly run (nightly Rust; the runtime ships in rust-std) | No findings. It catches `malloc` but misses an uncontended `std::sync::Mutex`, so `enth-engine` also has no `Mutex` or `RwLock` | One manual run (rustc 1.101.0-nightly 2026-10-17): no findings; an injected allocation is reported, an uncontended `Mutex` is not |
| Determinism | golden tests in CI | Hashes identical across platforms, wasm engines and build flags (Linux, wasm32 under wasmtime and node, Windows; aarch64 if runners exist); a feature summary with tolerances is only the diagnostic when a hash differs | 1,010 hashes identical on Linux x86-64, Windows x86-64 (CI) and `wasm32-wasip1` under wasmtime (with and without simd128), across debug, release, `target-cpu=native`, FMA, LTO, lanes and FTZ/DAZ. aarch64: manual CI job only. node/V8 not re-run (research 03 ran it on an earlier engine) |

## Real-time measurement (P1b)

2026-10-19, from the P1b research (report 03). The numbers are engine-only (`enth-engine` standing in for the plugin; there is no `enth-plugin` yet): paced 64-frame blocks at 48 kHz, one G1 instance, on this laptop at a **load average of 7 to 26** because other builds were running, so wall-clock numbers are upper bounds. Re-run the G1 numbers on a quiet machine before the gate, with the environment snapshot recorded. Lane RT did that with the plugin on 2026-10-20, at load 1.5–7: [the results](#lane-rt-the-plugin-measured-2026-10-20).

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
- **The plugin, 2026-10-20 (lane RT):** every gated condition passes ([below](#lane-rt-the-plugin-measured-2026-10-20)); the guards are `benches/swap.rs` and the move test.
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
- **Allocation checks.** `assert_process_allocs` wraps nice-plug's whole process wrapper in debug builds only, so keep it on for debug bundles under the test host and clap-validator. An RTSan-built test host (nightly, date pinned, linked with `--export-dynamic`) that wraps the plugin call in `__rtsan_realtime_enter` and `__rtsan_realtime_exit` catches `malloc` in a plain stable-built plugin. It misses an uncontended `std::sync::Mutex` and a `thread::sleep`, and the entry symbol is internal, not part of Clang's public API. R1/S1 lane INT built it: `cargo xtask rtsan` ([plugin.md](plugin.md#realtimesanitizer-lane-int-r1s1)), with the plugin built under RTSan too (`--cfg enth_rtsan` marks its `process()`), ten scenarios and a positive control.

### Lane RT: the plugin, measured (2026-10-20)

**Conditions.** The coordinator's release bundle of main `0f4f29d`, and the same build with `--features rt-stats` for the plugin's own share. The G1 patch (`tools/test-host/golden/enth-plugin.g1.patch.json`, which is `Patch::to_json()` of `crates/enth-engine/benches/common/g1.rs`) holds the 16-note chord. 64 frames at 48 kHz. `enth-test-host process-stats --require-rt`: rtkit RR 10, `RLIMIT_RTTIME` 200 ms, not pinned. Steady runs last 120 s after a 10 s warm-up; swap runs 120 s; flood and multi-instance runs 60 s. Every run held `flock -x /tmp/enth-quiet.lock`, so no lane built meanwhile.

**Environment.** i7-9750H, kernel 7.0.0-38 `PREEMPT_DYNAMIC`, governor `powersave` (EPP `balance_performance`), 10–11 GB of swap in use. PipeWire's quantum is 2048, which does not matter here: the test host drives its own timer. The load average was 1.5–7, because the owner's other work ran at times (a pytest at 240–500 % CPU early on). The raw JSON reports, with the environment snapshot and the load at the start and end of each run, are in `~/code/enth-scratch/p1b/rt/runs/`.

| Condition | Measured | Limit | Result | Load |
| --- | --- | --- | --- | --- |
| `process()` p99, steady | 112.5–123.3 µs, median 116.4 (3 runs; 8.4–9.2 %) | < 333 µs | **Pass** | 2.1–5.6 |
| `process()` p99.9, steady | 125.1–133.6 µs, median 128.7 (9.4–10.0 %) | < 333 µs | **Pass** | 2.1–5.6 |
| `process()` max, steady | 179–1,336 µs | information | — | 2.1–5.6 |
| Instant-swap block p99, an edit every 250 ms (480 swaps a run) | 106.7 and 147.9 µs (max 133 and 166) | < 333 µs | **Pass** | 1.5–4.9 |
| Crossfade blocks p99 | 180.5 and 179.7 µs (max 216 and 197) | < 333 µs | **Pass** | 1.7–2.1 |
| Crossfading swap block p99 | 336.6 and 339.2 µs (25.2–25.4 %); p50 220–225, max 484–521 | reported, not gated | At the line | 1.7–2.1 |
| 8 instances on 1 thread: callback p50 and max | p50 388–454 µs (29–34 %); max 1,243–2,778 µs (93–208 %) | information (research: 59 % and 91 %) | — | 2.1–4.1 |
| 16 instances on 4 threads: each thread's callback p99 | 353–397 µs (26.5–29.8 %) | < 50 % (information) | **Pass** | 2.0–4.1 |

**The plugin's own share.** Three rt-stats runs were interleaved with the three plain ones.

- The plugin's histogram (`process_block`, nice-plug's wrapper excluded) read p50 49.2–55.3, p99 81.9–118.8 and p99.9 114.7–131.1 µs.
- The host's view of the same calls read p50 48.1–56.6, p99 79.5–119.6 and p99.9 109.5–133.2 µs. The histogram's percentiles are bucket upper edges, up to 6.25 % high.
- The two agree within the histogram's resolution (about 3 µs at 50 µs), so the whole call is the plugin's work; the plugin's histogram also counts the 10 s warm-up. The spread between runs (p99 79.5–123.3 µs over six steady runs, either bundle) comes from the laptop (frequency, caches, other work), not from the feature.
- Callgrind agrees. For a 64-frame G1 block, nice-plug's `Wrapper::process` runs 405,798 instructions, 401,599 of them in `Engine::process`. The shell's own 4,199 (1.0 %) are the ledger's sweep (2,655), the host-value pass (572) and nice-plug (about 600).
- A silent G1 instance with the editor closed costs 7,576 instructions per block in all. The telemetry tap is off while the editor is closed, so ED2's quiet rule is not in these numbers.

**Max and misses.** Every steady run had a max of 0.18–1.7 ms and 2–14 deadline misses per 90,000 blocks, with wake-up lateness up to 2.5 ms. That is this laptop's noise, as research 03 found: max(wall − cpu) reached 200 µs, and the CPU time charged to the thread includes interrupts.

**Swaps** (`--swap-state`). The host's main thread loads the G1 state and a variant alternately every 250 ms: `g1-value` (osc1's level changed) gives an instant swap, `g1-lfo3` (an unrouted voice LFO added) a crossfading one.

- `state.load` takes p50 380–470 µs and p99 0.7–1.0 ms on the main thread (parse, compile, send).
- The host cannot see the swap, so the swap block is taken as the largest block of each load's window: an upper bound for the instant swap.
- The per-offset profile shows the crossfade's shape plainly. p50 at offsets −1 to +4 is 55, 225, 105, 102, 105 and 53 µs: one tall block and three raised ones.
- Against research 03's engine-only paced numbers at load 13–17: the instant-swap block p99 went from 797 to 107–148 µs (the move, E+); the crossfading swap block from 1,052 to 337–339 µs (it still copies; the load is lower); crossfade blocks from p50 174 to 100–104 µs.

**The pre-effects crossfade: do not reopen it for P1b.**

- The crossfading swap block is the one block that reaches 25 %: p99 337–339 µs (25.2–25.4 %) and max 484–521 µs (36–39 %), on a quiet machine.
- It happens once per edit that adds or removes a module while a reverb survives.
- It costs a single instance no deadline: the two crossfade runs had 2 and 5 misses, as many as steady runs.
- It is memory bandwidth, so it grows with memory contention: research 03 measured up to 1,057 µs at load 13–17.
- Keep it reported, and reopen the pre-effects crossfade (or effect state held outside plans) when any of these happens:
  - a second large-state effect lands (S2: a delay or a chorus adds its own copy to the same block);
  - the host-level xrun check attributes an xrun to an edit;
  - `benches/swap.rs`'s `crossfade_swap` budget has to grow.
- **S2a lane MOD, 2026-10-27: the chorus is not that second effect.** It copies 32 KiB at a crossfading swap (64 KiB at 96 kHz, 128 KiB at 192 kHz; the phaser 128 bytes), and the copy measured alone is 1 µs warm and at most 15 µs at p99 cold at 48 kHz, against the reverb's 221–452 µs. With both effects in the G1 chain the plugin's crossfading swap block moved by no more than the run-to-run spread (median p99 1,086 µs against 1,049 µs without, on a machine at load 3–14 where the unchanged G1 patch itself read 919–1,150 µs). The `delay` lane's rings are the next candidate; the figures and the method are in [dsp/chorus.md](dsp/chorus.md#memory-and-the-plan-swap).

**Event floods** (`--flood N`). One flood every 250 ms for 60 s, at load 4–7. Each flood block also presses the chord again (16 note-ons).

| Note events in one block | Flood block p50 / p99 / max | Next block p50 / p99 | Misses (of 45,000 blocks) |
| --- | --- | --- | --- |
| 1,000 | 330 / 626 / 640 µs | 248 / 483 µs | 1 |
| 2,000 | 554 / 1,047 / 1,069 µs | 372 / 679 µs | 2 |
| 4,000 | 1,042 / 1,913 / 1,946 µs | 580 / 1,088 µs | 80 |

- The cost is linear, about 0.25 µs per event in the flood block. The next block pays for the notes in the flood block's second half, which apply at the next segment boundary.
- Callgrind, over 12 floods of 4,016 events, gives 11.9 M instructions per flood (2,960 per event):
  - the engine's note handling is most of it: `NoteCx::apply`, 5.7 M a flood. Every note-on steals a voice, and `Plan::start_voice` prepares it, though only the last 16 survive to render;
  - the plugin's `push_note` takes 1.4 M: its same-frame scan for a zero-length note is quadratic in the events per frame;
  - then come `render_chunk` and the `NOTE_END`s (0.8 M), the ledger's sweep (0.7 M) and nice-plug's input handling (0.3 M).
- Not a G1 condition: no host sends thousands of note events in 1.3 ms (that would be more than 700,000 events a second). If it ever matters (S1), the engine could skip preparing voices that are stolen before they render, and `push_note` could index its scan. Neither is needed now.

**Instruction counts.** `crates/enth-engine/benches/swap.rs` holds research 03's three spiky blocks plus a reference block, on G1 with the held chord, 64-frame blocks, after 0.5 s:

| Case | Instructions | Against a normal block | Budget (+10 %) |
| --- | --- | --- | --- |
| `normal` | 407,869 | 1 | 449,000 |
| `instant` (the block an instant swap lands in) | 444,921 | 1.09× | 490,000 |
| `crossfade_swap` (the block a crossfading swap lands in) | 2,489,265 | 6.1× (the copy: 6.0 M L1 hits against 0.55 M) | 2,739,000 |
| `crossfade` (the 3 blocks after it) | 2,407,222 | 2.0× per block | 2,648,000 |
| `note_ons` (16 new notes, every voice stolen) | 647,848 | 1.6× | 713,000 |

The other engine budgets were re-measured on `0f4f29d` with `cargo xtask bench`, and every one holds. None of these benches attaches a tap or runs the plugin, so ED2's quiet rule and PL's ledger are not in them (the ledger is costed above).

| Bench | Measured | Budget | Headroom | Against the measurement in the bench file |
| --- | --- | --- | --- | --- |
| `engine` saw_1v / saw_16v | 3,542,811 / 36,884,506 | 3,617,000 / 38,412,000 | **2.1 % / 4.0 %** | +7.7 % / +5.6 % |
| `g1` g1_16v | 602,344,687 (6,274 per frame) | 650,000,000 | 7.3 % | +2.0 % |
| `svf` lp_1v / lp_16v / drive_16v / moving_16v | 4,995,647 / 56,836,437 / 63,077,925 / 73,189,536 | 6,070,000 / 71,722,000 / 78,588,000 / 78,520,000 | 17.7 / 20.8 / 19.7 / 6.8 % | −9.5 / −12.8 / −11.7 / +2.5 % |
| `reverb` 1v / 16v | 116,957,149 / 150,298,844 | 129,200,000 / 174,200,000 | 9.5 / 13.7 % | −0.4 / −5.1 % |
| `comp` 1v / 16v | 12,819,819 / 46,166,700 | 14,550,000 / 59,600,000 | 11.9 / 22.5 % | −2.8 / −14.7 % |
| `adsr` 1v / 16v | 3,782,354 / 38,709,519 | 4,732,000 / 51,779,000 | 20.1 / 25.2 % | −12.1 / −17.8 % |
| `lfo` voice_sine 1v / 16v, voice_random_16v, global_sine_16v | 4,009,093 / 39,817,448 / 38,663,288 / 37,255,081 | 5,064,000 / 53,822,000 / 52,553,000 / 48,175,000 | 20.8 / 26.0 / 26.4 / 22.7 % | −12.9 / −18.6 / −19.1 / −14.9 % |
| `noise` white_1v / white_16v / pink_16v | 4,822,796 / 53,714,046 / 81,665,046 | 5,710,000 / 65,660,000 / 96,400,000 | 15.5 / 18.2 / 15.3 % | (in docs/dsp/noise.md) |
| `osc` saw_1v / saw_16v / triangle / pulse / sine / table / table_on_frame / synced_saw (16v) | 5,500,975 / 64,012,267 / 76,596,219 / 71,927,439 / 77,056,725 / 117,189,897 / 102,562,855 / 66,549,593 | 6,830,000 / 80,640,000 / 94,490,000 / 89,370,000 / 94,990,000 / 139,170,000 / 123,080,000 / 83,430,000 | 15.8–20.6 % | (in docs/dsp/osc.md) |
| `wt` plain_1v / plain_16v / pm_16v / full_16v (S2a lane WT's budgets, recorded by lane SKS; measured on the SKS branch) | 8,534,956 / 111,925,466 / 121,159,455 / 171,778,450 | 9,409,000 / 123,490,000 / 133,776,000 / 187,692,000 | 9.3 / 9.4 / 9.4 / 8.5 % | (in docs/dsp/wt.md) |

Proposed for the coordinator; lane RT changed none of these budgets.

`svf` after the drive's make-up gain (FU1 lane DRV, engine 0.3.0; the same "before" counts as the row above): 5,001,648 / 56,932,453 / 63,894,229 / 73,309,536, so headroom 17.6 / 20.6 / 18.7 / 6.6 % (`drive_16v` +1.06 instructions per sample per voice, [dsp/svf.md](dsp/svf.md#cost)).

- **Re-base `engine` to measured + 10 %: 3,898,000 and 40,573,000.** It has 2.1 % and 4.0 % left after growing 7.7 % and 5.6 % since its recorded measurement (taken when D1, D2 and S2b merged, before P1b's engine work). The next small engine change in any lane would fail it for growth that is already merged.
- **Tighten the module budgets with 15–26 % of headroom to measured + 10 %.** The engine skeleton got cheaper under them, so each now lets a 15–25 % regression of its module through.

**Reproduce:**

```bash
B=~/code/enth-scratch/bundles/<sha>/enth-plugin.clap; G=tools/test-host/golden
flock -x /tmp/enth-quiet.lock enth-test-host process-stats $B --state $G/enth-plugin.g1.patch.json --gate g1
flock -x /tmp/enth-quiet.lock enth-test-host process-stats $B --state $G/enth-plugin.g1.patch.json --require-rt \
  --swap-state $G/enth-plugin.g1-lfo3.patch.json --swap-kind crossfade   # or g1-value.patch.json --swap-kind instant
flock -x /tmp/enth-quiet.lock enth-test-host process-stats $B --state $G/enth-plugin.g1.patch.json --require-rt --secs 60 --flood 2000
flock -x /tmp/enth-quiet.lock enth-test-host process-stats $B --state $G/enth-plugin.g1.patch.json --gate g1-multi --secs 60
ENTH_RT_STATS=rt.jsonl enth-test-host process-stats <rt-stats bundle> …           # the plugin's own histogram, at deactivate
GUNGRAUN_RUNNER=… flock -x /tmp/enth-quiet.lock cargo xtask bench --bench swap
```

**Not done: the private PipeWire xrun harness.** The agent tier has no JACK host for the plugin without a new dependency. The candidate is `carla-single` (VST3) over `pw-jack -r` on the private daemon (research 03's `pw/`). It needs three things: the G1 state injected through a Carla project, a MIDI source on the private graph, and a check that the client sits on the private daemon before any note plays, so nothing can reach the owner's speakers.

### The G2 row (S2a lane G2B, 2026-10-27)

G2 is the reference patch for S2a, the way G1 is for P1a: a dubstep growl voice that uses what S2a added, with its own instruction budget and its own real-time row. It lives in `crates/enth-engine/benches/common/g2.rs` (the builder), `benches/g2.rs` (the gungraun benches and their budgets), `crates/enth-engine/tests/g2.rs` (the patch plays, is block-independent, swaps without allocating), `tests/golden/cases/g2-growl.toml` (the golden case) and `tools/test-host/golden/enth-plugin.g2*.patch.json` (the test host's states; `tests/golden/tests/g2.rs` checks that all of them are this patch).

**The patch** is report 04's S4 "FM growl", built from the modules that exist:

- **Source:** a `wt` on the `basic` table at 60 % with 7-copy unison (22 cents, 80 % blend, 60 % spread), phase-modulated by a sine `osc` (level 0, heard only through the audio route), with an MSEG (one pass per eighth note, five bent points) moving `pm_index` by 4 radians and the table position by 25 %.
- **Voice FX:** `dist` (asymmetric, 18 dB, 2× oversampling, velocity on its drive) into a `ladder` band-pass (the MSEG on its cutoff) into a `formant` (a dotted-eighth synced LFO on F2).
- **FX:** `ott` (60 %), `chorus` (30 % mix) and a synced `delay` (dotted eighth, ping-pong, 30 % feedback, 25 % mix); master −9 dB.
- **Voices:** `polyphony` 8, an E-minor chord across the bass register (E1 B1 E2 G2 B2 E3 G3 B3) held for the whole run, so no voice is stolen.

#### Instruction budget

One 2 s render at 48 kHz (96,000 frames, 512-frame host blocks, the held chord), callgrind through gungraun, the pinned toolchain, main `f59fb60` plus this lane. A hard limit is the measured count plus 10 %, rounded up to 1,000, and needs no stored baseline. Counts are exact: the second run of every bench matched the first to the instruction.

| | Instructions | Per frame | Against G1's 12,000 | Hard limit |
| --- | --- | --- | --- | --- |
| G1, 16 voices (`benches/g1.rs`, unchanged) | 603,331,584 | 6,285 | 0.52× | 650,000,000 |
| **G2, 8 voices (the reference)** | **1,888,530,827** | **19,672** | **1.64×** | **2,077,384,000** |

**G2 does not fit G1's 12,000 per frame at 8 voices.** It needs 19,672, 7,672 more than the cap (64 % over). The cap is not raised here: the reference keeps its own hard limit, and the table below says what each choice would buy, for the coordinator to pick.

**Where the instructions go** (callgrind, inclusive, the reference):

| Part | Share | Notes |
| --- | --- | --- |
| `dist` | 39.4 % | the asymmetric curve calls libm `exp` (10.0 % of the whole run); 2× oversampling doubles the curve evaluations |
| `wt` (unison and PM) | 35.0 % | `UnisonVoice::render` 33.9 %; the `floorf` calls take 4.7 % of the run |
| the right-channel twin of the voice-FX chain | 26.3 % | already counted in the `dist`, `formant` and `ladder` shares, which cover both channels |
| `formant` | 7.2 % | |
| `ladder` | 6.0 % | |
| `ott` | 4.9 % | |
| `osc` (the PM source) | 2.1 % | |
| `chorus` | 1.5 % | |
| `delay` | 0.8 % | about 150 instructions per frame |

The cause is S2a's stereo voice path ([dsp/engine.md](dsp/engine.md#stereo-voice-path)): a `wt` with `unison` ≥ 2 makes the plan stereo, so every voice-FX module is instantiated twice (a right-channel twin) and each voice runs `dist`, `ladder` and `formant` twice. The chain is 52.7 % of the run, half of that is the twin. The unison lanes always compute 8 copies, so 7 copies cost almost what 8 do: going to 3 copies saves 4.4 %, and only 1 copy (which turns the stereo path off) halves the run.

**What each choice costs** (the bench measures every row; per frame, same chord):

| Variant | Per frame | Against the reference | Against 12,000 |
| --- | --- | --- | --- |
| Reference (8 voices, 7 copies, dist asymmetric ×2) | 19,672 | | 1.64× |
| `dist` oversampling ×4 | 26,186 | +33 % | 2.18× |
| `dist` soft curve | 19,612 | −0.3 % | 1.63× |
| `dist` hard clip | 15,121 | −23 % | 1.26× |
| `dist` oversampling off | 15,598 | −21 % | 1.30× |
| MSEG not on the table position | 19,390 | −1.4 % | 1.62× |
| no `ladder` | 18,411 | −6.4 % | 1.53× |
| no `formant` | 18,181 | −7.6 % | 1.52× |
| 3 unison copies | 18,805 | −4.4 % | 1.57× |
| 1 copy (mono plan) | 9,637 | −51 % | 0.80× |
| 6 voices | 15,157 | −23 % | 1.26× |
| 4 voices | 10,617 | −46 % | 0.88× |
| 6 voices, `dist` hard clip | 11,745 | −40 % | 0.98× |

Between 4, 6 and 8 voices the reference costs about 2,260 instructions per voice and frame plus about 1,560 fixed (the effects, the skeleton); with the hard clip it is about 1,690 per voice. Two voices in 12,000 are what the cap buys: **4 voices of the reference, or 6 voices with the hard clip (11,745, 2 % under)**.

**Options for the coordinator** (nothing here is decided; the reference keeps 8 voices and its own limit until it is):

1. **A G2-specific cap** of 20,000 per frame at 8 voices (the reference is 1.6 % under it; the hard limit above is 21,640). Cheapest; it records that the stereo voice path costs what it costs. It does not make the real-time row pass (below).
2. **4 voices** (10,617): meets the cap and brings the p99 close to the 25 % line (below), but the chord is half as wide.
3. **6 voices with the hard clip** (11,745): meets the cap, changes the sound (the asymmetric curve is the "tube" of report 04's S4).
4. **Make the voice-FX twin cheaper**, which is lane work outside G2B: the right channel's `dist`, `ladder` and `formant` run the same code on a second buffer, so a SIMD pair (left and right in one pass) could remove up to half of the 26.3 %; and `exp` (10.0 %) and `floorf` (4.7 %) have cheaper forms. That is an estimate of up to about 27 % together, not a measurement: about 14,400 per frame, nearer the cap and still over it.

#### The swap blocks, in instructions

`benches/g2.rs` measures the blocks a swap lands in, the way `benches/swap.rs` does for G1: 64-frame blocks of the held chord after 0.5 s, the engine and its plan link, one block each.

| Case | G2 instructions | Against a normal block | Hard limit | G1 (`benches/swap.rs`, against its normal block) |
| --- | --- | --- | --- | --- |
| `normal` | 1,257,684 (19,651 per frame) | 1× | 1,384,000 | 408,776 |
| `instant` (the block an instant swap lands in) | 1,351,210 | 1.07× | 1,487,000 | 449,504 (1.10×) |
| `crossfade_swap` (the block a crossfading swap lands in) | 3,405,141 | 2.71× | 3,746,000 | 2,495,571 (6.10×) |
| `crossfade` (the 3 blocks after it, together) | 7,561,321 (2,520,440 each) | 2.00× per block | 8,318,000 | 2,411,695 (1.97× per block) |

The swap block's extra over a crossfade block is 884,701 instructions for G2 (the delay's and the chorus's state) against 1,691,673 for G1 (the reverb's). Instruction counts under-state a memory copy, so the move test and the wall clock above are the guards for the instant swap, not these budgets. `crates/enth-engine/tests/g2.rs` runs both swaps under `assert_no_alloc`.

#### Real-time rows (wall clock)

**Conditions.** The release bundle of main `f59fb60` plus this lane (the G2 patch with `delay` in the chain), `enth-test-host process-stats --require-rt`: rtkit `SCHED_RR` 10, `RLIMIT_RTTIME` 200 ms, not pinned, 64 frames at 48 kHz, one instance, 120 s after a 10 s warm-up (60 s for the 6- and 4-voice variants); G1 beside G2 in the same session, two passes over the 8-voice patch and G1. The raw reports, with the environment snapshot and the load at the start and end of each run, are in `~/code/enth-scratch/s2/g2b/rt/out/`.

**Measured on a loaded machine (load 19.7–32.8).** `flock -x /tmp/enth-quiet.lock` could not be had: other lanes' gates held `flock -s` for the whole 30 minutes the brief allows (07:05–07:35), and long-running jobs of the owner's (three data-generation processes at about 200 % CPU each, a video call, a browser) kept the load at 20–33 on this 12-thread i7-9750H even when no lane held the lock. So every row below carries the caveat, and **the comparison that counts is G2 against G1 in the same session**. Kernel 7.0.0-38 `PREEMPT_DYNAMIC`, governor `powersave` (EPP `balance_performance`), 11.4 GB of swap in use, 4.2 GB of memory available. For scale, G1's own steady p99 read 162 and 171 µs here against 112.5–123.3 µs on a quiet machine (lane RT, 2026-10-20): the load costs about 1.4×, and about 2× on the memory-bound crossfading swap block (G1: 656–691 µs against 337–339 µs).

Steady, no edits (p99 as a share of the 1,333 µs deadline in brackets; the gate is 25 % = 333 µs):

| Patch | Voices | Instructions per frame | p50 | p99 | p99.9 | Max | Deadline misses (of 90,000) |
| --- | --- | --- | --- | --- | --- | --- | --- |
| G1 | 16 | 6,285 | 143 / 144 µs | 162 / 171 µs (12.1 / 12.8 %) | 173 / 190 µs | 1,083 / 288 µs | 0 / 0 |
| **G2** | **8** | **19,672** | **554 / 556 µs** | **642 / 628 µs (48.1 / 47.1 %)** | **682 / 669 µs** | 3,364 / 945 µs | 5 / 0 |
| G2, 6 voices (60 s) | 6 | 15,157 | 431 µs | 480 µs (36.0 %) | 504 µs | 605 µs | 0 |
| G2, 4 voices (60 s) | 4 | 10,617 | 314 µs | 356 µs (26.7 %) | 374 µs | 529 µs | 0 |

With an edit every 250 ms (480 swaps a run, 240 for the 60 s run): `--swap-state`, G1 and G2 each against a variant of themselves (`*-value`: one generator's level changed, an instant swap; `*-lfo3`: an unrouted voice LFO added, a crossfading swap).

| Patch | Instant swap: all blocks p99 | Instant swap block p99 (max) | Crossfade: swap block p99 (max) | Crossfade blocks p99 (3 after the swap) | Misses (of 90,000) |
| --- | --- | --- | --- | --- | --- |
| G1 | 177 / 174 µs (13.2 / 13.1 %) | 208 / 203 µs (547 / 213) | 691 / 656 µs (753 / 775) | 321 / 317 µs | 0 |
| **G2, 8 voices** | **654 / 620 µs (49.0 / 46.5 %)** | **713 / 672 µs (739 / 681)** | **1,423 / 1,420 µs (1,495 / 1,434)** | **1,232 / 1,209 µs** | 3 and 0 instant; **115 and 55 crossfading** |
| G2, 4 voices | 360 µs (27.0 %) | 395 µs (402) | 851 µs (903) | 662 µs | 0 |

(Two runs are two numbers, pass 1 then pass 2; the 4-voice rows are one 60 s run.)

**Reading them.**

- **G2 at 8 voices does not meet the 25 % row.** Steady p99 is 47–48 % of the deadline here, 3.7–4.0× G1's in the same session. Scaling by G1's quiet-machine p99 (116 µs, the median of lane RT's three runs) puts it at about 430–460 µs (32–35 %) on a quiet machine: an estimate, not a measurement, but 8 voices of this patch are over the line by more than any load correction. The wall clock is worse than the instruction ratio (3.1× G1's per frame): 3.7–4.0× for the steady blocks; the extra was not isolated.
- **4 voices is at the line** on this loaded machine (356 µs, 26.7 %, instant-swap blocks 27.0 %) and about 250 µs (19 %) after the same correction; 6 voices is about 335 µs (25 %).
- **The crossfading swap block is over the deadline at 8 voices** (p99 1,420–1,423 µs, 107 %, max up to 1,495 µs, 55 and 115 deadline misses in 90,000 blocks), 2.1× G1's 656–691 µs in the same session. The same correction (G1: 337–339 µs quiet against 656–691 here, 0.5) gives about 700 µs (53 %). At 4 voices it is 851 µs (64 %). The three blocks after it, which run both plans, are 1,209–1,232 µs (91–92 %) against G1's 317–321 µs.
- **Why the crossfade is that tall.** The two plans run together for four blocks, so each costs about twice a normal block (crossfade-block p50 1,087–1,098 µs against a normal block's 561–567 µs in the same runs: 1.9×; the instruction counts say 2.0×), and G2's normal block is already 3.9× G1's. On top, the swap block copies the surviving effect state, and **that copy is the delay's and the chorus's**: the swap block's p50 less the crossfade blocks' p50 is **153–155 µs at 8 voices and 161 µs at 4**, for the delay's 768,000-byte rings plus the chorus's 32 KiB; G1's reverb (1.6 MB) is 256–266 µs the same way. That is 12 % of the deadline for the copy alone, 0.88 M instructions over a crossfade block (`benches/g2.rs`, `crossfade_swap` 3,405,141 against 2,520,440 for each crossfade block). OTT's state is small and shows nothing.
- **A pre-effects crossfade would not rescue 8 voices.** The lane RT note above says to reopen it when a second large-state effect lands, and the delay is that effect. But the copy is 153–155 µs of 1,420, and a blend before one effect chain would save the effects' second run as well (OTT, chorus and delay are 7 % of a block, about 40 µs): about 200 µs in all, which leaves the swap block at about 1,200 µs. The cost is the doubled voice chains, so only fewer or cheaper voices help.
- **Instant swaps are fine in themselves**: the instant-swap block is 1.07× a normal block in instructions and p99 713 / 672 µs against 654 / 620 µs for all blocks, the move works with the delay's rings in the plan. They fail the 25 % row only because the steady block does.

#### Verdict

| Check | Result |
| --- | --- |
| G2's instructions within G1's 12,000 per frame at 8 voices | **No: 19,672 (1.64×).** 4 voices (10,617) or 6 voices with the hard clip (11,745) fit; the cap was not raised |
| G2 within its own budget | Yes: 2,077,384,000 per 2 s render (21,640 per frame), every `benches/g2.rs` case passes its hard limit |
| p99 under 25 % of the deadline with an edit every 250 ms | **No at 8 voices:** 46.5–49.0 % on a loaded machine (about 32–35 % corrected by G1's load factor). 4 voices: 26.7–27.0 % loaded, about 19 % corrected |
| Crossfading swap block p99 against the 1,333 µs deadline | **Over at 8 voices:** 1,420–1,423 µs loaded (107 %; about 700 µs, 53 %, corrected); 851 µs (64 %) at 4 voices |
| G1 unchanged | Yes: 603,331,584 instructions against 603,331,580 measured on this branch before the merges (+4, 0.000001 %); `benches/g1.rs` and G1's fixtures are untouched |

The plan's assumption that G2 fits G1's 12,000 per frame at 8 voices ([phase-s2.md](phase-s2.md#open-checks), "G2's voice count and budget") does not hold. The reference stays at 8 voices, the plan's default, with its own measured hard limit, and the table above prices each way to the cap, for the coordinator to choose.

**Reproduce:**

```bash
export GUNGRAUN_RUNNER=~/code/enth-scratch/lane-b/tools/bin/gungraun-runner
flock -s /tmp/enth-quiet.lock cargo bench -p enth-engine --bench g2      # the budgets (exit 0 = all hold)
B=target/bundled/enth-plugin.clap; G=tools/test-host/golden               # cargo xtask bundle enth-plugin --release
NOTES="E1@0+3600:0.8, B1@0+3600:0.8, E2@0+3600:0.8, G2@0+3600:0.8, B2@0+3600:0.8, E3@0+3600:0.8, G3@0+3600:0.8, B3@0+3600:0.8"
flock -x /tmp/enth-quiet.lock enth-test-host process-stats $B --state $G/enth-plugin.g2.patch.json --notes "$NOTES" --require-rt --rt 10 --secs 120 --warmup 10
flock -x /tmp/enth-quiet.lock enth-test-host process-stats $B --state $G/enth-plugin.g2.patch.json --notes "$NOTES" --require-rt --rt 10 --secs 120 --warmup 10 \
  --swap-state $G/enth-plugin.g2-lfo3.patch.json --swap-kind crossfade     # or g2-value.patch.json --swap-kind instant
# the same with enth-plugin.g1.patch.json (no --notes) for G1 beside it; fewer voices: drop notes from $NOTES
cargo test -p enth-golden --test g2 -- --ignored write_the_fixtures      # regenerate the fixtures after changing benches/common/g2.rs
```

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
