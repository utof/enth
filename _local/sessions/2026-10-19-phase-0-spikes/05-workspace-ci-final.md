# 05 — Workspace and CI (final)

- Date: 2026-10-19
- Sub-researcher: 05 (workspace and CI)
- Questions answered: 1–9 from the brief, in full, plus everything listed under Gaps in the first report.
- Relationship to the first report: [`05-workspace-ci.md`](05-workspace-ci.md) stays as written. This report is self-contained: it repeats what still stands and marks every difference.
  - **[new]**: evidence gathered after the first report.
  - **[changed]**: replaces a claim or draft from the first report.
  - **[confirmed]**: was documented or inferred in the first report and has now been measured.
- Method:
  - Web and API lookups (crates.io API, `gh api`, official docs, fetched 2026-10-19).
  - Local runs on the owner's laptop with **rustc/cargo 1.99.1** (installed via `rustup toolchain install 1.99.1 --profile minimal`), **cargo-deny 0.20.2** (`cargo install --root` into scratch), **actionlint 1.7.12** and **shellcheck 0.11.0** (release binaries, sha256 verified), **clap-validator 0.4.1** and **wasmtime 49.0.3** (release binaries, sha256 verified against GitHub's asset digests).
  - Scratch lives in `~/code/enth-scratch/2026-10-19-phase-0-spikes/05/` (`final/` = the Task 2 drafts, `final-golden/` = Task 2 plus the golden harness, `notes.md` = the running log).
  - Builds ran under `flock -s /tmp/enth-quiet.lock` with `CARGO_BUILD_JOBS` of 3–4; timings under `flock -x` with `uptime` recorded.
  - Two reboots wiped `/tmp` mid-run. Everything measured before the second crash is logged in `notes.md`, and the final drafts were rebuilt and re-verified in persistent storage.
- Labels: **measured** = run here; **documented** = official docs, changelogs, source or repo files; **anecdotal** = what other projects do or say.
- Side effects on the owner's machine (user-level only):
  - Toolchain 1.99.1 added to `~/.rustup`. rustup updated itself from 1.29.0 to 1.29.1 during that install (it self-updates by default), and auto-installed `wasm32-wasip1` for 1.99.1 the first time it ran inside a directory whose `rust-toolchain.toml` lists it.
  - One full `cargo deny check` refreshed the existing advisory-DB cache in `~/.cargo/advisory-dbs` (present since 2026-09-09).
  - Nothing went into `~/.cargo/bin` or system paths.

## Answer in brief

- **Toolchain:** pin `channel = "1.99.1"` with `profile = "minimal"`. 1.99.1 is the current stable (2026-10-15). Never pin 1.99.0: it miscompiles x86_64 FFI `bool` returns, and CLAP's C API returns `bool` everywhere. The regression is new in 1.99.0 (inferred: downstream projects pinned 1.98.1 to avoid it). Edition 2024 is still the newest. A virtual workspace still needs an explicit `resolver = "3"` (measured: without it, cargo falls back to resolver 1). **[confirmed]** The whole Task 2 skeleton passes on 1.99.1.
- **Warnings in CI:** use `CARGO_BUILD_WARNINGS=deny` (cargo ≥ 1.97) rather than `RUSTFLAGS=-Dwarnings`. **[new, measured]** It fails the build on rustc warnings, on clippy warnings **and on linker warnings** (`linker_messages`), and toggling it doesn't rebuild anything, while a `RUSTFLAGS` change re-checked the crate.
- **Lints:** clippy `all` + `pedantic` (priority −1) with DSP-specific allows, plus cherry-picked restriction lints, chiefly `allow_attributes_without_reason`. **[changed]** Two more allows (`unreadable_literal`, `inline_always`), and `missing_debug_implementations` dropped. **[new]** Measured on fundsp 0.23.0 (~30k lines of DSP): raw pedantic gives 2251 warnings, the first-report set 1530, the final set 939, most of them auto-fixable.
- **Core-crate rules:** wasm-check proves only that core crates *build* for wasm32. It does not stop I/O, threads or clocks: a wasm32 build accepts `Instant::now`, `fs::read`, `thread::spawn` and `env::var` (measured again on 1.99.1). A root `clippy.toml` with `disallowed-methods`/`disallowed-types` catches them; shells and tooling opt out.
- **cargo-deny 0.20.2:** the drafted config passes. **[new]** `--config` is now a top-level option (`cargo deny --config X check`). egui's fonts use SPDX **`Ubuntu-font-1.0`**, and I scope them to `epaint_default_fonts`.
  - **[new, measured]** On a real nice-plug-egui GUI dependency graph (272 crates), the only licence failure is **BSL-1.0**: `clipboard-win` and `error-code`, on Windows. No MPL-2.0 crate appears.
  - nice-plug-egui pulls the fonts crate only if egui's `default_fonts` feature is enabled.
- **Layering:** keep the custom `cargo xtask check-layers` (191 lines, about 0.1 s warm).
  - **[new]** I tried `cargo-depgraph-check` 0.1.0. It works, but it checks per-crate allow-lists rather than layers, and costs a 163 s install. cargo-deny's `wrappers` can ban workspace crates too (measured), but would mean keeping an allow-list per crate by hand.
- **xtask over just:** a cargo alias only invokes xtask.
- **CI:** GitHub Actions on `ubuntu-24.04`, with rust-cache and actions pinned by commit SHA.
  - **[new]** actionlint and shellcheck pass on the workflow. actionlint also caught a bug in my first report's clap-validator job: a job-level `if: hashFiles()` isn't allowed. The steps passed locally under `CI=true`, and the cold check sequence took 21.7 s here.
  - **[new]** Public nih-plug runs show the apt step at 13–29 s, so no apt cache is needed.
  - **[new]** clap-validator 0.4.1 runs headless: on a nice-plug + egui spike bundle, 44 tests ran (36 passed, 8 skipped) in 0.27 s with no display. It has no GUI tests and exits non-zero only on failures.
- **Golden tests:** an exact hash per platform plus a feature summary compared with a tolerance.
  - **[new]** The prototype harness and `cargo xtask golden [--update]` work on native and on wasm32-wasip1 under wasmtime.
  - **[new, measured]** The same `f32::sin` render hashes differently on x86_64-linux and wasm32-wasi, while the summaries agree within 0.01 dB. That's exactly why the design needs both.
  - **[new]** `std::env::consts::OS` is empty on wasm32-wasip1, so the platform key has to come from `cfg!`.
- **Plan check, high-value items:**
  - (1) Task 1's `libjack-dev` would remove the owner's `libjack-jackd2-0` (amd64 + i386) and `libasound2-plugins:i386`. It comes from nih-plug's CI list; use `libjack-jackd2-dev`.
  - (2) Replace `LicenseRef-UFL-1.0` with `Ubuntu-font-1.0`, and get the owner's decision on BSL-1.0 before the UI spike.
  - (3) Commit `_local/sessions/`.
  - (4) Add `default-members`.
  - (5) **[new]** The first report's `.gitignore` draft was broken: gitignore has no trailing comments, so its `*.wav  # …` line left WAVs tracked. Fixed and verified below.

## Findings

### 1. Toolchain, edition, resolver

| Claim | Evidence | Label |
| --- | --- | --- |
| Current stable is **1.99.1**, released **2026-10-15**; 1.99.0 was 2026-10-01; beta 1.100.0-beta.7 (2026-10-17). 1.100.0 is due about 2026-11-12 on the six-week cadence (inference) | `channel-rust-stable.toml`/`-beta.toml`; [RELEASES.md](https://raw.githubusercontent.com/rust-lang/rust/stable/RELEASES.md); [blog](https://blog.rust-lang.org/2026/10/15/Rust-1.99.1/) | documented |
| Since the owner's 1.95.0: 1.96.0 (2026-05-28), 1.97.0 (2026-07-09), 1.98.0 (2026-08-20). 1.95.0 itself was released 2026-04-16; the brief's 2026-04-14 is the commit date that `rustc --version` prints | RELEASES.md; local `rustc 1.95.0 (59807616e 2026-04-14)` | documented / measured |
| 1.99.1 fixes an x86_64 FFI `bool` return miscompilation ([#163911](https://github.com/rust-lang/rust/issues/163911), `P-critical`, `regression-from-stable-to-stable`; fix [#163917](https://github.com/rust-lang/rust/pull/163917) merged 2026-10-10). **[new]** Downstream repos pinned 1.98.1 as a workaround ("fix(ci): pin release Rust to 1.98.1 for correct FFI bool returns"), so 1.99.0 introduced it (inference from the issue's cross-references) | issue timeline via `gh api` | documented / inference |
| Cargo 1.97: `build.warnings` / `CARGO_BUILD_WARNINGS` | [1.97 blog](https://blog.rust-lang.org/2026/07/09/Rust-1.97.0/) | documented |
| **[new]** `CARGO_BUILD_WARNINGS=deny` fails on an unused-variable warning (`cargo check`, exit 101), on clippy warnings (`dbg_macro`) and on a linker warning (`warning: linker stderr: rust-lld: unknown -z value: bogusopt` under default `#[warn(linker_messages)]`), each with exit 101 | scratch, 1.99.1 | measured |
| **[new]** Toggling `CARGO_BUILD_WARNINGS` didn't rebuild anything; `RUSTFLAGS="-Dwarnings"` re-checked the crate, and switching back was fresh (cargo keeps separate artifacts per `RUSTFLAGS`) | scratch, 1.99.1 | measured |
| **[new]** The default linker on x86_64 Linux is rust-lld (it names itself in the warning; default since 1.90) | scratch; RELEASES.md 1.90.0 | measured / documented |
| Cargo 1.99 disables incremental compilation when `CI` is set; 1.98 makes `unsafe_code` fire for all unsafe attributes; 1.99 adds a built-in `debug` profile | RELEASES.md | documented |
| Edition 2024 (stable since 1.85.0, 2025-02-20) is the newest; no 2027 edition has been announced | RELEASES.md; [internals thread](https://internals.rust-lang.org/t/we-have-only-two-years-to-prepare-the-rust-2027-edition/22494) | documented / anecdotal |
| A virtual workspace without `resolver` falls back to resolver 1 and warns | scratch (cargo 1.95.0) | measured |
| **[new]** Resolver 3's fallback uses the pinned toolchain: `cargo generate-lockfile` printed "Locking 20 packages to highest Rust 1.99.1 compatible versions" | scratch, 1.99.1 | measured |
| rustup ≥ 1.28.1 auto-installs the toolchain-file toolchain. **[new]** `rustup toolchain install` without arguments works in CI-style use ("the active toolchain `1.99.1` has been installed … overridden by rust-toolchain.toml"), installs the listed targets, and also checks for a rustup self-update | [rustup CHANGELOG](https://github.com/rust-lang/rustup/blob/main/CHANGELOG.md); scratch | documented / measured |
| GitHub's `ubuntu-24.04` image 20261012 ships Rust 1.99.0 and rustup 1.29.1, so each job downloads 1.99.1 once | [runner-images](https://github.com/actions/runner-images/blob/main/images/ubuntu/Ubuntu2404-Readme.md) | documented |

Why an exact pin: pedantic gains lints every release (1.99 added `assert_is_empty` and `manual_bit_width`, per the [clippy CHANGELOG](https://github.com/rust-lang/rust-clippy/blob/master/CHANGELOG.md)). std's float functions are documented as non-deterministic across platforms *and Rust versions*, so a toolchain bump is a golden review (§6). `rust-version` isn't needed: with `publish = false`, resolver 3 already uses the running toolchain (measured above).

### 2. `[workspace.lints]` and clippy sets

| Claim | Evidence | Label |
| --- | --- | --- |
| Lint groups need a lower `priority`. A member **can't** override inherited lints: cargo errors "cannot override `workspace.lints` in `lints`". So `unsafe_code` stays `deny`, and FFI crates write `#![allow(unsafe_code, reason = "…")]` | [manifest docs](https://doc.rust-lang.org/cargo/reference/manifest.html); scratch | documented / measured |
| egui and rerun: `all` + `cargo` + `pedantic` with allow-lists. xilem and bevy: cherry-picked lints. Audio: clack denies `missing_docs`, `undocumented_unsafe_blocks` and the `cast_*` lints; Symphonia sets `unwrap_used = "warn"`; nice-plug, nih-plug, fundsp, cpal, kira, rodio, firewheel and web-audio-api-rs have no `[workspace.lints]` (nice-plug and cpal use `clippy -- -D warnings` in CI) | repo files fetched 2026-10-19 | anecdotal |
| clippy.toml: the nearest file wins (no merging). A root file covers `crates/*`; a crate-root `#![allow(clippy::disallowed_methods, reason = …)]` opts out; an empty directory-level `clippy.toml` (e.g. `spikes/`, `xtask/`) overrides the root | [clippy config docs](https://doc.rust-lang.org/clippy/configuration.html); scratch | documented / measured |
| **[confirmed]** On 1.99.1 the drafted `clippy.toml` flags `Instant::now`, `fs::read`, `env::var`, `Command::new`, `thread::scope`, `HashMap` (×3) and `Mutex`, while `cargo build --target wasm32-unknown-unknown` of the same code succeeds (exit 0). Without `xtask/clippy.toml`, xtask itself trips three rules (`env::var_os`, `fs::read_to_string`, `Command::new`), so the override is needed | scratch, 1.99.1 | measured |
| **[new]** Noise on real DSP code: fundsp 0.23.0 (~30.1k lines), `cargo clippy --lib` on 1.99.1. **Raw `all` + `pedantic`: 2251 warnings** (`must_use_candidate` 624, `unreadable_literal` 280, `wildcard_imports` 259, `cast_possible_truncation` 190, `cast_lossless` 171, `inline_always` 141, `cast_precision_loss` 94, `cast_sign_loss` 78, `missing_panics_doc` 65, …). **First-report set: 1530.** **Final set: 939** (`wildcard_imports` 259, `cast_lossless` 171, `elided_lifetimes_in_paths` 123, `allow_attributes_without_reason` 43, `return_self_not_must_use` 42, `doc_markdown` 41, `explicit_iter_loop` 40, …) | scratch | measured |

**[changed]** Final allow/warn choices:

- `unreadable_literal` → allow, so ported coefficients keep their upstream digits (ADR-0006 ports Signalsmith and others, and the numbers should be checkable against the source).
- `inline_always` → allow (egui does the same).
- `missing_debug_implementations` removed: it accounted for 170 of fundsp's warnings, for little value.
- The remaining top lints are mostly auto-fixable with `cargo clippy --fix` in new code (`cast_lossless`, `elided_lifetimes_in_paths`, `explicit_iter_loop`).
- `allow_attributes` stays off: crate-wide opt-outs need `#[allow]`, not `#[expect]`.
- No-panic lints on the audio path belong as module-level `#![deny(...)]` in `enth-engine`. That's judgment.

### 3. cargo-deny, licences and layer enforcement

| Claim | Evidence | Label |
| --- | --- | --- |
| Latest **cargo-deny 0.20.2** (2026-07-09; 0.20.0 refactored the CLI). The owner has 0.19.4. cargo-deny-action v2.1.1 bundles 0.20.2 in a `rust:1.85.0-alpine` image | crates.io; [CHANGELOG](https://github.com/EmbarkStudios/cargo-deny/blob/main/CHANGELOG.md); [action](https://github.com/EmbarkStudios/cargo-deny-action) | documented |
| **[new]** In 0.20.2, `--config`, `--locked`, `--manifest-path`, `--all-features` and friends are **top-level** options: `cargo deny --locked --config X check advisories` works, while `cargo deny check --config X` fails with a usage error. `check` no longer lists `-d/--disable-fetch` | scratch, `cargo deny --help` | measured |
| **[new]** cargo-deny 0.20.2 compiles from source in 294 s (3 jobs) to 381 s (4 jobs, heavier load), about 690 MB max RSS. Prebuilt binaries via taiki-e/install-action avoid that in CI | scratch | measured |
| `epaint_default_fonts` 0.37.0 (2026-10-15) is `(MIT OR Apache-2.0) AND OFL-1.1 AND Ubuntu-font-1.0` (SPDX id since 0.31.0, 2025-02-04). egui's deny.toml allows `Ubuntu-font-1.0`; clack scopes it to `epaint_default_fonts` | crates.io API; repos | documented / anecdotal |
| **[new]** nice-plug-egui 0.6.0 depends on egui 0.37.0 with `default-features = false, features = ["bytemuck"]`. Without the app enabling egui's `default_fonts` feature, `epaint_default_fonts` is **not** in the graph (`cargo tree -i` finds nothing). Researcher 01's spike enables `default_fonts` explicitly | copy of researcher 01's `spike-np-gui` manifest and lockfile | measured |
| **[new]** The drafted deny.toml on that GUI graph (nice-plug =0.4.2 with `vst3, editor`; nice-plug-egui =0.6.0; egui 0.37.0 with `default_fonts`; 272 lockfile packages) **fails `licenses` only for BSL-1.0**: `clipboard-win` 5.4.1 (via `copypasta` 0.10.2) and `error-code` 3.4.0 (via `clipboard-win`). With exceptions for those two: `bans ok, licenses ok, sources ok`, advisories ok (with `unmaintained = "all"` too). Duplicate warnings: objc2 ×2, objc2-app-kit ×2, objc2-foundation ×2, syn ×3. No MPL-2.0 `option-ext` in this graph | scratch, cargo-deny 0.20.2 | measured |
| `deny` + `wrappers` applies to workspace path crates | scratch, cargo-deny 0.19.4 | measured |
| **[new]** `cargo-depgraph-check` 0.1.0 (MIT, 2 stars): install 163 s / 284 MB. `generate` writes per-crate allow-lists with `strict = true` and `check_dev_deps = false`. It caught an upward dependency ("enth-dsp depends on enth-patch, but allowed deps are: []") and an unlisted member | scratch | measured |
| cargo-archtest is AGPL-3.0 (excluded); guppy 0.19.1 is a library | crates.io | documented |
| **[confirmed]** The prototype `check-layers` on 1.99.1 fails on the deliberate `enth-dsp → enth-patch` edge (exit 1) and passes otherwise. Earlier: same-layer, upward dev-dependency and unlisted-member detection; 0.09–0.12 s warm | scratch | measured |

Verdict unchanged: a custom `check-layers` keyed on layer numbers. cargo-depgraph-check would need an edit for every new *legitimate* downward edge, and a separate install. **[changed]** Use `unmaintained = "all"` (the default) instead of `"workspace"`: the advisories CI leg is allowed to fail anyway, and the GUI graph passes with `all` today. BSL-1.0 is OSI-approved and permissive. The drafts carry the two exceptions **commented out** pending the owner's decision under ADR-0004.

### 4. xtask vs just vs cargo aliases

Unchanged from the first report (documented / anecdotal):

- matklad's xtask pattern: an alias plus a workspace binary, with nothing beyond cargo needed.
- Users include rust-analyzer, egui, wgpu, nih-plug and nice-plug (whose plugin bundler is itself an xtask library). bevy uses `tools/`.
- `just` 1.59.0 (2026-10-15) is active; `cargo-make` hasn't released since 2025-01-18.
- Cargo aliases can't chain commands.

**[new]** xtask also hosts `golden` cleanly (§6), keeping the refuse-under-CI rule in one place. Verdict: xtask for every command with logic; no justfile.

### 5. GitHub Actions; Codeberg

| Claim | Evidence | Label |
| --- | --- | --- |
| Standard runners are free for public repos (Linux: 4 vCPU, 16 GB, 14 GB SSD). Cache allowance 10 GB per repo. `ubuntu-latest` = 24.04; 26.04 exists | [billing](https://docs.github.com/en/billing/concepts/product-billing/github-actions), [runners](https://docs.github.com/en/actions/reference/runners/github-hosted-runners), [runner-images](https://github.com/actions/runner-images) | documented |
| Swatinem/rust-cache v2.9.2; setup-rust-toolchain v2.0.0 (sets `CARGO_BUILD_WARNINGS=deny` by default); sccache v0.18.0 (can't cache `bin`/`dylib`/`cdylib`/`proc-macro` crates or incremental builds); taiki-e/install-action v2.87.29; awalsh128/cache-apt-pkgs-action v1.6.3 (used by cpal) | READMEs and releases | documented / anecdotal |
| **[new]** rust-cache's default `env-vars` key prefixes include `CARGO`, so `CARGO_BUILD_WARNINGS` is part of the cache key. Harmless while it stays constant | rust-cache README | documented |
| **[new]** apt cost: public nih-plug runs (2026-09-03, via `gh api`) show "Install dependencies" (`apt-get update` + 11 `-dev` packages, the list Task 1 copied) at **13 s** on ubuntu-22.04 and **29 s** on ubuntu-latest; "Set up Rust toolchain" 9 s. So no apt cache is needed | [nih-plug test.yml](https://github.com/robbert-vdh/nih-plug/blob/master/.github/workflows/test.yml), run 33783235795 | measured (public logs) |
| **[new]** actionlint 1.7.12 + shellcheck 0.11.0 report **0 errors** on the drafted `ci.yml` and on the later-jobs snippet. On the first report's clap-validator sketch, actionlint reported: *calling function "hashFiles" is not allowed here*. `hashFiles` works only in `steps.*` contexts, so that job-level `if:` was invalid | scratch | measured |
| **[new]** The CI step sequence (`rustup toolchain install`, `generate-lockfile`, fmt, clippy, test, check-layers, wasm-check, deny ×4) passes locally with `CI=true CARGO_BUILD_WARNINGS=deny` on 1.99.1. Cold sequence from an empty `target/`: **21.7 s**, 270 MB max single-process RSS (3 jobs, `flock -x`, load 8.7). Not run on GitHub: no repo exists yet | scratch | measured |
| **[new]** clap-validator 0.4.1 asset `clap-validator-0.4.1-127-g152b982-ubuntu-22.04.zip` (1,509,492 bytes, sha256 `49edadcf…ad90`) contains a single `clap-validator-0.3.2-127-g152b982-ubuntu-22.04.tar.gz` (the inner name says 0.3.2), which holds one binary (4.75 MB) linking only libc/libm/libgcc_s; `--version` prints 0.4.1 | scratch | measured |
| **[new]** Headless (`env -u DISPLAY -u WAYLAND_DISPLAY -u XAUTHORITY`) on a copy of researcher 01's `spike_np.clap` (nice-plug + egui editor, debug, 32.9 MB): **44 tests run, 36 passed, 0 failed, 0 warnings, 8 skipped, 0.27 s**, exit 0. `list tests` contains no GUI tests. Per `src/commands/validate.rs` it exits non-zero only when `num_failed > 0`. Options include `--json`, `--only-failed`, `-t/-x` regex filters, `-j`, `--in-process` and `--trace`. The drafted CI step (sha256 check → unzip → tar → validate) runs locally | scratch; [source](https://github.com/free-audio/clap-validator/blob/master/src/commands/validate.rs) | measured / documented |
| Rust blog (2026-09-21): caching `target/` can leak secrets (via Miri), so keep secrets out of cache-writing jobs. GitHub can enforce SHA-pinned actions (2025-08-15). **[new]** Dependabot updates both the SHA and the `# vX` comment of pinned actions ([changelog 2022-10-31](https://github.blog/changelog/2022-10-31-dependabot-now-updates-comments-in-github-actions-workflows-referencing-action-versions/)). `cooldown.default-days` delays version updates; the docs give 3 days as the default inside a `cooldown` block. The cargo ecosystem value is `cargo` | [Dependabot options reference](https://docs.github.com/en/code-security/dependabot/working-with-dependabot/dependabot-options-reference) | documented |
| Codeberg: hosted Forgejo Actions run in limited form (`codeberg-medium`: 4 CPU / 8 GB, *including filesystem writes* / 10 min); hosted Woodpecker needs an approval. nice-plug uses Woodpecker | [docs.codeberg.org/ci](https://docs.codeberg.org/ci/), [actions/meta](https://codeberg.org/actions/meta) | documented / anecdotal |

### 6. Golden and snapshot tests

| Claim | Evidence | Label |
| --- | --- | --- |
| insta 1.49.0 (2026-10-03): `INSTA_UPDATE=auto` means `no` in CI; since 1.48, `CI=true` makes `cargo insta test` act like `--check` | [CHANGELOG](https://github.com/mitsuhiko/insta/blob/master/CHANGELOG.md) | documented |
| **[new]** insta's `Comparator` (1.47+) is `trait Comparator: Send + Sync + 'static { fn matches(&self, reference: &Snapshot, test: &Snapshot) -> bool; fn matches_fully(..) /* defaults to matches */; fn dyn_clone(&self) -> Box<dyn Comparator>; }`, set via `Settings::set_comparator` or `with_settings!`. It could implement tolerant summary comparison, but it has no notion of an optional per-platform snapshot | [comparator.rs](https://github.com/mitsuhiko/insta/blob/master/insta/src/comparator.rs) | documented |
| std's `f32::sin` docs say precision varies by platform and Rust version | [std f32](https://doc.rust-lang.org/std/primitive.f32.html) | documented |
| **[new]** A prototype `enth-golden` harness plus `cargo xtask golden [--update]` (code below) behaves as designed. **Missing file:** panics with a hint. **`CI=true` + `--update`:** refused. **`--update`:** writes pretty TOML (one value per line, for reviewable diffs). **Amplitude change 0.5 → 0.5001:** fails with "render changed on x86_64-linux". **Same change with no hash for the platform:** passes on summary only (tolerance 0.01 dB). **0.5 → 0.6:** fails with `rms_db[0] -7.376 vs expected -8.96` | scratch, 1.99.1 | measured |
| **[new]** The x86_64-linux hash (`880917…5203`) for the prototype case was identical across two independent builds (before and after the crash) | scratch | measured |
| **[new]** On **wasm32-wasip1** under wasmtime 49.0.3 (via `CARGO_TARGET_WASM32_WASIP1_RUNNER`), the same 24,000-sample `f32::sin` render hashed `1f4ef2fe…3aa9`, versus `880917…5203` on x86_64-linux. The summaries agree within 0.01 dB. A single `sin(0.5)` was bit-identical (`3ef57744`) on both, so differences appear only across many values | scratch | measured |
| **[new]** On wasm32-wasip1, `std::env::consts::OS` and `FAMILY` are `""`, even though `rustc --print cfg` says `target_os="wasi"`, `target_env="p1"`. The harness keys platforms with `cfg!(target_os = "wasi")` | scratch | measured |
| **[new]** wasmtime doesn't pass host environment variables to the guest. `--env NAME=VALUE` is needed (e.g. `ENTH_GOLDEN_UPDATE`), and `--dir HOST` maps a directory at the same path | scratch | measured |

Recommendation:

- Use the custom harness for audio goldens: exact SHA-256 over the samples per `<arch>-<os>` key, plus a summary compared with a tolerance. All data is text in `tests/golden/{cases,expected}`, with no WAVs in git.
- Use insta for CLI JSON (as AGENTS.md says).
- `cargo xtask golden --update` writes only the current platform's hash and refuses under `CI`.
- Bumping the toolchain means running `cargo xtask golden`.

### 7. Folder topology

As in the first report (matklad's advice; egui, bevy and nice-plug practice; measured cargo behaviour):

- Rename `crates/dsp` to `crates/enth-dsp` and so on, so directory names match package names.
- Add `default-members` (globs work; spikes are skipped unless `--workspace` is passed).
- A members glob that matches nothing is an error, and so is a non-crate *directory* under a glob; plain files are fine.
- Make `tests/golden` the crate `enth-golden`: golden checks need L3 render + L3 analysis, and check-layers rejects same-layer dev-dependencies.
- Commit `Cargo.lock`.
- Commit `_local/sessions/`: ADR-0006, research.md and roadmap.md cite it; about 248 KB.

**[new]** The `.gitignore` draft was verified with `git check-ignore`. `target/`, `_local/*` other than `sessions/`, all `*.wav` and `.venv/` are ignored; `Cargo.lock` and `_local/sessions/**` are tracked. The first report's version with a trailing `# …` comment on the `*.wav` line left `x.wav` and `tests/golden/ref.wav` **tracked** (measured).

### 8. Release tooling (later)

Unchanged:

- **release-plz** 0.3.172 / action v0.5.142 (2026-10-12). For all-`publish = false` workspaces use `git_only = true`; `version_group` approximates a single shared version.
- **cargo-dist** v0.33.0 (2026-09-11) suits the CLI binary, not plugin bundles.
- Nothing needed in Phase 0.

## Recommendation for Phase 0

1. **Task 1:** use `libjack-jackd2-dev` (the owner has since installed it). Update cargo-deny to 0.20.2. Prefer the prebuilt wasmtime 49.0.3 tarball: verified here (sha256 `40012254…7fbb`; contains `wasmtime` and `wasmtime-min`), and it ran a wasm32-wasip1 test. I didn't measure compiling wasmtime from source; a 694 MB/6 min build for cargo-deny alone suggests it isn't worth it on this laptop.
2. **Task 2:** create exactly the files below. All were verified together in `05/final/` on rustc 1.99.1 and cargo-deny 0.20.2, under `CI=true CARGO_BUILD_WARNINGS=deny`:
   - `generate-lockfile`, `fmt --check` and `clippy -D warnings` pass.
   - `test` passes (13 results), and so do `check-layers` and `wasm-check`.
   - `deny` passes for bans, licenses, sources and advisories.
   - check-layers fails on the deliberate upward dependency.
   - The clippy.toml rules fire on violating code.
   - actionlint + shellcheck report 0 errors.
   - The `.gitignore` behaves as intended under `git check-ignore`.
3. **Task 3 / Task 5 / Phase 1:** append the later CI jobs and add the golden harness (drafts verified, below).
4. **Directory naming:** drafts use `crates/enth-<name>/`. If the owner keeps `crates/<name>/`, only the `[workspace.dependencies]` paths change.

### `Cargo.toml` (root) **[changed: lint list]**

```toml
[workspace]
resolver = "3"
members = ["crates/*", "xtask"]           # add "spikes/*", "tools/*" when the first one exists (an empty glob is an error)
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
unreadable_literal = "allow"              # ported coefficients keep upstream digits verbatim, so they can be checked against the source
inline_always = "allow"                   # #[inline(always)] is deliberate in per-sample DSP code
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

### `crates/enth-<name>/Cargo.toml` (template) and `src/lib.rs`

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

`src/lib.rs`: `//! enth-dsp (see docs/architecture.md).`

### `rust-toolchain.toml`

```toml
[toolchain]
channel = "1.99.1"        # never 1.99.0 (x86_64 FFI bool miscompilation); bump deliberately, then rerun goldens
profile = "minimal"
components = ["rustfmt", "clippy"]
targets = ["wasm32-unknown-unknown", "wasm32-wasip1"]
```

### `.cargo/config.toml`

```toml
[alias]
xtask = "run --quiet --package xtask --"
```

### `clippy.toml` (root)

```toml
# Lint *options* (lint *levels* live in Cargo.toml [workspace.lints]).
# Clippy uses the nearest clippy.toml walking up from each crate, so this applies to every member.
# Crates allowed to do I/O, spawn threads, lock or read clocks (L6-L8 shells) opt out at the crate root:
#   #![allow(clippy::disallowed_methods, clippy::disallowed_types, reason = "shell crate: owns I/O and threads")]
# Tooling directories (xtask/, tools/, spikes/, tests/golden/) carry their own empty clippy.toml instead.
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

`xtask/clippy.toml` (and later `tools/`, `spikes/`, `tests/golden/`):

```toml
# Tooling is exempt from the core-crate rules in the root clippy.toml
# (clippy uses the nearest clippy.toml, so this empty file overrides it for xtask).
```

### `deny.toml` **[changed: `unmaintained`, BSL note]**

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
unmaintained = "all"         # the default; the advisories CI leg may fail without blocking merges
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
  # Needed by the nice-plug-egui GUI stack on Windows (measured 2026-10-19); uncomment once the owner approves BSL-1.0:
  # { crate = "clipboard-win", allow = ["BSL-1.0"] },   # via egui-baseview -> copypasta
  # { crate = "error-code", allow = ["BSL-1.0"] },      # via clipboard-win
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

`MIT-0` isn't in ADR-0004's list; it's strictly more permissive than MIT. Drop it if the owner wants the list verbatim.

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

### `xtask/src/main.rs` (Task 2: `check-layers` + `wasm-check`; verified on 1.99.1)

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

Policy, all judgment:

- Dev- and build-dependencies obey the layers too.
- Layered crates may not depend on exempt tooling; exempt crates may depend on anything.
- wasm-check builds `--lib`, so dev-dependencies are ignored. It also catches hidden RNG: `getrandom` 0.4.3 doesn't build for `wasm32-unknown-unknown` without a backend (documented).

### `.github/workflows/ci.yml` (actionlint + shellcheck clean)

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

Notes:

- The SHAs were resolved from tags on 2026-10-19.
- Under `CARGO_BUILD_WARNINGS=deny`, a *linker* warning also fails CI (measured). If a toolchain or system linker ever prints harmless warnings, add `linker_messages = "allow"` to `[workspace.lints.rust]`, with a reason.
- `cargo clippy … -- -D warnings` duplicates `CARGO_BUILD_WARNINGS` in CI, but keeps the command identical to AGENTS.md for local runs.

### `.github/dependabot.yml` **[new]**

```yaml
version: 2
updates:
  - package-ecosystem: "github-actions"   # also bumps the `# vX.Y.Z` comment next to SHA pins
    directory: "/"
    schedule:
      interval: "weekly"
    cooldown:
      default-days: 7                     # let fresh releases age a week (supply-chain hygiene)
  - package-ecosystem: "cargo"
    directory: "/"
    schedule:
      interval: "weekly"
    cooldown:
      default-days: 7
```

The keys come from the Dependabot options reference. I checked that the YAML parses; GitHub's own validator wasn't run (no repo yet).

### `.gitignore` **[changed: verified with `git check-ignore`]**

```gitignore
/target
*.wav
.venv/
# Research sessions are committed (see Plan check); anything else under _local/ stays local.
/_local/*
!/_local/sessions/
```

Goldens are hashes and summaries, so WAVs render into `target/golden/` on demand. gitignore has **no trailing comments**: a `#` only starts a comment at the beginning of a line. If the owner keeps sessions out of git, replace the last two lines with `/_local/`. Commit `Cargo.lock`.

### Later CI jobs (Task 3 and Task 5/Phase 1) **[changed: no job-level `hashFiles`]**

Append each job to `ci.yml` when its input exists. Verified with actionlint + shellcheck; the clap-validator step script was also run locally.

```yaml
  # Task 3, once spikes/shell exists. clap-validator 0.4.1 has no GUI tests and ran headless locally.
  validate:
    runs-on: ubuntu-24.04
    timeout-minutes: 30
    steps:
      - uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1
        with:
          persist-credentials: false
      - run: rustup toolchain install
      - name: System libraries for plugin, GUI and audio crates
        run: |
          sudo apt-get update
          sudo apt-get install -y --no-install-recommends pkg-config \
            libasound2-dev libgl-dev libjack-jackd2-dev \
            libx11-xcb-dev libxcb1-dev libxcb-dri2-0-dev libxcb-icccm4-dev \
            libxcursor-dev libxkbcommon-dev libxcb-shape0-dev libxcb-xfixes0-dev
      - uses: Swatinem/rust-cache@6323deb102c322ba6fcbdcafc7e3dddab59af2b6 # v2.9.2
        with:
          save-if: ${{ github.ref == 'refs/heads/main' }}
      - run: cargo xtask bundle spike-shell --release
      - name: clap-validator 0.4.1
        env:
          GH_TOKEN: ${{ github.token }}
          CV_ZIP: clap-validator-0.4.1-127-g152b982-ubuntu-22.04.zip
          CV_SHA256: 49edadcfb407ea0dd946ce418300e853fbd2660fa4b0d00e4f19ff8eef24ad90
        run: |
          mkdir -p "$RUNNER_TEMP/cv"
          gh release download 0.4.1 --repo free-audio/clap-validator --pattern "$CV_ZIP" --dir "$RUNNER_TEMP/cv"
          echo "$CV_SHA256  $RUNNER_TEMP/cv/$CV_ZIP" | sha256sum -c -
          unzip -q "$RUNNER_TEMP/cv/$CV_ZIP" -d "$RUNNER_TEMP/cv"
          tar -xzf "$RUNNER_TEMP"/cv/*.tar.gz -C "$RUNNER_TEMP/cv"
          "$RUNNER_TEMP/cv/clap-validator" validate target/bundled/spike-shell.clap --only-failed

  # With the first golden case (Task 5 / Phase 1). Summary-only unless a wasm32-wasi hash is recorded.
  golden-wasm:
    runs-on: ubuntu-24.04
    timeout-minutes: 20
    steps:
      - uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1
        with:
          persist-credentials: false
      - run: rustup toolchain install
      - uses: taiki-e/install-action@8cca0918f8471c33515202b73f56ab27f2bed283 # v2.87.29
        with:
          tool: wasmtime@49.0.3
      - uses: Swatinem/rust-cache@6323deb102c322ba6fcbdcafc7e3dddab59af2b6 # v2.9.2
        with:
          save-if: ${{ github.ref == 'refs/heads/main' }}
      - name: Golden renders on wasm32-wasip1
        run: |
          export CARGO_TARGET_WASM32_WASIP1_RUNNER="wasmtime run --dir $GITHUB_WORKSPACE/tests/golden"
          cargo test --locked --target wasm32-wasip1 -p enth-golden --test golden
```

The bundle path `target/bundled/spike-shell.clap` assumes nice-plug's bundler layout. Check it in Task 3.

### Golden harness (Phase 1 / Task 5) **[new; verified in `05/final-golden/`]**

Add to the root `Cargo.toml`: `"tests/golden"` in `members` and `default-members`, and `sha2 = "0.11.1"` under `[workspace.dependencies]` (pure Rust, MIT/Apache, 2026-10-12). Add `tests/golden/clippy.toml` containing just a comment line.

`tests/golden/Cargo.toml`:

```toml
[package]
name = "enth-golden"
version.workspace = true
edition.workspace = true
license.workspace = true
publish.workspace = true

[dependencies]
# enth-render.workspace = true     (real renders, once they exist)
# enth-analysis.workspace = true   (real feature summaries)
serde.workspace = true
sha2.workspace = true
toml.workspace = true

[lints]
workspace = true
```

`tests/golden/src/lib.rs`:

```rust
//! Golden-render harness: an exact hash per platform, plus a feature summary compared with a
//! tolerance across platforms. `cargo xtask golden [--update]` drives it.
#![allow(
    clippy::print_stderr,
    reason = "test tooling reports missing platform hashes"
)]

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Set by `cargo xtask golden --update`.
pub const UPDATE_ENV: &str = "ENTH_GOLDEN_UPDATE";

/// What a case file records next to the render.
#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Expected {
    pub sample_rate: u32,
    pub frames: usize,
    /// Exact SHA-256 of the samples, keyed by `<arch>-<os>`; only platforms that were updated.
    pub hash: BTreeMap<String, String>,
    pub summary: Summary,
    pub tolerance: Tolerance,
}

/// Feature summary compared across platforms.
#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Summary {
    /// RMS per 10 ms frame, in dBFS, rounded to 0.001 dB.
    pub rms_db: Vec<f64>,
}

/// Allowed absolute difference per summary field.
#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Tolerance {
    pub rms_db: f64,
}

/// `x86_64-linux`, `aarch64-macos`, `wasm32-wasi`, …
/// (`std::env::consts::OS` is empty on `wasm32-wasip1`, so WASI is named explicitly.)
#[must_use]
pub fn platform_key() -> String {
    let os = if cfg!(target_os = "wasi") {
        "wasi"
    } else {
        std::env::consts::OS
    };
    format!("{}-{os}", std::env::consts::ARCH)
}

/// SHA-256 over (sample rate, channels, frames) and the little-endian bits of every sample.
#[must_use]
pub fn hash_samples(sample_rate: u32, channels: u16, samples: &[f32]) -> String {
    let mut h = Sha256::new();
    h.update(sample_rate.to_le_bytes());
    h.update(channels.to_le_bytes());
    h.update((samples.len() as u64).to_le_bytes());
    for s in samples {
        h.update(s.to_bits().to_le_bytes());
    }
    let mut out = String::with_capacity(64);
    for b in h.finalize() {
        let _ = write!(out, "{b:02x}");
    }
    out
}

/// RMS per 10 ms frame in dBFS (mono).
#[must_use]
pub fn summarise(sample_rate: u32, samples: &[f32]) -> Summary {
    let frame = (sample_rate / 100) as usize;
    let rms_db = samples
        .chunks(frame)
        .map(|c| {
            let ms = c.iter().map(|&s| f64::from(s) * f64::from(s)).sum::<f64>() / c.len() as f64;
            let db = 10.0 * ms.max(1e-20).log10();
            (db * 1000.0).round() / 1000.0
        })
        .collect();
    Summary { rms_db }
}

/// Compare a render with its expected file, or rewrite the file in update mode.
///
/// # Panics
/// On any mismatch, on a missing expected file, or when asked to update under `CI`.
pub fn check(expected_path: &Path, sample_rate: u32, samples: &[f32], default_tol: Tolerance) {
    let update = std::env::var_os(UPDATE_ENV).is_some_and(|v| v == "1");
    assert!(
        !(update && std::env::var_os("CI").is_some()),
        "refusing to update goldens under CI; run `cargo xtask golden --update` locally and commit the diff"
    );
    let key = platform_key();
    let hash = hash_samples(sample_rate, 1, samples);
    let summary = summarise(sample_rate, samples);
    let old: Option<Expected> = std::fs::read_to_string(expected_path)
        .ok()
        .map(|t| toml::from_str(&t).expect("expected file is not valid TOML"));

    if update {
        let mut new = old.unwrap_or(Expected {
            sample_rate,
            frames: samples.len(),
            hash: BTreeMap::new(),
            summary: Summary { rms_db: Vec::new() },
            tolerance: default_tol,
        });
        new.sample_rate = sample_rate;
        new.frames = samples.len();
        new.hash.insert(key, hash);
        new.summary = summary;
        let text = toml::to_string_pretty(&new).expect("serialise expected file");
        std::fs::write(expected_path, text).expect("write expected file");
        return;
    }

    let old = old.unwrap_or_else(|| {
        panic!(
            "{} is missing; run `cargo xtask golden --update` and commit it",
            expected_path.display()
        )
    });
    assert_eq!(old.frames, samples.len(), "frame count changed");
    match old.hash.get(&key) {
        Some(h) => assert_eq!(
            h,
            &hash,
            "{}: render changed on {key}",
            expected_path.display()
        ),
        None => eprintln!(
            "note: no {key} hash in {}; summary only",
            expected_path.display()
        ),
    }
    assert_eq!(
        old.summary.rms_db.len(),
        summary.rms_db.len(),
        "summary length changed"
    );
    for (i, (a, b)) in old.summary.rms_db.iter().zip(&summary.rms_db).enumerate() {
        assert!(
            (a - b).abs() <= old.tolerance.rms_db,
            "{}: rms_db[{i}] {b} vs expected {a} (tolerance {})",
            expected_path.display(),
            old.tolerance.rms_db
        );
    }
}

/// `tests/golden` inside the workspace.
#[must_use]
pub fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}
```

`tests/golden/tests/golden.rs`. The stand-in `render` becomes `enth_render::render(...)`, and cases become `<name>.patch.json` plus a performance and a render spec:

```rust
//! One test over every case in `cases/`.

use std::path::Path;

use enth_golden::{Tolerance, check, root};
use serde::Deserialize;

#[derive(Deserialize)]
struct Case {
    frequency_hz: f32,
    seconds: f32,
    sample_rate: u32,
}

/// Stand-in for `enth_render::render(patch, performance, spec)`.
fn render(case: &Case) -> Vec<f32> {
    let frames = (case.seconds * case.sample_rate as f32) as usize;
    let w = std::f32::consts::TAU * case.frequency_hz / case.sample_rate as f32;
    (0..frames).map(|n| 0.5 * (w * n as f32).sin()).collect()
}

#[test]
fn golden_renders() {
    let cases = root().join("cases");
    let mut names: Vec<_> = std::fs::read_dir(&cases)
        .expect("cases dir")
        .map(|e| e.expect("dir entry").path())
        .filter(|p| p.extension().is_some_and(|e| e == "toml"))
        .collect();
    names.sort();
    assert!(!names.is_empty(), "no golden cases");
    for path in names {
        let case: Case = toml::from_str(&std::fs::read_to_string(&path).expect("read case"))
            .expect("parse case");
        let samples = render(&case);
        let stem = path.file_stem().expect("file stem");
        let expected = Path::new(&root())
            .join("expected")
            .join(stem)
            .with_extension("toml");
        check(
            &expected,
            case.sample_rate,
            &samples,
            Tolerance { rms_db: 0.01 },
        );
    }
}
```

Additions to `xtask/src/main.rs`. Add `Some("golden") => golden(&args[1..]),` to the `match`, extend the usage string to `<check-layers|wasm-check|golden [--update]>`, and append:

```rust
fn golden(args: &[String]) -> Result<(), String> {
    let update = match args {
        [] => false,
        [flag] if flag == "--update" => true,
        _ => return Err("usage: cargo xtask golden [--update]".to_owned()),
    };
    if update && std::env::var_os("CI").is_some() {
        return Err("refusing to update goldens under CI".to_owned());
    }
    let mut cmd = cargo();
    cmd.args(["test", "--package", "enth-golden", "--test", "golden"]);
    if update {
        cmd.env("ENTH_GOLDEN_UPDATE", "1");
    }
    let status = cmd
        .status()
        .map_err(|e| format!("cannot run cargo test: {e}"))?;
    if status.success() {
        println!(
            "golden: ok{}",
            if update {
                " (expected files updated)"
            } else {
                ""
            }
        );
        Ok(())
    } else {
        Err("golden: renders differ from tests/golden/expected (see above)".to_owned())
    }
}
```

To record a wasm32-wasi hash locally, pass the update flag through to the guest:

`CARGO_TARGET_WASM32_WASIP1_RUNNER="wasmtime run --dir $PWD/tests/golden --env ENTH_GOLDEN_UPDATE=1" cargo test --target wasm32-wasip1 -p enth-golden --test golden`

## Plan check — what the evidence says should change

Only Rust and a native non-WebView UI are fixed.

1. **phase-0 Task 1 apt line: replace `libjack-dev` with `libjack-jackd2-dev`.** On the owner's machine, `apt-get -s` planned to remove `libjack-jackd2-0` (amd64 + i386) and `libasound2-plugins:i386` (measured). **[new]** The list comes from nih-plug's CI, where `libjack-dev` is harmless on a clean runner. The owner has since installed `libjack-jackd2-dev`; the doc still needs fixing.
2. **Task 1 tools:**
   - cargo-deny 0.20.2, to match CI. Note that `--config` is now a top-level option.
   - **wasmtime:** the prebuilt release tarball (verified), not `cargo install`.
   - **clap-validator:** the release zip contains a tar.gz (layout above).
3. **ADR-0004 / AGENTS.md / phase-0 Task 2 step 7:**
   - Replace `LicenseRef-UFL-1.0` with `Ubuntu-font-1.0`, and scope the font licences to `epaint_default_fonts`.
   - **[changed]** Decide on **BSL-1.0** (OSI-approved, permissive), which the nice-plug-egui stack needs for `clipboard-win` and `error-code` on Windows (measured). The MPL-2.0 `option-ext` I predicted from clack's config does **not** appear in this stack.
   - Note that the fonts crate appears only if egui's `default_fonts` feature is enabled.
4. **Task 2 step 5:** pin `1.99.1` explicitly with `profile = "minimal"`; never 1.99.0.
5. **Task 2 step 3:** add `default-members`, `[workspace.dependencies]` and the final lint set. **[changed]** The set has the two extra allows and no `missing_debug_implementations`.
6. **architecture.md rule 2 / Task 2 step 10:** wasm-check proves buildability only (confirmed on 1.99.1). Enforce "no I/O, threads, clocks, locks" with the root `clippy.toml`. Add to AGENTS.md: every `#[allow]` needs `reason = "…"`.
7. **Task 2 step 11 (CI):**
   - Pin `ubuntu-24.04`, set `CARGO_BUILD_WARNINGS=deny`, and keep in mind that it also fails on linker warnings.
   - Use rust-cache with `save-if`, an advisories leg that is allowed to fail, SHA pins and `--locked`.
   - Add `.github/dependabot.yml` with a cooldown, and commit `Cargo.lock`.
   - **[changed]** CI jobs that depend on later inputs are *added later*, not gated with job-level `hashFiles()` (invalid).
8. **Codeberg:** GitHub stays primary; Codeberg can be a push mirror.
9. **architecture.md topology:** `crates/enth-<name>/` directory names; `tests/golden` becomes the crate `enth-golden`; no WAVs in git. **[changed]** The `.gitignore` draft above, not the first report's broken one.
10. **research-process.md / phase-0 Task 2 step 1:** commit `_local/sessions/`. Before the repo goes public, review briefs for personal details such as hardware and driver state.
11. **Task 3 heads-up:**
    - **[changed]** nice-plug 0.4.2's `nice_export_clap!`/`nice_export_vst3!` don't trigger `unsafe_code = "deny"` on 1.99.1 (measured). Only hand-written `#[unsafe(no_mangle)]` code needs an allow.
    - clap-validator runs headless in CI.
12. **Task 5 heads-up [new]:**
    - Platform keys must come from `cfg!`, because `std::env::consts::OS` is empty on wasm32-wasip1.
    - wasmtime needs `--dir` and `--env` to reach files and flags.
    - The same render hashes differently on x86_64-linux and wasm32-wasi, while summaries agree, so the per-platform-hash plus tolerance design is required, not optional.

Nothing in Task 7 needs changing.

## Gaps — what I still couldn't verify, and the check that settles it

| Gap | Check |
| --- | --- |
| The CI workflow hasn't run on GitHub's runners (no repository yet). Locally it is actionlint/shellcheck-clean, and its steps pass under `CI=true` | Task 2: push, then read the `check` and `deny` job logs (expect about 1–2 min each, warm) |
| `dependabot.yml` checked only for YAML syntax | After pushing: *Insights → Dependency graph → Dependabot* shows the config parsed, or reports an error |
| Warm-cache timing of the CI sequence (cold: 21.7 s) | `flock -x /tmp/enth-quiet.lock` timed rerun in `05/final/` |
| Compiling wasmtime from source (RAM/time) wasn't measured; the prebuilt binary is the recommendation | Only if ever needed: `/usr/bin/time -v cargo install --locked --root <scratch> wasmtime-cli` |
| The bundle path for clap-validator in CI (`target/bundled/<name>.clap`) | Task 3: `cargo xtask bundle spike-shell --release && ls target/bundled` |
| BSL-1.0 approval | Owner decision; then uncomment the two `deny.toml` exceptions and update ADR-0004 |
| Behaviour of `CARGO_BUILD_WARNINGS=deny` on Windows/macOS runners (linker output differs) | When a Windows job is added (Task 5): watch for `linker_messages` warnings |
