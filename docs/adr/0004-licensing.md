# ADR-0004: Licensing — MIT, permissive dependencies only

- Status: **Accepted**
- Date: 2026-10-18

## Context

- The project should be open source and free, with the fewest restrictions on reuse.
- Many excellent open synths (Vital, Surge XT, Dexed, Odin 2) are GPL. Copying or porting their code would force GPL on us.
- The VST3 SDK has been MIT since version 3.8 (Oct 2025), so VST3 support no longer forces GPL. The older `vst3-sys` Rust bindings are still GPLv3; the `vst3` crate is MIT/Apache.

## Decision

- The project is **MIT**.
- Dependencies must be MIT, Apache-2.0, ISC, BSD-2/3-Clause, Zlib, Unicode-3.0 or CC0-1.0, enforced by `cargo deny check` in CI. Font licences OFL-1.1 and Ubuntu Font Licence 1.0 (`LicenseRef-UFL-1.0`) are also allowed, because egui bundles such fonts by default; they don't affect the code's MIT licence.
- No code copied or ported from GPL projects. Learning from papers, manuals and listening is fine; cite sources in `docs/dsp/`.
- Bundled assets (wavetables, samples, presets, model weights) must be our own or CC0/MIT-compatible, with provenance recorded next to them.

## Consequences

- Anyone can reuse the code, including commercially.
- Some convenient GPL code is off-limits; we implement from papers instead.

## How we'd know we're wrong

A must-have capability only exists under a copyleft licence — then reconsider explicitly with a new ADR.

## Note — 2026-10-19: ported source

`cargo deny` checks crate dependencies only; it can't see code we port into our own files. For ported source ([ADR-0006](0006-effects-strategy.md)):

- Ported source may come from MIT, BSD, ISC, Apache-2.0, Zlib, 0BSD, STK-4.3 (MIT-style) or public-domain code. Anything else, including code with no licence file, is reference-only.
- Check the licence of the actual file, function or module, not the repo's headline licence. Known traps: JUCE modules (AGPLv3 or commercial since JUCE 9, so `juce::Reverb` is off-limits; reimplement Freeverb from the public-domain original instead), DaisySP's LGPL submodule (compressor, Moog ladder, ReverbSc), chowdsp's DSP modules (GPLv3), Faust functions declared GPL (e.g. `vital_rev` and most of `compressors.lib`), `sst-basic-blocks` outside its MIT-marked headers, valib's plugins, nih-plug's `plugins/`, Mutable Instruments' AVR code.
- Every port keeps its notice: an entry in `THIRD_PARTY_NOTICES.md` (upstream URL, commit, SPDX id or `LicenseRef-STK-4.3`, copyright, derived files, full licence text) and provenance in `docs/dsp/<module>.md`.

## Note — 2026-10-19: licences found in the plugin shell dependency tree

The Phase 0 shell spike (nice-plug 0.4.2, nice-plug-egui 0.6.0, egui 0.37) put three licences in front of `cargo deny` that the allow-list did not cover. The owner delegated the call; accepted, with the scope below. `cargo deny --locked check bans licenses sources` passes with `deny.toml` as committed.

| Licence | Crates | Where it applies | Decision |
| --- | --- | --- | --- |
| `BSD-1-Clause` | `nice-assert-no-alloc` (nice-plug's `assert_process_allocs`); `assert_no_alloc` (planned for engine tests, Task 5) | allowed for all crates | Accepted. Permissive and OSI-approved. A one-clause BSD: source redistributions keep the copyright notice and disclaimer; there is no binary-notice or advertising clause. Debug builds of plugin crates link it; release builds compile it but do not use it |
| `Ubuntu-font-1.0` | `epaint_default_fonts` (egui's `default_fonts`, which egui needs to draw any text) | `exceptions` entry for that crate only, next to `OFL-1.1` | Accepted. This is the SPDX id egui has used since 0.31; the `LicenseRef-UFL-1.0` in the Decision section never matches. A font licence, not OSI-approved (SPDX list 3.29.0), so it stays scoped to the one crate that ships the font files; it does not touch the code's MIT licence |
| `BSL-1.0` (Boost) | `clipboard-win`, `error-code` | `exceptions` entries for those two crates only; they are Windows-only dependencies of the egui editor stack (`egui-baseview` → `copypasta` → `clipboard-win` → `error-code`) | Accepted. Permissive and OSI-approved |

The Decision section's list of allowed licences is read with these additions. Anything else new still needs an explicit decision.

## Note — 2026-10-19: BSL-1.0 is allowed for ported source (P1a research)

The P1a research (`_local/sessions/2026-10-19-p1a-headless-core/`, report 02) found that Signalsmith Basics' `stfx-library.h` still carries a Boost Software License header, although the repo's `LICENSE.txt` and the effect headers (`reverb.h`) are MIT. The reverb port needs that file's 20 ms parameter fade to match upstream bit for bit ([ADR-0006](0006-effects-strategy.md)). The owner delegated licence calls; accepted:

- **Ported source may carry BSL-1.0.** It is permissive and OSI-approved, and `deny.toml` already allows it as a scoped exception for two Windows-only crates (the note above). This extends the list in the 2026-10-19 ported-source note; the other rules there (check the actual file, keep the notice) are unchanged.
- The reverb's fade is re-implemented from the file's arithmetic, not copied. It still gets its own `THIRD_PARTY_NOTICES.md` entry with the BSL-1.0 text, to be safe.
- Asking upstream to correct the header is the owner's call; nothing here depends on it.
