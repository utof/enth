//! The Create view (lane PUI, feature `create`) over a real `Session`, by accessibility label, the way the plugin's
//! glue drives it each frame (`enth-plugin/src/editor.rs`): the shell's posts handed back (`Session::create_post`), the
//! create view handed over (`Editor::set_create`), the frame, the view's intents applied (`Editor::apply_create`), and
//! the folder the browser asked for listed (`Editor::take_create_browse`, `Editor::set_create_folder`).
//!
//! The test plays the shell: it takes the `Run`, posts the intake of a real clip (`enth_analysis`'s intake on a
//! synthetic chord, so its warning is real), rounds of K = 8 candidates through successive halving's rungs (8, then 4,
//! then 2), and the end; it records the auditions, the upload's and the cancels it is asked for, and lists folders
//! from a table. **Nothing plays**: no shell renders or mixes anything here.
//!
//! - the flow from a drop: candidates, select, audition, A/B against the upload, Use this, undo;
//! - the flow from the folder browser (open a folder, go up, choose a file), and a short clip shown as it is;
//! - cancel (one request, at once) and a failure's message and hint;
//! - the overflow report at 1200 × 800 with Create open, for Init and the P1 default, in every state;
//! - no idle frames in any state;
//! - snapshots (lavapipe).
#![cfg(feature = "create")]

mod common;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use egui::{UiBuilder, Vec2, ViewportId};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use enth_patch::{Op, ParamAddr, Patch};
use enth_session::Session;
use enth_session::create::{
    Candidate, CandidateState, CreateEdit, CreateIntake, CreateJobId, CreatePhase, CreatePost,
    CreateProgress, CreateRequest, CreateStop, Intake, Provenance, Which,
};
use enth_ui::Theme;
use enth_ui::editor::Editor;
use enth_ui::editor::create::{BrowseAt, EntryKind, Folder, FolderEntry};

struct App {
    theme: Theme,
    editor: Editor,
    session: Session,
    /// The shell's queue: posts to hand back at the next frame.
    posts: Vec<(CreateJobId, CreatePost)>,
    /// What the shell was asked for.
    asked: Vec<CreateRequest>,
    /// The folders the shell lists, by path (`BrowseAt::Start` is `/home/u/Documents`).
    folders: BTreeMap<PathBuf, Folder>,
    /// What the browser asked to list.
    browsed: Vec<BrowseAt>,
}

const START: &str = "/home/u/Documents";

fn harness_with(patch: Patch, render: bool) -> Harness<'static, App> {
    let theme = Theme::dark();
    let app = App {
        editor: Editor::new(&theme),
        theme: theme.clone(),
        session: Session::new(patch),
        posts: Vec::new(),
        asked: Vec::new(),
        folders: folders(),
        browsed: Vec::new(),
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
            for (id, post) in std::mem::take(&mut app.posts) {
                let _ = app.session.create_post(id, post);
            }
            app.editor.set_create(CreateEdit::create(&app.session));
            app.editor
                .frame(&mut root, &app.theme, &mut app.session, down);
            app.editor.apply_create(&mut app.session);
            if let Some(at) = app.editor.take_create_browse() {
                let path = match &at {
                    BrowseAt::Start => PathBuf::from(START),
                    BrowseAt::Path(p) => p.clone(),
                };
                let folder = app
                    .folders
                    .get(&path)
                    .cloned()
                    .unwrap_or_else(|| Folder::unreadable(path, "no such folder"));
                app.editor.set_create_folder(folder);
                app.browsed.push(at);
            }
            app.session.drain_requests().for_each(drop);
            app.asked.extend(app.session.drain_create_requests());
        },
        app,
    )
}

fn harness() -> Harness<'static, App> {
    harness_with(Patch::init(), false)
}

fn entry(dir: &str, name: &str, kind: EntryKind, bytes: Option<u64>) -> FolderEntry {
    FolderEntry {
        name: name.to_owned(),
        path: Path::new(dir).join(name),
        kind,
        bytes,
    }
}

fn folders() -> BTreeMap<PathBuf, Folder> {
    let samples = format!("{START}/Samples");
    let mut many: Vec<FolderEntry> = (1..=30)
        .map(|i| {
            entry(
                START,
                &format!("take {i:02}.wav"),
                EntryKind::Sound,
                Some(96_000),
            )
        })
        .collect();
    many.push(entry(START, "Samples", EntryKind::Folder, None));
    many.push(entry(START, "notes.txt", EntryKind::Sound, Some(10)));
    BTreeMap::from([
        (PathBuf::from(START), Folder::new(START.into(), many)),
        (
            PathBuf::from(&samples),
            Folder::new(
                samples.clone().into(),
                vec![
                    entry(&samples, "pad.flac", EntryKind::Sound, Some(1_300_000)),
                    entry(&samples, "click.wav", EntryKind::Sound, Some(1_000)),
                ],
            ),
        ),
    ])
}

fn click(h: &mut Harness<'_, App>, label: &str) {
    h.get_by_label(label).click();
    h.run_steps(3);
}

fn has(h: &Harness<'_, App>, label: &str) -> bool {
    h.query_by_label(label).is_some()
}

fn disabled(h: &Harness<'_, App>, label: &str) -> bool {
    h.get_by_label(label).accesskit_node().is_disabled()
}

fn level() -> ParamAddr {
    "osc1.level".parse().unwrap()
}

/// A clip at 48 kHz: a decaying A minor triad (220, 261.6, 329.6 Hz) of `seconds`, so the intake finds a chord.
fn chord(seconds: f32) -> Vec<f32> {
    let sr = 48_000.0_f32;
    let n = (seconds * sr) as usize;
    (0..n)
        .map(|i| {
            let t = i as f32 / sr;
            let tone = |f: f32| (std::f32::consts::TAU * f * t).sin();
            (tone(220.0) + tone(261.63) + tone(329.63)) * 0.25 * (-3.0 * t).exp()
        })
        .collect()
}

/// The intake of `audio`, as the shell posts it.
fn intake(audio: &[f32], notes: Vec<String>) -> CreateIntake {
    let it = Intake::new(audio, 1, 48_000).expect("the intake reads the clip");
    CreateIntake::new("WAV, 16-bit, 48000 Hz, mono", it.report, notes)
}

/// Candidate `branch` of K = 8: its patch has `osc1.level` = 10 + 10 · branch, so "Use this" shows which was taken.
fn candidate(branch: usize, distance: f64, state: CandidateState, rung: usize) -> Candidate {
    let mut patch = Patch::init();
    enth_patch::apply_all(
        &mut patch,
        &[Op::SetParam {
            addr: level(),
            value: 10.0 + 10.0 * branch as f32,
        }],
    )
    .unwrap();
    Candidate {
        branch,
        start: branch,
        provenance: match branch {
            0 => Provenance::Heuristic,
            1..=3 => Provenance::Template {
                name: ["pluck_1osc", "pad_2osc", "bass_sub"][branch - 1].to_owned(),
            },
            _ => Provenance::Knn {
                rank: branch - 4,
                bank_index: 100 + branch,
                seed: 7,
                difficulty: 2,
                musical: 50,
                embedding_distance: 0.1,
            },
        },
        proposal: Patch::init(),
        proposal_distance: Some(distance * 3.0),
        patch,
        distance,
        state,
        rung,
        renders: 50 * (rung + 1),
        runs: 1,
        trace: Vec::new(),
    }
}

/// Round 1 (every branch at rung 0), round 2 (four pruned, four on rung 1) and the end (two finalists on rung 2).
fn round(stage: usize) -> Vec<Candidate> {
    // Branch 3 is the best throughout, then 5.
    let distance = |b: usize| [0.61, 0.55, 0.72, 0.21, 0.66, 0.34, 0.58, 0.70][b];
    (0..8)
        .map(|b| {
            let d = distance(b) / (stage as f64 + 1.0);
            let survives1 = [3, 5, 1, 6].contains(&b);
            let survives2 = [3, 5].contains(&b);
            match stage {
                0 => candidate(b, d, CandidateState::Searching, 0),
                1 if survives1 => candidate(b, d, CandidateState::Searching, 1),
                1 => candidate(b, distance(b), CandidateState::Pruned, 0),
                _ if survives2 => candidate(b, d, CandidateState::Done, 2),
                _ if survives1 => candidate(b, distance(b) / 2.0, CandidateState::Pruned, 1),
                _ => candidate(b, distance(b), CandidateState::Pruned, 0),
            }
        })
        .collect()
}

/// A file dropped on the window, as an integration hands it to egui (only its path is read).
#[derive(Debug)]
struct Dropped(PathBuf);

impl egui::DroppedFile for Dropped {
    fn path(&self) -> &Path {
        &self.0
    }

    fn bytes(&self) -> Result<Vec<u8>, String> {
        Err("the view never reads the file".to_owned())
    }
}

fn drop_file(h: &mut Harness<'_, App>, path: &str) {
    h.input_mut()
        .dropped_files
        .push(Arc::new(Dropped(PathBuf::from(path))));
    h.run_steps(3);
}

/// The id of the `Run` the shell was asked for last.
fn run_id(h: &Harness<'_, App>) -> (CreateJobId, String) {
    h.state()
        .asked
        .iter()
        .rev()
        .find_map(|r| match r {
            CreateRequest::Run(run) => Some((run.id, run.source.name())),
            _ => None,
        })
        .expect("a Run went out")
}

fn post(h: &mut Harness<'_, App>, id: CreateJobId, post: CreatePost) {
    h.state_mut().posts.push((id, post));
    h.run_steps(3);
}

fn progress(rounds: usize, renders: usize, seconds: f64) -> CreateProgress {
    CreateProgress::new(rounds, renders, 4000, seconds, 60.0)
}

/// What the shell was asked to play since `from`, as words.
fn plays(h: &Harness<'_, App>, from: usize) -> Vec<String> {
    h.state().asked[from..]
        .iter()
        .filter_map(|r| match r {
            CreateRequest::Audition { branch, which, .. } => Some(format!(
                "branch {branch} {}",
                if *which == Which::Refined {
                    "refined"
                } else {
                    "start"
                }
            )),
            CreateRequest::AuditionUpload { .. } => Some("upload".to_owned()),
            CreateRequest::StopAudition => Some("stop".to_owned()),
            _ => None,
        })
        .collect()
}

/// Drop a sound and run its job to the end: the intake (a real chord), two rounds and the end.
fn job_to_the_end(h: &mut Harness<'_, App>) -> CreateJobId {
    h.run_steps(2);
    drop_file(h, "/sounds/pluck.wav");
    let (id, _) = run_id(h);
    post(
        h,
        id,
        CreatePost::Intake(Box::new(intake(
            &chord(0.8),
            vec!["objective: logmel-pool85-robust/1".to_owned()],
        ))),
    );
    for stage in 0..2 {
        post(
            h,
            id,
            CreatePost::Round {
                progress: progress(stage + 1, 400 * (stage + 1), 0.7 * (stage + 1) as f64),
                candidates: round(stage),
            },
        );
    }
    post(
        h,
        id,
        CreatePost::Done {
            progress: progress(9, 4000, 31.0),
            stop: CreateStop::Budget,
            candidates: round(2),
            notes: Vec::new(),
        },
    );
    id
}

/// **The flow from a drop** (PUI's acceptance): a file dropped anywhere on the window opens the panel and starts a job;
/// the intake's findings, the rounds through the rungs, a tile selected, its candidate and start played, the A/B
/// against the upload both ways, Stop, "Use this" as one undo step named after the file, and the header's Undo giving
/// the patch before back.
#[test]
fn the_flow_from_a_drop_to_use_this_and_undo() {
    let mut h = harness();
    h.run_steps(2);
    assert!(!has(&h, "Start create"), "the panel is closed");
    let before = h.state().session.patch().clone();
    drop_file(&mut h, "/sounds/pluck.wav");
    let (id, name) = run_id(&h);
    assert_eq!(name, "pluck.wav", "the drop started a job at once");
    assert!(has(&h, "Drop zone: pluck.wav"));
    assert!(has(&h, "Reading the file…"));
    assert!(disabled(&h, "Start create"), "one job at a time");
    assert!(!disabled(&h, "Cancel create"));

    // The intake: how it decoded, the note, the length, the chord warning.
    let it = intake(
        &chord(0.8),
        vec!["objective: logmel-pool85-robust/1".to_owned()],
    );
    let warnings = it.warnings();
    assert!(
        warnings.iter().any(|w| w.contains("pitches at once")),
        "the intake finds the chord: {warnings:?}"
    );
    post(&mut h, id, CreatePost::Intake(Box::new(it.clone())));
    assert!(has(&h, "Searching…"));
    assert!(has(&h, "WAV, 16-bit, 48000 Hz, mono"));
    for w in &warnings {
        assert!(has(&h, &format!("• {w}")), "warning {w:?} shows");
    }
    assert!(
        h.query_by_label_contains("Drop zone: pluck.wav 0.80 s")
            .is_some(),
        "the drop zone shows the length"
    );

    // Round 1 (0.7 s in): eight tiles, all searching on rung 0.
    post(
        &mut h,
        id,
        CreatePost::Round {
            progress: progress(1, 400, 0.7),
            candidates: round(0),
        },
    );
    for n in 1..=8 {
        assert!(has(&h, &format!("Select candidate {n}")), "tile {n}");
    }
    assert!(has(&h, "Rungs 8"));
    assert!(has(&h, "400 of 4,000 renders • 1 of 60 s"));
    // The best is selected until a tile is clicked.
    assert!(has(&h, "Candidate 1 • template bass_sub"));

    // Round 2: four pruned, four on rung 1.
    post(
        &mut h,
        id,
        CreatePost::Round {
            progress: progress(2, 800, 1.4),
            candidates: round(1),
        },
    );
    assert!(has(&h, "Rungs 8 › 4"));

    // Select the second best (branch 5, a k-NN start) and compare it with the upload.
    click(&mut h, "Select candidate 2");
    assert!(has(&h, "Candidate 2 • k-NN 2"));
    let from = h.state().asked.len();
    click(&mut h, "Play candidate 2");
    click(&mut h, "A/B candidate 2 against the upload");
    assert!(has(&h, "Played last: the upload"));
    click(&mut h, "A/B candidate 2 against the upload");
    assert!(has(&h, "Played last: candidate 2"));
    click(&mut h, "Play the start of candidate 2");
    click(&mut h, "A/B candidate 2 against the upload");
    click(&mut h, "A/B candidate 2 against the upload");
    click(&mut h, "Play the upload");
    click(&mut h, "Stop audition");
    assert_eq!(
        plays(&h, from),
        [
            "branch 5 refined",
            "upload",
            "branch 5 refined",
            "branch 5 start",
            "upload",
            "branch 5 start",
            "upload",
            "stop"
        ],
        "A/B alternates the candidate (as last played) and the upload"
    );
    assert!(disabled(&h, "Stop audition"));

    // The end: two finalists on rung 2.
    post(
        &mut h,
        id,
        CreatePost::Done {
            progress: progress(9, 4000, 31.0),
            stop: CreateStop::Budget,
            candidates: round(2),
            notes: vec!["stopped at the budget".to_owned()],
        },
    );
    assert!(has(&h, "Done"));
    assert!(has(&h, "Rungs 8 › 4 › 2"));
    assert!(disabled(&h, "Cancel create"));
    assert!(!disabled(&h, "Start create"), "a run again is possible");

    // Use this: one undo step, named after the file, no host write.
    click(&mut h, "Select candidate 1");
    click(&mut h, "Use candidate 1");
    let s = &h.state().session;
    assert_eq!(s.patch().value(&level()), Some(40.0), "branch 3's patch");
    assert_eq!(s.patch().meta.name, "pluck");
    assert!(s.history().can_undo());
    assert!(has(&h, "Took candidate 1"));
    assert_eq!(
        h.state().session.create().and_then(|v| v.used),
        Some((3, Which::Refined))
    );
    // Undo (the header's) gives the patch before back.
    click(&mut h, "Undo");
    assert_eq!(
        h.state().session.patch(),
        &before,
        "undo restores the patch"
    );
    assert!(!h.state().session.history().can_undo(), "it was one step");
    // "Use the start" is one step too.
    click(&mut h, "Use the start of candidate 1");
    assert_eq!(
        h.state().session.patch().value(&level()),
        before.value(&level())
    );
    assert_eq!(h.state().session.patch().meta.name, "pluck");
    click(&mut h, "Undo");
    assert_eq!(h.state().session.patch(), &before);
}

/// **The flow from the folder browser**: Browse… lists the shell's starting folder, a folder opens, ◀ Up goes back, a
/// sound file is chosen (the browser closes and the job starts), a short clip is shown as it is (milliseconds,
/// "short: kept whole"), and a typed path chooses a sound file or opens a folder.
#[test]
fn the_flow_from_the_folder_browser() {
    let mut h = harness();
    h.run_steps(2);
    click(&mut h, "Create");
    assert!(has(
        &h,
        "Drop zone: Drop a sound file here WAV, FLAC, Ogg Vorbis or MP3, any length"
    ));
    assert!(disabled(&h, "Start create"), "nothing chosen yet");
    click(&mut h, "Browse files");
    assert_eq!(h.state().browsed, [BrowseAt::Start]);
    assert!(has(&h, "Open folder Samples"));
    assert!(has(&h, "Choose take 01.wav"));
    assert!(!has(&h, "Choose notes.txt"), "only sound files are listed");
    click(&mut h, "Open folder Samples");
    assert!(has(&h, "Choose pad.flac"));
    assert!(has(&h, "Choose click.wav"));
    click(&mut h, "Parent folder");
    assert!(has(&h, "Open folder Samples"));
    // A typed folder opens (a typed sound file is chosen: `create_panel.rs`).
    h.get_by_label("Sound file path").click();
    h.run();
    h.get_by_label("Sound file path")
        .type_text(&format!("{START}/Samples"));
    h.run_steps(2);
    click(&mut h, "Choose file");
    assert!(has(&h, "Choose click.wav"), "the typed folder opened");
    click(&mut h, "Choose click.wav");
    assert!(!has(&h, "Close browser"), "the browser closed");
    let (id, name) = run_id(&h);
    assert_eq!(name, "click.wav", "the pick started a job at once");
    assert_eq!(
        h.state().browsed,
        [
            BrowseAt::Start,
            BrowseAt::Path(format!("{START}/Samples").into()),
            BrowseAt::Path(START.into()),
            BrowseAt::Path(format!("{START}/Samples").into()),
        ]
    );
    // A 20 ms click: kept, padded, shown in milliseconds.
    let it = intake(&chord(0.02), Vec::new());
    assert!(
        it.report.span.tail_pad_s > 0.0,
        "the intake pads a short clip"
    );
    post(&mut h, id, CreatePost::Intake(Box::new(it)));
    assert!(
        h.query_by_label_contains("Drop zone: click.wav 20 ms")
            .is_some(),
        "the length in milliseconds"
    );
    assert!(
        h.query_by_label_contains("kept whole").is_some(),
        "a short sound is kept, never refused"
    );
    // The browser opens again where the chosen file is.
    click(&mut h, "Browse files");
    assert_eq!(
        h.state().browsed.last(),
        Some(&BrowseAt::Path(format!("{START}/Samples").into()))
    );
    click(&mut h, "Close browser");
    assert!(has(&h, "Reading the file…") || has(&h, "Searching…"));
}

/// **Cancel stays responsive**: one click sends one `Cancel` at the end of that frame, the phase reads "Cancelling…"
/// and Cancel is disabled until the shell's last post; the candidates it found stay playable and usable. A failure
/// shows its message, code and hint.
#[test]
fn cancel_goes_out_at_once_and_a_failure_says_what_to_do() {
    let mut h = harness();
    h.run_steps(2);
    drop_file(&mut h, "/sounds/pluck.wav");
    let (id, _) = run_id(&h);
    post(
        &mut h,
        id,
        CreatePost::Round {
            progress: progress(1, 400, 0.7),
            candidates: round(0),
        },
    );
    let before = h.state().asked.len();
    h.get_by_label("Cancel create").click();
    h.step();
    let cancels: Vec<&CreateRequest> = h.state().asked[before..]
        .iter()
        .filter(|r| matches!(r, CreateRequest::Cancel(_)))
        .collect();
    assert_eq!(
        cancels,
        [&CreateRequest::Cancel(id)],
        "one Cancel, in the click's frame"
    );
    h.run_steps(2);
    assert!(has(&h, "Cancelling…"));
    assert!(disabled(&h, "Cancel create"));
    h.get_by_label("Cancel create").click();
    h.run_steps(2);
    assert_eq!(
        h.state().asked[before..]
            .iter()
            .filter(|r| matches!(r, CreateRequest::Cancel(_)))
            .count(),
        1,
        "never twice"
    );
    post(
        &mut h,
        id,
        CreatePost::Done {
            progress: progress(1, 400, 0.8),
            stop: CreateStop::Cancelled,
            candidates: round(0),
            notes: vec!["cancelled".to_owned()],
        },
    );
    assert!(has(&h, "Stopped"));
    assert_eq!(
        h.state().session.create().map(|v| v.phase),
        Some(CreatePhase::Done(CreateStop::Cancelled))
    );
    click(&mut h, "Use candidate 1");
    assert_eq!(h.state().session.patch().value(&level()), Some(40.0));

    // A file that does not decode.
    click(&mut h, "Start create");
    let (id, _) = run_id(&h);
    post(
        &mut h,
        id,
        CreatePost::Failed {
            code: "bad_file".to_owned(),
            message: "the file is not a sound this build reads".to_owned(),
            hint: "use WAV, FLAC, Ogg Vorbis or MP3".to_owned(),
        },
    );
    assert!(has(&h, "Could not create"));
    assert!(has(
        &h,
        "✗ the file is not a sound this build reads (bad_file)"
    ));
    assert!(has(&h, "use WAV, FLAC, Ogg Vorbis or MP3"));
}

/// The states the overflow and idle checks visit.
#[derive(Clone, Copy, Debug)]
enum State {
    Empty,
    Browsing,
    Running,
    Done,
}

/// The harness in `state`, the panel open.
fn in_state(patch: Patch, state: State, render: bool) -> Harness<'static, App> {
    let mut h = harness_with(patch, render);
    h.run_steps(2);
    match state {
        State::Empty => click(&mut h, "Create"),
        State::Browsing => {
            click(&mut h, "Create");
            click(&mut h, "Browse files");
        }
        State::Running => {
            drop_file(&mut h, "/sounds/pluck.wav");
            let (id, _) = run_id(&h);
            post(
                &mut h,
                id,
                CreatePost::Intake(Box::new(intake(
                    &chord(0.8),
                    vec![
                        "no k-NN bank (enth-bank-v1.bin) found (looked in /home/u/.clap/enth-bank-v1.bin, /home/u/Documents/enth/Models/enth-bank-v1.bin): running on the heuristic and the templates".to_owned(),
                        "objective: logmel-pool85-robust/1".to_owned(),
                    ],
                ))),
            );
            post(
                &mut h,
                id,
                CreatePost::Round {
                    progress: progress(2, 800, 1.4),
                    candidates: round(1),
                },
            );
            click(&mut h, "Select candidate 2");
            click(&mut h, "Play candidate 2");
        }
        State::Done => {
            job_to_the_end(&mut h);
            click(&mut h, "A/B candidate 1 against the upload");
            click(&mut h, "A/B candidate 1 against the upload");
            click(&mut h, "Use candidate 1");
        }
    }
    h.run_steps(3);
    h
}

fn overflow(h: &Harness<'_, App>) -> Vec<(&'static str, Vec2)> {
    h.state().editor.report().overflow.clone()
}

/// **The overflow report at 1200 × 800 with Create open** (`Editor::report()`): the panel itself never scrolls, in any
/// state (the browser's list scrolls inside itself); what the layout beside it reports is the P1 default's band (276 px
/// sideways) and overview (111 and 63 px), as lane SK3M measured (`learn_create.rs`), and nothing for Init or for the
/// candidate taken in the finished state.
#[test]
fn nothing_in_the_panel_scrolls_at_1200_by_800() {
    for state in [State::Empty, State::Browsing, State::Running, State::Done] {
        let h = in_state(Patch::init(), state, false);
        let over = overflow(&h);
        println!("Init, {state:?}: {over:?}");
        assert_eq!(over, Vec::new(), "Init beside the panel, {state:?}");
        let h = in_state(common::patches::p1_default(), state, false);
        let over = overflow(&h);
        println!("P1 default, {state:?}: {over:?}");
        assert!(
            over.iter().all(|(name, _)| *name != "create"),
            "the panel does not scroll, {state:?}: {over:?}"
        );
        let mut names: Vec<&str> = over.iter().map(|(n, _)| *n).collect();
        names.sort_unstable();
        if matches!(state, State::Done) {
            // "Use this" took a candidate (Init with another level), which fits beside the panel.
            assert_eq!(names, Vec::<&str>::new(), "{state:?}");
        } else {
            assert_eq!(
                names,
                ["modulators", "overview.matrix", "overview.tiles"],
                "the layout beside the panel, as without a job, {state:?}"
            );
        }
    }
}

/// **No idle frames**: in every state (a job running with an audition asked for, a finished one with the upload
/// played last and a candidate taken, the browser open), an idle editor's frames ask for no repaint. Progress comes
/// from the shell's posts, which wake the editor.
#[test]
fn an_idle_create_view_asks_for_no_frames() {
    for state in [State::Empty, State::Browsing, State::Running, State::Done] {
        let mut h = in_state(Patch::init(), state, false);
        // A text field that held the keyboard focus blinks its cursor: Escape lets it go first.
        h.key_press(egui::Key::Escape);
        h.run_steps(5);
        for n in 0..5 {
            h.step();
            let delay = h.output().viewport_output[&ViewportId::ROOT].repaint_delay;
            assert_eq!(
                delay,
                Duration::MAX,
                "frame {n} of an idle Create view ({state:?}) asked for a repaint"
            );
        }
    }
}

/// A snapshot at 1200 × 800 over Init (lavapipe) of the panel in `state`.
fn snapshot(state: State, name: &str) {
    let mut h = in_state(Patch::init(), state, true);
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    h.snapshot(name);
}

/// The empty panel: the drop zone, Browse… and a disabled Start.
#[test]
fn create_empty_snapshot() {
    snapshot(State::Empty, "create_empty");
}

/// The folder browser: the starting folder's 30 sound files and a folder, scrolling inside the list.
#[test]
fn create_browser_snapshot() {
    snapshot(State::Browsing, "create_browser");
}

/// A job running: the intake's findings and chord warning, round 2's progress and rungs, eight tiles (four pruned), a
/// tile selected and its candidate played, the shell's notes.
#[test]
fn create_running_snapshot() {
    snapshot(State::Running, "create_running");
}

/// A finished job: two finalists, the A/B with the upload played last, a candidate taken.
#[test]
fn create_done_snapshot() {
    snapshot(State::Done, "create_done");
}
