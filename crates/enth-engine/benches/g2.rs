//! Instruction budget for the G2 reference patch (S2a lane G2B): a dubstep growl voice, `wt` with 7-copy unison and
//! PM from a second generator, `dist` into a `ladder` and a `formant` (a synced LFO and an MSEG moving them), and
//! `ott`, `chorus` and `delay` in the FX chain; 8 voices (the default, docs/phase-s2.md "Open checks"), a held
//! eight-note chord in the bass register, one 2 s render in 512-frame host calls, via gungraun (callgrind). The patch
//! is `common/g2.rs`, whose header says what it holds and why.
//!
//! G2's target is G1's cap of 12,000 instructions per stereo frame (`g1.rs`). The budget below is the measured count
//! plus 10 %; docs/benchmarks.md ("The G2 row") has the numbers, the cost of each choice and the options when the
//! reference does not fit the cap.
//!
//! - `reference_8v`: the reference patch.
//! - the variants show what each choice costs: `dist_off` (ADAA-1 without oversampling), `dist_x4`, `dist_soft` and
//!   `dist_hard` (other curves), `static_position` (the MSEG not on the table position), `no_ladder`,
//!   `no_formant`, `copies_3` (3 unison copies), `mono_1_copy` (no unison, so no stereo voice path), `voices_6`,
//!   `voices_6_hard`, `voices_4`.
//! - the swap blocks (64 frames, the G1 real-time condition), as in `swap.rs` on the G2 patch: `normal`, `instant`
//!   (a value edit), `crossfade_swap` (an unrouted LFO added: both plans run from that segment and the surviving
//!   modules' state is copied) and `crossfade` (the next three blocks).
//!
//! Instruction counts under-state a memory copy, so the wall-clock swap blocks are `enth-test-host process-stats
//! --swap-state` (docs/benchmarks.md). Needs valgrind and a `gungraun-runner` of the same version as the gungraun
//! crate (`cargo xtask bench --bench g2`).
#![allow(missing_docs, reason = "benchmark harness")]

#[path = "common/mod.rs"]
mod common;

#[path = "common/g2.rs"]
mod g2_patch;

use std::hint::black_box;

use enth_engine::{Engine, Event, EventKind, Link, LinkCapacity, NoteKey, PrepareCfg, compile};
use enth_patch::Patch;
use enth_patch::catalogue::{DistMode, Oversample};
use g2_patch::{G2, KEYS, module_added, value_edit};
use gungraun::{library_benchmark, library_benchmark_group, main};

const SR: f32 = common::SR;
/// Frames per steady render: 2 s at 48 kHz, as G1's.
const FRAMES: usize = 96_000;
/// The host block of the swap cases: 64 frames, the G1 real-time condition.
const BLOCK: usize = 64;
/// The warm-up before the measured swap blocks: 0.5 s.
const WARM_FRAMES: usize = 24_000;
/// Blocks wholly inside the 256-frame crossfade after the swap block.
const CROSSFADE_BLOCKS: usize = 3;

/// The held chord: `KEYS` at frame 0, the first `voices` of them, velocity 0.8.
fn chord(voices: usize, first_id: i32) -> Vec<Event> {
    KEYS.iter()
        .cycle()
        .take(voices)
        .enumerate()
        .map(|(i, &key)| Event {
            offset: 0,
            kind: EventKind::NoteOn {
                key: NoteKey::new(key, first_id + i as i32),
                velocity: 0.8,
            },
        })
        .collect()
}

/// An engine with its output buffers and the chord of the steady render.
pub struct Steady {
    engine: Engine,
    left: Vec<f32>,
    right: Vec<f32>,
    events: Vec<Event>,
}

fn steady(patch: &Patch, voices: usize) -> Steady {
    let (plan, _) = compile(patch, &PrepareCfg::new(SR)).expect("G2 compiles");
    Steady {
        engine: Engine::new(plan, 0x0E17_5EED),
        left: vec![0.0; FRAMES],
        right: vec![0.0; FRAMES],
        events: chord(voices, 0),
    }
}

/// Render in [`common::BLOCK`]-frame host calls (the measured part).
fn render(mut p: Steady) -> f32 {
    let mut first = true;
    for (l, r) in p
        .left
        .chunks_mut(common::BLOCK)
        .zip(p.right.chunks_mut(common::BLOCK))
    {
        let events: &[Event] = if first { &p.events } else { &[] };
        first = false;
        p.engine.process(black_box(l), black_box(r), events);
    }
    black_box(p.left[p.left.len() / 2] + p.right[p.right.len() / 3])
}

fn variant(g: G2) -> Steady {
    steady(&g.patch(), g.voices as usize)
}

fn reference(_: ()) -> Steady {
    variant(G2::default())
}

fn dist_off(_: ()) -> Steady {
    variant(G2 {
        oversample: Oversample::Off,
        ..G2::default()
    })
}

fn dist_x4(_: ()) -> Steady {
    variant(G2 {
        oversample: Oversample::X4,
        ..G2::default()
    })
}

fn dist_soft(_: ()) -> Steady {
    variant(G2 {
        dist_mode: DistMode::Soft,
        ..G2::default()
    })
}

fn dist_hard(_: ()) -> Steady {
    variant(G2 {
        dist_mode: DistMode::Hard,
        ..G2::default()
    })
}

fn static_position(_: ()) -> Steady {
    variant(G2 {
        position_mod: false,
        ..G2::default()
    })
}

fn no_ladder(_: ()) -> Steady {
    variant(G2 {
        ladder: false,
        ..G2::default()
    })
}

fn no_formant(_: ()) -> Steady {
    variant(G2 {
        formant: false,
        ..G2::default()
    })
}

fn copies_3(_: ()) -> Steady {
    variant(G2 {
        copies: 3,
        ..G2::default()
    })
}

fn mono_1_copy(_: ()) -> Steady {
    variant(G2 {
        copies: 1,
        ..G2::default()
    })
}

fn voices_6(_: ()) -> Steady {
    variant(G2 {
        voices: 6,
        ..G2::default()
    })
}

fn voices_6_hard(_: ()) -> Steady {
    variant(G2 {
        voices: 6,
        dist_mode: DistMode::Hard,
        ..G2::default()
    })
}

fn voices_4(_: ()) -> Steady {
    variant(G2 {
        voices: 4,
        ..G2::default()
    })
}

// Budgets: the measured count plus 10 %, rounded up to 1,000, from x86_64 Linux (Intel i7-9750H, valgrind/callgrind
// via gungraun 0.20.0) with the pinned toolchain, 2026-10-27; docs/benchmarks.md ("The G2 row") has the table, the
// per-frame figures and what each variant shows.
#[library_benchmark]
#[bench::reference_8v(args = ((),), setup = reference, config = common::budget(REFERENCE_IR))]
#[bench::dist_off(args = ((),), setup = dist_off, config = common::budget(DIST_OFF_IR))]
#[bench::dist_x4(args = ((),), setup = dist_x4, config = common::budget(DIST_X4_IR))]
#[bench::dist_soft(args = ((),), setup = dist_soft, config = common::budget(DIST_SOFT_IR))]
#[bench::dist_hard(args = ((),), setup = dist_hard, config = common::budget(DIST_HARD_IR))]
#[bench::static_position(args = ((),), setup = static_position, config = common::budget(STATIC_POSITION_IR))]
#[bench::no_ladder(args = ((),), setup = no_ladder, config = common::budget(NO_LADDER_IR))]
#[bench::no_formant(args = ((),), setup = no_formant, config = common::budget(NO_FORMANT_IR))]
#[bench::copies_3(args = ((),), setup = copies_3, config = common::budget(COPIES_3_IR))]
#[bench::mono_1_copy(args = ((),), setup = mono_1_copy, config = common::budget(MONO_IR))]
#[bench::voices_6(args = ((),), setup = voices_6, config = common::budget(VOICES_6_IR))]
#[bench::voices_6_hard(args = ((),), setup = voices_6_hard, config = common::budget(VOICES_6_HARD_IR))]
#[bench::voices_4(args = ((),), setup = voices_4, config = common::budget(VOICES_4_IR))]
fn render_2s(p: Steady) -> f32 {
    render(p)
}

/// An engine mid-chord, its link, and one block's buffers.
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

/// G2 playing its held 8-note chord, warmed up for [`WARM_FRAMES`].
fn playing() -> Playing {
    let (plan, _) = compile(&g2_patch::g2_patch(), &PrepareCfg::new(SR)).expect("G2 compiles");
    let (mut engine, link) = Engine::with_link(plan, 0x0E17_5EED, LinkCapacity::default());
    let (mut left, mut right) = (vec![0.0; BLOCK], vec![0.0; BLOCK]);
    engine.process(&mut left, &mut right, &chord(8, 0));
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

#[library_benchmark]
#[bench::normal(args = ((),), setup = normal_setup, config = common::budget(NORMAL_IR))]
#[bench::instant(args = ((),), setup = instant_setup, config = common::budget(INSTANT_IR))]
#[bench::crossfade_swap(args = ((),), setup = crossfade_swap_setup, config = common::budget(CROSSFADE_SWAP_IR))]
fn one_block(mut p: Playing) -> Playing {
    p.block();
    p
}

#[library_benchmark]
#[bench::crossfade(args = ((),), setup = crossfading_setup, config = common::budget(CROSSFADE_IR))]
fn crossfade_blocks(mut p: Playing) -> Playing {
    for _ in 0..CROSSFADE_BLOCKS {
        p.block();
    }
    p
}

// Measured 2026-10-27 on main 6c3fc20 plus this lane, before `delay` merged (x86_64 Linux, Intel i7-9750H,
// callgrind via gungraun 0.20.0): instructions of the 2 s render, per frame (/96,000) in brackets.
// - reference_8v 1,872,632,942 (19,507); dist_off 1,481,519,510 (15,433); dist_x4 2,497,974,592 (26,021);
// - dist_soft 1,866,840,693 (19,446); dist_hard 1,435,691,370 (14,955); static_position 1,845,584,281 (19,225);
// - no_ladder 1,751,600,820 (18,246); no_formant 1,729,506,000 (18,016); copies_3 1,789,366,250 (18,639);
// - mono_1_copy 909,539,748 (9,474); voices_6 1,439,213,212 (14,992); voices_4 1,003,498,461 (10,453);
// - the 64-frame blocks: normal 1,247,086; instant 1,339,469; crossfade_swap 2,614,925; crossfade (three blocks)
//   7,497,733.
const REFERENCE_IR: u64 = 2_059_897_000;
const DIST_OFF_IR: u64 = 1_629_672_000;
const DIST_X4_IR: u64 = 2_747_773_000;
const DIST_SOFT_IR: u64 = 2_053_525_000;
const DIST_HARD_IR: u64 = 1_579_261_000;
const STATIC_POSITION_IR: u64 = 2_030_143_000;
const NO_LADDER_IR: u64 = 1_926_761_000;
const NO_FORMANT_IR: u64 = 1_902_457_000;
const COPIES_3_IR: u64 = 1_968_303_000;
const MONO_IR: u64 = 1_000_494_000;
const VOICES_6_IR: u64 = 1_583_135_000;
const VOICES_6_HARD_IR: u64 = 4_000_000_000;
const VOICES_4_IR: u64 = 1_103_849_000;
const NORMAL_IR: u64 = 1_372_000;
const INSTANT_IR: u64 = 1_474_000;
const CROSSFADE_SWAP_IR: u64 = 2_877_000;
const CROSSFADE_IR: u64 = 8_248_000;

library_benchmark_group!(name = g2_render, benchmarks = [render_2s]);
library_benchmark_group!(name = g2_swap, benchmarks = [one_block, crossfade_blocks]);
main!(library_benchmark_groups = g2_render, g2_swap);
