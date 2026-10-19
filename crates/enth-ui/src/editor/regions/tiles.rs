//! The modulation sources' live tiles: every envelope and LFO, then velocity, key and the mod wheel, each with its
//! shape, a route-count badge and the ⊕ handle that drag-to-modulate (lane ED2) starts from.

use egui::{Align2, Rect, Response, Ui, pos2};
use enth_patch::catalogue::LfoShape;
use enth_patch::{ModSource, ModuleKind, Patch};

use super::region_ui;
use crate::Theme;
use crate::editor::cards::{CardView, Hovered, Intents, ParamView, ids, shapes};
use crate::theme::glyph;
use crate::widgets::SourceTile;

/// What a tile's graph draws.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SourceShape {
    /// An envelope from its normalised attack, decay, sustain and release, placed like the card's display.
    Envelope([f32; 4]),
    /// An LFO's shape.
    Lfo(Option<LfoShape>),
    /// A rising line: velocity and key.
    Ramp,
    /// A level: the mod wheel.
    Level,
}

/// One modulation source.
#[derive(Clone, Debug, PartialEq)]
pub struct SourceView {
    pub src: ModSource,
    /// Its short name ("Env 2", "Vel").
    pub name: String,
    pub shape: SourceShape,
    /// How many routes start here.
    pub routes: usize,
    /// Where the source is in its shape now (0..1), while it runs (the engine's telemetry, lane ED2).
    pub phase: Option<f32>,
}

impl SourceView {
    /// The patch's modulators in order, then velocity, key and the mod wheel.
    pub fn all(patch: &Patch) -> Vec<SourceView> {
        let routes = |src: &ModSource| patch.routes.iter().filter(|r| &r.src == src).count();
        let mut out: Vec<SourceView> = patch
            .modulators
            .iter()
            .map(|m| {
                let card = CardView::module(patch, m);
                let norm = |key: &str| card.param(key).map_or(0.0, ParamView::norm);
                let shape = match m.kind {
                    ModuleKind::Lfo => SourceShape::Lfo(
                        card.param("shape")
                            .and_then(|p| LfoShape::from_value(p.value)),
                    ),
                    _ => SourceShape::Envelope([
                        norm("attack"),
                        norm("decay"),
                        norm("sustain"),
                        norm("release"),
                    ]),
                };
                let src = ModSource::Module(m.id.clone());
                SourceView {
                    routes: routes(&src),
                    name: card.title,
                    src,
                    shape,
                    phase: None,
                }
            })
            .collect();
        for (src, name, shape) in [
            (ModSource::Velocity, "Vel", SourceShape::Ramp),
            (ModSource::Key, "Key", SourceShape::Ramp),
            (ModSource::ModWheel, "Wheel", SourceShape::Level),
        ] {
            out.push(SourceView {
                routes: routes(&src),
                src,
                name: name.to_owned(),
                shape,
                phase: None,
            });
        }
        out
    }
}

/// The tiles region.
pub struct SourceTiles<'a> {
    theme: &'a Theme,
    sources: &'a [SourceView],
}

impl<'a> SourceTiles<'a> {
    pub fn new(theme: &'a Theme, sources: &'a [SourceView]) -> Self {
        Self { theme, sources }
    }

    /// Draws a captioned panel of tiles into `rect`, wrapping into rows, and reports a hovered handle to the status
    /// line. Returns each source's ⊕ handle response: drag-to-modulate (ED2) sets its payload from `drag_started`.
    pub fn show(self, ui: &mut Ui, rect: Rect, out: &mut Intents) -> Vec<(ModSource, Response)> {
        let t = self.theme;
        let mut ui = region_ui(ui, rect, "sources");
        ui.painter()
            .rect_filled(rect, t.radius.medium, t.color.surface.card);
        let inner = rect.shrink(t.space.gap);
        let caption = ui.painter().text(
            inner.left_top(),
            Align2::LEFT_TOP,
            "Sources",
            t.text.label.clone(),
            t.color.text.label,
        );
        let body = Rect::from_min_max(
            pos2(inner.left(), caption.bottom() + t.space.tight),
            inner.max,
        );
        let mut tiles = ui.new_child(egui::UiBuilder::new().max_rect(body));
        let stages = crate::editor::cards::envelope_stages(t);
        let mut handles = Vec::with_capacity(self.sources.len());
        egui::ScrollArea::vertical()
            .id_salt("scroll")
            .auto_shrink(false)
            .show(&mut tiles, |ui| {
                ui.spacing_mut().item_spacing = egui::vec2(t.space.tight, t.space.tight);
                ui.horizontal_wrapped(|ui| {
                    for source in self.sources {
                        let shape = source.shape;
                        let graph = move |x: f32| match shape {
                            SourceShape::Envelope([a, d, s, r]) => stages.curve(x, a, d, s, r),
                            SourceShape::Lfo(shape) => shapes::lfo(shape, x),
                            SourceShape::Ramp => x,
                            SourceShape::Level => 0.5,
                        };
                        let handle = SourceTile::new(t, "tile", &source.name, &graph)
                            .id(ids::source(&source.src))
                            .routes(source.routes)
                            .phase(source.phase)
                            .show(ui);
                        if handle.hovered() || handle.dragged() {
                            let routes = match source.routes {
                                1 => "1 route".to_owned(),
                                n => format!("{n} routes"),
                            };
                            out.hovered = Some(Hovered {
                                path: format!("Sources {} {}", glyph::PATH, source.name),
                                value: routes,
                                modulation: String::new(),
                            });
                        }
                        handles.push((source.src.clone(), handle));
                    }
                });
            });
        handles
    }
}
