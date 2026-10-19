# Phase 0 research — summary

- **Date:** 2026-10-19
- **Reports:**
  - first reports: `01`–`07`;
  - final reports: `01-final`, `02-final`, `03-final`, `04-final`, `05-final`, `06-final`;
  - also `_local/reference/ui/README.md` (UI references, local only).

  01–06 were cut short for a reboot, then revived and finished. 07 finished on its own.
- **Status:** the owner reads this at their own pace. Work proceeds on the assumption they agree; they steer if not.

## Verdict

**The plan holds: Rust, nice-plug, egui, CLAP first.** Nothing found argues for changing the stack. The evidence changes *how* Phase 0 runs and several numbers in the design:

- **Phase 0 runs as parallel lanes** after a one-day skeleton (done). That's about 2 weeks of calendar time, not 3–4. The shell lane is already merged: every hard must-have passes in native REAPER, headless.
- **Renders can be bit-exact across platforms, not just within one.** That holds for native x86-64 and wasm under V8 and wasmtime, across build flags and FTZ settings, if DSP follows four rules:
  - `libm` for transcendentals;
  - no `mul_add`;
  - fixed summation order;
  - denormals flushed in code.

  That makes "a challenge is just a seed" safe.
- **egui only, rendered with glow.** vizia and iced are out. The real risks sit in shared plumbing, not the toolkit: baseview (a small leak per editor open, a ≈30 fps cap, HiDPI clipping in REAPER) and an **NVIDIA 595 X-driver bug** that crashed Xorg today. So automated GUI work runs under Xvfb, and real-GPU checks need the owner present.
- **The scoring metric and search are cheaper than feared.** One evaluation (render + analysis) costs ~8 ms; a solve takes 2.6 s (median) on 8 workers; 18/20 targets are recovered with heuristic + random restarts side by side. Multi-resolution log-mel L1 is the provisional metric v1.
- **The machine was the biggest risk, not the code.** Three fixes are done:
  - the repo moved to ext4, which cut file-metadata calls from ~5 ms to ~3 µs;
  - native REAPER 7.82 is installed;
  - the reboot fixed the NVIDIA driver/library mismatch.

  Two remain: Carla 2.5.8 hosts VST3 only (no CLAP), and Bitwig (needed for poly-mod) isn't installed.

## Decisions

| Area | Decision | Evidence | Status |
| --- | --- | --- | --- |
| Toolchain | Pin Rust **1.99.1**, never 1.99.0 (it miscompiles x86_64 FFI `bool` returns, which CLAP uses everywhere). `CARGO_BUILD_WARNINGS=deny` in CI | 05 | Done (skeleton) |
| Workspace | `crates/enth-<name>/` directory names; `default-members` skip spikes; root `clippy.toml` bans I/O, clocks, threads and locks in core crates (wasm-check alone doesn't catch them); custom `check-layers`; xtask, no justfile | 05 | Done |
| CI | GitHub Actions on `ubuntu-24.04`, SHA-pinned actions, rust-cache writing only from main, cargo-deny 0.20.2, Dependabot with a 7-day cooldown, clap-validator 0.4.1 job | 05, 01 | Done; first run green |
| Licences | Allow BSD-1-Clause (`assert_no_alloc`) and `Ubuntu-font-1.0` (scoped to `epaint_default_fonts`); BSL-1.0 exceptions for two Windows clipboard crates | 01, 05 | Done (ADR-0004 note) |
| Repo | Private GitHub repo `utof/enth`; `_local/sessions/` committed, `_local/reference/` never | owner, 05 | Done |
| Shell | **nice-plug `=0.4.2` + nice-plug-egui `=0.6.0`**, from crates.io. (a), (c), (e), (f), (g) pass in REAPER (CLAP + VST3), Carla (VST3), clap-validator 36/0 and Steinberg's validator 537/537. clack + clap-wrapper stays the escape hatch | 01, lane A | Merged |
| Slot names | CLAP: rename via a 123-line add-only nice-plug patch, carried in a fork when needed. VST3 in REAPER: "Slot N" (REAPER rejects title changes) | 01 | Open: fork when P1 needs names |
| Host tests | `cargo xtask reaper-test`: headless REAPER + Lua, 3–5 s per scenario, local gate. A small clack-host test host only for CI and leak/CPU measurement | 01, 02, 06 | reaper-test merged |
| UI | egui with **glow**; no second toolkit. Every visual value comes from design tokens ([design-system.md](../../../docs/design-system.md)); direction "Lanes" (Phase Plant-like, flat, dark, two accents) | 02, UI refs, owner | Lane U1 running |
| GUI safety | Agents never open windows on the real display; Xvfb + Mesa for all automated GUI work; real-GPU checks owner-attended | 02, crash log | In AGENTS.md (pending) |
| Determinism | **Bit-exact renders across platforms**, enforced by golden hashes in CI; feature-summary tolerance only as a diagnostic. The old tolerance contract stays as a fallback until the Windows (and, if available, aarch64) CI hashes are green | 03, 05 | Lane B running |
| Scores across platforms | rustfft picks its code path by CPU (AVX/SSE/scalar give different bits), so distances differ by up to 2.7e-6 between native and wasm. For identical pass/fail decisions in duels, scoring uses a **scalar FFT path** (or a tolerance ≥ 1e-5 with a deterministic tie rule) | 03, 04 | Lane C/B to confirm cost |
| Denormals | Flush in code, end voices below −100 dB, never depend on or set FTZ/DAZ (nice-plug sets FTZ around `process()` anyway) | 03 | Design rule |
| Real-time checks | `assert_no_alloc` in debug tests; one nightly RTSan run; no Mutex/RwLock in `enth-engine` (RTSan misses uncontended std locks) | 03 | Lane B |
| Benchmarks | Regression gate = **instruction counts** (callgrind/gungraun) **+ a deterministic subnormal test**. Criterion reported false "improvements" twice with no code change; wall-clock is information only | 03 | Lane B |
| Search | Own ask–tell CMA-ES (no crate fits: `cmaes` panics on wasm, `basin` lacks ask–tell); λ ≥ workers; heuristic + random restarts side by side; waveform cycled at restarts; order-stable evaluator (results identical for 1 or N workers); 5,000 evaluations per solve | 04 | Lane C running |
| Metric v1 | Multi-resolution log-mel L1 (MFCC worst). Provisional; recalibrated by the owner's listening sessions and again in R1. CLAP embeddings (~0.65 s per call) only to re-rank final candidates | 04 | ADR-0007 draft (lane C) |
| Listening test | 2 sessions × 30 pairs + anchors (test–retest ceiling), Kendall τ-b with bootstrap CIs, ABX for "sounds the same"; a CLI the owner runs whenever. **Nothing blocks on it** | 04, 07 | Lane C |
| Dev loop | lld is already rustc's default linker; lean debuginfo dev profile (−15% clean build, half the memory, 4× smaller binaries); optimise named DSP crates, not all deps; stay on stable (Cranelift and the parallel front end don't pay yet); hot reload out for Phase 0 | 06 (final pending) | To add to Cargo.toml |
| Plugin IDs | Permanent, name-independent: CLAP `dev.enth.spike-shell` for the spike, plus a reserved product id ([naming.md](../../../docs/naming.md)) | 07, lane A | Done |

## Phase 0, re-planned as lanes

| Lane | What | Depends on | State |
| --- | --- | --- | --- |
| S | Skeleton + CI (Task 2) | — | **Merged**, CI green |
| A | Shell spike (Task 3): spike-shell, bundle/install, reaper-test, clap-validator CI | S | **Merged** |
| B | Engine + determinism (Task 5): cross-platform hashes, FTZ/lanes/subnormal tests, wasm vs native, Windows CI, instruction-count bench | S | Running |
| C | Metric + search (Task 6): features, 3 distances, CMA-ES, solve/bench, `listen` CLI, ADR-0007 | S | Running |
| U1 | Design tokens (`enth-ui::theme`), widgets, reference panel ("Lanes"), snapshot tests, `spike-ui-plugin` | S, A | Running |
| H | Minimal clack-host test host for CI + leak/CPU measurement | S | After 06-final |
| U2 | Panel in REAPER (Xvfb) + owner real-display checks; decide G0 UI thresholds | U1, H | Later |
| P | Agent-loop probe: an agent tries 5 text goals through `describe`/`check` over lane C's renderer | C | Optional, later |
| Z | Close: measured thresholds into the gates, ADR statuses, actual vs estimated effort | all | Last |

## Corrections to sub-reports

| Where | Said | Correction |
| --- | --- | --- |
| 07 | Carla 2.5.8 is a CLAP host | Ubuntu's Carla 2.5.8 hosts VST3 only; CLAP is on its unreleased main branch (01-final) |
| 02 (first) | iced is the better challenger | iced spins a core while idle and leaks a thread per open (02-final). No challenger |
| 03 (first) | fundsp builds with `default-features = false` | Only with `features = ["std"]`, on 0.23.0 and master (03-final). ADR-0006's fallback line needs that |
| 04 vs 03 | 04: bit-exactness only within one platform; 03: bit-exact across platforms | Both are right for what they measured. 03's renders used `libm`; 04's analysis used glibc maths and rustfft's CPU-dispatched paths. Renders: bit-exact. Scores: scalar FFT or tolerance (Decisions) |
| 05 (first) | `.gitignore` draft; job-level `hashFiles()` | Both broken; fixed in 05-final |
| 07 | Stepped ("notched") values at low difficulty, Syntorial-style | UI refs call notched values an anti-pattern, since the scorers handle continuous values. Open; decide in the R1 research session |
| Task 1 | `libjack-dev`; `cargo install wasmtime-cli` | `libjack-jackd2-dev` (the original removes packages); prebuilt wasmtime (`cargo install` fails on 1.95) |

## Owner checklist (when convenient; nothing waits on it)

Real display and hands. Save your work first: the NVIDIA X driver has crashed once already.

1. **Editor on the real display:** `cargo xtask install spike-shell --release` (later `spike-ui-plugin`), then in REAPER:
   - insert it 10× each as CLAP and VST3: ever blank?
   - resize it, and close/reopen it 20×;
   - leave it idle 60 s while watching `pidstat` / `nvidia-smi`.
2. **HiDPI and keyboard:** sharp at 100 % and 200 %? Does Space still reach REAPER's transport after you click the editor?
3. **Audio:** play through PipeWire/JACK (`pw-jack`); listen for clicks; check xruns.
4. **Listening sessions:** 2 × about 15 min, with lane C's `listen` command (instructions in `docs/spikes/metric.md` once merged).
5. **Optional:**
   - a Bitwig trial for poly-mod;
   - `sudo prime-select on-demand` plus a reboot, if the X driver crashes again (it moves the desktop to the Intel GPU).

Detailed commands: 01-final and 02-final (`OWNER-CHECKLIST.md` in 02's scratch), and `docs/spikes/shell.md`.

## Open checks

- Windows (and aarch64, if the private repo gets arm runners) golden hashes in CI (lane B).
- Scalar-FFT cost for cross-platform-identical scores (lanes B/C).
- Real-GPU glow vs wgpu, first-open reliability, keyboard focus, HiDPI in host (owner checklist).
- baseview leak (~70–160 KB per editor open), ≈30 fps cap, HiDPI clipping: report or fix upstream; the leak gate compares against a host-only baseline.
- Quiet-machine timings (every timing today ran at load 4–30).
- Effort: record actual vs estimated days per lane at close (Z).

## Docs to change

Applied in one pass after this summary, except files a running lane owns:
- lane A did ADR-0001, ADR-0004, naming.md and docs/spikes/shell.md;
- lane U1 does ADR-0002 and docs/spikes/ui.md;
- lane C does ADR-0007 and docs/spikes/metric.md;
- lane B does docs/spikes/engine.md.

| Doc | Change |
| --- | --- |
| `docs/phase-0.md` | Lanes replace "tasks in order". Task 1 reflects what's installed and the fixes above. Task 4 is egui/glow only, split into U1/H/U2, with the Xvfb rule and an owner-attended NVIDIA track. Task 5 compares hashes cross-platform and adds the FTZ/lanes/subnormal/RTSan tests. Task 6 gets the listening design, portfolio search, 5,000-evaluation budget and numeric reporting |
| `docs/roadmap.md` | Numeric G0 gates (metric: ρ ≥ 0.7, lower CI ≥ 0.4; search: ≥ 14/20 judged same within 60 s on 8 workers). Split P1 into **P1a headless core** and **P1b plugin + editor**. GR1 needs a friends' OS/DAW survey and a friend build. Second hosts: Carla (VST3), Bitwig trial |
| `docs/design.md` | **§4:** naming via CLAP rescan + patch; VST3 "Slot N". **§5:** bit-exact across platforms, the four DSP rules, voices end below −100 dB, scores via scalar FFT or tolerance. **§6:** EBU R128 loudness, categorical mismatch as its own component, beat-aware envelopes. **§7:** budget per evaluation, 4–5× scaling on 8 workers, 5,000 evaluations. **§8:** portfolio restarts, categorical cycling, λ ≥ workers. **§9:** target generation in parameter space only |
| `docs/benchmarks.md` | Two tiers (Xvfb automated / owner real-GPU). UI: wake-ups/s, first-open success, threads + PSS per cycle vs host baseline, ≤ 30 fps, in-host HiDPI, keyboard checks. Engine: instruction counts + subnormal test as the regression gate, an RTSan row, cross-platform hash row. Measurement hygiene: environment snapshot with every result |
| `AGENTS.md` | Never open GUI windows on the real display (Xvfb); every `#[allow]` needs a reason; no Mutex/RwLock in `enth-engine`; voices end below −100 dB; DSP from cited papers, never from memory of GPL synths; at most 2 heavy builds at once; one lane writes only its own `docs/spikes/<lane>.md`; agent scratch in persistent storage with a running `notes.md` |
| `docs/architecture.md` | Topology: `crates/enth-<name>/`, an `enth-golden` crate, `spikes/` and `tools/` members; the evaluator must be order-stable |
| `docs/adr/0006-effects-strategy.md` | fundsp fallback: `default-features = false, features = ["std"]` |
| `docs/research-process.md` | Scale sub-researchers to milestone risk; cap `summary.md` at about 2 pages plus a decision table; researchers work in persistent scratch with `notes.md`; Xvfb rule |
| `docs/README.md` | Developer environment: native REAPER (portable), Carla for VST3, Bitwig trial for poly-mod; the repo on ext4 |
| `docs/research.md` | §3: Carla without CLAP, clap-wrapper-rs lag, egui-baseview on Codeberg. §4: measured reactive repaint replaces "always redraws". §10: baseview #357, NVIDIA bug 6205955 |
| `Cargo.toml` | Lean dev profile (after 06-final) |
