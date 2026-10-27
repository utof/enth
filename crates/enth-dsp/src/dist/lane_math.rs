//! `ln cosh` on two lanes, bit for bit what the scalar `libm` path gives each lane (S2a lane PRF; `docs/dsp/dist.md`).
//!
//! `dist`'s ADAA step needs the antiderivative of `tanh`, `ln cosh u = |u| + ln(1 + e^(−2|u|)) − ln 2`, once per
//! oversampled sample: with the paired kernel that is two `exp` and two `log` calls per sample, the biggest cost of the
//! module. This file computes the same function for the two channels of a stereo voice at once, in an `f64x2`.
//!
//! **It is not an approximation.** [`exp_nonpos`] and [`log_1_2`] are `libm` 0.2.16's `exp` and `log` (pinned with
//! `=` in the workspace; FreeBSD msun's `e_exp.c` and `e_log.c`, the same algorithm and constants), restricted to the
//! arguments `ln cosh` gives them and written lane-wise with the same operations in the same order: every lane
//! performs exactly the IEEE `+ − × ÷` sequence the scalar function does, with the scalar function's branches turned
//! into compares and selects. The unit tests compare against `libm` bit for bit over millions of arguments and over
//! every threshold the scalar code branches on; a `libm` bump that changed either function fails them. What the lane
//! versions do not cover (an argument outside the restricted range, NaN) is computed by the scalar function itself,
//! lane by lane.
//!
//! Provenance and licence: the algorithms and constants are msun's, as carried by `libm` (MIT; the msun files keep
//! Sun Microsystems' permissive notice, reproduced in `THIRD_PARTY_NOTICES.md`).

use wide::f64x2;

use super::ln_cosh;

type V = f64x2;

const LN2_HI: f64 = f64::from_bits(0x3fe6_2e42_fee0_0000);
const LN2_LO: f64 = f64::from_bits(0x3dea_39ef_3579_3c76);
const INV_LN2: f64 = f64::from_bits(0x3ff7_1547_652b_82fe);
const P1: f64 = f64::from_bits(0x3FC5_5555_5555_553E);
const P2: f64 = f64::from_bits(0xBF66_C16C_16BE_BD93);
const P3: f64 = f64::from_bits(0x3F11_566A_AF25_DE2C);
const P4: f64 = f64::from_bits(0xBEBB_BD41_C5D2_6BF1);
const P5: f64 = f64::from_bits(0x3E66_3769_72BE_A4D0);

const LG1: f64 = f64::from_bits(0x3FE5_5555_5555_5593);
const LG2: f64 = f64::from_bits(0x3FD9_9999_9997_FA04);
const LG3: f64 = f64::from_bits(0x3FD2_4924_9422_9359);
const LG4: f64 = f64::from_bits(0x3FCC_71C5_1D8E_78AF);
const LG5: f64 = f64::from_bits(0x3FC7_4664_96CB_03DE);
const LG6: f64 = f64::from_bits(0x3FC3_9A09_D078_C69F);
const LG7: f64 = f64::from_bits(0x3FC2_F112_DF3E_5244);

/// `2^52`: adding and subtracting it rounds a value in `[0, 2^51)` to the nearest integer (ties to even), exactly.
const MAGIC: f64 = 4_503_599_627_370_496.0;

/// `exp`'s argument classes, as the scalar function takes them from the high word of `|x|` (`hx`): a comparison of
/// `hx` with a constant is a comparison of `|x|` with the double whose high word is that constant plus one (for `>`)
/// or the constant (for `>=`) and whose low word is zero.
///
/// `hx <= 0x3e300000` (`|x| ≤ 2^-28`): the scalar function returns `1 + x`.
const TINY_BELOW: u64 = 0x3e30_0001_0000_0000;
/// `hx > 0x3fd62e42` (`|x| > ½ ln 2`): the argument is reduced by `k ln 2`.
const REDUCE_FROM: u64 = 0x3fd6_2e43_0000_0000;
/// `hx >= 0x3ff0a2b2` (`|x| ≥ 1.5 ln 2`): `k` is the rounded `x / ln 2`; below it, `k` is `±1`.
const ROUND_FROM: u64 = 0x3ff0_a2b2_0000_0000;
/// `log`'s `hx >= 0x3ff6a09e` (`x ≥ √2`, within `[1, 2]`): the argument is halved and `k` is 1.
const SQRT2_FROM: u64 = 0x3ff6_a09e_0000_0000;

/// The largest `|x|` [`exp_nonpos`] is meant for; `ln_cosh_x2` keeps its argument below 40. Far inside the range
/// where the scalar function takes its plain path (`|x| < 708.39`).
#[cfg(test)]
const EXP_LIMIT: f64 = 100.0;

#[inline(always)]
fn splat(x: f64) -> V {
    V::splat(x)
}

#[inline(always)]
fn bits(x: u64) -> V {
    V::splat(f64::from_bits(x))
}

/// `⌊w⌋` for `0 ≤ w < 2^51`, exactly: round to the nearest integer with the `2^52` trick, then take one off where that
/// went up.
#[inline(always)]
fn floor_nonneg(w: V) -> V {
    let r = (w + splat(MAGIC)) - splat(MAGIC);
    r - (r.simd_gt(w) & splat(1.0))
}

/// `exp(x)` on each lane, for `−EXP_LIMIT ≤ x ≤ 0` (including `−0.0`): `libm::exp`'s bits.
#[inline(always)]
pub fn exp_nonpos(x: V) -> V {
    let ax = x.abs();
    let tiny = ax.simd_lt(bits(TINY_BELOW));
    let reduce = ax.simd_ge(bits(REDUCE_FROM));
    let round = ax.simd_ge(bits(ROUND_FROM));

    // k = (INVLN2·x − ½) as i32 where |x| ≥ 1.5 ln 2 (x < 0, so the scalar function adds HALF[1] = −0.5): the
    // truncation of a negative number is minus the floor of its magnitude. k = −1 where |x| is smaller but above
    // ½ ln 2, else 0.
    let v = splat(INV_LN2) * x + splat(-0.5);
    let k_round = -floor_nonneg(-v);
    let k = round.select(k_round, reduce.select(splat(-1.0), V::ZERO));

    let hi = x - k * splat(LN2_HI);
    let lo = k * splat(LN2_LO);
    let r = hi - lo;

    let xx = r * r;
    let poly = splat(P1) + xx * (splat(P2) + xx * (splat(P3) + xx * (splat(P4) + xx * splat(P5))));
    let c = r - xx * poly;
    let y = splat(1.0) + (r * c / (splat(2.0) - c) - lo + hi);

    // scalbn(y, k) is the single product y·2^k for k in [−1022, 1023]; k = 0 returns y, which y·1 is as well. 2^k is
    // built as the bits of (1023 + k) << 52: adding 2^52 to the integer 1023 + k puts it in the mantissa field.
    let e = (k + splat(1023.0)) + splat(MAGIC);
    let scale = V::from_bits(e.to_bits() << 52_u32);
    tiny.select(splat(1.0) + x, y * scale)
}

/// `log(x)` on each lane, for `1 ≤ x ≤ 2`: `libm::log`'s bits.
#[inline(always)]
pub fn log_1_2(x: V) -> V {
    // Reduce into [√2/2, √2]: from √2 up the scalar function takes k = 1 and the mantissa with its exponent lowered
    // by one, which is x·½ exactly.
    let up = x.simd_ge(bits(SQRT2_FROM));
    let m = up.select(x * splat(0.5), x);
    let dk = up.select(splat(1.0), V::ZERO);

    let f = m - splat(1.0);
    let hfsq = splat(0.5) * f * f;
    let s = f / (splat(2.0) + f);
    let z = s * s;
    let w = z * z;
    let t1 = w * (splat(LG2) + w * (splat(LG4) + w * splat(LG6)));
    let t2 = z * (splat(LG1) + w * (splat(LG3) + w * (splat(LG5) + w * splat(LG7))));
    let r = t2 + t1;
    s * (hfsq + r) + dk * splat(LN2_LO) - hfsq + f + dk * splat(LN2_HI)
}

/// `ln cosh u` on each lane: the scalar `dist::ln_cosh`'s bits.
#[inline(always)]
pub fn ln_cosh_x2(u: V) -> V {
    let a = u.abs();
    let lim = splat(20.0);
    // `a > 20` is the scalar function's branch; NaN is neither side and goes to the scalar function below.
    let small = a.simd_le(lim);
    let large = a.simd_gt(lim);
    let a_in = small.select(a, V::ZERO);
    let e = exp_nonpos(splat(-2.0) * a_in);
    let l = log_1_2(splat(1.0) + e);
    let r = large.select(
        a - splat(core::f64::consts::LN_2),
        a + l - splat(core::f64::consts::LN_2),
    );
    if (small | large).all() {
        r
    } else {
        let (ua, ra) = (u.to_array(), r.to_array());
        V::new(core::array::from_fn(|i| {
            if ua[i].is_nan() {
                ln_cosh(ua[i])
            } else {
                ra[i]
            }
        }))
    }
}

#[cfg(test)]
mod tests {
    use core::f64::consts::LN_10;

    use super::*;
    use crate::util::math::{exp as scalar_exp, log as scalar_log};
    use crate::util::rng::Rng;

    fn lanes(a: f64, b: f64) -> V {
        V::new([a, b])
    }

    /// Both lanes of `f` on `(a, b)` against `scalar` on each.
    fn check2(name: &str, f: impl Fn(V) -> V, scalar: impl Fn(f64) -> f64, a: f64, b: f64) {
        let got = f(lanes(a, b)).to_array();
        for (g, x) in got.iter().zip([a, b]) {
            let want = scalar(x);
            assert_eq!(
                g.to_bits(),
                want.to_bits(),
                "{name}({x:e}) = {g:e} ({:#x}), libm {want:e} ({:#x})",
                g.to_bits(),
                want.to_bits()
            );
        }
    }

    /// The arguments around every threshold the scalar code branches on, one ulp either side, and the extremes.
    fn thresholds(bits: &[u64]) -> Vec<f64> {
        let mut v = Vec::new();
        for &b in bits {
            for d in -3_i64..=3 {
                v.push(f64::from_bits((b as i64 + d) as u64));
            }
        }
        v
    }

    #[test]
    fn exp_matches_libm_at_every_threshold_and_over_a_dense_sweep() {
        let mut args = thresholds(&[TINY_BELOW, REDUCE_FROM, ROUND_FROM]);
        // Also the thresholds' negatives are the arguments (x ≤ 0), and the sign-free ones for completeness of |x|.
        args = args.iter().flat_map(|&a| [-a, -a.abs()]).collect();
        args.extend([
            -0.0,
            0.0,
            -1e-300,
            -f64::MIN_POSITIVE,
            -f64::from_bits(1),
            -EXP_LIMIT,
            -0.5,
            -1.0,
            -core::f64::consts::LN_2,
            -0.5 * core::f64::consts::LN_2,
            -1.5 * core::f64::consts::LN_2,
            -40.0,
        ]);
        // Multiples of ln 2 and the points halfway between: where k changes.
        for i in 0..=200 {
            let m = f64::from(i) * core::f64::consts::LN_2;
            for d in [-1e-12, 0.0, 1e-12] {
                args.push(-(m + d));
                args.push(-(m + 0.5 * core::f64::consts::LN_2 + d));
            }
        }
        for &x in &args {
            if x > 0.0 || x < -EXP_LIMIT {
                continue;
            }
            check2("exp", exp_nonpos, scalar_exp, x, x);
            check2("exp", exp_nonpos, scalar_exp, x, -x.abs() * 0.5);
        }
        // A dense linear sweep of the range ln cosh uses (x = −2a for a in 0..=20) and a log-spaced one down to 1e-30.
        for i in 0..=400_000 {
            let x = -40.0 * f64::from(i) / 400_000.0;
            check2(
                "exp",
                exp_nonpos,
                scalar_exp,
                x,
                -x * 0.0 - 40.0 * (1.0 - f64::from(i) / 400_000.0),
            );
        }
        for i in 0..=100_000 {
            let x = -scalar_exp(LN_10 * (-30.0 + 31.7 * f64::from(i) / 100_000.0));
            check2("exp", exp_nonpos, scalar_exp, x, x * 3.0);
        }
    }

    #[test]
    fn exp_matches_libm_over_random_bit_patterns_and_values() {
        let mut rng = Rng::new(0x5EED_EE11);
        for _ in 0..2_000_000 {
            // Random magnitude in [1e-20, 100], log-uniform, and uniform on [0, 45].
            let u = rng.unit_f64();
            let w = rng.unit_f64();
            let a = -scalar_exp(LN_10 * (-20.0 + 22.0 * u)).min(EXP_LIMIT);
            let b = -45.0 * w;
            check2("exp", exp_nonpos, scalar_exp, a, b);
        }
    }

    #[test]
    fn log_matches_libm_at_the_threshold_and_over_a_dense_sweep() {
        let mut args = thresholds(&[SQRT2_FROM, 0x3ff0_0000_0000_0000, 0x4000_0000_0000_0000]);
        args.extend([1.0, 2.0, 1.5, core::f64::consts::SQRT_2, 1.0 + f64::EPSILON]);
        for &x in &args {
            if !(1.0..=2.0).contains(&x) {
                continue;
            }
            check2("log", log_1_2, scalar_log, x, x);
            check2("log", log_1_2, scalar_log, x, 3.0 - x);
        }
        for i in 0..=400_000 {
            let x = 1.0 + f64::from(i) / 400_000.0;
            check2("log", log_1_2, scalar_log, x, 3.0 - x);
        }
        // 1 + e^(−2a) for a in 0..=20: the values ln cosh gives log.
        for i in 0..=100_000 {
            let a = 20.0 * f64::from(i) / 100_000.0;
            let x = 1.0 + scalar_exp(-2.0 * a);
            check2("log", log_1_2, scalar_log, x, x);
        }
    }

    #[test]
    fn log_matches_libm_over_random_values() {
        let mut rng = Rng::new(0x05EE_D106);
        for _ in 0..2_000_000 {
            let a = 1.0 + rng.unit_f64();
            let b = 1.0 + rng.unit_f64() * rng.unit_f64();
            check2("log", log_1_2, scalar_log, a, b);
        }
    }

    #[test]
    fn ln_cosh_matches_the_scalar_function_everywhere_including_the_edges() {
        let mut rng = Rng::new(0x05EE_DC05);
        let mut args = vec![
            0.0,
            -0.0,
            1e-300,
            -1e-300,
            20.0,
            -20.0,
            20.000_000_000_000_004,
            19.999_999_999_999_996,
            -19.999_999_999_999_996,
            1e5,
            -1e5,
            1e300,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NAN,
            -f64::NAN,
            f64::from_bits(0x7ff8_0000_0000_1234),
            f64::from_bits(1),
        ];
        for _ in 0..2_000_000 {
            let u = rng.unit_f64();
            let w = rng.unit_f64();
            args.push((u - 0.5) * 60.0);
            args.push(
                scalar_exp(LN_10 * (-12.0 + 14.0 * w))
                    * if rng.unit_f64() < 0.5 { -1.0 } else { 1.0 },
            );
        }
        for pair in args.chunks(2) {
            let (a, b) = (pair[0], *pair.get(1).unwrap_or(&pair[0]));
            let got = ln_cosh_x2(lanes(a, b)).to_array();
            for (g, x) in got.iter().zip([a, b]) {
                let want = ln_cosh(x);
                assert_eq!(
                    g.to_bits(),
                    want.to_bits(),
                    "ln_cosh({x:e}): {g:e} vs {want:e}"
                );
            }
        }
        // A NaN in one lane does not disturb the other.
        let got = ln_cosh_x2(lanes(f64::NAN, 3.5)).to_array();
        assert_eq!(got[1].to_bits(), ln_cosh(3.5).to_bits());
        let got = ln_cosh_x2(lanes(-7.25, f64::NAN)).to_array();
        assert_eq!(got[0].to_bits(), ln_cosh(-7.25).to_bits());
    }
}
