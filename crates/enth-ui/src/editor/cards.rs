//! Module cards built from the catalogue's `ParamSpec`s, one presenter for every kind, with widget ids keyed by
//! (module id, parameter address) so they survive a re-layout (lane UI-C).
//!
//! - [`view`]: the read model. A [`CardView`] is a module instance (or the built-in `voice` or `master`) with one
//!   [`ParamView`] per parameter, built from the patch and the catalogue every frame; nothing is written per kind.
//! - [`present`]: the presentation table, UI data rather than catalogue data: which control a spec gets (knob,
//!   stepped knob, segmented control or toggle), short labels for a 56 px cell, the enum the header names.
//! - [`format`]: values in the design system's form ("1.20 kHz", "−12 st", "Saw") and back.
//! - [`Card`]: draws a `CardView` as a strip or a stacked lane card: category stripe, enable toggle with whole-card
//!   dimming, a display per kind (the envelope's corners are handles on the knobs' own power scales), the
//!   generated controls, and a "More" disclosure for the advanced tier.
//! - [`Intent`]: what the controls ask for. Cards never edit the patch: they push intents into [`Intents`], and
//!   the editor (lane ED1) turns them into ops through the session at the end of the frame.
//! - `menu`: every control's right-click menu ("Type value…" with units, "Reset to default", "Automate") and its
//!   text field (lane ED1).
//! - [`ids`]: the stable widget ids.
//!
//! Colours, sizes and fonts come from `theme` tokens; the displays' shapes are content (`shapes`).

mod card;
mod envelope;
pub mod format;
mod intent;
mod menu;
pub mod present;
pub mod shapes;
pub mod view;

pub use card::{Card, CardOutput};
pub(crate) use card::{Form, control};
pub use envelope::stages as envelope_stages;
pub use intent::{Grip, Hovered, Intent, Intents};
pub use present::Control;
pub use view::{CardKey, CardKind, CardView, ParamView, RouteView};

/// Stable widget ids. Each depends only on what the widget controls (a module id, a parameter address, a source),
/// never on where the layout puts it, so drags, hovers and egui's per-widget memory survive a re-layout, a module
/// move and undo of a removal. One id must not be drawn twice in a frame: a layout that shows a module twice salts
/// one of the two views.
pub mod ids {
    use egui::Id;
    use enth_patch::{MacroIndex, ModSource, ModuleId, ParamAddr};

    use super::CardKey;

    /// A card's scope, header and enable toggle.
    pub fn card(key: &CardKey) -> Id {
        Id::unique(("enth.card", key))
    }

    /// The control of one parameter (knob, segmented control or toggle).
    pub fn param(addr: &ParamAddr) -> Id {
        Id::unique(("enth.param", addr))
    }

    /// The handles on a module's display; handle `i` is `handles(id).with(i)`.
    pub fn handles(module: &ModuleId) -> Id {
        Id::unique(("enth.handles", module))
    }

    /// The macro bar; macro `i`'s knob is `macros().with(i)`.
    pub fn macros() -> Id {
        Id::unique("enth.macros")
    }

    /// One macro knob.
    pub fn macro_knob(index: MacroIndex) -> Id {
        macros().with(index.get())
    }

    /// A modulation source's tile and its ⊕ handle.
    pub fn source(src: &ModSource) -> Id {
        Id::unique(("enth.source", src))
    }

    /// The modulation matrix; its cells are `matrix().with(i)`, row-major.
    pub fn matrix() -> Id {
        Id::unique("enth.matrix")
    }

    /// A region's own widgets (its scroll area, its buttons).
    pub fn region(name: &str) -> Id {
        Id::unique(("enth.region", name))
    }

    /// The open "Type value…" field (one at a time; it names the parameter it edits).
    pub fn entry() -> Id {
        Id::unique("enth.entry")
    }
}
