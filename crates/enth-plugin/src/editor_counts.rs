//! Editor lifecycle and frame counters (lane INT; frames by lane AM, R1/S1), behind the `editor-counts` cargo feature:
//! without it this module is not compiled and the editor counts nothing.
//!
//! The leak gate `cargo xtask leak-test` opens and closes the editor 200 times in `enth-test-host editor-cycles` and
//! must see the vendored nice-plug-egui call `NiceEguiApp::editor_closed` once per `build()` (the fork's patch 2;
//! stock nice-plug-egui 0.6.0 never calls it). The host cannot see that call, so the plugin counts both: `build()` and
//! `editor_closed()` each add one (relaxed atomics, on baseview's GUI thread).
//!
//! **Frames** (lane AM): every drawn frame (`NiceEguiApp::ui`) adds one, and a frame that follows one which asked for
//! a repaint (visible live data was changing) is a *live* frame: the time since that frame counts as live time. So
//! `live_frames / live_ms` is the frame rate the window really got while it animated, whatever baseview made of the
//! editor's 1/60 s requests (baseview 0.4.1 paces redraws at the X Present `msc + 2`), and idle time (0 frames) does
//! not dilute it. Owner check 1.3 reads it from a REAPER session with `ENTH_EDITOR_COUNTS` set; `leak-test` reports
//! the frames per run.
//!
//! When the last reference to the plugin's [`Shared`](crate::Shared) goes (the plugin and every editor are gone, so
//! no GUI thread can still be counting) and the environment variable `ENTH_EDITOR_COUNTS` names a file, one JSON line
//! (`enth.plugin-editor-counts/1`; the frame fields are additive) is appended to it (docs/plugin.md, "Leak gates").

use std::io::Write;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Instant;

use serde_json::{Value, json};

/// The environment variable naming the file the report is appended to.
pub const ENV: &str = "ENTH_EDITOR_COUNTS";

/// How often the editor was built and closed, and the frames it drew.
#[derive(Debug)]
pub struct EditorCounts {
    builds: AtomicU64,
    closed: AtomicU64,
    /// Frames drawn (`NiceEguiApp::ui` calls).
    frames: AtomicU64,
    /// Frames that followed a frame which asked for a repaint.
    live_frames: AtomicU64,
    /// Microseconds from each such frame's predecessor to it, summed.
    live_us: AtomicU64,
    /// When the last frame was drawn, in microseconds since `origin` plus one (0: no frame since the last open).
    last_frame_us: AtomicU64,
    /// The last frame asked for a repaint.
    last_asked: AtomicBool,
    origin: Instant,
}

impl Default for EditorCounts {
    fn default() -> Self {
        Self {
            builds: AtomicU64::new(0),
            closed: AtomicU64::new(0),
            frames: AtomicU64::new(0),
            live_frames: AtomicU64::new(0),
            live_us: AtomicU64::new(0),
            last_frame_us: AtomicU64::new(0),
            last_asked: AtomicBool::new(false),
            origin: Instant::now(),
        }
    }
}

/// What the counters hold.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub builds: u64,
    pub editor_closed: u64,
    pub frames: u64,
    pub live_frames: u64,
    pub live_us: u64,
}

impl Snapshot {
    /// Frames per second while live data changed, if any frame was live.
    #[must_use]
    pub fn live_fps(&self) -> Option<f64> {
        (self.live_us > 0).then(|| self.live_frames as f64 * 1e6 / self.live_us as f64)
    }
}

impl EditorCounts {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// `NiceEguiApp::build` ran: the window opened. A frame of the last window does not make the first one of this
    /// window live.
    pub fn built(&self) {
        self.builds.fetch_add(1, Ordering::Relaxed);
        self.last_frame_us.store(0, Ordering::Relaxed);
        self.last_asked.store(false, Ordering::Relaxed);
    }

    /// `NiceEguiApp::editor_closed` ran: the window closed.
    pub fn closed(&self) {
        self.closed.fetch_add(1, Ordering::Relaxed);
    }

    /// One frame drawn; `asked`: it asked for the next one (visible live data changed).
    pub fn frame(&self, asked: bool) {
        let now = self.now_us();
        self.frame_at(now, asked);
    }

    fn frame_at(&self, now_us: u64, asked: bool) {
        self.frames.fetch_add(1, Ordering::Relaxed);
        let last = self.last_frame_us.swap(now_us + 1, Ordering::Relaxed);
        let was_asked = self.last_asked.swap(asked, Ordering::Relaxed);
        if was_asked && last > 0 {
            self.live_frames.fetch_add(1, Ordering::Relaxed);
            self.live_us
                .fetch_add((now_us + 1).saturating_sub(last), Ordering::Relaxed);
        }
    }

    fn now_us(&self) -> u64 {
        u64::try_from(self.origin.elapsed().as_micros()).unwrap_or(u64::MAX - 1)
    }

    /// What the counters hold now.
    #[must_use]
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            builds: self.builds.load(Ordering::Relaxed),
            editor_closed: self.closed.load(Ordering::Relaxed),
            frames: self.frames.load(Ordering::Relaxed),
            live_frames: self.live_frames.load(Ordering::Relaxed),
            live_us: self.live_us.load(Ordering::Relaxed),
        }
    }

    /// The report line.
    #[must_use]
    pub fn to_json(&self) -> Value {
        let s = self.snapshot();
        json!({
            "schema": "enth.plugin-editor-counts/1",
            "builds": s.builds,
            "editor_closed": s.editor_closed,
            "frames": s.frames,
            "live_frames": s.live_frames,
            "live_ms": s.live_us / 1000,
            "live_fps": s.live_fps().map(|f| (f * 10.0).round() / 10.0),
        })
    }

    /// Append the report to the file `$ENTH_EDITOR_COUNTS` names, if any. An I/O error is ignored (a measurement aid
    /// must not break the plugin).
    pub fn write_report(&self) {
        let Some(path) = std::env::var_os(ENV) else {
            return;
        };
        let line = format!("{}\n", self.to_json());
        let _ = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .and_then(|mut f| f.write_all(line.as_bytes()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_and_closes_are_counted_apart() {
        let counts = EditorCounts::new();
        for _ in 0..3 {
            counts.built();
        }
        counts.closed();
        let s = counts.snapshot();
        assert_eq!((s.builds, s.editor_closed), (3, 1));
        assert_eq!(
            counts.to_json(),
            json!({"schema": "enth.plugin-editor-counts/1", "builds": 3, "editor_closed": 1, "frames": 0,
                   "live_frames": 0, "live_ms": 0, "live_fps": null})
        );
    }

    #[test]
    fn frames_after_a_repaint_request_count_as_live_and_idle_time_does_not() {
        let counts = EditorCounts::new();
        counts.built();
        // An input frame that asks for nothing; a note: 31 frames 33.3 ms apart, each asking for the next (baseview's
        // 30 fps); the frame that sees nothing change any more and asks for nothing; 10 s idle; an input frame.
        counts.frame_at(1_000, false);
        let mut t = 500_000;
        counts.frame_at(t, true);
        for _ in 0..30 {
            t += 33_333;
            counts.frame_at(t, true);
        }
        t += 33_333;
        counts.frame_at(t, false);
        counts.frame_at(t + 10_000_000, false);
        let s = counts.snapshot();
        assert_eq!(s.frames, 34);
        // Every frame that followed a request: the 30 after the first animated one, and the one that went quiet.
        assert_eq!(s.live_frames, 31);
        assert_eq!(s.live_us, 31 * 33_333);
        let fps = s.live_fps().unwrap();
        assert!(
            (fps - 30.0).abs() < 0.01,
            "idle time does not dilute it: {fps}"
        );
        // A new window starts afresh: its first frame is not live, whatever the last window asked.
        counts.built();
        counts.frame_at(t + 20_000_000, true);
        assert_eq!(counts.snapshot().live_frames, 31);
        counts.frame_at(t + 20_016_667, true);
        assert_eq!(counts.snapshot().live_frames, 32);
    }
}
