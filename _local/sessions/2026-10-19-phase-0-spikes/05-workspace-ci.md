# 05 — Workspace and CI

- Date: 2026-10-19
- Sub-researcher: 05 (workspace and CI)
- Questions answered: 1–9 from the brief. All are answered; some only partly (see Gaps). The research was cut short by the owner's reboot, so nothing below was run on Rust 1.99.1 or on GitHub's runners.
- Method: web and API lookups (crates.io API, `gh api`, static.rust-lang.org, official docs), plus local checks in scratch workspaces under `/tmp/enth-research/05/` with the owner's rustc/cargo **1.95.0**, clippy 1.95.0 and cargo-deny **0.19.4**, using `CARGO_BUILD_JOBS=2`. Every drafted Task 2 file below was built and run there: fmt, clippy `-D warnings`, test, cargo-deny, check-layers and wasm-check all pass, and check-layers fails on a deliberate upward dependency.
- Labels: **measured** = I ran it here; **documented** = official docs, changelogs or repo files, fetched today; **anecdotal** = what other projects do or say.

## Answer in brief

- **Pin `channel = "1.99.1"`** in `rust-toolchain.toml` with `profile = "minimal"`. 1.99.1 is the current stable (released 2026-10-15). The owner's 1.95.0 is four releases behind and lacks `CARGO_BUILD_WARNINGS` (cargo 1.97). Never pin 1.99.0: it has an x86_64 FFI `bool` miscompilation that 1.99.1 fixes, and CLAP's C API returns `bool` everywhere. Edition 2024 is still the newest edition. A virtual workspace still needs an explicit `resolver = "3"` (measured: without it, cargo 1.95 falls back to resolver 1 and warns).
- **Lints:** use `[workspace.lints]` with clippy `all` + `pedantic` at priority −1. Allow the DSP-noisy cast and naming lints, and cherry-pick restriction lints, mainly `allow_attributes_without_reason` so every agent `#[allow]` must say why. This follows egui and rerun. A member can't override inherited lints in `Cargo.toml` (measured: hard error), so FFI crates opt out in source with `#![allow(unsafe_code, reason = "…")]`.
- **New finding:** `cargo xtask wasm-check` does **not** enforce "core crates do no I/O, spawn no threads, read no clocks". `wasm32-unknown-unknown` compiles `Instant::now`, `fs::read`, `thread::spawn` and `env::var` without complaint (measured). Clippy's `disallowed-methods`/`disallowed-types` in a root `clippy.toml` does catch them (measured). Shells and tooling opt out at the crate root, or with a directory-level `clippy.toml`.
- **cargo-deny:** current is 0.20.2 (2026-07-09); the owner has 0.19.4. egui's fonts are now SPDX **`Ubuntu-font-1.0`**, not `LicenseRef-UFL-1.0` (`epaint_default_fonts` since 0.31.0, 2025-02-04). Scope `OFL-1.1`/`Ubuntu-font-1.0` as exceptions for `epaint_default_fonts`, as clack's deny.toml does (measured passing).
- **Layering:** cargo-deny `deny` + `wrappers` *does* apply to workspace path crates (measured). But it needs an allow-list per crate and can't spot unlayered members. Keep the custom `cargo xtask check-layers`. A 191-line prototype is drafted below: it reads `cargo metadata --no-deps` and runs in 0.1 s warm (measured). No adequate existing tool exists: `cargo-depgraph-check` has 2 stars, and `cargo-archtest` is AGPL.
- **xtask over just:** xtask holds the logic (it needs `cargo metadata` and TOML), uses one language, and needs nothing installed. Use a cargo alias only to invoke it. Skip a `justfile`, which would be a second source of truth.
- **CI:** GitHub Actions, standard runners free for public repos (documented), `ubuntu-24.04` pinned (`ubuntu-latest` is 24.04 today, 26.04 already exists). Install the toolchain with `rustup toolchain install` (reads the toolchain file). Cache with Swatinem/rust-cache v2.9.2, `save-if` main only. Install cargo-deny 0.20.2 via taiki-e/install-action, with advisories as an allowed-to-fail matrix leg. Pin actions by SHA. No sccache yet: it can't cache cdylib or proc-macro crates (documented).
- **Codeberg is not a good primary CI** for this build. Hosted Forgejo Actions caps at 4 CPU / 8 GB (filesystem writes count as RAM) / 10 min per job. Hosted Woodpecker needs an approval; nice-plug uses it.
- **Golden tests:** hash the f32 sample bits per platform and toolchain (SHA-256), and compare a feature summary with a tolerance across platforms. std's `sin` etc. are documented as non-deterministic across platforms *and Rust versions*, so a toolchain bump is a golden review. Keep fixtures as text in a dedicated `enth-golden` crate, with no WAVs in git. `cargo xtask golden --update` should set an env var and refuse under `CI`, like insta.
- **Plan check, high-value items:** (1) Task 1's `libjack-dev` would **remove** `libjack-jackd2-0` (amd64 + i386) and `libasound2-plugins:i386` on the owner's machine (measured with `apt-get -s`); use `libjack-jackd2-dev`, as nice-plug's CI does. (2) Replace `LicenseRef-UFL-1.0` with `Ubuntu-font-1.0` in ADR-0004, AGENTS.md and phase-0. (3) Commit `_local/sessions/`: ADR-0006 cites it as evidence, and it's 248 KB. (4) Add `default-members` so plain `cargo test` skips the heavy spikes.

## Findings

### 1. Toolchain, edition, resolver

| Claim | Evidence | Label |
| --- | --- | --- |
| Current stable is **1.99.1**, released **2026-10-15**; 1.99.0 was 2026-10-01 | `channel-rust-stable.toml` (manifest date 2026-10-15, rustc `1.99.1 (58cf8f9c3 2026-10-13)`); [RELEASES.md](https://raw.githubusercontent.com/rust-lang/rust/stable/RELEASES.md); [blog](https://blog.rust-lang.org/2026/10/15/Rust-1.99.1/) | documented |
| Beta is 1.100.0-beta.7 (2026-10-17). 1.100.0 is therefore due about 2026-11-12, six weeks after 1.99.0 (an inference from the cadence) | `channel-rust-beta.toml` | documented / inference |
| Releases since the owner's 1.95.0: 1.96.0 (2026-05-28), 1.97.0 (2026-07-09), 1.98.0 (2026-08-20), 1.99.0. 1.95.0 itself was released 2026-04-16; the brief's 2026-04-14 is the rustc commit date that `rustc --version` prints | RELEASES.md; local `rustc 1.95.0 (59807616e 2026-04-14)` | documented / measured |
| 1.99.1 fixes "miscompilation of FFI `bool` return values on x86_64" ([#163911](https://github.com/rust-lang/rust/issues/163911), opened 2026-10-07, labelled `regression-from-stable-to-stable`, `P-critical`). A Rust `extern "C" fn -> bool` could return 16 instead of 0/1. The blog says only "regressions from prior releases"; I didn't pin down the first affected version, but 1.99.0 is affected | issue + blog | documented |
| Cargo 1.97: `build.warnings` / `CARGO_BUILD_WARNINGS=deny` makes warnings fail the build **without invalidating the build cache** (unlike `RUSTFLAGS=-Dwarnings`) | [Rust 1.97.0 blog](https://blog.rust-lang.org/2026/07/09/Rust-1.97.0/); [cargo config docs](https://doc.rust-lang.org/cargo/reference/config.html) | documented |
| Rust 1.97 also shows linker output as a `linker_messages` warning, which the `warnings` group doesn't control. Silence it with `linker_messages = "allow"` if it gets noisy | 1.97 blog | documented |
| Cargo 1.99 disables incremental compilation by default when `CI` is set. The `CARGO_INCREMENTAL=0` that rust-cache sets becomes redundant but harmless | RELEASES.md 1.99.0 compatibility notes | documented |
| 1.98: "The UNSAFE_CODE lint is now consistently emitted for all unsafe attributes", so `#[unsafe(no_mangle)]` trips `unsafe_code = "deny"` | RELEASES.md 1.98.0 | documented |
| 1.99 adds a built-in `debug` profile (preparing to change `dev` defaults) and lets edition-2024 members override `default-features` on inherited workspace deps | RELEASES.md 1.99.0 Cargo | documented |
| 1.96 fixed two Cargo CVEs (CVE-2026-5222 low; CVE-2026-5223 medium, third-party registries only) | [blog 2026-05-25](https://blog.rust-lang.org/2026/05/25/cve-2026-5222/) | documented |
| Edition 2024 stabilized in 1.85.0 (2025-02-20) and is still the newest. I found no announced 2027 edition, only forum debate | RELEASES.md; [internals thread](https://internals.rust-lang.org/t/we-have-only-two-years-to-prepare-the-rust-2027-edition/22494) | documented / anecdotal |
| Resolver versions are 1, 2 and 3. `"3"` is the edition-2024 default (needs Rust 1.84+) and changes `incompatible-rust-versions` to `fallback` | [resolver docs](https://doc.rust-lang.org/cargo/reference/resolver.html) | documented |
| A virtual workspace without `resolver` falls back to `resolver = "1"` and warns, even when every member is edition 2024 | scratch workspace, cargo 1.95.0 | measured |
| rustup ≥ 1.28.1 auto-installs the toolchain named in `rust-toolchain.toml`, components and targets included. In CI, `rustup toolchain install` with no arguments installs it explicitly | [rustup CHANGELOG](https://github.com/rust-lang/rustup/blob/main/CHANGELOG.md) 1.28.0/1.28.1 | documented |
| GitHub's `ubuntu-24.04` image 20261012 ships Rust 1.99.0 and rustup 1.29.1, so a 1.99.1 pin costs one toolchain download per job | [runner-images Ubuntu2404-Readme](https://github.com/actions/runner-images/blob/main/images/ubuntu/Ubuntu2404-Readme.md) | documented |
| `wasm32-unknown-unknown` and `wasm32-wasip1` are tier 2 with std | [platform support](https://doc.rust-lang.org/rustc/platform-support.html) | documented |
| Pinning precedent: egui pins `1.95.0` today (three releases behind); nice-plug declares `rust-version = "1.88"` | egui `rust-toolchain`, nice-plug `Cargo.toml` (fetched 2026-10-19) | anecdotal |

Why pin an exact version rather than `stable`: clippy's `pedantic` gains lints every release (1.99 added `assert_is_empty` and `manual_bit_width`, per the [clippy CHANGELOG](https://github.com/rust-lang/rust-clippy/blob/master/CHANGELOG.md)). Under `-D warnings`, an unpinned toolchain turns CI red without any code change. Golden hashes can also shift between Rust versions (§6). A bump should be a deliberate commit.

`rust-version`: not needed. With `publish = false` and a pinned toolchain, resolver 3's fallback already uses the running rustc's version (documented). Adding `rust-version` would make a second place to bump.

### 2. `[workspace.lints]` and clippy sets

| Claim | Evidence | Label |
| --- | --- | --- |
| A group needs a lower `priority` than the individual lints that override it; `[lints] workspace = true` inherits | [manifest `[lints]`](https://doc.rust-lang.org/cargo/reference/manifest.html), [workspaces](https://doc.rust-lang.org/cargo/reference/workspaces.html) | documented |
| A member **cannot** combine `workspace = true` with overrides. Cargo errors: "cannot override `workspace.lints` in `lints`" | scratch, cargo 1.95.0 | measured |
| So `unsafe_code` must stay `deny`, not `forbid`. FFI crates write `#![allow(unsafe_code, reason = "…")]` (works, measured) | scratch | measured |
| egui (fetched 2026-10-19): `all` + `cargo` + `pedantic` at −1. It allows `cast_*`, `float_cmp`, `many_single_char_names`, `similar_names`, `too_many_lines`, `must_use_candidate`, `missing_panics_doc`, and enables about 70 restriction/nursery lints (`allow_attributes`, `dbg_macro`, `todo`, `unwrap_used`, `undocumented_unsafe_blocks`, `print_stdout`, `large_stack_frames`, …) | [egui Cargo.toml](https://github.com/emilk/egui/blob/main/Cargo.toml) | anecdotal |
| rerun: the same `all`/`cargo`/`pedantic` pattern plus `unknown_lints = "deny"`; xilem: cherry-picked (`allow_attributes_without_reason`, `cast_possible_truncation`, `missing_docs`, `unreachable_pub`…); bevy: cherry-picked pedantic lints | repos fetched 2026-10-19 | anecdotal |
| Audio/plugin projects: **clack** (CLAP bindings) denies `missing_docs`, `undocumented_unsafe_blocks` and all `cast_*`, with clippy.toml `msrv = 1.85.0` and `check-private-items`. **Symphonia** sets `unwrap_used = "warn"`. nice-plug, nih-plug, fundsp, cpal, kira, rodio, firewheel and web-audio-api-rs have **no** `[workspace.lints]`; nice-plug and cpal rely on `cargo clippy … -- -D warnings` in CI | repo files fetched 2026-10-19 | anecdotal |
| `module_name_repetitions` moved from pedantic to restriction (no need to allow it); `float_cmp` is pedantic | clippy CHANGELOG | documented |
| clippy.toml is found from `CLIPPY_CONF_DIR`, else `CARGO_MANIFEST_DIR`, then by walking up; the first file found wins, with no merging | [clippy configuration](https://doc.rust-lang.org/clippy/configuration.html) | documented |
| A root `clippy.toml` applies to `crates/*` members. A crate-root `#![allow(clippy::disallowed_methods, reason = …)]` opts out. A `spikes/clippy.toml` (empty) overrides the root for every crate under `spikes/` | scratch, clippy 1.95.0 | measured |
| `disallowed-methods` entries for `Instant::now`, `SystemTime::now`, `thread::spawn`, `thread::scope`, `env::var(_os)`, `fs::read(_to_string)`, `fs::write`, `File::open/create`, `OpenOptions::open` and `Command::new`, plus `disallowed-types` for `Mutex`, `RwLock`, `HashMap` and `HashSet`, are all valid paths, and each fires | scratch, clippy 1.95.0 | measured |

Recommended set (drafted in full below): `all` + `pedantic`, minus 11 allows chosen for DSP code and taste, plus 14 restriction lints. Beyond egui's practice, there are three agent-specific reasons:

- `allow_attributes_without_reason` makes every suppression carry a written reason, which reviewers and later agents can audit.
- `print_stdout`/`print_stderr` make I/O in core crates visible.
- `large_stack_arrays` catches audio buffers placed on the stack.

I did **not** enable `allow_attributes` (which forces `#[expect]`). A crate-wide opt-out like `#![allow(clippy::disallowed_methods)]` would then become `#[expect]`, which errors in any crate that happens not to need it. No-panic lints on the audio path (`unwrap_used`, `indexing_slicing`, `panic`) belong as module-level `#![deny(...)]` in `enth-engine`'s process modules, not workspace-wide: too many false hits in tests and offline code. That last point is judgment.

### 3. cargo-deny and layer enforcement

| Claim | Evidence | Label |
| --- | --- | --- |
| Latest **cargo-deny 0.20.2** (2026-07-09; MSRV 1.88). 0.20.0 refactored the CLI and removed deprecated options; it added `bans.std-replacements`. Unreleased: `advisories.ignore-expiry`, config search in `.config/` | crates.io API; [CHANGELOG](https://github.com/EmbarkStudios/cargo-deny/blob/main/CHANGELOG.md) | documented |
| The owner has 0.19.4. In 0.19.x, `unsound` defaults to `workspace` and `unmaintained` to `all`; advisory staleness defaults to 90 days; it shells out to `git` instead of `gix` | CHANGELOG; [advisories cfg](https://embarkstudios.github.io/cargo-deny/checks/advisories/cfg.html) | documented |
| `[licenses]` denies everything not in `allow`. `exceptions` scope extra licences to one crate. `unused-allowed-license` defaults to `warn`. `include-dev` defaults to `false` | [licenses cfg](https://embarkstudios.github.io/cargo-deny/checks/licenses/cfg.html) | documented |
| **`epaint_default_fonts` 0.37.0 (2026-10-15) declares `(MIT OR Apache-2.0) AND OFL-1.1 AND Ubuntu-font-1.0`**. It switched from `LicenseRef-UFL-1.0` at 0.31.0 (2025-02-04). `epaint` itself carried the font licence in 0.20–0.28 | crates.io API version history | documented |
| nice-plug-egui 0.6.0 (2026-10-18) depends on egui 0.37.0 (`default-features = false`), so the current id applies. Whether `epaint_default_fonts` is actually pulled in with those features is unverified (Gaps) | nice-plug repo | documented |
| egui's deny.toml allows `Ubuntu-font-1.0` globally. **clack's** deny.toml scopes `{ allow = ["OFL-1.1", "Ubuntu-font-1.0"], name = "epaint_default_fonts" }`. clack also needs exceptions for `option-ext` (MPL-2.0) and `error-code`/`clipboard-win` (BSL-1.0): GUI-stack crates enth will likely meet too | repo files fetched 2026-10-19 | anecdotal |
| The drafted deny.toml (allow list + font exceptions + `unused-license-exception = "allow"`) passes `licenses` with `epaint_default_fonts = "=0.37.0"` in the graph. It also passes without it, warning `license-exception-not-encountered` unless that option is set to `allow`. `advisories`, `bans` and `sources` pass too | scratch, cargo-deny 0.19.4 | measured |
| cargo-deny-action v2.1.1 (2026-07-13) runs cargo-deny 0.20.2 in a `rust:1.85.0-alpine` Docker image with default arguments `--all-features`. Its README recommends a matrix of `advisories` vs `bans licenses sources`, with `continue-on-error` for advisories | [action repo](https://github.com/EmbarkStudios/cargo-deny-action) | documented |
| **`bans.deny` with `wrappers` applies to workspace path crates.** Banning `enth-b` with `wrappers = ["enth-c"]` failed when a third member depended on it (`error[banned]`, plus a `warning[unmatched-wrapper]`) | scratch, cargo-deny 0.19.4 | measured |
| Existing layer tools: **cargo-depgraph-check** 0.1.0 (2026-03-11, MIT, 2 GitHub stars, ~1.2k recent downloads) checks a per-crate allow-list of workspace dependencies, with a `strict` mode for unlisted members. **cargo-archtest-cli** is AGPL-3.0 (excluded) and checks modules, not crates. **guppy** 0.19.1 (2026-09-24, MIT/Apache) is a graph library for writing custom checks | crates.io API; [cargo-depgraph-check README](https://github.com/1898andCo/cargo-depgraph-check) | documented |
| Prototype `cargo xtask check-layers` reads `cargo metadata --no-deps` and catches same-layer edges (`enth-c (L1) -> enth-b (L1) [normal]`), upward dev-dependencies (`enth-a (L0) -> enth-c (L2) [dev]`), unlayered members and deliberate upward deps in the six-crate skeleton. It ran in 0.09–0.12 s warm. A cold build took 35 s with `CARGO_BUILD_JOBS=2`, max single-process RSS 262 MB, 20 packages in its dependency tree (serde, serde_json, toml) | scratch | measured |

Verdict: **custom xtask check, as planned.** cargo-deny could emulate layering, but only with one `deny` entry per crate listing every allowed dependent, kept in sync with `layers.toml` by hand. It also can't report "member is neither layered nor exempt". cargo-depgraph-check uses allow-lists, not layers, and is too young to depend on. A useful cargo-deny addition later: `wrappers` to keep heavy crates (e.g. `nice-plug`, `egui`) confined to their shell crate.

### 4. xtask vs just vs cargo aliases

| Claim | Evidence | Label |
| --- | --- | --- |
| The xtask pattern: a workspace binary `xtask`, a `.cargo/config.toml` alias `xtask = "run --package xtask --"`, subcommands of one binary, compile time kept low. No tool other than cargo/rustc is needed | [matklad/cargo-xtask](https://github.com/matklad/cargo-xtask) (last push 2025-10-28) | documented |
| Users: rust-analyzer, egui, wgpu, nih-plug and nice-plug (`crates/xtask` wrapping `nice-plug-xtask`, the plugin bundler); bevy uses `tools/` | repo listings 2026-10-19 | anecdotal |
| `just` 1.59.0 (2026-10-15) is active. `cargo-make` last released 0.37.24 on 2025-01-18 | GitHub releases, crates.io | documented |
| Cargo aliases can't chain commands or hold logic | cargo docs (alias = argument expansion) | documented |

Verdict: **xtask** for every command with logic (check-layers, wasm-check, bundle, install, validate, golden, bench). It is Rust, which agents write well. It can use `cargo metadata` and TOML, works on Windows later, and nice-plug's bundler already plugs into it. Keep its dependencies to serde + serde_json + toml, with hand-parsed arguments (the prototype has no clap). The owner has `just` installed, but a justfile would duplicate xtask; skip it. If one is ever wanted, make it one-line forwards to `cargo xtask`. That's judgment.

### 5. GitHub Actions for Rust in 2026; Codeberg

| Claim | Evidence | Label |
| --- | --- | --- |
| Actions on standard GitHub-hosted runners are **free for public repositories**. Cache allowance is 10 GB per repo, then $0.07/GB-month if raised. Larger runners are always billed | [GitHub Actions billing](https://docs.github.com/en/billing/concepts/product-billing/github-actions) (fetched 2026-10-19) | documented |
| Standard Linux runner for public repos: 4 vCPU, 16 GB RAM, 14 GB SSD | [GitHub-hosted runners reference](https://docs.github.com/en/actions/reference/runners/github-hosted-runners) | documented |
| `ubuntu-latest` = `ubuntu-24.04`; `ubuntu-26.04` and `ubuntu-slim` exist | [runner-images README](https://github.com/actions/runner-images) | documented |
| **Swatinem/rust-cache v2.9.2** (2026-08-06) caches `~/.cargo` and the target dir's *dependencies* (not workspace crates), keyed on rustc version, lockfiles, toolchain file and `.cargo/config.toml`. Inputs include `save-if`, `shared-key` and `cache-provider` (github or warpbuild) | [README](https://github.com/Swatinem/rust-cache) | documented |
| **actions-rust-lang/setup-rust-toolchain v2.0.0** (2026-09-07) reads `rust-toolchain.toml`, wraps rust-cache, and now sets `CARGO_BUILD_WARNINGS=deny` by default instead of `RUSTFLAGS`. **dtolnay/rust-toolchain** has no releases (branch-based) but is active (1.99.1 branch added 2026-10-16) | READMEs, CHANGELOG, commits | documented |
| **sccache v0.18.0** (2026-09-14), **mozilla-actions/sccache-action v0.0.11** (2026-07-29). sccache can't cache crates that invoke the linker (`bin`, `dylib`, `cdylib`, `proc-macro`) or incremental builds | [sccache README](https://github.com/mozilla/sccache) | documented |
| **taiki-e/install-action v2.87.29** (2026-10-16) installs prebuilt `cargo-deny`, `cargo-nextest`, `cargo-insta`, `wasmtime`, `just`, `typos`, `release-plz` and `sccache`, pinnable as `tool@version` | [TOOLS.md](https://github.com/taiki-e/install-action/blob/main/TOOLS.md) | documented |
| apt caching: **awalsh128/cache-apt-pkgs-action v1.6.3** (2026-06-30). cpal's CI uses it `@latest`, together with dtolnay toolchain + rust-cache + `clippy -- -D warnings` | release; [cpal quality.yml](https://github.com/RustAudio/cpal/blob/master/.github/workflows/quality.yml) | anecdotal |
| nice-plug's CI (Codeberg Woodpecker, `rust:1-bookworm`) installs `pkg-config libasound2-dev libgl-dev libjack-jackd2-dev libx11-xcb-dev libxcb1-dev libxcb-dri2-0-dev libxcb-icccm4-dev libxcursor-dev libxkbcommon-dev libxcb-shape0-dev libxcb-xfixes0-dev`. It runs clippy `-D warnings` but **excludes the egui/iced crates** from clippy and doc | [.woodpecker/test.yaml](https://codeberg.org/RustAudio/nice-plug/src/branch/main/.woodpecker/test.yaml) | anecdotal |
| **clap-validator 0.4.1** (release 2026-07-22, tag → `152b982`, MIT, 112 stars) is not on crates.io. Linux asset: `clap-validator-0.4.1-127-g152b982-ubuntu-22.04.zip`; per its build workflow this is an artifact zip containing a `.tar.gz` with one binary. CLI: `validate <plugin.clap> --only-failed`, `--include`/`--exclude` regex, `clap-validator.toml`, `--in-process`, plus an experimental `fuzz`. Its README and CHANGELOG mention no GUI tests. That suggests it runs headless (an inference, not verified) | [repo](https://github.com/free-audio/clap-validator), [build.yml](https://github.com/free-audio/clap-validator/blob/master/.github/workflows/build.yml) | documented / inference |
| Security context: the Rust blog (2026-09-21) warned that caching `target/` can leak secrets (Miri stores env vars there). It advises keeping secrets out of jobs that write public caches. The blog (2026-09-17) also reported targeted attacks on Rust maintainers and a brief `arrayref` compromise. GitHub can enforce SHA-pinned actions by policy (2025-08-15) | [Miri post](https://blog.rust-lang.org/2026/09/21/github-actions-leaking-secrets-when-miri-output-is-cached/), [attacks post](https://blog.rust-lang.org/2026/09/17/targeted-attacks/), [GitHub changelog](https://github.blog/changelog/2025-08-15-github-actions-policy-now-supports-blocking-and-sha-pinning-actions/) | documented |
| actions/checkout v7.0.1 (2026-07-20). The v4–v7 breaking change only concerns `pull_request_target` (`allow-unsafe-pr-checkout`), which we won't use | release notes | documented |
| **Codeberg:** hosted Forgejo Actions run "in a limited fashion"; the docs say to use Woodpecker if you need Codeberg-hosted CI. Runner labels: `codeberg-tiny` 1 CPU/2 GB/2 min, `codeberg-small` 2/4 GB/5 min, `codeberg-medium` **4/8 GB/10 min**, and RAM "includes writes to the filesystem". Hosted Woodpecker needs an access request and builds linux/amd64 only. Self-hosted runners are possible | [docs.codeberg.org/ci](https://docs.codeberg.org/ci/), [codeberg.org/actions/meta](https://codeberg.org/actions/meta) (updated 2026-10-06) | documented |

Verdict: GitHub Actions on `ubuntu-24.04`, with two jobs. `check` does fmt, clippy, test, check-layers and wasm-check in one job so they share one toolchain install and cache. `deny` uses a two-leg matrix and compiles nothing. Use rust-cache, not sccache: rust-cache covers dependencies, while sccache can't cache the plugin cdylib or proc-macros, and caching workspace crates is generally ineffective per rust-cache's own docs. Start with plain apt-get (22 packages); add the apt-cache action only if that step is measured over about a minute. A target directory of egui/wgpu/vizia dependencies alone would likely blow Codeberg's 8 GB including-writes limit (inference). Use Codeberg at most as a mirror; a self-hosted runner on the owner's RAM-tight laptop is a poor idea.

### 6. Golden and snapshot tests

| Claim | Evidence | Label |
| --- | --- | --- |
| **insta 1.49.0** (2026-10-03); the owner has cargo-insta 1.47.2. Default `INSTA_UPDATE=auto` resolves to `no` in CI. Since 1.48, `CI=true` makes `cargo insta test` behave like `--check`. `--unreferenced=auto` deletes locally and rejects in CI. Binary snapshots have been experimental since 1.41. 1.47 added a `Comparator` trait for custom comparison (could carry tolerances; I couldn't read its docs page) | [insta CHANGELOG](https://github.com/mitsuhiko/insta/blob/master/CHANGELOG.md), [insta.rs advanced](https://insta.rs/docs/advanced/) | documented |
| Update conventions elsewhere: `UPDATE_GOLDENFILES=1` (goldenfile 1.11.0, 2026-02-25), `GOLDIE_UPDATE=1` (goldie 0.7.0) | docs.rs | documented |
| std's `f32::sin` docs say its precision is non-deterministic, varying by platform and Rust version | [std f32](https://doc.rust-lang.org/std/primitive.f32.html) | documented |
| egui keeps binary snapshot images (`*.png`) in Git LFS | [egui .gitattributes](https://github.com/emilk/egui/blob/main/.gitattributes) | anecdotal |
| `sha2` 0.11.1 (2026-10-12, MIT/Apache, pure Rust). `blake3` 1.8.7 is CC0/Apache but builds C/asm by default, so prefer `sha2` | crates.io API | documented |

Recommended pattern (judgment, consistent with design.md §5):

- **What to hash:** SHA-256 over the rendered f32 samples' little-endian bits, interleaved, prefixed by (sample rate, channels, frames). Never hash the WAV file (headers). Store the hash **per platform key** (`x86_64-linux`, `x86_64-windows`, `aarch64-macos`, `wasm32-wasip1`). On a platform with a recorded hash: exact match. Elsewhere: summary only.
- **Tolerance:** compare a small feature summary (RMS per 10 ms frame and spectral centroid per frame, as Task 5 already defines) within recorded tolerances. Task 5 sets the numbers.
- **Where:** make `tests/golden/` a workspace crate, package `enth-golden` (tooling, exempt in `layers.toml`). Golden checks need `enth-render` *and* `enth-analysis`, which are both L3, so they can't live in either crate's tests: check-layers rejects a same-layer dev-dependency (measured). Layout: `tests/golden/{Cargo.toml, tests/golden.rs, cases/<name>.patch.json, expected/<name>.toml}`. All text, so PR diffs are reviewable. **No WAVs in git.** `cargo xtask golden --wav <name>` writes them to `target/golden/` for listening. If reference audio must ever be versioned (e.g. Task 6 listening pairs), use LFS or release assets.
- **Update:** `cargo xtask golden --update` runs `ENTH_GOLDEN_UPDATE=1 cargo test -p enth-golden`. The harness rewrites only the current platform's hash and the summary. It **refuses when `CI` is set**, mirroring insta. As AGENTS.md already says, the commit message says why. A toolchain bump runs `cargo xtask golden` and either passes or is a deliberate update.
- **insta** stays for CLI JSON (`assert_json_snapshot!`), as AGENTS.md already says.

### 7. Folder topology (architecture.md) against current practice

| Item | Evidence | Verdict |
| --- | --- | --- |
| Virtual root manifest, flat `crates/*`, `version = "0.0.0"` for unpublished crates, xtask | matklad, [Large Rust Workspaces](https://matklad.github.io/2021/08/22/large-rust-workspaces.html); rust-analyzer, egui | Keep (anecdotal practice) |
| `crates/dsp` → package `enth-dsp` (stripped prefix) | matklad: "Don't succumb to the temptation to strip common prefix from folder names." egui (`crates/egui`, `crates/epaint`), bevy (`crates/bevy_ecs`) and nice-plug (`crates/nice-plug-core`) all match directory to package | **Change to `crates/enth-dsp/`** (cheap now): `-p enth-dsp` maps 1:1 to a path, so a grep for the name finds the directory |
| `members = ["crates/*", "xtask"]`, adding globs later | A glob that matches nothing is an **error**, and so is a non-crate *directory* under a glob; plain files (e.g. `README.md`) are fine (all measured) | Keep the phase-0 comment; never park non-crate folders under `crates/`, `spikes/` or `tools/` |
| `spikes/*` as members | In a virtual workspace, plain `cargo test`/`clippy` cover all members. `default-members = ["crates/*", "xtask"]` (globs work) makes them skip spikes; `--workspace` brings them back (measured) | Keep as members, **add `default-members`**. The heavy GUI spikes otherwise land on every agent's `cargo test` on a laptop with about 2.5 GiB free |
| `tests/golden/` as plain data | See §6 | Make it the `enth-golden` crate |
| `tools/test-host` | Fine; exempt | Keep |
| `_local/` ignored by git | ADR-0006, research.md and roadmap.md cite `_local/sessions/…` as evidence; sessions total about 248 KB (measured with `du`) | **Commit the sessions** (see Plan check) |
| Commit `Cargo.lock` | It's an application/plugin workspace; CI runs `--locked` | Add to Task 2 |

Shared feature unification across spikes: cargo's per-package feature unification (`resolver.feature-unification`) is still unstable (documented in the [unstable book](https://doc.rust-lang.org/cargo/reference/unstable.html)). `cargo build --workspace` therefore unifies egui and vizia features across spikes. That's acceptable for throwaway code. If it bites, move spikes into their own workspace (`exclude = ["spikes"]`).

### 8. Release and versioning tooling (later)

- **release-plz** 0.3.172 / action v0.5.142 (2026-10-12), actively released (40 releases in the last page of tags); the owner has 0.3.157. It handles version bumps, changelogs, tags and GitHub/Gitea/GitLab releases. For all-`publish = false` workspaces, set `git_only = true`: versions then come from git tags, and `publish = false` packages are still tagged. `version_group` approximates a single shared version. Forgejo isn't mentioned ([config docs](https://release-plz.dev/docs/config)). Documented.
- **cargo-dist** v0.33.0 (2026-09-11), repo active (pushed 2026-10-14). It fits the `enth` CLI binary and its installers, not CLAP/VST3 bundles: those come from nice-plug's bundler via `cargo xtask bundle` plus a tag workflow that uploads assets. Documented (activity); the scope fit is judgment.
- Recommendation: nothing in Phase 0. At the first public release, release-plz (`git_only`) for tags and changelog, plus a hand-written release workflow for plugin bundles; cargo-dist only if the CLI gets installers.

## Recommendation for Phase 0

1. **Task 1:** use `libjack-jackd2-dev`. Update cargo-deny to 0.20.2 (`cargo install --locked cargo-deny`). Prefer the prebuilt wasmtime release (v49.0.3, 2026-10-14, `wasmtime-v49.0.3-x86_64-linux.tar.xz`) over `cargo install wasmtime-cli` on a laptop with about 2.5 GiB free. Its compile RAM is unmeasured; researcher 03 owns this.
2. **Task 2:** create the files below. Every one was validated in `/tmp/enth-research/05/skel` with 1.95.0 substituted for 1.99.1: fmt, clippy `-D warnings`, `cargo test --workspace --locked`, `cargo deny check` (all four), `check-layers` (and its failure on `enth-dsp → enth-patch`) and `wasm-check` all pass. The CI YAML was only syntax-checked.
3. **Directory naming:** the drafts use `crates/enth-<name>/`. If the owner keeps `crates/<name>/`, only the `[workspace.dependencies]` paths change; `layers.toml` uses package names.

### `Cargo.toml` (root)

```toml
[workspace]
resolver = "3"
members = ["crates/*", "xtask"]           # add "spikes/*", "tools/*", "tests/golden" when the first one exists (an empty glob is an error)
default-members = ["crates/*", "xtask"]   # plain `cargo build/test/clippy` skip spikes and tools

[workspace.package]
version = "0.0.0"
edition = "2024"
license = "MIT"
publish = false

[workspace.dependencies]
# Internal crates: members write `enth-dsp.workspace = true`.
enth-dsp = { path = "crates/enth-dsp" }
enth-params = { path = "crates/enth-params" }
enth-patch = { path = "crates/enth-patch" }
enth-engine = { path = "crates/enth-engine" }
enth-render = { path = "crates/enth-render" }
enth-analysis = { path = "crates/enth-analysis" }
# xtask
serde = { version = "1.0.229", features = ["derive"] }
serde_json = "1.0.154"
toml = "1.1.8"

[workspace.lints.rust]
unsafe_code = "deny"                      # FFI crates opt out at the crate root: #![allow(unsafe_code, reason = "...")]
rust_2018_idioms = { level = "warn", priority = -1 }
missing_debug_implementations = "warn"
trivial_numeric_casts = "warn"
unused_import_braces = "warn"
unused_lifetimes = "warn"

[workspace.lints.clippy]
all = { level = "warn", priority = -1 }
pedantic = { level = "warn", priority = -1 }
# Pedantic lints that mostly fire on DSP arithmetic, or are taste:
cast_possible_truncation = "allow"
cast_possible_wrap = "allow"
cast_precision_loss = "allow"
cast_sign_loss = "allow"
float_cmp = "allow"                       # exact comparisons are deliberate in determinism tests; float_cmp_const stays on
many_single_char_names = "allow"          # x, y, a, b, q, g are normal DSP names
similar_names = "allow"
too_many_lines = "allow"
must_use_candidate = "allow"
missing_errors_doc = "allow"
missing_panics_doc = "allow"
# Restriction lints worth having in an agent-written audio codebase:
allow_attributes_without_reason = "warn"  # every #[allow] says why
dbg_macro = "warn"
todo = "warn"
unimplemented = "warn"
print_stdout = "warn"                     # core crates do no I/O; CLI and xtask opt out at the crate root
print_stderr = "warn"
undocumented_unsafe_blocks = "warn"
large_stack_arrays = "warn"
mem_forget = "warn"
exit = "warn"
float_cmp_const = "warn"
lossy_float_literal = "warn"
rc_mutex = "warn"
```

Versions are crates.io latest on 2026-10-19 (serde 1.0.229 of 2026-07-18, serde_json 1.0.154 of 2026-10-11, toml 1.1.8 of 2026-10-09).

### `crates/enth-<name>/Cargo.toml` (template)

```toml
[package]
name = "enth-dsp"
version.workspace = true
edition.workspace = true
license.workspace = true
publish.workspace = true

[dependencies]
# enth-params.workspace = true   (only strictly lower layers; see xtask/layers.toml)

[lints]
workspace = true
```

### `rust-toolchain.toml`

```toml
[toolchain]
channel = "1.99.1"        # never 1.99.0 (x86_64 FFI bool miscompilation); bump deliberately, then run golden tests
profile = "minimal"
components = ["rustfmt", "clippy"]
targets = ["wasm32-unknown-unknown", "wasm32-wasip1"]
```

### `.cargo/config.toml`

```toml
[alias]
xtask = "run --quiet --package xtask --"
```

(Linker and profile settings belong to researcher 06's report.)

### `clippy.toml` (root): enforces architecture rule 2, which wasm-check can't

```toml
# Lint *options* (lint *levels* live in Cargo.toml [workspace.lints]).
# Clippy uses the nearest clippy.toml walking up from each crate, so this applies to every member.
# Crates allowed to do I/O, spawn threads, lock or read clocks (L6–L8 shells) opt out at the crate root:
#   #![allow(clippy::disallowed_methods, clippy::disallowed_types, reason = "shell crate: owns I/O and threads")]
# Tooling directories (xtask/, tools/, spikes/) carry their own empty clippy.toml instead.
allow-unwrap-in-tests = true
allow-expect-in-tests = true
allow-print-in-tests = true
disallowed-methods = [
  { path = "std::time::Instant::now", reason = "core crates read no clocks (docs/architecture.md, rule 2)" },
  { path = "std::time::SystemTime::now", reason = "core crates read no clocks" },
  { path = "std::thread::spawn", reason = "core crates spawn no threads; the caller supplies parallelism" },
  { path = "std::thread::scope", reason = "core crates spawn no threads; the caller supplies parallelism" },
  { path = "std::env::var", reason = "core crates do no I/O; configuration is passed in" },
  { path = "std::env::var_os", reason = "core crates do no I/O; configuration is passed in" },
  { path = "std::fs::read", reason = "core crates do no I/O; the caller passes bytes in" },
  { path = "std::fs::read_to_string", reason = "core crates do no I/O; the caller passes text in" },
  { path = "std::fs::write", reason = "core crates do no I/O; return data to the caller" },
  { path = "std::fs::File::open", reason = "core crates do no I/O" },
  { path = "std::fs::File::create", reason = "core crates do no I/O" },
  { path = "std::fs::OpenOptions::open", reason = "core crates do no I/O" },
  { path = "std::process::Command::new", reason = "core crates do no I/O" },
]
disallowed-types = [
  { path = "std::sync::Mutex", reason = "no locks in core crates; the audio path is lock-free" },
  { path = "std::sync::RwLock", reason = "no locks in core crates; the audio path is lock-free" },
  { path = "std::collections::HashMap", reason = "RandomState iteration order changes per process; use BTreeMap or Vec for deterministic output" },
  { path = "std::collections::HashSet", reason = "RandomState iteration order changes per process; use BTreeSet" },
]
```

`xtask/clippy.toml` (and later `tools/clippy.toml` and `spikes/clippy.toml`):

```toml
# Tooling is exempt from the core-crate rules in the root clippy.toml
# (clippy uses the nearest clippy.toml, so this empty file overrides it here).
```

The HashMap/HashSet ban is my judgment for determinism: an agent iterating a `HashMap` while building a `RenderPlan` or serialising would produce run-to-run differences. Drop it if it proves too strict.

### `deny.toml`

```toml
# cargo-deny 0.20.x config. Run: cargo deny check
# Licence policy: docs/adr/0004-licensing.md. Ask the owner before adding a licence here.

[graph]
targets = [
  "x86_64-unknown-linux-gnu",
  "x86_64-pc-windows-msvc",
  "aarch64-apple-darwin",
  "x86_64-apple-darwin",
  "wasm32-unknown-unknown",
  "wasm32-wasip1",
]
all-features = true

[advisories]
unmaintained = "workspace"   # fail only when one of our crates depends on it directly
unsound = "all"
yanked = "deny"
ignore = []                  # entries: { id = "RUSTSEC-YYYY-NNNN", reason = "..." }

[licenses]
confidence-threshold = 0.93
allow = [
  "MIT",
  "MIT-0",
  "Apache-2.0",
  "ISC",
  "BSD-2-Clause",
  "BSD-3-Clause",
  "Zlib",
  "Unicode-3.0",
  "CC0-1.0",
]
# Font licences only for the crate that ships egui's default fonts (same scoping as clack's deny.toml).
exceptions = [
  { crate = "epaint_default_fonts", allow = ["OFL-1.1", "Ubuntu-font-1.0"] },
]
unused-allowed-license = "allow"     # the skeleton uses few licences yet
unused-license-exception = "allow"   # egui arrives with the UI spike

[licenses.private]
ignore = true                # our own crates are publish = false

[bans]
multiple-versions = "warn"
wildcards = "deny"
allow-wildcard-paths = true
deny = [
  { crate = "vst3-sys", reason = "GPL-3.0; use the MIT/Apache `vst3` crate (ADR-0004)" },
  { crate = "openssl-sys", reason = "system C library; use rustls if networking is ever needed" },
  { crate = "openssl", reason = "system C library; use rustls if networking is ever needed" },
]

[sources]
unknown-registry = "deny"
unknown-git = "deny"
allow-registry = ["https://github.com/rust-lang/crates.io-index"]
allow-git = []               # e.g. vizia-plug (not on crates.io) for the UI spike: add its exact URL
```

Notes:

- `MIT-0` isn't in ADR-0004's list; it's strictly more permissive than MIT. Drop it if the owner wants the list verbatim.
- Expect to need decisions on `BSL-1.0` (permissive; clack needs it for `error-code` and `clipboard-win` on Windows) and on an `MPL-2.0` exception for `option-ext`, once the GUI spikes land.
- nice-plug 0.4.2 and nice-plug-egui 0.6.0 are on crates.io (ISC), so no git source is needed for Task 3. vizia-plug is not on crates.io, so Task 4 needs an `allow-git` entry.

### `xtask/Cargo.toml`

```toml
[package]
name = "xtask"
version.workspace = true
edition.workspace = true
license.workspace = true
publish.workspace = true

[lints]
workspace = true

[dependencies]
serde.workspace = true
serde_json.workspace = true
toml.workspace = true
```

### `xtask/layers.toml`

```toml
# Single source of truth for crate layers (docs/architecture.md). Package names, not directories.
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
packages = ["xtask", "enth-test-host", "enth-golden"]
prefixes = ["spike-"]
```

Listing not-yet-existing crates is fine: the check only errors on members that are *missing* from the file.

### `xtask/src/main.rs` (validated prototype: rustfmt-clean, clippy-clean under the lint set above)

```rust
//! Project automation: `cargo xtask <command>`. Keep dependencies minimal.
#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "xtask is a CLI tool"
)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use serde::Deserialize;

#[derive(Deserialize)]
struct LayersFile {
    layers: BTreeMap<String, u8>,
    core: Core,
    exempt: Exempt,
}

#[derive(Deserialize)]
struct Core {
    max_layer: u8,
}

#[derive(Deserialize)]
struct Exempt {
    #[serde(default)]
    packages: Vec<String>,
    #[serde(default)]
    prefixes: Vec<String>,
}

#[derive(Deserialize)]
struct Metadata {
    packages: Vec<Package>,
    workspace_members: Vec<String>,
    workspace_root: PathBuf,
}

#[derive(Deserialize)]
struct Package {
    id: String,
    name: String,
    dependencies: Vec<Dependency>,
}

#[derive(Deserialize)]
struct Dependency {
    name: String,
    kind: Option<String>,
    path: Option<PathBuf>,
}

enum Class {
    Layer(u8),
    Exempt,
    Unlisted,
}

impl LayersFile {
    fn classify(&self, name: &str) -> Class {
        if let Some(&layer) = self.layers.get(name) {
            Class::Layer(layer)
        } else if self.exempt.packages.iter().any(|p| p == name)
            || self
                .exempt
                .prefixes
                .iter()
                .any(|p| name.starts_with(p.as_str()))
        {
            Class::Exempt
        } else {
            Class::Unlisted
        }
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("check-layers") => check_layers(),
        Some("wasm-check") => wasm_check(),
        _ => Err("usage: cargo xtask <check-layers|wasm-check>".to_owned()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("xtask: {msg}");
            ExitCode::FAILURE
        }
    }
}

fn cargo() -> Command {
    Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
}

fn metadata() -> Result<Metadata, String> {
    let out = cargo()
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .output()
        .map_err(|e| format!("cannot run cargo metadata: {e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).into_owned());
    }
    serde_json::from_slice(&out.stdout).map_err(|e| format!("bad cargo metadata JSON: {e}"))
}

fn layers(root: &Path) -> Result<LayersFile, String> {
    let path = root.join("xtask/layers.toml");
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
}

fn check_layers() -> Result<(), String> {
    let meta = metadata()?;
    let cfg = layers(&meta.workspace_root)?;
    let members: BTreeMap<&str, &Package> = meta
        .packages
        .iter()
        .filter(|p| meta.workspace_members.contains(&p.id))
        .map(|p| (p.name.as_str(), p))
        .collect();
    let mut errors = Vec::new();
    for (&name, pkg) in &members {
        let my_layer = match cfg.classify(name) {
            Class::Layer(l) => l,
            Class::Exempt => continue,
            Class::Unlisted => {
                errors.push(format!(
                    "{name}: workspace member is neither layered nor exempt in xtask/layers.toml"
                ));
                continue;
            }
        };
        for dep in pkg.dependencies.iter().filter(|d| d.path.is_some()) {
            if !members.contains_key(dep.name.as_str()) {
                continue;
            }
            let kind = dep.kind.as_deref().unwrap_or("normal");
            match cfg.classify(&dep.name) {
                Class::Layer(dep_layer) if dep_layer < my_layer => {}
                Class::Layer(dep_layer) => errors.push(format!(
                    "{name} (L{my_layer}) -> {} (L{dep_layer}) [{kind}]: dependencies must point to a strictly lower layer",
                    dep.name
                )),
                Class::Exempt | Class::Unlisted => errors.push(format!(
                    "{name} (L{my_layer}) -> {} [{kind}]: layered crates must not depend on tooling or unlisted crates",
                    dep.name
                )),
            }
        }
    }
    if errors.is_empty() {
        println!("check-layers: ok ({} workspace members)", members.len());
        Ok(())
    } else {
        Err(format!("check-layers failed:\n  {}", errors.join("\n  ")))
    }
}

fn wasm_check() -> Result<(), String> {
    let meta = metadata()?;
    let cfg = layers(&meta.workspace_root)?;
    let core: Vec<&str> = meta
        .packages
        .iter()
        .filter(|p| meta.workspace_members.contains(&p.id))
        .filter(|p| matches!(cfg.classify(&p.name), Class::Layer(l) if l <= cfg.core.max_layer))
        .map(|p| p.name.as_str())
        .collect();
    if core.is_empty() {
        println!("wasm-check: no core crates yet");
        return Ok(());
    }
    let mut cmd = cargo();
    cmd.args(["build", "--lib", "--target", "wasm32-unknown-unknown"]);
    for name in &core {
        cmd.args(["-p", name]);
    }
    let status = cmd
        .status()
        .map_err(|e| format!("cannot run cargo build: {e}"))?;
    if status.success() {
        println!("wasm-check: ok ({})", core.join(", "));
        Ok(())
    } else {
        Err("wasm-check failed: a core crate does not build for wasm32-unknown-unknown".to_owned())
    }
}
```

Policy encoded here, all judgment:

- Dev- and build-dependencies obey the layers too.
- Layered crates may not depend on exempt tooling.
- Exempt crates may depend on anything.
- wasm-check builds `--lib` only, so dev-dependencies don't matter. It does catch hidden RNG: `getrandom` 0.4.3 doesn't build for `wasm32-unknown-unknown` without an explicit backend ([README](https://github.com/rust-random/getrandom), documented).

Possible later extension: `check-core-deps`, which fails if any transitive normal dependency of an L0–L5 crate has a `links` key or depends on `cc`/`cmake`/`bindgen`. It would enforce AGENTS.md's "no crates that wrap C/C++ in L0–L5"; wasm-check is not a reliable guard for that, because clang is installed and `cc` can target wasm32.

### `.github/workflows/ci.yml`

```yaml
name: ci

on:
  push:
    branches: [main]
  pull_request:
  workflow_dispatch:
  schedule:
    - cron: "23 5 * * 1"   # weekly, so new RustSec advisories surface without a push

permissions:
  contents: read

concurrency:
  group: ${{ github.workflow }}-${{ github.ref }}
  cancel-in-progress: ${{ github.event_name == 'pull_request' }}

env:
  CARGO_TERM_COLOR: always
  CARGO_BUILD_WARNINGS: deny   # cargo >= 1.97: warnings fail the build without invalidating the cache (unlike RUSTFLAGS=-Dwarnings)
  RUST_BACKTRACE: "1"

jobs:
  check:
    name: fmt, clippy, test, layers, wasm
    runs-on: ubuntu-24.04      # pinned: the owner's Mint 22.3 is based on Ubuntu 24.04
    timeout-minutes: 30
    steps:
      - uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1
        with:
          persist-credentials: false

      - name: Install the toolchain from rust-toolchain.toml
        run: rustup toolchain install && rustc --version

      # Only needed once a crate links ALSA/JACK/X11/GL (the plugin and UI spikes). Same list as nice-plug's CI.
      - name: System libraries for plugin, GUI and audio crates
        if: hashFiles('spikes/**/Cargo.toml', 'tools/**/Cargo.toml') != ''
        run: |
          sudo apt-get update
          sudo apt-get install -y --no-install-recommends pkg-config \
            libasound2-dev libgl-dev libjack-jackd2-dev \
            libx11-xcb-dev libxcb1-dev libxcb-dri2-0-dev libxcb-icccm4-dev \
            libxcursor-dev libxkbcommon-dev libxcb-shape0-dev libxcb-xfixes0-dev

      - uses: Swatinem/rust-cache@6323deb102c322ba6fcbdcafc7e3dddab59af2b6 # v2.9.2
        with:
          save-if: ${{ github.ref == 'refs/heads/main' }}   # PRs read the cache, only main writes it

      - run: cargo fmt --all --check
      - run: cargo clippy --workspace --all-targets --locked -- -D warnings
      - run: cargo test --workspace --locked
      - run: cargo xtask check-layers
      - run: cargo xtask wasm-check

  deny:
    name: cargo-deny ${{ matrix.checks }}
    runs-on: ubuntu-24.04
    timeout-minutes: 10
    strategy:
      fail-fast: false
      matrix:
        checks: [advisories, bans licenses sources]
    # A newly published advisory should not turn every PR red; it shows as a failed (allowed) job.
    continue-on-error: ${{ matrix.checks == 'advisories' }}
    steps:
      - uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1
        with:
          persist-credentials: false
      - run: rustup toolchain install
      - uses: taiki-e/install-action@8cca0918f8471c33515202b73f56ab27f2bed283 # v2.87.29
        with:
          tool: cargo-deny@0.20.2
      - run: cargo deny --locked check ${{ matrix.checks }}
```

Why each choice:

- **SHA pins** were resolved from the tags today via `gh api`; keep them current with Dependabot or Renovate (not researched today).
- **`hashFiles` in a step `if`** is allowed ([contexts reference](https://docs.github.com/en/actions/reference/workflows-and-actions/contexts), documented).
- **No secrets** in any job, and caches are written only from `main`. This follows the 2026-09-21 Miri advisory.
- **taiki-e instead of cargo-deny-action:** it runs cargo-deny on the runner with our pinned toolchain, instead of a Docker image built on rust 1.85 that would still install 1.99.1 from the toolchain file.
- **`--locked`** needs `Cargo.lock` committed.

Optional cheap additions: `crate-ci/typos` (v1.51.1, 2026-10-06; the owner has typos-cli 1.45.1), and `cargo nextest` (0.9.148) for per-test timeouts once tests grow.

**For Task 3 (not Task 2): clap-validator job.** Unverified sketch. The bundle path depends on nice-plug's bundler output.

```yaml
  validate:
    needs: check
    if: hashFiles('spikes/shell/Cargo.toml') != ''
    runs-on: ubuntu-24.04
    steps:
      # checkout, rustup toolchain install, apt step, rust-cache: as in `check`
      - run: cargo xtask bundle spike-shell --release
      - name: clap-validator 0.4.1
        env:
          GH_TOKEN: ${{ github.token }}
        run: |
          gh release download 0.4.1 --repo free-audio/clap-validator --pattern '*ubuntu-22.04.zip' --dir "$RUNNER_TEMP/cv"
          cd "$RUNNER_TEMP/cv" && unzip -q ./*.zip && tar -xzf ./*.tar.gz
          "$RUNNER_TEMP/cv/clap-validator" validate "$GITHUB_WORKSPACE/target/bundled/spike-shell.clap" --only-failed
```

Record the asset's SHA-256 once and check it in the step, or build from source with `cargo install --locked --git https://github.com/free-audio/clap-validator --tag 0.4.1`, which rust-cache's `cache-bin` then keeps. Add `xvfb-run` only if a failure shows the plugin touching X11 at load.

### `.gitignore`

```gitignore
/target
*.wav                 # goldens are hashes + summaries; render WAVs into target/golden/ on demand
.venv/
# Research sessions are committed (see Plan check); anything else under _local/ stays local.
/_local/*
!/_local/sessions/
```

(If the owner keeps sessions out of git: replace the last two lines with `/_local/`.) Commit `Cargo.lock`.

## Plan check — what the evidence says should change

Only Rust and a native non-WebView UI are fixed; these are suggestions with reasons.

1. **phase-0.md Task 1, apt line: replace `libjack-dev` with `libjack-jackd2-dev`.** On the owner's machine, `apt-get -s install libjack-dev` plans to remove `libjack-jackd2-0` (amd64 and i386) and `libasound2-plugins:i386`, replacing them with JACK1's `libjack0` (measured, simulation only). The i386 ALSA plugins matter to 32-bit Wine audio, and the owner runs REAPER under Wine. The corrected list installs 22 packages and removes none (measured). nice-plug's CI uses the jackd2 package.
2. **Task 1, cargo-deny:** update to 0.20.2 to match CI (0.20.0 changed the CLI). **wasmtime:** prefer the prebuilt release over `cargo install wasmtime-cli` given the RAM. That's judgment; I didn't measure its build.
3. **ADR-0004, AGENTS.md (Licensing), phase-0 Task 2 step 7: `LicenseRef-UFL-1.0` → `Ubuntu-font-1.0`.** egui's font crate has used the SPDX id since 0.31.0 (2025-02-04); egui's and clack's deny.toml use it. Scope OFL-1.1/Ubuntu-font-1.0 as `exceptions` for `epaint_default_fonts`, not global `allow`. Also flag in ADR-0004 that BSL-1.0 (permissive) and an MPL-2.0 exception (`option-ext`) are likely to come up with the GUI stack (clack needs both), so the owner can decide ahead of time.
4. **Task 2 step 5:** pin `1.99.1` explicitly (not "current stable"), add `profile = "minimal"`, and note "never 1.99.0".
5. **Task 2 step 3:** add `default-members` (spikes would otherwise build on every plain `cargo test`), `[workspace.dependencies]` and the extended lint set. Keep the "add `spikes/*` when the first exists" comment: an empty glob is a hard error (measured).
6. **architecture.md rule 2 / Task 2 step 10:** say plainly that wasm-check proves *buildability* only. Add the root `clippy.toml` with `disallowed-methods`/`disallowed-types` as the enforcement for "no I/O, threads, clocks, locks", with shells and tooling opting out (measured working). Also add to AGENTS.md: every `#[allow]` needs `reason = "…"`.
7. **Task 2 step 11 (CI):** pin `ubuntu-24.04` rather than `ubuntu-latest` (26.04 exists and will take over the label at some point). Use `CARGO_BUILD_WARNINGS=deny`, rust-cache with `save-if`, the advisories leg allowed to fail, SHA-pinned actions and `--locked`. Add to step 1: **commit `Cargo.lock`**.
8. **Codeberg option in step 11:** keep GitHub as primary. Hosted Forgejo Actions is too small for this build (4 CPU / 8 GB including writes / 10 min), and hosted Woodpecker needs approval. Codeberg can be a push mirror.
9. **architecture.md folder topology:** rename `crates/dsp` → `crates/enth-dsp` (etc.), matching directory to package as matklad advises and egui, bevy and nice-plug do. Cheap now, a churn of every path later. Owner's call; the drafts assume it.
10. **`tests/golden/` → a workspace crate `enth-golden`** (exempt tooling): golden checks need render + analysis, which are both L3. **Change the `.gitignore` rule** from "`*.wav` except under `tests/golden/`" to "no WAVs in git"; keep hashes and summaries as text.
11. **research-process.md / phase-0 Task 2 step 1, `_local/`: commit the research sessions.** ADR-0006, research.md and roadmap.md cite `_local/sessions/…` as their evidence, and ignored files vanish from every clone, CI run and cloud agent. The sessions are about 248 KB. Optionally rename to a name that doesn't imply "local" (e.g. `research/sessions/`); that's about 10 doc references. Before the repo goes public, review briefs for personal details (this session's brief lists the owner's hardware and a driver mismatch).
12. **AGENTS.md "Done means":** fine as written. With `default-members`, plain `cargo test` covers product crates only, while CI runs `--workspace`. Say so, so agents know spikes aren't covered locally.
13. **phase-0 Task 3 (heads-up):** since Rust 1.98, `unsafe_code` fires on all unsafe attributes (`#[unsafe(no_mangle)]`). Spike and plugin crates will likely need `#![allow(unsafe_code, reason = "CLAP entry point")]`.

Nothing in Task 7 needs changing.

## Gaps — what I couldn't verify, and the local check that settles it

All checks below are for after the reboot; run them in a scratch copy of `/tmp/enth-research/05/skel`, or in the repo during Task 2.

| Gap | Check |
| --- | --- |
| Nothing was run on **Rust 1.99.1**: all local validation used 1.95.0 (`RUSTUP_TOOLCHAIN=stable`/`1.95.0`). New 1.96–1.99 clippy lints may fire on the drafts | In the skeleton with the real `rust-toolchain.toml`: `cargo fmt --all --check && cargo clippy --workspace --all-targets --locked -- -D warnings && cargo test --workspace --locked && cargo xtask check-layers && cargo xtask wasm-check` |
| Whether `CARGO_BUILD_WARNINGS=deny` also fails clippy warnings and `linker_messages` (needs cargo ≥ 1.97) | `CARGO_BUILD_WARNINGS=deny cargo clippy --workspace` with a deliberate `dbg!()` in a crate; expect failure |
| cargo-deny **0.20.2** not run (local 0.19.4); the drafted fields exist in both per the changelog | `cargo install --locked cargo-deny` (→ 0.20.2), then `cargo deny --locked check` in the skeleton |
| CI workflow never ran on GitHub; YAML only parsed (no `actionlint` installed) | Push the Task 2 repo; or `actionlint` via taiki-e/install-action in CI |
| clap-validator headless in CI and the zip → tar.gz layout (inferred from its build workflow) | Task 3: run the `validate` job sketch once; locally, `unzip -l` the asset |
| Whether nice-plug-egui (egui `default-features = false`) pulls `epaint_default_fonts`, and which BSL-1.0/MPL-2.0 crates appear with the GUI spikes | Task 4: `cargo tree -i epaint_default_fonts` and `cargo deny check licenses` after adding the spikes |
| `unsafe_code` vs nice-plug's export macro under 1.98+ (does a lint from an external macro fire in our crate?) | Task 3: `cargo clippy -p spike-shell` with workspace lints on |
| apt step duration on GitHub runners, and whether the apt cache action pays off | First CI runs: read the step timings |
| Prebuilt wasmtime vs `cargo install wasmtime-cli` RAM/time on this laptop | `/usr/bin/time -v cargo install --locked wasmtime-cli --root /tmp/wt` with `CARGO_BUILD_JOBS=2` (or just use the release tarball) |
| insta `Comparator` (1.47+) semantics: its docs page didn't load | `cargo doc -p insta --open` in a scratch crate, or read `src/comparator.rs` |
| The first version affected by the FFI-bool miscompilation (1.99.0 confirmed; earlier unknown) | Read rust-lang/rust#163911's linked fix PR; irrelevant if pinning 1.99.1 |
| `cargo-depgraph-check` was not tried (2 stars; not recommended) | Optional: `cargo install --root /tmp/dg cargo-depgraph-check`, run in the skeleton |
| Dependabot/Renovate config for SHA-pin updates not researched | Read GitHub's dependabot.yml docs when creating the repo |
