//! Tap probe: fork patch 12 in the vendored egui-baseview, driven with XTest under Xvfb.
//!
//! **Xvfb only.** It plays the host (a toplevel that holds the X input focus and wants keys, as REAPER's window does)
//! with the egui editor embedded in it. The editor has a knob-like button and a tap pad: a widget that takes egui's
//! focus when pressed and, while it has it, claims the keyboard (`egui_baseview::claim_keyboard`). Each frame records
//! every key and button press egui saw, with the arrival stamp patch 12 gives it (`egui_baseview::arrival`) and the
//! frame's own time (`InputState::time`, the instant the frame took its input).
//!
//!   xvfb-run -a -s "-screen 0 1920x1080x24" tap-probe [--reparent] [--no-claim]
//!
//! - **Claim** (patch 4 unchanged): hover and a knob click leave Space to the host; a press on the pad takes the
//!   keyboard, and Escape, a click elsewhere, another window taking the focus, or the pad going away give it back,
//!   without the editor taking it again on the next frames.
//! - **Arrival stamps**: mouse and key taps at uneven gaps, bursts within one frame, and taps during slow (40 ms)
//!   frames. Every event of every frame has a stamp; each stamp follows the instant XTest sent it and precedes the
//!   frame that saw it; stamps never go backwards; two taps in one frame keep their own instants; stamp intervals
//!   follow the sent intervals more closely than frame times do. The lags and interval errors are printed: they are
//!   the Xvfb half of lane TAP's jitter table.
//!
//! `--no-claim` is the negative control for the claim: the pad takes egui's focus but does not claim, so Space must
//! keep reaching the host (the run passes when it does). It skips the stamp phase, which needs the keyboard.
//!
//! Exit status: 0 when every check passed, 1 when one failed, 2 on a setup error.

use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use baseview::dpi::{LogicalSize, Size};
use egui_baseview::{EguiWindow, EguiWindowSettings, Frame, KeyboardFocus};
use input_probe::{ESCAPE, Host, Parent, SPACE, centre, percentile, require_xvfb, summary_ms};

// ---------------------------------------------------------------- the editor

#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind {
    Space,
    OtherKey,
    Button,
}

#[derive(Clone, Debug)]
struct Seen {
    kind: Kind,
    /// The arrival stamp patch 12 gave the event (`None`: no stamps for this frame).
    stamp: Option<Instant>,
    /// The frame's time: the instant `InputState::time` stands for.
    frame: Instant,
    frame_no: u64,
}

struct State {
    frames: u64,
    /// Every key press (not a repeat) and primary button press egui saw, in order.
    presses: Vec<Seen>,
    /// Every stamp of every frame (every event, pointer moves included), in order.
    all_stamps: Vec<Instant>,
    frames_with_events: u64,
    /// Frames that had events but no stamps aligned with them.
    unstamped_frames: u64,
    /// Frames whose time (origin + `InputState::time`) came out later than the start of their `ui()`.
    frame_after_ui: u64,
    no_origin: bool,
    pad: Option<egui::Rect>,
    knob: Option<egui::Rect>,
    pad_focused: bool,
    /// Set by the driver.
    show_pad: bool,
    claim: bool,
    slow_ms: u64,
}

struct Probe {
    shared: Arc<Mutex<State>>,
}

impl egui_baseview::App for Probe {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut Frame) {
        let ui_start = Instant::now();
        let ctx = ui.ctx().clone();
        let origin = egui_baseview::arrival::origin(&ctx);
        let time = ui.input(|i| i.time);
        let frame_at = origin.map_or(ui_start, |o| o + Duration::from_secs_f64(time));
        let stamps = egui_baseview::arrival::stamps(&ctx);
        let events = ui.input(|i| i.raw.events.clone());
        let (show_pad, claim, slow_ms) = {
            let s = self.shared.lock().unwrap();
            (s.show_pad, s.claim, s.slow_ms)
        };

        let knob = ui.button("knob");
        let pad = show_pad.then(|| {
            let (rect, response) =
                ui.allocate_exact_size(egui::vec2(240.0, 100.0), egui::Sense::click());
            if response.is_pointer_button_down_on() || response.clicked() {
                response.request_focus();
            }
            if claim && response.has_focus() {
                egui_baseview::claim_keyboard(&ctx, response.id);
            }
            let fill = if response.has_focus() {
                egui::Color32::DARK_GREEN
            } else {
                egui::Color32::DARK_GRAY
            };
            ui.painter().rect_filled(rect, 4.0, fill);
            response
        });

        let mut s = self.shared.lock().unwrap();
        s.frames += 1;
        let frame_no = s.frames;
        s.no_origin |= origin.is_none();
        if frame_at > ui_start {
            s.frame_after_ui += 1;
        }
        if !events.is_empty() {
            s.frames_with_events += 1;
            if stamps.is_none() {
                s.unstamped_frames += 1;
            }
        }
        if let Some(stamps) = &stamps {
            s.all_stamps.extend(stamps.iter().copied());
        }
        for (index, event) in events.iter().enumerate() {
            let kind = match event {
                egui::Event::Key { key: egui::Key::Space, pressed: true, repeat: false, .. } => {
                    Some(Kind::Space)
                }
                egui::Event::Key { pressed: true, repeat: false, .. } => Some(Kind::OtherKey),
                egui::Event::PointerButton {
                    button: egui::PointerButton::Primary, pressed: true, ..
                } => Some(Kind::Button),
                _ => None,
            };
            if let Some(kind) = kind {
                let stamp = stamps.as_ref().map(|stamps| stamps[index]);
                s.presses.push(Seen { kind, stamp, frame: frame_at, frame_no });
            }
        }
        s.knob = Some(knob.rect);
        s.pad = pad.as_ref().map(|p| p.rect);
        s.pad_focused = pad.as_ref().is_some_and(egui::Response::has_focus);
        drop(s);

        if slow_ms > 0 {
            // A heavy frame: events that arrive meanwhile wait in the X connection until the loop reads them.
            thread::sleep(Duration::from_millis(slow_ms));
        }
    }
}

// ---------------------------------------------------------------- measurements

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

/// Signed `a - b` in milliseconds.
fn diff_ms(a: Instant, b: Instant) -> f64 {
    if a >= b { ms(a - b) } else { -ms(b - a) }
}

/// Pairs the presses of one kind seen from `start` on with the instants they were sent, in order.
fn pair(shared: &Arc<Mutex<State>>, start: usize, kind: Kind, sent: &[Instant]) -> Vec<(Instant, Seen)> {
    let s = shared.lock().unwrap();
    let seen: Vec<Seen> = s.presses[start..].iter().filter(|p| p.kind == kind).cloned().collect();
    sent.iter().copied().zip(seen).collect()
}

struct Taps {
    name: &'static str,
    pairs: Vec<(Instant, Seen)>,
    expected: usize,
}

impl Taps {
    fn stamp_lags(&self) -> Vec<f64> {
        self.pairs.iter().filter_map(|(sent, seen)| seen.stamp.map(|st| diff_ms(st, *sent))).collect()
    }

    fn frame_lags(&self) -> Vec<f64> {
        self.pairs.iter().map(|(sent, seen)| diff_ms(seen.frame, *sent)).collect()
    }

    /// |interval between two taps' stamps (or frame times) - interval between their sends|, per pair of taps.
    fn interval_errors(&self, use_stamp: bool) -> Vec<f64> {
        self.pairs
            .windows(2)
            .filter_map(|w| {
                let (s0, a) = &w[0];
                let (s1, b) = &w[1];
                let (t0, t1) =
                    if use_stamp { (a.stamp?, b.stamp?) } else { (a.frame, b.frame) };
                Some((diff_ms(t1, t0) - diff_ms(*s1, *s0)).abs())
            })
            .collect()
    }

    fn frames_shared(&self) -> usize {
        self.pairs.windows(2).filter(|w| w[0].1.frame_no == w[1].1.frame_no).count()
    }
}

/// Sends `count` taps (`press` returns the instant each was sent) with the given gaps, cycled, then waits until the
/// editor saw them all.
fn send_taps(
    h: &mut Host,
    shared: &Arc<Mutex<State>>,
    name: &'static str,
    kind: Kind,
    count: usize,
    gaps_ms: &[u64],
    mut press: impl FnMut(&mut Host) -> Instant,
) -> Taps {
    h.settle(300);
    let start = shared.lock().unwrap().presses.len();
    let mut sent = Vec::with_capacity(count);
    for i in 0..count {
        sent.push(press(h));
        thread::sleep(Duration::from_millis(gaps_ms[i % gaps_ms.len()]));
    }
    let shared2 = Arc::clone(shared);
    h.wait(3000, move |_| {
        let s = shared2.lock().unwrap();
        s.presses[start..].iter().filter(|p| p.kind == kind).count() >= count
    });
    h.settle(200);
    Taps { name, pairs: pair(shared, start, kind, &sent), expected: count }
}

fn report(taps: &Taps) {
    println!(
        "  {}: {} of {} taps seen, {} consecutive pairs in one frame",
        taps.name,
        taps.pairs.len(),
        taps.expected,
        taps.frames_shared()
    );
    println!("    lag sent -> stamp: {}", summary_ms(&taps.stamp_lags()));
    println!("    lag sent -> frame: {}", summary_ms(&taps.frame_lags()));
    println!("    interval error, stamps: {}", summary_ms(&taps.interval_errors(true)));
    println!("    interval error, frames: {}", summary_ms(&taps.interval_errors(false)));
}

// ---------------------------------------------------------------- the run

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let reparent = args.iter().any(|a| a == "--reparent");
    let claim = !args.iter().any(|a| a == "--no-claim");
    if let Some(other) = args.iter().find(|a| *a != "--reparent" && *a != "--no-claim") {
        eprintln!("usage: tap-probe [--reparent] [--no-claim], not {other:?}");
        std::process::exit(2);
    }
    if let Err(e) = require_xvfb() {
        eprintln!("tap-probe: {e}. Run: xvfb-run -a -s \"-screen 0 1920x1080x24\" tap-probe");
        std::process::exit(2);
    }

    let mut h = Host::new();
    let shared = Arc::new(Mutex::new(State {
        frames: 0,
        presses: vec![],
        all_stamps: vec![],
        frames_with_events: 0,
        unstamped_frames: 0,
        frame_after_ui: 0,
        no_origin: false,
        pad: None,
        knob: None,
        pad_focused: false,
        show_pad: true,
        claim,
        slow_ms: 0,
    }));
    let parent = Parent(h.host);
    let settings = EguiWindowSettings::new()
        .with_title("tap-probe")
        .with_size(Size::Logical(LogicalSize { width: 300.0, height: 200.0 }))
        .with_resizable(false)
        .with_keyboard_focus(KeyboardFocus::WhileTyping);
    // A CLAP host creates the editor first and gives it its parent afterwards.
    let settings =
        if reparent { settings.with_wait_for_parent(true) } else { settings.with_parent(&parent) };
    let window = EguiWindow::create(settings, Probe { shared: Arc::clone(&shared) })
        .expect("create the editor window");
    if reparent {
        window
            .set_parent(baseview::ParentWindowHandle::from_window(&parent))
            .expect("set the parent window");
    }
    window.show().expect("show the editor window");

    let shared2 = Arc::clone(&shared);
    let up = h.wait(5000, move |_| {
        let s = shared2.lock().unwrap();
        s.frames >= 2 && s.pad.is_some()
    });
    if !up {
        eprintln!("tap-probe: the editor drew no frame in 5 s");
        std::process::exit(2);
    }
    h.find_child();
    h.settle(300);
    let (pad, knob) = {
        let s = shared.lock().unwrap();
        (centre(s.pad.unwrap()), centre(s.knob.unwrap()))
    };
    let set = |f: &dyn Fn(&mut State)| f(&mut shared.lock().unwrap());

    println!(
        "tap-probe: {}, {}; host window {:#x}, editor window {:#x}",
        if claim { "the pad claims the keyboard" } else { "negative control: the pad does not claim" },
        if reparent { "created without a parent, reparented (the CLAP way)" } else { "created inside the host window" },
        h.host,
        h.child
    );
    let f = h.focus();
    let name = h.name_of(f);
    h.record("start: the host has the input focus", f == h.host, format!("focus = {name}"));

    // ------------------------------------------------ the claim
    let space_to_host = |h: &mut Host, shared: &Arc<Mutex<State>>| -> (u32, usize) {
        let (host_before, seen_before) = (h.host_keys, shared.lock().unwrap().presses.len());
        h.key(SPACE);
        let editor = shared.lock().unwrap().presses[seen_before..]
            .iter()
            .filter(|p| p.kind == Kind::Space)
            .count();
        (h.host_keys - host_before, editor)
    };

    h.move_to(pad.0, pad.1);
    let (host_got, editor_got) = space_to_host(&mut h, &shared);
    h.record(
        "hover over the pad: Space reaches the host",
        host_got == 1 && editor_got == 0,
        format!("host got {host_got}, editor saw {editor_got}"),
    );

    h.move_to(knob.0, knob.1);
    h.click();
    let (host_got, editor_got) = space_to_host(&mut h, &shared);
    let detail = format!("{}, host got {host_got}, editor saw {editor_got}", h.focus_name());
    h.record("after a click on the knob: Space reaches the host", host_got == 1 && editor_got == 0, detail);

    h.move_to(pad.0, pad.1);
    h.click();
    let took = h.wait(1500, |h| h.focus() == h.child);
    let focused = shared.lock().unwrap().pad_focused;
    let (host_got, editor_got) = space_to_host(&mut h, &shared);
    let detail = format!(
        "pad focused = {focused}, took = {took}, {}, host got {host_got}, editor saw {editor_got}",
        h.focus_name()
    );
    if claim {
        h.record(
            "a press on the pad takes the keyboard: Space reaches the editor, not the host",
            focused && took && host_got == 0 && editor_got == 1,
            detail,
        );
    } else {
        h.record(
            "control, a press on the pad without a claim: Space still reaches the host",
            focused && !took && host_got == 1 && editor_got == 0,
            detail,
        );
    }

    if claim {
        h.key(ESCAPE);
        let back = h.wait(1500, |h| h.focus() == h.host);
        let focused = shared.lock().unwrap().pad_focused;
        let (host_got, editor_got) = space_to_host(&mut h, &shared);
        h.record(
            "Escape ends the claim: the host has the keyboard again",
            back && !focused && host_got == 1 && editor_got == 0,
            format!("back = {back}, pad focused = {focused}, host got {host_got}, editor saw {editor_got}"),
        );

        h.click();
        let took = h.wait(1500, |h| h.focus() == h.child);
        h.move_to(knob.0, knob.1);
        h.click();
        let back = h.wait(1500, |h| h.focus() == h.host);
        let (host_got, editor_got) = space_to_host(&mut h, &shared);
        h.record(
            "a click elsewhere ends the claim",
            took && back && host_got == 1 && editor_got == 0,
            format!("took = {took}, back = {back}, host got {host_got}, editor saw {editor_got}"),
        );

        h.move_to(pad.0, pad.1);
        h.click();
        let took = h.wait(1500, |h| h.focus() == h.child);
        h.focus_host();
        let released = h.wait(1500, |_| !shared.lock().unwrap().pad_focused);
        // Draw frames meanwhile (pointer moves): the pad must not take the keyboard back on its own.
        let mut retaken = false;
        for i in 0..12 {
            h.move_to_quiet(pad.0 + (i % 3) as f32 * 10.0, pad.1);
            h.settle(50);
            retaken |= h.focus() != h.host;
        }
        let (host_got, editor_got) = space_to_host(&mut h, &shared);
        h.record(
            "another window takes the focus: the claim ends and is not taken again",
            took && released && !retaken && host_got == 1 && editor_got == 0,
            format!(
                "took = {took}, released = {released}, retaken = {retaken}, host got {host_got}, editor saw {editor_got}"
            ),
        );

        h.click();
        let took = h.wait(1500, |h| h.focus() == h.child);
        set(&|s| s.show_pad = false);
        h.move_to(pad.0 + 5.0, pad.1 + 5.0);
        let back = h.wait(1500, |h| h.focus() == h.host);
        let (host_got, editor_got) = space_to_host(&mut h, &shared);
        h.record(
            "the pad goes away (another view): the claim ends",
            took && back && host_got == 1 && editor_got == 0,
            format!("took = {took}, back = {back}, host got {host_got}, editor saw {editor_got}"),
        );
        set(&|s| s.show_pad = true);
        h.move_to(pad.0, pad.1);
    }

    // ------------------------------------------------ the arrival stamps
    if claim {
        let gaps = [17, 41, 9, 63, 29, 5, 88, 23];
        let mouse = send_taps(&mut h, &shared, "mouse taps on the pad", Kind::Button, 24, &gaps, Host::press_button);

        h.click();
        let took = h.wait(1500, |h| h.focus() == h.child);
        h.record("the pad holds the keyboard for the key taps", took, h.focus_name());
        let keys = send_taps(&mut h, &shared, "Space taps", Kind::Space, 24, &gaps, |h| h.press_key(SPACE));

        // Bursts: four taps 3 ms apart, mostly within one frame.
        let mut burst_pairs = vec![];
        for _ in 0..6 {
            let taps = send_taps(&mut h, &shared, "burst", Kind::Space, 4, &[3], |h| h.press_key(SPACE));
            burst_pairs.extend(taps.pairs.windows(2).map(|w| (w[0].clone(), w[1].clone())));
        }

        set(&|s| s.slow_ms = 40);
        let slow = send_taps(&mut h, &shared, "Space taps during 40 ms frames", Kind::Space, 18, &[13, 7, 21], |h| {
            h.press_key(SPACE)
        });
        set(&|s| s.slow_ms = 0);

        println!("arrival stamps (Xvfb, llvmpipe; milliseconds from the instant XTest was sent):");
        for taps in [&mouse, &keys, &slow] {
            report(taps);
        }

        let mut all: Vec<(Instant, Seen)> = vec![];
        for taps in [&mouse, &keys, &slow] {
            let complete = taps.pairs.len() == taps.expected;
            h.record(
                &format!("{}: the editor saw every tap", taps.name),
                complete,
                format!("{} of {}", taps.pairs.len(), taps.expected),
            );
            all.extend(taps.pairs.iter().cloned());
        }
        all.extend(burst_pairs.iter().flat_map(|(a, b)| [a.clone(), b.clone()]));

        let s = shared.lock().unwrap();
        let (frames, with_events, unstamped, after_ui, no_origin) =
            (s.frames, s.frames_with_events, s.unstamped_frames, s.frame_after_ui, s.no_origin);
        let all_stamps = s.all_stamps.clone();
        drop(s);

        let missing = all.iter().filter(|(_, seen)| seen.stamp.is_none()).count();
        h.record(
            "every event of every frame has a stamp",
            unstamped == 0 && missing == 0 && !no_origin && !all_stamps.is_empty(),
            format!(
                "{frames} frames, {with_events} with events, {unstamped} without stamps; {missing} taps without a stamp; \
                 {} stamps; origin {}",
                all_stamps.len(),
                if no_origin { "missing" } else { "present" }
            ),
        );
        let before_frame: Vec<f64> =
            all.iter().filter_map(|(_, seen)| seen.stamp.map(|st| diff_ms(seen.frame, st))).collect();
        h.record(
            "each stamp precedes the frame that saw the event",
            before_frame.iter().all(|d| *d >= 0.0) && after_ui == 0,
            format!(
                "frame time - stamp: min {:.3} ms, {}; frames timed after their ui() started: {after_ui}",
                percentile(&before_frame, 0.0),
                summary_ms(&before_frame)
            ),
        );
        let after_send: Vec<f64> =
            all.iter().filter_map(|(sent, seen)| seen.stamp.map(|st| diff_ms(st, *sent))).collect();
        h.record(
            "each stamp follows the instant the event was sent",
            after_send.iter().all(|d| *d >= 0.0),
            format!("stamp - sent: min {:.3} ms", percentile(&after_send, 0.0)),
        );
        let backwards = all_stamps.windows(2).filter(|w| w[1] < w[0]).count();
        h.record(
            "stamps never go backwards (all events, in order)",
            backwards == 0,
            format!("{backwards} of {} steps backwards", all_stamps.len().saturating_sub(1)),
        );

        let same_frame: Vec<&((Instant, Seen), (Instant, Seen))> =
            burst_pairs.iter().filter(|(a, b)| a.1.frame_no == b.1.frame_no).collect();
        let errors: Vec<f64> = same_frame
            .iter()
            .filter_map(|((s0, a), (s1, b))| {
                Some((diff_ms(b.stamp?, a.stamp?) - diff_ms(*s1, *s0)).abs())
            })
            .collect();
        let distinct = same_frame.iter().all(|(a, b)| a.1.stamp < b.1.stamp);
        h.record(
            "two taps in one frame keep their own arrival instants",
            !same_frame.is_empty() && distinct && percentile(&errors, 50.0) <= 2.0,
            format!(
                "{} of {} burst pairs shared a frame; stamps distinct = {distinct}; |stamp gap - sent gap|: {}",
                same_frame.len(),
                burst_pairs.len(),
                summary_ms(&errors)
            ),
        );

        for taps in [&mouse, &keys] {
            let (st, fr) = (taps.interval_errors(true), taps.interval_errors(false));
            h.record(
                &format!("{}: stamp intervals follow the sent intervals better than frame times", taps.name),
                percentile(&st, 95.0) < percentile(&fr, 95.0) && percentile(&st, 50.0) <= percentile(&fr, 50.0),
                format!("p95 {:.2} ms against {:.2} ms", percentile(&st, 95.0), percentile(&fr, 95.0)),
            );
        }
    }

    window.close();
    h.settle(300);
    println!("{} of {} checks failed", h.failed(), h.checks());
    std::process::exit(i32::from(h.failed() != 0));
}
