//! Instruction budget for the G1 reference patch (spine lanes S2b, then E): 16 voices, every P1a module kind in use
//! with modulation routes from every kind of source, compressor and reverb on, one 2 s render of a held 16-note
//! chord in 512-frame host calls, via gungraun (callgrind).
//!
//! G1's CPU budget (roadmap.md, P1a summary): at most 12,000 instructions per stereo frame on the scalar path. Until
//! lane E re-bases it to the first full product measurement plus 10 %, the hard limit here is that provisional cap
//! (96,000 frames × 12,000), so lanes D1, D2 and FXW can land their modules without editing this file unless they
//! push G1 past its budget. The measured count, per lane, goes in the comment below and in docs/dsp/engine.md.
//! Needs valgrind and a `gungraun-runner` of the same version as the gungraun crate (`cargo xtask bench --bench g1`).
#![allow(missing_docs, reason = "benchmark harness")]

#[path = "common/mod.rs"]
mod common;

use enth_patch::catalogue::{
    AdsrParam, FilterMode, LfoParam, LfoScope, NoiseParam, OscParam, SvfParam, VoiceParam, Wave,
};
use enth_patch::{ModuleId, ModuleInstance, ModuleKind, Patch, Route};
use gungraun::{library_benchmark, library_benchmark_group, main};

/// Frames per G1 render: 2 s at 48 kHz.
const FRAMES: u64 = 96_000;

/// The provisional G1 cap: 12,000 instructions per stereo frame.
const G1_PER_FRAME: u64 = 12_000;

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

/// The G1 reference patch: two oscillators (saw and pulse) and noise into a resonant low-pass per voice, an amp and
/// a filter envelope, a vibrato LFO per voice and a slow global LFO, compressor and reverb, routes from envelopes,
/// LFOs, velocity, key and a macro; polyphony 16.
fn g1_patch() -> Patch {
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

fn g1(voices: usize) -> common::Prepared {
    common::prepare(&g1_patch(), voices, FRAMES as f32 / common::SR)
}

// Provisional budget: the G1 cap, 12,000 instructions per frame (1,152,000,000 per 2 s render).
// Measured on x86_64 Linux with the pinned toolchain:
// - S2b, 2026-10-19 (stub osc/noise/svf/lfo/reverb/comp, the reference envelope): 122,452,870 (1,276 per frame).
#[library_benchmark]
#[bench::g1_16v(args = (16,), setup = g1, config = common::budget(FRAMES * G1_PER_FRAME))]
fn render(p: common::Prepared) -> f32 {
    common::render(p)
}

library_benchmark_group!(name = g1, benchmarks = [render]);
main!(library_benchmark_groups = g1);
