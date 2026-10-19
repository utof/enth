//! The reference panel (docs/benchmarks.md) as a literal slice of direction A "Lanes", in Learn
//! mode: header, macro bar, lesson pane with the matrix under it, the generator column (scrolls),
//! one effect lane with three draggable cards, the modulator strip (source tiles, scope,
//! spectrum) and the status line. Every visual value comes from the theme.
//!
//! Repaint gating: a frame is scheduled only while something animated is visible and its data
//! changed: `motion.scope_hz` while a visible scope has fresh samples, `motion.animated_hz` while
//! the LFO runs and a modulated knob or a source tile is visible. Otherwise the panel asks the
//! feed to wake it on the next change and schedules nothing.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use egui::{Align, Align2, Layout, Rect, Response, TextureHandle, Ui, UiBuilder, pos2, vec2};
use enth_ui::Theme;
use enth_ui::theme::glyph;
use enth_ui::widgets::{
    Display, Ink, Knob, LessonPane, Macro, MacroBar, MatrixGrid, ModuleCard, Segmented, SourceTile,
    StatusLine, Trace,
};

use crate::feed::AudioFeed;
use crate::layout::{self, Figure, Module};
use crate::lesson;
use crate::params::{HostParams, ParamId};
use crate::spectrum::Analyser;

/// Window size of the reference panel in every UI arm (docs/benchmarks.md).
pub const REFERENCE_SIZE: egui::Vec2 = egui::vec2(1100.0, 768.0);
/// Oscilloscope length (docs/benchmarks.md).
pub const SCOPE_SAMPLES: usize = 2048;

const MODES: [&str; 4] = ["Play", "Recreate", "Learn", "From sound"];
const LEARN: usize = 2;
const TRANSPORT: [&str; 2] = ["Audio off", "Audio on"];

/// What this frame asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Repaint {
    /// Nothing scheduled; the feed wakes the panel on the next change.
    Idle,
    /// Live rings or tiles at `motion.animated_hz`.
    Animated,
    /// A fed scope at `motion.scope_hz`.
    Scope,
}

/// A source tile: name, shape, phase while running, route count.
type TileSpec<'a> = (&'a str, &'a dyn Fn(f32) -> f32, Option<f32>, usize);

/// The hovered control, for the status line.
struct Hover {
    path: String,
    value: String,
    modulation: String,
}

/// Wave tables for the oscillator displays (computed once).
struct Tables {
    saw: Vec<f32>,
    sine: Vec<f32>,
    noise: Vec<f32>,
    spread: Vec<f32>,
}

impl Tables {
    fn new() -> Self {
        let n = 256;
        let x = |i: usize| i as f32 / n as f32;
        let tau = std::f32::consts::TAU;
        let mut seed = 0x2545_f491_u32;
        Self {
            saw: (0..n).map(|i| enth_ui::gallery::saw(x(i))).collect(),
            sine: (0..n).map(|i| (tau * x(i)).sin() * 0.8).collect(),
            noise: (0..n)
                .map(|_| {
                    seed ^= seed << 13;
                    seed ^= seed >> 17;
                    seed ^= seed << 5;
                    (seed as f32 / u32::MAX as f32 - 0.5) * 1.4
                })
                .collect(),
            spread: (0..n)
                .map(|i| {
                    (0..4)
                        .map(|v| enth_ui::gallery::saw((x(i) * (1.0 + 0.03 * v as f32)).fract()))
                        .sum::<f32>()
                        * 0.3
                })
                .collect(),
        }
    }
}

pub struct ReferencePanel {
    theme: Arc<Theme>,
    feed: Arc<AudioFeed>,
    transport: Option<Arc<AtomicBool>>,
    mode: usize,
    /// Lane cards (indices into `layout::LANE`), top to bottom.
    lane_order: [usize; 3],
    /// Where each lane card was last frame (by `layout::LANE` index).
    lane_rects: [Rect; 3],
    enabled: [bool; layout::GENERATORS.len() + layout::LANE.len()],
    matrix: [f32; layout::TARGETS.len() * layout::SOURCES.len()],
    scope: Vec<f32>,
    analyser: Analyser,
    tables: Tables,
    seen: u64,
    figure: Option<TextureHandle>,
    hover: Option<Hover>,
}

impl ReferencePanel {
    pub fn new(theme: Theme, feed: Arc<AudioFeed>) -> Self {
        Self {
            theme: Arc::new(theme),
            feed,
            transport: None,
            mode: LEARN,
            lane_order: [0, 1, 2],
            lane_rects: [Rect::NOTHING; 3],
            enabled: [true; layout::GENERATORS.len() + layout::LANE.len()],
            matrix: layout::matrix(),
            scope: vec![0.0; SCOPE_SAMPLES],
            analyser: Analyser::new(),
            tables: Tables::new(),
            seen: u64::MAX,
            figure: None,
            hover: None,
        }
    }

    /// Standalone only: shows an audio on/off switch bound to `transport`.
    #[must_use]
    pub fn with_transport(mut self, transport: Arc<AtomicBool>) -> Self {
        self.transport = Some(transport);
        self
    }

    pub fn theme(&self) -> &Theme {
        &self.theme
    }

    /// Lane order (indices into `layout::LANE`), for tests.
    pub fn lane_order(&self) -> [usize; 3] {
        self.lane_order
    }

    /// Lays out the whole panel in `ui`'s rect and schedules the next frame.
    pub fn ui(&mut self, ui: &mut Ui, params: &mut dyn HostParams) -> Repaint {
        let theme = Arc::clone(&self.theme);
        let t = &*theme;
        let generation = self.feed.generation();
        let fresh = generation != self.seen;
        self.seen = generation;
        if fresh {
            self.feed.copy_latest(&mut self.scope);
            self.analyser.analyse(&self.scope);
        }
        let hidden = ui.input(|i| {
            let v = i.viewport();
            v.minimized == Some(true) || v.occluded == Some(true)
        });
        let lfo = if hidden { None } else { self.feed.lfo() };
        self.hover = None;
        if self.figure.is_none() {
            self.figure = Some(lesson::load_figure(ui.ctx(), t));
        }

        let r = Regions::new(t, ui.max_rect());
        ui.painter()
            .rect_filled(ui.max_rect(), 0.0, t.color.surface.background);
        self.header(ui, t, r.header);
        self.macro_bar(ui, t, r.macros, params);
        self.lesson(ui, t, r.lesson);
        self.matrix_card(ui, t, r.matrix);
        let rings = self.generators(ui, t, r.generators, params, lfo)
            | self.lane(ui, t, r.lane, params, lfo);
        let (tiles, scope_visible) = self.strip(ui, t, r.strip, lfo);

        let repaint = if hidden {
            Repaint::Idle
        } else if scope_visible && fresh {
            Repaint::Scope
        } else if lfo.is_some() && (rings || tiles) {
            Repaint::Animated
        } else {
            Repaint::Idle
        };
        let ctx = ui.ctx();
        match repaint {
            Repaint::Scope => {
                self.feed.awake();
                ctx.request_repaint_after(t.motion.scope_interval());
            }
            Repaint::Animated => {
                self.feed.awake();
                ctx.request_repaint_after(t.motion.animated_interval());
            }
            // Hidden: ask for nothing; showing the window again brings a frame by itself.
            Repaint::Idle if hidden => self.feed.awake(),
            Repaint::Idle => {
                if self.feed.sleep(generation) {
                    ctx.request_repaint();
                }
            }
        }
        self.status(ui, t, r.status, repaint);
        repaint
    }

    fn header(&mut self, ui: &mut Ui, t: &Theme, rect: Rect) {
        ui.painter()
            .rect_filled(rect, t.radius.medium, t.color.surface.card);
        let mut ui = child(
            ui,
            rect.shrink2(vec2(t.space.pad, 0.0)),
            Layout::left_to_right(Align::Center),
        );
        ui.spacing_mut().item_spacing.x = t.space.pad;
        ui.label(
            egui::RichText::new("enth")
                .font(t.text.title.clone())
                .color(t.color.text.primary),
        );
        ui.add(Segmented::new(t, "mode", &mut self.mode, &MODES));
        ui.painter().text(
            rect.center(),
            Align2::CENTER_CENTER,
            "Glass Pluck",
            t.text.body.clone(),
            t.color.text.primary,
        );
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(
                egui::RichText::new("UI 100 %")
                    .font(t.text.label.clone())
                    .color(t.color.text.label),
            );
            if let Some(transport) = &self.transport {
                let mut on = usize::from(transport.load(Ordering::Relaxed));
                if ui
                    .add(Segmented::new(t, "transport", &mut on, &TRANSPORT))
                    .changed()
                {
                    transport.store(on == 1, Ordering::Relaxed);
                }
            }
        });
    }

    fn macro_bar(&mut self, ui: &mut Ui, t: &Theme, rect: Rect, params: &mut dyn HostParams) {
        let mut values: [f32; layout::MACROS.len()] =
            std::array::from_fn(|i| params.get(ParamId::Macro(i)));
        let before = values;
        let mut macros: Vec<Macro<'_>> = values
            .iter_mut()
            .zip(layout::MACROS.iter())
            .map(|(value, spec)| {
                let value_text = format!("{:.0} %", *value * 100.0);
                Macro {
                    name: spec.name,
                    value,
                    value_text,
                }
            })
            .collect();
        let mut ui = child(ui, rect, Layout::top_down(Align::Min));
        let responses = MacroBar::new(t).show(&mut ui, &mut macros);
        drop(macros);
        for (i, response) in responses.iter().enumerate() {
            let id = ParamId::Macro(i);
            gesture(params, id, response, values[i]);
            if response.hovered() {
                self.hover = Some(Hover {
                    path: format!("Macro {} {} {}", i + 1, glyph::PATH, layout::MACROS[i].name),
                    value: format!("{:.0} %", before[i] * 100.0),
                    modulation: String::new(),
                });
            }
        }
    }

    fn lesson(&mut self, ui: &mut Ui, t: &Theme, rect: Rect) {
        let Some(figure) = &self.figure else { return };
        let blocks = lesson::blocks(figure.id());
        let mut ui = child(ui, rect, Layout::top_down(Align::Min));
        LessonPane::new(t, "lesson", lesson::HEADER, &blocks)
            .action("Start challenge")
            .show(&mut ui);
    }

    fn matrix_card(&mut self, ui: &mut Ui, t: &Theme, rect: Rect) {
        ui.painter()
            .rect_filled(rect, t.radius.medium, t.color.surface.card);
        let mut ui = child(ui, rect.shrink(t.space.gap), Layout::top_down(Align::Min));
        caps_header(&mut ui, t, "Matrix");
        MatrixGrid::new(
            t,
            "matrix",
            &layout::SOURCES,
            &layout::TARGETS,
            &mut self.matrix,
        )
        .show(&mut ui);
    }

    /// The generator column; returns whether an animated knob is visible.
    fn generators(
        &mut self,
        ui: &mut Ui,
        t: &Theme,
        rect: Rect,
        params: &mut dyn HostParams,
        lfo: Option<f32>,
    ) -> bool {
        let mut ui = child(ui, rect, Layout::top_down(Align::Min));
        let mut animated = false;
        let (enabled, hover, tables) = (&mut self.enabled, &mut self.hover, &self.tables);
        egui::ScrollArea::vertical()
            .id_salt("generators")
            .auto_shrink(false)
            .show(&mut ui, |ui| {
                ui.spacing_mut().item_spacing.y = t.space.gap;
                for (index, module) in layout::GENERATORS.iter().enumerate() {
                    let values = module_values(module, params);
                    ModuleCard::new(
                        t,
                        module.id,
                        module.title,
                        module.category,
                        &mut enabled[index],
                    )
                    .kind(module.kind)
                    .show(
                        ui,
                        |ui, rect| figure(ui, t, tables, module.figure, &values, rect),
                        |ui| {
                            for i in 0..module.params.len() {
                                animated |= knob(ui, t, module, i, params, lfo, hover);
                            }
                        },
                    );
                }
            });
        animated
    }

    /// The effect lane: three stacked cards, reordered by dragging their headers.
    fn lane(
        &mut self,
        ui: &mut Ui,
        t: &Theme,
        rect: Rect,
        params: &mut dyn HostParams,
        lfo: Option<f32>,
    ) -> bool {
        let header =
            Rect::from_min_size(rect.min, vec2(rect.width(), t.size.card.collapsed_height));
        let footer = Rect::from_min_max(
            pos2(rect.left(), rect.bottom() - t.size.card.collapsed_height),
            rect.max,
        );
        let painter = ui.painter();
        painter.text(
            header.left_center(),
            Align2::LEFT_CENTER,
            "Lane 1",
            t.text.title.clone(),
            t.color.text.primary,
        );
        painter.text(
            footer.left_center(),
            Align2::LEFT_CENTER,
            format!("Send to {} Master", glyph::SEND),
            t.text.label.clone(),
            t.color.text.label,
        );
        let cards = Rect::from_min_max(
            pos2(rect.left(), header.bottom() + t.space.tight),
            pos2(rect.right(), footer.top() - t.space.tight),
        );
        let mut ui = child(ui, cards, Layout::top_down(Align::Min));
        let mut animated = false;
        let mut dragged = None;
        let pointer = ui.ctx().pointer_interact_pos();
        let order = self.lane_order;
        let (enabled, hover, tables, rects) = (
            &mut self.enabled,
            &mut self.hover,
            &self.tables,
            &mut self.lane_rects,
        );
        egui::ScrollArea::vertical()
            .id_salt("lane")
            .auto_shrink(false)
            .show(&mut ui, |ui| {
                ui.spacing_mut().item_spacing.y = t.space.gap;
                for &index in &order {
                    let module = &layout::LANE[index];
                    let values = module_values(module, params);
                    let enabled = &mut enabled[layout::GENERATORS.len() + index];
                    let lifted = ui
                        .ctx()
                        .is_being_dragged(ui.make_persistent_id(module.id).with("header"));
                    let card =
                        ModuleCard::new(t, module.id, module.title, module.category, enabled)
                            .kind(module.kind)
                            .stacked(t.size.card.lane_display_height)
                            .lifted(lifted)
                            .show(
                                ui,
                                |ui, rect| figure(ui, t, tables, module.figure, &values, rect),
                                |ui| {
                                    for i in 0..module.params.len() {
                                        animated |= knob(ui, t, module, i, params, lfo, hover);
                                    }
                                },
                            );
                    rects[index] = card.rect;
                    if card.header.dragged() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                        dragged = Some(index);
                    } else if card.header.hovered() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
                    }
                }
            });
        if let (Some(index), Some(pointer)) = (dragged, pointer) {
            self.reorder(index, pointer.y);
        }
        animated
    }

    /// Moves the dragged lane card past a neighbour once the pointer crosses its middle.
    fn reorder(&mut self, index: usize, y: f32) {
        let Some(at) = self.lane_order.iter().position(|&i| i == index) else {
            return;
        };
        if at > 0 && y < self.lane_rects[self.lane_order[at - 1]].center().y {
            self.lane_order.swap(at, at - 1);
        } else if at + 1 < self.lane_order.len()
            && y > self.lane_rects[self.lane_order[at + 1]].center().y
        {
            self.lane_order.swap(at, at + 1);
        }
    }

    /// Source tiles, scope and spectrum. Returns (animated tile visible, scope visible).
    fn strip(&mut self, ui: &mut Ui, t: &Theme, rect: Rect, lfo: Option<f32>) -> (bool, bool) {
        let tiles_width = 4.0 * t.size.tile.width + 3.0 * t.space.tight;
        let tiles = Rect::from_min_size(rect.min, vec2(tiles_width, rect.height()));
        let displays = Rect::from_min_max(pos2(tiles.right() + t.space.gap, rect.top()), rect.max);
        let half = (displays.width() - t.space.gap) / 2.0;
        let scope = Rect::from_min_size(displays.min, vec2(half, displays.height()));
        let spectrum = Rect::from_min_max(
            pos2(scope.right() + t.space.gap, displays.top()),
            displays.max,
        );

        let mut ui_tiles = child(ui, tiles, Layout::top_down(Align::Min));
        caps_header(&mut ui_tiles, t, "Sources");
        let routes = |source: usize| {
            self.matrix
                .chunks(layout::SOURCES.len())
                .filter(|row| row.get(source).is_some_and(|&a| a != 0.0))
                .count()
        };
        let phase = |ratio: f32| lfo.map(|p| (p * ratio).fract());
        let shapes: [TileSpec<'_>; 8] = [
            (
                "LFO 1",
                &enth_ui::gallery::sine_shape,
                phase(1.0),
                routes(1),
            ),
            ("LFO 2", &triangle, phase(2.0), routes(2)),
            ("LFO 3", &ramp_down, phase(0.5), routes(3)),
            ("Env 2", &enth_ui::gallery::envelope, None, routes(0)),
            ("Vel", &ramp_up, None, routes(4)),
            ("Key", &ramp_up, None, routes(5)),
            ("Wheel", &flat, None, 0),
            ("Random", &steps, None, 0),
        ];
        let mut animated = false;
        for row in shapes.chunks(4) {
            ui_tiles.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = t.space.tight;
                for &(name, shape, phase, routes) in row {
                    let response = SourceTile::new(t, name, name, shape)
                        .phase(phase)
                        .routes(routes)
                        .show(ui);
                    animated |= phase.is_some() && ui.is_rect_visible(response.rect);
                }
            });
        }
        let scope_visible = ui.is_rect_visible(scope) || ui.is_rect_visible(spectrum);
        Display::new(t, Trace::Scope(&self.scope), scope.size())
            .caption("Scope")
            .paint(ui, scope);
        Display::new(t, Trace::Spectrum(&self.analyser.levels), spectrum.size())
            .caption("Spectrum")
            .paint(ui, spectrum);
        (animated, scope_visible)
    }

    fn status(&self, ui: &mut Ui, t: &Theme, rect: Rect, repaint: Repaint) {
        let state = match repaint {
            Repaint::Idle => "idle".to_owned(),
            Repaint::Animated => format!("animating · {:.0} Hz", t.motion.animated_hz),
            Repaint::Scope => format!("scope · {:.0} Hz", t.motion.scope_hz),
        };
        let mut ui = child(ui, rect, Layout::top_down(Align::Min));
        let line = StatusLine::new(t).right(&state);
        match &self.hover {
            Some(h) => line.path(&h.path).value(&h.value).modulation(&h.modulation),
            None => line.path("Hover a control to see its value and modulation"),
        }
        .show(&mut ui);
    }
}

/// The panel's regions, from the window rect and the tokens.
struct Regions {
    header: Rect,
    macros: Rect,
    lesson: Rect,
    matrix: Rect,
    generators: Rect,
    lane: Rect,
    strip: Rect,
    status: Rect,
}

impl Regions {
    fn new(t: &Theme, window: Rect) -> Self {
        let gap = t.space.gap;
        let area = window.shrink(gap);
        let band = |top: f32, height: f32| {
            Rect::from_min_size(pos2(area.left(), top), vec2(area.width(), height))
        };
        let header = band(area.top(), t.size.bar.header);
        let macros = band(header.bottom() + gap, t.size.bar.macros);
        let status = band(area.bottom() - t.size.bar.status, t.size.bar.status);
        let body = Rect::from_min_max(
            pos2(area.left(), macros.bottom() + gap),
            pos2(area.right(), status.top() - gap),
        );
        let left = Rect::from_min_size(body.min, vec2(t.size.pane.lesson_width, body.height()));
        let m = &t.size.matrix;
        let matrix_height = t.size.card.collapsed_height
            + m.row_height * (layout::TARGETS.len() + 1) as f32
            + 2.0 * gap;
        let matrix = Rect::from_min_max(pos2(left.left(), left.bottom() - matrix_height), left.max);
        let lesson = Rect::from_min_max(left.min, pos2(left.right(), matrix.top() - gap));
        let right = Rect::from_min_max(pos2(left.right() + gap, body.top()), body.max);
        let strip = Rect::from_min_max(
            pos2(right.left(), right.bottom() - t.size.bar.modulator_strip),
            right.max,
        );
        let top = Rect::from_min_max(right.min, pos2(right.right(), strip.top() - gap));
        let lane = Rect::from_min_max(
            pos2(top.right() - t.size.card.lane_width, top.top()),
            top.max,
        );
        let generators = Rect::from_min_max(top.min, pos2(lane.left() - gap, top.bottom()));
        Self {
            header,
            macros,
            lesson,
            matrix,
            generators,
            lane,
            strip,
            status,
        }
    }
}

/// A child `Ui` covering `rect`.
fn child(ui: &mut Ui, rect: Rect, layout: Layout) -> Ui {
    ui.new_child(UiBuilder::new().max_rect(rect).layout(layout))
}

/// One of the few caps section headers (design-system: at most three).
fn caps_header(ui: &mut Ui, t: &Theme, text: &str) {
    ui.label(
        egui::RichText::new(text.to_uppercase())
            .font(t.text.label.clone())
            .color(t.color.text.label),
    );
}

/// Host gestures for a knob response: begin/set/end around a drag, a single set otherwise.
fn gesture(params: &mut dyn HostParams, id: ParamId, response: &Response, value: f32) {
    if response.drag_started() {
        params.begin(id);
    }
    if response.changed() {
        if response.dragged() || response.drag_stopped() {
            params.set(id, value);
        } else {
            params.begin(id);
            params.set(id, value);
            params.end(id);
        }
    }
    if response.drag_stopped() {
        params.end(id);
    }
}

/// A knob bound to slot `module.first_slot + index`. Returns whether it animates and is visible.
fn knob(
    ui: &mut Ui,
    t: &Theme,
    module: &Module,
    index: usize,
    params: &mut dyn HostParams,
    lfo: Option<f32>,
    hover: &mut Option<Hover>,
) -> bool {
    let param = &module.params[index];
    let id = ParamId::Slot(module.first_slot + index);
    let mut value = params.get(id);
    let text = param.unit.format(value);
    let modulated = lfo
        .filter(|_| param.lfo_depth != 0.0)
        .map(|phase| (value + param.lfo_depth * layout::lfo_wave(phase)).clamp(0.0, 1.0));
    let mut widget = Knob::new(t, &mut value, param.name)
        .value_text(text)
        .default_value(param.default)
        .bipolar(param.bipolar);
    if let Some(m) = modulated {
        widget = widget.modulated(m);
    }
    let response = ui.add(widget);
    gesture(params, id, &response, value);
    if response.hovered() {
        *hover = Some(Hover {
            path: format!("{} {} {}", module.title, glyph::PATH, param.name),
            value: param.unit.format(value),
            modulation: if param.lfo_depth == 0.0 {
                String::new()
            } else {
                format!("LFO 1 {:+.0} %", param.lfo_depth * 100.0)
            },
        });
    }
    modulated.is_some() && ui.is_rect_visible(response.rect)
}

/// The module's current parameter values (normalised), for its display.
fn module_values(module: &Module, params: &dyn HostParams) -> Vec<f32> {
    (0..module.params.len())
        .map(|i| params.get(ParamId::Slot(module.first_slot + i)))
        .collect()
}

/// Paints a module's display from its parameter values.
fn figure(ui: &mut Ui, t: &Theme, tables: &Tables, figure: Figure, v: &[f32], rect: Rect) {
    let at = |i: usize| v.get(i).copied().unwrap_or_default();
    let display = |trace| Display::new(t, trace, rect.size());
    match figure {
        Figure::Saw => display(Trace::Wave(&tables.saw)).paint(ui, rect),
        Figure::Sine => display(Trace::Wave(&tables.sine)).paint(ui, rect),
        Figure::Noise => display(Trace::Wave(&tables.noise)).paint(ui, rect),
        Figure::Spread => display(Trace::Wave(&tables.spread)).paint(ui, rect),
        Figure::Pulse => {
            let width = at(3).clamp(0.05, 0.95);
            let pulse = move |x: f32| if x.fract() < width { 0.85 } else { 0.15 };
            display(Trace::Curve(&pulse)).paint(ui, rect);
        }
        Figure::LowPass => {
            let (cutoff, resonance) = (at(0), at(1));
            let response = move |x: f32| low_pass(x, cutoff, resonance);
            display(Trace::Curve(&response)).paint(ui, rect);
        }
        Figure::Envelope => {
            let (a, d, s, r) = (at(0), at(1), at(2), at(3));
            let env = move |x: f32| adsr(x, a, d, s, r);
            display(Trace::Curve(&env))
                .ink(Ink::Modulation)
                .paint(ui, rect);
        }
        Figure::Shaper => {
            let drive = 1.0 + at(0) * 8.0;
            let shape = move |x: f32| 0.5 + 0.5 * (drive * (2.0 * x - 1.0)).tanh() / drive.tanh();
            display(Trace::Curve(&shape)).paint(ui, rect);
        }
        Figure::Echoes => {
            let feedback = at(1);
            let echoes = move |x: f32| {
                let tap = (x * 6.0).fract();
                let n = (x * 6.0).floor();
                if tap < 0.08 {
                    0.9 * feedback.max(0.05).powf(n)
                } else {
                    0.0
                }
            };
            display(Trace::Curve(&echoes)).paint(ui, rect);
        }
        Figure::Decay => {
            let decay = 0.5 + at(1) * 4.0;
            let tail = move |x: f32| (-(x * 8.0) / decay).exp() * 0.9;
            display(Trace::Curve(&tail)).paint(ui, rect);
        }
    }
}

/// A resonant low-pass magnitude on a log-frequency axis, 0..1.
fn low_pass(x: f32, cutoff: f32, resonance: f32) -> f32 {
    let d = x - cutoff;
    let bump = resonance * 0.5 * (-(d / 0.04).powi(2)).exp();
    let fall = if d > 0.0 { (d * 4.0).powi(2) } else { 0.0 };
    (0.6 + bump - fall).clamp(0.0, 1.0)
}

/// An ADSR shape over 0..1 with the four stage lengths scaled into it.
fn adsr(x: f32, a: f32, d: f32, s: f32, r: f32) -> f32 {
    let (a, d, r) = (0.05 + a * 0.25, 0.05 + d * 0.25, 0.05 + r * 0.25);
    let hold = 1.0 - a - d - r;
    match x {
        x if x < a => x / a,
        x if x < a + d => 1.0 - (1.0 - s) * (x - a) / d,
        x if x < a + d + hold => s,
        x => s * (1.0 - (x - a - d - hold) / r).max(0.0),
    }
}

fn triangle(x: f32) -> f32 {
    1.0 - (2.0 * x - 1.0).abs()
}

fn ramp_up(x: f32) -> f32 {
    x
}

fn ramp_down(x: f32) -> f32 {
    1.0 - x
}

fn flat(_: f32) -> f32 {
    0.5
}

fn steps(x: f32) -> f32 {
    [0.3, 0.8, 0.5, 0.15, 0.65, 0.4][((x * 6.0) as usize).min(5)]
}
