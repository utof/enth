//! Process measurements from `/proc` (Linux).

use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Memory {
    pub pss_kb: u64,
    pub pss_anon_kb: u64,
    pub rss_kb: u64,
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
        let text = "55d0-7ffc ---p 00000000 00:00 0   [rollup]\nRss:               54321 kB\nPss:               12345 kB\nPss_Dirty:          100 kB\nPss_Anon:            2000 kB\nReferenced:        1 kB\n";
        let m = parse_smaps_rollup(text);
        assert_eq!(
            m,
            Memory {
                pss_kb: 12_345,
                pss_anon_kb: 2_000,
                rss_kb: 54_321
            }
        );
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
