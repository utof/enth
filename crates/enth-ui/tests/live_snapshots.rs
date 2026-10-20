//! Snapshots of the live feedback (lane ED2), from synthetic telemetry, rendered on lavapipe only (tests/common): a
//! filter card whose cutoff is modulated (range arc, live arc, depth handle), the source tiles with their levels, and
//! the output region as a scope and as a spectrum, with the meters. Two PNGs; the behaviour is asserted in
//! `tests/modulation.rs` and `tests/repaint.rs`, never on these pixels.
//!
//! Update after a deliberate change, then look at the PNG:
//! `UPDATE_SNAPSHOTS=1 cargo test -p enth-ui --test live_snapshots`.

mod common;

use egui::{Rect, pos2, vec2};
use enth_patch::{ModSource, ModuleId, modulation_range};
use enth_ui::Theme;
use enth_ui::editor::cards::view::CardView;
use enth_ui::editor::cards::{Card, Intents};
use enth_ui::editor::live::Fingerprint;
use enth_ui::editor::regions::{Output, OutputState, OutputView, SourceTiles, SourceView};

/// What the snapshot draws: a card, tiles and two output regions, with fixed live values.
struct Scene {
    card: CardView,
    sources: Vec<SourceView>,
    scope: Vec<f32>,
    scope_state: OutputState,
    spectrum_state: OutputState,
    /// Draw the spectrum instead of the scope.
    spectrum: bool,
}

fn scene() -> Scene {
    let patch = common::patches::p1_default();
    let id = ModuleId::new("filter1").unwrap();
    let (_, m) = patch.find(&id).unwrap();
    let mut card = CardView::module(&patch, m);
    for p in &mut card.params {
        if let Some(r) = modulation_range(&patch, &p.addr).filter(|r| r.is_modulated()) {
            p.range = Some((r.lo, r.hi));
            // The envelope is a third of the way up: the live arc runs from the value towards the range's top.
            p.live = Some(r.base + (r.hi - r.base) / 3.0);
        }
    }
    let mut sources = SourceView::all(&patch);
    let levels = [1.0, 0.6, 0.85, 0.25];
    for (s, level) in sources.iter_mut().zip(levels) {
        s.level = Some(level);
    }
    if let Some(wheel) = sources.iter_mut().find(|s| s.src == ModSource::ModWheel) {
        wheel.level = Some(0.4);
    }
    // A saw-ish note with its second harmonic: 1,024 samples of a 220 Hz tone at 48 kHz.
    let scope = (0..1024)
        .map(|i| {
            let x = std::f32::consts::TAU * 220.0 * i as f32 / 48_000.0;
            0.6 * x.sin() + 0.25 * (2.0 * x).sin()
        })
        .collect();
    let mut spectrum_state = OutputState::default();
    spectrum_state.spectrum = true;
    Scene {
        card,
        sources,
        scope,
        scope_state: OutputState::default(),
        spectrum_state,
        spectrum: false,
    }
}

#[test]
fn live_feedback_snapshot() {
    let theme = Theme::dark();
    let t = theme.clone();
    let size = vec2(720.0, 360.0);
    let mut h = common::harness(
        &theme,
        size,
        move |ui, s: &mut Scene| {
            let rect = ui.max_rect();
            ui.painter()
                .rect_filled(rect, 0.0, t.color.surface.background);
            let gap = t.space.gap;
            let mut out = Intents::default();
            let mut fingerprint = Fingerprint::default();
            let card = Rect::from_min_size(
                rect.min + vec2(gap, gap),
                vec2(t.size.card.lane_width, rect.height() - 2.0 * gap),
            );
            let mut card_ui = ui.new_child(egui::UiBuilder::new().max_rect(card));
            Card::new(&t, &s.card).show(&mut card_ui, &mut out);
            let right = card.right() + gap;
            let tiles = Rect::from_min_max(
                pos2(right, rect.top() + gap),
                pos2(
                    rect.right() - gap,
                    rect.top() + 2.0 * t.size.tile.height + 7.0 * gap,
                ),
            );
            let _ = SourceTiles::new(&t, &s.sources).show(ui, tiles, &mut out);
            let view = OutputView {
                scope: Some((&s.scope, 1024)),
                peaks: Some([0.7, 0.35]),
                dt: 0.0,
            };
            let first = Rect::from_min_size(
                pos2(right, tiles.bottom() + gap),
                vec2(t.size.output.width, t.size.bar.header),
            );
            // One output region per frame (its widgets have global ids): the scope, or the spectrum.
            let state = if s.spectrum {
                &mut s.spectrum_state
            } else {
                &mut s.scope_state
            };
            Output::new(&t, view).show(ui, first, state, &mut fingerprint, &mut out);
        },
        scene(),
    );
    h.run();
    h.snapshot("region_live");
    h.state_mut().spectrum = true;
    h.run();
    h.snapshot("region_live_spectrum");
}
