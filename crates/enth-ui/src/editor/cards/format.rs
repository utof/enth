//! Parameter values as the editor shows them (docs/design-system.md): three significant digits, a space before the
//! unit, the typographic minus "−", kHz and seconds with their trailing zeros so the width holds still while a knob
//! moves ("1.20 kHz", "−12 st", "+7 ct", "50 %", "4:1", "90°"); enum items by name, toggles "On" and "Off".
//!
//! [`parse_value`] reads that form back (and everything `ParamSpec::parse` reads): the editor's text entry
//! pre-fills a value and accepts what it showed.

use enth_params::{Kind, ParamSpec, ValueError};

/// Significant digits of a displayed value.
const DIGITS: u8 = 3;

/// The typographic minus the design system displays.
pub const MINUS: char = '\u{2212}';

/// `value` (plain, in the spec's unit) as the editor shows it.
pub fn value_text(spec: &ParamSpec, value: f32) -> String {
    match spec.kind {
        Kind::Enum(_) => spec
            .enum_item(value)
            .map_or_else(|| spec.display(value).to_string(), |i| i.name.to_owned()),
        Kind::Toggle => if value >= 0.5 { "On" } else { "Off" }.to_owned(),
        // Integers in full: a seed of 65,535 is not "65500".
        Kind::Int { .. } => typeset(&spec.display(value).to_string()),
        Kind::Float { .. } => typeset(&spec.display_rounded(value, DIGITS).to_string()),
    }
}

/// A signed percentage, for route depths in the status line ("+35 %", "−4 %").
pub fn signed_percent(depth: f32) -> String {
    let text = typeset(
        &enth_params::DEPTH
            .display_rounded(depth, DIGITS)
            .to_string(),
    );
    if depth > 0.0 {
        format!("+{text}")
    } else {
        text
    }
}

/// Reads a value typed or copied from the editor: an enum item's name or key, "On"/"Off", or a number with an
/// optional unit in either form ("1.20 kHz", "−12 st", "1.2kHz", "A4").
pub fn parse_value(spec: &ParamSpec, text: &str) -> Result<f32, ValueError> {
    if let Some(items) = spec.enum_items() {
        let t = text.trim();
        if let Some(i) = items.iter().position(|i| i.name.eq_ignore_ascii_case(t)) {
            return Ok(i as f32);
        }
    }
    spec.parse(text)
}

/// The CLI's compact form ("-1.2kHz", "+7st", "90deg") in the design system's form ("−1.20 kHz", "+7 st", "90°").
fn typeset(compact: &str) -> String {
    let (sign, rest) = match compact.as_bytes().first() {
        Some(b'-') => (MINUS.to_string(), &compact[1..]),
        Some(b'+') => ("+".to_owned(), &compact[1..]),
        _ => (String::new(), compact),
    };
    let split = rest
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(rest.len());
    let (number, unit) = rest.split_at(split);
    let number = match unit {
        // Scaled units keep three digits, so "1.20 kHz" doesn't shrink to "1.2 kHz" mid-drag.
        "kHz" | "s" => pad_digits(number, usize::from(DIGITS)),
        _ => number.to_owned(),
    };
    match unit {
        "" => format!("{sign}{number}"),
        ":1" => format!("{sign}{number}:1"),
        "deg" => format!("{sign}{number}°"),
        unit => format!("{sign}{number} {unit}"),
    }
}

/// Pads a plain decimal ("1.2", "20") with trailing zeros to `digits` significant digits ("1.20", "20.0").
fn pad_digits(number: &str, digits: usize) -> String {
    let significant = number
        .chars()
        .filter(char::is_ascii_digit)
        .skip_while(|&c| c == '0')
        .count();
    if significant == 0 || significant >= digits {
        return number.to_owned();
    }
    let mut out = number.to_owned();
    if !out.contains('.') {
        out.push('.');
    }
    out.extend(std::iter::repeat_n('0', digits - significant));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typesets_the_compact_form() {
        assert_eq!(typeset("1.2kHz"), "1.20 kHz");
        assert_eq!(typeset("20kHz"), "20.0 kHz");
        assert_eq!(typeset("1.23kHz"), "1.23 kHz");
        assert_eq!(typeset("440Hz"), "440 Hz");
        assert_eq!(typeset("-12st"), "\u{2212}12 st");
        assert_eq!(typeset("+7ct"), "+7 ct");
        assert_eq!(typeset("50%"), "50 %");
        assert_eq!(typeset("4:1"), "4:1");
        assert_eq!(typeset("90deg"), "90°");
        assert_eq!(typeset("1.5s"), "1.50 s");
        assert_eq!(typeset("300ms"), "300 ms");
        assert_eq!(typeset("2.5"), "2.5");
        assert_eq!(typeset("-6dB"), "\u{2212}6 dB");
        assert_eq!(pad_digits("0", 3), "0");
        assert_eq!(pad_digits("0.5", 3), "0.500");
    }
}

#[cfg(test)]
mod round_trip {
    use enth_params::ParamSpec;
    use enth_patch::ModuleKind;
    use enth_patch::catalogue::Builtin;

    use super::*;

    /// Every catalogue parameter at its minimum, default, middle and maximum: what the editor shows parses back
    /// to the value, within the three digits it shows (exactly for integers, enums and toggles). Negative values
    /// show U+2212, so this also holds enth-params' parser to reading it.
    #[test]
    fn what_the_editor_shows_parses_back() {
        let specs: Vec<&ParamSpec> = ModuleKind::ALL
            .iter()
            .flat_map(|k| k.params())
            .chain(Builtin::Voice.params())
            .chain(Builtin::Master.params())
            .collect();
        for spec in specs {
            for value in [spec.min(), spec.default, spec.from_norm(0.5), spec.max()] {
                let text = value_text(spec, value);
                let back = parse_value(spec, &text)
                    .unwrap_or_else(|e| panic!("{}: {text:?} does not parse: {e}", spec.key));
                let tolerance = if spec.is_discrete() {
                    0.0
                } else {
                    value.abs() * 5e-3 + 1e-6
                };
                assert!(
                    (back - value).abs() <= tolerance,
                    "{}: {value} shows as {text:?} and reads back as {back}",
                    spec.key
                );
            }
        }
        // The compressor's threshold defaults to -18 dB.
        let threshold = &enth_patch::catalogue::COMP_PARAMS[0];
        assert_eq!(value_text(threshold, threshold.default), "\u{2212}18 dB");
    }
}
