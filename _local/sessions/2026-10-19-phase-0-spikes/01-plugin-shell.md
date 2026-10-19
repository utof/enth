# 01 — Plugin shell today

- Date: 2026-10-19
- Sub-researcher: 01 (plugin shell)
- Questions answered: 1–8 from the brief. Q1–Q3 and Q8 are answered in depth from source, plus a local spike. Q4–Q7 are answered from repositories, release notes and the web.
- Evidence labels: **measured** (I ran it on the owner's laptop today), **documented** (source code, changelog, release notes, official pages), **anecdotal** (issue reports, forum posts, single users).
- Local work: everything was built in scratch crates under `/tmp/enth-research/01/` with `CARGO_BUILD_JOBS=2`, on rustc/cargo 1.95.0 stable. Nothing was installed and nothing in the enth folder was touched. Six other researchers were running at the same time, with about 2.5–4.9 GiB of RAM free, so build times are pessimistic.

## Answer in brief

1. **The pin is nice-plug 0.4.2 + nice-plug-egui 0.6.0, both from crates.io.** nice-plug 0.4.2 was released on 2026-09-14. Its `main` branch (commit `7197104`, 2026-10-18) only adds new adapter releases on top of it: nice-plug-egui 0.6.0 and nice-plug-iced 0.5.0, published to crates.io on 2026-10-18 and built on baseview 0.4.1 and egui 0.37. The licence is **ISC (I read the file)**. nice-plug still uses the MIT/Apache **`vst3` 0.3.0** crate and `clap-sys` 0.5.0. There is **no `vst3-sys` anywhere in the tree** (measured with `cargo tree`). *(documented + measured)*
2. **Two licence traps would break the planned `cargo deny` CI (measured with cargo-deny 0.19.4):**
   - nice-plug's `assert_process_allocs` feature pulls in `nice-assert-no-alloc`, which is **BSD-1-Clause**. The planned `assert_no_alloc` crate for engine tests is also BSD-1-Clause. I verified both LICENSE files.
   - egui 0.37's default fonts (`epaint_default_fonts`) declare **`Ubuntu-font-1.0`**, not the `LicenseRef-UFL-1.0` that ADR-0004 and Task 2 list.
3. **A GUI-less spike on nice-plug 0.4.2 already meets must-haves (a), (c), (f) and (g), and builds (e). All measured.** It has 72 params (8 macros + 64 slots) and a persisted JSON patch blob.
   - It builds on stable 1.95.0 in 1 min 40 s from cold.
   - The host sees **72 params with stable IDs**; the macros carry `MODULATABLE_PER_NOTE_ID` and the plugin exposes `voice-info`.
   - The patch blob is saved in the CLAP state.
   - **clap-validator 0.4.1: 36 passed, 0 failed, 8 skipped (all "not applicable").** A 90-second fuzz run was clean, with `assert_process_allocs` active.
   - An egui-editor variant built and validated the same way. It **links only libc/libm/libgcc**, because baseview 0.4.1 loads X11 and GL at runtime. So the Task 1 apt list isn't needed to build.
4. **Must-have (b), runtime slot renaming, does not work in nice-plug 0.4.2 (documented, from the code).** Parameter names are a `String` fixed when the parameter is constructed (`Param::name() -> &str`). The CLAP wrapper only ever calls `rescan(CLAP_PARAM_RESCAN_VALUES)`, never `RESCAN_INFO`, and the VST3 wrapper never sends `kParamTitlesChanged`. The CLAP spec allows `RESCAN_INFO` for name changes while the plugin is active, so a small patch would do it. Upstreaming that patch is the hard part (point 9).
5. **The egui adapter no longer redraws all the time while open.**
   - Since nice-plug-egui 0.4.0 (2026-08-17), it repaints only when asked: through egui's own `request_repaint`, host parameter changes, or a `RepaintNotifier` (an atomic flag safe to set from the audio thread).
   - 0.6.0 uses baseview 0.4's lazy redraw and **checks that flag on a timer every 1/64 s**; the interval can be changed (`with_repaint_poll_interval`).
   - Between 0.5.x and 0.6.0, a regression made `RepaintNotifier` do nothing until the mouse moved. It was fixed in egui-baseview 0.9.0 (2026-10-17/18).
6. **The biggest risk is churn in the GUI stack, not the parameter or state core.**
   - baseview went through 0.2, 0.3 and 0.4 between July and October 2026.
   - An open baseview PR (#357, 2026-10-13) moves X11 event handling onto the host's main thread through CLAP timer/fd support. That will be one more breaking change for nice-plug's adapters.
   - A REAPER-on-Linux bug where the editor stays blank on first open (nice-plug #95) was fixed only on 2026-10-18, and the reporter hasn't confirmed the fix yet.
7. **clack 0.2.0 and clap-wrapper are healthy, but switching to them costs weeks (my estimate) and loses things.**
   - clack 0.2.0 was released 2026-09-12. It calls itself feature-complete but still makes breaking changes, and it has one main maintainer, Adrien Prokopowicz, who now also leads baseview.
   - clap-wrapper (C++) released 0.16.0 on 2026-08-08 and is active. Its Rust crate, `clap-wrapper` 0.3.1 on crates.io, still bundles clap-wrapper 0.14.0.
   - Switching would cost roughly 2–4 weeks and lose nice-plug's standalone target. The shared maintainer means the GUI risk would remain.
8. **Hosts need changes before Task 3.**
   - **REAPER under Wine cannot load Linux `.clap`/`.vst3` binaries.** Install the native Linux REAPER 7.82 (released 2026-10-04). It is a tarball, can be installed in your home directory, and comes with a 60-day full evaluation.
   - **Ardour 9.8 (2026-08-23) has no CLAP support** that I could find, so it can only check the VST3 build. Bitwig, with a 30-day trial, is the only CLAP and poly-mod second host.
   - Reboot before any GUI check: the NVIDIA driver mismatch on your machine will break OpenGL editors.
9. **Upstreaming is constrained.** nice-plug and baseview follow the RustAudio AI policy, which disallows "vibe coding" whole PRs and AI-generated function blocks. enth is written mostly by agents, so plan to carry patches such as slot renaming in a fork, or have you write and submit them yourself.
10. **There has been no new Rust plugin framework since 2026-10-18.** Of the recent ones, truce uses a custom licence and lx-aura is GPL-3.0, so both fail the licence gate. *(documented)*

## Findings

### Q1 — nice-plug: version, cadence, maintainers, issues, licences, `vst3`

**Versions and activity (documented, 2026-10-19):**
- **crates.io:** `nice-plug` 0.4.2 (2026-09-14) is the latest. It had 19 versions between 0.1.0 (2026-06-04) and 0.4.2. The crate has 15,687 downloads in total and 12,206 in the last 90 days, MSRV 1.88, edition 2024 ([crates.io](https://crates.io/crates/nice-plug)).
- **Adapters:** `nice-plug-egui` 0.6.0 and `nice-plug-iced` 0.5.0 were both published on 2026-10-18 ([crates.io](https://crates.io/crates/nice-plug-egui)).
- **Codeberg `main`:** `7197104c0d` (2026-10-18), "update to baseview 0.4.1 (#97)". Everything after the `v0.4.2` tag (`e60b5db09d`) is adapter-only: egui 0.37, baseview 0.4.1, a configurable poll timer ([commits](https://codeberg.org/RustAudio/nice-plug/commits/branch/main); [CHANGELOG](https://codeberg.org/RustAudio/nice-plug/src/branch/main/CHANGELOG.md)).
- **Repo statistics (Codeberg API):** 79 stars, 39 forks, 12 open issues, 3 open PRs, 13 releases. Created 2026-04-17, not archived. CI runs on Woodpecker: tests, clippy, docs, and one bundle build. **It does not run clap-validator** ([.woodpecker/test.yaml](https://codeberg.org/RustAudio/nice-plug/src/branch/main/.woodpecker/test.yaml)).
- **Commits on `main` per month since the fork:** Apr 7, May 65, Jun 68, Jul 28, Aug 55, Sep 41, Oct 6. That is 270 commits, counted by author date.

**Maintainers (documented):** Billy Messenger (BillyDM) wrote 240 of those 270 commits, about 89%. No one else has more than 5. BillyDM is the **only crates.io owner** of nice-plug, nice-plug-egui and egui-baseview ([crates.io owners API](https://crates.io/api/v1/crates/nice-plug/owners)). In practice the bus factor is 1. The Codeberg members API needs a login token, so the organisation's membership is unknown.

**Breaking-change rate (documented, CHANGELOG):**
- The core crate had **3 breaking minor releases in about 7 weeks**: 0.2.0 (2026-07-24), 0.3.0 (2026-08-17) and 0.4.0 (2026-09-09).
- The egui adapter had **5 in under 4 months**: 0.2.0 (06-28), 0.3.0 (07-24), 0.4.0 (08-17), 0.5.0 (09-09) and 0.6.0 (10-18).
- Each core release listed 9–14 breaking items: the editor trait rewritten twice, `initialize` renamed to `activate`, note renamed to key, the MSRV bumped, and others.
- The README still says: "This project is currently experimental, and has recently undergone some large changes, so expect some bugs" ([README](https://codeberg.org/RustAudio/nice-plug/src/branch/main/README.md)).

**Open issues that matter to enth (documented/anecdotal, Codeberg, 2026-10-19):**
- [#95](https://codeberg.org/RustAudio/nice-plug/issues/95): an egui or iced editor doesn't render when first added in REAPER on Linux. It reproduces in REAPER 7.73 and 7.79, and in a clack + baseview example, so the bug is in baseview. The maintainer says it was fixed on 2026-10-18 and is waiting for the reporter to confirm.
- [#31](https://codeberg.org/RustAudio/nice-plug/issues/31) "switch to using clack?": opened by BillyDM, endorsed by the clack author, who suggests a gradual migration.
- [PR #96](https://codeberg.org/RustAudio/nice-plug/pulls/96): fixes spawning editors in the VST3 backend. Open since 2026-10-05.
- [PR #15](https://codeberg.org/RustAudio/nice-plug/pulls/15): lets the plugin update its own parameters. Stalled because the community pushed back.

**Licence of nice-plug itself:** the repo's `LICENSE` at `7197104` is the ISC licence, "Copyright (c) 2022-2024 Robbert van der Helm" *(documented, I read the file)*. The logos in `branding/` are CC BY-SA 4.0, which doesn't matter unless we ship the logo. The **crates.io packages contain no LICENSE file**, only `license = "ISC"`. Since ISC requires keeping the notice, the text belongs in `THIRD_PARTY_NOTICES.md`.

**Licences of the dependency tree (measured, 2026-10-19).** Two scratch crates were used:

| Configuration | Crates (Linux, normal + build) | `cargo deny check licenses` with the Task 2 allow-list |
|---|---|---|
| `nice-plug =0.4.2` (`assert_process_allocs`) + `nice-plug-egui =0.6.0` | 151 | **Fails: 1 crate.** `nice-assert-no-alloc` 1.0.0 is `BSD-1-Clause` (LICENSE file: Florian Jung's assert_no_alloc, one clause) |
| GUI-less spike (`nice-plug =0.4.2`, `default-features=false, features=["vst3","assert_process_allocs"]`) | 83 | Same failure, same crate |
| Editor spike (`nice-plug` + `nice-plug-egui 0.6.0` + `egui` with `default_fonts`) | — | **Fails: 1 crate.** `epaint_default_fonts` 0.37.0 is `(MIT OR Apache-2.0) AND OFL-1.1 AND Ubuntu-font-1.0`. `Ubuntu-font-1.0` is a real SPDX id, so `LicenseRef-UFL-1.0` doesn't match it |

- Everything else is MIT, Apache-2.0, ISC, Zlib, Unicode-3.0 or a dual licence that includes one of them. For example, `self_cell` is `Apache-2.0 OR GPL-2.0-only`, and `rustix`/`linux-raw-sys` offer MIT among their options.
- `epaint_default_fonts` has declared `Ubuntu-font-1.0` since at least 0.36.0, published 2026-08-05 ([crates.io](https://crates.io/crates/epaint_default_fonts)).
- The `assert_no_alloc` crate that Task 5 plans for engine tests is also `BSD-1-Clause`. I checked the LICENSE file in its 1.1.2 crate (last release 2021-08-03).

**`vst3` versus `vst3-sys` (measured):** `cargo tree -i vst3` shows `vst3 v0.3.0 → nice-plug v0.4.2`, and `cargo tree -i vst3-sys` finds no such package. The vst3 0.3.0 crate's LICENSE-MIT and LICENSE-APACHE files are present (`MIT OR Apache-2.0`), and it was published 2025-12-07 ([vst3-rs](https://github.com/coupler-rs/vst3-rs); last push 2026-03-28). nice-plug switched in PRs [#28/#29](https://codeberg.org/RustAudio/nice-plug/pulls/29) (2026-05-27).

### Q2 — nice-plug against ADR-0001's must-haves

Local spike: `/tmp/enth-research/01/spike-np` (GUI-less) and `spike-np-gui` (nice-plug-egui 0.6.0). Each is an 8-voice saw with a one-pole low-pass, 8 macros in `#[nested(array)]` with poly-mod IDs 0–7, 64 slots, and `#[persist = "patch"] patch: RwLock<String>`. I bundled them by copying the `.so` to `.clap` and to `.vst3/Contents/x86_64-linux/`. To read parameters and save state I used a small ctypes CLAP host, and I built clap-validator 0.4.1 from its tag (debug build).

| # | Must-have | Verdict | Evidence |
|---|---|---|---|
| a | 72 fixed host params, stable IDs | **Yes** (measured) | My host read `param count: 72`. A CLAP ID is a 31-bit multiplicative hash of the string ID, e.g. `macro_1` → `0x3135e47e` ([`hash_param_id`](https://codeberg.org/RustAudio/nice-plug/src/branch/main/crates/nice-plug/src/wrapper/util.rs), top bit cleared for VST3). Saved state keys parameters by **string ID**. So IDs stay stable while the `#[id]` strings do. Duplicate hashes trigger a debug warning (`clap/wrapper.rs` ~L560). Nested arrays name the IDs `macro_1…macro_8` and `slot_1…slot_64`, and each gets its own module ("Macro 1", "Slot 7"), which is cosmetic |
| b | Runtime slot renaming | **No in 0.4.2** (documented, from code) | `ParamInfo { name: String }` is set in `FloatParam::new` and read through `Param::name(&self) -> &str`, with no setter ([params.rs](https://codeberg.org/RustAudio/nice-plug/src/branch/main/crates/nice-plug-core/src/params.rs)). The only `rescan` calls use `CLAP_PARAM_RESCAN_VALUES` (`clap/wrapper.rs` L424, L475). VST3 never sends `kParamTitlesChanged`. CLAP allows `CLAP_PARAM_RESCAN_INFO` for a "name change … New info takes effect immediately", while active ([params.h, CLAP 1.2.10](https://github.com/free-audio/clap/blob/main/include/clap/ext/params.h)). A fix needs mutable names (an API change to the sealed `Param` trait), `rescan(INFO)` on the main thread, and a VST3 restart flag. Rough size: 100–200 lines, an estimate |
| c | Arbitrary patch blob saved with the project | **Yes** (measured) | CLAP `state.save` wrote 1,677 bytes: an 8-byte LE length, then JSON `{"version":…,"params":{"macro_1":{"f32":0.0},…},"fields":{"patch":"…"}}`. A `#[persist]` field is serialised to a JSON *string* and stored as a string, so a `String` holding JSON is **escaped twice** (each `"` becomes `\\\"`). Persisting a typed `Serialize` struct gives one level of escaping instead, or enable the `zstd` feature. clap-validator's `state-reproducibility-{basic,buffered,binary}` and `state-invalid-*` tests passed. The in-REAPER save/reload is not tested |
| d | CLAP poly-mod on macros | **Declared correctly** (measured); host behaviour untested | Macros report `AUTOMATABLE \| MODULATABLE \| MODULATABLE_PER_NOTE_ID`. `clap.voice-info` is present and `param-fuzz-modulation` passed. The API is `.with_poly_modulation_id(id)`, `CLAP_POLY_MODULATION_CONFIG`, `NoteEvent::PolyModulation`/`MonoAutomation`, and the plugin must send `VoiceTerminated` ([poly_mod_synth example](https://codeberg.org/RustAudio/nice-plug/src/branch/main/examples/poly_mod_synth/src/lib.rs)). I found nothing documenting REAPER's support for CLAP poly-mod. Bitwig remains the reference host |
| e | VST3 export | **Builds; loading untested** (measured) | The `.so` exports `GetPluginFactory`, `ModuleEntry`, `ModuleExit` and `clap_entry` (`nm -D`). No VST3 host or validator here: `cmake` is missing, so Steinberg's validator can't be built, and there is no native Linux host |
| f | Passes `clap-validator` | **Yes** (measured) | 0.4.1, GUI-less: **44 run, 36 passed, 0 failed, 0 warnings, 8 skipped**. The skips are preset-discovery ×3, 64-bit audio ×2, denormals (no input ports), audio-ports-activation and configurable-audio-ports, all because the extension isn't implemented. The editor build gave the identical result. `fuzz -j2 -d90s`: "OK: fuzzing finished successfully". clap-validator has **no GUI tests**, so this says nothing about the editor. Two validator failures (#84 in-place port pairing, #85 huge state length) were fixed in 0.4.0. Code reading: `ext_params_get_info` checks `param_index > count` where it should check `>=`; latent and harmless with sane hosts |
| g | Stable Rust | **Yes** (measured) | Built on rustc 1.95.0 stable. The only nightly feature is `simd`, which is optional ([Cargo.toml](https://codeberg.org/RustAudio/nice-plug/src/branch/main/crates/nice-plug/Cargo.toml)) |

**Build numbers (measured, under contention):**
- GUI-less spike, cold: **1 min 40 s**, peak RSS 341 MB.
- Editor spike, cold: **7 min 35 s**, peak RSS 960 MB. The debug `.so` is 194 MB with debuginfo.
- clap-validator debug build: **76 s**.

**Allocation check (measured):** the GUI-less spike ran all validator tests and the 90-second fuzz with `assert_process_allocs` in a debug build, and none of them aborted. nice-plug wraps its **whole** CLAP/VST3 process function, wrapper code included, in `assert_no_alloc` (`process_wrapper` in `wrapper/util.rs`).

**Denormals:** the wrapper sets FTZ only with the `unsafe_flush_denormals` feature, which is off by default. That is good for determinism, but a host may still set FTZ itself, which matches design.md §5's plan to flush in code *(documented)*.

### Q3 — Editor API, egui adapter, repaint gating, sizing, HiDPI

**Editor API (nice-plug-core 0.4.2, documented):**
- `Plugin::Editor` is an associated type, `()` when there is no editor.
- `Editor::spawn(parent, wait_for_parent, fallback_scale_factor, GuiContext, Option<HostMethods>) -> SpawnedEditor`.
- `EditorHandle` has `set_size`, `adjust_size`, `set_fallback_scale_factor`, `param_value_changed`, `state_changed` and `host_main_thread_callback`.
- `HostMethods` supplies `request_resize`/`destroyed` and a `HostMainThreadCaller`. That caller is needed only on X11; in CLAP it is `request_callback`.
- `GuiContext` gives `param_setter()` (begin/set/end gestures), `get_state`/`set_state` and `request_restart`.
- Source: [editor.rs](https://codeberg.org/RustAudio/nice-plug/src/branch/main/crates/nice-plug-core/src/editor.rs).
- The egui side implements `NiceEguiApp` (`build`, `ui`, `editor_closed`, `resized`, `track_info_changed`) and passes it to `create_egui_editor(state, repaint_notifier, settings, app)` ([gain_egui](https://codeberg.org/RustAudio/nice-plug/src/branch/main/examples/gain_egui/src/lib.rs)).

**Does it still redraw continuously? No (documented):**
- The nice-plug 0.3.0 / nice-plug-egui 0.4.0 changelog (2026-08-17) says: "`nice-plug-egui` no longer automatically repaints every frame. Use the new `RepaintNotifier`."
- In 0.6.0 (egui-baseview 0.9.0, baseview 0.4.1) a redraw happens only when:
  - egui requests one (`request_repaint`, animations), through `waker.request_redraw_after`;
  - input events arrive;
  - the host changes a parameter or the state, which calls `request_repaint` through the `EditorHandle` hooks;
  - a timer that checks the `RepaintNotifier` flag every 1/64 s by default (`EguiNiceSettings::with_repaint_poll_interval`) finds it set ([editor.rs](https://codeberg.org/RustAudio/nice-plug/src/branch/main/crates/nice-plug-egui/src/editor.rs); egui-baseview 0.9.0 `window.rs` `poll_repaint_timer`).
- So the idle cost while open is about 64 timer wake-ups a second with no rendering. **I didn't measure the CPU this costs.**
- baseview made lazy redraws the default in [#340](https://github.com/RustAudio/baseview/pull/340), merged 2026-10-09. That broke `RepaintNotifier` in egui-baseview 0.8.x: meters froze until the mouse moved ([egui-baseview #37](https://codeberg.org/RustAudio/egui-baseview/issues/37)). It was fixed with baseview's new timers ([#350](https://github.com/RustAudio/baseview/pull/350), 2026-10-08) in egui-baseview 0.9.0. **So pin 0.6.0 and avoid 0.5.x mixed with egui-baseview 0.8.x.**
- Gating pattern from the example:
  - The audio thread computes meter values only while `editor_state.is_open()`.
  - It calls `repaint_notifier.request_repaint()` only when a value changed.
  - The UI calls `ui.request_repaint()` only while a meter is decaying.

**Sizing (documented):**
- `ResizeHint` sets resizable or not, min/max logical or physical size, and aspect ratio. It answers CLAP `can_resize`/`get_resize_hints`/`adjust_size` and VST3 `canResize`.
- Host-driven resizing works in both CLAP and VST3 (PR #48), and the plugin can request a resize through `request_resize`.
- The window size is no longer stored in preset state (0.3.0), because the host keeps track of it.
- egui zoom is supported (`ui.set_zoom_factor`).
- There are **no floating windows** (`is_floating` → false; a patch was offered in #88 and closed) and **no child windows** (baseview [#216](https://github.com/RustAudio/baseview/issues/216) is open).

**HiDPI (documented/anecdotal):**
- CLAP `set_scale` is passed to baseview as a *fallback* scale (`clap/wrapper.rs` `ext_gui_set_scale`).
- On X11, baseview takes `Xft.dpi` first and uses the host's value only as a fallback. An open issue argues the host value should win ([baseview #359](https://github.com/RustAudio/baseview/issues/359), 2026-10-15). It also reports that "Reaper never calls set_scale" on Linux (anecdotal).
- On your Cinnamon/X11 setup the scale therefore comes from `Xft.dpi`.

**Keyboard (documented):**
- egui-baseview hands keyboard events back to the DAW when egui doesn't want them, so the spacebar reaches the host.
- nice-plug-egui has a `keyboard_focus_workaround` feature for a known focus problem ([egui-baseview #20](https://codeberg.org/RustAudio/egui-baseview/issues/20), Windows).
- `on_virtual_key_from_host` is a `TODO` (`false`), so VST3 host-forwarded keys are dropped.

**Build dependencies (measured):**
- baseview 0.4.1 on Linux uses `x11-dl`, `x11rb` with `dl-libxcb`, `xkbcommon-dl` and `libloading`, so everything is loaded at runtime.
- The editor `.clap` links only `libgcc_s`, `libm` and `libc` (`ldd`). It built **without any of the X11 `-dev` packages from Task 1**.
- The runtime libraries (`libGL.so.1`, `libX11-xcb.so.1`, `libxcb.so.1`, `libxkbcommon-x11.so.0`, `libXcursor.so.1`) are already installed.
- Only the `standalone` feature (cpal/JACK) needs `libasound2-dev`/`libjack-dev`.

### Q4 — clack, clap-wrapper and the cost of switching

**clack, plugin and host sides (documented, 2026-10-19):**
- Versions: `clack-plugin`, `clack-host`, `clack-extensions` and `clack-common` 0.2.0 were released 2026-09-12 as "v0.2 – Embracing re-entrancy", after 0.1.0 (05-03) and 0.1.1 (07-29). Licence `MIT OR Apache-2.0`, with both files in the repo; MSRV 1.85.
- Downloads: clack-plugin has 36,992 in total ([crates.io](https://crates.io/crates/clack-plugin)).
- Repo: 241 stars, last push 2026-09-12, 4 open issues/PRs. One is an open soundness issue in `Audio` borrowing ([#56](https://github.com/prokopyl/clack/issues/56), 2025-12-21).
- Maintainers: prokopyl has 339 commits; the next contributor has 11 ([repo](https://github.com/prokopyl/clack)).
- Its README: "Clack is now in a feature-complete status, however … APIs can still have breaking changes" and points "normal" plugins to nice-plug.
- `clack-extensions` 0.2.0 covers all the stable extensions we'd need, including `params`, `state`, `gui`, `voice-info`, `note-ports`, `timer`, `posix-fd`, `track-info`, `remote-controls` and a `clap-wrapper` extension.
- prokopyl is also baseview's most active committer now (148 commits; did the 0.4.0/0.4.1 work) and a crates.io owner of baseview. Moving to clack does not escape the GUI dependency.

**clap-wrapper, C++ (documented):**
- 0.16.0 was released 2026-08-08. It brought AUv2 MIDI 2.0 and fixes, and "VST3: parameter values are synced from the CLAP plugin after state load" ([release](https://github.com/free-audio/clap-wrapper/releases/tag/v0.16.0)).
- 0.15.1 (2026-07-13) added AAX and AUv3, VST3 `IPluginCompatibility`, and removed GTK from the Linux standalone ([release](https://github.com/free-audio/clap-wrapper/releases/tag/v0.15.1)).
- MIT, 294 stars, 48 open issues, last push 2026-10-15.

**clap-wrapper-rs, the Rust crate `clap-wrapper` (documented):**
- 0.3.1 on crates.io (2026-05-18) bundles **clap-wrapper 0.14.0**. Its `main` updated to 0.16.0 on 2026-08-20 but **hasn't been released**.
- It builds VST3 and AUv2 through the `cc` crate, with the SDKs embedded thanks to VST3's MIT licence. It has **no standalone**.
- `MIT OR Apache-2.0`; 34 stars; sole crates.io owner Quant1um ([repo](https://github.com/blepfx/clap-wrapper-rs)).
- Its CI builds Steinberg's VST3 `validator` with cmake and runs it on the bundles ([validate.yml](https://github.com/blepfx/clap-wrapper-rs/blob/HEAD/.github/workflows/validate.yml)).
- g++ 13.3 and clang 18 are installed here; cmake is not.

**What switching would cost (my estimates, not validated):**
- nice-plug's wrapper is 11,165 lines (the CLAP `wrapper.rs` alone is 3,550; measured with `wc -l`). We'd re-implement the subset we use:
  - entry/factory;
  - the params extension with gestures and flush;
  - state;
  - note/audio ports;
  - voice-info and poly-mod events;
  - the GUI extension glued to baseview + egui-baseview's `Host` API — the riskiest part, where nice-plug's last three months of host fixes live;
  - main-thread task queue;
  - bundling.
- We'd gain renaming (we control `rescan`), full CLAP coverage, and no dependence on BillyDM.
- We'd lose standalone, nice-plug's maintained parameter/smoother/state machinery, and the egui adapter glue.
- VST3 would come through clap-wrapper-rs: a C++ build step, a crate that lags upstream, and VST3 parameter-title changes depending on clap-wrapper's translation, which is unverified.
- **About 2–4 weeks of work**, against "about 2 days" in ADR-0001's decision rule. The clack-host test host (Task 4) is wanted anyway, and it would de-risk any later move.

### Q5 — Rust CLAP plugins in Linux hosts, and native versus Wine REAPER

**Known issues, 2025–2026:**

| Host | Issue | Status | Label |
|---|---|---|---|
| REAPER 7.73/7.79 (Linux) | baseview editor sometimes blank on first open; also reproduced with the clack + baseview example ([nice-plug #95](https://codeberg.org/RustAudio/nice-plug/issues/95)) | Maintainer: "should be fixed in the latest commit" (2026-10-18); reporter hasn't confirmed | anecdotal |
| REAPER (Linux) | Never calls CLAP `set_scale` ([baseview #359](https://github.com/RustAudio/baseview/issues/359)) | Open | anecdotal |
| Bitwig 6 (flatpak, XWayland) | clack + baseview editor black: baseview's X11 visibility tree treated unmapped XWayland parents as invisible ([baseview #331](https://github.com/RustAudio/baseview/issues/331)) | Closed 2026-10-09 | anecdotal + fix |
| Bitwig (Linux, CLAP) | `already mutably borrowed` panic when latency changes from `process` ([nice-plug #71](https://codeberg.org/RustAudio/nice-plug/issues/71)) | Fixed in 0.4.0 | anecdotal + fix |
| Bitwig (Linux, VST3) | nih-plug Spectral Compressor UI black; "software GL may fix" ([nih-plug #271](https://github.com/robbert-vdh/nih-plug/issues/271)) | Open | anecdotal |
| Ardour 8.4 (VST3) | nih-plug Spectral Compressor crashes Ardour ([nih-plug #259](https://github.com/robbert-vdh/nih-plug/issues/259)) | Open, old nih-plug | anecdotal |
| Carla (VST3) | Editor resize loop / thread panic ([nice-plug #76](https://codeberg.org/RustAudio/nice-plug/issues/76)) | Fixed through a baseview commit, confirmed 2026-09-06 | anecdotal + fix |
| All X11 hosts | baseview runs its own X11 thread and plans to move to host timer/fd callbacks ([#342](https://github.com/RustAudio/baseview/issues/342), [PR #357](https://github.com/RustAudio/baseview/pull/357)) | Open; will break the API | documented |

**Ardour (documented):** the 9.8 release notes (2026-08-23) mention LV2 and VST3 only. The CLAP request on Ardour's tracker ([9417](https://tracker.ardour.org/view.php?id=9417)) was open in its last visible update (2024-08), and the maintainers said then that they had no plans for it. No evidence that Ardour supports CLAP today ([whatsnew](https://ardour.org/whatsnew.html)). **Ardour can only test the VST3 build.**

**Bitwig (documented):** 30-day trial. After it ends, the program still runs but saving and exporting are blocked ([Bitwig support](https://bitwig.com/support/shop_license_activation/48); [try page](https://www.bitwig.com/try/): Ubuntu 24.04+ or Flatpak). The trial is once per account and computer, so start it when Task 3 (d)/(h) and Task 4 need it.

**Native REAPER versus Wine REAPER:**
- REAPER under Wine/Bottles is the Windows build. It loads only Windows (PE) plugin binaries, and a Linux `.clap` or `.vst3/Contents/x86_64-linux/*.so` is an ELF shared object it cannot load. *Inference from the binary formats; no source found that says so explicitly.*
- Testing a Windows build under Wine would need the `x86_64-pc-windows-gnu` target and a MinGW linker, neither installed (the linker needs sudo). It would exercise baseview's Win32 backend under Wine, not the X11 path we ship on Linux.
- Native REAPER is **7.82 (2026-10-04)**, offered as `reaper782_linux_x86_64.tar.xz` ([download](https://www.reaper.fm/download.php)).
  - It runs straight from the extracted folder, or `install-reaper.sh` installs it under `~/opt` for your user. Sources: Cockos wiki, which was blocked by a bot check so I only saw it in search snippets; an Ubuntu Studio guide (third-party).
  - The evaluation: "You get 60 days of evaluation free, with full functionality" ([purchase](https://www.reaper.fm/purchase.php)).
  - It can sit alongside the Wine copy, since configuration lives in `~/.config/REAPER` versus the Wine prefix.

### Q6 — clap-validator

**Status (documented):**
- 0.4.1 was released 2026-07-22; the CHANGELOG dates it 2026-07-19. It follows 0.4.0 (2026-03-28) and **ends a gap since 0.3.2 in 2023** ([releases](https://github.com/free-audio/clap-validator/releases/tag/0.4.1)).
- New in 0.4.x: an experimental multi-process **fuzzer** (`clap-validator fuzz`), parameter-rescan checks in the state tests, `render`, out-of-bounds write checks, about 25 new tests, `--include`/`--exclude` regex filters, a `clap-validator.toml` config, a 45-second timeout per test, and Perfetto traces.
- **MSRV 1.95.0**, which is exactly your toolchain.
- MIT ([LICENSE](https://github.com/free-audio/clap-validator/blob/HEAD/LICENSE)), 112 stars, last push 2026-07-24.
- Maintainers: robbert-vdh (379 commits) and Quant1um (119).
- The release zips are named `…-0.4.1-127-g152b982-ubuntu-22.04.zip` and others, but their inner tarballs say 0.3.2 and the layout differs per asset ([#39](https://github.com/free-audio/clap-validator/issues/39)).

**How projects run it in CI (documented):**
- GitHub code search found 80 workflow files with `clap-validator … validate` (2026-10-19).
- OctaSine (Rust) runs `cargo install --git https://github.com/free-audio/clap-validator.git`, then `clap-validator validate --invert-filter --test-filter 'param-conversions' …`. That is **the 0.3 filter syntax**; 0.4 uses `--include`/`--exclude` ([ci.yml](https://github.com/greatest-ape/OctaSine/blob/HEAD/.github/workflows/ci.yml)). So pin a version.
- truce wraps clap-validator, pluginval and auval in `cargo truce validate`.
- clap-wrapper-rs builds Steinberg's VST3 validator with cmake.
- Building 0.4.1 from its tag took **76 s (debug)** here (measured). It needs no system libraries.

### Q7 — New contenders since 2026-10-18

None found (web search and crates.io "newest" searches on 2026-10-19). Recent projects that aren't in research.md yet, for the watch list only:
- **truce** ([repo](https://github.com/truce-audio/truce)):
  - Covers CLAP, VST3, VST2, LV2, AU, AAX and standalone; hot reload; screenshot tests in CI; `cargo truce validate`.
  - Licence: **"The Truce License"** (`LicenseRef-TruceLicense-1.0`), MIT/Apache plus a "Framework Rider" restriction, so it **fails our licence gate**.
  - It has a single maintainer (1,639 commits) and published 82 versions between 2026-05-22 and 2026-07-18, with no push since.
- **lx-aura** 0.14.1 (2026-10-16): GPL-3.0-or-later, reference only.
- **plughost** 0.1.3 (updated 2026-10-18): MIT OR Apache-2.0. A VST3/AU/CLAP *host* that runs plugins in helper processes, relevant only to any future hosting question.

### Q8 — `request_callback`, timer-support, posix-fd-support, child windows (open check #10)

All documented, from code at `7197104`:
- **`request_callback`:** used internally by the CLAP wrapper's main-thread task queue (`Wrapper::schedule_gui`). Plugin code reaches it indirectly through `AsyncExecutor::execute_gui`, and editors through `HostMainThreadCaller::call_main_thread`. Plugin code gets no raw host pointer.
- **timer-support and posix-fd-support:** the CLAP wrapper neither implements nor queries `clap.timer-support` or `clap.posix-fd-support`, which are absent from `get_extension` and from host-extension queries. The VST3 wrapper on Linux does use `IRunLoop` with an fd event handler. baseview's own X11 thread does the GUI work today. That will change if baseview [#342/#357](https://github.com/RustAudio/baseview/issues/342) lands, and nice-plug would then have to implement the host timer/fd glue.
- **Child windows:** one embedded editor per instance. Floating editors are rejected and baseview can't create child windows ([#216](https://github.com/RustAudio/baseview/issues/216)).
- **Bottom line for hosting:** nice-plug 0.4.2 doesn't expose what's needed to relay a hosted plugin's Linux GUI. That would need a fork or clack.

## Recommendation for Phase 0

1. **Keep nice-plug for the Task 3 spike.** Pin `nice-plug = "=0.4.2"` and `nice-plug-egui = "=0.6.0"`, which pulls in egui 0.37.0, egui-baseview 0.9.0 and baseview 0.4.1, and commit `Cargo.lock`. Don't pin git `main`; everything on `main` is already on crates.io.
2. **Expect (a), (c), (f) and (g) to pass quickly, and (e) to build.** The GUI-less spike above shows this. Spend the spike time on what isn't covered:
   - (b) renaming;
   - the editor in native Linux REAPER: first open, repaint on automation with the mouse still, close and reopen;
   - (d) in Bitwig;
   - (h).
3. **Decide renaming explicitly.** Either accept "Slot N" for v1, as ADR-0001 already allows, or carry a small patch in a fork: mutable names, `rescan(CLAP_PARAM_RESCAN_INFO)`, VST3 `kParamTitlesChanged`. The RustAudio AI policy means an agent-written upstream PR isn't an option. First check whether REAPER honours `RESCAN_INFO` at all; if it doesn't, the patch buys nothing in REAPER.
4. **Fix the licence plumbing in Task 2:**
   - in `deny.toml`, allow `Ubuntu-font-1.0` instead of `LicenseRef-UFL-1.0`;
   - allow `BSD-1-Clause`, either globally (ADR-0004 amendment) or as an exception for `assert_no_alloc` and `nice-assert-no-alloc`;
   - add nice-plug's ISC text to the notices file.
5. **Machine setup for Task 1:**
   - install native Linux REAPER 7.82 and point it at `~/.clap`;
   - reboot to fix the NVIDIA mismatch before any editor check;
   - start the Bitwig trial only when (d)/(h) are scheduled;
   - use Ardour only for VST3;
   - build clap-validator from the `0.4.1` tag (or use the ubuntu-22.04 zip) instead of guessing release asset names;
   - drop the X11 `-dev` apt packages unless the standalone target is wanted. Keep `libasound2-dev`/`libjack-dev` only for `standalone`.
6. **CI:** add `clap-validator validate --only-failed` with a committed `clap-validator.toml` on every PR, and a short `fuzz -j2 -d2m` in a nightly job. Steinberg's VST3 validator needs cmake and can come later.
7. **Watch baseview #357.** When it merges, expect nice-plug-egui 0.7 with a host timer/fd API. Budget about a day for it, which is the sort of churn ADR-0001's "a day per month" tripwire measures.

## Plan check — what in the current docs the evidence says should change

- **ADR-0001 (keep nice-plug; change details).** The evidence supports nice-plug as the starting shell. None of the hard must-haves failed in a GUI-less spike, and clack + clap-wrapper costs weeks. Changes:
  - (1) Name the pin: nice-plug 0.4.2 + nice-plug-egui 0.6.0.
  - (2) State that (b) **fails by design in 0.4.2** and what a patch involves.
  - (3) Add the editor-specific risks the must-have table misses: REAPER-Linux first-open rendering (#95) and baseview's coming X11 main-thread change.
  - (4) Under Consequences, record the bus factor of 1 (BillyDM) and the RustAudio AI policy, which together mean enth carries its own patches.
  - (5) The decision rule (a, c, e, f, g within two days) is very unlikely to trigger. Consider adding an editor row to it, because the real way this could fail is a blank or frozen editor in REAPER.
- **phase-0.md Task 3:**
  - Add a must-have row "(i) editor in native Linux REAPER: opens on first insert 10/10, repaints on host automation with the mouse still, survives 20 close/reopen cycles".
  - (h) should say that Ardour tests VST3 only and that Bitwig is the only second host for CLAP.
  - Debug bundles with `assert_process_allocs` need the `BSD-1-Clause` allowance.
  - The "template" is a cookiecutter (`uvx cookiecutter https://codeberg.org/RustAudio/nice-plug-template`, last updated 2026-09-14), so it may lag 0.6.0.
  - The bundler is `nice-plug-xtask` 0.1.1 on crates.io; on Linux a bundle is just a renamed `.so`.
- **phase-0.md Task 1:**
  - Add "install native Linux REAPER; Wine REAPER can't load Linux plugins".
  - Add "reboot for the NVIDIA mismatch".
  - The X11 `-dev` apt list isn't needed to build with baseview 0.4.1 (measured), and `libasound2-dev`/`libjack-dev` only matter for `standalone`. Keep the list if the CI image also builds something else that needs it.
  - clap-validator from the `0.4.1` tag.
- **phase-0.md Task 2 and ADR-0004:** replace `LicenseRef-UFL-1.0` with `Ubuntu-font-1.0`, and decide on `BSD-1-Clause` (recommendation: allow it; it is more permissive than BSD-2-Clause). ADR-0004 is Accepted, so this needs a dated amendment note.
- **ADR-0002, Context:** "egui's plugin adapter historically redraws continuously" is out of date. Since 2026-08-17 it repaints on demand, and in 0.6.0 that means lazy redraws plus a 64 Hz flag check. The consequence "we must implement repaint gating" shrinks to "measure idle cost and use `RepaintNotifier` only on change".
- **research.md §3:**
  - Update to nice-plug 0.4.2 (2026-09-14) and nice-plug-egui 0.6.0 (2026-10-18).
  - egui-baseview is now `codeberg.org/RustAudio/egui-baseview` (0.9.0, 2026-10-17), owned by BillyDM.
  - baseview 0.4.1 (2026-10-09) is led by prokopyl.
  - clap-wrapper-rs 0.3.1 lags clap-wrapper 0.16.0.
  - clap-validator 0.4.1 (2026-07-22).
  - The `vst3` check is done at 0.4.2: `vst3` 0.3.0, no `vst3-sys`.
- **design.md §4:** add that refreshing slot names needs CLAP `RESCAN_INFO` and VST3 `kParamTitlesChanged`, and that nice-plug 0.4.2 has neither.
- **Open check #10 (previous session):** answered by Q8 above. The licence part is closed; the hosting part is "not exposed".
- **What stays unchanged:** Rust, a native non-WebView UI, CLAP first with VST3 second, egui as the default UI, and clack-host for the test host.

## Gaps — what I couldn't verify, and the local check that would settle it

| Gap | Why | Check that settles it |
|---|---|---|
| Behaviour in any real host for (a), (c), (d), (e) and (h) | No native Linux REAPER, Bitwig or Ardour installed | Install REAPER 7.82 (tarball, `~/opt`); copy `/tmp/enth-research/01/bundles/spike_np*.clap` to `~/.clap`; automate a macro and a slot, then save and reload; try the Bitwig trial for poly-mod |
| VST3 loads | No VST3 host; `cmake` is missing for Steinberg's validator | Load the `.vst3` in native REAPER or Ardour 9.8, or build `vst3sdk`'s `validator` target |
| Editor at runtime: first-open blank (#95), repaint with the mouse still, idle CPU of the 64 Hz poll, HiDPI | No host; NVIDIA driver mismatch | After a reboot, open and close the `spike_np_gui` editor 20× in REAPER; `pidstat`/`perf stat` with the editor open and idle |
| Whether REAPER honours `CLAP_PARAM_RESCAN_INFO` (name refresh) and CLAP poly-mod | Nothing documented | A clack-plugin test plugin (or a patched nice-plug) that renames a parameter at runtime and calls `rescan(INFO)` |
| Effort to switch to clack + clap-wrapper | Estimates only; `clap-wrapper` crate not built here | Build clap-wrapper-rs's `example-clack` (needs g++, present) and time the VST3 bundle |
| Cockos wiki page "REAPER for Linux" | Bot check blocked the fetch | Read it in a browser |
| nice-plug organisation membership | Codeberg API requires a token | Log in to Codeberg, or ask in the Rust Audio Discord |
| Whether nice-plug #95 is really fixed | The reporter hasn't confirmed | The editor test above in REAPER on Linux |
| Fuzzer coverage | `fuzz` printed only "OK" | Run `fuzz --json -j2 -d10m` |
| Incremental rebuild time | I recorded 1.1 s after a `touch` but didn't confirm it was a real rebuild (axis 6 covers this) | Run `cargo build -v` after a real edit |
