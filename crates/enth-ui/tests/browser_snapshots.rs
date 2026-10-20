//! Component snapshots of the preset browser (lane PBU), rendered on lavapipe only (tests/common): the header's
//! "◀ Name ▶ Browse Save" with an edited preset, the browse panel and the Save dialog over the body at the default
//! window, 1200 × 800. One PNG each; the interactions are asserted on the session in `tests/browser.rs`, never on
//! these pixels.
//!
//! Update after a deliberate change, then look at every changed PNG:
//! `UPDATE_SNAPSHOTS=1 cargo test -p enth-ui --test browser_snapshots`.

mod common;

use egui::{Ui, UiBuilder, vec2};
use egui_kittest::SnapshotResults;
use enth_patch::{Op, ParamAddr, Patch};
use enth_session::{MemStore, PresetKey, RawPreset, Session};
use enth_ui::Theme;
use enth_ui::editor::Editor;
use enth_ui::editor::browser::{HeaderPresets, PresetIntent};
use enth_ui::editor::cards::{Intent, Intents};
use enth_ui::editor::regions::{Header, HeaderView};

fn preset(name: &str, folder_tags: &[&str], author: &str, description: &str) -> Patch {
    let mut patch = Patch::init();
    name.clone_into(&mut patch.meta.name);
    author.clone_into(&mut patch.meta.author);
    description.clone_into(&mut patch.meta.description);
    patch.meta.tags = folder_tags.iter().map(|t| (*t).to_owned()).collect();
    patch
}

/// A small library: four factory presets in three folders, two of the user's, and a damaged file.
fn store() -> MemStore {
    let factory = [
        (
            "bass/Sub One",
            preset(
                "Sub One",
                &["bass", "dark", "mono"],
                "",
                "A round sine-ish sub",
            ),
        ),
        (
            "bass/Grit Bass",
            preset("Grit Bass", &["bass", "gritty"], "", ""),
        ),
        (
            "pad/Glass Pad",
            preset("Glass Pad", &["pad", "bright", "wide", "evolving"], "", ""),
        ),
        (
            "pluck/Soft Pluck",
            preset("Soft Pluck", &["pluck", "soft"], "", ""),
        ),
    ]
    .into_iter()
    .map(|(path, patch)| RawPreset {
        key: PresetKey::factory(path),
        text: patch.to_json(),
    })
    .collect();
    let mut store = MemStore::with_factory(factory);
    store.put_user(
        "Mine/Wobble",
        &preset("Wobble", &["lead", "rhythmic"], "Ada", "").to_json(),
    );
    store.put_user(
        "Night Keys",
        &preset("Night Keys", &["keys", "warm"], "Ada", "").to_json(),
    );
    store.put_user("Broken", "{ not a patch");
    store
}

/// The whole editor at the default window with a preset loaded and then edited, after `intents` were applied.
fn editor_snapshot(results: &mut SnapshotResults, name: &str, intents: Vec<PresetIntent>) {
    let theme = Theme::dark();
    let t = theme.clone();
    let mut editor = Editor::new(&theme);
    editor.presets_mut().set_store(Box::new(store()));
    let mut session = Session::new(Patch::init());
    let apply = |editor: &mut Editor, session: &mut Session, intents: Vec<PresetIntent>| {
        editor.apply(
            session,
            intents.into_iter().map(Intent::Preset).collect(),
            false,
        );
    };
    apply(
        &mut editor,
        &mut session,
        vec![
            PresetIntent::Browse(true),
            PresetIntent::Load {
                key: PresetKey::factory("pad/Glass Pad"),
                close: true,
            },
            PresetIntent::Favorite {
                key: PresetKey::factory("bass/Sub One"),
                on: true,
            },
        ],
    );
    let level = ParamAddr::module(Patch::init().generators[0].id.clone(), "level").unwrap();
    let _ = enth_session::SessionEdit::edit(
        &mut session,
        vec![Op::SetParam {
            addr: level,
            value: 70.0,
        }],
        None,
    );
    apply(&mut editor, &mut session, intents);
    let mut h = common::harness(
        &theme,
        theme.size.window.default,
        move |ui: &mut Ui, (editor, session): &mut (Editor, Session)| {
            let rect = ui.ctx().content_rect();
            let mut root = ui.new_child(UiBuilder::new().max_rect(rect));
            editor.frame(&mut root, &t, session, false);
        },
        (editor, session),
    );
    h.run();
    h.snapshot(name);
    results.extend_harness(&mut h);
}

#[test]
fn browser_snapshots() {
    let mut results = SnapshotResults::new();
    let theme = Theme::dark();

    // The header alone: an edited preset, the browser closed.
    let header = HeaderView {
        presets: HeaderPresets {
            name: "Glass Pad".to_owned(),
            from_preset: true,
            edited: true,
            browsing: false,
            saving: false,
        },
        patch_name: "Glass Pad".to_owned(),
        can_undo: true,
        can_redo: false,
        undo: Some("Load Glass Pad".to_owned()),
        redo: None,
        room: enth_ui::editor::add_module::Room::all(&Patch::init()),
        // The Recreate bar (lane SKR) draws nothing without a shell's round view.
        ..HeaderView::default()
    };
    let t = theme.clone();
    let mut h = common::harness(
        &theme,
        vec2(
            theme.size.window.default.x,
            theme.size.bar.header + 2.0 * theme.space.gap,
        ),
        move |ui: &mut Ui, (): &mut ()| {
            let rect = ui.ctx().content_rect();
            ui.painter()
                .rect_filled(rect, 0.0, t.color.surface.background);
            Header::new(&t, &header).show(ui, rect.shrink(t.space.gap), &mut Intents::default());
        },
        (),
    );
    h.run();
    h.snapshot("browser_header");
    results.extend_harness(&mut h);

    editor_snapshot(
        &mut results,
        "browser_panel",
        vec![PresetIntent::Browse(true)],
    );
    editor_snapshot(&mut results, "browser_save", vec![PresetIntent::OpenSave]);
    results.unwrap();
}
