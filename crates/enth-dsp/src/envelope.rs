//! RC-style exponential ADSR envelope core (P1a lane D2). Notes: `docs/dsp/envelope.md`.
//!
//! One envelope is a small state machine stepped once per control step (the engine's 32-frame segment). Each
//! segment (attack, decay, release) moves the level towards an *overshoot target* beyond its end,
//!
//! ```text
//! target = end + (end − start)·r         level ← target + (level − target)·c        c = (r / (1 + r))^(1/N)
//! ```
//!
//! which is an analogue RC charge curve cut off where it crosses the end. Because the overshoot is proportional to
//! the segment's own span, the curve crosses the end after exactly `N` steps from *any* start level (Nigel Redmon's
//! target-ratio idea, `EarLevel` 2013-06-03, used for the idea only; the code is ours). `N` is **fractional**: the
//! curve is the continuous-time one, sampled at whole steps, and the first step at or after `N` lands on the end
//! exactly (the step counter snaps it, whatever the rounding). The levels therefore vary continuously with the set
//! time (a 1e-3 nudge of a time moves the output by about as much, not by a whole step of a quantised length), which
//! finite-difference hints and the seed-stability test rely on.
//!
//! **Segment-duration semantics**: attack, decay and release are *times*, and the time is how long the segment
//! takes, so `decay = 300 ms` reaches the sustain level at 300 ms (at the first control step at or after it; never
//! early, at most one step late), not "a rate that would take 300 ms to fall the whole range".
//!
//! **Minimum ramp**: a segment lasts at least [`MIN_RAMP_MS`] (1 ms), 1.5 steps at 48 kHz, so it needs
//! [`Timing::min_steps`] = 2 control steps. A time of 0 ms is therefore a ramp of about a millisecond, never a jump,
//! and for the shortest segments the curve flattens towards a straight line ([`overshoot`]), so that ramp is a
//! real ramp and not a 40 dB drop in the first step.
//!
//! Determinism: `exp2`, `ln` and `exp` run only when a segment starts, in `f64` through `util::math`; a step is one
//! multiply-add (written as a multiply and an add), and the level is flushed. Zero-span segments (attack from 1.0,
//! decay to a sustain of 100 %, release from 0) finish on their first step.

use crate::util::flush::flush;
use crate::util::math::{DB_TO_LOG2, exp, exp2, log};

#[cfg(test)]
use crate::util::math::pow;

/// The shortest a segment may take, in milliseconds. A set time of zero (or anything shorter) lasts this long.
pub const MIN_RAMP_MS: f32 = 1.0;

/// The attack's overshoot ratio: a gently convex rise (the level at half the time is 0.68 of full, from any start).
pub const ATTACK_OVERSHOOT: f32 = 0.3;

/// The decay/release overshoot is 80 dB below the span at the longest: `r = 1e-4`.
pub const MAX_CURVE_DB: f32 = 80.0;

/// How many decibels of curve a decay/release gets per step beyond the first, until [`MAX_CURVE_DB`].
pub const CURVE_DB_PER_STEP: f32 = 8.0;

/// Time to control steps for one sample rate and one step length.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Timing {
    sample_rate: f64,
    step: f64,
    min_steps_f: f64,
    min_steps: u32,
}

impl Timing {
    /// For `sample_rate` Hz and control steps of `step` frames.
    #[must_use]
    pub fn new(sample_rate: f32, step: usize) -> Self {
        let mut t = Self {
            sample_rate: f64::from(sample_rate),
            step: step as f64,
            min_steps_f: 0.0,
            min_steps: 1,
        };
        t.min_steps_f = t.raw_steps(f64::from(MIN_RAMP_MS));
        t.min_steps = Self::to_ticks(t.min_steps_f);
        t
    }

    /// `ms · fs / (1000 · step)`. The product is exact in `f64` for `f32` inputs and the division is correctly
    /// rounded, so a time that is a whole number of steps (300 ms at 48 kHz is 450) comes out exact.
    fn raw_steps(&self, ms: f64) -> f64 {
        ms * self.sample_rate / (1000.0 * self.step)
    }

    fn to_ticks(steps: f64) -> u32 {
        let t = steps.ceil();
        if t >= 1.0e9 {
            1_000_000_000
        } else if t >= 1.0 {
            t as u32
        } else {
            1
        }
    }

    /// The length of a segment of `ms` milliseconds in control steps, fractional: `ms · fs / (1000 · step)`, at least
    /// the minimum ramp. NaN and negative times count as zero.
    #[must_use]
    pub fn steps(&self, ms: f32) -> f64 {
        let ms = if ms > 0.0 { f64::from(ms) } else { 0.0 };
        self.raw_steps(ms).max(self.min_steps_f).min(1.0e9)
    }

    /// The control steps a segment of `ms` milliseconds occupies: [`steps`](Self::steps) rounded up, so the segment
    /// ends on the first step at or after its set time. At least [`min_steps`](Self::min_steps).
    #[must_use]
    pub fn ticks(&self, ms: f32) -> u32 {
        Self::to_ticks(self.steps(ms))
    }

    /// The shortest segment in whole control steps: `ceil(1 ms · fs / (1000 · step))`, at least 1. Two at 48 kHz and
    /// 44.1 kHz, three at 96 kHz, one at and below 32 kHz.
    #[must_use]
    pub fn min_steps(&self) -> u32 {
        self.min_steps
    }
}

/// The overshoot ratio `r` of a decay or release of `steps` (fractional) control steps: `10^(−dB/20)` with
/// `dB = min(80, 8·(steps − 1))`, never below 0 dB. Long segments get the full 80 dB RC curve; a segment of two steps
/// gets an 8 dB curve (the level after the first step is 0.35 of where it started, where the full 80 dB curve would
/// leave 0.01), so the minimum ramp is a ramp and not a 40 dB drop in one step.
#[must_use]
pub fn overshoot(steps: f64) -> f32 {
    let db = (f64::from(CURVE_DB_PER_STEP) * (steps - 1.0).max(0.0)).min(f64::from(MAX_CURVE_DB));
    exp2(-db * DB_TO_LOG2) as f32
}

/// The per-step multiplier `c = (r/(1 + r))^(1/steps)` so that `c^steps = r/(1 + r)` and the curve crosses the end
/// at `steps` (fractional).
#[must_use]
pub fn coefficient(r: f32, steps: f64) -> f32 {
    let r = f64::from(r);
    exp(-log((1.0 + r) / r) / steps.max(f64::MIN_POSITIVE)) as f32
}

/// Where an envelope is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    /// Never triggered (a fresh voice before its first step).
    Idle,
    /// Rising to 1.
    Attack,
    /// Falling to the sustain level.
    Decay,
    /// Holding the sustain level while the key is down.
    Sustain,
    /// Falling to 0 after the key came up.
    Release,
    /// Released and silent: exactly 0.
    Off,
}

/// One envelope (one voice).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Envelope {
    stage: Stage,
    level: f32,
    start: f32,
    left: u32,
    c: f32,
    r: f32,
}

impl Default for Envelope {
    fn default() -> Self {
        Self::new()
    }
}

impl Envelope {
    /// A silent envelope that has not been triggered.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            stage: Stage::Idle,
            level: 0.0,
            start: 0.0,
            left: 0,
            c: 0.0,
            r: 0.0,
        }
    }

    /// The stage.
    #[must_use]
    pub fn stage(&self) -> Stage {
        self.stage
    }

    /// The level after the last step, 0–1.
    #[must_use]
    pub fn level(&self) -> f32 {
        self.level
    }

    /// Steps left in the running segment.
    #[must_use]
    pub fn steps_left(&self) -> u32 {
        self.left
    }

    /// Whether the key is considered down: attack, decay or sustain.
    #[must_use]
    pub fn is_gated(&self) -> bool {
        matches!(self.stage, Stage::Attack | Stage::Decay | Stage::Sustain)
    }

    /// Whether the envelope has finished its release (exactly 0 and silent for good).
    #[must_use]
    pub fn is_off(&self) -> bool {
        self.stage == Stage::Off
    }

    /// Whether the envelope is still in its attack (or has not started): a voice is not ended during it.
    #[must_use]
    pub fn in_attack(&self) -> bool {
        matches!(self.stage, Stage::Idle | Stage::Attack)
    }

    /// Note-on (or a retrigger): start the attack from the *current level*, so a retrigger is continuous. The attack
    /// takes `timing.steps(attack_ms)` steps from any start level.
    pub fn gate_on(&mut self, timing: &Timing, attack_ms: f32) {
        self.begin(Stage::Attack, self.level, 1.0, timing, attack_ms);
    }

    /// Note-off: start the release from the current level. Does nothing unless the key was down.
    pub fn gate_off(&mut self, timing: &Timing, release_ms: f32) {
        if self.is_gated() {
            self.begin(Stage::Release, self.level, 0.0, timing, release_ms);
        }
    }

    fn begin(&mut self, stage: Stage, from: f32, end: f32, timing: &Timing, ms: f32) {
        self.stage = stage;
        self.start = from;
        if end == from {
            // No span: finish on the next step instead of waiting out a segment that moves nothing.
            self.left = 1;
            self.r = 0.0;
            self.c = 0.0;
            return;
        }
        let steps = timing.steps(ms);
        self.left = Timing::to_ticks(steps);
        self.r = if stage == Stage::Attack {
            ATTACK_OVERSHOOT
        } else {
            overshoot(steps)
        };
        self.c = coefficient(self.r, steps);
    }

    /// One step of the running segment towards `end`; true when it ends (the level is then exactly `end`).
    fn advance(&mut self, end: f32) -> bool {
        self.left -= 1;
        if self.left == 0 {
            self.level = end;
            return true;
        }
        let target = end + (end - self.start) * self.r;
        self.level = target + (self.level - target) * self.c;
        false
    }

    /// Advance one control step and return the level (0–1). `sustain` is the sustain level now (0–1; it may move
    /// while held), `decay_ms` the decay time to use if the attack ends on this step.
    pub fn tick(&mut self, timing: &Timing, sustain: f32, decay_ms: f32) -> f32 {
        match self.stage {
            Stage::Idle | Stage::Off => {}
            Stage::Attack => {
                if self.advance(1.0) {
                    if sustain >= 1.0 {
                        self.stage = Stage::Sustain;
                    } else {
                        self.begin(Stage::Decay, 1.0, sustain, timing, decay_ms);
                    }
                } else {
                    self.level = self.level.min(1.0);
                }
            }
            Stage::Decay => {
                if self.advance(sustain) {
                    self.stage = Stage::Sustain;
                } else {
                    self.level = self.level.max(sustain);
                }
            }
            Stage::Sustain => self.level = sustain,
            Stage::Release => {
                if self.advance(0.0) {
                    self.stage = Stage::Off;
                } else {
                    self.level = self.level.max(0.0);
                }
            }
        }
        self.level = flush(self.level);
        self.level
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48_000.0;

    fn timing() -> Timing {
        Timing::new(SR, 32)
    }

    /// Run a fresh envelope from a given start level through one attack and return the level after each step.
    fn attack_from(start: f32, ms: f32) -> Vec<f32> {
        let t = timing();
        let mut e = Envelope::new();
        e.level = start;
        e.stage = Stage::Release; // anything but Off/Idle: gate_on starts from the level
        e.gate_on(&t, ms);
        let mut out = Vec::new();
        while e.stage() == Stage::Attack {
            out.push(e.tick(&t, 1.0, 0.0));
            assert!(out.len() < 1_000_000);
        }
        out
    }

    #[test]
    fn step_counts_round_up_and_respect_the_minimum() {
        let t = timing();
        assert_eq!(t.min_steps(), 2, "1 ms is 1.5 steps at 48 kHz");
        assert_eq!(t.steps(0.0), 1.5);
        assert_eq!(t.steps(-3.0), 1.5);
        assert_eq!(t.steps(f32::NAN), 1.5);
        assert_eq!(t.steps(0.5), 1.5);
        assert_eq!(t.steps(1.0), 1.5);
        assert_eq!(t.steps(5.0), 7.5);
        assert_eq!(t.steps(300.0), 450.0, "exact");
        assert_eq!(t.ticks(0.0), 2);
        assert_eq!(t.ticks(-3.0), 2);
        assert_eq!(t.ticks(f32::NAN), 2);
        assert_eq!(t.ticks(0.5), 2);
        assert_eq!(t.ticks(1.0), 2);
        assert_eq!(t.ticks(1.4), 3, "1.4 ms is 67.2 frames: 2.1 steps");
        assert_eq!(t.ticks(5.0), 8, "5 ms is 7.5 steps");
        assert_eq!(t.ticks(300.0), 450, "exact: not bumped to 451");
        assert_eq!(t.ticks(10_000.0), 15_000);
        assert_eq!(t.ticks(20_000.0), 30_000);
        assert_eq!(Timing::new(44_100.0, 32).min_steps(), 2);
        assert_eq!(Timing::new(96_000.0, 32).min_steps(), 3);
        assert_eq!(Timing::new(32_000.0, 32).min_steps(), 1, "exactly one step");
        assert_eq!(Timing::new(8_000.0, 32).min_steps(), 1);
        assert_eq!(Timing::new(384_000.0, 32).min_steps(), 12);
        assert_eq!(Timing::new(32_000.0, 32).ticks(1.0), 1);
        // The set time is never cut short: the ticks cover it, and the minimum is at least 1 ms (or one step).
        for sr in [
            8_000.0f32, 22_050.0, 44_100.0, 48_000.0, 88_200.0, 96_000.0, 192_000.0, 384_000.0,
        ] {
            let t = Timing::new(sr, 32);
            let ms = f64::from(t.min_steps()) * 32.0 * 1000.0 / f64::from(sr);
            assert!(ms >= 1.0 - 1e-9 || t.min_steps() == 1, "{sr}: {ms}");
            for ask in [0.0f32, 0.7, 1.0, 3.3, 77.7, 1234.5] {
                let covered = f64::from(t.ticks(ask)) * 32.0 * 1000.0 / f64::from(sr);
                assert!(
                    covered >= f64::from(ask.max(1.0)) - 1e-9 || t.ticks(ask) == 1,
                    "{sr} {ask}"
                );
            }
        }
    }

    #[test]
    fn a_segment_takes_exactly_its_steps_from_any_start_level() {
        for start in [0.0f32, 0.05, 0.18, 0.5, 0.9, 0.999] {
            for ms in [0.0f32, 5.0, 30.0, 300.0, 1_000.0] {
                let levels = attack_from(start, ms);
                let want = timing().ticks(ms) as usize;
                assert_eq!(levels.len(), want, "start {start}, {ms} ms");
                assert_eq!(*levels.last().unwrap(), 1.0, "ends exactly on 1.0");
                // Monotone and bounded.
                let mut last = start;
                for &l in &levels {
                    assert!(
                        l >= last && l <= 1.0,
                        "start {start} {ms} ms: {l} after {last}"
                    );
                    last = l;
                }
                // Never early, and at most one step late (the minimum aside).
                let took_ms = want as f64 * 32.0 / 48.0;
                let asked_ms = f64::from(ms).max(1.0);
                assert!(took_ms >= asked_ms - 1e-9, "{took_ms} < {asked_ms}");
                assert!(
                    took_ms < asked_ms + 2.0 * 32.0 / 48.0,
                    "{took_ms} vs {asked_ms}"
                );
            }
        }
    }

    #[test]
    fn the_curve_is_the_rc_curve_with_the_set_overshoot() {
        // Attack 0 → 1 in 300 ms (450 steps): level_n = 1.3 − 1.3·c^n with c^450 = 0.3/1.3.
        let t = timing();
        let mut e = Envelope::new();
        e.gate_on(&t, 300.0);
        let c = pow(0.3f64 / 1.3, 1.0 / 450.0);
        for n in 1..450 {
            let got = e.tick(&t, 1.0, 0.0);
            let want = (1.3 - 1.3 * pow(c, f64::from(n))) as f32;
            assert!((got - want).abs() < 2e-5, "{n}: {got} vs {want}");
        }
        assert_eq!(e.tick(&t, 1.0, 0.0), 1.0);
        assert_eq!(e.stage(), Stage::Sustain, "sustain 1.0 skips the decay");
    }

    #[test]
    fn decay_and_release_hit_their_ends_from_any_level() {
        let t = timing();
        for sustain in [0.0f32, 0.18, 0.5, 0.7, 0.99] {
            for decay_ms in [0.0f32, 3.0, 300.0, 2_000.0] {
                let mut e = Envelope::new();
                e.gate_on(&t, 0.0);
                let mut guard = 0;
                while e.stage() == Stage::Attack {
                    let _ = e.tick(&t, sustain, decay_ms);
                    guard += 1;
                }
                assert_eq!(guard, 2);
                assert_eq!(e.level(), 1.0);
                let want = t.ticks(decay_ms);
                assert_eq!(e.stage(), Stage::Decay);
                assert_eq!(e.steps_left(), want);
                let mut n = 0;
                let mut last = 1.0f32;
                while e.stage() == Stage::Decay {
                    let l = e.tick(&t, sustain, decay_ms);
                    assert!(l <= last && l >= sustain, "{sustain} {decay_ms}: {l}");
                    last = l;
                    n += 1;
                }
                assert_eq!(n, want, "sustain {sustain}, decay {decay_ms} ms");
                assert_eq!(
                    e.level().to_bits(),
                    sustain.to_bits(),
                    "lands on the sustain"
                );
                assert_eq!(e.stage(), Stage::Sustain);
            }
        }
        // Release from several levels, mid-attack, mid-decay and from sustain.
        for release_ms in [0.0f32, 1.0, 40.0, 400.0, 5_000.0] {
            for hold_steps in [0, 1, 3, 10, 700] {
                let mut e = Envelope::new();
                e.gate_on(&t, 20.0);
                for _ in 0..hold_steps {
                    let _ = e.tick(&t, 0.4, 100.0);
                }
                let from = e.level();
                e.gate_off(&t, release_ms);
                let want = t.ticks(release_ms);
                let mut n = 0u32;
                let mut last = from;
                while !e.is_off() {
                    let l = e.tick(&t, 0.4, 100.0);
                    assert!(l <= last && l >= 0.0);
                    last = l;
                    n += 1;
                    assert!(n <= want, "overran");
                }
                let expected = if from == 0.0 { 1 } else { want };
                assert_eq!(n, expected, "from {from}, release {release_ms} ms");
                assert_eq!(e.level().to_bits(), 0.0f32.to_bits());
            }
        }
    }

    #[test]
    fn the_minimum_ramp_is_a_ramp() {
        // A 0 ms attack, decay and release are about a millisecond, with an intermediate step, never a jump.
        let t = timing();
        let mut e = Envelope::new();
        e.gate_on(&t, 0.0);
        let a1 = e.tick(&t, 0.5, 0.0);
        assert!(a1 > 0.2 && a1 < 0.9, "attack step 1: {a1}");
        assert_eq!(e.tick(&t, 0.5, 0.0), 1.0);
        let d1 = e.tick(&t, 0.5, 0.0);
        assert!(d1 > 0.5 && d1 < 0.9, "decay step 1: {d1}");
        assert_eq!(e.tick(&t, 0.5, 0.0).to_bits(), 0.5f32.to_bits());
        e.gate_off(&t, 0.0);
        let r1 = e.tick(&t, 0.5, 0.0);
        // A shallow curve: the first step falls to about a quarter of the start, not by 40 dB.
        assert!(r1 > 0.05 && r1 < 0.35, "release step 1 from 0.5: {r1}");
        assert_eq!(e.tick(&t, 0.5, 0.0), 0.0);
        assert!(e.is_off());
    }

    #[test]
    fn a_retrigger_is_continuous_and_a_zero_span_segment_wastes_no_steps() {
        let t = timing();
        let mut e = Envelope::new();
        e.gate_on(&t, 10.0);
        for _ in 0..3 {
            let _ = e.tick(&t, 0.7, 300.0);
        }
        e.gate_off(&t, 400.0);
        for _ in 0..40 {
            let _ = e.tick(&t, 0.7, 300.0);
        }
        let before = e.level();
        assert!(before > 0.1 && before < 0.9);
        e.gate_on(&t, 10.0); // retrigger during the release
        let next = e.tick(&t, 0.7, 300.0);
        assert!(next > before, "rises from where it was: {before} -> {next}");
        assert!(next < before + 0.5);
        // At the peak a retrigger's attack has no span and finishes at once.
        let mut p = Envelope::new();
        p.gate_on(&t, 0.0);
        let _ = p.tick(&t, 1.0, 0.0);
        let _ = p.tick(&t, 1.0, 0.0);
        assert_eq!(p.level(), 1.0);
        p.gate_on(&t, 2_000.0);
        assert_eq!(p.steps_left(), 1);
        assert_eq!(p.tick(&t, 1.0, 0.0), 1.0);
        // Release from silence finishes on its first step.
        let mut s = Envelope::new();
        s.gate_on(&t, 0.0);
        s.stage = Stage::Sustain; // key down at level 0
        s.level = 0.0;
        s.gate_off(&t, 3_000.0);
        assert_eq!(s.steps_left(), 1);
        assert_eq!(s.tick(&t, 0.0, 0.0), 0.0);
        assert!(s.is_off());
    }

    #[test]
    fn sustain_follows_the_parameter_and_decay_survives_a_sustain_jump() {
        let t = timing();
        let mut e = Envelope::new();
        e.gate_on(&t, 0.0);
        for _ in 0..2 {
            let _ = e.tick(&t, 0.6, 500.0);
        }
        // Mid-decay the sustain jumps above the level: the level never goes down towards a stale target.
        for _ in 0..20 {
            let _ = e.tick(&t, 0.6, 500.0);
        }
        let mid = e.tick(&t, 0.6, 500.0);
        let jump = (mid + 0.04).min(0.99);
        let up = e.tick(&t, jump, 500.0);
        assert!(up >= mid && up >= jump - 1e-6, "{mid} -> {up}");
        // In sustain the level is the parameter, exactly.
        let mut s = Envelope::new();
        s.gate_on(&t, 0.0);
        for _ in 0..2 {
            let _ = s.tick(&t, 0.5, 0.0);
        }
        for _ in 0..2 {
            let _ = s.tick(&t, 0.5, 0.0);
        }
        assert_eq!(s.stage(), Stage::Sustain);
        assert_eq!(s.tick(&t, 0.25, 0.0).to_bits(), 0.25f32.to_bits());
    }

    #[test]
    fn the_release_passes_minus_100_db_before_it_ends() {
        let t = timing();
        for release_ms in [1.0f32, 50.0, 400.0, 20_000.0] {
            let mut e = Envelope::new();
            e.gate_on(&t, 0.0);
            for _ in 0..2 {
                let _ = e.tick(&t, 1.0, 0.0);
            }
            e.gate_off(&t, release_ms);
            let mut below = None;
            let mut steps = 0;
            while !e.is_off() {
                let l = e.tick(&t, 1.0, 0.0);
                steps += 1;
                if below.is_none() && l < 1e-5 {
                    below = Some(steps);
                }
            }
            let below = below.expect("crosses -100 dB");
            assert!(below <= steps, "{release_ms}");
        }
    }

    #[test]
    fn overshoot_and_coefficient_are_what_the_docs_say() {
        assert_eq!(overshoot(1.0), 1.0);
        assert_eq!(overshoot(0.25), 1.0, "never below 0 dB");
        assert!((overshoot(2.0) - 0.398).abs() < 1e-3);
        assert!((overshoot(1.5) - 0.631).abs() < 1e-3);
        assert!((overshoot(11.0) - 1e-4).abs() < 1e-8);
        assert_eq!(overshoot(500.0).to_bits(), overshoot(11.0).to_bits());
        // c^N = r / (1 + r), also for fractional N.
        for (r, n) in [
            (0.3f32, 8.0f64),
            (1e-4, 450.0),
            (0.398, 2.0),
            (0.631, 1.5),
            (0.3, 7.5),
        ] {
            let c = f64::from(coefficient(r, n));
            let got = pow(c, n);
            let want = f64::from(r) / (1.0 + f64::from(r));
            assert!((got / want - 1.0).abs() < 2e-3, "{r} {n}: {got} vs {want}");
        }
    }

    #[test]
    fn the_levels_vary_continuously_with_the_set_time() {
        // The seed-stability property: a 1e-3 relative nudge of a time moves the level at every step by little,
        // including across the step boundary where the tick count changes.
        let t = timing();
        let run = |release_ms: f32| {
            let mut e = Envelope::new();
            e.gate_on(&t, 0.0);
            for _ in 0..2 {
                let _ = e.tick(&t, 1.0, 0.0);
            }
            e.gate_off(&t, release_ms);
            let mut v = Vec::new();
            while !e.is_off() {
                v.push(e.tick(&t, 1.0, 0.0));
            }
            v.resize(700, 0.0);
            v
        };
        // 300 ms is exactly 450 steps; 300.3 ms is 450.45: the tick count changes from 450 to 451.
        let (a, b) = (run(300.0), run(300.3));
        assert_eq!(a.iter().position(|&x| x == 0.0), Some(449));
        assert_eq!(b.iter().position(|&x| x == 0.0), Some(450));
        let mut worst_db = 0.0f64;
        for (x, y) in a.iter().zip(&b) {
            if *x > 1e-3 {
                worst_db = worst_db.max((20.0 * crate::util::math::log10(f64::from(y / x))).abs());
            }
        }
        // 0.3 ms later on an 80 dB / 300 ms curve is about 0.08 dB, smoothly; a whole-step quantisation made 0.18.
        assert!(worst_db < 0.1, "{worst_db}");
        // The step into the end is tiny: the level before the final tick is already essentially the end.
        assert!(b[449] < 1e-5, "{}", b[449]);
    }

    #[test]
    fn the_state_never_goes_subnormal_or_nan() {
        let t = timing();
        let mut e = Envelope::new();
        e.gate_on(&t, 0.0);
        for _ in 0..3 {
            let _ = e.tick(&t, 0.3, 4.0);
        }
        e.gate_off(&t, 20_000.0);
        while !e.is_off() {
            let l = e.tick(&t, 0.3, 4.0);
            assert!(l.is_finite() && !l.is_subnormal());
        }
        for _ in 0..10 {
            assert_eq!(e.tick(&t, 0.3, 4.0).to_bits(), 0);
        }
    }
}
