//! **Owner: lane RUI** (the Recreate UI). The results drawer, in the overview slot (report 01 §2.6, report 03 §2.6).
//!
//! It takes the overview's place once a round has something to show, and the player can switch back to the
//! modulation overview ("Modulation") and return ("Results") at any time:
//!
//! 1. **Submit asks first:** "How close is yours to the target?" 1–5 (`game_rate`), or Skip; the score is held back
//!    until then, so it cannot anchor the answer (ADR-0007's note; docs/session.md, "Time, submits and feedback").
//! 2. **Then the score:** sound % against the pass mark (✓ Passed or ✗ Not yet: a glyph and words, never colour
//!    alone), structure % and combined %, the components in order with a badge (✓ close, ~ a bit off, ✗ far off) and
//!    lane SS's words (`component_text`), the time (or the time to pass), the submits, whether the round is assisted,
//!    the latest hint, the vote ("The score felt: Too low / About right / Too high", `game_vote`), "Disagree…" with
//!    an optional note (`game_dispute`, logged at once in the round log), and the next actions: Try again (the same
//!    challenge from its start), Reveal, New round.
//!
//! **No peeking:** a component's `target` and `answer` readings are never read here, and nor are SS's `jnd`,
//! `floor` and `scale` (for the CLI); only `percent`, `pass_mark`, `passed`, the badges and the words.

use std::sync::Arc;

use egui::text::{LayoutJob, TextWrapping};
use egui::{
    Align, Align2, Area, Button, Color32, FontId, Galley, Id, Layout, Order, Rect, RichText, Role,
    TextEdit, Ui, UiBuilder, Vec2, WidgetInfo, pos2, vec2,
};
use enth_challenge::model::DEFAULT_PASS_MARK;
use enth_challenge::score::sound::component_text;
use enth_challenge::{Agreement, ChallengeCode, Closeness, Component};
use enth_session::game::{HINT_LEVELS, RoundView};

use super::state::{clock, percent};
use super::{GameIntent, Recreate, ViewIntent};
use crate::Theme;
use crate::editor::cards::{Intent, Intents};
use crate::theme::{Score, glyph};

/// The badge a component's closeness draws: a score glyph and ink, never colour alone.
pub fn badge(closeness: Closeness) -> Option<Score> {
    match closeness {
        Closeness::Close => Some(Score::Good),
        Closeness::ABit => Some(Score::Near),
        Closeness::Far => Some(Score::Bad),
        Closeness::Unknown => None,
    }
}

/// A component's words, capitalised ("Attack slightly too fast"): lane SS's `component_text`, which never holds a
/// reading.
pub fn words(c: &Component) -> String {
    let text = component_text(c);
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => text,
    }
}

/// `text` laid out in `width`, at most `rows` lines, cut with "…".
fn wrapped(
    ui: &Ui,
    text: &str,
    font: &FontId,
    color: Color32,
    width: f32,
    rows: usize,
) -> Arc<Galley> {
    let mut job = LayoutJob::simple(text.to_owned(), font.clone(), color, width.max(0.0));
    job.wrap = TextWrapping {
        max_width: width.max(0.0),
        max_rows: rows,
        break_anywhere: false,
        overflow_character: Some('…'),
    };
    ui.painter().layout_job(job)
}

/// A painted label whose accessible name is its whole `text` (galleys painted by hand are not in the
/// accessibility tree); a line cut short shows the whole text as its tooltip.
fn text_at(ui: &mut Ui, rect: Rect, galley: Arc<Galley>, color: Color32, text: &str) {
    let cut = galley.elided;
    let response = ui.allocate_rect(rect, egui::Sense::hover());
    ui.painter().galley(rect.left_top(), galley, color);
    response.widget_info(|| WidgetInfo::labeled(Role::Label, true, text));
    if cut {
        response.on_hover_text(text.to_owned());
    }
}

/// Names a button for screen readers and tests, whatever its text shows.
fn named(response: &egui::Response, label: &str) {
    let enabled = response.enabled();
    response.widget_info(|| WidgetInfo::labeled(Role::Button, enabled, label));
}

/// What the drawer drew past its slot, per axis (zero when it fits).
pub struct DrawerOutput {
    pub overflow: Vec2,
}

/// Draws the drawer into `rect` (the overview slot) from the round `r` holds.
pub(crate) fn show(
    r: &mut Recreate,
    ui: &mut Ui,
    t: &Theme,
    rect: Rect,
    out: &mut Intents,
) -> DrawerOutput {
    let Some(round) = r.round().cloned() else {
        return DrawerOutput {
            overflow: Vec2::ZERO,
        };
    };
    let mut ui = ui.new_child(
        UiBuilder::new()
            .max_rect(rect)
            .id_salt("enth.recreate.results"),
    );
    ui.set_clip_rect(rect.intersect(ui.clip_rect()));
    ui.painter()
        .rect_filled(rect, t.radius.medium, t.color.surface.card);
    let inner = rect.shrink2(vec2(t.space.gap, t.space.tight));
    if r.asking() {
        return rating(&mut ui, t, inner, out);
    }
    let overflow = scored(r, &round, &mut ui, t, inner, out);
    if r.note.is_some() {
        note(r, ui.ctx(), t, rect, out);
    }
    DrawerOutput { overflow }
}

/// The blind rating: "How close is yours to the target?" 1–5, or Skip.
fn rating(ui: &mut Ui, t: &Theme, inner: Rect, out: &mut Intents) -> DrawerOutput {
    let c = &t.color;
    let game = |out: &mut Intents, g| out.push(Intent::Game(g));
    let mut col = ui.new_child(
        UiBuilder::new()
            .max_rect(inner)
            .layout(Layout::top_down(Align::Min)),
    );
    col.spacing_mut().item_spacing = vec2(t.space.tight, t.space.tight);
    col.horizontal(|ui| {
        ui.label(
            RichText::new("How close is yours to the target?")
                .font(t.text.title.clone())
                .color(c.text.primary),
        );
        ui.label(
            RichText::new("Your score shows after you answer, so it cannot sway you.")
                .font(t.text.label.clone())
                .color(c.text.label),
        );
    });
    col.horizontal(|ui| {
        ui.label(
            RichText::new("Nothing like it")
                .font(t.text.label.clone())
                .color(c.text.label),
        );
        for n in 1..=enth_challenge::model::RATING_MAX {
            let b = ui
                .add_sized(
                    [t.size.control_height, t.size.control_height],
                    Button::new(RichText::new(n.to_string()).font(t.text.strong.clone())),
                )
                .on_hover_text(match n {
                    1 => "Nothing like it",
                    2 => "Far off",
                    3 => "Some way off",
                    4 => "Close",
                    _ => "I can't tell them apart",
                });
            named(&b, &format!("Rate {n}"));
            if b.clicked() {
                game(out, GameIntent::Rate(Some(n)));
            }
        }
        ui.label(
            RichText::new("I can't tell them apart")
                .font(t.text.label.clone())
                .color(c.text.label),
        );
        ui.add_space(t.space.section);
        if ui
            .add(Button::new(RichText::new("Skip").font(t.text.body.clone())))
            .on_hover_text("Show the score without rating")
            .clicked()
        {
            game(out, GameIntent::Rate(None));
        }
    });
    DrawerOutput {
        overflow: Vec2::ZERO,
    }
}

/// The score, the components and the feedback column.
fn scored(
    r: &mut Recreate,
    round: &RoundView,
    ui: &mut Ui,
    t: &Theme,
    inner: Rect,
    out: &mut Intents,
) -> Vec2 {
    let s = &t.size.recreate;
    let gap = t.space.pad;
    let score_rect = Rect::from_min_size(inner.min, vec2(s.drawer_score, inner.height()));
    let comp_left = score_rect.right() + gap;
    let comp_rect = Rect::from_min_size(
        pos2(comp_left, inner.top()),
        vec2(2.0 * s.drawer_component + t.space.gap, inner.height()),
    );
    let feedback_rect = Rect::from_min_max(pos2(comp_rect.right() + gap, inner.top()), inner.max);
    let mut overflow = Vec2::ZERO;
    overflow.y = overflow.y.max(score_column(round, ui, t, score_rect));
    overflow.y = overflow.y.max(components(round, ui, t, comp_rect));
    let fb = feedback(r, round, ui, t, feedback_rect, out);
    overflow.y = overflow.y.max(fb.y);
    overflow.x = overflow.x.max(fb.x);
    overflow
}

/// The headline: sound % against the pass mark, structure and combined %, the time. Returns how far it reaches
/// past `rect`'s bottom.
fn score_column(round: &RoundView, ui: &mut Ui, t: &Theme, rect: Rect) -> f32 {
    let c = &t.color;
    let row = t.size.recreate.drawer_row;
    let mut y = rect.top();
    let mut line = |ui: &mut Ui, text: &str, font: &FontId, color: Color32, height: f32| {
        let galley = wrapped(ui, text, font, color, rect.width(), 1);
        text_at(
            ui,
            Rect::from_min_size(pos2(rect.left(), y), vec2(rect.width(), height)),
            galley,
            color,
            text,
        );
        y += height;
    };
    let pass_mark = round
        .report
        .as_ref()
        .map_or(DEFAULT_PASS_MARK, |r| r.pass_mark);
    match round.last {
        Some(last) => {
            let title_h = t.size.control_height;
            line(
                ui,
                &format!("Sound {}", percent(last.score.sound)),
                &t.text.title,
                c.text.primary,
                title_h,
            );
            let (score, verdict) = if last.score.passed {
                (Score::Good, "Passed")
            } else {
                (Score::Bad, "Not yet")
            };
            let ink = c.score.of(score);
            line(
                ui,
                &format!(
                    "{} {verdict} {} pass mark {pass_mark} %",
                    score.glyph(),
                    glyph::BULLET
                ),
                &t.text.strong,
                ink,
                row,
            );
            line(
                ui,
                &format!("Structure {}", percent(last.score.structure)),
                &t.text.body,
                c.text.label,
                row,
            );
            line(
                ui,
                &format!("Combined {}", percent(last.score.combined)),
                &t.text.body,
                c.text.label,
                row,
            );
            let time = match round.time_to_pass_ms {
                Some(ms) => format!("Passed at {}", clock(ms)),
                None => format!("Time {}", clock(round.active_ms)),
            };
            let mut facts = vec![time];
            if round.submits > 1 {
                if let Some(first) = round.first {
                    facts.push(format!("first {}", percent(first.score.sound)));
                }
                facts.push(format!("{} submits", round.submits));
            }
            if round.assisted {
                facts.push("assisted".to_owned());
            }
            line(
                ui,
                &facts.join(&format!(" {} ", glyph::BULLET)),
                &t.text.label,
                c.text.label,
                row,
            );
        }
        None if round.revealed => {
            line(
                ui,
                "Revealed",
                &t.text.title,
                c.text.primary,
                t.size.control_height,
            );
            line(
                ui,
                "The target is loaded: listen and look",
                &t.text.body,
                c.text.label,
                row,
            );
        }
        None if round.pending.scoring => {
            line(
                ui,
                "Scoring…",
                &t.text.title,
                c.text.primary,
                t.size.control_height,
            );
        }
        None => {
            line(
                ui,
                "Not scored yet",
                &t.text.title,
                c.text.primary,
                t.size.control_height,
            );
            line(
                ui,
                "Submit when it sounds close",
                &t.text.body,
                c.text.label,
                row,
            );
        }
    }
    (y - rect.bottom()).max(0.0)
}

/// The components, two columns of rows: a badge and the words. Returns how far they reach past `rect`.
fn components(round: &RoundView, ui: &mut Ui, t: &Theme, rect: Rect) -> f32 {
    let c = &t.color;
    let s = &t.size.recreate;
    let row = s.drawer_row;
    let Some(report) = round.report.as_ref() else {
        if round.submits > 0 {
            let galley = wrapped(
                ui,
                "Submit again to see what is off",
                &t.text.body,
                c.text.label,
                rect.width(),
                1,
            );
            text_at(
                ui,
                Rect::from_min_size(rect.min, vec2(rect.width(), row)),
                galley,
                c.text.label,
                "Submit again to see what is off",
            );
        }
        return 0.0;
    };
    let per_column = (rect.height() / row).floor().max(1.0) as usize;
    let list = &report.sound.components;
    for (i, comp) in list.iter().enumerate() {
        let (column, line) = (i / per_column, i % per_column);
        let left = rect.left() + column as f32 * (s.drawer_component + t.space.gap);
        let at = Rect::from_min_size(
            pos2(left, rect.top() + line as f32 * row),
            vec2(s.drawer_component, row),
        );
        let (mark, ink) = match badge(comp.closeness) {
            Some(score) => (score.glyph(), c.score.of(score)),
            None => (glyph::BULLET, c.text.dimmed),
        };
        let mark_w = t.size.recreate.level_dot;
        ui.painter().text(
            pos2(at.left() + mark_w / 2.0, at.center().y),
            Align2::CENTER_CENTER,
            mark,
            t.text.strong.clone(),
            ink,
        );
        let text = words(comp);
        let text_rect =
            Rect::from_min_max(pos2(at.left() + mark_w + t.space.tight, at.top()), at.max);
        let galley = wrapped(
            ui,
            &text,
            &t.text.body,
            c.text.primary,
            text_rect.width(),
            1,
        );
        let label = format!("{mark} {text}");
        let cut = galley.elided;
        let response = ui.allocate_rect(at, egui::Sense::hover());
        ui.painter().galley(
            pos2(
                text_rect.left(),
                text_rect.center().y - galley.size().y / 2.0,
            ),
            galley,
            c.text.primary,
        );
        response.widget_info(|| WidgetInfo::labeled(Role::Label, true, &label));
        if cut {
            response.on_hover_text(text);
        }
    }
    let columns = list.len().div_ceil(per_column);
    if columns > 2 {
        ((list.len() - 2 * per_column) as f32 * row).max(0.0)
    } else {
        0.0
    }
}

/// The hint, the vote with "Disagree…", and the next actions. Returns how far they reach past `rect`.
fn feedback(
    r: &mut Recreate,
    round: &RoundView,
    ui: &mut Ui,
    t: &Theme,
    rect: Rect,
    out: &mut Intents,
) -> Vec2 {
    let c = &t.color;
    let game = |out: &mut Intents, g| out.push(Intent::Game(g));
    let text = |s: &str| RichText::new(s).font(t.text.body.clone());
    let row = t.size.recreate.drawer_row;
    let h = t.size.control_height;
    let mut y = rect.top();
    let mut wide: f32 = 0.0;
    // The hint: up to two lines.
    let hint = if round.pending.hinting {
        Some("Thinking of a hint…".to_owned())
    } else {
        round
            .hint
            .as_ref()
            .map(|h| format!("Hint {}/{HINT_LEVELS}: {}", h.level, h.text))
    };
    let scored = round.submits > 0;
    // Room for the hint: what the vote and action rows leave.
    let rows_left =
        ((rect.height() - h - if scored { h + t.space.tight } else { 0.0 } - t.space.tight) / row)
            .floor()
            .max(1.0) as usize;
    if let Some(hint) = hint {
        let galley = wrapped(
            ui,
            &hint,
            &t.text.body,
            c.text.primary,
            rect.width(),
            rows_left.min(2),
        );
        let size = galley.size();
        text_at(
            ui,
            Rect::from_min_size(pos2(rect.left(), y), vec2(rect.width(), size.y)),
            galley,
            c.text.primary,
            &hint,
        );
        y += size.y + t.space.tight;
    }
    let mut row_ui = |y: f32| {
        ui.new_child(
            UiBuilder::new()
                .max_rect(Rect::from_min_size(
                    pos2(rect.left(), y),
                    vec2(rect.width(), h),
                ))
                .layout(Layout::left_to_right(Align::Center)),
        )
    };
    // The vote and the dispute, once there is a score.
    if scored {
        let mut v = row_ui(y);
        v.spacing_mut().item_spacing.x = t.space.tight;
        v.label(
            RichText::new("Score felt")
                .font(t.text.label.clone())
                .color(c.text.label),
        )
        .on_hover_text("How did the score feel against what you heard?");
        for (agreement, words) in [
            (Agreement::TooLow, "Too low"),
            (Agreement::AboutRight, "About right"),
            (Agreement::TooHigh, "Too high"),
        ] {
            let on = round.agreement == Some(agreement);
            if v.add(Button::new(text(words)).selected(on))
                .on_hover_text("Your vote goes to the round log, beside the score")
                .clicked()
            {
                game(out, GameIntent::Vote(agreement));
            }
        }
        let disagree = v
            .add(Button::new(text("Disagree…")).selected(r.note.is_some()))
            .on_hover_text("Tell us what the score got wrong: it goes to the round log at once, with both patches");
        if disagree.clicked() {
            let open = if r.note.is_some() {
                None
            } else {
                Some(String::new())
            };
            game(out, GameIntent::View(ViewIntent::Note(open)));
        }
        if let Some(thanks) = &r.thanked {
            v.label(
                RichText::new(glyph::GOOD)
                    .font(t.text.label.clone())
                    .color(c.text.label),
            )
            .on_hover_text(thanks.as_str());
        }
        wide = wide.max(v.min_rect().width());
        y += h + t.space.tight;
    }
    // The next actions, and the switch back to the modulation overview.
    let mut a = row_ui(y);
    a.spacing_mut().item_spacing.x = t.space.tight;
    let code = ChallengeCode::decode(&round.code).ok();
    let again = a
        .add_enabled(code.is_some(), Button::new(text("Try again")))
        .on_hover_text(
            "This challenge again, from its start patch (a new round with the same code)",
        );
    if again.clicked()
        && let Some(code) = code
    {
        game(out, GameIntent::Code(code));
    }
    if round.phase.is_open() {
        let reveal = a
            .add_enabled(round.ab.target_ready, Button::new(text("Reveal")))
            .on_hover_text("Give up: load the target (the round counts as assisted)");
        named(&reveal, "Reveal the target");
        if reveal.clicked() {
            game(out, GameIntent::Reveal);
        }
    }
    let spec = code.map_or_else(|| r.settings().spec(0), |c| *c.spec());
    let next = a
        .add(Button::new(
            RichText::new("New round").font(t.text.strong.clone()),
        ))
        .on_hover_text("Another round with the same settings and a new sound");
    if next.clicked() {
        game(out, GameIntent::New(spec));
    }
    let actions = a.min_rect().width();
    let modulation = a
        .with_layout(Layout::right_to_left(Align::Center), |ui| {
            let b = ui
                .add(
                    Button::new(RichText::new("Modulation").font(t.text.label.clone()))
                        .frame(false),
                )
                .on_hover_text(
                    "Show the source tiles and routes here instead; Results brings this back",
                );
            named(&b, "Show modulation");
            if b.clicked() {
                game(out, GameIntent::View(ViewIntent::Results(false)));
            }
            b.rect.width()
        })
        .inner;
    wide = wide.max(actions + t.space.gap + modulation);
    y += h;
    vec2((wide - rect.width()).max(0.0), (y - rect.bottom()).max(0.0))
}

/// The "Disagree…" note: a small card over the editor, above the drawer. Send logs the dispute at once.
fn note(r: &mut Recreate, ctx: &egui::Context, t: &Theme, drawer: Rect, out: &mut Intents) {
    let c = &t.color;
    let s = &t.size.recreate;
    let game = |out: &mut Intents, g| out.push(Intent::Game(g));
    let width = s.dispute_width;
    let at = pos2(drawer.right() - width, drawer.top() - t.space.gap);
    let Some(text) = r.note.as_mut() else {
        return;
    };
    let mut send = None;
    let mut cancel = false;
    Area::new(Id::unique("enth.recreate.dispute"))
        .order(Order::Foreground)
        .fixed_pos(at)
        .pivot(Align2::LEFT_BOTTOM)
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(c.surface.raised)
                .corner_radius(t.radius.medium)
                .inner_margin(t.space.gap)
                .show(ui, |ui| {
                    ui.set_width(width - 2.0 * t.space.gap);
                    ui.spacing_mut().item_spacing = vec2(t.space.tight, t.space.tight);
                    ui.label(
                        RichText::new("What did the score get wrong? (optional)")
                            .font(t.text.strong.clone())
                            .color(c.text.primary),
                    );
                    ui.label(
                        RichText::new(
                            "It goes to your round log with the code, both patches and the scores.",
                        )
                        .font(t.text.label.clone())
                        .color(c.text.label),
                    );
                    let field = ui.add(
                        TextEdit::multiline(text)
                            .desired_rows(s.dispute_rows as usize)
                            .desired_width(f32::INFINITY)
                            .hint_text("It sounded closer than that")
                            .font(t.text.body.clone()),
                    );
                    named_input(&field, "Dispute note");
                    ui.horizontal(|ui| {
                        if ui
                            .add(Button::new(
                                RichText::new("Send").font(t.text.strong.clone()),
                            ))
                            .clicked()
                        {
                            send = Some(text.clone());
                        }
                        if ui
                            .add(Button::new(
                                RichText::new("Cancel").font(t.text.body.clone()),
                            ))
                            .clicked()
                        {
                            cancel = true;
                        }
                    });
                });
        });
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        cancel = true;
    }
    if let Some(note) = send {
        game(out, GameIntent::Dispute(note.trim().to_owned()));
    } else if cancel {
        game(out, GameIntent::View(ViewIntent::Note(None)));
    }
}

fn named_input(response: &egui::Response, label: &str) {
    response.widget_info(|| WidgetInfo::labeled(Role::MultilineTextInput, true, label));
}

/// "Results", over the overview's top right corner while the player has the modulation overview up instead of the
/// drawer.
pub(crate) fn reopen(ui: &mut Ui, t: &Theme, rect: Rect, out: &mut Intents) {
    let size = vec2(t.size.recreate.drawer_score / 2.0, t.size.control_height);
    let at = Rect::from_min_size(
        pos2(
            rect.right() - size.x - t.space.tight,
            rect.top() + t.space.tight,
        ),
        size,
    );
    let mut b = ui.new_child(
        UiBuilder::new()
            .max_rect(at)
            .layout(Layout::right_to_left(Align::Center)),
    );
    let response = b
        .add(Button::new(
            RichText::new("Results").font(t.text.body.clone()),
        ))
        .on_hover_text("Back to the round's results");
    named(&response, "Show results");
    if response.clicked() {
        out.push(Intent::Game(GameIntent::View(ViewIntent::Results(true))));
    }
}
