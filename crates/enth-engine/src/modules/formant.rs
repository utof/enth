//! The `formant` module: a formant (vowel) filter per voice, the vowel placed by its first two formant frequencies
//! (S2a lane FLT). DSP core and equations: `enth_dsp::formant`; notes: `docs/dsp/formant.md`.
//!
//! Per voice and segment:
//!
//! 1. `f1`, `f2`, `shift` and `resonance` (this segment's values, after the engine's smoothing and modulation) give
//!    the end-of-segment [`Tuning`]: three band-pass sections at `f1`, `f2` and 2.7 kHz, all shifted by `shift`
//!    semitones, of one Q from `resonance`. It is cached per voice and recomputed (two `exp2`, three `tan`) only when
//!    one of the four inputs changed, so a static vowel costs no transcendental.
//! 2. [`enth_dsp::formant::process_block`] runs the 32 frames, ramping every section's coefficients and output weight
//!    linearly per sample from the end of the previous segment to the end of this one, and `mix` from its previous
//!    segment's value to this one's.
//!
//! The module keeps each voice's previous tuning itself (as `svf` does), so a retrigger or a smoother reset glides
//! over a segment instead of stepping; a fresh voice starts at its first segment's values. State per voice is six
//! floats (two integrators in each of three sections): no allocation outside `new`, and nothing to carry but those
//! on a plan swap.
//!
//! Parameters are read through `.index()` (a `usize` is a [`ParamIndex`](crate::params::ParamIndex)), so the shared
//! `params.rs` list stays as it is.

use enth_dsp::formant::{self, Segment, State, Tuning};
use enth_patch::catalogue::FormantParam;

use crate::module::{MAX_VOICES, PrepareCfg, VoiceCx, VoiceParams, VoiceProcessor, VoiceStart};

/// The inputs a voice's tuning was computed from: `f1`, `f2`, `shift` and `resonance`. Equal inputs give equal
/// tunings, so the cache is exact.
type Key = [f32; 4];

/// The formant filter.
#[derive(Clone, Debug)]
pub struct Formant {
    sample_rate: f32,
    state: [State; MAX_VOICES],
    /// The tuning at the end of the previous segment.
    tuning: [Tuning; MAX_VOICES],
    key: [Key; MAX_VOICES],
    /// The voice has run a segment since `start_voice`: `tuning` and `key` are valid.
    valid: [bool; MAX_VOICES],
}

impl Formant {
    /// Prepare a formant filter.
    #[must_use]
    pub fn new(_values: &[f32], cfg: &PrepareCfg) -> Self {
        let idle = Tuning::new(850.0, 1_610.0, 0.0, 50.0, cfg.sample_rate);
        Self {
            sample_rate: cfg.sample_rate,
            state: [State::ZERO; MAX_VOICES],
            tuning: [idle; MAX_VOICES],
            key: [[0.0; 4]; MAX_VOICES],
            valid: [false; MAX_VOICES],
        }
    }
}

impl Formant {
    /// The segment of voice `v`: where its tuning was and where it goes. Remembers the end for the next one.
    fn segment(&mut self, p: &VoiceParams<'_>, v: usize) -> Segment {
        let (f1_i, f2_i, shift_i, res_i, mix_i) = (
            FormantParam::F1.index(),
            FormantParam::F2.index(),
            FormantParam::Shift.index(),
            FormantParam::Resonance.index(),
            FormantParam::Mix.index(),
        );
        let key: Key = [
            p.cur(f1_i, v),
            p.cur(f2_i, v),
            p.cur(shift_i, v),
            p.cur(res_i, v),
        ];
        let first = !self.valid[v];
        let to = if first || key != self.key[v] {
            self.key[v] = key;
            Tuning::new(key[0], key[1], key[2], key[3], self.sample_rate)
        } else {
            self.tuning[v]
        };
        let from = if first { to } else { self.tuning[v] };
        self.tuning[v] = to;
        self.valid[v] = true;
        Segment {
            from,
            to,
            mix: [p.prev(mix_i, v) * 0.01, p.cur(mix_i, v) * 0.01],
        }
    }
}

impl VoiceProcessor for Formant {
    fn start_voice(&mut self, start: &VoiceStart<'_>) {
        let v = start.voice;
        self.state[v] = State::ZERO;
        self.valid[v] = false;
    }

    fn process(&mut self, cx: &mut VoiceCx<'_>) {
        let p = cx.params;
        for &v in cx.voices {
            let seg = self.segment(&p, v);
            formant::process_block(&mut self.state[v], &seg, &mut cx.io[v]);
        }
    }

    /// The voices of this module and of its right twin, two voices (four instances) at a time through
    /// `formant::process_lanes`: each instance gets the bits `process` gives it alone.
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
        for group in left_cx.voices.chunks(2) {
            let &[a, ..] = group else { continue };
            let seg_la = self.segment(&pl, a);
            let seg_ra = right.segment(&pr, a);
            if let &[_, b] = group {
                let seg_lb = self.segment(&pl, b);
                let seg_rb = right.segment(&pr, b);
                if let (Ok([sla, slb]), Ok([sra, srb]), Ok([ila, ilb]), Ok([ira, irb])) = (
                    self.state.get_disjoint_mut([a, b]),
                    right.state.get_disjoint_mut([a, b]),
                    left_cx.io.get_disjoint_mut([a, b]),
                    right_cx.io.get_disjoint_mut([a, b]),
                ) {
                    formant::process_lanes(
                        [sla, sra, slb, srb],
                        [&seg_la, &seg_ra, &seg_lb, &seg_rb],
                        [ila, ira, ilb, irb],
                    );
                } else {
                    formant::process_block(&mut self.state[a], &seg_la, &mut left_cx.io[a]);
                    formant::process_block(&mut right.state[a], &seg_ra, &mut right_cx.io[a]);
                    formant::process_block(&mut self.state[b], &seg_lb, &mut left_cx.io[b]);
                    formant::process_block(&mut right.state[b], &seg_rb, &mut right_cx.io[b]);
                }
            } else {
                formant::process_lanes(
                    [&mut self.state[a], &mut right.state[a]],
                    [&seg_la, &seg_ra],
                    [&mut left_cx.io[a], &mut right_cx.io[a]],
                );
            }
        }
    }

    fn copy_state_from(&mut self, other: &Self) {
        self.state = other.state;
        self.tuning = other.tuning;
        self.key = other.key;
        self.valid = other.valid;
    }

    fn subnormals(&self) -> usize {
        self.state.iter().filter(|s| s.has_subnormal()).count()
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

    /// Report 04's vowel table: `(name, F1, F2)` in Hz.
    const VOWELS: [(&str, f32, f32); 6] = [
        ("i", 240.0, 2_400.0),
        ("e", 390.0, 2_300.0),
        ("a", 850.0, 1_610.0),
        ("ɑ", 750.0, 940.0),
        ("o", 360.0, 640.0),
        ("u", 250.0, 595.0),
    ];

    /// Parameter tables for one voice with the given values, `cur == prev == base`.
    struct Rig {
        values: Vec<f32>,
        cur: Vec<[f32; MAX_VOICES]>,
        prev: Vec<[f32; MAX_VOICES]>,
        notes: NoteState,
        io: Box<VoiceBus>,
        filter: Formant,
    }

    impl Rig {
        fn new(set: &[(FormantParam, f32)]) -> Self {
            let mut values = ModuleKind::Formant.defaults();
            for &(p, v) in set {
                values[p.index()] = v;
            }
            let cur: Vec<[f32; MAX_VOICES]> = values.iter().map(|&v| [v; MAX_VOICES]).collect();
            let mut notes = NoteState::new();
            notes.gate[0] = true;
            notes.pitch_cur[0] = 60.0;
            notes.pitch_prev[0] = 60.0;
            let filter = Formant::new(&values, &PrepareCfg::new(SR));
            Self {
                prev: cur.clone(),
                values,
                cur,
                notes,
                io: Box::new([[0.0; SEG]; MAX_VOICES]),
                filter,
            }
        }

        fn set(&mut self, p: FormantParam, v: f32) {
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

    /// The local maxima of an impulse response's spectrum between 50 Hz and 12 kHz: `(Hz, magnitude)`.
    fn spectral_peaks(ir: &[f32]) -> Vec<(f64, f64)> {
        let mut re: Vec<f64> = ir.iter().map(|&v| f64::from(v)).collect();
        let mut im = vec![0.0; ir.len()];
        fft(&mut re, &mut im);
        let bin = f64::from(SR) / ir.len() as f64;
        let mag: Vec<f64> = (0..=ir.len() / 2)
            .map(|i| (re[i] * re[i] + im[i] * im[i]).sqrt())
            .collect();
        let (lo, hi) = ((50.0 / bin) as usize, (12_000.0 / bin) as usize);
        (lo + 1..hi)
            .filter(|&i| mag[i] > mag[i - 1] && mag[i] >= mag[i + 1])
            .map(|i| (i as f64 * bin, mag[i]))
            .collect()
    }

    /// The error in percent of the spectral peak nearest `target` (within a factor 1.5).
    fn error_at(peaks: &[(f64, f64)], target: f32) -> f64 {
        let t = f64::from(target);
        let best = peaks
            .iter()
            .filter(|p| p.0 > t / 1.5 && p.0 < t * 1.5)
            .min_by(|a, b| (a.0 - t).abs().total_cmp(&(b.0 - t).abs()))
            .expect("a peak near the formant");
        (best.0 / t - 1.0) * 100.0
    }

    #[test]
    fn a_static_filter_matches_the_dsp_core_and_caches_its_tuning() {
        let mut rig = Rig::new(&[(FormantParam::Resonance, 70.0), (FormantParam::Shift, 3.0)]);
        rig.start();
        let t = Tuning::new(850.0, 1_610.0, 3.0, 70.0, SR);
        let seg = Segment {
            from: t,
            to: t,
            mix: [1.0, 1.0],
        };
        let mut core = State::ZERO;
        let input: [f32; SEG] = core::array::from_fn(|i| sinf(i as f32 * 0.37));
        for _ in 0..10 {
            let got = rig.run(&input);
            let mut want = input;
            formant::process_block(&mut core, &seg, &mut want);
            assert_eq!(got, want);
        }
        assert_eq!(rig.filter.tuning[0], t);
    }

    #[test]
    fn report_04s_vowels_peak_where_f1_and_f2_say_through_the_module() {
        // Acceptance: for every vowel of the table, set through f1 and f2 at the default resonance, the spectrum of
        // the module's impulse response has its two peaks within 3 % of the table (they are within 1.1 %).
        for &(name, f1, f2) in &VOWELS {
            let mut rig = Rig::new(&[(FormantParam::F1, f1), (FormantParam::F2, f2)]);
            let ir = rig.impulse(131_072);
            let peaks = spectral_peaks(&ir);
            let (e1, e2) = (error_at(&peaks, f1), error_at(&peaks, f2));
            eprintln!("{name} {f1}/{f2}: F1 {e1:+.2} %, F2 {e2:+.2} %");
            assert!(e1.abs() < 3.0 && e2.abs() < 3.0, "{name}: {e1} {e2}");
        }
    }

    #[test]
    fn shift_moves_both_formants_and_the_third() {
        // +12 semitones doubles every centre: the "a" peaks at 1,700 and 3,220 Hz.
        let mut rig = Rig::new(&[(FormantParam::Shift, 12.0), (FormantParam::Resonance, 75.0)]);
        let ir = rig.impulse(131_072);
        let peaks = spectral_peaks(&ir);
        assert!(error_at(&peaks, 1_700.0).abs() < 1.5);
        assert!(error_at(&peaks, 3_220.0).abs() < 1.5);
    }

    #[test]
    fn a_change_ramps_from_the_previous_segment() {
        let mut rig = Rig::new(&[]);
        rig.start();
        let input = [0.5f32; SEG];
        let _ = rig.run(&input);
        let before = rig.filter.tuning[0];
        rig.set(FormantParam::F2, 700.0);
        let got = rig.run(&input);
        let to = Tuning::new(850.0, 700.0, 0.0, 50.0, SR);
        assert_eq!(rig.filter.tuning[0], to);
        let t0 = Tuning::new(850.0, 1_610.0, 0.0, 50.0, SR);
        assert_eq!(before, t0);
        let mut core = State::ZERO;
        let steady = Segment {
            from: t0,
            to: t0,
            mix: [1.0, 1.0],
        };
        let mut warm = input;
        formant::process_block(&mut core, &steady, &mut warm);
        let seg = Segment {
            from: t0,
            to,
            mix: [1.0, 1.0],
        };
        let mut want = input;
        formant::process_block(&mut core, &seg, &mut want);
        assert_eq!(got, want);
    }

    #[test]
    fn mix_zero_passes_the_input_bit_for_bit_and_a_new_voice_starts_clean() {
        let mut rig = Rig::new(&[(FormantParam::Mix, 0.0), (FormantParam::Resonance, 90.0)]);
        rig.start();
        let input: [f32; SEG] = core::array::from_fn(|i| ((i * 7) % 13) as f32 / 13.0 - 0.5);
        for _ in 0..50 {
            let out = rig.run(&input);
            assert_eq!(out.map(f32::to_bits), input.map(f32::to_bits));
        }
        // The filter has been running underneath: stepping the mix up now is not a cold start.
        assert!(rig.filter.state[0] != State::ZERO);
        rig.start();
        assert_eq!(rig.filter.state[0], State::ZERO);
        assert!(!rig.filter.valid[0]);
        let mut fresh = Rig::new(&[(FormantParam::Mix, 100.0), (FormantParam::Resonance, 90.0)]);
        fresh.start();
        rig.set(FormantParam::Mix, 100.0);
        rig.settle();
        assert_eq!(rig.run(&input), fresh.run(&input));
    }

    #[test]
    fn two_instances_keep_separate_state() {
        // Filter A alone, then A with B running between its segments on other input: A's output is the same bits.
        let settings = [(FormantParam::Resonance, 80.0), (FormantParam::F1, 300.0)];
        let mut alone = Rig::new(&settings);
        alone.start();
        let mut a = Rig::new(&settings);
        a.start();
        let mut b = Rig::new(&[(FormantParam::F2, 700.0), (FormantParam::Shift, -5.0)]);
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
        let mut a = Rig::new(&[(FormantParam::Resonance, 80.0), (FormantParam::Shift, -5.0)]);
        a.start();
        let input: [f32; SEG] = core::array::from_fn(|i| ((i * 7) % 13) as f32 / 13.0 - 0.5);
        for _ in 0..60 {
            let _ = a.run(&input);
        }
        let mut b = Rig::new(&[(FormantParam::Resonance, 80.0), (FormantParam::Shift, -5.0)]);
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
        // The acceptance stress through the module: a new random f1, f2, shift, resonance and mix every segment at
        // their catalogue extremes, noise in.
        let mut rig = Rig::new(&[]);
        rig.start();
        let mut rng = Rng::new(77);
        let mut peak = 0.0f32;
        for _ in 0..15_000 {
            rig.set(
                FormantParam::F1,
                100.0 * exp2f(rng.unit_f32() * log2f(10.0)),
            );
            rig.set(
                FormantParam::F2,
                400.0 * exp2f(rng.unit_f32() * log2f(10.0)),
            );
            rig.set(FormantParam::Shift, (rng.unit_f32() - 0.5) * 48.0);
            rig.set(FormantParam::Resonance, 100.0 * rng.unit_f32().sqrt());
            rig.set(FormantParam::Mix, 100.0 * rng.unit_f32());
            let input: [f32; SEG] = core::array::from_fn(|_| rng.bipolar_f32() * 0.5);
            for y in rig.run(&input) {
                assert!(y.is_finite());
                peak = peak.max(y.abs());
            }
        }
        assert!(peak < 4.0, "peak {peak}");
        // Park it and let it ring out: the state dies to exact zero, no subnormals.
        rig.set(FormantParam::F1, 850.0);
        rig.set(FormantParam::F2, 1_610.0);
        rig.set(FormantParam::Shift, 0.0);
        rig.set(FormantParam::Resonance, 100.0);
        rig.set(FormantParam::Mix, 100.0);
        for _ in 0..(48_000 * 30 / SEG) {
            let _ = rig.run(&[0.0; SEG]);
            assert_eq!(rig.filter.subnormals(), 0);
        }
        assert_eq!(rig.filter.state[0], State::ZERO);
    }

    /// `process_pair` on a formant filter and its twin gives, bit for bit, what `process` on each gives: through
    /// vowels that sweep and rest, shifts, resonances, mixes that switch between off, blended and full, a restarted
    /// voice, odd and even counts of voices, and a twin on other settings than the left (a stereo plan never does
    /// that).
    #[test]
    fn process_pair_is_two_process_calls_bit_for_bit() {
        use crate::modules::pair_test::check;
        let cfg = PrepareCfg::new(SR);
        for voices in [vec![0usize, 3, 4], vec![0, 1, 2, 5], vec![7], vec![2, 9]] {
            for twin_differs in [false, true] {
                check(
                    ModuleKind::Formant,
                    |values| Formant::new(values, &cfg),
                    &voices,
                    96,
                    |seg, right, t| {
                        let ix = |p: FormantParam| p.index();
                        let moving = (10..30).contains(&seg);
                        t.set(ix(FormantParam::F2), |v| {
                            let base = 900.0 + 120.0 * v as f32;
                            if moving {
                                base + 35.0 * (seg - 10) as f32 * (1 + v % 3) as f32
                            } else if seg >= 30 {
                                base + 500.0
                            } else {
                                base
                            }
                        });
                        t.all(ix(FormantParam::Shift), if seg >= 45 { 7.0 } else { 0.0 });
                        let res = if seg >= 40 { 90.0 } else { 40.0 };
                        t.all(
                            ix(FormantParam::Resonance),
                            if twin_differs && right {
                                res - 25.0
                            } else {
                                res
                            },
                        );
                        let mix = match seg {
                            80..=85 => 0.0,
                            86..=90 => 50.0,
                            _ => 100.0,
                        };
                        t.set(ix(FormantParam::Mix), |v| {
                            if twin_differs && right && v % 2 == 0 {
                                mix * 0.5
                            } else {
                                mix
                            }
                        });
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
