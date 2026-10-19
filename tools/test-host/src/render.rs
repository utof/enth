//! Offline rendering: a plugin instance, a note script and a fixed block size in, samples out.
//!
//! The render is deterministic by construction: blocks are all the same size, events are placed at exact
//! sample positions, there are no clocks, and one thread drives `process()` for the whole render. The
//! hash is SHA-256 over the interleaved little-endian `f32` samples of every output channel, in port
//! order, for `ceil(dur * sr / block) * block` frames.

use std::ffi::CStr;
use std::time::Instant;

use clack_extensions::state::PluginState;
use clack_host::events::event_types::{MidiEvent, NoteOffEvent, NoteOnEvent, ParamValueEvent};
use clack_host::events::spaces::CoreEventSpace;
use clack_host::prelude::*;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::host::{Host, instantiate};
use crate::notes::{self, Kind, Schedule};
use crate::query::{self, ParamDesc};

/// Fixed render settings.
#[derive(Clone, Copy, Debug)]
pub struct RenderConfig {
    pub sample_rate: f64,
    pub block: u32,
    /// Total frames: a whole number of blocks.
    pub frames: u64,
}

impl RenderConfig {
    pub fn new(sample_rate: f64, block: u32, duration_s: f64) -> Result<Self, String> {
        if !(sample_rate.is_finite() && sample_rate >= 1.0) {
            return Err(format!("--sr {sample_rate} must be a positive number"));
        }
        if block == 0 {
            return Err("--block must be at least 1".to_owned());
        }
        if !(duration_s.is_finite() && duration_s > 0.0) {
            return Err(format!(
                "--dur {duration_s} must be a positive number of seconds"
            ));
        }
        let wanted = (duration_s * sample_rate).ceil() as u64;
        let blocks = wanted.div_ceil(u64::from(block));
        Ok(Self {
            sample_rate,
            block,
            frames: blocks * u64::from(block),
        })
    }

    pub fn blocks(&self) -> u64 {
        self.frames / u64::from(self.block)
    }
}

/// Everything `render` needs besides the plugin.
#[derive(Clone, Debug)]
pub struct RenderRequest {
    pub script: String,
    /// `--param name=value`, applied at frame 0.
    pub params: Vec<(String, f64)>,
    pub state: Option<Vec<u8>>,
    pub config: RenderConfig,
}

/// Channels per audio port, in port order.
#[derive(Clone, Debug, Default)]
pub struct PortLayout {
    pub inputs: Vec<usize>,
    pub outputs: Vec<usize>,
}

impl PortLayout {
    pub fn output_channels(&self) -> usize {
        self.outputs.iter().sum()
    }
}

/// A plugin instance prepared for rendering: instantiated, state loaded, script parsed, not yet active.
pub struct Session {
    pub instance: PluginInstance<Host>,
    pub layout: PortLayout,
    pub schedule: Schedule,
    pub config: RenderConfig,
    pub params: Vec<ParamDesc>,
}

impl Session {
    /// Instantiates the plugin, loads `state` through the state extension (if given) and reads its parameters
    /// and audio ports. The schedule is empty: callers set `schedule` themselves.
    pub fn open_bare(
        entry: &PluginEntry,
        id: &CStr,
        state: Option<&[u8]>,
        config: RenderConfig,
    ) -> Result<Self, String> {
        let mut instance = instantiate(entry, id)?;
        // The names before the state: a plugin may rename parameters from its state (enth-plugin's slots), and
        // scripts keep working with the names a fresh instance shows.
        let before = state.map(|_| query::params(&mut instance));
        if let Some(state) = state {
            let handle = instance.plugin_handle();
            let ext = handle
                .get_extension::<PluginState>()
                .ok_or("--state given, but the plugin has no state extension")?;
            ext.load(&handle, &mut &*state)
                .map_err(|e| format!("plugin refused the state blob: {e}"))?;
        }
        let mut params = query::params(&mut instance);
        if let Some(before) = &before {
            query::note_initial_names(&mut params, before);
        }
        let (inputs, outputs) = query::audio_ports(&mut instance);
        let layout = PortLayout {
            inputs: inputs.iter().map(|p| p.channels as usize).collect(),
            outputs: outputs.iter().map(|p| p.channels as usize).collect(),
        };
        if layout.output_channels() == 0 {
            return Err("the plugin has no audio output port".to_owned());
        }
        Ok(Self {
            instance,
            layout,
            schedule: Schedule::default(),
            config,
            params,
        })
    }

    pub fn open(entry: &PluginEntry, id: &CStr, request: &RenderRequest) -> Result<Self, String> {
        let mut this = Self::open_bare(entry, id, request.state.as_deref(), request.config)?;
        let resolve = |name: &str| query::resolve_param(&this.params, name);
        let mut schedule = notes::parse(
            &request.script,
            request.config.sample_rate,
            request.config.frames,
            &resolve,
        )?;
        let mut at_start = Vec::new();
        for (name, value) in &request.params {
            let id =
                resolve(name).ok_or_else(|| format!("--param: no parameter named {name:?}"))?;
            at_start.push(notes::Scheduled {
                frame: 0,
                kind: Kind::Param { id, value: *value },
            });
        }
        // Command-line parameters run before anything the script does at frame 0.
        at_start.append(&mut schedule.events);
        schedule.events = at_start;
        schedule.events.sort_by_key(|e| (e.frame, e.kind.order()));
        this.schedule = schedule;
        Ok(this)
    }

    /// Activates the plugin for the fixed block size.
    pub fn activate(&mut self) -> Result<StoppedPluginAudioProcessor<Host>, String> {
        let configuration = PluginAudioConfiguration {
            sample_rate: self.config.sample_rate,
            min_frames_count: self.config.block,
            max_frames_count: self.config.block,
        };
        self.instance
            .activate(|_, _| (), configuration)
            .map_err(|e| format!("activation failed: {e}"))
    }

    pub fn setup(&self, processor: StoppedPluginAudioProcessor<Host>) -> JobSetup {
        JobSetup {
            processor,
            layout: self.layout.clone(),
            schedule: self.schedule.clone(),
            config: self.config,
            keep_samples: true,
            capture_events: false,
            record_timing: false,
            inject_event_cap: None,
            inject_busy: None,
        }
    }

    pub fn save_state(&mut self) -> Result<Vec<u8>, String> {
        let handle = self.instance.plugin_handle();
        let ext = handle
            .get_extension::<PluginState>()
            .ok_or("the plugin has no state extension")?;
        let mut bytes = Vec::new();
        ext.save(&handle, &mut bytes)
            .map_err(|e| format!("plugin could not save its state: {e}"))?;
        Ok(bytes)
    }

    pub fn close(mut self, processor: StoppedPluginAudioProcessor<Host>) {
        self.instance.deactivate(processor);
    }
}

/// The `Send` half of a render job. Audio buffers hold raw pointers and are not `Send`, so they are
/// built by [`JobSetup::into_runner`] on the thread that drives `process()`.
pub struct JobSetup {
    processor: StoppedPluginAudioProcessor<Host>,
    layout: PortLayout,
    schedule: Schedule,
    config: RenderConfig,
    /// Keep the rendered samples (off for the endless audio loop of `editor-cycles`).
    pub keep_samples: bool,
    /// Record the events the plugin sends back (`NOTE_END` and others) with their block and offset.
    pub capture_events: bool,
    /// Record the wall time of every plugin `process()` call.
    pub record_timing: bool,
    /// Negative-control fault: deliver only the first N events of each block, as a plugin with a smaller
    /// input queue would see them. `None` delivers everything.
    pub inject_event_cap: Option<usize>,
    /// Negative-control fault: spin for this many microseconds inside the timed `process()` window of every
    /// `every`-th block, as a plugin that is that much slower (or spikes that often) would.
    pub inject_busy: Option<BusyFault>,
}

/// A deliberate slowdown of `process()` for the timing checks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BusyFault {
    pub us: u64,
    /// Every block (1) or every N-th.
    pub every: u64,
}

impl BusyFault {
    /// `busy-us=N` with an optional `,every=K`.
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut us = None;
        let mut every = 1;
        for part in text.split(',').map(str::trim).filter(|p| !p.is_empty()) {
            let (name, value) = part
                .split_once('=')
                .ok_or_else(|| format!("--inject {part:?}: expected NAME=NUMBER"))?;
            let n: u64 = value
                .parse()
                .map_err(|_| format!("--inject {part:?}: {value:?} is not a number"))?;
            match name {
                "busy-us" => us = Some(n),
                "every" => every = n.max(1),
                other => return Err(format!("--inject {other:?}: use busy-us=N[,every=K]")),
            }
        }
        Ok(Self {
            us: us.ok_or("--inject needs busy-us=N")?,
            every,
        })
    }
}

impl JobSetup {
    pub fn into_runner(self) -> JobRunner {
        let block = self.config.block as usize;
        let in_channels: usize = self.layout.inputs.iter().sum();
        let out_channels = self.layout.output_channels();
        let silent_inputs = self
            .layout
            .inputs
            .iter()
            .map(|&c| vec![vec![0.0f32; block]; c])
            .collect();
        let outputs = self
            .layout
            .outputs
            .iter()
            .map(|&c| vec![vec![0.0f32; block]; c])
            .collect();
        let samples = if self.keep_samples {
            Vec::with_capacity(self.config.frames as usize * out_channels)
        } else {
            Vec::new()
        };
        JobRunner {
            processor: self.processor.into(),
            in_ports: AudioPorts::with_capacity(in_channels, self.layout.inputs.len()),
            out_ports: AudioPorts::with_capacity(out_channels, self.layout.outputs.len()),
            silent_inputs,
            outputs,
            events_in: EventBuffer::with_capacity(64),
            events_out: EventBuffer::with_capacity(64),
            schedule: self.schedule,
            config: self.config,
            next_block: 0,
            keep_samples: self.keep_samples,
            samples,
            out_channels,
            capture_events: self.capture_events,
            record_timing: self.record_timing,
            inject_event_cap: self.inject_event_cap,
            inject_busy: self.inject_busy,
            last_process_ns: 0,
            process_ns: Vec::new(),
            output_events: Vec::new(),
        }
    }
}

/// An event a plugin sent back to the host during `process()`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OutEvent {
    /// `CLAP_EVENT_NOTE_END`: the note tuple the host gave in the note-on. `None` means the wildcard (-1).
    NoteEnd {
        port: Option<u16>,
        channel: Option<u16>,
        key: Option<u16>,
        note_id: Option<u32>,
    },
    /// Anything else, by CLAP event type id.
    Other { type_id: u16 },
}

/// One captured output event: the block it came in, its offset inside that block, and what it was.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutRecord {
    pub block: u64,
    pub time: u32,
    pub event: OutEvent,
}

/// Drives one plugin instance block by block. Lives on a single thread.
pub struct JobRunner {
    processor: PluginAudioProcessor<Host>,
    in_ports: AudioPorts,
    out_ports: AudioPorts,
    silent_inputs: Vec<Vec<Vec<f32>>>,
    outputs: Vec<Vec<Vec<f32>>>,
    events_in: EventBuffer,
    events_out: EventBuffer,
    schedule: Schedule,
    config: RenderConfig,
    next_block: u64,
    keep_samples: bool,
    samples: Vec<f32>,
    out_channels: usize,
    capture_events: bool,
    record_timing: bool,
    inject_event_cap: Option<usize>,
    inject_busy: Option<BusyFault>,
    /// Wall time of the last plugin `process()` call, in ns.
    pub last_process_ns: u64,
    /// Wall time of every plugin `process()` call, in ns, when `record_timing` is on.
    pub process_ns: Vec<u64>,
    /// Events the plugin sent back, when `capture_events` is on.
    pub output_events: Vec<OutRecord>,
}

impl JobRunner {
    pub fn start(&mut self) -> Result<(), String> {
        self.processor
            .ensure_processing_started()
            .map(|_| ())
            .map_err(|e| format!("start_processing failed: {e}"))
    }

    pub fn blocks_done(&self) -> u64 {
        self.next_block
    }

    pub fn is_done(&self) -> bool {
        self.next_block >= self.config.blocks()
    }

    /// Processes the next block. Returns `Ok(false)` once every block has been rendered.
    pub fn step(&mut self) -> Result<bool, String> {
        if self.is_done() {
            return Ok(false);
        }
        self.process_block()?;
        Ok(true)
    }

    /// Processes one more block even past the configured length (the endless audio loop of
    /// `editor-cycles`: events come from the schedule only inside the configured length).
    pub fn process_block(&mut self) -> Result<(), String> {
        let block = u64::from(self.config.block);
        let start = self.next_block * block;
        self.events_in.clear();
        self.events_out.clear();
        let mut reset = false;
        let window = self.schedule.window(start, start + block);
        let window = match self.inject_event_cap {
            Some(cap) => &window[..window.len().min(cap)],
            None => window,
        };
        for scheduled in window {
            let time = (scheduled.frame - start) as u32;
            match &scheduled.kind {
                Kind::Reset => reset = true,
                Kind::NoteOn {
                    key,
                    note_id,
                    velocity,
                } => self.events_in.push(&NoteOnEvent::new(
                    time,
                    Pckn::new(0u16, 0u16, *key, *note_id),
                    *velocity,
                )),
                Kind::NoteOff { key, note_id } => self.events_in.push(&NoteOffEvent::new(
                    time,
                    Pckn::new(0u16, 0u16, *key, *note_id),
                    0.0,
                )),
                Kind::Param { id, value } => self.events_in.push(&ParamValueEvent::new(
                    time,
                    ClapId::new(*id),
                    Pckn::match_all(),
                    *value,
                )),
                Kind::MidiCc { channel, cc, value } => self.events_in.push(&MidiEvent::new(
                    time,
                    0,
                    [0xB0 | (channel & 0x0F), *cc & 0x7F, *value & 0x7F],
                )),
            }
        }
        for port in &mut self.outputs {
            for channel in port {
                channel.fill(0.0);
            }
        }

        let input_events = InputEvents::from_buffer(&self.events_in);
        let mut output_events = OutputEvents::from_buffer(&mut self.events_out);
        let input_audio = if self.silent_inputs.is_empty() {
            InputAudioBuffers::empty()
        } else {
            self.in_ports
                .with_input_buffers(self.silent_inputs.iter_mut().map(|port| {
                    AudioPortBuffer {
                        latency: 0,
                        channels: AudioPortBufferType::f32_input_only(
                            port.iter_mut()
                                .map(|c| InputChannel::constant(c.as_mut_slice())),
                        ),
                    }
                }))
        };
        let mut output_audio = self
            .out_ports
            .with_output_buffers(self.outputs.iter_mut().map(|port| AudioPortBuffer {
                latency: 0,
                channels: AudioPortBufferType::f32_output_only(
                    port.iter_mut().map(Vec::as_mut_slice),
                ),
            }));
        let started = self
            .processor
            .ensure_processing_started()
            .map_err(|e| format!("start_processing failed: {e}"))?;
        if reset {
            started.reset();
        }
        let called = Instant::now();
        if let Some(fault) = self.inject_busy
            && self.next_block.is_multiple_of(fault.every)
        {
            while called.elapsed().as_micros() < u128::from(fault.us) {
                std::hint::spin_loop();
            }
        }
        let result = started.process(
            &input_audio,
            &mut output_audio,
            &input_events,
            &mut output_events,
            Some(start),
            None,
        );
        self.last_process_ns = called.elapsed().as_nanos() as u64;
        result.map_err(|e| format!("process() failed at block {}: {e}", self.next_block))?;
        if self.record_timing {
            self.process_ns.push(self.last_process_ns);
        }
        if self.capture_events {
            for event in &self.events_out {
                self.output_events.push(OutRecord {
                    block: self.next_block,
                    time: event.header().time(),
                    event: match event.as_core_event() {
                        Some(CoreEventSpace::NoteEnd(end)) => OutEvent::NoteEnd {
                            port: end.port_index().into_specific(),
                            channel: end.channel().into_specific(),
                            key: end.key().into_specific(),
                            note_id: end.note_id().into_specific(),
                        },
                        _ => OutEvent::Other {
                            type_id: event.header().type_id(),
                        },
                    },
                });
            }
        }

        if self.keep_samples {
            let frames = self.config.block as usize;
            for frame in 0..frames {
                for port in &self.outputs {
                    for channel in port {
                        self.samples.push(channel[frame]);
                    }
                }
            }
        }
        self.next_block += 1;
        Ok(())
    }

    /// Stops processing and returns the processor and the interleaved samples.
    pub fn finish(self) -> (StoppedPluginAudioProcessor<Host>, Vec<f32>, usize) {
        (
            self.processor.into_stopped(),
            self.samples,
            self.out_channels,
        )
    }
}

/// How `process()` is driven.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProcessThread {
    /// A dedicated thread, like a real host's audio thread (the default).
    Dedicated,
    /// The calling thread (`render --process-on caller`: the plugin's main thread too). Reproduces hosts
    /// that process on the main thread (nice-plug then runs `execute_gui` tasks inline inside `process()`).
    Calling,
}

/// What a finished job leaves behind.
pub struct JobOutput {
    pub processor: StoppedPluginAudioProcessor<Host>,
    /// Interleaved samples (empty when the setup did not keep them).
    pub samples: Vec<f32>,
    pub channels: usize,
    /// Events the plugin sent back, when the setup captured them.
    pub output_events: Vec<OutRecord>,
    /// Wall time of every `process()` call in ns, when the setup recorded it.
    pub process_ns: Vec<u64>,
}

/// Runs a whole job on the chosen thread, returning everything it recorded.
pub fn run_job_ext(setup: JobSetup, thread: ProcessThread) -> Result<JobOutput, String> {
    fn drive(setup: JobSetup) -> Result<JobOutput, String> {
        let mut runner = setup.into_runner();
        runner.start()?;
        while runner.step()? {}
        let output_events = std::mem::take(&mut runner.output_events);
        let process_ns = std::mem::take(&mut runner.process_ns);
        let (processor, samples, channels) = runner.finish();
        Ok(JobOutput {
            processor,
            samples,
            channels,
            output_events,
            process_ns,
        })
    }
    match thread {
        ProcessThread::Calling => drive(setup),
        ProcessThread::Dedicated => std::thread::scope(|scope| {
            std::thread::Builder::new()
                .name("enth-audio".to_owned())
                .spawn_scoped(scope, move || drive(setup))
                .map_err(|e| format!("cannot spawn the audio thread: {e}"))?
                .join()
                .map_err(|_| "the audio thread panicked".to_owned())?
        }),
    }
}

/// Runs a whole job on the chosen thread.
pub fn run_job(
    setup: JobSetup,
    thread: ProcessThread,
) -> Result<(StoppedPluginAudioProcessor<Host>, Vec<f32>, usize), String> {
    let out = run_job_ext(setup, thread)?;
    Ok((out.processor, out.samples, out.channels))
}

pub fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        })
}

/// SHA-256 over the little-endian bytes of the samples.
pub fn hash_samples(samples: &[f32]) -> String {
    let mut hasher = Sha256::new();
    for s in samples {
        hasher.update(s.to_le_bytes());
    }
    hex(&hasher.finalize())
}

/// Level and sanity figures for a render.
#[derive(Clone, Debug, PartialEq)]
pub struct Stats {
    pub peak: f32,
    pub rms: f64,
    pub non_finite: usize,
    pub first_nonzero_frame: Option<usize>,
    pub last_nonzero_frame: Option<usize>,
}

pub fn stats(samples: &[f32], channels: usize) -> Stats {
    let mut peak = 0.0f32;
    let mut sum_sq = 0.0f64;
    let mut non_finite = 0;
    let (mut first, mut last) = (None, None);
    for (i, s) in samples.iter().enumerate() {
        if !s.is_finite() {
            non_finite += 1;
            continue;
        }
        peak = peak.max(s.abs());
        sum_sq += f64::from(*s) * f64::from(*s);
        if *s != 0.0 {
            let frame = i / channels.max(1);
            first.get_or_insert(frame);
            last = Some(frame);
        }
    }
    Stats {
        peak,
        rms: if samples.is_empty() {
            0.0
        } else {
            (sum_sq / samples.len() as f64).sqrt()
        },
        non_finite,
        first_nonzero_frame: first,
        last_nonzero_frame: last,
    }
}

/// The finished render of one instance.
pub struct Rendered {
    pub samples: Vec<f32>,
    pub channels: usize,
    pub sha256: String,
    pub stats: Stats,
    pub elapsed_ms: f64,
}

impl Rendered {
    pub fn new(samples: Vec<f32>, channels: usize, elapsed_ms: f64) -> Self {
        Self {
            sha256: hash_samples(&samples),
            stats: stats(&samples, channels),
            samples,
            channels,
            elapsed_ms,
        }
    }

    pub fn to_json(&self, config: &RenderConfig) -> Value {
        json!({
            "sha256": self.sha256,
            "hash_of": "interleaved little-endian f32 samples of all output channels",
            "channels": self.channels,
            "frames": config.frames,
            "peak": self.stats.peak,
            "rms": self.stats.rms,
            "non_finite_samples": self.stats.non_finite,
            "first_nonzero_frame": self.stats.first_nonzero_frame,
            "last_nonzero_frame": self.stats.last_nonzero_frame,
            "render_ms": self.elapsed_ms,
        })
    }
}

/// Opens, activates, renders and closes one instance, driving `process()` on `thread`.
pub fn render_once(
    entry: &PluginEntry,
    id: &CStr,
    request: &RenderRequest,
    thread: ProcessThread,
    save_state: bool,
) -> Result<RenderOutcome, String> {
    let mut session = Session::open(entry, id, request)?;
    let processor = session.activate()?;
    let started = Instant::now();
    let (processor, samples, channels) = run_job(session.setup(processor), thread)?;
    let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
    let state = if save_state {
        Some(session.save_state()?)
    } else {
        None
    };
    let params = session.params.clone();
    session.close(processor);
    Ok(RenderOutcome {
        rendered: Rendered::new(samples, channels, elapsed_ms),
        state,
        params,
    })
}

/// What [`render_once`] returns.
pub struct RenderOutcome {
    pub rendered: Rendered,
    /// The plugin's saved state, when asked for.
    pub state: Option<Vec<u8>>,
    pub params: Vec<ParamDesc>,
}
