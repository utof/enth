//! `render()` end to end: the host block size is not an input, the render seed is the only source of randomness,
//! moving a knob never reseeds anything (seed stability), every sample is finite, and bad input is a typed error.

use enth_params::Kind;
use enth_patch::catalogue::{AdsrParam, LfoParam, LfoScope, OscParam};
use enth_patch::{ModuleId, ModuleInstance, ModuleKind, Patch};
use enth_render::{
    Automation, DEFAULT_SEED, Note, Performance, RenderOutput, RenderSpec, render,
    render_with_block,
};
use proptest::prelude::*;

fn id(s: &str) -> ModuleId {
    ModuleId::new(s).unwrap()
}

fn bits(o: &RenderOutput) -> Vec<u32> {
    o.left.iter().chain(&o.right).map(|x| x.to_bits()).collect()
}

/// Two saws at the same pitch with free phases: their phase difference is the timbre, so a reseed (the bug the seed
/// rule prevents) moves the RMS envelope by decibels.
fn two_saws() -> Patch {
    let mut p = Patch::init();
    p.generators
        .push(ModuleInstance::new(id("osc2"), ModuleKind::Osc));
    p.generators[1].values[OscParam::Fine.index()] = 3.0;
    p.generators[1].values[OscParam::Level.index()] = 80.0;
    p.modulators[0].values[AdsrParam::Release.index()] = 300.0;
    p
}

/// One module of every kind (both LFO scopes), for properties that must hold whatever later lanes put in them.
fn every_kind() -> Patch {
    let mut p = two_saws();
    p.generators
        .push(ModuleInstance::new(id("noise1"), ModuleKind::Noise));
    p.voice_fx
        .push(ModuleInstance::new(id("filter1"), ModuleKind::Svf));
    p.modulators
        .push(ModuleInstance::new(id("lfo1"), ModuleKind::Lfo));
    let mut lfo2 = ModuleInstance::new(id("lfo2"), ModuleKind::Lfo);
    lfo2.values[LfoParam::Scope.index()] = LfoScope::Global.index() as f32;
    p.modulators.push(lfo2);
    p.fx.push(ModuleInstance::new(id("comp1"), ModuleKind::Comp));
    p.fx.push(ModuleInstance::new(id("reverb1"), ModuleKind::Reverb));
    p
}

fn chord(hold: f64, dur: f64) -> Performance {
    Performance {
        notes: [48u8, 55, 60, 64]
            .iter()
            .enumerate()
            .map(|(i, &k)| Note::new(k, 0.8, 0.013 * i as f64, hold))
            .collect(),
        duration_s: dur,
        ..Default::default()
    }
}

#[test]
fn the_reference_saw_renders_what_it_should() {
    let out = render(
        &Patch::init(),
        &Performance::note(69, 1.0, 0.5, 1.5),
        &RenderSpec::default(),
    )
    .unwrap();
    assert_eq!(out.frames(), 72_000);
    assert_eq!(out.left, out.right, "a centred mono voice");
    let peak = out.peak();
    // A saw of amplitude 1, sustain 70 %, -3 dB centre pan: about 0.71 at the attack peak.
    assert!(peak > 0.6 && peak < 0.75, "{peak}");
    // The reference release falls 60 dB in 400 ms: from 70 % to -100 dB in about 650 ms.
    assert_eq!(out.stats.voices_at_end, 0, "the release ended the voice");
    assert!(out.left[60_000..].iter().all(|x| *x == 0.0));
    // 440 Hz: count falling zero crossings over the held part (one per period).
    let held = &out.left[4800..24_000];
    let crossings = held
        .windows(2)
        .filter(|w| w[0] >= 0.0 && w[1] < 0.0)
        .count();
    assert!(
        (175..=177).contains(&crossings),
        "{crossings} periods in 0.4 s"
    );
}

#[test]
fn any_block_size_gives_the_same_bits() {
    let mut perf = chord(0.8, 1.5);
    perf.notes.push(Note::new(72, 0.5, 0.3337, 0.3337));
    perf.automation.push(Automation {
        time_s: 0.5001,
        addr: "osc2.level".parse().unwrap(),
        value: 20.0,
    });
    perf.automation.push(Automation {
        time_s: 0.9,
        addr: "master.level".parse().unwrap(),
        value: -12.0,
    });
    let spec = RenderSpec::default();
    let reference = render(&every_kind(), &perf, &spec).unwrap();
    for block in [1, 7, 63, 64, 65, 512, 4096] {
        let got = render_with_block(&every_kind(), &perf, &spec, block).unwrap();
        assert_eq!(bits(&got), bits(&reference), "block {block}");
    }
}

#[test]
fn the_render_seed_is_the_only_randomness() {
    let spec = RenderSpec::default();
    let a = render(&two_saws(), &chord(0.5, 0.8), &spec).unwrap();
    let b = render(&two_saws(), &chord(0.5, 0.8), &spec).unwrap();
    assert_eq!(bits(&a), bits(&b));
    let c = render(
        &two_saws(),
        &chord(0.5, 0.8),
        &RenderSpec { seed: 1, ..spec },
    )
    .unwrap();
    assert_ne!(bits(&a), bits(&c));
}

/// Mean-square power per 10 ms (decibels need `libm`, which this crate does not depend on; power ratios do).
fn power(x: &[f32], sr: u32) -> Vec<f64> {
    x.chunks((sr / 100) as usize)
        .map(|c| c.iter().map(|&s| f64::from(s) * f64::from(s)).sum::<f64>() / c.len() as f64)
        .collect()
}

/// The power ratio of a level change in dB: `10^(dB/10)`, from a few exact values (no `libm` here).
const RATIO_0_05_DB: f64 = 1.011_579_454_259_899; // 10^(0.005)
const RATIO_1_DB: f64 = 1.258_925_411_794_167_2; // 10^(0.1)
const RATIO_30_DB: f64 = 1e3;

/// The largest power ratio (>= 1) between two 10 ms RMS envelopes over the frames within 30 dB of the louder
/// render's peak frame (deeper in a release tail a 1e-3 change of a time constant legitimately moves the level by
/// more: 1e-3 of the tail's depth in dB). As decibels: `10·log10(ratio)`; near 1, about `4.343·(ratio − 1)`.
fn max_rms_change(a: &RenderOutput, b: &RenderOutput) -> f64 {
    let (pa, pb) = (
        power(&a.mid(), a.sample_rate),
        power(&b.mid(), b.sample_rate),
    );
    let peak = pa.iter().chain(&pb).fold(0.0f64, |m, x| m.max(*x));
    pa.iter()
        .zip(&pb)
        .filter(|(x, y)| x.max(**y) * RATIO_30_DB > peak && x.min(**y) > 0.0)
        .map(|(x, y)| x.max(*y) / x.min(*y))
        .fold(1.0, f64::max)
}

/// A 1e-3 relative move of a non-zero float value, kept inside its range; `None` for other kinds and zeros.
fn nudged(spec: &enth_params::ParamSpec, value: f32) -> Option<f32> {
    let Kind::Float { min, max, .. } = spec.kind else {
        return None;
    };
    if value == 0.0 {
        return None;
    }
    let up = value * 1.001;
    Some(if (min..=max).contains(&up) {
        up
    } else {
        value * 0.999
    })
}

/// Parameters whose effect decorrelates the waveform by nature, so a 1e-3 move changes the fine structure of the
/// output and with it a 10 ms RMS envelope, with no seed involved (a reverb's room size moves its delay taps).
/// A lane adds `"module_kind.key"` here with the reason in a comment; reseeding bugs never belong here.
const EXEMPT: &[&str] = &[];

/// Seed stability (P1a acceptance): moving any continuous parameter by 1e-3 relative changes the 10 ms RMS
/// envelope by under 0.05 dB, because no seed depends on a parameter value. Covers every non-zero float parameter
/// of every module (one of every kind, so the lanes' modules join automatically) and of `voice.*` and `master.*`.
#[test]
fn moving_any_continuous_parameter_by_1e_3_moves_the_rms_envelope_by_under_0_05_db() {
    let base = every_kind();
    let perf = chord(0.6, 1.0);
    let spec = RenderSpec::default();
    let reference = render(&base, &perf, &spec).unwrap();
    let mut variants: Vec<(String, Patch)> = Vec::new();
    for (pos, m) in base.modules() {
        for (i, ps) in m.kind.params().iter().enumerate() {
            if EXEMPT.contains(&format!("{}.{}", m.kind.key(), ps.key).as_str()) {
                continue;
            }
            if let Some(v) = nudged(ps, m.values[i]) {
                let mut p = base.clone();
                p.section_mut(pos.section)[pos.index].values[i] = v;
                variants.push((format!("{}.{} {} -> {v}", m.id, ps.key, m.values[i]), p));
            }
        }
    }
    for (i, ps) in enth_patch::catalogue::VOICE_PARAMS.iter().enumerate() {
        if let Some(v) = nudged(ps, base.voice.values[i]) {
            let mut p = base.clone();
            p.voice.values[i] = v;
            variants.push((format!("voice.{}", ps.key), p));
        }
    }
    for (i, ps) in enth_patch::catalogue::MASTER_PARAMS.iter().enumerate() {
        if let Some(v) = nudged(ps, base.master[i]) {
            let mut p = base.clone();
            p.master[i] = v;
            variants.push((format!("master.{}", ps.key), p));
        }
    }
    assert!(
        variants.len() >= 20,
        "only {} parameters were nudged",
        variants.len()
    );
    let mut worst = (1.0f64, String::new());
    for (label, p) in &variants {
        let d = max_rms_change(&reference, &render(p, &perf, &spec).unwrap());
        if d > worst.0 {
            worst = (d, label.clone());
        }
    }
    println!(
        "{} parameters nudged; worst power ratio {:.7} (about {:.5} dB) for {}",
        variants.len(),
        worst.0,
        4.343 * (worst.0 - 1.0),
        worst.1
    );
    assert!(
        worst.0 < RATIO_0_05_DB,
        "worst: power ratio {:.6} (about {:.4} dB) for {}",
        worst.0,
        4.343 * (worst.0 - 1.0),
        worst.1
    );

    // Negative control: seeding from the patch amounts to a different seed for every nudged patch, and that moves
    // the envelope by decibels, so this test can fail.
    let reseeded = render(
        &base,
        &perf,
        &RenderSpec {
            seed: DEFAULT_SEED ^ 0x5EED,
            ..spec
        },
    )
    .unwrap();
    let d = max_rms_change(&reference, &reseeded);
    println!("negative control: a reseed moved the envelope by a power ratio of {d:.3}");
    assert!(
        d > RATIO_1_DB,
        "a reseed moved the RMS envelope by a power ratio of only {d:.4}"
    );
}

/// A random valid value for every parameter of every module.
fn random_values() -> impl Strategy<Value = Vec<Vec<f32>>> {
    let kinds: Vec<ModuleKind> = every_kind().modules().map(|(_, m)| m.kind).collect();
    let per_module: Vec<_> = kinds
        .into_iter()
        .map(|kind| {
            let per_param: Vec<_> = kind
                .params()
                .iter()
                .map(|s| {
                    let (lo, hi) = (s.min(), s.max());
                    (0.0f64..=1.0).prop_map(move |u| {
                        let x = (f64::from(lo) + u * (f64::from(hi) - f64::from(lo))) as f32;
                        let x = if s.is_discrete() { x.round() } else { x };
                        s.clamp(x)
                    })
                })
                .collect();
            per_param
        })
        .collect();
    per_module
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 64, ..ProptestConfig::default() })]

    /// Every output sample is finite for random parameter values in every module (stubs today, the real modules
    /// once the lanes land).
    #[test]
    fn every_sample_is_finite_for_random_parameters(values in random_values()) {
        let mut p = every_kind();
        let positions: Vec<_> = p.modules().map(|(pos, _)| pos).collect();
        for (pos, v) in positions.into_iter().zip(values) {
            p.section_mut(pos.section)[pos.index].values = v;
        }
        // Keep the LFO scopes and the wavetable choice valid structurally; values are valid by construction.
        let out = render(&p, &chord(0.15, 0.3), &RenderSpec::default());
        let out = out.map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert!(out.left.iter().chain(&out.right).all(|x| x.is_finite()));
    }
}

#[test]
fn bad_input_is_a_typed_error() {
    let spec = RenderSpec::default();
    let p = Patch::init();
    let bad = |perf: Performance| render(&p, &perf, &spec).unwrap_err().code();
    assert_eq!(
        bad(Performance {
            duration_s: -1.0,
            ..Default::default()
        }),
        "bad_performance"
    );
    assert_eq!(
        bad(Performance {
            duration_s: f64::NAN,
            ..Default::default()
        }),
        "bad_performance"
    );
    assert_eq!(
        bad(Performance {
            duration_s: 1e9,
            ..Default::default()
        }),
        "bad_performance"
    );
    assert_eq!(
        bad(Performance {
            notes: vec![Note::new(60, 0.5, 1.0, 0.5)],
            duration_s: 2.0,
            ..Default::default()
        }),
        "bad_performance"
    );
    assert_eq!(
        bad(Performance {
            notes: vec![Note::new(200, 0.5, 0.0, 0.5)],
            duration_s: 1.0,
            ..Default::default()
        }),
        "bad_performance"
    );
    assert_eq!(
        bad(Performance {
            notes: vec![Note::new(60, 1.5, 0.0, 0.5)],
            duration_s: 1.0,
            ..Default::default()
        }),
        "bad_performance"
    );
    let auto = |addr: &str, value: f32| Performance {
        automation: vec![Automation {
            time_s: 0.1,
            addr: addr.parse().unwrap(),
            value,
        }],
        duration_s: 0.2,
        ..Default::default()
    };
    assert_eq!(bad(auto("filter9.cutoff", 100.0)), "unknown_param");
    assert_eq!(bad(auto("osc1.level", f32::INFINITY)), "bad_performance");
    assert_eq!(
        render(
            &p,
            &Performance::default(),
            &RenderSpec {
                sample_rate: 1000,
                ..spec
            }
        )
        .unwrap_err()
        .code(),
        "bad_spec"
    );
    // A disabled module's parameter exists: automating it is not an error, and changes nothing.
    let mut q = two_saws();
    q.generators[1].enabled = false;
    let quiet = render(&q, &auto("osc2.level", 0.0), &spec).unwrap();
    let plain = render(
        &q,
        &Performance {
            duration_s: 0.2,
            ..Default::default()
        },
        &spec,
    )
    .unwrap();
    assert_eq!(bits(&quiet), bits(&plain));
    // Compile errors pass through with their own code.
    let mut r = Patch::init();
    r.generators[0].values[OscParam::Wave.index()] = 99.0;
    assert_eq!(
        render(&r, &Performance::default(), &spec)
            .unwrap_err()
            .code(),
        "invalid_value"
    );
}
