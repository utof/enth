//! **Owner: lane LUI** (the Learn view, feature `learn`). Pre-declared by lane S0l (R2); lane SKL's minimal panel
//! (F2l) is replaced by this pane.
//!
//! **The lesson pane** fills the `lesson` region (the built-in layout `learn`, `layout/learn.json`): with no lesson, or
//! when asked, the lesson browser ([`super::browser`]); in a lesson, the step. It draws `enth-learn`'s markdown block
//! AST with design tokens only and never sees markdown (docs/learn.md, "The markdown block AST"):
//!
//! | AST | Drawn as |
//! | --- | --- |
//! | `Heading` 1 / 2–3 | `text.title` / `text.strong` |
//! | `Paragraph`, `List` (bulleted, numbered, nested) | flowing runs in `text.body`; `•` or `n.` markers, nested lists indented by `space.section` |
//! | `Strong`, `Emphasis`, `Code` | `text.strong`; italics; `text.value` on `surface.recessed` (a chip) |
//! | link `enth:<selector>` | the value accent; **hovering it outlines its control** on the cards (the overlay's highlight) |
//! | link `enth:play/n` | the value accent; a click plays example (or option) n |
//! | link `https:` | the value accent; a click opens the browser (egui's `OpenUrl`; R2 has no network code) |
//! | widget `play n` | a ▶ button for example n |
//! | widget `control <param>` | the live control in the text (a knob under its own id, so it never clashes with the card's); a module, route or macro shows as a chip that outlines it on hover |
//! | widget `scope` | the live scope (the telemetry the output region draws) |
//! | widget `shape <module>` | an envelope's stages or an LFO's wave, from the live patch |
//!
//! **Where things stand.** The title row (the lesson's title, **Lessons** for the browser, **Hide** to collapse the
//! pane into the header's "Lesson n/m" entry) and the progress row (a dot per step, "Step n of m" and the step's state)
//! sit at the top; the step's text scrolls in the middle; **the controls are anchored to the bottom**, so they stand at
//! the same place whatever the text's length (the REAPER `lesson` scenario clicks them by position;
//! `tests/learn_panel.rs` pins the points). From the bottom up: ◀ Back, **End lesson** (a menu: restore or keep) and
//! Next ▶ (Finish ▶ on the last step); the step's own controls (Listen: ▶ per example, ■ Stop and **A/B**, which flips
//! between the last two examples at the same position; Choose: the examples, then a button per answer; Do: **Hint**
//! and **Show me**); above them the verdict, the latest hint and the last refusal. A Recreate step is played with the
//! Recreate UI unchanged (the challenge bar in the header, the results drawer in the overview slot); the pane keeps
//! its text and the way on. Every control has an accessible name, so kittest drives it by label. The pane asks for no
//! frames of its own.

use egui::{
    Align, Button, CursorIcon, Label, Layout, Rect, Response, RichText, Role, ScrollArea, Sense,
    Ui, UiBuilder, Vec2, WidgetInfo, vec2,
};
use enth_learn::markdown::{self, Block, Doc, Inline, LinkTarget, Widget};
use enth_learn::{Example, Selector, StepKind, StepPhase};
use enth_patch::catalogue::LfoShape;
use enth_patch::{ModuleKind, ParamAddr, Patch};
use enth_session::game::EndChoice;
use enth_session::learn::LearnView;

use super::state::{Learn, LearnIntent};
use crate::Theme;
use crate::editor::cards::format::value_text;
use crate::editor::cards::{
    CardKind, Control, Grip, Intent, Intents, envelope_stages, ids, present, shapes,
};
use crate::editor::live::Live;
use crate::theme::glyph;
use crate::widgets::{Display, Emphasis as KnobEmphasis, Ink, Knob, Trace};

/// What the pane did this frame, besides its intents.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct PaneOutput {
    /// How far its scrolling text reached past its slot (the frame report lists it as `learn`).
    pub overflow: Vec2,
    /// The `enth:` link or `control` widget under the pointer: its control is outlined on the cards.
    pub pointing: Option<Selector>,
}

/// Names a widget for screen readers and tests, whatever its text shows.
pub(crate) fn named(response: &Response, role: Role, label: &str) {
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
    intent: LearnIntent,
    out: &mut Intents,
) -> Response {
    let response = ui.add_enabled(
        enabled,
        Button::new(RichText::new(text).font(t.text.body.clone())),
    );
    named(&response, Role::Button, label);
    if response.clicked() {
        out.push(Intent::Learn(intent));
    }
    response
}

/// A line of wrapped text in `font` and `ink`.
fn line(ui: &mut Ui, text: &str, font: egui::FontId, ink: egui::Color32) {
    ui.add(Label::new(RichText::new(text).font(font).color(ink)).wrap());
}

/// The width a button with `text` takes in the body font.
fn button_width(ui: &Ui, t: &Theme, text: &str) -> f32 {
    let galley =
        ui.painter()
            .layout_no_wrap(text.to_owned(), t.text.body.clone(), t.color.text.primary);
    galley.size().x + 2.0 * ui.spacing().button_padding.x
}

/// Splits buttons (by their texts) into rows that fit `width`, greedily, so the bottom-anchored controls know their
/// rows before drawing them bottom-up.
fn rows(ui: &Ui, t: &Theme, texts: &[String], width: f32) -> Vec<Vec<usize>> {
    let gap = ui.spacing().item_spacing.x;
    let mut out: Vec<Vec<usize>> = Vec::new();
    let mut used = 0.0;
    for (i, text) in texts.iter().enumerate() {
        let w = button_width(ui, t, text);
        match out.last_mut() {
            Some(row) if used + gap + w <= width => {
                row.push(i);
                used += gap + w;
            }
            _ => {
                out.push(vec![i]);
                used = w;
            }
        }
    }
    out
}

/// Draws the pane into `rect` (the `lesson` region) from `learn`, the live patch and the telemetry.
pub(crate) fn show(
    learn: &Learn,
    ui: &mut Ui,
    t: &Theme,
    rect: Rect,
    patch: &Patch,
    live: &Live,
    out: &mut Intents,
) -> PaneOutput {
    let mut ui = ui.new_child(
        UiBuilder::new()
            .max_rect(rect)
            .layout(Layout::top_down(Align::Min))
            .id_salt("enth.learn.pane"),
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
    col.spacing_mut().item_spacing = vec2(t.space.tight, t.space.tight);
    if learn.browsing() {
        let overflow = super::browser::show(learn, &mut col, t, out);
        return PaneOutput {
            overflow,
            pointing: None,
        };
    }
    lesson(learn, &mut col, t, inner, patch, live, out)
}

/// A lesson's step: the title and progress rows, the controls anchored to the bottom, the text between them.
fn lesson(
    learn: &Learn,
    ui: &mut Ui,
    t: &Theme,
    inner: Rect,
    patch: &Patch,
    live: &Live,
    out: &mut Intents,
) -> PaneOutput {
    let Some(view) = learn.view() else {
        return PaneOutput::default();
    };
    top(learn, view, ui, t, out);
    let top_bottom = ui.min_rect().bottom() + t.space.gap;
    let bottom_rect = Rect::from_min_max(egui::pos2(inner.left(), top_bottom), inner.max);
    let mut bottom = ui.new_child(
        UiBuilder::new()
            .max_rect(bottom_rect)
            .layout(Layout::bottom_up(Align::Min)),
    );
    bottom.spacing_mut().item_spacing = vec2(t.space.tight, t.space.tight);
    controls(learn, view, &mut bottom, t, out);
    let controls_top = bottom.min_rect().top() - t.space.gap;
    let text_rect = Rect::from_min_max(
        egui::pos2(inner.left(), top_bottom),
        egui::pos2(inner.right(), controls_top.max(top_bottom)),
    );
    let mut text = ui.new_child(
        UiBuilder::new()
            .max_rect(text_rect)
            .layout(Layout::top_down(Align::Min)),
    );
    let mut pointing = None;
    let shown = ScrollArea::vertical()
        .id_salt("enth.learn.text")
        .auto_shrink([false, false])
        .show(&mut text, |ui| {
            ui.spacing_mut().item_spacing = vec2(t.space.tight, t.space.gap);
            let cx = TextCx {
                learn,
                view,
                patch,
                live,
            };
            if let Some((bundle, _, step)) = learn.step()
                && let Some(source) = step.text.as_ref().and_then(|r| bundle.text(r))
            {
                let parsed = markdown::parse(source);
                doc(ui, t, &parsed.doc, &cx, &mut pointing, out);
            }
            if matches!(learn.step(), Some((_, _, s)) if matches!(s.kind, StepKind::Recreate { .. })) {
                line(
                    ui,
                    "Play it with the round's controls in the header: Target ▶ and Mine ▶ to compare, then Submit. \
                     Hint, Reveal and the results are the round's.",
                    t.text.label.clone(),
                    t.color.text.label,
                );
            }
        });
    let over = shown.content_size - shown.inner_rect.size();
    PaneOutput {
        overflow: vec2(over.x.max(0.0), over.y.max(0.0)),
        pointing,
    }
}

/// The step's state in words: what the progress row says after "Step n of m".
fn state(view: &LearnView) -> &'static str {
    if view.broken {
        "This lesson cannot continue: end it"
    } else if view.pending.opening {
        "Reopening…"
    } else if view.finished {
        "Lesson complete"
    } else {
        match view.phase {
            StepPhase::Passed => "✓ Passed",
            StepPhase::Checking { .. } => "Checking…",
            StepPhase::Working if view.pending.checking => "Checking…",
            StepPhase::Working => "",
        }
    }
}

/// The step kind's name for the player: the lesson shape's words (Hear, Name, Try, Do, Check).
fn kind_name(kind: &StepKind) -> &'static str {
    match kind {
        StepKind::Read => "Read",
        StepKind::Listen { .. } => "Listen",
        StepKind::Do { .. } => "Try",
        StepKind::Recreate { .. } => "Recreate",
        StepKind::Choose { .. } => "Choose",
    }
}

/// The title row and the progress row.
fn top(learn: &Learn, view: &LearnView, ui: &mut Ui, t: &Theme, out: &mut Intents) {
    let step = learn.step();
    let title = step.map_or("Lesson", |(b, _, _)| b.lesson.title.as_str());
    ui.horizontal(|ui| {
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            // In a Recreate step the challenge bar holds the header, where the Lesson entry would bring the pane back.
            let hide = button(ui, t, "Hide", "Hide lesson", !view.round, LearnIntent::Toggle, out);
            hide.on_hover_text("The whole editor; the lesson keeps going (the header's Lesson button brings it back)")
                .on_disabled_hover_text("The round's bar holds the header: the lesson stays beside it until the round");
            button(
                ui,
                t,
                "Lessons",
                "Lessons",
                true,
                LearnIntent::Browse(true),
                out,
            )
            .on_hover_text("The lesson browser (this lesson keeps going)");
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                ui.add(
                    Label::new(
                        RichText::new(title)
                            .font(t.text.title.clone())
                            .color(t.color.text.primary),
                    )
                    .truncate(),
                );
            });
        });
    });
    let index = view.lesson.as_ref().map_or(0, |r| usize::from(r.step));
    let count = usize::from(view.steps);
    let place = if count > 0 {
        format!("Step {} of {count}", index + 1)
    } else {
        String::new()
    };
    let state = state(view);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        // A dot per step: done in the label ink, the current one in the value accent.
        if let Some((bundle, _, _)) = step {
            let progress = learn.progress().lesson(&bundle.key());
            for (i, s) in bundle.lesson.steps.iter().enumerate() {
                let done = progress
                    .and_then(|p| p.steps.get(&s.id))
                    .is_some_and(|p| p.done);
                let (glyph, ink) = match (i == index, done) {
                    (true, _) => (glyph::DOT_ON, t.color.accent.value),
                    (false, true) => (glyph::DOT_ON, t.color.text.label),
                    (false, false) => (glyph::DOT_OFF, t.color.text.dimmed),
                };
                ui.label(RichText::new(glyph).font(t.text.label.clone()).color(ink));
            }
            ui.add_space(t.space.gap);
        }
        let kind = step.map_or("", |(_, _, s)| kind_name(&s.kind));
        let shown = [place.as_str(), kind, state]
            .iter()
            .filter(|s| !s.is_empty())
            .copied()
            .collect::<Vec<_>>()
            .join(&format!(" {} ", glyph::BULLET));
        let status = ui.label(
            RichText::new(shown)
                .font(t.text.label.clone())
                .color(t.color.text.label),
        );
        named(&status, Role::Status, format!("{place} {state}").trim());
    });
}

/// The controls, anchored to the bottom (`ui` lays out bottom-up): the way on, the step's own controls, then the
/// verdict, the hint and the last refusal above them.
fn controls(learn: &Learn, view: &LearnView, ui: &mut Ui, t: &Theme, out: &mut Intents) {
    let index = view.lesson.as_ref().map_or(0, |r| usize::from(r.step));
    let count = usize::from(view.steps);
    let live = !view.broken && !view.pending.opening && !view.finished;
    let width = ui.available_width();
    // The way on: Back, End lesson, Next (or Finish).
    ui.horizontal(|ui| {
        button(
            ui,
            t,
            &format!("{} Back", glyph::PREVIOUS),
            "Back",
            live && index > 0,
            LearnIntent::Back,
            out,
        );
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let last = count > 0 && index + 1 == count;
            let (text, label) = if last {
                (format!("Finish {}", glyph::NEXT), "Finish")
            } else {
                (format!("Next {}", glyph::NEXT), "Next")
            };
            button(ui, t, &text, label, live, LearnIntent::Next, out);
            end_menu(ui, t, out);
        });
    });
    let step = learn.step().filter(|_| live);
    if let Some((_, _, s)) = step {
        let open = view.phase != StepPhase::Passed;
        if matches!(s.kind, StepKind::Do { .. } | StepKind::Choose { .. }) {
            ui.horizontal(|ui| {
                button(
                    ui,
                    t,
                    "Hint",
                    "Hint",
                    open && !view.pending.hinting,
                    LearnIntent::Hint,
                    out,
                )
                .on_hover_text("The next hint (the step counts as helped)");
                button(ui, t, "Show me", "Show me", open, LearnIntent::Show, out)
                    .on_hover_text("Make the next change for me (the step counts as helped)");
            });
        }
        match &s.kind {
            StepKind::Listen { examples } => listen(learn, view, ui, t, examples, width, out),
            StepKind::Choose { options, .. } => {
                let labels: Vec<String> = options.iter().map(|e| e.label.clone()).collect();
                for row in rows(ui, t, &labels, width).iter().rev() {
                    ui.horizontal(|ui| {
                        for &i in row {
                            let o = u8::try_from(i).unwrap_or(u8::MAX);
                            let label = &labels[i];
                            button(
                                ui,
                                t,
                                label,
                                &format!("Choose {label}"),
                                open,
                                LearnIntent::Choose(o),
                                out,
                            );
                        }
                    });
                }
                caption(ui, t, "Your answer:");
                listen(learn, view, ui, t, options, width, out);
            }
            _ => {}
        }
    }
    // Above the controls: the verdict, the hint, a pending hint, the last refusal (bottom-up, so read upwards).
    let error = learn
        .said()
        .map(str::to_owned)
        .or_else(|| view.error.as_ref().map(|e| format!("{e}: {}", e.hint())));
    if let Some(e) = error {
        line(ui, &e, t.text.label.clone(), t.color.score.bad);
    }
    if view.pending.hinting {
        line(
            ui,
            "Working out a hint…",
            t.text.label.clone(),
            t.color.text.label,
        );
    }
    if let Some(hint) = view.hint.as_deref() {
        line(ui, hint, t.text.body.clone(), t.color.accent.value);
    }
    if let Some((_, _, s)) = step
        && let StepKind::Choose { options, .. } = &s.kind
    {
        let label = |i: u8| options.get(usize::from(i)).map(|e| e.label.as_str());
        let verdict = match (view.chose, view.answer) {
            (_, Some(a)) => label(a).map(|l| format!("{} The answer is {l}.", glyph::GOOD)),
            (Some((_, true)), _) => Some(format!("{} Right.", glyph::GOOD)),
            (Some((_, false)), _) => Some(format!("{} Not that one: listen again.", glyph::BAD)),
            _ => None,
        };
        if let Some(v) = verdict {
            line(ui, &v, t.text.body.clone(), t.color.text.primary);
        }
    }
}

/// A small caption above a group of controls.
fn caption(ui: &mut Ui, t: &Theme, text: &str) {
    ui.label(
        RichText::new(text)
            .font(t.text.label.clone())
            .color(t.color.text.label),
    );
}

/// The End lesson menu: restore my patch, or keep this one.
fn end_menu(ui: &mut Ui, t: &Theme, out: &mut Intents) {
    let menu = ui.menu_button(RichText::new("End").font(t.text.body.clone()), |ui| {
        let restore = ui
            .add(Button::new(
                RichText::new("End and restore my patch").font(t.text.body.clone()),
            ))
            .on_hover_text("Your own patch and its undo history come back");
        if restore.clicked() {
            out.push(Intent::Learn(LearnIntent::End(EndChoice::Restore)));
            ui.close();
        }
        let keep = ui
            .add(Button::new(
                RichText::new("End and keep this patch").font(t.text.body.clone()),
            ))
            .on_hover_text("Keep the lesson's patch as your own");
        if keep.clicked() {
            out.push(Intent::Learn(LearnIntent::End(EndChoice::Keep)));
            ui.close();
        }
    });
    named(&menu.response, Role::Button, "End lesson");
}

/// The examples' controls (bottom-up): ■ Stop and A/B, then a ▶ per example above them, in rows that fit.
fn listen(
    learn: &Learn,
    view: &LearnView,
    ui: &mut Ui,
    t: &Theme,
    examples: &[Example],
    width: f32,
    out: &mut Intents,
) {
    let ready = view.listen.ready;
    let playing = view.listen.example;
    ui.horizontal(|ui| {
        button(
            ui,
            t,
            &format!("{} Stop", glyph::STOP),
            "Stop listening",
            playing.is_some(),
            LearnIntent::Listen(None),
            out,
        );
        button(
            ui,
            t,
            "A/B",
            "A/B",
            learn.flip_to().is_some(),
            LearnIntent::Flip,
            out,
        )
        .on_hover_text("Flip between the last two examples you played, at the same moment");
        if view.pending.examples || (!ready && !examples.is_empty()) {
            caption(ui, t, "Rendering…");
        }
    });
    let texts: Vec<String> = examples
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let on = playing == u8::try_from(i).ok();
            format!("{} {}", if on { glyph::STOP } else { glyph::PLAY }, e.label)
        })
        .collect();
    for row in rows(ui, t, &texts, width).iter().rev() {
        ui.horizontal(|ui| {
            for &i in row {
                let e = u8::try_from(i).unwrap_or(u8::MAX);
                let on = playing == Some(e);
                button(
                    ui,
                    t,
                    &texts[i],
                    &format!("Listen {}", examples[i].label),
                    ready,
                    LearnIntent::Listen(if on { None } else { Some(e) }),
                    out,
                );
            }
        });
    }
}

/// What the step's text needs besides the AST.
struct TextCx<'a> {
    learn: &'a Learn,
    view: &'a LearnView,
    patch: &'a Patch,
    live: &'a Live,
}

impl TextCx<'_> {
    /// The step's examples (a Listen step's) or options (a Choose step's).
    fn examples(&self) -> &[Example] {
        match self.learn.step().map(|(_, _, s)| &s.kind) {
            Some(StepKind::Listen { examples }) => examples,
            Some(StepKind::Choose { options, .. }) => options,
            _ => &[],
        }
    }

    /// The intent that plays example `n` (1-based), or stops it while it plays.
    fn play(&self, n: u8) -> Option<LearnIntent> {
        let i = n.checked_sub(1)?;
        if usize::from(i) >= self.examples().len() {
            return None;
        }
        Some(LearnIntent::Listen(
            if self.view.listen.example == Some(i) {
                None
            } else {
                Some(i)
            },
        ))
    }
}

/// How a run of inline text is styled.
#[derive(Clone, Copy, Default)]
struct Style {
    strong: bool,
    italic: bool,
    /// The heading level (0: body text).
    heading: u8,
}

/// Draws a document's blocks.
fn doc(
    ui: &mut Ui,
    t: &Theme,
    doc: &Doc,
    cx: &TextCx<'_>,
    pointing: &mut Option<Selector>,
    out: &mut Intents,
) {
    for b in &doc.blocks {
        block(ui, t, b, cx, pointing, out);
    }
}

fn block(
    ui: &mut Ui,
    t: &Theme,
    b: &Block,
    cx: &TextCx<'_>,
    pointing: &mut Option<Selector>,
    out: &mut Intents,
) {
    match b {
        Block::Heading { level, inlines: i } => {
            let style = Style {
                heading: (*level).max(1),
                ..Style::default()
            };
            flow(ui, t, i, style, cx, pointing, out);
        }
        Block::Paragraph(i) => flow(ui, t, i, Style::default(), cx, pointing, out),
        Block::List { start, items } => {
            for (n, item) in items.iter().enumerate() {
                let marker = match start {
                    Some(first) => format!("{}.", first.saturating_add(n as u64)),
                    None => glyph::BULLET.to_owned(),
                };
                ui.horizontal_top(|ui| {
                    ui.add_sized(
                        vec2(t.space.section, ui.spacing().interact_size.y),
                        Label::new(
                            RichText::new(marker)
                                .font(t.text.body.clone())
                                .color(t.color.text.label),
                        ),
                    );
                    ui.vertical(|ui| {
                        for b in item {
                            block(ui, t, b, cx, pointing, out);
                        }
                    });
                });
            }
        }
        Block::Widgets(widgets) => {
            for w in widgets {
                widget(ui, t, w, cx, pointing, out);
            }
        }
        _ => {}
    }
}

/// A paragraph or a heading: its runs flow and wrap across the pane's width.
fn flow(
    ui: &mut Ui,
    t: &Theme,
    inlines: &[Inline],
    style: Style,
    cx: &TextCx<'_>,
    pointing: &mut Option<Selector>,
    out: &mut Intents,
) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        runs(ui, t, inlines, style, None, cx, pointing, out);
    });
}

/// The rich text of a run of `text` in `style` (a link's text underlined in the value accent).
fn rich(t: &Theme, text: &str, style: Style, link: bool) -> RichText {
    let font = match style.heading {
        1 => t.text.title.clone(),
        2 | 3 => t.text.strong.clone(),
        _ if style.strong => t.text.strong.clone(),
        _ => t.text.body.clone(),
    };
    let ink = if link {
        t.color.accent.value
    } else {
        t.color.text.primary
    };
    let mut r = RichText::new(text).font(font).color(ink);
    if style.italic {
        r = r.italics();
    }
    if link {
        r = r.underline();
    }
    r
}

/// Draws inline runs, each a label in the flow. Inside a link, `link` is its target.
#[allow(
    clippy::too_many_arguments,
    reason = "the run's whole context: where, theme, runs, style, its link, the step, hover and intents"
)]
fn runs(
    ui: &mut Ui,
    t: &Theme,
    inlines: &[Inline],
    style: Style,
    link: Option<&LinkTarget>,
    cx: &TextCx<'_>,
    pointing: &mut Option<Selector>,
    out: &mut Intents,
) {
    for inline in inlines {
        match inline {
            Inline::Text(text) => {
                let Some(target) = link else {
                    ui.add(Label::new(rich(t, text, style, false)).wrap());
                    continue;
                };
                let response = ui.add(
                    Label::new(rich(t, text, style, true))
                        .wrap()
                        .sense(Sense::click()),
                );
                named(&response, Role::Link, text);
                follow(ui, &response, target, cx, pointing, out);
            }
            Inline::Code(code) => {
                ui.add(
                    Label::new(
                        RichText::new(code)
                            .font(t.text.value.clone())
                            .color(t.color.text.primary)
                            .background_color(t.color.surface.recessed),
                    )
                    .wrap(),
                );
            }
            Inline::LineBreak => ui.end_row(),
            Inline::Emphasis(inner) => runs(
                ui,
                t,
                inner,
                Style {
                    italic: true,
                    ..style
                },
                link,
                cx,
                pointing,
                out,
            ),
            Inline::Strong(inner) => runs(
                ui,
                t,
                inner,
                Style {
                    strong: true,
                    ..style
                },
                link,
                cx,
                pointing,
                out,
            ),
            Inline::Link { target, inlines } => {
                runs(ui, t, inlines, style, Some(target), cx, pointing, out);
            }
            _ => {}
        }
    }
}

/// What a link does: a control's is outlined while hovered, a play link plays, a web link opens the browser.
fn follow(
    ui: &Ui,
    response: &Response,
    target: &LinkTarget,
    cx: &TextCx<'_>,
    pointing: &mut Option<Selector>,
    out: &mut Intents,
) {
    match target {
        LinkTarget::Control(selector) => {
            if response.hovered() {
                *pointing = Some(selector.clone());
            }
        }
        LinkTarget::Play(n) => {
            let intent = cx.play(*n).filter(|_| cx.view.listen.ready);
            if intent.is_some() {
                ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
            }
            if response.clicked()
                && let Some(intent) = intent
            {
                out.push(Intent::Learn(intent));
            }
        }
        LinkTarget::Web(url) => {
            if response.hovered() {
                ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
            }
            if response.clicked() {
                ui.ctx().open_url(egui::OpenUrl::new_tab(url));
            }
        }
        _ => {}
    }
}

/// One widget of an `enth` block.
fn widget(
    ui: &mut Ui,
    t: &Theme,
    w: &Widget,
    cx: &TextCx<'_>,
    pointing: &mut Option<Selector>,
    out: &mut Intents,
) {
    let width = ui.available_width();
    match w {
        Widget::Play(n) => {
            let label = cx
                .examples()
                .get(usize::from(n.saturating_sub(1)))
                .map_or_else(|| format!("example {n}"), |e| e.label.clone());
            let playing = cx.view.listen.example == n.checked_sub(1);
            let text = format!(
                "{} {label}",
                if playing { glyph::STOP } else { glyph::PLAY }
            );
            let intent = cx.play(*n);
            let response = ui.add_enabled(
                intent.is_some() && cx.view.listen.ready,
                Button::new(RichText::new(text).font(t.text.body.clone())),
            );
            named(&response, Role::Button, &format!("Play {label}"));
            if response.clicked()
                && let Some(intent) = intent
            {
                out.push(Intent::Learn(intent));
            }
        }
        Widget::Control(Selector::Param(addr)) => control(ui, t, addr, cx, pointing, out),
        Widget::Control(selector) => {
            let response = ui.add(
                Label::new(
                    RichText::new(selector.to_string())
                        .font(t.text.value.clone())
                        .color(t.color.accent.value)
                        .background_color(t.color.surface.recessed),
                )
                .sense(Sense::hover()),
            );
            if response.hovered() {
                *pointing = Some(selector.clone());
            }
        }
        Widget::Scope => {
            let samples = cx.live.scope().map_or(&[][..], |(s, _)| s);
            let (rect, _) =
                ui.allocate_exact_size(vec2(width, t.size.learn.scope_height), Sense::hover());
            Display::new(t, Trace::Scope(samples), rect.size())
                .caption("Scope")
                .paint(ui, rect);
        }
        Widget::Shape(module) => {
            let Some((_, m)) = cx.patch.find(module) else {
                return;
            };
            let (rect, response) =
                ui.allocate_exact_size(vec2(width, t.size.learn.shape_height), Sense::hover());
            let value = |key: &str| {
                ParamAddr::module(module.clone(), key)
                    .ok()
                    .and_then(|a| Some((a.clone(), cx.patch.value(&a)?)))
            };
            let norm = |key: &str| {
                value(key)
                    .and_then(|(a, v)| Some(cx.patch.spec(&a)?.to_norm(v)))
                    .unwrap_or(0.0)
            };
            match m.kind {
                ModuleKind::Adsr => {
                    let [peak, corner, release] = envelope_stages(t).corners(
                        norm("attack"),
                        norm("decay"),
                        norm("sustain"),
                        norm("release"),
                    );
                    let points = [vec2(0.0, 0.0), peak, corner, release, vec2(1.0, 0.0)];
                    let curve = |x: f32| through(&points, x);
                    Display::new(t, Trace::Curve(&curve), rect.size())
                        .ink(Ink::Modulation)
                        .paint(ui, rect);
                }
                ModuleKind::Lfo => {
                    let shape = value("shape").and_then(|(_, v)| LfoShape::from_value(v));
                    let curve = |x| shapes::lfo(shape, x);
                    Display::new(t, Trace::Curve(&curve), rect.size())
                        .ink(Ink::Modulation)
                        .paint(ui, rect);
                }
                _ => {}
            }
            if response.hovered() {
                *pointing = Some(Selector::Module(enth_patch::Target::Module(module.clone())));
            }
        }
        _ => {}
    }
}

/// `y` at `x` on the polyline through `points` (x rising).
fn through(points: &[Vec2], x: f32) -> f32 {
    for pair in points.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        if x <= b.x {
            let span = b.x - a.x;
            return if span > 0.0 {
                a.y + (b.y - a.y) * (x - a.x) / span
            } else {
                b.y
            };
        }
    }
    points.last().map_or(0.0, |p| p.y)
}

/// The `control` widget: the live control in the text, a knob under its own id (the card's knob keeps its), with
/// the same gestures as the card's: `Begin`, `Set`, `End`, a double-click reset. A switch or a short enum shows its
/// value as a chip. Hovering it outlines the card's control.
fn control(
    ui: &mut Ui,
    t: &Theme,
    addr: &ParamAddr,
    cx: &TextCx<'_>,
    pointing: &mut Option<Selector>,
    out: &mut Intents,
) {
    let (Some(spec), Some(value)) = (cx.patch.spec(addr), cx.patch.value(addr)) else {
        return;
    };
    let kind = match addr.target() {
        enth_patch::Target::Module(id) => match cx.patch.find(id) {
            Some((_, m)) => CardKind::Module(m.kind),
            None => return,
        },
        enth_patch::Target::Voice => CardKind::Voice,
        enth_patch::Target::Master => CardKind::Master,
    };
    let label = present::label(kind, spec);
    let text = match spec.enum_item(value) {
        Some(item) if spec.enum_items().is_some() => {
            present::item_label(kind, spec, item).to_owned()
        }
        _ => value_text(spec, value),
    };
    let response = match present::control(t, spec) {
        Control::Knob | Control::Stepped(_) => {
            let mut norm = spec.to_norm(value);
            let mut knob = Knob::new(t, &mut norm, label)
                .id(ids::param(addr).with("lesson"))
                .value_text(text)
                .default_value(spec.to_norm(spec.default))
                .bipolar(present::bipolar(spec));
            if let Control::Stepped(steps) = present::control(t, spec) {
                knob = knob.steps(steps);
            }
            let locked = cx
                .learn
                .overlay()
                .is_some_and(|o| crate::editor::overlay::Overlay::locked(o, addr));
            if locked {
                knob = knob.emphasis(KnobEmphasis::Dimmed);
            }
            let response = ui.add(knob);
            let grip = || Grip::Param(addr.clone());
            if response.drag_started() {
                out.push(Intent::Begin(grip()));
            }
            if response.changed() {
                let v = spec.clamp(spec.from_norm(norm));
                if v.to_bits() != value.to_bits() {
                    out.push(Intent::Set {
                        addr: addr.clone(),
                        value: v,
                    });
                }
            }
            if response.drag_stopped() {
                out.push(Intent::End(grip()));
            }
            response
        }
        Control::Segmented | Control::Toggle => ui.add(
            Label::new(
                RichText::new(format!("{label} {text}"))
                    .font(t.text.value.clone())
                    .color(t.color.text.primary)
                    .background_color(t.color.surface.recessed),
            )
            .sense(Sense::hover()),
        ),
    };
    if response.hovered() || response.dragged() {
        *pointing = Some(Selector::Param(addr.clone()));
    }
}
