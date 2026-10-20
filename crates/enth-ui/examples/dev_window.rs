//! The editor dev window (lane ED1, docs/editor.md): the real editor over a real `Session` and a real engine, with no
//! audio output. The engine runs on its own thread at real-time pace into a buffer nobody hears, so plans swap, the
//! return ring drains and telemetry flows as in a host. It opens a window: run it under Xvfb, or on the owner's
//! display with work saved (AGENTS.md: never from an agent on the real display).
//!
//! ```text
//! xvfb-run -a -s "-screen 0 1920x1080x24" \
//!   cargo run -p enth-ui --example dev_window --features dev-window -- [options]
//! ```
//!
//! | Option | Does |
//! | --- | --- |
//! | `--patch FILE` | start from a `.patch.json` (default: the init patch) |
//! | `--layout NAME_OR_FILE` | a built-in layout's name or a layout file; `ENTH_LAYOUT` says the same |
//! | `--theme FILE` | a theme file (the serde form `--dump-theme` prints); `ENTH_THEME` says the same |
//! | `--frames N` | quit after N frames (a smoke test; prints what the last frame reported) |
//! | `--dump-layout NAME` | print a built-in layout's JSON (a start for a variant) and exit |
//! | `--dump-theme` | print the dark theme's JSON and exit |
//!
//! Keys: F2 switches to the next built-in layout, F5 reloads the layout and theme files, F3 holds or releases an
//! audition note (middle C), so the telemetry rings, tiles, scope and meters move. The status line names the layout.
//! Switching needs no rebuild: edit the file, press F5. The engine thread turns the tap's wake flag into a repaint,
//! as the plugin's `process()` does, so the window draws only on input or while visible live data changes.

#![allow(
    clippy::disallowed_methods,
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "a dev tool: it reads files and the environment, prints, reads the clock and runs the engine on its own \
              thread, which the UI crate's library code never does"
)]

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use eframe::egui;
use enth_patch::Patch;
use enth_session::{Activation, Session};
use enth_ui::Theme;
use enth_ui::editor::Editor;
use enth_ui::editor::layout::{BUILTIN, LayoutSpec};

/// The engine's sample rate and block size: a common host setting.
const SAMPLE_RATE: f32 = 48_000.0;
const BLOCK: usize = 256;

struct Options {
    patch: Option<PathBuf>,
    layout: Option<String>,
    theme: Option<PathBuf>,
    frames: Option<u64>,
}

/// What the command line asks for.
enum Command {
    Run(Options),
    /// Print this and exit (`--dump-layout`, `--dump-theme`).
    Print(String),
}

fn parse_args() -> Result<Command, String> {
    let mut options = Options {
        patch: None,
        layout: std::env::var("ENTH_LAYOUT").ok(),
        theme: std::env::var_os("ENTH_THEME").map(PathBuf::from),
        frames: None,
    };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut value = || args.next().ok_or_else(|| format!("{arg} needs a value"));
        match arg.as_str() {
            "--patch" => options.patch = Some(PathBuf::from(value()?)),
            "--layout" => options.layout = Some(value()?),
            "--theme" => options.theme = Some(PathBuf::from(value()?)),
            "--frames" => {
                options.frames = Some(value()?.parse().map_err(|e| format!("--frames: {e}"))?);
            }
            "--dump-layout" => {
                let name = value()?;
                let spec = LayoutSpec::builtin(&name)
                    .ok_or_else(|| format!("no built-in layout {name:?}"))?;
                return Ok(Command::Print(spec.to_json()));
            }
            "--dump-theme" => {
                let text =
                    serde_json::to_string_pretty(&Theme::dark()).map_err(|e| e.to_string())?;
                return Ok(Command::Print(text));
            }
            other => return Err(format!("unknown option {other}; see the file's header")),
        }
    }
    Ok(Command::Run(options))
}

/// A layout by built-in name, else from a file.
fn load_layout(source: &str) -> Result<LayoutSpec, String> {
    if let Some(spec) = LayoutSpec::builtin(source) {
        return Ok(spec);
    }
    let text = std::fs::read_to_string(source).map_err(|e| format!("cannot read {source}: {e}"))?;
    LayoutSpec::from_json(&text).map_err(|e| format!("{source}: {e}"))
}

fn load_theme(path: &PathBuf) -> Result<Theme, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
}

struct DevApp {
    theme: Theme,
    theme_file: Option<PathBuf>,
    editor: Editor,
    session: Session,
    layout_source: String,
    frames: u64,
    quit_after: Option<u64>,
    running: Arc<AtomicBool>,
    /// An audition note is held (F3).
    holding: bool,
}

impl DevApp {
    /// Apply the layout and theme sources; problems go to the status line.
    fn reload(&mut self, ctx: &egui::Context) {
        let mut notes = Vec::new();
        if let Some(path) = &self.theme_file {
            match load_theme(path) {
                Ok(theme) => self.theme = theme,
                Err(e) => notes.push(e),
            }
        }
        self.theme.apply(ctx);
        match load_layout(&self.layout_source).and_then(|spec| {
            self.editor
                .set_layout(spec, &self.theme)
                .map_err(|e| e.to_string())
        }) {
            Ok(()) => notes.insert(0, format!("layout {}", self.editor.layout().name())),
            Err(e) => notes.insert(0, format!("{e}; still {}", self.editor.layout().name())),
        }
        self.editor.set_notice(notes.join(" · "));
    }
}

impl eframe::App for DevApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if ctx.input(|i| i.key_pressed(egui::Key::F2)) {
            let at = BUILTIN
                .iter()
                .position(|(name, _)| *name == self.editor.layout().name())
                .map_or(0, |i| (i + 1) % BUILTIN.len());
            BUILTIN[at].0.clone_into(&mut self.layout_source);
            self.reload(&ctx);
        }
        if ctx.input(|i| i.key_pressed(egui::Key::F5)) {
            self.reload(&ctx);
        }
        if ctx.input(|i| i.key_pressed(egui::Key::F3)) {
            self.holding = if self.holding {
                let _ = self.session.audition_all_off();
                false
            } else {
                self.session.audition_on(60, 0.8).is_ok()
            };
        }
        let down = ctx.input(|i| i.pointer.any_down());
        self.session.pump();
        self.editor.frame(ui, &self.theme, &mut self.session, down);
        // No host: the requests (host writes, renames) have nowhere to go.
        self.session.drain_requests().for_each(drop);
        self.frames += 1;
        if let Some(n) = self.quit_after {
            // A smoke test keeps drawing until it has its frames.
            ctx.request_repaint();
            if self.frames >= n {
                let report = self.editor.report();
                println!(
                    "dev_window: {} frames, layout {}, {} regions, overflow {:?}, repaint {:?}",
                    self.frames,
                    self.editor.layout().name(),
                    report.regions.len(),
                    report.overflow,
                    report.repaint
                );
                self.running.store(false, Ordering::Release);
                self.quit_after = None;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }
}

fn main() -> Result<(), String> {
    let options = match parse_args()? {
        Command::Run(options) => options,
        Command::Print(text) => {
            println!("{text}");
            return Ok(());
        }
    };
    let mut session = Session::new(Patch::init());
    if let Some(path) = &options.patch {
        // The way an editor preset load goes: Session::load_patch, never the host's state calls.
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        let report = session.load_patch(&text);
        if report.fell_back {
            return Err(format!("{}: {:?}", path.display(), report.errors));
        }
    }
    let mut engine = session
        .activate(Activation {
            sample_rate: SAMPLE_RATE,
            offline: false,
        })
        .map_err(|e| format!("activate: {e:?}"))?;
    // The window's context, for the engine thread's wake-up repaints (set once the window exists).
    let window: Arc<OnceLock<egui::Context>> = Arc::new(OnceLock::new());
    // The audio thread: the engine renders block after block at real-time pace into buffers nobody hears.
    let running = Arc::new(AtomicBool::new(true));
    let audio = {
        let running = Arc::clone(&running);
        let window = Arc::clone(&window);
        std::thread::Builder::new()
            .name("enth-dev-audio".to_owned())
            .spawn(move || {
                let (mut left, mut right) = (vec![0.0; BLOCK], vec![0.0; BLOCK]);
                let period = Duration::from_secs_f64(BLOCK as f64 / f64::from(SAMPLE_RATE));
                let mut next = Instant::now();
                while running.load(Ordering::Acquire) {
                    engine.process(&mut left, &mut right, &[]);
                    // As the plugin's process(): new telemetry for a sleeping editor is a repaint.
                    if engine.tap().is_some_and(|tap| tap.take_wake())
                        && let Some(ctx) = window.get()
                    {
                        ctx.request_repaint();
                    }
                    next += period;
                    if let Some(wait) = next.checked_duration_since(Instant::now()) {
                        std::thread::sleep(wait);
                    } else {
                        next = Instant::now();
                    }
                }
            })
            .map_err(|e| format!("audio thread: {e}"))?
    };
    let theme = Theme::dark();
    let size = theme.size.window.default;
    let min = theme.size.window.min;
    let native = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("enth editor (dev window, no audio)")
            .with_inner_size(size)
            .with_min_inner_size(min),
        ..Default::default()
    };
    let layout_source = options.layout.clone().unwrap_or_else(|| "lanes".to_owned());
    let app_running = Arc::clone(&running);
    let result = eframe::run_native(
        "enth-dev-window",
        native,
        Box::new(move |cc| {
            let _ = window.set(cc.egui_ctx.clone());
            let mut app = DevApp {
                editor: Editor::new(&theme),
                theme,
                theme_file: options.theme,
                session,
                layout_source,
                frames: 0,
                quit_after: options.frames,
                running: app_running,
                holding: false,
            };
            app.reload(&cc.egui_ctx);
            app.editor.open(&app.session, &app.theme);
            Ok(Box::new(app))
        }),
    );
    running.store(false, Ordering::Release);
    let _ = audio.join();
    result.map_err(|e| format!("window: {e}"))
}
