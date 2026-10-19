//! Focus probe: drives the vendored egui-baseview (keyboard focus only while typing) with XTest.
//!
//! **Xvfb only.** It plays the host: a toplevel window `H` that has the X input focus and selects key events
//! (what REAPER's window looks like to an embedded editor), with the egui editor as a child window. It then
//! clicks, types and presses Space through the XTest extension, and checks who has the input focus and whose
//! window receives each key: the host's, or the editor's.
//!
//!   xvfb-run -a -s "-screen 0 1920x1080x24" focus-probe [while-typing|on-click]
//!
//! `on-click` is upstream's behaviour: the negative control, which must show the Space key being swallowed.

use std::num::NonZeroU32;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use baseview::dpi::LogicalSize;
use baseview::dpi::Size;
use egui_baseview::{EguiWindow, EguiWindowSettings, Frame, KeyboardFocus};
use raw_window_handle::{
    HandleError, HasWindowHandle, RawWindowHandle, WindowHandle, XcbWindowHandle,
};
use x11rb::CURRENT_TIME;
use x11rb::connection::Connection;
use x11rb::protocol::Event;
use x11rb::protocol::xproto::{
    BUTTON_PRESS_EVENT, BUTTON_RELEASE_EVENT, ConnectionExt, CreateWindowAux, EventMask,
    InputFocus, KEY_PRESS_EVENT, KEY_RELEASE_EVENT, MOTION_NOTIFY_EVENT, Window, WindowClass,
};
use x11rb::protocol::xtest::ConnectionExt as _;
use x11rb::rust_connection::RustConnection;

// ---------------------------------------------------------------- the Xvfb guard (as in tools/test-host)

fn require_xvfb() -> Result<(), String> {
    if std::env::var_os("WAYLAND_DISPLAY").is_some_and(|v| !v.is_empty()) {
        return Err("WAYLAND_DISPLAY is set; refusing to open windows".into());
    }
    let display = std::env::var("DISPLAY").unwrap_or_default();
    let (host, rest) = display.split_once(':').ok_or("DISPLAY is not set or has no ':'")?;
    if !(host.is_empty() || host == "unix") {
        return Err(format!("DISPLAY={display} is remote; refusing"));
    }
    let number = rest.split('.').next().unwrap_or("");
    let want = format!(":{number}");
    for entry in std::fs::read_dir("/proc").map_err(|e| e.to_string())?.flatten() {
        let Ok(raw) = std::fs::read(entry.path().join("cmdline")) else { continue };
        let text = String::from_utf8_lossy(&raw);
        let argv: Vec<&str> = text.split('\0').filter(|a| !a.is_empty()).collect();
        let is_xvfb = argv.first().is_some_and(|p| p.rsplit('/').next() == Some("Xvfb"));
        if is_xvfb && argv[1..].contains(&want.as_str()) {
            return Ok(());
        }
    }
    Err(format!("DISPLAY={display} is not served by an Xvfb process; refusing to open windows"))
}

// ---------------------------------------------------------------- the editor

#[derive(Default, Clone)]
struct State {
    frames: u64,
    /// Key presses egui saw in its input.
    keys_seen: u32,
    text: String,
    text_focused: bool,
    button: Option<egui::Rect>,
    edit: Option<egui::Rect>,
}

struct Probe {
    shared: Arc<Mutex<State>>,
    text: String,
}

impl egui_baseview::App for Probe {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut Frame) {
        let keys = ui.input(|i| {
            i.events
                .iter()
                .filter(|e| matches!(e, egui::Event::Key { pressed: true, .. }))
                .count()
        });
        let button = ui.button("knob");
        let edit = ui.add(
            egui::TextEdit::singleline(&mut self.text)
                .desired_width(200.0),
        );
        let mut state = self.shared.lock().unwrap();
        state.frames += 1;
        state.keys_seen += keys as u32;
        state.text = self.text.clone();
        state.text_focused = edit.has_focus();
        state.button = Some(button.rect);
        state.edit = Some(edit.rect);
    }
}

struct Parent(u32);

impl HasWindowHandle for Parent {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        let id = NonZeroU32::new(self.0).ok_or(HandleError::Unavailable)?;
        // SAFETY: the handle names a window that this process created and keeps alive.
        Ok(unsafe { WindowHandle::borrow_raw(RawWindowHandle::Xcb(XcbWindowHandle::new(id))) })
    }
}

// ---------------------------------------------------------------- the host and the driver

struct Check {
    pass: bool,
}

struct Host {
    conn: RustConnection,
    root: Window,
    host: Window,
    child: Window,
    host_keys: u32,
    keymap: Vec<(u8, Vec<u32>)>,
    checks: Vec<Check>,
}

impl Host {
    fn drain(&mut self) {
        while let Some(event) = self.conn.poll_for_event().unwrap() {
            if let Event::KeyPress(e) = event
                && e.event == self.host
            {
                self.host_keys += 1;
            }
        }
    }

    fn settle(&mut self, ms: u64) {
        thread::sleep(Duration::from_millis(ms));
        self.drain();
    }

    fn focus(&mut self) -> u32 {
        self.conn.get_input_focus().unwrap().reply().unwrap().focus
    }

    fn fake(&self, kind: u8, detail: u8, x: i16, y: i16) {
        self.conn.xtest_fake_input(kind, detail, 0, self.root, x, y, 0).unwrap();
        self.conn.flush().unwrap();
    }

    /// Moves the pointer to a point of the editor window (editor coordinates, logical = physical here).
    fn move_to(&mut self, x: f32, y: f32) {
        let r = self
            .conn
            .translate_coordinates(self.child, self.root, x as i16, y as i16)
            .unwrap()
            .reply()
            .unwrap();
        self.fake(MOTION_NOTIFY_EVENT, 0, r.dst_x, r.dst_y);
        self.settle(120);
    }

    fn click(&mut self) {
        self.fake(BUTTON_PRESS_EVENT, 1, 0, 0);
        self.fake(BUTTON_RELEASE_EVENT, 1, 0, 0);
        self.settle(250);
    }

    fn keycode(&self, keysym: u32) -> u8 {
        self.keymap
            .iter()
            .find(|(_, syms)| syms.contains(&keysym))
            .map(|(code, _)| *code)
            .unwrap_or_else(|| panic!("no keycode for keysym {keysym:#x}"))
    }

    fn key(&mut self, keysym: u32) {
        let code = self.keycode(keysym);
        self.fake(KEY_PRESS_EVENT, code, 0, 0);
        self.fake(KEY_RELEASE_EVENT, code, 0, 0);
        self.settle(250);
    }

    fn wait(&mut self, ms: u64, mut cond: impl FnMut(&mut Self) -> bool) -> bool {
        let end = Instant::now() + Duration::from_millis(ms);
        loop {
            self.drain();
            if cond(self) {
                return true;
            }
            if Instant::now() > end {
                return false;
            }
            thread::sleep(Duration::from_millis(20));
        }
    }

    fn record(&mut self, name: &str, pass: bool, detail: String) {
        println!("{} {name}: {detail}", if pass { "PASS" } else { "FAIL" });
        self.checks.push(Check { pass });
    }

    fn focus_name(&mut self) -> String {
        let f = self.focus();
        format!("focus = {}", self.name_of(f))
    }

    fn name_of(&self, window: u32) -> String {
        match window {
            w if w == self.host => "host".into(),
            w if w == self.child => "editor".into(),
            0 => "None".into(),
            1 => "PointerRoot".into(),
            w => format!("{w:#x}"),
        }
    }
}

const SPACE: u32 = 0x20;
const KEY_A: u32 = 0x61;
const KEY_B: u32 = 0x62;
const KEY_C: u32 = 0x63;
const RETURN: u32 = 0xff0d;
const ESCAPE: u32 = 0xff1b;

fn centre(r: egui::Rect) -> (f32, f32) {
    (r.center().x, r.center().y)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let reparent = args.iter().any(|a| a == "--reparent");
    let policy = match args.iter().find(|a| !a.starts_with("--")).map(String::as_str) {
        None | Some("while-typing") => KeyboardFocus::WhileTyping,
        Some("on-click") => KeyboardFocus::OnClick,
        Some(other) => {
            eprintln!("usage: focus-probe [while-typing|on-click] [--reparent], not {other:?}");
            std::process::exit(2);
        }
    };
    if let Err(e) = require_xvfb() {
        eprintln!("focus-probe: {e}. Run: xvfb-run -a -s \"-screen 0 1920x1080x24\" focus-probe");
        std::process::exit(2);
    }
    let typing_policy = policy == KeyboardFocus::WhileTyping;

    // The host: a toplevel that has the input focus and wants key events.
    let (conn, screen_num) = RustConnection::connect(None).unwrap();
    let screen = conn.setup().roots[screen_num].clone();
    let host = conn.generate_id().unwrap();
    conn.create_window(
        screen.root_depth,
        host,
        screen.root,
        0,
        0,
        400,
        300,
        0,
        WindowClass::INPUT_OUTPUT,
        0,
        &CreateWindowAux::new()
            .background_pixel(0x00ff00ff)
            .event_mask(EventMask::KEY_PRESS | EventMask::KEY_RELEASE | EventMask::FOCUS_CHANGE),
    )
    .unwrap();
    conn.map_window(host).unwrap();
    conn.set_input_focus(InputFocus::POINTER_ROOT, host, CURRENT_TIME).unwrap();
    conn.flush().unwrap();

    let first = conn.setup().min_keycode;
    let count = conn.setup().max_keycode - first + 1;
    let mapping = conn.get_keyboard_mapping(first, count).unwrap().reply().unwrap();
    let per = mapping.keysyms_per_keycode as usize;
    let keymap: Vec<(u8, Vec<u32>)> = mapping
        .keysyms
        .chunks(per)
        .enumerate()
        .map(|(i, syms)| (first + i as u8, syms.to_vec()))
        .collect();

    // The editor, embedded in the host window.
    let shared = Arc::new(Mutex::new(State::default()));
    let parent = Parent(host);
    let settings = EguiWindowSettings::new()
        .with_title("focus-probe")
        .with_size(Size::Logical(LogicalSize { width: 300.0, height: 200.0 }))
        .with_resizable(false)
        .with_keyboard_focus(policy);
    // A CLAP host creates the editor first and gives it its parent afterwards.
    let settings = if reparent { settings.with_wait_for_parent(true) } else { settings.with_parent(&parent) };
    let window = EguiWindow::create(
        settings,
        Probe { shared: Arc::clone(&shared), text: String::new() },
    )
    .expect("create the editor window");
    if reparent {
        window
            .set_parent(baseview::ParentWindowHandle::from_window(&parent))
            .expect("set the parent window");
    }
    window.show().expect("show the editor window");

    let mut h = Host { conn, root: screen.root, host, child: 0, host_keys: 0, keymap, checks: vec![] };
    let up = |h: &mut Host, shared: &Arc<Mutex<State>>| {
        let shared = Arc::clone(shared);
        h.wait(5000, move |_| {
            let s = shared.lock().unwrap();
            s.frames >= 2 && s.button.is_some()
        })
    };
    assert!(up(&mut h, &shared), "the editor drew no frame in 5 s");
    h.child = *h.conn.query_tree(host).unwrap().reply().unwrap().children.last().expect("a child window");
    h.settle(300);
    let snapshot = |shared: &Arc<Mutex<State>>| shared.lock().unwrap().clone();
    let button = centre(snapshot(&shared).button.unwrap());
    let edit = centre(snapshot(&shared).edit.unwrap());

    println!("policy: {policy:?}, {}; host window {host:#x}, editor window {:#x}", if reparent { "created without a parent, reparented (the CLAP way)" } else { "created inside the host window" }, h.child);
    let f = h.focus();
    let name = h.name_of(f);
    h.record("start: the host has the input focus", f == host, format!("focus = {name}"));

    // 1. Hover only: the pointer is over the editor, nothing was ever clicked.
    h.move_to(button.0, button.1);
    let (host_before, seen_before) = (h.host_keys, snapshot(&shared).keys_seen);
    h.key(SPACE);
    let (host_got, editor_got) = (h.host_keys - host_before, snapshot(&shared).keys_seen - seen_before);
    h.record(
        "hover: Space reaches the host",
        (host_got == 1) == typing_policy,
        format!("host got {host_got}, editor saw {editor_got} (expected host {})", u8::from(typing_policy)),
    );

    // 2. Click a knob-like button: no text field, so no keyboard.
    h.click();
    let f = h.focus();
    let name = h.name_of(f);
    h.record(
        "click on a button: the input focus",
        if typing_policy { f == host } else { f == h.child },
        format!("focus = {name} (expected {})", if typing_policy { "host" } else { "editor" }),
    );
    let (host_before, seen_before) = (h.host_keys, snapshot(&shared).keys_seen);
    h.key(SPACE);
    let (host_got, editor_got) = (h.host_keys - host_before, snapshot(&shared).keys_seen - seen_before);
    h.record(
        "after a click on a button: Space reaches the host",
        (host_got == 1) == typing_policy,
        format!("host got {host_got}, editor saw {editor_got} (expected host {})", u8::from(typing_policy)),
    );

    // 3. Click the text field and type.
    h.move_to(edit.0, edit.1);
    h.click();
    let took = h.wait(1500, |h| h.focus() == h.child);
    let detail = h.focus_name();
    h.record("click on the text field: the editor takes the input focus", took, detail);
    let host_before = h.host_keys;
    h.key(KEY_A);
    h.key(KEY_B);
    let text = snapshot(&shared).text;
    h.record(
        "typing goes to the text field, not the host",
        text == "ab" && h.host_keys == host_before,
        format!("text = {text:?}, host got {}", h.host_keys - host_before),
    );

    if typing_policy {
        // 4. Click away: the field loses egui's focus, the host gets the keyboard back.
        h.move_to(button.0, button.1);
        h.click();
        let back = h.wait(1500, |h| h.focus() == h.host);
        let detail = h.focus_name();
        h.record("click away from the field: the host has the input focus again", back, detail);
        let (host_before, text_before) = (h.host_keys, snapshot(&shared).text);
        h.key(SPACE);
        let text_after = snapshot(&shared).text;
        h.record(
            "after typing: Space reaches the host again",
            h.host_keys - host_before == 1 && text_before == text_after,
            format!("host got {}, text {text_after:?}", h.host_keys - host_before),
        );

        // 5. Enter ends the edit.
        h.move_to(edit.0, edit.1);
        h.click();
        let took = h.wait(1500, |h| h.focus() == h.child);
        h.key(KEY_C);
        h.key(RETURN);
        let back = h.wait(1500, |h| h.focus() == h.host);
        let text = snapshot(&shared).text;
        h.record(
            "Enter ends the edit and gives the keyboard back",
            took && back && text.ends_with('c'),
            format!("took = {took}, back = {back}, text = {text:?}"),
        );

        // 6. Escape ends the edit.
        h.click();
        let took = h.wait(1500, |h| h.focus() == h.child);
        h.key(ESCAPE);
        let back = h.wait(1500, |h| h.focus() == h.host);
        h.record(
            "Escape ends the edit and gives the keyboard back",
            took && back,
            format!("took = {took}, back = {back}"),
        );

        // 7. Another window takes the focus while the field is open.
        h.click();
        let took = h.wait(1500, |h| h.focus() == h.child);
        h.conn.set_input_focus(InputFocus::POINTER_ROOT, host, CURRENT_TIME).unwrap();
        h.conn.flush().unwrap();
        let cancelled = h.wait(1500, |_| !shared.lock().unwrap().text_focused);
        let host_before = h.host_keys;
        h.key(SPACE);
        h.record(
            "focus taken by another window: the edit is cancelled and Space reaches the host",
            took && cancelled && h.host_keys - host_before == 1,
            format!("took = {took}, cancelled = {cancelled}, host got {}", h.host_keys - host_before),
        );
    }

    window.close();
    h.settle(300);
    let failed = h.checks.iter().filter(|c| !c.pass).count();
    println!("{} of {} checks failed ({policy:?})", failed, h.checks.len());
    if !typing_policy {
        println!(
            "negative control: upstream's behaviour swallows Space (the host got none) as soon as the pointer is over the editor, \
             and keeps the keyboard after a click; the checks above expect exactly that"
        );
    }
    std::process::exit(i32::from(failed != 0));
}
