//! `process-stats`: the timing distribution of `process()` at real-time block sizes, on real-time-priority threads.
//!
//! The G1 row for `process()` ([benchmarks.md](../../../docs/benchmarks.md#real-time-measurement-p1b)) is a latency
//! percentile under the scheduling a host gives its audio thread, so this command drives the plugin like one:
//!
//! - **Paced.** Each audio thread wakes every `block / sample_rate` seconds on an absolute `CLOCK_MONOTONIC`
//!   timer (`clock_nanosleep`, `TIMER_ABSTIME`), then processes its instances. Caches go cold and cores clock
//!   down during the sleep, which an unpaced loop hides: paced time is 1.5 to 1.8 times the offline time.
//! - **Real-time priority.** A `SCHED_OTHER` thread measures the scheduler (2 to 5 % of 64-frame blocks miss their
//!   deadline on this laptop whatever the plugin does). rtkit grants `SCHED_RR` once `RLIMIT_RTTIME` is at
//!   most 200 ms, so the command lowers that limit and asks rtkit over D-Bus (`busctl`) for each thread, then
//!   reads the policy back with `sched_getscheduler` rather than trust the reply. A real-time thread that never
//!   sleeps is killed by the limit instead of freezing the desktop; this loop sleeps every period, and resynchronises
//!   instead of spinning when it falls far behind. rtkit allows 25 real-time threads per user and `PipeWire` holds
//!   four: use few. `--rt none` measures at normal priority (the report says so); `--require-rt` makes a failed
//!   promotion an error.
//! - **Warm-up.** The first `--warmup` seconds are not recorded.
//! - **One recorded number is the plugin's own**: the wall time of the CLAP `process()` call alone. The callback
//!   (the host's event building around it, for every instance of the thread), the thread's CPU time, the wake-up
//!   lateness and the deadline misses are recorded next to it. `max` is laptop noise (interrupts are charged to
//!   the running thread, `IRQ_TIME_ACCOUNTING` is off); the gates are p99 and p99.9.
//! - **`--instances N --threads T`**: N instances spread round-robin over T threads, each thread with its own
//!   timer, like a host's worker pool. 16 instances on one thread cannot keep up on this laptop.
//! - **`--editor`** opens the plugin's editor in a host-made X11 window for the whole run, to compare against a run
//!   without. Only under Xvfb (`xvfb::require_xvfb`), never on a real display.
//! - **Spiky blocks** (lane RT, [`crate::spiky`]): `--swap-state FILE --swap-ms 250 --swap-kind instant|crossfade`
//!   loads two states alternately on the main thread while the audio thread plays, and reports the swap blocks and
//!   the crossfade blocks apart from the normal ones; `--flood N --flood-ms 250` puts N note events into one block
//!   that often and reports the flood blocks.
//!
//! The report carries an environment snapshot (`uptime`, load average at the start and the end, kernel and
//! preemption model, CPU governor and frequency, `PipeWire`'s quantum, rtkit's limits) because the numbers mean
//! little without it. Run timings under `flock -x /tmp/enth-quiet.lock` so nothing builds meanwhile.

use std::ffi::CStr;
use std::sync::Barrier;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use clack_host::prelude::{PluginEntry, StoppedPluginAudioProcessor};
use serde_json::{Value, json};

use crate::host::Host;
use crate::procfs;
use crate::render::{BusyFault, JobRunner, RenderConfig, RenderRequest, Session};
use crate::spiky::{self, Blocks, Flood, Swap};

/// The G1 chord: sixteen held notes.
pub const G1_NOTES: &str = "C2@0+3600:0.8, G2@0+3600:0.8, C3@0+3600:0.8, E3@0+3600:0.8, G3@0+3600:0.8, B3@0+3600:0.8, C4@0+3600:0.8, D4@0+3600:0.8, E4@0+3600:0.8, G4@0+3600:0.8, A4@0+3600:0.8, B4@0+3600:0.8, C5@0+3600:0.8, E5@0+3600:0.8, G5@0+3600:0.8, C6@0+3600:0.8";

#[derive(Clone, Debug)]
pub struct Options {
    pub secs: f64,
    pub warmup_secs: f64,
    pub sample_rate: f64,
    pub block: u32,
    /// rtkit `SCHED_RR` priority; `None` stays at normal priority.
    pub rt: Option<u32>,
    pub require_rt: bool,
    /// CPUs to pin the audio threads to, round-robin.
    pub pin: Vec<usize>,
    pub instances: usize,
    pub threads: usize,
    pub editor: bool,
    pub notes: String,
    pub state: Option<Vec<u8>>,
    /// Budgets, in microseconds on the plugin's `process()` percentiles.
    pub max_p99_us: Option<f64>,
    pub max_p999_us: Option<f64>,
    /// A budget on each thread's callback p99 as a share of the block period.
    pub max_callback_p99_pct: Option<f64>,
    /// Negative control: a deliberate slowdown inside the timed `process()` window.
    pub inject: Option<BusyFault>,
    /// Structural edits during the run ([`crate::spiky`]).
    pub swap: Option<Swap>,
    /// Event floods during the run ([`crate::spiky`]).
    pub flood: Option<Flood>,
    /// Budgets, in microseconds, on the swap blocks' and the crossfade blocks' plugin p99.
    pub max_swap_p99_us: Option<f64>,
    pub max_crossfade_p99_us: Option<f64>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            secs: 120.0,
            warmup_secs: 10.0,
            sample_rate: 48_000.0,
            block: 64,
            rt: Some(10),
            require_rt: false,
            pin: Vec::new(),
            instances: 1,
            threads: 1,
            editor: false,
            notes: G1_NOTES.to_owned(),
            state: None,
            max_p99_us: None,
            max_p999_us: None,
            max_callback_p99_pct: None,
            inject: None,
            swap: None,
            flood: None,
            max_swap_p99_us: None,
            max_crossfade_p99_us: None,
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Clocks and scheduling

fn clock_ns(clock: libc::clockid_t) -> u64 {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: a valid out-pointer; the clocks used here always exist on Linux.
    unsafe { libc::clock_gettime(clock, &raw mut ts) };
    ts.tv_sec as u64 * 1_000_000_000 + ts.tv_nsec as u64
}

pub fn now_ns() -> u64 {
    clock_ns(libc::CLOCK_MONOTONIC)
}

fn thread_cpu_ns() -> u64 {
    clock_ns(libc::CLOCK_THREAD_CPUTIME_ID)
}

fn sleep_until_ns(t: u64) {
    let ts = libc::timespec {
        tv_sec: (t / 1_000_000_000) as libc::time_t,
        tv_nsec: (t % 1_000_000_000) as libc::c_long,
    };
    // SAFETY: a valid timespec; an absolute monotonic sleep, retried when a signal interrupts it.
    while unsafe {
        libc::clock_nanosleep(
            libc::CLOCK_MONOTONIC,
            libc::TIMER_ABSTIME,
            &raw const ts,
            std::ptr::null_mut(),
        )
    } == libc::EINTR
    {}
}

fn gettid() -> i64 {
    // SAFETY: a plain syscall with no arguments.
    unsafe { libc::syscall(libc::SYS_gettid) }
}

/// Lowers `RLIMIT_RTTIME` to 200 ms (rtkit refuses a thread of a process whose limit is higher). Process-wide,
/// done once before any audio thread starts.
fn lower_rttime_limit() -> Result<(), String> {
    let limit = libc::rlimit {
        rlim_cur: 200_000,
        rlim_max: 200_000,
    };
    // SAFETY: a valid rlimit; lowering a limit needs no privilege.
    if unsafe { libc::setrlimit(libc::RLIMIT_RTTIME, &raw const limit) } == 0 {
        Ok(())
    } else {
        Err(format!(
            "setrlimit(RLIMIT_RTTIME) failed: {}",
            std::io::Error::last_os_error()
        ))
    }
}

/// Asks rtkit, over the system bus, to make thread `tid` of this process `SCHED_RR` at `priority`.
fn rtkit_make_realtime(tid: i64, priority: u32) -> Result<(), String> {
    let output = std::process::Command::new("busctl")
        .args([
            "call",
            "--system",
            "org.freedesktop.RealtimeKit1",
            "/org/freedesktop/RealtimeKit1",
            "org.freedesktop.RealtimeKit1",
            "MakeThreadRealtimeWithPID",
            "ttu",
            &std::process::id().to_string(),
            &tid.to_string(),
            &priority.to_string(),
        ])
        .output()
        .map_err(|e| format!("cannot run busctl: {e}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_owned())
    }
}

/// The scheduling policy and priority of a thread, read back from the kernel.
fn policy_of(tid: i64) -> Value {
    // SAFETY: plain syscalls on a thread id of this process; the out-pointer is valid.
    let policy = unsafe { libc::sched_getscheduler(tid as libc::pid_t) };
    let mut param = libc::sched_param { sched_priority: 0 };
    // SAFETY: as above.
    unsafe { libc::sched_getparam(tid as libc::pid_t, &raw mut param) };
    let base = policy & !libc::SCHED_RESET_ON_FORK;
    json!({
        "policy": match base {
            libc::SCHED_OTHER => "SCHED_OTHER",
            libc::SCHED_FIFO => "SCHED_FIFO",
            libc::SCHED_RR => "SCHED_RR",
            libc::SCHED_BATCH => "SCHED_BATCH",
            libc::SCHED_IDLE => "SCHED_IDLE",
            _ => "other",
        },
        "reset_on_fork": policy & libc::SCHED_RESET_ON_FORK != 0,
        "priority": param.sched_priority,
        "real_time": base == libc::SCHED_RR || base == libc::SCHED_FIFO,
    })
}

fn pin_to(cpu: usize) -> bool {
    if cpu >= libc::CPU_SETSIZE as usize {
        return false;
    }
    // SAFETY: a zeroed cpu_set_t is valid, `cpu` is below CPU_SETSIZE, and pid 0 means the calling thread.
    unsafe {
        let mut set: libc::cpu_set_t = std::mem::zeroed();
        libc::CPU_SET(cpu, &mut set);
        libc::sched_setaffinity(0, std::mem::size_of::<libc::cpu_set_t>(), &raw const set) == 0
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Distributions

/// Nearest-rank percentile of a sorted slice, in microseconds.
fn pct_us(sorted: &[u64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let rank = ((p * sorted.len() as f64).ceil() as usize).clamp(1, sorted.len()) - 1;
    sorted[rank] as f64 / 1000.0
}

/// The edges of the log2 histogram, in microseconds.
pub const HISTOGRAM_EDGES_US: [u64; 17] = [
    1, 2, 4, 8, 16, 32, 64, 128, 256, 512, 1024, 2048, 4096, 8192, 16_384, 32_768, 65_536,
];

/// Counts of samples (ns) below each edge and past the last one.
pub fn histogram(samples_ns: &[u64]) -> Vec<u64> {
    let mut counts = vec![0u64; HISTOGRAM_EDGES_US.len() + 1];
    for ns in samples_ns {
        let us = ns / 1000;
        let bucket = HISTOGRAM_EDGES_US
            .iter()
            .position(|edge| us < *edge)
            .unwrap_or(HISTOGRAM_EDGES_US.len());
        counts[bucket] += 1;
    }
    counts
}

/// Summary of a sample set in nanoseconds: count, mean, percentiles and max, in microseconds.
pub fn dist(samples_ns: &[u64]) -> Value {
    let mut v = samples_ns.to_vec();
    v.sort_unstable();
    let mean = if v.is_empty() {
        0.0
    } else {
        v.iter().map(|x| *x as f64).sum::<f64>() / v.len() as f64 / 1000.0
    };
    json!({
        "n": v.len(),
        "min_us": v.first().map_or(0.0, |x| *x as f64 / 1000.0),
        "mean_us": mean,
        "p50_us": pct_us(&v, 0.5),
        "p90_us": pct_us(&v, 0.9),
        "p99_us": pct_us(&v, 0.99),
        "p999_us": pct_us(&v, 0.999),
        "p9999_us": pct_us(&v, 0.9999),
        "max_us": v.last().map_or(0.0, |x| *x as f64 / 1000.0),
    })
}

fn dist_with_histogram(samples_ns: &[u64]) -> Value {
    let mut d = dist(samples_ns);
    d["histogram_us"] = json!({
        "edges": HISTOGRAM_EDGES_US,
        "counts": histogram(samples_ns),
        "note": "counts[i] is the number of samples below edges[i] and at least edges[i-1]; the last count is at or above the last edge",
    });
    d
}

// ---------------------------------------------------------------------------------------------------------------
// The environment snapshot

fn read_trim(path: &str) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .map(|s| s.trim().to_owned())
}

fn command_output(program: &str, args: &[&str]) -> Option<String> {
    let out = std::process::Command::new(program)
        .args(args)
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

fn meminfo_kb(key: &str) -> Option<u64> {
    std::fs::read_to_string("/proc/meminfo")
        .ok()?
        .lines()
        .find_map(|l| {
            l.strip_prefix(key)?
                .trim()
                .strip_suffix("kB")?
                .trim()
                .parse()
                .ok()
        })
}

fn rlimit(resource: libc::__rlimit_resource_t) -> Value {
    let mut limit = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: a valid out-pointer.
    if unsafe { libc::getrlimit(resource, &raw mut limit) } == 0 {
        let show = |v: libc::rlim_t| {
            if v == libc::RLIM_INFINITY {
                json!("unlimited")
            } else {
                json!(v)
            }
        };
        json!({ "soft": show(limit.rlim_cur), "hard": show(limit.rlim_max) })
    } else {
        Value::Null
    }
}

/// `PipeWire`'s clock settings, read-only (`pw-metadata -n settings 0` prints them).
fn pipewire_settings() -> Value {
    let Some(text) = command_output("pw-metadata", &["-n", "settings", "0"]) else {
        return Value::Null;
    };
    let mut out = serde_json::Map::new();
    for line in text.lines() {
        let Some(rest) = line.split("key:'").nth(1) else {
            continue;
        };
        let Some((key, rest)) = rest.split_once('\'') else {
            continue;
        };
        if let Some(value) = rest
            .split("value:'")
            .nth(1)
            .and_then(|v| v.split_once('\''))
            && key.starts_with("clock.")
        {
            out.insert(key.to_owned(), json!(value.0));
        }
    }
    Value::Object(out)
}

fn rtkit_property(name: &str) -> Option<String> {
    command_output(
        "busctl",
        &[
            "--system",
            "get-property",
            "org.freedesktop.RealtimeKit1",
            "/org/freedesktop/RealtimeKit1",
            "org.freedesktop.RealtimeKit1",
            name,
        ],
    )
    .and_then(|s| s.split_whitespace().last().map(str::to_owned))
}

/// What the machine was doing when the numbers were taken.
pub fn environment() -> Value {
    let cpu = "/sys/devices/system/cpu/cpu0/cpufreq";
    let version = read_trim("/proc/version").unwrap_or_default();
    let cpuinfo = std::fs::read_to_string("/proc/cpuinfo").unwrap_or_default();
    json!({
        "uptime": command_output("uptime", &[]),
        "uptime_seconds": read_trim("/proc/uptime").and_then(|s| s.split_whitespace().next().map(str::to_owned)),
        "loadavg": procfs::loadavg(),
        "kernel": read_trim("/proc/sys/kernel/osrelease"),
        "kernel_build": version.find('#').map(|i| version[i..].to_owned()),
        "preempt_dynamic": version.contains("PREEMPT_DYNAMIC"),
        "cpu_model": cpuinfo.lines().find_map(|l| l.strip_prefix("model name")?.split_once(':').map(|(_, v)| v.trim().to_owned())),
        "cpus": std::thread::available_parallelism().map(usize::from).ok(),
        "governor": read_trim(&format!("{cpu}/scaling_governor")),
        "energy_performance_preference": read_trim(&format!("{cpu}/energy_performance_preference")),
        "cpu0_freq_khz": read_trim(&format!("{cpu}/scaling_cur_freq")).and_then(|s| s.parse::<u64>().ok()),
        "mem_available_kb": meminfo_kb("MemAvailable:"),
        "swap_used_kb": meminfo_kb("SwapTotal:").zip(meminfo_kb("SwapFree:")).map(|(t, f)| t - f),
        "pipewire": pipewire_settings(),
        "rtkit": {
            "max_realtime_priority": rtkit_property("MaxRealtimePriority"),
            "rttime_usec_max": rtkit_property("RTTimeUSecMax"),
        },
        "rlimit_rtprio": rlimit(libc::RLIMIT_RTPRIO),
        "rlimit_rttime": rlimit(libc::RLIMIT_RTTIME),
        "display": std::env::var("DISPLAY").ok(),
        "wayland_display": std::env::var("WAYLAND_DISPLAY").ok(),
        "test_host_build": if cfg!(debug_assertions) { "debug" } else { "release" },
    })
}

// ---------------------------------------------------------------------------------------------------------------
// The run

/// What one audio thread recorded.
struct ThreadRun {
    tid: i64,
    scheduling: Value,
    promotion: Option<String>,
    pinned: Option<(usize, bool)>,
    /// Per instance of this thread: plugin `process()` times in ns.
    plugin: Vec<Vec<u64>>,
    wall: Vec<u64>,
    cpu: Vec<u64>,
    late: Vec<u64>,
    /// When each measured block's callback started and ended (`CLOCK_MONOTONIC` ns), for [`crate::spiky`].
    starts: Vec<u64>,
    ends: Vec<u64>,
    misses: usize,
    resyncs: usize,
    processors: Vec<StoppedPluginAudioProcessor<Host>>,
}

struct Plan<'a> {
    opts: &'a Options,
    warm_blocks: usize,
    measure_blocks: usize,
    period_ns: u64,
    barrier: &'a Barrier,
    stop: &'a AtomicBool,
    finished: &'a AtomicUsize,
}

fn audio_thread(
    index: usize,
    mut runners: Vec<JobRunner>,
    plan: &Plan<'_>,
) -> Result<ThreadRun, String> {
    let tid = gettid();
    let pinned = (!plan.opts.pin.is_empty()).then(|| {
        let cpu = plan.opts.pin[index % plan.opts.pin.len()];
        (cpu, pin_to(cpu))
    });
    let promotion = plan
        .opts
        .rt
        .map(|priority| rtkit_make_realtime(tid, priority).err().unwrap_or_default());
    let scheduling = policy_of(tid);
    for runner in &mut runners {
        runner.start()?;
    }
    let total = plan.warm_blocks + plan.measure_blocks;
    let instances = runners.len();
    let mut plugin: Vec<Vec<u64>> = (0..instances)
        .map(|_| Vec::with_capacity(plan.measure_blocks))
        .collect();
    let mut wall = Vec::with_capacity(plan.measure_blocks);
    let mut cpu = Vec::with_capacity(plan.measure_blocks);
    let mut late = Vec::with_capacity(plan.measure_blocks);
    let mut starts = Vec::with_capacity(plan.measure_blocks);
    let mut ends = Vec::with_capacity(plan.measure_blocks);
    let (mut misses, mut resyncs) = (0usize, 0usize);
    plan.barrier.wait();
    let mut next = now_ns() + 20_000_000;
    for block in 0..total {
        if plan.stop.load(Ordering::Relaxed) {
            break;
        }
        sleep_until_ns(next);
        let woke = now_ns();
        let lateness = woke.saturating_sub(next);
        let cpu_before = thread_cpu_ns();
        for runner in &mut runners {
            runner.process_block()?;
        }
        let done = now_ns();
        let cpu_used = thread_cpu_ns().saturating_sub(cpu_before);
        if block >= plan.warm_blocks {
            for (k, runner) in runners.iter().enumerate() {
                plugin[k].push(runner.last_process_ns);
            }
            wall.push(done - woke);
            cpu.push(cpu_used);
            late.push(lateness);
            starts.push(woke);
            ends.push(done);
            if lateness + (done - woke) > plan.period_ns {
                misses += 1;
            }
        }
        next += plan.period_ns;
        // Far behind (a thread that cannot keep up): resynchronise rather than spin, which an RT thread
        // must not do. The blocks lost are counted as resyncs.
        if done > next + 10 * plan.period_ns {
            next = done + plan.period_ns;
            resyncs += 1;
        }
    }
    plan.finished.fetch_add(1, Ordering::SeqCst);
    Ok(ThreadRun {
        tid,
        scheduling,
        promotion,
        pinned,
        plugin,
        wall,
        cpu,
        late,
        starts,
        ends,
        misses,
        resyncs,
        processors: runners.into_iter().map(|r| r.finish().0).collect(),
    })
}

pub fn run(entry: &PluginEntry, id: &CStr, opts: &Options) -> Result<(Value, bool), String> {
    if opts.instances == 0 || opts.threads == 0 {
        return Err("--instances and --threads need at least 1".to_owned());
    }
    let threads = opts.threads.min(opts.instances);
    if opts.editor {
        crate::xvfb::require_xvfb()?;
        if opts.instances != 1 {
            return Err("--editor opens the editor of one instance: use --instances 1".to_owned());
        }
    }
    if opts.swap.is_some() {
        if opts.instances != 1 {
            return Err(
                "--swap-state loads the state of one instance: use --instances 1".to_owned(),
            );
        }
        if opts.editor || opts.flood.is_some() {
            return Err("--swap-state runs alone: no --editor or --flood with it".to_owned());
        }
    }
    if opts.rt.is_some() {
        lower_rttime_limit()?;
    }
    let period_ns = (f64::from(opts.block) / opts.sample_rate * 1e9) as u64;
    let warm_blocks = (opts.warmup_secs * opts.sample_rate / f64::from(opts.block)) as usize;
    let measure_blocks = (opts.secs * opts.sample_rate / f64::from(opts.block)) as usize;
    if measure_blocks == 0 {
        return Err("--secs is shorter than one block".to_owned());
    }
    // Long enough that the notes never reach their (dropped) note-offs.
    let config = RenderConfig::new(
        opts.sample_rate,
        opts.block,
        opts.secs + opts.warmup_secs + 5.0,
    )?;
    let request = RenderRequest {
        script: opts.notes.clone(),
        params: Vec::new(),
        state: opts.state.clone(),
        config,
    };
    let mut sessions = Vec::new();
    let mut processors = Vec::new();
    let mut flood_plan = None;
    for _ in 0..opts.instances {
        let mut session = Session::open(entry, id, &request)?;
        if let Some(flood) = opts.flood {
            flood_plan = Some(spiky::add_floods(
                &mut session.schedule,
                flood,
                opts.block,
                opts.sample_rate,
                (warm_blocks + measure_blocks) as u64,
            ));
        }
        processors.push(session.activate()?);
        sessions.push(session);
    }
    // Instances to threads, round-robin.
    let mut per_thread: Vec<Vec<JobRunnerSetup>> = (0..threads).map(|_| Vec::new()).collect();
    for (i, (session, processor)) in sessions.iter().zip(processors).enumerate() {
        let mut setup = session.setup(processor);
        setup.keep_samples = false;
        setup.inject_busy = opts.inject;
        per_thread[i % threads].push(JobRunnerSetup { instance: i, setup });
    }

    let barrier = Barrier::new(threads);
    let stop = AtomicBool::new(false);
    let finished = AtomicUsize::new(0);
    let plan = Plan {
        opts,
        warm_blocks,
        measure_blocks,
        period_ns,
        barrier: &barrier,
        stop: &stop,
        finished: &finished,
    };
    let loadavg_start = procfs::loadavg();
    let env_start = environment();
    let started = Instant::now();
    let outcome = std::thread::scope(|scope| -> Result<_, String> {
        let handles: Vec<_> = per_thread
            .into_iter()
            .enumerate()
            .map(|(index, setups)| {
                let plan = &plan;
                std::thread::Builder::new()
                    .name(format!("enth-audio-{index}"))
                    .spawn_scoped(scope, move || {
                        let instances: Vec<usize> = setups.iter().map(|s| s.instance).collect();
                        let runners = setups.into_iter().map(|s| s.setup.into_runner()).collect();
                        audio_thread(index, runners, plan).map(|run| (instances, run))
                    })
                    .map_err(|e| format!("cannot spawn an audio thread: {e}"))
            })
            .collect::<Result<_, _>>()?;

        // The main thread: the host's main loop. With --editor it keeps the editor open and serviced; with
        // --swap-state it loads the two states in turn, from the end of the warm-up.
        let mut editor_info = Value::Null;
        let mut swap_loads = (Vec::new(), None);
        if let Some(swap) = &opts.swap {
            let begin = now_ns() + (opts.warmup_secs * 1e9) as u64 + 100_000_000;
            swap_loads = spiky::drive_swaps(&mut sessions[0].instance, swap, begin, &|| {
                finished.load(Ordering::SeqCst) >= threads
            });
            if swap_loads.1.is_some() {
                stop.store(true, Ordering::Relaxed);
            }
        } else if opts.editor {
            let session = &mut sessions[0];
            editor_info = crate::editor::run_open_until(&mut session.instance, 2000, &|| {
                finished.load(Ordering::SeqCst) >= threads
            })
            .unwrap_or_else(|e| {
                stop.store(true, Ordering::Relaxed);
                json!({ "error": e })
            });
        } else {
            while finished.load(Ordering::SeqCst) < threads {
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        let runs = handles
            .into_iter()
            .map(|h| {
                h.join()
                    .map_err(|_| "an audio thread panicked".to_owned())?
            })
            .collect::<Result<Vec<_>, String>>()?;
        Ok((runs, editor_info, swap_loads))
    });
    let (runs, editor_info, (swap_loads, swap_error)) = outcome?;
    let elapsed = started.elapsed().as_secs_f64();
    let loadavg_end = procfs::loadavg();

    // Collect.
    let mut all_plugin: Vec<u64> = Vec::new();
    let mut thread_rows = Vec::new();
    let mut instance_rows = Vec::new();
    let mut total_misses = 0;
    let mut rt_ok = true;
    let mut worst_callback_p99_pct: f64 = 0.0;
    let mut finished_runs: Vec<(usize, StoppedPluginAudioProcessor<Host>)> = Vec::new();
    let mut swaps_report = Value::Null;
    let mut floods_report = Value::Null;
    let mut swap_p99s = None;
    for (index, (instances, run)) in runs.into_iter().enumerate() {
        // Spiky blocks: thread 0's first instance (a swap run has one instance; a flood hits every instance alike).
        if index == 0 && !run.plugin.is_empty() {
            let blocks = Blocks {
                starts: &run.starts,
                ends: &run.ends,
                plugin: &run.plugin[0],
                callback: &run.wall,
            };
            if let Some(swap) = &opts.swap {
                let (report, swap_p99, crossfade_p99) =
                    spiky::swap_report(swap, &swap_loads, &blocks, opts.block);
                swaps_report = report;
                swap_p99s = Some((swap_p99, crossfade_p99));
            }
            if let (Some(flood), Some(plan)) = (opts.flood, &flood_plan) {
                floods_report = spiky::flood_report(flood, plan, warm_blocks as u64, &blocks).0;
            }
        }
        total_misses += run.misses;
        let real_time = run.scheduling["real_time"] == true;
        if opts.rt.is_some() && !real_time {
            rt_ok = false;
        }
        for (k, samples) in run.plugin.iter().enumerate() {
            all_plugin.extend_from_slice(samples);
            if opts.instances > 1 {
                instance_rows.push(json!({
                    "instance": instances[k],
                    "thread": index,
                    "plugin_process": dist(samples),
                }));
            }
        }
        let mut callback = run.wall.clone();
        callback.sort_unstable();
        let callback_p99_us = pct_us(&callback, 0.99);
        let pct = callback_p99_us / (period_ns as f64 / 1000.0) * 100.0;
        worst_callback_p99_pct = worst_callback_p99_pct.max(pct);
        let wall_minus_cpu_max = run
            .wall
            .iter()
            .zip(&run.cpu)
            .map(|(w, c)| w.saturating_sub(*c))
            .max()
            .unwrap_or(0);
        thread_rows.push(json!({
            "thread": index,
            "tid": run.tid,
            "scheduling": run.scheduling,
            "rtkit": run.promotion.as_ref().map(|e| if e.is_empty() { "granted".to_owned() } else { e.clone() }),
            "pinned": run.pinned.map(|(cpu, ok)| json!({ "cpu": cpu, "ok": ok })),
            "instances": instances,
            "callback_wall": dist(&run.wall),
            "callback_p99_share_of_period_pct": pct,
            "callback_cpu": dist(&run.cpu),
            "wake_lateness": dist(&run.late),
            "max_wall_minus_cpu_us": wall_minus_cpu_max as f64 / 1000.0,
            "deadline_misses": run.misses,
            "resyncs": run.resyncs,
            "blocks": run.wall.len(),
        }));
        for (instance, processor) in instances.into_iter().zip(run.processors) {
            finished_runs.push((instance, processor));
        }
    }
    // Stop the plugins and close the sessions on the main thread.
    finished_runs.sort_by_key(|(i, _)| *i);
    for ((_, processor), session) in finished_runs.into_iter().zip(sessions) {
        session.close(processor);
    }

    let mut sorted = all_plugin.clone();
    sorted.sort_unstable();
    let p99 = pct_us(&sorted, 0.99);
    let p999 = pct_us(&sorted, 0.999);
    let mut failures = Vec::new();
    if let Some(budget) = opts.max_p99_us
        && p99 > budget
    {
        failures.push(format!(
            "process() p99 {p99:.1} us is over the {budget} us budget"
        ));
    }
    if let Some(budget) = opts.max_p999_us
        && p999 > budget
    {
        failures.push(format!(
            "process() p99.9 {p999:.1} us is over the {budget} us budget"
        ));
    }
    if let Some(budget) = opts.max_callback_p99_pct
        && worst_callback_p99_pct > budget
    {
        failures.push(format!(
            "a thread's callback p99 is {worst_callback_p99_pct:.1} % of the period, over the {budget} % budget"
        ));
    }
    if let Some(error) = &swap_error {
        failures.push(format!("a structural edit failed: {error}"));
    }
    if let Some((swap_p99, crossfade_p99)) = swap_p99s {
        if let Some(budget) = opts.max_swap_p99_us
            && swap_p99 > budget
        {
            failures.push(format!(
                "the swap blocks' process() p99 {swap_p99:.1} us is over the {budget} us budget"
            ));
        }
        if let Some(budget) = opts.max_crossfade_p99_us
            && crossfade_p99 > budget
        {
            failures.push(format!(
                "the crossfade blocks' process() p99 {crossfade_p99:.1} us is over the {budget} us budget"
            ));
        }
    }
    if opts.rt.is_some() && !rt_ok && opts.require_rt {
        failures.push("a thread did not get real-time scheduling (--require-rt)".to_owned());
    }
    if opts.editor {
        if let Some(error) = editor_info["error"].as_str() {
            failures.push(format!("the editor failed: {error}"));
        } else if editor_info["drawn"] != true {
            failures.push("the editor never drew a frame during the run".to_owned());
        }
    }
    let mut warnings = Vec::new();
    if opts.rt.is_none() || !rt_ok {
        warnings.push(
            "at least one audio thread ran at normal priority: these numbers include the scheduler's own lateness (about 2 to 5 % of 64-frame blocks miss their deadline that way on this laptop), not only the plugin's cost".to_owned(),
        );
    }
    let period_us = period_ns as f64 / 1000.0;
    let ok = failures.is_empty();
    let report = json!({
        "config": {
            "block": opts.block,
            "sample_rate": opts.sample_rate,
            "period_us": period_us,
            "secs": opts.secs,
            "warmup_secs": opts.warmup_secs,
            "measured_blocks_per_instance": measure_blocks,
            "instances": opts.instances,
            "threads": threads,
            "rt_requested": opts.rt,
            "require_rt": opts.require_rt,
            "pin": opts.pin,
            "editor": opts.editor,
            "inject": opts.inject.map(|f| format!("busy-us={} every={}", f.us, f.every)),
            "swap": opts.swap.as_ref().map(|s| json!({ "kind": s.kind.name(), "every_ms": s.every_ms, "alt_state_bytes": s.alt_state.len() })),
            "flood": opts.flood.map(|f| json!({ "note_events": f.events / 2 * 2, "every_ms": f.every_ms })),
            "notes": opts.notes,
            "state_bytes": opts.state.as_ref().map(Vec::len),
            "wall_seconds": elapsed,
        },
        "environment": env_start,
        "loadavg_start": loadavg_start,
        "loadavg_end": loadavg_end,
        "real_time": rt_ok && opts.rt.is_some(),
        "plugin_process": dist_with_histogram(&all_plugin),
        "plugin_process_share_of_period_pct": {
            "p50": pct_us(&sorted, 0.5) / period_us * 100.0,
            "p99": p99 / period_us * 100.0,
            "p999": p999 / period_us * 100.0,
            "max": sorted.last().map_or(0.0, |x| *x as f64 / 1000.0) / period_us * 100.0,
        },
        "threads": thread_rows,
        "instances": instance_rows,
        "deadline_misses": total_misses,
        "editor_info": editor_info,
        "swaps": swaps_report,
        "floods": floods_report,
        "budgets": {
            "max_p99_us": opts.max_p99_us,
            "max_p999_us": opts.max_p999_us,
            "max_callback_p99_pct": opts.max_callback_p99_pct,
            "max_swap_p99_us": opts.max_swap_p99_us,
            "max_crossfade_p99_us": opts.max_crossfade_p99_us,
        },
        "warnings": warnings,
        "failures": failures,
        "ok": ok,
    });
    Ok((report, ok))
}

struct JobRunnerSetup {
    instance: usize,
    setup: crate::render::JobSetup,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentiles_are_nearest_rank() {
        let ns: Vec<u64> = (1..=1000).map(|i| i * 1000).collect(); // 1..=1000 us
        let d = dist(&ns);
        assert_eq!(d["n"], 1000);
        assert_eq!(d["p50_us"], 500.0);
        assert_eq!(d["p99_us"], 990.0);
        assert_eq!(d["p999_us"], 999.0);
        assert_eq!(d["max_us"], 1000.0);
        assert_eq!(d["min_us"], 1.0);
        assert_eq!(dist(&[])["p99_us"], 0.0);
    }

    #[test]
    fn the_histogram_buckets_by_powers_of_two() {
        // 0.5 us, 1 us, 3 us, 100 us, 70 ms.
        let counts = histogram(&[500, 1000, 3000, 100_000, 70_000_000]);
        assert_eq!(counts.len(), HISTOGRAM_EDGES_US.len() + 1);
        assert_eq!(counts[0], 1, "below 1 us");
        assert_eq!(counts[1], 1, "1 us is in [1, 2)");
        assert_eq!(counts[2], 1, "3 us is in [2, 4)");
        assert_eq!(counts[7], 1, "100 us is in [64, 128)");
        assert_eq!(*counts.last().unwrap(), 1, "70 ms is past the last edge");
        assert_eq!(counts.iter().sum::<u64>(), 5);
    }

    #[test]
    fn clocks_advance_and_the_sleep_waits() {
        let a = now_ns();
        sleep_until_ns(a + 2_000_000);
        let b = now_ns();
        assert!(b >= a + 2_000_000, "slept {} ns", b - a);
        assert!(thread_cpu_ns() > 0);
    }

    #[test]
    fn this_thread_is_not_real_time_and_the_snapshot_reads() {
        let policy = policy_of(gettid());
        assert_eq!(policy["real_time"], false);
        let env = environment();
        assert!(env["kernel"].is_string());
        assert!(env["loadavg"].is_string());
        assert!(env["rlimit_rttime"].is_object());
        assert!(pin_to(0) || !pin_to(0)); // pinning is a best effort; the call must not crash
    }
}
