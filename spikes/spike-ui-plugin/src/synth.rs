//! The spike's sound engine: 8 voices, each a saw oscillator through a one-pole low-pass and an
//! ADSR amplitude envelope. Plain Rust with no framework types, so it can be unit-tested.
//!
//! Real-time rules apply: nothing here allocates, locks or panics.

use std::f32::consts::TAU;

pub const NUM_VOICES: usize = 8;

const ATTACK_SECONDS: f32 = 0.005;
const DECAY_SECONDS: f32 = 0.08;
const SUSTAIN_LEVEL: f32 = 0.7;
const RELEASE_SECONDS: f32 = 0.12;

/// Lowest and highest cutoff in Hz; the normalised cutoff parameter sweeps between them exponentially.
const CUTOFF_MIN_HZ: f32 = 50.0;
const CUTOFF_MAX_HZ: f32 = 8_000.0;

/// Identifies a sounding voice: the host's voice id (or a fallback derived from channel and key
/// when the host gives none), plus its channel and key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VoiceKey {
    pub voice_id: i32,
    pub channel: u8,
    pub key: u8,
}

/// Which voices a note-off or choke addresses: the one with the host's voice id when the event
/// carries one, otherwise every voice on that channel and key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Target {
    pub voice_id: Option<i32>,
    pub channel: u8,
    pub key: u8,
}

impl Target {
    fn matches(self, voice: VoiceKey) -> bool {
        match self.voice_id {
            Some(id) => voice.voice_id == id,
            None => voice.channel == self.channel && voice.key == self.key,
        }
    }
}

/// A voice that has ended (released or stolen). CLAP hosts need to hear about every one so they
/// can free the resources they use for polyphonic modulation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Terminated(pub VoiceKey);

#[derive(Clone, Copy, Debug, PartialEq)]
enum Stage {
    Attack,
    Decay,
    Sustain,
    Release,
}

#[derive(Clone, Copy)]
struct Voice {
    id: VoiceKey,
    phase: f32,
    phase_inc: f32,
    level: f32,
    stage: Stage,
    lowpass: f32,
    /// Normalised offset from polyphonic modulation of macro 1 (gain). Zero when unmodulated.
    gain_offset: f32,
}

pub struct Synth {
    voices: [Option<Voice>; NUM_VOICES],
    sample_rate: f32,
}

impl Synth {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            voices: [None; NUM_VOICES],
            sample_rate,
        }
    }

    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate;
    }

    /// Drops every voice without telling the host (used on reset).
    pub fn reset(&mut self) {
        self.voices = [None; NUM_VOICES];
    }

    #[cfg(test)]
    fn active_voices(&self) -> usize {
        self.voices.iter().flatten().count()
    }

    /// Whether any voice is sounding (attack to the end of release).
    pub fn is_sounding(&self) -> bool {
        self.voices.iter().any(Option::is_some)
    }

    /// Starts a note. When all voices are busy, steals the quietest releasing voice, else the first;
    /// returns the stolen voice so the caller can report it.
    pub fn note_on(&mut self, id: VoiceKey) -> Option<Terminated> {
        let voice = Voice {
            id,
            phase: 0.0,
            phase_inc: midi_key_to_hz(id.key) / self.sample_rate,
            level: 0.0,
            stage: Stage::Attack,
            lowpass: 0.0,
            gain_offset: 0.0,
        };
        if let Some(free) = self.voices.iter_mut().find(|v| v.is_none()) {
            *free = Some(voice);
            return None;
        }
        let victim = self
            .voices
            .iter()
            .enumerate()
            .filter_map(|(i, v)| {
                v.filter(|v| v.stage == Stage::Release)
                    .map(|v| (i, v.level))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map_or(0, |(i, _)| i);
        self.voices[victim].replace(voice).map(|v| Terminated(v.id))
    }

    pub fn note_off(&mut self, target: Target) {
        for voice in self.voices.iter_mut().flatten() {
            if target.matches(voice.id) {
                voice.stage = Stage::Release;
            }
        }
    }

    /// Ends matching voices immediately, calling `terminated` for each.
    pub fn choke(&mut self, target: Target, mut terminated: impl FnMut(Terminated)) {
        for slot in &mut self.voices {
            if slot.is_some_and(|v| target.matches(v.id))
                && let Some(v) = slot.take()
            {
                terminated(Terminated(v.id));
            }
        }
    }

    /// Sets the polyphonic-modulation offset (normalised) of macro 1 for one voice.
    pub fn set_gain_offset(&mut self, voice_id: i32, offset: f32) {
        for voice in self.voices.iter_mut().flatten() {
            if voice.id.voice_id == voice_id {
                voice.gain_offset = offset;
            }
        }
    }

    /// Renders one mono sample. `gain` is macro 1 (normalised, smoothed) and `cutoff` is slot 1
    /// (normalised, smoothed). `terminated` is called for every voice whose release just ended.
    pub fn next_sample(
        &mut self,
        gain: f32,
        cutoff: f32,
        mut terminated: impl FnMut(Terminated),
    ) -> f32 {
        let lowpass_coeff = one_pole_coeff(cutoff_hz(cutoff), self.sample_rate);
        let attack_step = 1.0 / (ATTACK_SECONDS * self.sample_rate);
        let decay_step = (1.0 - SUSTAIN_LEVEL) / (DECAY_SECONDS * self.sample_rate);
        let release_step = 1.0 / (RELEASE_SECONDS * self.sample_rate);
        let mut sum = 0.0;
        for slot in &mut self.voices {
            let Some(voice) = slot else { continue };
            match voice.stage {
                Stage::Attack => {
                    voice.level += attack_step;
                    if voice.level >= 1.0 {
                        voice.level = 1.0;
                        voice.stage = Stage::Decay;
                    }
                }
                Stage::Decay => {
                    voice.level -= decay_step;
                    if voice.level <= SUSTAIN_LEVEL {
                        voice.level = SUSTAIN_LEVEL;
                        voice.stage = Stage::Sustain;
                    }
                }
                Stage::Sustain => {}
                Stage::Release => voice.level = (voice.level - release_step).max(0.0),
            }
            let saw = voice.phase * 2.0 - 1.0;
            voice.phase += voice.phase_inc;
            if voice.phase >= 1.0 {
                voice.phase -= 1.0;
            }
            voice.lowpass += lowpass_coeff * (saw - voice.lowpass);
            let voice_gain = (gain + voice.gain_offset).clamp(0.0, 1.0);
            sum += voice.lowpass * voice.level * voice_gain * VOICE_SCALE;
            if voice.stage == Stage::Release && voice.level == 0.0 {
                terminated(Terminated(voice.id));
                *slot = None;
            }
        }
        sum
    }
}

/// Headroom for eight voices.
const VOICE_SCALE: f32 = 0.25;

fn midi_key_to_hz(key: u8) -> f32 {
    440.0 * ((f32::from(key) - 69.0) / 12.0).exp2()
}

/// Normalised cutoff (0..1) to Hz, exponential.
pub fn cutoff_hz(normalised: f32) -> f32 {
    CUTOFF_MIN_HZ * (CUTOFF_MAX_HZ / CUTOFF_MIN_HZ).powf(normalised.clamp(0.0, 1.0))
}

fn one_pole_coeff(hz: f32, sample_rate: f32) -> f32 {
    (1.0 - (-TAU * hz / sample_rate).exp()).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48_000.0;

    fn note(key: u8) -> VoiceKey {
        VoiceKey {
            voice_id: i32::from(key),
            channel: 0,
            key,
        }
    }

    fn target(key: u8) -> Target {
        Target {
            voice_id: None,
            channel: 0,
            key,
        }
    }

    fn render(synth: &mut Synth, samples: usize, gain: f32, cutoff: f32) -> Vec<f32> {
        (0..samples)
            .map(|_| synth.next_sample(gain, cutoff, |_| {}))
            .collect()
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|s| s * s).sum::<f32>() / x.len() as f32).sqrt()
    }

    #[test]
    fn silent_without_notes() {
        let mut synth = Synth::new(SR);
        assert!(render(&mut synth, 256, 1.0, 1.0).iter().all(|&s| s == 0.0));
    }

    #[test]
    fn note_sounds_then_releases_and_frees_its_voice() {
        let mut synth = Synth::new(SR);
        synth.note_on(note(60));
        let held = render(&mut synth, 4800, 1.0, 0.8);
        assert!(rms(&held) > 0.01, "rms {}", rms(&held));
        assert!(held.iter().all(|s| s.is_finite() && s.abs() <= 1.0));
        synth.note_off(target(60));
        let mut ended = Vec::new();
        for _ in 0..(SR as usize / 2) {
            synth.next_sample(1.0, 0.8, |t| ended.push(t));
        }
        assert_eq!(ended, vec![Terminated(note(60))]);
        assert_eq!(synth.active_voices(), 0);
    }

    #[test]
    fn gain_and_cutoff_change_the_sound() {
        let mut quiet = Synth::new(SR);
        let mut loud = Synth::new(SR);
        quiet.note_on(note(48));
        loud.note_on(note(48));
        let quiet_rms = rms(&render(&mut quiet, 4800, 0.2, 0.5));
        let loud_rms = rms(&render(&mut loud, 4800, 0.8, 0.5));
        assert!(loud_rms > 2.0 * quiet_rms);

        let mut dark = Synth::new(SR);
        let mut bright = Synth::new(SR);
        dark.note_on(note(48));
        bright.note_on(note(48));
        let roughness = |x: &[f32]| rms(&x.windows(2).map(|w| w[1] - w[0]).collect::<Vec<_>>());
        assert!(
            roughness(&render(&mut bright, 4800, 1.0, 1.0))
                > 2.0 * roughness(&render(&mut dark, 4800, 1.0, 0.1))
        );
    }

    #[test]
    fn ninth_note_steals_a_voice_and_reports_it() {
        let mut synth = Synth::new(SR);
        for key in 40..48 {
            assert_eq!(synth.note_on(note(key)), None);
        }
        assert_eq!(synth.active_voices(), NUM_VOICES);
        assert_eq!(synth.note_on(note(60)), Some(Terminated(note(40))));
        assert_eq!(synth.active_voices(), NUM_VOICES);
    }

    #[test]
    fn polyphonic_offset_affects_only_its_voice() {
        let mut base = Synth::new(SR);
        let mut modulated = Synth::new(SR);
        for synth in [&mut base, &mut modulated] {
            synth.note_on(note(48));
            synth.note_on(note(55));
        }
        modulated.set_gain_offset(48, -0.5);
        let a = rms(&render(&mut base, 4800, 0.8, 0.5));
        let b = rms(&render(&mut modulated, 4800, 0.8, 0.5));
        assert!(b < a && b > 0.0, "{b} vs {a}");
    }

    #[test]
    fn rendering_is_deterministic() {
        let run = || {
            let mut synth = Synth::new(SR);
            synth.note_on(note(52));
            render(&mut synth, 2048, 0.7, 0.6)
        };
        assert_eq!(run(), run());
    }

    #[test]
    fn choke_ends_voices_at_once() {
        let mut synth = Synth::new(SR);
        synth.note_on(note(60));
        let mut ended = Vec::new();
        synth.choke(target(60), |t| ended.push(t));
        assert_eq!(ended, vec![Terminated(note(60))]);
        assert_eq!(synth.active_voices(), 0);
    }
}
