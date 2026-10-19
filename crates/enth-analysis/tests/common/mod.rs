//! Shared helpers for the integration tests: render the toy voice, describe it, the Phase 0 trial's patches and
//! goals (`docs/spikes/agent-probe.md`), and small statistics for the sweeps.

#![allow(dead_code, reason = "each test binary uses a different subset")]

use enth_analysis::descriptors::{Describer, Descriptors, NoteTiming};
use enth_analysis::envelope::{self, Envelope};
use enth_analysis::test_signals::{SR, ToyPatch, ToyPerf, Wave, render_toy};

/// The research prototype's default start phases (`spike-metric`'s render).
pub const PHASES: (f32, f32) = (0.0, 0.37);

pub fn render(p: &ToyPatch, perf: &ToyPerf) -> Vec<f32> {
    render_toy(SR, p, perf, PHASES, 0.0)
}

pub fn describe(d: &mut Describer, p: &ToyPatch, perf: &ToyPerf) -> Descriptors {
    describe_with(d, p, perf, PHASES, 0.0)
}

pub fn describe_with(
    d: &mut Describer,
    p: &ToyPatch,
    perf: &ToyPerf,
    phases: (f32, f32),
    master_db: f32,
) -> Descriptors {
    let x = render_toy(SR, p, perf, phases, master_db);
    d.describe(&x, NoteTiming::held(perf.hold_s))
        .expect("toy renders are finite")
}

/// The envelope of the same render with beat correction switched off (the raw 8 ms RMS reading).
pub fn raw_envelope(x: &[f32], hold_s: f64) -> Envelope {
    let (raw, hop) = envelope::rms_envelope(x, SR);
    let hop_ms = hop as f64 / SR * 1000.0;
    envelope::measure(
        &raw,
        hop_ms,
        0,
        Some((hold_s * 1000.0 / hop_ms).round() as usize),
        None,
    )
}

/// The two nuisance renders of report 03 §F6: a common phase shift always; a different inter-oscillator phase only
/// when the oscillators are detuned (at 0 ct the phase difference is a timbre change).
pub fn nuisance_phases(detuned: bool) -> [(f32, f32); 2] {
    if detuned {
        [(0.25, 0.81), (0.6, 0.1)]
    } else {
        [(0.25, 0.62), (0.6, 0.97)]
    }
}

/// A trial patch: `(name, patch, perf)`.
pub type Trial = (&'static str, ToyPatch, ToyPerf);

#[allow(clippy::too_many_arguments, reason = "a table row")]
const fn tp(
    detune_ct: f64,
    cutoff_hz: f64,
    resonance_q: f64,
    env_amount_oct: f64,
    env_decay_ms: f64,
    attack_ms: f64,
    decay_ms: f64,
    sustain_db: f64,
    release_ms: f64,
) -> ToyPatch {
    ToyPatch {
        wave: Wave::Saw,
        detune_ct,
        cutoff_hz,
        resonance_q,
        env_amount_oct,
        env_decay_ms,
        attack_ms,
        decay_ms,
        sustain_db,
        release_ms,
    }
}

const A3: f64 = 220.0;
const C3: f64 = 130.812_782_650_299_3;
const C2: f64 = 65.406_391_325_149_66;
const C5: f64 = 523.251_130_601_197_3;
const A4: f64 = 440.0;

/// The 15 final patches of the Phase 0 agent trial (`~/code/enth-scratch/lane-p-trial/run{1,2,3}/`, copied into the
/// P1a research scratch `03/trial/`), parameter for parameter.
pub fn trial_patches() -> Vec<Trial> {
    let pluck = ToyPerf {
        note_hz: A3,
        dur_s: 2.0,
        hold_s: 1.5,
    };
    let pad = ToyPerf {
        note_hz: C3,
        dur_s: 6.0,
        hold_s: 3.5,
    };
    let bass = ToyPerf {
        note_hz: C2,
        dur_s: 2.0,
        hold_s: 1.5,
    };
    let bell = ToyPerf {
        note_hz: C5,
        dur_s: 5.0,
        hold_s: 3.0,
    };
    let lead = ToyPerf {
        note_hz: A4,
        dur_s: 3.0,
        hold_s: 2.0,
    };
    vec![
        (
            "g1-pluck-run1",
            tp(0.0, 7000.0, 0.7, 2.0, 150.0, 1.0, 150.0, -60.0, 50.0),
            pluck,
        ),
        (
            "g1-pluck-run2",
            tp(0.0, 6000.0, 0.7, 3.0, 100.0, 1.0, 120.0, -60.0, 50.0),
            pluck,
        ),
        (
            "g1-pluck-run3",
            tp(0.0, 4000.0, 0.7, 4.0, 150.0, 2.0, 250.0, -60.0, 50.0),
            pluck,
        ),
        (
            "g2-pad-run1",
            tp(6.0, 500.0, 0.7, 0.0, 200.0, 700.0, 300.0, -1.0, 1500.0),
            pad,
        ),
        (
            "g2-pad-run2",
            tp(6.0, 500.0, 0.7, 0.0, 200.0, 600.0, 300.0, -1.0, 1500.0),
            pad,
        ),
        (
            "g2-pad-run3",
            tp(6.0, 350.0, 0.7, 0.0, 200.0, 700.0, 400.0, -1.0, 1200.0),
            pad,
        ),
        (
            "g3-buzzy-bass-run1",
            tp(0.0, 5000.0, 1.5, 1.0, 150.0, 3.0, 100.0, -2.0, 100.0),
            bass,
        ),
        (
            "g3-buzzy-bass-run2",
            tp(0.0, 8000.0, 0.7, 0.0, 200.0, 5.0, 100.0, -2.0, 100.0),
            bass,
        ),
        (
            "g3-buzzy-bass-run3",
            tp(0.0, 5000.0, 1.5, 1.0, 120.0, 3.0, 100.0, -2.0, 100.0),
            bass,
        ),
        (
            "g4-bell-run1",
            tp(24.0, 2500.0, 0.7, 1.0, 300.0, 10.0, 2000.0, -60.0, 800.0),
            bell,
        ),
        (
            "g4-bell-run2",
            tp(24.0, 2000.0, 0.7, 0.0, 200.0, 10.0, 2000.0, -45.0, 300.0),
            bell,
        ),
        (
            "g4-bell-run3",
            tp(24.0, 3000.0, 0.7, 0.0, 200.0, 6.0, 2000.0, -60.0, 800.0),
            bell,
        ),
        (
            "g5-supersaw-lead-run1",
            tp(26.84, 14000.0, 1.0, 0.0, 200.0, 5.0, 40.0, -6.0, 150.0),
            lead,
        ),
        (
            "g5-supersaw-lead-run2",
            tp(33.1, 9000.0, 0.7, 0.0, 200.0, 8.0, 60.0, -4.4, 150.0),
            lead,
        ),
        (
            "g5-supersaw-lead-run3",
            tp(24.6, 6000.0, 2.0, 0.0, 200.0, 5.0, 40.0, -5.4, 150.0),
            lead,
        ),
    ]
}

/// The probe's goal lists (`spikes/spike-agent/goals.toml`), by goal prefix of the trial names.
pub fn goal_list(name: &str) -> &'static str {
    match &name[..2] {
        "g1" => "attack<10ms, brightness>2kHz, sustain<-30dB, duration<800ms, pitch=A3±30ct",
        "g2" => {
            "attack>400ms, brightness<700Hz, sustain>-4dB, release>800ms, duration>4000ms, pitch=C3±30ct"
        }
        "g3" => {
            "pitch=C2±30ct, attack<15ms, brightness>1.2kHz, brightness<2.5kHz, sustain>-6dB, duration>1200ms, release<300ms"
        }
        "g4" => {
            "pitch=C5±40ct, attack>8ms, attack<40ms, brightness>800Hz, brightness<3kHz, decay>600ms, sustain<-35dB, duration>2000ms, beating>2Hz, beating<8Hz"
        }
        _ => {
            "pitch=A4±40ct, attack<20ms, brightness>3kHz, sustain>-4dB, beating>3Hz, beating<9Hz, release<400ms, duration>2000ms"
        }
    }
}

/// Nearest-rank quantile; NaN when empty.
pub fn quantile(v: &mut [f64], p: f64) -> f64 {
    if v.is_empty() {
        return f64::NAN;
    }
    v.sort_by(f64::total_cmp);
    v[((v.len() - 1) as f64 * p).round() as usize]
}

pub fn pct(k: usize, n: usize) -> f64 {
    100.0 * k as f64 / n.max(1) as f64
}
