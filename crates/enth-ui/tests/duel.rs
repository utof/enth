//! The duel card through the editor (lane SKD's minimal card, F2d; lane DUI grows it): kittest drives the controls by
//! label over a real `Session`, the way the plugin's glue does each frame: the round's clock and results, the round
//! view and the duel read model handed over (`Editor::set_round`, `Editor::set_duel`), the frame, then the round and
//! duel intents applied (`Editor::apply_game`, `Editor::apply_duel`). The game worker and the duel worker are played
//! by the test between frames with the real `enth-challenge` functions the plugin's worker calls (`prepare_round`,
//! `score_round`, `verify::prepare`, `verify::verify_prepared`).
//!
//! What is checked: nothing changes until a shell says duels are available; **Duel** opens the card and × closes it;
//! the clipboard is written only inside a click on **Copy** (egui's `CopyText`) and never on its own, and **read only on
//! Paste** (lane DUI): no frame but a click on **Paste** sends egui's `RequestPaste`, and the `Event::Paste` that answers
//! it fills the box and is read; a whole chat message pasted and read imports the friend's scorecard (shown as
//! reported) and their answer, which stays sealed until the player has played, and opens after a give-up or after a
//! submit; it is checked on this computer ("as reported", the commitment matched), the two results stand side by side,
//! and it loads as one undo step that Undo takes back; the history lists its rows and **Open** asks the shell for one;
//! the friend's name goes to the shell when the field is left; an answer over 64 symbols says what older builds print;
//! an idle card asks for no frames; and the REAPER `duel` scenario's click points hit their controls.
#![cfg(all(feature = "duel", feature = "recreate"))]

mod common;

use std::time::Duration;

use egui::{OutputCommand, UiBuilder, ViewportCommand, ViewportId};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use enth_challenge::answer::AnswerCode;
use enth_challenge::result::Scorecard;
use enth_challenge::spec::{LATEST_GENERATOR, SpecPreset};
use enth_challenge::{AssistsUsed, ChallengeCode, prepare_round, score_round, verify};
use enth_patch::Patch;
use enth_session::Session;
use enth_session::duel::{
    AnswerState, DuelEdit, DuelJobResult, DuelRequest, DuelVerified, DuelVerify, Rescored,
};
use enth_session::game::{EditorView, GameEdit, GameRequest, Job, JobKind, JobResult, Prepared};
use enth_ui::Theme;
use enth_ui::editor::Editor;
use enth_ui::editor::duel::{HistoryRow, ShareCodes, ShellAsks, ShellDuel};

/// The editor, its session, and what the shell would do between frames.
struct App {
    theme: Theme,
    editor: Editor,
    session: Session,
    now: u64,
    /// The shell plays duels (calls `set_duel` every frame), as the plugin with its `duel` feature does.
    duels: bool,
    shell: ShellDuel,
    /// Share clicks the shell took.
    shares: u32,
    /// What else the card asked of the shell (a history entry to open, imported codes, a name), in order.
    asks: Vec<ShellAsks>,
    /// The clipboard writes the frames asked for, in order.
    copies: Vec<String>,
    /// Clipboard reads the frames asked the integration for (`ViewportCommand::RequestPaste`).
    paste_requests: usize,
}

fn app(duels: bool) -> App {
    let theme = Theme::dark();
    App {
        editor: Editor::new(&theme),
        theme,
        session: Session::new(common::patches::p1_default()),
        now: 0,
        duels,
        shell: ShellDuel::default(),
        shares: 0,
        asks: Vec::new(),
        copies: Vec::new(),
        paste_requests: 0,
    }
}

/// The game worker's part, synchronously.
fn run_game(job: &Job) -> JobResult {
    match &job.kind {
        JobKind::Prepare { spec } => {
            let round = prepare_round(spec).unwrap();
            JobResult::Prepared(Box::new(Prepared::from_round(&round, None)))
        }
        JobKind::Score {
            challenge, patch, ..
        } => {
            let round = enth_challenge::prepare_challenge((**challenge).clone()).unwrap();
            JobResult::Scored(Box::new(score_round(&round, patch).unwrap()))
        }
        JobKind::Loudness { .. } => JobResult::Loudness { lufs: None },
        _ => JobResult::Failed {
            message: "not in this test".to_owned(),
        },
    }
}

/// A score in the session's hundredths.
fn rescored(r: &enth_challenge::ScoreReport) -> Rescored {
    let s = enth_session::game::summary(r);
    Rescored::new(s.sound, s.combined, s.passed)
}

/// The duel worker's verify job, as the plugin's shell runs it.
fn run_verify(v: &DuelVerify) -> DuelJobResult {
    let answer = AnswerCode::decode(&v.answer).unwrap();
    let card = v.card.as_deref().map(|t| Scorecard::decode(t).unwrap());
    let round = verify::prepare(&answer, verify::scorer_set(card.as_ref())).unwrap();
    let theirs = verify::verify_prepared(&round, &answer, card.as_ref()).unwrap();
    let own = v.own.as_ref().map(|p| {
        let canonical = AnswerCode::new(*answer.challenge(), 0, p).unwrap();
        rescored(&score_round(&round, canonical.answer()).unwrap())
    });
    DuelJobResult::Verified(Box::new(
        DuelVerified::new(own, rescored(&theirs.report))
            .with_patch(answer.answer().clone())
            .with_claim(theirs.claim),
    ))
}

fn harness(app: App) -> Harness<'static, App> {
    let theme = app.theme.clone();
    Harness::builder()
        .with_size(theme.size.window.default)
        .with_pixels_per_point(1.0)
        .with_step_dt(1.0 / 60.0)
        .with_context_setup(move |ctx| theme.apply(ctx))
        .build_ui_state(
            |ui, app: &mut App| {
                let down = ui.input(|i| i.pointer.any_down());
                let rect = ui.ctx().content_rect();
                let mut root = ui.new_child(UiBuilder::new().max_rect(rect));
                app.now += 16;
                let view = if app.now == 16 {
                    EditorView::Opened
                } else {
                    EditorView::Visible
                };
                app.session.game_tick(app.now, view);
                app.editor.set_round(app.session.round());
                if app.duels {
                    app.editor
                        .set_duel(DuelEdit::duel(&app.session), app.shell.clone());
                }
                app.editor
                    .frame(&mut root, &app.theme, &mut app.session, down);
                app.editor.apply_game(&mut app.session, &mut || 42);
                if app.duels {
                    app.editor.apply_duel(&mut app.session);
                    let mut asks = app.editor.take_duel_asks();
                    if std::mem::take(&mut asks.share) {
                        app.shares += 1;
                    }
                    if asks != ShellAsks::default() {
                        app.asks.push(asks);
                    }
                }
                app.session.drain_requests().for_each(drop);
                // The workers, between frames.
                let jobs: Vec<Job> = app
                    .session
                    .drain_game_requests()
                    .filter_map(|r| match r {
                        GameRequest::Run(job) => Some(job),
                        _ => None,
                    })
                    .collect();
                for job in jobs {
                    let result = run_game(&job);
                    let _ = app.session.game_job_done(job.id, result);
                }
                let duel: Vec<DuelRequest> = app.session.drain_duel_requests().collect();
                for request in duel {
                    if let DuelRequest::Verify(v) = request {
                        let result = run_verify(&v);
                        let _ = app.session.duel_job_done(v.job, result);
                    }
                }
                app.paste_requests += ui.ctx().viewport(|v| {
                    v.commands
                        .iter()
                        .filter(|c| matches!(c, ViewportCommand::RequestPaste))
                        .count()
                });
                app.copies.extend(
                    ui.ctx()
                        .output(|o| o.commands.clone())
                        .into_iter()
                        .filter_map(|c| match c {
                            OutputCommand::CopyText(t) => Some(t),
                            _ => None,
                        }),
                );
            },
            app,
        )
}

fn click(h: &mut Harness<'_, App>, label: &str) {
    h.get_by_label(label).click();
    h.run_steps(3);
}

fn enabled(h: &Harness<'_, App>, label: &str) -> bool {
    !h.get_by_label(label).accesskit_node().is_disabled()
}

/// The friend's codes for a level-2 challenge, made as the plugin's shell makes them: a passing answer (the target, with
/// a name in its meta, which the code strips), its scorecard after 3 submits in 2:41, and the share message.
fn friends_codes() -> (ChallengeCode, ShareCodes, Patch) {
    let challenge =
        ChallengeCode::short(SpecPreset::by_id(2).unwrap().spec(LATEST_GENERATOR, 77)).unwrap();
    let round = prepare_round(challenge.spec()).unwrap();
    let mut answer = round.target().clone();
    "Somebody Real".clone_into(&mut answer.meta.author);
    let code = AnswerCode::new(challenge, 0x0123_4567_89AB_CDEF, &answer).unwrap();
    let report = score_round(&round, code.answer()).unwrap();
    let card = Scorecard::new(
        challenge,
        &report,
        &round.challenge().scoring,
        3,
        161_000,
        &AssistsUsed::default(),
        code.commitment(),
    )
    .unwrap();
    let [a, b] = card.share_lines();
    let codes = ShareCodes {
        code: challenge.to_string(),
        message: format!("{a}\n{b}\n{}", card.encode()),
        scorecard: card.encode(),
        answer: code.encode(),
        scorecard_symbols: card.encode().replace('-', "").len(),
        answer_symbols: code.symbols(),
    };
    (challenge, codes, round.target().clone())
}

/// Paste `text` into the paste box the way egui-baseview delivers the platform's Paste, then Read it.
fn paste_and_read(h: &mut Harness<'_, App>, text: &str) {
    click(h, "Paste box");
    h.event(egui::Event::Paste(text.to_owned()));
    h.run_steps(2);
    click(h, "Read pasted codes");
}

fn has_text(h: &Harness<'_, App>, part: &str) -> bool {
    h.query_all_by_label_contains(part).next().is_some()
}

#[test]
fn nothing_changes_until_a_shell_says_duels_are_available() {
    let mut h = harness(app(false));
    h.run_steps(3);
    assert!(h.query_by_label("Open duel card").is_none());
    assert_eq!(h.state().editor.report().overflow, Vec::new());
}

#[test]
fn duel_opens_the_card_and_the_cross_closes_it() {
    let mut h = harness(app(true));
    h.run_steps(3);
    click(&mut h, "Open duel card");
    assert!(h.state().editor.duel().card_open());
    assert!(
        h.query_by_label("Open duel card").is_none(),
        "the opener hides while the card shows"
    );
    assert!(has_text(&h, "Duel codes are for friends"), "the About line");
    // Nothing to copy, verify or use yet.
    for label in [
        "Copy message",
        "Copy answer",
        "Verify their answer",
        "Use their patch",
    ] {
        assert!(!enabled(&h, label), "{label} is off");
    }
    click(&mut h, "Close duel card");
    assert!(!h.state().editor.duel().card_open());
    assert!(h.query_by_label("Open duel card").is_some());
}

#[test]
fn the_clipboard_is_written_only_inside_a_copy_click_and_never_read() {
    let (_, codes, _) = friends_codes();
    let mut a = app(true);
    a.shell.codes = Some(codes.clone());
    let mut h = harness(a);
    h.run_steps(3);
    click(&mut h, "Open duel card");
    // Many frames, hovering and Share clicks: nothing is written.
    h.run_steps(10);
    click(&mut h, "Share result");
    assert_eq!(h.state().shares, 1, "Share goes to the shell");
    assert!(
        h.state().copies.is_empty(),
        "no write without a Copy click: {:?}",
        h.state().copies
    );
    click(&mut h, "Copy message");
    assert_eq!(h.state().copies, vec![codes.message.clone()]);
    click(&mut h, "Copy answer");
    assert_eq!(
        h.state().copies,
        vec![codes.message.clone(), codes.answer.clone()]
    );
    h.run_steps(10);
    assert_eq!(h.state().copies.len(), 2, "and nothing after");
    // Nothing reads the clipboard: the paste box stays empty until a paste arrives as input.
    assert_eq!(h.state().editor.duel().paste_text(), "");
}

#[test]
fn a_pasted_message_imports_their_scorecard_and_the_answer_stays_sealed_until_you_have_played() {
    let (challenge, codes, target) = friends_codes();
    let mut h = harness(app(true));
    h.run_steps(3);
    click(&mut h, "Open duel card");
    // A whole chat message, mangled the way a phone does it (a dash, a non-breaking space, a wrapped code).
    let message = format!(
        "here's mine \u{2014} beat it!\n{}\nlol",
        codes.message.replace(' ', "\u{00A0}")
    );
    paste_and_read(&mut h, &message);
    let view = DuelEdit::duel(&h.state().session).expect("a duel opened on their card");
    assert_eq!(view.code, challenge.to_string());
    assert!(view.card.is_some());
    assert!(
        has_text(&h, "as reported by their app"),
        "their claim is shown as reported"
    );
    assert_eq!(
        h.state().editor.duel().paste_text(),
        "",
        "a clean read empties the box"
    );
    // Their answer: held, sealed.
    paste_and_read(&mut h, &format!("answer: {}", codes.answer));
    let view = DuelEdit::duel(&h.state().session).unwrap();
    assert_eq!(view.answer, AnswerState::Sealed);
    assert!(view.checked.is_none());
    assert!(!enabled(&h, "Verify their answer"), "sealed: Verify is off");
    assert!(has_text(&h, "sealed until you have submitted or given up"));
    // Play it, and give up: the answer opens.
    click(&mut h, "Play this challenge");
    h.run_steps(4);
    assert!(
        h.state().session.round().is_some(),
        "the round started on the duel's code"
    );
    click(&mut h, "Reveal");
    h.run_steps(3);
    let view = DuelEdit::duel(&h.state().session).unwrap();
    assert_eq!(view.answer, AnswerState::Open);
    assert!(enabled(&h, "Verify their answer"));
    click(&mut h, "Verify their answer");
    h.run_steps(3);
    let checked = DuelEdit::duel(&h.state().session).unwrap().checked.unwrap();
    assert_eq!(
        checked.theirs.sound, 10_000,
        "their passing answer is the target"
    );
    assert_eq!(checked.matches_claim(), Some(true));
    assert_eq!(checked.commitment_ok(), Some(true));
    assert!(has_text(
        &h,
        "Checked on your computer: their patch scores 100.0 %, as reported"
    ));
    assert!(has_text(
        &h,
        "is reported by their app; enth can't check times"
    ));
    // Their patch loads once the round is over, as one undo step.
    click(&mut h, "End");
    click(&mut h, "End and restore my patch");
    assert!(h.state().session.round().is_none());
    click(&mut h, "Use their patch");
    let session = &h.state().session;
    let mut loaded = session.patch().clone();
    loaded.meta = target.meta.clone();
    loaded.engine.clone_from(&target.engine);
    assert_eq!(loaded, target, "their patch, without their meta");
    assert!(
        session
            .history()
            .next_undo()
            .is_some_and(|step| step.load.is_some()),
        "one load step"
    );
}

/// The REAPER `duel` scenario clicks these points (offsets into the plugin's window at 1200 x 800, layout "lanes"; the
/// Paste button since lane DUI, for the answer): `xtask/reaper/duel.lua`'s `POINTS` must change with them.
#[test]
fn reaper_points_hit_their_duel_controls() {
    const OPENER: (f32, f32) = (32.0, 746.0);
    const SHARE: (f32, f32) = (902.0, 104.0);
    const COPY_MESSAGE: (f32, f32) = (1002.0, 104.0);
    const COPY_ANSWER: (f32, f32) = (1105.0, 104.0);
    const PASTE: (f32, f32) = (1020.0, 156.0);
    const PASTE_BUTTON: (f32, f32) = (1156.0, 132.0);
    const READ: (f32, f32) = (882.0, 208.0);
    const VERIFY: (f32, f32) = (937.0, 208.0);
    const PLAY: (f32, f32) = (880.0, 236.0);
    const REVEAL: (f32, f32) = (505.0, 28.0);
    let hits = |h: &Harness<'_, App>, label: &str, (x, y): (f32, f32)| {
        let rect = h.get_by_label(label).rect();
        assert!(
            rect.contains(egui::pos2(x, y)),
            "{label} is at {rect:?}, not under ({x}, {y}): update duel.lua's POINTS and these"
        );
    };
    let (_, codes, _) = friends_codes();
    let mut a = app(true);
    a.shell.codes = Some(codes);
    let mut h = harness(a);
    h.run_steps(3);
    hits(&h, "Open duel card", OPENER);
    click(&mut h, "Open duel card");
    hits(&h, "Share result", SHARE);
    hits(&h, "Copy message", COPY_MESSAGE);
    hits(&h, "Copy answer", COPY_ANSWER);
    hits(&h, "Paste box", PASTE);
    hits(&h, "Paste from clipboard", PASTE_BUTTON);
    hits(&h, "Read pasted codes", READ);
    hits(&h, "Verify their answer", VERIFY);
    hits(&h, "Play this challenge", PLAY);
    // The friend's scorecard opens a duel; Play starts its round, and the Recreate bar's Reveal gives up.
    let (_, codes, _) = friends_codes();
    paste_and_read(&mut h, &codes.message);
    click(&mut h, "Play this challenge");
    h.run_steps(4);
    hits(&h, "Reveal", REVEAL);
}

/// **No clipboard read without Paste** (lane DUI): hovering, typing, every other click and many frames send no
/// `RequestPaste`; a click on **Paste** sends exactly one, and the integration's answer (an `Event::Paste`) fills the
/// box and is read at once. A paste into the focused box (the platform's own Paste) goes into the box once.
#[test]
fn the_clipboard_is_read_only_on_a_paste_click() {
    let (challenge, codes, _) = friends_codes();
    let mut a = app(true);
    a.shell.codes = Some(codes.clone());
    let mut h = harness(a);
    h.run_steps(3);
    click(&mut h, "Open duel card");
    h.run_steps(10);
    for label in ["Share result", "Copy message", "Copy answer", "Paste box"] {
        click(&mut h, label);
    }
    h.event(egui::Event::Text("typed".to_owned()));
    h.run_steps(3);
    click(&mut h, "Read pasted codes");
    h.key_press(egui::Key::Escape);
    h.run_steps(10);
    assert_eq!(h.state().paste_requests, 0, "no read without Paste");
    // Paste: one request, and the card waits for its answer.
    h.get_by_label("Paste from clipboard").click();
    h.run_steps(1);
    assert_eq!(h.state().paste_requests, 1, "one read, on the click");
    assert!(h.state().editor.duel().paste_pending());
    h.run_steps(10);
    assert!(has_text(&h, "Reading the clipboard"));
    assert_eq!(h.state().paste_requests, 1, "and none after");
    // The integration answers: the whole message goes in the box and is read.
    h.event(egui::Event::Paste(format!("gg \u{2014} {}", codes.message)));
    h.run_steps(3);
    assert!(!h.state().editor.duel().paste_pending());
    let view = DuelEdit::duel(&h.state().session).expect("the pasted message opened a duel");
    assert_eq!(view.code, challenge.to_string());
    assert!(view.card.is_some(), "their scorecard was read");
    assert_eq!(h.state().editor.duel().paste_text(), "", "read and cleared");
    assert!(
        h.state()
            .asks
            .iter()
            .any(|a| a.imported.iter().any(|i| i.text == codes.scorecard)),
        "the shell is handed the imported scorecard to keep: {:?}",
        h.state().asks
    );
    // The platform's Paste into the focused box: once, in the box, and no request.
    click(&mut h, "Paste box");
    h.event(egui::Event::Paste("hello".to_owned()));
    h.run_steps(3);
    assert_eq!(h.state().editor.duel().paste_text(), "hello");
    assert_eq!(h.state().paste_requests, 1);
}

/// The sealed answer opens after a **submit**, not only after a give-up; while the round runs with nothing submitted it
/// stays sealed, and Verify stays off.
#[test]
fn the_sealed_answer_opens_after_a_submit() {
    let (_, codes, _) = friends_codes();
    let mut h = harness(app(true));
    h.run_steps(3);
    click(&mut h, "Open duel card");
    paste_and_read(&mut h, &format!("{}\n{}", codes.message, codes.answer));
    assert_eq!(
        DuelEdit::duel(&h.state().session).unwrap().answer,
        AnswerState::Sealed
    );
    click(&mut h, "Play this challenge");
    h.run_steps(4);
    assert!(h.state().session.round().is_some());
    h.run_steps(5);
    assert_eq!(
        DuelEdit::duel(&h.state().session).unwrap().answer,
        AnswerState::Sealed,
        "a live round with nothing submitted keeps it sealed"
    );
    assert!(!enabled(&h, "Verify their answer"));
    click(&mut h, "Submit");
    h.run_steps(4);
    let view = DuelEdit::duel(&h.state().session).unwrap();
    assert_eq!(view.answer, AnswerState::Open, "a scored submit opens it");
    assert!(enabled(&h, "Verify their answer"));
    click(&mut h, "Verify their answer");
    h.run_steps(3);
    assert!(has_text(
        &h,
        "Checked on your computer: their patch scores 100.0 %, as reported"
    ));
    // Side by side: their sound checked here, yours from your submit re-scored on the same check.
    assert!(
        h.query_by_label("Their sound: 100.0 %, checked on your computer")
            .is_some()
    );
    let own = DuelEdit::duel(&h.state().session)
        .unwrap()
        .checked
        .unwrap()
        .own
        .expect("your submit re-scored beside theirs");
    assert!(
        h.query_by_label(&format!(
            "Your sound: {:.1} %, checked on your computer",
            f64::from(own.sound) / 100.0
        ))
        .is_some()
    );
    assert!(h.query_by_label("Their time: 2:41, reported").is_some());
    assert!(h.query_by_label("Their submits: 3").is_some());
}

/// Their patch loads as **one undo step**: Undo brings the patch from before back.
#[test]
fn their_patch_loads_as_one_undo_step() {
    let (_, codes, target) = friends_codes();
    let mut h = harness(app(true));
    h.run_steps(3);
    click(&mut h, "Open duel card");
    paste_and_read(&mut h, &format!("{}\n{}", codes.message, codes.answer));
    click(&mut h, "Play this challenge");
    h.run_steps(4);
    click(&mut h, "Reveal");
    h.run_steps(3);
    click(&mut h, "Verify their answer");
    h.run_steps(3);
    click(&mut h, "End");
    click(&mut h, "End and restore my patch");
    let before = h.state().session.patch().clone();
    click(&mut h, "Use their patch");
    assert!(has_text(&h, "Their patch is loaded: Undo takes it back."));
    assert!(
        h.state()
            .session
            .history()
            .next_undo()
            .is_some_and(|step| step.load.is_some()),
        "a load step"
    );
    let mut loaded = h.state().session.patch().clone();
    loaded.meta = target.meta.clone();
    loaded.engine.clone_from(&target.engine);
    assert_eq!(loaded, target);
    click(&mut h, "Undo");
    assert_eq!(
        h.state().session.patch(),
        &before,
        "one Undo takes their whole patch back"
    );
    assert!(
        h.state()
            .session
            .history()
            .next_redo()
            .is_some_and(|step| step.load.is_some()),
        "the load is the step undone"
    );
}

fn rows() -> Vec<HistoryRow> {
    (1..=10)
        .rev()
        .map(|id| HistoryRow {
            id,
            code: format!("1080-1000-{id:04}-AB"),
            day: 20_740 + id as u32,
            own: Some(9_000 + id as u16),
            claimed: Some(900),
            checked: (id % 2 == 0).then_some(9_010),
            as_reported: (id % 2 == 0).then_some(true),
            name: if id == 10 {
                "Sam".to_owned()
            } else {
                String::new()
            },
        })
        .collect()
}

/// The history lists the newest rows (the rest are counted), each with **Open**, which asks the shell to reopen it.
#[test]
fn the_history_lists_its_rows_and_open_asks_the_shell() {
    let mut a = app(true);
    a.shell.history = rows();
    let mut h = harness(a);
    h.run_steps(3);
    click(&mut h, "Open duel card");
    assert!(has_text(
        &h,
        "2026-10-24 · 1080-1000-0010-AB · you 90.1 % · them 90.1 % checked here, as reported · from Sam"
    ));
    assert!(has_text(&h, "them 90.0 % reported"));
    assert!(has_text(&h, "2 older duels are kept in the history file."));
    assert!(
        h.query_by_label("Open duel 2").is_none(),
        "only the newest rows"
    );
    click(&mut h, "Open duel 7");
    assert_eq!(
        h.state().asks.last().and_then(|a| a.open),
        Some(7),
        "the shell reopens it"
    );
}

/// The friend's name, typed for the open duel, goes to the shell when the field is left; the side-by-side grid heads
/// their column with it.
#[test]
fn a_typed_name_goes_to_the_shell_when_the_field_is_left() {
    let (_, codes, _) = friends_codes();
    let mut h = harness(app(true));
    h.run_steps(3);
    click(&mut h, "Open duel card");
    assert!(
        h.query_by_label("Friend's name").is_none(),
        "no name field without a duel"
    );
    paste_and_read(&mut h, &codes.message);
    click(&mut h, "Friend's name");
    h.event(egui::Event::Text("Sam".to_owned()));
    h.run_steps(2);
    assert!(
        h.state().asks.iter().all(|a| a.name.is_none()),
        "nothing while typing"
    );
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert_eq!(
        h.state().asks.last().and_then(|a| a.name.clone()),
        Some("Sam".to_owned())
    );
    // The shell hands the kept name back with the entry.
    h.state_mut().shell.name = "Sam".to_owned();
    h.state_mut().shell.entry = Some(1);
    h.run_steps(3);
    assert!(h.query_by_label("Column: Sam").is_some());
}

/// An answer code over 64 symbols says what an enth from before duels prints for it.
#[test]
fn a_long_answer_says_older_builds_read_bad_length() {
    let (_, mut codes, _) = friends_codes();
    let mut a = app(true);
    a.shell.codes = Some(codes.clone());
    let mut h = harness(a);
    h.run_steps(3);
    click(&mut h, "Open duel card");
    assert!(
        !has_text(&h, "\"bad length\""),
        "{} symbols",
        codes.answer_symbols
    );
    codes.answer_symbols = 68;
    h.state_mut().shell.codes = Some(codes);
    h.run_steps(3);
    assert!(has_text(
        &h,
        "an enth from before duels calls it \"bad length\""
    ));
}

/// **0 idle frames**: the card open with a duel checked, its history listed, and with a Paste click waiting, asks for
/// no repaint once input stops.
#[test]
fn an_idle_duel_card_asks_for_no_frames() {
    let (_, codes, _) = friends_codes();
    let mut a = app(true);
    a.shell.history = rows();
    a.shell.codes = Some(codes.clone());
    let mut h = harness(a);
    h.run_steps(3);
    click(&mut h, "Open duel card");
    let idle = |h: &mut Harness<'_, App>, what: &str| {
        h.key_press(egui::Key::Escape);
        h.run_steps(10);
        for n in 0..4 {
            h.step();
            let delay = h.output().viewport_output[&ViewportId::ROOT].repaint_delay;
            assert_eq!(
                delay,
                Duration::MAX,
                "{what}: frame {n} asked for a repaint"
            );
        }
    };
    idle(&mut h, "the card with its history");
    paste_and_read(&mut h, &format!("{}\n{}", codes.message, codes.answer));
    idle(&mut h, "a sealed answer");
    click(&mut h, "Play this challenge");
    h.run_steps(4);
    click(&mut h, "Reveal");
    h.run_steps(3);
    click(&mut h, "Verify their answer");
    h.run_steps(3);
    click(&mut h, "End");
    click(&mut h, "End and restore my patch");
    idle(&mut h, "a checked duel, side by side");
    h.get_by_label("Paste from clipboard").click();
    idle(&mut h, "a Paste click waiting");
}
