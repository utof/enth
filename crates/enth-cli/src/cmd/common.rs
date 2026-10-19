//! Pieces several commands share: resolving `--ops`/`--set`, describing changes in real units, and turning
//! descriptors into output objects.

use std::collections::BTreeMap;

use enth_analysis::{Descriptors, Feature};
use enth_params::{ParamSpec, Scale, Unit};
use enth_patch::addr::ModSource;
use enth_patch::catalogue::ModuleKind;
use enth_patch::{Op, ParamAddr, Patch, apply, apply_all, wire};
use serde_json::Value;

use crate::cli::Edits;
use crate::error::{self, CliError, Ctx};
use crate::out::round4;
use crate::schema::{
    ChangeOut, DescribeOut, DetuneOut, EnvelopeOut, FilterSweepOut, LoudnessOut, ModulationOut,
    PerfOut, PitchOut, Reading, SpectrumOut, TonalOut, TremoloOut, ids,
};

/// A parameter value as a unit string with 4 significant digits (`1.2kHz`, `40ms`, `saw`, `on`).
pub fn show(spec: Option<&ParamSpec>, v: f32) -> String {
    match spec {
        Some(s) => s.display_rounded(v, 4).to_string(),
        None => round4(f64::from(v)).to_string(),
    }
}

fn depth_text(d: f32) -> String {
    format!("{}%", round4(f64::from(d)))
}

/// The text of an `--ops` argument: inline, `@file`, or `-` for stdin.
fn ops_text(src: &str, stdin: &str) -> Result<String, CliError> {
    let s = src.trim();
    if s == "-" {
        return Ok(stdin.to_owned());
    }
    if let Some(path) = s.strip_prefix('@') {
        let bytes = crate::patchio::read_bytes(std::path::Path::new(path))?;
        return Ok(String::from_utf8_lossy(&bytes).into_owned());
    }
    Ok(s.to_owned())
}

/// Applied edits: the ops in order, the patch after them, and the ops that undo them.
pub struct Edited {
    /// The ops applied, resolved.
    pub ops: Vec<Op>,
    /// The patch after them.
    pub after: Patch,
    /// The inverse transaction (apply to `after` to get the original back).
    pub inverse: Vec<Op>,
}

impl Edited {
    /// The applied ops, in exact wire form.
    pub fn ops_json(&self, before: &Patch) -> Vec<Value> {
        as_array(wire::to_json(before, &self.ops))
    }

    /// The inverse ops, in exact wire form.
    pub fn inverse_json(&self) -> Vec<Value> {
        as_array(wire::to_json(&self.after, &self.inverse))
    }
}

/// A JSON array's elements.
pub fn as_array(v: Value) -> Vec<Value> {
    match v {
        Value::Array(a) => a,
        other => vec![other],
    }
}

/// Resolve `--ops` (first) and `--set` (then) against `patch` and apply them as one transaction.
pub fn resolve_edits(patch: &Patch, edits: &Edits, stdin: &str) -> Result<Edited, CliError> {
    let mut scratch = patch.clone();
    let mut ops: Vec<Op> = Vec::new();
    if let Some(src) = &edits.ops {
        let text = ops_text(src, stdin)?;
        let parsed = wire::resolve_str(&scratch, &text).map_err(|inv| {
            error::from_invalid(
                &inv,
                Ctx {
                    patch: Some(&scratch),
                    ..Ctx::default()
                },
            )
        })?;
        apply_all(&mut scratch, &parsed).map_err(|e| {
            error::from_patch_error(&format!("ops[{}]", e.index), &e.error, Ctx::default())
        })?;
        ops.extend(parsed);
    }
    if !edits.set.is_empty() {
        let pairs = edits.set.join(",");
        let parsed = wire::set_ops(&scratch, &pairs).map_err(|inv| {
            error::from_invalid(
                &inv,
                Ctx {
                    patch: Some(&scratch),
                    set_pairs: &edits.set,
                    ..Ctx::default()
                },
            )
        })?;
        apply_all(&mut scratch, &parsed).map_err(|e| {
            error::from_patch_error(&format!("set[{}]", e.index), &e.error, Ctx::default())
        })?;
        ops.extend(parsed);
    }
    let mut after = patch.clone();
    let inverse = apply_all(&mut after, &ops).map_err(|e| {
        error::from_patch_error(&format!("ops[{}]", e.index), &e.error, Ctx::default())
    })?;
    Ok(Edited {
        ops,
        after,
        inverse,
    })
}

/// What each op changes, in real units, applied in order from `before`.
pub fn changes(before: &Patch, ops: &[Op]) -> Vec<ChangeOut> {
    let mut p = before.clone();
    let mut out = Vec::with_capacity(ops.len());
    for op in ops {
        out.push(change(&p, op));
        let _ = apply(&mut p, op);
    }
    out
}

/// The range a source swings over, and how to say "at its full swing": envelopes, velocity, the mod wheel and
/// macros run 0 to 1, an LFO -1 to 1 unless it is unipolar, `key` is `(note - 60) / 120`.
fn source_swing(p: &Patch, src: &ModSource) -> (f64, String) {
    match src {
        ModSource::Module(id) => match p.find(id) {
            Some((_, m)) if m.kind == ModuleKind::Lfo => {
                let spec = m
                    .kind
                    .spec()
                    .params
                    .iter()
                    .position(|s| s.key == "polarity")
                    .map(|i| &m.kind.spec().params[i]);
                let unipolar = spec
                    .zip(m.get("polarity"))
                    .and_then(|(s, v)| s.enum_item(v))
                    .is_some_and(|item| item.key == "unipolar");
                if unipolar {
                    (0.0, format!("at the top of {id} (unipolar, 0 to 1)"))
                } else {
                    (-1.0, format!("as {id} swings -1 to 1 (bipolar)"))
                }
            }
            _ => (0.0, format!("at full {id} (0 to 1)")),
        },
        ModSource::Key => (
            0.0,
            "at key 1.0, ten octaves above C4 (key is (note - 60) / 120)".to_owned(),
        ),
        other => (0.0, format!("at full {other} (0 to 1)")),
    }
}

/// The change from `a` to `b` of a parameter, as a musical interval where that is what it is.
fn interval(spec: &ParamSpec, a: f32, b: f32) -> String {
    let (a, b) = (f64::from(a), f64::from(b));
    match (spec.unit, spec.scale()) {
        (_, Some(Scale::Log)) if a > 0.0 && b > 0.0 => {
            format!(
                "{:+} oct",
                round4(enth_analysis::units::cents(b, a) / 1200.0)
            )
        }
        (Unit::Semitones | Unit::Cents | Unit::Octaves, _) => {
            format!("{:+} {}", round4(b - a), spec.unit.suffix())
        }
        _ => format!("{:+}{}", round4(b - a), spec.unit.suffix()),
    }
}

/// What a route's depth does to its destination (eval finding 2: `+12st` became 12.5 % silently): the value at
/// the source's full swing, as an interval, and the range the percentage is of. Routes move the destination's
/// normalised position by depth/100 times the source, clamped to the range (design.md §1).
pub fn route_means(p: &Patch, src: &ModSource, dst: &ParamAddr, depth: f32) -> Option<String> {
    let spec = p.spec(dst)?;
    let base = p.value(dst)?;
    let (min, max) = (spec.min(), spec.max());
    let n0 = f64::from(spec.to_norm(base));
    let d = f64::from(depth) / 100.0;
    let at = |s: f64| spec.from_norm((n0 + d * s) as f32);
    let clamped = |s: f64| !(0.0..=1.0).contains(&(n0 + d * s));
    let (lo_src, how) = source_swing(p, src);
    let hi = at(1.0);
    let range = format!(
        "{}% of {dst}'s {}..{} {} range",
        round4(f64::from(depth)),
        show(Some(spec), min),
        show(Some(spec), max),
        spec.scale().map_or("linear", Scale::name)
    );
    let clip = if clamped(1.0) || (lo_src < 0.0 && clamped(lo_src)) {
        ", clamped at the end of the range"
    } else {
        ""
    };
    Some(if lo_src < 0.0 {
        let lo = at(lo_src);
        format!(
            "{dst} swings {}..{} around {} ({} / {}) {how}{clip}; {range}",
            show(Some(spec), lo),
            show(Some(spec), hi),
            show(Some(spec), base),
            interval(spec, base, lo),
            interval(spec, base, hi),
        )
    } else {
        format!(
            "{dst} goes {} -> {} ({}) {how}{clip}; {range}",
            show(Some(spec), base),
            show(Some(spec), hi),
            interval(spec, base, hi),
        )
    })
}

fn change(p: &Patch, op: &Op) -> ChangeOut {
    let c = |op: &str, target: String, from: Option<String>, to: Option<String>| ChangeOut {
        op: op.to_owned(),
        target,
        from,
        to,
        means: None,
    };
    let on_off = |b: bool| if b { "on" } else { "off" }.to_owned();
    let route_depth = |src: &str, dst: &str| {
        p.routes
            .iter()
            .find(|r| r.src.to_string() == src && r.dst.to_string() == dst)
            .map(|r| depth_text(r.depth))
    };
    match op {
        Op::SetParam { addr, value } => {
            let spec = p.spec(addr);
            c(
                "set",
                addr.to_string(),
                p.value(addr).map(|v| show(spec, v)),
                Some(show(spec, *value)),
            )
        }
        Op::AddModule { at, module } => c(
            "add",
            module.id.to_string(),
            None,
            Some(format!("{} at {at}", module.kind.key())),
        ),
        Op::RemoveModule { id } => c(
            "remove",
            id.to_string(),
            p.find(id).map(|(_, m)| m.kind.key().to_owned()),
            None,
        ),
        Op::Move { id, to } => c(
            "move",
            id.to_string(),
            p.find(id).map(|(pos, _)| pos.to_string()),
            Some(to.to_string()),
        ),
        Op::InsertRoute { route, .. } => ChangeOut {
            means: route_means(p, &route.src, &route.dst, route.depth),
            ..c(
                "route",
                format!("{} -> {}", route.src, route.dst),
                None,
                Some(depth_text(route.depth)),
            )
        },
        Op::RemoveRoute { route } => c(
            "unroute",
            route.to_string(),
            route_depth(&route.src.to_string(), &route.dst.to_string()),
            None,
        ),
        Op::SetDepth { route, depth } => ChangeOut {
            means: route_means(p, &route.src, &route.dst, *depth),
            ..c(
                "route",
                route.to_string(),
                route_depth(&route.src.to_string(), &route.dst.to_string()),
                Some(depth_text(*depth)),
            )
        },
        Op::SetEnabled { id, enabled } => c(
            "enable",
            id.to_string(),
            p.find(id).map(|(_, m)| on_off(m.enabled)),
            Some(on_off(*enabled)),
        ),
        Op::SetAmpEnv { id } => c(
            "amp_env",
            "voice.amp_env".to_owned(),
            Some(p.voice.amp_env.to_string()),
            Some(id.to_string()),
        ),
        Op::SetMacro { index, value } => c(
            "macro",
            format!("macro{}", index.one_based()),
            p.macros
                .get(index.get())
                .map(|m| round4(f64::from(m.value)).to_string()),
            Some(round4(f64::from(*value)).to_string()),
        ),
        Op::SetMacroName { index, name } => c(
            "macro",
            format!("macro{}.name", index.one_based()),
            p.macros.get(index.get()).and_then(|m| m.name.clone()),
            name.clone(),
        ),
        Op::SetSlot { slot, addr } => c(
            "slot",
            format!("slot{}", slot.one_based()),
            p.slots
                .get(slot.get())
                .and_then(|s| s.as_ref().map(ToString::to_string)),
            addr.as_ref().map(ToString::to_string),
        ),
        Op::SetLabel { id, label } => c(
            "label",
            id.to_string(),
            p.find(id).and_then(|(_, m)| m.label.clone()),
            label.clone(),
        ),
        Op::SetMeta { meta } => {
            let fields = [
                ("name", p.meta.name.clone(), meta.name.clone()),
                ("author", p.meta.author.clone(), meta.author.clone()),
                (
                    "description",
                    p.meta.description.clone(),
                    meta.description.clone(),
                ),
                ("tags", p.meta.tags.join(" "), meta.tags.join(" ")),
            ];
            let changed: Vec<&(&str, String, String)> =
                fields.iter().filter(|(_, a, b)| a != b).collect();
            let side = |after: bool| {
                changed
                    .iter()
                    .map(|(k, a, b)| format!("{k}: {}", if after { b } else { a }))
                    .collect::<Vec<_>>()
                    .join("; ")
            };
            c(
                "meta",
                "meta".to_owned(),
                Some(side(false)),
                Some(side(true)),
            )
        }
        other => c(other.name(), String::new(), None, None),
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Descriptors
// ---------------------------------------------------------------------------------------------------------------

/// A descriptor reading for output: booleans as booleans.
pub fn reading(feature: Feature, v: f64) -> Reading {
    if feature.unit() == enth_analysis::units::Unit::Bool {
        Reading::Bool(v != 0.0)
    } else {
        Reading::Num(v)
    }
}

/// The paths every `step` echoes, whatever `--expect` names.
pub const HEADLINE: &[&str] = &[
    "pitch.note",
    "spectrum.centroid_hz",
    "envelope.attack_ms",
    "envelope.sustain_db",
    "envelope.duration_ms",
    "loudness.peak_dbfs",
    "loudness.clipping",
];

/// Every reading by path, plus `pitch.note` and `tonal.note`; null when not measured.
pub fn readings(d: &Descriptors) -> BTreeMap<String, Option<Reading>> {
    let mut m: BTreeMap<String, Option<Reading>> = Feature::ALL
        .iter()
        .map(|f| (f.path().to_owned(), d.get(*f).map(|v| reading(*f, v))))
        .collect();
    m.insert(
        "pitch.note".to_owned(),
        d.pitch.as_ref().map(|p| Reading::Text(p.note.clone())),
    );
    m.insert(
        "tonal.note".to_owned(),
        d.tonal.as_ref().map(|t| Reading::Text(t.note.clone())),
    );
    m
}

/// Why each null descriptor is null.
pub fn missing(d: &Descriptors) -> BTreeMap<String, String> {
    Feature::ALL
        .iter()
        .filter(|f| d.get(**f).is_none())
        .map(|f| {
            let why = if d.silent {
                "the sound is silent (sample peak below -80 dBFS)"
            } else {
                f.missing_reason()
            };
            (f.path().to_owned(), why.to_owned())
        })
        .collect()
}

/// The `enth.describe/2` object.
pub fn describe_out(source: &str, sample_rate: u32, perf: PerfOut, d: &Descriptors) -> DescribeOut {
    let lv = &d.loudness;
    DescribeOut {
        schema: ids::DESCRIBE.to_owned(),
        source: source.to_owned(),
        dur_s: d.dur_s,
        sample_rate,
        silent: d.silent,
        perf,
        pitch: d.pitch.as_ref().map(|p| PitchOut {
            f0_hz: p.f0_hz,
            note: p.note.clone(),
            cents_dev: p.cents_dev,
            stability: p.stability,
        }),
        tonal: d.tonal.as_ref().map(|t| TonalOut {
            f0_hz: t.f0_hz,
            note: t.note.clone(),
            cents_dev: t.cents_dev,
        }),
        envelope: d.envelope.map(|e| EnvelopeOut {
            attack_ms: e.attack_ms,
            decay_ms: e.decay_ms,
            sustain_db: e.sustain_db,
            release_ms: e.release_ms,
            duration_ms: e.duration_ms,
            ripple_db: e.ripple_db,
            beat_corrected: e.beat_corrected,
            release_extrapolated: e.release_extrapolated,
            duration_truncated: e.duration_truncated,
        }),
        spectrum: d.spectrum.map(|s| SpectrumOut {
            centroid_hz: s.centroid_hz,
            centroid_track_hz: s.centroid_track_hz.map(|t| t.to_vec()),
        }),
        noise_ratio: d.noise_ratio,
        detune: d.detune.map(|b| DetuneOut {
            beating_hz: b.beating_hz,
            depth_db: b.depth_db,
        }),
        modulation: ModulationOut {
            amp_lfo: d.modulation.amp_lfo.map(|t| TremoloOut {
                hz: t.hz,
                depth_db: t.depth_db,
            }),
            filter_sweep: d.modulation.filter_sweep.map(|s| FilterSweepOut {
                kind: s.kind.name().to_owned(),
                dir: s.dir.map(|d| d.name().to_owned()),
                ms: s.ms,
                hz: s.hz,
                depth_oct: s.depth_oct,
            }),
        },
        loudness: LoudnessOut {
            lufs_i: lv.lufs_i,
            peak_dbfs: lv.peak_dbfs,
            true_peak_dbtp: lv.true_peak_dbtp,
            clipped_pct: lv.clipped_pct,
            clipping: lv.clipping,
        },
        missing: missing(d),
    }
}
