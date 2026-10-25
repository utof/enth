//! Paste probe: fork patch 13 in the vendored egui-baseview (`ViewportCommand::RequestPaste`), under Xvfb.
//!
//! **Xvfb only.** It plays the host (a toplevel with the egui editor embedded in it) and a clipboard owner: on its own X
//! connection it owns `CLIPBOARD` and answers `ConvertSelection` requests, counting each one, so it sees every read of
//! the clipboard. The editor has a Paste button that sends `ViewportCommand::RequestPaste` (what a plugin's Paste button
//! does, lane DUI's duel card) and a knob-like button, and records each `egui::Event::Paste` it sees.
//!
//!   xvfb-run -a -s "-screen 0 1920x1080x24" paste-probe
//!
//! Checks: hovering, clicks on other widgets and plain keys read nothing; one Paste click reads the clipboard once and
//! the next frame has one `Event::Paste` with its text (Windows line endings as `\n`, as eframe does); a second click
//! reads the new text; an empty clipboard and a clipboard with no owner paste nothing and leave the editor drawing.
//! The library and this probe use no patched API, so `run.sh` also builds them against upstream egui-baseview 0.9.0:
//! the negative control, where the Paste button reads nothing and pastes nothing.
//!
//! Exit status: 0 when every check passed, 1 when one failed, 2 on a setup error.

use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use baseview::dpi::{LogicalSize, Size};
use egui_baseview::{EguiWindow, EguiWindowSettings, Frame};
use input_probe::{Host, KEY_V, Parent, SPACE, centre, require_xvfb};
use x11rb::connection::Connection;
use x11rb::protocol::Event;
use x11rb::protocol::xproto::{
    Atom, AtomEnum, ConnectionExt, CreateWindowAux, EventMask, PropMode, SELECTION_NOTIFY_EVENT,
    SelectionNotifyEvent, SelectionRequestEvent, WindowClass,
};
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;
use x11rb::{CURRENT_TIME, NONE};

// ---------------------------------------------------------------- the editor

#[derive(Default)]
struct State {
    frames: u64,
    /// The text of every `Event::Paste` egui saw, in order.
    pastes: Vec<String>,
    paste_button: Option<egui::Rect>,
    knob: Option<egui::Rect>,
}

struct Probe {
    shared: Arc<Mutex<State>>,
}

impl egui_baseview::App for Probe {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut Frame) {
        let pastes: Vec<String> = ui.input(|i| {
            i.events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::Paste(text) => Some(text.clone()),
                    _ => None,
                })
                .collect()
        });
        let paste = ui.button("Paste");
        if paste.clicked() {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::RequestPaste);
        }
        let knob = ui.button("knob");
        let mut s = self.shared.lock().unwrap();
        s.frames += 1;
        s.pastes.extend(pastes);
        s.paste_button = Some(paste.rect);
        s.knob = Some(knob.rect);
    }
}

// ---------------------------------------------------------------- the clipboard owner

#[derive(Default)]
struct Clipboard {
    /// What the owner hands out; `None`: it gives the selection up.
    text: Option<String>,
    /// The target of every `ConvertSelection` the owner was asked, in order.
    requests: Vec<String>,
    /// Set by the driver: the owner takes or gives up `CLIPBOARD` on its next turn.
    owned: bool,
}

/// Owns `CLIPBOARD` on its own connection while `owned` is set, and answers every request for it.
fn run_owner(clipboard: Arc<Mutex<Clipboard>>) {
    let (conn, screen_num) = RustConnection::connect(None).expect("connect the clipboard owner");
    let root = conn.setup().roots[screen_num].root;
    let window = conn.generate_id().unwrap();
    conn.create_window(
        0,
        window,
        root,
        0,
        0,
        1,
        1,
        0,
        WindowClass::INPUT_ONLY,
        0,
        &CreateWindowAux::new().event_mask(EventMask::PROPERTY_CHANGE),
    )
    .unwrap();
    let atom = |name: &str| conn.intern_atom(false, name.as_bytes()).unwrap().reply().unwrap().atom;
    let (clipboard_atom, utf8, targets) = (atom("CLIPBOARD"), atom("UTF8_STRING"), atom("TARGETS"));
    conn.flush().unwrap();

    let mut owning = false;
    loop {
        let want = clipboard.lock().unwrap().owned;
        if want != owning {
            let owner = if want { window } else { NONE };
            conn.set_selection_owner(owner, clipboard_atom, CURRENT_TIME).unwrap();
            conn.flush().unwrap();
            owning = want;
        }
        let Some(event) = conn.poll_for_event().unwrap() else {
            thread::sleep(Duration::from_millis(2));
            continue;
        };
        let Event::SelectionRequest(request) = event else { continue };
        let name = conn
            .get_atom_name(request.target)
            .unwrap()
            .reply()
            .map(|r| String::from_utf8_lossy(&r.name).into_owned())
            .unwrap_or_default();
        let text = {
            let mut c = clipboard.lock().unwrap();
            c.requests.push(name);
            c.text.clone()
        };
        let property = answer(&conn, &request, text.as_deref(), utf8, targets);
        let notify = SelectionNotifyEvent {
            response_type: SELECTION_NOTIFY_EVENT,
            sequence: 0,
            time: request.time,
            requestor: request.requestor,
            selection: request.selection,
            target: request.target,
            property,
        };
        conn.send_event(false, request.requestor, EventMask::NO_EVENT, notify).unwrap();
        conn.flush().unwrap();
    }
}

/// Writes the answer to a request on the requestor's property; returns the property, or `NONE` for a refusal.
fn answer(
    conn: &RustConnection,
    request: &SelectionRequestEvent,
    text: Option<&str>,
    utf8: Atom,
    targets: Atom,
) -> Atom {
    let property = if request.property == NONE { request.target } else { request.property };
    match text {
        Some(text) if request.target == utf8 => {
            conn.change_property8(PropMode::REPLACE, request.requestor, property, utf8, text.as_bytes())
                .unwrap();
            property
        }
        Some(_) if request.target == targets => {
            conn.change_property32(
                PropMode::REPLACE,
                request.requestor,
                property,
                AtomEnum::ATOM,
                &[targets, utf8],
            )
            .unwrap();
            property
        }
        _ => NONE,
    }
}

// ---------------------------------------------------------------- the run

fn main() {
    if let Some(other) = std::env::args().nth(1) {
        eprintln!("usage: paste-probe, not {other:?}");
        std::process::exit(2);
    }
    if let Err(e) = require_xvfb() {
        eprintln!("paste-probe: {e}. Run: xvfb-run -a -s \"-screen 0 1920x1080x24\" paste-probe");
        std::process::exit(2);
    }

    let clipboard = Arc::new(Mutex::new(Clipboard::default()));
    {
        let clipboard = Arc::clone(&clipboard);
        thread::spawn(move || run_owner(clipboard));
    }

    let mut h = Host::new();
    let shared = Arc::new(Mutex::new(State::default()));
    let parent = Parent(h.host);
    let settings = EguiWindowSettings::new()
        .with_title("paste-probe")
        .with_size(Size::Logical(LogicalSize { width: 300.0, height: 200.0 }))
        .with_resizable(false)
        .with_parent(&parent);
    let window = EguiWindow::create(settings, Probe { shared: Arc::clone(&shared) })
        .expect("create the editor window");
    window.show().expect("show the editor window");

    let shared2 = Arc::clone(&shared);
    let up = h.wait(5000, move |_| {
        let s = shared2.lock().unwrap();
        s.frames >= 2 && s.paste_button.is_some()
    });
    if !up {
        eprintln!("paste-probe: the editor drew no frame in 5 s");
        std::process::exit(2);
    }
    h.find_child();
    let (paste, knob) = {
        let s = shared.lock().unwrap();
        (centre(s.paste_button.unwrap()), centre(s.knob.unwrap()))
    };
    let set_clipboard = |text: Option<&str>| {
        let mut c = clipboard.lock().unwrap();
        c.owned = text.is_some();
        c.text = text.map(str::to_owned);
    };
    let requests = || clipboard.lock().unwrap().requests.len();
    let pastes = || shared.lock().unwrap().pastes.clone();

    let first = "ENTH-DUEL 7Q2K-W9XA ëñ 🎲\r\nsecond line";
    set_clipboard(Some(first));
    h.settle(300);
    println!("paste-probe: host window {:#x}, editor window {:#x}", h.host, h.child);

    // Everything but the Paste button.
    h.move_to(knob.0, knob.1);
    h.move_to(paste.0, paste.1);
    h.move_to(knob.0, knob.1);
    h.click();
    h.key(SPACE);
    h.key(KEY_V);
    h.settle(300);
    let (read, seen) = (requests(), pastes());
    h.record(
        "no clipboard read without Paste (hover, a knob click, keys)",
        read == 0 && seen.is_empty(),
        format!("{read} clipboard requests, {} paste events", seen.len()),
    );

    h.move_to(paste.0, paste.1);
    h.click();
    h.wait(2000, |_| !shared.lock().unwrap().pastes.is_empty());
    h.settle(200);
    let (read, seen) = (requests(), pastes());
    let want = first.replace("\r\n", "\n");
    h.record(
        "a Paste click reads the clipboard once and egui gets one Event::Paste with its text",
        read == 1 && seen == [want.clone()],
        format!("{read} clipboard requests ({:?}), paste events {seen:?}", clipboard.lock().unwrap().requests),
    );

    let second = "second paste";
    set_clipboard(Some(second));
    h.settle(100);
    h.click();
    h.wait(2000, |_| shared.lock().unwrap().pastes.len() >= 2);
    h.settle(200);
    let (read, seen) = (requests(), pastes());
    h.record(
        "a second Paste click reads the new text",
        read == 2 && seen == [want.clone(), second.to_owned()],
        format!("{read} clipboard requests, paste events {seen:?}"),
    );

    set_clipboard(Some(""));
    h.settle(100);
    h.click();
    h.settle(500);
    let (read, seen) = (requests(), pastes());
    h.record(
        "an empty clipboard pastes nothing",
        read == 3 && seen.len() == 2,
        format!("{read} clipboard requests, {} paste events", seen.len()),
    );

    set_clipboard(None);
    h.settle(100);
    h.click();
    h.settle(500);
    let frames_before = shared.lock().unwrap().frames;
    h.move_to(knob.0, knob.1);
    h.move_to(paste.0, paste.1);
    let drawing = h.wait(1500, |_| shared.lock().unwrap().frames > frames_before);
    let (read, seen) = (requests(), pastes());
    h.record(
        "a clipboard without an owner pastes nothing, and the editor keeps drawing",
        read == 3 && seen.len() == 2 && drawing,
        format!("{read} requests to the probe's owner, {} paste events, drawing = {drawing}", seen.len()),
    );

    window.close();
    h.settle(300);
    println!("{} of {} checks failed", h.failed(), h.checks());
    std::process::exit(i32::from(h.failed() != 0));
}
