//! The note and parameter script for `render`: a small text format, so goldens can live in a file.
//!
//! Entries are separated by `,`, `;` or a newline; `#` starts a comment that runs to the end of the
//! line. Times are in seconds from the start of the render.
//!
//! ```text
//! C4@0+1.5:0.8          note C4 (MIDI 60) from 0 s, held 1.5 s, velocity 0.8 (velocity is optional, default 0.8)
//! 64@0.5+1              MIDI key 64 from 0.5 s, held 1 s
//! Slot 1=0.2@0.5        set the parameter named "Slot 1" to the plain value 0.2 at 0.5 s (a numeric id works too)
//! reset@0.03            call the plugin's `reset` at the start of the block that contains 0.03 s (it drops
//!                       every sounding voice, like a host that restarts its plugins mid-render)
//! ```
//!
//! Every note gets its own note id (its index in the script), so note-offs find their note the way a
//! host that tracks voices would. Events that fall on the same sample run in this order: resets, parameter
//! changes, note-offs, note-ons, then script order. Events at or after the end of the render are dropped.

use std::fmt::Write as _;

pub const DEFAULT_VELOCITY: f64 = 0.8;

#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    /// `clap_plugin.reset()`: not an event; the runner applies it between blocks.
    Reset,
    Param {
        id: u32,
        value: f64,
    },
    NoteOff {
        key: u16,
        note_id: u32,
    },
    NoteOn {
        key: u16,
        note_id: u32,
        velocity: f64,
    },
}

impl Kind {
    /// Sort rank for events on the same sample.
    pub fn order(&self) -> u8 {
        match self {
            Kind::Reset => 0,
            Kind::Param { .. } => 1,
            Kind::NoteOff { .. } => 2,
            Kind::NoteOn { .. } => 3,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Scheduled {
    /// Absolute sample index from the start of the render.
    pub frame: u64,
    pub kind: Kind,
}

/// A script resolved to sample positions and sorted.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Schedule {
    pub events: Vec<Scheduled>,
}

impl Schedule {
    /// Events with `start <= frame < end`, as a slice (the schedule is sorted by frame).
    pub fn window(&self, start: u64, end: u64) -> &[Scheduled] {
        let lo = self.events.partition_point(|e| e.frame < start);
        let hi = self.events.partition_point(|e| e.frame < end);
        &self.events[lo..hi]
    }

    pub fn note_count(&self) -> usize {
        self.events
            .iter()
            .filter(|e| matches!(e.kind, Kind::NoteOn { .. }))
            .count()
    }
}

/// Resolves a parameter name or numeric id to a CLAP param id.
pub type ParamResolver<'a> = &'a dyn Fn(&str) -> Option<u32>;

/// Parses a MIDI key: `0..=127`, or a name such as `C4`, `F#3`, `Bb2` (C4 = 60).
pub fn parse_key(text: &str) -> Result<u16, String> {
    let t = text.trim();
    if let Ok(n) = t.parse::<u16>() {
        return if n <= 127 {
            Ok(n)
        } else {
            Err(format!("key {n} is out of range 0..=127"))
        };
    }
    let mut chars = t.chars();
    let letter = chars.next().ok_or("empty key")?;
    let base: i32 = match letter.to_ascii_uppercase() {
        'C' => 0,
        'D' => 2,
        'E' => 4,
        'F' => 5,
        'G' => 7,
        'A' => 9,
        'B' => 11,
        _ => return Err(format!("{t:?} is neither a MIDI number nor a note name")),
    };
    let rest = chars.as_str();
    let (accidental, octave_text) = match rest.chars().next() {
        Some('#') => (1, &rest[1..]),
        Some('b') => (-1, &rest[1..]),
        _ => (0, rest),
    };
    let octave: i32 = octave_text
        .parse()
        .map_err(|_| format!("{t:?} has no octave number"))?;
    let key = (octave + 1) * 12 + base + accidental;
    u16::try_from(key)
        .ok()
        .filter(|k| *k <= 127)
        .ok_or_else(|| format!("{t:?} is out of the MIDI range"))
}

pub fn seconds_to_frame(seconds: f64, sample_rate: f64) -> Result<u64, String> {
    if !(seconds.is_finite() && seconds >= 0.0) {
        return Err(format!("time {seconds} must be a finite number >= 0"));
    }
    Ok((seconds * sample_rate).round() as u64)
}

fn parse_f64(text: &str, what: &str) -> Result<f64, String> {
    text.trim()
        .parse::<f64>()
        .map_err(|_| format!("{what}: {text:?} is not a number"))
}

fn parse_entry(
    entry: &str,
    sample_rate: f64,
    note_id: &mut u32,
    resolve: ParamResolver<'_>,
    out: &mut Vec<Scheduled>,
) -> Result<(), String> {
    if let Some((name, rest)) = entry.split_once('=') {
        let (value, at) = rest
            .split_once('@')
            .ok_or_else(|| format!("{entry:?}: a parameter entry is `name=value@seconds`"))?;
        let id = resolve(name.trim()).ok_or_else(|| {
            format!(
                "{entry:?}: no parameter named or numbered {:?}",
                name.trim()
            )
        })?;
        out.push(Scheduled {
            frame: seconds_to_frame(parse_f64(at, entry)?, sample_rate)?,
            kind: Kind::Param {
                id,
                value: parse_f64(value, entry)?,
            },
        });
        return Ok(());
    }
    if let Some(at) = entry.strip_prefix("reset@") {
        out.push(Scheduled {
            frame: seconds_to_frame(parse_f64(at, entry)?, sample_rate)?,
            kind: Kind::Reset,
        });
        return Ok(());
    }
    let (key, rest) = entry
        .split_once('@')
        .ok_or_else(|| format!("{entry:?}: a note entry is `key@start+duration[:velocity]`"))?;
    let (timing, velocity) = match rest.split_once(':') {
        Some((t, v)) => (t, parse_f64(v, entry)?),
        None => (rest, DEFAULT_VELOCITY),
    };
    if !(0.0..=1.0).contains(&velocity) {
        return Err(format!("{entry:?}: velocity {velocity} is outside 0..=1"));
    }
    let (start, duration) = timing
        .split_once('+')
        .ok_or_else(|| format!("{entry:?}: a note entry is `key@start+duration[:velocity]`"))?;
    let key = parse_key(key).map_err(|e| format!("{entry:?}: {e}"))?;
    let start = parse_f64(start, entry)?;
    let duration = parse_f64(duration, entry)?;
    if duration <= 0.0 {
        return Err(format!("{entry:?}: duration must be > 0"));
    }
    let id = *note_id;
    *note_id += 1;
    out.push(Scheduled {
        frame: seconds_to_frame(start, sample_rate)?,
        kind: Kind::NoteOn {
            key,
            note_id: id,
            velocity,
        },
    });
    out.push(Scheduled {
        frame: seconds_to_frame(start + duration, sample_rate)?,
        kind: Kind::NoteOff { key, note_id: id },
    });
    Ok(())
}

/// Parses a script into a sorted schedule. `total_frames` drops events that would fall outside the render.
pub fn parse(
    script: &str,
    sample_rate: f64,
    total_frames: u64,
    resolve: ParamResolver<'_>,
) -> Result<Schedule, String> {
    let mut events = Vec::new();
    let mut note_id = 0;
    for line in script.lines() {
        let line = line.split('#').next().unwrap_or("");
        for entry in line.split([',', ';']) {
            let entry = entry.trim();
            if !entry.is_empty() {
                parse_entry(entry, sample_rate, &mut note_id, resolve, &mut events)?;
            }
        }
    }
    events.retain(|e| e.frame < total_frames);
    // `sort_by_key` is stable, so equal keys keep script order.
    events.sort_by_key(|e| (e.frame, e.kind.order()));
    Ok(Schedule { events })
}

/// Reads `--notes`: a path to a script file when one exists with that name, else the script itself.
pub fn load_script(arg: &str) -> Result<String, String> {
    let path = std::path::Path::new(arg);
    if path.is_file() {
        std::fs::read_to_string(path).map_err(|e| format!("{arg}: {e}"))
    } else {
        Ok(arg.to_owned())
    }
}

/// A one-line description of a schedule, for the JSON report.
pub fn describe(schedule: &Schedule) -> String {
    let mut s = String::new();
    for e in &schedule.events {
        let _ = match &e.kind {
            Kind::NoteOn { key, velocity, .. } => write!(s, "{}:on({key},{velocity}) ", e.frame),
            Kind::NoteOff { key, .. } => write!(s, "{}:off({key}) ", e.frame),
            Kind::Param { id, value } => write!(s, "{}:param({id}={value}) ", e.frame),
            Kind::Reset => write!(s, "{}:reset ", e.frame),
        };
    }
    s.trim_end().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_params(name: &str) -> Option<u32> {
        name.parse().ok()
    }

    #[test]
    fn keys_parse_as_numbers_and_names() {
        assert_eq!(parse_key("60"), Ok(60));
        assert_eq!(parse_key("C4"), Ok(60));
        assert_eq!(parse_key("A4"), Ok(69));
        assert_eq!(parse_key("F#3"), Ok(54));
        assert_eq!(parse_key("Bb2"), Ok(46));
        assert_eq!(parse_key("C-1"), Ok(0));
        assert_eq!(parse_key("G9"), Ok(127));
        assert!(parse_key("128").is_err());
        assert!(parse_key("G#9").is_err());
        assert!(parse_key("H4").is_err());
        assert!(parse_key("C").is_err());
    }

    #[test]
    fn a_chord_becomes_sorted_samples() {
        let s = parse("C4@0+1, E4@0.5+1:0.5", 48_000.0, 96_000, &no_params).unwrap();
        let frames: Vec<u64> = s.events.iter().map(|e| e.frame).collect();
        assert_eq!(frames, [0, 24_000, 48_000, 72_000]);
        assert_eq!(s.note_count(), 2);
        assert_eq!(
            s.events[1].kind,
            Kind::NoteOn {
                key: 64,
                note_id: 1,
                velocity: 0.5
            }
        );
    }

    #[test]
    fn same_sample_order_is_param_then_off_then_on() {
        let s = parse("60@0+1; 62@1+1; 7=0.5@1", 1000.0, 5000, &no_params).unwrap();
        let at_1000: Vec<&Kind> = s
            .events
            .iter()
            .filter(|e| e.frame == 1000)
            .map(|e| &e.kind)
            .collect();
        assert!(matches!(at_1000[0], Kind::Param { id: 7, .. }));
        assert!(matches!(at_1000[1], Kind::NoteOff { key: 60, .. }));
        assert!(matches!(at_1000[2], Kind::NoteOn { key: 62, .. }));
    }

    #[test]
    fn reset_is_scheduled_first() {
        let s = parse("60@0+1; reset@0", 1000.0, 5000, &no_params).unwrap();
        assert_eq!(s.events[0].kind, Kind::Reset);
        assert!(matches!(s.events[1].kind, Kind::NoteOn { .. }));
    }

    #[test]
    fn events_past_the_end_are_dropped_and_comments_ignored() {
        let s = parse("60@0+10 # held past the end\n", 1000.0, 2000, &no_params).unwrap();
        assert_eq!(s.events.len(), 1);
        assert!(matches!(s.events[0].kind, Kind::NoteOn { .. }));
    }

    #[test]
    fn bad_scripts_name_the_entry() {
        for bad in [
            "60", "60@1", "60@1+0", "60@1+1:2", "x=1@0", "p=1", "60@-1+1",
        ] {
            let err = parse(bad, 1000.0, 5000, &no_params).unwrap_err();
            assert!(!err.is_empty(), "{bad}");
        }
    }

    #[test]
    fn window_selects_by_frame() {
        let s = parse("60@0+1; 62@2+1", 10.0, 100, &no_params).unwrap();
        assert_eq!(s.window(0, 10).len(), 1);
        assert_eq!(s.window(10, 20).len(), 1); // the first note-off, at 10
        assert_eq!(s.window(20, 30).len(), 1);
        assert_eq!(s.window(0, 100).len(), 4);
    }
}
