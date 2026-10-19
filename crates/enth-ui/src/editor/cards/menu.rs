//! A control's right-click menu and its "Type value…" field (lane ED1).
//!
//! The menu is the same on every parameter control (knob, segmented control, switch, the header's inline knobs):
//!
//! - **Type value…** opens a field over the control, pre-filled with the value as the editor shows it and selected.
//!   Enter parses it with units in either form ("1.2kHz", "1.20 kHz", "A4", "−12 st" with the U+2212 minus the
//!   design system displays) and pushes one [`Intent::Set`] (one undo step, no gesture). A value that does not parse
//!   keeps the field open with the parser's hint under it; Escape or a click elsewhere cancels.
//! - **Reset to default** pushes one `Set` of the spec's default.
//! - **Automate** exposes the parameter to the host through an automation slot ([`Intent::Automate`]); once a slot
//!   drives it, the menu names the slot and offers **Stop automating** ([`Intent::StopAutomating`]). Structural
//!   parameters (they recompile the plan) are not automatable, and the item says so.
//!
//! The open field lives in egui's temporary memory under [`ids::entry`], one at a time, keyed by the parameter's
//! address, so it follows its control through a re-layout.

use egui::text::CCursorRange;
use egui::{Area, Frame, Key, Order, Rect, Response, RichText, TextEdit, Ui};
use enth_patch::ParamAddr;

use super::format::{parse_value, value_text};
use super::ids;
use super::intent::{Intent, Intents};
use super::present;
use super::view::{CardKind, ParamView};
use crate::Theme;
use crate::theme::glyph;

/// The open "Type value…" field.
#[derive(Clone, Debug)]
struct Entry {
    addr: ParamAddr,
    text: String,
    /// The parser's hint after a value that did not parse.
    error: Option<String>,
    /// Focus was requested (and the text selected) on the first frame.
    focused: bool,
    /// The last frame the field was drawn; a field whose control stopped drawing is dropped.
    frame: u64,
}

/// Adds the right-click menu to a parameter control's response.
pub(super) fn attach(
    t: &Theme,
    kind: CardKind,
    p: &ParamView,
    response: &Response,
    out: &mut Intents,
) {
    response.context_menu(|ui| {
        ui.set_min_width(t.size.editor.entry_width);
        if ui.button("Type value…").clicked() {
            let entry = Entry {
                addr: p.addr.clone(),
                text: value_text(p.spec, p.value),
                error: None,
                focused: false,
                frame: ui.ctx().cumulative_frame_nr(),
            };
            ui.ctx().data_mut(|d| d.insert_temp(ids::entry(), entry));
            ui.close();
        }
        let default = value_text(p.spec, p.spec.default);
        if ui.button(format!("Reset to default ({default})")).clicked() {
            if p.value.to_bits() != p.spec.default.to_bits() {
                out.push(Intent::Set {
                    addr: p.addr.clone(),
                    value: p.spec.default,
                });
            }
            ui.close();
        }
        ui.separator();
        automate(ui, t, kind, p, out);
    });
}

/// The automation items.
fn automate(ui: &mut Ui, t: &Theme, kind: CardKind, p: &ParamView, out: &mut Intents) {
    let name = present::label(kind, p.spec);
    if !p.spec.flags.automatable {
        ui.add_enabled(false, egui::Button::new("Automate"))
            .on_disabled_hover_text(format!(
                "{name} changes the patch's structure, so the host cannot automate it"
            ));
        return;
    }
    match p.slots.as_slice() {
        [] => {
            if ui
                .button("Automate")
                .on_hover_text("Expose it to the host as the next free automation slot")
                .clicked()
            {
                out.push(Intent::Automate(p.addr.clone()));
                ui.close();
            }
        }
        slots => {
            let names: Vec<String> = slots
                .iter()
                .map(|s| format!("slot {}", s.one_based()))
                .collect();
            ui.label(
                RichText::new(format!("Automated by {}", names.join(", ")))
                    .font(t.text.label.clone())
                    .color(t.color.text.label),
            );
            if ui.button("Stop automating").clicked() {
                out.push(Intent::StopAutomating(p.addr.clone()));
                ui.close();
            }
        }
    }
}

/// Draws the "Type value…" field over `anchor` if it is open for `p`, and pushes the typed value on Enter.
pub(super) fn entry(ui: &Ui, t: &Theme, p: &ParamView, anchor: Rect, out: &mut Intents) {
    let ctx = ui.ctx().clone();
    let Some(mut entry) = ctx.data(|d| d.get_temp::<Entry>(ids::entry())) else {
        return;
    };
    if entry.addr != p.addr {
        return;
    }
    let frame = ctx.cumulative_frame_nr();
    if frame > entry.frame + 1 {
        // Its control was not drawn last frame (scrolled away, removed): the field is stale.
        ctx.data_mut(|d| d.remove::<Entry>(ids::entry()));
        return;
    }
    entry.frame = frame;
    let edit_id = ids::param(&p.addr).with("entry");
    let mut open = true;
    Area::new(ids::param(&p.addr).with("entry_area"))
        .order(Order::Foreground)
        .fixed_pos(anchor.left_top())
        .show(&ctx, |ui| {
            Frame::popup(ui.style())
                .inner_margin(t.space.gap)
                .show(ui, |ui| {
                    ui.set_max_width(t.size.editor.entry_hint_width);
                    let mut output = TextEdit::singleline(&mut entry.text)
                        .id(edit_id)
                        .font(t.text.value.clone())
                        .desired_width(t.size.editor.entry_width)
                        .show(ui);
                    let response = output.response.response.clone();
                    let response = response.accessible_name(format!("{} value", p.spec.name));
                    if !entry.focused {
                        response.request_focus();
                        output
                            .state
                            .cursor
                            .set_char_range(Some(CCursorRange::select_all_str(&entry.text)));
                        output.state.store(ui.ctx(), edit_id);
                        entry.focused = true;
                    } else if response.lost_focus() {
                        if ui.input(|i| i.key_pressed(Key::Enter)) {
                            match parse_value(p.spec, &entry.text) {
                                Ok(value) => {
                                    let value = p.spec.clamp(value);
                                    if value.to_bits() != p.value.to_bits() {
                                        out.push(Intent::Set {
                                            addr: p.addr.clone(),
                                            value,
                                        });
                                    }
                                    open = false;
                                }
                                Err(e) => {
                                    entry.error = Some(e.hint());
                                    response.request_focus();
                                }
                            }
                        } else {
                            // Escape, or a click elsewhere: cancel, so a stray click never commits a half-typed value.
                            open = false;
                        }
                    }
                    if let Some(error) = &entry.error {
                        ui.label(
                            RichText::new(format!("{} {error}", glyph::BAD))
                                .font(t.text.label.clone())
                                .color(t.color.score.bad),
                        );
                    }
                });
        });
    ctx.data_mut(|d| {
        if open {
            d.insert_temp(ids::entry(), entry);
        } else {
            d.remove::<Entry>(ids::entry());
        }
    });
}
