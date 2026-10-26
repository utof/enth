//! The `osc` module (P1a lane D1): saw, triangle and pulse from the Plaits-derived polyBLEP/BLAMP core, a sine, and
//! the wavetable mode, with hard sync and the three phase modes. The DSP is in `enth_dsp::{osc, wavetable}`; sources,
//! deviations and measured numbers are in `docs/dsp/osc.md`.
//!
//! - **Wave classes.** `wave` is automatable, so a voice can change class (saw, triangle, pulse, sine, table) while a
//!   note sounds. The phase carries over; the new class starts from it (`Shape::new` / `Phased::new`), so the only
//!   click is the change of waveform itself. `wave` is read from `params.base` (a discrete parameter).
//! - **The table.** `table` is structural: the plan generates the named table in `new` even while `wave` is not
//!   `table`, so automating `wave` to `table` later finds it ready. Tables come from a process-wide cache
//!   (`Wavetable::shared`), so a second oscillator on the same table, or a second plan, costs a refcount.
//! - **Pitch.** `pitch_cur + semi + fine/100` semitones at the end of the segment, `pitch_prev + …` at its start
//!   (glide and vibrato); `midi_to_hz` through the pinned `libm`; the phase increment ramps linearly between the two
//!   over the segment. The increment is clamped to a quarter of the sample rate for saw, triangle and pulse (upstream's
//!   `kMaxFrequency`, 12 kHz at 48 kHz) and to 0.45 of it for sine and table.
//! - **Level** ramps per sample from the previous segment's value to this one's.
//! - **Hard sync.** The first enabled `osc` (the *master*) writes, for every frame where its phase wraps, the
//!   fraction of the frame that remained after the wrap into `VoiceCx::sync_out` (Plaits' `reset_time`, `[0, 1)`;
//!   [`NO_SYNC`] elsewhere). An `osc` with `sync` on reads `VoiceCx::sync_in` and restarts there, band-limited. The
//!   master ignores its own `sync`. Any wave can be the master; only `osc` modules take part.
//! - **Phase modes.** `free` draws a note's start phase from the voice seed (render seed, module id, note index);
//!   `reset` starts at `phase`; `seeded` draws it from the module's own `seed` (`crate::seed::seeded_voice_seed`).
//!   The start phase is applied when the voice first renders, so it is the same whatever the order of parameter events
//!   in the segment that started the note. A legato retrigger does not call `start_voice` and so keeps the phase.
//! - **One sample of latency.** The polyBLEP core, the sine and the table all output the previous step's sum, so
//!   this module is one sample late relative to its input timing (every wave alike, so oscillators stay aligned).

use std::sync::Arc;

use enth_dsp::osc::{
    MAX_DT_ANALOG, MAX_DT_PHASED, PULSE, Phased, SAW, Shape, TRIANGLE, clamp_dt, clamp_pw, sine,
};
use enth_dsp::util::math::midi_to_hz;
use enth_dsp::util::rng::Rng;
use enth_dsp::wavetable::{FRAMES, TableKind, Wavetable, alias_limit, level_for};
use enth_patch::catalogue::{OscParam, PhaseMode, Wave, Wavetable as CatalogueTable};

use crate::module::{
    Block, MAX_VOICES, PrepareCfg, RAMP, SEG, VoiceCx, VoiceProcessor, VoiceStart,
};
use crate::seed::seeded_voice_seed;

/// What a voice renders with: the state it keeps depends on the wave.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Class {
    Saw,
    Triangle,
    Pulse,
    Sine,
    Table,
}

impl Class {
    fn of(wave: Wave) -> Self {
        match wave {
            Wave::Saw => Class::Saw,
            Wave::Triangle => Class::Triangle,
            Wave::Pulse => Class::Pulse,
            Wave::Sine => Class::Sine,
            Wave::Table => Class::Table,
        }
    }
}

/// The oscillator.
#[derive(Clone, Debug)]
pub struct Osc {
    sample_rate: f32,
    alias_limit: f32,
    table: Arc<Wavetable>,
    /// `None` for a voice that has started but not yet rendered: the first `process` builds its state from that
    /// segment's parameters.
    class: [Option<Class>; MAX_VOICES],
    /// The start phase drawn by `start_voice`, used by the first `process`.
    phase0: [f32; MAX_VOICES],
    shape: [Shape; MAX_VOICES],
    phased: [Phased; MAX_VOICES],
}

/// The table the catalogue's `table` enum names.
fn table_kind(table: CatalogueTable) -> TableKind {
    match table {
        CatalogueTable::Basic => TableKind::Basic,
        CatalogueTable::Pulse => TableKind::Pulse,
        CatalogueTable::Tilt => TableKind::Tilt,
        CatalogueTable::Formant => TableKind::Formant,
        CatalogueTable::Organ => TableKind::Organ,
    }
}

impl Osc {
    /// Prepare an oscillator: generate (or fetch from the cache) the table named by the structural `table`
    /// parameter, whatever `wave` is now.
    #[must_use]
    pub fn new(values: &[f32], cfg: &PrepareCfg) -> Self {
        let table = values
            .get(OscParam::Table.index())
            .and_then(|&v| CatalogueTable::from_value(v))
            .unwrap_or(CatalogueTable::Basic);
        Self {
            sample_rate: cfg.sample_rate,
            alias_limit: alias_limit(cfg.sample_rate),
            table: Wavetable::shared(table_kind(table)),
            class: [None; MAX_VOICES],
            phase0: [0.0; MAX_VOICES],
            shape: [Shape::default(); MAX_VOICES],
            phased: [Phased::default(); MAX_VOICES],
        }
    }

    /// The table this oscillator can play.
    #[must_use]
    pub fn table(&self) -> &Wavetable {
        &self.table
    }

    /// Phase increment (cycles per sample) for a pitch in semitones.
    fn dt(&self, pitch: f32, max: f32) -> f32 {
        clamp_dt(midi_to_hz(pitch) / self.sample_rate, max)
    }

    /// The phase the voice has now, in whichever state its class keeps it.
    fn phase_of(&self, v: usize) -> f32 {
        match self.class[v] {
            None => self.phase0[v],
            Some(Class::Saw | Class::Triangle | Class::Pulse) => self.shape[v].phase(),
            Some(Class::Sine | Class::Table) => self.phased[v].phase(),
        }
    }

    /// Start (or switch to) `class` at `phase` with this segment's pulse width and table position.
    fn enter(&mut self, v: usize, class: Class, phase: f32, pw: f32, pos: f32, dt: f32) {
        match class {
            Class::Saw => self.shape[v] = Shape::new(SAW, phase, 0.5),
            Class::Triangle => self.shape[v] = Shape::new(TRIANGLE, phase, 0.5),
            Class::Pulse => self.shape[v] = Shape::new(PULSE, phase, pw),
            Class::Sine => self.phased[v] = Phased::new(phase, sine(phase)),
            Class::Table => {
                let level = level_for(dt, self.alias_limit);
                self.phased[v] = Phased::new(phase, self.table.sample(level, pos, phase));
            }
        }
        self.class[v] = Some(class);
    }
}

/// Start phase in `[0, 1)` for one note.
fn start_phase(start: &VoiceStart<'_>) -> f32 {
    let p = &start.params;
    match PhaseMode::from_value(p.base(OscParam::PhaseMode)) {
        Some(PhaseMode::Reset) => {
            let turns = p.base(OscParam::Phase) / 360.0;
            if turns >= 1.0 { turns - 1.0 } else { turns }
        }
        Some(PhaseMode::Seeded) => {
            let seed = p.base(OscParam::Seed) as u32;
            Rng::new(seeded_voice_seed(seed, start.module_key, start.note_index)).unit_f32()
        }
        Some(PhaseMode::Free) | None => Rng::new(start.seed).unit_f32(),
    }
}

/// `out[i] *= g0 + (g1 − g0)·RAMP[i]`, skipped when the gain is exactly 1 throughout.
#[inline(always)]
fn apply_level(out: &mut Block, g0: f32, g1: f32) {
    if g0 == 1.0 && g1 == 1.0 {
        return;
    }
    let step = g1 - g0;
    for (o, r) in out.iter_mut().zip(RAMP) {
        *o *= g0 + step * r;
    }
}

impl VoiceProcessor for Osc {
    fn start_voice(&mut self, start: &VoiceStart<'_>) {
        let v = start.voice;
        self.phase0[v] = start_phase(start);
        self.class[v] = None;
    }

    fn process(&mut self, cx: &mut VoiceCx<'_>) {
        let p = cx.params;
        let wave = Wave::from_value(p.base(OscParam::Wave)).unwrap_or(Wave::Saw);
        let class = Class::of(wave);
        let sync_on = p.base(OscParam::Sync) >= 0.5;
        let analog = matches!(class, Class::Saw | Class::Triangle | Class::Pulse);
        let max_dt = if analog { MAX_DT_ANALOG } else { MAX_DT_PHASED };
        for &v in cx.voices {
            let pitch1 =
                cx.notes.pitch_cur[v] + p.cur(OscParam::Semi, v) + p.cur(OscParam::Fine, v) * 0.01;
            let pitch0 = cx.notes.pitch_prev[v]
                + p.prev(OscParam::Semi, v)
                + p.prev(OscParam::Fine, v) * 0.01;
            let dt1 = self.dt(pitch1, max_dt);
            let dt0 = if pitch0.to_bits() == pitch1.to_bits() {
                dt1
            } else {
                self.dt(pitch0, max_dt)
            };
            let dt = [dt0, dt1];
            let pw = [
                clamp_pw(p.prev(OscParam::Pw, v) * 0.01, dt0),
                clamp_pw(p.cur(OscParam::Pw, v) * 0.01, dt1),
            ];
            let frame_scale = 0.01 * (FRAMES - 1) as f32;
            let pos = [
                p.prev(OscParam::Position, v) * frame_scale,
                p.cur(OscParam::Position, v) * frame_scale,
            ];

            if self.class[v] != Some(class) {
                let phase = self.phase_of(v);
                self.enter(v, class, phase, pw[1], pos[1], dt0.max(dt1));
            }

            let out = &mut cx.io[v];
            let sync_in = if sync_on {
                cx.sync_in.map(|s| &s[v])
            } else {
                None
            };
            let sync_out = cx.sync_out.as_deref_mut().map(|s| &mut s[v]);
            match class {
                Class::Saw => self.shape[v].render::<SAW, SEG>(dt, pw, sync_in, sync_out, out),
                Class::Triangle => {
                    self.shape[v].render::<TRIANGLE, SEG>(dt, pw, sync_in, sync_out, out);
                }
                Class::Pulse => self.shape[v].render::<PULSE, SEG>(dt, pw, sync_in, sync_out, out),
                Class::Sine => {
                    self.phased[v].render::<SEG>(dt, sync_in, sync_out, out, |ph, _| sine(ph));
                }
                Class::Table => {
                    let level = level_for(dt0.max(dt1), self.alias_limit);
                    let table = &*self.table;
                    let phased = &mut self.phased[v];
                    if pos[0].to_bits() == pos[1].to_bits() {
                        // The position holds still (the usual case): the frame rows and the cross-fade weight are
                        // fixed for the segment, and on a frame (position 0 %, 100 %, k/12) one Hermite read does.
                        let at = table.cursor(level, pos[1]);
                        if at.on_frame() {
                            phased.render::<SEG>(dt, sync_in, sync_out, out, |ph, _| {
                                at.read_first(ph)
                            });
                        } else {
                            phased.render::<SEG>(dt, sync_in, sync_out, out, |ph, _| at.read(ph));
                        }
                    } else {
                        let step = pos[1] - pos[0];
                        phased.render::<SEG>(dt, sync_in, sync_out, out, |ph, w| {
                            table.sample(level, pos[0] + step * w, ph)
                        });
                    }
                }
            }
            // An audio route leaves this oscillator (lane RTE): what travels is the output before the level ramp.
            if let Some(pre) = cx.pre_level.as_deref_mut() {
                pre[v] = *out;
            }
            apply_level(
                out,
                p.prev(OscParam::Level, v) * 0.01,
                p.cur(OscParam::Level, v) * 0.01,
            );
        }
    }

    fn copy_state_from(&mut self, other: &Self) {
        self.class = other.class;
        self.phase0 = other.phase0;
        self.shape = other.shape;
        self.phased = other.phased;
    }

    fn subnormals(&self) -> usize {
        self.shape.iter().filter(|s| s.has_subnormal()).count()
            + self.phased.iter().filter(|s| s.has_subnormal()).count()
            + self.phase0.iter().filter(|x| x.is_subnormal()).count()
    }
}
