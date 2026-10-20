//! Intents: what the editor's components ask for, collected during a frame and applied after it.
//!
//! Components never touch the patch or the session. A knob drag pushes `Begin`, then a `Set` whenever the value
//! changes, then `End`; a click on a segmented control pushes one `Select`; the card's switch pushes `Enable`. The
//! editor (lane ED1) applies the frame's intents in order through the session: `Begin` opens one undo gesture (and,
//! for an address bound to an automation slot or a macro, one host gesture), the edits inside it merge into one undo
//! step, and `End` closes it. A `Set` outside a gesture (a double-click reset) is one undo step on its own.
//!
//! [`Intent::ops`] gives the patch ops of the edits, so the editor and the tests don't each re-derive them.

use enth_params::DEPTH;
use enth_patch::{
    MacroIndex, ModuleId, ModuleInstance, ModuleKind, Op, ParamAddr, Patch, Position, Route,
    RouteAddr,
};
use enth_session::bridge;

use super::view::CardKey;

/// The control a drag belongs to. `Begin` and `End` carry it, so the editor can check that a gesture closes on the
/// control that opened it, and close it itself when the pointer is released wherever the control went.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Grip {
    /// A parameter's knob.
    Param(ParamAddr),
    /// A handle on a module's display, by index in the display's handle order: an envelope's attack peak (0),
    /// decay and sustain corner (1, one gesture over two addresses) and release start (2).
    Handle(ModuleId, usize),
    /// A macro knob.
    Macro(MacroIndex),
    /// A cell of the modulation matrix.
    Route(RouteAddr),
}

/// One thing a control asks for.
#[derive(Clone, Debug, PartialEq)]
pub enum Intent {
    /// The preset browser (lane PBU): the header's ◀ ▶, Browse and Save, the browse panel's loads, auditions and
    /// favorites, the Save dialog and the crash offer. The editor hands it to its preset state
    /// ([`Presets`](crate::editor::browser::Presets)); a load is the session's one-undo-step preset load.
    Preset(crate::editor::browser::PresetIntent),
    /// A drag started on a control: the edits up to the matching `End` are one gesture (one undo step).
    Begin(Grip),
    /// A parameter's new plain value, in real units and valid for its spec.
    Set { addr: ParamAddr, value: f32 },
    /// A macro knob's new position, 0–1.
    SetMacro { index: MacroIndex, value: f32 },
    /// A route's new depth, in percent of the destination's range (−100..=100). The route may not exist yet: a drag
    /// on an empty matrix cell creates it.
    SetDepth { route: RouteAddr, depth: f32 },
    /// The drag ended.
    End(Grip),
    /// An enum parameter's item was picked on a segmented control: one edit, no gesture. The value is the item's
    /// index.
    Select { addr: ParamAddr, index: usize },
    /// A toggle parameter was switched: one edit, no gesture.
    Toggle { addr: ParamAddr, on: bool },
    /// A module's enable switch.
    Enable { id: ModuleId, enabled: bool },
    /// Remove a route (a double-click on a matrix cell, the control menu's "Remove … modulation").
    Unroute(RouteAddr),
    /// A source's ⊕ was dropped on a control (drag-to-modulate, lane ED2): insert the route at
    /// [`NEW_ROUTE_DEPTH`](crate::editor::modulate::NEW_ROUTE_DEPTH) unless it exists. One edit, no gesture.
    Route(RouteAddr),
    /// Expose a parameter to the host through an automation slot (the right-click "Automate" item): the slot that
    /// already drives it, else the lowest free one (`bridge::free_slot`, F1b decision 4). One edit, no gesture.
    Automate(ParamAddr),
    /// Clear every automation slot that drives a parameter ("Stop automating"). One edit, no gesture.
    StopAutomating(ParamAddr),
    /// A card's "More" disclosure was opened or closed. Editor view state, not an edit: the editor keeps it outside
    /// the patch (it survives the editor closing, so it belongs to the session's editor state, not to egui).
    ShowMore { card: CardKey, open: bool },
    /// The header's Undo button.
    Undo,
    /// The header's Redo button.
    Redo,
    /// Add a module of `kind` at `index` of its section, with every parameter at its default and the lowest free id
    /// (`Patch::free_id`). From the header's Add menu, a region's right-click menu, the dashed "+ Add" slot and a card's
    /// "Add after" (lane AM). One edit (one undo step), no gesture; a full section is refused by the session
    /// (`too_many`, the status line's hint).
    AddModule { kind: ModuleKind, index: usize },
    /// Remove a module with its routes and slot assignments (a card's "Remove"; undo puts them back). The amplitude
    /// envelope is refused (`in_use_as_amp_env`). One edit, no gesture (lane AM).
    RemoveModule(ModuleId),
    /// Move a module to `index` of its section (a card's "Move up" or "Move down"). One edit, no gesture (lane AM).
    MoveModule { id: ModuleId, index: usize },
    /// Add a copy of a module right after it: its kind, values and switch, a new id and no label. One edit, no
    /// gesture; its routes are not copied (lane AM).
    Duplicate(ModuleId),
    /// Set a module's label, or clear it (`None`: the card shows its kind and number again). One edit, no gesture
    /// (lane AM).
    Rename { id: ModuleId, label: Option<String> },
    /// Make an envelope the voice's amplitude envelope (a card's "Make amp envelope"). One edit, no gesture (lane AM).
    MakeAmpEnv(ModuleId),
}

impl Intent {
    /// The patch ops this intent stands for, against `patch` as it is before the intent: empty for gesture edges,
    /// view state, undo and redo (the session's own calls). A depth on a route that does not exist yet inserts it
    /// at the end of the summation order; a depth of 0 on a missing route is nothing.
    pub fn ops(&self, patch: &Patch) -> Vec<Op> {
        match self {
            // The browser loads and saves through the session's own preset calls, never as ops (lane PBU).
            #[allow(
                clippy::match_same_arms,
                reason = "lane PBU's arm stays apart from the shared one, so concurrent lanes add arms without a conflict"
            )]
            Intent::Preset(_) => Vec::new(),
            Intent::Set { addr, value } => vec![Op::SetParam {
                addr: addr.clone(),
                value: *value,
            }],
            Intent::Select { addr, index } => vec![Op::SetParam {
                addr: addr.clone(),
                value: *index as f32,
            }],
            Intent::Toggle { addr, on } => vec![Op::SetParam {
                addr: addr.clone(),
                value: if *on { 1.0 } else { 0.0 },
            }],
            Intent::SetMacro { index, value } => vec![Op::SetMacro {
                index: *index,
                value: *value,
            }],
            Intent::Enable { id, enabled } => vec![Op::SetEnabled {
                id: id.clone(),
                enabled: *enabled,
            }],
            Intent::SetDepth { route, depth } => {
                let depth = DEPTH.clamp(*depth);
                let exists = patch
                    .routes
                    .iter()
                    .any(|r| r.src == route.src && r.dst == route.dst);
                if exists {
                    vec![Op::SetDepth {
                        route: route.clone(),
                        depth,
                    }]
                } else if depth == 0.0 {
                    Vec::new()
                } else {
                    vec![Op::InsertRoute {
                        index: patch.routes.len(),
                        route: Route {
                            src: route.src.clone(),
                            dst: route.dst.clone(),
                            depth,
                        },
                    }]
                }
            }
            Intent::Unroute(route) => vec![Op::RemoveRoute {
                route: route.clone(),
            }],
            Intent::Route(route) => {
                let exists = patch
                    .routes
                    .iter()
                    .any(|r| r.src == route.src && r.dst == route.dst);
                if exists {
                    Vec::new()
                } else {
                    vec![Op::InsertRoute {
                        index: patch.routes.len(),
                        route: Route {
                            src: route.src.clone(),
                            dst: route.dst.clone(),
                            depth: crate::editor::modulate::NEW_ROUTE_DEPTH,
                        },
                    }]
                }
            }
            Intent::Automate(addr) => {
                let automated = bridge::slots_for(patch, addr).next().is_some();
                match bridge::free_slot(patch) {
                    Some(slot) if !automated => vec![Op::SetSlot {
                        slot,
                        addr: Some(addr.clone()),
                    }],
                    _ => Vec::new(),
                }
            }
            Intent::StopAutomating(addr) => bridge::slots_for(patch, addr)
                .map(|slot| Op::SetSlot { slot, addr: None })
                .collect(),
            Intent::AddModule { kind, index } => vec![Op::AddModule {
                at: Position {
                    section: kind.section(),
                    index: *index,
                },
                module: ModuleInstance::new(patch.free_id(*kind), *kind),
            }],
            Intent::RemoveModule(id) => vec![Op::RemoveModule { id: id.clone() }],
            Intent::MoveModule { id, index } => match patch.find(id) {
                Some((from, _)) if from.index != *index => vec![Op::Move {
                    id: id.clone(),
                    to: Position {
                        section: from.section,
                        index: *index,
                    },
                }],
                _ => Vec::new(),
            },
            Intent::Duplicate(id) => match patch.find(id) {
                Some((at, original)) => {
                    let mut copy = original.clone();
                    copy.id = patch.free_id(original.kind);
                    copy.label = None;
                    vec![Op::AddModule {
                        at: Position {
                            section: at.section,
                            index: at.index + 1,
                        },
                        module: copy,
                    }]
                }
                None => Vec::new(),
            },
            Intent::Rename { id, label } => match patch.find(id) {
                Some((_, m)) if m.label != *label => vec![Op::SetLabel {
                    id: id.clone(),
                    label: label.clone(),
                }],
                _ => Vec::new(),
            },
            Intent::MakeAmpEnv(id) => {
                if patch.voice.amp_env == *id {
                    Vec::new()
                } else {
                    vec![Op::SetAmpEnv { id: id.clone() }]
                }
            }
            Intent::Begin(_)
            | Intent::End(_)
            | Intent::ShowMore { .. }
            | Intent::Undo
            | Intent::Redo => Vec::new(),
        }
    }
}

/// The control under the pointer, for the status line: "Filter 1 › Cutoff", "1.50 kHz", "Env 2 +40 %".
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Hovered {
    /// Card and parameter names.
    pub path: String,
    /// The value in real units.
    pub value: String,
    /// The routes into it, with their depths.
    pub modulation: String,
}

/// A frame's intents, in the order the controls pushed them, and the control under the pointer.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Intents {
    list: Vec<Intent>,
    /// The control under the pointer (or being dragged) this frame, if any.
    pub hovered: Option<Hovered>,
}

impl Intents {
    /// Records one intent.
    pub fn push(&mut self, intent: Intent) {
        self.list.push(intent);
    }

    /// This frame's intents so far.
    pub fn as_slice(&self) -> &[Intent] {
        &self.list
    }

    /// Takes the intents, leaving none (the hovered control stays).
    pub fn take(&mut self) -> Vec<Intent> {
        std::mem::take(&mut self.list)
    }

    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    /// Appends another frame part's intents after these; its hovered control wins if it has one.
    pub fn extend(&mut self, other: Intents) {
        self.list.extend(other.list);
        if other.hovered.is_some() {
            self.hovered = other.hovered;
        }
    }
}

impl IntoIterator for Intents {
    type Item = Intent;
    type IntoIter = std::vec::IntoIter<Intent>;

    fn into_iter(self) -> Self::IntoIter {
        self.list.into_iter()
    }
}
