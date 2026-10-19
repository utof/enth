//! In-process host tests (lane PL): the real [`Enth`] driven through fake nice-plug contexts, the way nice-plug's
//! CLAP wrapper drives it, so the shell's audio block is tested without building a bundle.
//!
//! [`Host`] activates the plugin, loads state through the persistent fields, and renders a schedule of host events
//! in host blocks of any size. Like nice-plug with `SAMPLE_ACCURATE_AUTOMATION = true`, it splits a block at every
//! parameter change and sets the parameter before the slice; notes go in with timings relative to the slice; a
//! reset is called before the block it falls in. It records what the plugin sent back (`NOTE_END`s with absolute
//! frames, rescan requests, voice-capacity reports, background tasks).
//!
//! The renders are compared with `enth_render` (what `enth render` runs) for the same patch and an equivalent
//! performance, bit for bit: slot automation (the CLI automates the slot's target with `from_norm` of the host
//! value), macros and the mod wheel (`controls`), the monitor bypass (the CLI renders the patch with the
//! `time_based` effects disabled), a zero-length note, a CLAP reset, and event floods past the event list.

use std::cell::Cell;
use std::collections::{BTreeMap, VecDeque};

use enth_engine::{EventKind, NoteKey};
use enth_patch::{MacroIndex, ParamAddr, Patch, SlotIndex};
use enth_render::{Automation, Control, Controller, Note, Performance, RenderSpec};
use enth_session::HostParam;
use nice_plug::context::process::SendEventError;
use nice_plug::midi::{Channel, Key, NoteEvent, VoiceID};
use nice_plug::prelude::*;

use crate::{Enth, Task};

type Ev = NoteEvent<()>;

const SR: u32 = 48_000;
/// The plugin-level golden patch: two oscillators into a filter with an LFO, a reverb, macro 1 on osc1's level,
/// slot 1 on filter1.cutoff, slot 2 on filter1.resonance.
const GOLDEN: &str = include_str!("../../../tools/test-host/golden/enth-plugin.patch.json");

/// The fake `ProcessContext`: one slice's input events, and everything the plugin sends back.
#[derive(Default)]
struct Ctx {
    input: VecDeque<Ev>,
    /// (slice start in absolute frames, event) of every `try_send_event`.
    sent: Vec<(u64, Ev)>,
    slice_start: u64,
    transport: Option<Transport>,
    rescans: Cell<u32>,
    capacities: std::cell::RefCell<Vec<u32>>,
    /// Background tasks scheduled (`execute_background`: the pump).
    background: Cell<u32>,
}

impl ProcessContext<Enth> for Ctx {
    fn plugin_api(&self) -> PluginApi {
        PluginApi::Clap
    }

    fn execute_background(&self, task: Task) {
        assert_eq!(task, Task::Pump);
        self.background.set(self.background.get() + 1);
    }

    fn execute_gui(&self, _task: Task) {
        panic!("process() must never schedule GUI work");
    }

    fn transport(&self) -> &Transport {
        self.transport.as_ref().expect("set by the host")
    }

    fn next_event(&mut self) -> Option<Ev> {
        self.input.pop_front()
    }

    fn try_send_event(&mut self, event: Ev) -> Result<(), (Ev, SendEventError)> {
        self.sent.push((self.slice_start, event));
        Ok(())
    }

    fn set_latency_samples(&self, _samples: u32) {}

    fn request_restart(&self) {}

    fn request_param_info_rescan(&self) {
        self.rescans.set(self.rescans.get() + 1);
    }

    fn set_current_voice_capacity(&self, capacity: u32) {
        self.capacities.borrow_mut().push(capacity);
    }
}

/// The fake `ActivateContext`.
#[derive(Default)]
struct ActCtx {
    capacities: std::cell::RefCell<Vec<u32>>,
}

impl ActivateContext<Enth> for ActCtx {
    fn plugin_api(&self) -> PluginApi {
        PluginApi::Clap
    }

    fn execute(&self, _task: Task) {}

    fn set_latency_samples(&self, _samples: u32) {}

    fn set_current_voice_capacity(&self, capacity: u32) {
        self.capacities.borrow_mut().push(capacity);
    }
}

/// One host event at an absolute frame.
#[derive(Clone, Debug)]
enum HostEvent {
    Note(Ev),
    Param(HostParam, f32),
    Reset,
}

#[derive(Clone, Debug)]
struct At {
    frame: u64,
    event: HostEvent,
}

fn at(frame: u64, event: HostEvent) -> At {
    At { frame, event }
}

fn on(frame: u64, key: u8, id: i32, velocity: f32) -> At {
    at(
        frame,
        HostEvent::Note(NoteEvent::NoteOn {
            timing: 0,
            voice_id: VoiceID::ID(id),
            channel: Channel::Number(0),
            key: Key::Number(key),
            velocity,
        }),
    )
}

fn off(frame: u64, key: u8, id: i32) -> At {
    at(
        frame,
        HostEvent::Note(NoteEvent::NoteOff {
            timing: 0,
            voice_id: VoiceID::ID(id),
            channel: Channel::Number(0),
            key: Key::Number(key),
            velocity: 0.0,
        }),
    )
}

fn with_timing(e: &Ev, timing: u32) -> Ev {
    let mut e = *e;
    match &mut e {
        NoteEvent::NoteOn { timing: t, .. }
        | NoteEvent::NoteOff { timing: t, .. }
        | NoteEvent::Choke { timing: t, .. }
        | NoteEvent::PolyModulation { timing: t, .. }
        | NoteEvent::MidiCC { timing: t, .. } => *t = timing,
        other => panic!("not used here: {other:?}"),
    }
    e
}

struct Host {
    plugin: Enth,
    ctx: Ctx,
    act: ActCtx,
    /// The next absolute frame.
    frame: u64,
}

impl Host {
    /// A plugin with `patch` (and `enth`, if given) loaded as state before activation at 48 kHz.
    fn new(patch: Option<&str>, enth: Option<&str>) -> Self {
        let plugin = Enth::default();
        let mut fields = BTreeMap::new();
        if let Some(p) = patch {
            fields.insert("patch".to_owned(), p.to_owned());
        }
        if let Some(e) = enth {
            fields.insert("enth".to_owned(), e.to_owned());
        }
        if !fields.is_empty() {
            plugin.host_params().deserialize_fields(&fields);
        }
        let mut host = Host {
            plugin,
            ctx: Ctx {
                transport: Some(Transport::new(SR as f32)),
                ..Ctx::default()
            },
            act: ActCtx::default(),
            frame: 0,
        };
        host.activate();
        host
    }

    fn activate(&mut self) {
        let layout = <Enth as Plugin>::AUDIO_IO_LAYOUTS[0];
        let config = BufferConfig {
            sample_rate: SR as f32,
            min_buffer_size: None,
            max_buffer_size: 4096,
            process_mode: ProcessMode::Realtime,
        };
        assert!(self.plugin.activate(&layout, &config, &mut self.act));
        self.frame = 0;
    }

    fn set_param(&self, p: HostParam, norm: f32) {
        // SAFETY: the pointer points into the plugin's params, which outlive the call.
        unsafe {
            self.plugin
                .host_params()
                .param(p)
                .as_ptr()
                ._internal_set_normalized_value(norm);
        }
    }

    /// One slice of `left.len()` frames at absolute frame `start`, with `events` (timings relative to the slice).
    fn slice(&mut self, left: &mut [f32], right: &mut [f32], start: u64, events: Vec<Ev>) {
        let n = left.len();
        self.ctx.input = events.into();
        self.ctx.slice_start = start;
        let mut buffer = Buffer::default();
        // SAFETY: both slices live until the end of this function and have `n` frames.
        unsafe {
            buffer.set_slices(n, |s| {
                *s = vec![left, right];
            });
        }
        let mut aux = AuxiliaryBuffers {
            inputs: &mut [],
            outputs: &mut [],
        };
        let _ = self.plugin.process(&mut buffer, &mut aux, &mut self.ctx);
    }

    /// Render `frames` frames in host blocks of `block`, playing `schedule` (absolute frames from now, sorted).
    fn render(&mut self, frames: usize, block: usize, schedule: &[At]) -> (Vec<f32>, Vec<f32>) {
        let mut left = vec![0.0f32; frames];
        let mut right = vec![0.0f32; frames];
        let base = self.frame;
        let mut next = 0;
        let mut b0 = 0;
        while b0 < frames {
            let b1 = (b0 + block).min(frames);
            let mut todo: Vec<&At> = Vec::new();
            while next < schedule.len() && (schedule[next].frame as usize) < b1 {
                todo.push(&schedule[next]);
                next += 1;
            }
            if todo
                .iter()
                .any(|a| matches!(a.event, HostEvent::Reset) && a.frame as usize == b0)
            {
                self.plugin.reset();
            }
            // Split at parameter changes, as nice-plug does with sample-accurate automation.
            let mut cuts: Vec<usize> = todo
                .iter()
                .filter(|a| matches!(a.event, HostEvent::Param(..)))
                .map(|a| a.frame as usize)
                .filter(|&f| f > b0)
                .collect();
            cuts.dedup();
            let mut s0 = b0;
            for s1 in cuts.into_iter().chain([b1]) {
                for a in &todo {
                    if let HostEvent::Param(p, v) = a.event
                        && a.frame as usize == s0
                    {
                        self.set_param(p, v);
                    }
                }
                let events: Vec<Ev> = todo
                    .iter()
                    .filter(|a| (s0..s1).contains(&(a.frame as usize)))
                    .filter_map(|a| match &a.event {
                        HostEvent::Note(e) => Some(with_timing(e, (a.frame as usize - s0) as u32)),
                        _ => None,
                    })
                    .collect();
                let (l, r) = (&mut left[s0..s1], &mut right[s0..s1]);
                self.slice(l, r, base + s0 as u64, events);
                s0 = s1;
            }
            b0 = b1;
        }
        self.frame = base + frames as u64;
        (left, right)
    }

    /// The `NOTE_END`s sent so far, as (absolute frame, note id).
    fn note_ends(&self) -> Vec<(u64, i32)> {
        self.ctx
            .sent
            .iter()
            .filter_map(|(start, e)| match e {
                NoteEvent::VoiceTerminated {
                    timing, voice_id, ..
                } => Some((start + u64::from(*timing), voice_id.id().unwrap_or(-1))),
                _ => None,
            })
            .collect()
    }
}

fn golden() -> Patch {
    enth_patch::file::load(GOLDEN).unwrap().patch
}

fn cli(patch: &Patch, perf: &Performance) -> (Vec<f32>, Vec<f32>) {
    let out =
        enth_render::render(patch, perf, &RenderSpec::new(SR, enth_render::DEFAULT_SEED)).unwrap();
    (out.left, out.right)
}

fn same(a: &(Vec<f32>, Vec<f32>), b: &(Vec<f32>, Vec<f32>)) -> bool {
    let bits = |v: &[f32]| v.iter().map(|x| x.to_bits()).collect::<Vec<_>>();
    a.0.len() == b.0.len() && bits(&a.0) == bits(&b.0) && bits(&a.1) == bits(&b.1)
}

fn loud(a: &(Vec<f32>, Vec<f32>)) -> bool {
    a.0.iter().chain(&a.1).any(|x| x.abs() > 1e-3)
}

fn audible(a: &(Vec<f32>, Vec<f32>)) -> bool {
    a.0.iter().chain(&a.1).any(|x| *x != 0.0)
}

const BLOCKS: [usize; 4] = [1, 64, 100, 4096];

fn slot(n: usize) -> HostParam {
    HostParam::Slot(SlotIndex::from_one_based(n).unwrap())
}

fn macro_(n: usize) -> HostParam {
    HostParam::Macro(MacroIndex::new(n - 1).unwrap())
}

#[test]
fn slot_and_macro_automation_and_the_mod_wheel_render_like_the_cli_at_every_block_size() {
    let mut patch = golden();
    // The mod wheel moves voice pan (the golden patch has no mod-wheel route).
    let index = patch.routes.len();
    enth_patch::apply(
        &mut patch,
        enth_patch::Op::InsertRoute {
            index,
            route: enth_patch::Route {
                src: enth_patch::ModSource::ModWheel,
                dst: "voice.pan".parse().unwrap(),
                depth: 50.0,
            },
        },
    )
    .unwrap();
    let text = patch.to_json();
    let cutoff: ParamAddr = "filter1.cutoff".parse().unwrap();
    let spec = patch.spec(&cutoff).unwrap();
    let frames = 14_400; // 0.3 s
    let sr = f64::from(SR);
    let perf = Performance {
        notes: vec![Note::new(48, 1.0, 0.0, 0.25), Note::new(55, 0.5, 0.02, 0.2)],
        automation: vec![
            Automation {
                time_s: 3_000.0 / sr,
                addr: cutoff.clone(),
                value: spec.from_norm(0.2),
            },
            Automation {
                time_s: 9_100.0 / sr,
                addr: cutoff.clone(),
                value: spec.from_norm(0.85),
            },
        ],
        controls: vec![
            Control {
                time_s: 4_000.0 / sr,
                controller: Controller::Macro(MacroIndex::new(0).unwrap()),
                value: 0.75,
            },
            Control {
                time_s: 6_001.0 / sr,
                controller: Controller::ModWheel,
                value: 1.0,
            },
            Control {
                time_s: 8_000.0 / sr,
                controller: Controller::ModWheel,
                value: 64.0 / 127.0,
            },
        ],
        duration_s: f64::from(frames as u32) / sr,
    };
    let expected = cli(&patch, &perf);
    assert!(loud(&expected));
    let cc = |frame, value| {
        at(
            frame,
            HostEvent::Note(NoteEvent::MidiCC {
                timing: 0,
                channel: 3,
                cc: 1,
                value,
            }),
        )
    };
    let schedule = vec![
        on(0, 48, 0, 1.0),
        on(960, 55, 1, 0.5),
        at(3_000, HostEvent::Param(slot(1), 0.2)),
        at(4_000, HostEvent::Param(macro_(1), 0.75)),
        cc(6_001, 1.0),
        cc(8_000, 64.0 / 127.0),
        at(9_100, HostEvent::Param(slot(1), 0.85)),
        off(9_600, 55, 1),
        off(12_000, 48, 0),
    ];
    for block in BLOCKS {
        let mut host = Host::new(Some(&text), None);
        let got = host.render(frames, block, &schedule);
        assert!(same(&got, &expected), "block {block}");
    }
}

#[test]
fn the_monitor_bypass_renders_like_the_patch_without_its_time_based_effects() {
    // Bypassed from the saved state: on a fresh engine the switch is immediate, so the whole render is dry.
    let enth = r#"{"schema":"enth.plugin-state/1","monitor":{"fx_bypass":true}}"#;
    let mut dry = golden();
    for m in &mut dry.fx {
        if m.kind.spec().time_based {
            m.enabled = false;
        }
    }
    let frames = 24_000;
    let perf = Performance {
        notes: vec![Note::new(60, 0.8, 0.0, 0.3), Note::new(64, 0.8, 0.0, 0.3)],
        duration_s: 0.5,
        ..Performance::default()
    };
    let schedule = [
        on(0, 60, 0, 0.8),
        on(0, 64, 1, 0.8),
        off(14_400, 60, 0),
        off(14_400, 64, 1),
    ];
    let expected = cli(&dry, &perf);
    let wet = cli(&golden(), &perf);
    assert!(!same(&expected, &wet), "the reverb is audible");
    for block in [64, 100] {
        let mut host = Host::new(Some(GOLDEN), Some(enth));
        assert!(host.plugin.engine.as_ref().unwrap().monitor_bypass());
        let got = host.render(frames, block, &schedule);
        assert!(same(&got, &expected), "block {block}");
    }

    // Switched on mid-render through the session (the editor's switch): the engine fades the reverb out over 20 ms
    // and the setting is saved with the instance, outside the patch.
    let mut host = Host::new(Some(GOLDEN), None);
    let first = host.render(4_800, 64, &schedule[..2]);
    let ((), _) = host
        .plugin
        .shared()
        .with_session(|s| s.set_monitor_bypass(true));
    let _ = host.render(64, 64, &[]);
    let engine = host.plugin.engine.as_ref().unwrap();
    assert!(engine.monitor_bypass() && engine.is_bypass_fading());
    let _ = host.render(1_920, 64, &[]);
    assert!(
        !host.plugin.engine.as_ref().unwrap().is_bypass_fading(),
        "20 ms later"
    );
    assert!(loud(&first));
    let saved = host.plugin.host_params().serialize_fields();
    assert!(saved["enth"].contains(r#""fx_bypass":true"#));
    assert_eq!(saved["patch"], GOLDEN, "the patch does not carry it");
}

#[test]
fn a_zero_length_note_whose_off_comes_first_still_starts_and_ends() {
    // The test host orders a note-off before a note-on on one frame; for the same note id that is a zero-length
    // note, which `enth render` plays as on, then off.
    let frames = 24_000;
    let perf = Performance {
        notes: vec![Note::new(62, 0.8, 0.25, 0.25)],
        duration_s: 0.5,
        ..Performance::default()
    };
    let expected = cli(&golden(), &perf);
    // (It is silent: released before its attack starts. Hanging after its own note-off, it would sound.)
    let held = cli(
        &golden(),
        &Performance {
            notes: vec![Note::new(62, 0.8, 0.25, 0.5)],
            ..perf.clone()
        },
    );
    assert!(audible(&held) && !audible(&expected));
    for block in [1, 64, 100] {
        let mut host = Host::new(Some(GOLDEN), None);
        let got = host.render(
            frames,
            block,
            &[off(12_000, 62, 10), on(12_000, 62, 10, 0.8)],
        );
        assert!(same(&got, &expected), "block {block}");
        let ends = host.note_ends();
        assert_eq!(ends.len(), 1, "{ends:?}");
        assert_eq!(ends[0].1, 10);
    }
    // A note-off for another note on that frame (a retrigger) is not moved.
    let mut host = Host::new(Some(GOLDEN), None);
    let _ = host.render(
        24_000,
        64,
        &[
            on(0, 62, 1, 0.8),
            off(12_000, 62, 1),
            on(12_000, 62, 2, 0.8),
        ],
    );
    assert_eq!(
        host.note_ends().iter().map(|e| e.1).collect::<Vec<_>>(),
        [1]
    );
}

#[test]
fn an_event_flood_past_the_event_list_is_rendered_in_chunks_and_nothing_is_dropped() {
    // 3,000 events in one 64-frame block (the list holds 1,096): 1,500 notes of 32 frames, all inside the block.
    let mut flood = Vec::new();
    for i in 0..1_500u64 {
        let frame = 1_024 + i % 32;
        let key = (24 + (i % 80)) as u8;
        let id = i as i32;
        flood.push(on(frame, key, id, 0.5));
        flood.push(off(frame + 32, key, id));
    }
    flood.sort_by_key(|a| a.frame);
    let frames = 48_000;
    let mut burst = Host::new(Some(GOLDEN), None);
    let got = burst.render(frames, 64, &flood);
    assert!(burst.plugin.audio.chunks() >= 2, "the list filled up");
    // The same events, one host block per frame: no list can fill up.
    let mut spread = Host::new(Some(GOLDEN), None);
    let reference = spread.render(frames, 1, &flood);
    assert_eq!(spread.plugin.audio.chunks(), 0);
    assert!(same(&got, &reference));
    assert!(audible(&got));
    // Every note ended, exactly once.
    let mut ended: Vec<i32> = burst.note_ends().iter().map(|e| e.1).collect();
    ended.sort_unstable();
    assert_eq!(ended, (0..1_500).collect::<Vec<_>>());
    let engine = burst.plugin.engine.as_ref().unwrap();
    assert_eq!(engine.stats().dropped_events, 0);
}

#[test]
fn a_reset_ends_every_voice_at_the_next_block_and_then_plays_like_a_fresh_engine() {
    let mut host = Host::new(Some(GOLDEN), None);
    let schedule = [
        on(0, 60, 0, 0.8),
        on(0, 67, 1, 0.8),
        at(4_800, HostEvent::Reset),
        on(4_800, 64, 2, 0.8),
        off(14_400, 64, 2),
    ];
    let (l, r) = host.render(24_000, 64, &schedule);
    let mut ends = host.note_ends();
    ends.sort_unstable();
    assert_eq!(ends[..2], [(4_800, 0), (4_800, 1)], "{ends:?}");
    assert_eq!(ends.len(), 3);
    // After the reset the engine counts from 0 again: the rest equals a fresh render of the later note.
    let perf = Performance {
        notes: vec![Note::new(64, 0.8, 0.0, 0.2)],
        duration_s: 0.4,
        ..Performance::default()
    };
    let expected = cli(&golden(), &perf);
    assert!(same(&(l[4_800..].to_vec(), r[4_800..].to_vec()), &expected));
}

#[test]
fn a_reactivation_starts_from_the_patch_with_no_stale_voice_ends_and_no_echo() {
    let mut host = Host::new(Some(GOLDEN), None);
    let _ = host.render(
        4_800,
        64,
        &[on(0, 60, 0, 0.8), at(100, HostEvent::Param(slot(1), 0.9))],
    );
    host.plugin.reset();
    host.plugin.deactivate();
    host.ctx.sent.clear();
    host.activate();
    let perf = Performance {
        notes: vec![Note::new(64, 0.8, 0.0, 0.2)],
        duration_s: 0.3,
        ..Performance::default()
    };
    // The host's slot 1 value is still 0.9, and it is not sent again (no echo): the patch's own cutoff plays. A
    // fold at activate wrote it into the patch, so the CLI comparison uses the patch the session now holds.
    let patch = host.plugin.shared().session.lock().patch().clone();
    let got = host.render(14_400, 64, &[on(0, 64, 0, 0.8), off(9_600, 64, 0)]);
    assert!(same(&got, &cli(&patch, &perf)));
    assert!(
        host.note_ends().iter().all(|e| e.0 > 0),
        "the reset's ends were for the old engine: {:?}",
        host.note_ends()
    );
}

#[test]
fn values_restored_with_a_load_are_adopted_not_sent() {
    let mut host = Host::new(None, None);
    let _ = host.render(640, 64, &[]);
    // A host load while processing: parameters first, then the fields.
    host.set_param(slot(1), 0.9);
    let mut fields = BTreeMap::new();
    fields.insert("patch".to_owned(), GOLDEN.to_owned());
    host.plugin.host_params().deserialize_fields(&fields);
    let _ = host.render(64, 64, &[]);
    let engine = host.plugin.engine.as_ref().unwrap();
    assert_eq!(
        engine.host_slot(0),
        None,
        "the loaded patch wins in the engine"
    );
    // A later host change is sent.
    let _ = host.render(64, 64, &[at(0, HostEvent::Param(slot(1), 0.3))]);
    let engine = host.plugin.engine.as_ref().unwrap();
    assert_eq!(engine.host_slot(0), Some(0.3));
}

#[test]
fn poly_modulation_reaches_only_the_voice_with_the_hosts_voice_id_and_its_end_carries_that_id() {
    let mut host = Host::new(Some(GOLDEN), None);
    let poly = |frame, voice_id, offset| {
        at(
            frame,
            HostEvent::Note(NoteEvent::PolyModulation {
                timing: 0,
                voice_id,
                poly_modulation_id: 0,
                normalized_offset: offset,
            }),
        )
    };
    let _ = host.render(
        3_200,
        64,
        &[
            on(0, 60, 7, 0.8),
            on(0, 64, 9, 0.8),
            poly(640, 9, 0.5),
            poly(640, 1234, 1.0),
        ],
    );
    let engine = host.plugin.engine.as_ref().unwrap();
    let voice_of = |id: i32| {
        engine
            .active_voices()
            .iter()
            .copied()
            .find(|&v| engine.voices().key(v).note_id == id)
            .unwrap()
    };
    assert_eq!(engine.macro_offset(0, voice_of(9)), Some(0.5));
    assert_eq!(engine.macro_offset(0, voice_of(7)), Some(0.0));
    let _ = host.render(24_000, 64, &[off(0, 60, 7), off(0, 64, 9)]);
    let mut ids: Vec<i32> = host.note_ends().iter().map(|e| e.1).collect();
    ids.sort_unstable();
    assert_eq!(ids, [7, 9]);
}

#[test]
fn a_load_renames_the_slots_and_asks_for_one_rescan_from_the_audio_thread() {
    let mut host = Host::new(None, None);
    let name = |host: &Host, p: HostParam| host.plugin.host_params().param(p).display_name();
    assert_eq!(name(&host, slot(1)), "Slot 1");
    let _ = host.render(640, 64, &[]);
    assert_eq!(host.ctx.rescans.get(), 0, "nothing changed");

    let mut patch = golden();
    enth_patch::apply(
        &mut patch,
        enth_patch::Op::SetMacroName {
            index: MacroIndex::new(1).unwrap(),
            name: Some("Bright".to_owned()),
        },
    )
    .unwrap();
    let mut fields = BTreeMap::new();
    fields.insert("patch".to_owned(), patch.to_json());
    host.plugin.host_params().deserialize_fields(&fields);
    // Set on the loading thread, before any block.
    assert_eq!(name(&host, slot(1)), "Filter 1 · Cutoff");
    assert_eq!(name(&host, slot(2)), "Filter 1 · Resonance");
    assert_eq!(name(&host, slot(3)), "Slot 3");
    assert_eq!(name(&host, macro_(2)), "Bright");
    assert_eq!(name(&host, macro_(1)), "Macro 1");
    let _ = host.render(4_800, 64, &[]);
    assert_eq!(host.ctx.rescans.get(), 1, "one rescan per change");

    // Back to the init patch: the names go back to the ones fixed at construction, with one more rescan.
    fields.insert("patch".to_owned(), Patch::init().to_json());
    host.plugin.host_params().deserialize_fields(&fields);
    assert_eq!(name(&host, slot(1)), "Slot 1");
    assert_eq!(name(&host, macro_(2)), "Macro 2");
    let _ = host.render(4_800, 64, &[]);
    assert_eq!(host.ctx.rescans.get(), 2);
}

#[test]
fn voice_info_reports_the_plans_voice_count_at_activation_and_after_a_swap() {
    let host = Host::new(Some(GOLDEN), None);
    assert_eq!(
        *host.act.capacities.borrow(),
        [8],
        "the golden patch's polyphony"
    );
    let mut mono = golden();
    enth_patch::apply(
        &mut mono,
        enth_patch::Op::SetParam {
            addr: "voice.mode".parse().unwrap(),
            value: 1.0,
        },
    )
    .unwrap();
    let mut host = host;
    let mut fields = BTreeMap::new();
    fields.insert("patch".to_owned(), mono.to_json());
    host.plugin.host_params().deserialize_fields(&fields);
    let _ = host.render(640, 64, &[]);
    assert_eq!(*host.ctx.capacities.borrow(), [1], "mono");
    let _ = host.render(640, 64, &[]);
    assert_eq!(host.ctx.capacities.borrow().len(), 1, "reported once");
    let _ = NoteKey::new(0, 0);
    let _ = EventKind::ModWheel { value: 0.0 };
}
