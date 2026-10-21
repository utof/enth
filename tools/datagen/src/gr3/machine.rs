//! The machine's conditions, as a run records them: the date, the load, the CPU time this process and every other
//! process used over a timed window, the machine itself, and the quiet lock. Linux `/proc` where it exists; on
//! another system the readings are absent (`None`), never guessed.

use std::fs;
use std::path::Path;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;

/// Clock ticks per second in `/proc/stat` and `/proc/self/stat` (`USER_HZ`, 100 on every Linux build we run).
const USER_HZ: f64 = 100.0;

/// The UTC date and time, ISO 8601 to the second (`2026-10-21T12:34:56Z`).
pub fn now_utc() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    iso_utc(secs)
}

/// `secs` since the epoch as ISO 8601 UTC (Howard Hinnant's civil-from-days).
pub fn iso_utc(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        (rem / 60) % 60,
        rem % 60
    )
}

/// The 1-minute load average.
pub fn load1() -> Option<f64> {
    fs::read_to_string("/proc/loadavg")
        .ok()?
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

/// (busy, total) ticks of the whole machine from `/proc/stat`'s first line; idle and iowait are not busy.
fn machine_ticks() -> Option<(u64, u64)> {
    let text = fs::read_to_string("/proc/stat").ok()?;
    let line = text.lines().next()?;
    let v: Vec<u64> = line
        .split_whitespace()
        .skip(1)
        .filter_map(|x| x.parse().ok())
        .collect();
    if v.len() < 5 {
        return None;
    }
    // user nice system idle iowait irq softirq steal (guest time is already in user).
    let total: u64 = v.iter().take(8).sum();
    let idle = v[3] + v[4];
    Some((total - idle, total))
}

/// This process's CPU ticks (user + system) from `/proc/self/stat`.
fn self_ticks() -> Option<u64> {
    let text = fs::read_to_string("/proc/self/stat").ok()?;
    // The command name is in parentheses and may hold spaces: fields count from after the last ')'.
    let rest = &text[text.rfind(')')? + 1..];
    let f: Vec<&str> = rest.split_whitespace().collect();
    // After ')': state is field 3, so utime (14) and stime (15) are at 11 and 12.
    Some(f.get(11)?.parse::<u64>().ok()? + f.get(12)?.parse::<u64>().ok()?)
}

/// A timed window's start.
pub struct Window {
    t0: Instant,
    machine: Option<(u64, u64)>,
    me: Option<u64>,
    load_before: Option<f64>,
}

/// What a timed window saw.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Conditions {
    /// Wall seconds.
    pub seconds: f64,
    /// CPU seconds this process used (every thread).
    pub cpu_s: Option<f64>,
    /// The cores other processes kept busy on average (their CPU seconds over the wall seconds): the contention a
    /// timed run met. Near 0 on a quiet machine.
    pub others_cores: Option<f64>,
    /// The 1-minute load average before and after.
    pub load1_before: Option<f64>,
    /// After.
    pub load1_after: Option<f64>,
}

impl Window {
    /// Start a window now.
    pub fn start() -> Window {
        Window {
            load_before: load1(),
            machine: machine_ticks(),
            me: self_ticks(),
            t0: Instant::now(),
        }
    }

    /// The window's start.
    pub fn t0(&self) -> Instant {
        self.t0
    }

    /// End it.
    pub fn end(&self) -> Conditions {
        let seconds = self.t0.elapsed().as_secs_f64();
        let me = match (self.me, self_ticks()) {
            (Some(a), Some(b)) => Some(b.saturating_sub(a) as f64 / USER_HZ),
            _ => None,
        };
        let others = match (self.machine, machine_ticks(), me) {
            (Some((b0, _)), Some((b1, _)), Some(me)) if seconds > 0.0 => {
                Some(((b1.saturating_sub(b0) as f64 / USER_HZ) - me).max(0.0) / seconds)
            }
            _ => None,
        };
        Conditions {
            seconds,
            cpu_s: me,
            others_cores: others,
            load1_before: self.load_before,
            load1_after: load1(),
        }
    }
}

/// The machine, once per run.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct MachineInfo {
    /// Hardware threads.
    pub threads: usize,
    /// The CPU's model name.
    pub cpu: Option<String>,
    /// The kernel release.
    pub kernel: Option<String>,
    /// Total RAM (GiB).
    pub ram_gib: Option<f64>,
    /// The 1-minute load average when the run started.
    pub load1: Option<f64>,
}

impl MachineInfo {
    /// Read it now.
    pub fn read() -> MachineInfo {
        let cpu = fs::read_to_string("/proc/cpuinfo").ok().and_then(|t| {
            t.lines()
                .find(|l| l.starts_with("model name"))
                .and_then(|l| l.split_once(':'))
                .map(|(_, v)| v.trim().to_owned())
        });
        let kernel = fs::read_to_string("/proc/sys/kernel/osrelease")
            .ok()
            .map(|s| s.trim().to_owned());
        let ram_gib = fs::read_to_string("/proc/meminfo").ok().and_then(|t| {
            t.lines()
                .find(|l| l.starts_with("MemTotal:"))
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|kb| kb.parse::<f64>().ok())
                .map(|kb| (kb / 1024.0 / 1024.0 * 10.0).round() / 10.0)
        });
        MachineInfo {
            threads: std::thread::available_parallelism().map_or(1, std::num::NonZero::get),
            cpu,
            kernel,
            ram_gib,
            load1: load1(),
        }
    }
}

/// The quiet lock, held exclusively (`flock -x`, the same lock `flock -s` builds take) until dropped.
pub struct QuietLock {
    file: fs::File,
    /// Seconds spent waiting for it.
    pub waited_s: f64,
}

impl QuietLock {
    /// Wait for the lock at `path` and take it exclusively.
    pub fn take(path: &Path) -> Result<QuietLock, String> {
        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)
            .map_err(|e| format!("cannot open the quiet lock {}: {e}", path.display()))?;
        let t = Instant::now();
        file.lock()
            .map_err(|e| format!("cannot take the quiet lock {}: {e}", path.display()))?;
        Ok(QuietLock {
            file,
            waited_s: t.elapsed().as_secs_f64(),
        })
    }
}

impl Drop for QuietLock {
    fn drop(&mut self) {
        // Closing the file releases the lock as well; unlocking first makes the release explicit.
        let _ = self.file.unlock();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_are_iso_utc() {
        assert_eq!(iso_utc(0), "1970-01-01T00:00:00Z");
        // 2026-10-21 12:34:56 UTC, by `date -u -d @1792586096`.
        assert_eq!(iso_utc(1_792_586_096), "2026-10-21T12:34:56Z");
        // A leap day.
        assert_eq!(iso_utc(951_782_400), "2000-02-29T00:00:00Z");
    }

    #[test]
    fn a_window_reads_this_process() {
        let w = Window::start();
        let mut x = 0u64;
        for i in 0..20_000_000u64 {
            x = x.wrapping_add(i * i);
        }
        assert_ne!(x, 1);
        let c = w.end();
        assert!(c.seconds > 0.0);
        if cfg!(target_os = "linux") {
            assert!(c.cpu_s.is_some() && c.others_cores.is_some());
        }
    }
}
