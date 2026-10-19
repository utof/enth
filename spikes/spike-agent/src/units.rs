//! Unit strings in and out: `800Hz`, `1.2kHz`, `40ms`, `0.5s`, `-6dB`, `12ct`, `3oct`, `30%`, note names (`C3`).
//!
//! Every parameter and every descriptor has one canonical [`Unit`]; a value with no suffix is read in that unit, a
//! value with a suffix is converted, and a suffix that makes no sense for the quantity is an error that lists what
//! is accepted.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unit {
    Hz,
    Ms,
    Seconds,
    Db,
    Lufs,
    Cents,
    Octaves,
    /// A 0..1 ratio that may also be written as a percentage (`30%` = 0.3).
    Ratio,
    /// A dimensionless number (resonance Q).
    Plain,
}

impl Unit {
    /// The suffix used when printing a value.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Unit::Hz => "Hz",
            Unit::Ms => "ms",
            Unit::Seconds => "s",
            Unit::Db => "dB",
            Unit::Lufs => "LUFS",
            Unit::Cents => "ct",
            Unit::Octaves => "oct",
            Unit::Ratio | Unit::Plain => "",
        }
    }

    fn accepted(self) -> &'static str {
        match self {
            Unit::Hz => "Hz, kHz",
            Unit::Ms => "ms, s",
            Unit::Seconds => "s, ms",
            Unit::Db => "dB",
            Unit::Lufs => "LUFS, dB",
            Unit::Cents => "ct",
            Unit::Octaves => "oct",
            Unit::Ratio => "% or a plain 0..1 number",
            Unit::Plain => "a plain number",
        }
    }

    /// Factor from `suffix` (case-insensitive) to this unit, or `None` if the suffix does not belong here.
    fn factor(self, suffix: &str) -> Option<f64> {
        let s = suffix.to_ascii_lowercase();
        match (self, s.as_str()) {
            (Unit::Hz, "hz")
            | (Unit::Ms, "ms")
            | (Unit::Seconds, "s")
            | (Unit::Db, "db")
            | (Unit::Lufs, "lufs" | "db" | "lu")
            | (Unit::Cents, "ct" | "cents" | "cent")
            | (Unit::Octaves, "oct" | "octaves") => Some(1.0),
            (Unit::Hz, "khz") | (Unit::Ms, "s") => Some(1000.0),
            (Unit::Seconds, "ms") => Some(0.001),
            (Unit::Ratio, "%") => Some(0.01),
            _ => None,
        }
    }
}

/// Split `"-12.5kHz"` into `(-12.5, "kHz")`.
fn split_number(s: &str) -> Option<(f64, &str)> {
    let s = s.trim();
    let bytes = s.as_bytes();
    let mut end = 0;
    if end < bytes.len() && (bytes[end] == b'-' || bytes[end] == b'+') {
        end += 1;
    }
    let digits_start = end;
    while end < bytes.len() && (bytes[end].is_ascii_digit() || bytes[end] == b'.') {
        end += 1;
    }
    if end == digits_start {
        return None;
    }
    let n: f64 = s[..end].parse().ok()?;
    Some((n, s[end..].trim()))
}

/// Parse `text` as a value of `unit`. The error string is ready to show to the agent.
pub fn parse_value(text: &str, unit: Unit) -> Result<f64, String> {
    let (n, suffix) = split_number(text)
        .ok_or_else(|| format!("'{text}' is not a number with an optional unit"))?;
    if suffix.is_empty() {
        return Ok(clean(n));
    }
    unit.factor(suffix).map(|f| clean(n * f)).ok_or_else(|| {
        format!(
            "unit '{suffix}' does not fit here; this quantity is in {}",
            unit.accepted()
        )
    })
}

/// Round to 9 significant digits, which removes floating-point noise (`1.2 * 1000`) and nothing a human typed.
#[must_use]
pub fn clean(x: f64) -> f64 {
    sig(x, 9)
}

/// Round to 4 significant digits (all numbers the CLI prints).
#[must_use]
pub fn r4(x: f64) -> f64 {
    sig(x, 4)
}

/// Round to `digits` significant digits. Zero and non-finite values come back as they are (`-0` becomes `0`).
#[must_use]
pub fn sig(x: f64, digits: i32) -> f64 {
    if x == 0.0 || !x.is_finite() {
        return if x == 0.0 { 0.0 } else { x };
    }
    let e = x.abs().log10().floor() as i32;
    let shift = digits - 1 - e;
    if shift.abs() > 300 {
        return x;
    }
    if shift >= 0 {
        let scale = 10f64.powi(shift);
        (x * scale).round() / scale
    } else {
        let scale = 10f64.powi(-shift);
        (x / scale).round() * scale
    }
}

/// `1234.5` as `"1.235kHz"`, `40.0` as `"40ms"`.
#[must_use]
pub fn fmt_value(v: f64, unit: Unit) -> String {
    if unit == Unit::Hz && v.abs() >= 1000.0 {
        return format!("{}kHz", r4(v / 1000.0));
    }
    format!("{}{}", r4(v), unit.label())
}

const NOTE_NAMES: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];

/// Frequency of MIDI note `m` (A4 = 69 = 440 Hz).
#[must_use]
pub fn midi_to_hz(m: f64) -> f64 {
    440.0 * ((m - 69.0) / 12.0).exp2()
}

/// Fractional MIDI number of `hz`.
#[must_use]
pub fn hz_to_midi(hz: f64) -> f64 {
    69.0 + 12.0 * (hz / 440.0).log2()
}

/// `"A3"`, `"C#4"`, `"Db2"` (A4 = 440 Hz) to Hz.
#[must_use]
pub fn parse_note(s: &str) -> Option<f64> {
    let s = s.trim();
    let mut chars = s.chars();
    let letter = chars.next()?.to_ascii_uppercase();
    let base = match letter {
        'C' => 0,
        'D' => 2,
        'E' => 4,
        'F' => 5,
        'G' => 7,
        'A' => 9,
        'B' => 11,
        _ => return None,
    };
    let rest = chars.as_str();
    let (accidental, rest) = if let Some(r) = rest.strip_prefix('#') {
        (1, r)
    } else if let Some(r) = rest.strip_prefix('b') {
        (-1, r)
    } else {
        (0, rest)
    };
    let octave: i32 = rest.parse().ok()?;
    if !(-1..=9).contains(&octave) {
        return None;
    }
    Some(midi_to_hz(f64::from((octave + 1) * 12 + base + accidental)))
}

/// Nearest note name for `hz` and the offset from it in cents, e.g. `("A3", 3.2)`.
#[must_use]
pub fn nearest_note(hz: f64) -> (String, f64) {
    let m = hz_to_midi(hz);
    let nearest = m.round();
    let idx = (nearest as i64).rem_euclid(12) as usize;
    let octave = (nearest as i64).div_euclid(12) - 1;
    (
        format!("{}{octave}", NOTE_NAMES[idx]),
        (m - nearest) * 100.0,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_units() {
        assert_eq!(parse_value("800Hz", Unit::Hz), Ok(800.0));
        assert_eq!(parse_value("1.2kHz", Unit::Hz), Ok(1200.0));
        assert_eq!(parse_value("40ms", Unit::Ms), Ok(40.0));
        assert_eq!(parse_value("0.5s", Unit::Ms), Ok(500.0));
        assert_eq!(parse_value("2000ms", Unit::Seconds), Ok(2.0));
        assert_eq!(parse_value("-6dB", Unit::Db), Ok(-6.0));
        assert_eq!(parse_value("12ct", Unit::Cents), Ok(12.0));
        assert_eq!(parse_value("3oct", Unit::Octaves), Ok(3.0));
        assert_eq!(parse_value("30%", Unit::Ratio), Ok(0.3));
        assert_eq!(parse_value("4.2", Unit::Plain), Ok(4.2));
        assert_eq!(parse_value(" 800 hz ", Unit::Hz), Ok(800.0));
    }

    #[test]
    fn rejects_wrong_units_and_text() {
        let e = parse_value("40ms", Unit::Hz).unwrap_err();
        assert!(e.contains("Hz, kHz"), "{e}");
        assert!(parse_value("loud", Unit::Db).is_err());
        assert!(parse_value("", Unit::Db).is_err());
        assert!(parse_value("12dB", Unit::Plain).is_err());
    }

    #[test]
    fn rounds_to_four_significant_digits() {
        assert_eq!(r4(1234.5678), 1235.0);
        assert_eq!(r4(0.123_456), 0.1235);
        assert_eq!(r4(-0.000_012_345_6), -0.000_012_35);
        assert_eq!(r4(0.0), 0.0);
        assert_eq!(fmt_value(1234.5, Unit::Hz), "1.235kHz");
        assert_eq!(fmt_value(999.0, Unit::Hz), "999Hz");
        assert_eq!(fmt_value(40.0, Unit::Ms), "40ms");
        assert_eq!(fmt_value(-6.0, Unit::Db), "-6dB");
    }

    #[test]
    fn notes_round_trip() {
        assert!((parse_note("A4").unwrap() - 440.0).abs() < 1e-9);
        assert!((parse_note("A3").unwrap() - 220.0).abs() < 1e-9);
        assert!((parse_note("C3").unwrap() - 130.812_78).abs() < 1e-3);
        assert!((parse_note("Db3").unwrap() - parse_note("C#3").unwrap()).abs() < 1e-9);
        assert!(parse_note("H3").is_none());
        let (name, cents) = nearest_note(221.0);
        assert_eq!(name, "A3");
        assert!((cents - 7.85).abs() < 0.1, "{cents}");
        assert_eq!(nearest_note(130.81).0, "C3");
    }
}
