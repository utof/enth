//! The `lfo` module: a low-frequency oscillator, per voice or global (P1a lane D2).
//! DSP core: `enth_dsp::lfo`; notes: `docs/dsp/lfo.md`.
//!
//! One type serves both roles: a voice-scope LFO is a `VoiceModulator` (started at each note-on), a global-scope one
//! a `GlobalModulator` (started at engine start); `ModuleImpl::new` picks the role from the structural `scope`
//! parameter. Once per 32-frame segment each LFO advances its 64-bit phase by one increment (`rate` read from the
//! segment's value, so a modulated rate changes the speed without a jump in the phase) and writes the value of its
//! shape at the *end* of the segment: −1..1 for `bipolar`, 0..1 for `unipolar`.
//!
//! Start and randomness, by `phase_mode`:
//!
//! | mode | start phase | random stream (sample & hold, smooth random) |
//! | --- | --- | --- |
//! | `reset` | the `phase` knob | `VoiceStart::seed` / `GlobalStart::seed` (render seed, module id, note index) |
//! | `free` | a draw from that same stream | that stream, after the start-phase draw |
//! | `seeded` | a draw from the module's own `seed` parameter (`seed::seeded_voice_seed` / `seeded_global_seed`) | that stream |
//!
//! Nothing here depends on the patch hash: moving any knob never reseeds the random shapes.
//!
//! **Tempo sync and one-shot (S2a, lane TMP).** With `sync` on, the speed comes from `beats` (one cycle as a note
//! division, [`Beats::length`]) and the engine's tempo (`cx.tempo_bpm`, read at each segment), and `rate` is
//! ignored; the core's phase is exact in beats and continuous across a tempo change. `beats` is read per voice at its
//! current (modulated) value: a modulated enum steps through its items in order, so the speed jumps from one division
//! to the next and never passes through a length between them. With `one_shot` on, the LFO runs one cycle from its
//! start phase (the note's, or the render start for a global LFO) and then holds the value it reached. Both are read
//! live at every segment; with `sync` and `one_shot` off the module is what it was.

use enth_dsp::lfo::{self, Lfo as Core, Shape, StartPhase};
use enth_patch::catalogue::{Beats, LfoParam, LfoShape, PhaseMode, Polarity};

use crate::module::{
    GlobalModCx, GlobalModulator, GlobalParams, GlobalStart, MAX_VOICES, PrepareCfg, SEG, VoiceIdx,
    VoiceModCx, VoiceModulator, VoiceParams, VoiceStart,
};
use crate::seed::{seeded_global_seed, seeded_voice_seed};

/// The LFO.
#[derive(Clone, Debug)]
pub struct Lfo {
    sample_rate: f32,
    /// One per voice slot (voice scope).
    voices: [Core; MAX_VOICES],
    /// The one global LFO (global scope).
    global: Core,
}

impl Lfo {
    /// Prepare an LFO.
    #[must_use]
    pub fn new(_values: &[f32], cfg: &PrepareCfg) -> Self {
        let idle = Core::new(0, StartPhase::Cycles(0.0));
        Self {
            sample_rate: cfg.sample_rate,
            voices: [idle; MAX_VOICES],
            global: idle,
        }
    }
}

fn shape_of(value: f32) -> Shape {
    match LfoShape::from_value(value) {
        Some(LfoShape::Triangle) => Shape::Triangle,
        Some(LfoShape::SawUp) => Shape::SawUp,
        Some(LfoShape::SawDown) => Shape::SawDown,
        Some(LfoShape::Square) => Shape::Square,
        Some(LfoShape::SampleHold) => Shape::SampleHold,
        Some(LfoShape::SmoothRandom) => Shape::SmoothRandom,
        Some(LfoShape::Sine) | None => Shape::Sine,
    }
}

/// One cycle in beats for a stored `beats` value (a quarter note for a value that is no item, which cannot happen).
fn cycle_beats(value: f32) -> (u32, u32) {
    Beats::from_value(value).unwrap_or(Beats::Quarter).length()
}

fn is_unipolar(value: f32) -> bool {
    Polarity::from_value(value) == Some(Polarity::Unipolar)
}

fn output(y: f32, unipolar: bool) -> f32 {
    if unipolar { lfo::unipolar(y) } else { y }
}

/// The start phase in cycles for the `reset` mode: the knob in degrees over 360.
fn knob_cycles(degrees: f32) -> f64 {
    f64::from(degrees) / 360.0
}

/// A core for one note of a voice-scope LFO.
fn voice_core(start: &VoiceStart<'_>) -> Core {
    let p = start.params;
    match PhaseMode::from_value(p.base(LfoParam::PhaseMode)) {
        Some(PhaseMode::Reset) => Core::new(
            start.seed,
            StartPhase::Cycles(knob_cycles(p.base(LfoParam::Phase))),
        ),
        Some(PhaseMode::Seeded) => {
            let seed = p.base(LfoParam::Seed) as u32;
            Core::new(
                seeded_voice_seed(seed, start.module_key, start.note_index),
                StartPhase::Random,
            )
        }
        Some(PhaseMode::Free) | None => Core::new(start.seed, StartPhase::Random),
    }
}

/// The core of the global-scope LFO.
fn global_core(start: &GlobalStart<'_>) -> Core {
    let p = start.params;
    match PhaseMode::from_value(p.base(LfoParam::PhaseMode)) {
        Some(PhaseMode::Reset) => Core::new(
            start.seed,
            StartPhase::Cycles(knob_cycles(p.base(LfoParam::Phase))),
        ),
        Some(PhaseMode::Seeded) => {
            let seed = p.base(LfoParam::Seed) as u32;
            Core::new(
                seeded_global_seed(seed, start.module_key),
                StartPhase::Random,
            )
        }
        Some(PhaseMode::Free) | None => Core::new(start.seed, StartPhase::Random),
    }
}

impl VoiceModulator for Lfo {
    fn start_voice(&mut self, start: &VoiceStart<'_>) {
        self.voices[start.voice] = voice_core(start);
    }

    fn tick(&mut self, cx: &mut VoiceModCx<'_>) {
        let p: VoiceParams<'_> = cx.params;
        let shape = shape_of(p.base(LfoParam::Shape));
        let unipolar = is_unipolar(p.base(LfoParam::Polarity));
        let synced = p.base(LfoParam::Sync) != 0.0;
        let one_shot = p.base(LfoParam::OneShot) != 0.0;
        for &v in cx.voices {
            let core = &mut self.voices[v];
            core.set_one_shot(one_shot);
            // A mono retrigger (a note-on on a voice that is still held or releasing) is a `trigger` without a fresh
            // voice: restart the cycle there, as an envelope restarts.
            if one_shot && cx.notes.trigger[v] && !cx.notes.fresh[v] {
                core.restart_one_shot();
            }
            if synced {
                let beats = cycle_beats(p.cur(LfoParam::Beats, v));
                core.set_synced(cx.tempo_bpm, beats, self.sample_rate, SEG);
            } else {
                core.set_rate(p.cur(LfoParam::Rate, v), self.sample_rate, SEG);
            }
            cx.out[v] = output(core.tick(shape), unipolar);
        }
    }

    fn voice_finished(&self, _v: VoiceIdx) -> bool {
        false
    }

    fn copy_state_from(&mut self, other: &Self) {
        self.voices = other.voices;
        self.global = other.global;
    }

    fn subnormals(&self) -> usize {
        self.voices.iter().filter(|c| c.has_subnormal()).count()
            + usize::from(self.global.has_subnormal())
    }
}

impl GlobalModulator for Lfo {
    fn reset(&mut self, start: &GlobalStart<'_>) {
        self.global = global_core(start);
    }

    fn tick(&mut self, cx: &mut GlobalModCx<'_>) -> f32 {
        let p: GlobalParams<'_> = cx.params;
        let shape = shape_of(p.base(LfoParam::Shape));
        let unipolar = is_unipolar(p.base(LfoParam::Polarity));
        self.global.set_one_shot(p.base(LfoParam::OneShot) != 0.0);
        if p.base(LfoParam::Sync) == 0.0 {
            self.global
                .set_rate(p.cur(LfoParam::Rate), self.sample_rate, SEG);
        } else {
            let beats = cycle_beats(p.cur(LfoParam::Beats));
            self.global
                .set_synced(cx.tempo_bpm, beats, self.sample_rate, SEG);
        }
        output(self.global.tick(shape), unipolar)
    }

    fn copy_state_from(&mut self, other: &Self) {
        self.voices = other.voices;
        self.global = other.global;
    }

    fn subnormals(&self) -> usize {
        VoiceModulator::subnormals(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::module::NoteState;
    use enth_patch::ModuleKind;
    use enth_patch::catalogue::LfoScope;

    const SR: f32 = 48_000.0;

    struct Rig {
        values: Vec<f32>,
        cur: Vec<[f32; MAX_VOICES]>,
        gcur: Vec<f32>,
        notes: NoteState,
        lfo: Lfo,
        tempo: f64,
    }

    impl Rig {
        fn new(set: &[(LfoParam, f32)]) -> Self {
            let mut values = ModuleKind::Lfo.defaults();
            for &(p, v) in set {
                values[p.index()] = v;
            }
            Self {
                cur: values.iter().map(|&v| [v; MAX_VOICES]).collect(),
                gcur: values.clone(),
                lfo: Lfo::new(&values, &PrepareCfg::new(SR)),
                values,
                notes: NoteState::new(),
                tempo: crate::tempo::DEFAULT_TEMPO_BPM,
            }
        }

        fn start_voice(&mut self, voice: usize, seed: u64, note_index: u64) {
            let params = VoiceParams::from_slices(&self.values, &self.cur, &self.cur);
            self.lfo.start_voice(&VoiceStart {
                voice,
                seed,
                module_key: 0xABCD,
                note_index,
                notes: &self.notes,
                params,
            });
        }

        fn tick_voice(&mut self, voice: usize) -> f32 {
            let params = VoiceParams::from_slices(&self.values, &self.cur, &self.cur);
            let mut out = [0.0; MAX_VOICES];
            VoiceModulator::tick(
                &mut self.lfo,
                &mut VoiceModCx {
                    voices: &[voice],
                    notes: &self.notes,
                    params,
                    tempo_bpm: self.tempo,
                    out: &mut out,
                },
            );
            out[voice]
        }

        fn reset_global(&mut self, seed: u64) {
            let params = GlobalParams::from_slices(&self.values, &self.gcur, &self.gcur);
            self.lfo.reset(&GlobalStart {
                seed,
                module_key: 0xABCD,
                params,
            });
        }

        fn tick_global(&mut self) -> f32 {
            let params = GlobalParams::from_slices(&self.values, &self.gcur, &self.gcur);
            GlobalModulator::tick(
                &mut self.lfo,
                &mut GlobalModCx {
                    params,
                    tempo_bpm: self.tempo,
                },
            )
        }
    }

    #[test]
    fn a_sine_at_50_hz_has_thirty_segments_per_cycle() {
        let mut rig = Rig::new(&[(LfoParam::Rate, 50.0)]);
        // 50 Hz: 1500 segments per second, 30 per cycle. The first value is at x = 1/30.
        rig.start_voice(0, 1, 0);
        let first = rig.tick_voice(0);
        assert!(
            (first - libm::sinf(core::f32::consts::TAU / 30.0)).abs() < 1e-5,
            "{first}"
        );
        let mut crossings = 0;
        let mut last = first;
        for _ in 0..1500 {
            let y = rig.tick_voice(0);
            if last < 0.0 && y >= 0.0 {
                crossings += 1;
            }
            last = y;
        }
        assert!((49..=51).contains(&crossings), "{crossings}");
    }

    #[test]
    fn rate_error_over_60_seconds_through_the_module() {
        // The rising zero crossings of a sine, located by linear interpolation, are 1/rate apart.
        for rate in [0.37f32, 5.0, 41.0] {
            let mut rig = Rig::new(&[(LfoParam::Rate, rate), (LfoParam::Phase, 10.0)]);
            rig.start_voice(0, 1, 0);
            let (mut first, mut last_t, mut n) = (None, 0.0f64, 0u32);
            let mut prev = rig.tick_voice(0);
            let segments = 60 * 1500;
            for s in 1..=segments {
                let y = rig.tick_voice(0);
                if prev < 0.0 && y >= 0.0 {
                    let frac = f64::from(-prev) / f64::from(y - prev);
                    let t = (f64::from(s) - 1.0 + frac) * 32.0 / 48_000.0;
                    if first.is_none() {
                        first = Some(t);
                    }
                    last_t = t;
                    n += 1;
                }
                prev = y;
            }
            let period = (last_t - first.unwrap()) / f64::from(n - 1);
            let err = (period * f64::from(rate) - 1.0).abs();
            assert!(err < 1e-5, "{rate} Hz: period {period} ({err:e})");
        }
    }

    #[test]
    fn polarity_maps_to_zero_one() {
        let mut bi = Rig::new(&[
            (LfoParam::Shape, LfoShape::SawUp.index() as f32),
            (LfoParam::Rate, 10.0),
        ]);
        let mut uni = Rig::new(&[
            (LfoParam::Shape, LfoShape::SawUp.index() as f32),
            (LfoParam::Rate, 10.0),
            (LfoParam::Polarity, Polarity::Unipolar.index() as f32),
        ]);
        bi.start_voice(0, 1, 0);
        uni.start_voice(0, 1, 0);
        for _ in 0..300 {
            let (b, u) = (bi.tick_voice(0), uni.tick_voice(0));
            assert!((-1.0..=1.0).contains(&b));
            assert!((0.0..=1.0).contains(&u));
            assert!((u - (0.5 + 0.5 * b)).abs() < 1e-7);
        }
    }

    #[test]
    fn phase_modes_pick_the_start_as_documented() {
        let value_at_start =
            |mode: PhaseMode, knob: f32, param_seed: f32, render: u64, note: u64| {
                let mut rig = Rig::new(&[
                    (LfoParam::Shape, LfoShape::SawUp.index() as f32),
                    (LfoParam::Rate, 0.01),
                    (LfoParam::PhaseMode, mode.index() as f32),
                    (LfoParam::Phase, knob),
                    (LfoParam::Seed, param_seed),
                ]);
                rig.start_voice(0, render, note);
                rig.tick_voice(0)
            };
        // reset: the knob. 90 degrees on a rising saw is x = 0.25 -> -0.5, whatever the seeds.
        for render in [1u64, 2, 3] {
            let y = value_at_start(PhaseMode::Reset, 90.0, 7.0, render, 4);
            assert!((y + 0.5).abs() < 1e-3, "{y}");
        }
        // free: depends on the render seed and the note, not on the knob or the module seed.
        let a = value_at_start(PhaseMode::Free, 0.0, 0.0, 11, 0);
        assert_eq!(a, value_at_start(PhaseMode::Free, 200.0, 99.0, 11, 0));
        assert_ne!(a, value_at_start(PhaseMode::Free, 0.0, 0.0, 12, 0));
        // seeded: depends on the module seed (and the note index), not on the render seed.
        let s = value_at_start(PhaseMode::Seeded, 0.0, 5.0, 11, 3);
        assert_eq!(s, value_at_start(PhaseMode::Seeded, 0.0, 5.0, 12, 3));
        assert_ne!(s, value_at_start(PhaseMode::Seeded, 0.0, 6.0, 11, 3));
        assert_ne!(s, value_at_start(PhaseMode::Seeded, 0.0, 5.0, 11, 4));
    }

    #[test]
    fn sample_and_hold_depends_on_the_seed_and_not_on_the_other_parameters() {
        let run = |render: u64, phase_knob: f32| {
            let mut rig = Rig::new(&[
                (LfoParam::Shape, LfoShape::SampleHold.index() as f32),
                (LfoParam::Rate, 20.0),
                (LfoParam::Phase, phase_knob),
            ]);
            rig.start_voice(3, render, 2);
            (0..600).map(|_| rig.tick_voice(3)).collect::<Vec<_>>()
        };
        let a = run(100, 0.0);
        assert_eq!(a, run(100, 0.0));
        assert_ne!(a, run(101, 0.0), "the render seed moves the draws");
        // The phase knob moves when the steps happen, but the drawn values are the same stream.
        let shifted = run(100, 180.0);
        assert_eq!(a[0], shifted[0]);
        let values = |x: &[f32]| {
            let mut v: Vec<f32> = Vec::new();
            for &y in x {
                if v.last() != Some(&y) {
                    v.push(y);
                }
            }
            v
        };
        let (va, vs) = (values(&a), values(&shifted));
        assert_eq!(va[..6], vs[..6], "the same sequence of held values");
        // Steps are held: values repeat between wraps and change at them.
        let changes = a.windows(2).filter(|w| w[0] != w[1]).count();
        assert!(
            (7..=9).contains(&changes),
            "20 Hz over 0.4 s is 8 cycles: {changes}"
        );
    }

    #[test]
    fn voices_are_independent_and_note_index_gives_each_its_own_stream() {
        let mut rig = Rig::new(&[
            (LfoParam::Shape, LfoShape::SmoothRandom.index() as f32),
            (LfoParam::Rate, 8.0),
        ]);
        rig.start_voice(0, 5, 0);
        rig.start_voice(1, 6, 1);
        let a: Vec<f32> = (0..200).map(|_| rig.tick_voice(0)).collect();
        let b: Vec<f32> = (0..200).map(|_| rig.tick_voice(1)).collect();
        assert_ne!(a, b);
        // Voice 0 alone gives the same stream.
        let mut solo = Rig::new(&[
            (LfoParam::Shape, LfoShape::SmoothRandom.index() as f32),
            (LfoParam::Rate, 8.0),
        ]);
        solo.start_voice(0, 5, 0);
        let c: Vec<f32> = (0..200).map(|_| solo.tick_voice(0)).collect();
        assert_eq!(a, c);
    }

    #[test]
    fn the_global_lfo_runs_from_reset_and_uses_the_global_seed() {
        let mut rig = Rig::new(&[
            (LfoParam::Scope, LfoScope::Global.index() as f32),
            (LfoParam::Shape, LfoShape::SampleHold.index() as f32),
            (LfoParam::Rate, 12.0),
        ]);
        rig.reset_global(77);
        let a: Vec<f32> = (0..400).map(|_| rig.tick_global()).collect();
        rig.reset_global(77);
        let b: Vec<f32> = (0..400).map(|_| rig.tick_global()).collect();
        assert_eq!(a, b, "reset restarts the stream");
        rig.reset_global(78);
        let c: Vec<f32> = (0..400).map(|_| rig.tick_global()).collect();
        assert_ne!(a, c);
        // Seeded mode ignores the render seed.
        let mut seeded = Rig::new(&[
            (LfoParam::Shape, LfoShape::SampleHold.index() as f32),
            (LfoParam::Rate, 12.0),
            (LfoParam::PhaseMode, PhaseMode::Seeded.index() as f32),
            (LfoParam::Seed, 9.0),
        ]);
        seeded.reset_global(1);
        let s1: Vec<f32> = (0..400).map(|_| seeded.tick_global()).collect();
        seeded.reset_global(2);
        let s2: Vec<f32> = (0..400).map(|_| seeded.tick_global()).collect();
        assert_eq!(s1, s2);
    }

    #[test]
    fn a_rate_change_keeps_the_phase_continuous() {
        let mut rig = Rig::new(&[(LfoParam::Rate, 2.0)]);
        rig.start_voice(0, 1, 0);
        for _ in 0..100 {
            let _ = rig.tick_voice(0);
        }
        let before = rig.lfo.voices[0].phase();
        rig.values[LfoParam::Rate.index()] = 20.0;
        rig.cur[LfoParam::Rate.index()] = [20.0; MAX_VOICES];
        let _ = rig.tick_voice(0);
        let step = rig.lfo.voices[0].phase().wrapping_sub(before);
        assert_eq!(step, enth_dsp::lfo::phase_increment(20.0, SR, SEG));
    }

    #[test]
    fn copy_state_continues_identically_and_nothing_is_subnormal() {
        let mut a = Rig::new(&[
            (LfoParam::Shape, LfoShape::SmoothRandom.index() as f32),
            (LfoParam::Rate, 9.0),
        ]);
        a.start_voice(4, 3, 0);
        for _ in 0..77 {
            let _ = a.tick_voice(4);
        }
        let mut b = Rig::new(&[
            (LfoParam::Shape, LfoShape::SmoothRandom.index() as f32),
            (LfoParam::Rate, 9.0),
        ]);
        VoiceModulator::copy_state_from(&mut b.lfo, &a.lfo);
        for _ in 0..300 {
            assert_eq!(a.tick_voice(4).to_bits(), b.tick_voice(4).to_bits());
        }
        assert_eq!(VoiceModulator::subnormals(&a.lfo), 0);
        assert!(!a.lfo.voice_finished(4));
    }
}
