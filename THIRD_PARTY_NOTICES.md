# Third-party notices

enth is MIT-licensed. This file lists source code **ported or copied** into enth from other projects, with the notices their licences require. Crate dependencies are not listed here: `cargo deny` checks their licences against `deny.toml`, and their list is generated from `Cargo.lock`.

Rules: [docs/adr/0004-licensing.md](docs/adr/0004-licensing.md) (which licences ported source may carry) and [docs/adr/0006-effects-strategy.md](docs/adr/0006-effects-strategy.md). Each port also has a provenance note in `docs/dsp/<module>.md`.

## Entry format

```markdown
### <Upstream name>

- Upstream: <URL> at commit `<full hash>`
- Licence: <SPDX id, or LicenseRef-STK-4.3>
- Copyright: <upstream copyright line, verbatim>
- Derived enth files: `crates/…`
- Changes from upstream: <e.g. RNG replaced by the engine's seeded PRNG; denormals flushed in code>

<full licence text, verbatim>
```

## Entries

### Signalsmith Basics (reverb)

- Upstream: <https://github.com/Signalsmith-Audio/basics> at commit `369e906e03760ec219d07b06fa06dc4fd099f377` (tag 1.2.1, 2026-09-30), `include/signalsmith-basics/reverb.h`
- Licence: MIT (the file header reads "MIT License - see LICENSE.txt and SUPPORT.txt"; the licence text is the repository's `LICENSE.txt`, below)
- Copyright: Copyright 2022-2025 Signalsmith Audio / Geraint Luff (file header: Copyright 2022+ Signalsmith Audio Ltd. / Geraint Luff)
- Derived enth files: `crates/enth-dsp/src/reverb/mod.rs` (`ReverbSTFX`: parameters and defaults, `configureSTFX`, `reset`, `processSTFX`, `updateFilters`, `getDecayDb`, `updateDelays`), `crates/enth-dsp/src/reverb/delay.rs` (`MultiDelay`) and `crates/enth-dsp/src/reverb/table.rs` (the draws of the five constant seeds)
- Changes from upstream: `std::mt19937` and the libstdc++ `uniform_real_distribution`/`uniform_int_distribution` replaced by a constant table of their 40 draws for the five constant seeds (a test regenerates it from a from-scratch `mt19937`); denormals flushed in code (feedback writes, biquad outputs and the wet input below 1e-15) instead of FTZ/DAZ (`StopDenormals` dropped); `reset()` also clears `delay4`, which upstream forgets, and cancels a running parameter fade; `roomMs` and every other parameter clamped to the ranges of upstream's UI; the 20 filter designs per block computed once per group and only when an input changed (same bits); parameters in real units (percent for the gains, milliseconds, seconds, Hz); the UI metadata (`state()`) and the "ambient" preset not ported; Rust; transcendentals through `libm`.

```text
Copyright 2022-2025 Signalsmith Audio / Geraint Luff

Permission is hereby granted, free of charge, to any person obtaining a copy of this software and associated documentation files (the “Software”), to deal in the Software without restriction, including without limitation the rights to use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies of the Software, and to permit persons to whom the Software is furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED “AS IS”, WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.
```

### Signalsmith DSP library (delay, mix, filters)

- Upstream: <https://github.com/Signalsmith-Audio/dsp> at commit `ca390d28c41ea5108a528e2e81faa4088f2f8ac8` (v1.7.3, 2026-10-13; `delay.h`, `mix.h` and `filters.h` are identical in v1.7.1, the version Basics pins)
- Licence: MIT
- Copyright: Copyright (c) 2021 Geraint Luff / Signalsmith Audio Ltd.
- Derived enth files: `crates/enth-dsp/src/reverb/delay.rs` (`Buffer`/`MultiBuffer`, `Reader::read`, `InterpolatorLagrangeN<7>`), `crates/enth-dsp/src/reverb/mix.rs` (`Hadamard<8>::unscaledInPlace`, `Householder<8>::inPlace`, `StereoMultiMixer<8>`) and `crates/enth-dsp/src/reverb/biquad.rs` (`BiquadStatic`: the high-pass, low-pass, low-shelf and high-shelf designs with the one-sided bandwidth compensation, and the direct-form-I section)
- Changes from upstream: only the parts listed are ported; the Vicanek, cookbook, band-pass, notch, peak and all-pass designs are not; f64 transcendentals through `libm`; the biquad output is flushed to zero below 1e-15; coefficients are separated from the section state so identical designs are computed once.

```text
MIT License

Copyright (c) 2021 Geraint Luff / Signalsmith Audio Ltd.

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

### Signalsmith STFX library (parameter fade)

- Upstream: <https://github.com/Signalsmith-Audio/basics> at commit `369e906e03760ec219d07b06fa06dc4fd099f377`, `include/stfx/stfx-library.h` (`LibraryParam::_libStartFade`, `Block::fade`/`smooth`/`setupFade`, and the fade bookkeeping of `LibraryEffect::process`)
- Licence: BSL-1.0 (the file header reads "Released under the Boost Software License"; the repository's `LICENSE.txt` is MIT and the file ships no Boost text, so the standard text from <https://www.boost.org/LICENSE_1_0.txt> is reproduced below). ADR-0004 allows BSL-1.0 for ported source.
- Copyright: Copyright 2021-2022 Geraint Luff / Signalsmith Audio Ltd
- Derived enth files: `crates/enth-dsp/src/reverb/mod.rs` (`Param`, `Segment`, `Reverb::begin_block` and `Reverb::end_block`: the 20 ms A/B parameter fade)
- Changes from upstream: re-implemented in Rust from the arithmetic of the original (the parameter fade is the same sequence of operations in `f64`, so the output is bit-identical); no other part of the STFX library is used; `StopDenormals` (which sets FTZ/DAZ through MXCSR) is not ported.

```text
Boost Software License - Version 1.0 - August 17th, 2003

Permission is hereby granted, free of charge, to any person or organization
obtaining a copy of the software and accompanying documentation covered by
this license (the "Software") to use, reproduce, display, distribute,
execute, and transmit the Software, and to prepare derivative works of the
Software, and to permit third-parties to whom the Software is furnished to
do so, all subject to the following:

The copyright notices in the Software and this entire statement, including
the above license grant, this restriction and the following disclaimer,
must be included in all copies of the Software, in whole or in part, and
all derivative works of the Software, unless such copies or derivative
works are solely in the form of machine-executable object code generated by
a source language processor.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE, TITLE AND NON-INFRINGEMENT. IN NO EVENT
SHALL THE COPYRIGHT HOLDERS OR ANYONE DISTRIBUTING THE SOFTWARE BE LIABLE
FOR ANY DAMAGES OR OTHER LIABILITY, WHETHER IN CONTRACT, TORT OR OTHERWISE,
ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER
DEALINGS IN THE SOFTWARE.
```

## Bundled assets

### Inter (fonts)

- Upstream: <https://github.com/rsms/inter>, release v4.1 (`Inter-4.1.zip`, sha256 `9883fdd4a49d4fb66bd8177ba6625ef9a64aa45899767dde3d36aa425756b11e`)
- Licence: OFL-1.1 (no Reserved Font Name)
- Copyright: Copyright (c) 2016 The Inter Project Authors (https://github.com/rsms/inter)
- enth files: `crates/enth-ui/assets/fonts/Inter-Regular.ttf` and `Inter-SemiBold.ttf` (unmodified, from `extras/ttf/`), `InterTabular-Regular.ttf` (derived), licence in `crates/enth-ui/assets/fonts/OFL.txt`
- Changes from upstream: `InterTabular-Regular.ttf` is Inter Regular with its `tnum` substitutions frozen into the cmap and subset to digits, `. , : + - −`, space and figure space, renamed "Inter Tabular"; `crates/enth-ui/assets/fonts/freeze_tabular.py` rebuilds it byte for byte. The derived font stays under OFL-1.1.

Expected in P1, each added in the commit that lands its port:

- Julius O. Smith's Faust `compressor_mono` — STK-4.3
- web-audio-api-rs `dynamics_compressor.rs` — MIT, only if code (not just the cross-check) is ported

Later: CloudSeedCore — MIT, with credit to Ghost Note Audio.
