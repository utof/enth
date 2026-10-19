//! Lane D1 through the whole engine: the `osc` and `noise` modules in compiled patches. Pitch, aliasing, hard sync,
//! seeds, parameter events, block-size invariance and the release tail. The module contract itself (start phases, state
//! copies, the sync bus) is in `d1_modules.rs`.

mod common;

use common::{SEED, SR, bits, events, id, note, render};
use enth_dsp::util::math;
use enth_engine::{EventKind, PrepareCfg, compile};
use enth_params::Kind;
use enth_patch::catalogue::{
    AdsrParam, NoiseColor, NoiseParam, OscParam, PhaseMode, Wave, Wavetable,
};
use enth_patch::{ModuleInstance, ModuleKind, Patch};

/// `Patch::init()` with a gate envelope: the voice level is exactly 1 while the key is held, so module goldens and
/// tests do not depend on lane D2's envelope.
fn gated(mut p: Patch) -> Patch {
    let env = &mut p.modulators[0].values;
    env[AdsrParam::Attack.index()] = 0.0;
    env[AdsrParam::Decay.index()] = 0.0;
    env[AdsrParam::Sustain.index()] = 100.0;
    env[AdsrParam::Release.index()] = 0.0;
    env[AdsrParam::Velocity.index()] = 0.0;
    p
}

fn osc_patch(edit: impl FnOnce(&mut [f32])) -> Patch {
    let mut p = gated(Patch::init());
    edit(&mut p.generators[0].values);
    p
}

fn set(v: &mut [f32], p: OscParam, x: f32) {
    v[p.index()] = x;
}

fn wave(w: Wave) -> f32 {
    w.index() as f32
}

fn noise_patch(color: NoiseColor, level: f32) -> Patch {
    let mut p = gated(Patch::init());
    let mut n = ModuleInstance::new(id("noise1"), ModuleKind::Noise);
    n.values[NoiseParam::Color.index()] = color.index() as f32;
    n.values[NoiseParam::Level.index()] = level;
    p.generators = vec![n];
    p
}

/// Mono signal of a one-note render (the centre pan makes left equal right).
fn tone(p: &Patch, key: u8, seconds: f64, seed: u64) -> Vec<f32> {
    let frames = (seconds * f64::from(SR)) as usize;
    render(p, &[note(key, 0, frames as u64)], frames, 512, seed).0
}

fn key_hz(key: u8) -> f64 {
    440.0 * math::exp2((f64::from(key) - 69.0) / 12.0)
}

/// The frequency from rising zero crossings (linear interpolation), over `x[from..]`.
fn measured_hz(x: &[f32], from: usize) -> f64 {
    let mut first = None;
    let mut last = 0.0;
    let mut count = 0usize;
    for i in from..x.len() - 1 {
        if x[i] < 0.0 && x[i + 1] >= 0.0 {
            let t = i as f64 + f64::from(-x[i]) / f64::from(x[i + 1] - x[i]);
            first.get_or_insert(t);
            last = t;
            count += 1;
        }
    }
    (count - 1) as f64 * f64::from(SR) / (last - first.unwrap())
}

#[test]
fn every_wave_plays_the_note_at_its_pitch() {
    for (w, table) in [
        (Wave::Sine, Wavetable::Basic),
        (Wave::Triangle, Wavetable::Basic),
        (Wave::Saw, Wavetable::Basic),
        (Wave::Pulse, Wavetable::Basic),
        (Wave::Table, Wavetable::Basic),
        (Wave::Table, Wavetable::Organ),
    ] {
        for key in [33u8, 69, 105] {
            // Position 0 of `basic` is a sine and of `organ` the first drawbar blend: both have a single rising zero
            // crossing per cycle that the interpolation finds exactly.
            let p = osc_patch(|v| {
                set(v, OscParam::Wave, wave(w));
                set(v, OscParam::Table, table.index() as f32);
            });
            let x = tone(&p, key, 4.0, SEED);
            let hz = measured_hz(&x, 4800);
            let want = key_hz(key);
            assert!(
                (hz / want - 1.0).abs() < 1e-4,
                "{w:?}/{table:?} key {key}: {hz} Hz, wanted {want}"
            );
        }
    }
}

#[test]
fn semitones_and_cents_transpose_the_pitch() {
    for (semi, fine) in [(12.0, 0.0), (-7.0, 0.0), (0.0, 50.0), (3.0, -25.0)] {
        let p = osc_patch(|v| {
            set(v, OscParam::Wave, wave(Wave::Sine));
            set(v, OscParam::Semi, semi);
            set(v, OscParam::Fine, fine);
        });
        let hz = measured_hz(&tone(&p, 60, 3.0, SEED), 4800);
        let want =
            440.0 * math::exp2((60.0 - 69.0 + f64::from(semi) + f64::from(fine) / 100.0) / 12.0);
        assert!(
            (hz / want - 1.0).abs() < 1e-4,
            "{semi} st {fine} ct: {hz} vs {want}"
        );
    }
}

// --- spectra ---------------------------------------------------------------------------------------------------------

fn fft(re: &mut [f64], im: &mut [f64]) {
    let n = re.len();
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let ang = -std::f64::consts::TAU / len as f64;
        for start in (0..n).step_by(len) {
            for k in 0..len / 2 {
                let (wr, wi) = (math::cos(ang * k as f64), math::sin(ang * k as f64));
                let (a, b) = (start + k, start + k + len / 2);
                let (tr, ti) = (re[b] * wr - im[b] * wi, re[b] * wi + im[b] * wr);
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
            }
        }
        len <<= 1;
    }
}

const N: usize = 32_768;

/// `|X|²` of `N` samples from `start` under a 4-term Blackman-Harris window.
fn power_spectrum(x: &[f32], start: usize) -> Vec<f64> {
    let tau = std::f64::consts::TAU;
    let mut re: Vec<f64> = (0..N)
        .map(|i| {
            let t = i as f64 / (N - 1) as f64;
            let w = 0.35875 - 0.48829 * math::cos(tau * t) + 0.14128 * math::cos(2.0 * tau * t)
                - 0.01168 * math::cos(3.0 * tau * t);
            f64::from(x[start + i]) * w
        })
        .collect();
    let mut im = vec![0.0; N];
    fft(&mut re, &mut im);
    (0..=N / 2).map(|i| re[i] * re[i] + im[i] * im[i]).collect()
}

/// Which bins lie within `half` bins of a multiple of `f0` below Nyquist (DC included: a restarted saw has a mean).
fn harmonic_bins(f0: f64, half: f64) -> Vec<bool> {
    let bin = f64::from(SR) / N as f64;
    let mut is = vec![false; N / 2 + 1];
    let mut k = 0.0;
    while k * f0 < f64::from(SR) / 2.0 {
        let c = k * f0 / bin;
        let (lo, hi) = (
            (c - half).floor().max(0.0) as usize,
            (c + half).ceil() as usize,
        );
        for h in is.iter_mut().take(hi.min(N / 2) + 1).skip(lo) {
            *h = true;
        }
        k += 1.0;
    }
    is
}

/// Research report 02's measure: power outside the harmonics of `f0` over power inside them, in dB, 20 Hz - 20 kHz.
fn alias_db(x: &[f32], f0: f64) -> f64 {
    let p = power_spectrum(x, 12_000);
    let bin = f64::from(SR) / N as f64;
    let is = harmonic_bins(f0, 8.0);
    let (mut sig, mut alias) = (0.0, 0.0);
    for (i, &v) in p.iter().enumerate() {
        let f = i as f64 * bin;
        if !(20.0..=20_000.0).contains(&f) {
            continue;
        }
        if is[i] {
            sig += v;
        } else {
            alias += v;
        }
    }
    10.0 * math::log10(alias / sig)
}

#[test]
fn a_high_saw_stays_under_report_02s_aliasing_thresholds() {
    // Report 02: saw at 440 Hz -40.6 dB, at 3520 Hz -28.8 dB (alias power over harmonic power, 20 Hz - 20 kHz).
    for (key, ceiling) in [(69u8, -40.0), (105, -28.0)] {
        let p = osc_patch(|v| {
            set(v, OscParam::Wave, wave(Wave::Saw));
            set(v, OscParam::PhaseMode, PhaseMode::Reset.index() as f32);
        });
        let db = alias_db(&tone(&p, key, 1.5, SEED), key_hz(key));
        eprintln!("saw key {key}: alias {db:.1} dB");
        assert!(db < ceiling, "saw key {key}: {db:.1} dB");
    }
}

#[test]
fn the_wavetable_aliases_far_less_than_the_polyblep_saw() {
    for table in [Wavetable::Basic, Wavetable::Tilt, Wavetable::Formant] {
        for key in [45u8, 69, 105] {
            for position in [0.0, 50.0, 100.0] {
                let p = osc_patch(|v| {
                    set(v, OscParam::Wave, wave(Wave::Table));
                    set(v, OscParam::Table, table.index() as f32);
                    set(v, OscParam::Position, position);
                });
                let db = alias_db(&tone(&p, key, 1.5, SEED), key_hz(key));
                assert!(db < -85.0, "{table:?} key {key} pos {position}: {db:.1} dB");
            }
        }
    }
}

#[test]
fn hard_sync_makes_any_slave_periodic_at_any_masters_pitch() {
    // osc1 is a silent master at 220 Hz (its level does not matter to the bus); osc2 is ~2.37x higher. Synced, every
    // harmonic of the output sits on a multiple of 220 Hz; free-running it sits on multiples of 521 Hz.
    let build = |master: Wave, slave: Wave, sync: bool| {
        let mut p = gated(Patch::init());
        set(&mut p.generators[0].values, OscParam::Wave, wave(master));
        set(&mut p.generators[0].values, OscParam::Level, 0.0);
        let mut o2 = ModuleInstance::new(id("osc2"), ModuleKind::Osc);
        set(&mut o2.values, OscParam::Wave, wave(slave));
        set(&mut o2.values, OscParam::Semi, 15.0);
        set(&mut o2.values, OscParam::Fine, -6.13);
        set(&mut o2.values, OscParam::Position, 40.0);
        set(&mut o2.values, OscParam::Sync, if sync { 1.0 } else { 0.0 });
        p.generators.push(o2);
        p
    };
    let share = |x: &[f32]| {
        let p = power_spectrum(x, 12_000);
        let is = harmonic_bins(key_hz(57), 6.0);
        let inside: f64 = p.iter().zip(&is).filter(|(_, h)| **h).map(|(v, _)| v).sum();
        inside / p.iter().sum::<f64>()
    };
    for master in [Wave::Sine, Wave::Saw, Wave::Pulse, Wave::Table] {
        for slave in [
            Wave::Saw,
            Wave::Triangle,
            Wave::Pulse,
            Wave::Sine,
            Wave::Table,
        ] {
            let synced = share(&tone(&build(master, slave, true), 57, 1.5, SEED));
            let free = share(&tone(&build(master, slave, false), 57, 1.5, SEED));
            eprintln!("master {master:?} slave {slave:?}: synced {synced:.4}, free {free:.4}");
            assert!(synced > 0.99, "master {master:?} slave {slave:?}: {synced}");
            assert!(free < 0.5, "master {master:?} slave {slave:?} free: {free}");
        }
    }
}

// --- seeds and phases ------------------------------------------------------------------------------------------------

#[test]
fn phase_modes_follow_the_render_seed_or_the_patch() {
    let two_notes = |p: &Patch, seed: u64| {
        let n = [note(69, 0, 9_600), note(69, 12_000, 21_600)];
        render(p, &n, 24_000, 512, seed).0
    };
    let mode = |m: PhaseMode, phase: f32, seed: f32| {
        osc_patch(|v| {
            set(v, OscParam::PhaseMode, m.index() as f32);
            set(v, OscParam::Phase, phase);
            set(v, OscParam::Seed, seed);
        })
    };
    // Free: the render seed decides, and each note differs.
    let free = mode(PhaseMode::Free, 0.0, 0.0);
    let (a, b) = (two_notes(&free, 1), two_notes(&free, 2));
    assert_ne!(
        bits(&a),
        bits(&b),
        "a different render seed moves free phases"
    );
    assert_eq!(bits(&a), bits(&two_notes(&free, 1)));
    assert_ne!(
        bits(&a[2000..4000]),
        bits(&a[14_000..16_000]),
        "two notes of one render start at different phases"
    );
    // Seeded: the patch decides, the render seed does not matter, the module seed does.
    let seeded = mode(PhaseMode::Seeded, 0.0, 77.0);
    assert_eq!(bits(&two_notes(&seeded, 1)), bits(&two_notes(&seeded, 2)));
    assert_ne!(
        bits(&two_notes(&seeded, 1)),
        bits(&two_notes(&mode(PhaseMode::Seeded, 0.0, 78.0), 1))
    );
    // Reset: both notes start the same way.
    let reset = mode(PhaseMode::Reset, 120.0, 0.0);
    let r = two_notes(&reset, 1);
    assert_eq!(bits(&r[..4000]), bits(&r[12_000..16_000]));
    assert_eq!(bits(&r), bits(&two_notes(&reset, 99)));
    assert_ne!(
        bits(&r[..4000]),
        bits(&two_notes(&mode(PhaseMode::Reset, 200.0, 0.0), 1)[..4000])
    );
}

#[test]
fn two_free_oscillators_with_the_same_settings_start_at_different_phases() {
    let one = gated(Patch::init());
    let mut two = gated(Patch::init());
    two.generators
        .push(ModuleInstance::new(id("osc2"), ModuleKind::Osc));
    let a = tone(&one, 69, 0.2, SEED);
    let b = tone(&two, 69, 0.2, SEED);
    // In phase the sum would be exactly twice the single saw (the module id is part of the seed).
    let doubled: Vec<u32> = a.iter().map(|x| (x * 2.0).to_bits()).collect();
    assert_ne!(bits(&b), doubled);
}

#[test]
fn noise_streams_are_per_note_and_per_render_seed_and_levels_scale_exactly() {
    let notes = [note(60, 0, 9_600), note(60, 12_000, 21_600)];
    let full = noise_patch(NoiseColor::White, 100.0);
    let (a, _, _) = render(&full, &notes, 24_000, 512, 5);
    let (a2, _, _) = render(&full, &notes, 24_000, 64, 5);
    let (b, _, _) = render(&full, &notes, 24_000, 512, 6);
    assert_eq!(bits(&a), bits(&a2), "block size and repeats do not matter");
    assert_ne!(bits(&a), bits(&b), "the render seed does");
    assert_ne!(
        bits(&a[2000..3000]),
        bits(&a[14_000..15_000]),
        "each note has its own stream"
    );
    assert!(a.iter().any(|x| x.abs() > 0.1));
    // Level: moving it never reseeds, and 50 % is exactly half of 100 % (the amp stage is linear).
    let (half, _, _) = render(
        &noise_patch(NoiseColor::White, 50.0),
        &notes,
        24_000,
        512,
        5,
    );
    for (h, f) in half.iter().zip(&a) {
        assert_eq!(h.to_bits(), (f * 0.5).to_bits());
    }
    // Colour: pink is a different signal from the same stream and is quieter than white.
    let (pink, _, _) = render(
        &noise_patch(NoiseColor::Pink, 100.0),
        &notes,
        24_000,
        512,
        5,
    );
    let rms = |x: &[f32]| {
        (x[2000..9000]
            .iter()
            .map(|v| f64::from(*v) * f64::from(*v))
            .sum::<f64>()
            / 7000.0)
            .sqrt()
    };
    assert!(rms(&pink) < rms(&a), "{} vs {}", rms(&pink), rms(&a));
    assert!(pink.iter().all(|x| x.abs() < 1.5));
}

#[test]
fn changing_noise_colour_mid_note_does_not_reseed() {
    let notes = [note(60, 0, 24_000)];
    let white = noise_patch(NoiseColor::White, 100.0);
    let (steady, _, _) = render(&white, &notes, 24_000, 512, 3);
    // Automate the colour to pink at frame 8,000: until then the two renders are bit-identical, and afterwards the
    // white stream carries on underneath (the pink filter starts from cold).
    let (plan, map) = compile(&white, &PrepareCfg::new(SR)).unwrap();
    let color = map.get(&"noise1.color".parse().unwrap()).unwrap();
    let mut e = enth_engine::Engine::new(plan, 3);
    let mut ev = events(&notes);
    ev.push((
        8_000,
        EventKind::Param {
            id: color,
            value: NoiseColor::Pink.index() as f32,
        },
    ));
    ev.sort_by_key(|(f, _)| *f);
    let (l, _) = common::drive(&mut e, &ev, 24_000, 512, |_, _, _| {});
    assert_eq!(bits(&l[..8_000]), bits(&steady[..8_000]));
    assert_ne!(bits(&l[8_192..9_000]), bits(&steady[8_192..9_000]));
}

// --- automation, block sizes, tails ----------------------------------------------------------------------------------

/// Absolute-frame parameter events for module parameters addressed by name.
fn param_events(patch: &Patch, sets: &[(u64, &str, f32)]) -> Vec<(u64, EventKind)> {
    let (_, map) = compile(patch, &PrepareCfg::new(SR)).unwrap();
    sets.iter()
        .map(|&(frame, addr, value)| {
            let id = map.get(&addr.parse().unwrap()).unwrap();
            (frame, EventKind::Param { id, value })
        })
        .collect()
}

fn run_with(patch: &Patch, ev: &[(u64, EventKind)], frames: usize, block: usize) -> Vec<f32> {
    let mut e = enth_engine::Engine::new(compile(patch, &PrepareCfg::new(SR)).unwrap().0, SEED);
    common::drive(&mut e, ev, frames, block, |_, _, _| {}).0
}

#[test]
fn switching_waves_and_scanning_the_table_mid_note_is_block_size_invariant() {
    // `table` is structural (formant); `wave` runs through every class, with `position` and `semi` moving too.
    let mut p = gated(Patch::init());
    set(
        &mut p.generators[0].values,
        OscParam::Table,
        Wavetable::Formant.index() as f32,
    );
    let mut o2 = ModuleInstance::new(id("osc2"), ModuleKind::Osc);
    set(&mut o2.values, OscParam::Sync, 1.0);
    set(&mut o2.values, OscParam::Semi, 12.0);
    p.generators.push(o2);
    let mut ev = events(&[note(52, 0, 40_000), note(59, 3_001, 40_000)]);
    ev.extend(param_events(
        &p,
        &[
            (3_000, "osc1.wave", 3.0),
            (6_007, "osc1.wave", 4.0),
            (9_100, "osc1.position", 30.0),
            (12_345, "osc1.position", 90.0),
            (15_000, "osc1.wave", 0.0),
            (18_000, "osc2.wave", 4.0),
            (21_000, "osc1.wave", 1.0),
            (24_000, "osc1.wave", 2.0),
            (27_000, "osc2.semi", -5.0),
            (30_000, "osc2.wave", 3.0),
            (30_001, "osc1.pw", 12.0),
        ],
    ));
    ev.sort_by_key(|(f, _)| *f);
    let reference = run_with(&p, &ev, 44_000, 512);
    assert!(reference.iter().all(|x| x.is_finite() && x.abs() < 4.0));
    assert!(reference[20_000..24_000].iter().any(|x| x.abs() > 0.1));
    for block in [1, 7, 31, 32, 33, 64, 4096] {
        assert_eq!(
            bits(&run_with(&p, &ev, 44_000, block)),
            bits(&reference),
            "block {block}"
        );
    }
}

#[test]
fn automating_wave_to_table_plays_the_prepared_table() {
    // The patch says `wave = saw` and `table = organ`; at 0.1 s `wave` becomes `table` and the organ sounds.
    let mut p = gated(Patch::init());
    set(
        &mut p.generators[0].values,
        OscParam::Table,
        Wavetable::Organ.index() as f32,
    );
    set(&mut p.generators[0].values, OscParam::Position, 100.0);
    let mut ev = events(&[note(45, 0, 30_000)]);
    ev.extend(param_events(&p, &[(4_800, "osc1.wave", 4.0)]));
    ev.sort_by_key(|(f, _)| *f);
    let x = run_with(&p, &ev, 24_000, 256);
    let mut direct = p.clone();
    set(
        &mut direct.generators[0].values,
        OscParam::Wave,
        wave(Wave::Table),
    );
    let y = run_with(&direct, &events(&[note(45, 0, 30_000)]), 24_000, 256);
    let rms = |s: &[f32]| {
        (s.iter().map(|v| f64::from(*v) * f64::from(*v)).sum::<f64>() / s.len() as f64).sqrt()
    };
    assert!(rms(&x[10_000..20_000]) > 0.05);
    // After the switch the phase is the saw's phase, so the waveforms agree up to a time shift: equal power.
    let (a, b) = (rms(&x[10_000..20_000]), rms(&y[10_000..20_000]));
    assert!((a / b - 1.0).abs() < 0.02, "{a} vs {b}");
}

#[test]
fn a_pink_noise_note_ends_without_subnormals_and_with_silence() {
    let mut p = noise_patch(NoiseColor::Pink, 100.0);
    p.modulators[0].values[AdsrParam::Release.index()] = 300.0;
    let mut e = enth_engine::Engine::new(compile(&p, &PrepareCfg::new(SR)).unwrap().0, SEED);
    let mut worst = 0;
    let (l, _) = common::drive(
        &mut e,
        &events(&[note(60, 0, 9_600)]),
        96_000,
        256,
        |e, _, _| {
            worst = worst.max(e.subnormals());
        },
    );
    assert_eq!(worst, 0);
    assert!(l[9_600..20_000].iter().any(|x| *x != 0.0));
    assert!(l[60_000..].iter().all(|x| *x == 0.0), "the voice ended");
}

// --- seed stability --------------------------------------------------------------------------------------------------

/// A patch exercising what the default one does not: a free-phase table oscillator as the sync master, a synced
/// pulse oscillator, and pink noise.
fn d1_patch() -> Patch {
    let mut p = Patch::init();
    p.modulators[0].values[AdsrParam::Release.index()] = 200.0;
    let v = &mut p.generators[0].values;
    set(v, OscParam::Wave, wave(Wave::Table));
    set(v, OscParam::Table, Wavetable::Formant.index() as f32);
    set(v, OscParam::Position, 55.0);
    let mut o2 = ModuleInstance::new(id("osc2"), ModuleKind::Osc);
    set(&mut o2.values, OscParam::Wave, wave(Wave::Pulse));
    set(&mut o2.values, OscParam::Pw, 30.0);
    set(&mut o2.values, OscParam::Sync, 1.0);
    set(&mut o2.values, OscParam::Semi, 7.0);
    set(&mut o2.values, OscParam::Level, 70.0);
    p.generators.push(o2);
    let mut o3 = ModuleInstance::new(id("osc3"), ModuleKind::Osc);
    set(&mut o3.values, OscParam::Wave, wave(Wave::Triangle));
    set(&mut o3.values, OscParam::Fine, 4.0);
    set(&mut o3.values, OscParam::Level, 50.0);
    p.generators.push(o3);
    let mut n = ModuleInstance::new(id("noise1"), ModuleKind::Noise);
    n.values[NoiseParam::Color.index()] = NoiseColor::Pink.index() as f32;
    n.values[NoiseParam::Level.index()] = 25.0;
    p.generators.push(n);
    p
}

fn chord_notes() -> Vec<common::Note> {
    [48u8, 55, 60, 64]
        .iter()
        .enumerate()
        .map(|(i, &k)| note(k, 624 * i as u64, 28_800))
        .collect()
}

/// Mean-square power per 10 ms of the left channel.
fn power10(x: &[f32]) -> Vec<f64> {
    x.chunks(480)
        .map(|c| c.iter().map(|&s| f64::from(s) * f64::from(s)).sum::<f64>() / c.len() as f64)
        .collect()
}

#[test]
fn moving_a_d1_parameter_by_1e_3_moves_the_rms_envelope_by_under_0_05_db() {
    const RATIO_0_05_DB: f64 = 1.011_579_454_259_899; // 10^(0.005)
    const RATIO_1_DB: f64 = 1.258_925_411_794_167_2; // 10^(0.1)
    let base = d1_patch();
    let notes = chord_notes();
    let reference = power10(&render(&base, &notes, 48_000, 512, SEED).0);
    let peak = reference.iter().fold(0.0f64, |m, x| m.max(*x));
    // The largest power ratio between two envelopes over the windows within 30 dB of the peak.
    let change = |other: &[f64]| {
        reference
            .iter()
            .zip(other)
            .filter(|(a, b)| a.max(**b) * 1e3 > peak && a.min(**b) > 0.0)
            .map(|(a, b)| a.max(*b) / a.min(*b))
            .fold(1.0, f64::max)
    };
    let mut nudged = 0;
    let mut worst = (1.0, String::new());
    for (pos, m) in base.modules() {
        for (i, ps) in m.kind.params().iter().enumerate() {
            let Kind::Float { min, max, .. } = ps.kind else {
                continue;
            };
            let value = m.values[i];
            if value == 0.0 || !matches!(m.kind, ModuleKind::Osc | ModuleKind::Noise) {
                continue;
            }
            let up = value * 1.001;
            let v = if (min..=max).contains(&up) {
                up
            } else {
                value * 0.999
            };
            let mut p = base.clone();
            p.section_mut(pos.section)[pos.index].values[i] = v;
            let ratio = change(&power10(&render(&p, &notes, 48_000, 512, SEED).0));
            nudged += 1;
            if ratio > worst.0 {
                worst = (ratio, format!("{}.{} {value} -> {v}", m.id, ps.key));
            }
        }
    }
    eprintln!(
        "{nudged} parameters nudged; worst power ratio {:.7} for {}",
        worst.0, worst.1
    );
    assert!(nudged >= 10, "only {nudged} parameters were nudged");
    assert!(
        worst.0 < RATIO_0_05_DB,
        "worst: {:.6} for {}",
        worst.0,
        worst.1
    );
    // Negative control: a different render seed moves free phases and noise streams by decibels.
    let reseeded = change(&power10(
        &render(&base, &notes, 48_000, 512, SEED ^ 0x5EED).0,
    ));
    eprintln!("negative control: a reseed moved the envelope by a power ratio of {reseeded:.3}");
    assert!(reseeded > RATIO_1_DB, "{reseeded}");
}
