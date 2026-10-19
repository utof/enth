//! Denormal flushing in code, never through CPU flags.
//!
//! AGENTS.md forbids depending on, or setting, FTZ/DAZ (the `no_denormals` crate does not build for wasm32, and the
//! flags differ between platforms), so a module that recurses on its own output (feedback delays, biquads, filters
//! with state) flushes tiny values itself. Subnormal arithmetic is 10-100x slower on x86, and an unflushed reverb
//! tail measured 4.2x slower overall with 2.3 million subnormal output samples (P1a report 02).
//!
//! The rule is one threshold for both widths: a value whose magnitude is below `1e-15` (-300 dB) becomes `+0.0`.
//! That is far above the subnormal range of `f32` (1.2e-38) and of `f64` (2.2e-308), so a flushed signal can
//! never reach subnormals through a few more multiplications by a gain, and far below anything audible. `-0.0` is
//! also turned into `+0.0`. NaN and infinities pass through untouched: a flush is not a sanitiser.
//!
//! Flush on every write into a feedback path and on the output of every recursive filter, not on every sample.
//! Renders stay bit-exact on every platform: this is a compare and a select, no flags are involved.

/// Magnitudes below this become zero (-300 dB re full scale).
pub const FLUSH_THRESHOLD: f32 = 1e-15;

/// Flush a tiny `f32` to `+0.0`.
#[inline(always)]
#[must_use]
pub fn flush(x: f32) -> f32 {
    if x.abs() < FLUSH_THRESHOLD { 0.0 } else { x }
}

/// Flush a tiny `f64` to `+0.0` (same threshold as [`flush`]).
#[inline(always)]
#[must_use]
pub fn flush64(x: f64) -> f64 {
    if x.abs() < f64::from(FLUSH_THRESHOLD) {
        0.0
    } else {
        x
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiny_values_become_positive_zero() {
        for x in [
            0.0,
            -0.0,
            1e-16,
            -1e-16,
            1e-30,
            f32::MIN_POSITIVE,
            f32::MIN_POSITIVE / 2.0, // subnormal
            -f32::MIN_POSITIVE / 8.0,
            f32::from_bits(1), // the smallest subnormal
            9.9e-16,
        ] {
            assert_eq!(flush(x).to_bits(), 0.0f32.to_bits(), "{x:e}");
            assert_eq!(flush64(f64::from(x)).to_bits(), 0.0f64.to_bits(), "{x:e}");
        }
        assert_eq!(flush64(1e-300).to_bits(), 0);
        assert_eq!(flush64(f64::from_bits(1)).to_bits(), 0);
    }

    #[test]
    fn audible_values_pass_through_unchanged() {
        for x in [
            1.1e-15,
            -1.1e-15,
            1e-10,
            0.5,
            -0.5,
            1.0,
            1e10,
            f32::MAX,
            f32::MIN,
        ] {
            assert_eq!(flush(x).to_bits(), x.to_bits());
            assert_eq!(flush64(f64::from(x)).to_bits(), f64::from(x).to_bits());
        }
    }

    #[test]
    fn non_finite_values_are_not_touched() {
        assert!(flush(f32::NAN).is_nan());
        assert_eq!(flush(f32::INFINITY), f32::INFINITY);
        assert_eq!(flush(f32::NEG_INFINITY), f32::NEG_INFINITY);
        assert!(flush64(f64::NAN).is_nan());
        assert_eq!(flush64(f64::INFINITY), f64::INFINITY);
    }
}
