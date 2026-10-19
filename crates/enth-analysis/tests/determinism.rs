//! The scalar FFT path gives the same bits everywhere (`docs/design.md` §5): pinned hashes of raw scalar FFTs, of
//! the spectral features and of the descriptors. Part of the gate natively; the same test binary runs under
//! wasmtime for the cross-platform check (`docs/dsp/analysis.md`, "Determinism"):
//!
//! ```text
//! CARGO_TARGET_WASM32_WASIP1_RUNNER=/path/to/wasmtime \
//!   cargo test -p enth-analysis --target wasm32-wasip1 --test determinism -- --nocapture
//! ```
//!
//! Loudness (LUFS) and true peak come from `ebur128`, which uses `std` maths: they are compared within a tolerance
//! and printed, not pinned. A pinned hash changes only when an analysis algorithm changes on purpose; update it in
//! the same commit and say why.

#![allow(
    clippy::print_stdout,
    reason = "prints the hashes for the native-vs-wasm comparison"
)]

use enth_analysis::descriptors::{Describer, Descriptors, NoteTiming};
use enth_analysis::features::{FeatureCfg, FeatureExtractor};
use enth_analysis::fft::{FftPath, FftPlanner};
use enth_analysis::test_signals::{SR, ToyPatch, ToyPerf, render_toy};
use realfft::num_complex::Complex32;

struct Fnv(u64);

impl Fnv {
    fn new() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }

    fn bytes(&mut self, b: &[u8]) {
        for x in b {
            self.0 ^= u64::from(*x);
            self.0 = self.0.wrapping_mul(0x0100_0000_01b3);
        }
    }

    fn f32(&mut self, v: f32) {
        self.bytes(&v.to_bits().to_le_bytes());
    }

    fn f64(&mut self, v: f64) {
        self.bytes(&v.to_bits().to_le_bytes());
    }

    fn opt(&mut self, v: Option<f64>) {
        match v {
            Some(x) => self.f64(x),
            None => self.bytes(b"none"),
        }
    }
}

fn signal(n: usize) -> Vec<f32> {
    let mut s: u32 = 12345;
    (0..n)
        .map(|_| {
            s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (s >> 8) as f32 / (1u32 << 24) as f32 - 0.5
        })
        .collect()
}

fn fft_hash(path: FftPath) -> u64 {
    let mut p = FftPlanner::new(path);
    let mut h = Fnv::new();
    for n in [512usize, 1024, 2048, 8192, 1000] {
        let (f, inv) = (p.forward(n), p.inverse(n));
        let mut x = signal(n);
        let mut spec = vec![Complex32::new(0.0, 0.0); f.bins()];
        let mut scratch = f.make_scratch();
        f.forward(&mut x, &mut spec, &mut scratch);
        for c in &spec {
            h.f32(c.re);
            h.f32(c.im);
        }
        let mut back = vec![0.0f32; n];
        let mut scratch = inv.make_scratch();
        inv.inverse(&mut spec, &mut back, &mut scratch);
        for v in &back {
            h.f32(*v);
        }
    }
    h.0
}

fn toy() -> (Vec<f32>, f64) {
    let p = ToyPatch {
        detune_ct: 24.0,
        cutoff_hz: 3000.0,
        attack_ms: 8.0,
        decay_ms: 600.0,
        sustain_db: -9.0,
        release_ms: 300.0,
        ..ToyPatch::default()
    };
    let perf = ToyPerf {
        note_hz: 440.0,
        dur_s: 1.5,
        hold_s: 1.1,
    };
    (render_toy(SR, &p, &perf, (0.0, 0.37), 0.0), perf.hold_s)
}

fn features_hash() -> u64 {
    let (x, _) = toy();
    let mut fx = FeatureExtractor::new(FeatureCfg {
        with_f0: true,
        ..FeatureCfg::default()
    });
    let f = fx.extract_unnormalised(&x);
    let mut h = Fnv::new();
    for v in f.logmel.iter().flatten() {
        h.f32(*v);
    }
    for v in f
        .env_db
        .iter()
        .chain(&f.crest_db)
        .chain(&f.centroid)
        .chain(&f.centroid_w)
        .chain(&f.mfcc)
        .chain(&f.f0_hz)
    {
        h.f32(*v);
    }
    h.0
}

fn descriptors() -> Descriptors {
    let (x, hold) = toy();
    Describer::new(SR)
        .describe(&x, NoteTiming::held(hold))
        .unwrap()
}

/// Every descriptor except the two from `ebur128` (LUFS, true peak).
fn descriptors_hash(d: &Descriptors) -> u64 {
    let mut h = Fnv::new();
    let p = d.pitch.as_ref().unwrap();
    for v in [p.f0_hz, p.cents_dev, p.stability, p.voiced] {
        h.f64(v);
    }
    let e = d.envelope.unwrap();
    for v in [e.attack_ms, e.duration_ms, e.ripple_db] {
        h.f64(v);
    }
    for v in [e.decay_ms, e.sustain_db, e.release_ms, e.beat_periods] {
        h.opt(v);
    }
    let s = d.spectrum.unwrap();
    h.f64(s.centroid_hz);
    s.centroid_track_hz.unwrap().iter().for_each(|v| h.f64(*v));
    h.opt(d.noise_ratio);
    let b = d.detune.unwrap();
    for v in [b.beating_hz, b.depth_db, b.prominence] {
        h.f64(v);
    }
    h.f64(d.loudness.peak_dbfs);
    h.f64(d.loudness.clipped_pct);
    h.0
}

/// Pinned on `x86_64` Linux (an AVX2 machine) and checked equal under wasmtime 48 (`wasm32-wasip1`) on 2026-10-19.
const FFT_SCALAR: u64 = 0xb7c8_97c5_1e15_be08;
const FEATURES_SCALAR: u64 = 0x7c65_ee25_fda7_f8ea;
const DESCRIPTORS_SCALAR: u64 = 0x270e_0957_e2c3_42e9;

#[test]
fn scalar_fft_is_pinned() {
    let h = fft_hash(FftPath::Scalar);
    println!(
        "scalar fft hash {h:#018x}; dispatched (information) {:#018x}",
        fft_hash(FftPath::Dispatched)
    );
    assert_eq!(h, FFT_SCALAR, "scalar FFT bits changed: {h:#018x}");
}

#[test]
fn scalar_features_are_pinned() {
    let h = features_hash();
    println!("scalar features hash {h:#018x}");
    assert_eq!(h, FEATURES_SCALAR, "feature bits changed: {h:#018x}");
}

#[test]
fn scalar_descriptors_are_pinned() {
    let d = descriptors();
    let h = descriptors_hash(&d);
    println!(
        "descriptors hash {h:#018x}; ebur128 (tolerance only): lufs {:?} true peak {} dBTP",
        d.loudness.lufs_i, d.loudness.true_peak_dbtp
    );
    assert_eq!(
        h, DESCRIPTORS_SCALAR,
        "descriptor bits changed: {h:#018x}\n{d:#?}"
    );
    // ebur128's numbers are compared within a tolerance against the native reading
    let lufs = d.loudness.lufs_i.unwrap();
    assert!((lufs - LUFS_NATIVE).abs() < 1e-6, "{lufs}");
    assert!(
        (d.loudness.true_peak_dbtp - TRUE_PEAK_NATIVE).abs() < 1e-6,
        "{}",
        d.loudness.true_peak_dbtp
    );
}

const LUFS_NATIVE: f64 = -16.492_280_478_318_243;
const TRUE_PEAK_NATIVE: f64 = -2.238_688_230_184_584;
