//! Real-input FFTs with a choice of code path.
//!
//! `rustfft` picks its kernels by CPU (AVX, SSE, NEON, wasm SIMD), and the kernels round differently, so the same
//! input can give spectra that differ in the last bits between machines. Scores that decide pass/fail must not
//! depend on that (`docs/design.md` §5), so every analysis in this crate runs on [`FftPath::Scalar`] by default:
//! `rustfft`'s scalar planner for the complex transform, plus our own even-length real-to-complex step whose
//! twiddles come from the pinned `libm`. [`FftPath::Dispatched`] uses `realfft`'s planner (the fast, CPU-dispatched
//! path) for callers that accept a tolerance, such as a search's inner loop. `docs/dsp/analysis.md` records the cost
//! of each.
//!
//! Both paths produce the same layout as `realfft`: `n / 2 + 1` bins, unnormalised; the inverse returns `n · x`.
//! Lengths must be even.

use realfft::num_complex::Complex32;
use realfft::{ComplexToReal, RealFftPlanner, RealToComplex};
use rustfft::{Fft, FftPlannerScalar};
use std::collections::BTreeMap;
use std::f64::consts::PI;
use std::sync::Arc;

/// Which FFT kernels to use.
#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    serde::Serialize,
    serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum FftPath {
    /// `rustfft`'s scalar planner and `libm` twiddles: the same bits on every CPU and on wasm32. The default.
    #[default]
    Scalar,
    /// `realfft`'s planner: SIMD chosen at run time. Faster; results agree with `Scalar` only within a tolerance.
    Dispatched,
}

enum Kind {
    Scalar {
        /// Complex FFT of length `n / 2` (forward or inverse).
        half: Arc<dyn Fft<f32>>,
        /// `exp(-2πi k / n)` for `k` in `0..n / 2`, from `libm` in f64, rounded to f32.
        twiddles: Vec<Complex32>,
    },
    Forward(Arc<dyn RealToComplex<f32>>),
    Inverse(Arc<dyn ComplexToReal<f32>>),
}

/// One planned real FFT (forward or inverse) of a fixed even length.
pub struct RealFft {
    n: usize,
    forward: bool,
    kind: Kind,
}

impl std::fmt::Debug for RealFft {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RealFft")
            .field("n", &self.n)
            .field("forward", &self.forward)
            .field("path", &self.path())
            .finish_non_exhaustive()
    }
}

impl RealFft {
    /// Transform length (real samples).
    #[must_use]
    pub fn len(&self) -> usize {
        self.n
    }

    /// Always false: lengths are at least 2.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        false
    }

    /// Number of complex bins, `n / 2 + 1`.
    #[must_use]
    pub fn bins(&self) -> usize {
        self.n / 2 + 1
    }

    #[must_use]
    pub fn path(&self) -> FftPath {
        match self.kind {
            Kind::Scalar { .. } => FftPath::Scalar,
            Kind::Forward(_) | Kind::Inverse(_) => FftPath::Dispatched,
        }
    }

    /// A zeroed scratch buffer large enough for [`RealFft::forward`] or [`RealFft::inverse`].
    #[must_use]
    pub fn make_scratch(&self) -> Vec<Complex32> {
        let len = match &self.kind {
            Kind::Scalar { half, .. } => self.n / 2 + half.get_inplace_scratch_len(),
            Kind::Forward(f) => f.get_scratch_len(),
            Kind::Inverse(f) => f.get_scratch_len(),
        };
        vec![Complex32::new(0.0, 0.0); len.max(1)]
    }

    /// Forward transform. `input` (length `n`) may be overwritten; `output` has `n / 2 + 1` bins.
    #[allow(
        clippy::manual_midpoint,
        reason = "0.5·(a + b) is the textbook split; f32::midpoint's code differs by target"
    )]
    ///
    /// # Panics
    /// If this plan is an inverse, or a buffer has the wrong length.
    pub fn forward(&self, input: &mut [f32], output: &mut [Complex32], scratch: &mut [Complex32]) {
        assert!(self.forward, "forward() on an inverse plan");
        assert_eq!(input.len(), self.n, "input length");
        assert_eq!(output.len(), self.bins(), "output length");
        match &self.kind {
            Kind::Forward(f) => f
                .process_with_scratch(input, output, scratch)
                .expect("buffer sizes were checked"),
            Kind::Scalar { half, twiddles } => {
                let h = self.n / 2;
                let (z, fft_scratch) = scratch.split_at_mut(h);
                for (m, zm) in z.iter_mut().enumerate() {
                    *zm = Complex32::new(input[2 * m], input[2 * m + 1]);
                }
                half.process_with_scratch(z, fft_scratch);
                // X[k] = E[k] + W^k O[k] with E[k] = (Z[k] + conj Z[h-k]) / 2, O[k] = (Z[k] - conj Z[h-k]) / 2i.
                output[0] = Complex32::new(z[0].re + z[0].im, 0.0);
                output[h] = Complex32::new(z[0].re - z[0].im, 0.0);
                for k in 1..h {
                    let a = z[k];
                    let b = z[h - k].conj();
                    let e = Complex32::new(0.5 * (a.re + b.re), 0.5 * (a.im + b.im));
                    // (a - b) / 2i = (im, -re) / 2
                    let o = Complex32::new(0.5 * (a.im - b.im), -0.5 * (a.re - b.re));
                    let w = twiddles[k];
                    output[k] = Complex32::new(
                        e.re + (w.re * o.re - w.im * o.im),
                        e.im + (w.re * o.im + w.im * o.re),
                    );
                }
            }
            Kind::Inverse(_) => unreachable!("checked above"),
        }
    }

    /// Inverse transform, unnormalised: returns `n · x`. `input` has `n / 2 + 1` bins and may be overwritten; the
    /// imaginary parts of the first and last bin must be zero.
    ///
    /// # Panics
    /// If this plan is a forward one, or a buffer has the wrong length.
    pub fn inverse(&self, input: &mut [Complex32], output: &mut [f32], scratch: &mut [Complex32]) {
        assert!(!self.forward, "inverse() on a forward plan");
        assert_eq!(input.len(), self.bins(), "input length");
        assert_eq!(output.len(), self.n, "output length");
        match &self.kind {
            Kind::Inverse(f) => f
                .process_with_scratch(input, output, scratch)
                .expect("buffer sizes were checked and the end bins are real"),
            Kind::Scalar { half, twiddles } => {
                let h = self.n / 2;
                let (z, fft_scratch) = scratch.split_at_mut(h);
                // E[k] = (X[k] + conj X[h-k]) / 2, O[k] = (X[k] - conj X[h-k]) conj(W^k) / 2, Z = E + iO.
                // Scaled by 2 so the half-length inverse returns n · x, like realfft.
                for (k, zk) in z.iter_mut().enumerate() {
                    let a = input[k];
                    let b = input[h - k].conj();
                    let e = Complex32::new(a.re + b.re, a.im + b.im);
                    let d = Complex32::new(a.re - b.re, a.im - b.im);
                    let w = twiddles[k];
                    // d * conj(w)
                    let o = Complex32::new(d.re * w.re + d.im * w.im, d.im * w.re - d.re * w.im);
                    // e + i o
                    *zk = Complex32::new(e.re - o.im, e.im + o.re);
                }
                half.process_with_scratch(z, fft_scratch);
                for (m, zm) in z.iter().enumerate() {
                    output[2 * m] = zm.re;
                    output[2 * m + 1] = zm.im;
                }
            }
            Kind::Forward(_) => unreachable!("checked above"),
        }
    }
}

/// Plans real FFTs on one path and caches them by length.
pub struct FftPlanner {
    path: FftPath,
    scalar: FftPlannerScalar<f32>,
    dispatched: RealFftPlanner<f32>,
    cache: BTreeMap<(usize, bool), Arc<RealFft>>,
}

impl std::fmt::Debug for FftPlanner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FftPlanner")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

impl FftPlanner {
    #[must_use]
    pub fn new(path: FftPath) -> Self {
        Self {
            path,
            scalar: FftPlannerScalar::new(),
            dispatched: RealFftPlanner::new(),
            cache: BTreeMap::new(),
        }
    }

    #[must_use]
    pub fn path(&self) -> FftPath {
        self.path
    }

    /// A forward real FFT of even length `n >= 2`.
    ///
    /// # Panics
    /// If `n` is odd or below 2.
    pub fn forward(&mut self, n: usize) -> Arc<RealFft> {
        self.plan(n, true)
    }

    /// An inverse real FFT of even length `n >= 2`.
    ///
    /// # Panics
    /// If `n` is odd or below 2.
    pub fn inverse(&mut self, n: usize) -> Arc<RealFft> {
        self.plan(n, false)
    }

    fn plan(&mut self, n: usize, forward: bool) -> Arc<RealFft> {
        assert!(
            n >= 2 && n.is_multiple_of(2),
            "real FFT lengths must be even and at least 2, got {n}"
        );
        if let Some(p) = self.cache.get(&(n, forward)) {
            return Arc::clone(p);
        }
        let kind = match self.path {
            FftPath::Scalar => {
                let h = n / 2;
                let half = if forward {
                    self.scalar.plan_fft_forward(h)
                } else {
                    self.scalar.plan_fft_inverse(h)
                };
                let twiddles = (0..h)
                    .map(|k| {
                        let a = -2.0 * PI * k as f64 / n as f64;
                        Complex32::new(libm::cos(a) as f32, libm::sin(a) as f32)
                    })
                    .collect();
                Kind::Scalar { half, twiddles }
            }
            FftPath::Dispatched if forward => Kind::Forward(self.dispatched.plan_fft_forward(n)),
            FftPath::Dispatched => Kind::Inverse(self.dispatched.plan_fft_inverse(n)),
        };
        let plan = Arc::new(RealFft { n, forward, kind });
        self.cache.insert((n, forward), Arc::clone(&plan));
        plan
    }
}

/// Periodic Hann window of length `n` (f32, from `libm`).
#[must_use]
pub fn hann(n: usize) -> Vec<f32> {
    (0..n)
        .map(|i| (0.5 - 0.5 * libm::cos(2.0 * PI * i as f64 / n as f64)) as f32)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signal(n: usize) -> Vec<f32> {
        let mut s: u32 = 12345;
        (0..n)
            .map(|_| {
                s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (s >> 8) as f32 / (1u32 << 24) as f32 - 0.5
            })
            .collect()
    }

    fn dft(x: &[f32]) -> Vec<(f64, f64)> {
        let n = x.len();
        (0..=n / 2)
            .map(|k| {
                let (mut re, mut im) = (0.0, 0.0);
                for (j, &v) in x.iter().enumerate() {
                    let a = -2.0 * PI * (k * j % n) as f64 / n as f64;
                    re += f64::from(v) * libm::cos(a);
                    im += f64::from(v) * libm::sin(a);
                }
                (re, im)
            })
            .collect()
    }

    #[test]
    fn both_paths_match_a_direct_dft() {
        for n in [2usize, 4, 6, 8, 30, 64, 100, 256] {
            let x = signal(n);
            let want = dft(&x);
            for path in [FftPath::Scalar, FftPath::Dispatched] {
                let mut p = FftPlanner::new(path);
                let f = p.forward(n);
                let mut input = x.clone();
                let mut out = vec![Complex32::new(0.0, 0.0); f.bins()];
                let mut scratch = f.make_scratch();
                f.forward(&mut input, &mut out, &mut scratch);
                for (k, (o, w)) in out.iter().zip(&want).enumerate() {
                    let err = (f64::from(o.re) - w.0).abs() + (f64::from(o.im) - w.1).abs();
                    assert!(
                        err < 1e-4 * n as f64,
                        "{path:?} n={n} bin {k}: {o} vs {w:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn inverse_undoes_forward_times_n() {
        for path in [FftPath::Scalar, FftPath::Dispatched] {
            let mut p = FftPlanner::new(path);
            for n in [8usize, 30, 512] {
                let x = signal(n);
                let (f, i) = (p.forward(n), p.inverse(n));
                let mut input = x.clone();
                let mut spec = vec![Complex32::new(0.0, 0.0); f.bins()];
                let mut scratch = f.make_scratch();
                f.forward(&mut input, &mut spec, &mut scratch);
                let mut back = vec![0.0f32; n];
                let mut scratch = i.make_scratch();
                i.inverse(&mut spec, &mut back, &mut scratch);
                for (a, b) in x.iter().zip(&back) {
                    assert!(
                        (a * n as f32 - b).abs() < 1e-3,
                        "{path:?} n={n}: {a} vs {}",
                        b / n as f32
                    );
                }
            }
        }
    }

    #[test]
    fn plans_are_cached() {
        let mut p = FftPlanner::new(FftPath::Scalar);
        let a = p.forward(64);
        let b = p.forward(64);
        assert!(Arc::ptr_eq(&a, &b));
        assert_eq!(a.path(), FftPath::Scalar);
        assert_eq!(
            FftPlanner::new(FftPath::Dispatched).forward(64).path(),
            FftPath::Dispatched
        );
    }
}
