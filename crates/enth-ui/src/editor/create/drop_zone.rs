//! **Owner: lane PUI** (the Create view, feature `create`). Pre-declared by lane S0s (R3).
//!
//! The drop zone and what the intake found (docs/r3.md, "The Create view and the session"). The zone
//! (`size.create.drop_zone` high, `surface.recessed`) takes egui's `dropped_files` wherever they land on the window
//! (the frame reads them, whatever the view; fork patch 11, lane FK3) and lights in the value accent while files are
//! dragged over the window. It shows what to drop, or the chosen file's name and, once the shell's intake post
//! arrived, the clip's length and its note (a file chosen while another file's job runs says it waits for Start).
//!
//! Under it, the intake's findings, in `text.label`: how the file decoded; the note (its name and f0, or "no stable
//! pitch"), how long it is held and the clip's length; the trim (the lead-in cut before the onset, a short sound
//! padded); what the upload lost (a low-pass, clipping, a confirmed noise floor); then the intake's warnings in the
//! near-score ink with a bullet (a chord, a phrase, several notes, a sound longer than the cap). **A short sound is
//! shown as it is, never refused**: its length reads in milliseconds below 0.1 s, and "short: kept whole" says it was
//! padded rather than dropped. Sizes come from `theme/sizes/create.rs`.

use egui::{Label, RichText, Sense, Ui, vec2};
use enth_session::create::{CreateIntake, CreateView};

use super::panel::elided;
use super::state::Create;
use crate::Theme;
use crate::theme::glyph;

/// A length in seconds, short: "20 ms" below 0.1 s, else "1.25 s".
pub(crate) fn seconds_text(s: f64) -> String {
    let ms = s * 1e3;
    if ms < 100.0 {
        format!("{ms:.0} ms")
    } else {
        format!("{s:.2} s")
    }
}

/// A MIDI key's name ("A3", "C#4"), middle C = C4 = 60.
pub(crate) fn key_name(key: u8) -> String {
    const NAMES: [&str; 12] = [
        "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
    ];
    format!(
        "{}{}",
        NAMES[usize::from(key % 12)],
        i32::from(key / 12) - 1
    )
}

/// The drop zone's second line: the clip's length and note, once the intake ran.
fn summary(intake: &CreateIntake) -> String {
    let perf = &intake.report.perf;
    let note = perf
        .f0_hz
        .map_or_else(|| "unpitched".to_owned(), |_| key_name(perf.key));
    format!(
        "{} {} {note}",
        seconds_text(intake.report.source_s),
        glyph::BULLET
    )
}

/// Draws the drop zone into `ui`: what to drop, or the chosen file and its summary; lit while files hover.
pub(crate) fn zone(create: &Create, ui: &mut Ui, t: &Theme, hovering: bool) {
    let width = ui.available_width();
    let (rect, response) =
        ui.allocate_exact_size(vec2(width, t.size.create.drop_zone), Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(
        rect,
        t.radius.medium,
        if hovering {
            t.color.accent.value_wash
        } else {
            t.color.surface.recessed
        },
    );
    if hovering {
        painter.rect_stroke(
            rect,
            t.radius.medium,
            t.stroke.thin(t.color.accent.value),
            egui::StrokeKind::Inside,
        );
    }
    let view = create.view();
    let chosen = create.file().map(|f| {
        f.file_name().map_or_else(
            || f.to_string_lossy().into_owned(),
            |n| n.to_string_lossy().into_owned(),
        )
    });
    let (first, second) = if hovering {
        ("Drop to make patches from this sound".to_owned(), None)
    } else if let Some(name) = chosen {
        // The job's findings belong to the job's file: a file chosen while another runs waits for Start.
        let second = match view {
            Some(v) if v.file == name => v.intake.as_ref().map(summary),
            Some(v) if v.phase.running() => Some(format!("Start it once {} is done", v.file)),
            _ => None,
        };
        (name, second)
    } else {
        (
            "Drop a sound file here".to_owned(),
            Some("WAV, FLAC, Ogg Vorbis or MP3, any length".to_owned()),
        )
    };
    let inner = rect.shrink(t.space.gap);
    let first = elided(ui, &first, &t.text.strong, inner.width());
    let line = t.text.strong.size + t.space.tight;
    let top = match &second {
        Some(_) => rect.center().y - line,
        None => rect.center().y - line * 0.5,
    };
    let painter = ui.painter();
    painter.text(
        egui::pos2(rect.center().x, top),
        egui::Align2::CENTER_TOP,
        &first,
        t.text.strong.clone(),
        t.color.text.primary,
    );
    if let Some(second) = &second {
        painter.text(
            egui::pos2(rect.center().x, top + line),
            egui::Align2::CENTER_TOP,
            second,
            t.text.label.clone(),
            t.color.text.label,
        );
    }
    let label = match &second {
        Some(s) => format!("Drop zone: {first} {s}"),
        None => format!("Drop zone: {first}"),
    };
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, &label));
}

/// The intake's findings under the drop zone (see the [module docs](self)).
pub(crate) fn findings(view: &CreateView, ui: &mut Ui, t: &Theme) {
    let Some(intake) = &view.intake else {
        return;
    };
    let report = &intake.report;
    let perf = &report.perf;
    let line = |ui: &mut Ui, text: String, color| {
        let shown =
            ui.add(Label::new(RichText::new(&text).font(t.text.label.clone()).color(color)).wrap());
        shown.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, &text));
    };
    line(ui, intake.decoded.clone(), t.color.text.label);
    let note = perf.f0_hz.map_or_else(
        || "No stable pitch".to_owned(),
        |f| format!("Note {} ({f:.1} Hz)", key_name(perf.key)),
    );
    line(
        ui,
        format!(
            "{note} {} held {} of {}",
            glyph::BULLET,
            seconds_text(perf.hold_s),
            seconds_text(perf.duration_s)
        ),
        t.color.text.label,
    );
    let mut trim = Vec::new();
    if report.span.start_s > 0.0 {
        trim.push(format!(
            "trimmed {} before the onset",
            seconds_text(report.span.start_s)
        ));
    }
    if report.span.tail_pad_s > 0.0 {
        trim.push("short: kept whole".to_owned());
    }
    let d = &report.degradation;
    if let Some(hz) = d.bandwidth_hz {
        trim.push(format!("low-passed at {:.1} kHz", hz / 1e3));
    }
    if d.clipping.is_some() {
        trim.push("clipped".to_owned());
    }
    if d.floor_confirmed
        && let Some(db) = d.noise_floor_dbfs
    {
        trim.push(format!("noise at {db:.0} dBFS"));
    }
    if !trim.is_empty() {
        let mut text = trim.join(&format!(" {} ", glyph::BULLET));
        if let Some(first) = text.get_mut(..1) {
            first.make_ascii_uppercase();
        }
        line(ui, text, t.color.text.label);
    }
    for w in intake.warnings() {
        line(ui, format!("{} {w}", glyph::BULLET), t.color.score.near);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lengths_and_keys_read_short() {
        assert_eq!(seconds_text(20e-3), "20 ms");
        assert_eq!(seconds_text(1254e-3), "1.25 s");
        assert_eq!(key_name(57), "A3");
        assert_eq!(key_name(61), "C#4");
        assert_eq!(key_name(0), "C-1");
    }
}
