//! The Save dialog (lane PBU), a card centred over the body: the name, a folder inside the user folder (made if new;
//! the factory presets are read-only, so the dialog never offers them), and tags from the vocabulary
//! (`enth_session::presets::tags`: one type, up to four character words; tags the patch already has outside the
//! vocabulary are kept and offered too). "Save preset" (or Enter in the name) saves through the session
//! (`SessionEdit::save_preset`, exactly `Patch::to_json()` with that name and tags). A name that exists is never
//! replaced silently: the dialog asks, and "Replace" saves again with `overwrite`. Any other refusal shows its message
//! and its one-line hint (`PresetError::hint`). Escape or Cancel closes it without saving.

use egui::{
    Align, Button, Frame, Id, Key, Layout, Rect, RichText, TextEdit, Ui, UiBuilder, pos2, vec2,
};
use enth_session::presets::tags;
use enth_session::{PresetKey, SaveRequest, SessionView, Source};

use super::{Message, PresetIntent};
use crate::Theme;
use crate::editor::cards::{Intent, Intents, ids};

/// The dialog's fields and state.
#[derive(Clone, Debug, Default)]
pub(super) struct SaveForm {
    pub(super) name: String,
    pub(super) folder: String,
    /// The chosen tags, in order.
    pub(super) tags: Vec<String>,
    /// The patch's tags outside the vocabulary, offered as chips beside it.
    others: Vec<String>,
    /// Where user presets go (`PresetStore::user_location`); `None`: there is no user folder.
    location: Option<String>,
    /// A save hit this existing preset: the dialog asks whether to replace it.
    pub(super) confirm: Option<PresetKey>,
    /// The last refusal, with its hint.
    pub(super) error: Option<Message>,
    /// It was opened from the browse panel, which comes back after a save.
    pub(super) from_browse: bool,
    /// Focus the name field on the first frame.
    focus: bool,
}

impl SaveForm {
    /// The dialog for `session`'s patch: its name and tags, and the folder of the user preset it came from.
    pub(super) fn open(
        session: &dyn SessionView,
        last: Option<PresetKey>,
        location: Option<String>,
        from_browse: bool,
    ) -> Self {
        let meta = &session.patch().meta;
        let folder = last
            .filter(|k| k.source == Source::User)
            .map(|k| k.folder().to_owned())
            .unwrap_or_default();
        let others = meta
            .tags
            .iter()
            .filter(|t| !tags::is_type(t) && !tags::is_character(t))
            .cloned()
            .collect();
        SaveForm {
            name: meta.name.clone(),
            folder,
            tags: meta.tags.clone(),
            others,
            location,
            confirm: None,
            error: None,
            from_browse,
            focus: true,
        }
    }

    /// What the session is asked to save.
    pub(super) fn request(&self, overwrite: bool) -> SaveRequest {
        SaveRequest {
            name: self.name.clone(),
            folder: self.folder.clone(),
            tags: self.tags.clone(),
            overwrite,
        }
    }

    fn has(&self, tag: &str) -> bool {
        self.tags.iter().any(|t| t.eq_ignore_ascii_case(tag))
    }

    fn remove(&mut self, tag: &str) {
        self.tags.retain(|t| !t.eq_ignore_ascii_case(tag));
    }

    /// A type chip: one type at a time; a second click clears it.
    fn toggle_type(&mut self, tag: &str) {
        let on = self.has(tag);
        self.tags.retain(|t| !tags::is_type(t));
        if !on {
            self.tags.push(tag.to_owned());
        }
    }

    /// A character or other chip.
    fn toggle(&mut self, tag: &str) {
        if self.has(tag) {
            self.remove(tag);
        } else {
            self.tags.push(tag.to_owned());
        }
    }

    fn characters(&self) -> usize {
        self.tags.iter().filter(|t| tags::is_character(t)).count()
    }
}

pub(super) fn show(form: &mut SaveForm, ui: &mut Ui, t: &Theme, body: Rect, out: &mut Intents) {
    let c = &t.color;
    let width = t.size.browser.dialog_width.min(body.width());
    let area = Rect::from_min_max(
        pos2(body.center().x - width / 2.0, body.top()),
        pos2(body.center().x + width / 2.0, body.bottom()),
    );
    let mut ui = ui.new_child(
        UiBuilder::new()
            .max_rect(area)
            .scope_id(ids::region("save"))
            .layout(Layout::top_down(Align::Min)),
    );
    ui.shrink_clip_rect(body);
    if ui.input(|i| i.key_pressed(Key::Escape)) {
        out.push(Intent::Preset(PresetIntent::Close));
    }
    let label = |text: &str| {
        RichText::new(text)
            .font(t.text.label.clone())
            .color(c.text.label)
    };
    let button = |text: &str| Button::new(RichText::new(text).font(t.text.body.clone()));
    let chip = |on: bool, text: &str| {
        Button::selectable(on, RichText::new(text).font(t.text.label.clone()))
    };
    Frame::new()
        .fill(c.surface.card)
        .corner_radius(t.radius.medium)
        .inner_margin(t.space.pad)
        .show(&mut ui, |ui| {
            ui.set_width(width - 2.0 * t.space.pad);
            ui.spacing_mut().item_spacing = vec2(t.space.tight, t.space.gap);
            ui.label(
                RichText::new("Save the sound as a preset")
                    .font(t.text.title.clone())
                    .color(c.text.primary),
            );
            match &form.location {
                Some(place) => ui.label(label(&format!("Into your presets: {place}"))),
                None => ui.label(
                    RichText::new("There is no folder for your presets: set ENTH_PRESETS_DIR")
                        .font(t.text.label.clone())
                        .color(c.score.bad),
                ),
            };

            ui.label(label("Name"));
            let name = ui
                .add(
                    TextEdit::singleline(&mut form.name)
                        .id(Id::unique("enth.save.name"))
                        .font(t.text.body.clone())
                        .desired_width(f32::INFINITY),
                )
                .accessible_name("Preset name");
            if form.focus {
                name.request_focus();
                form.focus = false;
            }
            let entered = name.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
            ui.label(label("Folder (optional; made if new)"));
            let folder = ui
                .add(
                    TextEdit::singleline(&mut form.folder)
                        .id(Id::unique("enth.save.folder"))
                        .hint_text("e.g. Leads")
                        .font(t.text.body.clone())
                        .desired_width(f32::INFINITY),
                )
                .accessible_name("Preset folder");
            if name.changed() || folder.changed() {
                form.confirm = None;
                form.error = None;
            }

            ui.label(label("Type"));
            ui.horizontal_wrapped(|ui| {
                for tag in tags::TYPES {
                    if ui.add(chip(form.has(tag), tag)).clicked() {
                        form.toggle_type(tag);
                    }
                }
            });
            ui.label(label(&format!("Character (up to {})", tags::MAX_CHARACTER)));
            ui.horizontal_wrapped(|ui| {
                let full = form.characters() >= tags::MAX_CHARACTER;
                for tag in tags::CHARACTER {
                    let on = form.has(tag);
                    if ui.add_enabled(on || !full, chip(on, tag)).clicked() {
                        form.toggle(tag);
                    }
                }
            });
            if !form.others.is_empty() {
                ui.label(label("Other tags"));
                let others = form.others.clone();
                ui.horizontal_wrapped(|ui| {
                    for tag in &others {
                        if ui.add(chip(form.has(tag), tag)).clicked() {
                            form.toggle(tag);
                        }
                    }
                });
            }

            if let Some(error) = &form.error {
                ui.label(
                    RichText::new(&error.text)
                        .font(t.text.label.clone())
                        .color(c.score.bad),
                );
                if let Some(hint) = &error.hint {
                    ui.label(label(hint));
                }
            }
            ui.add_space(t.space.tight);
            match form.confirm.clone() {
                Some(existing) => {
                    ui.label(
                        RichText::new(format!(
                            "\u{201c}{}\u{201d} is already in your presets. Replace it?",
                            existing.name()
                        ))
                        .font(t.text.body.clone())
                        .color(c.text.primary),
                    );
                    ui.horizontal(|ui| {
                        if ui.add(button("Replace")).clicked() {
                            out.push(Intent::Preset(PresetIntent::Save { overwrite: true }));
                        }
                        if ui.add(button("Cancel")).clicked() {
                            form.confirm = None;
                        }
                    });
                }
                None => {
                    ui.horizontal(|ui| {
                        let ready = !form.name.trim().is_empty();
                        if ui.add_enabled(ready, button("Save preset")).clicked()
                            || (entered && ready)
                        {
                            out.push(Intent::Preset(PresetIntent::Save { overwrite: false }));
                        }
                        if ui.add(button("Cancel")).clicked() {
                            out.push(Intent::Preset(PresetIntent::Close));
                        }
                    });
                }
            }
        });
}
