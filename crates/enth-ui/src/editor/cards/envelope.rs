//! The envelope display: the ADSR curve with its three corners as handles, an alternative to the knobs that stays
//! in sync with them both ways because both read and write the same addresses.
//!
//! The handles move on the parameters' real scales: a stage's width follows its knob's normalised position through
//! the spec's own power-4 scale (`ParamSpec::to_norm`), so a drag across `size.cards.envelope_stage_range` of the
//! width sweeps the knob once, the point stays under the pointer, and the value it writes is the knob's
//! `from_norm`. The decay and sustain corner is one gesture over two addresses.

use egui::{Rect, Ui};
use enth_patch::ModuleId;

use super::format::value_text;
use super::ids;
use super::intent::{Grip, Hovered, Intent, Intents};
use super::shapes::EnvelopeStages;
use super::view::{CardView, ParamView};
use crate::Theme;
use crate::theme::glyph;
use crate::widgets::{CurveHandles, Display, Drag, Handle, Ink, Trace};

/// Which parameters (in attack, decay, sustain, release order) each handle drags.
const HANDLE_PARAMS: [&[usize]; 3] = [&[0], &[1, 2], &[3]];

/// How an envelope display places its stages, from the theme (the cards and the source tiles share it).
pub fn stages(t: &Theme) -> EnvelopeStages {
    EnvelopeStages {
        min: t.size.cards.envelope_min_stage,
        range: t.size.cards.envelope_stage_range,
    }
}

/// Draws an `adsr` card's display with its handles into `rect` and records their edits. Falls back to a plain
/// display if the card lacks one of the four parameters.
pub(super) fn show(
    ui: &mut Ui,
    t: &Theme,
    card: &CardView,
    id: &ModuleId,
    rect: Rect,
    out: &mut Intents,
) {
    let params: Option<[&ParamView; 4]> = (|| {
        Some([
            card.param("attack")?,
            card.param("decay")?,
            card.param("sustain")?,
            card.param("release")?,
        ])
    })();
    let Some(params) = params else {
        return;
    };
    let geo = stages(t);
    let before: [f32; 4] = params.map(ParamView::norm);
    let mut after = before;
    let [peak, corner, release] = geo.corners(before[0], before[1], before[2], before[3]);
    let per_span = 1.0 / geo.range;
    let formats: [Box<dyn Fn(f32) -> String>; 4] = params.map(|p| {
        let spec = p.spec;
        Box::new(move |n: f32| value_text(spec, spec.from_norm(n))) as Box<dyn Fn(f32) -> String>
    });
    let defaults: [f32; 4] = params.map(|p| p.spec.to_norm(p.spec.default));
    let names = params.map(|p| p.spec.name);
    let edit = {
        let [a, d, s, r] = &mut after;
        let drag = |i: usize, value, span| drag(names[i], value, span, defaults[i], &*formats[i]);
        let title = &card.title;
        let mut handles = [
            Handle::new(format!("{title} attack"), peak).x(drag(0, a, per_span)),
            Handle::new(format!("{title} decay and sustain"), corner)
                .x(drag(1, d, per_span))
                .y(drag(2, s, 1.0)),
            Handle::new(format!("{title} release"), release).x(drag(3, r, -per_span)),
        ];
        CurveHandles::new(t, "envelope")
            .id(ids::handles(id))
            .interact(ui, rect, &mut handles)
    };
    let [a, d, s, r] = after;
    Display::new(t, Trace::Curve(&|x| geo.curve(x, a, d, s, r)), rect.size())
        .ink(Ink::Modulation)
        .handles(&edit)
        .paint(ui, rect);
    for (index, response) in edit.responses.iter().enumerate() {
        let grip = Grip::Handle(id.clone(), index);
        if response.drag_started() {
            out.push(Intent::Begin(grip.clone()));
        }
        if response.changed() {
            for &k in HANDLE_PARAMS[index] {
                let p = params[k];
                let value = p.spec.clamp(p.spec.from_norm(after[k]));
                if value.to_bits() != p.value.to_bits() {
                    out.push(Intent::Set {
                        addr: p.addr.clone(),
                        value,
                    });
                }
            }
        }
        if response.drag_stopped() {
            out.push(Intent::End(grip));
        }
    }
    if let Some(active) = edit.active() {
        let moved = HANDLE_PARAMS[active];
        let names: Vec<&str> = moved.iter().map(|&k| params[k].spec.name).collect();
        let values: Vec<String> = moved
            .iter()
            .map(|&k| {
                let p = params[k];
                value_text(p.spec, p.spec.clamp(p.spec.from_norm(after[k])))
            })
            .collect();
        out.hovered = Some(Hovered {
            path: format!("{} {} {}", card.title, glyph::PATH, names.join(", ")),
            value: values.join(", "),
            modulation: String::new(),
        });
    }
}

/// A handle drag on one parameter's normalised value, with its default and its value in real units.
fn drag<'a>(
    name: &'a str,
    value: &'a mut f32,
    per_span: f32,
    default: f32,
    format: &'a dyn Fn(f32) -> String,
) -> Drag<'a> {
    Drag::new(name, value, per_span)
        .default_value(default)
        .format(format)
}
