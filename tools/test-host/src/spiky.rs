//! Spiky blocks for `process-stats` (P1b lane RT): structural edits that swap the engine's plan, and event floods.
//!
//! # Swaps (`--swap-state FILE --swap-ms 250 --swap-kind instant|crossfade`)
//!
//! The host's main thread loads `--swap-state` and `--state` alternately through the CLAP state extension, every
//! `--swap-ms`, while the audio thread plays: what a host does on a preset recall, and the path an editor edit takes
//! too (enth-plugin's `Session::load_patch` compiles the patch on the calling thread and sends the plan; the engine
//! swaps it in at its next segment boundary). The host cannot see which block that is, so each load is bracketed by
//! its call's start and end, and its **window** is every block that overlapped the call plus the first block that
//! started after it. The swap block is always in the window: the plan is sent inside the call, and the engine polls
//! for plans at every segment.
//!
//! - `instant`: the two states have the same modules (a value differs), so the engine moves the state and does not
//!   crossfade. The swap block's figure is **the window's largest block**, an upper bound on the real one.
//! - `crossfade`: a module is added or removed, so both plans run for 256 frames and a surviving reverb's delay lines
//!   are copied in the swap block. The swap block is the window's largest block, and the **crossfade blocks** are the
//!   ones after it that lie wholly inside the 256 frames (three at 64-frame blocks, wherever in the swap block the
//!   swap fell).
//!
//! The other blocks of a window are left out of every class; all remaining blocks are **normal**. `profile` gives
//! the plugin's time at each offset from the swap block, so the classification can be checked by eye: a crossfade
//! shows one tall block and three raised ones.
//!
//! # Floods (`--flood N --flood-ms 250`)
//!
//! Every `--flood-ms`, one block carries N note events: note-on and note-off pairs (each off one frame after its on)
//! spread over the block's frames, so dozens share a frame, on keys 24–103 with fresh note ids. The pairs steal every
//! voice, so the block's last frame presses the run's opening chord again (16 more note-ons for the G1 chord) and the
//! voices between floods play what they played before. N above enth-plugin's event list (1,096) makes the plugin
//! render the block in several engine calls; N above 1,024 makes nice-plug grow its input queue once, inside its
//! wrapper. The flood block and the block after it are reported apart from the normal blocks.

use std::sync::atomic::Ordering;
use std::time::Duration;

use clack_extensions::state::PluginState;
use clack_host::prelude::PluginInstance;
use serde_json::{Value, json};

use crate::host::Host;
use crate::notes::{Kind, Schedule, Scheduled};
use crate::stats::{dist, now_ns};

/// The engine's plan-swap crossfade, in frames (`enth_engine::CROSSFADE_FRAMES`; the test host does not link the
/// engine, so the value is restated here).
pub const CROSSFADE_FRAMES: u32 = 256;

/// What the two states of a swap run differ by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SwapKind {
    /// The same modules: the engine moves the state, no crossfade.
    Instant,
    /// A module added or removed: a 256-frame crossfade, and a surviving reverb's state is copied.
    Crossfade,
}

impl SwapKind {
    pub fn parse(text: &str) -> Result<Self, String> {
        match text {
            "instant" => Ok(Self::Instant),
            "crossfade" => Ok(Self::Crossfade),
            other => Err(format!("--swap-kind {other:?}: use instant or crossfade")),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Instant => "instant",
            Self::Crossfade => "crossfade",
        }
    }
}

/// A swap run: two prepared state blobs, loaded alternately.
#[derive(Clone, Debug)]
pub struct Swap {
    /// `--state`, loaded on every second load (the run starts with it).
    pub base_state: Vec<u8>,
    /// `--swap-state`, loaded first.
    pub alt_state: Vec<u8>,
    pub every_ms: u64,
    pub kind: SwapKind,
}

/// One state load on the main thread: when the call started and returned (`CLOCK_MONOTONIC` ns).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Load {
    pub start: u64,
    pub end: u64,
}

/// Loads `swap`'s two states alternately every `every_ms`, from `begin_ns` until `done()`, servicing the plugin's
/// main-thread callback requests in between. Returns the loads made and the first error, if any.
pub fn drive_swaps(
    instance: &mut PluginInstance<Host>,
    swap: &Swap,
    begin_ns: u64,
    done: &dyn Fn() -> bool,
) -> (Vec<Load>, Option<String>) {
    let period = swap.every_ms.max(1) * 1_000_000;
    let mut loads = Vec::new();
    let mut next = begin_ns;
    while !done() {
        let now = now_ns();
        if now < next {
            std::thread::sleep(Duration::from_nanos((next - now).min(5_000_000)));
            service(instance);
            continue;
        }
        let bytes = if loads.len() % 2 == 0 {
            &swap.alt_state
        } else {
            &swap.base_state
        };
        let start = now_ns();
        let result = {
            let handle = instance.plugin_handle();
            match handle.get_extension::<PluginState>() {
                Some(ext) => ext
                    .load(&handle, &mut &bytes[..])
                    .map_err(|e| format!("the plugin refused a swap state: {e}")),
                None => Err("--swap-state given, but the plugin has no state extension".to_owned()),
            }
        };
        let end = now_ns();
        if let Err(e) = result {
            return (loads, Some(e));
        }
        loads.push(Load { start, end });
        service(instance);
        next += period;
        if next < end {
            next = end + period;
        }
    }
    (loads, None)
}

fn service(instance: &mut PluginInstance<Host>) {
    if instance.access_shared_handler(|s| s.callback_requested.swap(false, Ordering::SeqCst)) {
        instance.call_on_main_thread_callback();
    }
}

/// One audio thread's measured blocks, for the classifiers. All slices have one entry per block.
pub struct Blocks<'a> {
    /// When the block's callback started and ended (`CLOCK_MONOTONIC` ns).
    pub starts: &'a [u64],
    pub ends: &'a [u64],
    /// The plugin's `process()` wall time (ns) of the thread's instance.
    pub plugin: &'a [u64],
    /// The callback's wall time (ns).
    pub callback: &'a [u64],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Class {
    Normal,
    /// In a load's window, but not its swap block.
    Window,
    Swap,
    Crossfade,
}

/// The blocks of a swap run, classified.
#[derive(Debug, Default)]
pub struct SwapClasses {
    /// Per classified load: the swap block's index.
    pub swap_blocks: Vec<usize>,
    pub crossfade_blocks: Vec<usize>,
    pub normal_blocks: Vec<usize>,
    pub window_sizes: Vec<u64>,
    /// Loads outside the measured blocks (during the warm-up, or after the last block).
    pub skipped: usize,
}

/// Full blocks of `block` frames that lie inside the crossfade after the swap block, wherever the swap fell in it.
pub fn crossfade_blocks_after(block: u32) -> usize {
    (CROSSFADE_FRAMES / block.max(1)).saturating_sub(1) as usize
}

/// Classifies the blocks of a swap run (see the module docs).
pub fn classify_swaps(
    kind: SwapKind,
    loads: &[Load],
    blocks: &Blocks<'_>,
    block: u32,
) -> SwapClasses {
    let n = blocks.starts.len();
    let mut class = vec![Class::Normal; n];
    let mut out = SwapClasses::default();
    let after = match kind {
        SwapKind::Instant => 0,
        SwapKind::Crossfade => crossfade_blocks_after(block),
    };
    for load in loads {
        if n == 0 || load.start < blocks.starts[0] {
            out.skipped += 1;
            continue;
        }
        // The first block still running at (or starting after) the call's start, and the first block that started
        // after the call returned.
        let lo = blocks.ends.partition_point(|&e| e < load.start);
        let hi = blocks.starts.partition_point(|&s| s <= load.end);
        if hi + after >= n {
            out.skipped += 1;
            continue;
        }
        let swap = (lo..=hi)
            .max_by_key(|&i| (blocks.plugin[i], std::cmp::Reverse(i)))
            .unwrap_or(hi);
        out.window_sizes.push((hi - lo + 1) as u64);
        for c in &mut class[lo..=hi] {
            if *c == Class::Normal {
                *c = Class::Window;
            }
        }
        class[swap] = Class::Swap;
        out.swap_blocks.push(swap);
        for c in &mut class[swap + 1..=swap + after] {
            *c = Class::Crossfade;
        }
    }
    for (i, c) in class.iter().enumerate() {
        match c {
            Class::Normal => out.normal_blocks.push(i),
            Class::Crossfade => out.crossfade_blocks.push(i),
            Class::Window | Class::Swap => {}
        }
    }
    out
}

fn pick(values: &[u64], at: &[usize]) -> Vec<u64> {
    at.iter().map(|&i| values[i]).collect()
}

/// The `swaps` section of the report, and the swap block's and the crossfade blocks' plugin p99 (µs).
pub fn swap_report(
    swap: &Swap,
    loads: &[Load],
    blocks: &Blocks<'_>,
    block: u32,
) -> (Value, f64, f64) {
    let classes = classify_swaps(swap.kind, loads, blocks, block);
    let swap_plugin = pick(blocks.plugin, &classes.swap_blocks);
    let crossfade_plugin = pick(blocks.plugin, &classes.crossfade_blocks);
    // The plugin's time at each offset from the swap block.
    let profile: Vec<Value> = (-2i64..=6)
        .map(|offset| {
            let at: Vec<usize> = classes
                .swap_blocks
                .iter()
                .filter_map(|&s| usize::try_from(s as i64 + offset).ok())
                .filter(|&i| i < blocks.plugin.len())
                .collect();
            let d = dist(&pick(blocks.plugin, &at));
            json!({ "offset": offset, "p50_us": d["p50_us"], "p99_us": d["p99_us"], "max_us": d["max_us"] })
        })
        .collect();
    let load_call: Vec<u64> = loads.iter().map(|l| l.end - l.start).collect();
    let swap_dist = dist(&swap_plugin);
    let crossfade_dist = dist(&crossfade_plugin);
    let swap_p99 = swap_dist["p99_us"].as_f64().unwrap_or(0.0);
    let crossfade_p99 = crossfade_dist["p99_us"].as_f64().unwrap_or(0.0);
    let report = json!({
        "kind": swap.kind.name(),
        "every_ms": swap.every_ms,
        "loads": loads.len(),
        "loads_classified": classes.swap_blocks.len(),
        "loads_skipped": classes.skipped,
        "load_call": dist(&load_call),
        "window_blocks": dist(&classes.window_sizes.iter().map(|n| n * 1000).collect::<Vec<_>>()),
        "window_blocks_note": "blocks per load window, reported in thousandths (p50_us 2.0 = 2 blocks)",
        "swap_block": swap_dist,
        "swap_block_callback": dist(&pick(blocks.callback, &classes.swap_blocks)),
        "crossfade_blocks_after_swap": match swap.kind {
            SwapKind::Instant => 0,
            SwapKind::Crossfade => crossfade_blocks_after(block),
        },
        "crossfade_blocks": crossfade_dist,
        "crossfade_blocks_callback": dist(&pick(blocks.callback, &classes.crossfade_blocks)),
        "normal_blocks": dist(&pick(blocks.plugin, &classes.normal_blocks)),
        "profile": profile,
        "method": "plugin process() times; a load's window is every block that overlapped the state.load call plus the first block after it; the swap block is the window's largest (an upper bound for an instant swap); crossfade blocks are the full blocks after it inside the 256-frame crossfade",
    });
    (report, swap_p99, crossfade_p99)
}

/// An event flood every `every_ms`: `events` note events in one block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Flood {
    pub events: usize,
    pub every_ms: u64,
}

/// Where the floods are: runner block `first + k · every` (runner blocks count from the warm-up's first block).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FloodPlan {
    pub first: u64,
    pub every: u64,
    pub count: u64,
    /// Events in one flood block: the note events plus the chord pressed again.
    pub events_per_block: usize,
}

impl FloodPlan {
    pub fn is_flood(&self, block: u64) -> bool {
        block >= self.first && (block - self.first).is_multiple_of(self.every)
    }
}

/// A deterministic generator: the same plan always gives the same floods.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0 >> 33
    }
}

/// The first note id a flood uses (the notes script numbers its notes from 0).
const FLOOD_ID: u32 = 1_000_000;

/// Adds floods to `schedule` for a run of `total_blocks` blocks of `block` frames at `sample_rate`. The chord that
/// is pressed again is the schedule's note-ons in its first block.
pub fn add_floods(
    schedule: &mut Schedule,
    flood: Flood,
    block: u32,
    sample_rate: f64,
    total_blocks: u64,
) -> FloodPlan {
    let b = u64::from(block);
    let chord: Vec<(u16, f64)> = schedule
        .window(0, b)
        .iter()
        .filter_map(|e| match e.kind {
            Kind::NoteOn { key, velocity, .. } => Some((key, velocity)),
            _ => None,
        })
        .collect();
    let every =
        ((flood.every_ms as f64 / 1000.0 * sample_rate / f64::from(block)).round() as u64).max(2);
    // The first flood half a period in, clear of the chord's own first block.
    let first = (every / 2).max(1);
    let count = if total_blocks > first {
        (total_blocks - 1 - first) / every + 1
    } else {
        0
    };
    let pairs = flood.events / 2;
    let mut rng = Lcg(0x0E17_5EED ^ flood.events as u64);
    let mut id = FLOOD_ID;
    let mut events: Vec<Scheduled> =
        Vec::with_capacity((count as usize) * (pairs * 2 + chord.len()));
    for k in 0..count {
        let start = (first + k * every) * b;
        let last = start + b - 1;
        for p in 0..pairs {
            let frame = start + (p as u64 * 2 * b) / (pairs as u64 * 2).max(1);
            let key = 24 + (rng.next() % 80) as u16;
            let velocity = f64::from(1 + (rng.next() % 8) as u32) / 8.0;
            events.push(Scheduled {
                frame,
                kind: Kind::NoteOn {
                    key,
                    note_id: id,
                    velocity,
                },
            });
            events.push(Scheduled {
                frame: (frame + 1).min(last),
                kind: Kind::NoteOff { key, note_id: id },
            });
            id = id.wrapping_add(1);
        }
        for &(key, velocity) in &chord {
            events.push(Scheduled {
                frame: last,
                kind: Kind::NoteOn {
                    key,
                    note_id: id,
                    velocity,
                },
            });
            id = id.wrapping_add(1);
        }
    }
    schedule.events.append(&mut events);
    schedule.events.sort_by_key(|e| (e.frame, e.kind.order()));
    FloodPlan {
        first,
        every,
        count,
        events_per_block: pairs * 2 + chord.len(),
    }
}

/// The `floods` section of the report, and the flood blocks' plugin p99 (µs). Measured block `i` is runner block
/// `warm_blocks + i`.
pub fn flood_report(
    flood: Flood,
    plan: &FloodPlan,
    warm_blocks: u64,
    blocks: &Blocks<'_>,
) -> (Value, f64) {
    let (mut flooded, mut after, mut normal) = (Vec::new(), Vec::new(), Vec::new());
    for i in 0..blocks.plugin.len() {
        let b = warm_blocks + i as u64;
        if plan.is_flood(b) {
            flooded.push(i);
        } else if b > 0 && plan.is_flood(b - 1) {
            after.push(i);
        } else {
            normal.push(i);
        }
    }
    let flood_dist = dist(&pick(blocks.plugin, &flooded));
    let p99 = flood_dist["p99_us"].as_f64().unwrap_or(0.0);
    let report = json!({
        "note_events": flood.events / 2 * 2,
        "events_per_block": plan.events_per_block,
        "every_ms": flood.every_ms,
        "every_blocks": plan.every,
        "floods_in_run": plan.count,
        "flood_blocks": flood_dist,
        "flood_blocks_callback": dist(&pick(blocks.callback, &flooded)),
        "after_flood_blocks": dist(&pick(blocks.plugin, &after)),
        "normal_blocks": dist(&pick(blocks.plugin, &normal)),
        "method": "plugin process() times; flood blocks are known exactly (the schedule is fixed); the first flood also pays nice-plug's one-time input-queue growth",
    });
    (report, p99)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Block i starts at 1,000 + 1,000·i ns and lasts 100 ns, except block 5 (900 ns) and blocks 6–8 (300 ns).
    fn blocks(n: usize) -> (Vec<u64>, Vec<u64>, Vec<u64>) {
        let starts: Vec<u64> = (0..n as u64).map(|i| 1000 + i * 1000).collect();
        let plugin: Vec<u64> = (0..n)
            .map(|i| match i {
                5 => 900,
                6..=8 => 300,
                _ => 100,
            })
            .collect();
        let ends = starts.iter().zip(&plugin).map(|(s, p)| s + p).collect();
        (starts, ends, plugin)
    }

    #[test]
    fn a_crossfade_window_finds_the_tall_block_and_the_three_after_it() {
        let (starts, ends, plugin) = blocks(20);
        let b = Blocks {
            starts: &starts,
            ends: &ends,
            plugin: &plugin,
            callback: &plugin,
        };
        // A load from 4,500 to 5,050 ns: block 3 ended before it, block 4 (5,000–5,100) overlaps it, block 5 is the
        // first block after it.
        let loads = [Load {
            start: 4500,
            end: 5050,
        }];
        let c = classify_swaps(SwapKind::Crossfade, &loads, &b, 64);
        assert_eq!(c.swap_blocks, [5]);
        assert_eq!(c.crossfade_blocks, [6, 7, 8]);
        assert_eq!(c.window_sizes, [2]);
        for i in [4, 5, 6, 7, 8] {
            assert!(!c.normal_blocks.contains(&i), "block {i}");
        }
        assert!(c.normal_blocks.contains(&3) && c.normal_blocks.contains(&9));
        let (report, swap_p99, crossfade_p99) = swap_report(
            &Swap {
                base_state: Vec::new(),
                alt_state: Vec::new(),
                every_ms: 250,
                kind: SwapKind::Crossfade,
            },
            &loads,
            &b,
            64,
        );
        assert!((swap_p99 - 0.9).abs() < 1e-9);
        assert!((crossfade_p99 - 0.3).abs() < 1e-9);
        assert_eq!(report["loads_classified"], 1);
    }

    #[test]
    fn an_instant_window_takes_its_largest_block_and_loads_outside_the_run_are_skipped() {
        let (starts, ends, plugin) = blocks(10);
        let b = Blocks {
            starts: &starts,
            ends: &ends,
            plugin: &plugin,
            callback: &plugin,
        };
        let loads = [
            // Before the first block: the warm-up.
            Load { start: 10, end: 20 },
            // Blocks 2 (overlapping) and 3 (the first after).
            Load {
                start: 3050,
                end: 3200,
            },
            // Past the last block.
            Load {
                start: 50_000,
                end: 50_100,
            },
        ];
        let c = classify_swaps(SwapKind::Instant, &loads, &b, 64);
        assert_eq!(c.skipped, 2);
        assert_eq!(c.window_sizes, [2]);
        assert_eq!(c.swap_blocks, [2], "a tie goes to the earlier block");
        assert_eq!(c.crossfade_blocks, Vec::<usize>::new());
        assert!(
            !c.normal_blocks.contains(&3),
            "the window's other block is not normal"
        );
    }

    #[test]
    fn crossfade_blocks_after_the_swap_depend_on_the_block_size() {
        assert_eq!(crossfade_blocks_after(64), 3);
        assert_eq!(crossfade_blocks_after(32), 7);
        assert_eq!(crossfade_blocks_after(128), 1);
        assert_eq!(crossfade_blocks_after(256), 0);
        assert_eq!(crossfade_blocks_after(4096), 0);
    }

    #[test]
    fn floods_land_on_their_blocks_with_every_event_inside_and_the_chord_pressed_again() {
        let mut schedule = Schedule {
            events: vec![
                Scheduled {
                    frame: 0,
                    kind: Kind::NoteOn {
                        key: 36,
                        note_id: 0,
                        velocity: 0.8,
                    },
                },
                Scheduled {
                    frame: 0,
                    kind: Kind::NoteOn {
                        key: 43,
                        note_id: 1,
                        velocity: 0.8,
                    },
                },
            ],
        };
        // 100 ms at 48 kHz and 64-frame blocks: every 75 blocks, the first at block 37.
        let plan = add_floods(
            &mut schedule,
            Flood {
                events: 3000,
                every_ms: 100,
            },
            64,
            48_000.0,
            300,
        );
        assert_eq!((plan.first, plan.every, plan.count), (37, 75, 4));
        assert_eq!(plan.events_per_block, 3002);
        assert!(
            plan.is_flood(37) && plan.is_flood(262) && !plan.is_flood(38) && !plan.is_flood(36)
        );
        for k in 0..plan.count {
            let start = (plan.first + k * plan.every) * 64;
            let window = schedule.window(start, start + 64);
            assert_eq!(window.len(), 3002, "flood {k}");
            let ons = window
                .iter()
                .filter(|e| matches!(e.kind, Kind::NoteOn { .. }))
                .count();
            assert_eq!(ons, 1502);
            let last: Vec<_> = window.iter().filter(|e| e.frame == start + 63).collect();
            assert!(
                last.iter()
                    .any(|e| matches!(e.kind, Kind::NoteOn { key: 43, .. }))
            );
        }
        assert!(schedule.events.windows(2).all(|w| w[0].frame <= w[1].frame));
    }

    #[test]
    fn a_flood_report_separates_flood_after_and_normal_blocks() {
        let plan = FloodPlan {
            first: 3,
            every: 4,
            count: 3,
            events_per_block: 10,
        };
        // Warm-up of 2 blocks: measured blocks 0..10 are runner blocks 2..12; floods at runner 3, 7, 11.
        let plugin: Vec<u64> = (2..12u64)
            .map(|b| if plan.is_flood(b) { 5000 } else { 1000 })
            .collect();
        let starts: Vec<u64> = (0..10).collect();
        let b = Blocks {
            starts: &starts,
            ends: &starts,
            plugin: &plugin,
            callback: &plugin,
        };
        let (report, p99) = flood_report(
            Flood {
                events: 8,
                every_ms: 1,
            },
            &plan,
            2,
            &b,
        );
        assert_eq!(report["flood_blocks"]["n"], 3);
        assert_eq!(
            report["after_flood_blocks"]["n"], 2,
            "runner 4 and 8; 12 is past the run"
        );
        assert_eq!(report["normal_blocks"]["n"], 5);
        assert!((p99 - 5.0).abs() < 1e-9);
    }
}
