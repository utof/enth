# Third-party notices

enth is MIT-licensed. This file lists source code **ported or copied** into enth from other projects, with the notices their licences require. Crate dependencies are not listed here: `cargo deny` checks their licences against `deny.toml`, and their list is generated from `Cargo.lock`. The exception is the crates under `vendor/`: enth carries them in its own tree with local changes, so they are listed with the changes.

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

### Julius O. Smith III — Faust `compression_gain_mono` / `compressor_mono` (compressor structure)

- Upstream: <https://github.com/grame-cncm/faustlibraries> at commit `2edec0537f536eaa130d42c354884d7db971663c`, `compressors.lib` (`compression_gain_mono`, `compressor_mono`, `compressor_lad_mono`; each declares its own licence, the file header being LGPL-2.1+ and most other functions GPL-3.0-only, which are not used)
- Licence: LicenseRef-STK-4.3 (the Synthesis ToolKit 4.3.0 licence, MIT-style, text below)
- Copyright: Copyright (C) 2014-2020 by Julius O. Smith III <jos@ccrma.stanford.edu>
- Derived enth files: `crates/enth-dsp/src/compressor.rs`
- Changes from upstream: hand-written Rust from the function's equations, not Faust compiler output; stereo link by max(|L|,|R|) instead of |L|+|R|; Giannoulis, Massberg and Reiss (2012) quadratic soft knee; make-up gain and dry/wet mix; the follower written in increment form with `f64` coefficients; ratio clamped to at least 1; NaN guard and denormal flush in code. Provenance and equations: `docs/dsp/compressor.md`. (web-audio-api-rs, MIT, Copyright (c) 2021 Otto Rottier, appears only as the source of a gain-computer formula reproduced in a test; no code of it ships, so it has no entry.)

The licence declared by the Faust file for these functions, as shipped in faustlibraries `licenses/stk-4.3.0.md`:

```text
The Synthesis ToolKit in C++ (STK) is a set of open source audio
signal processing and algorithmic synthesis classes written in the
C++ programming language. STK was designed to facilitate rapid
development of music synthesis and audio processing software, with
an emphasis on cross-platform functionality, realtime control,
ease of use, and educational example code.  STK currently runs
with realtime support (audio and MIDI) on Linux, Macintosh OS X,
and Windows computer platforms. Generic, non-realtime support has
been tested under NeXTStep, Sun, and other platforms and should
work with any standard C++ compiler.

STK WWW site: http://ccrma.stanford.edu/software/stk/

The Synthesis ToolKit in C++ (STK)
Copyright (c) 1995--2017 Perry R. Cook and Gary P. Scavone

Permission is hereby granted, free of charge, to any person obtaining
a copy of this software and associated documentation files (the
"Software"), to deal in the Software without restriction, including
without limitation the rights to use, copy, modify, merge, publish,
distribute, sublicense, and/or sell copies of the Software, and to
permit persons to whom the Software is furnished to do so, subject to
the following conditions:

The above copyright notice and this permission notice shall be
included in all copies or substantial portions of the Software.

Any person wishing to distribute modifications to the Software is
asked to send the modifications to the original developer so that they
can be incorporated into the canonical version.  This is, however, not
a binding provision of this license.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND,
EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF
MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT.
IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY
CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT,
TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE
SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.
```

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

### Mutable Instruments Plaits and stmlib (variable shape oscillator, polyBLEP/BLAMP, Hermite interpolation)

- Upstream: <https://github.com/pichenettes/eurorack> at commit `f4cfa7697095a28d292bff758beb51deb49b5a86`, `plaits/dsp/oscillator/variable_shape_oscillator.h`; and <https://github.com/pichenettes/stmlib> at commit `d18def816c51d1da0c108236928b2bbd25c17481`, `dsp/polyblep.h` (`ThisBlepSample`, `NextBlepSample`, `ThisIntegratedBlepSample`, `NextIntegratedBlepSample`) and `dsp/dsp.h` (`InterpolateHermite`)
- Licence: MIT (each file header carries the text below and points to <http://creativecommons.org/licenses/MIT/>)
- Copyright: "Copyright 2016 Emilie Gillet." (`variable_shape_oscillator.h`), "Copyright 2017 Emilie Gillet." (`polyblep.h`), "Copyright 2012 Emilie Gillet." (`dsp.h`); each file lists "Author: Emilie Gillet (emilie.o.gillet@gmail.com)"
- Derived enth files: `crates/enth-dsp/src/osc.rs` (`Shape`: the `VariableShapeOscillator` render loop for saw, triangle and pulse, with the four band-limited step functions; `Phased`: the same one-sample-delayed loop and sync reset for the sine and the wavetable) and `crates/enth-dsp/src/wavetable.rs` (the 4-point Hermite form of `InterpolateHermite`); the module wrapper is `crates/enth-engine/src/modules/osc.rs`
- Changes from upstream: Rust; the waveform is a compile-time constant (`SAW`, `TRIANGLE`, `PULSE`) instead of a run-time `shape` blend, and only those three shapes are ported (no slope, no variable shape, no `master`/`slave` template output); the phase increment and the pulse width ramp linearly over a fixed 32-sample block (`stmlib::ParameterInterpolator` replaced by weights computed at compile time); the hard-sync reset time arrives from the engine's sync bus instead of the oscillator's own master phase (the master writes the same value upstream would compute, bit for bit, and a test checks it); a sync reset also applies to the sine and the wavetable, band-limited with the same step functions; the transition loop is bounded to three passes; the frequency clamp is a quarter of the sample rate (upstream's `kMaxFrequency` of 0.25) for the three analog waves and 0.45 for the sine and the table; pulse width is clamped away from 0 and 1 by the phase increment; no transcendentals in the per-sample path (`+ - × ÷` and comparisons only), and the state cannot become subnormal (the frequency clamp keeps phases and residuals away from zero), which the release-tail tests check. Provenance, deviations and measurements: `docs/dsp/osc.md`.

```text
Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in
all copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
THE SOFTWARE.
```

### nice-plug, nice-plug-core and nice-plug-egui (plugin framework; vendored with local changes)

- Upstream: <https://codeberg.org/RustAudio/nice-plug>; the crates.io packages nice-plug 0.4.2 and nice-plug-core 0.4.2 (built from commit `91ba75d20c43ca0cd21afa802d0ae8c0511f48ed`) and nice-plug-egui 0.6.0 (commit `faa0cd61d249bf7c5103f3072be0c65cf72e9f4e`); sha256 of each `.crate` in `vendor/README.md`
- Licence: ISC. The packages carry no licence file; the text below is the repository's `LICENSE` at the tag `v0.4.2` and sits in each of the three vendored directories
- Copyright: Copyright (c) 2022-2024 Robbert van der Helm
- Derived enth files: `vendor/nice-plug/crates/nice-plug/`, `vendor/nice-plug/crates/nice-plug-core/`, `vendor/nice-plug/crates/nice-plug-egui/` (compiled into the plugin through `[patch.crates-io]`)
- Changes from upstream (one git commit each, `vendor/README.md` lists them): runtime parameter names and `request_param_info_rescan()` (nice-plug, nice-plug-core); `NiceEguiApp::editor_closed()` is called when the editor window is destroyed (nice-plug-egui); in the CLAP wrapper, a note that precedes a splitting parameter or transport event on the same frame is no longer lost under `SAMPLE_ACCURATE_AUTOMATION` (nice-plug); `HostRequests` with `mark_dirty()` (CLAP `mark_dirty`, VST3 `setDirty`) and `request_param_info_rescan()` from outside a context, delivered through `Plugin::host_requests()` (nice-plug, nice-plug-core); `Vst3Plugin::VST3_MIDI_CONTROLLERS`, the MIDI controllers the VST3 wrapper maps to parameters (nice-plug); `ClapPlugin::CLAP_RESTART_ON_RENDER_MODE_CHANGE`, which lets a plugin opt out of the restart on a CLAP render-mode change (nice-plug); `is_main_thread()` compares the OS thread identity (`pthread_self`, `GetCurrentThreadId`, `pthread_main_np` on macOS) instead of `std::thread::current()`, so it never allocates, and under `--cfg enth_rtsan` the CLAP `process()` is a RealtimeSanitizer real-time context (nice-plug); each manifest caps lints with `[lints.rust] warnings = "allow"`

```text
ISC License

Copyright (c) 2022-2024 Robbert van der Helm

Permission to use, copy, modify, and/or distribute this software for any
purpose with or without fee is hereby granted, provided that the above
copyright notice and this permission notice appear in all copies.

THE SOFTWARE IS PROVIDED "AS IS" AND THE AUTHOR DISCLAIMS ALL WARRANTIES WITH
REGARD TO THIS SOFTWARE INCLUDING ALL IMPLIED WARRANTIES OF MERCHANTABILITY
AND FITNESS. IN NO EVENT SHALL THE AUTHOR BE LIABLE FOR ANY SPECIAL, DIRECT,
INDIRECT, OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES WHATSOEVER RESULTING FROM
LOSS OF USE, DATA OR PROFITS, WHETHER IN AN ACTION OF CONTRACT, NEGLIGENCE OR
OTHER TORTIOUS ACTION, ARISING OUT OF OR IN CONNECTION WITH THE USE OR
PERFORMANCE OF THIS SOFTWARE.
```

### baseview and egui-baseview (window and egui integration; vendored with local changes)

- Upstream: baseview 0.4.1, <https://github.com/RustAudio/baseview> (commit `8c6e6bab4b3c10f34b23dc12e878814349578777`); egui-baseview 0.9.0, <https://codeberg.org/RustAudio/egui-baseview> (commit `1f645a35d25fd63784dcf2f91c6d72d467c6036b`); sha256 of each `.crate` in `vendor/README.md`
- Licence: MIT OR Apache-2.0, used under the MIT licence; both licence files stay in the vendored directories
- Copyright: baseview's `LICENSE-MIT` states no copyright line (the package lists its authors in `vendor/baseview/Cargo.toml`); egui-baseview: Copyright (c) 2021 Billy Messenger
- Derived enth files: `vendor/baseview/`, `vendor/egui-baseview/` (compiled into the plugin through `[patch.crates-io]`)
- Changes from upstream (one git commit each, `vendor/README.md` lists them): baseview releases the xkb keymap reference after creating the state; baseview and egui-baseview take the keyboard only while a text field is edited (`WindowContext::set_keyboard_input`, `KeyboardFocus`); each manifest caps lints with `[lints.rust] warnings = "allow"`

baseview's `LICENSE-MIT`:

```text
Permission is hereby granted, free of charge, to any
person obtaining a copy of this software and associated
documentation files (the "Software"), to deal in the
Software without restriction, including without
limitation the rights to use, copy, modify, merge,
publish, distribute, sublicense, and/or sell copies of
the Software, and to permit persons to whom the Software
is furnished to do so, subject to the following
conditions:

The above copyright notice and this permission notice
shall be included in all copies or substantial portions
of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF
ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED
TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A
PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT
SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY
CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION
OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR
IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER
DEALINGS IN THE SOFTWARE.
```

egui-baseview's `LICENSE-MIT`:

```text
MIT License

Copyright (c) 2021 Billy Messenger

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

## Bundled assets

### Inter (fonts)

- Upstream: <https://github.com/rsms/inter>, release v4.1 (`Inter-4.1.zip`, sha256 `9883fdd4a49d4fb66bd8177ba6625ef9a64aa45899767dde3d36aa425756b11e`)
- Licence: OFL-1.1 (no Reserved Font Name)
- Copyright: Copyright (c) 2016 The Inter Project Authors (https://github.com/rsms/inter)
- enth files: `crates/enth-ui/assets/fonts/Inter-Regular.ttf` and `Inter-SemiBold.ttf` (unmodified, from `extras/ttf/`), `InterTabular-Regular.ttf` (derived), licence in `crates/enth-ui/assets/fonts/OFL.txt`
- Changes from upstream: `InterTabular-Regular.ttf` is Inter Regular with its `tnum` substitutions frozen into the cmap and subset to digits, `. , : + - −`, space and figure space, renamed "Inter Tabular"; `crates/enth-ui/assets/fonts/freeze_tabular.py` rebuilds it byte for byte. The derived font stays under OFL-1.1.

Later: CloudSeedCore — MIT, with credit to Ghost Note Audio.
