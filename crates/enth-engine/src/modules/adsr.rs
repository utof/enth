//! The `adsr` module: RC-style exponential envelope, one per voice (P1a lane D2).
//! DSP core and the maths: `enth_dsp::envelope`; notes: `docs/dsp/envelope.md`.
//!
//! An ADSR is every patch's amplitude envelope (`voice.amp_env`) as well as a modulation source. Once per 32-frame
//! segment, per active voice:
//!
//! - a note-on landing on the voice (`NoteState::trigger`) starts the attack **from the current level**, so a retrigger
//!   is continuous; the first segment of a fresh voice starts from 0;
//! - a note-off (the gate fell) starts the release from the current level;
//! - the envelope steps once and writes `level · (1 − v + v·velocity)`, with `v = velocity% / 100`.
//!
//! Times are segment durations (an attack of 5 ms reaches 1 in 5 ms, rounded up to a whole 32-frame step), with a
//! **minimum of 1 ms per segment**, so a time of 0 is a ramp of about a millisecond (two steps at 48 kHz), never a jump.
//! [`min_ramp_steps`] is that count for a sample rate. A voice is finished when the envelope is past its attack and
//! its output is below −100 dB ([`VOICE_END_LEVEL`]), or has finished its release.

use enth_dsp::envelope::{Envelope, Stage, Timing};
use enth_dsp::util::math::VOICE_END_LEVEL;
use enth_patch::catalogue::AdsrParam;

use crate::module::{
    MAX_VOICES, PrepareCfg, SEG, VoiceIdx, VoiceModCx, VoiceModulator, VoiceStart,
};

/// The shortest segment in control steps at `sample_rate`: `ceil(1 ms · fs / 32 frames)`, 2 at 48 kHz. A gate
/// envelope (attack 0, decay 0, sustain 100 %, release 0) reaches exactly 1.0 after this many segments, and exactly
/// 0.0 this many segments after the note-off (`tests/contract.rs`).
#[must_use]
pub fn min_ramp_steps(sample_rate: f32) -> u32 {
    Timing::new(sample_rate, SEG).min_steps()
}

/// The envelope.
#[derive(Clone, Debug)]
pub struct Adsr {
    timing: Timing,
    env: [Envelope; MAX_VOICES],
    out: [f32; MAX_VOICES],
}

impl Adsr {
    /// Prepare an envelope.
    #[must_use]
    pub fn new(_values: &[f32], cfg: &PrepareCfg) -> Self {
        Self {
            timing: Timing::new(cfg.sample_rate, SEG),
            env: [Envelope::new(); MAX_VOICES],
            out: [0.0; MAX_VOICES],
        }
    }
}

impl VoiceModulator for Adsr {
    fn start_voice(&mut self, start: &VoiceStart<'_>) {
        let v = start.voice;
        self.env[v] = Envelope::new();
        self.out[v] = 0.0;
    }

    fn tick(&mut self, cx: &mut VoiceModCx<'_>) {
        let p = cx.params;
        for &v in cx.voices {
            let env = &mut self.env[v];
            if cx.notes.trigger[v] || (cx.notes.gate[v] && env.stage() == Stage::Idle) {
                env.gate_on(&self.timing, p.cur(AdsrParam::Attack, v));
            }
            if !cx.notes.gate[v] {
                env.gate_off(&self.timing, p.cur(AdsrParam::Release, v));
            }
            let sustain = p.cur(AdsrParam::Sustain, v) * 0.01;
            let level = env.tick(&self.timing, sustain, p.cur(AdsrParam::Decay, v));
            let amount = p.cur(AdsrParam::Velocity, v) * 0.01;
            let out = level * (1.0 - amount + amount * cx.notes.velocity[v]);
            self.out[v] = out;
            cx.out[v] = out;
        }
    }

    fn voice_finished(&self, v: VoiceIdx) -> bool {
        let env = &self.env[v];
        env.is_off() || (!env.in_attack() && self.out[v] < VOICE_END_LEVEL)
    }

    fn copy_state_from(&mut self, other: &Self) {
        self.env = other.env;
        self.out = other.out;
    }

    fn subnormals(&self) -> usize {
        let levels = self.env.iter().filter(|e| e.level().is_subnormal()).count();
        levels + self.out.iter().filter(|x| x.is_subnormal()).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::module::{NoteState, VoiceParams};
    use enth_patch::ModuleKind;

    const SR: f32 = 48_000.0;

    struct Rig {
        values: Vec<f32>,
        cur: Vec<[f32; MAX_VOICES]>,
        notes: NoteState,
        env: Adsr,
        out: [f32; MAX_VOICES],
        voice: usize,
    }

    impl Rig {
        fn new(sr: f32, set: &[(AdsrParam, f32)]) -> Self {
            let mut values = ModuleKind::Adsr.defaults();
            for &(p, v) in set {
                values[p.index()] = v;
            }
            let cur = values.iter().map(|&v| [v; MAX_VOICES]).collect();
            Self {
                env: Adsr::new(&values, &PrepareCfg::new(sr)),
                values,
                cur,
                notes: NoteState::new(),
                out: [0.0; MAX_VOICES],
                voice: 5,
            }
        }

        fn set(&mut self, p: AdsrParam, v: f32) {
            self.values[p.index()] = v;
            self.cur[p.index()] = [v; MAX_VOICES];
        }

        fn note_on(&mut self, velocity: f32) {
            let v = self.voice;
            self.notes.gate[v] = true;
            self.notes.trigger[v] = true;
            self.notes.fresh[v] = true;
            self.notes.velocity[v] = velocity;
            let params = VoiceParams::from_slices(&self.values, &self.cur, &self.cur);
            self.env.start_voice(&VoiceStart {
                voice: v,
                seed: 1,
                module_key: 2,
                note_index: 0,
                notes: &self.notes,
                params,
            });
        }

        fn note_off(&mut self) {
            self.notes.gate[self.voice] = false;
            self.notes.release[self.voice] = true;
        }

        fn tick(&mut self) -> f32 {
            let v = self.voice;
            let params = VoiceParams::from_slices(&self.values, &self.cur, &self.cur);
            self.env.tick(&mut VoiceModCx {
                tempo_bpm: crate::tempo::DEFAULT_TEMPO_BPM,
                voices: &[v],
                notes: &self.notes,
                params,
                out: &mut self.out,
            });
            self.notes.trigger[v] = false;
            self.notes.fresh[v] = false;
            self.notes.release[v] = false;
            self.out[v]
        }

        fn finished(&self) -> bool {
            self.env.voice_finished(self.voice)
        }
    }

    #[test]
    fn the_minimum_ramp_is_a_millisecond_rounded_up() {
        for (sr, steps) in [
            (8_000.0f32, 1u32),
            (32_000.0, 1),
            (44_100.0, 2),
            (48_000.0, 2),
            (96_000.0, 3),
            (384_000.0, 12),
        ] {
            assert_eq!(min_ramp_steps(sr), steps, "{sr}");
            let ms = f64::from(steps) * 32.0 * 1000.0 / f64::from(sr);
            assert!(ms >= 1.0 - 1e-9 || steps == 1, "{sr}: {ms} ms");
        }
    }

    #[test]
    fn a_gate_envelope_ramps_in_the_minimum_and_is_exact_after_it() {
        for sr in [8_000.0f32, 44_100.0, 48_000.0, 96_000.0, 384_000.0] {
            let n = min_ramp_steps(sr);
            let mut rig = Rig::new(
                sr,
                &[
                    (AdsrParam::Attack, 0.0),
                    (AdsrParam::Decay, 0.0),
                    (AdsrParam::Sustain, 100.0),
                    (AdsrParam::Release, 0.0),
                    (AdsrParam::Velocity, 0.0),
                ],
            );
            rig.note_on(0.3);
            let mut last = 0.0;
            for i in 1..=n {
                let y = rig.tick();
                if i < n {
                    assert!(y > last && y < 1.0, "{sr} Hz step {i}: {y}");
                } else {
                    assert_eq!(y.to_bits(), 1.0f32.to_bits(), "{sr} Hz step {i}");
                }
                last = y;
            }
            for _ in 0..50 {
                assert_eq!(rig.tick().to_bits(), 1.0f32.to_bits());
            }
            rig.note_off();
            for i in 1..=n {
                let y = rig.tick();
                if i < n {
                    assert!(y > 0.0 && y < last, "{sr} Hz release step {i}: {y}");
                    assert!(!rig.finished());
                } else {
                    assert_eq!(y.to_bits(), 0.0f32.to_bits());
                    assert!(rig.finished());
                }
                last = y;
            }
        }
    }

    #[test]
    fn the_default_envelope_hits_its_times_and_levels() {
        // Defaults: 5 ms / 300 ms / 70 % / 400 ms.
        let mut rig = Rig::new(SR, &[]);
        rig.note_on(1.0);
        let attack: Vec<f32> = (0..8).map(|_| rig.tick()).collect();
        assert!(attack[..7].iter().all(|&y| y < 1.0) && attack.windows(2).all(|w| w[1] > w[0]));
        assert_eq!(attack[7], 1.0, "5 ms is 7.5 steps: 8");
        let mut steps = 0;
        let mut y = 1.0;
        while y > 0.7 {
            y = rig.tick();
            steps += 1;
        }
        assert_eq!(steps, 450, "decay 300 ms is 450 steps");
        assert_eq!(y.to_bits(), 0.7f32.to_bits());
        for _ in 0..100 {
            assert_eq!(rig.tick().to_bits(), 0.7f32.to_bits());
        }
        rig.note_off();
        let mut steps = 0;
        while !rig.env.env[rig.voice].is_off() {
            let _ = rig.tick();
            steps += 1;
            assert!(steps <= 600);
        }
        assert_eq!(steps, 600, "release 400 ms is 600 steps");
        assert!(rig.finished());
    }

    #[test]
    fn a_voice_ends_below_minus_100_db_before_the_release_is_over() {
        let mut rig = Rig::new(SR, &[(AdsrParam::Release, 2_000.0)]);
        rig.note_on(1.0);
        for _ in 0..10 {
            let _ = rig.tick();
        }
        rig.note_off();
        let mut ended_at = None;
        let mut level_then = 0.0;
        for i in 0..3_000 {
            let y = rig.tick();
            if rig.finished() && ended_at.is_none() {
                ended_at = Some(i);
                level_then = y;
            }
            if ended_at.is_some() {
                assert!(y < VOICE_END_LEVEL);
            }
        }
        let ended_at = ended_at.expect("the voice ends");
        assert!(level_then < VOICE_END_LEVEL);
        assert!(ended_at <= 3_000, "a 2 s release is 3000 steps");
        assert!(ended_at > 2_500, "{ended_at}: the tail is not cut short");
    }

    #[test]
    fn a_voice_is_not_finished_during_its_attack_or_while_held() {
        let mut rig = Rig::new(
            SR,
            &[(AdsrParam::Attack, 500.0), (AdsrParam::Velocity, 100.0)],
        );
        rig.note_on(0.0); // velocity 0 at 100 % scaling: silent output, but the attack is not over
        for _ in 0..100 {
            let y = rig.tick();
            assert_eq!(y, 0.0);
            assert!(!rig.finished(), "still in the attack");
        }
        let mut held = Rig::new(SR, &[(AdsrParam::Sustain, 50.0)]);
        held.note_on(1.0);
        for _ in 0..2_000 {
            let _ = held.tick();
            assert!(!held.finished());
        }
        // A pluck (sustain 0) ends while the key is still down, as soon as it is below -100 dB.
        let mut pluck = Rig::new(SR, &[(AdsrParam::Sustain, 0.0), (AdsrParam::Decay, 50.0)]);
        pluck.note_on(1.0);
        let mut ended = false;
        for _ in 0..200 {
            let _ = pluck.tick();
            ended |= pluck.finished();
        }
        assert!(ended);
    }

    #[test]
    fn velocity_scales_the_output_by_the_amount() {
        for (amount, velocity, want) in [
            (0.0f32, 0.3f32, 1.0f32),
            (100.0, 0.3, 0.3),
            (50.0, 0.4, 0.7),
        ] {
            let mut rig = Rig::new(
                SR,
                &[
                    (AdsrParam::Attack, 0.0),
                    (AdsrParam::Sustain, 100.0),
                    (AdsrParam::Velocity, amount),
                ],
            );
            rig.note_on(velocity);
            let mut y = 0.0;
            for _ in 0..10 {
                y = rig.tick();
            }
            assert!((y - want).abs() < 1e-6, "{amount} {velocity}: {y}");
        }
    }

    #[test]
    fn times_are_read_when_a_segment_starts() {
        let mut rig = Rig::new(SR, &[(AdsrParam::Release, 100.0), (AdsrParam::Attack, 0.0)]);
        rig.note_on(1.0);
        for _ in 0..5 {
            let _ = rig.tick();
        }
        rig.note_off();
        let _ = rig.tick(); // step 1 of the 150-step release
        rig.set(AdsrParam::Release, 5_000.0); // too late for the running segment
        let mut steps = 1;
        while !rig.env.env[rig.voice].is_off() {
            let _ = rig.tick();
            steps += 1;
            assert!(steps <= 150, "the running segment is not stretched");
        }
        assert_eq!(steps, 150);
    }

    #[test]
    fn a_retrigger_starts_from_the_current_level() {
        let mut rig = Rig::new(
            SR,
            &[(AdsrParam::Attack, 20.0), (AdsrParam::Release, 400.0)],
        );
        rig.note_on(1.0);
        for _ in 0..4 {
            let _ = rig.tick();
        }
        rig.note_off();
        let mut before = 0.0;
        for _ in 0..30 {
            before = rig.tick();
        }
        // A new note-on lands on the same voice (a mono retrigger): trigger without fresh.
        rig.notes.gate[rig.voice] = true;
        rig.notes.trigger[rig.voice] = true;
        let after = rig.tick();
        assert!(
            after > before && after < before + 0.6,
            "{before} -> {after}"
        );
    }

    #[test]
    fn one_voice_is_independent_of_another() {
        let mut values = ModuleKind::Adsr.defaults();
        values[AdsrParam::Attack.index()] = 40.0;
        let cur: Vec<[f32; MAX_VOICES]> = values.iter().map(|&v| [v; MAX_VOICES]).collect();
        let params = VoiceParams::from_slices(&values, &cur, &cur);
        let mut both = Adsr::new(&values, &PrepareCfg::new(SR));
        let mut only = Adsr::new(&values, &PrepareCfg::new(SR));
        let mut notes = NoteState::new();
        for v in [2usize, 9] {
            notes.gate[v] = true;
            notes.trigger[v] = true;
            notes.velocity[v] = 1.0;
        }
        for (m, voices) in [(&mut both, vec![2usize, 9]), (&mut only, vec![9])] {
            for &v in &voices {
                m.start_voice(&VoiceStart {
                    voice: v,
                    seed: 0,
                    module_key: 0,
                    note_index: 0,
                    notes: &notes,
                    params,
                });
            }
        }
        let mut oa = [0.0; MAX_VOICES];
        let mut ob = [0.0; MAX_VOICES];
        for _ in 0..20 {
            both.tick(&mut VoiceModCx {
                tempo_bpm: crate::tempo::DEFAULT_TEMPO_BPM,
                voices: &[2, 9],
                notes: &notes,
                params,
                out: &mut oa,
            });
            only.tick(&mut VoiceModCx {
                tempo_bpm: crate::tempo::DEFAULT_TEMPO_BPM,
                voices: &[9],
                notes: &notes,
                params,
                out: &mut ob,
            });
            assert_eq!(oa[9].to_bits(), ob[9].to_bits());
            notes.trigger = [false; MAX_VOICES];
        }
    }

    #[test]
    fn copy_state_continues_identically() {
        let mut a = Rig::new(SR, &[]);
        a.note_on(0.8);
        for _ in 0..100 {
            let _ = a.tick();
        }
        let mut b = Rig::new(SR, &[]);
        b.env.copy_state_from(&a.env);
        b.notes = a.notes.clone();
        for _ in 0..200 {
            assert_eq!(a.tick().to_bits(), b.tick().to_bits());
        }
        a.note_off();
        b.note_off();
        for _ in 0..200 {
            assert_eq!(a.tick().to_bits(), b.tick().to_bits());
        }
        assert_eq!(a.env.subnormals(), 0);
    }
}
