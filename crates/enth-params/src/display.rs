//! Formatting of plain values with their unit, the inverse of parsing.
//!
//! Two forms:
//!
//! - **exact** ([`ParamSpec::display`], [`Unit::display`]): the shortest text that parses back to the same `f32`
//!   bits, so `spec.parse(&spec.display(x).to_string()) == Ok(x)` for every valid `x`. Hz from 1 kHz up is shown in
//!   kHz and ms from 1 s up in seconds, by moving the decimal point in the text, never by float arithmetic;
//! - **rounded** ([`ParamSpec::display_rounded`]): `digits` significant digits (1–7), for humans and the CLI's
//!   four-digit convention (`1.235kHz`). Re-parsing and re-displaying gives the same text (up to 7 digits an
//!   `f32` always holds the rounded decimal).
//!
//! Positive intervals (cents, semitones, octaves) carry a `+`. Enums display their item key, toggles `on`/`off`.

use core::fmt;

use crate::spec::{Kind, ParamSpec};
use crate::units::Unit;

/// A plain value with its unit, ready to print. Made by [`Unit::display`]; prints the exact form.
#[derive(Clone, Copy, Debug)]
pub struct UnitValue {
    unit: Unit,
    value: f32,
    digits: Option<u8>,
}

/// A parameter value, ready to print. Made by [`ParamSpec::display`] or [`ParamSpec::display_rounded`].
#[derive(Clone, Copy, Debug)]
pub struct Formatted<'a> {
    spec: &'a ParamSpec,
    value: f32,
    digits: Option<u8>,
}

impl Unit {
    /// The exact display of a plain value in this unit (`1.2kHz`, `40ms`, `1.5s`, `+7st`, `30%`, `4:1`).
    pub fn display(self, value: f32) -> UnitValue {
        UnitValue {
            unit: self,
            value,
            digits: None,
        }
    }

    /// The display rounded to `digits` significant digits (clamped to 1..=7).
    pub fn display_rounded(self, value: f32, digits: u8) -> UnitValue {
        UnitValue {
            unit: self,
            value,
            digits: Some(digits.clamp(1, 7)),
        }
    }
}

impl ParamSpec {
    /// The exact display of a value: it parses back to the same bits.
    ///
    /// ```
    /// use enth_params::{ParamSpec, Scale, Unit};
    /// let cutoff = ParamSpec::float("cutoff", "Cutoff", Unit::Hz, 20.0, 20_000.0, Scale::Log, 2_000.0).basic("…");
    /// assert_eq!(cutoff.display(1234.5).to_string(), "1.2345kHz");
    /// assert_eq!(cutoff.display_rounded(1234.5, 4).to_string(), "1.234kHz"); // ties round to even
    /// let x = 1234.5678_f32; // stored as 1234.5677…
    /// assert_eq!(cutoff.parse(&cutoff.display(x).to_string()).map(f32::to_bits), Ok(x.to_bits()));
    /// ```
    pub fn display(&self, value: f32) -> Formatted<'_> {
        Formatted {
            spec: self,
            value,
            digits: None,
        }
    }

    /// The display rounded to `digits` significant digits (clamped to 1..=7); use [`display`](Self::display)
    /// for the exact value.
    pub fn display_rounded(&self, value: f32, digits: u8) -> Formatted<'_> {
        Formatted {
            spec: self,
            value,
            digits: Some(digits.clamp(1, 7)),
        }
    }
}

impl fmt::Display for Formatted<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.spec.kind {
            Kind::Enum(_) => match self.spec.enum_item(self.value) {
                Some(item) => f.write_str(item.key),
                // Not a valid index: show the number and a `?` so the problem is visible.
                None => write!(f, "{}?", self.value),
            },
            Kind::Toggle => f.write_str(if self.value >= 0.5 { "on" } else { "off" }),
            Kind::Float { .. } | Kind::Int { .. } => fmt::Display::fmt(
                &UnitValue {
                    unit: self.spec.unit,
                    value: self.value,
                    digits: self.digits,
                },
                f,
            ),
        }
    }
}

impl fmt::Display for UnitValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let v = self.value;
        if !v.is_finite() {
            return write!(f, "{v}");
        }
        let text = match self.digits {
            None => format!("{v}"),
            Some(d) => round_sig(v, d),
        };
        let (sign, digits) = match text.strip_prefix('-') {
            Some(rest) if rest.bytes().any(|b| b != b'0' && b != b'.') => ("-", rest),
            Some(rest) => ("", rest),
            None => (
                if self.unit.signed() && v > 0.0 {
                    "+"
                } else {
                    ""
                },
                text.as_str(),
            ),
        };
        // Decide kHz and seconds on the value as shown, so 999.95 rounded to 4 digits reads "1kHz".
        let large = digits.parse::<f64>().is_ok_and(|x| x >= 1000.0);
        let (body, suffix) = match self.unit {
            Unit::Hz if large => (shift_decimal(digits, -3), "kHz"),
            Unit::Ms if large => (shift_decimal(digits, -3), "s"),
            u => (digits.to_owned(), u.suffix()),
        };
        write!(f, "{sign}{body}{suffix}")
    }
}

/// `v` rounded to `digits` significant digits, as plain decimal text (`1234.5678`, 4 → `"1235"`).
fn round_sig(v: f32, digits: u8) -> String {
    let sci = format!("{:.*e}", usize::from(digits - 1), f64::from(v));
    sci_to_decimal(&sci)
}

/// `"-1.235e3"` → `"-1235"`, `"1.000e-1"` → `"0.1"`: scientific text from `{:e}` to plain decimal text, without
/// trailing zeros.
fn sci_to_decimal(sci: &str) -> String {
    let (mantissa, exp) = sci.split_once('e').unwrap_or((sci, "0"));
    let exp: i32 = exp.parse().unwrap_or(0);
    let (sign, mantissa) = match mantissa.strip_prefix('-') {
        Some(m) => ("-", m),
        None => ("", mantissa),
    };
    let out = shift_decimal(mantissa, exp);
    if out == "0" {
        out
    } else {
        format!("{sign}{out}")
    }
}

/// Move the decimal point of an unsigned decimal text (`"1234.5678"`) by `shift` places (positive = right), and
/// normalise: no leading zeros before the point except one, no trailing zeros after it, no trailing point.
fn shift_decimal(text: &str, shift: i32) -> String {
    let (int, frac) = text.split_once('.').unwrap_or((text, ""));
    let digits: String = [int, frac].concat();
    // Position of the decimal point within `digits`, after shifting.
    let point = int.len() as i64 + i64::from(shift);
    let mut s = String::new();
    if point <= 0 {
        s.push_str("0.");
        for _ in 0..(-point) {
            s.push('0');
        }
        s.push_str(&digits);
    } else if point as usize >= digits.len() {
        s.push_str(&digits);
        for _ in 0..(point as usize - digits.len()) {
            s.push('0');
        }
    } else {
        s.push_str(&digits[..point as usize]);
        s.push('.');
        s.push_str(&digits[point as usize..]);
    }
    normalise_decimal(&s)
}

fn normalise_decimal(s: &str) -> String {
    let (int, frac) = s.split_once('.').unwrap_or((s, ""));
    let int = int.trim_start_matches('0');
    let int = if int.is_empty() { "0" } else { int };
    let frac = frac.trim_end_matches('0');
    if frac.is_empty() {
        int.to_owned()
    } else {
        format!("{int}.{frac}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decimal_shifts() {
        assert_eq!(shift_decimal("1234.5678", -3), "1.2345678");
        assert_eq!(shift_decimal("1000", -3), "1");
        assert_eq!(shift_decimal("20000", -3), "20");
        assert_eq!(shift_decimal("12", -3), "0.012");
        assert_eq!(shift_decimal("1.5", 3), "1500");
        assert_eq!(shift_decimal("0.001", 0), "0.001");
        assert_eq!(sci_to_decimal("1.235e3"), "1235");
        assert_eq!(sci_to_decimal("1.000e-1"), "0.1");
        assert_eq!(sci_to_decimal("-6.000e0"), "-6");
        assert_eq!(sci_to_decimal("0.000e0"), "0");
        assert_eq!(sci_to_decimal("-0.000e0"), "0");
    }

    #[test]
    fn exact_display() {
        assert_eq!(Unit::Hz.display(800.0).to_string(), "800Hz");
        assert_eq!(Unit::Hz.display(1200.0).to_string(), "1.2kHz");
        assert_eq!(Unit::Hz.display(20_000.0).to_string(), "20kHz");
        assert_eq!(Unit::Hz.display(0.01).to_string(), "0.01Hz");
        assert_eq!(Unit::Ms.display(40.0).to_string(), "40ms");
        assert_eq!(Unit::Ms.display(1500.0).to_string(), "1.5s");
        assert_eq!(Unit::Db.display(-6.0).to_string(), "-6dB");
        assert_eq!(Unit::Db.display(0.0).to_string(), "0dB");
        assert_eq!(Unit::Semitones.display(7.0).to_string(), "+7st");
        assert_eq!(Unit::Semitones.display(-12.0).to_string(), "-12st");
        assert_eq!(Unit::Semitones.display(0.0).to_string(), "0st");
        assert_eq!(Unit::Cents.display(12.5).to_string(), "+12.5ct");
        assert_eq!(Unit::Percent.display(30.0).to_string(), "30%");
        assert_eq!(Unit::Ratio.display(4.0).to_string(), "4:1");
        assert_eq!(Unit::Degrees.display(90.0).to_string(), "90deg");
        assert_eq!(Unit::None.display(2.5).to_string(), "2.5");
        assert_eq!(Unit::None.display(1e-7).to_string(), "0.0000001");
    }

    #[test]
    fn rounded_display() {
        assert_eq!(
            Unit::Hz.display_rounded(1234.5678, 4).to_string(),
            "1.235kHz"
        );
        assert_eq!(Unit::Hz.display_rounded(999.95, 4).to_string(), "1kHz");
        assert_eq!(
            Unit::Hz.display_rounded(632.455_5, 4).to_string(),
            "632.5Hz"
        );
        assert_eq!(
            Unit::Ms.display_rounded(0.123_456, 4).to_string(),
            "0.1235ms"
        );
        assert_eq!(
            Unit::Db.display_rounded(-0.000_01, 4).to_string(),
            "-0.00001dB"
        );
        assert_eq!(Unit::Cents.display_rounded(0.0, 4).to_string(), "0ct");
    }
}
