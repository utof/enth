# Phase 0 — spikes and skeleton

**Goal: answer the four risky questions with code and numbers before building features.** Can the plugin shell do what we need? Which native UI toolkit is cheapest on your machine? Is the engine fast and deterministic enough? Does the scoring metric agree with your ears?

Time box: about 3–4 weeks (the tasks below add up to roughly 15–17 working days). Do the tasks in order. After each task, the agent stops, reports results against the acceptance criteria, and waits.

Spike code lives in `spikes/` as workspace members named `spike-*`. They are exempt from the layer rules and are not shipped. Real product crates start in Phase 1, except the workspace skeleton built here.

## Task 0 — Research session (1 day)

Follow [research-process.md](research-process.md). Session folder: `_local/sessions/<today>-phase-0-spikes/`. Sub-researchers, in parallel:

1. **Plugin shell today:** nice-plug's current version, API, egui adapter (repaint behaviour, editor API changes), parameter rescan/renaming, state persistence, VST3 export; clack and clap-wrapper status; known REAPER/Bitwig/Ardour issues on Linux.
2. **Native UI in plugins:** egui and vizia adapters (versions, open issues, repaint gating patterns, HiDPI and keyboard focus inside hosts), plus anything new since 2026-10-18.
3. **Engine and determinism:** cross-platform float determinism (x86-64, aarch64, wasm32), denormal handling in Rust, Criterion/hyperfine practice, fast-iteration options (fast linkers, Cranelift backend, hot-reload tools).
4. **Metric and search:** permissive Rust crates for STFT/mel/MFCC and CMA-ES; listening-test design for 30 pairs.
5. **Workspace and CI:** current best practice for `[workspace.lints]`, cargo-deny config, xtask patterns, GitHub Actions caching for Rust.

**Acceptance:** `brief.md`, one verbatim report per sub-researcher, and `summary.md` exist; you've read the summary; any doc changes it recommends are made before Task 2.

## Task 1 — Machine setup (½ day, you + agent)

On Linux Mint (X11, NVIDIA proprietary driver):

```bash
# Rust toolchain manager (skip if rustup is already installed)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
# Build deps for plugin GUIs and audio (list from nih-plug's CI; install anything else an error names)
sudo apt-get install -y build-essential pkg-config libasound2-dev libgl-dev libjack-dev libx11-xcb-dev \
  libxcb1-dev libxcb-dri2-0-dev libxcb-icccm4-dev libxcursor-dev libxkbcommon-dev libxcb-shape0-dev libxcb-xfixes0-dev
# Measurement tools
sudo apt-get install -y linux-tools-common linux-tools-$(uname -r) heaptrack valgrind smem sysstat powertop hyperfine
# Cargo tools
cargo install --locked cargo-deny
cargo install --locked wasmtime-cli
```

- The exact Rust version and the wasm32 targets come from `rust-toolchain.toml` (Task 2); rustup installs them automatically the first time you run `cargo` in the repo.
- Install `clap-validator` from its releases page: <https://github.com/free-audio/clap-validator>.
- CLAP plugins on Linux are found in `~/.clap`. In REAPER: *Options → Preferences → Plug-ins → CLAP*, make sure `~/.clap` is scanned, then *Re-scan*. Do the same in your second host (Bitwig or Ardour).
- Record versions in `docs/spikes/environment.md`: OS, kernel, NVIDIA driver, REAPER and second-host versions, rustup version.

**Acceptance:** `environment.md` exists; `cargo deny --version` and `wasmtime --version` work; both hosts scan `~/.clap`.

## Task 2 — Repo skeleton and CI (1 day)

**Steps**

1. `git init` in `enth/` if needed; `.gitignore` with `/target`, `*.wav` (except under `tests/golden/`), `.venv/`, and `_local/` (unless you decide to commit research sessions — ask the owner).
2. Copy `docs/AGENTS.md` to `AGENTS.md`. Create `CLAUDE.md` containing exactly: `Read AGENTS.md and docs/README.md before any task.`
3. Root `Cargo.toml`:

    ```toml
    [workspace]
    resolver = "3"
    members = ["crates/*", "xtask"]   # add "spikes/*" and "tools/*" when the first one exists

    [workspace.package]
    version = "0.0.0"
    edition = "2024"
    license = "MIT"
    publish = false

    [workspace.lints.rust]
    unsafe_code = "deny"              # shell crates that need FFI add #![allow(unsafe_code)]

    [workspace.lints.clippy]
    all = { level = "warn", priority = -1 }
    ```

4. Each crate's `Cargo.toml` inherits: `version.workspace = true`, `edition.workspace = true`, `license.workspace = true`, `publish.workspace = true`, and `[lints] workspace = true`.
5. `rust-toolchain.toml` pinned to the current stable version, with `components = ["rustfmt", "clippy"]` and `targets = ["wasm32-unknown-unknown", "wasm32-wasip1"]`.
6. `.cargo/config.toml` with `[alias] xtask = "run --package xtask --"`.
7. `LICENSE` (MIT). `deny.toml` allowing MIT, Apache-2.0, ISC, BSD-2-Clause, BSD-3-Clause, Zlib, Unicode-3.0, CC0-1.0, plus `OFL-1.1` and `LicenseRef-UFL-1.0` (the fonts egui bundles by default).
8. Empty library crates: `crates/{dsp,params,patch,engine,render,analysis}`, plus the `xtask` binary.
9. `xtask/layers.toml` — the single source of truth for layers:

    ```toml
    [layers]
    enth-dsp = 0
    enth-params = 0
    enth-patch = 1
    enth-engine = 2
    enth-render = 3
    enth-analysis = 3
    enth-challenge = 4
    enth-solver = 5
    enth-session = 6
    enth-ui = 7
    enth-plugin = 8
    enth-cli = 8

    [core]
    max_layer = 5          # layers 0..=5 must build for wasm32 and do no I/O

    [exempt]
    packages = ["xtask", "enth-test-host"]
    prefixes = ["spike-"]
    ```

10. `xtask` commands: `check-layers` (every workspace member is layered or exempt; dependencies only go to strictly lower layers) and `wasm-check` (builds every member with layer ≤ `core.max_layer` for `wasm32-unknown-unknown`). Other xtask commands are added in the task that first needs them.
11. A GitHub Actions workflow on `ubuntu-latest`: install the Task 1 apt packages, then fmt, clippy `-D warnings`, test, `cargo deny check`, `cargo xtask check-layers`, `cargo xtask wasm-check`. **You** create the remote repository (GitHub, or Codeberg with the workflow ported to Forgejo Actions) and push.

**Acceptance:** all commands pass locally; `cargo xtask check-layers` fails on a deliberately added upward dependency (then remove it); CI passes after you push.

## Task 3 — Shell spike on nice-plug (3–4 days)

Build `spikes/spike-shell` (package `spike-shell`): an 8-voice synth with one saw oscillator, a one-pole low-pass, an amp envelope and smoothed gain/cutoff. Start from nice-plug's template (<https://codeberg.org/RustAudio/nice-plug-template>). Wire nice-plug's bundler into our xtask so `cargo xtask bundle spike-shell` works, and add `cargo xtask install` (copies the bundle to `~/.clap`). Build debug bundles with nice-plug's `assert_process_allocs` feature.

Check the **must-haves** for our design ([design.md §4](design.md#4-host-parameters-macros--automation-slots)):

| # | Must-have | How to check |
| --- | --- | --- |
| a | 72 fixed host params (8 macros + 64 slots) with stable IDs | Automate a macro and a slot in REAPER; save, reload the project |
| b | Slot renaming (CLAP param rescan) | Assign slot 1 to "cutoff" at runtime; does REAPER show the new name after rescan? If not, record it — "Slot 1" is an acceptable fallback |
| c | An arbitrary patch blob (JSON) saved and restored with the project | Change cutoff, save, reopen REAPER |
| d | CLAP polyphonic modulation on a macro | Bitwig is the reference host for poly-mod; record REAPER's behaviour |
| e | VST3 export builds and loads | Load the `.vst3` in REAPER |
| f | `clap-validator validate` passes | Run on the bundle |
| g | Builds on stable Rust | No nightly features enabled |
| h | Runs in a second Linux host | Load, play, automate, save/reload in Bitwig or Ardour |

**Decision rule** (same as [ADR-0001](adr/0001-plugin-shell.md)): if (a), (c), (e), (f) or (g) can't be made to work within two days, repeat the spike on `clack` + `clap-wrapper` and compare. (b) and (d) failing is acceptable if recorded; (h) failing needs a note on whether the host or our code is at fault.

**Acceptance:** results table in `docs/spikes/shell.md`; ADR-0001 updated with the findings.

## Task 4 — UI toolkit spike (4–5 days)

Follow [benchmarks.md](benchmarks.md): build the **reference panel** twice, as `spike-ui-egui` (nice-plug-egui, with repaint gating) and `spike-ui-vizia` (vizia-plug), on top of the Task 3 shell. Optional third arm: NovoNotes' WRAC template with a WebView panel, cloned *outside* the workspace, only to have your own numbers.

1. Build `tools/test-host` (package `enth-test-host`): a minimal CLAP host on `clack-host` that loads a `.clap`, processes audio in a loop, opens/closes the editor N times, injects parameter changes, and logs timings and PSS as JSON.
2. Run the measurement protocol in REAPER and in the test host.
3. Fill the results table in `docs/spikes/ui.md`; apply the decision rules and write a recommendation.

**Acceptance:** results table complete for each arm; a recommended toolkit; the numbers added to ADR-0002 (its status changes in Task 7).

## Task 5 — Engine speed and determinism spike (2 days)

In `spikes/engine` (package `spike-engine`): osc → filter → amp with 16 voices, an offline renderer, fixed block size 64, seed passed in. Engine tests use the `assert_no_alloc` crate around `process()`.

- Measure ×realtime per core for 2 s renders (1 voice, 16 voices) with `hyperfine` and a Criterion bench.
- Render the same patch twice → identical SHA-256.
- Build for `wasm32-wasip1`, run under `wasmtime`, and compare a feature summary (RMS per 10 ms frame, spectral centroid per frame) with native Linux; in CI also compare Windows. Record the maximum difference.

**Acceptance:** numbers in `docs/spikes/engine.md`; ≥ 100× realtime per core on the 1-voice template; same-platform hashes identical; cross-platform summaries within a tolerance you record.

## Task 6 — Metric and search spike (3 days)

In `spikes/metric` (package `spike-metric`):

1. A 10-parameter subtractive template (osc wave, detune, cutoff, resonance, filter-env amount, amp A/D/S/R).
2. Features: multi-resolution log-mel, amplitude envelope, spectral centroid track, f0.
3. Three candidate distances: (A) multi-resolution log-mel L1; (B) A + envelope + centroid-track composite; (C) MFCC-based.
4. A listening script: renders 30 pairs (target vs candidate at varying distances); you rate each 1–5 for similarity, blind to the metric. Compute Spearman correlation per distance.
5. CMA-ES (a small Rust implementation or a permissively licensed crate) recovering 20 random targets from random starts with each distance; record time and final score; listen to the results.

**Acceptance:** `docs/spikes/metric.md` with correlations, recovery success rate and time; a recommended `metric v1`; an ADR draft for the metric.

## Task 7 — Close Phase 0 (½ day)

- Fill the **G0 thresholds** in [roadmap.md](roadmap.md) and [benchmarks.md](benchmarks.md) with measured numbers.
- Mark ADR-0001 and ADR-0002 *Accepted* (or record the change of plan).
- Write `docs/spikes/summary.md`: what we learned, what changed in the plan, what's next for Phase 1.
