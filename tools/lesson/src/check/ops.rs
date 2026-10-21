//! **Owner: lane LCK.** The patch arithmetic `check` needs: the ops from one patch to another with what each touches
//! (`enth_challenge::v2::touches`, the session's table), whether a measured hint could make it, and the touch log a
//! straight turn leaves.

use enth_challenge::v2::{Touch, touches};
use enth_learn::Knob;
use enth_learn::check::TouchLog;
use enth_params::{Kind, ParamSpec, Unit};
use enth_patch::{Op, Patch};

/// One op on the way from one patch to another ([`changes`]).
#[derive(Clone, Debug)]
pub(crate) struct Change {
    /// The op.
    pub op: Op,
    /// What it touches (`None` for an op kind this build does not know).
    pub touches: Option<Vec<Touch>>,
    /// Whether a measured hint could make it: every touch is a macro knob or a parameter `enth_solver::hint` moves.
    /// A route, a module switched or added, an enum, a toggle, an integer or a transposition is structure, which
    /// only the witness shows.
    pub continuous: bool,
}

impl Change {
    /// Whether it changes something a control shows (metadata and labels do not).
    pub fn is_move(&self) -> bool {
        self.touches.as_ref().is_none_or(|t| !t.is_empty())
    }
}

/// The ops from `a` to `b` (`enth_patch::ops_between`), each with what it touches against the patch as the ops before
/// it leave it. `None` when no ops lead there.
pub(crate) fn changes(a: &Patch, b: &Patch) -> Option<Vec<Change>> {
    let ops = enth_patch::ops_between(a, b)?;
    let mut scratch = a.clone();
    let mut out = Vec::with_capacity(ops.len());
    for op in ops {
        let touched = touches(&scratch, &op);
        let continuous = touched.as_ref().is_some_and(|list| {
            list.iter().all(|t| match t {
                Touch::Param(addr) => scratch.spec(addr).is_some_and(hint_moves),
                Touch::Macro(_) => true,
                Touch::Route(_) | Touch::Module(_) => false,
            })
        });
        enth_patch::apply(&mut scratch, &op).ok()?;
        out.push(Change {
            op,
            touches: touched,
            continuous,
        });
    }
    Some(out)
}

/// The moves from `a` to `b` (no metadata), or `None` when no ops lead there.
pub(crate) fn moves(a: &Patch, b: &Patch) -> Option<Vec<Change>> {
    changes(a, b).map(|list| list.into_iter().filter(Change::is_move).collect())
}

/// Whether `enth_solver::hint` may move a parameter of this spec (its own rule, `hint::space::movable`, with
/// transposition off, as the player's measured hints ask).
fn hint_moves(spec: &ParamSpec) -> bool {
    matches!(spec.kind, Kind::Float { .. })
        && !spec.flags.structural
        && !spec.flags.periodic
        && (spec.flags.modulatable || spec.flags.automatable)
        && spec.unit != Unit::Semitones
}

/// The op that sets `knob` to the plain `value` (a macro knob's position, 0–1).
pub(crate) fn set_knob(knob: &Knob, value: f32) -> Op {
    match knob {
        Knob::Param(addr) => Op::SetParam {
            addr: addr.clone(),
            value,
        },
        Knob::Macro(index) => Op::SetMacro {
            index: *index,
            value,
        },
    }
}

/// What a `touched` predicate would see if the player turned every control straight from its value in `a` to its
/// value in `b` (`--cross` has no edit log).
pub(crate) fn touch_log(a: &Patch, b: &Patch) -> TouchLog {
    let mut log = TouchLog::default();
    for c in changes(a, b).unwrap_or_default() {
        let knob = match &c.op {
            Op::SetParam { addr, .. } => Knob::Param(addr.clone()),
            Op::SetMacro { index, .. } => Knob::Macro(*index),
            _ => continue,
        };
        if let (Some(spec), Some(from), Some(to)) = (knob.spec(a), knob.value(a), knob.value(b)) {
            log.note(knob.clone(), spec, from);
            log.note(knob, spec, to);
        }
    }
    log
}
