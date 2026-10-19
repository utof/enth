//! Measurement hooks for the editor (GUI thread only, never the audio thread).
//!
//! - The GUI thread is renamed `enth-gui`, so `pidstat -t` and `/proc/<pid>/task` can attribute
//!   its CPU time.
//! - With `ENTH_UI_LOG=<file>`, every frame appends `t=<unix ms> frame=<n> ui_us=<µs>
//!   repaint=<idle|animated|scope>`, and at most once a second a `window` line with the frame
//!   count, mean `ui()` time and the thread's CPU time (from `/proc/thread-self/schedstat`).

use std::fs::File;
use std::io::Write;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use spike_ui::Repaint;

/// Renames the calling thread (Linux; ignored elsewhere or on error).
pub fn name_this_thread() {
    let _ = std::fs::write("/proc/thread-self/comm", "enth-gui");
}

/// CPU time the calling thread has used, in ns (Linux).
fn thread_cpu_ns() -> Option<u64> {
    std::fs::read_to_string("/proc/thread-self/schedstat")
        .ok()?
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

fn unix_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis())
}

pub struct FrameLog {
    file: File,
    frames: u64,
    window_start: Instant,
    window_cpu: u64,
    window_frames: u32,
    window_ui: Duration,
    window_counts: [u32; 3],
}

impl FrameLog {
    /// A log if `ENTH_UI_LOG` names a file (called when the editor opens).
    pub fn from_env() -> Option<Self> {
        let path = std::env::var_os("ENTH_UI_LOG")?;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .ok()?;
        let _ = writeln!(file, "t={} open pid={}", unix_ms(), std::process::id());
        Some(Self {
            file,
            frames: 0,
            window_start: Instant::now(),
            window_cpu: thread_cpu_ns().unwrap_or(0),
            window_frames: 0,
            window_ui: Duration::ZERO,
            window_counts: [0; 3],
        })
    }

    pub fn frame(&mut self, ui_time: Duration, repaint: Repaint) {
        self.frames += 1;
        self.window_frames += 1;
        self.window_ui += ui_time;
        let (name, slot) = match repaint {
            Repaint::Idle => ("idle", 0),
            Repaint::Animated => ("animated", 1),
            Repaint::Scope => ("scope", 2),
        };
        self.window_counts[slot] += 1;
        let _ = writeln!(
            self.file,
            "t={} frame={} ui_us={} repaint={name}",
            unix_ms(),
            self.frames,
            ui_time.as_micros()
        );
        let elapsed = self.window_start.elapsed();
        if elapsed >= Duration::from_secs(1) {
            let cpu = thread_cpu_ns().unwrap_or(self.window_cpu);
            let _ = writeln!(
                self.file,
                "t={} window secs={:.3} frames={} ui_ms_mean={:.3} cpu_ms={:.2} idle={} animated={} scope={}",
                unix_ms(),
                elapsed.as_secs_f64(),
                self.window_frames,
                self.window_ui.as_secs_f64() * 1000.0 / f64::from(self.window_frames),
                cpu.saturating_sub(self.window_cpu) as f64 / 1e6,
                self.window_counts[0],
                self.window_counts[1],
                self.window_counts[2],
            );
            self.window_start = Instant::now();
            self.window_cpu = cpu;
            self.window_frames = 0;
            self.window_ui = Duration::ZERO;
            self.window_counts = [0; 3];
        }
    }
}
