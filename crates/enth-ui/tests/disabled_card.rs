//! A disabled module card dims as a whole, checked on rendered pixels (CPU renderer only, like
//! the snapshots): switching it off fades its title and every knob, arcs and value text alike,
//! towards the card colour, while the card's fill stays as it was.

mod common;

use egui::{Rect, Role, pos2, vec2};
use egui_kittest::kittest::Queryable;
use enth_ui::Theme;
use enth_ui::theme::Category;
use enth_ui::widgets::{Display, Knob, ModuleCard, Trace};

struct State {
    enabled: bool,
    knobs: [f32; 2],
    /// Where the card put its display.
    display: Rect,
}

/// A rendered frame: RGBA bytes, row by row, at scale 1.0.
struct Frame {
    pixels: Vec<u8>,
    width: usize,
}

impl Frame {
    fn of<S>(h: &mut egui_kittest::Harness<'_, S>) -> Self {
        let image = h.render().unwrap();
        Self {
            width: image.width() as usize,
            pixels: image.into_raw(),
        }
    }

    fn pixel(&self, x: usize, y: usize) -> [f32; 3] {
        let at = 4 * (y * self.width + x);
        std::array::from_fn(|c| f32::from(self.pixels[at + c]))
    }

    /// How much `rect` stands out from a `background` colour: the summed difference of every
    /// channel of every pixel.
    fn ink(&self, rect: Rect, background: [f32; 3]) -> f32 {
        let mut sum = 0.0;
        for y in rect.top() as usize..rect.bottom() as usize {
            for x in rect.left() as usize..rect.right() as usize {
                let p = self.pixel(x, y);
                sum += (0..3).map(|c| (p[c] - background[c]).abs()).sum::<f32>();
            }
        }
        sum
    }
}

#[test]
fn switching_a_card_off_dims_everything_in_it() {
    let theme = Theme::dark();
    let ui_theme = theme.clone();
    let mut h = common::harness(
        &theme,
        vec2(600.0, 120.0),
        move |ui, s: &mut State| {
            let t = &ui_theme;
            let [level, width] = &mut s.knobs;
            ModuleCard::new(t, "osc-2", "Osc 2", Category::Generator, &mut s.enabled)
                .kind("Pulse")
                .show(
                    ui,
                    |ui, rect| {
                        Display::new(t, Trace::Curve(&|x| x), rect.size()).paint(ui, rect);
                        s.display = rect;
                    },
                    |ui| {
                        ui.add(Knob::new(t, level, "Level").value_text("-14.7 dB"));
                        ui.add(Knob::new(t, width, "Width").modulated(0.8));
                    },
                );
        },
        State {
            enabled: true,
            knobs: [0.7, 0.5],
            display: Rect::NOTHING,
        },
    );
    h.run();
    let header = h.get_by_label("Move Osc 2").rect();
    let toggle = h.get_by_role_and_label(Role::CheckBox, "Osc 2").rect();
    let title = Rect::from_min_max(pos2(toggle.right() + 1.0, header.top()), header.max);
    let level = h.get_by_label("Level").rect();
    let width = h.get_by_label("Width").rect();
    // Past the last knob: the card's own fill.
    let fill = pos2(header.right() - theme.space.pad, width.center().y);
    let on = Frame::of(&mut h);

    h.get_by_role_and_label(Role::CheckBox, "Osc 2").click();
    h.run();
    assert!(!h.state().enabled);
    // Keep the synthetic cursor out of the measured pixels.
    h.remove_cursor();
    h.run();
    let off = Frame::of(&mut h);

    let card = on.pixel(fill.x as usize, fill.y as usize);
    assert_eq!(
        card,
        off.pixel(fill.x as usize, fill.y as usize),
        "the card's fill stays as it was"
    );
    // Everything fades towards the card colour by the token's opacity. Shapes fade one by one, so
    // where two overlap (a pointer over an arc) the result is a little brighter; summed over a
    // whole region that stays within the tolerance.
    let expected = theme.state.disabled_opacity;
    let display = h.state().display;
    for (name, rect) in [
        ("title", title),
        ("display", display),
        ("Level knob", level),
        ("Width knob", width),
    ] {
        let ratio = off.ink(rect, card) / on.ink(rect, card);
        assert!(
            (ratio - expected).abs() < 0.05,
            "{name} faded to {ratio:.3} of its contrast (expected {expected})"
        );
    }
}
