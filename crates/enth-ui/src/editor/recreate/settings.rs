//! **Owner: lane RUI** (the Recreate UI).
//!
//! The randomizer settings page (report 01 §2.6, design.md §9): the difficulty, an amount per category, how musical
//! (and so how safe) the draws are, the template, and the `dim_unused` assist. It only produces a
//! [`RandomSpec`], which is integers only (`enth_challenge::spec`): [`Settings`] holds exactly the spec's fields
//! but the generator and the seed, which the shell supplies when the round starts (a fresh 40-bit seed, so a level's
//! code stays short). Settings that are a level's preset give a short code; any other gives the long form once the
//! target exists (docs/challenge.md, "Codes v1").
//!
//! The page is a sheet over the body, like the preset browser's: while it is open the body's regions are not drawn.

use egui::{
    Align, Button, Checkbox, ComboBox, Layout, Rect, RichText, Slider, Ui, UiBuilder, vec2,
};
use enth_challenge::RandomSpec;
use enth_challenge::spec::{Category, LATEST_GENERATOR, LATEST_TEMPLATE, PERCENT_MAX, SpecPreset};

use super::{GameIntent, Recreate};
use crate::Theme;
use crate::editor::cards::{Intent, Intents};
use crate::theme::glyph;

/// The levels a short code carries (code v1's presets, 1–5).
pub const LEVELS: u8 = 5;

/// The randomizer's settings without the generator and the seed: everything the page edits. Integers only, like
/// the spec it makes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Settings {
    /// 0–100: picks the level (0–20 → 1 … 81–100 → 5).
    pub difficulty: u8,
    /// An amount per category (0–100), indexed by [`Category::index`]; `None` follows the difficulty.
    pub amounts: [Option<u8>; Category::COUNT],
    /// 0–100: 100 draws from the musical priors only (and above 20 within the safe ranges), 0 uniformly.
    pub musical: u8,
    /// The archetype (0: the starter superset, the only one in this build).
    pub template: u8,
}

impl Default for Settings {
    /// Level 1.
    fn default() -> Self {
        Settings::level(1)
    }
}

impl Settings {
    /// A level's preset (1–5; anything else is level 1).
    pub fn level(level: u8) -> Settings {
        let preset = SpecPreset::by_id(level.clamp(1, LEVELS))
            .or_else(|| SpecPreset::by_id(1))
            .unwrap_or_else(|| unreachable!("code v1 has five presets"));
        Settings::of(&preset.spec(LATEST_GENERATOR, 0))
    }

    /// The settings of a spec (its generator and seed dropped).
    pub fn of(spec: &RandomSpec) -> Settings {
        Settings {
            difficulty: spec.difficulty,
            amounts: spec.amounts,
            musical: spec.musical,
            template: spec.template,
        }
    }

    /// The spec these settings make with `seed`, for this build's newest generator.
    pub fn spec(&self, seed: u64) -> RandomSpec {
        RandomSpec {
            generator: LATEST_GENERATOR,
            seed,
            difficulty: self.difficulty,
            amounts: self.amounts,
            musical: self.musical,
            template: self.template,
        }
    }

    /// The level the difficulty picks, 1–5 (lane RG's banding: 0–20, 21–40, 41–60, 61–80, 81–100).
    pub fn level_of(difficulty: u8) -> u8 {
        match difficulty {
            0..=20 => 1,
            21..=40 => 2,
            41..=60 => 3,
            61..=80 => 4,
            _ => LEVELS,
        }
    }

    /// The level these settings play at.
    pub fn current_level(&self) -> u8 {
        Settings::level_of(self.difficulty)
    }

    /// The preset these settings are, if any (they then give a short code).
    pub fn preset(&self) -> Option<u8> {
        self.spec(0).preset().map(|p| p.id)
    }
}

/// The category's name on the page.
pub fn category_name(c: Category) -> &'static str {
    match c {
        Category::Oscillators => "Oscillators",
        Category::Filter => "Filter",
        Category::Envelopes => "Envelopes",
        Category::Modulation => "Modulation",
        Category::Effects => "Effects",
        Category::Structure => "Structure",
    }
}

/// The template's name on the page.
pub fn template_name(template: u8) -> &'static str {
    match template {
        0 => "Starter (every module)",
        _ => "From a newer build",
    }
}

/// Draws the settings page over `body`: the settings are edited in place; Start, Close and the reset push intents
/// or change the page's state.
pub(crate) fn show(r: &mut Recreate, ui: &mut Ui, t: &Theme, body: Rect, out: &mut Intents) {
    let c = &t.color;
    let s = &t.size.recreate;
    let card = Rect::from_center_size(
        body.center(),
        vec2(s.settings_width.min(body.width()), body.height()),
    );
    let painter = ui.painter_at(body);
    painter.rect_filled(body, t.radius.medium, c.surface.background);
    painter.rect_filled(card, t.radius.medium, c.surface.card);
    let inner = card.shrink(t.space.section);
    let mut page = ui.new_child(
        UiBuilder::new()
            .max_rect(inner)
            .id_salt("enth.recreate.settings")
            .layout(Layout::top_down(Align::Min)),
    );
    page.set_clip_rect(card);
    page.spacing_mut().item_spacing = vec2(t.space.gap, t.space.gap);
    let label = |text: &str| {
        RichText::new(text)
            .font(t.text.body.clone())
            .color(c.text.label)
    };
    let note = |text: &str| {
        RichText::new(text)
            .font(t.text.label.clone())
            .color(c.text.dimmed)
    };

    // The title row, with × at its right end.
    page.horizontal(|ui| {
        ui.label(
            RichText::new("Recreate settings")
                .font(t.text.title.clone())
                .color(c.text.primary),
        );
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let close = ui
                .add(
                    Button::new(RichText::new(glyph::CLOSE).font(t.text.body.clone())).frame(false),
                )
                .on_hover_text("Close the settings (Escape)");
            close.widget_info(|| {
                egui::WidgetInfo::labeled(egui::Role::Button, true, "Close settings")
            });
            if close.clicked() {
                r.page = false;
            }
        });
    });
    page.label(note(
        "A round's target is drawn from these settings and a fresh seed. A level's settings give a short code to \
         share; any other settings give a longer one.",
    ));

    let s = &mut r.settings;
    let row = |ui: &mut Ui, name: &str, add: &mut dyn FnMut(&mut Ui)| {
        ui.horizontal(|ui| {
            let size = vec2(t.size.recreate.settings_label, t.size.control_height);
            ui.allocate_ui_with_layout(size, Layout::left_to_right(Align::Center), |ui| {
                ui.set_min_size(size);
                ui.label(label(name));
            });
            add(ui);
        });
    };

    // The level: a level's preset at once, or any difficulty in between.
    row(&mut page, "Level", &mut |ui| {
        for level in 1..=LEVELS {
            let preset = Settings::level(level);
            let on = s.preset() == Some(level);
            let b = ui
                .add(
                    Button::new(RichText::new(level.to_string()).font(t.text.body.clone()))
                        .selected(on),
                )
                .on_hover_text(format!("Level {level}'s settings: a short code"));
            b.widget_info(|| {
                egui::WidgetInfo::selected(
                    egui::Role::Button,
                    true,
                    on,
                    format!("Settings level {level}"),
                )
            });
            if b.clicked() {
                *s = preset;
            }
        }
    });
    row(&mut page, "Difficulty", &mut |ui| {
        ui.spacing_mut().slider_width = t.size.recreate.settings_slider;
        let level = Settings::level_of(s.difficulty);
        ui.add(Slider::new(&mut s.difficulty, 0..=PERCENT_MAX).text(format!("level {level}")))
            .on_hover_text("Each level adds what it changes: oscillators, the filter, envelopes, modulation, effects and structure");
    });

    page.label(label("What changes"));
    for category in Category::ALL {
        let i = category.index();
        row(&mut page, category_name(category), &mut |ui| {
            let mut auto = s.amounts[i].is_none();
            let name = category_name(category);
            let check = ui.add(Checkbox::new(&mut auto, label("Auto")));
            check.widget_info(|| {
                egui::WidgetInfo::selected(
                    egui::Role::CheckBox,
                    true,
                    auto,
                    format!("{name} follows the level"),
                )
            });
            if check.changed() {
                s.amounts[i] = if auto {
                    None
                } else {
                    Some(default_amount(category, s.difficulty))
                };
            }
            let mut amount = s.amounts[i].unwrap_or_else(|| default_amount(category, s.difficulty));
            ui.spacing_mut().slider_width = t.size.recreate.settings_slider;
            let slider = ui
                .add_enabled(
                    !auto,
                    Slider::new(&mut amount, 0..=PERCENT_MAX).suffix(" %"),
                )
                .on_hover_text("0: never changes; 100: changes the most it can");
            slider.widget_info(|| {
                egui::WidgetInfo::slider(!auto, f64::from(amount), format!("{name} amount"))
            });
            if slider.changed() && !auto {
                s.amounts[i] = Some(amount);
            }
        });
    }
    row(&mut page, "Musical", &mut |ui| {
        ui.spacing_mut().slider_width = t.size.recreate.settings_slider;
        ui.add(Slider::new(&mut s.musical, 0..=PERCENT_MAX).suffix(" %"))
            .on_hover_text("100: musical choices only, in safe ranges; 0: anything the module can do (loud, harsh or silent sounds happen)");
    });
    row(&mut page, "Template", &mut |ui| {
        let combo = ComboBox::from_id_salt("enth.recreate.template")
            .selected_text(template_name(s.template))
            .show_ui(ui, |ui| {
                for template in 0..=LATEST_TEMPLATE {
                    ui.selectable_value(&mut s.template, template, template_name(template));
                }
            });
        combo
            .response
            .widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, true, "Template"));
    });
    row(&mut page, "Assist", &mut |ui| {
        let check = ui.add(Checkbox::new(
            &mut r.dim_unused,
            label("Dim the modules the target leaves alone"),
        ));
        check.widget_info(|| {
            egui::WidgetInfo::selected(
                egui::Role::CheckBox,
                true,
                r.dim_unused,
                "Dim unused modules",
            )
        });
    });

    // The footer: what code these settings make, Start and Close.
    let s = &r.settings;
    let code = match s.preset() {
        Some(level) => format!("Level {level}: a short code"),
        None => "Custom settings: a long code (it carries the target's fingerprint)".to_owned(),
    };
    page.add_space(t.space.gap);
    page.horizontal(|ui| {
        ui.label(note(&code));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let start = ui.add(Button::new(
                RichText::new("Start round").font(t.text.strong.clone()),
            ));
            if start.clicked() {
                out.push(Intent::Game(GameIntent::New(s.spec(0))));
                r.page = false;
            }
        });
    });
    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        r.page = false;
    }
}

/// What an amount reads while the category follows the level: 100 when the level unlocks it, else 0.
fn default_amount(category: Category, difficulty: u8) -> u8 {
    if category.index() < usize::from(Settings::level_of(difficulty)) {
        PERCENT_MAX
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_level_is_its_preset_and_gives_a_short_code() {
        for level in 1..=LEVELS {
            let s = Settings::level(level);
            assert_eq!(s.preset(), Some(level));
            assert_eq!(s.current_level(), level);
            let spec = s.spec(42);
            assert!(spec.validate().is_ok());
            assert!(enth_challenge::ChallengeCode::short(spec).is_some());
        }
    }

    #[test]
    fn custom_settings_are_integers_and_valid_but_no_preset() {
        let mut s = Settings::level(3);
        s.amounts[Category::Filter.index()] = Some(40);
        s.musical = 60;
        assert_eq!(s.preset(), None);
        let spec = s.spec(7);
        assert!(spec.validate().is_ok());
        assert!(enth_challenge::ChallengeCode::short(spec).is_none());
        assert_eq!(
            Settings::of(&spec),
            s,
            "the page makes exactly the spec it shows"
        );
        let json = serde_json::to_string(&spec).unwrap();
        assert!(!json.contains('.'), "integers only: {json}");
    }

    #[test]
    fn levels_follow_lane_rgs_banding() {
        let bands = [
            (0, 1),
            (20, 1),
            (21, 2),
            (40, 2),
            (41, 3),
            (60, 3),
            (61, 4),
            (80, 4),
            (81, 5),
            (100, 5),
        ];
        for (difficulty, level) in bands {
            assert_eq!(
                Settings::level_of(difficulty),
                level,
                "difficulty {difficulty}"
            );
        }
    }
}
