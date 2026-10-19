//! Heuristic start for the toy template (design.md section 8, step 1): estimate the waveform order and the nine
//! continuous parameters from the target audio alone.
//!
//! * Beat rate of the 10 ms RMS (autocorrelation) gives the detune; detuned oscillators make the RMS beat.
//! * A crest (max-held) envelope gives attack, decay, sustain and release.
//! * Harmonic roll-off relative to the guessed waveform's ideal spectrum, early versus steady state, gives cutoff,
//!   filter-envelope amount and decay.
//! * Even/odd harmonic balance and H3/H1 guess the waveform.
//!
//! Resonance is not estimated (it comes out at 0.3, near chance). Pure: no I/O, no clocks.

use crate::render::{RenderCfg, Wave};
use crate::search::Start;
use core::f32::consts::{LN_2, PI};
use realfft::RealFftPlanner;

fn inv_exp_map(v: f32, lo: f32, hi: f32) -> f64 {
    f64::from(libm::logf(v.clamp(lo, hi) / lo) / libm::logf(hi / lo))
}

#[must_use]
pub fn heuristic(cfg: &RenderCfg, audio: &[f32]) -> Start {
    let sr = cfg.sample_rate;
    let f0 = cfg.note_hz;
    let no = cfg.note_off_s;
    let fl = (sr * 0.01) as usize;
    let rms: Vec<f32> = audio
        .chunks(fl)
        .map(|c| libm::sqrtf(c.iter().map(|v| v * v).sum::<f32>() / c.len() as f32))
        .collect();
    let nfr = rms.len();
    let fr = |t: f32| ((t / 0.01) as usize).min(nfr - 1);
    let off = fr(no);
    // --- beat detection on the held part -> detune ---
    let seg = &rms[fr(no / 3.0)..off];
    let m = seg.iter().sum::<f32>() / seg.len().max(1) as f32;
    let x: Vec<f32> = seg.iter().map(|v| v - m).collect();
    let e0: f32 = x.iter().map(|v| v * v).sum::<f32>().max(1e-12);
    let (mut best_lag, mut best_r) = (0usize, 0f32);
    for lag in 12..=80usize {
        if lag >= x.len() {
            break;
        }
        let r: f32 = (0..x.len() - lag).map(|i| x[i] * x[i + lag]).sum::<f32>() / e0;
        if r > best_r {
            best_r = r;
            best_lag = lag;
        }
    }
    let beating = best_r > 0.3 && best_lag > 0;
    let det_cents = if beating {
        let fb = 1.0 / (best_lag as f32 * 0.01);
        (fb * 1200.0 / (f0 * LN_2)).clamp(0.0, 50.0)
    } else {
        3.0
    };
    // --- crest envelope ---
    let h = if beating { (best_lag / 2).min(30) } else { 1 };
    let env: Vec<f32> = (0..nfr)
        .map(|i| {
            rms[i.saturating_sub(h)..(i + h + 1).min(nfr)]
                .iter()
                .copied()
                .fold(0.0, f32::max)
        })
        .collect();
    let (mut ipk, mut pk) = (0usize, 0f32);
    for i in 0..off {
        if env[i] > pk {
            pk = env[i];
            ipk = i;
        }
    }
    let pk = pk.max(1e-9);
    // attack from the raw RMS up to its first local maximum (before the first beat trough)
    let ifp = (1..off.saturating_sub(1))
        .find(|&i| rms[i] >= rms[i - 1] && rms[i] >= rms[i + 1] && rms[i] > 0.3 * pk)
        .unwrap_or(ipk);
    let i90 = (0..=ifp).find(|&i| rms[i] >= 0.9 * rms[ifp]).unwrap_or(ifp);
    let a = ((i90 as f32 + 0.5) * 0.01 / 0.9).clamp(0.001, 1.0);
    let late = fr(0.9 * no);
    let s = ((late..off).map(|i| env[i]).sum::<f32>() / (off - late).max(1) as f32 / pk)
        .clamp(0.0, 1.0);
    let exp_m2 = libm::expf(-2.0);
    let thr = pk * (s + (1.0 - s) * exp_m2);
    let d = if s > 0.9 {
        0.3
    } else {
        let ih = (i90..off).find(|&i| env[i] <= thr).unwrap_or(off);
        (2.0 * (ih as f32 - i90 as f32) * 0.01).clamp(0.005, 2.0)
    };
    let l0 = env[off].max(1e-9);
    let l1 = rms[fr(no + 0.1)].max(1e-9);
    let r = if l1 >= l0 {
        2.0
    } else {
        (4.0 * 0.1 / libm::logf(l0 / l1)).clamp(0.005, 2.0)
    };
    // --- spectra ---
    let n = 2048usize;
    let mut planner = RealFftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(n);
    let win: Vec<f32> = (0..n)
        .map(|i| 0.5 - 0.5 * libm::cosf(2.0 * PI * i as f32 / n as f32))
        .collect();
    let mut inb = fft.make_input_vec();
    let mut out = fft.make_output_vec();
    let mut spec_at = |t: f32| -> Vec<f32> {
        let p = ((t * sr) as usize).min(audio.len() - n);
        for i in 0..n {
            inb[i] = audio[p + i] * win[i];
        }
        fft.process(&mut inb, &mut out)
            .expect("fft sizes match the plan");
        out.iter().map(|c| libm::sqrtf(c.norm_sqr())).collect()
    };
    let bin = sr / n as f32;
    let harm = |sp: &[f32], k: usize| -> f32 {
        let c = (k as f32 * f0 / bin).round() as usize;
        (c.saturating_sub(2)..=c + 2)
            .map(|i| sp[i.min(sp.len() - 1)])
            .fold(0.0, f32::max)
    };
    let steady = spec_at(no * 5.0 / 6.0);
    let hs: Vec<f32> = (1..=6).map(|k| harm(&steady, k)).collect();
    let (h1, h2, h3, h4, h5, h6) = (hs[0], hs[1], hs[2], hs[3], hs[4], hs[5]);
    let even = (h2 + h4 + h6) / (h1 + h3 + h5).max(1e-12);
    let guess = if even > 0.15 {
        0
    } else if h3 / h1.max(1e-12) > 0.2 {
        1
    } else {
        2
    };
    let wave_order = match guess {
        0 => [Wave::Saw, Wave::Square, Wave::Triangle],
        1 => [Wave::Square, Wave::Saw, Wave::Triangle],
        _ => [Wave::Triangle, Wave::Square, Wave::Saw],
    };
    // cutoff from the roll-off: first harmonic whose level (relative to the waveform's ideal spectrum, normalised at
    // k = 1) drops below -6 dB; beyond fc a 2-pole lowpass gives rel ~ (fc/f)^2
    let kmax = (15_000.0 / f0) as usize;
    let fc_of = |sp: &[f32]| -> f32 {
        let h1 = harm(sp, 1).max(1e-12);
        let mut k = 2usize;
        while k <= kmax {
            if guess != 0 && k.is_multiple_of(2) {
                k += 1;
                continue;
            }
            let ideal = if guess == 2 {
                1.0 / (k * k) as f32
            } else {
                1.0 / k as f32
            };
            let rel = harm(sp, k) / h1 / ideal;
            if rel < 0.5 {
                return (k as f32 * f0 * libm::sqrtf(rel).max(0.05)).max(50.0);
            }
            k += 1;
        }
        16_000.0
    };
    let fc_ss = fc_of(&steady).clamp(50.0, 16_000.0);
    let t_early = 0.005f32;
    let fc_early = fc_of(&spec_at(t_early)).clamp(50.0, 16_000.0);
    let oct = libm::log2f(fc_early / fc_ss).max(0.0);
    let mut fdec = 0.3f32;
    if oct > 0.3 {
        let target = fc_ss * libm::exp2f(oct * exp_m2);
        let mut t = t_early;
        while t < 0.8 * no {
            if fc_of(&spec_at(t)) <= target {
                break;
            }
            t += 0.01;
        }
        fdec = (2.0 * (t - t_early).max(0.005)).clamp(0.005, 2.0);
    }
    let cont = vec![
        f64::from(det_cents / 50.0),
        inv_exp_map(fc_ss, 50.0, 16_000.0),
        0.3,
        f64::from((oct / 6.0).min(1.0)),
        inv_exp_map(fdec, 0.005, 2.0),
        inv_exp_map(a, 0.001, 1.0),
        inv_exp_map(d, 0.005, 2.0),
        f64::from(s),
        inv_exp_map(r, 0.005, 2.0),
    ];
    Start { wave_order, cont }
}
