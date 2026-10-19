//! CLI helpers: load average, JSON files, error type.

use std::path::Path;

pub type Res<T> = Result<T, Box<dyn std::error::Error>>;

/// `/proc/loadavg` as "1min 5min 15min" (the `uptime` load), or "n/a" off Linux.
pub fn load_avg() -> String {
    std::fs::read_to_string("/proc/loadavg").ok().map_or_else(
        || "n/a".to_string(),
        |s| s.split_whitespace().take(3).collect::<Vec<_>>().join(" "),
    )
}

pub fn write_json<T: serde::Serialize>(path: &Path, value: &T) -> Res<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(value)?)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

pub fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Res<T> {
    Ok(serde_json::from_str(&std::fs::read_to_string(path)?)?)
}

pub fn parse_list<T: std::str::FromStr>(s: &str) -> Res<Vec<T>> {
    s.split(',')
        .map(|p| {
            p.trim()
                .parse::<T>()
                .map_err(|_| format!("cannot parse {p:?}").into())
        })
        .collect()
}

pub fn host_threads() -> usize {
    std::thread::available_parallelism().map_or(1, usize::from)
}
