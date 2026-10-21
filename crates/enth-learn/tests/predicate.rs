//! Lane PLY's predicate grammar (`enth_learn::predicate`): print → parse round trips for every valid predicate
//! (proptest, fixed seed), no panic on any input, and the acceptance check that every unit form the CLI accepts
//! parses in a predicate and reads the value `ParamSpec::parse` reads.

use enth_learn::check::{Cmp, Knob, Plain, Predicate, TouchLog};
use enth_learn::pack::KindKey;
use enth_params::{ParamSpec, Unit};
use enth_patch::{
    Builtin, CATALOGUE, MacroIndex, ModSource, ModuleId, ModuleKind, ParamAddr, Patch, RouteAddr,
    Target,
};
use proptest::prelude::*;

/// The module id a lesson would give a kind (the starter patch's names).
fn id_for(kind: ModuleKind, n: u8) -> ModuleId {
    let prefix = match kind.key() {
        "svf" => "filter",
        "adsr" => "env",
        other => other,
    };
    ModuleId::new(&format!("{prefix}{n}")).unwrap()
}

/// Every parameter a predicate can name: (address, spec) for module kinds and the built-ins.
fn params() -> Vec<(ParamAddr, &'static ParamSpec)> {
    let mut out = Vec::new();
    for m in CATALOGUE {
        for spec in m.kind.params() {
            out.push((
                ParamAddr::new(Target::Module(id_for(m.kind, 1)), spec.key).unwrap(),
                spec,
            ));
        }
    }
    for (target, b) in [
        (Target::Voice, Builtin::Voice),
        (Target::Master, Builtin::Master),
    ] {
        for spec in b.params() {
            out.push((ParamAddr::new(target.clone(), spec.key).unwrap(), spec));
        }
    }
    out
}

/// A value as an author might write it for `spec` at normalised position `n`: its display, a plain number, or the
/// number with the unit's suffix; an enum key; on/off.
fn value_text(spec: &ParamSpec, n: f32, form: u8) -> String {
    let v = spec.from_norm(n);
    if let Some(item) = spec.enum_item(v) {
        return item.key.to_owned();
    }
    if spec.is_discrete() && spec.unit == Unit::None && spec.max() == 1.0 && spec.min() == 0.0 {
        return if v >= 0.5 { "on" } else { "off" }.to_owned();
    }
    match form % 3 {
        0 => spec.display(v).to_string(),
        1 => format!("{v}"),
        _ => format!("{v}{}", spec.unit.suffix()),
    }
}

fn cmp() -> impl Strategy<Value = Cmp> {
    prop_oneof![
        Just(Cmp::Lt),
        Just(Cmp::Le),
        Just(Cmp::Gt),
        Just(Cmp::Ge),
        Just(Cmp::Eq),
        Just(Cmp::Ne)
    ]
}

fn knob_and_spec() -> impl Strategy<Value = (Knob, &'static ParamSpec)> {
    let all = params();
    prop_oneof![
        (0usize..all.len()).prop_map(move |i| (Knob::Param(all[i].0.clone()), all[i].1)),
        (0usize..8).prop_map(|i| (
            Knob::Macro(MacroIndex::new(i).unwrap()),
            &enth_patch::catalogue::MACRO_VALUE
        )),
    ]
}

fn source() -> impl Strategy<Value = ModSource> {
    prop_oneof![
        (0usize..8).prop_map(|i| ModSource::Macro(MacroIndex::new(i).unwrap())),
        (1u8..4).prop_map(|n| ModSource::Module(ModuleId::new(&format!("lfo{n}")).unwrap())),
        (1u8..4).prop_map(|n| ModSource::Module(ModuleId::new(&format!("env{n}")).unwrap())),
        Just(ModSource::Velocity),
        Just(ModSource::Key),
        Just(ModSource::ModWheel),
    ]
}

/// Every valid predicate shape, with atoms as authors write them.
fn predicate() -> impl Strategy<Value = Predicate> {
    let value = (knob_and_spec(), cmp(), 0.0f32..=1.0, any::<u8>()).prop_map(
        |((knob, spec), cmp, n, form)| Predicate::Value {
            knob,
            cmp,
            value: Plain::new(value_text(spec, n, form)),
        },
    );
    let within = (knob_and_spec(), 0.0f32..=1.0, 0.0f32..=1.0, any::<u8>()).prop_map(
        |((knob, spec), a, b, form)| Predicate::Within {
            knob,
            lo: Plain::new(value_text(spec, a.min(b), form)),
            hi: Plain::new(value_text(spec, a.max(b), form / 3)),
        },
    );
    let all = params();
    let route = (
        source(),
        0usize..all.len(),
        cmp(),
        -100i32..=100,
        any::<bool>(),
    )
        .prop_map(move |(src, i, cmp, d, pct)| {
            let depth = if pct { format!("{d}%") } else { format!("{d}") };
            Predicate::Route {
                route: RouteAddr {
                    src,
                    dst: all[i].0.clone(),
                },
                cmp,
                depth: Plain::new(depth),
            }
        });
    let has = (0usize..ModuleKind::ALL.len()).prop_map(|i| Predicate::Has {
        module_kind: KindKey(ModuleKind::ALL[i]),
    });
    let enabled =
        (0usize..ModuleKind::ALL.len(), 1u8..10, any::<bool>()).prop_map(|(i, n, enabled)| {
            Predicate::Enabled {
                module: id_for(ModuleKind::ALL[i], n),
                enabled,
            }
        });
    let touched = (knob_and_spec(), 1u8..=100)
        .prop_map(|((knob, _), min_span_pct)| Predicate::Touched { knob, min_span_pct });
    prop_oneof![value, within, route, has, enabled, touched]
}

proptest! {
    /// Canonical print, then a strict parse, gives the same value back; printing again gives the same line; the
    /// line reads as the object wherever a lesson holds a predicate.
    #[test]
    fn print_then_parse_round_trips(p in predicate()) {
        let line = p.to_string();
        let q: Predicate = line.parse().map_err(|e| TestCaseError::fail(format!("{line}: {e}")))?;
        prop_assert_eq!(&q, &p, "{}", line);
        prop_assert_eq!(q.to_string(), line.clone());
        let from_json: Predicate = serde_json::from_value(serde_json::Value::String(line)).unwrap();
        prop_assert_eq!(&from_json, &p);
        let object: Predicate = serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
        prop_assert_eq!(object, p);
    }

    /// Any text: a predicate or a refusal, never a panic; whatever parses prints to a line that parses to it.
    #[test]
    fn any_text_parses_or_is_refused(text in "\\PC{0,40}") {
        if let Ok(p) = text.parse::<Predicate>() {
            prop_assert_eq!(p.to_string().parse::<Predicate>().unwrap(), p);
        }
    }

    /// Near-miss text built from the grammar's own pieces: never a panic.
    #[test]
    fn grammar_shaped_text_never_panics(
        a in prop::sample::select(vec!["filter1.cutoff", "macro1", "has", "touched", "enabled", "osc1.wave", "x", ""]),
        op in prop::sample::select(vec!["<", "<=", "==", "=", "!=", ">", "->", "in", "[", "]", ",", "%", ""]),
        b in "[-+0-9a-zA-Z.%:\\[\\], ]{0,12}",
    ) {
        let _ = format!("{a} {op} {b}").parse::<Predicate>();
        let _ = format!("{a}{op}{b}").parse::<Predicate>();
    }
}

/// The suffixes each unit reads (enth-params' tables), with long forms and case variants.
fn suffixes(unit: Unit) -> &'static [&'static str] {
    match unit {
        Unit::Hz => &["", "Hz", "hz", "HZ"],
        Unit::Ms => &["", "ms", "MS"],
        Unit::Db => &["", "dB", "db", "DB"],
        Unit::Cents => &["", "ct", "cent", "cents"],
        Unit::Semitones => &["", "st", "semi", "semis"],
        Unit::Octaves => &["", "oct", "octave", "octaves"],
        Unit::Percent => &["", "%"],
        Unit::Ratio => &["", ":1"],
        Unit::Degrees => &["", "deg", "°", "degrees"],
        _ => &[""],
    }
}

/// Every text form of `v` the CLI reads for `unit`: each suffix (with and without a space before it), the scaled
/// forms (kHz, s, st ⇄ ct ⇄ oct), a note name for Hz, and the display's typographic minus.
fn forms(spec: &ParamSpec, v: f32) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for s in suffixes(spec.unit) {
        out.push(format!("{v}{s}"));
        if !s.is_empty() {
            out.push(format!("{v} {s}"));
        }
    }
    let v64 = f64::from(v);
    match spec.unit {
        Unit::Hz => {
            out.push(format!("{}kHz", v64 / 1000.0));
            out.push(format!("{}khz", v64 / 1000.0));
        }
        Unit::Ms => {
            out.push(format!("{}s", v64 / 1000.0));
            out.push(format!("{}sec", v64 / 1000.0));
        }
        Unit::Cents => {
            out.push(format!("{}st", v64 / 100.0));
            out.push(format!("{}oct", v64 / 1200.0));
        }
        Unit::Semitones => {
            out.push(format!("{}ct", v64 * 100.0));
            out.push(format!("{}oct", v64 / 12.0));
        }
        Unit::Octaves => {
            out.push(format!("{}st", v64 * 12.0));
            out.push(format!("{}ct", v64 * 1200.0));
        }
        _ => {}
    }
    if v < 0.0 {
        out.push(format!("\u{2212}{}{}", -v, spec.unit.suffix()));
    }
    if v > 0.0 && spec.unit.signed() {
        out.push(format!("+{v}{}", spec.unit.suffix()));
    }
    out.push(spec.display(v).to_string());
    out
}

/// A patch with every module kind once, so every parameter address resolves.
fn full_patch() -> Patch {
    let mut p = Patch::init();
    for m in CATALOGUE {
        let id = id_for(m.kind, 1);
        if p.find(&id).is_none() {
            let section = m.kind.section();
            let at = enth_patch::Position { section, index: 0 };
            let module = enth_patch::ModuleInstance::new(id, m.kind);
            enth_patch::apply(&mut p, enth_patch::Op::AddModule { at, module }).unwrap();
        }
    }
    p
}

/// Acceptance (docs/phase-3.md, PLY): every unit form the CLI accepts parses in a predicate, and the predicate reads
/// the value `ParamSpec::parse` (what `enth set` uses) reads: with the control set to that value, `<=` and `>=` both
/// hold and `<` does not.
#[test]
fn every_cli_unit_form_parses_in_a_predicate() {
    let patch = full_patch();
    let none = TouchLog::default();
    let mut checked = 0;
    for (addr, spec) in params() {
        let mut texts: Vec<String> = Vec::new();
        let items: Vec<_> = (0..64).map_while(|i| spec.enum_item(i as f32)).collect();
        if !items.is_empty() {
            for item in items {
                texts.push(item.key.to_owned());
                texts.push(item.key.to_ascii_uppercase());
            }
        } else if spec.is_discrete()
            && spec.min() == 0.0
            && spec.max() == 1.0
            && spec.unit == Unit::None
        {
            texts.extend(
                ["on", "off", "true", "false", "1", "0", "yes", "no", "ON"].map(str::to_owned),
            );
        } else {
            for n in [0.0f32, 0.25, 0.5, 0.8, 1.0] {
                let v = spec.from_norm(n);
                let v = if spec.is_discrete() { v.round() } else { v };
                texts.extend(forms(spec, v));
            }
            if spec.unit == Unit::Hz {
                for note in ["A4", "C#3", "Db2", "c5", "A1"] {
                    if spec.parse(note).is_ok() {
                        texts.push(note.to_owned());
                    }
                }
            }
        }
        for text in texts {
            let Ok(want) = spec.parse(&text) else {
                continue; // a form whose value falls outside this control's range (8 kHz on a 500 Hz low cut)
            };
            let mut p = patch.clone();
            enth_patch::apply(
                &mut p,
                enth_patch::Op::SetParam {
                    addr: addr.clone(),
                    value: want,
                },
            )
            .unwrap();
            if p.value(&addr) != Some(want) {
                continue; // the patch stores another plain value for it (a phase of 360° is 0°)
            }
            for (op, holds) in [("<=", true), (">=", true), ("<", false), ("==", true)] {
                let line = format!("{addr} {op} {text}");
                let pred: Predicate = line.parse().unwrap_or_else(|e| panic!("{line}: {e}"));
                assert_eq!(pred.holds(&p, &none), holds, "{line}");
            }
            checked += 1;
        }
    }
    // The macro knob.
    let mut p = patch.clone();
    p.macros[0].value = 0.5;
    for text in ["0.5", ".5", "5e-1"] {
        let pred: Predicate = format!("macro1 >= {text}").parse().unwrap();
        assert!(pred.holds(&p, &none), "{text}");
    }
    assert!(checked > 500, "only {checked} forms checked");
}

/// Route depths read as the CLI reads them: percent, or an interval on the destination.
#[test]
fn route_depths_read_cli_forms() {
    let mut p = full_patch();
    let route: RouteAddr = "lfo1 -> filter1.cutoff".parse().unwrap();
    let spec = p.spec(&route.dst).unwrap();
    let base = p.value(&route.dst).unwrap();
    let none = TouchLog::default();
    for text in [
        "10%",
        "10",
        "-10%",
        "+1oct",
        "-1oct",
        "12st",
        "+1200ct",
        "+200Hz",
        "\u{2212}12 st",
    ] {
        let depth = if text.ends_with('%') || text.chars().all(|c| c.is_ascii_digit() || c == '-') {
            text.trim_end_matches('%').parse::<f32>().unwrap()
        } else {
            spec.depth_from_text(base, text).unwrap()
        };
        let mut q = p.clone();
        enth_patch::apply(
            &mut q,
            enth_patch::Op::InsertRoute {
                index: 0,
                route: enth_patch::Route {
                    src: route.src.clone(),
                    dst: route.dst.clone(),
                    depth,
                },
            },
        )
        .unwrap();
        for (op, holds) in [(">=", true), ("<=", true), ("==", true), ("<", false)] {
            let line = format!("{route} {op} {text}");
            let pred: Predicate = line.parse().unwrap_or_else(|e| panic!("{line}: {e}"));
            assert_eq!(pred.holds(&q, &none), holds, "{line}");
        }
    }
    p.routes.clear();
    assert!(
        "lfo1 -> filter1.cutoff == 0"
            .parse::<Predicate>()
            .unwrap()
            .holds(&p, &none)
    );
}
