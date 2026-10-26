//! The `wt` module (S2a lane WT): a wavetable oscillator on phase-read waves, with phase modulation and amplitude
//! modulation from other generators through audio routes, and self-feedback. The DSP is `enth_dsp::wt`; the sources,
//! the formulas, the measured spectra and the aliasing bound are in `docs/dsp/wt.md`, the parameters in
//! `enth-patch/src/catalogue/wt.rs` (lane CAT's F1m table).
//!
//! - **The table.** `table` is structural: the plan fetches the named table from a process-wide cache in `new`
//!   (`Table::shared`), so a second `wt` on the same table, or a second plan, costs a refcount.
//! - **Pitch.** `pitch_cur + semi + fine/100` semitones at the end of the segment, `pitch_prev + …` at its start
//!   (glide and vibrato); `midi_to_hz` through the pinned `libm`; the phase increment ramps linearly between the two
//!   over the segment and is clamped to 0.45 of the sample rate, as `osc`'s sine and table.
//! - **Position, warp, level** ramp per sample from the previous segment's value to this one's. `warp` is read from
//!   `params.base` (a discrete parameter); every warp is the identity at `warp_amount` 0.
//! - **PM.** The phase offset in radians is `pm_index · (pm/100) · pm_in(t)`, with `pm_in` the summed audio routed
//!   into `pm` (`VoiceCx::audio_in`, lane RTE): each source's pre-level output times its route depth. With no route,
//!   nothing happens whatever the two values. The table is read at `phase + offset`.
//! - **AM and ring modulation.** The output is multiplied by `am/100 + am_in(t)`: 100 % and no route is unity, a
//!   route above 0 % is AM, `am` at 0 % with a route is ring modulation, and without a route `am` scales the output.
//! - **Feedback** is the oscillator's own output (before AM) delayed 1.5 samples (the mean of the last two) times
//!   up to 2 radians of phase, per `feedback/100` (the most at which the loop never rings).
//! - **Audio routes out.** When a route leaves this module (`VoiceCx::pre_level`), the output after AM and before the
//!   `level` ramp is copied into it for every active voice, so `level` 0 makes a silent FM operator that still
//!   modulates.
//! - **Phase modes.** As `osc`: `reset` (the default; PM, warps and feedback make the start phase part of the
//!   timbre) starts at `phase`, `free` draws a note's start phase from the voice seed, `seeded` from the module's
//!   own `seed`.
//! - **Unison** (`unison`, `detune`, `blend`, `spread`; lane UNI). `unison` is structural: a `wt` with 2 to 8 copies
//!   renders them on `wide` lanes (`enth_dsp::wt::UnisonVoice`), each copy a plain oscillator voice with its own
//!   phase, increment and history, and sums them in lane order. The count fixes a plan's stereo flag
//!   ([`crate::stereo::patch_needs_stereo`]): in a stereo plan the module writes the left channel to `io` and the
//!   right to `io_r`, the copies panned across `spread`; without a right bus (`StereoMode::Off`) it writes the mono
//!   mix, which is exactly the left channel at `spread` 0. **An audio route leaving a unison `wt` carries the mono
//!   mix**, after `am` and before `level`: a modulator is one signal, whatever the spread does to the listener's
//!   image. A copy's start phase comes from `derive(voice_seed, k)` (`free`), from `derive(module key, k)` plus the
//!   `phase` knob as a common rotation (`reset`: the same every note) or from `derive(seeded voice seed, k)`
//!   (`seeded`), never from a parameter value. `unison` 1 is the single oscillator above, bit for bit.
//!   `docs/dsp/wt.md` has the laws and the measurements.

use std::sync::Arc;

use enth_dsp::osc::{MAX_DT_PHASED, clamp_dt};
use enth_dsp::util::math::midi_to_hz;
use enth_dsp::util::rng::{Rng, derive};
use enth_dsp::wavetable::{FRAMES, alias_limit};
use enth_dsp::wt::{
    CYCLES_PER_RADIAN, FEEDBACK_CYCLES, LANES, Mix, Out, Segment, Table, UnisonSegment,
    UnisonVoice, Voice, Warp as DspWarp, WtKind,
};
use enth_patch::catalogue::{PhaseMode, Warp, WtParam, WtTable};

use crate::module::{
    Block, MAX_VOICES, PrepareCfg, RAMP, SEG, VoiceCx, VoiceParams, VoiceProcessor, VoiceStart,
};
use crate::seed::seeded_voice_seed;

/// The wavetable oscillator.
#[derive(Clone, Debug)]
pub struct Wt {
    sample_rate: f32,
    alias_limit: f32,
    table: Arc<Table>,
    /// The copies per voice, 1 to 8, from the structural `unison` parameter (1 is off).
    unison: usize,
    /// The single oscillator of each voice (`unison` 1).
    voice: [Voice; MAX_VOICES],
    /// The copies of each voice (`unison` of 2 or more).
    lanes: [UnisonVoice; MAX_VOICES],
    /// Each voice's mix at the end of its last segment, with the values it was made from.
    mix: [Option<CachedMix>; MAX_VOICES],
}

/// A [`Mix`] with the key it was made from: `[count, detune, blend, spread]` as bits. The mix is a function of the
/// key alone, so a key that matches is a hit whatever happened in between and a stale entry cannot be wrong.
#[derive(Clone, Copy, Debug)]
struct CachedMix {
    key: [u32; 4],
    mix: Mix,
}

/// The number of copies the structural `unison` value in `values` (a patch's parameter values for a `wt`) asks for,
/// 1 to 8; 1 when the value is missing. [`crate::stereo::patch_needs_stereo`] and [`Wt::new`] both read it here.
#[must_use]
pub(crate) fn unison_count(values: &[f32]) -> usize {
    values.get(WtParam::Unison.index()).map_or(1, |&u| {
        (u.clamp(1.0, LANES as f32).round() as usize).clamp(1, LANES)
    })
}

/// The table the catalogue's `table` enum names.
fn table_kind(table: WtTable) -> WtKind {
    match table {
        WtTable::Basic => WtKind::Basic,
        WtTable::Fm => WtKind::Fm,
        WtTable::FmOctave => WtKind::FmOctave,
        WtTable::Formant => WtKind::Formant,
        WtTable::Metal => WtKind::Metal,
    }
}

/// The warp the catalogue's `warp` enum names.
fn dsp_warp(warp: Warp) -> DspWarp {
    match warp {
        Warp::Off => DspWarp::Off,
        Warp::Bend => DspWarp::Bend,
        Warp::Asymmetric => DspWarp::Asymmetric,
        Warp::Squeeze => DspWarp::Squeeze,
        Warp::Sync => DspWarp::Sync,
        Warp::Quantize => DspWarp::Quantize,
    }
}

impl Wt {
    /// Prepare a `wt` generator: fetch (or generate, once per process) the table the structural `table` names.
    #[must_use]
    pub fn new(values: &[f32], cfg: &PrepareCfg) -> Self {
        let table = values
            .get(WtParam::Table.index())
            .and_then(|&v| WtTable::from_value(v))
            .unwrap_or(WtTable::Basic);
        Self {
            sample_rate: cfg.sample_rate,
            alias_limit: alias_limit(cfg.sample_rate),
            table: Table::shared(table_kind(table)),
            unison: unison_count(values),
            voice: [Voice::new(0.0); MAX_VOICES],
            lanes: [UnisonVoice::new([0.0; LANES]); MAX_VOICES],
            mix: [None; MAX_VOICES],
        }
    }

    /// Whether this `wt` makes a stereo signal per voice: it has 2 or more copies. In a plan with the stereo voice
    /// path it is given a right channel.
    #[must_use]
    pub fn is_stereo(&self) -> bool {
        self.unison >= 2
    }

    /// The copies of the oscillator per voice, 1 to 8.
    #[must_use]
    pub fn copies(&self) -> usize {
        self.unison
    }

    /// The mix of voice `v` at the start and at the end of the segment, from the smoothed parameters. A key that
    /// matches the cache costs nothing: the end of one segment is the start of the next.
    fn mixes(&mut self, v: usize, p: &VoiceParams<'_>) -> [Mix; 2] {
        let n = self.unison;
        let key = |detune: f32, blend: f32, spread: f32| {
            [
                n as u32,
                detune.to_bits(),
                blend.to_bits(),
                spread.to_bits(),
            ]
        };
        let k0 = key(
            p.prev(WtParam::Detune, v),
            p.prev(WtParam::Blend, v),
            p.prev(WtParam::Spread, v),
        );
        let k1 = key(
            p.cur(WtParam::Detune, v),
            p.cur(WtParam::Blend, v),
            p.cur(WtParam::Spread, v),
        );
        let build = |detune: f32, blend: f32, spread: f32| {
            Mix::new(n, detune, blend / 100.0, spread / 100.0)
        };
        let cached = self.mix[v];
        let m0 = match cached {
            Some(c) if c.key == k0 => c.mix,
            _ => build(
                p.prev(WtParam::Detune, v),
                p.prev(WtParam::Blend, v),
                p.prev(WtParam::Spread, v),
            ),
        };
        let m1 = if k1 == k0 {
            m0
        } else {
            match cached {
                Some(c) if c.key == k1 => c.mix,
                _ => build(
                    p.cur(WtParam::Detune, v),
                    p.cur(WtParam::Blend, v),
                    p.cur(WtParam::Spread, v),
                ),
            }
        };
        self.mix[v] = Some(CachedMix { key: k1, mix: m1 });
        [m0, m1]
    }

    /// The table this oscillator plays.
    #[must_use]
    pub fn table(&self) -> &Table {
        &self.table
    }

    /// Phase increment (cycles per sample) for a pitch in semitones.
    fn dt(&self, pitch: f32) -> f32 {
        clamp_dt(midi_to_hz(pitch) / self.sample_rate, MAX_DT_PHASED)
    }
}

/// Start phase in `[0, 1)` for one note.
fn start_phase(start: &VoiceStart<'_>) -> f32 {
    let p = &start.params;
    match PhaseMode::from_value(p.base(WtParam::PhaseMode)) {
        Some(PhaseMode::Seeded) => {
            let seed = p.base(WtParam::Seed) as u32;
            Rng::new(seeded_voice_seed(seed, start.module_key, start.note_index)).unit_f32()
        }
        Some(PhaseMode::Free) => Rng::new(start.seed).unit_f32(),
        Some(PhaseMode::Reset) | None => {
            let turns = p.base(WtParam::Phase) / 360.0;
            if turns >= 1.0 { turns - 1.0 } else { turns }
        }
    }
}

/// The start phases of a unison voice's copies, one per lane (the lanes beyond the count are drawn too, so the draw
/// of copy `k` never depends on how many copies there are). Each copy's phase is drawn from a stream that is a
/// function of the seed rules only, never of a parameter value:
///
/// - `free`: `derive(voice_seed, k)`, which differs per note and per render seed;
/// - `seeded`: `derive(seeded_voice_seed(seed, module_key, note_index), k)`, from the patch's own `seed`;
/// - `reset`: `derive(module_key, k)`, the same every note and every render seed, so the attack is repeatable;
///   the `phase` knob then turns all copies together (a rotation, not a seed), so the knob that sets the timbre of a
///   single `wt` keeps its meaning.
fn copy_phases(start: &VoiceStart<'_>) -> [f32; LANES] {
    let p = &start.params;
    let mode = PhaseMode::from_value(p.base(WtParam::PhaseMode));
    let turns = {
        let t = p.base(WtParam::Phase) / 360.0;
        if t >= 1.0 { t - 1.0 } else { t }
    };
    let seeded = seeded_voice_seed(
        p.base(WtParam::Seed) as u32,
        start.module_key,
        start.note_index,
    );
    let mut phases = [0.0; LANES];
    for (k, phase) in phases.iter_mut().enumerate() {
        let k = k as u64;
        *phase = match mode {
            Some(PhaseMode::Seeded) => Rng::new(derive(seeded, k)).unit_f32(),
            Some(PhaseMode::Free) => Rng::new(derive(start.seed, k)).unit_f32(),
            Some(PhaseMode::Reset) | None => {
                let x = Rng::new(derive(start.module_key, k)).unit_f32() + turns;
                if x >= 1.0 { x - 1.0 } else { x }
            }
        };
    }
    phases
}

/// `out[i] *= g0 + (g1 − g0)·RAMP[i]`, skipped when the gain is exactly 1 throughout.
#[inline(always)]
fn apply_gain(out: &mut Block, g0: f32, g1: f32) {
    if g0 == 1.0 && g1 == 1.0 {
        return;
    }
    let step = g1 - g0;
    for (o, r) in out.iter_mut().zip(RAMP) {
        *o *= g0 + step * r;
    }
}

/// `out[i] *= am(i)`: the AM gain `a0 + (a1 − a0)·RAMP[i]`, plus the routed audio `am_in[i]` when a route reaches
/// `am`. Without a route the gain is `am` alone (and skipped when exactly 1).
#[inline(always)]
fn apply_am(out: &mut Block, a0: f32, a1: f32, am_in: Option<&Block>) {
    match am_in {
        Some(am) => {
            let step = a1 - a0;
            for ((o, r), m) in out.iter_mut().zip(RAMP).zip(am) {
                *o *= a0 + step * r + m;
            }
        }
        None => apply_gain(out, a0, a1),
    }
}

impl VoiceProcessor for Wt {
    fn start_voice(&mut self, start: &VoiceStart<'_>) {
        self.voice[start.voice] = Voice::new(start_phase(start));
        if self.unison >= 2 {
            self.lanes[start.voice] = UnisonVoice::new(copy_phases(start));
        }
    }

    fn process(&mut self, cx: &mut VoiceCx<'_>) {
        let p = cx.params;
        let warp = dsp_warp(Warp::from_value(p.base(WtParam::Warp)).unwrap_or(Warp::Off));
        let frame_scale = 0.01 * (FRAMES - 1) as f32;
        let mut pm_in: Block = [0.0; SEG];
        let mut am_in: Block = [0.0; SEG];
        for &v in cx.voices {
            let pitch1 =
                cx.notes.pitch_cur[v] + p.cur(WtParam::Semi, v) + p.cur(WtParam::Fine, v) * 0.01;
            let pitch0 =
                cx.notes.pitch_prev[v] + p.prev(WtParam::Semi, v) + p.prev(WtParam::Fine, v) * 0.01;
            let dt1 = self.dt(pitch1);
            let dt0 = if pitch0.to_bits() == pitch1.to_bits() {
                dt1
            } else {
                self.dt(pitch0)
            };
            // The audio routed into pm and am (lane RTE): the sum of each source's pre-level output times its depth.
            let (has_pm, has_am) = match cx.audio_in.as_ref() {
                Some(audio) => (
                    audio.read(WtParam::Pm.index(), v, &mut pm_in),
                    audio.read(WtParam::Am.index(), v, &mut am_in),
                ),
                None => (false, false),
            };
            let mix = (self.unison >= 2).then(|| self.mixes(v, &p));
            // Radians of phase per unit of input -> cycles.
            let pm_gain = |index: f32, trim: f32| index * (trim * 0.01) * CYCLES_PER_RADIAN;
            let seg = Segment {
                table: &self.table,
                limit: self.alias_limit,
                dt: [dt0, dt1],
                frame_pos: [
                    p.prev(WtParam::Position, v) * frame_scale,
                    p.cur(WtParam::Position, v) * frame_scale,
                ],
                warp,
                warp_amount: [
                    p.prev(WtParam::WarpAmount, v) * 0.01,
                    p.cur(WtParam::WarpAmount, v) * 0.01,
                ],
                pm: has_pm.then_some(&pm_in),
                pm_gain: [
                    pm_gain(p.prev(WtParam::PmIndex, v), p.prev(WtParam::Pm, v)),
                    pm_gain(p.cur(WtParam::PmIndex, v), p.cur(WtParam::Pm, v)),
                ],
                feedback: [
                    p.prev(WtParam::Feedback, v) * (0.01 * FEEDBACK_CYCLES),
                    p.cur(WtParam::Feedback, v) * (0.01 * FEEDBACK_CYCLES),
                ],
            };
            // AM: the gain is `am/100 + am_in(t)`, ramped like every parameter. With no route it is `am` alone.
            let (a0, a1) = (p.prev(WtParam::Am, v) * 0.01, p.cur(WtParam::Am, v) * 0.01);
            let am = has_am.then_some(&am_in);
            let (l0, l1) = (
                p.prev(WtParam::Level, v) * 0.01,
                p.cur(WtParam::Level, v) * 0.01,
            );

            let Some(mix) = mix else {
                let out = &mut cx.io[v];
                self.voice[v].render(&seg, out);
                apply_am(out, a0, a1, am);
                // An audio route leaves this module (lane RTE): what travels is the output before the level ramp.
                if let Some(pre) = cx.pre_level.as_deref_mut() {
                    pre[v] = *out;
                }
                apply_gain(out, l0, l1);
                continue;
            };

            // Unison (lane UNI): the copies on lanes. The routes carry the mono mix, whatever the spread does.
            let useg = UnisonSegment { seg, mix };
            if let Some(io_r) = cx.io_r.as_deref_mut() {
                let mut mid = cx.pre_level.as_deref_mut().map(|pre| &mut pre[v]);
                let (left, right) = (&mut cx.io[v], &mut io_r[v]);
                self.lanes[v].render(
                    &useg,
                    Out::Stereo {
                        left: &mut *left,
                        right: &mut *right,
                        mid: mid.as_deref_mut(),
                    },
                );
                apply_am(left, a0, a1, am);
                apply_am(right, a0, a1, am);
                if let Some(mid) = mid {
                    apply_am(mid, a0, a1, am);
                }
                apply_gain(left, l0, l1);
                apply_gain(right, l0, l1);
            } else {
                let out = &mut cx.io[v];
                self.lanes[v].render(&useg, Out::Mono(out));
                apply_am(out, a0, a1, am);
                if let Some(pre) = cx.pre_level.as_deref_mut() {
                    pre[v] = *out;
                }
                apply_gain(out, l0, l1);
            }
        }
    }

    fn copy_state_from(&mut self, other: &Self) {
        self.voice = other.voice;
        self.lanes = other.lanes;
        self.mix = other.mix;
        // A change of the copy count is a structural edit. Keep the sound continuous in phase at least: from one
        // oscillator to copies, every copy starts at the oscillator's phase (and drifts apart by its detune); from
        // copies to one oscillator, the oscillator continues from the centre copy.
        if self.unison != other.unison {
            if other.unison < 2 {
                for (lane, voice) in self.lanes.iter_mut().zip(&other.voice) {
                    *lane = UnisonVoice::new([voice.phase(); LANES]);
                }
            } else if self.unison < 2 {
                let centre = other.unison / 2;
                for (voice, lane) in self.voice.iter_mut().zip(&other.lanes) {
                    *voice = Voice::new(lane.phase(centre));
                }
            }
        }
    }

    fn subnormals(&self) -> usize {
        self.voice.iter().filter(|v| v.has_subnormal()).count()
            + self.lanes.iter().filter(|v| v.has_subnormal()).count()
    }
}
