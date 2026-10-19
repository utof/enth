# 03 — CLI v0 and descriptors

- **Date:** 2026-10-19
- **Sub-researcher:** 03 (P1a research session, `brief.md`)
- **Questions answered:** all six: (1) the CLI v0 command set with inputs, outputs, JSON shapes, exit codes and errors, and what changes from `spike-agent`; (2) crates; (3) descriptor fixes from the probe, prototyped against the trial's failure cases; (4) multi-parameter hints; (5) the G1 agent-eval protocol and harness; (6) the content of `enth guide`.
- **Evidence labels:** *measured* (I ran it, numbers below), *documented* (an official page, a crate's source or metadata, linked and dated), *anecdotal*, *estimate* (my arithmetic on stated assumptions).
- **Prototype:** `~/code/enth-scratch/2026-10-19-p1a/03/proto/` (crate `proto03`, path dependencies on `spike-agent` and `spike-metric`, its own target dir). Raw outputs are the `runs-*.txt` files beside it; the log is `notes.md`. `src/toy.rs` is a copy of `spike_metric::render` with two extra inputs (oscillator start phases and a master gain), bit-identical to the original at the defaults (checked: `proto03 selftest`).
- **Machine:** the owner's i7-9750H, release build, rustc 1.99.1. Load average was 7 to 10 throughout (other sessions running), so **all timings are upper bounds**; counts and descriptor values do not depend on load.

## Answer in brief

1. **The trial's 15/15 was partly a descriptor artefact** *(measured)*. Re-judged with beat-aware descriptors, 13 of the 15 final patches pass, and 3 of those 13 pass within measurement noise. g4 run 3 fails `attack>8ms` (programmed 6 ms; the old descriptor read 38 ms), and g5 run 1 fails `sustain>-4dB` (programmed −6 dB, now read −4.5 dB; the old one read −2.0 dB). Agents passed some goals *because of* the beat bug, e.g. by setting −6 dB to satisfy `> −4 dB`.
2. **Beat-aware envelope: fit the beat out, don't smooth it out** *(measured)*. Measure the beat rate first. Then fit the log envelope's periodic part at that rate, plus a polynomial trend, over the held part, and divide that pattern out of the whole envelope (bridging deep nulls). Against each patch's detune-0 twin, over 300 random patches, all four envelope descriptors agree within tolerance on 64 % of patches (raw RMS: 19 %; a one-beat boxcar, i.e. the "low-passed over a beat" idea: 23 %; synchronous folding: 49 %). Attack is within tolerance on 95 % of patches, release on 98 %, sustain on 81 % (p90 error 2.7 dB), decay on 74 %. A crest (max-hold) envelope can't fix attack: it moves rising edges earlier by half a window.
3. **Beating: a multi-harmonic estimator** *(measured)*. Harmonic h of two detuned oscillators beats at h·Δf. Summing each harmonic's modulation spectrum at h·r makes slow beats measurable and sharper. Within 3 % of theory on 333 of 378 grid cells (old: 198); wrong on 4 (old: 4); absent on 41 (old: 176, now only beats below 0.25 Hz or above 25 Hz). 0 false positives on 400 detune-0 patches. Comparing the same rate on every harmonic against h·rate tells a tremolo LFO from detune beating: 0 false beats on 4 tremolo cases (the old estimator: 1).
4. **Clipping corrupts brightness, not just level** *(measured)*. One trial intermediate patch (resonance 4) peaked at +6.7 dBFS with 2.2 % of samples over full scale. Its centroid read 3,712 Hz clipped against 2,995 Hz with the master 12 dB down. A square wave at resonance 10 read 2,533 Hz against 1,173 Hz. Fix: describe the float render (not the 16-bit file), report `peak_dbfs` above 0, `true_peak_dbtp`, `clipped_pct` and `clipping`, add a `master.level` parameter, and give an exact gain hint (gain is linear).
5. **Hints should solve all failing items at once** *(measured)*. Bounded Gauss-Newton on a squared-hinge objective, using the finite-difference Jacobian `check` already renders, verified by real renders with Broyden updates, then sparsified. A follower that applies this one hint per step passes **5 of 5 goals in a single hint step each** (11 to 41 renders per hint, 0.26 to 1.9 s). The old one-parameter follower passes 4 of 5 (g5 fails after 12 steps). When nothing works, the solver names the blocked item and the parameter at its bound.
6. **Margins with noise** *(measured)*. Every result carries `margin` (signed, in the feature's unit) and `noise`: the largest change over 2 nuisance re-renders, i.e. a common phase shift, plus a different inter-oscillator start phase only when detuned. Results with |margin| ≤ 2 × noise are marked `thin`. Changing the phase *difference* at 0 ct is not a nuisance: it comb-filters the harmonics, and YIN jumped an octave.
7. **`describe` on a WAV:** `render` writes a 32-bit float WAV with an `enth` RIFF chunk holding the performance. This round-trips bit-exact, ffprobe reads it, and hound 3.5.1 skips unknown chunks. Precedence is `--perf` > chunk > inferred note-off; inference found 132 of 143 sustained notes, 110 of them within 30 ms. Otherwise the value is null with a reason.
8. **Crates** *(documented, 2026-10-19)*: clap 4.6.7 (derive), schemars 1.2.3 (MIT; draft 2020-12 by default), insta 1.49.0 with `json` and `filters` plus insta-cmd 0.7.0 (both Apache-2.0), strsim 0.11.1 for `did_you_mean`, hound 3.5.1 to read external WAVs, `std::io::IsTerminal` (stable since 1.70) for TTY detection. Own unit parser: no crate covers dB/ct/st/oct/notes. **Avoid `jsonschema`**: its defaults pull reqwest and rustls with aws-lc-rs, which builds C, and `deny.toml` uses `all-features = true`.
9. **Eval harness:** `cargo xtask agent-eval` drives Claude Code headless: `claude --bare -p … --output-format stream-json --model <id> --max-turns --max-budget-usd --tools Bash,Read --allowedTools "Bash(enth *)" --permission-mode dontAsk --no-session-persistence`, run in an empty temp dir whose only tool is a budget-counting `enth` wrapper. It records the model, Claude Code version, tokens, `total_cost_usd` (a client-side estimate), renders, invalid-command rate and margins. Goals live in `content/agent-evals/goals/*.toml`. Never a merge gate.
10. **Cost** *(estimate)*: about $0.15 per trial with Sonnet 5.5 ($2/$10 per MTok, $0.10 cache reads), so about $9 for 10 goals × 3 trials × 2 tracks. The first run must measure it; the token count per trial is an assumption.
11. **`guide`:** one markdown page (≤ 6 KB, against 5.8 KB of JSON for the spike's). It covers workflow, ops and `--set` sugar, expectation syntax, how to read `margin`/`thin`/`fix`/`blocked`, the gotchas the trial hit (performance fixed by the judge, ranges, clipping, slow beats, clip length for release), exit codes, and a vocabulary table for writing your own expectations.
12. **Plan corrections:** move `content/agent-evals` and `xtask agent-eval` (text track) from R1 into P1a, since G1 needs them. The hint search needs a home at L4/L5 (a minimal `enth-solver`), not the CLI. design.md §7's "0.5 s clips for hints" is wrong for envelope items. design.md §6's "crest envelope" should become "fitted beat removal".

## Findings

### F1. Re-judging the trial's final patches *(measured)*

The 15 final patches from `~/code/enth-scratch/lane-p-trial/run{1,2,3}/`, rendered by the same toy synth, measured by the old descriptors (`spike-agent`, also the trial binary) and by the prototype (`runs-trial-final.txt`, `runs-margins-final.txt`):

| Patch | Programmed | Old: attack / decay / sustain | New: attack / decay / sustain | Detune-0 twin (new) |
| --- | --- | --- | --- | --- |
| g4 bell run 1 | A 10 ms, D 2000 ms, 24 ct | 38 ms / 750 ms / −52.1 dB | 9 ms / 1,837 ms / −51.9 dB | 10 ms / 1,816 ms / −52.1 dB |
| g4 bell run 3 | A 6 ms, D 2000 ms, 24 ct | 38 ms / 752 ms / −51.9 dB | 6 ms / 1,852 ms / −51.7 dB | 6 ms / 1,866 ms / −51.6 dB |
| g5 lead run 1 | S −6 dB, 26.84 ct | 6 ms / 15 ms / **−2.0 dB** | 5 ms / 15 ms / **−4.5 dB** | 5 ms / 16 ms / −4.5 dB |
| g5 lead run 2 | S −4.4 dB, 33.1 ct | 12 ms / 17 ms / −2.8 dB | 7 ms / 19 ms / −3.6 dB | — |
| g5 lead run 3 | S −5.4 dB, 24.6 ct | 7 ms / 17 ms / −2.3 dB | 5 ms / 15 ms / −4.0 dB | — |
| g2 pad run 1 | A 700 ms, S −1 dB, R 1500 ms, 6 ct | 633 ms / 473 ms / −3.7 dB; R 1,842 ms | 593 ms / 0 / −1.3 dB; R 1,741 ms | — |

- The new readings equal the twin's: the beat no longer moves them. The remaining gap to the programmed value is the same with or without detune. For g5, sustain reads −4.5 dB for a programmed −6 dB because the 8 ms RMS window blunts a decay spike with a 10 ms time constant. That is definitional, not beat-induced.
- **Judged against the goals:** g4 run 3 fails (`attack>8ms`, read 6 ms, margin −2 ms). g5 run 1 fails (`sustain>-4dB`, read −4.5 dB). g4 runs 1 and 2 pass `attack>8ms` at 9 ms with noise 1 to 2 ms (thin). g5 run 3 passes `sustain>-4dB` at −4.0 dB (thin). The other 10 pass with clear margins.
- g4 was the probe's "beat delays the peak" case: the old attack reading was about 1/(4 × beat rate), here ≈ 34 ms for a 7.3 Hz beat, whatever `amp.attack` said. g5 run 1 shows an agent learning the artefact: its log sets `amp.sustain=-6dB` to satisfy `sustain>-4dB`.

### F2. Beat-aware envelope: four candidates *(measured)*

**Test.** 300 random toy patches (wave, note A2–A5, cutoff, resonance, filter envelope, ADSR, 3 s clip, 2 s hold), each rendered with detune 5, 8, 12, 20, 30 or 45 ct and with detune 0. The detune-0 twin is the reference. Tolerances: attack within max(5 ms, 25 %), sustain within 1.5 dB (when the twin is above −40 dB), decay and release within 20 %. Same definitions in every variant (see the Recommended design). From `runs-beatinv-*.txt`:

| Variant | Attack ok | Sustain ok (p50 / p90 error) | Decay ok | Release ok | All four ok |
| --- | --- | --- | --- | --- | --- |
| Raw 8 ms RMS (spike-agent) | 72 % | 28 % (2.9 / 7.2 dB) | 40 % | 93 % | 19 % |
| Boxcar of one beat period on power ("low-passed over a beat") | 69 % | 49 % (1.5 / 6.3 dB) | 38 % | 41 % | 23 % |
| Fold (synchronous average over the beat period) | 87 % | 60 % (0.8 / 5.2 dB) | 62 % | 94 % | 51 % |
| **Fit (recommended)** | **95 %** | **81 % (0.5 / 2.7 dB)** | **74 %** | **98 %** | **64 %** |

The Fit row is the final configuration (`runs-beatinv-4.txt`, rerun after the last estimator change: 64.3 %). How the variants were tuned, in order (each row of `runs-beatinv-2..4.txt`):

- **Boxcar** removes a stationary two-tone beat exactly: the power of a two-tone beat is A² + B² + 2AB cos(…), and a boxcar of one period nulls it. But it smears every transient by a whole period (667 ms for a 1.5 Hz beat), which destroys attack and release. The fixed 100 ms max-hold "crest" envelope of `spike-metric` has the opposite flaw: dilation moves rising edges earlier and falling edges later by half the window.
- **Fold** needs at least 2 periods in the held part. Its first fallback for slow beats (a boxcar of 2.2 s) wrecked the pad (attack 482 ms, release 2,500 ms).
- **Fit** works from 1 period. Taking it below 1 period (0.6) gave sustain errors of up to 40 dB on 5 ct at C2–C3. Bridging nulls (pattern below −6 dB → interpolate the level in dB) and a robust peak (max of a 5 ms moving average) added 7 points. A sustain read by a straight-line fit in dB over one beat period added 3. A decay threshold in dB with a hold-off against short dips added 6.
- **Remaining failures:** beats slower than one period in the held part (5 ct below about 250 Hz, uncorrected and flagged); triangle waves, whose near-sinusoidal partials beat to deep nulls; and decays that settle within residual ripple of the sustain level.

**Calibration** on non-beating sounds (`runs-audit-final.txt`, saw A3, 6 kHz cutoff). With 25 ct (3.2 Hz beat), the new column is unchanged:

| Programmed | Old (0 ct) | New (0 ct) | Old (25 ct) | New (25 ct) |
| --- | --- | --- | --- | --- |
| attack 2 / 10 / 100 / 500 ms | 5 / 11 / 90 / 441 | 4 / 10 / 88 / 424 | 97 / 97 / 101 / 714 | 4 / 10 / 88 / 426 |
| sustain 0 / −6 / −20 dB | −0.5 / −6.1 / −20.0 | −0.1 / −5.5 / −19.1 | −6.2 / −8.0 / −20.6 | −0.1 / −5.4 / −19.0 |
| decay 500 ms (S −6 / −20 / off) | 222 / 272 / 279 | 249 / 358 / 472 | 115 / 249 / 258 | 249 / 353 / 492 |
| release 100 / 500 / 1000 ms | 114 / 571 / 1139 | 116 / 582 / 1166 | 131 / 578 / 1180 | 117 / 582 / 1166 |

The new decay is a different (documented) quantity from the old: a threshold in dB instead of "90 % of the way in amplitude". So `decay_ms` values change meaning between the spike and P1. Goals written against the spike's numbers need re-checking.

### F3. Beating estimator *(measured)*

Grid: notes C2, C3, A3, C5, A4, C6 × detune 3, 5, 8, 12, 20, 30, 50 ct × saw, square, triangle × three shapes (held; bell with sustain off; filter envelope at cutoff 600 Hz), 378 cells. Theory is f0·(2^(d/2400) − 2^(−d/2400)) (`runs-beatacc-3.txt`):

| | Within 3 % | Wrong | Absent |
| --- | --- | --- | --- |
| Old (fundamental only, 1 to 25 Hz) | 198 | 4 | 176 |
| New (harmonic sum, 0.15 to 25 Hz, agreement check, tremolo check) | **333** | 4 | 41 |

- **Absent cells:** 3 ct at C2/C3 (0.11 and 0.23 Hz) and 50 ct at C6 (30 Hz, above the band). These are correct "can't measure here" answers.
- **False positives** on 400 random detune-0 patches: 0 for both (`runs-falsepos.txt`).
- **Tremolo** (an amplitude LFO applied after rendering): the old estimator reported a 3 Hz tremolo of depth 0.8 as 3.0 Hz beating. The first version of the new one reported a 4 Hz tremolo at C3 as 4.03 Hz beating. With the same-rate-versus-h·rate test, 0 of 4 tremolo cases report beating, and the 20 ct control still reads 2.54 Hz (`runs-tremolo-2.txt`). The same test can feed `modulation.amp_lfo`. That is an inference: amp-LFO reporting was not built.

### F4. Clipping and level *(measured)*

`runs-clip-final.txt`: float render before the 16-bit clamp, with the master at 0, −6 and −12 dB.

| Patch | Master | Float peak | Samples ≥ 0 dBFS | Centroid | Old sustain | New sustain |
| --- | --- | --- | --- | --- | --- | --- |
| g5 run 3 step 2 (saw, Q 4, 4.5 kHz) | 0 dB | +6.7 dBFS | 2.2 % | 3,712 Hz | −1.67 dB | −0.13 dB |
| same | −12 dB | −5.3 dBFS | 0 | 2,995 Hz | −2.44 dB | −0.44 dB |
| square, Q 10, 1.5 kHz | 0 dB | +8.8 dBFS | 27.5 % | 2,533 Hz | −0.61 dB | −2.29 dB |
| same | −12 dB | −3.2 dBFS | 0 | 1,173 Hz | −1.87 dB | −1.99 dB |

- The trial's final patches peaked between −0.5 and −4.5 dBFS, so none of them clipped. The clipping the trial saw happened at intermediate steps.
- The spike measured on the clamped 16-bit samples. So `peak_dbfs` could never exceed 0, and the distortion leaked into brightness: +24 % and +116 % in these two cases.
- The new sustain is much less sensitive to clipping, because the robust peak and the line fit both average.

### F5. WAV with a performance *(measured, documented)*

- **Writing:** `src/wavmeta.rs` writes a mono IEEE-float WAV (format 3) with an `enth` chunk before `data` holding `{"schema":"enth.wavmeta/1","perf":{…},"patch_hash":…}`. Read back by our parser: samples bit-equal, metadata intact. `ffprobe` (system 6.x) reads it as `pcm_f32le`, 48 kHz, 3.000 s *(measured)*.
- **External readers:** hound 3.5.1 skips unknown chunks. Its own test `read_wav_skips_unknown_chunks` is in [src/read.rs at v3.5.1](https://github.com/ruuda/hound/blob/v3.5.1/src/read.rs) *(documented, 2026-10-19)*.
- **Note-off inference** for WAVs without metadata: the latest "knee" where a held level (slope above −6 dB/s over ≥ 100 ms) turns into a fall ≥ 30 dB/s steeper that continues for 15 dB, with fit residuals below 1.5 dB. Tested on 200 random patches with hold 0.8 to 2.6 s (`runs-wav.txt`):
  - Of 143 patches still sounding at note-off: found 132 (110 within 30 ms); median error 3 ms, p10 −117 ms (an earlier decay knee wins), p90 9 ms.
  - Of 57 patches already decayed: no knee reported on 37. A knee found there is a decay knee; sustain then reads near −80 dB, which is harmless.

### F6. Margins and noise *(measured)*

- **Margin:** distance to the nearest edge of the accepted interval, positive inside, in the feature's unit; for scale features also as a percentage.
- **Noise:** the largest change of the reading over 2 nuisance renders. In the toy this means start phases (0.25, 0.81) and (0.6, 0.1) when the oscillators are detuned. At 0 ct I used a common shift only, (0.25, 0.62) and (0.6, 0.97): a phase *difference* at 0 ct is a timbre change. With the difference changed, the fundamentals nearly cancelled, and YIN reported the octave on every g1 and g3 patch (noise 1,200 ct).
- **Results on the trial patches** (`runs-margins-final.txt`):
  - Noise is small where the descriptor is sound: beating 0.0 Hz, brightness 0.2 to 7 Hz, sustain 0.1 to 0.2 dB, decay up to 17 ms, attack 1 to 2 ms.
  - Thin results: g4 attack (9 ms against > 8 ms) and g5 run 3 sustain.
  - An old-descriptor margin would have looked comfortable: 38 − 8 = +30 ms on g4 attack.
- **Cost:** 2 renders plus descriptors per `check`, about 50 ms at the toy's speed.

### F7. Multi-parameter hints *(measured)*

Prototype `src/solver.rs`, measured with the beat-aware descriptors.

**Followers** start from the neutral patch with each goal's performance, apply the hint, re-check, and stop at a pass or after 12 steps (`runs-follow-final.txt`, `runs-follow-old.txt`):

| Goal | Old one-parameter follower (spike-agent hints, old descriptors) | One-parameter, new descriptors | **Multi-parameter, new descriptors** |
| --- | --- | --- | --- |
| g1 pluck | pass, 1 step | pass, 2 steps (23 renders) | pass, **1 step** (16 renders, 0.27 s) |
| g2 pad | pass, 2 steps | **fail** (brightness and sustain stuck) | pass, **1 step** (13 renders, 0.65 s) |
| g3 bass | pass, 1 step | pass, 1 step | pass, **1 step** (13 renders, 0.26 s) |
| g4 bell | pass, 2 steps | pass, 3 steps (74 renders) | pass, **1 step** (43 renders, 1.86 s) |
| g5 lead | **fail** after 12 steps | pass, 2 steps (53 renders) | pass, **1 step** (43 renders, 1.18 s) |

Renders include the follower's own re-check renders. For g4 and g5, 26 to 30 of the 41 hint renders are the wide probe that makes an absent `beating` appear (detune 0 at the start). Ordering that probe by `ParamSpec.perceptual` (detune → `detune.beating_hz`) would cut it to 1 to 4 renders. That is an inference, not built.

**Single hint calls on trial-like states** (`runs-cases-final.txt`, `runs-cases-infeasible.txt`):

- g5 run 1 final, failing `sustain>-4dB`: moves `amp.sustain −6 → −3.3 dB` and `amp.release 150 → 180 ms`, all pass, 11 renders, 0.40 s. Sparsification had removed five moves of under 3 % of their ranges.
- g4 with decay 400 ms and detune 30 ct, failing decay, duration and both beating bounds: `osc.detune 30 → 22.9 ct`, `amp.decay 400 → 648 ms`, `amp.sustain −60 → −36.6 dB`, all pass, 42 renders, 1.57 s. This is the probe's "lower detune for decay pushes beating out of its window" conflict, solved at once.
- g3, too bright with low sustain and a long release: 3 moves, all pass, 11 renders. The release had been a truncated lower bound with zero slope. Extrapolating the release (a line in dB over the observed tail) fixed it.
- An infeasible list (`sustain>-3dB, duration<500ms` with a 1.5 s hold): the solver fixes sustain and reports `duration<500ms` **blocked**, with `amp.release` at its minimum. In the CLI, the hint text adds that a held note lasts at least `perf.hold`.
- The conflict detector (cosine below −0.3 between items' needed directions in parameter space) found no conflicts in these cases. Its first version flagged spurious pitch-versus-attack conflicts. It needs tests on real conflicts before it ships.

**Cost per hint call:** a probe of one render per continuous parameter (9 here), up to 6 verification renders, and 1 sparsification render. That is 11 to 16 renders when every item is measurable, and up to 41 with the unguided wide probe. At about 30 ms per render + describe (load 10), a typical call takes 0.25 to 0.65 s single-threaded *(measured)*.

### F8. Crates *(documented; crates.io API, GitHub and local registry sources, 2026-10-19)*

| Need | Choice | Version (date) | Licence (verified from the file) | Health | Notes |
| --- | --- | --- | --- | --- | --- |
| Argument parsing | [clap](https://crates.io/crates/clap) derive | 4.6.7 (2026-09-14) | MIT OR Apache-2.0 (LICENSE-MIT, LICENSE-APACHE in the 4.6.7 source) | 257 M recent downloads | Already used by the spikes; MSRV 1.85. Keep `try_parse` → `enth.error/1` mapping as in spike-agent |
| JSON Schema output | [schemars](https://crates.io/crates/schemars) | 1.2.3 (2026-10-12) | MIT (LICENSE on master; 1.2.2 source identical) | Pushed 2026-10-12 | `impl Default for SchemaSettings` returns `draft2020_12()` (src/generate.rs). Honours `#[serde(…)]`. Feature `preserve_order` if property order matters |
| Snapshot tests | [insta](https://crates.io/crates/insta) + [insta-cmd](https://crates.io/crates/insta-cmd) | 1.49.0 (2026-10-03); 0.7.0 (2026-06-14) | Apache-2.0 (both LICENSE files) | Active | Features `json` (`assert_json_snapshot!`), `filters` (`Settings::add_filter`, for paths and timings), `redactions`. `assert_cmd_snapshot!` records stdout, stderr and exit code |
| Did-you-mean | [strsim](https://crates.io/crates/strsim) | 0.11.1 (2024-04-02) | MIT | Stable, already a clap dependency | Or keep spike-agent's 25-line edit distance with suffix matching (`cutoff` → `filter1.cutoff`), which strsim alone does not do |
| TTY detection | `std::io::IsTerminal` | stable since [1.70.0](https://doc.rust-lang.org/std/io/trait.IsTerminal.html) | — | — | No crate. `is-terminal` 0.4.17 is superseded |
| Read external WAVs | [hound](https://crates.io/crates/hound) | 3.5.1 (2023-09-25) | Apache-2.0 | Repo pushed 2026-08-09, not archived; no release since 2023 | Reads int 8/16/24/32 and float, skips unknown chunks. CLI only (L8). Write our own float WAV in `enth-render` (bytes only, no I/O) |
| Unit parsing | **own** (`enth-params`) | — | — | — | Searched crates.io for unit, decibel and frequency parsers: `uom` (typed quantities, no string parsing), `humantime` (durations only), `human-units`, `clap-num` (SI numbers) don't cover dB, cents, semitones, octaves, % and note names. spike-agent's `units.rs` (258 lines, tested) is the base |
| Validate outputs against schemas | **don't add** [jsonschema](https://crates.io/crates/jsonschema) | 0.59.0 (2026-10-16) | MIT | Active | Default features pull `reqwest` and `rustls` with `aws-lc-rs` (C build), and `deny.toml` sets `[graph] all-features = true`, so even `default-features = false` would put optional dependencies in front of cargo-deny (inferred, not run). Validate by typed round-trip instead (see the design) |

### F9. Driving an agent for evals *(documented, fetched 2026-10-19)*

- **Versions:** Claude Code here is 2.1.296; npm latest is 2.1.303. The Python Agent SDK `claude-agent-sdk` is 0.2.168 (MIT on PyPI, 2026-10-15); the TS SDK is 0.3.303. The SDK wraps the same CLI.
- **Non-interactive runs** ([headless](https://code.claude.com/docs/en/headless)):
  - `claude -p` with `--output-format json|stream-json`. The result carries `result`, `session_id`, `usage`, `total_cost_usd` and per-model `modelUsage`.
  - `--json-schema` returns `structured_output`.
  - `--bare` skips hooks, skills, plugins, MCP, auto memory and CLAUDE.md. It is "recommended mode for scripted and SDK calls" and needs `ANTHROPIC_API_KEY`, because it ignores the subscription login.
  - Exit code is non-zero on failure.
- **Flags** ([CLI reference](https://code.claude.com/docs/en/cli-reference)):
  - `--max-turns`, `--max-budget-usd` (checked against the client-side estimate; spend can pass it).
  - `--model`, `--fallback-model`.
  - `--tools` (restricts the built-in tools), `--allowedTools` (permission rules such as `Bash(enth *)`), `--disallowedTools`.
  - `--permission-mode dontAsk` (denies anything that would prompt), `--permission-prompts none`.
  - `--no-session-persistence`, `--append-system-prompt-file`, `--setting-sources`, `--strict-mcp-config`.
  - `--restricted`, described as for "an evaluation harness" on a shared machine: it removes command tools unless named in `--tools`, confines file tools to the working dirs, and loads only managed settings and `--settings`.
  - The reference lists **no seed or temperature flag**, so agent runs are not reproducible.
- **Costs** ([cost tracking](https://code.claude.com/docs/en/agent-sdk/cost-tracking)): `total_cost_usd` and `costUSD` are client-side estimates from a bundled price table. Read output tokens from the result message, because per-step `output_tokens` is a placeholder.
- **Subscription runs without `--bare`** ([env vars](https://code.claude.com/docs/en/env-vars)): set `CLAUDE_CODE_DISABLE_CLAUDE_MDS=1`, `CLAUDE_CODE_DISABLE_AUTO_MEMORY=1` and `CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1`, and optionally point `CLAUDE_CONFIG_DIR` at a clean directory. This keeps the owner's own CLAUDE.md, about codebase-memory tools, out of the agent's context.
- **Eval practice** ([Anthropic, "Demystifying evals for AI agents", 2026-01-09](https://www.anthropic.com/engineering/demystifying-evals-for-ai-agents)):
  - Task/trial/grader terms; several trials per task (its examples use 3).
  - pass@k (at least one success in k) against pass^k (all k succeed).
  - Grade the outcome, not the path.
  - Start every trial from a clean environment.
  - Read transcripts to check the graders.
  - Track cost, tokens, latency and error rates.
- **Prices** ([pricing](https://platform.claude.com/docs/en/about-claude/pricing)), per MTok, input / output / cache read:
  - Sonnet 5.5: $2 / $10 / $0.10.
  - Opus 5.5: $4 / $20 / $0.20.
  - Haiku 5.5: $0.10 / $0.50 / $0.01 (prompts up to 100k tokens).
  - The Batch API halves prices but cannot drive an interactive agent loop.

### F10. What the trial agents needed *(measured from the logs; transcripts were not kept)*

- They wrote 3 to 10 `--set key=value` pairs per `step`. Not one of them used ops JSON, which didn't exist in the spike. Keep `--set` as sugar.
- Every one of them ended with the judge's `check`. Each `step` re-sent the full expectation list.
- One out-of-range error (`amp.decay=2500ms`, maximum 2000) cost a step in run 2.
- `describe --explain` was used once (run 3, g4) when the numbers surprised the agent.
- The probe's five problems (beat-corrupted envelope, one-parameter hints, clipping without master gain, WAV without hold, range surprises and thin margins) are the gotchas the guide must cover. Items 1 to 4 now have fixes.

## Recommended design

### 1. Output conventions (all commands)

- **Envelope:**
  - One JSON object on stdout per call; JSONL only for streams.
  - Keys sorted (serde_json's default `Map` is a BTreeMap); numbers rounded to 4 significant digits.
  - Every object carries `"schema": "enth.<cmd>/<n>"`.
  - stderr is free text for humans and never needed by an agent.
- **Format:** JSON when `!std::io::stdout().is_terminal()`, otherwise a table. `--format json|table` overrides; `NO_COLOR` is respected.
- **Versioning:**
  - A breaking change (field removed, renamed or retyped, or its meaning changed, like `decay_ms` between the spike and P1) bumps `/n` and needs a snapshot update.
  - An additive optional field also updates the snapshots. Whether it bumps `/n` is an owner decision: AGENTS.md today says any shape change bumps, and I recommend keeping that while the CLI is v0.
  - `enth schema <name>` prints the JSON Schema (draft 2020-12, from schemars) for each `schema` id.
- **Types are the schema:**
  - Each output is a Rust struct with `Serialize + Deserialize + JsonSchema` and `#[serde(deny_unknown_fields)]`.
  - Tests snapshot the CLI output (insta-cmd), deserialize it back into the struct (which catches drift between the code and the schema without `jsonschema`), and snapshot `enth schema <name>` itself, so a schema change shows in review.
- **Determinism:** fixed seed, 48 kHz, internal block 64, default performance from the template. Descriptor numbers come from the float render, not the file.

### 2. Commands (CLI v0)

| Command | Inputs | Output `schema` and main fields | Exit |
| --- | --- | --- | --- |
| `enth guide [--topic expect\|ops\|descriptors]` | — | `enth.guide/1`: `{text: "<markdown>", version}` | 0 |
| `enth schema [name]` | none: list; name: one of `patch`, `ops`, `perf`, `describe`, `check`, `step`, `error`, … | `enth.schema/1`: `{names: […]}` or the JSON Schema with `$id: "enth.check/1"` | 0, 2 |
| `enth modules` | — | `enth.modules/1`: `{modules: [{kind, role: generator\|modulator\|filter\|fx, summary, params: n}]}` | 0 |
| `enth params <kind>` | module kind or instance (`filter1`) | `enth.params/1`: `{kind, params: [{key, unit, min, max, default, scale: log\|linear\|stepped\|enum, values?, modulatable, effect, moves: "spectrum.centroid_hz"}]}`. Range first in the table view (probe finding 5) | 0, 3 |
| `enth new --template <t> --out p.patch.json [--perf …]` | template name (`--list` lists them) | `enth.new/1`: `{wrote, template, perf, next}` | 0, 3 |
| `enth apply p --ops '<json>' \| --set k=v[,…] [--out p2] [--dry-run]` | ops array, or `--set` sugar that becomes `set` ops | `enth.apply/1`: `{wrote\|null, dry_run, applied: [{op, path, from, to}], inverse: [ops]}` | 0, 3, 5 |
| `enth get p [addr…]` | addresses (`filter1.cutoff`, `routes`, `meta`) | `enth.get/1`: `{values: {addr: "900Hz"}}` (unit strings; `--plain` for numbers) | 0, 3, 5 |
| `enth diff a b [--describe]` | two patches | `enth.diff/1`: `{ops: [a → b], describe?: {feature: {a, b}}}` | 0, 3, 5 |
| `enth render p [--perf perf.json \| --note C3 --vel 100 --dur 2s --hold 1.5s] [--out p.wav] [--bits f32\|16\|24]` | patch, performance | `enth.render/1`: `{wav, sample_rate, dur_s, peak_dbfs, true_peak_dbtp, clipped_pct, clipping, perf, patch_hash}`. Writes the float WAV with the `enth` chunk | 0, 3, 4, 5 |
| `enth describe <p\|x.wav> [--perf …] [--full\|--fields a,b]` | patch (rendered first) or WAV | `enth.describe/1` (shape below) | 0, 3, 4, 5 |
| `enth check <p\|x.wav> --expect '<list>' [--perf …] [--no-fix] [--fix-budget 16]` | expectation list | `enth.check/1` (shape below) | 0 all pass, 1 some fail, 2, 3, 4, 5 |
| `enth step p --ops … \| --set … [--expect …] [--perf …] --out p2 [--wav p2.wav] [--full]` | apply + render + describe + check | `enth.step/1`: `{patch, wav, applied, describe: <only features in --expect plus headline, unless --full>, check}` | as `check` |
| `enth bench [--patch p] [--voices 16] [--seconds 10]` | reference patch | `enth.bench/1`: `{realtime_x_per_core, ns_per_voice_sample, voices, block: 64, build: {profile, target}, note: "wall-clock, information only"}` | 0 |

- **Exit codes:** unchanged from agent-cli.md (0 to 5). `check` returns 1 even when a `fix` is offered. A budget-exhausted wrapper in the eval harness uses 9, outside the product.
- **Error object (`enth.error/1`):** `{error: <code>, message, exit, path?, did_you_mean?: [..], hint, try?, min?, max?, known?}`. `hint` is always present: the next command to run or the fix.
- **Error codes:**
  - exit 2: `usage`, `bad_ops`, `bad_expect`, `bad_unit`, `bad_perf`;
  - exit 3: `unknown_param`, `unknown_module`, `unknown_template`, `unknown_feature`, `out_of_range` (with `try` set to the clamped value), `bad_patch`, `patch_too_new` (migration needed);
  - exit 4: `unsupported_wav`, `render_failed`, `write_failed`, `analysis_failed`;
  - exit 5: `file_not_found`.

**`enth.describe/1`** (additions to agent-cli.md's example in bold):

```json
{"schema":"enth.describe/1","source":"lead.patch.json","dur_s":3,
 "perf":{"note":"A4","hold_s":2,"source":"patch"},
 "pitch":{"f0_hz":440,"note":"A4","cents_dev":0.2,"stability":1},
 "envelope":{"attack_ms":5,"decay_ms":15,"sustain_db":-4.5,"release_ms":174,"duration_ms":2154,
             "beat_corrected":true,"ripple_db":0.7,"release_extrapolated":false},
 "spectrum":{"centroid_hz":4349,"centroid_track_hz":[4400,4350,4300]},
 "detune":{"beating_hz":6.82,"depth_db":14.4},
 "loudness":{"lufs_i":-13.9,"peak_dbfs":-2.9,"true_peak_dbtp":-2.7,"clipped_pct":0,"clipping":false},
 "noise_ratio":0.01}
```

- **`perf.source`:** `patch`, `flag`, `wav` (the chunk) or `inferred`.
- **`beat_corrected: false` plus a `beat_note`:** set when a beat was found but too slow to correct (less than one period in the held part). The guide says sustain may then be off by a few dB.
- **`release_extrapolated: true`:** set when the clip ended first. The value is then an estimate, no longer a lower bound.

**`enth.check/1`:**

```json
{"schema":"enth.check/1","source":"lead.patch.json","perf":{"note":"A4","dur_s":3,"hold_s":2},
 "pass":false,"passed":7,"failed":1,"thin":1,
 "results":[
  {"expect":"envelope.sustain_db > -4","got":-4.5,"unit":"dB","pass":false,"margin":-0.5,"noise":0.1},
  {"expect":"envelope.attack_ms > 8","got":9,"unit":"ms","pass":true,"margin":1,"margin_pct":12.5,"noise":1,"thin":true},
  {"expect":"detune.beating_hz in [3Hz, 9Hz]","got":6.82,"unit":"Hz","pass":true,"margin":2.18,"margin_pct":32,"noise":0}],
 "fix":{"measured":true,"pass_all":true,"renders":11,
        "ops":[{"set":"amp1.sustain","to":"-3.3dB"},{"set":"amp1.release","to":"180ms"}],
        "after":{"envelope.sustain_db":-3.9,"envelope.attack_ms":9},
        "single_param_enough":false,"note":"apply all ops together; every item passed in this render"},
 "blocked":[]}
```

- **`fix`** is one multi-parameter move for the whole list, rendered and measured (`after` holds real values). When the best move doesn't pass everything, it says `pass_all: false` and lists what still fails.
- **`blocked: [{expect, why, at_bound: ["amp1.release"]}]`** names items no editable parameter can move further. `why` says when the performance or the structure is the limit (a held note lasts at least `perf.hold`; there's no noise source, …).
- **Structural hints** (`{"text": …, "measured": false}`) stay per result, for absent descriptors that no parameter of the current structure can create (e.g. "add a second oscillator with detune").
- **`check` on a WAV** gives margins and noise, with noise computed from re-analysis of shifted windows (no re-render), but no `fix`.

**Expectation syntax:**
- The spike's forms, which the trial agents used without errors: `brightness>2kHz`, `pitch=A3±30ct`, short aliases.
- Plus agent-cli.md's forms: canonical paths, `in [a,b]`, `~`, `!= null`, `== null`.
- ASCII `+-` is accepted for `±`.
- Output always echoes the canonical path.

### 3. What changes from `spike-agent`

| spike-agent | enth CLI v0 |
| --- | --- |
| Flat `toy10` patch, `--set` only | Patch v1 (modules, routes); `--ops` JSON, with `--set k=v` kept as sugar for `set` ops (the trial agents relied on it) |
| 8 commands | + `schema`, `modules`, `get`, `diff`, `bench`; `params <kind>` |
| Envelope on 8 ms RMS with a global peak | Beat-corrected envelope (§4.1), robust peak, line-fit sustain, decay threshold in dB with hold-off, extrapolated release. `decay_ms` changes meaning |
| Beating from the fundamental, 1 to 25 Hz | Multi-harmonic, 0.15 to 25 Hz, agreement and tremolo checks |
| Descriptors on 16-bit clamped samples; `clipping` bool | Float render; `peak_dbfs` can exceed 0; `true_peak_dbtp`, `clipped_pct`; `master.level` parameter; exact gain fix |
| 16-bit WAV; a WAV loses the hold | Float32 WAV with `enth` chunk; `--perf` > chunk > inferred |
| `check`: pass/fail, one hint per failing item (one parameter) | `margin`, `noise`, `thin` per item; one `fix` for all items; `blocked` |
| `step` echoes the whole `describe` | Only the features in `--expect` plus a headline, unless `--full` |
| `guide` as 5.8 KB of nested JSON | Markdown in one field, ≤ 6 KB, plus `--topic` pages |
| `enth.error/1` | Same shape; adds `try`, `min`, `max` for ranges; codes enumerated |

### 4. Descriptor algorithms (to code from)

All of these are pure functions of the samples and the known hold, in `enth-analysis` (L3). They use `f64`, and transcendentals are acceptable in analysis: design.md §5 keeps the pass/fail tolerance rule. The envelope runs at a 1 ms hop on an 8 ms RMS.

#### 4.1 Beat-corrected envelope

1. `E` = RMS over 8 ms, every 1 ms. Peak region: from the first time `E ≥ 0.3·max(E)` (+20 ms) to note-off − 15 ms (or the end).
2. Beat rate `r` from §4.2. With no beat, or fewer than 1.0 periods in the region, use `E` as it is, set `beat_corrected = false`, and report the beat.
3. Fit, by linear least squares over the region (every 2nd ms, ignoring samples below −30 dB re peak): `ln E(t) ≈ Σ_{p≤deg} c_p x^p + Σ_{k=1}^{K} (a_k cos 2πkrt + b_k sin 2πkrt)`, with x in [−1, 1] over the region. Use deg = 3 when ≥ 3 periods, 2 when ≥ 1.5, otherwise 1, and K = 6 when ≥ 2 periods, otherwise 2. `t` is absolute time from note-on: the oscillators' phases are fixed from note-on, so the pattern extends to the attack and the release.
4. Pattern `P(t) = Σ_k (…)`. Corrected envelope `C(t) = E(t)·exp(−clamp(P(t), ±2.3))`. Where `P(t) < −0.7` (a beat null deeper than 6 dB), drop the sample and interpolate `ln C` linearly from its valid neighbours.
5. On `C`:
   - Peak = max of a 5 ms moving average. Onset = first `C ≥ 0.01·peak`.
   - `attack_ms` = first `C ≥ 0.891·peak` (−1 dB) after the onset, minus the onset.
   - `sustain_db`: straight line through `20 log10 C` over [note-off − 10 − W, note-off − 10], where W = max(70 ms, 1 beat period when ≥ 2 periods), evaluated at the window's end, in dB re peak, floored at −80.
   - `decay_ms`: from the peak to the first time `C` drops below `S_dB + max(1.5, 0.2·|S_dB|)` (with `S_dB` floored at −40) and stays below for max(30 ms, half a beat period). It is 0 when the sustain is within 1 dB of the peak.
   - `release_ms`: from note-off to the *last* time `C` is above −40 dB re its level just before note-off. If the clip ends first, fit a line in dB over the tail and extrapolate (`release_extrapolated: true`).
   - `duration_ms`: onset to the last `C ≥ 0.01·peak`.
   - `ripple_db`: p95 − p5 of `C` against its 200 ms moving average over the held part.
6. Measured quality: §F2, final row.

#### 4.2 Beating rate

1. **Stretch:** the held part, from −6 dB re peak to note-off or to where the level stays more than 30 dB down; at least 0.5 s.
2. **Demodulation:** for each harmonic h = 1…12 with h·f0 < min(0.45·sr, 10 kHz), demodulate at h·f0 with a phasor recurrence, renormalised every 1,024 samples, so no per-sample `sin`. Boxcar of 4 periods of f0, which nulls the other harmonics; decimate to 500 Hz. Drop harmonics more than 30 dB below the strongest.
3. **Per-harmonic spectrum:** divide out a cubic fit of the log magnitude. Hann window, zero-pad ×8, real FFT, then scale to the relative wobble amplitude.
4. **Harmonic sum:** `S(r)` = amplitude-weighted mean over the usable harmonics of `M_h(h·r)` (linear interpolation). Harmonic h is usable for r when h·r·T ≥ 2.5 cycles, h·r < f0/8 and h·r ≤ 60 Hz. Grid: r from 0.15 to 25 Hz, log steps of 0.2 %. Take the global maximum, then prefer 2r or 3r if they score ≥ 0.85 of it, then refine on a ±2 % grid.
5. **Accept only if all of these hold:**
   - the best harmonic's wobble is ≥ 0.35;
   - `S(r)` is ≥ 6× the median of `S`;
   - at least one usable harmonic has its own peak within 3 % of h·r, or two such harmonics when r·T < 1;
   - **not a tremolo:** over harmonics ≥ 2, the weighted reading at r itself is lower than at h·r.
6. **Output:** `beating_hz`, `depth_db` (peak-to-trough of the deepest harmonic), `harmonics_used`, `prominence`. Measured quality: §F3.
7. **Cost:** about 10 to 20 ms per 2 s clip on this loaded laptop, against 10 ms for the whole old `describe` (§F7). Cache `r` across the probes of one hint call, since only detune and note move it.

#### 4.3 Clipping and level

- `peak_dbfs`: sample peak of the engine's float output (may be above 0).
- `true_peak_dbtp`: 4× oversampled, from `ebur128` in true-peak mode. The crate is already a dependency (0.1.10, MIT, verified in its source); the true-peak mode itself was not tested here.
- `clipped_pct`: share of samples with |x| ≥ 1.0. `clipping` = `clipped_pct > 0`.
- **Engine:** a `master.level` ParamSpec, in dB, linear mapping, range −36 to +12 dB, default 0. The range is my proposal. It is not modulatable and is applied after effects. Renders stay float until the file writer.
- **Fix rule:** when `clipping` is true, `fix` gets `{"set": "master.level", "to": now − (true_peak_dbtp + 1) dB}`. That fix is exact because gain is linear, so `measured: true` without a probe.
- Descriptors other than loudness are taken on the float render, which keeps distortion from the file format out of brightness.

#### 4.4 Performance for WAVs

1. `--perf` (file `enth.perf/1` or inline) wins.
2. Otherwise the `enth` chunk: `{"schema":"enth.wavmeta/1","engine","sample_rate","perf":{notes:[{note,vel,on_s,off_s}],dur_s},"patch_hash","seed"}`, written by `render` before `data`.
3. Otherwise the inferred note-off (§F5), with `perf.source: "inferred"`.
4. Otherwise decay, sustain and release are null, with `why_missing: "pass --perf or render with enth"`.

#### 4.5 Margins and noise

- **Margin:** `margin` = the signed distance to the nearest edge, in the feature's unit. `margin_pct` = 100·(got − edge)/edge for Hz/ms features.
- **Noise:** re-render the same patch with K = 2 nuisance variations, then take the largest |Δ| per descriptor. Variations:
  - always: a common phase shift of the oscillators, i.e. a time shift of the steady tone;
  - when detuned: a different inter-oscillator phase;
  - when the engine seeds noise or random phase per voice: another seed.
- **Thin:** `thin: true` when |margin| ≤ 2·noise.
- **Cost:** 2 renders + describe (about 50 ms at toy speed). `--no-noise` turns it off.

### 5. Multi-parameter hint solver

**Where:** a minimal `enth-solver` (L5) module `hint`, pure:

```rust
fn hint(base: &Patch, items: &[Expect], eval: &mut dyn Evaluator) -> Fix
```

The CLI supplies the evaluator (render + describe, rayon for the probe batch, results by index as design.md §7 requires). Expectation parsing and evaluation live in `enth-analysis` (`expect` module), next to the descriptor vocabulary.

**Algorithm** (prototype `src/solver.rs`):

1. **Free parameters:** continuous, editable, non-performance ParamSpecs of the patch's modules, on the normalised axis (`u` in 0..1). The search may use normalised values, which agent-cli.md allows "at ML boundaries"; output is in plain units.
2. **Probe:** one render per free parameter at u ± 0.1, toward the middle. Rows are the items' values in the search domain: ln for Hz/ms, linear for dB, ratios and cents.
3. **Absent descriptors** (no beating, no pitch) have no slope. Probe further (u ± 0.3, ± 0.6), first along parameters whose `ParamSpec.perceptual` names that descriptor. Move to the point where it appears closest to its band, and restart from there.
4. **Model step:** minimise `Σ_i w_i·dist(g_i + J_i·Δu, band_i)² + λ‖Δu‖²` subject to `0 ≤ u + Δu ≤ 1`.
   - `band_i` is the accepted interval shrunk by the item's hint margin (×1.1 for scale features, 1.5 dB for dB, 3 ct for pitch).
   - `w_i` = 1/margin².
   - Passing items are in the sum, so moves that push them out are penalised.
   - Solve by an active-set Gauss-Newton loop with clamping (≤ 30 inner iterations of a 9×9 solve: microseconds).
5. **Verify:** render u + Δu. Accept if the failing count drops, or stays equal with a lower total shortfall; then halve λ and apply a Broyden rank-1 update to J. Otherwise multiply λ by 4. At most 6 verify renders.
6. **Sparsify:** reset moves smaller than 3 % of their range, keeping the change only if the result holds (1 render). Round to 4 significant digits.
7. **Report:** `fix.ops`, the measured `after` values, `pass_all`, and `single_param_enough` (a model-only one-parameter check). Add `blocked` for items whose helpful parameters are all at a bound or have no effect, and `conflicts` (pairs of items whose needed directions have cosine < −0.3, with the shared parameter). The conflict output needs more testing before it ships (§F7).
8. **Categorical parameters** (waveform, filter mode): only when the continuous solve leaves failures. Try each alternative value, with its own probe if needed, and keep the best. This was not prototyped. The spike's wave probe (2 renders) is the cheap first check.

**Budget and latency:**
- Typical call: 11 to 16 renders, plus a wide probe only for absent items.
- Measured 0.25 to 0.65 s at toy speed under load 10.
- At the G0 floor of 100× realtime per core, a 2 s render is about 20 ms. With about 20 ms of describe, 16 evaluations are about 0.65 s serial, or about 0.25 s with the probe batch on 4 to 5 effective cores. That fits an interactive `check`/`step`.
- Default `--fix-budget 16`; `--no-fix` for scripts.
- **Probe clip length:** design.md §7's "~0.5 s clips" for hints cannot measure sustain, release or slow beats. Probes must use the goal's performance whenever the list has envelope or beating items. Short clips are only for spectral items.

### 6. Eval harness: `content/agent-evals/` and `cargo xtask agent-eval`

**Goal file** (`content/agent-evals/goals/<id>.toml`; one file per goal, so reviews and diffs stay small):

```toml
schema = "enth.agent-eval/1"
id = "pluck-bright-short"
added = "2026-10-19"
difficulty = 1                       # 1..5
tags = ["pluck", "envelope", "brightness"]
brief = "a short bright pluck"       # the only text the agent gets on the self-written track
perf = { notes = [{ note = "A3", vel = 100, on_s = 0.0, off_s = 1.5 }], dur_s = 2.0 }
template = "init"                    # starting patch for the agent (templates are part of the product)
expect = "envelope.attack_ms < 10, spectrum.centroid_hz > 2kHz, envelope.sustain_db < -30, envelope.duration_ms < 800, pitch.note ~ A3"
budget = { renders = 20, turns = 60, usd = 1.0 }
rationale = "…"                      # for maintainers; never shown to the agent
engine_min = "0.1.0"
```

**Layout:**
- `content/agent-evals/goals/*.toml`: the 10 starter goals, plus reference sounds in R1.
- `content/agent-evals/prompts/{given,self}.md`: the trial prompt templates, versioned. A template change is an eval-version change.
- `content/agent-evals/results/<date>-<model>.json`: committed summaries only.
- Transcripts, patches and WAVs go to `target/agent-eval/<stamp>/<goal>/<track>-t<N>/` (not committed): `stream.jsonl`, `final.patch.json`, `final.wav`, `judge.json`, `enth-calls.jsonl`.

**Invocation** (one trial):
1. Make an empty temp dir. Copy the release `enth` binary in, behind a wrapper script named `enth` that logs each call (argv, exit, wall time, whether it rendered) to `enth-calls.jsonl` and refuses a 21st render (exit 9, `{"error":"budget_exhausted"}`).
2. Run:
   ```bash
   claude --bare -p "$(render prompt)" --output-format stream-json --verbose \
     --model claude-sonnet-5-5 --max-turns 60 --max-budget-usd 1.00 \
     --tools "Bash,Read" --allowedTools "Bash(enth *)" "Read" --permission-mode dontAsk \
     --permission-prompts none --no-session-persistence \
     --append-system-prompt-file content/agent-evals/prompts/system.md \
     --json-schema '{"type":"object","required":["patch"],"properties":{"patch":{"type":"string"},"expect":{"type":"string"},"explanation":{"type":"string"}}}'
   ```
   On a subscription instead of an API key: drop `--bare` and set the `CLAUDE_CODE_DISABLE_*` variables and a clean `CLAUDE_CONFIG_DIR` (§F9).
3. **Judge:** `enth check <structured_output.patch> --perf <goal perf> --expect <goal list>` (exit 0 = solved). Record margins, and whether the patch was produced by `enth` (not hand-written JSON).
4. **Self-written track:** the prompt gives only `brief` and `perf`. The agent must return its own `expect`. Score both: does its own list pass (self-consistency), and does the hidden designer list pass (the real score).
5. Python or TS Agent SDK: not needed. The CLI is the same engine and keeps `xtask` in Rust.
6. A direct Messages API loop with an `enth`-only tool would cost less (no Claude Code system prompt) and could set sampling parameters. It needs an HTTP client in `xtask`, and rustls with ring rather than aws-lc-rs to pass `deny.toml`. Keep it as a later runner.

**Commands:**
- `cargo xtask agent-eval [--track given|self|both] [--goals …] [--trials 3] [--model …] [--runner claude-code] [--max-usd 20] [--jobs 1]`: sequential by default, for this laptop's RAM.
- `cargo xtask agent-eval report <dir>`.
- `cargo xtask agent-eval listen <dir>`: owner-run; plays audio, so only when the owner starts it.

**Metrics** per goal and track:
- solve rate over 3 trials; pass@3 and pass^3;
- median and maximum renders;
- median tokens (input, cache read, cache write, output, from `modelUsage`) and cost (`total_cost_usd`, marked as an estimate);
- invalid-command rate (exit 2/3 calls ÷ all calls); median wall time;
- final margins and thin count.

Header: model id from `system/init`, Claude Code version, enth git hash, eval and prompt version, date.

**Blind owner check:**
- Per goal, pick one passing trial (seeded choice, median renders); if none passed, the best by shortfall. That gives 10 WAVs.
- Loudness-match them to −23 LUFS as in `spike-metric listen`, and shuffle the names.
- Show the brief text only. The owner answers "matches the brief?" yes/no, with optional 1 to 5. G1 needs ≥ 7 of 10 yes.
- The answer files live under `_local/`, kept out of the repo.

**Schedule:** on demand, plus nightly through a systemd user timer on the owner's machine (or a scheduled GitHub Actions job with an API-key secret). **Never a merge gate**, as agent-cli.md already says.

**Cost** *(estimate)*, assuming about 10 turns per trial, about 25k cached context per turn and about 400 output tokens per turn:
- Sonnet 5.5 ≈ 250k cache reads × $0.10 + 30k cache writes × $2.50 + 4k output × $10 per MTok ≈ $0.14 per trial.
- 60 trials (10 goals × 3 × 2 tracks) ≈ $9 per full run; Opus 5.5 ≈ $18.
- The first run replaces this with measurements.

### 7. `enth guide` (one page)

Content, in this order, as markdown of at most about 6 KB:

1. **What enth is:** it renders and measures; you can't hear the result, so work from the numbers; descriptors approximate hearing. The workflow is `new` → `step` with `--expect` until `pass`, then `explain`.
2. **Commands** you need, one line each, with the exact form: `params <kind>`, `new --template`, `step … --set k=v,… --expect … --out`, `check`, `describe`, `diff`, `get`.
3. **Edits:** `--set k=v[,…]` (units: 800Hz, 1.2kHz, 40ms, 0.5s, -6dB, 12ct, +2st, 1oct, 30%, note names); ops JSON for add, route and remove. Ranges come from `params`, and an out-of-range value is an error that names the nearest valid value.
4. **Expectations:** syntax, the 12 most-used descriptors with units, and pointers to `enth guide --topic descriptors` and `--topic expect` for the rest.
5. **Reading `check`:**
   - `margin` (inside is positive) and `thin` (within measurement noise: aim further).
   - `fix`: apply all its ops in one step; `after` was measured.
   - `blocked`: no parameter in reach moves it; change the structure or rethink the goal.
   - Structural hints are suggestions, not measurements.
6. **Things the probe agents tripped on:**
   - The judge fixes the performance, so don't pass by changing note or length.
   - Pitch is the played note.
   - The clip must cover the release (otherwise `release_extrapolated`).
   - Clipping distorts brightness; lower `master.level` when `clipping` is true.
   - Slow beats (`beat_corrected: false`) leave sustain uncertain by a few dB.
   - `decay` is a dB-threshold time, not the ADSR parameter.
7. **Budget:** a `step` is one render, and `fix` renders are free to you. Stop when everything passes without `thin`.
8. **Exit codes** and the error object.
9. **Writing your own expectations:** a vocabulary table (pluck, pad, bass, lead, bell, bright, dark, buzzy, soft, detuned/chorused, wide, noisy) mapped to typical descriptor ranges. Phrase them as starting points, and don't copy the eval goals' lists.

## Plan check

| Doc | Says | Evidence says | Change |
| --- | --- | --- | --- |
| roadmap.md P1 / G1 | G1: an agent solves ≥ half of 10 goals within 20 renders. `content/agent-evals` and `cargo xtask agent-eval` are listed under **R1** | G1 (end of P1a/P1b) needs the harness and the 10 text goals, and agent-cli.md's protocol (3 trials, blind check, self-written track) already replaces the old wording | Move the text-goal eval set and `xtask agent-eval` (claude-code runner, both tracks) into **P1a**; reference sounds stay R1. Update G1's text to point to the protocol |
| design.md §6 | "Envelope features are beat-aware (a crest envelope)" | A crest/boxcar envelope gets 23 % beat invariance, against 64 % for fitted removal; crest moves attack edges (§F2) | "Beat removal by a fitted periodic pattern at the measured beat rate (§4.1 of report 03)" |
| design.md §7 | Hints and `suggest` use ~0.5 s clips | Sustain, release, decay and slow beats need the goal's performance | Short clips only for spectral items; probes use the goal's performance for envelope and beating items |
| agent-cli.md | Margins; hints solve all items at once; WAV perf "P1 decides" | Confirmed by the prototype | Decide: float WAV + `enth` chunk + `--perf` + inference; `check` output gains `fix`, `blocked`, `noise`, `thin` (§2) |
| agent-cli.md descriptor table | `loudness.clipping` bool | Not enough; clipping corrupted brightness by 24–116 % | Add `true_peak_dbtp`, `clipped_pct`; describe the float render; `master.level` in the catalogue |
| architecture / layers | P1a crates: params, dsp, patch, engine, render, analysis, cli | The hint search needs render + analysis + patch and must not live in the shell ("shells translate") | Add a minimal `enth-solver` (L5) in P1a with `hint`; CMA-ES joins in R1 |
| AGENTS.md CLI | Output shape change → version bump + insta | Fine; add the typed round-trip test so schema and code can't drift | Owner decides whether additive fields bump the version |
| deny.toml | `all-features = true` | Optional deps of dev-dependencies are checked too (inferred); `jsonschema` would drag rustls/aws-lc-rs | Don't add `jsonschema`; typed round-trip instead |
| Patch v1 (sub-researcher 01's area) | — | Oscillator start-phase policy changes descriptors (at 0 ct the phase difference is timbre) and defines the "nuisance seed" | Patch v1 needs an explicit phase mode per oscillator (free/reset/random-seeded) and a per-render nuisance seed separate from the patch seed |
| roadmap CLI v0 list | 13 commands | All 13 have a defined shape (§2); `random`, `compare`, `suggest`, `match`, `explain`, `validate`, `migrate` stay later | Keep; `--set` sugar in `apply`/`step`; `bench` reports wall-clock as information, with instruction counts in `xtask` per AGENTS.md |

**Proposed P1a work items for this axis** (sizes are unvalidated guesses, like every effort figure in the docs):
1. `enth-params` units parser (port spike-agent `units.rs` and add `st`, `+-`).
2. `enth-analysis`: the descriptors of §4 with tests ported from `proto03` (invariance, accuracy, false-positive and tremolo suites as `#[ignore]` sweeps, plus fast unit cases). Expectations, margins and noise.
3. `enth-solver`: `hint`.
4. `enth-render`: float WAV writer + `enth` chunk + `enth.perf/1`.
5. `enth-cli`: the 13 commands, clap derive, error mapping, schemas, insta-cmd snapshots, typed round-trip tests, `guide`.
6. `xtask agent-eval` + 10 goal files + prompts + `listen`.

Items 2 and 5 are the large ones.

## Gaps

- **Toy synth only:** two equal-level oscillators, one voice, mono. Not tested: unequal oscillator levels (partial beat depth), three oscillators or unison (several beat rates, so the beat pattern is not periodic), stereo, LFO-modulated sounds, noise, effects. The real engine will need the invariance, accuracy and false-positive sweeps rerun.
- **Slow beats** (less than one period in the held part, e.g. 5 ct below about 250 Hz) are left uncorrected and flagged. One idea, not built: estimate the fundamental's beat phase from two neighbouring higher harmonics (θ₁ = θ_{h+1} − θ_h) and predict the total power pattern from the harmonic weights. That would correct slow beats too.
- **Residual errors:** decay agrees with its twin on only 74 % of patches and sustain has a p90 error of 2.7 dB on detuned sounds. Triangle waves (deep nulls) dominate the failures. Note-off inference picks an earlier knee 10 % of the time (p10 −117 ms).
- **Tremolo discrimination** was tested on 5 synthetic cases only. `modulation.amp_lfo` from the same test was not built.
- **Hints:** categorical moves (waveform, filter mode) and the `ParamSpec.perceptual`-guided wide probe are designed but not built. The conflict detector is untested on real conflicts. Hint cost on the real engine is extrapolated, not measured.
- **No LLM agent was run** against the new descriptors or the `fix` hint: no API spend in this session. The follower table shows the mechanics, not agent behaviour. Token and cost figures for the harness are estimates on stated assumptions.
- **Determinism of analysis across platforms** was not checked here. The descriptors use `f64` with `std` maths, and pass/fail near a threshold could differ between platforms by tiny amounts. Margins plus design.md §5's tolerance rule cover this, but a test should pin it.
- **Not verified by running:** ebur128's true-peak mode, and cargo-deny's handling of optional dev-dependency features with `all-features = true`.
- **Timings** are upper bounds: load was 7 to 10 on a 6-core laptop with other sessions running.
- **The trial's transcripts were not kept**, so what agents "needed" (§F10) is read from command logs and the probe doc, not from their reasoning.
- **Process note:** early in this session my crates.io API queries (about 22 requests, `web/crate-*.json`) sent a User-Agent header containing the owner's email address. That broke the rule against personal details in headers. Every later request used the generic `enth-research/0.1`.
