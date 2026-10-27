//! The paired kernel of `dist`: the left and right voices of a stereo plan through one sample loop, two lanes of an
//! `f64x2` each (S2a lane PRF; `docs/dsp/dist.md`).
//!
//! A plan with the stereo voice path runs every per-voice effect on the left bus and again, on a second instance with
//! its own state, on the right bus (`enth-engine`'s `stereo`). The two see the same parameters and none of each
//! other's signal, so their per-sample work is the same operations on different numbers: this loop is [`super::run`]
//! written once for two lanes. **Every lane performs exactly the IEEE operations the scalar loop does, in the same
//! order** (the filters are the scalar ones made generic over [`Lane`], the ADAA step selects between its two
//! branches per lane, the shaping curves are the scalar functions applied to each lane), so each channel's output is
//! bit for bit what [`Voice::process`] gives it alone: the tests run both and compare every sample.
//!
//! The voices' states are packed into lanes for the block and written back after it, flushed once at the end as the
//! scalar block is. What this does not take (the digital modes, which are neither oversampled nor curves, and a pair
//! that is not in step) is left to [`Voice::process`] by the caller.

use wide::f64x2;

use crate::resample::{Lane, Oversampler, PhaseChain};
use crate::util::flush::flush64;

use super::{
    ADAA_EPS, Adaa, AsymCurve, Crossover, Ctl, Curve, DcBlock, HardCurve, Mode, OnePole, Setup,
    SineCurve, SoftCurve, Svf2, TriCurve, Voice, at, lerp,
};

type V = f64x2;

/// The ADAA stage of two voices: [`Adaa`] on two lanes.
#[derive(Clone, Copy)]
struct Adaa2 {
    u1: V,
    f1: V,
}

impl Adaa2 {
    /// One sample of both lanes through the curve `C`: the scalar [`Adaa::step`] on each.
    #[inline(always)]
    fn step<C: Curve>(&mut self, u: V) -> V {
        // The scalar step evaluates the antiderivative unconditionally, then takes the curve at the midpoint for a
        // small step and the divided difference otherwise.
        let fu = C::anti2(u);
        let d = u - self.u1;
        let small = d.abs().simd_lt(V::splat(ADAA_EPS));
        let quotient = (fu - self.f1) / d;
        let y = if small.any() {
            let mid = V::splat(0.5) * u + V::splat(0.5) * self.u1;
            small.select(C::f2(mid), quotient)
        } else {
            quotient
        };
        self.u1 = u;
        self.f1 = fu;
        y
    }

    fn flush(&mut self) {
        self.u1 = self.u1.flush();
        self.f1 = self.f1.flush();
    }
}

/// Everything a block of one voice's analog path touches, for two voices side by side (lane 0 left, lane 1 right).
struct Pair {
    xo: Crossover<V>,
    adaa: Adaa2,
    os: Oversampler<V>,
    dc: DcBlock<V>,
    pc_lo: PhaseChain<V>,
    pc_dry: PhaseChain<V>,
    tone: OnePole<V>,
    tone_ap: OnePole<V>,
}

#[inline(always)]
fn two(l: f64, r: f64) -> V {
    V::new([l, r])
}

#[inline(always)]
fn svf(l: &Svf2, r: &Svf2) -> Svf2<V> {
    Svf2 {
        ic1: two(l.ic1, r.ic1),
        ic2: two(l.ic2, r.ic2),
    }
}

#[inline(always)]
fn unsvf(p: &Svf2<V>, l: &mut Svf2, r: &mut Svf2) {
    let (a, b) = (p.ic1.to_array(), p.ic2.to_array());
    (l.ic1, r.ic1) = (a[0], a[1]);
    (l.ic2, r.ic2) = (b[0], b[1]);
}

impl Pair {
    #[inline(always)]
    fn pack(l: &Voice, r: &Voice) -> Self {
        Self {
            xo: Crossover {
                first: svf(&l.xo.first, &r.xo.first),
                low: svf(&l.xo.low, &r.xo.low),
                high: svf(&l.xo.high, &r.xo.high),
            },
            adaa: Adaa2 {
                u1: two(l.sh.adaa.u1, r.sh.adaa.u1),
                f1: two(l.sh.adaa.f1, r.sh.adaa.f1),
            },
            os: Oversampler::pack(&l.sh.os, &r.sh.os),
            dc: DcBlock {
                x1: two(l.dc.x1, r.dc.x1),
                y1: two(l.dc.y1, r.dc.y1),
            },
            pc_lo: PhaseChain::pack(&l.pc_lo, &r.pc_lo),
            pc_dry: PhaseChain::pack(&l.pc_dry, &r.pc_dry),
            tone: OnePole {
                s: two(l.tone.s, r.tone.s),
            },
            tone_ap: OnePole {
                s: two(l.tone_ap.s, r.tone_ap.s),
            },
        }
    }

    /// The scalar block ends with a flush of every state value (`Voice::flush`); this is the same, on the lanes.
    #[inline(always)]
    fn flush(&mut self) {
        self.xo.flush();
        self.adaa.flush();
        self.os.flush();
        self.dc.flush();
        self.pc_lo.flush();
        self.pc_dry.flush();
        self.tone.flush();
        self.tone_ap.flush();
    }

    #[inline(always)]
    fn unpack(&self, l: &mut Voice, r: &mut Voice) {
        unsvf(&self.xo.first, &mut l.xo.first, &mut r.xo.first);
        unsvf(&self.xo.low, &mut l.xo.low, &mut r.xo.low);
        unsvf(&self.xo.high, &mut l.xo.high, &mut r.xo.high);
        let (u, f) = (self.adaa.u1.to_array(), self.adaa.f1.to_array());
        l.sh.adaa = Adaa { u1: u[0], f1: f[0] };
        r.sh.adaa = Adaa { u1: u[1], f1: f[1] };
        self.os.unpack(&mut l.sh.os, &mut r.sh.os);
        let (x, y) = (self.dc.x1.to_array(), self.dc.y1.to_array());
        (l.dc.x1, r.dc.x1) = (x[0], x[1]);
        (l.dc.y1, r.dc.y1) = (y[0], y[1]);
        self.pc_lo.unpack(&mut l.pc_lo, &mut r.pc_lo);
        self.pc_dry.unpack(&mut l.pc_dry, &mut r.pc_dry);
        let (t, ta) = (self.tone.s.to_array(), self.tone_ap.s.to_array());
        (l.tone.s, r.tone.s) = (t[0], t[1]);
        (l.tone_ap.s, r.tone_ap.s) = (ta[0], ta[1]);
    }
}

/// One block for the voices `l` and `r` (both already begun, with the same `fade`): the same bits as
/// `Voice::render` and `Voice::flush` on each.
#[allow(
    clippy::too_many_arguments,
    reason = "the two voices, the shared setup and controls, the fade and the two blocks: all of it is the call"
)]
pub(super) fn render(
    l: &mut Voice,
    r: &mut Voice,
    su: &Setup,
    a: &Ctl,
    b: &Ctl,
    fade: (f64, f64),
    io_l: &mut [f32],
    io_r: &mut [f32],
) {
    if su.mode.is_digital() {
        // Converters have no curve and no oversampler: nothing to share.
        l.render_alone(su, a, b, fade, io_l);
        r.render_alone(su, a, b, fade, io_r);
        return;
    }
    #[cfg(test)]
    tests::PAIRED_BLOCKS.with(|c| c.set(c.get() + 1));
    let mut p = Pair::pack(l, r);
    let st = a == b;
    macro_rules! go {
        ($curve:ty, $k:literal) => {
            if st {
                run::<$curve, $k, true>(&mut p, su, a, b, fade, io_l, io_r)
            } else {
                run::<$curve, $k, false>(&mut p, su, a, b, fade, io_l, io_r)
            }
        };
    }
    match (su.mode, su.factor) {
        (Mode::Soft, 1) => go!(SoftCurve, 1),
        (Mode::Soft, 2) => go!(SoftCurve, 2),
        (Mode::Soft, _) => go!(SoftCurve, 4),
        (Mode::Hard, 1) => go!(HardCurve, 1),
        (Mode::Hard, 2) => go!(HardCurve, 2),
        (Mode::Hard, _) => go!(HardCurve, 4),
        (Mode::Asymmetric, 1) => go!(AsymCurve, 1),
        (Mode::Asymmetric, 2) => go!(AsymCurve, 2),
        (Mode::Asymmetric, _) => go!(AsymCurve, 4),
        (Mode::SineFold, 1) => go!(SineCurve, 1),
        (Mode::SineFold, 2) => go!(SineCurve, 2),
        (Mode::SineFold, _) => go!(SineCurve, 4),
        (Mode::TriangleFold, 1) => go!(TriCurve, 1),
        (Mode::TriangleFold, 2) => go!(TriCurve, 2),
        (Mode::TriangleFold, _) => go!(TriCurve, 4),
        // Handled above; the arm keeps the match total.
        (Mode::Bitcrush | Mode::Downsample, _) => {}
    }
    p.flush();
    p.unpack(l, r);
    // The converters' state is not packed (they never run here), but the scalar block flushes it too.
    for v in [l, r] {
        v.sh.hold = flush64(v.sh.hold);
        v.sh.acc = flush64(v.sh.acc);
    }
}

/// [`super::run`] for two voices: the sample loop with the analog shaper.
#[inline(always)]
fn run<C: Curve, const K: usize, const STATIC: bool>(
    p: &mut Pair,
    su: &Setup,
    a: &Ctl,
    b: &Ctl,
    fade: (f64, f64),
    io_l: &mut [f32],
    io_r: &mut [f32],
) {
    let inv_n = 1.0 / io_l.len() as f64;
    let open = a.tone_open && b.tone_open;
    let full = fade.0 == 1.0 && fade.1 == 1.0;
    for (i, (sl, sr)) in io_l.iter_mut().zip(io_r.iter_mut()).enumerate() {
        let t = (i + 1) as f64 * inv_n;
        let x = two(f64::from(*sl), f64::from(*sr));
        let (lo, hi) = if su.split {
            p.xo.step(&su.xo, x)
        } else {
            (V::ZERO, x)
        };
        let u = V::splat(at::<STATIC>(a.gain, b.gain, t)) * hi
            + V::splat(at::<STATIC>(a.bias, b.bias, t));
        let adaa = &mut p.adaa;
        let w = p.os.process::<K>(u, &mut |s| adaa.step::<C>(s));
        let mut wet = p.dc.step(su.dc_r, w);
        let d0 = if su.split {
            wet += p.pc_lo.step(&su.pc, su.pc_n, lo);
            lo + hi
        } else {
            x
        };
        let mut dry = p.pc_dry.step(&su.pc, su.pc_n, d0);
        if open {
            p.tone.s = wet;
            p.tone_ap.s = dry;
        } else {
            wet = p.tone.lp(at::<STATIC>(a.tone_g, b.tone_g, t), wet);
            dry = V::splat(2.0) * p.tone_ap.lp(at::<STATIC>(a.tone_ga, b.tone_ga, t), dry) - dry;
        }
        let m = at::<STATIC>(a.mix, b.mix, t);
        let blend = V::splat(1.0 - m) * dry + V::splat(m) * wet;
        let out = if full {
            blend
        } else {
            x + V::splat(lerp(fade.0, fade.1, t)) * (blend - x)
        };
        let [ol, or] = out.to_array();
        *sl = ol as f32;
        *sr = or as f32;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::math::sin;

    use core::cell::Cell;

    const SR: f32 = 48_000.0;
    const N: usize = 32;

    thread_local! {
        /// How many blocks went through the lane kernel on this thread (so a test can tell the kernel ran, not the
        /// plain calls the pair falls back to).
        pub(super) static PAIRED_BLOCKS: Cell<usize> = const { Cell::new(0) };
    }

    const MODES: [Mode; 7] = [
        Mode::Soft,
        Mode::Hard,
        Mode::Asymmetric,
        Mode::SineFold,
        Mode::TriangleFold,
        Mode::Bitcrush,
        Mode::Downsample,
    ];

    /// A block of channel `ch` (0 left, 1 right) at step `n`: levels from silence through the ADAA's tiny-step branch
    /// to a hard overdrive (past `ln cosh`'s 20), with a DC offset on some blocks, and different on the two channels.
    fn signal(n: usize, ch: usize) -> [f32; N] {
        let amp = [0.8_f64, 0.0, 0.0005, 2.0, 0.3, 40.0, 0.6, 0.0][(n + 3 * ch) % 8];
        let w = [0.037, 0.051][ch] * (1.0 + (n % 3) as f64);
        let dc = if (n + ch).is_multiple_of(5) { 0.3 } else { 0.0 };
        core::array::from_fn(|i| (amp * sin(w * (n * N + i) as f64) + dc) as f32)
    }

    /// The state of a voice as text: `Debug` prints the sign of a zero, so equal text is equal bits.
    fn state(v: &Voice) -> String {
        format!("{v:?}")
    }

    fn bits(x: &[f32; N]) -> Vec<u32> {
        x.iter().map(|s| s.to_bits()).collect()
    }

    /// One step of a scenario: the setup and the controls at the start and the end of the block.
    type Step = (Setup, Ctl, Ctl);

    /// Run `steps` on a left and a right voice twice, as two `Voice::process` calls and as one `process_pair`, and
    /// compare every output sample and the whole state of both voices after each block. `restart` resets one voice
    /// (0 left, 1 right) before the given block, in both runs.
    fn same_as_two_scalars(steps: &[Step], restart: Option<(usize, usize)>) {
        let (mut sl, mut sr) = (Voice::ZERO, Voice::ZERO);
        let (mut pl, mut pr) = (Voice::ZERO, Voice::ZERO);
        for (n, (su, a, b)) in steps.iter().enumerate() {
            if let Some((_, which)) = restart.filter(|&(at, _)| at == n) {
                [&mut sl, &mut sr][which].reset();
                [&mut pl, &mut pr][which].reset();
            }
            let (mut xl, mut xr) = (signal(n, 0), signal(n, 1));
            let (mut yl, mut yr) = (xl, xr);
            sl.process(su, a, b, &mut xl);
            sr.process(su, a, b, &mut xr);
            pl.process_pair(&mut pr, su, a, b, &mut yl, &mut yr);
            assert_eq!(
                bits(&xl),
                bits(&yl),
                "left output, block {n}, {:?}",
                su.key()
            );
            assert_eq!(
                bits(&xr),
                bits(&yr),
                "right output, block {n}, {:?}",
                su.key()
            );
            assert_eq!(
                state(&sl),
                state(&pl),
                "left state, block {n}, {:?}",
                su.key()
            );
            assert_eq!(
                state(&sr),
                state(&pr),
                "right state, block {n}, {:?}",
                su.key()
            );
        }
    }

    fn ctl(drive: f32, bias: f32, tone: f32, mix: f32) -> Ctl {
        Ctl::new(drive, bias, 6.0, 9_000.0, tone, mix, SR)
    }

    #[test]
    fn static_controls_in_every_mode_factor_crossover_and_tone() {
        for mode in MODES {
            for factor in [1, 2, 4] {
                for keep in [20.0, 140.0] {
                    for tone in [20_000.0, 3_500.0] {
                        let su = Setup::new(mode, factor, keep, SR);
                        let c = ctl(17.0, 11.0, tone, 85.0);
                        let steps = vec![(su, c, c); 24];
                        let before = PAIRED_BLOCKS.with(Cell::get);
                        same_as_two_scalars(&steps, None);
                        let ran = PAIRED_BLOCKS.with(Cell::get) - before;
                        // Every block takes the kernel but a converter's (it has no lanes to share).
                        assert_eq!(
                            ran,
                            if mode.is_digital() { 0 } else { 24 },
                            "{mode:?} x{factor}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn ramped_controls_in_every_mode() {
        for mode in MODES {
            for factor in [1, 2, 4] {
                let su = Setup::new(mode, factor, 90.0, SR);
                let mut steps = Vec::new();
                let mut prev = ctl(6.0, 0.0, 8_000.0, 100.0);
                for n in 0..30 {
                    let next = ctl(
                        6.0 + 1.3 * n as f32,
                        (n % 7) as f32 * 4.0,
                        if n % 4 == 0 {
                            20_000.0
                        } else {
                            2_000.0 + 400.0 * n as f32
                        },
                        100.0 - 2.0 * n as f32,
                    );
                    steps.push((su, prev, next));
                    prev = next;
                }
                same_as_two_scalars(&steps, None);
            }
        }
    }

    #[test]
    fn the_mix_fade_in_and_out_and_a_silent_start() {
        let su = Setup::new(Mode::Soft, 2, 140.0, SR);
        let off = ctl(12.0, 5.0, 6_000.0, 0.0);
        let on = ctl(12.0, 5.0, 6_000.0, 70.0);
        let mut steps = Vec::new();
        steps.extend(vec![(su, off, off); 6]);
        steps.push((su, off, on));
        steps.extend(vec![(su, on, on); 14]);
        steps.push((su, on, off));
        steps.extend(vec![(su, off, off); 14]);
        steps.extend(vec![(su, on, on); 6]);
        same_as_two_scalars(&steps, None);
    }

    #[test]
    fn a_structure_change_mid_stream() {
        let mut steps = Vec::new();
        let c = ctl(15.0, 3.0, 7_000.0, 90.0);
        for (n, mode) in MODES.iter().cycle().take(21).enumerate() {
            let su = Setup::new(
                *mode,
                [1, 2, 4][n % 3],
                if n % 2 == 0 { 20.0 } else { 110.0 },
                SR,
            );
            steps.extend(vec![(su, c, c); 3]);
        }
        same_as_two_scalars(&steps, None);
    }

    #[test]
    fn one_voice_restarting_mid_stream_leaves_the_pair_out_of_step() {
        // A restarted voice starts at a fade of 1 while the other is still ramping in: they cannot share a ramp, and
        // must still come out as the two scalar calls.
        let su = Setup::new(Mode::Asymmetric, 4, 140.0, SR);
        let off = ctl(12.0, 5.0, 6_000.0, 0.0);
        let on = ctl(12.0, 5.0, 6_000.0, 70.0);
        let mut steps = vec![(su, off, off); 4];
        steps.push((su, off, on));
        steps.extend(vec![(su, on, on); 12]);
        for which in [0, 1] {
            for at in [2, 5, 6, 9] {
                same_as_two_scalars(&steps, Some((at, which)));
            }
        }
    }

    #[test]
    fn blocks_the_pair_cannot_take_run_as_two_calls() {
        let su = Setup::new(Mode::Soft, 2, 20.0, SR);
        let c = ctl(14.0, 0.0, 20_000.0, 100.0);
        // Different lengths, and an empty block: the plain calls.
        let (mut sl, mut sr, mut pl, mut pr) = (Voice::ZERO, Voice::ZERO, Voice::ZERO, Voice::ZERO);
        let long = signal(1, 0);
        let (mut a, mut b) = (long, long);
        let (mut c1, mut c2) = (long, long);
        sl.process(&su, &c, &c, &mut a[..20]);
        sr.process(&su, &c, &c, &mut b[..12]);
        pl.process_pair(&mut pr, &su, &c, &c, &mut c1[..20], &mut c2[..12]);
        assert_eq!(bits(&a), bits(&c1));
        assert_eq!(bits(&b), bits(&c2));
        assert_eq!(state(&sl), state(&pl));
        assert_eq!(state(&sr), state(&pr));
        let mut none: [f32; 0] = [];
        let mut none2: [f32; 0] = [];
        pl.process_pair(&mut pr, &su, &c, &c, &mut none, &mut none2);
        assert_eq!(state(&sl), state(&pl));
    }

    /// The curves on two lanes, against the scalar functions, over every region the curves and `ln cosh` branch on.
    fn curve_lanes_match<C: Curve>(name: &str) {
        let mut probe = vec![
            0.0,
            -0.0,
            1e-300,
            -1e-300,
            1e-12,
            -1e-12,
            0.5,
            -0.5,
            1.0,
            -1.0,
            1.5,
            -1.5,
            3.0,
            -3.0,
            9.99,
            -9.99,
            10.0,
            -10.0,
            19.9,
            -19.9,
            20.0,
            -20.0,
            20.1,
            -20.1,
            39.99,
            -39.99,
            40.0,
            -40.0,
            40.1,
            -40.1,
            1e5,
            -1e5,
            1e300,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NAN,
            -f64::NAN,
        ];
        let mut rng = crate::util::rng::Rng::new(0xD157_0001);
        for _ in 0..200_000 {
            probe.push((rng.unit_f64() - 0.5) * 100.0);
            probe.push((rng.unit_f64() - 0.5) * 4.0);
            probe.push((rng.unit_f64() - 0.5) * 1e-4);
        }
        for pair in probe.chunks(2) {
            let (a, b) = (pair[0], pair[pair.len() - 1]);
            let (got_f, got_anti) = (C::f2(two(a, b)).to_array(), C::anti2(two(a, b)).to_array());
            for (i, x) in [a, b].into_iter().enumerate() {
                let (f, anti) = (C::f(x), C::anti(x));
                assert!(
                    got_f[i].to_bits() == f.to_bits() || (got_f[i].is_nan() && f.is_nan()),
                    "{name}.f({x:e}): {:e} vs {f:e}",
                    got_f[i]
                );
                assert!(
                    got_anti[i].to_bits() == anti.to_bits()
                        || (got_anti[i].is_nan() && anti.is_nan()),
                    "{name}.anti({x:e}): {:e} vs {anti:e}",
                    got_anti[i]
                );
            }
        }
    }

    #[test]
    fn every_curve_on_two_lanes_is_the_scalar_curve_on_each() {
        curve_lanes_match::<SoftCurve>("soft");
        curve_lanes_match::<HardCurve>("hard");
        curve_lanes_match::<AsymCurve>("asym");
        curve_lanes_match::<SineCurve>("sine");
        curve_lanes_match::<TriCurve>("tri");
    }

    #[test]
    fn the_lane_flush_is_the_scalar_flush() {
        let probe = [
            0.0,
            -0.0,
            1e-300,
            -1e-300,
            5e-324,
            -5e-324,
            1e-16,
            -1e-16,
            0.999e-15,
            -0.999e-15,
            1e-15,
            -1e-15,
            1.001e-15,
            -1.001e-15,
            3e-10,
            -0.5,
            1.0,
            1e300,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NAN,
        ];
        for pair in probe.windows(2) {
            let got = V::new([pair[0], pair[1]]).flush().to_array();
            for (g, x) in got.iter().zip(pair) {
                let want = flush64(*x);
                assert!(
                    g.to_bits() == want.to_bits() || (g.is_nan() && want.is_nan()),
                    "flush({x:e}): {g:e} vs {want:e}"
                );
            }
        }
    }
}
