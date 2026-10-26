//! `ott`: three-band upward and downward compression (the "OTT" sound of bass music) on the compressor's detector
//! (S2a lane OTT; equations, constants, measurements and sources in `docs/dsp/ott.md`).
//!
//! Signal flow, per stereo frame (the parameters are `OttParams`; `a_down`, `a_up`, the gains and `mix` ramp linearly
//! over a call from the previous segment's value to this one's):
//!
//! ```text
//! x      = sane(in) · input                              NaN/inf → 0, |x| ≤ 1e6
//! low, mid, high = LR4 split of x at low_split, high_split          flat sum (an allpass), zero latency
//! per band b (stereo-linked: the louder channel drives both gains):
//!   env   += k·(max(|L_b|, |R_b|) − env)                 the compressor's amplitude follower (attack / release)
//!   lvl    = 20·log10(max(env, 1e-10))                   dBFS
//!   G      = a_down · down_b(lvl) + a_up · up_b(lvl)     dB: downward ≤ 0 above its threshold, upward ≥ 0 below its
//!   gs    += k_smooth·(G − gs)                           one-pole gain smoother, time constant attack/2
//!   out_b  = band_b · 10^(gs/20) · band_gain_b           the lift is cut where it would carry a sample over the
//!                                                          upward threshold + 20 dB (the onset guard, below)
//! wet    = (low' + mid + high) · output                  low' = the low band through an allpass at high_split
//! out    = in·(1 − mix) + wet·mix                        mix 0 returns the input, bit for bit
//! ```
//!
//! - **The split** is the Linkwitz–Riley 4th order (two cascaded Butterworth sections per branch), built on the
//!   topology-preserving-transform state-variable filter in `f64`: `LP4 = LP2(LP2(x))`, and the high branch is
//!   `HP4 = A(x) − LP4` with the allpass `A(x) = x − 2·√2·BP(x)` that LR4's two branches sum to (`LP4 + HP4 = A`, since
//!   `(1 + s⁴)/(s² + √2·s + 1)² = (s² − √2·s + 1)/(s² + √2·s + 1)`). The low band goes through the second crossover's
//!   allpass, so the three unity-gain bands sum to `A₁(A₂(x))`: flat in magnitude, with the phase of two allpasses.
//! - **The curves** are the compressor's: the downward branch is [`StaticCurve`] (Giannoulis, Massberg and Reiss
//!   2012, eq. 4) and the upward branch is the same curve mirrored about the level axis. Upward gain is capped at
//!   [`UP_CAP_DB`] and fades to zero between [`UP_FADE_TOP_DB`] and [`UP_FADE_BOTTOM_DB`], so silence is not lifted.
//!   The thresholds, ratios and times of the three bands are [`BAND_SPECS`]: this lane's constants.
//! - **The onset guard.** The detector has no look-ahead (latency is 0), so a loud sample can meet a lift that was
//!   computed for the quiet signal before it: a note that starts after a gap, lifted by up to [`UP_CAP_DB`], would
//!   leave the module 20 dB or more over its input for the first milliseconds. A lift therefore never carries a band
//!   sample above the band's upward threshold plus [`LIFT_CEILING_ABOVE_THRESHOLD_DB`]: where it would, the lift is cut
//!   for that sample (never below the downward gain) and forgotten. A steady signal is brought to the upward
//!   threshold, a noise to about 12 dB over it, so the guard binds only on the first few milliseconds of an onset.
//! - **Intensity** is the slope: `a_down = depth·downward`, `a_up = depth·upward` (both 0–1) scale the dB gain of
//!   each curve, so depth 0 is the identity (every band at 0 dB, the flat split) and depth 1 the full curves.
//!
//! Real-time and determinism contract: the type owns no heap memory and nothing allocates; transcendentals go through
//! the pinned `libm`; no fused operations; the filter states are flushed in code (at the end of every `process` call,
//! which the engine makes a 32-frame segment) and the detector states every frame; NaN or infinite input never enters
//! the state. There is no look-ahead: zero latency.

use core::f64::consts::{PI, SQRT_2};

use crate::compressor::{
    DB_PER_LOG2, LOG2_PER_DB, MAX_LEVEL, MIN_LEVEL, StaticCurve, one_pole_gain, sane,
};
use crate::util::flush::{flush, flush64};

/// The number of bands.
pub const BANDS: usize = 3;
/// Width of the soft knee of every threshold, dB, centred on the threshold (the compressor's quadratic knee).
pub const KNEE_DB: f32 = 12.0;
/// The most any band is lifted by upward compression, dB.
pub const UP_CAP_DB: f32 = 30.0;
/// Upward gain is full at and above this detector level (dBFS)…
pub const UP_FADE_TOP_DB: f32 = -84.0;
/// …and zero at and below this one, with a straight line in dB between, so silence and dither are not lifted.
pub const UP_FADE_BOTTOM_DB: f32 = -96.0;

/// A lift never carries a sample above this many dB over the band's upward threshold (see [`Ott`]'s docs: the
/// onset guard). Well above any steady lifted signal (a sine comes out at the threshold, noise 12 dB over it), so it
/// binds only when a loud sample meets a lift computed for a quiet one.
pub const LIFT_CEILING_ABOVE_THRESHOLD_DB: f32 = 20.0;

/// Sample rate used before `prepare` is called.
const DEFAULT_SAMPLE_RATE: f32 = 48_000.0;

/// One band's constants at `time` 100 %: lane OTT's, since no public source gives the preset's numbers
/// (`docs/dsp/ott.md`, "Constants and evidence").
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BandSpec {
    /// Downward threshold, dBFS of the band's detector level.
    pub down_threshold_db: f32,
    /// Downward ratio `R:1` (1000 stands for the ∞:1 limiter).
    pub down_ratio: f32,
    /// Upward threshold, dBFS: below it the band is lifted.
    pub up_threshold_db: f32,
    /// Upward ratio `R:1`: the output rises 1 dB for `R` dB of input below the threshold.
    pub up_ratio: f32,
    /// Attack, ms: the follower's time constant when the level rises.
    pub attack_ms: f32,
    /// Release, ms: the follower's time constant when the level falls.
    pub release_ms: f32,
}

/// The bands, low to high. Lows and mids share thresholds; the high band sits 10 dB lower (a mastered spectrum
/// tilts down) and is a limiter on the way down (the sources agree on ∞:1 there) and 66.7:1 on the way up, as the
/// low and mid bands are; times follow the band's cycle length.
pub const BAND_SPECS: [BandSpec; BANDS] = [
    BandSpec {
        down_threshold_db: -16.0,
        down_ratio: 4.0,
        up_threshold_db: -38.0,
        up_ratio: 66.7,
        attack_ms: 20.0,
        release_ms: 200.0,
    },
    BandSpec {
        down_threshold_db: -16.0,
        down_ratio: 4.0,
        up_threshold_db: -38.0,
        up_ratio: 66.7,
        attack_ms: 8.0,
        release_ms: 100.0,
    },
    BandSpec {
        down_threshold_db: -26.0,
        down_ratio: 1000.0,
        up_threshold_db: -48.0,
        up_ratio: 66.7,
        attack_ms: 2.0,
        release_ms: 50.0,
    },
];

/// The static curve of one band: downward above its threshold, upward below its own, in dB of gain against the
/// band's detector level. Public so the editor can draw it and the tests can hold the module to it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BandCurve {
    down: StaticCurve,
    /// The upward curve is the compressor's curve with the level axis flipped: `up_db(L) = −mirrored.gain_db(−L)`.
    mirrored: StaticCurve,
}

impl BandCurve {
    /// The curve of `spec`.
    #[must_use]
    pub fn new(spec: &BandSpec) -> Self {
        Self {
            down: StaticCurve::new(spec.down_threshold_db, spec.down_ratio, KNEE_DB),
            mirrored: StaticCurve::new(-spec.up_threshold_db, spec.up_ratio, KNEE_DB),
        }
    }

    /// The downward gain in dB (`<= 0`) at a detector level of `level_db` dBFS, at full intensity.
    #[inline(always)]
    #[must_use]
    pub fn down_db(&self, level_db: f32) -> f32 {
        self.down.gain_db(level_db)
    }

    /// The upward gain in dB (`>= 0`) at `level_db`, at full intensity: the mirrored curve, capped at [`UP_CAP_DB`]
    /// and faded to zero below [`UP_FADE_TOP_DB`].
    #[inline(always)]
    #[must_use]
    pub fn up_db(&self, level_db: f32) -> f32 {
        let lift = (-self.mirrored.gain_db(-level_db)).min(UP_CAP_DB);
        let fade =
            ((level_db - UP_FADE_BOTTOM_DB) / (UP_FADE_TOP_DB - UP_FADE_BOTTOM_DB)).clamp(0.0, 1.0);
        lift * fade
    }

    /// The band's gain in dB at `level_db`, with the downward and upward intensities `a_down` and `a_up` (0–1).
    #[inline(always)]
    #[must_use]
    pub fn gain_db(&self, level_db: f32, a_down: f32, a_up: f32) -> f32 {
        a_down * self.down_db(level_db) + a_up * self.up_db(level_db)
    }
}

/// User-facing settings, in the catalogue's real units. Out-of-range values are clamped by
/// [`Ott::set_params`] (see [`OttParams::sanitized`]); a NaN or infinite field is replaced by its default.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OttParams {
    /// Overall intensity, percent 0–100: scales both curves of every band.
    pub depth: f32,
    /// Scales every band's attack and release, percent 10–1000.
    pub time: f32,
    /// How much of the upward curve is used, percent 0–100 (times `depth`).
    pub upward: f32,
    /// How much of the downward curve is used, percent 0–100 (times `depth`).
    pub downward: f32,
    /// Gain before the bands, dB −24…24.
    pub input_db: f32,
    /// Gain after the bands, dB −24…24.
    pub output_db: f32,
    /// Crossover between the low and mid bands, Hz 40–500.
    pub low_split_hz: f32,
    /// Crossover between the mid and high bands, Hz 500–10,000.
    pub high_split_hz: f32,
    /// Output level of the low band, dB −24…24.
    pub low_gain_db: f32,
    /// Output level of the mid band, dB −24…24.
    pub mid_gain_db: f32,
    /// Output level of the high band, dB −24…24.
    pub high_gain_db: f32,
    /// Wet share, percent 0–100: 0 is the input, bit for bit.
    pub mix: f32,
}

impl OttParams {
    /// The catalogue's defaults: `depth` 30 %, the reference timing, both curves fully used, unity levels, the
    /// crossovers at 88.3 Hz and 2.5 kHz, fully wet.
    pub const DEFAULT: Self = Self {
        depth: 30.0,
        time: 100.0,
        upward: 100.0,
        downward: 100.0,
        input_db: 0.0,
        output_db: 0.0,
        low_split_hz: 88.3,
        high_split_hz: 2_500.0,
        low_gain_db: 0.0,
        mid_gain_db: 0.0,
        high_gain_db: 0.0,
        mix: 100.0,
    };

    /// The values the core uses: every field clamped into its range, non-finite ones replaced by the default.
    #[must_use]
    pub fn sanitized(&self) -> Self {
        let d = Self::DEFAULT;
        Self {
            depth: sane(self.depth, 0.0, 100.0, d.depth),
            time: sane(self.time, 10.0, 1_000.0, d.time),
            upward: sane(self.upward, 0.0, 100.0, d.upward),
            downward: sane(self.downward, 0.0, 100.0, d.downward),
            input_db: sane(self.input_db, -24.0, 24.0, d.input_db),
            output_db: sane(self.output_db, -24.0, 24.0, d.output_db),
            low_split_hz: sane(self.low_split_hz, 40.0, 500.0, d.low_split_hz),
            high_split_hz: sane(self.high_split_hz, 500.0, 10_000.0, d.high_split_hz),
            low_gain_db: sane(self.low_gain_db, -24.0, 24.0, d.low_gain_db),
            mid_gain_db: sane(self.mid_gain_db, -24.0, 24.0, d.mid_gain_db),
            high_gain_db: sane(self.high_gain_db, -24.0, 24.0, d.high_gain_db),
            mix: sane(self.mix, 0.0, 100.0, d.mix),
        }
    }
}

impl Default for OttParams {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// The parameters that ramp across a call, in the units the per-sample code uses.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Gains {
    /// `depth · downward`, 0–1.
    a_down: f32,
    /// `depth · upward`, 0–1.
    a_up: f32,
    /// Linear gain before the bands.
    input: f32,
    /// Linear gain after the bands.
    output: f32,
    /// Linear gain of each band after its compression.
    band: [f32; BANDS],
    /// Wet share, 0–1.
    mix: f32,
}

/// `10^(db/20)` through the pinned `libm`.
#[inline]
fn db_to_gain(db: f32) -> f32 {
    libm::exp2f(db * LOG2_PER_DB)
}

impl Gains {
    /// The gains of sanitised parameters.
    fn of(p: &OttParams) -> Self {
        let depth = p.depth * 0.01;
        Self {
            a_down: depth * (p.downward * 0.01),
            a_up: depth * (p.upward * 0.01),
            input: db_to_gain(p.input_db),
            output: db_to_gain(p.output_db),
            band: [
                db_to_gain(p.low_gain_db),
                db_to_gain(p.mid_gain_db),
                db_to_gain(p.high_gain_db),
            ],
            mix: p.mix * 0.01,
        }
    }

    /// `from + (to − from)·w`, field by field: exactly `from` when the two are equal, and `to` to within one rounding
    /// at `w = 1` (exactly, when `to` is zero).
    #[inline(always)]
    fn lerp(from: &Self, to: &Self, w: f32) -> Self {
        let l = |a: f32, b: f32| a + (b - a) * w;
        Self {
            a_down: l(from.a_down, to.a_down),
            a_up: l(from.a_up, to.a_up),
            input: l(from.input, to.input),
            output: l(from.output, to.output),
            band: [
                l(from.band[0], to.band[0]),
                l(from.band[1], to.band[1]),
                l(from.band[2], to.band[2]),
            ],
            mix: l(from.mix, to.mix),
        }
    }
}

/// Coefficients of a Butterworth (`k = √2`) topology-preserving-transform state-variable filter.
#[derive(Clone, Copy, Debug, PartialEq)]
struct SvfCoef {
    a1: f64,
    a2: f64,
    a3: f64,
}

impl SvfCoef {
    /// For a corner of `hz` at `sample_rate`, clamped to 10 Hz … 0.45·fs so `tan` stays finite and the filter stable.
    fn new(hz: f32, sample_rate: f32) -> Self {
        let fs = f64::from(sample_rate);
        let fc = f64::from(hz).clamp(10.0, 0.45 * fs);
        let g = libm::tan(PI * fc / fs);
        let a1 = 1.0 / (1.0 + g * (g + SQRT_2));
        let a2 = g * a1;
        Self { a1, a2, a3: g * a2 }
    }
}

/// The two integrator states of one state-variable filter.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Svf {
    ic1: f64,
    ic2: f64,
}

impl Svf {
    /// One step: returns `(band, low)`; the high output is `x − √2·band − low`.
    #[inline(always)]
    fn step(&mut self, c: &SvfCoef, x: f64) -> (f64, f64) {
        let v3 = x - self.ic2;
        let v1 = c.a1 * self.ic1 + c.a2 * v3;
        let v2 = self.ic2 + c.a2 * self.ic1 + c.a3 * v3;
        self.ic1 = 2.0 * v1 - self.ic1;
        self.ic2 = 2.0 * v2 - self.ic2;
        (v1, v2)
    }

    fn flush(&mut self) {
        self.ic1 = flush64(self.ic1);
        self.ic2 = flush64(self.ic2);
    }

    fn is_zero(&self) -> bool {
        self.ic1 == 0.0 && self.ic2 == 0.0
    }
}

/// One channel's filters: the two crossovers and the allpass that aligns the low band with the second one.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Channel {
    /// First crossover, low branch (`lp1` then `lp2`) and the stage that also gives the band output for the allpass.
    s1a: Svf,
    s1b: Svf,
    /// Second crossover, on the first one's high branch.
    s2a: Svf,
    s2b: Svf,
    /// The second crossover's allpass, on the low band.
    ap: Svf,
}

impl Channel {
    /// Split one sample into `[low, mid, high]`, the low band already through the second crossover's allpass.
    #[inline(always)]
    fn split(&mut self, c1: &SvfCoef, c2: &SvfCoef, x: f64) -> [f64; BANDS] {
        let two_k = 2.0 * SQRT_2;
        // First crossover: LP4 = LP2(LP2(x)); HP4 = A(x) − LP4 with A(x) = x − 2k·BP(x).
        let (bp1, lp1) = self.s1a.step(c1, x);
        let (_, low) = self.s1b.step(c1, lp1);
        let rest = (x - two_k * bp1) - low;
        // Second crossover on the high branch.
        let (bp2, lp2) = self.s2a.step(c2, rest);
        let (_, mid) = self.s2b.step(c2, lp2);
        let high = (rest - two_k * bp2) - mid;
        // The low band gets the second crossover's allpass, so the unity-gain bands sum to A₁(A₂(x)).
        let (bpa, _) = self.ap.step(c2, low);
        [low - two_k * bpa, mid, high]
    }

    fn flush(&mut self) {
        for s in [
            &mut self.s1a,
            &mut self.s1b,
            &mut self.s2a,
            &mut self.s2b,
            &mut self.ap,
        ] {
            s.flush();
        }
    }

    fn is_zero(&self) -> bool {
        [self.s1a, self.s1b, self.s2a, self.s2b, self.ap]
            .iter()
            .all(Svf::is_zero)
    }

    fn count_subnormals(&self) -> usize {
        [self.s1a, self.s1b, self.s2a, self.s2b, self.ap]
            .iter()
            .flat_map(|s| [s.ic1, s.ic2])
            .filter(|x| x.is_subnormal())
            .count()
    }
}

/// One band's detector and gain state with its coefficients.
#[derive(Clone, Copy, Debug, PartialEq)]
struct BandDyn {
    curve: BandCurve,
    /// Follower coefficient while the level rises / falls, and the gain smoother's (attack/2).
    k_attack: f32,
    k_release: f32,
    k_smooth: f32,
    /// The linear sample level a lift may not carry a sample over.
    ceiling: f32,
    /// Amplitude follower state, linear, `>= 0`.
    env: f32,
    /// Smoothed downward gain in dB (`<= 0`) and upward gain in dB (`>= 0`), before the band gain.
    gs_down: f32,
    gs_up: f32,
}

impl BandDyn {
    fn new(spec: &BandSpec) -> Self {
        Self {
            curve: BandCurve::new(spec),
            k_attack: 1.0,
            k_release: 1.0,
            k_smooth: 1.0,
            ceiling: libm::exp2f(
                (spec.up_threshold_db + LIFT_CEILING_ABOVE_THRESHOLD_DB) * LOG2_PER_DB,
            ),
            env: 0.0,
            gs_down: 0.0,
            gs_up: 0.0,
        }
    }

    /// Set the times (ms, already scaled by `time`) for `sample_rate`.
    fn set_times(&mut self, attack_ms: f32, release_ms: f32, sample_rate: f32) {
        self.k_attack = one_pole_gain(attack_ms, sample_rate);
        self.k_smooth = one_pole_gain(0.5 * attack_ms, sample_rate);
        self.k_release = one_pole_gain(release_ms, sample_rate);
    }

    /// The smoothed compression gain in dB.
    fn gain_db(&self) -> f32 {
        self.gs_down + self.gs_up
    }

    /// The linear compression gain for this frame. `e` is the louder channel's band sample, `>= 0`: it drives the
    /// follower, and it is the sample the onset guard looks at.
    #[inline(always)]
    fn gain(&mut self, e: f32, a_down: f32, a_up: f32) -> f32 {
        let k = if e > self.env {
            self.k_attack
        } else {
            self.k_release
        };
        self.env = flush(self.env + k * (e - self.env));
        let level_db = DB_PER_LOG2 * libm::log2f(self.env.max(MIN_LEVEL));
        let down = self.curve.down_db(level_db);
        let up = self.curve.up_db(level_db);
        self.gs_down = flush(self.gs_down + self.k_smooth * (a_down * down - self.gs_down));
        self.gs_up = flush(self.gs_up + self.k_smooth * (a_up * up - self.gs_up));
        let mut gain = libm::exp2f((self.gs_down + self.gs_up) * LOG2_PER_DB);
        // The onset guard. The detector has no look-ahead, so a loud sample can meet a lift that was computed for the
        // quiet signal before it (60 dB of lift, a note that starts after a gap) and leave 20 dB over its input. A lift
        // never carries a sample over the ceiling: it is cut for this sample (to unity at worst: the downward gain
        // stays) and forgotten, so it does not come back until the detector asks for it. It never binds on a steady
        // signal, which a lift brings to the upward threshold.
        if self.gs_up > 0.0 && gain * e > self.ceiling {
            let g_down = libm::exp2f(self.gs_down * LOG2_PER_DB);
            let allowed = (self.ceiling / e).max(g_down);
            if allowed < gain {
                gain = allowed;
                self.gs_up =
                    flush((DB_PER_LOG2 * libm::log2f(allowed / g_down)).clamp(0.0, self.gs_up));
            }
        }
        gain
    }
}

/// The three-band upward and downward compressor, stereo, zero latency. See the module docs for the signal flow.
///
/// Lifecycle: [`new`](Self::new), [`prepare`](Self::prepare) with the sample rate, [`set_params`](Self::set_params)
/// (or [`set_ramp`](Self::set_ramp) for a per-segment ramp), then [`process_in_place`](Self::process_in_place).
/// [`reset`](Self::reset) clears the state, not the settings.
#[derive(Clone, Debug)]
pub struct Ott {
    sample_rate: f32,
    /// The settings in use (sanitised): the ramp's end.
    params: OttParams,
    /// The ramp's start and end for the next call.
    from: Gains,
    to: Gains,
    c1: SvfCoef,
    c2: SvfCoef,
    chan: [Channel; 2],
    band: [BandDyn; BANDS],
}

impl Default for Ott {
    fn default() -> Self {
        Self::new()
    }
}

impl Ott {
    /// A processor with the default settings at 48 kHz.
    #[must_use]
    pub fn new() -> Self {
        let params = OttParams::DEFAULT;
        let g = Gains::of(&params);
        let mut o = Self {
            sample_rate: DEFAULT_SAMPLE_RATE,
            params,
            from: g,
            to: g,
            c1: SvfCoef::new(params.low_split_hz, DEFAULT_SAMPLE_RATE),
            c2: SvfCoef::new(params.high_split_hz, DEFAULT_SAMPLE_RATE),
            chan: [Channel::default(); 2],
            band: BAND_SPECS.map(|s| BandDyn::new(&s)),
        };
        o.refresh_coefficients();
        o
    }

    /// Set the sample rate (clamped to 8 kHz..768 kHz, NaN gives 48 kHz), recompute every coefficient and reset the
    /// state. Allocates nothing; the engine calls it when a plan is compiled.
    pub fn prepare(&mut self, sample_rate: f32) {
        self.sample_rate = sane(sample_rate, 8_000.0, 768_000.0, DEFAULT_SAMPLE_RATE);
        self.refresh_coefficients();
        self.reset();
    }

    /// Clear the filters, the followers and the gain smoothers. The settings and the sample rate stay.
    pub fn reset(&mut self) {
        self.chan = [Channel::default(); 2];
        for b in &mut self.band {
            b.env = 0.0;
            b.gs_down = 0.0;
            b.gs_up = 0.0;
        }
    }

    /// Apply new settings at once (real units, see [`OttParams`]): the next call uses them from its first frame.
    pub fn set_params(&mut self, params: &OttParams) {
        self.set_ramp(params, params);
    }

    /// Apply new settings so that the next call ramps the ramped fields (`depth`, `upward`, `downward`, `input`,
    /// `output`, the three band gains and `mix`) linearly from `from` (the previous segment's values) to `to` (this
    /// segment's), reaching `to` on its last frame; the rest (`time`, the splits) take `to` at once. The engine
    /// calls it once per 32-frame segment, with the same `from` and `to` whether or not the module was skipped
    /// before, so a skipped segment cannot change the bits.
    pub fn set_ramp(&mut self, from: &OttParams, to: &OttParams) {
        let (from, to) = (from.sanitized(), to.sanitized());
        self.from = Gains::of(&from);
        self.to = Gains::of(&to);
        if to != self.params {
            let old = self.params;
            self.params = to;
            if to.time != old.time
                || to.low_split_hz != old.low_split_hz
                || to.high_split_hz != old.high_split_hz
            {
                self.refresh_coefficients();
            }
        }
    }

    /// The settings in use (sanitised).
    #[must_use]
    pub fn params(&self) -> OttParams {
        self.params
    }

    /// The sample rate set by `prepare`.
    #[must_use]
    pub fn sample_rate(&self) -> f32 {
        self.sample_rate
    }

    /// Latency in samples: always 0 (no look-ahead).
    #[must_use]
    pub fn latency_samples(&self) -> usize {
        0
    }

    /// The current compression gain of band `b` (0 low, 1 mid, 2 high) in dB, before the band gain: positive lifts,
    /// negative squashes. For a meter; read it after `process`.
    #[must_use]
    pub fn band_gain_db(&self, b: usize) -> f32 {
        self.band.get(b).map_or(0.0, BandDyn::gain_db)
    }

    /// The band's detector level in dBFS (the follower's reading).
    #[must_use]
    pub fn band_level_db(&self, b: usize) -> f32 {
        self.band.get(b).map_or(f32::NEG_INFINITY, |d| {
            DB_PER_LOG2 * libm::log2f(d.env.max(MIN_LEVEL))
        })
    }

    /// Whether every filter, follower and gain smoother is exactly zero: silent input then gives silent output (every
    /// band sample is zero, whatever the gains) and leaves the whole state unchanged, for any settings. The engine
    /// uses it to skip an idle module. A tail reaches this on its own: all of it is flushed below −300 dBFS.
    #[must_use]
    pub fn is_idle(&self) -> bool {
        self.chan.iter().all(Channel::is_zero)
            && self
                .band
                .iter()
                .all(|b| b.env == 0.0 && b.gs_down == 0.0 && b.gs_up == 0.0)
    }

    /// How many state values are subnormal; always 0 for a healthy processor. For the engine's tail test.
    #[must_use]
    pub fn count_subnormals(&self) -> usize {
        self.chan
            .iter()
            .map(Channel::count_subnormals)
            .sum::<usize>()
            + self
                .band
                .iter()
                .filter(|b| {
                    b.env.is_subnormal() || b.gs_down.is_subnormal() || b.gs_up.is_subnormal()
                })
                .count()
    }

    /// Split one stereo frame into `[low, mid, high]` for each channel (`[band][channel]`), the way the module does
    /// before the dynamics, advancing the filters. For tests and tools; `x` is used as it is.
    pub fn split_frame(&mut self, l: f64, r: f64) -> [[f64; 2]; BANDS] {
        let a = self.chan[0].split(&self.c1, &self.c2, l);
        let b = self.chan[1].split(&self.c1, &self.c2, r);
        [[a[0], b[0]], [a[1], b[1]], [a[2], b[2]]]
    }

    /// Process `min(len)` frames in place.
    pub fn process_in_place(&mut self, l: &mut [f32], r: &mut [f32]) {
        let n = l.len().min(r.len());
        if n == 0 {
            return;
        }
        let (from, to) = (self.from, self.to);
        let steady = from == to;
        let step = 1.0 / n as f32;
        for (i, (s_l, s_r)) in l.iter_mut().zip(r.iter_mut()).enumerate() {
            let g = if steady {
                to
            } else {
                Gains::lerp(&from, &to, (i + 1) as f32 * step)
            };
            let (y_l, y_r) = self.tick(*s_l, *s_r, &g);
            *s_l = y_l;
            *s_r = y_r;
        }
        self.from = self.to;
        for c in &mut self.chan {
            c.flush();
        }
    }

    /// One stereo frame with the gains `g` for this frame.
    #[inline(always)]
    fn tick(&mut self, l: f32, r: f32, g: &Gains) -> (f32, f32) {
        // A NaN or infinite sample must not enter the filters: they would hold it forever.
        let clean = |x: f32| {
            if x.is_finite() {
                f64::from(x.clamp(-MAX_LEVEL, MAX_LEVEL))
            } else {
                0.0
            }
        };
        let gin = f64::from(g.input);
        let bands_l = self.chan[0].split(&self.c1, &self.c2, clean(l) * gin);
        let bands_r = self.chan[1].split(&self.c1, &self.c2, clean(r) * gin);

        let (mut wet_l, mut wet_r) = (0.0_f64, 0.0_f64);
        for b in 0..BANDS {
            let (xl, xr) = (bands_l[b], bands_r[b]);
            // Stereo link: the louder channel drives both gains.
            let e = (xl.abs().max(xr.abs()) as f32).min(MAX_LEVEL);
            let gain = self.band[b].gain(e, g.a_down, g.a_up) * g.band[b];
            let gain = f64::from(gain);
            wet_l += xl * gain;
            wet_r += xr * gain;
        }
        let gout = f64::from(g.output);
        // Flushed: an input from the subnormal range must not leave as one (the engine's FTZ-independence rule).
        let (wet_l, wet_r) = (flush((wet_l * gout) as f32), flush((wet_r * gout) as f32));

        // Dry/wet: 0 is the input bit for bit (a NaN input passes as it is), 1 is the wet signal.
        if g.mix <= 0.0 {
            (l, r)
        } else if g.mix >= 1.0 {
            (wet_l, wet_r)
        } else {
            let dry = 1.0 - g.mix;
            (l * dry + wet_l * g.mix, r * dry + wet_r * g.mix)
        }
    }

    /// Recompute the crossover and time coefficients from the stored settings and sample rate.
    fn refresh_coefficients(&mut self) {
        let p = self.params;
        self.c1 = SvfCoef::new(p.low_split_hz, self.sample_rate);
        self.c2 = SvfCoef::new(p.high_split_hz, self.sample_rate);
        let scale = p.time * 0.01;
        for (b, spec) in self.band.iter_mut().zip(&BAND_SPECS) {
            b.set_times(
                spec.attack_ms * scale,
                spec.release_ms * scale,
                self.sample_rate,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    //! White-box tests of the internals (the follower's times, the ramp, the idle claim). The public contract (the
    //! split, the curves, the sums, the hash, allocation) is in `crates/enth-dsp/tests/ott.rs`.
    use super::*;

    /// A band's follower on a constant detector input of 0.5 reaches 63.2 % of the way after `attack_ms` (scaled by
    /// `time`), then falls to 36.8 % after `release_ms`: the compressor's times, for every band and every `time`.
    #[test]
    fn the_follower_hits_the_attack_and_release_times_scaled_by_time() {
        for &fs in &[44_100.0_f32, 48_000.0, 96_000.0] {
            for &time in &[10.0_f32, 100.0, 1_000.0] {
                for (b, spec) in BAND_SPECS.iter().enumerate() {
                    let mut o = Ott::new();
                    o.prepare(fs);
                    o.set_params(&OttParams {
                        time,
                        ..OttParams::DEFAULT
                    });
                    let attack = f64::from(spec.attack_ms * time * 0.01) * f64::from(fs) / 1000.0;
                    let release = f64::from(spec.release_ms * time * 0.01) * f64::from(fs) / 1000.0;
                    let band = &mut o.band[b];
                    let target = 0.5 * (1.0 - libm::expf(-1.0));
                    let mut n = 0_u32;
                    while band.env < target {
                        band.gain(0.5, 0.0, 0.0);
                        n += 1;
                        assert!(n < 2_000_000, "never rose");
                    }
                    assert!(
                        (f64::from(n) - attack).abs() <= (attack * 1e-4).max(1.0),
                        "band {b}, time {time} %, {fs}: rose in {n} samples, expected {attack}"
                    );
                    // Settle, then let go.
                    for _ in 0..(attack * 30.0) as u32 + 10 {
                        band.gain(0.5, 0.0, 0.0);
                    }
                    let top = band.env;
                    let target = top * libm::expf(-1.0);
                    let mut n = 0_u32;
                    while band.env > target {
                        band.gain(0.0, 0.0, 0.0);
                        n += 1;
                        assert!(n < 20_000_000, "never fell");
                    }
                    assert!(
                        (f64::from(n) - release).abs() <= (release * 1e-4).max(1.0),
                        "band {b}, time {time} %, {fs}: fell in {n} samples, expected {release}"
                    );
                }
            }
        }
    }

    #[test]
    fn the_gain_smoother_runs_at_half_the_attack() {
        let mut o = Ott::new();
        o.prepare(48_000.0);
        for (b, spec) in BAND_SPECS.iter().enumerate() {
            let want = one_pole_gain(0.5 * spec.attack_ms, 48_000.0);
            assert_eq!(o.band[b].k_smooth, want);
        }
    }

    #[test]
    fn a_ramp_reaches_its_end_and_is_exact_when_nothing_moves() {
        let a = Gains::of(&OttParams::DEFAULT);
        let b = Gains::of(&OttParams {
            depth: 80.0,
            mix: 0.0,
            input_db: 6.0,
            ..OttParams::DEFAULT
        });
        assert_eq!(Gains::lerp(&a, &a, 0.37), a);
        let end = Gains::lerp(&a, &b, 1.0);
        assert_eq!(end.mix, 0.0, "a ramp to 0 lands exactly on 0");
        assert!((end.a_down - b.a_down).abs() < 1e-7 && (end.input - b.input).abs() < 1e-6);
        let mid = Gains::lerp(&a, &b, 0.5);
        assert!(mid.mix > 0.0 && mid.mix < 1.0 && mid.input > a.input && mid.input < b.input);
    }

    #[test]
    fn exp2_of_zero_is_exactly_unity_so_depth_zero_is_exact() {
        assert_eq!(db_to_gain(0.0), 1.0);
        assert_eq!(libm::exp2f(0.0 * LOG2_PER_DB), 1.0);
    }

    #[test]
    fn an_idle_module_ignores_silence_for_any_settings() {
        // The claim behind `is_idle`: silent input leaves the whole state unchanged and gives exact zeros.
        for p in [
            OttParams::DEFAULT,
            OttParams {
                depth: 100.0,
                input_db: 24.0,
                output_db: 24.0,
                low_gain_db: 24.0,
                mid_gain_db: -24.0,
                high_gain_db: 24.0,
                time: 1_000.0,
                mix: 50.0,
                ..OttParams::DEFAULT
            },
            OttParams {
                depth: 0.0,
                mix: 0.0,
                ..OttParams::DEFAULT
            },
        ] {
            let mut o = Ott::new();
            o.prepare(48_000.0);
            o.set_params(&p);
            assert!(o.is_idle());
            let before = o.clone();
            let (mut l, mut r) = ([0.0_f32; 32], [0.0_f32; 32]);
            for _ in 0..50 {
                o.process_in_place(&mut l, &mut r);
            }
            assert!(l.iter().chain(&r).all(|x| x.to_bits() == 0));
            assert!(
                o.chan == before.chan && o.band == before.band,
                "state unchanged"
            );
        }
    }

    #[test]
    fn the_sanitiser_replaces_nan_and_clamps() {
        let wild = OttParams {
            depth: f32::NAN,
            time: -3.0,
            upward: f32::INFINITY,
            downward: 1e9,
            input_db: f32::NEG_INFINITY,
            output_db: 99.0,
            low_split_hz: 0.0,
            high_split_hz: 1e9,
            low_gain_db: -99.0,
            mid_gain_db: f32::NAN,
            high_gain_db: 99.0,
            mix: -1.0,
        };
        let s = wild.sanitized();
        let d = OttParams::DEFAULT;
        assert_eq!(s.depth, d.depth);
        assert_eq!((s.time, s.upward, s.downward), (10.0, d.upward, 100.0));
        assert_eq!(
            (s.output_db, s.low_split_hz, s.high_split_hz),
            (24.0, 40.0, 10_000.0)
        );
        assert_eq!((s.low_gain_db, s.high_gain_db, s.mix), (-24.0, 24.0, 0.0));
        assert_eq!(s, s.sanitized());
        let mut o = Ott::new();
        o.prepare(8_000.0);
        o.set_params(&wild);
        let (mut l, mut r) = ([0.7_f32; 32], [-0.7_f32; 32]);
        o.process_in_place(&mut l, &mut r);
        assert!(l.iter().chain(&r).all(|x| x.is_finite()));
    }
}
