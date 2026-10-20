//! The crash offer (lane PBU; the crash file itself is lane PBC's `enth_plugin::crash`). When the shell finds crash
//! files not offered yet, a small notice sits in the body's lower right corner: "enth stopped unexpectedly last
//! time", with "Show report" and "Dismiss". It is a foreground area, not a modal: the editor keeps working under and
//! beside it. Either button marks every waiting report as offered (`crash::mark_seen` renames it; nothing is
//! deleted), so it is offered once. "Show report" puts the newest report over the body, with Copy and Close. Nothing
//! is sent anywhere: the report stays a local file, and the user decides what to do with it.

use egui::{
    Align, Align2, Area, Button, Frame, Id, Key, Layout, Order, Rect, RichText, ScrollArea, Ui,
    UiBuilder, vec2,
};

use super::PresetIntent;
use crate::Theme;
use crate::editor::cards::{Intent, Intents, ids};

/// The crash files a shell found, behind the editor's back: the plugin implements it over its crash folder
/// (`enth_plugin::crash::pending`, `read_report`, `mark_seen`), tests with a fake. The editor does no I/O itself.
pub trait CrashReports: Send + Sync {
    /// How many reports wait to be offered.
    fn count(&self) -> usize;
    /// The newest report's text, or why it could not be read.
    fn newest(&self) -> Result<String, String>;
    /// Where the reports are, in words the user can find ("~/Documents/enth/Crashes").
    fn location(&self) -> String;
    /// Mark every waiting report as offered.
    fn mark_seen(&mut self) -> Result<(), String>;
}

/// The offer's state.
#[derive(Default)]
pub(super) struct CrashOffer {
    /// The reports waiting, while the notice shows.
    reports: Option<Box<dyn CrashReports>>,
    /// The report over the body, once shown; with where it is and how many there were.
    shown: Option<Shown>,
}

struct Shown {
    text: String,
    location: String,
    count: usize,
}

impl CrashOffer {
    pub(super) fn offer(&mut self, reports: Option<Box<dyn CrashReports>>) {
        self.reports = reports.filter(|r| r.count() > 0);
    }

    pub(super) fn notice(&self) -> bool {
        self.reports.is_some()
    }

    pub(super) fn text(&self) -> Option<&str> {
        self.shown.as_ref().map(|s| s.text.as_str())
    }

    /// "Show report": read the newest, mark them all offered, keep the text for the sheet.
    pub(super) fn show(&mut self) {
        let Some(mut reports) = self.reports.take() else {
            return;
        };
        let count = reports.count().max(1);
        let mut text = reports
            .newest()
            .unwrap_or_else(|e| format!("The report could not be read: {e}"));
        if let Err(e) = reports.mark_seen() {
            text = format!(
                "(It could not be marked as seen, so it will be offered again: {e})\n\n{text}"
            );
        }
        self.shown = Some(Shown {
            text,
            location: reports.location(),
            count,
        });
    }

    /// "Dismiss": mark them all offered, show nothing.
    pub(super) fn dismiss(&mut self) {
        if let Some(mut reports) = self.reports.take() {
            let _ = reports.mark_seen();
        }
    }
}

/// The notice, as a foreground area pinned to `body`'s lower right corner.
pub(super) fn show_notice(ui: &Ui, t: &Theme, body: Rect, out: &mut Intents) {
    let c = &t.color;
    Area::new(Id::unique("enth.crash.notice"))
        .order(Order::Foreground)
        .pivot(Align2::RIGHT_BOTTOM)
        .fixed_pos(body.right_bottom() - vec2(t.space.gap, t.space.gap))
        .show(ui.ctx(), |ui| {
            Frame::new()
                .fill(c.surface.raised)
                .corner_radius(t.radius.medium)
                .inner_margin(t.space.pad)
                .show(ui, |ui| {
                    ui.set_width(t.size.browser.notice_width);
                    ui.spacing_mut().item_spacing = vec2(t.space.tight, t.space.gap);
                    ui.label(
                        RichText::new("enth stopped unexpectedly last time")
                            .font(t.text.strong.clone())
                            .color(c.text.primary),
                    );
                    ui.label(
                        RichText::new(
                            "It wrote a report on this computer. Nothing was sent anywhere.",
                        )
                        .font(t.text.label.clone())
                        .color(c.text.label),
                    );
                    ui.horizontal(|ui| {
                        let button =
                            |text: &str| Button::new(RichText::new(text).font(t.text.body.clone()));
                        if ui.add(button("Show report")).clicked() {
                            out.push(Intent::Preset(PresetIntent::ShowCrash));
                        }
                        if ui.add(button("Dismiss")).clicked() {
                            out.push(Intent::Preset(PresetIntent::DismissCrash));
                        }
                    });
                });
        });
}

/// The report over the body.
pub(super) fn show_report(
    offer: &CrashOffer,
    ui: &mut Ui,
    t: &Theme,
    body: Rect,
    out: &mut Intents,
) {
    let c = &t.color;
    let mut ui = ui.new_child(
        UiBuilder::new()
            .max_rect(body)
            .scope_id(ids::region("crash"))
            .layout(Layout::top_down(Align::Min)),
    );
    ui.shrink_clip_rect(body);
    ui.painter()
        .rect_filled(body, t.radius.medium, c.surface.card);
    if ui.input(|i| i.key_pressed(Key::Escape)) {
        out.push(Intent::Preset(PresetIntent::Close));
    }
    let Some(shown) = &offer.shown else {
        return;
    };
    let inner = body.shrink(t.space.pad);
    let mut ui = ui.new_child(
        UiBuilder::new()
            .max_rect(inner)
            .layout(Layout::top_down(Align::Min)),
    );
    ui.spacing_mut().item_spacing = vec2(t.space.tight, t.space.gap);
    ui.label(
        RichText::new("enth stopped unexpectedly last time")
            .font(t.text.title.clone())
            .color(c.text.primary),
    );
    let more = match shown.count {
        1 => String::new(),
        n => format!(" This is the newest of {n}."),
    };
    ui.label(
        RichText::new(format!(
            "The report is a file on this computer, in {}.{more} Nothing was sent anywhere; it holds no patch, \
             project or personal data. Copy it into a bug report if you want to.",
            shown.location
        ))
        .font(t.text.label.clone())
        .color(c.text.label),
    );
    ui.horizontal(|ui| {
        let button = |text: &str| Button::new(RichText::new(text).font(t.text.body.clone()));
        if ui.add(button("Copy report")).clicked() {
            ui.ctx().copy_text(shown.text.clone());
        }
        if ui.add(button("Close report")).clicked() {
            out.push(Intent::Preset(PresetIntent::Close));
        }
    });
    ScrollArea::both()
        .id_salt("enth.crash.report")
        .auto_shrink([false, false])
        .show(&mut ui, |ui| {
            ui.label(
                RichText::new(&shown.text)
                    .font(t.text.value.clone())
                    .color(c.text.primary),
            );
        });
}
