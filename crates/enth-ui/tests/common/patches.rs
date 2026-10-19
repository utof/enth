//! Test patches. `g1` is the G1 reference patch (the shape of `crates/enth-engine/benches/g1.rs`, as research 02's
//! prototype used it): every P1 module kind, eight routes, 59 controls in the basic and intermediate tiers.

use enth_patch::catalogue::{
    AdsrParam, FilterMode, LfoParam, LfoScope, LfoShape, NoiseParam, OscParam, SvfParam,
    VoiceParam, Wave,
};
use enth_patch::{ModuleId, ModuleInstance, ModuleKind, Patch, Route};

fn id(s: &str) -> ModuleId {
    ModuleId::new(s).expect("valid id")
}

fn module(name: &str, kind: ModuleKind, set: &[(usize, f32)]) -> ModuleInstance {
    let mut m = ModuleInstance::new(id(name), kind);
    for &(i, v) in set {
        m.values[i] = v;
    }
    m
}

/// The G1 reference patch.
pub fn g1() -> Patch {
    let mut p = Patch::init();
    "G1 reference".clone_into(&mut p.meta.name);
    p.voice.values[VoiceParam::Polyphony.index()] = 16.0;
    p.generators = vec![
        module(
            "osc1",
            ModuleKind::Osc,
            &[(OscParam::Wave.index(), Wave::Saw.index() as f32)],
        ),
        module(
            "osc2",
            ModuleKind::Osc,
            &[
                (OscParam::Wave.index(), Wave::Pulse.index() as f32),
                (OscParam::Semi.index(), -12.0),
                (OscParam::Fine.index(), 7.0),
                (OscParam::Level.index(), 60.0),
                (OscParam::Pw.index(), 30.0),
            ],
        ),
        module(
            "noise1",
            ModuleKind::Noise,
            &[(NoiseParam::Level.index(), 15.0)],
        ),
    ];
    p.voice_fx = vec![module(
        "filter1",
        ModuleKind::Svf,
        &[
            (SvfParam::Mode.index(), FilterMode::Lp.index() as f32),
            (SvfParam::Cutoff.index(), 1500.0),
            (SvfParam::Resonance.index(), 30.0),
            (SvfParam::Keytrack.index(), 50.0),
        ],
    )];
    p.modulators = vec![
        module(
            "env1",
            ModuleKind::Adsr,
            &[
                (AdsrParam::Attack.index(), 10.0),
                (AdsrParam::Decay.index(), 400.0),
                (AdsrParam::Sustain.index(), 100.0),
                (AdsrParam::Release.index(), 600.0),
            ],
        ),
        module(
            "env2",
            ModuleKind::Adsr,
            &[
                (AdsrParam::Attack.index(), 2.0),
                (AdsrParam::Decay.index(), 250.0),
                (AdsrParam::Sustain.index(), 30.0),
            ],
        ),
        module("lfo1", ModuleKind::Lfo, &[(LfoParam::Rate.index(), 5.5)]),
        module(
            "lfo2",
            ModuleKind::Lfo,
            &[
                (LfoParam::Scope.index(), LfoScope::Global.index() as f32),
                (LfoParam::Shape.index(), LfoShape::Triangle.index() as f32),
                (LfoParam::Rate.index(), 0.3),
            ],
        ),
    ];
    p.fx = vec![
        ModuleInstance::new(id("comp1"), ModuleKind::Comp),
        ModuleInstance::new(id("reverb1"), ModuleKind::Reverb),
    ];
    for (src, dst, depth) in [
        ("env2", "filter1.cutoff", 40.0),
        ("velocity", "filter1.cutoff", 10.0),
        ("lfo1", "osc1.fine", 4.0),
        ("lfo1", "osc2.fine", -4.0),
        ("lfo2", "voice.pan", 30.0),
        ("lfo2", "reverb1.wet", 5.0),
        ("key", "osc2.pw", 20.0),
        ("macro1", "filter1.resonance", 25.0),
    ] {
        p.routes.push(Route {
            src: src.parse().expect("source"),
            dst: dst.parse().expect("destination"),
            depth,
        });
    }
    for (i, name) in ["Bright", "Pluck", "Space", "Drive"]
        .into_iter()
        .enumerate()
    {
        p.macros[i].name = Some(name.to_owned());
    }
    p.macros[0].value = 0.7;
    p.macros[1].value = 0.3;
    p.macros[2].value = 0.5;
    p
}

/// The P1 default patch the editor's layouts are judged at (the layout handoff's inventory, section 5): two
/// oscillators, noise, a filter, two envelopes, two LFOs, a compressor and a reverb. It is the G1 reference patch.
pub fn p1_default() -> Patch {
    g1()
}

/// The maximum patch (lane ED1's second whole-editor snapshot): 4 generators, 4 voice effects, 4 effects and 8
/// modulators, the patch model's limits, with the G1 patch's routes and macros.
pub fn maximum() -> Patch {
    let mut p = g1();
    "Maximum".clone_into(&mut p.meta.name);
    p.generators.push(module(
        "osc3",
        ModuleKind::Osc,
        &[
            (OscParam::Wave.index(), Wave::Triangle.index() as f32),
            (OscParam::Semi.index(), 12.0),
        ],
    ));
    for (name, mode, cutoff) in [
        ("filter2", FilterMode::Hp, 200.0),
        ("filter3", FilterMode::Bp, 900.0),
        ("filter4", FilterMode::Lp, 5000.0),
    ] {
        p.voice_fx.push(module(
            name,
            ModuleKind::Svf,
            &[
                (SvfParam::Mode.index(), mode.index() as f32),
                (SvfParam::Cutoff.index(), cutoff),
            ],
        ));
    }
    p.fx.push(ModuleInstance::new(id("comp2"), ModuleKind::Comp));
    p.fx.push(ModuleInstance::new(id("reverb2"), ModuleKind::Reverb));
    p.modulators.push(module(
        "env3",
        ModuleKind::Adsr,
        &[(AdsrParam::Attack.index(), 300.0)],
    ));
    p.modulators.push(module(
        "env4",
        ModuleKind::Adsr,
        &[(AdsrParam::Release.index(), 2000.0)],
    ));
    p.modulators.push(module(
        "lfo3",
        ModuleKind::Lfo,
        &[(LfoParam::Shape.index(), LfoShape::Square.index() as f32)],
    ));
    p.modulators.push(module(
        "lfo4",
        ModuleKind::Lfo,
        &[(LfoParam::Rate.index(), 0.1)],
    ));
    p
}
