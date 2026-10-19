# Spike: plugin shell on nice-plug (Phase 0, Task 3)

- Date: 2026-10-19
- Code: `spikes/spike-shell` (package `spike-shell`), branch `lane-a/shell`
- Decision it feeds: [ADR-0001](../adr/0001-plugin-shell.md)
- Verdict: **keep nice-plug.** Must-haves (a), (c), (e), (f) and (g) pass in a real host and with both validators. (b) and (d) behave as the ADR allows. (h) is partly done. (i), the editor on the real display, is on the owner checklist because the rule for this work is never to open a window on the real display.

## What was built

- An 8-voice synth: saw oscillator, one-pole low-pass, ADSR amp envelope (5 ms / 80 ms / 0.7 / 120 ms), smoothed gain and cutoff. Voice stealing takes the quietest releasing voice. About 790 lines in two files (`synth.rs` is pure Rust and unit-tested; `lib.rs` is the plugin).
- 72 host parameters from `#[nested(array)]`: ids `macro_1` to `macro_8` ("Macro 1" to "Macro 8") and `slot_1` to `slot_64` ("Slot 1" to "Slot 64"). Macro 1 is gain and slot 1 is cutoff; the rest are placeholders at 0.5. Macros are modulatable per note id; slots are plain automatable.
- An arbitrary JSON patch blob, `#[persist = "patch"]` on a `RwLock<serde_json::Value>`. A time-based nonce is written into it on the first `activate`. That is a test hook so a host round trip can prove the blob came back, not product behaviour.
- A placeholder egui editor (360 × 200: heading, two parameter sliders, the nonce).
- CLAP and VST3 export. Permanent ids are in [naming.md](../naming.md): CLAP `dev.enth.spike-shell`, VST3 class id `EnthSpikeShell01`, and a separate reserved product id.
- `assert_process_allocs` is on. It is inert in release builds (it is gated on `debug_assertions`), so allocation checks only happen in debug bundles.
- xtask commands: `cargo xtask bundle <pkg> [--release]`, `cargo xtask install <pkg>`, `cargo xtask reaper-test [--release] [--no-build] [--format clap|vst3]`.

Pins: `nice-plug =0.4.2`, `nice-plug-egui =0.6.0` (egui 0.37.0, egui-baseview 0.9.0, baseview 0.4.1), `nice-plug-xtask =0.1.1` for the bundler. Toolchain: stable Rust 1.99.1 from `rust-toolchain.toml`.

## Results

| # | Must-have | Result | Evidence |
| --- | --- | --- | --- |
| a | 72 fixed host params, stable ids | **Pass** (REAPER, CLAP and VST3; Carla, VST3) | REAPER 7.82 lists 75 params: the 72 plus its own Bypass, Wet and Delta. Names are "Macro 1–8" and "Slot 1–64", all automatable. REAPER's param idents are `index:hash`: `macro_1` is `0:825615486`, `slot_1` is `8:1248030256`, `slot_64` is `71:34232479`, identical for CLAP and VST3 and identical after a reload in a new REAPER process. Envelopes on macro 1, slot 1 and slot 64 survive a save and reload. A small ctypes CLAP host (the researcher's script, run on this bundle) reads `AUTOMATABLE \| MODULATABLE \| MOD_PER_NOTE_ID` on the macros and `AUTOMATABLE \| MODULATABLE` on the slots. Carla 2.5.8 shows 72 params with the same names |
| b | Slot renaming | **Not in the spike; fallback "Slot N" in force.** A 123-line patch makes CLAP renaming work in REAPER | Stock nice-plug 0.4.2 fixes parameter names at construction. The researcher's patch (13 files, +123/−4, additive; kept in scratch at `2026-10-19-phase-0-spikes/01/scripts/nice-plug-0.4.2-rename.patch`) adds a display-name override and a main-thread `rescan(INFO)`. With it REAPER's CLAP param name changed in 50 ms. REAPER 7.82 rejects the VST3 equivalent (`restartComponent(kParamTitlesChanged)` returns not-ok four times out of four), so VST3 in REAPER will show "Slot N". Carla's VST3 host does show the new name. An existing automation lane keeps its old label but stays bound to the parameter by id. Not re-run here; see [ADR-0001](../adr/0001-plugin-shell.md) |
| c | Patch blob saved and restored with the project | **Pass** (REAPER, CLAP and VST3; Carla, VST3) | `reaper-test` decodes the plugin state out of the saved project: macro 1 = 0.7 and slot 1 = 0.3 are in the chunk, and after a reload in a new REAPER process the same values and the same patch nonce come back (for example `1792378801879424754` before and after, CLAP). Carla: values restored, and the state chunk saved before and after the reload is byte-identical |
| d | CLAP poly-mod on a macro | **Declared; host behaviour untested** | The macros carry the per-note-modulation flag, the plugin exposes `voice-info` and the poly-modulation config (8 voices, overlapping). Unit tests cover the poly-mod ids and the handling of host voice ids. REAPER has no poly-mod UI, and Bitwig is not installed here. Owner checklist item 4 |
| e | VST3 export builds and loads | **Pass** | Loads, saves, reloads, renders and opens its editor in REAPER (the same 45 `reaper-test` checks for both formats). Steinberg's VST3 validator (SDK 3.8.1, built from source in scratch): **537 passed, 0 failed** with `-e`, on both the debug and the release bundle (47 / 0 without `-e`). Carla 2.5.8 loads it too |
| f | `clap-validator validate` passes | **Pass** | clap-validator 0.4.1 (official release binary): 44 tests run, **36 passed, 0 failed, 0 warnings, 8 skipped**, on both the debug and the release bundle. The 8 skipped are for features the plugin doesn't have: 3 preset-discovery tests (no factory), 2 tests with 64-bit audio, the denormals test (a synth has no audio input), and the `audio-ports-activation` and `configurable-audio-ports` extensions |
| g | Builds on stable Rust | **Pass** | `rust-toolchain.toml` pins stable 1.99.1; no nightly feature is enabled (nice-plug's `simd` feature is off: `default-features = false`). All crates build, clippy `-D warnings` is clean |
| h | Runs in a second Linux host | **Partly done** | Carla 2.5.8 (VST3 only) loads the plugin headless, shows 72 params, takes values and restores them after a project save and load. It has no CLAP host in this version, no real audio and no editor was tried. Ardour is VST3-only and not installed; Bitwig is not installed. "Play and automate" in a second host is not done. Owner checklist item 4 |
| i | Editor on the real display | **Owner checklist** | Not run, by rule. Under Xvfb with Mesa software GL, `reaper-test` opens the editor 5 times per format (each time a window appears) and checks that it stays open for 2 s. The researcher's longer run (22 opens per format, 0 frames when idle, about 26 frames/s while automation plays) used the instrumented `spike-np-gui` and is in the research report |

The decision rule in [phase-0.md](../phase-0.md) never triggers: (a), (c), (e), (f) and (g) all pass on the first build, so there is no reason to repeat the spike on clack.

## How to reproduce

```
cargo xtask bundle spike-shell [--release]       # target/bundled/spike-shell.{clap,vst3}
cargo xtask reaper-test [--release] [--no-build] # headless REAPER, always under xvfb-run, needs ENTH_REAPER or the default path
clap-validator validate target/bundled/spike-shell.clap
validator -e target/bundled/spike-shell.vst3     # Steinberg's, built from the VST3 SDK
```

Run anything that loads a plugin under `xvfb-run -a -s "-screen 0 1920x1080x24"`. `reaper-test` does this itself. The CI job `clap-validator on spike-shell` runs the second command (pinned 0.4.1, sha256-checked) on a debug bundle; it was checked with actionlint 1.7.12 and not run on GitHub from here.

### What `reaper-test` checks

For each of CLAP and VST3, in a throwaway copy of the REAPER install with Dummy Audio:

- **state-create:** the plugin loads, 75 params, names, all automatable, the three golden idents, set macro 1 = 0.7 and slot 1 = 0.3, add an envelope on slot 64, save the project.
- **state-reload:** a new REAPER process loads the project and sees one plugin, the same values, the envelope with 2 points and the same 72 idents. The xtask then decodes the saved plugin state itself and checks the values and the patch nonce in the saved and in the reloaded chunk.
- **automation:** an offline render of 20 short notes with macro 1 (gain) and slot 1 (cutoff) automated from 0 to 1 over 2 s. The render is finite, at least 3× louder in 1.7–2.0 s than in 0.25–0.55 s, and at least 2× brighter (high-frequency energy). Measured: RMS 0.0143 → 0.1090 and brightness 0.0293 → 0.1198, identical for CLAP and VST3 and bit-identical across 6 consecutive runs and across debug and release.
- **editor-smoke:** 5 open and close cycles; each shows a window; the window stays for 2 s.
- **cross-format:** CLAP and VST3 report the same 72 ids.

Total: 45 checks, about 16 s per format and about 45 s for the whole command on a warm build. It passed in 7 of 7 consecutive runs (one with a fresh build, five with `--no-build`, one against the release bundle).

## Measurements

The machine was shared with other agents the whole time (load average 9–25, `CARGO_BUILD_JOBS=3`, builds under a shared lock), so build times are upper-ish figures.

| Measure | Value |
| --- | --- |
| Bundle size, debug | `.clap` 191,116,968 B (182 MiB, full debuginfo); `.vst3` the same `.so` |
| Bundle size, release | `.clap` 9,114,152 B (8.7 MiB); `.vst3` the same `.so` |
| Cold release build of the bundle | about 3 min |
| Cold test build of the workspace with the spike | 1 min 44 s; the xtask additions (nice-plug-xtask and its tree) add about 32 s |
| Incremental rebuild after a one-constant edit | debug 1.3–2.9 s, release 5.3–6.7 s (`cargo build -p spike-shell`) |
| `cargo test --workspace --locked` | 1 min 15 s warm; 13 spike-shell tests (7 synth, 6 plugin) |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | 1 min 57 s |
| `reaper-test` | about 45 s warm |

## Findings

### Pitfalls in nice-plug 0.4.2 and 0.6.0

1. **`execute_gui` may run inline inside `process()`.** If `process()` is already on the thread nice-plug treats as main (Steinberg's validator does that), a scheduled GUI task runs on the audio path, and with `assert_process_allocs` any allocation aborts. Keep `process()` from scheduling main-thread work, and make any task it does schedule real-time safe. (Researcher's measurement; this spike never calls `execute_gui` from `process()`.)
2. **`NiceEguiApp::editor_closed()` is never called** by nice-plug-egui 0.6.0 or egui-baseview 0.9.0. Don't rely on it for teardown; detect a reopen in `build()` instead. (Researcher's measurement: 0 calls in 22 closes per format.)
3. **`VoiceID::id_or_fallback` ignores the host's voice id** (nice-plug-core 0.4.2, `midi.rs`, line 66): it always builds an id from key and channel and never returns the id the host supplied, although its doc comment says it is a fallback for when the host gave none. `spike-shell` reads `VoiceID::id()` and builds its own fallback (a unit test pins this). Poly-mod from a host that sends voice ids needs this.
4. **Any allocation in `process()` aborts in debug builds**, even `std::env::var_os`. Read configuration in `initialize` or at construction. (A debug flag read from the environment inside `process()` aborted an experiment and gave me a wrong result while I was investigating the render restart below.)
5. **A `#[persist]` field holding a `String` of JSON is escaped twice in the state.** Persist a typed value (`serde_json::Value` here), as the spike does.

### REAPER

- **REAPER adds three params of its own** (Bypass, Wet, Delta) after the plugin's 72, so a host-side count is 75. Idents are `index:hash` where the hash is a 31-bit hash of the string id; state stores the string ids.
- **A VST3 parameter set from a script takes effect only after a process cycle**, so a script that reads it back or saves right away sees the old value. The tests wait 1.5 s.
- **Offline render restarts CLAP plugins.** About 30 ms after an offline render starts (`-renderproject`, or a render started from a script), REAPER's main thread deactivates, reactivates and resets each CLAP plugin. A held note is dropped and the render goes silent. Real-time playback is not affected. In the first full run only the CLAP render went silent; the VST3 render did not. It is not caused by the editor, the nonce, the audio device or the compiler: the researcher's own `spike_np` behaves the same when built locally. I didn't find what triggers it. The automation test therefore renders 20 short retriggered notes instead of one long one. The researcher's prebuilt bundle rendered a long note without a restart, apparently by timing, so the render comparison between formats that I first wrote (equal hashes) was dropped; the levels still agree to four digits. Anything that renders offline in REAPER must not depend on a note spanning an `activate`.
- **Quitting REAPER from Lua** prompts to save unsaved projects. Load an empty project with the `noprompt:` prefix, then run action 40004.
- **A fresh `reaper.ini` in a portable copy** with `linux_audio_mode=2` (Dummy Audio) and the plugin paths raises no scan or nag dialog.

### Licences

`cargo deny` needed three changes ([ADR-0004](../adr/0004-licensing.md), 2026-10-19 note): `BSD-1-Clause` is allowed (nice-assert-no-alloc), `Ubuntu-font-1.0` is scoped to `epaint_default_fonts`, and `BSL-1.0` is excepted for two Windows-only crates (`clipboard-win` and `error-code`). `Ubuntu-font-1.0` is **not** OSI-approved in SPDX 3.29.0, which the task brief assumed; it is allowed because it is free and permissive, and the `LicenseRef-UFL-1.0` entry in ADR-0004's decision section never matches anything. `cargo deny` also warns about duplicate `objc2*` and `syn` versions; those are warnings by policy.

## Not done, and why

- **The editor on the real display** (rule: no windows or GL on the real display).
- **Poly-mod in a host** (no Bitwig; REAPER has no poly-mod UI).
- **Play and automate in a second host** (Carla was only loaded and its state round-tripped; no CLAP host in Carla 2.5.8; Ardour and Bitwig not installed).
- **Real audio device** (Dummy Audio only; xruns and latency untested).
- **Slot renaming** was not re-implemented here (see b). The patch lives in scratch only, so the decision to carry it in a fork is open.
- **The CI job** was linted but not run on GitHub.
- **A first-time `reaper-test` on a machine without the REAPER portable install** needs `ENTH_REAPER` pointing at it (default `~/apps/reaper-7.82/...`).

## Owner checklist

Needs your hands and eyes, with the bundles from `cargo xtask install spike-shell` (copies to `~/.clap` and `~/.vst3`; use `--release` for a quiet build):

1. **Editor on the real display (i):** insert `enth spike-shell` (CLAP, then VST3) on a fresh track 10 times. Is the first open ever blank or grey? Resize it, close and reopen it 20 times, and leave it open and idle for 60 s (`pidstat -p $(pgrep reaper) 1 60` and `nvidia-smi dmon -c 60`; compare with the editor closed).
2. **HiDPI and keyboard:** at 100% and 200% scale, is the editor sharp and the right size? Does the space bar still start the transport while the editor has focus?
3. **Real audio:** use PipeWire or JACK, play notes (macro 1 is gain, slot 1 is cutoff), listen for clicks, check xruns.
4. **Bitwig and a second host (d, h):** load the CLAP, add a per-voice modulator on Macro 1 and check the voices modulate independently; then save and reload. If Ardour is available, do the same plain checks with the VST3.
5. **Rename in the UI (b):** with the scratch patch applied, does the FX parameter list and an automation lane label show the new slot name, and does a lane created earlier keep its old label?
6. **Decide:** carry the rename patch in a fork (CLAP only) or accept "Slot N" everywhere.
