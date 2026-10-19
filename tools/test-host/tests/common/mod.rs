//! An in-process CLAP plugin ("Stub") for testing the host's checks.
//!
//! It stands in for `enth-plugin`: stereo output, a note input port, two parameters (`Gain`, which scales the
//! sound, and `Inert`, which does nothing), the state extension in nice-plug's format (a length-prefixed JSON
//! document whose `fields.patch` is a string) and `NOTE_END` output events. What it does wrong is chosen by the
//! **patch document** it was given through the state extension, so a test starts a faulty plugin the way a user
//! loads a patch: `{"note_end":"missing"}`, `{"cap":8,"policy":"drop"}`, `{"param_mode":"block"}` and so on.
//! A patch with no fault key is a well-behaved plugin.
//!
//! Faults (all keys optional):
//!
//! - `mode`: `"synth"` (default) plays the held notes; `"ramp"` outputs `0.5 * ((frame % 1000) / 1000)` on both
//!   channels where `frame` counts from activation (independent of block size and of the notes: a signal the test
//!   can compute itself); `"ramp-per-block"` restarts that ramp every block (block-size dependent).
//! - `cap` and `policy`: the plugin keeps at most `cap` events of a block. Past it: `"drop"` ignores the rest,
//!   `"wedge"` ignores every later note event of the whole run, `"nan"` writes NaN into the block.
//! - `note_end`: `"missing"` (every third note never ends), `"dup"`, `"wrong_key"`, `"outside"` (offset past the
//!   block), `"early"` (offset 0 whatever the note's time).
//! - `param_mode`: `"exact"` (default, sample accurate), `"block"` (applied at the next block start) or
//!   `"delay:N"` (N frames late). `smooth_ms`: a one-pole smoother on `Gain`.
//! - `nondeterministic`: the level depends on how many instances were activated before (shared state).
//! - `leak`: `"mem:KB"`, `"fd"` or `"thread"` at every activation. `busy_us` with `busy_every`: spin that long
//!   inside `process()` (every N-th block).
#![allow(dead_code, reason = "each test binary uses a part of the stub")]

use std::io::{Read, Write};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use clack_extensions::audio_ports::{
    AudioPortFlags, AudioPortInfo, AudioPortInfoWriter, AudioPortType, PluginAudioPorts,
    PluginAudioPortsImpl,
};
use clack_extensions::note_ports::{
    NoteDialect, NoteDialects, NotePortInfo, NotePortInfoWriter, PluginNotePorts,
    PluginNotePortsImpl,
};
use clack_extensions::params::{
    ParamDisplayWriter, ParamInfo, ParamInfoFlags, ParamInfoWriter, PluginAudioProcessorParams,
    PluginMainThreadParams, PluginParams,
};
use clack_extensions::state::{PluginState, PluginStateImpl};
use clack_host::prelude::PluginEntry;
use clack_plugin::events::event_types::NoteEndEvent;
use clack_plugin::events::spaces::CoreEventSpace;
use clack_plugin::prelude::*;
use clack_plugin::utils::Cookie;
use serde_json::{Value, json};

pub const GAIN: u32 = 1;
pub const INERT: u32 = 2;

/// Instances activated so far, process-wide (the shared state `nondeterministic` leans on).
static ACTIVATIONS: AtomicU32 = AtomicU32::new(0);

pub struct Stub;

pub struct StubMain {
    /// The persisted `patch` field.
    patch: Mutex<String>,
    values: Mutex<[f64; 2]>,
}

const DEFAULT_PATCH: &str = "{}";

impl PluginMainThread<'_, ()> for StubMain {}

impl PluginAudioPortsImpl for StubMain {
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

impl PluginNotePortsImpl for StubMain {
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

impl PluginMainThreadParams for StubMain {
    fn count(&self) -> u32 {
        2
    }

    fn get_info(&self, index: u32, info: &mut ParamInfoWriter<'_>) {
        let (id, name) = match index {
            0 => (GAIN, &b"Gain"[..]),
            1 => (INERT, &b"Inert"[..]),
            _ => return,
        };
        info.set(&ParamInfo {
            id: ClapId::new(id),
            flags: ParamInfoFlags::IS_AUTOMATABLE,
            cookie: Cookie::empty(),
            name,
            module: b"",
            min_value: 0.0,
            max_value: 1.0,
            default_value: 0.5,
        });
    }

    fn get_value(&self, id: ClapId) -> Option<f64> {
        let values = self.values.lock().ok()?;
        match id.get() {
            GAIN => Some(values[0]),
            INERT => Some(values[1]),
            _ => None,
        }
    }

    fn value_to_text(
        &self,
        _id: ClapId,
        value: f64,
        writer: &mut ParamDisplayWriter<'_>,
    ) -> std::fmt::Result {
        use std::fmt::Write as _;
        write!(writer, "{value:.3}")
    }

    fn text_to_value(&self, _id: ClapId, text: &std::ffi::CStr) -> Option<f64> {
        text.to_str().ok()?.parse().ok()
    }

    fn flush(&self, _input: &InputEvents<'_>, _output: &mut OutputEvents<'_>) {}
}

impl PluginStateImpl for StubMain {
    fn save(&self, output: &mut clack_plugin::stream::OutputStream<'_>) -> Result<(), PluginError> {
        let patch = self
            .patch
            .lock()
            .map_err(|_| PluginError::Message("poisoned"))?;
        let values = self
            .values
            .lock()
            .map_err(|_| PluginError::Message("poisoned"))?;
        let doc = json!({
            "version": "stub-1",
            "params": { "gain": { "f32": values[0] }, "inert": { "f32": values[1] } },
            "fields": { "patch": *patch },
        })
        .to_string();
        output
            .write_all(&(doc.len() as u64).to_le_bytes())
            .and_then(|()| output.write_all(doc.as_bytes()))
            .map_err(|_| PluginError::Message("write failed"))
    }

    fn load(&self, input: &mut clack_plugin::stream::InputStream<'_>) -> Result<(), PluginError> {
        let mut bytes = Vec::new();
        input
            .read_to_end(&mut bytes)
            .map_err(|_| PluginError::Message("read failed"))?;
        let (prefix, body) = bytes
            .split_first_chunk::<8>()
            .ok_or(PluginError::Message("short state"))?;
        if u64::from_le_bytes(*prefix) as usize != body.len() {
            return Err(PluginError::Message("bad length prefix"));
        }
        let doc: Value =
            serde_json::from_slice(body).map_err(|_| PluginError::Message("bad state JSON"))?;
        if let Some(patch) = doc["fields"]["patch"].as_str() {
            let mut guard = self
                .patch
                .lock()
                .map_err(|_| PluginError::Message("poisoned"))?;
            patch.clone_into(&mut guard);
        }
        let mut values = self
            .values
            .lock()
            .map_err(|_| PluginError::Message("poisoned"))?;
        if let Some(v) = doc["params"]["gain"]["f32"].as_f64() {
            values[0] = v;
        }
        if let Some(v) = doc["params"]["inert"]["f32"].as_f64() {
            values[1] = v;
        }
        Ok(())
    }
}

impl PluginAudioProcessorParams for StubProcessor {
    fn flush(&mut self, input: &InputEvents<'_>, _output: &mut OutputEvents<'_>) {
        for event in input.iter() {
            if let Some(CoreEventSpace::ParamValue(p)) = event.as_core_event()
                && p.param_id().map(ClapId::get) == Some(GAIN)
            {
                self.gain = (p.value() as f32, p.value() as f32);
            }
        }
    }
}

impl Plugin for Stub {
    type AudioProcessor<'a> = StubProcessor;
    type Shared<'a> = ();
    type MainThread<'a> = StubMain;

    fn declare_extensions(builder: &mut PluginExtensions<'_, Self>, _shared: Option<&()>) {
        builder
            .register::<PluginAudioPorts>()
            .register::<PluginNotePorts>()
            .register::<PluginParams>()
            .register::<PluginState>();
    }
}

impl DefaultPluginFactory for Stub {
    fn get_descriptor() -> PluginDescriptor {
        use clack_plugin::plugin::features::{INSTRUMENT, STEREO, SYNTHESIZER};
        PluginDescriptor::new("test.stub", "Test stub").with_features([
            INSTRUMENT,
            SYNTHESIZER,
            STEREO,
        ])
    }

    fn new_shared(_host: HostSharedHandle<'_>) -> Result<(), PluginError> {
        Ok(())
    }

    fn new_main_thread<'a>(
        _host: HostMainThreadHandle<'a>,
        _shared: &'a (),
    ) -> Result<StubMain, PluginError> {
        Ok(StubMain {
            patch: Mutex::new(DEFAULT_PATCH.to_owned()),
            values: Mutex::new([0.5, 0.5]),
        })
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Synth,
    Ramp,
    RampPerBlock,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Policy {
    Drop,
    Wedge,
    Nan,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum NoteEndFault {
    None,
    Missing,
    Dup,
    WrongKey,
    Outside,
    Early,
}

struct Voice {
    key: u16,
    note_id: u32,
    phase: f32,
    increment: f32,
}

struct Pending {
    at: u64,
    id: u32,
    value: f64,
}

pub struct StubProcessor {
    sample_rate: f32,
    mode: Mode,
    cap: Option<usize>,
    policy: Policy,
    note_end: NoteEndFault,
    param_delay: Option<u64>,
    smooth_coeff: Option<f32>,
    busy: Option<(u64, u64)>,
    voices: Vec<Voice>,
    pending: Vec<Pending>,
    /// Target and smoothed gain.
    gain: (f32, f32),
    scale: f32,
    pos: u64,
    blocks: u64,
    ended: u32,
    wedged: bool,
}

impl<'a> PluginAudioProcessor<'a, (), StubMain> for StubProcessor {
    fn activate(
        _host: HostAudioProcessorHandle<'a>,
        main_thread: &StubMain,
        _shared: &'a (),
        config: PluginAudioConfiguration,
    ) -> Result<Self, PluginError> {
        let n = ACTIVATIONS.fetch_add(1, Ordering::SeqCst);
        let patch: Value = main_thread
            .patch
            .lock()
            .ok()
            .and_then(|p| serde_json::from_str(&p).ok())
            .unwrap_or(Value::Null);
        let values = *main_thread
            .values
            .lock()
            .map_err(|_| PluginError::Message("poisoned"))?;
        if let Some(leak) = patch["leak"].as_str() {
            match leak.split_once(':') {
                Some(("mem", kb)) => {
                    let kb: usize = kb.parse().unwrap_or(16);
                    Box::leak(vec![0xA5u8; kb * 1024].into_boxed_slice());
                }
                None if leak == "fd" => {
                    if let Ok(file) = std::fs::File::open("/dev/null") {
                        let _leaked = std::os::fd::IntoRawFd::into_raw_fd(file);
                    }
                }
                None if leak == "thread" => {
                    std::thread::spawn(|| {
                        loop {
                            std::thread::park();
                        }
                    });
                }
                _ => {}
            }
        }
        let sample_rate = config.sample_rate as f32;
        Ok(Self {
            sample_rate,
            mode: match patch["mode"].as_str() {
                Some("ramp") => Mode::Ramp,
                Some("ramp-per-block") => Mode::RampPerBlock,
                _ => Mode::Synth,
            },
            cap: patch["cap"].as_u64().map(|c| c as usize),
            policy: match patch["policy"].as_str() {
                Some("wedge") => Policy::Wedge,
                Some("nan") => Policy::Nan,
                _ => Policy::Drop,
            },
            note_end: match patch["note_end"].as_str() {
                Some("missing") => NoteEndFault::Missing,
                Some("dup") => NoteEndFault::Dup,
                Some("wrong_key") => NoteEndFault::WrongKey,
                Some("outside") => NoteEndFault::Outside,
                Some("early") => NoteEndFault::Early,
                _ => NoteEndFault::None,
            },
            param_delay: match patch["param_mode"].as_str() {
                Some("block") => None,
                Some(other) => other
                    .strip_prefix("delay:")
                    .and_then(|n| n.parse().ok())
                    .or(Some(0)),
                None => Some(0),
            },
            smooth_coeff: patch["smooth_ms"]
                .as_f64()
                .filter(|ms| *ms > 0.0)
                .map(|ms| 1.0 - (-1.0 / (ms as f32 * sample_rate / 1000.0)).exp()),
            busy: patch["busy_us"]
                .as_u64()
                .map(|us| (us, patch["busy_every"].as_u64().unwrap_or(1).max(1))),
            voices: Vec::with_capacity(16),
            pending: Vec::new(),
            gain: (values[0] as f32, values[0] as f32),
            scale: if patch["nondeterministic"] == true {
                0.2 + n as f32 * 0.01
            } else {
                0.2
            },
            pos: 0,
            blocks: 0,
            ended: 0,
            wedged: false,
        })
    }

    fn process(
        &mut self,
        _process: Process<'_>,
        mut audio: Audio<'_>,
        events: Events<'_>,
    ) -> Result<ProcessStatus, PluginError> {
        let Some(mut port) = audio.port_pair(0) else {
            return Ok(ProcessStatus::Continue);
        };
        let channels = port
            .channels()?
            .into_f32()
            .ok_or(PluginError::Message("f32 only"))?;
        let frames = channels.frames_count() as usize;
        let block_start = self.pos;
        let mut mono = vec![0.0f32; frames];
        let started = Instant::now();
        if let Some((us, every)) = self.busy
            && self.blocks.is_multiple_of(every)
        {
            while started.elapsed() < Duration::from_micros(us) {
                std::hint::spin_loop();
            }
        }

        // Events, in time order.
        let mut seen = 0usize;
        let mut nan_block = false;
        let mut todo: Vec<(usize, Ev)> = Vec::new();
        for event in events.input.iter() {
            seen += 1;
            if let Some(cap) = self.cap
                && seen > cap
            {
                match self.policy {
                    Policy::Drop => continue,
                    Policy::Wedge => {
                        self.wedged = true;
                        continue;
                    }
                    Policy::Nan => {
                        nan_block = true;
                        continue;
                    }
                }
            }
            let time = event.header().time() as usize;
            match event.as_core_event() {
                Some(CoreEventSpace::NoteOn(on)) => todo.push((
                    time,
                    Ev::On {
                        key: on.key().into_specific().unwrap_or(60),
                        id: on.note_id().into_specific().unwrap_or(0),
                    },
                )),
                Some(CoreEventSpace::NoteOff(off)) => todo.push((
                    time,
                    Ev::Off {
                        id: off.note_id().into_specific().unwrap_or(0),
                    },
                )),
                Some(CoreEventSpace::ParamValue(p)) => {
                    let at = match self.param_delay {
                        Some(delay) => block_start + time as u64 + delay,
                        None => block_start + frames as u64,
                    };
                    self.pending.push(Pending {
                        at,
                        id: p.param_id().map_or(0, ClapId::get),
                        value: p.value(),
                    });
                }
                _ => {}
            }
        }
        let mut next = 0usize;
        for (frame, out) in mono.iter_mut().enumerate() {
            while next < todo.len() && todo[next].0 <= frame {
                if !self.wedged {
                    self.apply(&todo[next].1, frame as u32, &mut *events.output);
                }
                next += 1;
            }
            let abs = block_start + frame as u64;
            let pending = std::mem::take(&mut self.pending);
            for p in pending {
                if p.at <= abs {
                    if p.id == GAIN {
                        self.gain.0 = p.value as f32;
                        if self.smooth_coeff.is_none() {
                            self.gain.1 = self.gain.0;
                        }
                    }
                } else {
                    self.pending.push(p);
                }
            }
            if let Some(coeff) = self.smooth_coeff {
                self.gain.1 += (self.gain.0 - self.gain.1) * coeff;
            }
            *out = match self.mode {
                Mode::Synth => {
                    let mut sum = 0.0;
                    for voice in &mut self.voices {
                        voice.phase = (voice.phase + voice.increment).fract();
                        sum += voice.phase * 2.0 - 1.0;
                    }
                    sum * self.gain.1 * self.scale
                }
                Mode::Ramp => 0.5 * ((abs % 1000) as f32 / 1000.0),
                Mode::RampPerBlock => 0.5 * ((frame as u64 % 1000) as f32 / 1000.0),
            };
        }
        if nan_block {
            mono[0] = f32::NAN;
        }
        for channel in channels {
            if let ChannelPair::OutputOnly(buf) = channel {
                buf.copy_from_slice(&mono);
            }
        }
        self.pos += frames as u64;
        self.blocks += 1;
        Ok(ProcessStatus::Continue)
    }

    fn reset(&mut self) {
        self.voices.clear();
    }
}

enum Ev {
    On { key: u16, id: u32 },
    Off { id: u32 },
}

impl StubProcessor {
    fn apply(&mut self, event: &Ev, time: u32, out: &mut OutputEvents<'_>) {
        match *event {
            Ev::On { key, id } => {
                if self.voices.len() >= 16 {
                    let stolen = self.voices.remove(0);
                    self.send_end(out, time, stolen.key, stolen.note_id);
                }
                self.voices.push(Voice {
                    key,
                    note_id: id,
                    phase: 0.0,
                    increment: 440.0 * ((f32::from(key) - 69.0) / 12.0).exp2() / self.sample_rate,
                });
            }
            Ev::Off { id } => {
                if let Some(at) = self.voices.iter().position(|v| v.note_id == id) {
                    let voice = self.voices.remove(at);
                    self.send_end(out, time, voice.key, voice.note_id);
                }
            }
        }
    }

    fn send_end(&mut self, out: &mut OutputEvents<'_>, time: u32, key: u16, id: u32) {
        self.ended += 1;
        let (mut time, mut key) = (time, key);
        match self.note_end {
            NoteEndFault::Missing if self.ended.is_multiple_of(3) => return,
            NoteEndFault::WrongKey => key += 1,
            NoteEndFault::Outside => time = u32::MAX / 2,
            NoteEndFault::Early => time = 0,
            _ => {}
        }
        let end = NoteEndEvent::new(time, Pckn::new(0u16, 0u16, key, id));
        let _ = out.try_push(end);
        if self.note_end == NoteEndFault::Dup {
            let _ = out.try_push(end);
        }
    }
}

/// The plugin as a loadable entry, and its id.
pub fn entry() -> (PluginEntry, std::ffi::CString) {
    let entry = PluginEntry::load_from_clack::<SinglePluginEntry<Stub>>(c"/test/stub.clap")
        .expect("the in-process entry loads");
    let id = enth_test_host::host::resolve_plugin_id(&entry, None).expect("one plugin");
    (entry, id)
}

/// A state blob that gives the stub `patch` as its persisted patch document.
pub fn state_for(patch: &str) -> Vec<u8> {
    enth_test_host::state::wrap_patch(patch, "patch", "stub-1").expect("a valid patch")
}
