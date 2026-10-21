//! The Learn view's layout (lane LUI) and lane SK3's Create panel side by side (lane SK3M, the merge of `r3/sk3` with
//! main): the Create panel takes its column at the window's right edge, and the layout, the built-in `learn` one
//! included, gets the rest. Over a real `Session`, the way the plugin's glue drives both each frame
//! (`enth-plugin/src/editor.rs`): the lesson and create views handed over (`Editor::set_lesson`, `Editor::set_create`),
//! the frame, then their intents applied (`Editor::apply_learn`, `Editor::apply_create`). The library is the example
//! pack the plugin's tests use; no lesson is started, so no worker is needed.
//!
//! At 1200 × 800: neither pane, Learn only, Create only and both, with the overflow report; a dropped file still goes
//! to Create beside the lesson pane; a snapshot of both open (lavapipe).
#![cfg(all(feature = "learn", feature = "create"))]

mod common;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use egui::{Rect, UiBuilder, Vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use enth_learn::Progress;
use enth_learn::store::{MemStore, Origin, PackSource, load_library};
use enth_patch::Patch;
use enth_session::Session;
use enth_session::create::CreateEdit;
use enth_session::learn::LearnEdit;
use enth_ui::Theme;
use enth_ui::editor::{Editor, Region};

/// The bundled example pack, as the plugin embeds it.
const EXAMPLE: &[u8] = include_bytes!("../../enth-plugin/packs/example.enthpack");

struct App {
    theme: Theme,
    editor: Editor,
    session: Session,
}

fn harness_with(patch: Patch, render: bool) -> Harness<'static, App> {
    let theme = Theme::dark();
    let mut editor = Editor::new(&theme);
    let mut store = MemStore::new();
    store.insert(
        Origin::Bundled,
        "example.enthpack",
        PackSource::Archive(EXAMPLE.to_vec()),
    );
    let library = load_library(&mut store);
    assert!(library.problems.is_empty(), "{:?}", library.problems);
    editor.set_library(&library, Progress::new());
    let app = App {
        theme: theme.clone(),
        editor,
        session: Session::new(patch),
    };
    let mut builder = Harness::builder()
        .with_size(theme.size.window.default)
        .with_pixels_per_point(1.0)
        .with_step_dt(1.0 / 60.0)
        .with_context_setup(move |ctx| theme.apply(ctx));
    if render {
        builder = builder.renderer(common::cpu_renderer());
    }
    builder.build_ui_state(
        |ui, app: &mut App| {
            let down = ui.input(|i| i.pointer.any_down());
            let rect = ui.ctx().content_rect();
            let mut root = ui.new_child(UiBuilder::new().max_rect(rect));
            app.editor.set_lesson(LearnEdit::lesson(&app.session));
            app.editor.set_create(CreateEdit::create(&app.session));
            app.editor
                .frame(&mut root, &app.theme, &mut app.session, down);
            app.editor.apply_learn(&mut app.session);
            app.editor.apply_create(&mut app.session);
            app.session.drain_requests().for_each(drop);
            app.session.drain_learn_requests().for_each(drop);
            app.session.drain_create_requests().for_each(drop);
        },
        app,
    )
}

fn click(h: &mut Harness<'_, App>, label: &str) {
    h.get_by_label(label).click();
    h.run_steps(3);
}

/// The rectangle of the button named `label`.
fn rect_of(h: &Harness<'_, App>, label: &str) -> Rect {
    h.get_by_label(label).rect()
}

/// The right edge of the layout's regions under the header row (what the Create panel's column leaves them), and of
/// the header row's own.
fn rights(h: &Harness<'_, App>) -> (f32, f32) {
    let report = h.state().editor.report();
    let header = report.rect(Region::Header).expect("a header");
    let (top, below): (Vec<_>, Vec<_>) = report
        .regions
        .iter()
        .partition(|p| p.rect.bottom() <= header.bottom() + 0.5);
    let right = |v: Vec<&enth_ui::editor::layout::Placed>| {
        v.iter().map(|p| p.rect.right()).fold(f32::MIN, f32::max)
    };
    (right(below), right(top))
}

fn overflow(h: &Harness<'_, App>) -> Vec<(&'static str, Vec2)> {
    h.state().editor.report().overflow.clone()
}

fn lesson(h: &Harness<'_, App>) -> Option<Rect> {
    h.state().editor.report().rect(Region::Lesson)
}

/// Neither, Learn only, both, Create only, and neither again, at 1200 × 800 with Init: where the panes go and what the
/// overflow report says. The Create panel is `size.create.pane` wide at the window's right edge (inset by
/// `space.gap`) under the header row, which keeps the window's width; the lesson pane is the layout `learn`'s column
/// left of it, and nothing the layout places under the header row reaches into the panel's column.
#[test]
fn the_panes_open_alone_and_together_at_1200_by_800() {
    let t = Theme::dark();
    let width = t.size.window.default.x;
    let edge = width - t.space.gap;
    let split = edge - t.size.create.pane;
    let mut h = harness_with(Patch::init(), false);
    h.run_steps(2);

    // Neither: layout 1 over the whole window.
    assert!(lesson(&h).is_none());
    assert!(h.query_by_label("Start create").is_none());
    let (below, top) = rights(&h);
    assert!((below - edge).abs() < 0.5 && (top - edge).abs() < 0.5);
    assert_eq!(overflow(&h), Vec::new(), "Init fits layout 1");
    let header = h.state().editor.report().rect(Region::Header).unwrap();

    // Learn only: the layout `learn`, its pane at the right of the window.
    click(&mut h, "Learn");
    let pane = lesson(&h).expect("the lesson pane");
    assert!((pane.width() - t.size.learn.pane).abs() < 0.5, "{pane:?}");
    assert!((pane.right() - edge).abs() < 0.5, "{pane:?}");
    let learn_header = h.state().editor.report().rect(Region::Header).unwrap();
    assert_eq!(overflow(&h), Vec::new(), "Init fits beside the lesson pane");

    // Both: the Create panel at the right edge under the header row, the lesson pane left of it.
    click(&mut h, "Create");
    assert!(
        h.query_by_label("Start create").is_some(),
        "the panel opened"
    );
    let pane = lesson(&h).expect("the lesson pane stays");
    assert!(
        (pane.right() - (split - t.space.gap)).abs() < 0.5,
        "the lesson pane ends a gap left of the Create panel: {pane:?}"
    );
    assert!(
        (pane.width() - t.size.learn.pane).abs() < 0.5,
        "the lesson pane keeps its width: {pane:?}"
    );
    let report = h.state().editor.report().clone();
    assert_eq!(
        report.rect(Region::Header),
        Some(learn_header),
        "the header row keeps the window's width"
    );
    assert!((report.rect(Region::Output).unwrap().right() - edge).abs() < 0.5);
    let (below, _) = rights(&h);
    assert!(below <= split - t.space.gap + 0.5, "{below}");
    let close = rect_of(&h, "Close create");
    assert!(
        close.left() >= split && close.right() <= edge + 0.5,
        "{close:?}"
    );
    assert!(
        close.top() >= learn_header.bottom() + t.space.gap,
        "{close:?}"
    );
    let both = overflow(&h);
    println!("Learn and Create: {both:?}");
    assert_eq!(both, Vec::new(), "Init fits beside both panes");

    // Create only: Learn hidden, layout 1 beside the panel, its header row whole.
    click(&mut h, "Hide lessons");
    assert!(lesson(&h).is_none());
    assert!(h.query_by_label("Start create").is_some());
    assert_eq!(h.state().editor.report().rect(Region::Header), Some(header));
    let (below, top) = rights(&h);
    assert!(below <= split - t.space.gap + 0.5, "{below}");
    assert!((top - edge).abs() < 0.5, "{top}");
    assert_eq!(
        overflow(&h),
        Vec::new(),
        "Init fits beside the Create panel"
    );

    // Neither again: the whole window is layout 1's.
    click(&mut h, "Close create");
    assert!(h.query_by_label("Start create").is_none());
    let (below, top) = rights(&h);
    assert!((below - edge).abs() < 0.5 && (top - edge).abs() < 0.5);
    assert_eq!(overflow(&h), Vec::new());
}

/// The full P1 default with both panes open at 1200 × 800: what no longer fits is in the report, region by region (the
/// cards column is 1200 − 2 × 320 − the gaps). With the lesson pane alone the band overflows by 276 px and the
/// effects lane by 84 (`learn_view.rs`); beside both panes the band by 604, the effects lane by the same 84, and the
/// overview row, now as narrow as the cards column, by 51 (tiles) and 63 (routes). The idle panes fit their columns.
#[test]
fn a_full_patch_with_both_panes_reports_its_overflow() {
    let mut h = harness_with(common::patches::p1_default(), false);
    h.run_steps(2);
    click(&mut h, "Learn");
    click(&mut h, "Create");
    assert!(lesson(&h).is_some());
    assert!(h.query_by_label("Start create").is_some());
    let over = overflow(&h);
    let sources = h.state().editor.report().rect(Region::Sources).unwrap();
    println!(
        "P1 default, Learn and Create: {over:?}; sources {:.0} px wide",
        sources.width()
    );
    let of = |name: &str| {
        over.iter()
            .find(|(n, _)| *n == name)
            .map_or_else(|| panic!("{name} is in the report: {over:?}"), |(_, o)| *o)
    };
    assert!((of("modulators").x - 604.0).abs() < 2.0, "{over:?}");
    assert!((of("effects").y - 84.0).abs() < 2.0, "{over:?}");
    assert!((of("overview.tiles").y - 51.0).abs() < 2.0, "{over:?}");
    assert!((of("overview.matrix").y - 63.0).abs() < 2.0, "{over:?}");
    assert_eq!(over.len(), 4, "nothing else scrolls: {over:?}");
    // Hiding the lesson pane: layout 1 beside the Create panel, its band as with the lesson pane alone (276 px) and
    // its overview (beside the band's start, as in layout 1) by 111 (tiles) and 63 (routes).
    click(&mut h, "Hide lessons");
    let over = overflow(&h);
    println!("P1 default, Create only: {over:?}");
    let of = |name: &str| over.iter().find(|(n, _)| *n == name).map(|(_, o)| *o);
    assert!(
        of("modulators").is_some_and(|o| (o.x - 276.0).abs() < 2.0),
        "{over:?}"
    );
    assert!(
        of("overview.tiles").is_some_and(|o| (o.y - 111.0).abs() < 2.0),
        "{over:?}"
    );
    assert!(
        of("overview.matrix").is_some_and(|o| (o.y - 63.0).abs() < 2.0),
        "{over:?}"
    );
    assert_eq!(over.len(), 3, "{over:?}");
}

/// A dropped file, as an integration hands it to egui (only its path is read).
#[derive(Debug)]
struct Dropped(PathBuf);

impl egui::DroppedFile for Dropped {
    fn path(&self) -> &Path {
        &self.0
    }

    fn bytes(&self) -> Result<Vec<u8>, String> {
        Err("the test never reads the file".to_owned())
    }
}

/// A file dropped on the window while the lesson pane shows is still chosen by Create (whatever the view), which opens
/// its panel beside the lesson pane.
#[test]
fn a_dropped_file_goes_to_create_beside_the_lesson_pane() {
    let mut h = harness_with(Patch::init(), false);
    h.run_steps(2);
    click(&mut h, "Learn");
    assert!(lesson(&h).is_some());
    let path = PathBuf::from("/sounds/pad.flac");
    h.input_mut()
        .dropped_files
        .push(Arc::new(Dropped(path.clone())));
    h.run_steps(3);
    assert_eq!(h.state().editor.create().file(), Some(&path));
    assert!(
        h.query_by_label("Start create").is_some(),
        "the panel opened"
    );
    assert!(lesson(&h).is_some(), "the lesson pane stays");
}

/// Both panes open at 1200 × 800 with Init (lavapipe): the cards, the lesson browser, the Create panel at the right.
#[test]
fn learn_and_create_snapshot() {
    let mut h = harness_with(Patch::init(), true);
    h.run_steps(2);
    click(&mut h, "Learn");
    click(&mut h, "Create");
    h.run_steps(3);
    h.snapshot("learn_create_open");
}
