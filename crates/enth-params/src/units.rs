//! `Unit` and its suffix tables: Hz, ms, dB, cents, semitones, octaves, %, ratio, degrees, and note names for Hz.
//!
//! A parameter has one stored unit. Text in that unit may carry no suffix (a bare number is in the stored unit:
//! `30` on a percent parameter is 30 %, never 0.3), the stored unit's own suffix, or a related suffix that is
//! converted (`1.2kHz` → 1200 Hz, `0.5s` → 500 ms, `+1oct` → 12 st on a semitone parameter). Hz parameters also
//! accept note names (`A4` = 440 Hz, `C#3`, `Db2`). Anything else is a [`UnitError::WrongUnit`] that lists what the
//! unit accepts.
//!
//! Conversions by a power of ten (`kHz`, `s`, `ct` ⇄ `st`) are done on the decimal text, so they are exact: the
//! number is parsed once, correctly rounded, straight to `f32`. That is what makes `parse(display(x)) == x` hold
//! bit for bit. Other conversions (octaves ⇄ semitones, note names) go through `f64` and the pinned `libm`.

use core::fmt;

/// The stored unit of a parameter. Patches hold plain values in this unit (design.md §1).
///
/// The set is closed per catalogue version: adding a unit is additive (the enum is `#[non_exhaustive]`), changing
/// a parameter's unit is a patch migration.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum Unit {
    /// Hertz. Accepts `Hz`, `kHz` and note names (`A4` = 440 Hz).
    Hz,
    /// Milliseconds. Accepts `ms` and `s`; displays seconds from 1,000 ms up.
    Ms,
    /// Decibels. Accepts `dB`.
    Db,
    /// Cents (1/100 semitone). Accepts `ct`, `st` and `oct`.
    Cents,
    /// Semitones. Accepts `st`, `ct` and `oct`.
    Semitones,
    /// Octaves. Accepts `oct`, `st` and `ct`.
    Octaves,
    /// Percent. A bare number is a percent (`30` = 30 %), never a 0–1 fraction. Accepts `%`.
    Percent,
    /// A ratio such as a compressor's `4:1`; stored as the left-hand number. Accepts `:1`.
    Ratio,
    /// Degrees of phase, 0–360. Accepts `deg` and `°`.
    Degrees,
    /// No unit: a bare number. Enum and toggle parameters use this too.
    None,
}

/// Suffixes of the interval units, lowercase.
const CT: &[&str] = &["ct", "cent", "cents"];
const ST: &[&str] = &["st", "semi", "semis"];
const OCT: &[&str] = &["oct", "octave", "octaves"];

/// How a suffix converts into the stored unit.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Factor {
    /// Multiply by `10^k`: done on the decimal text, so exact.
    Pow10(i32),
    /// Multiply in `f64`.
    Mul(f64),
    /// Divide in `f64`.
    Div(f64),
}

impl Unit {
    /// The suffix used when displaying a value (`"Hz"`, `"ms"`, `"%"`); empty for [`Unit::None`].
    pub const fn suffix(self) -> &'static str {
        match self {
            Unit::Hz => "Hz",
            Unit::Ms => "ms",
            Unit::Db => "dB",
            Unit::Cents => "ct",
            Unit::Semitones => "st",
            Unit::Octaves => "oct",
            Unit::Percent => "%",
            Unit::Ratio => ":1",
            Unit::Degrees => "deg",
            Unit::None => "",
        }
    }

    /// A stable lowercase name for JSON and docs (`"hz"`, `"ms"`, `"db"`, `"cents"`, …).
    pub const fn name(self) -> &'static str {
        match self {
            Unit::Hz => "hz",
            Unit::Ms => "ms",
            Unit::Db => "db",
            Unit::Cents => "cents",
            Unit::Semitones => "semitones",
            Unit::Octaves => "octaves",
            Unit::Percent => "percent",
            Unit::Ratio => "ratio",
            Unit::Degrees => "degrees",
            Unit::None => "none",
        }
    }

    /// What text in this unit may look like, for error messages and `enth params` (`["800Hz", "1.2kHz", "A4"]`).
    pub const fn accepted(self) -> &'static [&'static str] {
        match self {
            Unit::Hz => &["800", "800Hz", "1.2kHz", "A4", "C#3", "Db2"],
            Unit::Ms => &["40", "40ms", "0.5s"],
            Unit::Db => &["-6", "-6dB"],
            Unit::Cents => &["12", "12ct", "+1st", "+1oct"],
            Unit::Semitones => &["7", "+7st", "50ct", "+1oct"],
            Unit::Octaves => &["1", "+1oct", "12st", "1200ct"],
            Unit::Percent => &["30", "30%"],
            Unit::Ratio => &["4", "4:1"],
            Unit::Degrees => &["90", "90deg"],
            Unit::None => &["a plain number"],
        }
    }

    /// Whether display writes a `+` before positive values (intervals: cents, semitones, octaves).
    pub const fn signed(self) -> bool {
        matches!(self, Unit::Cents | Unit::Semitones | Unit::Octaves)
    }

    /// The factor from `suffix` (already trimmed) into this unit, or `None` if the suffix does not belong here.
    fn factor(self, suffix: &str) -> Option<Factor> {
        if suffix.is_empty() {
            return Some(Factor::Pow10(0));
        }
        let s = suffix.to_ascii_lowercase();
        let s = s.as_str();
        let f = match self {
            Unit::Hz => match s {
                "hz" => Factor::Pow10(0),
                "khz" => Factor::Pow10(3),
                _ => return None,
            },
            Unit::Ms => match s {
                "ms" => Factor::Pow10(0),
                "s" | "sec" => Factor::Pow10(3),
                _ => return None,
            },
            Unit::Db => match s {
                "db" => Factor::Pow10(0),
                _ => return None,
            },
            Unit::Cents if CT.contains(&s) => Factor::Pow10(0),
            Unit::Cents if ST.contains(&s) => Factor::Pow10(2),
            Unit::Cents if OCT.contains(&s) => Factor::Mul(1200.0),
            Unit::Semitones if ST.contains(&s) => Factor::Pow10(0),
            Unit::Semitones if CT.contains(&s) => Factor::Pow10(-2),
            Unit::Semitones if OCT.contains(&s) => Factor::Mul(12.0),
            Unit::Octaves if OCT.contains(&s) => Factor::Pow10(0),
            Unit::Octaves if ST.contains(&s) => Factor::Div(12.0),
            Unit::Octaves if CT.contains(&s) => Factor::Div(1200.0),
            Unit::Percent if s == "%" => Factor::Pow10(0),
            Unit::Ratio if s == ":1" => Factor::Pow10(0),
            Unit::Degrees if s == "deg" || s == "°" || s == "degrees" => Factor::Pow10(0),
            _ => return None,
        };
        Some(f)
    }

    /// Parse `text` as a value in this unit (see the module docs). The result is finite and never `-0.0`.
    ///
    /// ```
    /// use enth_params::Unit;
    /// assert_eq!(Unit::Hz.parse("1.2kHz"), Ok(1200.0));
    /// assert_eq!(Unit::Hz.parse("A4"), Ok(440.0));
    /// assert_eq!(Unit::Ms.parse("0.5s"), Ok(500.0));
    /// assert_eq!(Unit::Semitones.parse("+1oct"), Ok(12.0));
    /// assert_eq!(Unit::Percent.parse("30"), Ok(30.0));
    /// assert!(Unit::Hz.parse("40ms").is_err());
    /// ```
    pub fn parse(self, text: &str) -> Result<f32, UnitError> {
        let t = text.trim();
        if self == Unit::Hz
            && let Some(hz) = parse_note(t)
        {
            return Ok(hz);
        }
        let (num, rest) =
            split_number(t).ok_or_else(|| UnitError::NotANumber { text: t.to_owned() })?;
        let suffix = rest.trim();
        let factor = self.factor(suffix).ok_or_else(|| UnitError::WrongUnit {
            suffix: suffix.to_owned(),
            unit: self,
        })?;
        let v = match factor {
            Factor::Pow10(k) => parse_scaled(num, k),
            Factor::Mul(m) => num.parse::<f64>().ok().map(|x| (x * m) as f32),
            Factor::Div(d) => num.parse::<f64>().ok().map(|x| (x / d) as f32),
        };
        match v {
            Some(v) if v.is_finite() => Ok(canonical_zero(v)),
            _ => Err(UnitError::NonFinite { text: t.to_owned() }),
        }
    }
}

impl fmt::Display for Unit {
    /// The display suffix, or `none` for [`Unit::None`].
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Unit::None => f.write_str("none"),
            u => f.write_str(u.suffix()),
        }
    }
}

/// Why a text is not a value in a unit. [`crate::ValueError`] adds the parameter it was meant for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UnitError {
    /// The text does not start with a number (and is not a note name where notes are allowed).
    NotANumber {
        /// The trimmed input.
        text: String,
    },
    /// The suffix does not belong to the unit (`40ms` for a Hz parameter).
    WrongUnit {
        /// The suffix found after the number.
        suffix: String,
        /// The unit that was expected.
        unit: Unit,
    },
    /// The number overflows `f32` (`1e39`).
    NonFinite {
        /// The trimmed input.
        text: String,
    },
}

impl fmt::Display for UnitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UnitError::NotANumber { text } => {
                write!(f, "'{text}' is not a number with an optional unit")
            }
            UnitError::WrongUnit { suffix, unit } => {
                write!(
                    f,
                    "unit '{suffix}' does not fit here; expected {} (e.g. {})",
                    unit.name(),
                    unit.accepted().join(", ")
                )
            }
            UnitError::NonFinite { text } => write!(f, "'{text}' is too large"),
        }
    }
}

impl std::error::Error for UnitError {}

/// `-0.0` becomes `+0.0`; everything else is unchanged. Patches never hold `-0.0` (design.md §2).
pub(crate) const fn canonical_zero(v: f32) -> f32 {
    if v == 0.0 { 0.0 } else { v }
}

/// Split `"-12.5kHz"` into `("-12.5", "kHz")`. The number is `[+-]?(digits[.digits]|.digits)([eE][+-]?digits)?`;
/// words such as `inf` or `NaN`, which `str::parse` would accept, are not numbers here.
pub(crate) fn split_number(s: &str) -> Option<(&str, &str)> {
    let b = s.as_bytes();
    let mut i = 0;
    if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
        i += 1;
    }
    let int_start = i;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
    }
    let mut digits = i - int_start;
    if i < b.len() && b[i] == b'.' {
        i += 1;
        let frac_start = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        digits += i - frac_start;
    }
    if digits == 0 {
        return None;
    }
    // An exponent only if digits follow, so "2e" stays a number with suffix "e".
    if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
        let mut j = i + 1;
        if j < b.len() && (b[j] == b'+' || b[j] == b'-') {
            j += 1;
        }
        let exp_start = j;
        while j < b.len() && b[j].is_ascii_digit() {
            j += 1;
        }
        if j > exp_start {
            i = j;
        }
    }
    Some((&s[..i], &s[i..]))
}

/// Parse a validated number literal multiplied by `10^k`, by shifting its decimal exponent in the text, so the
/// result is the correctly rounded `f32` of the exact decimal value.
fn parse_scaled(num: &str, k: i32) -> Option<f32> {
    let num = num.strip_prefix('+').unwrap_or(num);
    if k == 0 {
        return num.parse::<f32>().ok();
    }
    let (mantissa, exp) = match num.find(['e', 'E']) {
        Some(p) => (&num[..p], num[p + 1..].parse::<i64>().ok()?),
        None => (num, 0),
    };
    let exp = exp.checked_add(i64::from(k))?;
    format!("{mantissa}e{exp}").parse::<f32>().ok()
}

const NOTE_NAMES: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];

/// Frequency in Hz of a (fractional) MIDI note number; A4 = 69 = 440 Hz. Through the pinned `libm`.
pub fn midi_to_hz(note: f64) -> f64 {
    440.0 * libm::exp2((note - 69.0) / 12.0)
}

/// The fractional MIDI note number of a frequency in Hz (A4 = 69). Through the pinned `libm`.
pub fn hz_to_midi(hz: f64) -> f64 {
    69.0 + 12.0 * libm::log2(hz / 440.0)
}

/// The MIDI note number of a note name: `"A4"` → 69, `"C#3"`, `"Db2"`, `"c-1"` → 0. The letter is
/// case-insensitive; the accidental is `#` or `b`; octaves run from -1 to 9 (MIDI 0–127 and a little beyond).
pub fn note_number(name: &str) -> Option<i32> {
    let mut chars = name.chars();
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
    if rest.is_empty() || rest.starts_with('+') {
        return None;
    }
    let octave: i32 = rest.parse().ok()?;
    if !(-1..=9).contains(&octave) {
        return None;
    }
    Some((octave + 1) * 12 + base + accidental)
}

/// A note name as a frequency in Hz (`A4` = 440 Hz), rounded to `f32`.
pub fn parse_note(name: &str) -> Option<f32> {
    note_number(name).map(|m| midi_to_hz(f64::from(m)) as f32)
}

/// The nearest note name to a frequency and the offset from it in cents: `221.0` → `("A3", 7.85)`.
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
    fn parses_each_unit() {
        assert_eq!(Unit::Hz.parse("800Hz"), Ok(800.0));
        assert_eq!(Unit::Hz.parse("800"), Ok(800.0));
        assert_eq!(Unit::Hz.parse(" 800 hz "), Ok(800.0));
        assert_eq!(Unit::Hz.parse("1.2kHz"), Ok(1200.0));
        assert_eq!(Unit::Ms.parse("40ms"), Ok(40.0));
        assert_eq!(Unit::Ms.parse("0.5s"), Ok(500.0));
        assert_eq!(Unit::Db.parse("-6dB"), Ok(-6.0));
        assert_eq!(Unit::Db.parse("-6"), Ok(-6.0));
        assert_eq!(Unit::Semitones.parse("+2st"), Ok(2.0));
        assert_eq!(Unit::Semitones.parse("50ct"), Ok(0.5));
        assert_eq!(Unit::Semitones.parse("+1oct"), Ok(12.0));
        assert_eq!(Unit::Cents.parse("12ct"), Ok(12.0));
        assert_eq!(Unit::Cents.parse("+1st"), Ok(100.0));
        assert_eq!(Unit::Cents.parse("-1oct"), Ok(-1200.0));
        assert_eq!(Unit::Octaves.parse("+1oct"), Ok(1.0));
        assert_eq!(Unit::Octaves.parse("6st"), Ok(0.5));
        assert_eq!(Unit::Percent.parse("30%"), Ok(30.0));
        assert_eq!(Unit::Percent.parse("30"), Ok(30.0));
        assert_eq!(Unit::Ratio.parse("4:1"), Ok(4.0));
        assert_eq!(Unit::Degrees.parse("90deg"), Ok(90.0));
        assert_eq!(Unit::Degrees.parse("90°"), Ok(90.0));
        assert_eq!(Unit::None.parse("2.5"), Ok(2.5));
        assert_eq!(Unit::None.parse("1e3"), Ok(1000.0));
        assert_eq!(Unit::Hz.parse("2e3Hz"), Ok(2000.0));
        assert_eq!(Unit::Hz.parse("1.5e1kHz"), Ok(15000.0));
        assert_eq!(Unit::Hz.parse(".5kHz"), Ok(500.0));
    }

    #[test]
    fn power_of_ten_suffixes_are_exact() {
        // 1.2 * 1000 in f32 or f64 arithmetic can be off by an ulp; the textual shift is not.
        assert_eq!(
            Unit::Hz.parse("1.2345678kHz").unwrap().to_bits(),
            1234.5678f32.to_bits()
        );
        assert_eq!(
            Unit::Ms.parse("0.3s").unwrap().to_bits(),
            300.0f32.to_bits()
        );
        assert_eq!(
            Unit::Semitones.parse("33ct").unwrap().to_bits(),
            0.33f32.to_bits()
        );
    }

    #[test]
    fn notes() {
        assert_eq!(Unit::Hz.parse("A4"), Ok(440.0));
        assert_eq!(Unit::Hz.parse("a3"), Ok(220.0));
        assert_eq!(Unit::Hz.parse("Db3"), Unit::Hz.parse("C#3"));
        assert!((Unit::Hz.parse("C3").unwrap() - 130.812_78).abs() < 1e-3);
        assert_eq!(note_number("C-1"), Some(0));
        assert_eq!(note_number("G9"), Some(127));
        assert_eq!(note_number("H3"), None);
        assert_eq!(note_number("A10"), None);
        assert_eq!(note_number("A+4"), None);
        // Notes are only for Hz.
        assert!(Unit::Ms.parse("A4").is_err());
        let (name, cents) = nearest_note(221.0);
        assert_eq!(name, "A3");
        assert!((cents - 7.85).abs() < 0.1, "{cents}");
    }

    #[test]
    fn rejects_bad_text() {
        assert!(matches!(
            Unit::Hz.parse("40ms"),
            Err(UnitError::WrongUnit { .. })
        ));
        assert!(matches!(
            Unit::Percent.parse("0.3x"),
            Err(UnitError::WrongUnit { .. })
        ));
        assert!(matches!(
            Unit::None.parse("12dB"),
            Err(UnitError::WrongUnit { .. })
        ));
        assert!(matches!(
            Unit::Db.parse("loud"),
            Err(UnitError::NotANumber { .. })
        ));
        assert!(matches!(
            Unit::Db.parse(""),
            Err(UnitError::NotANumber { .. })
        ));
        assert!(matches!(
            Unit::None.parse("inf"),
            Err(UnitError::NotANumber { .. })
        ));
        assert!(matches!(
            Unit::None.parse("NaN"),
            Err(UnitError::NotANumber { .. })
        ));
        assert!(matches!(
            Unit::None.parse("-"),
            Err(UnitError::NotANumber { .. })
        ));
        assert!(matches!(
            Unit::None.parse("."),
            Err(UnitError::NotANumber { .. })
        ));
        assert!(matches!(
            Unit::None.parse("1e39"),
            Err(UnitError::NonFinite { .. })
        ));
        assert!(matches!(
            Unit::Hz.parse("1e36kHz"),
            Err(UnitError::NonFinite { .. })
        ));
        assert!(matches!(
            Unit::None.parse("2e"),
            Err(UnitError::WrongUnit { .. })
        ));
    }

    #[test]
    fn negative_zero_is_canonical() {
        assert_eq!(Unit::Db.parse("-0").unwrap().to_bits(), 0.0f32.to_bits());
        assert_eq!(
            Unit::Semitones.parse("-0ct").unwrap().to_bits(),
            0.0f32.to_bits()
        );
    }
}
