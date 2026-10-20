//! The editor's cost per frame, headless (lane ED1): egui's pass plus tessellation, no GL, the method of research 02
//! (F6) and of the UI spike, on the real editor (layout 1) over a detached session.
//!
//! ```text
//! cargo build --release -p enth-ui --example frame_bench
//! valgrind --tool=callgrind target/release/examples/frame_bench idle 50
//! valgrind --tool=callgrind target/release/examples/frame_bench idle 250
//! ```
//!
//! Instructions per frame = (I(250) − I(50)) / 200, which drops the start-up and the font atlas. Cases: `idle` (the
//! pointer moves over an empty strip of the header, so every frame is a full pass where nothing changes) and `drag`
//! (a knob drag, so every frame edits the patch through the session and redraws the changed knob). Patches: `p1`
//! (the P1 default, the G1 reference patch) or `max` (the maximum patch).
//!
//! Live cases (lane ED2): `live` runs an active session with a held note, renders 1,600 frames of audio (25 blocks of
//! 64, one 30 Hz frame's worth at 48 kHz) before every editor frame, and draws the telemetry (rings, tile levels,
//! the scope, the meters); `spectrum` is the same with the output region showing the spectrum; `engine` renders the
//! same audio with no editor, so the editor's share is `live − engine` per frame.
//!
//! Recreate (lane RUI): a fourth argument `recreate` plays every case inside a level-5 Recreate round whose patch is
//! the case's patch, scored and with a hint: the challenge bar in the header, the results drawer in the overview
//! slot, the rings on the knobs, and the round view handed over every frame as the plugin does. The worker's jobs
//! run before the measured frames, so `(I(250) − I(50)) / 200` drops them.
//!
//! ```text
//! valgrind --tool=callgrind target/release/examples/frame_bench spectrum 250 max recreate
//! ```

#![allow(
    clippy::print_stdout,
    reason = "a measurement tool: it prints the frame count it ran"
)]

#[path = "../tests/common/patches.rs"]
mod patches;

use egui::{Event, Pos2, RawInput, Rect, pos2, vec2};
use enth_patch::Patch;
use enth_session::game::{
    CoachHint, EditorView, GameEdit, GameRequest, JobKind, JobResult, Prepared, RoundSource,
};
use enth_session::{Activation, Session};
use enth_ui::Theme;
use enth_ui::editor::Editor;
use enth_ui::editor::cards::ids;

fn main() {
    let mut args = std::env::args().skip(1);
    let case = args.next().unwrap_or_else(|| "idle".to_owned());
    let frames: u32 = args.next().and_then(|n| n.parse().ok()).unwrap_or(100);
    let patch = match args.next().as_deref() {
        Some("max") => patches::maximum(),
        _ => patches::p1_default(),
    };
    let recreate = args.next().as_deref() == Some("recreate");
    let theme = Theme::dark();
    let ctx = egui::Context::default();
    theme.apply(&ctx);
    let mut session = Session::new(patch.clone());
    let mut editor = Editor::new(&theme);
    if recreate {
        round(&mut session, &patch);
    }
    let live = matches!(case.as_str(), "live" | "spectrum" | "engine");
    let mut render: Box<dyn FnMut()> = Box::new(|| {});
    if live {
        let mut engine = session
            .activate(Activation {
                sample_rate: 48_000.0,
                offline: false,
            })
            .expect("activates");
        editor.open(&session, &theme);
        let _ = enth_session::SessionEdit::audition_on(&mut session, 48, 1.0);
        session.pump();
        render = Box::new(move || {
            let (mut l, mut r) = ([0.0f32; 64], [0.0f32; 64]);
            for _ in 0..25 {
                engine.process(&mut l, &mut r, &[]);
            }
            std::hint::black_box(&l);
        });
    }
    if case == "engine" {
        for _ in 0..frames {
            render();
        }
        println!("frame_bench: {case}, {frames} frames");
        return;
    }
    let screen = Rect::from_min_size(Pos2::ZERO, theme.size.window.default);
    let mut run = |events: Vec<Event>, frame: u32| {
        render();
        session.pump();
        if recreate {
            session.game_tick(10_000 + u64::from(frame) * 33, EditorView::Visible);
            editor.set_round(session.round());
        }
        let input = RawInput {
            screen_rect: Some(screen),
            time: Some(f64::from(frame) / f64::from(theme.motion.animated_hz)),
            events,
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            let down = ui.input(|i| i.pointer.any_down());
            editor.frame(ui, &theme, &mut session, down);
        });
        if recreate {
            editor.apply_game(&mut session, &mut || 1);
            session.drain_game_requests().for_each(drop);
        }
        let primitives =
            ctx.tessellate(std::mem::take(&mut output.shapes), output.pixels_per_point);
        std::hint::black_box(primitives);
        output.textures_delta.clear();
        session.drain_requests().for_each(drop);
    };
    // Two frames to lay everything out, then find the cutoff knob for the drag case.
    run(Vec::new(), 0);
    run(Vec::new(), 1);
    let cutoff = "filter1.cutoff".parse().expect("an address");
    let knob = ctx
        .read_response(ids::param(&cutoff))
        .map_or(screen.center(), |r| r.rect.center());
    // An empty strip of the header, left of the patch name (in a round: right of the challenge bar's End).
    let idle_at = pos2(
        screen.width() * if recreate { 0.53 } else { 0.25 },
        theme.space.gap + theme.size.bar.header * 0.5,
    );
    match case.as_str() {
        "live" | "spectrum" => {
            if case == "spectrum" {
                // A click on the output display switches it to the spectrum.
                let display = ctx
                    .read_response(ids::region("output").with("display"))
                    .map_or(screen.center(), |r| r.rect.center());
                let click = |pressed| Event::PointerButton {
                    pos: display,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                };
                run(vec![Event::PointerMoved(display), click(true)], 2);
                run(vec![click(false), Event::PointerGone], 3);
            }
            for f in 0..frames {
                run(Vec::new(), f + 4);
            }
        }
        "drag" => {
            run(
                vec![
                    Event::PointerMoved(knob),
                    Event::PointerButton {
                        pos: knob,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
                2,
            );
            for f in 0..frames {
                // Up and down by a pixel: every frame changes the value, the drag never runs out of range.
                let dy = if f % 2 == 0 { 1.0 } else { -1.0 };
                run(vec![Event::PointerMoved(knob + vec2(0.0, dy))], f + 3);
            }
        }
        _ => {
            for f in 0..frames {
                let dx = if f % 2 == 0 { 1.0 } else { -1.0 };
                run(vec![Event::PointerMoved(idle_at + vec2(dx, 0.0))], f + 2);
            }
        }
    }
    println!("frame_bench: {case}, {frames} frames");
}

/// A level-5 round on `session` whose patch becomes `patch`, scored once, with its rings settled and a hint: the jobs
/// run here, synchronously, with the worker's functions.
fn round(session: &mut Session, patch: &Patch) {
    let spec = enth_challenge::SpecPreset::by_id(5)
        .expect("level 5")
        .spec(enth_challenge::spec::LATEST_GENERATOR, 7);
    let mut now = 0;
    // The round prepared once (as the plugin's worker keeps it), for the score, rings and hint jobs.
    let mut kept: Option<enth_challenge::PreparedRound> = None;
    let mut drive = |session: &mut Session, now: &mut u64| {
        for _ in 0..64 {
            *now += 100;
            session.game_tick(*now, EditorView::Visible);
            let jobs: Vec<_> = session
                .drain_game_requests()
                .filter_map(|r| match r {
                    GameRequest::Run(job) => Some(job),
                    _ => None,
                })
                .collect();
            for job in jobs {
                let result = match &job.kind {
                    JobKind::Prepare { spec } => {
                        let round = enth_challenge::prepare_round(spec).expect("prepares");
                        let prepared = Prepared::from_round(&round, None);
                        kept = Some(round);
                        JobResult::Prepared(Box::new(prepared))
                    }
                    JobKind::Score { patch, .. } => {
                        let round = kept.as_ref().expect("prepared first");
                        JobResult::Scored(Box::new(
                            enth_challenge::score_round(round, patch).expect("scores"),
                        ))
                    }
                    JobKind::Rings { patch, .. } => {
                        let round = kept.as_ref().expect("prepared first");
                        JobResult::Rings(Box::new(
                            enth_challenge::score_structure(round, patch).expect("rings"),
                        ))
                    }
                    JobKind::Hint { level, .. } => JobResult::Hint(Box::new(CoachHint::new(
                        *level,
                        "Your sound is darker than the target: look at the filter (filter1).",
                    ))),
                    _ => JobResult::Loudness { lufs: None },
                };
                let _ = session.game_job_done(job.id, result);
            }
        }
    };
    session.game_tick(now, EditorView::Opened);
    session
        .game_start(RoundSource::Settings(spec))
        .expect("starts");
    drive(session, &mut now);
    let ops = enth_patch::ops_between(session.patch(), patch).expect("a diff");
    session.edit(ops, None).expect("edits");
    session.game_submit().expect("submits");
    drive(session, &mut now);
    session.game_hint().expect("a hint");
    drive(session, &mut now);
    session.drain_requests().for_each(drop);
    let round = session.round().expect("a round");
    assert!(
        round.submits == 1 && round.hint.is_some() && !round.rings.is_empty(),
        "{round:?}"
    );
}
