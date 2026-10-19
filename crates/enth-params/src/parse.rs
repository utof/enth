//! Parsing text against a `ParamSpec` (`"800Hz"`, `"40ms"`, `"-3dB"`, `"saw"`, `"on"`), typed value errors, and
//! route depths given in percent or musical units (`"30%"`, `"+1oct"`).

use core::fmt;

use crate::scale::Scale;
use crate::spec::{EnumItem, Kind, ParamSpec};
use crate::units::{Unit, UnitError, canonical_zero};

/// Why a value is not valid for a parameter. Each variant has a stable snake-case [`code`](Self::code) for CLI
/// JSON (exit 3) and a [`hint`](Self::hint); the message names the parameter, the unit it expects and what it
/// accepts (design.md §1).
#[derive(Clone, Debug, PartialEq)]
pub enum ValueError {
    /// Not a number, enum key or on/off, whichever the parameter expects. Code `bad_value`.
    BadValue {
        /// The parameter key.
        param: &'static str,
        /// The trimmed input.
        text: String,
        /// What the parameter accepts.
        expected: Expected,
    },
    /// A unit suffix that does not belong to the parameter's unit. Code `wrong_unit`.
    WrongUnit {
        /// The parameter key.
        param: &'static str,
        /// The suffix found.
        suffix: String,
        /// The parameter's unit.
        unit: Unit,
    },
    /// A fractional value for an integer, enum or toggle parameter. Code `not_integral`.
    NotIntegral {
        /// The parameter key.
        param: &'static str,
        /// The value given.
        value: f32,
    },
    /// Outside `[min, max]`. Code `out_of_range`.
    OutOfRange {
        /// The parameter key.
        param: &'static str,
        /// The value given, in plain units.
        value: f32,
        /// Lowest valid value.
        min: f32,
        /// Highest valid value.
        max: f32,
        /// The parameter's unit.
        unit: Unit,
    },
    /// NaN, infinity, or a number too large for `f32`. Code `non_finite`.
    NonFinite {
        /// The parameter key.
        param: &'static str,
    },
}

/// What a parameter accepts as text, carried by [`ValueError::BadValue`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Expected {
    /// A number in this unit, with an optional suffix.
    Number(Unit),
    /// One of these enum item keys.
    Item(&'static [EnumItem]),
    /// `on`/`off` (or `true`/`false`, `1`/`0`).
    OnOff,
}

impl ValueError {
    /// Stable snake-case code: `bad_value`, `wrong_unit`, `not_integral`, `out_of_range`, `non_finite`.
    pub const fn code(&self) -> &'static str {
        match self {
            ValueError::BadValue { .. } => "bad_value",
            ValueError::WrongUnit { .. } => "wrong_unit",
            ValueError::NotIntegral { .. } => "not_integral",
            ValueError::OutOfRange { .. } => "out_of_range",
            ValueError::NonFinite { .. } => "non_finite",
        }
    }

    /// The parameter key the value was meant for.
    pub const fn param(&self) -> &'static str {
        match self {
            ValueError::BadValue { param, .. }
            | ValueError::WrongUnit { param, .. }
            | ValueError::NotIntegral { param, .. }
            | ValueError::OutOfRange { param, .. }
            | ValueError::NonFinite { param } => param,
        }
    }

    /// What to do about it, in one line.
    pub fn hint(&self) -> String {
        match self {
            ValueError::BadValue {
                expected: Expected::Item(items),
                ..
            } => {
                let keys: Vec<&str> = items.iter().map(|i| i.key).collect();
                format!("use one of: {}", keys.join(", "))
            }
            ValueError::BadValue {
                expected: Expected::OnOff,
                ..
            } => "use on or off".to_owned(),
            ValueError::BadValue {
                expected: Expected::Number(unit),
                ..
            }
            | ValueError::WrongUnit { unit, .. } => {
                format!(
                    "write a number in {}, e.g. {}",
                    unit.name(),
                    unit.accepted().join(", ")
                )
            }
            ValueError::NotIntegral { .. } => "use a whole number".to_owned(),
            ValueError::OutOfRange {
                value,
                min,
                max,
                unit,
                ..
            } => {
                let nearest = value.clamp(*min, *max);
                format!(
                    "the range is {min}..={max} {}; the nearest valid value is {nearest}",
                    unit.name()
                )
            }
            ValueError::NonFinite { .. } => "use a finite number".to_owned(),
        }
    }

    fn from_unit(param: &'static str, unit: Unit, e: UnitError) -> Self {
        match e {
            UnitError::NotANumber { text } => ValueError::BadValue {
                param,
                text,
                expected: Expected::Number(unit),
            },
            UnitError::WrongUnit { suffix, unit } => ValueError::WrongUnit {
                param,
                suffix,
                unit,
            },
            UnitError::NonFinite { .. } => ValueError::NonFinite { param },
        }
    }
}

impl fmt::Display for ValueError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ValueError::BadValue { param, text, .. } => {
                write!(f, "{param}: '{text}' is not a valid value; {}", self.hint())
            }
            ValueError::WrongUnit { param, suffix, .. } => {
                write!(f, "{param}: unit '{suffix}' does not fit; {}", self.hint())
            }
            ValueError::NotIntegral { param, value } => {
                write!(f, "{param}: {value} is not a whole number")
            }
            ValueError::OutOfRange { param, value, .. } => {
                write!(f, "{param}: {value} is out of range; {}", self.hint())
            }
            ValueError::NonFinite { param } => {
                write!(f, "{param}: the value is not a finite number")
            }
        }
    }
}

impl std::error::Error for ValueError {}

/// The spec of a route depth: a percentage of the destination's range in its normalised domain, −100..=100
/// (design.md §2). A depth is not a module parameter, but it parses, displays and checks like one.
pub const DEPTH: ParamSpec = ParamSpec::float(
    "depth",
    "Depth",
    Unit::Percent,
    -100.0,
    100.0,
    Scale::Linear,
    0.0,
)
.not_automatable()
.intermediate("How far a route moves its destination, as a percentage of the destination's range.");

const _: () = crate::spec::check_table("DEPTH", &[DEPTH], &[]);

impl ParamSpec {
    /// Parse text into a valid plain value: a number in the spec's unit with an optional suffix (see
    /// [`Unit::parse`]), an enum key (case-insensitive), or `on`/`off`, `true`/`false`, `1`/`0` for a toggle.
    /// The result passes [`ParamSpec::check`].
    ///
    /// ```
    /// use enth_params::{ParamSpec, Scale, Unit};
    /// let cutoff = ParamSpec::float("cutoff", "Cutoff", Unit::Hz, 20.0, 20_000.0, Scale::Log, 2_000.0).basic("…");
    /// assert_eq!(cutoff.parse("1.2kHz"), Ok(1200.0));
    /// assert_eq!(cutoff.parse("A4"), Ok(440.0));
    /// assert_eq!(cutoff.parse("40ms").unwrap_err().code(), "wrong_unit");
    /// assert_eq!(cutoff.parse("30kHz").unwrap_err().code(), "out_of_range");
    /// ```
    pub fn parse(&self, text: &str) -> Result<f32, ValueError> {
        let t = text.trim();
        let value = match self.kind {
            Kind::Enum(items) => match items.iter().position(|i| i.key.eq_ignore_ascii_case(t)) {
                Some(i) => i as f32,
                None => {
                    return Err(ValueError::BadValue {
                        param: self.key,
                        text: t.to_owned(),
                        expected: Expected::Item(items),
                    });
                }
            },
            Kind::Toggle => match t.to_ascii_lowercase().as_str() {
                "on" | "true" | "1" | "yes" => 1.0,
                "off" | "false" | "0" | "no" => 0.0,
                _ => {
                    return Err(ValueError::BadValue {
                        param: self.key,
                        text: t.to_owned(),
                        expected: Expected::OnOff,
                    });
                }
            },
            Kind::Float { .. } | Kind::Int { .. } => self
                .unit
                .parse(t)
                .map_err(|e| ValueError::from_unit(self.key, self.unit, e))?,
        };
        self.check(value)
    }

    /// A route depth from text, in percent of this (destination) parameter's range in its normalised domain, valid
    /// for [`DEPTH`]. `%` or a bare number is the depth itself (`"30%"` → 30). Any other suffix is an interval
    /// applied to `base`, the destination's current plain value, and converted at that value:
    ///
    /// - `oct`, `st`, `ct` on a Hz parameter are frequency ratios (`+1oct` doubles the frequency);
    /// - on a cents, semitone or octave parameter they are converted and added;
    /// - other suffixes of the parameter's unit (`+200Hz`, `+50ms`, `+3dB`) are added.
    ///
    /// On a Log scale an octave is the same depth at every base: `+1oct` on 20 Hz–20 kHz is 10.034 %. On other
    /// scales the depth depends on `base`. Linear and Log scales extrapolate past the range (so the depth keeps its
    /// musical meaning); the result must still lie within ±100 %.
    ///
    /// ```
    /// use enth_params::{ParamSpec, Scale, Unit};
    /// let cutoff = ParamSpec::float("cutoff", "Cutoff", Unit::Hz, 20.0, 20_000.0, Scale::Log, 2_000.0).basic("…");
    /// let d = cutoff.depth_from_text(2_000.0, "+1oct").unwrap();
    /// assert!((d - 10.034).abs() < 1e-3);
    /// assert_eq!(cutoff.depth_from_text(2_000.0, "30%"), Ok(30.0));
    /// ```
    pub fn depth_from_text(&self, base: f32, text: &str) -> Result<f32, ValueError> {
        let t = text.trim();
        let bad = |t: &str| ValueError::BadValue {
            param: DEPTH.key,
            text: t.to_owned(),
            expected: Expected::Number(Unit::Percent),
        };
        let (_, rest) = crate::units::split_number(t).ok_or_else(|| bad(t))?;
        let suffix = rest.trim();
        if suffix.is_empty() || suffix == "%" {
            return DEPTH.parse(t);
        }
        let lower = suffix.to_ascii_lowercase();
        let interval = matches!(
            lower.as_str(),
            "oct" | "octave" | "octaves" | "st" | "semi" | "semis" | "ct" | "cent" | "cents"
        );
        let base64 = f64::from(base);
        let target = if self.unit == Unit::Hz && interval {
            let octaves = Unit::Octaves
                .parse(t)
                .map_err(|e| ValueError::from_unit(DEPTH.key, Unit::Octaves, e))?;
            base64 * libm::exp2(f64::from(octaves))
        } else {
            let delta = self
                .unit
                .parse(t)
                .map_err(|e| ValueError::from_unit(self.key, self.unit, e))?;
            base64 + f64::from(delta)
        };
        let (lo, hi) = (f64::from(self.min()), f64::from(self.max()));
        let scale = self.scale().unwrap_or(Scale::Linear);
        let b = base64.clamp(lo, hi);
        let depth = 100.0 * (scale.to_norm_f64(lo, hi, target) - scale.to_norm_f64(lo, hi, b));
        let depth = depth as f32;
        if !depth.is_finite() {
            return Err(ValueError::NonFinite { param: DEPTH.key });
        }
        DEPTH.check(canonical_zero(depth))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::{EnumItem, Tier};

    const MODES: [EnumItem; 2] = [
        EnumItem::new("lp", "Low-pass", "x"),
        EnumItem::new("hp", "High-pass", "x"),
    ];

    fn cutoff() -> ParamSpec {
        ParamSpec::float(
            "cutoff",
            "Cutoff",
            Unit::Hz,
            20.0,
            20_000.0,
            Scale::Log,
            2_000.0,
        )
        .learn(Tier::Basic, "x")
    }

    #[test]
    fn parses_by_kind() {
        let mode = ParamSpec::enumeration("mode", "Mode", &MODES, 0).basic("x");
        assert_eq!(mode.parse("HP"), Ok(1.0));
        let e = mode.parse("bp").unwrap_err();
        assert_eq!(e.code(), "bad_value");
        assert!(e.hint().contains("lp, hp"), "{}", e.hint());
        assert!(mode.parse("1").is_err(), "enums take names, not indices");
        let sync = ParamSpec::toggle("sync", "Sync", false).basic("x");
        assert_eq!(sync.parse("on"), Ok(1.0));
        assert_eq!(sync.parse("False"), Ok(0.0));
        assert_eq!(sync.parse("maybe").unwrap_err().hint(), "use on or off");
        let oct = ParamSpec::int("octave", "Octave", Unit::Octaves, -3, 3, 0).basic("x");
        assert_eq!(oct.parse("+1oct"), Ok(1.0));
        assert_eq!(oct.parse("-24st"), Ok(-2.0));
        assert_eq!(oct.parse("6st").unwrap_err().code(), "not_integral");
        assert_eq!(oct.parse("4").unwrap_err().code(), "out_of_range");
        assert_eq!(cutoff().parse("loud").unwrap_err().code(), "bad_value");
        assert_eq!(cutoff().parse("1e39").unwrap_err().code(), "non_finite");
    }

    #[test]
    fn error_messages_name_the_parameter_and_unit() {
        let e = cutoff().parse("40ms").unwrap_err();
        let msg = e.to_string();
        assert!(msg.starts_with("cutoff:"), "{msg}");
        assert!(msg.contains("kHz") && msg.contains("A4"), "{msg}");
        let e = cutoff().parse("30kHz").unwrap_err();
        assert!(e.hint().contains("20000"), "{}", e.hint());
    }

    #[test]
    fn depth_in_musical_units() {
        let c = cutoff();
        let one_oct = c.depth_from_text(2_000.0, "+1oct").unwrap();
        // log2(2) / log2(1000) = 0.100343
        assert!((one_oct - 10.034_3).abs() < 1e-4, "{one_oct}");
        // The same at any base, even near the top (extrapolated).
        assert!((c.depth_from_text(15_000.0, "+1oct").unwrap() - one_oct).abs() < 1e-4);
        assert!((c.depth_from_text(2_000.0, "+12st").unwrap() - one_oct).abs() < 1e-4);
        assert!((c.depth_from_text(2_000.0, "-1200ct").unwrap() + one_oct).abs() < 1e-4);
        assert_eq!(c.depth_from_text(2_000.0, "-30%"), Ok(-30.0));
        assert_eq!(c.depth_from_text(2_000.0, "30"), Ok(30.0));
        assert_eq!(
            c.depth_from_text(2_000.0, "150%").unwrap_err().code(),
            "out_of_range"
        );
        assert_eq!(
            c.depth_from_text(2_000.0, "+11oct").unwrap_err().code(),
            "out_of_range"
        );
        assert_eq!(
            c.depth_from_text(2_000.0, "+3dB").unwrap_err().code(),
            "wrong_unit"
        );
        // A semitone parameter: intervals add.
        let semi = ParamSpec::float(
            "semi",
            "Semi",
            Unit::Semitones,
            -48.0,
            48.0,
            Scale::Linear,
            0.0,
        )
        .basic("x");
        assert!((semi.depth_from_text(0.0, "+12st").unwrap() - 12.5).abs() < 1e-5);
        assert!((semi.depth_from_text(0.0, "+1oct").unwrap() - 12.5).abs() < 1e-5);
        // A linear dB parameter: deltas add.
        let db = ParamSpec::float("level", "Level", Unit::Db, -36.0, 12.0, Scale::Linear, 0.0)
            .basic("x");
        assert!((db.depth_from_text(0.0, "+12dB").unwrap() - 25.0).abs() < 1e-5);
    }
}
