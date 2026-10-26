//! The deconstruct generator end to end on a small preset (lane DEC): the runtime entry point the plugin's worker calls
//! (`deconstruct::auto_lesson`'s two halves, `generate` and `pack_of`) builds a one-lesson pack the real reader
//! accepts, the planner's last target is the preset, and the authoring TOML reads back as the same lesson.
//!
//! Small on purpose (four elements, no "What disappeared?" and no variation goal): about a hundred renders. The six
//! factory studies are the full exercise (`studies_pack.rs`, and `cargo xtask lesson-check` on them).

use std::collections::BTreeMap;

use enth_learn::deconstruct::{Options, elements, generate, pack_of, toml_out::lesson_toml};
use enth_learn::{Lesson, StepKind};
use enth_patch::{Op, ParamAddr, Patch, RouteAddr, apply};

/// The starter with a dark resonant filter, a filter envelope and a reverb.
fn preset() -> Patch {
    let mut p =
        enth_patch::file::load(include_str!("../../../xtask/src/preset/starter.patch.json"))
            .unwrap()
            .patch;
    for (addr, value) in [
        ("filter1.cutoff", 400.0),
        ("filter1.resonance", 40.0),
        ("env2.sustain", 0.0),
        ("env2.decay", 200.0),
        ("reverb1.wet", -14.0),
    ] {
        apply(
            &mut p,
            Op::SetParam {
                addr: addr.parse::<ParamAddr>().unwrap(),
                value,
            },
        )
        .unwrap();
    }
    apply(
        &mut p,
        Op::SetEnabled {
            id: "reverb1".parse().unwrap(),
            enabled: true,
        },
    )
    .unwrap();
    let r: RouteAddr = "env2 -> filter1.cutoff".parse().unwrap();
    apply(
        &mut p,
        Op::InsertRoute {
            index: 0,
            route: enth_patch::Route {
                src: r.src,
                dst: r.dst,
                depth: 40.0,
            },
        },
    )
    .unwrap();
    let meta = enth_patch::PatchMeta {
        name: "Test Pluck".into(),
        ..Default::default()
    };
    apply(&mut p, Op::SetMeta { meta }).unwrap();
    p
}

#[test]
fn a_small_preset_becomes_a_one_lesson_pack_that_reads_back() {
    let p = preset();
    assert_eq!(
        elements(&p).len(),
        3,
        "the filter, the route with its envelope, the reverb"
    );
    let mut opts = Options::for_preset(&p);
    opts.choice = false;
    opts.variation = false;
    let g = generate(&p, &opts).unwrap_or_else(|e| panic!("{e} ({})", e.code()));

    // The plan ends on the preset, and every step adds something.
    let last = g.plan.steps.len() - 1;
    assert_eq!(g.plan.step_target(last), p);
    for k in 0..g.plan.steps.len() {
        assert_ne!(g.plan.step_start(k), g.plan.step_target(k));
    }
    assert!(g.report.renders.total() > 0);
    assert_eq!(g.report.steps.len(), g.plan.steps.len());

    // The authoring TOML reads back as the same lesson.
    let toml = lesson_toml(&g.lesson, &["a header".to_owned()], &BTreeMap::new());
    let value: serde_json::Value = toml::from_str(&toml).unwrap();
    let back = Lesson::from_json(&value.to_string()).unwrap_or_else(|e| panic!("{e}\n{toml}"));
    assert_eq!(back, g.lesson);

    // The runtime pack: verified by the real reader, its one lesson playable.
    let auto = pack_of(&p, &g, "my-presets").unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(auto.pack.id().as_str(), "my-presets");
    let bundle = auto
        .pack
        .bundle(&g.lesson.id)
        .expect("the lesson is in its pack");
    assert_eq!(bundle.lesson.steps.len(), g.lesson.steps.len());

    // `requires` says what the lesson uses, never this build's newest (the F1e review's M2): the highest challenge
    // version among its Recreate steps (v2 draws today), and the lesson's own version (no drill step: 1). An R2 or R3
    // build, which reads challenge 2 and lesson 1, can play the pack.
    let used = g
        .lesson
        .steps
        .iter()
        .filter_map(|s| match &s.kind {
            StepKind::Recreate { challenge, .. } => Some(challenge.version()),
            _ => None,
        })
        .max();
    let requires = &auto.pack.manifest().requires;
    assert_eq!(requires.challenge, used);
    assert_eq!(used, Some(2), "the rounds are v2 draws");
    assert!(used < Some(enth_challenge::model::CHALLENGE_VERSION));
    assert_eq!(requires.lesson, 1);
    assert_eq!(requires.lesson, g.lesson.version());
    for text in g.texts.values() {
        assert!(
            enth_learn::markdown::lint(text, enth_learn::markdown::STEP_WORD_BUDGET).is_empty(),
            "{text}"
        );
    }
}

/// Lane MTC: a preset that uses S2a's kinds (a ladder filter, a chorus) deconstructs like any other: every element
/// is taught, the last step is the preset, and the texts read.
#[test]
fn a_preset_with_s2a_kinds_deconstructs_too() {
    use enth_patch::addr::Position;
    use enth_patch::{ModuleInstance, ModuleKind};
    let mut p = preset();
    for (kind, id) in [
        (ModuleKind::Ladder, "ladder1"),
        (ModuleKind::Chorus, "chorus1"),
    ] {
        let at = Position {
            section: kind.section(),
            index: p.section(kind.section()).len(),
        };
        apply(
            &mut p,
            Op::AddModule {
                at,
                module: ModuleInstance::new(id.parse().unwrap(), kind),
            },
        )
        .unwrap();
    }
    for (addr, value) in [
        ("ladder1.cutoff", 700.0),
        ("ladder1.resonance", 50.0),
        ("chorus1.depth", 80.0),
        ("chorus1.mix", 60.0),
    ] {
        apply(
            &mut p,
            Op::SetParam {
                addr: addr.parse::<ParamAddr>().unwrap(),
                value,
            },
        )
        .unwrap();
    }
    let els = elements(&p);
    let ids: Vec<&str> = els.iter().map(|e| e.id.as_str()).collect();
    assert!(
        ids.contains(&"ladder1") && ids.contains(&"chorus1"),
        "{ids:?}"
    );
    let mut opts = Options::for_preset(&p);
    opts.choice = false;
    opts.variation = false;
    let g = generate(&p, &opts).unwrap_or_else(|e| panic!("{e} ({})", e.code()));
    let last = g.plan.steps.len() - 1;
    assert_eq!(g.plan.step_target(last), p);
    assert_eq!(g.report.steps.len(), g.plan.steps.len());
    for text in g.texts.values() {
        assert!(
            enth_learn::markdown::lint(text, enth_learn::markdown::STEP_WORD_BUDGET).is_empty(),
            "{text}"
        );
    }
    let toml = lesson_toml(&g.lesson, &["a header".to_owned()], &BTreeMap::new());
    let value: serde_json::Value = toml::from_str(&toml).unwrap();
    assert_eq!(Lesson::from_json(&value.to_string()).unwrap(), g.lesson);
}
