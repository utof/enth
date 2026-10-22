//! The model contract through the real product path (lane F3S, after the F3s judge's probes): renders, the intake,
//! feature set 1 and layout 1's labels and decoding together.
//!
//! - M1: labels are relative to the key the intake reads, so two sounds with one model input get one label.
//! - M3: decoding under the job's performance gives back the label.
//! - M4: feature set 1's band levels do not follow the sample rate.

use core::fmt::Write as _;

use enth_analysis::intake::Intake;
use enth_challenge::model::PhraseNote;
use enth_challenge::phrases::{HELD, phrase};
use enth_challenge::random;
use enth_challenge::spec::{Category, RandomSpec};
use enth_patch::{Op, Patch, apply};
use enth_render::{DEFAULT_SEED, Note, Performance, RenderSpec, render};
use enth_solver::create::Target;
use enth_solver::propose::decode::{Hearing, OutputLayout};
use enth_solver::propose::features::{Cue, ModelInput};

const SR: u32 = 48_000;

fn set(p: &mut Patch, addr: &str, v: f32) {
    apply(
        p,
        Op::SetParam {
            addr: addr.parse().unwrap(),
            value: v,
        },
    )
    .unwrap();
}

/// Render `notes` (from the clip's start) over `length_ms`, then run the product's intake.
fn intake(p: &Patch, notes: &[PhraseNote], length_ms: u32, sr: u32) -> Intake {
    let perf = Performance {
        notes: notes
            .iter()
            .map(|n| {
                let on = f64::from(n.start_ms) / 1000.0;
                let off = f64::from(n.start_ms + n.hold_ms) / 1000.0;
                Note::new(n.key, f32::from(n.velocity) / 127.0, on, off)
            })
            .collect(),
        duration_s: f64::from(length_ms) / 1000.0,
        ..Performance::default()
    };
    let audio = render(p, &perf, &RenderSpec::new(sr, DEFAULT_SEED))
        .unwrap()
        .mid();
    Intake::new(&audio, 1, sr).unwrap()
}

fn one(key: u8) -> [PhraseNote; 1] {
    [PhraseNote {
        key,
        velocity: 100,
        start_ms: 0,
        hold_ms: 1500,
    }]
}

fn v1(seed: u64) -> Patch {
    let spec = RandomSpec {
        generator: 1,
        seed,
        difficulty: 100,
        amounts: [None; Category::COUNT],
        musical: [100, 50, 0][seed as usize % 3],
        template: 0,
    };
    random::target(&spec).unwrap().patch
}

/// M1, the judge's probe pair: the template with `osc1.semi` +12 played at C3 and with 0 played at C4 give the
/// same model input, and now the same label.
#[test]
fn one_input_one_label() {
    let l = OutputLayout::v1();
    let mut up = l.template().clone();
    set(&mut up, "osc1.semi", 12.0);
    let a = intake(&up, &one(48), 3000, SR);
    let b = intake(l.template(), &one(60), 3000, SR);
    let (ca, cb) = (Cue::from_intake(&a), Cue::from_intake(&b));
    assert_eq!((ca.key, cb.key), (60, 60));
    let (ma, mb) = (
        ModelInput::from_intake(&a).unwrap(),
        ModelInput::from_intake(&b).unwrap(),
    );
    assert_eq!(ma.sha256(), mb.sha256(), "the model cannot tell them apart");
    assert_eq!(
        ma,
        ModelInput::compute(&a.audio, a.report.sample_rate, &ca).unwrap()
    );
    let la = l
        .encode(&up, &Hearing::sample(&ca, &one(48), 3000).unwrap())
        .unwrap();
    let lb = l
        .encode(l.template(), &Hearing::sample(&cb, &one(60), 3000).unwrap())
        .unwrap();
    assert_eq!(la, lb);
}

/// M1 over generator v1's targets heard at C3: each label's `osc1.semi` is the target's against the key the intake
/// reads, and decoding under the job (the intake's performance) gives it back, so a proposal plays at the clip's
/// pitch.
#[test]
fn labels_follow_the_intakes_key() {
    let l = OutputLayout::v1();
    let held = phrase(HELD).unwrap();
    let semi = l
        .continuous()
        .iter()
        .find(|e| e.addr.to_string() == "osc1.semi")
        .unwrap()
        .clone();
    let (mut rekeyed, mut checked, mut refused) = (0, 0, 0);
    for seed in 0..24u64 {
        let t = v1(9_200_000 + seed);
        let it = intake(&t, held.notes, held.length_ms, SR);
        let cue = Cue::from_intake(&it);
        let hearing = Hearing::sample(&cue, held.notes, held.length_ms).unwrap();
        let labels = match l.encode(&t, &hearing) {
            Ok(labels) => labels,
            Err(e) => {
                assert_eq!(e.code(), "rekey", "seed {seed}: {e}");
                refused += 1;
                continue;
            }
        };
        rekeyed += usize::from(hearing.shift() != 0);
        if labels.mask[semi.offset] == 1 {
            let label = semi.spec.from_norm(labels.values[semi.offset]);
            let played = t.value(&semi.addr).unwrap() + 48.0;
            assert!(
                (label + f32::from(cue.key) - played).abs() < 1e-3,
                "seed {seed}: label {label} told {} played {played}",
                cue.key
            );
            let target = Target::from_intake(&it, DEFAULT_SEED);
            let job = Hearing::job(&cue, &target.perf).unwrap();
            let decoded = l.decode(&labels.values, &job).unwrap();
            let back = decoded.value(&semi.addr).unwrap();
            assert!(
                (back - label).abs() < 1e-3,
                "seed {seed}: {back} vs {label}"
            );
            checked += 1;
        }
    }
    println!("{rekeyed} re-keyed, {checked} checked, {refused} refused of 24");
    // The judge measured the intake reading another key than C3 in 26 of 40 targets.
    assert!(rekeyed >= 6, "{rekeyed} re-keyed");
    assert!(checked >= 12, "{checked} checked");
    assert!(refused <= 4, "{refused} refused");
}

/// A dense multitone (noise-like) in 200-6000 Hz and a 220 Hz harmonic tone, as one continuous-time signal sampled
/// at `sr`, after 0.4 s of silence.
fn tone(sr: u32, dense: bool) -> Vec<f32> {
    let n = (1.5 * f64::from(sr)) as usize;
    let mut x = vec![0f64; n];
    let comps: Vec<(f64, f64, f64)> = if dense {
        let mut s = 0x9e37_79b9_7f4a_7c15u64;
        let mut u = || {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            (s >> 11) as f64 / (1u64 << 53) as f64
        };
        (0..1500)
            .map(|_| {
                (
                    200.0 + 5800.0 * u(),
                    core::f64::consts::TAU * u(),
                    1.0 / 1500f64.sqrt(),
                )
            })
            .collect()
    } else {
        (1..=27)
            .map(|h| (220.0 * f64::from(h), 0.0, 0.3 / f64::from(h)))
            .collect()
    };
    for (f, ph, a) in comps {
        let w = core::f64::consts::TAU * f / f64::from(sr);
        let (mut c, mut s) = (libm::cos(ph), libm::sin(ph));
        let (cw, sw) = (libm::cos(w), libm::sin(w));
        for v in &mut x {
            *v += a * s;
            let c2 = c * cw - s * sw;
            s = s * cw + c * sw;
            c = c2;
        }
    }
    let lead = (0.4 * f64::from(sr)) as usize;
    let mut out = vec![0f32; lead];
    let fade = (0.01 * f64::from(sr)) as usize;
    out.extend(x.iter().enumerate().map(|(i, v)| {
        let g = (i as f64 / fade as f64).min(1.0);
        (v * g * 0.25) as f32
    }));
    out
}

/// M4, the judge's `probe_rate_levels`: the same signal at 16-96 kHz reads the same band levels as at 48 kHz
/// (mean ln difference over blocks 40-100 and the bands below 6 kHz). With fixed FFT sizes a noise band moved
/// +0.54 nepers at 16 kHz and -0.35 at 96 kHz.
#[test]
fn band_levels_do_not_follow_the_rate() {
    let cue = Cue {
        key: 57,
        f0_hz: None,
        on_s: 0.4,
        hold_s: 1.5,
        bandwidth_hz: None,
    };
    let mut worst = 0.0f64;
    let mut report = String::new();
    for dense in [true, false] {
        let base = ModelInput::compute(&tone(48_000, dense), 48_000, &cue).unwrap();
        for sr in [96_000u32, 44_100, 32_000, 22_050, 16_000] {
            let m = ModelInput::compute(&tone(sr, dense), sr, &cue).unwrap();
            for r in 0..3 {
                let mut acc = 0.0f64;
                let mut k = 0;
                for t in 40..100 {
                    for b in 6..40 {
                        acc += f64::from(m.mel_at(r, t, b) - base.mel_at(r, t, b));
                        k += 1;
                    }
                }
                let mean = acc / f64::from(k);
                worst = worst.max(mean.abs());
                let _ = writeln!(
                    report,
                    "{} {sr} Hz r{r}: {mean:+.3}",
                    if dense { "multitone" } else { "harmonic" }
                );
            }
        }
    }
    println!("{report}");
    // Measured at F3S: at most 0.013 nepers (16 kHz); the judge's aim was 0.05.
    assert!(worst <= 0.025, "worst {worst:.3} nepers:\n{report}");
}

/// The cue comes from the intake, never from an override: a `--perf` override on the target changes the job's
/// hearing (its key and hold), not what the model is told.
#[test]
fn an_override_changes_the_job_not_the_cue() {
    let l = OutputLayout::v1();
    let it = intake(l.template(), &one(57), 3000, SR);
    let cue = Cue::from_intake(&it);
    let mut target = Target::from_intake(&it, DEFAULT_SEED);
    target.perf.key = cue.key + 12;
    target.perf.hold_s = 0.5;
    let job = Hearing::job(&cue, &target.perf).unwrap();
    assert_eq!(job.told_key(), cue.key);
    assert_eq!(job.shift(), 12);
    assert_eq!(job.phrase().notes[0].hold_ms, 500);
    assert_eq!(
        ModelInput::from_intake(&it).unwrap().context[0],
        f32::from(cue.key) - 60.0
    );
}
