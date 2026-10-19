//! `instances`: render the same script through N instances of one plugin in one process and compare
//! the hashes.
//!
//! Research report 06 saw a second `spike-shell` instance in one REAPER process render with a different
//! RMS from the first. This command separates "the plugin shares state between instances" from "the
//! host drove the two instances differently" by running the same instances four ways:
//!
//! - `sequential-fresh`: create, render and destroy instance 1, then instance 2, and so on. Only the
//!   loaded library (statics, thread-locals, allocator state) carries over.
//! - `sequential-late`: create instance 1 and render it, keep it alive, then create and render
//!   instance 2, and so on. Later instances are created after earlier ones have processed audio.
//! - `lockstep`: create all instances, then process block 1 of every instance, then block 2, on one
//!   thread. Any state the instances share is touched in an interleaved order.
//! - `concurrent`: create all instances, then render each on its own thread at the same time.
//!
//! A reference render (a lone instance) comes first. Every instance in every mode must hash equal to it.

use std::ffi::CStr;
use std::time::Instant;

use clack_host::prelude::*;
use serde_json::{Value, json};

use crate::render::{
    JobRunner, ProcessThread, RenderRequest, Rendered, Session, render_once, run_job,
};

pub const MODES: [&str; 4] = [
    "sequential-fresh",
    "sequential-late",
    "lockstep",
    "concurrent",
];

pub struct ModeResult {
    pub mode: &'static str,
    pub hashes: Vec<String>,
    pub rms: Vec<f64>,
    pub peak: Vec<f32>,
}

impl ModeResult {
    fn from_rendered(mode: &'static str, rendered: &[Rendered]) -> Self {
        Self {
            mode,
            hashes: rendered.iter().map(|r| r.sha256.clone()).collect(),
            rms: rendered.iter().map(|r| r.stats.rms).collect(),
            peak: rendered.iter().map(|r| r.stats.peak).collect(),
        }
    }

    fn identical_to(&self, reference: &str) -> bool {
        self.hashes.iter().all(|h| h == reference)
    }
}

fn sequential_fresh(
    entry: &PluginEntry,
    id: &CStr,
    request: &RenderRequest,
    n: usize,
) -> Result<ModeResult, String> {
    let mut out = Vec::new();
    for _ in 0..n {
        out.push(render_once(entry, id, request, ProcessThread::Dedicated, false)?.rendered);
    }
    Ok(ModeResult::from_rendered("sequential-fresh", &out))
}

fn sequential_late(
    entry: &PluginEntry,
    id: &CStr,
    request: &RenderRequest,
    n: usize,
) -> Result<ModeResult, String> {
    let mut alive = Vec::new();
    let mut out = Vec::new();
    for _ in 0..n {
        let mut session = Session::open(entry, id, request)?;
        let processor = session.activate()?;
        let started = Instant::now();
        let (processor, samples, channels) =
            run_job(session.setup(processor), ProcessThread::Dedicated)?;
        out.push(Rendered::new(
            samples,
            channels,
            started.elapsed().as_secs_f64() * 1000.0,
        ));
        alive.push((session, processor));
    }
    for (session, processor) in alive {
        session.close(processor);
    }
    Ok(ModeResult::from_rendered("sequential-late", &out))
}

fn open_all(
    entry: &PluginEntry,
    id: &CStr,
    request: &RenderRequest,
    n: usize,
) -> Result<Vec<(Session, StoppedPluginAudioProcessor<crate::host::Host>)>, String> {
    (0..n)
        .map(|_| {
            let mut session = Session::open(entry, id, request)?;
            let processor = session.activate()?;
            Ok((session, processor))
        })
        .collect()
}

fn lockstep(
    entry: &PluginEntry,
    id: &CStr,
    request: &RenderRequest,
    n: usize,
) -> Result<ModeResult, String> {
    let opened = open_all(entry, id, request, n)?;
    let mut sessions = Vec::new();
    let mut runners: Vec<JobRunner> = Vec::new();
    for (session, processor) in opened {
        let mut runner = session.setup(processor).into_runner();
        runner.start()?;
        runners.push(runner);
        sessions.push(session);
    }
    let started = Instant::now();
    loop {
        let mut any = false;
        for runner in &mut runners {
            any |= runner.step()?;
        }
        if !any {
            break;
        }
    }
    let elapsed = started.elapsed().as_secs_f64() * 1000.0;
    let mut out = Vec::new();
    for (session, runner) in sessions.into_iter().zip(runners) {
        let (processor, samples, channels) = runner.finish();
        out.push(Rendered::new(samples, channels, elapsed));
        session.close(processor);
    }
    Ok(ModeResult::from_rendered("lockstep", &out))
}

fn concurrent(
    entry: &PluginEntry,
    id: &CStr,
    request: &RenderRequest,
    n: usize,
) -> Result<ModeResult, String> {
    let opened = open_all(entry, id, request, n)?;
    let mut sessions = Vec::new();
    let mut setups = Vec::new();
    for (session, processor) in opened {
        setups.push(session.setup(processor));
        sessions.push(session);
    }
    let started = Instant::now();
    let results: Vec<_> = std::thread::scope(|scope| {
        let handles: Vec<_> = setups
            .into_iter()
            .enumerate()
            .map(|(i, setup)| {
                std::thread::Builder::new()
                    .name(format!("enth-audio-{i}"))
                    .spawn_scoped(scope, move || run_job(setup, ProcessThread::Calling))
                    .map_err(|e| format!("cannot spawn a thread: {e}"))
            })
            .collect();
        handles
            .into_iter()
            .map(|h| {
                h.and_then(|h| {
                    h.join()
                        .map_err(|_| "an audio thread panicked".to_owned())?
                })
            })
            .collect()
    });
    let elapsed = started.elapsed().as_secs_f64() * 1000.0;
    let mut out = Vec::new();
    for (session, result) in sessions.into_iter().zip(results) {
        let (processor, samples, channels) = result?;
        out.push(Rendered::new(samples, channels, elapsed));
        session.close(processor);
    }
    Ok(ModeResult::from_rendered("concurrent", &out))
}

pub struct Report {
    pub reference: Rendered,
    pub modes: Vec<ModeResult>,
    pub identical: bool,
}

pub fn run(
    entry: &PluginEntry,
    id: &CStr,
    request: &RenderRequest,
    n: usize,
    modes: &[&str],
) -> Result<Report, String> {
    if n < 2 {
        return Err("--instances needs at least 2".to_owned());
    }
    let reference = render_once(entry, id, request, ProcessThread::Dedicated, false)?.rendered;
    let mut results = Vec::new();
    for mode in modes {
        results.push(match *mode {
            "sequential-fresh" => sequential_fresh(entry, id, request, n)?,
            "sequential-late" => sequential_late(entry, id, request, n)?,
            "lockstep" => lockstep(entry, id, request, n)?,
            "concurrent" => concurrent(entry, id, request, n)?,
            other => {
                return Err(format!(
                    "unknown mode {other:?}; the modes are {}",
                    MODES.join(", ")
                ));
            }
        });
    }
    let identical = results.iter().all(|m| m.identical_to(&reference.sha256));
    Ok(Report {
        reference,
        modes: results,
        identical,
    })
}

impl Report {
    pub fn to_json(&self) -> Value {
        json!({
            "reference": { "sha256": self.reference.sha256, "rms": self.reference.stats.rms, "peak": self.reference.stats.peak },
            "modes": self.modes.iter().map(|m| json!({
                "mode": m.mode,
                "identical_to_reference": m.identical_to(&self.reference.sha256),
                "sha256": m.hashes,
                "rms": m.rms,
                "peak": m.peak,
            })).collect::<Vec<_>>(),
            "identical": self.identical,
        })
    }
}
