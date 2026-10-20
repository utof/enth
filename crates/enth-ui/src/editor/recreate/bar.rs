//! **Owner: lane RUI** (the Recreate UI). The challenge bar in the header row (report 01 §2.6, report 03 §2.6).
//!
//! It draws through the header's Recreate hook and nothing at all until a shell plays rounds
//! ([`Editor::set_round`](crate::editor::Editor::set_round)), so kittest, the dev window and every snapshot that
//! never hands over a round are unchanged.
//!
//! | When | The header row shows |
//! | --- | --- |
//! | Play (no round) | **Recreate**, left of "+ Add": enters Recreate mode |
//! | Recreate mode, before a round | the level (five dots, a click picks one), a **code** field ("Paste a code"), **Start**, **Settings…** and × (back to Play); the Presets group steps aside |
//! | During a round | the level, **Target ▶** / **Mine ▶** (a second click turns a side off) / **A/B** (flips at the same position), the timer, **Submit**, **Hint** with the levels used (n/4), **Reveal** and **End** (restore my patch, or keep this one) |
//!
//! No live score: Submit asks the blind rating in the results drawer and shows the score after it; the readout
//! (phase, scores, time, a refusal's hint) is the status line's right end ([`Recreate::readout`]). Nothing scrolls at
//! 1200 × 800 (`tests/recreate.rs`).

use egui::{
    Align, Button, Layout, Rect, Response, RichText, Role, TextEdit, Ui, UiBuilder, WidgetInfo,
    pos2,
};
use enth_session::game::{EndChoice, HINT_LEVELS, Listen, Phase};

use super::settings::LEVELS;
use crate::Theme;
use crate::editor::cards::{Intent, Intents};
use crate::editor::regions::HeaderView;
use crate::theme::glyph;

pub use super::state::{BarView, GameIntent, Recreate, ViewIntent, apply_game};

/// Names `response` for screen readers and tests (`label`), whatever its text shows.
fn named(response: &Response, role: Role, label: &str) {
    let enabled = response.enabled();
    response.widget_info(|| WidgetInfo::labeled(role, enabled, label));
}

/// The level's dots, filled up to `level`.
fn dots(level: u8) -> String {
    (1..=LEVELS)
        .map(|n| {
            if n <= level {
                glyph::DOT_ON
            } else {
                glyph::DOT_OFF
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The header's Recreate hook (feature `recreate`): called by the header with its right-hand row (laid out right to
/// left, after "+ Add") and its inner rectangle. In Play it adds **Recreate** to that row; in Recreate mode it draws
/// the bar left to right from the wordmark to the row's leftmost control, where the Presets group was.
pub(crate) fn header(ui: &mut Ui, t: &Theme, rect: Rect, view: &HeaderView, out: &mut Intents) {
    let bar = &view.recreate;
    if !bar.available {
        return;
    }
    let game = |out: &mut Intents, g| out.push(Intent::Game(g));
    let text = |s: &str| RichText::new(s).font(t.text.body.clone());
    ui.add_space(t.space.section);
    if !bar.mode {
        if ui
            .add(Button::new(text("Recreate")))
            .on_hover_text("A Recreate round: the synth makes a sound at the level you pick, and you rebuild it by ear")
            .clicked()
        {
            game(out, GameIntent::View(ViewIntent::Mode(true)));
        }
        return;
    }
    // Where the Presets group was: from the wordmark to the right-hand row's leftmost control.
    let wordmark = ui
        .painter()
        .layout_no_wrap(
            "enth".to_owned(),
            t.text.title.clone(),
            t.color.text.primary,
        )
        .size()
        .x;
    let right = ui.available_rect_before_wrap().right() - t.space.section;
    let left = rect.left() + wordmark + t.space.section;
    let span = Rect::from_min_max(pos2(left, rect.top()), pos2(right.max(left), rect.bottom()));
    let mut row = ui.new_child(
        UiBuilder::new()
            .max_rect(span)
            .id_salt("enth.recreate.bar")
            .layout(Layout::left_to_right(Align::Center)),
    );
    row.set_clip_rect(span.intersect(ui.clip_rect()));
    row.spacing_mut().item_spacing.x = t.space.tight;
    if bar.live {
        live(&mut row, t, bar, out);
    } else {
        before(&mut row, t, bar, out);
    }
}

/// Before a round: the level, the code field, Start, Settings… and ×.
fn before(ui: &mut Ui, t: &Theme, bar: &BarView, out: &mut Intents) {
    let c = &t.color;
    let game = |out: &mut Intents, g| out.push(Intent::Game(g));
    let text = |s: &str| RichText::new(s).font(t.text.body.clone());
    ui.label(
        RichText::new("Level")
            .font(t.text.label.clone())
            .color(c.text.label),
    );
    for level in 1..=LEVELS {
        let ink = if bar.custom {
            c.text.label
        } else {
            c.accent.value
        };
        let glyph = if level <= bar.level {
            glyph::DOT_ON
        } else {
            glyph::DOT_OFF
        };
        let dot = ui
            .add_sized(
                [t.size.recreate.level_dot, t.size.control_height],
                Button::new(RichText::new(glyph).font(t.text.body.clone()).color(ink)).frame(false),
            )
            .on_hover_text(format!("Level {level}: {}", level_words(level)));
        named(&dot, Role::Button, &format!("Level {level}"));
        if dot.clicked() {
            game(out, GameIntent::View(ViewIntent::Level(level)));
        }
    }
    if bar.custom {
        ui.label(
            RichText::new("custom")
                .font(t.text.label.clone())
                .color(c.text.label),
        );
    }
    ui.add_space(t.space.gap);
    let mut code = bar.code_text.clone();
    let field = ui.add(
        TextEdit::singleline(&mut code)
            .hint_text("Paste a code")
            .desired_width(t.size.recreate.code_field)
            .font(t.text.value.clone()),
    );
    let field =
        field.on_hover_text("A challenge code from a friend: the same target on every machine");
    named(&field, Role::TextInput, "Challenge code");
    if field.changed() {
        game(out, GameIntent::View(ViewIntent::CodeText(code.clone())));
    }
    let enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
    let start = ui
        .add(Button::new(text("Start")))
        .on_hover_text(if code.trim().is_empty() {
            format!(
                "A round at level {}{}",
                bar.level,
                if bar.custom {
                    " with your settings"
                } else {
                    ""
                }
            )
        } else {
            "A round from the code".to_owned()
        });
    if start.clicked() || enter {
        game(out, GameIntent::View(ViewIntent::Start));
    }
    if ui
        .add(Button::new(text("Settings…")))
        .on_hover_text("Difficulty, what changes, how musical, the template and assists")
        .clicked()
    {
        game(out, GameIntent::View(ViewIntent::Settings(true)));
    }
    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
        let leave = ui
            .add(Button::new(text(glyph::CLOSE)).frame(false))
            .on_hover_text("Leave Recreate: back to the presets");
        named(&leave, Role::Button, "Leave Recreate");
        if leave.clicked() {
            game(out, GameIntent::View(ViewIntent::Mode(false)));
        }
    });
}

/// What a level randomizes, in words (lane RG's levels).
fn level_words(level: u8) -> &'static str {
    match level {
        1 => "the oscillators",
        2 => "adds the filter",
        3 => "adds the envelopes",
        4 => "adds modulation",
        _ => "adds effects and structure",
    }
}

/// During a round: the level, Target ▶ / Mine ▶ / A/B, the timer, Submit, Hint, Reveal and End.
fn live(ui: &mut Ui, t: &Theme, bar: &BarView, out: &mut Intents) {
    let c = &t.color;
    let game = |out: &mut Intents, g| out.push(Intent::Game(g));
    let text = |s: &str| RichText::new(s).font(t.text.body.clone());
    let level = ui
        .label(
            RichText::new(dots(bar.level))
                .font(t.text.label.clone())
                .color(c.accent.value),
        )
        .on_hover_text(format!("Level {}: {}", bar.level, level_words(bar.level)));
    named(
        &level,
        Role::Label,
        &format!("Level {} of {LEVELS}", bar.level),
    );
    ui.add_space(t.space.gap);

    let on = |l| bar.listen == l;
    let flip = |l| if on(l) { Listen::Off } else { l };
    let target = format!("Target {}", glyph::PLAY);
    if ui
        .add_enabled(
            bar.ready,
            Button::new(text(&target)).selected(on(Listen::Target)),
        )
        .on_hover_text("Hear the target: a loop of the phrase it is scored on")
        .clicked()
    {
        game(out, GameIntent::Listen(flip(Listen::Target)));
    }
    let mine = format!("Mine {}", glyph::PLAY);
    if ui
        .add_enabled(
            bar.ready,
            Button::new(text(&mine)).selected(on(Listen::Mine)),
        )
        .on_hover_text("Hear yours: the same phrase, live, in step with the target's loop")
        .clicked()
    {
        game(out, GameIntent::Listen(flip(Listen::Mine)));
    }
    if ui
        .add_enabled(bar.ready, Button::new(text("A/B")))
        .on_hover_text("Flip between the target and yours at the same place in the phrase")
        .clicked()
    {
        game(out, GameIntent::Listen(bar.listen.flipped()));
    }
    ui.add_space(t.space.gap);
    let time = ui
        .add_sized(
            [t.size.recreate.timer.max(0.0), t.size.control_height],
            egui::Label::new(
                RichText::new(&bar.time)
                    .font(t.text.value.clone())
                    .color(c.text.primary),
            ),
        )
        .on_hover_text(format!(
            "Time spent on this round (it pauses while the window is closed). Code {}",
            bar.code
        ));
    named(&time, Role::Label, &format!("Time {}", bar.time));
    ui.add_space(t.space.gap);

    if ui
        .add_enabled(
            bar.can_submit,
            Button::new(RichText::new("Submit").font(t.text.strong.clone())),
        )
        .on_hover_text(
            "Score your patch: the first submit is the one recorded; submit again any time",
        )
        .clicked()
    {
        game(out, GameIntent::Submit);
    }
    let hint = ui
        .add_enabled(bar.can_hint, Button::new(text("Hint")))
        .on_hover_text("The next hint, from the aspect that is most off to a value range (the round counts as assisted)");
    if hint.clicked() {
        game(out, GameIntent::Hint);
    }
    if bar.hints > 0 {
        let used = ui.label(
            RichText::new(format!("{}/{HINT_LEVELS}", bar.hints))
                .font(t.text.label.clone())
                .color(c.text.label),
        );
        named(
            &used,
            Role::Label,
            &format!("Hint level {} of {HINT_LEVELS}", bar.hints),
        );
    }
    if ui
        .add_enabled(bar.can_reveal, Button::new(text("Reveal")))
        .on_hover_text("Give up: load the target (the round counts as assisted)")
        .clicked()
    {
        game(out, GameIntent::Reveal);
    }
    let broken = matches!(bar.phase, Phase::Broken | Phase::Unknown);
    ui.menu_button(text("End"), |ui| {
        if ui
            .add_enabled(true, Button::new(text("End and restore my patch")))
            .on_hover_text("Your own patch and its undo history come back")
            .clicked()
        {
            game(out, GameIntent::End(EndChoice::Restore));
            ui.close();
        }
        if ui
            .add_enabled(!broken, Button::new(text("End and keep this patch")))
            .on_hover_text("Keep the round's patch (and its undo history) as your own")
            .clicked()
        {
            game(out, GameIntent::End(EndChoice::Keep));
            ui.close();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dots_fill_up_to_the_level() {
        assert_eq!(dots(1), "● ○ ○ ○ ○");
        assert_eq!(dots(5), "● ● ● ● ●");
    }
}
