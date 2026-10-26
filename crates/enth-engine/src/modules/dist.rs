//! The `dist` module: distortion as a per-voice effect (S2a lane DST). DSP core and equations: `enth_dsp::dist` and
//! `enth_dsp::resample`; notes: `docs/dsp/dist.md`; the parameter table is lane CAT's (F1m,
//! `enth-patch/src/catalogue/dist.rs`).
//!
//! Per voice and segment:
//!
//! 1. The structure comes from the module's base values: the mode, the oversampling factor (structural: a change
//!    rebuilds the plan and the voices carry their state across it, `copy_state_from`) and the `keep_lows` corner.
//!    They are cached in a [`Setup`] and rebuilt only when one of the three changed.
//! 2. The six modulatable values (`drive`, `bias`, `bits`, `rate`, `tone`, `mix`) become a [`Ctl`] at the *end* of
//!    the segment, cached per voice and recomputed only when one of the six changed (an `exp2` for the drive, one for
//!    the bits, one `tan` for the tone), as `svf` does for its coefficients.
//! 3. `enth_dsp::dist::Voice::process` renders the 32 frames in place, interpolating the controls linearly from the
//!    previous segment's [`Ctl`] (kept here) to this one's. A fresh voice starts at its first segment's values.
//!
//! At `mix` 0 a voice is not processed and its bus is untouched, bit for bit; the module reports zero latency, the
//! oversampler being causal IIR (ADR-0006).

use enth_dsp::dist::{Ctl, Mode, Setup, Voice};
use enth_patch::catalogue::{DistMode, DistParam, Oversample};

use crate::module::{MAX_VOICES, PrepareCfg, VoiceCx, VoiceProcessor, VoiceStart};

/// What the structure was built from: `mode`, `oversample`, `keep_lows`.
type SetupKey = [f32; 3];
/// What a voice's controls were built from: `drive`, `bias`, `bits`, `rate`, `tone`, `mix`.
type CtlKey = [f32; 6];

/// The `dist` per-voice effect.
#[derive(Clone, Debug)]
pub struct Dist {
    sample_rate: f32,
    setup: Setup,
    setup_key: SetupKey,
    /// One per voice slot; on the heap because a voice is about a kilobyte (the stack array would be 30 KB).
    voice: Box<[Voice]>,
    /// The controls at the end of the previous segment.
    ctl: [Ctl; MAX_VOICES],
    key: [CtlKey; MAX_VOICES],
    /// The voice has run a segment since `start_voice`: `ctl` and `key` are valid.
    valid: [bool; MAX_VOICES],
}

/// The DSP mode for a catalogue value (`soft` for anything out of range: the engine clamps, so it cannot happen).
fn mode_of(value: f32) -> Mode {
    match DistMode::from_value(value) {
        Some(DistMode::Hard) => Mode::Hard,
        Some(DistMode::Asymmetric) => Mode::Asymmetric,
        Some(DistMode::SineFold) => Mode::SineFold,
        Some(DistMode::TriangleFold) => Mode::TriangleFold,
        Some(DistMode::Bitcrush) => Mode::Bitcrush,
        Some(DistMode::Downsample) => Mode::Downsample,
        Some(DistMode::Soft) | None => Mode::Soft,
    }
}

/// The oversampling factor for a catalogue value (2, the default, for anything out of range).
fn factor_of(value: f32) -> usize {
    match Oversample::from_value(value) {
        Some(Oversample::Off) => 1,
        Some(Oversample::X4) => 4,
        Some(Oversample::X2) | None => 2,
    }
}

fn setup_key(base: impl Fn(DistParam) -> f32) -> SetupKey {
    [
        base(DistParam::Mode),
        base(DistParam::Oversample),
        base(DistParam::KeepLows),
    ]
}

impl Dist {
    /// Prepare a `dist` effect.
    #[must_use]
    pub fn new(values: &[f32], cfg: &PrepareCfg) -> Self {
        let key = setup_key(|p| values.get(p.index()).copied().unwrap_or(0.0));
        let setup = Setup::new(mode_of(key[0]), factor_of(key[1]), key[2], cfg.sample_rate);
        let idle = Ctl::new(0.0, 0.0, 8.0, 6_000.0, 20_000.0, 100.0, cfg.sample_rate);
        Self {
            sample_rate: cfg.sample_rate,
            setup,
            setup_key: key,
            voice: vec![Voice::ZERO; MAX_VOICES].into_boxed_slice(),
            ctl: [idle; MAX_VOICES],
            key: [[0.0; 6]; MAX_VOICES],
            valid: [false; MAX_VOICES],
        }
    }
}

impl VoiceProcessor for Dist {
    fn start_voice(&mut self, start: &VoiceStart<'_>) {
        let v = start.voice;
        self.voice[v].reset();
        self.valid[v] = false;
    }

    fn process(&mut self, cx: &mut VoiceCx<'_>) {
        let p = cx.params;
        let skey = setup_key(|q| p.base(q));
        if skey != self.setup_key {
            self.setup = Setup::new(
                mode_of(skey[0]),
                factor_of(skey[1]),
                skey[2],
                self.sample_rate,
            );
            self.setup_key = skey;
        }
        for &v in cx.voices {
            let key: CtlKey = [
                p.cur(DistParam::Drive, v),
                p.cur(DistParam::Bias, v),
                p.cur(DistParam::Bits, v),
                p.cur(DistParam::Rate, v),
                p.cur(DistParam::Tone, v),
                p.cur(DistParam::Mix, v),
            ];
            let first = !self.valid[v];
            let to = if first || key != self.key[v] {
                self.key[v] = key;
                Ctl::new(
                    key[0],
                    key[1],
                    key[2],
                    key[3],
                    key[4],
                    key[5],
                    self.sample_rate,
                )
            } else {
                self.ctl[v]
            };
            let from = if first { to } else { self.ctl[v] };
            self.voice[v].process(&self.setup, &from, &to, &mut cx.io[v]);
            self.ctl[v] = to;
            self.valid[v] = true;
        }
    }

    fn copy_state_from(&mut self, other: &Self) {
        self.voice.copy_from_slice(&other.voice);
        self.ctl = other.ctl;
        self.key = other.key;
        self.valid = other.valid;
    }

    fn subnormals(&self) -> usize {
        self.voice.iter().map(Voice::subnormals).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::module::{NoteState, SEG, VoiceBus, VoiceParams};
    use enth_dsp::util::math::sin;
    use enth_patch::ModuleKind;

    const SR: f32 = 48_000.0;

    /// Parameter tables for the voices with the given values, `cur == prev == base`.
    struct Rig {
        values: Vec<f32>,
        cur: Vec<[f32; MAX_VOICES]>,
        notes: NoteState,
        io: Box<VoiceBus>,
        dist: Dist,
    }

    impl Rig {
        fn new(set: &[(DistParam, f32)]) -> Self {
            let mut values = ModuleKind::Dist.defaults();
            for &(p, v) in set {
                values[p.index()] = v;
            }
            let cur = values.iter().map(|&v| [v; MAX_VOICES]).collect();
            let dist = Dist::new(&values, &PrepareCfg::new(SR));
            Self {
                values,
                cur,
                notes: NoteState::new(),
                io: Box::new([[0.0; SEG]; MAX_VOICES]),
                dist,
            }
        }

        fn set(&mut self, p: DistParam, v: f32) {
            self.values[p.index()] = v;
            self.cur[p.index()] = [v; MAX_VOICES];
        }

        fn start(&mut self, voice: usize) {
            let params = VoiceParams::from_slices(&self.values, &self.cur, &self.cur);
            self.dist.start_voice(&VoiceStart {
                voice,
                seed: 0,
                module_key: 0,
                note_index: 0,
                notes: &self.notes,
                params,
            });
        }

        /// One segment for the voices in `voices`, with `inputs[i]` on voice `voices[i]`.
        fn run_voices(&mut self, voices: &[usize], inputs: &[[f32; SEG]]) -> Vec<[f32; SEG]> {
            for (&v, input) in voices.iter().zip(inputs) {
                self.io[v] = *input;
            }
            let params = VoiceParams::from_slices(&self.values, &self.cur, &self.cur);
            self.dist.process(&mut VoiceCx {
                voices,
                notes: &self.notes,
                params,
                io: &mut self.io,
                sync_out: None,
                sync_in: None,
                audio_in: None,
                pre_level: None,
            });
            voices.iter().map(|&v| self.io[v]).collect()
        }

        fn run(&mut self, input: &[f32; SEG]) -> [f32; SEG] {
            self.run_voices(&[0], &[*input])[0]
        }
    }

    fn wave(seg: usize, amp: f32, step: f32) -> [f32; SEG] {
        core::array::from_fn(|i| amp * sin(f64::from(step) * (seg * SEG + i) as f64) as f32)
    }

    /// What the module should do for static values: the DSP core directly.
    fn reference(values: &[f32]) -> (Setup, Ctl) {
        let get = |p: DistParam| values[p.index()];
        (
            Setup::new(
                mode_of(get(DistParam::Mode)),
                factor_of(get(DistParam::Oversample)),
                get(DistParam::KeepLows),
                SR,
            ),
            Ctl::new(
                get(DistParam::Drive),
                get(DistParam::Bias),
                get(DistParam::Bits),
                get(DistParam::Rate),
                get(DistParam::Tone),
                get(DistParam::Mix),
                SR,
            ),
        )
    }

    #[test]
    fn static_values_are_the_dsp_core_bit_for_bit_in_every_mode() {
        for mode in [
            DistMode::Soft,
            DistMode::Hard,
            DistMode::Asymmetric,
            DistMode::SineFold,
            DistMode::TriangleFold,
            DistMode::Bitcrush,
            DistMode::Downsample,
        ] {
            for os in [Oversample::Off, Oversample::X2, Oversample::X4] {
                let mut rig = Rig::new(&[
                    (DistParam::Mode, mode.index() as f32),
                    (DistParam::Oversample, os.index() as f32),
                    (DistParam::Drive, 14.0),
                    (DistParam::Bias, 12.0),
                    (DistParam::Tone, 5_000.0),
                    (DistParam::KeepLows, 120.0),
                    (DistParam::Mix, 80.0),
                ]);
                rig.start(0);
                let (su, ctl) = reference(&rig.values);
                let mut core = Voice::ZERO;
                for seg in 0..40 {
                    let input = wave(seg, 0.7, 0.21);
                    let got = rig.run(&input);
                    let mut want = input;
                    core.process(&su, &ctl, &ctl, &mut want);
                    assert_eq!(got, want, "{} {} seg {seg}", mode.key(), os.key());
                }
            }
        }
    }

    #[test]
    fn mix_zero_leaves_the_bus_untouched_in_every_mode() {
        for mode in 0..7 {
            let mut rig = Rig::new(&[
                (DistParam::Mode, mode as f32),
                (DistParam::Mix, 0.0),
                (DistParam::Drive, 30.0),
            ]);
            rig.start(0);
            for seg in 0..20 {
                let input = wave(seg, 0.9, 0.31);
                let got = rig.run(&input);
                assert!(
                    got.iter()
                        .zip(&input)
                        .all(|(a, b)| a.to_bits() == b.to_bits()),
                    "mode {mode} segment {seg}"
                );
            }
        }
    }

    #[test]
    fn a_drive_change_ramps_from_the_previous_segment() {
        let mut rig = Rig::new(&[(DistParam::Drive, 3.0), (DistParam::Tone, 8_000.0)]);
        rig.start(0);
        let (su, c0) = reference(&rig.values);
        let mut core = Voice::ZERO;
        let first = wave(0, 0.6, 0.2);
        let mut want = first;
        core.process(&su, &c0, &c0, &mut want);
        assert_eq!(rig.run(&first), want);
        rig.set(DistParam::Drive, 30.0);
        let (_, c1) = reference(&rig.values);
        let second = wave(1, 0.6, 0.2);
        let got = rig.run(&second);
        let mut want = second;
        core.process(&su, &c0, &c1, &mut want);
        assert_eq!(got, want);
        assert!(rig.dist.valid[0]);
    }

    #[test]
    fn two_instances_keep_separate_state() {
        // Two modules of one chain, with different settings, fed in turn; each equals a module run alone on the same
        // input, so neither saw the other's state.
        let a_set = [(DistParam::Drive, 24.0), (DistParam::Tone, 6_000.0)];
        let b_set = [
            (DistParam::Mode, DistMode::SineFold.index() as f32),
            (DistParam::Drive, 12.0),
            (DistParam::Bias, 20.0),
        ];
        let mut chain_a = Rig::new(&a_set);
        let mut chain_b = Rig::new(&b_set);
        let mut alone_b = Rig::new(&b_set);
        for rig in [&mut chain_a, &mut chain_b, &mut alone_b] {
            rig.start(0);
        }
        for seg in 0..60 {
            let x = wave(seg, 0.8, 0.17);
            let mid = chain_a.run(&x);
            let out = chain_b.run(&mid);
            let single = alone_b.run(&mid);
            assert_eq!(out, single, "segment {seg}");
        }
        assert_ne!(chain_a.dist.voice[0], chain_b.dist.voice[0]);
    }

    #[test]
    fn voices_are_independent_and_start_voice_resets_a_slot() {
        let mut rig = Rig::new(&[(DistParam::Drive, 18.0), (DistParam::Bias, 25.0)]);
        rig.start(0);
        rig.start(1);
        let x = wave(0, 0.8, 0.13);
        let silent = [0.0f32; SEG];
        let both = rig.run_voices(&[0, 1], &[x, silent]);
        assert!(both[1].iter().all(|&s| s == 0.0));
        for seg in 1..30 {
            let _ = rig.run_voices(&[0, 1], &[wave(seg, 0.8, 0.13), silent]);
        }
        assert_ne!(rig.dist.voice[0], Voice::ZERO);
        rig.start(0);
        assert_eq!(rig.dist.voice[0], Voice::ZERO);
        assert!(!rig.dist.valid[0]);
        // After a restart the first segment equals a fresh module's.
        let mut fresh = Rig::new(&[(DistParam::Drive, 18.0), (DistParam::Bias, 25.0)]);
        fresh.start(0);
        let x = wave(40, 0.8, 0.13);
        assert_eq!(rig.run(&x), fresh.run(&x));
    }

    #[test]
    fn copy_state_from_makes_two_instances_identical() {
        let set = [
            (DistParam::Drive, 20.0),
            (DistParam::KeepLows, 200.0),
            (DistParam::Tone, 3_000.0),
            (DistParam::Mix, 70.0),
        ];
        let mut old = Rig::new(&set);
        old.start(0);
        for seg in 0..25 {
            let _ = old.run(&wave(seg, 0.8, 0.09));
        }
        let mut new = Rig::new(&set);
        new.dist.copy_state_from(&old.dist);
        for seg in 25..60 {
            let x = wave(seg, 0.8, 0.09);
            assert_eq!(new.run(&x), old.run(&x), "segment {seg}");
        }
    }

    #[test]
    fn an_oversample_change_across_a_swap_carries_and_stays_finite() {
        let mut old = Rig::new(&[(DistParam::Oversample, 1.0), (DistParam::Drive, 24.0)]);
        old.start(0);
        for seg in 0..25 {
            let _ = old.run(&wave(seg, 0.8, 0.09));
        }
        for to in [0.0, 2.0] {
            let mut new = Rig::new(&[(DistParam::Oversample, to), (DistParam::Drive, 24.0)]);
            new.dist.copy_state_from(&old.dist);
            let mut peak = 0f32;
            for seg in 25..60 {
                let out = new.run(&wave(seg, 0.8, 0.09));
                assert!(out.iter().all(|s| s.is_finite()));
                peak = peak.max(out.iter().fold(0.0, |m, s| m.max(s.abs())));
            }
            assert!(peak > 0.3 && peak < 1.5, "oversample {to}: peak {peak}");
        }
    }

    #[test]
    fn a_tail_leaves_no_subnormals_and_a_silent_voice_stays_silent() {
        let mut rig = Rig::new(&[
            (DistParam::Drive, 30.0),
            (DistParam::Bias, 40.0),
            (DistParam::KeepLows, 300.0),
            (DistParam::Tone, 2_000.0),
        ]);
        rig.start(0);
        for seg in 0..60 {
            let _ = rig.run(&wave(seg, 0.9, 0.11));
        }
        let silent = [0.0f32; SEG];
        for _ in 0..(40 * 48_000 / SEG) {
            let out = rig.run(&silent);
            assert_eq!(rig.dist.subnormals(), 0);
            assert!(out.iter().all(|s| s.is_finite()));
        }
        // 40 s of silence: the DC from bias has been blocked and nothing rings (the rounding residue of the
        // oversampler's settling is far below -300 dB, which is where the states are flushed).
        assert!(rig.run(&silent).iter().all(|&s| s.abs() < 1.0e-12));
    }
}
