//! The hint seam: where `check` and `step` ask for one measured multi-parameter `fix` for all failing items, and
//! for the items that are `blocked` (docs/agent-cli.md, "One hint solves all failing items at once").
//!
//! The search itself is `enth-solver::hint` (L5, lane H, `docs/dsp/hints.md`); this module only translates. It
//! gives the solver the goal's performance and a rayon [`Parallel`] evaluator (the solver's own `measure`, so a
//! hint's `before` is the CLI's reading), maps the [`enth_solver::HintResult`] onto [`FixOutcome`] and adds the two
//! pieces the solver leaves to its caller:
//!
//! - **The exact clipping fix.** When an item about clipping or the peak level fails, `master.level` is lowered by
//!   exactly the overshoot (plus a margin) before the search starts, and frozen during it. Gain at the end of the
//!   chain moves the peak and the loudness one dB per dB and nothing else, so one parameter undoes clipping
//!   without retuning every module (agents in the Phase 0 probe had no way back from clipping). It is applied
//!   only when it fixes the clipping items and breaks no passing item, which is checked on the judged descriptors
//!   before any render; otherwise the solver searches the whole space as usual.
//! - **Structural text hints** (`"measured": false`) for the items whose descriptor no parameter of the current
//!   structure can reach (`blocked` for `no_effect`, `absent` or `at_bound`): add a second oscillator, a filter,
//!   an LFO, a noise source.
//!
//! `check` calls [`propose`] only for a patch (a WAV has nothing to move), only when at least one item failed, and
//! not with `--no-fix`. A `--fix-budget` of 0 searches nothing. The evaluator renders with the goal's performance
//! (`--perf`), because envelope and beating items cannot be judged on a short probe clip.

use enth_analysis::expect::Test;
use enth_analysis::{CheckReport, Descriptors, Expect, Feature};
use enth_patch::catalogue::MasterParam;
use enth_patch::{ModuleKind, Op, ParamAddr, Patch};
use enth_render::RenderSpec;
use enth_solver::{HintCfg, HintError, HintResult, hint};

use crate::error::{self, CliError};
use crate::eval::{Evaluator, Parallel};

/// The default `--fix-budget`: renders the search may use.
pub const DEFAULT_BUDGET: u32 = 16;

/// Headroom the exact clipping fix leaves below full scale, in dB. The solver aims at the same 1 dB for a
/// `clipping == false` item.
const CLIP_HEADROOM_DB: f64 = 1.0;

/// Everything the hint solver gets.
pub struct FixRequest<'a> {
    /// The patch being judged.
    pub patch: &'a Patch,
    /// The parsed expectation list, in order.
    pub items: &'a [Expect],
    /// The verdict on the judged render (with noise from the nuisance re-renders).
    pub report: &'a CheckReport,
    /// The descriptors of the judged render.
    pub base: &'a Descriptors,
    /// The goal's performance and the judged seed: every render of the search uses them.
    pub evaluator: &'a Evaluator,
    /// The most renders the search may use (`--fix-budget`, default [`DEFAULT_BUDGET`]).
    pub budget: u32,
}

/// One measured move for the whole list.
#[derive(Clone, Debug, PartialEq)]
pub struct Fix {
    /// The ops to apply together, in order (resolved, plain values).
    pub ops: Vec<Op>,
    /// The readings measured after applying them.
    pub after: Vec<(Feature, f64)>,
    /// Every item passes after the move.
    pub pass_all: bool,
    /// One parameter would have done.
    pub single_param_enough: bool,
    /// `after` holds real values from a render.
    pub measured: bool,
    /// Renders the search used.
    pub renders: u32,
    /// What to know about it ("apply these ops together; ...").
    pub note: String,
}

/// An item that no move of the patch's parameters can satisfy.
#[derive(Clone, Debug, PartialEq)]
pub struct Blocked {
    /// Index of the item in [`FixRequest::items`].
    pub item: usize,
    /// Why, in one sentence: the structure or the performance is the limit.
    pub why: String,
    /// The stable code of the reason (`performance`, `at_bound`, `conflict`, `no_effect`, `absent`, `unmodelled`).
    pub reason: &'static str,
    /// Parameters sitting at a bound.
    pub at_bound: Vec<ParamAddr>,
    /// Items the move this one needs would push out of their bands.
    pub conflicts_with: Vec<usize>,
}

/// A suggestion for one item (structural ones are not measured).
#[derive(Clone, Debug, PartialEq)]
pub struct ItemHint {
    /// The suggestion.
    pub text: String,
    /// It was rendered and measured.
    pub measured: bool,
}

/// What the solver found.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FixOutcome {
    /// The move, if one was found.
    pub fix: Option<Fix>,
    /// Items no move can satisfy.
    pub blocked: Vec<Blocked>,
    /// Per item (same length as the items, or empty): a suggestion.
    pub item_hints: Vec<Option<ItemHint>>,
}

/// Ask for a fix: the exact clipping fix first when it applies, then the solver's search on the rest.
///
/// # Errors
/// The solver could not run (the base render failed, or the evaluator broke its contract). A render that fails
/// inside the search only drops that candidate.
pub fn propose(req: &FixRequest<'_>) -> Result<FixOutcome, CliError> {
    if req.budget == 0 || req.items.is_empty() {
        return Ok(FixOutcome::default());
    }
    let clip = clip_fix(req);
    let start = clip.as_ref().map_or(req.patch, |c| &c.patch);
    let cfg = HintCfg {
        max_renders: req.budget as usize,
        frozen: clip.iter().map(|c| c.addr.clone()).collect(),
        ..HintCfg::default()
    };
    let spec = RenderSpec::new(req.evaluator.sample_rate(), req.evaluator.seed());
    let res = hint(
        start,
        req.evaluator.performance(),
        &spec,
        req.items,
        &mut Parallel,
        &cfg,
    )
    .map_err(|e| hint_error(&e))?;
    Ok(outcome(req, clip.as_ref(), &res))
}

/// The ops of a fix as `--ops` JSON, values in the units the parameter shows (`1.169kHz`, `-6.5dB`): four
/// significant digits when that parses back to the rendered value, the exact display otherwise. Replaying them
/// reproduces the measured render bit for bit.
pub fn ops_json(patch: &Patch, ops: &[Op]) -> Vec<serde_json::Value> {
    ops.iter()
        .flat_map(|op| match (op, op_spec(patch, op)) {
            (Op::SetParam { addr, value }, Some(spec)) => {
                let rounded = spec.display_rounded(*value, 4).to_string();
                let text = if spec
                    .parse(&rounded)
                    .is_ok_and(|v| v.to_bits() == value.to_bits())
                {
                    rounded
                } else {
                    spec.display(*value).to_string()
                };
                vec![serde_json::json!({ "set": addr.to_string(), "to": text })]
            }
            _ => match enth_patch::wire::to_json(patch, std::slice::from_ref(op)) {
                serde_json::Value::Array(a) => a,
                other => vec![other],
            },
        })
        .collect()
}

fn op_spec(patch: &Patch, op: &Op) -> Option<&'static enth_params::ParamSpec> {
    match op {
        Op::SetParam { addr, .. } => patch.spec(addr),
        _ => None,
    }
}

fn hint_error(e: &HintError) -> CliError {
    match e {
        HintError::Base(inner) => error::from_eval(inner),
        other => error::failed(
            other.code(),
            format!("the fix search could not run: {other}"),
            "retry with --no-fix to judge without a fix; report this as a bug",
        ),
    }
}

// ---------------------------------------------------------------------------------------------------------------
// The exact clipping fix
// ---------------------------------------------------------------------------------------------------------------

/// `master.level` lowered by the overshoot.
struct ClipFix {
    /// The patch with the move applied: where the search starts.
    patch: Patch,
    /// `master.level`.
    addr: ParamAddr,
    /// The move.
    op: Op,
}

/// How much quieter (dB) the output must be for a failing item about the level to pass, aimed one margin inside
/// its bound. `None` for items that are not about the level, or have no upper bound to get under.
fn attenuation_for(e: &Expect, d: &Descriptors) -> Option<f64> {
    let Test::Within(iv) = e.test else {
        return None;
    };
    let lv = &d.loudness;
    match e.feature {
        Feature::Clipping if iv.lo < 0.5 => Some(lv.peak_dbfs + CLIP_HEADROOM_DB),
        Feature::ClippedPct if iv.hi.is_finite() => Some(lv.peak_dbfs + CLIP_HEADROOM_DB),
        Feature::Peak if iv.hi.is_finite() => {
            Some(lv.peak_dbfs - (iv.hi - e.feature.hint_margin()))
        }
        Feature::TruePeak if iv.hi.is_finite() => {
            Some(lv.true_peak_dbtp - (iv.hi - e.feature.hint_margin()))
        }
        _ => None,
    }
}

/// The descriptors after `db` dB of gain at the end of the chain: the level readings move, every other reading is
/// relative and stays. `None` when the result would be silent to the analysis (below −80 dBFS).
fn attenuated(d: &Descriptors, db: f64) -> Option<Descriptors> {
    let mut s = d.clone();
    let lv = &mut s.loudness;
    lv.peak_dbfs -= db;
    lv.true_peak_dbtp -= db;
    lv.lufs_i = lv.lufs_i.map(|l| l - db);
    if lv.peak_dbfs >= 0.0 {
        return None;
    }
    lv.clipped_pct = 0.0;
    lv.clipping = false;
    (lv.peak_dbfs > -80.0).then_some(s)
}

/// The exact `master.level` fix, when an item about clipping or the peak level fails and lowering the master
/// level passes every such item without failing one that passes now.
fn clip_fix(req: &FixRequest<'_>) -> Option<ClipFix> {
    let needed = req
        .items
        .iter()
        .zip(&req.report.results)
        .filter(|(_, r)| !r.pass)
        .filter_map(|(e, _)| attenuation_for(e, req.base))
        .fold(None, |m: Option<f64>, a| Some(m.map_or(a, |m| m.max(a))))?;
    if !(needed.is_finite() && needed > 0.0) {
        return None;
    }
    let after = attenuated(req.base, needed)?;
    // Every item that passes keeps passing, and every level item that fails now passes.
    let clean = req.items.iter().zip(&req.report.results).all(|(e, r)| {
        let now = e.evaluate(&after, &[]).pass;
        now || (!r.pass && attenuation_for(e, req.base).is_none())
    });
    if !clean {
        return None;
    }
    let addr = ParamAddr::master(MasterParam::Level);
    let spec = req.patch.spec(&addr)?;
    let from = req.patch.value(&addr)?;
    let want = (f64::from(from) - needed) as f32;
    // What the CLI prints is what is rendered: four significant digits, as the solver does.
    let to = spec
        .parse(&spec.display_rounded(want, 4).to_string())
        .ok()?;
    if to.to_bits() == from.to_bits() || (f64::from(to) - f64::from(want)).abs() > 0.05 {
        return None; // clamped at the bottom of the range, or no move at all
    }
    let op = Op::SetParam {
        addr: addr.clone(),
        value: to,
    };
    let mut patch = req.patch.clone();
    enth_patch::apply(&mut patch, op.clone()).ok()?;
    Some(ClipFix { patch, addr, op })
}

// ---------------------------------------------------------------------------------------------------------------
// HintResult -> FixOutcome
// ---------------------------------------------------------------------------------------------------------------

fn outcome(req: &FixRequest<'_>, clip: Option<&ClipFix>, res: &HintResult) -> FixOutcome {
    let blocked: Vec<Blocked> = res
        .blocked
        .iter()
        .map(|b| Blocked {
            item: b.item,
            why: b.why.clone(),
            reason: b.reason.code(),
            at_bound: b.at_bound.clone(),
            conflicts_with: b.conflicts_with.clone(),
        })
        .collect();
    let renders = u32::try_from(res.stats.renders).unwrap_or(u32::MAX);
    let fix = match (&res.fix, clip) {
        (Some(f), None) => Some(Fix {
            ops: f.ops(),
            after: readings(req.items, &f.after),
            pass_all: f.pass_all,
            single_param_enough: f.single_param_enough,
            measured: true,
            renders,
            note: fix_note(req.items, &f.still_failing(), &blocked),
        }),
        (Some(f), Some(c)) => {
            let mut ops = vec![c.op.clone()];
            ops.extend(f.ops());
            Some(Fix {
                ops,
                after: readings(req.items, &f.after),
                pass_all: f.pass_all,
                single_param_enough: false,
                measured: true,
                renders,
                note: fix_note(req.items, &f.still_failing(), &blocked),
            })
        }
        // The lowered master level is the whole fix: the search started from it and found nothing to add (or
        // nothing to do, when it already passes every item).
        (None, Some(c)) => {
            let still: Vec<usize> = (0..res.before.len())
                .filter(|&i| !res.before[i].pass)
                .collect();
            Some(Fix {
                ops: vec![c.op.clone()],
                after: readings(req.items, &res.before),
                pass_all: still.is_empty(),
                single_param_enough: still.is_empty(),
                measured: true,
                renders,
                note: fix_note(req.items, &still, &blocked),
            })
        }
        (None, None) => None,
    };
    FixOutcome {
        fix,
        item_hints: item_hints(req, &blocked),
        blocked,
    }
}

/// The readings of the final render, by feature.
fn readings(items: &[Expect], after: &[enth_analysis::ItemResult]) -> Vec<(Feature, f64)> {
    items
        .iter()
        .zip(after)
        .filter_map(|(e, r)| r.got.map(|g| (e.feature, g)))
        .collect()
}

fn fix_note(items: &[Expect], still: &[usize], blocked: &[Blocked]) -> String {
    const APPLY: &str = "apply these ops together";
    if still.is_empty() {
        return format!("{APPLY}; every item passes after them");
    }
    let names: Vec<&str> = still
        .iter()
        .filter_map(|&i| items.get(i).map(|e| e.canonical.as_str()))
        .collect();
    let mut note = format!(
        "{APPLY}; {} of {} items still fail after them ({})",
        still.len(),
        items.len(),
        names.join(", ")
    );
    if still.iter().any(|i| blocked.iter().any(|b| b.item == *i)) {
        note.push_str("; see blocked for what cannot be done");
    }
    if still.iter().any(|i| !blocked.iter().any(|b| b.item == *i)) {
        note.push_str(
            "; the search ran out of budget or steps on the rest: raise --fix-budget or check the fixed patch again",
        );
    }
    note
}

// ---------------------------------------------------------------------------------------------------------------
// Structural text hints
// ---------------------------------------------------------------------------------------------------------------

/// A text hint for each blocked item whose structure, not a parameter, is the limit. The list is as long as the
/// items, or empty when no item has one.
fn item_hints(req: &FixRequest<'_>, blocked: &[Blocked]) -> Vec<Option<ItemHint>> {
    let mut hints: Vec<Option<ItemHint>> = vec![None; req.items.len()];
    for b in blocked {
        let structural = matches!(b.reason, "no_effect" | "absent" | "at_bound");
        if !structural {
            continue;
        }
        if let Some(text) = req
            .items
            .get(b.item)
            .and_then(|e| structural_hint(e.feature, req.patch))
        {
            hints[b.item] = Some(ItemHint {
                text,
                measured: false,
            });
        }
    }
    if hints.iter().all(Option::is_none) {
        hints.clear();
    }
    hints
}

fn count_enabled(patch: &Patch, kind: ModuleKind) -> usize {
    patch
        .modules()
        .filter(|(_, m)| m.kind == kind && m.enabled)
        .count()
}

/// What to add or change when no parameter of the patch reaches `feature`. Examples are valid `--ops` for `enth
/// step` (a unit test applies them).
pub(crate) fn structural_hint(feature: Feature, patch: &Patch) -> Option<String> {
    let first_osc = patch
        .modules()
        .find(|(_, m)| m.kind == ModuleKind::Osc && m.enabled)
        .map(|(_, m)| m.id.to_string());
    match feature {
        Feature::Beating | Feature::BeatDepth => Some(if count_enabled(patch, ModuleKind::Osc) < 2 {
            "Beating needs two oscillators detuned against each other. Add a second one: --ops '[{\"add\":\"osc\",\"params\":{\"wave\":\"saw\",\"fine\":\"7ct\"}}]'.".to_owned()
        } else {
            "The oscillators do not beat in the 0.15 to 25 Hz band that detune.beating_hz measures: detune them against each other with osc.fine, a few cents at the pitch played.".to_owned()
        }),
        Feature::Tremolo | Feature::TremoloDepth => {
            let target = first_osc?;
            Some(if count_enabled(patch, ModuleKind::Lfo) == 0 {
                format!(
                    "Tremolo needs an LFO on the level. Add one and route it: --ops '[{{\"add\":\"lfo\",\"params\":{{\"rate\":\"5Hz\"}}}},{{\"route\":\"lfo1 -> {target}.level\",\"depth\":\"30%\"}}]'."
                )
            } else {
                format!(
                    "Route an LFO to a level so the amplitude wobbles: --ops '[{{\"route\":\"<lfo id> -> {target}.level\",\"depth\":\"30%\"}}]' (rate 0.5 to 15 Hz)."
                )
            })
        }
        Feature::Brightness => Some(if count_enabled(patch, ModuleKind::Svf) == 0 {
            "The patch has no filter to shape the brightness. Add one: --ops '[{\"add\":\"svf\",\"params\":{\"mode\":\"lp\",\"cutoff\":\"2kHz\"}}]'.".to_owned()
        } else {
            "No setting of the filter reaches this. The oscillator waveform sets the harmonics the filter can shape: saw and pulse are bright, triangle and sine are dark (osc.wave).".to_owned()
        }),
        Feature::Noisiness if count_enabled(patch, ModuleKind::Noise) == 0 => Some(
            "There is no noise source. Add one and raise its level: --ops '[{\"add\":\"noise\",\"params\":{\"level\":\"20%\"}}]'.".to_owned(),
        ),
        Feature::Pitch | Feature::PitchCents | Feature::PitchStability | Feature::TonalPitch => {
            pitch_hint(feature, patch, first_osc.as_deref())
        }
        Feature::SweepRate | Feature::SweepDepth | Feature::SweepTime => {
            let lfo = if feature == Feature::SweepTime {
                // a one-shot sweep: an envelope on the cutoff
                let env = free_id(patch, "env");
                format!(
                    "{{\"add\":\"adsr\",\"id\":\"{env}\",\"params\":{{\"attack\":\"1ms\",\"decay\":\"300ms\",\"sustain\":\"0%\"}}}},{{\"route\":\"{env} -> filter1.cutoff\",\"depth\":\"+3oct\"}}"
                )
            } else {
                let lfo = free_id(patch, "lfo");
                format!(
                    "{{\"add\":\"lfo\",\"id\":\"{lfo}\",\"params\":{{\"rate\":\"2Hz\"}}}},{{\"route\":\"{lfo} -> filter1.cutoff\",\"depth\":\"+1oct\"}}"
                )
            };
            let routed = patch.routes.iter().any(|r| r.dst.param() == "cutoff" && r.depth != 0.0);
            if count_enabled(patch, ModuleKind::Svf) == 0 {
                Some(format!(
                    "A brightness sweep needs a filter and a source routed to its cutoff: --ops '[{{\"add\":\"svf\",\"id\":\"filter1\",\"params\":{{\"mode\":\"lp\",\"cutoff\":\"800Hz\"}}}},{lfo}]'."
                ))
            } else if !routed && patch.find(&enth_patch::ModuleId::new("filter1").ok()?).is_some() {
                Some(format!(
                    "Route a source to the filter's cutoff (an LFO for a cyclic sweep, an envelope for a one-shot one): --ops '[{lfo}]'."
                ))
            } else {
                None
            }
        }
        _ => None,
    }
}

/// The lowest free id with `prefix` (`lfo1`, `env3`). A patch holds at most a few dozen modules, so one of the
/// first hundred is free.
fn free_id(patch: &Patch, prefix: &str) -> String {
    (1..=100u32)
        .map(|n| format!("{prefix}{n}"))
        .find(|id| enth_patch::ModuleId::new(id).is_ok_and(|m| patch.find(&m).is_none()))
        .unwrap_or_else(|| format!("{prefix}101"))
}

/// Pitch items: a transposed oscillator sets the period (the hint solver never moves `semi`), noise hides it, or
/// there is no oscillator at all.
fn pitch_hint(feature: Feature, patch: &Patch, first_osc: Option<&str>) -> Option<String> {
    let transposed = patch
        .modules()
        .filter(|(_, m)| m.kind == ModuleKind::Osc && m.enabled)
        .find_map(|(_, m)| {
            m.get("semi")
                .filter(|s| *s != 0.0)
                .map(|s| (m.id.to_string(), s))
        });
    let noise = patch
        .modules()
        .find(|(_, m)| m.kind == ModuleKind::Noise && m.enabled)
        .map(|(_, m)| m.id.to_string());
    if first_osc.is_none() {
        return Some(
            "There is no oscillator to give a pitch. Add one: --ops '[{\"add\":\"osc\",\"params\":{\"wave\":\"saw\"}}]'."
                .to_owned(),
        );
    }
    if let Some((id, semi)) = transposed {
        return Some(format!(
            "{id} is transposed {semi:+} st, and the pitch follows the lowest strong oscillator; the fix never moves semi. Play the note with --ops '[{{\"set\":\"{id}.semi\",\"to\":\"0st\"}}]', or lower {id}.level."
        ));
    }
    match (noise, feature) {
        (Some(n), Feature::TonalPitch) => Some(format!(
            "The tone is too weak next to {n}: raise the oscillator's level or lower the noise, --ops '[{{\"set\":\"{n}.level\",\"to\":\"30%\"}}]'."
        )),
        (Some(n), _) => Some(format!(
            "{n} hides the pitch (the waveform no longer repeats). Judge the tone under the noise with tonal_pitch instead, or lower the noise: --ops '[{{\"set\":\"{n}.level\",\"to\":\"20%\"}}]'."
        )),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use enth_patch::wire;

    /// Every example in a structural hint is a valid, applicable `--ops` value on a patch it is given for.
    #[test]
    fn structural_examples_parse_and_apply() {
        let patch = Patch::init();
        let with = |ops: &str| {
            let mut p = Patch::init();
            let ops = wire::resolve_str(&p, ops).unwrap();
            enth_patch::apply_all(&mut p, &ops).unwrap();
            p
        };
        let sub = with(r#"[{"add":"osc","params":{"wave":"sine","semi":"-12st"}}]"#);
        let noisy = with(r#"[{"add":"noise","id":"noise1"}]"#);
        let filtered = with(r#"[{"add":"svf","id":"filter1"}]"#);
        let no_osc = with(r#"[{"remove":"osc1"}]"#);
        for (feature, patch) in [
            (Feature::Beating, &patch),
            (Feature::Tremolo, &patch),
            (Feature::Brightness, &patch),
            (Feature::Noisiness, &patch),
            (Feature::Pitch, &sub),
            (Feature::Pitch, &noisy),
            (Feature::TonalPitch, &noisy),
            (Feature::Pitch, &no_osc),
            (Feature::SweepRate, &patch),
            (Feature::SweepTime, &patch),
            (Feature::SweepDepth, &filtered),
        ] {
            let text = structural_hint(feature, patch)
                .unwrap_or_else(|| panic!("{feature:?} has a hint on the init patch"));
            let start = text.find("--ops '").expect("an example") + "--ops '".len();
            let end = start + text[start..].find("]'").unwrap();
            let json = &text[start..=end];
            let ops = wire::resolve_str(patch, json)
                .unwrap_or_else(|e| panic!("{feature:?}: {json}: {e:?}"));
            let mut p = patch.clone();
            enth_patch::apply_all(&mut p, &ops)
                .unwrap_or_else(|e| panic!("{feature:?}: {json}: {e:?}"));
        }
        // Nothing to say for features a parameter can reach, or a pitch with nothing structural in the way.
        assert!(structural_hint(Feature::Attack, &patch).is_none());
        assert!(structural_hint(Feature::Pitch, &patch).is_none());
        let sub_text = structural_hint(Feature::Pitch, &sub).unwrap();
        assert!(sub_text.contains("osc2 is transposed -12 st"), "{sub_text}");
    }

    #[test]
    fn hints_follow_the_structure() {
        let mut two = Patch::init();
        let ops = wire::resolve_str(&two, r#"[{"add":"osc"}]"#).unwrap();
        enth_patch::apply_all(&mut two, &ops).unwrap();
        let one = structural_hint(Feature::Beating, &Patch::init()).unwrap();
        let both = structural_hint(Feature::Beating, &two).unwrap();
        assert!(one.contains("Add a second one"), "{one}");
        assert!(both.contains("osc.fine"), "{both}");
    }

    #[test]
    fn a_note_names_what_is_left() {
        let items = enth_analysis::parse_list("attack < 10ms, brightness > 3kHz").unwrap();
        let blocked = [Blocked {
            item: 1,
            why: "x".to_owned(),
            reason: "at_bound",
            at_bound: Vec::new(),
            conflicts_with: Vec::new(),
        }];
        assert!(fix_note(&items, &[], &[]).ends_with("every item passes after them"));
        let n = fix_note(&items, &[1], &blocked);
        assert!(n.contains("1 of 2 items still fail"), "{n}");
        assert!(n.contains("see blocked"), "{n}");
        assert!(!n.contains("budget"), "{n}");
        let n = fix_note(&items, &[0], &blocked);
        assert!(n.contains("raise --fix-budget"), "{n}");
    }
}
