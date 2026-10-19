//! Design tokens (docs/design-system.md): every colour, size, spacing, radius, stroke, font and
//! repaint rate in the editor comes from [`Theme`].
//!
//! Three tiers: private primitives (`primitives.rs`), then the public semantic and component
//! tokens that widgets read, one file per area so concurrent lanes never edit the same file:
//!
//! | File | Tokens |
//! | --- | --- |
//! | `colors.rs` | surfaces, lines, text, accents, category and score hues |
//! | `space.rs` | spacing, radius, stroke widths |
//! | `text.rs` | text styles |
//! | `sizes/` | per-widget dimensions, one file per UI area (`knob.rs`, `card.rs`, `editor.rs`, ...) |
//! | `states.rs` | disabled opacity |
//! | `motion.rs` | repaint rates, transition time |
//! | `glyph.rs` | symbols with a fixed meaning |
//!
//! Each file defines its tokens and builds their dark-theme values in its own `dark()`; [`Theme::dark`] only
//! composes them. [`Theme::apply`] hands the same tokens to egui's own style, so built-in widgets (buttons,
//! text fields, scroll bars) match ours.

mod colors;
mod fonts;
pub mod glyph;
pub mod hex;
mod motion;
mod primitives;
mod sizes;
mod space;
mod states;
mod text;

use egui::{Color32, Shadow, Style, TextStyle, ThemePreference, Visuals, style::WidgetVisuals};
use serde::{Deserialize, Serialize};

pub use colors::*;
pub use motion::*;
pub use sizes::*;
pub use space::*;
pub use states::*;
pub use text::*;

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

impl Default for Theme {
    fn default() -> Self {
        Self::dark()
    }
}

impl Theme {
    /// The dark theme, one area at a time: each token file builds its own values from the private primitives.
    pub fn dark() -> Self {
        Self {
            color: Colors::dark(),
            space: Space::dark(),
            radius: Radius::dark(),
            stroke: Strokes::dark(),
            text: TextStyles::dark(),
            size: Sizes::dark(),
            state: States::dark(),
            motion: Motion::dark(),
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
