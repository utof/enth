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

#![allow(
    clippy::print_stdout,
    reason = "a measurement tool: it prints the frame count it ran"
)]

#[path = "../tests/common/patches.rs"]
mod patches;

use egui::{Event, Pos2, RawInput, Rect, pos2, vec2};
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
    let theme = Theme::dark();
    let ctx = egui::Context::default();
    theme.apply(&ctx);
    let mut session = Session::new(patch);
    let mut editor = Editor::new(&theme);
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
    // An empty strip of the header, left of the patch name.
    let idle_at = pos2(
        screen.width() * 0.25,
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
