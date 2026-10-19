//! CPU time per frame of the full reference panel, headless: egui's pass (layout, interaction)
//! plus tessellation, as the GUI thread does before handing triangles to the GPU. No GL and no
//! GPU, so it measures exactly the part that doesn't depend on the renderer.
//!
//! `cargo run -p spike-ui --release --example frame_bench [frames]`
//!
//! - idle: frames with nothing changing (what a mouse move costs while the panel is idle; a truly
//!   idle panel runs no frames at all);
//! - animating: every frame gets a new 60 Hz block of audio (scope, spectrum) and a new LFO
//!   phase (8 rings, 3 tiles).
#![allow(clippy::print_stdout, reason = "a benchmark that prints its results")]

use std::sync::Arc;
use std::time::{Duration, Instant};

use egui::{Pos2, RawInput, Rect, ViewportId, ViewportInfo};
use enth_ui::Theme;
use spike_ui::{
    AudioFeed, DEMO_SAMPLE_RATE, DemoChord, LocalParams, REFERENCE_SIZE, ReferencePanel, layout,
};

const FPS: f32 = 60.0;
const WARMUP: usize = 30;

fn input(frame: usize, pixels_per_point: f32) -> RawInput {
    let mut raw = RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, REFERENCE_SIZE)),
        time: Some(frame as f64 / f64::from(FPS)),
        ..Default::default()
    };
    raw.viewports.insert(
        ViewportId::ROOT,
        ViewportInfo {
            native_pixels_per_point: Some(pixels_per_point),
            ..Default::default()
        },
    );
    raw
}

fn publish(feed: &AudioFeed, chord: &mut DemoChord, frame: usize) {
    let block = (DEMO_SAMPLE_RATE / FPS) as usize;
    let mut writer = feed.writer();
    for _ in 0..block {
        writer.push(chord.next_sample());
    }
    let phase = (frame as f32 * layout::LFO1_HZ / FPS).fract();
    writer.finish(phase, true);
}

struct Stats {
    p50: f64,
    p95: f64,
    p99: f64,
    max: f64,
    mean: f64,
}

fn stats(mut times: Vec<Duration>) -> Stats {
    times.sort_unstable();
    let ms = |d: Duration| d.as_secs_f64() * 1000.0;
    let at = |q: f64| ms(times[((times.len() - 1) as f64 * q).round() as usize]);
    Stats {
        p50: at(0.5),
        p95: at(0.95),
        p99: at(0.99),
        max: ms(*times.last().unwrap_or(&Duration::ZERO)),
        mean: times.iter().map(|&d| ms(d)).sum::<f64>() / times.len() as f64,
    }
}

fn run(theme: &Theme, animating: bool, pixels_per_point: f32, frames: usize) -> Stats {
    let ctx = egui::Context::default();
    theme.apply(&ctx);
    let feed = Arc::new(AudioFeed::new());
    let mut panel = ReferencePanel::new(theme.clone(), Arc::clone(&feed));
    let mut params = LocalParams::default();
    let mut chord = DemoChord::default();
    let mut times = Vec::with_capacity(frames);
    for frame in 0..WARMUP + frames {
        if animating {
            publish(&feed, &mut chord, frame);
        }
        let started = Instant::now();
        let output = ctx.run_ui(input(frame, pixels_per_point), |ui| {
            panel.ui(ui, &mut params);
        });
        let primitives = ctx.tessellate(output.shapes, output.pixels_per_point);
        let elapsed = started.elapsed();
        std::hint::black_box(primitives);
        let mut textures = output.textures_delta;
        textures.clear();
        if frame >= WARMUP {
            times.push(elapsed);
        }
    }
    stats(times)
}

fn main() {
    let frames: usize = std::env::args()
        .nth(1)
        .and_then(|a| a.parse().ok())
        .unwrap_or(600);
    let theme = Theme::dark();
    println!("frames per case: {frames} (after {WARMUP} warm-up), window {REFERENCE_SIZE:?}");
    println!("case,scale,p50_ms,p95_ms,p99_ms,max_ms,mean_ms,cpu_pct_at_30fps,cpu_pct_at_60fps");
    for pixels_per_point in [1.0, 2.0] {
        for (name, animating) in [("idle", false), ("animating", true)] {
            let s = run(&theme, animating, pixels_per_point, frames);
            println!(
                "{name},{pixels_per_point},{:.3},{:.3},{:.3},{:.3},{:.3},{:.1},{:.1}",
                s.p50,
                s.p95,
                s.p99,
                s.max,
                s.mean,
                s.mean * 30.0 / 10.0,
                s.mean * 60.0 / 10.0,
            );
        }
    }
}
