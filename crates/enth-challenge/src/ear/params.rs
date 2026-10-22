//! **Owner: lane CV3** (the items; lane G4 tunes their ladder in [`super::ladder`]). Pre-declared by lane S0e (R4/R5,
//! freeze point F0e). The reference is docs/ear.md, "The games" and "The ear generator" (the parameter games).
//!
//! The parameter games need the catalogue, which `enth-music` cannot read, so their items are v2 draws built here
//! (games 1 to 3 of an [`EarSpec`]):
//!
//! | Game | Item | Answer | Clips |
//! | --- | --- | --- | --- |
//! | 1 **Spot the change** ([`spot_the_change`]) | 2 to 6 option controls; one moves by the level's distance | the moved option's index (`ear/1`) | A (the base), then B (one control moved) |
//! | 2 **Dial it in** ([`dial_it_in`]) | one control moved; turn it back to match | a patch (`sound/1`): a constrained Recreate round, a v2 [`Challenge`] on a one-item draw, `editable` the control | the target, as any round |
//! | 3 **Ear check** ([`ear_check`]) | one control; three intervals, one moved by the level's distance | the odd interval's position (`ear/1`) | three clips, two the base and one moved |
//!
//! **In parameter space only, as every generator rule is** (design.md §9): the base is template 0 (the starter, "Init
//! saw"), heard through the held one-note phrase (parameter games stay on one note, where `sound/1` can measure them,
//! and hold the pitch fixed while training brightness). A control moves by a whole number of its parameter steps
//! (generator v1's just-noticeable steps, [`crate::priors::jnd`]: cutoff 2 semitones, times × 1.5, levels 3 dB), up or
//! down as the seed says (the other way when the range ends first), to a reachable value (generator v1's quantization).
//! Only observable controls are offered or moved ([`crate::random::observable`] on the base, and the v2 draw's own
//! playability check on the moved patch). The moved value is a one-value set of a v2 [`Draw`], so an item is exactly
//! what a lesson's draw would play.
//!
//! **Keyed streams, as the ear generator's:** item `i` of game `g` at level `l` draws from `derive(seed,
//! fnv1a64("g/i/l"))`, so an item never depends on another, and an Ear check set's one control from `derive(seed,
//! fnv1a64("3/control/l"))`. A bad draw (a control the range stops short of, an unplayable target) draws again at
//! the next attempt, up to 16, as v2's draws do.
//!
//! **Levels are G4's.** This file has level 1 of each game, from docs/ear.md's ladder (Spot the change: two far
//! options, filter against envelope, 6 steps; Ear check: one control, starting at 8 steps; Dial it in: 6 steps, the
//! default pass mark), so the games play end to end; lane G4's ladder ([`super::ladder`]) replaces [`level`] with its
//! full table and its *not measured* list, keeping level 1 or moving it on purpose (the item pins in this file's tests
//! name the games). The parameter games' generator is version 1 ([`PARAMS_GENERATOR`]): a released level never
//! changes its items, and a rule change is a new version.

use core::fmt;

use enth_dsp::util::rng::{derive, fnv1a64};
use enth_patch::{ParamAddr, Patch};

use crate::model::{AnswerKind, Challenge, ChallengeError, Phrase, PhraseId, ScoringSpec, Start};
use crate::phrases::HELD;
use crate::priors::{Jnd, jnd};
use crate::random::{self, quantize};
use crate::v2::{Draw, DrawAt, DrawItem, DrawValue, DrawValues, MAX_ATTEMPTS, Selector};
use crate::v3::{EarError, EarSpec};

/// Game 1: Spot the change.
pub const SPOT_THE_CHANGE: u8 = 1;
/// Game 2: Dial it in (a constrained Recreate round, never an ear target).
pub const DIAL_IT_IN: u8 = 2;
/// Game 3: Ear check.
pub const EAR_CHECK: u8 = 3;

/// The newest parameter-games generator this build has: version 1.
pub const PARAMS_GENERATOR: u8 = 1;

/// The phrase every parameter-game clip plays: the held one-note phrase.
pub const PARAMS_PHRASE: PhraseId = HELD;

/// The template every parameter-game item starts from: 0, the starter ("Init saw").
pub const PARAMS_TEMPLATE: u8 = 0;

/// How many intervals an Ear check trial plays.
pub const EAR_CHECK_INTERVALS: u8 = 3;

/// Whether `game` is a parameter game (generated here, not by `enth-music`).
pub fn is_params_game(game: u8) -> bool {
    (SPOT_THE_CHANGE..=EAR_CHECK).contains(&game)
}

/// One level of a parameter game: what its items may draw.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParamLevel {
    /// How many option controls Spot the change offers (2 to 6); 1 for Dial it in and Ear check, which move one.
    pub options: u8,
    /// How far the moved control moves, in its parameter steps ([`crate::priors::jnd`]).
    pub steps: u8,
    /// The controls items draw from, as parameter addresses on the base.
    pub pool: &'static [&'static str],
    /// Whether Spot the change's options come from different modules (level 1's "filter against envelope").
    pub distinct_modules: bool,
    /// Dial it in's pass mark (sound %): docs/ear.md's tolerance in JND units, as G4 maps it.
    pub pass_mark: u8,
}

/// Level 1's controls: the filter's cutoff and resonance, the amp envelope's decay and release, each heard on the
/// held note of the starter.
const LEVEL_1_POOL: &[&str] = &[
    "filter1.cutoff",
    "filter1.resonance",
    "env1.decay",
    "env1.release",
];

/// Spot the change, level 1: two far options from different modules (filter against envelope), 6 steps apart.
pub const SPOT_LEVEL_1: ParamLevel = ParamLevel {
    options: 2,
    steps: 6,
    pool: LEVEL_1_POOL,
    distinct_modules: true,
    pass_mark: crate::model::DEFAULT_PASS_MARK,
};

/// Dial it in, level 1: one control 6 steps off, the default pass mark.
pub const DIAL_LEVEL_1: ParamLevel = ParamLevel {
    options: 1,
    steps: 6,
    pool: LEVEL_1_POOL,
    distinct_modules: false,
    pass_mark: crate::model::DEFAULT_PASS_MARK,
};

/// Ear check, level 1: one control, the odd interval 8 steps off (the staircase's start).
pub const CHECK_LEVEL_1: ParamLevel = ParamLevel {
    options: 1,
    steps: 8,
    pool: LEVEL_1_POOL,
    distinct_modules: false,
    pass_mark: crate::model::DEFAULT_PASS_MARK,
};

/// The level `level` of parameter game `game` in this build, or `None` above its top. Level 1 of each game until
/// G4's ladder lands.
pub fn level(game: u8, level: u8) -> Option<ParamLevel> {
    match (game, level) {
        (SPOT_THE_CHANGE, 1) => Some(SPOT_LEVEL_1),
        (DIAL_IT_IN, 1) => Some(DIAL_LEVEL_1),
        (EAR_CHECK, 1) => Some(CHECK_LEVEL_1),
        _ => None,
    }
}

/// The top level of parameter game `game` in this build.
pub fn top_level(game: u8) -> u8 {
    (1..=u8::MAX)
        .take_while(|&l| level(game, l).is_some())
        .last()
        .unwrap_or(0)
}

/// The answer a parameter game asks for, after checking its generator and level (`code_too_new`).
pub(crate) fn answer_kind(spec: &EarSpec) -> Result<AnswerKind, EarError> {
    if spec.generator > PARAMS_GENERATOR {
        return Err(EarError::GeneratorTooNew {
            game: spec.game,
            generator: spec.generator,
            latest: PARAMS_GENERATOR,
        });
    }
    if level(spec.game, spec.level).is_none() {
        return Err(EarError::LevelTooNew {
            game: spec.game,
            level: spec.level,
            top: top_level(spec.game),
        });
    }
    Ok(if spec.game == DIAL_IT_IN {
        AnswerKind::Patch
    } else {
        AnswerKind::Choice
    })
}

/// Why a parameter game's item cannot be made.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ParamError {
    /// The spec is refused ([`super::validate`]).
    Spec(EarError),
    /// The game is not a parameter game, or not this function's.
    WrongGame {
        /// The game.
        game: u8,
    },
    /// The level's pool has fewer observable controls on the base than the level needs: a ladder bug.
    Pool {
        /// What is missing.
        reason: String,
    },
    /// No attempt gave a playable item (every control stopped short of the distance, or the moved patch was
    /// unplayable): a ladder bug, never a player's.
    NoItem {
        /// The game.
        game: u8,
        /// The item.
        index: u8,
    },
}

impl ParamError {
    /// A stable snake-case code.
    pub fn code(&self) -> &'static str {
        match self {
            ParamError::Spec(e) => e.code(),
            ParamError::WrongGame { .. } => "bad_answer",
            ParamError::Pool { .. } | ParamError::NoItem { .. } => "draw_failed",
        }
    }
}

impl fmt::Display for ParamError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParamError::Spec(e) => write!(f, "{e}"),
            ParamError::WrongGame { game } => write!(f, "game {game} has no such item"),
            ParamError::Pool { reason } => f.write_str(reason),
            ParamError::NoItem { game, index } => {
                write!(f, "game {game} found no playable item {index}")
            }
        }
    }
}

impl std::error::Error for ParamError {}

/// A Spot the change item: the options in the order shown, which one moved, and the draw that moves it.
#[derive(Clone, Debug, PartialEq)]
pub struct SpotItem {
    /// The item's index in its set.
    pub index: u8,
    /// The option controls, in the order shown.
    pub options: Vec<ParamAddr>,
    /// The index of the option that moved.
    pub key: u8,
    /// The one-item draw that moves it over the base (a one-value set).
    pub draw: Draw,
    /// How far it moved, in its parameter steps (measured after quantization).
    pub steps: f64,
}

/// An Ear check trial: three intervals of one control, one moved.
#[derive(Clone, Debug, PartialEq)]
pub struct OddOneOut {
    /// The trial's index in its set.
    pub index: u8,
    /// The control.
    pub control: ParamAddr,
    /// The position of the moved interval, 0 to 2.
    pub position: u8,
    /// The one-item draw that moves it over the base.
    pub draw: Draw,
    /// How far it moved, in its parameter steps.
    pub steps: f64,
}

/// The base every item starts from: template 0's frozen start.
pub fn base() -> Patch {
    random::start(PARAMS_TEMPLATE).expect("template 0 is the frozen starter (tested)")
}

/// The phrase every clip plays.
pub fn phrase() -> &'static Phrase {
    crate::phrases::phrase(PARAMS_PHRASE).expect("the held phrase is in the table")
}

/// Item `index` of `spec` (game 1): Spot the change.
pub fn spot_the_change(spec: &EarSpec, index: u8) -> Result<SpotItem, ParamError> {
    let lvl = checked(spec, SPOT_THE_CHANGE)?;
    let (base, phrase) = (base(), phrase());
    let pool = observable_pool(&lvl, &base, phrase)?;
    let item = item_stream(spec, index);
    for attempt in 0..MAX_ATTEMPTS {
        let bits = derive(derive(item, fnv1a64(b"spot")), u64::from(attempt));
        let Some(options) = pick_options(&pool, &lvl, &base, derive(bits, fnv1a64(b"options")))
        else {
            return Err(ParamError::Pool {
                reason: format!(
                    "Spot the change level {} needs {} options{} from {:?}",
                    spec.level,
                    lvl.options,
                    if lvl.distinct_modules {
                        " from different modules"
                    } else {
                        ""
                    },
                    lvl.pool
                ),
            });
        };
        let key = (derive(bits, fnv1a64(b"key")) % u64::from(lvl.options)) as u8;
        let up = derive(bits, fnv1a64(b"dir")) & 1 == 1;
        let moved = &options[usize::from(key)];
        if let Some((draw, steps)) = move_control(&base, phrase, moved, lvl.steps, up, spec.seed) {
            return Ok(SpotItem {
                index,
                options,
                key,
                draw,
                steps,
            });
        }
    }
    Err(ParamError::NoItem {
        game: SPOT_THE_CHANGE,
        index,
    })
}

/// The control an Ear check set of `spec` (game 3) measures: one for the whole set, drawn from the level's observable
/// pool by `derive(seed, fnv1a64("3/control/level"))`.
pub fn ear_check_control(spec: &EarSpec) -> Result<ParamAddr, ParamError> {
    let lvl = checked(spec, EAR_CHECK)?;
    let pool = observable_pool(&lvl, &base(), phrase())?;
    let bits = derive(
        spec.seed,
        fnv1a64(format!("{EAR_CHECK}/control/{}", spec.level).as_bytes()),
    );
    Ok(pool[(bits % pool.len() as u64) as usize].clone())
}

/// Trial `index` of an Ear check set of `spec` (game 3) on its control ([`ear_check_control`]).
pub fn ear_check(spec: &EarSpec, index: u8) -> Result<OddOneOut, ParamError> {
    let lvl = checked(spec, EAR_CHECK)?;
    let control = ear_check_control(spec)?;
    ear_check_on(&control, lvl.steps, spec, index)
}

/// An Ear check trial on `control` at `steps` (a staircase's trial: the drill player picks the control and the
/// distance, and `spec` keys the trial's stream). The control must be observable on the base.
pub fn ear_check_on(
    control: &ParamAddr,
    steps: u8,
    spec: &EarSpec,
    index: u8,
) -> Result<OddOneOut, ParamError> {
    let (base, phrase) = (base(), phrase());
    if !random::observable(&base, phrase).is_observable(control) {
        return Err(ParamError::Pool {
            reason: format!("{control} is not heard on the base"),
        });
    }
    let item = item_stream(spec, index);
    for attempt in 0..MAX_ATTEMPTS {
        let bits = derive(derive(item, fnv1a64(b"check")), u64::from(attempt));
        let position = (derive(bits, fnv1a64(b"position")) % u64::from(EAR_CHECK_INTERVALS)) as u8;
        let up = derive(bits, fnv1a64(b"dir")) & 1 == 1;
        if let Some((draw, measured)) = move_control(&base, phrase, control, steps, up, spec.seed) {
            return Ok(OddOneOut {
                index,
                control: control.clone(),
                position,
                draw,
                steps: measured,
            });
        }
    }
    Err(ParamError::NoItem {
        game: EAR_CHECK,
        index,
    })
}

/// Round `index` of a Dial it in set of `spec` (game 2): a v2 challenge on a one-item draw over the base, the control
/// `editable` (the round's one knob), heard through the held phrase, scored by this build's Recreate scorers at the
/// level's pass mark. Validated; it writes `enth.challenge/2`.
pub fn dial_it_in(spec: &EarSpec, index: u8) -> Result<Challenge, ParamError> {
    let lvl = checked(spec, DIAL_IT_IN)?;
    let (base, phrase) = (base(), phrase());
    let pool = observable_pool(&lvl, &base, phrase)?;
    let item = item_stream(spec, index);
    for attempt in 0..MAX_ATTEMPTS {
        let bits = derive(derive(item, fnv1a64(b"dial")), u64::from(attempt));
        let control = &pool[(derive(bits, fnv1a64(b"control")) % pool.len() as u64) as usize];
        let up = derive(bits, fnv1a64(b"dir")) & 1 == 1;
        let Some((draw, _)) = move_control(&base, phrase, control, lvl.steps, up, spec.seed) else {
            continue;
        };
        let mut scoring = ScoringSpec::recreate(0);
        scoring.pass_mark = lvl.pass_mark;
        let mut c = Challenge::new(
            crate::model::Target::Draw(draw),
            Start::Template {
                template: PARAMS_TEMPLATE,
            },
            PARAMS_PHRASE,
            scoring,
        );
        c.constraints.editable = vec![Selector::Param(control.clone())];
        if c.validate().is_ok() {
            return Ok(c);
        }
    }
    Err(ParamError::NoItem {
        game: DIAL_IT_IN,
        index,
    })
}

/// The level of `spec`, after checking it is `game`'s and this build plays it.
fn checked(spec: &EarSpec, game: u8) -> Result<ParamLevel, ParamError> {
    if spec.game != game {
        return Err(ParamError::WrongGame { game: spec.game });
    }
    super::validate(spec).map_err(ParamError::Spec)?;
    level(spec.game, spec.level).ok_or(ParamError::Spec(EarError::LevelTooNew {
        game: spec.game,
        level: spec.level,
        top: top_level(spec.game),
    }))
}

/// Item `index`'s stream: `derive(seed, fnv1a64("game/index/level"))`.
fn item_stream(spec: &EarSpec, index: u8) -> u64 {
    derive(
        spec.seed,
        fnv1a64(format!("{}/{index}/{}", spec.game, spec.level).as_bytes()),
    )
}

/// The level's pool, as addresses observable on the base through the phrase, in the pool's order.
fn observable_pool(
    lvl: &ParamLevel,
    base: &Patch,
    phrase: &Phrase,
) -> Result<Vec<ParamAddr>, ParamError> {
    let mask = random::observable(base, phrase);
    let mut out = Vec::with_capacity(lvl.pool.len());
    for text in lvl.pool {
        let addr: ParamAddr = text.parse().map_err(|e| ParamError::Pool {
            reason: format!("{text}: {e}"),
        })?;
        if base.spec(&addr).is_some() && mask.is_observable(&addr) {
            out.push(addr);
        }
    }
    if out.len() < usize::from(lvl.options.max(1)) {
        return Err(ParamError::Pool {
            reason: format!(
                "{} observable controls in {:?}, {} needed",
                out.len(),
                lvl.pool,
                lvl.options
            ),
        });
    }
    Ok(out)
}

/// `options` controls from the pool, shuffled by `bits` (Fisher-Yates over keyed draws), taking the first from each
/// module when the level asks for different modules; `None` when the pool cannot give that many.
fn pick_options(
    pool: &[ParamAddr],
    lvl: &ParamLevel,
    base: &Patch,
    bits: u64,
) -> Option<Vec<ParamAddr>> {
    let mut order: Vec<&ParamAddr> = pool.iter().collect();
    for i in (1..order.len()).rev() {
        let j = (derive(bits, i as u64) % (i as u64 + 1)) as usize;
        order.swap(i, j);
    }
    let mut out: Vec<ParamAddr> = Vec::with_capacity(usize::from(lvl.options));
    for addr in order {
        if out.len() == usize::from(lvl.options) {
            break;
        }
        if lvl.distinct_modules && out.iter().any(|o| o.target() == addr.target()) {
            continue;
        }
        let _ = base;
        out.push(addr.clone());
    }
    (out.len() == usize::from(lvl.options)).then_some(out)
}

/// The control moved `steps` of its parameter steps from the base, up or down (the other way when the range stops
/// it first), quantized; the one-item draw that sets it, and the steps it measures. `None` when neither way reaches
/// the distance or the moved patch is unplayable through the phrase.
fn move_control(
    base: &Patch,
    phrase: &Phrase,
    control: &ParamAddr,
    steps: u8,
    up: bool,
    seed: u64,
) -> Option<(Draw, f64)> {
    let spec = base.spec(control)?;
    let from = base.value(control)?;
    let step = jnd(spec);
    for dir in if up { [1.0, -1.0] } else { [-1.0, 1.0] } {
        let to = quantize(spec, moved(step, from, f64::from(steps) * dir));
        let measured = step.steps(from, to);
        // Quantization to three significant digits may shave a little off; a tenth of a step is the most it may.
        if !measured.is_finite() || measured < f64::from(steps) - 0.1 {
            continue;
        }
        let draw = Draw {
            seed: seed & crate::spec::MAX_SEED,
            items: vec![DrawItem {
                at: DrawAt::Param(control.clone()),
                values: DrawValues::Set(vec![DrawValue::Number(to)]),
            }],
        };
        if let Ok(d) = draw.draw(base, phrase)
            && !d.forced
            && d.patch.value(control) == Some(to)
        {
            return Some((draw, measured));
        }
    }
    None
}

/// `from` moved by `k` steps (signed) of `step`, before quantization: the inverse of [`Jnd::steps`].
fn moved(step: Jnd, from: f32, k: f64) -> f32 {
    let a = f64::from(from);
    let v = match step {
        Jnd::Any => a,
        Jnd::Abs(s) => a + k * f64::from(s),
        Jnd::Ratio { factor, floor } => {
            let f = f64::from(floor);
            (a + f) * libm::pow(f64::from(factor), k) - f
        }
        Jnd::Semitones(st) => a * libm::pow(2.0, k * f64::from(st) / 12.0),
        Jnd::Db { db, floor } => {
            let f = f64::from(floor);
            (a + f) * libm::pow(10.0, k * f64::from(db) / 20.0) - f
        }
    };
    v as f32
}

/// The challenge error of a parameter item that cannot be made, for callers that prepare a set.
pub(crate) fn as_challenge_error(e: ParamError) -> ChallengeError {
    match e {
        ParamError::Spec(e) => ChallengeError::Ear(e),
        other => ChallengeError::BadChallenge {
            message: other.to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    use std::fmt::Write as _;

    fn spec(game: u8, seed: u64, count: u8) -> EarSpec {
        EarSpec {
            game,
            generator: 1,
            level: 1,
            seed,
            count,
        }
    }

    #[test]
    fn level_1_is_playable_and_only_level_1_exists_until_g4() {
        for game in [SPOT_THE_CHANGE, DIAL_IT_IN, EAR_CHECK] {
            assert_eq!(top_level(game), 1);
            let lvl = level(game, 1).unwrap();
            let pool = observable_pool(&lvl, &base(), phrase()).unwrap();
            assert_eq!(
                pool.len(),
                LEVEL_1_POOL.len(),
                "every level-1 control is heard"
            );
        }
        assert_eq!(level(SPOT_THE_CHANGE, 2), None);
        assert_eq!(
            super::super::validate(&spec(SPOT_THE_CHANGE, 1, 1)),
            Ok(AnswerKind::Choice)
        );
        assert_eq!(
            super::super::validate(&spec(EAR_CHECK, 1, 1)),
            Ok(AnswerKind::Choice)
        );
        assert_eq!(
            super::super::validate(&spec(DIAL_IT_IN, 1, 1)),
            Ok(AnswerKind::Patch)
        );
    }

    #[test]
    fn a_spot_item_moves_one_option_by_the_levels_distance() {
        let base = base();
        for seed in 0..200 {
            let s = spec(SPOT_THE_CHANGE, seed, 16);
            for index in 0..4 {
                let item = spot_the_change(&s, index).unwrap();
                assert_eq!(item.options.len(), 2);
                assert_ne!(
                    item.options[0].target(),
                    item.options[1].target(),
                    "filter against envelope"
                );
                let moved = &item.options[usize::from(item.key)];
                assert_eq!(item.draw.items.len(), 1);
                assert_eq!(item.draw.items[0].at, DrawAt::Param(moved.clone()));
                assert!(item.steps >= 5.9, "{seed}/{index}: {} steps", item.steps);
                let b = item.draw.draw(&base, phrase()).unwrap().patch;
                // B differs from A in the moved control only.
                for addr in &item.options {
                    let differs = b.value(addr) != base.value(addr);
                    assert_eq!(differs, addr == moved, "{seed}/{index} {addr}");
                }
                assert_eq!(enth_patch::ops_between(&base, &b).map(|o| o.len()), Some(1));
            }
        }
    }

    #[test]
    fn items_are_deterministic_and_keyed() {
        let s = spec(SPOT_THE_CHANGE, 42, 16);
        assert_eq!(spot_the_change(&s, 3), spot_the_change(&s, 3));
        // An item's stream is its own: another count or another item does not move it.
        let more = EarSpec { count: 40, ..s };
        assert_eq!(spot_the_change(&s, 3), spot_the_change(&more, 3));
    }

    #[test]
    fn no_position_leaks_the_key() {
        // Over many items the moved option sits in each position about as often, and moves up and down.
        let (mut first, mut up, mut n) = (0u32, 0u32, 0u32);
        let base = base();
        for seed in 0..500 {
            let item = spot_the_change(&spec(SPOT_THE_CHANGE, seed, 1), 0).unwrap();
            first += u32::from(item.key == 0);
            let moved = &item.options[usize::from(item.key)];
            let DrawValues::Set(v) = &item.draw.items[0].values else {
                panic!()
            };
            let DrawValue::Number(to) = v[0] else {
                panic!()
            };
            up += u32::from(to > base.value(moved).unwrap());
            n += 1;
        }
        assert!((200..=300).contains(&first), "first {first} of {n}");
        assert!((150..=350).contains(&up), "up {up} of {n}");
    }

    #[test]
    fn an_ear_check_set_measures_one_control() {
        let s = spec(EAR_CHECK, 9, 40);
        let control = ear_check_control(&s).unwrap();
        let mut positions = [0u32; 3];
        for index in 0..40 {
            let t = ear_check(&s, index).unwrap();
            assert_eq!(t.control, control);
            assert!(t.steps >= 7.9, "{}", t.steps);
            positions[usize::from(t.position)] += 1;
        }
        assert!(positions.iter().all(|&p| p >= 5), "{positions:?}");
        // A staircase trial on a chosen control and distance.
        let cutoff: ParamAddr = "filter1.cutoff".parse().unwrap();
        let t = ear_check_on(&cutoff, 2, &s, 0).unwrap();
        assert!(t.steps >= 1.9 && t.steps < 2.5, "{}", t.steps);
        let unheard: ParamAddr = "env2.decay".parse().unwrap();
        assert_eq!(
            ear_check_on(&unheard, 2, &s, 0).unwrap_err().code(),
            "draw_failed"
        );
    }

    #[test]
    fn dial_it_in_is_a_v2_draw_round() {
        for seed in 0..20 {
            let c = dial_it_in(&spec(DIAL_IT_IN, seed, 8), 0).unwrap();
            assert_eq!(c.version(), 2);
            assert_eq!(c.answer, AnswerKind::Patch);
            assert_eq!(c.constraints.editable.len(), 1);
            assert_eq!(Challenge::from_json(&c.to_json()), Ok(c.clone()));
        }
        assert_eq!(
            dial_it_in(&spec(SPOT_THE_CHANGE, 1, 8), 0)
                .unwrap_err()
                .code(),
            "bad_answer"
        );
    }

    #[test]
    fn a_newer_level_or_generator_is_refused() {
        let s = EarSpec {
            level: 2,
            ..spec(SPOT_THE_CHANGE, 1, 1)
        };
        assert_eq!(spot_the_change(&s, 0).unwrap_err().code(), "code_too_new");
        let s = EarSpec {
            generator: 2,
            ..spec(SPOT_THE_CHANGE, 1, 1)
        };
        assert_eq!(spot_the_change(&s, 0).unwrap_err().code(), "code_too_new");
    }

    /// The items are pinned: 100 seeds × 4 items of each game hash to one value. A change here is a change of what a
    /// released spec plays; until G4's ladder freezes with ear generator v1 (INT4's friend build), a deliberate
    /// change updates this pin and the commit names the games and why.
    #[test]
    fn the_items_are_pinned() {
        let mut h = Sha256::new();
        for seed in 0..100u64 {
            let seed = (seed * 0x9E37_79B9) & crate::spec::MAX_SEED;
            for index in 0..4 {
                let s = spot_the_change(&spec(SPOT_THE_CHANGE, seed, 4), index).unwrap();
                h.update(format!("{:?}|{}|", s.options, s.key));
                h.update(serde_json::to_string(&s.draw).unwrap());
                let e = ear_check(&spec(EAR_CHECK, seed, 4), index).unwrap();
                h.update(format!("{}|{}|", e.control, e.position));
                h.update(serde_json::to_string(&e.draw).unwrap());
                let d = dial_it_in(&spec(DIAL_IT_IN, seed, 4), index).unwrap();
                h.update(d.to_json());
            }
        }
        let hex = h.finalize().iter().fold(String::new(), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        });
        assert_eq!(hex, PARAMS_ITEMS_V1);
    }

    /// [`the_items_are_pinned`]'s hash.
    const PARAMS_ITEMS_V1: &str =
        "b8e57cb7f4031b01699859c23ab09add86fd52df323e3efcacc4cbf216cd2725";
}
