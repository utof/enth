//! **Owner: lane ECK.** The reference ladder: Ear check's parameter steps, measured.
//!
//! Ear check is the one shipped game whose difficulty is a number the code can set: the odd interval is the base with
//! one control moved by `steps` parameter steps (`params::ear_check_on`), and the staircase walks the steps. This
//! measures, for each control Ear check can ask (the observable ones in the level's pool), how far apart the base and
//! the odd interval are under `sound/1` at each of the steps 8 down to 1, and so gives the staircase simulations a real
//! ladder to run over: level 1 is 8 steps, level 8 is 1 step. It stands for the ladders G1 to G4 will deliver, and shows
//! G4 how steep its steps are, per control, before it places them (`ear.md`: "items placed by measured JND distance").
//!
//! The distance of a draw is the smaller of the two directions (the base as the target and the odd interval as the
//! target), as `choice_audibility` measures lesson options; a row is the median over the draws (both directions of
//! movement among them).

use enth_challenge::ear::params;
use enth_challenge::random;
use enth_patch::ParamAddr;

use super::check::{Plan, par_map};
use super::game::check_trial;
use super::metric::{self, Calibrations, Playable};
use super::staircase::{self, Evaluation, Ladder, Row};

/// The steps, hardest last: level `L` is `STEPS - L + 1` parameter steps.
pub const STEPS: u8 = 8;

/// Draws per (control, steps) point.
pub const DRAWS: u32 = 4;

/// One control's measured ladder.
#[derive(Clone, Debug)]
pub struct StepLadder {
    /// The control.
    pub control: String,
    /// The ladder: level 1 is 8 steps.
    pub ladder: Ladder,
    /// The simulations (`None` when the ladder is too short, never here).
    pub eval: Option<Evaluation>,
}

/// The controls Ear check's level 1 can ask: its pool, as heard on the base.
///
/// # Errors
/// A pool entry does not parse.
pub fn controls() -> Result<Vec<ParamAddr>, String> {
    let lvl = params::level(params::EAR_CHECK, 1).ok_or("Ear check has no level 1")?;
    let base = params::base();
    let mask = random::observable(&base, params::phrase());
    let mut out = Vec::new();
    for text in lvl.pool {
        let addr: ParamAddr = text.parse().map_err(|e| format!("{text}: {e}"))?;
        if mask.is_observable(&addr) {
            out.push(addr);
        }
    }
    Ok(out)
}

/// The distance, in JND, of one draw: the base against the odd interval.
fn draw_jnd(
    control: &ParamAddr,
    steps: u8,
    seed: u64,
    index: u32,
    cache: &Calibrations,
) -> Result<f64, String> {
    let (base, odd) = check_trial(control, steps, seed, index)?;
    let (pb, po) = (Playable::of(&base)?, Playable::of(&odd)?);
    let (hb, ho) = (metric::hear(&pb)?, metric::hear(&po)?);
    let (cb, co) = (cache.get(&base, &pb)?, cache.get(&odd, &po)?);
    Ok(metric::jnd_between(&cb, &hb, &ho).min(metric::jnd_between(&co, &hb, &ho)))
}

/// Measure every control's ladder and simulate the staircase over it.
///
/// # Errors
/// A trial cannot be drawn or rendered.
pub fn reference(plan: &Plan, cache: &Calibrations) -> Result<Vec<StepLadder>, String> {
    let controls = controls()?;
    let points: Vec<(usize, u8)> = (0..controls.len())
        .flat_map(|c| (1..=STEPS).map(move |s| (c, s)))
        .collect();
    let measured = par_map(&points, plan.jobs, |&(c, steps)| -> Result<f64, String> {
        let mut ds = Vec::new();
        for k in 0..DRAWS {
            let seed = plan.seed ^ (u64::from(steps) << 8) ^ u64::from(k);
            ds.push(draw_jnd(&controls[c], steps, seed, k, cache)?);
        }
        ds.sort_by(f64::total_cmp);
        Ok(ds[ds.len() / 2].midpoint(ds[(ds.len() - 1) / 2]))
    });
    let mut out = Vec::new();
    for (c, control) in controls.iter().enumerate() {
        let mut rows = Vec::new();
        // Level 1 is the most steps (the easiest).
        for steps in (1..=STEPS).rev() {
            let i = points
                .iter()
                .position(|p| *p == (c, steps))
                .ok_or("a point is missing")?;
            let jnd = measured[i]
                .clone()
                .map_err(|e| format!("{control} at {steps} steps: {e}"))?;
            rows.push(Row {
                jnd,
                options: usize::from(params::EAR_CHECK_INTERVALS),
            });
        }
        let ladder = Ladder {
            name: format!("Ear check steps, {control}"),
            rows,
        };
        let eval = staircase::evaluate(&ladder, plan.runs, plan.seed ^ (c as u64 + 1));
        out.push(StepLadder {
            control: control.to_string(),
            ladder,
            eval,
        });
    }
    Ok(out)
}
