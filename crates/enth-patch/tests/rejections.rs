//! Every way a file, an op or the ops JSON can be refused, with its error code, its path and, where a near miss is
//! likely, the suggestions (S1b). The codes are an interface: the CLI maps them to exit codes and agents branch on
//! them, so each one is pinned here from an input that produces it, and the last check fails if a code in
//! `error::CODES` is never produced.

mod common;

use std::collections::BTreeSet;

use common::*;
use enth_patch::PatchError;
use enth_patch::addr::{MacroIndex, ModSource, ModuleId, Position, RouteAddr, SlotIndex};
use enth_patch::canon::exact_hash;
use enth_patch::catalogue::{ModuleKind, OscParam};
use enth_patch::error::CODES;
use enth_patch::file::load;
use enth_patch::model::{ModuleInstance, Patch, Route, Section};
use enth_patch::ops::{Op, apply};
use enth_patch::validate::{validate, violations};
use enth_patch::wire;
use serde_json::{Value, json};

/// A patch with something in every section, routes of both rates, a global LFO and a slot, built through ops.
fn rich() -> Patch {
    let mut p = base_patch();
    let add = |p: &mut Patch, id: &str, kind: ModuleKind| {
        let section = kind.section();
        let m = ModuleInstance::new(ModuleId::new(id).unwrap(), kind);
        let at = Position {
            section,
            index: p.section(section).len(),
        };
        apply(p, &Op::AddModule { at, module: m }).unwrap();
    };
    add(&mut p, "filter1", ModuleKind::Svf);
    add(&mut p, "reverb1", ModuleKind::Reverb);
    add(&mut p, "env2", ModuleKind::Adsr);
    add(&mut p, "lfo1", ModuleKind::Lfo);
    add(&mut p, "lfo2", ModuleKind::Lfo);
    apply(
        &mut p,
        Op::SetParam {
            addr: "lfo2.scope".parse().unwrap(),
            value: 1.0,
        },
    )
    .unwrap();
    for (route, depth) in [
        ("env2 -> filter1.cutoff", 30.0),
        ("lfo1 -> osc1.semi", 10.0),
        ("lfo2 -> filter1.resonance", 5.0),
        ("lfo2 -> reverb1.wet", 20.0),
    ] {
        let r: RouteAddr = route.parse().unwrap();
        let index = p.routes.len();
        apply(
            &mut p,
            Op::InsertRoute {
                index,
                route: Route {
                    src: r.src,
                    dst: r.dst,
                    depth,
                },
            },
        )
        .unwrap();
    }
    let slot = SlotIndex::new(0).unwrap();
    apply(
        &mut p,
        Op::SetSlot {
            slot,
            addr: Some("filter1.cutoff".parse().unwrap()),
        },
    )
    .unwrap();
    validate(&p).unwrap();
    p
}

fn rich_json() -> Value {
    serde_json::from_str(&rich().to_json()).unwrap()
}

type Expect<'a> = &'a [(&'a str, &'a str)];

/// Collects every mismatch so one run shows them all, and the codes seen so the last check can tell whether the
/// vocabulary is covered.
#[derive(Default)]
struct Checks {
    failures: Vec<String>,
    seen: BTreeSet<&'static str>,
}

impl Checks {
    fn expect(&mut self, name: &str, got: &[(String, PatchError)], want: Expect<'_>) {
        for (_, e) in got {
            self.seen.insert(e.code());
            if e.hint().is_empty() || e.message().is_empty() || !CODES.contains(&e.code()) {
                self.failures.push(format!(
                    "{name}: {e:?} lacks a message, a hint or a listed code"
                ));
            }
        }
        let got_pairs: Vec<(String, &str)> =
            got.iter().map(|(p, e)| (p.clone(), e.code())).collect();
        let want_pairs: Vec<(String, &str)> =
            want.iter().map(|(p, c)| ((*p).to_owned(), *c)).collect();
        if got_pairs != want_pairs {
            self.failures.push(format!(
                "{name}:\n      got  {got_pairs:?}\n      want {want_pairs:?}"
            ));
        }
    }

    fn file_text(&mut self, name: &str, text: &str, want: Expect<'_>) {
        let got = match load(text) {
            Ok(_) => vec![("-".to_owned(), PatchError::MissingSchema)],
            Err(inv) => inv.0.into_iter().map(|v| (v.path, v.error)).collect(),
        };
        // An accepted file shows up as the placeholder error and fails the comparison unless that was expected.
        if want.is_empty() {
            if !got.iter().any(|(p, _)| p == "-") {
                self.failures.push(format!(
                    "{name}: expected the file to load, got {:?}",
                    got.iter().map(|(p, e)| (p, e.code())).collect::<Vec<_>>()
                ));
            }
            return;
        }
        self.expect(name, &got, want);
    }

    fn file(&mut self, name: &str, mutate: impl FnOnce(&mut Value), want: Expect<'_>) {
        let mut v = rich_json();
        mutate(&mut v);
        self.file_text(name, &v.to_string(), want);
    }

    /// The suggestions of the first error of a file.
    fn file_dym(&mut self, name: &str, mutate: impl FnOnce(&mut Value), want: &[&str]) {
        let mut v = rich_json();
        mutate(&mut v);
        match load(&v.to_string()) {
            Err(inv) => {
                let got = inv.0[0].error.did_you_mean();
                if got != want {
                    self.failures
                        .push(format!("{name}: did_you_mean {got:?}, want {want:?}"));
                }
            }
            Ok(_) => self.failures.push(format!("{name}: expected a refusal")),
        }
    }

    fn op(&mut self, name: &str, p: &Patch, op: &Op, code: &str) {
        let mut q = p.clone();
        match apply(&mut q, op) {
            Ok(_) => self
                .failures
                .push(format!("{name}: expected {code}, the op applied")),
            Err(e) => {
                self.seen.insert(e.code());
                if e.code() != code {
                    self.failures
                        .push(format!("{name}: expected {code}, got {} ({e})", e.code()));
                }
                if e.hint().is_empty() || e.message().is_empty() {
                    self.failures.push(format!("{name}: no message or hint"));
                }
                if !identical(&q, p) {
                    self.failures
                        .push(format!("{name}: a refused op changed the patch"));
                }
            }
        }
    }

    fn wire(&mut self, name: &str, p: &Patch, text: &str, want: Expect<'_>) {
        let got = match wire::resolve_str(p, text) {
            Ok(_) => vec![("-".to_owned(), PatchError::MissingSchema)],
            Err(inv) => inv.0.into_iter().map(|v| (v.path, v.error)).collect(),
        };
        if want.is_empty() {
            if !got.iter().any(|(p, _)| p == "-") {
                self.failures.push(format!(
                    "{name}: expected the ops to resolve, got {:?}",
                    got.iter().map(|(p, e)| (p, e.code())).collect::<Vec<_>>()
                ));
            }
            return;
        }
        self.expect(name, &got, want);
    }

    fn wire_dym(&mut self, name: &str, p: &Patch, text: &str, want: &[&str]) {
        match wire::resolve_str(p, text) {
            Err(inv) => {
                let got = inv.0[0].error.did_you_mean();
                if got != want {
                    self.failures
                        .push(format!("{name}: did_you_mean {got:?}, want {want:?}"));
                }
            }
            Ok(_) => self.failures.push(format!("{name}: expected a refusal")),
        }
    }

    fn done(self) {
        assert!(self.failures.is_empty(), "\n{}\n", self.failures.join("\n"));
        let missing: Vec<&&str> = CODES.iter().filter(|c| !self.seen.contains(**c)).collect();
        assert!(
            missing.is_empty(),
            "no case produces these codes: {missing:?}"
        );
    }
}

fn m(id: &str) -> ModuleId {
    ModuleId::new(id).unwrap()
}

fn at(section: Section, index: usize) -> Position {
    Position { section, index }
}

fn module(id: &str, kind: ModuleKind) -> ModuleInstance {
    ModuleInstance::new(m(id), kind)
}

fn route_op(index: usize, src: ModSource, dst: &str, depth: f32) -> Op {
    Op::InsertRoute {
        index,
        route: Route {
            src,
            dst: dst.parse().unwrap(),
            depth,
        },
    }
}

fn set(addr: &str, value: f32) -> Op {
    Op::SetParam {
        addr: addr.parse().unwrap(),
        value,
    }
}

#[test]
fn every_refusal_has_its_code_path_and_hint() {
    let mut c = Checks::default();

    // --- Files: values -----------------------------------------------------------------------------------------
    c.file(
        "a number for an enum",
        |v| v["generators"][0]["params"]["wave"] = json!(2),
        &[("generators[0].params.wave", "bad_value")],
    );
    c.file(
        "an unknown enum key",
        |v| v["generators"][0]["params"]["wave"] = json!("sawtooth"),
        &[("generators[0].params.wave", "bad_value")],
    );
    c.file(
        "1e39",
        |v| v["voice_fx"][0]["params"]["cutoff"] = json!(1e39),
        &[("voice_fx[0].params.cutoff", "non_finite")],
    );
    c.file(
        "past the range",
        |v| v["voice_fx"][0]["params"]["cutoff"] = json!(99_999),
        &[("voice_fx[0].params.cutoff", "out_of_range")],
    );
    c.file(
        "past the range, in a unit",
        |v| v["voice_fx"][0]["params"]["cutoff"] = json!("99kHz"),
        &[("voice_fx[0].params.cutoff", "out_of_range")],
    );
    c.file(
        "the wrong unit",
        |v| v["voice_fx"][0]["params"]["cutoff"] = json!("2s"),
        &[("voice_fx[0].params.cutoff", "wrong_unit")],
    );
    c.file(
        "a word for a number",
        |v| v["voice_fx"][0]["params"]["cutoff"] = json!("loud"),
        &[("voice_fx[0].params.cutoff", "bad_value")],
    );
    c.file(
        "null for a number",
        |v| v["generators"][0]["params"]["semi"] = Value::Null,
        &[("generators[0].params.semi", "bad_value")],
    );
    c.file(
        "an object for a number",
        |v| v["generators"][0]["params"]["semi"] = json!({"a": 1}),
        &[("generators[0].params.semi", "bad_value")],
    );
    c.file(
        "a fraction for a whole number",
        |v| v["voice"]["polyphony"] = json!(2.5),
        &[("voice.polyphony", "not_integral")],
    );
    c.file(
        "a parameter the kind lacks",
        |v| {
            v["voice_fx"][0]["params"]
                .as_object_mut()
                .unwrap()
                .insert("cutof".into(), json!(1000));
        },
        &[("voice_fx[0].params.cutof", "unknown_param")],
    );
    c.file(
        "a macro past 1",
        |v| v["macros"][0]["value"] = json!(1.5),
        &[("macros[0].value", "out_of_range")],
    );
    c.file(
        "a route depth past 100",
        |v| v["routes"][0]["depth"] = json!(101),
        &[("routes[0].depth", "out_of_range")],
    );
    c.file(
        "several at once, in document order",
        |v| {
            v["voice"]["polyphony"] = json!(2.5);
            v["generators"][0]["params"]["wave"] = json!(2);
            v["voice_fx"][0]["params"]["cutoff"] = json!(99_999);
        },
        &[
            ("voice.polyphony", "not_integral"),
            ("generators[0].params.wave", "bad_value"),
            ("voice_fx[0].params.cutoff", "out_of_range"),
        ],
    );
    // Canonical storage: accepted, and spelled one way.
    c.file(
        "-0.0 is accepted",
        |v| v["generators"][0]["params"]["semi"] = json!(-0.0),
        &[],
    );
    c.file(
        "a phase of 360 is accepted",
        |v| v["generators"][0]["params"]["phase"] = json!(360),
        &[],
    );
    c.file(
        "a toggle as 1",
        |v| v["generators"][0]["params"]["sync"] = json!(1),
        &[],
    );
    c.file(
        "a toggle as on",
        |v| v["generators"][0]["params"]["sync"] = json!("on"),
        &[],
    );
    c.file(
        "a file with fewer macros",
        |v| {
            v["macros"].as_array_mut().unwrap().pop();
        },
        &[],
    );
    c.file(
        "a file without the amp envelope takes the first adsr",
        |v| {
            v["voice"].as_object_mut().unwrap().remove("amp_env");
        },
        &[],
    );

    // --- Files: modules ----------------------------------------------------------------------------------------
    c.file(
        "an unknown kind",
        |v| v["generators"][0]["kind"] = json!("banana"),
        &[("generators[0].kind", "unknown_kind")],
    );
    c.file(
        "a kind in the wrong section",
        |v| v["generators"][0]["kind"] = json!("svf"),
        &[("generators[0]", "wrong_section")],
    );
    c.file(
        "a reserved id",
        |v| v["generators"][0]["id"] = json!("voice"),
        &[("generators[0].id", "reserved_id")],
    );
    c.file(
        "a future source name as an id",
        |v| v["generators"][0]["id"] = json!("aftertouch"),
        &[("generators[0].id", "reserved_id")],
    );
    c.file(
        "a malformed id",
        |v| v["generators"][0]["id"] = json!("Osc 1"),
        &[("generators[0].id", "bad_id")],
    );
    c.file(
        "a duplicate id",
        |v| {
            v["voice_fx"][0]["id"] = json!("osc1");
            v["routes"] = json!([]);
            v["slots"] = json!({});
        },
        &[("voice_fx[0].id", "duplicate_id")],
    );
    c.file(
        "too many generators",
        |v| {
            for i in 0..4 {
                let mut g = v["generators"][0].clone();
                g["id"] = json!(format!("osc{}", i + 2));
                v["generators"].as_array_mut().unwrap().push(g);
            }
        },
        &[("generators", "too_many")],
    );

    // --- Files: the amplitude envelope -------------------------------------------------------------------------
    c.file(
        "an amp envelope that is not there",
        |v| v["voice"]["amp_env"] = json!("env9"),
        &[("voice.amp_env", "unknown_module")],
    );
    c.file(
        "an amp envelope of the wrong kind",
        |v| v["voice"]["amp_env"] = json!("osc1"),
        &[("voice.amp_env", "wrong_kind")],
    );
    c.file(
        "a disabled amp envelope",
        |v| v["modulators"][0]["enabled"] = json!(false),
        &[("voice.amp_env", "in_use_as_amp_env")],
    );
    c.file(
        "no amp envelope and nothing to take",
        |v| {
            v["voice"].as_object_mut().unwrap().remove("amp_env");
            v["modulators"] = json!([]);
            v["routes"] = json!([]);
        },
        &[("voice.amp_env", "missing_amp_env")],
    );

    // --- Files: routes and slots -------------------------------------------------------------------------------
    c.file(
        "a route from an oscillator to a filter (a generator drives only an audio input)",
        |v| v["routes"] = json!([{"src": "osc1", "dst": "filter1.cutoff", "depth": 10}]),
        &[("routes[0].dst", "not_audio_dest")],
    );
    c.file(
        "a route from a filter",
        |v| v["routes"] = json!([{"src": "filter1", "dst": "osc1.semi", "depth": 10}]),
        &[("routes[0].src", "not_a_source")],
    );
    // S2a lane RTE: the audio routes. `wt` carries the S0d placeholder inputs `pm` and `am`.
    c.file(
        "a control source into an audio input",
        |v| {
            v["generators"]
                .as_array_mut()
                .unwrap()
                .push(json!({"id": "wt1", "kind": "wt"}));
            v["routes"] = json!([{"src": "env2", "dst": "wt1.pm", "depth": 10}]);
        },
        &[("routes[0].src", "not_audio_source")],
    );
    c.file(
        "velocity into an audio input",
        |v| {
            v["generators"]
                .as_array_mut()
                .unwrap()
                .push(json!({"id": "wt1", "kind": "wt"}));
            v["routes"] = json!([{"src": "velocity", "dst": "wt1.am", "depth": 10}]);
        },
        &[("routes[0].src", "not_audio_source")],
    );
    c.file(
        "an oscillator into a wavetable's audio input is fine",
        |v| {
            v["generators"]
                .as_array_mut()
                .unwrap()
                .push(json!({"id": "wt1", "kind": "wt"}));
            v["routes"] = json!([{"src": "osc1", "dst": "wt1.pm", "depth": 10}]);
        },
        &[],
    );
    c.file(
        "an oscillator into the level of a wavetable",
        |v| {
            v["generators"]
                .as_array_mut()
                .unwrap()
                .push(json!({"id": "wt1", "kind": "wt"}));
            v["routes"] = json!([{"src": "osc1", "dst": "wt1.level", "depth": 10}]);
        },
        &[("routes[0].dst", "not_audio_dest")],
    );
    c.file(
        "a generator driving itself",
        |v| {
            v["generators"]
                .as_array_mut()
                .unwrap()
                .push(json!({"id": "wt1", "kind": "wt"}));
            v["routes"] = json!([{"src": "wt1", "dst": "wt1.pm", "depth": 10}]);
        },
        &[("routes[0]", "audio_cycle")],
    );
    c.file(
        "two wavetables feeding each other",
        |v| {
            let gens = v["generators"].as_array_mut().unwrap();
            gens.push(json!({"id": "wt1", "kind": "wt"}));
            gens.push(json!({"id": "wt2", "kind": "wt"}));
            v["routes"] = json!([
                {"src": "wt1", "dst": "wt2.pm", "depth": 10},
                {"src": "wt2", "dst": "wt1.am", "depth": 10}
            ]);
        },
        &[("routes[1]", "audio_cycle")],
    );
    c.file(
        "a route to a module that is not there",
        |v| v["routes"] = json!([{"src": "env2", "dst": "filter9.cutoff", "depth": 10}]),
        &[("routes[0].dst", "unknown_module")],
    );
    c.file(
        "a route to an enum",
        |v| v["routes"] = json!([{"src": "env2", "dst": "filter1.mode", "depth": 10}]),
        &[("routes[0].dst", "not_modulatable")],
    );
    c.file(
        "a route to a key nobody has",
        |v| v["routes"] = json!([{"src": "env2", "dst": "filter1.cutof", "depth": 10}]),
        &[("routes[0].dst", "unknown_param")],
    );
    c.file(
        "a route address that is not an address",
        |v| v["routes"][0]["dst"] = json!("nonsense"),
        &[("routes[0].dst", "bad_address")],
    );
    c.file(
        "a duplicate route",
        |v| {
            let r = v["routes"][0].clone();
            v["routes"].as_array_mut().unwrap().push(r);
        },
        &[("routes[4]", "duplicate_route")],
    );
    c.file(
        "a per-voice source into a global LFO",
        |v| v["routes"] = json!([{"src": "env2", "dst": "lfo2.rate", "depth": 10}]),
        &[("routes[0]", "route_rate_mismatch")],
    );
    c.file(
        "velocity into an effect",
        |v| v["routes"] = json!([{"src": "velocity", "dst": "reverb1.wet", "depth": 10}]),
        &[("routes[0]", "route_rate_mismatch")],
    );
    c.file(
        "slot 0",
        |v| v["slots"] = json!({"0": "filter1.cutoff"}),
        &[("slots.0", "bad_index")],
    );
    c.file(
        "slot 65",
        |v| v["slots"] = json!({"65": "filter1.cutoff"}),
        &[("slots.65", "bad_index")],
    );
    c.file(
        "a structural parameter in a slot",
        |v| v["slots"] = json!({"1": "osc1.table"}),
        &[("slots.1", "not_automatable")],
    );
    c.file(
        "a slot on a module that is not there",
        |v| v["slots"] = json!({"1": "filter9.cutoff"}),
        &[("slots.1", "unknown_module")],
    );
    c.file(
        "a slot that is not an address",
        |v| v["slots"] = json!({"1": "cutoff"}),
        &[("slots.1", "bad_address")],
    );

    // --- Files: the envelope of the file -----------------------------------------------------------------------
    c.file(
        "no schema",
        |v| {
            v.as_object_mut().unwrap().remove("schema");
        },
        &[("schema", "missing_schema")],
    );
    c.file(
        "a schema that is text",
        |v| v["schema"] = json!("1"),
        &[("schema", "bad_schema")],
    );
    c.file(
        "a negative schema",
        |v| v["schema"] = json!(-1),
        &[("schema", "bad_schema")],
    );
    c.file(
        "a fractional schema",
        |v| v["schema"] = json!(1.5),
        &[("schema", "bad_schema")],
    );
    c.file(
        "a newer schema",
        |v| v["schema"] = json!(2),
        &[("schema", "schema_too_new")],
    );
    c.file(
        "an unknown top-level field",
        |v| {
            v.as_object_mut()
                .unwrap()
                .insert("tempo".into(), json!(120));
        },
        &[("", "bad_file")],
    );
    c.file(
        "an unknown module field",
        |v| {
            v["generators"][0]
                .as_object_mut()
                .unwrap()
                .insert("colour".into(), json!("red"));
        },
        &[("", "bad_file")],
    );
    c.file(
        "a number for the engine",
        |v| v["engine"] = json!(5),
        &[("", "bad_file")],
    );
    c.file(
        "text for enabled",
        |v| v["generators"][0]["enabled"] = json!("yes"),
        &[("", "bad_file")],
    );
    c.file(
        "an array for a file",
        |v| *v = json!([1]),
        &[("schema", "bad_file")],
    );
    c.file(
        "a slot key that is not a number",
        |v| v["slots"] = json!({"x": "filter1.cutoff"}),
        &[("", "bad_file")],
    );
    c.file_text("empty text", "", &[("", "bad_json")]);
    c.file_text("truncated text", "{\"schema\": 1,", &[("", "bad_json")]);
    c.file_text("not JSON", "not json", &[("", "bad_json")]);
    c.file_text(
        "a repeated key",
        "{\"schema\":1,\"schema\":1}",
        &[("", "bad_file")],
    );
    let repeated = rich()
        .to_json()
        .replacen("\"semi\":0.0", "\"semi\":0.0,\"semi\":1.0", 1);
    c.file_text("a repeated parameter", &repeated, &[("", "bad_file")]);

    // --- Migration ---------------------------------------------------------------------------------------------
    c.file_text(
        "a v0 file whose lanes are not lists",
        "{\"schema\":0,\"lanes\":3}",
        &[("schema", "migration_failed")],
    );
    c.file_text(
        "a v0 file with a text generator list",
        "{\"schema\":0,\"generators\":\"osc\"}",
        &[("schema", "migration_failed")],
    );

    // --- Suggestions -------------------------------------------------------------------------------------------
    c.file_dym(
        "a mistyped key",
        |v| {
            v["voice_fx"][0]["params"]
                .as_object_mut()
                .unwrap()
                .insert("cutof".into(), json!(1));
        },
        &["cutoff"],
    );
    c.file_dym(
        "a mistyped enum item",
        |v| v["generators"][0]["params"]["wave"] = json!("sawtooth"),
        &["saw"],
    );
    c.file_dym(
        "a mistyped kind",
        |v| v["generators"][0]["kind"] = json!("svff"),
        &["svf"],
    );
    c.file_dym(
        "a mistyped module in a route",
        |v| v["routes"][0]["dst"] = json!("filtre1.cutoff"),
        &["filter1"],
    );
    c.file_dym(
        "a mistyped parameter in a route",
        |v| v["routes"][0]["dst"] = json!("filter1.cutof"),
        &["cutoff"],
    );
    c.file_dym(
        "a mistyped amp envelope",
        |v| v["voice"]["amp_env"] = json!("env3"),
        &["env1", "env2"],
    );
    c.file_dym(
        "a mistyped slot",
        |v| v["slots"] = json!({"1": "filtre1.cutoff"}),
        &["filter1"],
    );

    // --- Ops ---------------------------------------------------------------------------------------------------
    let p = rich();
    c.op(
        "set: no such module",
        &p,
        &set("filtre1.cutoff", 1000.0),
        "unknown_module",
    );
    c.op(
        "set: a key of another kind",
        &p,
        &set("osc1.cutoff", 1000.0),
        "unknown_param",
    );
    c.op(
        "set: out of range",
        &p,
        &set("filter1.cutoff", 1.0e9),
        "out_of_range",
    );
    c.op(
        "set: NaN",
        &p,
        &set("filter1.cutoff", f32::NAN),
        "non_finite",
    );
    c.op(
        "set: infinity",
        &p,
        &set("filter1.cutoff", f32::INFINITY),
        "non_finite",
    );
    c.op(
        "set: not integral",
        &p,
        &set("voice.polyphony", 2.5),
        "not_integral",
    );
    c.op(
        "set: an enum index past the end",
        &p,
        &set("filter1.mode", 9.0),
        "out_of_range",
    );
    c.op(
        "set: a toggle of 2",
        &p,
        &set("osc1.sync", 2.0),
        "out_of_range",
    );
    c.op(
        "set: an LFO to voice rate while it drives an effect",
        &p,
        &set("lfo2.scope", 0.0),
        "route_rate_mismatch",
    );
    c.op(
        "add: a taken id",
        &p,
        &Op::AddModule {
            at: at(Section::Generators, 1),
            module: module("osc1", ModuleKind::Osc),
        },
        "duplicate_id",
    );
    c.op(
        "add: the wrong section",
        &p,
        &Op::AddModule {
            at: at(Section::Fx, 0),
            module: module("osc2", ModuleKind::Osc),
        },
        "wrong_section",
    );
    c.op(
        "add: past the end",
        &p,
        &Op::AddModule {
            at: at(Section::Generators, 5),
            module: module("osc2", ModuleKind::Osc),
        },
        "bad_index",
    );
    c.op(
        "add: too few values",
        &p,
        &Op::AddModule {
            at: at(Section::Generators, 1),
            module: {
                let mut x = module("osc2", ModuleKind::Osc);
                x.values.pop();
                x
            },
        },
        "wrong_value_count",
    );
    c.op(
        "add: a value out of range",
        &p,
        &Op::AddModule {
            at: at(Section::Generators, 1),
            module: {
                let mut x = module("osc2", ModuleKind::Osc);
                x.values[OscParam::Semi.index()] = 999.0;
                x
            },
        },
        "out_of_range",
    );
    let mut full = p.clone();
    for i in 0..3 {
        let n = full.generators.len();
        apply(
            &mut full,
            Op::AddModule {
                at: at(Section::Generators, n),
                module: module(&format!("osc{}", i + 2), ModuleKind::Osc),
            },
        )
        .unwrap();
    }
    c.op(
        "add: a fifth generator",
        &full,
        &Op::AddModule {
            at: at(Section::Generators, 4),
            module: module("osc9", ModuleKind::Osc),
        },
        "too_many",
    );
    c.op(
        "remove: no such module",
        &p,
        &Op::RemoveModule { id: m("filtre1") },
        "unknown_module",
    );
    c.op(
        "remove: the amplitude envelope",
        &p,
        &Op::RemoveModule { id: m("env1") },
        "in_use_as_amp_env",
    );
    c.op(
        "move: into another section",
        &p,
        &Op::Move {
            id: m("osc1"),
            to: at(Section::Fx, 0),
        },
        "wrong_section",
    );
    c.op(
        "move: past the end",
        &p,
        &Op::Move {
            id: m("osc1"),
            to: at(Section::Generators, 3),
        },
        "bad_index",
    );
    c.op(
        "move: no such module",
        &p,
        &Op::Move {
            id: m("oscc"),
            to: at(Section::Generators, 0),
        },
        "unknown_module",
    );
    c.op(
        "route: from an oscillator to a filter",
        &p,
        &route_op(0, ModSource::Module(m("osc1")), "filter1.cutoff", 1.0),
        "not_audio_dest",
    );
    c.op(
        "route: from a filter",
        &p,
        &route_op(0, ModSource::Module(m("filter1")), "osc1.semi", 1.0),
        "not_a_source",
    );
    {
        // S2a lane RTE: the audio routes through the op.
        let mut a = p.clone();
        for id in ["wt1", "wt2"] {
            let index = a.generators.len();
            apply(
                &mut a,
                &Op::AddModule {
                    at: Position {
                        section: Section::Generators,
                        index,
                    },
                    module: ModuleInstance::new(m(id), ModuleKind::Wt),
                },
            )
            .unwrap();
        }
        c.op(
            "route: an envelope into an audio input",
            &a,
            &route_op(0, ModSource::Module(m("env2")), "wt1.pm", 1.0),
            "not_audio_source",
        );
        c.op(
            "route: a macro into an audio input",
            &a,
            &route_op(
                0,
                ModSource::Macro(MacroIndex::new(0).unwrap()),
                "wt1.pm",
                1.0,
            ),
            "not_audio_source",
        );
        c.op(
            "route: an oscillator into a wavetable's level",
            &a,
            &route_op(0, ModSource::Module(m("osc1")), "wt1.level", 1.0),
            "not_audio_dest",
        );
        c.op(
            "route: a wavetable into itself",
            &a,
            &route_op(0, ModSource::Module(m("wt1")), "wt1.pm", 1.0),
            "audio_cycle",
        );
        apply(
            &mut a,
            route_op(0, ModSource::Module(m("wt1")), "wt2.pm", 50.0),
        )
        .unwrap();
        c.op(
            "route: back from the carrier",
            &a,
            &route_op(1, ModSource::Module(m("wt2")), "wt1.am", 50.0),
            "audio_cycle",
        );
    }
    c.op(
        "route: to an enum",
        &p,
        &route_op(0, ModSource::Velocity, "filter1.mode", 1.0),
        "not_modulatable",
    );
    c.op(
        "route: to no module",
        &p,
        &route_op(0, ModSource::Velocity, "filtre1.cutoff", 1.0),
        "unknown_module",
    );
    c.op(
        "route: from no module",
        &p,
        &route_op(0, ModSource::Module(m("lfo9")), "filter1.cutoff", 1.0),
        "unknown_module",
    );
    c.op(
        "route: twice",
        &p,
        &route_op(0, ModSource::Module(m("env2")), "filter1.cutoff", 1.0),
        "duplicate_route",
    );
    c.op(
        "route: at an index past the end",
        &p,
        &route_op(9, ModSource::Velocity, "filter1.cutoff", 1.0),
        "bad_index",
    );
    c.op(
        "route: a depth past 100",
        &p,
        &route_op(0, ModSource::Velocity, "filter1.cutoff", 101.0),
        "out_of_range",
    );
    c.op(
        "route: velocity into an effect",
        &p,
        &route_op(0, ModSource::Velocity, "reverb1.wet", 1.0),
        "route_rate_mismatch",
    );
    c.op(
        "route: an envelope into a global LFO",
        &p,
        &route_op(0, ModSource::Module(m("env2")), "lfo2.rate", 1.0),
        "route_rate_mismatch",
    );
    let mut routed = p.clone();
    for i in 0..60 {
        // fill up to the limit with distinct (source, destination) pairs: 8 macros x destinations
        let dsts = mod_targets(&routed);
        let src = ModSource::Macro(MacroIndex::new(i % 8).unwrap());
        let dst = dsts[i / 8 % dsts.len()].clone();
        let index = routed.routes.len();
        let _ = apply(
            &mut routed,
            Op::InsertRoute {
                index,
                route: Route {
                    src,
                    dst,
                    depth: 1.0,
                },
            },
        );
        if routed.routes.len() == 64 {
            break;
        }
    }
    assert_eq!(routed.routes.len(), 64, "the test fills the route table");
    c.op(
        "route: the 65th",
        &routed,
        &route_op(0, ModSource::Key, "voice.pan", 1.0),
        "too_many",
    );
    c.op(
        "unroute: no such route",
        &p,
        &Op::RemoveRoute {
            route: "velocity -> filter1.cutoff".parse().unwrap(),
        },
        "no_such_route",
    );
    c.op(
        "depth: no such route",
        &p,
        &Op::SetDepth {
            route: "velocity -> filter1.cutoff".parse().unwrap(),
            depth: 5.0,
        },
        "no_such_route",
    );
    c.op(
        "depth: past 100",
        &p,
        &Op::SetDepth {
            route: "env2 -> filter1.cutoff".parse().unwrap(),
            depth: 500.0,
        },
        "out_of_range",
    );
    c.op(
        "enable: the amplitude envelope off",
        &p,
        &Op::SetEnabled {
            id: m("env1"),
            enabled: false,
        },
        "in_use_as_amp_env",
    );
    c.op(
        "enable: no such module",
        &p,
        &Op::SetEnabled {
            id: m("env9"),
            enabled: false,
        },
        "unknown_module",
    );
    c.op(
        "amp_env: an oscillator",
        &p,
        &Op::SetAmpEnv { id: m("osc1") },
        "wrong_kind",
    );
    c.op(
        "amp_env: no such module",
        &p,
        &Op::SetAmpEnv { id: m("env9") },
        "unknown_module",
    );
    let mut off = p.clone();
    apply(
        &mut off,
        Op::SetEnabled {
            id: m("env2"),
            enabled: false,
        },
    )
    .unwrap();
    c.op(
        "amp_env: a disabled envelope",
        &off,
        &Op::SetAmpEnv { id: m("env2") },
        "module_disabled",
    );
    c.op(
        "macro: past 1",
        &p,
        &Op::SetMacro {
            index: MacroIndex::new(0).unwrap(),
            value: 1.5,
        },
        "out_of_range",
    );
    c.op(
        "macro: NaN",
        &p,
        &Op::SetMacro {
            index: MacroIndex::new(0).unwrap(),
            value: f32::NAN,
        },
        "non_finite",
    );
    c.op(
        "slot: a structural parameter",
        &p,
        &Op::SetSlot {
            slot: SlotIndex::new(1).unwrap(),
            addr: Some("osc1.table".parse().unwrap()),
        },
        "not_automatable",
    );
    c.op(
        "slot: no such module",
        &p,
        &Op::SetSlot {
            slot: SlotIndex::new(1).unwrap(),
            addr: Some("filtre1.cutoff".parse().unwrap()),
        },
        "unknown_module",
    );
    c.op(
        "slot: a key of another kind",
        &p,
        &Op::SetSlot {
            slot: SlotIndex::new(1).unwrap(),
            addr: Some("osc1.cutoff".parse().unwrap()),
        },
        "unknown_param",
    );
    c.op(
        "label: no such module",
        &p,
        &Op::SetLabel {
            id: m("filtre1"),
            label: Some("x".into()),
        },
        "unknown_module",
    );

    // --- The ops JSON ------------------------------------------------------------------------------------------
    c.wire(
        "wire: a mistyped key",
        &p,
        r#"[{"set":"filter1.cutof","to":1000}]"#,
        &[("ops[0].set", "unknown_param")],
    );
    c.wire(
        "wire: a mistyped module",
        &p,
        r#"[{"set":"filtre1.cutoff","to":1000}]"#,
        &[("ops[0].set", "unknown_module")],
    );
    c.wire(
        "wire: a number for an enum",
        &p,
        r#"[{"set":"osc1.wave","to":2}]"#,
        &[("ops[0].to", "bad_value")],
    );
    c.wire(
        "wire: a mistyped enum item",
        &p,
        r#"[{"set":"osc1.wave","to":"sawtooth"}]"#,
        &[("ops[0].to", "bad_value")],
    );
    c.wire(
        "wire: 1e39 as a number",
        &p,
        r#"[{"set":"osc1.semi","to":1e39}]"#,
        &[("ops[0].to", "non_finite")],
    );
    c.wire(
        "wire: 1e39 as text",
        &p,
        r#"[{"set":"osc1.semi","to":"1e39"}]"#,
        &[("ops[0].to", "non_finite")],
    );
    c.wire(
        "wire: the wrong unit",
        &p,
        r#"[{"set":"osc1.semi","to":"7Hz"}]"#,
        &[("ops[0].to", "wrong_unit")],
    );
    c.wire(
        "wire: -0.0 is accepted",
        &p,
        r#"[{"set":"osc1.semi","to":-0.0}]"#,
        &[],
    );
    c.wire(
        "wire: a phase of 360 is accepted",
        &p,
        r#"[{"set":"osc1.phase","to":360}]"#,
        &[],
    );
    c.wire(
        "wire: set without to",
        &p,
        r#"[{"set":"osc1.semi"}]"#,
        &[("ops[0].to", "bad_op")],
    );
    c.wire(
        "wire: no verb",
        &p,
        r#"[{"frobnicate":1}]"#,
        &[("ops[0]", "unknown_op")],
    );
    c.wire(
        "wire: a key the verb does not use",
        &p,
        r#"[{"set":"osc1.semi","to":1,"extra":3}]"#,
        &[("ops[0].extra", "bad_op")],
    );
    c.wire(
        "wire: an unknown kind",
        &p,
        r#"[{"add":"banana"}]"#,
        &[("ops[0].add", "unknown_kind")],
    );
    c.wire(
        "wire: a reserved id",
        &p,
        r#"[{"add":"osc","id":"voice"}]"#,
        &[("ops[0].id", "reserved_id")],
    );
    c.wire(
        "wire: a taken id",
        &p,
        r#"[{"add":"osc","id":"osc1"}]"#,
        &[("ops[0]", "duplicate_id")],
    );
    c.wire(
        "wire: a mistyped section",
        &p,
        r#"[{"add":"osc","at":"generator[0]"}]"#,
        &[("ops[0].at", "bad_section")],
    );
    c.wire(
        "wire: a position past the end",
        &p,
        r#"[{"add":"osc","at":"generators[9]"}]"#,
        &[("ops[0].at", "bad_index")],
    );
    c.wire(
        "wire: a bad parameter in add",
        &p,
        r#"[{"add":"osc","params":{"semi":"loud"}}]"#,
        &[("ops[0].params.semi", "bad_value")],
    );
    c.wire(
        "wire: an unknown parameter in add",
        &p,
        r#"[{"add":"osc","params":{"sem":1}}]"#,
        &[("ops[0].params.sem", "unknown_param")],
    );
    c.wire(
        "wire: removing the amplitude envelope",
        &p,
        r#"[{"remove":"env1"}]"#,
        &[("ops[0]", "in_use_as_amp_env")],
    );
    c.wire(
        "wire: disabling the amplitude envelope",
        &p,
        r#"[{"enable":"env1","to":false}]"#,
        &[("ops[0]", "in_use_as_amp_env")],
    );
    c.wire(
        "wire: a mistyped route",
        &p,
        r#"[{"route":"velocity -> filter1.cutof","depth":1}]"#,
        &[("ops[0].route", "unknown_param")],
    );
    c.wire(
        "wire: a route that is not one",
        &p,
        r#"[{"route":"nonsense","depth":1}]"#,
        &[("ops[0].route", "bad_address")],
    );
    c.wire(
        "wire: a depth in octaves is accepted",
        &p,
        r#"[{"route":"velocity -> filter1.cutoff","depth":"+1oct"}]"#,
        &[],
    );
    c.wire(
        "wire: a depth that is a word",
        &p,
        r#"[{"route":"velocity -> filter1.cutoff","depth":"loud"}]"#,
        &[("ops[0].depth", "bad_value")],
    );
    c.wire(
        "wire: an index for an existing route",
        &p,
        r#"[{"route":"env2 -> filter1.cutoff","depth":3,"index":1}]"#,
        &[("ops[0].index", "duplicate_route")],
    );
    c.wire(
        "wire: macro 0",
        &p,
        r#"[{"macro":0,"to":0.5}]"#,
        &[("ops[0].macro", "bad_index")],
    );
    c.wire(
        "wire: macro 9",
        &p,
        r#"[{"macro":9,"to":0.5}]"#,
        &[("ops[0].macro", "bad_index")],
    );
    c.wire(
        "wire: a macro with nothing to do",
        &p,
        r#"[{"macro":1}]"#,
        &[("ops[0]", "bad_op")],
    );
    c.wire(
        "wire: a macro past 1",
        &p,
        r#"[{"macro":1,"to":1.5}]"#,
        &[("ops[0].to", "out_of_range")],
    );
    c.wire(
        "wire: slot 65",
        &p,
        r#"[{"slot":65,"to":null}]"#,
        &[("ops[0].slot", "bad_index")],
    );
    c.wire(
        "wire: a mistyped slot target",
        &p,
        r#"[{"slot":1,"to":"filter1.cutof"}]"#,
        &[("ops[0].to", "unknown_param")],
    );
    c.wire(
        "wire: a string for the list",
        &p,
        r#""set osc1.semi""#,
        &[("ops", "bad_file")],
    );
    c.wire(
        "wire: a number for an op",
        &p,
        r"[1]",
        &[("ops[0]", "bad_op")],
    );
    c.wire(
        "wire: the second op fails",
        &p,
        r#"[{"set":"osc1.semi","to":1},{"set":"osc1.semi","to":"loud"}]"#,
        &[("ops[1].to", "bad_value")],
    );
    c.wire(
        "wire: an op may use an earlier add",
        &p,
        r#"[{"add":"osc","id":"osc9"},{"set":"osc9.semi","to":3}]"#,
        &[],
    );
    c.wire("wire: no ops is fine", &p, "[]", &[]);
    c.wire("wire: not JSON", &p, "{", &[("ops", "bad_json")]);
    c.wire(
        "wire: a move into another section",
        &p,
        r#"[{"move":"osc1","to":"fx[0]"}]"#,
        &[("ops[0]", "wrong_section")],
    );
    c.wire(
        "wire: text for enable",
        &p,
        r#"[{"enable":"env2","to":"no"}]"#,
        &[("ops[0].to", "bad_op")],
    );
    c.wire(
        "wire: a mistyped meta field",
        &p,
        r#"[{"meta":{"nme":"x"}}]"#,
        &[("ops[0].meta.nme", "bad_op")],
    );
    c.wire(
        "wire: tags that are text",
        &p,
        r#"[{"meta":{"tags":"x"}}]"#,
        &[("ops[0].meta.tags", "bad_op")],
    );
    c.wire_dym(
        "wire: did you mean a key",
        &p,
        r#"[{"set":"filter1.cutof","to":1000}]"#,
        &["cutoff"],
    );
    c.wire_dym(
        "wire: did you mean a module",
        &p,
        r#"[{"set":"filtre1.cutoff","to":1000}]"#,
        &["filter1"],
    );
    c.wire_dym(
        "wire: did you mean an item",
        &p,
        r#"[{"set":"osc1.wave","to":"sawtooth"}]"#,
        &["saw"],
    );
    c.wire_dym(
        "wire: did you mean a verb",
        &p,
        r#"[{"sett":"osc1.semi","to":1}]"#,
        &["set"],
    );
    c.wire_dym(
        "wire: did you mean a kind",
        &p,
        r#"[{"add":"svff"}]"#,
        &["svf"],
    );
    c.wire_dym(
        "wire: did you mean a section",
        &p,
        r#"[{"add":"osc","at":"generator[0]"}]"#,
        &["generators"],
    );
    c.wire_dym(
        "wire: did you mean a route target",
        &p,
        r#"[{"route":"velocity -> filter1.cutof","depth":1}]"#,
        &["cutoff"],
    );
    for (text, want) in [
        ("osc1.semi=3,filter1.cutoff=800Hz", &[][..]),
        ("osc1.semi", &[("set[0]", "bad_op")][..]),
        ("osc1.semi=loud", &[("set[0].to", "bad_value")][..]),
        ("filter1.cutof=1", &[("set[0].set", "unknown_param")][..]),
    ] {
        let got = match wire::set_ops(&p, text) {
            Ok(_) => Vec::new(),
            Err(inv) => inv.0.into_iter().map(|v| (v.path, v.error)).collect(),
        };
        if want.is_empty() {
            assert!(got.is_empty(), "{text}");
        } else {
            c.expect(&format!("set_ops {text}"), &got, want);
        }
    }

    // --- validate: what no op or file can produce, and the order of violations ---------------------------------
    let mut q = rich();
    q.generators[0].values[OscParam::Phase.index()] = 360.0;
    q.generators[0].values[OscParam::Semi.index()] = -0.0;
    q.macros[0].value = -0.0;
    q.routes[0].depth = -0.0;
    let got: Vec<(String, PatchError)> = violations(&q)
        .into_iter()
        .map(|v| (v.path, v.error))
        .collect();
    c.expect(
        "validate: values that have a canonical spelling",
        &got,
        &[
            ("generators[0].params.semi", "not_canonical"),
            ("generators[0].params.phase", "not_canonical"),
            ("routes[0].depth", "not_canonical"),
            ("macros[0].value", "not_canonical"),
        ],
    );
    let mut q = rich();
    q.generators[0].values.pop();
    let got: Vec<(String, PatchError)> = violations(&q)
        .into_iter()
        .map(|v| (v.path, v.error))
        .collect();
    c.expect(
        "validate: a module with a missing value",
        &got,
        &[("generators[0]", "wrong_value_count")],
    );

    c.done();
}

/// Canonical storage, checked on the stored bits: `-0.0` is `0.0`, the top of a periodic range is its bottom, and
/// the three ways in (file, op, wire) agree.
#[test]
fn values_are_stored_in_one_spelling() {
    let p = rich();
    let mut v = rich_json();
    v["generators"][0]["params"]["semi"] = json!(-0.0);
    v["generators"][0]["params"]["phase"] = json!(360);
    v["routes"][0]["depth"] = json!(-0.0);
    v["macros"][1]["value"] = json!(-0.0);
    let from_file = load(&v.to_string()).unwrap().patch;
    let semi = from_file.generators[0].values[OscParam::Semi.index()];
    let phase = from_file.generators[0].values[OscParam::Phase.index()];
    assert_eq!(semi.to_bits(), 0.0f32.to_bits(), "-0.0 is stored as +0.0");
    assert_eq!(
        phase.to_bits(),
        0.0f32.to_bits(),
        "a phase of 360 is stored as 0"
    );
    assert_eq!(from_file.routes[0].depth.to_bits(), 0.0f32.to_bits());
    assert_eq!(from_file.macros[1].value.to_bits(), 0.0f32.to_bits());
    validate(&from_file).unwrap();

    // The op and the wire form agree with the file.
    let mut by_op = p.clone();
    apply(&mut by_op, set("osc1.semi", -0.0)).unwrap();
    apply(&mut by_op, set("osc1.phase", 360.0)).unwrap();
    let mut by_wire = p.clone();
    let ops = wire::resolve_str(
        &by_wire,
        r#"[{"set":"osc1.semi","to":-0.0},{"set":"osc1.phase","to":"360deg"}]"#,
    )
    .unwrap();
    for op in &ops {
        apply(&mut by_wire, op).unwrap();
    }
    assert!(identical(&by_op, &by_wire));
    assert_eq!(
        by_op.generators[0].values[OscParam::Phase.index()].to_bits(),
        0.0f32.to_bits()
    );
    // ... and 360 degrees and 0 degrees are the same patch, byte for byte.
    assert_eq!(exact_hash(&by_op), exact_hash(&p));
    assert_eq!(by_op.to_json(), p.to_json());

    // The inverse of storing 360 is storing the old value: the old value is never 360.
    let mut q = p.clone();
    let inv = apply(&mut q, set("osc1.phase", 90.0)).unwrap();
    apply(&mut q, set("osc1.phase", 360.0)).unwrap();
    assert_eq!(q.generators[0].values[OscParam::Phase.index()], 0.0);
    let inv2 = apply(&mut q, set("osc1.phase", 90.0)).unwrap();
    assert_eq!(inv2, vec![set("osc1.phase", 0.0)]);
    assert_eq!(inv, vec![set("osc1.phase", 0.0)]);
}

/// What a person gets when they type a number for an enum: the items, in the message and in the hint.
#[test]
fn a_number_for_an_enum_lists_the_items() {
    let inv = load(&{
        let mut v = rich_json();
        v["generators"][0]["params"]["wave"] = json!(2);
        v.to_string()
    })
    .unwrap_err();
    let e = &inv.0[0].error;
    assert_eq!(e.code(), "bad_value");
    for item in ["sine", "triangle", "saw", "pulse", "table"] {
        assert!(e.hint().contains(item), "{}", e.hint());
    }
    assert!(e.message().contains("wave"));
}

/// The amplitude envelope is protected on every path that could lose it, and the refusal does not change anything.
#[test]
fn the_amplitude_envelope_cannot_be_lost() {
    let p = rich();
    for op in [
        Op::RemoveModule { id: m("env1") },
        Op::SetEnabled {
            id: m("env1"),
            enabled: false,
        },
    ] {
        let mut q = p.clone();
        let e = apply(&mut q, &op).unwrap_err();
        assert_eq!(e.code(), "in_use_as_amp_env");
        assert!(
            e.hint().contains("amp_env"),
            "the hint says how to get out: {}",
            e.hint()
        );
        assert!(identical(&q, &p));
    }
    // Name another envelope first and the old one can go.
    let mut q = p.clone();
    apply(&mut q, Op::SetAmpEnv { id: m("env2") }).unwrap();
    apply(
        &mut q,
        Op::SetEnabled {
            id: m("env1"),
            enabled: false,
        },
    )
    .unwrap();
    apply(&mut q, Op::RemoveModule { id: m("env1") }).unwrap();
    validate(&q).unwrap();
}

/// The rate rule holds for ops that add routes and for ops that change an LFO's scope: a per-voice source never
/// ends up driving a global target.
#[test]
fn the_rate_rule_holds_for_routes_and_for_scope_changes() {
    let mut p = rich();
    // lfo2 is global and drives the reverb; making it per voice would send a per-voice source to an effect.
    let e = apply(&mut p, set("lfo2.scope", 0.0)).unwrap_err();
    assert_eq!(e.code(), "route_rate_mismatch");
    // lfo1 is per voice and drives only a per-voice target (osc1.semi): it may go global and come back.
    apply(&mut p, set("lfo1.scope", 1.0)).unwrap();
    apply(&mut p, set("lfo1.scope", 0.0)).unwrap();
    // Once a per-voice source (velocity) drives lfo1's rate, lfo1 must stay per voice: a global LFO is a global
    // target, and velocity cannot drive it.
    let index = p.routes.len();
    apply(
        &mut p,
        route_op(index, ModSource::Velocity, "lfo1.rate", 10.0),
    )
    .unwrap();
    let e = apply(&mut p, set("lfo1.scope", 1.0)).unwrap_err();
    assert_eq!(
        e.code(),
        "route_rate_mismatch",
        "velocity -> lfo1.rate would reach a global LFO"
    );
    validate(&p).unwrap();
}

/// `validate` reports every violation of a patch, not the first.
#[test]
fn validate_reports_every_violation_in_document_order() {
    let mut p = rich();
    p.voice.values[enth_patch::catalogue::VoiceParam::Polyphony.index()] = 2.5;
    p.generators[0].values[OscParam::Semi.index()] = 99.0;
    p.macros[3].value = 7.0;
    p.slots[5] = Some("filter9.cutoff".parse().unwrap());
    let inv = validate(&p).unwrap_err();
    assert_eq!(
        inv.codes(),
        [
            "not_integral",
            "out_of_range",
            "out_of_range",
            "unknown_module"
        ]
    );
    let paths: Vec<&str> = inv.0.iter().map(|v| v.path.as_str()).collect();
    assert_eq!(
        paths,
        [
            "voice.polyphony",
            "generators[0].params.semi",
            "macros[3].value",
            "slots.6"
        ]
    );
}
