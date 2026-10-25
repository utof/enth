//! **Owner: lane DUI** (the duel card, feature `duel`; lane SKD wrote the first copy buttons at F2d). Pre-declared by
//! lane S0e (R4/R5).
//!
//! The share line and the copy buttons (docs/duels.md, "The flow"): a human line (`enth duel · Level 2 · 1080-1000-0001-AB`,
//! `93.4 % in 2:41 · 4 submits · 0 hints`) followed by the scorecard, which spoils nothing and is sent when you finish;
//! then, once your friend has played, the answer code. **Share result** asks the shell to make both (it draws the duel's
//! salt from OS randomness and scores your answer on its worker); **Copy message** and **Copy answer** write the
//! clipboard with egui's `copy_text`, inside the click and nowhere else: the card never writes it on its own.

use egui::Ui;
use enth_challenge::Outcome;

use super::card::{button, line};

/// The longest code an enth build from before duels (R1–R3) reads: code v1's decoder refuses more symbols as "bad
/// length" before it reads anything else, so a longer answer code reads "bad length" there and a shorter one "code
/// too new" ("update enth"). Answers grow about 7 symbols per knob left at an exact value (docs/duels.md, "Sizes").
pub const OLD_BUILD_SYMBOLS: usize = 64;
use super::state::{Duel, DuelIntent};
use crate::Theme;
use crate::editor::cards::Intents;

/// **Share result**, **Copy message**, **Copy answer**, in one row that never moves.
pub fn buttons(duel: &Duel, ui: &mut Ui, t: &Theme, out: &mut Intents) {
    let making = duel.shell().making;
    button(
        ui,
        t,
        "Share result",
        "Share result",
        !making,
        DuelIntent::Share,
        out,
    )
    .on_hover_text("Make the message to send your friend, and your answer code for later")
    .on_disabled_hover_text("Making your codes…");
    let codes = duel.shell().codes.clone();
    let message = button(
        ui,
        t,
        "Copy message",
        "Copy message",
        codes.is_some(),
        DuelIntent::Copied("message"),
        out,
    )
    .on_hover_text("Your result and its scorecard: send it now, it spoils nothing");
    if message.clicked()
        && let Some(codes) = &codes
    {
        ui.ctx().copy_text(codes.message.clone());
    }
    let answer = button(
        ui,
        t,
        "Copy answer",
        "Copy answer",
        codes.is_some(),
        DuelIntent::Copied("answer"),
        out,
    )
    .on_hover_text(
        "Your answer code: it gives your patch away, so send it once your friend has played",
    );
    if answer.clicked()
        && let Some(codes) = &codes
    {
        ui.ctx().copy_text(codes.answer.clone());
    }
}

/// `2:41` from milliseconds (whole seconds, rounded down).
pub fn clock(ms: u64) -> String {
    let s = ms / 1000;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

/// A score in hundredths of a percent as the cards print it: one decimal.
pub fn hundredths(h: u16) -> String {
    format!("{:.1} %", f64::from(h) / 100.0)
}

/// Your result and the codes, as text.
pub fn text(duel: &Duel, ui: &mut Ui, t: &Theme) {
    let own = duel.view().and_then(|v| v.own.clone());
    match &own {
        Some(own) => {
            let outcome = match own.outcome {
                Outcome::Passed => "Passed",
                Outcome::GaveUp => "Gave up",
                _ => "Not passed",
            };
            let mut s = format!("{outcome}, {} submit", own.submits);
            if own.submits != 1 {
                s.push('s');
            }
            if let Some(score) = own.score {
                s = format!("{s}: {} sound", hundredths(score.sound));
            }
            if let Some(ms) = own.time_ms {
                s = format!("{s} in {} (reported)", clock(ms));
            }
            line(ui, t, &s, false);
            if !own.can_share() {
                line(
                    ui,
                    t,
                    if own.assists.revealed {
                        "A revealed target gives no scorecard"
                    } else {
                        "Pass the round, or end it, before you share"
                    },
                    true,
                );
            }
        }
        None => line(
            ui,
            t,
            "Pass a round (or end it), then Share result: the message spoils nothing.",
            true,
        ),
    }
    if duel.shell().making {
        line(ui, t, "Making your codes on this computer…", true);
    }
    if let Some(codes) = &duel.shell().codes {
        line(ui, t, &codes.message, false);
        line(
            ui,
            t,
            &format!(
                "Scorecard {} symbols; answer code {} symbols: send it once your friend has played.",
                codes.scorecard_symbols, codes.answer_symbols
            ),
            true,
        );
        if codes.answer_symbols > OLD_BUILD_SYMBOLS {
            line(
                ui,
                t,
                &format!(
                    "Your answer code is longer than {OLD_BUILD_SYMBOLS} symbols: an enth from before duels calls it \"bad length\". Your friend needs to update enth."
                ),
                true,
            );
        }
    }
    if let Some(what) = duel.copied() {
        line(ui, t, &format!("Copied the {what}."), true);
    }
}
