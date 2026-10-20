//! The header's Presets hook (feature `presets`): "◀ Name ▶ Browse Save" right after the wordmark, so the group
//! never runs into the header's right-hand controls (Add, Undo, Redo, the master knob), down to the minimum window.
//! The name is the last loaded or saved preset's, with a dot while the sound differs from it; clicking it opens the
//! browser too. ◀ and ▶ come first (report 03): inside a host the arrow keys may go to the host, not the plugin.

use egui::{
    Align, Align2, Button, Id, Layout, Rect, RichText, Role, Sense, Ui, UiBuilder, WidgetInfo,
};
use egui::{pos2, vec2};

use super::{PresetIntent, fitted, glyph_button, square};
use crate::Theme;
use crate::editor::cards::{Intent, Intents};
use crate::editor::regions::HeaderView;
use crate::theme::glyph;

/// Draws the group into the header's inner `rect` (the wordmark sits at its left end).
pub(crate) fn header(ui: &mut Ui, theme: &Theme, rect: Rect, view: &HeaderView, out: &mut Intents) {
    let t = theme;
    let c = &t.color;
    let p = &view.presets;
    let mut push = |intent: PresetIntent| out.push(Intent::Preset(intent));
    let wordmark = ui
        .painter()
        .layout_no_wrap("enth".to_owned(), t.text.title.clone(), c.text.primary)
        .size()
        .x;
    let side = t.size.control_height;
    let top = rect.center().y - side / 2.0;
    let previous = Rect::from_min_size(
        pos2(rect.left() + wordmark + t.space.section, top),
        square(t),
    );
    let name = Rect::from_min_size(
        pos2(previous.right() + t.space.tight, top),
        vec2(t.size.browser.header_name, side),
    );
    let next = Rect::from_min_size(pos2(name.right() + t.space.tight, top), square(t));

    if glyph_button(
        ui,
        t,
        previous,
        Id::unique("enth.presets.previous"),
        glyph::PREVIOUS,
        c.text.primary,
        "Previous preset",
    )
    .on_hover_text("The previous preset of the browser's list")
    .clicked()
    {
        push(PresetIntent::Step(-1));
    }

    // The name: a button that opens the browser.
    let shown = if p.name.is_empty() {
        "Untitled"
    } else {
        p.name.as_str()
    };
    let response = ui.interact(name, Id::unique("enth.presets.name"), Sense::click());
    let visuals = ui.style().interact(&response);
    if response.hovered() || p.browsing {
        ui.painter()
            .rect_filled(name, visuals.corner_radius, visuals.weak_bg_fill);
    }
    let ink = if p.name.is_empty() {
        c.text.label
    } else {
        c.text.primary
    };
    let dot = ui.painter().layout_no_wrap(
        glyph::BULLET.to_owned(),
        t.text.body.clone(),
        c.accent.value,
    );
    let room = name.width() - 2.0 * (dot.size().x + t.space.tight) - 2.0 * t.space.gap;
    let galley = fitted(ui, shown, &t.text.body, ink, room);
    let at = Align2::CENTER_CENTER
        .align_size_within_rect(galley.size(), name)
        .min;
    let end = at.x + galley.size().x;
    ui.painter().galley(at, galley, ink);
    if p.edited {
        let y = name.center().y - dot.size().y / 2.0;
        ui.painter()
            .galley(pos2(end + t.space.tight, y), dot, c.accent.value);
    }
    let label = if p.edited {
        format!("Preset {shown} (edited)")
    } else {
        format!("Preset {shown}")
    };
    response.widget_info(|| WidgetInfo::labeled(Role::Button, true, &label));
    let tip = match (p.from_preset, p.edited) {
        (true, true) => "Edited since the preset was loaded or saved. Click to browse presets",
        _ => "Browse presets",
    };
    if response.on_hover_text(tip).clicked() {
        push(PresetIntent::Browse(!p.browsing));
    }

    if glyph_button(
        ui,
        t,
        next,
        Id::unique("enth.presets.next"),
        glyph::NEXT,
        c.text.primary,
        "Next preset",
    )
    .on_hover_text("The next preset of the browser's list")
    .clicked()
    {
        push(PresetIntent::Step(1));
    }

    let mut row = ui.new_child(
        UiBuilder::new()
            .max_rect(Rect::from_min_max(
                pos2(next.right() + t.space.section, rect.top()),
                rect.max,
            ))
            .layout(Layout::left_to_right(Align::Center)),
    );
    row.spacing_mut().item_spacing.x = t.space.tight;
    let button = |on: bool, text: &str| {
        Button::selectable(on, RichText::new(text).font(t.text.body.clone()))
    };
    if row
        .add(button(p.browsing, "Browse"))
        .on_hover_text("Browse presets")
        .clicked()
    {
        push(PresetIntent::Browse(!p.browsing));
    }
    if row
        .add(button(p.saving, "Save"))
        .on_hover_text("Save the sound as a preset")
        .clicked()
    {
        push(if p.saving {
            PresetIntent::Close
        } else {
            PresetIntent::OpenSave
        });
    }
}
