//! The `reverb` module through its public contract (lane FXW): it is the core with the catalogue's conversions, in
//! the engine's 32-frame segments; it survives NaN; its tail ends in exact zeros with no subnormal anywhere; an
//! idle reverb may be skipped without changing a bit; it never allocates after `new`.
//!
//! The allocation tests run under `assert_no_alloc` in debug builds (release builds lack `AllocDisabler`; the guard
//! is then a no-op). Under nextest an abort fails exactly one test.

mod common;

use assert_no_alloc::assert_no_alloc;
use common::{SEED, bits, engine, events, id, note, render};
use enth_dsp::reverb::{Reverb as Core, ReverbParams};
use enth_dsp::util::rng::Rng;
use enth_engine::module::{
    Block, GlobalCx, GlobalParams, GlobalProcessor, GlobalStart, PrepareCfg, SEG, count_subnormals,
};
use enth_engine::modules::reverb::{Reverb, core_params};
use enth_patch::catalogue::{AdsrParam, ReverbParam};
use enth_patch::{ModuleInstance, ModuleKind, Patch};

#[cfg(debug_assertions)]
#[global_allocator]
static ALLOCATOR: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;

const SR: f32 = 48_000.0;

/// A module and the parameter values it sees (`base = cur = prev`), driven like the engine drives it.
struct Rig {
    module: Reverb,
    vals: Vec<f32>,
}

impl Rig {
    fn new(sample_rate: f32, vals: &[f32]) -> Self {
        let mut rig = Self {
            module: Reverb::new(vals, &PrepareCfg::new(sample_rate)),
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

    fn set(&mut self, p: ReverbParam, v: f32) {
        self.vals[p.index()] = v;
    }

    /// One segment, always called.
    fn run(&mut self, l: &mut Block, r: &mut Block) {
        let params = GlobalParams::from_slices(&self.vals, &self.vals, &self.vals);
        self.module.process(&mut GlobalCx {
            tempo_bpm: enth_engine::tempo::DEFAULT_TEMPO_BPM,
            params,
            left: l,
            right: r,
        });
    }

    /// One segment the way the engine does it: an idle module with silent input is skipped. Returns whether it ran.
    fn run_like_the_engine(&mut self, l: &mut Block, r: &mut Block) -> bool {
        let zero = |b: &Block| b.iter().all(|x| *x == 0.0);
        if self.module.is_silent() && zero(l) && zero(r) {
            return false;
        }
        self.run(l, r);
        true
    }
}

fn values(settings: &[(ReverbParam, f32)]) -> Vec<f32> {
    let mut v = ModuleKind::Reverb.defaults();
    for &(p, x) in settings {
        v[p.index()] = x;
    }
    v
}

/// `n` segments of stereo noise in `[-amp, amp)`, then `silent` segments of zeros.
fn noise_then_silence(seed: u64, noisy: usize, silent: usize, amp: f32) -> Vec<(Block, Block)> {
    let mut rng = Rng::new(seed);
    let mut out = Vec::with_capacity(noisy + silent);
    for i in 0..noisy + silent {
        let mut seg = ([0.0; SEG], [0.0; SEG]);
        if i < noisy {
            for k in 0..SEG {
                seg.0[k] = amp * rng.bipolar_f32();
                seg.1[k] = amp * rng.bipolar_f32();
            }
        }
        out.push(seg);
    }
    out
}

fn is_zero(b: &Block) -> bool {
    b.iter().all(|x| *x == 0.0)
}

// --- the module is the core with the catalogue's conversions -------------------------------------------------

#[test]
fn the_module_is_the_core_with_the_catalogue_conversions_in_32_frame_segments() {
    let mut vals = values(&[
        (ReverbParam::Dry, -6.0),
        (ReverbParam::Wet, 0.0),
        (ReverbParam::Room, 120.0),
        (ReverbParam::Decay, 2_400.0),
        (ReverbParam::Early, 100.0),
        (ReverbParam::Detune, 8.0),
        (ReverbParam::LowCut, 150.0),
        (ReverbParam::HighCut, 8_000.0),
        (ReverbParam::LowDamp, 3.0),
        (ReverbParam::HighDamp, 4.0),
    ]);
    let mut rig = Rig::new(SR, &vals);
    let mut core = Core::prepare(SR);
    core.set_params(&core_params(|p| vals[p.index()]));

    let input = noise_then_silence(5, 24_000 / SEG, 48_000 / SEG, 0.4);
    for (i, (l, r)) in input.iter().enumerate() {
        // Parameters move twice: a core-faded set (room, wet) and a core-unfaded set (the high cut).
        if i == 400 {
            rig.set(ReverbParam::Room, 60.0);
            rig.set(ReverbParam::Wet, -12.0);
            vals = rig.vals.clone();
            core.set_params(&core_params(|p| vals[p.index()]));
        }
        if i == 900 {
            rig.set(ReverbParam::HighCut, 3_000.0);
            rig.set(ReverbParam::Decay, 600.0);
            vals = rig.vals.clone();
            core.set_params(&core_params(|p| vals[p.index()]));
        }
        let (mut ml, mut mr) = (*l, *r);
        let (mut cl, mut cr) = (*l, *r);
        rig.run(&mut ml, &mut mr);
        core.process_in_place(&mut cl, &mut cr);
        assert_eq!(bits(&ml), bits(&cl), "left, segment {i}");
        assert_eq!(bits(&mr), bits(&cr), "right, segment {i}");
    }
}

#[test]
fn the_default_module_reproduces_the_cores_default_up_to_the_wet_level() {
    // dry 0 dB = 100 % exactly; wet -6 dB = 50.12 % against the core's 50 %: same network, a gain 0.0024 dB apart.
    let vals = ModuleKind::Reverb.defaults();
    let c = core_params(|p| vals[p.index()]);
    assert_eq!(c.dry_pct, ReverbParams::default().dry_pct);
    assert!((c.wet_pct / ReverbParams::default().wet_pct - 1.0).abs() < 3e-3);
}

#[test]
fn the_reverb_is_time_based_and_the_compressor_is_not() {
    // The flag S2b's global FX bypass reads.
    assert!(ModuleKind::Reverb.spec().time_based);
    assert!(!ModuleKind::Comp.spec().time_based);
}

#[test]
fn dry_off_and_wet_off_are_exact() {
    // wet -60 dB (off): the module is the identity bit for bit, tail or not.
    let vals = values(&[(ReverbParam::Wet, -60.0)]);
    let mut rig = Rig::new(SR, &vals);
    for (l, r) in noise_then_silence(8, 100, 100, 0.7) {
        let (mut ml, mut mr) = (l, r);
        rig.run(&mut ml, &mut mr);
        assert_eq!(bits(&ml), bits(&l));
        assert_eq!(bits(&mr), bits(&r));
    }
    // dry -60 dB (off): the output is the tail alone, so it starts silent and is nonzero only after the input.
    let vals = values(&[(ReverbParam::Dry, -60.0), (ReverbParam::Wet, 0.0)]);
    let mut rig = Rig::new(SR, &vals);
    let mut core = Core::prepare(SR);
    core.set_params(&core_params(|p| vals[p.index()]));
    assert_eq!(core_params(|p| vals[p.index()]).dry_pct, 0.0);
    for (l, r) in noise_then_silence(9, 50, 50, 0.7) {
        let (mut ml, mut mr) = (l, r);
        let (mut cl, mut cr) = (l, r);
        rig.run(&mut ml, &mut mr);
        core.process_in_place(&mut cl, &mut cr);
        assert_eq!(bits(&ml), bits(&cl));
        assert_eq!(bits(&mr), bits(&cr));
    }
}

// --- idle: skipping is the same as running ---------------------------------------------------------------------

#[test]
fn an_idle_reverb_skipped_by_the_engine_gives_the_same_bits_as_one_that_is_always_called() {
    // A short decay so the tail dies exactly within a couple of seconds; changes made while idle; two bursts.
    let vals = values(&[(ReverbParam::Decay, 150.0), (ReverbParam::Detune, 20.0)]);
    let mut always = Rig::new(SR, &vals);
    let mut skipping = Rig::new(SR, &vals);
    let (mut skipped, mut idle_seen, mut woke) = (0usize, 0usize, 0usize);
    // 0.25 s of noise, 4 s of silence with a parameter change in the middle of it, 0.25 s of noise, 4 s of silence.
    let burst = 375;
    let gap = 4 * 48_000 / SEG;
    let mut input = noise_then_silence(1, burst, gap, 0.5);
    input.extend(noise_then_silence(3, burst, gap, 0.5));
    let second_burst = burst + gap;
    let mut was_idle = false;
    for (i, (l, r)) in input.iter().enumerate() {
        if i == burst + gap / 2 {
            // While idle: change faded and unfaded parameters, and the detune that drives the LFO.
            for rig in [&mut always, &mut skipping] {
                rig.set(ReverbParam::Room, 33.0);
                rig.set(ReverbParam::Wet, -3.0);
                rig.set(ReverbParam::LowCut, 300.0);
                rig.set(ReverbParam::Detune, 40.0);
                rig.set(ReverbParam::Decay, 400.0);
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
        if idle && i % 16 == 0 {
            // `is_silent` is a proof: the network really holds only zeros.
            assert!(skipping.module.core().is_state_zero(), "segment {i}");
        }
        idle_seen += usize::from(idle);
        woke += usize::from(was_idle && !idle);
        was_idle = idle;
    }
    // The test is only worth something if the module did go idle, got skipped, and woke up again.
    assert!(skipped > 1_000, "only {skipped} segments were skipped");
    assert!(idle_seen > 1_000);
    assert_eq!(woke, 1, "it should wake exactly once, for the second burst");
    assert!(second_burst < input.len());
}

#[test]
fn waking_from_idle_is_a_fresh_start_with_the_current_parameters() {
    // After a long silence the network holds zeros, so a note that arrives sounds exactly like the same note into a
    // brand-new core with the parameters as they are now: the pending changes apply at once (no 20 ms fade from
    // the old settings) and the detune LFO starts at phase 0.
    let old = values(&[(ReverbParam::Decay, 150.0), (ReverbParam::Detune, 20.0)]);
    let mut rig = Rig::new(SR, &old);
    for (l, r) in noise_then_silence(4, 100, 3 * 48_000 / SEG, 0.5) {
        let (mut l, mut r) = (l, r);
        rig.run_like_the_engine(&mut l, &mut r);
    }
    assert!(rig.module.is_silent(), "idle after the tail");
    // Change everything the core fades, and the filters it does not.
    rig.set(ReverbParam::Room, 33.0);
    rig.set(ReverbParam::Wet, -3.0);
    rig.set(ReverbParam::Dry, -12.0);
    rig.set(ReverbParam::LowCut, 300.0);
    rig.set(ReverbParam::HighCut, 4_000.0);
    rig.set(ReverbParam::Detune, 40.0);
    rig.set(ReverbParam::Decay, 400.0);
    let mut fresh = Core::prepare(SR);
    fresh.set_params(&core_params(|p| rig.vals[p.index()]));
    for (i, (l, r)) in noise_then_silence(5, 200, 200, 0.5).into_iter().enumerate() {
        let (mut ml, mut mr) = (l, r);
        let (mut fl, mut fr) = (l, r);
        rig.run_like_the_engine(&mut ml, &mut mr);
        fresh.process_in_place(&mut fl, &mut fr);
        assert_eq!(bits(&ml), bits(&fl), "left, segment {i}");
        assert_eq!(bits(&mr), bits(&fr), "right, segment {i}");
    }
}

#[test]
fn the_tail_ends_in_exact_zeros_with_no_subnormals_and_the_module_goes_idle() {
    // 1 s of noise and 60 s of silence in engine segments: the core's own 61 s test, through the module.
    for (sr, secs, decay) in [(48_000.0f32, 60usize, 3_000.0f32), (96_000.0, 12, 300.0)] {
        let vals = values(&[(ReverbParam::Decay, decay)]);
        let mut rig = Rig::new(sr, &vals);
        let per_second = sr as usize / SEG;
        let mut last_nonzero = 0usize;
        let mut idle_at = None;
        let mut subnormal_outputs = 0usize;
        let mut worst_state = 0usize;
        let mut seg_no = 0usize;
        for (l, r) in noise_then_silence(21, per_second, secs * per_second, 0.5) {
            let (mut l, mut r) = (l, r);
            rig.run_like_the_engine(&mut l, &mut r);
            subnormal_outputs += count_subnormals(&l) + count_subnormals(&r);
            if !(is_zero(&l) && is_zero(&r)) {
                last_nonzero = seg_no;
            }
            if seg_no.is_multiple_of(per_second) {
                worst_state = worst_state.max(rig.module.subnormals());
            }
            if idle_at.is_none() && rig.module.is_silent() {
                idle_at = Some(seg_no);
            }
            seg_no += 1;
        }
        worst_state = worst_state.max(rig.module.subnormals());
        assert_eq!(subnormal_outputs, 0, "{sr} Hz: subnormal output samples");
        assert_eq!(worst_state, 0, "{sr} Hz: subnormal values in the network");
        let idle_at = idle_at.expect("the module goes idle once the tail is gone");
        assert!(rig.module.is_silent());
        assert!(
            idle_at > last_nonzero,
            "idle ({idle_at}) only after the last nonzero output ({last_nonzero})"
        );
        // The output really is exactly zero from the last nonzero segment on, and that is far before the end.
        assert!(last_nonzero + per_second < seg_no, "{sr} Hz: tail too long");
        // RT60 = decay: the tail reaches -300 dB (the flush) after about 15 RT20, and a bit more for the filters.
        let last_s = last_nonzero as f32 / per_second as f32;
        let rt20 = decay / 3_000.0;
        assert!(
            last_s > 1.0 + 8.0 * rt20 && last_s < 1.0 + 30.0 * rt20,
            "{sr} Hz: the tail ended at {last_s} s (RT20 {rt20} s)"
        );
    }
}

#[test]
fn reset_gives_silence_and_the_same_bits_as_a_fresh_module() {
    let vals = values(&[(ReverbParam::Room, 150.0), (ReverbParam::Detune, 15.0)]);
    let input = noise_then_silence(33, 200, 200, 0.5);
    let render = |rig: &mut Rig| -> Vec<u32> {
        let mut out = Vec::new();
        for (l, r) in &input {
            let (mut l, mut r) = (*l, *r);
            rig.run(&mut l, &mut r);
            out.extend(bits(&l));
            out.extend(bits(&r));
        }
        out
    };
    let mut fresh = Rig::new(SR, &vals);
    let reference = render(&mut fresh);
    let mut used = Rig::new(SR, &vals);
    render(&mut used);
    assert!(!used.module.is_silent(), "a tail is still running");
    used.reset();
    assert!(used.module.is_silent(), "reset leaves a silent network");
    assert_eq!(render(&mut used), reference);
    // And a reset module fed silence outputs silence at once.
    used.reset();
    let (mut l, mut r) = ([0.0; SEG], [0.0; SEG]);
    used.run(&mut l, &mut r);
    assert!(is_zero(&l) && is_zero(&r));
}

// --- NaN and infinity ------------------------------------------------------------------------------------------

#[test]
fn non_finite_input_never_reaches_the_network() {
    let vals = values(&[(ReverbParam::Detune, 10.0)]);
    let mut dirty = Rig::new(SR, &vals);
    let mut clean = Rig::new(SR, &vals);
    let mut rng = Rng::new(77);
    let junk = [
        f32::NAN,
        f32::INFINITY,
        f32::NEG_INFINITY,
        3.0e38,
        -3.0e38,
        -f32::NAN,
    ];
    for i in 0..600 {
        let (mut dl, mut dr) = ([0.0; SEG], [0.0; SEG]);
        for k in 0..SEG {
            dl[k] = 0.3 * rng.bipolar_f32();
            dr[k] = 0.3 * rng.bipolar_f32();
        }
        let (mut cl, mut cr) = (dl, dr);
        if i % 7 == 3 {
            // Poison a few samples of the dirty block; the clean twin gets what the wrapper must make of them.
            for (n, &x) in junk.iter().enumerate() {
                let at = (n * 5 + i) % SEG;
                dl[at] = x;
                cl[at] = if x.is_nan() {
                    0.0
                } else {
                    x.clamp(-1.0e6, 1.0e6)
                };
                dr[SEG - 1 - at] = x;
                cr[SEG - 1 - at] = cl[at];
            }
        }
        dirty.run(&mut dl, &mut dr);
        clean.run(&mut cl, &mut cr);
        assert!(
            dl.iter().chain(&dr).all(|x| x.is_finite()),
            "segment {i}: non-finite output"
        );
        assert_eq!(bits(&dl), bits(&cl), "segment {i}");
        assert_eq!(bits(&dr), bits(&cr), "segment {i}");
    }
    // The tail afterwards is clean too.
    for _ in 0..2_000 {
        let (mut l, mut r) = ([0.0; SEG], [0.0; SEG]);
        dirty.run(&mut l, &mut r);
        assert!(l.iter().chain(&r).all(|x| x.is_finite()));
    }
}

// --- state carry (the plan swap) -------------------------------------------------------------------------------

#[test]
fn copy_state_makes_two_modules_identical_from_then_on() {
    let vals = values(&[(ReverbParam::Detune, 12.0)]);
    let mut a = Rig::new(SR, &vals);
    let mut b = Rig::new(SR, &vals);
    let input = noise_then_silence(41, 300, 0, 0.4);
    for (l, r) in &input {
        let (mut l, mut r) = (*l, *r);
        a.run(&mut l, &mut r);
    }
    // `b` has run something else, so every part of its state differs.
    for (l, r) in noise_then_silence(42, 50, 0, 0.9) {
        let (mut l, mut r) = (l, r);
        b.run(&mut l, &mut r);
    }
    b.module.copy_state_from(&a.module);
    // A parameter change right after the copy lands in a fade in both.
    for rig in [&mut a, &mut b] {
        rig.set(ReverbParam::Room, 45.0);
        rig.set(ReverbParam::Wet, -2.0);
    }
    for (l, r) in noise_then_silence(43, 100, 500, 0.4) {
        let (mut al, mut ar) = (l, r);
        let (mut bl, mut br) = (l, r);
        a.run(&mut al, &mut ar);
        b.run(&mut bl, &mut br);
        assert_eq!(bits(&al), bits(&bl));
        assert_eq!(bits(&ar), bits(&br));
    }
}

#[test]
fn copying_between_different_sample_rates_copies_nothing() {
    let vals = ModuleKind::Reverb.defaults();
    let mut a = Rig::new(48_000.0, &vals);
    let mut b = Rig::new(44_100.0, &vals);
    for (l, r) in noise_then_silence(51, 10, 0, 0.4) {
        let (mut l, mut r) = (l, r);
        a.run(&mut l, &mut r);
    }
    assert!(!a.module.is_silent());
    assert!(b.module.is_silent());
    b.module.copy_state_from(&a.module);
    assert!(b.module.is_silent(), "nothing was copied");
}

// --- real time -------------------------------------------------------------------------------------------------

#[test]
fn nothing_after_new_allocates() {
    let vals = values(&[(ReverbParam::Detune, 10.0)]);
    let mut a = Rig::new(SR, &vals);
    let mut b = Rig::new(SR, &vals);
    let mut rng = Rng::new(3);
    let junk = [f32::NAN, f32::INFINITY, -f32::INFINITY];
    let mut l: Block = [0.0; SEG];
    let mut r: Block = [0.0; SEG];
    // A busy stretch with every parameter moving (including NaN and out-of-range values), junk in the input, a
    // reset, a state copy, then silence until the module goes idle and wakes again.
    for i in 0..3_000 {
        for k in 0..SEG {
            l[k] = if i < 600 {
                0.3 * rng.bipolar_f32()
            } else {
                0.0
            };
            r[k] = l[k];
        }
        if i % 5 == 1 {
            l[i % SEG] = junk[i % 3];
        }
        for (n, &p) in ReverbParam::ALL.iter().enumerate() {
            if (i + n) % 13 == 0 {
                let spec = p.spec();
                let t = rng.unit_f32();
                a.vals[p.index()] = match (i + n) % 5 {
                    0 => f32::NAN,
                    1 => spec.max() * 3.0,
                    _ => spec.min() + t * (spec.max() - spec.min()),
                };
            }
        }
        assert_no_alloc(|| a.run(&mut l, &mut r));
        if i == 1_000 {
            assert_no_alloc(|| a.reset());
        }
        if i == 1_500 {
            assert_no_alloc(|| b.module.copy_state_from(&a.module));
        }
    }
    // Quiet for a long time with sane parameters: the scan, the idle skip and the wake all run guarded.
    a.vals = values(&[(ReverbParam::Decay, 100.0)]);
    for i in 0..(10 * 48_000 / SEG) {
        let burst = i % 6_000 == 0;
        for k in 0..SEG {
            l[k] = if burst { 0.5 * rng.bipolar_f32() } else { 0.0 };
            r[k] = l[k];
        }
        assert_no_alloc(|| a.run_like_the_engine(&mut l, &mut r));
    }
    assert!(a.module.is_silent());
}

/// Negative control: an allocation inside the guard aborts the process, so run it in a child test process.
#[test]
#[cfg(all(debug_assertions, not(target_family = "wasm")))]
#[allow(
    clippy::disallowed_methods,
    reason = "the negative control re-runs this test binary as a child process, flagged by an environment variable"
)]
fn an_allocation_inside_the_guard_is_detected() {
    if std::env::var_os("ENTH_REVERB_NOALLOC_CHILD").is_some() {
        assert_no_alloc(|| {
            let v: Vec<f32> = Vec::with_capacity(16);
            std::hint::black_box(v);
        });
        return;
    }
    let exe = std::env::current_exe().expect("test executable path");
    let out = std::process::Command::new(exe)
        .args([
            "an_allocation_inside_the_guard_is_detected",
            "--exact",
            "--nocapture",
        ])
        .env("ENTH_REVERB_NOALLOC_CHILD", "1")
        .output()
        .expect("spawn child test");
    assert!(
        !out.status.success(),
        "an allocation inside assert_no_alloc was not detected"
    );
}

// --- through the engine ----------------------------------------------------------------------------------------

/// A saw through a gate envelope into a reverb with a short decay (so its tail dies within the render).
fn reverb_patch(settings: &[(ReverbParam, f32)]) -> Patch {
    let mut p = Patch::init();
    p.modulators[0].values[AdsrParam::Attack.index()] = 0.0;
    p.modulators[0].values[AdsrParam::Decay.index()] = 0.0;
    p.modulators[0].values[AdsrParam::Sustain.index()] = 100.0;
    p.modulators[0].values[AdsrParam::Release.index()] = 0.0;
    let mut m = ModuleInstance::new(id("reverb1"), ModuleKind::Reverb);
    for &(param, x) in settings {
        m.values[param.index()] = x;
    }
    p.fx.push(m);
    p
}

#[test]
fn the_engine_renders_a_tail_and_the_host_block_size_never_matters() {
    let patch = reverb_patch(&[
        (ReverbParam::Dry, -60.0),
        (ReverbParam::Wet, 0.0),
        (ReverbParam::Decay, 200.0),
    ]);
    let notes = [note(60, 0, 4_800), note(67, 10_000, 10_000)];
    let frames = 48_000 * 4;
    let (l0, r0, e0) = render(&patch, &notes, frames, 512, SEED);
    // A tail: there is signal long after the 0.1 s note, and it is not just the dry saw (dry is off).
    let late = &l0[24_000..48_000];
    assert!(late.iter().any(|x| x.abs() > 1e-6), "no tail");
    assert_eq!(e0.subnormals(), 0);
    for block in [1, 7, 32, 33, 64, 480, 4096] {
        let (l, r, _) = render(&patch, &notes, frames, block, SEED);
        assert_eq!(bits(&l), bits(&l0), "left, block {block}");
        assert_eq!(bits(&r), bits(&r0), "right, block {block}");
    }
}

#[test]
fn the_engine_runs_the_module_as_it_is_driven_by_hand() {
    // Decay 100 ms: the tail dies exactly after ~1.5 s, so the reverb is idle (and skipped by the engine) between
    // the two notes, and the second note arrives at 6 s. The reverb patch's output must equal a hand-driven module
    // fed the signal the reverb sees: the same patch without the reverb (dry is off, so the reverb patch's output
    // is the tail alone).
    let settings = [
        (ReverbParam::Dry, -60.0),
        (ReverbParam::Wet, 0.0),
        (ReverbParam::Decay, 100.0),
        (ReverbParam::Detune, 30.0),
    ];
    let with_reverb = reverb_patch(&settings);
    let mut without = reverb_patch(&settings);
    without.fx.clear();
    let notes = [note(55, 0, 2_400), note(62, 288_000, 291_000)];
    let frames = 48_000 * 8;
    let (wl, wr, _) = render(&with_reverb, &notes, frames, 64, SEED);
    let (dl, dr, _) = render(&without, &notes, frames, 64, SEED);

    let mut twin = Rig::new(SR, &values(&settings));
    let mut idle_segments = 0usize;
    let segments = dl.chunks(SEG).zip(dr.chunks(SEG));
    for (i, ((dl, dr), (wl, wr))) in segments.zip(wl.chunks(SEG).zip(wr.chunks(SEG))).enumerate() {
        let (mut l, mut r): (Block, Block) = (dl.try_into().unwrap(), dr.try_into().unwrap());
        twin.run(&mut l, &mut r);
        assert_eq!(bits(&l), bits(wl), "left, segment {i}");
        assert_eq!(bits(&r), bits(wr), "right, segment {i}");
        idle_segments += usize::from(twin.module.is_silent());
    }
    assert!(
        idle_segments > 2_000,
        "the module should have been idle for a good while ({idle_segments})"
    );
}

#[test]
fn the_engine_with_a_reverb_does_not_allocate() {
    // The engine-level noalloc tests cover the whole path; this one is the reverb alone with its tail, a
    // re-trigger and parameter automation values, the case the wrapper's idle machinery makes interesting.
    let patch = reverb_patch(&[(ReverbParam::Decay, 400.0)]);
    let mut e = engine(&patch, SEED);
    let ev = events(&[note(60, 0, 3_000), note(64, 70_000, 72_000)]);
    let (mut l, mut r) = (vec![0.0f32; 480], vec![0.0f32; 480]);
    let mut chunk = Vec::new();
    for block in 0..400 {
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
