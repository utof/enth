//! **Owner: lane DUI** (the duel card, feature `duel`; lane SKD wrote its minimal card first, F2d). Pre-declared by
//! lane S0e (R4/R5).
//!
//! The duel card's entry points: [`show`] draws the card in its column at the window's right edge (`size.duel.pane`,
//! under the layout's top row, as the Create panel's; the layout places the rest beside it), and [`opener`] draws
//! **Duel** in the overview region's bottom-left corner while the card is closed. Both draw nothing until a shell says
//! duels are available ([`Duel::available`]), so a build, a kittest frame or a dev window that never says so draws
//! exactly as before.
//!
//! The card, top to bottom on `surface.card`, **the controls first, in rows that never move** (the REAPER `duel`
//! scenario clicks them by position; `tests/duel.rs` pins the points; lane SKD's minimal card set them, lane DUI kept
//! them), then the text:
//!
//! 1. **Duel** and **×**;
//! 2. **Share result**, **Copy message**, **Copy answer** ([`super::share`]): the codes are made by the shell; a copy
//!    writes the clipboard inside the click and nowhere else;
//! 3. the paste box (the platform's Paste puts a whole chat message in it) and **Paste** at its right
//!    ([`super::paste::button`]: the one click that reads the clipboard; what it brings is read at once);
//! 4. **Read**, **Verify**, **Use theirs**, **▶ Theirs** / **■ Stop** ([`super::answer`]);
//! 5. **Play** (the duel's challenge, through the Recreate bar's code start) and **Close duel**;
//! 6. what the shell or the session said; **You**: your result and the codes (and, over 64 symbols, that an enth from
//!    before duels reads the answer as "bad length"); **Your friend**: their optional name (kept on this computer
//!    only), what the last Read found, the two results **side by side**, their scorecard **as reported**, the sealed
//!    answer, the verification **checked on your computer** ([`super::verify`]); **History**: your duels on this
//!    computer, each with **Open** ([`super::history`]); and the About line, said once.
//!
//! Every control has an accessible name, so kittest drives it by label. It asks for no frames of its own: the shell's
//! posts wake the editor, and nothing in it animates.

use egui::{
    Align, Button, Label, Layout, Rect, Response, RichText, Role, ScrollArea, TextEdit, Ui,
    UiBuilder, WidgetInfo, pos2, vec2,
};

use super::state::{Duel, DuelIntent};
use crate::Theme;
use crate::editor::cards::{Intent, Intents};
use crate::theme::glyph;

/// The About line, said once (docs/duels.md, "Wording").
pub const ABOUT: &str = "Duel codes are for friends. A code proves what a patch scores, not who made it or how long it \
                         took. enth's own tools can solve any challenge.";

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
    intent: DuelIntent,
    out: &mut Intents,
) -> Response {
    let response = ui.add_enabled(
        enabled,
        Button::new(RichText::new(text).font(t.text.body.clone())),
    );
    named(&response, label);
    if response.clicked() {
        out.push(Intent::Duel(intent));
    }
    response
}

/// A wrapped line of `text` in the label font, dimmed or not; its accessible name is the whole text.
pub(crate) fn line(ui: &mut Ui, t: &Theme, text: &str, dimmed: bool) {
    let color = if dimmed {
        t.color.text.dimmed
    } else {
        t.color.text.primary
    };
    let shown =
        ui.add(Label::new(RichText::new(text).font(t.text.label.clone()).color(color)).wrap());
    labeled(&shown, Role::Label, text);
}

/// A refusal, in the bad-score ink with its glyph (never colour alone).
pub(crate) fn refusal(ui: &mut Ui, t: &Theme, text: &str) {
    let text = format!("{} {text}", glyph::BAD);
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

/// A section's heading.
fn heading(ui: &mut Ui, t: &Theme, text: &str) {
    ui.add_space(t.space.tight);
    ui.label(
        RichText::new(text)
            .font(t.text.strong.clone())
            .color(t.color.text.primary),
    );
}

/// **Duel**, in the bottom-left corner of `rect` (the overview region), while duels are available and the card is
/// closed. The card has no header entry of its own: it belongs with a round's results and a paste box.
pub fn opener(duel: &Duel, ui: &mut Ui, t: &Theme, rect: Rect, out: &mut Intents) {
    if !duel.available() || duel.card_open() {
        return;
    }
    let size = vec2(t.size.duel.opener, t.size.control_height);
    let at = Rect::from_min_size(
        pos2(
            rect.left() + t.space.tight,
            rect.bottom() - size.y - t.space.tight,
        ),
        size,
    );
    let mut b = ui.new_child(
        UiBuilder::new()
            .max_rect(at)
            .layout(Layout::left_to_right(Align::Center)),
    );
    button(
        &mut b,
        t,
        "Duel",
        "Open duel card",
        true,
        DuelIntent::Toggle,
        out,
    )
    .on_hover_text("Share a round's result with a friend, or check theirs");
}

/// Draws the card into `rect` from `duel`. Returns how far its content reached past `rect` (it scrolls; the frame report
/// lists it as `duel`).
pub fn show(duel: &mut Duel, ui: &mut Ui, t: &Theme, rect: Rect, out: &mut Intents) -> egui::Vec2 {
    let mut ui = ui.new_child(
        UiBuilder::new()
            .max_rect(rect)
            .layout(Layout::top_down(Align::Min))
            .id_salt("enth.duel.card"),
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
        .id_salt("enth.duel.scroll")
        .auto_shrink([false, false])
        .show(&mut col, |ui| {
            ui.spacing_mut().item_spacing = vec2(t.space.gap, t.space.tight);
            body(duel, ui, t, out);
        });
    let over = shown.content_size - shown.inner_rect.size();
    vec2(over.x.max(0.0), over.y.max(0.0))
}

fn body(duel: &mut Duel, ui: &mut Ui, t: &Theme, out: &mut Intents) {
    // 1. The title and ×.
    ui.horizontal(|ui| {
        ui.label(
            RichText::new("Duel")
                .font(t.text.title.clone())
                .color(t.color.text.primary),
        );
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            button(
                ui,
                t,
                glyph::CLOSE,
                "Close duel card",
                true,
                DuelIntent::Toggle,
                out,
            )
            .on_hover_text("Close the card (the duel stays open)");
        });
    });
    // 2. Share and copy.
    ui.horizontal(|ui| super::share::buttons(duel, ui, t, out));
    // 3. The paste box: the platform's Paste puts a whole chat message here, and so does **Paste** at its right (the
    //    one click that reads the clipboard); nothing reads it otherwise.
    super::paste::arrived(duel, ui, out);
    let mut text = duel.paste_text().to_owned();
    ui.horizontal_top(|ui| {
        let width = ui.available_width() - t.size.duel.paste_button - ui.spacing().item_spacing.x;
        let edit = ui.add_sized(
            vec2(width, t.size.duel.paste_height),
            TextEdit::multiline(&mut text)
                .id(super::paste::box_id())
                .font(t.text.label.clone())
                .hint_text("Paste your friend's whole message here (Ctrl+V)"),
        );
        labeled(&edit, Role::TextInput, "Paste box");
        super::paste::button(ui, t, out);
    });
    if text != duel.paste_text() {
        out.push(Intent::Duel(DuelIntent::PasteText(text)));
    }
    // 4. Read, then their answer's buttons.
    ui.horizontal(|ui| {
        button(
            ui,
            t,
            "Read",
            "Read pasted codes",
            !duel.paste_text().trim().is_empty(),
            DuelIntent::Read,
            out,
        )
        .on_hover_text("Read every code in the box: a challenge, a scorecard, an answer");
        super::answer::buttons(duel, ui, t, out);
    });
    // 5. Play the duel's challenge, close the duel.
    ui.horizontal(|ui| {
        play(duel, ui, t, out);
        button(
            ui,
            t,
            "Close duel",
            "Close duel",
            duel.view().is_some(),
            DuelIntent::Close,
            out,
        )
        .on_hover_text("Forget this duel (your round and your patch stay)");
    });
    // 6. What was said, then the duel.
    if let Some(said) = duel.shell().said.clone() {
        refusal(ui, t, &said);
    }
    if let Some(said) = duel.said().map(str::to_owned) {
        refusal(ui, t, &said);
    }
    if duel.paste_pending() {
        line(
            ui,
            t,
            "Reading the clipboard… If nothing arrives, click the box and press Ctrl+V.",
            true,
        );
    }
    heading(ui, t, "You");
    super::share::text(duel, ui, t);
    heading(ui, t, "Your friend");
    if duel.view().is_some() {
        name(duel, ui, t, out);
    }
    for read in duel.read_lines().to_vec() {
        line(ui, t, &read, true);
    }
    super::verify::text(duel, ui, t);
    heading(ui, t, "History");
    super::history::list(duel, ui, t, out);
    ui.add_space(t.space.tight);
    line(ui, t, ABOUT, true);
}

/// Their name, typed by you for the open duel (optional; kept only in the history on this computer, never in a
/// code): the shell hears of it when the field is left.
fn name(duel: &mut Duel, ui: &mut Ui, t: &Theme, out: &mut Intents) {
    let kept = duel.shell().name.clone();
    let edit = ui.add_sized(
        vec2(ui.available_width(), t.size.control_height),
        TextEdit::singleline(duel.name_buffer())
            .char_limit(super::state::NAME_CHARS)
            .font(t.text.label.clone())
            .hint_text("Their name (optional, kept on this computer)"),
    );
    labeled(&edit, Role::TextInput, "Friend's name");
    if edit.lost_focus() && duel.name_text().trim() != kept.trim() {
        out.push(Intent::Duel(DuelIntent::Name(duel.name_text().to_owned())));
    }
}

/// **Play**: start a Recreate round on the duel's challenge, through the Recreate bar's code start (feature
/// `recreate`), while the player has not played it.
#[cfg(feature = "recreate")]
fn play(duel: &Duel, ui: &mut Ui, t: &Theme, out: &mut Intents) {
    let code = duel.to_play();
    let response = ui.add_enabled(
        code.is_some(),
        Button::new(RichText::new("Play").font(t.text.body.clone())),
    );
    named(&response, "Play this challenge");
    let response = response.on_hover_text("Start a round on the duel's challenge");
    if response.clicked()
        && let Some(code) = code
    {
        out.push(Intent::Game(crate::editor::recreate::GameIntent::Code(
            code,
        )));
    }
}

/// Without the Recreate bar there is no round to start here.
#[cfg(not(feature = "recreate"))]
fn play(_duel: &Duel, _ui: &mut Ui, _t: &Theme, _out: &mut Intents) {}
