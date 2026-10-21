//! The host-parameter bridge: the 72 host parameters (8 macros, 64 automation slots) with permanent ids, normalised
//! host values ⇄ plain patch values through each target's `ParamSpec`, the "patch wins, write back only on a true
//! difference" rule, host gesture writes, and parameter names (lane S1s; SK and S2s build on it).
//!
//! # The rule (docs/design.md §4)
//!
//! Plain → normalised → plain is not a round trip: `from_norm(to_norm(v)) != v` for 33–82 % of values on Log, Power
//! and signed parameters (research report 01, F3). So:
//!
//! 1. **The patch wins at load.** The host's values restored with a state are never pulled into the patch during the
//!    load; the first [`Session::fold_host`] after a load only records them.
//! 2. **A host value is folded only when it changed since the session last saw it**, so a stale host value (after a
//!    slot reassignment, an in-session load, or a host that restored no parameters) never overwrites the patch.
//! 3. **And only when it truly differs** ([`fold_value`]): if the host's `n` equals `to_norm(p)` bit for bit the
//!    patch keeps its exact `p`; otherwise the candidate is `from_norm(n)` (canonical), written only if it differs
//!    from `p` bit for bit. So folding the same host value twice, or after a save and reload, never moves the patch.
//!
//! The engine resolves slot events with the same equality rule (lane E+): the host's `n` is the exact value its slot
//! table holds if `n == to_norm` of it, else `from_norm(n)`. That table holds the value the running plan was compiled
//! with, or the latest editor value event, which is *not* always the patch's current value: an earlier fold moved
//! the patch and not the plan (F1b review, issue 4). So [`Session::fold_host`] resolves against the plan's value
//! ([`fold_against`]): it remembers, per address a fold moved, the value the engine still holds, until a plan is
//! sent or the patch's value changes some other way. A host that returns exactly to `to_norm` of the compiled value
//! therefore leaves the patch on that exact value, the one the engine plays.
//!
//! # Ids
//!
//! The host sees 72 `FloatParam`s, linear 0–1, with the permanent string ids `macro_1`…`macro_8` and
//! `slot_1`…`slot_64` (docs/naming.md; nice-plug hashes them to the 31-bit ids hosts store). [`HostParam`] is the
//! session's name for one of them; [`HOST_PARAM_IDS`] is the table, dense index 0–71 (macros first).

use enth_params::ParamSpec;
use enth_patch::catalogue::MACRO_VALUE;
use enth_patch::model::{MACROS, SLOTS};
use enth_patch::ops::canonical_value;
use enth_patch::{MacroIndex, ModuleId, Op, ParamAddr, Patch, SlotIndex, Target};

use crate::model::Session;

/// Host parameters in all: 8 macros + 64 slots.
pub const HOST_PARAMS: usize = MACROS + SLOTS;

/// A slot's host default: nice-plug fixes defaults at construction, so the host's "reset to default" is the
/// midpoint of whatever the slot targets.
pub const SLOT_DEFAULT_NORM: f32 = 0.5;

/// A macro's host default (a macro's host value is its patch value, 0–1).
pub const MACRO_DEFAULT_NORM: f32 = 0.0;

/// Between a module's title and a parameter name in a slot name: "Osc 1 · Cutoff".
pub const NAME_SEPARATOR: &str = " · ";

/// The permanent host parameter ids, by dense index ([`HostParam::index`]): the 8 macros, then the 64 slots.
/// Changing one breaks every saved project's automation (docs/naming.md); a test pins them.
pub static HOST_PARAM_IDS: [&str; HOST_PARAMS] = [
    "macro_1", "macro_2", "macro_3", "macro_4", "macro_5", "macro_6", "macro_7", "macro_8", //
    "slot_1", "slot_2", "slot_3", "slot_4", "slot_5", "slot_6", "slot_7", "slot_8", //
    "slot_9", "slot_10", "slot_11", "slot_12", "slot_13", "slot_14", "slot_15", "slot_16", //
    "slot_17", "slot_18", "slot_19", "slot_20", "slot_21", "slot_22", "slot_23", "slot_24", //
    "slot_25", "slot_26", "slot_27", "slot_28", "slot_29", "slot_30", "slot_31", "slot_32", //
    "slot_33", "slot_34", "slot_35", "slot_36", "slot_37", "slot_38", "slot_39", "slot_40", //
    "slot_41", "slot_42", "slot_43", "slot_44", "slot_45", "slot_46", "slot_47", "slot_48", //
    "slot_49", "slot_50", "slot_51", "slot_52", "slot_53", "slot_54", "slot_55", "slot_56", //
    "slot_57", "slot_58", "slot_59", "slot_60", "slot_61", "slot_62", "slot_63", "slot_64", //
];

/// One of the 72 host parameters.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum HostParam {
    /// A macro knob (`macro_1`…`macro_8`). Its host value is its patch value; it also takes CLAP poly-mod.
    Macro(MacroIndex),
    /// An automation slot (`slot_1`…`slot_64`), driving whatever `Patch::slots` assigns it to.
    Slot(SlotIndex),
}

impl HostParam {
    /// The dense index 0–71: macros 0–7, slots 8–71 (the order of [`HOST_PARAM_IDS`] and of the plugin's params).
    pub const fn index(self) -> usize {
        match self {
            HostParam::Macro(i) => i.get(),
            HostParam::Slot(s) => MACROS + s.get(),
        }
    }

    /// The parameter at a dense index.
    pub const fn from_index(index: usize) -> Option<HostParam> {
        if index < MACROS {
            match MacroIndex::new(index) {
                Some(i) => Some(HostParam::Macro(i)),
                None => None,
            }
        } else {
            match SlotIndex::new(index - MACROS) {
                Some(s) => Some(HostParam::Slot(s)),
                None => None,
            }
        }
    }

    /// The permanent string id (`"macro_1"`, `"slot_64"`).
    pub fn id(self) -> &'static str {
        HOST_PARAM_IDS[self.index()]
    }

    /// The parameter with this permanent id.
    pub fn from_id(id: &str) -> Option<HostParam> {
        HOST_PARAM_IDS
            .iter()
            .position(|&s| s == id)
            .and_then(HostParam::from_index)
    }

    /// All 72, in dense order.
    pub fn all() -> impl Iterator<Item = HostParam> {
        (0..HOST_PARAMS).filter_map(HostParam::from_index)
    }

    /// The host's default normalised value.
    pub const fn default_norm(self) -> f32 {
        match self {
            HostParam::Macro(_) => MACRO_DEFAULT_NORM,
            HostParam::Slot(_) => SLOT_DEFAULT_NORM,
        }
    }
}

/// The 72 normalised host values (unmodulated), as the shell reads them from nice-plug's params.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HostValues {
    /// `macro_1`…`macro_8`.
    pub macros: [f32; MACROS],
    /// `slot_1`…`slot_64`.
    pub slots: [f32; SLOTS],
}

impl Default for HostValues {
    /// Every parameter at its host default.
    fn default() -> Self {
        HostValues {
            macros: [MACRO_DEFAULT_NORM; MACROS],
            slots: [SLOT_DEFAULT_NORM; SLOTS],
        }
    }
}

impl HostValues {
    /// One value.
    pub fn get(&self, p: HostParam) -> f32 {
        match p {
            HostParam::Macro(i) => self.macros[i.get()],
            HostParam::Slot(s) => self.slots[s.get()],
        }
    }

    /// Set one value.
    pub fn set(&mut self, p: HostParam, norm: f32) {
        match p {
            HostParam::Macro(i) => self.macros[i.get()] = norm,
            HostParam::Slot(s) => self.slots[s.get()] = norm,
        }
    }
}

/// One step of a host gesture, for nice-plug's `ParamSetter`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GestureStep {
    /// `begin_set_parameter`.
    Begin,
    /// `set_parameter_normalized` with this value, 0–1.
    Set(f32),
    /// `end_set_parameter`.
    End,
}

/// A host write the shell forwards ([`ShellRequest::Host`](crate::model::ShellRequest::Host)).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HostWrite {
    /// Which parameter.
    pub param: HostParam,
    /// Which step.
    pub step: GestureStep,
}

/// What a host value means for the patch ([`fold_value`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Fold {
    /// The patch keeps its value.
    Keep,
    /// Write this plain value into the patch (no history).
    Write(f32),
}

/// The patch-wins rule for one value: `spec` is the target's spec, `host_norm` the host's normalised value,
/// `patch_plain` the patch's plain value. Non-finite host values are ignored (nice-plug #86: NaN from a host).
///
/// This is [`fold_against`] for a target the engine holds at the patch's own value.
pub fn fold_value(spec: &ParamSpec, host_norm: f32, patch_plain: f32) -> Fold {
    fold_against(spec, host_norm, patch_plain, patch_plain)
}

/// The patch-wins rule for a slot's target against the value **the engine's slot table holds** (lane S2s, F1b review
/// issue 4): `engine_plain` is the patch value the running plan was compiled with, or the latest editor value event
/// since. The engine resolves a host value `n` as exactly that value if `n` is bit-equal to its `to_norm`, else as
/// `from_norm(n)`; the fold applies the same rule, so the patch ends on the exact plain value the engine plays:
///
/// - `n == to_norm(engine_plain)` → the target is `engine_plain` itself (the host returned to the compiled value);
/// - otherwise → the canonical `from_norm(n)`;
///
/// and it is written only if it differs from `patch_plain` bit for bit. With `engine_plain == patch_plain` (nothing
/// moved the patch since the plan) this is the plain rule above. Non-finite host values are ignored.
pub fn fold_against(spec: &ParamSpec, host_norm: f32, patch_plain: f32, engine_plain: f32) -> Fold {
    if !host_norm.is_finite() {
        return Fold::Keep;
    }
    let n = host_norm.clamp(0.0, 1.0);
    let target = if n.to_bits() == spec.to_norm(engine_plain).to_bits() {
        engine_plain
    } else {
        match canonical_value(spec, spec.from_norm(n)) {
            Ok(plain) => plain,
            Err(_) => return Fold::Keep,
        }
    };
    if target.to_bits() == patch_plain.to_bits() {
        Fold::Keep
    } else {
        Fold::Write(target)
    }
}

/// The normalised value the host should show for a plain patch value: `to_norm`, the one pinned mapping (the engine's
/// slot table uses the same function, so the host's echo of it resolves to the exact plain value).
pub fn host_norm(spec: &ParamSpec, plain: f32) -> f32 {
    spec.to_norm(plain)
}

/// What a slot drives in a patch.
#[derive(Clone, Debug, PartialEq)]
pub struct SlotTarget {
    /// The slot.
    pub slot: SlotIndex,
    /// The parameter it drives.
    pub addr: ParamAddr,
    /// That parameter's spec (for value text and the norm mapping).
    pub spec: &'static ParamSpec,
}

/// What `slot` drives in `patch`, if it is assigned to a parameter that exists.
pub fn slot_target(patch: &Patch, slot: SlotIndex) -> Option<SlotTarget> {
    let addr = patch.slots[slot.get()].as_ref()?;
    let spec = patch.spec(addr)?;
    Some(SlotTarget {
        slot,
        addr: addr.clone(),
        spec,
    })
}

/// The slots assigned to `addr`, in slot order (two slots may drive one address; the higher one wins a fold).
pub fn slots_for<'a>(
    patch: &'a Patch,
    addr: &'a ParamAddr,
) -> impl Iterator<Item = SlotIndex> + 'a {
    patch
        .slots
        .iter()
        .enumerate()
        .filter(move |(_, s)| s.as_ref() == Some(addr))
        .filter_map(|(i, _)| SlotIndex::new(i))
}

/// The lowest unassigned slot, for the editor's "Automate" menu (which reuses an existing slot for the address
/// first: see [`slots_for`]).
pub fn free_slot(patch: &Patch) -> Option<SlotIndex> {
    patch
        .slots
        .iter()
        .position(Option::is_none)
        .and_then(SlotIndex::new)
}

/// The value each host parameter should show for `patch`: a macro's value, `to_norm` of a slot's target, and the
/// slot default for an empty slot. This is what a host sees after a fresh activate (nothing is pushed: the patch
/// wins and the host's own restored values are left alone).
pub fn host_values(patch: &Patch) -> HostValues {
    let mut v = HostValues::default();
    for (i, m) in patch.macros.iter().enumerate() {
        v.macros[i] = m.value;
    }
    for s in (0..SLOTS).filter_map(SlotIndex::new) {
        if let Some(t) = slot_target(patch, s)
            && let Some(plain) = patch.value(&t.addr)
        {
            v.slots[s.get()] = host_norm(t.spec, plain);
        }
    }
    v
}

/// The host writes an applied editor op implies, as (parameter, normalised value) pairs, appended to `out`, read
/// against the patch **after** the op: a `SetParam` on a slotted address sets each of its slots; a `SetMacro` sets
/// its macro; a `SetSlot` sets the slot to its new target's value (a reassignment sets at most one host value; a
/// cleared slot is left alone). Host folds and loads never produce writes.
pub fn host_updates(after: &Patch, op: &Op, out: &mut Vec<(HostParam, f32)>) {
    match op {
        Op::SetParam { addr, .. } => {
            let (Some(spec), Some(plain)) = (after.spec(addr), after.value(addr)) else {
                return;
            };
            for s in slots_for(after, addr) {
                out.push((HostParam::Slot(s), host_norm(spec, plain)));
            }
        }
        Op::SetMacro { index, .. } => {
            out.push((HostParam::Macro(*index), after.macros[index.get()].value));
        }
        Op::SetSlot {
            slot,
            addr: Some(_),
        } => {
            if let Some(t) = slot_target(after, *slot)
                && let Some(plain) = after.value(&t.addr)
            {
                out.push((HostParam::Slot(*slot), host_norm(t.spec, plain)));
            }
        }
        _ => {}
    }
}

/// The slots (bit `i` = slot index `i`) assigned, in `after`, to an address that `ops` set with `SetParam`: the
/// slots whose remembered host value the plan that delivers a recompiling transaction must forget
/// ([`enth_engine::Plan::forget_host_slots`]), so the engine plays the edited value rather than re-applying the old
/// host value at the swap. Read against the patch after the transaction.
pub fn slots_written(after: &Patch, ops: &[Op]) -> u64 {
    let mut mask = 0u64;
    for op in ops {
        if let Op::SetParam { addr, .. } = op {
            for s in slots_for(after, addr) {
                mask |= 1 << s.get();
            }
        }
    }
    mask
}

/// The display name of a host parameter: a macro's name or "Macro N"; a slot's target as "Osc 1 · Cutoff" (the
/// module's label, else its catalogue name and the number its id ends with), a built-in's parameter name
/// ("Glide", "Master level"), or "Slot N" when empty. Shown where the host allows renaming (CLAP with the fork's
/// rename patch); VST3 in REAPER keeps "Slot N".
pub fn host_param_name(patch: &Patch, p: HostParam) -> String {
    match p {
        HostParam::Macro(i) => patch.macros[i.get()]
            .name
            .clone()
            .unwrap_or_else(|| format!("Macro {}", i.one_based())),
        HostParam::Slot(s) => match slot_target(patch, s) {
            Some(t) => match t.addr.target() {
                // The built-ins' parameter names already say what they are ("Glide", "Master level").
                Target::Voice | Target::Master => t.spec.name.to_owned(),
                Target::Module(id) => {
                    format!("{}{NAME_SEPARATOR}{}", module_title(patch, id), t.spec.name)
                }
            },
            None => format!("Slot {}", s.one_based()),
        },
    }
}

/// The display names of all 72 host parameters for `patch`, in dense order ([`HostParam::index`]): what the shell
/// sets on a [`ShellRequest::RenameParams`](crate::model::ShellRequest::RenameParams).
pub fn host_param_names(patch: &Patch) -> Vec<String> {
    HostParam::all()
        .map(|p| host_param_name(patch, p))
        .collect()
}

/// Whether the host would see a different set of parameters in `after` than in `before`: any of the 72 display names
/// differs, or a slot drives another kind of parameter (its value text and range differ even when a name does not).
/// The session pushes [`ShellRequest::RenameParams`](crate::model::ShellRequest::RenameParams) only when this holds
/// (and the op could change names at all: [`crate::apply::renames_params`]), so a label on a module no slot drives
/// never makes the host rescan.
pub fn host_layout_changed(before: &Patch, after: &Patch) -> bool {
    HostParam::all().any(|p| {
        let spec = |patch: &Patch| match p {
            HostParam::Slot(s) => slot_target(patch, s).map(|t| std::ptr::from_ref(t.spec)),
            HostParam::Macro(_) => None,
        };
        host_param_name(before, p) != host_param_name(after, p) || spec(before) != spec(after)
    })
}

/// A module's title: its label, else its catalogue name plus the number its id ends with ("filter2" → "Filter 2";
/// an id with no number → "Filter (bass)").
fn module_title(patch: &Patch, id: &ModuleId) -> String {
    let Some((_, m)) = patch.find(id) else {
        return id.to_string();
    };
    if let Some(label) = &m.label {
        return label.clone();
    }
    let name = m.kind.spec().name;
    let digits = id
        .as_str()
        .trim_start_matches(|c: char| !c.is_ascii_digit());
    if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) {
        format!("{name} {digits}")
    } else {
        format!("{name} ({id})")
    }
}

/// What the session last saw of the host's values (rule 2 of the [module docs](self)), and what the engine holds
/// where a fold moved the patch away from it.
#[derive(Debug, Default)]
pub(crate) struct HostSync {
    /// `None` after construction or a load: the next fold records the host's values and writes nothing.
    pub(crate) seen: Option<HostValues>,
    /// Addresses a fold moved off the value the engine holds ([`FoldMemo`]). Empty while detached.
    pub(crate) memos: Vec<FoldMemo>,
}

/// A fold wrote `written` into the patch for `addr`, while the running plan still holds `engine` for it (the value it
/// was compiled with, or the latest editor value event): the reference the next fold of that address resolves the
/// host's value against, exactly as the engine's slot table does ([`fold_against`]). It lasts until something else
/// settles the two: a plan sent (a new epoch: it is compiled from the patch), or any other change of the patch's
/// value (an edit, undo or redo sent the engine an event, and the engine's value became the patch's).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct FoldMemo {
    pub(crate) addr: ParamAddr,
    pub(crate) engine: f32,
    pub(crate) written: f32,
    pub(crate) epoch: u64,
}

/// What [`Session::fold_host`] did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct FoldReport {
    /// The host's values were only recorded (the first fold after construction or a load): the patch won.
    pub baseline: bool,
    /// Values written into the patch (no history).
    pub written: u32,
}

impl Session {
    /// Fold the host's values into the patch, without history (rules 1–3 of the [module docs](self)). The editor
    /// glue calls it once per frame before the editor reads the patch; [`Session::save_patch`] calls it before
    /// writing. A written value needs no engine event: the engine already follows the host through slot and macro
    /// events from `process()`.
    pub fn fold_host(&mut self, host: &HostValues) -> FoldReport {
        let epoch = self.port.as_ref().map(|p| p.link.epoch());
        let patch = &self.patch;
        // A memo lives while its plan is the latest sent and the patch still holds what the fold wrote.
        self.host.memos.retain(|m| {
            epoch == Some(m.epoch)
                && patch
                    .value(&m.addr)
                    .is_some_and(|v| v.to_bits() == m.written.to_bits())
        });
        let Some(seen) = self.host.seen.replace(*host) else {
            return FoldReport {
                baseline: true,
                written: 0,
            };
        };
        let mut report = FoldReport::default();
        for p in HostParam::all() {
            let n = host.get(p);
            if n.to_bits() == seen.get(p).to_bits() {
                continue;
            }
            let (op, memo) = match p {
                HostParam::Macro(index) => {
                    match fold_value(&MACRO_VALUE, n, self.patch.macros[index.get()].value) {
                        Fold::Write(value) => (Op::SetMacro { index, value }, None),
                        Fold::Keep => continue,
                    }
                }
                HostParam::Slot(s) => {
                    let Some(t) = slot_target(&self.patch, s) else {
                        continue;
                    };
                    let Some(plain) = self.patch.value(&t.addr) else {
                        continue;
                    };
                    // The engine resolves the host value against the plan's value, not the patch's.
                    let engine = self
                        .host
                        .memos
                        .iter()
                        .find(|m| m.addr == t.addr)
                        .map_or(plain, |m| m.engine);
                    match fold_against(t.spec, n, plain, engine) {
                        Fold::Write(value) => (
                            Op::SetParam {
                                addr: t.addr.clone(),
                                value,
                            },
                            Some((t.addr, engine, value)),
                        ),
                        Fold::Keep => continue,
                    }
                }
            };
            // A canonical in-range value on an existing target always applies; a failure leaves the patch as is.
            if enth_patch::apply(&mut self.patch, &op).is_ok() {
                report.written += 1;
                self.game_note_fold(&op); // a live Recreate round logs it (lane SG, crate::game)
                if let (Some((addr, engine, written)), Some(epoch)) = (memo, epoch) {
                    self.host.note_fold(addr, engine, written, epoch);
                }
            }
        }
        if report.written > 0 {
            self.learn_note_fold(); // a live lesson's player sees host automation (lane SL, crate::learn)
        }
        report
    }

    /// The value each host parameter should show for the current patch ([`host_values`]).
    pub fn host_values(&self) -> HostValues {
        host_values(&self.patch)
    }

    /// Forget what the host showed: the next [`Session::fold_host`] only records (the patch wins). Called by every
    /// load, which also replaces the plan (nothing is left to remember about the old one).
    pub(crate) fn reset_host_baseline(&mut self) {
        self.host.seen = None;
        self.host.memos.clear();
    }
}

impl HostSync {
    /// A fold wrote `written` for `addr` while the engine holds `engine`: remember the pair, or forget it when the
    /// two are the same value again.
    fn note_fold(&mut self, addr: ParamAddr, engine: f32, written: f32, epoch: u64) {
        let known = self.memos.iter().position(|m| m.addr == addr);
        if written.to_bits() == engine.to_bits() {
            if let Some(i) = known {
                self.memos.swap_remove(i);
            }
            return;
        }
        let memo = FoldMemo {
            addr,
            engine,
            written,
            epoch,
        };
        match known {
            Some(i) => self.memos[i] = memo,
            None => self.memos.push(memo),
        }
    }
}
