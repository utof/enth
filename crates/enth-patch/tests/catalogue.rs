//! The catalogue as a persisted contract: an insta snapshot of every spec, and properties that every `ParamSpec`
//! in it must satisfy (parse ∘ display round trips, monotonic and invertible scales).
//!
//! A failing snapshot means a spec changed. That is a patch-format change unless proven otherwise: rename, unit,
//! range, scale, default or order changes need a migration (design.md §2); additions need a default that keeps
//! old patches sounding the same. Accept with `INSTA_UPDATE=always cargo test -p enth-patch --test catalogue`
//! (or `cargo insta review`) and say why in the commit message.

use enth_params::{EnumItem, Flags, Kind, LearnMeta, ParamSpec, Scale, Smoothing};
use enth_patch::catalogue::{Builtin, CATALOGUE, MACRO_VALUE, ModuleKind, ModuleSpec, SvfParam};
use proptest::prelude::*;
use serde_json::{Value, json};

/// Every spec in the catalogue, with a label (`osc.semi`, `voice.glide`, `macro.value`, `route.depth`).
fn all_specs() -> Vec<(String, &'static ParamSpec)> {
    let mut v = Vec::new();
    for m in &CATALOGUE {
        for p in m.params {
            v.push((format!("{}.{}", m.key, p.key), p));
        }
    }
    for b in Builtin::ALL {
        for p in b.params() {
            v.push((format!("{}.{}", b.key(), p.key), p));
        }
    }
    v.push(("macro.value".to_owned(), &MACRO_VALUE));
    v.push(("route.depth".to_owned(), &enth_params::DEPTH));
    v
}

fn spec_json(p: &ParamSpec) -> Value {
    // Exhaustive destructuring: a new ParamSpec field fails to compile here until the snapshot covers it.
    let ParamSpec {
        key,
        name,
        unit,
        kind,
        default,
        smoothing,
        flags,
        learn,
        perceptual,
    } = *p;
    let Flags {
        modulatable,
        automatable,
        structural,
        periodic,
        audio,
    } = flags;
    let LearnMeta { tier, desc } = learn;
    let kind = match kind {
        Kind::Float { min, max, scale } => {
            let (scale, k) = match scale {
                Scale::Linear => ("linear", None),
                Scale::Log => ("log", None),
                Scale::Power(k) => ("power", Some(k)),
            };
            json!({ "type": "float", "min": min, "max": max, "scale": scale, "k": k })
        }
        Kind::Int { min, max } => json!({ "type": "int", "min": min, "max": max }),
        Kind::Enum(items) => {
            let items: Vec<Value> = items
                .iter()
                .map(|&EnumItem { key, name, desc }| json!({ "key": key, "name": name, "desc": desc }))
                .collect();
            json!({ "type": "enum", "items": items })
        }
        Kind::Toggle => json!({ "type": "toggle" }),
    };
    let smoothing = match smoothing {
        Smoothing::None => json!(null),
        Smoothing::Linear { ms } => json!({ "linear_ms": ms }),
        Smoothing::OnePole { ms } => json!({ "one_pole_ms": ms }),
    };
    // `audio` (S2a) is written only when set, so the snapshot of the P1 kinds, none of which has it, stays as it was.
    let mut flags = json!({ "modulatable": modulatable, "automatable": automatable, "structural": structural, "periodic": periodic });
    if audio {
        flags["audio"] = json!(true);
    }
    json!({
        "key": key,
        "name": name,
        "unit": unit.name(),
        "kind": kind,
        "default": default,
        "default_text": p.display(default).to_string(),
        "smoothing": smoothing,
        "flags": flags,
        "tier": tier.name(),
        "desc": desc,
        "perceptual": perceptual,
    })
}

fn module_json(m: &ModuleSpec) -> Value {
    let ModuleSpec {
        kind,
        key,
        name,
        id_prefix,
        section,
        time_based,
        params,
        learn: LearnMeta { tier, desc },
        hidden: _,
    } = *m;
    json!({
        "index": kind as u8,
        "key": key,
        "name": name,
        "id_prefix": id_prefix,
        "section": section.key(),
        "time_based": time_based,
        "tier": tier.name(),
        "desc": desc,
        "params": params.iter().map(spec_json).collect::<Vec<_>>(),
    })
}

/// The listed kinds (the hidden S2a ones are in `reserved_kinds`'s snapshot until INT-S2a clears their flag, when a
/// kind moves from that snapshot to this one).
fn catalogue_json() -> Value {
    let modules: Vec<Value> = CATALOGUE
        .iter()
        .filter(|m| !m.hidden)
        .map(module_json)
        .collect();
    let builtins: Vec<Value> = Builtin::ALL
        .iter()
        .map(|b| json!({ "key": b.key(), "params": b.params().iter().map(spec_json).collect::<Vec<_>>() }))
        .collect();
    json!({
        "modules": modules,
        "builtins": builtins,
        "macro": spec_json(&MACRO_VALUE),
        "route_depth": spec_json(&enth_params::DEPTH),
        "limits": {
            "generators": enth_patch::model::MAX_GENERATORS,
            "voice_fx": enth_patch::model::MAX_VOICE_FX,
            "fx": enth_patch::model::MAX_FX,
            "modulators": enth_patch::model::MAX_MODULATORS,
            "routes": enth_patch::model::MAX_ROUTES,
            "macros": enth_patch::model::MACROS,
            "slots": enth_patch::model::SLOTS,
        },
        "reserved_ids": enth_patch::addr::RESERVED_IDS,
    })
}

#[test]
fn catalogue() {
    insta::assert_json_snapshot!(catalogue_json());
}

/// The reserved S2a kinds (lane S0d, F0m): fixed discriminants, keys, id prefixes and sections; placeholder tables
/// that lane CAT replaces before F1m (each change shows up here).
#[test]
fn reserved_kinds() {
    let reserved: Vec<Value> = CATALOGUE
        .iter()
        .filter(|m| m.hidden)
        .map(module_json)
        .collect();
    insta::assert_json_snapshot!(reserved);
}

#[test]
fn every_kind_has_a_spec_in_order() {
    assert_eq!(CATALOGUE.len(), ModuleKind::ALL.len());
    for (i, &k) in ModuleKind::ALL.iter().enumerate() {
        assert_eq!(k as usize, i);
        assert_eq!(CATALOGUE[i].kind, k);
        assert_eq!(ModuleKind::from_key(k.key()), Some(k));
    }
}

#[test]
fn defaults_are_valid_and_display_round_trips() {
    for (label, p) in all_specs() {
        assert!(
            p.is_valid(p.default),
            "{label}: default {} is not valid",
            p.default
        );
        let text = p.display(p.default).to_string();
        assert_eq!(p.parse(&text), Ok(p.default), "{label}: '{text}'");
    }
}

#[test]
fn one_octave_on_the_filter_cutoff_is_10_034_percent() {
    let cutoff = SvfParam::Cutoff.spec();
    let d = cutoff.depth_from_text(cutoff.default, "+1oct").unwrap();
    assert!((d - 10.034).abs() < 5e-4, "{d}");
}

#[test]
fn scales_are_monotonic_on_a_fine_grid() {
    for (label, p) in all_specs() {
        let mut prev_x = f32::NEG_INFINITY;
        for i in 0..=4096 {
            let n = i as f32 / 4096.0;
            let x = p.from_norm(n);
            assert!(x >= prev_x, "{label}: from_norm not monotonic at {n}");
            assert!(p.is_valid(x), "{label}: from_norm({n}) = {x} is not valid");
            prev_x = x;
        }
        assert_eq!(p.from_norm(0.0), p.min(), "{label}");
        assert_eq!(p.from_norm(1.0), p.max(), "{label}");
        assert_eq!(p.to_norm(p.min()), 0.0, "{label}");
        assert_eq!(p.to_norm(p.max()), 1.0, "{label}");
    }
}

fn spec_and_value() -> impl Strategy<Value = (String, &'static ParamSpec, f32)> {
    let specs = all_specs();
    (0..specs.len(), 0.0f32..=1.0, any::<u32>()).prop_map(move |(i, n, bits)| {
        let (label, p) = specs[i].clone();
        // Half the cases: a value on the knob; half: any valid value reachable by bits.
        let v = if bits % 2 == 0 {
            p.from_norm(n)
        } else {
            p.clamp(p.min() + (p.max() - p.min()) * (bits as f32 / u32::MAX as f32))
        };
        (label, p, v)
    })
}

proptest! {
    /// `parse(display(x)) == x` bit for bit, for every spec in the catalogue and every valid value.
    #[test]
    fn parse_display_round_trips((label, p, x) in spec_and_value()) {
        prop_assume!(p.is_valid(x));
        let text = p.display(x).to_string();
        prop_assert_eq!(p.parse(&text).map(f32::to_bits), Ok(x.to_bits()), "{}: {} -> '{}'", label, x, text);
    }

    /// The four-digit display used by the CLI re-parses within the range and re-displays identically.
    #[test]
    fn rounded_display_is_stable((label, p, x) in spec_and_value()) {
        prop_assume!(p.is_valid(x));
        let text = p.display_rounded(x, 4).to_string();
        let back = p.unit.parse(&text).ok().or_else(|| p.parse(&text).ok());
        let back = back.unwrap_or_else(|| panic!("{label}: '{text}' does not parse"));
        prop_assert_eq!(p.display_rounded(back, 4).to_string(), text);
    }

    /// Plain → normalised → plain: monotonic, and exact up to rounding the normalised value to f32.
    #[test]
    fn mapping_is_monotonic_and_invertible((label, p, x) in spec_and_value(), (_l2, _p2, y) in spec_and_value()) {
        prop_assume!(p.is_valid(x));
        let y = p.clamp(y);
        let (lo, hi) = if x <= y { (x, y) } else { (y, x) };
        prop_assert!(p.to_norm(lo) <= p.to_norm(hi), "{}: to_norm not monotonic", label);
        let back = p.from_norm(p.to_norm(x));
        if p.is_discrete() {
            prop_assert_eq!(back, x, "{}", label);
        } else {
            // Re-mapping lands within one f32 step of the same normalised value.
            let (n, m) = (p.to_norm(x), p.to_norm(back));
            let step = f32::EPSILON * 2.0;
            prop_assert!((n - m).abs() <= step, "{}: {} -> {} -> {} ({} vs {})", label, x, n, back, n, m);
        }
    }
}

/// A parameter's range for the markdown tables.
fn md_range(p: &ParamSpec) -> String {
    match p.kind {
        Kind::Enum(items) => items
            .iter()
            .map(|i| format!("`{}`", i.key))
            .collect::<Vec<_>>()
            .join(", "),
        Kind::Toggle => "off, on".to_owned(),
        _ => format!("{} … {}", p.display(p.min()), p.display(p.max())),
    }
}

/// A parameter's scale for the markdown tables.
fn md_scale(p: &ParamSpec) -> String {
    match p.kind {
        Kind::Float {
            scale: Scale::Power(k),
            ..
        } => format!("power {k}"),
        Kind::Float { scale, .. } => scale.name().to_owned(),
        Kind::Int { .. } => "int".to_owned(),
        Kind::Enum(_) => "enum".to_owned(),
        Kind::Toggle => "toggle".to_owned(),
    }
}

/// A parameter's flags for the markdown tables.
fn md_flags(p: &ParamSpec) -> String {
    let mut f = Vec::new();
    if p.flags.modulatable {
        f.push("mod");
    }
    if p.flags.automatable {
        f.push("auto");
    }
    if p.flags.structural {
        f.push("structural");
    }
    if p.flags.periodic {
        f.push("periodic");
    }
    if p.flags.audio {
        f.push("audio");
    }
    f.join(", ")
}

/// Prints one markdown table: a title, then a row per parameter.
#[allow(
    clippy::print_stdout,
    reason = "the two ignored markdown printers below call it; printing is their job"
)]
fn md_table(title: &str, params: &[ParamSpec]) {
    println!("### {title}\n");
    println!("| Key | Unit | Range | Scale | Default | Flags | Tier | What it does |");
    println!("| --- | --- | --- | --- | --- | --- | --- | --- |");
    for p in params {
        println!(
            "| `{}` | {} | {} | {} | {} | {} | {} | {} |",
            p.key,
            p.unit.name(),
            md_range(p),
            md_scale(p),
            p.display(p.default),
            md_flags(p),
            p.learn.tier.name(),
            p.learn.desc
        );
    }
    println!();
}

/// Prints the catalogue as the markdown table in `docs/dsp/catalogue.md`. Regenerate with
/// `cargo test -p enth-patch --test catalogue -- --ignored --nocapture print_markdown`.
#[test]
#[ignore = "prints docs/dsp/catalogue.md's tables; run by hand"]
fn print_markdown() {
    // Reserved S2a kinds stay out of docs/dsp/catalogue.md until INT-S2a lists them (lane CAT regenerates it).
    for m in CATALOGUE.iter().filter(|m| !m.hidden) {
        md_table(
            &format!(
                "`{}` — {} ({}, ids `{}1`, `{}2`…)",
                m.key,
                m.name,
                m.section.key(),
                m.id_prefix,
                m.id_prefix
            ),
            m.params,
        );
    }
    md_table("`voice` (built-in)", Builtin::Voice.params());
    md_table("`master` (built-in)", Builtin::Master.params());
    md_table(
        "Macro knob and route depth",
        &[MACRO_VALUE, enth_params::DEPTH],
    );
}

/// Prints the reserved S2a kinds' tables, one per kind, for the "Parameters" section of each `docs/dsp/<kind>.md`
/// (lane CAT, F1m). Regenerate with `cargo test -p enth-patch --test catalogue -- --ignored --nocapture
/// print_reserved_markdown`.
#[test]
#[ignore = "prints the reserved kinds' tables for docs/dsp/<kind>.md; run by hand"]
fn print_reserved_markdown() {
    for m in CATALOGUE.iter().filter(|m| m.hidden) {
        md_table(
            &format!(
                "`{}` — {} ({}, ids `{}1`, `{}2`…; {} parameters)",
                m.key,
                m.name,
                m.section.key(),
                m.id_prefix,
                m.id_prefix,
                m.params.len()
            ),
            m.params,
        );
    }
}
