//! **Owner: lane ECK.** The staircase simulations (docs/ear.md, "Difficulty and the staircase" and "GR4"): simulated
//! listeners with known thresholds play the 3-down/1-up drill (`enth_learn::drill::Staircase`) over a game's **measured**
//! ladder, and the estimate it returns is held to the F1e judge's ruling. Lane DRL's tests also simulate the staircase,
//! in `enth-learn`, over invented ladders of a given slope; this is the check of the shipped games, with the slope of
//! each ladder as `sound/1` measures it.
//!
//! **The ladder** is one row per level: the distance, in JND units, of the pair of options that limits the level (the
//! median over items of each item's closest pair) and how many options the level offers. Level 1 is the easiest.
//!
//! **The listener** hears a distance `d` (JND) with `d' = 0.954 * gain * d`, the sensitivity at which an ideal
//! two-alternative observer is right 75 % of the time at one JND (`Phi(d' / sqrt 2) = 0.75`; one JND is that
//! distance), and answers an `m`-option question with the Gaussian m-alternative rule:
//! `P(right) = (1 - lapse) * Pm(d') + lapse / m`, `Pm(d') = integral phi(x - d') * Phi(x)^(m-1) dx`. Listeners differ in
//! **acuity**, the `gain`: 0.5, 0.7, 1, 1.4 and 2 times the JND scale's, with lapse 0 and 3 %. A listener's **true
//! threshold** is a consequence of its gain and the ladder: the (fractional) level where the chance of a right answer
//! is `0.5^(1/3)`, the point a 3-down/1-up track converges to (about 79 %). The ladder's own distances fix how fast
//! the chance falls above and below it, so a ladder of half a JND per level gives every listener a shallow curve.
//! Listeners whose threshold is off the ladder (it is too easy or too hard for them) are listed and not simulated; a
//! ladder that holds fewer than three of the five acuities cannot place the people it is for.
//!
//! **Runs** are 40 trials, 300 per cell, from three starts: the threshold's level, one level too hard and one too
//! easy; draws come from `enth-dsp`'s pinned `Rng`, keyed by the cell and run.
//!
//! **Criteria** (the F1e judge's ruling):
//!
//! | Ladder | Estimate within |
//! | --- | --- |
//! | **steep**: about 0.75 JND per level or more (median over the ladder) | one level in at least 90 % of runs pooled, and at least 75 % in every listener-and-start cell |
//! | **flat**: less | two levels in at least 90 % of runs pooled |
//!
//! and, for both: a listener who starts at its threshold is right 75 to 85 % of the time over its runs (the drill
//! trains at about 80 %), and the ladder is monotone (a harder level is never more than a quarter JND easier than the
//! one below it).

use enth_dsp::util::rng::{Rng, derive, fnv1a64};
use enth_learn::drill::Staircase;

/// The chance of a right answer at the staircase's fixed point: `0.5^(1/3)`.
pub const TARGET: f64 = 0.7937;

/// The sensitivity at one JND: an ideal two-alternative observer is right 75 % of the time.
pub const D_PRIME_AT_JND: f64 = 0.954;

/// Trials per run.
pub const TRIALS: u32 = 40;

/// The shallowest a steep ladder is: JND per level (the median step).
pub const STEEP_STEP: f64 = 0.75;

/// A level of a ladder.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Row {
    /// The distance of the pair that limits the level, in JND units.
    pub jnd: f64,
    /// How many options the level offers.
    pub options: usize,
}

/// A ladder of difficulty rows, level 1 first (easiest).
#[derive(Clone, Debug, PartialEq)]
pub struct Ladder {
    /// Its name in reports.
    pub name: String,
    /// The rows.
    pub rows: Vec<Row>,
}

impl Ladder {
    /// The median drop in distance per level (JND per level), over adjacent rows; 0 for fewer than two rows.
    #[must_use]
    pub fn step(&self) -> f64 {
        let mut drops: Vec<f64> = self.rows.windows(2).map(|w| w[0].jnd - w[1].jnd).collect();
        if drops.is_empty() {
            return 0.0;
        }
        drops.sort_by(f64::total_cmp);
        let mid = drops.len() / 2;
        if drops.len() % 2 == 1 {
            drops[mid]
        } else {
            drops[mid - 1].midpoint(drops[mid])
        }
    }

    /// Whether a harder level is never more than a quarter JND easier than the one below it.
    #[must_use]
    pub fn monotone(&self) -> bool {
        self.rows.windows(2).all(|w| w[1].jnd <= w[0].jnd + 0.25)
    }
}

// ---------------------------------------------------------------------------------------------------------------
// The m-alternative listener

/// The standard normal density.
fn phi(x: f64) -> f64 {
    (-0.5 * x * x).exp() / (2.0 * std::f64::consts::PI).sqrt()
}

/// The standard normal distribution (Abramowitz and Stegun 7.1.26, error under 1.5e-7).
fn big_phi(x: f64) -> f64 {
    let z = x.abs() / std::f64::consts::SQRT_2;
    let t = 1.0 / (1.0 + 0.327_591_1 * z);
    let poly = t
        * (0.254_829_592
            + t * (-0.284_496_736
                + t * (1.421_413_741 + t * (-1.453_152_027 + t * 1.061_405_429))));
    let erf = 1.0 - poly * (-z * z).exp();
    1.0_f64.midpoint(if x < 0.0 { -erf } else { erf })
}

/// The chance of choosing the signal among `m` alternatives at sensitivity `d'` (Simpson over the density).
#[must_use]
pub fn p_afc(d_prime: f64, m: usize) -> f64 {
    let m = m.max(2);
    let (lo, hi) = (-8.0, d_prime.max(0.0) + 8.0);
    let steps = (((hi - lo) / 0.02).ceil() as usize + 1) & !1;
    let h = (hi - lo) / steps as f64;
    let f = |x: f64| phi(x - d_prime) * big_phi(x).powi(i32::try_from(m - 1).unwrap_or(i32::MAX));
    let mut sum = f(lo) + f(hi);
    for i in 1..steps {
        let x = lo + h * i as f64;
        sum += f(x) * if i % 2 == 1 { 4.0 } else { 2.0 };
    }
    (sum * h / 3.0).clamp(1.0 / m as f64, 1.0)
}

/// The chance of a right answer at distance `jnd` for a listener of `gain`.
fn p_right(gain: f64, jnd: f64, options: usize, lapse: f64) -> f64 {
    let m = options.max(2);
    (1.0 - lapse) * p_afc(D_PRIME_AT_JND * gain * jnd.max(0.0), m) + lapse / m as f64
}

/// The acuities of the simulated listeners: times the sensitivity of the JND scale.
pub const GAINS: [f64; 5] = [0.5, 0.7, 1.0, 1.4, 2.0];

/// The lapse rates of the simulated listeners.
pub const LAPSES: [f64; 2] = [0.0, 0.03];

/// The fewest acuities (of [`GAINS`]) a ladder must hold a threshold of.
pub const MIN_IN_RANGE: usize = 3;

/// One simulated listener over a ladder.
#[derive(Clone, Debug)]
struct Listener {
    /// The chance of a right answer at each level.
    p: Vec<f64>,
    /// The true threshold: the highest level (a fraction) where a right answer is still [`TARGET`] likely, or why the
    /// listener is off the ladder.
    threshold: Result<f64, &'static str>,
}

fn listener(ladder: &Ladder, gain: f64, lapse: f64) -> Listener {
    let p: Vec<f64> = ladder
        .rows
        .iter()
        .map(|r| p_right(gain, r.jnd, r.options, lapse))
        .collect();
    let n = p.len();
    // The last level a right answer is still target-likely, and where the curve crosses to the next one.
    let threshold = match p.iter().rposition(|v| *v >= TARGET) {
        None => Err("the ladder is too hard"),
        Some(i) if i + 1 >= n => Err("the ladder is too easy"),
        Some(i) => {
            let t = (i + 1) as f64 + (p[i] - TARGET) / (p[i] - p[i + 1]);
            // A threshold in the first or last half level is not placed by a staircase (the ends swallow moves).
            if t < 1.5 {
                Err("at the easy end")
            } else if t > n as f64 - 0.5 {
                Err("at the hard end")
            } else {
                Ok(t)
            }
        }
    };
    Listener { p, threshold }
}

// ---------------------------------------------------------------------------------------------------------------
// The evaluation

/// What the simulations found for one ladder.
#[derive(Clone, Debug, PartialEq)]
pub struct Evaluation {
    /// How many levels.
    pub levels: usize,
    /// The median JND per level.
    pub step: f64,
    /// Whether the ladder is flat (under [`STEEP_STEP`] per level).
    pub flat: bool,
    /// Whether the ladder is monotone.
    pub monotone: bool,
    /// The share of runs whose estimate is within one level of the truth, pooled over every cell.
    pub within_one: f64,
    /// The share within two levels.
    pub within_two: f64,
    /// The worst cell's share within one level, and which cell.
    pub worst_one: (f64, String),
    /// The success rate of the listeners who start at their threshold: the lowest and the highest.
    pub success: (f64, f64),
    /// The share of runs that ended with no estimate (fewer than four reversals).
    pub no_estimate: f64,
    /// How many of the [`GAINS`] have a threshold on the ladder (at the lapse of 0).
    pub in_range: usize,
    /// The acuities with no threshold on the ladder, each with why.
    pub off: Vec<(f64, &'static str)>,
    /// Cells simulated.
    pub cells: usize,
    /// Runs per cell.
    pub runs: u64,
}

impl Evaluation {
    /// The estimate criterion for this ladder's class: steep, within one level in 90 % pooled and 75 % in every cell;
    /// flat, within two levels in 90 % pooled.
    #[must_use]
    pub fn estimate_ok(&self) -> bool {
        if self.cells == 0 {
            return false;
        }
        if self.flat {
            self.within_two >= 0.90
        } else {
            self.within_one >= 0.90 && self.worst_one.0 >= 0.75
        }
    }

    /// Whether training at a listener's threshold holds 75 to 85 % success.
    #[must_use]
    pub fn success_ok(&self) -> bool {
        self.cells > 0 && self.success.0 >= 0.75 && self.success.1 <= 0.85
    }

    /// Whether the ladder holds the thresholds of enough acuities to place the people it is for.
    #[must_use]
    pub fn range_ok(&self) -> bool {
        self.in_range >= MIN_IN_RANGE
    }

    /// Whether every criterion holds.
    #[must_use]
    pub fn passes(&self) -> bool {
        self.monotone && self.range_ok() && self.estimate_ok() && self.success_ok()
    }
}

/// The fewest levels a ladder needs to be simulated.
pub const MIN_LEVELS: usize = 4;

/// Simulate `runs` 40-trial runs per cell over `ladder`; `None` when it has fewer than [`MIN_LEVELS`] levels.
#[must_use]
pub fn evaluate(ladder: &Ladder, runs: u64, seed: u64) -> Option<Evaluation> {
    let n = ladder.rows.len();
    if n < MIN_LEVELS {
        return None;
    }
    let levels = u8::try_from(n).ok()?;
    let (mut one, mut two, mut none, mut total) = (0_u64, 0_u64, 0_u64, 0_u64);
    let mut worst = (1.0_f64, String::new());
    let mut success = (1.0_f64, 0.0_f64);
    let (mut cells, mut in_range) = (0, 0);
    let mut off = Vec::new();
    for (gi, gain) in GAINS.iter().enumerate() {
        for (li, lapse) in LAPSES.iter().enumerate() {
            let l = listener(ladder, *gain, *lapse);
            let threshold = match l.threshold {
                Ok(t) => t,
                Err(why) => {
                    if li == 0 {
                        off.push((*gain, why));
                    }
                    continue;
                }
            };
            if li == 0 {
                in_range += 1;
            }
            let at = (threshold.round() as u8).clamp(1, levels);
            for (si, (tag, start)) in [
                ("at", at),
                ("hard", (at + 1).min(levels)),
                ("easy", at.saturating_sub(1).max(1)),
            ]
            .iter()
            .enumerate()
            {
                cells += 1;
                let cell_seed = derive(
                    derive(seed, fnv1a64(b"ear-check/staircase")),
                    (gi * 100 + li * 10 + si) as u64,
                );
                let (mut hit, mut right, mut trials) = (0_u64, 0_u64, 0_u64);
                for run in 0..runs {
                    let mut rng = Rng::new(derive(cell_seed, run));
                    let mut s = Staircase::new(levels, *start);
                    for _ in 0..TRIALS {
                        let p = l.p[usize::from(s.level()) - 1];
                        s.answer(rng.unit_f64() < p);
                    }
                    total += 1;
                    match s.estimate() {
                        Some(e) => {
                            let off = (e - threshold).abs();
                            one += u64::from(off <= 1.0);
                            two += u64::from(off <= 2.0);
                            hit += u64::from(off <= 1.0);
                        }
                        None => none += 1,
                    }
                    right += u64::from(s.right());
                    trials += u64::from(s.trials());
                }
                let share = hit as f64 / runs as f64;
                if share < worst.0 {
                    worst = (
                        share,
                        format!(
                            "acuity {gain}, threshold {threshold:.1}, lapse {lapse}, from {tag}"
                        ),
                    );
                }
                if *tag == "at" {
                    let rate = right as f64 / trials as f64;
                    success = (success.0.min(rate), success.1.max(rate));
                }
            }
        }
    }
    let step = ladder.step();
    let share = |x: u64| {
        if total == 0 {
            0.0
        } else {
            x as f64 / total as f64
        }
    };
    Some(Evaluation {
        levels: n,
        step,
        flat: step < STEEP_STEP,
        monotone: ladder.monotone(),
        within_one: share(one),
        within_two: share(two),
        worst_one: if cells == 0 {
            (0.0, "no listener is on the ladder".to_owned())
        } else {
            worst
        },
        success: if cells == 0 { (0.0, 0.0) } else { success },
        no_estimate: share(none),
        in_range,
        off,
        cells,
        runs,
    })
}
