//! Standalone dev window for the reference panel (eframe, glow). For the owner to run on the
//! real display; agents never launch it (it opens a GL window).
//!
//! ```text
//! cargo run -p spike-ui --release                      # the panel, demo audio off
//! cargo run -p spike-ui --release -- --gallery         # the token gallery
//! cargo run -p spike-ui --release -- --theme my.json   # tokens from a file
//! cargo run -p spike-ui --release -- --dump-theme      # print the default tokens as JSON
//! ```
//!
//! "Audio on" in the header starts a fake audio thread (a chord through the feed at 48 kHz in
//! 256-sample blocks), so scopes and modulation rings animate; "Audio off" lets the panel go idle.
#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "a developer tool with a command line"
)]

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use enth_ui::Theme;
use enth_ui::gallery::{GALLERY_SIZE, GalleryState, token_gallery};
use spike_ui::{AudioFeed, LocalParams, REFERENCE_SIZE, ReferencePanel};

const SAMPLE_RATE: f32 = 48_000.0;
const BLOCK: usize = 256;
/// LFO 1 rate of the fake audio thread.
const LFO_HZ: f32 = 0.4;

enum Page {
    Panel(Box<ReferencePanel>, LocalParams),
    Gallery(Box<Theme>, GalleryState),
}

struct DevApp {
    page: Page,
}

impl eframe::App for DevApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        match &mut self.page {
            Page::Panel(panel, params) => {
                panel.ui(ui, params);
            }
            Page::Gallery(theme, state) => {
                egui::ScrollArea::vertical().show(ui, |ui| token_gallery(ui, theme, state));
            }
        }
    }
}

/// The fake audio thread: a three-note saw chord while `transport` is on, silence otherwise.
fn spawn_fake_audio(feed: Arc<AudioFeed>, transport: Arc<AtomicBool>, ctx: egui::Context) {
    std::thread::Builder::new()
        .name("fake-audio".to_owned())
        .spawn(move || {
            let notes = [220.0_f32, 277.18, 329.63];
            let mut phases = [0.0_f32; 3];
            let mut lfo = 0.0_f32;
            loop {
                let on = transport.load(Ordering::Relaxed);
                let mut writer = feed.writer();
                for _ in 0..BLOCK {
                    let mut sample = 0.0;
                    for (phase, hz) in phases.iter_mut().zip(notes) {
                        if on {
                            sample += (*phase * 2.0 - 1.0) * 0.2;
                        }
                        *phase = (*phase + hz / SAMPLE_RATE).fract();
                    }
                    writer.push(sample);
                }
                lfo = (lfo + LFO_HZ * BLOCK as f32 / SAMPLE_RATE).fract();
                if writer.finish(lfo, on) {
                    ctx.request_repaint();
                }
                std::thread::sleep(Duration::from_secs_f32(BLOCK as f32 / SAMPLE_RATE));
            }
        })
        .expect("cannot start the fake audio thread");
}

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let theme = match args.iter().position(|a| a == "--theme") {
        Some(i) => {
            let path = args.get(i + 1).expect("--theme needs a file");
            let text = std::fs::read_to_string(path).expect("cannot read the theme file");
            serde_json::from_str(&text).expect("not a theme file (see --dump-theme)")
        }
        None => Theme::dark(),
    };
    if args.iter().any(|a| a == "--dump-theme") {
        println!(
            "{}",
            serde_json::to_string_pretty(&theme).expect("theme serialises")
        );
        return Ok(());
    }
    let gallery = args.iter().any(|a| a == "--gallery");
    let size = if gallery {
        GALLERY_SIZE
    } else {
        REFERENCE_SIZE
    };
    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Glow,
        viewport: egui::ViewportBuilder::default()
            .with_title("enth spike-ui")
            .with_inner_size(size)
            .with_min_inner_size(theme.size.window.min.min(size)),
        ..Default::default()
    };
    eframe::run_native(
        "enth spike-ui",
        options,
        Box::new(move |cc| {
            theme.apply(&cc.egui_ctx);
            let page = if gallery {
                Page::Gallery(Box::new(theme), GalleryState::default())
            } else {
                let feed = Arc::new(AudioFeed::new());
                let transport = Arc::new(AtomicBool::new(false));
                spawn_fake_audio(
                    Arc::clone(&feed),
                    Arc::clone(&transport),
                    cc.egui_ctx.clone(),
                );
                let panel = ReferencePanel::new(theme, feed).with_transport(transport);
                Page::Panel(Box::new(panel), LocalParams::default())
            };
            Ok(Box::new(DevApp { page }))
        }),
    )
}
