# 06 — Dev-loop speed for AI + human iteration (final)

- Date: 2026-10-19
- Sub-researcher: 06 (dev-loop speed)
- Scope: all six original questions, plus every item left under Gaps in the first report (`06-dev-loop-speed.md`, which stays as written). This report stands on its own and replaces the first report's numbers and advice where they differ.
- Marks:
  - *measured*: run on the owner's laptop today, with the command and conditions given.
  - *documented*: official docs, release notes, source code, registries.
  - *anecdotal*: forums, blogs, third-party projects.
  - *inference*: my reasoning, not checked.
- Change tags: **[changed]** contradicts or corrects the first report. **[new]** was not in it. **[confirmed]** was in it and now has better evidence.

## How and where this was measured

- **Machine**: the owner's Acer ConceptD 5. Linux Mint 22.3, kernel 7.0.0-38, 12 threads, 15 GiB RAM. The repo and scratch space are on the internal NVMe (ext4).
  - After the reboots: no swap in use, 6–12 GiB available, NVIDIA driver 595.99.02 matching.
  - Every GUI or GL process ran under `xvfb-run`, with `DISPLAY` unset everywhere else.
- **Lock discipline**: timings ran under `flock -x /tmp/enth-quiet.lock`, with `CARGO_BUILD_JOBS=12` unless noted, and `uptime` logged with every run.
- **Noise**: other projects ran outside the lock (another project's cargo/flutter/python jobs, a researcher's node run, cinnamon), so the 1-minute load average was 2–21 during timings. Each row gives its load.
  - Clean builds of the same configuration varied by about ±15% between runs (e.g. 1.95.0 lean: 59.8 s and 68.3 s).
  - For that reason, incremental comparisons alternate configurations ("interleaved") over 5 rounds and report the median.
- **Workloads**:
  - **toy**: a three-crate workspace. `leaf` (DSP) → `mid` → `app`, a binary on `eframe 0.37.0` with default features, which pull in wgpu 30.0.1 and winit 0.30.13 (425 packages locked).
  - **enth**: a git clone of `/home/vboxuser/code/enth` at commit `3fb2278`, with toolchain 1.99.1 from its `rust-toolchain.toml`. The main target is `spike-shell` (nice-plug 0.4.2 + nice-plug-egui 0.6.0 + egui 0.37.0; cdylib + lib).
  - "Incremental" means a real constant change in the DSP code, then a rebuild: `leaf`'s `SALT` in the toy, `VOICE_SCALE` in `spikes/spike-shell/src/synth.rs` in enth.
- **Profiles compared**:
  - **default**: no `[profile]` section, as in the repo today.
  - **lean**: `profile.dev.debug = "line-tables-only"` plus `profile.dev.package."*".debug = false`, passed with `--config`.
- **Raw data and scripts**: `~/code/enth-scratch/2026-10-19-phase-0-spikes/06/`.
  - `notes.md` is the running log; `results/quiet.tsv` has every timing with load and peak RSS.
  - `scripts/m.sh` is the timer; `scripts/ld-switch.sh` is the linker wrapper.
  - The `reaper-tests/*.lua` scripts and the `miniclap/` test plugin are there too.
  - Earlier noisy rows are in `results/times.tsv`.

## Answer in brief

1. **No linker install is needed for Phase 0.** **[changed]**
   - rustc's bundled `rust-lld` has been the default since Rust 1.90 *(documented)*.
   - On the real plugin, with the lean profile, the link takes **0.21 s of a 0.66 s incremental build**. wild cuts it to 0.10 s, mold to 0.17 s; GNU ld takes 1.87 s *(measured)*.
   - On the heavier eframe + wgpu toy, the link is 0.82 s of 1.69 s; wild brings it to 0.28 s (−40% total) and mold to 0.52 s.
   - Adopt wild later, when the plugin's link grows.
2. **Adopt the lean dev profile.** **[confirmed]** On `spike-shell` *(measured)*:
   - clean build 36.2 → 26.9 s;
   - incremental build 0.88 → 0.66 s;
   - `.clap`/`.so` 191 → 54 MB;
   - peak RSS of an incremental build 0.82 → 0.36 GB.
3. **Optimise named DSP crates; don't optimise all dependencies.** **[changed]**
   - Dependencies at `opt-level = 2` made the toy's clean build **2.95×** slower (247 s vs 84 s). The first report's 6× was noise. `opt-level = 1` cost 2.05×. Either saves only about 0.4 s per incremental build *(measured)*.
   - `spike-shell` itself doesn't need optimising: its unoptimised debug build renders 8 voices at **45× realtime** in REAPER, against 54× at `opt-level = 3` and 54× for release *(measured)*.
4. **Stay on stable.** **[changed]**
   - 1.99.1, the repo's pin and the current stable, builds about as fast as 1.95.0 (clean 56 vs 60 s, incremental 1.07 vs 1.24 s).
   - Nightly `-Zthreads=8`: −6% clean, no incremental gain.
   - Cranelift: +18% clean and no incremental gain on this workload. The edited crates are tiny, so the link dominates.
5. **[new] The AGENTS.md "Done means" gate takes about 3 s** on the actual repo after a DSP edit *(measured)*: `fmt --check` 0.10 s, `clippy --workspace --all-targets -D warnings` 0.37 s, `test --workspace` 1.28 s (1.00 s lean), `xtask check-layers` 0.11 s, `xtask wasm-check` 0.21 s, `cargo deny --offline check` 1.04 s.
6. **[new] REAPER renders headless from the command line.** `reaper -nosplash -newinst -renderproject p.rpp` takes 1.1–1.5 s including startup.
   - Headless runs need three seeded `reaper.ini` keys to skip a first-run dialog, and `linux_audio_mode=2` (Dummy Audio).
   - REAPER reloads a rebuilt **trivial** CLAP in-process once its last instance is removed, but **not `spike-shell`**: the old code stays loaded until REAPER restarts.
   - The realistic host loop is bundle (1.9 s, lean debug) + `-renderproject` (1.1 s), about 3 s *(measured)*.
7. **Keep `target/` off ntfs-3g.** **[changed]**
   - Idle NTFS `stat` costs 67–73 µs against 2–3 µs on ext4. The first report's 4–7 ms was a contended worst case.
   - A target dir on NTFS **doubles** incremental builds (2.54 s vs 1.17 s); sources alone on NTFS cost almost nothing.
   - The repo has already moved to ext4.
8. **Hot reload: none in the plugin.** **[confirmed, now measured]** A DAW-loaded nice-plug plugin isn't even re-loaded from disk inside REAPER. Dioxus `subsecond` stays an optional nice-to-have for a standalone egui window.
9. **[new] UI and test loops are fast; rust-analyzer is the memory hog.**
   - insta: the leaf crate's test loop, insta snapshot included, takes 0.47 s after an edit.
   - egui_kittest image snapshots run headless on lavapipe (software Vulkan), with no GPU and no display: 0.20–0.23 s per snapshot test, 1.37 s for the app crate's test loop after an edit.
   - rust-analyzer peaks at 2.7 GB RSS on the enth tree and 3.6 GB on the eframe + wgpu tree.
10. **[new] Tooling status.**
    - sccache cuts a fresh worktree's clean build by 31%, but only with `--target-dir` plus `SCCACHE_BASEDIRS`: it hashes the `CARGO_TARGET_DIR` env var.
    - cargo-watch is archived; its own README recommends bacon or watchexec.

## Findings

### Q1 — Build-time levers (October 2026)

**Linker defaults.** **[confirmed]**
- *Documented.* Since Rust 1.90.0 (2025-09-18), `x86_64-unknown-linux-gnu` links with the bundled `rust-lld`. The Rust team measured about 7× faster linking on an incremental ripgrep rebuild (about 40% end-to-end) and about 20% on a clean debug build. The opt-out is `-C linker-features=-lld` ([Rust blog, 2025-09-01](https://blog.rust-lang.org/2025/09/01/rust-lld-on-1.90.0-stable), [1.90.0 release](https://blog.rust-lang.org/2025/09/18/Rust-1.90.0/)).
- *Measured.* The binaries' `.comment` section reads `Linker: LLD 22.1.2`. No system `ld.lld` is needed; `clang -fuse-ld=lld` fails here, which doesn't matter.
- *Documented.* Current releases: **mold 3.0.0** (2026-10-05) is a Rust rewrite with performance "on par with" 2.42.1, MIT ([release](https://github.com/rui314/mold/releases/tag/v3.0.0)). **wild 0.10.0** (2026-08-04) is MIT/Apache-2.0 and still describes itself as not production-ready ([release](https://github.com/wild-linker/wild/releases/tag/0.10.0)). Both publish x86_64 Linux tarballs, without checksum files.
- *Anecdotal.* Zed's RFC (2025-09) measured incremental rebuilds of 3.9 s with wild, 5.0 s with mold and 6.6 s with lld ([zed#37717](https://github.com/zed-industries/zed/pull/37717)).

**How the linkers were compared.** **[new]**
- A wrapper set as `CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER` (`scripts/ld-switch.sh`) picks the linker from an env var, so one target dir serves all four linkers. It also logs each link's duration.
- rustc still passes `-fuse-ld=lld -B<sysroot>/gcc-ld` to a custom linker. The wrapper strips these for GNU ld (BFD), mold and wild; mold and wild run via `clang --ld-path=`.
- The `.comment` sections confirmed which linker produced each binary.
- Each table is 5 interleaved rounds, medians.

| Workload / profile | rust-lld (default) | mold 3.0.0 | wild 0.10.0 | GNU ld 2.42 | Load |
| --- | --- | --- | --- | --- | --- |
| spike-shell, lean: incremental build | 0.63 s | 0.58 s | 0.56 s | 2.27 s | 4.4–5.7 |
| spike-shell, lean: link step | 0.21 s | 0.17 s | 0.10 s | 1.87 s | |
| spike-shell, full debug: incremental build | 0.84 s | 0.76 s | 0.67 s | 3.65 s | |
| spike-shell, full debug: link step | 0.34 s | 0.30 s | 0.22 s | 3.19 s | |
| spike-shell, full debug: peak RSS | 0.80 GB | 0.30 GB | 0.30 GB | 0.84 GB | |
| toy eframe + wgpu, lean: incremental build | 1.69 s | 1.32 s | 1.01 s | 6.92 s | 14.7–18.5 |
| toy, lean: link step | 0.82 s | 0.52 s | 0.28 s | 6.13 s | |
| toy, full debug: incremental build | 1.97 s | 1.61 s | 1.79 s | 11.17 s | 12.4–18.2 |
| toy, full debug: link step | 1.23 s | 0.88 s | 0.66 s | 10.38 s | |
| toy, full debug: peak RSS | 1.67 GB | 0.29 GB | 0.29 GB | 1.64 GB | |

- *Inference.* Today the plugin's link is small. wild's advantage grows with the binary and the amount of debuginfo, and it also removes rust-lld's memory peak on full-debug links. Revisit wild when the plugin's incremental link passes about 0.5 s.

**Debuginfo and profiles** *(measured)*:

| Toy, 12 jobs | default (`debug = true`) | lean | `split-debuginfo = "unpacked"` (full debug) |
| --- | --- | --- | --- |
| Clean build | 80.8 s, back to back with lean (94.5 s at load 15) | 68.3 s back to back (59.8 s at load 10.9) | 100.0 s (load 17.8) |
| Incremental build, 5 interleaved rounds, load 18–22 | 2.09 s | 1.49 s | 1.77 s |
| Peak RSS, incremental | 1.70 GB | 0.71 GB | 0.98 GB |
| `app` binary / target dir | 377 MB / 3.0 GB | 93 MB / 1.4 GB | 160 MB / 2.3 GB |

| spike-shell (`cargo build -p spike-shell`) | default | lean |
| --- | --- | --- |
| Clean build (load 10–11) | 36.16 s, peak 0.93 GB | 26.91 s, peak 0.55 GB |
| Incremental build, 5 interleaved rounds, load 7–9 | 0.88 s, peak 0.82 GB | 0.66 s, peak 0.36 GB |
| `libspike_shell.so` | 191 MB | 54 MB |

- Lean keeps file:line in panics and backtraces. Split debuginfo helps less than simply dropping dependency DWARF.

**Optimising dependencies.** Toy, lean debuginfo throughout *(measured)*:

| Toy | deps `opt-level` 0 (lean) | deps 1 | deps 2 | deps 2 + workspace 1 |
| --- | --- | --- | --- | --- |
| Clean build | 59.8–68.3 s | 171.3 s (load 3.6) | 247.1 s (load 6.0) | (same artefacts as deps 2) |
| Incremental, 5 interleaved rounds, load 10–11 | 1.12 s | 0.77 s | 0.72 s | 0.98 s |
| `app` binary | 93 MB | 41.7 MB | 38.9 MB | |

- **[changed]** The first report's 6× clean-build penalty came from a swapping machine; the quiet figure is about 3× for `opt-level = 2`.
- Optimised dependencies make incremental builds faster (smaller link inputs). That doesn't pay for 2–3× clean builds, which every fresh agent worktree, `cargo clean` and toolchain bump pays again.
- **Per-crate `opt-level = 3`** on the DSP crates cost nothing on the toy (incremental 1.38 s vs 1.49 s).
- On `spike-shell`, the first `opt-level = 3` build took 5.3 s; later one-constant edits took 0.70 s against 0.66 s at O0.

**Does audio code need optimising in dev?** *(measured)*
- In REAPER, rendering a 30 s 8-note chord through `spike-shell` took:
  - debug at O0: 0.65 s (45× realtime);
  - O3: 0.54 s (54×);
  - release: 0.55 s (54×).
- REAPER's own render overhead dominates for this tiny synth.
- *Inference.* The real `enth-engine` (modulation matrix, effects) will be much heavier, so give `enth-dsp` and `enth-engine` `opt-level = 3` now. Don't optimise `spike-shell` or the plugin shell crate.

**Toolchain versions, parallel front end and Cranelift** *(measured)*:

| Lean profile, toy | Clean build (load 6–12) | Incremental, 5 interleaved rounds (load 10–13.8) |
| --- | --- | --- |
| stable 1.95.0 | 59.8 s | 1.24 s |
| stable 1.99.1 (repo pin; current stable, 2026-10-13) | 56.2 s | 1.07 s |
| nightly 1.101.0 (2026-10-17) | 57.0 s | 1.07 s |
| nightly + `-Zthreads=8` | 53.8 s (−6%) | 1.17 s |
| nightly + Cranelift for all crates | 67.2 s (+18%) | 1.08 s |
| nightly + Cranelift for workspace crates only (`profile.dev.package."*".codegen-backend = "llvm"`) | 69.3 s | 1.01 s |

- **Parallel front end.** **[changed]** The first report's −16% was noise. It is still unstabilised; the 2026 project goal lists Cargo integration, a test suite and query-cycle consistency as outstanding, quotes no speedups and gives no date ([2026 goal](https://goals.rust-lang.org/2026/parallel-front-end.html)).
- **Cranelift.** **[new]** It is a nightly-only component (`rustc-codegen-cranelift-preview`). Its README calls Linux x86_64 fully supported, makes `panic=abort` the default, calls unwinding experimental, and says `std::arch` is only partly supported ([in-tree README, fetched 2026-10-19](https://rust.googlesource.com/rust/+show/HEAD/compiler/rustc_codegen_cranelift/Readme.md)).
  - *Inference.* Here the edited crates are tiny and the link dominates, so a faster codegen backend can't show. Cranelift would only pay off when a large workspace crate (a real engine or UI crate) is edited. Even then, it conflicts with stable-only (must-have g) and with SIMD DSP.
- **share-generics.** *Documented* in rustc source (`compiler/rustc_session/src/config.rs`, master, fetched 2026-10-19): it defaults to on at opt-levels 0, 1, s and z, and off at 2 and 3. Dev builds already get it; `-Zshare-generics` is nightly-only. Nothing to set.

**sccache.** **[new]**
- *Documented.* It can't cache crates that invoke the linker (bin, dylib, cdylib, proc-macro) or incrementally compiled crates (workspace members in dev) ([README](https://github.com/mozilla/sccache), v0.18.0 released 2026-09-14).
- *Documented in source.* It hashes **every `CARGO_*` env var** except `CARGO_MAKEFLAGS`, `CARGO_REGISTRIES_*`, `CARGO_BUILD_JOBS` and `CARGO_ENCODED_RUSTFLAGS` ([src/compiler/rust.rs](https://github.com/mozilla/sccache/blob/main/src/compiler/rust.rs)).
- *Measured.*
  - With `CARGO_TARGET_DIR` set as an env var, a second clean build in a new target dir got **0 hits**, even with `SCCACHE_BASEDIRS`.
  - With `--target-dir` plus `SCCACHE_BASEDIRS` naming both target dirs: cold 64.9 s, warm build in a *new* target dir **44.6 s (−31%)**, 221/270 hits.
  - No overhead on incremental builds (1.09 s vs 1.09 s). The cache was 272 MB.
- *Inference.* It is worth it only if agents routinely start fresh worktrees, and then only with that exact setup.

### Q2 — Hot reload

- **Dioxus subsecond.** *Documented.* crates.io lists 0.7.10 (record updated 2026-10-16); Dioxus is at v0.8.0-alpha.3 (2026-10-16) ([docs.rs](https://docs.rs/subsecond/latest/subsecond/), [Dioxus 0.7 post, 2025-09-08](https://dioxuslabs.com/blog/release-070/)).
  - You wrap entry points in `subsecond::call`, and `dx serve --hot-patch` (with the ThinLink linker wrapper, which ships only inside the `dx` CLI) rebuilds the code and sends a new jump table.
  - Limits: it patches only the "tip" (binary) crate, can't change struct layouts, doesn't observe static-initialiser changes, and resets thread-locals on each patch.
  - *Anecdotal.* An egui maintainer's example shows it working with eframe in a standalone window ([lucasmerlin/egui-subsecond-example](https://github.com/lucasmerlin/egui-subsecond-example), 2025-09-08, no licence file).
- **hot-lib-reloader-rs**: last release 0.8.2 on 2025-08-11 *(documented, GitHub API)*.
- **Inside a DAW.** **[confirmed, now measured]**
  - In REAPER, a rebuilt `spike-shell` binary was **not** re-loaded from disk even after its last instance was removed (details under Q3).
  - Patching a running plugin is therefore out of reach, and so is a dylib-swap scheme inside the plugin, since it would face the same module-lifetime problem *(inference)*.
- **Verdict.** No hot reload in Phase 0. The loop is a standalone egui window plus a 0.66 s incremental build of the plugin crate, or the about 3 s bundle-and-render host loop under Q3. subsecond in the standalone editor window is optional (a two-hour trial in Phase 1). eframe 0.37's `persistence` feature (egui/persistence + ron + serde) can restore window state across relaunches *(documented, crates.io feature list)*.

### Q3 — REAPER on Linux, host automation, headless hosts

All runs used a scratch copy of the owner's portable REAPER 7.82, under `xvfb-run`.

**Headless startup** **[new]** *(measured)*:
- A fresh portable install shows a modal "Portable installation: scan system VST/CLAP/LV2 paths? Yes/No" dialog on every launch until answered. It blocks headless runs.
- Seeding `reaper.ini` with `clap_path_linux-x86_64=…`, `vstpath=…` and `lv2path_linux=…` avoids it.
- No licence dialog blocked startup; REAPER writes a `[nag]` key and the title says "EVALUATION LICENSE".
- `linux_audio_mode=2` selects Dummy Audio. 3 is PulseAudio, which would join the real PipeWire graph. Without a working device, an "Error opening devices" dialog appears.
- A Lua script given on the command line (`reaper -nosplash -newinst script.lua`) runs at startup; a probe script exits in 0.8–1.6 s.
- REAPER also does an online version check (`[verchk]`).

**Command-line rendering** **[new]**:
- *Documented.* The binary's own usage text lists `-renderproject file.rpp` ("render project and exit"), `-nosplash`, `-newinst`/`-nonewinst`, `-cfgfile`, `-ignoreerrors`, `-close[all][:save|:nosave][:exit]`, `-batchconvert`, `-template` and `-saveas`.
- *Measured.* `reaper -nosplash -newinst -renderproject p.rpp` rendered a 2 s project in 1.45 s for the test CLAP and 1.12 s for `spike-shell`, startup and exit included. It always uses the plugin file currently installed.
- *Measured.* Through Lua, a 2 s render with action 42230 ("Render project, using the most recent render settings, auto-close render dialog") took 0.07–0.17 s.
- *Measured.* Other action IDs:
  - 40015: Render project to disk…
  - 41824: Render with the most recent settings
  - 43349: Dry-run render
  - 40004: Quit. It blocks on a "save changes?" query if the project is dirty.
- The ReaScript API has `TrackFX_AddByName`, `TrackFX_GetParam`/`SetParam` and `TrackFX_GetParamName` *(documented, [ReaScript API](https://www.reaper.fm/sdk/reascript/reascripthelp.html))*.

**Rescan and reload** **[changed]**. The first report relied on forum posts; these are measurements:
- REAPER's CLAP cache (`reaper-clap-linux-x86_64.ini`) stores a timestamp per file. At startup it re-scans changed files; a renamed plugin showed up under its new name on the next launch.
- No action in the main section has "scan" in its name, so there is no scriptable rescan. The GUI path (Preferences → Plug-ins → CLAP → Re-scan) wasn't exercised.
- **In-process reload, trivial plugin**: a minimal clap-sys synth (about 150 lines, no threads or TLS) was swapped by atomic rename after deleting its only instance. Re-adding it loaded the new code: the parameter was renamed and the tone went from 440 to 880 Hz, with no restart.
- **In-process reload, real plugin: fails.**
  - Same procedure with `spike-shell`, swapping to a build whose `VOICE_SCALE = 0` (silent): the re-added instance still produced sound, i.e. the old code.
  - A fresh process (`-renderproject`) with the same installed file rendered silence.
  - *Inference.* Rust std, nice-plug or egui register thread-local destructors (the binary imports `__cxa_thread_atexit_impl`) or keep threads alive, so glibc keeps the module mapped after `dlclose`.
  - Consequence: after rebuilding the plugin, restart REAPER or render in a new process.
- **Side observation** *(measured, needs follow-up)*:
  - A second `spike-shell` instance created in the same REAPER process rendered with a different RMS from the first (0.086–0.096 vs 0.123).
  - First-instance renders were bit-identical across processes.
  - This hints at state carried across instances, a determinism risk worth a Task 3/5 test.

**Headless CLAP renderers** **[changed]**: the first report found none.

| Tool | Licence, status | Renders a synth with notes to WAV? |
| --- | --- | --- |
| [dfl/clap-trap](https://github.com/dfl/clap-trap) | MIT, C++, v0.1.0 2026-01-12 | No in v0.1.0. *Measured* on the test plugin: `validate`, `info`, `bench` (2,222× realtime) and `process` work headless, but `process` feeds silence and the release has no `notes` command (the README describes a newer one). The binary links webkit2gtk. |
| [blablack/patch-render](https://github.com/blablack/patch-render) | AGPL-3.0, pybind11, pushed 2026-10-13 | Yes: `render_clap()` with a MIDI list. Usable as an external tool only; never vendor it. |
| [sourcebox/clap-test-host](https://github.com/sourcebox/clap-test-host) | Apache-2.0, Rust (fork of clack's cpal example) | No: realtime audio, GUI and MIDI input |
| [LX-Audiolabs/clap-host-rs](https://github.com/LX-Audiolabs/clap-host-rs) | MIT, Rust, pre-alpha, 0 stars | No: realtime, CLI and Slint GUI |
| Carla 2.5.8 (Ubuntu package) | GPL | No CLAP hosting: no "clap" strings in `libcarla_standalone2.so` *(measured)* |
| REAPER `-renderproject` | proprietary host | **Yes**, in 1.1–1.5 s headless *(measured)* |

- clack-host 0.2.0 (2026-09-12) remains the base for our own host *(documented)*.

### Q4 — Feedback loops for coding agents

**Inner-loop timings on the toy** (default profile, warm, medians of 5, load 14–20) *(measured)*:
- `cargo check --workspace`: 0.48 s
- `cargo check --all-targets`: 0.42 s
- `cargo clippy --workspace --all-targets`: 1.25 s
- `cargo test -p leaf --no-run`: 0.32 s
- `cargo nextest run -p leaf`: 0.60 s
- `cargo test --workspace --no-run`: 1.02 s
- `cargo nextest run --workspace`: 1.52 s
- `cargo build --workspace`: 1.87 s (relinks the eframe binary)
- Clean `cargo check`: 57.8 s; clean `cargo build`: 94.5 s at load 15 (80.8 s at load 9).

**On the enth repo** (spike-shell, lean, warm) **[new]** *(measured)*:
- `check -p spike-shell`: 0.23 s
- `clippy -p spike-shell --all-targets`: 0.32 s
- `cargo test -p spike-shell`: 0.82 s (build + run its unit tests)
- `nextest run -p spike-shell`: 0.95 s
- The full Done-means gate is about 3.1 s (Answer in brief, item 5). Clippy pedantic caught my first salt edit (`unnecessary_cast`), so the gate works as intended.
- `cargo xtask bundle spike-shell` after a DSP edit:
  - debug, default profile: 2.54 s (191 MB `.clap`);
  - debug, lean: 1.93 s (54 MB);
  - release: 3.90 s (9.1 MB; clean release 65.4 s).
  - The bundler adds about 1.3 s over a plain `cargo build -p spike-shell`.

**insta and egui_kittest** **[new]** *(measured; toy plus `insta = "1.49"` and `egui_kittest = { version = "0.37", features = ["wgpu", "snapshot"] }`, lean profile)*:
- Clean test build: 158 s (3 jobs, shared lock, load 11–16).
- After a leaf edit:
  - `nextest run --workspace`: 1.38 s (7 tests, including one insta and one kittest image snapshot);
  - `nextest run -p leaf` (with insta): 0.47 s;
  - `nextest run -p app` (kittest snapshot + query): 1.37 s.
- A rerun with no change: 0.62 s. `cargo insta test --accept` with nothing to review: 0.50 s.
- Per test: kittest snapshot 0.20–0.23 s (lavapipe device included); kittest query without rendering 0.037 s; insta 0.06 s.
- **Headless setup**: `VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json` (plus `VK_DRIVER_FILES`), `WGPU_BACKEND=vulkan`, no `DISPLAY`. This avoids the NVIDIA stack entirely and matches CI.
- A change in the 4th decimal of a label ("rms = 0.5412" → "0.5414") failed the image snapshot with 41–42 differing pixels at the default threshold, so the snapshots catch text changes.
- *Documented* ([egui_kittest 0.37.0 README](https://github.com/emilk/egui/blob/0.37.0/crates/egui_kittest/README.md)):
  - snapshots are updated with `UPDATE_SNAPSHOTS=true`;
  - a `kittest.toml` holds the thresholds;
  - an accessibility check panics when an input widget has no accessible name;
  - prefer insta or plain tests over image tests where possible (slower, brittle across renderers).
- **[new]** Also measured: egui/eframe **0.37.0** (2026-10-15) replaced `App::update(ctx, frame)` with `App::ui(ui, frame)` plus an optional `logic(ctx, frame)`. My scratch app, written to the old API, failed to compile.

**rust-analyzer** **[new]** (*measured*, standalone 2026-10-12 build, `analysis-stats --parallel`):

| Tree | Lines analysed | RA-reported total | Wall | Peak RSS |
| --- | --- | --- | --- | --- |
| enth repo (spike-shell) | 1,735 workspace + 4.25 M dependency | 20.4 s / 2.29 GB | 36.3 s (load 3.5) | **2.71 GB** |
| toy (eframe + wgpu) | 4.82 M including dependencies | 58.5 s / 3.03 GB | 120.9 s (load 8–17) | **3.57 GB** |

- *Documented.* Claude Code has an official `rust-analyzer-lsp` code-intelligence plugin. With it, Claude gets diagnostics after every edit plus an LSP navigation tool; the docs warn that memory rises while the server indexes ([code intelligence docs](https://code.claude.com/docs/en/plugins/code-intelligence)).
- *Inference.* On this 15 GB laptop, one rust-analyzer per concurrent agent or worktree is affordable for one or two agents, not five.

**What Claude Code's docs advise** *(documented, [best practices](https://code.claude.com/docs/en/best-practices), undated, fetched 2026-10-19)*:
- give the agent a check it can run (tests, a build, a screenshot compared against a target);
- make checks hard gates with a Stop hook;
- treat the context window as the scarce resource (compact outputs);
- use worktrees for parallel sessions.

All of these match enth's CLI-first, JSON, golden-test design.

**Reports from people using Claude Code or Codex on audio** **[new]**, *anecdotal*:
- **faderframe** (KVR thread from 2026-10-10; [repo](https://github.com/BurningTreeC/faderframe), MIT):
  - A Rust DAW, Linux-first, built with Claude Code, with Codex as the fallback when session limits hit. About 50 crates and 100+ commits in 11 days.
  - The author reports fast scaffolding and feature layering, but "a lot of human work": design decisions, overruling the AI, testing by recording.
  - Its dev profile uses `opt-level = 1` for workspace crates, `opt-level = 2` for dependencies and `opt-level = 3` for the DSP crates. That is the opposite trade to the one measured here; it suits a DAW whose DSP lives in many workspace crates.
- **KVR "vibecoding plugins" (2025-08/09)**: JUCE projects. AI helped with loading errors, clicks and pops, and cross-platform issues; nobody answered how regressions were checked. Experienced developers found debugging generated code demoralising.
- **The Audio Programmer (2026-02-26)**: a JUCE gain plugin built with Claude Code. Install troubleshooting and custom look-and-feel work were the hard parts.
- **A Claude Code skill for nih-plug** ([Hornfisk/plugindev-plugin](https://github.com/Hornfisk/plugindev-plugin), MIT, 2026-04/05) encodes the gotchas agents hit:
  - keep `assert_process_allocs` on;
  - a "persist race" in which a timer overwrites DAW-restored state;
  - standalone backends report the transport as always playing.
  - It also recommends `try_lock`, which conflicts with enth's no-locks rule.
- **HN (2026-01-03)**: an AI-built Cmajor hardware emulation (160 points). A former emulation developer described black-box comparison of test signals against a reference, the same idea as enth's render + descriptors.
- *Inference.* What worked across these reports was concrete, testable steps and host-level smoke tests. What failed was UI polish, regressions without tests, and host-integration gotchas. That supports enth's plan: golden renders, descriptor checks, `assert_no_alloc`, and a scripted host check (REAPER `-renderproject` / `xtask reaper-test`).

### Q5 — Test runner and watchers

- cargo-nextest 0.9.148 (crates.io record 2026-10-08; owner has 0.9.133) *(documented)*.
  - *Measured* on tiny crates: about 0.3 s more per process than `cargo test -p` (0.60 vs 0.32 s on the toy; 0.95 vs 0.82 s on spike-shell).
  - *Inference.* Use plain `cargo test -p <crate>` for one-crate inner loops, and nextest for the workspace suite.
- **cargo-watch.** **[changed]** The repo is **archived** (last push 2025-01-14); its README says it is "on life support" and recommends bacon or watchexec. Last crate release 8.5.3 (2024-10-02) *(documented)*.
- **bacon 3.26.0** (2026-09-26). *Documented* (its `--help` and config docs):
  - `--headless`;
  - `-e/--export-locations`, which writes `.bacon-locations` with kind, path, line, column and message;
  - a `--listen`/`--send` Unix socket.
  - *Inference.* A human can keep bacon running while agents read `.bacon-locations`, but agents gain little over calling `cargo` directly.

### Q6 — Local measurements: done

Every number above is *measured* unless marked otherwise, with conditions given.

**Not done**: `cargo-mutants` and fuzz loops (out of scope).

### The repo's file system

**[changed]** The first report's 4–7 ms `stat` was taken while ntfs-3g was at 87% CPU.

*Measured* while ntfs-3g was idle (USB SATA SSD, mounted as `fuseblk`):

| | NTFS (ntfs-3g) | ext4 (NVMe) |
| --- | --- | --- |
| `stat()` | 67–73 µs | 1.8–3.4 µs |
| Clean build, lean, sources on NTFS + target on ext4 | 58.4 s (load 7.8) | |
| Clean build, lean, sources + target on NTFS | 68.8 s (load 12.7) | |
| Incremental build (5 interleaved rounds) | 1.18 s with target on ext4; **2.54 s** with target on NTFS | 1.17 s |

The bench directory was deleted afterwards. The repo now lives on ext4. Keep any `target/` there too, including for worktrees.

## Recommendation for Phase 0

**1. Exact `[profile.*]` block for the repo's root `Cargo.toml`.** It has none today. *Measured effects on spike-shell: clean 36→27 s, incremental 0.88→0.66 s, `.clap` 191→54 MB, incremental peak RSS 0.82→0.36 GB.*

```toml
[profile.dev]
debug = "line-tables-only"        # file:line in panics and backtraces; full DWARF only where you ask for it

[profile.dev.package."*"]
debug = false                     # no DWARF for nice-plug, egui, baseview and other dependencies

# Real-time crates run optimised even in debug builds (debug assertions and overflow checks stay on).
# Add each L0-L2 crate that runs per-sample code. Do not optimise all dependencies:
# deps at opt-level 2 made clean builds 2.95x slower for about 0.4 s off each incremental build.
[profile.dev.package.enth-dsp]
opt-level = 3

[profile.dev.package.enth-engine]
opt-level = 3

# Profiling and heaptrack runs keep symbols without debug-build slowness: cargo build --profile profiling
[profile.profiling]
inherits = "release"
debug = "line-tables-only"
```

- `[profile.test]` inherits `dev`, so tests get the same settings.
- Leave `[profile.release]` at its defaults until the shipping spike (Task 7). Release bundle incremental builds take 3.9 s.
- Don't add `spike-shell` or the plugin shell to the `opt-level = 3` list: unoptimised it renders at 45× realtime.
- To debug a crate with a debugger, use `--config 'profile.dev.package.<crate>.debug=true'`.

**2. `.cargo/config.toml`.** Keep only the `xtask` alias.
- No linker entry: rust-lld is the default, and wild would save about 0.1 s per incremental build on the current plugin.
- Revisit wild when the plugin's incremental link passes about 0.5 s; configure it only under `[target.x86_64-unknown-linux-gnu]`, so wasm builds are unaffected.
- Don't set `build.jobs` in the repo. Agents running in parallel on the laptop should export `CARGO_BUILD_JOBS=4` each.

**3. Task 1 installs.**
- **Needed**: nothing more for builds. Native REAPER 7.82 is already installed.
- **Optional**: bacon, for the human; the Claude Code `rust-analyzer-lsp` plugin, with a single instance and its 2.7 GB in mind; sccache, only if agents start many worktrees, and then with `--target-dir` and `SCCACHE_BASEDIRS` set to the worktree roots.
- **Don't install**: mold, wild, or a system lld.

**4. Dev harness.**
- **Headless REAPER is the host-level check today.** Use `cargo xtask bundle spike-shell` (lean debug, 1.9 s), then an atomic install, then `xvfb-run reaper -nosplash -newinst -renderproject test.rpp` (1.1–1.5 s). That is about 3 s from edit to a WAV rendered by the real host.
  - Seed `reaper.ini` with the three plugin-path keys and `linux_audio_mode=2`.
  - The repo's `xtask reaper-test` already does most of this.
  - **Restart REAPER after rebuilding the plugin**: in-process reload doesn't work for nice-plug/egui.
  - `cargo xtask install` should keep replacing files rather than overwriting in place: it deletes, then copies, so the old inode survives while it is mapped.
- **`tools/test-host`** (clack-host) still has jobs REAPER can't do in CI: editor open/close timing and PSS, and rendering without REAPER. Give it a `render` subcommand (notes in, WAV out) so CI can run plugin-level goldens. No permissive Rust tool does this today, and clap-trap's release can't send notes.
- **Editor**: a standalone egui window. Add egui_kittest snapshot tests on lavapipe, which work headless and in CI.

**5. Agent loop for AGENTS.md** (timings from this machine):
- `cargo check` / `cargo clippy -p <crate>` and `cargo test -p <crate>`: 0.2–0.8 s.
- `cargo nextest run --workspace` before reporting.
- The full Done-means gate: about 3 s.
- Image snapshots: `VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json WGPU_BACKEND=vulkan`, never on the real display.
- Host check: bundle + `-renderproject`, about 3 s; restart REAPER after every rebuild.

## Plan check — what the evidence says should change

- **Root `Cargo.toml` (phase-0 Task 2 listing): change.** Add the profile block above. It is measured, cheap, and its memory saving matters on this laptop.
- **AGENTS.md "Fast feedback loops": small change.**
  - Its "rebuild of about 1.5 s" came from the toy. The real plugin crate's lean incremental build is 0.66 s, and the full gate is about 3 s.
  - Add "restart REAPER after rebuilding the plugin (no in-process reload)".
  - Add the headless-REAPER recipe: `xvfb-run`, seeded ini, Dummy audio, `-renderproject`.
  - Keep "hot reload is out".
- **phase-0 Task 1: change.** Drop any fast-linker install. Note that headless REAPER needs the three ini keys plus `linux_audio_mode=2`; the first-run portable dialog otherwise blocks it.
- **phase-0 Task 4 (`tools/test-host`): change.** Add a `render` subcommand for CI. Local host-level rendering is already covered by REAPER `-renderproject` in about 1.1 s.
- **Tasks 3/5: change.** Add a test that two plugin instances in one host process render identically. The second instance in REAPER differed from the first.
- **Toolchain pin 1.99.1: keep.** Stay off nightly: neither the parallel front end nor Cranelift helped here.
- **Nothing measured here argues against Rust, egui or nice-plug.**

## Gaps

- **Cranelift and the parallel front end on a heavy edited crate.** The toy's and spike-shell's edited crates are small, so codegen speed can't show. To settle it, re-run batch 5 (`scripts/batch5a.sh`, `batch5b.sh`) once `enth-engine` has real code, salting a constant in `enth-engine`.
- **wild on a bigger plugin.** Re-run `scripts/batchE3.sh incr` when the plugin's incremental link passes about 0.5 s.
- **Why REAPER keeps the old module.** The TLS-destructor explanation is an inference. To check it, run `LD_DEBUG=files` on REAPER during a delete and re-add, and look for `calling fini` for `spike-shell.clap`.
- **GUI rescan in REAPER** (Preferences → Re-scan) was not exercised. A scriptable path doesn't exist in the main action list.
- **The second-instance RMS difference** needs a minimal reproduction: render two instances in one process with `scripts/rtx.sh`-style Lua, then diff the WAVs.
- **egui_kittest on the NVIDIA GPU** was deliberately not tested (crash risk after the Xorg crash); only lavapipe was used.
- **perf is blocked** (`perf_event_paranoid=4`), so there is no compile-phase breakdown; link times come from the wrapper's own timing.
- **subsecond hands-on** would need the `dx` CLI (`cargo install dioxus-cli`); not tried.
- **Reports of agent use on Rust audio are thin.** One Rust DAW and a few JUCE threads. No systematic study found.
- **Clean-build numbers vary about ±15%** because of load outside the lock. Incremental numbers are interleaved medians and more reliable.

## What changed versus the first report

| Topic | First report | Final |
| --- | --- | --- |
| Clean builds | 92–166 s at 4 jobs on a swapping machine | 57–95 s at 12 jobs, quiet lock; spike-shell 27–36 s |
| Linkers | mold and wild "Task 1 candidates", unmeasured | Measured on the real plugin and the toy; **don't install now** |
| Dependency opt-level | "+6× clean build" | **2.95×** (O2) and 2.05× (O1) |
| Parallel front end | −16% clean (noisy) | −6% clean, no incremental gain |
| Cranelift | Not measured | +18% clean, no incremental gain; stay on stable |
| Rust version | 1.95 | 1.99.1 is the pin and current stable; similar speed, incremental slightly faster |
| NTFS | `stat` 4–7 ms; move the repo | 67–73 µs when idle; a target dir on NTFS doubles incremental builds; repo already moved |
| REAPER | Needs a native install; rescan per forum posts | Headless `-renderproject` works in about 1.1 s; ini seeding + Dummy audio; reload works only for trivial plugins, so **restart REAPER for nice-plug** |
| Headless CLAP render | None found | clap-trap (no notes in release), patch-render (AGPL), REAPER `-renderproject`; still build `test-host render` for CI |
| Hot reload | In-plugin not viable (inference) | Confirmed by measurement (module not reloaded) |
| insta, kittest, rust-analyzer | Not measured | Measured: 0.47 s / 1.37 s test loops, kittest on lavapipe, RA 2.7–3.6 GB |
| Agent reports | Not researched | Summarised (faderframe, KVR, Audio Programmer, nih-plug skill) |
| cargo-watch | No release for two years | Archived; README recommends bacon or watchexec |
| sccache | Not measured | −31% for a fresh worktree, only with `--target-dir` + `SCCACHE_BASEDIRS` |
| Profile advice | Lean debuginfo + DSP crates at O3 | Same, confirmed on the real repo; exact block given; spike-shell needs no O3 |

## Sources

All accessed 2026-10-19. The registry dates are crates.io records or GitHub release dates.

- Rust: [rust-lld default (2025-09-01)](https://blog.rust-lang.org/2025/09/01/rust-lld-on-1.90.0-stable), [1.90.0 (2025-09-18)](https://blog.rust-lang.org/2025/09/18/Rust-1.90.0/); stable 1.99.1 per `static.rust-lang.org/dist/channel-rust-stable.toml` (2026-10-15); [parallel front-end goal 2026](https://goals.rust-lang.org/2026/parallel-front-end.html); [Cranelift README](https://rust.googlesource.com/rust/+show/HEAD/compiler/rustc_codegen_cranelift/Readme.md); [rustc config.rs (share_generics)](https://github.com/rust-lang/rust/blob/master/compiler/rustc_session/src/config.rs).
- Linkers: [mold v3.0.0](https://github.com/rui314/mold/releases/tag/v3.0.0), [wild 0.10.0](https://github.com/wild-linker/wild/releases/tag/0.10.0), [Zed RFC #37717](https://github.com/zed-industries/zed/pull/37717).
- Caching and watchers: [sccache README](https://github.com/mozilla/sccache), [sccache Rust hashing](https://github.com/mozilla/sccache/blob/main/src/compiler/rust.rs), [cargo-watch (archived)](https://github.com/watchexec/cargo-watch), [bacon config docs](https://dystroy.org/bacon/config/), bacon v3.26.0 release (2026-09-26).
- Hot reload: [subsecond docs](https://docs.rs/subsecond/latest/subsecond/), [Dioxus 0.7](https://dioxuslabs.com/blog/release-070/), [egui-subsecond-example](https://github.com/lucasmerlin/egui-subsecond-example), [hot-lib-reloader-rs](https://github.com/rksm/hot-lib-reloader-rs).
- UI and test: [egui_kittest 0.37.0 README](https://github.com/emilk/egui/blob/0.37.0/crates/egui_kittest/README.md); eframe 0.37.0, egui_kittest 0.37.0 (2026-10-15), insta 1.49.0 (2026-10-03), cargo-nextest 0.9.148 (2026-10-08) on crates.io; rust-analyzer release 2026-10-12.
- Hosts: [ReaScript API](https://www.reaper.fm/sdk/reascript/reascripthelp.html); the REAPER 7.82 binary's usage text (local); [clap-trap](https://github.com/dfl/clap-trap), [patch-render](https://github.com/blablack/patch-render), [clap-test-host](https://github.com/sourcebox/clap-test-host), [clap-host-rs](https://github.com/LX-Audiolabs/clap-host-rs); clack-host 0.2.0 (2026-09-12).
- Agents: [Claude Code best practices](https://code.claude.com/docs/en/best-practices), [code intelligence plugins](https://code.claude.com/docs/en/plugins/code-intelligence); [KVR: DAW with AI](https://www.kvraudio.com/forum/viewtopic.php?p=9310970), [KVR: vibecoding plugins](https://www.kvraudio.com/forum/viewtopic.php?t=622935), [faderframe](https://github.com/BurningTreeC/faderframe), [The Audio Programmer (2026-02-26)](https://www.theaudioprogrammer.com/content/building-an-audio-plugin-with-claude-code-a-vibe-coding-experiment), [HN 46471648](https://news.ycombinator.com/item?id=46471648), [Hornfisk/plugindev-plugin](https://github.com/Hornfisk/plugindev-plugin).
