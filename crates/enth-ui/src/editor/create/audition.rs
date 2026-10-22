//! **Owner: lane PUI** (the Create view, feature `create`). Pre-declared by lane S0s (R3).
//!
//! The compare area under the tiles: the selected branch (the best until a tile is clicked), heard against the upload
//! and taken (docs/r3.md, "The Create view and the session"). Branches keep both the start they grew from and what
//! search made of it, Synplant-style, so you hear what the start gave and what search added:
//!
//! - a title ("Candidate 2 • template `pluck_1osc`") and a line with the start's distance, the candidate's, its renders
//!   and its rung ("start 0.812 → 0.234 • 1,240 renders • rung 2");
//! - **▶ Candidate**, **▶ Start** (its proposal) and **▶ Upload** (the job's scored clip): each plays once through the
//!   plugin's audition path (lane SK3's one-shot slot, mixed into the plugin's output);
//! - **A/B**: the candidate if the upload was played last or nothing was, else the upload, so repeated clicks
//!   alternate. **Level-matched**: the shell plays the louder of the two at the quieter one's RMS (never louder than
//!   it is), so the comparison is of sounds, not levels. Each side restarts from its beginning (a clip is at most
//!   `create::SCORE_S`, 3 s, long). The button is drawn selected while the upload was asked for last; **■ Stop** stops
//!   either; a line says what was asked for last;
//! - **Use this** and **Use the start**: the session loads that patch as **one undo step** (no host writes; refused
//!   while a lesson is live, as a preset load is), named after the file; the header's **Undo** gives the patch before
//!   back. "✓ Took candidate 2" says which was taken last.
//!
//! Nothing plays unless a button is pressed; the editor never plays audio itself (the shell renders and plays, and in
//! kittest nothing does).

use egui::{Button, Label, RichText, Ui};
use enth_session::create::{CreateView, Which};

use super::panel::{button, labeled, named};
use super::state::CreateIntent;
use super::tiles::distance_text;
use crate::Theme;
use crate::editor::cards::{Intent, Intents};
use crate::theme::glyph;

/// What was asked to play last, as the compare area says it ("the upload", "candidate 2", "candidate 2's start").
fn last_played(view: &CreateView) -> Option<String> {
    let name = |b: usize| {
        view.candidates
            .iter()
            .find(|c| c.branch == b)
            .map(|c| c.rank + 1)
    };
    if view.upload {
        return Some("the upload".to_owned());
    }
    let (b, which) = view.auditioning?;
    let n = name(b)?;
    Some(match which {
        Which::Refined => format!("candidate {n}"),
        Which::Proposal => format!("candidate {n}'s start"),
    })
}

/// Draws the compare area for `branch` of `view`; `upload_ready`: the intake read the file, so the upload can play.
pub(crate) fn show(
    view: &CreateView,
    branch: usize,
    upload_ready: bool,
    ui: &mut Ui,
    t: &Theme,
    out: &mut Intents,
) {
    let Some(c) = view.candidates.iter().find(|c| c.branch == branch) else {
        return;
    };
    let n = c.rank + 1;
    let title = format!("Candidate {n} {} {}", glyph::BULLET, c.label());
    let shown = ui.add(
        Label::new(
            RichText::new(&title)
                .font(t.text.strong.clone())
                .color(t.color.text.primary),
        )
        .truncate(),
    );
    labeled(&shown, egui::Role::Label, &title);
    let detail = format!(
        "start {} {} {} {} {} renders {} rung {}",
        distance_text(c.proposal_distance),
        glyph::SEND,
        distance_text(c.distance),
        glyph::BULLET,
        super::progress::count(c.renders),
        glyph::BULLET,
        c.rung
    );
    let shown = ui.add(
        Label::new(
            RichText::new(&detail)
                .font(t.text.label.clone())
                .color(t.color.text.label),
        )
        .truncate(),
    );
    labeled(&shown, egui::Role::Label, &detail);
    ui.horizontal(|ui| {
        button(
            ui,
            t,
            &format!("{} Candidate", glyph::PLAY),
            &format!("Play candidate {n}"),
            true,
            CreateIntent::Audition(branch, Which::Refined),
            out,
        );
        button(
            ui,
            t,
            &format!("{} Start", glyph::PLAY),
            &format!("Play the start of candidate {n}"),
            true,
            CreateIntent::Audition(branch, Which::Proposal),
            out,
        );
        button(
            ui,
            t,
            &format!("{} Upload", glyph::PLAY),
            "Play the upload",
            upload_ready,
            CreateIntent::AuditionUpload,
            out,
        );
    });
    ui.horizontal(|ui| {
        let ab = ui.add_enabled(
            upload_ready,
            Button::new(RichText::new("A/B").font(t.text.body.clone())).selected(view.upload),
        );
        named(&ab, &format!("A/B candidate {n} against the upload"));
        let ab = ab.on_hover_text(
            "The candidate, then the upload, level-matched: each click plays the other",
        );
        if ab.clicked() {
            out.push(Intent::Create(CreateIntent::Flip(branch)));
        }
        let playing = view.upload || view.auditioning.is_some();
        button(
            ui,
            t,
            &format!("{} Stop", glyph::STOP),
            "Stop audition",
            playing,
            CreateIntent::StopAudition,
            out,
        );
        if let Some(last) = last_played(view) {
            let text = format!("{} {last}", glyph::PLAY);
            let shown = ui.add(
                Label::new(
                    RichText::new(&text)
                        .font(t.text.label.clone())
                        .color(t.color.accent.value),
                )
                .truncate(),
            );
            labeled(&shown, egui::Role::Label, &format!("Played last: {last}"));
        }
    });
    ui.horizontal(|ui| {
        button(
            ui,
            t,
            "Use this",
            &format!("Use candidate {n}"),
            true,
            CreateIntent::Use(branch, Which::Refined),
            out,
        )
        .on_hover_text("Load it as your patch: one undo step (the header's Undo gives yours back)");
        button(
            ui,
            t,
            "Use the start",
            &format!("Use the start of candidate {n}"),
            true,
            CreateIntent::Use(branch, Which::Proposal),
            out,
        );
    });
    if let Some((b, which)) = view.used {
        let used = view
            .candidates
            .iter()
            .find(|c| c.branch == b)
            .map(|c| c.rank + 1);
        if let Some(m) = used {
            let what = match which {
                Which::Refined => format!("candidate {m}"),
                Which::Proposal => format!("candidate {m}'s start"),
            };
            let text = format!(
                "{} Took {what}: the header's Undo gives your patch back",
                glyph::GOOD
            );
            let shown = ui.add(
                Label::new(
                    RichText::new(&text)
                        .font(t.text.label.clone())
                        .color(t.color.text.label),
                )
                .wrap(),
            );
            labeled(&shown, egui::Role::Label, &format!("Took {what}"));
        }
    }
}
