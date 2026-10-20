//! The browse panel (lane PBU), drawn over the body: the tabs (All, Factory, User, Favorites), the search field, one
//! row per preset and a footer line.
//!
//! A row is ☆ (star it), ▶ (hold to hear it: it loads the preset, then plays middle C until the pointer is released),
//! the name, where it lives, its tags and its author. A click on the row loads it (one undo step), a double-click
//! loads it and closes the panel; Escape and × close it. The loaded preset's row is highlighted. The search field
//! takes words in any order, `#tag` and `by:author` (`Library::search`), and is never focused by itself: inside a
//! host the keyboard reaches the plugin only while a field is being typed in (fork patch 4), so space still starts the
//! host's transport while browsing.

use egui::{
    Align, Align2, Button, Id, Key, Label, Layout, Rect, RichText, Role, ScrollArea, Sense,
    TextEdit, Ui, UiBuilder, WidgetInfo, pos2, vec2,
};
use enth_session::{PresetEntry, PresetKey};

use super::{PresetIntent, Presets, Tab, glyph_button, square, text_left};
use crate::Theme;
use crate::editor::cards::{Intent, Intents, ids};
use crate::theme::glyph;

/// The columns of one row (and of the column headings).
struct Columns {
    star: Rect,
    play: Rect,
    name: Rect,
    place: Rect,
    tags: Rect,
    author: Rect,
}

impl Columns {
    fn of(t: &Theme, row: Rect) -> Self {
        let s = &t.size.browser;
        let side = t.size.control_height;
        let span =
            |from: f32, width: f32| Rect::from_x_y_ranges(from..=from + width, row.y_range());
        let star = span(row.left(), side);
        let play = span(star.right() + t.space.tight, side);
        let name = span(play.right() + t.space.gap, s.name_column);
        let place = span(name.right() + t.space.gap, s.folder_column);
        let author = span(row.right() - s.author_column, s.author_column);
        let tags_left = place.right() + t.space.gap;
        let tags = Rect::from_x_y_ranges(
            tags_left..=(author.left() - t.space.gap).max(tags_left),
            row.y_range(),
        );
        Columns {
            star,
            play,
            name,
            place,
            tags,
            author,
        }
    }
}

/// The stable id of a preset's row; its ☆ and ▶ are `.with("star")` and `.with("play")`.
fn row_id(key: &PresetKey) -> Id {
    Id::unique(("enth.preset", key))
}

/// Where a preset lives, as the row shows it: its folder, after its source in the All and Favorites tabs
/// ("Factory › bass").
fn place(entry: &PresetEntry, with_source: bool) -> String {
    let folder = entry.key.folder();
    match (with_source, folder.is_empty()) {
        (false, _) => folder.to_owned(),
        (true, true) => entry.key.source.label().to_owned(),
        (true, false) => format!("{} {} {folder}", entry.key.source.label(), glyph::PATH),
    }
}

pub(super) fn show(
    p: &mut Presets,
    ui: &mut Ui,
    t: &Theme,
    body: Rect,
    last: Option<&PresetKey>,
    out: &mut Intents,
) {
    let c = &t.color;
    let mut ui = ui.new_child(
        UiBuilder::new()
            .max_rect(body)
            .scope_id(ids::region("browser")),
    );
    ui.shrink_clip_rect(body);
    ui.painter()
        .rect_filled(body, t.radius.medium, c.surface.card);
    if ui.input(|i| i.key_pressed(Key::Escape)) {
        out.push(Intent::Preset(PresetIntent::Close));
    }
    let inner = body.shrink(t.space.pad);
    let side = t.size.control_height;
    let top = Rect::from_min_size(inner.min, vec2(inner.width(), side));
    let heads = top.translate(vec2(0.0, side + t.space.gap));
    let footer = Rect::from_min_max(pos2(inner.left(), inner.bottom() - side), inner.max);
    let list = Rect::from_min_max(
        pos2(inner.left(), heads.bottom() + t.space.tight),
        pos2(inner.right(), footer.top() - t.space.gap),
    );

    // The tabs and the search field.
    let mut row = ui.new_child(
        UiBuilder::new()
            .max_rect(top)
            .layout(Layout::left_to_right(Align::Center)),
    );
    row.spacing_mut().item_spacing.x = t.space.tight;
    for tab in Tab::ALL {
        let text = RichText::new(tab.label()).font(t.text.body.clone());
        if row.add(Button::selectable(p.tab == tab, text)).clicked() {
            p.tab = tab;
        }
    }
    row.add_space(t.space.section);
    let search = TextEdit::singleline(&mut p.search)
        .id(Id::unique("enth.browser.search"))
        .hint_text("Search: words, #tag, by:author")
        .font(t.text.body.clone())
        .desired_width(t.size.browser.search_width);
    let _ = row.add(search).accessible_name("Search presets");
    let close = Rect::from_min_size(pos2(top.right() - side, top.top()), square(t));
    if glyph_button(
        &ui,
        t,
        close,
        Id::unique("enth.browser.close"),
        glyph::CLOSE,
        c.text.primary,
        "Close browser",
    )
    .on_hover_text("Close (Escape)")
    .clicked()
    {
        out.push(Intent::Preset(PresetIntent::Close));
    }

    let count = p.list().len();
    let counted = match count {
        1 => "1 preset".to_owned(),
        n => format!("{n} presets"),
    };
    ui.painter().text(
        pos2(close.left() - t.space.gap, top.center().y),
        Align2::RIGHT_CENTER,
        counted,
        t.text.label.clone(),
        c.text.label,
    );

    // Column headings.
    let with_source = matches!(p.tab, Tab::All | Tab::Favorites);
    let cols = Columns::of(t, heads);
    for (rect, text) in [
        (cols.name, "Name"),
        (cols.place, if with_source { "Where" } else { "Folder" }),
        (cols.tags, "Tags"),
        (cols.author, "Author"),
    ] {
        text_left(&ui, rect, text, &t.text.label, c.text.label);
    }

    // The rows.
    let holding = p.holding;
    let (shown, favorites) = (&p.shown, &p.favorites);
    let mut rows = ui.new_child(
        UiBuilder::new()
            .max_rect(list)
            .layout(Layout::top_down(Align::Min)),
    );
    rows.spacing_mut().item_spacing.y = t.space.hairline;
    if shown.is_empty() {
        let empty = match (p.tab, p.search.trim().is_empty()) {
            (_, false) => "No presets match the search",
            (Tab::User, true) => "No presets of your own yet: Save one",
            (Tab::Favorites, true) => "No favorites yet: star a preset with ☆",
            _ => "No presets",
        };
        rows.with_layout(Layout::top_down(Align::Center), |ui| {
            ui.add_space(t.space.wide);
            ui.label(
                RichText::new(empty)
                    .font(t.text.body.clone())
                    .color(c.text.label),
            );
        });
    } else {
        ScrollArea::vertical()
            .id_salt("enth.browser.list")
            .auto_shrink([false, false])
            .show_rows(&mut rows, side, shown.len(), |ui, range| {
                for entry in &shown[range] {
                    let row = Row {
                        entry,
                        loaded: last == Some(&entry.key),
                        favorite: favorites.contains(&entry.key),
                        with_source,
                        holding,
                    };
                    row.show(ui, t, out);
                }
            });
    }

    // The footer: the last message, and the files the scan could not read.
    let problems = p.library.as_ref().map_or(0, |l| l.problems().len());
    let mut message_rect = footer;
    if problems > 0 {
        let text = match problems {
            1 => "1 file could not be read".to_owned(),
            n => format!("{n} files could not be read"),
        };
        let galley = ui
            .painter()
            .layout_no_wrap(text.clone(), t.text.label.clone(), c.text.label);
        let rect = Align2::RIGHT_CENTER.align_size_within_rect(galley.size(), footer);
        let list = p
            .library
            .as_ref()
            .map(|l| {
                l.problems()
                    .iter()
                    .map(|q| format!("{}: {}", q.key, q.message))
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default();
        let shown = ui
            .put(
                rect.expand2(vec2(t.space.tight, 0.0)),
                Label::new(
                    RichText::new(text)
                        .font(t.text.label.clone())
                        .color(c.text.label),
                )
                .sense(Sense::hover()),
            )
            .on_hover_text(list);
        message_rect.max.x = shown.rect.left() - t.space.section;
    }
    if let Some(message) = &p.message {
        let ink = if message.error {
            c.score.bad
        } else {
            c.text.primary
        };
        let text = match &message.hint {
            Some(hint) => format!("{} {} {hint}", message.text, glyph::BULLET),
            None => message.text.clone(),
        };
        ui.put(
            message_rect,
            Label::new(RichText::new(text).font(t.text.label.clone()).color(ink)).truncate(),
        );
    }
}

/// One preset's row.
#[allow(
    clippy::struct_excessive_bools,
    reason = "a row's read model: independent facts it draws"
)]
struct Row<'a> {
    entry: &'a PresetEntry,
    loaded: bool,
    favorite: bool,
    with_source: bool,
    holding: bool,
}

impl Row<'_> {
    fn show(&self, ui: &mut Ui, t: &Theme, out: &mut Intents) {
        let c = &t.color;
        let entry = self.entry;
        let key = &entry.key;
        let (rect, _) = ui.allocate_exact_size(
            vec2(ui.available_width(), t.size.control_height),
            Sense::hover(),
        );
        let id = row_id(key);
        let title = entry.title();
        // The row first, then its buttons on top of it (the later widget takes the click).
        let response = ui.interact(rect, id, Sense::click());
        response.widget_info(|| WidgetInfo::selected(Role::Button, true, self.loaded, title));
        if self.loaded {
            ui.painter()
                .rect_filled(rect, t.radius.small, c.accent.value_wash);
        } else if response.hovered() {
            ui.painter()
                .rect_filled(rect, t.radius.small, c.surface.raised);
        }
        let cols = Columns::of(t, rect);
        let (star, ink) = if self.favorite {
            (glyph::FAVORITE, c.accent.value)
        } else {
            (glyph::NOT_FAVORITE, c.text.label)
        };
        let starred = glyph_button(
            ui,
            t,
            cols.star,
            id.with("star"),
            star,
            ink,
            &format!("Favorite {title}"),
        );
        if starred.clicked() {
            out.push(Intent::Preset(PresetIntent::Favorite {
                key: key.clone(),
                on: !self.favorite,
            }));
        }
        let play = glyph_button(
            ui,
            t,
            cols.play,
            id.with("play"),
            glyph::PLAY,
            c.text.primary,
            &format!("Audition {title}"),
        )
        .on_hover_text("Hold to hear it (loads it)");
        if play.is_pointer_button_down_on() && !self.holding {
            out.push(Intent::Preset(PresetIntent::Audition(key.clone())));
        }
        text_left(ui, cols.name, title, &t.text.body, c.text.primary);
        text_left(
            ui,
            cols.place,
            &place(entry, self.with_source),
            &t.text.label,
            c.text.label,
        );
        text_left(
            ui,
            cols.tags,
            &entry.meta.tags.join(", "),
            &t.text.label,
            c.text.label,
        );
        text_left(
            ui,
            cols.author,
            &entry.meta.author,
            &t.text.label,
            c.text.label,
        );
        let description = entry.meta.description.trim();
        let response = if description.is_empty() {
            response
        } else {
            response.on_hover_text(description)
        };
        // A triple click counts too: egui takes a fast pair right after another click for one.
        if response.double_clicked() || response.triple_clicked() {
            out.push(Intent::Preset(PresetIntent::Load {
                key: key.clone(),
                close: true,
            }));
        } else if response.clicked() {
            out.push(Intent::Preset(PresetIntent::Load {
                key: key.clone(),
                close: false,
            }));
        }
    }
}
