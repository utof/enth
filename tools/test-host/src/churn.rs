//! `churn`: create, play and destroy a plugin N times and see what each instance leaves behind.
//!
//! nih-plug #222 is the class this catches: a wrong `Arc` count meant a plugin's `Drop` never ran, leaking
//! hundreds of MB per instance. A project opens and closes instances all day, so an instance that leaves 16 KB,
//! a thread or a file descriptor behind is a leak that a one-instance test never sees. There is no editor here
//! (`editor-cycles` has that); this is the host-only half.
//!
//! One cycle is the whole life of an instance: instantiate, load the state, activate, process a few blocks of a
//! chord that is released halfway (so note-off and release paths run), deactivate, drop. After the settle time
//! it records
//!
//! - **PSS** from `/proc/self/smaps_rollup` (shared libraries are counted once, not per process like RSS),
//! - the **thread count**, `/proc/self/task`,
//! - open **file descriptors**, `/proc/self/fd` (the listing's own descriptor is not counted),
//! - the number of memory **mappings**, `/proc/self/maps` (information).
//!
//! The first `--warmup` cycles are ignored: the first instance loads libraries, grows the allocator and warms
//! caches. Before each sample the host gives every free page back to the kernel (`malloc_trim(0)`, unless
//! `--system-malloc`; lane LK), so PSS counts the pages live allocations hold. Over the rest, the gate is the least-squares PSS slope (both over all of them and over the second
//! half) against `--max-kb-per-cycle`, and the growth in threads and descriptors against `--max-thread-growth`
//! and `--max-fd-growth` (default 0: any thread or descriptor a cycle leaves behind fails the run).
//!
//! **Negative controls.** `--inject leak-kb=N`, `--inject fd` and `--inject thread` make the host itself leak
//! N KB, a descriptor or a parked thread after every cycle, as a leaky plugin would. A run with the fault must
//! fail on the matching measure and a run without must pass, or the check proves nothing.

use std::ffi::CStr;
use std::time::Duration;

use clack_host::prelude::PluginEntry;
use serde_json::{Value, json};

use crate::procfs;
use crate::render::{ProcessThread, RenderConfig, RenderRequest, Session, run_job_ext};

/// A leak the host makes on purpose.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Inject {
    LeakKb(usize),
    Fd,
    Thread,
}

impl Inject {
    pub fn parse(text: &str) -> Result<Self, String> {
        match text.split_once('=') {
            Some(("leak-kb", n)) => n
                .parse()
                .map(Self::LeakKb)
                .map_err(|_| format!("--inject {text:?}: leak-kb needs a number of KB")),
            None if text == "fd" => Ok(Self::Fd),
            None if text == "thread" => Ok(Self::Thread),
            _ => Err(format!(
                "--inject {text:?}: churn knows `leak-kb=N`, `fd` and `thread`"
            )),
        }
    }

    fn apply(self) {
        match self {
            Self::LeakKb(kb) => {
                // Touched, so the pages are really resident, and never freed.
                Box::leak(vec![0xA5u8; kb * 1024].into_boxed_slice());
            }
            Self::Fd => {
                if let Ok(file) = std::fs::File::open("/dev/null") {
                    // The raw descriptor is never closed.
                    let _leaked = std::os::fd::IntoRawFd::into_raw_fd(file);
                }
            }
            Self::Thread => {
                std::thread::spawn(|| {
                    loop {
                        std::thread::park();
                    }
                });
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct Options {
    pub cycles: usize,
    pub warmup: usize,
    pub sample_rate: f64,
    pub block: u32,
    /// Blocks processed in every cycle.
    pub blocks: u32,
    pub notes: String,
    pub state: Option<Vec<u8>>,
    pub settle_ms: u64,
    pub max_kb_per_cycle: f64,
    pub max_thread_growth: i64,
    pub max_fd_growth: i64,
    pub inject: Option<Inject>,
    /// Give every free page back to the kernel before each sample ([`procfs::trim_heap`]; lane LK): each cycle drops
    /// a whole instance, and a leak of a few KB fills the holes it leaves in pages that are already resident (with the
    /// deterministic allocator and no trim, `--inject leak-kb=2` read 0.10 KB per cycle; with the trim, 2.43).
    pub trim: bool,
    pub process_on: ProcessThread,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            cycles: 200,
            warmup: 10,
            sample_rate: 48_000.0,
            block: 64,
            blocks: 64,
            notes: "C4@0+0.04:0.8, E4@0+0.04:0.8, G4@0.01+0.05:0.8".to_owned(),
            state: None,
            settle_ms: 10,
            max_kb_per_cycle: 1.0,
            max_thread_growth: 0,
            max_fd_growth: 0,
            inject: None,
            trim: true,
            process_on: ProcessThread::Dedicated,
        }
    }
}

/// Open file descriptors of this process, not counting the one used to list them.
pub fn fd_count() -> usize {
    std::fs::read_dir("/proc/self/fd")
        .map_or(0, Iterator::count)
        .saturating_sub(1)
}

/// Memory mappings of this process.
pub fn map_count() -> usize {
    std::fs::read_to_string("/proc/self/maps").map_or(0, |t| t.lines().count())
}

/// The first and last of `values` after the warm-up, as signed growth.
fn growth<T: Copy + Into<i64>>(values: &[T], warmup: usize) -> i64 {
    match (values.get(warmup), values.last()) {
        (Some(first), Some(last)) => (*last).into() - (*first).into(),
        _ => 0,
    }
}

#[derive(Default)]
struct Series {
    pss_kb: Vec<u64>,
    threads: Vec<usize>,
    fds: Vec<usize>,
    maps: Vec<usize>,
}

/// One whole life of an instance.
fn one_cycle(
    entry: &PluginEntry,
    id: &CStr,
    request: &RenderRequest,
    thread: ProcessThread,
) -> Result<(), String> {
    let mut session = Session::open(entry, id, request)?;
    let processor = session.activate()?;
    let mut setup = session.setup(processor);
    setup.keep_samples = false;
    let out = run_job_ext(setup, thread)?;
    session.close(out.processor);
    // `session` (the instance) drops here.
    Ok(())
}

pub fn run(entry: &PluginEntry, id: &CStr, opts: &Options) -> Result<Value, String> {
    let frames = u64::from(opts.blocks) * u64::from(opts.block);
    let request = RenderRequest {
        script: opts.notes.clone(),
        params: Vec::new(),
        state: opts.state.clone(),
        config: RenderConfig {
            sample_rate: opts.sample_rate,
            block: opts.block,
            frames,
        },
    };
    let before = procfs::memory()?;
    let before_counts = (procfs::thread_count(), fd_count(), map_count());
    let mut series = Series::default();
    for _ in 0..opts.cycles {
        one_cycle(entry, id, &request, opts.process_on)?;
        if let Some(inject) = opts.inject {
            inject.apply();
        }
        std::thread::sleep(Duration::from_millis(opts.settle_ms));
        if opts.trim {
            procfs::trim_heap();
        }
        series.pss_kb.push(procfs::memory()?.pss_kb);
        series.threads.push(procfs::thread_count());
        series.fds.push(fd_count());
        series.maps.push(map_count());
    }
    let warmup = opts.warmup.min(opts.cycles.saturating_sub(2));
    let tail: Vec<f64> = series.pss_kb[warmup..].iter().map(|v| *v as f64).collect();
    let half: Vec<f64> = series.pss_kb[series.pss_kb.len() / 2..]
        .iter()
        .map(|v| *v as f64)
        .collect();
    let slope_all = procfs::slope(&tail);
    let slope_half = procfs::slope(&half);
    let pss_growth_kb = growth(
        &series.pss_kb.iter().map(|v| *v as i64).collect::<Vec<_>>(),
        warmup,
    );
    let thread_growth = growth(
        &series.threads.iter().map(|v| *v as i64).collect::<Vec<_>>(),
        warmup,
    );
    let fd_growth = growth(
        &series.fds.iter().map(|v| *v as i64).collect::<Vec<_>>(),
        warmup,
    );
    let map_growth = growth(
        &series.maps.iter().map(|v| *v as i64).collect::<Vec<_>>(),
        warmup,
    );
    let pss_ok = slope_all.max(slope_half) <= opts.max_kb_per_cycle;
    let threads_ok = thread_growth <= opts.max_thread_growth;
    let fds_ok = fd_growth <= opts.max_fd_growth;
    let mut failures = Vec::new();
    if !pss_ok {
        failures.push(format!(
            "PSS grows {:.2} KB per cycle (second half {:.2}), more than the {} KB allowed",
            slope_all, slope_half, opts.max_kb_per_cycle
        ));
    }
    if !threads_ok {
        failures.push(format!(
            "{thread_growth} thread(s) more after {} cycles than after the warm-up (allowed {})",
            opts.cycles, opts.max_thread_growth
        ));
    }
    if !fds_ok {
        failures.push(format!(
            "{fd_growth} file descriptor(s) more after {} cycles than after the warm-up (allowed {})",
            opts.cycles, opts.max_fd_growth
        ));
    }
    Ok(json!({
        "cycles": opts.cycles,
        "warmup": warmup,
        "block": opts.block,
        "blocks_per_cycle": opts.blocks,
        "sample_rate": opts.sample_rate,
        "settle_ms": opts.settle_ms,
        "process_thread": match opts.process_on { ProcessThread::Dedicated => "a new thread per cycle", ProcessThread::Calling => "the calling thread" },
        "inject": opts.inject.map(|i| format!("{i:?}")),
        "trim_before_sample": opts.trim,
        "loadavg": procfs::loadavg(),
        "before_cycles": { "pss_kb": before.pss_kb, "threads": before_counts.0, "fds": before_counts.1, "maps": before_counts.2 },
        "series": { "pss_kb": series.pss_kb, "threads": series.threads, "fds": series.fds, "maps": series.maps },
        "summary": {
            "pss_kb_first_after_warmup": series.pss_kb.get(warmup),
            "pss_kb_last": series.pss_kb.last(),
            "pss_growth_after_warmup_kb": pss_growth_kb,
            "pss_kb_per_cycle_after_warmup": slope_all,
            "pss_kb_per_cycle_second_half": slope_half,
            "threads_min": series.threads.iter().min(),
            "threads_max": series.threads.iter().max(),
            "thread_growth_after_warmup": thread_growth,
            "fd_growth_after_warmup": fd_growth,
            "fds_last": series.fds.last(),
            "map_growth_after_warmup": map_growth,
        },
        "limits": {
            "max_kb_per_cycle": opts.max_kb_per_cycle,
            "max_thread_growth": opts.max_thread_growth,
            "max_fd_growth": opts.max_fd_growth,
        },
        "checks": { "pss": pss_ok, "threads": threads_ok, "fds": fds_ok },
        "failures": failures,
        "ok": failures.is_empty(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_injected_faults_parse() {
        assert_eq!(Inject::parse("leak-kb=8").unwrap(), Inject::LeakKb(8));
        assert_eq!(Inject::parse("fd").unwrap(), Inject::Fd);
        assert_eq!(Inject::parse("thread").unwrap(), Inject::Thread);
        assert!(Inject::parse("leak-kb=x").is_err());
        assert!(Inject::parse("nope").is_err());
    }

    #[test]
    fn growth_is_last_minus_the_first_after_warmup() {
        assert_eq!(growth(&[1i64, 2, 5, 9], 1), 7);
        assert_eq!(growth(&[4i64], 3), 0);
    }

    #[test]
    fn descriptors_and_mappings_are_counted() {
        // Other tests open and close descriptors at the same time, so hold 64 and count at least those.
        let held: Vec<_> = (0..64)
            .map(|_| std::fs::File::open("/dev/null").unwrap())
            .collect();
        assert!(fd_count() >= 64);
        drop(held);
        assert!(map_count() > 10);
    }
}
