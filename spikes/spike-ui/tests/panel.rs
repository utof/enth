//! The reference panel: full-panel snapshots, repaint gating, lane reordering, host gestures,
//! envelope handles kept in sync with their knobs.
//! Update the snapshots after a deliberate change:
//! `UPDATE_SNAPSHOTS=1 cargo test -p spike-ui --test panel`.

mod common;

use std::sync::Arc;
use std::time::Duration;

use egui::{Role, ViewportId};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
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

/// Host slots of the amp envelope's attack, decay and sustain (`layout::GENERATORS`).
const AMP_ATTACK: usize = 12;
const AMP_DECAY: usize = 13;
const AMP_SUSTAIN: usize = 14;

/// Drags from the centre of the node labelled `label` by `delta`, in four steps.
fn drag(h: &mut Harness<'_, State>, label: &str, delta: egui::Vec2) {
    let start = h.get_by_label(label).rect().center();
    h.hover_at(start);
    h.drag_at(start);
    h.run();
    for step in 1..=4 {
        h.hover_at(start + delta * step as f32 / 4.0);
        h.run();
    }
    h.drop_at(start + delta);
    h.run();
}

/// Height of a generator strip's trace area: the card less its bottom inset, header and the
/// display's own inset above and below.
fn strip_trace_height(t: &Theme) -> f32 {
    t.size.card.generator_height
        - t.space.tight
        - t.size.card.collapsed_height
        - 2.0 * t.space.tight
}

#[test]
fn dragging_the_sustain_handle_changes_sustain_only() {
    let theme = Theme::dark();
    let feed = Arc::new(AudioFeed::new());
    let mut h = harness(&feed);
    h.run();
    let before = h.state().params.slots;
    drag(&mut h, "Amp env decay and sustain", egui::vec2(0.0, 20.0));
    let state = h.state();
    let expected = before[AMP_SUSTAIN] - 20.0 / strip_trace_height(&theme);
    let sustain = state.params.slots[AMP_SUSTAIN];
    assert!(
        (sustain - expected).abs() < 1e-3,
        "sustain after dragging down: {sustain}, expected {expected}"
    );
    assert_eq!(
        state.params.slots[AMP_DECAY], before[AMP_DECAY],
        "a vertical drag leaves the decay alone"
    );
    assert_eq!(state.params.open_gestures, 0, "every begin has an end");
}

#[test]
fn dragging_the_attack_handle_moves_the_attack_knob() {
    let theme = Theme::dark();
    let feed = Arc::new(AudioFeed::new());
    let mut h = harness(&feed);
    h.run();
    let before = h.state().params.slots[AMP_ATTACK];
    let handle = h.get_by_label("Amp env attack").rect().center();
    drag(&mut h, "Amp env attack", egui::vec2(16.0, 0.0));
    // The curve gives a stage `STAGE_RANGE` of the display's width over the knob's range.
    let expected = before + 16.0 / theme.size.card.display_width / spike_ui::shapes::STAGE_RANGE;
    let attack = h.state().params.slots[AMP_ATTACK];
    assert!(
        (attack - expected).abs() < 1e-3,
        "attack after dragging right: {attack}, expected {expected}"
    );
    let moved = h.get_by_label("Amp env attack").rect().center() - handle;
    assert!(
        (moved - egui::vec2(16.0, 0.0)).length() < 1.0,
        "the peak stays under the pointer: moved {moved:?}"
    );
    // The generator column lists the amp envelope before the filter envelope.
    let knob = h
        .get_all_by_role_and_label(Role::Slider, "Attack")
        .next()
        .unwrap()
        .accesskit_node()
        .numeric_value()
        .unwrap();
    assert!(
        (knob - f64::from(attack)).abs() < 1e-6,
        "the Attack knob shows the dragged value: {knob} vs {attack}"
    );
    assert_eq!(h.state().params.open_gestures, 0, "every begin has an end");
}

#[test]
fn disabled_card_and_hovered_handle_snapshot() {
    let feed = Arc::new(AudioFeed::new());
    publish(&feed, 4096, 0.2, true);
    let mut h = harness(&feed);
    h.run();
    h.get_by_role_and_label(Role::CheckBox, "Osc 2").click();
    h.run();
    // The readout goes above the handle; above the release there is only empty display.
    h.get_by_label("Amp env release").hover();
    h.run();
    h.snapshot("disabled_card_and_hovered_handle");
}
