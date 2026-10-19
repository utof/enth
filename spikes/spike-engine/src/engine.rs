//! The voice engine: up to three polyBLEP saw oscillators -> TPT state-variable low-pass -> rational tanh
//! -> amplitude envelope, one voice per note, summed in voice order, then an optional FDN reverb.
//!
//! Layout follows design.md §3: per-voice state in structure-of-arrays form, control-rate modulation
//! (envelopes, LFO, filter coefficients every [`CTRL`] samples with a per-sample amplitude ramp), a fixed
//! block size, and `process()` that never allocates, locks, logs or does I/O.
//!
//! Determinism (design.md §5): every operation is IEEE `+ - * /` in a fixed order, transcendentals only
//! through [`crate::math`] (pinned `libm`), no fused multiply-add, voices summed in index order.
//! The scalar path and the SIMD-lane paths (`wide` f32x4 / f32x8, one voice per lane) evaluate the same
//! expressions in the same order, so their output is bit-identical; tests enforce that.

use wide::{f32x4, f32x8};

use crate::fdn::Fdn;
use crate::math::{self, FLUSH_THRESHOLD, VOICE_END, flush, tanh_approx};
use crate::patch::{Env, Patch};
use crate::prng::{self, Rng};
use crate::{BLOCK, CTRL, MAX_VOICES, SAMPLE_RATE};

/// A note of the performance: frames, MIDI pitch (fractional allowed), velocity 0..1.
/// Start and end are quantised up to the control rate ([`CTRL`] samples = 0.67 ms).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Note {
    pub start: usize,
    pub len: usize,
    pub pitch: f32,
    pub vel: f32,
}

/// Which voice-bank implementation runs. All three must give identical output.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Path {
    /// One voice at a time.
    Scalar,
    /// Four voices per lane group (`f32x4`).
    X4,
    /// Eight voices per lane group (`f32x8`).
    X8,
}

/// Denormal policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flush {
    /// Product behaviour: recursive state is flushed in code, voices end below -100 dB.
    On,
    /// Negative control for the FTZ and subnormal tests only: nothing is flushed, voices never end.
    Off,
}

const ATT: u8 = 0;
const DEC: u8 = 1;
const REL: u8 = 2;
const IDLE: u8 = 3;

const MAX_OSC: usize = 3;
const SEGMENTS: usize = BLOCK / CTRL;
const PI_OVER_SR: f32 = core::f32::consts::PI / SAMPLE_RATE;
const TAU: f32 = core::f32::consts::TAU;
/// log2(10) / 20: dB to octaves of amplitude (so `2^(db * this)` = 10^(db/20)).
const DB_TO_LOG2: f32 = 0.166_096_4;
const MAX_CUTOFF_HZ: f32 = 0.45 * SAMPLE_RATE;

#[derive(Clone, Copy)]
struct EnvCoef {
    att_per_sample: f32,
    dec_c: f32,
    sus: f32,
    rel_c: f32,
}

impl EnvCoef {
    fn new(e: &Env) -> Self {
        let per_ms = 0.001 * SAMPLE_RATE;
        Self {
            att_per_sample: 1.0 / (e.attack_ms * per_ms),
            dec_c: math::exp(-(CTRL as f32) / (e.decay_ms * per_ms)),
            sus: e.sustain_pct * 0.01,
            rel_c: math::exp(-(CTRL as f32) / (e.release_ms * per_ms)),
        }
    }
}

#[derive(Clone, Copy)]
struct EnvState {
    level: f32,
    stage: u8,
}

impl EnvState {
    const NEW: Self = Self {
        level: 0.0,
        stage: ATT,
    };

    #[inline(always)]
    fn step(&mut self, c: &EnvCoef, release: bool, len: usize, flush_on: bool) {
        if release && self.stage != REL {
            self.stage = REL;
        }
        match self.stage {
            ATT => {
                self.level += len as f32 * c.att_per_sample;
                if self.level >= 1.0 {
                    self.level = 1.0;
                    self.stage = DEC;
                }
            }
            DEC => {
                let mut d = (self.level - c.sus) * c.dec_c;
                if flush_on {
                    d = flush(d);
                }
                self.level = c.sus + d;
            }
            _ => {
                let mut l = self.level * c.rel_c;
                if flush_on {
                    l = flush(l);
                }
                self.level = l;
            }
        }
    }
}

/// Per-voice control state (always scalar; shared by every path).
#[derive(Clone, Copy)]
struct Ctl {
    on: usize,
    off: usize,
    vel: f32,
    key_oct: f32,
    ae: EnvState,
    fe: EnvState,
}

/// Patch-derived constants, computed once in `Engine::new`.
#[derive(Clone, Copy)]
struct Consts {
    n_osc: usize,
    lvl: [f32; MAX_OSC],
    osc_scale: f32,
    cutoff: f32,
    res_k: f32,
    env_oct: f32,
    lfo_oct: f32,
    drive: f32,
    vib_oct: f32,
    amp_env: EnvCoef,
    filter_env: EnvCoef,
    lfo_inc: f32,
}

/// The engine. `new` allocates; `process` does not.
pub struct Engine {
    path: Path,
    flush_on: bool,
    k: Consts,
    n: usize,
    pos: usize,
    lfo_phase: f32,
    ctl: Vec<Ctl>,
    ph: [Vec<f32>; MAX_OSC],
    dt: [Vec<f32>; MAX_OSC],
    base_dt: [Vec<f32>; MAX_OSC],
    ic1: Vec<f32>,
    ic2: Vec<f32>,
    a1: Vec<f32>,
    a2: Vec<f32>,
    a3: Vec<f32>,
    amp: Vec<f32>,
    inc: Vec<f32>,
    gain: Vec<f32>,
    fdn: Option<Fdn>,
}

fn quantise_up(x: usize) -> usize {
    x.div_ceil(CTRL) * CTRL
}

impl Engine {
    /// Prepare an engine for `patch` and a performance. Allocates. `seed` drives the per-voice start phases
    /// and detune (stream `derive(seed, voice)`) and the reverb's delay jitter (stream `derive(seed, MODULE_REVERB)`).
    ///
    /// # Panics
    /// If there are more than [`MAX_VOICES`] notes.
    #[must_use]
    pub fn new(patch: &Patch, notes: &[Note], seed: u64, path: Path, flush: Flush) -> Self {
        let n = notes.len();
        assert!(n <= MAX_VOICES, "at most {MAX_VOICES} voices");
        let flush_on = flush == Flush::On;
        let pad = n.next_multiple_of(8).max(8);
        let n_osc = usize::from(patch.osc_count).clamp(1, MAX_OSC);
        let k = Consts {
            n_osc,
            lvl: patch.osc.map(|o| o.level_pct * 0.01),
            osc_scale: 1.0 / n_osc as f32,
            cutoff: patch.filter.cutoff_hz,
            res_k: 1.0 / patch.filter.q,
            env_oct: patch.filter.env_oct,
            lfo_oct: patch.filter.lfo_oct,
            drive: patch.filter.drive,
            vib_oct: patch.lfo.vibrato_cents / 1200.0,
            amp_env: EnvCoef::new(&patch.amp_env),
            filter_env: EnvCoef::new(&patch.filter_env),
            lfo_inc: patch.lfo.rate_hz * CTRL as f32 / SAMPLE_RATE,
        };
        let master = math::exp2(patch.master_db * DB_TO_LOG2);
        let key_track = patch.filter.key_track_pct * 0.01;
        let zeros = || vec![0.0f32; pad];
        let mut e = Self {
            path,
            flush_on,
            k,
            n,
            pos: 0,
            lfo_phase: 0.0,
            ctl: vec![
                Ctl {
                    on: 0,
                    off: 0,
                    vel: 0.0,
                    key_oct: 0.0,
                    ae: EnvState {
                        level: 0.0,
                        stage: IDLE
                    },
                    fe: EnvState::NEW
                };
                pad
            ],
            ph: [zeros(), zeros(), zeros()],
            dt: [zeros(), zeros(), zeros()],
            base_dt: [zeros(), zeros(), zeros()],
            ic1: zeros(),
            ic2: zeros(),
            a1: zeros(),
            a2: zeros(),
            a3: zeros(),
            amp: zeros(),
            inc: zeros(),
            gain: zeros(),
            fdn: (patch.reverb.mix_pct > 0.0).then(|| Fdn::new(&patch.reverb, seed, flush_on)),
        };
        // Padding lanes never sound but must not divide by zero.
        for o in 0..MAX_OSC {
            e.dt[o].fill(0.01);
            e.base_dt[o].fill(0.01);
        }
        for (v, note) in notes.iter().enumerate() {
            let mut rng = Rng::new(prng::derive(seed, v as u64));
            let detune = (rng.unit() - 0.5) * 0.2; // +-10 cents, in semitones
            for o in 0..MAX_OSC {
                e.ph[o][v] = rng.unit();
                let sp = &patch.osc[o];
                let semis = note.pitch + sp.semitones + sp.cents * 0.01 + detune;
                let hz = 440.0 * math::exp2((semis - 69.0) / 12.0);
                let dt = (hz / SAMPLE_RATE).min(0.45);
                e.dt[o][v] = dt;
                e.base_dt[o][v] = dt;
            }
            e.gain[v] = 0.5 * master / n as f32;
            e.ctl[v] = Ctl {
                on: quantise_up(note.start),
                off: quantise_up(note.start + note.len),
                vel: note.vel,
                key_oct: key_track * ((note.pitch - 60.0) / 12.0),
                ae: EnvState::NEW,
                fe: EnvState::NEW,
            };
        }
        e
    }

    /// Frames rendered so far.
    #[must_use]
    pub fn position(&self) -> usize {
        self.pos
    }

    /// Render the next `out.len()` (at most [`BLOCK`]) mono samples. Never allocates, locks or blocks.
    pub fn process(&mut self, out: &mut [f32]) {
        debug_assert!(out.len() <= BLOCK);
        out.fill(0.0);
        // The LFO is global, advanced once per control segment before any voice runs.
        let mut lfo = [0.0f32; SEGMENTS];
        for l in lfo.iter_mut().take(out.len().div_ceil(CTRL)) {
            *l = math::sin(TAU * self.lfo_phase);
            self.lfo_phase += self.k.lfo_inc;
            if self.lfo_phase >= 1.0 {
                self.lfo_phase -= 1.0;
            }
        }
        match self.path {
            Path::Scalar => self.process_scalar(out, lfo),
            Path::X4 => self.process_x4(out, lfo),
            Path::X8 => self.process_x8(out, lfo),
        }
        if let Some(fdn) = self.fdn.as_mut() {
            fdn.process(out);
        }
        if self.flush_on {
            for x in out.iter_mut() {
                *x = flush(*x);
            }
        }
        self.pos += out.len();
    }

    #[inline(always)]
    fn live(&self, v: usize, j_abs: usize) -> bool {
        self.ctl[v].ae.stage != IDLE && j_abs >= self.ctl[v].on
    }

    /// Control-rate update of voice `v` for the segment starting at absolute frame `j_abs`.
    #[inline(always)]
    fn control(&mut self, v: usize, j_abs: usize, len: usize, lfo: f32) {
        let flush_on = self.flush_on;
        let k = &self.k;
        let c = &mut self.ctl[v];
        if c.ae.stage == IDLE || j_abs < c.on {
            return;
        }
        let start_level = c.ae.level;
        let release = j_abs >= c.off;
        c.ae.step(&k.amp_env, release, len, flush_on);
        c.fe.step(&k.filter_env, release, len, flush_on);
        if flush_on && c.ae.stage != ATT && c.ae.level < VOICE_END {
            c.ae = EnvState {
                level: 0.0,
                stage: IDLE,
            };
            self.ic1[v] = 0.0;
            self.ic2[v] = 0.0;
            self.amp[v] = 0.0;
            self.inc[v] = 0.0;
            return;
        }
        let oct = k.env_oct * c.fe.level + k.lfo_oct * lfo + c.key_oct;
        let fc = (k.cutoff * math::exp2(oct)).clamp(20.0, MAX_CUTOFF_HZ);
        let g = math::tan(PI_OVER_SR * fc);
        let a1 = 1.0 / (1.0 + g * (g + k.res_k));
        let a2 = g * a1;
        self.a1[v] = a1;
        self.a2[v] = a2;
        self.a3[v] = g * a2;
        let vf = math::exp2(k.vib_oct * lfo);
        for o in 0..k.n_osc {
            self.dt[o][v] = self.base_dt[o][v] * vf;
        }
        if flush_on {
            self.ic1[v] = flush(self.ic1[v]);
            self.ic2[v] = flush(self.ic2[v]);
        }
        let amp0 = start_level * c.vel;
        self.amp[v] = amp0;
        self.inc[v] = (c.ae.level * c.vel - amp0) / len as f32;
    }

    /// Scalar bank: voices outer, samples inner; `out[k]` accumulates voice 0, 1, 2, ... in that order.
    fn process_scalar(&mut self, out: &mut [f32], lfo: [f32; SEGMENTS]) {
        let len = out.len();
        let k = self.k;
        for v in 0..self.n {
            let mut j = 0;
            let mut seg = 0;
            while j < len {
                let cend = (j + CTRL).min(len);
                let j_abs = self.pos + j;
                self.control(v, j_abs, cend - j, lfo[seg]);
                if self.live(v, j_abs) {
                    let mut ph = [self.ph[0][v], self.ph[1][v], self.ph[2][v]];
                    let dt = [self.dt[0][v], self.dt[1][v], self.dt[2][v]];
                    let (a1, a2, a3) = (self.a1[v], self.a2[v], self.a3[v]);
                    let (mut ic1, mut ic2) = (self.ic1[v], self.ic2[v]);
                    let (mut amp, inc, gain) = (self.amp[v], self.inc[v], self.gain[v]);
                    for o in &mut out[j..cend] {
                        let mut s = 0.0f32;
                        for oi in 0..k.n_osc {
                            let t = ph[oi];
                            let saw = 2.0 * t - 1.0 - poly_blep(t, dt[oi]);
                            s += k.lvl[oi] * saw;
                            let p = t + dt[oi];
                            ph[oi] = if p >= 1.0 { p - 1.0 } else { p };
                        }
                        s *= k.osc_scale;
                        let v3 = s - ic2;
                        let v1 = a1 * ic1 + a2 * v3;
                        let v2 = ic2 + a2 * ic1 + a3 * v3;
                        ic1 = 2.0 * v1 - ic1;
                        ic2 = 2.0 * v2 - ic2;
                        amp += inc;
                        *o += tanh_approx(v2 * k.drive) * amp * gain;
                    }
                    for (dst, &p) in self.ph.iter_mut().zip(&ph) {
                        dst[v] = p;
                    }
                    self.ic1[v] = ic1;
                    self.ic2[v] = ic2;
                    self.amp[v] = amp;
                }
                j = cend;
                seg += 1;
            }
        }
    }

    /// Number of subnormal floats in the engine's state (voices, envelopes, LFO, reverb).
    #[must_use]
    pub fn count_subnormals(&self) -> usize {
        let sub = |v: &Vec<f32>| v.iter().filter(|x| x.is_subnormal()).count();
        let mut n: usize = [
            &self.ic1, &self.ic2, &self.a1, &self.a2, &self.a3, &self.amp, &self.inc, &self.gain,
        ]
        .into_iter()
        .map(sub)
        .sum();
        for o in 0..MAX_OSC {
            n += sub(&self.ph[o]) + sub(&self.dt[o]) + sub(&self.base_dt[o]);
        }
        for c in &self.ctl {
            n += usize::from(c.ae.level.is_subnormal()) + usize::from(c.fe.level.is_subnormal());
        }
        n += usize::from(self.lfo_phase.is_subnormal());
        n + self.fdn.as_ref().map_or(0, Fdn::count_subnormals)
    }

    /// Number of voices that are still sounding or waiting to start.
    #[must_use]
    pub fn active_voices(&self) -> usize {
        self.ctl[..self.n]
            .iter()
            .filter(|c| c.ae.stage != IDLE)
            .count()
    }
}

#[inline(always)]
fn poly_blep(t: f32, dt: f32) -> f32 {
    if t < dt {
        let x = t / dt;
        x + x - x * x - 1.0
    } else if t > 1.0 - dt {
        let x = (t - 1.0) / dt;
        x * x + x + x + 1.0
    } else {
        0.0
    }
}

/// Lane implementations: `$L` voices per group, one per lane, same expressions as `process_scalar`.
macro_rules! lanes_impl {
    ($name:ident, $V:ty, $L:expr) => {
        impl Engine {
            fn $name(&mut self, out: &mut [f32], lfo: [f32; SEGMENTS]) {
                let len = out.len();
                let k = self.k;
                let zero = <$V>::splat(0.0);
                let one = <$V>::splat(1.0);
                let two = <$V>::splat(2.0);
                let lvl = k.lvl.map(<$V>::splat);
                let osc_scale = <$V>::splat(k.osc_scale);
                let drive = <$V>::splat(k.drive);
                let (lo, hi) = (<$V>::splat(-3.0), <$V>::splat(3.0));
                let (c27, c9) = (<$V>::splat(27.0), <$V>::splat(9.0));
                for base in (0..self.n).step_by($L) {
                    let mut j = 0;
                    let mut seg = 0;
                    while j < len {
                        let cend = (j + CTRL).min(len);
                        let j_abs = self.pos + j;
                        let mut live = [false; $L];
                        for (l, lv) in live.iter_mut().enumerate() {
                            let v = base + l;
                            if v < self.n {
                                self.control(v, j_abs, cend - j, lfo[seg]);
                                *lv = self.live(v, j_abs);
                            }
                        }
                        if live.iter().any(|&b| b) {
                            let ld = |x: &[f32]| -> $V {
                                let mut a = [0.0f32; $L];
                                a.copy_from_slice(&x[base..base + $L]);
                                <$V>::from(a)
                            };
                            let mut ph = [ld(&self.ph[0]), ld(&self.ph[1]), ld(&self.ph[2])];
                            let dt = [ld(&self.dt[0]), ld(&self.dt[1]), ld(&self.dt[2])];
                            let (a1, a2, a3) = (ld(&self.a1), ld(&self.a2), ld(&self.a3));
                            let (mut ic1, mut ic2) = (ld(&self.ic1), ld(&self.ic2));
                            let (mut amp, inc, gain) =
                                (ld(&self.amp), ld(&self.inc), ld(&self.gain));
                            for o in &mut out[j..cend] {
                                let mut s = zero;
                                for oi in 0..k.n_osc {
                                    let t = ph[oi];
                                    let d = dt[oi];
                                    let x1 = t / d;
                                    let b1 = x1 + x1 - x1 * x1 - one;
                                    let x2 = (t - one) / d;
                                    let b2 = x2 * x2 + x2 + x2 + one;
                                    let blep = t
                                        .simd_lt(d)
                                        .select(b1, t.simd_gt(one - d).select(b2, zero));
                                    let saw = two * t - one - blep;
                                    s += lvl[oi] * saw;
                                    let p = t + d;
                                    ph[oi] = p.simd_ge(one).select(p - one, p);
                                }
                                s *= osc_scale;
                                let v3 = s - ic2;
                                let v1 = a1 * ic1 + a2 * v3;
                                let v2 = ic2 + a2 * ic1 + a3 * v3;
                                ic1 = two * v1 - ic1;
                                ic2 = two * v2 - ic2;
                                amp += inc;
                                let x = (v2 * drive).max(lo).min(hi);
                                let x2 = x * x;
                                let th = x * (c27 + x2) / (c27 + c9 * x2);
                                let y = (th * amp * gain).to_array();
                                // Fixed order: lane 0, 1, 2, ... = voice order, exactly like the scalar bank.
                                for (l, yl) in y.iter().enumerate() {
                                    if live[l] {
                                        *o += *yl;
                                    }
                                }
                            }
                            let ph_arr = ph.map(|p| p.to_array());
                            let (ic1, ic2, amp) = (ic1.to_array(), ic2.to_array(), amp.to_array());
                            for (l, &is_live) in live.iter().enumerate() {
                                if is_live {
                                    let v = base + l;
                                    for (dst, p) in self.ph.iter_mut().zip(&ph_arr) {
                                        dst[v] = p[l];
                                    }
                                    self.ic1[v] = ic1[l];
                                    self.ic2[v] = ic2[l];
                                    self.amp[v] = amp[l];
                                }
                            }
                        }
                        j = cend;
                        seg += 1;
                    }
                }
            }
        }
    };
}

lanes_impl!(process_x4, f32x4, 4);
lanes_impl!(process_x8, f32x8, 8);

const _: () = assert!(BLOCK.is_multiple_of(CTRL) && SEGMENTS == 2);
const _: () = assert!(
    FLUSH_THRESHOLD > 1e-30,
    "flush threshold must sit far above the subnormal range"
);
