# Friend builds and distribution

> **As of:** 2026-10-20 (lane FB of the R1/S1 milestones). **Evidence:** research report 04 (`_local/sessions/2026-10-20-r1-s1-recreate-and-instrument/04-platforms-distribution.md`) and its summary, which wins where they differ. **Status:** the per-push cross-target guard runs on every push; `release.yml` is written and checked for syntax but **has never run**, so nothing here has been linked or run on Windows or macOS yet, and the cost figures are estimates. The first run settles both.

Friends play the Recreate game on Windows and macOS, so they need plugin files they can load. This page says how to produce them, what they do with them, what it costs, and what is still undecided. The page to forward to a friend is [friends-install.md](friends-install.md).

## What exists

| Piece | Where | Runs |
| --- | --- | --- |
| Cross-target guard | job `cross-target` in `.github/workflows/ci.yml` | every push to main and every pull request that changes more than Markdown or `docs/` |
| Friend build | `.github/workflows/release.yml` | a pushed tag `v*`, or by hand; **never on a push to main** |
| `cargo xtask bundle --universal` | `xtask/src/bundle.rs` | macOS only; the release macOS leg |
| Install sheet | [friends-install.md](friends-install.md) | copied into every zip as `INSTALL.md` |

## The per-push guard

`cargo clippy -p enth-plugin --all-targets --all-features --locked --target <T> -- -D warnings` for `x86_64-pc-windows-gnu` and `aarch64-apple-darwin`, two parallel jobs on `ubuntu-24.04`. It catches code that stops compiling or starts warning on Windows or macOS (a `cfg(unix)` slip, a missing import, a new lint) within a couple of minutes of the push, for the price of Linux minutes.

- Measured locally (3 jobs, cold target directory): 38 s and 29 s, exit 0 for both, and also with `PKG_CONFIG=/bin/false`, so the jobs install no Linux libraries.
- It stops after type checking: nothing is linked and nothing runs. The Windows target is `-gnu` because it needs no linker for a check; the code is the same `cfg(windows)` as for MSVC. The MSVC link, the static runtime and the macOS universal merge are only exercised by `release.yml`.
- The jobs' cache key is the target (the matrix legs share a job id and would otherwise overwrite each other's cache); only main writes it.

## Cutting a friend build

1. **Pick a commit that is green on main** (the `ci` workflow and the goldens passed).
2. **Start the workflow**, either way:
   - **Tag** (starts all three legs): `git tag v0.1.0-friend1 <commit>` then `git push origin v0.1.0-friend1`. Name tags `v<version>-friend<N>`; the plugin's own crate version is still `0.0.0`, so the tag is the build's name.
   - **By hand** (choose the legs): Actions tab > **release** > **Run workflow**, pick the branch or tag, untick the legs you do not need. The same from a terminal: `gh workflow run release.yml --ref main -f macos=false`. A run by hand on a branch is named `dev-<7-digit commit>`.
3. **Wait** for the run (estimates below). Each job's summary lists its zip's size and SHA-256.
4. **Download** the zips from the run's page, under **Artifacts** (`gh run download <run-id>` works too, untried). They are named `enth-<version>-linux-x86_64.zip`, `enth-<version>-windows-x86_64.zip` and `enth-<version>-macos-universal.zip`, each uploaded as the zip itself, so a download is the zip and not a zip of a zip. **They expire after 14 days** (Actions artefacts; a release asset would not expire, see [Delivery](#delivery)).
5. **Forward** the zip for each friend's OS together with [friends-install.md](friends-install.md) (the zip carries a copy as `INSTALL.md`).

Each zip holds a folder `enth-<version>-<os>/` with `enth-plugin.clap`, `enth-plugin.vst3`, `INSTALL.md`, `gr1-friends.md` (how to play the 20 rounds and send the log; added by R1/S1 lane INT, so `INSTALL.md`'s link to it works inside the zip), `LICENSE`, `THIRD_PARTY_NOTICES.md` and `BUILD.txt` (version, commit, compiler, runner). Bug reports quote the first two lines of `BUILD.txt`.

**The lesson packs and GR2's sheets** (R2 lane INT2, 2026-10-22): each zip also holds `Packs/` with the three packs the plugin does not bundle (`shaping.enthpack`, `space-and-dynamics.enthpack` and `studies.enthpack`, built on the leg by `cargo xtask pack`; the tour needs the first two), `gr2-friends.md` (adding the packs through the Learn view's **Open packs folder**, the tour, the tests, what to send back) and `gr2-answers.toml` (the answers template, `content/gr2/answers-template.toml`). The `basics` pack is inside the plugin. The pack step builds the lesson tool on each runner, a few minutes more per leg; the tool passes clippy for the Windows and macOS targets locally, but the step has not run on those runners yet.

**The Linux leg with packs, rehearsed locally** (R2 lane INT2, 2026-10-22, at `620a67f`; no workflow was run): `cargo xtask validate --package enth-plugin` (debug bundle: clap-validator 0.4.1, 36 passed, 0 failed, 8 skipped; 236 s at a load of about 27), `cargo xtask bundle --package enth-plugin --release --locked` (419 s), `validate --release --no-build` (36, 0, 8), then the leg's packaging commands by hand: a **14,255,295-byte zip** (SHA-256 `1e2b59cc…`) holding the folder with the plugin (the `.clap` 19,723,472 bytes, 15,208,840 stripped, default features `recreate` and `learn`), `INSTALL.md`, both friend sheets, `gr2-answers.toml`, `LICENSE`, `THIRD_PARTY_NOTICES.md`, `BUILD.txt` and `Packs/` (148,956 + 108,770 + 104,439 bytes, the SHA-256s `pack --check` reproduces). The packs read through the same verified reader the plugin's Packs folder uses (`enth lesson list` on the three files: every lesson and the tour track; [r2-gates.md](r2-gates.md) row G4).

**The Linux leg, rehearsed locally** (R1/S1 lane INT, 2026-10-20, at `192749c`): the release bundle, clap-validator 0.4.1 on the debug and release bundles (36 passed, 0 failed, 8 skipped each), Steinberg's VST3 validator (47 of 47, both), then the leg's packaging commands by hand: an 11 MB zip (each plugin file 16 MB unpacked; FB measured 7.8 MB before the Recreate round) with the folder and the seven entries above. The unzipped `.clap` loads and plays in the test host and links only `libc`, `libm` and `libgcc_s`, but **needs glibc 2.39** (Ubuntu 24.04, Debian 13, Fedora 40 or newer): a friend on Ubuntu 22.04 cannot load it. Building that leg on `ubuntu-22.04` would lower it to 2.35; not done, since no friend is known to need it.

### What each leg does

| Leg | Runner | Steps |
| --- | --- | --- |
| Linux | `ubuntu-24.04` | system libraries, `cargo xtask validate` (debug bundle, the pinned clap-validator 0.4.1), `cargo xtask bundle --release --locked`, validator on the release bundle, zip |
| Windows | `windows-latest` (Server 2025, MSVC) | `RUSTFLAGS=-C target-feature=+crt-static`; debug bundle; clap-validator 0.4.1 fetched and checked against its SHA-256, run on the debug bundle; release bundle; `dumpbin /dependents` must show no `vcruntime`/`msvcp` DLL; validator on the release bundle; 7-Zip |
| macOS | `macos-latest` (macOS 26, arm64) | both Apple targets installed; `MACOSX_DEPLOYMENT_TARGET=11.0`; debug bundle (arm64) and validator; `cargo xtask bundle --release --universal` (x86-64 and arm64 built one after the other, merged with `lipo`); ad-hoc `codesign --force --deep --sign -` and `codesign --verify`; `lipo -archs` must list both; validator on the release bundle; `ditto` zip, whose signature is verified again after unpacking |

- The debug and release bundles share `target/bundled`, so every leg validates the debug bundle first and builds the release one after.
- The debug bundle has nice-plug's `assert_process_allocs` on, which aborts the validator on any allocation in `process()`. The validator on the release bundle checks the file that ships.
- The two non-Linux validators are the 0.4.1 release assets, fetched with the generic User-Agent `enth-research/0.1` and pinned by the SHA-256 GitHub lists for them (the Linux asset's listed digest equals the pin in `xtask/src/validate.rs`). `xtask validate` stays Linux x86-64 only.
- `CARGO_BUILD_WARNINGS` is not set in `release.yml` (it is in `ci.yml`): the guard already fails a warning in our code, and a new linker warning on a runner image must not block a friend build.
- By-hand inputs: `linux`, `windows`, `macos` (tick the legs) and `validate` (turn off only while a validator problem is investigated).

### What to check on the first run

Written without a Windows or macOS machine, so these are the steps most likely to need a fix:

| Step | What could be wrong |
| --- | --- |
| Fetch clap-validator (Windows, macOS) | The asset layout is read from upstream's build workflow (the Windows zip holds `clap-validator.exe`, the macOS one a tarball with `binaries/clap-validator`); the steps `find` the binary, but nobody has unpacked these two zips |
| `dumpbin` (Windows) | Found through `vswhere`; if it cannot be run the check is skipped with a warning, so read the log for `::warning::` |
| Release link (Windows) | First MSVC link of the plugin; `+crt-static` also applies to build scripts and proc macros |
| `codesign --verify` (macOS) | nice-plug-xtask writes a minimal `Info.plist`; a verify failure would stop the leg |
| Zip names and layout | 7-Zip's `-bso0 -bsp0` flags; the zip's single top folder |
| Job times | Replace the estimates below, and write them into this page |

## Cost per run

**Since 2026-10-20 the repository is public, so GitHub's standard runners cost no money** ([Delivery](#delivery)); the dollar figures below were for the private repository and stay as a measure of runner time (a run still takes 20 to 55 runner minutes, and a macOS job is the slowest). Estimates, not measurements. They come from the repository's own job times (the Linux plugin-validation job took 2 minutes warm and 8 to 9 minutes cold when it built two spike plugins; its Windows twins run 1.3 to 1.7 times longer) and from list prices per minute: Linux $0.006, Windows $0.010, macOS $0.062 (GitHub, 2026-01-01 prices). Every job rounds up to a whole minute. Private-repository runners have 2 vCPUs (Linux, Windows) or 3 (macOS).

| Leg | Builds | Cold (no cache) | Warm (cache restored) |
| --- | --- | --- | --- |
| Linux | debug + release | ~12 min, $0.07 | ~4 min, $0.02 |
| Windows | debug + release | ~19 min, $0.19 | ~7 min, $0.07 |
| macOS | debug + release x86-64 + release arm64 | ~23 min, $1.43 | ~9 min, $0.56 |
| **All three** | | **~54 min, about $1.7** | **~20 min, about $0.65** |

- **macOS is 80 % of the cost.** A run without it is about $0.26 cold; the by-hand inputs exist for that.
- **Cold or warm:** a tag run reads the cache a by-hand run on `main` saved, and cannot save its own. The first run is cold. Do one by hand on `main` before the first tag run if the second one matters.
- **Against included minutes:** if the owner's plan applies the multipliers third parties report (Windows x2, macOS x10; GitHub's current docs give only dollar rates), a cold run uses about 280 minutes of the 2,000 (Free) or 3,000 (Pro) a month. The plan is not known yet (report 04: it needs `gh auth refresh -s user`, or the billing page).
- **For scale:** one push to main used about 32 Linux and 12 Windows minutes before the docs-only filter (report 04). The guard adds two short Linux jobs to each code push.
- **Storage:** the Linux zip measured 7.8 MB (the release `.clap` and the `.vst3` are 11 MB each); Windows and macOS (two architectures) will be larger, so a run keeps perhaps 30 to 45 MB for 14 days, inside the 500 MB (Free) or 1 GB (Pro) artefact quota.
- **Worst case:** the job timeouts (Linux 40, Windows 50, macOS 50 minutes) cap a hung run at about $4 ($3.10 of it macOS).

## What friends do

The full steps are in [friends-install.md](friends-install.md). In short: unzip, copy one plugin file into the plugin folder, rescan in the DAW.

| | Windows | macOS |
| --- | --- | --- |
| Signature | none | ad hoc (`codesign -s -`): arm64 macOS wants some signature, but it is not a developer identity |
| What the system says | No source says SmartScreen checks a DLL that a DAW loads (report 04: evidence missing), and unsigned free plugins are common. **Smart App Control** (Windows 11, on only for some clean installs) blocks an unknown unsigned DLL with "Part of this app has been blocked" and no override | "The developer cannot be verified". **"Open Anyway" in System Settings does not work for plugins** (a JUCE forum report from 2024; anecdotal; it works for apps) |
| The fix | Unblock the zip before extracting (Properties > Unblock). Smart App Control has no fix short of a signed build | `xattr -dr com.apple.quarantine <folder>` before or after copying. Quarantine is set by browsers, Mail, Messages and AirDrop, not by `curl` |
| Runtime | `+crt-static`: no Visual C++ redistributable needed | macOS 11 or newer, Apple silicon and Intel in one universal bundle; the x86-64 slice is never run in CI |

### Install paths (no administrator needed)

| OS | CLAP | VST3 |
| --- | --- | --- |
| Windows | `%LOCALAPPDATA%\Programs\Common\CLAP` (system-wide: `C:\Program Files\Common Files\CLAP`) | `%LOCALAPPDATA%\Programs\Common\VST3` (system-wide: `C:\Program Files\Common Files\VST3`) |
| macOS | `~/Library/Audio/Plug-Ins/CLAP` | `~/Library/Audio/Plug-Ins/VST3` |
| Linux | `~/.clap` | `~/.vst3` |

On Windows the CLAP file is `enth-plugin.clap` (a DLL) and the VST3 is the folder `enth-plugin.vst3\Contents\x86_64-win\enth-plugin.vst3`; on macOS both are bundles (`Contents/MacOS/enth-plugin`, `Info.plist`, `PkgInfo`). Sources: CLAP's `entry.h` and Steinberg's VST3 location page, as report 04 §2.3 read them. A DAW that scans only the system-wide VST3 folder needs the file copied there (one administrator prompt).

## Delivery

**The repository is public since 2026-10-20** (`utof/enth`; the earlier private repository is archived as `utof/enth-private-archive`). Standard GitHub-hosted runner minutes are free for public repositories, which is why it went public. Actions artefacts of a public repository still need a GitHub login to download, so:

| Option | Cost | Notes |
| --- | --- | --- |
| **The owner downloads the zips and forwards them** (chat, a cloud link) | none | Works for anyone. A zip is about 11 MB (Linux, measured) to perhaps 25 MB (the universal macOS bundle). It carries `INSTALL.md` and `gr1-friends.md` |
| **Friends with a GitHub account download the artefacts themselves** | none | From the `release.yml` run's page; artefacts expire after 14 days |
| **A release with the zips as assets** | needs `contents: write` in `release.yml` (it is read-only now) | Downloadable without a login, and assets do not expire. Not built; the next step if forwarding gets tedious. It would also be the place for debug symbols (PDB, dSYM) |

## DAW coverage

From report 04 §2.7 (2026; "anecdotal" rows are reviews and forums). VST3 and CLAP on Windows and macOS cover everything except Logic, GarageBand and Pro Tools (AAX is out).

| DAW | CLAP | VST3 | AU | Install | Source |
| --- | --- | --- | --- | --- | --- |
| Ableton Live 12.4 (May 2026) | no | yes | Mac | VST3 | Antares 2026-07-27; no CLAP in Ableton's release notes |
| Logic Pro, GarageBand | no | no | **only** | **none yet** (AU deferred; the standalone app would cover it) | Antares |
| FL Studio | **yes, since 2024.1** | yes | Mac | VST3 or CLAP | Image-Line changelog (Antares' table is wrong here) |
| Cubase 15 (Nov 2025) | no | yes | | VST3 | launch coverage |
| Studio One Pro 7, Fender Studio Pro 8 | yes | yes | Mac | VST3 or CLAP | reviews (anecdotal); CLAP host list |
| Bitwig, REAPER | yes | yes | Mac | CLAP or VST3 | CLAP host list |
| Pro Tools | no | no | no | none (AAX only) | |

There is no solid 2025 to 2026 share data (the only survey found, Production Expert 2025, is a professional crowd that puts Pro Tools first). **The friend survey decides** which rows matter: ask each friend for their OS and DAW (5 minutes, the first open check in the research summary), then build only the legs they need.

## Signing and the public release (S1)

R1 ships unsigned builds: the macOS bundles are ad-hoc signed (the signature `arm64` needs, not an identity), and Windows has no signature. A public S1 release needs both:

| | What | Cost |
| --- | --- | --- |
| macOS | Developer ID Application certificate, hardened runtime, a signed `.pkg` or `.dmg`, `notarytool submit`, staple; all runnable on `macos-latest` with the certificate and an App Store Connect key as secrets | $99 a year |
| Windows | Azure Artifact Signing (GA January 2026; $9.99 a month; limited to organisations or individuals in the USA and Canada, or organisations in the EU or UK), else an OV certificate from a reseller, about EUR 110 to 170 a year, else SignPath Foundation (OSI-licensed projects only, manual approval per release) | see left |
| Both | A third-party notices file for the crate dependencies (the repository's `THIRD_PARTY_NOTICES.md` covers ported code and the vendored crates only; the zips ship it with `LICENSE` as the best current notice) | a `cargo-about` run |

The bundles' `Info.plist` is nice-plug-xtask's template (`CFBundleIdentifier` `com.nice-plug.enth-plugin` for both bundles, version `1.0.0`); public builds need their own identifier and version, which the bundler's TODO leaves to a post-processing step.

AU for Logic is deferred past S1: if needed, wrap the release `.clap` with upstream clap-wrapper 0.16.0 or newer through CMake in the macOS job (MIT; gated on `auval`), 1 to 2 days plus a Mac tester.

## The standalone app: an owner decision

**Status: pending. Not built, not started.** Default until the owner says yes: plugin bundles only.

The summary's plan is `enth-standalone` (an L8 crate; `standalone` is already an L8 name in `AGENTS.md`, so no layer change) on nice-plug's `standalone` feature. It is the cheapest way to reach Logic users and friends with no DAW, and a signed-or-not `.app` or `.exe` sidesteps the plugin-quarantine problem (macOS "Open Anyway" works for apps; Windows shows SmartScreen's "Run anyway").

Why it needs the owner: the feature pulls `cpal` (Apache-2.0), `midir` (MIT), `jack` (MIT, loaded with `dlopen`) and on Linux `alsa`, about 34 more crates, and `AGENTS.md` says to ask before adding a dependency that pulls in a system library. Report 04 measured that `cargo deny` licences and bans pass with the feature and that a trial crate compiles on Linux, Windows and macOS arm64; nothing was run.

Limits to accept: the wrapper saves nothing between runs (the shell would have to save the patch, settings and game progress), has no device screen (48 kHz and a 512-sample period by default, set by flags), takes MIDI input only with `--midi-input`, and needs `#![windows_subsystem = "windows"]` so no console window opens. R1's round needs no MIDI: Target and Mine use the editor's audition notes. ASIO is out (it needs Steinberg's SDK). If approved: a small crate, one more release leg step per OS (bundle the binary as `.app` on macOS), and the install sheet gains a paragraph.

## The other workflows

| Workflow | Runs | Cost shape |
| --- | --- | --- |
| `ci` | push to main and pull requests, unless only Markdown, `docs/` or `_local/` changed; weekly (Monday) for advisories | ~32 Linux min per code push, plus the guard |
| `engine-golden`, `golden`, `budget` | push to main and pull requests that touch their paths; `golden` has a weekly macOS leg | Windows legs ~12 min per code push |
| `nightly` | 02:17 UTC every day, and by hand | RTSan, 10,000-case proptest, the leak gates: Linux only |
| `release` | a `v*` tag or by hand, **never on a push** | the table above |

### Why `nightly` shows 0 runs (checked 2026-10-20, read-only)

The YAML is valid (GitHub parsed it: the workflow is listed as `nightly`, active, with its cron `17 2 * * *`), Actions is enabled with all actions allowed, and runs on push succeed, so it is not a syntax, permission or billing problem. It reached `main` on 2026-10-19 at 08:04 UTC. Its first slot, 02:17 UTC on 2026-10-20, had not fired by 03:46 UTC. GitHub documents that scheduled runs can be delayed at high load and that a queued scheduled job may be dropped. This repository's only other scheduled run is the weekly `ci` (cron `23 5 * * 1`, Monday), which fired **5 h 33 min late**, at 10:56 UTC. So hours of delay is what scheduled runs have done in this repository so far, and the file needs no change. Check again with `gh run list --workflow nightly.yml`; if the 02:17 UTC slot of 2026-10-21 passes with still no run, start it once by hand (`gh workflow run nightly.yml`) and look again at the repository's Actions settings.

## Open items

| Item | Who |
| --- | --- |
| Friend survey: OS and DAW for each friend | owner |
| The owner's GitHub plan and minutes | owner |
| First run of `release.yml` (the table in "What to check on the first run"), then replace the cost estimates with measured job times | coordinator |
| The standalone app: yes or no | owner |
| A dependency-notices file for public builds; Developer ID and Windows signing | S1 |
| The x86-64 slice of the macOS bundle, Windows on ARM, and macOS older than 11: not built or not tested | friends / later |
| The Linux build needs glibc 2.39 (Ubuntu 24.04 or newer); an `ubuntu-22.04` runner would give 2.35 | when a Linux friend needs it |
| Debug symbols (PDB, dSYM) kept per release | with the draft-release follow-up |
| A crash file the editor offers on next load (report 04 §2.10) | a plugin lane |
