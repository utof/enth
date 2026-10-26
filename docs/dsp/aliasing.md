# Measuring aliasing (`enth-dsp::alias`, the S2a harness)

- Lane: S2a S0d (freeze F0m, [phase-s2.md](../phase-s2.md)); every S2a module lane uses it for its "aliasing measured spectrally" acceptance.
- Code: `crates/enth-dsp/src/alias.rs` (the measure, a deterministic FFT, sweeps); `crates/enth-engine/tests/alias.rs` (the pattern through the engine, calibrated).

A golden hash cannot see aliasing: a module that folds harmonics back below Nyquist is as deterministic as one that does not. So each module lane measures it the way [osc.md](osc.md#aliasing-measured) did, with **research report 02's measure**: alias power over harmonic power in dB, 20 Hz – 20 kHz, from 32,768 samples under a 4-term Blackman-Harris window, every bin within ±8 bins of a harmonic `k·f0` counting as signal. Lower is cleaner.

## The API

| Item | What it does |
| --- | --- |
| `alias_db(x, f0, sample_rate, &Measure::REPORT_02)` | The measure on the **last** 32,768 samples of `x` (render a little longer than that and the attack is skipped); `+inf` for a silent block |
| `Sweep::run(freqs, sr, &m, \|f0\| render(f0))`, `Sweep::run_keys(keys, sr, &m, \|key\| render(key))` | One measure per frequency or MIDI key (measured against `midi_hz(key)`) |
| `sweep.assert_below(bound_db, "what")`, `sweep.worst()`, `sweep.table()` | The lane's bound check (it prints the whole sweep when it fails), the worst point, a table for this page |
| `sine(f, amplitude, sr, frames)`, `log_sweep(lo, hi, points)`, `midi_hz(key)` | Test inputs: a clean `f64` sine, a stepped log sweep, a key's frequency |
| `power_spectrum(x)`, `fft(re, im)` | The pieces, for a lane that needs its own reading |

Everything is `f64` with the pinned `libm` and a fixed loop order, so a measurement is the same on every platform (unlike `rustfft`, which picks its path by CPU). A bound still keeps a margin, because a module's output may legitimately move between engine versions.

## How each kind of module measures

| Module | Input | Fundamental to pass |
| --- | --- | --- |
| Effect with a static nonlinearity (`dist`, the drive of a filter) | A sine (an `osc` sine voice in an engine patch, or `sine`) at a sweep of keys | The sine's |
| Filter (`comb`, `formant`, `ladder`) | The same, at the settings where it is nonlinear or resonant | The sine's |
| Modulated effect (`chorus`, `phaser`) | The same with the modulation depth at 0, or a modulation rate that is a harmonic of the sine (sidebands at `f0 ± rate` count as aliasing otherwise) | The sine's |
| Dynamics (`ott`) | A sine at a level that drives each band | The sine's |
| Generator (`wt`) | The generator itself at a sweep of pitches; with PM from another generator, at a high index | Its fundamental; with a modulator at a ratio `p/q` in lowest terms, `f0 / q` |
| Modulator (`mseg`) | Not an audio path | — |

The engine's sine is a clean input: −93 to −120 dB from key 33 to 105 (`tests/alias.rs`), which is the measure's own floor: the window leaks about −92 dB beyond its main lobe, so nothing measures lower than about −93 dB at low pitches.

## Recording a bound

A lane measures its sweep, records the table in its `docs/dsp/<kind>.md` ("Aliasing" section) and asserts **the worst point plus a small margin** (1–2 dB) in its own test file with `assert_below`, so a later change that adds aliasing fails. Where a research report or a reference gives a ceiling, assert that too, as osc.md does.

## Calibration and an example (measured 2026-10-25, `tests/alias.rs`)

| Case | Measured | Bound |
| --- | --- | --- |
| polyBLEP saw through the engine, keys 69 and 105 | −40.6 and −28.8 dB (osc.md's numbers) | ±1 dB of osc.md |
| Engine sine, keys 33, 57, 81, 105 | −93.3, −97.8, −95.5, −119.8 dB | −90 dB |
| Sine into the filter's drive (24 dB, cutoff 20 kHz), keys 45–93 | −100.5, −93.9, −58.2, −37.3, −25.2 dB (110 Hz – 1.76 kHz) | −23.7 dB |

The driven filter is the pattern an S2a effect follows: its saturator runs at the base rate, so the folded harmonics rise with pitch. `dist`'s ADAA and oversampling exist to push that curve down. `dist` is that pattern pushed down (measured 2026-10-26, a sine at 24 dB of drive, keys 45–105, the worst point of the sweep): the soft clip is −27.3 dB with oversampling off, −48.4 dB at 2× and −70.4 dB at 4×, the hard clip −26.5, −44.9 and −69.4 dB; the tables for all five shapes, the recorded bounds and what ADAA contributes are in [dist.md](dist.md#aliasing).
