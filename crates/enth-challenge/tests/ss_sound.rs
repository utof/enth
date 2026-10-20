//! Lane SS: `sound/1`'s properties (docs/phase-2.md, "Test strategy for the game logic"; docs/challenge.md, "Sound
//! score (lane SS)").
//!
//! - **P7** scores stay in 0–100 and the target itself scores 100; equivalent patches (module ids relabelled, an
//!   extra module that is off, generators reordered) score as the target does.
//! - **P10** sound stability: a 1e-3 relative move of a parameter changes the score by under 0.5 point;
//!   a nuisance parameter (phase mode, start phase, seed) gives at least 99. These render, so 32 cases each.
//! - The score is monotone in an injected error, and the start of a round never passes.
//! - **P11** (with `ss_loudness.rs`): the A/B level match and the scored comparison share one gain function.
//!
//! One target per level is prepared once (`OnceLock`) and shared by every case.

use std::sync::OnceLock;

use enth_challenge::calibrate::{self, jittered_params, step};
use enth_challenge::loudness;
use enth_challenge::model::{SCORING_SAMPLE_RATE, SCORING_SEED};
use enth_challenge::random::{self, Generated, Visibility};
use enth_challenge::score::sound::{SoundPrepared, SoundV1, component_text};
use enth_challenge::{
    Challenge, Closeness, ComponentKind, Direction, Scorer, SoundScore, SpecPreset, TargetContext,
};
use enth_patch::addr::Position;
use enth_patch::{ModuleId, ModuleInstance, ModuleKind, Op, ParamAddr, Patch, Section};
use enth_render::{RenderSpec, render};
use proptest::prelude::*;

/// The seed of each level's shared target (a 40-bit seed, so its code is short).
const SEEDS: [u64; 5] = [2, 5, 7, 3, 1];

struct Round {
    generated: Generated,
    start: Patch,
    prepared: SoundPrepared,
}

fn round(level: usize) -> &'static Round {
    static ROUNDS: [OnceLock<Round>; 5] = [const { OnceLock::new() }; 5];
    ROUNDS[level - 1].get_or_init(|| {
        let spec = SpecPreset::by_id(level as u8)
            .unwrap()
            .spec(1, SEEDS[level - 1]);
        let generated = random::target(&spec).unwrap();
        let start = random::start(spec.template).unwrap();
        let challenge = Challenge::recreate(spec, generated.phrase);
        let prepared = SoundV1
            .prepare(&TargetContext {
                challenge: &challenge,
                target: &generated.patch,
                start: &start,
                phrase: generated.phrase(),
                randomized: &generated.params,
            })
            .unwrap();
        Round {
            generated,
            start,
            prepared,
        }
    })
}

fn score(level: usize, answer: &Patch) -> SoundScore {
    SoundV1.score(&round(level).prepared, answer).unwrap()
}

fn set(p: &Patch, addr: &ParamAddr, value: f32) -> Patch {
    let mut q = p.clone();
    enth_patch::apply(
        &mut q,
        Op::SetParam {
            addr: addr.clone(),
            value,
        },
    )
    .unwrap();
    q
}

/// `steps` calibration steps of `addr` from the target's value, up or down, clamped to the range.
fn moved(target: &Patch, addr: &ParamAddr, steps: usize, up: bool) -> Patch {
    let spec = target.spec(addr).unwrap();
    let s = step(spec).unwrap();
    let mut v = target.value(addr).unwrap();
    for _ in 0..steps {
        v = s.apply(v, up);
    }
    set(target, addr, v.clamp(spec.min(), spec.max()))
}

fn in_range(s: &SoundScore) {
    assert!(
        s.percent.is_finite() && (0.0..=100.0).contains(&s.percent),
        "{s:?}"
    );
    assert!(s.distance.is_finite() && s.distance >= 0.0, "{s:?}");
    assert!(s.jnd.is_finite() && s.jnd >= 0.0, "{s:?}");
    assert!(s.scale >= calibrate::SCALE_MIN && s.floor >= 0.0, "{s:?}");
}

#[test]
fn p7_the_target_scores_100_and_scores_stay_in_range() {
    for level in 1..=5 {
        let r = round(level);
        let s = score(level, &r.generated.patch);
        in_range(&s);
        assert_eq!(
            (s.percent, s.distance, s.jnd),
            (100.0, 0.0, 0.0),
            "level {level}"
        );
        assert!(
            s.components
                .iter()
                .all(|c| c.direction == Direction::Same && c.closeness == Closeness::Close),
            "{:?}",
            s.components
        );
        let start = score(level, &r.start);
        in_range(&start);
    }
}

#[test]
fn the_start_never_passes_and_reads_in_words() {
    for level in 1..=5 {
        let r = round(level);
        let s = score(level, &r.start);
        assert!(
            s.percent < 90.0,
            "level {level}: the start scores {}",
            s.percent
        );
        let text: Vec<String> = s.components.iter().map(component_text).collect();
        assert!(
            text.iter().any(|t| !t.ends_with("matches the target")),
            "level {level}: {text:?}"
        );
    }
}

/// `p` with every module renamed through a fresh bijection (`m1`, `m2`, … in module order), references included.
fn relabelled(p: &Patch) -> Patch {
    let ids: Vec<ModuleId> = p.modules().map(|(_, m)| m.id.clone()).collect();
    let f = |id: &ModuleId| {
        let i = ids.iter().position(|x| x == id).unwrap();
        ModuleId::numbered("m", i as u32 + 1).unwrap()
    };
    let mut q = p.clone();
    for section in Section::ALL {
        for m in q.section_mut(section) {
            m.id = f(&m.id);
        }
    }
    let map_addr = |a: &ParamAddr| match a.target() {
        enth_patch::addr::Target::Module(id) => {
            ParamAddr::new(enth_patch::addr::Target::Module(f(id)), a.param()).unwrap()
        }
        _ => a.clone(),
    };
    for r in &mut q.routes {
        if let enth_patch::addr::ModSource::Module(id) = &r.src {
            r.src = enth_patch::addr::ModSource::Module(f(id));
        }
        r.dst = map_addr(&r.dst);
    }
    for s in q.slots.iter_mut().flatten() {
        *s = map_addr(s);
    }
    q.voice.amp_env = f(&q.voice.amp_env);
    q
}

#[test]
fn p7_equivalent_patches_score_as_the_target() {
    for level in 1..=5 {
        let target = &round(level).generated.patch;
        // Every module under another id: the scorer maps them back before it renders.
        let s = score(level, &relabelled(target));
        assert!(
            (100.0 - s.percent) < 0.5,
            "level {level}, relabelled: {s:?}"
        );
        // An extra oscillator that is off.
        let mut extra = target.clone();
        let mut osc = ModuleInstance::new(ModuleId::new("osc9").unwrap(), ModuleKind::Osc);
        osc.enabled = false;
        let index = extra.generators.len();
        enth_patch::apply(
            &mut extra,
            Op::AddModule {
                at: Position {
                    section: Section::Generators,
                    index,
                },
                module: osc,
            },
        )
        .unwrap();
        let s = score(level, &extra);
        assert!(
            (100.0 - s.percent) < 0.5,
            "level {level}, extra module: {s:?}"
        );
        // The generators in another order, same ids.
        if target.generators.len() > 1 {
            let mut swapped = target.clone();
            let last = swapped.generators.last().unwrap().id.clone();
            enth_patch::apply(
                &mut swapped,
                Op::Move {
                    id: last,
                    to: Position {
                        section: Section::Generators,
                        index: 0,
                    },
                },
            )
            .unwrap();
            let s = score(level, &swapped);
            assert!((100.0 - s.percent) < 0.5, "level {level}, reordered: {s:?}");
        }
    }
}

/// Within this the score may wobble where a parameter has stopped mattering (a cutoff far above every harmonic);
/// P10's stability bound.
const WOBBLE: f64 = 0.5;

#[test]
fn the_score_is_monotone_in_an_injected_error() {
    for level in 1..=5 {
        let target = &round(level).generated.patch;
        let phrase = round(level).generated.phrase();
        let (params, _) = jittered_params(target, phrase);
        let oscillators = target.generators.iter().filter(|g| g.enabled).count();
        for key in ["cutoff", "fine", "release", "sustain", "decay"] {
            let Some(addr) = params.iter().find(|a| a.param() == key) else {
                continue;
            };
            // With two oscillators a fine move is a detune move, which the metric reads as decorrelation, not as
            // pitch, and not monotonically (report 02 §2.3): a known limit, see docs/challenge.md.
            if key == "fine" && oscillators > 1 {
                continue;
            }
            let mut lowest = 100.0f64;
            for up in [true, false] {
                let mut last = 100.0f64;
                let mut seen = Vec::new();
                for steps in [0usize, 1, 2, 4, 8, 16] {
                    let s = score(level, &moved(target, addr, steps, up));
                    seen.push((steps, s.percent));
                    assert!(
                        s.percent <= last + WOBBLE,
                        "level {level}, {addr} {}: not monotone: {seen:?}",
                        if up { "up" } else { "down" }
                    );
                    last = last.min(s.percent);
                }
                lowest = lowest.min(last);
            }
            // A decay towards a high sustain (the starter's 70 %) moves the level by 3 dB: the headline barely sees
            // it (the decay row does), so only these three must move it.
            if ["cutoff", "fine", "release"].contains(&key) {
                assert!(
                    lowest < 99.0,
                    "level {level}, {addr}: 16 steps either way changed nothing"
                );
            }
        }
    }
}

#[test]
fn components_name_the_error() {
    // Each level's target with its oscillator's wave moved between saw and triangle: a saw is brighter, the pitch
    // stays.
    let wave: ParamAddr = "osc1.wave".parse().unwrap();
    let (saw, triangle) = (2.0, 1.0);
    for level in 1..=5 {
        let target = &round(level).generated.patch;
        let w = target.value(&wave).unwrap();
        let (to, brighter) = if w == saw {
            (triangle, false)
        } else {
            (saw, true)
        };
        if w != saw && w != triangle {
            continue;
        }
        let s = score(level, &set(target, &wave, to));
        let row = s
            .components
            .iter()
            .find(|c| c.kind == ComponentKind::Brightness)
            .unwrap();
        let want = if brighter {
            Direction::Higher
        } else {
            Direction::Lower
        };
        assert_eq!(row.direction, want, "level {level}: {s:?}");
        let words = if brighter {
            "brighter than the target"
        } else {
            "darker than the target"
        };
        assert!(
            component_text(row).contains(words),
            "{}",
            component_text(row)
        );
        let pitch = s
            .components
            .iter()
            .find(|c| c.kind == ComponentKind::Pitch)
            .unwrap();
        assert_eq!(pitch.direction, Direction::Same, "level {level}");
        assert!(
            s.percent < 90.0,
            "level {level}: another wave passes: {s:?}"
        );
    }
    // A slower attack reads "attack ... too slow".
    let target = &round(3).generated.patch;
    let attack: ParamAddr = "env1.attack".parse().unwrap();
    let a = target.value(&attack).unwrap();
    let s = score(3, &set(target, &attack, (a * 8.0 + 40.0).min(1_500.0)));
    let row = s
        .components
        .iter()
        .find(|c| c.kind == ComponentKind::Attack)
        .unwrap();
    assert_eq!(row.direction, Direction::Higher, "{s:?}");
    assert!(
        component_text(row).contains("too slow"),
        "{}",
        component_text(row)
    );
}

#[test]
fn p11_the_ab_match_uses_the_scorers_loudness_and_never_boosts() {
    let r = round(3);
    let mid = |p: &Patch| {
        render(
            p,
            &r.generated.phrase().performance(),
            &RenderSpec::new(SCORING_SAMPLE_RATE, SCORING_SEED),
        )
        .unwrap()
        .mid()
    };
    let (t, m) = (mid(&r.generated.patch), mid(&r.start));
    let (lt, lm) = (
        loudness::integrated_lufs(&t, SCORING_SAMPLE_RATE),
        loudness::integrated_lufs(&m, SCORING_SAMPLE_RATE),
    );
    // The scorer's clip loudness is this measurement.
    assert_eq!(r.prepared.target_clip().lufs, lt);
    let (gt, gm) = loudness::ab_gains_db(lt, lm);
    assert!(gt <= 0.0 && gm <= 0.0 && (gt == 0.0 || gm == 0.0));
    let apply = |x: &[f32], db: f64| -> Vec<f32> {
        let a = loudness::db_to_amp(db) as f32;
        assert!(a <= 1.0, "never a boost");
        x.iter().map(|v| v * a).collect()
    };
    let (at, am) = (
        loudness::integrated_lufs(&apply(&t, gt), SCORING_SAMPLE_RATE).unwrap(),
        loudness::integrated_lufs(&apply(&m, gm), SCORING_SAMPLE_RATE).unwrap(),
    );
    assert!((at - am).abs() < 1e-3, "matched: {at} {am}");
}

/// The observable continuous parameters of a level's target, for the properties to pick from.
fn continuous(level: usize) -> &'static [ParamAddr] {
    static PARAMS: [OnceLock<Vec<ParamAddr>>; 5] = [const { OnceLock::new() }; 5];
    PARAMS[level - 1].get_or_init(|| {
        let r = round(level);
        jittered_params(&r.generated.patch, r.generated.phrase()).0
    })
}

/// The nuisance parameters of a level's target (phase modes, start phases, seeds of enabled modules).
fn nuisance(level: usize) -> &'static [ParamAddr] {
    static PARAMS: [OnceLock<Vec<ParamAddr>>; 5] = [const { OnceLock::new() }; 5];
    PARAMS[level - 1].get_or_init(|| {
        let r = round(level);
        random::observable(&r.generated.patch, r.generated.phrase())
            .params()
            .filter(|(_, v)| *v == Visibility::Nuisance)
            .map(|(a, _)| a.clone())
            .collect()
    })
}

/// A near miss per level: the target with its first jittered parameter two steps up. P10's moves start here too,
/// where the curve is steep.
fn near_miss(level: usize) -> &'static (Patch, f64) {
    static NEAR: [OnceLock<(Patch, f64)>; 5] = [const { OnceLock::new() }; 5];
    NEAR[level - 1].get_or_init(|| {
        let target = &round(level).generated.patch;
        let p = moved(target, &continuous(level)[0], 2, true);
        let s = score(level, &p).percent;
        (p, s)
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(32))]

    /// P10: a 1e-3 relative move of one parameter (its plain value × (1 ± 1e-3), or ± 1e-3 of its unit at 0), from
    /// the target or from a near miss, moves the score by under 0.5 point.
    #[test]
    fn p10_a_tiny_move_barely_moves_the_score(
        level in 1usize..=5,
        pick in any::<prop::sample::Index>(),
        up in any::<bool>(),
        from_near in any::<bool>(),
    ) {
        let (base, base_score) = if from_near {
            let (p, s) = near_miss(level);
            (p.clone(), *s)
        } else {
            (round(level).generated.patch.clone(), 100.0)
        };
        let params = continuous(level);
        let addr = &params[pick.index(params.len())];
        let spec = base.spec(addr).unwrap();
        let v0 = base.value(addr).unwrap();
        let d = if v0 == 0.0 { 1e-3 } else { 1e-3 * v0.abs() };
        let v = if up { v0 + d } else { v0 - d }.clamp(spec.min(), spec.max());
        let s = score(level, &set(&base, addr, v));
        prop_assert!(
            (s.percent - base_score).abs() < 0.5,
            "level {}, {} {} -> {}: {} -> {}", level, addr, base.value(addr).unwrap(), v, base_score, s.percent
        );
    }

    /// P10: a nuisance parameter set to another value scores at least 99.
    #[test]
    fn p10_a_nuisance_change_scores_at_least_99(
        level in 1usize..=5,
        pick in any::<prop::sample::Index>(),
        u in 0.0f32..1.0,
    ) {
        let params = nuisance(level);
        prop_assume!(!params.is_empty());
        let target = &round(level).generated.patch;
        let addr = &params[pick.index(params.len())];
        let spec = target.spec(addr).unwrap();
        let mut v = spec.from_norm(u).clamp(spec.min(), spec.max());
        if spec.is_discrete() {
            v = libm::roundf(v);
        }
        let s = score(level, &set(target, addr, v));
        prop_assert!(s.percent >= 99.0, "level {}, {} -> {}: {:?}", level, addr, v, s);
    }

    /// P11: the A/B gains are symmetric, never boost, match the two sides, and differ from the scorer's gains by one
    /// common offset (one gain function).
    #[test]
    fn p11_ab_gains_are_symmetric_and_never_boost(
        t in -90.0f64..20.0,
        m in -90.0f64..20.0,
        none in 0u8..4,
    ) {
        let (ot, om) = match none {
            1 => (None, Some(m)),
            2 => (Some(t), None),
            _ => (Some(t), Some(m)),
        };
        let (gt, gm) = loudness::ab_gains_db(ot, om);
        let (rt, rm) = loudness::ab_gains_db(om, ot);
        prop_assert_eq!((gt, gm), (rm, rt));
        prop_assert!(gt <= 0.0 && gm <= 0.0);
        if let (Some(t), Some(m)) = (ot, om) {
            prop_assert!(gt == 0.0 || gm == 0.0);
            prop_assert!(((t + gt) - (m + gm)).abs() < 1e-9);
            let st = loudness::gain_db(t, loudness::SCORING_LUFS);
            let sm = loudness::gain_db(m, loudness::SCORING_LUFS);
            prop_assert!(((gt - st) - (gm - sm)).abs() < 1e-9);
        } else {
            prop_assert_eq!((gt, gm), (0.0, 0.0));
        }
    }
}
