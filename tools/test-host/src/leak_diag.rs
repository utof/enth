//! Leak diagnostics for `editor-cycles` (lane LK), on only when `ENTH_LEAK_DIAG` is set in the environment (it reaches
//! the measurement processes through `cargo xtask leak-test`, `xvfb-run` and `--mode both`).
//!
//! When it is on, each editor run's report gets a `diag` object:
//!
//! - `thread_events`: the thread names whenever the count after a close changes (the first read, before the test
//!   host lets an exiting thread go; see `editor::settled_threads`);
//! - `anon_steps`: every jump of more than 2 MB in `Pss_Anon` between two samples, with the mappings of
//!   `/proc/self/smaps` whose anonymous kilobytes changed by 256 KB or more since the last snapshot (taken every 10th
//!   cycle), grouped by kind as well, and four follow-up samples 20 to 700 ms later that show whether the memory was
//!   only freed late.
//!
//! This is how lane LK found that the gate's 53 KB and 405 KB per cycle were glibc arenas (a 64 MB mapping filling up
//! when the new window thread got another arena) and that the fourth thread was Mesa's `disk$0` queue thread.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, json};

use crate::procfs::{self, Mapping, Memory};

/// A change in `Pss_Anon` above this between two samples is a step worth explaining.
const STEP_KB: i64 = 2048;
/// A mapping whose anonymous kilobytes changed by at least this is listed.
const MAPPING_KB: i64 = 256;

#[derive(Default)]
pub struct Diag {
    on: bool,
    last_anon: Option<u64>,
    last_threads: Option<usize>,
    last_maps: Vec<Mapping>,
    followup: bool,
    thread_events: Vec<Value>,
    anon_steps: Vec<Value>,
}

/// A mapping's identity across samples: its start address, permissions and name (a heap or an arena that grows keeps
/// its start; its end moves).
fn key(m: &Mapping) -> String {
    let f: Vec<&str> = m.header.split(' ').collect();
    let start = f
        .first()
        .and_then(|r| r.split('-').next())
        .unwrap_or_default();
    format!("{start} {} {}", f.get(1).unwrap_or(&""), name(m))
}

fn name(m: &Mapping) -> String {
    m.header.split(' ').skip(5).collect::<Vec<_>>().join(" ")
}

/// What changed between two snapshots: the mappings whose anonymous kilobytes moved by [`MAPPING_KB`] or more, and the
/// change per kind (file name, or `anon <perms>`).
pub fn diff(before: &[Mapping], after: &[Mapping]) -> (Vec<Value>, BTreeMap<String, Value>) {
    let b: BTreeMap<String, &Mapping> = before.iter().map(|m| (key(m), m)).collect();
    let a: BTreeMap<String, &Mapping> = after.iter().map(|m| (key(m), m)).collect();
    let anon = |m: Option<&&Mapping>| m.map_or(0, |m| m.anonymous_kb as i64);
    let size = |m: Option<&&Mapping>| m.map_or(0, |m| m.size_kb as i64);
    let mut changed = Vec::new();
    for k in b.keys().chain(a.keys()).collect::<BTreeSet<_>>() {
        let (mb, ma) = (b.get(k), a.get(k));
        if (anon(ma) - anon(mb)).abs() >= MAPPING_KB {
            changed.push(json!({
                "mapping": k, "size_before_kb": size(mb), "size_after_kb": size(ma),
                "anon_before_kb": anon(mb), "anon_after_kb": anon(ma),
            }));
        }
    }
    let mut kinds: BTreeMap<String, (i64, i64)> = BTreeMap::new();
    for (maps, sign) in [(before, -1), (after, 1)] {
        for m in maps {
            let n = name(m);
            let perms = m.header.split(' ').nth(1).unwrap_or_default();
            let kind = if n.is_empty() {
                format!("anon {perms}")
            } else {
                n
            };
            let e = kinds.entry(kind).or_default();
            e.0 += sign * m.anonymous_kb as i64;
            e.1 += sign * m.size_kb as i64;
        }
    }
    let kinds = kinds
        .into_iter()
        .filter(|(_, (a, _))| a.abs() >= MAPPING_KB)
        .map(|(k, (a, s))| (k, json!({ "anon_kb": a, "size_kb": s })))
        .collect();
    (changed, kinds)
}

impl Diag {
    pub fn from_env() -> Self {
        Self {
            on: std::env::var_os("ENTH_LEAK_DIAG").is_some(),
            ..Self::default()
        }
    }

    /// One sample after a close (`cycle` counts from 0).
    pub fn observe(&mut self, cycle: usize, memory: Memory, threads: usize) {
        if !self.on {
            return;
        }
        if self.last_threads.is_some_and(|t| t != threads) {
            self.thread_events.push(json!({
                "cycle": cycle, "from": self.last_threads, "to": threads, "names": procfs::thread_names(),
            }));
        }
        self.last_threads = Some(threads);
        let delta = self
            .last_anon
            .map_or(0, |last| memory.pss_anon_kb as i64 - last as i64);
        self.last_anon = Some(memory.pss_anon_kb);
        let jump = delta.abs() > STEP_KB;
        if !jump && !cycle.is_multiple_of(10) && !self.last_maps.is_empty() {
            return;
        }
        let maps = procfs::mappings();
        if jump {
            let (mappings, by_kind) = diff(&self.last_maps, &maps);
            self.anon_steps.push(json!({
                "cycle": cycle, "delta_kb": delta, "pss_anon_kb": memory.pss_anon_kb,
                "swap_pss_kb": memory.swap_pss_kb, "by_kind": by_kind, "mappings": mappings,
                "threads": procfs::thread_names(),
            }));
            self.followup = true;
        } else {
            self.last_maps = maps;
        }
    }

    /// After a step: is it still there a little later (memory freed late), and which threads are alive meanwhile?
    /// `wait(ms)` services the plugin while it waits.
    pub fn follow_up(&mut self, wait: &mut dyn FnMut(u64)) {
        if !std::mem::take(&mut self.followup) {
            return;
        }
        let mut after = Vec::new();
        for ms in [20, 80, 200, 700] {
            wait(ms);
            after.push(json!({
                "after_more_ms": ms,
                "pss_anon_kb": procfs::memory().map_or(0, |m| m.pss_anon_kb),
                "threads": procfs::thread_names(),
            }));
        }
        if let Some(step) = self.anon_steps.last_mut() {
            step["followup"] = Value::Array(after);
        }
    }

    pub fn report(&self) -> Value {
        if !self.on {
            return Value::Null;
        }
        json!({ "thread_events": self.thread_events, "anon_steps": self.anon_steps })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(header: &str, size_kb: u64, anonymous_kb: u64) -> Mapping {
        Mapping {
            header: header.to_owned(),
            size_kb,
            anonymous_kb,
            swap_kb: 0,
        }
    }

    #[test]
    fn a_growing_arena_is_one_mapping_and_one_kind() {
        let before = [
            map("7e6518000000-7e6518021000 rw-p 00000000 00:00 0", 132, 4),
            map("5555-6666 rw-p 00000000 00:00 0 [heap]", 900, 800),
        ];
        let after = [
            map(
                "7e6518000000-7e651be80000 rw-p 00000000 00:00 0",
                63988,
                35208,
            ),
            map("5555-6666 rw-p 00000000 00:00 0 [heap]", 900, 810),
        ];
        let (changed, kinds) = diff(&before, &after);
        assert_eq!(changed.len(), 1, "{changed:?}");
        assert_eq!(changed[0]["mapping"], "7e6518000000 rw-p ");
        assert_eq!(changed[0]["anon_after_kb"], 35208);
        assert_eq!(kinds["anon rw-p"]["anon_kb"], 35204);
        assert!(!kinds.contains_key("[heap]"));
    }
}
