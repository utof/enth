//! Helpers shared by the hint tests: patch builders, evaluators that change how work is spread (real threads,
//! reversed order), a recording wrapper, and an analytic stand-in for render + describe that runs in microseconds
//! and does not depend on the engine's modules (lanes D1/D2/S2b are still replacing them).

#![allow(dead_code, reason = "each test binary uses a subset of the helpers")]

use enth_analysis::Descriptors;
use enth_analysis::beat::Beating;
use enth_analysis::descriptors::{ModulationDesc, Spectrum};
use enth_analysis::envelope::Envelope;
use enth_analysis::loudness::Level;
use enth_analysis::pitch::Pitch;
use enth_analysis::{Expect, parse_list};
use enth_patch::{ModuleId, ModuleInstance, ModuleKind, Op, Patch, Position, Section, apply};
use enth_solver::{EvalError, Evaluator, Job, measure};

/// `Patch::init()` plus a second oscillator; the two detuned by `fine1` and `fine2` cents.
pub fn two_osc(fine1: f32, fine2: f32) -> Patch {
    let mut p = Patch::init();
    let osc2 = ModuleInstance::new(ModuleId::new("osc2").unwrap(), ModuleKind::Osc);
    let at = Position {
        section: Section::Generators,
        index: 1,
    };
    apply(&mut p, &Op::AddModule { at, module: osc2 }).unwrap();
    set(&mut p, "osc1.fine", fine1);
    set(&mut p, "osc2.fine", fine2);
    p
}

/// Add a filter (`filter1`, an `svf`) after the generators.
pub fn with_filter(mut p: Patch, cutoff: f32) -> Patch {
    let f = ModuleInstance::new(ModuleId::new("filter1").unwrap(), ModuleKind::Svf);
    let at = Position {
        section: Section::VoiceFx,
        index: 0,
    };
    apply(&mut p, &Op::AddModule { at, module: f }).unwrap();
    set(&mut p, "filter1.cutoff", cutoff);
    p
}

/// Set a parameter through an op.
pub fn set(p: &mut Patch, addr: &str, value: f32) {
    apply(
        p,
        &Op::SetParam {
            addr: addr.parse().unwrap(),
            value,
        },
    )
    .unwrap();
}

/// Parse an expectation list.
pub fn items(list: &str) -> Vec<Expect> {
    parse_list(list).unwrap()
}

/// `measure` on `n` real worker threads: worker `w` takes jobs `w, w + n, …`, runs them in reverse order with its
/// own describer, and results go back by index.
pub struct Threads(pub usize);

impl Evaluator for Threads {
    #[allow(
        clippy::disallowed_methods,
        reason = "test-only: real worker threads show that a hint does not depend on the worker count"
    )]
    fn evaluate(&mut self, jobs: &[Job<'_>]) -> Vec<Result<Descriptors, EvalError>> {
        let n = self.0.max(1);
        let mut out: Vec<Option<Result<Descriptors, EvalError>>> = vec![None; jobs.len()];
        std::thread::scope(|s| {
            let workers: Vec<_> = (0..n)
                .map(|w| {
                    s.spawn(move || {
                        let mut describer = None;
                        let mine: Vec<usize> = (w..jobs.len()).step_by(n).collect();
                        mine.into_iter()
                            .rev()
                            .map(|i| (i, measure(&jobs[i], &mut describer)))
                            .collect::<Vec<_>>()
                    })
                })
                .collect();
            for w in workers {
                for (i, r) in w.join().unwrap() {
                    out[i] = Some(r);
                }
            }
        });
        out.into_iter().map(Option::unwrap).collect()
    }
}

/// Wraps an evaluator: counts jobs and batches and records each job's clip length.
pub struct Recorder<E> {
    pub inner: E,
    pub jobs: usize,
    pub batches: usize,
    pub clip_lengths: Vec<f64>,
}

impl<E> Recorder<E> {
    pub fn new(inner: E) -> Self {
        Self {
            inner,
            jobs: 0,
            batches: 0,
            clip_lengths: Vec::new(),
        }
    }
}

impl<E: Evaluator> Evaluator for Recorder<E> {
    fn evaluate(&mut self, jobs: &[Job<'_>]) -> Vec<Result<Descriptors, EvalError>> {
        self.jobs += jobs.len();
        self.batches += 1;
        self.clip_lengths
            .extend(jobs.iter().map(|j| j.perf.duration_s));
        self.inner.evaluate(jobs)
    }
}

/// Evaluates the batch back to front (results still by index).
pub struct Reversed<E>(pub E);

impl<E: Evaluator> Evaluator for Reversed<E> {
    fn evaluate(&mut self, jobs: &[Job<'_>]) -> Vec<Result<Descriptors, EvalError>> {
        let mut out: Vec<Option<Result<Descriptors, EvalError>>> = vec![None; jobs.len()];
        for i in (0..jobs.len()).rev() {
            out[i] = self.0.evaluate(std::slice::from_ref(&jobs[i])).pop();
        }
        out.into_iter().map(Option::unwrap).collect()
    }
}

/// Returns one result too few (a broken evaluator).
pub struct Short;

impl Evaluator for Short {
    fn evaluate(&mut self, jobs: &[Job<'_>]) -> Vec<Result<Descriptors, EvalError>> {
        let mut out = Synthetic.evaluate(jobs);
        out.pop();
        out
    }
}

/// An analytic synth model in place of render + describe: two detuned oscillators, an ADSR, an optional filter
/// and the master level, with time readings quantised to the envelope's 1 ms frames like the real descriptors.
pub struct Synthetic;

impl Evaluator for Synthetic {
    fn evaluate(&mut self, jobs: &[Job<'_>]) -> Vec<Result<Descriptors, EvalError>> {
        jobs.iter().map(|j| Ok(synth(j))).collect()
    }
}

fn db(x: f64) -> f64 {
    20.0 * libm::log10(x.max(1e-9))
}

fn synth(job: &Job<'_>) -> Descriptors {
    let p = &job.patch;
    let get = |a: &str| p.value(&a.parse().unwrap()).map(f64::from);
    let note = job.perf.notes[0];
    let hold_ms = (note.off_s - note.on_s) * 1000.0;
    let clip_ms = (job.perf.duration_s - note.on_s) * 1000.0;
    let f1 = get("osc1.fine").unwrap_or(0.0);
    let f2 = get("osc2.fine");
    let key_hz = 440.0 * libm::pow(2.0, (f64::from(note.key) - 69.0) / 12.0);
    let mean = f2.map_or(f1, |f2| f64::midpoint(f1, f2));
    let f0 = key_hz * libm::pow(2.0, mean / 1200.0);
    let attack = get("env1.attack").unwrap_or(5.0);
    let decay = get("env1.decay").unwrap_or(300.0);
    let sustain = get("env1.sustain").unwrap_or(70.0) / 100.0;
    let release = get("env1.release").unwrap_or(400.0);
    let sustain_db = db(sustain).max(-80.0);
    let released = note.off_s < job.perf.duration_s;
    // Level falls 60 dB in the segment time; the sound lasts until 40 dB below the peak.
    let full = if sustain_db > -40.0 {
        hold_ms + if released { release * 2.0 / 3.0 } else { 0.0 }
    } else {
        (attack + decay * 2.0 / 3.0 * (40.0 / (-sustain_db).max(40.0))).min(hold_ms)
            + if released && sustain_db > -40.0 {
                release * 2.0 / 3.0
            } else {
                0.0
            }
    };
    let truncated = full > clip_ms;
    let duration = libm::round(full.min(clip_ms));
    let steady_ms = if sustain_db > -30.0 {
        hold_ms
    } else {
        decay * 0.5
    };
    let detune = f2.and_then(|f2| {
        let hz = key_hz * (libm::pow(2.0, (f2 - f1) / 1200.0) - 1.0).abs();
        ((0.15..=25.0).contains(&hz) && steady_ms >= 500.0).then_some(Beating {
            beating_hz: hz,
            depth_db: 12.0,
            harmonics_used: 6,
            prominence: 20.0,
        })
    });
    let raw_centroid = 12.0 * f0;
    let centroid = match get("filter1.cutoff") {
        Some(c) => 1.0 / (1.0 / raw_centroid + 1.0 / (1.5 * c)),
        None => raw_centroid,
    };
    let levels = get("osc1.level").unwrap_or(100.0) + get("osc2.level").unwrap_or(0.0);
    let peak = get("master.level").unwrap_or(0.0) + db(0.5 * levels / 100.0);
    let cents_dev = 1200.0 * libm::log2(f0 / key_hz);
    Descriptors {
        dur_s: job.perf.duration_s,
        silent: false,
        pitch: Some(Pitch {
            f0_hz: f0,
            note: "A4".into(),
            cents_dev,
            stability: 1.0,
            voiced: 1.0,
        }),
        envelope: Some(Envelope {
            attack_ms: libm::round(attack.max(1.0)),
            decay_ms: Some(libm::round(decay * 0.5 * (1.0 - sustain))),
            sustain_db: Some(sustain_db),
            release_ms: released.then(|| libm::round(release * 2.0 / 3.0)),
            release_extrapolated: false,
            duration_ms: duration,
            duration_truncated: truncated,
            ripple_db: 0.5,
            beat_corrected: true,
            beat_periods: detune.map(|b| b.beating_hz * hold_ms / 1000.0),
        }),
        spectrum: Some(Spectrum {
            centroid_hz: centroid,
            centroid_track_hz: None,
        }),
        noise_ratio: Some(0.01),
        detune,
        modulation: ModulationDesc::default(),
        loudness: Level {
            lufs_i: Some(peak - 10.0),
            peak_dbfs: peak,
            true_peak_dbtp: peak + 0.2,
            clipped_pct: if peak >= 0.0 { 1.0 } else { 0.0 },
            clipping: peak >= 0.0,
        },
    }
}
