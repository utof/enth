//! The overlay: what Learn and Recreate will lay over the editor, and nothing in P1b (lane ED1, research 02 D10).
//!
//! A lesson step dims or hides the controls it doesn't use; a challenge's results put a score ring with its glyph
//! (✓ ~ ✗) on each control. Both are data an [`Overlay`] gives per parameter and per module; the editor applies it to
//! the read model before drawing ([`apply`]), so no component knows about lessons or challenges. P1b ships
//! [`NoOverlay`], which leaves everything as it is. Recreate never hides ("difficulty never locks features"), it
//! dims and scores; only a lesson hides. A Recreate round's revealed hint also points at a control or a module
//! ([`Pointer`], [`Overlay::module_hint`]; lane HINT, FU1): an outline in the value accent, never a value.
//!
//! **A lesson step** (lane LUI, R2) adds two things, both static, so neither asks for a frame: what the step is about
//! is [`Emphasis::Highlighted`] (a thin outline in the value accent, [`paint_highlight`]), and what it locks is
//! [`Overlay::locked`] / [`Overlay::module_locked`] (drawn dimmed with a lock glyph, [`paint_lock`]; hovering it names
//! the reason on the status line, as a refused drag-to-modulate target does). **Locks are the session's rule**
//! (`SessionError::Locked`): the overlay only draws them.

use enth_patch::{ModuleId, ParamAddr, RouteAddr};

use super::cards::{CardKey, CardView};
use crate::theme::Score;

/// How strongly a control or a module is shown.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Emphasis {
    /// As usual.
    #[default]
    Normal,
    /// Drawn in the dimmed ink; still editable.
    Dimmed,
    /// What a lesson step is about (lane LUI): drawn as usual, with a static outline in the value accent
    /// ([`paint_highlight`]), so it asks for no frames. A module's highlight outlines its card.
    Highlighted,
    /// Not drawn (a lesson step that doesn't use it). Stable ids keep its state while it is hidden.
    Hidden,
}

/// Where a revealed hint points on a control (a Recreate round, lane HINT): an outline in the value accent, with the
/// way to turn the control when the hint's text says so (↑ raise, ↓ lower). Never a value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pointer {
    /// Look here (a categorical parameter, a route's destination, or a hint that names no direction).
    Here,
    /// Turn it up.
    Raise,
    /// Turn it down.
    Lower,
}

impl Pointer {
    /// The glyph drawn at the outline's corner, if any.
    pub fn glyph(self) -> Option<&'static str> {
        match self {
            Pointer::Here => None,
            Pointer::Raise => Some(crate::theme::glyph::RAISE),
            Pointer::Lower => Some(crate::theme::glyph::LOWER),
        }
    }
}

/// What an overlay says about one parameter.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ParamOverlay {
    pub emphasis: Emphasis,
    /// A score ring on the control, always drawn with its glyph.
    pub score: Option<Score>,
    /// A revealed hint points at the control (a Recreate round, lane HINT).
    pub hint: Option<Pointer>,
}

/// Emphasis and scores laid over the editor by a lesson or a challenge.
pub trait Overlay {
    /// One parameter's emphasis and score.
    fn param(&self, addr: &ParamAddr) -> ParamOverlay;
    /// A whole module's emphasis (a module the lesson doesn't use).
    fn module(&self, id: &ModuleId) -> Emphasis;
    /// A score ring on a module's enable toggle (a Recreate round's enable-flag row; lane RUI). None by default.
    fn enable(&self, _id: &ModuleId) -> Option<Score> {
        None
    }
    /// A score ring on a modulation route's cell or row (a Recreate round's route row; lane RUI). None by default.
    fn route(&self, _route: &RouteAddr) -> Option<Score> {
        None
    }
    /// A revealed hint points at this module as a whole (a Recreate round's hint, lane HINT): its card is outlined.
    /// False by default.
    fn module_hint(&self, _id: &ModuleId) -> bool {
        false
    }
    /// The lesson step locks this parameter (lane LUI): drawn dimmed with a lock glyph, and hovering it says why. The
    /// session refuses its edits whatever the overlay says. False by default.
    fn locked(&self, _addr: &ParamAddr) -> bool {
        false
    }
    /// The lesson step locks this module as a whole (`filter1.*`, lane LUI): its card's header carries the lock glyph
    /// and every parameter in it is locked. False by default.
    fn module_locked(&self, _id: &ModuleId) -> bool {
        false
    }
}

/// Outlines what a lesson step is about (lane LUI, [`Emphasis::Highlighted`]): a thin stroke in the value accent a
/// tight gap outside `rect`. A revealed hint's thick outline ([`paint_hint`]) reads stronger, so both can show.
pub fn paint_highlight(painter: &egui::Painter, t: &crate::Theme, rect: egui::Rect) {
    painter.rect_stroke(
        rect.expand(t.space.tight),
        t.radius.small,
        t.stroke.thin(t.color.accent.value),
        egui::StrokeKind::Outside,
    );
}

/// A lock glyph on `rect`'s top right corner (lane LUI): a filled body under a shackle, `size.learn.lock` tall, in
/// the label ink, centred on the corner like a switch's score glyph, so it stays clear of a centred label. Drawn as two
/// shapes, since Inter has no padlock. (A locked control is never one the round changes, so its score ring, if any,
/// reads Good and the two rarely meet; the hint's arrow sits inside the top left.)
pub fn paint_lock(painter: &egui::Painter, t: &crate::Theme, rect: egui::Rect) {
    let side = t.size.learn.lock;
    let ink = t.color.text.label;
    let half = side * 0.5;
    let corner = egui::pos2(rect.right() - half, rect.top() - half + t.space.hairline);
    let body = egui::Rect::from_min_size(corner + egui::vec2(0.0, half), egui::vec2(side, half));
    painter.circle_stroke(
        egui::pos2(corner.x + half, body.top()),
        half - t.space.hairline,
        t.stroke.thin(ink),
    );
    painter.rect_filled(body, t.radius.small, ink);
}

/// Outlines the control or card a revealed hint points at (lane HINT): a thick stroke in the value accent a tight gap
/// outside `rect`, and the [`Pointer`]'s glyph (↑ or ↓) inside its top left corner when it has one. Never colour
/// alone for a direction: the glyph says it.
pub fn paint_hint(painter: &egui::Painter, t: &crate::Theme, rect: egui::Rect, pointer: Pointer) {
    let ink = t.color.accent.value;
    painter.rect_stroke(
        rect.expand(t.space.tight),
        t.radius.small,
        t.stroke.thick(ink),
        egui::StrokeKind::Outside,
    );
    // Inside the outline's top left corner (the score glyph takes the top right), clear of the stroke.
    if let Some(glyph) = pointer.glyph() {
        painter.text(
            egui::pos2(
                rect.left() + t.space.hairline,
                rect.top() + t.space.hairline,
            ),
            egui::Align2::LEFT_TOP,
            glyph,
            t.text.label.clone(),
            ink,
        );
    }
}

/// Paints a score ring around a control's `rect` with its glyph at the top right corner: the ring of a switch, a
/// segmented control, an enable toggle or a route (lane RUI; a knob draws its own on its dial). Never colour alone.
pub fn paint_score(painter: &egui::Painter, t: &crate::Theme, rect: egui::Rect, score: Score) {
    let ink = t.color.score.of(score);
    painter.rect_stroke(
        rect.expand(t.space.hairline),
        t.radius.small,
        t.stroke.thin(ink),
        egui::StrokeKind::Outside,
    );
    painter.text(
        egui::pos2(
            rect.right() + t.space.hairline,
            rect.top() - t.space.hairline,
        ),
        egui::Align2::CENTER_CENTER,
        score.glyph(),
        t.text.label.clone(),
        ink,
    );
}

/// The P1b overlay: everything normal, no scores.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NoOverlay;

impl Overlay for NoOverlay {
    fn param(&self, _addr: &ParamAddr) -> ParamOverlay {
        ParamOverlay::default()
    }

    fn module(&self, _id: &ModuleId) -> Emphasis {
        Emphasis::Normal
    }
}

/// Two overlays, one over the other (lane LUI): a lesson's Recreate step lays the round's rings, hint and dimming
/// (`top`) over the step's highlights, locks and hidden modules (`base`). The stronger emphasis wins (hidden, then
/// dimmed, then highlighted); scores, rings and hints are the top's, locks the base's.
pub struct Layered<'a> {
    /// The round's overlay.
    pub top: &'a dyn Overlay,
    /// The lesson step's overlay.
    pub base: &'a dyn Overlay,
}

/// The stronger of two emphases: hidden, then dimmed, then highlighted.
fn stronger(a: Emphasis, b: Emphasis) -> Emphasis {
    let rank = |e: Emphasis| match e {
        Emphasis::Normal => 0,
        Emphasis::Highlighted => 1,
        Emphasis::Dimmed => 2,
        Emphasis::Hidden => 3,
    };
    if rank(a) >= rank(b) { a } else { b }
}

impl Overlay for Layered<'_> {
    fn param(&self, addr: &ParamAddr) -> ParamOverlay {
        let (top, base) = (self.top.param(addr), self.base.param(addr));
        // The step's own ring and pointer, when it has one ("How close am I?"'s answer, lane LFU), read over the
        // round's: they answer the question just asked. Otherwise the round's.
        ParamOverlay {
            emphasis: stronger(top.emphasis, base.emphasis),
            score: base.score.or(top.score),
            hint: base.hint.or(top.hint),
        }
    }

    fn module(&self, id: &ModuleId) -> Emphasis {
        stronger(self.top.module(id), self.base.module(id))
    }

    fn enable(&self, id: &ModuleId) -> Option<Score> {
        self.top.enable(id)
    }

    fn route(&self, route: &RouteAddr) -> Option<Score> {
        self.top.route(route)
    }

    fn module_hint(&self, id: &ModuleId) -> bool {
        self.top.module_hint(id)
    }

    fn locked(&self, addr: &ParamAddr) -> bool {
        self.base.locked(addr)
    }

    fn module_locked(&self, id: &ModuleId) -> bool {
        self.base.module_locked(id)
    }
}

/// Applies `overlay` to the cards: each parameter takes its own emphasis and score, a dimmed module dims every
/// parameter in it, and a hidden module's card is dropped. A highlighted module outlines its card; a locked module
/// locks every parameter in it (lane LUI).
pub fn apply(overlay: &dyn Overlay, cards: &mut Vec<CardView>) {
    cards.retain(|card| match &card.key {
        CardKey::Module(id) => overlay.module(id) != Emphasis::Hidden,
        CardKey::Voice | CardKey::Master => true,
    });
    for card in cards {
        let module = match &card.key {
            CardKey::Module(id) => overlay.module(id),
            CardKey::Voice | CardKey::Master => Emphasis::Normal,
        };
        card.enable_score = match &card.key {
            CardKey::Module(id) => overlay.enable(id),
            CardKey::Voice | CardKey::Master => None,
        };
        card.hinted = match &card.key {
            CardKey::Module(id) => overlay.module_hint(id),
            CardKey::Voice | CardKey::Master => false,
        };
        card.highlighted = module == Emphasis::Highlighted;
        card.locked = match &card.key {
            CardKey::Module(id) => overlay.module_locked(id),
            CardKey::Voice | CardKey::Master => false,
        };
        for p in &mut card.params {
            let o = overlay.param(&p.addr);
            p.emphasis = match (module, o.emphasis) {
                (_, Emphasis::Hidden) => Emphasis::Hidden,
                (Emphasis::Dimmed, _) | (_, Emphasis::Dimmed) => Emphasis::Dimmed,
                (_, Emphasis::Highlighted) => Emphasis::Highlighted,
                _ => Emphasis::Normal,
            };
            p.score = o.score;
            p.hint = o.hint;
            p.locked = card.locked || overlay.locked(&p.addr);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::cards::view;
    use enth_patch::Patch;

    /// A lesson that hides the level and dims every other parameter of `osc1`.
    struct Lesson;

    impl Overlay for Lesson {
        fn param(&self, addr: &ParamAddr) -> ParamOverlay {
            ParamOverlay {
                emphasis: if addr.param() == "level" {
                    Emphasis::Hidden
                } else {
                    Emphasis::Normal
                },
                score: (addr.param() == "wave").then_some(Score::Good),
                hint: None,
            }
        }

        fn module(&self, id: &ModuleId) -> Emphasis {
            if id.as_str() == "env1" {
                Emphasis::Hidden
            } else {
                Emphasis::Dimmed
            }
        }
    }

    #[test]
    fn no_overlay_changes_nothing_and_a_lesson_dims_hides_and_scores() {
        let patch = Patch::init();
        let before = view::all(&patch);
        let mut cards = before.clone();
        apply(&NoOverlay, &mut cards);
        assert_eq!(cards, before);
        apply(&Lesson, &mut cards);
        assert!(
            cards
                .iter()
                .all(|c| c.key != CardKey::Module(ModuleId::new("env1").unwrap()))
        );
        let osc = &cards[0];
        assert_eq!(osc.param("level").unwrap().emphasis, Emphasis::Hidden);
        assert_eq!(osc.param("semi").unwrap().emphasis, Emphasis::Dimmed);
        assert_eq!(osc.param("wave").unwrap().score, Some(Score::Good));
    }

    /// A lesson step (lane LUI) that highlights the cutoff and the envelope, and locks osc 1 as a whole and the
    /// filter's drive.
    struct Step;

    impl Overlay for Step {
        fn param(&self, addr: &ParamAddr) -> ParamOverlay {
            ParamOverlay {
                emphasis: if addr.to_string() == "filter1.cutoff" {
                    Emphasis::Highlighted
                } else {
                    Emphasis::Normal
                },
                score: None,
                hint: None,
            }
        }

        fn module(&self, id: &ModuleId) -> Emphasis {
            if id.as_str() == "env1" {
                Emphasis::Highlighted
            } else {
                Emphasis::Normal
            }
        }

        fn locked(&self, addr: &ParamAddr) -> bool {
            addr.to_string() == "filter1.drive"
        }

        fn module_locked(&self, id: &ModuleId) -> bool {
            id.as_str() == "osc1"
        }
    }

    #[test]
    fn a_round_over_a_lesson_step_keeps_both() {
        let layered = Layered {
            top: &Lesson,
            base: &Step,
        };
        let cutoff: ParamAddr = "filter1.cutoff".parse().unwrap();
        let wave: ParamAddr = "osc1.wave".parse().unwrap();
        assert_eq!(
            layered.param(&cutoff).emphasis,
            Emphasis::Highlighted,
            "the step's outline shows through"
        );
        assert_eq!(
            layered.param(&wave).score,
            Some(Score::Good),
            "the round's ring"
        );
        assert_eq!(
            layered.module(&ModuleId::new("env1").unwrap()),
            Emphasis::Hidden,
            "the stronger emphasis wins"
        );
        assert!(layered.module_locked(&ModuleId::new("osc1").unwrap()));
        assert!(layered.locked(&"filter1.drive".parse().unwrap()));
    }

    #[test]
    fn a_lesson_step_highlights_and_locks() {
        let mut patch = Patch::init();
        patch.voice_fx.push(enth_patch::ModuleInstance::new(
            ModuleId::new("filter1").unwrap(),
            enth_patch::ModuleKind::Svf,
        ));
        let mut cards = view::all(&patch);
        apply(&Step, &mut cards);
        let card = |id: &str| {
            cards
                .iter()
                .find(|c| c.key == CardKey::Module(ModuleId::new(id).unwrap()))
                .unwrap()
        };
        let filter = card("filter1");
        assert_eq!(
            filter.param("cutoff").unwrap().emphasis,
            Emphasis::Highlighted
        );
        assert!(!filter.param("cutoff").unwrap().locked);
        assert!(filter.param("drive").unwrap().locked);
        assert!(!filter.highlighted && !filter.locked);
        let osc = card("osc1");
        assert!(osc.locked, "the module's lock is on its card");
        assert!(
            osc.params.iter().all(|p| p.locked),
            "and on every parameter"
        );
        let env = card("env1");
        assert!(env.highlighted, "a module's highlight outlines its card");
        assert!(
            env.params.iter().all(|p| p.emphasis == Emphasis::Normal),
            "its controls draw as usual"
        );
        let mut plain = view::all(&patch);
        apply(&NoOverlay, &mut plain);
        assert!(
            plain
                .iter()
                .all(|c| !c.highlighted && !c.locked && c.params.iter().all(|p| !p.locked))
        );
    }
}
