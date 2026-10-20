//! The G1 reference patch, shared by the `g1` and `swap` benches (`#[path = "common/g1.rs"] mod g1_patch;`).
//!
//! The test host's fixture `tools/test-host/golden/enth-plugin.g1.patch.json` is `Patch::to_json()` of this patch
//! (generated 2026-10-20 by lane RT); change both together.
#![allow(dead_code, reason = "each bench uses a different subset")]

use enth_patch::catalogue::{
    AdsrParam, FilterMode, LfoParam, LfoScope, NoiseParam, OscParam, SvfParam, VoiceParam, Wave,
};
use enth_patch::{ModuleId, ModuleInstance, ModuleKind, Patch, Route};

fn id(s: &str) -> ModuleId {
    ModuleId::new(s).expect("valid id")
}

pub fn module(name: &str, kind: ModuleKind, set: &[(usize, f32)]) -> ModuleInstance {
    let mut m = ModuleInstance::new(id(name), kind);
    for &(i, v) in set {
        m.values[i] = v;
    }
    m
}

/// The G1 reference patch: two oscillators (saw and pulse) and noise into a resonant low-pass per voice, an amp and
/// a filter envelope, a vibrato LFO per voice and a slow global LFO, compressor and reverb, routes from envelopes,
/// LFOs, velocity, key and a macro; polyphony 16.
pub fn g1_patch() -> Patch {
    let mut p = Patch::init();
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
                (LfoParam::Rate.index(), 0.3),
            ],
        ),
    ];
    p.fx = vec![
        ModuleInstance::new(id("comp1"), ModuleKind::Comp),
        ModuleInstance::new(id("reverb1"), ModuleKind::Reverb),
    ];
    p.macros[0].value = 0.4;
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
    p
}
