//! Unit tests of the generator machinery (lane RG). The properties P2-P5, the mask cases and the golden are
//! integration tests (`tests/rg_*.rs`); keyed-stream stability (P6) needs the table, so it lives here.

use super::*;
use crate::priors::v1::SLOTS;
use enth_patch::canon::exact_hash;

fn spec(seed: u64, difficulty: u8, musical: u8) -> RandomSpec {
    RandomSpec {
        generator: GENERATOR_V1,
        seed,
        difficulty,
        amounts: [None; Category::COUNT],
        musical,
        template: TEMPLATE_STARTER,
    }
}

#[test]
fn levels_follow_the_difficulty() {
    let cases = [
        (0, 1),
        (20, 1),
        (21, 2),
        (40, 2),
        (41, 3),
        (60, 3),
        (61, 4),
        (80, 4),
        (81, 5),
        (100, 5),
        (255, 5),
    ];
    for (d, l) in cases {
        assert_eq!(level(d), l, "difficulty {d}");
    }
    // CT's presets sit one per level, in order.
    let levels: Vec<u8> = crate::spec::PRESETS
        .iter()
        .map(|p| level(p.difficulty))
        .collect();
    assert_eq!(levels, [1, 2, 3, 4, 5]);
    let unlocked: Vec<u8> = Category::ALL.iter().map(|c| unlock_level(*c)).collect();
    assert_eq!(unlocked, [1, 2, 3, 4, 5, 5]);
}

#[test]
fn reachable_rounding() {
    assert_eq!(sig3(1234.5), 1230.0);
    assert_eq!(sig3(0.012_345), 0.0123);
    assert_eq!(sig3(-17.66), -17.7);
    assert_eq!(sig3(99.96), 100.0);
    assert_eq!(sig3(0.0), 0.0);
    assert!(sig3(-0.000_1).is_sign_negative() || sig3(-0.000_1) == -0.0001);
    assert_eq!(depth_step(12.4), 10.0);
    assert_eq!(depth_step(12.6), 15.0);
    assert_eq!(depth_step(-2.0), -5.0);
    assert_eq!(depth_step(1.0), 5.0);
    assert_eq!(depth_step(140.0), 100.0);
}

/// The frozen starter is the F0r starter: it loads, it is valid, and its hash is pinned (a change to it moves every
/// v1 target, so it never changes).
#[test]
fn the_frozen_starter_is_pinned() {
    let p = start(TEMPLATE_STARTER).unwrap();
    enth_patch::validate::validate(&p).unwrap();
    assert_eq!(
        exact_hash(&p).to_hex(),
        "af7ab509c8d3f7e95088d5b779150136d90c0945c8890d354930c6bca3257cb0"
    );
    assert_eq!(start(1), Err(GenError::UnknownTemplate { template: 1 }));
}

#[test]
fn bad_specs_are_refused() {
    // The spec's own check runs first: refused, never clamped.
    let mut s = spec(1, 50, 100);
    s.generator = LATEST_GENERATOR + 1;
    assert_eq!(target(&s).unwrap_err().code(), "code_too_new");
    s.generator = 0;
    assert_eq!(target(&s).unwrap_err().code(), "no_generator");
    let mut s = spec(1, 101, 100);
    assert_eq!(
        target(&s).unwrap_err(),
        GenError::Spec(s.validate().unwrap_err())
    );
    s.difficulty = 50;
    s.amounts[3] = Some(200);
    let e = target(&s).unwrap_err();
    assert_eq!(e.code(), "out_of_range");
    assert!(e.to_string().contains("amounts.modulation"), "{e}");
    assert_ne!(e.hint(), "");
    let mut s = spec(1, 50, 101);
    assert_eq!(target(&s).unwrap_err().code(), "out_of_range");
    s.musical = 100;
    s.template = 9;
    assert_eq!(target(&s).unwrap_err().code(), "unknown_template");
}

/// Every address, id and source in the table parses and exists in the starter, with the kind the slot expects.
#[test]
fn the_table_resolves_against_the_starter() {
    let p = start(TEMPLATE_STARTER).unwrap();
    for slot in SLOTS {
        match &slot.what {
            What::Param { addr, .. } => {
                let a: ParamAddr = addr.parse().unwrap();
                assert!(p.spec(&a).is_some(), "{addr}");
            }
            What::Enable { id, percent } => {
                assert!(p.find(&ModuleId::new(id).unwrap()).is_some(), "{id}");
                assert!(*percent <= 100);
            }
            What::Route { src, .. } => {
                let s: ModSource = src.parse().unwrap();
                assert!(p.source_rate(&s).is_some(), "{src}");
            }
        }
    }
    // Each category has a default set with at least one counted slot.
    for c in Category::ALL {
        assert!(
            SLOTS.iter().any(|s| s.category == c && s.default && s.step),
            "{c:?}"
        );
    }
}

/// Generation is a pure function of (spec, start).
#[test]
fn generation_is_deterministic() {
    for seed in 0..20 {
        let s = spec(
            seed,
            (seed * 13 % 101) as u8,
            if seed % 3 == 0 { 0 } else { 100 },
        );
        let a = target(&s).unwrap();
        let b = target(&s).unwrap();
        assert!(enth_patch::canon::identical(&a.patch, &b.patch));
        assert_eq!(a.randomized, b.randomized);
        assert_eq!(a.phrase, b.phrase);
    }
}

/// The lowest level is a tutorial: osc 1's wave and octave (and a pulse width), never the start.
#[test]
fn level_one_draws_osc1_wave_and_octave() {
    let base = start(TEMPLATE_STARTER).unwrap();
    for seed in 0..200 {
        let g = target(&spec(seed, 0, 100)).unwrap();
        let params: Vec<String> = g.randomized_params().map(ToString::to_string).collect();
        assert!(params.len() >= 2 && params.len() <= 3, "{params:?}");
        assert_eq!(&params[..2], ["osc1.wave", "osc1.semi"]);
        assert_eq!(g.phrase, phrases::HELD);
        let differs = ["osc1.wave", "osc1.semi", "osc1.pw"].iter().any(|a| {
            let a: ParamAddr = a.parse().unwrap();
            base.value(&a) != g.patch.value(&a)
        });
        assert!(differs, "seed {seed}: the target is the start");
    }
}

/// P6, keyed-stream stability: with an extra test parameter in the table, every shared parameter draws the same
/// value. The extra goes in the middle of the table, uncounted (as a level-5-dependent draw would be) and counted
/// (then only its own category may redraw).
#[test]
fn p6_an_extra_parameter_never_moves_the_others() {
    let extra = |counted: bool| Slot {
        what: What::Param {
            addr: "osc1.fine",
            prior: |_, d| 50.0 * d.u() - 25.0,
            uniform: |_| Domain::Range(-100.0, 100.0),
        },
        category: Category::Oscillators,
        default: true,
        step: counted,
        when: always,
    };
    for counted in [false, true] {
        let mut table: Vec<Slot> = SLOTS.to_vec();
        let at = table
            .iter()
            .position(|s| {
                matches!(
                    s.what,
                    What::Param {
                        addr: "osc1.semi",
                        ..
                    }
                )
            })
            .unwrap();
        table.insert(at + 1, extra(counted));
        let mut compared = 0;
        for seed in 0..300u64 {
            let s = spec(
                seed,
                (seed * 7 % 101) as u8,
                [100, 100, 50, 0][seed as usize % 4],
            );
            let base = start(TEMPLATE_STARTER).unwrap();
            let a = generate_with(&s, &base, SLOTS).unwrap();
            let b = generate_with(&s, &base, &table).unwrap();
            assert_eq!(a.phrase, b.phrase);
            let osc_rounds = |g: &Generated| {
                g.steps
                    .iter()
                    .find(|st| st.category == Category::Oscillators)
                    .map(|st| st.rounds)
            };
            // A counted extra joins the oscillators' minimum step, so it may change their attempts, and the filter
            // prior follows osc 1's pitch: compare those seeds only where the step was met at once in both.
            if counted && !(osc_rounds(&a) == Some(1) && osc_rounds(&b) == Some(1)) {
                continue;
            }
            for r in &a.randomized {
                assert!(b.randomized.contains(r), "seed {seed}: {r:?} lost");
                if let Drawn::Param(addr) = &r.drawn {
                    assert_eq!(
                        a.patch.value(addr),
                        b.patch.value(addr),
                        "seed {seed}: {addr}"
                    );
                    compared += 1;
                }
            }
            for (ra, rb) in a.patch.routes.iter().zip(&b.patch.routes) {
                assert_eq!(ra, rb, "seed {seed}");
            }
        }
        assert!(compared > 1_000, "{compared}");
    }
}

/// P3 at the unit level: a higher difficulty keeps the lower categories' draws and values.
#[test]
fn higher_levels_keep_lower_values() {
    for seed in 0..100 {
        let mut prev: Option<Generated> = None;
        for d in [0, 20, 40, 60, 80, 100] {
            let g = target(&spec(seed, d, if seed % 2 == 0 { 100 } else { 40 })).unwrap();
            if let Some(p) = &prev {
                for r in &p.randomized {
                    assert!(g.randomized.contains(r), "seed {seed} d {d}: {r:?}");
                    if let Drawn::Param(a) = &r.drawn {
                        assert_eq!(p.patch.value(a), g.patch.value(a), "seed {seed} d {d}: {a}");
                    }
                }
            }
            prev = Some(g);
        }
    }
}

/// A narrow amount pulls values towards the start, so the step is forced, and still met.
#[test]
fn a_narrow_amount_forces_the_step() {
    let mut forced = 0;
    for seed in 0..50 {
        // One slot (env1.attack) at 1 % width: every draw stays within a step of the start.
        let mut s = spec(seed, 0, 100);
        s.amounts[Category::Envelopes.index()] = Some(1);
        let g = target(&s).unwrap();
        let step = g
            .steps
            .iter()
            .find(|st| st.category == Category::Envelopes)
            .unwrap();
        assert!(step.met, "seed {seed}: {step:?}");
        forced += usize::from(step.forced);
    }
    assert!(forced > 0);
}

/// The headroom rule: a second full-level source lowers `master.level`, and the master is never drawn.
#[test]
fn headroom_lowers_the_master_gauge() {
    let mut lowered = 0;
    for seed in 0..100 {
        let g = target(&spec(seed, 100, 100)).unwrap();
        let sum: f32 = g
            .patch
            .generators
            .iter()
            .filter(|m| m.enabled)
            .map(|m| m.get("level").unwrap() / 100.0)
            .sum();
        let gain = libm::powf(10.0, g.patch.master[0] / 20.0);
        assert!(sum * gain <= 1.0 + 1e-4, "seed {seed}: {sum} × {gain}");
        assert!(g.patch.master[0] > -12.5);
        lowered += usize::from(g.patch.master[0] < 0.0);
        assert!(
            !g.randomized_params()
                .any(|a| a.to_string() == "master.level")
        );
    }
    assert!(lowered > 20, "{lowered}");
}
