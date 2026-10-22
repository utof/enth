//! **Owner: lane DUI** (the duel card, feature `duel`). Pre-declared by lane S0e (R4/R5); **lane SKD** wrote the first
//! buttons for freeze point F2d.
//!
//! The sealed answer (docs/duels.md, "Spoilers, and the duel as a lesson"): a friend's answer stays sealed until you have
//! submitted or given up, a UI guard and not cryptography (the session refuses every intent on it until then). Afterwards
//! you can check it, audition their patch or load it as **one undo step**, so a duel becomes a lesson.

use egui::Ui;
use enth_session::duel::{AnswerState, VerifyState};

use super::card::button;
use super::state::{Duel, DuelIntent};
use crate::Theme;
use crate::editor::cards::Intents;
use crate::theme::glyph;

/// **Verify**, **Use theirs** and **▶ Theirs** / **■ Stop**, after **Read** in one row that never moves: enabled once
/// their answer is open, and use and audition once it is checked.
pub fn buttons(duel: &Duel, ui: &mut Ui, t: &Theme, out: &mut Intents) {
    let view = duel.view();
    let open = view.is_some_and(|v| v.answer == AnswerState::Open);
    let running = view.is_some_and(|v| v.verify == VerifyState::Running);
    let checked = view
        .and_then(|v| v.checked.as_ref())
        .is_some_and(|c| c.has_patch);
    button(
        ui,
        t,
        "Verify",
        "Verify their answer",
        open && !running,
        DuelIntent::Verify,
        out,
    )
    .on_hover_text("Score their answer, and yours, on this computer")
    .on_disabled_hover_text(if view.is_some_and(|v| v.answer == AnswerState::Sealed) {
        "Their answer opens once you have submitted or given up"
    } else {
        "Paste their answer code first"
    });
    button(
        ui,
        t,
        "Use theirs",
        "Use their patch",
        open && checked,
        DuelIntent::Use,
        out,
    )
    .on_hover_text("Load their patch (one undo step); end your round first");
    let auditioning = view.is_some_and(|v| v.auditioning);
    if auditioning {
        button(
            ui,
            t,
            &format!("{} Stop", glyph::STOP),
            "Stop audition",
            true,
            DuelIntent::StopAudition,
            out,
        );
    } else {
        button(
            ui,
            t,
            &format!("{} Theirs", glyph::PLAY),
            "Audition their patch",
            open && checked,
            DuelIntent::Audition,
            out,
        )
        .on_hover_text("Play their patch with the challenge's phrase");
    }
}
