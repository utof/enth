//! The `svf` module: the Cytomic/Simper TPT state-variable filter as a per-voice effect (P1a lane D2).
//! DSP core and equations: `enth_dsp::svf`; notes: `docs/dsp/svf.md`.
//!
//! Per voice and segment:
//!
//! 1. The effective cutoff is `cutoff · 2^(keytrack · (pitch − 60) / 12)` (`pitch` is `NoteState::pitch_cur`),
//!    clamped to 20 Hz … 0.45·fs. Resonance maps to the damping `k = 2 · 50^(−resonance %)` (Q 0.5 … 25,
//!    exponential in Q), and drive to the saturator's two coefficients (`svf::Drive`: `x / sqrt(b + c·x²)` with
//!    make-up gain, from `a = 10^(dB/10) − 1`).
//! 2. One `tanf` (plus one `exp2` for the resonance) gives the coefficients for the *end* of the segment. They are
//!    cached per voice and only recomputed when one of the four inputs (cutoff, resonance, drive, key-tracking term)
//!    changed, so a static filter costs no transcendental.
//! 3. `enth_dsp::svf::process_block` filters the 32 frames, ramping `a1, a2, a3, k` (and the drive's `b`, `c`) linearly
//!    from the end of the *previous* segment to this one's, then flushes the two integrator states.
//!
//! The module keeps the previous segment's coefficients itself, instead of recomputing them from the parameter views'
//! `prev` values: that halves the `tanf` calls and makes a mono retrigger or a smoother reset a 32-frame glide in
//! the coefficients rather than a step. A fresh voice (no previous segment) starts at its first segment's values.

use enth_dsp::svf::{self, Coeffs, Drive, Mode, State};
use enth_dsp::util::math::exp2f;
#[cfg(test)]
use enth_dsp::util::math::{log2f, sinf};
use enth_patch::catalogue::{FilterMode, SvfParam};

use crate::module::{MAX_VOICES, PrepareCfg, SEG, VoiceCx, VoiceProcessor, VoiceStart};

/// The inputs a voice's coefficients were computed from: cutoff, resonance, drive and the key-tracking term in
/// octaves. Equal inputs give equal coefficients, so the cache is exact.
type Key = [f32; 4];

/// The state-variable filter.
#[derive(Clone, Debug)]
pub struct Svf {
    sample_rate: f32,
    state: [State; MAX_VOICES],
    /// The coefficients at the end of the previous segment.
    coeffs: [Coeffs; MAX_VOICES],
    /// The saturator at the end of the previous segment.
    drive: [Drive; MAX_VOICES],
    key: [Key; MAX_VOICES],
    /// The voice has run a segment since `start_voice`: `coeffs`, `drive` and `key` are valid.
    valid: [bool; MAX_VOICES],
}

impl Svf {
    /// Prepare a filter.
    #[must_use]
    pub fn new(_values: &[f32], cfg: &PrepareCfg) -> Self {
        let idle = Coeffs::new(1_000.0, 1.0, cfg.sample_rate);
        Self {
            sample_rate: cfg.sample_rate,
            state: [State::ZERO; MAX_VOICES],
            coeffs: [idle; MAX_VOICES],
            drive: [Drive::OFF; MAX_VOICES],
            key: [[0.0; 4]; MAX_VOICES],
            valid: [false; MAX_VOICES],
        }
    }
}

/// The DSP mode for a catalogue value (`lp` for anything out of range: the engine clamps, so it cannot happen).
fn mode_of(value: f32) -> Mode {
    match FilterMode::from_value(value) {
        Some(FilterMode::Bp) => Mode::Bp,
        Some(FilterMode::Hp) => Mode::Hp,
        Some(FilterMode::Notch) => Mode::Notch,
        Some(FilterMode::Lp) | None => Mode::Lp,
    }
}

impl VoiceProcessor for Svf {
    fn start_voice(&mut self, start: &VoiceStart<'_>) {
        let v = start.voice;
        self.state[v] = State::ZERO;
        self.valid[v] = false;
    }

    fn process(&mut self, cx: &mut VoiceCx<'_>) {
        let p = cx.params;
        let mode = mode_of(p.base(SvfParam::Mode));
        for &v in cx.voices {
            let cutoff = p.cur(SvfParam::Cutoff, v);
            let resonance = p.cur(SvfParam::Resonance, v);
            let drive_db = p.cur(SvfParam::Drive, v);
            let keytrack = p.cur(SvfParam::Keytrack, v);
            // Octaves of key tracking: 100 % is one octave per octave played, relative to middle C.
            let tracked = if keytrack == 0.0 {
                0.0
            } else {
                (cx.notes.pitch_cur[v] - 60.0) * keytrack * (1.0 / 1200.0)
            };
            let key: Key = [cutoff, resonance, drive_db, tracked];
            let first = !self.valid[v];
            let (to, drive_to) = if first || key != self.key[v] {
                let fc = if tracked == 0.0 {
                    cutoff
                } else {
                    cutoff * exp2f(tracked)
                };
                self.key[v] = key;
                (
                    Coeffs::new(fc, svf::k_from_resonance(resonance), self.sample_rate),
                    Drive::from_db(drive_db),
                )
            } else {
                (self.coeffs[v], self.drive[v])
            };
            let (from, drive_from) = if first {
                (to, drive_to)
            } else {
                (self.coeffs[v], self.drive[v])
            };
            let io: &mut [f32; SEG] = &mut cx.io[v];
            svf::process_block(
                &mut self.state[v],
                mode,
                &from,
                &to,
                drive_from,
                drive_to,
                io,
            );
            self.coeffs[v] = to;
            self.drive[v] = drive_to;
            self.valid[v] = true;
        }
    }

    fn copy_state_from(&mut self, other: &Self) {
        self.state = other.state;
        self.coeffs = other.coeffs;
        self.drive = other.drive;
        self.key = other.key;
        self.valid = other.valid;
    }

    fn subnormals(&self) -> usize {
        let states = self.state.iter().filter(|s| s.has_subnormal()).count();
        let drives = self
            .drive
            .iter()
            .filter(|d| d.b().is_subnormal() || d.c().is_subnormal())
            .count();
        states + drives
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::module::{NoteState, VoiceBus, VoiceParams};
    use enth_patch::ModuleKind;

    const SR: f32 = 48_000.0;

    /// Parameter tables for one voice with the given values, `cur == prev == base`.
    struct Rig {
        values: Vec<f32>,
        cur: Vec<[f32; MAX_VOICES]>,
        notes: NoteState,
        io: Box<VoiceBus>,
        svf: Svf,
    }

    impl Rig {
        fn new(set: &[(SvfParam, f32)]) -> Self {
            let mut values = ModuleKind::Svf.defaults();
            for &(p, v) in set {
                values[p.index()] = v;
            }
            let cur = values.iter().map(|&v| [v; MAX_VOICES]).collect();
            let mut notes = NoteState::new();
            notes.gate[0] = true;
            notes.pitch_cur[0] = 60.0;
            notes.pitch_prev[0] = 60.0;
            let svf = Svf::new(&values, &PrepareCfg::new(SR));
            Self {
                values,
                cur,
                notes,
                io: Box::new([[0.0; SEG]; MAX_VOICES]),
                svf,
            }
        }

        fn set(&mut self, p: SvfParam, v: f32) {
            self.values[p.index()] = v;
            self.cur[p.index()] = [v; MAX_VOICES];
        }

        fn start(&mut self) {
            let params = VoiceParams::from_slices(&self.values, &self.cur, &self.cur);
            self.svf.start_voice(&VoiceStart {
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
            let params = VoiceParams::from_slices(&self.values, &self.cur, &self.cur);
            self.svf.process(&mut VoiceCx {
                voices: &[0],
                notes: &self.notes,
                params,
                io: &mut self.io,
                sync_out: None,
                sync_in: None,
                audio_in: None,
                pre_level: None,
                io_r: None,
            });
            self.io[0]
        }
    }

    #[test]
    fn a_static_filter_matches_the_dsp_core_and_caches_its_coefficients() {
        let mut rig = Rig::new(&[(SvfParam::Cutoff, 1_800.0), (SvfParam::Resonance, 40.0)]);
        rig.start();
        let c = Coeffs::new(1_800.0, svf::k_from_resonance(40.0), SR);
        let mut core = State::ZERO;
        let input: [f32; SEG] = core::array::from_fn(|i| sinf(i as f32 * 0.37));
        for _ in 0..10 {
            let got = rig.run(&input);
            let mut want = input;
            svf::process_block(
                &mut core,
                Mode::Lp,
                &c,
                &c,
                Drive::OFF,
                Drive::OFF,
                &mut want,
            );
            assert_eq!(got, want);
        }
        assert_eq!(rig.svf.coeffs[0], c);
    }

    #[test]
    fn a_cutoff_change_ramps_from_the_previous_segment() {
        let mut rig = Rig::new(&[(SvfParam::Cutoff, 400.0)]);
        rig.start();
        let input = [0.5f32; SEG];
        let _ = rig.run(&input);
        let before = rig.svf.coeffs[0];
        rig.set(SvfParam::Cutoff, 6_000.0);
        let got = rig.run(&input);
        let to = Coeffs::new(6_000.0, svf::k_from_resonance(10.0), SR);
        let mut want = input;
        let mut core = State::ZERO;
        let mut warm = input;
        let c0 = Coeffs::new(400.0, svf::k_from_resonance(10.0), SR);
        svf::process_block(
            &mut core,
            Mode::Lp,
            &c0,
            &c0,
            Drive::OFF,
            Drive::OFF,
            &mut warm,
        );
        svf::process_block(
            &mut core,
            Mode::Lp,
            &before,
            &to,
            Drive::OFF,
            Drive::OFF,
            &mut want,
        );
        assert_eq!(got, want);
        assert_eq!(rig.svf.coeffs[0], to);
    }

    #[test]
    fn keytracking_moves_the_cutoff_an_octave_per_octave() {
        let measure = |note: f32, track: f32| {
            let mut rig = Rig::new(&[(SvfParam::Cutoff, 1_000.0), (SvfParam::Keytrack, track)]);
            rig.notes.pitch_cur[0] = note;
            rig.start();
            let _ = rig.run(&[0.0; SEG]);
            rig.svf.coeffs[0]
        };
        // At 100 % an octave up (note 72) doubles the cutoff: the same coefficients as 2 kHz untracked.
        let up = measure(72.0, 100.0);
        let want = Coeffs::new(2_000.0 * 1.0, svf::k_from_resonance(10.0), SR);
        assert!((up.a1 - want.a1).abs() < 1e-5, "{up:?} vs {want:?}");
        // At 50 % half an octave: sqrt(2) kHz.
        let half = measure(72.0, 50.0);
        let want = Coeffs::new(1_414.213_5, svf::k_from_resonance(10.0), SR);
        assert!((half.a1 - want.a1).abs() < 1e-5);
        // At 0 % or at middle C nothing moves.
        let none = measure(84.0, 0.0);
        let mid = measure(60.0, 100.0);
        let plain = Coeffs::new(1_000.0, svf::k_from_resonance(10.0), SR);
        assert_eq!(none, plain);
        assert_eq!(mid, plain);
    }

    #[test]
    fn every_mode_filters_and_a_new_voice_starts_clean() {
        for (mode, name) in [
            (FilterMode::Lp, "lp"),
            (FilterMode::Bp, "bp"),
            (FilterMode::Hp, "hp"),
            (FilterMode::Notch, "notch"),
        ] {
            let mut rig = Rig::new(&[
                (SvfParam::Mode, mode.index() as f32),
                (SvfParam::Cutoff, 2_000.0),
            ]);
            rig.start();
            let loud = [1.0f32; SEG];
            for _ in 0..50 {
                let _ = rig.run(&loud);
            }
            assert!(rig.svf.state[0] != State::ZERO, "{name}");
            rig.start();
            assert_eq!(rig.svf.state[0], State::ZERO, "{name}: start_voice resets");
            assert!(!rig.svf.valid[0]);
            // After a restart the first block equals a fresh filter's.
            let mut fresh = Rig::new(&[
                (SvfParam::Mode, mode.index() as f32),
                (SvfParam::Cutoff, 2_000.0),
            ]);
            fresh.start();
            assert_eq!(rig.run(&loud), fresh.run(&loud), "{name}");
        }
    }

    #[test]
    fn drive_is_bit_transparent_at_zero_db_and_saturates_above() {
        let input: [f32; SEG] = core::array::from_fn(|i| 2.0 * sinf(i as f32 * 0.9));
        let mut clean = Rig::new(&[(SvfParam::Cutoff, 20_000.0), (SvfParam::Resonance, 0.0)]);
        clean.start();
        let a = clean.run(&input);
        let mut driven = Rig::new(&[
            (SvfParam::Cutoff, 20_000.0),
            (SvfParam::Resonance, 0.0),
            (SvfParam::Drive, 24.0),
        ]);
        driven.start();
        let b = driven.run(&input);
        assert_ne!(a, b);
        // Saturated: the saturator holds its output below 1/sqrt(c) = sqrt(1/3 + 1/a) = 0.581 at 24 dB, where the
        // clean input peaks at 2 (the filter's response to the squared-off 6.9 kHz wave overshoots that a little,
        // 0.77). The make-up keeps it near the reference level 1/√3 = 0.577, not at the old curve's 1/sqrt(a) = 0.063.
        let peak = |x: &[f32; SEG]| x.iter().fold(0.0f32, |m, y| m.max(y.abs()));
        assert!(
            peak(&b) < 0.8 && peak(&b) > 0.5 && peak(&a) > 1.0,
            "{} {}",
            peak(&b),
            peak(&a)
        );
    }

    #[test]
    fn a_drive_change_ramps_from_the_previous_segment() {
        let mut rig = Rig::new(&[(SvfParam::Cutoff, 20_000.0)]);
        rig.start();
        let input: [f32; SEG] = core::array::from_fn(|i| 0.9 * sinf(i as f32 * 0.3));
        let _ = rig.run(&input);
        assert_eq!(rig.svf.drive[0], Drive::OFF);
        rig.set(SvfParam::Drive, 18.0);
        let got = rig.run(&input);
        let c = Coeffs::new(20_000.0, svf::k_from_resonance(10.0), SR);
        let mut core = State::ZERO;
        let mut warm = input;
        svf::process_block(
            &mut core,
            Mode::Lp,
            &c,
            &c,
            Drive::OFF,
            Drive::OFF,
            &mut warm,
        );
        let mut want = input;
        svf::process_block(
            &mut core,
            Mode::Lp,
            &c,
            &c,
            Drive::OFF,
            Drive::from_db(18.0),
            &mut want,
        );
        assert_eq!(got, want);
        assert_eq!(rig.svf.drive[0], Drive::from_db(18.0));
        // Back to 0 dB: one ramp down, then the saturator is skipped again: from the same filter state, the next
        // block equals a never-driven filter's.
        rig.set(SvfParam::Drive, 0.0);
        let _ = rig.run(&input);
        assert_eq!(rig.svf.drive[0], Drive::OFF);
        let mut clean = Rig::new(&[(SvfParam::Cutoff, 20_000.0)]);
        clean.start();
        clean.svf.copy_state_from(&rig.svf);
        assert_eq!(rig.run(&input), clean.run(&input));
    }

    #[test]
    fn copy_state_makes_two_filters_identical() {
        let mut a = Rig::new(&[(SvfParam::Cutoff, 900.0), (SvfParam::Resonance, 80.0)]);
        a.start();
        let input: [f32; SEG] = core::array::from_fn(|i| ((i * 7) % 13) as f32 / 13.0 - 0.5);
        for _ in 0..20 {
            let _ = a.run(&input);
        }
        let mut b = Rig::new(&[(SvfParam::Cutoff, 900.0), (SvfParam::Resonance, 80.0)]);
        b.svf.copy_state_from(&a.svf);
        for _ in 0..20 {
            assert_eq!(a.run(&input), b.run(&input));
        }
    }

    #[test]
    fn full_resonance_with_random_cutoff_jumps_stays_finite_and_dies_out() {
        // The acceptance stress through the module: a new random cutoff every segment, Q 25, white noise.
        let mut rig = Rig::new(&[(SvfParam::Resonance, 100.0), (SvfParam::Mode, 2.0)]);
        rig.start();
        let mut rng = enth_dsp::util::rng::Rng::new(77);
        for _ in 0..15_000 {
            let fc = 20.0 * exp2f(rng.unit_f32() * log2f(1000.0));
            rig.set(SvfParam::Cutoff, fc);
            let input: [f32; SEG] = core::array::from_fn(|_| rng.bipolar_f32() * 0.5);
            for y in rig.run(&input) {
                assert!(y.is_finite() && y.abs() < 30.0);
            }
        }
        rig.set(SvfParam::Cutoff, 8_000.0);
        for _ in 0..5_000 {
            let _ = rig.run(&[0.0; SEG]);
        }
        assert_eq!(rig.svf.state[0], State::ZERO);
        assert_eq!(rig.svf.subnormals(), 0);
    }
}
