//! S2a's reserved kinds (lane S0d, freeze F0m) through the plan compiler and the segment loop: each compiles in its
//! section beside `Patch::init()` and renders finite samples without allocating in `process`, so the dispatch arms
//! the module lanes inherit are exercised end to end (the registry test in `modules/mod.rs` covers the roles, and
//! `tests/swap.rs` the state carry, for every kind). This holds whatever a lane puts in its stub.
#![cfg(debug_assertions)]

mod common;

use assert_no_alloc::{AllocDisabler, assert_no_alloc};
use common::{SEED, engine, events, id, note};
use enth_engine::{Event, SEG};
use enth_patch::{ModuleInstance, ModuleKind, Patch};

#[global_allocator]
static ALLOCATOR: AllocDisabler = AllocDisabler;

#[test]
fn every_reserved_kind_compiles_and_renders_without_allocating() {
    let reserved: Vec<ModuleKind> = ModuleKind::ALL
        .iter()
        .copied()
        .filter(|k| k.is_hidden())
        .collect();
    assert_eq!(reserved.len(), 11);
    for kind in reserved {
        let mut p = Patch::init();
        let m = ModuleInstance::new(id(&format!("{}1", kind.spec().id_prefix)), kind);
        p.section_mut(kind.section()).push(m);
        let mut e = engine(&p, SEED);
        let ev = events(&[note(60, 0, 4_000), note(67, 1_000, 6_000)]);
        let frames = 8_192;
        let (mut left, mut right) = (vec![0.0f32; frames], vec![0.0f32; frames]);
        let mut chunk: Vec<Event> = Vec::with_capacity(ev.len());
        for (b, (l, r)) in left
            .chunks_mut(SEG * 4)
            .zip(right.chunks_mut(SEG * 4))
            .enumerate()
        {
            let start = (b * SEG * 4) as u64;
            chunk.clear();
            chunk.extend(
                ev.iter()
                    .filter(|(f, _)| (start..start + l.len() as u64).contains(f))
                    .map(|&(f, kind)| Event {
                        offset: (f - start) as u32,
                        kind,
                    }),
            );
            assert_no_alloc(|| e.process(l, r, &chunk));
        }
        assert!(
            left.iter().chain(&right).all(|x| x.is_finite()),
            "{kind}: finite"
        );
    }
}
