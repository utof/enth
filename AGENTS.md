# AGENTS.md

Rules for AI coding agents working on `enth`. Read `docs/README.md` first, then the doc for the area you're touching. Phase 0, Task 2 copies this file to the repo root.

## Architecture (non-negotiable)

- Layers, lowest first: `dsp`, `params` (L0) → `patch` (L1) → `engine` (L2) → `render`, `analysis` (L3) → `challenge` (L4) → `solver` (L5) → `session` (L6) → `ui` (L7) → `plugin`, `cli`, `standalone`, `wasm`, `py` (L8). Depend only on strictly lower layers. `xtask/layers.toml` is the source of truth; `cargo xtask check-layers` must pass. Tooling (`xtask`, `enth-test-host`, `spike-*`) is exempt.
- Core crates (L0–L5) do no I/O, spawn no threads, read no clocks, use no global RNG, and must build for `wasm32-unknown-unknown` (`cargo xtask wasm-check`). Callers supply parallelism (e.g. the CLI passes a rayon-backed evaluator to the solver) and I/O.
- Audio thread: `engine::process` and everything it calls never allocate, lock, log, do I/O or panic. Engine tests wrap `process()` with the `assert_no_alloc` crate; plugin debug builds also enable nice-plug's `assert_process_allocs`.
- Every patch mutation is an `Op` from `enth-patch`. No other way to change a patch.
- Patches store **plain values in real units** (Hz, ms, dB, cents, %, octaves). Normalised 0–1 values exist only at host automation, UI gesture and ML boundaries.
- Persisted or emitted types (patch, challenge, lesson, CLI JSON) carry a version and have fixture tests. Changing one requires a migration or a version bump.
- A new module = `ParamSpec`s in the `patch` catalogue + implementation in `engine` + golden render test + a note in `docs/dsp/` + a lesson and an agent eval when the module is user-facing.
- Shells (`plugin`, `cli`, …) translate; they never contain product rules.
- All randomness is seeded and passed in.
- Ported DSP follows the same contract: replace any upstream RNG (`rand()`, `std::mt19937`, `thread_rng`) with the engine's seeded PRNG; flush denormals in code, not through CPU flags; allocate only in `prepare`. Effects report zero latency unless an ADR says otherwise. No crates that wrap C/C++ in L0–L5, and no third-party plugin hosting in the engine (`docs/adr/0006-effects-strategy.md`).

## UI

- Native Rust UI (egui by default; see `docs/adr/0002-ui-toolkit.md`). No WebView.
- Repaint only on input, parameter change, or new data for a *visible* scope. No timers or work when the editor is closed or hidden.
- The UI never blocks on the audio thread and never shares a lock with it; use atomics and lock-free rings.
- **Design tokens only** (`docs/design-system.md`): every colour, size, spacing, radius, stroke, font and repaint rate comes from `enth-ui`'s `theme` module. No raw values in widget code; need a new one, add a token. Keep the look minimal: flat, two accents, small scales.

## CLI

- JSON output when stdout is not a TTY; every object has `"schema": "enth.<command>/<version>"`.
- Real units in and out. Stable exit codes (see `docs/agent-cli.md`). Errors include a `hint`.
- Output shape changes require a schema version bump and updated snapshot tests (`insta`).

## Licensing

- The project is MIT. Allowed dependency licences are listed in `deny.toml` (MIT, Apache-2.0, ISC, BSD, Zlib, Unicode-3.0, CC0, plus OFL-1.1 / UFL-1.0 for egui's bundled fonts). `cargo deny check` must pass.
- Never copy or port code from GPL, LGPL or AGPL projects (Vital, Surge XT, Dexed, Odin 2, HexoSynth/synfx-dsp or others). Papers, manuals and documentation are fine; cite them in `docs/dsp/`.
- Check licences per file, function or module, not per repo. Repos that look permissive but mix in copyleft: JUCE modules (AGPLv3 since JUCE 9), DaisySP's LGPL submodule, chowdsp's DSP modules, GPL-declared Faust functions, `sst-basic-blocks` outside its MIT-marked headers, valib's plugins, nih-plug's `plugins/`. Code with no licence file (e.g. Signalsmith's ADC 2021 reverb example) is reference-only.
- Ported source may only carry the licences listed in the 2026-10-19 note of `docs/adr/0004-licensing.md`. Every port records upstream URL, commit, licence and copyright in `THIRD_PARTY_NOTICES.md` and in `docs/dsp/<module>.md`.

## Done means

- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`, `cargo deny check`, `cargo xtask check-layers`, `cargo xtask wasm-check` all pass.
- Once they exist: `cargo xtask golden` passes (DSP changes that alter output update goldens deliberately with `cargo xtask golden --update`, and the commit message says why), and plugin changes pass `cargo xtask validate` (clap-validator).
- Performance-sensitive changes include before/after numbers from `enth bench` or the relevant benchmark.

## Research before every milestone (mandatory)

- Before starting any milestone in `docs/roadmap.md` (P0, P1, R1–R5, S1–S4), run a research session per `docs/research-process.md`: `_local/sessions/<YYYY-MM-DD>-<slug>/` with `brief.md`, one verbatim report per sub-researcher, and `summary.md`.
- Never edit a sub-report after it's written; put corrections in `summary.md`.
- Don't write milestone code until the owner has read the summary and the docs are updated.
- Don't rely on training data for versions, APIs or project health — check current sources and date them.

## Choosing libraries

- Apply the criteria in `docs/research-process.md#library-choice-criteria`: licence and real-time gates first, then activity, maintainers, adoption, docs, AI familiarity, platforms, exit cost.
- Prefer reusing permissively licensed DSP (reverb, compression, etc.) over writing it from scratch: usually by porting it into `enth-dsp` behind our own `Module` implementation, and as a crate dependency only when the crate passes the gates.
- Ask before adding a dependency that pulls in a C/C++ toolchain, a system library, or a licence not in `deny.toml`.

## Fast feedback loops

- Prefer loops that avoid rebuilding the plugin: test DSP through `enth render` / `enth describe` and golden tests; iterate on the editor in a standalone egui dev window.
- Keep incremental builds fast (fast linker, sensible profiles); hot-reload tooling is a nice-to-have to evaluate in a research session, not a requirement.

## Working style

- Small steps; after each task, report results against its acceptance criteria and stop.
- Prefer measured facts over assumptions. When a decision changes, update the ADR in `docs/adr/`.
