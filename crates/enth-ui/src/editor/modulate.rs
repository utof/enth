//! Drag-to-modulate (lane ED2; the layout handoff's §4): drag a source's ⊕ handle onto a control to route it, then
//! drag the control's orange depth handle to set how much.
//!
//! - **The drag** is egui's drag and drop. A source tile's ⊕ handle sets a [`SourceDrag`] payload when its drag
//!   starts (`regions/tiles.rs`) and draws a wire to the pointer while it lasts.
//! - **Targets.** While a payload is up, the editor checks every parameter against it once per frame
//!   ([`check`], into `ParamView::drop`): a modulatable parameter the source may drive is a target (an orange
//!   outline, filled while the pointer is over it); one the source already drives is outlined faintly; anything else is
//!   **refused, visibly**: drawn dimmed, and hovering it names the reason on the status line with a not-allowed cursor.
//!   A per-voice source (an envelope, a voice LFO, velocity, key) never drives a global destination (an effect, a
//!   global LFO, `master.*`): the patch refuses that route (`route_rate_mismatch`), so the editor refuses the drop.
//!   Every parameter control is drawn by `cards::control`, so this happens in one place for the cards and the header.
//! - **The drop** pushes [`Intent::Route`]: one `InsertRoute` at [`NEW_ROUTE_DEPTH`], one edit, one undo step.
//! - **The depth handle** ([`depth_handle`]): an orange dot on the knob's modulation ring at the end of its newest
//!   route's depth (`value + depth`). Dragging it vertically is one gesture of `SetDepth`s on that route (one undo
//!   step), which the session sends as depth events: the engine ramps each over 10 ms, and nothing recompiles unless
//!   the depth goes to or from 0. Other routes into the knob are edited in the overview; any route is removed from the
//!   control's right-click menu ("Remove … modulation"). An audio input (S2a: `wt`'s `pm` and `am`) takes a generator's
//!   output through an audio route, not a modulation source: it refuses a drop with that reason and has no handle (lane
//!   SKS); its routes' depths are edited in the overview and removed from the same menu ("Remove … audio route").

use egui::{CursorIcon, Id, LayerId, Order, Pos2, Rect, Response, Sense, Ui, Vec2, WidgetInfo};
use enth_params::{DEPTH, ParamSpec};
use enth_patch::model::MAX_ROUTES;
use enth_patch::validate::{self, closes_audio_cycle};
use enth_patch::{ModSource, ParamAddr, Patch, Rate, Route, RouteAddr, Target};

use super::cards::format::signed_percent;
use super::cards::view::source_name;
use super::cards::{CardView, Grip, Hovered, Intent, Intents, ParamView, ids};
use crate::Theme;
use crate::widgets::Knob;

/// The depth a route made by a drop starts at, in percent of the destination's range: audible at once, and the depth
/// handle then sets it (`enth_patch::modulation::DEFAULT_ROUTE_DEPTH`).
pub use enth_patch::modulation::DEFAULT_ROUTE_DEPTH as NEW_ROUTE_DEPTH;

/// The depth an **audio route** made by a drop starts at, in percent of the source's output (lane EDS): the full
/// output, so the carrier's own amount control (`wt`'s `pm_index`, the AM gain `am`) sets how much, as in every S2a
/// template (`content/templates/s2a`). Not [`NEW_ROUTE_DEPTH`]: that is a share of a *parameter's range*, which an
/// audio input does not have.
pub const AUDIO_ROUTE_DEPTH: f32 = 100.0;

/// The depth the route `src → dst` starts at when a drop makes it: [`AUDIO_ROUTE_DEPTH`] for an audio route (a
/// generator into an audio input), else [`NEW_ROUTE_DEPTH`].
pub fn new_route_depth(patch: &Patch, route: &RouteAddr) -> f32 {
    let audio = Route {
        src: route.src.clone(),
        dst: route.dst.clone(),
        depth: AUDIO_ROUTE_DEPTH,
    };
    if validate::is_audio_route(patch, &audio) {
        AUDIO_ROUTE_DEPTH
    } else {
        NEW_ROUTE_DEPTH
    }
}

/// The payload of a source's ⊕ handle while it is dragged: a modulator's, or (lane EDS) a generator's audio output.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceDrag {
    pub src: ModSource,
    /// Its short name ("Env 2"), for the status line.
    pub name: String,
}

/// What dropping the dragged source on a parameter would do.
#[derive(Clone, Debug, PartialEq)]
pub enum DropCheck {
    /// The drop creates the route.
    Route,
    /// The source already drives it, by this depth (percent); the drop changes nothing.
    Routed(f32),
    /// Not a target; the reason, for the status line.
    Refused(String),
}

/// The title of the module (or built-in) an address belongs to: "Filter 1", "Voice", "Master".
fn target_title(patch: &Patch, addr: &ParamAddr) -> String {
    match addr.target() {
        Target::Module(id) => source_name(patch, &ModSource::Module(id.clone())),
        Target::Voice => "Voice".to_owned(),
        Target::Master => "Master".to_owned(),
    }
}

/// Whether `src` may drive `addr` in `patch`, and if not, why (the patch's own route rules, `enth-patch::validate`).
pub fn check(patch: &Patch, src: &ModSource, addr: &ParamAddr, spec: &ParamSpec) -> DropCheck {
    let source = source_name(patch, src);
    if let Some(r) = patch
        .routes
        .iter()
        .find(|r| &r.src == src && &r.dst == addr)
    {
        return DropCheck::Routed(r.depth);
    }
    if validate::is_generator_source(patch, src) {
        return check_audio(patch, src, addr, spec, &source);
    }
    if spec.flags.audio {
        // An audio input takes a generator's output through an audio route (S2a): a modulation source is refused
        // (a generator's ⊕ handle makes the route, lane EDS).
        return DropCheck::Refused(format!(
            "{} takes the audio of a generator, not a modulation source",
            spec.name
        ));
    }
    if !spec.flags.modulatable {
        return DropCheck::Refused(format!("{} cannot be modulated", spec.name));
    }
    if let (Some(Rate::Voice), Some(Rate::Global)) =
        (patch.source_rate(src), patch.target_rate(addr.target()))
    {
        return DropCheck::Refused(format!(
            "{source} runs per voice and {} is global: only a global LFO, a macro or the mod wheel can modulate it",
            target_title(patch, addr)
        ));
    }
    if patch.routes.len() >= MAX_ROUTES {
        return DropCheck::Refused(format!(
            "A patch holds at most {MAX_ROUTES} routes; remove one first"
        ));
    }
    DropCheck::Route
}

/// What dropping the generator `src` on `addr` would do (lane EDS): a generator's audio goes only to an audio input of
/// a generator (`wt`'s `pm` and `am`), and never in a loop (the patch's own rules, `enth_patch::validate`).
fn check_audio(
    patch: &Patch,
    src: &ModSource,
    addr: &ParamAddr,
    spec: &ParamSpec,
    source: &str,
) -> DropCheck {
    if !spec.flags.audio || !validate::is_generator_dest(patch, addr) {
        return DropCheck::Refused(format!(
            "{source} is a generator: its audio can drive only an audio input (a wavetable's PM or AM), not {}",
            spec.name
        ));
    }
    let route = Route {
        src: src.clone(),
        dst: addr.clone(),
        depth: AUDIO_ROUTE_DEPTH,
    };
    let target = target_title(patch, addr);
    if closes_audio_cycle(patch, &route) {
        let itself =
            matches!(addr.target(), Target::Module(id) if *src == ModSource::Module(id.clone()));
        return DropCheck::Refused(if itself {
            format!("{source} cannot drive itself: a loop inside {target} is its Feedback control")
        } else {
            format!("{target} already drives {source}: audio flows one way, never in a loop")
        });
    }
    if validate::audio_into_sync_master(patch, &route) {
        return DropCheck::Refused(format!(
            "{source} comes after the hard-sync master, which has to run first: it cannot feed it"
        ));
    }
    if patch.routes.len() >= MAX_ROUTES {
        return DropCheck::Refused(format!(
            "A patch holds at most {MAX_ROUTES} routes; remove one first"
        ));
    }
    DropCheck::Route
}

/// Whether the control is refused as a drop target this frame (the caller draws it dimmed).
pub(crate) fn refused(p: &ParamView) -> bool {
    matches!(p.drop, Some(DropCheck::Refused(_)))
}

/// A parameter control as a drop target, after it is drawn: its outline, the status line's reading and the cursor
/// while the payload is over it, and the route intent when it is released there.
pub(crate) fn target(
    ui: &Ui,
    t: &Theme,
    view: &CardView,
    p: &ParamView,
    response: &Response,
    out: &mut Intents,
) {
    let Some(check) = &p.drop else { return };
    let c = &t.color;
    let over = response.dnd_hover_payload::<SourceDrag>();
    let rect = response.rect;
    let painter = ui.painter();
    match check {
        DropCheck::Route => {
            if over.is_some() {
                painter.rect_filled(rect, t.radius.small, c.accent.modulation_wash);
            }
            let stroke = if over.is_some() {
                t.stroke.thick(c.accent.modulation)
            } else {
                t.stroke.thin(c.accent.modulation)
            };
            painter.rect_stroke(rect, t.radius.small, stroke, egui::StrokeKind::Inside);
        }
        DropCheck::Routed(_) => {
            painter.rect_stroke(
                rect,
                t.radius.small,
                t.stroke.thin(c.accent.modulation_wash),
                egui::StrokeKind::Inside,
            );
        }
        DropCheck::Refused(_) => {}
    }
    if let Some(drag) = &over {
        let name = &drag.name;
        let modulation = match check {
            // An audio input (lane EDS): the generator's audio goes in at the audio depth, not a share of a range.
            DropCheck::Route if p.spec.flags.audio => format!(
                "Drop to feed it the audio of {name} at {}",
                signed_percent(AUDIO_ROUTE_DEPTH)
            ),
            DropCheck::Route => format!(
                "Drop to modulate it from {name} by {}",
                signed_percent(NEW_ROUTE_DEPTH)
            ),
            DropCheck::Routed(depth) if p.spec.flags.audio => {
                format!("{name} already feeds it ({})", signed_percent(*depth))
            }
            DropCheck::Routed(depth) => {
                format!("{name} already modulates it ({})", signed_percent(*depth))
            }
            DropCheck::Refused(reason) => {
                ui.ctx().set_cursor_icon(CursorIcon::NotAllowed);
                reason.clone()
            }
        };
        out.hovered = Some(Hovered {
            path: view.path(p.spec),
            value: String::new(),
            modulation,
        });
    }
    if let Some(drag) = response.dnd_release_payload::<SourceDrag>()
        && *check == DropCheck::Route
    {
        out.push(Intent::Route(RouteAddr {
            src: drag.src.clone(),
            dst: p.addr.clone(),
        }));
    }
}

/// The id of a knob's depth handle.
pub fn depth_id(addr: &ParamAddr) -> Id {
    ids::param(addr).with("depth")
}

/// A knob's depth handle (module docs): drawn on the knob's modulation ring at `value + depth` of its newest route;
/// dragging it vertically is one gesture of `SetDepth`s on that route. `knob` is the knob's response, `inline` its
/// form, `label` its label (the handle is named "<label> depth from <source>").
#[allow(
    clippy::too_many_arguments,
    reason = "the knob's whole context: where, theme, card, parameter, its response, form and label, intents"
)]
pub(crate) fn depth_handle(
    ui: &mut Ui,
    t: &Theme,
    view: &CardView,
    p: &ParamView,
    knob: &Response,
    inline: bool,
    label: &str,
    out: &mut Intents,
) {
    let Some(route) = p.routes.last() else { return };
    // An audio route's depth is a share of its source's output, not a distance along the knob's range: its handle
    // would sit at a meaningless place on the ring, so an audio input has none (the overview edits the depth).
    if p.spec.flags.audio {
        return;
    }
    let m = &t.size.modulation;
    let at = (p.norm() + route.depth / 100.0).clamp(0.0, 1.0);
    let center = Knob::ring_point(t, ui, knob.rect, inline, at);
    let rect = Rect::from_center_size(center, Vec2::splat(2.0 * m.depth_reach));
    let handle = ui.interact(rect, depth_id(&p.addr), Sense::drag());
    let depth = route.depth;
    handle.widget_info(|| {
        WidgetInfo::slider(
            true,
            f64::from(depth),
            format!("{label} depth from {}", route.name),
        )
    });
    let addr = RouteAddr {
        src: route.src.clone(),
        dst: p.addr.clone(),
    };
    if handle.drag_started() {
        out.push(Intent::Begin(Grip::Route(addr.clone())));
    }
    if handle.dragged() {
        let k = &t.size.knob;
        let fine = if ui.input(|i| i.modifiers.shift) {
            k.fine_ratio
        } else {
            1.0
        };
        let travel = -handle.drag_delta().y;
        if travel != 0.0 {
            let next = DEPTH.clamp(depth + travel / k.drag_travel * 100.0 * fine);
            if next.to_bits() != depth.to_bits() {
                out.push(Intent::SetDepth {
                    route: addr.clone(),
                    depth: next,
                });
            }
        }
    }
    if handle.drag_stopped() {
        out.push(Intent::End(Grip::Route(addr)));
    }
    let lit = handle.hovered() || handle.dragged();
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        if lit {
            painter.circle_filled(center, m.depth_reach, t.color.accent.modulation_wash);
        }
        painter.circle_filled(center, m.depth_handle, t.color.accent.modulation);
    }
    if lit {
        out.hovered = Some(Hovered {
            path: view.path(p.spec),
            value: format!("{} {}", route.name, signed_percent(depth)),
            modulation: "Drag to set the depth".to_owned(),
        });
    }
}

/// While a source's ⊕ is dragged: an orange wire from the handle to the pointer, over everything.
pub(crate) fn wire(ui: &Ui, t: &Theme, from: Pos2, id: Id) {
    let Some(to) = ui.ctx().pointer_latest_pos() else {
        return;
    };
    let painter = ui.ctx().layer_painter(LayerId::new(Order::Tooltip, id));
    painter.line_segment([from, to], t.stroke.thick(t.color.accent.modulation));
    painter.circle_filled(
        to,
        t.size.modulation.depth_handle,
        t.color.accent.modulation,
    );
}
