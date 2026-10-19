//! The `osc` module: S2a's **reference saw**, to be replaced by lane D1.
//!
//! STUB (S2a). What it does, so the walking skeleton has one real sound to hash on every platform:
//!
//! - `wave = saw`: a naive saw with a two-sample polyBLEP correction (Välimäki and Huovilainen 2007, the textbook
//!   form), phase in `f32`, pitch `notes.pitch_cur + semi + fine/100` evaluated once per segment, level ramped per
//!   sample from the previous segment's value. Every other wave outputs silence.
//! - `phase_mode`: `free` draws the start phase from the voice seed (render seed, module id, note index); `reset`
//!   starts at `phase`; `seeded` draws it from the module's `seed` parameter (`crate::seed::seeded_voice_seed`).
//! - As the sync master (the first enabled `osc`), it writes its phase wraps to `VoiceCx::sync_out`; as a slave it
//!   ignores `sync`. `pw`, `table` and `position` are ignored.
//!
//! D1 replaces this file with the port of Plaits' `VariableShapeOscillator` (saw, pulse with PWM, triangle, hard
//! sync with polyBLEP/BLAMP), a sine, and the wavetable mode (tables generated in `new` from the structural `table`
//! parameter, sized from `PrepareCfg` only). Keep `Osc::new(values, cfg)` and the `VoiceProcessor` impl; the golden
//! cases built on this saw (`tests/golden/cases/ref-saw-*`) are then updated on purpose, with the reason in the
//! commit message.

use enth_dsp::util::math::midi_to_hz;
use enth_dsp::util::rng::Rng;
use enth_patch::catalogue::{OscParam, PhaseMode, Wave};

use crate::module::{
    Block, MAX_VOICES, NO_SYNC, PrepareCfg, RAMP, SEG, VoiceCx, VoiceProcessor, VoiceStart,
    count_subnormals,
};
use crate::seed::seeded_voice_seed;

/// The highest phase increment per sample (0.45 of the sample rate): the reference saw stays below Nyquist.
const MAX_DT: f32 = 0.45;

/// The oscillator (S2a reference saw).
#[derive(Clone, Debug)]
pub struct Osc {
    sample_rate: f32,
    phase: [f32; MAX_VOICES],
}

impl Osc {
    /// Prepare an oscillator. `values` are the patch values (unused by the stub; D1 generates the wavetable chosen
    /// by `table` here).
    #[must_use]
    pub fn new(_values: &[f32], cfg: &PrepareCfg) -> Self {
        Self {
            sample_rate: cfg.sample_rate,
            phase: [0.0; MAX_VOICES],
        }
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

/// The two-sample polyBLEP residual at phase `t` for increment `dt`.
#[inline(always)]
fn poly_blep(t: f32, dt: f32) -> f32 {
    if t < dt {
        let x = t / dt;
        x + x - x * x - 1.0
    } else if t > 1.0 - dt {
        let x = (t - 1.0) / dt;
        x * x + x + x + 1.0
    } else {
        0.0
    }
}

/// One segment of the reference saw. When `sync` is given (this oscillator is the sync master), it records where
/// the phase wrapped: the fraction of the frame left after the wrap, [`NO_SYNC`] elsewhere.
fn saw(
    phase: &mut f32,
    dt: f32,
    gain_prev: f32,
    gain_cur: f32,
    out: &mut Block,
    mut sync: Option<&mut Block>,
) {
    let mut t = *phase;
    let step = gain_cur - gain_prev;
    for (i, (o, r)) in out.iter_mut().zip(RAMP).enumerate() {
        let y = 2.0 * t - 1.0 - poly_blep(t, dt);
        *o = y * (gain_prev + step * r);
        t += dt;
        let wrapped = t >= 1.0;
        if wrapped {
            t -= 1.0;
        }
        if let Some(s) = sync.as_deref_mut() {
            s[i] = if wrapped && dt > 0.0 { t / dt } else { NO_SYNC };
        }
    }
    *phase = t;
}

impl VoiceProcessor for Osc {
    fn start_voice(&mut self, start: &VoiceStart<'_>) {
        self.phase[start.voice] = start_phase(start);
    }

    fn process(&mut self, cx: &mut VoiceCx<'_>) {
        let p = cx.params;
        let is_saw = Wave::from_value(p.base(OscParam::Wave)) == Some(Wave::Saw);
        for &v in cx.voices {
            let out = &mut cx.io[v];
            if !is_saw {
                *out = [0.0; SEG];
                continue;
            }
            let pitch =
                cx.notes.pitch_cur[v] + p.cur(OscParam::Semi, v) + p.cur(OscParam::Fine, v) * 0.01;
            let dt = (midi_to_hz(pitch) / self.sample_rate).clamp(0.0, MAX_DT);
            let gain_prev = p.prev(OscParam::Level, v) * 0.01;
            let gain_cur = p.cur(OscParam::Level, v) * 0.01;
            let sync = cx.sync_out.as_deref_mut().map(|s| &mut s[v]);
            saw(&mut self.phase[v], dt, gain_prev, gain_cur, out, sync);
        }
    }

    fn copy_state_from(&mut self, other: &Self) {
        self.phase = other.phase;
    }

    fn subnormals(&self) -> usize {
        count_subnormals(&self.phase)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_saw_is_bounded_and_has_the_right_period() {
        let mut phase = 0.25;
        let dt = 440.0 / 48_000.0;
        let mut out = [0.0; SEG];
        let mut crossings = 0;
        let mut last = 0.0f32;
        for _ in 0..1500 {
            saw(&mut phase, dt, 1.0, 1.0, &mut out, None);
            for &y in &out {
                assert!(y.abs() <= 1.0 + 1e-6, "{y}");
                if last >= 0.0 && y < 0.0 {
                    crossings += 1; // falling zero crossings: one per period, at the reset
                }
                last = y;
            }
        }
        // 1500 segments = 1 s at 48 kHz: 440 periods, give or take the first.
        assert!((439..=441).contains(&crossings), "{crossings}");
    }

    #[test]
    fn the_sync_master_marks_each_wrap_once() {
        let (mut phase, dt) = (0.9, 0.3);
        let mut out = [0.0; SEG];
        let mut sync = [0.5; SEG];
        saw(&mut phase, dt, 1.0, 1.0, &mut out, Some(&mut sync));
        // 0.9 -> 1.2 wraps in frame 0 with 0.2 of a phase step left: 0.2 / 0.3 of the frame.
        assert!((sync[0] - 0.2 / 0.3).abs() < 1e-5, "{}", sync[0]);
        assert_eq!(sync[1].to_bits(), NO_SYNC.to_bits());
        let wraps = sync
            .iter()
            .filter(|x| x.to_bits() != NO_SYNC.to_bits())
            .count();
        assert_eq!(wraps, 10, "0.9 + 32 * 0.3 = 10.5: ten wraps");
        assert!(
            sync.iter()
                .all(|x| x.to_bits() == NO_SYNC.to_bits() || (0.0..1.0).contains(x))
        );
    }

    #[test]
    fn poly_blep_is_zero_away_from_the_edge() {
        assert_eq!(poly_blep(0.5, 0.01), 0.0);
        assert!(poly_blep(0.0, 0.01) < 0.0);
        assert!(poly_blep(0.999, 0.01) > 0.0);
    }
}
