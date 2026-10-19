# P1a research — summary

- **Date:** 2026-10-19
- **Reports:**
  - `01-engine-architecture.md`
  - `02-dsp-modules.md`
  - `03-cli-descriptors.md`
  - `04-risks-tests-plan.md`
- **Status:** the owner reads this at their own pace. Work proceeds on the assumption they agree.

## Verdict

**P1a is buildable as planned, with five design corrections, and most of its risky parts already have working prototypes.**

1. **Never seed randomness from the patch.** design.md §5 seeded voices and effects from the patch hash, so any knob move reseeds every voice. A 0.1 % cutoff move shifted the loudness envelope by up to 6.6 dB, against 0.008 dB with a fixed seed. That would wreck finite-difference hints and search. Fix: `seed = derive(derive(render_seed, fnv1a64(module_id)), note_index)`. Reports 01, 02 and 04 found this independently.
2. **The processing quantum is a 32-frame segment, not "block 64".** A one-segment FIFO adapter makes renders independent of host block size: 1,200 of 1,200 cases matched bit for bit across host blocks of 1 to 4,096.
3. **The reverb needs no RNG.** Signalsmith's seeds are constants, so the port uses a constant table. The Rust port is already **bit-exact with the upstream C++** (`-O2`, no FMA), including a mid-render parameter change, and identical on native and wasm.
4. **The probe's 15/15 was partly a descriptor bug.** With fitted beat removal, 13 of 15 trial patches pass, 3 of them within noise. Multi-parameter hints let a mechanical follower pass 5/5 goals in one hint step each. The fixes are prototyped and measured.
5. **Two more crates belong in P1a:** a minimal `enth-solver` (L5), because hints need render + analysis and must not live in the CLI, and `enth-golden`. The agent-eval harness and the 10 text goals move from R1 into P1a, since G1 needs them.

## Decisions

| Area | Decision | Evidence |
| --- | --- | --- |
| Parameters | `const` tables in plain Rust plus a small `macro_rules!` emitting the index enum and table together; compile-time validation via `const fn`. `ParamSpec.kind: Float{min,max,scale} \| Int \| Enum \| Toggle`, `Scale: Linear \| Log \| Power(k)`, flags `{modulatable, automatable, structural, periodic}`. **The scale is part of the persisted contract** (route depth, ML, randomiser), so changing it needs a migration; scale maths through pinned `libm`. An insta snapshot of the whole catalogue | 01 (measured compile-time check) |
| Patch v1 | File layer (`PatchFile`: sparse in, dense out, enums as names, `float_roundtrip`, reject non-finite, canonicalise −0.0, params parsed against their `ParamSpec` for typed errors) vs in-memory `Patch` (a dense `Vec<f32>` per module). Sections `generators`, `voice_fx`, `fx`, `modulators`; built-ins `voice` and `master` (so `master.level`, `voice.glide` are addresses); a per-oscillator phase mode (free / reset / seeded); route depth = % of range, normalised domain, polarity on the source | 01, 03, 04 |
| Ops | `apply -> Result<Vec<Op>, OpError>` returns the inverse; transactions are atomic; `AddModule` carries the full instance; knob drags merge only edits to the **same address**. Property suite: apply∘inverse = id, atomicity, JSON round trip, history vs model | 01 (4 properties × 2,000 cases pass; negative controls fail) |
| Migrations | Pure `serde_json::Value → Value` steps, one per version, with fixtures; no versioning crate | 01 |
| Hashes | `exact_hash` for render caches and goldens; `canonical_hash` (symmetry-free) for dedup and structure scores. Canonical-equal patches are **not** promised bit-identical renders | 01 |
| Engine | Enum dispatch (`ModuleImpl`) by role; an exhaustive `match ModuleKind` makes a missing implementation a compile error. `prepare` = constructor from `PrepareCfg`. Fixed segment order: commands → notes → smoothers → global mods → voice mods → params → voices → FX | 01 |
| Plan swap | One `rtrb` command ring (plans and value events in order) plus a return ring with a pending slot: nothing freed on the audio thread. **Carry state for modules whose id and kind survive**; crossfade only added or removed modules. `triple_buffer` (MPL), `basedrop`, `arc-swap` rejected | 01, 04 |
| Voices | Lowest free of 32 slots (polyphony ≤ 16); steal oldest released, then oldest held, with a 64-frame fade; 16-key stack for mono/legato; constant-time linear glide; end below −100 dB; CLAP (port, channel, key, note_id) addressing | 01 |
| Real-time | No `std::sync::mpsc` or unbounded channels in `enth-engine` (clippy ban); every ring/queue gets an overflow test under `assert_no_alloc`; engine tests under nextest; nightly RTSan | 04 |
| Oscillators | Port Plaits' `VariableShapeOscillator` (MIT, © Emilie Gillet): saw/pulse+PWM/triangle/hard sync with polyBLEP/BLAMP. Wavetables generated in code (2048 × 11 mips, f64 additive, 4-point Hermite; ≤ −93 dB aliasing) | 02 (measured) |
| Filter | Cytomic/Simper TPT SVF (LP/BP/HP/notch); `g = libm::tanf` every 32 frames with per-sample interpolation of `a1..a3` (13–23 dB closer than stepping); stable under random 20 Hz–21.6 kHz jumps at Q 25 | 02 |
| Envelopes, LFO, noise | RC-style exponential segments with overshoot targets (exact segment times); u32-phase LFOs; xorshift32 white + JOS 3rd-order pink, seeded per voice | 02 |
| Reverb | Port Signalsmith Basics `reverb.h` (basics `369e906e…`, dsp `ca390d28…`) with `delay.h`, `mix.h`, `filters.h` and the fade logic of `stfx-library.h`. Constant table instead of mt19937. 1.64 MB in `prepare`; a 20 ms crossfade on room changes; 0 subnormals; ~1.6 % of a core. Fix upstream's `reset()` missing `delay4` in our port | 02 (bit-exact vs C++) |
| Licence | **Accept BSL-1.0 for ported source** (`stfx-library.h` still carries a Boost header). It's permissive and OSI-approved, and deny.toml already allows it as a scoped exception. ADR-0004 gets a dated note | 02; owner delegated licence calls |
| Compressor | JOS `compression_gain_mono` structure + Giannoulis 2012 soft knee + make-up gain, stereo link by `max(|L|,|R|)`, no look-ahead; cross-checked against web-audio-api-rs (within 1e-6 dB). ADR-0006's wording ("follows JOS … soft knee, make-up") is corrected | 02 |
| FX bypass | Instance-level monitor mask on effects flagged `time_based`, in plugin state; `enth-render` can't set it, so scoring and the CLI ignore it by construction | 02 |
| CPU budget (G1) | 16-voice reference patch + compressor + reverb: **≥ 10× realtime** offline on one core; **≤ 12,000 instructions per frame** (gungraun hard limit, re-based to the first product measurement + 10 %); p99 `process()` ≤ 25 % of the 64-sample deadline (P1b). The prototype measured 6,353 instructions per frame, 22.7× realtime | 02 (corrects lane Z's 340 M per 2 s; see Corrections) |
| Descriptors | Beat removal by a fitted periodic pattern at the measured beat rate (not crest or boxcar smoothing); a multi-harmonic beating estimator that tells tremolo from detune; `peak_dbfs`, `true_peak_dbtp`, `clipped_pct` on the float render; a signed `margin` + `noise` from 2 nuisance re-renders, `thin` when \|margin\| ≤ 2 × noise; a per-render nuisance seed separate from the patch | 03 (measured on the trial's failures) |
| Hints | `enth-solver::hint`: bounded Gauss-Newton on a squared hinge over all failing items, keeping passing ones; finite-difference Jacobian verified by real renders (Broyden), sparsified; explicit `blocked` verdicts. 0.26–1.9 s per hint | 03 |
| CLI v0 | 13 commands with defined shapes; clap 4.6.7, schemars 1.2.3, insta 1.49 + insta-cmd 0.7, strsim, hound 3.5.1, std `IsTerminal`; own unit parser; **no `jsonschema` crate** (C build via aws-lc-rs); typed round-trip tests so schemas can't drift. `render` writes a float WAV with an `enth` chunk holding the performance | 03 |
| Agent evals | `content/agent-evals/goals/*.toml` + `cargo xtask agent-eval` driving headless Claude Code (`--bare -p`, tools limited to a budget-counting `enth` wrapper), both tracks, 3 runs per goal, model and cost recorded. About $9 per full run (estimate). Never a merge gate | 03 |
| Tests | Fast signal in this order: crate unit tests → fixed-seed proptest (with `proptest-regressions/` committed) → goldens with summary diffs → in-process insta snapshots → `assert_no_alloc`/subnormal/block-size invariants. CI only: gungraun budgets, Windows + wasm goldens per push, **macOS on demand or weekly** (≈ 10× minutes on a private repo), nightly RTSan and proptest exploration | 04 |
| Fuzzing | Default: proptest "fuzz-lite" inside `cargo test`. cargo-fuzz (nightly, C++, NCSA licence) stays off unless the owner wants it | 04 |
| G1 wording | "a bass, pad, pluck and lead you like" moves to P1b (it needs the editor) | 04 |

## P1a lane plan

From report 04, adjusted.
- **Serial spine with three freezes:**
  - **S0** workspace prep (all P1a deps pinned, empty crates and files, xtask/CI stubs) → **F0**;
  - **S1a** params + patch types with the **full P1a catalogue** → **F1** (reviewed before fan-out);
  - **S2a** walking skeleton (module trait, segment grid, seeds, stub modules, reference saw, goldens on Linux, Windows and wasm, invariance tests) → **F2**.
- **Waves:**
  1. S0;
  2. S1a + A (analysis) + the reverb and compressor DSP cores (enth-dsp only);
  3. S1b (ops/serde/migrations) + S2a;
  4. S2b (voices, modulation, plan swap) + D1 (osc/noise) + D2 (filter/env/LFO) + FX module wrappers + C phase 1;
  5. H (hints) + C phase 2 + T (listening tools: ABX and reverb A/B);
  6. E (evals, G1 table).
- **After F2, spine types change additively only.** At most 4 light lanes build at once (`CARGO_BUILD_JOBS=3` each). The coordinator alone edits `[workspace.dependencies]` and `Cargo.lock` after S0.
- **Estimate:** about 52–93 agent-hours in total, with a critical path of about 18–35.
- **Owner time:** about 1.5–2 h (reverb A/B 20–30 min, G1 blind listen 15–20 min, optional catalogue skim).

## Corrections to sub-reports and earlier docs

| Where | Said | Correction |
| --- | --- | --- |
| design.md §5, ADR-0006 | Seed voices and effects from the patch hash | Seed from (render seed, module id, note index); never from parameter values (01, 02, 04) |
| brief.md, design.md §5, AGENTS.md | Re-chunk onto a fixed internal block of 64 | A 32-frame segment behind a FIFO; renders may call `process` with any chunk size (01) |
| Lane Z (roadmap G1) | 16-voice budget 340 M instructions per 2 s render (≈ 3,540 per frame) | Too tight for the real P1 module set: 02 measured 6,353 per frame with compressor + reverb. Use 02's ≤ 12,000 instructions per frame and ≥ 10× realtime |
| ADR-0006 | Compressor "follows JOS `compressor_mono`" with soft knee and make-up | JOS has neither; the knee is Giannoulis 2012, the link is max, both ours (02) |
| Effects research summary | DaisySP's SVF is TPT | It's a double-sampled Chamberlin SVF; stmlib and fundsp are TPT (02) |
| design.md §6 | "Crest envelope" for beat-aware envelopes | Fitted beat removal; crest/boxcar reach only 23 % invariance and move attack edges (03) |
| design.md §7 | Hints use 0.5 s clips | Only for spectral items; envelope and beating items need the goal's performance (03) |
| roadmap R1 | `content/agent-evals` and `xtask agent-eval` | Text-goal track moves to P1a (03) |
| docs/spikes/agent-probe.md | 15/15 goals passed | 13/15 under corrected descriptors, 3 within noise (03) |
| All four reports | — | Early crates.io requests by all four researchers carried the owner's email in the User-Agent (about 25 requests in total) before a rule was issued. Since then AGENTS.md and research-process.md carry a Privacy rule, and every agent prompt states it |

## Docs to change

| Doc | Change |
| --- | --- |
| `docs/design.md` | §1 ParamSpec kinds, scales and flags, with the scale as a persisted contract. §2 Patch v1 sections, file vs memory forms, ops returning inverses, two hashes, phase mode. §3 32-frame segment, enum-dispatch roles, segment order, plan swap with state carry. §5 seeding rule; segment and FIFO. §6 fitted beat removal, clipping fields, margins/noise/thin. §7 performance-length probes; hint solver |
| `docs/architecture.md` | Add `enth-solver` (minimal, P1a) and `enth-golden`; enth-patch files (`file`, `validate`, `addr`, `wire`, `history`); units in `enth-params`; registry check by exhaustive match; P1 compiles plans on the main thread |
| `docs/agent-cli.md` | Op additions (`at`, `move`, `unroute`, `enable`, `macro`, `slot`, `amp_env`), `--set` sugar, `check` output fields (`fix`, `blocked`, `noise`, `thin`), float WAV + `enth` chunk, descriptor fields, eval harness command |
| `docs/roadmap.md` | P1a scope includes `enth-solver` (hints), `enth-golden`, the text-goal eval harness. G1: CPU budget numbers above; macOS goldens on demand/weekly; "bass/pad/pluck/lead you like" → P1b. A P1a lane plan section |
| `docs/adr/0006-effects-strategy.md` | Constant-table reverb (no RNG); compressor provenance; bit-exact C++ golden condition (no FMA contraction) |
| `docs/adr/0004-licensing.md` | Dated note: BSL-1.0 allowed for ported source |
| `AGENTS.md` | Seeds never depend on parameter values; channel ban; overflow tests; nextest + fixed proptest seed; host automation applies `SetParam` without a history entry |
| `docs/phase-1a.md` (new) | The lane table, freezes, waves, acceptance per lane (from report 04), adjusted as above |

## Open checks

- Plan-compile time and crossfade cost (P1b, test host).
- Scalar-FFT cost for cross-platform-identical scores (lane A records it).
- Very slow beats (5 ct below about 250 Hz) are flagged, not corrected (03).
- Hints choosing among waveforms or filter modes: designed, not built (03).
- The first real agent-eval run's cost (estimated at $9).
