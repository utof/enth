//! Instruction budgets for the `eq` module (S2a lane EQ), via gungraun (callgrind):
//!
//! - `static_1v` / `static_16v`: a held chord of 1 and of 16 voices through one `eq` at its catalogue defaults
//!   (all six bands in the path, none moving), one second at 48 kHz each. Subtract the matching `engine` bench to read
//!   the module;
//! - `moving_16v`: the same with all twelve band controls moved by a global LFO (every band redesigns and ramps every
//!   segment, the worst case for the core), and `routes_16v`: the same routes at depth 0 (the LFO and the routing
//!   without any movement), so that `moving - routes` is the cost of the movement itself;
//! - the swap blocks on the G1 reference patch with a held 16-note chord, 64-frame blocks (`benches/swap.rs`'s
//!   method), with and without an `eq` in the chain: `swap_normal_*` is an ordinary block, `swap_crossfade_*` the
//!   block in which a crossfading swap lands (a module added elsewhere), where both plans run from that segment on and
//!   the surviving modules' state is copied. The difference between the `eq` and the base rows is the equaliser's
//!   share of the swap block: two more runs of the module and the copy of its 536 bytes of state.
//!
//! Setup is outside the measured function. Instruction counts are exact and independent of runner load; they
//! under-report subnormal penalties, so the subnormal tests (`tests/eq.rs`) stay mandatory. Needs valgrind and a
//! `gungraun-runner` of the same version as the gungraun crate (`cargo xtask bench --bench eq`).
#![allow(missing_docs, reason = "benchmark harness")]

#[path = "common/mod.rs"]
mod common;
#[path = "common/g1.rs"]
mod g1_patch;

use std::hint::black_box;

use enth_engine::{Engine, Event, EventKind, Link, LinkCapacity, NoteKey, PrepareCfg, compile};
use enth_patch::catalogue::{EqParam, LfoParam, LfoScope};
use enth_patch::{ModuleInstance, ModuleKind, Patch, Route};
use g1_patch::{g1_patch, module};
use gungraun::{library_benchmark, library_benchmark_group, main};

fn chord(voices: usize) -> common::Prepared {
    common::prepare(&common::patch_with(ModuleKind::Eq), voices, 1.0)
}

/// `patch_with(Eq)` (the module is `eq2`) plus a global sine LFO routed to every band control at `depth`.
fn routed(depth: f32) -> Patch {
    let mut p = common::patch_with(ModuleKind::Eq);
    let mut lfo = ModuleInstance::new(
        enth_patch::ModuleId::new("lfo1").expect("valid id"),
        ModuleKind::Lfo,
    );
    lfo.values[LfoParam::Scope.index()] = LfoScope::Global.index() as f32;
    lfo.values[LfoParam::Rate.index()] = 7.0;
    p.modulators.push(lfo);
    for &param in EqParam::ALL {
        if param == EqParam::Mix {
            continue;
        }
        p.routes.push(Route {
            src: "lfo1".parse().unwrap(),
            dst: format!("eq2.{}", param.spec().key).parse().unwrap(),
            depth,
        });
    }
    p
}

fn moving(voices: usize) -> common::Prepared {
    common::prepare(&routed(8.0), voices, 1.0)
}

fn routes_only(voices: usize) -> common::Prepared {
    common::prepare(&routed(0.0), voices, 1.0)
}

// Budgets: measured on x86_64 Linux (valgrind/callgrind via gungraun 0.20.0) with the pinned toolchain on 2026-10-27,
// plus 10 %, rounded up to 1,000 (docs/dsp/eq.md, "Cost"):
// - static_1v / static_16v: 18,034,747 / 51,714,587. Minus the `engine` bench measured in the same tree (3,610,649 and
//   37,290,489) that is 14,424,098 for both, 300.5 instructions per frame whatever the voice count: all six bands in
//   the path, both channels;
// - moving_16v: 63,960,661 and routes_16v: 52,135,425 (the same routes at depth 0), so moving every band control
//   costs a further 11,825,236, 246 per frame (six designs and six ramps per segment): 547 per frame in all;
// - swap_normal_base / _eq: 408,454 / 427,560 (the eq adds 19,106 per 64-frame block);
// - swap_crossfade_base / _eq: 2,494,771 / 2,534,638 (the eq adds 39,867: two plans run for two segments, and the
//   536 bytes of state are copied).
#[library_benchmark]
#[bench::static_1v(args = (1,), setup = chord, config = common::budget(STATIC_1V_IR))]
#[bench::static_16v(args = (16,), setup = chord, config = common::budget(STATIC_16V_IR))]
#[bench::moving_16v(args = (16,), setup = moving, config = common::budget(MOVING_16V_IR))]
#[bench::routes_16v(args = (16,), setup = routes_only, config = common::budget(ROUTES_16V_IR))]
fn render(p: common::Prepared) -> f32 {
    common::render(p)
}

// ---- the swap blocks (the method of benches/swap.rs, on G1 with and without an eq) ----

const SR: f32 = 48_000.0;
/// The host block: 64 frames, the G1 real-time condition.
const BLOCK: usize = 64;
/// The warm-up before the measured blocks: 0.5 s.
const WARM_FRAMES: usize = 24_000;

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

/// G1, with an `eq` (a few bands in use) at the end of the chain when `with_eq`.
fn base(with_eq: bool) -> Patch {
    let mut p = g1_patch();
    if with_eq {
        p.fx.push(module(
            "eq1",
            ModuleKind::Eq,
            &[
                (EqParam::Bell1Gain.index(), 4.0),
                (EqParam::Bell2Gain.index(), -3.0),
                (EqParam::HighGain.index(), 2.0),
            ],
        ));
    }
    p
}

/// `base` playing a held 16-note chord, warmed up for [`WARM_FRAMES`].
fn playing(with_eq: bool) -> Playing {
    let (plan, _) = compile(&base(with_eq), &PrepareCfg::new(SR)).expect("G1 compiles");
    let (mut engine, link) = Engine::with_link(plan, 0x0E17_5EED, LinkCapacity::default());
    let (mut left, mut right) = (vec![0.0; BLOCK], vec![0.0; BLOCK]);
    let notes: Vec<Event> = (0..16u8)
        .map(|i| Event {
            offset: 0,
            kind: EventKind::NoteOn {
                key: NoteKey::new(36 + (i * 5) % 60, i32::from(i)),
                velocity: 0.8,
            },
        })
        .collect();
    engine.process(&mut left, &mut right, &notes);
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

/// `playing` with an unrouted voice LFO added in the next plan: a module added, so the swap crossfades.
fn sent(with_eq: bool) -> Playing {
    let mut p = playing(with_eq);
    let mut next = base(with_eq);
    next.modulators.push(module(
        "lfo3",
        ModuleKind::Lfo,
        &[(LfoParam::Rate.index(), 2.0)],
    ));
    let (plan, _) = compile(&next, &PrepareCfg::new(SR)).expect("variant compiles");
    p.link.send_plan(plan).expect("one plan in flight");
    p
}

fn normal_base(_: ()) -> Playing {
    playing(false)
}

fn normal_eq(_: ()) -> Playing {
    playing(true)
}

fn crossfade_base(_: ()) -> Playing {
    sent(false)
}

fn crossfade_eq(_: ()) -> Playing {
    sent(true)
}

#[library_benchmark]
#[bench::swap_normal_base(args = ((),), setup = normal_base, config = common::budget(NORMAL_BASE_IR))]
#[bench::swap_normal_eq(args = ((),), setup = normal_eq, config = common::budget(NORMAL_EQ_IR))]
#[bench::swap_crossfade_base(args = ((),), setup = crossfade_base, config = common::budget(CROSSFADE_BASE_IR))]
#[bench::swap_crossfade_eq(args = ((),), setup = crossfade_eq, config = common::budget(CROSSFADE_EQ_IR))]
fn one_block(mut p: Playing) -> Playing {
    p.block();
    p
}

const STATIC_1V_IR: u64 = 19_839_000;
const STATIC_16V_IR: u64 = 56_887_000;
const MOVING_16V_IR: u64 = 70_357_000;
const ROUTES_16V_IR: u64 = 57_349_000;
const NORMAL_BASE_IR: u64 = 450_000;
const NORMAL_EQ_IR: u64 = 471_000;
const CROSSFADE_BASE_IR: u64 = 2_745_000;
const CROSSFADE_EQ_IR: u64 = 2_789_000;

library_benchmark_group!(name = eq, benchmarks = [render, one_block]);
main!(library_benchmark_groups = eq);
