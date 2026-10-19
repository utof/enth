# Benchmarks and measurement protocol

**Measure what users feel, not just averages.** A plugin that averages 1% CPU but occasionally takes 3 ms on the audio thread causes dropouts; an editor that idles at 0% but leaks 2 MB per open/close bloats a long session. This doc defines what we measure, how, and the pass/fail thresholds.

Three areas: the **UI stack** (decided in Phase 0), the **engine** (tracked forever), and the **agent loop** (from Phase 1).

## What to watch beyond CPU, GPU, RAM and startup

| Dimension | Why it matters | Metric |
| --- | --- | --- |
| Worst-case audio block time | Dropouts come from spikes, not averages | p99.9 and max `process()` time vs the block deadline |
| Dropouts (xruns) while the GUI is busy | A UI that steals CPU or locks causes clicks | xrun count over 10 min with editors animating |
| Cost with the editor **closed** | Most instances in a project are closed | GUI-thread CPU ≈ 0, no timers |
| Cost when **occluded or minimised** | Hidden editors shouldn't keep drawing | CPU ≤ idle value, ideally 0 |
| Memory per instance (PSS, not RSS) | RSS double-counts shared libraries | Marginal PSS per instance, closed and open |
| Scaling | Fixed per-instance costs multiply | 1 / 10 / 50 instances |
| Leaks | Long sessions open editors hundreds of times | PSS growth after 200 open/close cycles |
| Frame time and jank | Dragging a knob must feel instant | Frame time p50/p99; frames > 50 ms |
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
- 2 scopes: an oscilloscope (2048 samples) and a 512-bin spectrum, fed from the audio thread through a lock-free ring at 60 Hz.
- A lesson pane: ~1,500 words with headings, bold text, one image and a scrolling list.
- Window 1100×768 logical pixels; test at scale 1.0 and 2.0.

**Repaint gating (egui):** repaint only on input, on a parameter change, or when a *visible* scope has new data; stop entirely when the window is hidden or the editor is closed.

## Procedure

1. REAPER, 48 kHz, 64-sample buffer, 8 instances each playing a held chord. Editor states: closed → open idle → open animating → open but covered by another window → minimised. 120 s per state.
2. Repeat with 1, 10 and 50 instances (editors closed), then 10 editors open.
3. In `tools/test-host`: 200 scripted open/close cycles; editor open time; knob → sound latency; process-time distribution.
4. Repeat a subset in a second host (Bitwig or Ardour).
5. Run the host-integration checklist.

### Tools (Linux)

```bash
pidstat -t -p "$(pidof reaper)" 1 120 > cpu.txt                 # per-thread CPU
smem -P reaper -c "pid pss uss" ; cat /proc/<pid>/smaps_rollup   # PSS / USS
nvidia-smi dmon -s um -d 1                                       # GPU SM % and memory
perf stat -e task-clock,context-switches -p <pid> -- sleep 60    # overall cost
perf record -g -p <pid> -- sleep 30                              # where the GUI spends time
heaptrack <test-host> …                                          # allocations / leaks
sudo powertop                                                    # watts on battery
clap-validator validate ~/.clap/enth.clap                        # correctness
```

REAPER's *Performance Meter* (View menu) shows real-time CPU and xruns per track.

## UI thresholds

Proposed defaults; Phase 0 may adjust them with evidence.

| Metric | Pass |
| --- | --- |
| Extra xruns attributable to the GUI (10 min, 8 instances, 64-sample buffer) | 0 |
| Real-time CPU increase from opening an editor | < 1% |
| GUI CPU, editor closed | ≈ 0% (no timers) |
| GUI CPU, open and idle | < 1% of one core per editor |
| GUI CPU, open and animating (scopes 60 fps) | < 5% of one core per editor |
| GUI CPU, occluded or minimised | ≤ idle |
| Frame time p99 | < 16.7 ms; no frame > 50 ms during a knob drag |
| Marginal PSS per open editor | < 30 MB |
| Marginal PSS per closed instance (above DSP state) | < 5 MB |
| PSS growth after 200 open/close cycles | < 2 MB |
| Editor open time | < 150 ms warm, < 400 ms cold |
| GPU per open editor | < 5% SM, < 100 MB VRAM |
| Knob → sound latency | ≤ 1 audio block + 1 frame |
| Host-integration checklist | All pass in REAPER and one other Linux host |

### Host-integration checklist

- [ ] Opens, closes and reopens without crashing or leaking
- [ ] Resizes (host-driven and plugin-driven); correct at HiDPI scale 2.0
- [ ] Keyboard: typing into a value field works; spacebar still reaches the host for transport when no field is focused
- [ ] Copy/paste in text fields
- [ ] Drag-and-drop inside the editor
- [ ] Tooltips and context menus appear in the right place
- [ ] Mouse wheel and fine-drag (Shift) on knobs
- [ ] Host automation moves the knobs; knob moves record as automation
- [ ] Works when the host runs the plugin bridged/sandboxed (REAPER option)

## Decision rules for the UI toolkit (Gate G0)

1. Reject any arm with GUI-caused xruns, a crash or hang in REAPER, or failing the PSS or CPU thresholds after reasonable tuning.
2. Among the survivors, pick **egui** unless vizia is at least 30% cheaper on "open and animating" CPU **and** the lesson pane is clearly better.
3. WebView (if measured) is chosen only if it passes every threshold on this machine *and* a native arm fails — it is the fallback, not the default ([ADR-0002](adr/0002-ui-toolkit.md)).

## Engine benchmarks (tracked from Phase 1, in CI)

| Metric | Tool | Target |
| --- | --- | --- |
| Offline render speed | `enth bench`, Criterion | ≥ 100× realtime per core for a basic patch *(set in P0)* |
| CPU per voice at 48 kHz | Criterion per module | Tracked per module; regressions > 10% fail CI |
| p99.9 / max `process()` time at 64 samples | test host | < 25% of the block deadline (1.33 ms at 48 kHz) |
| Plan swap time | test host | Swap without audible glitch; compile off-thread |
| Allocations on the audio thread | `assert_no_alloc` in engine tests; nice-plug's `assert_process_allocs` in plugin debug builds | 0 |
| Determinism | golden tests | Same-platform hashes identical |

## Agent-loop benchmarks (from Phase 1)

Defined in [agent-cli.md](agent-cli.md#evaluating-the-cli-like-a-product): solve rate, renders per solve, tokens per solve, invalid-command rate, on `content/agent-evals`.

## Results template

Copy into `docs/spikes/ui.md`:

| Metric | egui | vizia | WebView (optional) | Threshold | Notes |
| --- | --- | --- | --- | --- | --- |
| GUI CPU, closed | | | | ≈ 0% | |
| GUI CPU, open idle | | | | < 1% | |
| GUI CPU, open animating | | | | < 5% | |
| GUI CPU, occluded | | | | ≤ idle | |
| xruns (10 min) | | | | 0 | |
| PSS per open editor | | | | < 30 MB | |
| PSS growth, 200 cycles | | | | < 2 MB | |
| Frame time p99 | | | | < 16.7 ms | |
| Editor open (warm / cold) | | | | 150 / 400 ms | |
| GPU SM % / VRAM | | | | 5% / 100 MB | |
| Watts, open animating | | | | report | |
| Checklist (REAPER / 2nd host) | | | | all pass | |
