//! `latency`: the time from a parameter event to an audible change in the output (knob → sound).
//!
//! The measurement is differential and needs no microphone. Two fresh instances play the same held notes. One
//! also gets a parameter event at an exact frame `F`. The two outputs are bit-identical before `F` (checked: if
//! they are not, the plugin is not deterministic and the number would mean nothing) and differ after it by
//! `delta[i] = with[i] - without[i]`. From `delta`:
//!
//! - **onset**: the first frame at or after `F` where the outputs differ at all. The plugin's own event latency:
//!   0 for a sample-accurate plugin, up to a block for one that applies parameters at block starts, a segment
//!   (32 frames in the engine) for one that applies them at segment boundaries.
//! - **audible**: the first frame where the peak of `|delta|` over the last millisecond reaches `--threshold-db`
//!   (default −60 dBFS). What a listener could notice, which for a smoothed parameter comes after the onset.
//! - **settled**: the first frame where that envelope reaches 90 % of its final level (taken over the last
//!   quarter of the render). How long the change takes to complete: a 10 ms smoother shows up here.
//!
//! The event is placed at several offsets inside its block (`--offsets`, default 0, 17 and 63 for 64-frame blocks)
//! so block-quantised handling shows: for a sample-accurate plugin the onset is the same at every offset, for a
//! plugin that applies events at the next block start it is `block - offset`.
//!
//! Latency is in **frames** (and milliseconds at the sample rate), not wall-clock time: the host is offline
//! and deterministic. What a musician feels is this plus the audio buffer, which the host owns. The wall-clock
//! cost of the block that carries the event is reported too (`process_us`).
//!
//! **Negative controls.** A parameter that does nothing (`--param "Macro 2"` on spike-shell) must end with "no
//! audible change" and exit 1; `--inject delay-blocks=K` delivers the event K blocks late while the measurement
//! still expects it on time, so the onset must grow by K blocks and trip `--max-onset-ms`.

use std::ffi::CStr;

use clack_host::prelude::PluginEntry;
use serde_json::{Value, json};

use crate::notes::{Kind, Schedule, Scheduled};
use crate::query;
use crate::render::{ProcessThread, RenderConfig, Session, run_job_ext};

#[derive(Clone, Debug)]
pub struct Options {
    pub param: String,
    /// The plain value the parameter is set to.
    pub value: f64,
    /// The plain value it is set to at frame 0 in both renders (so a parameter at its default can still change).
    pub from: Option<f64>,
    pub notes: String,
    pub sample_rate: f64,
    pub block: u32,
    /// When the event is due, in seconds (the block containing it is chosen from this).
    pub at_s: f64,
    /// Positions inside the block to test.
    pub offsets: Vec<u32>,
    /// How long to render after the event.
    pub after_s: f64,
    pub threshold_db: f64,
    pub state: Option<Vec<u8>>,
    /// Fault: send the event this many blocks later than the measurement expects.
    pub inject_delay_blocks: u64,
    /// A budget for the onset in milliseconds; exceeding it fails the run.
    pub max_onset_ms: Option<f64>,
    /// A budget for the audible time in milliseconds.
    pub max_audible_ms: Option<f64>,
}

/// What one offset measured.
#[derive(Clone, Debug, PartialEq)]
pub struct Measure {
    pub offset: u32,
    pub event_frame: u64,
    pub onset: Option<u64>,
    pub audible: Option<u64>,
    pub settled: Option<u64>,
    /// The final level of the difference (peak of `|delta|`), for scale.
    pub final_level: f32,
    /// Frames before the event where the two renders differed (must be 0).
    pub early_differences: usize,
}

/// Analyses two interleaved renders. `with` is the one that got the event at `event_frame`.
pub fn analyse(
    without: &[f32],
    with: &[f32],
    channels: usize,
    event_frame: u64,
    sample_rate: f64,
    threshold: f32,
) -> Measure {
    let frames = without.len().min(with.len()) / channels.max(1);
    let delta = |i: usize| -> f32 {
        (0..channels)
            .map(|c| (with[i * channels + c] - without[i * channels + c]).abs())
            .fold(0.0f32, f32::max)
    };
    let differs = |i: usize| {
        (0..channels)
            .any(|c| with[i * channels + c].to_bits() != without[i * channels + c].to_bits())
    };
    let f = (event_frame as usize).min(frames);
    let early = (0..f).filter(|&i| differs(i)).count();
    let onset = (f..frames).find(|&i| differs(i)).map(|i| (i - f) as u64);
    // The envelope: the peak of |delta| over the last millisecond (trailing, so it never reports a change before
    // it has happened), a sliding maximum with a monotone deque of indices whose values fall from the front,
    // the maximum, to the back.
    let window = ((sample_rate / 1000.0).round() as usize).max(1);
    let deltas: Vec<f32> = (0..frames).map(delta).collect();
    let mut env = vec![0.0f32; frames];
    let mut deque: std::collections::VecDeque<usize> = std::collections::VecDeque::new();
    for i in f..frames {
        while deque.back().is_some_and(|&j| deltas[j] <= deltas[i]) {
            deque.pop_back();
        }
        deque.push_back(i);
        while deque.front().is_some_and(|&j| j + window <= i) {
            deque.pop_front();
        }
        env[i] = deque.front().map_or(0.0, |&j| deltas[j]);
    }
    let tail_from = f + (frames - f) * 3 / 4;
    let final_level = (tail_from..frames).map(|i| env[i]).fold(0.0f32, f32::max);
    let audible = (f..frames)
        .find(|&i| env[i] >= threshold)
        .map(|i| (i - f) as u64);
    let settled = if final_level > 0.0 {
        (f..frames)
            .find(|&i| env[i] >= 0.9 * final_level)
            .map(|i| (i - f) as u64)
    } else {
        None
    };
    Measure {
        offset: 0,
        event_frame,
        onset,
        audible,
        settled,
        final_level,
        early_differences: early,
    }
}

fn ms(frames: u64, sample_rate: f64) -> f64 {
    frames as f64 / sample_rate * 1000.0
}

fn render(
    entry: &PluginEntry,
    id: &CStr,
    opts: &Options,
    param_id: u32,
    event: Option<u64>,
    frames: u64,
) -> Result<(Vec<f32>, usize, Vec<u64>), String> {
    let config = RenderConfig {
        sample_rate: opts.sample_rate,
        block: opts.block,
        frames: frames.div_ceil(u64::from(opts.block)) * u64::from(opts.block),
    };
    let mut session = Session::open_bare(entry, id, opts.state.as_deref(), config)?;
    let resolve = |name: &str| query::resolve_param(&session.params, name);
    let mut schedule = crate::notes::parse(&opts.notes, opts.sample_rate, config.frames, &resolve)?;
    let mut events: Vec<Scheduled> = Vec::new();
    if let Some(from) = opts.from {
        events.push(Scheduled {
            frame: 0,
            kind: Kind::Param {
                id: param_id,
                value: from,
            },
        });
    }
    if let Some(frame) = event {
        events.push(Scheduled {
            frame,
            kind: Kind::Param {
                id: param_id,
                value: opts.value,
            },
        });
    }
    events.append(&mut schedule.events);
    events.sort_by_key(|e| (e.frame, e.kind.order()));
    session.schedule = Schedule { events };
    let processor = session.activate()?;
    let mut setup = session.setup(processor);
    setup.record_timing = true;
    let out = run_job_ext(setup, ProcessThread::Dedicated)?;
    session.close(out.processor);
    Ok((out.samples, out.channels, out.process_ns))
}

pub fn run(entry: &PluginEntry, id: &CStr, opts: &Options) -> Result<Value, String> {
    let params = {
        let config = RenderConfig::new(opts.sample_rate, opts.block, 0.1)?;
        Session::open_bare(entry, id, opts.state.as_deref(), config)?.params
    };
    let param_id = query::resolve_param(&params, &opts.param)
        .ok_or_else(|| format!("--param: no parameter named or numbered {:?}", opts.param))?;
    let block = u64::from(opts.block);
    let base_block = ((opts.at_s * opts.sample_rate) as u64) / block;
    let threshold = 10f32.powf(opts.threshold_db as f32 / 20.0);
    let mut rows = Vec::new();
    let mut errors = Vec::new();
    let mut measures = Vec::new();
    for &offset in &opts.offsets {
        if u64::from(offset) >= block {
            return Err(format!(
                "--offsets: {offset} is not inside a block of {block} frames"
            ));
        }
        let event_frame = base_block * block + u64::from(offset);
        let frames = event_frame
            + (opts.after_s * opts.sample_rate) as u64
            + opts.inject_delay_blocks * block;
        let (without, channels, _) = render(entry, id, opts, param_id, None, frames)?;
        let sent_at = event_frame + opts.inject_delay_blocks * block;
        let (with, _, process_ns) = render(entry, id, opts, param_id, Some(sent_at), frames)?;
        let mut m = analyse(
            &without,
            &with,
            channels,
            event_frame,
            opts.sample_rate,
            threshold,
        );
        m.offset = offset;
        let process_us = process_ns
            .get((sent_at / block) as usize)
            .map(|ns| *ns as f64 / 1000.0);
        if m.early_differences > 0 {
            errors.push(format!(
                "offset {offset}: the two renders differ in {} frames before the event; the plugin is not deterministic (or looks ahead), so no latency can be read",
                m.early_differences
            ));
        }
        rows.push(json!({
            "offset": offset,
            "event_frame": event_frame,
            "onset_frames": m.onset,
            "onset_ms": m.onset.map(|f| ms(f, opts.sample_rate)),
            "audible_frames": m.audible,
            "audible_ms": m.audible.map(|f| ms(f, opts.sample_rate)),
            "settled_frames": m.settled,
            "settled_ms": m.settled.map(|f| ms(f, opts.sample_rate)),
            "final_delta_peak": m.final_level,
            "delta_db": if m.final_level > 0.0 { Some(20.0 * f64::from(m.final_level).log10()) } else { None },
            "block_latency": m.onset.map(|f| (u64::from(offset) + f) / block),
            "process_us": process_us,
        }));
        measures.push(m);
    }
    let silent: Vec<u32> = measures
        .iter()
        .filter(|m| m.audible.is_none())
        .map(|m| m.offset)
        .collect();
    if !silent.is_empty() {
        errors.push(format!(
            "no audible change (>= {} dBFS) within {} s of the event at offset(s) {silent:?}: the parameter does nothing, or does nothing the notes can reveal",
            opts.threshold_db, opts.after_s
        ));
    }
    let worst_onset = measures.iter().filter_map(|m| m.onset).max();
    let worst_audible = measures.iter().filter_map(|m| m.audible).max();
    let onsets: Vec<u64> = measures.iter().filter_map(|m| m.onset).collect();
    // Absolute onset = offset + onset; equal across offsets means events land on a grid (block or segment).
    let absolute: Vec<u64> = measures
        .iter()
        .filter_map(|m| m.onset.map(|o| u64::from(m.offset) + o))
        .collect();
    let sample_accurate = onsets.windows(2).all(|w| w[0] == w[1]);
    let grid_quantised =
        absolute.len() > 1 && absolute.windows(2).all(|w| w[0] == w[1]) && !sample_accurate;
    if let (Some(budget), Some(worst)) = (opts.max_onset_ms, worst_onset)
        && ms(worst, opts.sample_rate) > budget
    {
        errors.push(format!(
            "onset {:.3} ms exceeds the budget {budget} ms",
            ms(worst, opts.sample_rate)
        ));
    }
    if let (Some(budget), Some(worst)) = (opts.max_audible_ms, worst_audible)
        && ms(worst, opts.sample_rate) > budget
    {
        errors.push(format!(
            "audible time {:.3} ms exceeds the budget {budget} ms",
            ms(worst, opts.sample_rate)
        ));
    }
    Ok(json!({
        "param": { "name": opts.param, "id": param_id, "from": opts.from, "to": opts.value },
        "block": opts.block,
        "sample_rate": opts.sample_rate,
        "at_s": opts.at_s,
        "threshold_db": opts.threshold_db,
        "inject_delay_blocks": opts.inject_delay_blocks,
        "offsets": rows,
        "summary": {
            "worst_onset_frames": worst_onset,
            "worst_onset_ms": worst_onset.map(|f| ms(f, opts.sample_rate)),
            "worst_audible_frames": worst_audible,
            "worst_audible_ms": worst_audible.map(|f| ms(f, opts.sample_rate)),
            "sample_accurate": sample_accurate,
            "events_land_on_a_grid": grid_quantised,
        },
        "failures": errors,
        "ok": errors.is_empty(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A mono tone of amplitude 0.25 with a step to 0.5 that begins `latency` frames after the event.
    fn pair(event: usize, latency: usize, ramp: usize, frames: usize) -> (Vec<f32>, Vec<f32>) {
        let tone = |i: usize| (i as f32 * 0.3).sin();
        let without: Vec<f32> = (0..frames).map(|i| 0.25 * tone(i)).collect();
        let with: Vec<f32> = (0..frames)
            .map(|i| {
                let start = event + latency;
                let gain = if i < start {
                    0.25
                } else {
                    0.25 + 0.25 * (((i - start + 1) as f32) / ramp.max(1) as f32).min(1.0)
                };
                gain * tone(i)
            })
            .collect();
        (without, with)
    }

    #[test]
    fn onset_audible_and_settled_are_found() {
        let (a, b) = pair(1000, 17, 480, 4800);
        let m = analyse(&a, &b, 1, 1000, 48_000.0, 0.001);
        assert_eq!(m.early_differences, 0);
        // The first differing sample may sit on a zero crossing of the tone, so the onset is the ramp's first
        // non-zero step within a few frames of the latency.
        let onset = m.onset.unwrap();
        assert!((17..=22).contains(&onset), "onset {onset}");
        let audible = m.audible.unwrap();
        assert!(audible >= onset && audible < 100, "audible {audible}");
        let settled = m.settled.unwrap();
        assert!((400..=520).contains(&settled), "settled {settled}");
    }

    #[test]
    fn a_later_event_gives_a_later_onset() {
        let (a, b) = pair(1000, 17, 48, 4800);
        let (a2, b2) = pair(1000, 17 + 64, 48, 4800);
        let near = analyse(&a, &b, 1, 1000, 48_000.0, 0.001).onset.unwrap();
        let far = analyse(&a2, &b2, 1, 1000, 48_000.0, 0.001).onset.unwrap();
        assert!(far >= near + 60 && far <= near + 68, "{near} then {far}");
    }

    #[test]
    fn a_parameter_that_does_nothing_has_no_onset() {
        let (a, _) = pair(1000, 17, 48, 4800);
        let m = analyse(&a, &a, 1, 1000, 48_000.0, 0.001);
        assert_eq!((m.onset, m.audible, m.settled), (None, None, None));
        assert_eq!(m.final_level, 0.0);
    }

    #[test]
    fn a_difference_before_the_event_is_counted() {
        let (a, mut b) = pair(1000, 0, 48, 4800);
        b[10] += 0.5;
        assert_eq!(
            analyse(&a, &b, 1, 1000, 48_000.0, 0.001).early_differences,
            1
        );
    }

    #[test]
    fn stereo_takes_the_louder_channel() {
        let frames = 2000;
        let a = vec![0.0f32; frames * 2];
        let mut b = a.clone();
        for i in 600..frames {
            b[i * 2 + 1] = 0.1; // only the right channel changes
        }
        let m = analyse(&a, &b, 2, 500, 48_000.0, 0.001);
        assert_eq!(m.onset, Some(100));
        assert!((m.final_level - 0.1).abs() < 1e-6);
    }
}
