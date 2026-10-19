//! Phase 0 shell spike (docs/phase-0.md, Task 3): an 8-voice synth on nice-plug with the host
//! parameter surface from docs/design.md §4 — 8 macros and 64 automation slots, 72 fixed
//! parameters with stable ids — plus an arbitrary JSON patch blob saved in the host project and a
//! placeholder egui editor. Throwaway code: the real panel is a separate spike.
//!
//! Sound mapping: macro 1 is the output gain (and takes CLAP polyphonic modulation per voice),
//! slot 1 is the low-pass cutoff. Every other parameter is declared but not yet used.
#![allow(
    unsafe_code,
    reason = "nice_export_clap! and nice_export_vst3! expand to extern \"C\" entry points"
)]

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
    create_egui_editor, widgets,
};

use synth::{NUM_VOICES, Synth, Target, Terminated, VoiceKey};

pub const NUM_MACROS: usize = 8;
pub const NUM_SLOTS: usize = 64;

/// Permanent plugin ids (docs/naming.md). Hosts key saved projects and presets by these: never change them.
pub const CLAP_ID: &str = "dev.enth.spike-shell";
pub const VST3_CLASS_ID: [u8; 16] = *b"EnthSpikeShell01";

/// Index of the macro that drives the output gain (and takes polyphonic modulation).
const GAIN_MACRO: usize = 0;
/// Index of the slot that drives the filter cutoff.
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

/// The 72 host parameters, in host order: `macro_1..8`, then `slot_1..64`. The string ids are the
/// stable part (nice-plug hashes them for the host); `#[nested(array)]` appends `_1`, `_2`, ….
#[derive(Params)]
pub struct ShellParams {
    #[nested(array)]
    pub macros: [MacroParam; NUM_MACROS],
    #[nested(array)]
    pub slots: [SlotParam; NUM_SLOTS],
    /// Arbitrary patch blob saved with the host project. A `serde_json::Value` rather than a JSON
    /// `String`: nice-plug serialises each persisted field to a string again, so a `String` would
    /// be escaped twice.
    #[persist = "patch"]
    pub patch: RwLock<serde_json::Value>,
}

fn unit_range() -> FloatRange {
    FloatRange::Linear { min: 0.0, max: 1.0 }
}

impl Default for ShellParams {
    fn default() -> Self {
        let mut number = 0;
        let macros = [(); NUM_MACROS].map(|()| {
            number += 1;
            let default = if number == 1 { 0.5 } else { 0.0 };
            let mut value = FloatParam::new(format!("Macro {number}"), default, unit_range())
                .with_poly_modulation_id(number - 1);
            if number == 1 {
                value = value.with_smoother(SmoothingStyle::Linear(10.0));
            }
            MacroParam { value }
        });
        let mut number = 0;
        let slots = [(); NUM_SLOTS].map(|()| {
            number += 1;
            let mut value = FloatParam::new(format!("Slot {number}"), 0.5, unit_range());
            if number == 1 {
                value = value.with_smoother(SmoothingStyle::Linear(10.0));
            }
            SlotParam { value }
        });
        Self {
            macros,
            slots,
            patch: RwLock::new(default_patch()),
        }
    }
}

fn default_patch() -> serde_json::Value {
    serde_json::json!({ "version": 1, "modules": [] })
}

/// Stamps a nonce into the patch blob the first time the plugin is activated without one. A
/// fresh instance has none, so seeing the same nonce after a project reload proves the blob came
/// back from the host's saved state. Test hook for `cargo xtask reaper-test`.
fn stamp_nonce(patch: &mut serde_json::Value) {
    if let Some(object) = patch.as_object_mut()
        && !object.contains_key("nonce")
    {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        object.insert("nonce".to_owned(), nanos.to_string().into());
    }
}

pub struct SpikeShell {
    params: Arc<ShellParams>,
    synth: Synth,
    editor_state: Arc<EguiEditorState>,
    repaint: RepaintNotifier,
}

impl Default for SpikeShell {
    fn default() -> Self {
        Self {
            params: Arc::new(ShellParams::default()),
            synth: Synth::new(48_000.0),
            editor_state: EguiEditorState::from_size(LogicalSize::new(360.0, 200.0), 1.0),
            repaint: RepaintNotifier::new(),
        }
    }
}

/// The voice id for a new note: the host's id when it gives one (needed for polyphonic modulation),
/// else one built from channel and key. Not `VoiceID::id_or_fallback`: in nice-plug 0.4.2 that
/// ignores the host's id and always computes the fallback.
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

impl SpikeShell {
    fn handle_event(&mut self, event: NoteEvent<()>, context: &mut impl ProcessContext<Self>) {
        let timing = event.timing();
        // A failed send only means the host's event queue is full; nothing to do about it here.
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

impl Plugin for SpikeShell {
    const NAME: &'static str = "enth spike-shell";
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

    type Editor = EguiEditor<Ui>;
    type SysExMessage = ();
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn editor(&mut self, _async_executor: AsyncExecutor<Self>) -> Option<Self::Editor> {
        create_egui_editor(
            self.editor_state.clone(),
            self.repaint.clone(),
            EguiNiceSettings::new(),
            Ui {
                params: self.params.clone(),
                gui: None,
            },
        )
    }

    fn activate(
        &mut self,
        _layout: &AudioIOLayout,
        config: &BufferConfig,
        _context: &mut impl ActivateContext<Self>,
    ) -> bool {
        self.synth.set_sample_rate(config.sample_rate);
        if let Ok(mut patch) = self.params.patch.write() {
            stamp_nonce(&mut patch);
        }
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
        }
        ProcessStatus::KeepAlive
    }
}

impl ClapPlugin for SpikeShell {
    const CLAP_ID: &'static str = CLAP_ID;
    const CLAP_DESCRIPTION: Option<&'static str> =
        Some("Phase 0 shell spike: 8 macros, 64 slots, JSON patch blob");
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

impl Vst3Plugin for SpikeShell {
    const VST3_CLASS_ID: [u8; 16] = VST3_CLASS_ID;
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] =
        &[Vst3SubCategory::Instrument, Vst3SubCategory::Synth];
}

nice_export_clap!(SpikeShell);
nice_export_vst3!(SpikeShell);

/// Placeholder editor: two sliders and the patch nonce. The real panel is a separate spike.
pub struct Ui {
    params: Arc<ShellParams>,
    gui: Option<GuiContext>,
}

impl NiceEguiApp for Ui {
    fn build(
        &mut self,
        _egui_ctx: egui::Context,
        gui: GuiContext,
        _frame: &mut Frame,
    ) -> Result<(), HandlerError> {
        self.gui = Some(gui);
        Ok(())
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut Frame) {
        let Some(gui) = &self.gui else { return };
        let setter = gui.param_setter();
        ui.heading("enth spike-shell");
        ui.add(widgets::ParamSlider::for_param(
            &self.params.macros[GAIN_MACRO].value,
            &setter,
        ));
        ui.add(widgets::ParamSlider::for_param(
            &self.params.slots[CUTOFF_SLOT].value,
            &setter,
        ));
        let nonce = self
            .params
            .patch
            .read()
            .ok()
            .and_then(|patch| patch["nonce"].as_str().map(str::to_owned));
        ui.label(format!("patch nonce: {}", nonce.as_deref().unwrap_or("-")));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_sees_72_params_with_stable_ids_and_names() {
        let params = ShellParams::default();
        let map = params.param_map();
        assert_eq!(map.len(), NUM_MACROS + NUM_SLOTS);
        let expected_ids = (1..=NUM_MACROS)
            .map(|n| format!("macro_{n}"))
            .chain((1..=NUM_SLOTS).map(|n| format!("slot_{n}")));
        for ((id, ptr, group), expected) in map.iter().zip(expected_ids) {
            assert_eq!(*id, expected);
            assert_eq!(group, "", "{id} must not be grouped");
            // SAFETY: the pointer points into `params`, which outlives this loop.
            let name = unsafe { ptr.name().to_owned() };
            let (kind, n) = id.split_once('_').unwrap();
            assert_eq!(
                name,
                format!("{} {n}", if kind == "macro" { "Macro" } else { "Slot" })
            );
        }
        let ids: Vec<&str> = map.iter().map(|(id, _, _)| id.as_str()).collect();
        assert_eq!(ids.first(), Some(&"macro_1"));
        assert_eq!(ids[7], "macro_8");
        assert_eq!(ids[8], "slot_1");
        assert_eq!(ids.last(), Some(&"slot_64"));
    }

    #[test]
    fn only_macros_take_polyphonic_modulation() {
        let params = ShellParams::default();
        for (n, m) in params.macros.iter().enumerate() {
            assert_eq!(m.value.poly_modulation_id(), Some(n as u32));
        }
        for s in &params.slots {
            assert_eq!(s.value.poly_modulation_id(), None);
        }
    }

    #[test]
    fn arbitrary_json_patch_round_trips_through_saved_state() {
        let original = ShellParams::default();
        let blob = serde_json::json!({
            "version": 1,
            "nonce": "42",
            "modules": [{ "kind": "osc", "wave": "saw", "tune": [0.5, -12, null], "text": "q\"uote \u{e9}" }],
            "slots": { "1": "filter.cutoff" },
        });
        *original.patch.write().unwrap() = blob.clone();
        let saved = original.serialize_fields();
        assert_eq!(
            saved.keys().map(String::as_str).collect::<Vec<_>>(),
            ["patch"]
        );

        let restored = ShellParams::default();
        restored.deserialize_fields(&saved);
        assert_eq!(*restored.patch.read().unwrap(), blob);
    }

    #[test]
    fn nonce_is_stamped_once() {
        let mut patch = default_patch();
        stamp_nonce(&mut patch);
        let first = patch["nonce"].clone();
        assert!(first.as_str().is_some_and(|n| !n.is_empty()));
        stamp_nonce(&mut patch);
        assert_eq!(patch["nonce"], first);
    }

    #[test]
    fn plugin_ids_are_permanent() {
        assert_eq!(<SpikeShell as ClapPlugin>::CLAP_ID, "dev.enth.spike-shell");
        assert_eq!(
            <SpikeShell as Vst3Plugin>::VST3_CLASS_ID,
            *b"EnthSpikeShell01"
        );
    }

    #[test]
    fn host_voice_id_wins_over_the_fallback() {
        let key = voice_key(VoiceID::ID(1234), Channel::Number(2), Key::Number(60));
        assert_eq!((key.voice_id, key.channel, key.key), (1234, 2, 60));
        let fallback = voice_key(VoiceID::Wildcard, Channel::Number(2), Key::Number(60));
        assert_eq!(fallback.voice_id, 0x0002_003c);
    }
}
