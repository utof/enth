//! The card presenter: one [`CardView`] drawn as a module card, with a control per parameter chosen from its spec.
//!
//! Everything a card draws runs inside a `Ui` whose scope id is the card's own ([`ids::card`]), and every control
//! interacts under the id of its address ([`ids::param`]), so a card drawn somewhere else next frame (another region,
//! another layout, another position in its section) keeps its drags and hovers.

use egui::{Rect, Response, Ui, UiBuilder};
use enth_patch::ModuleKind;
use enth_patch::catalogue::{FilterMode, LfoShape, NoiseColor, Wave, Wavetable};

use super::format::{signed_percent, value_text};
use super::intent::{Grip, Hovered, Intent, Intents};
use super::present::{self, Control};
use super::view::{CardKey, CardKind, CardView, ParamView};
use super::{envelope, ids, menu, shapes};
use crate::Theme;
use crate::editor::overlay::Emphasis;
use crate::editor::{add_module, modulate};
use crate::widgets::{
    CardShape, Display, Emphasis as KnobEmphasis, Ink, Knob, ModuleCard, Segmented, Toggle, Trace,
};

/// What a card did this frame, besides its intents.
pub struct CardOutput {
    /// The whole card.
    pub rect: Rect,
    /// The header row (drag it to reorder; ED1 decides what a header drag does). A right-click on it opens the
    /// module's menu (lane AM).
    pub header: Response,
    /// The "…" module menu button (lane AM); `None` for the built-ins.
    pub menu: Option<Response>,
}

/// Draws one card.
pub struct Card<'a> {
    theme: &'a Theme,
    view: &'a CardView,
    shape: CardShape,
    /// The card sits in a band, left to right (its menu's moves read "left" and "right").
    along_x: bool,
}

impl<'a> Card<'a> {
    /// A stacked (lane) card, the shape of effects and modulators.
    pub fn new(theme: &'a Theme, view: &'a CardView) -> Self {
        Self {
            theme,
            view,
            shape: CardShape::Stacked,
            along_x: false,
        }
    }

    /// The card sits in a band, left to right: its menu's moves read "Move left" and "Move right" (lane AM).
    #[must_use]
    pub fn along_x(mut self, along_x: bool) -> Self {
        self.along_x = along_x;
        self
    }

    /// A strip: the display left of one row of controls (two with "More" open), the full width of the `Ui`.
    #[must_use]
    pub fn strip(mut self) -> Self {
        self.shape = CardShape::Strip;
        self
    }

    /// A stacked card: the display above the controls, `size.card.lane_width` wide.
    #[must_use]
    pub fn stacked(mut self) -> Self {
        self.shape = CardShape::Stacked;
        self
    }

    /// Draws the card where the `Ui`'s layout puts it and records its intents (and its hovered control) in `out`.
    pub fn show(self, ui: &mut Ui, out: &mut Intents) -> CardOutput {
        let t = self.theme;
        let view = self.view;
        let id = ids::card(&view.key);
        ui.scope_builder(UiBuilder::new().scope_id(id), |ui| {
            let mut enabled = view.enabled.unwrap_or(true);
            let more = view
                .has_more()
                .then_some(if view.more_open { "Less" } else { "More" });
            let mut card = ModuleCard::new(t, "card", &view.title, view.category(), &mut enabled)
                .id(id)
                .switchable(view.enabled.is_some())
                .menu(view.place.is_some());
            if let Some(kind) = view.kind_text() {
                card = card.kind(kind);
            }
            if let Some(more) = more {
                card = card.action(more);
            }
            card = match self.shape {
                CardShape::Strip => card.rows(if view.more_open && view.has_more() {
                    2
                } else {
                    1
                }),
                CardShape::Stacked => card.stacked(display_height(t, view)),
            };
            let mut display_out = Intents::default();
            let mut params_out = Intents::default();
            let shape = self.shape;
            let response = card.show(
                ui,
                |ui, rect| display(ui, t, view, rect, &mut display_out),
                |ui| params(ui, t, view, shape, &mut params_out),
            );
            out.extend(display_out);
            out.extend(params_out);
            // A Recreate round's ring on the enable toggle (lane RUI): the toggle sits at the header row's left end.
            if let Some(score) = view.enable_score
                && view.enabled.is_some()
            {
                let side = t.size.card.toggle;
                let header = response.header.rect;
                let toggle = Rect::from_center_size(
                    egui::pos2(header.left() + side / 2.0, header.center().y),
                    egui::vec2(side, side),
                );
                crate::editor::overlay::paint_score(ui.painter(), t, toggle, score);
            }
            // A revealed hint (lane HINT): the module it names, or one of its controls that "More" hides now.
            if view.hinted || view.hint_behind_more() {
                crate::editor::overlay::paint_hint(
                    ui.painter(),
                    t,
                    response.rect,
                    crate::editor::overlay::Pointer::Here,
                );
            }
            if response.toggled
                && let CardKey::Module(module) = &view.key
            {
                out.push(Intent::Enable {
                    id: module.clone(),
                    enabled,
                });
            }
            if response.action.as_ref().is_some_and(Response::clicked) {
                out.push(Intent::ShowMore {
                    card: view.key.clone(),
                    open: !view.more_open,
                });
            }
            // Lane AM: the module menu (the "…" button, a right-click on the title row) and its "Rename…" field.
            add_module::card_menu(
                t,
                view,
                response.menu.as_ref(),
                &response.header,
                self.along_x,
                out,
            );
            add_module::rename_field(ui, t, view, response.header.rect, out);
            CardOutput {
                rect: response.rect,
                header: response.header,
                menu: response.menu,
            }
        })
        .inner
    }
}

/// The display's height in a stacked card: none for the built-ins, which have nothing to picture.
fn display_height(t: &Theme, view: &CardView) -> f32 {
    match view.kind {
        CardKind::Module(_) => t.size.card.lane_display_height,
        CardKind::Voice | CardKind::Master => 0.0,
    }
}

/// The card's display: a picture of the module's sound or shape from its real values; the envelope's is editable.
fn display(ui: &mut Ui, t: &Theme, view: &CardView, rect: Rect, out: &mut Intents) {
    let CardKind::Module(kind) = view.kind else {
        return;
    };
    let value = |key: &str| view.param(key).map_or(0.0, |p| p.value);
    let norm = |key: &str| view.param(key).map_or(0.0, ParamView::norm);
    let size = rect.size();
    match kind {
        ModuleKind::Osc => {
            let wave = shapes::osc(
                Wave::from_value(value("wave")),
                norm("pw"),
                Wavetable::from_value(value("table")),
                norm("position"),
            );
            Display::new(t, Trace::Wave(&wave), size).paint(ui, rect);
        }
        ModuleKind::Noise => {
            let noise = shapes::noise(NoiseColor::from_value(value("color")));
            Display::new(t, Trace::Wave(&noise), size).paint(ui, rect);
        }
        ModuleKind::Svf => {
            let (mode, cutoff, resonance) = (
                FilterMode::from_value(value("mode")),
                norm("cutoff"),
                norm("resonance"),
            );
            let curve = |x| shapes::filter(mode, cutoff, resonance, x);
            Display::new(t, Trace::Curve(&curve), size).paint(ui, rect);
        }
        ModuleKind::Adsr => {
            if let CardKey::Module(id) = &view.key {
                envelope::show(ui, t, view, id, rect, out);
            }
        }
        ModuleKind::Lfo => {
            let shape = LfoShape::from_value(value("shape"));
            let curve = |x| shapes::lfo(shape, x);
            Display::new(t, Trace::Curve(&curve), size)
                .ink(Ink::Modulation)
                .paint(ui, rect);
        }
        ModuleKind::Reverb => {
            let (room, decay, wet, early) =
                (value("room"), value("decay"), value("wet"), value("early"));
            let curve = |x| shapes::reverb(room, decay, wet, early, x);
            Display::new(t, Trace::Curve(&curve), size).paint(ui, rect);
        }
        ModuleKind::Comp => {
            let (threshold, ratio, knee, makeup) = (
                value("threshold"),
                value("ratio"),
                value("knee"),
                value("makeup"),
            );
            let curve = |x| shapes::compressor(threshold, ratio, knee, makeup, x);
            Display::new(t, Trace::Curve(&curve), size).paint(ui, rect);
        }
    }
}

/// How a control is shaped where it is drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Form {
    /// A knob cell: label above, the control on the knob's centre line, the value below.
    Cell,
    /// A row this wide (a lane card's enum): label on the left, the control at the right end.
    Row(f32),
    /// A knob with its label and value to its right, in a cell this wide (the header's built-ins). A card with one
    /// parameter (master) shows its title as the label.
    Inline(f32),
}

/// The controls: the shown tiers, then (with "More" open) a thin rule and the rest. A strip puts each group in one
/// row of cells. A lane card puts a group's enums first, one row each, then its cells in rows spread across the
/// card's width.
fn params(ui: &mut Ui, t: &Theme, view: &CardView, shape: CardShape, out: &mut Intents) {
    let shown: Vec<&ParamView> = view.shown().collect();
    let hidden: Vec<&ParamView> = if view.more_open {
        view.hidden().collect()
    } else {
        Vec::new()
    };
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(t.space.tight, t.space.tight);
        for (index, group) in [shown, hidden].into_iter().enumerate() {
            if group.is_empty() {
                continue;
            }
            if index > 0 && shape == CardShape::Stacked {
                rule(ui, t);
            }
            match shape {
                CardShape::Strip => {
                    ui.horizontal_top(|ui| {
                        for p in group {
                            control(ui, t, view, p, Form::Cell, out);
                        }
                    });
                }
                CardShape::Stacked => lane_group(ui, t, view, &group, out),
            }
        }
    });
}

/// One group of a lane card: its segmented enums as rows, then its cells spread over rows.
fn lane_group(ui: &mut Ui, t: &Theme, view: &CardView, group: &[&ParamView], out: &mut Intents) {
    let width = ui.available_width();
    let (rows, cells): (Vec<&ParamView>, Vec<&ParamView>) = group
        .iter()
        .partition(|p| present::control(t, p.spec) == Control::Segmented);
    for p in rows {
        control(ui, t, view, p, Form::Row(width), out);
    }
    // Whole cells per row, spread so the first and last touch the card's edges.
    let cell = t.size.knob.cell_width;
    let per_row = ((width + t.space.tight) / (cell + t.space.tight))
        .floor()
        .max(1.0);
    let spread = if per_row > 1.0 {
        (width - per_row * cell) / (per_row - 1.0)
    } else {
        0.0
    };
    for chunk in cells.chunks(per_row as usize) {
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = spread;
            for p in chunk {
                control(ui, t, view, p, Form::Cell, out);
            }
        });
    }
}

/// The thin rule between a lane card's shown and advanced controls.
fn rule(ui: &mut Ui, t: &Theme) {
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), t.space.tight),
        egui::Sense::hover(),
    );
    ui.painter().hline(
        rect.x_range(),
        rect.center().y,
        t.stroke.thin(t.color.line.grid),
    );
}

/// One parameter's control, and its intents: the control the spec asks for, its right-click menu ("Type value…",
/// "Reset to default", "Automate"), the "Type value…" field while it is open, and the status line's reading while
/// it is hovered. The overlay's emphasis applies: a hidden parameter is not drawn, a dimmed one is drawn in the
/// dimmed ink and stays editable.
pub(crate) fn control(
    ui: &mut Ui,
    t: &Theme,
    view: &CardView,
    p: &ParamView,
    form: Form,
    out: &mut Intents,
) {
    if p.emphasis == Emphasis::Hidden {
        return;
    }
    let label = match form {
        Form::Inline(_) if view.params.len() == 1 => view.title.as_str(),
        _ => present::label(view.kind, p.spec),
    };
    let dimmed = p.emphasis == Emphasis::Dimmed;
    // While a source's ⊕ is dragged, a control it cannot modulate draws dimmed (lane ED2, `editor::modulate`).
    let response = if modulate::refused(p) {
        ui.scope(|ui| {
            ui.multiply_opacity(t.state.disabled_opacity);
            control_body(ui, t, view, p, label, form, dimmed, out)
        })
        .inner
    } else {
        control_body(ui, t, view, p, label, form, dimmed, out)
    };
    // A score ring on a switch or a segmented control (a knob draws its own on its dial; lane RUI).
    if let Some(score) = p.score
        && matches!(
            present::control(t, p.spec),
            Control::Segmented | Control::Toggle
        )
    {
        crate::editor::overlay::paint_score(ui.painter(), t, response.rect, score);
    }
    // A revealed hint points here (lane HINT): an outline, and ↑ or ↓ when the hint says which way.
    if let Some(pointer) = p.hint {
        crate::editor::overlay::paint_hint(ui.painter(), t, response.rect, pointer);
    }
    modulate::target(ui, t, view, p, &response, out);
    menu::attach(t, view.kind, p, &response, out);
    menu::entry(ui, t, p, response.rect, out);
    if response.hovered() || response.dragged() {
        out.hovered = Some(hovered(view, p));
    }
}

/// The control the spec asks for, and its edits.
#[allow(
    clippy::too_many_arguments,
    reason = "the control's whole context: where, theme, card, parameter, label, form, dimming, intents"
)]
fn control_body(
    ui: &mut Ui,
    t: &Theme,
    view: &CardView,
    p: &ParamView,
    label: &str,
    form: Form,
    dimmed: bool,
    out: &mut Intents,
) -> Response {
    let id = ids::param(&p.addr);
    match present::control(t, p.spec) {
        Control::Knob | Control::Stepped(_) => knob(ui, t, view, p, label, form, out),
        Control::Segmented => {
            let items = p.spec.enum_items().unwrap_or(&[]);
            let labels: Vec<&str> = items
                .iter()
                .map(|i| present::item_label(view.kind, p.spec, i))
                .collect();
            let mut selected = p.value as usize;
            let mut segmented = Segmented::new(t, "segmented", &mut selected, &labels)
                .id(id)
                .label(label);
            if let Form::Row(width) | Form::Inline(width) = form {
                segmented = segmented.inline(width);
            }
            let response = ui
                .scope(|ui| {
                    if dimmed {
                        ui.multiply_opacity(t.state.disabled_opacity);
                    }
                    ui.add(segmented)
                })
                .inner;
            if response.changed() {
                out.push(Intent::Select {
                    addr: p.addr.clone(),
                    index: selected,
                });
            }
            response
        }
        Control::Toggle => {
            let mut on = p.value >= 0.5;
            let response = ui
                .scope(|ui| {
                    if dimmed {
                        ui.multiply_opacity(t.state.disabled_opacity);
                    }
                    ui.add(Toggle::new(t, &mut on, label).id(id))
                })
                .inner;
            if response.changed() {
                out.push(Intent::Toggle {
                    addr: p.addr.clone(),
                    on,
                });
            }
            response
        }
    }
}

/// A knob (stepped for integers and long enums): `Begin` when a drag starts, `Set` when the value it writes
/// changes, `End` when the drag stops. A double-click reset is a lone `Set`.
fn knob(
    ui: &mut Ui,
    t: &Theme,
    view: &CardView,
    p: &ParamView,
    label: &str,
    form: Form,
    out: &mut Intents,
) -> Response {
    let spec = p.spec;
    let mut norm = p.norm();
    let text = match spec.enum_item(p.value) {
        Some(item) if spec.enum_items().is_some() => {
            present::item_label(view.kind, spec, item).to_owned()
        }
        _ => value_text(spec, p.value),
    };
    let mut knob = Knob::new(t, &mut norm, label)
        .id(ids::param(&p.addr))
        .value_text(text)
        .default_value(spec.to_norm(spec.default))
        .bipolar(present::bipolar(spec))
        .score(p.score);
    if p.emphasis == Emphasis::Dimmed {
        knob = knob.emphasis(KnobEmphasis::Dimmed);
    }
    if let Form::Inline(width) = form {
        knob = knob.inline(width);
    }
    if let Control::Stepped(steps) = present::control(t, spec) {
        knob = knob.steps(steps);
    }
    if let Some(live) = p.live {
        knob = knob.modulated(live);
    }
    if let Some((lo, hi)) = p.range {
        knob = knob.range(lo, hi);
    }
    let response = ui.add(knob);
    if p.emphasis == Emphasis::Normal {
        modulate::depth_handle(
            ui,
            t,
            view,
            p,
            &response,
            matches!(form, Form::Inline(_)),
            label,
            out,
        );
    }
    let grip = || Grip::Param(p.addr.clone());
    if response.drag_started() {
        out.push(Intent::Begin(grip()));
    }
    if response.changed() {
        let value = spec.clamp(spec.from_norm(norm));
        if value.to_bits() != p.value.to_bits() {
            out.push(Intent::Set {
                addr: p.addr.clone(),
                value,
            });
        }
    }
    if response.drag_stopped() {
        out.push(Intent::End(grip()));
    }
    response
}

/// The status line's reading of a control: its path, its value, the routes into it with their depths, and the
/// automation slots that drive it.
fn hovered(view: &CardView, p: &ParamView) -> Hovered {
    let mut modulation: Vec<String> = p
        .routes
        .iter()
        .map(|r| format!("{} {}", r.name, signed_percent(r.depth)))
        .collect();
    modulation.extend(
        p.slots
            .iter()
            .map(|s| format!("automation slot {}", s.one_based())),
    );
    Hovered {
        path: view.path(p.spec),
        value: value_text(p.spec, p.value),
        modulation: modulation.join(", "),
    }
}
