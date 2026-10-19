//! Click and discontinuity flags for stimuli. **Nothing is filtered by duration or content**: the owner wants short
//! and percussive sounds in the tests. This only marks stimuli whose waveform has something a listener could hear as
//! a defect, so the report can say "this trial had a click" after the fact (it is never shown during a session,
//! because a flag on one side of a trial would unblind it).
//!
//! | Flag | Meaning |
//! | --- | --- |
//! | `click` | one isolated step between neighbouring samples of at least 10 % of the clip's peak: no other step within 50 ms is even a quarter as big. A periodic waveform (a saw's reset, a square's edges) has steps in every window, so it is not flagged, even when it decays fast (a plucked saw loses about half its size per period, and a band-limited saw splits its reset between two samples); a percussive attack is many large steps, also not flagged |
//! | `starts_nonzero` | the first sample is at least 5 % of the peak: the clip starts with a step |
//! | `ends_abruptly` | the last sample is at least 5 % of the peak: the clip is cut, not faded |
//! | `non_finite` | a NaN or infinity |
//! | `silent` | the clip is all but silent (peak under 1e-5) |

use std::collections::VecDeque;

use serde::{Deserialize, Serialize};

/// A step must be at least this share of the clip's peak to count as a click.
pub const JUMP_MIN_REL: f32 = 0.10;
/// A click is isolated when no other step within the window is at least this share of its size. A quarter, not
/// a half: a saw that decays fast (a pluck, a percussive hit) has resets that shrink by about half every period,
/// and a band-limited reset is split between two samples, so the next reset can be well under half the first.
pub const ISOLATION_RATIO: f32 = 0.25;
/// Half-width of the isolation window.
pub const ISOLATION_WINDOW_S: f64 = 0.05;
/// First and last samples above this share of the peak mark a start or end step.
pub const EDGE_REL: f32 = 0.05;
/// Flags of one kind closer than this are merged into one.
pub const MERGE_S: f64 = 0.005;
/// At most this many flags of one kind per clip are kept.
pub const MAX_PER_KIND: usize = 8;

/// What a flag says.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FlagKind {
    /// An isolated step inside the clip.
    Click,
    /// The clip starts with a step.
    StartsNonzero,
    /// The clip ends with a step.
    EndsAbruptly,
    /// A NaN or infinity.
    NonFinite,
    /// Nothing audible.
    Silent,
}

/// One finding in a clip.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Flag {
    /// What was found.
    pub kind: FlagKind,
    /// Where, in seconds from the start of the clip.
    pub at_s: f64,
    /// A short description with the numbers.
    pub detail: String,
}

/// `out[j] = max(a[j + 1 - len ..= j])` (the window shrinks at the start).
fn sliding_max(a: &[f32], len: usize) -> Vec<f32> {
    let len = len.max(1);
    let mut dq: VecDeque<usize> = VecDeque::new();
    let mut out = Vec::with_capacity(a.len());
    for (j, v) in a.iter().enumerate() {
        while dq.back().is_some_and(|&b| a[b] <= *v) {
            dq.pop_back();
        }
        dq.push_back(j);
        while dq.front().is_some_and(|&f| f + len <= j) {
            dq.pop_front();
        }
        out.push(dq.front().map_or(0.0, |&f| a[f]));
    }
    out
}

fn push_merged(flags: &mut Vec<Flag>, f: Flag) {
    let near = flags.iter().rev().find(|g| g.kind == f.kind);
    if near.is_some_and(|g| (f.at_s - g.at_s).abs() < MERGE_S) {
        return;
    }
    if flags.iter().filter(|g| g.kind == f.kind).count() < MAX_PER_KIND {
        flags.push(f);
    }
}

fn check_channel(x: &[f32], sr: u32, peak: f32, label: &str, flags: &mut Vec<Flag>) {
    let n = x.len();
    let t = |i: usize| i as f64 / f64::from(sr);
    if n == 0 {
        return;
    }
    if let Some(i) = x.iter().position(|v| !v.is_finite()) {
        push_merged(
            flags,
            Flag {
                kind: FlagKind::NonFinite,
                at_s: t(i),
                detail: format!("{label}: sample {i} is not finite"),
            },
        );
        return;
    }
    if x[0].abs() >= EDGE_REL * peak {
        push_merged(
            flags,
            Flag {
                kind: FlagKind::StartsNonzero,
                at_s: 0.0,
                detail: format!("{label}: first sample {:.3} of peak {peak:.3}", x[0]),
            },
        );
    }
    if x[n - 1].abs() >= EDGE_REL * peak {
        push_merged(
            flags,
            Flag {
                kind: FlagKind::EndsAbruptly,
                at_s: t(n - 1),
                detail: format!("{label}: last sample {:.3} of peak {peak:.3}", x[n - 1]),
            },
        );
    }
    // steps between neighbouring samples
    let mut a = vec![0.0f32; n];
    for i in 1..n {
        a[i] = (x[i] - x[i - 1]).abs();
    }
    let w = ((ISOLATION_WINDOW_S * f64::from(sr)) as usize).max(4);
    let span = w - 2;
    let left_max = sliding_max(&a, span); // left_max[j] covers [j + 1 - span, j]
    let rev: Vec<f32> = a.iter().rev().copied().collect();
    let right_max = sliding_max(&rev, span); // right_max[n - 1 - j] covers [j, j + span - 1]
    for i in 3..n.saturating_sub(3) {
        let step = a[i];
        if step < JUMP_MIN_REL * peak || step < a[i - 1] || step < a[i + 1] {
            continue;
        }
        // the neighbours at distance 3 to w: [i - w, i - 3] and [i + 3, i + w] (i + 3 < n by the loop bound)
        let left = left_max[i - 3];
        let right = right_max[n - 4 - i];
        if left < ISOLATION_RATIO * step && right < ISOLATION_RATIO * step {
            push_merged(
                flags,
                Flag {
                    kind: FlagKind::Click,
                    at_s: t(i),
                    detail: format!(
                        "{label}: a step of {:.1} % of the peak with no similar step within {:.0} ms",
                        100.0 * step / peak,
                        1000.0 * ISOLATION_WINDOW_S
                    ),
                },
            );
        }
    }
}

/// Check a final clip (`channels` as the listener hears it, after level matching).
#[must_use]
pub fn check(channels: &[&[f32]], sample_rate: u32) -> Vec<Flag> {
    let mut flags = Vec::new();
    let finite = channels.iter().all(|c| c.iter().all(|v| v.is_finite()));
    let peak = if finite {
        channels
            .iter()
            .flat_map(|c| c.iter())
            .fold(0.0f32, |m, v| m.max(v.abs()))
    } else {
        1.0
    };
    if finite && peak < 1e-5 {
        flags.push(Flag {
            kind: FlagKind::Silent,
            at_s: 0.0,
            detail: format!("peak {peak:e}"),
        });
        return flags;
    }
    for (c, x) in channels.iter().enumerate() {
        let label = match (channels.len(), c) {
            (1, _) => "mono",
            (_, 0) => "left",
            _ => "right",
        };
        check_channel(x, sample_rate, peak, label, &mut flags);
    }
    flags.sort_by(|a, b| a.at_s.total_cmp(&b.at_s));
    flags
}

/// A one-line summary: `none`, or the kinds and counts (`click x2, ends_abruptly`).
#[must_use]
pub fn summary(flags: &[Flag]) -> String {
    if flags.is_empty() {
        return "none".to_string();
    }
    let mut kinds: Vec<(String, usize)> = Vec::new();
    for f in flags {
        let name = serde_json::to_string(&f.kind)
            .unwrap_or_default()
            .trim_matches('"')
            .to_string();
        match kinds.iter_mut().find(|(k, _)| *k == name) {
            Some((_, c)) => *c += 1,
            None => kinds.push((name, 1)),
        }
    }
    kinds
        .iter()
        .map(|(k, c)| {
            if *c > 1 {
                format!("{k} x{c}")
            } else {
                k.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: u32 = 48_000;

    fn saw(freq: f64, seconds: f64) -> Vec<f32> {
        let n = (seconds * f64::from(SR)) as usize;
        (0..n)
            .map(|i| {
                let ph = (i as f64 * freq / f64::from(SR)).fract();
                (0.5 * (2.0 * ph - 1.0)) as f32
            })
            .collect()
    }

    fn faded(mut x: Vec<f32>) -> Vec<f32> {
        // start at zero, end faded
        let n = x.len();
        for i in 0..200 {
            x[i] *= i as f32 / 200.0;
            x[n - 1 - i] *= i as f32 / 200.0;
        }
        x
    }

    fn kinds(f: &[Flag]) -> Vec<FlagKind> {
        f.iter().map(|f| f.kind).collect()
    }

    #[test]
    fn a_periodic_saw_has_no_clicks_at_any_pitch() {
        for hz in [20.0, 55.0, 220.0, 3520.0] {
            let x = faded(saw(hz, 1.0));
            assert!(
                check(&[&x], SR).is_empty(),
                "{hz} Hz: {:?}",
                check(&[&x], SR)
            );
        }
    }

    #[test]
    fn a_fast_decaying_saw_after_silence_is_not_a_click() {
        // 220 Hz at 48 kHz (a 218-sample period) with a 300-sample time constant: each reset is about half the last.
        // It starts mid-ramp at zero, so the first reset is the loudest step and only the next one is its neighbour.
        let x: Vec<f32> = (0..SR / 2)
            .map(|i| {
                let t = i.saturating_sub(1_000);
                if i < 1_000 {
                    return 0.0;
                }
                let ph = (f64::from(t) * 220.0 / f64::from(SR) + 0.5).fract();
                (0.9 * (2.0 * ph - 1.0) * libm::exp(-f64::from(t) / 300.0)) as f32
            })
            .collect();
        let f = check(&[&x], SR);
        assert!(!kinds(&f).contains(&FlagKind::Click), "{f:?}");
    }

    #[test]
    fn an_injected_click_is_flagged_once_where_it_is() {
        let mut x = faded(
            (0..SR)
                .map(|i| 0.3 * libm::sin(f64::from(i) * 0.05) as f32)
                .collect(),
        );
        x[20_000] += 0.4;
        let f = check(&[&x], SR);
        assert_eq!(kinds(&f), vec![FlagKind::Click], "{f:?}");
        assert!(
            (f[0].at_s - 20_000.0 / f64::from(SR)).abs() < 0.003,
            "{f:?}"
        );
    }

    #[test]
    fn a_percussive_attack_is_not_a_click_but_a_hard_start_is_marked() {
        // noise burst starting at sample 0 with an exponential decay: many large steps, one big start
        let mut s = 12_345u32;
        let x: Vec<f32> = (0..24_000)
            .map(|i| {
                s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let n = (s >> 8) as f32 / 8_388_608.0 - 1.0;
                n * libm::exp(-f64::from(i) / 2_000.0) as f32 * 0.5
            })
            .collect();
        let f = check(&[&x], SR);
        assert!(!kinds(&f).contains(&FlagKind::Click), "{f:?}");
        assert!(kinds(&f).contains(&FlagKind::StartsNonzero), "{f:?}");
    }

    #[test]
    fn a_cut_end_nan_and_silence_are_marked() {
        let x = saw(220.0, 0.5); // neither faded in nor out
        let f = check(&[&x], SR);
        assert!(kinds(&f).contains(&FlagKind::EndsAbruptly), "{f:?}");
        let mut y = faded(saw(220.0, 0.2));
        y[100] = f32::NAN;
        assert_eq!(kinds(&check(&[&y], SR)), vec![FlagKind::NonFinite]);
        assert_eq!(
            kinds(&check(&[&vec![0.0; 1000]], SR)),
            vec![FlagKind::Silent]
        );
    }

    #[test]
    fn short_clips_and_stereo_work_and_the_summary_reads() {
        let tiny = vec![0.0, 0.5, -0.5, 0.0, 0.0, 0.0, 0.0, 0.0];
        let _ = check(&[&tiny], SR);
        let _ = check(&[&[0.1f32][..]], SR);
        let _ = check(&[&[][..]], SR);
        let l = faded(
            (0..SR / 2)
                .map(|i| 0.3 * libm::sin(f64::from(i) * 0.03) as f32)
                .collect(),
        );
        let mut r = l.clone();
        r[10_000] += 0.4;
        let f = check(&[&l, &r], SR);
        assert_eq!(kinds(&f), vec![FlagKind::Click]);
        assert!(f[0].detail.starts_with("right"));
        assert_eq!(summary(&[]), "none");
        assert_eq!(summary(&f), "click");
    }
}
