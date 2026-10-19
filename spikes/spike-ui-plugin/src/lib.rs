//! Phase 0 UI spike in a host (docs/phase-0.md, Task 4): a copy of the shell spike's plugin
//! (8-voice synth, 8 macros and 64 slots with stable ids, JSON patch blob) whose editor is the
//! reference panel from `spike-ui`. Lane U2 measures it in REAPER.
//!
//! The audio thread feeds the panel through `spike_ui::AudioFeed` (atomics only) while the
//! editor is open: the output samples for the scopes, LFO 1's phase for the modulation rings,
//! and whether anything sounds. It wakes a sleeping editor with nice-plug-egui's
//! `RepaintNotifier` (an atomic store), so an idle editor draws nothing.
//!
//! `ENTH_UI_LOG=<file>` (read when the editor opens) logs every frame and a once-a-second
//! summary with the GUI thread's CPU time, for measurements. `ENTH_THEME=<file.json>` replaces the
//! dark theme's tokens (the format of `spike-ui-dev --dump-theme`).
#![allow(
    unsafe_code,
    reason = "nice_export_clap! and nice_export_vst3! expand to extern \"C\" entry points"
)]

mod frame_log;
mod synth;

use std::num::NonZeroU32;
use std::sync::{Arc, RwLock};

use nice_plug::context::gui::GuiContext;
use nice_plug::editor::dpi::LogicalSize;
use nice_plug::midi::{Channel, Key, VoiceID};
use nice_plug::prelude::*;
use nice_plug_egui::baseview::HandlerError;
use nice_plug_egui::{
    EguiEditor, EguiEditorState, EguiNiceSettings, Frame, NiceEguiApp, RepaintNotifier,
    create_egui_editor,
};
use spike_ui::{AudioFeed, HostParams, ParamId, REFERENCE_SIZE, ReferencePanel, layout};

use frame_log::FrameLog;
use synth::{NUM_VOICES, Synth, Target, Terminated, VoiceKey};

pub const NUM_MACROS: usize = 8;
pub const NUM_SLOTS: usize = 64;

/// Permanent plugin ids (docs/naming.md). Never change them.
pub const CLAP_ID: &str = "dev.enth.spike-ui";
pub const VST3_CLASS_ID: [u8; 16] = *b"EnthSpikeUiPnl01";

/// Index of the macro that drives the output gain (and takes polyphonic modulation).
const GAIN_MACRO: usize = 0;
/// Index of the slot that drives the filter cutoff (the panel's Filter › Cutoff).
const CUTOFF_SLOT: usize = 0;

#[derive(Params)]
pub struct MacroParam {
    #[id = "macro"]
    pub value: FloatParam,
}

#[derive(Params)]
pub struct SlotParam {
    #[id = "slot"]
    pub value: FloatParam,
}

/// The 72 host parameters, in host order: `macro_1..8`, then `slot_1..64` (as spike-shell).
#[derive(Params)]
pub struct ShellParams {
    #[nested(array)]
    pub macros: [MacroParam; NUM_MACROS],
    #[nested(array)]
    pub slots: [SlotParam; NUM_SLOTS],
    /// Arbitrary patch blob saved with the host project (a typed value: see spike-shell).
    #[persist = "patch"]
    pub patch: RwLock<serde_json::Value>,
}

fn unit_range() -> FloatRange {
    FloatRange::Linear { min: 0.0, max: 1.0 }
}

impl Default for ShellParams {
    fn default() -> Self {
        let macros = std::array::from_fn(|i| {
            let mut value = FloatParam::new(
                format!("Macro {}", i + 1),
                layout::MACROS[i].default,
                unit_range(),
            )
            .with_poly_modulation_id(i as u32);
            if i == GAIN_MACRO {
                value = value.with_smoother(SmoothingStyle::Linear(10.0));
            }
            MacroParam { value }
        });
        let slots = std::array::from_fn(|i| {
            let mut value = FloatParam::new(
                format!("Slot {}", i + 1),
                layout::slot_default(i),
                unit_range(),
            );
            if i == CUTOFF_SLOT {
                value = value.with_smoother(SmoothingStyle::Linear(10.0));
            }
            SlotParam { value }
        });
        Self {
            macros,
            slots,
            patch: RwLock::new(serde_json::json!({ "version": 1, "modules": [] })),
        }
    }
}

impl ShellParams {
    fn param(&self, id: ParamId) -> &FloatParam {
        match id {
            ParamId::Macro(i) => &self.macros[i].value,
            ParamId::Slot(i) => &self.slots[i].value,
        }
    }
}

pub struct SpikeUiPlugin {
    params: Arc<ShellParams>,
    synth: Synth,
    sample_rate: f32,
    lfo_phase: f32,
    feed: Arc<AudioFeed>,
    editor_state: Arc<EguiEditorState>,
    repaint: RepaintNotifier,
}

impl Default for SpikeUiPlugin {
    fn default() -> Self {
        Self {
            params: Arc::new(ShellParams::default()),
            synth: Synth::new(48_000.0),
            sample_rate: 48_000.0,
            lfo_phase: 0.0,
            feed: Arc::new(AudioFeed::new()),
            editor_state: EguiEditorState::from_size(
                LogicalSize::new(REFERENCE_SIZE.x, REFERENCE_SIZE.y),
                1.0,
            ),
            repaint: RepaintNotifier::new(),
        }
    }
}

/// The voice id for a new note: the host's id when it gives one, else one from channel and key
/// (not `VoiceID::id_or_fallback`, which ignores the host's id in nice-plug 0.4.2).
fn voice_key(voice_id: VoiceID, channel: Channel, key: Key) -> VoiceKey {
    let channel_number = channel.number().unwrap_or(0);
    let key_number = key.number_or_middle_c();
    VoiceKey {
        voice_id: voice_id
            .id()
            .unwrap_or_else(|| i32::from(key_number) | (i32::from(channel_number) << 16)),
        channel: channel_number,
        key: key_number,
    }
}

fn target(voice_id: VoiceID, channel: Channel, key: Key) -> Target {
    Target {
        voice_id: voice_id.id(),
        channel: channel.number().unwrap_or(0),
        key: key.number_or_middle_c(),
    }
}

fn terminated_event(timing: u32, voice: VoiceKey) -> NoteEvent<()> {
    NoteEvent::VoiceTerminated {
        timing,
        voice_id: VoiceID::ID(voice.voice_id),
        channel: Channel::Number(voice.channel),
        key: Key::Number(voice.key),
    }
}

impl SpikeUiPlugin {
    fn handle_event(&mut self, event: NoteEvent<()>, context: &mut impl ProcessContext<Self>) {
        let timing = event.timing();
        let mut report = |Terminated(voice): Terminated| {
            let _ = context.try_send_event(terminated_event(timing, voice));
        };
        match event {
            NoteEvent::NoteOn {
                voice_id,
                channel,
                key,
                ..
            } => {
                if let Some(stolen) = self.synth.note_on(voice_key(voice_id, channel, key)) {
                    report(stolen);
                }
            }
            NoteEvent::NoteOff {
                voice_id,
                channel,
                key,
                ..
            } => self.synth.note_off(target(voice_id, channel, key)),
            NoteEvent::Choke {
                voice_id,
                channel,
                key,
                ..
            } => self.synth.choke(target(voice_id, channel, key), report),
            NoteEvent::PolyModulation {
                voice_id,
                poly_modulation_id,
                normalized_offset,
                ..
            } if poly_modulation_id as usize == GAIN_MACRO => {
                self.synth.set_gain_offset(voice_id, normalized_offset);
            }
            _ => {}
        }
    }
}

impl Plugin for SpikeUiPlugin {
    const NAME: &'static str = "enth spike-ui";
    const VENDOR: &'static str = "enth";
    const URL: &'static str = "";
    const EMAIL: &'static str = "";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[AudioIOLayout {
        main_input_channels: None,
        main_output_channels: NonZeroU32::new(2),
        ..AudioIOLayout::const_default()
    }];
    const MIDI_INPUT: MidiConfig = MidiConfig::Basic;
    const SAMPLE_ACCURATE_AUTOMATION: bool = true;

    type Editor = EguiEditor<Editor>;
    type SysExMessage = ();
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn editor(&mut self, _async_executor: AsyncExecutor<Self>) -> Option<Self::Editor> {
        create_egui_editor(
            self.editor_state.clone(),
            self.repaint.clone(),
            EguiNiceSettings::new().with_tile("enth spike-ui"),
            Editor {
                params: self.params.clone(),
                feed: self.feed.clone(),
                gui: None,
                panel: None,
                log: None,
            },
        )
    }

    fn activate(
        &mut self,
        _layout: &AudioIOLayout,
        config: &BufferConfig,
        _context: &mut impl ActivateContext<Self>,
    ) -> bool {
        self.sample_rate = config.sample_rate;
        self.synth.set_sample_rate(config.sample_rate);
        true
    }

    fn reset(&mut self) {
        self.synth.reset();
    }

    fn process(
        &mut self,
        buffer: &mut Buffer<'_>,
        _aux: &mut AuxiliaryBuffers<'_>,
        context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        let samples = buffer.samples();
        let out = buffer.as_slice();
        let editor_open = self.editor_state.is_open();
        let feed = Arc::clone(&self.feed);
        let mut writer = feed.writer();
        let mut next_event = context.next_event();
        for sample in 0..samples {
            while let Some(event) = next_event {
                if event.timing() as usize > sample {
                    break;
                }
                self.handle_event(event, context);
                next_event = context.next_event();
            }
            let gain = self.params.macros[GAIN_MACRO].value.smoothed.next();
            let cutoff = self.params.slots[CUTOFF_SLOT].value.smoothed.next();
            let timing = sample as u32;
            let mixed = self.synth.next_sample(gain, cutoff, |Terminated(voice)| {
                let _ = context.try_send_event(terminated_event(timing, voice));
            });
            for channel in out.iter_mut() {
                channel[sample] = mixed;
            }
            if editor_open {
                writer.push(mixed);
            }
        }
        // LFO 1 only drives the rings on screen; the DSP doesn't use it.
        self.lfo_phase =
            (self.lfo_phase + layout::LFO1_HZ * samples as f32 / self.sample_rate).fract();
        if editor_open && writer.finish(self.lfo_phase, self.synth.is_sounding()) {
            self.repaint.request_repaint();
        }
        ProcessStatus::KeepAlive
    }
}

impl ClapPlugin for SpikeUiPlugin {
    const CLAP_ID: &'static str = CLAP_ID;
    const CLAP_DESCRIPTION: Option<&'static str> =
        Some("Phase 0 UI spike: the reference panel on the shell spike's synth");
    const CLAP_MANUAL_URL: Option<&'static str> = None;
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[
        ClapFeature::Instrument,
        ClapFeature::Synthesizer,
        ClapFeature::Stereo,
    ];
    const CLAP_POLY_MODULATION_CONFIG: Option<PolyModulationConfig> = Some(PolyModulationConfig {
        max_voice_capacity: NUM_VOICES as u32,
        supports_overlapping_voices: true,
    });
}

impl Vst3Plugin for SpikeUiPlugin {
    const VST3_CLASS_ID: [u8; 16] = VST3_CLASS_ID;
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] =
        &[Vst3SubCategory::Instrument, Vst3SubCategory::Synth];
}

nice_export_clap!(SpikeUiPlugin);
nice_export_vst3!(SpikeUiPlugin);

/// The editor: the reference panel bound to the host parameters.
pub struct Editor {
    params: Arc<ShellParams>,
    feed: Arc<AudioFeed>,
    gui: Option<GuiContext>,
    panel: Option<ReferencePanel>,
    log: Option<FrameLog>,
}

/// The dark theme, or the tokens in the JSON file named by `ENTH_THEME` (read when the editor
/// opens; for trying token values, e.g. repaint rates, without rebuilding). A file that doesn't
/// parse falls back to the dark theme.
fn theme_from_env() -> enth_ui::Theme {
    std::env::var_os("ENTH_THEME")
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_else(enth_ui::Theme::dark)
}

/// The panel's view of the host: normalised values and gestures through nice-plug.
struct Host<'a> {
    params: &'a ShellParams,
    setter: nice_plug::context::gui::ParamSetter<'a>,
}

impl HostParams for Host<'_> {
    fn get(&self, id: ParamId) -> f32 {
        self.params.param(id).unmodulated_normalized_value()
    }

    fn begin(&mut self, id: ParamId) {
        self.setter.begin_set_parameter(self.params.param(id));
    }

    fn set(&mut self, id: ParamId, normalized: f32) {
        self.setter
            .set_parameter_normalized(self.params.param(id), normalized);
    }

    fn end(&mut self, id: ParamId) {
        self.setter.end_set_parameter(self.params.param(id));
    }
}

impl NiceEguiApp for Editor {
    fn build(
        &mut self,
        egui_ctx: egui::Context,
        gui: GuiContext,
        _frame: &mut Frame,
    ) -> Result<(), HandlerError> {
        // nice-plug-egui 0.6.0 never calls editor_closed(), so every open starts afresh here: the
        // panel's textures belong to the previous window's context.
        frame_log::name_this_thread();
        let theme = theme_from_env();
        theme.apply(&egui_ctx);
        self.panel = Some(ReferencePanel::new(theme, self.feed.clone()));
        self.gui = Some(gui);
        self.log = FrameLog::from_env();
        Ok(())
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut Frame) {
        let (Some(gui), Some(panel)) = (&self.gui, &mut self.panel) else {
            return;
        };
        let started = std::time::Instant::now();
        let mut host = Host {
            params: &self.params,
            setter: gui.param_setter(),
        };
        let repaint = panel.ui(ui, &mut host);
        if let Some(log) = &mut self.log {
            log.frame(started.elapsed(), repaint);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_sees_72_params_with_stable_ids() {
        let params = ShellParams::default();
        let map = params.param_map();
        assert_eq!(map.len(), NUM_MACROS + NUM_SLOTS);
        let ids: Vec<&str> = map.iter().map(|(id, _, _)| id.as_str()).collect();
        assert_eq!(ids.first(), Some(&"macro_1"));
        assert_eq!(ids[8], "slot_1");
        assert_eq!(ids.last(), Some(&"slot_64"));
    }

    #[test]
    fn slot_defaults_come_from_the_panel_layout() {
        let params = ShellParams::default();
        let cutoff = params.slots[CUTOFF_SLOT].value.default_plain_value();
        assert_eq!(cutoff, layout::GENERATORS[1].params[0].default);
    }

    #[test]
    fn plugin_ids_are_permanent() {
        assert_eq!(<SpikeUiPlugin as ClapPlugin>::CLAP_ID, "dev.enth.spike-ui");
        assert_eq!(
            <SpikeUiPlugin as Vst3Plugin>::VST3_CLASS_ID,
            *b"EnthSpikeUiPnl01"
        );
    }
}
