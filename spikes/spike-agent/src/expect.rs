//! Descriptor vocabulary and expectations: `brightness>2kHz, attack<10ms, sustain<-20dB, pitch=C3±20ct`.
//!
//! A [`Feature`] names one descriptor (short alias or canonical path, both accepted). An [`Expect`] compares it with
//! a value in real units. Everything the hint search needs to know about a descriptor (its unit, whether it moves on
//! a log scale, what counts as noise) lives on [`Feature`].

use crate::descriptors::{Descriptors, Want};
use crate::error::{Res, invalid, usage};
use crate::patch::{did_you_mean, note_or_hz};
use crate::units::{Unit, parse_value};
use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Feature {
    Brightness,
    Attack,
    Decay,
    Sustain,
    Release,
    Duration,
    Pitch,
    Noisiness,
    Loudness,
    Peak,
    Beating,
}

pub const FEATURES: [Feature; 11] = [
    Feature::Brightness,
    Feature::Attack,
    Feature::Decay,
    Feature::Sustain,
    Feature::Release,
    Feature::Duration,
    Feature::Pitch,
    Feature::Noisiness,
    Feature::Loudness,
    Feature::Peak,
    Feature::Beating,
];

impl Feature {
    /// Canonical path, as it appears in `describe` output.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Feature::Brightness => "spectrum.centroid_hz",
            Feature::Attack => "envelope.attack_ms",
            Feature::Decay => "envelope.decay_ms",
            Feature::Sustain => "envelope.sustain_db",
            Feature::Release => "envelope.release_ms",
            Feature::Duration => "envelope.duration_ms",
            Feature::Pitch => "pitch.f0_hz",
            Feature::Noisiness => "noise_ratio",
            Feature::Loudness => "loudness.lufs_i",
            Feature::Peak => "loudness.peak_dbfs",
            Feature::Beating => "detune.beating_hz",
        }
    }

    /// The short name used in expectations and hints.
    #[must_use]
    pub fn alias(self) -> &'static str {
        match self {
            Feature::Brightness => "brightness",
            Feature::Attack => "attack",
            Feature::Decay => "decay",
            Feature::Sustain => "sustain",
            Feature::Release => "release",
            Feature::Duration => "duration",
            Feature::Pitch => "pitch",
            Feature::Noisiness => "noisiness",
            Feature::Loudness => "loudness",
            Feature::Peak => "peak",
            Feature::Beating => "beating",
        }
    }

    #[must_use]
    pub fn unit(self) -> Unit {
        match self {
            Feature::Brightness | Feature::Pitch | Feature::Beating => Unit::Hz,
            Feature::Attack | Feature::Decay | Feature::Release | Feature::Duration => Unit::Ms,
            Feature::Sustain | Feature::Peak => Unit::Db,
            Feature::Noisiness => Unit::Ratio,
            Feature::Loudness => Unit::Lufs,
        }
    }

    /// Scale quantities (Hz, ms) move by ratios, so the hint search works on their logarithm.
    #[must_use]
    pub fn log(self) -> bool {
        matches!(
            self,
            Feature::Brightness
                | Feature::Attack
                | Feature::Decay
                | Feature::Release
                | Feature::Duration
                | Feature::Beating
        )
    }

    /// Margin a hint aims past a threshold, in the descriptor's own unit (ratio for log features).
    #[must_use]
    pub fn margin(self) -> f64 {
        match self {
            f if f.log() => 1.1,
            Feature::Sustain | Feature::Peak => 1.5,
            Feature::Loudness => 1.0,
            Feature::Noisiness => 0.03,
            _ => 3.0,
        }
    }

    /// Smallest change (in the hint search's domain) that counts as a real effect of a parameter.
    #[must_use]
    pub fn noise_floor(self) -> f64 {
        match self {
            f if f.log() => 0.01,
            Feature::Noisiness => 0.005,
            Feature::Pitch => 1.0,
            _ => 0.1,
        }
    }

    #[must_use]
    pub fn meaning(self) -> &'static str {
        match self {
            Feature::Brightness => {
                "Average frequency of the sound's energy, weighted by loudness (spectral centroid). Higher is brighter; a pure sine reads its own pitch"
            }
            Feature::Attack => {
                "Time for the level to rise from near-silence (-40 dB) to within 1 dB of its peak. Under 10 ms is a click; over 200 ms is a swell"
            }
            Feature::Decay => {
                "Time for the level to fall from the peak 90% of the way (in amplitude) to the held level; 0 means no decay stage"
            }
            Feature::Sustain => {
                "Level held at the end of the note-on part, in dB below the peak: 0 is as loud as the peak, -80 means silent"
            }
            Feature::Release => {
                "Time after note-off for the level to fall 40 dB; 0 if the sound was already gone. A lower bound if the clip ends first"
            }
            Feature::Duration => {
                "How long the sound stays within 40 dB of its peak, from its onset. A lower bound if still audible at the clip end"
            }
            Feature::Pitch => {
                "Fundamental frequency measured from the audio (median over the loud frames), with the nearest note and offset in cents"
            }
            Feature::Noisiness => {
                "Share of spectral energy not in steady peaks: 0 is a pure tone or clean harmonic sound, 1 is noise"
            }
            Feature::Loudness => {
                "Integrated loudness in LUFS (EBU R128). About -23 is a quiet broadcast level, -14 is loud"
            }
            Feature::Peak => "Highest sample level in dBFS; 0 or above clips",
            Feature::Beating => {
                "Rate in Hz of the slow amplitude wobble of the fundamental (detune beating), 0.5 to 25 Hz; absent when there is none or it cannot be measured"
            }
        }
    }

    /// Descriptor groups that must be computed to know this feature.
    #[must_use]
    pub fn want(self) -> Want {
        match self {
            Feature::Brightness => Want::SPECTRUM,
            Feature::Pitch => Want::PITCH,
            Feature::Noisiness => Want::NOISE,
            Feature::Loudness => Want::LOUDNESS,
            Feature::Beating => Want::BEATING,
            _ => Want::NONE,
        }
    }

    #[must_use]
    pub fn get(self, d: &Descriptors) -> Option<f64> {
        match self {
            Feature::Brightness => d.centroid_hz,
            Feature::Attack => d.attack_ms,
            Feature::Decay => d.decay_ms,
            Feature::Sustain => d.sustain_db,
            Feature::Release => d.release_ms,
            Feature::Duration => d.duration_ms,
            Feature::Pitch => d.f0_hz,
            Feature::Noisiness => d.noise_ratio,
            Feature::Loudness => d.lufs_i,
            Feature::Peak => d.silent.then_some(f64::NEG_INFINITY).or(Some(d.peak_dbfs)),
            Feature::Beating => d.beating_hz,
        }
    }

    /// True if the clip ended before this measurement could finish (the value is a lower bound).
    #[must_use]
    pub fn truncated(self, d: &Descriptors) -> bool {
        match self {
            Feature::Release => d.release_truncated,
            Feature::Duration => d.duration_truncated,
            _ => false,
        }
    }

    #[must_use]
    pub fn parse(s: &str) -> Option<Feature> {
        let t = s.trim().to_ascii_lowercase();
        FEATURES
            .into_iter()
            .find(|f| f.alias() == t || f.name() == t)
    }

    /// Why a measurement can be missing, for the hint text.
    #[must_use]
    pub fn missing_reason(self) -> &'static str {
        match self {
            Feature::Sustain | Feature::Decay => {
                "it needs a note held for at least 80 ms after the attack: raise perf.hold or shorten amp.attack"
            }
            Feature::Beating => {
                "no beating was found: it needs two detuned oscillators (osc.detune of a few cents or more) and at least 0.5 s of steady sound within 30 dB of the peak"
            }
            Feature::Pitch => "no steady pitch was found",
            Feature::Brightness => "the sound is too short or too quiet to analyse",
            Feature::Loudness => "the sound is silent",
            _ => "the sound is silent or too short",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
}

impl Op {
    fn symbol(self) -> &'static str {
        match self {
            Op::Lt => "<",
            Op::Le => "<=",
            Op::Gt => ">",
            Op::Ge => ">=",
            Op::Eq => "=",
        }
    }
}

/// One parsed expectation.
#[derive(Clone, Debug, PartialEq)]
pub struct Expect {
    /// The item as the agent wrote it.
    pub text: String,
    pub feature: Feature,
    pub op: Op,
    /// In the feature's unit (Hz for pitch).
    pub value: f64,
    /// Tolerance for `=`: cents for pitch, the feature's unit otherwise.
    pub tol: f64,
}

/// Result of judging one expectation against measured descriptors.
#[derive(Clone, Debug, PartialEq)]
pub struct Eval {
    pub got: Option<f64>,
    pub pass: bool,
    /// The measurement is a lower bound (the clip ended first).
    pub lower_bound: bool,
    /// For `pitch=`: signed error in cents (measured minus expected).
    pub error_ct: Option<f64>,
}

const DEFAULT_PITCH_TOL_CT: f64 = 25.0;

fn split_ops(item: &str) -> Option<(&str, Op, &str)> {
    let at = item.find(['<', '>', '=', '~'])?;
    let (head, rest) = item.split_at(at);
    let (op, len) = if rest.starts_with("<=") {
        (Op::Le, 2)
    } else if rest.starts_with(">=") {
        (Op::Ge, 2)
    } else if rest.starts_with("==") {
        (Op::Eq, 2)
    } else if rest.starts_with('<') {
        (Op::Lt, 1)
    } else if rest.starts_with('>') {
        (Op::Gt, 1)
    } else {
        (Op::Eq, 1)
    };
    Some((head.trim(), op, rest[len..].trim()))
}

/// Parse one item such as `brightness>2kHz` or `pitch = C3 ± 20ct`.
pub fn parse_item(item: &str) -> Res<Expect> {
    let item = item.trim();
    let bad = |why: &str| {
        usage("bad_expect", format!("cannot read expectation '{item}': {why}")).hint(
            "write <feature><op><value><unit>, e.g. brightness>2kHz, attack<10ms, sustain<-20dB, pitch=C3±20ct; ops are < <= > >= =",
        )
    };
    let (head, op, tail) = split_ops(item).ok_or_else(|| bad("no operator"))?;
    if head.is_empty() || tail.is_empty() {
        return Err(bad("missing feature or value"));
    }
    let feature = Feature::parse(head).ok_or_else(|| {
        let names: Vec<&'static str> = FEATURES.iter().map(|f| f.alias()).collect();
        invalid("unknown_feature", format!("no descriptor named '{head}'"))
            .with("path", head)
            .with("did_you_mean", did_you_mean(head, &names))
            .with("known", json!(names))
            .hint("spike-agent guide lists the descriptors")
    })?;
    let (main, tol_text) = match tail.split_once('±') {
        Some((a, b)) => (a.trim(), Some(b.trim())),
        None => match tail.split_once("+-") {
            Some((a, b)) => (a.trim(), Some(b.trim())),
            None => (tail, None),
        },
    };
    let value = if feature == Feature::Pitch {
        note_or_hz(main)
            .ok_or_else(|| bad("pitch needs a note name (C3, A#4) or a frequency (130.8Hz)"))?
    } else {
        parse_value(main, feature.unit()).map_err(|m| bad(&m))?
    };
    if tol_text.is_some() && op != Op::Eq {
        return Err(bad("a ± tolerance only goes with ="));
    }
    let tol = match tol_text {
        Some(t) if feature == Feature::Pitch => parse_value(t, Unit::Cents).map_err(|m| bad(&m))?,
        Some(t) => match t.strip_suffix('%') {
            Some(p) => {
                let pct: f64 = p
                    .trim()
                    .parse()
                    .map_err(|_| bad("the tolerance percentage is not a number"))?;
                value.abs() * pct / 100.0
            }
            None => parse_value(t, feature.unit()).map_err(|m| bad(&m))?,
        },
        None => match feature {
            Feature::Pitch => DEFAULT_PITCH_TOL_CT,
            f if f.log() => value.abs() * 0.1,
            Feature::Sustain | Feature::Peak => 2.0,
            Feature::Loudness => 1.5,
            _ => 0.05,
        },
    };
    Ok(Expect {
        text: item.to_string(),
        feature,
        op,
        value,
        tol: tol.abs(),
    })
}

/// Parse a comma- (or semicolon-) separated list.
pub fn parse_list(list: &str) -> Res<Vec<Expect>> {
    let items: Vec<&str> = list
        .split([',', ';'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    if items.is_empty() {
        return Err(usage("bad_expect", "the expectation list is empty")
            .hint("example: --expect 'brightness>2kHz, attack<10ms'"));
    }
    items.into_iter().map(parse_item).collect()
}

impl Expect {
    /// True for `pitch=`: judged in cents, not in Hz.
    #[must_use]
    pub fn in_cents(&self) -> bool {
        self.feature == Feature::Pitch && self.op == Op::Eq
    }

    /// Value in the domain the comparison and the hint search use: the descriptor itself, or the signed pitch
    /// error in cents for `pitch=`.
    #[must_use]
    pub fn compared(&self, d: &Descriptors) -> Option<f64> {
        let got = self.feature.get(d).filter(|g| g.is_finite())?;
        if self.in_cents() {
            Some(1200.0 * (got / self.value).log2())
        } else {
            Some(got)
        }
    }

    /// Accepted interval `(low, high)` in the compared domain; open ends are infinite.
    #[must_use]
    pub fn interval(&self) -> (f64, f64) {
        match self.op {
            Op::Lt | Op::Le => (f64::NEG_INFINITY, self.value),
            Op::Gt | Op::Ge => (self.value, f64::INFINITY),
            Op::Eq if self.in_cents() => (-self.tol, self.tol),
            Op::Eq => (self.value - self.tol, self.value + self.tol),
        }
    }

    /// Point a hint should aim for (inside the interval, with a margin past the threshold).
    #[must_use]
    pub fn aim(&self) -> f64 {
        let m = self.feature.margin();
        let log = self.feature.log();
        match self.op {
            Op::Lt | Op::Le => {
                if log {
                    self.value / m
                } else {
                    self.value - m
                }
            }
            Op::Gt | Op::Ge => {
                if log {
                    self.value * m
                } else {
                    self.value + m
                }
            }
            Op::Eq if self.in_cents() => 0.0,
            Op::Eq => self.value,
        }
    }

    fn holds(&self, c: f64) -> bool {
        let (lo, hi) = self.interval();
        match self.op {
            Op::Lt => c < hi,
            Op::Le => c <= hi,
            Op::Gt => c > lo,
            Op::Ge => c >= lo,
            Op::Eq => c >= lo && c <= hi,
        }
    }

    /// How far a reading is outside the accepted interval, 0 when it passes. Log features count in nepers
    /// (`ln` of the ratio to the nearest edge), others in their own unit; infinite when not measurable. Used to
    /// rank near misses (hints, the reachability search).
    #[must_use]
    pub fn shortfall(&self, d: &Descriptors) -> f64 {
        let Some(c) = self.compared(d) else {
            return f64::INFINITY;
        };
        let (lo, hi) = self.interval();
        let edge = if c < lo {
            lo
        } else if c > hi {
            hi
        } else {
            return 0.0;
        };
        if self.feature.log() && !self.in_cents() {
            if c > 0.0 && edge > 0.0 {
                (c / edge).ln().abs()
            } else {
                f64::INFINITY
            }
        } else {
            (c - edge).abs()
        }
    }

    #[must_use]
    pub fn eval(&self, d: &Descriptors) -> Eval {
        let got = self.feature.get(d).filter(|g| g.is_finite());
        let lower_bound = self.feature.truncated(d);
        let error_ct = self.compared(d).filter(|_| self.in_cents());
        let mut pass = self.compared(d).is_some_and(|c| self.holds(c));
        if lower_bound {
            // The true value is at least the reading: only "greater than" can be decided.
            pass = pass && matches!(self.op, Op::Gt | Op::Ge);
        }
        Eval {
            got,
            pass,
            lower_bound,
            error_ct,
        }
    }

    #[must_use]
    pub fn to_json(&self) -> Value {
        json!({"feature": self.feature.name(), "op": self.op.symbol(), "value": self.value})
    }
}

/// Union of the descriptor groups needed by a list of expectations.
#[must_use]
pub fn want_for(expects: &[Expect]) -> Want {
    expects
        .iter()
        .fold(Want::NONE, |w, e| w.or(e.feature.want()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_documented_examples() {
        let l = parse_list("brightness>2kHz, attack<10ms ,sustain<-20dB;pitch=C3±20ct").unwrap();
        assert_eq!(l.len(), 4);
        assert_eq!(
            (l[0].feature, l[0].op, l[0].value),
            (Feature::Brightness, Op::Gt, 2000.0)
        );
        assert_eq!(
            (l[1].feature, l[1].op, l[1].value),
            (Feature::Attack, Op::Lt, 10.0)
        );
        assert_eq!(
            (l[2].feature, l[2].op, l[2].value),
            (Feature::Sustain, Op::Lt, -20.0)
        );
        assert_eq!((l[3].feature, l[3].op), (Feature::Pitch, Op::Eq));
        assert!((l[3].value - 130.81).abs() < 0.01);
        assert_eq!(l[3].tol, 20.0);
    }

    #[test]
    fn accepts_canonical_names_and_spaces() {
        let e = parse_item("spectrum.centroid_hz >= 1.5 kHz").unwrap();
        assert_eq!(
            (e.feature, e.op, e.value),
            (Feature::Brightness, Op::Ge, 1500.0)
        );
        let e = parse_item("noisiness < 10%").unwrap();
        assert!((e.value - 0.1).abs() < 1e-12);
        let e = parse_item("brightness = 1500Hz ± 10%").unwrap();
        assert_eq!(e.tol, 150.0);
    }

    #[test]
    fn errors_teach() {
        let e = parse_item("brigthness>2kHz").unwrap_err();
        assert_eq!((e.exit, e.code), (3, "unknown_feature"));
        assert_eq!(e.to_json()["did_you_mean"][0], json!("brightness"));
        let e = parse_item("attack<10Hz").unwrap_err();
        assert_eq!((e.exit, e.code), (2, "bad_expect"));
        assert_eq!(parse_item("attack 10ms").unwrap_err().exit, 2);
        assert_eq!(parse_list(" , ").unwrap_err().exit, 2);
        assert_eq!(parse_item("attack<10ms±2ms").unwrap_err().exit, 2);
    }

    #[test]
    fn evaluates_and_marks_lower_bounds() {
        let d = Descriptors {
            attack_ms: Some(4.0),
            release_ms: Some(900.0),
            release_truncated: true,
            f0_hz: Some(131.5),
            ..Descriptors::default()
        };
        assert!(parse_item("attack<10ms").unwrap().eval(&d).pass);
        assert!(!parse_item("attack>10ms").unwrap().eval(&d).pass);
        assert!(parse_item("release>500ms").unwrap().eval(&d).pass);
        let short = parse_item("release<2000ms").unwrap().eval(&d);
        assert!(!short.pass && short.lower_bound);
        let p = parse_item("pitch=C3±20ct").unwrap().eval(&d);
        assert!(p.pass);
        assert!((p.error_ct.unwrap() - 9.0).abs() < 1.0, "{p:?}");
        assert!(!parse_item("pitch=C3±5ct").unwrap().eval(&d).pass);
        assert!(!parse_item("brightness>1Hz").unwrap().eval(&d).pass);
    }
}
