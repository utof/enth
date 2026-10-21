//! **Owner: lane CV2** (Challenge v2, freeze point F1l; the F1l judge reviews it with the lesson model and the
//! session's Learn API). Pre-declared by lane S0l (R2). The reference is docs/challenge.md, "Challenge v2".
//!
//! Challenge v1 has only `Target::Seed`, `Start::Template`, an empty `Constraints` and `AnswerKind::Patch`, so it
//! cannot express a lesson. Challenge v2 adds, to the same types in [`crate::model`]:
//!
//! | Piece | What | Here |
//! | --- | --- | --- |
//! | `Target::Patch` | an explicit target patch, held inline | [`InlinePatch`] |
//! | `Target::Draw` | a seeded draw over a parameter and route list with value sets or ranges, over the start | [`Draw`] |
//! | `Start::Patch` | the step's own start patch, held inline | [`InlinePatch`] |
//! | `Constraints { editable, locked }` | what the player is meant to change; what the session refuses | [`Constraints`], [`Selector`] |
//!
//! Left out on purpose (the F1l seams, decided with lanes LM and SL through the coordinator): `AnswerKind::Choice`
//! and `Target::Expectations`. Lessons ask ear questions with `enth-learn`'s `StepKind::Choose` and check sound with
//! its `Check::Sound { phrase, expect }`, so a challenge variant would be a second representation of each. A later
//! version adds either when something plays it.
//!
//! **Versions.** One `Challenge` type reads both versions through one strict path ([`read`]): `from_json` and the
//! embedded serde path alike check the schema, refuse a newer one, decode with unknown fields refused (v1 through its
//! frozen v1 shapes, so a v2 field in a v1 file is unknown), migrate v1 (no change of meaning: a seed target, a
//! template start, no constraint) and validate. **A challenge is written at the lowest version that holds it**
//! ([`Challenge::version`]): a generated round's challenge stays `enth.challenge/1`, byte for byte as before (the
//! golden challenges, `enth compare`, an older build reading a challenge file), and one with any v2 piece is
//! `enth.challenge/2`.
//!
//! **Integers only, except patch values.** The challenge's own fields stay integers (a seed through a float could
//! change). An inline patch is a patch file with its own schema, version and migration, read through
//! `enth_patch::file::load_value` and written through the typed `PatchFile` (shortest `f32` text, exact both ways),
//! and a draw's values are plain values in the patch file's own value form (numbers in real units, enum keys, unit
//! text, `true`/`false`), resolved by the catalogue exactly as a patch file's are. Both are `f32` patch values, the
//! same bits on every platform.

use core::fmt;
use core::str::FromStr;

use enth_dsp::util::rng::{derive, fnv1a64};
use enth_params::{DEPTH, Kind, ParamSpec};
use enth_patch::file::{RawValue, resolve_value};
use enth_patch::ops::canonical_value;
use enth_patch::{
    MacroIndex, ModSource, ModuleId, ModuleKind, Op, ParamAddr, Patch, Route, RouteAddr, apply,
};
use serde::de::Error as _;
use serde::ser::SerializeStruct;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;

use crate::model::{
    AnswerKind, Assists, CHALLENGE_KIND, CHALLENGE_VERSION, Challenge, ChallengeError,
    ChallengeMeta, Phrase, PhraseId, ScoringSpec, Start, Target,
};
use crate::priors::{self, Jnd, ROUTE_JND};
use crate::random::{self, Drawn, Generated, Randomized, observable};
use crate::spec::{Category, MAX_SEED, RandomSpec};

/// The `schema` string of a v1 challenge.
pub const SCHEMA_V1: &str = "enth.challenge/1";

/// The `schema` string of a v2 challenge.
pub const SCHEMA_V2: &str = "enth.challenge/2";

/// The `generator` of a [`Generated`] that no generator version made: a prepared round on a patch or draw target
/// fills the same shape (its target, phrase, what it varies, its fingerprint), with level 0 and no minimum steps.
pub const NOT_GENERATED: u16 = 0;

// ---------------------------------------------------------------------------------------------------------------
// Selectors and constraints
// ---------------------------------------------------------------------------------------------------------------

/// What a lock or an edit list names (one typed selector for every lane: the challenge's [`Constraints`], lane LM's
/// step view, lane SL's enforcement). Text form, parsed strictly and printed canonically:
///
/// | Text | Selector | Covers |
/// | --- | --- | --- |
/// | `filter1.cutoff`, `voice.glide`, `master.level` | [`Selector::Param`] | that parameter |
/// | `filter1.*`, `voice.*`, `master.*` | [`Selector::Module`] | the module (or built-in): its parameters, the module itself (enable, remove, move, label), routes from it and routes into its parameters |
/// | `lfo1 -> filter1.cutoff` | [`Selector::Route`] | that route (adding, removing it, its depth) |
/// | `macro3` | [`Selector::Macro`] | that macro's knob |
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Selector {
    /// One parameter.
    Param(ParamAddr),
    /// A whole module, `voice` or `master`.
    Module(enth_patch::Target),
    /// One route.
    Route(RouteAddr),
    /// One macro knob.
    Macro(MacroIndex),
}

/// One thing an edit changes, for [`Selector::covers`] and [`Constraints`]. [`touches`] maps each op to what it
/// touches: the one table the challenge's lock check, lane LM's player (Show, the measured hints' free set) and lane
/// SL's edit path all use (F1l review M1).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Touch {
    /// A parameter's value (or its effect: a route into it, a module's enable flag).
    Param(ParamAddr),
    /// A route (added, removed, or its depth).
    Route(RouteAddr),
    /// A module itself (added, enabled, removed, moved, relabelled, its role) or a built-in.
    Module(enth_patch::Target),
    /// A macro's knob (or its name).
    Macro(MacroIndex),
}

impl fmt::Display for Touch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Touch::Param(addr) => write!(f, "{addr}"),
            Touch::Route(route) => write!(f, "{route}"),
            Touch::Module(target) => write!(f, "{target}"),
            Touch::Macro(index) => write!(f, "macro{}", index.one_based()),
        }
    }
}

/// What `op` changes, applied to `before` (the patch as the ops before it leave it). `None` for an op kind this build
/// does not know (`Op` is `non_exhaustive`): refuse it while anything is locked (fail closed).
///
/// | Op | Touches |
/// | --- | --- |
/// | `SetParam` | the parameter |
/// | `InsertRoute`, `RemoveRoute`, `SetDepth` | the route, and its destination parameter (modulation moves it) |
/// | `SetMacro` | the macro, and each route from it with its destination |
/// | `SetMacroName` | the macro |
/// | `AddModule`, `RemoveModule`, `SetEnabled` | the module, every parameter it has and every route from or into it |
/// | `Move`, `SetLabel` | the module |
/// | `SetAmpEnv` | the new amplitude envelope and the old one |
/// | `SetSlot` | the slot's new target and its old one (a slot lets the host move a parameter) |
/// | `SetMeta` | nothing |
///
/// One table for every lock check (F1l review M1): [`Challenge::validate`] refuses a lock on anything the target's
/// edits touch, lane SL's session refuses an edit that touches a lock, and lane LM's player picks Show's op and the
/// measured hints' free set with it, so none of the three can accept what another refuses.
pub fn touches(before: &Patch, op: &Op) -> Option<Vec<Touch>> {
    let mut out = Vec::new();
    match op {
        Op::SetParam { addr, .. } => out.push(Touch::Param(addr.clone())),
        Op::AddModule { module, .. } => {
            module_whole(&mut out, before, &module.id, Some(module.kind));
        }
        Op::RemoveModule { id } | Op::SetEnabled { id, .. } => {
            module_whole(&mut out, before, id, None);
        }
        Op::Move { id, .. } | Op::SetLabel { id, .. } => {
            out.push(Touch::Module(enth_patch::Target::Module(id.clone())));
        }
        Op::SetAmpEnv { id } => {
            out.push(Touch::Module(enth_patch::Target::Module(id.clone())));
            out.push(Touch::Module(enth_patch::Target::Module(
                before.voice.amp_env.clone(),
            )));
        }
        Op::InsertRoute { route, .. } => route_touches(
            &mut out,
            &RouteAddr {
                src: route.src.clone(),
                dst: route.dst.clone(),
            },
        ),
        Op::RemoveRoute { route } | Op::SetDepth { route, .. } => route_touches(&mut out, route),
        Op::SetMacro { index, .. } => {
            out.push(Touch::Macro(*index));
            for r in before
                .routes
                .iter()
                .filter(|r| r.src == ModSource::Macro(*index))
            {
                route_touches(
                    &mut out,
                    &RouteAddr {
                        src: r.src.clone(),
                        dst: r.dst.clone(),
                    },
                );
            }
        }
        Op::SetMacroName { index, .. } => out.push(Touch::Macro(*index)),
        Op::SetSlot { slot, addr } => {
            out.extend(addr.iter().cloned().map(Touch::Param));
            out.extend(before.slots[slot.get()].iter().cloned().map(Touch::Param));
        }
        Op::SetMeta { .. } => {}
        _ => return None,
    }
    Some(out)
}

fn route_touches(out: &mut Vec<Touch>, route: &RouteAddr) {
    out.push(Touch::Route(route.clone()));
    out.push(Touch::Param(route.dst.clone()));
}

/// A module, all its parameters (from `kind`, or its kind in `before`) and every route from or into it.
fn module_whole(out: &mut Vec<Touch>, before: &Patch, id: &ModuleId, kind: Option<ModuleKind>) {
    out.push(Touch::Module(enth_patch::Target::Module(id.clone())));
    let kind = kind.or_else(|| before.find(id).map(|(_, m)| m.kind));
    if let Some(kind) = kind {
        out.extend(
            kind.params()
                .iter()
                .filter_map(|spec| ParamAddr::module(id.clone(), spec.key).ok())
                .map(Touch::Param),
        );
    }
    let source = ModSource::Module(id.clone());
    out.extend(
        before
            .routes
            .iter()
            .filter(|r| r.dst.module_id() == Some(id) || r.src == source)
            .map(|r| {
                Touch::Route(RouteAddr {
                    src: r.src.clone(),
                    dst: r.dst.clone(),
                })
            }),
    );
}

impl Selector {
    /// Whether this selector covers `touch` (see the [type docs](Selector)).
    pub fn covers(&self, touch: &Touch) -> bool {
        match (self, touch) {
            (Selector::Param(a), Touch::Param(b)) => a == b,
            (Selector::Route(a), Touch::Route(b)) => a == b,
            (Selector::Macro(a), Touch::Macro(b)) => a == b,
            (Selector::Module(t), Touch::Module(u)) => t == u,
            (Selector::Module(t), Touch::Param(a)) => a.target() == t,
            (Selector::Module(t), Touch::Route(r)) => {
                r.dst.target() == t
                    || matches!((t, &r.src), (enth_patch::Target::Module(id), ModSource::Module(src)) if id == src)
            }
            _ => false,
        }
    }

    /// Check that the selector names something `start` has: a parameter of it, a module of it (or a built-in), a
    /// route whose source exists and whose destination is a modulatable parameter of it (the route itself may be
    /// added later), or a macro. A lock on nothing would silently lock nothing.
    pub fn check_against(&self, start: &Patch) -> Result<(), ChallengeError> {
        let bad = |reason: &str| ChallengeError::Selector {
            selector: self.to_string(),
            reason: reason.to_owned(),
        };
        match self {
            Selector::Param(addr) => start
                .spec(addr)
                .map(drop)
                .ok_or_else(|| bad("the start patch has no such parameter")),
            Selector::Module(enth_patch::Target::Module(id)) => start
                .find(id)
                .map(drop)
                .ok_or_else(|| bad("the start patch has no such module")),
            Selector::Module(_) | Selector::Macro(_) => Ok(()),
            Selector::Route(route) => check_route(start, route).map_err(|e| bad(&e)),
        }
    }
}

/// A route `start` could hold: its source exists (a module of the start, or a built-in source) and its destination
/// is a modulatable parameter of the start.
fn check_route(start: &Patch, route: &RouteAddr) -> Result<(), String> {
    if let ModSource::Module(id) = &route.src
        && start.find(id).is_none()
    {
        return Err(format!("the start patch has no module {id}"));
    }
    match start.spec(&route.dst) {
        None => Err(format!("the start patch has no parameter {}", route.dst)),
        Some(spec) if !spec.flags.modulatable => Err(format!("{} cannot be modulated", route.dst)),
        Some(_) => Ok(()),
    }
}

impl fmt::Display for Selector {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Selector::Param(a) => write!(f, "{a}"),
            Selector::Module(t) => write!(f, "{t}.*"),
            Selector::Route(r) => write!(f, "{r}"),
            Selector::Macro(i) => write!(f, "macro{}", i.one_based()),
        }
    }
}

impl FromStr for Selector {
    type Err = String;

    /// Strict: the text must be the selector's canonical form (what [`Display`](fmt::Display) prints), so a
    /// challenge's bytes survive a read and a write. `lfo1->filter1.cutoff` is refused with the canonical spelling in
    /// the message.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let parsed = if text.contains("->") {
            text.parse::<RouteAddr>()
                .map(Selector::Route)
                .map_err(|e| e.to_string())?
        } else if let Some(owner) = text.strip_suffix(".*") {
            Selector::Module(match owner {
                "voice" => enth_patch::Target::Voice,
                "master" => enth_patch::Target::Master,
                id => enth_patch::Target::Module(ModuleId::new(id).map_err(|e| e.to_string())?),
            })
        } else if !text.contains('.') && text.starts_with("macro") {
            match text.parse::<ModSource>().map_err(|e| e.to_string())? {
                ModSource::Macro(i) => Selector::Macro(i),
                _ => return Err(format!("'{text}' is not a selector")),
            }
        } else {
            text.parse::<ParamAddr>()
                .map(Selector::Param)
                .map_err(|e| e.to_string())?
        };
        let canonical = parsed.to_string();
        if canonical != text {
            return Err(format!("write '{text}' as '{canonical}'"));
        }
        Ok(parsed)
    }
}

impl Serialize for Selector {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Selector {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let text = String::deserialize(d)?;
        text.parse().map_err(|e: String| {
            D::Error::custom(format!(
                "selector '{text}': {e}; a selector is filter1.cutoff, filter1.*, lfo1 -> filter1.cutoff or macro3"
            ))
        })
    }
}

/// What the player is meant to change, and what the session refuses (design.md §6; docs/learn.md, "Locks are the
/// session's rule"). v1's `{}` is "no constraint": both lists empty.
///
/// - `locked` is **enforced**: the session refuses an op that touches anything a locked selector covers
///   (`SessionError::Locked`, lane SL), and the overlay draws the lock. Locked always wins.
/// - `editable` is **advisory**: the controls the step is about. The editor emphasises them, lane CLS's "How close am
///   I?" searches only them, and `lesson-check` requires the witness to touch only them; the session does not refuse
///   an edit outside them (the round is scored on sound either way). Empty means "no list".
///
/// JSON: `{"editable":["filter1.cutoff"],"locked":["filter1.resonance"]}`; each list is written only when it is not
/// empty, and a missing list reads as empty, so `{}` reads in both versions.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Constraints {
    /// What the step is about (advisory).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub editable: Vec<Selector>,
    /// What the session refuses to change (enforced).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub locked: Vec<Selector>,
}

impl Constraints {
    /// No constraint at all (v1's `{}`).
    pub fn is_empty(&self) -> bool {
        self.editable.is_empty() && self.locked.is_empty()
    }

    /// The first locked selector that covers `touch`: what the session names when it refuses the op.
    pub fn lock_of(&self, touch: &Touch) -> Option<&Selector> {
        self.locked.iter().find(|s| s.covers(touch))
    }

    /// Whether the session refuses `touch`.
    pub fn is_locked(&self, touch: &Touch) -> bool {
        self.lock_of(touch).is_some()
    }

    /// Whether `touch` is one of the step's controls: not locked, and covered by `editable` (or `editable` is empty).
    pub fn is_editable(&self, touch: &Touch) -> bool {
        !self.is_locked(touch)
            && (self.editable.is_empty() || self.editable.iter().any(|s| s.covers(touch)))
    }

    /// Check every selector against the start patch ([`Selector::check_against`]), and that no selector is listed
    /// twice in one list.
    pub fn check_against(&self, start: &Patch) -> Result<(), ChallengeError> {
        for list in [&self.editable, &self.locked] {
            for (i, s) in list.iter().enumerate() {
                s.check_against(start)?;
                if list[..i].contains(s) {
                    return Err(ChallengeError::Selector {
                        selector: s.to_string(),
                        reason: "listed twice".to_owned(),
                    });
                }
            }
        }
        Ok(())
    }
}

/// The edits the target asks of the player, from the start: one op per thing it varies, as the player would make it
/// (a parameter's value; a route's depth, or the route added or removed; a module enabled, added or removed; a macro
/// turned), each with what it varies, for the message. A draw's items, or a patch target's differences from the
/// start. A seed target's draws change with its seed, so [`Challenge::validate`] refuses any lock on one before this
/// runs, and it gives none here.
fn target_edits(challenge: &Challenge, start: &Patch) -> Vec<(String, Op)> {
    let route_op =
        |r: &RouteAddr, in_start: bool, in_target: bool, depth: f32| match (in_start, in_target) {
            (true, true) => Op::SetDepth {
                route: r.clone(),
                depth,
            },
            (true, false) => Op::RemoveRoute { route: r.clone() },
            (false, _) => Op::InsertRoute {
                index: 0,
                route: Route {
                    src: r.src.clone(),
                    dst: r.dst.clone(),
                    depth,
                },
            },
        };
    let has_route =
        |p: &Patch, r: &RouteAddr| p.routes.iter().any(|x| x.src == r.src && x.dst == r.dst);
    match &challenge.target {
        Target::Seed(_) => Vec::new(),
        Target::Draw(draw) => draw
            .items
            .iter()
            .map(|item| {
                let op = match &item.at {
                    DrawAt::Param(addr) => Op::SetParam {
                        addr: addr.clone(),
                        value: start.value(addr).unwrap_or(0.0),
                    },
                    DrawAt::Route(r) => route_op(r, has_route(start, r), true, 0.0),
                };
                (item.at.to_string(), op)
            })
            .collect(),
        Target::Patch(target) => patch_edits(start, target, route_op),
    }
}

/// [`target_edits`] for a patch target: modules the start lacks (or has as another kind) added, modules the target
/// lacks removed, enable flags, every parameter value, routes and macros that differ.
fn patch_edits(
    start: &Patch,
    target: &Patch,
    route_op: impl Fn(&RouteAddr, bool, bool, f32) -> Op,
) -> Vec<(String, Op)> {
    let mut out: Vec<(String, Op)> = Vec::new();
    let set = |out: &mut Vec<(String, Op)>, addr: ParamAddr, value: f32| {
        out.push((addr.to_string(), Op::SetParam { addr, value }));
    };
    for (_, s) in start.modules() {
        if target.find(&s.id).is_none_or(|(_, m)| m.kind != s.kind) {
            out.push((
                format!("the module {}", s.id),
                Op::RemoveModule { id: s.id.clone() },
            ));
        }
    }
    for (at, m) in target.modules() {
        match start.find(&m.id) {
            Some((_, s)) if s.kind == m.kind => {
                if s.enabled != m.enabled {
                    out.push((
                        format!("the module {}", m.id),
                        Op::SetEnabled {
                            id: m.id.clone(),
                            enabled: m.enabled,
                        },
                    ));
                }
                for (spec, (a, b)) in m.spec().params.iter().zip(m.values.iter().zip(&s.values)) {
                    if a.to_bits() != b.to_bits()
                        && let Ok(addr) = ParamAddr::module(m.id.clone(), spec.key)
                    {
                        set(&mut out, addr, *a);
                    }
                }
            }
            _ => out.push((
                format!("the module {}", m.id),
                Op::AddModule {
                    at,
                    module: m.clone(),
                },
            )),
        }
    }
    for (owner, target_values, start_values, specs) in [
        (
            enth_patch::Target::Voice,
            &target.voice.values[..],
            &start.voice.values[..],
            enth_patch::Builtin::Voice.params(),
        ),
        (
            enth_patch::Target::Master,
            &target.master[..],
            &start.master[..],
            enth_patch::Builtin::Master.params(),
        ),
    ] {
        for (spec, (a, b)) in specs.iter().zip(target_values.iter().zip(start_values)) {
            if a.to_bits() != b.to_bits()
                && let Ok(addr) = ParamAddr::new(owner.clone(), spec.key)
            {
                set(&mut out, addr, *a);
            }
        }
    }
    let depth = |p: &Patch, r: &RouteAddr| {
        p.routes
            .iter()
            .find(|x| x.src == r.src && x.dst == r.dst)
            .map(|x| x.depth)
    };
    let mut seen: Vec<RouteAddr> = Vec::new();
    for r in target.routes.iter().chain(&start.routes) {
        let addr = RouteAddr {
            src: r.src.clone(),
            dst: r.dst.clone(),
        };
        if seen.contains(&addr) {
            continue;
        }
        let (from, to) = (depth(start, &addr), depth(target, &addr));
        if from.map(f32::to_bits) != to.map(f32::to_bits) {
            let op = route_op(&addr, from.is_some(), to.is_some(), to.unwrap_or(0.0));
            out.push((addr.to_string(), op));
        }
        seen.push(addr);
    }
    for (i, (a, b)) in target.macros.iter().zip(&start.macros).enumerate() {
        if a.value.to_bits() != b.value.to_bits()
            && let Some(index) = MacroIndex::new(i)
        {
            out.push((
                format!("macro{}", index.one_based()),
                Op::SetMacro {
                    index,
                    value: a.value,
                },
            ));
        }
    }
    out
}

/// Refuse a lock on anything the target's edits touch ([`touches`], the session's own table): the session would
/// refuse the very edit the round asks for (`bad_selector`). A draw on `env2 -> filter1.cutoff` touches
/// `filter1.cutoff` too (modulation moves it), and a patch target that enables a module touches its parameters and
/// routes. Lesson 1.2's planned "a level-1 round with `osc1.wave` locked" is refused earlier (a seed target takes no
/// lock); a draw on `osc1.semi` says it instead.
pub(crate) fn check_locks(challenge: &Challenge, start: &Patch) -> Result<(), ChallengeError> {
    for (what, op) in target_edits(challenge, start) {
        // Every op built above is a kind this build has, so `touches` knows it.
        for touch in touches(start, &op).unwrap_or_default() {
            if let Some(lock) = challenge.constraints.lock_of(&touch) {
                let edit = if touch_text(&touch) == what {
                    String::new()
                } else {
                    format!(" (the round's edit of {what} changes it)")
                };
                return Err(ChallengeError::Selector {
                    selector: lock.to_string(),
                    reason: format!(
                        "it locks {}, which the target varies{edit}: the round could not be passed",
                        touch_text(&touch)
                    ),
                });
            }
        }
    }
    Ok(())
}

fn touch_text(touch: &Touch) -> String {
    match touch {
        Touch::Param(a) => a.to_string(),
        Touch::Route(r) => r.to_string(),
        Touch::Module(t) => format!("the module {t}"),
        Touch::Macro(i) => format!("macro{}", i.one_based()),
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Inline patches
// ---------------------------------------------------------------------------------------------------------------

/// A patch held inline in a challenge (`Target::Patch`, `Start::Patch`): a whole patch file as a JSON object,
/// `{"schema":1,"engine":"0.3.0",…}`. Read through `enth_patch::file::load_value` (an older patch schema migrates;
/// every value is checked), written through the typed `PatchFile` (shortest `f32` text, so it reads back to the
/// same bits and writes the same bytes).
///
/// `Eq` holds because a loaded patch has finite values only (the patch file refuses non-finite numbers), so `==` on
/// its `f32`s is an equivalence.
#[derive(Clone, Debug, PartialEq)]
pub struct InlinePatch(Box<Patch>);

impl Eq for InlinePatch {}

impl InlinePatch {
    /// Hold `patch` inline.
    pub fn new(patch: Patch) -> InlinePatch {
        InlinePatch(Box::new(patch))
    }

    /// The patch.
    pub fn patch(&self) -> &Patch {
        &self.0
    }

    /// The patch, owned.
    pub fn into_patch(self) -> Patch {
        *self.0
    }
}

impl From<Patch> for InlinePatch {
    fn from(patch: Patch) -> Self {
        InlinePatch::new(patch)
    }
}

impl core::ops::Deref for InlinePatch {
    type Target = Patch;
    fn deref(&self) -> &Patch {
        &self.0
    }
}

impl Serialize for InlinePatch {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.0.to_file().serialize(s)
    }
}

impl<'de> Deserialize<'de> for InlinePatch {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(d)?;
        enth_patch::file::load_value(value)
            .map(|loaded| InlinePatch::new(loaded.patch))
            .map_err(|e| D::Error::custom(format!("the inline patch: {e}")))
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Draws
// ---------------------------------------------------------------------------------------------------------------

/// The most items one draw holds.
pub const MAX_DRAW_ITEMS: usize = 16;

/// The most values one item's set holds.
pub const MAX_SET_VALUES: usize = 32;

/// The most attempts a draw makes before it forces a target (generator v1's [`random::MAX_ROUNDS`]).
pub const MAX_ATTEMPTS: u16 = random::MAX_ROUNDS;

/// A seeded draw over the start (`Target::Draw`, content/lessons/PLAN.md §1.3): the target is the challenge's start
/// with each item set to a value drawn from its set or range. A new seed gives a sibling for a redo or a parallel test
/// form. JSON: `{"kind":"draw","seed":7,"items":[{"param":"filter1.cutoff","set":[300.0,600.0,1200.0]},
/// {"route":"env2 -> filter1.cutoff","range":[-60.0,60.0]}]}`.
///
/// **How a target is drawn** ([`Draw::draw`]), through generator v1's keyed streams and just-noticeable steps
/// (design.md §9):
///
/// 1. Each item draws from its own stream, `derive(derive(seed, fnv1a64(key)), attempt)`, keyed by its address
///    (`filter1.cutoff`) or its route (`env2->filter1.cutoff`). A set picks one value (`bits % n`); a range draws
///    uniformly in the parameter's normalised scale (a cutoff log-uniformly) and keeps a reachable value (3
///    significant digits, discrete values exact, a route depth in 5 % steps), within the range.
/// 2. The values go onto the start through ops: a parameter by `SetParam`; a route by `SetDepth`, or inserted at the
///    end of the summation order when the start lacks it; **a depth of 0 is no route** (removed when the start has
///    it, never added).
/// 3. The target must be playable: every drawn parameter, and every drawn route with a non-zero depth, observable
///    through the challenge's phrase (lane RG's mask: nothing drawn is dead), and at least one item a just-noticeable
///    step or more from the start (so the start never passes by itself). Otherwise every item draws again at the next
///    attempt.
/// 4. After [`MAX_ATTEMPTS`] attempts the first item that can be moved a step from the start is set to its first
///    value that does so (the forced step), and if no such target is playable the draw fails (`draw_failed`).
///
/// **Route depths and their sign are drawn** (Q4, settled here): a route item's values are depths in percent of the
/// destination's range, −100 to 100, so a set such as `[-60.0,-30.0,30.0,60.0]` draws the sign too. A depth can also
/// be written as patch-op text, an interval at the start's value (`"+12st"` on `osc1.semi`, `"+1oct"` on a
/// cutoff), as `enth`'s op JSON reads it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Draw {
    /// The seed, at most 2⁵³ − 1 (a JSON number keeps it exactly).
    pub seed: u64,
    /// What is drawn, in order (the ops apply in this order).
    pub items: Vec<DrawItem>,
}

/// One item of a [`Draw`]: what is drawn and from what.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DrawItem {
    /// A parameter or a route.
    pub at: DrawAt,
    /// Its values.
    pub values: DrawValues,
}

/// What a [`DrawItem`] draws.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DrawAt {
    /// A parameter's value (`{"param":"filter1.cutoff",…}`).
    Param(ParamAddr),
    /// A route's depth in percent, signed (`{"route":"env2 -> filter1.cutoff",…}`).
    Route(RouteAddr),
}

impl DrawAt {
    /// The stream key: the address, or `<src>-><dst>` (the fingerprint's route key).
    pub fn key(&self) -> String {
        match self {
            DrawAt::Param(a) => a.to_string(),
            DrawAt::Route(r) => format!("{}->{}", r.src, r.dst),
        }
    }

    /// What an edit of it touches.
    pub fn touch(&self) -> Touch {
        match self {
            DrawAt::Param(a) => Touch::Param(a.clone()),
            DrawAt::Route(r) => Touch::Route(r.clone()),
        }
    }
}

impl fmt::Display for DrawAt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DrawAt::Param(a) => write!(f, "{a}"),
            DrawAt::Route(r) => write!(f, "{r}"),
        }
    }
}

/// The values an item draws from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DrawValues {
    /// One of these (`"set":[…]`), each equally likely: early lessons' 3–5 values.
    Set(Vec<DrawValue>),
    /// Anything between `low` and `high` (`"range":[low,high]`), uniform in the parameter's normalised scale:
    /// siblings and review. With `step` (`"step":1.0`, a number in the parameter's unit, or percent for a route),
    /// the value is `low` plus a whole number of steps (whole semitones in `[-24, 24]`); without it, a reachable
    /// value (3 significant digits, discrete values exact, route depths in 5 % steps).
    Range {
        /// The low end.
        low: DrawValue,
        /// The high end.
        high: DrawValue,
        /// The grid, from `low`.
        step: Option<DrawValue>,
    },
}

/// A plain value as a patch file writes it: a number in the parameter's unit (a route's depth in percent), text (an
/// enum key `"saw"`, a unit string `"1.2kHz"`, a depth interval `"+12st"`) or a toggle.
///
/// Equality is on the number's bits, so it is an equivalence (`Eq`) and two values are equal exactly when they write
/// the same text.
#[derive(Clone, Debug)]
pub enum DrawValue {
    /// A number in the parameter's unit.
    Number(f32),
    /// Text, resolved by the parameter's spec.
    Text(String),
    /// A toggle.
    Bool(bool),
}

impl PartialEq for DrawValue {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (DrawValue::Number(a), DrawValue::Number(b)) => a.to_bits() == b.to_bits(),
            (DrawValue::Text(a), DrawValue::Text(b)) => a == b,
            (DrawValue::Bool(a), DrawValue::Bool(b)) => a == b,
            _ => false,
        }
    }
}

impl Eq for DrawValue {}

impl fmt::Display for DrawValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DrawValue::Number(x) => write!(f, "{x}"),
            DrawValue::Text(t) => write!(f, "\"{t}\""),
            DrawValue::Bool(b) => write!(f, "{b}"),
        }
    }
}

impl Serialize for DrawValue {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            // As the patch file writes an f32: shortest round-trip text.
            DrawValue::Number(x) => s.serialize_f32(*x),
            DrawValue::Text(t) => s.serialize_str(t),
            DrawValue::Bool(b) => s.serialize_bool(*b),
        }
    }
}

impl<'de> Deserialize<'de> for DrawValue {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        // The patch file's own raw value (a number reads as an f32 exactly as a patch file's does).
        match RawValue::deserialize(d)? {
            RawValue::Num(x) if x.is_finite() => Ok(DrawValue::Number(x)),
            RawValue::Num(x) => Err(D::Error::custom(format!(
                "a draw value must be finite, found {x}"
            ))),
            RawValue::Text(t) => Ok(DrawValue::Text(t)),
            RawValue::Bool(b) => Ok(DrawValue::Bool(b)),
            RawValue::Other(v) => Err(D::Error::custom(format!(
                "a draw value is a number, text or true/false, found {v}"
            ))),
        }
    }
}

impl DrawValue {
    fn raw(&self) -> RawValue {
        match self {
            DrawValue::Number(x) => RawValue::Num(*x),
            DrawValue::Text(t) => RawValue::Text(t.clone()),
            DrawValue::Bool(b) => RawValue::Bool(*b),
        }
    }
}

/// The on-disk shape of a [`DrawItem`]: exactly one of `param` and `route`, exactly one of `set` and `range`, and
/// `step` only with `range`.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DrawItemFile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    param: Option<ParamAddrText>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    route: Option<RouteAddrText>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    set: Option<Vec<DrawValue>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    range: Option<[DrawValue; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    step: Option<DrawValue>,
}

/// A parameter address as canonical text.
#[derive(Clone)]
struct ParamAddrText(ParamAddr);

/// A route as canonical text (`lfo1 -> filter1.cutoff`).
#[derive(Clone)]
struct RouteAddrText(RouteAddr);

impl Serialize for ParamAddrText {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for ParamAddrText {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let text = String::deserialize(d)?;
        match Selector::from_str(&text).map_err(D::Error::custom)? {
            Selector::Param(a) => Ok(ParamAddrText(a)),
            _ => Err(D::Error::custom(format!(
                "'{text}' is not a parameter address (filter1.cutoff)"
            ))),
        }
    }
}

impl Serialize for RouteAddrText {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for RouteAddrText {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let text = String::deserialize(d)?;
        match Selector::from_str(&text).map_err(D::Error::custom)? {
            Selector::Route(r) => Ok(RouteAddrText(r)),
            _ => Err(D::Error::custom(format!(
                "'{text}' is not a route (lfo1 -> filter1.cutoff)"
            ))),
        }
    }
}

impl Serialize for DrawItem {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let (param, route) = match &self.at {
            DrawAt::Param(a) => (Some(ParamAddrText(a.clone())), None),
            DrawAt::Route(r) => (None, Some(RouteAddrText(r.clone()))),
        };
        let (set, range, step) = match &self.values {
            DrawValues::Set(v) => (Some(v.clone()), None, None),
            DrawValues::Range { low, high, step } => {
                (None, Some([low.clone(), high.clone()]), step.clone())
            }
        };
        DrawItemFile {
            param,
            route,
            set,
            range,
            step,
        }
        .serialize(s)
    }
}

impl<'de> Deserialize<'de> for DrawItem {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let f = DrawItemFile::deserialize(d)?;
        let at = match (f.param, f.route) {
            (Some(p), None) => DrawAt::Param(p.0),
            (None, Some(r)) => DrawAt::Route(r.0),
            _ => {
                return Err(D::Error::custom(
                    "a draw item names exactly one of \"param\" and \"route\"",
                ));
            }
        };
        let values = match (f.set, f.range, f.step) {
            (Some(set), None, None) => DrawValues::Set(set),
            (None, Some([low, high]), step) => DrawValues::Range { low, high, step },
            (Some(_), None, Some(_)) => {
                return Err(D::Error::custom(
                    "\"step\" goes with \"range\", not \"set\"",
                ));
            }
            _ => {
                return Err(D::Error::custom(
                    "a draw item has exactly one of \"set\" and \"range\"",
                ));
            }
        };
        Ok(DrawItem { at, values })
    }
}

/// An item with its values resolved against the start patch.
#[derive(Clone, Debug)]
struct Resolved {
    at: DrawAt,
    key: String,
    /// The parameter's spec, or [`DEPTH`] for a route.
    spec: &'static ParamSpec,
    jnd: Jnd,
    domain: Domain,
}

#[derive(Clone, Debug)]
enum Domain {
    Set(Vec<f32>),
    /// Low, high and the grid step (0 for none).
    Range(f32, f32, f32),
}

/// A drawn target ([`Draw::draw`]).
#[derive(Clone, Debug, PartialEq)]
pub struct Drawing {
    /// The target: the start with every item set.
    pub patch: Patch,
    /// Each item with its drawn plain value (a route's depth in percent; 0 is no route), in item order.
    pub values: Vec<(DrawAt, f32)>,
    /// The attempt that gave it (0 for the first).
    pub attempt: u16,
    /// Whether the step was forced after [`MAX_ATTEMPTS`] attempts.
    pub forced: bool,
}

impl Drawing {
    /// The drawn parameters, in address order: a round's randomized parameters (`TargetContext::randomized`).
    pub fn params(&self) -> Vec<ParamAddr> {
        let mut out: Vec<ParamAddr> = self
            .values
            .iter()
            .filter_map(|(at, _)| match at {
                DrawAt::Param(a) => Some(a.clone()),
                DrawAt::Route(_) => None,
            })
            .collect();
        out.sort();
        out
    }

    /// The drawn routes, in item order.
    pub fn routes(&self) -> Vec<RouteAddr> {
        self.values
            .iter()
            .filter_map(|(at, _)| match at {
                DrawAt::Route(r) => Some(r.clone()),
                DrawAt::Param(_) => None,
            })
            .collect()
    }
}

/// Why a draw is refused, or could not draw a target.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum DrawError {
    /// The seed is above 2⁵³ − 1 (`out_of_range`).
    SeedTooLarge {
        /// The seed.
        seed: u64,
    },
    /// No items, or more than [`MAX_DRAW_ITEMS`].
    ItemCount {
        /// How many.
        count: usize,
    },
    /// The same parameter or route twice.
    Repeated {
        /// The item.
        at: String,
    },
    /// The start patch has no such parameter, or the route cannot exist on it.
    NotOnStart {
        /// The item.
        at: String,
        /// Why.
        reason: String,
    },
    /// An empty set, or more than [`MAX_SET_VALUES`] values.
    SetSize {
        /// The item.
        at: String,
        /// How many values.
        count: usize,
    },
    /// A value that does not resolve for the item (out of range, a wrong unit, an unknown enum key).
    BadValue {
        /// The item.
        at: String,
        /// The value as written.
        value: String,
        /// The catalogue's reason.
        reason: String,
    },
    /// Two values of a set less than one just-noticeable step apart: they are the same target to the ear and to the
    /// rings (`draw_too_close`).
    TooClose {
        /// The item.
        at: String,
        /// The two values, resolved.
        values: [String; 2],
    },
    /// A range on an enum or a toggle (draw those from a set), or a range less than a step wide or upside down.
    BadRange {
        /// The item.
        at: String,
        /// Why.
        reason: String,
    },
    /// An op that sets a drawn value failed on the start.
    Apply {
        /// The item.
        at: String,
        /// The op's error.
        reason: String,
    },
    /// No attempt gave a playable target, and no forced step did either (`draw_failed`).
    NoTarget {
        /// What failed last: an item that is not heard, or a target the same as the start.
        reason: String,
    },
}

impl DrawError {
    /// A stable snake-case code.
    pub fn code(&self) -> &'static str {
        match self {
            DrawError::SeedTooLarge { .. } => "out_of_range",
            DrawError::TooClose { .. } => "draw_too_close",
            DrawError::NoTarget { .. } => "draw_failed",
            _ => "bad_draw",
        }
    }

    /// What to do about it, in one line.
    pub fn hint(&self) -> String {
        match self {
            DrawError::SeedTooLarge { .. } => format!("a draw's seed is at most {MAX_SEED}"),
            DrawError::ItemCount { .. } => {
                format!("a draw has 1 to {MAX_DRAW_ITEMS} items")
            }
            DrawError::Repeated { .. } => "name each parameter or route once".to_owned(),
            DrawError::NotOnStart { .. } => {
                "draw parameters the start patch has, and routes into modulatable ones".to_owned()
            }
            DrawError::SetSize { .. } => format!("a set has 1 to {MAX_SET_VALUES} values"),
            DrawError::BadValue { .. } => {
                "write values as a patch file does: numbers in the parameter's unit, enum keys, true/false".to_owned()
            }
            DrawError::TooClose { .. } => {
                "keep a set's values at least one just-noticeable step apart (lessons use two)".to_owned()
            }
            DrawError::BadRange { .. } => {
                "a range is [low, high], at least a step wide, on a number; draw enums and toggles from a set"
                    .to_owned()
            }
            DrawError::Apply { .. } => "this value cannot be set on the start patch".to_owned(),
            DrawError::NoTarget { .. } => "check that every drawn control is heard through the phrase, and that \
                                           some value differs from the start by a step"
                .to_owned(),
        }
    }
}

impl fmt::Display for DrawError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DrawError::SeedTooLarge { seed } => {
                write!(f, "the draw's seed {seed} is above 2^53 - 1")
            }
            DrawError::ItemCount { count } => write!(f, "a draw with {count} items"),
            DrawError::Repeated { at } => write!(f, "{at} is drawn twice"),
            DrawError::NotOnStart { at, reason }
            | DrawError::BadRange { at, reason }
            | DrawError::Apply { at, reason } => write!(f, "{at}: {reason}"),
            DrawError::SetSize { at, count } => write!(f, "{at}: a set of {count} values"),
            DrawError::BadValue { at, value, reason } => write!(f, "{at}: {value}: {reason}"),
            DrawError::TooClose { at, values } => write!(
                f,
                "{at}: {} and {} are less than a just-noticeable step apart",
                values[0], values[1]
            ),
            DrawError::NoTarget { reason } => write!(f, "no playable target: {reason}"),
        }
    }
}

impl std::error::Error for DrawError {}

/// The keyed stream of one item: `derive(derive(seed, fnv1a64(key)), attempt)`, as generator v1 draws.
fn stream(seed: u64, key: &str, attempt: u16) -> u64 {
    derive(derive(seed, fnv1a64(key.as_bytes())), u64::from(attempt))
}

/// 24 random bits as a uniform in `[0, 1)`, exact in `f32` (generator v1's `unit_of`).
const fn unit_of(bits: u64) -> f32 {
    (bits >> 40) as f32 / 16_777_216.0
}

/// A route depth in a range: 5 % steps, kept within the range (0 allowed: no route).
fn depth_in(lo: f32, hi: f32, depth: f32) -> f32 {
    let q = (libm::round(f64::from(depth) / 5.0) * 5.0) as f32;
    let q = q.clamp(lo, hi);
    if q == 0.0 { 0.0 } else { q }
}

/// `lo` plus the whole number of steps nearest `v`, within the range (in `f64`, then to `f32`).
fn on_grid(lo: f32, hi: f32, step: f32, v: f32) -> f32 {
    let (lo64, step64) = (f64::from(lo), f64::from(step));
    let k = libm::round((f64::from(v) - lo64) / step64);
    let q = ((lo64 + k * step64) as f32).clamp(lo, hi);
    if q == 0.0 { 0.0 } else { q }
}

/// The value of an item in a patch: a parameter's, or a route's depth (0 when the patch has no such route).
fn value_in(patch: &Patch, at: &DrawAt) -> Option<f32> {
    match at {
        DrawAt::Param(a) => patch.value(a),
        DrawAt::Route(r) => Some(
            patch
                .routes
                .iter()
                .find(|x| x.src == r.src && x.dst == r.dst)
                .map_or(0.0, |x| x.depth),
        ),
    }
}

impl Draw {
    /// Check the draw against the start patch it draws over: the seed, the item count, each item on the start once,
    /// every value resolving, sets of distinct values (a step apart), ranges on numbers at least a step wide.
    pub fn validate(&self, start: &Patch) -> Result<(), DrawError> {
        self.resolve(start).map(drop)
    }

    fn resolve(&self, start: &Patch) -> Result<Vec<Resolved>, DrawError> {
        if self.seed > MAX_SEED {
            return Err(DrawError::SeedTooLarge { seed: self.seed });
        }
        if self.items.is_empty() || self.items.len() > MAX_DRAW_ITEMS {
            return Err(DrawError::ItemCount {
                count: self.items.len(),
            });
        }
        let mut out: Vec<Resolved> = Vec::with_capacity(self.items.len());
        for item in &self.items {
            let at = item.at.to_string();
            if out.iter().any(|r| r.at == item.at) {
                return Err(DrawError::Repeated { at });
            }
            let not_on = |reason: String| DrawError::NotOnStart {
                at: at.clone(),
                reason,
            };
            let (spec, jnd, base, dst_spec): (&'static ParamSpec, Jnd, f32, Option<&ParamSpec>) =
                match &item.at {
                    DrawAt::Param(a) => {
                        let spec = start.spec(a).ok_or_else(|| {
                            not_on("the start patch has no such parameter".into())
                        })?;
                        (spec, priors::jnd(spec), 0.0, None)
                    }
                    DrawAt::Route(r) => {
                        check_route(start, r).map_err(not_on)?;
                        let dst = start.spec(&r.dst);
                        let base = start.value(&r.dst).unwrap_or(0.0);
                        (&DEPTH, ROUTE_JND, base, dst)
                    }
                };
            let resolve = |v: &DrawValue| -> Result<f32, DrawError> {
                let bad = |reason: String| DrawError::BadValue {
                    at: at.clone(),
                    value: v.to_string(),
                    reason,
                };
                match (&item.at, v, dst_spec) {
                    (DrawAt::Param(_), _, _) => {
                        resolve_value(spec, &v.raw()).map_err(|e| bad(e.to_string()))
                    }
                    (DrawAt::Route(_), DrawValue::Number(x), _) => {
                        canonical_value(&DEPTH, *x).map_err(|e| bad(e.to_string()))
                    }
                    (DrawAt::Route(_), DrawValue::Text(t), Some(dst)) => dst
                        .depth_from_text(base, t)
                        .map_err(|e| bad(e.to_string()))
                        .and_then(|d| canonical_value(&DEPTH, d).map_err(|e| bad(e.to_string()))),
                    (DrawAt::Route(_), _, _) => Err(bad(
                        "a route's depth is a number in percent or text such as \"+12st\"".into(),
                    )),
                }
            };
            let domain = match &item.values {
                DrawValues::Set(values) => {
                    if values.is_empty() || values.len() > MAX_SET_VALUES {
                        return Err(DrawError::SetSize {
                            at,
                            count: values.len(),
                        });
                    }
                    let plain = values
                        .iter()
                        .map(resolve)
                        .collect::<Result<Vec<f32>, _>>()?;
                    for (i, a) in plain.iter().enumerate() {
                        if let Some(b) = plain[..i].iter().find(|b| !jnd.exceeded(**b, *a)) {
                            return Err(DrawError::TooClose {
                                at,
                                values: [b.to_string(), a.to_string()],
                            });
                        }
                    }
                    Domain::Set(plain)
                }
                DrawValues::Range { low, high, step } => {
                    if matches!(spec.kind, Kind::Enum(_) | Kind::Toggle) {
                        return Err(DrawError::BadRange {
                            at,
                            reason: "a range on an enum or a toggle".into(),
                        });
                    }
                    let (lo, hi) = (resolve(low)?, resolve(high)?);
                    if lo >= hi || !jnd.exceeded(lo, hi) {
                        return Err(DrawError::BadRange {
                            at,
                            reason: format!("[{lo}, {hi}] is not a range at least a step wide"),
                        });
                    }
                    let step = match step {
                        None => 0.0,
                        Some(DrawValue::Number(x)) if *x > 0.0 && *x <= hi - lo => *x,
                        Some(other) => {
                            return Err(DrawError::BadRange {
                                at,
                                reason: format!(
                                    "the step {other} is not a positive number at most the range's width"
                                ),
                            });
                        }
                    };
                    Domain::Range(lo, hi, step)
                }
            };
            out.push(Resolved {
                at: item.at.clone(),
                key: item.at.key(),
                spec,
                jnd,
                domain,
            });
        }
        Ok(out)
    }

    /// Draw the target over `start`, heard through `phrase` (see the [type docs](Draw)). Pure and deterministic: the
    /// same draw, start and phrase give the same target on every machine.
    pub fn draw(&self, start: &Patch, phrase: &Phrase) -> Result<Drawing, DrawError> {
        let items = self.resolve(start)?;
        let mut last = String::new();
        for attempt in 0..MAX_ATTEMPTS {
            let values: Vec<f32> = items
                .iter()
                .map(|r| pick(r, stream(self.seed, &r.key, attempt)))
                .collect();
            let patch = place(start, &items, &values)?;
            match playable(start, &patch, &items, phrase) {
                Ok(()) => {
                    return Ok(drawing(patch, &items, &values, attempt, false));
                }
                Err(why) => last = why,
            }
        }
        let first: Vec<f32> = items
            .iter()
            .map(|r| pick(r, stream(self.seed, &r.key, 0)))
            .collect();
        force(start, &items, &first, phrase)?.ok_or(DrawError::NoTarget { reason: last })
    }
}

/// The forced step: from the first attempt's values, the first item that can move a step from the start takes its
/// first value that does (a set's in order; a range's low end, then its high end), when that target is playable.
fn force(
    start: &Patch,
    items: &[Resolved],
    first: &[f32],
    phrase: &Phrase,
) -> Result<Option<Drawing>, DrawError> {
    for (i, r) in items.iter().enumerate() {
        let from = value_in(start, &r.at).unwrap_or(r.spec.default);
        let candidates: Vec<f32> = match &r.domain {
            Domain::Set(v) => v.clone(),
            Domain::Range(lo, hi, _) => vec![*lo, *hi],
        };
        for v in candidates.into_iter().filter(|v| r.jnd.exceeded(from, *v)) {
            let mut values = first.to_vec();
            values[i] = v;
            let patch = place(start, items, &values)?;
            if playable(start, &patch, items, phrase).is_ok() {
                return Ok(Some(drawing(patch, items, &values, MAX_ATTEMPTS, true)));
            }
        }
    }
    Ok(None)
}

fn drawing(
    patch: Patch,
    items: &[Resolved],
    values: &[f32],
    attempt: u16,
    forced: bool,
) -> Drawing {
    Drawing {
        patch,
        values: items
            .iter()
            .zip(values)
            .map(|(r, v)| (r.at.clone(), *v))
            .collect(),
        attempt,
        forced,
    }
}

/// One value of an item from its stream's bits.
fn pick(r: &Resolved, bits: u64) -> f32 {
    match &r.domain {
        Domain::Set(values) => values[(bits % values.len() as u64) as usize],
        Domain::Range(lo, hi, step) => {
            let u = unit_of(bits);
            let v = match r.at {
                DrawAt::Route(_) => lo + (hi - lo) * u,
                DrawAt::Param(_) => {
                    let (a, b) = (r.spec.to_norm(*lo), r.spec.to_norm(*hi));
                    r.spec.from_norm(a + (b - a) * u)
                }
            };
            if *step > 0.0 {
                on_grid(*lo, *hi, *step, v)
            } else {
                match r.at {
                    DrawAt::Route(_) => depth_in(*lo, *hi, v),
                    DrawAt::Param(_) => random::quantize(r.spec, v).clamp(*lo, *hi),
                }
            }
        }
    }
}

/// The start with every item set to its value, through ops.
fn place(start: &Patch, items: &[Resolved], values: &[f32]) -> Result<Patch, DrawError> {
    let mut patch = start.clone();
    for (r, value) in items.iter().zip(values) {
        let op = match &r.at {
            DrawAt::Param(addr) => Some(Op::SetParam {
                addr: addr.clone(),
                value: *value,
            }),
            DrawAt::Route(route) => {
                let exists = patch
                    .routes
                    .iter()
                    .any(|x| x.src == route.src && x.dst == route.dst);
                match (exists, *value == 0.0) {
                    (true, true) => Some(Op::RemoveRoute {
                        route: route.clone(),
                    }),
                    (true, false) => Some(Op::SetDepth {
                        route: route.clone(),
                        depth: *value,
                    }),
                    (false, true) => None,
                    (false, false) => Some(Op::InsertRoute {
                        index: patch.routes.len(),
                        route: Route {
                            src: route.src.clone(),
                            dst: route.dst.clone(),
                            depth: *value,
                        },
                    }),
                }
            }
        };
        if let Some(op) = op {
            apply(&mut patch, op).map_err(|e| DrawError::Apply {
                at: r.at.to_string(),
                reason: e.to_string(),
            })?;
        }
    }
    Ok(patch)
}

/// Whether a drawn target can be played: everything drawn is heard (a route at depth 0 is no route, and that is
/// heard as its absence), and something is a step from the start.
fn playable(
    start: &Patch,
    target: &Patch,
    items: &[Resolved],
    phrase: &Phrase,
) -> Result<(), String> {
    let mask = observable(target, phrase);
    for r in items {
        let heard = match &r.at {
            DrawAt::Param(a) => mask.is_observable(a),
            DrawAt::Route(route) => {
                value_in(target, &r.at) == Some(0.0)
                    || mask
                        .route(route)
                        .is_some_and(random::Visibility::is_observable)
            }
        };
        if !heard {
            return Err(format!("{} is not heard through the phrase", r.at));
        }
    }
    let moved = items.iter().any(
        |r| match (value_in(start, &r.at), value_in(target, &r.at)) {
            (Some(a), Some(b)) => r.jnd.exceeded(a, b),
            _ => true,
        },
    );
    if moved {
        Ok(())
    } else {
        Err("every drawn value is within a step of the start".to_owned())
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Prepared rounds on patch and draw targets
// ---------------------------------------------------------------------------------------------------------------

/// The category of a module kind, as generator v1 orders them (for a hand-made round's [`Randomized`] list).
fn category_of_kind(kind: Option<ModuleKind>) -> Category {
    match kind {
        Some(ModuleKind::Osc | ModuleKind::Noise) => Category::Oscillators,
        Some(ModuleKind::Svf) => Category::Filter,
        Some(ModuleKind::Adsr) => Category::Envelopes,
        Some(ModuleKind::Lfo) => Category::Modulation,
        Some(ModuleKind::Reverb | ModuleKind::Comp) => Category::Effects,
        None => Category::Structure,
    }
}

/// The category an item belongs to: its module's (voice and master are structure); a route's is its source's
/// (envelopes, LFOs; velocity, key, the wheel and macros are structure).
pub(crate) fn category_of(patch: &Patch, drawn: &Drawn) -> Category {
    let kind = |id: &ModuleId| patch.find(id).map(|(_, m)| m.kind);
    match drawn {
        Drawn::Param(a) => match a.target() {
            enth_patch::Target::Module(id) => category_of_kind(kind(id)),
            _ => Category::Structure,
        },
        Drawn::Route(r) => match &r.src {
            ModSource::Module(id) => match kind(id) {
                Some(k @ (ModuleKind::Adsr | ModuleKind::Lfo)) => category_of_kind(Some(k)),
                _ => Category::Structure,
            },
            _ => Category::Structure,
        },
        Drawn::Enable(id) => category_of_kind(kind(id)),
    }
}

/// A [`Generated`] for a target no generator version made ([`NOT_GENERATED`], level 0, no minimum steps): the target,
/// its phrase, what the round compares, and its patch-target fingerprint
/// ([`crate::fingerprint::of_patch`]). `drawn` lists what the target varies, in order; `params` are the compared
/// parameters (`TargetContext::randomized`).
pub(crate) fn hand_made(
    target: Patch,
    phrase: PhraseId,
    drawn: Vec<Drawn>,
    params: Vec<ParamAddr>,
) -> Generated {
    let randomized = drawn
        .into_iter()
        .map(|d| Randomized {
            category: category_of(&target, &d),
            drawn: d,
            counted: true,
        })
        .collect();
    Generated {
        generator: NOT_GENERATED,
        fingerprint: crate::fingerprint::of_patch(&target, phrase),
        patch: target,
        phrase,
        level: 0,
        randomized,
        params,
        steps: Vec::new(),
    }
}

// ---------------------------------------------------------------------------------------------------------------
// The challenge's file forms: v1 (frozen) and v2
// ---------------------------------------------------------------------------------------------------------------

/// Challenge v1's target, frozen: only a seed (`{"kind":"seed","spec":{…}}`).
#[derive(Deserialize)]
#[serde(
    tag = "kind",
    content = "spec",
    rename_all = "snake_case",
    deny_unknown_fields
)]
enum TargetV1 {
    Seed(RandomSpec),
}

/// Challenge v1's start, frozen: only a template.
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum StartV1 {
    Template { template: u8 },
}

/// Challenge v1's constraints, frozen: no fields at all.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConstraintsV1 {}

/// Challenge v1's answer, frozen: only a patch.
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum AnswerV1 {
    Patch,
}

/// The v1 file, frozen: every v2 field is unknown here.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FileV1 {
    #[serde(rename = "schema")]
    _schema: String,
    target: TargetV1,
    answer: AnswerV1,
    phrase: PhraseId,
    start: StartV1,
    #[serde(rename = "constraints")]
    _constraints: ConstraintsV1,
    assists: Assists,
    scoring: ScoringSpec,
    meta: ChallengeMeta,
}

impl FileV1 {
    /// The v1 → v2 migration: the same challenge, with no change of meaning (a seed target, a template start, no
    /// constraint, a patch answer).
    fn migrate(self) -> Challenge {
        let TargetV1::Seed(spec) = self.target;
        let StartV1::Template { template } = self.start;
        let AnswerV1::Patch = self.answer;
        Challenge {
            target: Target::Seed(spec),
            answer: AnswerKind::Patch,
            phrase: self.phrase,
            start: Start::Template { template },
            constraints: Constraints::default(),
            assists: self.assists,
            scoring: self.scoring,
            meta: self.meta,
        }
    }
}

/// The v2 file.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FileV2 {
    #[serde(rename = "schema")]
    _schema: String,
    target: Target,
    answer: AnswerKind,
    phrase: PhraseId,
    start: Start,
    constraints: Constraints,
    assists: Assists,
    scoring: ScoringSpec,
    meta: ChallengeMeta,
}

/// What both versions write, borrowed: schema first, then the fields in declaration order. A v1-expressible challenge
/// writes v1's exact bytes through it (a seed target, a template start, `{}`).
#[derive(Serialize)]
struct FileOut<'a> {
    schema: &'static str,
    target: &'a Target,
    answer: &'a AnswerKind,
    phrase: PhraseId,
    start: &'a Start,
    constraints: &'a Constraints,
    assists: &'a Assists,
    scoring: &'a ScoringSpec,
    meta: &'a ChallengeMeta,
}

/// Write `c` at [`Challenge::version`].
pub(crate) fn write<S: Serializer>(c: &Challenge, s: S) -> Result<S::Ok, S::Error> {
    FileOut {
        schema: if c.version() == 1 {
            SCHEMA_V1
        } else {
            SCHEMA_V2
        },
        target: &c.target,
        answer: &c.answer,
        phrase: c.phrase,
        start: &c.start,
        constraints: &c.constraints,
        assists: &c.assists,
        scoring: &c.scoring,
        meta: &c.meta,
    }
    .serialize(s)
}

/// The version of a challenge's JSON: its `schema` must be `enth.challenge/<N>` with `N ≥ 1`.
fn version_of(value: &Value) -> Result<u32, ChallengeError> {
    let schema = value
        .as_object()
        .ok_or_else(|| ChallengeError::BadChallenge {
            message: "a challenge is a JSON object".to_owned(),
        })?
        .get("schema")
        .ok_or(ChallengeError::MissingSchema)?;
    let bad = || ChallengeError::BadSchema {
        found: schema.to_string(),
    };
    let text = schema.as_str().ok_or_else(bad)?;
    let n = text
        .strip_prefix(CHALLENGE_KIND)
        .and_then(|rest| rest.strip_prefix('/'))
        .ok_or_else(bad)?;
    if n.is_empty() || n.starts_with('0') || !n.bytes().all(|b| b.is_ascii_digit()) {
        return Err(bad());
    }
    n.parse::<u32>().map_err(|_| bad())
}

/// Read a challenge of any version this build has (the one strict path of both `Challenge::from_json` and the
/// embedded serde path): check the schema (`missing_schema`, `bad_schema`, `challenge_too_new`), decode with unknown
/// fields refused (v1 through its frozen shapes), migrate v1, and validate.
pub fn read(value: Value) -> Result<Challenge, ChallengeError> {
    let version = version_of(&value)?;
    let decode_error = |e: serde_json::Error| ChallengeError::BadChallenge {
        message: e.to_string(),
    };
    let challenge = match version {
        1 => serde_json::from_value::<FileV1>(value)
            .map_err(decode_error)?
            .migrate(),
        2 => {
            let f = serde_json::from_value::<FileV2>(value).map_err(decode_error)?;
            Challenge {
                target: f.target,
                answer: f.answer,
                phrase: f.phrase,
                start: f.start,
                constraints: f.constraints,
                assists: f.assists,
                scoring: f.scoring,
                meta: f.meta,
            }
        }
        found => {
            return Err(ChallengeError::TooNew {
                found,
                supported: CHALLENGE_VERSION,
            });
        }
    };
    challenge.validate()?;
    Ok(challenge)
}

/// The on-disk target of v2, for reading: `{"kind":"seed","spec":…}`, `{"kind":"patch","patch":…}`,
/// `{"kind":"draw","seed":…,"items":[…]}`.
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum TargetFile {
    Seed { spec: RandomSpec },
    Patch { patch: InlinePatch },
    Draw { seed: u64, items: Vec<DrawItem> },
}

impl Serialize for Target {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            Target::Seed(spec) => {
                let mut st = s.serialize_struct("Target", 2)?;
                st.serialize_field("kind", "seed")?;
                st.serialize_field("spec", spec)?;
                st.end()
            }
            Target::Patch(patch) => {
                let mut st = s.serialize_struct("Target", 2)?;
                st.serialize_field("kind", "patch")?;
                st.serialize_field("patch", patch)?;
                st.end()
            }
            Target::Draw(draw) => {
                let mut st = s.serialize_struct("Target", 3)?;
                st.serialize_field("kind", "draw")?;
                st.serialize_field("seed", &draw.seed)?;
                st.serialize_field("items", &draw.items)?;
                st.end()
            }
        }
    }
}

impl<'de> Deserialize<'de> for Target {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(match TargetFile::deserialize(d)? {
            TargetFile::Seed { spec } => Target::Seed(spec),
            TargetFile::Patch { patch } => Target::Patch(patch),
            TargetFile::Draw { seed, items } => Target::Draw(Draw { seed, items }),
        })
    }
}

impl Serialize for Challenge {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        write(self, s)
    }
}

impl<'de> Deserialize<'de> for Challenge {
    /// The embedded path (a challenge inside a lesson, a saved state): the same strict reader as
    /// `Challenge::from_json`, for every version this build has. A key repeated in any object is refused, as v1's
    /// derived reader refused a repeated field (F1l review, nit 9), rather than taking the last value.
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let StrictValue(value) = StrictValue::deserialize(d)?;
        read(value).map_err(|e| D::Error::custom(format!("{}: {e} ({})", e.code(), e.hint())))
    }
}

/// A JSON value read with every repeated key refused, at any depth (`serde_json::Value` keeps the last one). Both
/// challenge paths read through it: [`Challenge::from_json`] and the embedded [`Deserialize`].
pub(crate) struct StrictValue(pub(crate) Value);

impl<'de> Deserialize<'de> for StrictValue {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        d.deserialize_any(StrictVisitor).map(StrictValue)
    }
}

struct StrictVisitor;

impl<'de> serde::de::Visitor<'de> for StrictVisitor {
    type Value = Value;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a JSON value")
    }

    fn visit_bool<E>(self, v: bool) -> Result<Value, E> {
        Ok(Value::Bool(v))
    }

    fn visit_i64<E>(self, v: i64) -> Result<Value, E> {
        Ok(Value::from(v))
    }

    fn visit_u64<E>(self, v: u64) -> Result<Value, E> {
        Ok(Value::from(v))
    }

    fn visit_f64<E>(self, v: f64) -> Result<Value, E> {
        Ok(serde_json::Number::from_f64(v).map_or(Value::Null, Value::Number))
    }

    fn visit_str<E>(self, v: &str) -> Result<Value, E> {
        Ok(Value::String(v.to_owned()))
    }

    fn visit_string<E>(self, v: String) -> Result<Value, E> {
        Ok(Value::String(v))
    }

    fn visit_none<E>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }

    fn visit_unit<E>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }

    fn visit_some<D: Deserializer<'de>>(self, d: D) -> Result<Value, D::Error> {
        StrictValue::deserialize(d).map(|v| v.0)
    }

    fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut seq: A) -> Result<Value, A::Error> {
        let mut out = Vec::new();
        while let Some(StrictValue(v)) = seq.next_element()? {
            out.push(v);
        }
        Ok(Value::Array(out))
    }

    fn visit_map<A: serde::de::MapAccess<'de>>(self, mut map: A) -> Result<Value, A::Error> {
        let mut out = serde_json::Map::new();
        while let Some(key) = map.next_key::<String>()? {
            let StrictValue(v) = map.next_value()?;
            if out.contains_key(&key) {
                return Err(A::Error::custom(format!("the key {key:?} is repeated")));
            }
            out.insert(key, v);
        }
        Ok(Value::Object(out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::phrases::{HELD, OCTAVE_PAIR};
    use crate::score::ScorerId;
    use proptest::prelude::*;

    fn starter() -> Patch {
        random::start(0).unwrap()
    }

    fn held() -> &'static Phrase {
        crate::phrases::phrase(HELD).unwrap()
    }

    fn addr(text: &str) -> ParamAddr {
        text.parse().unwrap()
    }

    fn route(text: &str) -> RouteAddr {
        text.parse().unwrap()
    }

    fn nums(values: &[f32]) -> Vec<DrawValue> {
        values.iter().copied().map(DrawValue::Number).collect()
    }

    fn set(at: &str, values: Vec<DrawValue>) -> DrawItem {
        DrawItem {
            at: if at.contains("->") {
                DrawAt::Route(route(at))
            } else {
                DrawAt::Param(addr(at))
            },
            values: DrawValues::Set(values),
        }
    }

    fn one(seed: u64, item: DrawItem) -> Draw {
        Draw {
            seed,
            items: vec![item],
        }
    }

    fn scoring() -> ScoringSpec {
        ScoringSpec::with_scorers(
            0,
            ScorerId::SOUND_V1,
            ScorerId::STRUCTURE_V1,
            ScorerId::COMBINED_V1,
            ScorerId::TIME_V1,
        )
    }

    // -------------------------------------------------------------------------------------------------------------
    // Selectors

    #[test]
    fn selectors_parse_and_print_canonically() {
        for text in [
            "filter1.cutoff",
            "voice.glide",
            "master.level",
            "filter1.*",
            "voice.*",
            "master.*",
            "lfo1 -> filter1.cutoff",
            "velocity -> filter1.cutoff",
            "macro3 -> reverb1.wet",
            "macro3",
            "macro8",
        ] {
            let s: Selector = text.parse().unwrap_or_else(|e| panic!("{text}: {e}"));
            assert_eq!(s.to_string(), text);
            let json = serde_json::to_string(&s).unwrap();
            assert_eq!(serde_json::from_str::<Selector>(&json).unwrap(), s);
        }
        assert!(matches!("macro3".parse(), Ok(Selector::Macro(_))));
        assert!(matches!("filter1.*".parse(), Ok(Selector::Module(_))));
    }

    #[test]
    fn selectors_are_strict() {
        for bad in [
            "lfo1->filter1.cutoff",
            " filter1.cutoff",
            "filter1.cutoff ",
            "filter1.nope",
            "filter1",
            "macro9",
            "macro0",
            "*",
            "filter1.**",
            "",
            "Filter1.cutoff",
        ] {
            assert!(bad.parse::<Selector>().is_err(), "{bad:?}");
        }
        let e = "lfo1->filter1.cutoff".parse::<Selector>().unwrap_err();
        assert!(e.contains("'lfo1 -> filter1.cutoff'"), "{e}");
        assert!(serde_json::from_str::<Selector>("\"lfo1->filter1.cutoff\"").is_err());
    }

    #[test]
    fn what_a_selector_covers() {
        let s = |t: &str| t.parse::<Selector>().unwrap();
        let p = |t: &str| Touch::Param(addr(t));
        let r = |t: &str| Touch::Route(route(t));
        let m = |id: &str| Touch::Module(enth_patch::Target::Module(ModuleId::new(id).unwrap()));
        let k = |n: usize| Touch::Macro(MacroIndex::from_one_based(n).unwrap());
        assert!(s("filter1.cutoff").covers(&p("filter1.cutoff")));
        assert!(!s("filter1.cutoff").covers(&p("filter1.resonance")));
        assert!(!s("filter1.cutoff").covers(&r("lfo1 -> filter1.cutoff")));
        assert!(!s("filter1.cutoff").covers(&m("filter1")));
        let module = s("filter1.*");
        assert!(module.covers(&p("filter1.cutoff")) && module.covers(&p("filter1.mode")));
        assert!(module.covers(&m("filter1")));
        assert!(module.covers(&r("lfo1 -> filter1.cutoff")), "into it");
        assert!(!module.covers(&p("osc1.wave")) && !module.covers(&m("osc1")));
        let lfo = s("lfo1.*");
        assert!(lfo.covers(&r("lfo1 -> filter1.cutoff")), "from it");
        assert!(!lfo.covers(&r("lfo2 -> filter1.cutoff")));
        assert!(s("voice.*").covers(&p("voice.glide")));
        assert!(s("voice.*").covers(&Touch::Module(enth_patch::Target::Voice)));
        assert!(!s("voice.*").covers(&p("master.level")));
        assert!(s("lfo1 -> filter1.cutoff").covers(&r("lfo1 -> filter1.cutoff")));
        assert!(!s("lfo1 -> filter1.cutoff").covers(&p("filter1.cutoff")));
        assert!(s("macro3").covers(&k(3)) && !s("macro3").covers(&k(4)));
        assert!(!s("macro3").covers(&r("macro3 -> filter1.cutoff")));
    }

    #[test]
    fn constraints_lock_and_suggest() {
        let s = |t: &str| t.parse::<Selector>().unwrap();
        let none = Constraints::default();
        let cutoff = Touch::Param(addr("filter1.cutoff"));
        let reso = Touch::Param(addr("filter1.resonance"));
        assert!(none.is_empty() && !none.is_locked(&cutoff) && none.is_editable(&cutoff));
        let c = Constraints {
            editable: vec![s("filter1.*")],
            locked: vec![s("filter1.resonance")],
        };
        assert!(c.is_editable(&cutoff));
        assert!(c.is_locked(&reso) && !c.is_editable(&reso), "locked wins");
        assert_eq!(c.lock_of(&reso), Some(&s("filter1.resonance")));
        let wave = Touch::Param(addr("osc1.wave"));
        assert!(
            !c.is_editable(&wave) && !c.is_locked(&wave),
            "advisory only"
        );
        assert_eq!(serde_json::to_string(&none).unwrap(), "{}");
        assert_eq!(
            serde_json::to_string(&c).unwrap(),
            r#"{"editable":["filter1.*"],"locked":["filter1.resonance"]}"#
        );
        let only = serde_json::from_str::<Constraints>(r#"{"locked":["macro1"]}"#).unwrap();
        assert_eq!(only.locked, [s("macro1")]);
        assert!(serde_json::from_str::<Constraints>(r#"{"frozen":[]}"#).is_err());
        let start = starter();
        assert_eq!(c.check_against(&start), Ok(()));
        let twice = Constraints {
            locked: vec![s("macro1"), s("macro1")],
            ..Constraints::default()
        };
        assert_eq!(
            twice.check_against(&start).unwrap_err().code(),
            "bad_selector"
        );
        for bad in [
            "osc3.wave",
            "osc3.*",
            "osc3 -> filter1.cutoff",
            "lfo1 -> voice.mode",
        ] {
            let c = Constraints {
                locked: vec![s(bad)],
                ..Constraints::default()
            };
            assert_eq!(
                c.check_against(&start).unwrap_err().code(),
                "bad_selector",
                "{bad}"
            );
        }
        // A route the start lacks may be named: the step may be about adding it.
        let add = Constraints {
            editable: vec![s("env2 -> filter1.cutoff")],
            ..Constraints::default()
        };
        assert_eq!(add.check_against(&start), Ok(()));
    }

    fn any_selector() -> impl Strategy<Value = Selector> {
        let start = starter();
        let params: Vec<ParamAddr> = observable(&start, held())
            .params()
            .map(|(a, _)| a.clone())
            .collect();
        let modules: Vec<String> = start.modules().map(|(_, m)| m.id.to_string()).collect();
        let dsts: Vec<ParamAddr> = params
            .iter()
            .filter(|a| start.spec(a).is_some_and(|s| s.flags.modulatable))
            .cloned()
            .collect();
        let srcs = vec![
            "env2", "lfo1", "lfo2", "velocity", "key", "modwheel", "macro2",
        ];
        prop_oneof![
            prop::sample::select(params).prop_map(Selector::Param),
            prop::sample::select(modules).prop_map(|id| Selector::Module(
                enth_patch::Target::Module(ModuleId::new(&id).unwrap())
            )),
            Just(Selector::Module(enth_patch::Target::Voice)),
            (prop::sample::select(srcs), prop::sample::select(dsts)).prop_map(|(s, d)| {
                Selector::Route(RouteAddr {
                    src: s.parse().unwrap(),
                    dst: d,
                })
            }),
            (1usize..=8).prop_map(|n| Selector::Macro(MacroIndex::from_one_based(n).unwrap())),
        ]
    }

    /// Valid draw items over the starter, one per key: what a draw strategy picks from.
    fn item_pool() -> Vec<DrawItem> {
        let text = |t: &str| DrawValue::Text(t.to_owned());
        vec![
            set(
                "filter1.cutoff",
                nums(&[300.0, 600.0, 1200.0, 4000.0, 8000.0]),
            ),
            set("filter1.mode", vec![text("lp"), text("hp"), text("bp")]),
            DrawItem {
                at: DrawAt::Param(addr("filter1.resonance")),
                values: DrawValues::Range {
                    low: DrawValue::Number(0.0),
                    high: DrawValue::Number(80.0),
                    step: None,
                },
            },
            DrawItem {
                at: DrawAt::Param(addr("osc1.semi")),
                values: DrawValues::Range {
                    low: DrawValue::Number(-24.0),
                    high: DrawValue::Number(24.0),
                    step: Some(DrawValue::Number(1.0)),
                },
            },
            set("env1.attack", nums(&[2.0, 25.0, 200.0, 900.0])),
            set(
                "env2 -> filter1.cutoff",
                vec![
                    DrawValue::Number(-40.0),
                    text("+1oct"),
                    DrawValue::Number(60.0),
                ],
            ),
            set(
                "osc1.wave",
                vec![text("sine"), text("triangle"), text("pulse")],
            ),
        ]
    }

    fn any_draw_challenge() -> impl Strategy<Value = Challenge> {
        let pool = item_pool();
        (
            0..=MAX_SEED,
            1u32..(1 << pool.len()),
            prop::collection::btree_set(any_selector(), 0..4),
            prop::collection::btree_set(any_selector(), 0..3),
            any::<bool>(),
        )
            .prop_map(move |(seed, mask, editable, locked, patch_start)| {
                let items = pool
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| mask & (1 << i) != 0)
                    .map(|(_, item)| item.clone())
                    .collect();
                let start = if patch_start {
                    Start::Patch {
                        patch: InlinePatch::new(starter()),
                    }
                } else {
                    Start::Template { template: 0 }
                };
                let mut c =
                    Challenge::new(Target::Draw(Draw { seed, items }), start, HELD, scoring());
                c.constraints = Constraints {
                    editable: editable.into_iter().collect(),
                    locked: locked.into_iter().collect(),
                };
                c
            })
            .prop_filter("a valid challenge", |c| c.validate().is_ok())
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 64, ..ProptestConfig::default() })]

        /// Any valid draw challenge round-trips through both paths to the same value and the same bytes, and draws
        /// the same target twice.
        #[test]
        fn any_v2_challenge_round_trips_to_the_same_bytes(c in any_draw_challenge()) {
            let json = c.to_json();
            let v2 = json.starts_with(r#"{"schema":"enth.challenge/2""#);
            prop_assert!(v2, "written as v2");
            let back = Challenge::from_json(&json).unwrap();
            prop_assert_eq!(&back, &c);
            prop_assert_eq!(back.to_json(), json.clone());
            prop_assert_eq!(serde_json::from_str::<Challenge>(&json).unwrap(), c.clone());
            prop_assert_eq!(Challenge::from_json(&c.to_json_pretty()).unwrap(), c.clone());
            let start = c.start_patch().unwrap();
            let a = c.draw().unwrap().draw(&start, held());
            prop_assert_eq!(a, c.draw().unwrap().draw(&start, held()));
        }
    }

    proptest! {
        /// Print → parse gives the selector back, and the text is what JSON carries.
        #[test]
        fn any_selector_round_trips(s in any_selector()) {
            let text = s.to_string();
            prop_assert_eq!(text.parse::<Selector>(), Ok(s.clone()));
            prop_assert_eq!(serde_json::to_string(&s).unwrap(), format!("\"{text}\""));
        }
    }

    // -------------------------------------------------------------------------------------------------------------
    // Inline patches

    #[test]
    fn an_inline_patch_round_trips_to_the_same_bytes() {
        let mut p = starter();
        apply(
            &mut p,
            Op::SetParam {
                addr: addr("filter1.cutoff"),
                value: 0.1 + 1234.5,
            },
        )
        .unwrap();
        let inline = InlinePatch::new(p.clone());
        let json = serde_json::to_string(&inline).unwrap();
        assert_eq!(json, p.to_json(), "the patch file's own bytes");
        let back: InlinePatch = serde_json::from_str(&json).unwrap();
        assert_eq!(back, inline);
        assert_eq!(serde_json::to_string(&back).unwrap(), json);
        let bad = json.replacen("\"schema\":1", "\"schema\":1,\"x\":1", 1);
        assert!(serde_json::from_str::<InlinePatch>(&bad).is_err());
        let too_new = json.replacen("\"schema\":1", "\"schema\":99", 1);
        assert!(serde_json::from_str::<InlinePatch>(&too_new).is_err());
    }

    // -------------------------------------------------------------------------------------------------------------
    // Draws

    #[test]
    fn a_set_draw_is_deterministic_never_the_start_and_covers_its_values() {
        let start = starter();
        let values = [300.0, 600.0, 1200.0, 2000.0, 4000.0, 8000.0];
        let mut seen = std::collections::BTreeSet::new();
        for seed in 0..120 {
            let draw = one(seed, set("filter1.cutoff", nums(&values)));
            let a = draw.draw(&start, held()).unwrap();
            assert_eq!(a, draw.draw(&start, held()).unwrap(), "deterministic");
            let v = a.patch.value(&addr("filter1.cutoff")).unwrap();
            assert!(values.contains(&v));
            assert_ne!(v, 2000.0, "the start's value never makes a target");
            assert!(!a.forced);
            seen.insert(v.to_bits());
            assert_eq!(a.values, [(DrawAt::Param(addr("filter1.cutoff")), v)]);
            assert_eq!(a.params(), [addr("filter1.cutoff")]);
        }
        assert_eq!(seen.len(), 5, "every other value is drawn");
    }

    #[test]
    fn enum_keys_unit_text_and_toggles_resolve_as_a_patch_file_does() {
        let start = starter();
        let modes = vec![
            DrawValue::Text("hp".into()),
            DrawValue::Text("bp".into()),
            DrawValue::Text("notch".into()),
        ];
        let d = one(3, set("filter1.mode", modes))
            .draw(&start, held())
            .unwrap();
        let mode = d.patch.value(&addr("filter1.mode")).unwrap();
        assert!(mode >= 1.0, "an item index, never lp");
        let text = one(
            4,
            set(
                "filter1.cutoff",
                vec![DrawValue::Text("1.2kHz".into()), DrawValue::Number(300.0)],
            ),
        );
        assert_eq!(text.validate(&start), Ok(()));
        let sync = one(5, set("osc1.sync", vec![DrawValue::Bool(true)]));
        assert_eq!(sync.validate(&start), Ok(()));
        let wrong = one(6, set("filter1.mode", vec![DrawValue::Number(1.0)]));
        assert_eq!(wrong.validate(&start).unwrap_err().code(), "bad_draw");
    }

    #[test]
    fn a_range_draw_stays_in_range_with_reachable_values() {
        let start = starter();
        let item = DrawItem {
            at: DrawAt::Param(addr("filter1.cutoff")),
            values: DrawValues::Range {
                low: DrawValue::Number(200.0),
                high: DrawValue::Number(12_000.0),
                step: None,
            },
        };
        let mut distinct = std::collections::BTreeSet::new();
        for seed in 0..60 {
            let d = one(seed, item.clone()).draw(&start, held()).unwrap();
            let v = d.patch.value(&addr("filter1.cutoff")).unwrap();
            assert!((200.0..=12_000.0).contains(&v), "{v}");
            assert_eq!(v, random::sig3(v), "3 significant digits");
            assert!(priors::exceeds_jnd(
                start.spec(&addr("filter1.cutoff")).unwrap(),
                2000.0,
                v
            ));
            distinct.insert(v.to_bits());
        }
        assert!(distinct.len() > 40, "{}", distinct.len());
        let semis = DrawItem {
            at: DrawAt::Param(addr("osc1.semi")),
            values: DrawValues::Range {
                low: DrawValue::Number(-24.0),
                high: DrawValue::Number(24.0),
                step: Some(DrawValue::Number(1.0)),
            },
        };
        for seed in 0..30 {
            let v = one(seed, semis.clone())
                .draw(&start, held())
                .unwrap()
                .patch
                .value(&addr("osc1.semi"))
                .unwrap();
            assert_eq!(v.fract(), 0.0, "whole semitones");
        }
    }

    /// Q4: route depths and their sign are drawn; depth 0 is no route.
    #[test]
    fn route_depths_and_signs_are_drawn_and_zero_is_no_route() {
        let mut start = starter();
        let r = route("env2 -> filter1.cutoff");
        let mut signs = (false, false);
        for seed in 0..40 {
            let d = one(
                seed,
                set("env2 -> filter1.cutoff", nums(&[-60.0, -30.0, 30.0, 60.0])),
            )
            .draw(&start, held())
            .unwrap();
            let depth = value_in(&d.patch, &DrawAt::Route(r.clone())).unwrap();
            assert!([-60.0, -30.0, 30.0, 60.0].contains(&depth));
            signs.0 |= depth < 0.0;
            signs.1 |= depth > 0.0;
            assert_eq!(d.routes(), std::slice::from_ref(&r));
            assert_eq!(d.patch.routes.len(), 1, "inserted once");
        }
        assert_eq!(signs, (true, true));
        // From a start that has the route at 30 %: 0 removes it, 30 is never a target.
        apply(
            &mut start,
            Op::InsertRoute {
                index: 0,
                route: Route {
                    src: r.src.clone(),
                    dst: r.dst.clone(),
                    depth: 30.0,
                },
            },
        )
        .unwrap();
        let mut removed = false;
        for seed in 0..40 {
            let d = one(
                seed,
                set("env2 -> filter1.cutoff", nums(&[0.0, 30.0, 60.0])),
            )
            .draw(&start, held())
            .unwrap();
            let depth = value_in(&d.patch, &DrawAt::Route(r.clone())).unwrap();
            assert_ne!(depth, 30.0);
            if depth == 0.0 {
                removed = true;
                assert!(d.patch.routes.is_empty(), "depth 0 is no route");
            }
        }
        assert!(removed);
        // An interval as patch-op text: +12 st on osc1.semi's ±48 st range is 12.5 %.
        let st = one(
            1,
            set("env2 -> osc1.semi", vec![DrawValue::Text("+12st".into())]),
        );
        let d = st.draw(&starter(), held()).unwrap();
        assert_eq!(d.values[0].1, 12.5);
    }

    #[test]
    fn seeds_give_siblings() {
        let start = starter();
        let draw = |seed| {
            Draw {
                seed,
                items: vec![
                    set(
                        "filter1.cutoff",
                        nums(&[300.0, 600.0, 1200.0, 4000.0, 8000.0]),
                    ),
                    set("filter1.resonance", nums(&[30.0, 50.0, 90.0])),
                ],
            }
            .draw(&start, held())
            .unwrap()
            .values
        };
        let distinct: std::collections::BTreeSet<Vec<u32>> = (0..20)
            .map(|seed| draw(seed).iter().map(|(_, v)| v.to_bits()).collect())
            .collect();
        assert!(distinct.len() > 8, "{}", distinct.len());
        // Keyed streams: adding an item never moves another item's draw.
        let base = draw(7);
        let more = Draw {
            seed: 7,
            items: vec![
                set(
                    "filter1.cutoff",
                    nums(&[300.0, 600.0, 1200.0, 4000.0, 8000.0]),
                ),
                set("filter1.resonance", nums(&[30.0, 50.0, 90.0])),
                set("env1.attack", nums(&[2.0, 25.0, 200.0])),
            ],
        }
        .draw(&start, held())
        .unwrap();
        if more.attempt == 0 {
            assert_eq!(&more.values[..2], &base[..]);
        }
    }

    #[test]
    fn a_draw_of_something_unheard_fails() {
        // The pulse width is dead on a saw: no attempt and no forced step can make it heard.
        let e = one(1, set("osc1.pw", nums(&[10.0, 30.0])))
            .draw(&starter(), held())
            .unwrap_err();
        assert_eq!(e.code(), "draw_failed");
        assert!(e.to_string().contains("osc1.pw"), "{e}");
        // Keytrack is heard only through two notes an octave apart.
        let key = one(1, set("filter1.keytrack", nums(&[50.0, 100.0])));
        assert_eq!(
            key.draw(&starter(), held()).unwrap_err().code(),
            "draw_failed"
        );
        let octave = crate::phrases::phrase(OCTAVE_PAIR).unwrap();
        assert!(key.draw(&starter(), octave).is_ok());
    }

    #[test]
    fn the_forced_step_takes_the_first_value_a_step_from_the_start() {
        let start = starter();
        let draw = Draw {
            seed: 1,
            items: vec![
                set("filter1.resonance", nums(&[10.0])),
                set("filter1.cutoff", nums(&[2000.0, 3000.0, 8000.0])),
            ],
        };
        let items = draw.resolve(&start).unwrap();
        let forced = force(&start, &items, &[10.0, 2000.0], held())
            .unwrap()
            .unwrap();
        assert!(forced.forced);
        assert_eq!(forced.attempt, MAX_ATTEMPTS);
        assert_eq!(forced.values[1].1, 3000.0);
        let stuck = Draw {
            seed: 1,
            items: vec![set("filter1.resonance", nums(&[10.0]))],
        };
        let items = stuck.resolve(&start).unwrap();
        assert_eq!(force(&start, &items, &[10.0], held()), Ok(None));
        assert_eq!(
            stuck.draw(&start, held()).unwrap_err().code(),
            "draw_failed"
        );
    }

    #[test]
    fn bad_draws_are_refused_with_their_codes() {
        let start = starter();
        let cases: Vec<(Draw, &str)> = vec![
            (
                Draw {
                    seed: 0,
                    items: vec![],
                },
                "bad_draw",
            ),
            (
                one(MAX_SEED + 1, set("filter1.cutoff", nums(&[300.0]))),
                "out_of_range",
            ),
            (
                one(0, set("osc3.wave", vec![DrawValue::Text("saw".into())])),
                "bad_draw",
            ),
            (one(0, set("lfo1 -> voice.mode", nums(&[10.0]))), "bad_draw"),
            (
                one(0, set("osc3 -> filter1.cutoff", nums(&[10.0]))),
                "bad_draw",
            ),
            (one(0, set("filter1.cutoff", vec![])), "bad_draw"),
            (one(0, set("filter1.cutoff", nums(&[30_000.0]))), "bad_draw"),
            (
                one(
                    0,
                    set("filter1.mode", vec![DrawValue::Text("zigzag".into())]),
                ),
                "bad_draw",
            ),
            (
                one(0, set("filter1.cutoff", nums(&[1000.0, 1050.0]))),
                "draw_too_close",
            ),
            (
                one(0, set("env2 -> filter1.cutoff", nums(&[20.0, 22.0]))),
                "draw_too_close",
            ),
            (
                one(0, set("env2 -> filter1.cutoff", nums(&[120.0]))),
                "bad_draw",
            ),
            (
                one(
                    0,
                    set("env2 -> filter1.cutoff", vec![DrawValue::Bool(true)]),
                ),
                "bad_draw",
            ),
            (
                one(
                    0,
                    DrawItem {
                        at: DrawAt::Param(addr("filter1.mode")),
                        values: DrawValues::Range {
                            low: DrawValue::Number(0.0),
                            high: DrawValue::Number(3.0),
                            step: None,
                        },
                    },
                ),
                "bad_draw",
            ),
            (
                one(
                    0,
                    DrawItem {
                        at: DrawAt::Param(addr("filter1.cutoff")),
                        values: DrawValues::Range {
                            low: DrawValue::Number(900.0),
                            high: DrawValue::Number(300.0),
                            step: None,
                        },
                    },
                ),
                "bad_draw",
            ),
            (
                Draw {
                    seed: 0,
                    items: vec![
                        set("filter1.cutoff", nums(&[300.0])),
                        set("filter1.cutoff", nums(&[600.0])),
                    ],
                },
                "bad_draw",
            ),
            (
                one(
                    0,
                    DrawItem {
                        at: DrawAt::Param(addr("osc1.semi")),
                        values: DrawValues::Range {
                            low: DrawValue::Number(-12.0),
                            high: DrawValue::Number(12.0),
                            step: Some(DrawValue::Number(0.0)),
                        },
                    },
                ),
                "bad_draw",
            ),
            (
                one(
                    0,
                    DrawItem {
                        at: DrawAt::Param(addr("osc1.semi")),
                        values: DrawValues::Range {
                            low: DrawValue::Number(-12.0),
                            high: DrawValue::Number(12.0),
                            step: Some(DrawValue::Number(48.0)),
                        },
                    },
                ),
                "bad_draw",
            ),
            (
                Draw {
                    seed: 0,
                    items: (0..=MAX_DRAW_ITEMS)
                        .map(|_| set("filter1.cutoff", nums(&[300.0])))
                        .collect(),
                },
                "bad_draw",
            ),
        ];
        for (draw, code) in cases {
            let e = draw.validate(&start).unwrap_err();
            assert_eq!(e.code(), code, "{e}");
            assert_ne!(e.hint(), "");
        }
    }

    #[test]
    fn draw_items_are_strict_json() {
        let item = set("env2 -> filter1.cutoff", nums(&[-30.0, 30.0]));
        let json = serde_json::to_string(&item).unwrap();
        assert_eq!(
            json,
            r#"{"route":"env2 -> filter1.cutoff","set":[-30.0,30.0]}"#
        );
        assert_eq!(serde_json::from_str::<DrawItem>(&json).unwrap(), item);
        let grid = DrawItem {
            at: DrawAt::Param(addr("osc1.semi")),
            values: DrawValues::Range {
                low: DrawValue::Number(-24.0),
                high: DrawValue::Number(24.0),
                step: Some(DrawValue::Number(1.0)),
            },
        };
        let json = serde_json::to_string(&grid).unwrap();
        assert_eq!(
            json,
            r#"{"param":"osc1.semi","range":[-24.0,24.0],"step":1.0}"#
        );
        assert_eq!(serde_json::from_str::<DrawItem>(&json).unwrap(), grid);
        for bad in [
            r#"{"param":"filter1.cutoff","set":[1.0],"step":1.0}"#,
            r#"{"param":"filter1.cutoff"}"#,
            r#"{"set":[1.0]}"#,
            r#"{"param":"filter1.cutoff","route":"env2 -> filter1.cutoff","set":[1.0]}"#,
            r#"{"param":"filter1.cutoff","set":[1.0],"range":[1.0,2.0]}"#,
            r#"{"param":"filter1.cutoff","set":[1.0],"x":1}"#,
            r#"{"param":"filter1.cutoff","set":[null]}"#,
            r#"{"param":"filter1.cutoff","range":[1.0]}"#,
            r#"{"param":"env2 -> filter1.cutoff","set":[1.0]}"#,
            r#"{"route":"filter1.cutoff","set":[1.0]}"#,
            r#"{"route":"env2->filter1.cutoff","set":[1.0]}"#,
            r#"{"param":"filter1.cutoff","set":[1e39]}"#,
        ] {
            assert!(serde_json::from_str::<DrawItem>(bad).is_err(), "{bad}");
        }
    }

    // -------------------------------------------------------------------------------------------------------------
    // Challenges

    fn lesson_draw() -> Challenge {
        let mut start = starter();
        apply(
            &mut start,
            Op::SetParam {
                addr: addr("filter1.resonance"),
                value: 20.0,
            },
        )
        .unwrap();
        let mut c = Challenge::new(
            Target::Draw(Draw {
                seed: 11,
                items: vec![set(
                    "filter1.cutoff",
                    nums(&[300.0, 600.0, 1200.0, 4000.0, 8000.0]),
                )],
            }),
            Start::Patch {
                patch: InlinePatch::new(start),
            },
            HELD,
            scoring(),
        );
        c.constraints = Constraints {
            editable: vec!["filter1.cutoff".parse().unwrap()],
            locked: vec!["filter1.resonance".parse().unwrap()],
        };
        c
    }

    #[test]
    fn a_v2_challenge_round_trips_and_says_its_version() {
        let c = lesson_draw();
        assert_eq!(c.version(), 2);
        let json = c.to_json();
        assert!(json.starts_with(r#"{"schema":"enth.challenge/2","target":{"kind":"draw","seed":11,"items":[{"param":"filter1.cutoff","set":[300.0,"#), "{json}");
        assert_eq!(Challenge::from_json(&json), Ok(c.clone()));
        assert_eq!(serde_json::from_str::<Challenge>(&json).unwrap(), c);
        assert_eq!(Challenge::from_json(&c.to_json_pretty()), Ok(c.clone()));
        assert_eq!(
            Challenge::from_json(&json).unwrap().to_json(),
            json,
            "bytes survive"
        );
        assert_eq!(
            c.code(crate::code::TargetPrefix(1)),
            None,
            "no code for a draw"
        );
        assert_eq!(c.spec(), None);
        let sibling = c.with_seed(12).unwrap();
        assert_eq!(sibling.draw().unwrap().seed, 12);
        // A seed challenge with constraints is v2 too (an editable list: a seed target takes no lock).
        let mut seed =
            Challenge::recreate(crate::spec::SpecPreset::by_id(1).unwrap().spec(1, 1), HELD);
        assert_eq!(seed.version(), 1);
        seed.constraints.editable = vec!["osc1.*".parse().unwrap()];
        assert_eq!(seed.version(), 2);
        assert!(
            seed.to_json()
                .starts_with(r#"{"schema":"enth.challenge/2""#)
        );
        assert_eq!(Challenge::from_json(&seed.to_json()), Ok(seed));
    }

    #[test]
    fn a_v1_file_refuses_v2_pieces_and_a_v2_file_refuses_unknown_fields() {
        let v1 = Challenge::recreate(crate::spec::SpecPreset::by_id(1).unwrap().spec(1, 1), HELD);
        let json = v1.to_json();
        assert!(json.starts_with(r#"{"schema":"enth.challenge/1""#));
        let locked = json.replacen(
            r#""constraints":{}"#,
            r#""constraints":{"editable":["osc1.*"]}"#,
            1,
        );
        assert_ne!(locked, json);
        assert_eq!(
            Challenge::from_json(&locked).unwrap_err().code(),
            "bad_challenge",
            "a v2 field in a v1 file"
        );
        let as_v2 = locked.replacen("enth.challenge/1", "enth.challenge/2", 1);
        assert!(
            Challenge::from_json(&as_v2).is_ok(),
            "the same field in a v2 file"
        );
        let lesson = lesson_draw().to_json();
        for (from, to) in [
            (r#""seed":11,"#, r#""seed":11,"x":1,"#),
            (r#""editable":"#, r#""x":[],"editable":"#),
            (
                r#""kind":"patch","patch":{"#,
                r#""kind":"patch","x":1,"patch":{"#,
            ),
            (r#""seed":11"#, r#""seed":11.0"#),
            (r#""kind":"draw""#, r#""kind":"audio""#),
        ] {
            let bad = lesson.replacen(from, to, 1);
            assert_ne!(bad, lesson, "{from}");
            assert_eq!(
                Challenge::from_json(&bad).unwrap_err().code(),
                "bad_challenge",
                "{to}"
            );
        }
    }

    /// A repeated key is refused at any depth, by both paths (F1l review, nit 9): v1's derived reader refused a
    /// repeated field, and a `Value` would keep the last one silently.
    #[test]
    fn a_repeated_key_is_refused_by_both_paths() {
        let lesson = lesson_draw().to_json();
        let v1 = Challenge::recreate(crate::spec::SpecPreset::by_id(1).unwrap().spec(1, 1), HELD)
            .to_json();
        for (json, from, to) in [
            (&v1, r#""phrase":1,"#, r#""phrase":1,"phrase":2,"#),
            (&v1, r#""seed":1,"#, r#""seed":1,"seed":2,"#),
            (&lesson, r#""seed":11,"#, r#""seed":11,"seed":12,"#),
            (&lesson, r#""editable":"#, r#""locked":[],"editable":"#),
        ] {
            let bad = json.replacen(from, to, 1);
            assert_ne!(&bad, json, "{from}");
            let e = Challenge::from_json(&bad).unwrap_err();
            assert_eq!(e.code(), "bad_json", "{to}");
            assert!(e.to_string().contains("repeated"), "{e}");
            let embedded = serde_json::from_str::<Challenge>(&bad).unwrap_err();
            assert!(embedded.to_string().contains("repeated"), "{embedded}");
        }
        assert_eq!(
            serde_json::from_str::<Challenge>(&lesson).unwrap(),
            lesson_draw()
        );
    }

    /// A lock on anything the target's edits touch is refused, through the session's own table ([`touches`]): the
    /// session would refuse the edit the round asks for (F1l review M1).
    #[test]
    fn a_lock_on_what_the_target_varies_is_refused() {
        let lock = |c: &mut Challenge, s: &str| c.constraints.locked = vec![s.parse().unwrap()];
        // A draw's items.
        let mut draw = lesson_draw();
        lock(&mut draw, "filter1.cutoff");
        assert_eq!(draw.validate().unwrap_err().code(), "bad_selector");
        lock(&mut draw, "filter1.*");
        assert_eq!(draw.validate().unwrap_err().code(), "bad_selector");
        lock(&mut draw, "osc1.*");
        assert_eq!(draw.validate(), Ok(()));
        // A patch target's changes: a parameter, a route (through its module), but not what it leaves alone.
        let start = starter();
        let mut target = start.clone();
        apply(
            &mut target,
            Op::SetParam {
                addr: addr("osc1.semi"),
                value: 7.0,
            },
        )
        .unwrap();
        apply(
            &mut target,
            Op::InsertRoute {
                index: 0,
                route: Route {
                    src: "lfo1".parse().unwrap(),
                    dst: addr("filter1.cutoff"),
                    depth: 20.0,
                },
            },
        )
        .unwrap();
        let mut patch = Challenge::new(
            Target::Patch(InlinePatch::new(target)),
            Start::Patch {
                patch: InlinePatch::new(start),
            },
            HELD,
            scoring(),
        );
        for bad in [
            "osc1.semi",
            "osc1.*",
            "lfo1 -> filter1.cutoff",
            "lfo1.*",
            "filter1.*",
            // The route's destination: inserting the route moves it (the session's table).
            "filter1.cutoff",
        ] {
            lock(&mut patch, bad);
            assert_eq!(
                patch.validate().unwrap_err().code(),
                "bad_selector",
                "{bad}"
            );
        }
        for fine in ["osc1.wave", "filter1.resonance", "macro1", "voice.*"] {
            lock(&mut patch, fine);
            assert_eq!(patch.validate(), Ok(()), "{fine}");
        }
    }

    /// The F1l judge's probes: what the session refuses, the challenge refuses too (F1l review M1).
    #[test]
    fn the_challenge_refuses_a_lock_the_session_would_refuse_the_round_for() {
        let lock = |c: &mut Challenge, s: &str| c.constraints.locked = vec![s.parse().unwrap()];
        // A draw on the route env2 -> filter1.cutoff with filter1.cutoff locked: the session refuses the depth edit
        // (it touches its destination), so the challenge refuses the lock, naming the edit.
        let mut draw = Challenge::new(
            Target::Draw(one(
                3,
                DrawItem {
                    at: DrawAt::Route(route("env2 -> filter1.cutoff")),
                    values: DrawValues::Set(nums(&[-40.0, 40.0])),
                },
            )),
            Start::Patch {
                patch: InlinePatch::new(starter()),
            },
            HELD,
            scoring(),
        );
        assert_eq!(draw.validate(), Ok(()));
        lock(&mut draw, "filter1.cutoff");
        let e = draw.validate().unwrap_err();
        assert_eq!(e.code(), "bad_selector");
        assert!(
            e.to_string().contains("env2 -> filter1.cutoff"),
            "names the edit: {e}"
        );
        // The session's table says why: the depth edit touches the destination.
        let edit = Op::SetDepth {
            route: route("env2 -> filter1.cutoff"),
            depth: 40.0,
        };
        assert!(
            touches(&starter(), &edit)
                .unwrap()
                .contains(&Touch::Param(addr("filter1.cutoff")))
        );
        // A patch target that enables a module whose parameter is locked: enabling touches every parameter it has.
        let mut start = starter();
        apply(
            &mut start,
            Op::SetEnabled {
                id: ModuleId::new("lfo1").unwrap(),
                enabled: false,
            },
        )
        .unwrap();
        let mut patch = Challenge::new(
            Target::Patch(InlinePatch::new(starter())),
            Start::Patch {
                patch: InlinePatch::new(start.clone()),
            },
            HELD,
            scoring(),
        );
        assert_eq!(patch.validate(), Ok(()));
        lock(&mut patch, "lfo1.rate");
        let e = patch.validate().unwrap_err();
        assert_eq!(e.code(), "bad_selector");
        assert!(e.to_string().contains("lfo1.rate"), "{e}");
        // A target that removes a module: removing it touches its parameters too.
        let mut fewer = starter();
        apply(
            &mut fewer,
            Op::RemoveModule {
                id: ModuleId::new("lfo1").unwrap(),
            },
        )
        .unwrap();
        let mut removed = Challenge::new(
            Target::Patch(InlinePatch::new(fewer)),
            Start::Patch {
                patch: InlinePatch::new(starter()),
            },
            HELD,
            scoring(),
        );
        assert_eq!(removed.validate(), Ok(()));
        lock(&mut removed, "lfo1.rate");
        assert_eq!(removed.validate().unwrap_err().code(), "bad_selector");
        // A target that turns a routed macro: turning it touches the route's destination.
        let mut routed = starter();
        apply(
            &mut routed,
            Op::InsertRoute {
                index: 0,
                route: Route {
                    src: "macro2".parse().unwrap(),
                    dst: addr("filter1.resonance"),
                    depth: 30.0,
                },
            },
        )
        .unwrap();
        let mut turned = routed.clone();
        apply(
            &mut turned,
            Op::SetMacro {
                index: MacroIndex::new(1).unwrap(),
                value: 0.75,
            },
        )
        .unwrap();
        let mut macro_round = Challenge::new(
            Target::Patch(InlinePatch::new(turned)),
            Start::Patch {
                patch: InlinePatch::new(routed),
            },
            HELD,
            scoring(),
        );
        assert_eq!(macro_round.validate(), Ok(()));
        for bad in ["macro2", "filter1.resonance"] {
            lock(&mut macro_round, bad);
            assert_eq!(
                macro_round.validate().unwrap_err().code(),
                "bad_selector",
                "{bad}"
            );
        }
    }

    /// A seed target takes no lock, whatever its draws (they change with the seed); editable stays allowed (F1l
    /// review M2).
    #[test]
    fn a_seed_target_takes_no_lock() {
        let mut seed =
            Challenge::recreate(crate::spec::SpecPreset::by_id(1).unwrap().spec(1, 42), HELD);
        for text in ["osc1.wave", "osc1.*", "filter1.*", "macro1"] {
            seed.constraints.locked = vec![text.parse().unwrap()];
            let e = seed.validate().unwrap_err();
            assert_eq!(e.code(), "bad_selector", "{text}");
            assert!(e.hint().contains("draw target"), "{}", e.hint());
            assert!(e.to_string().contains(text), "{e}");
        }
        seed.constraints.locked.clear();
        seed.constraints.editable = vec!["osc1.*".parse().unwrap()];
        assert_eq!(seed.validate(), Ok(()));
        assert_eq!(Challenge::from_json(&seed.to_json()), Ok(seed));
    }

    /// A seed target's sibling takes the phrase the generator picks for its seed, so every sibling prepares (F1l
    /// review M2: keeping the old phrase made about a third of them `phrase_mismatch`).
    #[test]
    fn a_seed_sibling_regenerates_its_target_and_phrase() {
        let mut changed = 0;
        for level in 1..=5 {
            let spec = crate::spec::SpecPreset::by_id(level).unwrap().spec(1, 0);
            let base = Challenge::recreate(spec, random::target(&spec).unwrap().phrase);
            for seed in 0..114 {
                let sibling = base.with_seed(seed).unwrap();
                let spec = *sibling.spec().unwrap();
                assert_eq!(spec.seed, seed);
                let generated = random::target(&spec).unwrap();
                assert_eq!(
                    sibling.phrase, generated.phrase,
                    "level {level} seed {seed}"
                );
                assert_eq!(sibling.validate(), Ok(()));
                assert_eq!(sibling.version(), 1, "a generated sibling stays v1");
                changed += usize::from(sibling.phrase != base.phrase);
            }
        }
        assert!(
            changed > 20,
            "{changed} of 570 siblings play another phrase than seed 0's"
        );
        let base = Challenge::recreate(crate::spec::SpecPreset::by_id(3).unwrap().spec(1, 0), HELD);
        assert_eq!(
            base.with_seed(MAX_SEED + 1).unwrap_err().code(),
            "out_of_range"
        );
        assert_eq!(
            lesson_draw().with_seed(MAX_SEED + 1).unwrap_err().code(),
            "out_of_range"
        );
    }

    #[test]
    fn v2_validation_names_what_is_wrong() {
        let mut c = lesson_draw();
        c.phrase = PhraseId(99);
        assert_eq!(c.validate().unwrap_err().code(), "phrase_too_new");
        assert_eq!(
            Challenge::from_json(&c.to_json()).unwrap_err().code(),
            "phrase_too_new"
        );
        let mut c = lesson_draw();
        c.phrase = crate::lesson_phrases::CHORD;
        assert_eq!(
            c.validate(),
            Ok(()),
            "a lesson phrase plays a lesson challenge"
        );
        let mut c = lesson_draw();
        c.constraints.locked.push("osc9.*".parse().unwrap());
        assert_eq!(c.validate().unwrap_err().code(), "bad_selector");
        let mut c = lesson_draw();
        c.target = Target::Draw(one(0, set("filter1.cutoff", nums(&[1000.0, 1050.0]))));
        assert_eq!(c.validate().unwrap_err().code(), "draw_too_close");
        let mut c = lesson_draw();
        c.start = Start::Template { template: 9 };
        assert_eq!(c.validate().unwrap_err().code(), "code_too_new");
        let mut seed =
            Challenge::recreate(crate::spec::SpecPreset::by_id(1).unwrap().spec(1, 1), HELD);
        seed.start = Start::Patch {
            patch: InlinePatch::new(starter()),
        };
        assert_eq!(seed.validate().unwrap_err().code(), "start_mismatch");
        assert_eq!(
            seed.with_seed(5).unwrap_err().code(),
            "start_mismatch",
            "a sibling is validated"
        );
        let patch = Challenge::new(
            Target::Patch(InlinePatch::new(starter())),
            Start::Template { template: 0 },
            HELD,
            scoring(),
        );
        assert_eq!(patch.validate(), Ok(()));
        assert_eq!(
            patch.with_seed(5).unwrap_err().code(),
            "no_seed",
            "a patch target has no seed"
        );
        assert_eq!(patch.version(), 2);
    }
}
