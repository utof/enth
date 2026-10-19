//! The design step: calibrate the three decays, render the sources, match levels, plan the trials.

use std::collections::HashSet;
use std::io::Write;
use std::time::Instant;

use enth_render::RenderOutput;
use spike_metric::rng::Rng;

use super::fund::{self, Kind};
use super::rt60::rt60_t20;
use super::sources::{Source, render_dry, render_ours_wet, sources};
use super::{
    Calibration, Candidate, ClipInfo, Config, Manifest, SCHEMA_MANIFEST, SourceInfo, Trial,
};
use crate::Res;
use crate::flags;
use crate::loudness::{
    PEAK_CEILING, StereoMeter, TARGET_LUFS, db_to_gain, gain_to_db, peak_dbfs, rms_dbfs,
};
use crate::wav::Wav;

/// A finished design: the manifest and the clip audio (written to `clips/` by the caller).
#[derive(Debug)]
pub struct Designed {
    /// The key.
    pub manifest: Manifest,
    /// `(clip id, audio)`.
    pub clips: Vec<(String, Wav)>,
}

/// One reverb's mix of a source before the overall level match.
struct Mix {
    cand: Candidate,
    wet_db: f64,
    audio: [Vec<f32>; 2],
}

/// Fade-out at the very end of each clip (the tails are far below -60 dB by then; this only closes the file).
const END_FADE_S: f64 = 0.03;
/// Calibration stops when the measured RT60 is within this share of the target (looser in quick mode, where fundsp
/// runs unoptimised and every round costs seconds).
const RT60_TOLERANCE: f64 = 0.03;
const RT60_TOLERANCE_QUICK: f64 = 0.12;
/// At most this many measure-and-adjust rounds per reverb.
const MAX_ROUNDS: usize = 8;

/// Adjust `value` (an RT60 control, roughly proportional to the measured RT60) until `measure(value)` is within
/// `tolerance` of `target`. Returns `(value, measured, rounds)` of the best round.
fn calibrate(
    target: f64,
    start: f64,
    tolerance: f64,
    max_rounds: usize,
    mut measure: impl FnMut(f64) -> Res<Option<f64>>,
) -> Res<(f64, f64, usize)> {
    let mut value = start;
    let mut best: Option<(f64, f64, f64)> = None; // (relative error, value, measured)
    let mut rounds = 0;
    while rounds < max_rounds {
        rounds += 1;
        // a decay that never reaches -25 dB inside the window is longer than the window: shorten the control
        let measured = measure(value)?;
        let next = match measured {
            Some(m) => {
                let err = (m - target).abs() / target;
                if best.is_none_or(|(e, _, _)| err < e) {
                    best = Some((err, value, m));
                }
                if err <= tolerance {
                    break;
                }
                value * (target / m).clamp(0.25, 4.0)
            }
            None => value * 0.6,
        };
        value = next;
    }
    let (_, v, m) =
        best.ok_or("the decay could not be measured: the reverb's output never fell 25 dB")?;
    Ok((v, m, rounds))
}

fn unique_id(rng: &mut Rng, prefix: &str, used: &mut HashSet<String>) -> String {
    loop {
        let id = format!("{prefix}{:06x}", rng.next_u64() & 0xff_ffff);
        if used.insert(id.clone()) {
            return id;
        }
    }
}

fn frames_of(sample_rate: u32, seconds: f64) -> usize {
    (seconds * f64::from(sample_rate)).round() as usize
}

fn pair_of(out: &RenderOutput) -> [&[f32]; 2] {
    [&out.left, &out.right]
}

/// Clip length: the dry sound plus a tail of `tail_rt60s` reverberation times.
fn clip_seconds(src: &Source, cfg: &Config) -> f64 {
    ((src.dry_end_s + cfg.tail_rt60s * cfg.rt60_s) * 10.0).ceil() / 10.0
}

/// Calibrate the three reverbs, render every source through each, match levels and plan the trials.
///
/// # Errors
/// A render failure or a decay that cannot be measured.
pub fn design(cfg: &Config, quick: bool, out: &mut dyn Write) -> Res<Designed> {
    let sr = cfg.sample_rate;
    let srcs = sources(quick);
    let cal_source = srcs[2].first_hit_only();

    // 1. calibrate: every reverb gets the same transient and is tuned until its measured RT60 is the target
    writeln!(
        out,
        "calibrating the three decays to RT60 = {} s ...",
        cfg.rt60_s
    )?;
    let t_cal = Instant::now();
    // long enough that a decay up to about twice the target still ends well inside the window (quick mode: shorter)
    let window = if quick { 1.2 } else { 2.0 };
    let cal_seconds = cal_source.dry_end_s + window * cfg.rt60_s + 0.1;
    let cal_frames = frames_of(sr, cal_seconds);
    let start = frames_of(sr, cal_source.dry_end_s);
    let cal_dry = render_dry(&cal_source, sr, cfg.seed, cal_seconds)?;
    let mut calibration = Vec::new();
    let tol = if quick {
        RT60_TOLERANCE_QUICK
    } else {
        RT60_TOLERANCE
    };
    let (v, m, rounds) = calibrate(
        cfg.rt60_s,
        cfg.rt60_s * 1000.0,
        tol,
        MAX_ROUNDS,
        |decay_ms| {
            let w = render_ours_wet(&cal_source, sr, cfg.seed, cal_seconds, cfg.wet_db, decay_ms)?;
            Ok(rt60_t20(&w.left, &w.right, sr, start))
        },
    )?;
    calibration.push(Calibration {
        candidate: Candidate::Ours,
        parameter: "decay (ms)".into(),
        value: v,
        measured_rt60_s: m,
        rounds,
    });
    let ours_decay_ms = v;
    let mut fund_time = [0.0f64; 2];
    for (k, (cand, kind)) in [
        (Candidate::Reverb2, Kind::Reverb2),
        (Candidate::Reverb4, Kind::Reverb4),
    ]
    .into_iter()
    .enumerate()
    {
        let (v, m, rounds) = calibrate(cfg.rt60_s, cfg.rt60_s, tol, MAX_ROUNDS, |time| {
            let [l, r] = fund::wet(kind, time, sr, pair_of(&cal_dry), cal_frames);
            Ok(rt60_t20(&l, &r, sr, start))
        })?;
        calibration.push(Calibration {
            candidate: cand,
            parameter: "time (s)".into(),
            value: v,
            measured_rt60_s: m,
            rounds,
        });
        fund_time[k] = v;
    }
    for c in &calibration {
        writeln!(
            out,
            "  {}: {} = {:.3} gives RT60 {:.2} s ({} rounds)",
            c.candidate.label(),
            c.parameter,
            c.value,
            c.measured_rt60_s,
            c.rounds
        )?;
        if (c.measured_rt60_s - cfg.rt60_s).abs() > 0.1 * cfg.rt60_s {
            writeln!(
                out,
                "  warning: {} could not be brought within 10 % of the target decay; the A/B is less fair for it",
                c.candidate.label()
            )?;
        }
    }
    writeln!(
        out,
        "  calibrated in {:.1} s",
        t_cal.elapsed().as_secs_f64()
    )?;

    // 2. render the sources through each reverb; match the wet level, then the overall level per source
    let mut rng = Rng::new(cfg.seed ^ 0x7e7e_b0b0_0000_0001);
    let mut used: HashSet<String> = HashSet::new();
    let mut meter = StereoMeter::new(sr);
    let mut clips = Vec::new();
    let mut infos = Vec::new();
    let mut source_infos = Vec::new();
    for src in &srcs {
        let seconds = clip_seconds(src, cfg);
        let frames = frames_of(sr, seconds);
        let t_src = Instant::now();
        writeln!(out, "rendering {} ({seconds:.1} s) ...", src.name)?;
        source_infos.push(SourceInfo {
            name: src.name.into(),
            description: src.description.into(),
            seconds,
        });
        let dry = render_dry(src, sr, cfg.seed, seconds)?;
        let ours = render_ours_wet(src, sr, cfg.seed, seconds, cfg.wet_db, ours_decay_ms)?;
        let mut wets: Vec<(Candidate, [Vec<f32>; 2])> =
            vec![(Candidate::Ours, [ours.left.clone(), ours.right.clone()])];
        wets.push((
            Candidate::Reverb2,
            fund::wet(Kind::Reverb2, fund_time[0], sr, pair_of(&dry), frames),
        ));
        wets.push((
            Candidate::Reverb4,
            fund::wet(Kind::Reverb4, fund_time[1], sr, pair_of(&dry), frames),
        ));
        let ours_lufs = meter
            .integrated_any_level(&wets[0].1[0], &wets[0].1[1])
            .ok_or_else(|| format!("{}: our reverb's wet signal is silent", src.name))?;
        // mixes: the same dry signal plus each wet signal at our wet loudness
        let mut mixes = Vec::new();
        for (cand, w) in wets {
            let wet_db = if cand == Candidate::Ours {
                0.0
            } else {
                let l = meter
                    .integrated_any_level(&w[0], &w[1])
                    .ok_or_else(|| format!("{}: {} is silent", src.name, cand.label()))?;
                ours_lufs - l
            };
            let g = db_to_gain(wet_db) as f32;
            let mix: Vec<Vec<f32>> = (0..2)
                .map(|c| {
                    let d: &[f32] = if c == 0 { &dry.left } else { &dry.right };
                    (0..frames)
                        .map(|i| d.get(i).copied().unwrap_or(0.0) + g * w[c][i])
                        .collect()
                })
                .collect();
            let mut it = mix.into_iter();
            let (l, r) = (
                it.next().expect("two channels"),
                it.next().expect("two channels"),
            );
            mixes.push(Mix {
                cand,
                wet_db,
                audio: [l, r],
            });
        }
        // the end fade, then the loudness of each mix and one shared peak factor for the group
        let fade = frames_of(sr, END_FADE_S).max(1);
        for m in &mut mixes {
            for ch in &mut m.audio {
                let n = ch.len();
                for (i, v) in ch.iter_mut().enumerate().skip(n.saturating_sub(fade)) {
                    *v *= (n - 1 - i) as f32 / fade as f32;
                }
            }
        }
        let gains: Vec<f64> = mixes
            .iter()
            .map(|m| {
                let l = meter
                    .integrated_any_level(&m.audio[0], &m.audio[1])
                    .ok_or_else(|| format!("{}: {} mix is silent", src.name, m.cand.label()))?;
                Ok::<f64, String>(db_to_gain(TARGET_LUFS - l))
            })
            .collect::<Result<_, _>>()?;
        let peak = mixes
            .iter()
            .zip(&gains)
            .map(|(m, g)| {
                f64::from(m.audio.iter().flatten().fold(0.0f32, |a, v| a.max(v.abs()))) * g
            })
            .fold(0.0, f64::max);
        let shared = if peak > f64::from(PEAK_CEILING) {
            f64::from(PEAK_CEILING) / peak
        } else {
            1.0
        };
        for (m, g) in mixes.into_iter().zip(gains) {
            let gain = g * shared;
            let [mut l, mut r] = m.audio;
            for v in l.iter_mut().chain(r.iter_mut()) {
                *v = (f64::from(*v) * gain) as f32;
            }
            let lufs = meter
                .integrated_any_level(&l, &r)
                .unwrap_or(f64::NEG_INFINITY);
            let id = unique_id(&mut rng, "c", &mut used);
            infos.push(ClipInfo {
                id: id.clone(),
                source: src.name.into(),
                candidate: m.cand,
                wet_match_db: m.wet_db,
                final_gain_db: gain_to_db(gain),
                lufs,
                peak_dbfs: peak_dbfs(&[&l, &r]),
                rms_dbfs: rms_dbfs(&[&l, &r]),
                flags: flags::check(&[&l, &r], sr),
            });
            clips.push((id, Wav::stereo(sr, l, r)));
        }
        writeln!(
            out,
            "  {} done in {:.1} s",
            src.name,
            t_src.elapsed().as_secs_f64()
        )?;
    }

    // 3. trials: every source x pair x repeat, with the order of the two reverbs balanced over the repeats
    let pairs = [
        (Candidate::Ours, Candidate::Reverb2),
        (Candidate::Ours, Candidate::Reverb4),
        (Candidate::Reverb2, Candidate::Reverb4),
    ];
    let clip_of = |source: &str, cand: Candidate| {
        infos
            .iter()
            .find(|c| c.source == source && c.candidate == cand)
            .map(|c| c.id.clone())
            .expect("every reverb has a clip per source")
    };
    let mut trials = Vec::new();
    for src in &srcs {
        for (first, second) in pairs {
            let flip = rng.uniform() < 0.5;
            let mut order: Vec<bool> = (0..cfg.repeats).map(|i| (i % 2 == 0) != flip).collect();
            rng.shuffle(&mut order);
            for (rep, first_in_a) in order.into_iter().enumerate() {
                let (a, b) = if first_in_a {
                    (first, second)
                } else {
                    (second, first)
                };
                trials.push(Trial {
                    n: 0,
                    id: String::new(),
                    source: src.name.into(),
                    repeat: rep + 1,
                    a: clip_of(src.name, a),
                    b: clip_of(src.name, b),
                    a_candidate: a,
                    b_candidate: b,
                });
            }
        }
    }
    rng.shuffle(&mut trials);
    for (k, t) in trials.iter_mut().enumerate() {
        t.n = k + 1;
        t.id = unique_id(&mut rng, "t", &mut used);
    }
    Ok(Designed {
        manifest: Manifest {
            schema: SCHEMA_MANIFEST.to_string(),
            config: cfg.clone(),
            sources: source_infos,
            calibration,
            clips: infos,
            trials,
        },
        clips,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calibration_converges_on_a_proportional_control() {
        // a toy "reverb" whose RT60 is 0.7 x its control, measured with a little bias
        let mut calls = 0;
        let (v, m, rounds) = calibrate(2.0, 2.0, 0.03, 8, |x| {
            calls += 1;
            Ok(Some(0.7 * x))
        })
        .unwrap();
        assert!(
            (m - 2.0).abs() / 2.0 <= 0.03 && (v * 0.7 - m).abs() < 1e-12,
            "{v} {m}"
        );
        assert!(rounds <= 3 && calls == rounds);
    }

    #[test]
    fn calibration_recovers_from_a_window_that_is_too_short_and_reports_failure() {
        // None means "longer than the window": the control is shortened until a decay can be measured
        let (_, m, _) = calibrate(1.0, 8.0, 0.03, 8, |x| Ok((x < 3.0).then_some(x))).unwrap();
        assert!((m - 1.0).abs() < 0.1, "{m}");
        assert!(calibrate(1.0, 1.0, 0.03, 4, |_| Ok(None)).is_err());
    }
}
