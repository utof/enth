//! **Owner: lane AM** (add and remove modules, the Init patch). Pre-declared by lane S0r (R1/S1).
//!
//! Adding and removing modules (report 03 §2.1, summary "Add/remove modules"). Every entry point pushes an intent
//! ([`Intent::AddModule`], [`Intent::RemoveModule`], [`Intent::MoveModule`], [`Intent::Duplicate`], [`Intent::Rename`],
//! [`Intent::MakeAmpEnv`]); the editor applies each as **one op and one undo step** through the session's structural
//! path (one `session.edit`, which recompiles the plan and swaps it), and a refusal (`too_many`,
//! `in_use_as_amp_env`) shows as the status line's hint. The entry points:
//!
//! - **the card's "…" menu** (a right-click on the card's title row opens it too): Add after ▸ (the kinds of the
//!   card's section), Duplicate, Move up and down (left and right in a band), Rename…, Make amp envelope (an
//!   envelope that is not the amplitude envelope), Remove;
//! - **a right-click on empty region space** opens the Add menu of the sections the layout places there;
//! - **the dashed "+ Add" slot** after a region's cards, drawn only where the region has room for it (it never makes
//!   a region scroll) and while one of its sections is under its limit; a click opens the same menu;
//! - **the header's "+ Add" menu** (the Add hook, feature `add-module`, on by default): every section, so a kind can
//!   be added in any layout, however full its regions are drawn.
//!
//! A full section's items are drawn disabled and say why. The status line names what an add or a removal did
//! ("Removed Filter 1, 2 routes and automation slot 3"; [`note`]). Sizes are tokens in `theme/sizes/add_module.rs`,
//! the two glyphs in `theme/glyph.rs`.
//!
//! The same lane gives a fresh plugin instance the Init patch (`content/presets/init.patch.json`, gain-staged; see
//! `enth-plugin`'s `init` module) and decided the minimum window (docs/editor.md, "Adding and removing modules").

use egui::text::CCursorRange;
use egui::{
    Area, Button, Frame, Id, Key, Order, Popup, Rect, Response, RichText, Sense, SetOpenCommand,
    Shape, TextEdit, Ui, pos2,
};
use enth_patch::{ModSource, ModuleId, ModuleKind, ParamAddr, Patch, Section};

use super::cards::{CardKind, CardView, Intent, Intents};
use super::layout::Part;
use crate::Theme;
use crate::theme::glyph;

/// Where a module's card sits: its section, its index there, and how many modules the section holds (the card's
/// menu: Move up and down, Add after, Duplicate).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Place {
    pub section: Section,
    pub index: usize,
    pub len: usize,
}

impl Place {
    /// The place of the module `id` in `patch`, if it is there.
    pub fn of(patch: &Patch, id: &ModuleId) -> Option<Place> {
        patch.find(id).map(|(at, _)| Place {
            section: at.section,
            index: at.index,
            len: patch.section(at.section).len(),
        })
    }

    /// Whether the section can take one more module.
    pub fn has_room(&self) -> bool {
        self.len < self.section.max_len()
    }
}

/// The sections an add entry point can add to, with how many modules each holds now, in section order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Room {
    pub sections: Vec<(Section, usize)>,
}

impl Room {
    /// The given sections of `patch` (a region's), in section order, each once.
    pub fn of(patch: &Patch, sections: impl IntoIterator<Item = Section>) -> Room {
        let mut sections: Vec<Section> = sections.into_iter().collect();
        sections.sort_unstable();
        sections.dedup();
        Room {
            sections: sections
                .into_iter()
                .map(|s| (s, patch.section(s).len()))
                .collect(),
        }
    }

    /// Every section of `patch` (the header's menu).
    pub fn all(patch: &Patch) -> Room {
        Room::of(patch, Section::ALL)
    }

    /// The sections of the parts a layout places in one region.
    pub fn of_parts(patch: &Patch, parts: impl IntoIterator<Item = Part>) -> Room {
        Room::of(patch, parts.into_iter().filter_map(section_of))
    }

    /// Whether any of the sections can take one more module.
    pub fn has_room(&self) -> bool {
        self.sections.iter().any(|&(s, len)| len < s.max_len())
    }

    /// Whether there is no section to add to (a region of built-ins only).
    pub fn is_empty(&self) -> bool {
        self.sections.is_empty()
    }
}

/// The section a layout part shows; `None` for the built-ins (`voice`, `master`).
pub fn section_of(part: Part) -> Option<Section> {
    match part {
        Part::Generators => Some(Section::Generators),
        Part::VoiceFx => Some(Section::VoiceFx),
        Part::Fx => Some(Section::Fx),
        Part::Modulators => Some(Section::Modulators),
        Part::Voice | Part::Master => None,
    }
}

/// The kinds a section holds, in catalogue order.
pub fn kinds(section: Section) -> impl Iterator<Item = ModuleKind> {
    ModuleKind::ALL
        .iter()
        .copied()
        .filter(move |k| k.section() == section)
}

/// A section's heading in the add menus.
pub fn section_name(section: Section) -> &'static str {
    match section {
        Section::Generators => "Sources",
        Section::VoiceFx => "Voice effects",
        Section::Fx => "Effects",
        Section::Modulators => "Modulators",
    }
}

/// One module of a section, for the "+ Add" slot's name and the refusals ("Add effect", "4 effects").
fn section_noun(section: Section) -> (&'static str, &'static str) {
    match section {
        Section::Generators => ("source", "sources"),
        Section::VoiceFx => ("voice effect", "voice effects"),
        Section::Fx => ("effect", "effects"),
        Section::Modulators => ("modulator", "modulators"),
    }
}

/// Why a full section's add items are disabled.
fn full(section: Section) -> String {
    format!(
        "A patch holds at most {} {}",
        section.max_len(),
        section_noun(section).1
    )
}

/// The add menu's items: a heading per section (when there are several), then a button per kind ("Oscillator",
/// "Filter", …) that appends it to its section. A full section's buttons are disabled and say why.
fn add_items(ui: &mut Ui, t: &Theme, room: &Room, out: &mut Intents) {
    ui.set_min_width(t.size.editor.entry_width);
    let headings = room.sections.len() > 1;
    for (n, &(section, len)) in room.sections.iter().enumerate() {
        if headings {
            if n > 0 {
                ui.separator();
            }
            ui.label(
                RichText::new(section_name(section))
                    .font(t.text.label.clone())
                    .color(t.color.text.label),
            );
        }
        let room_left = len < section.max_len();
        for kind in kinds(section) {
            let button = ui.add_enabled(room_left, Button::new(kind.spec().name));
            let button = button.on_disabled_hover_text(full(section));
            if button.clicked() {
                out.push(Intent::AddModule { kind, index: len });
                ui.close();
            }
        }
    }
}

/// The header's Add hook point (feature `add-module`, on by default): a "+ Add" menu button just left of Undo, with
/// every section ([`HeaderView::room`](super::regions::HeaderView::room)).
#[cfg(feature = "add-module")]
pub(crate) fn header(
    row: &mut Ui,
    theme: &Theme,
    _rect: Rect,
    view: &super::regions::HeaderView,
    out: &mut Intents,
) {
    let t = theme;
    let text = RichText::new(format!("{} Add", glyph::ADD)).font(t.text.body.clone());
    row.menu_button(text, |ui| add_items(ui, t, &view.room, out))
        .response
        .on_hover_text("Add a module");
}

/// A card's "…" menu: from its button (a click) and from its title row (a right-click). `along_x`: the card sits
/// in a band (left to right), so its moves read "left" and "right".
pub(crate) fn card_menu(
    t: &Theme,
    view: &CardView,
    button: Option<&Response>,
    header: &Response,
    along_x: bool,
    out: &mut Intents,
) {
    if view.place.is_none() {
        return;
    }
    if let Some(button) = button {
        Popup::menu(button).show(|ui| card_items(ui, t, view, along_x, out));
    }
    Popup::context_menu(header).show(|ui| card_items(ui, t, view, along_x, out));
}

/// The card menu's items (module docs).
fn card_items(ui: &mut Ui, t: &Theme, view: &CardView, along_x: bool, out: &mut Intents) {
    let (Some(place), super::cards::CardKey::Module(id), CardKind::Module(kind)) =
        (view.place, &view.key, view.kind)
    else {
        return;
    };
    ui.set_min_width(t.size.editor.entry_width);
    let room = place.has_room();
    let after = ui.add_enabled_ui(room, |ui| {
        ui.menu_button("Add after", |ui| {
            ui.set_min_width(t.size.editor.entry_width);
            for k in kinds(place.section) {
                if ui.button(k.spec().name).clicked() {
                    out.push(Intent::AddModule {
                        kind: k,
                        index: place.index + 1,
                    });
                    ui.close();
                }
            }
        })
    });
    after.response.on_disabled_hover_text(full(place.section));
    let duplicate = ui
        .add_enabled(room, Button::new("Duplicate"))
        .on_disabled_hover_text(full(place.section));
    if duplicate.clicked() {
        out.push(Intent::Duplicate(id.clone()));
        ui.close();
    }
    ui.separator();
    let (earlier, later) = if along_x {
        ("Move left", "Move right")
    } else {
        ("Move up", "Move down")
    };
    if ui
        .add_enabled(place.index > 0, Button::new(earlier))
        .clicked()
    {
        out.push(Intent::MoveModule {
            id: id.clone(),
            index: place.index - 1,
        });
        ui.close();
    }
    if ui
        .add_enabled(place.index + 1 < place.len, Button::new(later))
        .clicked()
    {
        out.push(Intent::MoveModule {
            id: id.clone(),
            index: place.index + 1,
        });
        ui.close();
    }
    if ui.button("Rename…").clicked() {
        start_rename(ui.ctx(), id, view);
        ui.close();
    }
    if kind == ModuleKind::Adsr && !view.amp_env {
        let enabled = view.enabled.unwrap_or(true);
        let make = ui
            .add_enabled(enabled, Button::new("Make amp envelope"))
            .on_hover_text("This envelope shapes every voice's volume")
            .on_disabled_hover_text("Switch it on first");
        if make.clicked() {
            out.push(Intent::MakeAmpEnv(id.clone()));
            ui.close();
        }
    }
    ui.separator();
    let remove = ui
        .add_enabled(!view.amp_env, Button::new("Remove"))
        .on_disabled_hover_text(
            "It is the amp envelope: make another envelope the amp envelope first",
        );
    if remove.clicked() {
        out.push(Intent::RemoveModule(id.clone()));
        ui.close();
    }
}

/// Right-click on a card region's empty space: the region's Add menu. `background` is the region's own response,
/// registered under its cards (so they and their controls take their own clicks); `over_card` says the click landed
/// on a card's body, which is not empty space.
pub(crate) fn region_menu(
    t: &Theme,
    background: &Response,
    over_card: bool,
    room: &Room,
    out: &mut Intents,
) {
    let open = if background.secondary_clicked() && !over_card {
        Some(SetOpenCommand::Bool(true))
    } else if background.clicked() {
        Some(SetOpenCommand::Bool(false))
    } else {
        None
    };
    Popup::menu(background)
        .open_memory(open)
        .at_pointer_fixed()
        .show(|ui| add_items(ui, t, room, out));
}

/// The dashed "+ Add" slot in `rect`: a flat outline (no fill) with the glyph and "Add" in its middle. A click opens
/// the add menu of `room`. Its accessible name says what it adds ("Add effect").
pub(crate) fn slot(ui: &mut Ui, t: &Theme, rect: Rect, id: Id, room: &Room, out: &mut Intents) {
    let response = ui.interact(rect, id, Sense::click());
    let name = match room.sections.as_slice() {
        [(section, _)] => format!("Add {}", section_noun(*section).0),
        _ => "Add module".to_owned(),
    };
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &name));
    let ink = if response.hovered() {
        t.color.text.primary
    } else {
        t.color.text.label
    };
    let stroke = t.stroke.thin(if response.hovered() {
        t.color.text.label
    } else {
        t.color.line.grid
    });
    let r = rect.shrink(t.stroke.thin * 0.5);
    let outline = [
        r.left_top(),
        r.right_top(),
        r.right_bottom(),
        r.left_bottom(),
        r.left_top(),
    ];
    let a = &t.size.add_module;
    ui.painter()
        .extend(Shape::dashed_line(&outline, stroke, a.dash, a.dash_gap));
    let text = if rect.width() < rect.height() {
        glyph::ADD.to_owned()
    } else {
        format!("{} Add", glyph::ADD)
    };
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        text,
        t.text.body.clone(),
        ink,
    );
    Popup::menu(&response).show(|ui| add_items(ui, t, room, out));
}

/// The open "Rename…" field (one at a time).
#[derive(Clone, Debug)]
struct Renaming {
    id: ModuleId,
    text: String,
    focused: bool,
    /// The last frame the field was drawn; a field whose card stopped drawing is dropped.
    frame: u64,
}

fn rename_id() -> Id {
    Id::unique("enth.rename")
}

fn start_rename(ctx: &egui::Context, id: &ModuleId, view: &CardView) {
    let renaming = Renaming {
        id: id.clone(),
        text: view.title.clone(),
        focused: false,
        frame: ctx.cumulative_frame_nr(),
    };
    ctx.data_mut(|d| d.insert_temp(rename_id(), renaming));
}

/// Draws the "Rename…" field over the card's title row (`anchor`) while it is open for this card. Enter pushes one
/// [`Intent::Rename`]: the text, or no label (the card's kind and number) when it is empty. Escape, or a click
/// elsewhere, cancels. The field is named "<title> name" for accessibility.
pub(crate) fn rename_field(ui: &Ui, t: &Theme, view: &CardView, anchor: Rect, out: &mut Intents) {
    let super::cards::CardKey::Module(id) = &view.key else {
        return;
    };
    let ctx = ui.ctx().clone();
    let Some(mut renaming) = ctx.data(|d| d.get_temp::<Renaming>(rename_id())) else {
        return;
    };
    if &renaming.id != id {
        return;
    }
    let frame = ctx.cumulative_frame_nr();
    if frame > renaming.frame + 1 {
        ctx.data_mut(|d| d.remove::<Renaming>(rename_id()));
        return;
    }
    renaming.frame = frame;
    let edit_id = rename_id().with("edit");
    let mut open = true;
    Area::new(rename_id().with("area"))
        .order(Order::Foreground)
        .fixed_pos(pos2(anchor.left(), anchor.top()))
        .show(&ctx, |ui| {
            Frame::popup(ui.style())
                .inner_margin(t.space.gap)
                .show(ui, |ui| {
                    let mut output = TextEdit::singleline(&mut renaming.text)
                        .id(edit_id)
                        .font(t.text.body.clone())
                        .desired_width(t.size.editor.entry_hint_width)
                        .show(ui);
                    let response = output
                        .response
                        .response
                        .clone()
                        .accessible_name(format!("{} name", view.title));
                    if !renaming.focused {
                        response.request_focus();
                        output
                            .state
                            .cursor
                            .set_char_range(Some(CCursorRange::select_all_str(&renaming.text)));
                        output.state.store(ui.ctx(), edit_id);
                        renaming.focused = true;
                    } else if response.lost_focus() {
                        if ui.input(|i| i.key_pressed(Key::Enter)) {
                            let text = renaming.text.trim();
                            out.push(Intent::Rename {
                                id: id.clone(),
                                label: (!text.is_empty()).then(|| text.to_owned()),
                            });
                        }
                        open = false;
                    }
                });
        });
    ctx.data_mut(|d| {
        if open {
            d.insert_temp(rename_id(), renaming);
        } else {
            d.remove::<Renaming>(rename_id());
        }
    });
}

/// What an add, a duplicate or a removal did, for the status line ("Added Osc 3", "Removed Filter 1, 2 routes and
/// automation slot 3"), from the patch before it; `None` for the other intents.
pub fn note(before: &Patch, intent: &Intent) -> Option<String> {
    let title = super::cards::view::title;
    match intent {
        Intent::AddModule { kind, .. } => {
            Some(format!("Added {}", title(*kind, &before.free_id(*kind))))
        }
        Intent::Duplicate(id) => {
            let (_, m) = before.find(id)?;
            Some(format!(
                "Added {}, a copy of {}",
                title(m.kind, &before.free_id(m.kind)),
                super::cards::view::source_name(before, &ModSource::Module(id.clone()))
            ))
        }
        Intent::RemoveModule(id) => {
            let name = super::cards::view::source_name(before, &ModSource::Module(id.clone()));
            let routes = before
                .routes
                .iter()
                .filter(|r| r.dst.module_id() == Some(id) || r.src == ModSource::Module(id.clone()))
                .count();
            let slots: Vec<String> = before
                .slots
                .iter()
                .enumerate()
                .filter(|(_, s)| s.as_ref().and_then(ParamAddr::module_id) == Some(id))
                .map(|(n, _)| (n + 1).to_string())
                .collect();
            let mut parts = Vec::new();
            match routes {
                0 => {}
                1 => parts.push("1 route".to_owned()),
                n => parts.push(format!("{n} routes")),
            }
            match slots.as_slice() {
                [] => {}
                [one] => parts.push(format!("automation slot {one}")),
                [rest @ .., last] => {
                    parts.push(format!("automation slots {} and {last}", rest.join(", ")));
                }
            }
            Some(match parts.as_slice() {
                [] => format!("Removed {name}"),
                [one] => format!("Removed {name} and {one}"),
                [a, b] => format!("Removed {name}, {a} and {b}"),
                _ => unreachable!("at most two parts"),
            })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_full_section_has_no_room() {
        let mut p = Patch::init();
        assert!(Room::all(&p).has_room());
        for _ in 0..3 {
            let id = p.free_id(ModuleKind::Osc);
            p.generators
                .push(enth_patch::ModuleInstance::new(id, ModuleKind::Osc));
        }
        let gens = Room::of(&p, [Section::Generators]);
        assert_eq!(gens.sections, vec![(Section::Generators, 4)]);
        assert!(!gens.has_room());
        assert!(Room::of_parts(&p, [Part::Voice, Part::Master]).is_empty());
        assert_eq!(
            Room::of_parts(&p, [Part::Fx, Part::Voice]).sections,
            vec![(Section::Fx, 0)]
        );
    }

    #[test]
    fn every_kind_is_offered_in_its_own_section() {
        let offered: Vec<ModuleKind> = Section::ALL.into_iter().flat_map(kinds).collect();
        assert_eq!(offered.len(), ModuleKind::ALL.len());
        for kind in offered {
            assert!(kinds(kind.section()).any(|k| k == kind));
        }
    }

    #[test]
    fn notes_name_what_an_add_or_a_removal_did() {
        let mut p = Patch::init();
        let filter = ModuleId::new("filter1").unwrap();
        p.voice_fx.push(enth_patch::ModuleInstance::new(
            filter.clone(),
            ModuleKind::Svf,
        ));
        let cutoff: ParamAddr = "filter1.cutoff".parse().unwrap();
        p.routes.push(enth_patch::Route {
            src: ModSource::Velocity,
            dst: cutoff.clone(),
            depth: 1.0,
        });
        p.routes.push(enth_patch::Route {
            src: ModSource::Key,
            dst: cutoff.clone(),
            depth: 1.0,
        });
        p.slots[2] = Some(cutoff);
        assert_eq!(
            note(&p, &Intent::RemoveModule(filter.clone())).as_deref(),
            Some("Removed Filter 1, 2 routes and automation slot 3")
        );
        assert_eq!(
            note(
                &p,
                &Intent::AddModule {
                    kind: ModuleKind::Osc,
                    index: 1
                }
            )
            .as_deref(),
            Some("Added Osc 2")
        );
        assert_eq!(
            note(&p, &Intent::Duplicate(filter)).as_deref(),
            Some("Added Filter 2, a copy of Filter 1")
        );
        assert_eq!(
            note(
                &Patch::init(),
                &Intent::RemoveModule(ModuleId::new("osc1").unwrap())
            )
            .as_deref(),
            Some("Removed Osc 1")
        );
    }
}
