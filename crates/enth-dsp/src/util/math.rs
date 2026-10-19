//! The only door to transcendentals: thin wrappers over the pinned `libm` crate, plus approximations and helpers
//! built from them or from `+ − × ÷ sqrt` alone.
//!
//! **Why a door.** `std`'s `sin`, `exp`, `powf`… call the platform's maths library, which differs between glibc,
//! musl, Windows and wasm (measured in Phase 0: `std` and `libm` disagree on up to 14 % of inputs), and LLVM rewrites
//! some of them (`powf(2, x)` into `exp2`). `libm` is plain Rust, pinned with `=` in the workspace, so the same input
//! gives the same bits everywhere (design.md §5, rule 1). The root `clippy.toml` bans `std`'s transcendentals in every
//! core crate; code after freeze F2 calls these wrappers rather than `libm` directly, so a future change of backend
//! (or a bump of `libm`, which is an engine-version change) happens in one file.
//!
//! Names follow `libm`: an `f` suffix is the `f32` version (`sinf`), no suffix is `f64` (`sin`). `sqrt`, `abs`,
//! `floor`, `round` and friends are exact in IEEE 754 and stay on the primitive types.
//!
//! The other DSP rules hold here too: no `mul_add` (Rust never contracts `a * b + c` on its own), a fixed operation
//! order, and nothing reads CPU flags.

// ---------------------------------------------------------------------------------------------------------------
// f32
// ---------------------------------------------------------------------------------------------------------------

/// `sin(x)`, `x` in radians.
#[inline(always)]
#[must_use]
pub fn sinf(x: f32) -> f32 {
    libm::sinf(x)
}

/// `cos(x)`, `x` in radians.
#[inline(always)]
#[must_use]
pub fn cosf(x: f32) -> f32 {
    libm::cosf(x)
}

/// `tan(x)`, `x` in radians.
#[inline(always)]
#[must_use]
pub fn tanf(x: f32) -> f32 {
    libm::tanf(x)
}

/// `atan(x)`.
#[inline(always)]
#[must_use]
pub fn atanf(x: f32) -> f32 {
    libm::atanf(x)
}

/// `e^x`.
#[inline(always)]
#[must_use]
pub fn expf(x: f32) -> f32 {
    libm::expf(x)
}

/// `e^x − 1`, accurate for small `x`.
#[inline(always)]
#[must_use]
pub fn expm1f(x: f32) -> f32 {
    libm::expm1f(x)
}

/// `2^x`.
#[inline(always)]
#[must_use]
pub fn exp2f(x: f32) -> f32 {
    libm::exp2f(x)
}

/// Natural logarithm.
#[inline(always)]
#[must_use]
pub fn logf(x: f32) -> f32 {
    libm::logf(x)
}

/// Base-2 logarithm.
#[inline(always)]
#[must_use]
pub fn log2f(x: f32) -> f32 {
    libm::log2f(x)
}

/// Base-10 logarithm.
#[inline(always)]
#[must_use]
pub fn log10f(x: f32) -> f32 {
    libm::log10f(x)
}

/// `x^y`.
#[inline(always)]
#[must_use]
pub fn powf(x: f32, y: f32) -> f32 {
    libm::powf(x, y)
}

/// `tanh(x)` (exact to `libm`'s accuracy; see [`tanh_approx`] for the cheap rational form).
#[inline(always)]
#[must_use]
pub fn tanhf(x: f32) -> f32 {
    libm::tanhf(x)
}

/// `sinh(x)`.
#[inline(always)]
#[must_use]
pub fn sinhf(x: f32) -> f32 {
    libm::sinhf(x)
}

// ---------------------------------------------------------------------------------------------------------------
// f64
// ---------------------------------------------------------------------------------------------------------------

/// `sin(x)`, `x` in radians.
#[inline(always)]
#[must_use]
pub fn sin(x: f64) -> f64 {
    libm::sin(x)
}

/// `cos(x)`, `x` in radians.
#[inline(always)]
#[must_use]
pub fn cos(x: f64) -> f64 {
    libm::cos(x)
}

/// `tan(x)`, `x` in radians.
#[inline(always)]
#[must_use]
pub fn tan(x: f64) -> f64 {
    libm::tan(x)
}

/// `e^x`.
#[inline(always)]
#[must_use]
pub fn exp(x: f64) -> f64 {
    libm::exp(x)
}

/// `e^x − 1`, accurate for small `x`.
#[inline(always)]
#[must_use]
pub fn expm1(x: f64) -> f64 {
    libm::expm1(x)
}

/// `2^x`.
#[inline(always)]
#[must_use]
pub fn exp2(x: f64) -> f64 {
    libm::exp2(x)
}

/// Natural logarithm.
#[inline(always)]
#[must_use]
pub fn log(x: f64) -> f64 {
    libm::log(x)
}

/// Base-2 logarithm.
#[inline(always)]
#[must_use]
pub fn log2(x: f64) -> f64 {
    libm::log2(x)
}

/// Base-10 logarithm.
#[inline(always)]
#[must_use]
pub fn log10(x: f64) -> f64 {
    libm::log10(x)
}

/// `x^y`.
#[inline(always)]
#[must_use]
pub fn pow(x: f64, y: f64) -> f64 {
    libm::pow(x, y)
}

/// `tanh(x)`.
#[inline(always)]
#[must_use]
pub fn tanh(x: f64) -> f64 {
    libm::tanh(x)
}

// ---------------------------------------------------------------------------------------------------------------
// Approximations and helpers
// ---------------------------------------------------------------------------------------------------------------

/// `log2(10) / 20`: decibels to octaves of amplitude, so `2^(dB · DB_TO_LOG2) = 10^(dB / 20)`.
pub const DB_TO_LOG2: f64 = 0.166_096_404_744_368_1;

/// A voice whose amplitude envelope, past its attack, is below this (−100 dB) is ended (design.md §5, rule 4).
pub const VOICE_END_LEVEL: f32 = 1e-5;

/// A rational `tanh` from `+ − × ÷` only, clamped to `±3` where it reaches `±1` exactly (the Phase 0 spike's; the
/// \[3/2\] Padé form `x(27 + x²) / (27 + 9x²)`, whose derivative `9(x² − 9)² / (27 + 9x²)²` is never negative).
/// Bit-exact everywhere and much cheaper than [`tanhf`]; odd, within 0.025 of `tanh`, and within one `f32` step of
/// `±1` (rounding puts it at `1.0000001` just below 3).
#[inline(always)]
#[must_use]
pub fn tanh_approx(x: f32) -> f32 {
    let x = x.clamp(-3.0, 3.0);
    let x2 = x * x;
    x * (27.0 + x2) / (27.0 + 9.0 * x2)
}

/// Decibels to a linear gain, `10^(dB / 20)`, computed in `f64` and rounded once (`0 dB` gives exactly `1.0`).
/// For per-segment use; per-sample code should keep its own `f32` form.
#[inline]
#[must_use]
pub fn db_to_gain(db: f32) -> f32 {
    exp2(f64::from(db) * DB_TO_LOG2) as f32
}

/// A linear gain to decibels, `20·log10(|g|)`, with a floor of −300 dB for zero.
#[inline]
#[must_use]
pub fn gain_to_db(gain: f32) -> f32 {
    let g = f64::from(gain.abs()).max(1e-15);
    (20.0 * log10(g)) as f32
}

/// The frequency ratio of an interval in semitones, `2^(st / 12)`.
#[inline]
#[must_use]
pub fn semitones_to_ratio(st: f32) -> f32 {
    exp2f(st / 12.0)
}

/// The frequency in Hz of a (fractional) MIDI note number, A4 = 69 = 440 Hz: `440 · 2^((note − 69) / 12)`.
#[inline]
#[must_use]
pub fn midi_to_hz(note: f32) -> f32 {
    440.0 * exp2f((note - 69.0) / 12.0)
}

/// Constant-power pan gains `(left, right)` for `pan` in `[-1, 1]` (−1 is hard left): `θ = (pan + 1)·π/4`,
/// `(cos θ, sin θ)`, computed in `f64` and rounded once. The centre is −3 dB on both channels and exactly equal
/// (`0.70710677`); the ends are exactly `(1, 0)` and `(0, 1)`; `left² + right² = 1` within rounding everywhere.
/// Out-of-range input is clamped.
#[must_use]
pub fn constant_power_pan(pan: f32) -> (f32, f32) {
    if pan <= -1.0 {
        return (1.0, 0.0);
    }
    if pan >= 1.0 {
        return (0.0, 1.0);
    }
    let theta = (f64::from(pan) + 1.0) * core::f64::consts::FRAC_PI_4;
    (cos(theta) as f32, sin(theta) as f32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrappers_are_libm() {
        // Spot checks that the door is wired to the right functions (bit-for-bit with libm by construction).
        assert_eq!(sinf(0.5).to_bits(), libm::sinf(0.5).to_bits());
        assert_eq!(exp2f(-1.5).to_bits(), libm::exp2f(-1.5).to_bits());
        assert_eq!(pow(10.0, 0.3).to_bits(), libm::pow(10.0, 0.3).to_bits());
        assert_eq!(log2(8.0), 3.0);
        assert_eq!(exp2(0.0), 1.0);
    }

    #[test]
    fn tanh_approx_is_odd_bounded_and_close() {
        for i in -400..=400 {
            let x = i as f32 / 100.0;
            let y = tanh_approx(x);
            assert_eq!(y.to_bits(), (-tanh_approx(-x)).to_bits(), "odd at {x}");
            assert!(y.abs() <= 1.000_000_2, "{x}: {y}");
            assert!((y - tanhf(x)).abs() < 0.025, "{x}: {y} vs {}", tanhf(x));
        }
        assert_eq!(tanh_approx(3.0), 1.0);
        assert_eq!(tanh_approx(100.0), 1.0);
        assert_eq!(tanh_approx(0.0), 0.0);
    }

    #[test]
    fn decibels() {
        assert_eq!(db_to_gain(0.0), 1.0);
        assert!((db_to_gain(-6.020_6) - 0.5).abs() < 1e-6);
        assert!((db_to_gain(20.0) - 10.0).abs() < 1e-5);
        assert!((gain_to_db(0.5) - -6.020_6).abs() < 1e-3);
        assert_eq!(gain_to_db(0.0), -300.0);
    }

    #[test]
    fn pitch_helpers() {
        assert_eq!(midi_to_hz(69.0), 440.0);
        assert!((midi_to_hz(81.0) - 880.0).abs() < 1e-3);
        assert!((midi_to_hz(60.0) - 261.625_58).abs() < 1e-3);
        assert_eq!(semitones_to_ratio(0.0), 1.0);
        assert_eq!(semitones_to_ratio(12.0), 2.0);
    }

    #[test]
    fn pan_law_is_constant_power_with_minus_3_db_at_centre() {
        let (l, r) = constant_power_pan(0.0);
        assert_eq!(l.to_bits(), r.to_bits(), "centre is symmetric");
        assert!((gain_to_db(l) - -3.0103).abs() < 1e-3);
        assert_eq!(constant_power_pan(-1.0), (1.0, 0.0));
        assert_eq!(constant_power_pan(1.0), (0.0, 1.0));
        assert_eq!(constant_power_pan(-7.0), (1.0, 0.0));
        for i in -100..=100 {
            let (l, r) = constant_power_pan(i as f32 / 100.0);
            let power = f64::from(l) * f64::from(l) + f64::from(r) * f64::from(r);
            assert!((power - 1.0).abs() < 1e-6, "{i}: {power}");
            assert!(l >= 0.0 && r >= 0.0);
        }
        // Mirror image: pan p left gain equals pan -p right gain.
        let (l, _) = constant_power_pan(0.3);
        let (_, r) = constant_power_pan(-0.3);
        assert!((l - r).abs() < 1e-7);
    }
}
