//! Component snapshots (lane UI-C): one PNG per catalogue card and per region, rendered on lavapipe only
//! (tests/common), from the G1 reference patch. A pixel snapshot is never the only check of an interaction: the
//! interaction tests are in `tests/cards.rs`.
//!
//! Update after a deliberate change, then look at every changed PNG:
//! `UPDATE_SNAPSHOTS=1 cargo test -p enth-ui --test components`.

mod common;

use egui::{Rect, Ui, vec2};
use egui_kittest::SnapshotResults;
use enth_patch::{ModuleId, Patch, Section};
use enth_ui::Theme;
use enth_ui::editor::cards::view::{self, CardView};
use enth_ui::editor::cards::{Card, Hovered, Intents};
use enth_ui::editor::regions::{
    CardRegion, Header, HeaderView, MacroView, Macros, Matrix, MatrixView, SourceTiles, SourceView,
    Status,
};

/// The width of a strip card's snapshot: room for the display and one row of eight cells.
const STRIP_WIDTH: f32 = 720.0;

fn module_card(patch: &Patch, id: &str) -> CardView {
    let id = ModuleId::new(id).unwrap();
    let (_, m) = patch.find(&id).unwrap();
    CardView::module(patch, m)
}

/// Snapshots one card on the window background, as a strip or a stacked card, cropped to it.
fn card_snapshot(results: &mut SnapshotResults, name: &str, card: CardView, strip: bool) {
    let theme = Theme::dark();
    let t = theme.clone();
    let width = if strip {
        STRIP_WIDTH
    } else {
        theme.size.card.lane_width + 2.0 * theme.space.gap
    };
    let mut h = common::harness(
        &theme,
        vec2(width, 600.0),
        move |ui: &mut Ui, card: &mut CardView| {
            egui::Frame::new()
                .fill(t.color.surface.background)
                .inner_margin(t.space.gap)
                .show(ui, |ui| {
                    let mut out = Intents::default();
                    let c = Card::new(&t, card);
                    if strip { c.strip() } else { c }.show(ui, &mut out);
                });
        },
        card,
    );
    h.run();
    h.fit_contents();
    h.snapshot(name);
    results.extend_harness(&mut h);
}

#[test]
fn card_snapshots() {
    let mut results = SnapshotResults::new();
    let r = &mut results;
    let patch = common::patches::g1();
    card_snapshot(r, "card_osc", module_card(&patch, "osc2"), true);
    card_snapshot(
        r,
        "card_osc_more",
        module_card(&patch, "osc1").with_more(true),
        true,
    );
    // Disabled: the whole card fades but for its fill and its switch.
    let mut noise = module_card(&patch, "noise1");
    noise.enabled = Some(false);
    card_snapshot(r, "card_noise", noise, true);
    card_snapshot(r, "card_svf", module_card(&patch, "filter1"), false);
    card_snapshot(r, "card_adsr", module_card(&patch, "env2"), false);
    card_snapshot(
        r,
        "card_lfo",
        module_card(&patch, "lfo2").with_more(true),
        false,
    );
    card_snapshot(r, "card_reverb", module_card(&patch, "reverb1"), false);
    card_snapshot(r, "card_comp", module_card(&patch, "comp1"), false);
    card_snapshot(r, "card_voice", CardView::voice(&patch), false);
    card_snapshot(r, "card_master", CardView::master(&patch), false);
    results.unwrap();
}

/// Snapshots one region drawn into a `size` rectangle on the window background.
fn region_snapshot(
    results: &mut SnapshotResults,
    name: &str,
    size: egui::Vec2,
    draw: impl Fn(&mut Ui, Rect, &Theme) + 'static,
) {
    let theme = Theme::dark();
    let t = theme.clone();
    let mut h = common::harness(
        &theme,
        size,
        move |ui: &mut Ui, (): &mut ()| {
            // The whole window, not the panel's inner rect.
            let rect = ui.ctx().content_rect();
            ui.painter()
                .rect_filled(rect, 0.0, t.color.surface.background);
            draw(ui, rect.shrink(t.space.gap), &t);
        },
        (),
    );
    h.run();
    h.snapshot(name);
    results.extend_harness(&mut h);
}

#[test]
fn region_snapshots() {
    let mut results = SnapshotResults::new();
    let r = &mut results;
    let patch = common::patches::g1();
    let t = Theme::dark();
    let band = |height: f32| vec2(t.size.window.default.x, height + 2.0 * t.space.gap);

    let header = HeaderView {
        presets: enth_ui::editor::browser::HeaderPresets::default(),
        patch_name: patch.meta.name.clone(),
        can_undo: true,
        can_redo: false,
        undo: Some("Filter 1 › Cutoff".to_owned()),
        redo: None,
        room: enth_ui::editor::add_module::Room::all(&patch),
        // The hook fields of later lanes (the Recreate bar draws nothing without a shell's round view).
        ..HeaderView::default()
    };
    region_snapshot(
        r,
        "region_header",
        band(t.size.bar.header),
        move |ui, rect, t| {
            Header::new(t, &header).show(ui, rect, &mut Intents::default());
        },
    );

    let macros = MacroView::all(&patch);
    region_snapshot(
        r,
        "region_macros",
        band(t.size.bar.macros),
        move |ui, rect, t| {
            Macros::new(t, &macros).show(ui, rect, &mut Intents::default());
        },
    );

    let generators = view::section(&patch, Section::Generators);
    region_snapshot(
        r,
        "region_generators",
        vec2(760.0, 340.0),
        move |ui, rect, t| {
            CardRegion::generators(t, &generators).show(ui, rect, &mut Intents::default());
        },
    );

    let mut voice_fx = view::section(&patch, Section::VoiceFx);
    voice_fx.push(CardView::voice(&patch));
    let lane = t.size.card.lane_width + 2.0 * t.space.gap;
    region_snapshot(
        r,
        "region_voice_effects",
        vec2(lane, 420.0),
        move |ui, rect, t| {
            CardRegion::voice_effects(t, &voice_fx).show(ui, rect, &mut Intents::default());
        },
    );

    let mut fx = view::section(&patch, Section::Fx);
    fx.push(CardView::master(&patch));
    region_snapshot(
        r,
        "region_effects",
        vec2(lane, 560.0),
        move |ui, rect, t| {
            CardRegion::effects(t, &fx).show(ui, rect, &mut Intents::default());
        },
    );

    let modulators = view::section(&patch, Section::Modulators);
    region_snapshot(
        r,
        "region_modulators",
        vec2(960.0, 280.0),
        move |ui, rect, t| {
            CardRegion::modulators(t, &modulators).show(ui, rect, &mut Intents::default());
        },
    );

    let sources = SourceView::all(&patch);
    region_snapshot(
        r,
        "region_sources",
        vec2(420.0, 170.0),
        move |ui, rect, t| {
            SourceTiles::new(t, &sources).show(ui, rect, &mut Intents::default());
        },
    );

    let matrix = MatrixView::of(&patch);
    region_snapshot(
        r,
        "region_matrix",
        vec2(360.0, 190.0),
        move |ui, rect, t| {
            Matrix::new(t, &matrix).show(ui, rect, &mut Intents::default());
        },
    );

    let hovered = Hovered {
        path: "Filter 1 › Cutoff".to_owned(),
        value: "1.50 kHz".to_owned(),
        modulation: "Env 2 +40 %, Vel +10 %".to_owned(),
    };
    region_snapshot(
        r,
        "region_status",
        band(t.size.bar.status),
        move |ui, rect, t| {
            Status::new(t, Some(&hovered)).right("idle").show(ui, rect);
        },
    );
    results.unwrap();
}
