//! **Owner: lane DEC.** What the planner is fed, measured: every element's ablation, each step in its context, the
//! sound items a Do step or the variation goal checks, and the options of "What disappeared?". Pure (renders through
//! `enth-render`, as the sound checks do, and no I/O, clock or thread), so `lesson-gen` and the plugin's worker get
//! the same lesson for the same preset.
//!
//! **Ablation.** One `sound/1` round is prepared per phrase on the full preset, as `enth compare --target preset
//! --phrase N` prepares it (the compared parameters are what the preset moved off its defaults, so every element
//! is measured on one scale), and the preset without each element is scored against it: its distance in
//! just-noticeable steps and the coach's descriptor rows. An element is measured through the first phrase that
//! reveals it ([`reveal_phrase`]): the held note, unless RG's observability mask says only a legato pair (glide, the
//! voice mode), an octave pair (key tracking, velocity), a chord or the long hold (a slow LFO) can.
//!
//! **Cost**, counted in renders by `sound/1`'s documented costs: [`PREPARE_RENDERS`] to prepare a round, one to score a
//! patch, one per description, three per nuisance-checked sound check.

use std::collections::BTreeMap;
use std::sync::Arc;

use enth_analysis::descriptors::Want;
use enth_analysis::{Describer, Descriptors, Feature, NoteTiming};
use enth_challenge::lesson_phrases::{CHORD, LONG_HOLD};
use enth_challenge::model::{SCORING_SAMPLE_RATE, SCORING_SEED};
use enth_challenge::patch_round::{Compared, prepare_patch_round};
use enth_challenge::phrases::{HELD, LEGATO_PAIR, OCTAVE_PAIR};
use enth_challenge::random::{Visibility, observable};
use enth_challenge::v2::InlinePatch;
use enth_challenge::{
    Challenge, PhraseId, PreparedRound, ScoringSpec, Start, Target, prepare_challenge, score_round,
};
use enth_dsp::util::math;
use enth_patch::catalogue::ModuleKind;
use enth_patch::{ModSource, Patch};
use enth_render::{RenderSpec, render};

use super::DeconstructError;
use super::element::{Element, Part, elements, project};
use super::plan::{Aspect, Change, Rank, Ranks, Way, change_of, merge_changes};
use crate::check::{
    Audibility, MIN_JND, PairJnd, SoundCheck, SoundOptions, choice_audibility,
    evaluate_sounds_report,
};
use crate::player::Audition;

/// Renders `sound/1` spends to prepare a round: the target, two floor renders and 16 jittered copies.
pub const PREPARE_RENDERS: u32 = 19;

/// An LFO slower than this is measured through the long hold first: a sweep is found only after two or more cycles
/// (agent-evals finding 8), and the held note holds 1.5 s.
pub const SLOW_LFO_HZ: f32 = 2.0;

/// The phrases tried, in order, for an element that is not an LFO route.
const REVEAL_ORDER: [PhraseId; 5] = [HELD, LEGATO_PAIR, OCTAVE_PAIR, CHORD, LONG_HOLD];

/// The smallest margin a sound item keeps against the start and the witness (docs/learn.md: "at least 1e-3").
pub const MIN_MARGIN: f64 = 1e-3;

/// Renders spent, by what they were for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct Renders {
    /// The ablations: one prepared round per phrase, one score per element.
    pub ablation: u32,
    /// The steps in their context: a prepared round and a score each, and a Do step's sound items.
    pub steps: u32,
    /// "What disappeared?": the candidates' rounds and their pairs.
    pub choice: u32,
    /// The variation goal: descriptions and nuisance-checked sound checks.
    pub variation: u32,
}

impl Renders {
    /// Every render.
    pub fn total(&self) -> u32 {
        self.ablation + self.steps + self.choice + self.variation
    }
}

/// The first phrase that reveals `element` in `preset`, or `None` when none does. A route from an LFO slower than
/// [`SLOW_LFO_HZ`] tries the long hold first.
pub fn reveal_phrase(preset: &Patch, element: &Element) -> Option<PhraseId> {
    let slow_lfo = element.parts.iter().any(|p| match p {
        Part::Route(r) => match &r.src {
            ModSource::Module(id) => preset.find(id).is_some_and(|(_, m)| {
                m.kind == ModuleKind::Lfo && m.get("rate").is_some_and(|hz| hz < SLOW_LFO_HZ)
            }),
            _ => false,
        },
        _ => false,
    });
    let order: Vec<PhraseId> = if slow_lfo {
        std::iter::once(LONG_HOLD)
            .chain(REVEAL_ORDER.into_iter().filter(|p| *p != LONG_HOLD))
            .collect()
    } else {
        REVEAL_ORDER.to_vec()
    };
    order.into_iter().find(|id| {
        enth_challenge::phrases::phrase(*id).is_some_and(|phrase| {
            let mask = observable(preset, phrase);
            element.parts.iter().any(|p| match p {
                Part::Param(a) => mask.is_observable(a),
                Part::Route(r) => mask.route(r).is_some_and(Visibility::is_observable),
                Part::Enable(id) => mask.module(id).is_some_and(Visibility::is_observable),
            })
        })
    })
}

fn round_error(what: &str, e: impl std::fmt::Display) -> DeconstructError {
    DeconstructError::Round {
        message: format!("{what}: {e}"),
    }
}

/// The default scoring of a deconstruct round: this build's Recreate scorers, sound weighted 50 %, pass mark 90.
pub fn scoring() -> ScoringSpec {
    let mut s = ScoringSpec::recreate(0);
    s.sound_weight = 50;
    s.pass_mark = 90;
    s
}

/// The `enth compare --target preset --phrase N` round: the preset as the target, its base as the start, the
/// parameters it moved off their defaults compared.
fn preset_round(
    preset: &Patch,
    base: &Patch,
    phrase: PhraseId,
) -> Result<PreparedRound, DeconstructError> {
    let challenge = Challenge::new(
        Target::Patch(InlinePatch::new(preset.clone())),
        Start::Patch {
            patch: InlinePatch::new(base.clone()),
        },
        phrase,
        scoring(),
    );
    prepare_patch_round(challenge, Compared::OffDefaults)
        .map_err(|e| round_error("the preset's round", e))
}

/// The descriptors a heard change is read on, with the aspect and the tolerance (`sound/1`'s component tolerances:
/// 25 ct for pitch, 10 % for frequencies, decay and release over a 10 ms floor, 25 % for attacks over 5 ms, 2 dB for
/// the held level, 0.05 of noise, 0.2 octave of sweep depth).
const MOVES: [(Feature, Aspect, Tol); 10] = [
    (Feature::Pitch, Aspect::Pitch, Tol::Cents(25.0)),
    (
        Feature::Brightness,
        Aspect::Brightness,
        Tol::Ratio(1.1, 0.0),
    ),
    (Feature::Attack, Aspect::Attack, Tol::Ratio(1.25, 5.0)),
    (Feature::Decay, Aspect::Decay, Tol::Ratio(1.1, 10.0)),
    (Feature::Sustain, Aspect::Sustain, Tol::Abs(2.0)),
    (Feature::Release, Aspect::Release, Tol::Ratio(1.1, 10.0)),
    (Feature::Noisiness, Aspect::Noisiness, Tol::Abs(0.05)),
    (Feature::Beating, Aspect::Beating, Tol::Ratio(1.1, 0.0)),
    (Feature::Tremolo, Aspect::Tremolo, Tol::Ratio(1.1, 0.0)),
    (Feature::SweepDepth, Aspect::Sweep, Tol::Abs(0.2)),
];

/// A descriptor change counts when it moves at least this many tolerances.
pub const MIN_MOVE_STEPS: f64 = 2.0;

/// ... and at least this many times the reading's own nuisance noise (its largest change over `enth check`'s two
/// nuisance renders of the preset).
pub const NOISE_FACTOR: f64 = 3.0;

/// How `without` reads against `with`, descriptor by descriptor (see [`MOVES`]): a change for each that moved at
/// least [`MIN_MOVE_STEPS`] tolerances and [`NOISE_FACTOR`] times its noise over the `noise` renders of `with` (none:
/// no noise), and for each beat, tremolo, sweep or pitch that `with` has and `without` lacks (or the other way), when
/// the noise renders agree on its presence.
pub fn descriptor_changes(
    with: &Descriptors,
    without: &Descriptors,
    noise: &[Descriptors],
) -> Vec<Change> {
    let mut out = Vec::new();
    for (f, aspect, tol) in MOVES {
        let stable = noise
            .iter()
            .all(|n| n.get(f).is_some() == with.get(f).is_some());
        match (with.get(f), without.get(f)) {
            (Some(a), Some(b)) => {
                let (Some(da), Some(db)) = (tol.domain(a), tol.domain(b)) else {
                    continue;
                };
                let steps = (db - da).abs() / tol.unit();
                let noise_d = noise
                    .iter()
                    .filter_map(|n| n.get(f).and_then(|v| tol.domain(v)))
                    .map(|dn| (dn - da).abs())
                    .fold(0.0, f64::max);
                if stable && steps >= MIN_MOVE_STEPS && (db - da).abs() > NOISE_FACTOR * noise_d {
                    out.push(Change {
                        aspect,
                        way: if b > a { Way::Higher } else { Way::Lower },
                        steps,
                        with: Some(a),
                        without: Some(b),
                    });
                }
            }
            (Some(a), None) if stable => out.push(Change {
                aspect,
                way: Way::Gone,
                steps: 4.0,
                with: Some(a),
                without: None,
            }),
            (None, Some(b)) if stable => out.push(Change {
                aspect,
                way: Way::New,
                steps: 4.0,
                with: None,
                without: Some(b),
            }),
            _ => {}
        }
    }
    out
}

/// Describe `patch` through `phrase` at the scoring seed and at `enth check`'s two nuisance seeds.
fn describe_with_noise(
    patch: &Patch,
    phrase: PhraseId,
) -> Result<(Descriptors, Vec<Descriptors>), DeconstructError> {
    let main = describe_at(patch, phrase, SCORING_SEED)?;
    let noise = crate::check::NUISANCE_OFFSETS
        .iter()
        .map(|o| describe_at(patch, phrase, SCORING_SEED.wrapping_add(*o)))
        .collect::<Result<_, _>>()?;
    Ok((main, noise))
}

/// Every element's [`Rank`] (see the [module docs](self)), and the renders spent: per phrase, the preset's round and
/// its three descriptions; per element, one score and one description.
///
/// # Errors
/// [`DeconstructError::Round`] when the preset's round cannot be prepared or a patch cannot be scored, and
/// [`DeconstructError::Sound`] when a description fails.
pub fn measure_ablations(preset: &Patch) -> Result<(Ranks, u32), DeconstructError> {
    let els = elements(preset);
    let base = project(preset, &els, &std::collections::BTreeSet::default());
    let mut heard: BTreeMap<u16, (PreparedRound, Descriptors, Vec<Descriptors>)> = BTreeMap::new();
    let mut ranks = Ranks::new();
    let mut renders = 0;
    for (i, e) in els.iter().enumerate() {
        let Some(phrase) = reveal_phrase(preset, e) else {
            ranks.insert(e.id.clone(), Rank::unobservable());
            continue;
        };
        if let std::collections::btree_map::Entry::Vacant(slot) = heard.entry(phrase.0) {
            let round = preset_round(preset, &base, phrase)?;
            let (desc, noise) = describe_with_noise(preset, phrase)?;
            slot.insert((round, desc, noise));
            renders += PREPARE_RENDERS + 3;
        }
        let (round, desc, noise) = &heard[&phrase.0];
        let without = project(preset, &els, &(0..els.len()).filter(|j| *j != i).collect());
        let score = score_round(round, &without)
            .map_err(|err| round_error(&format!("the preset without {}", e.id), err))?;
        let without_desc = describe(&without, phrase)?;
        renders += 2;
        let rows = harmonics_rows(&score.sound.components);
        ranks.insert(
            e.id.clone(),
            Rank {
                phrase: Some(phrase),
                jnd: score.sound.jnd,
                percent: score.sound.percent,
                changes: merge_changes(rows, descriptor_changes(desc, &without_desc, noise)),
            },
        );
    }
    Ok((ranks, renders))
}

/// How a step's round heard its start: `sound/1` of the start against the step's target, in its own round, and the
/// descriptions of both.
#[derive(Clone, Debug, PartialEq)]
pub struct StepMeasure {
    /// The start's sound %.
    pub percent: f64,
    /// The start's distance in just-noticeable steps.
    pub jnd: f64,
    /// The heard changes, start ("without") against target ("with").
    pub changes: Vec<Change>,
    /// The start's description through the step's phrase.
    pub start: Descriptors,
    /// The target's.
    pub target: Descriptors,
}

/// Renders [`measure_step`] spends: the round, the start's score, the start's description and the target's three.
pub const STEP_RENDERS: u32 = PREPARE_RENDERS + 5;

/// `sound/1`'s harmonic-content rows that moved far (or appeared, or went): the one aspect no description reads.
/// Its other rows read the same descriptors as [`descriptor_changes`], but with no nuisance guard, so they are left
/// to it (a 3-tolerance "decay" row on a ten-second noise was noise).
fn harmonics_rows(rows: &[enth_challenge::Component]) -> Vec<Change> {
    rows.iter()
        .filter(|c| c.kind == enth_challenge::ComponentKind::Harmonics)
        .filter_map(change_of)
        .filter(|c| c.steps >= 3.0)
        .collect()
}

/// Score `start` in the round `challenge` prepares (a step's real round: its patch target, its start, the parameters
/// the target changes compared), and describe both through its phrase.
///
/// # Errors
/// [`DeconstructError::Round`] when the round cannot be prepared or the start cannot be scored;
/// [`DeconstructError::Sound`] when a description fails.
pub fn measure_step(
    challenge: &Challenge,
    start: &Patch,
    target: &Patch,
) -> Result<StepMeasure, DeconstructError> {
    let round =
        prepare_challenge(challenge.clone()).map_err(|e| round_error("a step's round", e))?;
    let score = score_round(&round, start).map_err(|e| round_error("a step's start", e))?;
    let s = describe(start, challenge.phrase)?;
    let (t, noise) = describe_with_noise(target, challenge.phrase)?;
    Ok(StepMeasure {
        percent: score.sound.percent,
        jnd: score.sound.jnd,
        changes: merge_changes(
            harmonics_rows(&score.sound.components),
            descriptor_changes(&t, &s, &noise),
        ),
        start: s,
        target: t,
    })
}

/// Describe `patch` through `phrase` with every descriptor group (one render): the steps `enth check --phrase N`
/// takes (render at the scoring rate and seed, the mid channel, the first note's timing).
///
/// # Errors
/// [`DeconstructError::Sound`] when it does not render or the analysis refuses it.
pub fn describe(patch: &Patch, phrase: PhraseId) -> Result<Descriptors, DeconstructError> {
    describe_at(patch, phrase, SCORING_SEED)
}

/// [`describe`] at another render seed (a nuisance render).
///
/// # Errors
/// As [`describe`].
pub fn describe_at(
    patch: &Patch,
    phrase: PhraseId,
    seed: u64,
) -> Result<Descriptors, DeconstructError> {
    use crate::check::SoundError;
    let ph = enth_challenge::phrases::phrase(phrase).ok_or(DeconstructError::Sound(
        SoundError::UnknownPhrase { phrase: phrase.0 },
    ))?;
    let perf = ph.performance();
    let out = render(patch, &perf, &RenderSpec::new(SCORING_SAMPLE_RATE, seed)).map_err(|e| {
        DeconstructError::Sound(SoundError::Render {
            code: e.code(),
            message: e.to_string(),
        })
    })?;
    let first = perf
        .notes
        .iter()
        .enumerate()
        .min_by(|(i, a), (j, b)| a.on_s.total_cmp(&b.on_s).then(i.cmp(j)))
        .map(|(_, n)| n);
    let timing = match first {
        Some(n) => NoteTiming {
            on_s: n.on_s,
            off_s: Some(n.off_s),
        },
        None => NoteTiming::unknown_off(),
    };
    Describer::new(f64::from(SCORING_SAMPLE_RATE))
        .describe_some(&out.mid(), timing, Want::ALL)
        .map_err(|e| {
            DeconstructError::Sound(SoundError::Analysis {
                message: e.to_string(),
            })
        })
}

/// The descriptors a sound item may test, with the tolerance below which a difference is not proposed: the ratio
/// (over a floor, in the feature's unit) for times and frequencies, the step for levels and ratios. Two tolerances
/// apart is the least a proposed item needs, so its threshold sits a tolerance from each side.
const ITEM_FEATURES: [(Feature, Tol); 9] = [
    (Feature::Brightness, Tol::Ratio(1.1, 0.0)),
    (Feature::Attack, Tol::Ratio(1.25, 5.0)),
    (Feature::Decay, Tol::Ratio(1.1, 10.0)),
    (Feature::Sustain, Tol::Abs(2.0)),
    (Feature::Release, Tol::Ratio(1.1, 10.0)),
    (Feature::Noisiness, Tol::Abs(0.05)),
    (Feature::Beating, Tol::Ratio(1.1, 0.0)),
    (Feature::Tremolo, Tol::Ratio(1.1, 0.0)),
    (Feature::SweepDepth, Tol::Abs(0.2)),
];

#[derive(Clone, Copy)]
enum Tol {
    Ratio(f64, f64),
    Abs(f64),
    Cents(f64),
}

impl Tol {
    /// A reading in the tolerance's domain: the log over the floor for a ratio, cents for a pitch.
    fn domain(self, v: f64) -> Option<f64> {
        match self {
            Tol::Ratio(_, floor) => (v + floor > 0.0).then(|| math::log(v + floor)),
            Tol::Abs(_) => Some(v),
            Tol::Cents(_) => (v > 0.0).then(|| 1200.0 * math::log2(v)),
        }
    }

    /// One tolerance in the domain.
    fn unit(self) -> f64 {
        match self {
            Tol::Ratio(factor, _) => math::log(factor),
            Tol::Abs(step) | Tol::Cents(step) => step,
        }
    }

    /// How many tolerances apart `a` and `b` are, and the threshold halfway between them in the tolerance's scale.
    fn split(self, a: f64, b: f64) -> (f64, f64) {
        match self {
            Tol::Ratio(factor, floor) => {
                let (x, y) = (a + floor, b + floor);
                if x <= 0.0 || y <= 0.0 {
                    return (0.0, 0.0);
                }
                let steps = math::log(y / x).abs() / math::log(factor);
                (steps, (x * y).sqrt() - floor)
            }
            Tol::Abs(step) | Tol::Cents(step) => ((b - a).abs() / step, f64::midpoint(a, b)),
        }
    }
}

/// The unit text of a feature's threshold in an expectation item.
fn unit_text(f: Feature) -> &'static str {
    match f {
        Feature::Brightness | Feature::Beating | Feature::Tremolo => "Hz",
        Feature::Attack | Feature::Decay | Feature::Release => "ms",
        Feature::Sustain => "dB",
        Feature::SweepDepth => "oct",
        _ => "",
    }
}

/// `v` with three significant digits (at least one decimal for small values), as an item's threshold.
fn round3(v: f64) -> String {
    let a = v.abs();
    let decimals = if a >= 100.0 {
        0
    } else if a >= 10.0 {
        1
    } else if a >= 1.0 {
        2
    } else {
        3
    };
    let s = format!("{v:.decimals$}");
    if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.').to_owned()
    } else {
        s
    }
}

/// Candidate items that tell `to` from `from`: for each descriptor both have and that moved at least two tolerances,
/// `feature>t` or `feature<t` with `t` halfway; for a movement `to` has and `from` lacks, `feature>t` at half its
/// reading. The furthest moves first.
pub fn propose_items(from: &Descriptors, to: &Descriptors) -> Vec<String> {
    let mut out: Vec<(f64, String)> = Vec::new();
    for (f, tol) in ITEM_FEATURES {
        let name = f.aliases()[0];
        let unit = unit_text(f);
        match (from.get(f), to.get(f)) {
            (Some(a), Some(b)) => {
                let (steps, t) = tol.split(a, b);
                if steps >= 2.0 {
                    let op = if b > a { '>' } else { '<' };
                    out.push((steps, format!("{name}{op}{}{unit}", round3(t))));
                }
            }
            (None, Some(b))
                if matches!(f, Feature::Beating | Feature::Tremolo | Feature::SweepDepth) =>
            {
                out.push((8.0, format!("{name}>{}{unit}", round3(0.5 * b))));
            }
            _ => {}
        }
    }
    out.sort_by(|x, y| y.0.total_cmp(&x.0));
    out.into_iter().map(|(_, s)| s).collect()
}

/// Of `items` heard through `phrase`, the ones `witness` passes and `start` fails, each by at least [`MIN_MARGIN`] and
/// never thin under `enth check`'s nuisance renders (lesson-check's margin rule); at most `max`. And the renders spent.
///
/// # Errors
/// [`DeconstructError::Sound`] when a patch does not render or an item does not parse.
pub fn verify_items(
    items: &[String],
    phrase: PhraseId,
    start: &Patch,
    witness: &Patch,
    max: usize,
) -> Result<(Vec<String>, u32), DeconstructError> {
    if items.is_empty() {
        return Ok((Vec::new(), 0));
    }
    let check = SoundCheck {
        phrase,
        expect: items.to_vec(),
        macros: Vec::new(),
    };
    let opts = SoundOptions { nuisance: true };
    let s = evaluate_sounds_report(std::slice::from_ref(&check), start, opts)
        .map_err(DeconstructError::Sound)?;
    let w = evaluate_sounds_report(std::slice::from_ref(&check), witness, opts)
        .map_err(DeconstructError::Sound)?;
    let renders = 6;
    let ok = |r: &enth_analysis::expect::ItemResult, pass: bool| {
        r.pass == pass && !r.thin && !r.unstable && r.margin.is_some_and(|m| m.abs() >= MIN_MARGIN)
    };
    let kept = items
        .iter()
        .zip(s[0].report.results.iter().zip(&w[0].report.results))
        .filter(|(_, (rs, rw))| ok(rs, false) && ok(rw, true))
        .map(|(item, _)| item.clone())
        .take(max)
        .collect();
    Ok((kept, renders))
}

/// The options of "What disappeared?": from `candidates` (each an ablation with its label, in preference order), the
/// largest set of at most `max` that are pairwise at least [`MIN_JND`] apart under `sound/1` (the audibility gate),
/// picked greedily in order. Returns the chosen indices into `candidates`, the closest chosen pair's distance, and the
/// renders spent.
///
/// # Errors
/// [`DeconstructError::Sound`] when a candidate cannot be scored.
pub fn pick_options(
    candidates: &[(String, Patch)],
    phrase: PhraseId,
    max: usize,
) -> Result<(Vec<usize>, f64, u32), DeconstructError> {
    let auditions: Vec<Audition> = candidates
        .iter()
        .map(|(label, patch)| Audition {
            label: label.clone(),
            patch: Arc::new(patch.clone()),
            phrase,
            transpose: 0,
            level_match: true,
            seed: None,
            curve: None,
        })
        .collect();
    let a: Audibility = choice_audibility(&auditions).map_err(DeconstructError::Sound)?;
    let n = candidates.len() as u32;
    let renders = n * PREPARE_RENDERS + n * n.saturating_sub(1);
    let dist = |i: usize, j: usize| {
        let (x, y) = (i.min(j), i.max(j));
        a.pairs
            .iter()
            .find(|p| p.a == x && p.b == y)
            .and_then(|p| match p.jnd {
                PairJnd::Measured(d) => Some(d),
                PairJnd::NotMeasured(_) => None,
            })
            .unwrap_or(0.0)
    };
    let mut chosen: Vec<usize> = Vec::new();
    for i in 0..candidates.len() {
        if chosen.len() >= max {
            break;
        }
        if chosen.iter().all(|j| dist(i, *j) >= MIN_JND) {
            chosen.push(i);
        }
    }
    let closest = chosen
        .iter()
        .enumerate()
        .flat_map(|(n, i)| chosen[n + 1..].iter().map(move |j| (*i, *j)))
        .map(|(i, j)| dist(i, j))
        .fold(f64::INFINITY, f64::min);
    Ok((chosen, closest, renders))
}
