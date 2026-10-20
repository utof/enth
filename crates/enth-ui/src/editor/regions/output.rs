//! The output region (lane ED2): a scope of the engine's newest output, or its spectrum (click to switch), and a
//! stereo meter, all from the telemetry tap. Layout 1 places it right of the header. With no telemetry (a detached
//! session, nothing published yet) it draws an empty display and empty meters.
//!
//! - **Scope**: half of the newest `size.output.scope_samples` samples of the mono mix `(L + R) / 2` the tap keeps,
//!   starting at the first rising zero crossing of the older half (a trigger, so a steady tone stands still).
//! - **Spectrum**: the same window through a Hann window and a radix-2 FFT, in dB from `size.output.meter_floor_db`
//!   to 0 dBFS, on the display's log-frequency axis. Computed only when the tap brought new samples.
//! - **Meters**: each channel's peak of the last published block in dB, falling back at `size.output.meter_fall_db_per_s`
//!   between peaks, so a meter reads like one at 30 frames a second.

use egui::{Rect, Response, Role, Sense, Ui, WidgetInfo, pos2, vec2};

use super::region_ui;
use crate::Theme;
use crate::editor::cards::{Hovered, Intents, ids};
use crate::editor::live::Fingerprint;
use crate::meter::{db, spectrum};
use crate::theme::glyph;
use crate::widgets::{Display, Trace};

/// What the output shows this frame.
#[derive(Clone, Copy, Debug, Default)]
pub struct OutputView<'a> {
    /// The newest scope samples, oldest first, and how many the tap has written in all.
    pub scope: Option<(&'a [f32], u64)>,
    /// The last published block's peak per channel, linear.
    pub peaks: Option<[f32; 2]>,
    /// Seconds since the last frame (the meters' fall), from egui's input.
    pub dt: f32,
}

/// The output's state across frames: the display mode, the meters' falling levels and the last spectrum.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OutputState {
    /// The display shows the spectrum instead of the scope.
    pub spectrum: bool,
    /// Each meter's level now, in dB (the floor when silent).
    meters: Option<[f32; 2]>,
    /// The spectrum's bins (0..1, linear in frequency) and the scope count they were computed from.
    bins: Vec<f32>,
    bins_for: u64,
}

/// The output region.
pub struct Output<'a> {
    theme: &'a Theme,
    view: OutputView<'a>,
}

impl<'a> Output<'a> {
    pub fn new(theme: &'a Theme, view: OutputView<'a>) -> Self {
        Self { theme, view }
    }

    /// Draws into `rect`; a click on the display switches scope and spectrum. Adds what it shows to `fingerprint`
    /// (the region is visible whenever it is drawn).
    pub fn show(
        self,
        ui: &mut Ui,
        rect: Rect,
        state: &mut OutputState,
        fingerprint: &mut Fingerprint,
        out: &mut Intents,
    ) {
        let t = self.theme;
        let c = &t.color;
        let o = &t.size.output;
        let ui = region_ui(ui, rect, "output");
        ui.painter()
            .rect_filled(rect, t.radius.medium, c.surface.card);
        let inner = rect.shrink(t.space.tight);
        let meters_w = 2.0 * o.meter_width + t.space.tight;
        let display = Rect::from_min_max(
            inner.min,
            pos2(inner.right() - meters_w - t.space.tight, inner.bottom()),
        );
        let meters = Rect::from_min_max(
            pos2(display.right() + t.space.tight, inner.top()),
            inner.max,
        );

        // The display: scope or spectrum.
        let samples = self.view.scope.map_or(&[][..], |(s, _)| s);
        let written = self.view.scope.map_or(0, |(_, n)| n);
        let (caption, trace) = if state.spectrum {
            if state.bins_for != written || state.bins.is_empty() {
                spectrum(samples, o.meter_floor_db, &mut state.bins);
                state.bins_for = written;
            }
            ("Spectrum", Trace::Spectrum(&state.bins))
        } else {
            ("Scope", Trace::Scope(triggered(samples)))
        };
        Display::new(t, trace, display.size())
            .caption(caption)
            .paint(&ui, display);
        let click = ui.interact(
            display,
            ids::region("output").with("display"),
            Sense::click(),
        );
        let switch = if state.spectrum { "scope" } else { "spectrum" };
        click.widget_info(|| WidgetInfo::labeled(Role::Button, true, caption));
        if click.clicked() {
            state.spectrum = !state.spectrum;
        }
        if click.hovered() {
            out.hovered = Some(Hovered {
                path: format!("Output {} {caption}", glyph::PATH),
                value: String::new(),
                modulation: format!("click for the {switch}"),
            });
        }
        fingerprint.count(("output.scope", state.spectrum), written);

        // The meters: the newest peaks, falling back between them.
        let floor = o.meter_floor_db;
        let fall = o.meter_fall_db_per_s * self.view.dt.max(0.0);
        let levels = match (self.view.peaks, state.meters) {
            (Some(peaks), before) => {
                let before = before.unwrap_or([floor; 2]);
                core::array::from_fn(|i| db(peaks[i], floor).max(before[i] - fall).max(floor))
            }
            (None, _) => [floor; 2],
        };
        state.meters = Some(levels);
        for (i, level) in levels.into_iter().enumerate() {
            let left = meters.left() + i as f32 * (o.meter_width + t.space.tight);
            let bar = Rect::from_min_size(
                pos2(left, meters.top()),
                vec2(o.meter_width, meters.height()),
            );
            let painter = ui.painter();
            painter.rect_filled(bar, t.radius.small, c.surface.recessed);
            let frac = ((level - floor) / -floor).clamp(0.0, 1.0);
            if frac > 0.0 {
                let fill = Rect::from_min_max(
                    pos2(bar.left(), bar.bottom() - frac * bar.height()),
                    bar.max,
                );
                painter.rect_filled(fill, t.radius.small, c.accent.value);
            }
            fingerprint.add(("output.meter", i), frac);
        }
        let meter: Response =
            ui.interact(meters, ids::region("output").with("meters"), Sense::hover());
        let text = format!("{:.1} dB, {:.1} dB", levels[0], levels[1]);
        meter
            .widget_info(|| WidgetInfo::labeled(Role::Meter, true, format!("Output level {text}")));
        if meter.hovered() {
            out.hovered = Some(Hovered {
                path: format!("Output {} Level", glyph::PATH),
                value: text,
                modulation: String::new(),
            });
        }
    }
}

/// The scope's window: the second half's length of `samples`, starting at the first rising zero crossing in the first
/// half (else the newest half), so a periodic signal draws at the same phase frame after frame.
fn triggered(samples: &[f32]) -> &[f32] {
    let half = samples.len() / 2;
    let start = (1..half)
        .find(|&i| samples[i - 1] < 0.0 && samples[i] >= 0.0)
        .unwrap_or(half);
    &samples[start..start + half]
}
