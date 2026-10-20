//! Instruction budgets for the plan swap's spiky blocks (P1b lane RT), on the G1 reference patch with a held
//! 16-note chord, 64-frame blocks at 48 kHz, via gungraun (callgrind):
//!
//! - `normal`: one ordinary block, the reference the others are read against;
//! - `instant`: the block in which an instant swap lands (the same modules, one value changed: the state moves, no
//!   crossfade; `Engine::swap` with `take_state_from`);
//! - `crossfade_swap`: the block in which a crossfading swap lands (an unrouted voice LFO added): both plans run from
//!   that segment on, and the surviving modules' state, the reverb's 1.6 MB of delay lines among it, is copied;
//! - `crossfade`: the next three blocks, wholly inside the 256-frame crossfade (both plans run).
//!
//! The setup renders 0.5 s first, so the voices, the envelopes and the reverb are in their steady state, and sends the
//! plan from the bench's own thread (the session compiles and sends on the main thread; neither is measured). The
//! measured function returns the engine, so dropping it (and the plans it holds) is not counted.
//!
//! Instruction counts under-state a memory copy (a `rep movs` of 1.6 MB is cheap in instructions and costs
//! 150–250 µs of wall time), so the unit test that an instant swap moves the state
//! (`an_instant_swap_moves_the_effect_state_and_a_crossfade_copies_it`, `src/plan.rs`), not these budgets, guards
//! against a copy coming back. Wall-clock swap blocks come from `enth-test-host process-stats --swap-state`
//! (docs/benchmarks.md, "Lane RT"). Needs valgrind and a `gungraun-runner` of the same version as the gungraun crate
//! (`cargo xtask bench --bench swap`).
#![allow(missing_docs, reason = "benchmark harness")]

#[path = "common/g1.rs"]
mod g1_patch;

use std::hint::black_box;

use enth_engine::{Engine, Event, EventKind, Link, LinkCapacity, NoteKey, PrepareCfg, compile};
use enth_patch::catalogue::{LfoParam, OscParam};
use enth_patch::{ModuleKind, Patch};
use g1_patch::{g1_patch, module};
use gungraun::{
    Callgrind, EventKind as Ir, LibraryBenchmarkConfig, library_benchmark, library_benchmark_group,
    main,
};

const SR: f32 = 48_000.0;
/// The host block: 64 frames, the G1 real-time condition.
const BLOCK: usize = 64;
/// The warm-up before the measured blocks: 0.5 s.
const WARM_FRAMES: usize = 24_000;
/// Blocks wholly inside the 256-frame crossfade after the swap block.
const CROSSFADE_BLOCKS: usize = 3;

fn budget(ir: u64) -> LibraryBenchmarkConfig {
    LibraryBenchmarkConfig::default()
        .tool(Callgrind::default().hard_limits([(Ir::Ir, ir)]))
        .clone()
}

/// An engine mid-chord, its link and one block's buffers.
pub struct Playing {
    engine: Engine,
    link: Link,
    left: Vec<f32>,
    right: Vec<f32>,
}

impl Playing {
    fn block(&mut self) {
        self.engine
            .process(black_box(&mut self.left), black_box(&mut self.right), &[]);
    }
}

/// G1 playing a held 16-note chord (the bench helpers' chord), warmed up for [`WARM_FRAMES`].
fn playing() -> Playing {
    let (plan, _) = compile(&g1_patch(), &PrepareCfg::new(SR)).expect("G1 compiles");
    let (mut engine, link) = Engine::with_link(plan, 0x0E17_5EED, LinkCapacity::default());
    let chord: Vec<Event> = (0..16u8)
        .map(|i| Event {
            offset: 0,
            kind: EventKind::NoteOn {
                key: NoteKey::new(36 + (i * 5) % 60, i32::from(i)),
                velocity: 0.8,
            },
        })
        .collect();
    let (mut left, mut right) = (vec![0.0; BLOCK], vec![0.0; BLOCK]);
    engine.process(&mut left, &mut right, &chord);
    for _ in 1..WARM_FRAMES / BLOCK {
        engine.process(&mut left, &mut right, &[]);
    }
    Playing {
        engine,
        link,
        left,
        right,
    }
}

/// G1 with osc1's level at 95 %: the same modules, so the swap is instant.
fn value_edit() -> Patch {
    let mut p = g1_patch();
    p.generators[0].values[OscParam::Level.index()] = 95.0;
    p
}

/// G1 with an unrouted voice LFO added: a module added, so the swap crossfades (research 03's `g1_plus_lfo`).
fn module_added() -> Patch {
    let mut p = g1_patch();
    p.modulators.push(module(
        "lfo3",
        ModuleKind::Lfo,
        &[(LfoParam::Rate.index(), 2.0)],
    ));
    p
}

/// Playing, with `patch`'s plan sent: the next block swaps it in at its first segment.
fn sent(patch: &Patch) -> Playing {
    let mut p = playing();
    let (plan, _) = compile(patch, &PrepareCfg::new(SR)).expect("variant compiles");
    p.link.send_plan(plan).expect("one plan in flight");
    p
}

fn normal_setup(_: ()) -> Playing {
    playing()
}

fn instant_setup(_: ()) -> Playing {
    sent(&value_edit())
}

fn crossfade_swap_setup(_: ()) -> Playing {
    sent(&module_added())
}

/// The crossfading swap has landed (its block rendered in the setup).
fn crossfading_setup(_: ()) -> Playing {
    let mut p = sent(&module_added());
    p.block();
    assert!(p.engine.is_crossfading(), "the swap crossfades");
    p
}

// Budgets: measured on x86_64 Linux (Intel i7-9750H, valgrind/callgrind via gungraun 0.20.0) with the pinned toolchain
// on 2026-10-20 (lane RT, main aa0fec0), plus 10 %, rounded up to 1,000:
// - normal: 407,853 (6,373 per frame);
// - instant: 444,905 (+9 % on a normal block: `carry_from` pairs the modules, the reverb's core is exchanged);
// - crossfade_swap: 2,489,249 (6.1×: both plans run two segments and the surviving state is copied; callgrind counts
//   the copy's memory traffic, 6.0 M L1 hits against 0.55 M);
// - crossfade: 2,407,206 for three blocks, 802,402 each (2.0×: both plans run).
#[library_benchmark]
#[bench::normal(args = ((),), setup = normal_setup, config = budget(NORMAL_IR))]
#[bench::instant(args = ((),), setup = instant_setup, config = budget(INSTANT_IR))]
#[bench::crossfade_swap(args = ((),), setup = crossfade_swap_setup, config = budget(CROSSFADE_SWAP_IR))]
fn one_block(mut p: Playing) -> Playing {
    p.block();
    p
}

#[library_benchmark]
#[bench::crossfade(args = ((),), setup = crossfading_setup, config = budget(CROSSFADE_IR))]
fn crossfade_blocks(mut p: Playing) -> Playing {
    for _ in 0..CROSSFADE_BLOCKS {
        p.block();
    }
    p
}

const NORMAL_IR: u64 = 449_000;
const INSTANT_IR: u64 = 490_000;
const CROSSFADE_SWAP_IR: u64 = 2_739_000;
const CROSSFADE_IR: u64 = 2_648_000;

library_benchmark_group!(name = swap, benchmarks = [one_block, crossfade_blocks]);
main!(library_benchmark_groups = swap);
