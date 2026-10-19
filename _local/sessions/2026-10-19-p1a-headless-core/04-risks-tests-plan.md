# 04 — Risks, prior art, tests and the P1a lane plan

- **Date:** 2026-10-19
- **Sub-researcher:** 04 (risks, prior art, test strategy, P1a lane plan), P1a research session.
- **Questions answered:** all five in the brief:
  1. prior art in Rust synth and audio engines;
  2. real-time Rust pitfalls and the tools that catch them;
  3. the P1a test strategy, and which tests give agents the fastest reliable signal;
  4. a P1a lane plan;
  5. the top 8 P1a risks, each with its cheapest retirement.
- **Method:**
  - I read the brief, AGENTS.md, docs/README.md, roadmap.md, design.md, architecture.md, agent-cli.md and everything in `docs/spikes/`. That includes `summary.md`, `environment.md` and `owner-checklist.md`, committed during this session at `2d1d2f4`, plus the matching edits to design.md, agent-cli.md, AGENTS.md and roadmap.md. I also read the Phase 0 research summary, report 07 and the relevant parts of 03, 05 and 06.
  - Web: WebSearch and WebFetch, `gh api`, the crates.io API and curl.
  - Local, read-only: `cargo generate-lockfile` plus `cargo tree` in a scratch workspace to count dependencies, sources in the local cargo registry, `free`, `rustup toolchain list` and the installed cargo tools.
  - **No builds.** The brief said none were needed, and sub-researchers 02 and 03 were building at the time. So compile costs are dependency counts plus Phase 0's measured analogues, marked *estimated*.
  - Late in the session I read sub-researchers 01 and 02's scratch `notes.md` to cross-check. Where I use them they are cited as "01's notes (unpublished)", never as my own measurement.
- **Labels:** *measured* (local output, or numbers read from an API), *documented* (docs, source code, READMEs, issue text), *anecdotal* (forums, blogs, user reports), *estimated* (my arithmetic from measured analogues), *inference*.
- **Scratch:** `~/code/enth-scratch/2026-10-19-p1a/04/` (`notes.md`, `deptree/`).

## Answer in brief

- **Prior art adds three rules to the docs:**
  - **Carry module state across plan swaps by stable id**, rather than always crossfading into a fresh plan. fundsp's `Net` and Glicol's LCS graph diff both keep state for nodes that survive an edit.
  - **Never seed randomness from structure or patch content.** fundsp documents that adding a node "modifies its sound subtly".
  - **Expect allocation to sneak in through rare paths** (fundsp #54, Firewheel #142), not the happy path.
- **design.md §5 seeds voices and effects "from the patch hash". For P1a that is a bug.** Every knob move reseeds random phases and detune, so finite-difference hints, `suggest` and CMA-ES see noise. Seed from (render seed, stable module id, note index) instead. 01's notes measure the effect: a 0.1 % cutoff move changes the RMS envelope by up to 6.6 dB with patch-hash seeding, against 0.008 dB with a fixed render seed.
- **The infrastructure-first stall shows again.** Meadowlark's GitHub org holds 25 repos, almost all archived on 2025-09-22, among them its own GUI library, audio I/O and engine. P1a should get one module through render, golden and CI before fanning out.
- **Real-time pitfalls that agent-written code will hit:**
  - overflow paths that `push` (Firewheel #142);
  - `Vec::clone` dropping its reserved capacity (fundsp #54);
  - `std::sync::mpsc` and unbounded crossbeam channels, which allocate a block every 31 messages;
  - the last `Arc` dropped on the audio thread;
  - panics and formatting;
  - NaN from `log(0)` or from filters that go unstable under modulation (Actuate #17).

  `assert_no_alloc` catches allocations *and* frees in debug tests, and RTSan (nightly) adds syscalls and locks. Neither catches an uncontended `Mutex`, so the clippy ban stays.
- **JSON traps for "patches round-trip":**
  - serde turns a JSON number into an `f32` with `f64 as f32`, so `1e39` loads as `inf`;
  - serde_json parses floats with "best-effort precision" by default (the `float_roundtrip` feature fixes that at about twice the parse cost);
  - NaN and infinity serialise as `null`;
  - an untagged `ParamValue` produces "did not match any variant" errors that agents can't act on.
- **Fastest reliable signal for agents, in order:**
  1. the crate's unit tests;
  2. fixed-seed proptest properties on params, patch and ops (thousands of cases in about 0.1 s);
  3. golden hashes with summary diffs;
  4. in-process CLI snapshots (insta, 0.06 s per test);
  5. `assert_no_alloc`, subnormal and block-size invariance tests.

  Then CI-only: gungraun instruction budgets, Windows and wasm goldens, nightly RTSan and proptest exploration, optional fuzzing. Agent evals never gate.
- **cargo-fuzz needs nightly and a C++ compiler.** Its `libfuzzer-sys` is licensed `(MIT OR Apache-2.0) AND NCSA`, and NCSA is not in `deny.toml`, so per AGENTS.md it is the owner's call. Stable proptest "fuzz-lite" on the same entry points gives most of the value inside `cargo test`.
- **The brief's crate list is missing the crate that owns `check` hints.** Hints need render and analysis, which are both L3, so they can't live in either of those crates or in the CLI. P1a needs a minimal `enth-solver` (L5, `sensitivity` only), plus the `enth-golden` harness crate.
- **Lane plan:**
  - a serial spine of three short freeze points: workspace prep, then params and patch types, then a walking skeleton of `Module` trait, render and goldens;
  - then up to four light lanes at a time: oscillators, filter/envelope/LFO, reverb, compressor, analysis, voice and modulation, CLI, hints.

  The critical path is about **18–35 agent-hours**, the total about **52–93**, and the owner's share about **1.5–2 h**: read the summary, a reverb A/B and the G1 blind listen.
- **The top 8 risks:** spine API churn, patch-hash seeding, host block size leaking into renders, op and JSON correctness, real-time and NaN regressions on rare paths, golden churn and rubber-stamped updates, G1 validity and owner bandwidth, and merge hotspots plus machine contention. Each has a retirement that costs under a day, and most of them are tests written in the spine lanes.
- **G1's "goldens on three OSes":** GitHub bills macOS minutes at about 10× Linux on a private repo. Run macOS goldens on demand or weekly, not on every push.

## Findings

### Q1 — Prior art: engine shape, what slowed it, what to copy

Activity figures are from `gh api`, the Codeberg API and the crates.io API on 2026-10-19 (*measured*); the architecture notes are from READMEs, design docs and source (*documented*).

| Project | Licence and activity | Engine architecture | What slowed or broke it | Copy / avoid in P1a |
| --- | --- | --- | --- | --- |
| [fundsp](https://github.com/SamiPerttu/fundsp) | MIT OR Apache-2.0. 0.23.0 released 2026-01-07; repo last pushed 2026-03-03; 1,226 stars | Two styles: static generic graphs, where "Mismatched connectivity will result in a compilation error", and a dynamic `Net` split into frontend and backend. Edits take effect on `commit()`; `crossfade(id, Fade::Smooth, …)` replaces a node "to avoid clicks"; `allocate()` "preallocates all needed memory". Buffers are always 64 samples. Parameters are `shared()` atomics, smoothed with `follow()` (README) | [#54](https://github.com/SamiPerttu/fundsp/issues/54) (2024-09): containers cloned for the backend lost their reserved capacity, so the sequencer allocated whenever an event was scheduled. The no_std build is broken again on master (03-final) | **Copy:** frontend/backend with commit, and moving state by node id into the new net (01's notes describe `Net::migrate`). **Avoid:** the README says "Generator phases are seeded from network structure and node location", and adding a node changes the others' sound |
| [Firewheel](https://codeberg.org/BillyDM/Firewheel) | MIT OR Apache-2.0. `firewheel` 0.14.0 released 2026-09-05; GitHub mirror has 316 stars | Any DAG, compiled into a schedule in `update()` and sent to the processor over a "realtime-safe message channel". Every buffer carries a silence flag. Goal: "Properly respect realtime constraints (no mutexes!)" ([design doc, 0.12.1](https://docs.rs/crate/firewheel/0.12.1/source/DESIGN_DOC.md)). Deferred drop through `ArcGc` (#30) | A review of upstreaming it into Bevy (undated, [hackmd](https://hackmd.io/@bevy/rkwFscXObx)) cites "nontrivial UB that must be resolved" and heavy `unsafe`. [#142](https://github.com/BillyDM/Firewheel/issues/142) (2026-08-26): in DropEvents mode a full per-node event queue fell through to `Vec::push`, which reallocated on the audio thread. [#112](https://github.com/BillyDM/Firewheel/issues/112) (2026-03-11): use slice APIs instead of `resize`/`extend_from_slice`, so that real-time safety is obvious from the code | **Copy:** silence flags, compiling the schedule off the audio thread, a node arena keyed by id. **Avoid:** overflow paths that allocate, and an `unsafe`-heavy core |
| [Meadowlark](https://codeberg.org/Meadowlark/Meadowlark) / Dropseed | AGPL (ideas only). The [MeadowlarkDAW GitHub org](https://github.com/MeadowlarkDAW) has 25 repos, almost all archived on 2025-09-22; the Codeberg repo still reads "(currently incomplete)" | A DAW engine (Dropseed) plus its own GUI library (Yarrow, rootvg), its own audio I/O (rainout), an audio-graph crate and a sample loader | Infrastructure was rebuilt several times. No postmortem found; this is *inference* from the repo list | **Avoid:** infrastructure before a playable instrument. P1a takes one module end to end first |
| [OctaSine](https://github.com/greatest-ape/OctaSine) | AGPL-3.0 (ideas only). Last push 2024-07-03; 818 stars | Fixed 4-operator FM voice structure. Runtime AVX / SSE2 / fallback dispatch (`is_x86_feature_detected!` in `octasine/src/audio/gen/mod.rs`); `octasine-cli/src/bench_process.rs` checks that every SIMD path produces the same output hash (source read 2026-10-19) | Mostly host and GUI issues (HiDPI, an FL Studio crash, no GUI in Bitwig). [#205](https://github.com/greatest-ape/OctaSine/issues/205) (2025-08-20, open): a click when a quiet note follows a loud one, i.e. velocity smoothing carried across a retrigger (*anecdotal*, user report). #181: aarch64 build broke in the SIMD types | **Copy:** a hash check that every kernel path agrees (enth already tests lanes = scalar). **Avoid:** runtime dispatch between kernels (AGENTS.md rule 3), and smoother state that survives a voice retrigger |
| [Actuate](https://github.com/ardura/Actuate) | GPL-3.0 (ideas only). Last push 2026-06-04; 83 days from repo to v1.0 (07) | Fixed generator slots and filters, on nih-plug and egui | [#17](https://github.com/ardura/Actuate/issues/17) (2024-01): an LFO on SVF resonance left a DC offset that "never goes away" (fixed). [#32](https://github.com/ardura/Actuate/issues/32) (2024-03, open): cutoff changes don't reach held notes. #48 and #31: state not restored when a project reopens. #55: broke on nightly. [#62](https://github.com/ardura/Actuate/issues/62) (2026-05): considering BillyDM's nih-plug fork | **Avoid:** snapshotting parameters per voice at note-on, filters that go unstable under modulation (use a TPT/Cytomic SVF), and untested state round trips |
| [nih-plug `poly_mod_synth`](https://github.com/robbert-vdh/nih-plug/blob/master/plugins/examples/poly_mod_synth/src/lib.rs) | ISC: the nih-plug README licenses the framework and `plugins/examples/` under ISC, while the other plugins "may be licensed under the GPLv3" | Voices in `[Option<Voice>; 16]`; steals the oldest by an internal counter; per-voice smoothers for CLAP poly-mod. It renders chunks of at most 64 samples **and ends each chunk at the next event's timing** (`block_end = event.timing()`) | — | **Copy:** a fixed voice array, oldest-voice stealing, resetting poly-mod smoothers on a new voice. **Avoid:** splitting blocks at event times, which makes the control grid depend on when events arrive. enth quantises events onto a free-running 32-sample grid instead |
| [Glicol](https://github.com/chaosprint/glicol) | MIT. Last push 2025-04-06; 3,012 stars | Graph-oriented live-coding engine; an "`LCS` algorithm … for smooth and efficient whole graph updating"; runs in wasm in an AudioWorklet | — | **Copy:** diff the old and new plan by stable id, and keep the state of surviving modules |
| [knyst](https://github.com/ErikNatanael/knyst) | MIT OR Apache-2.0. 0.5.1 released 2024-11-25; 44 stars | Real-time audio graph | Quiet for about two years | Not a dependency candidate |
| [GlicolVerb](https://vmunix.com/posts/glicol-verb-first-release) (nih-plug + egui effect) | Blog post, 2025-12-30 | Glicol engine behind a wrapper of about 50 lines | The UI took 9 of 28 commits; the author vendored a patched baseview to fix a macOS crash (*anecdotal*) | Supports building P1a before P1b |

**What to copy, and what to avoid, in P1a:**

1. **State by id across plan swaps.**
   - design.md §3 swaps a new plan in "with a short crossfade". If the new plan starts from fresh state, held notes and reverb tails drop out on every structural edit, such as adding an LFO.
   - fundsp and Glicol keep the state of nodes that are unchanged.
   - Rule for enth: the compiled plan names its modules by id. At the swap, the audio thread moves over the state of every module whose id and kind survive, and only added or removed modules crossfade.
2. **Seed from stable ids**, never from structure, position or parameter values.
3. **Voice manager:**
   - a fixed voice array;
   - steal with a short fade (01's notes cite Phase Plant's "oldest and quietest");
   - reset smoothers on a retrigger (OctaSine #205);
   - apply parameter changes to held voices (Actuate #32);
   - keep CLAP's (port, channel, key, note_id) tuple and report when a voice ends, so P1b's poly-mod doesn't rework the voice manager. The [CLAP events.h](https://github.com/free-audio/clap/blob/main/include/clap/events.h) comments say "NOTE_END is sent by the plugin to the host", and -1 is a wildcard in every field of the tuple (*documented*).
4. **Quantise events onto a free-running grid; never split blocks at event times.**
5. **Build a walking skeleton before the fan-out.**

### Q2 — Real-time Rust pitfalls in practice

| Pitfall | Evidence | How it would reach enth | What catches it |
| --- | --- | --- | --- |
| Allocation on an overflow or first-use path | Firewheel #142 (2026-08-26): a full queue falls through to `Vec::push`. fundsp #54 (2024-09): `Vec::clone` doesn't keep reserved capacity (*documented*, issue text) | An event queue filled by an automation burst, a voice steal, a plan swap while the return ring is full, a buffer sized lazily on first use | `assert_no_alloc` tests that **deliberately** fill every ring and queue, steal every voice and swap plans while the return ring is full |
| Channels that allocate | crossbeam-channel [`flavors/list.rs`](https://github.com/crossbeam-rs/crossbeam/blob/master/crossbeam-channel/src/flavors/list.rs) and std [`sync/mpmc/list.rs`](https://github.com/rust-lang/rust/blob/master/library/std/src/sync/mpmc/list.rs): an unbounded channel allocates a new block every `BLOCK_CAP` (31) messages (*documented*, source) | `std::sync::mpsc::channel()` used for parameter events | A clippy `disallowed-methods` ban on `std::sync::mpsc` and crossbeam constructors in `enth-engine`. Use `rtrb` 0.4.0 (wait-free SPSC, MIT OR Apache-2.0, released 2026-08-18) |
| A drop on the audio thread | The `assert_no_alloc` [README](https://github.com/Windfisch/rust-assert-no-alloc): dropping a `Box` would trigger it. An arc-swap reader can end up holding the last reference (a [dev.to comment](https://dev.to/zacharylee/how-i-made-a-rust-hot-path-27x-faster-and-the-ai-fix-i-refused-to-merge-3llg/comments), Jul 2026, *anecdotal*; 01's notes read the same in arc-swap's docs) | The old plan; replaced wavetables | The return ring for old plans (architecture.md) with a pending slot for when it is full; `assert_no_alloc` catches frees too. Don't use arc-swap or basedrop: `basedrop` 0.1.3 (2025-10-29) works, but the ring already does the job |
| Panics and formatting | Formatting a panic message allocates; slice indexing panics in release builds; debug builds check for integer overflow (*inference* from std behaviour) | `buf[i]` with the wrong length, `unwrap()` in `process`, `debug_assert!(x.is_finite(), "{x}")` | Render properties under `assert_no_alloc` with random valid patches. Deny clippy's `unwrap_used`, `expect_used` and `panic` in `enth-engine`'s process modules. [`no-panic`](https://github.com/dtolnay/no-panic) 0.1.37 checks at link time, but its README warns that it can need optimisation to pass and only fires when a linker runs: not worth it in P1a |
| Locks | Research 03 and the engine spike: RTSan misses an uncontended std `Mutex` (*measured*) | Any `Mutex` | The root `clippy.toml` `disallowed-types` ban, already in place |
| Denormals | engine.md: a 40 s tail takes 33.5× the wall time but only 4.1× the instructions without flushing (*measured*) | Reverb feedback, filter and envelope tails, a compressor's detector | Flush in code, plus the subnormal test. An instruction-count gate alone misses it |
| NaN and infinity | Actuate #17 (*anecdotal*); serde's `f64 as f32` turns `1e39` into `inf` (serde_core 1.0.229 `src/de/impls.rs`, `num_as_copysign_self`, read from the local registry; *documented*); `log10(0)` in a compressor's dB detector (*inference*) | A compressor at silence, an SVF's `tan` near Nyquist, out-of-range numbers in JSON | Validation rejects non-finite values; cutoff is clamped below Nyquist; hashing panics on a non-finite sample (as in the spike); a render property that every output sample is finite |
| Branchy or dispatched per-sample code | OctaSine dispatches between AVX, SSE2 and fallback kernels at runtime (source). engine.md measured lanes = scalar | A per-sample `match` on the module kind, `if`/`else` per voice | Branch at control rate; compile the plan into flat arrays. gungraun budgets catch cost regressions exactly |

**Tools:**

- **`assert_no_alloc` 1.1.2** (BSD-1-Clause, 2021-08-03, already in `Cargo.lock`):
  - checks "memory (de)allocations for a thread", per thread through thread-local storage;
  - `permit_alloc` is the escape hatch;
  - tests need `#[cfg(debug_assertions)]` because `disable_release` is a default feature (README; measured in 03 and in the engine spike);
  - a violation aborts the process, which is one reason to use nextest (below).
- **RTSan:**
  - `-Zsanitizer=realtime` with `#[sanitize(realtime = "nonblocking" | "blocking" | "caller")]` ([unstable book](https://doc.rust-lang.org/nightly/unstable-book/compiler-flags/sanitizer.html), fetched with curl on 2026-10-19);
  - `"blocking"` can mark our own spin-waits as violations;
  - the book recommends `-Zbuild-std` for full instrumentation;
  - the local nightly 1.101.0 (2026-10-17) ships `librustc-nightly_rt.rtsan.a` (*measured*);
  - so far it has only been run by hand in scratch, so it should become a nightly CI job.
- **cargo-nextest:**
  - runs "each individual test in a separate process", and "One test segfaulting does not take down a bunch of other tests" ([how it works](https://nexte.st/docs/design/how-it-works/), [why process-per-test](https://nexte.st/docs/design/why-process-per-test/));
  - so an `assert_no_alloc` abort fails one test, not the whole test binary;
  - 0.9.148 on crates.io (2026-10-08); 0.9.133 installed;
  - it is generally understood not to run doctests (I did not find this on the pages I fetched), so CI keeps `cargo test` as well.

## Test strategy

### Versions and licences (crates.io API, 2026-10-19; *documented*)

| Crate | Version (date) | Licence | Dependencies (`cargo tree`, normal + build, root excluded; *measured*) |
| --- | --- | --- | --- |
| proptest | 1.12.0 (2026-10-10), MSRV 1.88 | MIT OR Apache-2.0 | 26, of which about 12 are not yet in `Cargo.lock` (rand 0.10, regex-syntax, rusty-fork, wait-timeout, …) |
| proptest-state-machine | 0.9.0 (2026-10-10) | MIT OR Apache-2.0 | — |
| quickcheck | 1.1.0 (2026-02-10) | Unlicense OR MIT | 14 |
| insta (`json`) | 1.49.0 (2026-10-03); cargo-insta 1.47.2 installed | Apache-2.0 | 14 (`console` and `similar` are new) |
| gungraun | 0.20.0 (2026-09-26) | Apache-2.0 OR MIT | 28 (already in `Cargo.lock` through spike-engine) |
| arbitrary (`derive`) | 1.5.0 (2026-10-06) | MIT OR Apache-2.0 | 6 |
| libfuzzer-sys | 0.4.13 (2026-06-04); cargo-fuzz 0.13.1 installed | **(MIT OR Apache-2.0) AND NCSA** | 7, including `cc`, which compiles libFuzzer's C++ |
| trycmd / assert_cmd / insta-cmd | 1.2.1 / 2.2.2 / 0.7.0 | MIT OR Apache-2.0 / MIT OR Apache-2.0 / Apache-2.0 | 54 / 12 / not resolved |
| cargo-mutants | 27.1.0 (2026-06-02); 27.0.0 installed | MIT | tool only |

### The layers, by cost of signal

| Layer | What it proves | New crates; cold `--no-run` cost | Cost per iteration | Where it runs | Signal for agents |
| --- | --- | --- | --- | --- | --- |
| Unit and doc tests in each crate | Local logic | 0 | `cargo test -p leaf --no-run` 0.32 s warm; whole toy workspace 1.02 s (06, *measured*) | Every edit | Fastest; only as good as the asserts |
| Property tests (ops, serde, migrations, units, canonicalise, engine invariants) | Invariants over thousands of inputs | proptest: about 12 new crates, **15–40 s cold (*estimated*** from 03's 1.2–2.5 s per crate under load) | 4 properties × 2,000 cases ≈ 0.1 s (01's notes, measured) | Every edit with a fixed seed; nightly with random seeds and 10× the cases | High, if the gate's seed is fixed and `proptest-regressions/` is committed |
| Golden renders (`enth-golden`) | Bit-exact output, per module and composite | 0 (`sha2` and `toml` are already in the lock) | 1–40 ms per render (engine.md); one test binary | Every DSP edit; CI on Linux, Windows and wasm | Exact but opaque, so it needs the summary diff |
| Real-time invariants: `assert_no_alloc`, 40 s subnormal tail, FTZ on = off, block-size invariance, lanes = scalar | The four DSP rules; no allocation | 0 | A 40 s tail renders in about 40 ms (engine.md) | Every engine edit (debug, with opt-level 3 for DSP crates) | Exact and cheap |
| CLI snapshots (insta) | JSON shapes, errors, exit codes | 14 crates, **5–15 s cold (*estimated*)** | 0.06 s per snapshot; 0.47 s leaf loop (06, *measured*) | Every CLI edit | High; readable diffs |
| Instruction budgets (gungraun) | CPU regressions, exactly | **37 s cold, 6.4 s incremental**, plus a one-time 205 s `gungraun-runner` install (03, *measured*) | Seconds per case under callgrind | CI on Linux, and on demand | Exact, but Linux and valgrind only |
| Fuzz-lite (proptest over arbitrary JSON and random renders) | No panics, typed errors, finite output | 0 beyond proptest | About 1–5 s for 32 render cases (*estimated*) | Small runs per edit; large runs nightly | Good |
| cargo-fuzz | Deeper parser coverage | A separate nightly workspace, an ASan rebuild of the whole tree and the libFuzzer C++ build (**not measured**) | Minutes per target | Nightly, time-boxed | Slow; needs the owner's OK |
| Mutation testing (cargo-mutants) | Whether the tests really constrain the code | 0 | One build and test run per mutant | Weekly, or `--in-diff` on spine PRs | Finds weak asserts |
| RTSan | Syscalls, locks and allocations inside `process` | A nightly rebuild | — | Nightly CI | Complements `assert_no_alloc` |
| Agent eval | Whether the agent loop works | — | API spend | Nightly or on demand; never a gate (agent-cli.md) | Noisy |

### Golden renders (`enth-golden`)

Report 05's verified harness is the base: cases as text, no WAVs, `--update` refused under CI. Change these:

- **One hash per case, valid on every platform.** This is design.md §5 now; keep per-platform overrides only for aarch64 and macOS until they have run. 05's draft still keys hashes by `<arch>-<os>`.
- **Report every mismatching case in one run, then fail.** Show the summary diff (RMS per 10 ms, centroid) for each. 05's draft panics at the first mismatch, so an agent fixes one case at a time.
- **Small per-module cases, owned by the module lanes**: `osc-saw-1v`, `svf-lp-res-sweep`, `reverb-impulse`, `comp-step` and so on. A few composite cases belong to the spine: the 16-voice G1 patch with every module, a phrase with voice steals, the 40 s tail. A broken hash then names the module.
- **Cases are read from `tests/golden/cases/*.toml` at runtime**, as in 05's harness. Adding a case means adding two text files, with no code change and no rebuild, and one file per case means no merge conflicts.
- **`--update` prints the list of changed cases**, which goes into the commit message. Reviewers reject golden updates in commits that don't say why; AGENTS.md already requires the reason.
- **Never assert `render(p) == render(canonicalise(p))` bit for bit.** Canonicalising reorders interchangeable modules, and f32 addition of three or more terms is not associative (*inference*; 01's notes reach the same conclusion). Assert that the canonical hash is stable and that descriptors stay close instead.

### Property tests (proptest, not quickcheck)

proptest shrinks per strategy, persists failures to `proptest-regressions/` files by default, and has `proptest-state-machine` for sequences of ops. quickcheck is lighter (14 crates), but one framework is enough.

Properties for the spine:

- **Params:**
  - `parse(display(x)) == x` for every `ParamSpec` and every accepted unit;
  - plain ⇄ normalised round trips where the mapping is invertible.
- **Patch JSON:**
  - the JSON round trip is bit-exact for every finite f32, with serde_json's `float_roundtrip` feature on (its [Cargo.toml](https://github.com/serde-rs/json/blob/master/Cargo.toml): "makes f64 -> JSON -> f64 produce output identical to the input", at "approximately 2x" parse cost);
  - `-0.0` is canonicalised to `+0.0`;
  - non-finite values are rejected with a typed error, because NaN and infinity serialise to `null` ([json-canon docs](https://docs.rs/json-canon), *documented*).
- **Ops:**
  - apply, then the inverse, gives back the original, with exact structural equality rather than canonical equality. That includes `RemoveModule` with routes, automation slots and macros pointing at the removed module;
  - the inverse of an inverse;
  - random op histories with undo and redo agree with a snapshot model (proptest-state-machine);
  - keep negative controls: 01's notes show that dropping the route restore from `RemoveModule`'s inverse fails 3 properties, and merging a gesture's `SetParam`s across *different* addresses fails the history property.
- **Migrations:**
  - every fixture of every version loads;
  - `migrate` is idempotent;
  - every migrated patch validates.
- **Canonicalise:** idempotent; the hash is unchanged when interchangeable modules are permuted.
- **Engine:**
  - a small parameter move gives a small descriptor move (this catches patch-hash seeding);
  - the render is the same for host block sizes {1, 7, 63, 64, 65, 512, 4096, random};
  - every output sample is finite;
  - voices end below −100 dB.

Determinism of the tests themselves:

- The gate runs with a fixed seed (`Config::rng_seed` or `PROPTEST_RNG_SEED`) and the default 256 cases ([Config docs](https://docs.rs/proptest/1.12.0/proptest/test_runner/struct.Config.html)).
- A nightly job runs random seeds with `PROPTEST_CASES=10000` and commits any new regression files.
- Agents see a deterministic red or green; exploration happens off the critical path.

### Fuzzing

- **Stable first: fuzz-lite inside proptest.**
  - Arbitrary JSON values and mutated valid patches go into `Patch::from_json`, `migrate`, `validate` and `apply(ops)`. Each must never panic and must always return a typed error with a hint; anything that loads must round-trip.
  - The unit parser gets random strings.
  - Random valid patches and performances go into `render` under `assert_no_alloc`: no panic, finite output, no subnormals at the end.
  - serde_json's default recursion limit of 128 (`remaining_depth: 128` in `de.rs`) already protects the stack.
- **cargo-fuzz only with the owner's OK.**
  - The [rust-fuzz book](https://rust-fuzz.github.io/book/cargo-fuzz/setup.html) says it "requires the nightly compiler" and "a C++ compiler with C++11 support".
  - `libfuzzer-sys` brings the NCSA licence. AGENTS.md says to ask before adding a C/C++ toolchain or a licence that `deny.toml` lacks. `cargo fuzz init` creates a separate workspace, so `cargo deny` on the main workspace would not even see it.
  - If approved: run nightly, time-boxed at about 5 minutes per target; commit the corpus and replay it in a stable test, so every find becomes an ordinary regression test.
  - The book's serde example mutates JSON, and in its experiment mutation gave better coverage than generation ([structure-aware fuzzing](https://rust-fuzz.github.io/book/cargo-fuzz/structure-aware-fuzzing.html)).

### Instruction budgets (gungraun)

- **A gate from the first engine commit,** as the Phase 0 summary's carry-over list asks:
  - absolute `hard_limits` per case at the measured count plus 10 %;
  - the G1 case (16 voices, every P1 module, reverb and compressor on) at ≤ 340 M instructions per 2 s render (roadmap.md).
- **One case per module lane,** with its own budget, so a regression names the module. Declare the bench targets up front in S0 (below), or `enth-engine/Cargo.toml` becomes a merge hotspot.
- **Linux and valgrind only,** and callgrind under-counts the denormal penalty, so the subnormal test stays.
- **Not in the per-edit loop:** run it in CI and before merging any DSP change.

### CLI snapshots (insta)

- **Snapshot the CLI in process.** Make `enth-cli` a thin binary over `enth_cli::run(args, stdin) -> Output { code, stdout, stderr }`, and snapshot that with `assert_json_snapshot!`; no binary rebuild or process spawn per test.
- **Keep a handful of end-to-end tests** that spawn the built binary with `std::process::Command`, for TTY detection and exit codes. trycmd (54 crates) is too heavy, and assert_cmd isn't needed.
- **One snapshot per command and per error class** (unknown parameter with `did_you_mean`, out of range, bad unit, file not found). Together they are the error contract agents rely on.
- **Round floats in analysis output** with insta's `rounded_redaction` (needs the `redactions` feature; [docs](https://docs.rs/insta/1.49.0/insta/fn.rounded_redaction.html)), or snapshot `describe`/`check` on Linux only. rustfft picks its code path by CPU, with relative differences up to 2.7e-6 (design.md §5), so late digits can differ between CI platforms (*inference*).
- **In CI, insta acts like `--check`** (`CI=true`, insta ≥ 1.48, per report 05).

### Test layout and runners

- **One integration-test binary per crate** (`tests/it/main.rs` with modules). Every `tests/*.rs` file is its own binary; consolidating Cargo's own tests that way cut their compile time about 3× and their artifacts about 5× ([matklad, 2021-02-27](https://matklad.github.io/2021/02/27/delete-cargo-integration-tests.html), *documented*).
- **nextest for engine tests;** `cargo test` stays in CI for doctests.
- **`cargo mutants --in-diff`** on `enth-params` and `enth-patch` for every spine PR. The [mutants.rs docs](https://mutants.rs/in-diff.html) call it a fast first check, "not a substitute for a full test run". Agent-written tests tend to assert too little (*inference*), and surviving mutants make that visible.

### What gives agents the fastest, most reliable signal

1. **`cargo nextest run -p <crate>`**: unit tests, fixed-seed properties, goldens and real-time invariants for the crate being edited. About 0.5–2 s after an incremental edit (06 measured 0.47–0.95 s on small crates). This is each lane's inner loop.
2. **`cargo xtask golden`** after any DSP change: exact, with summary diffs.
3. **`cargo clippy`** with the numeric, I/O, lock and channel bans. It catches determinism and real-time rule breaks statically; the engine spike's probe caught 5 of 5 violations.
4. **The full Done-means gate** before every commit (about 3 s on the skeleton today).
5. **CI only:** Windows and wasm goldens, gungraun budgets, nightly RTSan, nightly proptest exploration, nightly cargo-fuzz if approved, and macOS goldens weekly or on demand.
6. **Agent evals** nightly or on demand; never a gate.

Reliability rules: no wall-clock assertions anywhere, no random seeds in the gate, no test that depends on another lane's goldens, and a flaky test is a bug.

## Proposed P1a lane plan

The serial spine has three freeze points:

- **F0:** workspace prep is merged.
- **F1:** the params and patch *types* are merged and reviewed.
- **F2:** the walking skeleton is merged: a reference saw goes through `Module`, `render`, a golden and CI on Linux, Windows and wasm.

Lanes fan out after each freeze. After F2, changes to spine types are additive only: new fields get defaults.

Sizes are agent-hours. In Phase 0, each lane took 0.8–2.8 agent-hours starting from a working probe (`docs/spikes/summary.md`). P1a has probes for most items (spike-engine, spike-metric, spike-agent, and 01's and 02's prototypes), but it adds migrations, integration and review, so I estimate 2–4× Phase 0's time per lane (*estimated*).

| Lane | Owns | Depends on | Acceptance | Size (h) | Needs the owner? |
| --- | --- | --- | --- | --- | --- |
| **S0 Workspace prep** (F0) | Root `Cargo.toml` with every P1a workspace dependency pinned: `libm =0.2.16`, serde_json with `float_roundtrip`, sha2, rtrb, assert_no_alloc, proptest, insta (`json`, `redactions`), clap, schemars, rustfft/realfft, ebur128, gungraun. `Cargo.lock`. Root `clippy.toml`: the numeric bans moved from spike-engine, plus the channel ban. Empty crates `enth-cli`, `enth-solver` and `tests/golden` (`enth-golden`). Pre-declared `mod` lines and empty files in `enth-dsp`, `enth-engine` and `enth-analysis`. `xtask golden` and bench commands. CI jobs: goldens on Linux, Windows and wasm; the gungraun budget; nightly RTSan and proptest. One bench target per module, declared up front | — | The Done-means gate is green. `cargo xtask golden` passes one trivial case on Linux, in Windows CI and on wasm. `cargo deny` passes with the new dependencies | 1–3 | No (yes only if fuzzing dependencies are wanted) |
| **S1a Params and patch types** (F1) | `enth-params`: `ParamSpec`, `Unit`, `Range`, `Mapping`, `Smoothing`, parse and display. `enth-patch`: model types, and a catalogue holding the `ParamSpec`s of **every** P1a module kind (oscillators, wavetable, noise, SVF, ADSR, LFO, reverb, compressor, voice settings, macros, master level) | S0 | Parse/display property passes. Catalogue checked at compile time (01's notes: a `const fn` over a static `[ParamSpec]` rejects a default that is out of range). Reviewed by a judge agent before the fan-out | 3–5 | Optional: skim the parameter list and ranges (15 min) |
| **S1b Ops, serde and migrations** | `enth-patch`: ops with inverses (including the cascades), parameters parsed from `Value` against their `ParamSpec`, validate, canonicalise, an exact hash and a canonical hash, schema v1, the migration framework and a v0 fixture | S1a (alongside S2a) | The property suite above; fixtures per version; surviving mutants from `cargo mutants --in-diff` reviewed | 4–8 | No |
| **S2a Engine walking skeleton** (F2) | `enth-engine`: `Module`, `PrepareCfg`, `ProcessCx`, a minimal plan compiler, the free-running 32-sample grid with FIFO re-chunking, seeds from (render seed, module id, note), a stub (silent) module per kind, a reference saw. `enth-render`: `render(patch, perf, spec)`. The first golden cases. `assert_no_alloc`, subnormal, FTZ and block-size tests. The gungraun scaffold | S1a | The saw patch has one hash on Linux, Windows and wasm in CI. Host blocks of {1, 7, 64, 512, 4096} give the same hash. `assert_no_alloc` passes. A registry test: every descriptor has an implementation and vice versa | 4–8 | No |
| **S2b Voices, modulation and plan swap** | The voice manager (poly, mono, legato, glide, stealing with a fade, smoother reset on retrigger, the CLAP tuple, voice end). The modulation matrix, macros and smoothing. Plan swap with state carried by id and crossfades only for added or removed modules. The rtrb command ring and return ring | S2a | Goldens per voice mode. Overflow tests under `assert_no_alloc`: full rings, every voice stolen, a swap while the return ring is full. A held note survives an unrelated structural edit | 6–10 | No |
| **D1 Oscillators and noise** | `enth-dsp` oscillator, wavetable and noise files; engine generator modules; golden cases `osc-*` and `noise-*`; `docs/dsp/osc.md`, `noise.md` | F2 (DSP cores can start after F0) | Goldens; an aliasing check (spectrum of a high note under a threshold); an instruction budget per module; docs cite their sources | 4–8 | No |
| **D2 Filter, envelopes and LFO** | `enth-dsp` SVF, envelope and LFO files; modules; goldens; docs | F2 | Goldens. The SVF stays stable under audio-rate cutoff modulation at full resonance: finite output and no stuck DC offset (Actuate #17). Envelopes end below −100 dB | 3–6 | No |
| **FX1 Reverb port** | `enth-dsp/reverb/`; the module; the `THIRD_PARTY_NOTICES.md` entry; `docs/dsp/reverb.md`; a C++ reference harness kept in scratch | F2 (DSP core after F0) | Bit-exact against the upstream C++ built at `-O2`, which 02's notes show is feasible on gcc, clang and wasm. Seeded from the module id. Zero latency. Builds for wasm. Subnormal tail test. Level-matched A/B renders against fundsp's `reverb2`/`reverb4`, kept in scratch | 3–6 | **Yes:** a blind, level-matched A/B (20–30 min) |
| **FX2 Compressor** | `enth-dsp` compressor; the module; docs | F2 | Step-response goldens; no NaN at silence (`log` of 0); zero latency; cross-checked against web-audio-api-rs | 3–5 | Optional listen |
| **A Analysis** | `enth-analysis`: spike-metric's features, YIN and loudness; spike-agent's descriptors with beat-smoothed envelopes, clipping and margins; the expectation parser and checker; a measurement of the scalar-FFT cost | S0 only | agent-probe.md's descriptor audit tables become regression tests (sweeps against theory). Detuned cases read correctly. Scalar-FFT cost recorded | 6–10 | No |
| **C CLI v0** | `enth-cli` (13 commands), `content/templates`, insta snapshots, JSON Schemas | Phase 1: S1b. Phase 2: S2a, A and H | In-process snapshots per command and error class; exit codes; `enth guide`; `enth bench` | 6–10 | No |
| **H Hints** | `enth-solver::sensitivity`: a finite-difference Jacobian; one multi-parameter least-squares hint that keeps passing items passing; an explicit "no single move works" verdict | S2a and A | Conflicting goals like the probe's g5 get a hint that doesn't trade one failure for another. Identical results for any worker count | 4–6 | No |
| **T Listening tools** | ABX (the G0 search gate) and a blind, level-matched A/B (the reverb), reusing spike-metric's `listen` design; writes WAVs and answer files | A (loudness) | Dry run by an agent; one page of instructions for the owner | 2–3 | The owner uses them |
| **E Evals and G1 close** | `content/agent-evals` (10 goals with their lists), `cargo xtask agent-eval` or a script, the 3-run protocol, WAVs for the blind listen, the 16-voice budget case, the G1 table | Everything | The agent-cli.md protocol; at least 5 of 10 goals pass; the headless G1 table filled in | 3–5, plus API spend | **Yes:** a blind listen, at least 7 of 10 (15–20 min); approve the API spend |

**Total: about 52–93 agent-hours.**

**Waves:**

1. S0.
2. S1a, A, and the FX1 and FX2 DSP cores (only `enth-dsp`, no `Module` yet).
3. After F1: S1b, S2a.
4. After F2: S2b, D1, D2, the FX module wrappers, and C phase 1.
5. H, C phase 2, T.
6. E and the owner's gates.

**Critical path:** S0 (1–3) → S1a (3–5) → S2a (4–8) → D2 or FX1 (3–8) → H with C phase 2 (4–6) → E (3–5), about **18–35 agent-hours**, plus the owner's gates.

**Merge hotspots and how the plan defuses them:**

| Hotspot | Who edits it | Mitigation |
| --- | --- | --- |
| `[workspace.dependencies]`, `Cargo.lock` | S0 only; later additions by the coordinator, one commit each | Lanes ask for a dependency; they never edit the lockfile |
| Catalogue registration in `enth-patch` | S1a declares every P1a kind | DSP lanes touch only their own module's spec file, and only if ranges change |
| Engine module factory (`match kind`) and `mod` lines | S2a, with stubs | Each lane replaces its own stub's file |
| `mod` lines in `enth-dsp/src/lib.rs` and `enth-analysis/src/lib.rs` | S0 | Empty files exist from the start |
| `tests/golden/cases` and `expected` | One file per case | Composite cases are spine-only |
| `[[bench]]` entries in `enth-engine/Cargo.toml` | S0 declares one bench file per module | Lanes fill in their own bench file |
| `xtask/src/main.rs`, CI workflows | S0 | Every P1a command and job is stubbed in S0 |
| `THIRD_PARTY_NOTICES.md` | FX1 only | — |
| `enth-cli` | Lane C only | — |
| Docs | One `docs/dsp/<module>.md` per lane, plus the lane's report | — |

**Concurrency:**

- P1a lanes are light: none of them pulls nice-plug, baseview, egui or clack-host. AGENTS.md's limit of 2 heavy builds therefore binds only if P1b work starts alongside.
- The machine had 8.4 GiB available, at load about 7, on 12 threads when I checked (*measured*). So: at most 4 lanes building at once, `CARGO_BUILD_JOBS=3` each, a separate `CARGO_TARGET_DIR` per worktree on ext4, and `flock -x` around gungraun and any timing run.
- Phase 0 ran up to 3 lanes at once (summary).
- One coordinator merges in wave order and runs the full gate after every merge.

**The headless G1 checks:**

| Check | How | Who | When |
| --- | --- | --- | --- |
| 16 voices ≤ 340 M instructions per 2 s render | A gungraun case in CI | Agents (S2b, E) | From the first engine commit, with a provisional budget; final in E |
| p99.9 `process()` under 25 % of the deadline | Test host | P1b | — |
| Patches round-trip through JSON | Properties, fixtures, fuzz-lite | S1b | Continuously |
| Goldens on three OSes and wasm32, reverb and compressor included | CI on Linux, Windows and wasm per push; macOS on demand or weekly (about 10× the minutes) and once before G1 | Agents; the owner approves macOS minutes if quota matters | Before G1 |
| Reverb chosen by a blind, level-matched A/B | T and FX1 | **Owner**, 20–30 min | As soon as FX1 lands |
| Agent eval: at least 5 of 10 goals, and a blind listen with at least 7 of 10 | E | Agents and the **owner**, 15–20 min | At the end |
| "A bass, pad, pluck and lead you like" | Through the editor, or the CLI with an agent | Owner | P1b (recommended) |

## Top risks

| # | Risk | Evidence | Impact | Cheapest retirement | Lane |
| --- | --- | --- | --- | --- | --- |
| 1 | **The spine's APIs churn after the fan-out** (`ParamSpec`, `Patch`, `Module`, `ProcessCx` change while four lanes build on them) | The Phase 0 spikes were single-author, with no migrations or cross-lane integration (summary). Projects stall on infrastructure rewrites (Meadowlark, inference) | Rework across lanes; conflict storms | Two freeze points with a judge review (Opus) before each fan-out; a walking skeleton taking one real module through render, a golden and CI on three targets; only additive changes after F2 | S1a, S2a |
| 2 | **Seeding from the patch hash** (design.md §5) | fundsp documents that structure-seeded phases change other nodes' sound. 01's notes: 6.6 dB against 0.008 dB RMS-envelope change for a 0.1 % cutoff move | Noisy hints, `suggest` and CMA-ES; goldens of noisy or detuned patches change on any parameter edit | Change §5 to (render seed, module id, note index); add the property "a small move gives a small descriptor change" | S2a (doc edit now) |
| 3 | **Host block size and event timing leak into renders**, so the plugin and the CLI disagree | engine.md: 48-sample blocks change the hash. nih-plug's example splits blocks at events | P1b's plugin goldens disagree with the CLI's; the owner hears the CLI and the plugin differ | A free-running grid with a FIFO, plus a block-size invariance property that includes 1 and 4096. 01's prototype: 1,200 of 1,200 identical | S2a |
| 4 | **Op and JSON correctness** | serde's `f64 as f32` (`inf` for `1e39`); best-effort float parsing; NaN becomes `null`; untagged-enum errors ([serde PR #1544](https://github.com/serde-rs/serde/pull/1544) shows the bare message). 01's negative control on the `RemoveModule` cascade | Undo corruption later (P1b, S1); the round-trip gate fails; agents get errors they can't act on | `float_roundtrip`; reject non-finite values; canonicalise −0.0; parse parameters from `Value`; the property suite and fuzz-lite | S1b |
| 5 | **Real-time and numeric regressions on rare paths** | Firewheel #142, fundsp #54, Actuate #17, OctaSine #205 | Xruns and clicks in P1b; NaN in renders | Overflow tests under `assert_no_alloc`; nextest; nightly RTSan; a finite-output render property; an SVF modulation stress test; the channel ban in `enth-engine` | S2b, D2, FX2 |
| 6 | **Golden churn**, rubber-stamped `--update`, opaque failures, analysis drifting across platforms in snapshots | A broken hash says nothing about what changed. rustfft's CPU paths differ by up to 2.7e-6 relative (design.md §5) | Real regressions merged as "updated goldens"; flaky CI | Per-module cases, summary diffs, every failure reported in one run, CI refuses updates, the commit's reason checked in review; rounded redactions or Linux-only `describe` snapshots | S0, every DSP lane |
| 7 | **G1's validity and the owner's bandwidth** | The probe passed 15 of 15 goals trivially; descriptors are in-house code; the owner's checks are now the critical path, with about 3 h still pending from Phase 0 (summary) | The gate passes without good sounds, or waits on the owner | Turn the descriptor audit into regression tests; announce the two short owner sessions (reverb A/B, final listen) early; have the listening tools ready before they're needed | A, T, E |
| 8 | **Merge hotspots and machine contention** | `Cargo.lock` and registration conflicts (07, citing a ten-agent postmortem); 8.4 GiB available at load 7 (*measured*) | Hours lost to conflicts and swapping | S0 declares dependencies, `mod` lines, catalogue kinds, bench targets and CI up front; at most 4 lanes building; one coordinator merging in wave order | S0, coordinator |

Just outside the top 8:

- The hints need a crate at L4 or above: a decision to make, not a risk.
- cargo-fuzz's C++ toolchain and NCSA licence: the owner's call.
- macOS CI minutes: a cost.
- The scalar-FFT cost is still unknown: lane A measures it.

## Plan check

| Doc | Current text | Proposed change | Why |
| --- | --- | --- | --- |
| design.md §5 | "per voice from the patch hash and note index; per effect from the patch hash and module id" | Per voice from (render seed, voice or note index); per module from (render seed, module id). The patch hash only keys caches | Finite differences need smooth renders. fundsp's documentation; 01's measurement |
| design.md §5, AGENTS.md | "Block size is part of the render function … re-chunks the host's buffers onto that fixed internal block" | A free-running 32-sample grid behind a FIFO, so renders don't depend on the host's block size at all; events quantise to the grid; a property test across host block sizes | Avoid nih-plug's split-at-events pattern; 01's prototype |
| design.md §3 | "swap it at a block boundary with a short crossfade" | Carry over the state of every module whose id and kind survive; crossfade only modules that were added or removed | fundsp's migration and Glicol's LCS diff; held notes and tails survive edits |
| design.md §2 | `ParamValue` "serde: untagged"; nothing on floats | Parse parameters from `Value` against their `ParamSpec` for typed errors; `float_roundtrip`; reject non-finite values; canonicalise −0.0 | serde source and docs; agent-facing errors |
| architecture.md, the brief | The P1a crates | Add a minimal `enth-solver` (sensitivity only) for the hints in `check` and `step`, and `enth-golden` | Hints need render and analysis, which are both L3 |
| AGENTS.md, Done means | — | Engine tests run under nextest (process per test); one integration-test binary per crate; a fixed proptest seed in the gate, with `proptest-regressions/` committed; a golden update lists the cases it changed | Faster, deterministic signal |
| AGENTS.md, audio thread | The current rules | Add: no `std::sync::mpsc` or unbounded channels in `enth-engine` (a clippy ban); every ring and queue has an overflow test under `assert_no_alloc` | Firewheel #142; crossbeam and std channel source |
| roadmap.md, G1 | "golden tests pass on three OSes" | Linux, Windows and wasm on every push; macOS on demand or weekly, and once before G1 | macOS costs about 10× Linux on a private repo ([GitHub billing](https://docs.github.com/en/billing/concepts/product-billing/github-actions): $0.062 against $0.006 a minute; Free plan 2,000 minutes a month) |
| roadmap.md, G1 | "you can make a bass, pad, pluck and lead you like" | Mark it P1b (it needs the editor), or explicitly allow patches made through the CLI in P1a | Makes the headless half unambiguous |
| AGENTS.md / deny.toml (fuzzing) | — | The owner decides on cargo-fuzz (C++ and NCSA); default to proptest fuzz-lite | AGENTS.md: "Ask before adding a dependency that pulls in a C/C++ toolchain … or a licence not in deny.toml" |
| Effort estimates | Human-days | Agent-hours per lane plus owner-hours per gate, as in this plan | The Phase 0 summary's calibration |

## Gaps

- **No builds.** The cold compile costs of proptest, insta and cargo-fuzz are estimates from dependency counts and Phase 0's analogues; only gungraun's are measured (report 03).
- **Meadowlark's reasons for restarting** are inferred from its repo list; I found no postmortem.
- **arc-swap's drop semantics:** the docs pages I fetched don't state them. I relied on 01's notes and an anecdotal comment.
- **Firewheel's return ring** ignoring a failed push, which would drop on the audio thread, comes from 01's reading of the source, not mine.
- **KVR and the Rust Audio Discord:** I found no Rust-specific threads, and Discord can't be searched.
- **Not resolved or verified:** whether nextest runs doctests; insta-cmd's dependency tree. bolero was not evaluated beyond its README.
- **Not checked:** how long a macOS golden job runs, and whether arm runners are available.
- **Estimates, not measurements:** the agent-hour ranges and G1's 1.5× allowance per voice.
- **Process slip:** before the coordinator's rule arrived, two batches of crates.io requests in this session sent a User-Agent containing the owner's email. Later requests used a generic User-Agent.
