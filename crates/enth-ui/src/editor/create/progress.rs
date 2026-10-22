//! **Owner: lane PUI** (the Create view, feature `create`). Pre-declared by lane S0s (R3).
//!
//! The job's progress and Cancel (docs/r3.md, "The Create view and the session"): one line with the phase ("Reading
//! the file…", "Searching…", "Cancelling…", "Done", "Stopped", "Could not create") and **Cancel** at its right; one
//! progress bar (`size.create.progress_height`) for the renders against the cap or the time against the cap, whichever
//! is further, with "1,234 of 4,000 renders • 12 of 60 s" on it; and **the progress per rung** of successive halving:
//! "Rungs 8 › 4 › 2", the branches that reached each rung so far, the current one in the value accent, with the
//! branches still searching. A failure shows its message and hint instead, in the bad-score ink.
//!
//! A cancel goes out at the end of the frame it was clicked in, and the shell's pool stops within one batch (about one
//! evaluation; docs/plugin.md, "Create"); until its last post arrives the phase reads "Cancelling…" and Cancel is
//! disabled. The bar is drawn from the shell's posts only: it never animates, so it asks for no frames.

use egui::{Align, Label, Layout, ProgressBar, RichText, Role, Ui, vec2};
use enth_session::create::{CandidateState, CreatePhase, CreateStop, CreateView};

use super::panel::{button, labeled};
use super::state::{Create, CreateIntent};
use crate::Theme;
use crate::editor::cards::Intents;
use crate::theme::glyph;

/// The phase as the view says it.
pub(crate) fn phase_text(phase: CreatePhase) -> &'static str {
    match phase {
        CreatePhase::Starting => "Reading the file…",
        CreatePhase::Searching => "Searching…",
        CreatePhase::Cancelling => "Cancelling…",
        CreatePhase::Done(CreateStop::Cancelled) => "Stopped",
        CreatePhase::Done(_) => "Done",
        CreatePhase::Failed => "Could not create",
        _ => "",
    }
}

/// A count with thousands separators ("4,000").
pub(crate) fn count(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// The branches that took part in each rung so far (rung 0 holds every branch).
pub(crate) fn rungs(view: &CreateView) -> Vec<usize> {
    let top = view.candidates.iter().map(|c| c.rung).max().unwrap_or(0);
    (0..=top)
        .map(|r| view.candidates.iter().filter(|c| c.rung >= r).count())
        .collect()
}

/// Draws the phase, Cancel, the bar and the rungs (see the [module docs](self)).
pub(crate) fn show(create: &Create, view: &CreateView, ui: &mut Ui, t: &Theme, out: &mut Intents) {
    let running = view.phase.running();
    ui.horizontal(|ui| {
        let phase = phase_text(view.phase);
        let shown = ui.add(Label::new(
            RichText::new(phase)
                .font(t.text.strong.clone())
                .color(t.color.text.primary),
        ));
        labeled(&shown, Role::Label, phase);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            button(
                ui,
                t,
                "Cancel",
                "Cancel create",
                running && !create.cancelling(),
                CreateIntent::Cancel,
                out,
            );
        });
    });
    if let Some(e) = &view.error {
        let text = format!("{} {} ({})", glyph::BAD, e.message, e.code);
        let shown = ui.add(
            Label::new(
                RichText::new(&text)
                    .font(t.text.label.clone())
                    .color(t.color.score.bad),
            )
            .wrap(),
        );
        labeled(&shown, Role::Label, &text);
        ui.add(
            Label::new(
                RichText::new(&e.hint)
                    .font(t.text.label.clone())
                    .color(t.color.text.label),
            )
            .wrap(),
        );
        return;
    }
    let Some(p) = &view.progress else {
        return;
    };
    let text = format!(
        "{} of {} renders {} {:.0} of {:.0} s",
        count(p.renders),
        count(p.budget),
        glyph::BULLET,
        p.seconds,
        p.cap_s
    );
    let bar = ui.add(
        ProgressBar::new(p.fraction())
            .desired_width(ui.available_width())
            .desired_height(t.size.create.progress_height)
            .text(RichText::new(&text).font(t.text.label.clone())),
    );
    labeled(&bar, Role::ProgressIndicator, &text);
    let rungs = rungs(view);
    if rungs.is_empty() || view.candidates.is_empty() {
        return;
    }
    let searching = view
        .candidates
        .iter()
        .filter(|c| {
            matches!(
                c.state,
                CandidateState::Searching | CandidateState::Proposed
            )
        })
        .count();
    let row = ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing = vec2(t.space.tight, 0.0);
        ui.label(
            RichText::new("Rungs")
                .font(t.text.label.clone())
                .color(t.color.text.label),
        );
        let last = rungs.len() - 1;
        for (i, n) in rungs.iter().enumerate() {
            if i > 0 {
                ui.label(
                    RichText::new(glyph::PATH)
                        .font(t.text.label.clone())
                        .color(t.color.text.dimmed),
                );
            }
            let color = if i == last && running {
                t.color.accent.value
            } else {
                t.color.text.primary
            };
            ui.label(
                RichText::new(n.to_string())
                    .font(t.text.value.clone())
                    .color(color),
            );
        }
        let state = if running {
            format!("{} {searching} searching", glyph::BULLET)
        } else {
            format!("{} {} finalists", glyph::BULLET, rungs[last])
        };
        ui.label(
            RichText::new(state)
                .font(t.text.label.clone())
                .color(t.color.text.label),
        );
    });
    let label = format!(
        "Rungs {}",
        rungs
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(" › ")
    );
    labeled(&row.response, Role::Label, &label);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_have_thousands_separators() {
        assert_eq!(count(7), "7");
        assert_eq!(count(1234), "1,234");
        assert_eq!(count(4_000_000), "4,000,000");
    }
}
