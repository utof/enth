//! `wt` unison end to end (S2a lane UNI): the stereo flag, `spread` 0 equal to mono on both channels bit for bit, a
//! centred voice widened symmetrically (a correlation test), what an audio route leaving a stereo `wt` carries, an
//! `osc` beside a stereo `wt`, separate state per instance, and the host block size not being an input. The lanes
//! themselves are proved equal to plain oscillator voices, bit for bit, in `enth-dsp`'s `wt::unison` tests; the
//! seeds, the swaps, the 40 s tail and the aliasing are in this file's siblings (`wt_unison_*`) and the module's own
//! tests.
//!
//! The rig is `tests/wt.rs`'s: a gate envelope (no attack, decay or release; sustain 100 %), so the amplitude is
//! constant and the copies' own beating is all that moves.

mod common;

use common::{Note, SEED, SR, bits, drive, events, id, note};
use enth_engine::stereo::patch_needs_stereo;
use enth_engine::{Engine, PrepareCfg, StereoMode, compile_with};
use enth_patch::catalogue::{
    AdsrParam, FilterMode, NoiseParam, OscParam, PhaseMode, SvfParam, VoiceParam, Wave, WtParam,
};
use enth_patch::{ModuleInstance, ModuleKind, Patch, Route};

// ---------------------------------------------------------------------------------------------------------------
// The rig
// ---------------------------------------------------------------------------------------------------------------

/// `Patch::init()` without generators, under a gate envelope.
fn gated() -> Patch {
    let mut p = Patch::init();
    p.generators.clear();
    let env = &mut p.modulators[0].values;
    env[AdsrParam::Attack.index()] = 0.0;
    env[AdsrParam::Decay.index()] = 0.0;
    env[AdsrParam::Sustain.index()] = 100.0;
    env[AdsrParam::Release.index()] = 0.0;
    p
}

/// A `wt` at its defaults with `set` applied.
fn wt(name: &str, set: &[(WtParam, f32)]) -> ModuleInstance {
    let mut m = ModuleInstance::new(id(name), ModuleKind::Wt);
    for &(param, value) in set {
        m.values[param.index()] = value;
    }
    m
}

/// A saw `osc` at `level` in the `reset` phase mode.
fn osc(name: &str, level: f32) -> ModuleInstance {
    let mut m = ModuleInstance::new(id(name), ModuleKind::Osc);
    m.values[OscParam::Wave.index()] = Wave::Saw.index() as f32;
    m.values[OscParam::PhaseMode.index()] = PhaseMode::Reset.index() as f32;
    m.values[OscParam::Level.index()] = level;
    m
}

fn route(src: &str, dst: &str, depth: f32) -> Route {
    Route {
        src: src.parse().unwrap(),
        dst: dst.parse().unwrap(),
        depth,
    }
}

/// The unison knobs: `count` copies at `detune` cents, `blend` % and `spread` %.
fn knobs(count: u32, detune: f32, blend: f32, spread: f32) -> [(WtParam, f32); 4] {
    [
        (WtParam::Unison, count as f32),
        (WtParam::Detune, detune),
        (WtParam::Blend, blend),
        (WtParam::Spread, spread),
    ]
}

/// A lone `wt1` with `count` copies, on the `basic` table at a saw-like position.
fn unison_patch(count: u32, detune: f32, blend: f32, spread: f32) -> Patch {
    let mut p = gated();
    let mut set = vec![(WtParam::Position, 66.0)];
    set.extend(knobs(count, detune, blend, spread));
    p.generators.push(wt("wt1", &set));
    p
}

fn render_mode(
    patch: &Patch,
    mode: StereoMode,
    notes: &[Note],
    frames: usize,
    block: usize,
) -> (Vec<f32>, Vec<f32>) {
    let (plan, _) = compile_with(patch, &PrepareCfg::new(SR), mode).expect("the patch compiles");
    let mut e = Engine::new(plan, SEED);
    drive(&mut e, &events(notes), frames, block, |_, _, _| {})
}

fn chord(len: u64) -> Vec<Note> {
    vec![
        note(48, 0, len),
        note(55, 0, len),
        note(60, 200, len),
        note(64, 2_000, len + 4_000),
    ]
}

/// Both channels of `patch` with the stereo flag as set by the patch (`Auto`) and forced off, equal bit for bit at
/// several host blocks; the mono render must not be silent.
fn assert_equal_to_mono(name: &str, patch: &Patch, notes: &[Note], frames: usize) {
    for block in [32, 100, 512] {
        let (ml, mr) = render_mode(patch, StereoMode::Off, notes, frames, block);
        let (sl, sr) = render_mode(patch, StereoMode::Auto, notes, frames, block);
        assert!(
            ml.iter().any(|x| x.abs() > 0.05),
            "{name}: the mono render is silent, so the comparison would prove nothing"
        );
        assert_eq!(bits(&sl), bits(&ml), "{name}: left, block {block}");
        assert_eq!(bits(&sr), bits(&mr), "{name}: right, block {block}");
    }
}

// ---------------------------------------------------------------------------------------------------------------
// The flag
// ---------------------------------------------------------------------------------------------------------------

#[test]
fn the_flag_is_an_enabled_wt_with_unison_of_two_or_more_and_nothing_else() {
    let flag = |p: &Patch| {
        let want = patch_needs_stereo(p);
        let (auto, _) = compile_with(p, &PrepareCfg::new(SR), StereoMode::Auto).unwrap();
        assert_eq!(auto.is_stereo(), want, "Auto follows patch_needs_stereo");
        let (forced, _) = compile_with(p, &PrepareCfg::new(SR), StereoMode::Forced).unwrap();
        assert!(forced.is_stereo(), "Forced sets it");
        let (off, _) = compile_with(p, &PrepareCfg::new(SR), StereoMode::Off).unwrap();
        assert!(!off.is_stereo(), "Off clears it");
        want
    };
    assert!(!flag(&Patch::init()), "an init patch is mono");
    assert!(!flag(&unison_patch(1, 20.0, 100.0, 50.0)), "unison 1");
    for n in 2..=8 {
        assert!(flag(&unison_patch(n, 20.0, 100.0, 50.0)), "unison {n}");
    }
    // Spread 0 is still a stereo plan: the flag is structural, never keyed to a modulatable value.
    assert!(flag(&unison_patch(4, 0.0, 0.0, 0.0)));
    // A disabled wt does not count, an enabled one beside it does; a wt in any position among the generators.
    let mut p = unison_patch(4, 20.0, 100.0, 50.0);
    p.generators[0].enabled = false;
    assert!(!flag(&p), "a disabled wt");
    p.generators.insert(0, osc("osc1", 100.0));
    p.generators.push(wt("wt2", &knobs(3, 20.0, 100.0, 50.0)));
    assert!(flag(&p), "an enabled second wt");
    p.generators[2].enabled = false;
    assert!(!flag(&p), "both wts disabled");
}

// ---------------------------------------------------------------------------------------------------------------
// spread 0 equals mono
// ---------------------------------------------------------------------------------------------------------------

#[test]
fn spread_zero_equals_mono_on_both_channels_for_every_count_and_phase_mode() {
    for n in 2..=8 {
        for mode in [PhaseMode::Reset, PhaseMode::Free, PhaseMode::Seeded] {
            let mut p = unison_patch(n, 35.0, 60.0, 0.0);
            p.generators[0].values[WtParam::PhaseMode.index()] = mode.index() as f32;
            p.generators[0].values[WtParam::Seed.index()] = 9.0;
            assert_equal_to_mono(&format!("unison {n}, {mode:?}"), &p, &chord(9_000), 12_000);
        }
    }
}

#[test]
fn spread_zero_equals_mono_with_neighbours_routes_effects_pans_and_steals() {
    // An osc and a noise beside the wt, in both orders; a voice chain; a pan; polyphony 3 with five notes (steals).
    let mut crowd = gated();
    crowd.generators.push(osc("osc1", 60.0));
    let mut set = vec![(WtParam::Semi, -12.0)];
    set.extend(knobs(5, 60.0, 80.0, 0.0));
    crowd.generators.push(wt("wt1", &set));
    let mut noise = ModuleInstance::new(id("noise1"), ModuleKind::Noise);
    noise.values[NoiseParam::Level.index()] = 10.0;
    crowd.generators.push(noise);
    crowd
        .voice_fx
        .push(ModuleInstance::new(id("filter1"), ModuleKind::Svf));
    crowd.voice.values[VoiceParam::Pan.index()] = -35.0;
    crowd.voice.values[VoiceParam::Polyphony.index()] = 3.0;
    let notes = [
        note(36, 0, 8_000),
        note(43, 500, 8_000),
        note(52, 1_000, 9_000),
        note(59, 3_000, 9_000),
        note(64, 4_000, 11_000),
    ];
    assert_equal_to_mono("crowd", &crowd, &notes, 14_000);

    // The same with the wt first.
    let mut first = crowd.clone();
    first.generators.swap(0, 1);
    assert_equal_to_mono("wt first", &first, &notes, 14_000);

    // Audio routes: a stereo wt modulated by an osc, and a stereo wt feeding a plain wt's pm and am.
    let mut routed = gated();
    routed.generators.push(osc("osc1", 0.0));
    let mut set = vec![(WtParam::PmIndex, 3.0), (WtParam::Feedback, 20.0)];
    set.extend(knobs(4, 40.0, 100.0, 0.0));
    routed.generators.push(wt("wt1", &set));
    routed
        .generators
        .push(wt("wt2", &[(WtParam::PmIndex, 2.0), (WtParam::Am, 60.0)]));
    routed.routes.push(route("osc1", "wt1.pm", 70.0));
    routed.routes.push(route("wt1", "wt2.pm", 50.0));
    routed.routes.push(route("wt1", "wt2.am", 30.0));
    assert_equal_to_mono("routed", &routed, &notes, 14_000);

    // A routed stereo wt that is itself a carrier of another stereo wt.
    let mut chain = gated();
    chain
        .generators
        .push(wt("wt1", &knobs(3, 25.0, 100.0, 0.0)));
    let mut set = vec![(WtParam::Semi, 7.0), (WtParam::PmIndex, 2.0)];
    set.extend(knobs(6, 15.0, 70.0, 0.0));
    chain.generators.push(wt("wt2", &set));
    chain.routes.push(route("wt1", "wt2.pm", 80.0));
    assert_equal_to_mono("chain", &chain, &notes, 14_000);
}

// ---------------------------------------------------------------------------------------------------------------
// Widening
// ---------------------------------------------------------------------------------------------------------------

/// The correlation coefficient of `l` and `r`, and their power ratio in dB.
fn correlation(l: &[f32], r: &[f32]) -> (f64, f64) {
    let (mut ll, mut rr, mut lr) = (0.0f64, 0.0f64, 0.0f64);
    for (&a, &b) in l.iter().zip(r) {
        let (a, b) = (f64::from(a), f64::from(b));
        ll += a * a;
        rr += b * b;
        lr += a * b;
    }
    (
        lr / (ll * rr).sqrt(),
        10.0 * enth_dsp::util::math::log10(ll / rr),
    )
}

#[test]
fn spread_widens_a_centred_voice_symmetrically() {
    // One second of key 48, unison 7 across 30 cents, from the attack on.
    let frames = 48_000;
    let notes = [note(48, 0, frames as u64)];
    let mut rows = Vec::new();
    let mut last = 2.0;
    for spread in [0.0f32, 10.0, 25.0, 50.0, 75.0, 100.0] {
        let p = unison_patch(7, 30.0, 100.0, spread);
        let (l, r) = render_mode(&p, StereoMode::Auto, &notes, frames, 512);
        let (c, balance) = correlation(&l[2_000..], &r[2_000..]);
        rows.push(format!(
            "spread {spread:>5.1} %: corr {c:.4}, L/R {balance:+.3} dB"
        ));
        // Wider is less correlated, strictly; and the image does not lean: the two sides carry the same power.
        assert!(c < last, "corr must fall with spread\n{}", rows.join("\n"));
        last = c;
        assert!(
            balance.abs() < 0.35,
            "the power of the two sides differs by {balance:.3} dB at spread {spread}\n{}",
            rows.join("\n")
        );
        if spread == 0.0 {
            assert_eq!(bits(&l), bits(&r), "spread 0 is the same signal twice");
        }
    }
    eprintln!("{}", rows.join("\n"));
    assert!(last < 0.6, "fully spread is wide: corr {last:.3}");

    // Two copies at the extremes are hard left and hard right: two unrelated signals, uncorrelated.
    let p = unison_patch(2, 30.0, 100.0, 100.0);
    let (l, r) = render_mode(&p, StereoMode::Auto, &notes, frames, 512);
    let (c, balance) = correlation(&l[2_000..], &r[2_000..]);
    eprintln!("unison 2, spread 100 %: corr {c:.4}, L/R {balance:+.3} dB");
    assert!(c.abs() < 0.15, "two hard-panned copies: corr {c:.3}");
    assert!(
        balance.abs() < 0.35,
        "two hard-panned copies: {balance:.3} dB"
    );
}

#[test]
fn a_wide_voice_is_balanced_by_the_pan_and_a_hard_pan_keeps_only_one_side() {
    let frames = 24_000;
    let notes = [note(48, 0, frames as u64)];
    let mut p = unison_patch(7, 30.0, 100.0, 100.0);
    p.voice.values[VoiceParam::Pan.index()] = -100.0;
    let (l, r) = render_mode(&p, StereoMode::Auto, &notes, frames, 512);
    assert!(l.iter().any(|x| x.abs() > 0.05));
    assert!(
        r.iter().all(|x| *x == 0.0),
        "hard left drops the right channel"
    );
    p.voice.values[VoiceParam::Pan.index()] = 100.0;
    let (l, r) = render_mode(&p, StereoMode::Auto, &notes, frames, 512);
    assert!(r.iter().any(|x| x.abs() > 0.05));
    assert!(
        l.iter().all(|x| *x == 0.0),
        "hard right drops the left channel"
    );
}

// ---------------------------------------------------------------------------------------------------------------
// What a route carries
// ---------------------------------------------------------------------------------------------------------------

#[test]
fn a_route_leaving_a_stereo_wt_carries_the_mono_mix_whatever_the_spread() {
    // wt1 (7 copies) is silent (level 0) and only modulates wt2's phase and amplitude; wt2 is a plain voice, so what
    // is heard depends on the route's signal alone.
    let notes = chord(10_000);
    let rig = |spread: f32| {
        let mut p = gated();
        let mut set = vec![(WtParam::Level, 0.0)];
        set.extend(knobs(7, 45.0, 70.0, spread));
        p.generators.push(wt("wt1", &set));
        p.generators.push(wt(
            "wt2",
            &[
                (WtParam::PmIndex, 3.0),
                (WtParam::Am, 50.0),
                (WtParam::Semi, 12.0),
            ],
        ));
        p.routes.push(route("wt1", "wt2.pm", 60.0));
        p.routes.push(route("wt1", "wt2.am", 40.0));
        p
    };
    let reference = render_mode(&rig(0.0), StereoMode::Off, &notes, 14_000, 512);
    assert!(reference.0.iter().any(|x| x.abs() > 0.05));
    for spread in [0.0f32, 35.0, 100.0] {
        let (plan, _) = compile_with(&rig(spread), &PrepareCfg::new(SR), StereoMode::Auto).unwrap();
        assert!(plan.is_stereo());
        let mono_flag = render_mode(&rig(spread), StereoMode::Off, &notes, 14_000, 512);
        let stereo = render_mode(&rig(spread), StereoMode::Auto, &notes, 14_000, 512);
        // The modulator's own contribution is silent, so the voice is wt2 alone and it is the same on both channels
        // and at every spread: the route carried the mono mix.
        assert_eq!(bits(&stereo.0), bits(&reference.0), "spread {spread}: left");
        assert_eq!(
            bits(&stereo.1),
            bits(&reference.1),
            "spread {spread}: right"
        );
        assert_eq!(
            bits(&mono_flag.0),
            bits(&reference.0),
            "spread {spread}: off"
        );
    }
}

#[test]
fn the_route_signal_is_the_mono_mix_after_am_and_before_level() {
    // A stereo wt with level 100 and one with level 0 modulate identical carriers identically: the level ramp is
    // after the route's tap, and `am` is before it.
    let notes = [note(50, 0, 9_000)];
    let rig = |level: f32, am: f32| {
        let mut p = gated();
        let mut set = vec![(WtParam::Level, level), (WtParam::Am, am)];
        set.extend(knobs(3, 30.0, 100.0, 80.0));
        p.generators.push(wt("wt1", &set));
        p.generators
            .push(wt("wt2", &[(WtParam::PmIndex, 4.0), (WtParam::Level, 0.0)]));
        p.routes.push(route("wt1", "wt2.pm", 100.0));
        p
    };
    // wt2 is silent (level 0), so isolate the route: compare wt2's signal through wt1.level only by making wt1
    // silent in both; the outputs are zero and the carriers' spectra are not observable, so route into a heard wt3.
    let heard = |level: f32, am: f32| {
        let mut p = rig(level, am);
        p.generators[1].values[WtParam::Level.index()] = 100.0;
        p
    };
    let a = render_mode(&heard(0.0, 100.0), StereoMode::Auto, &notes, 12_000, 512);
    let b = render_mode(&heard(0.0, 100.0), StereoMode::Off, &notes, 12_000, 512);
    assert_eq!(bits(&a.0), bits(&b.0));
    // wt1's `am` scales what travels, so a different `am` changes wt2's phase excursion.
    let c = render_mode(&heard(0.0, 50.0), StereoMode::Auto, &notes, 12_000, 512);
    assert_ne!(bits(&a.0), bits(&c.0), "am is before the route's tap");
    // ... while `level` is after it: with the carrier's pan centred, wt2's own contribution is unchanged by wt1's
    // level when wt1's level moves from 0 to 100 and wt1's own sound is subtracted.
    let loud = render_mode(&heard(100.0, 100.0), StereoMode::Auto, &notes, 12_000, 512);
    let solo = {
        let mut p = heard(100.0, 100.0);
        p.generators[1].values[WtParam::Level.index()] = 0.0;
        render_mode(&p, StereoMode::Auto, &notes, 12_000, 512)
    };
    for i in 0..12_000 {
        let carrier = loud.0[i] - solo.0[i];
        assert!(
            (carrier - a.0[i]).abs() < 1e-5,
            "level after the tap: sample {i}: {carrier} vs {}",
            a.0[i]
        );
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Neighbours and separate state
// ---------------------------------------------------------------------------------------------------------------

/// The max absolute difference between two renders.
fn max_diff(a: &[f32], b: &[f32]) -> f32 {
    a.iter()
        .zip(b)
        .map(|(x, y)| (x - y).abs())
        .fold(0.0, f32::max)
}

#[test]
fn an_osc_beside_a_stereo_wt_lands_on_both_channels_as_it_does_in_mono() {
    let notes = chord(9_000);
    let frames = 12_000;
    for wt_first in [false, true] {
        for routed in [false, true] {
            let build = |osc_level: f32, wt_level: f32| {
                let mut p = gated();
                let mut set = vec![(WtParam::Level, wt_level), (WtParam::PmIndex, 2.0)];
                set.extend(knobs(5, 50.0, 100.0, 100.0));
                let w = wt("wt1", &set);
                let o = osc("osc1", osc_level);
                if wt_first {
                    p.generators.push(w);
                    p.generators.push(o);
                } else {
                    p.generators.push(o);
                    p.generators.push(w);
                }
                if routed {
                    p.routes.push(route("osc1", "wt1.pm", 20.0));
                }
                p
            };
            let name = format!("wt first {wt_first}, routed {routed}");
            let both = render_mode(&build(70.0, 80.0), StereoMode::Auto, &notes, frames, 100);
            let osc_only = render_mode(&build(70.0, 0.0), StereoMode::Auto, &notes, frames, 100);
            let wt_only = render_mode(&build(0.0, 80.0), StereoMode::Auto, &notes, frames, 100);
            assert!(
                osc_only.0.iter().any(|x| x.abs() > 0.05),
                "{name}: the osc is heard"
            );
            assert_eq!(
                bits(&osc_only.0),
                bits(&osc_only.1),
                "{name}: the osc alone is centred"
            );
            // The osc's share is the same in both channels, and the wt's share is wide (not the same in both).
            for ch in 0..2 {
                let (b, o, w) = if ch == 0 {
                    (&both.0, &osc_only.0, &wt_only.0)
                } else {
                    (&both.1, &osc_only.1, &wt_only.1)
                };
                let sum: Vec<f32> = o.iter().zip(w).map(|(x, y)| x + y).collect();
                // The routed osc modulates the wt's phase only while the osc has level; its pre-level signal travels
                // at any level, so the wt's share is unchanged and the sum is exact up to rounding.
                let d = max_diff(b, &sum);
                assert!(d < 2e-5, "{name}: channel {ch}: osc + wt differs by {d}");
            }
            assert!(
                max_diff(&wt_only.0, &wt_only.1) > 0.05,
                "{name}: the wt is wide beside the osc"
            );
        }
    }
}

#[test]
fn two_unison_wts_keep_separate_state() {
    let notes = chord(9_000);
    let frames = 12_000;
    let a_set: Vec<_> = [(WtParam::Semi, 0.0f32)]
        .into_iter()
        .chain(knobs(4, 30.0, 100.0, 70.0))
        .collect();
    let b_set: Vec<_> = [(WtParam::Semi, 7.0f32), (WtParam::Feedback, 25.0)]
        .into_iter()
        .chain(knobs(6, 12.0, 50.0, 40.0))
        .collect();
    let single = |which: u8| {
        let mut p = gated();
        if which == 0 {
            p.generators.push(wt("wt1", &a_set));
            p.generators.push(wt("wt2", &[(WtParam::Level, 0.0)]));
        } else {
            p.generators.push(wt("wt1", &[(WtParam::Level, 0.0)]));
            p.generators.push(wt("wt2", &b_set));
        }
        p
    };
    let mut both = gated();
    both.generators.push(wt("wt1", &a_set));
    both.generators.push(wt("wt2", &b_set));
    let (l, r) = render_mode(&both, StereoMode::Auto, &notes, frames, 512);
    let (la, ra) = render_mode(&single(0), StereoMode::Auto, &notes, frames, 512);
    let (lb, rb) = render_mode(&single(1), StereoMode::Auto, &notes, frames, 512);
    let sum_l: Vec<f32> = la.iter().zip(&lb).map(|(x, y)| x + y).collect();
    let sum_r: Vec<f32> = ra.iter().zip(&rb).map(|(x, y)| x + y).collect();
    assert!(
        max_diff(&l, &sum_l) < 2e-5,
        "left: {}",
        max_diff(&l, &sum_l)
    );
    assert!(
        max_diff(&r, &sum_r) < 2e-5,
        "right: {}",
        max_diff(&r, &sum_r)
    );
    assert!(max_diff(&la, &lb) > 0.1, "the two differ");
}

#[test]
fn the_host_block_size_is_not_an_input_for_a_stereo_wt() {
    let mut p = gated();
    p.generators.push(osc("osc1", 0.0));
    let mut set = vec![
        (WtParam::PmIndex, 3.0),
        (WtParam::Feedback, 30.0),
        (WtParam::Am, 80.0),
    ];
    set.extend(knobs(7, 55.0, 60.0, 90.0));
    p.generators.push(wt("wt1", &set));
    p.generators.push(wt("wt2", &knobs(2, 8.0, 100.0, 100.0)));
    p.routes.push(route("osc1", "wt1.pm", 80.0));
    p.routes.push(route("wt1", "wt2.pm", 30.0));
    let notes = [
        note(48, 0, 9_000),
        note(55, 1_500, 9_000),
        note(64, 2_001, 7_000),
    ];
    let (reference, rr) = render_mode(&p, StereoMode::Auto, &notes, 12_000, 512);
    assert!(reference.iter().any(|x| x.abs() > 0.1));
    assert!(max_diff(&reference, &rr) > 0.05, "the render is wide");
    for block in [1usize, 7, 32, 64, 100, 4_096] {
        let (l, r) = render_mode(&p, StereoMode::Auto, &notes, 12_000, block);
        assert_eq!(bits(&l), bits(&reference), "left at block {block}");
        assert_eq!(bits(&r), bits(&rr), "right at block {block}");
    }
}

#[test]
fn a_filter_after_a_stereo_wt_runs_once_per_channel() {
    // The right chain is its own instance: a filter with state sees the right channel, not a copy of the left.
    let mut p = unison_patch(5, 40.0, 100.0, 100.0);
    let mut f = ModuleInstance::new(id("filter1"), ModuleKind::Svf);
    f.values[SvfParam::Mode.index()] = FilterMode::Lp.index() as f32;
    f.values[SvfParam::Cutoff.index()] = 900.0;
    p.voice_fx.push(f);
    let notes = [note(48, 0, 20_000)];
    let (l, r) = render_mode(&p, StereoMode::Auto, &notes, 24_000, 512);
    let (c, _) = correlation(&l[4_000..], &r[4_000..]);
    assert!(
        c < 0.8,
        "the channels stay different through the filter: corr {c:.3}"
    );
    assert!(l.iter().chain(&r).all(|x| x.is_finite()));
}
