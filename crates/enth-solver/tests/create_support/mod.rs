//! Lane F1S: what `create_replay.rs` and `create_intake.rs` share (the templates, a cheap target and a cheap
//! objective, as `create.rs` builds them).
#![allow(dead_code, reason = "each test file uses part of it")]

use enth_patch::{Op, ParamAddr, Patch, apply};
use enth_render::render;
use enth_solver::create::{CreateCfg, Inputs, Target, TargetPerf, Template};

/// A low rate and a short note keep a debug build's renders cheap.
pub const SR: u32 = 22_050;

pub fn set(p: &mut Patch, addr: &str, v: f32) {
    apply(
        p,
        Op::SetParam {
            addr: addr.parse().unwrap(),
            value: v,
        },
    )
    .unwrap();
}

pub fn load(text: &str) -> Patch {
    enth_patch::file::load(text).unwrap().patch
}

pub fn init() -> Patch {
    load(include_str!("../../../../content/templates/init.patch.json"))
}

pub fn templates() -> Vec<Template> {
    vec![
        Template {
            name: "pluck_1osc".into(),
            patch: load(include_str!(
                "../../../../content/templates/pluck_1osc.patch.json"
            )),
        },
        Template {
            name: "pad_2osc".into(),
            patch: load(include_str!(
                "../../../../content/templates/pad_2osc.patch.json"
            )),
        },
        Template {
            name: "bass_sub".into(),
            patch: load(include_str!(
                "../../../../content/templates/bass_sub.patch.json"
            )),
        },
    ]
}

/// The inputs over `init` and `templates`, without a bank or proposals.
pub fn inputs<'a>(init: &'a Patch, templates: &'a [Template]) -> Inputs<'a> {
    Inputs {
        init,
        templates,
        bank: None,
        proposals: &[],
    }
}

/// The patch behind [`target`]: a pluck-like sine.
pub fn target_patch() -> Patch {
    let mut p = init();
    set(&mut p, "osc1.wave", 0.0);
    set(&mut p, "env1.attack", 4.0);
    set(&mut p, "env1.decay", 180.0);
    set(&mut p, "env1.sustain", 30.0);
    set(&mut p, "env1.release", 120.0);
    p
}

/// The upload: [`target_patch`] rendered with a pre-roll of four segments, as FE's intake keeps one.
pub fn target() -> Target {
    let mut t = Target {
        audio: Vec::new(),
        sample_rate: SR,
        render_seed: 5,
        perf: TargetPerf {
            key: 57,
            velocity: 100.0 / 127.0,
            on_s: 128.0 / f64::from(SR),
            hold_s: 0.3,
            duration_s: 0.7,
            f0_hz: None,
        },
        bandwidth_hz: None,
        noise_floor_dbfs: None,
    };
    t.audio = render(&target_patch(), &t.performance(), &t.render_spec())
        .unwrap()
        .mid();
    t
}

/// A cheap stand-in for render + metric: how far a few parameters are from the upload's, on their normalised axes;
/// a patch missing one pays 1.
pub fn proxy(p: &Patch) -> f64 {
    let goal = [
        ("osc1.wave", 0.0),
        ("env1.attack", 4.0),
        ("env1.decay", 180.0),
        ("env1.sustain", 30.0),
        ("env1.release", 120.0),
        ("osc1.semi", 0.0),
    ];
    goal.iter()
        .map(|(a, g)| {
            let addr: ParamAddr = a.parse().unwrap();
            match (p.spec(&addr), p.value(&addr)) {
                (Some(s), Some(v)) => f64::from((s.to_norm(v) - s.to_norm(*g)).abs()),
                _ => 1.0,
            }
        })
        .sum()
}

/// The judge's probe settings: K 4, 3 neighbours, no spread, seed 11.
pub fn cfg(budget: usize) -> CreateCfg {
    CreateCfg {
        k: 4,
        budget,
        seed: 11,
        neighbours: 3,
        min_spread: 0.0,
        ..CreateCfg::default()
    }
}
