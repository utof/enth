//! Process measurements from `/proc` (Linux).

use std::collections::BTreeMap;

use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Memory {
    pub pss_kb: u64,
    pub pss_anon_kb: u64,
    pub rss_kb: u64,
    /// The proportional share of this process's swapped-out pages (`SwapPss`): `pss_anon_kb + swap_pss_kb` is the
    /// anonymous footprint whether or not the kernel swapped part of it out (lane SOAK's metric).
    pub swap_pss_kb: u64,
}

/// Parses the text of `/proc/<pid>/smaps_rollup`.
pub fn parse_smaps_rollup(text: &str) -> Memory {
    let mut m = Memory::default();
    for line in text.lines() {
        let Some((key, rest)) = line.split_once(':') else {
            continue;
        };
        let kb = rest
            .trim()
            .strip_suffix("kB")
            .and_then(|n| n.trim().parse::<u64>().ok());
        match (key, kb) {
            ("Pss", Some(kb)) => m.pss_kb = kb,
            ("Pss_Anon", Some(kb)) => m.pss_anon_kb = kb,
            ("Rss", Some(kb)) => m.rss_kb = kb,
            ("SwapPss", Some(kb)) => m.swap_pss_kb = kb,
            _ => {}
        }
    }
    m
}

/// Proportional set size of this process, from `/proc/self/smaps_rollup`.
pub fn memory() -> Result<Memory, String> {
    let text = std::fs::read_to_string("/proc/self/smaps_rollup")
        .map_err(|e| format!("/proc/self/smaps_rollup: {e}"))?;
    Ok(parse_smaps_rollup(&text))
}

/// One mapping of `/proc/self/smaps`, its anonymous and swapped kilobytes (the leak diagnostics, lane LK).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Mapping {
    /// `start-end perms offset dev inode [name]`, the mapping's header line.
    pub header: String,
    pub size_kb: u64,
    pub anonymous_kb: u64,
    pub swap_kb: u64,
}

/// Parses the text of `/proc/<pid>/smaps` into its mappings.
pub fn parse_smaps(text: &str) -> Vec<Mapping> {
    let mut maps: Vec<Mapping> = Vec::new();
    for line in text.lines() {
        let first = line.split_whitespace().next().unwrap_or_default();
        if first.contains('-') && !first.ends_with(':') {
            maps.push(Mapping {
                header: line.split_whitespace().collect::<Vec<_>>().join(" "),
                ..Mapping::default()
            });
            continue;
        }
        let (Some(map), Some((key, rest))) = (maps.last_mut(), line.split_once(':')) else {
            continue;
        };
        let kb = rest
            .trim()
            .strip_suffix("kB")
            .and_then(|n| n.trim().parse::<u64>().ok());
        match (key, kb) {
            ("Size", Some(kb)) => map.size_kb = kb,
            ("Anonymous", Some(kb)) => map.anonymous_kb = kb,
            ("Swap", Some(kb)) => map.swap_kb = kb,
            _ => {}
        }
    }
    maps
}

/// This process's mappings, from `/proc/self/smaps`.
pub fn mappings() -> Vec<Mapping> {
    std::fs::read_to_string("/proc/self/smaps").map_or_else(|_| Vec::new(), |t| parse_smaps(&t))
}

/// The block size from which [`deterministic_malloc`] maps blocks on their own, and the free space at the top of the
/// heap above which it trims: 128 KiB, glibc's starting value for both, held fixed.
pub const MALLOC_THRESHOLD: i32 = 128 * 1024;

/// Makes glibc's allocator deterministic for a measurement process (lane LK; `editor-cycles` and `churn`); call it
/// before the plugin is loaded and before any thread starts.
///
/// - One arena (`M_ARENA_MAX` 1): with glibc's defaults a new thread (an editor's window thread, a churn cycle's audio
///   thread) can get another arena while the last one keeps its freed memory resident; `editor-cycles` read 53 and
///   405 KB per cycle that way, and churn's slope after the warm-up wandered from 0.41 to 0.92 KB per cycle against
///   its 1 KB limit (about 0.0 with these settings).
/// - Blocks of [`MALLOC_THRESHOLD`] or more mapped on their own, and the heap trimmed once that much is free at its
///   top, both fixed: setting them turns off glibc's sliding thresholds, which follow the largest block freed so far
///   (with them a 9 MB block was still resident after some editor closes and not after others).
///
/// A leak still shows: memory never freed stays resident whatever the policy, and memory freed is given back instead
/// of hiding the next leak in pages that are already resident (a 26 KB per open leak read 16 and 21 KB per cycle with
/// glibc's defaults, 26 above the residual with these). Returns what it set, for the report.
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

/// Gives every free page of every malloc arena back to the kernel (glibc's `malloc_trim(0)`), so that a sample after
/// it counts the pages live allocations occupy, not the free memory glibc keeps for reuse. Returns whether it ran.
pub fn trim_heap() -> bool {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    {
        // SAFETY: `malloc_trim` takes each arena's lock and only releases free pages.
        unsafe { libc::malloc_trim(0) };
        true
    }
    #[cfg(not(all(target_os = "linux", target_env = "gnu")))]
    {
        false
    }
}

/// Number of threads in this process.
pub fn thread_count() -> usize {
    std::fs::read_dir("/proc/self/task").map_or(0, Iterator::count)
}

/// Thread names (`comm`) of this process with their counts.
pub fn thread_names() -> BTreeMap<String, usize> {
    let mut names = BTreeMap::new();
    if let Ok(dir) = std::fs::read_dir("/proc/self/task") {
        for entry in dir.flatten() {
            let comm = std::fs::read_to_string(entry.path().join("comm"))
                .map_or_else(|_| "?".to_owned(), |c| c.trim().to_owned());
            *names.entry(comm).or_insert(0) += 1;
        }
    }
    names
}

/// The 1, 5 and 15 minute load averages, as printed by the kernel.
pub fn loadavg() -> String {
    std::fs::read_to_string("/proc/loadavg").map_or_else(
        |_| "?".to_owned(),
        |t| t.split_whitespace().take(3).collect::<Vec<_>>().join(" "),
    )
}

/// Least-squares slope of `ys` against their index, in units of `y` per step.
pub fn slope(ys: &[f64]) -> f64 {
    let n = ys.len() as f64;
    if ys.len() < 2 {
        return 0.0;
    }
    let mean_x = (n - 1.0) / 2.0;
    let mean_y = ys.iter().sum::<f64>() / n;
    let (mut num, mut den) = (0.0, 0.0);
    for (i, y) in ys.iter().enumerate() {
        let dx = i as f64 - mean_x;
        num += dx * (y - mean_y);
        den += dx * dx;
    }
    num / den
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smaps_rollup_is_parsed() {
        let text = "55d0-7ffc ---p 00000000 00:00 0   [rollup]\nRss:               54321 kB\nPss:               12345 kB\nPss_Dirty:          100 kB\nPss_Anon:            2000 kB\nReferenced:        1 kB\nSwap:               700 kB\nSwapPss:            600 kB\n";
        let m = parse_smaps_rollup(text);
        assert_eq!(
            m,
            Memory {
                pss_kb: 12_345,
                pss_anon_kb: 2_000,
                rss_kb: 54_321,
                swap_pss_kb: 600,
            }
        );
    }

    #[test]
    fn smaps_mappings_are_parsed() {
        let text = "5555-6666 rw-p 00000000 00:00 0                          [heap]\nSize:               132 kB\nAnonymous:           64 kB\nSwap:                 8 kB\n7f00-7f10 r-xp 00001000 08:02 123  /usr/lib/libGL.so.1\nSize:                 64 kB\nAnonymous:            0 kB\nVmFlags: rd ex mr mw me\n";
        let maps = parse_smaps(text);
        assert_eq!(maps.len(), 2);
        assert_eq!(maps[0].header, "5555-6666 rw-p 00000000 00:00 0 [heap]");
        assert_eq!(
            (maps[0].size_kb, maps[0].anonymous_kb, maps[0].swap_kb),
            (132, 64, 8)
        );
        assert_eq!((maps[1].size_kb, maps[1].anonymous_kb), (64, 0));
    }

    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    #[test]
    fn the_allocator_takes_the_deterministic_settings() {
        let set = deterministic_malloc();
        assert_eq!(set["arena_max"], 1, "{set}");
        assert_eq!(set["mmap_threshold"], MALLOC_THRESHOLD);
    }

    #[test]
    fn slope_of_a_line() {
        assert!((slope(&[10.0, 12.0, 14.0, 16.0]) - 2.0).abs() < 1e-12);
        assert_eq!(slope(&[5.0]), 0.0);
        assert!(slope(&[3.0, 3.0, 3.0]).abs() < 1e-12);
    }

    #[test]
    fn this_process_has_memory_and_threads() {
        assert!(memory().unwrap().pss_kb > 0);
        assert!(thread_count() >= 1);
        assert!(!thread_names().is_empty());
    }
}
