//! **Owner: lane ECK.** Audibility per level (docs/ear.md, "GR4"): every option pair at least 2 JND apart under
//! `sound/1` (at least 6 at level 1), measured offline from the rendered questions. Pairs `sound/1` cannot hear are
//! listed apart as *not measured* and get a listening review. The ladder lanes (G1 to G4) read these tables to place
//! their levels.
//!
//! **What is a pair.** For an item with options `0..n`, the clip of each option is what the question would sound like if
//! that option were right (`game.rs`). The distance of two options is the distance of their clips, in the units of the
//! item's own calibration (`metric.rs`):
//!
//! | Game | The pairs |
//! | --- | --- |
//! | label games (4 to 9) | every two options' clips, under the calibration of the right answer's clip |
//! | Spot the change | every two B clips (which control moved), and A against each B (a change is heard at all) |
//! | Ear check | the base against the odd interval, which is every two positions' difference |
//!
//! **Not measured**, and why (the listening review's list):
//!
//! - **order or timing only**: two options in which every pitch class sounds as long as in the other and the notes
//!   still differ (rhythm patterns of the same size, progressions with the same chords in another order). `sound/1`'s
//!   features are pooled over the clip, so they carry no order or placement; the pair is credited to the ideal observer
//!   as distinguishable, and listed;
//! - **a control `sound/1` is deaf to**: attacks under about 60 ms, sustain, slow beating (`DEAF`).
//!
//! **Cost.** A music item is 19 renders for the calibration and one per other option (about 0.5 s); a parameter game's
//! clips are calibrated once each (the base is shared). The sample per (game, level) is small and seeded.

use enth_challenge::ear::question::Clip as Played;
use enth_challenge::score::sound;

use super::game::{Item, Layout};
use super::metric::{self, Calibrations, Playable};

/// The least distance of two options, in JND units, at levels above the first.
pub const MIN_JND: f64 = 2.0;

/// The least distance at level 1 (docs/ear.md: "at least 6 at L1").
pub const MIN_JND_L1: f64 = 6.0;

/// The least distance for a level.
#[must_use]
pub fn floor_for(level: u8) -> f64 {
    if level <= 1 { MIN_JND_L1 } else { MIN_JND }
}

/// Controls (by parameter key) `sound/1` cannot hear, with the reason; an attack counts only under 60 ms.
const DEAF: [(&str, &str); 3] = [
    ("attack", "an attack under about 60 ms"),
    ("sustain", "a sustain level"),
    ("detune", "slow beating"),
];

/// Why a control cannot be measured, if it cannot, given its values in the two states compared.
fn deaf_reason(control: &str, values: [Option<f32>; 2]) -> Option<&'static str> {
    let key = control.rsplit('.').next().unwrap_or(control);
    let (_, why) = DEAF.iter().find(|(k, _)| *k == key)?;
    if key == "attack" && values.iter().flatten().any(|v| *v >= 60.0) {
        return None;
    }
    Some(why)
}

/// A pair of options and how far apart they are.
#[derive(Clone, Debug, PartialEq)]
pub struct PairOut {
    /// The first option.
    pub a: usize,
    /// The second option (`a < b`).
    pub b: usize,
    /// The distance in JND units, or why `sound/1` cannot say.
    pub jnd: Result<f64, &'static str>,
}

/// One item, measured.
#[derive(Clone, Debug)]
pub struct Measured {
    /// How many options.
    pub options: usize,
    /// The right option.
    pub key: usize,
    /// Every pair of options.
    pub pairs: Vec<PairOut>,
    /// The change pairs (A against each B; base against odd), by option for Spot the change, one for Ear check.
    pub changes: Vec<Result<f64, &'static str>>,
    /// Each option's loudness before `sound/1`'s gain (LUFS), as the player would hear that option's clip.
    pub loudness: Vec<Option<f64>>,
}

impl Measured {
    /// The distance between options `i` and `j` (`None` when not measured or `i == j`).
    #[must_use]
    pub fn distance(&self, i: usize, j: usize) -> Option<f64> {
        let (a, b) = if i < j { (i, j) } else { (j, i) };
        self.pairs
            .iter()
            .find(|p| p.a == a && p.b == b)
            .and_then(|p| p.jnd.ok())
    }

    /// The closest measured pair or change, in JND units.
    #[must_use]
    pub fn closest(&self) -> Option<f64> {
        self.pairs
            .iter()
            .map(|p| p.jnd)
            .chain(self.changes.iter().copied())
            .filter_map(Result::ok)
            .min_by(f64::total_cmp)
    }

    /// The closest measured pair or change and what it is: the distance and "options 1 and 2" or "the change of
    /// option 1" (Spot the change) or "the base against the odd interval" (Ear check).
    #[must_use]
    pub fn closest_what(&self, layout: Layout) -> Option<(f64, String)> {
        let pairs = self.pairs.iter().filter_map(|p| {
            p.jnd
                .ok()
                .map(|j| (j, format!("options {} and {}", p.a, p.b)))
        });
        let changes = self.changes.iter().enumerate().filter_map(|(k, c)| {
            c.ok().map(|j| {
                let what = match layout {
                    Layout::OddOne => "the base against the odd interval".to_owned(),
                    _ => format!("the change of option {k}"),
                };
                (j, what)
            })
        });
        pairs.chain(changes).min_by(|a, b| a.0.total_cmp(&b.0))
    }

    /// How many pairs and changes `sound/1` could not measure.
    #[must_use]
    pub fn unmeasured(&self) -> usize {
        self.pairs
            .iter()
            .map(|p| p.jnd)
            .chain(self.changes.iter().copied())
            .filter(Result::is_err)
            .count()
    }

    /// How many pairs and changes there are.
    #[must_use]
    pub fn total(&self) -> usize {
        self.pairs.len() + self.changes.len()
    }

    /// The change the answer must be heard through (`None` for the label games).
    #[must_use]
    pub fn detect(&self, item_layout: Layout) -> Option<Result<f64, &'static str>> {
        match item_layout {
            Layout::PerOption => None,
            Layout::Changed => self.changes.get(self.key).copied(),
            Layout::OddOne => self.changes.first().copied(),
        }
    }
}

/// The time each pitch class sounds in a stimulus clip (the sum of its notes' holds, ms), and its timbre and band.
fn pitch_class_time(clip: &Played) -> Option<([u64; 12], u8, Option<enth_music::ear::EqBand>)> {
    let Played::Stimulus(s) = clip else {
        return None;
    };
    let mut time = [0_u64; 12];
    for n in &s.notes {
        time[usize::from(n.key % 12)] += u64::from(n.hold_ms);
    }
    Some((time, s.timbre, s.band))
}

/// Whether two stimuli differ in order or timing only: the same timbre and band, every pitch class sounding as long in
/// one as in the other, and yet not the same notes. `sound/1` pools its features over the clip, so it cannot tell
/// I-IV-V-I from I-V-IV-I, or one rhythm pattern from another of the same size.
fn order_only(a: &Played, b: &Played) -> bool {
    match (a, b, pitch_class_time(a), pitch_class_time(b)) {
        (Played::Stimulus(x), Played::Stimulus(y), Some(ha), Some(hb)) => {
            ha == hb && x.notes != y.notes
        }
        _ => false,
    }
}

/// Measure an item's pairs.
///
/// # Errors
/// A render or analysis failed (named for the report).
pub fn measure(item: &Item, cache: &Calibrations) -> Result<Measured, String> {
    let playable: Vec<Playable> = item
        .states
        .iter()
        .map(Playable::of)
        .collect::<Result<_, _>>()?;
    let heard: Vec<sound::Clip> = playable
        .iter()
        .map(metric::hear)
        .collect::<Result<_, _>>()?;
    // The distance of two states, in JND units.
    let calib_of = |s: usize| cache.get(&item.states[s], &playable[s]);
    let value = |s: usize, control: &str| -> Option<f32> {
        match &item.states[s] {
            Played::Patch { patch, .. } => control.parse().ok().and_then(|a| patch.value(&a)),
            Played::Stimulus(_) => None,
        }
    };
    let pair_jnd = |x: usize, y: usize| -> Result<f64, String> {
        match item.layout {
            Layout::PerOption => {
                // One calibration per item: the right answer's clip.
                let c = calib_of(item.key)?;
                Ok(metric::jnd_between(&c, &heard[x], &heard[y]))
            }
            Layout::Changed | Layout::OddOne => {
                let (cx, cy) = (calib_of(x)?, calib_of(y)?);
                Ok(metric::jnd_between(&cx, &heard[x], &heard[y])
                    .min(metric::jnd_between(&cy, &heard[x], &heard[y])))
            }
        }
    };
    let control_of = |option: usize| -> Option<&str> {
        match item.layout {
            Layout::Changed => item.controls.get(option).map(String::as_str),
            Layout::OddOne => item.controls.first().map(String::as_str),
            Layout::PerOption => None,
        }
    };
    let mut pairs = Vec::new();
    for a in 0..item.options {
        for b in a + 1..item.options {
            let (x, y) = item.option_pair(a, b);
            let why = if item.layout == Layout::PerOption {
                order_only(&item.states[x], &item.states[y])
                    .then_some("the options differ only in order or timing")
            } else {
                [a, b].iter().find_map(|o| {
                    control_of(*o).and_then(|c| deaf_reason(c, [value(x, c), value(y, c)]))
                })
            };
            let jnd = match why {
                Some(w) => Err(w),
                None => Ok(pair_jnd(x, y)?),
            };
            pairs.push(PairOut { a, b, jnd });
        }
    }
    let mut changes = Vec::new();
    for (x, y) in item.change_pairs() {
        let option = if item.layout == Layout::Changed {
            y - 1
        } else {
            item.key
        };
        let why = control_of(option).and_then(|c| deaf_reason(c, [value(x, c), value(y, c)]));
        changes.push(match why {
            Some(w) => Err(w),
            None => Ok(pair_jnd(x, y)?),
        });
    }
    let loudness = (0..item.options)
        .map(|j| {
            let state = match item.layout {
                Layout::PerOption => j,
                Layout::Changed => 1 + j,
                Layout::OddOne => usize::from(j == item.key),
            };
            metric::lufs(&heard[state])
        })
        .collect();
    Ok(Measured {
        options: item.options,
        key: item.key,
        pairs,
        changes,
        loudness,
    })
}

/// The ideal observer's chance of a right answer on a measured item, exactly.
///
/// The observer hears the item's clip and picks uniformly among the options it cannot tell from it: those within one
/// JND of the right answer's clip. If the change itself is not heard (Spot the change and Ear check: under one JND from
/// the reference) it can tell nothing and guesses. A pair `sound/1` cannot measure is credited as distinguishable.
#[must_use]
pub fn observer_chance(m: &Measured, layout: Layout) -> f64 {
    let n = m.options.max(1) as f64;
    if let Some(Ok(d)) = m.detect(layout)
        && d < 1.0
    {
        return 1.0 / n;
    }
    let confusable = (0..m.options)
        .filter(|&j| j != m.key)
        .filter(|&j| m.distance(m.key, j).is_some_and(|d| d < 1.0))
        .count();
    1.0 / (1 + confusable) as f64
}

/// The summary of a cell's measured items.
#[derive(Clone, Debug, PartialEq)]
pub struct Summary {
    /// Items measured.
    pub items: usize,
    /// The least distance asked at this level.
    pub need: f64,
    /// Items whose closest measured pair is under `need`.
    pub short: usize,
    /// The closest pair of all items.
    pub min: Option<f64>,
    /// The 10th percentile of the items' closest pairs.
    pub p10: Option<f64>,
    /// The median of the items' closest pairs.
    pub median: Option<f64>,
    /// Pairs and changes `sound/1` could not measure, and all pairs and changes.
    pub unmeasured: (usize, usize),
    /// The reasons not measured, with counts.
    pub reasons: Vec<(&'static str, usize)>,
    /// The ideal observer's expected score over the items.
    pub observer: f64,
    /// Where the closest pair of all is: the item (its seed and index) and the pair (set by the caller, which has the
    /// items; `None` until then).
    pub worst: Option<String>,
}

impl Summary {
    /// Whether every item's closest measured pair reaches the level's distance.
    #[must_use]
    pub fn passes(&self) -> bool {
        self.short == 0
    }
}

/// Summarise a cell's measured items.
#[must_use]
pub fn summarise(level: u8, layout: Layout, items: &[Measured]) -> Summary {
    let need = floor_for(level);
    let mut closest: Vec<f64> = items.iter().filter_map(Measured::closest).collect();
    closest.sort_by(f64::total_cmp);
    let pick = |q: f64| -> Option<f64> {
        (!closest.is_empty()).then(|| closest[((closest.len() - 1) as f64 * q).floor() as usize])
    };
    let mut reasons: Vec<(&'static str, usize)> = Vec::new();
    for m in items {
        for r in m
            .pairs
            .iter()
            .map(|p| p.jnd)
            .chain(m.changes.iter().copied())
        {
            if let Err(why) = r {
                match reasons.iter_mut().find(|(w, _)| *w == why) {
                    Some((_, c)) => *c += 1,
                    None => reasons.push((why, 1)),
                }
            }
        }
    }
    Summary {
        items: items.len(),
        need,
        short: closest.iter().filter(|d| **d < need).count(),
        min: closest.first().copied(),
        p10: pick(0.1),
        median: pick(0.5),
        unmeasured: (
            items.iter().map(Measured::unmeasured).sum(),
            items.iter().map(Measured::total).sum(),
        ),
        reasons,
        observer: if items.is_empty() {
            0.0
        } else {
            items
                .iter()
                .map(|m| observer_chance(m, layout))
                .sum::<f64>()
                / items.len() as f64
        },
        worst: None,
    }
}
