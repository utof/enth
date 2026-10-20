//! **Owner: lane SS** (the sound scorer).
//!
//! **`sound/1`** ([`SoundV1`]): how close the player's audio is to the target's, the score that decides pass
//! (report 02 §2.3–2.4; ADR-0007's 2026-10-20 note, fixes 1–5). The reference is docs/challenge.md, "Sound score
//! (lane SS)".
//!
//! Once per challenge ([`Scorer::prepare`]):
//!
//! 1. render the target with the challenge's phrase at [`SCORING_SAMPLE_RATE`] and [`SCORING_SEED`], take the mid
//!    channel, bring it to −23 LUFS with the in-house `libm` loudness ([`crate::loudness`]), and extract metric
//!    v1.1's pooled log-mel ([`enth_analysis::pooled`], scalar FFT) and the descriptor readings behind the components;
//! 2. **the nuisance floor:** the same at [`FLOOR_RENDERS`] other render seeds (other free-running start phases and
//!    noise); the floor is the largest of their distances to the target, and the guard's floors the largest change
//!    of the pitch and odd-share readings;
//! 3. **the JND scale:** the median distance of [`crate::calibrate::jitter_set`]'s copies (every observable
//!    parameter one calibration step away).
//!
//! Per submit ([`Scorer::score`]):
//!
//! 1. **map the player's modules onto the target's ids** ([`SoundPrepared::scoring_patch`]: lane ST's round
//!    matcher, the pairing the structure score and the rings use), so start phases and noise, which
//!    follow module ids, match, and give every nuisance parameter (phase modes, start phases, seeds) the target's
//!    value; then render and analyse as above;
//! 2. `distance` = metric v1.1 to the target; `jnd` = `max(0, distance − floor) / scale` plus the **pitch and
//!    waveform guard** ([`guard`]: the pitch difference in units of 25 ct and the odd-share difference in units of
//!    0.25, each net of the target's own floor), because the metric barely sees an octave or a waveform once both
//!    clips are loudness-matched; `percent` = [`Curve::SOUND_V1`] of `jnd`;
//! 3. **components:** one row per descriptor both or either clip has ([`ComponentKind::ALL`] order), each with a
//!    direction and a close / a bit / far badge ([`TOLERANCES`]); [`component_text`] says it in words.
//!
//! **Metric v1.1, not v1.** Report 02 measured v1.1's floor 2–20× lower with the same cutoff sensitivity and a
//! slightly better fit to the owner's session 1. On 40 generated targets (levels 1–5) v1's floor was a median 0.31
//! of its JND scale and above it on 13 of the 40 (a three-step cutoff move then scores as nothing), v1.1's 0.06
//! (docs/challenge.md). Both are deterministic on the scalar FFT path; the in-house loudness removes `ebur128`'s
//! platform-dependent gain, so a `sound/1` score is the same bits on every platform (a test pins it natively and
//! under wasmtime).
//!
//! **The dispatch hook** ([`prepare`], F1r review S5): `score::round` asks this file for the sound scorer a challenge
//! names, after the stub (`sound-stub/1`).

use enth_analysis::descriptors::{Describer, Descriptors, NoteTiming, Want};
use enth_analysis::features::{FeatureCfg, FeatureExtractor};
use enth_analysis::fft::{FftPath, FftPlanner};
use enth_analysis::harmonics;
use enth_analysis::pooled::{self, Pooled};
use enth_dsp::util::rng::{derive, fnv1a64};
use enth_patch::addr::ParamAddr;
use enth_patch::{Op, Patch};
use enth_render::{RenderSpec, render};

use super::matching::{self, Matcher};
use super::round::{PreparedScorer, Ready};
use super::{
    Closeness, Component, ComponentKind, Direction, ScoreError, Scorer, ScorerId, SoundScore,
    TargetContext,
};
use crate::calibrate::{self, Curve};
use crate::loudness;
use crate::model::{Phrase, SCORING_SAMPLE_RATE, SCORING_SEED};
use crate::random::{self, Visibility};

/// The distance `sound/1` uses: metric v1.1, `logmel-pool85-l1/1`.
pub const METRIC: &str = pooled::METRIC_V1_1;

/// How many renders at other seeds measure the target's nuisance floor.
pub const FLOOR_RENDERS: u64 = 2;

/// The pitch guard's unit: this many cents of pitch difference (beyond the target's own pitch floor) add one JND to
/// `jnd`, so a pitch error alone fails from about 25 ct (`check`'s tolerance for "the same pitch").
pub const GUARD_PITCH_CT: f64 = 25.0;

/// The waveform guard's unit: this much difference in the odd share of the harmonic energy (beyond the target's own
/// floor) adds one JND to `jnd`: a saw against a square (0.35 against 1) is about 2.6 JND. The odd share jumps near a
/// 50 % pulse (a 45 % pulse reads about 0.54), so the unit is wide.
pub const GUARD_WAVE: f64 = 0.25;

/// The render seed of nuisance render `k` (1-based): `derive(SCORING_SEED, fnv1a64("sound/1 floor") + k)`.
pub fn floor_seed(k: u64) -> u64 {
    derive(SCORING_SEED, fnv1a64(b"sound/1 floor").wrapping_add(k))
}

/// The sound scorer `id`, prepared on `ctx`; `None` when this file has no scorer by that id (the caller refuses it,
/// `unknown_scorer`).
pub(crate) fn prepare(
    id: &ScorerId,
    ctx: &TargetContext<'_>,
) -> Option<Result<Box<dyn PreparedScorer<SoundScore>>, ScoreError>> {
    if *id == ScorerId::SOUND_V1 {
        return Some(Ready::prepare(SoundV1, ctx));
    }
    None
}

/// `sound/1`: see the [module docs](self).
#[derive(Clone, Copy, Debug, Default)]
pub struct SoundV1;

/// What `sound/1` keeps from the target.
#[derive(Clone, Debug)]
pub struct SoundPrepared {
    phrase: Phrase,
    /// The round's module pairing (lane ST's [`Matcher`]), the same the structure score and the rings use.
    matcher: Matcher,
    clip: Clip,
    floor: f64,
    scale: f64,
    pitch_floor: f64,
    wave_floor: f64,
    /// The target's nuisance parameters (lane RG's mask) and their values, in address order.
    nuisance: Vec<(ParamAddr, f32)>,
}

impl SoundPrepared {
    /// The answer as `sound/1` renders it: its modules paired with the target's by the round's [`Matcher`] and
    /// renamed onto the target's ids, sections and routes in the target's order ([`Matcher::relabel`]).
    pub fn scoring_patch(&self, answer: &Patch) -> Patch {
        self.matcher.relabel(&self.matcher.pair(answer), answer)
    }

    /// The target's nuisance floor (metric v1.1).
    pub fn floor(&self) -> f64 {
        self.floor
    }

    /// The per-challenge JND scale (metric v1.1).
    pub fn scale(&self) -> f64 {
        self.scale
    }

    /// The target's analysed clip.
    pub fn target_clip(&self) -> &Clip {
        &self.clip
    }

    /// The target's own pitch and waveform floors: the largest change of its pitch reading (cents) and of its odd
    /// harmonic share over the nuisance renders.
    pub fn guard_floors(&self) -> (f64, f64) {
        (self.pitch_floor, self.wave_floor)
    }

    /// The pitch and waveform guard of an answer's readings, in JND units (see [`guard`]).
    pub fn guard(&self, answer: &Readings) -> f64 {
        guard(
            &self.clip.readings,
            answer,
            self.pitch_floor,
            self.wave_floor,
        )
    }
}

/// The pitch and waveform guard, in JND units: `max(0, |Δpitch| − pitch_floor) / GUARD_PITCH_CT + max(0, |Δodd share|
/// − wave_floor) / GUARD_WAVE`, each term only when both clips have the reading. Metric v1.1 barely sees an octave
/// or a waveform through a filter once both clips are loudness-matched (an octave-down saw is 1.7 JND, a 42 % pulse
/// against a saw 0.9, so the start of a level-1 round could pass), and level 1 is exactly waveform plus octave; the
/// guard puts the two readings the components already measure into `jnd`. Lane SS's call, measured in
/// docs/challenge.md ("Sound score (lane SS)"); its units are provisional like the curve.
pub fn guard(target: &Readings, answer: &Readings, pitch_floor: f64, wave_floor: f64) -> f64 {
    let term = |t: Option<f64>, a: Option<f64>, floor: f64, unit: f64| match (t, a) {
        (Some(t), Some(a)) => ((a - t).abs() - floor).max(0.0) / unit,
        _ => 0.0,
    };
    term(target.pitch, answer.pitch, pitch_floor, GUARD_PITCH_CT)
        + term(target.harmonics, answer.harmonics, wave_floor, GUARD_WAVE)
}

/// The largest absolute change of a reading between the target and a nuisance render (0 when either lacks it).
fn spread(t: Option<f64>, o: Option<f64>) -> f64 {
    match (t, o) {
        (Some(t), Some(o)) => (o - t).abs(),
        _ => 0.0,
    }
}

impl Scorer for SoundV1 {
    type Prepared = SoundPrepared;
    type Score = SoundScore;

    fn id(&self) -> ScorerId {
        ScorerId::SOUND_V1
    }

    fn prepare(&self, ctx: &TargetContext<'_>) -> Result<SoundPrepared, ScoreError> {
        let phrase = *ctx.phrase;
        let nuisance = nuisance_of(ctx.target, &phrase);
        let mut an = Analyser::new();
        let clip = an.clip(
            &render_mid(ctx.target, &phrase, SCORING_SEED)?,
            &phrase,
            true,
        )?;
        if clip.pooled.silent {
            return Ok(SoundPrepared {
                phrase,
                matcher: Matcher::for_round(ctx),
                clip,
                floor: 0.0,
                scale: calibrate::SCALE_MIN,
                pitch_floor: 0.0,
                wave_floor: 0.0,
                nuisance,
            });
        }
        let (mut floor, mut pitch_floor, mut wave_floor) = (0.0f64, 0.0f64, 0.0f64);
        for k in 1..=FLOOR_RENDERS {
            let other = an.clip(
                &render_mid(ctx.target, &phrase, floor_seed(k))?,
                &phrase,
                true,
            )?;
            floor = floor.max(distance(&clip.pooled, &other.pooled));
            pitch_floor = pitch_floor.max(spread(clip.readings.pitch, other.readings.pitch));
            wave_floor = wave_floor.max(spread(clip.readings.harmonics, other.readings.harmonics));
        }
        let mut jitter = Vec::with_capacity(calibrate::JITTERS);
        for p in calibrate::jitter_set(ctx.target, &phrase) {
            let c = an.clip(&render_mid(&p, &phrase, SCORING_SEED)?, &phrase, false)?;
            jitter.push(distance(&clip.pooled, &c.pooled));
        }
        Ok(SoundPrepared {
            phrase,
            matcher: Matcher::for_round(ctx),
            clip,
            floor,
            scale: calibrate::scale(&jitter),
            pitch_floor,
            wave_floor,
            nuisance,
        })
    }

    fn score(&self, prepared: &SoundPrepared, answer: &Patch) -> Result<SoundScore, ScoreError> {
        let mapped = with_nuisance(prepared.scoring_patch(answer), &prepared.nuisance);
        let mut an = Analyser::new();
        let clip = an.clip(
            &render_mid(&mapped, &prepared.phrase, SCORING_SEED)?,
            &prepared.phrase,
            true,
        )?;
        let distance = distance(&prepared.clip.pooled, &clip.pooled);
        let jnd = calibrate::jnd(distance, prepared.floor, prepared.scale)
            + prepared.guard(&clip.readings);
        Ok(SoundScore {
            scorer: ScorerId::SOUND_V1,
            percent: Curve::SOUND_V1.percent(jnd),
            distance,
            floor: prepared.floor,
            scale: prepared.scale,
            jnd,
            components: components(&prepared.clip.readings, &clip.readings),
        })
    }
}

/// Metric v1.1 between two clips, 0 when both are silent (the silent target's only answer).
fn distance(a: &Pooled, b: &Pooled) -> f64 {
    if a.silent && b.silent {
        0.0
    } else {
        pooled::metric_v1_1(a, b)
    }
}

/// Render `patch` playing `phrase` at the scoring rate and `seed`, and take the mid channel.
fn render_mid(patch: &Patch, phrase: &Phrase, seed: u64) -> Result<Vec<f32>, ScoreError> {
    render(
        patch,
        &phrase.performance(),
        &RenderSpec::new(SCORING_SAMPLE_RATE, seed),
    )
    .map(|out| out.mid())
    .map_err(|e| ScoreError::Render {
        message: e.to_string(),
    })
}

// ---------------------------------------------------------------------------------------------------------------
// Module mapping
// ---------------------------------------------------------------------------------------------------------------

/// The answer with its modules renamed onto the target's ids, as `sound/1` renders it (ADR-0007's note, fix 2):
/// start phases and noise follow module ids (design.md §5's seed rule), so an equivalent patch under other ids would
/// otherwise score as different.
///
/// The pairing is lane ST's matcher ([`super::matching`]): present modules pair per kind by content (disabled
/// modules, silent generators and unrouted modulators are absent and keep their ids unless a partner took one), and
/// [`Matcher::relabel`] renames the pairs onto the target's ids and puts generators, modulators and routes in the
/// target's order, so an answer equal to the target up to ids and order renders the target's samples bit for bit.
/// Only ids and order change: the render plays the player's parameters. This context-free form pairs as
/// [`matching::match_modules`] does (held phrase, nothing randomized); the scorer itself uses the round's matcher
/// ([`SoundPrepared::scoring_patch`]).
pub fn scoring_patch(target: &Patch, answer: &Patch) -> Patch {
    let held =
        crate::phrases::phrase(crate::phrases::HELD).expect("the held phrase is in the table");
    let matcher = Matcher::new(target, held, &matching::Draws::default());
    matcher.relabel(&matcher.pair(answer), answer)
}

/// The target's nuisance parameters (phase modes, start phases, seeds: lane RG's [`Visibility::Nuisance`]) with
/// their values.
fn nuisance_of(target: &Patch, phrase: &Phrase) -> Vec<(ParamAddr, f32)> {
    random::observable(target, phrase)
        .params()
        .filter(|(_, v)| *v == Visibility::Nuisance)
        .filter_map(|(a, _)| Some((a.clone(), target.value(a)?)))
        .collect()
}

/// `p` (already on the target's ids) with the target's value at each nuisance address it has. A nuisance parameter
/// changes the samples, not the sound (design.md §9), and the generator never draws one, so it is not part of the
/// challenge: an LFO retriggered at another start phase moved a level-4 score to 88 % before this (P10 asks for at
/// least 99). The player still hears their own value; the scored render uses the target's.
fn with_nuisance(mut p: Patch, nuisance: &[(ParamAddr, f32)]) -> Patch {
    for (addr, value) in nuisance {
        if p.value(addr).is_some_and(|v| v != *value) {
            let _ = enth_patch::apply(
                &mut p,
                Op::SetParam {
                    addr: addr.clone(),
                    value: *value,
                },
            );
        }
    }
    p
}

// ---------------------------------------------------------------------------------------------------------------
// Analysis
// ---------------------------------------------------------------------------------------------------------------

/// One analysed render: metric v1.1's pooled log-mel and the descriptor readings.
#[derive(Clone, Debug, PartialEq)]
pub struct Clip {
    /// The pooled log-mel after the −23 LUFS gain.
    pub pooled: Pooled,
    /// The mid channel's integrated loudness before the gain (the in-house meter); `None` when silent.
    pub lufs: Option<f64>,
    /// The component readings (empty for the floor and jitter renders).
    pub readings: Readings,
}

/// The descriptor readings of one clip, in each [`ComponentKind::unit`]; `None` where the clip has none (no pitch,
/// no movement, a silent clip, or not measured).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Readings {
    /// f0 in cents re A4 (440 Hz).
    pub pitch: Option<f64>,
    /// The spectral centroid, Hz.
    pub brightness: Option<f64>,
    /// The odd share of the harmonic energy above the fundamental, 0–1.
    pub harmonics: Option<f64>,
    /// Attack, ms.
    pub attack: Option<f64>,
    /// Decay to the sustain level, ms.
    pub envelope: Option<f64>,
    /// Sustain level, dB re the peak.
    pub sustain: Option<f64>,
    /// Release, ms.
    pub release: Option<f64>,
    /// The noise ratio, 0–1.
    pub noisiness: Option<f64>,
    /// The rate of the strongest movement (beating, tremolo or a filter LFO), Hz.
    pub movement: Option<f64>,
}

impl Readings {
    /// The reading of a kind.
    pub fn get(&self, kind: ComponentKind) -> Option<f64> {
        match kind {
            ComponentKind::Pitch => self.pitch,
            ComponentKind::Brightness => self.brightness,
            ComponentKind::Harmonics => self.harmonics,
            ComponentKind::Attack => self.attack,
            ComponentKind::Envelope => self.envelope,
            ComponentKind::Sustain => self.sustain,
            ComponentKind::Release => self.release,
            ComponentKind::Noisiness => self.noisiness,
            ComponentKind::Movement => self.movement,
            ComponentKind::Unknown => None,
        }
    }
}

/// The feature extractor, describer and FFT planner of one scoring call (scalar FFT path throughout).
struct Analyser {
    fx: FeatureExtractor,
    describer: Describer,
    planner: FftPlanner,
}

impl Analyser {
    fn new() -> Analyser {
        let sr = f64::from(SCORING_SAMPLE_RATE);
        Analyser {
            fx: FeatureExtractor::new(FeatureCfg {
                sample_rate: sr,
                target_lufs: loudness::SCORING_LUFS,
                fft: FftPath::Scalar,
                ..FeatureCfg::default()
            }),
            describer: Describer::with_path(sr, FftPath::Scalar),
            planner: FftPlanner::new(FftPath::Scalar),
        }
    }

    /// Bring `mid` to −23 LUFS with the in-house loudness and analyse it; the readings only `with_readings`.
    fn clip(
        &mut self,
        mid: &[f32],
        phrase: &Phrase,
        with_readings: bool,
    ) -> Result<Clip, ScoreError> {
        let Some(lufs) = loudness::integrated_lufs(mid, SCORING_SAMPLE_RATE) else {
            return Ok(Clip {
                pooled: Pooled {
                    logmel: Vec::new(),
                    silent: true,
                },
                lufs: None,
                readings: Readings::default(),
            });
        };
        let gain = loudness::db_to_amp(loudness::gain_db(lufs, loudness::SCORING_LUFS)) as f32;
        let x: Vec<f32> = mid.iter().map(|v| v * gain).collect();
        let features = self.fx.extract_unnormalised(&x);
        let readings = if with_readings {
            self.readings(&x, phrase)?
        } else {
            Readings::default()
        };
        Ok(Clip {
            pooled: pooled::pool(&features),
            lufs: Some(lufs),
            readings,
        })
    }

    /// The readings of the phrase's first note: from the start to the next note-on (or the end), with its note-off.
    fn readings(&mut self, x: &[f32], phrase: &Phrase) -> Result<Readings, ScoreError> {
        let sr = f64::from(SCORING_SAMPLE_RATE);
        let Some(first) = phrase.notes.first() else {
            return Ok(Readings::default());
        };
        let to_sample = |ms: u32| ((f64::from(ms) / 1000.0 * sr) as usize).min(x.len());
        let end = phrase
            .notes
            .iter()
            .skip(1)
            .map(|n| n.start_ms)
            .filter(|ms| *ms > first.start_ms)
            .min()
            .map_or(x.len(), to_sample);
        let on_s = f64::from(first.start_ms) / 1000.0;
        let off_s = f64::from(first.start_ms + first.hold_ms) / 1000.0;
        let seg = &x[..end];
        let want = Want {
            spectrum: true,
            noise: true,
            loudness: false,
        };
        let d = self
            .describer
            .describe_some(
                seg,
                NoteTiming {
                    on_s,
                    off_s: Some(off_s),
                },
                want,
            )
            .map_err(|e| ScoreError::Analysis {
                message: format!("{e:?}"),
            })?;
        let mut r = readings_of(&d);
        if let (Some(f0), Some(e)) = (f0_of(&d), d.envelope) {
            let held_from =
                to_sample(first.start_ms) + ((e.attack_ms.max(30.0) / 1000.0) * sr) as usize;
            let held_to = to_sample(first.start_ms + first.hold_ms).min(end);
            r.harmonics = harmonics::measure(&mut self.planner, seg, sr, f0, (held_from, held_to))
                .map(|h| h.odd_share);
        }
        Ok(r)
    }
}

fn f0_of(d: &Descriptors) -> Option<f64> {
    d.pitch
        .as_ref()
        .map(|p| p.f0_hz)
        .or(d.tonal.as_ref().map(|t| t.f0_hz))
}

/// The descriptors' readings for the components (harmonics apart).
fn readings_of(d: &Descriptors) -> Readings {
    if d.silent {
        return Readings::default();
    }
    let env = d.envelope;
    // Movement: the strongest of a detune beat, a tremolo (both in dB of level wobble) and a filter LFO (its swing
    // in octaves counted at 6 dB an octave, a filter's slope near its cutoff).
    let mut movement: Option<(f64, f64)> = None;
    let mut consider = |hz: f64, strength: f64| {
        if hz.is_finite() && hz > 0.0 && movement.is_none_or(|(_, s)| strength > s) {
            movement = Some((hz, strength));
        }
    };
    if let Some(b) = d.detune {
        consider(b.beating_hz, b.depth_db);
    }
    if let Some(t) = d.modulation.amp_lfo {
        consider(t.hz, t.depth_db);
    }
    if let Some(s) = d.modulation.filter_sweep
        && let Some(hz) = s.hz
    {
        consider(hz, 6.0 * s.depth_oct);
    }
    Readings {
        pitch: f0_of(d)
            .filter(|f| *f > 0.0)
            .map(|f| 1200.0 * libm::log2(f / 440.0)),
        brightness: d.spectrum.map(|s| s.centroid_hz).filter(|c| *c > 0.0),
        harmonics: None,
        attack: env.map(|e| e.attack_ms),
        envelope: env.and_then(|e| e.decay_ms),
        sustain: env.and_then(|e| e.sustain_db),
        release: env.and_then(|e| e.release_ms),
        noisiness: d.noise_ratio,
        movement: movement.map(|(hz, _)| hz),
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Components
// ---------------------------------------------------------------------------------------------------------------

/// How a component's tolerance is measured: one "step" is one tolerance.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Tolerance {
    /// An absolute difference in the kind's unit.
    Abs(f64),
    /// A ratio of `(reading + floor)`: `|ln((a + floor) / (t + floor))| / ln(factor)` steps.
    Ratio {
        /// One tolerance (1.1 for 10 %).
        factor: f64,
        /// Added to both readings first.
        floor: f64,
    },
}

impl Tolerance {
    /// How many tolerances `answer` is from `target`.
    pub fn steps(self, target: f64, answer: f64) -> f64 {
        match self {
            Tolerance::Abs(t) => (answer - target).abs() / t,
            Tolerance::Ratio { factor, floor } => {
                let (t, a) = (target + floor, answer + floor);
                if t > 0.0 && a > 0.0 {
                    libm::fabs(libm::log(a / t)) / libm::log(factor)
                } else if t == a {
                    0.0
                } else {
                    f64::INFINITY
                }
            }
        }
    }
}

/// Each kind's tolerance, in [`ComponentKind::ALL`] order: pitch 25 ct; brightness and movement 10 %; harmonics 0.1;
/// attack 25 % over a 5 ms floor (the descriptor resolves about 5 ms at C3); decay and release 10 % over 10 ms;
/// sustain 2 dB; noisiness 0.05. A reading within one tolerance is `close` and `same`, within three `a_bit`, beyond
/// that `far`.
pub const TOLERANCES: [(ComponentKind, Tolerance); 9] = [
    (ComponentKind::Pitch, Tolerance::Abs(25.0)),
    (
        ComponentKind::Brightness,
        Tolerance::Ratio {
            factor: 1.1,
            floor: 0.0,
        },
    ),
    (ComponentKind::Harmonics, Tolerance::Abs(0.1)),
    (
        ComponentKind::Attack,
        Tolerance::Ratio {
            factor: 1.25,
            floor: 5.0,
        },
    ),
    (
        ComponentKind::Envelope,
        Tolerance::Ratio {
            factor: 1.1,
            floor: 10.0,
        },
    ),
    (ComponentKind::Sustain, Tolerance::Abs(2.0)),
    (
        ComponentKind::Release,
        Tolerance::Ratio {
            factor: 1.1,
            floor: 10.0,
        },
    ),
    (ComponentKind::Noisiness, Tolerance::Abs(0.05)),
    (
        ComponentKind::Movement,
        Tolerance::Ratio {
            factor: 1.1,
            floor: 0.0,
        },
    ),
];

/// Within this many tolerances a reading is `a_bit` rather than `far`.
const A_BIT_STEPS: f64 = 3.0;

/// One component row from the two readings; `None` when neither clip has one.
pub fn component(
    kind: ComponentKind,
    target: Option<f64>,
    answer: Option<f64>,
) -> Option<Component> {
    let tol = TOLERANCES.iter().find(|(k, _)| *k == kind)?.1;
    let (direction, closeness) = match (target, answer) {
        (None, None) => return None,
        (Some(_), None) => (Direction::Missing, Closeness::Far),
        (None, Some(_)) => (Direction::Extra, Closeness::Far),
        (Some(t), Some(a)) => {
            let s = tol.steps(t, a);
            let direction = if s < 1.0 {
                Direction::Same
            } else if a > t {
                Direction::Higher
            } else {
                Direction::Lower
            };
            let closeness = if s < 1.0 {
                Closeness::Close
            } else if s < A_BIT_STEPS {
                Closeness::ABit
            } else {
                Closeness::Far
            };
            (direction, closeness)
        }
    };
    Some(Component {
        kind,
        direction,
        closeness,
        target,
        answer,
    })
}

/// Every component row, in [`ComponentKind::ALL`] order.
pub fn components(target: &Readings, answer: &Readings) -> Vec<Component> {
    ComponentKind::ALL
        .iter()
        .filter_map(|k| component(*k, target.get(*k), answer.get(*k)))
        .collect()
}

/// A component in words, the player's sound relative to the target's ("brighter than the target", "attack slightly
/// too fast", "pitch matches the target"). For the results drawer and the CLI's `compare`; it never shows a reading,
/// so it is safe during a round.
pub fn component_text(c: &Component) -> String {
    let (name, higher, lower) = match c.kind {
        ComponentKind::Pitch => (
            "pitch",
            "{}higher than the target",
            "{}lower than the target",
        ),
        ComponentKind::Brightness => (
            "brightness",
            "{}brighter than the target",
            "{}darker than the target",
        ),
        ComponentKind::Harmonics => (
            "harmonics",
            "{}hollower than the target (more odd harmonics)",
            "{}buzzier than the target (more even harmonics)",
        ),
        ComponentKind::Attack => ("attack", "attack {}too slow", "attack {}too fast"),
        ComponentKind::Envelope => ("decay", "decay {}too long", "decay {}too short"),
        ComponentKind::Sustain => ("sustain", "sustain {}too loud", "sustain {}too quiet"),
        ComponentKind::Release => ("release", "release {}too long", "release {}too short"),
        ComponentKind::Noisiness => (
            "noise",
            "{}noisier than the target",
            "{}less noisy than the target",
        ),
        ComponentKind::Movement => ("movement", "movement {}too fast", "movement {}too slow"),
        ComponentKind::Unknown => (
            "this",
            "{}higher than the target",
            "{}lower than the target",
        ),
    };
    let adverb = match c.closeness {
        Closeness::ABit => "slightly ",
        Closeness::Far => "much ",
        _ => "",
    };
    match c.direction {
        Direction::Same => format!("{name} matches the target"),
        Direction::Higher => higher.replace("{}", adverb),
        Direction::Lower => lower.replace("{}", adverb),
        Direction::Missing => match c.kind {
            ComponentKind::Pitch => "no clear pitch; the target has one".to_owned(),
            ComponentKind::Harmonics => "fewer harmonics than the target".to_owned(),
            ComponentKind::Movement => {
                "no movement; the target beats, wobbles or sweeps".to_owned()
            }
            _ => format!("no {name} reading; the target has one"),
        },
        Direction::Extra => match c.kind {
            ComponentKind::Pitch => "a clear pitch the target lacks".to_owned(),
            ComponentKind::Harmonics => "harmonics the target lacks".to_owned(),
            ComponentKind::Movement => "movement the target lacks".to_owned(),
            _ => format!("a {name} reading the target lacks"),
        },
        Direction::Unknown => format!("{name}: no comparison"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn components_read_in_words() {
        let t = Readings {
            pitch: Some(-900.0),
            brightness: Some(1000.0),
            attack: Some(20.0),
            movement: Some(5.0),
            ..Readings::default()
        };
        let a = Readings {
            pitch: Some(-895.0),
            brightness: Some(1500.0),
            attack: Some(12.0),
            noisiness: Some(0.5),
            ..Readings::default()
        };
        let rows = components(&t, &a);
        let text: Vec<String> = rows.iter().map(component_text).collect();
        assert_eq!(
            text,
            [
                "pitch matches the target",
                "much brighter than the target",
                "attack slightly too fast",
                "a noise reading the target lacks",
                "no movement; the target beats, wobbles or sweeps",
            ]
        );
        assert_eq!(rows[1].direction, Direction::Higher);
        assert_eq!(rows[2].closeness, Closeness::ABit);
        assert_eq!(rows[4].direction, Direction::Missing);
        let kinds: Vec<ComponentKind> = rows.iter().map(|c| c.kind).collect();
        let mut sorted = kinds.clone();
        sorted.sort();
        assert_eq!(kinds, sorted, "ComponentKind::ALL order");
        assert!(TOLERANCES.iter().map(|(k, _)| *k).eq(ComponentKind::ALL));
    }

    #[test]
    fn floor_seeds_differ_from_the_scoring_seed() {
        assert_ne!(floor_seed(1), SCORING_SEED);
        assert_ne!(floor_seed(1), floor_seed(2));
    }

    #[test]
    fn tolerance_steps() {
        let r = Tolerance::Ratio {
            factor: 1.1,
            floor: 0.0,
        };
        assert!((r.steps(1000.0, 1100.0) - 1.0).abs() < 1e-12);
        assert_eq!(r.steps(0.0, 0.0), 0.0);
        assert_eq!(r.steps(0.0, 1.0), f64::INFINITY);
        assert_eq!(Tolerance::Abs(2.0).steps(-6.0, -10.0), 2.0);
    }
}
