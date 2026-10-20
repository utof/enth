# 04 Platforms, builds and distribution (R1 friend build, S1)

- **Researcher:** sub-researcher 04, 2026-10-20.
- **Scratch:** `~/code/enth-scratch/2026-10-20-r1-s1/04/`: `notes.md`, the build logs, `runs.txt` and `jobs.tsv` (CI usage), `repo-copy/` (the tree plus a trial `enth-standalone` crate), `soak/` (a REAPER sandbox and Lua probes).
- **Toolchain change:** I added the `aarch64-apple-darwin` and `x86_64-apple-darwin` targets to toolchain 1.99.1 (`rustup target remove` undoes it).
- **Repo:** unchanged apart from this file.
- **Labels:** *measured* (run here), *documented* (source or official docs), *anecdotal* (forums, reviews, third-party blogs).

## 1. Summary

The vendored fork already compiles for Windows and macOS. CI money is the real limit, and it is already tight. Friends can run unsigned builds with one manual step on macOS. The standalone app is the cheapest way to reach Logic users and friends with no DAW. AU can wait. The soak can be automated in the existing REAPER harness, but only through the virtual keyboard: the transport does not move under Dummy Audio.

| Area | Recommendation | Evidence | Confidence | Cost |
| --- | --- | --- | --- | --- |
| Fork on Windows and macOS | **It compiles.** Add a per-push guard on the Linux runner: `cargo clippy --target x86_64-pc-windows-gnu` and `--target aarch64-apple-darwin`, both `-p enth-plugin --all-targets --all-features -- -D warnings` | *Measured:* both targets pass check and clippy (37 s and 25 s cold). Nothing was linked or run | High for compiling; none for running | ~1–2 Linux min per push |
| Friend and release builds | **One `release.yml`, on a tag or by hand, never on push:** Linux; `windows-latest` (MSVC, `+crt-static`); `macos-latest` with `bundle-universal`. clap-validator runs on each leg's debug bundle | *Documented:* runner prices, nice-plug-xtask's bundler, clap-validator's Windows and macOS zips | Medium (build times estimated) | **~$1–1.5 per run (estimate)** |
| CI minutes | **Add `paths-ignore` for Markdown to the push triggers.** Fix the stale "no arm64 runners in private repos" comment | *Measured:* 1,458 Linux + 418 Windows minutes in the repo's first ~1.1 days; 69 of 107 commits on main touched only Markdown | High | Saves a large share of minutes |
| macOS signing | **R1:** unsigned, ad-hoc signed bundles plus an `xattr` line; or the standalone `.app`, approved once with "Open Anyway". **Public S1:** Developer ID + notarization in CI | *Documented:* Apple $99/yr; Sequoia's Gatekeeper change. *Anecdotal:* "Open Anyway" fails for plugins | Medium-high | $0 now; $99/yr later |
| Windows signing | **None for friends.** The standalone `.exe` gets SmartScreen's "Run anyway". **S1:** Azure Artifact Signing if eligible, else an OV certificate | *Documented:* Artifact Signing terms; Smart App Control blocks unknown unsigned DLLs | Medium | $0; later $9.99/month or ~€110–170/yr |
| Standalone app | **Ship it in the R1 friend build** as the L8 crate `enth-standalone` on nice-plug's `standalone` feature. The app must save its own state | *Measured:* compiles on Linux, Windows and macOS; `cargo deny` licences and bans pass | Medium-high | One small crate |
| AU for Logic | **Defer past S1.** If needed, wrap the release `.clap` with upstream clap-wrapper ≥ 0.16.0 through CMake in the macOS job, gated on `auval` | *Documented:* MIT licence and Apache-2.0 AudioUnitSDK (files read); `auval` in its CI | Medium | 1–2 days plus a Mac tester |
| DAW coverage | **VST3 + CLAP on Windows and macOS** covers Ableton, FL Studio, Cubase, Studio One, Bitwig and REAPER. Only Logic and GarageBand need AU or the standalone. Run the friend survey | §2.7 | Medium | — |
| One-hour soak | **`cargo xtask reaper-soak --minutes 60`:** debug bundle, virtual-keyboard notes, parameter sweeps, editor open/close, PSS and threads sampled. Local, not CI | *Measured:* a 3-minute probe ran end to end | High (method) | ~65 min of wall clock |
| Audio-thread checks | **`nightly.yml` has never run: start it by hand.** Add RTSan to the test host around CLAP `process()`; run clap-validator on Windows and macOS | *Measured:* 0 nightly runs | High | Nightly Linux minutes |
| Crash reports | **A local panic hook** through `Plugin::setup_logger`, writing a crash file the editor offers on next load. No signal handlers in a host | *Documented:* a panic in a host callback aborts the host; enth has no hook | High | Small |
| Delivery | **Friends can't download from a private repo:** the owner forwards zips or adds friends as read-only collaborators | GitHub access rules | High | — |

## 2. Findings

### 2.1 Does the fork build on Windows and macOS? (*measured*)

All runs used `flock -s`, 3 jobs and a scratch target directory, against the current tree.

| Command | Result |
| --- | --- |
| `cargo check --locked --target x86_64-pc-windows-gnu -p enth-plugin` | Pass, 0 warnings, 37 s, 765 MB RSS |
| The same for `aarch64-apple-darwin` and `x86_64-apple-darwin` | Pass, 0 warnings (25 s and 30 s) |
| `cargo clippy --locked --target {windows-gnu, aarch64-apple-darwin} -p enth-plugin --all-targets --all-features -- -D warnings` | Both pass |
| Trial `enth-standalone` (nice-plug `standalone` feature), checked for Linux, Windows and macOS arm64 | All three pass |
| `cargo deny --offline check licenses bans` with the standalone feature (34 more crates: cpal Apache-2.0, jack MIT and loaded with `dlopen`, midir MIT, alsa, coreaudio-rs, coremidi, objc2-*) | `bans ok, licenses ok` |

- **Not done:** linking (no mingw-w64 here; the macOS SDK only runs on Apple hardware) and running anything. The baseview + egui/glow editor has never run on Windows or macOS.
- **Known caveats:** baseview consumes every key there (ADR-0001 notes). OpenGL has been deprecated on macOS since 10.14; I found no evidence of removal in macOS 26.
- **Correction to the brief:** CI's Windows legs build with `1.99.1-x86_64-pc-windows-msvc`, rustup's default on `windows-latest` (*measured* from the job log), not `-gnu`.

### 2.2 CI runners, prices and the repo's own usage

**Prices per minute** (*documented*, [GitHub pricing](https://docs.github.com/en/billing/reference/actions-runner-pricing)):

| Runner | Price |
| --- | --- |
| Linux x64 | $0.006 |
| Linux arm64 | $0.005 |
| Windows x64 | $0.010 |
| Windows arm64 | $0.010 |
| macOS (M1 or Intel) | $0.062 |

- **Rounding:** each job rounds up to a whole minute.
- **Included minutes:** 2,000 a month on Free, 3,000 on Pro ([billing](https://docs.github.com/en/billing/concepts/product-billing/github-actions)).
- **Multipliers:** the docs give dollar rates only. Third-party pages still quote 2× (Windows) and 10× (macOS) against included minutes (*anecdotal*, unconfirmed).
- **2026 changes:** prices fell by up to 39 % on 2026-01-01; the self-hosted runner fee was postponed ([changelog](https://github.blog/changelog/2025-12-16-coming-soon-simpler-pricing-and-a-better-experience-for-github-actions/)).
- **arm64 runners:** since 2026-01-29, `ubuntu-24.04-arm` and `windows-11-arm` work in private repos with 2 vCPUs ([changelog](https://github.blog/changelog/2026-01-29-arm64-standard-runners-are-now-available-in-private-repositories)). So `engine-golden.yml`'s comment is out of date, and the manual aarch64 job can run.
- **macOS images:** `macos-latest` is macOS 26 on arm64; the macOS 14 image retires on 2026-11-02 ([changelog](https://github.blog/changelog/2026-10-01-github-actions-macos-14-runner-image-retirement)).

**The repo's usage** (*measured*, `gh api`, to 2026-10-20 03:00 UTC):
- **Totals:** 211 runs (200 on 2026-10-19), 800 jobs: 1,458 Linux, 418 Windows and 2 macOS minutes.
- **Cost:** $13.05 at list prices, or 2,314 "minutes" with 2×/10× multipliers, which is more than Free's quota.
- **One push to main:** about 32 Linux and 12 Windows minutes.
- **The cause:** `ci`, `budget`, `golden` and `engine-golden` run on every push to main without path filters, and 69 of the 107 first-parent commits since 2026-10-18 changed only Markdown.
- **Unknown:** the owner's plan (Gaps).
- **Implication:** at P1b's pace the included minutes last one or two days a month, and a macOS plugin build on every push would cost about $1 per push. Windows and macOS plugin builds belong on tags, manual runs and a weekly schedule.

**Cross-compiling versus native:**
- **Windows:** build artifacts natively with MSVC on `windows-latest`. MSVC links the C runtime dynamically by default ([Rust reference](https://doc.rust-lang.org/reference/linkage.html#static-and-dynamic-c-runtimes)), and a clean Windows may lack `vcruntime140.dll`, so set `-C target-feature=+crt-static` and check the imports once. A mingw build from Linux is cheaper but unlinked here. `cargo-xwin` pulls the MSVC SDK under Microsoft's licence: avoid it.
- **macOS:** build on `macos-latest`, add `x86_64-apple-darwin` on the same runner, and `nice-plug-xtask bundle-universal` merges the slices with `lipo` (*documented*, xtask 0.1.1 source). Cross-building from Linux needs Apple's SDK, whose licence ties it to Apple hardware.
- **The per-push guard:** compile-check both targets on Linux (§2.1).
- **Windows on ARM:** low priority, not researched.
- **Storage:** artifact storage is 500 MB on Free and 1 GB on Pro, and a debug bundle is 55–65 MB. Use `retention-days: 7`, and keep symbols as release assets.

### 2.3 Bundle layout and installing without an installer (*documented*)

- **Windows:** nice-plug-xtask 0.1.1 writes `enth-plugin.clap` (a DLL) and `enth-plugin.vst3/Contents/x86_64-win/enth-plugin.vst3` (`arm_64-win` for arm64).
- **macOS:** `*.clap|vst3/Contents/MacOS/…`, with `Info.plist` and `PkgInfo`. It ad-hoc signs every macOS bundle (`codesign -f -s -`), which arm64 may require, and bundles a binary as `*.app`.
- **Per-user paths, no admin needed:**
  - CLAP: `%LOCALAPPDATA%\Programs\Common\CLAP` and `~/Library/Audio/Plug-Ins/CLAP` ([`entry.h`](https://github.com/free-audio/clap/blob/main/include/clap/entry.h));
  - VST3: `%LOCALAPPDATA%/Programs/Common/VST3/` and `~/Library/Audio/Plug-ins/VST3/` ([VST3 locations](https://steinbergmedia.github.io/vst3_dev_portal/pages/Technical+Documentation/Locations+Format/Plugin+Locations.html)).
- So a zip plus "copy these folders" is enough for friends.

### 2.4 Signing, notarization, and what friends see

**macOS:**
- **Gatekeeper:** Sequoia removed the Control-click override for software that is not signed or notarized; users go to System Settings > Privacy & Security instead (*documented*, [Apple, 2024-08-06](https://developer.apple.com/news/?id=saqachfa)). That "Open Anyway" path works for apps.
- **Plugins** (*anecdotal*, [JUCE forum, 2024-12](https://forum.juce.com/t/mac-open-anyway-not-working-for-unsigned-mac-vst3/64574)): "Open Anyway" does not work for plugins. The tester sees "the developer can't be verified" with no prompt. Fixes: `xattr -dr com.apple.quarantine <bundle>`, or an unsigned `.pkg` the user allows.
- **Quarantine:** browsers, Mail, Messages and AirDrop set it; `curl` does not.
- **R1 path:** zip the ad-hoc-signed universal bundles, plus one Terminal line per bundle. Friends who won't use Terminal can use the standalone `.app`.
- **S1 path** (*documented*, [$99 a year](https://developer.apple.com/programs/), notarization included):
  - Developer ID Application signature with hardened runtime and timestamp;
  - a `.pkg` signed with Developer ID Installer, or a `.dmg`;
  - `notarytool submit`, then staple.
  - All of it runs on `macos-latest` with the certificate and an App Store Connect key as secrets.

**Windows:**
- **Plugin DLLs:** no source says SmartScreen checks a DLL a DAW loads (evidence missing). Unsigned free plugins are common (*anecdotal*).
- **The exception is Smart App Control** (*documented* by a Microsoft PM, [2026-04-28](https://textslashplain.com/2026/04/28/smart-app-control/)): it "evaluates trust/signatures of all code (DLLs, scripts, etc)", and blocks an unknown unsigned plugin DLL with "Part of this app has been blocked", with no override. It is on only on some clean Windows 11 installs; since 25H2 a user can switch it back on.
- **Standalone `.exe`:** "Windows protected your PC" → "More info → Run anyway" (*anecdotal*, [HISE forum](https://forum.hise.audio/post/122131)).
- **Artifact Signing** (formerly Trusted Signing; GA January 2026; [DevClass](https://devclass.com/2026/01/14/code-signing-windows-apps-may-be-easier-and-more-secure-with-new-azure-artifact-service/)): $9.99 a month for 5,000 signatures; 24-hour certificates, with signatures timestamped; not EV.
  - Eligibility: "organizations or individuals in the USA and Canada, or organizations in the EU or UK".
  - Billing starts at account creation and is not prorated ([pricing](https://azure.microsoft.com/pricing/details/artifact-signing/)).
- **Fallbacks:** a Certum OV certificate in a cloud HSM, about €109–169 a year (*anecdotal*, [SSLPoint](https://sslpoint.com/eu/code-signing)). SignPath Foundation (free for OSI-licensed projects with no proprietary parts and manual approval of each release; [terms](https://signpath.org/terms), a draft that mentions neither a public repo nor macOS).

**Delivery:** release assets and Actions artifacts in a private repo need a GitHub login with access.

### 2.5 Standalone app (*documented* from the vendored nice-plug 0.4.2 source; it compiles: *measured*)

**The wrapper:**
- `nice_export_standalone::<Enth>()` opens the editor and blocks until it closes. Its doc says "mainly useful for quickly testing plugin GUIs".
- **Backends:** `auto` tries JACK, then ALSA, CoreAudio or WASAPI. ASIO is optional and needs the ASIO SDK: skip it.
- **Settings:** 48 kHz and a 512 period by default, set by flags; there are no settings screens.
- **MIDI:** input only with `--midi-input`.
- **State:** nothing is saved between runs.

**For enth:**
- R1's round needs no MIDI: Target ▶ / Mine ▶ use the editor's audition notes.
- Defaults can be passed through `nice_export_standalone_with_args`.
- The standalone shell must save the patch, settings and game progress to the user's data folder. Shells may do I/O.
- Add `#![windows_subsystem = "windows"]` so no console window opens.
- `standalone` is already an L8 name in AGENTS.md, so the crate needs no layer change.
- **Not run:** real audio, the `.app`, the `.exe` (Gaps).

### 2.6 AU for Logic

- **Licences** (*documented*, files read): clap-wrapper's `LICENSE` is MIT, "Copyright (c) 2022 defiantnerd". Apple's AudioUnitSDK is Apache-2.0. The VST3 SDK is now MIT ("Copyright (c) 2026, Steinberg").
- **Activity:** v0.16.0 on 2026-08-08; last push 2026-10-19; 294 stars; 7 open issues mention AU, among them AUv2 #573, a resize re-entrancy bug (2026-09-27).
- **Maturity:** the wiki called AUv2 "feature complete" at 0.9.0. 0.14.0 (March 2026) put `auval` in CI, and the project says it is "not yet at a 1.0 release" ([ChangeLog](https://github.com/free-audio/clap-wrapper/wiki/ChangeLog)).
- **nice-plug can use it** by wrapping the exported `clap_entry`:
  - **(a) clap-wrapper-rs**, the crate `clap-wrapper` 0.3.1 (2026-05-18, MIT OR Apache-2.0, [repo](https://github.com/blepfx/clap-wrapper-rs)): `export_auv2!()` compiles the C++ with `cc`. But it bundles clap-wrapper **0.14.0**, which lacks 0.16.0's VST3 state fix (ADR-0001 notes). It has one maintainer and 6,823 downloads, and it puts a C++ toolchain into the workspace (AGENTS.md: ask first).
  - **(b) upstream clap-wrapper through CMake in the macOS job only**, wrapping the built `.clap` into `enth.component` and gated on `auval -v aumu …`. The Rust workspace is untouched. Prefer (b).
- **Defer:** the standalone covers Logic users in R1, and the owner has no Mac for Logic tests.

### 2.7 DAWs and format support (2026)

| DAW | CLAP | VST3 | AU | Source |
| --- | --- | --- | --- | --- |
| Ableton Live 12.4 (May 2026) | no | yes | Mac | Antares, 2026-07-27; no CLAP in Ableton's press releases |
| Logic Pro / GarageBand | no | no | **only** | Antares |
| FL Studio | **yes, since 2024.1** | yes | Mac | [Image-Line changelog](https://forum.image-line.com/viewtopic.php?p=1956185) (the Antares table is wrong here) |
| Cubase 15 (Nov 2025) | no | yes | — | Launch coverage |
| Studio One Pro 7 / Fender Studio Pro 8 | yes | yes | Mac | Reviews (*anecdotal*); [CLAP host list](https://cleveraudio.org/hosts-and-plug-ins/) |
| Bitwig, REAPER | yes | yes | Mac | CLAP host list |

- **Popularity** (*anecdotal*): there is no solid 2025–26 share data. [Production Expert 2025](https://www.production-expert.com/production-expert-1/2025-daw-survey-the-results) (2,500+ answers, a professional crowd) puts Pro Tools first, then Logic and Studio One. Hobbyist sources put Ableton, Logic and FL Studio on top. The friend survey decides.
- **Decision:** VST3 and CLAP plus the standalone covers everyone except Logic purists and Pro Tools (AAX is out).

### 2.8 The one-hour REAPER soak, headless (*measured*)

**Setup:** REAPER 7.82 in a scratch sandbox under `xvfb-run`, `linux_audio_mode=2` (it reports "Dummy Audio", 48 kHz / 512), and the repo's current debug `libenth_plugin.so`, copied. In that build `assert_process_allocs` aborts on any allocation in `process()`.

**What happened:**
- **The transport does not move under Dummy Audio:** after `Transport: Play` or `OnPlayButton()`, `GetPlayState()` = 0 and the position stays 0.000, while `Audio_IsRunning()` = 1. So arrange-view MIDI items and envelopes don't play headless; the existing scenarios use offline renders.
- **The virtual keyboard works:** with the track armed and monitored (`I_RECINPUT = 4096+(63<<5)`, `I_RECMON = 1`, `I_RECARM = 1`, `I_PERFFLAGS = 2`), `StuffMIDIMessage` notes are processed in real time.
- **Level:** the peak reached 2.37–2.49 (linear) for a four-note chord on `Patch::init()`.

**The probe** (`soak-probe.lua`), run for 180 s and then 120 s:
- **Load:** a chord every 0.5 s, a macro sweep at 4 Hz (668 and 444 sets), the editor toggled every 10 s (17 and 11 times).
- **Result:** both runs finished with no abort, so no audio-thread allocation. `GetUnderrunTime()` stayed 0/0.
- **Threads:** a constant 27 (closed) and 54 (open).
- **PSS with the editor closed:** 195 → 216.6 → 224.1 → 224.9 → 225.4 → 225.9 MB. That is about 0.5–0.7 MB per open/close cycle after warm-up, over 4 cycles, and noisy (Mesa, SWELL). The test host's leak gate allows 26 KB per cycle, so 60 minutes must show slope or plateau.

**Recommended design** (extending `xtask/src/reaper.rs`):
- `cargo xtask reaper-soak --minutes 60 [--format clap|vst3]`, with its own timeout instead of `LAUNCH_TIMEOUT_SECS = 90`.
- 4 instances.
- Virtual-keyboard patterns: chords, runs, 16-voice stealing bursts.
- Macro and slot sweeps as the automation stand-in.
- A state load every 5 minutes through the harness's state injection.
- The editor opened and closed every 60 s, with an XTest edit through `xinput.py`.
- PSS, threads and file descriptors sampled every 30 s.
- **Pass:** the `finished` line exists; threads and fds end where they started; after 10 minutes the PSS slope with the editor closed stays under 5 MB/h (a proposed budget).
- **Scope:** Dummy Audio has no deadline. Deadlines come from `enth-test-host process-stats` under rtkit RR; real xruns from the owner's real-device session.
- **Where:** run it locally before GS1 and after editor lanes. A nightly hour in CI costs about 1,800 Linux minutes a month, and REAPER isn't in CI; a 5-minute test-host soak fits the nightly workflow instead.

### 2.9 Audio-thread allocation checks in CI

- **Today:**
  - engine tests run under `assert_no_alloc`;
  - clap-validator and host-render use the debug bundle, where nice-plug's `assert_process_allocs` wraps the whole callback and nice-assert-no-alloc 1.0.0 aborts by default (*documented*: `warn_debug` is off);
  - the nightly RTSan job covers the engine and render crates.
- **But `nightly.yml` has 0 runs** since it landed on 2026-10-19 07:58 UTC (*measured*; its 02:17 UTC slot had not fired by 03:00 UTC). RTSan, proptest exploration and the leak gates have never run in CI. Start it once with `workflow_dispatch`.
- **Add (1):** P1b report 03's recommendation 6, an RTSan-built `enth-test-host`, nightly with a pinned date and linked with `--export-dynamic`, that wraps CLAP `process()` in `__rtsan_realtime_enter/exit`. In P1b this caught `malloc` in a plain stable-built cdylib (*measured*). The symbol is internal, hence the pinned date.
- **Add (2):** clap-validator on the debug bundle in the Windows and macOS legs. 0.4.1 ships those zips (*documented*, [releases](https://github.com/free-audio/clap-validator/releases), 2026-07-22). `xtask validate` refuses to run off Linux x86-64 today, so it needs a per-OS pin and hash.
- **Not needed:** `rtsan-standalone` 0.3.0 (Apache-2.0, last release 2025-09-27). Nightly rustc ships the runtime.

### 2.10 Crash reporting without telemetry

- **Today** (*documented*):
  - since Rust 1.81, a panic escaping `extern "C"` aborts ([Rust 1.81](https://blog.rust-lang.org/2024/09/05/Rust-1.81.0/));
  - the vendored nice-plug has no `catch_unwind`, so a panic in any host callback takes the DAW down;
  - nice-plug installs its panic hook only through its logger, and enth builds it without `tracing-subscriber` and does not override `Plugin::setup_logger()`. So no hook exists, and a release panic leaves nothing except the host's crash log.
- **Plugin:** implement `Plugin::setup_logger()`, which runs once per library load. Install a chained panic hook that keeps the previous one, since hooks are process-wide.
- **What the hook writes:** `enth-crash-<time>.txt` with version, git hash, target, thread, message, location and `Backtrace::force_capture()`, into `%LOCALAPPDATA%\enth\crash`, `~/Library/Logs/enth` or `$XDG_STATE_HOME/enth/crash`. Wrap the write in `permit_alloc`.
- **Next load:** the editor shows "enth crashed last time: open folder". Sending the file is the friend's choice.
- **Not in the plugin:** signal or SEH handlers; they fight the DAW's crash reporter.
- **Native crashes:** use the DAW's dumps, macOS `~/Library/Logs/DiagnosticReports/*.ips`, or Windows WER LocalDumps (admin to enable; default `%LOCALAPPDATA%\CrashDumps`; skipped for apps with their own crash reporting; [Microsoft Learn](https://learn.microsoft.com/en-us/windows/win32/wer/collecting-user-mode-dumps)).
- **Symbols:** keep PDB, dSYM and debuginfo per release as release assets.
- **Standalone:** it owns its process, so it may later add `crash-handler` 0.8.1 and `minidumper` 0.11.0 (MIT OR Apache-2.0).

## 3. Risks and open questions

1. **Nothing has run on Windows or macOS** (the editor, audio, state, focus, HiDPI). The release legs run clap-validator; ask one Windows friend and one Mac friend for a 15-minute checklist. On Windows, baseview may swallow DAW shortcuts.
2. **The CI budget is already strained.** Decide the plan and spending limit before adding legs; path filters first.
3. **macOS OpenGL** is deprecated. If it is removed, egui on glow breaks (ADR-0002).
4. **Smart App Control** blocks unsigned DLLs, with no override: a friend with it on needs a signed build.
5. **The default patch peaks at +7.9 dBFS** on a four-note chord (*measured*). The friend build should start quieter (report 03, presets).
6. **Editor PSS growth in REAPER**, about 0.5–0.7 MB per cycle, is unexplained (perhaps Mesa or SWELL). The 60-minute soak decides.
7. **Standalone limits:** no saved state, no device screen, fixed 48 kHz / 512. A device that refuses 48 kHz is untested.
8. **The macOS x86_64 slice** is never run in CI without an Intel job (that job's retirement date is unchecked).

## 4. Gaps

- **The owner's GitHub plan:** the billing API needs the `user` scope (`gh auth refresh -s user`, or the billing page). Also whether 2×/10× multipliers still apply.
- **Linking and running on Windows and macOS:** not done. The first manual `release.yml` run settles it, so the release workflow's cost figures are estimates.
- **The standalone on a real device**, and the `.app` and `.exe`: not run (no audio allowed here).
- **"Open Anyway" for plugins on macOS 15 and 26**, and whether files installed by an unsigned `.pkg` carry quarantine: only late-2024 forum evidence.
- **SmartScreen and DLLs** loaded by a host: no primary source.
- **clap-wrapper's AUv2 in Logic:** the wiki's compatibility page is empty, and I found no shipping Logic reports.
- **Windows on ARM** host and plugin support: not researched.
- **Dummy Audio:** whether it ever reports xruns, and why the transport doesn't start. I measured only that it doesn't.
- **OpenGL in macOS 26:** status not found.

## 5. Sources (read 2026-10-20 unless dated)

- GitHub: [runner pricing](https://docs.github.com/en/billing/reference/actions-runner-pricing); [billing](https://docs.github.com/en/billing/concepts/product-billing/github-actions); [2026 pricing](https://github.blog/changelog/2025-12-16-coming-soon-simpler-pricing-and-a-better-experience-for-github-actions/); [arm64 in private repos](https://github.blog/changelog/2026-01-29-arm64-standard-runners-are-now-available-in-private-repositories); [macOS 14 retirement](https://github.blog/changelog/2026-10-01-github-actions-macos-14-runner-image-retirement).
- Apple: [Developer Program](https://developer.apple.com/programs/); [Sequoia runtime protection](https://developer.apple.com/news/?id=saqachfa).
- Forums: [JUCE forum](https://forum.juce.com/t/mac-open-anyway-not-working-for-unsigned-mac-vst3/64574); [HISE forum](https://forum.hise.audio/post/122131).
- Microsoft and signing: [Smart App Control](https://textslashplain.com/2026/04/28/smart-app-control/); [Artifact Signing](https://learn.microsoft.com/en-us/azure/artifact-signing/overview) and [pricing](https://azure.microsoft.com/pricing/details/artifact-signing/); [DevClass](https://devclass.com/2026/01/14/code-signing-windows-apps-may-be-easier-and-more-secure-with-new-azure-artifact-service/); [SignPath terms](https://signpath.org/terms); [SSLPoint](https://sslpoint.com/eu/code-signing); [WER LocalDumps](https://learn.microsoft.com/en-us/windows/win32/wer/collecting-user-mode-dumps).
- CLAP and wrappers: [clap-wrapper](https://github.com/free-audio/clap-wrapper) and its [ChangeLog](https://github.com/free-audio/clap-wrapper/wiki/ChangeLog); [clap-wrapper-rs](https://github.com/blepfx/clap-wrapper-rs); [AudioUnitSDK LICENSE](https://github.com/apple/AudioUnitSDK/blob/main/LICENSE.txt); [vst3sdk LICENSE](https://github.com/steinbergmedia/vst3sdk/blob/master/LICENSE.txt); [clap entry.h](https://github.com/free-audio/clap/blob/main/include/clap/entry.h); [VST3 locations](https://steinbergmedia.github.io/vst3_dev_portal/pages/Technical+Documentation/Locations+Format/Plugin+Locations.html); [clap-validator releases](https://github.com/free-audio/clap-validator/releases).
- DAWs: [Antares, 2026-07-27](https://www.antarestech.com/blog/vst3-au-aax-clap-plugin-formats-2026); [Image-Line 24.1.1](https://forum.image-line.com/viewtopic.php?p=1956185); [CLAP hosts (Nov 2024)](https://cleveraudio.org/hosts-and-plug-ins/); [Cubase 15, 2025-11-06](https://sonicstate.com/news/2025/11/06/steinberg-releases-cubase-15/); [Production Expert 2025](https://www.production-expert.com/production-expert-1/2025-daw-survey-the-results).
- Rust: [Rust 1.81](https://blog.rust-lang.org/2024/09/05/Rust-1.81.0/); [crt-static](https://doc.rust-lang.org/reference/linkage.html); crates.io API (rtsan-standalone, assert_no_alloc, nice-assert-no-alloc, crash-handler, minidumper, clap-wrapper).
- Local: vendored nice-plug 0.4.2 (`wrapper/standalone*`, `wrapper/util.rs`, `plugin.rs`); nice-plug-xtask 0.1.1 and nice-assert-no-alloc 1.0.0 (cargo registry); `xtask/src/reaper.rs`, `validate.rs`; the workflows; P1b `03-realtime-host.md`.
