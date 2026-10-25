//! **Owner: lane CL.** `enth match`: fit a patch to a sound, a patch or a challenge's target (`enth_solver`'s search,
//! lane MT: a heuristic start from the target's descriptors, then CMA-ES with restarts inside a render budget). The
//! file is `matching.rs` because `match` is a keyword.
//!
//! ```text
//! enth match --target t.wav    --start p.patch.json --free filter1,env1.release --out p2.patch.json
//! enth match --target t.patch.json                  --level 2 --stop-at 0.005
//! enth match --target 1080-1000-0001-AB             --free drawn --budget 600
//! ```
//!
//! - **The target** is a WAV (its performance comes from `enth render`'s chunk or `--perf`), a patch (played with
//!   `--perf`, default C3 held 1.5 s) or a challenge code (played through its phrase, at the scoring rate and seed,
//!   from the challenge's start patch). The distance is metric v1 (`logmel-l1/1`); for a code the game's own score
//!   of the best patch is printed too.
//! - **The free set** is what may move: addresses and module ids (`--free filter1,env1.release`), `observable`
//!   (every parameter that can be heard, the default), `drawn` (a code's randomized parameters) or `--level N` (the
//!   observable parameters of the Recreate categories up to level N). Structure is never searched: modules and routes
//!   stay as the start has them.
//! - **Deterministic.** The search is a pure function of its inputs, its seed and the budget; the batches go through
//!   one rayon pool of metric workers and come back by index, so the output is identical for any `--workers`. Only
//!   `--max-seconds` (a wall-clock cap, off by default) can make it depend on the machine.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};
use std::time::Instant;

use enth_challenge::random::{self, Mask, unlock_level};
use enth_challenge::{Category, Phrase, PreparedRound, score_round};
use enth_patch::catalogue::{Builtin, ModuleKind};
use enth_patch::{Op, ParamAddr, Patch, PatchError, Target, wire};
use enth_render::{DEFAULT_SAMPLE_RATE, DEFAULT_SEED, RenderSpec};
use enth_solver::heuristic::Heuristic;
use enth_solver::search::{
    FreeSet, MetricTarget, MetricWorker, Objective, SearchCfg, SearchResult, Stage, StopCause,
    TraceStep, search_with,
};
use rayon::prelude::*;

use super::common::{as_array, changes};
use super::game::{self, Resolved};
use super::{Reply, Res};
use crate::error::{self, CliError, Ctx};
use crate::out::round4;
use crate::perf::{Perf, PerfArgs};
use crate::schema::{
    FreeOut, MatchOut, MatchSettingsOut, MatchTargetOut, TraceOut, TraceStepOut, ids,
};
use crate::{aliases, patchio, templates};

/// The default render budget (`SearchCfg::default()`'s).
const DEFAULT_BUDGET: usize = 1500;
/// The most points `trace.steps` shows without `--full`.
const TRACE_POINTS: usize = 12;
/// The highest Recreate level.
const MAX_LEVEL: u8 = 5;

/// `enth match`'s arguments.
#[derive(Clone, Debug, PartialEq, clap::Args)]
pub struct Args {
    /// What to match: a WAV, a patch file, or a challenge code.
    #[arg(long, value_name = "WAV|PATCH|CODE")]
    pub target: String,
    /// The patch to search from (default: the challenge's start for a code, the `init` template otherwise).
    #[arg(long, value_name = "PATCH")]
    pub start: Option<PathBuf>,
    /// The parameters that may move, comma-separated: addresses (`filter1.cutoff`), module ids (`filter1`),
    /// `observable` (every audible parameter; the default) or `drawn` (a code's randomized ones).
    #[arg(
        long,
        value_name = "LIST",
        value_delimiter = ',',
        conflicts_with = "level"
    )]
    pub free: Vec<String>,
    /// Free the observable parameters of Recreate levels 1 to N instead: 1 oscillators and noise, 2 the filter,
    /// 3 envelopes, 4 LFOs, 5 effects and the voice.
    #[arg(long, value_name = "N", conflicts_with = "free")]
    pub level: Option<u8>,
    /// The most renders, the probe included.
    #[arg(long, value_name = "N", default_value_t = DEFAULT_BUDGET)]
    pub budget: usize,
    /// Stop as soon as the distance is at or below this (metric v1: 0.005 is a close match).
    #[arg(long, value_name = "D")]
    pub stop_at: Option<f64>,
    /// Worker threads (default: every core). The output is the same for any number.
    #[arg(long, value_name = "N")]
    pub workers: Option<usize>,
    /// The search's random seed. Another seed explores differently; the same one repeats the search exactly.
    #[arg(long, value_name = "SEED", default_value_t = 1)]
    pub seed: u64,
    /// A wall-clock cap in seconds. Off by default: with a cap the result depends on the machine's speed.
    #[arg(long, value_name = "SECONDS")]
    pub max_seconds: Option<f64>,
    /// Write the best patch here.
    #[arg(long, value_name = "PATCH")]
    pub out: Option<PathBuf>,
    /// Show every batch of the search, not the points where it improved.
    #[arg(long)]
    pub full: bool,
    #[command(flatten)]
    pub perf: PerfArgs,
}

/// A target loaded: what the metric compares against, and what the search starts from.
struct Loaded {
    kind: &'static str,
    source: String,
    fingerprint: Option<String>,
    round: Option<PreparedRound>,
    metric: MetricTarget,
    start: Patch,
    start_label: String,
    /// The phrase the observability mask is read through (the target's own for a code, the held note otherwise).
    phrase: &'static Phrase,
    notes: Vec<String>,
}

/// Run `enth match`.
pub fn run(args: &Args, stdin: &str) -> Res {
    check_settings(args)?;
    let mut loaded = load_target(args, stdin)?;
    let mut notes = std::mem::take(&mut loaded.notes);
    let (free, how) = free_set(args, &loaded)?;

    let heuristic = loaded
        .metric
        .descriptors()
        .map(|d| Heuristic::new(d, loaded.metric.performance()));
    let cfg = SearchCfg {
        seed: args.seed,
        budget: args.budget,
        stop_at: args.stop_at,
        ..SearchCfg::default()
    };
    let t0 = Instant::now();
    let cap = args.max_seconds;
    let found = game::with_workers(args.workers, || {
        let mut objective = PoolObjective::new(&loaded.metric);
        search_with(
            &loaded.start,
            &free,
            heuristic.as_ref(),
            &mut objective,
            &cfg,
            &mut |_| cap.is_none_or(|c| t0.elapsed().as_secs_f64() < c),
        )
    })?
    .map_err(|e| game::from_search(&e))?;
    if found.stop == StopCause::Cancelled {
        notes.push(format!(
            "stopped at the --max-seconds cap after {} renders; the result depends on this machine's speed",
            found.renders
        ));
    }

    let (score, start_score) = match &loaded.round {
        Some(round) => {
            let best = score_round(round, &found.best).map_err(|e| game::from_score(&e))?;
            let before = score_round(round, &loaded.start).map_err(|e| game::from_score(&e))?;
            (Some(game::score_out(&best)), Some(game::score_out(&before)))
        }
        None => (None, None),
    };

    // The changes, as exact ops: the free parameters that moved, in the free set's order.
    let ops: Vec<Op> = free
        .addrs()
        .filter_map(|addr| {
            let (from, to) = (loaded.start.value(addr)?, found.best.value(addr)?);
            (from != to).then(|| Op::SetParam {
                addr: addr.clone(),
                value: to,
            })
        })
        .collect();
    let wrote = match &args.out {
        Some(path) => {
            patchio::write(path, &found.best)?;
            Some(patchio::show(path))
        }
        None => None,
    };

    let passed = score.as_ref().map(|s| s.passed);
    let out = MatchOut {
        schema: ids::MATCH.to_owned(),
        target: MatchTargetOut {
            kind: loaded.kind.to_owned(),
            source: loaded.source.clone(),
            fingerprint: loaded.fingerprint.clone(),
        },
        start: loaded.start_label.clone(),
        free: FreeOut {
            how,
            count: free.len(),
            continuous: free.continuous().len(),
            categorical: free.categorical().len(),
            addrs: free.addrs().map(ToString::to_string).collect(),
        },
        distance: found.distance,
        start_distance: found
            .start_distance
            .is_finite()
            .then_some(found.start_distance),
        heuristic_distance: found.heuristic_distance,
        score,
        start_score,
        renders: found.renders,
        runs: found.runs,
        stop: stop_name(found.stop).to_owned(),
        settings: MatchSettingsOut {
            budget: args.budget,
            seed: args.seed,
            stop_at: args.stop_at,
            max_seconds: args.max_seconds,
        },
        changes: changes(&loaded.start, &ops),
        ops: as_array(wire::to_json(&loaded.start, &ops)),
        wrote,
        trace: trace_out(&found.trace, args.full),
        hint: next_step(args, &found, passed),
    };
    let mut reply = Reply::new(&out).text(summary(&out));
    for n in &notes {
        reply = reply.note(n);
    }
    Ok(reply)
}

// ---------------------------------------------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------------------------------------------

fn check_settings(args: &Args) -> Result<(), CliError> {
    game::check_workers(args.workers)?;
    if args.budget == 0 {
        return Err(error::usage(
            "bad_budget",
            "--budget 0: a search needs at least one render",
            "the default is 1500; a close match to a level 1 or 2 target takes 20 to 600",
        ));
    }
    if let Some(d) = args.stop_at
        && !(d.is_finite() && d >= 0.0)
    {
        return Err(error::usage(
            "bad_stop_at",
            format!("--stop-at {d}: a distance is zero or more"),
            "metric v1 distances run from 0 (identical) to over 1 (a different sound); 0.005 is a close match",
        ));
    }
    if let Some(s) = args.max_seconds
        && !(s.is_finite() && s > 0.0)
    {
        return Err(error::usage(
            "bad_seconds",
            format!("--max-seconds {s}: the cap is a positive number of seconds"),
            "leave it out for no cap; without one the result is the same on any machine",
        ));
    }
    if let Some(n) = args.level
        && !(1..=MAX_LEVEL).contains(&n)
    {
        return Err(error::usage(
            "bad_level",
            format!("--level {n}: the levels run 1 to {MAX_LEVEL}"),
            "1 oscillators and noise, 2 the filter, 3 envelopes, 4 LFOs, 5 effects and the voice",
        ));
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------------------------
// The target
// ---------------------------------------------------------------------------------------------------------------

/// Whether `--target` names a file: it exists, or it looks like a path (a separator or an extension). Anything else
/// is read as a challenge code, so a mistyped code gets the code's own error.
fn is_file(text: &str) -> bool {
    let p = Path::new(text);
    p.exists() || text.contains(['/', '\\']) || p.extension().is_some()
}

fn load_target(args: &Args, stdin: &str) -> Result<Loaded, CliError> {
    let text = args.target.trim();
    let flag = Perf::from_args(&args.perf, stdin)?;
    let mut loaded = if is_file(text) {
        let path = Path::new(text);
        let bytes = patchio::read_bytes(path)?;
        if patchio::is_wav(&bytes) {
            wav_target(&bytes, text, flag)?
        } else {
            patch_target(&bytes, text, flag)?
        }
    } else {
        if flag.is_some() {
            return Err(error::usage(
                "perf_fixed",
                "a challenge code fixes the performance: its phrase is what the target and the search play",
                "drop --perf/--note/--vel/--hold/--dur, or match a patch or WAV target with them",
            ));
        }
        code_target(text)?
    };
    if let Some(path) = &args.start {
        let (patch, notes) = patchio::load(path)?;
        loaded.start = patch;
        loaded.start_label = patchio::show(path);
        loaded.notes.extend(notes);
    }
    Ok(loaded)
}

fn code_target(text: &str) -> Result<Loaded, CliError> {
    let Resolved { round, code, notes } = game::from_code(text).map_err(|mut e| {
        e.0.hint = format!(
            "{}; --target takes a WAV or patch file, or a challenge code",
            e.0.hint
        );
        e
    })?;
    let perf = round.phrase().performance();
    let spec = RenderSpec::new(
        enth_challenge::model::SCORING_SAMPLE_RATE,
        enth_challenge::model::SCORING_SEED,
    );
    let metric =
        MetricTarget::from_patch(round.target(), &perf, &spec).map_err(|e| error::from_eval(&e))?;
    Ok(Loaded {
        kind: "code",
        source: code.unwrap_or_else(|| text.to_owned()),
        fingerprint: Some(round.fingerprint().to_hex()),
        start: round.start().clone(),
        start_label: "challenge start".to_owned(),
        phrase: round.phrase(),
        metric,
        round: Some(round),
        notes,
    })
}

fn patch_target(bytes: &[u8], name: &str, flag: Option<Perf>) -> Result<Loaded, CliError> {
    let (patch, notes) = patchio::parse(bytes, name)?;
    let perf = flag.unwrap_or_else(Perf::default_perf).to_render();
    let spec = RenderSpec::new(DEFAULT_SAMPLE_RATE, DEFAULT_SEED);
    let metric =
        MetricTarget::from_patch(&patch, &perf, &spec).map_err(|e| error::from_eval(&e))?;
    Ok(Loaded {
        kind: "patch",
        source: name.to_owned(),
        fingerprint: None,
        round: None,
        metric,
        start: game::init_patch(),
        start_label: "init".to_owned(),
        phrase: game::held(),
        notes,
    })
}

fn wav_target(bytes: &[u8], name: &str, flag: Option<Perf>) -> Result<Loaded, CliError> {
    let audio = game::load_audio(bytes, name, flag, "a match")?;
    let spec = RenderSpec::new(audio.sample_rate, audio.seed);
    let metric = MetricTarget::from_audio(&audio.mono, &audio.perf.to_render(), &spec);
    Ok(Loaded {
        kind: "wav",
        source: name.to_owned(),
        fingerprint: None,
        round: None,
        metric,
        start: game::init_patch(),
        start_label: "init".to_owned(),
        phrase: game::held(),
        notes: Vec::new(),
    })
}

// ---------------------------------------------------------------------------------------------------------------
// The free set
// ---------------------------------------------------------------------------------------------------------------

/// The Recreate level that first randomizes the category of a parameter's module (lane RG's categories: the
/// oscillators and the noise, the filter, the envelopes, the LFOs, the effects and the voice).
fn level_of(start: &Patch, addr: &ParamAddr) -> u8 {
    let category = match addr.target() {
        Target::Voice | Target::Master => Category::Structure,
        Target::Module(id) => match start.find(id).map(|(_, m)| m.kind) {
            // S2a's reserved kinds sit with their section's P1 kinds (lane MTC may refine this when they are listed).
            Some(ModuleKind::Osc | ModuleKind::Noise | ModuleKind::Wt) => Category::Oscillators,
            Some(
                ModuleKind::Svf
                | ModuleKind::Dist
                | ModuleKind::Comb
                | ModuleKind::Formant
                | ModuleKind::Ladder,
            ) => Category::Filter,
            Some(ModuleKind::Adsr) => Category::Envelopes,
            Some(ModuleKind::Lfo | ModuleKind::Mseg) => Category::Modulation,
            Some(
                ModuleKind::Reverb
                | ModuleKind::Comp
                | ModuleKind::Ott
                | ModuleKind::Chorus
                | ModuleKind::Phaser
                | ModuleKind::Eq
                | ModuleKind::Delay,
            )
            | None => Category::Effects,
        },
    };
    unlock_level(category)
}

fn free_set(args: &Args, loaded: &Loaded) -> Result<(FreeSet, String), CliError> {
    let start = &loaded.start;
    let mask = random::observable(start, loaded.phrase);
    let mut addrs: Vec<ParamAddr> = Vec::new();
    let how = if let Some(n) = args.level {
        addrs.extend(
            mask.observable_params()
                .filter(|a| level_of(start, a) <= n)
                .cloned(),
        );
        format!("level {n}")
    } else {
        let tokens: Vec<&str> = args
            .free
            .iter()
            .map(|t| t.trim())
            .filter(|t| !t.is_empty())
            .collect();
        let tokens = if tokens.is_empty() {
            vec!["observable"]
        } else {
            tokens
        };
        for t in &tokens {
            addrs.extend(expand(t, start, &mask, loaded.round.as_ref())?);
        }
        match tokens.as_slice() {
            ["observable"] => "observable".to_owned(),
            ["drawn"] => "drawn".to_owned(),
            _ => "listed".to_owned(),
        }
    };
    addrs.sort();
    addrs.dedup();
    let free = FreeSet::new(start, &addrs).map_err(|e| {
        let mut err = game::from_search(&e);
        if matches!(e, enth_solver::search::SearchError::NoFreeParameters) {
            err.0.hint = format!(
                "nothing in the start patch can be heard to change ({how}); name parameters to search, e.g. --free filter1.cutoff, or --start a patch with the modules the target needs"
            );
        }
        err
    })?;
    Ok((free, how))
}

/// One `--free` token: a keyword, a module id, `voice`/`master`, or a parameter address.
fn expand(
    token: &str,
    start: &Patch,
    mask: &Mask,
    round: Option<&PreparedRound>,
) -> Result<Vec<ParamAddr>, CliError> {
    let ctx = Ctx {
        patch: Some(start),
        addr: Some(token),
        ..Ctx::default()
    };
    match token {
        "observable" => return Ok(mask.observable_params().cloned().collect()),
        "drawn" => {
            return match round {
                Some(r) => Ok(r
                    .params()
                    .iter()
                    .filter(|a| start.spec(a).is_some())
                    .cloned()
                    .collect()),
                None => Err(error::usage(
                    "bad_free",
                    "--free drawn needs a challenge code as the target: only a challenge has randomized parameters",
                    "use --free observable, a module id (filter1) or addresses (filter1.cutoff)",
                )),
            };
        }
        _ => {}
    }
    if !token.contains('.') {
        // A built-in or a module id: every observable parameter it has.
        let target = match token {
            "voice" => Target::Voice,
            "master" => Target::Master,
            _ => {
                let found = enth_patch::ModuleId::new(token)
                    .ok()
                    .filter(|id| start.find(id).is_some());
                match found {
                    Some(id) => Target::Module(id),
                    None => return Err(unknown_module(start, token)),
                }
            }
        };
        let addrs: Vec<ParamAddr> = mask
            .observable_params()
            .filter(|a| *a.target() == target)
            .cloned()
            .collect();
        if addrs.is_empty() {
            return Err(error::invalid(
                "nothing_to_search",
                format!("{token} has no parameter that can be heard to change in this start patch"),
                "it is disabled, unrouted or silent; enable or route it first, or name its parameters (e.g. filter1.cutoff) to search them anyway",
            )
            .at(token));
        }
        return Ok(addrs);
    }
    let addr: ParamAddr = token
        .parse()
        .map_err(|e| error::from_patch_error(token, &PatchError::Addr(e), ctx))?;
    if start.spec(&addr).is_some() {
        return Ok(vec![addr]);
    }
    if let Target::Module(id) = addr.target() {
        return Err(match start.find(id) {
            None => unknown_module(start, id.as_str()),
            Some((_, m)) => error::from_patch_error(
                token,
                &PatchError::UnknownParam {
                    addr: token.to_owned(),
                    owner: m.kind.key(),
                    keys: m.kind.params().iter().map(|s| s.key).collect(),
                },
                ctx,
            ),
        });
    }
    let owner = if matches!(addr.target(), Target::Voice) {
        Builtin::Voice
    } else {
        Builtin::Master
    };
    Err(error::invalid(
        "unknown_param",
        format!("'{token}' names no parameter"),
        format!("`enth params {}` lists the keys", owner.key()),
    )
    .at(token))
}

fn unknown_module(start: &Patch, id: &str) -> CliError {
    error::from_patch_error(
        id,
        &PatchError::UnknownModule {
            id: id.to_owned(),
            existing: templates::module_ids(start),
        },
        Ctx {
            patch: Some(start),
            addr: Some(id),
            ..Ctx::default()
        },
    )
    .suggest(
        aliases::module_ids(start, id)
            .into_iter()
            .chain(error::nearest(
                id,
                templates::module_ids(start).iter().map(String::as_str),
            ))
            .take(3)
            .collect(),
    )
}

// ---------------------------------------------------------------------------------------------------------------
// The search's distance
// ---------------------------------------------------------------------------------------------------------------

/// Metric v1 of every candidate against the target, on rayon: one `MetricWorker` per pool thread (its FFT plans and
/// filter banks are built once, not per batch), results collected by index. A distance is a pure function of its
/// candidate, whichever worker computes it, so the search is the same for any number of threads.
struct PoolObjective<'a> {
    workers: Vec<Mutex<MetricWorker<'a>>>,
}

impl<'a> PoolObjective<'a> {
    fn new(target: &'a MetricTarget) -> Self {
        PoolObjective {
            workers: (0..rayon::current_num_threads().max(1))
                .map(|_| Mutex::new(target.worker()))
                .collect(),
        }
    }
}

impl Objective for PoolObjective<'_> {
    fn distances(&mut self, candidates: &[Patch]) -> Vec<f64> {
        let workers = &self.workers;
        candidates
            .par_iter()
            .map(|patch| {
                let i = rayon::current_thread_index().unwrap_or(0) % workers.len();
                workers[i]
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .distance(patch)
            })
            .collect()
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Output
// ---------------------------------------------------------------------------------------------------------------

fn stop_name(s: StopCause) -> &'static str {
    match s {
        StopCause::Budget => "budget",
        StopCause::Goal => "goal",
        StopCause::Cancelled => "time",
        StopCause::Exhausted => "exhausted",
    }
}

fn stage_name(s: Stage) -> String {
    match s {
        Stage::Probe => "probe".to_owned(),
        Stage::Run(n) => format!("run {n}"),
    }
}

/// The trace: the batches where the best distance improved, thinned to [`TRACE_POINTS`] (first and last kept), or
/// every batch with `--full`.
fn trace_out(trace: &[TraceStep], full: bool) -> TraceOut {
    let mut points: Vec<&TraceStep> = Vec::new();
    let mut best = f64::INFINITY;
    for step in trace {
        if full || step.best < best {
            points.push(step);
        }
        best = best.min(step.best);
    }
    if !full && points.len() > TRACE_POINTS {
        let n = points.len();
        points = (0..TRACE_POINTS)
            .map(|k| points[k * (n - 1) / (TRACE_POINTS - 1)])
            .collect();
    }
    TraceOut {
        batches: trace.len(),
        steps: points
            .into_iter()
            .map(|s| TraceStepOut {
                renders: s.renders,
                stage: stage_name(s.stage),
                best: s.best,
            })
            .collect(),
    }
}

fn next_step(args: &Args, found: &SearchResult, passed: Option<bool>) -> String {
    let apply = if args.out.is_some() {
        "the best patch is written to --out"
    } else if args.start.is_some() {
        "`enth apply <start> --ops '<ops>'` writes the best patch, or rerun with --out"
    } else {
        "rerun with --out <patch> to write the best patch"
    };
    match (passed, found.stop) {
        (Some(true), _) => format!("it passes the challenge: {apply}"),
        (Some(false), StopCause::Budget) => format!(
            "it does not pass yet: raise --budget, free more (--free observable, or a higher --level), or try another --seed; {apply}"
        ),
        (Some(false), _) => format!(
            "it does not pass yet: free more parameters (--free observable) or ask `enth suggest` what is still off; {apply}"
        ),
        (None, StopCause::Budget) => format!(
            "the budget ran out: raise --budget, free more parameters, or start from a patch with the right modules; {apply}"
        ),
        (None, _) => apply.to_owned(),
    }
}

/// What a terminal prints.
fn summary(o: &MatchOut) -> String {
    let mut s = format!(
        "match {} {} from {}\n  free: {} ({} continuous, {} categorical)\n  distance: {} (start {})\n",
        o.target.kind,
        o.target.source,
        o.start,
        o.free.how,
        o.free.continuous,
        o.free.categorical,
        round4(o.distance),
        o.start_distance
            .map_or_else(|| "-".to_owned(), |d| round4(d).to_string()),
    );
    if let Some(sc) = &o.score {
        let _ = writeln!(
            s,
            "  score: sound {}%, structure {}%, combined {}% ({} the pass mark {})",
            round4(sc.sound),
            round4(sc.structure),
            round4(sc.combined),
            if sc.passed { "passes" } else { "below" },
            sc.pass_mark
        );
    }
    let _ = writeln!(
        s,
        "  {} renders, {} runs, stopped on {}",
        o.renders, o.runs, o.stop
    );
    for c in &o.changes {
        let _ = writeln!(
            s,
            "  {} {} -> {}",
            c.target,
            c.from.as_deref().unwrap_or("-"),
            c.to.as_deref().unwrap_or("-")
        );
    }
    if let Some(w) = &o.wrote {
        let _ = writeln!(s, "  wrote {w}");
    }
    let _ = writeln!(s, "{}", o.hint);
    s
}
