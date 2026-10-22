//! **Owner: lane DUI** (the duel card, feature `duel`). Pre-declared by lane S0e (R4/R5); **lane SKD** wrote the first
//! wording for freeze point F2d.
//!
//! Verification and the two results (docs/duels.md, "Verification" and "Wording"): "Checked on your computer: their
//! patch scores 93.4 %, as reported", or "… 92.9 % here (they reported 93.4 % on engine 0.3.0; you run 0.4.0)". Your
//! machine's scores decide; the time reads "reported by their app; enth can't check times". Never "proof", "verified
//! time", "verified player" or "anti-cheat". A job result, so the card asks for no frames of its own.

use egui::Ui;
use enth_session::duel::{AnswerState, DuelView, VerifyState};

use super::card::{line, refusal};
use super::share::hundredths;
use super::state::Duel;
use crate::Theme;

/// A claim in tenths of a percent.
fn tenths(t: u16) -> String {
    format!("{:.1} %", f64::from(t) / 10.0)
}

/// What the card says about their scorecard, their answer and the check, one line each.
pub fn lines(view: &DuelView) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(card) = &view.card {
        let [_, claim] = card.card.share_lines();
        out.push(format!(
            "Their scorecard, as reported by their app: {claim}. Swap answer codes to check it."
        ));
        if view.engine_differs {
            out.push(format!(
                "They play engine {}; you run {}: a check shows both numbers.",
                card.card.fields().engine,
                enth_challenge::EngineVersion::CURRENT
            ));
        }
        if view.scorers_differ {
            out.push(
                "Their scorecard was scored with a newer scorer set: this computer checks with its own, and shows both numbers."
                    .to_owned(),
            );
        }
    }
    match view.answer {
        AnswerState::None => out.push("No answer code from them yet.".to_owned()),
        AnswerState::Sealed => out.push(
            "Their answer code is here, sealed until you have submitted or given up.".to_owned(),
        ),
        _ => {}
    }
    if view.verify == VerifyState::Running {
        out.push("Checking on your computer…".to_owned());
    }
    if let Some(c) = &view.checked {
        let theirs = hundredths(c.theirs.sound);
        match c.claim {
            Some(_) if c.matches_claim() == Some(true) => out.push(format!(
                "Checked on your computer: their patch scores {theirs}, as reported."
            )),
            Some(claim) => {
                let mut s = format!(
                    "Checked on your computer: their patch scores {theirs} here (they reported {} on engine {}",
                    tenths(claim.claimed_sound_tenths),
                    claim.claimed_engine
                );
                if claim.engine_differs() {
                    s = format!("{s}; you run {}", claim.engine);
                }
                if claim.scorers_differ() {
                    s = format!(
                        "{s}; scored with set {}, checked with set {}",
                        claim.claimed_scorers, claim.scorers
                    );
                }
                out.push(format!("{s})."));
            }
            None => out.push(format!(
                "Checked on your computer: their patch scores {theirs}."
            )),
        }
        match c.commitment_ok() {
            Some(true) => {
                out.push("Their answer is the one their scorecard fixed.".to_owned());
            }
            Some(false) => out.push(
                "Their answer is not the one their scorecard fixed: it was changed after the scorecard."
                    .to_owned(),
            ),
            None => {}
        }
        if let Some(own) = c.own {
            let mut s = format!(
                "Your patch scores {} on the same check",
                hundredths(own.sound)
            );
            if !c.own_current {
                s.push_str(" (before your latest submit)");
            }
            out.push(format!("{s}."));
        }
        if let Some(card) = &view.card {
            let ds = u64::from(card.card.fields().time_ds);
            out.push(format!(
                "Their time, {}, is reported by their app; enth can't check times.",
                super::share::clock(ds * 100)
            ));
        }
    }
    if view.used {
        out.push("Their patch is loaded: Undo takes it back.".to_owned());
    }
    out
}

/// Their side, as text on the card.
pub fn text(duel: &Duel, ui: &mut Ui, t: &Theme) {
    let Some(view) = duel.view() else {
        line(
            ui,
            t,
            "Paste your friend's message (a challenge, their scorecard or their answer) and Read it.",
            true,
        );
        return;
    };
    let view = view.clone();
    line(ui, t, &format!("Duel on {}", view.code), false);
    for l in lines(&view) {
        line(ui, t, &l, false);
    }
    if let VerifyState::Failed(f) = &view.verify {
        refusal(ui, t, &format!("{}: {}", f.message, f.hint));
    }
}
