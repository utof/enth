//! Feed-forward compressor core: soft knee, make-up gain, max stereo link (P1a lane FX2).
//!
//! Signal flow, per frame (equations and provenance in `docs/dsp/compressor.md`):
//!
//! ```text
//! e   = max(|L|, |R|)                          stereo link: the louder channel drives both gains
//! env += k·(e − env),  k = k_attack if e > env else k_release        amplitude follower (JOS amp_follower_ar)
//! lvl = 20·log10(max(env, 1e-10))             dB, floored so log(0) never happens
//! G   = static_curve(lvl − T)                 Giannoulis, Massberg and Reiss (2012) soft knee, <= 0 dB
//! gs  += k_smooth·(G − gs)                    one-pole gain smoother, time constant attack/2 (JOS kneesmooth)
//! gain = 10^((gs + makeup)/20)
//! out  = in·((1 − mix) + mix·gain)            mix 100 % is the plain compressor, less is parallel compression
//! ```
//!
//! The structure follows Julius O. Smith III's `compression_gain_mono` (Faust `compressors.lib`, STK-4.3); the soft
//! knee, the make-up gain, the `max` stereo link and the mix are ours. There is no look-ahead: zero latency.
//!
//! Real-time and determinism contract: no allocation anywhere (the type owns no heap memory), transcendentals only
//! through the pinned `libm` crate, no fused multiply-add, denormals flushed in code and never through CPU flags.
//! Silence in gives silence out for any setting, and NaN or infinite input never poisons the state.

/// Floor of the detector level before the logarithm: -200 dBFS. Keeps `log2(0)` out of the audio path.
const MIN_LEVEL: f32 = 1.0e-10;
/// Detector inputs are clamped to this (+120 dBFS) so an infinite sample cannot turn the state into NaN.
const MAX_LEVEL: f32 = 1.0e6;
/// Follower and smoother states below this magnitude are set to zero (about -300 dBFS), long before subnormals.
const FLUSH_BELOW: f32 = 1.0e-15;
/// `20·log10(2)`: dB per octave of amplitude. `20·log10(x) = DB_PER_LOG2·log2(x)`.
const DB_PER_LOG2: f32 = 6.020_6;
/// `log2(10)/20`: octaves of amplitude per dB. `10^(x/20) = 2^(x·LOG2_PER_DB)`.
const LOG2_PER_DB: f32 = 0.166_096_4;

/// Sample rate used before `prepare` is called.
const DEFAULT_SAMPLE_RATE: f32 = 48_000.0;

/// Denormal flush in code: a state that small is silence. Never depends on FTZ/DAZ.
#[inline(always)]
fn flush(x: f32) -> f32 {
    if x.abs() < FLUSH_BELOW { 0.0 } else { x }
}

/// One-pole coefficient `k = 1 − exp(−1/(τ·fs))` for a time constant of `time_ms` (the 63 % time of a step).
///
/// Computed in `f64` with `expm1`, so a 10 s release at 192 kHz (`k` about 5e-7) keeps its full relative precision
/// instead of being quantised by `1 − p` in `f32`. A zero (or negative) time gives `k = 1`, an instant response.
fn one_pole_gain(time_ms: f32, sample_rate: f32) -> f32 {
    let samples = f64::from(time_ms) * 0.001 * f64::from(sample_rate);
    if samples <= 0.0 {
        1.0
    } else {
        (-libm::expm1(-1.0 / samples)) as f32
    }
}

/// Replace a non-finite value by `fallback` and clamp the rest into `lo..=hi`.
fn sane(value: f32, lo: f32, hi: f32, fallback: f32) -> f32 {
    if value.is_finite() {
        value.clamp(lo, hi)
    } else {
        fallback
    }
}

/// User-facing compressor settings, in real units.
///
/// Out-of-range values are clamped by [`Compressor::set_params`] (see [`CompressorParams::sanitized`]); a NaN or
/// infinite field is replaced by its default. The ranges the *patch catalogue* offers are narrower (they live in
/// `enth-patch`), the core accepts anything sensible so a module wrapper never has to guard it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CompressorParams {
    /// Level above which the gain is reduced, dBFS. Clamped to -120..=24.
    pub threshold_db: f32,
    /// Compression ratio `R:1` (1 = off, 1000 = near-limiter). Clamped to 1..=1000.
    pub ratio: f32,
    /// Width of the soft knee in dB, centred on the threshold (0 = hard knee). Clamped to 0..=48.
    pub knee_db: f32,
    /// Attack, ms: time constant of the amplitude follower when the level rises. The gain smoother runs at half of
    /// this, so the 63 % time of the gain reduction is about 1.1 to 1.3 times it (`docs/dsp/compressor.md`).
    /// 0 is instant. Clamped to 0..=2000.
    pub attack_ms: f32,
    /// Release, ms: time constant of the amplitude follower when the level falls. 0 is instant. Clamped to
    /// 0..=10000.
    pub release_ms: f32,
    /// Make-up gain in dB, applied to the compressed (wet) path only. Clamped to -48..=48.
    pub makeup_db: f32,
    /// Wet share in percent: 100 = fully compressed, 0 = dry (bit-identical input), between = parallel
    /// compression. Clamped to 0..=100.
    pub mix_pct: f32,
}

impl CompressorParams {
    /// The defaults of report 02: -18 dB, 4:1, 6 dB knee, 10 ms / 200 ms, no make-up, fully wet.
    pub const DEFAULT: Self = Self {
        threshold_db: -18.0,
        ratio: 4.0,
        knee_db: 6.0,
        attack_ms: 10.0,
        release_ms: 200.0,
        makeup_db: 0.0,
        mix_pct: 100.0,
    };

    /// The values the core actually uses: every field clamped into its accepted range, non-finite ones replaced by
    /// the default.
    #[must_use]
    pub fn sanitized(&self) -> Self {
        let d = Self::DEFAULT;
        Self {
            threshold_db: sane(self.threshold_db, -120.0, 24.0, d.threshold_db),
            ratio: sane(self.ratio, 1.0, 1000.0, d.ratio),
            knee_db: sane(self.knee_db, 0.0, 48.0, d.knee_db),
            attack_ms: sane(self.attack_ms, 0.0, 2000.0, d.attack_ms),
            release_ms: sane(self.release_ms, 0.0, 10_000.0, d.release_ms),
            makeup_db: sane(self.makeup_db, -48.0, 48.0, d.makeup_db),
            mix_pct: sane(self.mix_pct, 0.0, 100.0, d.mix_pct),
        }
    }
}

impl Default for CompressorParams {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// The static compression curve in the dB domain: gain change as a function of the detector level.
///
/// Giannoulis, Massberg and Reiss (2012), eq. 4, written as the gain change `G = y − x` (always `<= 0`) with
/// `d = x − T`, knee width `W` and `s = 1/R − 1`:
///
/// ```text
/// G = 0                            if 2d <= −W
/// G = s·(d + W/2)² / (2W)          if |2d| < W
/// G = s·d                          if 2d >= W
/// ```
///
/// It is evaluated in `f64` and rounded once to `f32`, so the result is within half an `f32` ulp of the exact value
/// (under 1e-6 dB for any reduction below 32 dB). `W = 0` is JOS's hard-knee `max(d, 0)·s`. Public so the editor
/// can draw the transfer curve from the very same code.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StaticCurve {
    threshold_db: f64,
    knee_db: f64,
    /// `1/R − 1`, in `(−1, 0]`.
    slope: f64,
}

impl StaticCurve {
    /// `ratio` is clamped to `>= 1`, `knee_db` to `>= 0`; non-finite inputs are the caller's mistake and give NaN.
    #[must_use]
    pub fn new(threshold_db: f32, ratio: f32, knee_db: f32) -> Self {
        Self {
            threshold_db: f64::from(threshold_db),
            knee_db: f64::from(knee_db.max(0.0)),
            slope: 1.0 / f64::from(ratio.max(1.0)) - 1.0,
        }
    }

    /// Gain change in dB (`<= 0`) for a detector level of `level_db` dBFS.
    #[inline(always)]
    #[must_use]
    pub fn gain_db(&self, level_db: f32) -> f32 {
        let d = f64::from(level_db) - self.threshold_db;
        let w = self.knee_db;
        let g = if 2.0 * d <= -w {
            0.0
        } else if 2.0 * d < w {
            let t = d + 0.5 * w;
            self.slope * t * t / (2.0 * w)
        } else {
            self.slope * d
        };
        g as f32
    }

    /// Output level in dB for an input level in dB: the transfer curve of Giannoulis et al., `x + G(x)`.
    #[inline]
    #[must_use]
    pub fn output_db(&self, level_db: f32) -> f32 {
        level_db + self.gain_db(level_db)
    }
}

/// A stereo feed-forward compressor with zero latency. See the module docs for the signal flow.
///
/// Lifecycle: [`new`](Self::new) (or `Default`), [`prepare`](Self::prepare) with the sample rate,
/// [`set_params`](Self::set_params) whenever a setting changes (cheap and idempotent, safe to call every audio
/// segment), then [`process`](Self::process). [`reset`](Self::reset) clears the state, not the settings.
#[derive(Clone, Debug)]
pub struct Compressor {
    sample_rate: f32,
    params: CompressorParams,
    curve: StaticCurve,
    /// Follower coefficient while the level rises.
    k_attack: f32,
    /// Follower coefficient while the level falls.
    k_release: f32,
    /// Gain smoother coefficient (time constant attack/2).
    k_smooth: f32,
    makeup_db: f32,
    /// Share of the dry input in the output.
    dry: f32,
    /// Share of the compressed signal in the output.
    wet: f32,
    /// Amplitude follower state, linear, `>= 0`.
    env: f32,
    /// Smoothed static gain in dB, `<= 0`, without the make-up. `-gr` is the gain reduction.
    gr: f32,
    /// Largest gain reduction in dB since the last `take_peak_gain_reduction_db`.
    peak_gr: f32,
}

impl Default for Compressor {
    fn default() -> Self {
        Self::new()
    }
}

impl Compressor {
    /// A compressor with the default settings at 48 kHz.
    #[must_use]
    pub fn new() -> Self {
        let mut c = Self {
            sample_rate: DEFAULT_SAMPLE_RATE,
            params: CompressorParams::DEFAULT,
            curve: StaticCurve::new(0.0, 1.0, 0.0),
            k_attack: 1.0,
            k_release: 1.0,
            k_smooth: 1.0,
            makeup_db: 0.0,
            dry: 0.0,
            wet: 1.0,
            env: 0.0,
            gr: 0.0,
            peak_gr: 0.0,
        };
        c.refresh_all();
        c
    }

    /// Set the sample rate (clamped to 8 kHz..768 kHz, NaN gives 48 kHz), recompute every coefficient and reset the
    /// state. Allocates nothing (the type owns no buffers); the engine calls it when a plan is compiled.
    pub fn prepare(&mut self, sample_rate: f32) {
        self.sample_rate = sane(sample_rate, 8_000.0, 768_000.0, DEFAULT_SAMPLE_RATE);
        self.refresh_all();
        self.reset();
    }

    /// Clear the follower, the gain smoother and the meters. The settings and the sample rate stay.
    pub fn reset(&mut self) {
        self.env = 0.0;
        self.gr = 0.0;
        self.peak_gr = 0.0;
    }

    /// Apply new settings (real units, see [`CompressorParams`]). Values are sanitised first; only the coefficients
    /// whose inputs changed are recomputed, and an identical call costs a few comparisons. The filter state is
    /// untouched, so a change never clicks by itself: the gain smoother glides to the new curve.
    pub fn set_params(&mut self, params: &CompressorParams) {
        let new = params.sanitized();
        let old = self.params;
        if new == old {
            return;
        }
        if new.threshold_db != old.threshold_db
            || new.ratio != old.ratio
            || new.knee_db != old.knee_db
        {
            self.curve = StaticCurve::new(new.threshold_db, new.ratio, new.knee_db);
        }
        if new.attack_ms != old.attack_ms {
            self.k_attack = one_pole_gain(new.attack_ms, self.sample_rate);
            self.k_smooth = one_pole_gain(0.5 * new.attack_ms, self.sample_rate);
        }
        if new.release_ms != old.release_ms {
            self.k_release = one_pole_gain(new.release_ms, self.sample_rate);
        }
        self.makeup_db = new.makeup_db;
        let wet = new.mix_pct * 0.01;
        self.wet = wet;
        self.dry = 1.0 - wet;
        self.params = new;
    }

    /// The settings in use (sanitised).
    #[must_use]
    pub fn params(&self) -> CompressorParams {
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

    /// Current gain reduction in dB, `>= 0`: how far the compressed path is turned down, make-up not included.
    /// For the meter; read it after `process`.
    #[must_use]
    pub fn gain_reduction_db(&self) -> f32 {
        (-self.gr).max(0.0)
    }

    /// Largest gain reduction in dB seen since the previous call (or since `reset`), then restart the peak from the
    /// current value. A meter polled once per UI frame reads this instead of a single instant.
    pub fn take_peak_gain_reduction_db(&mut self) -> f32 {
        let peak = self.peak_gr;
        self.peak_gr = self.gain_reduction_db();
        peak
    }

    /// Process `min(len)` frames of a stereo signal from `l`, `r` into `out_l`, `out_r`. The caller passes four
    /// slices of equal length; a mismatch processes the shortest (never panics).
    pub fn process(&mut self, l: &[f32], r: &[f32], out_l: &mut [f32], out_r: &mut [f32]) {
        for (((&in_l, &in_r), o_l), o_r) in
            l.iter().zip(r).zip(out_l.iter_mut()).zip(out_r.iter_mut())
        {
            let (y_l, y_r) = self.tick(in_l, in_r);
            *o_l = y_l;
            *o_r = y_r;
        }
    }

    /// [`process`](Self::process) on buffers that are overwritten.
    pub fn process_in_place(&mut self, l: &mut [f32], r: &mut [f32]) {
        for (s_l, s_r) in l.iter_mut().zip(r.iter_mut()) {
            let (y_l, y_r) = self.tick(*s_l, *s_r);
            *s_l = y_l;
            *s_r = y_r;
        }
    }

    /// One stereo frame.
    #[inline(always)]
    pub fn tick(&mut self, l: f32, r: f32) -> (f32, f32) {
        // Stereo link: the louder channel drives both. `max` ignores a single NaN channel; both NaN is zeroed and an
        // infinite sample is clamped, so the follower state can never become NaN.
        let peak = l.abs().max(r.abs());
        let e = if peak.is_nan() {
            0.0
        } else {
            peak.min(MAX_LEVEL)
        };

        // Amplitude follower with separate attack and release (a leaky integrator with a unity-gain fixed point).
        let k = if e > self.env {
            self.k_attack
        } else {
            self.k_release
        };
        self.env = flush(self.env + k * (e - self.env));

        // dB domain, static curve, one-pole smoothing of the gain in dB, back to linear.
        let level_db = DB_PER_LOG2 * libm::log2f(self.env.max(MIN_LEVEL));
        let target = self.curve.gain_db(level_db);
        self.gr = flush(self.gr + self.k_smooth * (target - self.gr));
        self.peak_gr = self.peak_gr.max(-self.gr);
        let gain = libm::exp2f((self.gr + self.makeup_db) * LOG2_PER_DB);

        // Parallel mix: 100 % gives `0 + 1·gain` exactly, 0 % gives `1 + 0·gain` exactly (the input, bit for bit).
        let m = self.dry + self.wet * gain;
        (l * m, r * m)
    }

    /// Recompute every coefficient from the stored settings and sample rate.
    fn refresh_all(&mut self) {
        let p = self.params;
        self.curve = StaticCurve::new(p.threshold_db, p.ratio, p.knee_db);
        self.k_attack = one_pole_gain(p.attack_ms, self.sample_rate);
        self.k_smooth = one_pole_gain(0.5 * p.attack_ms, self.sample_rate);
        self.k_release = one_pole_gain(p.release_ms, self.sample_rate);
        self.makeup_db = p.makeup_db;
        self.wet = p.mix_pct * 0.01;
        self.dry = 1.0 - self.wet;
    }
}

#[cfg(test)]
mod tests {
    //! White-box tests of the internals. The public contract (static curve, step responses, hash, allocation) is in
    //! `crates/enth-dsp/tests/compressor.rs`.
    use super::*;

    fn comp(attack_ms: f32, release_ms: f32, fs: f32) -> Compressor {
        let mut c = Compressor::new();
        c.prepare(fs);
        c.set_params(&CompressorParams {
            attack_ms,
            release_ms,
            ..CompressorParams::DEFAULT
        });
        c
    }

    /// First frame index at which `value` has risen to or past `target`.
    fn first_reaching(mut value: impl FnMut() -> f32, target: f32, limit: usize) -> usize {
        (0..limit)
            .find(|_| value() >= target)
            .expect("target never reached")
    }

    #[test]
    fn follower_attack_reaches_63_percent_at_the_attack_time() {
        for &fs in &[44_100.0_f32, 48_000.0, 96_000.0] {
            for &attack_ms in &[1.0_f32, 10.0, 100.0] {
                let mut c = comp(attack_ms, 200.0, fs);
                let target = 0.5 * (1.0 - libm::expf(-1.0));
                let n = first_reaching(
                    || {
                        c.tick(0.5, 0.5);
                        c.env
                    },
                    target,
                    200_000,
                );
                // Index n is the (n+1)-th sample: the time constant in samples is attack_ms·fs/1000.
                let expect = f64::from(attack_ms) * f64::from(fs) / 1000.0;
                assert!(
                    (f64::from(n as u32 + 1) - expect).abs() <= (expect * 1e-4).max(1.0),
                    "attack {attack_ms} ms at {fs}: crossed after {} samples, expected {expect}",
                    n + 1
                );
            }
        }
    }

    #[test]
    fn follower_release_falls_to_37_percent_at_the_release_time() {
        for &fs in &[44_100.0_f32, 48_000.0, 96_000.0] {
            for &release_ms in &[10.0_f32, 200.0, 2000.0] {
                let mut c = comp(0.0, release_ms, fs);
                c.tick(0.5, 0.5); // attack 0: the follower jumps to the input
                assert!((c.env - 0.5).abs() < 1e-6);
                let target = 0.5 * libm::expf(-1.0);
                let n = (0..2_000_000)
                    .find(|_| {
                        c.tick(0.0, 0.0);
                        c.env <= target
                    })
                    .expect("release never reached 1/e");
                let expect = f64::from(release_ms) * f64::from(fs) / 1000.0;
                assert!(
                    (f64::from(n as u32 + 1) - expect).abs() <= (expect * 1e-4).max(1.0),
                    "release {release_ms} ms at {fs}: crossed after {} samples, expected {expect}",
                    n + 1
                );
            }
        }
    }

    #[test]
    fn long_times_at_high_rates_keep_their_precision() {
        // 10 s at 192 kHz: k is about 5.2e-7. `1 − exp(−1/n)` in f32 would be quantised to a multiple of 6e-8.
        let k = one_pole_gain(10_000.0, 192_000.0);
        let expect = 1.0 - libm::exp(-1.0 / 1.92e6_f64);
        assert!(
            (f64::from(k) - expect).abs() / expect < 1e-6,
            "k = {k}, expected {expect}"
        );
    }

    #[test]
    fn zero_time_is_instant() {
        assert_eq!(one_pole_gain(0.0, 48_000.0), 1.0);
        assert_eq!(one_pole_gain(-5.0, 48_000.0), 1.0);
    }

    #[test]
    fn states_flush_to_zero_after_a_loud_burst() {
        let mut c = comp(10.0, 500.0, 48_000.0);
        for _ in 0..4_800 {
            c.tick(0.9, -0.9);
        }
        assert!(c.gr < -1.0);
        let mut subnormal_seen = false;
        let mut zero_at = None;
        for n in 0..48_000 * 30 {
            let (a, b) = c.tick(0.0, 0.0);
            subnormal_seen |=
                c.env.is_subnormal() || c.gr.is_subnormal() || a.is_subnormal() || b.is_subnormal();
            if zero_at.is_none() && c.env == 0.0 && c.gr == 0.0 {
                zero_at = Some(n);
            }
        }
        assert!(!subnormal_seen, "a state went subnormal");
        assert!(
            zero_at.is_some(),
            "the states never flushed to exactly zero within 30 s"
        );
    }

    #[test]
    fn set_params_with_identical_values_changes_nothing() {
        let mut c = Compressor::new();
        c.prepare(48_000.0);
        let p = CompressorParams {
            threshold_db: -30.0,
            ratio: 6.0,
            ..CompressorParams::DEFAULT
        };
        c.set_params(&p);
        let before = c.clone();
        c.set_params(&p);
        assert_eq!(before.k_attack, c.k_attack);
        assert_eq!(before.k_release, c.k_release);
        assert_eq!(before.k_smooth, c.k_smooth);
        assert_eq!(before.curve, c.curve);
        assert_eq!(before.params, c.params);
    }

    fn assert_same_coefficients(a: &Compressor, b: &Compressor, what: &str) {
        assert_eq!(a.curve, b.curve, "{what}: curve");
        assert_eq!(a.k_attack, b.k_attack, "{what}: k_attack");
        assert_eq!(a.k_release, b.k_release, "{what}: k_release");
        assert_eq!(a.k_smooth, b.k_smooth, "{what}: k_smooth");
        assert_eq!(a.makeup_db, b.makeup_db, "{what}: makeup");
        assert_eq!(a.wet, b.wet, "{what}: wet");
        assert_eq!(a.dry, b.dry, "{what}: dry");
        assert_eq!(a.params, b.params, "{what}: params");
    }

    #[test]
    fn partial_updates_match_a_fresh_build() {
        // Changing one field at a time must, after every single step, land on the coefficients of a compressor that
        // was given the same settings at once. Forwards and backwards, so no later step can repair an earlier one.
        let target = CompressorParams {
            threshold_db: -33.0,
            ratio: 9.0,
            knee_db: 11.0,
            attack_ms: 3.0,
            release_ms: 450.0,
            makeup_db: 5.0,
            mix_pct: 40.0,
        };
        for (from, to) in [
            (CompressorParams::DEFAULT, target),
            (target, CompressorParams::DEFAULT),
        ] {
            let mut stepwise = Compressor::new();
            stepwise.prepare(44_100.0);
            stepwise.set_params(&from);
            let mut p = from;
            for field in 0..7 {
                match field {
                    0 => p.threshold_db = to.threshold_db,
                    1 => p.ratio = to.ratio,
                    2 => p.knee_db = to.knee_db,
                    3 => p.attack_ms = to.attack_ms,
                    4 => p.release_ms = to.release_ms,
                    5 => p.makeup_db = to.makeup_db,
                    _ => p.mix_pct = to.mix_pct,
                }
                stepwise.set_params(&p);
                let mut direct = Compressor::new();
                direct.prepare(44_100.0);
                direct.set_params(&p);
                assert_same_coefficients(&stepwise, &direct, &format!("field {field}"));
            }
        }
    }

    #[test]
    fn flush_zeroes_only_magnitudes_below_the_threshold() {
        assert_eq!(flush(1.0e-15).to_bits(), 1.0e-15_f32.to_bits());
        assert_eq!(flush(-1.0e-15).to_bits(), (-1.0e-15_f32).to_bits());
        assert_eq!(flush(0.99e-15).to_bits(), 0);
        assert_eq!(flush(-0.99e-15).to_bits(), 0);
        assert_eq!(flush(f32::MIN_POSITIVE / 4.0).to_bits(), 0);
        assert_eq!(flush(0.25).to_bits(), 0.25_f32.to_bits());
    }

    #[test]
    fn prepare_recomputes_coefficients_for_the_new_rate() {
        let mut c = Compressor::new();
        c.prepare(48_000.0);
        let k48 = c.k_attack;
        c.prepare(96_000.0);
        assert!(
            c.k_attack < k48 * 0.6 && c.k_attack > k48 * 0.4,
            "{} vs {}",
            c.k_attack,
            k48
        );
        assert_eq!(c.sample_rate(), 96_000.0);
    }

    #[test]
    fn sanitized_clamps_and_replaces_non_finite_values() {
        let p = CompressorParams {
            threshold_db: f32::NAN,
            ratio: 0.2,
            knee_db: -3.0,
            attack_ms: f32::INFINITY,
            release_ms: 1.0e9,
            makeup_db: -1.0e3,
            mix_pct: 250.0,
        }
        .sanitized();
        let d = CompressorParams::DEFAULT;
        assert_eq!(p.threshold_db, d.threshold_db);
        assert_eq!(p.ratio, 1.0);
        assert_eq!(p.knee_db, 0.0);
        assert_eq!(p.attack_ms, d.attack_ms);
        assert_eq!(p.release_ms, 10_000.0);
        assert_eq!(p.makeup_db, -48.0);
        assert_eq!(p.mix_pct, 100.0);
    }
}
