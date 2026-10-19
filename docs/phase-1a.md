# Phase 1a — the headless core

**Goal: a deterministic synth you can drive from a terminal.** An agent (or you) builds a patch with `enth`, renders it, reads how it sounds, and gets measured hints, with no editor and no plugin. This is the headless half of Gate G1 ([roadmap.md](roadmap.md#p1--minimal-synth--cli-v0-46-weeks)); P1b adds the plugin shell and the editor.

Size: about **52–93 agent-hours** in total, with a critical path of about **18–35**. Your own time is about **1.5–2 h**: reading the research summary, two short listening sessions and an optional skim. In Phase 0 each lane took 0.8–2.8 agent-hours, starting from a working probe ([spikes/summary.md](spikes/summary.md)). P1a has probes for most items, but it adds migrations, integration and review, so the estimates are 2–4× that per lane. They are guesses until lane results come in; the closing summary records actual against estimated hours, as Phase 0 did.

**Status (2026-10-19): the research is done, lane S0 is in progress, the rest has not started.** Research session: `_local/sessions/2026-10-19-p1a-headless-core/` (`summary.md` for the decisions, reports 01–04 for the evidence and the numbers). The docs were updated to match on the same day: [design.md](design.md), [architecture.md](architecture.md), [agent-cli.md](agent-cli.md), [roadmap.md](roadmap.md), [ADR-0004](adr/0004-licensing.md), [ADR-0006](adr/0006-effects-strategy.md) and [AGENTS.md](../AGENTS.md).

Product crates start here, in `crates/`. The Phase 0 spikes stay in `spikes/` as the source of working prototypes to port from (`spike-engine`, `spike-metric`, `spike-agent`); they are exempt from the layer rules and are not shipped.

## What the research changed

Lanes should read these before they read the table, because each one changes what a lane builds.

- **Seeds never come from the patch.** A voice's seed is `derive(derive(render_seed, fnv1a64(module_id)), note_index)`. Seeding from the patch hash moved the loudness envelope by up to 6.6 dB for a 0.1 % cutoff move, against 0.008 dB with a fixed seed ([design.md §5](design.md#5-determinism)).
- **The processing quantum is a 32-frame segment behind a one-segment FIFO**, not "block 64". Renders are the same for any host block size: 1,200 of 1,200 prototype cases matched bit for bit ([design.md §3](design.md#3-engine-and-real-time-safety)).
- **The reverb needs no RNG.** Signalsmith's seeds are constants, so the port uses a constant table. The Rust port is bit-exact with the upstream C++ at `-O2` without FMA contraction ([ADR-0006](adr/0006-effects-strategy.md)).
- **The probe's 15 of 15 was partly a descriptor bug.** With fitted beat removal 13 of 15 trial patches pass, 3 of them within noise. Multi-parameter hints pass 5 of 5 goals in one hint step each ([design.md §6](design.md#6-challenge-and-scoring), [agent-cli.md](agent-cli.md#descriptors-enth-describe)).
- **Two more crates belong in P1a:** a minimal `enth-solver` (L5, the `hint` module), because hints need render and analysis and must not live in the CLI; and `enth-golden`. The agent-eval harness and the 10 text goals move from R1 into P1a, since G1 needs them.
- **Parameters are `const` tables** with a scale that is part of the persisted contract, a dense in-memory patch and a sparse-in, dense-out file layer, ops that return their inverses, and two hashes ([design.md §1–§2](design.md#1-one-parameter-schema-eight-consumers)).

## Freezes

The spine is serial and has three freeze points. Lanes fan out after each. **After F2, changes to spine types are additive only**: new fields get defaults, nothing is renamed or removed.

| Freeze | What is merged | Gate before the fan-out |
| --- | --- | --- |
| **F0** | S0: all P1a dependencies pinned, empty crates and files, the xtask and CI stubs | The Done-means gate is green ([AGENTS.md](../AGENTS.md#done-means)) |
| **F1** | S1a: the `ParamSpec` types and the patch *types*, with the **full P1a catalogue** | A judge agent (Opus) reviews the types before any lane builds on them. Optional: you skim the parameter list |
| **F2** | S2a: a walking skeleton. A reference saw goes through `Module`, `render`, a golden and CI on Linux, Windows and wasm | The saw has one hash on all three; invariance and real-time tests pass |

The coordinator alone edits `[workspace.dependencies]` and `Cargo.lock` after S0. A lane that needs a dependency asks for it.

## Lanes

Sizes are agent-hours (estimates). "Owner" is your time.

| Lane | Owns | Depends on | Size | Owner | State (2026-10-19) |
| --- | --- | --- | --- | --- | --- |
| **S0** Workspace prep (→ F0) | Root `Cargo.toml` with every P1a dependency pinned (list below); `Cargo.lock`; root `clippy.toml` (the numeric bans moved from `spike-engine`, plus the channel ban); empty crates `enth-cli`, `enth-solver` and `tests/golden` (`enth-golden`); pre-declared `mod` lines and empty files in `enth-dsp`, `enth-engine` and `enth-analysis`; one bench target per module, declared up front; `xtask golden` and bench commands; CI jobs | — | 1–3 | No | In progress |
| **S1a** Params and patch types (→ F1) | `enth-params`: `ParamSpec`, `Kind`, `Scale`, `Flags`, `Unit`, `Smoothing`, parse and display. `enth-patch`: the model types and a catalogue holding the `ParamSpec`s of **every** P1a module kind (oscillators, wavetable, noise, SVF, ADSR, LFO, reverb, compressor), the built-in `voice` and `master`, and macros | S0 | 3–5 | Optional skim, 15 min | Not started |
| **S1b** Ops, serde, migrations | `enth-patch`: `ops` with inverses (cascades included), `wire` (the ops JSON), `history` (undo), `file` (`PatchFile`), `validate`, `addr`, `canon` (canonicalise, exact and canonical hash), `schema` v1, the `migrate` framework and a v0 fixture | S1a (runs beside S2a) | 4–8 | No | Not started |
| **S2a** Engine walking skeleton (→ F2) | `enth-engine`: the four module roles, `PrepareCfg`, a minimal plan compiler, the free-running 32-frame grid with the FIFO adapter, seeds, a stub (silent) module per kind behind the exhaustive `ModuleImpl::new`, a reference saw. `enth-render`: `render(patch, performance, spec)`. The first golden cases; the `assert_no_alloc`, subnormal, FTZ and block-size tests; the gungraun scaffold | S1a | 4–8 | No | Not started |
| **S2b** Voices, modulation, plan swap | The voice manager (poly, mono, legato, glide, stealing with a fade, smoother reset on retrigger, the CLAP note tuple, voice end); the modulation matrix, macros and smoothing; the plan swap with state carried by module ID; the `rtrb` command and return rings | S2a | 6–10 | No | Not started |
| **D1** Oscillators and noise | `enth-dsp` oscillator, wavetable and noise files (Plaits' `VariableShapeOscillator` and stmlib's polyBLEP; wavetables generated in code); the engine generator modules; golden cases `osc-*` and `noise-*`; `docs/dsp/osc.md`, `noise.md` | F2 (the `enth-dsp` files need only F0) | 4–8 | No | Not started |
| **D2** Filter, envelopes, LFO | `enth-dsp` SVF, envelope and LFO files; the modules; goldens; docs | F2 (the `enth-dsp` files need only F0) | 3–6 | No | Not started |
| **FX1** Reverb | Phase 1 (`enth-dsp/reverb`, the port with its constant table, and a C++ reference harness kept in scratch); phase 2 (the module wrapper, the `THIRD_PARTY_NOTICES.md` entries, `docs/dsp/reverb.md`) | Phase 1: F0. Phase 2: F2 | 3–6 | **Reverb A/B, 20–30 min** | Not started |
| **FX2** Compressor | `enth-dsp` compressor (phase 1), then the module wrapper and docs (phase 2) | Phase 1: F0. Phase 2: F2 | 3–5 | Optional listen | Not started |
| **A** Analysis | `enth-analysis`: spike-metric's features, YIN and loudness; spike-agent's descriptors rebuilt with fitted beat removal, the multi-harmonic beating estimator, clipping and margins/noise; the expectation parser and checker; a measurement of the scalar-FFT cost | S0 | 6–10 | No | Not started |
| **C** CLI v0 | `enth-cli` (13 commands), `content/templates`, insta snapshots, JSON Schemas. **Phase 1** (needs only `enth-patch`): `schema`, `modules`, `params`, `new`, `apply`, `get`, `diff`, `guide`. **Phase 2** (needs render, analysis and hints): `render`, `describe`, `check`, `step`, `bench` | Phase 1: S1b. Phase 2: S2a, A, H | 6–10 | No | Not started |
| **H** Hints | `enth-solver::hint`: the finite-difference Jacobian, one multi-parameter least-squares fix that keeps passing items passing, an explicit `blocked` verdict | S2a, A | 4–6 | No | Not started |
| **T** Listening tools | The ABX tool (the G0 search gate) and a blind, level-matched A/B (the reverb), reusing spike-metric's `listen` design; they write WAVs and answer files | A (loudness) | 2–3 | You use them | Not started |
| **E** Evals and G1 close | `content/agent-evals` (10 goals with their lists, prompts), `cargo xtask agent-eval`, the WAVs for the blind listen, the 16-voice budget case, the headless G1 table | Everything | 3–5, plus API spend | **Blind listen, 15–20 min** | Not started |

**Total: about 52–93 agent-hours.** The dependencies S0 pins, from the research (2026-10-19; S0 checks them against `Cargo.lock`): `libm =0.2.16`, `rtrb =0.4.0` (no default features), `assert_no_alloc =1.1.2`, `sha2 =0.11.1`, `serde_json` with `float_roundtrip`, `proptest 1.12.0`, `insta 1.49.0` (`json`, `filters`, `redactions`) with `insta-cmd 0.7.0`, `clap 4.6.7`, `schemars 1.2.3`, `strsim 0.11.1`, `hound 3.5.1` (CLI only), `rustfft`/`realfft`, `ebur128` and `gungraun`. Not added: `jsonschema` (its defaults pull in a C build), `triple_buffer` (MPL-2.0), `basedrop`, `arc-swap`, `cargo-fuzz` (below).

## Waves

| Wave | Lanes | Starts when |
| --- | --- | --- |
| 1 | S0 | — |
| 2 | S1a, A, and the reverb and compressor DSP cores (FX1 and FX2 phase 1, `enth-dsp` only) | F0 |
| 3 | S1b, S2a | F1 |
| 4 | S2b, D1, D2, the effect module wrappers (FX1 and FX2 phase 2), C phase 1 | F2 |
| 5 | H, C phase 2, T | S2a and A merged |
| 6 | E, then the owner's gates | Everything else merged |

**Critical path:** S0 (1–3) → S1a (3–5) → S2a (4–8) → D2 or FX1 (3–8) → H with C phase 2 (4–6) → E (3–5), about **18–35 agent-hours**, plus the owner's gates.

## Acceptance per lane

Each lane stops at its own criteria and reports against them, as in Phase 0.

| Lane | Acceptance |
| --- | --- |
| **S0** | The Done-means gate is green. `cargo xtask golden` passes one trivial case on Linux, in Windows CI and on wasm. `cargo deny` passes with the new dependencies. A probe using `std::sync::mpsc` in `enth-engine` fails clippy |
| **S1a** | `parse(display(x)) == x` for every `ParamSpec` and accepted unit as a property, and plain ⇄ normalised round trips where the scale is invertible (`+1oct` on a 20 Hz–20 kHz Log parameter is 10.034 %). The catalogue is checked at compile time: a default out of range is a compile error. An insta snapshot of the whole catalogue. Reviewed by a judge agent before the fan-out |
| **S1b** | The property suite: apply then inverse gives back the original with exact structural equality (including `RemoveModule` with routes, slots and macros); transactions are atomic; the JSON round trip is exact for every finite f32; undo/redo with merges matches a snapshot model. The negative controls fail as they should (dropping route restore from `RemoveModule`'s inverse fails 3 properties; merging across addresses fails the history property). Canonicalise is idempotent and its hash is unchanged by permuting interchangeable modules. Fixtures for every schema version load; `migrate` is the identity on the current version; a future version gives `TooNew`; non-finite numbers give a typed error. Fuzz-lite: arbitrary and mutated JSON never panics and returns a typed error with a hint. `cargo mutants --in-diff` survivors reviewed |
| **S2a** | The saw patch has one hash on Linux, Windows and wasm in CI. Host blocks of {1, 7, 63, 64, 65, 512, 4096} give the same hash. `assert_no_alloc` passes. A missing implementation is a compile error. Seed stability: moving any continuous parameter by 1e-3 relative changes the 10 ms RMS envelope by under 0.05 dB. A 40 s release tail has zero subnormals and the same hash with FTZ off, FTZ only and FTZ+DAZ. Every output sample is finite |
| **S2b** | Goldens per voice mode. Overflow tests under `assert_no_alloc`: a full command ring, every voice stolen, a plan swap while the return ring is full (the plan is kept, not dropped). A held note survives an unrelated structural edit. For every `ModuleKind`, copying state from plan A into plan B and running both gives bit-identical output. The same performance gives the same slot assignment. Stale-epoch events are ignored. A 40 s tail ends with zero active voices. The 16-voice reference patch has a gungraun case with a provisional budget |
| **D1** | Goldens at 55, 440 and 3,520 Hz for saw, pulse 50 %, pulse 20 %, triangle and sync ×2.37. Alias ceilings at 440 Hz: saw ≤ −38 dB, triangle ≤ −80 dB, wavetable ≤ −85 dB (also at 3,520 Hz). A 10 s zero-crossing frequency within 1e-4. The table data hashes the same on native and wasm. Noise: distinct streams per note, a pink slope of −10 ± 0.3 dB/decade over 100 Hz–10 kHz. An instruction budget per module. Docs cite their sources; the Plaits and stmlib notices are in `THIRD_PARTY_NOTICES.md` |
| **D2** | Impulse-response hashes for 4 modes × 3 cutoffs × 2 Q values, and \|H\| at DC, cutoff and Nyquist within 0.1 dB of the analytic value. The SVF stays stable under random cutoff jumps between 20 Hz and 21.6 kHz at Q 25: finite output and no stuck DC offset. Envelope segments reach their end within one control step (0.67 ms), a retrigger is continuous, and voices end below −100 dB. LFO rate within 1e-5 over 60 s; sample-and-hold depends on the seed and not on the patch hash |
| **FX1** | Bit-exact with the upstream C++ built at `-O2` without FMA contraction (baseline x86-64 or `-ffp-contract=off`, no fast-math), including a parameter change mid-render, and the Rust hash is identical on every platform. The constant table is regenerated by a test. A 61 s tail has zero subnormals. `assert_no_alloc` around `process`. Zero latency; builds for wasm32. `room` and `decay` are not modulatable and the smoothing is `None`. Level-matched A/B renders against fundsp's `reverb2`/`reverb4` are kept in scratch for the owner gate |
| **FX2** | A static-curve table; agreement with web-audio-api-rs's gain formula within 1e-5 dB (it is a test oracle, not ported); a steady sine; step-response 63 % times; no NaN at silence; zero latency; the cascaded-smoother timing is documented. Within 250 instructions per frame |
| **A** | The descriptor audit tables in [spikes/agent-probe.md](spikes/agent-probe.md) become regression tests (sweeps against theory), and the detuned cases read correctly. The invariance, accuracy, false-positive and tremolo suites are kept as `#[ignore]` sweeps with their results recorded (the prototype: 64 % of 300 patches agree with their detune-0 twin). The sweeps are rerun on the real engine once S2a lands, because the prototype used a toy synth. The scalar-FFT cost is recorded |
| **C** | In-process snapshots per command and per error class, through `enth_cli::run(args, stdin) -> Output { code, stdout, stderr }`. A handful of end-to-end tests that spawn the binary, for TTY detection and exit codes. Each output deserializes back into its Rust type and its schema is snapshotted. `enth guide` fits in about 6 KB. `enth bench` reports wall-clock as information. `render` writes the float WAV with the `enth` chunk, and `describe` reads it back |
| **H** | Conflicting goals like the probe's g5 get a fix that does not trade one failure for another. A follower passes the five probe goals in one hint step each. An infeasible list gets a `blocked` verdict. Identical results for any worker count. Probes use the goal's performance for envelope and beating items |
| **T** | An agent dry run. One page of instructions for you. Answer files go under `_local/`, which is never committed |
| **E** | The protocol in [agent-cli.md](agent-cli.md#evaluating-the-cli-like-a-product): 10 goals, 3 runs each, both tracks; at least 5 of 10 goals pass within 20 renders. The 16-voice budget case is in CI. The headless G1 table below is filled in. The first real run's cost is recorded against the $9 estimate |

## Owner gates

| Gate | What you do | Time | When | Tool |
| --- | --- | --- | --- | --- |
| Parameter list (optional) | Skim `enth params` for every module: names, units, ranges, defaults | 15 min | After F1 | S1a |
| Reverb A/B | A blind, level-matched A/B of the ported reverb against fundsp's, then the pick | 20–30 min | As soon as the FX1 wrapper lands | T |
| API spend | Approve the first `agent-eval` run (estimated $9 for 10 goals × 3 runs × 2 tracks; the run measures it) | — | Before E | E |
| G1 blind listen | Judge whether each of the 10 final sounds matches its text, without seeing the numbers; at least 7 of 10 | 15–20 min | At the end | E |

Two decisions default without you, and you can change them: `cargo-fuzz` stays off (it needs nightly, a C++ toolchain and the NCSA licence, which `deny.toml` lacks; proptest "fuzz-lite" inside `cargo test` covers most of it), and macOS goldens run on demand or weekly.

## The headless G1 checks

| Check | How | Who | When |
| --- | --- | --- | --- |
| 16 voices: at least 10× realtime offline, and at most 12,000 instructions per frame | A gungraun case in CI (hard limit re-based to the first product measurement + 10 %) | Agents (S2b, E) | Provisional from S2a, final in E |
| p99 `process()` under 25 % of the 64-sample deadline | The test host | P1b | — |
| Patches round-trip through JSON | Properties, fixtures, fuzz-lite | S1b | Continuously |
| Goldens on Linux, Windows and wasm32, reverb and compressor included | CI on every push. macOS: on demand or weekly (about 10× the minutes on a private repo), and once before G1 | Agents; you approve macOS minutes if the quota matters | Before G1 |
| The reverb chosen by a blind, level-matched A/B | T and FX1 | **You**, 20–30 min | As soon as FX1 lands |
| Agent eval: at least 5 of 10 goals, and a blind listen with at least 7 of 10 | E | Agents and **you**, 15–20 min | At the end |
| "A bass, pad, pluck and lead you like" | Through the editor | You | P1b: it needs the editor |

## Rules for every lane

- Branch from the latest freeze, and write only the files your lane owns (the table above). Per module, write `docs/dsp/<module>.md` (sources, licences, deviations from upstream), and add your own upstream entries to `THIRD_PARTY_NOTICES.md` (append-only, one section per upstream). Report against your acceptance criteria when you stop.
- **Concurrency.** P1a lanes are light: none pulls nice-plug, baseview, egui or clack-host, so AGENTS.md's limit of 2 heavy builds binds only if P1b work starts alongside. At most **4 lanes build at once**, with `CARGO_BUILD_JOBS=3` each and a separate `CARGO_TARGET_DIR` per worktree on ext4; wrap gungraun and any timing run in `flock -x`. One coordinator merges in wave order and runs the full gate after every merge.
- **Merge hotspots are defused up front.**

  | Hotspot | Who edits it |
  | --- | --- |
  | `[workspace.dependencies]`, `Cargo.lock` | S0, then the coordinator, one commit per addition |
  | Catalogue registration in `enth-patch` | S1a declares every P1a kind; a DSP lane touches only its own module's spec file, and only if ranges change |
  | The engine's `match ModuleKind` and `mod` lines | S2a, with stubs; each lane replaces its own stub's file |
  | `mod` lines in `enth-dsp` and `enth-analysis`; `[[bench]]` entries in `enth-engine` | S0 declares them and creates the empty files |
  | `tests/golden/cases/*.toml` | One file per case. Composite cases (the 16-voice patch, a phrase with steals, the 40 s tail) belong to the spine lanes |
  | `xtask/src/main.rs`, CI workflows | S0 stubs every P1a command and job |
  | `enth-cli` | Lane C only |

- **Golden cases are small and per module** (`osc-saw-1v`, `svf-lp-res-sweep`, `reverb-impulse`, `comp-step`), so a broken hash names the module. A mismatch reports every failing case in one run with a summary diff (RMS per 10 ms, centroid), never the first only. `--update` is refused under CI and prints the cases it changed for the commit message. Never assert `render(p) == render(canonicalise(p))` bit for bit: assert that the canonical hash is stable and that descriptors stay close.
- **The test loop, fastest signal first:** the crate's unit tests; fixed-seed proptest (with `proptest-regressions/` committed); goldens with summary diffs; in-process insta snapshots; then the `assert_no_alloc`, subnormal and block-size invariants. A lane's inner loop is `cargo nextest run -p <crate>` (about 0.5–2 s after an edit). CI only: gungraun budgets, Windows and wasm goldens on every push, macOS goldens on demand or weekly, nightly RTSan, nightly proptest exploration (random seeds, 10,000 cases). No wall-clock assertions, no random seeds in the gate, no test that depends on another lane's goldens; a flaky test is a bug.
- **Fuzzing.** Proptest "fuzz-lite" on the same entry points (`Patch::from_json`, `migrate`, `validate`, `apply`, the unit parser, `render` under `assert_no_alloc`). Real `cargo-fuzz` only if you ask for it.
- Risks (report 04 lists the top eight with their cheapest retirement): API churn after the fan-out (the freezes and the judge review), patch-hash seeding (the seed-stability test), host block size leaking into renders (the invariance property), op and JSON correctness (the S1b suite), real-time and NaN regressions on rare paths (the overflow tests), golden churn (per-module cases and summary diffs), G1's validity and your bandwidth (the descriptor audit as regression tests; announcing the two listening sessions early), and merge hotspots (the table above).

## Open checks

Not settled by the research; each is owned by a lane or by P1b.

- Plan-compile time and crossfade cost (P1b, in the test host). During the 256-frame crossfade the CPU doubles, so the steady state must stay under about 12 % of the deadline to keep p99 under 25 %.
- Scalar-FFT cost for cross-platform-identical scores (lane A records it).
- Very slow beats (5 ct below about 250 Hz) are flagged with `beat_corrected: false`, not corrected.
- Hints that choose among waveforms or filter modes: designed, not built.
- The voice bus: a mono bus with a per-voice pan is cheaper (a stereo voice path doubles filter cost), but unison spread needs a stereo bus. S2a decides before F2.
- The cost of the first real agent-eval run (estimated at $9).
