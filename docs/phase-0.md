# Phase 0 — spikes and skeleton

**Goal: answer the four risky questions with code and numbers before building features.** Can the plugin shell do what we need? Is the egui editor cheap and reliable on your machine? Is the engine fast and deterministic enough? Does the scoring metric agree with your ears?

Time box: about 2 weeks of calendar time (2026-10-19: was 3–4 weeks in the first plan). After a one-day skeleton, the work runs as **parallel lanes**. Each lane stops at its own acceptance criteria and writes its own results into `docs/spikes/<lane>.md` (and nothing else under `docs/spikes/`). Nothing in the lanes waits for the owner; the owner's real-display and listening checks are collected in [spikes/owner-checklist.md](spikes/owner-checklist.md) and feed the final numbers.

**Status (2026-10-19): Phase 0 is done, except the owner's checks.** Every lane is merged; results, the G0 table and actual against estimated effort are in [spikes/summary.md](spikes/summary.md).

Spike code lives in `spikes/` as workspace members named `spike-*`. They are exempt from the layer rules and are not shipped. Real product crates start in Phase 1, except the workspace skeleton built here.

## Lanes

Research summary: `_local/sessions/2026-10-19-phase-0-spikes/summary.md` (decisions, evidence, owner checklist).

| Lane | Task | What | Depends on | State (2026-10-19) |
| --- | --- | --- | --- | --- |
| S | 2 | Skeleton + CI | — | **Done**, merged 2026-10-19, CI green |
| A | 3 | Shell spike: `spike-shell`, bundle/install, `reaper-test`, clap-validator CI | S | **Done**, merged 2026-10-19 — [spikes/shell.md](spikes/shell.md) |
| B | 5 | Engine + determinism: cross-platform hashes, FTZ/lanes/subnormal tests, wasm vs native, Windows CI, instruction-count bench | S | **Done**, merged 2026-10-19 — [spikes/engine.md](spikes/engine.md); Windows hashes green in CI, aarch64 manual only |
| C | 6 | Metric + search: features, three distances, CMA-ES, solve/bench, `listen` CLI, ADR-0007 | S | **Done**, merged 2026-10-19 — [spikes/metric.md](spikes/metric.md); the listening sessions are on the owner checklist |
| U1 | 4 | Design tokens (`enth-ui::theme`), widgets, the "Lanes" reference panel, snapshot tests, `spike-ui-plugin` | S, A | **Done**, merged 2026-10-19 — [spikes/ui.md](spikes/ui.md) |
| H | 4 | Minimal clack-host test host for CI, leak and CPU measurement | S | **Done**, merged 2026-10-19 — [spikes/test-host.md](spikes/test-host.md) |
| U2 | 4 | The panel in REAPER (Xvfb) plus owner real-display checks; decide the G0 UI thresholds | U1, H | **Done by other lanes:** U1 and H made the REAPER and test-host measurements under Xvfb ([ui.md](spikes/ui.md), [test-host.md](spikes/test-host.md)); the real-display items moved to the [owner checklist](spikes/owner-checklist.md); lane Z set the G0 UI statuses ([benchmarks.md](benchmarks.md#ui-thresholds)) |
| P | — | Agent-loop probe: an agent tries 5 text goals through `describe`/`check` over lane C's renderer | C | **Done** (part 1 and a 3-agent trial), merged 2026-10-19 — [spikes/agent-probe.md](spikes/agent-probe.md) |
| Z | 7 | Close: measured thresholds into the gates, ADR statuses, actual vs estimated effort | all | **Done** 2026-10-19 — [spikes/summary.md](spikes/summary.md) |

Tasks 0 and 1 are done. Rules for every lane:

- Branch from the skeleton and write only your own `docs/spikes/<lane>.md` (the Task 7 lane writes `docs/spikes/summary.md`).
- At most 2 heavy builds (nice-plug, baseview, egui, clack-host) at the same time on this machine.
- Everything that opens a window or loads a plugin editor runs under Xvfb (see [AGENTS.md](../AGENTS.md)).
- Keep a running `notes.md` in persistent scratch storage outside the repo; a crash or reboot loses `/tmp`.

## Task 0 — Research session (1 day)

**Done 2026-10-19.**

Follow [research-process.md](research-process.md). Session folder: `_local/sessions/2026-10-19-phase-0-spikes/`. Sub-researchers, in parallel:

1. **Plugin shell today:** nice-plug's current version, API, egui adapter (repaint behaviour, editor API changes), parameter rescan/renaming, state persistence, VST3 export; clack and clap-wrapper status; known REAPER/Bitwig/Ardour issues on Linux.
2. **Native UI in plugins:** egui and vizia adapters (versions, open issues, repaint gating patterns, HiDPI and keyboard focus inside hosts), plus anything new since 2026-10-18.
3. **Engine and determinism:** cross-platform float determinism (x86-64, aarch64, wasm32), denormal handling in Rust, Criterion/hyperfine practice, fast-iteration options (fast linkers, Cranelift backend, hot-reload tools).
4. **Metric and search:** permissive Rust crates for STFT/mel/MFCC and CMA-ES; listening-test design for 30 pairs.
5. **Workspace and CI:** current best practice for `[workspace.lints]`, cargo-deny config, xtask patterns, GitHub Actions caching for Rust.

The session also ran a dev-loop researcher (06) and a plan-risks researcher (07), seven in all. [research-process.md](research-process.md) now says to scale the number of sub-researchers to the milestone's risk.

**Acceptance:** `brief.md`, one verbatim report per sub-researcher, and `summary.md` exist; you've read the summary; any doc changes it recommends are made before Task 2. Status: the summary exists and its doc changes were applied on 2026-10-19.

## Task 1 — Machine setup (½ day, you + agent)

**Done 2026-10-19**; the snapshot is [spikes/environment.md](spikes/environment.md).

On Linux Mint (X11, NVIDIA proprietary driver). The research found that the machine, not the code, was the biggest risk, so this task now records what was fixed and what is installed:

- **The repo is on ext4** (`/home`), not on the NTFS/FUSE USB disk. File-metadata calls dropped from about 5 ms to about 3 µs, which is most of why cargo, rust-analyzer and file watchers were slow.
- **Native REAPER 7.82** is installed in portable mode under `~/apps/reaper-7.82/` (60-day full evaluation). The Wine/Windows REAPER cannot load Linux plugins.
- **The reboot fixed the NVIDIA driver/library mismatch.** The NVIDIA 595-series X driver still has a bug: it crashed Xorg on 2026-10-19 after short-lived X clients ran on the real display, matching NVIDIA bug 6205955 (no fix yet). Plugin editors open and close windows constantly, so they would trigger it. So agents never open windows on the real display (see Task 4).
- **Second hosts:** Carla 2.5.8 from apt hosts **VST3 only** (CLAP support exists only on Carla's unreleased main branch). Ardour hosts VST3 only and is not installed. Bitwig is the only CLAP host with poly-mod; it is not installed. A Bitwig trial is optional and only for the poly-mod check.

```bash
# Rust toolchain manager (skip if rustup is already installed)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
# Build deps for plugin GUIs and audio (nih-plug's CI list). Use libjack-jackd2-dev: libjack-dev would remove libjack-jackd2-0 and libasound2-plugins:i386.
sudo apt-get install -y build-essential pkg-config libasound2-dev libgl-dev libjack-jackd2-dev libx11-xcb-dev \
  libxcb1-dev libxcb-dri2-0-dev libxcb-icccm4-dev libxcursor-dev libxkbcommon-dev libxcb-shape0-dev libxcb-xfixes0-dev
# Headless GUI testing (Xvfb + Mesa software rendering), JACK for Carla, a VST3 second host
sudo apt-get install -y xvfb mesa-vulkan-drivers pipewire-jack carla
# Measurement tools (apt's hyperfine is 1.18, without the 2.0 instruction metrics)
sudo apt-get install -y linux-tools-common linux-tools-$(uname -r) heaptrack valgrind smem sysstat powertop hyperfine
# Cargo tools (match CI)
cargo install --locked cargo-deny --version 0.20.2
```

- The exact Rust version (1.99.1) and the wasm32 targets come from `rust-toolchain.toml`; rustup installs them the first time you run `cargo` in the repo.
- **wasmtime:** download the prebuilt release tarball from <https://github.com/bytecodealliance/wasmtime/releases> and check its sha256. `cargo install wasmtime-cli` fails on Rust 1.95 (the latest release needs 1.96) and takes about 12 minutes and 2 GB of RAM when it works. Node (already installed) also runs `wasm32-unknown-unknown` modules.
- **clap-validator 0.4.1:** use the official release binary from <https://github.com/free-audio/clap-validator/releases> (the zip contains a tar.gz with a different inner name; ignore that). It runs headless.
- **CMake** for Steinberg's VST3 validator: `uv tool run --from cmake cmake`.
- **`perf` is blocked** (`kernel.perf_event_paranoid` is 4). Instruction counts (callgrind) are the load-independent measurement. The owner may run `sudo sysctl kernel.perf_event_paranoid=1` for quiet-machine timing runs.
- **REAPER headless settings** (the `reaper-test` command writes these into a throwaway copy; never point automation at your own REAPER config): `linux_audio_mode=2` (Dummy Audio), `linux_audio_srate=48000`, `clap_path_linux-x86_64=` and `vstpath=` for the plugin folders. With them set, no scan prompt or evaluation dialog blocks a scripted run.
- CLAP plugins on Linux are found in `~/.clap`, VST3 in `~/.vst3`. In REAPER: *Options → Preferences → Plug-ins → CLAP*, make sure `~/.clap` is scanned, then *Re-scan*.
- Record versions in `docs/spikes/environment.md`: OS, kernel, NVIDIA driver, REAPER and Carla versions, rustup version, filesystem, free RAM and swap.

**Acceptance:** `environment.md` exists; `cargo deny --version` and `wasmtime --version` work; REAPER (CLAP and VST3) and Carla (VST3) load plugins from `~/.clap` and `~/.vst3`. Status: done; [environment.md](spikes/environment.md) written by lane Z. It found that the `cargo-deny` on `PATH` is 0.19.4, not the 0.20.2 CI pins, and that `wasmtime` lives in scratch, not on `PATH`.

## Task 2 — Repo skeleton and CI (1 day)

**Lane S. Done: merged 2026-10-19, CI green.**

**Steps** (as built; the files themselves are the source of truth)

1. `git init`; `.gitignore` with `/target`, `*.wav`, `.venv/`, and `/_local/*` except `!/_local/sessions/`. Research sessions are committed; `_local/reference/` and anything else under `_local/` never is. gitignore has no trailing comments, so keep comments on their own lines (a trailing `# …` on the `*.wav` line left WAVs tracked). Before the repo goes public, review committed briefs for personal details such as hardware and driver state.
2. `AGENTS.md` at the repo root is the single copy (`docs/AGENTS.md` only points to it). `CLAUDE.md` contains exactly: `Read AGENTS.md and docs/README.md before any task.`
3. Root `Cargo.toml` (abridged; see the file for the full lint set):

    ```toml
    [workspace]
    resolver = "3"
    members = ["crates/*", "xtask", "spikes/*"]   # add "tools/*" when the first one exists (an empty glob is an error)
    default-members = ["crates/*", "xtask"]       # plain cargo build/test/clippy skip spikes; CI passes --workspace

    [workspace.package]
    version = "0.0.0"
    edition = "2024"
    license = "MIT"
    publish = false

    [workspace.dependencies]                      # internal crates and shared versions

    [workspace.lints.rust]
    unsafe_code = "deny"                          # FFI crates opt out at the crate root with a reason

    [workspace.lints.clippy]
    all = { level = "warn", priority = -1 }
    pedantic = { level = "warn", priority = -1 }  # minus DSP-specific allows (casts, float_cmp, single-char names…)
    allow_attributes_without_reason = "warn"      # every #[allow] says why
    ```

4. Each crate's `Cargo.toml` inherits: `version.workspace = true`, `edition.workspace = true`, `license.workspace = true`, `publish.workspace = true`, and `[lints] workspace = true`.
5. `rust-toolchain.toml` pinned to **1.99.1** (never 1.99.0: it miscompiles x86_64 FFI `bool` returns, which CLAP uses everywhere), `profile = "minimal"`, `components = ["rustfmt", "clippy"]`, `targets = ["wasm32-unknown-unknown", "wasm32-wasip1"]`. CI sets `CARGO_BUILD_WARNINGS=deny` (warnings, clippy and linker warnings fail the build without invalidating the cache).
6. `.cargo/config.toml` with `[alias] xtask = "run --package xtask --"`. Commands go through xtask, not a justfile.
7. `LICENSE` (MIT). `deny.toml` (cargo-deny 0.20.2) allowing MIT, Apache-2.0, ISC, BSD-2-Clause, BSD-3-Clause, Zlib, Unicode-3.0, CC0-1.0, and `BSD-1-Clause` (`assert_no_alloc`, nice-plug's `assert_process_allocs`). Scoped exceptions: `OFL-1.1` and `Ubuntu-font-1.0` for `epaint_default_fonts` only (egui's fonts; the SPDX id is `Ubuntu-font-1.0`, not `LicenseRef-UFL-1.0`), and `BSL-1.0` for the Windows-only `clipboard-win` and `error-code`. See the 2026-10-19 note in [ADR-0004](adr/0004-licensing.md).
8. Library crates in `crates/enth-{dsp,params,patch,engine,render,analysis}` (directory names carry the `enth-` prefix), plus the `xtask` binary. Spikes go in `spikes/`, and a `clippy.toml` in tooling directories keeps the core-crate bans off them.
9. `xtask/layers.toml` — the single source of truth for layers (package names, not directories). The layer table is in [architecture.md](architecture.md#crate-layers); tooling is exempt:

    ```toml
    [core]
    max_layer = 5          # layers 0..=5 must build for wasm32 and do no I/O

    [exempt]
    packages = ["xtask", "enth-test-host", "enth-golden"]
    prefixes = ["spike-"]
    ```

10. `xtask` commands: `check-layers` (every workspace member is layered or exempt; dependencies only go to strictly lower layers) and `wasm-check` (builds every member with layer ≤ `core.max_layer` for `wasm32-unknown-unknown`). `wasm-check` proves only that core crates *build* for wasm32; a wasm32 build accepts `Instant::now`, `fs::read` and `thread::spawn`. The root `clippy.toml` (`disallowed-methods`, `disallowed-types`) is what bans I/O, clocks, threads, locks and `HashMap` in core crates. Lane A added `bundle`, `install` and `reaper-test`; other commands are added in the task that first needs them.
11. GitHub Actions on `ubuntu-24.04` (pinned): fmt, clippy `-D warnings`, test, `cargo xtask check-layers`, `cargo xtask wasm-check`, `cargo deny check`, and a clap-validator 0.4.1 job on the spike bundle. Actions are pinned by commit SHA, `rust-cache` writes only from `main`, and Dependabot has a 7-day cooldown. A CI job that needs inputs which don't exist yet is added later, not gated with a job-level `hashFiles()` (not allowed there). The remote is the private GitHub repo `utof/enth`; Codeberg may become a push mirror.

**Acceptance:** all commands pass locally; `cargo xtask check-layers` fails on a deliberately added upward dependency (then remove it); CI passes after you push. Status: passed; the first CI run was green.

## Task 3 — Shell spike on nice-plug (3–4 days)

**Lane A. Done: merged 2026-10-19.**

Build `spikes/spike-shell` (package `spike-shell`): an 8-voice synth with one saw oscillator, a one-pole low-pass, an amp envelope and smoothed gain/cutoff. Start from nice-plug's template (<https://codeberg.org/RustAudio/nice-plug-template>, a cookiecutter: `uvx cookiecutter …`), but take the crates from crates.io, pinned `nice-plug =0.4.2` and `nice-plug-egui =0.6.0`, never a git dependency. Wire nice-plug's bundler into our xtask so `cargo xtask bundle spike-shell` works, and add `cargo xtask install` (copies the bundle to `~/.clap`). Build debug bundles with nice-plug's `assert_process_allocs` feature.

Check the **must-haves** for our design ([design.md §4](design.md#4-host-parameters-macros--automation-slots)). Most checks run without hands through `cargo xtask reaper-test` (headless REAPER + Lua under Xvfb, 3–5 s per scenario; a local gate, not CI):

| # | Must-have | How to check | Result (2026-10-19) |
| --- | --- | --- | --- |
| a | 72 fixed host params (8 macros + 64 slots) with stable IDs | Automate a macro and a slot in REAPER; save, reload the project | Pass in REAPER (CLAP and VST3) and Carla (VST3) |
| b | Slot renaming (CLAP param rescan) | Assign slot 1 to "cutoff" at runtime; does REAPER show the new name after rescan? If not, record it — "Slot 1" is an acceptable fallback | CLAP: works with a 123-line add-only nice-plug patch (name changes in 50 ms). VST3 in REAPER: rejected by the host, so "Slot N" |
| c | An arbitrary patch blob (JSON) saved and restored with the project | Change cutoff, save, reopen REAPER | Pass (REAPER, Carla) |
| d | CLAP polyphonic modulation on a macro | Bitwig is the reference host for poly-mod; record REAPER's behaviour | Declared correctly, untested in a host (REAPER has no poly-mod UI). Bitwig trial is optional; the real check is S1 |
| e | VST3 export builds and loads | Load the `.vst3` in REAPER | Pass; Steinberg's validator 537/537 |
| f | `clap-validator validate` passes | Run on the bundle | Pass: 36 passed, 0 failed |
| g | Builds on stable Rust | No nightly features enabled | Pass on 1.99.1 |
| h | Runs in a second Linux host | Load, play, automate, save/reload in a second host | Partly: Carla 2.5.8 (VST3 only) loads, shows 72 params, restores state. No CLAP second host here; Bitwig and Ardour are not installed |
| i | Editor on the real display in REAPER | First insert 10/10 draws, repaint on automation with the mouse still, 20 close/reopen cycles, HiDPI, keyboard | Owner checklist (never automated on the real display). Under Xvfb: 22/22 opens drew |

**Decision rule** (same as [ADR-0001](adr/0001-plugin-shell.md)): if (a), (c), (e), (f) or (g) can't be made to work within two days, repeat the spike on `clack` + `clap-wrapper` and compare. (b) and (d) failing is acceptable if recorded; (h) failing needs a note on whether the host or our code is at fault. The rule did not trigger: (a), (c), (e), (f) and (g) passed on the first build.

Also settled here: permanent plugin IDs ([naming.md](naming.md)) and the two nice-plug pitfalls (`execute_gui` tasks may run inline inside `process()`; `editor_closed()` is never called). Not checked yet: notes the plugin triggers itself while the host transport is stopped (needed for auditioning in Recreate); check it in P1b.

**Acceptance:** results table in `docs/spikes/shell.md`; ADR-0001 updated with the findings. Status: done.

## Task 4 — UI toolkit spike (4–5 days)

**Done 2026-10-19: lanes U1 ([spikes/ui.md](spikes/ui.md)) and H ([spikes/test-host.md](spikes/test-host.md)); U2's Xvfb measurements were made by those two, and its real-display items are on the [owner checklist](spikes/owner-checklist.md).**

**egui with glow is the only arm.** 2026-10-19: vizia is stale (vizia-plug last pushed 2026-06-13, issue #41 unanswered) and iced spins a core while idle and leaks a thread per editor open, so neither is a challenger; the WebView arm is cut. wgpu is out too: it creates a device per editor and adds 16–21 MB of PSS under software rendering. If egui fails a threshold after tuning, run a new research session instead of keeping a second toolkit warm ([ADR-0002](adr/0002-ui-toolkit.md)). The risk sits in shared plumbing: baseview (a small leak per editor open, a ≈30 fps cap, HiDPI clipping in REAPER) and the NVIDIA X driver bug.

**Safety rule.** Agents never open windows on the real display. All automated GUI work (the plugin in REAPER, standalone egui windows, validators that open editors) runs under `xvfb-run` with Mesa software rendering. Checks on the real GPU are owner-attended, with work saved and at least 10 s between launches. Measurements are split in two tiers in [benchmarks.md](benchmarks.md).

Follow [benchmarks.md](benchmarks.md) and build the **reference panel** once, in the "Lanes" direction of [design-system.md](design-system.md), as three lanes:

1. **Lane U1 — panel and tokens.** `enth-ui::theme` (design tokens: every colour, size, spacing, radius, stroke, font and repaint rate), widgets, the reference panel as a standalone dev window, `egui_kittest` snapshot tests on lavapipe (edit → rebuild → snapshot takes about 3.5 s), and `spike-ui-plugin` (the panel inside the Task 3 shell, with repaint gating: scopes request the next frame only while visible and fed; measured: a 1/60 s request (`motion.scope_hz`) gives ≈30 fps under baseview 0.4.1, a 1/30 s request ≈20 fps, so the token is 60). Name the GUI thread and log CPU time per frame window.
2. **Lane H — test host.** Build `tools/test-host` (package `enth-test-host`): a minimal CLAP host on `clack-host` that loads a `.clap`, processes audio in a loop, opens/closes the editor N times, injects parameter changes, and logs timings, PSS and thread counts as JSON. It is the heaptrack-capable cycle runner (REAPER crashes under heaptrack) and the CI leak check.
3. **Lane U2 — in the host.** Run the measurement protocol in headless REAPER under Xvfb and in the test host, then the owner's real-display checks. Decide the G0 UI thresholds.

Two more tracks fold in here:

- **Integration-bug track (2–3 days).** Reproduce, minimise and report: the first-frame stall seen on the real display (nice-plug #95), the baseview open/close leak (≈70 KB per cycle), the HiDPI scale mismatch in REAPER (#359), a focus policy (focus only for text fields, return focus to the host), `editor_closed()` never being called, and the "X11 thread disconnected" close warning. RustAudio's AI policy rules out upstreaming agent-written patches, so these go upstream as reports, or live in our fork.
- **NVIDIA safety track (owner-attended, about half a day).** glow vs wgpu on the real GPU (open time, VRAM, fps), first-open reliability (20 opens in REAPER), the same with `prime-select on-demand`, and a decision on whether a CPU-rendered fallback (softbuffer or tiny-skia) is needed.

**Acceptance:** results table complete for both tiers in `docs/spikes/ui.md`; the numbers added to [ADR-0002](adr/0002-ui-toolkit.md) (its status changes in Task 7); the integration bugs listed with a link or a reason for each. Status: the automated tier is complete across [ui.md](spikes/ui.md), [test-host.md](spikes/test-host.md) and the [G0 UI table](benchmarks.md#ui-thresholds); the owner tier is on the [owner checklist](spikes/owner-checklist.md); ADR-0002 is accepted; the integration bugs are listed in [spikes/summary.md](spikes/summary.md#integration-bugs-task-4-track) (the baseview leak root-caused, with a draft upstream report).

## Task 5 — Engine speed and determinism spike (2 days)

**Lane B. Done: merged 2026-10-19 — [spikes/engine.md](spikes/engine.md).**

In `spikes/engine` (package `spike-engine`): osc → filter → amp with 16 voices, an offline renderer, fixed block size 64, seed passed in. The contract being tested is [design.md §5](design.md#5-determinism): renders are **bit-exact across platforms**, and the old tolerance contract stays as a fallback until the Windows (and, if available, aarch64) CI hashes are green.

- **Speed.** Measure ×realtime per core for 2 s renders (1 voice, 16 voices) with `hyperfine`, reported as min and median with the load average recorded, next to callgrind instruction counts (gungraun). Time render **plus analysis** per candidate, since analysis costs as much as a render.
- **Hashes, not summaries.** Render the same patch twice → identical SHA-256. Compare hashes across native Linux, wasm32 under wasmtime and node (`wasm32-wasip1` and `wasm32-unknown-unknown`), and CI on Windows (and aarch64 where runners exist). Record the feature-summary maximum (RMS per 10 ms frame, spectral centroid per frame) only where hashes differ. Platform keys come from `cfg!`, because `std::env::consts::OS` is empty on wasip1; wasmtime needs `--dir` and `--env` to reach files and flags.
- **DSP rules under test:** transcendentals only through a pinned `libm` or our own approximations; no `mul_add`; fixed summation order; denormals flushed in code.
- **Tests to add:**
  - lanes = scalar: `wide` f32x4/f32x8 gives the scalar hash;
  - FTZ off = on: the same hash with FTZ off, FTZ only (what nice-plug sets) and FTZ+DAZ;
  - a subnormal count over a 40 s release tail is zero (voices end below −100 dB);
  - `assert_no_alloc` around `process()`, gated on `debug_assertions` (release builds lack `AllocDisabler`);
  - one nightly RTSan run (the runtime ships in rust-std). RTSan misses an uncontended `std::sync::Mutex`, so `enth-engine` has no `Mutex` or `RwLock`.
- **Bench gate.** Instruction counts plus the deterministic subnormal test. Criterion reported false "improvements" twice with no code change under load, so wall-clock stays information.

**Acceptance:** numbers in `docs/spikes/engine.md`; ≥ 100× realtime per core on the 1-voice template (16 voices reported too); hashes identical across the platforms that have run, with the Windows/aarch64 status stated; any hash difference explained with its feature-summary maximum. Status: passed. 1,993× realtime on the 1-voice template (16 voices 129× scalar, 245× with lanes); 1,010 hashes identical on Linux, Windows (CI) and wasm32 under wasmtime; aarch64 runs only by manual dispatch; no hash differed.

## Task 6 — Metric and search spike (3 days)

**Lane C. Done: merged 2026-10-19 — [spikes/metric.md](spikes/metric.md).** (Lane P, the agent-loop probe, built on its renderer: [spikes/agent-probe.md](spikes/agent-probe.md).)

In `spikes/metric` (package `spike-metric`):

1. A 10-parameter subtractive template (osc wave, detune, cutoff, resonance, filter-env amount, amp A/D/S/R).
2. Features, on `rustfft`/`realfft` (our own mel/MFCC/centroid/envelope code): multi-resolution log-mel at hop n/2 (halves the analysis cost), a beat-aware amplitude envelope, spectral centroid track, f0, and EBU R128 loudness (`ebur128`) for level matching.
3. Three candidate distances: (A) multi-resolution log-mel L1; (B) A + envelope + centroid-track composite; (C) MFCC-based. Scores must match across platforms: use a scalar FFT path, or a tolerance ≥ 1e-5 with a deterministic tie rule. Measure the scalar path's cost (rustfft picks AVX, SSE or scalar by CPU, and the three give different bits).
4. A `listen` CLI the owner runs whenever; nothing blocks on it. Two sessions of 30 pairs plus anchors, loudness-matched and reshuffled between sessions, rated 1–5 blind to the metric. The second session gives the test–retest ceiling. Report Kendall τ-b and Spearman per distance with bootstrap confidence intervals (with 30 pairs, ρ = 0.7 has a 95% CI of about [0.42, 0.86]), and use ABX for "sounds the same" on solved and unsolved targets.
5. An own ask–tell CMA-ES (no crate fits: `cmaes` panics on wasm32, `basin` has no ask–tell) recovering 20 random targets from random starts with each distance:
   - population λ ≥ number of workers (16 for 8 workers);
   - a heuristic start (pitch → tuning, centroid → filter, envelope → ADSR) and random restarts run side by side;
   - the waveform (categorical) cycles at restarts;
   - the evaluator is a work queue that returns results by input index, so results are identical for 1 or N workers;
   - 5,000 evaluations per solve.
6. Report numbers, not impressions: evaluations to threshold and wall time at 1 and 8 workers with the load average (`uptime`), per distance, plus how many of the 20 targets are solved.

**Acceptance:** `docs/spikes/metric.md` with correlations and their intervals, recovery success rate and time; a recommended `metric v1` (provisional, recalibrated by the listening sessions and again in R1); an ADR draft for the metric (ADR-0007). Status: done except the correlations, which need the owner's two sessions (34 trials each: 30 pairs plus 4 anchors; [owner checklist](spikes/owner-checklist.md) A1–A2). Recovery: 17 of 20 at the strict proxy with distance A and a heuristic start, median 3.5 s per solve on 8 workers. Metric v1 = distance A ([ADR-0007](adr/0007-metric-v1.md), Proposed).

## Task 7 — Close Phase 0 (½ day)

**Lane Z. Done 2026-10-19 — [spikes/summary.md](spikes/summary.md).** The G0 numbers are in [roadmap.md](roadmap.md#p0--spikes-and-skeleton-done-2026-10-19-except-owner-checks) and [benchmarks.md](benchmarks.md#ui-thresholds); ADR-0001 and ADR-0002 are accepted; the owner's checks are in [spikes/owner-checklist.md](spikes/owner-checklist.md).

- Fill the **G0 thresholds** in [roadmap.md](roadmap.md) and [benchmarks.md](benchmarks.md) with measured numbers.
- Mark ADR-0001 and ADR-0002 *Accepted* (or record the change of plan).
- Write `docs/spikes/summary.md`: what we learned, what changed in the plan, what's next for Phase 1, and **actual vs estimated days per lane** (the first real calibration of every effort figure in these docs).
