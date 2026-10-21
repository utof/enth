//! **Owner: lane PUI** (the Create view, feature `create`). **Lane SK3** wrote this minimal panel for freeze point
//! F2s, which PUI replaces with the real view (the drop zone, the folder browser, the K tiles, A/B against the upload).
//!
//! The panel takes a column at the window's right edge while it is open (`size.create.pane`), under the layout's top
//! row; the layout, Learn's included, places the rest beside it (`frame.rs`, `beside`; docs/editor.md, "Beside the
//! Create panel"). It draws, in plain text with the theme's tokens, top to bottom:
//!
//! - the title and **Close**;
//! - the drop zone (`size.create.drop_zone` high): the chosen file's name, or what to drop; it lights while files are
//!   dragged over the window (any file drop on the editor chooses it, whatever the view: the frame reads egui's
//!   `dropped_files`), with a path field and **Choose** as the fallback where drops do not reach the editor;
//! - **Start** and **Cancel**, then the phase, one progress bar (renders against the cap, seconds against the time
//!   cap) and the job's notes;
//! - what the intake found: how the file decoded, the note, the length, the warnings;
//! - the K candidates from best to worst: provenance, state and distance, **▶** (play it once), **▶ start** (the
//!   proposal it grew from) and **Use this** (one undo step); **■ Stop** while one plays.
//!
//! Every button has an accessible name, so kittest drives it by label. It asks for no frames of its own: the shell's
//! posts wake the editor.

use egui::{
    Align, Button, Label, Layout, ProgressBar, Rect, RichText, Role, ScrollArea, Sense, TextEdit,
    Ui, UiBuilder, WidgetInfo, vec2,
};
use enth_session::create::{CandidateState, CreatePhase, CreateStop, Which};

use super::state::{Create, CreateIntent};
use crate::Theme;
use crate::editor::cards::{Intent, Intents};

/// Names a widget for screen readers and tests, whatever its text shows.
fn named(response: &egui::Response, label: &str) {
    let enabled = response.enabled();
    response.widget_info(|| WidgetInfo::labeled(Role::Button, enabled, label));
}

/// A button with `text`, named `label`, enabled when `enabled`; pushes `intent` when clicked.
fn button(
    ui: &mut Ui,
    t: &Theme,
    text: &str,
    label: &str,
    enabled: bool,
    intent: CreateIntent,
    out: &mut Intents,
) {
    let response = ui.add_enabled(
        enabled,
        Button::new(RichText::new(text).font(t.text.body.clone())),
    );
    named(&response, label);
    if response.clicked() {
        out.push(Intent::Create(intent));
    }
}

fn line(ui: &mut Ui, t: &Theme, text: &str, color: egui::Color32) {
    ui.add(Label::new(RichText::new(text).font(t.text.label.clone()).color(color)).wrap());
}

/// Draws the panel into `rect` from `create`; `hovering`: files are being dragged over the window. Returns how far its
/// content reached past `rect` (it scrolls; the frame report lists it).
pub(crate) fn show(
    create: &mut Create,
    ui: &mut Ui,
    t: &Theme,
    rect: Rect,
    hovering: bool,
    out: &mut Intents,
) -> egui::Vec2 {
    let mut ui = ui.new_child(
        UiBuilder::new()
            .max_rect(rect)
            .layout(Layout::top_down(Align::Min))
            .id_salt("enth.create.panel"),
    );
    ui.set_clip_rect(rect.intersect(ui.clip_rect()));
    ui.painter()
        .rect_filled(rect, t.radius.medium, t.color.surface.card);
    let inner = rect.shrink2(vec2(t.space.gap, t.space.tight));
    let mut col = ui.new_child(
        UiBuilder::new()
            .max_rect(inner)
            .layout(Layout::top_down(Align::Min)),
    );
    let shown = ScrollArea::vertical()
        .id_salt("enth.create.scroll")
        .auto_shrink([false, false])
        .show(&mut col, |ui| {
            ui.spacing_mut().item_spacing = vec2(t.space.gap, t.space.tight);
            body(create, ui, t, hovering, out);
        });
    let over = shown.content_size - shown.inner_rect.size();
    vec2(over.x.max(0.0), over.y.max(0.0))
}

fn phase_text(phase: CreatePhase) -> &'static str {
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

fn state_text(state: CandidateState) -> &'static str {
    match state {
        CandidateState::Proposed => "proposed",
        CandidateState::Searching => "searching",
        CandidateState::Done => "done",
        CandidateState::Pruned => "pruned",
        _ => "",
    }
}

fn body(create: &mut Create, ui: &mut Ui, t: &Theme, hovering: bool, out: &mut Intents) {
    ui.horizontal(|ui| {
        ui.label(
            RichText::new("Create from sound")
                .font(t.text.title.clone())
                .color(t.color.text.primary),
        );
        button(
            ui,
            t,
            "Close",
            "Close create",
            true,
            CreateIntent::Toggle,
            out,
        );
    });
    drop_zone(create, ui, t, hovering, out);
    let view = create.view().cloned();
    let running = view.as_ref().is_some_and(|v| v.phase.running());
    ui.horizontal(|ui| {
        button(
            ui,
            t,
            "Start",
            "Start create",
            create.file().is_some() && !running,
            CreateIntent::Start,
            out,
        );
        button(
            ui,
            t,
            "Cancel",
            "Cancel create",
            running
                && view
                    .as_ref()
                    .is_some_and(|v| v.phase != CreatePhase::Cancelling),
            CreateIntent::Cancel,
            out,
        );
    });
    if let Some(said) = create.said() {
        line(ui, t, said, t.color.score.bad);
    }
    let Some(view) = view else {
        return;
    };
    let status = ui.label(
        RichText::new(format!("{}  {}", view.file, phase_text(view.phase)))
            .font(t.text.label.clone())
            .color(t.color.text.label),
    );
    named(&status, phase_text(view.phase));
    if let Some(p) = &view.progress {
        ui.add(
            ProgressBar::new(p.fraction()).text(
                RichText::new(format!(
                    "{} / {} renders · {:.0} / {:.0} s",
                    p.renders, p.budget, p.seconds, p.cap_s
                ))
                .font(t.text.label.clone()),
            ),
        );
    }
    if let Some(e) = &view.error {
        line(
            ui,
            t,
            &format!("{} ({})", e.message, e.code),
            t.color.score.bad,
        );
        line(ui, t, &e.hint, t.color.text.label);
    }
    if let Some(intake) = &view.intake {
        line(ui, t, &intake.decoded, t.color.text.label);
        let perf = &intake.report.perf;
        let pitch = perf
            .f0_hz
            .map_or_else(|| "unpitched".to_owned(), |f| format!("{f:.1} Hz"));
        line(
            ui,
            t,
            &format!(
                "note {} ({pitch}), held {:.2} s, clip {:.2} s",
                perf.key, perf.hold_s, perf.duration_s
            ),
            t.color.text.label,
        );
        for w in intake.warnings() {
            line(ui, t, &format!("⚠ {w}"), t.color.score.near);
        }
    }
    for note in &view.notes {
        line(ui, t, note, t.color.text.dimmed);
    }
    if view.candidates.is_empty() {
        return;
    }
    ui.horizontal(|ui| {
        ui.label(
            RichText::new("Candidates")
                .font(t.text.strong.clone())
                .color(t.color.text.primary),
        );
        button(
            ui,
            t,
            "■ Stop",
            "Stop audition",
            view.auditioning.is_some(),
            CreateIntent::StopAudition,
            out,
        );
    });
    let mut ranked: Vec<_> = view.candidates.iter().collect();
    ranked.sort_by_key(|c| c.rank);
    for c in ranked {
        let distance = c
            .distance
            .map_or_else(|| "–".to_owned(), |d| format!("{d:.3}"));
        let used = view.used.is_some_and(|(b, _)| b == c.branch);
        let text = format!(
            "{}. {} · {} · {distance}{}",
            c.rank + 1,
            c.label(),
            state_text(c.state),
            if used { " · in use" } else { "" }
        );
        line(ui, t, &text, t.color.text.primary);
        ui.horizontal(|ui| {
            let n = c.rank + 1;
            button(
                ui,
                t,
                "▶",
                &format!("Play candidate {n}"),
                true,
                CreateIntent::Audition(c.branch, Which::Refined),
                out,
            );
            button(
                ui,
                t,
                "▶ start",
                &format!("Play the start of candidate {n}"),
                true,
                CreateIntent::Audition(c.branch, Which::Proposal),
                out,
            );
            button(
                ui,
                t,
                "Use this",
                &format!("Use candidate {n}"),
                true,
                CreateIntent::Use(c.branch, Which::Refined),
                out,
            );
        });
    }
}

/// The drop zone, the path field and **Choose**.
fn drop_zone(create: &mut Create, ui: &mut Ui, t: &Theme, hovering: bool, out: &mut Intents) {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(width, t.size.create.drop_zone), Sense::hover());
    let (fill, text) = if hovering {
        (
            t.color.surface.raised,
            "Drop to choose this file".to_owned(),
        )
    } else {
        (
            t.color.surface.recessed,
            create.file().map_or_else(
                || "Drop a sound file here (WAV, FLAC, Ogg Vorbis, MP3)".to_owned(),
                |f| {
                    f.file_name().map_or_else(
                        || f.to_string_lossy().into_owned(),
                        |n| n.to_string_lossy().into_owned(),
                    )
                },
            ),
        )
    };
    ui.painter().rect_filled(rect, t.radius.small, fill);
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        text,
        t.text.body.clone(),
        t.color.text.primary,
    );
    ui.horizontal(|ui| {
        let field = ui.add(
            TextEdit::singleline(&mut create.path_text)
                .hint_text("or type a file's path")
                .font(t.text.label.clone())
                .desired_width(width - t.size.create.choose),
        );
        field.widget_info(|| WidgetInfo::labeled(Role::TextInput, true, "Sound file path"));
        let typed = create.path_text.trim().to_owned();
        let response = ui.add_enabled(
            !typed.is_empty(),
            Button::new(RichText::new("Choose").font(t.text.body.clone())),
        );
        named(&response, "Choose file");
        if response.clicked() {
            out.push(Intent::Create(CreateIntent::Choose(typed.into())));
        }
    });
}
