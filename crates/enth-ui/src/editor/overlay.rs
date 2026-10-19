//! The overlay: what Learn and Recreate will lay over the editor, and nothing in P1b (lane ED1, research 02 D10).
//!
//! A lesson step dims or hides the controls it doesn't use; a challenge's results put a score ring with its glyph
//! (✓ ~ ✗) on each control. Both are data an [`Overlay`] gives per parameter and per module; the editor applies it to
//! the read model before drawing ([`apply`]), so no component knows about lessons or challenges. P1b ships
//! [`NoOverlay`], which leaves everything as it is. Recreate never hides ("difficulty never locks features"), it
//! dims and scores; only a lesson hides.

use enth_patch::{ModuleId, ParamAddr};

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
    /// Not drawn (a lesson step that doesn't use it). Stable ids keep its state while it is hidden.
    Hidden,
}

/// What an overlay says about one parameter.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ParamOverlay {
    pub emphasis: Emphasis,
    /// A score ring on the control, always drawn with its glyph.
    pub score: Option<Score>,
}

/// Emphasis and scores laid over the editor by a lesson or a challenge.
pub trait Overlay {
    /// One parameter's emphasis and score.
    fn param(&self, addr: &ParamAddr) -> ParamOverlay;
    /// A whole module's emphasis (a module the lesson doesn't use).
    fn module(&self, id: &ModuleId) -> Emphasis;
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

/// Applies `overlay` to the cards: each parameter takes its own emphasis and score, a dimmed module dims every
/// parameter in it, and a hidden module's card is dropped.
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
        for p in &mut card.params {
            let o = overlay.param(&p.addr);
            p.emphasis = match (module, o.emphasis) {
                (_, Emphasis::Hidden) => Emphasis::Hidden,
                (Emphasis::Dimmed, _) | (_, Emphasis::Dimmed) => Emphasis::Dimmed,
                _ => Emphasis::Normal,
            };
            p.score = o.score;
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
}
