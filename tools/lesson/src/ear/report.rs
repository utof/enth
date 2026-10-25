//! **Owner: lane ECK.** What `ear-check` found, and the exit policy.
//!
//! Every check is one [`Check`]: a name, the cell it was about (`Interval stacks L3`), pass or fail, and a [`Gate`].
//!
//! | Gate | Checks | Fails the run |
//! | --- | --- | --- |
//! | [`Gate::Hard`] | golden drift, the key oracle through the judge, random and deaf at chance, the bias bots at chance +- 3 %, structure (valid items, the counterfactual's identity, skill keys, repeat determinism) | always |
//! | [`Gate::Ladder`] | audibility, the ideal observer at the top level, the staircase simulations, coverage | when the game has two levels or more (its ladder is delivered, so green is the owner's acceptance), or under `--strict`; never under `--ladders report` |
//! | [`Gate::Note`] | the loudest option as rendered (playback level-matches clips, docs/ear.md, so a loudness difference is only a finding for the lane that owns the render) | never |
//!
//! A one-level ladder (every game today but the Ear check's reference steps) is a stub its owner lane (G1 to G4) tunes:
//! its ladder checks are printed as `report` and do not fail the run, so a stub can sit in the tree. The moment a game has
//! a second level the same checks are hard, which is G1 to G4's acceptance ("`ear-check` green on their games").

use std::fmt::Write as _;

/// Whether a failed check fails the run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gate {
    /// Always.
    Hard,
    /// Once the game has a ladder (two levels or more), or under `--strict`.
    Ladder,
    /// Never: a finding listed for the lane that owns it (what playback's level match must remove).
    Note,
}

/// How `--ladders` treats the [`Gate::Ladder`] checks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ladders {
    /// Hard for a game with two levels or more, report-only below (the default).
    Auto,
    /// Hard for every game (`--strict`).
    Enforce,
    /// Never hard.
    Report,
}

/// One check and its result.
#[derive(Clone, Debug, PartialEq)]
pub struct Check {
    /// The check's name (`golden`, `bias`, `audibility`, ...).
    pub name: &'static str,
    /// What it was about.
    pub cell: String,
    /// Whether it held.
    pub pass: bool,
    /// Whether a failure fails the run.
    pub gate: Gate,
    /// How many levels the game's ladder has (the [`Gate::Ladder`] rule).
    pub levels: u8,
    /// The numbers, for the report.
    pub detail: String,
}

impl Check {
    /// Whether a failure of this check fails the run under `ladders`.
    #[must_use]
    pub fn enforced(&self, ladders: Ladders) -> bool {
        match self.gate {
            Gate::Hard => true,
            Gate::Note => false,
            Gate::Ladder => match ladders {
                Ladders::Auto => self.levels >= 2,
                Ladders::Enforce => true,
                Ladders::Report => false,
            },
        }
    }
}

/// Everything a run found.
#[derive(Clone, Debug, Default)]
pub struct Report {
    /// The checks, in the order they were made.
    pub checks: Vec<Check>,
}

impl Report {
    /// Add a check.
    pub fn push(&mut self, c: Check) {
        self.checks.push(c);
    }

    /// Add a passing or failing check.
    pub fn check(
        &mut self,
        name: &'static str,
        cell: impl Into<String>,
        pass: bool,
        gate: Gate,
        levels: u8,
        detail: impl Into<String>,
    ) {
        self.push(Check {
            name,
            cell: cell.into(),
            pass,
            gate,
            levels,
            detail: detail.into(),
        });
    }

    /// The failures that fail the run.
    #[must_use]
    pub fn failures(&self, ladders: Ladders) -> Vec<&Check> {
        self.checks
            .iter()
            .filter(|c| !c.pass && c.enforced(ladders))
            .collect()
    }

    /// The failures that are reported and do not fail the run.
    #[must_use]
    pub fn reported(&self, ladders: Ladders) -> Vec<&Check> {
        self.checks
            .iter()
            .filter(|c| !c.pass && !c.enforced(ladders))
            .collect()
    }

    /// The summary lines: counts, then every failure and every report-only failure.
    #[must_use]
    pub fn summary(&self, ladders: Ladders) -> String {
        let (fails, reports) = (self.failures(ladders), self.reported(ladders));
        let passed = self.checks.iter().filter(|c| c.pass).count();
        let mut s = String::new();
        let _ = writeln!(
            s,
            "ear-check: {} checks, {passed} pass, {} fail the run, {} report only",
            self.checks.len(),
            fails.len(),
            reports.len()
        );
        for c in &fails {
            let _ = writeln!(s, "  FAIL {} [{}]: {}", c.name, c.cell, c.detail);
        }
        for c in &reports {
            let _ = writeln!(s, "  report {} [{}]: {}", c.name, c.cell, c.detail);
        }
        s
    }
}

/// `x` as a percentage with one decimal.
#[must_use]
pub fn pct(x: f64) -> String {
    format!("{:.1}%", 100.0 * x)
}

/// An optional JND distance for a table: `-` when there is none.
#[must_use]
pub fn jnd(x: Option<f64>) -> String {
    x.map_or_else(|| "-".to_owned(), |v| format!("{v:.1}"))
}
