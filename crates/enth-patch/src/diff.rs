//! `ops_between`: the ops that turn one patch into another (moved down from `enth-cli`'s `diff_ops`, lane PBC of
//! R1/S1, so the session's undoable preset load and the CLI share one verified implementation).
//!
//! The list is built in an order that keeps every intermediate patch valid (removals before additions, so no section
//! limit is passed; the amplitude envelope is swapped only once the new one is in place; routes come last, in the
//! target's summation order) and is **checked by applying it**: a result that does not reproduce the target is
//! `None`, never a wrong list.
//!
//! Two plans are tried. The first changes as little as possible, so `enth diff` reads like a human edit: a route
//! both patches have stays where it is and only its depth moves. A plan can still be refused when a kept route would
//! be invalid in a state between the two (a module's rate scope changing under it), so the second plan takes every
//! route out first and puts `b`'s back at the end, when the modules and values they refer to are final.

use crate::addr::{MacroIndex, ModuleId, ParamAddr, Position, RouteAddr, SlotIndex, Target};
use crate::catalogue::Builtin;
use crate::model::{ModuleInstance, Patch, Route, Section};
use crate::ops::{Op, apply};

/// The ops that turn `a` into `b`, verified by applying them to a copy of `a`: `Some(ops)` only when the result equals
/// `b`, `None` otherwise (the list is not trusted, it is checked).
///
/// The engine stamp (`Patch::engine`, "authored on") is the one thing ops never change, so it is left out of the
/// comparison: applying the result to `a` gives `b` with `a`'s stamp. Order of the ops: routes `b` does not have,
/// modules `b` does not have (the amplitude envelope last), modules `a` does not have, the amplitude envelope, the
/// order inside each section, module values, enabled flags and labels, the built-in `voice` and `master`, routes in
/// `b`'s summation order, macros, slots, metadata. Identical patches give `Some(vec![])`.
///
/// An editor preset load is built on it (`Session::load_preset` in `enth-session`): one history entry with these
/// ops. The CLI's `diff` command uses it too.
pub fn ops_between(a: &Patch, b: &Patch) -> Option<Vec<Op>> {
    plan(a, b, false).or_else(|| plan(a, b, true))
}

/// A patch being turned into the target, with the ops that did it.
struct Walk {
    cur: Patch,
    ops: Vec<Op>,
}

impl Walk {
    fn push(&mut self, op: Op) -> Option<()> {
        apply(&mut self.cur, &op).ok()?;
        self.ops.push(op);
        Some(())
    }

    /// Add the modules of `b` that the patch lacks, at the end of their section; one whose section is full waits for
    /// a later call (the old amplitude envelope still holds a place in `modulators`).
    fn add_missing(&mut self, b: &Patch) -> Option<()> {
        for (pos, m) in b.modules() {
            let full = self.cur.section(pos.section).len() >= pos.section.max_len();
            if self.cur.find(&m.id).is_none() && !full {
                let at = Position {
                    section: pos.section,
                    index: self.cur.section(pos.section).len(),
                };
                self.push(Op::AddModule {
                    at,
                    module: m.clone(),
                })?;
            }
        }
        Some(())
    }
}

fn same_route(x: &Route, y: &Route) -> bool {
    x.src == y.src && x.dst == y.dst
}

fn route_addr(r: &Route) -> RouteAddr {
    RouteAddr {
        src: r.src.clone(),
        dst: r.dst.clone(),
    }
}

/// One plan; `strip_routes` takes every route out first (the second plan).
fn plan(a: &Patch, b: &Patch, strip_routes: bool) -> Option<Vec<Op>> {
    let mut w = Walk {
        cur: a.clone(),
        ops: Vec::new(),
    };
    let same_module =
        |p: &Patch, m: &ModuleInstance| p.find(&m.id).is_some_and(|(_, o)| o.kind == m.kind);
    // 1. Routes b does not have (all of them in the second plan).
    for r in &a.routes {
        if strip_routes || !b.routes.iter().any(|x| same_route(x, r)) {
            w.push(Op::RemoveRoute {
                route: route_addr(r),
            })?;
        }
    }
    // 2. Modules b does not have, or has as another kind (the amplitude envelope last, after b's is in place).
    let removals: Vec<ModuleId> = a
        .modules()
        .filter(|(_, m)| !same_module(b, m))
        .map(|(_, m)| m.id.clone())
        .collect();
    let amp = w.cur.voice.amp_env.clone();
    for id in removals.iter().filter(|id| **id != amp) {
        w.push(Op::RemoveModule { id: id.clone() })?;
    }
    // 3. Modules a does not have, at the end of their section.
    w.add_missing(b)?;
    // 4. The amplitude envelope, then the old one if b dropped it.
    if w.cur.voice.amp_env != b.voice.amp_env {
        let old = w.cur.voice.amp_env.clone();
        if w.cur.find(&b.voice.amp_env).is_none() {
            // `modulators` is full (the old envelope holds a place): take out another module for the moment. It is
            // not the envelope, and the `add_missing` after this step puts it back from `b`; the routes and slots it
            // had come back in steps 8 and 9.
            let room = w.cur.modulators.iter().rev().find(|m| m.id != old)?;
            let id = room.id.clone();
            w.push(Op::RemoveModule { id })?;
            let (pos, m) = b.find(&b.voice.amp_env)?;
            let at = Position {
                section: pos.section,
                index: w.cur.section(pos.section).len(),
            };
            w.push(Op::AddModule {
                at,
                module: m.clone(),
            })?;
        }
        // The envelope has to be on to take the place; step 6 sets the flag `b` wants for every module.
        if w.cur
            .find(&b.voice.amp_env)
            .is_some_and(|(_, m)| !m.enabled)
        {
            w.push(Op::SetEnabled {
                id: b.voice.amp_env.clone(),
                enabled: true,
            })?;
        }
        w.push(Op::SetAmpEnv {
            id: b.voice.amp_env.clone(),
        })?;
        if removals.contains(&old) {
            w.push(Op::RemoveModule { id: old })?;
        }
    }
    // Modules that waited for room (only `modulators` can be full, and only while the old envelope is in it).
    w.add_missing(b)?;
    // 5. Order within each section.
    for s in Section::ALL {
        for (i, m) in b.section(s).iter().enumerate() {
            if w.cur.section(s).get(i).map(|x| &x.id) != Some(&m.id) {
                let to = Position {
                    section: s,
                    index: i,
                };
                w.push(Op::Move {
                    id: m.id.clone(),
                    to,
                })?;
            }
        }
    }
    // 6. Values, enabled flags and labels of the modules.
    for (_, m) in b.modules() {
        let c = w.cur.find(&m.id).map(|(_, c)| c.clone())?;
        for ((spec, have), want) in m.kind.params().iter().zip(&c.values).zip(&m.values) {
            if have.to_bits() != want.to_bits() {
                let addr = ParamAddr::module(m.id.clone(), spec.key).ok()?;
                w.push(Op::SetParam { addr, value: *want })?;
            }
        }
        if c.enabled != m.enabled {
            w.push(Op::SetEnabled {
                id: m.id.clone(),
                enabled: m.enabled,
            })?;
        }
        if c.label != m.label {
            w.push(Op::SetLabel {
                id: m.id.clone(),
                label: m.label.clone(),
            })?;
        }
    }
    // 7. voice and master.
    for (b_vals, builtin, target) in [
        (&b.voice.values[..], Builtin::Voice, Target::Voice),
        (&b.master[..], Builtin::Master, Target::Master),
    ] {
        for (i, spec) in builtin.params().iter().enumerate() {
            let have = match builtin {
                Builtin::Voice => w.cur.voice.values[i],
                Builtin::Master => w.cur.master[i],
            };
            if have.to_bits() != b_vals[i].to_bits() {
                let addr = ParamAddr::new(target.clone(), spec.key).ok()?;
                w.push(Op::SetParam {
                    addr,
                    value: b_vals[i],
                })?;
            }
        }
    }
    // 8. Routes, in b's summation order.
    for (i, r) in b.routes.iter().enumerate() {
        match w.cur.routes.iter().position(|x| same_route(x, r)) {
            Some(j) if j == i => {
                if w.cur.routes[j].depth.to_bits() != r.depth.to_bits() {
                    w.push(Op::SetDepth {
                        route: route_addr(r),
                        depth: r.depth,
                    })?;
                }
            }
            Some(_) => {
                w.push(Op::RemoveRoute {
                    route: route_addr(r),
                })?;
                w.push(Op::InsertRoute {
                    index: i,
                    route: r.clone(),
                })?;
            }
            None => w.push(Op::InsertRoute {
                index: i,
                route: r.clone(),
            })?,
        }
    }
    // 9. Macros, slots, metadata.
    for (i, (have, want)) in w.cur.macros.clone().iter().zip(&b.macros).enumerate() {
        let index = MacroIndex::new(i)?;
        if have.value.to_bits() != want.value.to_bits() {
            w.push(Op::SetMacro {
                index,
                value: want.value,
            })?;
        }
        if have.name != want.name {
            w.push(Op::SetMacroName {
                index,
                name: want.name.clone(),
            })?;
        }
    }
    for (i, (have, want)) in w.cur.slots.clone().iter().zip(&b.slots).enumerate() {
        if have != want {
            let slot = SlotIndex::new(i)?;
            w.push(Op::SetSlot {
                slot,
                addr: want.clone(),
            })?;
        }
    }
    if w.cur.meta != b.meta {
        w.push(Op::SetMeta {
            meta: b.meta.clone(),
        })?;
    }
    // The stamp is the one thing no op changes.
    w.cur.engine.clone_from(&b.engine);
    (w.cur == *b).then_some(w.ops)
}
