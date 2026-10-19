# 06 — Dev-loop speed for AI + human iteration

- Date: 2026-10-19
- Sub-researcher: 06 (dev-loop speed)
- Questions: 1 build-time levers (answered, with local measurements); 2 hot reload (answered from docs); 3 iterating in REAPER and headless hosts (partly answered); 4 agent feedback loops (local timings answered; reports from other Claude Code / Codex audio projects **not researched**); 5 test runner and watchers (partly answered); 6 local measurements (done for the default linker, GNU ld, profile tweaks and the nightly parallel front end; not done for mold, wild, Cranelift or sccache, none of which are installed).
- **Cut short.** The coordinator asked me to wrap up before a reboot. Everything unfinished is listed under Gaps, with the commands to re-run it.
- Marks: *measured* (run on this machine today), *documented* (official docs, release notes, registries), *anecdotal* (forums, blogs, third-party projects), *inference* (my reasoning, not checked).

## Answer in brief

- **lld is already the default linker.** rustc has used its bundled `rust-lld` on x86_64 Linux since 1.90.0 (2025-09-18) *(documented)*. The owner's rustc 1.95.0 binaries carry `Linker: LLD 22.1.2` *(measured)*. "lld missing" in the brief only means there is no system `ld.lld`, and none is needed. `clang -fuse-ld=lld` fails here for that reason *(measured)*. Opting back into GNU ld made incremental builds of an eframe app about **5× slower** (12.2 s vs 2.3 s, measured).
- **The cheapest win is a lean-debuginfo dev profile.** It means `debug = "line-tables-only"` for workspace crates and `debug = false` for dependencies. It shrank the eframe dev binary from 377 MB to 93 MB, cut incremental builds from 2.3 s to 1.5 s and clean builds from 157 s to 126 s, and cut peak build memory from 1.7 GB to 0.7 GB *(measured)*. The memory cut matters on a laptop with about 2.5 GiB free.
- **Don't optimise all dependencies in dev.** With `opt-level = 2` on all dependencies (egui, wgpu, winit…), the clean build took **743 s instead of 126 s** *(measured under heavy load; re-check)*, and incremental builds were only about 0.4 s faster. enth's hot audio code lives in workspace crates (`dsp`, `engine`), so optimise those crates by name. That cost almost nothing on the toy crates *(measured)*.
- **Nightly levers don't pay yet.** The parallel front end (`-Zthreads=4`, nightly) gave −16% on a clean build (noisy), no change on incremental builds, and +21% peak memory *(measured)*. It is still unstabilised *(documented)*. Cranelift is nightly-only, with `std::arch` only partly supported *(documented)*, and it isn't installed. The project should stay on stable, which is also must-have (g).
- **Inner loops on this machine take about 0.25–1.5 s once warm** *(measured on a toy workspace)*: `cargo test -p <leaf> --no-run` 0.24 s, `nextest run -p <leaf>` 0.6 s, `clippy --all-targets` 0.55 s, `check` 0.5–1.2 s, a workspace build that relinks the egui binary 1.5 s (lean profile). The loop is already fast enough that wild or mold can only shave part of the ~1.5 s. Measure them in Task 1 before adopting either.
- **The repo's location is likely the biggest slowdown on this machine.** `enth/` sits on a USB SATA SSD formatted **NTFS and mounted through ntfs-3g (FUSE)**. A `stat()` there took **4–7 ms, against 3–4 µs on the internal ext4 NVMe** (`/home`). That was measured while ntfs-3g was busy at 87% CPU, so treat it as an upper bound. Cargo, rust-analyzer and file watchers all stat source files constantly, and a `target/` on that mount would be far worse. **Move the working repo to `/home` (ext4, 358 GB free) in Task 1.**
- **Hot reload isn't worth adopting in Phase 0.** Dioxus `subsecond` works with egui in a standalone window run by `dx serve --hot-patch`. It patches only the "tip" (binary) crate and needs the `dx` CLI to launch the process *(documented)*, so it can't help inside a DAW-loaded `.clap` *(inference)*. `hot-lib-reloader` has had no release since 2025-08-11 *(documented)*. A standalone egui dev window plus a ~1.5 s rebuild is the loop to build.
- **REAPER iteration needs a native Linux REAPER, which the owner doesn't have.** The owner's REAPER runs under Wine, and a Windows REAPER can't load a Linux `.clap` (ELF) *(inference)*. Forum reports say a new or rebuilt CLAP needs a rescan from Preferences, or a REAPER restart *(anecdotal)*. I found no documented hot-reload of a loaded plugin. So rebuild → restart or rescan → reload the project is the realistic loop. Task 1 needs a native Linux REAPER install.
- **Headless CLAP rendering has to be built.** I found no ready-made clack-host tool that loads a `.clap`, sends notes and writes a WAV (one search, not exhaustive). Phase 0's `tools/test-host` should get a `render` subcommand. That lets agents test the built plugin (and run plugin-level goldens) without a DAW.
- **egui 0.37.0 (crates.io, 2026-10-15) changed the eframe app API**: `App::update(ctx, frame)` became `App::ui(ui, frame)` plus an optional `logic(ctx, frame)` *(measured: my scratch app written to the old API failed to compile)*. Agents writing egui code from training data will hit this. Keep pointing them to current docs.rs and the pinned version, as AGENTS.md already says.

## Findings

### Local measurement setup (applies to every *measured* number below)

- Scratch workspace `/tmp/enth-research/06/ws` (ext4 on the internal NVMe), resolver 3, edition 2024. It has three crates:
  - `leaf`: a lib standing in for `enth-dsp` (saw, one-pole, two unit tests).
  - `mid`: a lib standing in for `enth-engine`.
  - `app`: a binary on `eframe = "0.37"` with default features (resolved to eframe 0.37.0 with the **wgpu** renderer, wgpu 30.0.1, winit 0.30.13; 425 packages locked, 364 unique crates in `app`'s tree).
- Each "incremental" step rewrites `crates/leaf/src/salt.rs` with a new constant that `leaf::render` uses. That is a real code change, so `leaf`, `mid` and `app` all recompile and the binary relinks.
- Timing used `/usr/bin/time -f "%e %M"` through the script `/tmp/enth-research/06/scripts/m.sh`. `%M` is the peak RSS of the largest child process (rustc or the linker). Raw log: `/tmp/enth-research/06/results/times.tsv`.
- Every build used `CARGO_BUILD_JOBS=4`. Toolchain: stable rustc 1.95.0 (2026-04-14, LLVM 22.1.2); nightly runs used `nightly-2026-04-15` (rustc 1.97.0-nightly a5c825cd8).
- **Machine conditions were noisy.** At the start: 15 GiB of swap in use, 1.5–1.6 GiB RAM available, load average 12–16 on 12 threads, and six other researchers building concurrently. Over the session, available RAM ranged from 1.5 to 11 GiB and the load from 5.9 to 30. Each row below gives the load it ran at.
- I report medians of three runs. The cleanest comparison is the interleaved round J (load 7.7–9.1), where configurations alternated so they shared the same conditions.

Configurations:

| Id | What |
| --- | --- |
| A | Defaults: rust-lld (implicit), `debug = true` (full), nothing optimised |
| B | A + `RUSTFLAGS="-C linker-features=-lld"`, i.e. GNU ld 2.42 (BFD) |
| C1 | Lean debuginfo: `profile.dev.debug = "line-tables-only"`, `profile.dev.package."*".debug = false` |
| C4 | C1 + `profile.dev.package.{leaf,mid}.opt-level = 3`, i.e. "optimise the DSP crates by name" |
| C2 | C1 + `profile.dev.package."*".opt-level = 2`, i.e. "optimise all dependencies" |
| C3 | C2 + `profile.dev.opt-level = 1` (workspace crates at O1 too) |
| D0 / D1 | C1 on nightly, without / with `RUSTFLAGS="-Zthreads=4"` (parallel front end) |

Profiles were passed with `--config 'profile.dev....'`, so the scratch `Cargo.toml` never changed. Example: `cargo build --workspace --config 'profile.dev.debug="line-tables-only"' --config 'profile.dev.package."*".debug=false'`.

### Q1 — Build-time levers (October 2026)

**Linkers.**

- *Documented.* Since Rust 1.90.0 (2025-09-18), `x86_64-unknown-linux-gnu` links with the bundled `rust-lld` by default. The Rust team's ripgrep benchmark showed links about 7× faster on an incremental debug rebuild (about 40% end-to-end) and about 20% faster from scratch. The opt-out is `-C linker-features=-lld`. Sources: [Rust blog, 2025-09-01](https://blog.rust-lang.org/2025/09/01/rust-lld-on-1.90.0-stable), [Rust 1.90.0 release, 2025-09-18](https://blog.rust-lang.org/2025/09/18/Rust-1.90.0/).
- *Measured.* `readelf -p .comment` on config A's binary shows `Linker: LLD 22.1.2`. `rust-lld` and `gcc-ld/ld.lld` are present in the 1.95.0 sysroot. There is no system `ld.lld`, so `clang -fuse-ld=lld` fails with "invalid linker name". That doesn't matter, because rustc doesn't need a system lld.
- *Measured.* Default rust-lld against GNU ld:

  | Step | A (rust-lld) | B (GNU ld) | Conditions |
  | --- | --- | --- | --- |
  | Clean `cargo build --workspace`, empty target | 157.1 s | 166.3 s | A at load 5.9; B at load 14.9 |
  | Incremental `cargo build`, sequential | 2.26 s (6.85 / 2.26 / 2.12) | 13.91 s (15.27 / 13.04 / 13.91) | load 13.5–17.8 |
  | Incremental `cargo build`, interleaved round J | **2.27 s** (2.91 / 2.27 / 2.09) | **12.16 s** (14.47 / 12.16 / 12.09) | load 7.7–9.1 |
  | Incremental `cargo test --workspace --no-run` | 0.95 s | 1.45 s | load 12–17 |

- *Documented and anecdotal.* There are two faster linkers to consider:
  - **mold 3.0.0** (released 2026-10-05) is mold rewritten in Rust. Its release notes say it is a drop-in replacement whose "linking performance remains on par with 2.42.1"; licence MIT ([release](https://github.com/rui314/mold/releases/tag/v3.0.0)).
  - **wild 0.10.0** (released 2026-08-04; Apache-2.0 per the GitHub API; repo now `wild-linker/wild`). Its release notes say "Not much changed in terms of performance", and its README still says it isn't production-ready ([release](https://github.com/wild-linker/wild/releases/tag/0.10.0)). Its 0.10.0 benchmark page publishes charts only, no numeric tables ([benchmarks](https://github.com/wild-linker/wild/blob/0.10.0/benchmarks/ryzen-9955hx.md)).

  The best real-Rust comparison I found is Zed's RFC (opened 2025-09-07, merged 2025-09-25) *(anecdotal; one machine)*. Zed's incremental rebuilds took 3.9 s with wild 0.6, 5.0 s with mold 2.37.1 and 6.6 s with lld; clean builds took 176, 184 and 182 s. Zed now recommends wild for local builds and keeps mold in CI ([zed#37717](https://github.com/zed-industries/zed/pull/37717)).
  - *Inference.* Our measured incremental build is about 1.5 s in total with the lean profile, so a faster linker can only shave the link share of it. I didn't isolate the link time (see Gaps).
  - *Status.* mold and wild aren't installed; they are the Task 1 candidates.

**Debuginfo and profiles.** *Measured*, interleaved round J (load 7.7–9.1, 10.4–11.2 GB available) unless noted:

| Config | Incremental build (median of 3) | Peak RSS, incremental | `app` binary | Target dir | Clean build (empty target) |
| --- | --- | --- | --- | --- | --- |
| A default | 2.27 s | 1.70 GB | 377 MB | 2.9 GB | 157.1 s (load 5.9) |
| C1 lean debuginfo | 1.53 s | 0.71 GB | 93 MB | 1.3 GB | 126.0 s (load 15.3) |
| C4 C1 + DSP crates at O3 | 1.41 s | 0.71 GB | — | shared with C1 | — (only the two small crates rebuild) |
| C2 C1 + all deps at O2 | 1.09 s | 0.35 GB | 39 MB | 1.2 GB | **742.9 s** (load 14–28; see Gaps) |
| C3 C2 + workspace at O1 | 0.91 s | 0.35 GB | — | shared with C2 | — |

- Under heavy load (round I: load 19–30), the gaps widened: A took 10.1 s, B 21.7 s, C1 3.6 s, C4 2.2 s, C2 2.0 s and C3 1.3 s. *Inference:* the 2.9 GB full-debuginfo target and its 1.7 GB link peak compete with page cache and swap. On a laptop with about 2.5 GiB free, lean debuginfo matters more than the quiet-machine numbers suggest.
- C1 keeps file:line in backtraces and panics (`line-tables-only`). It loses local-variable inspection in a debugger for workspace code. When needed, re-enable it per crate (`[profile.dev.package.enth-engine] debug = true`) or with a `--config` override.
- **Optimising dependencies**, the common Bevy-style advice, buys a smaller binary and a faster link. On an egui + wgpu tree it multiplied the clean build by about 6 *(measured, noisy)*. That clean cost recurs for every fresh agent worktree, every `cargo clean` and every toolchain bump.
- enth's real-time code is in workspace crates, so the right lever is **per-package opt-level for the DSP crates** (C4). On these toy crates it cost nothing. On real `enth-dsp` / `enth-engine` it will cost something per edit; measure it in Task 3 or 5 (see Gaps).
- *Not measured:* `split-debuginfo = "unpacked"` (on Linux it would move DWARF into `.dwo` files and off the link path) and `-Zshare-generics`.

**Parallel front end.**

- *Documented.* It is not stabilised. The 2026 project goal lists deadlocks and ICEs fixed and the parallel front end enabled in bootstrap. Remaining work: a dedicated test suite, Cargo integration and inconsistent query-cycle errors. The goal page quotes no speedups and gives no date ([2026 goal](https://goals.rust-lang.org/2026/parallel-front-end.html)).
- *Measured* (nightly-2026-04-15, C1 profile):
  - Clean build: D0 135.6 s (load 19.6), D1 with `-Zthreads=4` 114.3 s (load 14.6). That is −16%, partly explained by the lower load.
  - Peak RSS: 837 MB → 1017 MB (+21%).
  - Incremental, interleaved (load 8.4–8.8): D0 1.40 s, D1 1.41 s, stable C1 1.41 s, i.e. no difference.

**Cranelift.**

- *Documented.* It is still a nightly-only rustup component, installed with `rustup component add rustc-codegen-cranelift-preview --toolchain nightly`. Enable it per profile with `codegen-backend = "cranelift"` and `-Zcodegen-backend` (or `cargo-features`).
- The README says Linux x86_64 is "fully supported and tested", `-Cpanic=abort` is the default, unwinding is experimental, `std::simd` works and `std::arch` is only partly supported ([in-tree README, fetched 2026-10-19](https://rust.googlesource.com/rust/+show/HEAD/compiler/rustc_codegen_cranelift/Readme.md)).
- I found no announcement that it reached stable.
- *Inference.* Partial `std::arch` support and no stable channel make it a poor fit for a DSP project that requires stable Rust (must-have g).
- *Not measured:* the component isn't installed on the owner's nightly toolchain, and adding it falls outside this session's permissions (see Gaps).

**sccache.**

- *Documented:* crates.io lists 0.18.0 (record updated 2026-09-16; queried 2026-10-19). It isn't installed, and I didn't measure it.
- *Inference.* It caches dependency compilation across target directories, not incremental workspace builds. Its main use here would be agents working in several git worktrees, where each worktree otherwise pays a full clean build (about 2–2.5 min and 1.3 GB with the lean profile, measured above).

**Recommended for a workspace with egui/wgpu and a plugin framework.** Keep the stable toolchain and the default rust-lld. Adopt C1, with per-crate O3 for the DSP crates. Move the repo to ext4. Treat wild or mold as a measured Task 1 option. Skip nightly levers. Details are under Recommendation.

**The repo's file system** (an addition: it came up while checking the machine).

- *Measured.* `findmnt` and `lsblk -f`: `/media/vboxuser/G-samsung` is `/dev/sdb2`, **NTFS mounted with ntfs-3g (fuseblk)**, on a "Samsung SSD 870 QVO 1TB" attached over USB. `/home` is ext4 on the internal WD SN720 NVMe, with 358 GB free.
- *Measured.* A Python `os.stat()` loop ran three times at 500 iterations each:

  | Location | Time per stat |
  | --- | --- |
  | `enth/docs/AGENTS.md` | 5.8 / 4.1 / 7.4 ms |
  | `/home` | 3.1–3.5 µs |
  | `/tmp` | 3.7–3.9 µs |

  - An earlier 20,000-iteration run gave 4.5 ms against 3.6 µs, and an open+read took 9.0 ms against 13 µs.
  - Caveat: `mount.ntfs-3g` was using 87% CPU at the time, from other activity, so this is a contended upper bound.
  - All files show mode 777 on that mount, so git's executable-bit tracking is meaningless there.
- I did not build on that mount: writing there was out of scope (see Gaps).

### Q2 — Hot reload

- **Dioxus subsecond.** crates.io lists 0.7.10 (record updated 2026-10-16); Dioxus is now at v0.8.0-alpha.3 (2026-10-16), per the GitHub releases API.
  - *Documented* ([docs.rs](https://docs.rs/subsecond/latest/subsecond/)):
    - You wrap entry points in `subsecond::call`. An external tool rebuilds the changed code and sends a new jump table. The ThinLink linker wrapper ships only inside the `dx` CLI, and hot-patching requires `dx serve --hot-patch`.
    - It is active only with `debug_assertions`.
    - Platforms include Linux x86_64 and aarch64.
    - It "currently only patches the 'tip' crate". Struct layout changes aren't supported, static initialiser changes aren't observed, and thread-locals reset on each patch.
  - The [Dioxus 0.7 post (2025-09-08)](https://dioxuslabs.com/blog/release-070/) says `dx` works with any Rust project, Bevy and Iced have adopted Subsecond, and the engine is to be spun out of `dx` later.
- **egui with subsecond.** *Anecdotal.* An egui maintainer's example ([lucasmerlin/egui-subsecond-example](https://github.com/lucasmerlin/egui-subsecond-example), 2025-09-08, no licence file) wraps `eframe::run_native` and the UI function in `subsecond::call`, registers a repaint handler, and runs under `dx serve --hot-patch`. A community egui template notes that hot-patchable code must live in the binary crate. That source came from a search result, and I didn't open the template.
- **hot-lib-reloader-rs.** *Documented* via the GitHub and crates.io APIs: version 0.8.2, last commit and release 2025-08-11, 763 stars, MIT. It reloads a `dylib` from a host binary.
- **Inside a DAW plugin?** *Inference.*
  - subsecond needs `dx` to launch the process it patches and only patches the tip binary crate. A `.clap` is a cdylib the DAW loads, so subsecond doesn't apply. The docs don't discuss cdylibs.
  - A hot-lib-reloader-style inner dylib inside the plugin is conceivable. It means unloading code on the host's threads, with statics, allocators and TLS crossing reloads, which is too risky for an audio plugin.
  - Verdict: in-plugin hot reload is not viable now. A standalone egui dev window is the place for it.
- **Recommendation.** Skip hot reload in Phase 0. With C1, an edit → rebuild → relaunch cycle of the standalone dev window costs about 1.5 s of build time on the toy workspace (measured). An optional two-hour subsecond trial on the standalone editor fits Phase 1, once the editor exists. eframe's state persistence could make relaunching cheaper (not checked; see Gaps).

### Q3 — Iterating in REAPER on Linux; host automation; headless hosts

- **The owner's REAPER can't load enth's Linux build.** The brief says REAPER 7.80 runs only under Wine/Bottles, as the Windows build. *Inference:* a Windows REAPER loads Windows `.clap` DLLs; a Linux `cdylib` is ELF and can't load there. Phase 0 Tasks 1 and 3 assume a native Linux REAPER scanning `~/.clap`, so a native Linux REAPER install has to be added. I didn't check the reaper.fm download page (see Gaps).
- **Rescan or reload.**
  - *Anecdotal.* A 2023 Linux guide says to rescan from Preferences or restart REAPER after installing CLAP plugins ([LinuxGameCast, 2023-08](https://linuxgamecast.com/2023/08/reaper-on-linux-installing-plugins/)).
  - *Anecdotal.* An iPlug2 forum thread on a REAPER dev build says a full CLAP rescan from Preferences was needed, and the F5 rescan in the FX browser was not enough ([iPlug2 forum, date not checked](https://iplug2.discourse.group/t/when-trying-your-clap-plugins-in-the-latest-reaper-dev-build-0917/705)).
  - I found no documented way to hot-swap a loaded plugin's binary in REAPER. *Inference:* the realistic loop is rebuild → `cargo xtask install` → restart REAPER, or remove the FX, rescan and re-add it → reload the project. Expect tens of seconds per cycle, which is why AGENTS.md already prefers the CLI, goldens and the standalone window.
- **Automating REAPER.**
  - *Documented.* ReaScript has `TrackFX_AddByName`, `TrackFX_GetParam`/`SetParam`(`Normalized`), `TrackFX_GetFormattedParamValue` and `Main_OnCommand` ([ReaScript API page](https://www.reaper.fm/sdk/reascript/reascripthelp.html); I read only the first 100k of 650k characters, and the page itself warns it may not be the latest version).
  - *Anecdotal.* A community REAPER MCP server renders through `Main_OnCommand(41824)` ([reaper-mcp render tool](https://glama.ai/mcp/servers/bonfire-systems/reaper-mcp/tools/render_project)).
  - I didn't verify REAPER's command-line render flags (`-renderproject`, `-nosplash`). See Gaps.
- **Headless CLAP host in Rust.**
  - *Documented:* clack-host 0.2.0 (crates.io record updated 2026-09-12).
  - *Anecdotal:* one search found no ready-made tool that loads a `.clap`, sends notes and writes a WAV. The `rack` crate lists CLAP as planned, and Firewheel has a proposal for a clack-based CLAP node.
  - *Inference:* build it into `tools/test-host` (see Recommendation).
  - clap-validator is already in the plan (Task 1).

### Q4 — Feedback loops for coding agents

**Measured loop timings** on the toy workspace, warm, config A unless noted; medians of three; load 12–18:

| Command after a one-line change in the leaf crate | Median | Notes |
| --- | --- | --- |
| `cargo test -p leaf --no-run` | 0.24 s | doesn't touch the GUI crates |
| `cargo nextest run -p leaf` | 0.62 s | build + run |
| `cargo clippy --workspace --all-targets` | 0.55 s | load about 12.5; reuses dependency metadata from `check` |
| `cargo check --workspace --all-targets` | 0.49 s | load about 12.5 |
| `cargo check --workspace` | 1.20 s | load about 16 |
| `cargo test --workspace --no-run` | 0.95 s | the test binary barely uses eframe |
| `cargo nextest run --workspace` | 1.13 s | |
| `cargo build --workspace` (relinks the egui binary) | 2.27 s (A) / 1.53 s (C1) | quiet round J |
| Clean `cargo check --workspace` | 92 s | load 16.7 |

- Keeping the agent's inner loop on `-p <crate>` tests and `check`/`clippy` keeps it under about a second. Relinking the GUI or the plugin is the expensive step.
- **What I did not reach** (see Gaps):
  - timings of insta snapshot tests and egui_kittest screenshot tests (egui_kittest 0.37.0, crates.io record 2026-10-15; insta 1.49.0, record 2026-10-03);
  - rust-analyzer memory and latency on this machine;
  - render-to-WAV plus descriptor loops, which only become measurable once enth's CLI exists;
  - **any 2025–2026 reports from people using Claude Code or Codex on Rust audio or plugin projects.** Not researched.
- **Measured: an API break that will trip agents.** egui/eframe 0.37 (2026-10-15) replaced `App::update`, so my first scratch app, written to the old API, failed with E0046/E0407/E0308. Code examples in nice-plug-egui, templates and LLM memory may target older egui. Pin the egui version through the shell's adapter and have agents check docs.rs for that version.

### Q5 — Test runner and watchers

- *Documented* via the crates.io API, queried 2026-10-19:

  | Crate | Latest | Record updated |
  | --- | --- | --- |
  | cargo-nextest | 0.9.148 (owner has 0.9.133) | 2026-10-08 |
  | bacon | 3.26.0 | 2026-09-26 |
  | cargo-watch | 8.5.3 | 2024-10-02 |

  So cargo-watch has had no release for two years. I didn't read its README for a maintenance statement (see Gaps).
- *Measured:* `nextest run -p leaf` took 0.62 s against 0.24 s for `cargo test -p leaf --no-run`. On a two-test crate, nextest's per-process model adds about 0.3–0.4 s. *Inference:* that overhead stays roughly constant while the speedup from parallel test processes grows with suite size. nextest is a good default for the workspace suite. For a one-crate inner loop, plain `cargo test -p` is just as good.
- *Inference.* Agents call commands directly, so watchers serve the human. bacon is the maintained choice. I didn't check bacon's export-to-file feature for agents.

### Q6 — Local measurements

Done; all numbers are above. Not done:
- mold and wild: not installed, deliberately. They are the Task 1 candidates.
- Cranelift: component not installed.
- sccache: not installed.
- clang + system lld: no system `ld.lld`; rust-lld is used anyway.
- A low-load re-run of the C2 clean build, and a link-time breakdown.

## Recommendation for Phase 0

1. **Task 1 installs and setup.**
   - **Move the working copy to `/home/vboxuser/…` (ext4 NVMe)** before Task 2's `git init`. Keep the USB NTFS copy as a backup only. If moving is impossible, at least set `CARGO_TARGET_DIR` to an ext4 path in the owner's shell.
   - **Install a native Linux REAPER** (x86_64 tarball, portable install) alongside the Wine one. The Linux `.clap` can only be tested there.
   - **Install no system lld**; rust-lld is already the default.
   - **Optionally install wild, or mold 3.0.** Adopt one only if a measured incremental build of `spike-shell` (plugin crate plus egui editor) improves by at least 20% over rust-lld with the lean profile. Keep it out of CI unless it's mold. For wild, configure it under `[target.x86_64-unknown-linux-gnu]` only, so wasm32 builds are unaffected, using the setup in wild's README (not verified today).
   - Skip sccache unless agents routinely work in several worktrees.
   - hyperfine is already in Task 1.
2. **Task 2: root `Cargo.toml` profiles** (measured effect in brackets):

    ```toml
    [profile.dev]
    debug = "line-tables-only"        # file:line backtraces; binary 377→93 MB, incremental 2.3→1.5 s, peak RSS 1.7→0.7 GB

    [profile.dev.package."*"]
    debug = false                     # no DWARF for egui/wgpu/winit/nice-plug

    # Real-time code runs optimised even in dev builds (add each crate as it appears):
    [profile.dev.package.enth-dsp]
    opt-level = 3
    [profile.dev.package.enth-engine]
    opt-level = 3
    ```

    - Don't set `opt-level` for `"*"` (+6× clean build, measured).
    - If the egui editor turns out too slow in debug builds inside REAPER, try `opt-level = 1` for named crates (`egui`, `epaint`) and measure; that wasn't done today.
    - Phase 0's spike crates (`spike-shell`, `spike-engine`) need the same `opt-level = 3` override to hit realtime in a debug bundle.
    - Leave `.cargo/config.toml` as planned (`[alias] xtask = …`), plus the optional linker block once Task 1 has a measurement.
    - Stay on stable in `rust-toolchain.toml`: no `-Zthreads`, no Cranelift.
3. **Task 3 and Task 4: dev harness.**
   - Add a standalone egui dev window for the editor early; nice-plug's standalone mode or an eframe window both work.
   - Extend `tools/test-host` (clack-host) beyond the editor and timing measurements with **`enth-test-host render <plugin.clap> --notes C3:100:2s --params … --out x.wav`**. Agents can then exercise the actual built plugin (state save and restore, parameters, audio) and diff it against `enth render` goldens without a DAW.
   - Record the edit → hear-in-REAPER cycle time (build, install, rescan or restart) in `docs/spikes/shell.md`, so the owner knows the real cost of host testing.
4. **Agent loop (AGENTS.md).**
   - Inner loop: `cargo check` / `cargo clippy -p <crate>` and `cargo nextest run -p <crate>` (or `cargo test -p`). These avoid linking the GUI or plugin and run in under about 1 s on this machine.
   - Workspace-wide `cargo nextest run --workspace` before reporting.
   - Bundle and install the plugin only for host checks.
   - With several agents running at once on this laptop, cap at `CARGO_BUILD_JOBS=4` per agent and at most two concurrent clean builds. With the lean profile, a clean build peaks at about 0.8 GB RSS (measured).

## Plan check

- **phase-0.md Task 1: should change.** (a) Add "move the repo to ext4", because of the NTFS-FUSE source location (measured stat latency 1000× worse, contended). (b) Add a native Linux REAPER install. The text assumes REAPER scans `~/.clap`, but the owner's REAPER is the Windows build under Wine (brief), which can't load a Linux `.clap` (inference). (c) Add the optional linker install as "measure, then adopt". (d) Note that rust-lld is already the default, so nobody chases a missing `lld`.
- **phase-0.md Task 2: should change.** Add the `[profile.dev]` block above to the root `Cargo.toml` listing. It is measured and cheap, and its memory saving matters on this machine.
- **phase-0.md Task 4: should change.** Give `tools/test-host` a `render` subcommand (notes in, WAV out) as well as editor open/close timing. It becomes the agent's headless way to test the plugin binary.
- **AGENTS.md "Fast feedback loops": small change.** Replace "fast linker, sensible profiles" with the concrete inner-loop commands and profile rules above. Keep "hot reload is a nice-to-have": the evidence says in-plugin hot reload isn't viable, and standalone hot-patching is optional.
- **phase-0.md Task 0, item 3** listed fast-iteration options under engine research. This report covers them, and nothing in it argues against Rust or a native UI.
- **No change** to the toolkit or shell choices from this axis. The egui 0.37 API break is a minor AI-familiarity cost, not a reason to switch.

## Gaps

Re-run after the reboot, on a quiet machine. Keep `CARGO_BUILD_JOBS=4`. The scratch workspace and script are in `/tmp/enth-research/06/`; `/tmp` may be wiped by the reboot. If so, recreate the three-crate workspace described under "Local measurement setup".

1. **Clean-build numbers on a quiet machine.** These were taken at load 6–28 with swapping. Settles the A / C1 / C2 clean-build ratios, especially the 743 s figure:
   `S=/tmp/enth-research/06/scripts/m.sh; L=(--config 'profile.dev.debug="line-tables-only"' --config 'profile.dev.package."*".debug=false'); $S A /tmp/e06/tA clean 1 -- cargo build --workspace; $S C1 /tmp/e06/tC1 clean 1 -- cargo build --workspace "${L[@]}"; $S C2 /tmp/e06/tC2 clean 1 -- cargo build --workspace "${L[@]}" --config 'profile.dev.package."*".opt-level=2'`
2. **Link share of an incremental build** (tells whether wild or mold can matter):
   `cargo build --workspace --timings` after a leaf edit, then read the `app` unit's time in `target/cargo-timings/`. Or use `RUSTFLAGS="-Clink-arg=-Wl,--time-trace"` and inspect the trace.
3. **wild and mold** (after the owner installs them in Task 1): add `[target.x86_64-unknown-linux-gnu] linker = "clang"`, `rustflags = ["-C", "link-arg=--ld-path=/path/to/wild"]` (or `.../mold`) to the scratch `.cargo/config.toml`. Then run the interleaved incremental loop from round J against C1.
4. **Cranelift** (needs the owner's OK to add a rustup component): `rustup component add rustc-codegen-cranelift-preview --toolchain nightly-2026-04-15`, then `CARGO_PROFILE_DEV_CODEGEN_BACKEND=cranelift cargo +nightly-2026-04-15 build -Zcodegen-backend --workspace` (clean and incremental). Also check that a DSP crate using `std::arch` compiles.
5. **The repo on NTFS-FUSE vs ext4.** Copy the scratch workspace to a folder on the G-samsung drive outside `enth/` (owner's OK needed) and time `cargo check` / `cargo build` incremental with the target dir both on ext4 and on the drive. Re-run the stat loop while ntfs-3g is idle.
6. **sccache with two worktrees:** time a clean build in a second checkout with and without `RUSTC_WRAPPER=sccache`.
7. **Real enth crates:** once `spike-shell` exists, re-measure incremental times with the profile above, including the per-edit cost of `opt-level = 3` on real DSP crates and whether a debug bundle runs 8 voices in realtime in REAPER.
8. **REAPER:**
   - Download page and licence terms for native Linux REAPER (reaper.fm).
   - Whether REAPER 7.x on Linux picks up a rebuilt `.clap` without restarting: test remove FX → *Preferences → Plug-ins → CLAP → Re-scan* → re-add.
   - The command-line render syntax (`reaper -renderproject <file.rpp>`, unverified) and a ReaScript that adds the plugin by name, sets parameters and renders. Check the REAPER user guide's command-line section.
9. **Headless CLAP hosts:** search GitHub more thoroughly for clack-host based CLI renderers, e.g. `gh search code "clack_host" "wav"` and `gh search repos clap host render`, before writing our own.
10. **Agent reports:** Claude Code or Codex on Rust audio or plugin projects, 2025–2026 (blogs, forums, GitHub discussions). Not researched at all.
11. **Test and UI loop timings:** insta snapshot test time; egui_kittest screenshot test time and whether it needs a GPU or software (lavapipe) wgpu adapter on this machine after the reboot fixes the NVIDIA driver mismatch; rust-analyzer RSS on the workspace.
12. **Watcher status:** read the cargo-watch README for a maintenance statement, and bacon's docs for its export (locations-to-file) feature for agents.
13. **eframe state persistence** (to make relaunching the dev window cheap): check eframe 0.37's `persistence` feature on docs.rs.
14. **Not measured at all:** `split-debuginfo = "unpacked"` and `-Zshare-generics`. To measure them, run C1 plus `--config 'profile.dev.split-debuginfo="unpacked"'` with full `debug = true`, using the round-J loop.
