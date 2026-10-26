//! The `comp` module through its public contract (lane FXW): it is the core with the catalogue's values, in the
//! engine's 32-frame segments; its step response through the module is the documented one; an idle compressor may
//! be skipped without changing a bit; NaN does not poison it; it never allocates.
//!
//! The allocation tests run under `assert_no_alloc` in debug builds (release builds lack `AllocDisabler`; the guard
//! is then a no-op).

mod common;

use assert_no_alloc::assert_no_alloc;
use common::{SEED, bits, engine, events, id, note, render};
use enth_dsp::compressor::{Compressor, CompressorParams};
use enth_dsp::util::rng::Rng;
use enth_engine::module::{
    Block, GlobalCx, GlobalParams, GlobalProcessor, GlobalStart, PrepareCfg, SEG,
};
use enth_engine::modules::comp::{Comp, core_params};
use enth_patch::catalogue::{AdsrParam, CompParam};
use enth_patch::{ModuleInstance, ModuleKind, Patch};

#[cfg(debug_assertions)]
#[global_allocator]
static ALLOCATOR: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;

const SR: f32 = 48_000.0;

/// A module and the parameter values it sees (`base = cur = prev`).
struct Rig {
    module: Comp,
    vals: Vec<f32>,
}

impl Rig {
    fn new(sample_rate: f32, vals: &[f32]) -> Self {
        let mut rig = Self {
            module: Comp::new(vals, &PrepareCfg::new(sample_rate)),
            vals: vals.to_vec(),
        };
        rig.reset();
        rig
    }

    fn reset(&mut self) {
        let params = GlobalParams::from_slices(&self.vals, &self.vals, &self.vals);
        self.module.reset(&GlobalStart {
            seed: 1,
            module_key: 2,
            params,
        });
    }

    fn set(&mut self, p: CompParam, v: f32) {
        self.vals[p.index()] = v;
    }

    fn run(&mut self, l: &mut Block, r: &mut Block) {
        let params = GlobalParams::from_slices(&self.vals, &self.vals, &self.vals);
        self.module.process(&mut GlobalCx {
            tempo_bpm: enth_engine::tempo::DEFAULT_TEMPO_BPM,
            params,
            left: l,
            right: r,
        });
    }

    /// The way the engine drives it: an idle module with silent input is skipped. Returns whether it ran.
    fn run_like_the_engine(&mut self, l: &mut Block, r: &mut Block) -> bool {
        let zero = |b: &Block| b.iter().all(|x| *x == 0.0);
        if self.module.is_silent() && zero(l) && zero(r) {
            return false;
        }
        self.run(l, r);
        true
    }
}

fn values(settings: &[(CompParam, f32)]) -> Vec<f32> {
    let mut v = ModuleKind::Comp.defaults();
    for &(p, x) in settings {
        v[p.index()] = x;
    }
    v
}

/// `n` segments of stereo noise in `[-amp, amp)`, then `silent` segments of zeros.
fn noise_then_silence(seed: u64, noisy: usize, silent: usize, amp: f32) -> Vec<(Block, Block)> {
    let mut rng = Rng::new(seed);
    (0..noisy + silent)
        .map(|i| {
            let mut seg = ([0.0; SEG], [0.0; SEG]);
            if i < noisy {
                for k in 0..SEG {
                    seg.0[k] = amp * rng.bipolar_f32();
                    seg.1[k] = amp * rng.bipolar_f32();
                }
            }
            seg
        })
        .collect()
}

// --- the module is the core -------------------------------------------------------------------------------------

#[test]
fn the_module_is_the_core_with_the_catalogue_values_in_32_frame_segments() {
    let mut vals = values(&[
        (CompParam::Threshold, -30.0),
        (CompParam::Ratio, 8.0),
        (CompParam::Attack, 3.0),
        (CompParam::Release, 120.0),
        (CompParam::Knee, 10.0),
        (CompParam::Makeup, 4.0),
        (CompParam::Mix, 80.0),
    ]);
    let mut rig = Rig::new(SR, &vals);
    let mut core = Compressor::new();
    core.prepare(SR);
    core.set_params(&core_params(|p| vals[p.index()]));
    let input = noise_then_silence(7, 600, 400, 0.6);
    for (i, (l, r)) in input.iter().enumerate() {
        // Moves in every field, one or two at a time, as the engine's smoothers and automation would make them.
        match i {
            100 => rig.set(CompParam::Makeup, 6.0),
            150 => rig.set(CompParam::Mix, 100.0),
            200 => {
                rig.set(CompParam::Threshold, -24.0);
                rig.set(CompParam::Ratio, 3.0);
            }
            300 => rig.set(CompParam::Attack, 20.0),
            400 => rig.set(CompParam::Release, 40.0),
            450 => rig.set(CompParam::Knee, 0.0),
            _ => {}
        }
        vals.clone_from(&rig.vals);
        core.set_params(&core_params(|p| vals[p.index()]));
        let (mut ml, mut mr) = (*l, *r);
        let (mut cl, mut cr) = (*l, *r);
        rig.run(&mut ml, &mut mr);
        core.process_in_place(&mut cl, &mut cr);
        assert_eq!(bits(&ml), bits(&cl), "left, segment {i}");
        assert_eq!(bits(&mr), bits(&cr), "right, segment {i}");
    }
}

#[test]
fn mix_zero_is_the_input_bit_for_bit_and_a_signal_below_the_threshold_passes_untouched() {
    // mix 0 %: the dry input, whatever the compressor does.
    let mut rig = Rig::new(
        SR,
        &values(&[(CompParam::Mix, 0.0), (CompParam::Makeup, 12.0)]),
    );
    for (l, r) in noise_then_silence(8, 200, 50, 0.9) {
        let (mut ml, mut mr) = (l, r);
        rig.run(&mut ml, &mut mr);
        assert_eq!(bits(&ml), bits(&l));
        assert_eq!(bits(&mr), bits(&r));
    }
    // A quiet signal under a high threshold, no make-up: transparent once the smoother has settled (immediately,
    // here, since it never leaves zero).
    let mut rig = Rig::new(
        SR,
        &values(&[(CompParam::Threshold, 0.0), (CompParam::Knee, 0.0)]),
    );
    for (l, r) in noise_then_silence(9, 200, 50, 0.1) {
        let (mut ml, mut mr) = (l, r);
        rig.run(&mut ml, &mut mr);
        assert_eq!(bits(&ml), bits(&l));
        assert_eq!(bits(&mr), bits(&r));
    }
}

// --- the step response through the module ---------------------------------------------------------------------

/// Gain reduction in dB of each frame of a DC step of `level` (both channels) of `on` frames, then silence for `off`
/// frames, measured on the module's output (`out / in` while the step is on; the follower's tail is read as the
/// same gain applied to the last input otherwise, so the release is measured with a small probe, see below).
struct Step {
    attack: Vec<f64>,
    release: Vec<f64>,
}

/// Run `on` frames of DC `level`, then `off` frames of DC `probe` (small and below the threshold, so the gain
/// reduction can be read from the output while the compressor lets go). Everything in 32-frame segments through the
/// module.
fn step_response(vals: &[f32], level: f32, on: usize, probe: f32, off: usize) -> Step {
    let mut rig = Rig::new(SR, vals);
    let mut run = |value: f32, frames: usize| -> Vec<f64> {
        let mut gr = Vec::with_capacity(frames);
        for _ in 0..frames / SEG {
            let (mut l, mut r) = ([value; SEG], [value; SEG]);
            rig.run(&mut l, &mut r);
            assert_eq!(bits(&l), bits(&r), "a mono signal stays mono");
            gr.extend(l.iter().map(|&y| -20.0 * libm::log10(f64::from(y / value))));
        }
        gr
    };
    let attack = run(level, on);
    let release = run(probe, off);
    Step { attack, release }
}

/// First index at which `gr` has gone `fraction` of the way from `from` to `to`.
fn first_crossing(gr: &[f64], from: f64, to: f64, fraction: f64) -> usize {
    let target = from + (to - from) * fraction;
    gr.iter()
        .position(|&g| if to > from { g >= target } else { g <= target })
        .expect("never crossed")
}

#[test]
fn step_response_through_the_module_is_the_documented_one() {
    // docs/dsp/compressor.md, "Attack and release": T -24 dB, 4:1, hard knee, a DC step of 0.5 (-6.02 dBFS,
    // 17.98 dB over), 48 kHz. Final reduction 13.48 dB; the gain reduction reaches 63.2 % of it after 1.253 attack
    // times (60 / 601 / 1804 samples for 1 / 10 / 30 ms); after the step it lets go, losing 63.2 % of its value
    // after 0.632 * d / 8.686 release times + attack / 2 (131.4 / 266.7 / 276.7 ms).
    for (attack, release, attack_samples, release_t63_ms) in [
        (1.0f32, 100.0f32, 60usize, 131.4f32),
        (10.0, 200.0, 601, 266.7),
        (30.0, 200.0, 1804, 276.7),
    ] {
        let vals = values(&[
            (CompParam::Threshold, -24.0),
            (CompParam::Ratio, 4.0),
            (CompParam::Knee, 0.0),
            (CompParam::Attack, attack),
            (CompParam::Release, release),
        ]);
        // The probe (-80 dBFS) is far below the threshold: it reads the gain without adding to the detector.
        let frames = |ms: f32| (ms * SR / 1000.0) as usize / SEG * SEG;
        let step = step_response(&vals, 0.5, frames(2_000.0), 1e-4, frames(3_000.0));
        let fin = *step.attack.last().unwrap();
        assert!((fin - 13.4846).abs() < 0.001, "final reduction {fin} dB");
        // `first_crossing` is a 0-based index; the doc counts samples.
        let n = first_crossing(&step.attack, 0.0, fin, 0.632) + 1;
        assert!(
            n.abs_diff(attack_samples) <= 1,
            "attack {attack} ms: {n} samples to 63 %, documented {attack_samples}"
        );
        // Release: the reduction (starting from `fin`) loses 63.2 % of its value. The probe sits well below the
        // detector, so the follower decays as in the doc's measurement (a drop to silence).
        let m = first_crossing(&step.release, fin, 0.0, 0.632) as f32;
        let ms = m * 1000.0 / SR;
        assert!(
            (ms - release_t63_ms).abs() <= 0.01 * release_t63_ms,
            "release {release} ms: t63 {ms} ms against the documented {release_t63_ms} ms"
        );
        // And it ends at no reduction.
        assert!(step.release.last().unwrap().abs() < 1e-3);
    }
}

#[test]
fn the_step_response_through_the_module_equals_the_cores() {
    let vals = values(&[(CompParam::Threshold, -24.0), (CompParam::Knee, 0.0)]);
    let step = step_response(&vals, 0.5, 4_800, 1e-4, 9_600);
    let mut core = Compressor::new();
    core.prepare(SR);
    core.set_params(&core_params(|p| vals[p.index()]));
    let mut expect = Vec::new();
    for n in 0..4_800 + 9_600 {
        let x = if n < 4_800 { 0.5f32 } else { 1e-4 };
        let (y, _) = core.tick(x, x);
        expect.push(-20.0 * libm::log10(f64::from(y / x)));
    }
    let got: Vec<f64> = step.attack.iter().chain(&step.release).copied().collect();
    assert_eq!(got.len(), expect.len());
    for (n, (a, b)) in got.iter().zip(&expect).enumerate() {
        assert_eq!(a.to_bits(), b.to_bits(), "frame {n}");
    }
}

// --- idle, NaN, state carry ------------------------------------------------------------------------------------

#[test]
fn an_idle_compressor_skipped_by_the_engine_gives_the_same_bits_as_one_that_is_always_called() {
    // Release 10 ms and attack 0.1 ms: both states flush to zero within ~0.5 s of silence. Settings change while
    // idle (the skipped module never sees them until it wakes).
    let vals = values(&[(CompParam::Release, 10.0), (CompParam::Attack, 0.1)]);
    let mut always = Rig::new(SR, &vals);
    let mut skipping = Rig::new(SR, &vals);
    let burst = 300;
    let gap = 2 * 48_000 / SEG;
    let mut input = noise_then_silence(1, burst, gap, 0.8);
    input.extend(noise_then_silence(2, burst, gap, 0.8));
    let (mut skipped, mut woke, mut was_idle) = (0usize, 0usize, false);
    for (i, (l, r)) in input.iter().enumerate() {
        if i == burst + gap / 2 {
            for rig in [&mut always, &mut skipping] {
                rig.set(CompParam::Threshold, -40.0);
                rig.set(CompParam::Ratio, 12.0);
                rig.set(CompParam::Makeup, 6.0);
                rig.set(CompParam::Attack, 5.0);
                rig.set(CompParam::Mix, 70.0);
            }
        }
        let (mut al, mut ar) = (*l, *r);
        let (mut sl, mut sr) = (*l, *r);
        always.run(&mut al, &mut ar);
        let ran = skipping.run_like_the_engine(&mut sl, &mut sr);
        skipped += usize::from(!ran);
        assert_eq!(bits(&al), bits(&sl), "left, segment {i}");
        assert_eq!(bits(&ar), bits(&sr), "right, segment {i}");
        let idle = skipping.module.is_silent();
        assert_eq!(always.module.is_silent(), idle, "segment {i}");
        woke += usize::from(was_idle && !idle);
        was_idle = idle;
        if idle {
            // The proof of `is_silent`: the follower and the smoother are exactly zero.
            assert_eq!(skipping.module.core().gain_reduction_db(), 0.0);
        }
    }
    assert!(skipped > 1_000, "only {skipped} segments were skipped");
    assert_eq!(woke, 1);
}

#[test]
fn the_tail_ends_with_no_subnormals_and_the_compressor_goes_idle() {
    // A loud burst, then 12 s of silence with the slowest catalogue release: nothing subnormal, ever, in the output
    // or the state, and the state ends in exact zeros.
    for (attack, release) in [(0.1f32, 10.0f32), (10.0, 200.0), (100.0, 2_000.0)] {
        let vals = values(&[
            (CompParam::Threshold, -60.0),
            (CompParam::Ratio, 20.0),
            (CompParam::Attack, attack),
            (CompParam::Release, release),
        ]);
        let mut rig = Rig::new(SR, &vals);
        let mut idle_at = None;
        // The follower falls 8.7 dB per release time and is flushed at -300 dBFS: about 35 release times.
        let silence = (40.0 * release / 1000.0 + 2.0) as usize * 48_000 / SEG;
        for (i, (l, r)) in noise_then_silence(3, 200, silence, 1.0)
            .into_iter()
            .enumerate()
        {
            let (mut l, mut r) = (l, r);
            rig.run_like_the_engine(&mut l, &mut r);
            assert_eq!(
                enth_engine::module::count_subnormals(&l)
                    + enth_engine::module::count_subnormals(&r),
                0
            );
            assert_eq!(rig.module.subnormals(), 0, "segment {i}");
            if idle_at.is_none() && rig.module.is_silent() {
                idle_at = Some(i);
            }
        }
        assert!(
            idle_at.is_some(),
            "attack {attack}, release {release}: never idle"
        );
    }
}

#[test]
fn nan_input_does_not_poison_the_state() {
    let vals = ModuleKind::Comp.defaults();
    let mut dirty = Rig::new(SR, &vals);
    let mut clean = Rig::new(SR, &vals);
    for (i, (l, r)) in noise_then_silence(11, 400, 100, 0.8)
        .into_iter()
        .enumerate()
    {
        let (mut dl, mut dr) = (l, r);
        let (mut cl, mut cr) = (l, r);
        if i % 9 == 4 {
            // NaN on one channel, then on both, at a few frames; the clean twin sees 0 (what the detector makes of it).
            dl[3] = f32::NAN;
            cl[3] = 0.0;
            dl[20] = f32::NAN;
            dr[20] = f32::NAN;
            cl[20] = 0.0;
            cr[20] = 0.0;
        }
        dirty.run(&mut dl, &mut dr);
        clean.run(&mut cl, &mut cr);
        // Every frame without NaN in it is bit-identical; the NaN frames are the caller's problem, not the state's.
        for k in 0..SEG {
            if !(dl[k].is_nan() || dr[k].is_nan()) {
                assert_eq!(dl[k].to_bits(), cl[k].to_bits(), "segment {i} frame {k}");
                assert_eq!(dr[k].to_bits(), cr[k].to_bits(), "segment {i} frame {k}");
            }
        }
    }
    assert!(dirty.module.core().gain_reduction_db().is_finite());
    // An infinite sample is clamped in the detector: the state stays finite and the next frames are finite.
    let (mut l, mut r) = ([0.3; SEG], [0.3; SEG]);
    l[5] = f32::INFINITY;
    dirty.run(&mut l, &mut r);
    let (mut l, mut r) = ([0.3; SEG], [0.3; SEG]);
    dirty.run(&mut l, &mut r);
    assert!(l.iter().chain(&r).all(|x| x.is_finite()));
}

#[test]
fn copy_state_makes_two_modules_identical_from_then_on() {
    let vals = ModuleKind::Comp.defaults();
    let mut a = Rig::new(SR, &vals);
    let mut b = Rig::new(SR, &vals);
    for (l, r) in noise_then_silence(21, 200, 0, 0.9) {
        let (mut l, mut r) = (l, r);
        a.run(&mut l, &mut r);
    }
    for (l, r) in noise_then_silence(22, 50, 0, 0.1) {
        let (mut l, mut r) = (l, r);
        b.run(&mut l, &mut r);
    }
    b.module.copy_state_from(&a.module);
    for rig in [&mut a, &mut b] {
        rig.set(CompParam::Ratio, 10.0);
    }
    for (l, r) in noise_then_silence(23, 100, 100, 0.7) {
        let (mut al, mut ar) = (l, r);
        let (mut bl, mut br) = (l, r);
        a.run(&mut al, &mut ar);
        b.run(&mut bl, &mut br);
        assert_eq!(bits(&al), bits(&bl));
        assert_eq!(bits(&ar), bits(&br));
    }
    assert_eq!(
        a.module.core().gain_reduction_db().to_bits(),
        b.module.core().gain_reduction_db().to_bits()
    );
}

#[test]
fn reset_clears_the_reduction_and_reads_the_patch_values() {
    let vals = values(&[(CompParam::Threshold, -40.0)]);
    let mut rig = Rig::new(SR, &vals);
    for (l, r) in noise_then_silence(31, 200, 0, 0.9) {
        let (mut l, mut r) = (l, r);
        rig.run(&mut l, &mut r);
    }
    assert!(rig.module.core().gain_reduction_db() > 3.0);
    rig.reset();
    assert!(rig.module.is_silent());
    assert_eq!(rig.module.core().gain_reduction_db(), 0.0);
    assert_eq!(rig.module.core().params().threshold_db, -40.0);
}

// --- real time -------------------------------------------------------------------------------------------------

#[test]
fn nothing_after_new_allocates() {
    let vals = ModuleKind::Comp.defaults();
    let mut a = Rig::new(SR, &vals);
    let mut b = Rig::new(SR, &vals);
    let mut rng = Rng::new(5);
    let junk = [f32::NAN, f32::INFINITY, -f32::INFINITY];
    let mut l: Block = [0.0; SEG];
    let mut r: Block = [0.0; SEG];
    for i in 0..4_000 {
        for k in 0..SEG {
            l[k] = if i < 1_000 {
                0.8 * rng.bipolar_f32()
            } else {
                0.0
            };
            r[k] = l[k];
        }
        if i % 5 == 1 {
            l[i % SEG] = junk[i % 3];
        }
        for (n, &p) in CompParam::ALL.iter().enumerate() {
            if (i + n) % 7 == 0 {
                let spec = p.spec();
                let t = rng.unit_f32();
                a.vals[p.index()] = match (i + n) % 5 {
                    0 => f32::NAN,
                    1 => spec.max() * 4.0,
                    2 => spec.min() - 100.0,
                    _ => spec.min() + t * (spec.max() - spec.min()),
                };
            }
        }
        assert_no_alloc(|| a.run_like_the_engine(&mut l, &mut r));
        if i == 1_500 {
            assert_no_alloc(|| a.reset());
        }
        if i == 2_000 {
            assert_no_alloc(|| b.module.copy_state_from(&a.module));
        }
    }
}

// --- through the engine ----------------------------------------------------------------------------------------

/// A saw through a gate envelope into a compressor with the given settings.
fn comp_patch(settings: &[(CompParam, f32)]) -> Patch {
    let mut p = Patch::init();
    p.modulators[0].values[AdsrParam::Attack.index()] = 0.0;
    p.modulators[0].values[AdsrParam::Decay.index()] = 0.0;
    p.modulators[0].values[AdsrParam::Sustain.index()] = 100.0;
    p.modulators[0].values[AdsrParam::Release.index()] = 0.0;
    let mut m = ModuleInstance::new(id("comp1"), ModuleKind::Comp);
    for &(param, x) in settings {
        m.values[param.index()] = x;
    }
    p.fx.push(m);
    p
}

fn peak(x: &[f32]) -> f32 {
    x.iter().fold(0.0f32, |m, v| m.max(v.abs()))
}

#[test]
fn the_engine_compresses_and_the_host_block_size_never_matters() {
    let notes = [note(48, 0, 24_000), note(60, 12_000, 36_000)];
    let frames = 48_000;
    let strong = comp_patch(&[
        (CompParam::Threshold, -30.0),
        (CompParam::Ratio, 10.0),
        (CompParam::Knee, 0.0),
    ]);
    let none = {
        let mut p = strong.clone();
        p.fx.clear();
        p
    };
    let (cl, cr, e) = render(&strong, &notes, frames, 512, SEED);
    let (ul, _, _) = render(&none, &notes, frames, 512, SEED);
    assert!(
        peak(&cl[24_000..30_000]) < 0.5 * peak(&ul[24_000..30_000]),
        "10:1 above -30 dB turns a loud saw down by well over 6 dB"
    );
    assert_eq!(e.subnormals(), 0);
    for block in [1, 7, 32, 33, 64, 480, 4096] {
        let (l, r, _) = render(&strong, &notes, frames, block, SEED);
        assert_eq!(bits(&l), bits(&cl), "left, block {block}");
        assert_eq!(bits(&r), bits(&cr), "right, block {block}");
    }
}

#[test]
fn a_transparent_compressor_leaves_the_engine_output_bit_identical() {
    // Ratio 1 never reduces, and with no make-up and mix 100 % the gain is exactly 1.0: the compressor is the
    // identity. (Mix 0 % is the same by another road.)
    let notes = [note(48, 0, 24_000), note(60, 12_000, 36_000)];
    let none = {
        let mut p = comp_patch(&[]);
        p.fx.clear();
        p
    };
    let (rl, rr, _) = render(&none, &notes, 48_000, 128, SEED);
    for settings in [
        vec![(CompParam::Ratio, 1.0)],
        vec![(CompParam::Mix, 0.0), (CompParam::Makeup, 9.0)],
    ] {
        let (l, r, _) = render(&comp_patch(&settings), &notes, 48_000, 128, SEED);
        assert_eq!(bits(&l), bits(&rl), "{settings:?}");
        assert_eq!(bits(&r), bits(&rr), "{settings:?}");
    }
}

#[test]
fn the_engine_with_a_compressor_does_not_allocate() {
    let patch = comp_patch(&[(CompParam::Threshold, -40.0), (CompParam::Ratio, 8.0)]);
    let mut e = engine(&patch, SEED);
    let ev = events(&[note(60, 0, 20_000), note(64, 30_000, 33_000)]);
    let (mut l, mut r) = (vec![0.0f32; 480], vec![0.0f32; 480]);
    let mut chunk = Vec::new();
    for block in 0..200 {
        let start = (block * 480) as u64;
        chunk.clear();
        for &(f, kind) in &ev {
            if f >= start && f < start + 480 {
                chunk.push(enth_engine::Event {
                    offset: (f - start) as u32,
                    kind,
                });
            }
        }
        assert_no_alloc(|| e.process(&mut l, &mut r, &chunk));
    }
}

#[test]
fn the_default_settings_are_the_cores_defaults() {
    let v = ModuleKind::Comp.defaults();
    assert_eq!(core_params(|p| v[p.index()]), CompressorParams::DEFAULT);
}
