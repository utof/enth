//! Performances: what is played when a patch is rendered or judged.
//!
//! Patch files (schema 1) carry no performance, so every command that renders takes one from its flags, or uses
//! the default ([`Perf::default_perf`]: C3 at velocity 100, held 1.5 s, in a 3 s clip). A WAV that `enth render`
//! wrote carries its performance in the `enth` chunk.
//!
//! `--perf` takes, in this order: `-` (stdin), inline JSON (`enth.perf/1`, `schema` optional), a file holding that
//! JSON, or the short form `note=A3,vel=100,hold=1.5s,dur=2.5s`. `--note`, `--vel`, `--hold` and `--dur` change one
//! field of a single-note performance (the default, or the one `--perf` gave).

use std::path::Path;

use enth_analysis::NoteTiming;
use enth_render::{Note as RenderNote, Performance};

use crate::error::{self, CliError};
use crate::schema::{NoteOut, NoteRef, PerfFile, PerfNote, PerfOut, ids};

/// The default note, C3 (MIDI 48).
pub const DEFAULT_NOTE: u8 = 48;
/// The default MIDI velocity.
pub const DEFAULT_VEL: u8 = 100;
/// The default hold, in seconds.
pub const DEFAULT_HOLD_S: f64 = 1.5;
/// The default clip length, in seconds.
pub const DEFAULT_DUR_S: f64 = 3.0;
/// Tail added after the hold when only the hold is given, in seconds.
pub const DEFAULT_TAIL_S: f64 = 1.5;
/// The longest clip a command renders, in seconds.
pub const MAX_DUR_S: f64 = 120.0;
/// The shortest clip, in seconds (the analysis needs 20 ms).
pub const MIN_DUR_S: f64 = 0.05;

const NOTE_NAMES: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];

const PERF_HINT: &str = "--perf takes a file, inline JSON like {\"notes\":[{\"note\":\"A3\",\"vel\":100,\"on_s\":0,\"off_s\":1.5}],\"dur_s\":2.5}, or note=A3,vel=100,hold=1.5s,dur=2.5s; see `enth schema perf`";

/// Where a performance came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// `--perf` or the single-field flags.
    Flag,
    /// The CLI's default.
    Default,
    /// The `enth` chunk of a WAV.
    Wav,
    /// A note-off inferred from the audio.
    Inferred,
    /// Unknown.
    None,
}

impl Source {
    /// The name printed in `perf.source`.
    pub const fn name(self) -> &'static str {
        match self {
            Source::Flag => "flag",
            Source::Default => "default",
            Source::Wav => "wav",
            Source::Inferred => "inferred",
            Source::None => "none",
        }
    }
}

/// One note. Key and velocity are unknown for an inferred performance.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Note {
    /// MIDI key.
    pub key: Option<u8>,
    /// MIDI velocity, 1-127.
    pub vel: Option<u8>,
    /// Note-on, seconds.
    pub on_s: f64,
    /// Note-off, seconds; unknown for a WAV without its performance.
    pub off_s: Option<f64>,
}

/// A performance and where it came from.
#[derive(Clone, Debug, PartialEq)]
pub struct Perf {
    /// The notes.
    pub notes: Vec<Note>,
    /// Clip length, seconds.
    pub dur_s: f64,
    /// Where it came from.
    pub source: Source,
}

/// The performance flags of `render`, `describe`, `check`, `step` and `diff --describe`.
#[derive(Clone, Debug, Default, PartialEq, Eq, clap::Args)]
pub struct PerfArgs {
    /// The performance: a file, inline JSON (`enth schema perf`), `-` for stdin, or `note=A3,vel=100,hold=1.5s,dur=2.5s`.
    #[arg(long, value_name = "PERF")]
    pub perf: Option<String>,
    /// The note played (`C3`, `A#4`, or a MIDI number). Default C3.
    #[arg(long, value_name = "NOTE")]
    pub note: Option<String>,
    /// MIDI velocity 1-127. Default 100.
    #[arg(long, value_name = "VEL")]
    pub vel: Option<String>,
    /// How long the note is held (`1.5s`, `800ms`, or seconds). Default 1.5 s.
    #[arg(long, value_name = "TIME")]
    pub hold: Option<String>,
    /// Length of the clip (`3s`, or seconds); it must cover the release. Default 3 s, or hold + 1.5 s.
    #[arg(long, value_name = "TIME")]
    pub dur: Option<String>,
}

impl PerfArgs {
    /// No performance flag was given.
    pub fn is_empty(&self) -> bool {
        self.perf.is_none()
            && self.note.is_none()
            && self.vel.is_none()
            && self.hold.is_none()
            && self.dur.is_none()
    }
}

/// The name of a MIDI key (`48` is `C3`).
pub fn note_name(key: u8) -> String {
    let k = usize::from(key);
    format!("{}{}", NOTE_NAMES[k % 12], k as i32 / 12 - 1)
}

fn bad(message: impl Into<String>) -> CliError {
    error::usage("bad_perf", message, PERF_HINT)
}

fn parse_key(text: &str) -> Result<u8, CliError> {
    let t = text.trim();
    if let Ok(n) = t.parse::<u8>() {
        return if n <= 127 {
            Ok(n)
        } else {
            Err(bad(format!("note {n} is not a MIDI key (0-127)")))
        };
    }
    enth_params::units::note_number(t)
        .and_then(|m| u8::try_from(m).ok())
        .filter(|m| *m <= 127)
        .ok_or_else(|| {
            bad(format!(
                "'{t}' is not a note name (C3, A#4, Db2) or a MIDI key"
            ))
        })
}

fn parse_vel(text: &str) -> Result<u8, CliError> {
    let t = text.trim();
    t.parse::<u8>()
        .ok()
        .filter(|v| (1..=127).contains(v))
        .ok_or_else(|| {
            bad(format!(
                "velocity '{t}' is not a whole number from 1 to 127"
            ))
        })
}

/// Seconds from `1.5s`, `800ms` or a bare number of seconds.
fn parse_time(text: &str) -> Result<f64, CliError> {
    let t = text.trim();
    let lower = t.to_ascii_lowercase();
    let (num, scale) = if let Some(n) = lower.strip_suffix("ms") {
        (n, 0.001)
    } else if let Some(n) = lower.strip_suffix('s') {
        (n, 1.0)
    } else {
        (lower.as_str(), 1.0)
    };
    num.trim()
        .parse::<f64>()
        .ok()
        .filter(|x| x.is_finite() && *x >= 0.0)
        .map(|x| x * scale)
        .ok_or_else(|| bad(format!("'{t}' is not a time (1.5s, 800ms, or seconds)")))
}

impl Perf {
    /// The default performance: C3, velocity 100, held 1.5 s, in a 3 s clip.
    pub fn default_perf() -> Self {
        Perf {
            notes: vec![Note {
                key: Some(DEFAULT_NOTE),
                vel: Some(DEFAULT_VEL),
                on_s: 0.0,
                off_s: Some(DEFAULT_HOLD_S),
            }],
            dur_s: DEFAULT_DUR_S,
            source: Source::Default,
        }
    }

    /// An unknown performance for a clip of `dur_s` (decay, sustain and release cannot be read).
    pub fn unknown(dur_s: f64) -> Self {
        Perf {
            notes: Vec::new(),
            dur_s,
            source: Source::None,
        }
    }

    /// A performance inferred from audio: one note from 0, released at `off_s`.
    pub fn inferred(off_s: f64, dur_s: f64) -> Self {
        Perf {
            notes: vec![Note {
                key: None,
                vel: None,
                on_s: 0.0,
                off_s: Some(off_s),
            }],
            dur_s,
            source: Source::Inferred,
        }
    }

    /// The performance the flags describe, or `None` when no flag was given. `stdin` serves `--perf -`.
    pub fn from_args(args: &PerfArgs, stdin: &str) -> Result<Option<Perf>, CliError> {
        if args.is_empty() {
            return Ok(None);
        }
        let mut perf = match &args.perf {
            Some(text) => {
                let mut p = parse_perf_arg(text, stdin)?;
                p.source = Source::Flag;
                p
            }
            None => Perf::default_perf(),
        };
        let from_default = args.perf.is_none();
        let single = args.note.is_some() || args.vel.is_some() || args.hold.is_some();
        if single && perf.notes.len() != 1 {
            return Err(bad(
                "--note, --vel and --hold change a single-note performance; this one has several notes",
            ));
        }
        if let Some(n) = &args.note {
            perf.notes[0].key = Some(parse_key(n)?);
        }
        if let Some(v) = &args.vel {
            perf.notes[0].vel = Some(parse_vel(v)?);
        }
        if let Some(h) = &args.hold {
            let hold = parse_time(h)?;
            let on = perf.notes[0].on_s;
            perf.notes[0].off_s = Some(on + hold);
            if from_default && args.dur.is_none() {
                perf.dur_s = on + hold + DEFAULT_TAIL_S;
            }
        }
        if let Some(d) = &args.dur {
            perf.dur_s = parse_time(d)?;
        }
        perf.source = Source::Flag;
        perf.validate()?;
        Ok(Some(perf))
    }

    /// A performance from its JSON form.
    pub fn from_file(f: &PerfFile, source: Source) -> Result<Perf, CliError> {
        if let Some(s) = &f.schema
            && s != ids::PERF
        {
            return Err(bad(format!("schema '{s}' is not {}", ids::PERF)));
        }
        let notes = f
            .notes
            .iter()
            .map(|n| {
                let key = match &n.note {
                    NoteRef::Key(k) => parse_key(&k.to_string())?,
                    NoteRef::Name(s) => parse_key(s)?,
                };
                Ok(Note {
                    key: Some(key),
                    vel: Some(parse_vel(&n.vel.to_string())?),
                    on_s: n.on_s,
                    off_s: Some(n.off_s),
                })
            })
            .collect::<Result<Vec<_>, CliError>>()?;
        let p = Perf {
            notes,
            dur_s: f.dur_s,
            source,
        };
        p.validate()?;
        Ok(p)
    }

    fn validate(&self) -> Result<(), CliError> {
        if self.notes.is_empty() {
            return Err(bad("a performance needs at least one note"));
        }
        if !(self.dur_s.is_finite() && (MIN_DUR_S..=MAX_DUR_S).contains(&self.dur_s)) {
            return Err(bad(format!(
                "dur {} s is outside {MIN_DUR_S}..={MAX_DUR_S} s",
                self.dur_s
            )));
        }
        for (i, n) in self.notes.iter().enumerate() {
            let off = n.off_s.unwrap_or(n.on_s);
            if !(n.on_s.is_finite() && off.is_finite() && n.on_s >= 0.0 && off >= n.on_s) {
                return Err(bad(format!(
                    "note {i}: on {} s, off {off} s; times are finite and 0 <= on <= off",
                    n.on_s
                )));
            }
        }
        Ok(())
    }

    /// The JSON form (`enth.perf/1`). Only for complete performances (flag, default, wav).
    pub fn to_file(&self) -> PerfFile {
        PerfFile {
            schema: Some(ids::PERF.to_owned()),
            notes: self
                .notes
                .iter()
                .map(|n| PerfNote {
                    note: NoteRef::Name(note_name(n.key.unwrap_or(DEFAULT_NOTE))),
                    vel: n.vel.unwrap_or(DEFAULT_VEL),
                    on_s: n.on_s,
                    off_s: n.off_s.unwrap_or(self.dur_s),
                })
                .collect(),
            dur_s: self.dur_s,
        }
    }

    /// What commands print.
    pub fn to_out(&self) -> PerfOut {
        PerfOut {
            source: self.source.name().to_owned(),
            dur_s: self.dur_s,
            notes: self
                .notes
                .iter()
                .map(|n| NoteOut {
                    note: n.key.map(note_name),
                    vel: n.vel,
                    on_s: n.on_s,
                    off_s: n.off_s,
                })
                .collect(),
            why_missing: (self.source == Source::None).then(|| {
                "the WAV has no enth chunk and no note-off was found in the audio: pass --perf (or render with enth)"
                    .to_owned()
            }),
        }
    }

    /// The render performance. Only for complete performances.
    pub fn to_render(&self) -> Performance {
        Performance {
            notes: self
                .notes
                .iter()
                .map(|n| {
                    RenderNote::new(
                        n.key.unwrap_or(DEFAULT_NOTE),
                        f32::from(n.vel.unwrap_or(DEFAULT_VEL)) / 127.0,
                        n.on_s,
                        n.off_s.unwrap_or(self.dur_s),
                    )
                })
                .collect(),
            automation: Vec::new(),
            duration_s: self.dur_s,
        }
    }

    /// The note timing the analysis reads the envelope against: the first note (the earliest note-on, ties in
    /// order) and its note-off, `None` when that is unknown. It is the rule of [`enth_solver::note_timing`], which
    /// `measure` applies to a patch, so a WAV that `render` wrote is analysed like the patch it came from.
    pub fn timing(&self) -> NoteTiming {
        let first = self
            .notes
            .iter()
            .enumerate()
            .min_by(|(i, a), (j, b)| a.on_s.total_cmp(&b.on_s).then(i.cmp(j)))
            .map(|(_, n)| n);
        match first {
            Some(n) => NoteTiming {
                on_s: n.on_s,
                off_s: n.off_s,
            },
            None => NoteTiming::unknown_off(),
        }
    }
}

/// Parse the value of `--perf`.
fn parse_perf_arg(text: &str, stdin: &str) -> Result<Perf, CliError> {
    let t = text.trim();
    if t == "-" {
        return parse_json(stdin, "stdin");
    }
    if t.starts_with('{') {
        return parse_json(t, "--perf");
    }
    let path = Path::new(t);
    if t.contains('=') && !path.exists() {
        return parse_short(t);
    }
    let bytes = crate::patchio::read_bytes(path)?;
    parse_json(&String::from_utf8_lossy(&bytes), t)
}

fn parse_json(text: &str, from: &str) -> Result<Perf, CliError> {
    let f: PerfFile = serde_json::from_str(text.trim())
        .map_err(|e| bad(format!("{from} is not an enth.perf/1 object: {e}")))?;
    Perf::from_file(&f, Source::Flag)
}

/// `note=A3,vel=100,hold=1.5s,dur=2.5s` (any subset; the rest from the default).
fn parse_short(text: &str) -> Result<Perf, CliError> {
    let mut p = Perf::default_perf();
    let mut dur_given = false;
    let mut hold = None;
    for pair in text.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        let (k, v) = pair
            .split_once('=')
            .ok_or_else(|| bad(format!("'{pair}' is not key=value")))?;
        match k.trim().to_ascii_lowercase().as_str() {
            "note" => p.notes[0].key = Some(parse_key(v)?),
            "vel" | "velocity" => p.notes[0].vel = Some(parse_vel(v)?),
            "hold" => hold = Some(parse_time(v)?),
            "dur" | "duration" => {
                p.dur_s = parse_time(v)?;
                dur_given = true;
            }
            other => {
                return Err(bad(format!(
                    "unknown performance key '{other}'; the keys are note, vel, hold, dur"
                )));
            }
        }
    }
    if let Some(h) = hold {
        p.notes[0].off_s = Some(h);
        if !dur_given {
            p.dur_s = h + DEFAULT_TAIL_S;
        }
    }
    p.source = Source::Flag;
    p.validate()?;
    Ok(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(perf: Option<&str>, note: Option<&str>, hold: Option<&str>) -> PerfArgs {
        PerfArgs {
            perf: perf.map(str::to_owned),
            note: note.map(str::to_owned),
            hold: hold.map(str::to_owned),
            ..PerfArgs::default()
        }
    }

    #[test]
    fn names_and_keys() {
        assert_eq!(note_name(48), "C3");
        assert_eq!(note_name(69), "A4");
        assert_eq!(note_name(0), "C-1");
        assert_eq!(parse_key("A#4").unwrap(), 70);
        assert_eq!(parse_key("60").unwrap(), 60);
        assert_eq!(parse_key("H2").unwrap_err().code(), "bad_perf");
    }

    #[test]
    fn the_three_forms_agree() {
        let short = Perf::from_args(
            &args(Some("note=A3,vel=90,hold=1.5s,dur=2s"), None, None),
            "",
        )
        .unwrap()
        .unwrap();
        let json = r#"{"notes":[{"note":"A3","vel":90,"on_s":0,"off_s":1.5}],"dur_s":2}"#;
        let inline = Perf::from_args(&args(Some(json), None, None), "")
            .unwrap()
            .unwrap();
        let piped = Perf::from_args(&args(Some("-"), None, None), json)
            .unwrap()
            .unwrap();
        assert_eq!(short, inline);
        assert_eq!(inline, piped);
        assert_eq!(short.notes[0].key, Some(57));
        assert_eq!(short.source, Source::Flag);
    }

    #[test]
    fn hold_alone_extends_the_default_clip() {
        let p = Perf::from_args(&args(None, Some("E2"), Some("3s")), "")
            .unwrap()
            .unwrap();
        assert_eq!(p.notes[0].off_s, Some(3.0));
        assert_eq!(p.dur_s, 4.5);
        assert_eq!(p.timing(), NoteTiming::held(3.0));
    }

    #[test]
    fn timing_is_the_solvers_rule_for_the_first_note() {
        let note = |key, on_s, off_s| Note {
            key: Some(key),
            vel: Some(100),
            on_s,
            off_s,
        };
        let p = Perf {
            notes: vec![
                note(60, 0.5, Some(1.0)),
                note(62, 0.25, Some(2.0)),
                note(64, 0.25, Some(3.0)),
            ],
            dur_s: 3.0,
            source: Source::Flag,
        };
        // The earliest note-on (ties in order) and its own note-off, not the last one.
        assert_eq!(
            p.timing(),
            NoteTiming {
                on_s: 0.25,
                off_s: Some(2.0)
            }
        );
        assert_eq!(p.timing(), enth_solver::note_timing(&p.to_render()));
        assert_eq!(Perf::unknown(2.0).timing(), NoteTiming::unknown_off());
    }

    #[test]
    fn bad_values_are_usage_errors() {
        for bad in [
            "note=Z9",
            "hold=-1s",
            "dur=1000s",
            "colour=red",
            "{\"notes\":[]}",
        ] {
            let e = Perf::from_args(&args(Some(bad), None, None), "").unwrap_err();
            assert_eq!((e.code(), e.exit()), ("bad_perf", 2), "{bad}");
        }
    }

    #[test]
    fn round_trips_through_its_json() {
        let p = Perf::default_perf();
        let back = Perf::from_file(&p.to_file(), Source::Default).unwrap();
        assert_eq!(back, p);
        assert_eq!(p.to_render().notes[0].key, DEFAULT_NOTE);
    }
}
