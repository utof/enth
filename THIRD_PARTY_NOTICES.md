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

## Bundled assets

### Inter (fonts)

- Upstream: <https://github.com/rsms/inter>, release v4.1 (`Inter-4.1.zip`, sha256 `9883fdd4a49d4fb66bd8177ba6625ef9a64aa45899767dde3d36aa425756b11e`)
- Licence: OFL-1.1 (no Reserved Font Name)
- Copyright: Copyright (c) 2016 The Inter Project Authors (https://github.com/rsms/inter)
- enth files: `crates/enth-ui/assets/fonts/Inter-Regular.ttf` and `Inter-SemiBold.ttf` (unmodified, from `extras/ttf/`), `InterTabular-Regular.ttf` (derived), licence in `crates/enth-ui/assets/fonts/OFL.txt`
- Changes from upstream: `InterTabular-Regular.ttf` is Inter Regular with its `tnum` substitutions frozen into the cmap and subset to digits, `. , : + - −`, space and figure space, renamed "Inter Tabular"; `crates/enth-ui/assets/fonts/freeze_tabular.py` rebuilds it byte for byte. The derived font stays under OFL-1.1.

Expected in P1, each added in the commit that lands its port:

- Signalsmith Basics (`reverb.h` and helpers) and the Signalsmith DSP library — MIT

Later: CloudSeedCore — MIT, with credit to Ghost Note Audio.
