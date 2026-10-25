//! **Owner: lane DUI** (the duel card, feature `duel`). Pre-declared by lane S0e (R4/R5).
//!
//! The local duel history (docs/duels.md, "The duel history"): the codes, both re-scored results and the date you played,
//! with optional names the receiver types ("from: Sam"). It lives in a per-user file the shell owns (`enth-plugin`'s
//! `duel/history.rs`: `<Documents>/enth/Duels/duels.json`), and leaves the machine only if you send it. The shell hands
//! the card its rows ([`HistoryRow`], newest first, in `ShellDuel::history`); the card lists the first [`SHOWN`], each
//! with **Open**, which asks the shell to reopen that duel ([`DuelIntent::OpenHistory`]): your result, its salt and
//! codes (so **Copy answer** gives the same code as before), and their codes, so a duel survives closing the editor,
//! the plugin and the project.
//!
//! A row says what is known and where it comes from: your sound (your round's), theirs **checked on your computer**
//! once a check ran, else **reported** by their scorecard; the date is the UTC day you played (a date, never a time
//! of day).

use egui::{Align, Layout, Ui};

use super::card::{button, line};
use super::share::hundredths;
use super::state::{Duel, DuelIntent};
use crate::Theme;
use crate::editor::cards::Intents;

/// How many rows the card lists (the newest); the rest stay in the file.
pub const SHOWN: usize = 8;

/// One duel in the history, as the card shows it (the shell builds it from its file).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HistoryRow {
    /// The entry's id in the file.
    pub id: u64,
    /// The challenge code.
    pub code: String,
    /// The UTC day you played (days since 1970-01-01), or the day it was kept.
    pub day: u32,
    /// Your compared submit's sound, in hundredths, once you have played.
    pub own: Option<u16>,
    /// Their sound as their scorecard reported it, in tenths.
    pub claimed: Option<u16>,
    /// Their sound re-scored on this computer, in hundredths, once checked.
    pub checked: Option<u16>,
    /// The check gave their claimed numbers ("as reported").
    pub as_reported: Option<bool>,
    /// The friend's name, typed by you (empty: none).
    pub name: String,
}

/// `2026-10-24` from days since 1970-01-01 (the proleptic Gregorian calendar, in integers).
pub fn date(day: u32) -> String {
    let z = i64::from(day) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

/// A row's text: the date, the code, the two sounds and their sources, the name.
pub fn row_text(row: &HistoryRow) -> String {
    let mut s = format!("{} · {}", date(row.day), row.code);
    if let Some(own) = row.own {
        s = format!("{s} · you {}", hundredths(own));
    }
    match (row.checked, row.claimed) {
        (Some(c), _) => {
            s = format!("{s} · them {} checked here", hundredths(c));
            if row.as_reported == Some(true) {
                s.push_str(", as reported");
            }
        }
        (None, Some(t)) => s = format!("{s} · them {}.{} % reported", t / 10, t % 10),
        (None, None) => {}
    }
    if !row.name.trim().is_empty() {
        s = format!("{s} · from {}", row.name.trim());
    }
    s
}

/// The history's rows, newest first, each with **Open** (not on the duel that is open now).
pub fn list(duel: &Duel, ui: &mut Ui, t: &Theme, out: &mut Intents) {
    let rows = &duel.shell().history;
    if rows.is_empty() {
        line(
            ui,
            t,
            "Your duels are kept on this computer once you share or read one.",
            true,
        );
        return;
    }
    let open = duel.view().and(duel.shell().entry);
    for row in rows.iter().take(SHOWN) {
        ui.horizontal(|ui| {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let here = open == Some(row.id);
                button(
                    ui,
                    t,
                    if here { "Open now" } else { "Open" },
                    &format!("Open duel {}", row.id),
                    !here,
                    DuelIntent::OpenHistory(row.id),
                    out,
                )
                .on_hover_text(
                    "Reopen this duel: your codes, theirs, and a new check on this computer",
                );
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    line(ui, t, &row_text(row), !here);
                });
            });
        });
    }
    if rows.len() > SHOWN {
        line(
            ui,
            t,
            &format!(
                "{} older duels are kept in the history file.",
                rows.len() - SHOWN
            ),
            true,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_from_days() {
        assert_eq!(date(0), "1970-01-01");
        assert_eq!(date(11_016), "2000-02-29");
        assert_eq!(date(20_751), "2026-10-25");
        assert_eq!(date(20_750), "2026-10-24");
    }

    #[test]
    fn a_row_says_where_each_number_comes_from() {
        let mut row = HistoryRow {
            id: 3,
            code: "1080-1000-0001-AB".to_owned(),
            day: 20_750,
            own: Some(9_340),
            claimed: Some(929),
            ..HistoryRow::default()
        };
        assert_eq!(
            row_text(&row),
            "2026-10-24 · 1080-1000-0001-AB · you 93.4 % · them 92.9 % reported"
        );
        row.checked = Some(9_290);
        row.as_reported = Some(true);
        "Sam".clone_into(&mut row.name);
        assert_eq!(
            row_text(&row),
            "2026-10-24 · 1080-1000-0001-AB · you 93.4 % · them 92.9 % checked here, as reported · from Sam"
        );
    }
}
