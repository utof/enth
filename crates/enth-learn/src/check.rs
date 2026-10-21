//! **Owner: lane LM** (the types, freeze point F1l, and a basic patch evaluation); **PLY** (the predicate text
//! grammar, sound and choice evaluation). The reference is docs/learn.md, "Checks: four kinds" and "The lesson model
//! (lane LM)".
//!
//! A Do step's goal is a [`Check`]: `patch` (a list of [`Predicate`]s on the live patch, re-run after every delivered
//! edit), `sound` (an expectation list in `enth-analysis`'s grammar, rendered through a phrase as a job once the
//! patch settles), or `all` of several. The other two kinds of check are steps of their own: the Recreate score is a
//! real round (`StepKind::Recreate`) and the ear question is `StepKind::Choose`.
//!
//! **The persisted shape is structured** (tagged objects, `"kind"` first, unknown fields refused), and every atom
//! inside is text in a form the project already reads: addresses and routes as `enth-patch` prints them, values as
//! `ParamSpec::parse` reads them (`1kHz`, `-12dB`, `saw`, `on`, a bare number in the parameter's unit),
//! expectations as `enth check` reads them. So the shape is frozen at F1l without PLY's one-line grammar, which can
//! later read `filter1.cutoff < 1kHz` into the same [`Predicate`] as a convenience.
//!
//! **Q5, decided:** a sound check may pin macro positions ([`MacroPin`], rendered with the knobs set), and the
//! predicate form of the knob is `macro1` ([`Knob::Macro`], a 0–1 position).

use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

use enth_challenge::PhraseId;
use enth_params::ParamSpec;
use enth_patch::catalogue::MACRO_VALUE;
use enth_patch::{MacroIndex, ModSource, ModuleId, ParamAddr, Patch, RouteAddr};
use serde::{Deserialize, Serialize};

use crate::error::{Doc, LearnError, bad_field};
use crate::pack::KindKey;

/// The most predicates or expectation items one check may hold.
pub const MAX_ITEMS: usize = 16;

/// A control a predicate reads: a parameter (`filter1.cutoff`) or a macro knob (`macro1`, a 0–1 position).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Knob {
    /// A parameter.
    Param(ParamAddr),
    /// A macro knob.
    Macro(MacroIndex),
}

impl Knob {
    /// Its plain value in `patch` (`None` when the module is absent).
    pub fn value(&self, patch: &Patch) -> Option<f32> {
        match self {
            Knob::Param(a) => patch.value(a),
            Knob::Macro(i) => Some(patch.macros[i.get()].value),
        }
    }

    /// Its parameter spec in `patch`.
    pub fn spec(&self, patch: &Patch) -> Option<&'static ParamSpec> {
        match self {
            Knob::Param(a) => patch.spec(a),
            Knob::Macro(_) => Some(&MACRO_VALUE),
        }
    }
}

impl fmt::Display for Knob {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Knob::Param(a) => write!(f, "{a}"),
            Knob::Macro(i) => write!(f, "macro{}", i.one_based()),
        }
    }
}

impl FromStr for Knob {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let t = s.trim();
        if !t.contains('.')
            && let Ok(ModSource::Macro(i)) = t.parse::<ModSource>()
        {
            return Ok(Knob::Macro(i));
        }
        t.parse().map(Knob::Param).map_err(|e| e.to_string())
    }
}

impl Serialize for Knob {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Knob {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = <std::borrow::Cow<'de, str>>::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

/// A value as text, in the unit of the control it is compared with: `1kHz`, `-12dB`, `30%`, `saw`, `on`, `0.5`. A
/// number is read as its text (`1000` is `"1000"`), so TOML authors may write either; JSON holds the text.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Plain(String);

impl Plain {
    /// A value from text.
    pub fn new(text: impl Into<String>) -> Plain {
        Plain(text.into())
    }

    /// The text.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The plain value for `spec` (units, enum keys, `on`/`off`), as `ParamSpec::parse` reads it.
    pub fn resolve(&self, spec: &ParamSpec) -> Option<f32> {
        spec.parse(&self.0).ok()
    }

    /// A route depth in percent, −100 to 100 (`20`, `20%`, `-50%`).
    pub fn depth(&self) -> Option<f32> {
        let t = self.0.trim();
        let n: f32 = t.strip_suffix('%').unwrap_or(t).trim().parse().ok()?;
        (n.is_finite() && (-100.0..=100.0).contains(&n)).then_some(n)
    }
}

impl Serialize for Plain {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for Plain {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl serde::de::Visitor<'_> for V {
            type Value = Plain;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a value as text (1kHz, saw, on) or a number")
            }
            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Plain, E> {
                if v.trim().is_empty() {
                    return Err(E::custom("a value is not empty"));
                }
                Ok(Plain(v.to_owned()))
            }
            fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<Plain, E> {
                Ok(Plain(v.to_string()))
            }
            fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<Plain, E> {
                Ok(Plain(v.to_string()))
            }
            fn visit_f64<E: serde::de::Error>(self, v: f64) -> Result<Plain, E> {
                if v.is_finite() {
                    Ok(Plain(v.to_string()))
                } else {
                    Err(E::custom("a value is finite"))
                }
            }
        }
        d.deserialize_any(V)
    }
}

/// A comparison, written as its symbol.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Cmp {
    /// `<`
    #[serde(rename = "<")]
    Lt,
    /// `<=`
    #[serde(rename = "<=")]
    Le,
    /// `>`
    #[serde(rename = ">")]
    Gt,
    /// `>=`
    #[serde(rename = ">=")]
    Ge,
    /// `==`: exact, for enums, toggles and integers; use `within` for continuous values.
    #[serde(rename = "==")]
    Eq,
    /// `!=`
    #[serde(rename = "!=")]
    Ne,
}

impl Cmp {
    /// `a cmp b`.
    pub fn holds(self, a: f32, b: f32) -> bool {
        match self {
            Cmp::Lt => a < b,
            Cmp::Le => a <= b,
            Cmp::Gt => a > b,
            Cmp::Ge => a >= b,
            Cmp::Eq => a == b,
            Cmp::Ne => a != b,
        }
    }
}

/// One fact about the live patch. JSON: `{"kind":"value","knob":"filter1.cutoff","cmp":"<","value":"1kHz"}`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Predicate {
    /// A control compared with a value in its unit.
    Value {
        /// The control.
        knob: Knob,
        /// The comparison.
        cmp: Cmp,
        /// The value.
        value: Plain,
    },
    /// A control within `[lo, hi]`, both included.
    Within {
        /// The control.
        knob: Knob,
        /// The low end.
        lo: Plain,
        /// The high end.
        hi: Plain,
    },
    /// A route's depth (percent, signed; an absent route has depth 0) compared with a value.
    Route {
        /// The route.
        route: RouteAddr,
        /// The comparison.
        cmp: Cmp,
        /// The depth, in percent.
        depth: Plain,
    },
    /// An enabled module of this kind is in the patch.
    Has {
        /// The kind (`svf`).
        module_kind: KindKey,
    },
    /// This module is enabled (or, with `false`, disabled or absent).
    Enabled {
        /// The module.
        module: ModuleId,
        /// The state.
        enabled: bool,
    },
    /// The player turned this control across at least `min_span_pct` of its range during the step ("turn it and
    /// listen", from the edit log; host automation and loads do not count).
    Touched {
        /// The control.
        knob: Knob,
        /// The span, 1–100 % of the normalised range.
        min_span_pct: u8,
    },
}

/// How far the player turned each control during a step, in its normalised range: the lowest and highest position
/// they gave it. The player keeps it from `EditSource::Player` edits only.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TouchLog {
    spans: BTreeMap<Knob, (f32, f32)>,
}

impl TouchLog {
    /// Note `knob` at plain value `value` (normalised through `spec`).
    pub fn note(&mut self, knob: Knob, spec: &ParamSpec, value: f32) {
        let n = spec.to_norm(value);
        self.spans
            .entry(knob)
            .and_modify(|(lo, hi)| {
                *lo = lo.min(n);
                *hi = hi.max(n);
            })
            .or_insert((n, n));
    }

    /// The span `knob` covered, 0–1.
    pub fn span(&self, knob: &Knob) -> f32 {
        self.spans.get(knob).map_or(0.0, |(lo, hi)| hi - lo)
    }

    /// Forget everything (a new step).
    pub fn clear(&mut self) {
        self.spans.clear();
    }
}

impl Predicate {
    /// Whether it holds for `patch`, given what the player touched. The basic evaluation (lane LM); PLY owns its
    /// refinement. A control the patch lacks fails every comparison.
    pub fn holds(&self, patch: &Patch, touched: &TouchLog) -> bool {
        match self {
            Predicate::Value { knob, cmp, value } => {
                compare(knob, patch, value).is_some_and(|(have, want)| cmp.holds(have, want))
            }
            Predicate::Within { knob, lo, hi } => (|| {
                let spec = knob.spec(patch)?;
                let have = knob.value(patch)?;
                Some(lo.resolve(spec)? <= have && have <= hi.resolve(spec)?)
            })()
            .unwrap_or(false),
            Predicate::Route { route, cmp, depth } => {
                let have = patch
                    .routes
                    .iter()
                    .find(|r| r.src == route.src && r.dst == route.dst)
                    .map_or(0.0, |r| r.depth);
                depth.depth().is_some_and(|want| cmp.holds(have, want))
            }
            Predicate::Has { module_kind } => patch
                .modules()
                .any(|(_, m)| m.kind == module_kind.0 && m.enabled),
            Predicate::Enabled { module, enabled } => {
                patch.find(module).is_some_and(|(_, m)| m.enabled) == *enabled
            }
            Predicate::Touched { knob, min_span_pct } => {
                touched.span(knob) * 100.0 >= f32::from(*min_span_pct) - 1e-3
            }
        }
    }

    fn validate(&self, field: &str) -> Result<(), LearnError> {
        let bad = |m: String| bad_field(Doc::Lesson, field, m);
        let check_value = |knob: &Knob, v: &Plain| -> Result<(), LearnError> {
            if parses_somewhere(knob, v) {
                Ok(())
            } else {
                Err(bad(format!("{:?} is not a value of {knob}", v.as_str())))
            }
        };
        match self {
            Predicate::Value { knob, value, .. } => check_value(knob, value),
            Predicate::Within { knob, lo, hi } => {
                check_value(knob, lo)?;
                check_value(knob, hi)
            }
            Predicate::Route { depth, .. } => match depth.depth() {
                Some(_) => Ok(()),
                None => Err(bad(format!(
                    "depth {:?} is not −100 to 100 %",
                    depth.as_str()
                ))),
            },
            Predicate::Touched { min_span_pct, .. } if !(1..=100).contains(min_span_pct) => {
                Err(bad("min_span_pct is 1 to 100".to_owned()))
            }
            Predicate::Has { .. } | Predicate::Enabled { .. } | Predicate::Touched { .. } => Ok(()),
        }
    }
}

/// The patch's value of `knob` and `value` resolved in its unit.
fn compare(knob: &Knob, patch: &Patch, value: &Plain) -> Option<(f32, f32)> {
    let spec = knob.spec(patch)?;
    Some((knob.value(patch)?, value.resolve(spec)?))
}

/// Whether `value` parses for some parameter `knob` could name (the module's kind is known only against a patch, so
/// reading checks every kind with that key; the evaluation resolves the right one).
fn parses_somewhere(knob: &Knob, value: &Plain) -> bool {
    match knob {
        Knob::Macro(_) => value.resolve(&MACRO_VALUE).is_some(),
        Knob::Param(a) => match a.target() {
            enth_patch::Target::Module(_) => enth_patch::CATALOGUE
                .iter()
                .filter_map(|m| m.kind.param(a.param()))
                .any(|spec| value.resolve(spec).is_some()),
            enth_patch::Target::Voice | enth_patch::Target::Master => Patch::init()
                .spec(a)
                .is_some_and(|spec| value.resolve(spec).is_some()),
        },
    }
}

/// A macro knob held at a position while a sound check renders (Q5): the check hears the patch with `macro` at
/// `value` (0–1), whatever the knob shows.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MacroPin {
    /// The knob.
    #[serde(rename = "macro")]
    pub knob: MacroKnob,
    /// Its position, 0–1.
    pub value: Plain,
}

/// A macro knob by its text name, `macro1`…`macro8`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MacroKnob(pub MacroIndex);

impl Serialize for MacroKnob {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(&ModSource::Macro(self.0))
    }
}

impl<'de> Deserialize<'de> for MacroKnob {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = <std::borrow::Cow<'de, str>>::deserialize(d)?;
        match s.parse::<ModSource>() {
            Ok(ModSource::Macro(i)) => Ok(MacroKnob(i)),
            _ => Err(serde::de::Error::custom(format!(
                "{s:?} is not macro1…macro8"
            ))),
        }
    }
}

/// The sound half of a check: render the patch (with the pins applied) through `phrase` and test `expect`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SoundCheck {
    /// The phrase.
    pub phrase: PhraseId,
    /// The expectation items, as written (`brightness<500Hz`).
    pub expect: Vec<String>,
    /// Macro positions held during the render.
    pub macros: Vec<MacroPin>,
}

impl SoundCheck {
    /// `patch` as this check hears it: the macro pins applied.
    pub fn pinned(&self, patch: &Patch) -> Patch {
        let mut p = patch.clone();
        for pin in &self.macros {
            if let Some(v) = pin.value.resolve(&MACRO_VALUE) {
                p.macros[pin.knob.0.get()].value = v;
            }
        }
        p
    }
}

/// A Do step's goal. JSON: `{"kind":"sound","phrase":1,"expect":["brightness<500Hz"]}`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Check {
    /// Every predicate holds for the live patch (checked after each delivered edit, in microseconds).
    Patch {
        /// 1 to 16 predicates.
        predicates: Vec<Predicate>,
    },
    /// The rendered phrase meets every expectation (a job, once the patch settles). Numbers keep a margin of at
    /// least 1e-3 against the start and the witness (`lesson-check`).
    Sound {
        /// The phrase from the table.
        phrase: PhraseId,
        /// 1 to 16 expectation items in `enth-analysis`'s grammar.
        expect: Vec<String>,
        /// Macro positions held during the render (Q5).
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        macros: Vec<MacroPin>,
    },
    /// Every check passes, for the same patch revision. Not nested.
    All {
        /// 2 to 8 checks, none of them `all`.
        checks: Vec<Check>,
    },
}

impl Check {
    /// Whether every patch predicate in it holds (true when it has none).
    pub fn patch_holds(&self, patch: &Patch, touched: &TouchLog) -> bool {
        match self {
            Check::Patch { predicates } => predicates.iter().all(|p| p.holds(patch, touched)),
            Check::Sound { .. } => true,
            Check::All { checks } => checks.iter().all(|c| c.patch_holds(patch, touched)),
        }
    }

    /// Its sound checks, in order (empty for a patch-only check).
    pub fn sounds(&self) -> Vec<SoundCheck> {
        match self {
            Check::Patch { .. } => Vec::new(),
            Check::Sound {
                phrase,
                expect,
                macros,
            } => vec![SoundCheck {
                phrase: *phrase,
                expect: expect.clone(),
                macros: macros.clone(),
            }],
            Check::All { checks } => checks.iter().flat_map(Check::sounds).collect(),
        }
    }

    /// Every control its predicates read, for the editor's emphasis and `lesson-coverage`.
    pub fn knobs(&self) -> Vec<Knob> {
        match self {
            Check::Patch { predicates } => predicates
                .iter()
                .filter_map(|p| match p {
                    Predicate::Value { knob, .. }
                    | Predicate::Within { knob, .. }
                    | Predicate::Touched { knob, .. } => Some(knob.clone()),
                    _ => None,
                })
                .collect(),
            Check::Sound { .. } => Vec::new(),
            Check::All { checks } => checks.iter().flat_map(Check::knobs).collect(),
        }
    }

    /// Check what the types cannot: item counts, values that parse for their controls, expectations that parse,
    /// phrases this build has, pins at 0–1, no nested `all`.
    pub fn validate(&self, field: &str) -> Result<(), LearnError> {
        self.validate_at(field, true)
    }

    fn validate_at(&self, field: &str, top: bool) -> Result<(), LearnError> {
        let bad = |name: &str, m: String| bad_field(Doc::Lesson, format!("{field}.{name}"), m);
        match self {
            Check::Patch { predicates } => {
                if predicates.is_empty() || predicates.len() > MAX_ITEMS {
                    return Err(bad("predicates", format!("holds 1 to {MAX_ITEMS}")));
                }
                for (i, p) in predicates.iter().enumerate() {
                    p.validate(&format!("{field}.predicates[{i}]"))?;
                }
            }
            Check::Sound {
                phrase,
                expect,
                macros,
            } => {
                if enth_challenge::phrases::phrase(*phrase).is_none() {
                    return Err(bad(
                        "phrase",
                        format!("phrase {} is not in this build", phrase.0),
                    ));
                }
                if expect.is_empty() || expect.len() > MAX_ITEMS {
                    return Err(bad("expect", format!("holds 1 to {MAX_ITEMS} items")));
                }
                for (i, item) in expect.iter().enumerate() {
                    enth_analysis::expect::parse_item(item).map_err(|e| {
                        bad(
                            &format!("expect[{i}]"),
                            format!("{} ({})", e.message, e.hint),
                        )
                    })?;
                }
                for (i, pin) in macros.iter().enumerate() {
                    if pin.value.resolve(&MACRO_VALUE).is_none() {
                        return Err(bad(
                            &format!("macros[{i}].value"),
                            "is a position 0 to 1".to_owned(),
                        ));
                    }
                }
                let mut seen: Vec<MacroIndex> = macros.iter().map(|m| m.knob.0).collect();
                seen.sort_unstable();
                if seen.windows(2).any(|w| w[0] == w[1]) {
                    return Err(bad("macros", "pins a macro twice".to_owned()));
                }
            }
            Check::All { checks } => {
                if !top {
                    return Err(bad(
                        "checks",
                        "an all check is not nested in another".to_owned(),
                    ));
                }
                if !(2..=8).contains(&checks.len()) {
                    return Err(bad("checks", "holds 2 to 8 checks".to_owned()));
                }
                for (i, c) in checks.iter().enumerate() {
                    c.validate_at(&format!("{field}.checks[{i}]"), false)?;
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn init() -> Patch {
        Patch::init()
    }

    #[test]
    fn value_predicates_read_units() {
        let p = init();
        let lt = |v: &str| Predicate::Value {
            knob: "filter1.cutoff".parse().unwrap(),
            cmp: Cmp::Lt,
            value: Plain::new(v),
        };
        // Init has no filter: a missing control fails every comparison.
        assert!(!lt("1kHz").holds(&p, &TouchLog::default()));
        let wave = Predicate::Value {
            knob: "osc1.wave".parse().unwrap(),
            cmp: Cmp::Eq,
            value: Plain::new("saw"),
        };
        assert!(wave.holds(&p, &TouchLog::default()));
        let master = Predicate::Within {
            knob: "master.level".parse().unwrap(),
            lo: Plain::new("-24dB"),
            hi: Plain::new("0dB"),
        };
        assert!(master.holds(&p, &TouchLog::default()));
    }

    #[test]
    fn macros_routes_and_modules() {
        let mut p = init();
        p.macros[0].value = 1.0;
        let knob = Predicate::Value {
            knob: "macro1".parse().unwrap(),
            cmp: Cmp::Ge,
            value: Plain::new("0.5"),
        };
        assert!(knob.holds(&p, &TouchLog::default()));
        let route = Predicate::Route {
            route: "macro1 -> osc1.level".parse().unwrap(),
            cmp: Cmp::Gt,
            depth: Plain::new("20%"),
        };
        assert!(
            !route.holds(&p, &TouchLog::default()),
            "an absent route has depth 0"
        );
        let has = Predicate::Has {
            module_kind: KindKey(enth_patch::ModuleKind::Osc),
        };
        assert!(has.holds(&p, &TouchLog::default()));
        let off = Predicate::Enabled {
            module: ModuleId::new("noise1").unwrap(),
            enabled: false,
        };
        assert!(
            off.holds(&p, &TouchLog::default()),
            "absent counts as not enabled"
        );
    }

    #[test]
    fn touched_counts_the_normalised_span() {
        let p = init();
        let knob: Knob = "master.level".parse().unwrap();
        let spec = knob.spec(&p).unwrap();
        let mut log = TouchLog::default();
        let pred = Predicate::Touched {
            knob: knob.clone(),
            min_span_pct: 10,
        };
        log.note(knob.clone(), spec, spec.from_norm(0.5));
        assert!(!pred.holds(&p, &log));
        log.note(knob, spec, spec.from_norm(0.62));
        assert!(pred.holds(&p, &log));
    }

    #[test]
    fn checks_validate_their_atoms() {
        let sound: Check = serde_json::from_str(
            r#"{"kind":"sound","phrase":1,"expect":["brightness<500Hz"],"macros":[{"macro":"macro1","value":1}]}"#,
        )
        .unwrap();
        assert!(sound.validate("check").is_ok());
        let typo: Check =
            serde_json::from_str(r#"{"kind":"sound","phrase":1,"expect":["brightnes<500Hz"]}"#)
                .unwrap();
        assert_eq!(typo.validate("check").unwrap_err().code(), "bad_field");
        let bad_value: Check = serde_json::from_str(
            r#"{"kind":"patch","predicates":[{"kind":"value","knob":"osc1.wave","cmp":"==","value":"bogus"}]}"#,
        )
        .unwrap();
        assert!(bad_value.validate("check").is_err());
        assert!(
            serde_json::from_str::<Check>(r#"{"kind":"patch","predicates":[],"extra":1}"#).is_err(),
            "unknown fields are refused"
        );
        let nested: Check = serde_json::from_str(
            r#"{"kind":"all","checks":[{"kind":"all","checks":[]},{"kind":"sound","phrase":1,"expect":["brightness<1kHz"]}]}"#,
        )
        .unwrap();
        assert!(nested.validate("check").is_err());
    }
}
