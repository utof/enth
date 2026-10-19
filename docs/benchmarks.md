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
| Worst-case audio block time | Dropouts come from spikes, not averages | p99.9 and max `process()` time vs the block deadline |
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

**Repaint gating (egui):** repaint only on input, on a parameter change, or when a *visible* scope has new data (scopes call `request_repaint_after(1/30 s)` only while visible and fed); stop entirely when the editor is closed. Measured 2026-10-19 under Xvfb: 0 frames while idle, about 26 frames/s while host automation plays. Name the GUI thread (`prctl(PR_SET_NAME)` inside `build()`) and log `CLOCK_THREAD_CPUTIME_ID` per window of frames, to attribute CPU per editor.

## Procedure

1. **Automated tier.** Headless REAPER under Xvfb (Dummy Audio, driven by `cargo xtask reaper-test`-style Lua scripts), 48 kHz, 64-sample buffer, 8 instances each playing a held chord. Editor states: closed → open idle → open animating (→ covered → minimised, owner tier only: under Xvfb a covering window does not stop drawing, because there is no compositor). 120 s per state. The REAPER copy used for automation is a throwaway, never your own config.
2. Repeat with 1, 10 and 50 instances (editors closed), then 10 editors open. Scaling to 50 instances and watts move to S1.
3. In `tools/test-host` (clack-host): 200 scripted open/close cycles, with threads and PSS recorded per cycle, under heaptrack for leak attribution (REAPER crashes under heaptrack); editor open time; knob → sound latency; process-time distribution.
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

Proposed defaults; Phase 0 adjusts them with evidence (lane U2 decides). The tier column says who measures: **A** automated (Xvfb), **O** owner-attended (real GPU).

| Metric | Pass | Tier |
| --- | --- | --- |
| Extra xruns attributable to the GUI (10 min, 8 instances, 64-sample buffer), by editor-open vs editor-closed A/B | 0 | A, O |
| Real-time CPU increase from opening an editor | < 1% | A |
| GUI CPU, editor closed | ≈ 0% (no timers) | A |
| GUI CPU, open and idle | < 1% of one core per editor; report wake-ups/s | A |
| GUI CPU, open and animating (scopes ≤ 30 fps) | < 5% of one core per editor (borderline at 20–30 fps: about 3.5–6%) | A, O |
| GUI CPU, occluded or minimised | ≤ idle (gate only on "closed" until the owner has checked a real window manager) | O |
| CPU time per frame p99 | < 16.7 ms; no frame > 50 ms during a knob drag; achieved fps reported | A |
| Marginal PSS per open editor | < 30 MB | A |
| Marginal PSS per closed instance (above DSP state) | < 5 MB | A |
| PSS growth after 200 open/close cycles | No growth beyond the host-only baseline; threads constant. The unfixed baseview leak is ≈70–160 KB per cycle, so the strict "< 2 MB" is a goal once it is fixed upstream | A |
| Editor open time | < 150 ms warm, < 400 ms cold (measured: ≈100 ms to the first frame under Xvfb) | A, O |
| First open | Draws within 1 s on 20 of 20 opens | O |
| GPU per open editor | < 5% SM, < 100 MB VRAM | O |
| Knob → sound latency | ≤ 1 audio block + 1 frame | A |
| Host-integration checklist | All pass in REAPER and one other Linux host (Carla, VST3) | A, O |

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

## Engine benchmarks (tracked from Phase 1, in CI)

| Metric | Tool | Target |
| --- | --- | --- |
| Offline render speed | `enth bench`, `hyperfine` | ≥ 100× realtime per core for a basic patch *(set in P0)*; record the 1-voice and 16-voice numbers (measured on toy voices: ≈500–2,000× per voice per core; 16 voices 50–236×), as min and median with the load average |
| CPU per voice at 48 kHz | **Instruction counts** (callgrind, gungraun), per module | Regression gate: a change that raises the instruction count of a benchmark fails CI. Criterion and wall-clock time are information only: with no code change Criterion reported two false "improvements" (−13%, −36%) under load |
| Subnormals | A deterministic test: render a 40 s release tail and count subnormal values | 0. Instruction counts under-report the denormal penalty by about 18×, so this test is half of the gate |
| p99.9 / max `process()` time at 64 samples | test host | < 25% of the block deadline (1.33 ms at 48 kHz) |
| Plan swap time | test host | Swap without audible glitch; compile off-thread |
| Allocations on the audio thread | `assert_no_alloc` in engine tests (debug builds only); nice-plug's `assert_process_allocs` in plugin debug builds | 0 |
| Real-time safety | RTSan, one nightly run (nightly Rust; the runtime ships in rust-std) | No findings. It catches `malloc` but misses an uncontended `std::sync::Mutex`, so `enth-engine` also has no `Mutex` or `RwLock` |
| Determinism | golden tests in CI | Hashes identical across platforms, wasm engines and build flags (Linux, wasm32 under wasmtime and node, Windows; aarch64 if runners exist); a feature summary with tolerances is only the diagnostic when a hash differs |

## Measurement hygiene

Every timing taken so far in Phase 0 ran at a load average of 4–30 on a shared laptop, so treat wall-clock numbers as upper bounds and instruction counts as the reference.

- Measure on a rebooted machine with heavy applications closed, the repo and `target/` on ext4, and nothing else building.
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
