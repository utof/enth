//! Unit tests of the generator machinery (lane RG). The properties P2-P5, the mask cases and the golden are
//! integration tests (`tests/rg_*.rs`); keyed-stream stability (P6) needs the table, so it lives here.

use super::*;
use crate::priors::v1::SLOTS;
use enth_params::Kind;
use enth_patch::canon::canonical_hash;
use sha2::{Digest, Sha256};
use std::fmt::Write as _;

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

/// The frozen starter is the F0r starter: it loads, it is valid, and its bytes are pinned (a change to them moves
/// every v1 target, so they never change). The pin is the file's SHA-256, not a patch hash, so a patch schema bump
/// (which migrates the start on load) never trips it.
#[test]
fn the_frozen_starter_is_pinned() {
    let p = start(TEMPLATE_STARTER).unwrap();
    enth_patch::validate::validate(&p).unwrap();
    let digest: [u8; 32] = Sha256::digest(STARTER_V1.as_bytes()).into();
    let hex = digest.iter().fold(String::new(), |mut h, b| {
        let _ = write!(h, "{b:02x}");
        h
    });
    assert_eq!(
        hex,
        "24f9326d042869fe280aa687ac558f824f11486fd91838a7ddfba357900029cd"
    );
    assert_eq!(start(1), Err(GenError::UnknownTemplate { template: 1 }));
    // Every template up to the newest exists; the next is a newer build's.
    for t in 0..=LATEST_TEMPLATE {
        assert!(start(t).is_ok(), "template {t}");
    }
    assert!(start(LATEST_TEMPLATE + 1).is_err());
}

/// SG's host-automation rule, which RG follows (F1r review N3): the start patch has no slot assignments and no route
/// from a macro or the mod wheel, and no target adds one, so host automation cannot reach a round's patch.
#[test]
fn the_start_has_no_slots_and_no_macro_routes_and_targets_add_none() {
    let p = start(TEMPLATE_STARTER).unwrap();
    let controls = |p: &Patch| {
        p.routes
            .iter()
            .filter(|r| matches!(r.src, ModSource::Macro(_) | ModSource::ModWheel))
            .count()
    };
    assert!(p.slots.iter().all(Option::is_none), "{:?}", p.slots);
    assert_eq!(controls(&p), 0);
    for seed in 0..200 {
        let g = target(&spec(
            seed,
            (seed * 11 % 101) as u8,
            [100, 0][seed as usize % 2],
        ))
        .unwrap();
        assert_eq!(g.patch.slots, p.slots, "seed {seed}");
        assert_eq!(controls(&g.patch), 0, "seed {seed}");
    }
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
    // A template this build does not have is a newer build's: the spec's own check refuses it first.
    s.template = 9;
    assert_eq!(target(&s).unwrap_err().code(), "code_too_new");
    let base = start(TEMPLATE_STARTER).unwrap();
    assert_eq!(generate(&s, &base).unwrap_err().code(), "code_too_new");
    // A seed a JSON reader could change is refused (F1r review S4).
    let mut s = spec(1 << 53, 0, 100);
    assert_eq!(target(&s).unwrap_err().code(), "out_of_range");
    s.seed = crate::spec::MAX_SEED;
    assert!(target(&s).is_ok());
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

// ---------------------------------------------------------------------------------------------------------------
// The fingerprint (F1r review M1, T1) and the catalogue pin (M2, T2)
// ---------------------------------------------------------------------------------------------------------------

/// Whether the headroom rule set `master.level`: the start's master level differs from the target's.
fn headroom_set(g: &Generated) -> bool {
    let addr = ParamAddr::master(MasterParam::Level);
    start(TEMPLATE_STARTER).unwrap().value(&addr) != g.patch.value(&addr)
}

/// T1: the fingerprint is made of what the generator decided, and of nothing else. Re-encoding the target (a patch
/// file round trip, which a schema bump's migration is), or changing a value v1 never decides (a start phase: a
/// nuisance; a parameter a newer catalogue would add sits in the same place), moves the canonical hash and keeps the
/// fingerprint. The preimage holds no schema at all (`fingerprint::tests::the_v1_preimage_is_pinned`).
#[test]
fn t1_the_fingerprint_holds_only_what_the_generator_decided() {
    let phase: ParamAddr = "osc1.phase".parse().unwrap();
    let mut headrooms = 0;
    for seed in 0..60 {
        for d in [0, 50, 100] {
            let g = target(&spec(seed, d, [100, 0, 50][seed as usize % 3])).unwrap();
            let h = headroom_set(&g);
            headrooms += usize::from(h);
            assert_eq!(
                fingerprint_v1(g.generator, g.phrase, &g.randomized, &g.patch, h),
                g.fingerprint,
                "seed {seed} d {d}: recomputed"
            );
            let back = enth_patch::file::load(&g.patch.to_json()).unwrap().patch;
            assert_eq!(
                fingerprint_v1(g.generator, g.phrase, &g.randomized, &back, h),
                g.fingerprint,
                "seed {seed} d {d}: through the patch file"
            );
            let mut moved = g.patch.clone();
            let old = moved.value(&phase).unwrap();
            apply(
                &mut moved,
                Op::SetParam {
                    addr: phase.clone(),
                    value: if old == 90.0 { 45.0 } else { 90.0 },
                },
            )
            .unwrap();
            assert_ne!(canonical_hash(&moved), canonical_hash(&g.patch));
            assert_eq!(
                fingerprint_v1(g.generator, g.phrase, &g.randomized, &moved, h),
                g.fingerprint,
                "seed {seed} d {d}: an undecided value"
            );
        }
    }
    assert!(headrooms > 0, "the headroom entry is exercised");
}

/// T1, the table side: a slot added to a category the spec does not reach changes no target and no fingerprint
/// (keyed streams, P6), while a drawn value that differs is another fingerprint.
#[test]
fn t1_an_unreached_slot_keeps_every_fingerprint() {
    let mut table: Vec<Slot> = SLOTS.to_vec();
    table.push(Slot {
        what: What::Param {
            addr: "osc1.fine",
            prior: |_, d| 50.0 * d.u() - 25.0,
            uniform: |_| Domain::Range(-100.0, 100.0),
        },
        category: Category::Effects,
        default: true,
        step: false,
        when: always,
    });
    let base = start(TEMPLATE_STARTER).unwrap();
    let mut differ = 0;
    for seed in 0..100 {
        for d in [0, 30, 60, 80] {
            let s = spec(seed, d, 100);
            let a = generate_with(&s, &base, SLOTS).unwrap();
            let b = generate_with(&s, &base, &table).unwrap();
            assert_eq!(a.fingerprint, b.fingerprint, "seed {seed} d {d}");
        }
        let s = spec(seed, 100, 100);
        let a = generate_with(&s, &base, SLOTS).unwrap();
        let b = generate_with(&s, &base, &table).unwrap();
        differ += usize::from(a.fingerprint != b.fingerprint);
    }
    assert!(
        differ > 90,
        "a reached slot is drawn and fingerprinted: {differ}"
    );
}

/// The fingerprint of a few targets, pinned here as well as in the golden: a quick local check.
#[test]
fn fingerprints_are_pinned() {
    let got: Vec<String> = [(42, 0), (42, 100), (7, 50)]
        .iter()
        .map(|&(seed, d)| {
            target(&spec(seed, d, 100)).unwrap().fingerprint.to_hex()[..16].to_owned()
        })
        .collect();
    assert_eq!(
        got,
        ["dcdd135154fc58d4", "cdffa0ca71722819", "acb53ebf10d6e623"]
    );
}

/// Every destination v1's routes can take (the route slots pick from these; `every_drawn_route_is_in_the_pin`
/// checks it).
const ROUTE_DESTINATIONS: [&str; 4] = ["filter1.cutoff", "osc1.fine", "osc1.level", "osc1.pw"];

/// The catalogue fields generator v1 reads, for every address it reads them at, as text.
fn catalogue_v1() -> String {
    let p = start(TEMPLATE_STARTER).unwrap();
    let mut addrs: Vec<String> = SLOTS
        .iter()
        .filter_map(|s| match s.what {
            What::Param { addr, .. } => Some(addr.to_owned()),
            _ => None,
        })
        .chain(ROUTE_DESTINATIONS.iter().map(|a| (*a).to_owned()))
        .chain(std::iter::once("master.level".to_owned()))
        .collect();
    addrs.sort();
    addrs.dedup();
    let mut out = String::new();
    for a in addrs {
        let addr: ParamAddr = a.parse().unwrap();
        let spec = p.spec(&addr).unwrap_or_else(|| panic!("{a}"));
        let kind = match spec.kind {
            Kind::Float { min, max, scale } => format!("float {min:?}..{max:?} {scale:?}"),
            Kind::Int { min, max } => format!("int {min}..{max}"),
            Kind::Enum(items) => format!(
                "enum [{}]",
                items.iter().map(|i| i.key).collect::<Vec<_>>().join(",")
            ),
            Kind::Toggle => "toggle".to_owned(),
        };
        let f = spec.flags;
        let _ = writeln!(
            out,
            "{a} key={} unit={:?} {kind} default={:?} flags={}{}{}{}",
            spec.key,
            spec.unit,
            spec.default,
            if f.modulatable { "m" } else { "-" },
            if f.automatable { "a" } else { "-" },
            if f.structural { "s" } else { "-" },
            if f.periodic { "p" } else { "-" },
        );
    }
    out
}

/// T2 (F1r review M2): **the catalogue pin for generator v1.** v1 runs on the migrated start and reads live
/// `ParamSpec`s: ranges and scales (uniform draws, quantization), units and keys (the just-noticeable steps), defaults
/// (an empty item domain), items (discrete draws) and flags (which routes it may draw). This pins every one of them for
/// every address v1 reads them at. **If it fails, do not update it:** keep v1's old values in a frozen copy that v1
/// reads, or ship generator v2 with its own golden (docs/challenge.md, "Generator versions are frozen").
#[test]
fn the_catalogue_fields_v1_reads_are_pinned() {
    let got = catalogue_v1();
    let want = include_str!("catalogue-v1.txt");
    assert!(
        got == want,
        "a catalogue field generator v1 reads changed: freeze v1's copy or ship v2. Now:\n<<<\n{got}>>>"
    );
}

/// The pin covers every route destination v1 draws.
#[test]
fn every_drawn_route_is_in_the_pin() {
    let mut seen = std::collections::BTreeSet::new();
    for seed in 0..400 {
        let g = target(&spec(
            seed,
            [60, 80, 100][seed as usize % 3],
            [100, 0][seed as usize % 2],
        ))
        .unwrap();
        for r in &g.randomized {
            if let Drawn::Route(route) = &r.drawn {
                let dst = route.dst.to_string();
                assert!(ROUTE_DESTINATIONS.contains(&dst.as_str()), "{dst}");
                seen.insert(dst);
            }
        }
    }
    assert_eq!(seen.len(), ROUTE_DESTINATIONS.len(), "{seen:?}");
}
