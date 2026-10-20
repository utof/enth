//! **Owner: lane SN** (sensitivity and coach). Pre-declared empty by lane S0r (R1/S1).
//!
//! The coach (report 02, summary "Coach and CLI"): the target's descriptors become an `Expect` list for
//! [`hint`](crate::hint()), **unchanged**, so a Recreate hint is the same measured move `enth check` suggests. A hint
//! marks the round as assisted (the session's `game` module records it).
//!
//! # The two steps
//!
//! 1. **[`CoachTarget::prepare`]**, once per round: render and describe the target, measure the
//!    [sensitivity](crate::sensitivity) of its randomized parameters (the same renders), and turn each descriptor of
//!    the target into one expectation, `target ± tolerance` ([`Item`]): pitch against the target's note, brightness,
//!    attack, decay, sustain level, release and noisiness as bands around the target's reading (in ln for Hz and ms,
//!    so the band is a ratio), and beating, tremolo and a filter sweep either as a band or, when the target has none,
//!    as `== null`. The tolerance of each is [`Sensitivity::tolerance`]: what the descriptor does when every
//!    randomized parameter is one just-noticeable step off, kept within [`tolerance_bounds`]. Loudness, peak and
//!    clipping are not in the list: the A/B switch matches the level and the sound score ignores it.
//! 2. **[`CoachTarget::plan`]**, per hint request: run [`hint`](crate::hint()) on the player's patch against that
//!    list, with the randomized parameters (and any other the player's patch has moved away from the target) as
//!    the only ones it may move, and turn the verified [`Fix`](crate::Fix) into four [`Hint`]s, one per level. One
//!    pass gives all four, so asking for level 2 after level 1 costs nothing when the patch has not changed.
//!
//! # The levels
//!
//! | Level | Says | Example |
//! | --- | --- | --- |
//! | 1 | which aspect is off, and how | "Check the brightness: your sound is darker than the target." |
//! | 2 | which module (or region) | "Your sound is darker than the target: look at the filter (filter1)." |
//! | 3 | which parameter, and which way to turn it | "…: raise Cutoff on the filter (filter1)." |
//! | 4 | a concrete value range | "…: set Cutoff on the filter (filter1) to between 1.6kHz and 2.6kHz." |
//!
//! **No early leak.** Levels 1 to 3 carry no number at all (a module id such as `filter1` is a name, not a value),
//! and the structured fields are empty until their level: `module` from 2, `param` and `turn` from 3, `range` at 4.
//! The text is built from the player's patch, the target's descriptors and the verified fix only; the target's
//! parameter values are read for one thing, the set of parameters the player may be told about. Even level 4 gives a
//! range, not the target's value: the verified fix's value plus and minus the distance at which the descriptor
//! stays within its tolerance (the sensitivity's slope), rounded outward to two significant digits.
//!
//! **What the pick is.** The aspects that fail are ranked by how far off they are in tolerance units. The first
//! one (worst first) that a move of the fix demonstrably serves, by the sensitivity's slope and sign, is the hint's
//! subject, and the move with the largest predicted effect on it is the hint's parameter. Failing that, a
//! categorical difference between the player's patch and the target (a waveform, a filter mode, a module switched
//! off) in a module that can affect the aspect is named (`measured: false`: the solver does not move categorical
//! parameters yet, `docs/dsp/hints.md`); failing that, one measured move the fix makes anyway; failing that, general
//! advice for the aspect. Nothing failing: every level says so.
//!
//! # Cost and determinism
//!
//! `prepare` costs `1 +` the randomized continuous parameters renders (at most [`SensitivityCfg::max_renders`]);
//! `plan` at most `HintCfg::max_renders` (the player's base render included), 24 by default. Both are pure: the same
//! inputs give the same text for any worker count, because every render goes through the caller's evaluator in
//! batches whose results are read by job index.

use core::fmt;

use enth_analysis::descriptors::{Descriptors, Feature};
use enth_analysis::expect::{Expect, ItemResult, Test};
use enth_analysis::margin::Interval;
use enth_challenge::{PreparedRound, model::SCORING_SAMPLE_RATE, model::SCORING_SEED};
use enth_params::{Kind, ParamSpec};
use enth_patch::catalogue::ModuleKind;
use enth_patch::{ModuleId, Op, ParamAddr, Patch, Target, ops_between};
use enth_render::{Performance, RenderSpec};

use crate::eval::Evaluator;
use crate::hint::{Fix, HintCfg, HintError, HintResult, Move, hint};
use crate::sensitivity::{
    Sensitivity, SensitivityCfg, SensitivityError, aim, domain_of, domain_value, is_log,
    sensitivity,
};

/// How many levels a hint has. The session's `HINT_LEVELS` is the same number (it lives above the solver, so this
/// crate cannot import it).
pub const HINT_LEVELS: u8 = 4;

/// An aspect that is missing from, or extra in, the player's sound counts as this many tolerance units off, which
/// ranks it before any measured difference.
const PRESENCE_ERR: f64 = 8.0;
/// The half-width of a level-4 range when the sensitivity has no slope for the move, on the normalised axis.
const DEFAULT_HALF_RANGE: f64 = 0.08;
/// The narrowest and widest half-width of a level-4 range, on the normalised axis (at least one probe step).
const MIN_HALF_RANGE: f64 = 0.02;
const MAX_HALF_RANGE: f64 = 0.25;

// ---------------------------------------------------------------------------------------------------------------
// Aspects
// ---------------------------------------------------------------------------------------------------------------

/// What a hint can say is off: report 02's per-descriptor breakdown.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum Aspect {
    /// The pitch of the note (cents against the target's).
    Pitch,
    /// The spectral centroid.
    Brightness,
    /// The time to the peak.
    Attack,
    /// The time from the peak to the held level.
    Decay,
    /// The held level.
    Sustain,
    /// The time to fall after the note-off.
    Release,
    /// The share of energy that is not in steady peaks.
    Noisiness,
    /// The rate of detune beating.
    Beating,
    /// The rate of an amplitude LFO.
    Tremolo,
    /// The depth of a filter sweep.
    Sweep,
}

impl Aspect {
    /// Every aspect, in the order the Expect list lists them.
    pub const ALL: [Aspect; 10] = [
        Aspect::Pitch,
        Aspect::Brightness,
        Aspect::Attack,
        Aspect::Decay,
        Aspect::Sustain,
        Aspect::Release,
        Aspect::Noisiness,
        Aspect::Beating,
        Aspect::Tremolo,
        Aspect::Sweep,
    ];

    /// The aspect a descriptor belongs to.
    #[must_use]
    pub fn from_feature(feature: Feature) -> Option<Aspect> {
        Some(match feature {
            Feature::Pitch | Feature::TonalPitch | Feature::PitchCents => Aspect::Pitch,
            Feature::Brightness => Aspect::Brightness,
            Feature::Attack => Aspect::Attack,
            Feature::Decay => Aspect::Decay,
            Feature::Sustain => Aspect::Sustain,
            Feature::Release => Aspect::Release,
            Feature::Noisiness => Aspect::Noisiness,
            Feature::Beating | Feature::BeatDepth => Aspect::Beating,
            Feature::Tremolo | Feature::TremoloDepth => Aspect::Tremolo,
            Feature::SweepRate | Feature::SweepDepth | Feature::SweepTime => Aspect::Sweep,
            _ => return None,
        })
    }

    /// A stable snake-case name.
    #[must_use]
    pub fn code(self) -> &'static str {
        match self {
            Aspect::Pitch => "pitch",
            Aspect::Brightness => "brightness",
            Aspect::Attack => "attack",
            Aspect::Decay => "decay",
            Aspect::Sustain => "sustain",
            Aspect::Release => "release",
            Aspect::Noisiness => "noisiness",
            Aspect::Beating => "beating",
            Aspect::Tremolo => "tremolo",
            Aspect::Sweep => "sweep",
        }
    }

    /// The name a person reads ("the brightness", "the sustain level").
    fn name(self) -> &'static str {
        match self {
            Aspect::Sustain => "sustain level",
            Aspect::Sweep => "filter sweep",
            other => other.code(),
        }
    }

    /// What the aspect is, as a noun phrase, for "the target has … that your sound lacks".
    fn noun(self) -> &'static str {
        match self {
            Aspect::Pitch => "a steady pitch",
            Aspect::Brightness => "brightness",
            Aspect::Attack => "an attack",
            Aspect::Decay => "a decay stage",
            Aspect::Sustain => "a held level",
            Aspect::Release => "a release",
            Aspect::Noisiness => "noise",
            Aspect::Beating => "beating (a slow wobble from detuned oscillators)",
            Aspect::Tremolo => "tremolo (a level wobble)",
            Aspect::Sweep => "a filter sweep",
        }
    }

    /// How the player's sound differs from the target's, `higher` when the player's reading is the larger.
    fn clause(self, higher: bool) -> &'static str {
        match (self, higher) {
            (Aspect::Pitch, true) => "Your sound is higher than the target",
            (Aspect::Pitch, false) => "Your sound is lower than the target",
            (Aspect::Brightness, true) => "Your sound is brighter than the target",
            (Aspect::Brightness, false) => "Your sound is darker than the target",
            (Aspect::Attack, true) => "Your attack is slower than the target's",
            (Aspect::Attack, false) => "Your attack is faster than the target's",
            (Aspect::Decay, true) => "Your decay is longer than the target's",
            (Aspect::Decay, false) => "Your decay is shorter than the target's",
            (Aspect::Sustain, true) => "Your sustain level is higher than the target's",
            (Aspect::Sustain, false) => "Your sustain level is lower than the target's",
            (Aspect::Release, true) => "Your release is longer than the target's",
            (Aspect::Release, false) => "Your release is shorter than the target's",
            (Aspect::Noisiness, true) => "Your sound is noisier than the target",
            (Aspect::Noisiness, false) => "Your sound is cleaner than the target",
            (Aspect::Beating, true) => "Your beating is faster than the target's",
            (Aspect::Beating, false) => "Your beating is slower than the target's",
            (Aspect::Tremolo, true) => "Your tremolo is faster than the target's",
            (Aspect::Tremolo, false) => "Your tremolo is slower than the target's",
            (Aspect::Sweep, true) => "Your filter sweep is deeper than the target's",
            (Aspect::Sweep, false) => "Your filter sweep is shallower than the target's",
        }
    }

    /// Where to look, when no particular module is known.
    fn region(self) -> &'static str {
        match self {
            Aspect::Pitch | Aspect::Beating => "the oscillators",
            Aspect::Brightness => "the filter and the oscillators' waveforms",
            Aspect::Attack | Aspect::Decay | Aspect::Sustain => "the amplitude envelope",
            Aspect::Release => "the amplitude envelope and the reverb",
            Aspect::Noisiness => "the noise generator and the oscillators' waveforms",
            Aspect::Tremolo => "the LFOs and where they are routed",
            Aspect::Sweep => "the filter, and the envelopes and LFOs routed to it",
        }
    }

    /// General advice for the aspect (never a value), for when no measured move or categorical difference is known.
    fn advice(self) -> &'static str {
        match self {
            Aspect::Pitch => "tune the oscillator, by semitones for the note and by cents for the last bit",
            Aspect::Brightness => "open or close the filter, or pick a waveform with more or fewer harmonics",
            Aspect::Attack => "change the envelope's attack time",
            Aspect::Decay => "change the envelope's decay time, and its sustain level with it",
            Aspect::Sustain => "change the envelope's sustain level",
            Aspect::Release => "change the envelope's release time",
            Aspect::Noisiness => "add or remove the noise generator, or pick a purer waveform",
            Aspect::Beating => "set two oscillators slightly apart in tune, or switch the second one off",
            Aspect::Tremolo => "route an LFO to the level, and set its rate",
            Aspect::Sweep => "route an envelope or an LFO to the filter's cutoff, and set its depth",
        }
    }

    /// Whether a module of this kind can change the aspect.
    fn involves(self, kind: ModuleKind) -> bool {
        match self {
            Aspect::Pitch | Aspect::Beating => kind == ModuleKind::Osc,
            Aspect::Brightness => matches!(kind, ModuleKind::Osc | ModuleKind::Svf | ModuleKind::Noise),
            Aspect::Attack | Aspect::Decay | Aspect::Sustain => kind == ModuleKind::Adsr,
            Aspect::Release => matches!(kind, ModuleKind::Adsr | ModuleKind::Reverb),
            Aspect::Noisiness => matches!(kind, ModuleKind::Noise | ModuleKind::Osc),
            Aspect::Tremolo => kind == ModuleKind::Lfo,
            Aspect::Sweep => matches!(kind, ModuleKind::Svf | ModuleKind::Lfo | ModuleKind::Adsr),
        }
    }
}

impl fmt::Display for Aspect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}

// ---------------------------------------------------------------------------------------------------------------
// The hint
// ---------------------------------------------------------------------------------------------------------------

/// How the player's sound differs from the target's in the hint's aspect.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Difference {
    /// The player's reading is larger (higher pitch, brighter, a slower attack, a longer release…).
    Higher,
    /// The player's reading is smaller.
    Lower,
    /// The target has the aspect and the player's sound does not.
    Missing,
    /// The player's sound has the aspect and the target does not.
    Extra,
}

/// Which way to turn a knob.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Turn {
    /// Up: a larger plain value.
    Raise,
    /// Down: a smaller plain value.
    Lower,
}

/// One revealed hint. The structured fields are filled only from their level on, so a caller (or a test) can check
/// that nothing is shown early.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Hint {
    /// 1 ..= [`HINT_LEVELS`]: the session's `CoachHint::level`.
    pub level: u8,
    /// What to show, plain language: the session's `CoachHint::text`.
    pub text: String,
    /// Level 1 on: the aspect that is off (none when the sound already matches everything measured).
    pub aspect: Option<Aspect>,
    /// Level 1 on: how the player's sound differs in that aspect.
    pub difference: Option<Difference>,
    /// Level 2 on: the module to look at (none for a region such as the voice settings, or general advice).
    pub module: Option<ModuleId>,
    /// Level 3 on: the parameter to change.
    pub param: Option<ParamAddr>,
    /// Level 3 on: which way to turn it (none for a categorical parameter, which is changed, not turned).
    pub turn: Option<Turn>,
    /// Level 4: the plain values the parameter should land between, rounded outward to two significant digits
    /// (none for a categorical parameter, whose level-4 text names the setting).
    pub range: Option<(f32, f32)>,
    /// The hint comes from a verified render ([`hint`](crate::hint())'s fix), not from a comparison of patches or
    /// general advice.
    pub measured: bool,
}

impl Hint {
    fn new(level: u8, text: String) -> Hint {
        Hint {
            level,
            text,
            aspect: None,
            difference: None,
            module: None,
            param: None,
            turn: None,
            range: None,
            measured: false,
        }
    }
}

/// What a plan found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Verdict {
    /// Everything measured is within tolerance: nothing to say.
    Close,
    /// A move of the verified fix serves the aspect the hint names.
    Measured,
    /// A categorical difference (a waveform, a mode, a switched-off module) is named; it is not a measured move.
    Structure,
    /// No measured move and no categorical difference: general advice for the worst aspect.
    Advice,
}

/// The four hints for one patch, and what they came from.
#[derive(Clone, Debug, PartialEq)]
pub struct CoachPlan {
    /// The hints, level 1 to [`HINT_LEVELS`], in order.
    pub hints: Vec<Hint>,
    /// What the plan found.
    pub verdict: Verdict,
    /// The aspects that fail, worst first.
    pub failing: Vec<Aspect>,
    /// The solver's own result (the fix, the blocked verdicts, the counters): `result.stats.renders` is what the
    /// job cost. It holds exact values: show a player the [`hints`](Self::hints), not this.
    pub result: HintResult,
}

impl CoachPlan {
    /// The hint at `level` (clamped to 1 ..= [`HINT_LEVELS`]).
    #[must_use]
    pub fn hint(&self, level: u8) -> &Hint {
        let i = usize::from(level.clamp(1, HINT_LEVELS) - 1);
        &self.hints[i]
    }

    /// Evaluations the plan used.
    #[must_use]
    pub fn renders(&self) -> usize {
        self.result.stats.renders
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Configuration, errors
// ---------------------------------------------------------------------------------------------------------------

/// Settings of the coach. Build with `..Default::default()`; fields may be added, with defaults.
#[derive(Clone, Debug, PartialEq)]
pub struct CoachCfg {
    /// The target's measurement: `max_renders` is the budget of [`CoachTarget::prepare`], the target's render
    /// included.
    pub sensitivity: SensitivityCfg,
    /// The solver's settings: `max_renders` is the budget of one [`CoachTarget::plan`], the player's base render
    /// included. `only` and `frozen` are set by the coach; `transpose` is on by default (`osc.semi` is a randomized
    /// parameter: "raise Semitones" is the level-1 hint of the game).
    pub hint: HintCfg,
}

impl Default for CoachCfg {
    fn default() -> Self {
        Self {
            sensitivity: SensitivityCfg::default(),
            hint: HintCfg {
                max_renders: 24,
                transpose: true,
                ..HintCfg::default()
            },
        }
    }
}

/// Why the coach could not answer.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum CoachError {
    /// The target could not be measured.
    Target(SensitivityError),
    /// The solver could not run on the player's patch.
    Hint(HintError),
    /// The target is silent or has none of the descriptors the coach reads.
    Silent,
}

impl CoachError {
    /// A stable snake-case code.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            CoachError::Target(e) => e.code(),
            CoachError::Hint(e) => e.code(),
            CoachError::Silent => "silent_target",
        }
    }
}

impl fmt::Display for CoachError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CoachError::Target(e) => write!(f, "the target could not be measured: {e}"),
            CoachError::Hint(e) => write!(f, "the hint could not run: {e}"),
            CoachError::Silent => f.write_str("the target has nothing the coach can measure"),
        }
    }
}

impl std::error::Error for CoachError {}

// ---------------------------------------------------------------------------------------------------------------
// The target: the Expect list
// ---------------------------------------------------------------------------------------------------------------

/// One expectation of the coach's list, with what the coach needs to talk about it.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    /// The aspect.
    pub aspect: Aspect,
    /// The descriptor the expectation reads.
    pub feature: Feature,
    /// The target's reading in the compare domain ([`domain_of`]); `None` for an `== null` item (the target has no
    /// such movement).
    pub target: Option<f64>,
    /// The tolerance, in the same domain ([`Sensitivity::tolerance`]); 0 for an `== null` item.
    pub tolerance: f64,
}

/// A target prepared for coaching: its expectation list and its sensitivity. Made once per round
/// ([`prepare`](Self::prepare), or [`for_round`](Self::for_round)) and kept with it; `plan` is then one hint job.
#[derive(Clone, Debug)]
pub struct CoachTarget {
    perf: Performance,
    spec: RenderSpec,
    target: Patch,
    randomized: Vec<ParamAddr>,
    items: Vec<Item>,
    expects: Vec<Expect>,
    sensitivity: Sensitivity,
}

impl CoachTarget {
    /// Render and describe `target`, measure the sensitivity of the `randomized` parameters and build the list.
    ///
    /// # Errors
    /// The target cannot be rendered or described, the budget is 0, or the target is silent.
    pub fn prepare(
        target: &Patch,
        randomized: &[ParamAddr],
        perf: &Performance,
        spec: &RenderSpec,
        eval: &mut dyn Evaluator,
        cfg: &CoachCfg,
    ) -> Result<CoachTarget, CoachError> {
        let sens = sensitivity(target, randomized, perf, spec, eval, &cfg.sensitivity)
            .map_err(CoachError::Target)?;
        let items = build_items(&sens);
        if items.is_empty() {
            return Err(CoachError::Silent);
        }
        let expects = items.iter().map(|i| expect_of(i, &sens.base)).collect();
        Ok(CoachTarget {
            perf: perf.clone(),
            spec: *spec,
            target: target.clone(),
            randomized: randomized.to_vec(),
            items,
            expects,
            sensitivity: sens,
        })
    }

    /// [`prepare`](Self::prepare) for a prepared round: its target, its randomized parameters, its phrase played at
    /// the scoring sample rate and seed (so the coach hears what the scorer hears).
    ///
    /// # Errors
    /// As [`prepare`](Self::prepare).
    pub fn for_round(
        round: &PreparedRound,
        eval: &mut dyn Evaluator,
        cfg: &CoachCfg,
    ) -> Result<CoachTarget, CoachError> {
        let perf = round.phrase().performance();
        let spec = RenderSpec::new(SCORING_SAMPLE_RATE, SCORING_SEED);
        CoachTarget::prepare(round.target(), round.params(), &perf, &spec, eval, cfg)
    }

    /// The expectation list the coach holds the player's patch to, one per [`Item`].
    #[must_use]
    pub fn expects(&self) -> &[Expect] {
        &self.expects
    }

    /// What the list is made of.
    #[must_use]
    pub fn items(&self) -> &[Item] {
        &self.items
    }

    /// The target's sensitivity (kept for the tolerances and the slopes; ST reads it too).
    #[must_use]
    pub fn sensitivity(&self) -> &Sensitivity {
        &self.sensitivity
    }

    /// The four hints for `patch`: run [`hint`](crate::hint()) on it against the list and map the verified fix.
    ///
    /// # Errors
    /// The player's patch cannot be rendered or described, or the budget is 0.
    pub fn plan(
        &self,
        patch: &Patch,
        eval: &mut dyn Evaluator,
        cfg: &CoachCfg,
    ) -> Result<CoachPlan, CoachError> {
        // What the solver may move: the randomized parameters, and any other the player's patch has moved away from
        // the target (a knob the player turned that the target left alone).
        let diff = ops_between(patch, &self.target).unwrap_or_default();
        let mut only = self.randomized.clone();
        for op in &diff {
            if let Op::SetParam { addr, .. } = op {
                if !only.contains(addr) {
                    only.push(addr.clone());
                }
            }
        }
        let hint_cfg = HintCfg {
            only: Some(only),
            ..cfg.hint.clone()
        };
        let result = hint(patch, &self.perf, &self.spec, &self.expects, eval, &hint_cfg)
            .map_err(CoachError::Hint)?;
        Ok(self.explain(patch, &diff, result))
    }

    /// The hint at `level` for `patch`: [`plan`](Self::plan), then the one level.
    ///
    /// # Errors
    /// As [`plan`](Self::plan).
    pub fn hint(
        &self,
        patch: &Patch,
        level: u8,
        eval: &mut dyn Evaluator,
        cfg: &CoachCfg,
    ) -> Result<Hint, CoachError> {
        Ok(self.plan(patch, eval, cfg)?.hint(level).clone())
    }
}

/// One-shot coaching for a prepared round: prepare the target, plan the player's patch, return the hint at `level`.
/// Spends up to `cfg.sensitivity.max_renders + cfg.hint.max_renders` evaluations; keep a [`CoachTarget`] to pay the
/// first part once per round.
///
/// # Errors
/// As [`CoachTarget::for_round`] and [`CoachTarget::plan`].
pub fn coach_hint(
    round: &PreparedRound,
    patch: &Patch,
    level: u8,
    eval: &mut dyn Evaluator,
    cfg: &CoachCfg,
) -> Result<Hint, CoachError> {
    CoachTarget::for_round(round, eval, cfg)?.hint(patch, level, eval, cfg)
}

/// The items of the list, from the target's descriptors and the sensitivity's tolerances.
fn build_items(sens: &Sensitivity) -> Vec<Item> {
    let base = &sens.base;
    let mut items = Vec::new();
    for aspect in Aspect::ALL {
        let feature = match aspect {
            Aspect::Pitch if base.pitch.is_some() => Feature::Pitch,
            Aspect::Pitch if base.tonal.is_some() => Feature::TonalPitch,
            Aspect::Pitch => continue,
            Aspect::Brightness => Feature::Brightness,
            Aspect::Attack => Feature::Attack,
            Aspect::Decay => Feature::Decay,
            Aspect::Sustain => Feature::Sustain,
            Aspect::Release => Feature::Release,
            Aspect::Noisiness => Feature::Noisiness,
            Aspect::Beating => Feature::Beating,
            Aspect::Tremolo => Feature::Tremolo,
            Aspect::Sweep => Feature::SweepDepth,
        };
        let movement = matches!(aspect, Aspect::Beating | Aspect::Tremolo | Aspect::Sweep);
        match domain_value(feature, base) {
            Some(target) => items.push(Item {
                aspect,
                feature,
                target: Some(target),
                tolerance: sens.tolerance(feature),
            }),
            // Movement the target does not have is part of its sound: `== null`.
            None if movement && base.get(feature).is_none() => items.push(Item {
                aspect,
                feature,
                target: None,
                tolerance: 0.0,
            }),
            None => {}
        }
    }
    items
}

/// A number for a canonical text: three decimals, trailing zeros trimmed.
fn short(v: f64) -> String {
    let s = format!("{v:.3}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// The `Expect` of one item: a band around the target's reading (a ratio for log features, cents for pitch), or
/// `== null`.
fn expect_of(item: &Item, base: &Descriptors) -> Expect {
    let path = item.feature.path();
    let raw = base.get(item.feature);
    let (canonical, test, reference_hz) = match (item.target, raw) {
        (Some(_), Some(raw)) if item.feature.is_pitch() => (
            format!(
                "{path} = {}Hz ± {}ct",
                short(raw),
                short(item.tolerance)
            ),
            Test::Within(Interval::closed(-item.tolerance, item.tolerance)),
            Some(raw),
        ),
        (Some(t), Some(_)) => {
            let (lo, hi) = band(item.feature, t, item.tolerance);
            (
                format!("{path} in [{}, {}]", short(lo), short(hi)),
                Test::Within(Interval::closed(lo, hi)),
                None,
            )
        }
        _ => (format!("{path} == null"), Test::IsNull, None),
    };
    Expect {
        source: canonical.clone(),
        canonical,
        feature: item.feature,
        test,
        reference_hz,
    }
}

/// The accepted interval around a target reading `t` (compare domain) with half-width `tol`, in the descriptor's
/// own unit.
fn band(feature: Feature, t: f64, tol: f64) -> (f64, f64) {
    if is_log(feature) {
        // The band is a ratio.
        (libm::exp(t - tol), libm::exp(t + tol))
    } else {
        (t - tol, t + tol)
    }
}

// ---------------------------------------------------------------------------------------------------------------
// From the solver's result to four hints
// ---------------------------------------------------------------------------------------------------------------

/// An aspect that fails, and how.
#[derive(Clone, Copy, Debug)]
struct Finding {
    item: usize,
    aspect: Aspect,
    feature: Feature,
    /// How far off, in tolerance units.
    err: f64,
    difference: Difference,
}

/// What the hints are about.
enum Subject<'a> {
    /// A move of the verified fix.
    Move {
        finding: Finding,
        mv: &'a Move,
    },
    /// A categorical difference between the player's patch and the target.
    Structure {
        finding: Finding,
        change: Categorical,
    },
    /// Nothing known to touch: general advice.
    Advice {
        finding: Finding,
    },
}

/// A categorical difference between the player's patch and the target.
#[derive(Clone, Debug)]
enum Categorical {
    /// A categorical parameter (waveform, filter mode, toggle) set differently; `value` is the target's.
    Param {
        addr: ParamAddr,
        module: ModuleId,
        kind: ModuleKind,
        spec: &'static ParamSpec,
        value: f32,
    },
    /// A module switched on in one patch and off in the other; `on` is the target's.
    Enabled {
        module: ModuleId,
        kind: ModuleKind,
        on: bool,
    },
}

impl Categorical {
    fn kind(&self) -> ModuleKind {
        match self {
            Categorical::Param { kind, .. } | Categorical::Enabled { kind, .. } => *kind,
        }
    }

    fn module(&self) -> &ModuleId {
        match self {
            Categorical::Param { module, .. } | Categorical::Enabled { module, .. } => module,
        }
    }
}

/// The player's reading of an item in the compare domain.
fn player_value(item: &Item, r: &ItemResult) -> Option<f64> {
    r.got.and_then(|v| domain_of(item.feature, v))
}

/// The failing items, worst first (ties by list order).
fn findings(items: &[Item], before: &[ItemResult]) -> Vec<Finding> {
    let mut out: Vec<Finding> = Vec::new();
    for (i, (item, r)) in items.iter().zip(before).enumerate() {
        if r.pass {
            continue;
        }
        let (difference, err) = match (item.target, player_value(item, r)) {
            (Some(t), Some(p)) => {
                let diff = p - t;
                let d = if diff > 0.0 {
                    Difference::Higher
                } else {
                    Difference::Lower
                };
                (d, libm::fabs(diff) / item.tolerance.max(1e-12))
            }
            (Some(_), None) => (Difference::Missing, PRESENCE_ERR),
            (None, Some(_)) => (Difference::Extra, PRESENCE_ERR),
            (None, None) => continue,
        };
        out.push(Finding {
            item: i,
            aspect: item.aspect,
            feature: item.feature,
            err,
            difference,
        });
    }
    out.sort_by(|a, b| b.err.total_cmp(&a.err).then(a.item.cmp(&b.item)));
    out
}

/// The move of `fix` that serves `finding` best, by the sensitivity's slope: the largest predicted change of the
/// descriptor toward the target. For an aspect that is missing or extra, a move whose parameter is tagged with the
/// aspect's descriptor (largest normalised move).
fn attribute<'a>(finding: &Finding, fix: &'a Fix, sens: &Sensitivity) -> Option<&'a Move> {
    let mut best: Option<(&Move, f64)> = None;
    for m in &fix.moves {
        let score = match finding.difference {
            Difference::Higher | Difference::Lower => {
                let want = if finding.difference == Difference::Higher {
                    -1.0
                } else {
                    1.0
                };
                let Some(slope) = sens
                    .param(&m.addr)
                    .and_then(|p| p.slope(finding.feature))
                else {
                    continue;
                };
                let predicted = slope * m.normalised() * want;
                if predicted < 0.1 * aim(finding.feature) {
                    continue;
                }
                predicted
            }
            Difference::Missing | Difference::Extra => {
                let tagged = m
                    .spec
                    .perceptual
                    .and_then(Feature::parse)
                    .and_then(Aspect::from_feature)
                    == Some(finding.aspect);
                if !tagged {
                    continue;
                }
                libm::fabs(m.normalised())
            }
        };
        if best.is_none_or(|(_, s)| score > s) {
            best = Some((m, score));
        }
    }
    best.map(|(m, _)| m)
}

/// The categorical differences between `patch` and the target, from the verified op list that turns one into the
/// other: categorical parameters and switched modules, in patch order.
fn categorical_differences(patch: &Patch, target: &Patch, diff: &[Op]) -> Vec<Categorical> {
    let mut out = Vec::new();
    for op in diff {
        match op {
            Op::SetParam { addr, value } => {
                let (Some(spec), Target::Module(id)) = (target.spec(addr), addr.target()) else {
                    continue;
                };
                if matches!(spec.kind, Kind::Float { .. }) {
                    continue;
                }
                let Some((_, m)) = target.find(id) else {
                    continue;
                };
                out.push(Categorical::Param {
                    addr: addr.clone(),
                    module: id.clone(),
                    kind: m.kind,
                    spec,
                    value: *value,
                });
            }
            Op::SetEnabled { id, enabled } => {
                let kind = target
                    .find(id)
                    .or_else(|| patch.find(id))
                    .map(|(_, m)| m.kind);
                if let Some(kind) = kind {
                    out.push(Categorical::Enabled {
                        module: id.clone(),
                        kind,
                        on: *enabled,
                    });
                }
            }
            _ => {}
        }
    }
    out
}

impl CoachTarget {
    /// The hints, from the solver's result on `patch`.
    fn explain(&self, patch: &Patch, diff: &[Op], result: HintResult) -> CoachPlan {
        let found = findings(&self.items, &result.before);
        let failing: Vec<Aspect> = {
            let mut seen: Vec<Aspect> = Vec::new();
            for f in &found {
                if !seen.contains(&f.aspect) {
                    seen.push(f.aspect);
                }
            }
            seen
        };
        let Some(worst) = found.first().copied() else {
            return CoachPlan {
                hints: close_hints(),
                verdict: Verdict::Close,
                failing,
                result,
            };
        };
        let categorical = categorical_differences(patch, &self.target, diff);
        let sens = &self.sensitivity;
        let fix = result.fix.as_ref();
        // Worst first: a move that serves it, else a categorical difference in a module that can change it.
        let mut subject: Option<Subject<'_>> = None;
        for f in &found {
            if let Some(mv) = fix.and_then(|fx| attribute(f, fx, sens)) {
                subject = Some(Subject::Move { finding: *f, mv });
                break;
            }
            if let Some(change) = categorical.iter().find(|c| f.aspect.involves(c.kind())) {
                subject = Some(Subject::Structure {
                    finding: *f,
                    change: change.clone(),
                });
                break;
            }
        }
        // Else a move the fix makes anyway (the biggest), else advice.
        let subject = subject.unwrap_or_else(|| {
            match fix.and_then(|fx| {
                fx.moves
                    .iter()
                    .max_by(|a, b| a.normalised().abs().total_cmp(&b.normalised().abs()))
            }) {
                Some(mv) => Subject::Move { finding: worst, mv },
                None => Subject::Advice { finding: worst },
            }
        });
        let (hints, verdict) = self.tell(patch, &subject, fix);
        CoachPlan {
            hints,
            verdict,
            failing,
            result,
        }
    }

    /// The four hints for a subject.
    fn tell(&self, patch: &Patch, subject: &Subject<'_>, fix: Option<&Fix>) -> (Vec<Hint>, Verdict) {
        let (finding, verdict) = match subject {
            Subject::Move { finding, .. } => (finding, Verdict::Measured),
            Subject::Structure { finding, .. } => (finding, Verdict::Structure),
            Subject::Advice { finding } => (finding, Verdict::Advice),
        };
        let lead = lead(finding);
        let mut hints: Vec<Hint> = (1..=HINT_LEVELS)
            .map(|level| {
                let mut h = Hint::new(level, String::new());
                h.aspect = Some(finding.aspect);
                h.difference = Some(finding.difference);
                h
            })
            .collect();
        hints[0].text = format!("Check the {}: {}.", finding.aspect.name(), lower_first(&lead));
        match subject {
            Subject::Move { finding, mv } => {
                let place = place(patch, &self.target, mv.addr.target());
                let turn = if mv.to > mv.from { Turn::Raise } else { Turn::Lower };
                let verb = if turn == Turn::Raise { "raise" } else { "lower" };
                let (lo, hi) = self.range(mv, finding);
                hints[1].text = format!("{lead}: look at {place}.");
                hints[2].text = format!("{lead}: {verb} {} on {place}.", mv.spec.name);
                hints[3].text = format!(
                    "{lead}: set {} on {place} to between {} and {}.{}",
                    mv.spec.name,
                    mv.spec.display(lo),
                    mv.spec.display(hi),
                    self.also(patch, mv, fix),
                );
                for h in &mut hints {
                    h.measured = true;
                }
                for h in &mut hints[1..] {
                    h.module = mv.addr.module_id().cloned();
                }
                for h in &mut hints[2..] {
                    h.param = Some(mv.addr.clone());
                    h.turn = Some(turn);
                }
                hints[3].range = Some((lo, hi));
            }
            Subject::Structure { change, .. } => {
                let module = change.module();
                let place = place(patch, &self.target, &Target::Module(module.clone()));
                hints[1].text = format!("{lead}: look at {place}.");
                match change {
                    Categorical::Param {
                        addr, spec, value, ..
                    } => {
                        hints[2].text = format!("{lead}: change {} on {place}.", spec.name);
                        hints[3].text = format!(
                            "{lead}: set {} on {place} to {}.",
                            spec.name,
                            spec.display(*value)
                        );
                        for h in &mut hints[2..] {
                            h.param = Some(addr.clone());
                        }
                    }
                    Categorical::Enabled { on, .. } => {
                        hints[2].text =
                            format!("{lead}: check whether {place} should be switched on or off.");
                        hints[3].text = format!(
                            "{lead}: switch {place} {}.",
                            if *on { "on" } else { "off" }
                        );
                    }
                }
                for h in &mut hints[1..] {
                    h.module = Some(module.clone());
                }
            }
            Subject::Advice { finding } => {
                let a = finding.aspect;
                hints[1].text = format!("{lead}: look at {}.", a.region());
                hints[2].text = format!("{lead}: {}.", a.advice());
                hints[3].text = format!(
                    "{lead}: {}. No single knob move helped in my tests, so this is general advice.",
                    a.advice()
                );
            }
        }
        (hints, verdict)
    }

    /// The level-4 range of a move: the verified value plus and minus the distance at which the descriptor stays
    /// within its tolerance, on the normalised axis, rounded outward to two significant digits.
    fn range(&self, mv: &Move, finding: &Finding) -> (f32, f32) {
        let spec = mv.spec;
        let u = f64::from(spec.to_norm(mv.to));
        let ps = self.sensitivity.param(&mv.addr);
        let tolerance = self.items[finding.item].tolerance;
        let half = ps
            .and_then(|p| p.slope(finding.feature))
            .filter(|s| libm::fabs(*s) > 1e-9)
            .map_or(DEFAULT_HALF_RANGE, |s| tolerance / libm::fabs(s));
        let floor = ps.map_or(0.0, |p| libm::fabs(p.step)).max(MIN_HALF_RANGE);
        let half = half.clamp(floor, MAX_HALF_RANGE);
        let lo = spec.from_norm((u - half).max(0.0) as f32);
        let hi = spec.from_norm((u + half).min(1.0) as f32);
        let lo = spec.clamp(round_outward(f64::from(lo), false) as f32);
        let hi = spec.clamp(round_outward(f64::from(hi), true) as f32);
        (lo.min(mv.to), hi.max(mv.to))
    }

    /// A closing sentence naming the other knobs the fix moved (names only).
    fn also(&self, patch: &Patch, mv: &Move, fix: Option<&Fix>) -> String {
        let Some(fix) = fix else {
            return String::new();
        };
        let others: Vec<String> = fix
            .moves
            .iter()
            .filter(|m| m.addr != mv.addr)
            .take(2)
            .map(|m| {
                format!(
                    "{} on {}",
                    m.spec.name,
                    place(patch, &self.target, m.addr.target())
                )
            })
            .collect();
        if others.is_empty() {
            String::new()
        } else {
            format!(" The same fix also moves {}.", others.join(" and "))
        }
    }
}

/// The sentence a finding opens with, capitalised and without a full stop.
fn lead(f: &Finding) -> String {
    match f.difference {
        Difference::Higher => f.aspect.clause(true).to_string(),
        Difference::Lower => f.aspect.clause(false).to_string(),
        Difference::Missing => format!("The target has {} that your sound lacks", f.aspect.noun()),
        Difference::Extra => format!("Your sound has {} that the target lacks", f.aspect.noun()),
    }
}

fn lower_first(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(first) => first.to_lowercase().chain(c).collect(),
        None => String::new(),
    }
}

/// The four hints when everything measured is within tolerance.
fn close_hints() -> Vec<Hint> {
    let text = "Everything I can measure already matches the target: pitch, brightness, the envelope and any \
                movement are all within tolerance. If it still sounds different, listen for what I cannot measure, \
                such as the waveform or the exact stereo and effect settings.";
    (1..=HINT_LEVELS)
        .map(|level| Hint::new(level, text.to_string()))
        .collect()
}

/// "the filter (filter1)", "the voice settings", "the master section".
fn place(patch: &Patch, target: &Patch, t: &Target) -> String {
    match t {
        Target::Module(id) => {
            let kind = patch
                .find(id)
                .or_else(|| target.find(id))
                .map(|(_, m)| m.kind);
            match kind {
                Some(kind) => format!("the {} ({id})", kind_noun(kind)),
                None => format!("module {id}"),
            }
        }
        Target::Voice => "the voice settings".to_string(),
        Target::Master => "the master section".to_string(),
    }
}

fn kind_noun(kind: ModuleKind) -> &'static str {
    match kind {
        ModuleKind::Osc => "oscillator",
        ModuleKind::Noise => "noise generator",
        ModuleKind::Svf => "filter",
        ModuleKind::Adsr => "envelope",
        ModuleKind::Lfo => "LFO",
        ModuleKind::Reverb => "reverb",
        ModuleKind::Comp => "compressor",
    }
}

/// `v` rounded outward (down for a lower end, up for an upper end) to two significant digits.
fn round_outward(v: f64, up: bool) -> f64 {
    if v == 0.0 || !v.is_finite() {
        return v;
    }
    let e = libm::floor(libm::log10(libm::fabs(v))) as i32;
    let places = 1 - e; // decimal places that keep two significant digits
    let pick = |x: f64| if up { libm::ceil(x) } else { libm::floor(x) };
    if places >= 0 {
        let scale = libm::pow(10.0, f64::from(places));
        pick(v * scale) / scale
    } else {
        let step = libm::pow(10.0, f64::from(-places));
        pick(v / step) * step
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outward_rounding_keeps_two_significant_digits() {
        assert_eq!(round_outward(1_234.0, false), 1_200.0);
        assert_eq!(round_outward(1_234.0, true), 1_300.0);
        assert_eq!(round_outward(0.1234, false), 0.12);
        assert_eq!(round_outward(0.1234, true), 0.13);
        assert_eq!(round_outward(-1.37, false), -1.4);
        assert_eq!(round_outward(-1.37, true), -1.3);
        assert_eq!(round_outward(0.0, true), 0.0);
        assert_eq!(round_outward(1_600.0, true), 1_600.0);
    }

    #[test]
    fn every_aspect_has_words_and_no_digits() {
        for a in Aspect::ALL {
            for text in [
                a.name(),
                a.noun(),
                a.clause(true),
                a.clause(false),
                a.region(),
                a.advice(),
            ] {
                assert!(!text.is_empty());
                assert!(
                    !text.chars().any(|c| c.is_ascii_digit()),
                    "{a:?}: {text}"
                );
            }
        }
    }

    #[test]
    fn the_close_hints_are_the_same_at_every_level() {
        let h = close_hints();
        assert_eq!(h.len(), usize::from(HINT_LEVELS));
        assert!(h.iter().all(|x| x.text == h[0].text && x.aspect.is_none()));
        assert_eq!(
            h.iter().map(|x| x.level).collect::<Vec<_>>(),
            vec![1, 2, 3, 4]
        );
    }

    #[test]
    fn a_band_is_a_ratio_on_log_features_and_a_difference_otherwise() {
        let (lo, hi) = band(Feature::Brightness, libm::log(2_000.0), 0.2);
        assert!((lo - 2_000.0 * libm::exp(-0.2)).abs() < 1e-9);
        assert!((hi - 2_000.0 * libm::exp(0.2)).abs() < 1e-9);
        assert_eq!(band(Feature::Sustain, -6.0, 3.0), (-9.0, -3.0));
        assert_eq!(crate::sensitivity::tolerance_bounds(Feature::Sustain), (3.0, 12.0));
    }
}
