//! The `comb` module: a keytracked feedback comb filter per voice (S2a lane FLT). DSP core and equations:
//! `enth_dsp::comb`; notes: `docs/dsp/comb.md`.
//!
//! Per voice and segment:
//!
//! 1. The comb's frequency is `max(20 Hz, 261.63 Hz · 2^((semi + keytrack · (pitch − 60)) / 12))`, clamped to 0.45·fs
//!    (`pitch` is `NoteState::pitch_cur`, so a glide moves the teeth), so at `keytrack` 100 % and `semi` 0 the teeth sit
//!    on the note's harmonics. The frequency and `damping` give the loop's delay (one period less the low-pass's phase
//!    delay) and the low-pass coefficient ([`enth_dsp::comb::Tuning`]): one `exp2`, one `tan` and one `atan`, cached
//!    per voice and recomputed only when `semi`, the key-tracking term or `damping` changed, so a static comb costs no
//!    transcendental.
//! 2. [`enth_dsp::comb::process_block`] runs the 32 frames, ramping the delay, the low-pass coefficient, `feedback`
//!    and `mix` linearly per sample from the end of the previous segment to the end of this one.
//!
//! Each voice has its own delay line (`fs/20` samples rounded up to a power of two: 4,096 floats at 48 kHz, 128 KB for
//! all 32 voices), zeroed by `start_voice`. The plan swap copies only the lines of voices that have started, which
//! keeps the copy to the voices that sound.
//!
//! Parameters are read through `.index()` (a `usize` is a [`ParamIndex`](crate::params::ParamIndex)), so the shared
//! `params.rs` list stays as it is.

use enth_dsp::comb::{self, Segment, State, Tuning};
use enth_patch::catalogue::CombParam;

use crate::module::{MAX_VOICES, PrepareCfg, VoiceCx, VoiceProcessor, VoiceStart};

/// The inputs a voice's tuning was computed from: `semi`, the key-tracking term in semitones and `damping`. Equal
/// inputs give equal tunings, so the cache is exact.
type Key = [f32; 3];

/// The keytracked comb filter.
#[derive(Clone, Debug)]
pub struct Comb {
    sample_rate: f32,
    /// One delay line per voice slot, `line_len` floats each, back to back.
    lines: Vec<f32>,
    line_len: usize,
    state: [State; MAX_VOICES],
    /// The tuning at the end of the previous segment.
    tuning: [Tuning; MAX_VOICES],
    key: [Key; MAX_VOICES],
    /// The voice has run a segment since `start_voice`: `tuning` and `key` are valid.
    valid: [bool; MAX_VOICES],
    /// The voice's line has been started at least once, so it may hold signal (and a plan swap must copy it).
    used: [bool; MAX_VOICES],
}

impl Comb {
    /// Prepare a comb: allocates the delay lines, sized from the sample rate only.
    #[must_use]
    pub fn new(_values: &[f32], cfg: &PrepareCfg) -> Self {
        let line_len = comb::line_len(cfg.sample_rate);
        let idle = Tuning::new(comb::MIDDLE_C_HZ, 10_000.0, cfg.sample_rate);
        Self {
            sample_rate: cfg.sample_rate,
            lines: vec![0.0; MAX_VOICES * line_len],
            line_len,
            state: [State::ZERO; MAX_VOICES],
            tuning: [idle; MAX_VOICES],
            key: [[0.0; 3]; MAX_VOICES],
            valid: [false; MAX_VOICES],
            used: [false; MAX_VOICES],
        }
    }

    fn line(&mut self, v: usize) -> &mut [f32] {
        &mut self.lines[v * self.line_len..(v + 1) * self.line_len]
    }
}

impl VoiceProcessor for Comb {
    fn start_voice(&mut self, start: &VoiceStart<'_>) {
        let v = start.voice;
        self.line(v).fill(0.0);
        self.state[v] = State::ZERO;
        self.valid[v] = false;
        self.used[v] = true;
    }

    fn process(&mut self, cx: &mut VoiceCx<'_>) {
        let p = cx.params;
        let (semi_i, key_i, fb_i, damp_i, mix_i) = (
            CombParam::Semi.index(),
            CombParam::Keytrack.index(),
            CombParam::Feedback.index(),
            CombParam::Damping.index(),
            CombParam::Mix.index(),
        );
        for &v in cx.voices {
            let semi = p.cur(semi_i, v);
            let damping = p.cur(damp_i, v);
            let keytrack = p.cur(key_i, v);
            // Semitones of key tracking: 100 % follows the note one for one, relative to middle C.
            let tracked = if keytrack == 0.0 {
                0.0
            } else {
                (cx.notes.pitch_cur[v] - 60.0) * keytrack * 0.01
            };
            let key: Key = [semi, tracked, damping];
            let first = !self.valid[v];
            let to = if first || key != self.key[v] {
                self.key[v] = key;
                Tuning::new(
                    comb::frequency_hz(semi, tracked, self.sample_rate),
                    damping,
                    self.sample_rate,
                )
            } else {
                self.tuning[v]
            };
            let from = if first { to } else { self.tuning[v] };
            let seg = Segment {
                from,
                to,
                feedback: [p.prev(fb_i, v) * 0.01, p.cur(fb_i, v) * 0.01],
                mix: [p.prev(mix_i, v) * 0.01, p.cur(mix_i, v) * 0.01],
            };
            let line = &mut self.lines[v * self.line_len..(v + 1) * self.line_len];
            comb::process_block(line, &mut self.state[v], &seg, &mut cx.io[v]);
            self.tuning[v] = to;
            self.valid[v] = true;
        }
    }

    fn copy_state_from(&mut self, other: &Self) {
        self.state = other.state;
        self.tuning = other.tuning;
        self.key = other.key;
        self.valid = other.valid;
        self.used = other.used;
        // A voice that never started has a zero line in both; a voice is read only after `start_voice` zeroes it.
        if self.line_len == other.line_len {
            for v in 0..MAX_VOICES {
                if other.used[v] {
                    let range = v * self.line_len..(v + 1) * self.line_len;
                    self.lines[range.clone()].copy_from_slice(&other.lines[range]);
                }
            }
        }
    }

    fn subnormals(&self) -> usize {
        (0..MAX_VOICES)
            .filter(|&v| {
                self.state[v].has_subnormal()
                    || self.lines[v * self.line_len..(v + 1) * self.line_len]
                        .iter()
                        .any(|x| x.is_subnormal())
            })
            .count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::module::{NoteState, SEG, VoiceBus, VoiceParams};
    use enth_dsp::alias::fft;
    use enth_dsp::util::math::{exp2f, log2f, sinf};
    use enth_dsp::util::rng::Rng;
    use enth_patch::ModuleKind;

    const SR: f32 = 48_000.0;

    /// Parameter tables for one voice with the given values, `cur == prev == base`.
    struct Rig {
        values: Vec<f32>,
        cur: Vec<[f32; MAX_VOICES]>,
        prev: Vec<[f32; MAX_VOICES]>,
        notes: NoteState,
        io: Box<VoiceBus>,
        comb: Comb,
    }

    impl Rig {
        fn new(set: &[(CombParam, f32)]) -> Self {
            let mut values = ModuleKind::Comb.defaults();
            for &(p, v) in set {
                values[p.index()] = v;
            }
            let cur: Vec<[f32; MAX_VOICES]> = values.iter().map(|&v| [v; MAX_VOICES]).collect();
            let mut notes = NoteState::new();
            notes.gate[0] = true;
            notes.pitch_cur[0] = 60.0;
            notes.pitch_prev[0] = 60.0;
            let comb = Comb::new(&values, &PrepareCfg::new(SR));
            Self {
                prev: cur.clone(),
                values,
                cur,
                notes,
                io: Box::new([[0.0; SEG]; MAX_VOICES]),
                comb,
            }
        }

        fn set(&mut self, p: CombParam, v: f32) {
            self.values[p.index()] = v;
            self.prev[p.index()] = self.cur[p.index()];
            self.cur[p.index()] = [v; MAX_VOICES];
        }

        fn settle(&mut self) {
            self.prev = self.cur.clone();
        }

        fn start(&mut self) {
            let params = VoiceParams::from_slices(&self.values, &self.cur, &self.cur);
            self.comb.start_voice(&VoiceStart {
                voice: 0,
                seed: 0,
                module_key: 0,
                note_index: 0,
                notes: &self.notes,
                params,
            });
        }

        fn run(&mut self, input: &[f32; SEG]) -> [f32; SEG] {
            self.io[0] = *input;
            let params = VoiceParams::from_slices(&self.values, &self.cur, &self.prev);
            self.comb.process(&mut VoiceCx {
                voices: &[0],
                notes: &self.notes,
                params,
                io: &mut self.io,
                sync_out: None,
                sync_in: None,
                audio_in: None,
                pre_level: None,
            });
            self.settle();
            self.io[0]
        }

        /// The impulse response, `len` samples (a multiple of the segment).
        fn impulse(&mut self, len: usize) -> Vec<f32> {
            self.start();
            let mut out = Vec::with_capacity(len);
            let mut first = true;
            while out.len() < len {
                let mut block = [0.0f32; SEG];
                if first {
                    block[0] = 1.0;
                    first = false;
                }
                out.extend_from_slice(&self.run(&block));
            }
            out
        }
    }

    /// The tooth nearest `ideal` Hz in an impulse response's spectrum.
    fn tooth(ir: &[f32], ideal: f64, spacing: f64) -> f64 {
        let mut re: Vec<f64> = ir.iter().map(|&v| f64::from(v)).collect();
        let mut im = vec![0.0; ir.len()];
        fft(&mut re, &mut im);
        let bin = f64::from(SR) / ir.len() as f64;
        let lo = ((ideal - 0.25 * spacing) / bin) as usize;
        let hi = ((ideal + 0.25 * spacing) / bin) as usize;
        let best = (lo..=hi)
            .max_by(|&a, &b| {
                (re[a] * re[a] + im[a] * im[a]).total_cmp(&(re[b] * re[b] + im[b] * im[b]))
            })
            .expect("a non-empty range");
        best as f64 * bin
    }

    #[test]
    fn a_static_comb_matches_the_dsp_core_and_caches_its_tuning() {
        let mut rig = Rig::new(&[(CombParam::Feedback, 70.0), (CombParam::Damping, 6_000.0)]);
        rig.start();
        let f = comb::frequency_hz(0.0, 0.0, SR);
        let t = Tuning::new(f, 6_000.0, SR);
        let seg = Segment {
            from: t,
            to: t,
            feedback: [0.7, 0.7],
            mix: [1.0, 1.0],
        };
        let mut line = vec![0.0; comb::line_len(SR)];
        let mut core = State::ZERO;
        let input: [f32; SEG] = core::array::from_fn(|i| sinf(i as f32 * 0.37));
        for _ in 0..10 {
            let got = rig.run(&input);
            let mut want = input;
            comb::process_block(&mut line, &mut core, &seg, &mut want);
            assert_eq!(got, want);
        }
        assert_eq!(rig.comb.tuning[0], t);
    }

    #[test]
    fn key_tracking_puts_the_teeth_on_the_notes_harmonics_within_one_percent() {
        // Three keys: C2 (65.4 Hz), C4 (261.6 Hz) and C6 (1,046.5 Hz) at 100 % tracking, through the module with
        // the note's pitch from the engine: every tooth below 4 kHz is within 1 % of a harmonic of the note.
        for (key, f0) in [(36.0f32, 65.406f64), (60.0, 261.626), (84.0, 1_046.50)] {
            let mut rig = Rig::new(&[(CombParam::Feedback, 90.0)]);
            rig.notes.pitch_cur[0] = key;
            rig.notes.pitch_prev[0] = key;
            let ir = rig.impulse(131_072);
            let mut checked = 0;
            for k in 1..100 {
                let ideal = f0 * f64::from(k);
                if ideal > 4_000.0 {
                    break;
                }
                let found = tooth(&ir, ideal, f0);
                assert!(
                    (found - ideal).abs() / ideal < 0.01,
                    "key {key}: tooth {k} at {found:.1} Hz, wanted {ideal:.1} Hz"
                );
                checked += 1;
            }
            assert!(checked >= 3, "key {key}: {checked} teeth");
        }
    }

    #[test]
    fn semi_moves_the_teeth_and_zero_key_tracking_holds_them() {
        // +12 st doubles the spacing: teeth at 523 Hz multiples. At 0 % tracking the note does not matter.
        let mut up = Rig::new(&[(CombParam::Feedback, 90.0), (CombParam::Semi, 12.0)]);
        let ir = up.impulse(65_536);
        let found = tooth(&ir, 523.25, 523.25);
        assert!((found - 523.25).abs() / 523.25 < 0.01, "{found}");
        for key in [36.0f32, 72.0, 96.0] {
            let mut fixed = Rig::new(&[(CombParam::Feedback, 90.0), (CombParam::Keytrack, 0.0)]);
            fixed.notes.pitch_cur[0] = key;
            fixed.notes.pitch_prev[0] = key;
            let ir = fixed.impulse(65_536);
            let found = tooth(&ir, 261.63, 261.63);
            assert!((found - 261.63).abs() / 261.63 < 0.01, "key {key}: {found}");
        }
        // 50 % tracking at key 72 is half an octave... six semitones above middle C: 369.99 Hz.
        let mut half = Rig::new(&[(CombParam::Feedback, 90.0), (CombParam::Keytrack, 50.0)]);
        half.notes.pitch_cur[0] = 72.0;
        half.notes.pitch_prev[0] = 72.0;
        let ir = half.impulse(65_536);
        let found = tooth(&ir, 369.99, 369.99);
        assert!((found - 369.99).abs() / 369.99 < 0.01, "{found}");
    }

    #[test]
    fn the_frequency_is_clamped_at_twenty_hertz_whatever_semi_and_the_key() {
        // semi -48 at key 0 would be 0.5 Hz; the clamp keeps the delay at the 20 Hz period, 2,400 samples.
        let mut rig = Rig::new(&[(CombParam::Semi, -48.0), (CombParam::Feedback, 50.0)]);
        rig.notes.pitch_cur[0] = 0.0;
        rig.notes.pitch_prev[0] = 0.0;
        rig.start();
        let _ = rig.run(&[0.0; SEG]);
        let d = rig.comb.tuning[0].delay;
        assert!(d <= 2_400.0 && d > 2_390.0, "{d}");
        assert!(rig.comb.lines.len() == MAX_VOICES * 4_096);
    }

    #[test]
    fn a_pitch_change_ramps_from_the_previous_segment() {
        let mut rig = Rig::new(&[]);
        rig.start();
        let input = [0.5f32; SEG];
        let _ = rig.run(&input);
        let before = rig.comb.tuning[0];
        rig.set(CombParam::Semi, 7.0);
        let got = rig.run(&input);
        let to = Tuning::new(comb::frequency_hz(7.0, 0.0, SR), 10_000.0, SR);
        assert_eq!(rig.comb.tuning[0], to);
        let mut line = vec![0.0; comb::line_len(SR)];
        let mut core = State::ZERO;
        let t0 = Tuning::new(comb::frequency_hz(0.0, 0.0, SR), 10_000.0, SR);
        assert_eq!(before, t0);
        let steady = Segment {
            from: t0,
            to: t0,
            feedback: [0.5, 0.5],
            mix: [1.0, 1.0],
        };
        let mut warm = input;
        comb::process_block(&mut line, &mut core, &steady, &mut warm);
        let seg = Segment {
            from: t0,
            to,
            feedback: [0.5, 0.5],
            mix: [1.0, 1.0],
        };
        let mut want = input;
        comb::process_block(&mut line, &mut core, &seg, &mut want);
        assert_eq!(got, want);
    }

    #[test]
    fn mix_zero_passes_the_input_bit_for_bit_and_a_new_voice_starts_clean() {
        let mut rig = Rig::new(&[(CombParam::Mix, 0.0), (CombParam::Feedback, 90.0)]);
        rig.start();
        let input: [f32; SEG] = core::array::from_fn(|i| ((i * 7) % 13) as f32 / 13.0 - 0.5);
        for _ in 0..50 {
            let out = rig.run(&input);
            assert_eq!(out.map(f32::to_bits), input.map(f32::to_bits));
        }
        // The comb has been running underneath: stepping the mix up now is not a cold start.
        assert!(rig.comb.state[0] != State::ZERO);
        rig.start();
        assert_eq!(rig.comb.state[0], State::ZERO);
        assert!(rig.comb.line(0).iter().all(|&x| x == 0.0));
        assert!(!rig.comb.valid[0]);
        // After a restart the first block equals a fresh comb's.
        let mut fresh = Rig::new(&[(CombParam::Mix, 100.0), (CombParam::Feedback, 90.0)]);
        fresh.start();
        rig.set(CombParam::Mix, 100.0);
        rig.settle();
        assert_eq!(rig.run(&input), fresh.run(&input));
    }

    #[test]
    fn two_instances_keep_separate_state() {
        // Comb A alone, then A with B running between its segments on other input: A's output is the same bits.
        let settings = [(CombParam::Feedback, 85.0), (CombParam::Damping, 4_000.0)];
        let mut alone = Rig::new(&settings);
        alone.start();
        let mut a = Rig::new(&settings);
        a.start();
        let mut b = Rig::new(&[(CombParam::Feedback, -60.0), (CombParam::Semi, 5.0)]);
        b.start();
        let mut rng = Rng::new(9);
        for _ in 0..200 {
            let x: [f32; SEG] = core::array::from_fn(|_| rng.bipolar_f32() * 0.3);
            let y: [f32; SEG] = core::array::from_fn(|_| rng.bipolar_f32());
            let _ = b.run(&y);
            assert_eq!(alone.run(&x).map(f32::to_bits), a.run(&x).map(f32::to_bits));
        }
        assert!(b.comb.state[0] != a.comb.state[0]);
    }

    #[test]
    fn copy_state_makes_two_combs_identical() {
        let mut a = Rig::new(&[(CombParam::Feedback, 80.0), (CombParam::Semi, -5.0)]);
        a.start();
        let input: [f32; SEG] = core::array::from_fn(|i| ((i * 7) % 13) as f32 / 13.0 - 0.5);
        for _ in 0..60 {
            let _ = a.run(&input);
        }
        let mut b = Rig::new(&[(CombParam::Feedback, 80.0), (CombParam::Semi, -5.0)]);
        b.comb.copy_state_from(&a.comb);
        for _ in 0..60 {
            assert_eq!(
                a.run(&input).map(f32::to_bits),
                b.run(&input).map(f32::to_bits)
            );
        }
    }

    #[test]
    fn extreme_settings_with_a_new_random_pitch_every_segment_stay_finite_and_die_out() {
        // The acceptance stress through the module: a new random pitch, damping and feedback every segment, noise in.
        for fb_extreme in [99.0f32, -99.0] {
            let mut rig = Rig::new(&[]);
            rig.start();
            let mut rng = Rng::new(77);
            let mut peak = 0.0f32;
            for _ in 0..15_000 {
                rig.set(CombParam::Semi, (rng.unit_f32() - 0.5) * 96.0);
                rig.set(
                    CombParam::Damping,
                    500.0 * exp2f(rng.unit_f32() * log2f(40.0)),
                );
                rig.set(CombParam::Feedback, fb_extreme * rng.unit_f32().sqrt());
                rig.set(CombParam::Mix, 100.0 * rng.unit_f32());
                rig.notes.pitch_cur[0] = 24.0 + rng.unit_f32() * 80.0;
                let input: [f32; SEG] = core::array::from_fn(|_| rng.bipolar_f32() * 0.5);
                for y in rig.run(&input) {
                    assert!(y.is_finite());
                    peak = peak.max(y.abs());
                }
            }
            assert!(peak < 2.0, "fb {fb_extreme}: peak {peak}");
            // Park it at a short period and let it ring out: the state dies to exact zero, no subnormals.
            rig.set(CombParam::Semi, 24.0);
            rig.set(CombParam::Damping, 6_000.0);
            rig.set(CombParam::Feedback, 90.0 * fb_extreme.signum());
            rig.set(CombParam::Mix, 100.0);
            rig.notes.pitch_cur[0] = 60.0;
            for _ in 0..(48_000 * 30 / SEG) {
                let _ = rig.run(&[0.0; SEG]);
            }
            assert_eq!(rig.comb.subnormals(), 0);
            assert!(rig.comb.lines.iter().all(|&x| x == 0.0 || x.abs() >= 1e-15));
        }
    }
}
