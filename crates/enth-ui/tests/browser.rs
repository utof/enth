//! The preset browser through a real session (lane PBU): kittest drives egui's input and finds the controls by their
//! accessibility labels, with no renderer; every assertion is on the session (its patch, its history, its requests)
//! or on the browser's state, never on pixels. The snapshots are in `tests/browser_snapshots.rs`.
//!
//! The harness runs `Editor::frame` over a `Session` the way the plugin does, minus the lock: pump, frame (draw, then
//! apply the intents), drain the requests. The store is a `MemStore` with three presets of its own (the embedded
//! factory list is lane PR's and may be empty).

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use egui::{Key, Modifiers, UiBuilder};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use enth_patch::{Op, ParamAddr, Patch};
use enth_session::{
    Activation, MemStore, PresetKey, PresetState, RawPreset, Session, ShellRequest,
};
use enth_ui::Theme;
use enth_ui::editor::Editor;
use enth_ui::editor::browser::{CrashReports, PresetIntent, Sheet, Tab};

/// Renders one block of the session's engine into two buffers.
type Render = Box<dyn FnMut(&mut [f32], &mut [f32])>;

/// The editor, its session, every request the session made, and (when active) the engine's renderer.
struct App {
    theme: Theme,
    editor: Editor,
    session: Session,
    requests: Vec<ShellRequest>,
    /// Renders one block from the session's engine (an audition test activates the session).
    render: Option<Render>,
}

impl App {
    fn host_writes(&self) -> usize {
        self.requests
            .iter()
            .filter(|r| matches!(r, ShellRequest::Host(_)))
            .count()
    }

    fn edited(&self) -> usize {
        self.requests
            .iter()
            .filter(|r| **r == ShellRequest::PatchEdited)
            .count()
    }

    fn last(&self) -> Option<PresetKey> {
        self.session
            .instance()
            .preset
            .as_ref()
            .and_then(PresetState::last_key)
    }

    /// How many undo steps the history holds (undone and redone again, so nothing changes).
    fn depth(&mut self) -> usize {
        let mut n = 0;
        while self.session.history().can_undo() {
            self.session.undo().unwrap();
            n += 1;
        }
        for _ in 0..n {
            self.session.redo().unwrap();
        }
        self.session.drain_requests().for_each(drop);
        n
    }

    /// The loudest sample of `blocks` blocks of 256 frames from the engine.
    fn peak(&mut self, blocks: usize) -> f32 {
        let render = self.render.as_mut().expect("an active session");
        let (mut left, mut right) = (vec![0.0; 256], vec![0.0; 256]);
        let mut peak = 0.0_f32;
        for _ in 0..blocks {
            render(&mut left, &mut right);
            for s in left.iter().chain(&right) {
                peak = peak.max(s.abs());
            }
        }
        peak
    }
}

fn level() -> ParamAddr {
    ParamAddr::module(Patch::init().generators[0].id.clone(), "level").unwrap()
}

/// A preset: the init patch with a name, tags, an author and osc 1's level at `level` (so loading it changes a value).
fn preset(name: &str, tags: &[&str], author: &str, value: f32) -> Patch {
    let mut patch = Patch::init();
    name.clone_into(&mut patch.meta.name);
    author.clone_into(&mut patch.meta.author);
    patch.meta.tags = tags.iter().map(|t| (*t).to_owned()).collect();
    enth_patch::apply(
        &mut patch,
        Op::SetParam {
            addr: level(),
            value,
        },
    )
    .unwrap();
    patch
}

/// Two factory presets and one user preset.
fn store() -> MemStore {
    let factory = vec![
        RawPreset {
            key: PresetKey::factory("bass/Sub One"),
            text: preset("Sub One", &["bass", "dark"], "Ada", 80.0).to_json(),
        },
        RawPreset {
            key: PresetKey::factory("pad/Glass Pad"),
            text: preset("Glass Pad", &["pad", "bright", "wide"], "Lin", 60.0).to_json(),
        },
    ];
    let mut store = MemStore::with_factory(factory);
    store.put_user(
        "Mine/Wobble",
        &preset("Wobble", &["lead"], "Ada", 30.0).to_json(),
    );
    store
}

fn harness_with(store: MemStore, active: bool) -> Harness<'static, App> {
    let theme = Theme::dark();
    let mut editor = Editor::new(&theme);
    editor.presets_mut().set_store(Box::new(store));
    let mut session = Session::new(Patch::init());
    let render: Option<Render> = if active {
        let mut engine = session
            .activate(Activation {
                sample_rate: 48_000.0,
                offline: false,
            })
            .unwrap();
        Some(Box::new(move |l: &mut [f32], r: &mut [f32]| {
            engine.process(l, r, &[]);
        }))
    } else {
        None
    };
    let app = App {
        editor,
        theme: theme.clone(),
        session,
        requests: Vec::new(),
        render,
    };
    let mut h = Harness::builder()
        .with_size(theme.size.window.default)
        .with_pixels_per_point(1.0)
        .with_step_dt(1.0 / 60.0)
        .with_context_setup(move |ctx| theme.apply(ctx))
        .build_ui_state(
            |ui, app: &mut App| {
                let down = ui.input(|i| i.pointer.any_down());
                let rect = ui.ctx().content_rect();
                let mut root = ui.new_child(UiBuilder::new().max_rect(rect));
                app.session.pump();
                app.editor
                    .frame(&mut root, &app.theme, &mut app.session, down);
                app.requests.extend(app.session.drain_requests());
                // An active session's engine runs beside the editor, as a host's audio thread does.
                if let Some(render) = app.render.as_mut() {
                    let (mut left, mut right) = ([0.0; 256], [0.0; 256]);
                    render(&mut left, &mut right);
                }
            },
            app,
        );
    h.run_steps(2);
    h
}

fn harness() -> Harness<'static, App> {
    harness_with(store(), false)
}

fn click(h: &mut Harness<'_, App>, label: &str) {
    h.get_by_label(label).click();
    h.run_steps(3);
}

fn browse(h: &mut Harness<'_, App>) {
    click(h, "Browse");
    assert_eq!(h.state().editor.presets().sheet(), Sheet::Browse);
}

/// Clicks the search field, selects what it holds and types `text` over it.
fn search(h: &mut Harness<'_, App>, text: &str) {
    click(h, "Search presets");
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.get_by_label("Search presets").type_text(text);
    h.run_steps(3);
}

#[test]
fn browse_shows_every_preset_and_hides_the_body() {
    let mut h = harness();
    assert!(
        h.query_by_label("Attack").is_some(),
        "the body's cards draw"
    );
    browse(&mut h);
    for name in ["Sub One", "Glass Pad", "Wobble"] {
        assert!(h.query_by_label(name).is_some(), "{name} is listed");
    }
    assert!(
        h.query_by_label("Attack").is_none(),
        "the panel replaces the body: no card is drawn under it"
    );
    // Escape closes it, and the body comes back.
    h.key_press(Key::Escape);
    h.run_steps(3);
    assert_eq!(h.state().editor.presets().sheet(), Sheet::Closed);
    assert!(h.query_by_label("Attack").is_some());
    // Browse twice toggles; × closes too.
    browse(&mut h);
    click(&mut h, "Close browser");
    assert_eq!(h.state().editor.presets().sheet(), Sheet::Closed);
}

#[test]
fn a_click_loads_one_undo_step_with_no_host_writes_and_marks_the_project_dirty() {
    let mut h = harness();
    let before = h.state().session.patch().clone();
    browse(&mut h);
    h.state_mut().requests.clear();
    click(&mut h, "Glass Pad");
    let app = h.state();
    assert_eq!(app.session.patch().meta.name, "Glass Pad");
    assert_eq!(app.session.patch().value(&level()), Some(60.0));
    assert_eq!(app.last(), Some(PresetKey::factory("pad/Glass Pad")));
    let step = app.session.history().next_undo().expect("one undo step");
    assert_eq!(step.load.as_deref(), Some("Glass Pad"));
    assert_eq!(app.host_writes(), 0, "no host parameter is written");
    assert_eq!(app.edited(), 1, "the project is marked dirty once");
    assert_eq!(
        app.editor.presets().sheet(),
        Sheet::Browse,
        "a single click keeps the panel open"
    );
    // The header names it, without the edited dot, and its Undo says what it undoes.
    assert!(h.query_by_label("Preset Glass Pad").is_some());
    let header = h.state().editor.presets().header(&h.state().session);
    assert!(header.from_preset && !header.edited && header.browsing);

    // One Undo takes the whole load back, one Redo brings it again, still without host writes.
    h.state_mut().requests.clear();
    click(&mut h, "Undo");
    assert_eq!(h.state().session.patch(), &before);
    assert!(!h.state().session.history().can_undo());
    click(&mut h, "Redo");
    assert_eq!(h.state().session.patch().meta.name, "Glass Pad");
    assert_eq!(h.state().host_writes(), 0);
    assert_eq!(
        h.state().edited(),
        2,
        "undo and redo each mark the project dirty"
    );
}

#[test]
fn a_double_click_loads_and_closes() {
    let mut h = harness();
    browse(&mut h);
    // Well after the click on Browse, so the pair is not taken for a triple click.
    h.run_steps(30);
    let row = h.get_by_label("Sub One");
    row.click();
    h.step();
    h.get_by_label("Sub One").click();
    h.run_steps(3);
    let app = h.state_mut();
    assert_eq!(app.session.patch().meta.name, "Sub One");
    assert_eq!(app.editor.presets().sheet(), Sheet::Closed);
    assert_eq!(
        app.depth(),
        1,
        "the second click of the pair loads nothing more"
    );
}

#[test]
fn the_search_takes_words_tags_and_authors() {
    let mut h = harness();
    browse(&mut h);
    search(&mut h, "glass");
    assert!(h.query_by_label("Glass Pad").is_some());
    assert!(h.query_by_label("Sub One").is_none());
    search(&mut h, "#bass");
    assert!(h.query_by_label("Sub One").is_some());
    assert!(h.query_by_label("Glass Pad").is_none());
    search(&mut h, "by:ada");
    assert!(h.query_by_label("Sub One").is_some());
    assert!(h.query_by_label("Wobble").is_some());
    assert!(h.query_by_label("Glass Pad").is_none());
    search(&mut h, "nothing-like-this");
    assert!(h.query_by_label("No presets match the search").is_some());
    // A click on a found preset loads it.
    search(&mut h, "wob");
    click(&mut h, "Wobble");
    assert_eq!(h.state().session.patch().meta.name, "Wobble");
}

#[test]
fn tabs_filter_by_source_and_favorites_are_saved_by_the_store() {
    let mut h = harness();
    browse(&mut h);
    click(&mut h, "User");
    assert_eq!(h.state().editor.presets().tab(), Tab::User);
    assert!(h.query_by_label("Wobble").is_some());
    assert!(h.query_by_label("Sub One").is_none());
    click(&mut h, "Factory");
    assert!(h.query_by_label("Wobble").is_none());
    assert!(h.query_by_label("Sub One").is_some());
    click(&mut h, "Favorites");
    assert!(
        h.query_by_label("No favorites yet: star a preset with ☆")
            .is_some()
    );

    click(&mut h, "All");
    click(&mut h, "Favorite Glass Pad");
    let glass = PresetKey::factory("pad/Glass Pad");
    assert!(h.state().editor.presets().is_favorite(&glass));
    assert!(
        !h.state().session.history().can_undo(),
        "starring is not an edit"
    );
    click(&mut h, "Favorites");
    assert!(h.query_by_label("Glass Pad").is_some());
    assert!(h.query_by_label("Sub One").is_none());
    // The store kept it: a fresh scan (closing and opening the panel) still has it.
    click(&mut h, "Close browser");
    browse(&mut h);
    let library = h.state().editor.presets().library().unwrap();
    assert!(library.favorites().contains(&glass));
    // And a second click unstars it.
    click(&mut h, "Favorite Glass Pad");
    assert!(!h.state().editor.presets().is_favorite(&glass));
}

#[test]
fn the_header_steps_through_the_list_and_marks_edits() {
    let mut h = harness();
    // No preset yet: ▶ gives the first of the list (browsing order: factory by folder, then the user's).
    click(&mut h, "Next preset");
    assert_eq!(h.state().last(), Some(PresetKey::factory("bass/Sub One")));
    click(&mut h, "Next preset");
    assert_eq!(h.state().last(), Some(PresetKey::factory("pad/Glass Pad")));
    click(&mut h, "Next preset");
    assert_eq!(h.state().last(), Some(PresetKey::user("Mine/Wobble")));
    click(&mut h, "Next preset");
    assert_eq!(
        h.state().last(),
        Some(PresetKey::factory("bass/Sub One")),
        "it wraps"
    );
    click(&mut h, "Previous preset");
    assert_eq!(h.state().last(), Some(PresetKey::user("Mine/Wobble")));
    assert_eq!(h.state().session.patch().meta.name, "Wobble");
    assert_eq!(h.state_mut().depth(), 5, "one undo step per load");
    assert_eq!(h.state().host_writes(), 0);
    assert!(h.query_by_label("Preset Wobble").is_some());

    // An edit marks the name; the steps follow the browser's search.
    let edit = vec![enth_ui::editor::cards::Intent::Set {
        addr: level(),
        value: 12.0,
    }];
    let app = h.state_mut();
    app.editor.apply(&mut app.session, edit, false);
    h.run_steps(2);
    assert!(h.query_by_label("Preset Wobble (edited)").is_some());
    browse(&mut h);
    search(&mut h, "#pad");
    click(&mut h, "Close browser");
    click(&mut h, "Next preset");
    assert_eq!(h.state().last(), Some(PresetKey::factory("pad/Glass Pad")));
    assert!(
        h.query_by_label("Preset Glass Pad").is_some(),
        "a load is clean again"
    );
}

#[test]
fn save_names_tags_and_asks_before_replacing() {
    let mut h = harness();
    click(&mut h, "Save");
    assert_eq!(h.state().editor.presets().sheet(), Sheet::Save);
    // The name field is focused and holds the patch's name: select it and type over it.
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.get_by_label("Preset name").type_text("My Lead");
    h.run_steps(2);
    click(&mut h, "lead");
    click(&mut h, "bright");
    click(&mut h, "Save preset");
    let app = h.state();
    assert_eq!(app.editor.presets().sheet(), Sheet::Closed);
    assert_eq!(app.session.patch().meta.name, "My Lead");
    assert_eq!(app.session.patch().meta.tags, vec!["lead", "bright"]);
    assert_eq!(app.last(), Some(PresetKey::user("My Lead")));
    assert!(!app.session.history().is_dirty(), "a save counts as saved");
    assert!(
        h.query_by_label("Preset My Lead").is_some(),
        "the header names it"
    );
    // It is in the store, tags and all.
    browse(&mut h);
    let saved = h
        .state()
        .editor
        .presets()
        .library()
        .and_then(|l| l.get(&PresetKey::user("My Lead")).cloned())
        .expect("the saved preset is scanned");
    assert_eq!(saved.meta.tags, vec!["lead", "bright"]);

    // Saving under a name in use asks first; Cancel keeps the dialog, Replace saves.
    click(&mut h, "Save");
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.get_by_label("Preset name").type_text("Wobble");
    h.run_steps(2);
    h.get_by_label("Preset folder").click();
    h.run_steps(2);
    h.get_by_label("Preset folder").type_text("Mine");
    h.run_steps(2);
    click(&mut h, "Save preset");
    assert_eq!(
        h.state().editor.presets().save_confirm(),
        Some(&PresetKey::user("Mine/Wobble"))
    );
    assert!(h.query_by_label_contains("Replace it?").is_some());
    click(&mut h, "Cancel");
    assert!(h.state().editor.presets().save_confirm().is_none());
    assert_eq!(h.state().editor.presets().sheet(), Sheet::Save);
    click(&mut h, "Save preset");
    click(&mut h, "Replace");
    assert_eq!(h.state().last(), Some(PresetKey::user("Mine/Wobble")));
    assert_eq!(
        h.state().editor.presets().sheet(),
        Sheet::Browse,
        "opened from the panel, the dialog goes back to it"
    );
}

#[test]
fn save_shows_refusals_with_their_hints() {
    let mut h = harness();
    click(&mut h, "Save");
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.get_by_label("Preset name").type_text("///");
    h.run_steps(2);
    click(&mut h, "Save preset");
    let error = h
        .state()
        .editor
        .presets()
        .save_error()
        .cloned()
        .expect("a bad name is refused");
    assert!(error.text.contains("cannot be saved"), "{error:?}");
    let hint = error.hint.unwrap();
    assert!(h.query_by_label(&hint).is_some(), "the hint shows: {hint}");
    assert!(!h.state().session.history().can_undo());

    // A store that cannot write says so, with its hint, and nothing changes.
    let mut failing = store();
    failing.fail_writes = true;
    let mut h = harness_with(failing, false);
    click(&mut h, "Save");
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.get_by_label("Preset name").type_text("Anything");
    h.run_steps(2);
    click(&mut h, "Save preset");
    let error = h.state().editor.presets().save_error().cloned().unwrap();
    assert!(error.text.contains("could not write"), "{error:?}");
    assert!(
        h.query_by_label("check that the user folder exists and can be written")
            .is_some()
    );
    assert_eq!(h.state().session.patch().meta.name, Patch::init().meta.name);
    // Escape closes it without saving.
    h.key_press(Key::Escape);
    h.run_steps(3);
    assert_eq!(h.state().editor.presets().sheet(), Sheet::Closed);
}

#[test]
fn holding_play_loads_the_preset_and_auditions_it_until_release() {
    let mut h = harness_with(store(), true);
    assert!(h.state_mut().peak(4) == 0.0, "silent before");
    browse(&mut h);
    let play = h.get_by_label("Audition Sub One").rect().center();
    h.hover_at(play);
    h.step();
    h.drag_at(play);
    h.run_steps(3);
    let app = h.state_mut();
    assert_eq!(app.session.patch().meta.name, "Sub One", "▶ loads it first");
    assert_eq!(app.depth(), 1);
    let peak = app.peak(8);
    assert!(
        peak > 0.0,
        "a note sounds while ▶ is held: {:?} {:?}",
        app.editor.presets().message(),
        app.session.status()
    );
    h.drop_at(play);
    h.run_steps(3);
    // Released: the note ends (its release tail dies away within a couple of seconds).
    let _ = h.state_mut().peak(400);
    assert!(h.state_mut().peak(40) == 0.0, "silent after the release");
    // A second hold on the loaded preset loads nothing more.
    h.hover_at(play);
    h.step();
    h.drag_at(play);
    h.run_steps(2);
    h.drop_at(play);
    h.run_steps(2);
    assert_eq!(h.state_mut().depth(), 1);
}

#[test]
fn audition_without_an_engine_says_why() {
    let mut h = harness();
    browse(&mut h);
    let play = h.get_by_label("Audition Wobble").rect().center();
    h.hover_at(play);
    h.step();
    h.drag_at(play);
    h.run_steps(3);
    h.drop_at(play);
    h.run_steps(2);
    let message = h.state().editor.presets().message().cloned().unwrap();
    assert!(message.error, "{message:?}");
    assert_eq!(
        h.state().session.patch().meta.name,
        "Wobble",
        "it still loads"
    );
}

/// A crash folder in memory: `seen` counts the `mark_seen` calls.
struct FakeReports {
    pending: usize,
    seen: Arc<AtomicUsize>,
}

impl CrashReports for FakeReports {
    fn count(&self) -> usize {
        self.pending
    }
    fn newest(&self) -> Result<String, String> {
        Ok("thread 'main' panicked at src/lib.rs:1:1: test panic".to_owned())
    }
    fn location(&self) -> String {
        "~/Documents/enth/Crashes".to_owned()
    }
    fn mark_seen(&mut self) -> Result<(), String> {
        self.pending = 0;
        self.seen.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
}

fn crashed(h: &mut Harness<'_, App>) -> Arc<AtomicUsize> {
    let seen = Arc::new(AtomicUsize::new(0));
    h.state_mut()
        .editor
        .presets_mut()
        .offer_crash(Some(Box::new(FakeReports {
            pending: 2,
            seen: Arc::clone(&seen),
        })));
    h.run_steps(3);
    seen
}

#[test]
fn the_crash_notice_never_blocks_and_shows_the_report_once() {
    let mut h = harness();
    let seen = crashed(&mut h);
    assert!(h.state().editor.presets().crash_notice());
    assert!(
        h.query_by_label("enth stopped unexpectedly last time")
            .is_some()
    );
    // The editor works with the notice up: a header step loads a preset.
    click(&mut h, "Next preset");
    assert_eq!(h.state().session.patch().meta.name, "Sub One");
    click(&mut h, "Show report");
    assert_eq!(seen.load(Ordering::Relaxed), 1, "offered once: marked seen");
    assert_eq!(h.state().editor.presets().sheet(), Sheet::Report);
    assert!(
        h.state()
            .editor
            .presets()
            .crash_report()
            .is_some_and(|r| r.contains("test panic"))
    );
    assert!(!h.state().editor.presets().crash_notice());
    click(&mut h, "Close report");
    assert_eq!(h.state().editor.presets().sheet(), Sheet::Closed);

    // Dismiss marks them seen and shows nothing.
    let mut h = harness();
    let seen = crashed(&mut h);
    click(&mut h, "Dismiss");
    assert_eq!(seen.load(Ordering::Relaxed), 1);
    assert!(!h.state().editor.presets().crash_notice());
    assert_eq!(h.state().editor.presets().sheet(), Sheet::Closed);
    // No reports, no notice.
    h.state_mut().editor.presets_mut().offer_crash(None);
    assert!(!h.state().editor.presets().crash_notice());
}

#[test]
fn a_preset_the_store_lost_is_refused_with_the_reason() {
    // The store loses Wobble between the scan and the load: the user's file was deleted.
    let mut h = harness();
    browse(&mut h);
    click(&mut h, "Close browser");
    let app = h.state_mut();
    app.editor
        .presets_mut()
        .set_store(Box::new(MemStore::with_factory(Vec::new())));
    // A load of a key the store does not have, as a stale row would ask.
    let missing = vec![enth_ui::editor::cards::Intent::Preset(PresetIntent::Load {
        key: PresetKey::user("Mine/Wobble"),
        close: false,
    })];
    app.editor.apply(&mut app.session, missing, false);
    let message = app.editor.presets().message().cloned().unwrap();
    assert!(message.error);
    assert_eq!(
        message.hint.as_deref(),
        Some("the file was moved or deleted: open the browser again")
    );
    assert!(!app.session.history().can_undo());
    // With an empty list the steps say so instead.
    let step = vec![enth_ui::editor::cards::Intent::Preset(PresetIntent::Step(
        1,
    ))];
    app.editor.apply(&mut app.session, step, false);
    assert_eq!(
        app.editor.presets().message().map(|m| m.text.as_str()),
        Some("No presets in this list")
    );
}

/// The click points of `xtask/reaper/preset.lua` (offsets into the plugin's 1200 × 800 window): keep both in step.
const REAPER_POINTS: [(&str, (f32, f32)); 6] = [
    ("browse", (347.0, 28.0)),
    ("user_tab", (138.0, 80.0)),
    ("first_row", (200.0, 140.0)),
    ("first_play", (60.0, 140.0)),
    ("undo", (759.0, 28.0)),
    ("redo", (809.0, 28.0)),
];

fn point(name: &str) -> egui::Pos2 {
    let (_, (x, y)) = REAPER_POINTS
        .iter()
        .find(|(n, _)| *n == name)
        .expect("a REAPER point");
    egui::pos2(*x, *y)
}

/// A press and a release at `pos`, as `xinput.py click` sends them.
fn click_at(h: &mut Harness<'_, App>, pos: egui::Pos2) {
    h.hover_at(pos);
    h.step();
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        });
    }
    h.run_steps(3);
}

#[test]
fn reaper_points_hit_their_controls() {
    // As the REAPER scenario sets it up: one user preset, whatever the factory holds.
    let mut store = MemStore::default();
    store.put_user(
        "roundtrip",
        &preset("reaper-test roundtrip", &["pad"], "enth", 25.0).to_json(),
    );
    let mut h = harness_with(store, false);
    let contains = |h: &Harness<'_, App>, label: &str, name: &str| {
        let rect = h.get_by_label(label).rect();
        assert!(
            rect.contains(point(name)),
            "{name} {:?} is not on {label:?} at {rect:?}: update REAPER_POINTS and xtask/reaper/preset.lua",
            point(name)
        );
    };
    contains(&h, "Browse", "browse");
    contains(&h, "Undo", "undo");
    contains(&h, "Redo", "redo");
    click_at(&mut h, point("browse"));
    contains(&h, "User", "user_tab");
    click_at(&mut h, point("user_tab"));
    contains(&h, "reaper-test roundtrip", "first_row");
    contains(&h, "Audition reaper-test roundtrip", "first_play");
    // The scenario's sequence: the row loads, Undo takes it back, Redo again.
    let before = h.state().session.patch().clone();
    click_at(&mut h, point("first_row"));
    assert_eq!(h.state().session.patch().meta.name, "reaper-test roundtrip");
    assert_eq!(h.state().last(), Some(PresetKey::user("roundtrip")));
    click_at(&mut h, point("undo"));
    assert_eq!(h.state().session.patch(), &before);
    click_at(&mut h, point("redo"));
    assert_eq!(h.state().session.patch().meta.name, "reaper-test roundtrip");
    assert_eq!(h.state().host_writes(), 0);
}
