//! **Owner: lane CL.** `enth explain <addr|module|kind>`: what a parameter, a module or a module kind does, in plain
//! language and as JSON, from the catalogue (what it does, its unit and range, the descriptor it moves). With
//! `--patch`, it also says what the parameter does *in that patch*, measured: lane SN's sensitivity renders the patch
//! once and once with each parameter moved one just-noticeable step, and reports which descriptors moved, and which
//! parameters do nothing there.
//!
//! ```text
//! enth explain filter1.cutoff                          # the catalogue's facts
//! enth explain svf                                     # a module kind and its parameters
//! enth explain filter1.cutoff --patch p.patch.json     # plus what a step does to this patch
//! ```

use std::fmt::Write as _;
use std::path::PathBuf;

use enth_analysis::Feature;
use enth_params::{Kind, ParamSpec, Scale};
use enth_patch::catalogue::{Builtin, CATALOGUE, ModuleKind};
use enth_patch::{ModuleId, ParamAddr, Patch, PatchError, Target};
use enth_render::{DEFAULT_SAMPLE_RATE, DEFAULT_SEED, RenderSpec};
use enth_solver::sensitivity::{
    Change, ParamSensitivity, Sensitivity, SensitivityCfg, is_log, sensitivity, tolerance_bounds,
};

use super::common::show;
use super::{Reply, Res};
use crate::error::{self, CliError, Ctx, nearest};
use crate::eval::Parallel;
use crate::perf::{Perf, PerfArgs};
use crate::schema::{
    EffectOut, ExplainOut, MeasuredOut, MovesOut, ParamBriefOut, ParamEffectOut, ParamFactsOut,
    Reading, ids,
};
use crate::{aliases, knobs, patchio};

/// The most descriptors listed per parameter.
const EFFECTS_SHOWN: usize = 4;
/// The render budget of a probe (the patch itself, then one per continuous parameter).
const PROBE_RENDERS: usize = 48;

/// `enth explain`'s arguments.
#[derive(Clone, Debug, PartialEq, Eq, clap::Args)]
pub struct Args {
    /// A parameter address (`filter1.cutoff`, `svf.cutoff`, `voice.glide`), a module id (`filter1`, with --patch), a
    /// module kind (`svf`), or `voice` / `master`.
    pub what: String,
    /// A patch: module ids are looked up in it, and the parameters are measured on it.
    #[arg(long, value_name = "PATCH")]
    pub patch: Option<PathBuf>,
    #[command(flatten)]
    pub perf: PerfArgs,
}

/// What is being explained.
enum Subject {
    /// One parameter of a module kind or a built-in.
    Param {
        owner: Owner,
        spec: &'static ParamSpec,
        /// The module id or kind as written (`filter1`, `svf`, `voice`).
        target: String,
        /// The parameter's address, when the target is a module in the patch (or a built-in): it can be measured.
        addr: Option<ParamAddr>,
    },
    /// A whole module, kind or built-in.
    Whole {
        owner: Owner,
        target: String,
        /// The module's id in the patch, when the target is one.
        module: Option<ModuleId>,
    },
}

#[derive(Clone, Copy)]
enum Owner {
    Kind(ModuleKind),
    Builtin(Builtin),
}

impl Owner {
    fn key(self) -> &'static str {
        match self {
            Owner::Kind(k) => k.key(),
            Owner::Builtin(b) => b.key(),
        }
    }

    fn params(self) -> &'static [ParamSpec] {
        match self {
            Owner::Kind(k) => k.params(),
            Owner::Builtin(b) => b.params(),
        }
    }

    fn name(self) -> &'static str {
        match self {
            Owner::Kind(k) => k.spec().name,
            Owner::Builtin(b) => b.key(),
        }
    }

    fn summary(self) -> &'static str {
        match self {
            Owner::Kind(k) => k.spec().learn.desc,
            Owner::Builtin(Builtin::Voice) => {
                "The voice manager and amp stage: mode (poly, mono, legato), polyphony, glide, pan; voice.amp_env names the amplitude envelope."
            }
            Owner::Builtin(Builtin::Master) => "The output stage after the effects: level in dB.",
        }
    }
}

/// Run `enth explain`.
pub fn run(args: &Args, stdin: &str) -> Res {
    let patch = match &args.patch {
        Some(path) => Some(patchio::load(path)?),
        None => None,
    };
    let (loaded, mut notes) = match patch {
        Some((p, n)) => (Some(p), n),
        None => (None, Vec::new()),
    };
    let subject = resolve(args.what.trim(), loaded.as_ref())?;
    let measured = match (&loaded, &args.patch) {
        (Some(patch), Some(path)) => measure(&subject, patch, path, &args.perf, stdin)?,
        _ => None,
    };
    let out = explain_out(&subject, measured, args.patch.is_some());
    let mut reply = Reply::new(&out).text(format!("{}\n", out.text));
    for n in notes.drain(..) {
        reply = reply.note(&n);
    }
    Ok(reply)
}

// ---------------------------------------------------------------------------------------------------------------
// Resolving what was asked
// ---------------------------------------------------------------------------------------------------------------

fn unknown_kind(name: &str) -> CliError {
    // Reserved S2a kinds are accepted by key but never suggested (`ModuleSpec::hidden`).
    let mut pool: Vec<&str> = ModuleKind::listed().map(ModuleKind::key).collect();
    pool.extend(CATALOGUE.iter().filter(|m| !m.hidden).map(|m| m.id_prefix));
    pool.extend(["voice", "master"]);
    let mut known: Vec<String> = ModuleKind::listed().map(|k| k.key().to_owned()).collect();
    known.extend(["voice".to_owned(), "master".to_owned()]);
    error::invalid(
        "unknown_kind",
        format!("'{name}' is not a parameter, module, module kind or built-in"),
        "`enth modules` lists the kinds; `enth params <kind>` their parameters; a module id like filter1 works with --patch",
    )
    .suggest(nearest(name, pool))
    .known(known)
}

/// The module id, kind or built-in a name stands for, and the module's id when the patch has it.
fn resolve_owner(name: &str, patch: Option<&Patch>) -> Result<(Owner, Option<ModuleId>), CliError> {
    match name {
        "voice" => return Ok((Owner::Builtin(Builtin::Voice), None)),
        "master" => return Ok((Owner::Builtin(Builtin::Master), None)),
        _ => {}
    }
    if let Some(p) = patch
        && let Ok(id) = ModuleId::new(name)
        && let Some((_, m)) = p.find(&id)
    {
        return Ok((Owner::Kind(m.kind), Some(id)));
    }
    if let Some(k) = ModuleKind::from_key(name).or_else(|| aliases::kind_for_name(name)) {
        // With a patch, `filter1` that the patch lacks is an unknown module, not a kind.
        if let Some(p) = patch
            && name.ends_with(|c: char| c.is_ascii_digit())
        {
            let existing = crate::templates::module_ids(p);
            return Err(error::from_patch_error(
                name,
                &PatchError::UnknownModule {
                    id: name.to_owned(),
                    existing,
                },
                Ctx {
                    patch: Some(p),
                    addr: Some(name),
                    ..Ctx::default()
                },
            ));
        }
        return Ok((Owner::Kind(k), None));
    }
    Err(unknown_kind(name))
}

fn resolve(what: &str, patch: Option<&Patch>) -> Result<Subject, CliError> {
    let Some((target, key)) = what.rsplit_once('.') else {
        let (owner, module) = resolve_owner(what, patch)?;
        return Ok(Subject::Whole {
            owner,
            target: what.to_owned(),
            module,
        });
    };
    let (owner, module) = resolve_owner(target, patch)?;
    let Some(spec) = owner.params().iter().find(|s| s.key == key) else {
        let keys: Vec<&'static str> = owner.params().iter().map(|s| s.key).collect();
        let e = PatchError::UnknownParam {
            addr: what.to_owned(),
            owner: owner.key(),
            keys,
        };
        return Err(error::from_patch_error(
            what,
            &e,
            Ctx {
                patch,
                addr: Some(what),
                ..Ctx::default()
            },
        ));
    };
    let addr = match (owner, module) {
        (Owner::Builtin(Builtin::Voice), _) => ParamAddr::new(Target::Voice, key).ok(),
        (Owner::Builtin(Builtin::Master), _) => ParamAddr::new(Target::Master, key).ok(),
        (Owner::Kind(_), Some(id)) => ParamAddr::new(Target::Module(id), key).ok(),
        (Owner::Kind(_), None) => None,
    };
    Ok(Subject::Param {
        owner,
        spec,
        target: target.to_owned(),
        addr,
    })
}

// ---------------------------------------------------------------------------------------------------------------
// The catalogue's facts
// ---------------------------------------------------------------------------------------------------------------

fn scale_name(kind: Kind) -> &'static str {
    match kind {
        Kind::Float { scale, .. } => match scale {
            Scale::Linear => "linear",
            Scale::Log => "log",
            Scale::Power(_) => "power",
        },
        Kind::Int { .. } => "int",
        Kind::Enum(_) => "enum",
        Kind::Toggle => "toggle",
    }
}

fn default_reading(s: &ParamSpec) -> Reading {
    match s.kind {
        Kind::Enum(_) => Reading::Text(
            s.enum_item(s.default)
                .map_or_else(String::new, |i| i.key.to_owned()),
        ),
        Kind::Toggle => Reading::Bool(s.default >= 0.5),
        _ => Reading::Num(f64::from(s.default)),
    }
}

/// The range in real units (`20Hz..20kHz`), or the values (`lp|bp|hp|notch`, `off|on`).
fn range_text(s: &ParamSpec) -> String {
    match s.kind {
        Kind::Enum(items) => items.iter().map(|i| i.key).collect::<Vec<_>>().join("|"),
        Kind::Toggle => "off|on".to_owned(),
        _ => format!("{}..{}", show(Some(s), s.min()), show(Some(s), s.max())),
    }
}

fn default_text(s: &ParamSpec) -> String {
    show(Some(s), s.default)
}

fn bounds(s: &ParamSpec) -> (Option<f64>, Option<f64>) {
    match s.kind {
        Kind::Float { min, max, .. } => (Some(f64::from(min)), Some(f64::from(max))),
        Kind::Int { min, max } => (Some(f64::from(min)), Some(f64::from(max))),
        _ => (None, None),
    }
}

fn moves_of(s: &ParamSpec) -> Option<MovesOut> {
    let path = s.perceptual?;
    Some(MovesOut {
        feature: path.to_owned(),
        meaning: Feature::parse(path).map_or_else(String::new, |f| f.meaning().to_owned()),
    })
}

fn facts(owner: Owner, s: &ParamSpec) -> ParamFactsOut {
    let (min, max) = bounds(s);
    ParamFactsOut {
        key: s.key.to_owned(),
        unit: s.unit.suffix().to_owned(),
        min,
        max,
        default: default_reading(s),
        scale: scale_name(s.kind).to_owned(),
        values: s
            .enum_items()
            .map(|items| items.iter().map(|i| i.key.to_owned()).collect())
            .unwrap_or_default(),
        accepts: match s.kind {
            Kind::Enum(items) => items.iter().map(|i| i.key.to_owned()).collect(),
            Kind::Toggle => vec!["on".to_owned(), "off".to_owned()],
            _ => s.unit.accepted().iter().map(|a| (*a).to_owned()).collect(),
        },
        tier: s.learn.tier.name().to_owned(),
        modulatable: s.flags.modulatable,
        automatable: s.flags.automatable,
        moves: moves_of(s),
        reads_as: knobs::reads_as(owner.key(), s.key).map(str::to_owned),
    }
}

fn brief(owner: Owner, s: &ParamSpec) -> ParamBriefOut {
    let _ = owner;
    ParamBriefOut {
        key: s.key.to_owned(),
        name: s.name.to_owned(),
        range: range_text(s),
        default: default_text(s),
        effect: s.learn.desc.to_owned(),
        moves: s.perceptual.map(str::to_owned),
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Measuring on a patch
// ---------------------------------------------------------------------------------------------------------------

/// The addresses of every parameter of the subject's module, or the one parameter.
fn probe_targets(subject: &Subject, patch: &Patch) -> Vec<ParamAddr> {
    match subject {
        Subject::Param { addr, .. } => addr.iter().cloned().collect(),
        Subject::Whole { owner, module, .. } => {
            let target = match (owner, module) {
                (Owner::Builtin(Builtin::Voice), _) => Some(Target::Voice),
                (Owner::Builtin(Builtin::Master), _) => Some(Target::Master),
                (Owner::Kind(_), Some(id)) => Some(Target::Module(id.clone())),
                (Owner::Kind(_), None) => None,
            };
            target
                .map(|t| {
                    owner
                        .params()
                        .iter()
                        .filter_map(|s| ParamAddr::new(t.clone(), s.key).ok())
                        .filter(|a| patch.spec(a).is_some())
                        .collect()
                })
                .unwrap_or_default()
        }
    }
}

fn measure(
    subject: &Subject,
    patch: &Patch,
    path: &std::path::Path,
    perf_args: &PerfArgs,
    stdin: &str,
) -> Result<Option<MeasuredOut>, CliError> {
    let addrs = probe_targets(subject, patch);
    if addrs.is_empty() {
        return Ok(None);
    }
    let perf = Perf::from_args(perf_args, stdin)?.unwrap_or_else(Perf::default_perf);
    let spec = RenderSpec::new(DEFAULT_SAMPLE_RATE, DEFAULT_SEED);
    let cfg = SensitivityCfg {
        max_renders: PROBE_RENDERS,
        ..SensitivityCfg::default()
    };
    let sens =
        sensitivity(patch, &addrs, &perf.to_render(), &spec, &mut Parallel, &cfg).map_err(|e| {
            error::failed(
                e.code(),
                format!("the patch could not be measured: {e}"),
                "check the patch with `enth describe` and the performance with --perf",
            )
        })?;
    let mut params: Vec<ParamEffectOut> = sens
        .params
        .iter()
        .map(|ps| param_effect(&sens, ps))
        .collect();
    // The ones that matter most first; the catalogue's order among equals.
    params.sort_by(|a, b| b.reach.total_cmp(&a.reach));
    let skipped = sens
        .discrete
        .iter()
        .chain(&sens.unprobed)
        .map(ToString::to_string)
        .collect();
    Ok(Some(MeasuredOut {
        patch: patchio::show(path),
        perf: perf.to_out(),
        renders: sens.renders,
        params,
        skipped,
    }))
}

/// A descriptor reading after a move of `delta` in its compare domain.
fn after_reading(feature: Feature, from: f64, delta: f64) -> f64 {
    if feature.is_pitch() {
        from * (delta / 1200.0).exp2()
    } else if is_log(feature) {
        from * delta.exp()
    } else {
        from + delta
    }
}

fn param_effect(sens: &Sensitivity, ps: &ParamSensitivity) -> ParamEffectOut {
    let spec = ps.spec;
    let mut effects: Vec<(f64, EffectOut)> = Vec::new();
    for e in &ps.effects {
        let f = e.feature;
        let from = sens.base.get(f);
        let reading = |v: f64| crate::cmd::common::reading(f, v);
        match e.change {
            Change::Moved(_) => {
                let (Some(d), Some(from)) = (ps.delta(f), from) else {
                    continue;
                };
                if d == 0.0 {
                    continue;
                }
                effects.push((
                    d.abs() / tolerance_bounds(f).0,
                    EffectOut {
                        feature: f.path().to_owned(),
                        change: if d > 0.0 { "higher" } else { "lower" }.to_owned(),
                        from: Some(reading(from)),
                        to: Some(reading(after_reading(f, from, d))),
                    },
                ));
            }
            Change::Appeared => effects.push((
                f64::INFINITY,
                EffectOut {
                    feature: f.path().to_owned(),
                    change: "appears".to_owned(),
                    from: None,
                    to: None,
                },
            )),
            Change::Vanished => effects.push((
                f64::INFINITY,
                EffectOut {
                    feature: f.path().to_owned(),
                    change: "vanishes".to_owned(),
                    from: from.map(reading),
                    to: None,
                },
            )),
            Change::Absent => {}
        }
    }
    effects.sort_by(|a, b| b.0.total_cmp(&a.0));
    effects.truncate(EFFECTS_SHOWN);
    ParamEffectOut {
        param: ps.addr.to_string(),
        from: show(Some(spec), ps.from),
        to: show(Some(spec), ps.to),
        reach: ps.reach(),
        inert: ps.inert(),
        effects: effects.into_iter().map(|(_, e)| e).collect(),
    }
}

// ---------------------------------------------------------------------------------------------------------------
// The output and its text
// ---------------------------------------------------------------------------------------------------------------

fn reading_text(r: &Reading) -> String {
    match r {
        Reading::Bool(b) => b.to_string(),
        Reading::Num(n) => crate::out::round4(*n).to_string(),
        Reading::Text(t) => t.clone(),
    }
}

fn effect_text(e: &EffectOut) -> String {
    let unit = Feature::parse(&e.feature).map_or("", |f| f.unit().label());
    match (&e.change[..], &e.from, &e.to) {
        ("appears", _, _) => format!("{} appears", e.feature),
        ("vanishes", Some(from), _) => format!(
            "{} vanishes (was {}{}{unit})",
            e.feature,
            reading_text(from),
            if unit.is_empty() { "" } else { " " }
        ),
        (_, Some(from), Some(to)) => format!(
            "{} goes {} ({} to {} {unit})",
            e.feature,
            e.change,
            reading_text(from),
            reading_text(to)
        ),
        _ => format!("{} {}", e.feature, e.change),
    }
}

fn measured_paragraph(m: &MeasuredOut, whole: bool) -> String {
    let mut s = format!("\nMeasured on {} ({} renders):\n", m.patch, m.renders);
    for p in &m.params {
        let verdict = if p.inert {
            "no descriptor notices it in this patch".to_owned()
        } else if p.effects.is_empty() {
            "moves no listed descriptor".to_owned()
        } else {
            p.effects
                .iter()
                .map(effect_text)
                .collect::<Vec<_>>()
                .join("; ")
        };
        if whole {
            let _ = writeln!(
                s,
                "  {}: {} -> {} (reach {}): {verdict}",
                p.param,
                p.from,
                p.to,
                crate::out::round4(p.reach)
            );
        } else {
            let _ = writeln!(
                s,
                "  One just-noticeable step of {}, {} -> {}, in this patch: {verdict}.\n  Its reach is {}: how far the most affected descriptor moves, in units of the difference the game tolerates in it (under 0.25, nothing here notices it).",
                p.param,
                p.from,
                p.to,
                crate::out::round4(p.reach)
            );
        }
    }
    if !m.skipped.is_empty() {
        let _ = writeln!(
            s,
            "  Not measured (a step means nothing for a choice, or over the budget): {}.",
            m.skipped.join(", ")
        );
    }
    s
}

fn param_text(owner: Owner, target: &str, s: &ParamSpec) -> String {
    let mut t = format!(
        "{target}.{} ({}) is a parameter of the {} ({}).\n{}\n",
        s.key,
        s.name,
        owner.name().to_lowercase(),
        owner.key(),
        s.learn.desc
    );
    match s.kind {
        Kind::Enum(items) => {
            let _ = writeln!(
                t,
                "A choice of {}; the default is {}.",
                items.iter().map(|i| i.key).collect::<Vec<_>>().join(", "),
                default_text(s)
            );
        }
        Kind::Toggle => {
            let _ = writeln!(
                t,
                "A switch, {}; the default is {}.",
                range_text(s),
                default_text(s)
            );
        }
        _ => {
            let unit = s.unit.suffix();
            let scale = match s.kind {
                Kind::Float {
                    scale: Scale::Log, ..
                } => {
                    " Its knob is logarithmic: equal steps are equal ratios, so a turn near the bottom is as big a musical move as one near the top."
                }
                Kind::Float {
                    scale: Scale::Power(_),
                    ..
                } => " Its knob is curved, with more resolution toward one end.",
                _ => "",
            };
            let _ = writeln!(
                t,
                "In {}, from {} to {}, default {}.{scale}",
                if unit.is_empty() {
                    "plain numbers"
                } else {
                    unit
                },
                show(Some(s), s.min()),
                show(Some(s), s.max()),
                default_text(s)
            );
            let _ = writeln!(
                t,
                "Write it as {}.",
                s.unit
                    .accepted()
                    .iter()
                    .take(3)
                    .copied()
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
    }
    if let Some(m) = moves_of(s) {
        let _ = writeln!(t, "It mainly moves {}: {}.", m.feature, m.meaning);
    }
    if let Some(r) = knobs::reads_as(owner.key(), s.key) {
        let _ = writeln!(t, "{r}.");
    }
    let who = match (s.flags.modulatable, s.flags.automatable) {
        (true, true) => {
            "Routes can modulate it (an LFO or envelope into this address), and host automation can drive it."
        }
        (true, false) => "Routes can modulate it.",
        (false, true) => "Host automation can drive it; routes cannot.",
        (false, false) => "It is fixed per voice: neither routes nor automation move it.",
    };
    t.push_str(who);
    t.push('\n');
    t
}

fn whole_text(owner: Owner, target: &str, module: Option<&ModuleId>) -> String {
    let mut t = match (owner, module) {
        (Owner::Builtin(b), _) => format!("{}: {}\n", b.key(), owner.summary()),
        (Owner::Kind(k), Some(id)) => format!(
            "{id} is a {} ({}). {}\n",
            owner.name().to_lowercase(),
            k.key(),
            owner.summary()
        ),
        (Owner::Kind(k), None) => format!(
            "{target}: the {} module kind ({}). {}\n",
            owner.name().to_lowercase(),
            k.key(),
            owner.summary()
        ),
    };
    t.push_str("Parameters:\n");
    for s in owner.params() {
        let _ = writeln!(
            t,
            "  {}: {}, default {}. {}{}",
            s.key,
            range_text(s),
            default_text(s),
            s.learn.desc,
            s.perceptual
                .map_or_else(String::new, |p| format!(" [moves {p}]"))
        );
    }
    t
}

fn explain_out(subject: &Subject, measured: Option<MeasuredOut>, asked_patch: bool) -> ExplainOut {
    let (what, subject_text, owner, name, param, params, mut text) = match subject {
        Subject::Param {
            owner,
            spec,
            target,
            addr,
        } => {
            let shown = addr
                .as_ref()
                .map_or_else(|| format!("{target}.{}", spec.key), ToString::to_string);
            (
                "param",
                shown,
                *owner,
                spec.name.to_owned(),
                Some(facts(*owner, spec)),
                Vec::new(),
                param_text(*owner, target, spec),
            )
        }
        Subject::Whole {
            owner,
            target,
            module,
        } => (
            match (owner, module) {
                (Owner::Builtin(_), _) => "builtin",
                (Owner::Kind(_), Some(_)) => "module",
                (Owner::Kind(_), None) => "kind",
            },
            module
                .as_ref()
                .map_or_else(|| target.clone(), ToString::to_string),
            *owner,
            owner.name().to_owned(),
            None,
            owner.params().iter().map(|s| brief(*owner, s)).collect(),
            whole_text(*owner, target, module.as_ref()),
        ),
    };
    let whole = param.is_none();
    if let Some(m) = &measured {
        text.push_str(&measured_paragraph(m, whole));
    }
    let hint = match (&measured, asked_patch, subject) {
        (Some(_), _, Subject::Param { .. }) => {
            "`enth explain <module id> --patch <patch>` ranks every parameter of a module; `enth apply <patch> --set <addr>=<value>` changes one"
                .to_owned()
        }
        (Some(_), _, Subject::Whole { .. }) => {
            "`enth explain <addr> --patch <patch>` details one parameter; `enth apply <patch> --set <addr>=<value>` changes it"
                .to_owned()
        }
        (
            None,
            true,
            Subject::Whole {
                module: None,
                owner: Owner::Kind(_),
                ..
            }
            | Subject::Param { addr: None, .. },
        ) => {
            "a kind has no module in the patch to measure: name a module id (filter1) or an address (filter1.cutoff)"
                .to_owned()
        }
        (None, false, _) => {
            "add --patch <patch> to measure what it does in your patch; `enth params <kind>` lists units and ranges"
                .to_owned()
        }
        (None, true, _) => {
            "nothing to measure here; `enth params <kind>` lists units and ranges".to_owned()
        }
    };
    ExplainOut {
        schema: ids::EXPLAIN.to_owned(),
        what: what.to_owned(),
        subject: subject_text,
        kind: owner.key().to_owned(),
        name,
        summary: match subject {
            Subject::Param { spec, .. } => spec.learn.desc.to_owned(),
            Subject::Whole { owner, .. } => owner.summary().to_owned(),
        },
        param,
        params,
        text: text.trim_end().to_owned(),
        measured,
        hint,
    }
}
