//! **Owner: lane MUS** (Music, freeze point F1e). Pre-declared by lane S0e (R4/R5, freeze point F0e). The reference is
//! docs/ear.md, "Where R4 lives" and "The ear generator".
//!
//! The tables the games draw from, all integers (a key is a MIDI note number, C4 = 60, a size is in semitones):
//!
//! | Table | What | Answer keys are |
//! | --- | --- | --- |
//! | [`INTERVALS`] | unison to octave, with the names the games print (`P5`, `m3`, `TT`) | the size in semitones (the index) |
//! | [`CHORDS`] | triads (major, minor, diminished, augmented, sus2, sus4), sevenths; [`chord_keys`] builds inversions | the index |
//! | [`SCALES`] | major, the minors, the modes, pentatonics, whole tone, chromatic | the index |
//! | [`PROGRESSIONS`] | four-chord loops in Roman numerals; [`voice_progression`] voices them with small moves | the index |
//! | [`NAMES_SHARP`], [`NAMES_FLAT`] | pitch-class names; [`note_name`] adds the octave | the pitch class |
//!
//! **Append-only.** A game's answer keys are indices into these tables and ear generator v1 is frozen by a golden, so a
//! row is never edited or reordered: a new interval, chord or progression goes at the end (`ear-check` and the golden
//! tests pin the tables). A generated phrase stays within C1 to C7 ([`KEY_MIN`] to [`KEY_MAX`], MIDI 24 to 96), with at
//! most 8 notes at once and at most 10 s (docs/ear.md); [`crate::ear::Question::validate`] checks it.
//!
//! No floats anywhere: pitch height is a key, and the rendering crate turns it into hertz through the pinned `libm`.

use std::fmt;

/// The lowest key a generated question may use: C1 (MIDI 24, about 32.7 Hz).
pub const KEY_MIN: u8 = 24;
/// The highest key a generated question may use: C7 (MIDI 96, about 2093 Hz).
pub const KEY_MAX: u8 = 96;
/// Middle C. Octave numbers follow scientific pitch notation: C4 = 60, so C1 = 24 and C7 = 96.
pub const MIDDLE_C: u8 = 60;

// ---------------------------------------------------------------------------------------------------------------
// Note names
// ---------------------------------------------------------------------------------------------------------------

/// Pitch-class names with sharps, index = pitch class (C = 0).
pub const NAMES_SHARP: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];
/// Pitch-class names with flats, index = pitch class (C = 0).
pub const NAMES_FLAT: [&str; 12] = [
    "C", "Db", "D", "Eb", "E", "F", "Gb", "G", "Ab", "A", "Bb", "B",
];
/// Ascii slugs of the pitch classes, for skill keys (`ear.pitch.cs`).
pub const PITCH_SLUGS: [&str; 12] = [
    "c", "cs", "d", "ds", "e", "f", "fs", "g", "gs", "a", "as", "b",
];

/// The pitch class of `key`, 0 (C) to 11 (B).
#[must_use]
pub const fn pitch_class(key: u8) -> u8 {
    key % 12
}

/// The octave number of `key` in scientific pitch notation (60 is C4, 24 is C1, 0 is C-1).
#[must_use]
pub const fn octave(key: u8) -> i8 {
    (key / 12) as i8 - 1
}

/// The name of `key` with its octave, sharps: `note_name(61)` is `"C#4"`.
#[must_use]
pub fn note_name(key: u8) -> String {
    format!(
        "{}{}",
        NAMES_SHARP[usize::from(pitch_class(key))],
        octave(key)
    )
}

/// The name of `key` with its octave, flats: `note_name_flat(61)` is `"Db4"`.
#[must_use]
pub fn note_name_flat(key: u8) -> String {
    format!(
        "{}{}",
        NAMES_FLAT[usize::from(pitch_class(key))],
        octave(key)
    )
}

// ---------------------------------------------------------------------------------------------------------------
// Intervals
// ---------------------------------------------------------------------------------------------------------------

/// One interval, indexed by its size in semitones.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Interval {
    /// The size in semitones, 0 to 12; equal to its index in [`INTERVALS`].
    pub semitones: u8,
    /// The short name the games print: `P5`, `m3`, `TT`.
    pub short: &'static str,
    /// The long name: `perfect fifth`.
    pub name: &'static str,
    /// The ascii slug of a skill key: `ear.interval.<slug>` (`p5`, `m3`, `M3`, `tt`).
    pub skill: &'static str,
}

const fn iv(
    semitones: u8,
    short: &'static str,
    name: &'static str,
    skill: &'static str,
) -> Interval {
    Interval {
        semitones,
        short,
        name,
        skill,
    }
}

/// The intervals within an octave, index = semitones. Append-only (nothing can follow the octave here; a compound
/// interval would be a new table).
pub const INTERVALS: [Interval; 13] = [
    iv(0, "P1", "perfect unison", "p1"),
    iv(1, "m2", "minor second", "m2"),
    iv(2, "M2", "major second", "M2"),
    iv(3, "m3", "minor third", "m3"),
    iv(4, "M3", "major third", "M3"),
    iv(5, "P4", "perfect fourth", "p4"),
    iv(6, "TT", "tritone", "tt"),
    iv(7, "P5", "perfect fifth", "p5"),
    iv(8, "m6", "minor sixth", "m6"),
    iv(9, "M6", "major sixth", "M6"),
    iv(10, "m7", "minor seventh", "m7"),
    iv(11, "M7", "major seventh", "M7"),
    iv(12, "P8", "perfect octave", "p8"),
];

/// The interval of `semitones`, or `None` above an octave.
#[must_use]
pub fn interval(semitones: u8) -> Option<&'static Interval> {
    INTERVALS.get(usize::from(semitones))
}

// ---------------------------------------------------------------------------------------------------------------
// Chords
// ---------------------------------------------------------------------------------------------------------------

/// One chord quality: the semitone steps above the root, in root position.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChordQuality {
    /// The long name: `major`, `half-diminished seventh`.
    pub name: &'static str,
    /// The chord symbol suffix: `""` for major, `m`, `dim`, `aug`, `sus2`, `7`, `maj7`, `m7`, `m7b5`, `dim7`.
    pub symbol: &'static str,
    /// The ascii slug of a skill key: `ear.chord.<slug>` (`maj`, `min`, `dim`, `aug`, `sus2`, `sus4`, `dom7`...).
    pub skill: &'static str,
    /// The steps above the root in root position, ascending, starting with 0.
    pub steps: &'static [u8],
}

/// Major triad: index into [`CHORDS`].
pub const MAJOR: u8 = 0;
/// Minor triad.
pub const MINOR: u8 = 1;
/// Diminished triad.
pub const DIMINISHED: u8 = 2;
/// Augmented triad.
pub const AUGMENTED: u8 = 3;
/// Suspended second.
pub const SUS2: u8 = 4;
/// Suspended fourth.
pub const SUS4: u8 = 5;
/// Dominant seventh.
pub const DOMINANT7: u8 = 6;
/// Major seventh.
pub const MAJOR7: u8 = 7;
/// Minor seventh.
pub const MINOR7: u8 = 8;
/// Half-diminished seventh (minor seventh flat five).
pub const HALF_DIMINISHED7: u8 = 9;
/// Diminished seventh.
pub const DIMINISHED7: u8 = 10;

const fn cq(
    name: &'static str,
    symbol: &'static str,
    skill: &'static str,
    steps: &'static [u8],
) -> ChordQuality {
    ChordQuality {
        name,
        symbol,
        skill,
        steps,
    }
}

/// The chord qualities, index = the constants above. Append-only.
pub const CHORDS: [ChordQuality; 11] = [
    cq("major", "", "maj", &[0, 4, 7]),
    cq("minor", "m", "min", &[0, 3, 7]),
    cq("diminished", "dim", "dim", &[0, 3, 6]),
    cq("augmented", "aug", "aug", &[0, 4, 8]),
    cq("suspended second", "sus2", "sus2", &[0, 2, 7]),
    cq("suspended fourth", "sus4", "sus4", &[0, 5, 7]),
    cq("dominant seventh", "7", "dom7", &[0, 4, 7, 10]),
    cq("major seventh", "maj7", "maj7", &[0, 4, 7, 11]),
    cq("minor seventh", "m7", "min7", &[0, 3, 7, 10]),
    cq("half-diminished seventh", "m7b5", "hdim7", &[0, 3, 6, 10]),
    cq("diminished seventh", "dim7", "dim7", &[0, 3, 6, 9]),
];

/// The quality with index `quality`, or `None` past the table.
#[must_use]
pub fn chord(quality: u8) -> Option<&'static ChordQuality> {
    CHORDS.get(usize::from(quality))
}

/// The keys of the chord on `root` with the given quality and inversion, ascending. The `n`-th inversion moves the
/// lowest `n` notes up an octave (0 is root position, 1 first inversion). `None` for an unknown quality, an inversion
/// past the chord's size, or a key above 127.
#[must_use]
pub fn chord_keys(root: u8, quality: u8, inversion: u8) -> Option<Vec<u8>> {
    let q = chord(quality)?;
    if usize::from(inversion) >= q.steps.len() {
        return None;
    }
    let mut keys = Vec::with_capacity(q.steps.len());
    for (i, step) in q.steps.iter().enumerate() {
        let mut key = u16::from(root) + u16::from(*step);
        if i < usize::from(inversion) {
            key += 12;
        }
        keys.push(u8::try_from(key).ok().filter(|k| *k <= 127)?);
    }
    keys.sort_unstable();
    Some(keys)
}

// ---------------------------------------------------------------------------------------------------------------
// Scales
// ---------------------------------------------------------------------------------------------------------------

/// One scale: semitone offsets from the tonic within one octave.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Scale {
    /// The name: `major`, `natural minor`.
    pub name: &'static str,
    /// The ascii slug: `ear.scale.<slug>`.
    pub skill: &'static str,
    /// The offsets, ascending, starting with 0, below 12.
    pub steps: &'static [u8],
}

const fn sc(name: &'static str, skill: &'static str, steps: &'static [u8]) -> Scale {
    Scale { name, skill, steps }
}

/// Major (Ionian): index into [`SCALES`].
pub const SCALE_MAJOR: u8 = 0;
/// Natural minor (Aeolian).
pub const SCALE_NATURAL_MINOR: u8 = 1;

/// The scales, index = the constants above. Append-only.
pub const SCALES: [Scale; 13] = [
    sc("major", "major", &[0, 2, 4, 5, 7, 9, 11]),
    sc("natural minor", "minor", &[0, 2, 3, 5, 7, 8, 10]),
    sc("harmonic minor", "harmonic-minor", &[0, 2, 3, 5, 7, 8, 11]),
    sc("melodic minor", "melodic-minor", &[0, 2, 3, 5, 7, 9, 11]),
    sc("dorian", "dorian", &[0, 2, 3, 5, 7, 9, 10]),
    sc("phrygian", "phrygian", &[0, 1, 3, 5, 7, 8, 10]),
    sc("lydian", "lydian", &[0, 2, 4, 6, 7, 9, 11]),
    sc("mixolydian", "mixolydian", &[0, 2, 4, 5, 7, 9, 10]),
    sc("locrian", "locrian", &[0, 1, 3, 5, 6, 8, 10]),
    sc("major pentatonic", "major-pentatonic", &[0, 2, 4, 7, 9]),
    sc("minor pentatonic", "minor-pentatonic", &[0, 3, 5, 7, 10]),
    sc("whole tone", "whole-tone", &[0, 2, 4, 6, 8, 10]),
    sc(
        "chromatic",
        "chromatic",
        &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11],
    ),
];

/// The key of `degree` (0 is the tonic, counting up through octaves) of `scale` on `tonic`. `None` for an unknown
/// scale or a key above 127.
#[must_use]
pub fn scale_key(tonic: u8, scale: u8, degree: u32) -> Option<u8> {
    let s = SCALES.get(usize::from(scale))?;
    let n = u32::try_from(s.steps.len()).ok()?;
    let octaves = degree / n;
    let step = u32::from(s.steps[(degree % n) as usize]);
    let key = u32::from(tonic) + octaves * 12 + step;
    u8::try_from(key).ok().filter(|k| *k <= 127)
}

// ---------------------------------------------------------------------------------------------------------------
// Progressions
// ---------------------------------------------------------------------------------------------------------------

/// One chord of a progression, relative to the tonic.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProgressionChord {
    /// Semitones of the chord's root above the tonic, 0 to 11.
    pub offset: u8,
    /// The quality: an index into [`CHORDS`].
    pub quality: u8,
}

/// A chord progression: a short loop in Roman numerals, over a major or a minor tonic.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Progression {
    /// The numerals joined by hyphens: `I-IV-V-I`.
    pub name: &'static str,
    /// The ascii slug: `ear.progression.<slug>`.
    pub skill: &'static str,
    /// Whether the tonic chord is minor (the numerals are then lowercase at `i`).
    pub minor: bool,
    /// The chords in order.
    pub chords: &'static [ProgressionChord],
}

const fn pc(offset: u8, quality: u8) -> ProgressionChord {
    ProgressionChord { offset, quality }
}

/// The progressions, index = id. Append-only. The first rows follow the order a student meets functions in (tonic,
/// subdominant, dominant, then the relative minor); sevenths and the minor loops come after.
pub const PROGRESSIONS: [Progression; 12] = [
    Progression {
        name: "I-IV-V-I",
        skill: "i-iv-v-i",
        minor: false,
        chords: &[pc(0, MAJOR), pc(5, MAJOR), pc(7, MAJOR), pc(0, MAJOR)],
    },
    Progression {
        name: "I-V-vi-IV",
        skill: "i-v-vi-iv",
        minor: false,
        chords: &[pc(0, MAJOR), pc(7, MAJOR), pc(9, MINOR), pc(5, MAJOR)],
    },
    Progression {
        name: "I-vi-IV-V",
        skill: "i-vi-iv-v",
        minor: false,
        chords: &[pc(0, MAJOR), pc(9, MINOR), pc(5, MAJOR), pc(7, MAJOR)],
    },
    Progression {
        name: "I-vi-ii-V",
        skill: "i-vi-ii-v",
        minor: false,
        chords: &[pc(0, MAJOR), pc(9, MINOR), pc(2, MINOR), pc(7, MAJOR)],
    },
    Progression {
        name: "I-IV-I-V",
        skill: "i-iv-i-v",
        minor: false,
        chords: &[pc(0, MAJOR), pc(5, MAJOR), pc(0, MAJOR), pc(7, MAJOR)],
    },
    Progression {
        name: "I-ii-V-I",
        skill: "i-ii-v-i",
        minor: false,
        chords: &[pc(0, MAJOR), pc(2, MINOR), pc(7, MAJOR), pc(0, MAJOR)],
    },
    Progression {
        name: "I-iii-IV-V",
        skill: "i-iii-iv-v",
        minor: false,
        chords: &[pc(0, MAJOR), pc(4, MINOR), pc(5, MAJOR), pc(7, MAJOR)],
    },
    Progression {
        name: "i-iv-V-i",
        skill: "min-i-iv-v-i",
        minor: true,
        chords: &[pc(0, MINOR), pc(5, MINOR), pc(7, MAJOR), pc(0, MINOR)],
    },
    Progression {
        name: "i-VI-III-VII",
        skill: "min-i-vi-iii-vii",
        minor: true,
        chords: &[pc(0, MINOR), pc(8, MAJOR), pc(3, MAJOR), pc(10, MAJOR)],
    },
    Progression {
        name: "i-VII-VI-V",
        skill: "min-i-vii-vi-v",
        minor: true,
        chords: &[pc(0, MINOR), pc(10, MAJOR), pc(8, MAJOR), pc(7, MAJOR)],
    },
    Progression {
        name: "i-iv-VII-III",
        skill: "min-i-iv-vii-iii",
        minor: true,
        chords: &[pc(0, MINOR), pc(5, MINOR), pc(10, MAJOR), pc(3, MAJOR)],
    },
    Progression {
        name: "ii7-V7-Imaj7",
        skill: "ii7-v7-imaj7",
        minor: false,
        chords: &[pc(2, MINOR7), pc(7, DOMINANT7), pc(0, MAJOR7)],
    },
];

/// The progression with this id, or `None` past the table.
#[must_use]
pub fn progression(id: u8) -> Option<&'static Progression> {
    PROGRESSIONS.get(usize::from(id))
}

/// The lowest key a voiced bass note may take: the bass of every chord sits in `BASS_LOW` to `BASS_LOW + 11` (C2 to
/// B2).
pub const BASS_LOW: u8 = 36;
/// The lowest and highest key of the upper voices of a voiced progression (G3 to C6).
pub const UPPER_RANGE: (u8, u8) = (55, 84);

/// Voice a progression in the key whose tonic has pitch class `tonic_pc` (0 to 11): per chord, the root in the bass
/// (C2 to B2) first, then the upper voices ascending (G3 to C6).
///
/// "Simple voice leading": the first chord is close position, root position, its root in C4 to B4; each next chord
/// takes the inversion and octave whose upper voices move least in total (the sum of the absolute differences between
/// the sorted voices of the two chords), the first of equal candidates in the order inversion 0, 1, ..., octave low to
/// high. Pure integer arithmetic, so the voicing is the same on every platform. Chords of different sizes compare the
/// voices they share.
///
/// # Panics
///
/// Never in practice: every `tonic_pc` below 12 and every table progression has a voicing (a test covers them all).
#[must_use]
pub fn voice_progression(tonic_pc: u8, progression: &Progression) -> Vec<Vec<u8>> {
    let tonic_pc = tonic_pc % 12;
    let mut out: Vec<Vec<u8>> = Vec::with_capacity(progression.chords.len());
    let mut previous: Option<Vec<u8>> = None;
    for c in progression.chords {
        let root_pc = (tonic_pc + c.offset) % 12;
        let bass = BASS_LOW + (root_pc + 12 - BASS_LOW % 12) % 12;
        // Candidate upper voicings: every inversion, shifted by whole octaves, kept inside the range.
        let mut best: Option<(u32, Vec<u8>)> = None;
        let upper = match &previous {
            None => chord_keys(MIDDLE_C + root_pc, c.quality, 0).unwrap_or_default(),
            Some(prev) => {
                let n = chord(c.quality).map_or(0, |q| q.steps.len());
                for inversion in 0..n {
                    // Start from an octave that puts the chord's root near the middle, then try shifts.
                    let base =
                        chord_keys(48 + root_pc, c.quality, inversion as u8).unwrap_or_default();
                    for shift in 0..4u8 {
                        let cand: Vec<u8> = base.iter().map(|k| k + 12 * shift).collect();
                        let (lo, hi) = (cand[0], cand[cand.len() - 1]);
                        if lo < UPPER_RANGE.0 || hi > UPPER_RANGE.1 {
                            continue;
                        }
                        let cost: u32 = cand
                            .iter()
                            .zip(prev.iter())
                            .map(|(a, b)| u32::from(a.abs_diff(*b)))
                            .sum();
                        if best.as_ref().is_none_or(|(c0, _)| cost < *c0) {
                            best = Some((cost, cand));
                        }
                    }
                }
                best.map(|(_, v)| v).unwrap_or_default()
            }
        };
        let mut voiced = Vec::with_capacity(upper.len() + 1);
        voiced.push(bass);
        voiced.extend_from_slice(&upper);
        previous = Some(upper);
        out.push(voiced);
    }
    out
}

/// Spelled-out interval label for logs: `"P5 (perfect fifth, 7 semitones)"`.
impl fmt::Display for Interval {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} ({}, {} semitones)",
            self.short, self.name, self.semitones
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn note_names_and_octaves() {
        assert_eq!(note_name(60), "C4");
        assert_eq!(note_name(61), "C#4");
        assert_eq!(note_name_flat(61), "Db4");
        assert_eq!(note_name(KEY_MIN), "C1");
        assert_eq!(note_name(KEY_MAX), "C7");
        assert_eq!(note_name(69), "A4");
        assert_eq!(pitch_class(69), 9);
        assert_eq!(octave(0), -1);
    }

    #[test]
    fn the_interval_table_is_indexed_by_size() {
        for (i, iv) in INTERVALS.iter().enumerate() {
            assert_eq!(usize::from(iv.semitones), i);
            assert_eq!(interval(iv.semitones), Some(iv));
        }
        assert_eq!(interval(13), None);
        assert_eq!(INTERVALS[7].short, "P5");
        assert_eq!(INTERVALS[6].name, "tritone");
        let mut skills: Vec<&str> = INTERVALS.iter().map(|i| i.skill).collect();
        skills.sort_unstable();
        skills.dedup();
        assert_eq!(skills.len(), INTERVALS.len(), "skill slugs are unique");
    }

    #[test]
    fn chords_build_with_their_inversions() {
        assert_eq!(chord_keys(60, MAJOR, 0), Some(vec![60, 64, 67]));
        assert_eq!(chord_keys(60, MAJOR, 1), Some(vec![64, 67, 72]));
        assert_eq!(chord_keys(60, MAJOR, 2), Some(vec![67, 72, 76]));
        assert_eq!(chord_keys(60, MAJOR, 3), None);
        assert_eq!(chord_keys(60, MINOR, 0), Some(vec![60, 63, 67]));
        assert_eq!(chord_keys(60, DOMINANT7, 3), Some(vec![70, 72, 76, 79]));
        assert_eq!(chord_keys(60, 11, 0), None);
        assert_eq!(chord_keys(125, MAJOR, 0), None, "a key above 127");
        for (i, q) in CHORDS.iter().enumerate() {
            assert_eq!(q.steps[0], 0, "{}", q.name);
            assert!(q.steps.windows(2).all(|w| w[0] < w[1]), "{}", q.name);
            assert!(q.steps.iter().all(|s| *s < 12), "{}", q.name);
            assert_eq!(chord(i as u8), Some(q));
        }
        let mut skills: Vec<&str> = CHORDS.iter().map(|c| c.skill).collect();
        skills.sort_unstable();
        skills.dedup();
        assert_eq!(skills.len(), CHORDS.len());
    }

    #[test]
    fn scales_are_ascending_and_index_through_octaves() {
        for s in &SCALES {
            assert_eq!(s.steps[0], 0, "{}", s.name);
            assert!(s.steps.windows(2).all(|w| w[0] < w[1]), "{}", s.name);
            assert!(s.steps.iter().all(|x| *x < 12), "{}", s.name);
        }
        assert_eq!(scale_key(60, SCALE_MAJOR, 0), Some(60));
        assert_eq!(scale_key(60, SCALE_MAJOR, 2), Some(64));
        assert_eq!(scale_key(60, SCALE_MAJOR, 7), Some(72));
        assert_eq!(scale_key(60, SCALE_MAJOR, 9), Some(76));
        assert_eq!(scale_key(60, SCALE_NATURAL_MINOR, 2), Some(63));
        assert_eq!(scale_key(60, 13, 0), None);
        assert_eq!(scale_key(120, SCALE_MAJOR, 14), None);
    }

    #[test]
    fn progressions_are_well_formed_and_voice_within_range() {
        for (id, p) in PROGRESSIONS.iter().enumerate() {
            assert!(progression(id as u8).is_some());
            assert!((3..=4).contains(&p.chords.len()), "{}", p.name);
            for tonic in 0..12u8 {
                let voiced = voice_progression(tonic, p);
                assert_eq!(voiced.len(), p.chords.len());
                for (chord, c) in voiced.iter().zip(p.chords) {
                    let q = &CHORDS[usize::from(c.quality)];
                    assert_eq!(chord.len(), q.steps.len() + 1, "{} in {tonic}", p.name);
                    // The bass is the root.
                    assert_eq!(
                        pitch_class(chord[0]),
                        (tonic + c.offset) % 12,
                        "{} in {tonic}",
                        p.name
                    );
                    assert!((BASS_LOW..BASS_LOW + 12).contains(&chord[0]));
                    // The upper voices are the chord tones, ascending, inside the range.
                    let mut want: Vec<u8> = q
                        .steps
                        .iter()
                        .map(|s| (tonic + c.offset + s) % 12)
                        .collect();
                    let mut got: Vec<u8> = chord[1..].iter().map(|k| pitch_class(*k)).collect();
                    want.sort_unstable();
                    got.sort_unstable();
                    assert_eq!(got, want, "{} in {tonic}", p.name);
                    assert!(chord[1..].windows(2).all(|w| w[0] < w[1]));
                    assert!(chord[1] >= UPPER_RANGE.0 && chord[chord.len() - 1] <= UPPER_RANGE.1);
                    assert!(chord.iter().all(|k| (KEY_MIN..=KEY_MAX).contains(k)));
                }
            }
        }
        assert_eq!(progression(PROGRESSIONS.len() as u8), None);
    }

    #[test]
    fn voice_leading_is_smooth_and_pinned() {
        // I-IV-V-I in C: close position first, then the nearest inversions.
        let voiced = voice_progression(0, &PROGRESSIONS[0]);
        assert_eq!(
            voiced,
            vec![
                vec![36, 60, 64, 67],
                vec![41, 60, 65, 69],
                vec![43, 59, 62, 67],
                vec![36, 60, 64, 67],
            ]
        );
        // No upper voice leaps more than a fourth in a I-IV-V-I loop.
        for tonic in 0..12 {
            let v = voice_progression(tonic, &PROGRESSIONS[0]);
            for pair in v.windows(2) {
                for (a, b) in pair[0][1..].iter().zip(&pair[1][1..]) {
                    assert!(a.abs_diff(*b) <= 5, "{tonic}: {pair:?}");
                }
            }
        }
    }

    #[test]
    fn the_tables_are_pinned() {
        // Append-only: these shapes are what ear generator v1's answer keys index into.
        let names: Vec<&str> = PROGRESSIONS.iter().map(|p| p.name).collect();
        assert_eq!(
            names,
            [
                "I-IV-V-I",
                "I-V-vi-IV",
                "I-vi-IV-V",
                "I-vi-ii-V",
                "I-IV-I-V",
                "I-ii-V-I",
                "I-iii-IV-V",
                "i-iv-V-i",
                "i-VI-III-VII",
                "i-VII-VI-V",
                "i-iv-VII-III",
                "ii7-V7-Imaj7"
            ]
        );
        let chords: Vec<&str> = CHORDS.iter().map(|c| c.skill).collect();
        assert_eq!(
            chords,
            [
                "maj", "min", "dim", "aug", "sus2", "sus4", "dom7", "maj7", "min7", "hdim7", "dim7"
            ]
        );
        let scales: Vec<&str> = SCALES.iter().map(|s| s.skill).collect();
        assert_eq!(scales[0], "major");
        assert_eq!(scales[12], "chromatic");
        assert_eq!(
            interval(7).map(ToString::to_string),
            Some("P5 (perfect fifth, 7 semitones)".to_owned())
        );
    }
}
