//! **Owner: lane LUI** (the Learn view, feature `learn`). Pre-declared by lane S0l (R2); **lane SKL** wrote this
//! minimal panel for freeze point F2l, which LUI replaces with the real lesson pane.
//!
//! The planned pane: the `LessonPane` of `widgets/lesson_pane.rs` fed by `enth-learn`'s block AST: headings, bold,
//! bullets and `enth:` links (a link such as `[Cutoff](enth:filter1.cutoff)` highlights its control on hover),
//! drawn with design tokens only; the Hear / Try / Do / Check controls; the hint tiers. A 320 px pane fits 1200 x 800
//! only when the step hides unused modules (the G1 patch overflows by 276 to 732 px), so LUI needs a collapsible pane
//! or a focus-card region, with an overflow report. Sizes go in `theme/sizes/learn.rs`.
//!
//! **SKL's panel** takes a column at the window's right edge while it is open (`size.learn.pane`) and draws, in plain
//! text with the theme's tokens: with no lesson, the installed lessons with a **Start** each (and the packs the shell
//! refused); in a lesson, its title, "Step n of m" and where the step stands, the step's text (markdown reduced to
//! plain lines), the examples' Listen buttons, the ear question's options, Hint and Show me, Back and Next, and End
//! with Keep or Restore. Every button has an accessible name, so kittest drives it by label. It asks for no frames
//! of its own.

use egui::{
    Align, Button, Label, Layout, Rect, RichText, Role, ScrollArea, Ui, UiBuilder, WidgetInfo, vec2,
};
use enth_learn::{StepKind, StepPhase};
use enth_session::game::EndChoice;

use super::state::{Learn, LearnIntent};
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
    intent: LearnIntent,
    out: &mut Intents,
) {
    let response = ui.add_enabled(
        enabled,
        Button::new(RichText::new(text).font(t.text.body.clone())),
    );
    named(&response, label);
    if response.clicked() {
        out.push(Intent::Learn(intent));
    }
}

/// A step's markdown as plain lines: headings marked (drawn in the strong font), `**bold**` and `[text](link)`
/// reduced to their text, bullets drawn as `•`. Lane LUI replaces this with the block AST.
pub fn plain(markdown: &str) -> Vec<(bool, String)> {
    let mut out = Vec::new();
    for line in markdown.lines() {
        let line = line.trim_end();
        if line.trim().is_empty() {
            continue;
        }
        let (heading, body) = match line.trim_start().strip_prefix('#') {
            Some(rest) => (true, rest.trim_start_matches('#').trim()),
            None => (false, line),
        };
        let body = match body.trim_start().strip_prefix("- ") {
            Some(rest) => format!("• {rest}"),
            None => body.to_owned(),
        };
        out.push((heading, unlink(&body.replace("**", ""))));
    }
    out
}

/// `[text](target)` → `text`.
fn unlink(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(open) = rest.find('[') {
        let after = &rest[open + 1..];
        let link = after.find("](").and_then(|mid| {
            after[mid + 2..]
                .find(')')
                .map(|close| (mid, mid + 2 + close))
        });
        if let Some((mid, close)) = link {
            out.push_str(&rest[..open]);
            out.push_str(&after[..mid]);
            rest = &after[close + 1..];
        } else {
            out.push_str(&rest[..=open]);
            rest = after;
        }
    }
    out.push_str(rest);
    out
}

/// Draws the panel into `rect` (its column at the window's right edge) from `learn`. Returns how far its content reached past
/// it (it scrolls; the frame report lists it).
pub(crate) fn show(
    learn: &Learn,
    ui: &mut Ui,
    t: &Theme,
    rect: Rect,
    out: &mut Intents,
) -> egui::Vec2 {
    let mut ui = ui.new_child(
        UiBuilder::new()
            .max_rect(rect)
            .layout(Layout::top_down(Align::Min))
            .id_salt("enth.learn.panel"),
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
        .id_salt("enth.learn.scroll")
        .auto_shrink([false, false])
        .show(&mut col, |ui| {
            ui.spacing_mut().item_spacing = vec2(t.space.gap, t.space.tight);
            if learn.view().is_some() {
                lesson(learn, ui, t, out);
            } else {
                list(learn, ui, t, out);
            }
        });
    let over = shown.content_size - shown.inner_rect.size();
    vec2(over.x.max(0.0), over.y.max(0.0))
}

/// No lesson live: the installed lessons, each with Start.
fn list(learn: &Learn, ui: &mut Ui, t: &Theme, out: &mut Intents) {
    ui.horizontal(|ui| {
        ui.label(
            RichText::new("Lessons")
                .font(t.text.title.clone())
                .color(t.color.text.primary),
        );
        button(
            ui,
            t,
            "Close",
            "Close lessons",
            true,
            LearnIntent::Toggle,
            out,
        );
    });
    if learn.lessons().is_empty() {
        ui.label(
            RichText::new("No lessons are installed.")
                .font(t.text.body.clone())
                .color(t.color.text.dimmed),
        );
    }
    for (i, bundle) in learn.lessons().iter().enumerate() {
        let title = bundle.lesson.title.as_str();
        ui.horizontal(|ui| {
            button(
                ui,
                t,
                "Start",
                &format!("Start {title}"),
                true,
                LearnIntent::Start(i),
                out,
            );
            ui.add(Label::new(
                RichText::new(format!("{title} · {}", bundle.key()))
                    .font(t.text.body.clone())
                    .color(t.color.text.primary),
            ));
        });
        ui.add(
            Label::new(
                RichText::new(bundle.lesson.objective.as_str())
                    .font(t.text.label.clone())
                    .color(t.color.text.label),
            )
            .wrap(),
        );
    }
    for problem in learn.problems() {
        ui.add(
            Label::new(
                RichText::new(problem)
                    .font(t.text.label.clone())
                    .color(t.color.text.dimmed),
            )
            .wrap(),
        );
    }
    if let Some(said) = learn.said() {
        ui.add(
            Label::new(
                RichText::new(said)
                    .font(t.text.label.clone())
                    .color(t.color.score.bad),
            )
            .wrap(),
        );
    }
}

/// A lesson live: the step and its controls. The controls come before the step's text, in a fixed order (Back and
/// Next; the step's Listen, Choose, Hint and Show me), so they stand at the same place whatever the text's length
/// (the REAPER `lesson` scenario clicks them by position; `tests/learn_panel.rs` pins the points).
fn lesson(learn: &Learn, ui: &mut Ui, t: &Theme, out: &mut Intents) {
    let Some(view) = learn.view() else {
        return;
    };
    let step = learn.step();
    let title = step.map_or("Lesson", |(b, _, _)| b.lesson.title.as_str());
    let index = view.lesson.as_ref().map_or(0, |r| usize::from(r.step));
    let count = usize::from(view.steps);
    let state = if view.broken {
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
    };
    ui.label(
        RichText::new(title)
            .font(t.text.title.clone())
            .color(t.color.text.primary),
    );
    let place = if count > 0 {
        format!("Step {} of {count}", index + 1)
    } else {
        String::new()
    };
    let status = ui.label(
        RichText::new(format!("{place}  {state}"))
            .font(t.text.label.clone())
            .color(t.color.text.label),
    );
    named(&status, format!("{place} {state}").trim());
    let live = !view.broken && !view.pending.opening && !view.finished;
    ui.horizontal(|ui| {
        button(
            ui,
            t,
            "◀ Back",
            "Back",
            live && index > 0,
            LearnIntent::Back,
            out,
        );
        let last = count > 0 && index + 1 == count;
        let next = if last { "Finish ▶" } else { "Next ▶" };
        button(
            ui,
            t,
            next,
            if last { "Finish" } else { "Next" },
            live,
            LearnIntent::Next,
            out,
        );
    });
    // The step's own controls.
    if let Some((_, _, s)) = step
        && live
    {
        let listen = |ui: &mut Ui, out: &mut Intents, labels: &[String], stop: bool| {
            ui.horizontal_wrapped(|ui| {
                for (i, label) in labels.iter().enumerate() {
                    let i = u8::try_from(i).unwrap_or(u8::MAX);
                    let playing = view.listen.example == Some(i);
                    button(
                        ui,
                        t,
                        &format!("{} {label}", if playing { "■" } else { "▶" }),
                        &format!("Listen {label}"),
                        view.listen.ready,
                        LearnIntent::Listen(if playing { None } else { Some(i) }),
                        out,
                    );
                }
                if stop {
                    button(
                        ui,
                        t,
                        "Stop",
                        "Stop listening",
                        view.listen.example.is_some(),
                        LearnIntent::Listen(None),
                        out,
                    );
                }
            });
        };
        match &s.kind {
            StepKind::Listen { examples } => {
                let labels: Vec<String> = examples.iter().map(|e| e.label.clone()).collect();
                listen(ui, out, &labels, true);
            }
            StepKind::Choose { options, .. } => {
                let labels: Vec<String> = options.iter().map(|e| e.label.clone()).collect();
                listen(ui, out, &labels, true);
                ui.horizontal_wrapped(|ui| {
                    for (i, label) in labels.iter().enumerate() {
                        let i = u8::try_from(i).unwrap_or(u8::MAX);
                        button(
                            ui,
                            t,
                            label,
                            &format!("Choose {label}"),
                            view.phase != StepPhase::Passed,
                            LearnIntent::Choose(i),
                            out,
                        );
                    }
                });
                let verdict = match (view.chose, view.answer) {
                    (_, Some(a)) => labels
                        .get(usize::from(a))
                        .map(|l| format!("The answer is {l}.")),
                    (Some((_, true)), _) => Some("✓ Right.".to_owned()),
                    (Some((_, false)), _) => Some("✗ Not that one: listen again.".to_owned()),
                    _ => None,
                };
                if let Some(v) = verdict {
                    ui.label(
                        RichText::new(v)
                            .font(t.text.body.clone())
                            .color(t.color.text.primary),
                    );
                }
            }
            StepKind::Recreate { .. } => {
                ui.label(
                    RichText::new("Recreate the target with the round's controls above.")
                        .font(t.text.label.clone())
                        .color(t.color.text.label),
                );
            }
            _ => {}
        }
        if matches!(s.kind, StepKind::Do { .. } | StepKind::Choose { .. }) {
            ui.horizontal(|ui| {
                let open = view.phase != StepPhase::Passed;
                button(
                    ui,
                    t,
                    "Hint",
                    "Hint",
                    open && !view.pending.hinting,
                    LearnIntent::Hint,
                    out,
                );
                button(ui, t, "Show me", "Show me", open, LearnIntent::Show, out);
            });
        }
    }
    if let Some(hint) = view.hint.as_deref() {
        ui.add(
            Label::new(
                RichText::new(hint)
                    .font(t.text.body.clone())
                    .color(t.color.accent.value),
            )
            .wrap(),
        );
    }
    if view.pending.hinting {
        ui.label(
            RichText::new("Working out a hint…")
                .font(t.text.label.clone())
                .color(t.color.text.label),
        );
    }
    if let Some((bundle, _, s)) = step
        && let Some(text) = s.text.as_ref().and_then(|r| bundle.text(r))
    {
        for (heading, line) in plain(text) {
            let font = if heading {
                t.text.strong.clone()
            } else {
                t.text.body.clone()
            };
            ui.add(Label::new(RichText::new(line).font(font).color(t.color.text.primary)).wrap());
        }
    }
    ui.horizontal_wrapped(|ui| {
        ui.label(
            RichText::new("End lesson:")
                .font(t.text.label.clone())
                .color(t.color.text.label),
        );
        button(
            ui,
            t,
            "Restore my patch",
            "End and restore my patch",
            true,
            LearnIntent::End(EndChoice::Restore),
            out,
        );
        button(
            ui,
            t,
            "Keep this patch",
            "End and keep this patch",
            true,
            LearnIntent::End(EndChoice::Keep),
            out,
        );
    });
    let error = learn
        .said()
        .map(str::to_owned)
        .or_else(|| view.error.as_ref().map(|e| format!("{e}: {}", e.hint())));
    if let Some(e) = error {
        ui.add(
            Label::new(
                RichText::new(e)
                    .font(t.text.label.clone())
                    .color(t.color.score.bad),
            )
            .wrap(),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_becomes_plain_lines() {
        let lines = plain(
            "# Open and closed\n\nOnly the [cutoff](enth:filter1.cutoff) differs.\n\n- **Open** lets it through.\n",
        );
        assert_eq!(
            lines,
            vec![
                (true, "Open and closed".to_owned()),
                (false, "Only the cutoff differs.".to_owned()),
                (false, "• Open lets it through.".to_owned()),
            ]
        );
        assert_eq!(unlink("a [b] c"), "a [b] c");
    }
}
