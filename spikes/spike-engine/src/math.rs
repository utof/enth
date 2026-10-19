//! The only door to transcendental functions, plus the flush and saturation helpers.
//!
//! Bit-exactness rules (docs/spikes/engine.md):
//! - `sin`, `exp`, `exp2`, `tan` come from the pinned `libm` crate: the same Rust code on every target.
//!   `std` maths differs between glibc, musl, Windows and wasm, and LLVM rewrites `powf(2, x)` into `exp2`.
//! - `sqrt` is correctly rounded by IEEE 754 on every target, so `f32::sqrt` would be fine; nothing uses it yet.
//! - No `mul_add` and no `algebraic_*` anywhere (`clippy.toml` bans them): Rust never contracts `a * b + c`.
//! - Denormals are flushed in code ([`flush`]), never by setting MXCSR/FPCR.
//!
//! The `std-math` feature swaps in `std` maths as a negative control for the cross-platform tests.

/// Below this magnitude recursive state is zeroed: far above the subnormal range (1.2e-38), far below audibility.
pub const FLUSH_THRESHOLD: f32 = 1e-15;

/// A voice whose amplitude envelope falls below this (-100 dB) after its attack is ended.
pub const VOICE_END: f32 = 1e-5;

#[cfg(not(feature = "std-math"))]
mod backend {
    #[inline(always)]
    pub fn sin(x: f32) -> f32 {
        libm::sinf(x)
    }
    #[inline(always)]
    pub fn exp(x: f32) -> f32 {
        libm::expf(x)
    }
    #[inline(always)]
    pub fn exp2(x: f32) -> f32 {
        libm::exp2f(x)
    }
    #[inline(always)]
    pub fn tan(x: f32) -> f32 {
        libm::tanf(x)
    }
}

#[cfg(feature = "std-math")]
#[allow(
    clippy::disallowed_methods,
    reason = "negative control: this module is the one place std maths is allowed, and only behind std-math"
)]
mod backend {
    #[inline(always)]
    pub fn sin(x: f32) -> f32 {
        x.sin()
    }
    #[inline(always)]
    pub fn exp(x: f32) -> f32 {
        x.exp()
    }
    #[inline(always)]
    pub fn exp2(x: f32) -> f32 {
        x.exp2()
    }
    #[inline(always)]
    pub fn tan(x: f32) -> f32 {
        x.tan()
    }
}

/// `sin(x)`, x in radians.
#[inline(always)]
pub fn sin(x: f32) -> f32 {
    backend::sin(x)
}

/// `e^x`.
#[inline(always)]
pub fn exp(x: f32) -> f32 {
    backend::exp(x)
}

/// `2^x`.
#[inline(always)]
pub fn exp2(x: f32) -> f32 {
    backend::exp2(x)
}

/// `tan(x)`, x in radians.
#[inline(always)]
pub fn tan(x: f32) -> f32 {
    backend::tan(x)
}

/// Zero tiny values so recursive state never reaches the subnormal range.
#[inline(always)]
pub fn flush(x: f32) -> f32 {
    if x.abs() < FLUSH_THRESHOLD { 0.0 } else { x }
}

/// Rational tanh approximation from `+ - * /` only (bit-exact everywhere; clamped to +-3).
#[inline(always)]
#[allow(
    clippy::manual_clamp,
    reason = "mirrors the lane path, where `wide` offers only max/min; identical for every non-NaN input"
)]
pub fn tanh_approx(x: f32) -> f32 {
    let x = x.max(-3.0).min(3.0);
    let x2 = x * x;
    x * (27.0 + x2) / (27.0 + 9.0 * x2)
}
