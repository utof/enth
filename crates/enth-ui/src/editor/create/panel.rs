//! **Owner: lane PUI** (the Create view, feature `create`). Lane SK3 wrote a minimal panel here for freeze point F2s;
//! lane PUI replaced it with the view.
//!
//! The panel takes a column at the window's right edge while it is open (`size.create.pane`), under the layout's top
//! row; the layout, Learn's included, places the rest beside it (`frame.rs`, `beside`; docs/editor.md, "The Create
//! view"). Top to bottom, on `surface.card`:
//!
//! - the title, **Create from sound**, and **×** (Close);
//! - **the drop zone** ([`super::drop_zone`]): what to drop, or the chosen file and its length and note;
//! - **Browse…** (the folder browser) and **Start** (a run again; a drop or a pick starts by itself), and the last
//!   refusal in the bad-score ink;
//! - either **the folder browser** ([`super::browser`]), or the job: **what the intake found**
//!   ([`super::drop_zone::findings`]), **the progress** with Cancel and the rungs ([`super::progress`]), **the K
//!   tiles** ([`super::tiles`]), **the compare area** for the selected tile ([`super::audition`]) and, last, the
//!   shell's notes on the job (the objective, the k-NN bank, the cap), in the dimmed ink.
//!
//! The column scrolls if its content does not fit (the frame report lists it as `create`); at 1200 × 800 a job with
//! K = 8 and its notes fits (`tests/create_view.rs`). Every control has an accessible name, so kittest drives it by
//! label. It asks for no frames of its own: the shell's posts wake the editor, and nothing in it animates.

use egui::{
    Align, Button, FontId, Galley, Label, Layout, Rect, Response, RichText, Role, ScrollArea, Ui,
    UiBuilder, WidgetInfo, vec2,
};

use super::state::{Create, CreateIntent};
use crate::Theme;
use crate::editor::cards::{Intent, Intents};
use crate::theme::glyph;

/// Names a widget for screen readers and tests as a button, whatever its text shows.
pub(crate) fn named(response: &Response, label: &str) {
    labeled(response, Role::Button, label);
}

/// Names a widget for screen readers and tests with `role`.
pub(crate) fn labeled(response: &Response, role: Role, label: &str) {
    let enabled = response.enabled();
    response.widget_info(|| WidgetInfo::labeled(role, enabled, label));
}

/// A button with `text`, named `label`, enabled when `enabled`; pushes `intent` when clicked.
pub(crate) fn button(
    ui: &mut Ui,
    t: &Theme,
    text: &str,
    label: &str,
    enabled: bool,
    intent: CreateIntent,
    out: &mut Intents,
) -> Response {
    let response = ui.add_enabled(
        enabled,
        Button::new(RichText::new(text).font(t.text.body.clone())),
    );
    named(&response, label);
    if response.clicked() {
        out.push(Intent::Create(intent));
    }
    response
}

/// `text` cut to `width` in `font`, ending with "…" when it was cut.
pub(crate) fn elided(ui: &Ui, text: &str, font: &FontId, width: f32) -> String {
    let fits = |s: &str| -> bool {
        let g: std::sync::Arc<Galley> =
            ui.painter()
                .layout_no_wrap(s.to_owned(), font.clone(), ui.visuals().text_color());
        g.size().x <= width
    };
    if fits(text) {
        return text.to_owned();
    }
    let chars: Vec<char> = text.chars().collect();
    let (mut lo, mut hi) = (0, chars.len());
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        let candidate: String = chars[..mid].iter().collect::<String>() + glyph::MENU;
        if fits(&candidate) {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    chars[..lo].iter().collect::<String>() + glyph::MENU
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
    let inner = rect.shrink2(vec2(t.space.gap, t.space.gap));
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

fn body(create: &mut Create, ui: &mut Ui, t: &Theme, hovering: bool, out: &mut Intents) {
    ui.horizontal(|ui| {
        ui.label(
            RichText::new("Create from sound")
                .font(t.text.title.clone())
                .color(t.color.text.primary),
        );
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            button(
                ui,
                t,
                glyph::CLOSE,
                "Close create",
                true,
                CreateIntent::Toggle,
                out,
            )
            .on_hover_text("Close the panel (a running job goes on)");
        });
    });
    super::drop_zone::zone(create, ui, t, hovering);
    let running = create.running();
    ui.horizontal(|ui| {
        let browsing = create.browsing();
        let from = create.browse_from();
        let response = ui.add(
            Button::new(RichText::new("Browse…").font(t.text.body.clone())).selected(browsing),
        );
        named(&response, "Browse files");
        if response.clicked() {
            out.push(Intent::Create(if browsing {
                CreateIntent::CloseBrowser
            } else {
                CreateIntent::Browse(from)
            }));
        }
        button(
            ui,
            t,
            "Start",
            "Start create",
            create.file().is_some() && !running,
            CreateIntent::Start,
            out,
        )
        .on_hover_text("Make patches from the chosen sound (again: new candidates)")
        .on_disabled_hover_text(if running {
            "A job runs: cancel it first, or wait for it"
        } else {
            "Drop or choose a sound file first"
        });
    });
    if let Some(said) = create.said() {
        let text = format!("{} {said}", glyph::BAD);
        let shown = ui.add(
            Label::new(
                RichText::new(&text)
                    .font(t.text.label.clone())
                    .color(t.color.score.bad),
            )
            .wrap(),
        );
        labeled(&shown, Role::Label, &text);
    }
    if create.browsing() {
        super::browser::show(create, ui, t, out);
        return;
    }
    let Some(view) = create.view().cloned() else {
        return;
    };
    super::drop_zone::findings(&view, ui, t);
    ui.add_space(t.space.tight);
    super::progress::show(create, &view, ui, t, out);
    if !view.candidates.is_empty() {
        ui.add_space(t.space.tight);
        super::tiles::show(&view, create.selected(), ui, t, out);
        if let Some(branch) = create.selected() {
            ui.add_space(t.space.tight);
            super::audition::show(&view, branch, create.has_intake(), ui, t, out);
        }
    }
    if !view.notes.is_empty() {
        ui.add_space(t.space.tight);
        for note in &view.notes {
            ui.add(
                Label::new(
                    RichText::new(note)
                        .font(t.text.label.clone())
                        .color(t.color.text.dimmed),
                )
                .wrap(),
            );
        }
    }
}
