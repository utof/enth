//! **Owner: lane SS** (the sound scorer).
//!
//! An in-house ITU-R BS.1770-4 integrated loudness for the scored path, on `libm` only (summary "Scored-path
//! loudness", report 05; ADR-0007's 2026-10-20 note, fix 5). Metric v1 took its −23 LUFS gain from `ebur128`, which
//! uses `std` maths, so a sound % could differ in its last bits between platforms. This one gives the same bits on
//! every platform: `libm` for the three transcendentals (`tan` and `pow` in the filter design, `log10` at the end),
//! IEEE `+ − × ÷` everywhere else, and one fixed summation order. `ebur128` stays for display and true peak
//! (`enth_analysis::loudness`).
//!
//! **What it measures** (BS.1770-4 §2 and Annex 1, mono, channel weight 1):
//!
//! 1. **K-weighting:** a high shelf (+4 dB above about 1.7 kHz, the head) then a high pass (the RLB curve, about
//!    38 Hz), two biquads in series in `f64`. The coefficients come from the analog prototypes through the bilinear
//!    transform, so any sample rate works; at 48 kHz they reproduce the standard's Tables 1 and 2 within 1e-9
//!    (`the_48k_coefficients_are_the_standards`). The prototype values (shelf f0 1681.97 Hz, gain 3.99984 dB,
//!    Q 0.70718; high pass f0 38.135 Hz, Q 0.50033) are the ones libebur128 (MIT) and De Man's 2018 AES paper fit to
//!    those tables; the code is written from the formulas, not ported.
//! 2. **Blocks:** the mean square of the weighted signal over 400 ms blocks every 100 ms (75 % overlap). Squares
//!    are summed per 100 ms segment, in sample order, and a block adds its four segments in order.
//! 3. **Gates:** blocks above −70 LUFS (absolute), then above the mean of those minus 10 LU (relative). The
//!    integrated loudness is `−0.691 + 10·log10(mean square)` over the blocks that pass both.
//! 4. **Short clips:** a clip shorter than one block, or one where no block passes, falls back to the ungated mean
//!    square over the whole clip (as `enth_analysis::loudness` does). Below −70 LUFS a clip is **silent**: `None`.
//!
//! **The shared gain** (property P11, design.md §9): the scorer brings each clip to [`SCORING_LUFS`] with
//! [`gain_db`]`(lufs, SCORING_LUFS)`, and the A/B level match brings both sides to the quieter one's loudness with
//! the same function ([`ab_gains_db`]): the louder side is attenuated by the difference, the quieter never boosted.
//! So the heard A/B and the scored comparison are the same level match, up to one common gain.

/// The loudness every scored clip is brought to (EBU R 128's programme level).
pub const SCORING_LUFS: f64 = -23.0;
/// BS.1770's absolute gate, and the level at or below which a clip counts as silent.
pub const ABSOLUTE_GATE_LUFS: f64 = -70.0;
/// BS.1770's relative gate, below the absolute-gated loudness.
pub const RELATIVE_GATE_LU: f64 = -10.0;

/// The block length and the step between blocks, in milliseconds.
const BLOCK_MS: u32 = 400;
const STEP_MS: u32 = 100;
/// BS.1770's offset in `−0.691 + 10·log10(z)`.
const OFFSET_DB: f64 = -0.691;
/// Filter states below this are flushed to 0, so a long silent tail never runs on denormals (a comparison, the same
/// everywhere; 1e-30 is 600 dB below full scale).
const FLUSH: f64 = 1e-30;

/// One biquad, `a0` normalised to 1.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Biquad {
    b: [f64; 3],
    a: [f64; 2],
}

/// Direct form I state of one biquad.
#[derive(Clone, Copy, Debug, Default)]
struct State {
    x1: f64,
    x2: f64,
    y1: f64,
    y2: f64,
}

impl State {
    #[inline]
    fn tick(&mut self, q: &Biquad, x: f64) -> f64 {
        let y =
            q.b[0] * x + q.b[1] * self.x1 + q.b[2] * self.x2 - q.a[0] * self.y1 - q.a[1] * self.y2;
        self.x2 = self.x1;
        self.x1 = x;
        self.y2 = self.y1;
        self.y1 = if y.abs() < FLUSH { 0.0 } else { y };
        self.y1
    }
}

/// The K-weighting filter's coefficients at a sample rate.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct KWeighting {
    shelf: Biquad,
    highpass: Biquad,
}

impl KWeighting {
    /// The coefficients at `sample_rate` Hz (any rate above 2 × 1.7 kHz; the scorer uses 48 kHz).
    ///
    /// # Panics
    /// If `sample_rate` is below 4 kHz, where the shelf's prototype frequency is above Nyquist.
    pub fn new(sample_rate: u32) -> KWeighting {
        assert!(sample_rate >= 4_000, "sample rate {sample_rate}");
        let rate = f64::from(sample_rate);
        // High shelf: the bilinear transform of the analog shelf, with the band gain at the shelf's midpoint
        // (Vb = Vh^0.49967, the fitted exponent that reproduces BS.1770's 48 kHz table).
        let (f0, g_db, q) = (
            1_681.974_450_955_533,
            3.999_843_853_973_347,
            0.707_175_236_955_419_6,
        );
        let k = libm::tan(core::f64::consts::PI * f0 / rate);
        let vh = libm::pow(10.0, g_db / 20.0);
        let vb = libm::pow(vh, 0.499_666_774_154_541_6);
        let a0 = 1.0 + k / q + k * k;
        let shelf = Biquad {
            b: [
                (vh + vb * k / q + k * k) / a0,
                2.0 * (k * k - vh) / a0,
                (vh - vb * k / q + k * k) / a0,
            ],
            a: [2.0 * (k * k - 1.0) / a0, (1.0 - k / q + k * k) / a0],
        };
        // High pass (RLB): numerator 1, −2, 1 as in the standard's Table 2.
        let (f0, q) = (38.135_470_876_024_44, 0.500_327_037_323_877_3);
        let k = libm::tan(core::f64::consts::PI * f0 / rate);
        let a0 = 1.0 + k / q + k * k;
        let highpass = Biquad {
            b: [1.0, -2.0, 1.0],
            a: [2.0 * (k * k - 1.0) / a0, (1.0 - k / q + k * k) / a0],
        };
        KWeighting { shelf, highpass }
    }
}

/// Integrated loudness of a mono clip at `sample_rate`, in LUFS (see the [module docs](self)); `None` when silent
/// (at or below −70 LUFS, empty, or not finite). Deterministic: the same bits on every platform.
pub fn integrated_lufs(x: &[f32], sample_rate: u32) -> Option<f64> {
    let z = gated_mean_square(x, sample_rate)?;
    let lufs = OFFSET_DB + 10.0 * libm::log10(z);
    (lufs.is_finite() && lufs > ABSOLUTE_GATE_LUFS).then_some(lufs)
}

/// The mean square that [`integrated_lufs`] takes the log of: over the blocks that pass both gates, or over the whole
/// clip when none does. `None` for an empty clip, a non-finite sample, or digital silence.
fn gated_mean_square(x: &[f32], sample_rate: u32) -> Option<f64> {
    if x.is_empty() || sample_rate == 0 {
        return None;
    }
    let kw = KWeighting::new(sample_rate);
    let step = (sample_rate * STEP_MS / 1000).max(1) as usize;
    let per_block = (BLOCK_MS / STEP_MS) as usize;
    // Squares of the weighted signal, summed per 100 ms segment in sample order; the tail after the last full
    // segment only counts towards the whole-clip fallback.
    let (mut s1, mut s2) = (State::default(), State::default());
    let mut segments: Vec<f64> = Vec::with_capacity(x.len() / step + 1);
    let mut acc = 0.0f64;
    let mut total = 0.0f64;
    let mut in_segment = 0usize;
    for &v in x {
        let v = f64::from(v);
        if !v.is_finite() {
            return None;
        }
        let y = s2.tick(&kw.highpass, s1.tick(&kw.shelf, v));
        let e = y * y;
        acc += e;
        in_segment += 1;
        if in_segment == step {
            segments.push(acc);
            total += acc;
            acc = 0.0;
            in_segment = 0;
        }
    }
    total += acc;
    let block_len = (step * per_block) as f64;
    let blocks: Vec<f64> = if segments.len() >= per_block {
        (0..=segments.len() - per_block)
            .map(|j| {
                let mut s = 0.0;
                for seg in &segments[j..j + per_block] {
                    s += *seg;
                }
                s / block_len
            })
            .collect()
    } else {
        Vec::new()
    };
    let z_abs = libm::pow(10.0, (ABSOLUTE_GATE_LUFS - OFFSET_DB) / 10.0);
    let mean_above = |gate: f64| {
        let (mut s, mut n) = (0.0f64, 0usize);
        for z in blocks.iter().filter(|z| **z > gate) {
            s += *z;
            n += 1;
        }
        (n > 0).then(|| s / n as f64)
    };
    let gated = mean_above(z_abs).and_then(|abs_mean| {
        let z_rel = abs_mean * libm::pow(10.0, RELATIVE_GATE_LU / 10.0);
        mean_above(z_abs.max(z_rel))
    });
    let z = gated.unwrap_or(total / x.len() as f64);
    (z > 0.0).then_some(z)
}

/// The gain in dB that brings a clip measured at `lufs` to `reference` LUFS: `reference − lufs`. **The one gain
/// function** of the scored comparison and the A/B level match (P11).
pub fn gain_db(lufs: f64, reference: f64) -> f64 {
    reference - lufs
}

/// A gain in dB as a linear factor, `10^(db/20)` through `libm`.
pub fn db_to_amp(db: f64) -> f64 {
    libm::pow(10.0, db / 20.0)
}

/// The linear gain that brings `x` to [`SCORING_LUFS`]: [`db_to_amp`]`(`[`gain_db`]`(lufs, SCORING_LUFS))`; `None` when
/// `x` is silent. What the sound scorer applies before its features.
pub fn scoring_gain(x: &[f32], sample_rate: u32) -> Option<f64> {
    integrated_lufs(x, sample_rate).map(|l| db_to_amp(gain_db(l, SCORING_LUFS)))
}

/// The A/B level match in dB, `(target, mine)`: both sides brought to the quieter one's loudness with [`gain_db`], so
/// the louder side is attenuated by the difference and the quieter one gets exactly 0 dB (never a boost, which can
/// clip). Symmetric: swapping the sides swaps the gains. Either side unmeasured, silent or not finite leaves both at
/// 0 dB. Lane SG's `game::ab_gains` calls it; measure both sides with [`integrated_lufs`] on their 48 kHz renders.
pub fn ab_gains_db(target_lufs: Option<f64>, mine_lufs: Option<f64>) -> (f64, f64) {
    match (target_lufs, mine_lufs) {
        (Some(t), Some(m)) if t.is_finite() && m.is_finite() => {
            let quieter = t.min(m);
            (gain_db(t, quieter), gain_db(m, quieter))
        }
        _ => (0.0, 0.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: u32 = 48_000;

    fn sine(hz: f64, amp: f64, seconds: f64) -> Vec<f32> {
        let n = (seconds * f64::from(SR)) as usize;
        (0..n)
            .map(|i| {
                (amp * libm::sin(2.0 * core::f64::consts::PI * hz * i as f64 / f64::from(SR)))
                    as f32
            })
            .collect()
    }

    /// The direct form of the two biquads multiplied out, against BS.1770-4 Tables 1 and 2 (48 kHz).
    #[test]
    fn the_48k_coefficients_are_the_standards() {
        let k = KWeighting::new(48_000);
        let shelf_b = [
            1.535_124_859_586_97,
            -2.691_696_189_406_38,
            1.198_392_810_852_85,
        ];
        let shelf_a = [-1.690_659_293_182_41, 0.732_480_774_215_85];
        let hp_a = [-1.990_047_454_833_98, 0.990_072_250_366_21];
        for (got, want) in k
            .shelf
            .b
            .iter()
            .zip(shelf_b)
            .chain(k.shelf.a.iter().zip(shelf_a))
        {
            assert!((got - want).abs() < 1e-9, "{got} {want}");
        }
        for (got, want) in k.highpass.a.iter().zip(hp_a) {
            assert!((got - want).abs() < 1e-9, "{got} {want}");
        }
        assert_eq!(k.highpass.b, [1.0, -2.0, 1.0]);
    }

    #[test]
    fn a_997hz_sine_reads_the_standards_level() {
        // BS.1770: a 0 dBFS 997 Hz sine reads −3.01 LUFS, so amplitude 0.1 reads −23.01.
        let l = integrated_lufs(&sine(997.0, 0.1, 3.0), SR).unwrap();
        assert!((l + 23.01).abs() < 0.02, "{l}");
        let l = integrated_lufs(&sine(997.0, 1.0, 3.0), SR).unwrap();
        assert!((l + 3.01).abs() < 0.02, "{l}");
    }

    #[test]
    fn silence_short_clips_and_bad_input() {
        assert_eq!(integrated_lufs(&[], SR), None);
        assert_eq!(integrated_lufs(&vec![0.0; 48_000], SR), None);
        assert_eq!(
            integrated_lufs(&sine(997.0, 1e-5, 2.0), SR),
            None,
            "-103 LUFS is silent"
        );
        let mut x = sine(997.0, 0.5, 1.0);
        x[100] = f32::NAN;
        assert_eq!(integrated_lufs(&x, SR), None);
        // A 50 ms blip has no full block: the ungated fallback still reads it.
        let blip = integrated_lufs(&sine(997.0, 0.5, 0.05), SR).unwrap();
        assert!((blip + 9.03).abs() < 0.2, "{blip}");
    }

    #[test]
    fn gates_ignore_silence_around_a_sound() {
        // One second of tone between seconds of silence: the absolute gate drops the silent blocks; the blocks that
        // straddle an edge count, so 7 full blocks and 2 × (3/4 + 2/4 + 1/4) partial ones read 10/13 of the tone's
        // mean square (BS.1770's arithmetic, which ebur128 agrees with).
        let tone = -23.01;
        let mut x = vec![0.0f32; 2 * 48_000];
        x.extend(sine(997.0, 0.1, 1.0));
        x.extend(vec![0.0f32; 2 * 48_000]);
        let l = integrated_lufs(&x, SR).unwrap();
        let want = tone + 10.0 * libm::log10(10.0 / 13.0);
        assert!((l - want).abs() < 0.02, "{l} {want}");
        // A passage 20 dB down is gated out relatively (17 full blocks and the three that straddle the change
        // remain); one 6 dB down is not, and pulls the reading down.
        let loud = sine(997.0, 0.1, 2.0);
        let mut far: Vec<f32> = loud.clone();
        far.extend(sine(997.0, 0.01, 2.0));
        let l_far = integrated_lufs(&far, SR).unwrap();
        let want = tone + 10.0 * libm::log10((17.0 + 0.7575 + 0.505 + 0.2525) / 20.0);
        assert!((l_far - want).abs() < 0.02, "{l_far} {want}");
        let mut near = loud;
        near.extend(sine(997.0, 0.05, 2.0));
        let l_near = integrated_lufs(&near, SR).unwrap();
        assert!(l_near < l_far - 1.0, "{l_near} {l_far}");
    }

    #[test]
    fn level_scales_exactly_and_the_scoring_gain_lands_on_minus_23() {
        let x = sine(440.0, 0.3, 2.0);
        let l = integrated_lufs(&x, SR).unwrap();
        let g = scoring_gain(&x, SR).unwrap();
        let y: Vec<f32> = x.iter().map(|v| (f64::from(*v) * g) as f32).collect();
        let ly = integrated_lufs(&y, SR).unwrap();
        assert!((ly - SCORING_LUFS).abs() < 1e-4, "{l} -> {ly}");
        let quieter: Vec<f32> = x.iter().map(|v| v * 0.5).collect();
        let lq = integrated_lufs(&quieter, SR).unwrap();
        assert!((l - lq - 6.0206).abs() < 1e-3, "{l} {lq}");
    }

    #[test]
    fn ab_gains_attenuate_the_louder_side_only() {
        assert_eq!(ab_gains_db(Some(-14.0), Some(-20.0)), (-6.0, 0.0));
        assert_eq!(ab_gains_db(Some(-20.0), Some(-14.0)), (0.0, -6.0));
        assert_eq!(ab_gains_db(Some(-18.0), Some(-18.0)), (0.0, 0.0));
        assert!(ab_gains_db(Some(-18.0), Some(-18.0)).1.is_sign_positive());
        assert!(ab_gains_db(Some(-14.0), Some(-20.0)).1.is_sign_positive());
        assert_eq!(ab_gains_db(None, Some(-14.0)), (0.0, 0.0));
        assert_eq!(
            ab_gains_db(Some(f64::NEG_INFINITY), Some(-14.0)),
            (0.0, 0.0)
        );
        assert_eq!(ab_gains_db(Some(-14.0), Some(f64::NAN)), (0.0, 0.0));
        assert_eq!(ab_gains_db(Some(-14.0), None), (0.0, 0.0));
    }
}
