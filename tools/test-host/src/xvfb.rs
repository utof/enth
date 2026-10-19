//! The safety guard for everything that opens a window.
//!
//! On 2026-10-19 the NVIDIA 595-series X driver crashed Xorg after short-lived X clients ran on the real
//! display (AGENTS.md, "UI"). `editor-cycles` opens and closes editors hundreds of times, so it refuses to
//! run unless `DISPLAY` points at an Xvfb server that is running on this machine.
//!
//! The check is on the server process, not on the display number or the vendor string (a real Xorg
//! reports the same vendor): some process called `Xvfb` must have been started with `:N` as an argument,
//! where `N` is the number in `DISPLAY`. `xvfb-run` starts it exactly that way.

use std::path::Path;

/// The display number in a local `DISPLAY` value: `:99`, `:99.0` and `unix:99` give 99. Remote
/// displays (`host:0`) are errors: only a local server can be verified.
pub fn parse_display(display: &str) -> Result<u32, String> {
    let (host, rest) = display
        .split_once(':')
        .ok_or_else(|| format!("DISPLAY={display:?} has no ':'"))?;
    if !(host.is_empty() || host == "unix") {
        return Err(format!(
            "DISPLAY={display:?} names a remote host; only a local Xvfb can be verified"
        ));
    }
    let number = rest.split('.').next().unwrap_or("");
    number
        .parse::<u32>()
        .map_err(|_| format!("DISPLAY={display:?} has no display number"))
}

/// Whether `argv` (a process command line) starts an `Xvfb` server for display `display`.
pub fn is_xvfb_for(argv: &[&str], display: u32) -> bool {
    let Some(program) = argv.first() else {
        return false;
    };
    let name = Path::new(program)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");
    name == "Xvfb" && argv[1..].contains(&format!(":{display}").as_str())
}

/// Finds a running Xvfb for `display` by scanning `/proc/*/cmdline`; returns its pid.
pub fn find_xvfb(display: u32) -> Option<u32> {
    for entry in std::fs::read_dir("/proc").ok()?.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|n| n.parse::<u32>().ok())
        else {
            continue;
        };
        let Ok(raw) = std::fs::read(entry.path().join("cmdline")) else {
            continue;
        };
        let text = String::from_utf8_lossy(&raw);
        let argv: Vec<&str> = text.split('\0').filter(|a| !a.is_empty()).collect();
        if is_xvfb_for(&argv, display) {
            return Some(pid);
        }
    }
    None
}

/// What the guard found.
#[derive(Clone, Debug)]
pub struct Xvfb {
    pub display: String,
    pub pid: u32,
}

/// Refuses (with a hint) unless this process is going to talk to an Xvfb server.
pub fn require_xvfb() -> Result<Xvfb, String> {
    const HINT: &str = "run it as: xvfb-run -a -s \"-screen 0 1920x1080x24\" enth-test-host ...";
    if std::env::var_os("WAYLAND_DISPLAY").is_some_and(|v| !v.is_empty()) {
        return Err(format!(
            "WAYLAND_DISPLAY is set; refusing to open windows. unset WAYLAND_DISPLAY. {HINT}"
        ));
    }
    let display = std::env::var("DISPLAY").unwrap_or_default();
    if display.is_empty() {
        return Err(format!(
            "DISPLAY is not set; refusing to open windows. {HINT}"
        ));
    }
    let number = parse_display(&display).map_err(|e| format!("{e}. {HINT}"))?;
    match find_xvfb(number) {
        Some(pid) => Ok(Xvfb { display, pid }),
        None => Err(format!(
            "DISPLAY={display} is not served by an Xvfb process on this machine (no `Xvfb :{number}` found); \
             refusing to open windows on a real display. {HINT}"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_numbers() {
        assert_eq!(parse_display(":99"), Ok(99));
        assert_eq!(parse_display(":0.0"), Ok(0));
        assert_eq!(parse_display("unix:7.1"), Ok(7));
        assert!(parse_display("localhost:10.0").is_err());
        assert!(parse_display("99").is_err());
        assert!(parse_display(":").is_err());
        assert!(parse_display("").is_err());
    }

    #[test]
    fn only_an_xvfb_command_line_for_that_display_matches() {
        let xvfb = [
            "Xvfb",
            ":99",
            "-screen",
            "0",
            "1920x1080x24",
            "-nolisten",
            "tcp",
        ];
        assert!(is_xvfb_for(&xvfb, 99));
        assert!(!is_xvfb_for(&xvfb, 98));
        assert!(is_xvfb_for(&["/usr/bin/Xvfb", ":5"], 5));
        // The real servers and look-alikes are refused.
        assert!(!is_xvfb_for(
            &["/usr/lib/xorg/Xorg", ":0", "-seat", "seat0"],
            0
        ));
        assert!(!is_xvfb_for(&["Xwayland", ":0"], 0));
        assert!(!is_xvfb_for(&["xvfb-run", ":99"], 99));
        assert!(!is_xvfb_for(&["vim", "Xvfb", ":99"], 99));
        assert!(!is_xvfb_for(&[], 0));
    }

    #[test]
    fn scanning_proc_finds_nothing_for_an_unused_display() {
        assert_eq!(find_xvfb(4_000_000_000), None);
    }
}
