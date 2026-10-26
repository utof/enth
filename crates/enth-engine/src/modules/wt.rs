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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::module::NoteState;
    use crate::plan::{Plan, compile};
    use crate::seed::{module_key, voice_seed};
    use enth_patch::catalogue::PhaseMode as Phase;
    use enth_patch::{ModuleId, ModuleInstance, ModuleKind, Patch};

    const SEED: u64 = 0x0E17_5EED;

    /// A patch with one `wt1`, `set` applied.
    fn patch(set: &[(WtParam, f32)]) -> Patch {
        let mut p = Patch::init();
        p.generators.clear();
        let mut m = ModuleInstance::new(ModuleId::new("wt1").unwrap(), ModuleKind::Wt);
        for &(param, value) in set {
            m.values[param.index()] = value;
        }
        p.generators.push(m);
        p
    }

    fn plan(set: &[(WtParam, f32)]) -> Plan {
        compile(&patch(set), &PrepareCfg::new(48_000.0)).unwrap().0
    }

    fn notes(pitch: f32) -> NoteState {
        let mut n = NoteState::new();
        n.pitch_prev = [pitch; MAX_VOICES];
        n.pitch_cur = [pitch; MAX_VOICES];
        n.velocity = [0.8; MAX_VOICES];
        n.gate = [true; MAX_VOICES];
        n.fresh[0] = true;
        n
    }

    /// Start voice 0 of `plan`'s `wt1` as note `note_index` of a render with `render_seed`; the copies' start phases.
    fn phases(plan: &mut Plan, render_seed: u64, note_index: u64) -> [f32; LANES] {
        let mut n = notes(60.0);
        n.note_index[0] = note_index;
        let Plan {
            generators,
            voice_table,
            ..
        } = plan;
        let slot = &mut generators[0];
        let start = VoiceStart {
            voice: 0,
            seed: voice_seed(render_seed, slot.key, note_index),
            module_key: slot.key,
            note_index,
            notes: &n,
            params: voice_table.view(slot.params),
        };
        slot.module.start_voice(&start);
        let crate::modules::VoiceProcessorImpl::Wt(wt) = &slot.module else {
            panic!("a wt");
        };
        core::array::from_fn(|k| wt.lanes[0].phase(k))
    }

    fn mode(m: Phase) -> (WtParam, f32) {
        (WtParam::PhaseMode, m.index() as f32)
    }

    /// Every parameter that is not a seed rule's input, moved to a different value from the reference's: the count,
    /// detune, blend, spread, level, position, pitch, warp, pm, am, feedback.
    fn variants() -> Vec<Vec<(WtParam, f32)>> {
        vec![
            vec![(WtParam::Unison, 2.0)],
            vec![(WtParam::Unison, 8.0), (WtParam::Detune, 100.0)],
            vec![
                (WtParam::Unison, 5.0),
                (WtParam::Detune, 0.0),
                (WtParam::Blend, 0.0),
            ],
            vec![
                (WtParam::Unison, 7.0),
                (WtParam::Spread, 100.0),
                (WtParam::Level, 10.0),
            ],
            vec![
                (WtParam::Unison, 3.0),
                (WtParam::Detune, 63.0),
                (WtParam::Position, 80.0),
                (WtParam::Semi, 7.0),
                (WtParam::Fine, -20.0),
                (WtParam::Feedback, 50.0),
                (WtParam::PmIndex, 5.0),
                (WtParam::Am, 30.0),
            ],
        ]
    }

    #[test]
    fn a_copys_start_phase_never_depends_on_a_parameter_value() {
        for m in [Phase::Reset, Phase::Free, Phase::Seeded] {
            let mut reference_plan =
                plan(&[mode(m), (WtParam::Seed, 77.0), (WtParam::Unison, 4.0)]);
            let reference = phases(&mut reference_plan, SEED, 3);
            for set in variants() {
                let mut set = set;
                set.push(mode(m));
                set.push((WtParam::Seed, 77.0));
                let mut p = plan(&set);
                assert_eq!(phases(&mut p, SEED, 3), reference, "{m:?} with {set:?}");
            }
            // The eight draws are all different and inside [0, 1).
            for (i, a) in reference.iter().enumerate() {
                assert!((0.0..1.0).contains(a), "{m:?}: copy {i} starts at {a}");
                for b in &reference[i + 1..] {
                    assert_ne!(
                        a.to_bits(),
                        b.to_bits(),
                        "{m:?}: two copies share a start phase"
                    );
                }
            }
        }
    }

    #[test]
    fn the_start_phases_follow_the_seed_rules_and_nothing_else() {
        let id_key = module_key(&ModuleId::new("wt1").unwrap());
        // Reset: from the module id and the copy, so the same every note and every render seed.
        let mut p = plan(&[mode(Phase::Reset), (WtParam::Unison, 4.0)]);
        let a = phases(&mut p, SEED, 0);
        assert_eq!(phases(&mut p, SEED, 9), a, "the same every note");
        assert_eq!(phases(&mut p, 1, 0), a, "the same at every render seed");
        for (k, &x) in a.iter().enumerate() {
            let want = Rng::new(derive(id_key, k as u64)).unit_f32();
            assert_eq!(
                x.to_bits(),
                want.to_bits(),
                "copy {k} is derive(module key, k)"
            );
        }
        // ... and the phase knob turns them together, exactly.
        let mut q = plan(&[
            mode(Phase::Reset),
            (WtParam::Unison, 4.0),
            (WtParam::Phase, 90.0),
        ]);
        let b = phases(&mut q, SEED, 0);
        for k in 0..LANES {
            let mut want = a[k] + 0.25;
            if want >= 1.0 {
                want -= 1.0;
            }
            assert_eq!(
                b[k].to_bits(),
                want.to_bits(),
                "copy {k} rotated by the knob"
            );
        }

        // Free: derive(voice seed, k): a new note or a new render seed draws again.
        let mut f = plan(&[mode(Phase::Free), (WtParam::Unison, 4.0)]);
        let a = phases(&mut f, SEED, 2);
        for (k, &x) in a.iter().enumerate() {
            let want = Rng::new(derive(voice_seed(SEED, id_key, 2), k as u64)).unit_f32();
            assert_eq!(
                x.to_bits(),
                want.to_bits(),
                "copy {k} is derive(voice seed, k)"
            );
        }
        assert_ne!(phases(&mut f, SEED, 3), a, "another note");
        assert_ne!(phases(&mut f, SEED + 1, 2), a, "another render seed");

        // Seeded: from the patch's own seed, so the render seed does not move it; another `seed` does.
        let mut s = plan(&[
            mode(Phase::Seeded),
            (WtParam::Seed, 5.0),
            (WtParam::Unison, 4.0),
        ]);
        let a = phases(&mut s, SEED, 2);
        assert_eq!(
            phases(&mut s, 1234, 2),
            a,
            "the render seed does not matter"
        );
        let mut t = plan(&[
            mode(Phase::Seeded),
            (WtParam::Seed, 6.0),
            (WtParam::Unison, 4.0),
        ]);
        assert_ne!(phases(&mut t, SEED, 2), a, "another seed");
        for (k, &x) in a.iter().enumerate() {
            let want = Rng::new(derive(seeded_voice_seed(5, id_key, 2), k as u64)).unit_f32();
            assert_eq!(
                x.to_bits(),
                want.to_bits(),
                "copy {k} is derive(seeded voice seed, k)"
            );
        }
    }

    #[test]
    fn a_unison_wt_starts_its_first_sample_at_the_table_with_no_latency() {
        // The sine frame, the copies' start phases from the module id: the first sample is the mix of the sines at
        // those phases, to the table's interpolation error.
        let mut p = plan(&[
            mode(Phase::Reset),
            (WtParam::Unison, 3.0),
            (WtParam::Detune, 25.0),
            (WtParam::Blend, 60.0),
        ]);
        let start = phases(&mut p, SEED, 0);
        let mix = Mix::new(3, 25.0, 0.6, 0.5);
        let Plan {
            generators,
            voice_table,
            ..
        } = &mut p;
        let slot = &mut generators[0];
        let n = notes(48.0);
        let mut io = Box::new([[0.0f32; SEG]; MAX_VOICES]);
        slot.module.process(&mut VoiceCx {
            voices: &[0],
            notes: &n,
            params: voice_table.view(slot.params),
            io: &mut io,
            sync_out: None,
            sync_in: None,
            audio_in: None,
            pre_level: None,
            io_r: None,
        });
        let want: f32 = (0..3)
            .map(|k| {
                mix.mono[k]
                    * enth_dsp::util::math::sin(core::f64::consts::TAU * f64::from(start[k])) as f32
            })
            .sum();
        assert!(
            (io[0][0] - want).abs() < 2e-4,
            "first sample {} against {want}",
            io[0][0]
        );
        assert!(
            want.abs() > 0.05,
            "the start phases make the check meaningful"
        );
    }

    #[test]
    fn the_count_is_read_from_the_structural_value_and_clamped() {
        assert_eq!(unison_count(&[]), 1);
        let mut v = patch(&[]).generators[0].values.clone();
        for (value, want) in [
            (1.0, 1),
            (2.0, 2),
            (8.0, 8),
            (9.0, 8),
            (0.0, 1),
            (f32::NAN, 1),
        ] {
            v[WtParam::Unison.index()] = value;
            assert_eq!(unison_count(&v), want, "unison {value}");
        }
    }
}
