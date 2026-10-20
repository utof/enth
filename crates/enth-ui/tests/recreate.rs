//! The Recreate round through the editor (lane SKR's seam, F2r; lane RUI's challenge bar, results drawer, settings page
//! and score overlay): kittest drives the controls by label over a real `Session`, the way the plugin's glue does each
//! frame (docs/editor.md, "The Recreate hook"): a clock tick, the worker's results handed back through
//! `Editor::hold_scores`, the round view handed over (`Editor::set_round`), the frame, then the round intents applied
//! to the session's `GameEdit` (`Editor::apply_game`). The game worker is played by the test between frames, running
//! the real `enth-challenge` functions (`prepare_round`, `score_round` with lane SS's `sound/1`, `score_structure`
//! with lane ST's `structure/1`); the coach's hint job answers with a fixed text (lane SN's coach is the plugin's), and,
//! when a test names a control (`App::hint_at`), with what the hint points at, filled at every level as no coach would:
//! the session keeps it on the level ladder (lane HINT, FU1).
//!
//! Recreate → Start → Target ▶ → Mine ▶ → A/B → Submit → the blind rating (the score held back until it is given) →
//! the results drawer and the rings → Hint → the vote → Disagree… → Reveal → End, and the round log's lines.

mod common;

use egui::UiBuilder;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use enth_challenge::spec::{LATEST_GENERATOR, SpecPreset};
use enth_challenge::{
    Agreement, ChallengeCode, PreparedRound, prepare_round, score_round, score_structure,
};
use enth_patch::Patch;
use enth_session::Session;
use enth_session::game::{
    CoachHint, EditorView, GameEdit, GameRequest, Job, JobKind, JobResult, Listen, LogEvent, Phase,
    Prepared, RingAt, RoundLogEntry, RoundView,
};
use enth_ui::Theme;
use enth_ui::editor::overlay::{Emphasis, Overlay, Pointer};
use enth_ui::editor::{Editor, Region};
use enth_ui::theme::Score;

/// The hint the test's worker answers with (lane SN's coach runs in the plugin's worker).
const HINT: &str = "Check the brightness: your sound is darker than the target.";

/// The editor, its session, the jobs the session asked for, the results waiting to be handed back and the round
/// log's lines.
struct App {
    theme: Theme,
    editor: Editor,
    session: Session,
    now: u64,
    jobs: Vec<Job>,
    results: Vec<(enth_session::game::JobId, JobResult)>,
    seed: u64,
    /// Seeds the shell draws before falling back to `seed` (R1/S1 lane INT: a silent deal, then another).
    seeds: std::collections::VecDeque<u64>,
    /// Seeds whose target the worker answers as silent, standing in for `prepare_round`'s `silent_target` (the
    /// level presets are never silent; enth-challenge and the plugin's worker test the real refusal).
    silent: Vec<u64>,
    log: Vec<RoundLogEntry>,
    /// The shell plays rounds (calls `set_round` every frame), as the plugin does.
    plays: bool,
    /// The control the worker's hints point at (lane HINT): `None` answers with the text alone.
    hint_at: Option<enth_patch::ParamAddr>,
}

/// The worker's part, synchronously: the same functions the plugin's worker calls.
fn run(job: &Job, silent: &[u64], hint_at: Option<&enth_patch::ParamAddr>) -> JobResult {
    match &job.kind {
        JobKind::Prepare { spec } if silent.contains(&spec.seed) => JobResult::SilentTarget,
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
        JobKind::Rings {
            challenge, patch, ..
        } => {
            let round = enth_challenge::prepare_challenge((**challenge).clone()).unwrap();
            JobResult::Rings(Box::new(score_structure(&round, patch).unwrap()))
        }
        JobKind::Loudness { .. } => JobResult::Loudness { lufs: None },
        JobKind::Hint { level, .. } => {
            let mut hint = CoachHint::new(*level, HINT);
            if let Some(at) = hint_at {
                hint = hint
                    .with_aspect("brightness", Some(enth_session::game::Direction::Lower))
                    .with_param(at.clone(), Some(enth_session::game::HintTurn::Raise));
                if let Some(module) = at.module_id() {
                    hint = hint.with_module(module.clone());
                }
            }
            JobResult::Hint(Box::new(hint))
        }
        _ => JobResult::Failed {
            message: "not in this test".to_owned(),
        },
    }
}

fn app(patch: Patch, plays: bool) -> App {
    let theme = Theme::dark();
    App {
        editor: Editor::new(&theme),
        theme,
        session: Session::new(patch),
        now: 0,
        jobs: Vec::new(),
        results: Vec::new(),
        seed: 42,
        seeds: std::collections::VecDeque::new(),
        silent: Vec::new(),
        log: Vec::new(),
        plays,
        hint_at: None,
    }
}

fn harness() -> Harness<'static, App> {
    harness_with(app(common::patches::p1_default(), true), false)
}

/// The harness over `app`, rendering on lavapipe when `render` (snapshots). Tooltips show at once, so the no-peek
/// test can read every one.
fn harness_with(app: App, render: bool) -> Harness<'static, App> {
    let theme = app.theme.clone();
    let mut builder = Harness::builder()
        .with_size(theme.size.window.default)
        .with_pixels_per_point(1.0)
        .with_step_dt(1.0 / 60.0)
        .with_context_setup(move |ctx| {
            theme.apply(ctx);
            ctx.global_style_mut(|s| {
                s.interaction.tooltip_delay = 0.0;
                s.interaction.show_tooltips_only_when_still = false;
            });
        });
    if render {
        builder = builder.renderer(common::cpu_renderer());
    }
    builder.build_ui_state(
        |ui, app: &mut App| {
            let down = ui.input(|i| i.pointer.any_down());
            let rect = ui.ctx().content_rect();
            let mut root = ui.new_child(UiBuilder::new().max_rect(rect));
            // The glue's frame: the clock, results back, the round view, draw, the round intents.
            app.now += 16;
            let view = if app.now == 16 {
                EditorView::Opened
            } else {
                EditorView::Visible
            };
            app.session.game_tick(app.now, view);
            for (id, result) in app.editor.hold_scores(std::mem::take(&mut app.results)) {
                let _ = app.session.game_job_done(id, result);
            }
            if app.plays {
                app.editor.set_round(app.session.round());
            }
            app.editor
                .frame(&mut root, &app.theme, &mut app.session, down);
            let fallback = app.seed;
            let seeds = &mut app.seeds;
            app.editor.apply_game(&mut app.session, &mut || {
                seeds.pop_front().unwrap_or(fallback)
            });
            app.session.drain_requests().for_each(drop);
            for request in app.session.drain_game_requests() {
                match request {
                    GameRequest::Run(job) => app.jobs.push(job),
                    GameRequest::Log(line) => app.log.push(*line),
                    _ => {}
                }
            }
            // The worker, between frames.
            for job in std::mem::take(&mut app.jobs) {
                let result = run(&job, &app.silent, app.hint_at.as_ref());
                app.results.push((job.id, result));
            }
        },
        app,
    )
}

fn click(h: &mut Harness<'_, App>, label: &str) {
    h.get_by_label(label).click();
    h.run_steps(3);
}

fn round(h: &Harness<'_, App>) -> Option<RoundView> {
    h.state().session.round()
}

fn phase(h: &Harness<'_, App>) -> Option<Phase> {
    round(h).map(|r| r.phase)
}

/// The target `seed` and `level` give, as the worker prepares it.
fn target(level: u8, seed: u64) -> PreparedRound {
    prepare_round(
        &SpecPreset::by_id(level)
            .unwrap()
            .spec(LATEST_GENERATOR, seed),
    )
    .unwrap()
}

/// Recreate mode, then Start at `level`: the round plays once the worker prepared it.
fn start(h: &mut Harness<'_, App>, level: u8) {
    h.run_steps(2);
    click(h, "Recreate");
    click(h, &format!("Level {level}"));
    click(h, "Start");
    h.run_steps(2);
    assert_eq!(phase(h), Some(Phase::Playing), "prepared on the worker");
}

/// Submit, rate (or skip), and wait for the score.
fn submit(h: &mut Harness<'_, App>, rating: Option<u8>) {
    click(h, "Submit");
    match rating {
        Some(n) => click(h, &format!("Rate {n}")),
        None => click(h, "Skip"),
    }
    h.run_steps(3);
}

#[test]
fn a_round_from_recreate_to_end_through_the_bar_and_the_drawer() {
    let mut h = harness();
    h.run_steps(2);
    assert_eq!(phase(&h), None);
    let own = h.state().session.patch().clone();
    // Play: one "Recreate" button beside the Presets group.
    assert!(h.query_by_label("Previous preset").is_some());
    assert!(h.query_by_label("Start").is_none());

    start(&mut h, 2);
    let t = target(2, 42);
    assert_eq!(h.state().session.patch(), t.start(), "the start plays");
    assert!(
        h.query_by_label("Previous preset").is_none(),
        "the Presets group steps aside in Recreate"
    );
    assert_eq!(round(&h).unwrap().difficulty, Some(25));
    let _ = h.get_by_label("Level 2 of 5");

    click(&mut h, "Target ▶");
    assert_eq!(h.state().session.game_ab().listen, Listen::Target);
    click(&mut h, "Mine ▶");
    assert_eq!(h.state().session.game_ab().listen, Listen::Mine);
    click(&mut h, "A/B");
    assert_eq!(
        h.state().session.game_ab().listen,
        Listen::Target,
        "A/B flips"
    );
    click(&mut h, "Target ▶");
    assert_eq!(
        h.state().session.game_ab().listen,
        Listen::Off,
        "a second click turns it off"
    );

    // Submit asks the blind rating in the drawer; the score is computed but held back until it is given.
    click(&mut h, "Submit");
    h.run_steps(3);
    assert!(h.state().editor.recreate().asking());
    assert!(
        h.query_by_label("How close is yours to the target?")
            .is_some()
    );
    let r = round(&h).unwrap();
    assert_eq!(r.first, None, "the score waits for the rating");
    assert!(r.pending.scoring);
    assert!(r.rings.is_empty(), "no ring before the score");
    assert!(
        h.state()
            .editor
            .recreate()
            .overlay()
            .unwrap()
            .param(&"osc1.wave".parse().unwrap())
            .score
            .is_none()
    );
    click(&mut h, "Rate 3");
    h.run_steps(3);
    let r = round(&h).unwrap();
    assert_eq!(r.rating, Some(3), "the rating came before the score");
    let first = r.first.expect("then the score");

    // The drawer: sound against the pass mark (a glyph and words), structure, combined, the components' words.
    let report = r.report.clone().expect("the report is in memory");
    assert!(!report.passed, "level 2's start is audibly away");
    let sound = enth_ui::editor::recreate::percent(first.score.sound);
    let _ = h.get_by_label(&format!("Sound {sound}"));
    let _ = h.get_by_label("✗ Not yet • pass mark 90 %");
    let _ = h.get_by_label(&format!(
        "Structure {}",
        enth_ui::editor::recreate::percent(first.score.structure)
    ));
    let _ = h.get_by_label(&format!(
        "Combined {}",
        enth_ui::editor::recreate::percent(first.score.combined)
    ));
    assert_ne!(report.sound.components.len(), 0);
    for c in &report.sound.components {
        let mark = enth_ui::editor::recreate::results::badge(c.closeness).map_or("•", Score::glyph);
        let words = enth_ui::editor::recreate::results::words(c);
        let _ = h.get_by_label(&format!("{mark} {words}"));
    }

    // The rings: on the player's controls, after the first score, Good / Near / Bad from the report.
    assert_ne!(r.rings.len(), 0);
    let overlay = h.state().editor.recreate().overlay().unwrap().clone();
    for ring in &r.rings {
        let want = match ring.verdict {
            enth_challenge::Verdict::Good => Score::Good,
            enth_challenge::Verdict::Near => Score::Near,
            _ => Score::Bad,
        };
        match &ring.at {
            RingAt::Param(addr) => assert_eq!(overlay.param(addr).score, Some(want), "{addr}"),
            RingAt::Route(route) => assert_eq!(overlay.route(route), Some(want), "{route}"),
            RingAt::Enable(id) => assert_eq!(overlay.enable(id), Some(want), "{id}"),
        }
    }

    // A hint: its text in the drawer, the levels used beside Hint; the round is assisted.
    click(&mut h, "Hint");
    h.run_steps(2);
    let r = round(&h).unwrap();
    assert_eq!(r.hints, 1);
    assert!(r.assisted);
    let _ = h.get_by_label(&format!("Hint 1/4: {HINT}"));
    let _ = h.get_by_label("Hint level 1 of 4");

    // The vote, then a dispute with a note: logged at once.
    click(&mut h, "About right");
    assert_eq!(round(&h).unwrap().agreement, Some(Agreement::AboutRight));
    click(&mut h, "Disagree…");
    h.get_by_label("Dispute note").click();
    h.run_steps(1);
    h.get_by_label("Dispute note")
        .type_text("the attack felt right");
    h.run_steps(2);
    click(&mut h, "Send");
    assert!(
        h.query_by_label("Dispute note").is_none(),
        "the note closes"
    );
    let dispute = h
        .state()
        .log
        .iter()
        .find(|l| l.event == LogEvent::Dispute)
        .expect("a dispute line at once");
    assert_eq!(dispute.dispute.as_deref(), Some("the attack felt right"));

    // Reveal loads the target; End restores the player's own patch.
    click(&mut h, "Reveal");
    assert_eq!(phase(&h), Some(Phase::Revealed));
    assert_eq!(
        h.state().session.patch(),
        t.target(),
        "the target is loaded"
    );
    click(&mut h, "End");
    click(&mut h, "End and restore my patch");
    assert_eq!(phase(&h), None);
    let mut back = h.state().session.patch().clone();
    back.engine.clone_from(&own.engine);
    assert_eq!(back, own, "the player's own patch is back");
    let _ = h.get_by_label("Start"); // still in Recreate mode, ready for the next round
    let end = h
        .state()
        .log
        .iter()
        .find(|l| l.event == LogEvent::End)
        .expect("the end line");
    let result = end.result.as_ref().unwrap();
    assert_eq!(result.agreement, Some(Agreement::AboutRight));
    assert_eq!(result.rating, Some(3));
    assert_eq!(end.dispute.as_deref(), Some("the attack felt right"));

    // The round log as the shell writes it (JSONL, one line per entry) reads back with the vote and the dispute.
    let jsonl: String = h
        .state()
        .log
        .iter()
        .map(|l| serde_json::to_string(l).unwrap() + "\n")
        .collect();
    let lines: Vec<serde_json::Value> = jsonl
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(lines.len(), 2, "{jsonl}");
    assert_eq!(lines[0]["event"], "dispute");
    assert_eq!(lines[0]["dispute"], "the attack felt right");
    assert_eq!(lines[1]["event"], "end");
    assert_eq!(lines[1]["result"]["agreement"], "about_right");
    assert_eq!(lines[1]["result"]["rating"], 3);

    // × leaves Recreate: the Presets group is back.
    click(&mut h, "Leave Recreate");
    assert!(h.query_by_label("Previous preset").is_some());
}

#[test]
fn skipping_the_rating_shows_the_score_without_one() {
    let mut h = harness();
    start(&mut h, 1);
    submit(&mut h, None);
    let r = round(&h).unwrap();
    assert_eq!(r.rating, None);
    assert_eq!(r.submits, 1);
    assert!(!h.state().editor.recreate().asking());
    assert!(
        h.query_by_label("How close is yours to the target?")
            .is_none()
    );
}

#[test]
fn rings_follow_the_players_edits_after_the_first_submit() {
    let mut h = harness();
    start(&mut h, 2);
    submit(&mut h, Some(2));
    let r = round(&h).unwrap();
    let t = target(2, 42);
    // Rebuild the target by edits: the rings settle (SETTLE_MS) and come back all good, live, without a submit.
    let ops = enth_patch::ops_between(h.state().session.patch(), t.target()).unwrap();
    h.state_mut().session.edit(ops, None).unwrap();
    h.run_steps(40);
    let after = round(&h).unwrap();
    assert_eq!(after.submits, r.submits, "no submit");
    assert_ne!(after.rings.len(), 0);
    assert!(
        after
            .rings
            .iter()
            .filter(|r| r.randomized)
            .all(|r| r.verdict == enth_challenge::Verdict::Good),
        "{:?}",
        after.rings
    );
    let overlay = h.state().editor.recreate().overlay().unwrap().clone();
    for ring in after.rings.iter().filter(|r| r.randomized) {
        if let RingAt::Param(addr) = &ring.at {
            assert_eq!(overlay.param(addr).score, Some(Score::Good));
        }
    }
}

#[test]
fn level_5_rings_routes_and_enable_toggles() {
    // Level 5 randomizes routes and enable flags (lane RG); find a seed whose target draws both.
    let seed = (1..200u64)
        .find(|&s| {
            let t = target(5, s);
            let drawn = &t.generated().randomized;
            drawn
                .iter()
                .any(|d| matches!(d.drawn, enth_challenge::random::Drawn::Route(_)))
                && drawn
                    .iter()
                    .any(|d| matches!(d.drawn, enth_challenge::random::Drawn::Enable(_)))
        })
        .expect("a level-5 target with a route and an enable flag");
    let mut a = app(common::patches::p1_default(), true);
    a.seed = seed;
    let mut h = harness_with(a, false);
    start(&mut h, 5);
    submit(&mut h, None);
    // The start lacks the target's routes, so its route rows have no control to ring yet: rebuild the target and let
    // the rings settle (live, no submit).
    let ops = enth_patch::ops_between(h.state().session.patch(), target(5, seed).target()).unwrap();
    h.state_mut().session.edit(ops, None).unwrap();
    h.run_steps(40);
    let r = round(&h).unwrap();
    assert!(
        r.rings.iter().any(|g| matches!(g.at, RingAt::Route(_))),
        "{:?}",
        r.rings
    );
    assert!(
        r.rings.iter().any(|g| matches!(g.at, RingAt::Enable(_))),
        "{:?}",
        r.rings
    );
    let overlay = h.state().editor.recreate().overlay().unwrap().clone();
    for ring in &r.rings {
        match &ring.at {
            RingAt::Route(route) => assert!(overlay.route(route).is_some()),
            RingAt::Enable(id) => assert!(overlay.enable(id).is_some()),
            RingAt::Param(_) => {}
        }
    }
    // The route rings show in the overview's routes: the drawer steps aside for it, and comes back.
    click(&mut h, "Show modulation");
    assert!(h.query_by_label("Show results").is_some());
    assert!(h.query_by_label("Try again").is_none());
    click(&mut h, "Show results");
    let _ = h.get_by_label("Try again");
}

#[test]
fn try_again_replays_the_code_and_new_round_draws_a_new_sound() {
    let mut h = harness();
    start(&mut h, 1);
    submit(&mut h, None);
    let code = round(&h).unwrap().code;
    click(&mut h, "Try again");
    h.run_steps(2);
    let again = round(&h).unwrap();
    assert_eq!(again.code, code, "the same challenge");
    assert_eq!(again.submits, 0, "from its start");
    assert_eq!(
        h.state().log.last().unwrap().ended_by,
        Some(enth_session::game::EndedBy::NewRound)
    );
    submit(&mut h, None);
    h.state_mut().seed = 4242;
    click(&mut h, "New round");
    h.run_steps(2);
    let next = round(&h).unwrap();
    assert_ne!(next.code, code, "a new sound");
    assert_eq!(next.difficulty, Some(0), "the same level");
    assert_eq!(
        h.state().log.len(),
        2,
        "both rounds logged as `new_round` (GR1's \"want more\")"
    );
}

#[test]
fn a_pasted_code_starts_its_round_and_a_bad_code_says_why() {
    let mut h = harness();
    h.run_steps(2);
    click(&mut h, "Recreate");
    h.get_by_label("Challenge code").click();
    h.run_steps(1);
    h.get_by_label("Challenge code").type_text("NOT-A-CODE");
    h.run_steps(2);
    click(&mut h, "Start");
    assert_eq!(phase(&h), None);
    let readout = h.state().editor.recreate().readout().unwrap_or_default();
    assert!(readout.starts_with("That code does not work"), "{readout}");
    let code =
        ChallengeCode::short(SpecPreset::by_id(3).unwrap().spec(LATEST_GENERATOR, 777)).unwrap();
    h.get_by_label("Challenge code").click();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.run_steps(1);
    h.get_by_label("Challenge code")
        .type_text(&code.to_string());
    h.run_steps(2);
    click(&mut h, "Start");
    h.run_steps(2);
    let r = round(&h).unwrap();
    assert_eq!(r.code, code.to_string());
    assert_eq!(r.phase, Phase::Playing);
    assert_eq!(h.state().session.patch(), target(3, 777).start());
}

#[test]
fn the_settings_page_makes_a_custom_spec_and_the_dim_assist_dims() {
    let mut h = harness();
    h.run_steps(2);
    click(&mut h, "Recreate");
    click(&mut h, "Settings…");
    let _ = h.get_by_label("Recreate settings");
    assert!(h.query_by_label("Previous preset").is_none());
    click(&mut h, "Settings level 3");
    assert_eq!(h.state().editor.recreate().settings().preset(), Some(3));
    click(&mut h, "Filter follows the level");
    let s = *h.state().editor.recreate().settings();
    assert_eq!(
        s.preset(),
        None,
        "an amount set by hand makes custom settings"
    );
    assert_eq!(
        s.amounts[enth_challenge::Category::Filter.index()],
        Some(100)
    );
    click(&mut h, "Dim unused modules");
    assert!(h.state().editor.recreate().dim_unused());
    click(&mut h, "Start round");
    h.run_steps(3);
    assert!(
        h.query_by_label("Recreate settings").is_none(),
        "the page closes"
    );
    let r = round(&h).unwrap();
    assert_eq!(r.phase, Phase::Playing);
    let code = ChallengeCode::decode(&r.code).unwrap();
    assert_eq!(
        code.form(),
        enth_challenge::CodeForm::Long,
        "custom settings make a long code"
    );
    assert_eq!(
        enth_ui::editor::recreate::settings::Settings::of(code.spec()),
        s
    );
    // The assist: modules the target leaves alone dim; the randomized ones stay.
    let overlay = h.state().editor.recreate().overlay().unwrap().clone();
    let modules: std::collections::BTreeSet<_> = r
        .params
        .iter()
        .filter_map(|a| match a.target() {
            enth_patch::Target::Module(id) => Some(id.clone()),
            _ => None,
        })
        .collect();
    for m in h
        .state()
        .session
        .patch()
        .modulators
        .iter()
        .chain(&h.state().session.patch().generators)
    {
        let want = if modules.contains(&m.id) {
            Emphasis::Normal
        } else {
            Emphasis::Dimmed
        };
        assert_eq!(overlay.module(&m.id), want, "{}", m.id);
    }
}

/// Lane HINT (FU1): the dim assist counts as an assist. A round played with it is assisted as soon as the dimming
/// shows (no hint, no reveal), and the round log's result says so; without it, the same start is not assisted.
#[test]
fn the_dim_assist_marks_the_round_assisted_and_the_log_says_so() {
    for dim in [false, true] {
        let mut h = harness();
        h.run_steps(2);
        click(&mut h, "Recreate");
        click(&mut h, "Settings…");
        if dim {
            click(&mut h, "Dim unused modules");
        }
        click(&mut h, "Start round");
        h.run_steps(3);
        let r = round(&h).unwrap();
        assert_eq!(r.phase, Phase::Playing);
        assert!(!r.params.is_empty(), "the target's parameters are known");
        assert_eq!((r.dim_unused, r.assisted), (dim, dim), "dim {dim}");
        assert_eq!((r.hints, r.revealed), (0, false));
        click(&mut h, "End");
        click(&mut h, "End and restore my patch");
        assert_eq!(phase(&h), None);
        let end = h
            .state()
            .log
            .iter()
            .find(|l| l.event == LogEvent::End)
            .expect("the end line");
        let assists = end.result.as_ref().unwrap().assists;
        assert_eq!((assists.dim_unused, assists.any()), (dim, dim), "dim {dim}");
        let line = serde_json::to_string(end).unwrap();
        assert_eq!(line.contains(r#""dim_unused":true"#), dim, "{line}");
    }
}

/// The control a test's hints point at: the round's filter cutoff if it was randomized (shown on its card without
/// "More"), else the first randomized module parameter.
fn hint_control(r: &RoundView) -> enth_patch::ParamAddr {
    r.params
        .iter()
        .find(|a| a.to_string() == "filter1.cutoff")
        .or_else(|| r.params.iter().find(|a| a.module_id().is_some()))
        .cloned()
        .expect("a randomized module parameter")
}

/// Lane HINT (FU1): a revealed hint marks what its level names on the player's controls: level 1 nothing (an aspect),
/// level 2 the module's card, level 3 the control with ↑ (raise). The worker fills every field at every level; the
/// session keeps each hint on the ladder, so nothing is marked early. The next hint replaces the mark, a reveal clears
/// it, and so does the end of the round.
#[test]
fn a_hint_highlights_its_control() {
    let mut h = harness();
    start(&mut h, 3);
    let at = hint_control(&round(&h).unwrap());
    let module = at.module_id().cloned().unwrap();
    h.state_mut().hint_at = Some(at.clone());
    let marks = |h: &Harness<'_, App>| {
        let o = h.state().editor.recreate().overlay().cloned().unwrap();
        (o.module_hint(&module), o.param(&at).hint)
    };
    assert_eq!(marks(&h), (false, None), "no hint, no mark");
    click(&mut h, "Hint");
    h.run_steps(2);
    assert_eq!(round(&h).unwrap().hints, 1);
    assert_eq!(
        marks(&h),
        (false, None),
        "level 1 names an aspect, not a control"
    );
    click(&mut h, "Hint");
    h.run_steps(2);
    assert_eq!(marks(&h), (true, None), "level 2: the module's card");
    click(&mut h, "Hint");
    h.run_steps(2);
    assert_eq!(
        marks(&h),
        (false, Some(Pointer::Raise)),
        "level 3: the control, and which way"
    );
    let _ = h.get_by_label(&format!("Hint 3/4: {HINT}"));
    click(&mut h, "Reveal");
    assert_eq!(phase(&h), Some(Phase::Revealed));
    assert_eq!(marks(&h), (false, None), "a reveal clears the mark");
    click(&mut h, "End");
    click(&mut h, "End and restore my patch");
    assert_eq!(phase(&h), None);
    assert!(
        h.state().editor.recreate().overlay().is_none(),
        "no round, no mark"
    );
}

#[test]
fn nothing_scrolls_at_1200_by_800_in_recreate() {
    let mut h = harness();
    h.run_steps(2);
    click(&mut h, "Recreate");
    let header = h.state().editor.report().rect(Region::Header).unwrap();
    let add = h.get_by_label("+ Add").rect();
    // Before a round, every control sits in the header row, left of "+ Add".
    for label in [
        "Level 1",
        "Level 5",
        "Challenge code",
        "Start",
        "Settings…",
        "Leave Recreate",
    ] {
        let r = h.get_by_label(label).rect();
        assert!(
            header.contains_rect(r) && r.right() < add.left(),
            "{label} at {r:?}"
        );
    }
    click(&mut h, "Start");
    h.run_steps(2);
    let overflow = &h.state().editor.report().overflow;
    assert!(overflow.is_empty(), "{overflow:?}");
    for label in [
        "Level 1 of 5",
        "Target ▶",
        "Mine ▶",
        "A/B",
        "Time 0:00",
        "Submit",
        "Hint",
        "Reveal",
        "End",
    ] {
        let r = h.query_by_label_contains(label).unwrap().rect();
        assert!(
            header.contains_rect(r) && r.right() < add.left(),
            "{label} at {r:?}"
        );
    }
    submit(&mut h, Some(4));
    click(&mut h, "Hint");
    h.run_steps(2);
    let overflow = &h.state().editor.report().overflow;
    assert!(
        overflow.is_empty(),
        "the drawer fits the overview slot: {overflow:?}"
    );
    let drawer = h.state().editor.report().rect(Region::Overview).unwrap();
    for label in [
        "Too low",
        "About right",
        "Too high",
        "Disagree…",
        "Try again",
        "Reveal the target",
        "New round",
        "Show modulation",
    ] {
        let r = h.get_all_by_label(label).last().unwrap().rect();
        assert!(
            drawer.contains_rect(r),
            "{label} at {r:?}, drawer {drawer:?}"
        );
    }
}

#[test]
fn a_shell_that_plays_no_rounds_shows_no_bar() {
    let mut h = harness_with(app(common::patches::p1_default(), false), false);
    h.run_steps(2);
    assert!(h.query_by_label("Recreate").is_none());
    assert!(h.query_by_label("Previous preset").is_some());
}

// ---------------------------------------------------------------------------------------------------------------
// No peeking
// ---------------------------------------------------------------------------------------------------------------

/// Every text the last frame showed: painted galleys (tooltips, menus and popups included) and the accessibility
/// tree's labels, values and descriptions.
fn texts(h: &Harness<'_, App>) -> Vec<String> {
    fn walk(shape: &egui::Shape, out: &mut Vec<String>) {
        match shape {
            egui::Shape::Text(t) => out.push(t.galley.text().to_owned()),
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    for clipped in &h.output().shapes {
        walk(&clipped.shape, &mut out);
    }
    for node in h.root().query_all_by(|_| true) {
        let a = node.accesskit_node();
        out.extend(
            [a.label(), a.value(), a.description()]
                .into_iter()
                .flatten(),
        );
    }
    out
}

/// A reading written the ways a display might: whole, one and two decimals, with either minus sign.
fn reading_forms(v: f64) -> Vec<String> {
    let mut forms = Vec::new();
    for text in [
        format!("{v:.0}"),
        format!("{v:.1}"),
        format!("{v:.2}"),
        format!("{:.2}", v / 1000.0),
    ] {
        // Significant digits only (no leading zeros): a reading as short as "25" or "0.02" would match unrelated text.
        let significant = text
            .trim_start_matches(['-', '0', '.'])
            .chars()
            .filter(char::is_ascii_digit)
            .count();
        if significant >= 3 {
            forms.push(text.replace('-', "\u{2212}"));
            forms.push(text);
        }
    }
    forms
}

/// The forbidden strings found in `texts`: a target value as the editor writes it, or a reading as a whole token.
fn leaks(texts: &[String], values: &[String], readings: &[String]) -> Vec<String> {
    let mut found = Vec::new();
    for t in texts {
        for v in values {
            if t.contains(v.as_str()) {
                found.push(format!("{v:?} in {t:?}"));
            }
        }
        for token in
            t.split(|c: char| c.is_whitespace() || c == '(' || c == ')' || c == ',' || c == ':')
        {
            if readings.iter().any(|r| r == token) {
                found.push(format!("reading {token:?} in {t:?}"));
            }
        }
    }
    found
}

#[test]
fn no_target_value_or_reading_is_visible_or_reachable_until_reveal() {
    let level = 3;
    let t = target(level, 42);
    // What the editor shows of the start patch outside any round: those strings are the player's own, not leaks.
    let mut base = harness_with(app(t.start().clone(), false), false);
    base.run_steps(3);
    let baseline = texts(&base).join("\n");
    // The target's randomized values as the editor writes them, minus what the start patch already shows.
    let values: Vec<String> = t
        .params()
        .iter()
        .filter_map(|addr| {
            let spec = t.target().spec(addr)?;
            spec.enum_items().is_none().then(|| {
                enth_ui::editor::cards::format::value_text(spec, t.target().value(addr).unwrap())
            })
        })
        .filter(|v| !baseline.contains(v.as_str()))
        .collect();
    assert!(
        values.len() >= 3,
        "a level-{level} target with values to hide: {values:?}"
    );

    let mut h = harness();
    start(&mut h, level);
    submit(&mut h, Some(3));
    click(&mut h, "Hint");
    h.run_steps(2);
    let report = round(&h).unwrap().report.clone().unwrap();
    let readings: Vec<String> = report
        .sound
        .components
        .iter()
        .flat_map(|c| [c.target, c.answer])
        .flatten()
        .flat_map(reading_forms)
        .filter(|f| !baseline.split_whitespace().any(|w| w == f))
        .collect();
    assert!(!readings.is_empty(), "the report has readings to hide");
    assert_eq!(
        leaks(&[format!("x {} y", readings[0])], &values, &readings).len(),
        1,
        "the check sees a reading"
    );

    let mut seen: Vec<String> = Vec::new();
    let look = |h: &Harness<'_, App>, seen: &mut Vec<String>| seen.extend(texts(h));
    look(&h, &mut seen);
    // Every interactive control's tooltip and the status line while it is hovered: the drawer's, then the overview's.
    for pass in 0..2 {
        let rects: Vec<egui::Rect> = h
            .root()
            .query_all_by(|n| {
                matches!(
                    n.role(),
                    egui::accesskit::Role::Button
                        | egui::accesskit::Role::Slider
                        | egui::accesskit::Role::CheckBox
                        | egui::accesskit::Role::Switch
                        | egui::accesskit::Role::RadioButton
                        | egui::accesskit::Role::Label
                        | egui::accesskit::Role::TextInput
                )
            })
            .map(|n| n.rect())
            .collect();
        assert!(rects.len() > 50, "{} controls", rects.len());
        for r in rects {
            h.hover_at(r.center());
            h.run_steps(2);
            look(&h, &mut seen);
        }
        if pass == 0 {
            click(&mut h, "Show modulation");
        }
    }
    // The menus: End, and every randomized control's right-click menu.
    click(&mut h, "End");
    look(&h, &mut seen);
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    for addr in t.params() {
        let label = enth_ui::editor::cards::present::label(
            enth_ui::editor::cards::CardKind::Module(match addr.target() {
                enth_patch::Target::Module(id) => {
                    h.state().session.patch().find(id).unwrap().1.kind
                }
                _ => continue,
            }),
            t.target().spec(addr).unwrap(),
        );
        let at = h
            .query_all_by_label(label)
            .next()
            .map(|n| n.rect().center());
        if let Some(at) = at {
            h.hover_at(at);
            h.run_steps(1);
            node_right_click(&mut h, at);
            look(&h, &mut seen);
            h.key_press(egui::Key::Escape);
            h.run_steps(2);
        }
    }
    let found = leaks(&seen, &values, &readings);
    assert!(
        found.is_empty(),
        "the round shows target values or readings: {found:#?}"
    );

    // After Reveal the target is loaded and its values show: the check would have seen them.
    click(&mut h, "Show results");
    click(&mut h, "Reveal");
    h.run_steps(2);
    let after = texts(&h);
    assert!(
        !leaks(&after, &values, &[]).is_empty(),
        "after Reveal the target's values are on the knobs"
    );
}

/// A secondary click at `at` (a control's right-click menu).
fn node_right_click(h: &mut Harness<'_, App>, at: egui::Pos2) {
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Secondary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        });
        h.run_steps(1);
    }
    h.run_steps(2);
}

// ---------------------------------------------------------------------------------------------------------------
// Idle frames
// ---------------------------------------------------------------------------------------------------------------

/// The Recreate UI asks for no frames of its own: with a scored round live (the drawer open, rings drawn), an idle
/// editor's frames ask for nothing (`Duration::MAX`). The round's own deadlines (the timer's next second, settled
/// jobs) are the shell's `RoundView::wake_at_ms`, which the editor never schedules.
#[test]
fn an_idle_editor_with_a_scored_round_asks_for_no_frames() {
    let mut h = harness();
    start(&mut h, 1);
    submit(&mut h, Some(2));
    h.run_steps(40);
    let theme = Theme::dark();
    let ctx = egui::Context::default();
    theme.apply(&ctx);
    let App {
        editor, session, ..
    } = h.state_mut();
    let mut time = 0.0;
    let mut delays = Vec::new();
    for _ in 0..8 {
        time += 1.0 / 30.0;
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                theme.size.window.default,
            )),
            time: Some(time),
            ..Default::default()
        };
        editor.set_round(session.round());
        let mut output = ctx.run_ui(input, |ui| editor.frame(ui, &theme, session, false));
        output.textures_delta.clear();
        delays.push(output.viewport_output[&egui::ViewportId::ROOT].repaint_delay);
    }
    assert!(editor.recreate().drawer_open());
    assert!(
        delays[3..].iter().all(|d| *d == std::time::Duration::MAX),
        "idle frames ask for nothing: {delays:?}"
    );
}

// ---------------------------------------------------------------------------------------------------------------
// Snapshots and REAPER's click points
// ---------------------------------------------------------------------------------------------------------------

/// Snapshots at the default window (1200 × 800), each checked by label first. Update on purpose and look at them:
/// `UPDATE_SNAPSHOTS=1 cargo test -p enth-ui --test recreate`.
#[test]
fn recreate_snapshots() {
    let mut h = harness_with(app(common::patches::p1_default(), true), true);
    h.run_steps(2);
    click(&mut h, "Recreate");
    h.run_steps(2);
    h.snapshot("recreate_before");
    click(&mut h, "Settings…");
    h.run_steps(2);
    h.snapshot("recreate_settings");
    click(&mut h, "Close settings");
    click(&mut h, "Start");
    h.run_steps(3);
    click(&mut h, "Target ▶");
    h.run_steps(3);
    h.snapshot("recreate_round_live");
    click(&mut h, "Submit");
    h.run_steps(3);
    let _ = h.get_by_label("Rate 1");
    h.snapshot("recreate_rating");
    click(&mut h, "Rate 2");
    click(&mut h, "Hint");
    h.run_steps(3);
    let _ = h.get_by_label("Try again");
    h.remove_cursor();
    h.run_steps(2);
    h.snapshot("recreate_results");
    // Lane HINT (FU1): a level-3 hint marks its control (an outline in the value accent, ↑ raise).
    let at = hint_control(&round(&h).unwrap());
    h.state_mut().hint_at = Some(at);
    click(&mut h, "Hint");
    h.run_steps(3);
    click(&mut h, "Hint");
    h.run_steps(3);
    let _ = h.get_by_label(&format!("Hint 3/4: {HINT}"));
    h.remove_cursor();
    h.run_steps(2);
    h.snapshot("recreate_hint");
}

/// The click points `xtask/reaper/round.lua` uses (offsets into the plugin window, layout "lanes" at 1200 × 800): each
/// lands on its control, so a layout change fails here before it fails in REAPER.
#[test]
fn reaper_points_hit_their_controls() {
    const RECREATE: (f32, f32) = (628.0, 28.0);
    const START: (f32, f32) = (353.0, 28.0);
    const TARGET: (f32, f32) = (174.0, 28.0);
    const MINE: (f32, f32) = (240.0, 28.0);
    const SUBMIT: (f32, f32) = (402.0, 28.0);
    const SKIP: (f32, f32) = (380.0, 708.0);
    const HINT_AT: (f32, f32) = (454.0, 28.0);
    const END: (f32, f32) = (554.0, 28.0);
    // After a hint the levels used ("1/4") stand beside Hint, so Reveal and End move right (lane HINT).
    const END_HINTED: (f32, f32) = (574.0, 28.0);
    const RESTORE: (f32, f32) = (620.0, 61.0);
    let hits = |h: &Harness<'_, App>, label: &str, (x, y): (f32, f32)| {
        let rect = h.get_by_label(label).rect();
        assert!(
            rect.contains(egui::pos2(x, y)),
            "{label} is at {rect:?}, not under ({x}, {y}): update round.lua's POINTS and these"
        );
    };
    let mut h = harness();
    h.run_steps(2);
    hits(&h, "Recreate", RECREATE);
    click(&mut h, "Recreate");
    hits(&h, "Start", START);
    click(&mut h, "Start");
    h.run_steps(2);
    hits(&h, "Target ▶", TARGET);
    hits(&h, "Mine ▶", MINE);
    hits(&h, "Submit", SUBMIT);
    hits(&h, "End", END);
    click(&mut h, "Submit");
    h.run_steps(2);
    hits(&h, "Skip", SKIP);
    click(&mut h, "Skip");
    h.run_steps(2);
    hits(&h, "Hint", HINT_AT);
    click(&mut h, "Hint");
    h.run_steps(2);
    hits(&h, "Hint", HINT_AT);
    hits(&h, "End", END_HINTED);
    click(&mut h, "End");
    hits(&h, "End and restore my patch", RESTORE);
}

/// R1/S1 lane INT (lane GC's finding): a silent target is refused, and the editor deals the same level again with a
/// fresh seed; the silent round was never played, so the round log stays empty.
#[test]
fn a_silent_target_is_dealt_again_with_a_fresh_seed() {
    let mut a = app(common::patches::p1_default(), true);
    a.seeds = [7, 8].into();
    a.silent = vec![7];
    let mut h = harness_with(a, false);
    h.run_steps(2);
    click(&mut h, "Recreate");
    click(&mut h, "Level 2");
    click(&mut h, "Start");
    h.run_steps(4);
    assert_eq!(phase(&h), Some(Phase::Playing), "dealt again and prepared");
    let dealt = SpecPreset::by_id(2).unwrap().spec(LATEST_GENERATOR, 8);
    assert_eq!(
        round(&h).unwrap().code,
        ChallengeCode::short(dealt).unwrap().to_string(),
        "the second seed's round"
    );
    assert!(h.state().log.is_empty(), "{:?}", h.state().log);
    assert!(h.state().seeds.is_empty());
}
