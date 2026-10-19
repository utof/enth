//! `process()` timings inside the plugin (lane PL), behind the `rt-stats` cargo feature: without it this module is
//! not compiled and `process()` reads no clock.
//!
//! The audio thread records each block's duration (`Instant::now()` at the top of `Enth::process_block` and
//! `elapsed()` at its end: the plugin's own work, nice-plug's wrapper excluded) into a log-scale histogram of atomic
//! counters, with 16 buckets per octave of nanoseconds (each bucket is at most 6.25 % wide), plus the count, the
//! frames, the sum and the maximum. Recording is a handful of relaxed atomic adds: no lock, no allocation.
//!
//! Any other thread reads a [`Snapshot`] (percentiles are the upper edge of the bucket that holds them, so they
//! overstate by at most one bucket). At `deactivate` (host main thread), when the environment variable
//! `ENTH_RT_STATS` names a file, one JSON line (`enth.plugin-rt-stats/1`) is appended to it and the counters start
//! again. `enth-test-host process-stats` measures the whole `process()` call from the host's side; this
//! histogram splits off the plugin's part (docs/plugin.md, "rt-stats").

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde_json::{Value, json};

/// Sub-buckets per octave.
const SUB: u32 = 16;
/// `log2(SUB)`.
const SUB_BITS: u32 = 4;
/// Buckets: values below [`SUB`] ns get one each, then [`SUB`] per octave up to `u64::MAX`.
pub const BUCKETS: usize = (64 * SUB) as usize;

/// The environment variable naming the file `deactivate` appends a report to.
pub const ENV: &str = "ENTH_RT_STATS";

/// The bucket of a duration in nanoseconds.
#[must_use]
pub fn bucket(ns: u64) -> usize {
    if ns < u64::from(SUB) {
        return ns as usize;
    }
    let msb = ns.ilog2();
    let sub = (ns >> (msb - SUB_BITS)) & u64::from(SUB - 1);
    (msb * SUB) as usize + sub as usize
}

/// The smallest duration (ns) above bucket `i`: its exclusive upper edge.
#[must_use]
pub fn upper_edge(i: usize) -> u64 {
    if i < SUB as usize {
        return i as u64 + 1;
    }
    let msb = i as u32 / SUB;
    if msb < SUB_BITS {
        // Buckets SUB..SUB·SUB_BITS are never used: every value below SUB has its own bucket.
        return u64::from(SUB);
    }
    let sub = u64::from(i as u32 % SUB);
    // Bucket i holds [(SUB + sub) << (msb - SUB_BITS), (SUB + sub + 1) << (msb - SUB_BITS)).
    (u64::from(SUB) + sub + 1).saturating_mul(1 << (msb - SUB_BITS))
}

/// The histogram and counters the audio thread writes.
pub struct RtStats {
    buckets: Box<[AtomicU64]>,
    blocks: AtomicU64,
    frames: AtomicU64,
    total_ns: AtomicU64,
    max_ns: AtomicU64,
}

impl Default for RtStats {
    fn default() -> Self {
        Self::new()
    }
}

impl RtStats {
    /// Empty counters (allocates the histogram once, here).
    #[must_use]
    pub fn new() -> Self {
        RtStats {
            buckets: (0..BUCKETS).map(|_| AtomicU64::new(0)).collect(),
            blocks: AtomicU64::new(0),
            frames: AtomicU64::new(0),
            total_ns: AtomicU64::new(0),
            max_ns: AtomicU64::new(0),
        }
    }

    /// One block of `frames` frames took `elapsed` (audio thread: relaxed atomics only).
    pub fn record(&self, elapsed: Duration, frames: u64) {
        let ns = u64::try_from(elapsed.as_nanos()).unwrap_or(u64::MAX);
        self.buckets[bucket(ns)].fetch_add(1, Ordering::Relaxed);
        self.blocks.fetch_add(1, Ordering::Relaxed);
        self.frames.fetch_add(frames, Ordering::Relaxed);
        self.total_ns.fetch_add(ns, Ordering::Relaxed);
        self.max_ns.fetch_max(ns, Ordering::Relaxed);
    }

    /// The counters now (any thread; a block recorded meanwhile may be half in it).
    #[must_use]
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            buckets: self
                .buckets
                .iter()
                .map(|b| b.load(Ordering::Relaxed))
                .collect(),
            blocks: self.blocks.load(Ordering::Relaxed),
            frames: self.frames.load(Ordering::Relaxed),
            total_ns: self.total_ns.load(Ordering::Relaxed),
            max_ns: self.max_ns.load(Ordering::Relaxed),
        }
    }

    /// Start again from zero (not while the audio thread records, or the next snapshot mixes the two).
    pub fn clear(&self) {
        for b in &self.buckets {
            b.store(0, Ordering::Relaxed);
        }
        for c in [&self.blocks, &self.frames, &self.total_ns, &self.max_ns] {
            c.store(0, Ordering::Relaxed);
        }
    }

    /// At `deactivate` (host main thread, the audio thread is stopped): append the report to the file `$ENTH_RT_STATS`
    /// names, if any, and clear. An I/O error is ignored (a measurement aid must not break the plugin).
    pub fn write_report(&self) {
        let Some(path) = std::env::var_os(ENV) else {
            return;
        };
        let snapshot = self.snapshot();
        if snapshot.blocks > 0 {
            use std::io::Write;
            let line = format!("{}\n", snapshot.to_json());
            let _ = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .and_then(|mut f| f.write_all(line.as_bytes()));
        }
        self.clear();
    }
}

/// A copy of the counters.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    /// Blocks per bucket ([`bucket`], [`upper_edge`]).
    pub buckets: Vec<u64>,
    /// Blocks recorded.
    pub blocks: u64,
    /// Frames in them.
    pub frames: u64,
    /// Their total duration, ns.
    pub total_ns: u64,
    /// The longest block, ns.
    pub max_ns: u64,
}

impl Snapshot {
    /// The `q`-quantile (0–1) in ns: the upper edge of the bucket that holds it, capped at the maximum; 0 with no
    /// blocks.
    #[must_use]
    pub fn quantile(&self, q: f64) -> u64 {
        if self.blocks == 0 {
            return 0;
        }
        // The rank of the quantile, 1-based: the smallest k with k >= q · n.
        let rank = ((q.clamp(0.0, 1.0) * self.blocks as f64).ceil() as u64).max(1);
        let mut seen = 0;
        for (i, &n) in self.buckets.iter().enumerate() {
            seen += n;
            if seen >= rank {
                return upper_edge(i).min(self.max_ns);
            }
        }
        self.max_ns
    }

    /// The mean block duration, ns.
    #[must_use]
    pub fn mean_ns(&self) -> f64 {
        if self.blocks == 0 {
            0.0
        } else {
            self.total_ns as f64 / self.blocks as f64
        }
    }

    /// The report `deactivate` writes: counts, the mean and the percentiles in microseconds, and the non-empty
    /// buckets as `[upper edge ns, blocks]`.
    #[must_use]
    pub fn to_json(&self) -> Value {
        let us = |ns: u64| ns as f64 / 1000.0;
        json!({
            "schema": "enth.plugin-rt-stats/1",
            "blocks": self.blocks,
            "frames": self.frames,
            "mean_frames": if self.blocks == 0 { 0.0 } else { self.frames as f64 / self.blocks as f64 },
            "mean_us": self.mean_ns() / 1000.0,
            "p50_us": us(self.quantile(0.5)),
            "p99_us": us(self.quantile(0.99)),
            "p999_us": us(self.quantile(0.999)),
            "max_us": us(self.max_ns),
            "buckets": self.buckets.iter().enumerate().filter(|(_, n)| **n > 0)
                .map(|(i, n)| json!([upper_edge(i), n])).collect::<Vec<_>>(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buckets_are_monotonic_and_narrow() {
        let mut last = 0;
        for ns in (0..100_000u64).chain([1 << 20, 1 << 33, u64::MAX / 2, u64::MAX]) {
            let b = bucket(ns);
            assert!(b >= last, "{ns}");
            assert!(b < BUCKETS);
            last = b;
            assert!(upper_edge(b) > ns || upper_edge(b) == u64::MAX, "{ns} {b}");
            if b > 0 {
                assert!(upper_edge(b - 1) <= ns, "{ns}: bucket {b} starts above it");
            }
            if (16..u64::MAX / 32).contains(&ns) {
                // At most 1/16 of the bucket's lower edge wide.
                let lower = upper_edge(b - 1);
                assert!((upper_edge(b) - lower) * 16 <= lower, "{ns}");
            }
        }
    }

    #[test]
    fn quantiles_come_from_the_histogram() {
        let stats = RtStats::new();
        for _ in 0..990 {
            stats.record(Duration::from_micros(10), 64);
        }
        for _ in 0..10 {
            stats.record(Duration::from_micros(200), 64);
        }
        let s = stats.snapshot();
        assert_eq!((s.blocks, s.frames, s.max_ns), (1000, 64_000, 200_000));
        let p50 = s.quantile(0.5);
        assert!((10_000..=10_700).contains(&p50), "{p50}");
        let p99 = s.quantile(0.99);
        assert!((10_000..=10_700).contains(&p99), "{p99}");
        assert_eq!(s.quantile(0.999), 200_000, "capped at the maximum");
        assert!((s.mean_ns() - 11_900.0).abs() < 1.0);
        let report = s.to_json();
        assert_eq!(report["schema"], "enth.plugin-rt-stats/1");
        assert_eq!(report["buckets"].as_array().unwrap().len(), 2);
        stats.clear();
        assert_eq!(stats.snapshot().blocks, 0);
        assert_eq!(stats.snapshot().quantile(0.99), 0);
    }
}
