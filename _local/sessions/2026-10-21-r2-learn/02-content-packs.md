# 02 — Content packs and delivery

- **Session:** R2 Learn, 2026-10-21. **Axis 2** of [brief.md](brief.md). Sub-researcher 02.
- **Scratch:** `~/code/enth-scratch/2026-10-21-r2/02/` (`notes.md`; trial crates in `trial/`; the crates.io snapshot in `crates-io-2026-10-21.txt`).
- **Labels:** *measured* (run here today: rustc 1.99.1, x86_64 Linux, 2 build jobs), *documented* (a linked primary source), *anecdotal*. Versions and dates come from the crates.io API (User-Agent `enth-research/0.1`) or the project page, read 2026-10-21.

## The answer in brief

1. **Bundle every first-party lesson.** 60 lessons come to about 0.5 MB raw, 0.2 MB zipped (*measured* per-file sizes, extrapolated). The markdown renderer alone adds 0.9 MB and a TLS stack 2.2 MB. Downloading doesn't keep the plugin light; it only lets packs ship between releases.
2. **Stage 1 needs no network code.** Bundled packs, a user folder for dropped `.enthpack` files, and a "Get more lessons" button. The button opens the GitHub release page through egui's `OpenUrl`, which the vendored egui-baseview already serves with the `open` crate, so it adds no dependency (*measured* from source). It also respects "no networking until GR1" and the no-telemetry rule by construction.
3. **Render with `egui_commonmark` 0.26.0** (no default features; MIT OR Apache-2.0). It came out the same day as egui 0.37.0 and works headless with our pin. Swap its alert colours and emoji for design tokens.
4. **Download inside the plugin is stage 2, the owner's call.** `ureq` 3.4.2 + `rustls` + `rustls-platform-verifier` (the OS trust store) works, and its licences pass on native targets. The crypto provider is the catch: **ring** compiles C and breaks the per-push cross-target guard (*measured*); **graviola** is pure Rust but new, panics on older CPUs and conflicts with valgrind.
5. **Format:** a zip named `.enthpack`, or a folder with the same layout. It holds a JSON manifest `enth.pack/1` listing every file's SHA-256, markdown, `Patch::to_json()` patches, and challenges as codes or challenge JSON. A newer version is refused and named, never ignored. No pre-rendered audio of our own patches.
6. **Integrity:** SHA-256 per file and per pack. Packs a build knows are pinned inside it. An online catalogue needs a minisign signature (`minisign-verify`, MIT, zero dependencies). Turn on GitHub's immutable releases.
7. **Tooling:** `cargo xtask lesson-check` plays each lesson's reference solution through a real `Session`, proves each check fails before it, and computes P1 module coverage, which is GR2's first half. **Lesson-only commits get no CI today** (`ci.yml` ignores `**/*.md`), so they need a `lessons.yml` workflow.
8. **Licences:** lesson text CC BY 4.0, patches and codes CC0, code MIT. The owner decides.

## 1. What exists and constrains this

| Fact | Source |
| --- | --- |
| Factory presets are **embedded** with `include_str!` from `enth-session/build.rs`, because "a plugin has no reliable install location" | [presets.md](../../../docs/presets.md) |
| User presets live in `<Documents>/enth/Presets`, found by hand-written code (`dirs` was rejected: `option-ext` is MPL-2.0). Crash reports go to `<Documents>/enth/Crashes` | presets.md |
| The planned layout is `content/lessons/` ("lesson.toml + markdown + patches; targets rendered on the fly"). `enth-ui` already has empty `learn` and `recreate` features | architecture.md |
| `Challenge.target` is `seed` only in v1; patch targets for lessons are "later variants". Unknown fields are refused and every value is an integer | [challenge.md](../../../docs/challenge.md) |
| `deny.toml` bans openssl ("use rustls if networking is ever needed"); no CDLA, MPL or OpenSSL licence is allowed. The roadmap holds networking until GR1 | deny.toml, roadmap.md |
| `release.yml` is `contents: read`, so no release assets exist yet. `ci.yml` ignores `**/*.md`, `docs/**` and `_local/**` | workflows |
| The plugin's normal graph has no `toml`, `image`, `flate2` or `cc`, but does have `open` 5.4.4 (via egui-baseview) | *measured*, `cargo tree` |

## 2. The pack format

### 2.1 Layout

```
basics.enthpack            # zip (deflate or stored), or an unpacked folder with the same layout
├── pack.json              # manifest enth.pack/1 (built from pack.toml by `xtask pack`)
├── lessons/01-saw/lesson.json, step-1.md, …   # lesson schema: report 03
├── patches/*.patch.json   # exactly Patch::to_json()
├── media/                 # reserved, empty in v1
└── CREDITS.md             # authors and licences, shown in the app
```

- **`.enthpack`, not `.zip`:** Safari's default "Open 'safe' files after downloading" unpacks zips and discards the archive (*anecdotal*: [Apple community](https://discussions.apple.com/thread/1958374); a CIS benchmark has a key to turn it off, [`AutoOpenSafeDownloads`](https://tenable.com/audits/items/CIS_Apple_macOS_13.0_Ventura_v2.0.0_L1.audit:9768bab78d6e93c210209c7792811b1e)). Prior art: an Ableton `.alp` installs by double-click or by dragging it into Live ([Ableton help](https://help.ableton.com/hc/en-us/articles/115001930644), *documented*).
- **Authoring vs shipping:** authors write TOML in the repo (comments, multi-line text); `xtask pack` writes canonical JSON into the pack. The plugin then needs only `serde_json`, which it already has, while the same serde types read TOML behind a feature that only xtask and the CLI use.

### 2.2 Manifest (sketch)

```json
{"schema":"enth.pack/1","id":"basics","version":"1.2.0","title":"Synth basics",
 "authors":["enth contributors"],"license":{"text":"CC-BY-4.0","patches":"CC0-1.0"},
 "requires":{"lesson":1,"challenge":1,"patch":3,"generator":1,"scorers":["sound/1","structure/1"]},
 "covers":["osc","filter","adsr"],
 "files":[{"path":"lessons/01-saw/step-1.md","size":2113,"sha256":"…"}]}
```

### 2.3 Versioning rules

| Field | Rule | Why |
| --- | --- | --- |
| `schema` | A newer one is refused as `pack_too_new` ("needs a newer enth") and listed, not hidden (as presets list problem files). Unknown fields are refused | Same rule as `enth.challenge/1` |
| `requires.*` | The **highest** version the pack uses, for the lesson, challenge and patch schemas, the generator and the scorer ids. The pack loads only if this build has them all; older patches migrate | Generators and scorers are frozen per id, so a check means the same thing everywhere |
| Engine version | **Not** a gate. Checks lean on structure, sound checks keep a margin, and CI re-checks every bundled pack | An engine change may move audio slightly (`engine_differs`) |
| `version` (semver) | A newer install replaces the older one with the same `id`. Progress is keyed by pack, lesson and step ids, which are append-only | Updates mustn't reset progress |

Lesson challenges are codes or challenge JSON, so every machine builds the same target. Patch targets need challenge v2's `Target::Patch`, which is report 03's call.

### 2.4 Audio, images, size

- **No pre-rendered audio of our own patches.** Targets render deterministically, as Recreate's Target ▶ does. A 2 s, 48 kHz, 16-bit stereo WAV is 384 KB, about the size of the whole text curriculum. Recordings, if ever: WAV through `hound` (Apache-2.0, already in the CLI). Not `symphonia`, which is MPL-2.0.
- **No images in v1.** They would need `egui_extras` + `image` + `png` and wouldn't follow the theme. Live diagrams drawn by `enth-ui` (scope, envelope, the highlighted knob) teach better.
- **Size (*measured*):** the 26 existing patches total 57,087 B (2.2 KB each; zipped 25,321 B), and markdown deflates about 2.3×. Estimate: 60 lessons (3 KB of text, 2 patches and 1 KB of metadata each) ≈ 0.5 MB raw, 0.2 MB zipped, against an 11 MB release `.clap`. **Embed the built `.enthpack` with `include_bytes!`** and read it through the same reader as any other pack.

## 3. Markdown in egui

| | `egui_commonmark` 0.26.0 | Our own renderer on `pulldown-cmark` 0.13.4 |
| --- | --- | --- |
| Licence | MIT OR Apache-2.0 (files in the crate, verified) | MIT (verified) |
| Health | One maintainer; 180 stars, 7 issues, 5 PRs ([GitHub](https://github.com/lampsitter/egui_commonmark), 2026-10-21); 581k downloads in 90 days. **Releases follow egui:** 0.25.0 with egui 0.36.0 (both 2026-08-05), 0.26.0 with 0.37.0 (both 2026-10-15) | 58M downloads in 90 days; egui_commonmark is built on it |
| Features | Tables, task lists, footnotes, strikethrough, GFM alerts, link hooks, search ([docs.rs](https://docs.rs/egui_commonmark/0.26.0/egui_commonmark/)) | Whatever we write (a few hundred lines for a step panel) |
| Cost | **+875 KB stripped** (*measured*); 7 crates: backend, `egui_extras` (no defaults), `enum-map`, `memchr`, `pulldown-cmark`, `unicase` | Smaller |

**Trial (*measured*, `trial/md`).** egui 0.37.0 + egui_commonmark with `default-features = false` and `pulldown_cmark`, rendering a lesson with a heading, a table, a task list, an alert, a footnote, a code block and `enth:` links in a 320 px panel, headless:

- builds; `cargo deny` licences and bans pass;
- 74 shapes, about 0.3–0.4 ms of layout per frame (release, no painting);
- the binary is 4,969,456 B stripped, against 4,093,976 B for a plain `ui.label`.

**Integration** (read from the source):

- **Tokens.** Heading sizes come from the `Style`'s Heading and Body text styles, which `Theme::apply` sets. Alert colours are hard-coded RGB and the icons are emoji (❕ 💡 ⚠ …). The editor installs only Inter, so the emoji would draw as missing-glyph boxes. Pass `.alerts(AlertBundle::from_alerts(…))` with colours from `theme/colors.rs` and glyphs from `theme/glyph.rs`.
- **Actions in text.** `CommonMarkCache::add_link_hook(name)` turns a link such as `[Cutoff](enth:control/filter1.cutoff)` (highlight a control) or `[hear it](enth:play/target)` into a flag read after `show()`. Hooks match the link text exactly, so the panel registers each step's `enth:` links first; the step is parsed once with pulldown-cmark (the `trial/packfmt` lint does this).
- **Widgets between paragraphs.** Split the step at fenced blocks tagged `enth` and draw our widget for each (play button, mini scope).
- **Lint.** No HTML, no images, no code blocks (they would render in Inter), `https:` links only, shown with `url_in_tooltip`. Links without a hook open in the browser through egui-baseview's `open::that_detached` (`vendor/egui-baseview/src/window.rs:769`).

## 4. Delivery

### 4.1 Options

| | How | New dependencies | Notes |
| --- | --- | --- | --- |
| **A. Bundled** | `include_bytes!` from a build.rs, as for presets | none | Offline, can't go missing |
| **B. Folder + browser** | "Get more lessons" opens `github.com/utof/enth/releases` (egui `OpenUrl`). The user saves an `.enthpack` into `<Documents>/enth/Packs`; "Open packs folder" opens it the same way; the view rescans when it opens | none | Three steps instead of one. No network code at all. Ableton's `.alp` works the same way |
| **C. In-plugin, ring** | `ureq` 3.4.2 (no defaults: `rustls-no-provider`, `platform-verifier`) + `rustls` 0.23.46 with `ring` | ~25 crates; C | 4.2 |
| **D. In-plugin, graviola** | the same with `rustls-graviola` 0.4.0 | no C | 4.2 |
| **E. OS curl** | run `curl` from a background thread | none | In Windows since build 17063, Dec 2017 ([Microsoft](https://techcommunity.microsoft.com/t5/containers/-/ba-p/382409)), and in macOS; launches a process from inside the DAW. Not tried |

### 4.2 Measurements (`trial/net`, `trial/netgrav` against a std-only `trial/base` of 339,040 B)

| | C (ring) | D (graviola) |
| --- | --- | --- |
| Release build, cold | 1 min 23 s | 39 s |
| Stripped size added | **+2,188,176 B** | **+3,455,944 B** |
| A 1,509,492 B GitHub release asset (302 to `release-assets.githubusercontent.com`) | 200, all bytes | 200, all bytes |
| badssl expired, self-signed, untrusted root | all refused | expired refused |
| `cargo check` for windows-gnu on Linux | **fails** (needs `x86_64-w64-mingw32-gcc`) | passes |
| `cargo check` for aarch64-apple-darwin on Linux | **fails** (needs macOS SDK headers) | passes |
| `cargo deny` with enth's deny.toml | fails **only** on `webpki-root-certs` (CDLA-Permissive-2.0), which `rustls-platform-verifier` pulls for **wasm32 only**; passes without the wasm targets | same |
| Maturity | ring 0.17.14 (2025-03-11): no release in 19 months, but commits June–July 2026 ([commits](https://github.com/briansmith/ring/commits/main)); the unmaintained advisory RUSTSEC-2025-0007 was **withdrawn** ([RustSec](https://rustsec.org/advisories/RUSTSEC-2025-0007)) | graviola 0.4.1 (2026-06-24): README says "very new… exercise due caution". It `assert!`s aes, pclmulqdq, bmi1 and adx on x86_64, and adx breaks under valgrind unless rebuilt with a flag (source) |
| Licence (verified) | ISC (new code) + Apache-2.0 (from BoringSSL) | Apache-2.0 OR ISC OR MIT-0 (SPDX headers; no LICENSE file in the package) |

What follows:

- **Size.** +2.2 MB is about 20 % of the 11 MB plugin. The plugin already links much of std's I/O, so the real increase is smaller (not measured in the plugin).
- **ring breaks the cross-target guard** (Linux runner, `--all-features`). Fix: install `mingw-w64` for the Windows leg and leave a `net` feature out of the macOS leg (`release.yml` builds it on a Mac), or choose graviola. AGENTS.md needs the owner's OK for a dependency that needs a C toolchain; the plugin compiles no C today. aws-lc-rs (rustls's default provider, 1.18.2) also needs a C compiler ([requirements](https://aws.github.io/aws-lc-rs/requirements/index.html)); not tried.
- **Keep CDLA out.** ureq's default features pull `webpki-roots`, which is CDLA (*measured*). Use `platform-verifier` (Security.framework, CryptoAPI, Linux system certificates) under `[target.'cfg(not(target_arch = "wasm32"))'.dependencies]`, or ask the owner for a scoped licence exception.
- **Plugin hygiene** (ureq 3.4.2 source):
  - pass the provider with `TlsConfig::unversioned_rustls_crypto_provider`, never `install_default()`, which sets process-wide state inside the host;
  - set `timeout_global` and cap the body at the catalogue's size; proxy variables are read by `Proxy::try_from_env`;
  - run the download on a short-lived L8 thread, never the GUI or audio thread or under the session lock, and write atomically (a temporary file, then a rename).
- **Privacy.** A click sends one GET with User-Agent `enth/<version>`: no cookies, no query string. GitHub's per-asset `download_count` gives the owner numbers without telemetry.

### 4.3 GitHub releases as the host

- **Limits:** each file under 2 GiB, 1,000 assets per release, "no limit on the total size of a release, nor bandwidth usage" ([docs](https://docs.github.com/en/repositories/releasing-projects-on-github/about-releases)).
- **Digests:** SHA-256 digests for every asset since 2025-06-03 ([changelog](https://github.blog/changelog/2025-06-03-releases-now-expose-digests-for-release-assets/)).
- **Immutable releases**, generally available since 2025-10-28: assets locked, tag protected, a Sigstore attestation ([changelog](https://github.blog/changelog/2025-10-28-immutable-releases-are-now-generally-available/)). **Enable it**, but don't verify attestations inside the plugin (too heavy).
- **Don't use `releases/latest/download/…` for packs** ([docs](https://docs.github.com/en/repositories/releasing-projects-on-github/linking-to-releases)): "latest" is the plugin's latest release.
- **Catalogue:** packs get their own immutable tags (`packs-2026.11.1`). The catalogue `enth.catalogue/1` (id, version, size, SHA-256, URL, `requires`, `tested_with`) and its `.minisig` live in `content/packs/` on main, are reviewed like code, and are fetched from `raw.githubusercontent.com`. Publishing needs a workflow with `contents: write`.

## 5. Integrity

| Layer | Mechanism | Dependency |
| --- | --- | --- |
| A file in a pack | `files[].sha256` and `size`; files the manifest doesn't list are refused | `sha2` 0.11.1 (already pinned) |
| A whole pack | SHA-256 in the catalogue, **embedded in each build** for the packs it knows | — |
| The online catalogue (stage 2) | minisign signature, public key compiled in, secret key kept off CI | `minisign-verify` 0.3.0 (2026-09-25; MIT, verified; zero dependencies) |
| Reading | `rawzip` 0.5.2 (2026-10-04; MIT, verified; zero dependencies) + `miniz_oxide` 0.9.1, all in memory, never extracted | — |

**Trial (*measured*, `trial/packfmt`, no_std + alloc).** minisign + rawzip + miniz_oxide + sha2 + pulldown-cmark build natively and for `wasm32-unknown-unknown`. minisign's test vector verifies and one changed byte fails. A Python-written zip reads back with CRCs checked.

**Finding.** rawzip's `try_normalize` quietly turns `../../evil.txt` into `evil.txt`: no escape, but a silent rename. **Refuse any entry whose normalised name differs from its raw name**, and also duplicates, too many entries, oversize entries, and inflation past the listed size.

The `zip` crate (9.0.5) is the alternative, but it had six releases in 8 days (2026-10-09 to 10-17), and its defaults pull bzip2, zstd (C), AES and lzma.

**Threat model.** A pack is data. The worst cases are a phishing link (https only, URL shown), a huge file (caps), and an extreme patch: the validator refuses out-of-range values, and a loud patch is the same risk a preset carries (the S2 safety limiter covers it).

## 6. Where packs live

| What | Where |
| --- | --- |
| Bundled packs | inside the binary |
| User packs | `<Documents>/enth/Packs/` (`ENTH_PACKS_DIR` overrides), from the presets resolver: next to Presets and Crashes, visible and droppable, no new dependency |
| Progress (report 03) | e.g. `<Documents>/enth/Learn/progress.json`, or the platform data folder: `$XDG_DATA_HOME` (default `~/.local/share`; `XDG_STATE_HOME` is for minor state such as history, [basedir 0.8](https://specifications.freedesktop.org/basedir/latest/)), `%APPDATA%`, `~/Library/Application Support` |

If hand-written paths grow: `etcetera` 0.11.0 (MIT OR Apache-2.0; depends on `cfg-if` and `windows-sys`, so no MPL). The OneDrive Documents gap from presets.md applies here too.

## 7. wasm32 and layers

- Format, manifest checks, zip reading, hashing, minisign and the markdown lint are **pure functions over bytes** and build for wasm32 (*measured*), so they fit the core crate that holds lessons (architecture.md gives lessons to L4; report 03 decides).
- I/O goes through a `PackStore` seam shaped like `PresetStore` (`scan`, `read`, `install`), with `MemStore` for tests and `FsStore` in the plugin.
- The downloader is **L8 only**, behind a `net` feature, `cfg(not(wasm32))`, and off in the light build. A web build would use the browser's `fetch`.
- Nothing touches the audio thread; auditions go through the session's existing path.

## 8. Authoring and CI

**`cargo xtask lesson-check`** runs over every pack in `content/lessons/`:

1. Schemas with unknown fields refused; every listed file present at its size, nothing unlisted.
2. Patches validate and equal `to_json()` byte for byte (as `tests/presets.rs` checks).
3. Codes decode; challenge JSON validates; generator, template and scorer ids exist in this build.
4. Each `enth:` link resolves (the catalogue address, the patch in the pack); no HTML or images; `https` only; a length budget per step.
5. **Playthrough:** from each step's start, apply the reference solution through a headless `Session`. Every check must pass after it and **fail before it**. Sound checks print their margin, as the golden challenges do.
6. **Coverage:** list the module kinds and Basic-tier parameters (`ParamSpec.learn.tier`) each lesson touches, and fail if any P1 module has no lesson.
7. **Hash pinning:** `xtask pack` builds reproducibly (sorted entries, 1980 timestamps, a fixed deflate level, `miniz_oxide` pinned with `=`), and the result must match `content/packs/catalogue.json`.

**CI:** GitHub can't add exceptions to `paths-ignore`, so add **`lessons.yml`** with `paths: [content/lessons/**, content/packs/**, xtask/**]` running `lesson-check` on Linux. Report 03's scripted kittest player sits on top. Friends and LLM-drafted lessons go through the same checker, which names the failing check and its margin.

## 9. Content licences

| Content | Licence | Why |
| --- | --- | --- |
| Lesson text | **CC BY 4.0** | Creative Commons recommends its licences for "software documentation", not software ([CC FAQ](https://creativecommons.org/faq/)); credits shown in the app |
| Patches, factory presets, codes | **CC0-1.0** | People make music with them, so no question of MIT notices on a sound. Relicensing the presets is the owner's call |
| Code, schemas, xtask | MIT | unchanged |

`deny.toml` covers crates only, so `lesson-check` enforces the pack `license` ids (CC0-1.0 or CC-BY-4.0). A `content/lessons/LICENSE` file states the terms for contributions (GitHub's inbound = outbound).

## 10. Risks

| Risk | Mitigation |
| --- | --- |
| egui_commonmark (one maintainer) lags an egui bump | `=` pin, behind our widget; our own pulldown-cmark renderer takes a few hundred lines |
| An engine change makes a lesson's sound check unreachable | `lesson-check` in CI on every engine change, with margins; prefer structure checks |
| An online pack made for an older build | `requires` + `tested_with`; CI re-checks catalogue packs per release |
| TLS advisories over the years (`unmaintained = "all"`) | behind the `net` feature, off in light builds; option B always works |
| A hostile pack | §5 |

## Gaps

- No Windows or macOS run: the OS trust store path is untested; graviola passed only `cargo check`; ring couldn't check without cross toolchains.
- Not measured inside `enth-plugin`: the linked size of TLS and egui_commonmark, and egui_commonmark in the real editor under Xvfb (Inter's coverage of bullets and task boxes needs a kittest snapshot).
- Proxies, captive portals and option E (curl) were not tried. No documented rate limit found for `raw.githubusercontent.com`.
- The GitHub API was rate-limited for this shared IP, so repo health comes from web pages. Safari's unpacking is community-sourced. `cargo deny` ran as 0.19.4 locally; the repo targets 0.20.x.

## Recommendations

1. **Stage 1 (R2, no networking):** `enth.pack/1` as in §2. Bundle all first-party lessons as an embedded `.enthpack` (~0.2 MB); add `<Documents>/enth/Packs` with "Open packs folder" and "Get more lessons" (browser) buttons. No new network dependency.
2. **`egui_commonmark` =0.26.0** (no defaults, `pulldown_cmark`) in `enth-ui` behind `learn`, with a token adapter for alerts, colours and glyphs. Actions as `enth:` link hooks; widgets as `enth` fenced blocks.
3. **A pure, wasm-clean reader:** `serde_json` + `rawzip` =0.5.2 + `miniz_oxide` + `sha2`, a `PackStore` seam, strict entry rules.
4. **`cargo xtask lesson-check`, `cargo xtask pack` and a `lessons.yml` workflow, first**, so authoring (owner, friends, LLM) has a loop from day one.
5. **No pre-rendered audio and no images in v1.**
6. **Licences:** CC BY 4.0 text, CC0 patches and codes (owner decision, also for the factory presets).
7. **Stage 2 (after GR1, with the owner's OK):** a `net` feature in L8 only: `ureq` =3.4.2 without defaults + `rustls-platform-verifier` + an explicit provider, off wasm32. **The owner picks ring** (mature; needs C; fix the guard with mingw-w64 and no `net` in the macOS leg) **or graviola** (no C; new; CPU asserts; valgrind flag). Add a minisign-signed catalogue on main, immutable `packs-*` tags, and `contents: write` for publishing.
8. **Turn on immutable releases for `utof/enth` now.** It costs nothing and protects friend builds too.
