//! Expectations: the parser (`brightness>2kHz, attack<10ms, pitch=C3±20ct, beating in [3Hz, 9Hz]`) and the
//! checker, which returns per item pass/fail, the value, a signed margin in the feature's unit and, given nuisance
//! re-renders, the noise and the `thin` verdict ([`crate::margin`]).
//!
//! Grammar, one item per comma or semicolon (commas inside `[...]` belong to the item):
//!
//! ```text
//! item    := feature op value [tol] | feature "in" "[" value "," value "]" | feature ("==" | "!=") "null"
//! feature := a canonical path (spectrum.centroid_hz) or an alias (brightness), case-insensitive
//! op      := "<" | "<=" | ">" | ">=" | "=" | "==" | "~" | "!=" (booleans only)
//! value   := number with an optional unit (2kHz, 10ms, -6dB, 30%), a note name for pitch (C3, A#4), true/false
//! tol     := ("±" | "+-") (value | number "%")      only after "=", "==" or "~"
//! ```
//!
//! `=` and `~` without a tolerance use the feature's default: 25 ct for `pitch`, 50 ct for `pitch.note` (the same
//! note), 10 % for Hz and ms features, 2 dB for dB features. A pitch compared with `=` or `~` is judged in cents
//! from the target note, so its margin is in cents. The checker echoes every item in a canonical form with the
//! canonical path, e.g. `envelope.attack_ms < 10`.

use crate::descriptors::{AnalysisError, Describer, Descriptors, Feature, NoteTiming, Scale, Want};
use crate::margin::{Interval, Noise, is_thin, margin_pct, noise};
use crate::units::{Unit, cents, nearest_note, note_or_hz, parse_value};
use serde::{Deserialize, Serialize};

/// Default pitch tolerance for `pitch = C3` (cents).
pub const PITCH_TOL_CT: f64 = 25.0;
/// Default tolerance for `pitch.note = C3`: the same note (cents).
pub const NOTE_TOL_CT: f64 = 50.0;

/// What an item tests.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Test {
    /// The compared value must lie in the interval.
    Within(Interval),
    /// The feature must be absent.
    IsNull,
    /// The feature must be present.
    NotNull,
}

/// One parsed expectation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Expect {
    /// The item as written.
    pub source: String,
    /// The item in canonical form, e.g. `envelope.attack_ms < 10`.
    pub canonical: String,
    pub feature: Feature,
    pub test: Test,
    /// For pitch judged in cents: the target frequency (Hz). The compared value is then the cents error.
    pub reference_hz: Option<f64>,
}

/// Why an item could not be parsed. `code` is `bad_expect` (a usage error) or `unknown_feature` (a validation
/// error), as in `docs/agent-cli.md`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExpectError {
    pub code: &'static str,
    pub item: String,
    pub message: String,
    pub did_you_mean: Vec<String>,
    pub hint: String,
}

impl std::fmt::Display for ExpectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {} ({})", self.code, self.message, self.hint)
    }
}

impl std::error::Error for ExpectError {}

const SYNTAX_HINT: &str = "write <feature> <op> <value>, e.g. brightness>2kHz, attack<10ms, sustain>-6dB, pitch=C3±20ct, beating in [3Hz, 9Hz], tremolo == null; ops are < <= > >= = ~ in, and != only for null or true/false";

fn bad(item: &str, why: &str) -> ExpectError {
    ExpectError {
        code: "bad_expect",
        item: item.to_string(),
        message: format!("cannot read expectation '{item}': {why}"),
        did_you_mean: Vec::new(),
        hint: SYNTAX_HINT.to_string(),
    }
}

/// Edit distance (Levenshtein) between two short ASCII strings.
fn edit_distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut cur = vec![i + 1; b.len() + 1];
        for (j, cb) in b.iter().enumerate() {
            let sub = prev[j] + usize::from(ca != *cb);
            cur[j + 1] = sub.min(prev[j + 1] + 1).min(cur[j] + 1);
        }
        prev = cur;
    }
    prev[b.len()]
}

/// Up to three known names close to `name` (edit distance, or a matching last path segment).
fn did_you_mean(name: &str) -> Vec<String> {
    let t = name.trim().to_ascii_lowercase();
    let mut scored: Vec<(usize, &'static str)> = Vec::new();
    for f in Feature::ALL {
        for cand in std::iter::once(f.path()).chain(f.aliases().iter().copied()) {
            let tail = cand.rsplit('.').next().unwrap_or(cand);
            let d = edit_distance(&t, cand).min(edit_distance(&t, tail));
            if d <= 2.max(t.len() / 4) || (t.len() >= 4 && cand.contains(&t)) {
                scored.push((d, f.path()));
            }
        }
    }
    scored.sort_unstable();
    let mut out: Vec<String> = Vec::new();
    for (_, p) in scored {
        if !out.iter().any(|o| o == p) {
            out.push(p.to_string());
        }
        if out.len() == 3 {
            break;
        }
    }
    out
}

/// Split a list on commas and semicolons outside brackets.
fn split_items(list: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let (mut depth, mut start) = (0i32, 0usize);
    for (i, ch) in list.char_indices() {
        match ch {
            '[' | '(' => depth += 1,
            ']' | ')' => depth -= 1,
            ',' | ';' if depth <= 0 => {
                out.push(&list[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(&list[start..]);
    out.into_iter()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect()
}

/// Parse a comma- or semicolon-separated list. Fails on the first bad item.
pub fn parse_list(list: &str) -> Result<Vec<Expect>, ExpectError> {
    let items = split_items(list);
    if items.is_empty() {
        let mut e = bad(list.trim(), "the expectation list is empty");
        e.message = "the expectation list is empty".into();
        return Err(e);
    }
    items.into_iter().map(parse_item).collect()
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Op {
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Approx,
    Ne,
    In,
}

/// Split `head op tail`; the feature name is the leading run of name characters.
fn split_op(item: &str) -> Option<(&str, Op, &str)> {
    let end = item
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '.' || c == '_'))
        .unwrap_or(item.len());
    let (head, rest) = item.split_at(end);
    let rest = rest.trim_start();
    let ops: [(&str, Op); 9] = [
        ("<=", Op::Le),
        (">=", Op::Ge),
        ("==", Op::Eq),
        ("!=", Op::Ne),
        ("<", Op::Lt),
        (">", Op::Gt),
        ("=", Op::Eq),
        ("~", Op::Approx),
        ("in", Op::In),
    ];
    for (sym, op) in ops {
        if let Some(tail) = rest.strip_prefix(sym) {
            // "in" must be a word: "in [" or "in["
            if op == Op::In && !tail.trim_start().starts_with('[') {
                continue;
            }
            return Some((head, op, tail.trim()));
        }
    }
    None
}

/// Format a number for the canonical text (no float noise).
fn num(v: f64) -> String {
    format!("{}", crate::units::clean(v))
}

fn default_tol(feature: Feature, value: f64, note_alias: bool) -> f64 {
    match feature {
        Feature::Pitch if note_alias => NOTE_TOL_CT,
        Feature::Pitch => PITCH_TOL_CT,
        f if f.scale() == Scale::Log => value.abs() * 0.1,
        Feature::Loudness => 1.5,
        Feature::PitchCents => 10.0,
        Feature::ClippedPct => 0.1,
        Feature::Clipping => 0.0,
        f if matches!(f.unit(), Unit::Db | Unit::Dbfs | Unit::Dbtp) => 2.0,
        _ => 0.05,
    }
}

/// Parse one item, such as `brightness>2kHz` or `pitch = C3 ± 20ct`.
pub fn parse_item(item: &str) -> Result<Expect, ExpectError> {
    let item = item.trim();
    let (head, op, tail) = split_op(item).ok_or_else(|| bad(item, "no operator"))?;
    if head.is_empty() {
        return Err(bad(item, "missing feature name"));
    }
    if tail.is_empty() {
        return Err(bad(item, "missing value"));
    }
    let feature = Feature::parse(head).ok_or_else(|| ExpectError {
        code: "unknown_feature",
        item: item.to_string(),
        message: format!("no descriptor named '{head}'"),
        did_you_mean: did_you_mean(head),
        hint: "enth guide --topic descriptors lists the descriptors".to_string(),
    })?;
    let note_alias = matches!(
        head.trim().to_ascii_lowercase().as_str(),
        "pitch.note" | "note"
    );
    let path = feature.path();
    let unit = feature.unit();
    let parse_v = |text: &str| -> Result<f64, ExpectError> {
        if feature == Feature::Pitch {
            note_or_hz(text).ok_or_else(|| {
                bad(
                    item,
                    "pitch needs a note name (C3, A#4) or a frequency (130.8Hz)",
                )
            })
        } else {
            parse_value(text, unit).map_err(|m| bad(item, &m))
        }
    };
    let show = |v: f64| -> String {
        if feature == Feature::Pitch && v > 0.0 {
            let (name, c) = nearest_note(v);
            if c.abs() < 0.01 {
                name
            } else {
                format!("{}Hz", num(v))
            }
        } else if unit == Unit::Bool {
            (if v == 0.0 { "false" } else { "true" }).to_string()
        } else {
            num(v)
        }
    };
    if tail.eq_ignore_ascii_case("null") {
        let test = match op {
            Op::Eq => Test::IsNull,
            Op::Ne => Test::NotNull,
            _ => return Err(bad(item, "null only goes with == or !=")),
        };
        let sym = if test == Test::IsNull { "==" } else { "!=" };
        return Ok(Expect {
            source: item.to_string(),
            canonical: format!("{path} {sym} null"),
            feature,
            test,
            reference_hz: None,
        });
    }
    let (main, tol_text) = match tail.split_once('±').or_else(|| tail.split_once("+-")) {
        Some((a, b)) => (a.trim(), Some(b.trim())),
        None => (tail, None),
    };
    if tol_text.is_some() && !matches!(op, Op::Eq | Op::Approx) {
        return Err(bad(item, "a ± tolerance only goes with =, == or ~"));
    }
    if unit == Unit::Bool && !matches!(op, Op::Eq | Op::Approx | Op::Ne) {
        return Err(bad(
            item,
            "a true/false descriptor only goes with =, == or !=",
        ));
    }
    if op == Op::Ne && unit != Unit::Bool {
        return Err(bad(
            item,
            "!= only goes with null or a true/false descriptor",
        ));
    }
    let mk = |canonical: String, test: Test, reference_hz: Option<f64>| Expect {
        source: item.to_string(),
        canonical,
        feature,
        test,
        reference_hz,
    };
    match op {
        Op::In => {
            let inner = main
                .strip_prefix('[')
                .and_then(|s| s.strip_suffix(']'))
                .ok_or_else(|| bad(item, "write a window as in [low, high]"))?;
            let (a, b) = inner
                .split_once(',')
                .ok_or_else(|| bad(item, "write a window as in [low, high]"))?;
            let (lo, hi) = (parse_v(a.trim())?, parse_v(b.trim())?);
            if lo > hi {
                return Err(bad(item, "the window's low end is above its high end"));
            }
            Ok(mk(
                format!("{path} in [{}, {}]", show(lo), show(hi)),
                Test::Within(Interval::closed(lo, hi)),
                None,
            ))
        }
        Op::Lt | Op::Le | Op::Gt | Op::Ge => {
            let v = parse_v(main)?;
            let (iv, sym) = match op {
                Op::Lt => (Interval::below(v, false), "<"),
                Op::Le => (Interval::below(v, true), "<="),
                Op::Gt => (Interval::above(v, false), ">"),
                _ => (Interval::above(v, true), ">="),
            };
            Ok(mk(
                format!("{path} {sym} {}", show(v)),
                Test::Within(iv),
                None,
            ))
        }
        Op::Ne => {
            let v = parse_v(main)?;
            let other = if v == 0.0 { 1.0 } else { 0.0 };
            Ok(mk(
                format!("{path} = {}", show(other)),
                Test::Within(Interval::closed(other, other)),
                None,
            ))
        }
        Op::Eq | Op::Approx => {
            let v = parse_v(main)?;
            if unit == Unit::Bool {
                return Ok(mk(
                    format!("{path} = {}", show(v)),
                    Test::Within(Interval::closed(v, v)),
                    None,
                ));
            }
            let tol = match tol_text {
                Some(t) if feature == Feature::Pitch => {
                    parse_value(t, Unit::Cents).map_err(|m| bad(item, &m))?
                }
                Some(t) => match t.strip_suffix('%') {
                    Some(p) => {
                        let pct: f64 = p
                            .trim()
                            .parse()
                            .map_err(|_| bad(item, "the tolerance percentage is not a number"))?;
                        v.abs() * pct / 100.0
                    }
                    None => parse_value(t, unit).map_err(|m| bad(item, &m))?,
                },
                None => default_tol(feature, v, note_alias),
            }
            .abs();
            if feature == Feature::Pitch {
                let canon = format!("{path} = {} ± {}ct", show(v), num(tol));
                return Ok(mk(
                    canon,
                    Test::Within(Interval::closed(-tol, tol)),
                    Some(v),
                ));
            }
            let canon = format!("{path} = {} ± {}", show(v), num(tol));
            Ok(mk(
                canon,
                Test::Within(Interval::closed(v - tol, v + tol)),
                None,
            ))
        }
    }
}

impl Expect {
    /// The unit of the compared value and the margin: cents for pitch judged against a note.
    #[must_use]
    pub fn unit(&self) -> Unit {
        if self.reference_hz.is_some() {
            Unit::Cents
        } else {
            self.feature.unit()
        }
    }

    /// Log-scale comparison (Hz and ms features, not pitch in cents).
    #[must_use]
    pub fn log_scale(&self) -> bool {
        self.reference_hz.is_none() && self.feature.scale() == Scale::Log
    }

    /// The accepted interval, `None` for null tests.
    #[must_use]
    pub fn interval(&self) -> Option<Interval> {
        match self.test {
            Test::Within(iv) => Some(iv),
            Test::IsNull | Test::NotNull => None,
        }
    }

    /// The reading in the compared domain: the descriptor itself, or the cents error for pitch against a note.
    #[must_use]
    pub fn compared(&self, d: &Descriptors) -> Option<f64> {
        let got = d.get(self.feature).filter(|g| g.is_finite())?;
        match self.reference_hz {
            Some(r) if got > 0.0 => Some(cents(got, r)),
            Some(_) => None,
            None => Some(got),
        }
    }

    /// How far the reading is outside the accepted interval, 0 when it passes: nepers (ln of the ratio to the
    /// edge) for log-scale features, the feature's unit otherwise; infinite when not measurable. For null tests,
    /// 0 or infinite. The hint solver's objective.
    #[must_use]
    pub fn shortfall(&self, d: &Descriptors) -> f64 {
        let c = self.compared(d);
        match self.test {
            Test::IsNull => {
                if c.is_none() {
                    0.0
                } else {
                    f64::INFINITY
                }
            }
            Test::NotNull => {
                if c.is_some() {
                    0.0
                } else {
                    f64::INFINITY
                }
            }
            Test::Within(iv) => c.map_or(f64::INFINITY, |v| iv.shortfall(v, self.log_scale())),
        }
    }

    /// Judge this item on `main`, with the noise from `nuisance` (the same descriptors on nuisance re-renders).
    #[must_use]
    pub fn evaluate(&self, main: &Descriptors, nuisance: &[Descriptors]) -> ItemResult {
        let got = main.get(self.feature).filter(|g| g.is_finite());
        let c = self.compared(main);
        let lower_bound = main.is_lower_bound(self.feature);
        let others: Vec<Option<f64>> = nuisance.iter().map(|d| self.compared(d)).collect();
        let nz: Option<Noise> = noise(c, &others);
        let estimated = match self.feature {
            Feature::Release => main.envelope.is_some_and(|e| e.release_extrapolated),
            Feature::Attack | Feature::Decay | Feature::Sustain | Feature::Duration => {
                main.envelope.is_some_and(|e| !e.beat_corrected)
            }
            _ => false,
        };
        let (pass, margin, margin_pct_v) = match self.test {
            Test::IsNull => (c.is_none(), None, None),
            Test::NotNull => (c.is_some(), None, None),
            Test::Within(iv) => match c {
                None => (false, None, None),
                Some(v) => {
                    let mut pass = iv.contains(v);
                    if lower_bound {
                        // The true value is at least the reading: only "greater than" can be decided.
                        pass = pass && iv.hi.is_infinite();
                    }
                    let pct = if self.log_scale() {
                        margin_pct(&iv, v)
                    } else {
                        None
                    };
                    (pass, Some(iv.margin(v)), pct)
                }
            },
        };
        let thin = match self.test {
            Test::Within(_) => is_thin(margin, nz),
            Test::IsNull | Test::NotNull => nz.is_some_and(|n| n.unstable),
        };
        ItemResult {
            expect: self.canonical.clone(),
            feature: self.feature,
            got,
            compared: c,
            unit: self.unit(),
            pass,
            margin,
            margin_pct: margin_pct_v,
            noise: nz.filter(|_| c.is_some()).map(|n| n.max_abs_delta),
            thin,
            unstable: nz.is_some_and(|n| n.unstable),
            lower_bound,
            estimated,
        }
    }
}

/// The verdict on one item.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "a flat record of independent verdict flags"
)]
pub struct ItemResult {
    /// The item in canonical form.
    pub expect: String,
    pub feature: Feature,
    /// The reading in the feature's unit (Hz for pitch); `None` when absent or not measured.
    pub got: Option<f64>,
    /// The reading in the compared domain (the cents error for pitch against a note, else `got`).
    pub compared: Option<f64>,
    /// Unit of `compared`, `margin` and `noise`.
    pub unit: Unit,
    pub pass: bool,
    /// Signed distance to the nearest edge, positive inside; `None` for null tests and absent readings.
    pub margin: Option<f64>,
    /// `margin` relative to that edge, in percent, for Hz and ms features.
    pub margin_pct: Option<f64>,
    /// Largest change of the reading over the nuisance re-renders; `None` without them.
    pub noise: Option<f64>,
    /// |margin| ≤ 2 × noise, or the reading is unstable: an inaudible change could flip it.
    pub thin: bool,
    /// Present in some renders and absent in others.
    pub unstable: bool,
    /// The reading is a lower bound (the clip ended first); only `>` items can pass.
    pub lower_bound: bool,
    /// The reading is an estimate: an extrapolated release, or an envelope with an uncorrected slow beat.
    pub estimated: bool,
}

/// The verdict on a whole list.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CheckReport {
    pub pass: bool,
    pub passed: usize,
    pub failed: usize,
    pub thin: usize,
    pub results: Vec<ItemResult>,
}

/// Judge every item on `main`, with noise from the `nuisance` descriptors (may be empty).
#[must_use]
pub fn check(items: &[Expect], main: &Descriptors, nuisance: &[Descriptors]) -> CheckReport {
    let results: Vec<ItemResult> = items.iter().map(|e| e.evaluate(main, nuisance)).collect();
    let passed = results.iter().filter(|r| r.pass).count();
    CheckReport {
        pass: passed == results.len(),
        passed,
        failed: results.len() - passed,
        thin: results.iter().filter(|r| r.thin).count(),
        results,
    }
}

/// Describe `main` and each nuisance re-render (computing only what the items need), then [`check`].
pub fn check_signals(
    describer: &mut Describer,
    items: &[Expect],
    main: &[f32],
    nuisance: &[&[f32]],
    timing: NoteTiming,
) -> Result<CheckReport, AnalysisError> {
    let features: Vec<Feature> = items.iter().map(|e| e.feature).collect();
    let want = Want::for_features(&features);
    let d = describer.describe_some(main, timing, want)?;
    let n: Vec<Descriptors> = nuisance
        .iter()
        .map(|x| describer.describe_some(x, timing, want))
        .collect::<Result<_, _>>()?;
    Ok(check(items, &d, &n))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::beat::Beating;
    use crate::envelope::Envelope;
    use crate::loudness::Level;
    use crate::pitch::Pitch;

    fn desc() -> Descriptors {
        Descriptors {
            dur_s: 3.0,
            silent: false,
            pitch: Some(Pitch {
                f0_hz: 131.5,
                note: "C3".into(),
                cents_dev: 9.0,
                stability: 1.0,
                voiced: 1.0,
            }),
            envelope: Some(Envelope {
                attack_ms: 9.0,
                decay_ms: Some(15.0),
                sustain_db: Some(-4.5),
                release_ms: Some(174.0),
                release_extrapolated: false,
                duration_ms: 2154.0,
                duration_truncated: false,
                ripple_db: 0.7,
                beat_corrected: true,
                beat_periods: Some(12.0),
            }),
            spectrum: Some(crate::descriptors::Spectrum {
                centroid_hz: 3211.0,
                centroid_track_hz: None,
            }),
            noise_ratio: Some(0.01),
            detune: Some(Beating {
                beating_hz: 6.82,
                depth_db: 14.4,
                harmonics_used: 6,
                prominence: 20.0,
            }),
            modulation: crate::descriptors::ModulationDesc::default(),
            loudness: Level {
                lufs_i: Some(-13.9),
                peak_dbfs: -2.9,
                true_peak_dbtp: -2.7,
                clipped_pct: 0.0,
                clipping: false,
            },
        }
    }

    #[test]
    fn parses_the_documented_forms() {
        let l = parse_list(
            "brightness>2kHz, attack<10ms ,sustain<-20dB;pitch=C3±20ct, beating in [3Hz, 9Hz]",
        )
        .unwrap();
        assert_eq!(l.len(), 5);
        let canon: Vec<&str> = l.iter().map(|e| e.canonical.as_str()).collect();
        assert_eq!(
            canon,
            [
                "spectrum.centroid_hz > 2000",
                "envelope.attack_ms < 10",
                "envelope.sustain_db < -20",
                "pitch.f0_hz = C3 ± 20ct",
                "detune.beating_hz in [3, 9]"
            ]
        );
        assert_eq!(
            l[3].reference_hz.map(|r| (r * 100.0).round() / 100.0),
            Some(130.81)
        );
        let e = parse_item("spectrum.centroid_hz >= 1.5 kHz").unwrap();
        assert_eq!(e.test, Test::Within(Interval::above(1500.0, true)));
        assert_eq!(
            parse_item("noisiness < 10%").unwrap().test,
            Test::Within(Interval::below(0.1, false))
        );
        assert_eq!(
            parse_item("brightness = 1500Hz ± 10%").unwrap().test,
            Test::Within(Interval::closed(1350.0, 1650.0))
        );
        assert_eq!(
            parse_item("pitch=A3+-30ct").unwrap().canonical,
            "pitch.f0_hz = A3 ± 30ct"
        );
        assert_eq!(
            parse_item("pitch.note ~ A3").unwrap().canonical,
            "pitch.f0_hz = A3 ± 50ct"
        );
        assert_eq!(
            parse_item("pitch ~ A3").unwrap().canonical,
            "pitch.f0_hz = A3 ± 25ct"
        );
        assert_eq!(
            parse_item("modulation.amp_lfo != null").unwrap().test,
            Test::NotNull
        );
        assert_eq!(
            parse_item("tremolo == null").unwrap().canonical,
            "modulation.amp_lfo.hz == null"
        );
        assert_eq!(
            parse_item("clipping = false").unwrap().canonical,
            "loudness.clipping = false"
        );
        assert_eq!(
            parse_item("clipping != true").unwrap().canonical,
            "loudness.clipping = false"
        );
        assert_eq!(
            parse_item("release > 0.5s").unwrap().canonical,
            "envelope.release_ms > 500"
        );
        assert_eq!(
            parse_item("sustain ~ -6dB").unwrap().test,
            Test::Within(Interval::closed(-8.0, -4.0))
        );
        assert_eq!(
            parse_item("loudness.clipped_pct < 0.5%").unwrap().test,
            Test::Within(Interval::below(0.5, false))
        );
    }

    #[test]
    fn errors_teach() {
        let e = parse_item("brigthness>2kHz").unwrap_err();
        assert_eq!(e.code, "unknown_feature");
        assert_eq!(
            e.did_you_mean.first().map(String::as_str),
            Some("spectrum.centroid_hz")
        );
        let e = parse_item("attack_ms<10").unwrap_err();
        assert!(
            e.did_you_mean.contains(&"envelope.attack_ms".to_string()),
            "{e:?}"
        );
        for b in [
            "attack<10Hz",
            "attack 10ms",
            "attack<",
            "<10ms",
            "attack<10ms±2ms",
            "beating in [9Hz, 3Hz]",
            "beating in 3Hz",
            "attack != 10ms",
            "clipping > 0",
            "attack > null",
            "pitch=Q3",
        ] {
            assert_eq!(parse_item(b).unwrap_err().code, "bad_expect", "{b}");
        }
        assert_eq!(parse_list(" , ").unwrap_err().code, "bad_expect");
    }

    #[test]
    fn checks_with_margins() {
        let d = desc();
        let items = parse_list(
            "attack>8ms, sustain>-4dB, beating in [3Hz, 9Hz], pitch=C3±20ct, tremolo==null",
        )
        .unwrap();
        let r = check(&items, &d, &[]);
        assert_eq!((r.passed, r.failed, r.thin), (4, 1, 0));
        let a = &r.results[0];
        assert!(a.pass && a.margin == Some(1.0) && a.margin_pct == Some(12.5) && a.noise.is_none());
        let s = &r.results[1];
        assert!(!s.pass && s.margin == Some(-0.5) && s.margin_pct.is_none());
        let b = &r.results[2];
        assert!(b.pass && (b.margin.unwrap() - 2.18).abs() < 1e-9);
        let p = &r.results[3];
        assert_eq!(p.unit, Unit::Cents);
        assert!(
            p.pass
                && (p.compared.unwrap() - 9.1).abs() < 0.2
                && (p.margin.unwrap() - 10.9).abs() < 0.2
        );
        assert!(r.results[4].pass);
    }

    #[test]
    fn noise_makes_results_thin() {
        let d = desc();
        let mut n1 = desc();
        n1.envelope.as_mut().unwrap().attack_ms = 8.0;
        let mut n2 = desc();
        n2.envelope.as_mut().unwrap().attack_ms = 10.0;
        n2.detune = None;
        let items = parse_list("attack>8ms, attack>2ms, beating>1Hz").unwrap();
        let r = check(&items, &d, &[n1, n2]);
        assert!(r.results[0].thin && r.results[0].noise == Some(1.0));
        assert!(!r.results[1].thin);
        assert!(r.results[2].unstable && r.results[2].thin);
        assert_eq!(r.thin, 2);
    }

    #[test]
    fn absent_and_lower_bound_readings() {
        let mut d = desc();
        d.detune = None;
        d.envelope.as_mut().unwrap().duration_truncated = true;
        let items = parse_list("beating>2Hz, beating==null, duration>1s, duration<3s").unwrap();
        let r = check(&items, &d, &[]);
        let v: Vec<bool> = r.results.iter().map(|x| x.pass).collect();
        assert_eq!(v, [false, true, true, false]);
        assert!(r.results[0].margin.is_none() && r.results[3].lower_bound);
        assert!(items[0].shortfall(&d).is_infinite());
        assert_eq!(items[2].shortfall(&d), 0.0);
    }

    #[test]
    fn shortfall_is_in_nepers_for_scale_features() {
        let d = desc();
        let e = parse_item("brightness>6422Hz").unwrap();
        assert!((e.shortfall(&d) - libm::log(2.0)).abs() < 1e-3);
        assert!((parse_item("sustain>-2.5dB").unwrap().shortfall(&d) - 2.0).abs() < 1e-12);
    }
}
