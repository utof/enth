//! The token gallery snapshot (docs/design-system.md): every colour, text style, scale and
//! component in one PNG. A token change shows up as an image diff.
//! Update after a deliberate change: `UPDATE_SNAPSHOTS=1 cargo test -p enth-ui --test token_gallery`.

mod common;

use enth_ui::Theme;
use enth_ui::gallery::{GALLERY_SIZE, GalleryState, token_gallery};

#[test]
fn token_gallery_snapshot() {
    let theme = Theme::dark();
    let ui_theme = theme.clone();
    let mut harness = common::harness(
        &theme,
        GALLERY_SIZE,
        move |ui, state: &mut GalleryState| token_gallery(ui, &ui_theme, state),
        GalleryState::default(),
    );
    harness.run();
    harness.snapshot("token_gallery");
}
