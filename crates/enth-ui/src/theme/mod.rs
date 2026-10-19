//! Design tokens (docs/design-system.md): every colour, size, spacing, radius, stroke, font and
//! repaint rate in the editor comes from [`Theme`].
//!
//! Three tiers: private primitives (`primitives.rs`), then the public semantic and component
//! tokens (`tokens.rs`) that widgets read. [`Theme::apply`] hands the same tokens to egui's own
//! style, so built-in widgets (buttons, text fields, scroll bars) match ours.

mod fonts;
pub mod hex;
mod primitives;
mod tokens;

use egui::{Color32, Shadow, Style, TextStyle, ThemePreference, Visuals, style::WidgetVisuals};
use serde::{Deserialize, Serialize};

pub use tokens::*;

use primitives as p;

/// All tokens of one theme. Serialisable, so a dev-time override file or a light theme needs
/// no widget changes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Theme {
    pub color: Colors,
    pub space: Space,
    pub radius: Radius,
    pub stroke: Strokes,
    pub text: TextStyles,
    pub size: Sizes,
    #[serde(default)]
    pub state: States,
    pub motion: Motion,
}

/// Module categories; each has a stripe colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Category {
    Generator,
    Filter,
    Modulator,
    Effect,
}

/// Challenge feedback on a control.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Score {
    Good,
    Near,
    Bad,
}

/// Symbols with a fixed meaning. All are covered by Inter (checked by a test).
pub mod glyph {
    /// Score: correct.
    pub const GOOD: &str = "✓";
    /// Score: close.
    pub const NEAR: &str = "~";
    /// Score: wrong.
    pub const BAD: &str = "✗";
    /// Drag handle of a modulation source.
    pub const HANDLE: &str = "⊕";
    /// Play / audition.
    pub const PLAY: &str = "▶";
    /// Separator in a parameter path ("Filter › Cutoff").
    pub const PATH: &str = "›";
    /// Flow to the next stage ("Send to › Master").
    pub const SEND: &str = "→";
    /// Bullet in lists.
    pub const BULLET: &str = "•";
    /// Filled and empty dots (difficulty).
    pub const DOT_ON: &str = "●";
    pub const DOT_OFF: &str = "○";
}

impl Categories {
    pub fn of(&self, category: Category) -> Color32 {
        match category {
            Category::Generator => self.generator,
            Category::Filter => self.filter,
            Category::Modulator => self.modulator,
            Category::Effect => self.effect,
        }
    }
}

impl Scores {
    pub fn of(&self, score: Score) -> Color32 {
        match score {
            Score::Good => self.good,
            Score::Near => self.near,
            Score::Bad => self.bad,
        }
    }
}

impl Score {
    pub fn glyph(self) -> &'static str {
        match self {
            Score::Good => glyph::GOOD,
            Score::Near => glyph::NEAR,
            Score::Bad => glyph::BAD,
        }
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::dark()
    }
}

impl Theme {
    /// The dark theme, built from the private primitives.
    pub fn dark() -> Self {
        Self {
            color: Colors {
                surface: Surfaces {
                    background: p::GRAY_900,
                    card: p::GRAY_850,
                    raised: p::GRAY_800,
                    recessed: p::GRAY_950,
                },
                line: Lines { grid: p::GRAY_825 },
                text: TextColors {
                    primary: p::GRAY_100,
                    label: p::GRAY_300,
                    dimmed: p::GRAY_500,
                },
                accent: Accents {
                    value: p::BLUE_400,
                    value_wash: p::BLUE_WASH,
                    modulation: p::ORANGE_400,
                    modulation_wash: p::ORANGE_WASH,
                },
                category: Categories {
                    generator: p::TEAL_400,
                    filter: p::YELLOW_500,
                    modulator: p::SLATE_400,
                    effect: p::PURPLE_400,
                },
                score: Scores {
                    good: p::GREEN_400,
                    near: p::AMBER_300,
                    bad: p::RED_400,
                },
            },
            space: Space {
                hairline: p::SPACE_0,
                tight: p::SPACE_1,
                gap: p::SPACE_2,
                pad: p::SPACE_3,
                section: p::SPACE_4,
                wide: p::SPACE_5,
                band: p::SPACE_6,
            },
            radius: Radius {
                small: p::RADIUS_SMALL,
                medium: p::RADIUS_MEDIUM,
            },
            stroke: Strokes {
                thin: p::STROKE_THIN,
                thick: p::STROKE_THICK,
            },
            text: TextStyles {
                label: fonts::regular(p::TEXT_SMALL),
                body: fonts::regular(p::TEXT_MEDIUM),
                value: fonts::tabular(p::TEXT_MEDIUM),
                title: fonts::semibold(p::TEXT_LARGE),
                strong: fonts::semibold(p::TEXT_MEDIUM),
            },
            size: Sizes {
                window: WindowSizes {
                    default: egui::vec2(1200.0, 800.0),
                    min: egui::vec2(1000.0, 680.0),
                },
                bar: BarSizes {
                    header: 40.0,
                    macros: 56.0,
                    modulator_strip: 140.0,
                    status: 22.0,
                },
                card: CardSizes {
                    generator_height: 96.0,
                    collapsed_height: 24.0,
                    lane_width: 220.0,
                    display_width: 128.0,
                    lane_display_height: 40.0,
                    toggle: 12.0,
                },
                knob: KnobSizes {
                    diameter: 36.0,
                    track: p::STROKE_THICK,
                    cell_width: 56.0,
                    drag_travel: 200.0,
                    fine_ratio: 0.1,
                    sweep_degrees: 270.0,
                },
                tile: TileSizes {
                    width: 72.0,
                    height: 56.0,
                    badge: 14.0,
                },
                matrix: MatrixSizes {
                    row_height: 16.0,
                    target_width: 88.0,
                    cell_width: 36.0,
                },
                pane: PaneSizes {
                    lesson_width: 320.0,
                },
                display: DisplaySizes {
                    grid_columns: 4,
                    grid_rows: 2,
                    marker_radius: 3.0,
                },
                control_height: 24.0,
            },
            state: States {
                disabled_opacity: 0.4,
            },
            motion: Motion {
                animated_hz: p::RATE_ANIMATED,
                scope_hz: p::RATE_SCOPE,
                transition: 0.1,
            },
        }
    }

    /// Called once when the editor opens: installs the fonts and sets egui's style and visuals
    /// (for both of egui's dark and light slots) from the tokens.
    pub fn apply(&self, ctx: &egui::Context) {
        ctx.set_fonts(fonts::definitions());
        let style = self.egui_style();
        ctx.set_style_of(egui::Theme::Dark, style.clone());
        ctx.set_style_of(egui::Theme::Light, style);
        ctx.set_theme(ThemePreference::Dark);
    }

    /// egui's [`Style`] expressed in our tokens.
    pub fn egui_style(&self) -> Style {
        let mut style = Style {
            text_styles: [
                (TextStyle::Small, self.text.label.clone()),
                (TextStyle::Body, self.text.body.clone()),
                (TextStyle::Button, self.text.body.clone()),
                (TextStyle::Heading, self.text.title.clone()),
                (TextStyle::Monospace, self.text.value.clone()),
            ]
            .into(),
            drag_value_text_style: TextStyle::Monospace,
            animation_time: self.motion.transition,
            visuals: self.egui_visuals(),
            ..Style::default()
        };
        let s = &mut style.spacing;
        s.item_spacing = egui::vec2(self.space.gap, self.space.tight);
        s.button_padding = egui::vec2(self.space.gap, self.space.tight);
        s.interact_size = egui::vec2(self.size.knob.cell_width, self.size.control_height);
        s.window_margin = self.space.pad.into();
        s.menu_margin = self.space.gap.into();
        s.indent = self.space.section;
        s.icon_width = self.size.card.toggle;
        s.icon_width_inner = self.size.card.toggle - 2.0 * self.space.hairline;
        s.icon_spacing = self.space.tight;
        s.scroll = egui::style::ScrollStyle {
            bar_width: self.space.tight,
            floating_width: self.space.hairline,
            floating_allocated_width: 0.0,
            bar_inner_margin: self.space.hairline,
            bar_outer_margin: self.space.hairline,
            ..egui::style::ScrollStyle::floating()
        };
        style
    }

    fn egui_visuals(&self) -> Visuals {
        let c = &self.color;
        let widget = |bg: Color32, fg: Color32, outline: Option<Color32>| WidgetVisuals {
            bg_fill: bg,
            weak_bg_fill: bg,
            bg_stroke: outline.map_or(egui::Stroke::NONE, |o| self.stroke.thin(o)),
            corner_radius: self.radius.medium.into(),
            fg_stroke: self.stroke.thin(fg),
            expansion: 0.0,
        };
        let mut v = Visuals::dark();
        v.widgets.noninteractive = widget(c.surface.card, c.text.label, Some(c.line.grid));
        v.widgets.inactive = widget(c.surface.raised, c.text.primary, None);
        v.widgets.hovered = widget(c.surface.raised, c.text.primary, Some(c.line.grid));
        v.widgets.active = widget(c.surface.raised, c.text.primary, Some(c.accent.value));
        v.widgets.open = widget(c.surface.raised, c.text.primary, Some(c.accent.value));
        v.selection.bg_fill = c.accent.value_wash;
        v.selection.stroke = self.stroke.thin(c.accent.value);
        v.text_cursor.stroke = self.stroke.thin(c.accent.value);
        v.hyperlink_color = c.accent.value;
        v.weak_text_color = Some(c.text.dimmed);
        v.disabled_alpha = self.state.disabled_opacity;
        v.faint_bg_color = c.surface.card;
        v.extreme_bg_color = c.surface.recessed;
        v.text_edit_bg_color = Some(c.surface.recessed);
        v.code_bg_color = c.surface.recessed;
        v.warn_fg_color = c.score.near;
        v.error_fg_color = c.score.bad;
        v.panel_fill = c.surface.background;
        v.window_fill = c.surface.raised;
        v.window_stroke = self.stroke.thin(c.line.grid);
        v.window_corner_radius = self.radius.medium.into();
        v.menu_corner_radius = self.radius.medium.into();
        // Flat: no shadows anywhere.
        v.window_shadow = Shadow::NONE;
        v.popup_shadow = Shadow::NONE;
        v.resize_corner_size = self.space.pad;
        v.collapsing_header_frame = false;
        v.indent_has_left_vline = false;
        v.striped = false;
        v
    }
}
