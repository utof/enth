//! Helpers for the in-process CLI tests: a scratch directory per test, running commands, the typed round trip,
//! masking audio-dependent numbers, and snapshots with the scratch path filtered out.
#![allow(
    dead_code,
    clippy::disallowed_methods,
    reason = "shared by several test binaries, each using a subset; tests do file I/O"
)]

use std::path::{Path, PathBuf};

use enth_cli::{Env, Output};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

/// A fresh scratch directory for one test.
pub struct Scratch {
    pub dir: PathBuf,
}

impl Scratch {
    pub fn new(name: &str) -> Self {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join("enth-cli-tests")
            .join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Scratch { dir }
    }

    /// A path inside the directory, as a string argument.
    pub fn path(&self, file: &str) -> String {
        self.dir.join(file).to_string_lossy().into_owned()
    }

    /// Write a file.
    pub fn write(&self, file: &str, text: &str) -> String {
        let p = self.path(file);
        std::fs::write(&p, text).unwrap();
        p
    }

    /// `enth new --template <t> --out <file>`, asserting success.
    pub fn new_patch(&self, template: &str, file: &str) -> String {
        let p = self.path(file);
        let o = run(&["new", "--template", template, "--out", &p]);
        assert_eq!(o.code, 0, "{}", o.stdout);
        p
    }

    /// Normalise path separators in output text (Windows paths print with escaped backslashes in JSON), so the
    /// insta filter from [`Scratch::settings`] can replace the scratch directory.
    pub fn clean(&self, text: &str) -> String {
        if cfg!(windows) {
            let dir = self.dir.to_string_lossy().into_owned();
            let json_dir = serde_json::to_string(&dir).unwrap();
            text.replace(json_dir.trim_matches('"'), &dir.replace('\\', "/"))
                .replace(&dir, &dir.replace('\\', "/"))
                .replace("\\\\", "/")
        } else {
            text.to_owned()
        }
    }

    /// Snapshot settings: an insta filter that replaces the scratch directory with `[TMP]`.
    pub fn settings(&self) -> insta::Settings {
        let dir = self.dir.to_string_lossy().replace('\\', "/");
        let mut s = insta::Settings::clone_current();
        s.add_filter(&regex_escape(&dir), "[TMP]");
        s.set_prepend_module_to_snapshot(false);
        s
    }
}

fn regex_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if "\\.+*?()|[]{}^$#&-~".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// Run one command in process (stdout is not a terminal: JSON).
pub fn run(args: &[&str]) -> Output {
    enth_cli::run(args.iter().copied(), "")
}

/// Run one command with stdin.
pub fn run_stdin(args: &[&str], stdin: &str) -> Output {
    enth_cli::run(args.iter().copied(), stdin)
}

/// Run one command as if stdout were a terminal (tables).
pub fn run_tty(args: &[&str]) -> Output {
    enth_cli::run_with(args.iter().copied(), "", Env { stdout_tty: true })
}

/// The stdout of a JSON call, parsed.
pub fn json(o: &Output) -> Value {
    serde_json::from_str(&o.stdout).unwrap_or_else(|e| panic!("not JSON ({e}): {}", o.stdout))
}

/// Deserialize stdout into its type (`deny_unknown_fields`) and check that serialising it again gives the same
/// JSON: the code and the published schema agree.
pub fn round_trip<T: DeserializeOwned + Serialize>(o: &Output) -> T {
    let v = json(o);
    let t: T = serde_json::from_value(v.clone())
        .unwrap_or_else(|e| panic!("does not match its type: {e}\n{}", o.stdout));
    assert_eq!(
        enth_cli::out::to_value(&t),
        v,
        "round trip changed the output"
    );
    t
}

/// Keys whose numbers do not depend on the engine's audio (kept in snapshots).
const STABLE_NUMBERS: &[&str] = &[
    "sample_rate",
    "channels",
    "seed",
    "dur_s",
    "on_s",
    "off_s",
    "vel",
    "passed",
    "failed",
    "exit",
    "voices",
    "seconds",
    "bytes",
    "depth",
];

/// Keys whose booleans depend on measurement noise or the engine (masked in snapshots).
const NOISY_BOOLS: &[&str] = &[
    "thin",
    "unstable",
    "estimated",
    "lower_bound",
    "beat_corrected",
    "release_extrapolated",
    "duration_truncated",
    "clipping",
    "loudness.clipping",
];

/// Replace audio-dependent numbers with `"<num>"` and noisy booleans with `"<bool>"`, so snapshots pin the shape
/// and the deliberate values (performance, pass/fail of robust items, units, texts) but not the DSP. A fix's `ops`
/// and the number of renders it took are the solver's measurements of the current DSP, so they are masked too
/// (`fix.ops` becomes `"<ops>"`); the ops anywhere else are patch values and stay.
pub fn mask(v: &Value) -> Value {
    fn go(parent: &str, key: &str, v: &Value) -> Value {
        match v {
            _ if key == "ops" && parent == "fix" => Value::from("<ops>"),
            // Op lists hold patch values, not measurements.
            _ if key == "ops" || key == "inverse" => v.clone(),
            Value::Number(_) if !STABLE_NUMBERS.contains(&key) => Value::from("<num>"),
            Value::Bool(_) if NOISY_BOOLS.contains(&key) => Value::from("<bool>"),
            Value::Array(a) => Value::Array(a.iter().map(|x| go(parent, key, x)).collect()),
            Value::Object(m) => {
                Value::Object(m.iter().map(|(k, x)| (k.clone(), go(key, k, x))).collect())
            }
            other => other.clone(),
        }
    }
    go("", "", v)
}

/// Pretty JSON for a snapshot.
pub fn pretty(v: &Value) -> String {
    serde_json::to_string_pretty(v).unwrap()
}

/// The text a snapshot records for a call: exit code, stdout (pretty JSON when it is JSON) and stderr.
pub fn record(scratch: &Scratch, o: &Output, masked: bool) -> String {
    let body = match serde_json::from_str::<Value>(&o.stdout) {
        Ok(v) if masked => pretty(&mask(&v)),
        Ok(v) => pretty(&v),
        Err(_) => o.stdout.clone(),
    };
    scratch.clean(&format!(
        "exit: {}\n--- stdout\n{}\n--- stderr\n{}",
        o.code, body, o.stderr
    ))
}
