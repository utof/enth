//! Generator properties P2-P5 (lane RG; docs/phase-2.md "Test strategy for the game logic"). P6, keyed-stream
//! stability, needs the table and is a unit test in `src/random/tests.rs`. Fixed seed in the gate
//! (`PROPTEST_RNG_SEED`) and proptest's default 256 cases; 10,000 nightly.
//!
//! - **P2** Generated patches pass `validate`, respect the challenge's constraints (R1 has none, so: the start's
//!   modules, ids and order are kept, nothing the start has on is switched off, routes only from the start's
//!   modulators or velocity) and stay in musical bounds (level, resonance, pitch) when `musical` is high.
//! - **P3** Difficulty is monotone (the category sets nest, and a higher level keeps the lower draws and values);
//!   per-category `amounts` override the knob.
//! - **P4** Reachable and observable: every randomized parameter exists in the start, is in range, quantized, and
//!   observable through the challenge's phrase; disabled modules and lone gauges are never randomized.
//! - **P5** The minimum step from the start holds in every active category.

use enth_challenge::phrases::REFERENCE_KEY;
use enth_challenge::priors::{ROUTE_JND, jnd};
use enth_challenge::random::{self, Drawn, Generated, Visibility, level, unlock_level};
use enth_challenge::{Category, RandomSpec};
use enth_patch::canon::exact_hash;
use enth_patch::{ModSource, Patch, RouteAddr};
use proptest::prelude::*;

fn starter() -> Patch {
    random::start(random::TEMPLATE_STARTER).unwrap()
}

fn amounts() -> impl Strategy<Value = [Option<u8>; Category::COUNT]> {
    proptest::array::uniform6(prop_oneof![6 => Just(None), 1 => (0u8..=100).prop_map(Some)])
}

fn specs() -> impl Strategy<Value = RandomSpec> {
    (
        0..=enth_challenge::spec::MAX_SEED,
        0u8..=100,
        0u8..=100,
        amounts(),
    )
        .prop_map(|(seed, difficulty, musical, amounts)| RandomSpec {
            generator: random::GENERATOR_V1,
            seed,
            difficulty,
            amounts,
            musical,
            template: random::TEMPLATE_STARTER,
        })
}

fn route_depth(p: &Patch, r: &RouteAddr) -> f32 {
    p.routes
        .iter()
        .find(|x| x.src == r.src && x.dst == r.dst)
        .map_or(0.0, |x| x.depth)
}

/// Whether one randomized item differs from the start by its just-noticeable step (recomputed here, independently
/// of the generator's bookkeeping).
fn passes(start: &Patch, g: &Generated, d: &Drawn) -> bool {
    match d {
        Drawn::Param(a) => {
            let spec = g.patch.spec(a).unwrap();
            jnd(spec).exceeded(start.value(a).unwrap(), g.patch.value(a).unwrap())
        }
        Drawn::Route(r) => ROUTE_JND.exceeded(route_depth(start, r), route_depth(&g.patch, r)),
        Drawn::Enable(id) => {
            g.patch.find(id).unwrap().1.enabled && !start.find(id).unwrap().1.enabled
        }
    }
}

fn val(p: &Patch, addr: &str) -> f32 {
    p.value(&addr.parse().unwrap()).unwrap()
}

proptest! {
    /// P2: valid, within the challenge's constraints, and in musical bounds above `musical` 20.
    #[test]
    fn p2_valid_constrained_and_musical(spec in specs()) {
        let start = starter();
        let g = random::generate(&spec, &start).unwrap();
        let p = &g.patch;
        let v = enth_patch::validate::validate(p);
        prop_assert!(v.is_ok(), "{}", v.unwrap_err());

        // The start's structure: same modules, kinds, sections and order; nothing switched off; no slot or macro
        // touched; routes only from the start's modulators or velocity.
        let ids = |q: &Patch| q.modules().map(|(pos, m)| (pos, m.id.clone(), m.kind)).collect::<Vec<_>>();
        prop_assert_eq!(ids(p), ids(&start));
        for ((_, a), (_, b)) in p.modules().zip(start.modules()) {
            prop_assert!(a.enabled || !b.enabled, "{} switched off", a.id);
        }
        prop_assert_eq!(&p.slots, &start.slots);
        prop_assert_eq!(&p.macros, &start.macros);
        for r in &p.routes {
            let ok = match &r.src {
                ModSource::Module(id) => start.modulators.iter().any(|m| &m.id == id),
                ModSource::Velocity => true,
                _ => false,
            };
            prop_assert!(ok, "route from {}", r.src);
        }

        if spec.musical > 20 {
            // Resonance capped, pitch within two octaves of the phrase, the cutoff relative to the note, and the
            // summed level within full scale after the headroom.
            prop_assert!(val(p, "filter1.resonance") <= 80.0);
            for semi in ["osc1.semi", "osc2.semi"] {
                prop_assert!(val(p, semi).abs() <= 24.0, "{semi} = {}", val(p, semi));
            }
            let f0 = 440.0 * libm::pow(2.0, (f64::from(REFERENCE_KEY) - 69.0 + f64::from(val(p, "osc1.semi"))) / 12.0);
            // (Only a drawn cutoff: an amount may draw the mode alone, and the start's 2 kHz then stays.)
            let cutoff = f64::from(val(p, "filter1.cutoff"));
            if g.randomized_params().any(|a| a.to_string() == "filter1.cutoff") {
                match val(p, "filter1.mode") as u8 {
                    0 => prop_assert!(cutoff >= 0.995 * f0, "LP cutoff {cutoff} under f0 {f0}"),
                    2 => prop_assert!(cutoff <= 8.05 * f0, "HP cutoff {cutoff} above 8 f0 {f0}"),
                    _ => {}
                }
            }
            let sum: f32 = p.generators.iter().filter(|m| m.enabled).map(|m| m.get("level").unwrap() / 100.0).sum();
            let gain = libm::powf(10.0, val(p, "master.level") / 20.0);
            prop_assert!(sum * gain <= 1.0 + 1e-4, "level {sum} × {gain}");
        }
        if spec.musical == 100 && spec.amounts.iter().all(Option::is_none) {
            // Priors only: tighter still.
            prop_assert!(val(p, "filter1.resonance") <= 60.5);
            prop_assert!([0.0, 12.0, -12.0].contains(&val(p, "osc1.semi")));
            for t in ["env1.attack", "env1.decay", "env1.release", "env2.decay"] {
                prop_assert!(val(p, t) <= 1_500.0, "{t}");
            }
        }
    }

    /// P3: the category sets nest with the difficulty and a higher level keeps every lower draw and value; an
    /// amount overrides the level for its category.
    #[test]
    fn p3_monotone_in_difficulty(spec in specs(), d2 in 0u8..=100) {
        let mut lo = spec;
        lo.amounts = [None; Category::COUNT];
        let mut hi = lo;
        (lo.difficulty, hi.difficulty) = (lo.difficulty.min(d2), lo.difficulty.max(d2));
        let (a, b) = (random::target(&lo).unwrap(), random::target(&hi).unwrap());
        let cats = |g: &Generated| g.categories().collect::<Vec<_>>();
        for c in Category::ALL {
            prop_assert_eq!(cats(&a).contains(&c), level(lo.difficulty) >= unlock_level(c), "{:?}", c);
        }
        prop_assert!(cats(&a).iter().all(|c| cats(&b).contains(c)));
        for r in &a.randomized {
            prop_assert!(b.randomized.contains(r), "{:?} lost at difficulty {}", r, hi.difficulty);
            match &r.drawn {
                Drawn::Param(addr) => prop_assert_eq!(a.patch.value(addr), b.patch.value(addr), "{}", addr),
                Drawn::Route(ra) => prop_assert_eq!(route_depth(&a.patch, ra), route_depth(&b.patch, ra)),
                Drawn::Enable(id) => {
                    prop_assert_eq!(a.patch.find(id).unwrap().1.enabled, b.patch.find(id).unwrap().1.enabled);
                }
            }
        }
    }

    /// P3: `amounts` override the knob, category by category.
    #[test]
    fn p3_amounts_override_the_knob(spec in specs(), which in 0usize..Category::COUNT, amount in 0u8..=100) {
        let mut s = spec;
        s.amounts = [None; Category::COUNT];
        s.amounts[which] = Some(amount);
        let g = random::target(&s).unwrap();
        let c = Category::ALL[which];
        let active: Vec<Category> = g.categories().collect();
        for other in Category::ALL {
            let want = if other == c { amount > 0 } else { level(s.difficulty) >= unlock_level(other) };
            prop_assert_eq!(active.contains(&other), want, "{:?} (amount {} for {:?})", other, amount, c);
        }
        if amount == 0 {
            prop_assert!(g.randomized.iter().all(|r| r.category != c));
        }
    }

    /// P4: every randomized item exists in the start, is reachable, and is observable through the phrase; no gauge
    /// and nothing disabled is ever randomized.
    #[test]
    fn p4_reachable_and_observable(spec in specs()) {
        let start = starter();
        let g = random::generate(&spec, &start).unwrap();
        let mask = g.mask();
        for r in &g.randomized {
            match &r.drawn {
                Drawn::Param(addr) => {
                    prop_assert!(start.value(addr).is_some(), "{} is not in the start", addr);
                    let ps = g.patch.spec(addr).unwrap();
                    let v = g.patch.value(addr).unwrap();
                    prop_assert!(ps.is_valid(v), "{} = {}", addr, v);
                    prop_assert_eq!(enth_params::ParamSpec::clamp(ps, v), v);
                    if !ps.is_discrete() {
                        // Three significant digits: printing with 3 digits gives the value back.
                        let text = format!("{:.2e}", f64::from(v));
                        prop_assert_eq!(text.parse::<f64>().unwrap() as f32, v, "{} = {} is not 3 digits", addr, v);
                    }
                    prop_assert_eq!(mask.param(addr), Some(Visibility::Observable), "{}", addr);
                    let name = addr.to_string();
                    prop_assert!(!["master.level", "voice.pan", "osc1.level", "comp1.makeup", "reverb1.dry"]
                        .contains(&name.as_str()), "{} is a gauge", name);
                }
                Drawn::Route(ra) => {
                    let depth = route_depth(&g.patch, ra);
                    prop_assert!(depth != 0.0 && depth.abs() <= 100.0 && depth % 5.0 == 0.0, "{} @ {}", ra, depth);
                    prop_assert_eq!(mask.route(ra), Some(Visibility::Observable), "{}", ra);
                }
                Drawn::Enable(id) => {
                    prop_assert_eq!(mask.module(id), Some(Visibility::Observable), "{}", id);
                    prop_assert!(!start.find(id).unwrap().1.enabled, "{} was already on", id);
                }
            }
        }
    }

    /// P5: in every active category with something counted to draw, a counted draw differs from the start by its
    /// just-noticeable step; and the target is never the start.
    #[test]
    fn p5_the_minimum_step_holds(spec in specs()) {
        let start = starter();
        let g = random::generate(&spec, &start).unwrap();
        for step in &g.steps {
            let counted: Vec<&Drawn> = g
                .randomized
                .iter()
                .filter(|r| r.category == step.category && r.counted)
                .map(|r| &r.drawn)
                .collect();
            if counted.is_empty() {
                prop_assert!(!step.met);
                continue;
            }
            prop_assert!(step.met, "{:?}", step);
            prop_assert!(counted.iter().any(|d| passes(&start, &g, d)), "{:?}: {:?}", step.category, counted);
        }
        if g.steps.iter().any(|s| s.met) {
            prop_assert_ne!(exact_hash(&g.patch), exact_hash(&start));
        }
    }
}
