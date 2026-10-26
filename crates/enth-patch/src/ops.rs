//! Edit [`Op`]s with inverses, atomic transactions and [`effect`] (P1a lane S1b).
//!
//! **Every patch mutation is an op** (AGENTS.md). [`apply`] changes a patch and returns the ops that undo it;
//! [`apply_all`] applies a list atomically. Host automation applies `SetParam` through the same function and just
//! does not record a history entry.
//!
//! # The ops
//!
//! | Op | Does | Inverse |
//! | --- | --- | --- |
//! | `SetParam { addr, value }` | a parameter of a module, `voice` or `master`, in plain units | `SetParam` with the old value |
//! | `AddModule { at, module }` | inserts the full instance (so an inverse can re-add it) | `RemoveModule` |
//! | `RemoveModule { id }` | removes a module, its routes and its slot assignments | `AddModule`, then `InsertRoute`s at their original indices (ascending), then `SetSlot`s |
//! | `Move { id, to }` | reorders a module within its section (`to.index` is the final index) | `Move` back |
//! | `InsertRoute { index, route }` | inserts a route at a place in the summation order (`index == routes.len()` appends) | `RemoveRoute` |
//! | `RemoveRoute { route }` | removes the route with this source and destination | `InsertRoute` at its old index |
//! | `SetDepth { route, depth }` | changes the depth of an existing route | `SetDepth` with the old depth |
//! | `SetEnabled { id, enabled }` | bypasses or restores a module | `SetEnabled` |
//! | `SetAmpEnv { id }` | names the amplitude envelope (an enabled `adsr`) | `SetAmpEnv` with the old one |
//! | `SetMacro { index, value }` | a macro knob position, 0–1 | `SetMacro` |
//! | `SetSlot { slot, addr }` | assigns or clears a host automation slot | `SetSlot` |
//! | `SetLabel { id, label }` | a module's display label | `SetLabel` |
//! | `SetMeta { meta }` | name, author, description, tags | `SetMeta` |
//! | `SetMacroName { index, name }` | a macro's display name | `SetMacroName` |
//!
//! `Move` is `MoveModule`, `RemoveRoute` is `Unroute`, `SetDepth` is the update half of `SetRoute` and `SetSlot`
//! is `AssignSlot` in docs/design.md: the lane decision split `SetRoute` (insert or update) in two so every op
//! has one meaning. The wire form (`crate::wire`) still has the single `route` verb.
//!
//! # Guarantees (all property-tested, with exact f32 bits)
//!
//! - **Inverse:** for a valid patch `p` and any op that applies, applying the returned inverse ops in order gives
//!   `p` back exactly (`canon::identical`).
//! - **Failure changes nothing:** an op that errors leaves the patch untouched. Every op checks first and mutates
//!   after.
//! - **Validity:** an op that applies to a valid patch leaves a valid patch (`validate` is `Ok`).
//! - **Atomic transactions:** [`apply_all`] either applies every op or none, and returns the inverse transaction.
//!
//! # Stored values
//!
//! Values are checked against their `ParamSpec` and stored canonically: `-0.0` becomes `+0.0`, and **the top of a
//! periodic range is stored as the bottom** (a phase of 360° is stored as 0°: the same sound, one spelling, so the
//! file round trip and the hashes never see two forms). See [`canonical_value`].
//!
//! # Rules ops enforce
//!
//! The invariants of [`model`](crate::model) hold after every op. The ones that need explaining:
//!
//! - `RemoveModule` of the amplitude envelope, and `SetEnabled { enabled: false }` on it, fail with
//!   `in_use_as_amp_env`: name another envelope with `SetAmpEnv` first.
//! - A route may never send a per-voice source to a global destination (`route_rate_mismatch`). Changing an LFO's
//!   `scope` is checked against its routes the same way, so the rule survives structural edits.
//! - There is no cycle rule for control routes: a modulator may modulate an earlier one, or itself, and reads that
//!   source's previous-segment value (docs/design.md §2).
//! - **Audio routes** (S2a lane RTE): a generator may drive only an `audio` parameter (`not_audio_dest`), an `audio`
//!   parameter takes only a generator (`not_audio_source`), and audio routes never form a loop, a generator driving
//!   itself included (`audio_cycle`): audio runs in one direction, and feedback is a parameter inside a module. The
//!   rules are `validate::check_route_ends` and `validate::closes_audio_cycle`, shared with the file layer.

use core::borrow::Borrow;

use enth_params::{DEPTH, ParamSpec};

use crate::addr::{
    MacroIndex, ModSource, ModuleId, ParamAddr, Position, RouteAddr, SlotIndex, Target,
};
use crate::catalogue::{Builtin, MACRO_VALUE, MASTER_PARAMS, ModuleKind, Rate, VOICE_PARAMS};
use crate::error::{OpError, PatchError};
use crate::model::{MAX_ROUTES, ModuleInstance, Patch, PatchMeta, Route};
use crate::validate::{check_route_ends, closes_audio_cycle};

/// One edit to a patch. See the [module docs](self) for what each does and its inverse.
///
/// New variants are appended; match with a wildcard arm.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum Op {
    /// Set a parameter to a plain value. Also reaches `voice.*` and `master.*`.
    SetParam {
        /// The parameter.
        addr: ParamAddr,
        /// The new plain value.
        value: f32,
    },
    /// Insert a module at a position in its section.
    AddModule {
        /// Where; `at.index <= len`.
        at: Position,
        /// The full instance.
        module: ModuleInstance,
    },
    /// Remove a module and, with it, its routes and slot assignments.
    RemoveModule {
        /// The module.
        id: ModuleId,
    },
    /// Move a module within its section.
    Move {
        /// The module.
        id: ModuleId,
        /// Where it ends up (`to.index` counts the section without the module).
        to: Position,
    },
    /// Insert a route at an index of the summation order.
    InsertRoute {
        /// Where; `index <= routes.len()`.
        index: usize,
        /// The route.
        route: Route,
    },
    /// Remove a route.
    RemoveRoute {
        /// Its source and destination.
        route: RouteAddr,
    },
    /// Change the depth of an existing route.
    SetDepth {
        /// Its source and destination.
        route: RouteAddr,
        /// The new depth in percent, −100..=100.
        depth: f32,
    },
    /// Bypass or restore a module.
    SetEnabled {
        /// The module.
        id: ModuleId,
        /// Whether it runs.
        enabled: bool,
    },
    /// Name the amplitude envelope.
    SetAmpEnv {
        /// An enabled `adsr` in `modulators`.
        id: ModuleId,
    },
    /// Set a macro knob.
    SetMacro {
        /// Which macro.
        index: MacroIndex,
        /// Knob position, 0–1.
        value: f32,
    },
    /// Assign or clear a host automation slot.
    SetSlot {
        /// Which slot.
        slot: SlotIndex,
        /// An automatable parameter, or `None` to clear.
        addr: Option<ParamAddr>,
    },
    /// Set or clear a module's display label.
    SetLabel {
        /// The module.
        id: ModuleId,
        /// The label.
        label: Option<String>,
    },
    /// Replace the patch metadata.
    SetMeta {
        /// The new metadata.
        meta: PatchMeta,
    },
    /// Set or clear a macro's display name.
    SetMacroName {
        /// Which macro.
        index: MacroIndex,
        /// The name.
        name: Option<String>,
    },
}

impl Op {
    /// The variant name (`"SetParam"`), for messages.
    pub const fn name(&self) -> &'static str {
        match self {
            Op::SetParam { .. } => "SetParam",
            Op::AddModule { .. } => "AddModule",
            Op::RemoveModule { .. } => "RemoveModule",
            Op::Move { .. } => "Move",
            Op::InsertRoute { .. } => "InsertRoute",
            Op::RemoveRoute { .. } => "RemoveRoute",
            Op::SetDepth { .. } => "SetDepth",
            Op::SetEnabled { .. } => "SetEnabled",
            Op::SetAmpEnv { .. } => "SetAmpEnv",
            Op::SetMacro { .. } => "SetMacro",
            Op::SetSlot { .. } => "SetSlot",
            Op::SetLabel { .. } => "SetLabel",
            Op::SetMeta { .. } => "SetMeta",
            Op::SetMacroName { .. } => "SetMacroName",
        }
    }

    /// [`apply`] as a method.
    pub fn apply(&self, patch: &mut Patch) -> Result<Vec<Op>, OpError> {
        apply(patch, self)
    }
}

/// What an op changes for the audio side, which decides how `enth-session` delivers it (docs/design.md §3).
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum Effect {
    /// A smoothed value event for this parameter.
    Value(ParamAddr),
    /// A value event for the depth of this route.
    Depth(RouteAddr),
    /// A value event for this macro knob.
    Macro(MacroIndex),
    /// The render plan must be recompiled: modules or routes changed, or a structural parameter.
    Structural,
    /// Nothing audible: labels, metadata, names, slots.
    NoAudio,
}

/// How applying `op` to `patch` reaches the audio: a value event or a recompile. Reads
/// `ParamSpec::flags.structural` for `SetParam`; asks about the patch because the same address can be structural
/// for one kind and not another. An op that would not apply is reported `Structural` (the safe answer).
pub fn effect(patch: &Patch, op: &Op) -> Effect {
    match op {
        Op::SetParam { addr, .. } => match resolve(patch, addr) {
            Ok((_, spec)) if !spec.flags.structural => Effect::Value(addr.clone()),
            _ => Effect::Structural,
        },
        Op::SetDepth { route, .. } => Effect::Depth(route.clone()),
        Op::SetMacro { index, .. } => Effect::Macro(*index),
        Op::SetSlot { .. } | Op::SetLabel { .. } | Op::SetMeta { .. } | Op::SetMacroName { .. } => {
            Effect::NoAudio
        }
        Op::AddModule { .. }
        | Op::RemoveModule { .. }
        | Op::Move { .. }
        | Op::InsertRoute { .. }
        | Op::RemoveRoute { .. }
        | Op::SetEnabled { .. }
        | Op::SetAmpEnv { .. } => Effect::Structural,
    }
}

/// A failed transaction: the index of the op that failed and why. The patch is unchanged.
#[derive(Clone, Debug, PartialEq)]
pub struct TxError {
    /// Index into the ops list.
    pub index: usize,
    /// The op's error.
    pub error: OpError,
}

impl core::fmt::Display for TxError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "op {}: {}", self.index, self.error)
    }
}

impl std::error::Error for TxError {}

/// Store a value the way a patch holds it: checked against `spec` (finite, in range, integral for discrete
/// kinds), `-0.0` as `+0.0`, and the top of a periodic range as its bottom (360° as 0°).
pub fn canonical_value(spec: &ParamSpec, value: f32) -> Result<f32, enth_params::ValueError> {
    let v = spec.check(value)?;
    Ok(if spec.flags.periodic && v == spec.max() {
        spec.min()
    } else {
        v
    })
}

/// Apply one op, returning the ops that undo it (apply them in order). On error the patch is unchanged.
///
/// Takes the op by reference or by value.
pub fn apply(patch: &mut Patch, op: impl Borrow<Op>) -> Result<Vec<Op>, OpError> {
    apply_one(patch, op.borrow())
}

/// Apply a transaction atomically: every op or none (on error the patch is bit-identical to before). Returns the
/// inverse transaction, to apply in order to undo all of it.
pub fn apply_all(patch: &mut Patch, ops: &[Op]) -> Result<Vec<Op>, TxError> {
    match ops {
        [] => Ok(Vec::new()),
        [only] => apply_one(patch, only).map_err(|error| TxError { index: 0, error }),
        _ => {
            let snapshot = patch.clone();
            let mut undo: Vec<Vec<Op>> = Vec::with_capacity(ops.len());
            for (index, op) in ops.iter().enumerate() {
                match apply_one(patch, op) {
                    Ok(inv) => undo.push(inv),
                    Err(error) => {
                        *patch = snapshot;
                        return Err(TxError { index, error });
                    }
                }
            }
            Ok(undo.into_iter().rev().flatten().collect())
        }
    }
}

fn apply_one(p: &mut Patch, op: &Op) -> Result<Vec<Op>, OpError> {
    match op {
        Op::SetParam { addr, value } => set_param(p, addr, *value),
        Op::AddModule { at, module } => add_module(p, *at, module),
        Op::RemoveModule { id } => remove_module(p, id),
        Op::Move { id, to } => move_module(p, id, *to),
        Op::InsertRoute { index, route } => insert_route(p, *index, route),
        Op::RemoveRoute { route } => remove_route(p, route),
        Op::SetDepth { route, depth } => set_depth(p, route, *depth),
        Op::SetEnabled { id, enabled } => set_enabled(p, id, *enabled),
        Op::SetAmpEnv { id } => set_amp_env(p, id),
        Op::SetMacro { index, value } => {
            let v = canonical_value(&MACRO_VALUE, *value)?;
            let old = core::mem::replace(&mut p.macros[index.get()].value, v);
            Ok(vec![Op::SetMacro {
                index: *index,
                value: old,
            }])
        }
        Op::SetSlot { slot, addr } => {
            if let Some(a) = addr {
                check_automatable(p, a)?;
            }
            let old = core::mem::replace(&mut p.slots[slot.get()], addr.clone());
            Ok(vec![Op::SetSlot {
                slot: *slot,
                addr: old,
            }])
        }
        Op::SetLabel { id, label } => {
            let (pos, _) = locate(p, id)?;
            let m = &mut p.section_mut(pos.section)[pos.index];
            let old = core::mem::replace(&mut m.label, label.clone());
            Ok(vec![Op::SetLabel {
                id: id.clone(),
                label: old,
            }])
        }
        Op::SetMeta { meta } => {
            let old = core::mem::replace(&mut p.meta, meta.clone());
            Ok(vec![Op::SetMeta { meta: old }])
        }
        Op::SetMacroName { index, name } => {
            let old = core::mem::replace(&mut p.macros[index.get()].name, name.clone());
            Ok(vec![Op::SetMacroName {
                index: *index,
                name: old,
            }])
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Resolving ids and addresses against a patch
// ---------------------------------------------------------------------------------------------------------------

/// Where a parameter's value lives in a patch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Cell {
    Module(Position, usize),
    Voice(usize),
    Master(usize),
}

pub(crate) fn existing_ids(p: &Patch) -> Vec<String> {
    p.modules().map(|(_, m)| m.id.to_string()).collect()
}

pub(crate) fn unknown_module(p: &Patch, id: &ModuleId) -> PatchError {
    PatchError::UnknownModule {
        id: id.to_string(),
        existing: existing_ids(p),
    }
}

pub(crate) fn locate<'a>(
    p: &'a Patch,
    id: &ModuleId,
) -> Result<(Position, &'a ModuleInstance), PatchError> {
    p.find(id).ok_or_else(|| unknown_module(p, id))
}

fn unknown_param(owner: &'static str, keys: Vec<&'static str>, addr: &ParamAddr) -> PatchError {
    PatchError::UnknownParam {
        addr: addr.to_string(),
        owner,
        keys,
    }
}

/// The cell and spec an address names in this patch.
pub(crate) fn resolve(
    p: &Patch,
    addr: &ParamAddr,
) -> Result<(Cell, &'static ParamSpec), PatchError> {
    match addr.target() {
        Target::Module(id) => {
            let (pos, m) = locate(p, id)?;
            let i = m.kind.param_index(addr.param()).ok_or_else(|| {
                unknown_param(
                    m.kind.key(),
                    m.kind.params().iter().map(|s| s.key).collect(),
                    addr,
                )
            })?;
            // A module with fewer values than its kind has parameters is invalid (`validate` says so); refuse the
            // op rather than index past the end.
            if m.values.len() != m.kind.params().len() {
                return Err(PatchError::WrongValueCount {
                    id: id.to_string(),
                    expected: m.kind.params().len(),
                    found: m.values.len(),
                });
            }
            Ok((Cell::Module(pos, i), &m.kind.params()[i]))
        }
        Target::Voice => {
            let i = Builtin::Voice.param_index(addr.param()).ok_or_else(|| {
                unknown_param("voice", VOICE_PARAMS.iter().map(|s| s.key).collect(), addr)
            })?;
            Ok((Cell::Voice(i), &VOICE_PARAMS[i]))
        }
        Target::Master => {
            let i = Builtin::Master.param_index(addr.param()).ok_or_else(|| {
                unknown_param(
                    "master",
                    MASTER_PARAMS.iter().map(|s| s.key).collect(),
                    addr,
                )
            })?;
            Ok((Cell::Master(i), &MASTER_PARAMS[i]))
        }
    }
}

fn cell_mut(p: &mut Patch, cell: Cell) -> &mut f32 {
    match cell {
        Cell::Module(pos, i) => &mut p.section_mut(pos.section)[pos.index].values[i],
        Cell::Voice(i) => &mut p.voice.values[i],
        Cell::Master(i) => &mut p.master[i],
    }
}

fn check_automatable(p: &Patch, addr: &ParamAddr) -> Result<(), PatchError> {
    let (_, spec) = resolve(p, addr)?;
    if spec.flags.automatable {
        Ok(())
    } else {
        Err(PatchError::NotAutomatable {
            addr: addr.to_string(),
        })
    }
}

fn route_text(src: &ModSource, dst: &ParamAddr) -> String {
    format!("{src} -> {dst}")
}

/// Whether `route`'s source and destination exist and obey the rate rule, given the rate `id` would have
/// (`rate_of` overrides one module's rate).
fn check_route_rate(
    p: &Patch,
    route: &Route,
    overridden: Option<(&ModuleId, Rate)>,
) -> Result<(), PatchError> {
    let rate_of_module = |id: &ModuleId, fallback: Option<Rate>| match overridden {
        Some((o, rate)) if o == id => Some(rate),
        _ => fallback,
    };
    let src = match &route.src {
        ModSource::Module(id) => rate_of_module(id, p.source_rate(&route.src)),
        other => p.source_rate(other),
    };
    let dst = match route.dst.target() {
        Target::Module(id) => rate_of_module(id, p.target_rate(route.dst.target())),
        other => p.target_rate(other),
    };
    match (src, dst) {
        (Some(Rate::Voice), Some(Rate::Global)) => Err(PatchError::RouteRateMismatch {
            route: route_text(&route.src, &route.dst),
            src: Rate::Voice,
            dst: Rate::Global,
        }),
        _ => Ok(()),
    }
}

fn touches(route: &Route, id: &ModuleId) -> bool {
    route.dst.module_id() == Some(id) || matches!(&route.src, ModSource::Module(src) if src == id)
}

// ---------------------------------------------------------------------------------------------------------------
// The ops
// ---------------------------------------------------------------------------------------------------------------

fn set_param(p: &mut Patch, addr: &ParamAddr, value: f32) -> Result<Vec<Op>, OpError> {
    let (cell, spec) = resolve(p, addr)?;
    let new = canonical_value(spec, value)?;
    if let Cell::Module(pos, i) = cell {
        let m = &p.section(pos.section)[pos.index];
        if spec.flags.structural {
            // The only structural parameter that moves a module between per-voice and global is an LFO's scope.
            let mut values = m.values.clone();
            values[i] = new;
            let new_rate = m.kind.rate(&values);
            if new_rate != m.rate() {
                for route in p.routes.iter().filter(|r| touches(r, &m.id)) {
                    check_route_rate(p, route, Some((&m.id, new_rate)))?;
                }
            }
        }
    }
    let old = core::mem::replace(cell_mut(p, cell), new);
    Ok(vec![Op::SetParam {
        addr: addr.clone(),
        value: old,
    }])
}

fn add_module(p: &mut Patch, at: Position, module: &ModuleInstance) -> Result<Vec<Op>, OpError> {
    if p.find(&module.id).is_some() {
        return Err(PatchError::DuplicateId {
            id: module.id.to_string(),
        });
    }
    let kind = module.kind;
    if kind.section() != at.section {
        return Err(PatchError::WrongSection {
            id: module.id.to_string(),
            kind,
            expected: kind.section(),
            got: at.section,
        });
    }
    let len = p.section(at.section).len();
    if len >= at.section.max_len() {
        return Err(PatchError::TooMany {
            what: at.section.key(),
            max: at.section.max_len(),
        });
    }
    if at.index > len {
        return Err(PatchError::BadIndex {
            what: "position",
            index: at.index,
            max: len,
        });
    }
    if module.values.len() != kind.params().len() {
        return Err(PatchError::WrongValueCount {
            id: module.id.to_string(),
            expected: kind.params().len(),
            found: module.values.len(),
        });
    }
    let mut stored = module.clone();
    for (v, spec) in stored.values.iter_mut().zip(kind.params()) {
        *v = canonical_value(spec, *v)?;
    }
    let id = stored.id.clone();
    p.section_mut(at.section).insert(at.index, stored);
    Ok(vec![Op::RemoveModule { id }])
}

fn remove_module(p: &mut Patch, id: &ModuleId) -> Result<Vec<Op>, OpError> {
    let (pos, _) = locate(p, id)?;
    if &p.voice.amp_env == id {
        return Err(PatchError::InUseAsAmpEnv {
            id: id.to_string(),
            action: "removed",
        });
    }
    let module = p.section_mut(pos.section).remove(pos.index);
    let mut inverse = vec![Op::AddModule { at: pos, module }];
    // Routes that touch the module go with it; the inverse puts each back at its original index. Ascending
    // order is exact: when route `i` is re-inserted, every route before it is already in place.
    let mut kept = Vec::with_capacity(p.routes.len());
    for (index, route) in core::mem::take(&mut p.routes).into_iter().enumerate() {
        if touches(&route, id) {
            inverse.push(Op::InsertRoute { index, route });
        } else {
            kept.push(route);
        }
    }
    p.routes = kept;
    for (n, slot) in p.slots.iter_mut().enumerate() {
        if slot.as_ref().and_then(ParamAddr::module_id) == Some(id)
            && let Some(index) = SlotIndex::new(n)
        {
            inverse.push(Op::SetSlot {
                slot: index,
                addr: slot.take(),
            });
        }
    }
    Ok(inverse)
}

fn move_module(p: &mut Patch, id: &ModuleId, to: Position) -> Result<Vec<Op>, OpError> {
    let (from, m) = locate(p, id)?;
    if m.kind.section() != to.section {
        return Err(PatchError::WrongSection {
            id: id.to_string(),
            kind: m.kind,
            expected: m.kind.section(),
            got: to.section,
        });
    }
    // `to.index` is the final index. On a valid patch the module is in its own section, so there is one fewer to
    // count; on a corrupted one (a module out of its section) this stays in bounds rather than panic.
    let len = p.section(to.section).len();
    let max = if from.section == to.section {
        len.saturating_sub(1)
    } else {
        len
    };
    if to.index > max {
        return Err(PatchError::BadIndex {
            what: "position",
            index: to.index,
            max,
        });
    }
    let module = p.section_mut(from.section).remove(from.index);
    p.section_mut(to.section).insert(to.index, module);
    Ok(vec![Op::Move {
        id: id.clone(),
        to: from,
    }])
}

fn check_route(p: &Patch, route: &Route) -> Result<f32, PatchError> {
    let depth = canonical_value(&DEPTH, route.depth)?;
    // The routing contract (control and audio routes, S2a lane RTE) is `validate`'s, shared so a file and an op
    // refuse the same routes with the same codes.
    let ends = check_route_ends(p, route);
    if let Some(e) = ends.src_error.or(ends.dst_error) {
        return Err(e);
    }
    if closes_audio_cycle(p, route) {
        return Err(PatchError::AudioCycle {
            route: route_text(&route.src, &route.dst),
        });
    }
    check_route_rate(p, route, None)?;
    Ok(depth)
}

fn insert_route(p: &mut Patch, index: usize, route: &Route) -> Result<Vec<Op>, OpError> {
    let depth = check_route(p, route)?;
    if p.routes
        .iter()
        .any(|r| r.src == route.src && r.dst == route.dst)
    {
        return Err(PatchError::DuplicateRoute {
            route: route_text(&route.src, &route.dst),
        });
    }
    if p.routes.len() >= MAX_ROUTES {
        return Err(PatchError::TooMany {
            what: "routes",
            max: MAX_ROUTES,
        });
    }
    if index > p.routes.len() {
        return Err(PatchError::BadIndex {
            what: "route index",
            index,
            max: p.routes.len(),
        });
    }
    p.routes.insert(
        index,
        Route {
            src: route.src.clone(),
            dst: route.dst.clone(),
            depth,
        },
    );
    Ok(vec![Op::RemoveRoute {
        route: RouteAddr {
            src: route.src.clone(),
            dst: route.dst.clone(),
        },
    }])
}

fn route_index(p: &Patch, route: &RouteAddr) -> Result<usize, PatchError> {
    p.routes
        .iter()
        .position(|r| r.src == route.src && r.dst == route.dst)
        .ok_or_else(|| PatchError::NoSuchRoute {
            route: route.to_string(),
        })
}

fn remove_route(p: &mut Patch, route: &RouteAddr) -> Result<Vec<Op>, OpError> {
    let index = route_index(p, route)?;
    let removed = p.routes.remove(index);
    Ok(vec![Op::InsertRoute {
        index,
        route: removed,
    }])
}

fn set_depth(p: &mut Patch, route: &RouteAddr, depth: f32) -> Result<Vec<Op>, OpError> {
    let index = route_index(p, route)?;
    let depth = canonical_value(&DEPTH, depth)?;
    let old = core::mem::replace(&mut p.routes[index].depth, depth);
    Ok(vec![Op::SetDepth {
        route: route.clone(),
        depth: old,
    }])
}

fn set_enabled(p: &mut Patch, id: &ModuleId, enabled: bool) -> Result<Vec<Op>, OpError> {
    let (pos, _) = locate(p, id)?;
    if !enabled && &p.voice.amp_env == id {
        return Err(PatchError::InUseAsAmpEnv {
            id: id.to_string(),
            action: "disabled",
        });
    }
    let m = &mut p.section_mut(pos.section)[pos.index];
    let old = core::mem::replace(&mut m.enabled, enabled);
    Ok(vec![Op::SetEnabled {
        id: id.clone(),
        enabled: old,
    }])
}

fn set_amp_env(p: &mut Patch, id: &ModuleId) -> Result<Vec<Op>, OpError> {
    let (_, m) = locate(p, id)?;
    if m.kind != ModuleKind::Adsr {
        return Err(PatchError::WrongKind {
            id: id.to_string(),
            expected: ModuleKind::Adsr,
            found: m.kind,
        });
    }
    if !m.enabled {
        return Err(PatchError::ModuleDisabled { id: id.to_string() });
    }
    let old = core::mem::replace(&mut p.voice.amp_env, id.clone());
    Ok(vec![Op::SetAmpEnv { id: old }])
}

impl Patch {
    /// The lowest free id for a new module of this kind: `<prefix><n>` for the smallest `n >= 1` not in use
    /// (`osc1`, `filter2`). This is the id `add` takes when none is given.
    pub fn free_id(&self, kind: ModuleKind) -> ModuleId {
        let prefix = kind.spec().id_prefix;
        // A patch holds at most 20 modules, so one of the first 21 numbers is free.
        (1u32..=64)
            .filter_map(|n| ModuleId::numbered(prefix, n).ok())
            .find(|id| self.find(id).is_none())
            .unwrap_or_else(|| unreachable!("a patch holds at most 20 modules"))
    }
}
