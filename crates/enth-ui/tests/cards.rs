//! Interaction tests for the catalogue cards and regions (lane UI-C), through egui's input and the AccessKit tree,
//! with no renderer. A small stand-in for the editor (ED1) draws the regions from the patch every frame and applies
//! the frame's intents to the patch through `Intent::ops` and `enth_patch::apply`, the way the session will.
//!
//! - a slow drag on a discrete knob moves it (sub-step motion accumulates);
//! - envelope handles whose grab areas overlap pick the nearest handle, and handles move on the knobs' scales;
//! - every control keeps its id across a re-layout (research 02's 59 of 59), and a drag survives one.

mod common;

use std::collections::BTreeSet;

use egui::{Rect, Role, Sense, Ui, Vec2, pos2, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use enth_patch::catalogue::{AdsrParam, VoiceParam};
use enth_patch::{ModuleId, ParamAddr, Patch, Section};
use enth_ui::Theme;
use enth_ui::editor::cards::view::{self, CardView};
use enth_ui::editor::cards::{Card, CardKey, Grip, Intent, Intents, ids};
use enth_ui::editor::regions::CardRegion;
use enth_ui::widgets::{CurveHandles, Display, Drag, Handle, Knob, Trace};

/// Two arrangements of the same regions: the second moves every region and draws the generators as stacked cards
/// in a band instead of strips.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Layout {
    Lanes,
    Moved,
}

/// The editor stand-in: the patch, the layout, "More" state, and every intent it applied.
struct Editor {
    theme: Theme,
    patch: Patch,
    layout: Layout,
    more: BTreeSet<CardKey>,
    log: Vec<Intent>,
}

impl Editor {
    fn new(patch: Patch) -> Self {
        Self {
            theme: Theme::dark(),
            patch,
            layout: Layout::Lanes,
            more: BTreeSet::new(),
            log: Vec::new(),
        }
    }

    fn cards(&self, section: Option<Section>) -> Vec<CardView> {
        let cards = match section {
            Some(s) => view::section(&self.patch, s),
            None => vec![CardView::voice(&self.patch), CardView::master(&self.patch)],
        };
        cards
            .into_iter()
            .map(|c| {
                let open = self.more.contains(&c.key);
                c.with_more(open)
            })
            .collect()
    }

    fn frame(&mut self, ui: &mut Ui) {
        let t = self.theme.clone();
        let generators = self.cards(Some(Section::Generators));
        let mut voice_fx = self.cards(Some(Section::VoiceFx));
        let mut fx = self.cards(Some(Section::Fx));
        let modulators = self.cards(Some(Section::Modulators));
        let builtins = self.cards(None);
        voice_fx.push(builtins[0].clone());
        fx.push(builtins[1].clone());
        let r = |x0: f32, y0: f32, x1: f32, y1: f32| Rect::from_min_max(pos2(x0, y0), pos2(x1, y1));
        let mut out = Intents::default();
        match self.layout {
            Layout::Lanes => {
                CardRegion::generators(&t, &generators).show(
                    ui,
                    r(0.0, 0.0, 760.0, 380.0),
                    &mut out,
                );
                CardRegion::modulators(&t, &modulators).show(
                    ui,
                    r(0.0, 390.0, 1000.0, 1000.0),
                    &mut out,
                );
                CardRegion::voice_effects(&t, &voice_fx).show(
                    ui,
                    r(1010.0, 0.0, 1250.0, 1000.0),
                    &mut out,
                );
                CardRegion::effects(&t, &fx).show(ui, r(1260.0, 0.0, 1500.0, 1000.0), &mut out);
            }
            Layout::Moved => {
                CardRegion::effects(&t, &fx).show(ui, r(0.0, 0.0, 240.0, 1000.0), &mut out);
                CardRegion::voice_effects(&t, &voice_fx).show(
                    ui,
                    r(250.0, 0.0, 490.0, 1000.0),
                    &mut out,
                );
                CardRegion::new(
                    &t,
                    "generators",
                    &generators,
                    enth_ui::editor::regions::Arrangement::Band,
                )
                .show(ui, r(500.0, 0.0, 1500.0, 350.0), &mut out);
                CardRegion::modulators(&t, &modulators).show(
                    ui,
                    r(500.0, 360.0, 1500.0, 1000.0),
                    &mut out,
                );
            }
        }
        for intent in out {
            self.apply(intent);
        }
    }

    fn apply(&mut self, intent: Intent) {
        if let Intent::ShowMore { card, open } = &intent {
            if *open {
                self.more.insert(card.clone());
            } else {
                self.more.remove(card);
            }
        }
        for op in intent.ops(&self.patch) {
            enth_patch::apply(&mut self.patch, op).expect("an intent's op applies");
        }
        self.log.push(intent);
    }

    fn value(&self, addr: &str) -> f32 {
        self.patch.value(&addr.parse().unwrap()).unwrap()
    }
}

const WINDOW: Vec2 = vec2(1500.0, 1000.0);

fn harness(patch: Patch) -> Harness<'static, Editor> {
    Harness::builder()
        .with_size(WINDOW)
        .with_pixels_per_point(1.0)
        .with_step_dt(1.0 / 60.0)
        .with_context_setup(|ctx| Theme::dark().apply(ctx))
        .build_ui_state(|ui, e: &mut Editor| e.frame(ui), Editor::new(patch))
}

fn center(h: &Harness<'_, Editor>, label: &str) -> egui::Pos2 {
    h.get_by_label(label).rect().center()
}

/// Presses at `from`, moves by `delta` in `frames` equal steps (one frame each), releases.
fn drag<S>(h: &mut Harness<'_, S>, from: egui::Pos2, delta: Vec2, frames: usize) {
    h.hover_at(from);
    h.step();
    h.drag_at(from);
    h.step();
    for i in 1..=frames {
        h.hover_at(from + delta * i as f32 / frames as f32);
        h.step();
    }
    h.drop_at(from + delta);
    h.step();
    h.step();
}

/// Checks that every `Begin` has its `End` on the same control, with no other gesture inside it.
fn gestures_balanced(log: &[Intent]) -> usize {
    let mut open: Option<&Grip> = None;
    let mut count = 0;
    for intent in log {
        match intent {
            Intent::Begin(g) => {
                assert!(open.is_none(), "a gesture opened inside another: {log:?}");
                open = Some(g);
                count += 1;
            }
            Intent::End(g) => {
                assert_eq!(
                    open,
                    Some(g),
                    "a gesture closed on another control: {log:?}"
                );
                open = None;
            }
            _ => {}
        }
    }
    assert!(open.is_none(), "a gesture stayed open: {log:?}");
    count
}

#[test]
fn a_slow_drag_moves_a_discrete_knob() {
    let mut h = harness(common::patches::g1());
    h.run_steps(2);
    assert_eq!(h.state().value("voice.polyphony"), 16.0);
    // Polyphony is an Int (1–16): one step is 200 / 15 ≈ 13 px of travel. 60 px at 1 px per frame never moves a
    // whole step in one frame.
    let at = center(&h, "Polyphony");
    drag(&mut h, at, vec2(0.0, 60.0), 60);
    let e = h.state();
    let polyphony = e.value("voice.polyphony");
    assert!(
        (11.0..=12.0).contains(&polyphony),
        "60 px down moves 4.5 steps from 16: {polyphony}"
    );
    assert_eq!(gestures_balanced(&e.log), 1, "one drag, one gesture");
    let addr: ParamAddr = "voice.polyphony".parse().unwrap();
    let sets: Vec<f32> = e
        .log
        .iter()
        .filter_map(|i| match i {
            Intent::Set { addr: a, value } if *a == addr => Some(*value),
            _ => None,
        })
        .collect();
    assert!(
        sets.windows(2).all(|w| w[1] == w[0] - 1.0),
        "one Set per whole step, never a fraction: {sets:?}"
    );
}

#[test]
fn without_accumulation_the_same_slow_drag_is_lost() {
    // The negative control: a knob that re-derives its position from the rounded value every frame (as the editor
    // does from the patch), but isn't told it has steps, drops every sub-step move.
    let theme = Theme::dark();
    let spec = &enth_patch::catalogue::VOICE_PARAMS[VoiceParam::Polyphony.index()];
    let mut h = Harness::builder()
        .with_size(vec2(200.0, 200.0))
        .with_step_dt(1.0 / 60.0)
        .with_context_setup(|ctx| Theme::dark().apply(ctx))
        .build_ui_state(
            move |ui, (value, steps): &mut (f32, bool)| {
                let mut norm = spec.to_norm(*value);
                let mut knob = Knob::new(&theme, &mut norm, "Polyphony");
                if *steps {
                    knob = knob.steps(spec.step_count().unwrap());
                }
                if ui.add(knob).changed() {
                    *value = spec.clamp(spec.from_norm(norm));
                }
            },
            (16.0, false),
        );
    h.run_steps(2);
    let at = h.get_by_label("Polyphony").rect().center();
    drag(&mut h, at, vec2(0.0, 60.0), 60);
    assert_eq!(h.state().0, 16.0, "sub-step moves rounded away every frame");
    h.state_mut().1 = true;
    drag(&mut h, at, vec2(0.0, 60.0), 60);
    assert!(h.state().0 < 16.0, "with steps the same drag moves it");
}

/// A 200 × 100 display with two handles a few px apart on one row: each drags its own value sideways.
fn two_handles(theme: &Theme, gap: f32) -> Harness<'_, [f32; 2]> {
    Harness::builder()
        .with_size(vec2(300.0, 200.0))
        .with_step_dt(1.0 / 60.0)
        .with_context_setup(|ctx| Theme::dark().apply(ctx))
        .build_ui_state(
            move |ui, values: &mut [f32; 2]| {
                let (rect, _) = ui.allocate_exact_size(vec2(200.0, 100.0), Sense::hover());
                let [a, b] = values;
                // Each point follows its value; the second starts `gap` px right of the first.
                let (at_a, at_b) = (*a, *b + gap / 200.0);
                let mut handles = [
                    Handle::new("First", vec2(at_a, 0.5)).x(Drag::new("A", a, 1.0)),
                    Handle::new("Second", vec2(at_b, 0.5)).x(Drag::new("B", b, 1.0)),
                ];
                let edit = CurveHandles::new(theme, "pair").interact(ui, rect, &mut handles);
                Display::new(theme, Trace::Curve(&|x| x), rect.size())
                    .handles(&edit)
                    .paint(ui, rect);
            },
            [0.5, 0.5],
        )
}

#[test]
fn overlapping_handles_pick_the_nearest() {
    let theme = Theme::dark();
    let reach = theme.size.handle.reach;
    // 6 px apart: each handle's square (2 × reach wide) covers the other's centre.
    let gap = 6.0;
    let mut h = two_handles(&theme, gap);
    h.run();
    let first = h.get_by_label("First").rect();
    let second = h.get_by_label("Second").rect();
    assert!(
        first.right() <= second.left() + 0.01,
        "the grab areas don't overlap: {first:?} {second:?}"
    );
    // The first handle's centre, from its unclipped left edge; press a little to the right of it, which is inside
    // the second's square too. egui alone would pick the second (the later, topmost widget).
    let first_center = pos2(first.left() + reach, first.center().y);
    drag(&mut h, first_center + vec2(1.0, 0.0), vec2(20.0, 0.0), 4);
    let [a, b] = *h.state();
    assert!(
        a > 0.5 && b == 0.5,
        "the nearer, first handle moved: {a} {b}"
    );
    let second_center = pos2(second.right() - reach, second.center().y);
    drag(&mut h, second_center - vec2(1.0, 0.0), vec2(20.0, 0.0), 4);
    let [_, b] = *h.state();
    assert!(b > 0.5, "and the second is still reachable: {b}");

    // At exactly the same point, the later one wins (egui's topmost), and dragging it apart frees the other.
    let mut h = two_handles(&theme, 0.0);
    h.run();
    let at = h.get_by_label("Second").rect().center();
    drag(&mut h, at, vec2(30.0, 0.0), 4);
    let [a, b] = *h.state();
    assert!(
        a == 0.5 && b > 0.5,
        "coincident: the later handle moved: {a} {b}"
    );
    let at = h.get_by_label("First").rect().center();
    drag(&mut h, at, vec2(-20.0, 0.0), 4);
    assert!(h.state()[0] < 0.5, "then the first is reachable again");
}

/// An envelope with no attack, no decay and full sustain: its attack peak and its decay corner sit
/// `envelope_min_stage` of the width apart, closer than two grab reaches on a lane card.
fn tight_envelope() -> Patch {
    let mut p = common::patches::g1();
    let env = &mut p.modulators[1];
    env.values[AdsrParam::Attack.index()] = 0.0;
    env.values[AdsrParam::Decay.index()] = 0.0;
    env.values[AdsrParam::Sustain.index()] = 100.0;
    p
}

#[test]
fn coincident_envelope_handles_pick_the_expected_one() {
    let theme = Theme::dark();
    let reach = theme.size.handle.reach;
    let mut h = harness(tight_envelope());
    h.run_steps(2);
    let peak_area = h.get_by_label("Env 2 attack").rect();
    let corner_area = h.get_by_label("Env 2 decay and sustain").rect();
    // Neither area is clipped on its outer side or at the bottom (the release is far to the right).
    let peak = pos2(peak_area.left() + reach, peak_area.bottom() - reach);
    let corner = pos2(corner_area.right() - reach, corner_area.bottom() - reach);
    let apart = corner.x - peak.x;
    assert!(
        apart < 2.0 * reach,
        "the two grab squares overlap ({apart} px apart), so picking matters"
    );
    // Just right of the peak: inside both squares, nearer the peak.
    drag(&mut h, peak + vec2(apart / 4.0, 0.0), vec2(12.0, 0.0), 6);
    let e = h.state();
    assert!(e.value("env2.attack") > 0.0, "the attack moved");
    assert_eq!(e.value("env2.decay"), 0.0, "the decay did not");
    assert_eq!(
        e.log.first(),
        Some(&Intent::Begin(Grip::Handle(
            ModuleId::new("env2").unwrap(),
            0
        )))
    );
    gestures_balanced(&e.log);

    let mut h = harness(tight_envelope());
    h.run_steps(2);
    drag(&mut h, corner - vec2(apart / 4.0, 0.0), vec2(12.0, 6.0), 6);
    let e = h.state();
    assert_eq!(e.value("env2.attack"), 0.0, "the attack did not move");
    assert!(e.value("env2.decay") > 0.0, "the decay moved");
    assert!(
        e.value("env2.sustain") < 100.0,
        "and the sustain, in the same gesture"
    );
    assert_eq!(gestures_balanced(&e.log), 1);
}

#[test]
fn envelope_handles_move_on_the_knobs_power_scale() {
    // A drag across `envelope_stage_range` of the trace's width sweeps the knob's whole normalised range, through
    // the spec's own power-4 scale: half of that travel from 0 ms lands on from_norm(0.5) = 625 ms, not 5 s.
    let theme = Theme::dark();
    let mut h = harness(tight_envelope());
    h.run_steps(2);
    let reach = theme.size.handle.reach;
    let card = h.get_by_label("Move Env 2").rect();
    let trace_width = theme.size.card.lane_width - 2.0 * theme.space.gap;
    assert!(card.width() >= trace_width);
    let peak_area = h.get_by_label("Env 2 attack").rect();
    let peak = pos2(peak_area.left() + reach, peak_area.bottom() - reach);
    let half_sweep = theme.size.cards.envelope_stage_range * trace_width / 2.0;
    drag(&mut h, peak, vec2(half_sweep, 0.0), 8);
    let attack = h.state().value("env2.attack");
    let spec = &enth_patch::catalogue::ADSR_PARAMS[AdsrParam::Attack.index()];
    assert!(
        (spec.to_norm(attack) - 0.5).abs() < 0.02,
        "half the sweep is half the knob: {attack} ms (norm {})",
        spec.to_norm(attack)
    );
    assert!(
        (attack - 625.0).abs() < 80.0,
        "625 ms on the power-4 scale: {attack}"
    );
}

/// The addresses whose controls show with every card's "More" closed: basic and intermediate tiers.
fn shown_addresses(patch: &Patch) -> Vec<ParamAddr> {
    view::all(patch)
        .iter()
        .flat_map(|c| c.shown().map(|p| p.addr.clone()).collect::<Vec<_>>())
        .collect()
}

/// The ids of every parameter control in the AccessKit tree (knobs, segmented groups, switches).
fn control_ids(h: &Harness<'_, Editor>) -> BTreeSet<u64> {
    h.root()
        .query_all_by(|n| matches!(n.role(), Role::Slider | Role::RadioGroup | Role::Switch))
        .map(|n| n.accesskit_node().locate().0.0)
        .collect()
}

#[test]
fn every_control_keeps_its_id_across_a_relayout() {
    let patch = common::patches::g1();
    let expected: BTreeSet<u64> = shown_addresses(&patch)
        .iter()
        .map(|a| ids::param(a).accesskit_id().0)
        .collect();
    assert_eq!(
        expected.len(),
        59,
        "the G1 patch shows 59 controls (research 02)"
    );
    let mut h = harness(patch);
    h.run_steps(2);
    let lanes = control_ids(&h);
    h.state_mut().layout = Layout::Moved;
    h.run_steps(2);
    let moved = control_ids(&h);
    let kept = expected
        .iter()
        .filter(|id| lanes.contains(id) && moved.contains(id))
        .count();
    println!(
        "{kept} of {} control ids unchanged across the re-layout",
        expected.len()
    );
    assert_eq!(
        kept, 59,
        "every control's id is its address's, in both layouts"
    );
    // The rest are the envelopes' handles (three each), whose ids come from their module, and the depth handles of
    // the seven routed parameters (lane ED2), whose ids come from their address.
    let handles = |set: &BTreeSet<u64>| set.difference(&expected).count();
    assert_eq!((handles(&lanes), handles(&moved)), (6 + 7, 6 + 7));
}

#[test]
fn a_drag_survives_a_relayout() {
    let mut h = harness(common::patches::g1());
    h.run_steps(2);
    let start = h.state().value("filter1.cutoff");
    let p = center(&h, "Cutoff");
    h.hover_at(p);
    h.step();
    h.drag_at(p);
    h.step();
    for dy in [20.0, 40.0] {
        h.hover_at(p - vec2(0.0, dy));
        h.step();
    }
    let mid = h.state().value("filter1.cutoff");
    // The filter card moves from x ≈ 1010 to x ≈ 250; the pointer keeps going up from where it is.
    h.state_mut().layout = Layout::Moved;
    for dy in [60.0, 80.0] {
        h.hover_at(p - vec2(0.0, dy));
        h.step();
    }
    h.drop_at(p - vec2(0.0, 80.0));
    h.step();
    h.step();
    let e = h.state();
    let end = e.value("filter1.cutoff");
    assert!(
        mid > start && end > mid,
        "kept dragging after the relayout: {start} → {mid} → {end}"
    );
    assert_eq!(gestures_balanced(&e.log), 1, "one gesture, closed");
}

#[test]
fn a_more_disclosure_shows_the_advanced_tier_and_controls_edit_the_patch() {
    let mut h = harness(common::patches::g1());
    h.run_steps(2);
    assert!(
        h.query_by_label("Seed").is_none(),
        "advanced controls wait behind More"
    );
    // Both oscillators and both LFOs have a More; open Osc 1's.
    let root = h.root();
    let more = root
        .query_all_by_role_and_label(Role::Button, "More")
        .min_by(|a, b| {
            let (a, b) = (a.rect().center(), b.rect().center());
            a.y.total_cmp(&b.y).then(a.x.total_cmp(&b.x))
        })
        .unwrap();
    more.click();
    h.run_steps(2);
    assert!(
        h.state()
            .more
            .contains(&CardKey::Module(ModuleId::new("osc1").unwrap()))
    );
    assert_eq!(
        h.query_all_by_role_and_label(Role::Slider, "Seed").count(),
        1,
        "Osc 1's seed knob shows"
    );
    // A segmented click is one Select; a switch is one Toggle; the card switch is one Enable.
    h.get_by_label("Notch").click();
    h.run_steps(2);
    assert_eq!(h.state().value("filter1.mode"), 3.0);
    h.get_all_by_label("Sync").next().unwrap().click();
    h.run_steps(2);
    assert_eq!(h.state().value("osc1.sync"), 1.0);
    h.get_by_role_and_label(Role::CheckBox, "Noise 1").click();
    h.run_steps(2);
    assert!(!h.state().patch.generators[2].enabled);
}

#[test]
fn a_card_drawn_alone_reports_its_hovered_control() {
    let theme = Theme::dark();
    let patch = common::patches::g1();
    let card = {
        let (_, m) = patch.find(&ModuleId::new("filter1").unwrap()).unwrap();
        CardView::module(&patch, m)
    };
    let mut h = Harness::builder()
        .with_size(vec2(260.0, 360.0))
        .with_context_setup(|ctx| Theme::dark().apply(ctx))
        .build_ui_state(
            move |ui, last: &mut Intents| {
                let mut out = Intents::default();
                Card::new(&theme, &card).show(ui, &mut out);
                *last = out;
            },
            Intents::default(),
        );
    h.run();
    let at = h.get_by_label("Cutoff").rect().center();
    h.hover_at(at);
    h.run();
    let hovered = h
        .state()
        .hovered
        .clone()
        .expect("the knob under the pointer is reported");
    assert_eq!(hovered.path, "Filter 1 › Cutoff");
    assert_eq!(hovered.value, "1.50 kHz");
    assert_eq!(hovered.modulation, "Env 2 +40 %, Vel +10 %");
}
