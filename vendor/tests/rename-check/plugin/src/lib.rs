//! Research scratch: 8 macros (poly-mod) + 64 slots = 72 fixed params, a persisted JSON patch
//! blob, an 8-voice saw. Not product code.
use nice_plug::midi::{Channel, Key, VoiceID};
use nice_plug::prelude::*;
use std::sync::{Arc, RwLock};

const NUM_VOICES: usize = 8;

#[derive(Params)]
struct SlotParams {
    #[id = "slot"]
    value: FloatParam,
}

#[derive(Params)]
struct MacroParams {
    #[id = "macro"]
    value: FloatParam,
}

#[derive(Params)]
struct SpikeParams {
    #[nested(array, group = "Macro")]
    macros: [MacroParams; 8],
    #[nested(array, group = "Slot")]
    slots: [SlotParams; 64],
    /// Arbitrary patch blob persisted with the host project.
    #[persist = "patch"]
    patch: RwLock<String>,
}

impl Default for SpikeParams {
    fn default() -> Self {
        let mut i = 0u32;
        let macros = [(); 8].map(|_| {
            let id = i;
            i += 1;
            MacroParams {
                value: FloatParam::new(format!("Macro {}", id + 1), 0.0, FloatRange::Linear { min: 0.0, max: 1.0 })
                    .with_poly_modulation_id(id)
                    .with_smoother(SmoothingStyle::Linear(10.0)),
            }
        });
        let mut j = 0u32;
        let slots = [(); 64].map(|_| {
            j += 1;
            SlotParams {
                value: FloatParam::new(format!("Slot {j}"), 0.5, FloatRange::Linear { min: 0.0, max: 1.0 }),
            }
        });
        Self { macros, slots, patch: RwLock::new(String::from("{\"version\":1,\"modules\":[]}")) }
    }
}

pub enum RenameTask {
    Rename(usize, &'static str),
}

#[derive(Clone, Copy)]
struct Voice {
    voice_id: i32,
    channel: Channel,
    key: Key,
    phase: f32,
    dphase: f32,
    env: f32,
    releasing: bool,
    lp: f32,
}

struct Spike {
    renamed: [bool; 2],
    params: Arc<SpikeParams>,
    voices: [Option<Voice>; NUM_VOICES],
    sr: f32,
}

impl Default for Spike {
    fn default() -> Self {
        Self { renamed: [false; 2], params: Arc::new(SpikeParams::default()), voices: [None; NUM_VOICES], sr: 48_000.0 }
    }
}

impl Spike {
    fn release(&mut self, voice_id: VoiceID, channel: Channel, key: Key) {
        for v in self.voices.iter_mut().flatten() {
            if voice_id == VoiceID::ID(v.voice_id) || (channel == v.channel && key == v.key) {
                v.releasing = true;
            }
        }
    }
}

impl Plugin for Spike {
    const NAME: &'static str = "enth spike-np-rename";
    const VENDOR: &'static str = "enth research";
    const URL: &'static str = "https://example.invalid";
    const EMAIL: &'static str = "none@example.invalid";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");
    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[AudioIOLayout {
        main_input_channels: None,
        main_output_channels: NonZeroU32::new(2),
        ..AudioIOLayout::const_default()
    }];
    const MIDI_INPUT: MidiConfig = MidiConfig::Basic;
    const SAMPLE_ACCURATE_AUTOMATION: bool = true;
    type Editor = ();
    type SysExMessage = ();
    type BackgroundTask = RenameTask;

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn task_executor(&mut self) -> TaskExecutor<Self> {
        let params = self.params.clone();
        Box::new(move |task| match task {
            // Normally runs on the main thread (scheduled with execute_gui).
            RenameTask::Rename(slot, name) => nice_plug::util::permit_alloc(|| {
                // Hosts that call process() on the thread nice-plug treats as main (e.g. Steinberg's
                // validator) run execute_gui tasks inline inside process(), so permit allocation here.
                params.slots[slot].value.set_display_name(Some(name.to_string()));
                let log = format!("rename slot {} -> {} (thread {:?})\n", slot + 1, name, std::thread::current().id());
                let path = std::env::var("ENTH_RENAME_LOG").unwrap_or_else(|_| "/dev/null".into());
                let _ = std::fs::OpenOptions::new().create(true).append(true).open(path).and_then(|mut f| std::io::Write::write_all(&mut f, log.as_bytes()));
            }),
        })
    }

    fn activate(&mut self, _l: &AudioIOLayout, cfg: &BufferConfig, _c: &mut impl ActivateContext<Self>) -> bool {
        self.sr = cfg.sample_rate;
        // Research: stamp a nonce into the patch blob once, so a project save/reload can prove
        // the blob round-trips through the host (a fresh instance would not have this nonce).
        if let Ok(mut p) = self.params.patch.write() {
            if !p.contains("nonce") {
                let n = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
                *p = format!("{{\"version\":1,\"nonce\":\"{n}\",\"modules\":[]}}");
            }
        }
        true
    }

    fn reset(&mut self) {
        self.voices = [None; NUM_VOICES];
    }

    fn process(&mut self, buffer: &mut Buffer, _aux: &mut AuxiliaryBuffers, context: &mut impl ProcessContext<Self>) -> ProcessStatus {
        let n = buffer.samples();
        let out = buffer.as_slice();
        let cutoff = self.params.slots[0].value.value();
        let a = (1.0 - (-2.0 * std::f32::consts::PI * (50.0 + cutoff * 8000.0) / self.sr).exp()).clamp(0.0, 1.0);
        let env_step = 1.0 / (0.005 * self.sr);
        for (k, (trigger, slot, name)) in [(63usize, 0usize, "Osc 1 · Cutoff"), (62, 1, "Filter · Res")].into_iter().enumerate() {
            if !self.renamed[k] && self.params.slots[trigger].value.value() > 0.9 {
                self.renamed[k] = true;
                context.execute_gui(RenameTask::Rename(slot, name));
                context.request_param_info_rescan();
            }
        }
        let mut ev = context.next_event();
        for s in 0..n {
            while let Some(e) = ev {
                if e.timing() as usize > s { break; }
                match e {
                    NoteEvent::NoteOn { voice_id, channel, key, velocity, .. } => {
                        let v = Voice {
                            voice_id: voice_id.id_or_fallback(key, channel), channel, key,
                            phase: 0.0, dphase: util::midi_key_to_freq(key.number_or_middle_c()) / self.sr,
                            env: 0.0, releasing: false, lp: 0.0,
                        };
                        let _ = velocity;
                        if let Some(slot) = self.voices.iter_mut().find(|v| v.is_none()) {
                            *slot = Some(v);
                        } else if let Some(old) = self.voices[0].take() {
                            let _ = context.try_send_event(NoteEvent::VoiceTerminated { timing: s as u32, voice_id: VoiceID::ID(old.voice_id), channel: old.channel, key: old.key });
                            self.voices[0] = Some(v);
                        }
                    }
                    NoteEvent::NoteOff { voice_id, channel, key, .. } => self.release(voice_id, channel, key),
                    NoteEvent::Choke { voice_id, channel, key, .. } => {
                        for slot in self.voices.iter_mut() {
                            if let Some(v) = slot {
                                if voice_id == VoiceID::ID(v.voice_id) || (channel == v.channel && key == v.key) {
                                    let _ = context.try_send_event(NoteEvent::VoiceTerminated { timing: s as u32, voice_id: VoiceID::ID(v.voice_id), channel: v.channel, key: v.key });
                                    *slot = None;
                                }
                            }
                        }
                    }
                    _ => {}
                }
                ev = context.next_event();
            }
            let gain = 0.2 * (1.0 + self.params.macros[0].value.smoothed.next());
            let mut acc = 0.0f32;
            for slot in self.voices.iter_mut() {
                if let Some(v) = slot {
                    v.env = if v.releasing { (v.env - env_step).max(0.0) } else { (v.env + env_step).min(1.0) };
                    let saw = v.phase * 2.0 - 1.0;
                    v.phase += v.dphase;
                    if v.phase >= 1.0 { v.phase -= 1.0; }
                    v.lp += a * (saw - v.lp);
                    acc += v.lp * v.env * gain;
                    if v.releasing && v.env == 0.0 {
                        let _ = context.try_send_event(NoteEvent::VoiceTerminated { timing: s as u32, voice_id: VoiceID::ID(v.voice_id), channel: v.channel, key: v.key });
                        *slot = None;
                    }
                }
            }
            out[0][s] = acc;
            out[1][s] = acc;
        }
        ProcessStatus::KeepAlive
    }
}

impl ClapPlugin for Spike {
    const CLAP_ID: &'static str = "invalid.enth-research.spike-np-rename";
    const CLAP_DESCRIPTION: Option<&'static str> = None;
    const CLAP_MANUAL_URL: Option<&'static str> = None;
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[ClapFeature::Instrument, ClapFeature::Synthesizer, ClapFeature::Stereo];
    const CLAP_POLY_MODULATION_CONFIG: Option<PolyModulationConfig> = Some(PolyModulationConfig { max_voice_capacity: NUM_VOICES as u32, supports_overlapping_voices: true });
}

impl Vst3Plugin for Spike {
    const VST3_CLASS_ID: [u8; 16] = *b"enthSpikeNPren01";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] = &[Vst3SubCategory::Instrument, Vst3SubCategory::Synth];
}

nice_export_clap!(Spike);
nice_export_vst3!(Spike);
