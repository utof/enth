//! Units of the descriptor vocabulary, and note names.
//!
//! Small on purpose: only the units descriptors are measured in (Hz, ms, dB, dBFS, dBTP, LUFS, cents, percent,
//! ratios) and note names (`A4` = 440 Hz). Parameter units live in `enth-params`. A value with no suffix is read in
//! the descriptor's own unit; a suffix that does not fit the quantity is an error that lists what is accepted.
//! Ported from `spikes/spike-agent/src/units.rs` (in-repo, MIT), with `libm` for the logarithms.

use serde::{Deserialize, Serialize};

/// The unit a descriptor is measured and reported in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Unit {
    Hz,
    Ms,
    Db,
    Dbfs,
    Dbtp,
    Lufs,
    Cents,
    /// A percentage, 0 to 100 (`clipped_pct`).
    Percent,
    /// A 0 to 1 ratio that may also be written as a percentage (`30%` = 0.3).
    Ratio,
    /// true or false (`loudness.clipping`).
    Bool,
}

impl Unit {
    /// Suffix used when printing a value (empty for ratios and booleans).
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Unit::Hz => "Hz",
            Unit::Ms => "ms",
            Unit::Db => "dB",
            Unit::Dbfs => "dBFS",
            Unit::Dbtp => "dBTP",
            Unit::Lufs => "LUFS",
            Unit::Cents => "ct",
            Unit::Percent => "%",
            Unit::Ratio | Unit::Bool => "",
        }
    }

    /// The suffixes accepted for this unit, for error messages.
    #[must_use]
    pub fn accepted(self) -> &'static str {
        match self {
            Unit::Hz => "Hz, kHz",
            Unit::Ms => "ms, s",
            Unit::Db => "dB",
            Unit::Dbfs => "dBFS, dB",
            Unit::Dbtp => "dBTP, dB",
            Unit::Lufs => "LUFS, LU, dB",
            Unit::Cents => "ct, st (100 ct)",
            Unit::Percent => "% or a plain number of percent",
            Unit::Ratio => "% or a plain 0..1 number",
            Unit::Bool => "true, false",
        }
    }

    /// Factor from `suffix` (case-insensitive) to this unit, or `None` if the suffix does not belong here.
    fn factor(self, suffix: &str) -> Option<f64> {
        let s = suffix.to_ascii_lowercase();
        match (self, s.as_str()) {
            (Unit::Hz, "hz")
            | (Unit::Ms, "ms")
            | (Unit::Db, "db")
            | (Unit::Dbfs, "dbfs" | "db")
            | (Unit::Dbtp, "dbtp" | "db")
            | (Unit::Lufs, "lufs" | "lu" | "db")
            | (Unit::Cents, "ct" | "cent" | "cents")
            | (Unit::Percent, "%") => Some(1.0),
            (Unit::Hz, "khz") | (Unit::Ms, "s") => Some(1000.0),
            (Unit::Cents, "st" | "semitone" | "semitones") => Some(100.0),
            (Unit::Ratio, "%") => Some(0.01),
            _ => None,
        }
    }
}

/// Split `"-12.5kHz"` into `(-12.5, "kHz")`. Accepts an exponent (`1e3Hz`).
fn split_number(s: &str) -> Option<(f64, &str)> {
    let s = s.trim();
    let b = s.as_bytes();
    let mut end = 0;
    if end < b.len() && (b[end] == b'-' || b[end] == b'+') {
        end += 1;
    }
    let digits = end;
    while end < b.len() && (b[end].is_ascii_digit() || b[end] == b'.') {
        end += 1;
    }
    if end == digits {
        return None;
    }
    // exponent, only when digits follow (so "1e" stays a bad suffix rather than a silent number)
    if end + 1 < b.len() && (b[end] == b'e' || b[end] == b'E') {
        let mut e = end + 1;
        if e < b.len() && (b[e] == b'-' || b[e] == b'+') {
            e += 1;
        }
        let exp_digits = e;
        while e < b.len() && b[e].is_ascii_digit() {
            e += 1;
        }
        if e > exp_digits {
            end = e;
        }
    }
    let n: f64 = s[..end].parse().ok()?;
    n.is_finite().then(|| (n, s[end..].trim()))
}

/// Parse `text` as a value of `unit`. The error text is ready to show to an agent.
pub fn parse_value(text: &str, unit: Unit) -> Result<f64, String> {
    if unit == Unit::Bool {
        return match text.trim().to_ascii_lowercase().as_str() {
            "true" | "yes" | "1" => Ok(1.0),
            "false" | "no" | "0" => Ok(0.0),
            _ => Err(format!("'{}' is not true or false", text.trim())),
        };
    }
    let (n, suffix) = split_number(text)
        .ok_or_else(|| format!("'{}' is not a number with an optional unit", text.trim()))?;
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

/// Round to 9 significant digits: removes floating-point noise (`1.2 * 1000`) and nothing a person typed.
#[must_use]
pub fn clean(x: f64) -> f64 {
    if x == 0.0 || !x.is_finite() {
        return if x == 0.0 { 0.0 } else { x };
    }
    let e = libm::floor(libm::log10(x.abs())) as i32;
    let shift = 8 - e;
    if !(-300..=300).contains(&shift) {
        return x;
    }
    let scale = libm::pow(10.0, f64::from(shift.abs()));
    if shift >= 0 {
        libm::round(x * scale) / scale
    } else {
        libm::round(x / scale) * scale
    }
}

const NOTE_NAMES: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];

/// Frequency of MIDI note `m` (A4 = 69 = 440 Hz).
#[must_use]
pub fn midi_to_hz(m: f64) -> f64 {
    440.0 * libm::exp2((m - 69.0) / 12.0)
}

/// Fractional MIDI number of `hz`.
#[must_use]
pub fn hz_to_midi(hz: f64) -> f64 {
    69.0 + 12.0 * libm::log2(hz / 440.0)
}

/// Interval from `b` to `a` in cents.
#[must_use]
pub fn cents(a: f64, b: f64) -> f64 {
    1200.0 * libm::log2(a / b)
}

/// `"A3"`, `"C#4"`, `"Db2"` (A4 = 440 Hz) to Hz. Octaves -1 to 9.
#[must_use]
pub fn parse_note(s: &str) -> Option<f64> {
    let s = s.trim();
    let mut chars = s.chars();
    let base = match chars.next()?.to_ascii_uppercase() {
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
    (-1..=9)
        .contains(&octave)
        .then(|| midi_to_hz(f64::from((octave + 1) * 12 + base + accidental)))
}

/// A note name (`C3`) or a frequency (`130.8Hz`, `1.2kHz`, a bare number in Hz) to Hz.
#[must_use]
pub fn note_or_hz(s: &str) -> Option<f64> {
    parse_note(s).or_else(|| parse_value(s, Unit::Hz).ok().filter(|v| *v > 0.0))
}

/// Nearest note name for `hz` and the offset from it in cents, e.g. `("A3", 3.2)`.
#[must_use]
pub fn nearest_note(hz: f64) -> (String, f64) {
    let m = hz_to_midi(hz);
    let nearest = libm::round(m);
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
        assert_eq!(parse_value("1e3", Unit::Hz), Ok(1000.0));
        assert_eq!(parse_value("40ms", Unit::Ms), Ok(40.0));
        assert_eq!(parse_value("0.5s", Unit::Ms), Ok(500.0));
        assert_eq!(parse_value("-6dB", Unit::Db), Ok(-6.0));
        assert_eq!(parse_value("-1dBTP", Unit::Dbtp), Ok(-1.0));
        assert_eq!(parse_value("-14LUFS", Unit::Lufs), Ok(-14.0));
        assert_eq!(parse_value("12ct", Unit::Cents), Ok(12.0));
        assert_eq!(parse_value("+2st", Unit::Cents), Ok(200.0));
        assert_eq!(parse_value("30%", Unit::Ratio), Ok(0.3));
        assert_eq!(parse_value("0.5%", Unit::Percent), Ok(0.5));
        assert_eq!(parse_value(" 800 hz ", Unit::Hz), Ok(800.0));
        assert_eq!(parse_value("false", Unit::Bool), Ok(0.0));
    }

    #[test]
    fn rejects_wrong_units_and_text() {
        let e = parse_value("40ms", Unit::Hz).unwrap_err();
        assert!(e.contains("Hz, kHz"), "{e}");
        assert!(parse_value("loud", Unit::Db).is_err());
        assert!(parse_value("", Unit::Db).is_err());
        assert!(parse_value("1e", Unit::Hz).is_err());
        assert!(parse_value("maybe", Unit::Bool).is_err());
        assert!(parse_value("1e999", Unit::Hz).is_err());
    }

    #[test]
    fn notes_round_trip() {
        assert!((parse_note("A4").unwrap() - 440.0).abs() < 1e-9);
        assert!((parse_note("A3").unwrap() - 220.0).abs() < 1e-9);
        assert!((parse_note("C3").unwrap() - 130.812_78).abs() < 1e-3);
        assert!((parse_note("Db3").unwrap() - parse_note("C#3").unwrap()).abs() < 1e-9);
        assert!(parse_note("H3").is_none());
        let (name, c) = nearest_note(221.0);
        assert_eq!(name, "A3");
        assert!((c - 7.85).abs() < 0.1, "{c}");
        assert_eq!(nearest_note(130.81).0, "C3");
        assert!((note_or_hz("1.2kHz").unwrap() - 1200.0).abs() < 1e-9);
        assert!((cents(440.0, 220.0) - 1200.0).abs() < 1e-9);
    }

    #[test]
    fn clean_keeps_typed_values() {
        assert_eq!(clean(1.2 * 1000.0), 1200.0);
        assert_eq!(clean(0.1 + 0.2), 0.3);
        assert_eq!(clean(-6.0), -6.0);
        assert_eq!(clean(123_456.789), 123_456.789);
    }
}
