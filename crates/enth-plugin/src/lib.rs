//! enth-plugin (L8, see docs/architecture.md and docs/plugin.md): the CLAP and VST3 shell around `enth-session`,
//! `enth-engine` and `enth-ui`, built on nice-plug (docs/adr/0001-plugin-shell.md). It translates between the host
//! and the product and holds no product rules: parameters, notes, audio blocks, state and the editor window.
//!
//! Threads (the full contract is in docs/session.md, the shell's side in docs/plugin.md):
//!
//! - **Audio thread** (`process`, `reset`): owns the [`Engine`](enth_engine::Engine), a preallocated event list and
//!   the host values it last sent. It never locks and never touches the session. Host automation goes to the
//!   engine as `EventKind::Slot` and `EventKind::Macro` (never a bare `Param`); the one thing it schedules is the
//!   rate-limited background pump.
//! - **Host main thread** (`activate`, `deactivate`, state save and load through the persistent fields) and
//!   **baseview's GUI thread** (the editor) share the `Session` behind one `parking_lot::Mutex` in [`Shared`].
//!   Whoever locks drains the session's requests before unlocking and acts on them only after unlocking.
//! - **nice-plug's background worker** runs [`Task::Pump`]: lock, pump, unlock.
//!
//! Never call `GuiContext::set_state` (nice-plug would load the state on the audio thread) and never schedule GUI
//! work from `process()`; `clippy.toml` bans both.
//!
//! | Module | Holds | Lane |
//! | --- | --- | --- |
//! | [`params`] | the 72 host parameters (8 macros, 64 automation slots), stable ids | SK |
//! | [`process`] | the audio block: events and notes in, the engine, audio and `NOTE_END` out, the pump flag | SK, PL |
//! | [`notes`] | the note tuple, `NOTE_END`, polyphonic modulation with host voice ids | SK, PL |
//! | [`ledger`] | the host notes not reported ended yet: exactly one `NOTE_END` per host note | PL |
//! | [`state`] | the persistent fields bridging nice-plug's state and the session blob | SK |
//! | [`editor`] | the `NiceEguiApp` window around the `enth-ui` editor | SK, ED1 |
//! | `rt_stats` | with the `rt-stats` cargo feature: a timing histogram of `process()` | PL |
//! | `editor_counts` | with the `editor-counts` cargo feature: editor opens, `editor_closed` calls and drawn frames, for the leak gate and owner check 1.3 | INT, AM |
//! | [`game`] | a Recreate round's clock, worker, target and phrase players, A/B crossfade, round log | SKR (R1/S1; pre-declared by S0r) |
//! | [`presets`] | the preset files: the user folder, the lazy scan, saving `Patch::to_json()` | PBC (R1/S1; pre-declared by S0r) |
#![allow(
    unsafe_code,
    reason = "nice_export_clap! and nice_export_vst3! expand to extern \"C\" entry points"
)]

pub mod editor;
#[cfg(feature = "editor-counts")]
pub mod editor_counts;
pub mod game;
pub mod ledger;
pub mod notes;
pub mod params;
pub mod presets;
pub mod process;
#[cfg(feature = "rt-stats")]
pub mod rt_stats;
pub mod state;
#[cfg(test)]
mod tests_host;

use std::num::NonZeroU32;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock, Weak};

use enth_engine::Engine;
use enth_session::{Activation, HostParam, HostValues, Session, ShellRequest};
use nice_plug::editor::dpi::LogicalSize;
use nice_plug::prelude::*;
use nice_plug_egui::{EguiEditor, EguiEditorState, RepaintNotifier};
use parking_lot::Mutex;

use crate::params::EnthParams;
use crate::process::Audio;

/// Permanent CLAP id (docs/naming.md). Hosts key saved projects and presets by it: never change it.
pub const CLAP_ID: &str = "dev.enth.synth";

/// Permanent VST3 class id `55555EAD-AA77-C9FA-F878-DDDDB0B9D9BD` (docs/naming.md): never change it.
pub const VST3_CLASS_ID: [u8; 16] = [
    0x55, 0x55, 0x5E, 0xAD, 0xAA, 0x77, 0xC9, 0xFA, 0xF8, 0x78, 0xDD, 0xDD, 0xB0, 0xB9, 0xD9, 0xBD,
];

/// The background tasks `process()` and the shell schedule on nice-plug's worker (never inline).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Task {
    /// Lock the session and pump it: a compile left pending while nobody else enters the session (docs/session.md,
    /// "Plan delivery"), and the editor-closed duties.
    Pump,
}

/// What the host main thread, the GUI thread and the background worker share. The audio thread reads only the
/// atomics.
pub struct Shared {
    /// The one lock: host main thread, GUI thread, background worker; never the audio thread.
    pub session: Mutex<Session>,
    /// A compile is pending: `process()` schedules [`Task::Pump`] at most every [`process::PUMP_INTERVAL_S`].
    pub pump_wanted: AtomicBool,
    /// The monitor FX bypass `process()` hands the engine (ADR-0006).
    pub monitor_bypass: AtomicBool,
    /// A state load happened: `process()` takes the values it restored ([`Shared::restored`]) as its "last sent"
    /// without sending them, so values restored with the state never override the loaded patch in the engine, while
    /// a host change after the load still reaches it (docs/plugin.md).
    pub rebaseline: AtomicBool,
    /// The host values the last state load restored, for [`Shared::rebaseline`].
    pub restored: process::Restored,
    /// The editor window closed (seen by `process()` through `EguiEditorState::is_open`): the next pump releases
    /// audition notes.
    pub editor_closed: AtomicBool,
    /// The parameters, for reading the host's values on the main and GUI threads (weak: the params own the
    /// persistent fields, which own this).
    params: OnceLock<Weak<EnthParams>>,
    /// nice-plug's handle for host requests outside a context (lane FK2's fork patches 6 and 8): marking the
    /// project dirty and the parameter-info rescan. Set once by the wrapper right after construction, before any
    /// state load; absent in tests that do not set it.
    host: OnceLock<HostRequests>,
    /// `process()` timings (the `rt-stats` cargo feature).
    #[cfg(feature = "rt-stats")]
    pub rt_stats: rt_stats::RtStats,
    /// Editor opens, `editor_closed` calls and drawn frames (the `editor-counts` cargo feature), reported when this
    /// drops.
    #[cfg(feature = "editor-counts")]
    pub editor_counts: editor_counts::EditorCounts,
}

/// The last reference went: the plugin and every editor window are gone, so the counts are final.
#[cfg(feature = "editor-counts")]
impl Drop for Shared {
    fn drop(&mut self) {
        self.editor_counts.write_report();
    }
}

impl Shared {
    fn new(session: Session) -> Self {
        Shared {
            session: Mutex::new(session),
            pump_wanted: AtomicBool::new(false),
            monitor_bypass: AtomicBool::new(false),
            rebaseline: AtomicBool::new(false),
            restored: process::Restored::new(),
            editor_closed: AtomicBool::new(false),
            params: OnceLock::new(),
            host: OnceLock::new(),
            #[cfg(feature = "rt-stats")]
            rt_stats: rt_stats::RtStats::new(),
            #[cfg(feature = "editor-counts")]
            editor_counts: editor_counts::EditorCounts::new(),
        }
    }

    /// The host's 72 unmodulated values, if the parameters still exist.
    pub fn host_values(&self) -> Option<HostValues> {
        self.params
            .get()
            .and_then(Weak::upgrade)
            .map(|p| p.host_values())
    }

    /// Lock the session, run `f`, drain its requests and note whether a compile is left pending, unlock; then act
    /// on the requests that need no host call ([`Shared::settle`]) and return the host writes for the caller to
    /// forward (only the editor produces them, on the GUI thread).
    ///
    /// On a `ShellRequest::RenameParams` it also sets the 72 display names from the patch **before** unlocking, so
    /// two threads that rename one after the other (the GUI's edit, the host's load) leave the later patch's names.
    /// `Param::set_display_name` only stores a string behind the parameter's own lock, which nothing else holds while
    /// calling out, so this never calls the host under the session lock. The rescan request comes after unlocking
    /// ([`Shared::settle`]).
    pub fn with_session<R>(&self, f: impl FnOnce(&mut Session) -> R) -> (R, Vec<ShellRequest>) {
        let (result, requests, pending) = {
            let mut session = self.session.lock();
            let result = f(&mut session);
            let requests: Vec<ShellRequest> = session.drain_requests().collect();
            if requests
                .iter()
                .any(|r| matches!(r, ShellRequest::RenameParams))
            {
                self.set_names(session.patch());
            }
            (result, requests, session.status().compile_pending)
        };
        let host = self.settle(requests, pending);
        (result, host)
    }

    /// The host parameters' display names for `patch` (`enth_session::bridge::host_param_name`: "Osc 1 · Cutoff" for
    /// a slot with a target, a macro's own name), set through the fork; a name equal to the one fixed at construction
    /// ("Slot 7", "Macro 2") clears the override. Never on the audio thread (it allocates and locks).
    fn set_names(&self, patch: &enth_patch::Patch) {
        let Some(params) = self.params.get().and_then(Weak::upgrade) else {
            return;
        };
        for p in HostParam::all() {
            let param = params.param(p);
            let name = enth_session::bridge::host_param_name(patch, p);
            param.set_display_name((name != param.name()).then_some(name));
        }
    }

    /// After unlocking: store the monitor bypass, ask the host to re-read renamed parameters, mark the host project
    /// dirty after an edit the host cannot see, arm the pump flag if a compile is pending, and keep only the host
    /// writes.
    ///
    /// `ShellRequest::RenameParams` becomes [`HostRequests::request_param_info_rescan`] (CLAP `rescan(INFO | TEXT)`,
    /// VST3 `restartComponent(kParamTitlesChanged)`, which REAPER ignores while reading titles live). This runs on the
    /// host's main thread (a state load: the fork performs the rescan there and then, before the host processes the
    /// loaded state) or the GUI thread (an editor rename: queued for the main thread), never in `process()`, which may
    /// run on the host's main thread in an offline render (design.md §4).
    ///
    /// `ShellRequest::PatchEdited` (an editor edit, undo or redo: the session pushes it once per drain) becomes
    /// [`HostRequests::mark_dirty`]: CLAP `clap_host_state.mark_dirty`, VST3 `IComponentHandler2::setDirty(true)`, which
    /// the fork performs on the host's main thread (queued from the GUI thread; one waiting request absorbs the next).
    /// A slot-bound edit also moves a host parameter, which dirties the project anyway; marking it again is harmless.
    fn settle(&self, requests: Vec<ShellRequest>, compile_pending: bool) -> Vec<ShellRequest> {
        if compile_pending {
            self.pump_wanted.store(true, Ordering::Release);
        }
        let mut host = Vec::new();
        for request in requests {
            match request {
                ShellRequest::MonitorBypass(on) => self.monitor_bypass.store(on, Ordering::Release),
                ShellRequest::RenameParams => {
                    if let Some(requests) = self.host.get() {
                        requests.request_param_info_rescan();
                    }
                }
                ShellRequest::Host(_) => host.push(request),
                ShellRequest::PatchEdited => {
                    if let Some(requests) = self.host.get() {
                        requests.mark_dirty();
                    }
                }
                _ => {}
            }
        }
        host
    }

    /// The background pump ([`Task::Pump`]): editor-closed duties, then a pump; re-arms the flag while a compile is
    /// still pending. Host writes cannot come out of it (only editor edits make them).
    fn background_pump(&self) {
        let closed = self.editor_closed.swap(false, Ordering::AcqRel);
        let ((), host) = self.with_session(|s| {
            if closed {
                let _ = s.audition_all_off();
            }
            s.pump();
        });
        debug_assert!(host.is_empty(), "only editor edits make host writes");
    }
}

/// The plugin: the audio thread's state, plus the shared session.
pub struct Enth {
    params: Arc<EnthParams>,
    shared: Arc<Shared>,
    editor_state: Arc<EguiEditorState>,
    repaint: RepaintNotifier,
    engine: Option<Engine>,
    audio: Audio,
}

impl Default for Enth {
    fn default() -> Self {
        let shared = Arc::new(Shared::new(Session::default()));
        let params = Arc::new(EnthParams::new(&shared));
        // Set once, here: nothing else can have set it.
        let _ = shared.params.set(Arc::downgrade(&params));
        let size = enth_ui::Theme::dark().size.window.default;
        Enth {
            params,
            shared,
            editor_state: EguiEditorState::from_size(LogicalSize::new(size.x, size.y), 1.0),
            repaint: RepaintNotifier::new(),
            engine: None,
            audio: Audio::new(),
        }
    }
}

impl Enth {
    /// The shared session (tests and the editor).
    pub fn shared(&self) -> &Arc<Shared> {
        &self.shared
    }

    /// The host parameters.
    pub fn host_params(&self) -> &Arc<EnthParams> {
        &self.params
    }
}

impl Plugin for Enth {
    const NAME: &'static str = "enth";
    const VENDOR: &'static str = "enth";
    const URL: &'static str = "";
    const EMAIL: &'static str = "";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[AudioIOLayout {
        main_input_channels: None,
        main_output_channels: NonZeroU32::new(2),
        ..AudioIOLayout::const_default()
    }];
    // `MidiCCs`: the mod wheel (MIDI CC 1) reaches the engine. For VST3 nice-plug maps each controller in
    // `VST3_MIDI_CONTROLLERS` to a hidden parameter per MIDI channel, which REAPER lists; CLAP pays nothing. Lane PL's
    // decision, with the measurements, in docs/plugin.md ("MIDI CCs"); `Basic` turns the mod wheel off in both formats.
    const MIDI_INPUT: MidiConfig = MidiConfig::MidiCCs;
    // Required: without it plugin renders stop matching CLI renders at large host blocks (AGENTS.md).
    const SAMPLE_ACCURATE_AUTOMATION: bool = true;

    type Editor = EguiEditor<editor::EditorApp>;
    type SysExMessage = ();
    type BackgroundTask = Task;

    fn task_executor(&mut self) -> TaskExecutor<Self> {
        let shared = Arc::clone(&self.shared);
        Box::new(move |task| match task {
            Task::Pump => shared.background_pump(),
        })
    }

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    /// The fork's host requests (marking the project dirty, the parameter-info rescan), kept for [`Shared::settle`].
    /// Called once, right after construction.
    fn host_requests(&mut self, requests: HostRequests) {
        let _ = self.shared.host.set(requests);
    }

    fn editor(&mut self, _async_executor: AsyncExecutor<Self>) -> Option<Self::Editor> {
        editor::create(
            Arc::clone(&self.shared),
            Arc::clone(&self.params),
            Arc::clone(&self.editor_state),
            self.repaint.clone(),
        )
    }

    fn activate(
        &mut self,
        _audio_io_layout: &AudioIOLayout,
        buffer_config: &BufferConfig,
        context: &mut impl ActivateContext<Self>,
    ) -> bool {
        let activation = Activation {
            sample_rate: buffer_config.sample_rate,
            offline: buffer_config.process_mode == ProcessMode::Offline,
        };
        let host = self.params.host_values();
        let (result, writes) = self.shared.with_session(|s| {
            // Fold first, so the engine starts from the patch the host sees (rule 1: the first fold after a load
            // only records).
            s.fold_host(&host);
            let bypass = s.instance().monitor.fx_bypass;
            s.activate(activation).map(|engine| (engine, bypass))
        });
        debug_assert_eq!(writes.len(), 0, "only editor edits make host writes");
        // Only a sample rate outside the engine's range (8–384 kHz) fails: a valid patch always compiles. Then the
        // plugin plays silence rather than refuse to activate (clap-validator sweeps 1 kHz to 768 kHz); the session
        // stays detached and edits still work. The previous engine, if any, is dropped here on the main thread.
        self.engine = result.ok().map(|(engine, bypass)| {
            self.shared.monitor_bypass.store(bypass, Ordering::Release);
            engine
        });
        // CLAP voice info: the patch's voice count (its polyphony, or one in mono and legato), on the main thread.
        let voices = self.engine.as_ref().map_or(0, process::voice_count);
        if voices > 0 {
            context.set_current_voice_capacity(voices);
        }
        // Nothing is echoed: the engine starts from the patch and hears only later host changes. That is also the
        // re-baseline a state load before activation asked for, so a change the host makes in the first block (a
        // `PARAM_VALUE` at frame 0) still reaches the engine.
        self.audio
            .activate(&self.params, buffer_config.sample_rate, voices);
        self.shared.rebaseline.store(false, Ordering::Release);
        true
    }

    fn deactivate(&mut self) {
        let ((), writes) = self.shared.with_session(Session::deactivate);
        debug_assert_eq!(writes.len(), 0, "only editor edits make host writes");
        self.engine = None;
        #[cfg(feature = "rt-stats")]
        self.shared.rt_stats.write_report();
    }

    fn reset(&mut self) {
        if let Some(engine) = self.engine.as_mut() {
            // CLAP reset: the engine frees its voices without reporting them, so the next block reports them.
            self.audio.note_reset();
            engine.reset();
        }
    }

    fn process(
        &mut self,
        buffer: &mut Buffer<'_>,
        _aux: &mut AuxiliaryBuffers<'_>,
        context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        self.process_block(buffer, context)
    }
}

impl ClapPlugin for Enth {
    const CLAP_ID: &'static str = CLAP_ID;
    const CLAP_DESCRIPTION: Option<&'static str> = Some("A synthesizer");
    const CLAP_MANUAL_URL: Option<&'static str> = None;
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[
        ClapFeature::Instrument,
        ClapFeature::Synthesizer,
        ClapFeature::Stereo,
    ];
    const CLAP_POLY_MODULATION_CONFIG: Option<PolyModulationConfig> = Some(PolyModulationConfig {
        max_voice_capacity: enth_engine::MAX_VOICES as u32,
        supports_overlapping_voices: true,
    });
    // No restart when the host switches to offline rendering while the plugin is active (the fork's patch 9). REAPER
    // does that at the start of every offline render and restarts the plugin about 30 ms of wall-clock time later,
    // mid-render: a fresh engine continues from a point that depends on the render's speed, and the note sounding
    // there is lost (the `macro-automation` flake). The engine renders the same in both modes; only the session's
    // audition refusal reads the mode, at activation (docs/plugin.md, "Offline renders").
    const CLAP_RESTART_ON_RENDER_MODE_CHANGE: bool = false;
}

impl Vst3Plugin for Enth {
    const VST3_CLASS_ID: [u8; 16] = VST3_CLASS_ID;
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] =
        &[Vst3SubCategory::Instrument, Vst3SubCategory::Synth];
    // The MIDI controllers `notes::engine_event` reads, and no others (the fork's patch 7): VST3 delivers a CC only
    // through a parameter that `IMidiMapping` maps it to, one per controller and MIDI channel, and REAPER lists every
    // one. CC 1 on 16 channels is 16 parameters, where nice-plug's default (all 130 controllers) is 2,080. A test keeps
    // this list and `engine_event` in step; add pitch bend (129) here when the engine takes it.
    const VST3_MIDI_CONTROLLERS: &'static [u8] = &[nice_plug::midi::control_change::MODULATION_MSB];
}

/// The CLAP and VST3 entry points. In a module of their own so the allow below covers only nice-plug's expansion.
#[allow(
    clippy::disallowed_types,
    reason = "nice-plug's export macros use std's HashSet internally; nothing here iterates it"
)]
mod export {
    use super::Enth;
    use nice_plug::prelude::*;

    nice_export_clap!(Enth);
    nice_export_vst3!(Enth);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugin_ids_are_permanent() {
        assert_eq!(<Enth as ClapPlugin>::CLAP_ID, "dev.enth.synth");
        // docs/naming.md: 55555EAD-AA77-C9FA-F878-DDDDB0B9D9BD.
        let hex = <Enth as Vst3Plugin>::VST3_CLASS_ID
            .iter()
            .fold(String::new(), |mut s, b| {
                use std::fmt::Write;
                let _ = write!(s, "{b:02X}");
                s
            });
        assert_eq!(hex, "55555EADAA77C9FAF878DDDDB0B9D9BD");
    }

    #[test]
    fn sample_accurate_automation_is_on() {
        const { assert!(<Enth as Plugin>::SAMPLE_ACCURATE_AUTOMATION) };
    }

    /// A host that counts the fork's requests.
    #[derive(Default)]
    struct CountingHost {
        dirty: std::sync::atomic::AtomicU32,
    }

    impl nice_plug::context::host::HostRequestsInner for CountingHost {
        fn mark_dirty(&self) {
            self.dirty.fetch_add(1, Ordering::SeqCst);
        }

        fn request_param_info_rescan(&self) {}
    }

    #[test]
    fn edits_undo_and_redo_mark_the_host_project_dirty_and_loads_and_folds_do_not() {
        let mut plugin = Enth::default();
        let host = Arc::new(CountingHost::default());
        plugin.host_requests(HostRequests::new(host.clone()));
        let dirty = || host.dirty.load(Ordering::SeqCst);
        let shared = Arc::clone(plugin.shared());
        let level = enth_patch::ParamAddr::module(
            enth_patch::Patch::init().generators[0].id.clone(),
            "level",
        )
        .unwrap();
        let edit = |value: f32| {
            let (applied, writes) = shared.with_session(|s| {
                enth_session::SessionEdit::edit(
                    s,
                    vec![enth_patch::Op::SetParam {
                        addr: level.clone(),
                        value,
                    }],
                    None,
                )
            });
            assert!(applied.is_ok());
            assert!(
                writes.is_empty(),
                "an unslotted address makes no host writes"
            );
        };
        edit(40.0);
        assert_eq!(dirty(), 1, "an editor edit the host cannot see");
        edit(30.0);
        assert_eq!(dirty(), 2);
        let ((), _) = shared.with_session(|s| {
            assert_eq!(enth_session::SessionEdit::undo(s), Ok(true));
        });
        assert_eq!(dirty(), 3, "undo");
        let ((), _) = shared.with_session(|s| {
            assert_eq!(enth_session::SessionEdit::redo(s), Ok(true));
        });
        assert_eq!(dirty(), 4, "redo");
        // The host loads a state and folds its values: it knows about those, so nothing is marked.
        let mut fields = std::collections::BTreeMap::new();
        fields.insert("patch".to_owned(), enth_patch::Patch::init().to_json());
        plugin.host_params().deserialize_fields(&fields);
        let values = HostValues::default();
        let _ = shared.with_session(|s| s.fold_host(&values));
        assert_eq!(dirty(), 4, "a host load and a fold mark nothing");
    }

    /// Every MIDI controller the plugin turns into an engine event is one the VST3 wrapper maps (nice-plug delivers
    /// no other CC there), and the list holds nothing the plugin ignores.
    #[test]
    fn the_vst3_midi_controllers_are_exactly_the_ones_the_plugin_reads() {
        let reads = |controller: u8| {
            let event: NoteEvent<()> = match controller {
                0..=127 => NoteEvent::MidiCC {
                    timing: 0,
                    channel: 3,
                    cc: controller,
                    value: 0.5,
                },
                128 => NoteEvent::MidiChannelPressure {
                    timing: 0,
                    channel: 3,
                    pressure: 0.5,
                },
                _ => NoteEvent::MidiPitchBend {
                    timing: 0,
                    channel: 3,
                    value: 0.5,
                },
            };
            notes::engine_event(&event).is_some()
        };
        let read: Vec<u8> = (0..130).filter(|&c| reads(c)).collect();
        assert_eq!(read, <Enth as Vst3Plugin>::VST3_MIDI_CONTROLLERS);
    }

    #[test]
    fn the_session_is_shared_with_the_params() {
        let plugin = Enth::default();
        let host = plugin.shared().host_values().expect("params are alive");
        assert_eq!(host, HostValues::default());
    }
}
