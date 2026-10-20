//! `enth explain`: what a parameter, a module or a module kind does, from the catalogue, and with `--patch` what a
//! step of it does to that patch, measured. Every parameter of the catalogue has an explanation, the measurement does
//! not depend on the number of rayon workers, and the errors carry a code, a hint and the nearest names.
#![allow(
    clippy::disallowed_methods,
    reason = "tests read and write scratch files and build thread pools"
)]

mod common;
mod gamekit;

use common::{Scratch, round_trip, run};
use enth_cli::schema::{ErrorOut, ExplainOut, Reading};
use enth_patch::catalogue::{Builtin, ModuleKind};

fn explain(args: &[&str]) -> (enth_cli::Output, ExplainOut) {
    let mut a = vec!["explain"];
    a.extend_from_slice(args);
    let o = run(&a);
    assert_eq!(o.code, 0, "{args:?}: {}", o.stdout);
    let e = round_trip::<ExplainOut>(&o);
    (o, e)
}

#[test]
fn every_parameter_of_the_catalogue_can_be_explained() {
    let mut owners: Vec<(String, Vec<(&'static str, &'static str)>)> = ModuleKind::ALL
        .iter()
        .map(|k| {
            (
                k.key().to_owned(),
                k.params().iter().map(|p| (p.key, p.learn.desc)).collect(),
            )
        })
        .collect();
    for b in [Builtin::Voice, Builtin::Master] {
        owners.push((
            b.key().to_owned(),
            b.params().iter().map(|p| (p.key, p.learn.desc)).collect(),
        ));
    }
    let mut count = 0;
    for (owner, params) in owners {
        let (_, whole) = explain(&[&owner]);
        assert_eq!(whole.schema, "enth.explain/1");
        assert!(
            ["kind", "builtin"].contains(&whole.what.as_str()),
            "{owner}: {}",
            whole.what
        );
        assert_eq!(whole.params.len(), params.len(), "{owner}");
        assert!(whole.param.is_none() && whole.measured.is_none());
        assert!(whole.text.contains("Parameters:"), "{owner}");
        for (key, desc) in params {
            let addr = format!("{owner}.{key}");
            let (_, e) = explain(&[&addr]);
            let facts = e
                .param
                .as_ref()
                .unwrap_or_else(|| panic!("{addr}: no facts"));
            assert_eq!(
                (e.what.as_str(), facts.key.as_str()),
                ("param", key),
                "{addr}"
            );
            assert_eq!(e.kind, owner, "{addr}");
            assert_eq!(e.summary, desc, "{addr}");
            assert!(e.text.contains(desc), "{addr}: {}", e.text);
            assert!(
                !facts.accepts.is_empty() && !facts.tier.is_empty(),
                "{addr}"
            );
            match facts.scale.as_str() {
                "enum" | "toggle" => assert!(facts.min.is_none() && facts.max.is_none(), "{addr}"),
                _ => assert!(facts.min.unwrap() < facts.max.unwrap(), "{addr}"),
            }
            count += 1;
        }
    }
    assert!(count > 40, "only {count} parameters in the catalogue?");
}

#[test]
fn a_parameter_says_its_unit_range_and_descriptor() {
    let (o, e) = explain(&["filter1.cutoff"]);
    let p = e.param.unwrap();
    assert_eq!(
        (e.what.as_str(), e.kind.as_str(), e.subject.as_str()),
        ("param", "svf", "filter1.cutoff")
    );
    assert_eq!(
        (p.unit.as_str(), p.min, p.max),
        ("Hz", Some(20.0), Some(20_000.0))
    );
    assert_eq!((p.default, p.scale.as_str()), (Reading::Num(2000.0), "log"));
    assert!(p.modulatable && p.automatable);
    assert!(p.accepts.iter().any(|a| a == "800Hz"), "{:?}", p.accepts);
    assert_eq!(p.moves.unwrap().feature, "spectrum.centroid_hz");
    assert!(
        e.text.contains("20Hz") && e.text.contains("20kHz"),
        "{}",
        e.text
    );
    assert!(e.text.contains("spectrum.centroid_hz"), "{}", e.text);
    assert!(e.measured.is_none());
    assert!(e.hint.contains("--patch"), "{}", e.hint);
    // The terminal gets the text.
    let t = common::run_tty(&["explain", "filter1.cutoff"]);
    assert_eq!(t.stdout.trim_end(), e.text);
    let _ = o;
}

#[test]
fn a_choice_lists_its_values() {
    let (_, e) = explain(&["filter1.mode"]);
    let p = e.param.unwrap();
    assert_eq!(p.scale, "enum");
    assert_eq!(p.values, ["lp", "bp", "hp", "notch"]);
    assert_eq!(p.default, Reading::Text("lp".to_owned()));
    assert!(e.text.contains("lp, bp, hp, notch"), "{}", e.text);
}

#[test]
fn a_module_in_the_patch_and_its_kind_both_explain() {
    let s = Scratch::new("explain_module");
    let patch = s.new_patch("pluck_1osc", "p.patch.json");
    let (_, module) = explain(&["filter1", "--patch", &patch]);
    assert_eq!(
        (
            module.what.as_str(),
            module.subject.as_str(),
            module.kind.as_str()
        ),
        ("module", "filter1", "svf")
    );
    let (_, kind) = explain(&["svf"]);
    assert_eq!((kind.what.as_str(), kind.subject.as_str()), ("kind", "svf"));
    assert_eq!(module.params, kind.params);
    let keys: Vec<_> = kind.params.iter().map(|p| p.key.as_str()).collect();
    assert_eq!(keys, ["mode", "cutoff", "resonance", "keytrack", "drive"]);
    // The alias the guide teaches works for a kind.
    let (_, built) = explain(&["voice"]);
    assert_eq!(built.what, "builtin");
    let (_, master) = explain(&["master.level"]);
    assert_eq!(master.param.unwrap().unit, "dB");
}

#[test]
fn a_parameter_is_measured_on_the_patch() {
    let s = Scratch::new("explain_measured");
    let patch = s.new_patch("pluck_1osc", "p.patch.json");
    let (o, e) = explain(&["filter1.cutoff", "--patch", &patch]);
    let m = e.measured.expect("measured");
    assert_eq!(m.params.len(), 1);
    assert!(m.renders >= 2, "{}", m.renders);
    let p = &m.params[0];
    assert_eq!(p.param, "filter1.cutoff");
    assert!(p.reach > 0.0 && !p.inert, "{p:?}");
    assert!(!p.effects.is_empty(), "{p:?}");
    assert!(
        p.effects
            .iter()
            .any(|f| f.feature == "spectrum.centroid_hz"),
        "{p:?}"
    );
    assert!(e.text.contains("Measured on"), "{}", e.text);
    assert!(m.skipped.is_empty(), "{:?}", m.skipped);

    // The same measurement on one worker and on several.
    let pool = |n| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(n)
            .build()
            .unwrap()
    };
    let args = ["explain", "filter1.cutoff", "--patch", &patch];
    let one = pool(1).install(|| run(&args));
    let four = pool(4).install(|| run(&args));
    assert_eq!(one.stdout, o.stdout);
    assert_eq!(four.stdout, o.stdout);
}

#[test]
fn a_module_is_measured_parameter_by_parameter() {
    let s = Scratch::new("explain_module_measured");
    let patch = s.new_patch("pluck_1osc", "p.patch.json");
    let (_, e) = explain(&["filter1", "--patch", &patch]);
    let m = e.measured.expect("measured");
    // The choices have no step to measure; the rest are ordered by how much they matter.
    assert_eq!(m.skipped, ["filter1.mode"]);
    assert_eq!(m.params.len(), 4);
    let reach: Vec<f64> = m.params.iter().map(|p| p.reach).collect();
    assert!(reach.windows(2).all(|w| w[0] >= w[1]), "{reach:?}");
    assert!(m.renders <= 48, "{}", m.renders);
    // A kind has no module in the patch to measure; the hint says what to name instead.
    let (_, kind) = explain(&["svf", "--patch", &patch]);
    assert!(kind.measured.is_none());
    assert!(kind.hint.contains("module id"), "{}", kind.hint);
}

#[test]
fn errors_have_a_code_a_hint_and_the_nearest_names() {
    let s = Scratch::new("explain_errors");
    let patch = s.new_patch("pluck_1osc", "p.patch.json");
    let missing = s.path("nope.patch.json");
    let cases: Vec<(Vec<&str>, i32, &str, Option<&str>)> = vec![
        (vec!["explain"], 2, "usage", None),
        (vec!["explain", "svff"], 3, "unknown_kind", Some("svf")),
        (
            vec!["explain", "filter1.cutof", "--patch", &patch],
            3,
            "unknown_param",
            Some("filter1.cutoff"),
        ),
        (
            vec!["explain", "svf.cutof"],
            3,
            "unknown_param",
            Some("svf.cutoff"),
        ),
        (
            vec!["explain", "filter9", "--patch", &patch],
            3,
            "unknown_module",
            Some("filter1"),
        ),
        (
            vec!["explain", "filter1", "--patch", &missing],
            5,
            "file_not_found",
            None,
        ),
        (
            vec!["explain", "filter1", "--patch", &patch, "--perf", "note=Z9"],
            2,
            "bad_perf",
            None,
        ),
    ];
    for (args, exit, error, near) in cases {
        let o = run(&args);
        let e: ErrorOut = round_trip(&o);
        assert_eq!(o.code, exit, "{args:?}: {}", o.stdout);
        assert_eq!(e.exit, exit);
        if !error.is_empty() {
            assert_eq!(e.error, error, "{args:?}");
        }
        assert_ne!(e.hint, "", "{args:?}");
        if let Some(near) = near {
            assert!(
                e.did_you_mean.iter().any(|d| d == near),
                "{args:?}: {}",
                o.stdout
            );
        }
    }
}

#[test]
fn snapshots() {
    let s = Scratch::new("explain_snap");
    let patch = s.new_patch("pluck_1osc", "p.patch.json");
    let snap = |name: &str, args: &[&str], hide: &[&str]| {
        let mut a = vec!["explain"];
        a.extend_from_slice(args);
        let o = run(&a);
        s.settings().bind(|| {
            insta::assert_snapshot!(name, gamekit::record(&s, &o, &[], hide));
        });
    };
    snap("explain_param", &["filter1.cutoff"], &[]);
    snap("explain_enum", &["filter1.mode"], &[]);
    snap("explain_kind", &["svf"], &[]);
    snap("explain_builtin", &["voice.glide"], &[]);
    // What the engine measured moves with the DSP: the shape is pinned, the findings are not.
    snap(
        "explain_measured",
        &["filter1.cutoff", "--patch", &patch],
        &["/text", "/measured/params/0/effects"],
    );
    snap("err_explain_unknown_kind", &["svff"], &[]);
    snap("err_explain_unknown_param", &["svf.cutof"], &[]);
}
