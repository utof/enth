//! The `ladder` module: a four-pole TPT Moog ladder as a per-voice effect, mirroring `svf` (S2a lane FLT). DSP core
//! and equations: `enth_dsp::ladder`; notes: `docs/dsp/ladder.md`.
//!
//! Per voice and segment, exactly as `modules/svf.rs`:
//!
//! 1. The effective cutoff is `cutoff · 2^(keytrack · (pitch − 60) / 12)` (`pitch` is `NoteState::pitch_cur`, so a
//!    glide moves it), clamped to 20 Hz … 0.45·fs; resonance maps to the feedback gain `k = 4.2 · resonance/100`,
//!    and drive to the svf's saturator (`svf::Drive`, from `a = 10^(dB/10) − 1`).
//! 2. One `tanf` gives the coefficients for the *end* of the segment, cached per voice and recomputed only when one of
//!    the four inputs (cutoff, resonance, drive, key-tracking term) changed, so a static filter costs no
//!    transcendental.
//! 3. [`enth_dsp::ladder::process_block`] filters the 32 frames, ramping `G`, `k` and the drive's `b` and `c`
//!    linearly from the end of the *previous* segment to this one's, and `mix` from its previous segment's value to
//!    this one's, then flushes the four stage states.
//!
//! The module keeps the previous segment's coefficients itself (a retrigger or a smoother reset is a 32-frame glide);
//! a fresh voice starts at its first segment's values. Parameters are read through `.index()` (a `usize` is a
//! [`ParamIndex`](crate::params::ParamIndex)), so the shared `params.rs` list stays as it is.

use enth_dsp::ladder::{self, Coeffs, Mode, Segment, State};
use enth_dsp::svf::Drive;
use enth_dsp::util::math::exp2f;
use enth_patch::catalogue::{LadderMode, LadderParam};

use crate::module::{
    MAX_VOICES, NoteState, PrepareCfg, VoiceCx, VoiceParams, VoiceProcessor, VoiceStart,
};

/// The inputs a voice's coefficients were computed from: cutoff, resonance, drive and the key-tracking term in
/// octaves. Equal inputs give equal coefficients, so the cache is exact.
type Key = [f32; 4];

/// The ladder filter.
#[derive(Clone, Debug)]
pub struct Ladder {
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

impl Ladder {
    /// Prepare a filter.
    #[must_use]
    pub fn new(_values: &[f32], cfg: &PrepareCfg) -> Self {
        let idle = Coeffs::new(1_000.0, 0.0, cfg.sample_rate);
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

/// The DSP mode for a catalogue value (`lp24` for anything out of range: the engine clamps, so it cannot happen).
fn mode_of(value: f32) -> Mode {
    match LadderMode::from_value(value) {
        Some(LadderMode::Lp12) => Mode::Lp12,
        Some(LadderMode::Bp) => Mode::Bp,
        Some(LadderMode::Hp) => Mode::Hp,
        Some(LadderMode::Lp24) | None => Mode::Lp24,
    }
}

impl Ladder {
    /// The segment of voice `v`: where its coefficients were and where they go. Remembers the end for the next one.
    fn segment(&mut self, p: &VoiceParams<'_>, notes: &NoteState, v: usize, mode: Mode) -> Segment {
        let (cutoff_i, res_i, key_i, drive_i, mix_i) = (
            LadderParam::Cutoff.index(),
            LadderParam::Resonance.index(),
            LadderParam::Keytrack.index(),
            LadderParam::Drive.index(),
            LadderParam::Mix.index(),
        );
        let cutoff = p.cur(cutoff_i, v);
        let resonance = p.cur(res_i, v);
        let drive_db = p.cur(drive_i, v);
        let keytrack = p.cur(key_i, v);
        // Octaves of key tracking: 100 % is one octave per octave played, relative to middle C (as the svf).
        let tracked = if keytrack == 0.0 {
            0.0
        } else {
            (notes.pitch_cur[v] - 60.0) * keytrack * (1.0 / 1200.0)
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
                Coeffs::new(fc, ladder::k_from_resonance(resonance), self.sample_rate),
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
        self.coeffs[v] = to;
        self.drive[v] = drive_to;
        self.valid[v] = true;
        Segment {
            mode,
            from,
            to,
            drive_from,
            drive_to,
            mix: [p.prev(mix_i, v) * 0.01, p.cur(mix_i, v) * 0.01],
        }
    }
}

impl VoiceProcessor for Ladder {
    fn start_voice(&mut self, start: &VoiceStart<'_>) {
        let v = start.voice;
        self.state[v] = State::ZERO;
        self.valid[v] = false;
    }

    fn process(&mut self, cx: &mut VoiceCx<'_>) {
        let p = cx.params;
        let mode = mode_of(p.base(LadderParam::Mode.index()));
        for &v in cx.voices {
            let seg = self.segment(&p, cx.notes, v, mode);
            ladder::process_block(&mut self.state[v], &seg, &mut cx.io[v]);
        }
    }

    /// The voices of this module and of its right twin, two voices (four instances) at a time through
    /// `ladder::process_lanes`: each instance gets the bits `process` gives it alone.
    fn process_pair(
        &mut self,
        right: &mut Self,
        left_cx: &mut VoiceCx<'_>,
        right_cx: &mut VoiceCx<'_>,
    ) {
        if left_cx.voices != right_cx.voices {
            self.process(left_cx);
            right.process(right_cx);
            return;
        }
        let (pl, pr) = (left_cx.params, right_cx.params);
        let mode_i = LadderParam::Mode.index();
        let (ml, mr) = (mode_of(pl.base(mode_i)), mode_of(pr.base(mode_i)));
        for group in left_cx.voices.chunks(2) {
            let &[a, ..] = group else { continue };
            let seg_la = self.segment(&pl, left_cx.notes, a, ml);
            let seg_ra = right.segment(&pr, right_cx.notes, a, mr);
            if let &[_, b] = group {
                let seg_lb = self.segment(&pl, left_cx.notes, b, ml);
                let seg_rb = right.segment(&pr, right_cx.notes, b, mr);
                if let (Ok([sla, slb]), Ok([sra, srb]), Ok([ila, ilb]), Ok([ira, irb])) = (
                    self.state.get_disjoint_mut([a, b]),
                    right.state.get_disjoint_mut([a, b]),
                    left_cx.io.get_disjoint_mut([a, b]),
                    right_cx.io.get_disjoint_mut([a, b]),
                ) {
                    ladder::process_lanes(
                        [sla, sra, slb, srb],
                        [&seg_la, &seg_ra, &seg_lb, &seg_rb],
                        [ila, ira, ilb, irb],
                    );
                } else {
                    ladder::process_block(&mut self.state[a], &seg_la, &mut left_cx.io[a]);
                    ladder::process_block(&mut right.state[a], &seg_ra, &mut right_cx.io[a]);
                    ladder::process_block(&mut self.state[b], &seg_lb, &mut left_cx.io[b]);
                    ladder::process_block(&mut right.state[b], &seg_rb, &mut right_cx.io[b]);
                }
            } else {
                ladder::process_lanes(
                    [&mut self.state[a], &mut right.state[a]],
                    [&seg_la, &seg_ra],
                    [&mut left_cx.io[a], &mut right_cx.io[a]],
                );
            }
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
    use crate::module::{NoteState, SEG, VoiceBus, VoiceParams};
    use enth_dsp::alias::fft;
    use enth_dsp::svf;
    use enth_dsp::util::math::{log2f, sinf};
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
        filter: Ladder,
    }

    impl Rig {
        fn new(set: &[(LadderParam, f32)]) -> Self {
            let mut values = ModuleKind::Ladder.defaults();
            for &(p, v) in set {
                values[p.index()] = v;
            }
            let cur: Vec<[f32; MAX_VOICES]> = values.iter().map(|&v| [v; MAX_VOICES]).collect();
            let mut notes = NoteState::new();
            notes.gate[0] = true;
            notes.pitch_cur[0] = 60.0;
            notes.pitch_prev[0] = 60.0;
            let filter = Ladder::new(&values, &PrepareCfg::new(SR));
            Self {
                prev: cur.clone(),
                values,
                cur,
                notes,
                io: Box::new([[0.0; SEG]; MAX_VOICES]),
                filter,
            }
        }

        fn set(&mut self, p: LadderParam, v: f32) {
            self.values[p.index()] = v;
            self.prev[p.index()] = self.cur[p.index()];
            self.cur[p.index()] = [v; MAX_VOICES];
        }

        fn settle(&mut self) {
            self.prev = self.cur.clone();
        }

        fn start(&mut self) {
            let params = VoiceParams::from_slices(&self.values, &self.cur, &self.cur);
            self.filter.start_voice(&VoiceStart {
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
            self.filter.process(&mut VoiceCx {
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
            self.settle();
            self.io[0]
        }

        /// The impulse response of an impulse of `amp` (so the limiter stays linear), divided by `amp`.
        fn impulse(&mut self, len: usize, amp: f32) -> Vec<f32> {
            self.start();
            let mut out = Vec::with_capacity(len);
            let mut first = true;
            while out.len() < len {
                let mut block = [0.0f32; SEG];
                if first {
                    block[0] = amp;
                    first = false;
                }
                out.extend(self.run(&block).iter().map(|&v| v / amp));
            }
            out
        }
    }

    #[test]
    fn a_static_filter_matches_the_dsp_core_and_caches_its_coefficients() {
        for (mode_item, mode) in [
            (LadderMode::Lp24, Mode::Lp24),
            (LadderMode::Lp12, Mode::Lp12),
            (LadderMode::Bp, Mode::Bp),
            (LadderMode::Hp, Mode::Hp),
        ] {
            let mut rig = Rig::new(&[
                (LadderParam::Mode, mode_item.index() as f32),
                (LadderParam::Cutoff, 1_500.0),
                (LadderParam::Resonance, 70.0),
                (LadderParam::Drive, 6.0),
            ]);
            rig.start();
            let c = Coeffs::new(1_500.0, ladder::k_from_resonance(70.0), SR);
            let seg = Segment {
                mode,
                from: c,
                to: c,
                drive_from: Drive::from_db(6.0),
                drive_to: Drive::from_db(6.0),
                mix: [1.0, 1.0],
            };
            let mut core = State::ZERO;
            let input: [f32; SEG] = core::array::from_fn(|i| sinf(i as f32 * 0.37) * 0.6);
            for _ in 0..10 {
                let got = rig.run(&input);
                let mut want = input;
                ladder::process_block(&mut core, &seg, &mut want);
                assert_eq!(got, want, "{mode:?}");
            }
            assert_eq!(rig.filter.coeffs[0], c);
        }
    }

    #[test]
    fn the_cutoff_tracks_the_key_exactly_as_the_svf_does() {
        // The same cutoff, key tracking and note give the svf's `g = tan(π·fc/fs)` in the ladder's `G = g/(1 + g)`.
        for (cutoff, keytrack, key) in [
            (2_000.0f32, 100.0f32, 72.0f32),
            (500.0, 50.0, 36.0),
            (8_000.0, 100.0, 96.0),
            (1_000.0, 0.0, 84.0),
            (20.0, 100.0, 12.0),
        ] {
            let mut rig = Rig::new(&[
                (LadderParam::Cutoff, cutoff),
                (LadderParam::Keytrack, keytrack),
            ]);
            rig.notes.pitch_cur[0] = key;
            rig.notes.pitch_prev[0] = key;
            rig.start();
            let _ = rig.run(&[0.0; SEG]);
            let ladder_g = rig.filter.coeffs[0].g;
            // The svf module's rule, written out: cutoff · 2^(keytrack · (pitch − 60) / 1200 octaves).
            let tracked = (key - 60.0) * keytrack * (1.0 / 1200.0);
            let fc = if tracked == 0.0 {
                cutoff
            } else {
                cutoff * exp2f(tracked)
            };
            let s = svf::Coeffs::new(fc, 1.0, SR);
            let g = s.a2 / s.a1;
            assert!(
                (ladder_g - g / (1.0 + g)).abs() < 1e-6,
                "cutoff {cutoff}, tracking {keytrack} %, key {key}: {ladder_g} vs {}",
                g / (1.0 + g)
            );
        }
        // And in Hz: key 72 at 100 % doubles a 1 kHz cutoff; the impulse response's −3 dB corner follows.
        let mut up = Rig::new(&[
            (LadderParam::Mode, LadderMode::Lp12.index() as f32),
            (LadderParam::Cutoff, 1_000.0),
            (LadderParam::Resonance, 0.0),
            (LadderParam::Keytrack, 100.0),
        ]);
        up.notes.pitch_cur[0] = 72.0;
        up.notes.pitch_prev[0] = 72.0;
        let ir = up.impulse(65_536, 1e-3);
        let mut re: Vec<f64> = ir.iter().map(|&v| f64::from(v)).collect();
        let mut im = vec![0.0; ir.len()];
        fft(&mut re, &mut im);
        let bin = f64::from(SR) / ir.len() as f64;
        let h = |f: f64| {
            let i = (f / bin).round() as usize;
            (re[i] * re[i] + im[i] * im[i]).sqrt()
        };
        // lp12 at k = 0: two poles at 2 kHz, |H| = 1/2 (−6.02 dB) at the cutoff.
        assert!((h(2_000.0) - 0.5).abs() < 0.005, "{}", h(2_000.0));
    }

    #[test]
    fn at_the_top_of_the_knob_the_module_rings_at_the_cutoff() {
        let mut rig = Rig::new(&[
            (LadderParam::Cutoff, 1_000.0),
            (LadderParam::Resonance, 100.0),
        ]);
        rig.start();
        let mut rng = Rng::new(5);
        let mut tail = Vec::new();
        for b in 0..(6 * 48_000 / SEG) {
            let input: [f32; SEG] = if b == 0 {
                core::array::from_fn(|_| rng.bipolar_f32() * 0.05)
            } else {
                [0.0; SEG]
            };
            let out = rig.run(&input);
            if b >= (6 * 48_000 - 65_536) / SEG {
                tail.extend_from_slice(&out);
            }
        }
        let n = tail.len().next_power_of_two() / 2;
        let tail = &tail[tail.len() - n..];
        let mut re: Vec<f64> = tail.iter().map(|&v| f64::from(v)).collect();
        let mut im = vec![0.0; n];
        fft(&mut re, &mut im);
        let best = (1..n / 2)
            .max_by(|&a, &b| {
                (re[a] * re[a] + im[a] * im[a]).total_cmp(&(re[b] * re[b] + im[b] * im[b]))
            })
            .expect("bins");
        let f = best as f64 * f64::from(SR) / n as f64;
        assert!((f / 1_000.0 - 1.0).abs() < 0.02, "rings at {f} Hz");
        assert!(tail.iter().any(|v| v.abs() > 0.1), "the ring died");
        assert!(tail.iter().all(|v| v.abs() < 1.0));
    }

    #[test]
    fn a_change_ramps_from_the_previous_segment() {
        let mut rig = Rig::new(&[(LadderParam::Resonance, 60.0)]);
        rig.start();
        let input = [0.4f32; SEG];
        let _ = rig.run(&input);
        let before = rig.filter.coeffs[0];
        rig.set(LadderParam::Cutoff, 5_000.0);
        let got = rig.run(&input);
        let k = ladder::k_from_resonance(60.0);
        let to = Coeffs::new(5_000.0, k, SR);
        assert_eq!(rig.filter.coeffs[0], to);
        let t0 = Coeffs::new(2_000.0, k, SR);
        assert_eq!(before, t0);
        let mut core = State::ZERO;
        let steady = Segment {
            mode: Mode::Lp24,
            from: t0,
            to: t0,
            drive_from: Drive::OFF,
            drive_to: Drive::OFF,
            mix: [1.0, 1.0],
        };
        let mut warm = input;
        ladder::process_block(&mut core, &steady, &mut warm);
        let seg = Segment { to, ..steady };
        let mut want = input;
        ladder::process_block(&mut core, &seg, &mut want);
        assert_eq!(got, want);
    }

    #[test]
    fn mix_zero_passes_the_input_bit_for_bit_and_a_new_voice_starts_clean() {
        let mut rig = Rig::new(&[(LadderParam::Mix, 0.0), (LadderParam::Resonance, 90.0)]);
        rig.start();
        let input: [f32; SEG] = core::array::from_fn(|i| ((i * 7) % 13) as f32 / 13.0 - 0.5);
        for _ in 0..50 {
            let out = rig.run(&input);
            assert_eq!(out.map(f32::to_bits), input.map(f32::to_bits));
        }
        assert_ne!(rig.filter.state[0], State::ZERO);
        rig.start();
        assert_eq!(rig.filter.state[0], State::ZERO);
        assert!(!rig.filter.valid[0]);
        let mut fresh = Rig::new(&[(LadderParam::Mix, 100.0), (LadderParam::Resonance, 90.0)]);
        fresh.start();
        rig.set(LadderParam::Mix, 100.0);
        rig.settle();
        assert_eq!(rig.run(&input), fresh.run(&input));
    }

    #[test]
    fn two_instances_keep_separate_state() {
        let settings = [(LadderParam::Resonance, 80.0), (LadderParam::Cutoff, 900.0)];
        let mut alone = Rig::new(&settings);
        alone.start();
        let mut a = Rig::new(&settings);
        a.start();
        let mut b = Rig::new(&[
            (LadderParam::Mode, LadderMode::Hp.index() as f32),
            (LadderParam::Cutoff, 300.0),
        ]);
        b.start();
        let mut rng = Rng::new(9);
        for _ in 0..200 {
            let x: [f32; SEG] = core::array::from_fn(|_| rng.bipolar_f32() * 0.3);
            let y: [f32; SEG] = core::array::from_fn(|_| rng.bipolar_f32());
            let _ = b.run(&y);
            assert_eq!(alone.run(&x).map(f32::to_bits), a.run(&x).map(f32::to_bits));
        }
        assert_ne!(b.filter.state[0], a.filter.state[0]);
    }

    #[test]
    fn copy_state_makes_two_filters_identical() {
        let set = [(LadderParam::Resonance, 85.0), (LadderParam::Drive, 9.0)];
        let mut a = Rig::new(&set);
        a.start();
        let input: [f32; SEG] = core::array::from_fn(|i| ((i * 7) % 13) as f32 / 13.0 - 0.5);
        for _ in 0..60 {
            let _ = a.run(&input);
        }
        let mut b = Rig::new(&set);
        b.filter.copy_state_from(&a.filter);
        for _ in 0..60 {
            assert_eq!(
                a.run(&input).map(f32::to_bits),
                b.run(&input).map(f32::to_bits)
            );
        }
    }

    #[test]
    fn extreme_settings_with_new_random_values_every_segment_stay_finite_and_die_out() {
        for mode in [
            LadderMode::Lp24,
            LadderMode::Lp12,
            LadderMode::Bp,
            LadderMode::Hp,
        ] {
            let mut rig = Rig::new(&[(LadderParam::Mode, mode.index() as f32)]);
            rig.start();
            let mut rng = Rng::new(77);
            let mut peak = 0.0f32;
            for _ in 0..15_000 {
                rig.set(
                    LadderParam::Cutoff,
                    20.0 * enth_dsp::util::math::exp2f(rng.unit_f32() * log2f(1_000.0)),
                );
                rig.set(LadderParam::Resonance, 100.0 * rng.unit_f32());
                rig.set(LadderParam::Drive, 24.0 * rng.unit_f32());
                rig.set(LadderParam::Mix, 100.0 * rng.unit_f32());
                let input: [f32; SEG] = core::array::from_fn(|_| rng.bipolar_f32() * 0.5);
                for y in rig.run(&input) {
                    assert!(y.is_finite());
                    peak = peak.max(y.abs());
                }
            }
            assert!(peak < 20.0, "{mode:?}: peak {peak}");
            rig.set(LadderParam::Cutoff, 2_000.0);
            rig.set(LadderParam::Resonance, 80.0);
            rig.set(LadderParam::Drive, 0.0);
            rig.set(LadderParam::Mix, 100.0);
            for _ in 0..(48_000 * 30 / SEG) {
                let _ = rig.run(&[0.0; SEG]);
                assert_eq!(rig.filter.subnormals(), 0);
            }
            assert_eq!(rig.filter.state[0], State::ZERO);
        }
    }

    /// `process_pair` on a ladder and its twin gives, bit for bit, what `process` on each gives: through every
    /// response, a cutoff that moves and rests, a self-oscillating resonance, a saturator on some voices only, mixes
    /// that switch between off, blended and full, a restarted voice, odd and even counts of voices, and a twin on
    /// other settings than the left (a stereo plan never does that).
    #[test]
    fn process_pair_is_two_process_calls_bit_for_bit() {
        use crate::modules::pair_test::check;
        let cfg = PrepareCfg::new(SR);
        for mode in [
            LadderMode::Lp24,
            LadderMode::Lp12,
            LadderMode::Bp,
            LadderMode::Hp,
        ] {
            for voices in [vec![0usize, 3, 4], vec![0, 1, 2, 5], vec![7], vec![2, 9]] {
                for twin_differs in [false, true] {
                    check(
                        ModuleKind::Ladder,
                        |values| Ladder::new(values, &cfg),
                        &voices,
                        96,
                        |seg, right, t| {
                            let ix = |p: LadderParam| p.index();
                            t.all(ix(LadderParam::Mode), mode.index() as f32);
                            let moving = (10..30).contains(&seg);
                            t.set(ix(LadderParam::Cutoff), |v| {
                                let base = 300.0 + 90.0 * v as f32;
                                if moving {
                                    base + 55.0 * (seg - 10) as f32 * (1 + v % 3) as f32
                                } else if seg >= 30 {
                                    base + 1_000.0
                                } else {
                                    base
                                }
                            });
                            let res = if seg >= 40 { 97.0 } else { 35.0 };
                            t.all(
                                ix(LadderParam::Resonance),
                                if twin_differs && right {
                                    res - 20.0
                                } else {
                                    res
                                },
                            );
                            t.set(ix(LadderParam::Drive), |v| {
                                if (60..75).contains(&seg) && v % 2 == 1 {
                                    12.0
                                } else if (70..80).contains(&seg) && twin_differs && right {
                                    6.0
                                } else {
                                    0.0
                                }
                            });
                            t.set(ix(LadderParam::Keytrack), |v| {
                                if seg % 7 < 3 {
                                    0.0
                                } else {
                                    20.0 * (v % 3) as f32
                                }
                            });
                            let mix = match seg {
                                80..=85 => 0.0,
                                86..=90 => 50.0,
                                _ => 100.0,
                            };
                            t.all(ix(LadderParam::Mix), mix);
                        },
                        |seg| {
                            if seg == 50 {
                                vec![(voices[voices.len() - 1], true)]
                            } else {
                                Vec::new()
                            }
                        },
                    );
                }
            }
        }
    }
}
