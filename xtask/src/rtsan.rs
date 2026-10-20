//! `cargo xtask rtsan`: the plugin's `process()` under `RealtimeSanitizer`, in the test host (R1/S1 lane INT; the
//! "`RTSan`-built test host" of docs/phase-2.md's open checks).
//!
//! The nightly workflow runs the engine's and render's own tests under `RTSan`; this runs the whole plugin the way a
//! host does. Both binaries are built with a nightly toolchain and `-Zsanitizer=realtime --cfg enth_rtsan`:
//!
//! - `enth-plugin` (release, with `ab-toggle` so the A/B scenario can run; the feature does nothing unless the test
//!   host sets its variable): `--cfg enth_rtsan` marks `Plugin::process` (and the engine's `process`)
//!   `#[sanitize(realtime = "nonblocking")]`, so everything that runs inside it is a real-time context;
//! - `enth-test-host`, which links `RTSan`'s runtime and exports it (`-Wl,--export-dynamic`) so the plugin's
//!   `__rtsan_realtime_enter`/`exit` calls resolve when it is loaded. Any allocation, lock or blocking call inside
//!   `process()`, the host's own callbacks included, aborts the run (exit 43) with the call's stack.
//!
//! The runtime ships in the nightly `rust-std` for `x86_64-unknown-linux-gnu`: no system package and no download
//! beyond the toolchain. Linux x86-64 only. The scenarios:
//!
//! | Check | Test host command | Expect |
//! | --- | --- | --- |
//! | `control` | `render` with `ENTH_RTSAN_CONTROL=alloc` (the plugin allocates in `process()`, `--cfg enth_rtsan` only) | caught: proves the sanitizer is live |
//! | `render-chord` | `render`, Init, a four-note chord, 64-frame blocks | clean |
//! | `render-block-1`, `render-block-4096` | `render`, the REAPER round-trip patch (3 generators, effects, routes, a slot) | clean |
//! | `swaps-crossfade`, `swaps-instant` | `process-stats`, a state load every 250 ms between Init and the rich patch (plan swaps, crossfades, the background pump, a voice-count change) | clean |
//! | `flood` | `process-stats --flood 64`: parameter floods | clean |
//! | `ab` | `process-stats --ab 250`: a Recreate round's A/B flipping every 250 ms (the target loop, the phrase player) | clean |
//! | `bursts` | `bursts`: 16 to 4096 events in one block, every voice stolen | clean |
//! | `note-end` | `note-end`: note ends at several block sizes | clean |
//! | `no-thread-check` | `swaps-instant` in a host without CLAP's thread-check extension | information |
//!
//! Every scenario but the last runs with the test host offering thread-check (`ENTH_TEST_HOST_THREAD_CHECK=1`), as
//! REAPER and Bitwig do. Without it, nice-plug's `is_main_thread` falls back to `std::thread::current()` in the
//! plugin's own std when a voice-count change schedules a GUI task from `process()`, which allocates the first time
//! on that thread (vendored nice-plug's FIXME, under `permit_alloc`); `RTSan` sees it, so the last row reports it.
//!
//! Reports go to `target/rtsan/`: one `<check>.json` and `<check>.err` per scenario; a failure prints the
//! sanitizer's report with the plugin's and the host's frames named from their symbol tables. The exit status is the
//! verdict. About 5 minutes to build (cold) and 1–2 minutes to run.

use std::path::Path;
use std::process::Command;

const USAGE: &str =
    "usage: cargo xtask rtsan [--toolchain nightly] [--secs 10] [--no-build] [--only CHECK]...";

/// The target triple: the sanitizer runtime ships for it.
const TRIPLE: &str = "x86_64-unknown-linux-gnu";

/// The flags for every crate of the build (`--target` keeps them off build scripts and proc macros).
const RUSTFLAGS: &str = "-Zsanitizer=realtime --cfg enth_rtsan -C link-arg=-Wl,--export-dynamic";

/// What a scenario should give.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Expect {
    /// Exit 0, no sanitizer report.
    Clean,
    /// A sanitizer report (the positive control).
    Caught,
    /// Printed, never a failure.
    Info,
}

struct Check {
    name: &'static str,
    args: Vec<String>,
    env: Vec<(&'static str, &'static str)>,
    expect: Expect,
    why: &'static str,
}

struct Options {
    toolchain: String,
    secs: u32,
    build: bool,
    only: Vec<String>,
}

fn parse(args: &[String]) -> Result<Options, String> {
    let mut o = Options {
        toolchain: "nightly".to_owned(),
        secs: 10,
        build: true,
        only: Vec::new(),
    };
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--toolchain" => iter
                .next()
                .ok_or("--toolchain needs a name")?
                .clone_into(&mut o.toolchain),
            "--secs" => {
                o.secs = iter
                    .next()
                    .and_then(|s| s.parse().ok())
                    .filter(|s| *s > 0)
                    .ok_or("--secs needs a whole number of seconds")?;
            }
            "--no-build" => o.build = false,
            "--only" => o
                .only
                .push(iter.next().ok_or("--only needs a check")?.clone()),
            "-h" | "--help" => return Err(USAGE.to_owned()),
            other => return Err(format!("unknown option {other}; {USAGE}")),
        }
    }
    Ok(o)
}

/// The rustup proxy, with the outer cargo's toolchain variables removed so `+toolchain` decides.
fn rustup_tool(tool: &str, toolchain: &str) -> Command {
    let mut c = Command::new(tool);
    c.arg(format!("+{toolchain}"));
    for var in ["RUSTUP_TOOLCHAIN", "RUSTC", "RUSTDOC", "CARGO"] {
        c.env_remove(var);
    }
    c
}

pub fn rtsan(args: &[String]) -> Result<(), String> {
    let o = parse(args)?;
    if !(cfg!(target_os = "linux") && cfg!(target_arch = "x86_64")) {
        return Err("rtsan runs on Linux x86-64 only (the sanitizer runtime's target)".to_owned());
    }
    let meta = crate::metadata()?;
    let root = meta.workspace_root.clone();
    let dir = meta.target_directory.join("rtsan");
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let version = rustup_tool("rustc", &o.toolchain)
        .arg("-V")
        .output()
        .map_err(|e| format!("cannot run rustc +{}: {e}", o.toolchain))?;
    if !version.status.success() {
        return Err(format!(
            "no {} toolchain: {} (rustup toolchain install {} --profile minimal)",
            o.toolchain,
            String::from_utf8_lossy(&version.stderr).trim(),
            o.toolchain
        ));
    }
    let version = String::from_utf8_lossy(&version.stdout).trim().to_owned();
    println!(
        "rtsan: {version}, RUSTFLAGS=\"{RUSTFLAGS}\", into {}",
        dir.display()
    );
    if o.build {
        let status = rustup_tool("cargo", &o.toolchain)
            .current_dir(&root)
            .env("CARGO_TARGET_DIR", &dir)
            .env("RUSTFLAGS", RUSTFLAGS)
            .args([
                "build",
                "--locked",
                "--release",
                "--target",
                TRIPLE,
                "-p",
                "enth-plugin",
                "--features",
                "enth-plugin/ab-toggle",
                "-p",
                "enth-test-host",
            ])
            .status()
            .map_err(|e| format!("cannot run cargo +{}: {e}", o.toolchain))?;
        if !status.success() {
            return Err(format!("the RTSan build failed ({status})"));
        }
    }
    let out = dir.join(TRIPLE).join("release");
    let host = out.join("enth-test-host");
    let plugin = dir.join("enth-plugin.clap");
    std::fs::copy(out.join("libenth_plugin.so"), &plugin)
        .map_err(|e| format!("{}: {e} (build first)", out.display()))?;
    let checks = checks(&root, &plugin, o.secs);
    let mut failed = Vec::new();
    let mut rows = Vec::new();
    for check in checks
        .iter()
        .filter(|c| o.only.is_empty() || o.only.iter().any(|n| n == c.name))
    {
        let run = Command::new(&host)
            .args(&check.args)
            .envs(check.env.iter().copied())
            .current_dir(&dir)
            .output()
            .map_err(|e| format!("cannot run {}: {e}", host.display()))?;
        let stderr = String::from_utf8_lossy(&run.stderr).into_owned();
        let _ = std::fs::write(dir.join(format!("{}.json", check.name)), &run.stdout);
        let _ = std::fs::write(dir.join(format!("{}.err", check.name)), &stderr);
        let caught = stderr.contains("ERROR: RealtimeSanitizer");
        let code = run.status.code().unwrap_or(-1);
        let (ok, verdict) = match check.expect {
            Expect::Clean => (
                code == 0 && !caught,
                if caught {
                    "CAUGHT"
                } else if code == 0 {
                    "clean"
                } else {
                    "FAILED"
                },
            ),
            Expect::Caught => (caught, if caught { "caught" } else { "NOT CAUGHT" }),
            Expect::Info => (
                true,
                if caught {
                    "caught (info)"
                } else {
                    "clean (info)"
                },
            ),
        };
        let what = report_line(&stderr);
        rows.push(format!(
            "  {:<4} {:<18} {:<14} exit {:<3} {}{}",
            if ok { "ok" } else { "FAIL" },
            check.name,
            verdict,
            code,
            check.why,
            what.map(|w| format!(" [{w}]")).unwrap_or_default()
        ));
        if !ok || (check.expect == Expect::Info && caught) {
            println!("--- {} ({verdict}) ---", check.name);
            println!("{}", symbolized(&stderr, &[&plugin, &host]));
        }
        if !ok {
            failed.push(check.name);
        }
    }
    println!("\nrtsan ({version}):");
    for row in &rows {
        println!("{row}");
    }
    if failed.is_empty() {
        println!("rtsan: ok ({} checks)", rows.len());
        Ok(())
    } else {
        Err(format!(
            "rtsan: {} failed: {}",
            failed.len(),
            failed.join(", ")
        ))
    }
}

fn checks(root: &Path, plugin: &Path, secs: u32) -> Vec<Check> {
    let p = plugin.display().to_string();
    let init = root
        .join("content/presets/init.patch.json")
        .display()
        .to_string();
    let rich = root
        .join("xtask/reaper/enth-roundtrip.patch.json")
        .display()
        .to_string();
    let secs = secs.to_string();
    let v = |items: &[&str]| -> Vec<String> { items.iter().map(|s| (*s).to_owned()).collect() };
    let stats = |extra: &[&str]| -> Vec<String> {
        let mut a = v(&[
            "process-stats",
            &p,
            "--secs",
            &secs,
            "--warmup",
            "1",
            "--rt",
            "none",
        ]);
        a.extend(extra.iter().map(|s| (*s).to_owned()));
        a
    };
    let tc = vec![("ENTH_TEST_HOST_THREAD_CHECK", "1")];
    let chord = "C4@0+1:0.8,E4@0+1,G4@0+1,B4@0+1";
    let staggered = "C4@0+1:0.8,E4@0.1+1,G4@0.2+1,C2@0.3+0.5";
    let swaps = |kind: &str| {
        stats(&[
            "--state",
            &init,
            "--swap-state",
            &rich,
            "--swap-ms",
            "250",
            "--swap-kind",
            kind,
        ])
    };
    vec![
        Check {
            name: "control",
            args: v(&["render", &p, "--notes", chord, "--dur", "1"]),
            env: vec![
                ("ENTH_TEST_HOST_THREAD_CHECK", "1"),
                ("ENTH_RTSAN_CONTROL", "alloc"),
            ],
            expect: Expect::Caught,
            why: "the plugin allocates in process() on purpose",
        },
        Check {
            name: "render-chord",
            args: v(&[
                "render", &p, "--notes", chord, "--state", &init, "--dur", "2",
            ]),
            env: tc.clone(),
            expect: Expect::Clean,
            why: "Init, a chord, 64-frame blocks",
        },
        Check {
            name: "render-block-1",
            args: v(&[
                "render", &p, "--notes", staggered, "--state", &rich, "--block", "1", "--dur", "1",
            ]),
            env: tc.clone(),
            expect: Expect::Clean,
            why: "the rich patch, 1-frame blocks",
        },
        Check {
            name: "render-block-4096",
            args: v(&[
                "render", &p, "--notes", staggered, "--state", &rich, "--block", "4096", "--dur",
                "2",
            ]),
            env: tc.clone(),
            expect: Expect::Clean,
            why: "the rich patch, 4096-frame blocks",
        },
        Check {
            name: "swaps-crossfade",
            args: swaps("crossfade"),
            env: tc.clone(),
            expect: Expect::Clean,
            why: "a state load every 250 ms, crossfaded",
        },
        Check {
            name: "swaps-instant",
            args: swaps("instant"),
            env: tc.clone(),
            expect: Expect::Clean,
            why: "a state load every 250 ms, instant",
        },
        Check {
            name: "flood",
            args: stats(&["--state", &rich, "--flood", "64"]),
            env: tc.clone(),
            expect: Expect::Clean,
            why: "parameter floods of 64 events",
        },
        Check {
            name: "ab",
            args: stats(&["--ab", "250"]),
            env: tc.clone(),
            expect: Expect::Clean,
            why: "a Recreate round, A/B every 250 ms",
        },
        Check {
            name: "bursts",
            args: v(&["bursts", &p]),
            env: tc.clone(),
            expect: Expect::Clean,
            why: "16 to 4096 events in one block",
        },
        Check {
            name: "note-end",
            args: v(&["note-end", &p]),
            env: tc,
            expect: Expect::Clean,
            why: "note ends at several block sizes",
        },
        Check {
            name: "no-thread-check",
            args: swaps("instant"),
            env: Vec::new(),
            expect: Expect::Info,
            why: "a host without CLAP thread-check (nice-plug's thread::current() fallback)",
        },
    ]
}

/// The sanitizer's one-line summary (`unsafe-library-call` and the call), if it reported.
fn report_line(stderr: &str) -> Option<String> {
    stderr
        .lines()
        .find(|l| l.starts_with("Intercepted call") || l.contains("blocking-call"))
        .map(|l| l.trim().chars().take(100).collect())
}

/// The report's first lines, each frame `(<module>+0x<offset>)` of `modules` followed by its symbol (the nearest
/// text symbol at or below the offset, from `nm`), since the nightly runtime finds no `llvm-symbolizer` here.
fn symbolized(stderr: &str, modules: &[&Path]) -> String {
    let tables: Vec<(String, Vec<(u64, String)>)> = modules
        .iter()
        .map(|m| (m.display().to_string(), symbols(m)))
        .collect();
    stderr
        .lines()
        .take(40)
        .map(|line| {
            let named = tables.iter().find_map(|(path, table)| {
                let at = line.find(&format!("({path}+0x"))?;
                let hex = &line[at + path.len() + 4..];
                let hex = &hex[..hex.find(')')?];
                let offset = u64::from_str_radix(hex, 16).ok()?;
                let i = table
                    .partition_point(|(a, _)| *a <= offset)
                    .checked_sub(1)?;
                Some(table[i].1.chars().take(160).collect::<String>())
            });
            match named {
                Some(name) => format!("{line}\n        {name}"),
                None => line.to_owned(),
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// `nm -C --defined-only`'s text symbols of `path`, by address.
fn symbols(path: &Path) -> Vec<(u64, String)> {
    let Ok(out) = Command::new("nm")
        .args(["-C", "--defined-only"])
        .arg(path)
        .output()
    else {
        return Vec::new();
    };
    let mut table: Vec<(u64, String)> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| {
            let mut parts = l.splitn(3, ' ');
            let addr = u64::from_str_radix(parts.next()?, 16).ok()?;
            let kind = parts.next()?;
            matches!(kind, "t" | "T" | "w" | "W")
                .then(|| (addr, parts.next().unwrap_or("").to_owned()))
        })
        .collect();
    table.sort();
    table
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options_parse() {
        let o = parse(&[
            "--secs".into(),
            "3".into(),
            "--no-build".into(),
            "--only".into(),
            "ab".into(),
        ])
        .unwrap();
        assert_eq!(
            (o.secs, o.build, o.only.as_slice(), o.toolchain.as_str()),
            (3, false, &["ab".to_owned()][..], "nightly")
        );
        assert!(parse(&["--secs".into(), "0".into()]).is_err());
        assert!(parse(&["--bogus".into()]).is_err());
    }

    #[test]
    fn frames_are_named_from_the_symbol_table() {
        let text = "    #3 0x7f (/x/enth-plugin.clap+0x10) (BuildId: 1)";
        let tables = [(
            "/x/enth-plugin.clap".to_owned(),
            vec![(0x8, "a".to_owned()), (0x20, "b".to_owned())],
        )];
        // The same lookup `symbolized` makes, on a hand-made table.
        let (path, table) = &tables[0];
        let at = text.find(&format!("({path}+0x")).unwrap();
        let hex = &text[at + path.len() + 4..];
        let offset = u64::from_str_radix(&hex[..hex.find(')').unwrap()], 16).unwrap();
        let i = table.partition_point(|(a, _)| *a <= offset) - 1;
        assert_eq!(table[i].1, "a");
        assert_eq!(
            report_line("x\nIntercepted call to real-time unsafe function `malloc`\n"),
            Some("Intercepted call to real-time unsafe function `malloc`".to_owned())
        );
    }

    #[test]
    fn the_scenarios_have_one_control_and_unique_names() {
        let checks = checks(Path::new("/r"), Path::new("/t/enth-plugin.clap"), 5);
        let mut names: Vec<&str> = checks.iter().map(|c| c.name).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), checks.len());
        assert_eq!(
            checks.iter().filter(|c| c.expect == Expect::Caught).count(),
            1
        );
        assert!(checks.iter().all(|c| c.args[1] == "/t/enth-plugin.clap"));
    }
}
