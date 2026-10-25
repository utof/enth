//! **Owner: lane ECK** (Ear check). Pre-declared by lane S0e (R4/R5), built in R4 wave 3. The reference is docs/ear.md,
//! "GR4" (the mechanical half), "The ear generator" and "`ear-check` as built".
//!
//! `ear-check`: GR4's mechanical half, as `check` is the lessons'. `cargo xtask ear-check` is a thin driver of this
//! command (`xtask/src/ear.rs`), so `xtask` stays light, and `.github/workflows/ear.yml` runs it on `crates/enth-music/**`,
//! the ear files of `enth-challenge`, this directory and the crates a question's sound depends on, under 10 minutes. It
//! fails on a seeded bad game of each kind (`tests.rs` plants each): a leak a bias bot finds, an inaudible pair, an
//! unsolvable top level, a drifting hash. One file per concern, so the ladder lanes (G1 to G4) and ECK never edit each
//! other's:
//!
//! | File | Holds |
//! | --- | --- |
//! | `game` | an [`game::EarGame`] draws [`game::Item`]s: a question **and the clip of every option** (the counterfactual builders: `enth_music::ladder::with_answer`, the parameter games' public API) |
//! | `metric` | `sound/1`'s analysis and calibration replayed over any clip, in JND units; held to `score_round` by the anchor test |
//! | `players` | the key oracle, the ideal observer, the random and deaf players and the bias bots (first, last, longest, loudest, highest, densest), all judged by the real `ear/1` |
//! | `audibility` | every option pair's distance in JND units per level (at least 2, at least 6 at level 1), the pairs `sound/1` cannot hear listed as *not measured*, the ideal observer's chance |
//! | `staircase` | simulated listeners with known thresholds over a measured ladder: the 3-down/1-up estimate against the F1e judge's criteria |
//! | `steps` | the reference ladder: Ear check's parameter steps (8 down to 1) per control, measured, and simulated |
//! | `golden` | ear generator v1's golden recomputed from the file, and 1,000 items per (game, level) hashed (the question and every option's clip) |
//! | `coverage` | every observable Basic-tier parameter is in game 1 or 2; bursts find five items per skill and move Review boxes |
//! | `check` | a game's cells played and judged; the ladders simulated |
//! | `report` | the checks and the exit policy |
//!
//! **Exit policy.** Hard, always: golden drift, the key oracle, random and deaf at chance, the bias bots at chance +- 3 %,
//! structure (items valid and repeatable, the counterfactual's identity, Review skill keys, bursts). Ladder criteria
//! (audibility, the ideal observer at the top level, the staircase simulations, parameter coverage): **report only
//! while a game has a single level** (a stub its lane G1 to G4 tunes), hard from two levels on, which is those lanes'
//! acceptance ("`ear-check` green on their games"). `--strict` hardens every one; `--ladders report` softens them all.

mod audibility;
mod check;
mod coverage;
mod game;
mod golden;
mod metric;
mod players;
mod report;
mod staircase;
mod steps;
#[cfg(test)]
mod tests;

use std::time::Instant;

use enth_music::ear::MAX_SEED;

use check::{Output, Plan};
use game::{EarGame, Shipped, shipped};
use report::{Gate, Ladders, Report, pct};

const USAGE: &str = "usage: enth-lesson-tool ear-check [options]
  --items N          items per level rendered and measured under sound/1 (default 24, fewer when there are many levels; 0 renders nothing)
  --bot-items N      items per level the players play without rendering (default 10000)
  --runs N           staircase runs per listener-and-start cell (default 300)
  --jobs N           threads (default: the cores, at most 4); the results do not depend on it
  --seed N           the run's seed (default 0x45434B5F7631)
  --strict           ladder criteria fail the run for every game (--ladders enforce)
  --ladders MODE     auto (the default: hard from two levels), enforce, report
  --only GAME        only this game (an id or part of a name); repeatable
  --update-golden    rewrite tools/lesson/src/ear/golden/items-v1.txt (refused under CI) and stop
Exit 0 when nothing hard failed, 1 otherwise; the report is on stdout.";

struct Options {
    plan: Plan,
    /// Whether `--items` was given (otherwise the sample is sized to the number of levels).
    items_given: bool,
    only: Vec<String>,
    update_golden: bool,
}

/// The measured items per level when `--items` is not given: 24, fewer when there are many levels so the run keeps
/// its time (a measured item costs about one second of one core): 288 in all, at least 4 a level.
fn default_items(cells: usize) -> usize {
    (288 / cells.max(1)).clamp(4, 24)
}

fn number(args: &mut std::slice::Iter<'_, String>, flag: &str) -> Result<u64, String> {
    let v = args
        .next()
        .ok_or_else(|| format!("{flag} needs a number"))?;
    match v.strip_prefix("0x") {
        Some(hex) => u64::from_str_radix(hex, 16),
        None => v.parse(),
    }
    .map_err(|_| format!("{flag}: {v:?} is not a number"))
}

fn parse(args: &[String]) -> Result<Option<Options>, String> {
    let mut plan = Plan::default();
    let (mut only, mut update_golden, mut items_given) = (Vec::new(), false, false);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--items" => {
                plan.audible_items =
                    usize::try_from(number(&mut it, a)?).map_err(|e| e.to_string())?;
                items_given = true;
            }
            "--bot-items" => {
                plan.bot_items = usize::try_from(number(&mut it, a)?).map_err(|e| e.to_string())?;
            }
            "--runs" => plan.runs = number(&mut it, a)?.max(1),
            "--jobs" => {
                plan.jobs = usize::try_from(number(&mut it, a)?)
                    .map_err(|e| e.to_string())?
                    .max(1);
            }
            "--seed" => {
                plan.seed = number(&mut it, a)?;
                if plan.seed > MAX_SEED {
                    return Err(format!("--seed: at most {MAX_SEED}"));
                }
            }
            "--strict" => plan.ladders = Ladders::Enforce,
            "--ladders" => {
                plan.ladders = match it.next().map(String::as_str) {
                    Some("auto") => Ladders::Auto,
                    Some("enforce") => Ladders::Enforce,
                    Some("report") => Ladders::Report,
                    other => {
                        return Err(format!("--ladders: auto, enforce or report, not {other:?}"));
                    }
                }
            }
            "--only" => only.push(it.next().ok_or("--only needs a game")?.to_lowercase()),
            "--update-golden" => update_golden = true,
            "-h" | "--help" => {
                println!("{USAGE}");
                return Ok(None);
            }
            other => return Err(format!("unknown option {other:?}\n{USAGE}")),
        }
    }
    Ok(Some(Options {
        plan,
        items_given,
        only,
        update_golden,
    }))
}

fn selected(game: Shipped, only: &[String]) -> bool {
    only.is_empty()
        || only
            .iter()
            .any(|o| o == &game.id().to_string() || game.name().to_lowercase().contains(o.as_str()))
}

/// The committed items digest's path, in the source tree (for `--update-golden`).
fn items_path() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/ear/golden/items-v1.txt")
}

#[allow(
    clippy::disallowed_methods,
    reason = "`--update-golden` writes the committed digest file; a tool, not library code"
)]
fn update_golden() -> Result<(), String> {
    if std::env::var_os("CI").is_some() {
        return Err("--update-golden is refused under CI: a golden changes on purpose, locally, and the commit message says why".to_owned());
    }
    let text = golden::items_file(&golden::cells())?;
    std::fs::write(items_path(), &text).map_err(|e| format!("{}: {e}", items_path().display()))?;
    println!(
        "wrote {} ({} lines)",
        items_path().display(),
        text.lines().filter(|l| l.starts_with("g=")).count()
    );
    Ok(())
}

fn section(title: &str) {
    println!("\n== {title}");
}

/// The golden checks.
fn golden_checks(plan: &Plan, report: &mut Report, only: &[String]) {
    section("golden");
    let found = golden::check_music_real(golden::MUSIC_GOLDEN);
    println!(
        "ear generator v{}: crates/enth-music/tests/golden/ear-v1.txt recomputed from its own seeds and counts: {} difference(s)",
        golden::generator(),
        found.len()
    );
    for f in found.iter().take(10) {
        println!("  {}: {}", f.cell, f.detail);
    }
    report.check(
        "golden-music",
        "ear generator v1",
        found.is_empty(),
        Gate::Hard,
        1,
        found.first().map_or_else(String::new, |f| {
            format!(
                "{} difference(s), first {}: {}",
                found.len(),
                f.cell,
                f.detail
            )
        }),
    );
    let cells: Vec<(Shipped, u8)> = golden::cells()
        .into_iter()
        .filter(|(g, _)| selected(*g, only))
        .collect();
    let digests = check::par_map(&cells, plan.jobs, |(g, l)| {
        golden::items_digest(g, *l).map(|d| (g.id(), *l, d))
    });
    let mut computed = Vec::new();
    for (d, (g, l)) in digests.into_iter().zip(&cells) {
        match d {
            Ok(x) => computed.push(x),
            Err(e) => report.check(
                "golden-items",
                format!("{} L{l}", g.name()),
                false,
                Gate::Hard,
                1,
                e,
            ),
        }
    }
    let mut found = golden::compare_items(golden::ITEMS_GOLDEN, &computed);
    if !only.is_empty() {
        // A partial run cannot judge completeness: only the lines it computed.
        found.retain(|f| {
            computed
                .iter()
                .any(|(g, l, _)| f.cell == format!("g={g:02} l={l:02}"))
                && !f.detail.contains("has no line")
        });
    }
    println!(
        "items digest: {} (game, level) cells x {} seeds x up to {} items: {} difference(s)",
        computed.len(),
        golden::ITEM_SEEDS,
        golden::ITEMS_PER_SEED,
        found.len()
    );
    for f in found.iter().take(10) {
        println!("  {}: {}", f.cell, f.detail);
    }
    report.check(
        "golden-items",
        "items digest",
        found.is_empty(),
        Gate::Hard,
        1,
        found.first().map_or_else(String::new, |f| {
            format!(
                "{} difference(s), first {}: {}",
                found.len(),
                f.cell,
                f.detail
            )
        }),
    );
}

fn print_cells(out: &Output) {
    section("levels: audibility, the ideal observer, the players");
    println!("{}", check::header());
    for c in &out.cells {
        println!("{}", check::row(c));
    }
    println!(
        "(closest = each item's closest pair, in JND, min / 10th percentile / median; need = 6 at level 1, else 2; unmeas = pairs sound/1 cannot hear; observer = the ideal observer's chance; random, deaf and bias max = chance +- 3 %)"
    );
    for c in &out.cells {
        if let Some(s) = &c.summary
            && !s.reasons.is_empty()
        {
            let why: Vec<String> = s
                .reasons
                .iter()
                .map(|(w, n)| format!("{n} x {w}"))
                .collect();
            println!(
                "  not measured, {} L{}: {}",
                c.name,
                c.level,
                why.join("; ")
            );
        }
        if let (Some(l), Some(o)) = (&c.loud_render, &c.observer) {
            println!(
                "  {} L{}: the observer through the judge {} (the sample is {} items); the loudest option as rendered {} against chance {}",
                c.name,
                c.level,
                pct(o.accuracy()),
                o.items,
                pct(l.accuracy()),
                pct(l.chance())
            );
        }
    }
}

fn print_ladders(out: &Output, steps: &[steps::StepLadder]) {
    section("staircases: simulated listeners over each ladder (40 trials)");
    for (ladder, eval) in &out.ladders {
        match eval {
            Some(e) => println!(
                "{}: {} [{}]",
                ladder.name,
                check::describe(e),
                if e.passes() { "pass" } else { "fail" }
            ),
            None => println!(
                "{}: {} level(s), fewer than {}: not simulated (G1 to G4 deliver ladders)",
                ladder.name,
                ladder.rows.len(),
                staircase::MIN_LEVELS
            ),
        }
    }
    for s in steps {
        let rows: Vec<String> = s
            .ladder
            .rows
            .iter()
            .map(|r| format!("{:.1}", r.jnd))
            .collect();
        println!(
            "{} ({}): JND per step 8..1 [{}]",
            s.ladder.name,
            s.control,
            rows.join(" ")
        );
        if let Some(e) = &s.eval {
            println!(
                "  {} [{}]",
                check::describe(e),
                if e.passes() { "pass" } else { "fail" }
            );
        }
    }
}

/// `ear-check`: GR4's mechanical half.
///
/// # Errors
/// A hard check failed (the report names it), or the arguments do not read.
pub fn run(args: &[String]) -> Result<(), String> {
    let Some(opts) = parse(args)? else {
        return Ok(());
    };
    if opts.update_golden {
        return update_golden();
    }
    let started = Instant::now();
    let mut plan = opts.plan.clone();
    let games: Vec<Shipped> = shipped()
        .into_iter()
        .filter(|g| selected(*g, &opts.only))
        .collect();
    if !opts.items_given {
        plan.audible_items = default_items(games.iter().map(|g| usize::from(g.levels())).sum());
    }
    let plan = &plan;
    println!(
        "ear-check: seed {:#x}, {} audible and {} bot items per level, {} staircase runs per cell, {} thread(s), ladders {:?}",
        plan.seed, plan.audible_items, plan.bot_items, plan.runs, plan.jobs, plan.ladders
    );
    let mut report = Report::default();
    let t = Instant::now();
    golden_checks(plan, &mut report, &opts.only);
    println!("({:.1} s)", t.elapsed().as_secs_f64());

    let refs: Vec<&dyn EarGame> = games.iter().map(|g| g as &dyn EarGame).collect();
    let t = Instant::now();
    let mut out = check::check_games(&refs, plan);
    print_cells(&out);
    println!("({:.1} s)", t.elapsed().as_secs_f64());

    let t = Instant::now();
    let mut step_ladders = Vec::new();
    if plan.audible_items > 0
        && games
            .iter()
            .any(|g| g.id() == enth_challenge::ear::params::EAR_CHECK)
    {
        let cache = metric::Calibrations::default();
        match steps::reference(plan, &cache) {
            Ok(ladders) => {
                let levels = games
                    .iter()
                    .find(|g| g.id() == enth_challenge::ear::params::EAR_CHECK)
                    .map_or(1, EarGame::levels);
                for s in &ladders {
                    if let Some(e) = &s.eval {
                        out.report.check(
                            "staircase",
                            s.ladder.name.clone(),
                            e.passes(),
                            Gate::Ladder,
                            levels,
                            check::describe(e),
                        );
                    }
                }
                step_ladders = ladders;
            }
            Err(e) => out
                .report
                .check("staircase", "Ear check steps", false, Gate::Hard, 1, e),
        }
    }
    print_ladders(&out, &step_ladders);
    println!("({:.1} s)", t.elapsed().as_secs_f64());

    section("coverage");
    println!(
        "Dial it in (game 2) is a Recreate round: no options, judged by sound/1; it is covered here as a source of parameters."
    );
    match coverage::basic_tier() {
        Ok(b) => {
            println!(
                "Basic-tier parameters heard on the base through the held phrase: {}; in a pool of game 1 or 2: {}; missing: {}",
                b.observable.len(),
                b.covered.len(),
                b.missing.len()
            );
            if !b.missing.is_empty() {
                println!("  missing: {}", b.missing.join(", "));
            }
            let levels = games
                .iter()
                .find(|g| g.id() == enth_challenge::ear::params::SPOT_THE_CHANGE)
                .map_or(1, EarGame::levels);
            out.report.check(
                "coverage",
                "Basic-tier parameters",
                b.missing.is_empty(),
                Gate::Ladder,
                levels,
                format!(
                    "{} of {} observable Basic-tier parameters are in game 1 or 2; missing {}",
                    b.covered.len(),
                    b.observable.len(),
                    b.missing.join(", ")
                ),
            );
        }
        Err(e) => out
            .report
            .check("coverage", "Basic-tier parameters", false, Gate::Hard, 1, e),
    }
    let bursts: usize = out.cells.iter().map(|c| c.skills.len()).sum();
    println!(
        "bursts: {bursts} (game, level, skill) combinations found five items each and moved their boxes (see the bursts checks)"
    );

    for c in out.report.checks.drain(..) {
        report.push(c);
    }
    section("result");
    print!("{}", report.summary(plan.ladders));
    println!("ear-check took {:.1} s", started.elapsed().as_secs_f64());
    let fails = report.failures(plan.ladders);
    if fails.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "ear-check: {} check(s) failed: {}",
            fails.len(),
            fails
                .iter()
                .map(|c| format!("{} [{}]", c.name, c.cell))
                .collect::<Vec<_>>()
                .join(", ")
        ))
    }
}
