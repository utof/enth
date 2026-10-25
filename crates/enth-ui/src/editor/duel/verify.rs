//! **Owner: lane DUI** (the duel card, feature `duel`). Pre-declared by lane S0e (R4/R5); **lane SKD** wrote the first
//! wording for freeze point F2d; lane DUI added the two results side by side.
//!
//! Verification and the two results (docs/duels.md, "Verification" and "Wording"): "Checked on your computer: their
//! patch scores 93.4 %, as reported", or "… 92.9 % here (they reported 93.4 % on engine 0.3.0; you run 0.4.0)". Your
//! machine's scores decide; the time reads "reported by their app; enth can't check times". Never "proof", "verified
//! time", "verified player" or "anti-cheat". A job result, so the card asks for no frames of its own.
//!
//! **Side by side** ([`sides`]): a grid of you and them, row by row: sound, passed, submits, time, assists. Your sound is
//! your round's until a check re-scores it beside theirs on this computer; theirs is **reported** (their scorecard)
//! until the check, then **checked on your computer**. Times and assists stay reported on both sides. Each cell's
//! accessible name says whose it is and what it is ("Their sound: 93.4 %, checked on your computer"), so kittest and
//! screen readers read the grid by label.

use egui::{Grid, Label, RichText, Role, Ui};
use enth_challenge::Outcome;
use enth_challenge::result::CardAssists;
use enth_challenge::{AssistsUsed, EngineVersion};
use enth_session::duel::{AnswerState, DuelOwn, DuelView, FriendScorecard, VerifyState};

use super::card::{labeled, line, refusal};
use super::share::{clock, hundredths};
use super::state::Duel;
use crate::Theme;

/// A claim in tenths of a percent.
fn tenths(t: u16) -> String {
    format!("{}.{} %", t / 10, t % 10)
}

/// What the card says about their scorecard, their answer and the check, one line each (the numbers are in the grid,
/// [`sides`]; these say where they come from and what was checked).
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
                EngineVersion::CURRENT
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
        _ => {
            if view.checked.is_none() && view.verify != VerifyState::Running {
                out.push(
                    "Their answer is open: Verify scores it, and yours, on this computer."
                        .to_owned(),
                );
            }
        }
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
                clock(ds * 100)
            ));
        }
    }
    if view.used {
        out.push("Their patch is loaded: Undo takes it back.".to_owned());
    }
    out
}

/// A round's help in words ("none", "2 hints, live meter").
fn assists_text(hints: u8, flags: [(bool, &str); 4]) -> String {
    let mut parts = Vec::new();
    match hints {
        0 => {}
        1 => parts.push("1 hint".to_owned()),
        n => parts.push(format!("{n} hints")),
    }
    for (on, what) in flags {
        if on {
            parts.push(what.to_owned());
        }
    }
    if parts.is_empty() {
        "none".to_owned()
    } else {
        parts.join(", ")
    }
}

fn own_assists(a: AssistsUsed) -> String {
    assists_text(
        a.hints,
        [
            (a.live_meter, "live meter"),
            (a.dim_unused, "dim unused"),
            (a.closest, "how close"),
            (a.revealed, "target revealed"),
        ],
    )
}

fn card_assists(a: CardAssists) -> String {
    assists_text(
        a.hints,
        [
            (a.live_meter, "live meter"),
            (a.dim_unused, "dim unused"),
            (a.closest, "how close"),
            (a.revealed, "target revealed after passing"),
        ],
    )
}

fn yes_no(b: bool) -> &'static str {
    if b { "yes" } else { "no" }
}

/// One side's cells, top to bottom: sound, passed, submits, time, assists; each with its accessible tail
/// ("checked on your computer", "reported").
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Side {
    /// Sound, and where the number comes from.
    pub sound: (String, &'static str),
    /// Passed.
    pub passed: (String, &'static str),
    /// Submits up to the compared one.
    pub submits: String,
    /// The time, reported.
    pub time: String,
    /// The help used, reported.
    pub assists: String,
}

/// Your side: the round's numbers, your sound re-scored beside theirs once checked.
fn your_side(own: &DuelOwn, view: &DuelView) -> Side {
    let rescored = view
        .checked
        .as_ref()
        .and_then(|c| c.own.filter(|_| c.own_current));
    let sound = match (rescored, own.score) {
        (Some(r), _) => (hundredths(r.sound), "checked on your computer"),
        (None, Some(s)) => (hundredths(s.sound), "your round"),
        (None, None) => ("no score".to_owned(), "your round"),
    };
    let passed = rescored.map_or_else(
        || {
            (
                yes_no(own.outcome == Outcome::Passed).to_owned(),
                "your round",
            )
        },
        |r| (yes_no(r.passed).to_owned(), "checked on your computer"),
    );
    Side {
        sound,
        passed,
        submits: own.number.unwrap_or(own.submits).to_string(),
        time: clock(own.time_ms.unwrap_or(own.active_ms)),
        assists: own_assists(own.assists),
    }
}

/// Their side: their scorecard as reported, their sound and pass checked on this computer once checked.
fn their_side(card: Option<&FriendScorecard>, view: &DuelView) -> Side {
    let f = card.map(|c| c.card.fields());
    let checked = view.checked.as_ref();
    let sound = match (checked, f) {
        (Some(c), _) => (hundredths(c.theirs.sound), "checked on your computer"),
        (None, Some(f)) => (tenths(f.sound_tenths), "reported"),
        (None, None) => ("–".to_owned(), "not known yet"),
    };
    let passed = match (checked, f) {
        (Some(c), _) => (
            yes_no(c.theirs.passed).to_owned(),
            "checked on your computer",
        ),
        (None, Some(f)) => (yes_no(f.passed).to_owned(), "reported"),
        (None, None) => ("–".to_owned(), "not known yet"),
    };
    Side {
        sound,
        passed,
        submits: f.map_or_else(|| "–".to_owned(), |f| f.submits.to_string()),
        time: f.map_or_else(|| "–".to_owned(), |f| clock(u64::from(f.time_ds) * 100)),
        assists: f.map_or_else(|| "–".to_owned(), |f| card_assists(f.assists)),
    }
}

/// Both sides, when there is something to compare: yours once you have played, theirs once their scorecard or a check
/// is here.
pub fn both(view: &DuelView) -> Option<(Option<Side>, Option<Side>)> {
    let yours = view.own.as_ref().map(|o| your_side(o, view));
    let theirs = (view.card.is_some() || view.checked.is_some())
        .then(|| their_side(view.card.as_ref(), view));
    (yours.is_some() || theirs.is_some()).then_some((yours, theirs))
}

/// A grid row: what it is, then your cell and theirs, each with its accessible tail.
type Row<'a> = (&'a str, (&'a str, Option<&'a str>), (&'a str, Option<&'a str>));

/// A grid cell: `text`, named "`who` `what`: `text`" (with its tail when it has one).
fn cell(ui: &mut Ui, t: &Theme, text: &str, name: &str, tail: Option<&str>, strong: bool) {
    let rich = RichText::new(text).font(if strong {
        t.text.strong.clone()
    } else {
        t.text.label.clone()
    });
    let shown = ui.add(Label::new(rich.color(t.color.text.primary)));
    let name = match tail {
        Some(tail) if !tail.is_empty() => format!("{name}: {text}, {tail}"),
        _ => format!("{name}: {text}"),
    };
    labeled(&shown, Role::Label, &name);
}

/// **Side by side**: you and them, row by row, with where each number comes from under the grid.
pub fn sides(duel: &Duel, ui: &mut Ui, t: &Theme) {
    let Some(view) = duel.view() else { return };
    let Some((yours, theirs)) = both(view) else {
        return;
    };
    let name = duel.shell().name.trim();
    let them = if name.is_empty() { "Them" } else { name };
    let dash = || Side {
        sound: ("–".to_owned(), ""),
        passed: ("–".to_owned(), ""),
        submits: "–".to_owned(),
        time: "–".to_owned(),
        assists: "–".to_owned(),
    };
    let yours = yours.unwrap_or_else(dash);
    let theirs = theirs.unwrap_or_else(dash);
    Grid::new("enth.duel.sides")
        .num_columns(3)
        .min_col_width(t.size.duel.side_column)
        .spacing([t.space.gap, t.space.tight])
        .show(ui, |ui| {
            ui.label("");
            cell(ui, t, "You", "Column", None, true);
            cell(ui, t, them, "Column", None, true);
            ui.end_row();
            let rows: [Row<'_>; 5] = [
                (
                    "sound",
                    (&yours.sound.0, Some(yours.sound.1)),
                    (&theirs.sound.0, Some(theirs.sound.1)),
                ),
                (
                    "passed",
                    (&yours.passed.0, Some(yours.passed.1)),
                    (&theirs.passed.0, Some(theirs.passed.1)),
                ),
                ("submits", (&yours.submits, None), (&theirs.submits, None)),
                (
                    "time",
                    (&yours.time, Some("reported")),
                    (&theirs.time, Some("reported")),
                ),
                (
                    "assists",
                    (&yours.assists, Some("reported")),
                    (&theirs.assists, Some("reported")),
                ),
            ];
            for (what, (mine, my_tail), (their, their_tail)) in rows {
                let head = format!("{}{}", what[..1].to_uppercase(), &what[1..]);
                ui.label(
                    RichText::new(&head)
                        .font(t.text.label.clone())
                        .color(t.color.text.label),
                );
                cell(ui, t, mine, &format!("Your {what}"), my_tail, false);
                cell(ui, t, their, &format!("Their {what}"), their_tail, false);
                ui.end_row();
            }
        });
    line(
        ui,
        t,
        "Scores marked checked come from this computer; their unchecked numbers, times and assists are reported.",
        true,
    );
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
    sides(duel, ui, t);
    for l in lines(&view) {
        line(ui, t, &l, false);
    }
    if let VerifyState::Failed(f) = &view.verify {
        refusal(ui, t, &format!("{}: {}", f.message, f.hint));
    }
}
