# AGENTS.md

Rules for AI coding agents working on `enth`. Read `docs/README.md` first, then the doc for the area you're touching. This is the single copy of the rules (`docs/AGENTS.md` only points here).

## Architecture (non-negotiable)

- Layers, lowest first: `dsp`, `params` (L0) → `patch` (L1) → `engine` (L2) → `render`, `analysis` (L3) → `challenge` (L4) → `solver` (L5) → `session` (L6) → `ui` (L7) → `plugin`, `cli`, `standalone`, `wasm`, `py` (L8). Depend only on strictly lower layers. `xtask/layers.toml` is the source of truth; `cargo xtask check-layers` must pass. Tooling (`xtask`, `enth-test-host`, `enth-golden`, `spike-*`) is exempt.
- Core crates (L0–L5) do no I/O, spawn no threads, read no clocks, use no global RNG, and must build for `wasm32-unknown-unknown` (`cargo xtask wasm-check`). Callers supply parallelism (e.g. the CLI passes a rayon-backed evaluator to the solver) and I/O.
- Audio thread: `engine::process` and everything it calls never allocate, lock, log, do I/O or panic. Engine tests wrap `process()` with the `assert_no_alloc` crate (gated on `debug_assertions`: release builds lack `AllocDisabler`); plugin debug builds also enable nice-plug's `assert_process_allocs`.
- **No `Mutex` or `RwLock` in `enth-engine`**, not even uncontended: the nightly RTSan run does not catch an uncontended std lock. Use atomics and lock-free rings.
- **Determinism rules for DSP** (L0–L3; renders must be bit-exact on every platform, `docs/design.md` §5):
  - transcendentals (`sin`, `cos`, `exp`, `powf`, `tanh`…) only through the `libm` crate pinned with `=`, or our own approximations built from `+ − × ÷ sqrt`; never `std`'s;
  - no `mul_add` and no `algebraic_*` operations (including `wide`'s `mul_add`/`mul_sub`);
  - a fixed voice order and a fixed summation order; no runtime CPU dispatch between numerically different kernels;
  - flush denormals in code and **end voices below −100 dB**; never depend on, or set, FTZ/DAZ (no `no_denormals` crate: it does not build for wasm32);
  - scores that decide pass/fail use a scalar FFT path or a tolerance (`docs/design.md` §5), because `rustfft` picks its code path by CPU;
  - block size and the 32-sample control grid are inputs of `render()`: blocks of 32 and 64 give the same hash, 48 does not. A host-driven engine re-chunks the host's buffers onto a fixed internal block, and notes start and end on the grid.
- The numeric bans (`std` transcendentals, `mul_add`, `algebraic_*`, `wide`'s fused ops) are clippy `disallowed-methods`. When `enth-dsp`/`enth-engine` get code they go in the root `clippy.toml` next to the I/O bans. A crate-level `clippy.toml` *replaces* the root one rather than extending it, so a crate that adds one repeats the workspace bans.
- Every `#[allow(...)]` carries `reason = "…"` (the workspace enables `clippy::allow_attributes_without_reason`). Crates that need `unsafe` or I/O opt out at the crate root, with a reason.
- Every patch mutation is an `Op` from `enth-patch`. No other way to change a patch.
- Patches store **plain values in real units** (Hz, ms, dB, cents, %, octaves). Normalised 0–1 values exist only at host automation, UI gesture and ML boundaries.
- Persisted or emitted types (patch, challenge, lesson, CLI JSON) carry a version and have fixture tests. Changing one requires a migration or a version bump.
- A new module = `ParamSpec`s in the `patch` catalogue + implementation in `engine` + golden render test + a note in `docs/dsp/` + a lesson and an agent eval when the module is user-facing.
- Shells (`plugin`, `cli`, …) translate; they never contain product rules.
- All randomness is seeded and passed in.
- Ported DSP follows the same contract: replace any upstream RNG (`rand()`, `std::mt19937`, `thread_rng`) with the engine's seeded PRNG; flush denormals in code, not through CPU flags; allocate only in `prepare`. Effects report zero latency unless an ADR says otherwise. No crates that wrap C/C++ in L0–L5, and no third-party plugin hosting in the engine (`docs/adr/0006-effects-strategy.md`).

## UI

- Native Rust UI: egui rendered with glow (see `docs/adr/0002-ui-toolkit.md`). No WebView, no second toolkit. egui's bundled fonts are opt-in through its `default_fonts` feature; their licence is `Ubuntu-font-1.0`.
- **Never open a GUI window on the real display.** Anything that creates a window or loads a plugin editor (the plugin in REAPER, an egui binary, a validator or host that opens editors, Carla) runs under `xvfb-run -a -s "-screen 0 1920x1080x24"` with Mesa software rendering. Why: the NVIDIA 595-series X driver crashed Xorg on 2026-10-19 after short-lived X clients ran on the real display (a known driver bug that plugin editors would trigger), which takes the whole desktop session and the DAW down. Real-GPU checks are owner-attended, with work saved (`docs/benchmarks.md`, two tiers).
- Repaint only on input, parameter change, or new data for a *visible* scope. No timers or work when the editor is closed or hidden. Scopes request their next frame at `motion.scope_hz` = 60 Hz: under baseview 0.4.1 that gives about 30 fps, while a 30 Hz request gives about 20.
- Snapshot tests (`egui_kittest`) render only on Mesa's CPU Vulkan driver (lavapipe): `.cargo/config.toml` points the Vulkan loader at its ICD, so tests never load the NVIDIA driver; CI installs `mesa-vulkan-drivers`.
- Editor leak checks run in the test host (`enth-test-host editor-cycles`, which refuses to run outside Xvfb) against its no-editor baseline. baseview 0.4.1 leaks one xkb keymap per editor open (about 61 of the 77 KB per cycle, `docs/spikes/test-host.md`); the fix goes upstream; don't carry a patched baseview until we decide how we depend on it (record that in ADR-0002). heaptrack segfaults once a plugin opens a window: attribute editor leaks with `valgrind --leak-check=full`.
- The UI never blocks on the audio thread and never shares a lock with it; use atomics and lock-free rings.
- **Design tokens only** (`docs/design-system.md`): every colour, size, spacing, radius, stroke, font and repaint rate comes from `enth-ui`'s `theme` module. No raw values in widget code; need a new one, add a token. Keep the look minimal: flat, two accents, small scales.

## CLI

- JSON output when stdout is not a TTY; every object has `"schema": "enth.<command>/<version>"`.
- Real units in and out. Stable exit codes (see `docs/agent-cli.md`). Errors include a `hint`.
- Output shape changes require a schema version bump and updated snapshot tests (`insta`).

## Licensing

- The project is MIT. Allowed dependency licences are listed in `deny.toml` (MIT, Apache-2.0, ISC, BSD incl. BSD-1-Clause, Zlib, Unicode-3.0, CC0), with scoped exceptions for egui's bundled fonts (OFL-1.1 and `Ubuntu-font-1.0`, `epaint_default_fonts` only) and BSL-1.0 (two Windows-only crates); see the 2026-10-19 notes in `docs/adr/0004-licensing.md`. `cargo deny check` must pass.
- Never copy or port code from GPL, LGPL or AGPL projects (Vital, Surge XT, Dexed, Odin 2, HexoSynth/synfx-dsp or others). Papers, manuals and documentation are fine; cite them in `docs/dsp/`. **Write DSP from cited papers, manuals and permissively licensed sources, never from memory of a GPL synth's code**: retyping a GPL synth's code from memory is still copying it.
- Check licences per file, function or module, not per repo. Repos that look permissive but mix in copyleft: JUCE modules (AGPLv3 since JUCE 9), DaisySP's LGPL submodule, chowdsp's DSP modules, GPL-declared Faust functions, `sst-basic-blocks` outside its MIT-marked headers, valib's plugins, nih-plug's `plugins/`. Code with no licence file (e.g. Signalsmith's ADC 2021 reverb example) is reference-only.
- Ported source may only carry the licences listed in the 2026-10-19 note of `docs/adr/0004-licensing.md`. Every port records upstream URL, commit, licence and copyright in `THIRD_PARTY_NOTICES.md` and in `docs/dsp/<module>.md`.

## Done means

- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`, `cargo deny check`, `cargo xtask check-layers`, `cargo xtask wasm-check` all pass. CI sets `CARGO_BUILD_WARNINGS=deny`, so any warning (including a linker warning) fails the build. The toolchain is the pinned stable 1.99.1 in `rust-toolchain.toml`; never 1.99.0, which miscompiles x86_64 FFI `bool` returns.
- Once they exist: `cargo xtask golden` passes (DSP changes that alter output update goldens deliberately with `cargo xtask golden --update`, and the commit message says why), and plugin changes pass `cargo xtask validate` (clap-validator) and, locally, `cargo xtask reaper-test` (headless REAPER under Xvfb; a local gate, not CI).
- Plugin changes pass `cargo xtask host-render` (plugin-level golden renders through the test host; CI job `host-render`; `--update` only on purpose, and the commit says why). Plugin-level goldens never come from a REAPER offline render: REAPER deactivates, reactivates and resets CLAP plugins about 30 ms into an offline render and drops held notes.
- Performance-sensitive changes include before/after numbers from `enth bench` or the relevant benchmark: instruction counts (callgrind/gungraun), not wall-clock time, which is information only (`docs/benchmarks.md`).

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
- Keep incremental builds fast. lld is already rustc's default linker; optimise named DSP crates, not all dependencies, and stay on stable. Hot reload is out for Phase 0 (it cannot patch a plugin loaded in a DAW): the loop is a standalone egui dev window plus a rebuild of about 1.5 s.

## Privacy

- Never send the owner's email, name or other personal details to any service: not in User-Agent or other headers, URLs or payloads. Use a generic User-Agent (`enth-research/0.1`) where an API asks for contact info.

## Working in parallel (Phase 0 lanes and any concurrent agents)

- At most **2 heavy builds at once** (nice-plug, baseview, egui, clack-host): this laptop has little free RAM. Give each git worktree its own `CARGO_TARGET_DIR` on ext4.
- A lane writes only its own `docs/spikes/<lane>.md`, and does not touch files another running lane owns.
- Keep scratch work in persistent storage outside the repo (for example `~/code/enth-scratch/<session>/<lane>/`), never only in `/tmp`, with a running `notes.md`: a crash or reboot loses `/tmp`, and the notes are what a replacement agent resumes from.

## Working style

- Small steps; after each task, report results against its acceptance criteria and stop.
- Prefer measured facts over assumptions. When a decision changes, update the ADR in `docs/adr/`.
