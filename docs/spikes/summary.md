# Phase 0 summary (Task 7)

- Date: 2026-10-19. Lane Z, after every lane was merged into `main` (last merge `64414bf`, 09:19).
- Sources: the lane docs in this folder ([shell](shell.md), [engine](engine.md), [metric](metric.md), [ui](ui.md), [test-host](test-host.md), [agent-probe](agent-probe.md)), the research summary `_local/sessions/2026-10-19-phase-0-spikes/summary.md`, `git log`, the lanes' scratch notes, and the GitHub Actions runs of 2026-10-19.
- Companions: [owner-checklist.md](owner-checklist.md) (everything waiting on the owner), [environment.md](environment.md) (the machine).

## Verdict

**The stack holds, and G0 passes on everything an agent can measure.** nice-plug carries a 72-parameter CLAP/VST3 synth with a full egui editor through both validators and headless REAPER. Renders are bit-exact across Linux, Windows and wasm32; the 1-voice template renders about 20 times faster than the G0 floor. The UI costs nothing while idle and about 4 % of a core while animating, before the GPU. Two kinds of thing remain. **The owner's checks**: the real NVIDIA display, the ears for the metric and the agent trial, Bitwig, a real audio device. **One known failure**: baseview 0.4.1 leaks about 77 KB per editor open; 61 KB of that is one missing `xkb_keymap_unref`, and a draft upstream report is ready. ADR-0001 and ADR-0002 are accepted; ADR-0007 waits for the listening sessions.

## What Phase 0 learned

- **Shell (A).** nice-plug 0.4.2 passed every hard must-have on the first build (clap-validator 36 / 0, Steinberg 537 / 537, 45 REAPER checks for CLAP and VST3, state round trips in REAPER and Carla). CLAP slot renaming needs a 123-line nice-plug patch; VST3 in REAPER stays "Slot N". REAPER restarts CLAP plugins about 30 ms into an offline render, so its renders can't be goldens.
- **UI (U1, H).** The full reference panel draws 0 frames while idle and costs 1.2–1.4 ms of CPU per animating frame; clippy and a source scan enforce the design tokens. Every remaining UI risk sits in baseview, below egui: the keymap leak, ≈30 fps pacing (60 Hz requested gives 29 fps, 30 Hz gives 20), HiDPI clipping in REAPER, and Space lost to the host after a click in the editor. All measured under Xvfb; nothing has run on the real display.
- **Engine (B).** 1,010 hashes are identical on Linux, Windows (CI) and wasm32, across build flags, SIMD lanes and FTZ/DAZ, as long as DSP follows the four rules, which clippy enforces. Without them `std` maths changes 13 of 1,000 seeded *patches*. 1,993× realtime for one voice, 152× for 8 P1-like voices with reverb (one core, scalar). Instruction budgets are the regression gate and also hold on GitHub's runner. **Block size is part of the render function** (32 = 64, 48 differs).
- **Metric and search (C).** An evaluation costs 7.4–7.8 ms; 8 workers give 4.3–4.7× with bit-identical results; distance A with a heuristic start reaches the strict proxy on 17 of 20 targets, median 3.5 s. The research's portfolio and λ = 16 gains didn't reproduce. Nobody has listened, so the metric is unvalidated.
- **Test host (H).** Plugin-level goldens (25 ms per 2 s render, in CI) and a leak harness that refuses a real display. No cross-instance state in the plugin; the editor leak attributed with valgrind.
- **Agent loop (P).** With the expectation list given, three fresh agents passed 15 of 15 goals in 1–7 renders, so that alone can't be the G1 gate. Their trouble pointed at descriptor fixes (beat-smoothed envelopes, multi-parameter hints, a master level, `describe` on a WAV with its performance, margins in `check`).
- **Process.** Seven agent lanes in about 5 hours of wall clock on one shared laptop (load 2–30). The no-real-display rule held, at the price of software-GL numbers for every GUI measurement.

## G0 at a glance

Full tables: [roadmap.md](../roadmap.md#p0--spikes-and-skeleton-done-2026-10-19-except-owner-checks) and [benchmarks.md](../benchmarks.md#ui-thresholds).

| Gate | Number | Status |
| --- | --- | --- |
| Validators | clap-validator 36 passed / 0 failed; Steinberg 537 / 537 | Passed |
| Hosts | REAPER CLAP + VST3, 45 checks, 7 / 7 runs; Carla VST3 load and state | Passed; Carla play and automate pending owner |
| GUI-caused xruns | not measured (Dummy Audio) | Pending owner |
| Engine ×realtime, 1 voice | 1,993× best, 1,783× median | Passed |
| Engine ×realtime, P1-like (8 voices, reverb) | 152× / 149× scalar, 231× / 217× f32x8 | Passed |
| Cross-platform hashes | 1,010 identical on Linux, Windows (CI), wasm32 (wasmtime, ± simd128) | Passed; aarch64 pending |
| Instruction budgets | 6 cases at measured + 10 %, e.g. 1 voice 8.15 M (budget 9.0 M), P1-like scalar 112.4 M (124 M) per 2 s render | Passed (locally and in CI) |
| Editor idle | 0 frames in 22 s; GUI thread 0.05–0.19 %; 64 wake-ups/s | Passed |
| Frame CPU | 1.16–1.41 ms per animating frame, p99 ≤ 2.30 ms (3.5–4.2 % at 30 fps) before the GPU | Passed on the CPU side; GPU pending owner |
| Open time | 53.1 ms median, 87 ms max (test host); 44–67 ms (REAPER) | Passed (Xvfb); real display pending owner |
| PSS growth per editor cycle | 76.9 KB above baseline; 16.1 KB with the one-line xkb fix | **Failed** (baseview) |
| Threads per editor cycle | 3 closed / 30 open, constant over 200 cycles | Passed |
| PSS per open editor | +71 MB under llvmpipe (not a GPU number) | Pending owner |
| First open | 200 / 200 under Xvfb | Pending owner (real display) |
| Host checklist | HiDPI clipped, Space lost after a click (Xvfb) | **Failed** (baseview); rest pending owner |
| Metric | no ratings yet | Pending owner |
| Search | 17 / 20 at the strict proxy, median 3.5 s, max 10.9 s; no ABX tool | Pending owner (and an ABX tool) |

## What changed in the plan

- **ADRs:** 0001 and 0002 accepted at G0 (0002 lists what would reopen it); 0007 stays Proposed until the listening sessions; 0006 unchanged.
- **design.md:** §5 bit-exact across platforms (Windows proven, aarch64 on the old fallback), block size as a render input with host re-chunking, how the four rules are enforced and tested, plugin goldens through the test host; §6 beat-aware descriptors, metric v1, the listening protocol, margins and multi-parameter hints; §7–§8 lane C's numbers, without the portfolio and λ claims; §9 the 13-in-1,000 seed result.
- **benchmarks.md:** measured value and G0 status per UI threshold; engine numbers, absolute instruction budgets and P1 speed targets (≥ 1,000× for 1 voice, ≥ 100× for 8 P1-like voices, scalar); 60 Hz scope requests; valgrind, not heaptrack, for editor leaks.
- **roadmap.md:** the G0 result table; G1's 16-voice budget (below) and agent-eval protocol.
- **AGENTS.md:** re-chunking, where the numeric clippy bans go, the scope rate, lavapipe-only snapshots, editor leak checks and `cargo xtask host-render`.

### The 16-voice CPU budget for G1

Proposed: **≤ 340 M instructions per 2 s render (≈ 220 per sample and voice), about 50× realtime on one core of this laptop, scalar path**, for a 16-voice P1 patch with every P1 module in use, reverb and compressor on; plus p99.9 `process()` under 25 % of the 64-sample deadline in the test host.

How it was derived ([engine.md](engine.md#speed)):

1. The spike's P1-like voice (3 saws, filter, 2 envelopes, an LFO, a 4-line FDN reverb) costs 146.3 instructions per sample and voice: 112.4 M per 2 s render at 8 voices, which ran 149–152× realtime.
2. Scaled to 16 voices: ≈ 225 M instructions, ≈ 75× realtime scalar (≈ 115× with f32x8 lanes).
3. The P1 engine adds what the spike lacks: noise, wavetables, a modulation matrix and macros, a real reverb (the Signalsmith port, larger than 4 delay lines) and a compressor. Allow 1.5× the spike's per-voice cost: ≈ 220 instructions per sample and voice, so 220 × 96,000 × 16 ≈ 338 M per 2 s render.
4. This machine retires about 8–8.5 G instructions per second in these renders (112.4 M in 13.2 ms; 122.4 M in 15.5 ms). So 340 M takes 40–43 ms per 2 s of audio: 47–50× realtime, 2 % of a core per 16-voice instance.

The instruction count is the gate (load-independent); the ×realtime figure is information. Revisit the 1.5× allowance in the P1 research session once the module list is fixed.

### Corrections and things I couldn't reconcile

- [engine.md](engine.md) says Windows hashes have not run. They have since: `engine-golden` on GitHub passed `hashes on windows-x86_64` on 2026-10-19 (runs 39201513259 and 39209206987: all 1,010 hashes, FTZ and subnormal tests). The aarch64 job is manual-only and skipped.
- [test-host.md](test-host.md) says REAPER runs only under Wine on this machine. Native REAPER 7.82 is installed and used by `reaper-test` ([environment.md](environment.md)), so the REAPER cross-check of the second-instance difference just wasn't done. It goes to P1b.
- Research 03's "instruction counts under-report the denormal penalty by about 18×": lane B measured 4.1× more instructions against 33.5× more wall time on a 40 s tail. benchmarks.md now quotes lane B.
- benchmarks.md and phase-0.md said scopes call `request_repaint_after(1/30 s)`; the code and design-system.md use 60 Hz, because 1/30 s lands on ≈20 fps. Fixed to 60 Hz.
- `cargo-deny` on `PATH` is 0.19.4; CI and Task 1 pin 0.20.2 (only scratch copies match). Task 1's acceptance passed on the wrong version.
- Not measured by anyone, though automated: xruns with 8 instances at 64 samples (it needs a real audio device, so it went to the owner), knob → sound latency, p99.9 `process()` and plan-swap time (all P1 test-host work), and the scalar-FFT scoring cost (P1a).

## Integration bugs (Task 4 track)

RustAudio's AI policy rules out agent-written patches upstream, so these are reports the owner writes or vouches for, or fixes in our own code.

| Bug | Status | Link or reason |
| --- | --- | --- |
| Editor open/close leak | Root-caused: baseview 0.4.1 `XkbcommonState::new` never unrefs its keymap (61 of 77 KB per cycle); still so on baseview's `master` on 2026-10-19. The remaining 16 KB is unattributed (about 3.4 KB from EGL under Mesa) | Draft report `_local/drafts/baseview-xkb-keymap-leak.md` (not committed), waiting for the owner |
| HiDPI clipping in REAPER | Reproduced under Xvfb at `Xft.dpi: 192` (research 02) | [baseview #359](https://github.com/RustAudio/baseview/issues/359); real display: owner checklist C5 |
| First-frame stall on NVIDIA | Not seen under Xvfb (200 / 200 test host, 22 / 22 REAPER); upstream says fixed (2026-10-18), but the standalone window still stalled on this GPU in research 02 | [nice-plug #95](https://codeberg.org/RustAudio/nice-plug/issues/95); owner checklist B1, C1 |
| Focus policy (Space lost after a click in the editor) | Reproduced under Xvfb; typing into a field works | Not reported: whether focus returns to REAPER needs a real window manager first (owner checklist C4) |
| `editor_closed()` never called | Documented (0 calls in 22 closes); worked around by rebuilding in `build()` | Not reported yet; low priority, workaround is cheap |
| "X11 thread disconnected" on close | Not reproduced: 0 lines in 400 test-host cycles | Nothing to report |

## Effort: estimated vs actual

All lanes ran on 2026-10-19 (times +03:00). Start = the lane's worktree or scratch folder; end = its last commit (the merge follows within minutes). Estimates are from [phase-0.md](../phase-0.md), in developer-days. **Agent hours are not human days**: see the notes under the table.

| Lane | Task | Estimate | Start → last commit (merge) | Elapsed |
| --- | --- | --- | --- | --- |
| Research | 0 | 1 day | ≈ 01:30 (`brief.md`) → 06:26 summary committed; 06-final 08:01 | ≈ 5 h (6.5 h with 06-final), 7 researchers in parallel, one reboot and one Xorg crash |
| Machine setup | 1 | ½ day | not in git (REAPER unpacked 02:01; `environment.md` written by lane Z) | — |
| S, skeleton + CI | 2 | 1 day | drafted and verified inside research 05 (scratch 02:14 → 04:06), committed 04:23, CI green 37 s later | ≈ 2 h, as part of research 05 |
| A, shell | 3 | 3–4 days | 04:45 → 06:20 (fast-forward 06:23) | 1 h 35 min |
| B, engine | 5 | 2 days | 06:09 → 07:47 (07:51) | 1 h 38 min |
| C, metric and search | 6 | 3 days | 06:04 → 07:43 (07:46) | 1 h 39 min |
| U1, tokens and panel | 4 | 4–5 days for U1 + H + U2 together, plus 2–3 days for the integration-bug track | 06:24 → 09:11 (09:19) | 2 h 47 min |
| H, test host | 4 | (in Task 4) | 08:07 → 08:55 (08:59) | 48 min |
| U2, panel in the host | 4 | (in Task 4) | not run as a lane: U1 and H made its Xvfb measurements, the real-display items went to the owner checklist | 0 |
| P, agent probe | — | none (research 07: half a day) | 07:46 → 08:49, including the 3-agent trial (08:42–08:48) | 1 h 03 min |
| Z, close | 7 | ½ day | 09:20 → 09:46 (this commit) | 26 min |

The lanes A–P took about 9.5 agent-hours in all and about 5 hours of wall clock (04:23 → 09:19). The plan for the same work was 14.5 developer-days (S, A, B, C, Task 4, Z), "about 2 weeks" of calendar time. Read the gap with care:

- **Parallelism:** up to three lanes at once (A, B, C 06:09–06:20; B, C, U1 06:24–07:43; U1, P, H 08:07–08:49).
- **Head start:** each lane began from a working research probe (01's `spike_np`, 03's engine probe, 04's metric probe, 05's verified skeleton, 02's UI measurements), so part of the research hours belongs to the lanes.
- **Left out:** the owner's time (about 3 hours of checks to come, now the critical path), the orchestrator's reviews and merges, and scope cut before or during the lanes (vizia and WebView, U2, the ABX tool, most of the integration-bug track).
- **Spike code:** throwaway and single-author, with no migrations or cross-lane integration; P1 product code will not move at this rate.

**Calibration for P1:** for well-specified agent spikes with a working probe, budget hours per lane, not days. Human-day estimates overstated wall clock by about an order of magnitude and said nothing about owner time. The P1 research session should re-estimate P1a and P1b as agent-hours per lane plus owner-hours per gate.

## Next for P1

1. **P1a research session** (`_local/sessions/2026-10-19-p1a-headless-core/`, started), then the owner reads the summary before any milestone code (AGENTS.md).
2. **Carry into P1a:**
   - fixed internal block with host re-chunking (design §5);
   - measure the scalar-FFT scoring cost;
   - the descriptor fixes and margins in `describe` / `check`;
   - an ABX tool for the G0 search gate;
   - the Signalsmith reverb port with a golden against the C++;
   - the G1 instruction budget as a CI gate from the first engine commit;
   - move the spike's numeric clippy bans into the root `clippy.toml`.
3. **Carry into P1b:**
   - the slot-rename decision (fork or "Slot N");
   - how we depend on baseview: wait for upstream, or a fork with the xkb fix (an ADR-0002 note);
   - knob → sound latency and p99.9 `process()` in the test host;
   - the REAPER cross-check of the second-instance difference;
   - notes the plugin triggers itself while the transport is stopped (Task 3 leftover);
   - lay out only what is visible in the editor (the biggest frame saving in [ui.md](ui.md#what-id-still-improve)).
4. **Owner, whenever:** [owner-checklist.md](owner-checklist.md). Its results update ADR-0002, ADR-0007 and the G0 table.
5. **Quiet-machine timings** (load under 1, `perf_event_paranoid` lowered by the owner) before any wall-clock number is quoted as more than an upper bound.
