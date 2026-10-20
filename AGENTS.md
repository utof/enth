# AGENTS.md

Rules for AI coding agents working on `enth`. Read `docs/README.md` first, then the doc for the area you're touching. This is the single copy of the rules (`docs/AGENTS.md` only points here).

## Architecture (non-negotiable)

- Layers, lowest first: `dsp`, `params` (L0) → `patch` (L1) → `engine` (L2) → `render`, `analysis` (L3) → `challenge` (L4) → `solver` (L5) → `session` (L6) → `ui` (L7) → `plugin`, `cli`, `standalone`, `wasm`, `py` (L8). Depend only on strictly lower layers. `xtask/layers.toml` is the source of truth; `cargo xtask check-layers` must pass. Tooling (`xtask`, `enth-test-host`, `enth-golden`, `spike-*`) is exempt.
- Core crates (L0–L5) do no I/O, spawn no threads, read no clocks, use no global RNG, and must build for `wasm32-unknown-unknown` (`cargo xtask wasm-check`). Callers supply parallelism (e.g. the CLI passes a rayon-backed evaluator to the solver) and I/O.
- Audio thread: `engine::process` and everything it calls never allocate, free, lock, log, do I/O or panic. Engine tests wrap `process()` with the `assert_no_alloc` crate (gated on `debug_assertions`: release builds lack `AllocDisabler`); plugin debug builds also enable nice-plug's `assert_process_allocs`. Old plans go back on a return ring with a pending slot, so a full ring never drops one on the audio thread.
- **No `std::sync::mpsc` and no unbounded channels in `enth-engine`** (a clippy ban): an unbounded channel allocates a new block every 31 messages. Use `rtrb` rings. **Every ring and queue has an overflow test under `assert_no_alloc`**: a full command ring, every voice stolen, a plan swap while the return ring is full. Allocation sneaks in through rare paths (a full queue falling through to `push`, a clone that loses its reserved capacity), not the happy path.
- **No `Mutex` or `RwLock` in `enth-engine`**, not even uncontended: the nightly RTSan run does not catch an uncontended std lock. Use atomics and lock-free rings.
- **Determinism rules for DSP** (L0–L3; renders must be bit-exact on every platform, `docs/design.md` §5):
  - transcendentals (`sin`, `cos`, `exp`, `powf`, `tanh`…) only through the `libm` crate pinned with `=`, or our own approximations built from `+ − × ÷ sqrt`; never `std`'s;
  - no `mul_add` and no `algebraic_*` operations (including `wide`'s `mul_add`/`mul_sub`);
  - a fixed voice order and a fixed summation order; no runtime CPU dispatch between numerically different kernels;
  - flush denormals in code and **end voices below −100 dB**; never depend on, or set, FTZ/DAZ (no `no_denormals` crate: it does not build for wasm32);
  - scores that decide pass/fail use a scalar FFT path or a tolerance (`docs/design.md` §5), because `rustfft` picks its code path by CPU;
  - **the host's block size is not an input of `render()`.** The engine renders on a free-running 32-frame grid (the segment) through a one-segment FIFO, so any host block size gives the same bits; events take effect at the first segment boundary at or after their frame, and notes start and end on the grid. Never split a block at an event time. (A grid that restarts per call is why 48-sample blocks differed in the Phase 0 spike);
  - **seeds never depend on parameter values or on the patch hash.** A voice's seed is `derive(derive(render_seed, fnv1a64(module_id)), note_index)`, a global module's is `derive(render_seed, fnv1a64(module_id))`. Otherwise every knob move reseeds every voice, and finite-difference hints and search measure noise (a 0.1 % cutoff move shifted the RMS envelope by up to 6.6 dB with hash seeds, against 0.008 dB with a fixed seed). A module that wants random character gets an explicit `seed` parameter.
- The numeric bans (`std` transcendentals, `mul_add`, `algebraic_*`, `wide`'s fused ops) and the channel ban are clippy `disallowed-methods`. They live in the root `clippy.toml` next to the I/O bans and apply to every core crate (the channel ban is root-wide too: core crates spawn no threads, so they have no use for channels; shells opt out at the crate root with a reason). A crate-level `clippy.toml` *replaces* the root one rather than extending it, so a crate that adds one repeats the workspace bans.
- Every `#[allow(...)]` carries `reason = "…"` (the workspace enables `clippy::allow_attributes_without_reason`). Crates that need `unsafe` or I/O opt out at the crate root, with a reason.
- Every patch mutation is an `Op` from `enth-patch`. No other way to change a patch. Host automation applies `SetParam` without a history entry (it is an op, but not an undo step), and only when the host's normalised value differs from `to_norm` of the patch value (`docs/design.md` §4): plain → normalised → plain is not an exact round trip.
- Patches store **plain values in real units** (Hz, ms, dB, cents, %, octaves). Normalised 0–1 values exist only at host automation, UI gesture and ML boundaries.
- Persisted or emitted types (patch, challenge, lesson, CLI JSON) carry a version and have fixture tests. Changing one requires a migration or a version bump.
- A new module = `ParamSpec`s in the `patch` catalogue + implementation in `engine` + golden render test + a note in `docs/dsp/` + a lesson and an agent eval when the module is user-facing.
- Shells (`plugin`, `cli`, …) translate; they never contain product rules.
- All randomness is seeded and passed in, from the render seed, the module id and the note index (see the determinism rules above).
- Ported DSP follows the same contract: replace any upstream RNG (`rand()`, `std::mt19937`, `thread_rng`) with the engine's seeded PRNG (from the render seed and the module id, never the patch hash), or with a constant table where upstream's seeds are constants (the reverb); flush denormals in code, not through CPU flags; allocate only in `prepare`. Effects report zero latency unless an ADR says otherwise. No crates that wrap C/C++ in L0–L5, and no third-party plugin hosting in the engine (`docs/adr/0006-effects-strategy.md`).

## UI

- Native Rust UI: egui rendered with glow (see `docs/adr/0002-ui-toolkit.md`). No WebView, no second toolkit. egui's bundled fonts are opt-in through its `default_fonts` feature; their licence is `Ubuntu-font-1.0`.
- **Never open a GUI window on the real display.** Anything that creates a window or loads a plugin editor (the plugin in REAPER, an egui binary, a validator or host that opens editors, Carla) runs under `xvfb-run -a -s "-screen 0 1920x1080x24"` with Mesa software rendering. Why: the NVIDIA 595-series X driver crashed Xorg on 2026-10-19 after short-lived X clients ran on the real display (a known driver bug that plugin editors would trigger), which takes the whole desktop session and the DAW down. Real-GPU checks are owner-attended, with work saved (`docs/benchmarks.md`, two tiers).
- Repaint only on input, parameter change, or new data for a *visible* live value (knob rings, tiles, the scope and meters). No timers or work when the editor is closed or hidden. The editor asks for its next frame at `motion.animated_hz` = 30 Hz while visible live data changes, the scope included (lane ED2; `motion.scope_hz` = 60 is not used by the editor). Under baseview 0.4.1 a 60 Hz request gave about 30 fps and a 30 Hz request about 20 in Phase 0, so the rate on the real display is an open check (`docs/phase-1b.md`).
- Snapshot tests (`egui_kittest`) render only on Mesa's CPU Vulkan driver (lavapipe): `.cargo/config.toml` points the Vulkan loader at its ICD, so tests never load the NVIDIA driver; CI installs `mesa-vulkan-drivers`.
- Editor leak checks run in the test host (`enth-test-host editor-cycles`, which refuses to run outside Xvfb) against its no-editor baseline; `cargo xtask leak-test` runs it and instance churn on `enth-plugin` as a gate (it starts its own Xvfb; before merging an editor lane, and nightly). baseview 0.4.1 leaks one xkb keymap per editor open (about 61 of the 77 KB per cycle, `docs/spikes/test-host.md`); the vendored baseview under `vendor/` carries a one-line fix (about 16 KB per cycle is left, and the gate is 26 KB, `docs/benchmarks.md`), and the owner may report it upstream (ADR-0001 and ADR-0002 notes, 2026-10-19). heaptrack segfaults once a plugin opens a window: attribute editor leaks with `valgrind --leak-check=full`.
- The UI never blocks on the audio thread and never shares a lock with it; use atomics and lock-free rings.
- **Design tokens only** (`docs/design-system.md`): every colour, size, spacing, radius, stroke, font and repaint rate comes from `enth-ui`'s `theme` module. No raw values in widget code; need a new one, add a token **in your own area's file** under `crates/enth-ui/src/theme/` and no other: component sizes go in `sizes/<area>.rs` (one file per UI area; a new area is a new file plus its field in `sizes/mod.rs`), and `colors.rs`, `space.rs`, `text.rs`, `states.rs`, `motion.rs` and `glyph.rs` hold the rest, so concurrent lanes never edit the same token file. A new field needs a serde default, so older theme files still load. Keep the look minimal: flat, two accents, small scales.

## Plugin shell (nice-plug)

- **Never call `GuiContext::set_state` while the plugin is processing.** nice-plug would load that state on the audio thread. Patches load through state on the host's main thread, never through host automation (a preset browser that set about 150 parameters crashed VST3 hosts, nih-plug #92).
- **Never schedule GUI work (`execute_gui`) from `process()`:** the task may run inline inside it. The one thing `process()` may schedule is nice-plug's rate-limited background pump, through `execute_background` (its worker thread, never inline), and only while a compile is pending (`docs/session.md`, "Plan delivery").
- **nice-plug 0.4.2 does not re-run `initialize` (or `reset`) after a state load**, unlike nih-plug: the plugin learns of a load only through its persistent fields' `set` on the host main thread and keeps its engine. `Session::load_patch` sends the loaded patch's plan at once and marks it to forget the engine's remembered host slot values, so the loaded patch wins in the engine too (`docs/session.md`, `docs/design.md` §4).
- **`SAMPLE_ACCURATE_AUTOMATION = true`, always.** Without it, plugin renders stop matching CLI renders at large host blocks.
- **Read `VoiceID::id()`, never `id_or_fallback`** (it ignores the host's voice id).
- The session sits behind one mutex shared by the host main thread, the GUI thread and nice-plug's background worker (the pump task). The audio thread never touches it, and nothing is locked from `process()`. Enforce the two bans above with clippy `disallowed-methods` in `enth-plugin`'s `clippy.toml` (a crate-level file replaces the root one, so repeat the workspace bans).
- `NiceEguiApp::editor_closed()` fires only in the vendored fork. `EguiEditorState::is_open()` is reliable either way; treat every `build()` as a fresh open.
- Stock nice-plug never marks the project dirty; the fork's patch 6 adds `HostRequests::mark_dirty()`, which the plugin calls on the session's `PatchEdited`. `reaper-test`'s `dirty-flag` scenario checks it.
- **Vendored crates** (`vendor/`: nice-plug, nice-plug-core, nice-plug-egui, baseview and egui-baseview with our patches, behind `[patch.crates-io]`) are edited only by lane FK; any other lane that needs a change asks for it, and the coordinator edits `[patch.crates-io]`. Cargo caps lints in registry dependencies but not in path dependencies, so each vendored manifest ends with `[lints.rust] warnings = "allow"`, which gives these crates what a crates.io dependency gets: a warning from a newer toolchain cannot fail CI under `CARGO_BUILD_WARNINGS=deny`. A hard error from a toolchain bump still needs a patch. No source file is touched for the cap; `vendor/README.md` ("Lints") says how to build a crate you edit without it (`docs/architecture.md`, ADR-0001).

## CLI

- JSON output when stdout is not a TTY; every object has `"schema": "enth.<command>/<version>"`.
- Real units in and out. Stable exit codes (see `docs/agent-cli.md`). Errors include a `hint`.
- Output shape changes require a schema version bump and updated snapshot tests (`insta`).

## Licensing

- The project is MIT. Allowed dependency licences are listed in `deny.toml` (MIT, Apache-2.0, ISC, BSD incl. BSD-1-Clause, Zlib, Unicode-3.0, CC0), with scoped exceptions for egui's bundled fonts (OFL-1.1 and `Ubuntu-font-1.0`, `epaint_default_fonts` only) and BSL-1.0 (two Windows-only crates); see the 2026-10-19 notes in `docs/adr/0004-licensing.md`. `cargo deny check` must pass.
- Never copy or port code from GPL, LGPL or AGPL projects (Vital, Surge XT, Dexed, Odin 2, HexoSynth/synfx-dsp or others). Papers, manuals and documentation are fine; cite them in `docs/dsp/`. **Write DSP from cited papers, manuals and permissively licensed sources, never from memory of a GPL synth's code**: retyping a GPL synth's code from memory is still copying it.
- Check licences per file, function or module, not per repo. Repos that look permissive but mix in copyleft: JUCE modules (AGPLv3 since JUCE 9), DaisySP's LGPL submodule, chowdsp's DSP modules, GPL-declared Faust functions, `sst-basic-blocks` outside its MIT-marked headers, valib's plugins, nih-plug's `plugins/`. Code with no licence file (e.g. Signalsmith's ADC 2021 reverb example) is reference-only.
- Ported source may only carry the licences listed in the 2026-10-19 notes of `docs/adr/0004-licensing.md` (which now include BSL-1.0). Every port records upstream URL, commit, licence and copyright in `THIRD_PARTY_NOTICES.md` and in `docs/dsp/<module>.md`.

## Done means

- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test` (engine tests also run under `cargo nextest`, one process per test, so an `assert_no_alloc` abort fails one test; CI keeps `cargo test` for doctests), `cargo deny check`, `cargo xtask check-layers`, `cargo xtask wasm-check` all pass. Property tests (proptest) run in this gate with a fixed seed (`PROPTEST_RNG_SEED`, set in `.cargo/config.toml`'s `[env]`) and the default 256 cases, and `proptest-regressions/` is committed, so a red or green result is deterministic; random seeds and 10,000 cases run in a nightly job. CI sets `CARGO_BUILD_WARNINGS=deny`, so any warning (including a linker warning) fails the build. The toolchain is the pinned stable 1.99.1 in `rust-toolchain.toml`; never 1.99.0, which miscompiles x86_64 FFI `bool` returns.
- Performance budgets: `cargo xtask bench` runs the gungraun instruction budgets (needs valgrind and a `gungraun-runner` matching the pinned `gungraun`).
- `cargo xtask golden` passes (DSP changes that alter output update goldens deliberately with `cargo xtask golden --update`, which prints the cases it changed; the commit message lists them and says why), and plugin changes pass `cargo xtask validate [--package enth-plugin]` (the pinned clap-validator 0.4.1 on the debug bundle, which has `assert_process_allocs` on; `--release` checks the release one) and, locally, `cargo xtask reaper-test` (headless REAPER under Xvfb; a local gate, not CI).
- Plugin changes pass `cargo xtask host-render` (plugin-level golden renders through the test host; CI job `host-render`; `--update` only on purpose, and the commit says why). Plugin-level goldens never come from a REAPER offline render: REAPER renders depend on the host (stock nice-plug also asked REAPER to restart CLAP plugins about 30 ms into an offline render and dropped held notes; the fork's patch 9 turns that off for enth), and the test host is deterministic. For `enth-plugin` it then runs `enth-test-host cli-equal` (lane INT wired it in): the engine's own golden cases rendered through the plugin must give the engine's hashes at host blocks 1, 64, 100 and 4096, so CLI = plugin is checked without new hash files. Changes to the editor (`enth-ui`'s editor, `enth-plugin`'s editor, `vendor/`) also pass `cargo xtask leak-test` (editor-cycles at most 26 KB per cycle with threads constant, instance churn flat; under its own Xvfb, a few minutes).
- **Gates (P1b).** A lane runs `scripts/gate.sh lane <worktree>` before it reports: fmt, workspace clippy, check-layers, wasm-check, and the tests of the crates the change touches plus their dependants (`cargo xtask affected`). The coordinator runs `scripts/gate.sh full <worktree>` before a merge: the same with the whole workspace's tests, plus `cargo deny`, golden, `host-render` and `validate`. Both print the commit they checked and end with `GATE PASSED`. Judge every gate step by its exit status only: in `cmd | tail -1 && echo ok` the status is `tail`'s, so a failing `cmd` still prints ok (the scripts send each step's output to a log file for this reason). Run long commands in the foreground with an explicit timeout above their measured length, or in the background with a wake-up that checks on them (backgrounded agents can go idle and never resume).
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

- At most **2 heavy builds at once** (nice-plug, baseview, egui, clack-host): this laptop has little free RAM. Pair each heavy lane with a light one, and let harness lanes reuse the coordinator's release-bundle cache instead of rebuilding the plugin. Give each git worktree its own `CARGO_TARGET_DIR` on ext4.
- A lane writes only its own `docs/spikes/<lane>.md`, and does not touch files another running lane owns.
- Keep scratch work in persistent storage outside the repo (for example `~/code/enth-scratch/<session>/<lane>/`), never only in `/tmp`, with a running `notes.md`: a crash or reboot loses `/tmp`, and the notes are what a replacement agent resumes from.

## Working style

- Small steps; after each task, report results against its acceptance criteria and stop.
- Prefer measured facts over assumptions. When a decision changes, update the ADR in `docs/adr/`.
