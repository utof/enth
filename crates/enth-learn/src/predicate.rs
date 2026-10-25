//! **Owner: lane PLY** (player and checks). Pre-declared by lane S0l (R2). The reference is docs/learn.md, "The
//! predicate grammar (lane PLY)".
//!
//! **The one-line form of a patch predicate.** The persisted shape stays LM's structured object ([`Predicate`] in
//! [`crate::check`], frozen at F1l); this file reads and writes the same value as one line of text, so lessons,
//! `enth lesson`, `lesson-check` and the docs can say `filter1.cutoff < 1kHz`. The grammar follows the expectation
//! grammar of `enth check` (`feature op value`, `feature in [lo, hi]`):
//!
//! ```text
//! predicate := value | within | route | has | enabled | touched
//! value     := knob cmp plain                       filter1.cutoff < 1kHz    osc1.wave == saw    macro1 >= 0.5
//! within    := knob "in" "[" plain "," plain "]"     master.level in [-24dB, 0dB]
//! route     := source "->" param cmp depth            macro1 -> filter1.cutoff > 20%    env2 -> filter1.cutoff >= 1oct
//! has       := "has" kind                             has svf
//! enabled   := ("enabled" | "disabled") module        enabled osc2    disabled noise1
//! touched   := "touched" knob ">=" span               touched filter1.resonance >= 30%
//! knob      := module "." param | "voice." param | "master." param | "macro" 1–8
//! cmp       := "<" | "<=" | ">" | ">=" | "==" | "!="  ("=" reads as "==")
//! plain     := a value as `ParamSpec::parse` reads it for the knob: every CLI unit (800Hz, 1.2kHz, A4, 0.5s, -6dB,
//!              +7st, 50ct, +1oct, 30%, 4:1, 90deg, −12 st), an enum key (saw), on/off; the rest of the line
//! depth     := a route depth as the CLI reads it: percent (20, 20%, -50%), or an interval on the destination
//!              (+1oct, -12st, +200Hz) converted at its current value (`ParamSpec::depth_from_text`)
//! span      := 1–100, percent of the control's normalised travel (30, 30%)
//! ```
//!
//! Spaces around operators are optional (`osc1.pw<30`); `→` reads as `->`. Parsing is **strict**: every atom must
//! parse (addresses and kinds as `enth-patch` reads them, a value that parses for some parameter the knob can name, a
//! depth within ±100 %, a span of 1–100), or the line is refused with a [`PredicateError`]. **Printing is
//! canonical**: single spaces around operators, `==` and `->`, a span with `%`, values as written (trimmed). For every
//! valid predicate, `parse(print(p)) == p` (a proptest).
//!
//! A lesson may write a predicate as this line wherever it writes the object (the authoring TOML:
//! `predicates = ["filter1.cutoff < 1kHz", "touched filter1.resonance >= 30%"]`); it reads into the same value and is
//! always written back as the object, so a built pack's `lesson.json` holds only the frozen shape.
//!
//! **Tolerances** (no exact `==` on real values):
//!
//! | Comparison | On a continuous control | On an enum, integer or toggle |
//! | --- | --- | --- |
//! | `<`, `<=`, `>`, `>=` | exact thresholds on the plain value (`lesson-check` keeps start and witness at least a step away) | exact |
//! | `==` | within one just-noticeable step of the value ([`same_value`]): `sound/1`'s calibration step (10 % for Hz and ms, 2 dB, 10 ct, 0.1 st, 5 points of %, 2 dB on levels, 10 % for ratios and unitless values, 10°), and never narrower than 1 % of the control's normalised travel ([`EQ_FLOOR_NORM`]) | exact |
//! | `!=` | the negation of `==` | exact |
//! | `in [lo, hi]` | both ends included, exactly: the explicit tolerance | exact |
//! | route `==` | within [`ROUTE_STEP`] = 2.5 points of depth, `sound/1`'s route step | — |

use core::fmt;
use std::str::FromStr;

use enth_challenge::calibrate;
use enth_params::ParamSpec;
use enth_patch::{ModSource, ModuleId, ModuleKind, ParamAddr, Patch, RouteAddr};

use crate::check::{Cmp, Knob, Plain, Predicate};
use crate::pack::KindKey;

pub use enth_challenge::calibrate::ROUTE_STEP;

/// The narrowest band `==` accepts on a continuous control, in its normalised travel (1 %): a calibration step that
/// is a ratio has no width at 0 (`macro1 == 0`, a reverb detune of 0), and a knob dragged to a displayed value may sit
/// a hair off it.
pub const EQ_FLOOR_NORM: f32 = 0.01;

/// Why a line is not a predicate. Its code is `bad_predicate`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PredicateError {
    /// The line, trimmed.
    pub text: String,
    /// What is wrong.
    pub message: String,
}

impl PredicateError {
    fn new(text: &str, message: impl Into<String>) -> PredicateError {
        PredicateError {
            text: text.trim().to_owned(),
            message: message.into(),
        }
    }

    /// A stable snake-case code.
    pub fn code(&self) -> &'static str {
        "bad_predicate"
    }

    /// What a predicate looks like.
    pub fn hint(&self) -> String {
        "write <knob> <op> <value> (filter1.cutoff < 1kHz, osc1.wave == saw, macro1 >= 0.5), <knob> in [lo, hi], \
         <source> -> <param> <op> <depth> (macro1 -> filter1.cutoff > 20%), has <kind>, enabled <module>, \
         disabled <module>, or touched <knob> >= <span>%; ops are < <= > >= == !="
            .to_owned()
    }
}

impl fmt::Display for PredicateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "cannot read predicate '{}': {}", self.text, self.message)
    }
}

impl std::error::Error for PredicateError {}

// ---------------------------------------------------------------------------------------------------------------
// Printing
// ---------------------------------------------------------------------------------------------------------------

impl Cmp {
    /// Its symbol: `<`, `<=`, `>`, `>=`, `==`, `!=`.
    pub const fn symbol(self) -> &'static str {
        match self {
            Cmp::Lt => "<",
            Cmp::Le => "<=",
            Cmp::Gt => ">",
            Cmp::Ge => ">=",
            Cmp::Eq => "==",
            Cmp::Ne => "!=",
        }
    }
}

impl fmt::Display for Cmp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.symbol())
    }
}

impl fmt::Display for Predicate {
    /// The canonical one-line form (see the [module docs](self)).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Predicate::Value { knob, cmp, value } => {
                write!(f, "{knob} {cmp} {}", value.as_str().trim())
            }
            Predicate::Within { knob, lo, hi } => {
                write!(
                    f,
                    "{knob} in [{}, {}]",
                    lo.as_str().trim(),
                    hi.as_str().trim()
                )
            }
            Predicate::Route { route, cmp, depth } => {
                write!(f, "{route} {cmp} {}", depth.as_str().trim())
            }
            Predicate::Has { module_kind } => write!(f, "has {}", module_kind.0.key()),
            Predicate::Enabled { module, enabled } => {
                let word = if *enabled { "enabled" } else { "disabled" };
                write!(f, "{word} {module}")
            }
            Predicate::Touched { knob, min_span_pct } => {
                write!(f, "touched {knob} >= {min_span_pct}%")
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------------------------------------------

impl FromStr for Predicate {
    type Err = PredicateError;

    /// Read one line strictly (see the [module docs](self)).
    fn from_str(s: &str) -> Result<Predicate, PredicateError> {
        parse_line(s)
    }
}

/// Read one predicate line strictly: the syntax, then every atom ([`Predicate`]'s own validation, as a lesson's
/// load runs it).
pub fn parse_line(text: &str) -> Result<Predicate, PredicateError> {
    let t = text.trim();
    let err = |m: &str| PredicateError::new(t, m);
    if t.is_empty() {
        return Err(err("the line is empty"));
    }
    if t.contains('\n') {
        return Err(err("a predicate is one line"));
    }
    let p = if let Some(rest) = keyword(t, "has") {
        let key = single_token(rest).ok_or_else(|| err("has takes one module kind (has svf)"))?;
        let kind = ModuleKind::from_key(key).ok_or_else(|| {
            let kinds: Vec<&str> = ModuleKind::listed().map(ModuleKind::key).collect();
            err(&format!(
                "'{key}' is not a module kind ({})",
                kinds.join(", ")
            ))
        })?;
        Predicate::Has {
            module_kind: KindKey(kind),
        }
    } else if let Some((rest, enabled)) = keyword(t, "enabled")
        .map(|r| (r, true))
        .or_else(|| keyword(t, "disabled").map(|r| (r, false)))
    {
        let id = single_token(rest).ok_or_else(|| err("takes one module id (enabled osc2)"))?;
        let module = ModuleId::new(id).map_err(|e| err(&e.to_string()))?;
        Predicate::Enabled { module, enabled }
    } else if let Some(rest) = keyword(t, "touched") {
        let (knob, rest) = knob_token(rest).map_err(|m| err(&m))?;
        let rest = rest.trim_start();
        let span = rest
            .strip_prefix(">=")
            .ok_or_else(|| err("touched takes >= and a span (touched filter1.resonance >= 30%)"))?
            .trim();
        let n = span.strip_suffix('%').unwrap_or(span).trim();
        let min_span_pct = (!n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
            .then(|| n.parse::<u8>().ok())
            .flatten()
            .filter(|v| (1..=100).contains(v))
            .ok_or_else(|| err(&format!("span '{span}' is not a whole 1 to 100 %")))?;
        Predicate::Touched { knob, min_span_pct }
    } else if let Some((src, rest)) = t.split_once("->").or_else(|| t.split_once('→')) {
        let src: ModSource = src
            .trim()
            .parse()
            .map_err(|e: enth_patch::AddrError| err(&e.to_string()))?;
        let (dst_text, rest) = take_addr(rest.trim_start());
        let dst: ParamAddr = dst_text
            .parse()
            .map_err(|e: enth_patch::AddrError| err(&e.to_string()))?;
        let (cmp, depth) = cmp_and_rest(rest).map_err(|m| err(&m))?;
        Predicate::Route {
            route: RouteAddr { src, dst },
            cmp,
            depth: Plain::new(depth),
        }
    } else {
        let (knob, rest) = knob_token(t).map_err(|m| err(&m))?;
        let rest = rest.trim_start();
        if let Some(range) = rest
            .strip_prefix("in")
            .filter(|r| r.starts_with(|c: char| c.is_whitespace() || c == '['))
        {
            let inner = range
                .trim()
                .strip_prefix('[')
                .and_then(|r| r.strip_suffix(']'))
                .ok_or_else(|| err("in takes a range [lo, hi]"))?;
            let (lo, hi) = inner
                .split_once(',')
                .filter(|(lo, hi)| {
                    !lo.trim().is_empty() && !hi.trim().is_empty() && !hi.contains(',')
                })
                .ok_or_else(|| err("a range is [lo, hi], two values"))?;
            Predicate::Within {
                knob,
                lo: Plain::new(lo.trim()),
                hi: Plain::new(hi.trim()),
            }
        } else {
            let (cmp, value) = cmp_and_rest(rest).map_err(|m| err(&m))?;
            Predicate::Value {
                knob,
                cmp,
                value: Plain::new(value),
            }
        }
    };
    p.validate("predicate").map_err(|e| match e {
        crate::LearnError::BadField { message, .. } => err(&message),
        other => err(&other.to_string()),
    })?;
    Ok(p)
}

/// The rest of `t` after `word` and at least one space, when `t` starts with that keyword.
fn keyword<'a>(t: &'a str, word: &str) -> Option<&'a str> {
    let rest = t.strip_prefix(word)?;
    rest.starts_with(char::is_whitespace)
        .then(|| rest.trim_start())
}

/// The one token `rest` holds, if it holds exactly one.
fn single_token(rest: &str) -> Option<&str> {
    let t = rest.trim();
    (!t.is_empty() && !t.contains(char::is_whitespace)).then_some(t)
}

/// The leading address characters of `t` (`[a-z0-9_.]`) and the rest.
fn take_addr(t: &str) -> (&str, &str) {
    let end = t
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '.'))
        .unwrap_or(t.len());
    t.split_at(end)
}

/// A knob at the start of `t`, and the rest.
fn knob_token(t: &str) -> Result<(Knob, &str), String> {
    let (k, rest) = take_addr(t.trim_start());
    if k.is_empty() {
        return Err(
            "a predicate starts with a control (filter1.cutoff, voice.glide, macro1)".to_owned(),
        );
    }
    let knob = k.parse::<Knob>().map_err(|m| {
        if k.contains('.') {
            m
        } else {
            format!("'{k}' is not a control: write <module>.<param> or macro1…macro8")
        }
    })?;
    Ok((knob, rest))
}

/// A comparison at the start of `t` (spaces allowed before it) and the non-empty rest, trimmed.
fn cmp_and_rest(t: &str) -> Result<(Cmp, &str), String> {
    const OPS: [(&str, Cmp); 7] = [
        ("<=", Cmp::Le),
        (">=", Cmp::Ge),
        ("==", Cmp::Eq),
        ("!=", Cmp::Ne),
        ("<", Cmp::Lt),
        (">", Cmp::Gt),
        ("=", Cmp::Eq),
    ];
    let t = t.trim_start();
    let (sym, cmp) = OPS
        .iter()
        .find(|(sym, _)| t.starts_with(sym))
        .ok_or_else(|| {
            let found = t.split_whitespace().next().unwrap_or("nothing");
            format!("expected a comparison (< <= > >= == !=), found '{found}'")
        })?;
    let rest = t[sym.len()..].trim();
    if rest.is_empty() {
        return Err(format!("nothing after '{sym}': a value follows it"));
    }
    Ok((*cmp, rest))
}

// ---------------------------------------------------------------------------------------------------------------
// Reading from a lesson: the object or the line
// ---------------------------------------------------------------------------------------------------------------

/// The frozen object form, derived against [`Predicate`] itself (serde's remote derive refuses to compile if the two
/// drift apart).
#[derive(serde::Deserialize)]
#[serde(
    remote = "Predicate",
    tag = "kind",
    rename_all = "snake_case",
    deny_unknown_fields
)]
#[allow(
    dead_code,
    reason = "a serde remote mirror: only its generated deserialize is used"
)]
enum PredicateObject {
    Value {
        knob: Knob,
        cmp: Cmp,
        value: Plain,
    },
    Within {
        knob: Knob,
        lo: Plain,
        hi: Plain,
    },
    Route {
        route: RouteAddr,
        cmp: Cmp,
        depth: Plain,
    },
    Has {
        module_kind: KindKey,
    },
    Enabled {
        module: ModuleId,
        enabled: bool,
    },
    Touched {
        knob: Knob,
        min_span_pct: u8,
    },
}

impl<'de> serde::Deserialize<'de> for Predicate {
    /// The object (`{"kind":"value",…}`, strict as before) or the one-line form (strict, [`parse_line`]).
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Predicate, D::Error> {
        struct Either;
        impl<'de> serde::de::Visitor<'de> for Either {
            type Value = Predicate;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a predicate: an object with a kind, or one line such as \"filter1.cutoff < 1kHz\"")
            }
            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Predicate, E> {
                parse_line(v).map_err(|e| E::custom(format!("{e} ({})", e.hint())))
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                map: A,
            ) -> Result<Predicate, A::Error> {
                PredicateObject::deserialize(serde::de::value::MapAccessDeserializer::new(map))
            }
        }
        d.deserialize_any(Either)
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Tolerances and depths
// ---------------------------------------------------------------------------------------------------------------

/// The band `==` accepts around `want` on a control with `spec`: exactly `want` for an enum, integer or toggle;
/// otherwise one `sound/1` calibration step either side ([`calibrate::step`]), widened to at least
/// [`EQ_FLOOR_NORM`] of the normalised travel either side, within the range.
pub fn equal_band(spec: &ParamSpec, want: f32) -> (f32, f32) {
    let Some(step) = calibrate::step(spec).filter(|_| !spec.is_discrete()) else {
        return (want, want);
    };
    let (a, b) = (step.apply(want, false), step.apply(want, true));
    let n = spec.to_norm(want);
    let lo = a
        .min(b)
        .min(spec.from_norm((n - EQ_FLOOR_NORM).max(0.0)))
        .max(spec.min());
    let hi = a
        .max(b)
        .max(spec.from_norm((n + EQ_FLOOR_NORM).min(1.0)))
        .min(spec.max());
    (lo, hi)
}

/// Whether `have` is the same value as `want` on a control with `spec`, by [`equal_band`].
pub fn same_value(spec: &ParamSpec, have: f32, want: f32) -> bool {
    if spec.is_discrete() {
        return have == want;
    }
    let (lo, hi) = equal_band(spec, want);
    lo <= have && have <= hi
}

/// `have cmp want` on a control with `spec`: thresholds exactly, `==` and `!=` by [`same_value`].
pub fn compare_value(spec: &ParamSpec, cmp: Cmp, have: f32, want: f32) -> bool {
    match cmp {
        Cmp::Eq => same_value(spec, have, want),
        Cmp::Ne => !same_value(spec, have, want),
        ordered => ordered.holds(have, want),
    }
}

/// `have cmp want` on route depths (percent): thresholds exactly, `==` and `!=` within [`ROUTE_STEP`].
pub fn compare_depth(cmp: Cmp, have: f32, want: f32) -> bool {
    let same = (have - want).abs() <= ROUTE_STEP;
    match cmp {
        Cmp::Eq => same,
        Cmp::Ne => !same,
        ordered => ordered.holds(have, want),
    }
}

/// A route depth from text against `patch`: percent as written (`20`, `20%`), or an interval on the destination
/// (`+1oct`, `-12st`, `+200Hz`) converted at its current value, as the CLI reads route depths
/// (`ParamSpec::depth_from_text`). `None` when it does not read (an interval on a destination the patch lacks).
pub fn resolve_depth(patch: &Patch, dst: &ParamAddr, text: &Plain) -> Option<f32> {
    if let Some(d) = text.depth() {
        return Some(d);
    }
    let spec = patch.spec(dst)?;
    let base = patch.value(dst)?;
    spec.depth_from_text(base, text.as_str()).ok()
}

/// Whether a depth text reads for some destination this address could name: percent, or an interval valid on a
/// parameter with that key at its default value (the module's kind is known only against a patch).
pub(crate) fn depth_parses_somewhere(dst: &ParamAddr, text: &Plain) -> bool {
    if text.depth().is_some() {
        return true;
    }
    match dst.target() {
        enth_patch::Target::Module(_) => enth_patch::CATALOGUE
            .iter()
            .filter_map(|m| m.kind.param(dst.param()))
            .any(|spec| spec.depth_from_text(spec.default, text.as_str()).is_ok()),
        enth_patch::Target::Voice | enth_patch::Target::Master => {
            let init = Patch::init();
            init.spec(dst)
                .is_some_and(|spec| spec.depth_from_text(spec.default, text.as_str()).is_ok())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::check::TouchLog;

    fn p(s: &str) -> Predicate {
        s.parse().unwrap_or_else(|e: PredicateError| panic!("{e}"))
    }

    #[test]
    fn every_form_parses_and_prints_canonically() {
        for (line, canonical) in [
            ("filter1.cutoff < 1kHz", "filter1.cutoff < 1kHz"),
            ("osc1.pw<30", "osc1.pw < 30"),
            ("osc1.wave = saw", "osc1.wave == saw"),
            ("macro1>=0.5", "macro1 >= 0.5"),
            (
                "master.level in [-24dB,0dB]",
                "master.level in [-24dB, 0dB]",
            ),
            ("master.level in[ -24dB , 0 ]", "master.level in [-24dB, 0]"),
            (
                "macro1 → filter1.cutoff > 20%",
                "macro1 -> filter1.cutoff > 20%",
            ),
            (
                "env2->filter1.cutoff>=+1oct",
                "env2 -> filter1.cutoff >= +1oct",
            ),
            ("has  svf", "has svf"),
            ("enabled osc2", "enabled osc2"),
            ("disabled noise1", "disabled noise1"),
            (
                "touched filter1.resonance >= 30",
                "touched filter1.resonance >= 30%",
            ),
            ("voice.glide != 0ms", "voice.glide != 0ms"),
            ("osc1.semi == −12 st", "osc1.semi == −12 st"),
        ] {
            let pred = p(line);
            assert_eq!(pred.to_string(), canonical, "{line}");
            assert_eq!(p(canonical), pred, "{canonical}");
        }
    }

    #[test]
    fn it_reads_the_same_value_as_the_object() {
        let json = r#"{"kind":"route","route":"macro1 -> filter1.cutoff","cmp":">","depth":"20%"}"#;
        let object: Predicate = serde_json::from_str(json).unwrap();
        assert_eq!(p("macro1 -> filter1.cutoff > 20%"), object);
        let line: Predicate = serde_json::from_str(r#""macro1 -> filter1.cutoff > 20%""#).unwrap();
        assert_eq!(line, object);
        assert_eq!(
            serde_json::to_string(&line).unwrap(),
            json,
            "always written as the object"
        );
    }

    #[test]
    fn a_check_mixes_lines_and_objects() {
        let check: crate::check::Check = serde_json::from_str(
            r#"{"kind":"patch","predicates":["filter1.cutoff < 1kHz",{"kind":"has","module_kind":"svf"},"touched filter1.resonance >= 30%"]}"#,
        )
        .unwrap();
        assert!(check.validate("check").is_ok());
        let crate::check::Check::Patch { predicates } = &check else {
            panic!("a patch check")
        };
        assert_eq!(predicates[0], p("filter1.cutoff < 1kHz"));
        assert_eq!(predicates[1], p("has svf"));
        let toml_check: crate::check::Check = toml::from_str(
            "kind = \"patch\"\npredicates = [\"filter1.cutoff < 1kHz\", \"has svf\", \"touched filter1.resonance >= 30%\"]\n",
        )
        .unwrap();
        assert_eq!(toml_check, check);
        let bad = serde_json::from_str::<crate::check::Check>(
            r#"{"kind":"patch","predicates":["filter1.cutoff < bogus"]}"#,
        )
        .unwrap_err();
        assert!(bad.to_string().contains("bogus"), "{bad}");
    }

    #[test]
    fn bad_lines_are_refused_with_a_reason() {
        for line in [
            "",
            "filter1.cutoff",
            "filter1.cutoff < ",
            "filter1.cutoff ~ 1kHz",
            "filter1.cutoff < bogus",
            "filter1.cutof < 1kHz",
            "filter1 < 1kHz",
            "macro9 >= 0.5",
            "osc1.wave == triangle_x",
            "master.level in [-24dB]",
            "master.level in -24dB, 0dB",
            "macro1 -> filter1.cutoff > 200%",
            "macro1 -> filter1.cutoff > 1s",
            "has svf osc",
            "has filter",
            "enabled",
            "enabled Osc2",
            "touched filter1.resonance >= 0",
            "touched filter1.resonance >= 101%",
            "touched filter1.resonance > 30%",
            "touched filter1.resonance >= 2.5%",
            "a\nb",
        ] {
            let e = line.parse::<Predicate>().unwrap_err();
            assert_eq!(e.code(), "bad_predicate");
            assert!(!e.message.is_empty() && !e.hint().is_empty(), "{line}");
        }
    }

    #[test]
    fn equality_on_a_continuous_control_is_one_step_wide() {
        let mut patch = Patch::init();
        let set = |patch: &mut Patch, addr: &str, v: f32| {
            enth_patch::apply(
                patch,
                enth_patch::Op::SetParam {
                    addr: addr.parse().unwrap(),
                    value: v,
                },
            )
            .unwrap();
        };
        let none = TouchLog::default();
        // master.level is dB: a step is 2 dB.
        set(&mut patch, "master.level", -5.0);
        assert!(p("master.level == -6dB").holds(&patch, &none));
        assert!(p("master.level == -4dB").holds(&patch, &none));
        assert!(!p("master.level == -8dB").holds(&patch, &none));
        assert!(p("master.level != -8dB").holds(&patch, &none));
        // osc1.semi: 0.1 st, never narrower than 1 % of 96 st.
        set(&mut patch, "osc1.semi", 12.5);
        assert!(p("osc1.semi == 12st").holds(&patch, &none));
        assert!(!p("osc1.semi == 11st").holds(&patch, &none));
        // A macro at 0: the ratio step has no width there, the 1 % floor does.
        patch.macros[0].value = 0.005;
        assert!(p("macro1 == 0").holds(&patch, &none));
        patch.macros[0].value = 0.05;
        assert!(!p("macro1 == 0").holds(&patch, &none));
        // Thresholds are exact.
        set(&mut patch, "master.level", -6.0);
        assert!(p("master.level <= -6dB").holds(&patch, &none));
        assert!(!p("master.level < -6dB").holds(&patch, &none));
        // Enums are exact.
        assert!(p("osc1.wave == saw").holds(&patch, &none));
        assert!(!p("osc1.wave == pulse").holds(&patch, &none));
    }

    #[test]
    fn route_depths_read_percent_or_an_interval() {
        let mut patch = Patch::init();
        let route: RouteAddr = "macro1 -> osc1.semi".parse().unwrap();
        enth_patch::apply(
            &mut patch,
            enth_patch::Op::InsertRoute {
                index: 0,
                route: enth_patch::Route {
                    src: route.src,
                    dst: route.dst,
                    depth: 12.5,
                },
            },
        )
        .unwrap();
        let none = TouchLog::default();
        // 12 st on a 96 st linear range is 12.5 %.
        assert!(p("macro1 -> osc1.semi >= +12st").holds(&patch, &none));
        assert!(p("macro1 -> osc1.semi == 12.5%").holds(&patch, &none));
        assert!(
            p("macro1 -> osc1.semi == 14%").holds(&patch, &none),
            "2.5 points"
        );
        assert!(!p("macro1 -> osc1.semi == 16%").holds(&patch, &none));
        assert!(
            !p("lfo1 -> osc1.semi > 0").holds(&patch, &none),
            "absent: depth 0"
        );
        assert!(p("lfo1 -> osc1.semi == 0").holds(&patch, &none));
    }

    #[test]
    fn bands_stay_in_range() {
        for m in enth_patch::CATALOGUE {
            for spec in m.kind.params() {
                for n in [0.0f32, 0.3, 1.0] {
                    let want = spec.from_norm(n);
                    let (lo, hi) = equal_band(spec, want);
                    assert!(lo <= want && want <= hi, "{}", spec.key);
                    assert!(spec.min() <= lo && hi <= spec.max(), "{}", spec.key);
                    assert!(same_value(spec, want, want));
                }
            }
        }
    }
}
