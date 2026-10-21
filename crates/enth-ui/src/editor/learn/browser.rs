//! **Owner: lane LUI** (the Learn view, feature `learn`). Pre-declared by lane S0l (R2).
//!
//! **The lesson browser**, in the lesson pane when no lesson is live (or over one, from the pane's **Lessons**): the
//! installed packs and their lessons, with each lesson's progress, **the tour first**. Its shelves are tabs, so each
//! lesson's **Start** has one accessible name at a time: a tab per track (an ordered list of lessons across packs:
//! "The tour", its lessons in order, a lesson whose pack is missing listed as not installed) and **All lessons** (every
//! pack, its title, id, version and lessons in the pack's order). A row is **Start**, the lesson's title, its progress
//! ("3/11", "✓ 11/11" once every step is done) and its objective. The packs the shell refused are listed at the end
//! with their reasons ("needs a newer enth" included, never hidden).
//!
//! **Review** (lane REV's queue of due skills, fed by Recreate's rings) has its entry here already: a button that is
//! disabled and says it comes later, behind nothing; REV enables it and handles `LearnIntent::Review`. "Open packs
//! folder" and "Get more lessons" are left for a later lane: they need the shell's folder and the releases page.
//! The view rescans when the window opens (the shell's `set_library`). Sizes come from `theme/sizes/learn.rs`.

use egui::{Align, Label, Layout, RichText, Role, ScrollArea, Ui, Vec2, vec2};

use super::pane::{button, named};
use super::state::{Learn, LearnIntent, Shelf};
use crate::Theme;
use crate::editor::cards::Intents;
use crate::theme::glyph;

/// Draws the browser into `ui` (the pane's inner column). Returns how far its list reached past the pane.
pub(crate) fn show(learn: &Learn, ui: &mut Ui, t: &Theme, out: &mut Intents) -> Vec2 {
    ui.horizontal(|ui| {
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            button(
                ui,
                t,
                "Hide",
                "Hide lessons",
                true,
                LearnIntent::Toggle,
                out,
            )
            .on_hover_text("The whole editor");
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                ui.label(
                    RichText::new("Lessons")
                        .font(t.text.title.clone())
                        .color(t.color.text.primary),
                );
            });
        });
    });
    ui.horizontal_wrapped(|ui| {
        if learn.view().is_some() {
            button(
                ui,
                t,
                &format!("{} Back to the lesson", glyph::PREVIOUS),
                "Back to the lesson",
                true,
                LearnIntent::Browse(false),
                out,
            );
        }
        // Lane REV's entry: there, and disabled, until the Review queue exists.
        button(ui, t, "Review", "Review", false, LearnIntent::Review, out)
            .on_disabled_hover_text(
                "Review: a few items from the lessons you passed, by your weakest skills (comes in a later update)",
            );
    });
    let shelf = learn.shelf();
    ui.horizontal_wrapped(|ui| {
        for (i, track) in learn.tracks().iter().enumerate() {
            tab(
                ui,
                t,
                &track.title,
                shelf == Shelf::Track(i),
                Shelf::Track(i),
                out,
            );
        }
        if !learn.lessons().is_empty() || learn.tracks().is_empty() {
            tab(
                ui,
                t,
                "All lessons",
                shelf == Shelf::Packs,
                Shelf::Packs,
                out,
            );
        }
    });
    let shown = ScrollArea::vertical()
        .id_salt("enth.learn.browser")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing = vec2(t.space.tight, t.space.tight);
            match shelf {
                Shelf::Track(i) => {
                    if let Some(track) = learn.tracks().get(i) {
                        for (n, entry) in track.entries.iter().enumerate() {
                            if let Some(lesson) = entry.lesson {
                                row(learn, ui, t, lesson, Some(n + 1), out);
                            } else {
                                let text = format!(
                                    "{}. {} {} not installed",
                                    n + 1,
                                    entry.key,
                                    glyph::BULLET
                                );
                                ui.add(
                                    Label::new(
                                        RichText::new(text)
                                            .font(t.text.label.clone())
                                            .color(t.color.text.dimmed),
                                    )
                                    .wrap(),
                                );
                            }
                        }
                    }
                }
                Shelf::Packs => {
                    if learn.lessons().is_empty() {
                        ui.label(
                            RichText::new("No lessons are installed.")
                                .font(t.text.body.clone())
                                .color(t.color.text.dimmed),
                        );
                    }
                    for pack in learn.packs() {
                        ui.add_space(t.space.tight);
                        ui.label(
                            RichText::new(&pack.title)
                                .font(t.text.strong.clone())
                                .color(t.color.text.primary),
                        );
                        let about = [
                            pack.id.as_str().to_owned(),
                            pack.version.clone(),
                            format!("{} lessons", pack.lessons.len()),
                        ]
                        .into_iter()
                        .filter(|s| !s.is_empty())
                        .collect::<Vec<_>>()
                        .join(&format!(" {} ", glyph::BULLET));
                        ui.label(
                            RichText::new(about)
                                .font(t.text.label.clone())
                                .color(t.color.text.label),
                        );
                        for &lesson in &pack.lessons {
                            row(learn, ui, t, lesson, None, out);
                        }
                    }
                }
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
        });
    let over = shown.content_size - shown.inner_rect.size();
    vec2(over.x.max(0.0), over.y.max(0.0))
}

/// A shelf's tab.
fn tab(ui: &mut Ui, t: &Theme, title: &str, on: bool, shelf: Shelf, out: &mut Intents) {
    let response = ui.selectable_label(on, RichText::new(title).font(t.text.body.clone()));
    named(&response, Role::Tab, title);
    if response.clicked() && !on {
        out.push(crate::editor::cards::Intent::Learn(LearnIntent::Shelf(
            shelf,
        )));
    }
}

/// One lesson: Start, its title (numbered on a track), its progress, its objective.
fn row(
    learn: &Learn,
    ui: &mut Ui,
    t: &Theme,
    lesson: usize,
    number: Option<usize>,
    out: &mut Intents,
) {
    let Some(bundle) = learn.lessons().get(lesson) else {
        return;
    };
    let title = bundle.lesson.title.as_str();
    let standing = learn.standing(lesson);
    let live = learn
        .view()
        .and_then(|v| v.lesson.as_ref())
        .is_some_and(|r| r.pack == bundle.pack && r.lesson == bundle.lesson.id);
    ui.horizontal(|ui| {
        let start = button(
            ui,
            t,
            if live { "Restart" } else { "Start" },
            &format!("Start {title}"),
            true,
            LearnIntent::Start(lesson),
            out,
        );
        if learn.view().is_some() {
            start.on_hover_text("Leaves the lesson you are in (your own patch stays stashed)");
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let progress = if standing.completed {
                format!("{} {}/{}", glyph::GOOD, standing.done, standing.steps)
            } else {
                format!("{}/{}", standing.done, standing.steps)
            };
            let ink = if standing.completed {
                t.color.score.good
            } else {
                t.color.text.label
            };
            let shown = ui.add_sized(
                vec2(t.size.learn.progress_width, ui.spacing().interact_size.y),
                Label::new(
                    RichText::new(&progress)
                        .font(t.text.label.clone())
                        .color(ink),
                ),
            );
            named(
                &shown,
                Role::Status,
                &format!("{title} progress {}/{}", standing.done, standing.steps),
            );
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                let text = match number {
                    Some(n) => format!("{n}. {title}"),
                    None => title.to_owned(),
                };
                ui.add(
                    Label::new(
                        RichText::new(text)
                            .font(t.text.body.clone())
                            .color(t.color.text.primary),
                    )
                    .truncate(),
                );
            });
        });
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
