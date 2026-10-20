//! Editor lifecycle counters (lane INT), behind the `editor-counts` cargo feature: without it this module is not
//! compiled and the editor counts nothing.
//!
//! The leak gate `cargo xtask leak-test` opens and closes the editor 200 times in `enth-test-host editor-cycles` and
//! must see the vendored nice-plug-egui call `NiceEguiApp::editor_closed` once per `build()` (the fork's patch 2;
//! stock nice-plug-egui 0.6.0 never calls it). The host cannot see that call, so the plugin counts both: `build()` and
//! `editor_closed()` each add one (relaxed atomics, on baseview's GUI thread). When the last reference to the
//! plugin's [`Shared`](crate::Shared) goes (the plugin and every editor are gone, so no GUI thread can still be
//! counting) and the environment variable `ENTH_EDITOR_COUNTS` names a file, one JSON line
//! (`enth.plugin-editor-counts/1`) is appended to it (docs/plugin.md, "Leak gates").

use std::io::Write;
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::{Value, json};

/// The environment variable naming the file the report is appended to.
pub const ENV: &str = "ENTH_EDITOR_COUNTS";

/// How often the editor was built and closed.
#[derive(Debug, Default)]
pub struct EditorCounts {
    builds: AtomicU64,
    closed: AtomicU64,
}

impl EditorCounts {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// `NiceEguiApp::build` ran: the window opened.
    pub fn built(&self) {
        self.builds.fetch_add(1, Ordering::Relaxed);
    }

    /// `NiceEguiApp::editor_closed` ran: the window closed.
    pub fn closed(&self) {
        self.closed.fetch_add(1, Ordering::Relaxed);
    }

    /// (builds, `editor_closed` calls).
    #[must_use]
    pub fn snapshot(&self) -> (u64, u64) {
        (
            self.builds.load(Ordering::Relaxed),
            self.closed.load(Ordering::Relaxed),
        )
    }

    /// The report line.
    #[must_use]
    pub fn to_json(&self) -> Value {
        let (builds, closed) = self.snapshot();
        json!({
            "schema": "enth.plugin-editor-counts/1",
            "builds": builds,
            "editor_closed": closed,
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
        assert_eq!(counts.snapshot(), (3, 1));
        assert_eq!(
            counts.to_json(),
            json!({"schema": "enth.plugin-editor-counts/1", "builds": 3, "editor_closed": 1})
        );
    }
}
