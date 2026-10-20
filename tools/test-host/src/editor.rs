//! `editor-cycles`: open and close a plugin's embedded editor N times and record what it costs.
//!
//! **Only under Xvfb.** [`crate::xvfb::require_xvfb`] runs first and refuses anything else (AGENTS.md, "UI").
//!
//! One run is one process: the host loads the plugin, activates it, keeps `process()` running on an
//! audio thread (a held chord, paced at real time) and then, N times, creates the editor in a host-made
//! X11 parent window, waits for the first drawn frame, holds it open, and destroys it. After each
//! close it records PSS (`/proc/self/smaps_rollup`) and the thread count. The `baseline` mode does
//! everything except the plugin's GUI calls (it still creates, shows and destroys the parent window), so
//! growth that the host itself causes shows up there and not in the plugin's account.
//!
//! "Drawn" means a pixel in the parent window differs from the colour the host painted it, read back
//! with `GetImage`: a first open that never paints stays that colour (the nice-plug #95 symptom).
//!
//! **What makes the numbers repeatable** (lane LK, R1/S1; `docs/plugin.md`, "Leak gates"):
//!
//! - A measurement process sets glibc's allocator to one arena with blocks of 128 KiB or more mapped on their own and
//!   trimmed at once ([`deterministic_malloc`], before the plugin is loaded; `--system-malloc` leaves the defaults).
//!   With the defaults, each editor's window thread can get another arena, and the freed ~55 MB of the last one stays
//!   resident: one run read 405 KB per cycle that way, and a 9 MB block that stayed or went at random read 53 KB.
//! - Each sample records `SwapPss` beside `Pss_Anon`; their sum is the anonymous footprint whether or not the kernel
//!   swapped part of it out (lane SOAK's metric: `Pss_Anon` alone fell 130 KB per cycle in a run that swapped).
//! - The thread count after a close is read once the threads that are still exiting are gone ([`settled_threads`]):
//!   under load, Mesa's `disk$0` queue thread was sometimes still listed 50 ms after a close and gone by the next.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::{CStr, c_ulong};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use clack_extensions::gui::{GuiApiType, GuiConfiguration, GuiSize, PluginGui, Window};
use clack_host::prelude::*;
use serde_json::{Value, json};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{ConnectionExt, CreateWindowAux, ImageFormat, WindowClass};
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;

use crate::host::Host;
use crate::leak_diag::Diag;
use crate::procfs;
use crate::render::{RenderConfig, RenderRequest, Session};

/// The colour the host paints its parent window; any other pixel inside it means the plugin drew.
const PARENT_BG: u32 = 0x00FF_00FF;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Editor,
    Baseline,
}

impl Mode {
    pub fn name(self) -> &'static str {
        match self {
            Mode::Editor => "editor",
            Mode::Baseline => "baseline",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Options {
    pub cycles: usize,
    /// Cycles ignored when fitting the growth rate (the first opens load Mesa and warm caches).
    pub warmup: usize,
    /// How long the editor stays open after its first frame.
    pub hold_ms: u64,
    /// How long the host waits after closing before it measures.
    pub settle_ms: u64,
    pub first_frame_timeout_ms: u64,
    pub sample_rate: f64,
    pub block: u32,
    /// Size of the host window in baseline mode (editor mode uses the size the plugin reports).
    pub window: (u16, u16),
    pub mode: Mode,
}

/// The block size from which [`deterministic_malloc`] maps blocks on their own, and the free space at the top of the
/// heap above which it trims: 128 KiB, glibc's starting value for both, held fixed.
const MALLOC_THRESHOLD: i32 = 128 * 1024;
/// How long a close gets for its exiting threads to go before the count is recorded, and how often it looks.
pub const THREAD_SETTLE_MS: u64 = 1000;
const THREAD_POLL_MS: u64 = 10;
/// After this many closes whose threads outlived [`THREAD_SETTLE_MS`], a run stops waiting: those threads are not
/// exiting, and a thread left behind by every close would otherwise cost a second per cycle.
const THREAD_SETTLE_GIVE_UP: usize = 5;

/// Makes glibc's allocator deterministic for a measurement process; call it before the plugin is loaded and before
/// any thread starts. One arena (`M_ARENA_MAX` 1), so a new window thread never brings a fresh arena while the last
/// one keeps its freed memory; blocks of [`MALLOC_THRESHOLD`] or more mapped on their own, and the heap trimmed once
/// that much is free at its top, both fixed (setting them turns off glibc's sliding thresholds, which follow the
/// largest block freed so far; with them a 9 MB block was still resident after some closes and not after others). A
/// leak still shows: memory never freed stays resident whatever the policy, and memory freed is given back instead of
/// hiding the next leak in pages that are already resident (a 26 KB per open leak read 16 and 21 KB per cycle with
/// glibc's defaults, 26 more than the residual with these). Returns what it set, for the report.
pub fn deterministic_malloc() -> Value {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    {
        // SAFETY: `mallopt` only changes the allocator's parameters; it takes the arena lock itself.
        let set = |param, value| unsafe { libc::mallopt(param, value) } == 1;
        if set(libc::M_ARENA_MAX, 1)
            && set(libc::M_MMAP_THRESHOLD, MALLOC_THRESHOLD)
            && set(libc::M_TRIM_THRESHOLD, MALLOC_THRESHOLD)
        {
            json!({ "arena_max": 1, "mmap_threshold": MALLOC_THRESHOLD, "trim_threshold": MALLOC_THRESHOLD })
        } else {
            json!("system (mallopt refused)")
        }
    }
    #[cfg(not(all(target_os = "linux", target_env = "gnu")))]
    {
        json!("system (not glibc)")
    }
}

/// The thread count after a close, given time for threads that are still exiting to go: starting from the `first`
/// read, it waits (`wait`, then `count` again) while the count is above `ceiling` (the count after the previous close,
/// or before the first open), at most `polls` times. A thread that stays is counted. Returns the count and the number
/// of waits.
pub fn settled_threads(
    first: usize,
    ceiling: usize,
    polls: usize,
    mut count: impl FnMut() -> usize,
    mut wait: impl FnMut(),
) -> (usize, usize) {
    let mut n = first;
    let mut waits = 0;
    while n > ceiling && waits < polls {
        wait();
        waits += 1;
        n = count();
    }
    (n, waits)
}

impl Default for Options {
    fn default() -> Self {
        Self {
            cycles: 200,
            warmup: 10,
            hold_ms: 100,
            settle_ms: 50,
            first_frame_timeout_ms: 1000,
            sample_rate: 48_000.0,
            block: 256,
            window: (360, 200),
            mode: Mode::Editor,
        }
    }
}

/// The host's X11 connection and the parent windows it makes.
struct XServer {
    conn: RustConnection,
    root: u32,
}

#[derive(Clone, Copy, Debug, Default)]
struct Pixels {
    total: usize,
    non_background: usize,
}

impl XServer {
    fn connect() -> Result<Self, String> {
        let (conn, screen) =
            x11rb::connect(None).map_err(|e| format!("cannot connect to the X server: {e}"))?;
        let root = conn.setup().roots[screen].root;
        Ok(Self { conn, root })
    }

    fn x_err(e: impl std::fmt::Display) -> String {
        format!("X11: {e}")
    }

    fn create_parent(&self, width: u16, height: u16) -> Result<u32, String> {
        let window = self.conn.generate_id().map_err(Self::x_err)?;
        self.conn
            .create_window(
                x11rb::COPY_FROM_PARENT as u8,
                window,
                self.root,
                0,
                0,
                width,
                height,
                0,
                WindowClass::INPUT_OUTPUT,
                x11rb::COPY_FROM_PARENT,
                &CreateWindowAux::new().background_pixel(PARENT_BG),
            )
            .map_err(Self::x_err)?;
        self.conn.map_window(window).map_err(Self::x_err)?;
        self.conn.sync().map_err(Self::x_err)?;
        Ok(window)
    }

    fn resize(&self, window: u32, width: u32, height: u32) -> Result<(), String> {
        self.conn
            .configure_window(
                window,
                &x11rb::protocol::xproto::ConfigureWindowAux::new()
                    .width(width)
                    .height(height),
            )
            .map_err(Self::x_err)?;
        self.conn.flush().map_err(Self::x_err)
    }

    fn destroy(&self, window: u32) -> Result<(), String> {
        self.conn.destroy_window(window).map_err(Self::x_err)?;
        self.conn.sync().map_err(Self::x_err)?;
        // Drain whatever the server queued for us (errors for windows the plugin already removed).
        while let Ok(Some(_)) = self.conn.poll_for_event() {}
        Ok(())
    }

    /// Reads back up to 16 one-pixel-high strips across the window: cheap enough to poll every millisecond.
    fn probe(&self, window: u32, width: u16, height: u16) -> Result<Pixels, String> {
        let rows = height.clamp(1, 16);
        let cookies: Vec<_> = (0..rows)
            .map(|i| {
                let y = (u32::from(height) * (2 * u32::from(i) + 1) / (2 * u32::from(rows))) as i16;
                self.conn
                    .get_image(ImageFormat::Z_PIXMAP, window, 0, y, width, 1, !0)
            })
            .collect::<Result<_, _>>()
            .map_err(Self::x_err)?;
        let mut pixels = Pixels::default();
        for cookie in cookies {
            let reply = cookie.reply().map_err(Self::x_err)?;
            for px in reply.data.as_chunks::<4>().0 {
                pixels.total += 1;
                if u32::from_le_bytes([px[0], px[1], px[2], 0]) != PARENT_BG {
                    pixels.non_background += 1;
                }
            }
        }
        Ok(pixels)
    }

    /// Reads back the whole window and counts its distinct colours (a drawn egui panel has dozens; a
    /// blank or black window has one).
    fn distinct_colors(&self, window: u32, width: u16, height: u16) -> Result<usize, String> {
        let reply = self
            .conn
            .get_image(ImageFormat::Z_PIXMAP, window, 0, 0, width, height, !0)
            .map_err(Self::x_err)?
            .reply()
            .map_err(Self::x_err)?;
        let colors: BTreeSet<u32> = reply
            .data
            .as_chunks::<4>()
            .0
            .iter()
            .map(|px| u32::from_le_bytes([px[0], px[1], px[2], 0]))
            .collect();
        Ok(colors.len())
    }
}

/// Services the plugin's requests that a real host answers on its main thread.
fn service(instance: &mut PluginInstance<Host>, x: &XServer, parent: Option<u32>) {
    if instance.access_shared_handler(|s| s.callback_requested.swap(false, Ordering::SeqCst)) {
        instance.call_on_main_thread_callback();
    }
    let resize =
        instance.access_shared_handler(|s| s.pending_resize.lock().ok().and_then(|mut p| p.take()));
    if let (Some((w, h)), Some(window)) = (resize, parent) {
        let _ = x.resize(window, w, h);
    }
    while let Ok(Some(_)) = x.conn.poll_for_event() {}
}

fn pause(instance: &mut PluginInstance<Host>, x: &XServer, parent: Option<u32>, for_ms: u64) {
    let end = Instant::now() + Duration::from_millis(for_ms);
    while Instant::now() < end {
        service(instance, x, parent);
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[derive(Default)]
struct Series {
    pss_kb: Vec<u64>,
    pss_anon_kb: Vec<u64>,
    swap_pss_kb: Vec<u64>,
    threads_closed: Vec<usize>,
    threads_open: Vec<usize>,
    open_ms: Vec<Option<f64>>,
    drawn_in_time: Vec<bool>,
    final_colors: Vec<Option<usize>>,
    /// Whether the host's parent window read back as all background colour before the plugin drew
    /// (proves the "drawn" probe can tell a blank window from a drawn one).
    blank_before_show: Vec<bool>,
    /// The closes whose thread count had to settle: cycle, first and settled count, time waited, names at first.
    late_thread_exits: Vec<Value>,
}

fn median(values: &[f64]) -> Option<f64> {
    let mut v = values.to_vec();
    v.sort_by(f64::total_cmp);
    match v.len() {
        0 => None,
        n if n % 2 == 1 => Some(v[n / 2]),
        n => Some(f64::midpoint(v[n / 2 - 1], v[n / 2])),
    }
}

fn percentile(values: &[f64], p: f64) -> Option<f64> {
    let mut v = values.to_vec();
    v.sort_by(f64::total_cmp);
    if v.is_empty() {
        return None;
    }
    let rank = ((v.len() - 1) as f64 * p).round() as usize;
    Some(v[rank])
}

/// Runs one measurement process: `opts.cycles` open/close cycles (or host-only cycles in baseline mode).
pub fn run_cycles(entry: &PluginEntry, id: &CStr, opts: &Options) -> Result<Value, String> {
    let xvfb = crate::xvfb::require_xvfb()?;
    let x = XServer::connect()?;

    // A held chord keeps the audio thread doing real work for the whole run.
    let request = RenderRequest {
        script: "C4@0+3600, E4@0+3600, G4@0+3600".to_owned(),
        params: Vec::new(),
        state: None,
        config: RenderConfig::new(opts.sample_rate, opts.block, 3600.0)?,
    };
    let mut session = Session::open(entry, id, &request)?;
    let processor = session.activate()?;
    let mut setup = session.setup(processor);
    setup.keep_samples = false;

    let gui = session
        .instance
        .plugin_handle()
        .get_extension::<PluginGui>();
    if opts.mode == Mode::Editor && gui.is_none() {
        return Err("the plugin has no gui extension".to_owned());
    }

    let stop = AtomicBool::new(false);
    let period = Duration::from_secs_f64(f64::from(opts.block) / opts.sample_rate);
    let before = procfs::memory()?;
    let threads_before = procfs::thread_count();
    let names_before = procfs::thread_names();

    let outcome = std::thread::scope(|scope| {
        let audio = std::thread::Builder::new()
            .name("enth-audio".to_owned())
            .spawn_scoped(scope, || -> Result<_, String> {
                let mut runner = setup.into_runner();
                runner.start()?;
                let mut next = Instant::now();
                while !stop.load(Ordering::Relaxed) {
                    runner.process_block()?;
                    next += period;
                    let now = Instant::now();
                    if next > now {
                        std::thread::sleep(next - now);
                    } else {
                        next = now;
                    }
                }
                let (processor, _, _) = runner.finish();
                Ok(processor)
            })
            .map_err(|e| format!("cannot spawn the audio thread: {e}"))?;

        let result = cycle_loop(&mut session.instance, gui, &x, opts);
        stop.store(true, Ordering::Relaxed);
        let processor = audio
            .join()
            .map_err(|_| "the audio thread panicked".to_owned())??;
        Ok::<_, String>((result?, processor))
    });
    let ((series, extras), processor) = outcome?;
    session.close(processor);

    let names_end = procfs::thread_names();
    let w = opts.warmup.min(series.pss_kb.len().saturating_sub(2));
    let tail: Vec<f64> = series.pss_kb[w..].iter().map(|v| *v as f64).collect();
    let half: Vec<f64> = series.pss_kb[series.pss_kb.len() / 2..]
        .iter()
        .map(|v| *v as f64)
        .collect();
    let open: Vec<f64> = series.open_ms.iter().flatten().copied().collect();
    let drawn = series.drawn_in_time.iter().filter(|d| **d).count();
    let first = series.pss_kb.first().copied().unwrap_or(0);
    let last = series.pss_kb.last().copied().unwrap_or(0);
    let threads_tail = &series.threads_closed[w..];
    Ok(json!({
        "mode": opts.mode.name(),
        "display": xvfb.display,
        "xvfb_pid": xvfb.pid,
        "cycles": series.pss_kb.len(),
        "warmup": w,
        "hold_ms": opts.hold_ms,
        "settle_ms": opts.settle_ms,
        "block": opts.block,
        "sample_rate": opts.sample_rate,
        "loadavg": procfs::loadavg(),
        "before_cycles": { "pss_kb": before.pss_kb, "pss_anon_kb": before.pss_anon_kb, "threads": threads_before },
        "thread_settle_ms": THREAD_SETTLE_MS,
        "series": {
            "pss_kb": series.pss_kb,
            "pss_anon_kb": series.pss_anon_kb,
            "swap_pss_kb": series.swap_pss_kb,
            "threads_closed": series.threads_closed,
            "threads_open": series.threads_open,
            "open_ms": series.open_ms,
            "drawn_in_time": series.drawn_in_time,
            "final_colors": series.final_colors,
            "blank_before_show": series.blank_before_show,
        },
        "summary": {
            "pss_kb_first": first,
            "pss_kb_last": last,
            "pss_growth_total_kb": last as i64 - first as i64,
            "pss_growth_kb_per_cycle_after_warmup": procfs::slope(&tail),
            "pss_growth_kb_per_cycle_second_half": procfs::slope(&half),
            "pss_growth_after_warmup_kb": tail.last().zip(tail.first()).map(|(l, f)| l - f),
            "threads_min": series.threads_closed.iter().min(),
            "threads_max": series.threads_closed.iter().max(),
            "threads_constant_after_warmup": threads_tail.iter().all(|t| *t == threads_tail[0]),
            "threads_open_max": series.threads_open.iter().max(),
            "late_thread_exit_cycles": series.late_thread_exits.len(),
            "opens_drawn_in_time": drawn,
            "first_open_drawn_in_time": series.drawn_in_time.first(),
            "open_ms": {
                "min": open.iter().copied().min_by(f64::total_cmp),
                "median": median(&open),
                "p95": percentile(&open, 0.95),
                "max": open.iter().copied().max_by(f64::total_cmp),
            },
        },
        "thread_names_before": names_before,
        "thread_names_open_first_cycle": extras.thread_names_open_first,
        "thread_names_closed_last_cycle": extras.thread_names_closed_last,
        "thread_names_after_shutdown": names_end,
        "gui_closed_notifications": extras.gui_closed,
        "resize_requests": extras.resize_requests,
        "create_failures": extras.create_failures,
        "plugin_log": extras.plugin_log,
        "late_thread_exits": series.late_thread_exits,
        "diag": extras.diag.report(),
    }))
}

/// Keeps the plugin's editor open in a host-made X11 window until `done()` says stop, servicing the plugin's main
/// thread requests meanwhile (`process-stats --editor`: the audio thread's cost with a GUI alive). Only under Xvfb.
pub fn run_open_until(
    instance: &mut PluginInstance<Host>,
    first_frame_timeout_ms: u64,
    done: &dyn Fn() -> bool,
) -> Result<Value, String> {
    let xvfb = crate::xvfb::require_xvfb()?;
    let x = XServer::connect()?;
    let gui = instance
        .plugin_handle()
        .get_extension::<PluginGui>()
        .ok_or("the plugin has no gui extension")?;
    let config = GuiConfiguration {
        api_type: GuiApiType::X11,
        is_floating: false,
    };
    if !gui.is_api_supported(&instance.plugin_handle(), config) {
        return Err("the plugin does not support an embedded X11 editor".to_owned());
    }
    let started = Instant::now();
    gui.create(&instance.plugin_handle(), config)
        .map_err(|e| format!("gui.create failed: {e}"))?;
    let size = {
        let handle = instance.plugin_handle();
        let _ = gui.set_scale(&handle, 1.0);
        gui.get_size(&handle).unwrap_or(GuiSize {
            width: 360,
            height: 200,
        })
    };
    let (w, h) = (size.width as u16, size.height as u16);
    let parent = x.create_parent(w, h)?;
    {
        let handle = instance.plugin_handle();
        // SAFETY: `parent` is a live X11 window until we destroy it after `gui.destroy`.
        unsafe { gui.set_parent(&handle, Window::from_x11_handle(c_ulong::from(parent))) }
            .map_err(|e| format!("gui.set_parent failed: {e}"))?;
        gui.show(&handle)
            .map_err(|e| format!("gui.show failed: {e}"))?;
    }
    let deadline = started + Duration::from_millis(first_frame_timeout_ms);
    let mut first_frame_ms = None;
    while !done() {
        service(instance, &x, Some(parent));
        if first_frame_ms.is_none() {
            if x.probe(parent, w, h)?.non_background > 0 {
                first_frame_ms = Some(started.elapsed().as_secs_f64() * 1000.0);
            } else if Instant::now() > deadline {
                first_frame_ms = Some(f64::NAN);
            }
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    let colors = x.distinct_colors(parent, w, h).ok();
    {
        let handle = instance.plugin_handle();
        let _ = gui.hide(&handle);
        gui.destroy(&handle);
    }
    x.destroy(parent)?;
    Ok(json!({
        "display": xvfb.display,
        "size": [w, h],
        "first_frame_ms": first_frame_ms.filter(|v| v.is_finite()),
        "drawn": first_frame_ms.is_some_and(f64::is_finite),
        "distinct_colors": colors,
        "open_seconds": started.elapsed().as_secs_f64(),
    }))
}

#[derive(Default)]
struct Extras {
    thread_names_open_first: BTreeMap<String, usize>,
    thread_names_closed_last: BTreeMap<String, usize>,
    gui_closed: u32,
    resize_requests: u32,
    create_failures: u32,
    plugin_log: Vec<String>,
    diag: Diag,
}

fn cycle_loop(
    instance: &mut PluginInstance<Host>,
    gui: Option<PluginGui>,
    x: &XServer,
    opts: &Options,
) -> Result<(Series, Extras), String> {
    let mut series = Series::default();
    let mut extras = Extras {
        diag: Diag::from_env(),
        ..Extras::default()
    };
    let threads_idle = procfs::thread_count();
    let config = GuiConfiguration {
        api_type: GuiApiType::X11,
        is_floating: false,
    };
    if let (Mode::Editor, Some(gui)) = (opts.mode, &gui) {
        let handle = instance.plugin_handle();
        if !gui.is_api_supported(&handle, config) {
            return Err("the plugin does not support an embedded X11 editor".to_owned());
        }
    }
    for _ in 0..opts.cycles {
        let started = Instant::now();
        let mut open_ms = None;
        let mut drawn_in_time = false;
        let mut final_colors = None;
        let mut blank_before_show = true;
        let threads_open;
        if let (Mode::Editor, Some(gui)) = (opts.mode, &gui) {
            {
                let create = {
                    let handle = instance.plugin_handle();
                    gui.create(&handle, config)
                        .map_err(|e| format!("gui.create failed: {e}"))
                };
                if let Err(e) = create {
                    extras.create_failures += 1;
                    extras.plugin_log.push(e);
                    series.open_ms.push(None);
                    series.drawn_in_time.push(false);
                    series.final_colors.push(None);
                    series.blank_before_show.push(true);
                    series.threads_open.push(procfs::thread_count());
                    let threads = settle_threads(instance, x, &mut series, threads_idle);
                    sample(&mut series, &mut extras.diag, threads)?;
                    continue;
                }
                let size = {
                    let handle = instance.plugin_handle();
                    let _ = gui.set_scale(&handle, 1.0);
                    gui.get_size(&handle).unwrap_or(GuiSize {
                        width: u32::from(opts.window.0),
                        height: u32::from(opts.window.1),
                    })
                };
                let (w, h) = (size.width as u16, size.height as u16);
                let parent = x.create_parent(w, h)?;
                blank_before_show = x.probe(parent, w, h)?.non_background == 0;
                {
                    let handle = instance.plugin_handle();
                    // SAFETY: `parent` is a live X11 window until we destroy it after `gui.destroy`.
                    unsafe {
                        gui.set_parent(&handle, Window::from_x11_handle(c_ulong::from(parent)))
                    }
                    .map_err(|e| format!("gui.set_parent failed: {e}"))?;
                    gui.show(&handle)
                        .map_err(|e| format!("gui.show failed: {e}"))?;
                }
                let deadline = started + Duration::from_millis(opts.first_frame_timeout_ms);
                while Instant::now() < deadline {
                    service(instance, x, Some(parent));
                    if x.probe(parent, w, h)?.non_background > 0 {
                        open_ms = Some(started.elapsed().as_secs_f64() * 1000.0);
                        drawn_in_time = true;
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(1));
                }
                pause(instance, x, Some(parent), opts.hold_ms);
                final_colors = x.distinct_colors(parent, w, h).ok();
                threads_open = procfs::thread_count();
                if extras.thread_names_open_first.is_empty() {
                    extras.thread_names_open_first = procfs::thread_names();
                }
                {
                    let handle = instance.plugin_handle();
                    let _ = gui.hide(&handle);
                    gui.destroy(&handle);
                }
                x.destroy(parent)?;
            }
        } else {
            {
                let (w, h) = opts.window;
                let parent = x.create_parent(w, h)?;
                pause(instance, x, Some(parent), opts.hold_ms);
                threads_open = procfs::thread_count();
                x.destroy(parent)?;
            }
        }
        series.open_ms.push(open_ms);
        series.drawn_in_time.push(drawn_in_time);
        series.final_colors.push(final_colors);
        series.blank_before_show.push(blank_before_show);
        series.threads_open.push(threads_open);
        pause(instance, x, None, opts.settle_ms);
        let threads = settle_threads(instance, x, &mut series, threads_idle);
        sample(&mut series, &mut extras.diag, threads)?;
        extras
            .diag
            .follow_up(&mut |ms| pause(instance, x, None, ms));
    }
    extras.thread_names_closed_last = procfs::thread_names();
    extras.gui_closed = instance.access_shared_handler(|s| s.gui_closed.load(Ordering::SeqCst));
    extras.resize_requests =
        instance.access_shared_handler(|s| s.resize_requests.load(Ordering::SeqCst));
    extras.plugin_log.extend(
        instance.access_shared_handler(|s| s.log.lock().map(|l| l.clone()).unwrap_or_default()),
    );
    Ok((series, extras))
}

/// The thread count after this cycle's close ([`settled_threads`]), noting a close that needed the wait.
fn settle_threads(
    instance: &mut PluginInstance<Host>,
    x: &XServer,
    series: &mut Series,
    threads_idle: usize,
) -> usize {
    let ceiling = series
        .threads_closed
        .last()
        .copied()
        .unwrap_or(threads_idle);
    let first = procfs::thread_count();
    let stayed = series
        .late_thread_exits
        .iter()
        .filter(|e| e["settled"].as_u64() > e["ceiling"].as_u64())
        .count();
    if first <= ceiling || stayed >= THREAD_SETTLE_GIVE_UP {
        return first;
    }
    let names = procfs::thread_names();
    let started = Instant::now();
    let (threads, waits) = settled_threads(
        first,
        ceiling,
        (THREAD_SETTLE_MS / THREAD_POLL_MS) as usize,
        procfs::thread_count,
        || pause(instance, x, None, THREAD_POLL_MS),
    );
    series.late_thread_exits.push(json!({
        "cycle": series.threads_closed.len(), "ceiling": ceiling, "first": first, "settled": threads, "waits": waits,
        "waited_ms": started.elapsed().as_secs_f64() * 1000.0, "names_at_first": names,
    }));
    threads
}

fn sample(series: &mut Series, diag: &mut Diag, threads: usize) -> Result<(), String> {
    let memory = procfs::memory()?;
    diag.observe(series.pss_kb.len(), memory, threads);
    series.pss_kb.push(memory.pss_kb);
    series.pss_anon_kb.push(memory.pss_anon_kb);
    series.swap_pss_kb.push(memory.swap_pss_kb);
    series.threads_closed.push(threads);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_thread_that_is_still_exiting_is_let_go_and_one_that_stays_is_counted() {
        // Mesa's disk$0 thread under load: listed at the first read, gone two polls later.
        let mut reads = [4, 4, 3].into_iter();
        let mut waits = 0;
        let (n, polls) = settled_threads(4, 3, 100, || reads.next().unwrap_or(3), || waits += 1);
        assert_eq!((n, polls, waits), (3, 3, 3));

        // A thread left behind by every close never goes: counted after the last poll.
        let (n, polls) = settled_threads(4, 3, 100, || 4, || {});
        assert_eq!((n, polls), (4, 100));

        // Nothing above the ceiling: no wait at all.
        let (n, polls) = settled_threads(3, 3, 100, || unreachable!(), || unreachable!());
        assert_eq!((n, polls), (3, 0));
    }

    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    #[test]
    fn the_allocator_takes_the_deterministic_settings() {
        let set = deterministic_malloc();
        assert_eq!(set["arena_max"], 1, "{set}");
        assert_eq!(set["mmap_threshold"], MALLOC_THRESHOLD);
    }
}
