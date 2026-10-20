//! The crash file (lane PBC of R1/S1): a panic hook that writes what a panic knew to a local folder, so the next
//! launch can offer it ("enth stopped unexpectedly: here is what it knew"). Local only: **no network, no signal
//! handlers, no uploads**. The plugin never sends it anywhere; the user decides.
//!
//! # What it does
//!
//! [`install`] runs once when the library loads, from [`Plugin::setup_logger`](nice_plug::prelude::Plugin::setup_logger)
//! (the earliest point nice-plug gives a plugin; it returns `Some(false)` so nice-plug neither installs a subscriber
//! nor its own `log_panics`, which would replace this hook). It takes the current panic hook, installs its own, and
//! **chains to the one it took**: whatever the host or the default hook printed before is still printed.
//!
//! On a panic the hook, once and never recursively:
//!
//! 1. captures a backtrace (`Backtrace::force_capture`; `RUST_BACKTRACE` is not needed, and when symbols were
//!    stripped the frames are addresses);
//! 2. decides whether the panic is enth's: when the backtrace has symbol names and none mentions `enth`, the panic
//!    belongs to another plugin in the same host process and is left alone. With no symbols, or in doubt, it writes;
//! 3. writes `enth-crash-<UTC date>-<UTC time>-<counter>-<process id>.txt` into the crash folder: the time, version, platform,
//!    thread, location, message and backtrace, the user's home folder replaced by `~`. Nothing else: no patch, no
//!    host, no names of projects, no environment.
//!
//! It is bounded: at most [`MAX_FILES_PER_PROCESS`] files per process (a panic loop cannot fill a disk), nothing is
//! written once the folder holds [`MAX_FILES_IN_DIR`] crash files, and a report is cut at [`MAX_REPORT_BYTES`]. It
//! never deletes anything.
//!
//! # Real time
//!
//! The non-panic path costs nothing: installing is a one-off at library load (it allocates there, on the host's
//! main thread) and no code of this module runs again until a panic. In a panic, which may be on the audio thread,
//! the hook allocates (a backtrace, strings, a file) inside `nice_plug::util::permit_alloc`, because a debug build's
//! `assert_process_allocs` would abort on the first allocation otherwise. A panic on the audio thread is already the
//! end of that block's real-time guarantee.
//!
//! # The folder
//!
//! `ENTH_CRASH_DIR` when set, else `<Documents>/enth/Crashes` (the same Documents rules as the presets,
//! [`documents_dir`](crate::presets::documents_dir)). It is created by the hook, when there is something to write.
//! [`pending`] lists the files the editor has not offered yet, [`read_report`] reads one, and [`mark_seen`] renames
//! it to `….seen.txt` so it is offered once.

use std::backtrace::Backtrace;
use std::cell::Cell;
use std::fs;
use std::io::{self, Write as _};
use std::panic::{self, AssertUnwindSafe, PanicHookInfo};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::presets::{Environment, Os, SystemEnv, documents_dir, home_dir};

/// The most crash files one process writes.
pub const MAX_FILES_PER_PROCESS: u32 = 3;
/// No crash file is written once the folder holds this many.
pub const MAX_FILES_IN_DIR: usize = 50;
/// A report is cut here (a deep backtrace is a few kilobytes).
pub const MAX_REPORT_BYTES: usize = 64 * 1024;

const PREFIX: &str = "enth-crash-";
const SUFFIX: &str = ".txt";
const SEEN_SUFFIX: &str = ".seen.txt";

/// Where the hook writes and what it hides.
struct Config {
    dir: PathBuf,
    /// The home folder, to be replaced by `~` in the report.
    home: Option<String>,
}

static CONFIG: OnceLock<Config> = OnceLock::new();
static WRITTEN: AtomicU32 = AtomicU32::new(0);

thread_local! {
    /// This thread is inside the hook: a panic in the hook itself is left to the chained hook.
    static IN_HOOK: Cell<bool> = const { Cell::new(false) };
}

/// The crash folder for an environment: `ENTH_CRASH_DIR` if set and not empty, else `<Documents>/enth/Crashes`.
pub fn crash_dir(env: &dyn Environment, os: Os) -> Option<PathBuf> {
    env.var("ENTH_CRASH_DIR")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| documents_dir(env, os).map(|d| d.join("enth").join("Crashes")))
}

/// Install the hook for the folder the environment names. Once per process; later calls do nothing. Returns whether
/// this call installed it (`false`: already installed, or no folder could be found).
pub fn install() -> bool {
    let os = Os::current();
    let env = SystemEnv;
    let home = home_dir(&env, os).map(|h| h.to_string_lossy().into_owned());
    install_at(crash_dir(&env, os), home)
}

/// Install the hook writing into `dir` (`None`: nothing to install), hiding `home` in reports.
pub fn install_at(dir: Option<PathBuf>, home: Option<String>) -> bool {
    let Some(dir) = dir else {
        return false;
    };
    if CONFIG.set(Config { dir, home }).is_err() {
        return false;
    }
    let previous = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        on_panic(info);
        previous(info);
    }));
    true
}

/// Resets [`IN_HOOK`] when the hook ends, however it ends.
struct Reentry;

impl Drop for Reentry {
    fn drop(&mut self) {
        IN_HOOK.with(|c| c.set(false));
    }
}

fn on_panic(info: &PanicHookInfo<'_>) {
    if IN_HOOK.with(|c| c.replace(true)) {
        return;
    }
    let _reset = Reentry;
    let Some(config) = CONFIG.get() else {
        return;
    };
    // Allocation is fine here (see the module docs); a panic inside is caught, never escapes the hook.
    nice_plug::util::permit_alloc(|| {
        let _ = panic::catch_unwind(AssertUnwindSafe(|| {
            if WRITTEN.load(Ordering::Relaxed) >= MAX_FILES_PER_PROCESS {
                return;
            }
            let backtrace = Backtrace::force_capture().to_string();
            let location = info
                .location()
                .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()));
            if !concerns_enth(location.as_deref(), &backtrace) {
                return;
            }
            let thread = std::thread::current();
            let report = Report {
                version: env!("CARGO_PKG_VERSION"),
                debug: cfg!(debug_assertions),
                unix_secs: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_or(0, |d| d.as_secs()),
                thread_name: thread.name().unwrap_or("unnamed"),
                thread_id: format!("{:?}", thread.id()),
                location: location.as_deref(),
                message: payload_text(info),
                backtrace: &backtrace,
            };
            if write_report(&config.dir, &report, config.home.as_deref()).is_ok() {
                WRITTEN.fetch_add(1, Ordering::Relaxed);
            }
        }));
    });
}

/// The panic's message, as `&str` or `String` payloads carry it.
fn payload_text(info: &PanicHookInfo<'_>) -> String {
    let payload = info.payload();
    payload
        .downcast_ref::<&'static str>()
        .map(|s| (*s).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "(a panic with a payload that is not text)".to_owned())
}

/// Whether the panic is enth's: its location, or a symbolised frame of its backtrace, names `enth`. A backtrace with
/// no symbols at all cannot say, so it counts (better a report too many than none).
fn concerns_enth(location: Option<&str>, backtrace: &str) -> bool {
    if location.is_some_and(|l| l.to_lowercase().contains("enth")) {
        return true;
    }
    let mut symbolised = false;
    for line in backtrace.lines() {
        // Frame lines read `  12: symbol::name`; unresolved ones `  12:   0x7f12...`.
        let Some((index, rest)) = line.trim_start().split_once(':') else {
            continue;
        };
        if index.is_empty() || !index.bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        let rest = rest.trim_start();
        if rest.starts_with("0x") {
            continue;
        }
        symbolised = true;
        if rest.to_lowercase().contains("enth") {
            return true;
        }
    }
    !symbolised
}

/// What a report says.
struct Report<'a> {
    version: &'a str,
    debug: bool,
    unix_secs: u64,
    thread_name: &'a str,
    thread_id: String,
    location: Option<&'a str>,
    message: String,
    backtrace: &'a str,
}

/// The report's text, with `home` replaced by `~`, cut at [`MAX_REPORT_BYTES`].
fn render(report: &Report<'_>, home: Option<&str>) -> String {
    let (y, mo, d, h, mi, s) = civil(report.unix_secs);
    let mut text = format!(
        "enth crash report\n\
         \n\
         This file was written by enth when it hit a bug. It stays on this computer; enth never sends it anywhere.\n\
         It holds no patch, project or personal data. If you report the bug, attach this file.\n\
         \n\
         version: {version} ({profile})\n\
         time: {y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z (unix {secs})\n\
         platform: {os} {arch}\n\
         thread: {thread} ({id})\n\
         location: {location}\n\
         message: {message}\n\
         \n\
         backtrace:\n\
         {backtrace}\n",
        version = report.version,
        profile = if report.debug { "debug" } else { "release" },
        secs = report.unix_secs,
        os = std::env::consts::OS,
        arch = std::env::consts::ARCH,
        thread = report.thread_name,
        id = report.thread_id,
        location = report.location.unwrap_or("(unknown)"),
        message = report.message,
        backtrace = report.backtrace.trim_end(),
    );
    if let Some(home) = home {
        text = hide_home(&text, home);
    }
    if text.len() > MAX_REPORT_BYTES {
        let mut cut = MAX_REPORT_BYTES;
        while !text.is_char_boundary(cut) {
            cut -= 1;
        }
        text.truncate(cut);
        text.push_str("\n(cut)\n");
    }
    text
}

/// `text` with the home folder, written either way round, replaced by `~`. A home folder of one character (`/`)
/// is left alone: replacing it would garble every path.
fn hide_home(text: &str, home: &str) -> String {
    let home = home.trim_end_matches(['/', '\\']);
    if home.len() < 2 {
        return text.to_owned();
    }
    let mut out = text.replace(home, "~");
    for variant in [home.replace('\\', "/"), home.replace('/', "\\")] {
        if variant != home {
            out = out.replace(&variant, "~");
        }
    }
    out
}

/// Write `report` as a new file `enth-crash-<UTC date>-<UTC time>-<counter>-<process id>.txt` in `dir` (created if
/// needed). Refuses when the folder is full of reports.
fn write_report(dir: &Path, report: &Report<'_>, home: Option<&str>) -> io::Result<PathBuf> {
    fs::create_dir_all(dir)?;
    if count_files(dir) >= MAX_FILES_IN_DIR {
        return Err(io::Error::other("the crash folder is full"));
    }
    let (y, mo, d, h, mi, s) = civil(report.unix_secs);
    let text = render(report, home);
    // `create_new`: never replace a report. A counter before the process id keeps the names in time order when two
    // reports share a second.
    for n in 0..100u32 {
        let name = format!(
            "{PREFIX}{y:04}{mo:02}{d:02}-{h:02}{mi:02}{s:02}-{n:02}-{}{SUFFIX}",
            std::process::id()
        );
        let path = dir.join(name);
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut file) => {
                file.write_all(text.as_bytes())?;
                file.sync_all()?;
                return Ok(path);
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::other("no free file name"))
}

fn count_files(dir: &Path) -> usize {
    fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with(PREFIX))
        .count()
}

/// The crash files in `dir` that have not been marked seen, oldest first (the names start with the UTC time).
pub fn pending(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            name.starts_with(PREFIX) && name.ends_with(SUFFIX) && !name.ends_with(SEEN_SUFFIX)
        })
        .map(|e| e.path())
        .collect();
    files.sort();
    files
}

/// The text of a crash file (at most [`MAX_REPORT_BYTES`] bytes, lossily decoded).
pub fn read_report(path: &Path) -> io::Result<String> {
    use std::io::Read as _;
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(MAX_REPORT_BYTES as u64)
        .read_to_end(&mut bytes)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// Mark a crash file as offered: renamed to `….seen.txt`, kept (the user may still want to send it). Returns the
/// new path.
pub fn mark_seen(path: &Path) -> io::Result<PathBuf> {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "not a crash file"))?;
    let stem = name
        .strip_suffix(SUFFIX)
        .filter(|_| name.starts_with(PREFIX) && !name.ends_with(SEEN_SUFFIX))
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "not a pending crash file"))?;
    let seen = path.with_file_name(format!("{stem}{SEEN_SUFFIX}"));
    fs::rename(path, &seen)?;
    Ok(seen)
}

/// Seconds since the Unix epoch as a UTC calendar time (year, month, day, hour, minute, second): Howard Hinnant's
/// `civil_from_days`, so no date crate is needed.
fn civil(unix_secs: u64) -> (u64, u64, u64, u64, u64, u64) {
    let days = unix_secs / 86_400;
    let rem = unix_secs % 86_400;
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z % 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + u64::from(month <= 2);
    (year, month, day, rem / 3_600, rem % 3_600 / 60, rem % 60)
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use super::*;

    #[test]
    fn the_calendar_is_right() {
        assert_eq!(civil(0), (1970, 1, 1, 0, 0, 0));
        assert_eq!(civil(951_782_400), (2000, 2, 29, 0, 0, 0), "a leap day");
        assert_eq!(civil(1_000_000_000), (2001, 9, 9, 1, 46, 40));
        assert_eq!(civil(1_700_000_000), (2023, 11, 14, 22, 13, 20));
        assert_eq!(civil(4_102_444_799), (2099, 12, 31, 23, 59, 59));
        assert_eq!(
            civil(4_102_444_800),
            (2100, 1, 1, 0, 0, 0),
            "2100 is not a leap year"
        );
        assert_eq!(civil(1_709_251_199), (2024, 2, 29, 23, 59, 59));
        assert_eq!(civil(1_709_251_200), (2024, 3, 1, 0, 0, 0));
    }

    fn sample<'a>(backtrace: &'a str, location: Option<&'a str>) -> Report<'a> {
        Report {
            version: "1.2.3",
            debug: false,
            unix_secs: 1_700_000_000,
            thread_name: "audio",
            thread_id: "ThreadId(7)".to_owned(),
            location,
            message: "index out of bounds".to_owned(),
            backtrace,
        }
    }

    #[test]
    fn a_report_says_when_where_and_what() {
        let text = render(
            &sample(
                "   0: enth_plugin::process\n",
                Some("crates/enth-plugin/src/process.rs:10:5"),
            ),
            None,
        );
        for line in [
            "version: 1.2.3 (release)",
            "time: 2023-11-14T22:13:20Z (unix 1700000000)",
            "thread: audio (ThreadId(7))",
            "location: crates/enth-plugin/src/process.rs:10:5",
            "message: index out of bounds",
            "backtrace:\n   0: enth_plugin::process",
        ] {
            assert!(text.contains(line), "{line:?} missing from\n{text}");
        }
        assert!(text.contains(&format!("platform: {} ", std::env::consts::OS)));
        assert!(text.contains("never sends it anywhere"));
    }

    #[test]
    fn the_home_folder_never_reaches_the_file() {
        let bt = "   3: foo\n             at /home/ana/.cargo/registry/src/x/lib.rs:1\n   4: bar\n             at C:\\Users\\ana\\proj\\a.rs:2\n";
        let mut report = sample(bt, Some("/home/ana/src/a.rs:1:1"));
        report.message = "could not open /home/ana/Documents/enth/Presets/x".to_owned();
        let text = render(&report, Some("/home/ana"));
        assert!(!text.contains("/home/ana"), "{text}");
        assert!(text.contains("~/.cargo/registry"));
        assert!(text.contains("message: could not open ~/Documents/enth/Presets/x"));
        // A Windows home is hidden in both spellings.
        let text = render(&report, Some("C:\\Users\\ana"));
        assert!(
            !text.contains("Users\\ana") && !text.contains("Users/ana"),
            "{text}"
        );
        // A trailing separator, and a one-character home that would garble everything, are handled.
        assert_eq!(hide_home("/home/ana/x", "/home/ana/"), "~/x");
        assert_eq!(hide_home("/home/ana/x", "/"), "/home/ana/x");
        assert_eq!(hide_home("/home/ana/x", ""), "/home/ana/x");
    }

    #[test]
    fn a_huge_report_is_cut_on_a_character() {
        let big = "é".repeat(MAX_REPORT_BYTES);
        let text = render(&sample(&big, None), None);
        assert!(text.len() <= MAX_REPORT_BYTES + 8, "{}", text.len());
        assert!(text.ends_with("(cut)\n"));
    }

    #[test]
    fn only_enth_panics_are_written() {
        let ours =
            "   0: std::panicking::begin_panic\n   1: enth_session::presets::load\n   2: main\n";
        let theirs = "   0: std::panicking::begin_panic\n   1: other_synth::process\n   2: main\n";
        let stripped = "   0:     0x7f0012345678\n   1:     0x7f0012345abc\n";
        assert!(concerns_enth(None, ours));
        assert!(!concerns_enth(None, theirs));
        assert!(concerns_enth(None, stripped), "no symbols: in doubt, write");
        assert!(concerns_enth(None, ""), "nothing at all: in doubt, write");
        // The location alone is enough, in a path or a crate name.
        assert!(concerns_enth(
            Some("crates/enth-engine/src/lib.rs:1:1"),
            theirs
        ));
        assert!(concerns_enth(Some("vendor/Enth/x.rs:1:1"), theirs));
        assert!(!concerns_enth(
            Some("/cargo/registry/other/lib.rs:1:1"),
            theirs
        ));
        // `at path` lines of a frame do not count as frames.
        let at_only = "   0: other::f\n             at /home/x/enth/other.rs:1\n";
        assert!(!concerns_enth(None, at_only));
    }

    /// A scratch folder, removed when dropped.
    struct Temp(PathBuf);

    impl Temp {
        fn new() -> Temp {
            static N: AtomicU32 = AtomicU32::new(0);
            Temp(std::env::temp_dir().join(format!(
                "enth-pbc-crash-{}-{}",
                std::process::id(),
                N.fetch_add(1, Ordering::Relaxed)
            )))
        }
    }

    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn reports_are_files_that_are_listed_read_and_marked_seen() {
        let temp = Temp::new();
        let dir = temp.0.join("Crashes");
        assert_eq!(
            pending(&dir),
            Vec::<PathBuf>::new(),
            "no folder is no reports"
        );
        let bt = "   0: enth_plugin::x\n";
        let mut report = sample(bt, None);
        let first = write_report(&dir, &report, None).unwrap();
        report.unix_secs += 1;
        let second = write_report(&dir, &report, None).unwrap();
        // The same second twice does not replace the first.
        let third = write_report(&dir, &report, None).unwrap();
        assert_ne!(second, third);
        assert!(
            first
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with("enth-crash-20231114-221320-00-")
        );
        fs::write(dir.join("notes.txt"), "not a crash file").unwrap();
        fs::write(dir.join("enth-crash-x.seen.txt"), "old").unwrap();
        assert_eq!(
            pending(&dir),
            [first.clone(), second.clone(), third.clone()]
        );
        assert!(read_report(&first).unwrap().contains("enth crash report"));
        let seen = mark_seen(&first).unwrap();
        assert_eq!(
            seen.file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .rsplit_once(".seen.txt")
                .map(|p| p.1),
            Some("")
        );
        assert!(seen.exists() && !first.exists(), "renamed, not deleted");
        assert_eq!(pending(&dir), [second, third]);
        assert!(mark_seen(&seen).is_err(), "already seen");
        assert!(
            mark_seen(&dir.join("notes.txt")).is_err(),
            "not a crash file"
        );
    }

    #[test]
    fn a_full_folder_takes_no_more_reports() {
        let temp = Temp::new();
        fs::create_dir_all(&temp.0).unwrap();
        for i in 0..MAX_FILES_IN_DIR {
            fs::write(temp.0.join(format!("{PREFIX}{i:04}{SUFFIX}")), "x").unwrap();
        }
        let report = sample("", None);
        assert!(write_report(&temp.0, &report, None).is_err());
        assert_eq!(count_files(&temp.0), MAX_FILES_IN_DIR);
    }

    #[test]
    fn a_report_is_never_read_past_its_cap() {
        let temp = Temp::new();
        fs::create_dir_all(&temp.0).unwrap();
        let path = temp.0.join("enth-crash-big.txt");
        fs::write(&path, "a".repeat(MAX_REPORT_BYTES * 2)).unwrap();
        assert_eq!(read_report(&path).unwrap().len(), MAX_REPORT_BYTES);
    }

    struct Fake(Vec<(&'static str, &'static str)>);

    impl Environment for Fake {
        fn var(&self, name: &str) -> Option<OsString> {
            self.0
                .iter()
                .find(|(n, _)| *n == name)
                .map(|(_, v)| OsString::from(v))
        }

        fn read_text(&self, _: &Path) -> Option<String> {
            None
        }
    }

    #[test]
    fn the_crash_folder_follows_the_environment() {
        let norm = |p: Option<PathBuf>| p.map(|p| p.to_string_lossy().replace('\\', "/"));
        let env = Fake(vec![("ENTH_CRASH_DIR", "/tmp/c"), ("HOME", "/home/ana")]);
        assert_eq!(norm(crash_dir(&env, Os::Linux)), Some("/tmp/c".to_owned()));
        let env = Fake(vec![("ENTH_CRASH_DIR", ""), ("HOME", "/home/ana")]);
        assert_eq!(
            norm(crash_dir(&env, Os::Linux)),
            Some("/home/ana/Documents/enth/Crashes".to_owned())
        );
        let env = Fake(vec![("USERPROFILE", "C:\\Users\\ana")]);
        assert_eq!(
            norm(crash_dir(&env, Os::Windows)),
            Some("C:/Users/ana/Documents/enth/Crashes".to_owned())
        );
        assert_eq!(crash_dir(&Fake(vec![]), Os::Linux), None);
        assert!(!install_at(None, None), "no folder, no hook");
    }
}
