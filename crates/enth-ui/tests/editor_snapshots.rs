//! Whole-editor snapshots for layout 1, "lanes" (lane ED1): the P1 default patch and the maximum patch at the default
//! window, 1200 × 800, rendered on lavapipe only (tests/common). One PNG each; the interactions are asserted on the
//! session in `tests/editor.rs`, never on these pixels.
//!
//! Update after a deliberate change, then look at both PNGs:
//! `UPDATE_SNAPSHOTS=1 cargo test -p enth-ui --test editor_snapshots`.

mod common;

use egui::UiBuilder;
use egui_kittest::SnapshotResults;
use enth_patch::Patch;
use enth_session::Session;
use enth_ui::Theme;
use enth_ui::editor::Editor;

fn snapshot(results: &mut SnapshotResults, name: &str, patch: Patch) {
    let theme = Theme::dark();
    let t = theme.clone();
    let mut h = common::harness(
        &theme,
        theme.size.window.default,
        move |ui, (editor, session): &mut (Editor, Session)| {
            let rect = ui.ctx().content_rect();
            let mut root = ui.new_child(UiBuilder::new().max_rect(rect));
            editor.frame(&mut root, &t, session, false);
        },
        (Editor::new(&theme), Session::new(patch)),
    );
    h.run();
    h.snapshot(name);
    results.extend_harness(&mut h);
}

#[test]
fn lanes_snapshots() {
    let mut results = SnapshotResults::new();
    snapshot(
        &mut results,
        "editor_lanes_p1",
        common::patches::p1_default(),
    );
    snapshot(&mut results, "editor_lanes_max", common::patches::maximum());
    results.unwrap();
}
