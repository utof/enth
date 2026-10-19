//! The patch file: the toy synth's ten parameters in real units, plus the performance (note, length, hold).
//!
//! ```json
//! {"schema":1,"engine":"toy10",
//!  "params":{"amp.attack":5,"amp.decay":300,"amp.release":200,"amp.sustain":-6,"filter.cutoff":2000,
//!            "filter.env_amount":0,"filter.env_decay":200,"filter.resonance":0.7,"osc.detune":0,"osc.wave":"saw"},
//!  "perf":{"dur":2,"hold":1.5,"note":"A3"}}
//! ```
//!
//! Plain numbers in each parameter's unit (see [`CONT`]); the wave is a name. Normalised `0..1` positions (`u`) exist
//! only inside the hint search, never in a file or in output.

use crate::error::{CliError, EXIT_VALIDATION, Res, invalid};
use crate::synth::SAMPLE_RATE;
use crate::units::{Unit, clean, fmt_value, nearest_note, parse_note, parse_value, r4};
use serde_json::{Map, Value, json};
use spike_metric::render::{self, RenderCfg};

/// Number of continuous parameters (everything except the waveform).
pub const N_CONT: usize = 9;

pub const DETUNE: usize = 0;
pub const CUTOFF: usize = 1;
pub const RESONANCE: usize = 2;
pub const ENV_AMOUNT: usize = 3;
pub const ENV_DECAY: usize = 4;
pub const ATTACK: usize = 5;
pub const DECAY: usize = 6;
pub const SUSTAIN: usize = 7;
pub const RELEASE: usize = 8;

/// `amp.sustain` at or below this many dB means "off" (zero level).
pub const SUSTAIN_OFF_DB: f64 = -60.0;

/// A continuous parameter: key, unit, range, default, scale and its one-line perceptual effect.
pub struct Spec {
    pub key: &'static str,
    pub unit: Unit,
    pub min: f64,
    pub max: f64,
    pub default: f64,
    pub log: bool,
    pub effect: &'static str,
}

impl Spec {
    /// Position of `v` on the normalised `0..=1` axis (log or linear), clamped.
    #[must_use]
    pub fn to_u(&self, v: f64) -> f64 {
        let u = if self.log {
            (v / self.min).ln() / (self.max / self.min).ln()
        } else {
            (v - self.min) / (self.max - self.min)
        };
        u.clamp(0.0, 1.0)
    }

    #[must_use]
    pub fn from_u(&self, u: f64) -> f64 {
        let u = u.clamp(0.0, 1.0);
        if self.log {
            self.min * (self.max / self.min).powf(u)
        } else {
            self.min + (self.max - self.min) * u
        }
    }

    /// `"50Hz to 16kHz"`.
    #[must_use]
    pub fn range_text(&self) -> String {
        format!(
            "{} to {}",
            fmt_value(self.min, self.unit),
            fmt_value(self.max, self.unit)
        )
    }
}

pub const CONT: [Spec; N_CONT] = [
    Spec {
        key: "osc.detune",
        unit: Unit::Cents,
        min: 0.0,
        max: 50.0,
        default: 0.0,
        log: false,
        effect: "Pitch spread between the two oscillators. 0 is one clean voice; 5 to 15 ct adds slow chorus-like beating; 25 ct or more sounds wide and unstable",
    },
    Spec {
        key: "filter.cutoff",
        unit: Unit::Hz,
        min: 50.0,
        max: 16_000.0,
        default: 2000.0,
        log: true,
        effect: "Lowpass corner: lower is darker and duller, higher is brighter. The strongest control of brightness",
    },
    Spec {
        key: "filter.resonance",
        unit: Unit::Plain,
        min: 0.5,
        max: 10.0,
        default: 0.7,
        log: true,
        effect: "Emphasis at the cutoff (Q): 0.7 is neutral; higher gives a nasal, whistling peak that brightens and raises the level",
    },
    Spec {
        key: "filter.env_amount",
        unit: Unit::Octaves,
        min: 0.0,
        max: 6.0,
        default: 0.0,
        log: false,
        effect: "How far above the cutoff the filter opens at note start, in octaves; gives the bite of plucks and basses",
    },
    Spec {
        key: "filter.env_decay",
        unit: Unit::Ms,
        min: 5.0,
        max: 2000.0,
        default: 200.0,
        log: true,
        effect: "How fast the extra filter opening fades back to the cutoff; short is a click or pluck, long is a slow sweep",
    },
    Spec {
        key: "amp.attack",
        unit: Unit::Ms,
        min: 1.0,
        max: 1000.0,
        default: 5.0,
        log: true,
        effect: "Fade-in time. Under 10 ms is a click or pluck; over 200 ms is a swell or pad",
    },
    Spec {
        key: "amp.decay",
        unit: Unit::Ms,
        min: 5.0,
        max: 2000.0,
        default: 300.0,
        log: true,
        effect: "How long the level takes to fall from the peak to the sustain level",
    },
    Spec {
        key: "amp.sustain",
        unit: Unit::Db,
        min: SUSTAIN_OFF_DB,
        max: 0.0,
        default: -6.0,
        log: false,
        effect: "Level held while the note is down, relative to the peak. 0 dB is full; -60 dB means off, so the note dies away after the decay (pluck, bell)",
    },
    Spec {
        key: "amp.release",
        unit: Unit::Ms,
        min: 5.0,
        max: 2000.0,
        default: 200.0,
        log: true,
        effect: "Fade-out time after the note ends. Short is clipped and dry; long is a ringing tail",
    },
];

pub const WAVE_KEY: &str = "osc.wave";
pub const WAVE_EFFECT: &str = "Timbre of both oscillators. saw is bright and buzzy (all harmonics); square is hollow (odd harmonics only); triangle is soft and flute-like (few, weak harmonics)";

pub const NOTE_KEY: &str = "perf.note";
pub const DUR_KEY: &str = "perf.dur";
pub const HOLD_KEY: &str = "perf.hold";
pub const NOTE_MIN_HZ: f64 = 50.0;
pub const NOTE_MAX_HZ: f64 = 2000.0;
pub const DUR_MIN_S: f64 = 0.2;
pub const DUR_MAX_S: f64 = 10.0;
pub const HOLD_MIN_S: f64 = 0.05;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wave {
    Saw,
    Square,
    Triangle,
}

impl Wave {
    pub const ALL: [Wave; 3] = [Wave::Saw, Wave::Square, Wave::Triangle];

    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Wave::Saw => "saw",
            Wave::Square => "square",
            Wave::Triangle => "triangle",
        }
    }

    #[must_use]
    pub fn parse(s: &str) -> Option<Wave> {
        Wave::ALL
            .into_iter()
            .find(|w| w.name().eq_ignore_ascii_case(s.trim()))
    }

    fn to_toy(self) -> render::Wave {
        match self {
            Wave::Saw => render::Wave::Saw,
            Wave::Square => render::Wave::Square,
            Wave::Triangle => render::Wave::Triangle,
        }
    }
}

/// How the note is played: pitch, clip length and how long the note is held. Not part of the sound design, but
/// every descriptor depends on it, so it lives in the file and every command reads it from there.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Perf {
    pub note_hz: f64,
    pub dur_s: f64,
    pub hold_s: f64,
}

impl Default for Perf {
    fn default() -> Self {
        Self {
            note_hz: 220.0,
            dur_s: 2.0,
            hold_s: 1.5,
        }
    }
}

/// A parameter address.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Wave,
    Cont(usize),
    Note,
    Dur,
    Hold,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Patch {
    pub wave: Wave,
    pub v: [f64; N_CONT],
    pub perf: Perf,
}

impl Default for Patch {
    fn default() -> Self {
        let mut v = [0.0; N_CONT];
        for (slot, spec) in v.iter_mut().zip(&CONT) {
            *slot = spec.default;
        }
        Self {
            wave: Wave::Saw,
            v,
            perf: Perf::default(),
        }
    }
}

/// One edit, for the `apply` diff.
#[derive(Clone, Debug, PartialEq)]
pub struct Change {
    pub param: &'static str,
    pub from: String,
    pub to: String,
}

/// Every key, in the order `params` lists them.
#[must_use]
pub fn all_keys() -> Vec<&'static str> {
    let mut k = vec![WAVE_KEY];
    k.extend(CONT.iter().map(|s| s.key));
    k.extend([NOTE_KEY, DUR_KEY, HOLD_KEY]);
    k
}

fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut cur = vec![i + 1];
        for (j, cb) in b.iter().enumerate() {
            let sub = prev[j] + usize::from(ca != cb);
            cur.push(sub.min(prev[j + 1] + 1).min(cur[j] + 1));
        }
        prev = cur;
    }
    prev[b.len()]
}

/// Closest known names to `typed` among `names`: exact suffix matches first (`cutoff` finds `filter.cutoff`), then
/// small edit distances.
#[must_use]
pub fn did_you_mean(typed: &str, names: &[&'static str]) -> Vec<&'static str> {
    let t = typed.trim().to_ascii_lowercase();
    let mut scored: Vec<(usize, &'static str)> = names
        .iter()
        .filter_map(|&n| {
            let suffix = n.rsplit('.').next().unwrap_or(n);
            let d = edit_distance(&t, n).min(edit_distance(&t, suffix)).min(
                if n.contains(t.as_str()) && !t.is_empty() {
                    0
                } else {
                    usize::MAX
                },
            );
            (d <= 3.max(t.len() / 3)).then_some((d, n))
        })
        .collect();
    scored.sort_unstable();
    scored.into_iter().map(|(_, n)| n).take(3).collect()
}

/// Resolve a parameter key; unknown keys come back as a teaching error.
pub fn resolve(key: &str) -> Res<Key> {
    let k = key.trim();
    if k == WAVE_KEY {
        return Ok(Key::Wave);
    }
    if let Some(i) = CONT.iter().position(|s| s.key == k) {
        return Ok(Key::Cont(i));
    }
    match k {
        NOTE_KEY => return Ok(Key::Note),
        DUR_KEY => return Ok(Key::Dur),
        HOLD_KEY => return Ok(Key::Hold),
        _ => {}
    }
    Err(
        invalid("unknown_param", format!("no parameter named '{k}'"))
            .with("path", k)
            .with("did_you_mean", did_you_mean(k, &all_keys()))
            .hint("spike-agent params lists every key with its unit and range"),
    )
}

fn range_error(key: &str, shown: &str, lo: &str, hi: &str) -> CliError {
    invalid(
        "out_of_range",
        format!("{key}={shown} is outside {lo} to {hi}"),
    )
    .with("path", key)
    .with("min", lo)
    .with("max", hi)
    .hint(format!("choose a value from {lo} to {hi}"))
}

/// Linear amplitude of the sustain level.
#[must_use]
pub fn sustain_linear(db: f64) -> f64 {
    if db <= SUSTAIN_OFF_DB + 1e-9 {
        0.0
    } else {
        10f64.powf(db / 20.0)
    }
}

impl Patch {
    /// Display text of one parameter, in the form `--set` accepts.
    #[must_use]
    pub fn show(&self, key: Key) -> String {
        match key {
            Key::Wave => self.wave.name().to_string(),
            Key::Cont(i) => fmt_value(self.v[i], CONT[i].unit),
            Key::Note => note_text(self.perf.note_hz),
            Key::Dur => fmt_value(self.perf.dur_s, Unit::Seconds),
            Key::Hold => fmt_value(self.perf.hold_s, Unit::Seconds),
        }
    }

    /// Set one parameter from a user string. Unit conversion and range checks happen here.
    pub fn set(&mut self, key: &str, raw: &str) -> Res<Change> {
        let k = resolve(key)?;
        let from = self.show(k);
        let canon = match k {
            Key::Wave => WAVE_KEY,
            Key::Cont(i) => CONT[i].key,
            Key::Note => NOTE_KEY,
            Key::Dur => DUR_KEY,
            Key::Hold => HOLD_KEY,
        };
        match k {
            Key::Wave => {
                self.wave = Wave::parse(raw).ok_or_else(|| {
                    invalid(
                        "bad_value",
                        format!("'{raw}' is not a waveform; use saw, square or triangle"),
                    )
                    .with("path", canon)
                    .with("allowed", json!(["saw", "square", "triangle"]))
                    .hint("spike-agent params")
                })?;
            }
            Key::Cont(i) => {
                let spec = &CONT[i];
                let v = parse_value(raw, spec.unit).map_err(|m| {
                    invalid("bad_unit", format!("{canon}: {m}"))
                        .with("path", canon)
                        .hint(format!("range is {}", spec.range_text()))
                })?;
                if !(spec.min..=spec.max).contains(&v) {
                    return Err(range_error(
                        canon,
                        raw,
                        &fmt_value(spec.min, spec.unit),
                        &fmt_value(spec.max, spec.unit),
                    ));
                }
                self.v[i] = v;
            }
            Key::Note => {
                let hz = parse_note(raw)
                    .map_or_else(|| parse_value(raw, Unit::Hz), Ok)
                    .map_err(|m| {
                        invalid("bad_unit", format!("{canon}: {m}"))
                            .with("path", canon)
                            .hint(
                                "use a note name such as A3 or C#4, or a frequency such as 130.8Hz",
                            )
                    })?;
                if !(NOTE_MIN_HZ..=NOTE_MAX_HZ).contains(&hz) {
                    return Err(range_error(canon, raw, "50Hz", "2kHz"));
                }
                self.perf.note_hz = hz;
            }
            Key::Dur | Key::Hold => {
                let v = parse_value(raw, Unit::Seconds).map_err(|m| {
                    invalid("bad_unit", format!("{canon}: {m}"))
                        .with("path", canon)
                        .hint("use seconds, e.g. 3s or 2500ms")
                })?;
                let (lo, hi) = if k == Key::Dur {
                    (DUR_MIN_S, DUR_MAX_S)
                } else {
                    (HOLD_MIN_S, DUR_MAX_S)
                };
                if !(lo..=hi).contains(&v) {
                    return Err(range_error(
                        canon,
                        raw,
                        &fmt_value(lo, Unit::Seconds),
                        &fmt_value(hi, Unit::Seconds),
                    ));
                }
                if k == Key::Dur {
                    self.perf.dur_s = v;
                } else {
                    self.perf.hold_s = v;
                }
            }
        }
        Ok(Change {
            param: canon,
            from,
            to: self.show(k),
        })
    }

    /// Whole-patch consistency, checked after a batch of edits.
    pub fn validate(&self) -> Res<()> {
        if self.perf.hold_s > self.perf.dur_s {
            return Err(invalid(
                "bad_perf",
                format!(
                    "perf.hold ({}) is longer than perf.dur ({})",
                    fmt_value(self.perf.hold_s, Unit::Seconds),
                    fmt_value(self.perf.dur_s, Unit::Seconds)
                ),
            )
            .hint("the note must end inside the clip: raise perf.dur or lower perf.hold"));
        }
        Ok(())
    }

    /// Apply `key=value` edits in order (all or nothing).
    pub fn apply_sets(&self, sets: &[String]) -> Res<(Patch, Vec<Change>)> {
        let mut next = self.clone();
        let mut changes = Vec::new();
        for item in sets {
            let item = item.trim();
            if item.is_empty() {
                continue;
            }
            let Some((k, v)) = item.split_once('=') else {
                return Err(
                    crate::error::usage("bad_set", format!("'{item}' is not key=value"))
                        .hint("example: --set filter.cutoff=800Hz,amp.attack=40ms"),
                );
            };
            changes.push(next.set(k, v)?);
        }
        next.validate()?;
        Ok((next, changes))
    }

    /// A copy with continuous parameter `i` at normalised position `u`, rounded to the 4 digits a hint can show.
    #[must_use]
    pub fn with_u(&self, i: usize, u: f64) -> Patch {
        let mut p = self.clone();
        let spec = &CONT[i];
        p.v[i] = r4(spec.from_u(u)).clamp(spec.min, spec.max);
        p
    }

    /// The toy synth's own types.
    #[must_use]
    pub fn to_toy(&self) -> (RenderCfg, render::Patch) {
        let cfg = RenderCfg {
            sample_rate: SAMPLE_RATE,
            duration_s: self.perf.dur_s as f32,
            note_off_s: self.perf.hold_s as f32,
            note_hz: self.perf.note_hz as f32,
        };
        let v = &self.v;
        let toy = render::Patch {
            wave: self.wave.to_toy(),
            detune_cents: v[DETUNE] as f32,
            cutoff_hz: v[CUTOFF] as f32,
            resonance_q: v[RESONANCE] as f32,
            filter_env_octaves: v[ENV_AMOUNT] as f32,
            filter_env_decay_s: (v[ENV_DECAY] / 1000.0) as f32,
            attack_s: (v[ATTACK] / 1000.0) as f32,
            decay_s: (v[DECAY] / 1000.0) as f32,
            sustain: sustain_linear(v[SUSTAIN]) as f32,
            release_s: (v[RELEASE] / 1000.0) as f32,
        };
        (cfg, toy)
    }

    #[must_use]
    pub fn to_json(&self) -> Value {
        let mut params = Map::new();
        params.insert(WAVE_KEY.into(), json!(self.wave.name()));
        for (spec, v) in CONT.iter().zip(&self.v) {
            params.insert(spec.key.into(), json!(*v));
        }
        let mut perf = Map::new();
        perf.insert("note".into(), note_json(self.perf.note_hz));
        perf.insert("dur".into(), json!(self.perf.dur_s));
        perf.insert("hold".into(), json!(self.perf.hold_s));
        json!({"schema": 1, "engine": "toy10", "params": params, "perf": perf})
    }

    /// Parse and validate a patch document. Missing keys take their defaults; unknown keys are errors.
    pub fn from_json(doc: &Value) -> Res<Patch> {
        let bad = |m: String| {
            CliError::new(EXIT_VALIDATION, "bad_patch", m)
                .hint("create a valid file with: spike-agent new --out p.json")
        };
        let obj = doc
            .as_object()
            .ok_or_else(|| bad("the patch file must be a JSON object".into()))?;
        if obj.get("schema") != Some(&json!(1)) {
            return Err(bad("\"schema\" must be 1".into()));
        }
        let mut patch = Patch::default();
        if let Some(params) = obj.get("params") {
            let params = params
                .as_object()
                .ok_or_else(|| bad("\"params\" must be an object".into()))?;
            for (k, v) in params {
                let text = value_text(v)
                    .ok_or_else(|| bad(format!("{k}: expected a number, unit string or name")))?;
                patch.set(k, &text)?;
            }
        }
        if let Some(perf) = obj.get("perf") {
            let perf = perf
                .as_object()
                .ok_or_else(|| bad("\"perf\" must be an object".into()))?;
            for (k, v) in perf {
                let text = value_text(v)
                    .ok_or_else(|| bad(format!("perf.{k}: expected a number or string")))?;
                patch.set(&format!("perf.{k}"), &text)?;
            }
        }
        patch.validate()?;
        Ok(patch)
    }
}

fn value_text(v: &Value) -> Option<String> {
    match v {
        Value::Number(n) => Some(n.to_string()),
        Value::String(s) => Some(s.clone()),
        _ => None,
    }
}

/// `"A3"` when the frequency is an equal-tempered note (within 0.01 ct), otherwise `"220.5Hz"`.
#[must_use]
pub fn note_text(hz: f64) -> String {
    let (name, cents) = nearest_note(hz);
    if cents.abs() < 0.01 {
        name
    } else {
        format!("{}Hz", clean(hz))
    }
}

fn note_json(hz: f64) -> Value {
    let (name, cents) = nearest_note(hz);
    if cents.abs() < 0.01 {
        json!(name)
    } else {
        json!(clean(hz))
    }
}

/// Frequency of a note name or a Hz value, used by the pitch expectation as well.
#[must_use]
pub fn note_or_hz(text: &str) -> Option<f64> {
    parse_note(text).or_else(|| parse_value(text, Unit::Hz).ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_patch_round_trips_through_json() {
        let p = Patch::default();
        let q = Patch::from_json(&p.to_json()).unwrap();
        assert_eq!(p, q);
        assert_eq!(p.to_json()["perf"]["note"], json!("A3"));
    }

    #[test]
    fn set_converts_units_and_checks_ranges() {
        let mut p = Patch::default();
        let c = p.set("filter.cutoff", "1.2kHz").unwrap();
        assert_eq!(p.v[CUTOFF], 1200.0);
        assert_eq!((c.from.as_str(), c.to.as_str()), ("2kHz", "1.2kHz"));
        p.set("amp.attack", "0.5s").unwrap();
        assert_eq!(p.v[ATTACK], 500.0);
        p.set("amp.sustain", "-20dB").unwrap();
        assert_eq!(p.v[SUSTAIN], -20.0);
        p.set("perf.note", "C3").unwrap();
        assert!((p.perf.note_hz - 130.8128).abs() < 1e-3);
        let e = p.set("filter.cutoff", "20kHz").unwrap_err();
        assert_eq!((e.exit, e.code), (3, "out_of_range"));
        let e = p.set("filter.cutoff", "40ms").unwrap_err();
        assert_eq!((e.exit, e.code), (3, "bad_unit"));
    }

    #[test]
    fn unknown_keys_get_suggestions() {
        let e = Patch::default().set("filter.cutof", "1kHz").unwrap_err();
        assert_eq!(e.code, "unknown_param");
        assert_eq!(e.to_json()["did_you_mean"][0], json!("filter.cutoff"));
        let e = Patch::default().set("cutoff", "1kHz").unwrap_err();
        assert_eq!(e.to_json()["did_you_mean"][0], json!("filter.cutoff"));
    }

    #[test]
    fn batch_is_all_or_nothing_and_checks_hold() {
        let p = Patch::default();
        let sets = vec!["perf.hold=3s".to_string()];
        assert_eq!(p.apply_sets(&sets).unwrap_err().code, "bad_perf");
        let sets = vec!["perf.dur=4s".to_string(), "perf.hold=3s".to_string()];
        let (q, changes) = p.apply_sets(&sets).unwrap();
        assert_eq!(changes.len(), 2);
        assert_eq!(q.perf.dur_s, 4.0);
        let bad = vec!["amp.attack=10ms".to_string(), "amp.attak=1ms".to_string()];
        assert!(p.apply_sets(&bad).is_err());
    }

    #[test]
    fn u_axis_maps_both_scales() {
        let c = &CONT[CUTOFF];
        assert!((c.to_u(50.0)).abs() < 1e-12);
        assert!((c.to_u(16_000.0) - 1.0).abs() < 1e-12);
        assert!((c.from_u(c.to_u(800.0)) - 800.0).abs() < 1e-9);
        let s = &CONT[SUSTAIN];
        assert!((s.from_u(0.5) + 30.0).abs() < 1e-12);
    }

    #[test]
    fn sustain_off_is_exactly_zero() {
        assert_eq!(sustain_linear(-60.0), 0.0);
        assert!((sustain_linear(-6.0) - 0.501_187).abs() < 1e-5);
    }
}
