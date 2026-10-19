//! YIN fundamental-frequency tracker (de Cheveigne and Kawahara, 2002), with the difference function computed by FFT.
//! Pure: no I/O, no clocks.

use realfft::num_complex::Complex32;
use realfft::{ComplexToReal, RealFftPlanner, RealToComplex};
use std::sync::Arc;

pub struct Yin {
    sample_rate: f32,
    frame: usize,
    window: usize,
    tau_min: usize,
    tau_max: usize,
    threshold: f32,
    fwd: Arc<dyn RealToComplex<f32>>,
    inv: Arc<dyn ComplexToReal<f32>>,
    buf_a: Vec<f32>,
    buf_b: Vec<f32>,
    spec_a: Vec<Complex32>,
    spec_b: Vec<Complex32>,
    scratch: Vec<Complex32>,
    scratch_inv: Vec<Complex32>,
    cmnd: Vec<f32>,
}

impl Yin {
    /// `frame` must be a power of two; the integration window is `frame - tau_max`.
    #[must_use]
    pub fn new(sample_rate: f32, frame: usize, f_min: f32, f_max: f32) -> Self {
        let tau_max = ((sample_rate / f_min) as usize).min(frame / 2);
        let tau_min = ((sample_rate / f_max) as usize).max(2);
        let mut planner = RealFftPlanner::<f32>::new();
        let fwd = planner.plan_fft_forward(frame);
        let inv = planner.plan_fft_inverse(frame);
        Self {
            sample_rate,
            frame,
            window: frame - tau_max,
            tau_min,
            tau_max,
            threshold: 0.15,
            buf_a: fwd.make_input_vec(),
            buf_b: fwd.make_input_vec(),
            spec_a: fwd.make_output_vec(),
            spec_b: fwd.make_output_vec(),
            scratch: fwd.make_scratch_vec(),
            scratch_inv: inv.make_scratch_vec(),
            cmnd: vec![0.0; tau_max + 1],
            fwd,
            inv,
        }
    }

    /// Estimate f0 in Hz for one `frame`-sample block. `None` = unvoiced or silent.
    pub fn estimate(&mut self, x: &[f32]) -> Option<f32> {
        debug_assert_eq!(x.len(), self.frame);
        let (w, tau_max) = (self.window, self.tau_max);
        self.buf_a.fill(0.0);
        self.buf_a[..w].copy_from_slice(&x[..w]);
        self.buf_b.copy_from_slice(x);
        self.fwd
            .process_with_scratch(&mut self.buf_a, &mut self.spec_a, &mut self.scratch)
            .ok()?;
        self.fwd
            .process_with_scratch(&mut self.buf_b, &mut self.spec_b, &mut self.scratch)
            .ok()?;
        for (a, b) in self.spec_a.iter_mut().zip(&self.spec_b) {
            *a = a.conj() * b;
        }
        self.inv
            .process_with_scratch(&mut self.spec_a, &mut self.buf_a, &mut self.scratch_inv)
            .ok()?;
        let norm = 1.0 / self.frame as f32;
        // prefix sums of x^2 for the shifted energy term
        let mut pre = vec![0.0f64; self.frame + 1];
        for (i, v) in x.iter().enumerate() {
            pre[i + 1] = pre[i] + f64::from(*v) * f64::from(*v);
        }
        let e0 = pre[w];
        if e0 < 1e-9 {
            return None;
        }
        // d(tau) = e(0..w) + e(tau..tau+w) - 2 * acf(tau); then cumulative-mean-normalised
        self.cmnd[0] = 1.0;
        let mut running = 0.0f64;
        for tau in 1..=tau_max {
            let acf = f64::from(self.buf_a[tau] * norm);
            let d = (e0 + (pre[tau + w] - pre[tau]) - 2.0 * acf).max(0.0);
            running += d;
            self.cmnd[tau] = if running > 0.0 {
                (d * tau as f64 / running) as f32
            } else {
                1.0
            };
        }
        // first dip under the threshold, walked down to its local minimum
        let mut tau = self.tau_min;
        let mut found = None;
        while tau < tau_max {
            if self.cmnd[tau] < self.threshold {
                while tau + 1 < tau_max && self.cmnd[tau + 1] < self.cmnd[tau] {
                    tau += 1;
                }
                found = Some(tau);
                break;
            }
            tau += 1;
        }
        let tau = found?;
        // parabolic interpolation around the minimum
        let (y0, y1, y2) = (
            self.cmnd[tau - 1],
            self.cmnd[tau],
            self.cmnd[(tau + 1).min(tau_max)],
        );
        let denom = y0 - 2.0 * y1 + y2;
        let shift = if denom.abs() > 1e-12 {
            0.5 * (y0 - y2) / denom
        } else {
            0.0
        };
        Some(self.sample_rate / (tau as f32 + shift.clamp(-1.0, 1.0)))
    }

    /// f0 per hop (0.0 where unvoiced), starting at sample 0.
    pub fn track(&mut self, x: &[f32], hop: usize) -> Vec<f32> {
        let mut out = Vec::new();
        let mut pos = 0;
        while pos + self.frame <= x.len() {
            out.push(self.estimate(&x[pos..pos + self.frame]).unwrap_or(0.0));
            pos += hop;
        }
        out
    }
}
