//! Drop probe: drops files on the vendored egui-baseview through XDND (fork patch 11).
//!
//! **Xvfb only.** It plays a host and a file manager. The host is a toplevel window that is not XDND-aware, with the egui
//! editor embedded in it as a child (`--reparent`: created without a parent and reparented, the CLAP way). The file
//! manager is a drag source on its own X connection that speaks XDND version 5: it finds the outermost XdndAware window
//! under the drop point (Qt's search from the root), owns `XdndSelection`, sends XdndEnter, XdndPosition and XdndDrop
//! (or XdndLeave), and answers the target's `ConvertSelection` with a `text/uri-list`.
//!
//! It checks both sides of the drop:
//!
//! - `[x11]`: what the source hears back (XdndStatus, XdndFinished). That is baseview's part, the same with or without
//!   the patch;
//! - `[egui]`: what egui sees in `RawInput::hovered_files` and `dropped_files`, and whether a drag event alone draws a
//!   frame. That is the patch's part.
//!
//!   xvfb-run -a -s "-screen 0 1920x1080x24" drop-probe [--reparent]
//!
//! Exit status: 0 when every check passed; 1 when only `[egui]` checks failed (baseview took the drop and egui never saw
//! it: upstream egui-baseview 0.9.0, the negative control); 3 when an `[x11]` check failed; 2 on a setup error.
//!
//!   drop-probe --into <window title>
//!
//! A measurement, not a check: on a display where a host already shows a plugin window (REAPER's FX window, on the
//! same Xvfb), it lists the XdndAware windows and the plugin window's ancestors, and drops a WAV file at the plugin
//! window's centre, once where a Qt-style source would and once on the plugin window directly, printing who asked for
//! the data and who answered.

use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use baseview::dpi::{LogicalSize, Size};
use egui_baseview::{EguiWindow, EguiWindowSettings, Frame, RepaintNotifier};
use raw_window_handle::{
    HandleError, HasWindowHandle, RawWindowHandle, WindowHandle, XcbWindowHandle,
};
use x11rb::NONE;
use x11rb::connection::Connection;
use x11rb::protocol::Event;
use x11rb::protocol::xproto::{
    Atom, AtomEnum, ClientMessageEvent, ConnectionExt, CreateWindowAux, EventMask, MapState, PropMode,
    SELECTION_NOTIFY_EVENT, SelectionNotifyEvent, SelectionRequestEvent, Window, WindowClass,
};
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;

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
    /// `hovered_files` in the last frame.
    hovered: Vec<PathBuf>,
    /// Every path that appeared in `dropped_files`, in order.
    dropped: Vec<PathBuf>,
    /// The length `DroppedFile::bytes()` gave for each dropped file.
    dropped_bytes: Vec<Result<usize, String>>,
    /// Frames whose `dropped_files` was not empty.
    drop_frames: u32,
}

struct Probe {
    shared: Arc<Mutex<State>>,
}

impl egui_baseview::App for Probe {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut Frame) {
        let (hovered, dropped) = ui.input(|i| {
            let hovered: Vec<PathBuf> = i.raw.hovered_files.iter().filter_map(|f| f.path.clone()).collect();
            let dropped: Vec<(PathBuf, Result<usize, String>)> = i
                .raw
                .dropped_files
                .iter()
                .map(|f| (f.path().to_path_buf(), f.bytes().map(|b| b.len())))
                .collect();
            (hovered, dropped)
        });
        ui.label(if hovered.is_empty() { "drop a file here" } else { "release to drop" });
        let mut state = self.shared.lock().unwrap();
        state.frames += 1;
        state.hovered = hovered;
        if !dropped.is_empty() {
            state.drop_frames += 1;
            for (path, bytes) in dropped {
                state.dropped.push(path);
                state.dropped_bytes.push(bytes);
            }
        }
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

// ---------------------------------------------------------------- the drag source

fn intern(conn: &RustConnection, name: &str) -> Atom {
    conn.intern_atom(false, name.as_bytes()).unwrap().reply().unwrap().atom
}

struct Atoms {
    aware: Atom,
    proxy: Atom,
    enter: Atom,
    position: Atom,
    status: Atom,
    leave: Atom,
    drop: Atom,
    finished: Atom,
    selection: Atom,
    action_copy: Atom,
    uri_list: Atom,
    text_plain: Atom,
    stamp: Atom,
}

impl Atoms {
    fn new(conn: &RustConnection) -> Self {
        Self {
            aware: intern(conn, "XdndAware"),
            proxy: intern(conn, "XdndProxy"),
            enter: intern(conn, "XdndEnter"),
            position: intern(conn, "XdndPosition"),
            status: intern(conn, "XdndStatus"),
            leave: intern(conn, "XdndLeave"),
            drop: intern(conn, "XdndDrop"),
            finished: intern(conn, "XdndFinished"),
            selection: intern(conn, "XdndSelection"),
            action_copy: intern(conn, "XdndActionCopy"),
            uri_list: intern(conn, "text/uri-list"),
            text_plain: intern(conn, "text/plain"),
            stamp: intern(conn, "_DROP_PROBE_STAMP"),
        }
    }
}

#[derive(Debug, PartialEq)]
enum Reply {
    Status { accept: bool, action: Atom },
    Finished { accepted: bool, action: Atom },
}

/// A drag source on its own connection, as a file manager would be.
struct Source {
    conn: RustConnection,
    root: Window,
    window: Window,
    atoms: Atoms,
    /// Server time at which the source took `XdndSelection`.
    time: u32,
    /// The `text/uri-list` served on `ConvertSelection`.
    uris: Vec<u8>,
    /// `ConvertSelection` requests answered with the list.
    conversions: u32,
    /// The window of every `ConvertSelection` request, answered or not, in order.
    requestors: Vec<Window>,
}

impl Source {
    fn new() -> Self {
        let (conn, screen_num) = RustConnection::connect(None).unwrap();
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
        conn.flush().unwrap();
        let atoms = Atoms::new(&conn);
        Self { conn, root, window, atoms, time: 0, uris: Vec::new(), conversions: 0, requestors: Vec::new() }
    }

    /// A server timestamp, from a zero-length property append (ICCCM §2.1).
    fn server_time(&self) -> u32 {
        self.conn
            .change_property8(PropMode::APPEND, self.window, self.atoms.stamp, AtomEnum::STRING, &[])
            .unwrap();
        self.conn.flush().unwrap();
        loop {
            if let Event::PropertyNotify(e) = self.conn.wait_for_event().unwrap()
                && e.window == self.window
                && e.atom == self.atoms.stamp
            {
                return e.time;
            }
        }
    }

    /// Takes `XdndSelection` for a new drag of `paths`.
    fn start_drag(&mut self, paths: &[PathBuf]) -> bool {
        self.uris = paths.iter().map(|p| file_uri(p) + "\r\n").collect::<String>().into_bytes();
        self.time = self.server_time();
        self.conn.set_selection_owner(self.window, self.atoms.selection, self.time).unwrap();
        let owner = self.conn.get_selection_owner(self.atoms.selection).unwrap().reply().unwrap().owner;
        owner == self.window
    }

    /// The outermost XdndAware window under a root point, searching down from the root (Qt's `QXcbDrag::move`), and the
    /// protocol version it announces.
    fn find_target(&self, x: i16, y: i16) -> Option<(Window, u32)> {
        let mut window = self.root;
        loop {
            let child = self.conn.translate_coordinates(self.root, window, x, y).unwrap().reply().unwrap().child;
            if child == NONE {
                return None;
            }
            let aware = self
                .conn
                .get_property(false, child, self.atoms.aware, AtomEnum::ATOM, 0, 1)
                .unwrap()
                .reply()
                .unwrap();
            if aware.type_ != NONE {
                let version = aware.value32().and_then(|mut v| v.next()).unwrap_or(0);
                return Some((child, version));
            }
            window = child;
        }
    }

    fn has_proxy(&self, window: Window) -> bool {
        let reply =
            self.conn.get_property(false, window, self.atoms.proxy, AtomEnum::WINDOW, 0, 1).unwrap().reply().unwrap();
        reply.type_ != NONE
    }

    fn send(&self, target: Window, kind: Atom, data: [u32; 5]) {
        let event = ClientMessageEvent::new(32, target, kind, data);
        self.conn.send_event(false, target, EventMask::NO_EVENT, event).unwrap();
        self.conn.flush().unwrap();
    }

    fn enter(&self, target: Window, types: [Atom; 3]) {
        self.send(target, self.atoms.enter, [self.window, 5 << 24, types[0], types[1], types[2]]);
    }

    fn position(&self, target: Window, x: i16, y: i16) {
        let xy = ((x as u16 as u32) << 16) | (y as u16 as u32);
        self.send(target, self.atoms.position, [self.window, 0, xy, self.time, self.atoms.action_copy]);
    }

    fn send_drop(&self, target: Window) {
        self.send(target, self.atoms.drop, [self.window, 0, self.time, 0, 0]);
    }

    fn leave(&self, target: Window) {
        self.send(target, self.atoms.leave, [self.window, 0, 0, 0, 0]);
    }

    /// Answers `ConvertSelection` requests until a window sends XdndStatus or XdndFinished, or `ms` pass. Returns the
    /// window that answered (its id is the message's first field) and the answer.
    fn next_reply(&mut self, ms: u64) -> Option<(Window, Reply)> {
        let end = Instant::now() + Duration::from_millis(ms);
        loop {
            while let Some(event) = self.conn.poll_for_event().unwrap() {
                match event {
                    Event::SelectionRequest(request) => self.serve(request),
                    Event::ClientMessage(m) if m.format == 32 => {
                        let d = m.data.as_data32();
                        if m.type_ == self.atoms.status {
                            return Some((d[0], Reply::Status { accept: d[1] & 1 == 1, action: d[4] }));
                        }
                        if m.type_ == self.atoms.finished {
                            return Some((d[0], Reply::Finished { accepted: d[1] & 1 == 1, action: d[2] }));
                        }
                    }
                    _ => {}
                }
            }
            if Instant::now() > end {
                return None;
            }
            thread::sleep(Duration::from_millis(5));
        }
    }

    /// Like [`Self::next_reply`], but waits for XdndFinished: a host's window may send more XdndStatus first. Returns the
    /// answer and how many XdndStatus came before it.
    fn next_finished(&mut self, ms: u64) -> (Option<(Window, Reply)>, u32) {
        let end = Instant::now() + Duration::from_millis(ms);
        let mut statuses = 0;
        loop {
            let left = end.saturating_duration_since(Instant::now()).as_millis() as u64;
            match self.next_reply(left) {
                Some((_, Reply::Status { .. })) => statuses += 1,
                other => return (other, statuses),
            }
        }
    }

    fn serve(&mut self, request: SelectionRequestEvent) {
        self.requestors.push(request.requestor);
        let property = if request.property == NONE { request.target } else { request.property };
        let ok = request.selection == self.atoms.selection && request.target == self.atoms.uri_list;
        if ok {
            self.conn
                .change_property8(PropMode::REPLACE, request.requestor, property, self.atoms.uri_list, &self.uris)
                .unwrap();
            self.conversions += 1;
        }
        let notify = SelectionNotifyEvent {
            response_type: SELECTION_NOTIFY_EVENT,
            sequence: 0,
            time: request.time,
            requestor: request.requestor,
            selection: request.selection,
            target: request.target,
            property: if ok { property } else { NONE },
        };
        self.conn.send_event(false, request.requestor, EventMask::NO_EVENT, notify).unwrap();
        self.conn.flush().unwrap();
    }
}

/// A `file://` URI with every byte outside the unreserved set and `/` percent-encoded (RFC 3986, RFC 8089).
fn file_uri(path: &Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    let mut uri = String::from("file://");
    for &b in path.as_os_str().as_bytes() {
        if b.is_ascii_alphanumeric() || b"/-._~".contains(&b) {
            uri.push(b as char);
        } else {
            uri.push_str(&format!("%{b:02X}"));
        }
    }
    uri
}

// ---------------------------------------------------------------- the checks

#[derive(Clone, Copy, PartialEq)]
enum Side {
    X11,
    Egui,
}

struct Checks {
    list: Vec<(Side, bool)>,
}

impl Checks {
    fn record(&mut self, side: Side, name: &str, pass: bool, detail: String) {
        let tag = match side {
            Side::X11 => "[x11] ",
            Side::Egui => "[egui]",
        };
        println!("{} {tag} {name}: {detail}", if pass { "PASS" } else { "FAIL" });
        self.list.push((side, pass));
    }

    fn failed(&self, side: Side) -> usize {
        self.list.iter().filter(|(s, pass)| *s == side && !pass).count()
    }
}

fn wait(ms: u64, mut cond: impl FnMut() -> bool) -> bool {
    let end = Instant::now() + Duration::from_millis(ms);
    loop {
        if cond() {
            return true;
        }
        if Instant::now() > end {
            return false;
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn names(paths: &[PathBuf]) -> Vec<String> {
    paths.iter().map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()).collect()
}

// ---------------------------------------------------------------- --into: a drop on another program's plugin window

fn window_name(conn: &RustConnection, window: Window) -> String {
    conn.get_property(false, window, AtomEnum::WM_NAME, AtomEnum::ANY, 0, 256)
        .ok()
        .and_then(|cookie| cookie.reply().ok())
        .map(|reply| String::from_utf8_lossy(&reply.value).into_owned())
        .unwrap_or_default()
}

fn aware_version(conn: &RustConnection, atoms: &Atoms, window: Window) -> Option<u32> {
    let reply = conn.get_property(false, window, atoms.aware, AtomEnum::ATOM, 0, 1).ok()?.reply().ok()?;
    (reply.type_ != NONE).then(|| reply.value32().and_then(|mut v| v.next()).unwrap_or(0))
}

fn proxy_of(conn: &RustConnection, atoms: &Atoms, window: Window) -> Option<Window> {
    let reply = conn.get_property(false, window, atoms.proxy, AtomEnum::WINDOW, 0, 1).ok()?.reply().ok()?;
    if reply.type_ == NONE { None } else { reply.value32().and_then(|mut v| v.next()) }
}

fn viewable(conn: &RustConnection, window: Window) -> bool {
    conn.get_window_attributes(window)
        .ok()
        .and_then(|cookie| cookie.reply().ok())
        .is_some_and(|a| a.map_state == MapState::VIEWABLE)
}

/// Every viewable XdndAware window below `top`, depth first, with its depth below `top`.
fn aware_descendants(conn: &RustConnection, atoms: &Atoms, top: Window, depth: usize, out: &mut Vec<(Window, usize)>) {
    for child in conn.query_tree(top).unwrap().reply().unwrap().children {
        if !viewable(conn, child) {
            continue;
        }
        if aware_version(conn, atoms, child).is_some() {
            out.push((child, depth + 1));
        }
        aware_descendants(conn, atoms, child, depth + 1, out);
    }
}

fn describe(conn: &RustConnection, atoms: &Atoms, window: Window) -> String {
    let g = conn.get_geometry(window).unwrap().reply().unwrap();
    format!(
        "{window:#x} {:?} {}x{}, XdndAware {}, XdndProxy {}",
        window_name(conn, window),
        g.width,
        g.height,
        aware_version(conn, atoms, window).map_or("-".to_owned(), |v| v.to_string()),
        proxy_of(conn, atoms, window).map_or("-".to_owned(), |w| format!("{w:#x}")),
    )
}

fn reply_text(conn: &RustConnection, reply: &Option<(Window, Reply)>) -> String {
    let name = |atom: Atom| {
        conn.get_atom_name(atom)
            .ok()
            .and_then(|cookie| cookie.reply().ok())
            .map(|r| String::from_utf8_lossy(&r.name).into_owned())
            .unwrap_or_else(|| format!("{atom}"))
    };
    match reply {
        None => "no answer in 3 s".to_owned(),
        Some((from, Reply::Status { accept, action })) => {
            format!("XdndStatus from {from:#x}: accept {accept}, action {}", name(*action))
        }
        Some((from, Reply::Finished { accepted, action })) => {
            format!("XdndFinished from {from:#x}: accepted {accepted}, action {}", name(*action))
        }
    }
}

/// One drag of `files` onto `target` at a root point: enter, position, then drop if the target accepted, else leave.
fn drop_once(source: &mut Source, target: Window, x: i16, y: i16, files: &[PathBuf]) -> String {
    source.requestors.clear();
    let owned = source.start_drag(files);
    source.enter(target, [source.atoms.uri_list, NONE, NONE]);
    source.position(target, x, y);
    let status = source.next_reply(3000);
    let accepted = matches!(status, Some((_, Reply::Status { accept: true, .. })));
    let finished = if accepted {
        source.send_drop(target);
        Some(source.next_finished(3000))
    } else {
        source.leave(target);
        None
    };
    let requestors: Vec<String> = source.requestors.iter().map(|w| format!("{w:#x}")).collect();
    format!(
        "selection owned {owned}; {}; data requested by [{}]; {}",
        reply_text(&source.conn, &status),
        requestors.join(", "),
        match &finished {
            Some((f, 0)) => reply_text(&source.conn, f),
            Some((f, n)) => format!("{n} more XdndStatus, then {}", reply_text(&source.conn, f)),
            None => "not accepted, so XdndLeave instead of XdndDrop".to_owned(),
        }
    )
}

/// A short WAV file of silence (48 kHz, 16-bit, mono, 0.1 s), so that a host that imports what is dropped can.
fn write_silence(path: &Path) {
    let frames: u32 = 4800;
    let data_len = frames * 2;
    let mut wav = Vec::with_capacity(44 + data_len as usize);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_len).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
    wav.extend_from_slice(&1u16.to_le_bytes()); // mono
    wav.extend_from_slice(&48_000u32.to_le_bytes());
    wav.extend_from_slice(&96_000u32.to_le_bytes()); // bytes per second
    wav.extend_from_slice(&2u16.to_le_bytes()); // block align
    wav.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    wav.resize(44 + data_len as usize, 0);
    std::fs::write(path, wav).unwrap();
}

/// `--into <title>`: no editor of its own. Finds the toplevel whose WM_NAME contains `title` (a host's plugin window),
/// reports every XdndAware window in it and the plugin window's ancestors, then drops a WAV file at the plugin
/// window's centre twice: on the window a Qt-style source picks (the outermost XdndAware window under the point), and
/// on the plugin window directly. Prints what it saw; exit 0 when it ran, 2 when the window is missing.
fn run_into(title: &str) -> i32 {
    let mut source = Source::new();
    let root = source.root;
    let tops: Vec<Window> = {
        let conn = &source.conn;
        conn.query_tree(root)
            .unwrap()
            .reply()
            .unwrap()
            .children
            .into_iter()
            .filter(|&w| viewable(conn, w) && window_name(conn, w).contains(title))
            .collect()
    };
    // The last one is the topmost in the stacking order.
    let Some(&top) = tops.last() else {
        eprintln!("drop-probe --into: no viewable toplevel window has {title:?} in its name");
        return 2;
    };
    let (conn, atoms) = (&source.conn, &source.atoms);
    println!("toplevel: {}", describe(conn, atoms, top));
    let mut aware = Vec::new();
    aware_descendants(conn, atoms, top, 0, &mut aware);
    for &(window, depth) in &aware {
        println!("XdndAware below it at depth {depth}: {}", describe(conn, atoms, window));
    }
    // The plugin's window: the deepest XdndAware window below the toplevel (baseview marks its own window).
    let Some(&(plugin, _)) = aware.iter().max_by_key(|(_, depth)| *depth) else {
        eprintln!("drop-probe --into: no XdndAware window below the toplevel");
        return 2;
    };
    println!("the plugin window and its ancestors, innermost first:");
    let mut window = plugin;
    loop {
        println!("  {}", describe(conn, atoms, window));
        let parent = conn.query_tree(window).unwrap().reply().unwrap().parent;
        if parent == root || parent == NONE {
            break;
        }
        window = parent;
    }
    let g = conn.get_geometry(plugin).unwrap().reply().unwrap();
    let at = conn
        .translate_coordinates(plugin, root, (g.width / 2) as i16, (g.height / 2) as i16)
        .unwrap()
        .reply()
        .unwrap();
    let (x, y) = (at.dst_x, at.dst_y);
    let qt = source.find_target(x, y);
    // GTK 3 asks only the toplevel under the pointer (its client window; there is no window manager here, so that is
    // the toplevel itself), through its XdndProxy if it has one.
    let under = conn.translate_coordinates(root, root, x, y).unwrap().reply().unwrap().child;
    let gtk = proxy_of(conn, atoms, under).unwrap_or(under);
    println!(
        "drop point ({x}, {y}) on the root: a Qt-style source drops on {}, a GTK-style source on {gtk:#x} (XdndAware {})",
        qt.map_or("nothing".to_owned(), |(w, v)| format!("{w:#x} (XdndAware {v})")),
        aware_version(conn, atoms, gtk).map_or("-".to_owned(), |v| v.to_string()),
    );

    let dir = std::env::temp_dir().join(format!("drop-probe-into-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.canonicalize().unwrap().join("drop-probe-silence.wav");
    write_silence(&file);
    let files = [file];
    match qt {
        Some((target, _)) => {
            println!("drop on the Qt-style target {target:#x}: {}", drop_once(&mut source, target, x, y, &files))
        }
        None => println!("drop on the Qt-style target: there is none"),
    }
    thread::sleep(Duration::from_millis(500));
    println!("drop on the plugin window {plugin:#x} directly: {}", drop_once(&mut source, plugin, x, y, &files));
    thread::sleep(Duration::from_millis(500));
    let _ = std::fs::remove_dir_all(&dir);
    0
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Err(e) = require_xvfb() {
        eprintln!("drop-probe: {e}. Run: xvfb-run -a -s \"-screen 0 1920x1080x24\" drop-probe");
        std::process::exit(2);
    }
    if args.first().is_some_and(|a| a == "--into") {
        let Some(title) = args.get(1).filter(|_| args.len() == 2) else {
            eprintln!("usage: drop-probe --into <window title>");
            std::process::exit(2);
        };
        std::process::exit(run_into(title));
    }
    let reparent = args.iter().any(|a| a == "--reparent");
    if let Some(other) = args.iter().find(|a| *a != "--reparent") {
        eprintln!("usage: drop-probe [--reparent] | --into <window title>, not {other:?}");
        std::process::exit(2);
    }

    // The files to drop: one plain name, one that needs percent-encoding (a space and a non-ASCII letter).
    let dir = std::env::temp_dir().join(format!("drop-probe-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let dir = dir.canonicalize().unwrap();
    let files = [dir.join("kick.wav"), dir.join("drop probe \u{e9}.wav")];
    let sizes = [44usize, 1000];
    for (file, size) in files.iter().zip(sizes) {
        std::fs::write(file, vec![0u8; size]).unwrap();
    }

    // The host: a toplevel that is not XDND-aware, away from the root's origin.
    let (conn, screen_num) = RustConnection::connect(None).unwrap();
    let screen = conn.setup().roots[screen_num].clone();
    let host = conn.generate_id().unwrap();
    conn.create_window(
        screen.root_depth,
        host,
        screen.root,
        200,
        150,
        400,
        300,
        0,
        WindowClass::INPUT_OUTPUT,
        0,
        &CreateWindowAux::new().background_pixel(0x00ff00ff),
    )
    .unwrap();
    conn.map_window(host).unwrap();
    conn.flush().unwrap();

    // The editor, embedded in the host window.
    let shared = Arc::new(Mutex::new(State::default()));
    let notifier = RepaintNotifier::new();
    let parent = Parent(host);
    let settings = EguiWindowSettings::new()
        .with_title("drop-probe")
        .with_size(Size::Logical(LogicalSize { width: 300.0, height: 200.0 }))
        .with_resizable(false)
        .with_repaint_notifier(notifier.clone());
    // A CLAP host creates the editor first and gives it its parent afterwards.
    let settings = if reparent { settings.with_wait_for_parent(true) } else { settings.with_parent(&parent) };
    let window = EguiWindow::create(settings, Probe { shared: Arc::clone(&shared) }).expect("create the editor window");
    if reparent {
        window.set_parent(baseview::ParentWindowHandle::from_window(&parent)).expect("set the parent window");
    }
    window.show().expect("show the editor window");

    let snapshot = || shared.lock().unwrap().clone();
    if !wait(5000, || snapshot().frames >= 2) {
        eprintln!("drop-probe: the editor drew no frame in 5 s");
        std::process::exit(2);
    }
    let editor = *conn.query_tree(host).unwrap().reply().unwrap().children.last().expect("a child window");
    // The drop point: the middle of the editor, in root coordinates.
    let at = conn.translate_coordinates(editor, screen.root, 150, 100).unwrap().reply().unwrap();
    let (x, y) = (at.dst_x, at.dst_y);

    let mut source = Source::new();
    let copy = source.atoms.action_copy;
    let mut c = Checks { list: vec![] };
    println!(
        "{}; host window {host:#x} (not XDND-aware), editor window {editor:#x}, drop point ({x}, {y}) on the root",
        if reparent { "created without a parent, reparented (the CLAP way)" } else { "created inside the host window" }
    );

    // ---------------------------------------------------------------- 1. two files, dropped
    let target = source.find_target(x, y);
    c.record(
        Side::X11,
        "the source finds the editor as the drop target (outermost XdndAware window under the point)",
        target.is_some_and(|(w, v)| w == editor && v == 5) && !source.has_proxy(editor),
        format!("found {target:x?}, editor {editor:#x}, version 5 expected, no XdndProxy"),
    );
    let target = editor;

    // The editor is idle: no frame is pending before the drag starts.
    thread::sleep(Duration::from_millis(400));
    let idle_before = snapshot().frames;
    thread::sleep(Duration::from_millis(300));
    let idle = snapshot().frames == idle_before;
    println!("idle before the drag: {idle} ({} frames drawn so far)", snapshot().frames);

    let owned = source.start_drag(&files);
    let frames_before = snapshot().frames;
    source.enter(target, [source.atoms.uri_list, NONE, NONE]);
    source.position(target, x, y);
    let reply = source.next_reply(3000);
    c.record(
        Side::X11,
        "XdndEnter + XdndPosition: the editor fetches the list and accepts the drag as a copy",
        owned && reply == Some((target, Reply::Status { accept: true, action: copy })) && source.conversions == 1,
        format!("selection owned {owned}, reply {reply:?}, conversions {}", source.conversions),
    );

    let drew = wait(1000, || snapshot().frames > frames_before);
    c.record(
        Side::Egui,
        "the drag event alone draws a frame (no other repaint was asked for)",
        idle && drew,
        format!("idle before {idle}, frames {frames_before} -> {}", snapshot().frames),
    );

    // From here on the probe asks for frames itself, so a missing file is missing from egui's input, not a frame that
    // never came.
    notifier.request_repaint();
    let hovered = wait(2000, || snapshot().hovered == files);
    let s = snapshot();
    c.record(
        Side::Egui,
        "while the files hover: hovered_files lists both, in order",
        hovered,
        format!(
            "hovered_files {:?}, expected {:?} ({} frames drawn)",
            names(&s.hovered),
            names(&files),
            s.frames
        ),
    );
    c.record(
        Side::Egui,
        "while the files hover: nothing is dropped",
        s.dropped.is_empty(),
        format!("dropped_files so far {:?}", names(&s.dropped)),
    );

    source.position(target, x + 20, y + 10);
    let reply = source.next_reply(3000);
    c.record(
        Side::X11,
        "a move: XdndStatus still accepts the copy",
        reply == Some((target, Reply::Status { accept: true, action: copy })),
        format!("reply {reply:?}"),
    );

    source.send_drop(target);
    let reply = source.next_reply(3000);
    c.record(
        Side::X11,
        "XdndDrop: XdndFinished, accepted as a copy",
        reply == Some((target, Reply::Finished { accepted: true, action: copy })),
        format!("reply {reply:?}"),
    );

    notifier.request_repaint();
    let dropped = wait(2000, || snapshot().dropped == files);
    let s = snapshot();
    c.record(
        Side::Egui,
        "after the drop: dropped_files lists both files, in one frame",
        dropped && s.drop_frames == 1,
        format!("dropped_files {:?} in {} frame(s) ({} frames drawn)", names(&s.dropped), s.drop_frames, s.frames),
    );
    let expected_bytes: Vec<Result<usize, String>> = sizes.iter().map(|&n| Ok(n)).collect();
    c.record(
        Side::Egui,
        "after the drop: DroppedFile::bytes() reads each file",
        s.dropped_bytes == expected_bytes,
        format!("lengths {:?}, expected {:?}", s.dropped_bytes, expected_bytes),
    );
    c.record(
        Side::Egui,
        "after the drop: hovered_files is empty",
        wait(1000, || snapshot().hovered.is_empty()),
        format!("hovered_files {:?}", names(&snapshot().hovered)),
    );

    // ---------------------------------------------------------------- 2. one file, then XdndLeave
    let owned = source.start_drag(&files[..1]);
    source.enter(target, [source.atoms.uri_list, NONE, NONE]);
    source.position(target, x, y);
    let reply = source.next_reply(3000);
    c.record(
        Side::X11,
        "a second drag: accepted as a copy",
        owned && reply == Some((target, Reply::Status { accept: true, action: copy })),
        format!("selection owned {owned}, reply {reply:?}"),
    );
    notifier.request_repaint();
    let hovered = wait(2000, || snapshot().hovered == files[..1]);
    c.record(
        Side::Egui,
        "the second drag: hovered_files lists its one file",
        hovered,
        format!("hovered_files {:?}", names(&snapshot().hovered)),
    );
    let dropped_before = snapshot().dropped.len();
    source.leave(target);
    notifier.request_repaint();
    let cleared = wait(2000, || snapshot().hovered.is_empty());
    thread::sleep(Duration::from_millis(200));
    let s = snapshot();
    c.record(
        Side::Egui,
        "XdndLeave: hovered_files is empty again and nothing more is dropped",
        cleared && s.dropped.len() == dropped_before,
        format!("hovered_files {:?}, dropped {} -> {}", names(&s.hovered), dropped_before, s.dropped.len()),
    );

    // ---------------------------------------------------------------- 3. a drag of text, not files
    let owned = source.start_drag(&[]);
    source.enter(target, [source.atoms.text_plain, NONE, NONE]);
    source.position(target, x, y);
    let reply = source.next_reply(3000);
    c.record(
        Side::X11,
        "a drag of text/plain only: XdndStatus refuses it",
        owned && matches!(reply, Some((w, Reply::Status { accept: false, .. })) if w == target),
        format!("reply {reply:?}"),
    );
    notifier.request_repaint();
    thread::sleep(Duration::from_millis(300));
    let s = snapshot();
    c.record(
        Side::Egui,
        "a drag of text: no hovered file and nothing dropped",
        s.hovered.is_empty() && s.dropped.len() == dropped_before,
        format!("hovered_files {:?}, dropped {}", names(&s.hovered), s.dropped.len()),
    );
    source.leave(target);

    window.close();
    thread::sleep(Duration::from_millis(300));
    let _ = std::fs::remove_dir_all(&dir);

    let (x11_failed, egui_failed) = (c.failed(Side::X11), c.failed(Side::Egui));
    println!(
        "{} of {} checks failed ([x11] {x11_failed}, [egui] {egui_failed})",
        x11_failed + egui_failed,
        c.list.len()
    );
    let status = if x11_failed > 0 {
        3
    } else if egui_failed > 0 {
        1
    } else {
        0
    };
    std::process::exit(status);
}
