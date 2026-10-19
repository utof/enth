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
