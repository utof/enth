//! Negative controls (S1b): each property the suite relies on is also run against a deliberately broken
//! implementation, and must find a counterexample there while passing on the real one. A property that cannot fail
//! proves nothing. Failure persistence is off here: these runs are *meant* to fail, and the regression files are
//! for real finds.

mod common;

use common::*;
use enth_patch::addr::ModuleId;
use enth_patch::canon::{canonical_hash, exact_hash};
use enth_patch::catalogue::{ModuleKind, OscParam};
use enth_patch::history::{GestureId, History};
use enth_patch::model::{Patch, Section};
use enth_patch::ops::{Op, apply, apply_all};
use proptest::prelude::*;
use proptest::test_runner::{Config, TestCaseError, TestRunner};

const CASES: u32 = 1500;

fn runner() -> TestRunner {
    TestRunner::new(Config {
        cases: CASES,
        failure_persistence: None,
        max_shrink_iters: 200,
        max_global_rejects: 1 << 20,
        ..Config::default()
    })
}

/// The property holds on the real implementation ...
fn holds<T: std::fmt::Debug>(
    s: impl Strategy<Value = T>,
    body: impl Fn(T) -> Result<(), TestCaseError>,
) {
    if let Err(e) = runner().run(&s, body) {
        panic!("the real implementation fails its own property: {e}");
    }
}

/// ... and a broken one is caught.
fn is_caught<T: std::fmt::Debug>(
    s: impl Strategy<Value = T>,
    body: impl Fn(T) -> Result<(), TestCaseError>,
) {
    assert!(
        runner().run(&s, body).is_err(),
        "the property found nothing wrong with the broken implementation"
    );
}

// ---------------------------------------------------------------------------------------------------------------
// Inverses: a RemoveModule whose inverse forgets the routes and slots
// ---------------------------------------------------------------------------------------------------------------

type Remove = fn(&mut Patch, &ModuleId) -> Option<Vec<Op>>;

fn real_remove(p: &mut Patch, id: &ModuleId) -> Option<Vec<Op>> {
    apply(p, Op::RemoveModule { id: id.clone() }).ok()
}

/// Removes the module and the routes and slots that touch it (the patch stays valid) but only remembers the module.
fn naive_remove(p: &mut Patch, id: &ModuleId) -> Option<Vec<Op>> {
    if *id == p.voice.amp_env {
        return None;
    }
    let (pos, m) = p.find(id)?;
    let module = m.clone();
    p.section_mut(pos.section).remove(pos.index);
    p.routes.retain(|r| {
        r.dst.module_id() != Some(id)
            && !matches!(&r.src, enth_patch::addr::ModSource::Module(s) if s == id)
    });
    for s in &mut p.slots {
        if s.as_ref().and_then(|a| a.module_id()) == Some(id) {
            *s = None;
        }
    }
    Some(vec![Op::AddModule { at: pos, module }])
}

fn removal_is_undone(remove: Remove, (setup, pick): (Vec<Raw>, u8)) -> Result<(), TestCaseError> {
    let mut p = build(&setup);
    let ids: Vec<ModuleId> = p.modules().map(|(_, m)| m.id.clone()).collect();
    let id = &ids[usize::from(pick) % ids.len()];
    let before = p.clone();
    if let Some(inv) = remove(&mut p, id) {
        for op in &inv {
            apply(&mut p, op).map_err(|e| TestCaseError::fail(e.to_string()))?;
        }
        prop_assert!(
            identical(&p, &before),
            "removing {id} and undoing it changed the patch"
        );
    }
    Ok(())
}

#[test]
fn control_inverse_of_remove_must_restore_routes_and_slots() {
    let strategy = || (raws(10..60), any::<u8>());
    holds(strategy(), |a| removal_is_undone(real_remove, a));
    is_caught(strategy(), |a| removal_is_undone(naive_remove, a));
}

// ---------------------------------------------------------------------------------------------------------------
// Transactions: an apply_all that does not roll back
// ---------------------------------------------------------------------------------------------------------------

type ApplyAll = fn(&mut Patch, &[Op]) -> bool;

fn real_apply_all(p: &mut Patch, ops: &[Op]) -> bool {
    apply_all(p, ops).is_ok()
}

fn no_rollback(p: &mut Patch, ops: &[Op]) -> bool {
    ops.iter().all(|op| apply(p, op).is_ok())
}

fn transaction_is_atomic(
    f: ApplyAll,
    (setup, rs): (Vec<Raw>, Vec<Raw>),
) -> Result<(), TestCaseError> {
    let mut p = build(&setup);
    let before = p.clone();
    let mut scratch = p.clone();
    let mut fresh = 2000;
    let mut ops = Vec::new();
    for r in &rs {
        let op = make_op(&scratch, *r, &mut fresh);
        let _ = apply(&mut scratch, &op);
        ops.push(op);
    }
    if !f(&mut p, &ops) {
        prop_assert!(
            identical(&p, &before),
            "a failed transaction left its first ops applied"
        );
    }
    Ok(())
}

#[test]
fn control_a_failed_transaction_must_roll_back() {
    let strategy = || (raws(0..40), raws(2..14));
    holds(strategy(), |a| transaction_is_atomic(real_apply_all, a));
    is_caught(strategy(), |a| transaction_is_atomic(no_rollback, a));
}

// ---------------------------------------------------------------------------------------------------------------
// History: merging a drag across addresses loses undo data
// ---------------------------------------------------------------------------------------------------------------

/// The first draft of the merge: every `SetParam` of a gesture replaces the last one, whatever its address; the
/// entry keeps the first inverse and the last forward.
#[derive(Default)]
struct MergeEverything {
    done: Vec<(Vec<Op>, Vec<Op>, Option<GestureId>)>,
}

impl MergeEverything {
    fn edit(&mut self, p: &mut Patch, op: Op, g: Option<GestureId>) {
        if let Ok(inv) = apply(p, &op) {
            if let (Some(g), Some(last)) = (g, self.done.last_mut())
                && last.2 == Some(g)
            {
                last.0 = vec![op];
                return;
            }
            self.done.push((vec![op], inv, g));
        }
    }

    fn undo(&mut self, p: &mut Patch) {
        if let Some((_, inv, _)) = self.done.pop() {
            for op in &inv {
                let _ = apply(p, op);
            }
        }
    }
}

fn drag_is_undone_in_one_step(
    naive: bool,
    (setup, edits): (Vec<Raw>, Vec<Raw>),
) -> Result<(), TestCaseError> {
    let mut p = build(&setup);
    let start = p.clone();
    let mut fresh = 3000;
    let ops: Vec<Op> = edits
        .iter()
        .map(|r| make_op(&start, *r, &mut fresh))
        .filter(|op| matches!(op, Op::SetParam { .. }))
        .collect();
    if naive {
        let mut h = MergeEverything::default();
        let g = Some(GestureId(1));
        for op in ops {
            h.edit(&mut p, op, g);
        }
        h.undo(&mut p);
    } else {
        let mut h = History::new();
        let g = h.begin_gesture(&p);
        for op in ops {
            let _ = h.edit(&mut p, vec![op], Some(g));
        }
        h.end_gesture(&p);
        let _ = h.undo(&mut p);
    }
    prop_assert!(
        identical(&p, &start),
        "one undo did not return to the start of the drag"
    );
    Ok(())
}

#[test]
fn control_a_drag_across_addresses_must_keep_every_inverse() {
    let strategy = || (raws(0..20), raws(2..10));
    holds(strategy(), |a| drag_is_undone_in_one_step(false, a));
    is_caught(strategy(), |a| drag_is_undone_in_one_step(true, a));
}

// ---------------------------------------------------------------------------------------------------------------
// Hashes
// ---------------------------------------------------------------------------------------------------------------

/// Canonical equality under relabelling and reordering, for any hash function.
fn hash_survives_symmetries(
    hash: fn(&Patch) -> enth_patch::canon::PatchHash,
    (setup, seed): (Vec<Raw>, u64),
) -> Result<(), TestCaseError> {
    let p = build(&setup);
    prop_assert_eq!(hash(&p), hash(&scramble(&p, seed)));
    Ok(())
}

#[test]
fn control_the_exact_hash_is_not_a_canonical_hash() {
    let strategy = || (raws(5..60), any::<u64>());
    holds(strategy(), |a| hash_survives_symmetries(canonical_hash, a));
    is_caught(strategy(), |a| hash_survives_symmetries(exact_hash, a));
}

/// A plausible first canonical form: drop what is silent, sort generators and modulators by (kind, parameter bits),
/// name modules by position. No routes in the sort key and no rule for hard sync.
fn naive_canonical(p: &Patch) -> Patch {
    let mut q = p.clone();
    q.engine.clear();
    for section in Section::ALL {
        q.section_mut(section).retain(|m| m.enabled);
    }
    q.routes.retain(|r| r.depth != 0.0);
    let enabled = |id: &ModuleId| q.find(id).is_some();
    let live: Vec<_> = q
        .routes
        .iter()
        .filter(|r| {
            r.dst.module_id().is_none_or(enabled)
                && match &r.src {
                    enth_patch::addr::ModSource::Module(id) => enabled(id),
                    _ => true,
                }
        })
        .cloned()
        .collect();
    q.routes = live;
    for section in [Section::Generators, Section::Modulators] {
        q.section_mut(section).sort_by_key(|m| {
            (
                m.kind as u8,
                m.values.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
            )
        });
    }
    let old: Vec<ModuleId> = q.modules().map(|(_, m)| m.id.clone()).collect();
    let mut counts = std::collections::BTreeMap::new();
    let names: Vec<ModuleId> = q
        .modules()
        .map(|(_, m)| {
            let prefix = m.kind.spec().id_prefix;
            let n = counts.entry(prefix).or_insert(0);
            *n += 1;
            ModuleId::numbered(prefix, *n).unwrap()
        })
        .collect();
    let mut q = rename(&q, |id| {
        names[old.iter().position(|x| x == id).unwrap()].clone()
    });
    q.routes
        .sort_by(|a, b| (&a.dst, &a.src).cmp(&(&b.dst, &b.src)));
    q.slots = std::array::from_fn(|_| None);
    q
}

#[test]
fn control_equal_parameters_with_different_routes_need_more_than_a_value_sort() {
    // Real: the two spellings are the same patch. Naive: stable sort keeps the written order, so lfo1 stays lfo1
    // and the routes name different modules.
    assert_eq!(
        canonical_hash(&two_lfos(true)),
        canonical_hash(&two_lfos(false))
    );
    assert!(!identical(
        &naive_canonical(&two_lfos(true)),
        &naive_canonical(&two_lfos(false))
    ));
}

#[test]
fn control_a_value_sort_does_not_hear_the_sync_master() {
    // Three oscillators with different parameters, the last one synced. Swapping the first two changes which one
    // is the master, hence the sound. The real hash notices; the naive form sorts the difference away.
    let mut p = base_patch();
    for (i, semi) in [7.0f32, -12.0, 3.0].into_iter().enumerate() {
        if i > 0 {
            let id = p.free_id(ModuleKind::Osc);
            let at = enth_patch::addr::Position {
                section: Section::Generators,
                index: p.generators.len(),
            };
            apply(
                &mut p,
                &Op::AddModule {
                    at,
                    module: enth_patch::model::ModuleInstance::new(id, ModuleKind::Osc),
                },
            )
            .unwrap();
        }
        let id = p.generators[i].id.clone();
        apply(
            &mut p,
            &Op::SetParam {
                addr: format!("{id}.semi").parse().unwrap(),
                value: semi,
            },
        )
        .unwrap();
    }
    let last = p.generators[2].id.clone();
    apply(
        &mut p,
        &Op::SetParam {
            addr: format!("{last}.{}", OscParam::Sync.key()).parse().unwrap(),
            value: 1.0,
        },
    )
    .unwrap();
    let mut swapped = p.clone();
    swapped.generators.swap(0, 1);
    assert_ne!(
        canonical_hash(&p),
        canonical_hash(&swapped),
        "the real hash hears the master"
    );
    assert!(
        identical(&naive_canonical(&p), &naive_canonical(&swapped)),
        "the naive form does not"
    );
}

/// Colour refinement alone (the classic Weisfeiler-Leman test): a multiset of node colours after the colours
/// stabilise. Used to show that a hexagon and two triangles are indistinguishable to it.
fn colour_refinement(n: usize, edges: &[(usize, usize)]) -> Vec<u64> {
    let mut colour = vec![1u64; n];
    for _ in 0..n {
        let mut next = vec![0u64; n];
        for v in 0..n {
            let mut ins: Vec<u64> = edges
                .iter()
                .filter(|e| e.1 == v)
                .map(|e| colour[e.0])
                .collect();
            let mut outs: Vec<u64> = edges
                .iter()
                .filter(|e| e.0 == v)
                .map(|e| colour[e.1])
                .collect();
            ins.sort_unstable();
            outs.sort_unstable();
            next[v] = ins
                .iter()
                .chain([&u64::MAX])
                .chain(&outs)
                .fold(colour[v].wrapping_mul(0x9E37_79B9_7F4A_7C15), |h, &x| {
                    (h ^ x).wrapping_mul(0x100_0000_01B3).rotate_left(17)
                });
        }
        colour = next;
    }
    colour.sort_unstable();
    colour
}

#[test]
fn control_colour_refinement_alone_cannot_tell_a_hexagon_from_two_triangles() {
    let hexagon = [(0, 1), (1, 2), (2, 3), (3, 4), (4, 5), (5, 0)];
    let triangles = [(0, 1), (1, 2), (2, 0), (3, 4), (4, 5), (5, 3)];
    assert_eq!(
        colour_refinement(6, &hexagon),
        colour_refinement(6, &triangles),
        "1-WL is blind to this pair"
    );
    // So the canonical form needs its individualisation search, and has it:
    assert_ne!(
        canonical_hash(&lfo_graph(6, &hexagon)),
        canonical_hash(&lfo_graph(6, &triangles)),
        "a canonical form built on colour refinement alone would hash these alike"
    );
}

// ---------------------------------------------------------------------------------------------------------------
// The file writer
// ---------------------------------------------------------------------------------------------------------------

#[test]
fn control_writing_through_serde_json_value_widens_every_number() {
    let strategy = || raws(5..60);
    // Real: shortest text. Naive: `to_value` widens f32 to f64 (0.1 becomes 0.10000000149011612).
    holds(strategy(), |setup| {
        prop_assert!(max_digits(&build(&setup).to_json()) <= 9);
        Ok(())
    });
    is_caught(strategy(), |setup| {
        let p = build(&setup);
        let widened = serde_json::to_value(p.to_file()).unwrap().to_string();
        prop_assert!(max_digits(&widened) <= 9);
        Ok(())
    });
}
