//! enth's editor design system (docs/design-system.md): the [`theme`] tokens and the
//! [`widgets`] built only from them. Widget code never contains a raw colour, size, font or
//! rate; the crate's `clippy.toml` bans egui's raw constructors outside `theme/`.

pub mod gallery;
pub mod theme;
pub mod widgets;

pub use theme::Theme;
