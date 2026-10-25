//! What `tap-probe` and `paste-probe` share: the Xvfb guard, a host window that holds the X input focus with the egui
//! editor embedded in it, and XTest input.
//!
//! Nothing here uses an API of the fork's patches, so `paste-probe` also builds against upstream egui-baseview 0.9.0.

use std::num::NonZeroU32;
use std::thread;
use std::time::{Duration, Instant};

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

pub const SPACE: u32 = 0x20;
pub const KEY_V: u32 = 0x76;
pub const ESCAPE: u32 = 0xff1b;

// ---------------------------------------------------------------- the Xvfb guard (as in tools/test-host)

/// Refuses any display that an Xvfb process of this machine does not serve (AGENTS.md, "UI").
pub fn require_xvfb() -> Result<(), String> {
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

/// The host window, as a parent for the editor.
pub struct Parent(pub u32);

impl HasWindowHandle for Parent {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        let id = NonZeroU32::new(self.0).ok_or(HandleError::Unavailable)?;
        // SAFETY: the handle names a window that this process created and keeps alive.
        Ok(unsafe { WindowHandle::borrow_raw(RawWindowHandle::Xcb(XcbWindowHandle::new(id))) })
    }
}

// ---------------------------------------------------------------- the host and the driver

/// A toplevel that has the X input focus and selects key events (what REAPER's window looks like to an embedded
/// editor), and the XTest driver.
pub struct Host {
    pub conn: RustConnection,
    pub root: Window,
    pub host: Window,
    /// The editor's window, once it exists.
    pub child: Window,
    /// Key presses the host window received.
    pub host_keys: u32,
    keymap: Vec<(u8, Vec<u32>)>,
    checks: Vec<bool>,
}

impl Host {
    /// Creates and maps the host window and gives it the input focus.
    pub fn new() -> Self {
        let (conn, screen_num) = RustConnection::connect(None).expect("connect to the X server");
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
        let keymap = mapping
            .keysyms
            .chunks(per)
            .enumerate()
            .map(|(i, syms)| (first + i as u8, syms.to_vec()))
            .collect();

        Self { conn, root: screen.root, host, child: 0, host_keys: 0, keymap, checks: vec![] }
    }

    /// Finds the editor window: the host's last child.
    pub fn find_child(&mut self) {
        let tree = self.conn.query_tree(self.host).unwrap().reply().unwrap();
        self.child = *tree.children.last().expect("a child window");
    }

    pub fn drain(&mut self) {
        while let Some(event) = self.conn.poll_for_event().unwrap() {
            if let Event::KeyPress(e) = event
                && e.event == self.host
            {
                self.host_keys += 1;
            }
        }
    }

    pub fn settle(&mut self, ms: u64) {
        thread::sleep(Duration::from_millis(ms));
        self.drain();
    }

    pub fn focus(&self) -> u32 {
        self.conn.get_input_focus().unwrap().reply().unwrap().focus
    }

    /// Gives the input focus to the host window, as a click on the host would.
    pub fn focus_host(&self) {
        self.conn.set_input_focus(InputFocus::POINTER_ROOT, self.host, CURRENT_TIME).unwrap();
        self.conn.flush().unwrap();
    }

    /// Sends one XTest event and returns the instant just before it was sent.
    pub fn fake(&self, kind: u8, detail: u8, x: i16, y: i16) -> Instant {
        let sent = Instant::now();
        self.conn.xtest_fake_input(kind, detail, 0, self.root, x, y, 0).unwrap();
        self.conn.flush().unwrap();
        sent
    }

    /// Moves the pointer to a point of the editor window (editor coordinates, logical = physical here).
    pub fn move_to(&mut self, x: f32, y: f32) {
        self.move_to_quiet(x, y);
        self.settle(120);
    }

    /// Moves the pointer without waiting.
    pub fn move_to_quiet(&mut self, x: f32, y: f32) {
        let r = self
            .conn
            .translate_coordinates(self.child, self.root, x as i16, y as i16)
            .unwrap()
            .reply()
            .unwrap();
        self.fake(MOTION_NOTIFY_EVENT, 0, r.dst_x, r.dst_y);
    }

    pub fn click(&mut self) {
        self.press_button();
        self.settle(250);
    }

    /// Presses and releases the left button without waiting; returns the instant the press was sent.
    pub fn press_button(&mut self) -> Instant {
        let sent = self.fake(BUTTON_PRESS_EVENT, 1, 0, 0);
        self.fake(BUTTON_RELEASE_EVENT, 1, 0, 0);
        sent
    }

    pub fn keycode(&self, keysym: u32) -> u8 {
        self.keymap
            .iter()
            .find(|(_, syms)| syms.contains(&keysym))
            .map(|(code, _)| *code)
            .unwrap_or_else(|| panic!("no keycode for keysym {keysym:#x}"))
    }

    pub fn key(&mut self, keysym: u32) {
        self.press_key(keysym);
        self.settle(250);
    }

    /// Presses and releases a key without waiting; returns the instant the press was sent.
    pub fn press_key(&mut self, keysym: u32) -> Instant {
        let code = self.keycode(keysym);
        let sent = self.fake(KEY_PRESS_EVENT, code, 0, 0);
        self.fake(KEY_RELEASE_EVENT, code, 0, 0);
        sent
    }

    /// Waits up to `ms` for `cond`, draining the host's events meanwhile.
    pub fn wait(&mut self, ms: u64, mut cond: impl FnMut(&mut Self) -> bool) -> bool {
        let end = Instant::now() + Duration::from_millis(ms);
        loop {
            self.drain();
            if cond(self) {
                return true;
            }
            if Instant::now() > end {
                return false;
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    pub fn record(&mut self, name: &str, pass: bool, detail: impl AsRef<str>) {
        println!("{} {name}: {}", if pass { "PASS" } else { "FAIL" }, detail.as_ref());
        self.checks.push(pass);
    }

    pub fn failed(&self) -> usize {
        self.checks.iter().filter(|pass| !**pass).count()
    }

    pub fn checks(&self) -> usize {
        self.checks.len()
    }

    pub fn name_of(&self, window: u32) -> String {
        match window {
            w if w == self.host => "host".into(),
            w if w == self.child && w != 0 => "editor".into(),
            0 => "None".into(),
            1 => "PointerRoot".into(),
            w => format!("{w:#x}"),
        }
    }

    pub fn focus_name(&self) -> String {
        format!("focus = {}", self.name_of(self.focus()))
    }
}

impl Default for Host {
    fn default() -> Self {
        Self::new()
    }
}

pub fn centre(r: egui::Rect) -> (f32, f32) {
    (r.center().x, r.center().y)
}

/// The `p`-th percentile (0 to 100, nearest rank) of some values, in their unit.
pub fn percentile(values: &[f64], p: f64) -> f64 {
    if values.is_empty() {
        return f64::NAN;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let rank = ((p / 100.0) * sorted.len() as f64).ceil() as usize;
    sorted[rank.clamp(1, sorted.len()) - 1]
}

/// `p50 / p95 / max` of some milliseconds, for a log line.
pub fn summary_ms(values: &[f64]) -> String {
    format!(
        "p50 {:.2} / p95 {:.2} / max {:.2} ms (n = {})",
        percentile(values, 50.0),
        percentile(values, 95.0),
        percentile(values, 100.0),
        values.len()
    )
}
