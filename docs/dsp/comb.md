# Comb filter (`comb`; `enth-dsp::comb`, `enth-engine::modules::comb`)

- **Status: implemented (S2a lane FLT, wave 3).** Kind `comb`, discriminant 10, section `voice_fx`, id prefix `comb` (comb1, comb2…), rate voice. Fixed and persisted from F0m; still hidden from the Add menu, `enth modules` and the generator until lane INT-S2a clears its flag.
- Code: `crates/enth-dsp/src/comb.rs` (the filter, the closed-form response, white-box tests), `crates/enth-engine/src/modules/comb.rs` (the per-voice module). Catalogue entry: `crates/enth-patch/src/catalogue/comb.rs`. Engine-level acceptance tests: `crates/enth-engine/tests/flt.rs`. Bench: `crates/enth-engine/benches/comb.rs`. Goldens: `tests/golden/cases/comb-*.toml`.
- A keytracked feedback comb per voice: teeth at the note's harmonics (or half way between them at negative feedback), a low-pass in the loop (`damping`), a level that never exceeds its input, a 20 Hz frequency floor that bounds the delay line, and `mix` with the dry signal. A noise burst at feedback near 99 % is a Karplus–Strong pluck; an LFO on `semi` flanges.
- Sources: Julius O. Smith, *Physical Audio Signal Processing* ("Feedback Comb Filters") for the structure; D. Jaffe and J. O. Smith, "Extensions of the Karplus-Strong plucked-string algorithm", *Computer Music Journal* 7(2), 1983, for the loop filter; research report 05 §5 for the choice of kind (`_local/sessions/2026-10-25-s2-depth-and-fast-training/`). **Provenance: written from those equations; no code ported, no third-party source consulted, so no `THIRD_PARTY_NOTICES.md` entry.** The Lagrange weights are the textbook formula; the TPT one-pole is the bilinear transform with the corner pre-warped (Zavalishin, *The Art of VA Filter Design*, as in [svf.md](svf.md)).

## Parameters (lane CAT, F1m)

The table lane FLT implements, generated from `crates/enth-patch/src/catalogue/comb.rs` by `cargo test -p enth-patch --test catalogue -- --ignored --nocapture print_reserved_markdown`. After F1m a key, unit, range, scale, default or flag changes only through the coordinator. The file's doc comment states the signal path the meanings assume; the decisions behind the table are in [catalogue.md](catalogue.md#s2a-decisions-lane-cat-f1m).

- **Pitch:** `f = max(20 Hz, 261.63 Hz · 2^((semi + keytrack · (pitch − 60)) / 12))` with `keytrack` as a fraction (svf's key-60 reference), so at 100 % and `semi` 0 the teeth sit on the note's harmonics and follow glides. **The frequency is clamped to at least 20 Hz**, which bounds the per-voice delay line: the period is at most 50 ms (2,400 samples at 48 kHz). Without the clamp, `semi` −48 at key 0 is 0.5 Hz, a 2 s buffer for every voice. (The comb also needs a period of at least two samples; the clamp's upper end is lane FLT's: 0.45·fs, see below.)
- **Structure and level:** `y[n] = (1 − |feedback|) · x[n] + feedback · lowpass(y[n − period])` (the low-pass is `damping`, open at 20 kHz; negative feedback puts the teeth between the harmonics), then `mix` (0 % bit for bit). The loop's input is scaled by `1 − |feedback|`, so the gain at the teeth peaks at 0 dB at every `feedback` (the bare `x + feedback · lowpass(y[n − period])` peaks at `1 / (1 − |feedback|)`: +6 dB on every harmonic of a keytracked note at the default 50 %, +40 dB at 99 %). A noise burst and feedback near 99 % make a Karplus–Strong pluck, a quiet one since the burst is scaled by `1 − |feedback|`; an LFO on `semi` flanges. Lane FLT measures the level (catalogue rule 3): [Level](#level).
- **Defaults:** a gentle useful setting: feedback 50 %, damping 10 kHz; no louder than its input.

### `comb` — Comb filter (voice_fx, ids `comb1`, `comb2`…; 5 parameters)

| Key | Unit | Range | Scale | Default | Flags | Tier | What it does |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `semi` | semitones | -48st … +48st | linear | 0st | mod, auto | basic | The comb's pitch in semitones from the played note (from middle C when keytrack is 0 %): +12st puts the teeth an octave up; an LFO here flanges. |
| `keytrack` | percent | 0% … 100% | linear | 100% | auto | intermediate | How much the comb's pitch follows the played note: 100 % keeps the teeth on the note's harmonics, 0 % holds a fixed pitch. |
| `feedback` | percent | -99% … 99% | linear | 50% | mod, auto | basic | How strongly the comb rings: higher is more metallic and longer; negative puts the resonances between the harmonics, hollow and an octave down. |
| `damping` | hz | 500Hz … 20kHz | log | 10kHz | mod, auto | intermediate | A low-pass inside the feedback loop: lower makes the high teeth die faster, for a softer, plucked ring; 20 kHz leaves it bright. |
| `mix` | percent | 0% … 100% | linear | 100% | mod, auto | intermediate | How much of the effect is heard: at 0 % the input passes unchanged; lower values blend the dry signal back in and soften the teeth. |

**The `mix` description does not match the maths (reported to the coordinator, not changed).** The blend `(1 − m)·x + m·y` has the gain `(1 − m) + m·H(ω)`. `H` is real and positive at every valley (`(1 − |fb|)/(1 + |fb|)`, from the loop gain `−fb` there), so adding the dry signal never produces a notch: it lifts the valleys and leaves the teeth at 0 dB, a shallower comb (at `mix` 50 % and feedback 90 % the valleys are at −5.6 dB instead of −25.6 dB). A true notch comb is a feed-forward comb (`x − x[n − D]`) or an inverted wet path; the catalogue's feedback structure has neither, and changing it would move every key in the F1m table. Nothing in the module depends on the sentence.

## The filter

Per sample, with `x` the input, `D` the delay in samples, `g = G/(1 − G)` and `G` the low-pass coefficient:

```text
delayed = Σ_k c_k · y[n − (m − 1 + k)]          k = 0…3, m = floor(D), c = Lagrange weights of the fraction D − m
v   = (delayed − s) · G
low = v + s              s = low + v            (the TPT one-pole: unity at DC, −3 dB at `damping`, a zero at Nyquist)
y   = (1 − |fb|)·x + fb·low                    written to the line, flushed
out = (1 − mix)·x + mix·y
```

- **The delay** is chosen so the loop is exactly one period at the comb frequency: `D = fs/f − atan(tan(ω/2)/g)/ω` with `ω = 2π·f/fs`, the period less the low-pass's phase delay at `f` (closed form from the bilinear one-pole `1/(1 + j·tan(ω/2)/g)`). The fundamental tooth is therefore on pitch at every `damping`; the higher teeth drift a little flat as the low-pass bends the loop phase.
- **The fractional read** is a four-point (third-order) Lagrange interpolator on the taps at `m − 1 … m + 2`, so `D ≥ 2` and the loop never reads the sample it writes. Its magnitude response is at most 1 for every fraction (a test sweeps them), so the interpolator cannot make the loop unstable; it is flat to better than 0.1 dB below fs/10 and rolls off above, which is the high-tooth damping a fractional delay always adds.
- **Range of the frequency:** `20 Hz … 0.45·fs` (21.6 kHz at 48 kHz, where the period is 2.2 samples); a NaN becomes 20 Hz. The delay is also clamped to `[2, fs/20]` samples. At 48 kHz the line holds 4,096 floats per voice (`fs/20` plus the four taps, rounded up to a power of two so reads wrap with a mask): 128 KB for all 32 voices, allocated once in `new`.
- **`feedback` is clamped to ±0.99** (the catalogue range; a NaN is 0), `mix` to `0 … 1`, `damping` to `20 Hz … 0.45·fs` in the core (the catalogue's minimum is 500 Hz).

### Modulation and zipper-free updates

The engine runs modules once per 32-frame segment. As the svf does, the module computes the tuning for the *end* of the segment and `process_block` ramps the **delay, the low-pass coefficient, the feedback, its input scale and the mix linearly per sample** from the previous segment's values. A moving delay is the flanger's Doppler: the read position slides smoothly and the interpolation weights are recomputed every sample, so a sweep never steps.

- The module keeps each voice's previous tuning itself, so a retrigger or a smoother reset is a 32-frame glide, not a step; a fresh voice starts at its first segment's values (`from = to`).
- The three inputs (`semi`, the key-tracking term in semitones, `damping`) are the cache key. When they are bit-identical to the previous segment's, nothing is recomputed: **a static comb costs no transcendental**. A recompute is one `exp2`, one `tan` and one `atan` (plus one more `tan` for the corner), all in `f64` through the pinned `libm`.
- When the delay, low-pass and feedback do not change across a segment the ramp is skipped and the weights are computed once per block.
- `mix` 0 at both ends of a segment leaves `io` untouched: the input passes **bit for bit**, while the comb keeps running so that raising `mix` finds its state current. (The state is not zeroed: a `mix` automated back up rings from what the line holds.)
- The line is flushed on every write (below 1e-15 to exactly 0, in code, not by FTZ/DAZ flags) and the low-pass memory once per block, so a decaying tail reaches exact zero and never a subnormal.
- `start_voice` zeroes the voice's line and state.
- **Plan swap:** `copy_state_from` copies the small arrays and the delay lines of the voices that have started (not the whole 128 KB), so a swap that keeps a comb keeps its ring. A new plan with a different sample rate (a different line length) copies nothing.

## Level

Closed form, `|H(ω)| = (1 − |fb|) / |1 − fb·L(ω)·I(ω)·e^(−jω(m − 1))|` with `L` the low-pass and `I` the interpolator (both of magnitude at most 1), so **`|H| ≤ 1` for every setting**: the comb never amplifies, catalogue rule 3 and F1m verdict M6. `enth_dsp::comb::response` evaluates it in `f64` complex arithmetic.

| Check | Result |
| --- | --- |
| Measured response (an impulse response through `process_block`, FFT) against `response`, several frequencies, feedbacks, dampings | within 0.03 dB (the test's bound) |
| Engine, a sine through the module on a held note at `semi` set so the sine falls on a tooth and on a valley, against `response` | within 0.05 dB (`the_comb_gain_at_its_teeth_and_valleys_is_the_closed_forms`) |
| Peak over a scan, feedback 99 %, comb at 2 kHz, damping 10 kHz (the worst case: fewest samples per period, the interpolator and low-pass both bite) | 0.938, **−0.56 dB** |
| Valley depth `(1 − \|fb\|)/(1 + \|fb\|)` | −9.5 dB at 50 %, −25.6 dB at 90 %, −46 dB at 99 % |
| Valley depth with `mix` 50 % at feedback 90 % | −5.6 dB (the teeth stay at 0 dB) |

Teeth sit on the harmonics (the impulse response's spectrum at default damping, every tooth below 4 kHz, the largest bin within a quarter of the spacing; worst tooth error at feedback 90 %):

| Comb frequency | Teeth checked | Worst error | Bound |
| --- | --- | --- | --- |
| 65.41 Hz (key 36) | 61 | 0.217 % | 1 % |
| 261.63 Hz (key 60) | 15 | 0.059 % | 1 % |
| 1,046.5 Hz (key 84) | 3 | 0.013 % | 1 % |
| 261.63 Hz, feedback −90 % (teeth at (k − ½)·f) | 15 | 0.059 % | 1 % |
| 261.63 Hz, feedback 50 % | 15 | 0.059 % | 1 % |

The same three keys through the engine module (a held note, the comb at 100 % key tracking) are within 1 % (`key_tracking_puts_the_teeth_on_the_notes_harmonics_within_one_percent`).

## Stability under modulation

Required: finite, bounded output and a tail that dies, with `semi`, `feedback` and `damping` jumping every segment at the parameter extremes. Tests:

- **Random-jump stress** (`enth-dsp`): white noise (±0.5) in; every segment a new random frequency (20 Hz … 20 kHz, log-uniform), a new damping (500 Hz … 20 kHz), a new feedback (uniform over ±0.99, with the ends over-represented by a clamp) and a new mix, for 30,000 segments (20 s): every sample finite and the peak 0.499 (asserted below 2). Then the parameters park at 2 kHz, feedback 90 %, damping 8 kHz with silence in: after 30 s the state and the line hold no subnormal and the output is exactly 0.0.
- **Module stress** (`modules/comb.rs`): the same through the engine module, 15,000 segments with a new random `semi` (±48), `damping`, `feedback` (to ±99 %, both signs) and `mix`, and a new random key every segment (`pitch_cur`); finite, peak below 2, then the same park-and-die-out check.
- **Audio-rate LFOs** (`tests/flt.rs`): LFOs at 50, 37 and 43 Hz on `semi`, `feedback` and `damping` at full depth on a held saw: the peak is 1.04 against 1.36 for the dry saw (never above 1.5× dry, asserted).
- **Smoothness** (`a_moving_delay_is_smooth_and_a_pitch_sweep_follows_the_ramp`): a 200 Hz sine through a comb retuned from 200 to 206 Hz across one block has no sample-to-sample step above 0.06 (the sine's own largest step is 0.026; a stepped delay would show a click as large as the ring).

## Aliasing

**Bound: −91.0 dB** against the sine's fundamental (report 02's measure, `enth_dsp::alias`, [aliasing.md](aliasing.md)), enforced in `tests/flt.rs::a_comb_adds_no_aliasing`. Setup: an `osc` sine through the comb at keys 33 … 105 (55 Hz … 3.5 kHz in the printed sweep; the sine at a key, the comb at its defaults except where stated); feedback +99 % with damping 20 kHz is the worst case and is swept as well; feedback −99 % is swept from key 57 after a 7 s pre-roll (below that the negative comb's ring decays slower than the measurement window). The comb is linear: it adds nothing but the engine's own sine floor, so the measurement is that floor, not the comb. The values:

| Sine | 55 Hz | 110 Hz | 220 Hz | 440 Hz | 880 Hz | 1,760 Hz | 3,520 Hz |
| --- | --- | --- | --- | --- | --- | --- | --- |
| default comb, dB | −93.3 | −103.4 | −97.8 | −93.4 | −95.5 | −92.7 | −119.8 |
| feedback +99 %, damping 20 kHz, dB | −93.8 | −98.1 | −97.4 | −94.7 | −96.4 | −92.8 | −119.7 |
| feedback −99 %, damping 20 kHz, dB | | | −97.8 | −93.4 | −95.5 | −92.7 | −119.2 |

Worst −92.7 dB; the bound is set 1.7 dB above it. Fast modulation does not alias either: a comb is linear and time-varying only through the (smoothly ramped) delay, which is a bandlimited interpolation and not a nonlinearity.

## Determinism

Transcendentals (`exp2`, `tan`, `atan`) run per voice and changed segment in `f64` through `enth_dsp::util::math` and the pinned `libm`; the per-sample code is `+ − ×` and integer index maths (`wrapping_sub` and a mask), correctly rounded, with no `mul_add`. The weights are computed in `f32` from the fraction by a fixed expression. Tests: the same bits under every FTZ/DAZ mode (`a_comb_renders_the_same_bits_under_every_ftz_mode`), zero subnormals in the output or any engine state over a 40 s render of four notes through two combs in series (feedback 99 % with damping open, and a −90 % comb at 20 Hz), with a 3 s and a 20 s release, and every voice below −100 dB at the end (`a_40_s_tail_through_combs_holds_no_subnormals`), no allocation in `process` (`a_comb_never_allocates_in_process`, `assert_no_alloc`), zero latency (the output sample uses the input sample directly, `y[n] = (1 − |fb|)·x[n] + …`; at feedback 0 the comb is the identity and the engine renders the same bits as without it), two instances keep separate state (`two_combs_keep_separate_state`). The same bits on six targets are the CI golden job's; the seven `comb-*` goldens are the pin.

## Goldens

| Case | What it pins |
| --- | --- |
| `comb-teeth` | a saw at 110 Hz through the comb at feedback 90 %: delay maths, interpolator, level bound |
| `comb-keytrack` | three notes (keys 36, 48, 72) at 100 % key tracking: the per-voice pitch read and the fresh-voice start |
| `comb-negative` | feedback −90 %: the sign of the loop and the `|fb|` input scale |
| `comb-flange` | a 0.7 Hz LFO on `semi`: the per-sample delay ramp, the interpolator under modulation, the tuning cache with a moving key |
| `comb-pluck` | an 8 ms noise burst into feedback 98 %, damping 4 kHz: the Karplus–Strong ring |
| `comb-mix` | `mix` 40 %: the blend |
| `comb-low` | `semi` −48, key tracking 0: the 20 Hz clamp and the longest delay |

## Cost

Measured with callgrind (`cargo xtask bench -p enth-engine --bench comb`), 1 s at 48 kHz, whole engine, on the build of 2026-10-26; the "marginal" column subtracts the `engine` bench of the same build with the same voices (3,621,181 at 1 voice, 37,278,521 at 16: the reference saw and the ADSR are in both):

| Case | Instructions | Budget (+10 %) | Marginal per sample per voice |
| --- | --- | --- | --- |
| `static_1v` | 6,361,238 | 6,998,000 | (fixed costs dominate at 1 voice) |
| `static_16v` | 78,791,448 | 86,671,000 | 54.1 |
| `moving_16v` (a `semi` event every segment) | 134,965,117 | 148,462,000 | 127.2 |

A static comb costs about twice an svf (54 against 28 instructions per sample per voice): four masked line reads, the interpolator's four multiply-adds, the one-pole, the feedback mix, the flush and the store. A moving pitch adds, per sample, the ramp of the five values and the weights, and, per voice and segment, a new tuning (`exp2`, two `tan`, one `atan` in `f64`): together the extra 73 instructions per sample (not split further; the static path is the one that runs most of the time). The line zeroing in `start_voice` (4,096 floats) is in the 1-voice figure.

## Behaviour other lanes must know

- **The level never exceeds the input** (peak −0.56 dB in the worst case, usually 0 dB at a tooth). The teeth are the same level as the dry signal and the valleys are lower: a comb at the default 50 % makes a sound *thinner*, never louder. A patch that needs the resonance louder adds gain after it.
- **`mix` does not make notches** (above); the catalogue's sentence for `mix` is the coordinator's to change.
- **A voice start cuts the comb's ring**: `start_voice` zeroes the line, so a voice stolen or retriggered rings from silence (the module has no state to carry across a voice's life).
- **`feedback` and `mix` ramp from `prev`** (the engine's smoothed value) and the delay from the module's own previous tuning, so a `mix` automated from 0 % rings from the line's current content.
- **The high teeth die faster than `damping` alone says**: the interpolator also rolls off above fs/10, so a comb tuned above a few kHz rings less at its top teeth than the low-pass suggests.

## Files

| What | Where | Today |
| --- | --- | --- |
| Parameter table | `crates/enth-patch/src/catalogue/comb.rs` | lane CAT's F1m table ([Parameters](#parameters-lane-cat-f1m)) |
| DSP core | `crates/enth-dsp/src/comb.rs` | implemented, 11 white-box tests |
| Engine module | `crates/enth-engine/src/modules/comb.rs` | implemented, 9 module tests; the dispatch arms are S0d's |
| Engine acceptance tests | `crates/enth-engine/tests/flt.rs` | 9 tests (aliasing, closed form, bit-exact mix 0, latency, instances, modulation, tail, FTZ, no alloc) |
| Instruction budget | `crates/enth-engine/benches/comb.rs` | measured + 10 % ([Cost](#cost)) |
| Golden cases | `tests/golden/cases/comb-*.toml` (the case-name prefix is the lane's namespace; the directory is flat) | seven ([Goldens](#goldens)) |
| WAVs | `~/code/enth-scratch/s2/flt/wav/comb-*.wav` | noise sweep, saw flange, saw negative feedback, pluck (rendered, never played) |

## Acceptance (from phase-s2.md, every module lane)

| Row | Where it is checked |
| --- | --- |
| Same bits on six targets | the seven goldens (CI) |
| FTZ-independent | `tests/flt.rs::a_comb_renders_the_same_bits_under_every_ftz_mode` |
| Zero subnormals after a 40 s tail | `tests/flt.rs::a_40_s_tail_through_combs_holds_no_subnormals` |
| Latency 0 | `tests/flt.rs::a_comb_has_zero_latency` |
| Budget +10 % | [Cost](#cost) |
| Two instances keep separate state | `tests/flt.rs::two_combs_keep_separate_state`, `modules/comb.rs` |
| `mix` 0 passes the input bit for bit | `tests/flt.rs::mix_zero_renders_the_same_bits_as_no_comb`, `dsp/comb.rs`, `modules/comb.rs` |
| Aliasing measured spectrally, bound recorded | [Aliasing](#aliasing): −91.0 dB, `tests/flt.rs::a_comb_adds_no_aliasing` |
| Teeth within 1 % at three keys, peak and valley depth documented | [Level](#level) |
| Stability under fast modulation of comb pitch | [Stability under modulation](#stability-under-modulation) |

## References

- J. O. Smith, *Physical Audio Signal Processing*, CCRMA: "Feedback Comb Filters" and "Lagrange Interpolation". The structure and the interpolator.
- D. Jaffe, J. O. Smith, "Extensions of the Karplus-Strong plucked-string algorithm", *Computer Music Journal* 7(2), 1983. The loop low-pass and the tuning by the filter's phase delay.
- V. Zavalishin, *The Art of VA Filter Design*, Native Instruments, rev. 2.1. The TPT one-pole and pre-warping.
- Research report 05 §5 (filters beyond the SVF).
