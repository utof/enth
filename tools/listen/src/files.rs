//! JSON files written atomically (a temporary file, then a rename), and small path helpers.

use std::path::Path;

use crate::Res;

/// Write `value` as pretty JSON to `path` through a temporary file, so a crash never leaves half a file.
pub fn write_json<T: serde::Serialize>(path: &Path, value: &T) -> Res<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(value)?)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Read a JSON file.
pub fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Res<T> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()).into())
}

/// `/proc/loadavg` as "1min 5min 15min" (the `uptime` load), or "n/a" where it does not exist.
#[must_use]
pub fn load_avg() -> String {
    std::fs::read_to_string("/proc/loadavg").ok().map_or_else(
        || "n/a".to_string(),
        |s| s.split_whitespace().take(3).collect::<Vec<_>>().join(" "),
    )
}

/// Logical threads of this machine.
#[must_use]
pub fn host_threads() -> usize {
    std::thread::available_parallelism().map_or(1, usize::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_round_trip_through_a_temp_file() {
        let dir = std::env::temp_dir().join(format!("enth-listen-files-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("x.json");
        write_json(&p, &vec![1u32, 2, 3]).unwrap();
        assert!(!dir.join("x.tmp").exists());
        assert_eq!(read_json::<Vec<u32>>(&p).unwrap(), vec![1, 2, 3]);
        assert!(read_json::<Vec<u32>>(&dir.join("missing.json")).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
