//! Tests the host logic against plugins built in this file (loaded in-process through clack), so the
//! `cargo test` run needs neither a plugin bundle nor a display. The real plugin is covered by
//! `cargo xtask host-render`.

use std::sync::atomic::{AtomicU32, Ordering};

use clack_extensions::audio_ports::{
    AudioPortFlags, AudioPortInfo, AudioPortInfoWriter, AudioPortType, PluginAudioPorts,
    PluginAudioPortsImpl,
};
use clack_extensions::note_ports::{
    NoteDialect, NoteDialects, NotePortInfo, NotePortInfoWriter, PluginNotePorts,
    PluginNotePortsImpl,
};
use clack_host::prelude::PluginEntry;
use clack_plugin::events::spaces::CoreEventSpace;
use clack_plugin::prelude::*;
use enth_test_host::instances;
use enth_test_host::query;
use enth_test_host::render::{ProcessThread, RenderConfig, RenderRequest, render_once};

/// How many instances have been activated, process-wide. `LEAKY` plugins let it change their sound:
/// the shared state `instances` exists to catch.
static ACTIVATIONS: AtomicU32 = AtomicU32::new(0);

/// A saw synth: note-on starts one monophonic voice at the key's pitch, note-off stops it, reset kills it.
struct Tone<const LEAKY: bool>;

struct ToneMainThread;

impl PluginMainThread<'_, ()> for ToneMainThread {}

impl PluginAudioPortsImpl for ToneMainThread {
    fn count(&self, is_input: bool) -> u32 {
        u32::from(!is_input)
    }

    fn get(&self, index: u32, is_input: bool, writer: &mut AudioPortInfoWriter<'_>) {
        if index == 0 && !is_input {
            writer.set(&AudioPortInfo {
                id: ClapId::new(0),
                name: b"Output",
                channel_count: 2,
                flags: AudioPortFlags::IS_MAIN,
                port_type: Some(AudioPortType::STEREO),
                in_place_pair: None,
            });
        }
    }
}

impl PluginNotePortsImpl for ToneMainThread {
    fn count(&self, is_input: bool) -> u32 {
        u32::from(is_input)
    }

    fn get(&self, index: u32, is_input: bool, writer: &mut NotePortInfoWriter<'_>) {
        if index == 0 && is_input {
            writer.set(&NotePortInfo {
                id: ClapId::new(0),
                name: b"Notes",
                supported_dialects: NoteDialects::CLAP,
                preferred_dialect: Some(NoteDialect::Clap),
            });
        }
    }
}

struct ToneProcessor<const LEAKY: bool> {
    sample_rate: f32,
    phase: f32,
    increment: f32,
    note_id: Option<u32>,
    gain: f32,
}

impl<const LEAKY: bool> Plugin for Tone<LEAKY> {
    type AudioProcessor<'a> = ToneProcessor<LEAKY>;
    type Shared<'a> = ();
    type MainThread<'a> = ToneMainThread;

    fn declare_extensions(builder: &mut PluginExtensions<'_, Self>, _shared: Option<&()>) {
        builder
            .register::<PluginAudioPorts>()
            .register::<PluginNotePorts>();
    }
}

impl<const LEAKY: bool> DefaultPluginFactory for Tone<LEAKY> {
    fn get_descriptor() -> PluginDescriptor {
        use clack_plugin::plugin::features::{INSTRUMENT, STEREO, SYNTHESIZER};
        let id = if LEAKY { "test.leaky" } else { "test.tone" };
        PluginDescriptor::new(id, "Test tone").with_features([INSTRUMENT, SYNTHESIZER, STEREO])
    }

    fn new_shared(_host: HostSharedHandle<'_>) -> Result<(), PluginError> {
        Ok(())
    }

    fn new_main_thread<'a>(
        _host: HostMainThreadHandle<'a>,
        _shared: &'a (),
    ) -> Result<ToneMainThread, PluginError> {
        Ok(ToneMainThread)
    }
}

impl<'a, const LEAKY: bool> PluginAudioProcessor<'a, (), ToneMainThread> for ToneProcessor<LEAKY> {
    fn activate(
        _host: HostAudioProcessorHandle<'a>,
        _main_thread: &ToneMainThread,
        _shared: &'a (),
        config: PluginAudioConfiguration,
    ) -> Result<Self, PluginError> {
        let n = ACTIVATIONS.fetch_add(1, Ordering::SeqCst);
        Ok(Self {
            sample_rate: config.sample_rate as f32,
            phase: 0.0,
            increment: 0.0,
            note_id: None,
            // A well-behaved plugin ignores `n`; the leaky one lets the instance count change its level.
            gain: if LEAKY { 0.1 + n as f32 * 0.01 } else { 0.1 },
        })
    }

    fn process(
        &mut self,
        _process: Process<'_>,
        mut audio: Audio<'_>,
        events: Events<'_>,
    ) -> Result<ProcessStatus, PluginError> {
        let mut pending = events.input.iter().peekable();
        let Some(mut port) = audio.port_pair(0) else {
            return Ok(ProcessStatus::Continue);
        };
        let channels = port
            .channels()?
            .into_f32()
            .ok_or(PluginError::Message("f32 only"))?;
        let frames = channels.frames_count() as usize;
        let mut mono = vec![0.0f32; frames];
        for (frame, out) in mono.iter_mut().enumerate() {
            while let Some(event) = pending.next_if(|e| e.header().time() as usize <= frame) {
                match event.as_core_event() {
                    Some(CoreEventSpace::NoteOn(on)) => {
                        let key = on.key().into_specific().unwrap_or(60);
                        self.increment =
                            440.0 * ((f32::from(key) - 69.0) / 12.0).exp2() / self.sample_rate;
                        self.note_id = on.note_id().into_specific();
                    }
                    Some(CoreEventSpace::NoteOff(off))
                        if off.note_id().into_specific() == self.note_id =>
                    {
                        self.note_id = None;
                    }
                    _ => {}
                }
            }
            if self.note_id.is_some() {
                self.phase = (self.phase + self.increment).fract();
                *out = (self.phase * 2.0 - 1.0) * self.gain;
            }
        }
        for channel in channels {
            if let ChannelPair::OutputOnly(buf) = channel {
                buf.copy_from_slice(&mono);
            }
        }
        Ok(ProcessStatus::Continue)
    }

    fn reset(&mut self) {
        self.note_id = None;
    }
}

fn entry<const LEAKY: bool>() -> (PluginEntry, std::ffi::CString) {
    let entry = PluginEntry::load_from_clack::<SinglePluginEntry<Tone<LEAKY>>>(c"/test/tone.clap")
        .expect("the in-process entry loads");
    let id = enth_test_host::host::resolve_plugin_id(&entry, None).expect("one plugin");
    (entry, id)
}

fn request(script: &str, dur: f64) -> RenderRequest {
    RenderRequest {
        script: script.to_owned(),
        params: Vec::new(),
        state: None,
        config: RenderConfig::new(48_000.0, 64, dur).unwrap(),
    }
}

#[test]
fn info_reports_ports_and_descriptor() {
    let (entry, id) = entry::<false>();
    let report = query::plugin_report(&entry, &id).unwrap();
    assert_eq!(report["descriptor"]["id"], "test.tone");
    assert_eq!(report["audio_ports"]["outputs"][0]["channels"], 2);
    assert_eq!(report["audio_ports"]["inputs"].as_array().unwrap().len(), 0);
    assert_eq!(report["note_ports"]["inputs"].as_array().unwrap().len(), 1);
    assert_eq!(report["extensions"]["gui"], false);
}

#[test]
fn a_render_sounds_when_the_note_starts_and_not_before() {
    let (entry, id) = entry::<false>();
    let outcome = render_once(
        &entry,
        &id,
        &request("A4@0.1+0.2", 0.5),
        ProcessThread::Dedicated,
        false,
    )
    .unwrap();
    let r = outcome.rendered;
    assert_eq!(r.channels, 2);
    assert_eq!(r.samples.len(), 24_000 * 2);
    assert_eq!(r.stats.non_finite, 0);
    assert!(
        r.stats.peak > 0.05 && r.stats.peak <= 0.1,
        "peak {}",
        r.stats.peak
    );
    // The note starts at 0.1 s = frame 4800 (sample-accurate even though 4800 is not a multiple of... 64): it is 75 blocks.
    assert_eq!(r.stats.first_nonzero_frame, Some(4800));
    // And stops at 0.3 s = frame 14400.
    assert_eq!(r.stats.last_nonzero_frame, Some(14_399));
}

#[test]
fn the_same_render_gives_the_same_hash_on_either_thread() {
    let (entry, id) = entry::<false>();
    let req = request("C4@0+0.3, E4@0.05+0.3, G4@0.1+0.3", 0.5);
    let a = render_once(&entry, &id, &req, ProcessThread::Dedicated, false)
        .unwrap()
        .rendered;
    let b = render_once(&entry, &id, &req, ProcessThread::Dedicated, false)
        .unwrap()
        .rendered;
    let c = render_once(&entry, &id, &req, ProcessThread::Calling, false)
        .unwrap()
        .rendered;
    assert_eq!(a.sha256, b.sha256);
    assert_eq!(a.sha256, c.sha256);
    assert_eq!(a.sha256.len(), 64);
}

#[test]
fn a_different_script_changes_the_hash() {
    let (entry, id) = entry::<false>();
    let a = render_once(
        &entry,
        &id,
        &request("C4@0+0.3", 0.5),
        ProcessThread::Dedicated,
        false,
    )
    .unwrap()
    .rendered;
    let b = render_once(
        &entry,
        &id,
        &request("D4@0+0.3", 0.5),
        ProcessThread::Dedicated,
        false,
    )
    .unwrap()
    .rendered;
    assert_ne!(a.sha256, b.sha256);
}

#[test]
fn reset_drops_the_sounding_voice() {
    let (entry, id) = entry::<false>();
    let held = render_once(
        &entry,
        &id,
        &request("A4@0+0.4", 0.5),
        ProcessThread::Dedicated,
        false,
    )
    .unwrap()
    .rendered;
    let cut = render_once(
        &entry,
        &id,
        &request("A4@0+0.4, reset@0.1", 0.5),
        ProcessThread::Dedicated,
        false,
    )
    .unwrap()
    .rendered;
    assert_eq!(held.stats.last_nonzero_frame, Some(19_199));
    // The reset is applied at the start of the block holding 0.1 s = frame 4800 = block 75 exactly.
    assert_eq!(cut.stats.last_nonzero_frame, Some(4_799));
    assert_ne!(held.sha256, cut.sha256);
}

#[test]
fn well_behaved_plugins_pass_the_instances_check() {
    let (entry, id) = entry::<false>();
    let report = instances::run(
        &entry,
        &id,
        &request("C4@0+0.3, G4@0.1+0.3", 0.5),
        3,
        &instances::MODES,
    )
    .unwrap();
    assert!(report.identical, "{}", report.to_json());
    assert_eq!(report.modes.len(), 4);
    assert!(report.modes.iter().all(|m| m.hashes.len() == 3));
}

#[test]
fn the_instances_check_catches_shared_state() {
    let (entry, id) = entry::<true>();
    let report =
        instances::run(&entry, &id, &request("C4@0+0.3", 0.5), 2, &instances::MODES).unwrap();
    assert!(
        !report.identical,
        "a plugin whose level depends on a static must be flagged"
    );
    // Every mode sees it: each instance is activated after the reference render and after each other.
    for mode in &report.modes {
        let distinct: std::collections::BTreeSet<&String> = mode.hashes.iter().collect();
        assert_eq!(
            distinct.len(),
            2,
            "{}: instances should differ from each other",
            mode.mode
        );
    }
}

#[test]
fn bad_input_is_an_error_not_a_panic() {
    let (entry, id) = entry::<false>();
    assert!(
        render_once(
            &entry,
            &id,
            &request("C4", 0.5),
            ProcessThread::Dedicated,
            false
        )
        .is_err()
    );
    let mut with_param = request("C4@0+0.3", 0.5);
    with_param.params.push(("Nope".to_owned(), 1.0));
    assert!(render_once(&entry, &id, &with_param, ProcessThread::Dedicated, false).is_err());
    let mut with_state = request("C4@0+0.3", 0.5);
    with_state.state = Some(vec![1, 2, 3]);
    assert!(render_once(&entry, &id, &with_state, ProcessThread::Dedicated, false).is_err());
}
