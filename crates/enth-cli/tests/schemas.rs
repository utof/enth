//! The published JSON Schemas: a snapshot of each (so a change shows in review), and typed round trips that keep
//! the input schemas (patch, ops, perf, wavmeta) in step with the code that reads and writes those documents.
//! There is no `jsonschema` crate (its default features build C code): the Rust types are the schema.
#![allow(
    clippy::disallowed_methods,
    reason = "tests read and write scratch files"
)]

mod common;

use common::{Scratch, json, run};
use enth_cli::schema::{self, OpDoc, PatchDoc, PerfFile, WavMeta};
use enth_patch::{Op, Patch, file, wire};
use insta::assert_json_snapshot;
use serde_json::Value;

#[test]
fn every_schema_is_draft_2020_12_with_its_id() {
    for e in schema::ENTRIES {
        let o = run(&["schema", e.name]);
        assert_eq!(o.code, 0, "{}", e.name);
        let v = json(&o);
        assert_eq!(v["$id"], e.id);
        assert_eq!(
            v["$schema"], "https://json-schema.org/draft/2020-12/schema",
            "{}",
            e.name
        );
        if v["properties"]["schema"]["type"] == "string" {
            assert_eq!(v["properties"]["schema"]["const"], e.id, "{}", e.name);
        }
        // By id works too.
        assert_eq!(run(&["schema", e.id]).stdout, o.stdout);
        let mut settings = insta::Settings::clone_current();
        settings.set_prepend_module_to_snapshot(false);
        settings.bind(|| assert_json_snapshot!(format!("schema_{}", e.name), v));
    }
}

#[test]
fn every_output_schema_id_matches_its_objects() {
    // Each command's object carries the id its schema publishes.
    let s = Scratch::new("schema_ids");
    let p = s.new_patch("init", "p.patch.json");
    let cases: Vec<(&str, Vec<String>)> = vec![
        ("modules", vec!["modules".into()]),
        ("params", vec!["params".into(), "osc".into()]),
        ("guide", vec!["guide".into()]),
        ("templates", vec!["new".into(), "--list".into()]),
        ("get", vec!["get".into(), p.clone()]),
        ("diff", vec!["diff".into(), p.clone(), p.clone()]),
        ("schema", vec!["schema".into()]),
        ("explain", vec!["explain".into(), "svf".into()]),
        (
            "challenge-new",
            vec![
                "challenge".into(),
                "new".into(),
                "--seed".into(),
                "7".into(),
            ],
        ),
        (
            "challenge-show",
            vec![
                "challenge".into(),
                "show".into(),
                "1040-1000-0000-7M".into(),
            ],
        ),
        (
            "challenge-decode",
            vec![
                "challenge".into(),
                "decode".into(),
                "1040-1000-0000-7M".into(),
            ],
        ),
    ];
    for (name, args) in cases {
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let v = json(&run(&args));
        assert_eq!(v["schema"], schema::entry(name).unwrap().id, "{name}");
    }
}

#[test]
fn patch_files_match_the_patch_schema() {
    let s = Scratch::new("schema_patch");
    for t in ["init", "pluck_1osc", "pad_2osc", "bass_sub"] {
        // The sparse template as shipped, and the dense file `new` writes.
        let path = s.new_patch(t, &format!("{t}.patch.json"));
        let dense = std::fs::read_to_string(&path).unwrap();
        let doc: PatchDoc = serde_json::from_str(&dense).unwrap_or_else(|e| panic!("{t}: {e}"));
        let back: Value = serde_json::to_value(&doc).unwrap();
        let orig: Value = serde_json::from_str(&dense).unwrap();
        assert_eq!(back, orig, "{t}: the schema type drops or changes nothing");
        let sparse = std::fs::read_to_string(format!(
            "{}/../../content/templates/{t}.patch.json",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap();
        let _: PatchDoc = serde_json::from_str(&sparse).unwrap_or_else(|e| panic!("{t}: {e}"));
    }
    // A patch with every section filled in.
    let mut p = Patch::init();
    let ops = wire::resolve_str(
        &p,
        r#"[{"add": "svf", "label": "Body"}, {"add": "lfo"}, {"add": "reverb", "enabled": false},
            {"route": "lfo1 -> filter1.cutoff", "depth": 20}, {"macro": 2, "to": 0.5, "name": "Bright"},
            {"slot": 3, "to": "filter1.cutoff"}, {"meta": {"name": "Full", "author": "a", "tags": ["x"]}}]"#,
    )
    .unwrap();
    enth_patch::apply_all(&mut p, &ops).unwrap();
    let text = p.to_json();
    let doc: PatchDoc = serde_json::from_str(&text).unwrap();
    assert_eq!(
        serde_json::to_value(&doc).unwrap(),
        serde_json::from_str::<Value>(&text).unwrap()
    );
    assert!(file::load(&text).is_ok());
}

#[test]
fn every_op_matches_the_ops_schema() {
    // Every verb in input form, and every resolved op (inverses included) in the wire form the CLI prints.
    let input = r#"[
      {"set": "env1.attack", "to": "600ms"}, {"set": "voice.mode", "to": "legato"},
      {"set": "osc1.sync", "to": true}, {"set": "master.level", "to": -6},
      {"add": "lfo", "id": "lfo1", "at": "modulators[1]", "params": {"rate": "0.2Hz"}},
      {"add": "svf", "label": "Body"}, {"add": "comp", "enabled": false},
      {"move": "comp1", "to": "fx[0]"}, {"route": "lfo1 -> filter1.cutoff", "depth": "30%"},
      {"route": "lfo1 -> osc1.fine", "depth": 10, "index": 0}, {"unroute": "lfo1 -> osc1.fine"},
      {"enable": "comp1", "to": true}, {"enable": "comp1"}, {"macro": 1, "to": 0.5, "name": "Brightness"},
      {"macro": 2, "name": null}, {"slot": 7, "to": "filter1.cutoff"}, {"slot": 7, "to": null},
      {"amp_env": "env1"}, {"label": "osc1", "to": "Bass"}, {"label": "osc1", "to": null},
      {"meta": {"name": "Soft pluck", "tags": ["pluck"]}}, {"remove": "comp1"}
    ]"#;
    let docs: Vec<OpDoc> = serde_json::from_str(input).unwrap();
    assert_eq!(docs.len(), 22);
    let p = Patch::init();
    let ops: Vec<Op> = wire::resolve_str(&p, input).unwrap();
    let mut after = p.clone();
    let inverse = enth_patch::apply_all(&mut after, &ops).unwrap();
    for (patch, list) in [(&p, &ops), (&after, &inverse)] {
        let wire_json = wire::to_json(patch, list);
        let parsed: Vec<OpDoc> = serde_json::from_value(wire_json.clone())
            .unwrap_or_else(|e| panic!("{e}: {wire_json}"));
        assert_eq!(parsed.len(), list.len());
    }
    // A misspelled key is not an op.
    assert!(serde_json::from_str::<OpDoc>(r#"{"set": "osc1.fine", "too": 1}"#).is_err());
}

#[test]
fn perf_and_wavmeta_documents_round_trip() {
    let s = Scratch::new("schema_perf");
    let p = s.new_patch("init", "p.patch.json");
    let wav = s.path("p.wav");
    let o = run(&[
        "render",
        &p,
        "--out",
        &wav,
        "--perf",
        "note=A#2,vel=64,hold=0.5s,dur=1s",
    ]);
    assert_eq!(o.code, 0, "{}", o.stdout);
    let d = enth_cli::wav::decode(&std::fs::read(&wav).unwrap()).unwrap();
    let meta: WavMeta = d.meta.unwrap();
    assert_eq!(meta.schema, "enth.wavmeta/1");
    assert_eq!(meta.perf.schema.as_deref(), Some("enth.perf/1"));
    let text = serde_json::to_string(&meta.perf).unwrap();
    let back: PerfFile = serde_json::from_str(&text).unwrap();
    assert_eq!(back, meta.perf);
    // The document a goal file holds (no schema field) is accepted as --perf.
    let goal =
        r#"{"notes": [{"note": "A3", "vel": 100, "on_s": 0.0, "off_s": 1.5}], "dur_s": 2.0}"#;
    let _: PerfFile = serde_json::from_str(goal).unwrap();
    let c = run(&["check", &p, "--expect", "pitch.note ~ A3", "--perf", goal]);
    assert_eq!(c.code, 0, "{}", c.stdout);
}
