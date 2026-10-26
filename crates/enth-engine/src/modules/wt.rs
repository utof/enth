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
//! - **Unison** (`unison`, `detune`, `blend`, `spread`) is lane UNI's and does nothing yet: any `unison` renders as 1.

use std::sync::Arc;

use enth_dsp::osc::{MAX_DT_PHASED, clamp_dt};
use enth_dsp::util::math::midi_to_hz;
use enth_dsp::util::rng::Rng;
use enth_dsp::wavetable::{FRAMES, alias_limit};
use enth_dsp::wt::{
    CYCLES_PER_RADIAN, FEEDBACK_CYCLES, Segment, Table, Voice, Warp as DspWarp, WtKind,
};
use enth_patch::catalogue::{PhaseMode, Warp, WtParam, WtTable};

use crate::module::{
    Block, MAX_VOICES, PrepareCfg, RAMP, SEG, VoiceCx, VoiceProcessor, VoiceStart,
};
use crate::seed::seeded_voice_seed;

/// The wavetable oscillator.
#[derive(Clone, Debug)]
pub struct Wt {
    sample_rate: f32,
    alias_limit: f32,
    table: Arc<Table>,
    voice: [Voice; MAX_VOICES],
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
            voice: [Voice::new(0.0); MAX_VOICES],
        }
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

impl VoiceProcessor for Wt {
    fn start_voice(&mut self, start: &VoiceStart<'_>) {
        self.voice[start.voice] = Voice::new(start_phase(start));
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
            let out = &mut cx.io[v];
            self.voice[v].render(&seg, out);

            // AM: the gain is `am/100 + am_in(t)`, ramped like every parameter. With no route it is `am` alone.
            let (a0, a1) = (p.prev(WtParam::Am, v) * 0.01, p.cur(WtParam::Am, v) * 0.01);
            if has_am {
                let step = a1 - a0;
                for ((o, r), m) in out.iter_mut().zip(RAMP).zip(&am_in) {
                    *o *= a0 + step * r + m;
                }
            } else {
                apply_gain(out, a0, a1);
            }
            // An audio route leaves this module (lane RTE): what travels is the output before the level ramp.
            if let Some(pre) = cx.pre_level.as_deref_mut() {
                pre[v] = *out;
            }
            apply_gain(
                out,
                p.prev(WtParam::Level, v) * 0.01,
                p.cur(WtParam::Level, v) * 0.01,
            );
        }
    }

    fn copy_state_from(&mut self, other: &Self) {
        self.voice = other.voice;
    }

    fn subnormals(&self) -> usize {
        self.voice.iter().filter(|v| v.has_subnormal()).count()
    }
}
