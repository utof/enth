//! The `adsr` module: S2a's **reference envelope**, to be replaced by lane D2.
//!
//! STUB (S2a), but not silent: an ADSR is every patch's amplitude envelope (`voice.amp_env`), so a silent one would
//! make every render silent and leave the voice-end rule untested. What it does, evaluated once per 32-frame segment
//! (the engine ramps the amplitude linearly across the segment):
//!
//! - **attack:** linear from the current level to 1, at a rate that takes `attack` ms from 0;
//! - **decay:** exponential toward `sustain`, falling 60 dB of the distance in `decay` ms;
//! - **release:** exponential toward 0, falling 60 dB in `release` ms, flushed below 1e-15;
//! - output `level · (1 − v + v·velocity)` with `v = velocity% / 100`;
//! - a voice is finished when the envelope is past its attack and its output is below −100 dB.
//!
//! D2 replaces this file with RC-style exponential segments whose overshoot targets make every segment end in
//! exactly its set time from any start level (segment-duration semantics; `docs/dsp/envelope.md`). Keep
//! `Adsr::new(values, cfg)`, the `VoiceModulator` impl and the voice-end rule; the goldens that use the reference
//! envelope are then updated on purpose.

use enth_dsp::util::flush::flush;
use enth_dsp::util::math::{DB_TO_LOG2, VOICE_END_LEVEL, exp2};
use enth_patch::catalogue::AdsrParam;

use crate::module::{
    MAX_VOICES, PrepareCfg, SEG, VoiceIdx, VoiceModCx, VoiceModulator, VoiceStart, count_subnormals,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    Attack,
    Decay,
    Release,
    Off,
}

/// The envelope (S2a reference).
#[derive(Clone, Debug)]
pub struct Adsr {
    sample_rate: f32,
    level: [f32; MAX_VOICES],
    out: [f32; MAX_VOICES],
    stage: [Stage; MAX_VOICES],
}

impl Adsr {
    /// Prepare an envelope.
    #[must_use]
    pub fn new(_values: &[f32], cfg: &PrepareCfg) -> Self {
        Self {
            sample_rate: cfg.sample_rate,
            level: [0.0; MAX_VOICES],
            out: [0.0; MAX_VOICES],
            stage: [Stage::Off; MAX_VOICES],
        }
    }

    /// Frames in `ms` milliseconds.
    fn frames(&self, ms: f32) -> f64 {
        f64::from(ms) * f64::from(self.sample_rate) / 1000.0
    }

    /// The per-segment multiplier that falls 60 dB in `ms`: `10^(−3·SEG / frames)`; 0 for a zero time.
    fn fall_60db(&self, ms: f32) -> f32 {
        let n = self.frames(ms);
        if n < 1.0 {
            return 0.0;
        }
        exp2(-60.0 * DB_TO_LOG2 * SEG as f64 / n) as f32
    }
}

impl VoiceModulator for Adsr {
    fn start_voice(&mut self, start: &VoiceStart<'_>) {
        let v = start.voice;
        self.level[v] = 0.0;
        self.out[v] = 0.0;
        self.stage[v] = Stage::Attack;
    }

    fn tick(&mut self, cx: &mut VoiceModCx<'_>) {
        let p = cx.params;
        for &v in cx.voices {
            if cx.notes.trigger[v] {
                self.stage[v] = Stage::Attack;
            }
            if !cx.notes.gate[v] && matches!(self.stage[v], Stage::Attack | Stage::Decay) {
                self.stage[v] = Stage::Release;
            }
            let sustain = p.cur(AdsrParam::Sustain, v) * 0.01;
            let mut level = self.level[v];
            match self.stage[v] {
                Stage::Attack => {
                    let n = self.frames(p.cur(AdsrParam::Attack, v));
                    if n < 1.0 {
                        level = 1.0;
                    } else {
                        level += (SEG as f64 / n) as f32;
                    }
                    if level >= 1.0 {
                        level = 1.0;
                        self.stage[v] = Stage::Decay;
                    }
                }
                Stage::Decay => {
                    let c = self.fall_60db(p.cur(AdsrParam::Decay, v));
                    level = sustain + flush((level - sustain) * c);
                }
                Stage::Release => {
                    let c = self.fall_60db(p.cur(AdsrParam::Release, v));
                    level = flush(level * c);
                    if level == 0.0 {
                        self.stage[v] = Stage::Off;
                    }
                }
                Stage::Off => level = 0.0,
            }
            self.level[v] = level;
            let amount = p.cur(AdsrParam::Velocity, v) * 0.01;
            let out = level * (1.0 - amount + amount * cx.notes.velocity[v]);
            self.out[v] = out;
            cx.out[v] = out;
        }
    }

    fn voice_finished(&self, v: VoiceIdx) -> bool {
        self.stage[v] != Stage::Attack && self.out[v] < VOICE_END_LEVEL
    }

    fn copy_state_from(&mut self, other: &Self) {
        self.level = other.level;
        self.out = other.out;
        self.stage = other.stage;
    }

    fn subnormals(&self) -> usize {
        count_subnormals(&self.level) + count_subnormals(&self.out)
    }
}
