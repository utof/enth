//! **Owner: lane ECK.** The checks of a set of games: each (game, level) is a **cell**, played by every player,
//! measured on a seeded sample, and judged against GR4's mechanical bullets (docs/ear.md); a game's cells make its
//! ladder, which the staircase simulations judge. The input is any [`EarGame`], so the tests plant a bad game of each
//! kind and see the run fail.
//!
//! Per cell:
//!
//! | Check | Gate | Over | Holds when |
//! | --- | --- | --- | --- |
//! | `structure` | hard | the bot items | every item draws, repeats exactly, names a Review skill, and (music) its counterfactual for the right answer is the question; the judge accepts every answer |
//! | `oracle` | hard | the bot items | the key, read through the real judge, is right 100 % (Tap back: the key's own taps) |
//! | `chance` | hard | the bot items | random and deaf (answers by the item's number) are at chance +- 3 % (five standard errors on a run shorter than 10,000 items) |
//! | `bias` | hard | the bot items | every bias bot (first, last, longest, loudest, highest, densest) is at chance +- 3 % (same allowance) |
//! | `bias-render` | note | the measured sample | the loudest option **as rendered** is at chance, with a binomial allowance (+- 3 % + 3 sigma); playback level-matches clips (docs/ear.md), so this is a finding for the lane that owns the render, never a failure |
//! | `audibility` | ladder | the measured sample | every option pair is at least 2 JND apart, 6 at level 1; not measured pairs are listed apart |
//! | `observer` | ladder | the measured sample | at the top level the ideal observer scores at least 95 % |
//!
//! and per game with four levels or more, `staircase` (ladder): the F1e judge's criteria over the game's measured rows.

use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use enth_dsp::util::rng::{Rng, derive, fnv1a64};
use enth_learn::review::Skill;
use enth_music::ear::MAX_SEED;

use super::audibility::{self, Measured, Summary};
use super::coverage;
use super::game::{EarGame, Item};
use super::golden;
use super::metric::Calibrations;
use super::players::{self, Player, Tally};
use super::report::{Gate, Ladders, Report, jnd, pct};
use super::staircase::{self, Evaluation, Ladder, Row};

/// What to run.
#[derive(Clone, Debug)]
pub struct Plan {
    /// The run's seed: every draw derives from it.
    pub seed: u64,
    /// Items per cell that are rendered and measured (0: no rendering).
    pub audible_items: usize,
    /// Items per cell the players play without rendering.
    pub bot_items: usize,
    /// Staircase runs per cell.
    pub runs: u64,
    /// Threads.
    pub jobs: usize,
    /// How ladder checks are enforced.
    pub ladders: Ladders,
}

impl Default for Plan {
    fn default() -> Self {
        Plan {
            seed: 0x0000_4543_4B5F_7631 & MAX_SEED,
            audible_items: 24,
            bot_items: 10_000,
            runs: 300,
            jobs: std::thread::available_parallelism()
                .map_or(2, usize::from)
                .min(4),
            ladders: Ladders::Auto,
        }
    }
}

/// The tolerance of a bot at chance: +- 3 %.
pub const CHANCE_BAND: f64 = 0.03;

/// What a bot at chance may drift from it: the band, or five standard errors where the run is too short for the band
/// to be five (it is 6 at the 10,000 items GR4 asks for, so there the band rules and a short run cannot trip a fair
/// game by luck).
#[must_use]
pub fn allowance(t: &Tally) -> f64 {
    CHANCE_BAND.max(5.0 * t.sigma())
}

/// The ideal observer's floor at the top level.
pub const OBSERVER_TOP: f64 = 0.95;

/// What one cell found.
#[derive(Clone, Debug)]
pub struct CellResult {
    /// The game's id.
    pub game: u8,
    /// The game's name.
    pub name: String,
    /// The level.
    pub level: u8,
    /// How many levels the game has.
    pub levels: u8,
    /// Items the players played.
    pub played: u64,
    /// Problems with structure (capped), and how many there were.
    pub problems: (Vec<String>, usize),
    /// The tallies of the bot items.
    pub bots: Vec<Tally>,
    /// The audibility summary of the measured sample (`None`: nothing measured).
    pub summary: Option<Summary>,
    /// The ideal observer through the judge over the sample.
    pub observer: Option<Tally>,
    /// The loudest-as-rendered bot over the sample.
    pub loud_render: Option<Tally>,
    /// The row this cell is in its game's ladder (`None` when nothing was measured).
    pub row: Option<Row>,
    /// The mean number of options.
    pub options: f64,
    /// The Review skills the cell's items exercise, each with the first seeds an item of it was drawn from (a
    /// parameter game's skill depends on the seed).
    pub skills: Vec<(String, Vec<u64>)>,
    /// Problems with bursts on them (empty: bursts of five are found for each and move their boxes).
    pub bursts: Vec<String>,
}

/// The seed of the `k`-th seed of a cell's bot items.
fn bot_seed(plan: &Plan, game: u8, level: u8, k: u64) -> u64 {
    derive(
        derive(
            plan.seed,
            fnv1a64(format!("ear-check/bots/{game}/{level}").as_bytes()),
        ),
        k,
    ) & MAX_SEED
}

/// Order-preserving parallel map over `tasks` on `jobs` threads.
pub fn par_map<T: Sync, R: Send>(tasks: &[T], jobs: usize, f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let next = AtomicUsize::new(0);
    let out: Mutex<Vec<(usize, R)>> = Mutex::new(Vec::with_capacity(tasks.len()));
    std::thread::scope(|s| {
        for _ in 0..jobs.clamp(1, tasks.len().max(1)) {
            s.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(task) = tasks.get(i) else { break };
                    let r = f(task);
                    if let Ok(mut v) = out.lock() {
                        v.push((i, r));
                    }
                }
            });
        }
    });
    let mut v = out.into_inner().unwrap_or_default();
    v.sort_by_key(|(i, _)| *i);
    v.into_iter().map(|(_, r)| r).collect()
}

/// Items per chunk of a cell's bot items: a chunk is one task with its own draws, so the results do not depend on how
/// many threads run them.
pub const CHUNK: usize = 2500;

/// What one chunk of a cell's bot items found.
struct Chunk {
    tallies: Vec<Tally>,
    problems: Vec<String>,
    n_problems: usize,
    played: u64,
    options_sum: f64,
    skills: Vec<(String, Vec<u64>)>,
}

impl Chunk {
    fn problem(&mut self, text: String) {
        self.n_problems += 1;
        if self.problems.len() < 3 {
            self.problems.push(text);
        }
    }
}

/// The items per seed of a cell's bot items.
fn per_seed(game: &dyn EarGame) -> usize {
    game.items_per_seed().min(250) as usize
}

/// Item `k` of a cell's seeded sequence (the bot items, and the measured sample is the first of them).
fn nth_item(game: &dyn EarGame, level: u8, plan: &Plan, k: usize) -> Result<Item, String> {
    let per = per_seed(game);
    game.item(
        level,
        bot_seed(plan, game.id(), level, (k / per) as u64),
        (k % per) as u32,
    )
}

/// Play the players over chunk `chunk` of a cell's bot items.
fn bot_chunk(game: &dyn EarGame, level: u8, plan: &Plan, chunk: usize) -> Chunk {
    let mut c = Chunk {
        tallies: players::tallies(),
        problems: Vec::new(),
        n_problems: 0,
        played: 0,
        options_sum: 0.0,
        skills: Vec::new(),
    };
    let mut rng = Rng::new(derive(
        plan.seed,
        fnv1a64(format!("ear-check/play/{}/{level}/{chunk}", game.id()).as_bytes()),
    ));
    for k in chunk * CHUNK..((chunk + 1) * CHUNK).min(plan.bot_items) {
        let item = match nth_item(game, level, plan, k) {
            Ok(i) => i,
            Err(e) => {
                c.problem(e);
                continue;
            }
        };
        if let Some(s) = &item.skill {
            if Skill::parse(s).is_none() {
                c.problem(format!(
                    "item {} (seed {}): {s:?} is not a Review skill key",
                    item.index, item.seed
                ));
            }
            match c.skills.iter_mut().find(|(k, _)| k == s) {
                Some((_, seeds)) => {
                    if seeds.len() < 3 && !seeds.contains(&item.seed) {
                        seeds.push(item.seed);
                    }
                }
                None => c.skills.push((s.clone(), vec![item.seed])),
            }
        }
        if item.options == 0 {
            if let Err(e) = players::tap_oracle(&item) {
                c.problem(e);
            }
        } else if let Err(e) = players::play_item(&item, &mut rng, &mut c.tallies) {
            c.problem(e);
        }
        c.played += 1;
        c.options_sum += item.options as f64;
        // The first few items are drawn a second time, and must be the same.
        if k < 16 {
            match nth_item(game, level, plan, k) {
                Ok(again) if golden::item_bytes(&again) == golden::item_bytes(&item) => {}
                Ok(_) => c.problem(format!(
                    "item {} (seed {}) differs the second time it is drawn",
                    item.index, item.seed
                )),
                Err(e) => c.problem(e),
            }
        }
    }
    c
}

/// Measure item `k` of a cell's sequence (`None` for an item with no options).
fn measure_one(
    game: &dyn EarGame,
    level: u8,
    plan: &Plan,
    k: usize,
    cache: &Calibrations,
) -> Result<Option<(Item, Measured)>, String> {
    let item = nth_item(game, level, plan, k)?;
    if item.options == 0 {
        return Ok(None);
    }
    let m = audibility::measure(&item, cache)
        .map_err(|e| format!("item {} (seed {}): {e}", item.index, item.seed))?;
    Ok(Some((item, m)))
}

/// A cell, put together from its chunks and its measured sample.
fn assemble(
    game: &dyn EarGame,
    level: u8,
    plan: &Plan,
    chunks: Vec<Chunk>,
    sample: Vec<Result<Option<(Item, Measured)>, String>>,
) -> CellResult {
    let mut res = CellResult {
        game: game.id(),
        name: game.name(),
        level,
        levels: game.levels(),
        played: 0,
        problems: (Vec::new(), 0),
        bots: players::tallies(),
        summary: None,
        observer: None,
        loud_render: None,
        row: None,
        options: 0.0,
        skills: Vec::new(),
        bursts: Vec::new(),
    };
    let mut options_sum = 0.0;
    let problem = |res: &mut CellResult, text: String| {
        res.problems.1 += 1;
        if res.problems.0.len() < 3 {
            res.problems.0.push(text);
        }
    };
    for c in chunks {
        for (mine, theirs) in res.bots.iter_mut().zip(&c.tallies) {
            mine.right += theirs.right;
            mine.items += theirs.items;
            mine.chance += theirs.chance;
        }
        res.problems.1 += c.n_problems;
        for p in c.problems {
            if res.problems.0.len() < 3 {
                res.problems.0.push(p);
            }
        }
        res.played += c.played;
        options_sum += c.options_sum;
        for (skill, seeds) in c.skills {
            match res.skills.iter_mut().find(|(k, _)| *k == skill) {
                Some((_, mine)) => {
                    for s in seeds {
                        if mine.len() < 3 && !mine.contains(&s) {
                            mine.push(s);
                        }
                    }
                }
                None => res.skills.push((skill, seeds)),
            }
        }
    }
    if res.played > 0 {
        res.options = options_sum / res.played as f64;
    }
    res.skills.sort();
    if game.real() {
        res.bursts = coverage::bursts(game.id(), level, &res.skills);
    }
    let mut measured: Vec<(Item, Measured)> = Vec::new();
    for r in sample {
        match r {
            Ok(Some(x)) => measured.push(x),
            Ok(None) => {}
            Err(e) => problem(&mut res, e),
        }
    }
    if let Some((first, _)) = measured.first() {
        let layout = first.layout;
        let ms: Vec<Measured> = measured.iter().map(|(_, m)| m.clone()).collect();
        let summary = audibility::summarise(level, layout, &ms);
        let mean_options =
            measured.iter().map(|(i, _)| i.options as f64).sum::<f64>() / measured.len() as f64;
        res.row = summary.median.map(|jnd| Row {
            jnd,
            options: mean_options.round().max(2.0) as usize,
        });
        let mut orng = Rng::new(derive(
            plan.seed,
            fnv1a64(format!("ear-check/observer/{}/{level}", game.id()).as_bytes()),
        ));
        match players::play_observer(&measured, &mut orng) {
            Ok(t) => res.observer = Some(t),
            Err(e) => problem(&mut res, e),
        }
        match players::play_rendered_loudest(&measured, &mut orng) {
            Ok(t) => res.loud_render = Some(t),
            Err(e) => problem(&mut res, e),
        }
        res.summary = Some(summary);
    }
    res
}

/// One game's ladder from its cells' rows, in level order (`None` when a level has no measured row).
#[must_use]
pub fn ladder_of(name: &str, cells: &[&CellResult]) -> Option<Ladder> {
    let rows: Option<Vec<Row>> = cells.iter().map(|c| c.row).collect();
    Some(Ladder {
        name: name.to_owned(),
        rows: rows?,
    })
}

/// What a whole run found.
#[derive(Clone, Debug)]
pub struct Output {
    /// Every cell.
    pub cells: Vec<CellResult>,
    /// Each game's ladder and what the staircase simulations found (`None`: fewer than four levels).
    pub ladders: Vec<(Ladder, Option<Evaluation>)>,
    /// The checks.
    pub report: Report,
}

/// One unit of work.
enum Task {
    /// Chunk `.1` of cell `.0`'s bot items.
    Bots(usize, usize),
    /// Item `.1` of cell `.0`'s measured sample.
    Measure(usize, usize),
}

/// One measured item (`None`: the game has no such item), or why it could not be measured.
type Sample = Result<Option<(Item, Measured)>, String>;

/// What a task found.
enum Done {
    Bots(Chunk),
    Measured(Sample),
}

/// Run every cell of `games`, judge them, and simulate their ladders.
#[must_use]
pub fn check_games(games: &[&dyn EarGame], plan: &Plan) -> Output {
    let cache = Calibrations::default();
    let cell_ids: Vec<(usize, u8)> = games
        .iter()
        .enumerate()
        .flat_map(|(g, game)| (1..=game.levels()).map(move |l| (g, l)))
        .collect();
    // The bot chunks first (the longest tasks start early), then the measured items.
    let mut tasks = Vec::new();
    for c in 0..cell_ids.len() {
        for ch in 0..plan.bot_items.div_ceil(CHUNK) {
            tasks.push(Task::Bots(c, ch));
        }
    }
    for c in 0..cell_ids.len() {
        for k in 0..plan.audible_items {
            tasks.push(Task::Measure(c, k));
        }
    }
    let done = par_map(&tasks, plan.jobs, |t| match t {
        Task::Bots(c, ch) => {
            let (g, l) = cell_ids[*c];
            Done::Bots(bot_chunk(games[g], l, plan, *ch))
        }
        Task::Measure(c, k) => {
            let (g, l) = cell_ids[*c];
            Done::Measured(measure_one(games[g], l, plan, *k, &cache))
        }
    });
    let mut chunks: Vec<Vec<Chunk>> = cell_ids.iter().map(|_| Vec::new()).collect();
    let mut samples: Vec<Vec<Sample>> = cell_ids.iter().map(|_| Vec::new()).collect();
    for (t, d) in tasks.iter().zip(done) {
        match (t, d) {
            (Task::Bots(c, _), Done::Bots(x)) => chunks[*c].push(x),
            (Task::Measure(c, _), Done::Measured(x)) => samples[*c].push(x),
            _ => {}
        }
    }
    let cells: Vec<CellResult> = cell_ids
        .iter()
        .zip(chunks.into_iter().zip(samples))
        .map(|((g, l), (ch, sm))| assemble(games[*g], *l, plan, ch, sm))
        .collect();
    let mut report = Report::default();
    for c in &cells {
        judge_cell(c, &mut report);
    }
    let mut ladders = Vec::new();
    for game in games {
        let mine: Vec<&CellResult> = cells.iter().filter(|c| c.game == game.id()).collect();
        let Some(ladder) = ladder_of(&game.name(), &mine) else {
            continue;
        };
        let eval = staircase::evaluate(&ladder, plan.runs, derive(plan.seed, u64::from(game.id())));
        if let Some(e) = &eval {
            judge_ladder(&ladder, e, game.levels(), &mut report);
        }
        ladders.push((ladder, eval));
    }
    Output {
        cells,
        ladders,
        report,
    }
}

fn label(c: &CellResult) -> String {
    format!("{} L{}", c.name, c.level)
}

/// Turn a cell's findings into checks.
fn judge_cell(c: &CellResult, r: &mut Report) {
    let cell = label(c);
    let by = |name: &str| c.bots.iter().find(|t| t.name == name);
    r.check(
        "structure",
        &cell,
        c.problems.1 == 0 && c.played > 0,
        Gate::Hard,
        c.levels,
        if c.played == 0 {
            "no item could be drawn".to_owned()
        } else if c.problems.1 == 0 {
            format!("{} items draw, repeat, name a skill and judge", c.played)
        } else {
            format!(
                "{} problems, first: {}",
                c.problems.1,
                c.problems.0.join("; ")
            )
        },
    );
    if c.skills.len() + c.bursts.len() > 0 {
        r.check(
            "bursts",
            &cell,
            c.bursts.is_empty(),
            Gate::Hard,
            c.levels,
            if c.bursts.is_empty() {
                format!(
                    "a burst of five is found on each of {} skills and moves its box",
                    c.skills.len()
                )
            } else {
                format!("{} problems, first: {}", c.bursts.len(), c.bursts[0])
            },
        );
    }
    if let Some(o) = by("oracle")
        && o.items > 0
    {
        r.check(
            "oracle",
            &cell,
            o.right == o.items,
            Gate::Hard,
            c.levels,
            format!(
                "the key through the judge: {} of {} right",
                o.right, o.items
            ),
        );
    }
    for name in ["random", "deaf"] {
        if let Some(t) = by(name)
            && t.items > 0
        {
            r.check(
                "chance",
                &cell,
                t.edge().abs() <= allowance(t),
                Gate::Hard,
                c.levels,
                format!(
                    "{name} {} against chance {} over {} items",
                    pct(t.accuracy()),
                    pct(t.chance()),
                    t.items
                ),
            );
        }
    }
    let bots: Vec<&Tally> = Player::ALL
        .iter()
        .filter(|p| p.is_bias_bot())
        .filter_map(|p| by(p.name()))
        .filter(|t| t.items > 0)
        .collect();
    if let Some(worst) = bots
        .iter()
        .max_by(|a, b| (a.edge().abs() - allowance(a)).total_cmp(&(b.edge().abs() - allowance(b))))
    {
        r.check(
            "bias",
            &cell,
            worst.edge().abs() <= allowance(worst),
            Gate::Hard,
            c.levels,
            format!(
                "worst bot {}: {} against chance {} over {} items",
                worst.name,
                pct(worst.accuracy()),
                pct(worst.chance()),
                worst.items
            ),
        );
    }
    if let Some(t) = &c.loud_render
        && t.items >= 8
    {
        let allow = CHANCE_BAND + 3.0 * t.sigma();
        r.check(
            "bias-render",
            &cell,
            t.edge() <= allow,
            Gate::Note,
            c.levels,
            format!(
                "the loudest option as rendered {} against chance {} over {} items (allowance {})",
                pct(t.accuracy()),
                pct(t.chance()),
                t.items,
                pct(allow)
            ),
        );
    }
    if let Some(s) = &c.summary {
        let (un, all) = s.unmeasured;
        r.check(
            "audibility",
            &cell,
            s.passes(),
            Gate::Ladder,
            c.levels,
            format!(
                "closest pair min {} p10 {} median {} JND against {:.0} needed; {} of {} items short; {un} of {all} pairs not measured",
                jnd(s.min),
                jnd(s.p10),
                jnd(s.median),
                s.need,
                s.short,
                s.items
            ),
        );
        if c.level == c.levels {
            r.check(
                "observer",
                &cell,
                s.observer >= OBSERVER_TOP,
                Gate::Ladder,
                c.levels,
                format!(
                    "the ideal observer scores {} over {} items, {} needed",
                    pct(s.observer),
                    s.items,
                    pct(OBSERVER_TOP)
                ),
            );
        }
    }
}

/// Turn a ladder's simulations into checks.
fn judge_ladder(ladder: &Ladder, e: &Evaluation, levels: u8, r: &mut Report) {
    let cell = format!("{} ladder", ladder.name);
    r.check(
        "staircase",
        &cell,
        e.passes(),
        Gate::Ladder,
        levels,
        describe(e),
    );
}

/// A staircase evaluation in a line.
#[must_use]
pub fn describe(e: &Evaluation) -> String {
    let off: Vec<String> = e.off.iter().map(|(g, why)| format!("{g}: {why}")).collect();
    format!(
        "{} levels, {:.2} JND per level ({}), {}; {} of {} acuities have a threshold on the ladder{}; within one level {} (worst cell {} {}), within two {}; success at threshold {} to {}; {} of runs with no estimate",
        e.levels,
        e.step,
        if e.flat { "flat" } else { "steep" },
        if e.monotone {
            "monotone"
        } else {
            "NOT monotone"
        },
        e.in_range,
        super::staircase::GAINS.len(),
        if off.is_empty() {
            String::new()
        } else {
            format!(" (off: {})", off.join(", "))
        },
        pct(e.within_one),
        pct(e.worst_one.0),
        e.worst_one.1,
        pct(e.within_two),
        pct(e.success.0),
        pct(e.success.1),
        pct(e.no_estimate),
    )
}

/// A cell as a line of the table.
#[must_use]
pub fn row(c: &CellResult) -> String {
    let by = |name: &str| {
        c.bots
            .iter()
            .find(|t| t.name == name)
            .map_or_else(|| "-".to_owned(), |t| pct(t.accuracy()))
    };
    let bias = c
        .bots
        .iter()
        .filter(|t| {
            Player::ALL
                .iter()
                .any(|p| p.name() == t.name && p.is_bias_bot())
        })
        .filter(|t| t.items > 0)
        .map(Tally::edge)
        .fold(0.0_f64, |a, e| if e.abs() > a.abs() { e } else { a });
    let (closest, need, unmeasured) = c.summary.as_ref().map_or_else(
        || ("-".to_owned(), "-".to_owned(), "-".to_owned()),
        |s| {
            (
                format!("{}/{}/{}", jnd(s.min), jnd(s.p10), jnd(s.median)),
                format!("{:.0}", s.need),
                format!("{}/{}", s.unmeasured.0, s.unmeasured.1),
            )
        },
    );
    format!(
        "{:<20} {:>2} {:>5.1} {:>6} {:>16} {:>4} {:>7} {:>8} {:>7} {:>7} {:>+7.1}%",
        c.name,
        c.level,
        c.options,
        c.played,
        closest,
        need,
        unmeasured,
        c.summary
            .as_ref()
            .map_or_else(|| "-".to_owned(), |s| pct(s.observer)),
        by("random"),
        by("deaf"),
        100.0 * bias
    )
}

/// The header of [`row`].
#[must_use]
pub fn header() -> String {
    format!(
        "{:<20} {:>2} {:>5} {:>6} {:>16} {:>4} {:>7} {:>8} {:>7} {:>7} {:>8}",
        "game",
        "L",
        "opts",
        "items",
        "closest min/p10/med",
        "need",
        "unmeas",
        "observer",
        "random",
        "deaf",
        "bias max"
    )
}
