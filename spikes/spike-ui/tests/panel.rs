//! The reference panel: full-panel snapshot, repaint gating, lane reordering, host gestures.
//! Update the snapshot after a deliberate change:
//! `UPDATE_SNAPSHOTS=1 cargo test -p spike-ui --test panel`.

mod common;

use std::sync::Arc;
use std::time::Duration;

use egui::ViewportId;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use enth_ui::Theme;
use spike_ui::{AudioFeed, DemoChord, LocalParams, REFERENCE_SIZE, ReferencePanel, Repaint};

struct State {
    panel: ReferencePanel,
    params: LocalParams,
    last: Repaint,
}

fn harness(feed: &Arc<AudioFeed>) -> Harness<'static, State> {
    let theme = Theme::dark();
    let state = State {
        panel: ReferencePanel::new(theme.clone(), Arc::clone(feed)),
        params: LocalParams::default(),
        last: Repaint::Idle,
    };
    common::harness(
        &theme,
        REFERENCE_SIZE,
        |ui, s: &mut State| s.last = s.panel.ui(ui, &mut s.params),
        state,
    )
}

/// One block of the demo chord (deterministic), with LFO 1 at `phase`.
fn publish(feed: &AudioFeed, samples: usize, phase: f32, active: bool) -> bool {
    let mut chord = DemoChord::default();
    let mut writer = feed.writer();
    for _ in 0..samples {
        let sample = chord.next_sample();
        writer.push(if active { sample } else { 0.0 });
    }
    writer.finish(phase, active)
}

fn repaint_delay(h: &Harness<'_, State>) -> Duration {
    h.output().viewport_output[&ViewportId::ROOT].repaint_delay
}

#[test]
fn reference_panel_snapshot() {
    let feed = Arc::new(AudioFeed::new());
    publish(&feed, 4096, 0.2, true);
    let mut h = harness(&feed);
    h.run();
    h.snapshot("reference_panel");
}

#[test]
fn idle_panel_schedules_nothing() {
    let feed = Arc::new(AudioFeed::new());
    let mut h = harness(&feed);
    h.run();
    assert_eq!(h.state().last, Repaint::Idle);
    assert_eq!(
        repaint_delay(&h),
        Duration::MAX,
        "no frame scheduled while idle"
    );
    assert!(
        publish(&feed, 64, 0.0, true),
        "the sleeping panel asks to be woken"
    );
}

#[test]
fn animation_runs_at_the_motion_rates_and_stops() {
    let theme = Theme::dark();
    let feed = Arc::new(AudioFeed::new());
    let mut h = harness(&feed);
    h.run();

    publish(&feed, 256, 0.1, true);
    h.step();
    assert_eq!(
        h.state().last,
        Repaint::Scope,
        "fresh samples, visible scope"
    );
    assert_eq!(repaint_delay(&h), theme.motion.scope_interval());

    h.step();
    assert_eq!(
        h.state().last,
        Repaint::Animated,
        "LFO running, rings visible"
    );
    assert_eq!(repaint_delay(&h), theme.motion.animated_interval());

    publish(&feed, 256, 0.2, false); // notes released: one silent block clears the scope
    h.step();
    assert_eq!(h.state().last, Repaint::Scope);
    h.step();
    assert_eq!(h.state().last, Repaint::Idle);
    assert_eq!(repaint_delay(&h), Duration::MAX);
    assert!(
        !publish(&feed, 256, 0.3, false),
        "silence after silence: nothing to wake for"
    );
}

#[test]
fn lane_cards_reorder_by_dragging_their_headers() {
    let feed = Arc::new(AudioFeed::new());
    let mut h = harness(&feed);
    h.run();
    assert_eq!(h.state().panel.lane_order(), [0, 1, 2]);
    let from = h.get_by_label("Move Delay").rect().center();
    let to = h.get_by_label("Move Drive").rect().center() - egui::vec2(0.0, 4.0);
    h.hover_at(from);
    h.drag_at(from);
    h.run();
    for step in 1..=6 {
        h.hover_at(from + (to - from) * step as f32 / 6.0);
        h.run();
    }
    h.drop_at(to);
    h.run();
    assert_eq!(
        h.state().panel.lane_order(),
        [1, 0, 2],
        "Delay moved above Drive"
    );
}

#[test]
fn knob_drags_reach_the_host_as_balanced_gestures() {
    let feed = Arc::new(AudioFeed::new());
    let mut h = harness(&feed);
    h.run();
    let before = h.state().params.slots[0];
    let start = h.get_by_label("Cutoff").rect().center();
    h.hover_at(start);
    h.drag_at(start);
    h.run();
    for step in 1..=4 {
        h.hover_at(start - egui::vec2(0.0, 10.0 * step as f32));
        h.run();
    }
    h.drop_at(start - egui::vec2(0.0, 40.0));
    h.run();
    let state = h.state();
    assert!(state.params.slots[0] > before, "slot 1 (cutoff) went up");
    assert_eq!(state.params.open_gestures, 0, "every begin has an end");
}
