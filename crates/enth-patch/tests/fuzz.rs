//! Fuzz-lite (S1b): damaged files, damaged text, arbitrary JSON, damaged ops and corrupted patches go into the
//! readers and the validator. Nothing may panic; every refusal carries a code from the published list, a message
//! and a hint; anything that is accepted is valid and round-trips exactly. Fixed proptest seed, default cases.

mod common;

use common::*;
use enth_patch::PatchError;
use enth_patch::addr::{ModSource, ModuleId, ParamAddr};
use enth_patch::canon::{canonical_hash, canonicalise, exact_hash};
use enth_patch::error::Invalid;
use enth_patch::file::{load, load_value};
use enth_patch::migrate::migrate;
use enth_patch::model::{Patch, Route, Section};
use enth_patch::ops::{apply, apply_all};
use enth_patch::validate::{validate, violations};
use enth_patch::wire;
use proptest::prelude::*;
use serde_json::{Map, Value, json};

fn typed(inv: &Invalid) -> Result<(), TestCaseError> {
    prop_assert!(!inv.0.is_empty(), "an empty list of violations");
    for v in &inv.0 {
        assert_typed(&v.error);
    }
    Ok(())
}

/// A file text must load into a valid patch that writes back to something that loads to the same patch, or be
/// refused with typed errors.
fn check_load(text: &str) -> Result<(), TestCaseError> {
    match load(text) {
        Ok(loaded) => {
            prop_assert!(validate(&loaded.patch).is_ok());
            let again = load(&loaded.patch.to_json()).expect("what we write, we read");
            // (A file may leave the engine stamp empty; writing stamps the current version.)
            let mut expected = loaded.patch.clone();
            if expected.engine.is_empty() {
                enth_patch::file::ENGINE_VERSION.clone_into(&mut expected.engine);
            }
            prop_assert!(identical(&again.patch, &expected));
            prop_assert_eq!(again.from_schema, enth_patch::file::SCHEMA);
        }
        Err(inv) => typed(&inv)?,
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------------------------
// Damaging JSON
// ---------------------------------------------------------------------------------------------------------------

fn donors() -> Vec<Value> {
    vec![
        Value::Null,
        json!(true),
        json!(false),
        json!(0),
        json!(1),
        json!(-1),
        json!(0.5),
        json!(-0.0),
        json!(1.0e39),
        json!(-1.0e39),
        json!(1.0e-45),
        json!(360),
        json!(360.0),
        json!(u64::MAX),
        json!(i64::MIN),
        json!(2),
        json!(""),
        json!("x"),
        json!("saw"),
        json!("sine"),
        json!("440Hz"),
        json!("NaN"),
        json!("inf"),
        json!("1e39"),
        json!("-0"),
        json!("osc1"),
        json!("osc1.semi"),
        json!("voice.pan"),
        json!("lfo1 -> osc1.semi"),
        json!("velocity -> voice.pan"),
        json!("generators[0]"),
        json!("\u{0}\u{1f}\u{202e}日本"),
        json!([]),
        json!([1, 2]),
        json!([[]]),
        json!({}),
        json!({"a": 1}),
        json!({"id": "osc1", "kind": "osc"}),
        Value::String("a".repeat(5000)),
    ]
}

fn pointers(v: &Value, here: &mut String, out: &mut Vec<String>) {
    out.push(here.clone());
    let esc = |k: &str| k.replace('~', "~0").replace('/', "~1");
    match v {
        Value::Object(m) => {
            for (k, x) in m {
                let n = here.len();
                here.push('/');
                here.push_str(&esc(k));
                pointers(x, here, out);
                here.truncate(n);
            }
        }
        Value::Array(a) => {
            for (i, x) in a.iter().enumerate() {
                let n = here.len();
                here.push('/');
                here.push_str(&i.to_string());
                pointers(x, here, out);
                here.truncate(n);
            }
        }
        _ => {}
    }
}

/// One damage at a place chosen by `pick`: replace, delete, rename a key, wrap in an array, or duplicate.
fn damage(v: &mut Value, pick: u32, what: u32, donor: &Value) {
    let mut ptrs = Vec::new();
    pointers(v, &mut String::new(), &mut ptrs);
    let at = ptrs[pick as usize % ptrs.len()].clone();
    if at.is_empty() {
        if what.is_multiple_of(4) {
            *v = donor.clone();
        }
        return;
    }
    let (parent, last) = at.rsplit_once('/').unwrap();
    let unesc = |k: &str| k.replace("~1", "/").replace("~0", "~");
    let Some(parent) = (if parent.is_empty() {
        Some(&mut *v)
    } else {
        v.pointer_mut(parent)
    }) else {
        return;
    };
    match parent {
        Value::Object(m) => {
            let key = unesc(last);
            match what % 6 {
                0 | 1 => {
                    m.insert(key, donor.clone());
                }
                2 => {
                    m.remove(&key);
                }
                3 => {
                    if let Some(x) = m.remove(&key) {
                        let mut k = key;
                        if what.is_multiple_of(2) {
                            k.push('x');
                        } else {
                            k.pop();
                        }
                        m.insert(k, x);
                    }
                }
                4 => {
                    if let Some(x) = m.get(&key).cloned() {
                        m.insert(key, Value::Array(vec![x]));
                    }
                }
                _ => {
                    m.insert(format!("{key}_extra"), donor.clone());
                }
            }
        }
        Value::Array(a) => {
            let Ok(i) = last.parse::<usize>() else { return };
            match what % 4 {
                0 | 1 => a[i] = donor.clone(),
                2 => {
                    a.remove(i);
                }
                _ => {
                    let x = a[i].clone();
                    a.insert(i, x);
                }
            }
        }
        _ => {}
    }
}

fn damaged(v: &Value, damages: &[(u32, u32, u8)]) -> Value {
    let donors = donors();
    let mut v = v.clone();
    for &(pick, what, d) in damages {
        damage(&mut v, pick, what, &donors[usize::from(d) % donors.len()]);
    }
    v
}

fn damages() -> impl Strategy<Value = Vec<(u32, u32, u8)>> {
    prop::collection::vec((any::<u32>(), any::<u32>(), any::<u8>()), 1..4)
}

proptest! {
    /// A real file with a few nodes replaced, removed, renamed or wrapped.
    #[test]
    fn damaged_files_are_refused_with_typed_errors_or_load_cleanly(setup in raws(0..40), d in damages(), schema in 0u32..3) {
        let p = build(&setup);
        let mut v: Value = serde_json::from_str(&p.to_json()).unwrap();
        if schema != 1 {
            v["schema"] = json!(schema);
        }
        let v = damaged(&v, &d);
        check_load(&v.to_string())?;
        match load_value(v.clone()) {
            Ok(l) => prop_assert!(validate(&l.patch).is_ok()),
            Err(inv) => typed(&inv)?,
        }
        match migrate(v) {
            Ok(m) => prop_assert_eq!(m.value["schema"].as_u64(), Some(1)),
            Err(e) => assert_typed(&e),
        }
    }

    /// The text of a file cut, spliced, with a character changed or inserted.
    #[test]
    fn damaged_text_is_refused_with_typed_errors_or_loads_cleanly(
        setup in raws(0..40), cuts in prop::collection::vec((any::<u16>(), any::<u16>(), any::<u8>()), 1..4),
    ) {
        let p = build(&setup);
        let mut chars: Vec<char> = p.to_json_pretty().chars().collect();
        let junk = ['{', '}', '[', ']', '"', ',', ':', '\\', '0', '-', 'e', '.', '\u{0}', 'é', '\n', ' '];
        for (a, b, kind) in cuts {
            if chars.is_empty() { break; }
            let at = usize::from(a) % chars.len();
            match kind % 4 {
                0 => chars.truncate(at),
                1 => { let end = (at + usize::from(b) % 8).min(chars.len()); chars.drain(at..end); }
                2 => chars[at] = junk[usize::from(b) % junk.len()],
                _ => chars.insert(at, junk[usize::from(b) % junk.len()]),
            }
        }
        check_load(&chars.iter().collect::<String>())?;
    }

    /// Wire ops from a real history, damaged the same way.
    #[test]
    fn damaged_ops_are_refused_with_typed_errors_or_apply(setup in raws(0..40), d in damages()) {
        let (ops, _) = build_ops(&setup);
        let base = base_patch();
        let v = damaged(&wire::to_json(&base, &ops), &d);
        match wire::resolve(&base, &v) {
            Ok(resolved) => {
                let mut p = base.clone();
                prop_assert!(apply_all(&mut p, &resolved).is_ok(), "resolve said {resolved:?} applies");
                prop_assert!(validate(&p).is_ok());
            }
            Err(inv) => typed(&inv)?,
        }
        match wire::resolve_str(&base, &v.to_string()) {
            Ok(_) => {}
            Err(inv) => typed(&inv)?,
        }
    }

    /// Arbitrary JSON built from the vocabulary of the file and the wire form, thrown at every reader.
    #[test]
    fn arbitrary_json_never_panics(v in json_tree(), setup in raws(0..20)) {
        let p = build(&setup);
        match load_value(v.clone()) {
            Ok(l) => prop_assert!(validate(&l.patch).is_ok()),
            Err(inv) => typed(&inv)?,
        }
        check_load(&v.to_string())?;
        if let Err(e) = migrate(v.clone()) { assert_typed(&e); }
        match wire::resolve(&p, &v) {
            Ok(_) => {}
            Err(inv) => typed(&inv)?,
        }
    }

    /// `set_ops` takes `key=value` text from the command line.
    #[test]
    fn arbitrary_set_text_never_panics(text in "[ -~]{0,40}", setup in raws(0..20)) {
        let p = build(&setup);
        match wire::set_ops(&p, &text) {
            Ok(_) => {}
            Err(inv) => typed(&inv)?,
        }
    }
}

const WORDS: &[&str] = &[
    "schema",
    "engine",
    "meta",
    "name",
    "author",
    "tags",
    "description",
    "voice",
    "master",
    "generators",
    "voice_fx",
    "fx",
    "modulators",
    "routes",
    "macros",
    "slots",
    "id",
    "kind",
    "label",
    "enabled",
    "params",
    "src",
    "dst",
    "depth",
    "value",
    "amp_env",
    "osc",
    "svf",
    "lfo",
    "adsr",
    "set",
    "add",
    "remove",
    "move",
    "route",
    "unroute",
    "enable",
    "macro",
    "slot",
    "to",
    "at",
    "index",
    "wave",
    "semi",
    "level",
    "cutoff",
    "rate",
    "attack",
    "mode",
    "pan",
    "polyphony",
    "osc1",
    "env1",
    "lfo1",
    "filter1",
    "velocity",
    "key",
    "modwheel",
    "macro1",
    "voice.pan",
    "osc1.semi",
    "lfo1 -> osc1.semi",
    "generators[0]",
    "fx[1]",
    "saw",
    "sine",
    "440Hz",
    "-6dB",
    "+1oct",
    "30%",
    "1e39",
    "NaN",
    "",
];

fn json_tree() -> impl Strategy<Value = Value> {
    let leaf = prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::from),
        (-3i64..400).prop_map(Value::from),
        (-2.0f64..2.0).prop_map(|x| json!(x)),
        Just(json!(1.0e39)),
        prop::sample::select(WORDS).prop_map(Value::from),
        "[ -~]{0,8}".prop_map(Value::from),
    ];
    leaf.prop_recursive(5, 48, 6, |inner| {
        prop_oneof![
            prop::collection::vec(inner.clone(), 0..5).prop_map(Value::Array),
            prop::collection::vec((prop::sample::select(WORDS), inner), 0..6).prop_map(|kv| {
                let mut m = Map::new();
                for (k, v) in kv {
                    m.insert(k.to_owned(), v);
                }
                Value::Object(m)
            }),
        ]
    })
    .prop_map(|mut v| {
        // Most trees would die at the top: give half of them a real schema number.
        if let Value::Object(m) = &mut v
            && m.len() % 2 == 0
        {
            m.insert("schema".to_owned(), json!(1));
        }
        v
    })
}

// ---------------------------------------------------------------------------------------------------------------
// Corrupted patches
// ---------------------------------------------------------------------------------------------------------------

/// Break a valid patch in one of many ways at the level of the typed model (no file layer in between).
fn corrupt(p: &mut Patch, kind: u8, pick: u8, bits: u32) {
    let ms: Vec<ModuleId> = p.modules().map(|(_, m)| m.id.clone()).collect();
    let id = ms[usize::from(pick) % ms.len()].clone();
    let sections = Section::ALL;
    match kind % 14 {
        0 | 1 => {
            let (pos, _) = p.find(&id).unwrap();
            let m = &mut p.section_mut(pos.section)[pos.index];
            let n = m.values.len();
            let bad = [
                f32::NAN,
                f32::INFINITY,
                -0.0,
                1.0e30,
                -1.0e30,
                0.5,
                f32::from_bits(bits),
            ];
            m.values[usize::from(pick) % n] = bad[bits as usize % bad.len()];
        }
        2 => {
            let (pos, _) = p.find(&id).unwrap();
            let m = &mut p.section_mut(pos.section)[pos.index];
            if bits.is_multiple_of(2) {
                m.values.pop();
            } else {
                m.values.push(0.0);
            }
        }
        3 => p.voice.amp_env = ModuleId::new("nonexistent").unwrap(),
        4 => p.voice.amp_env = id,
        5 => {
            let (pos, _) = p.find(&id).unwrap();
            let m = p.section_mut(pos.section).remove(pos.index);
            let to = sections[bits as usize % 4];
            p.section_mut(to).push(m);
        }
        6 => {
            let (pos, _) = p.find(&id).unwrap();
            let m = p.section(pos.section)[pos.index].clone();
            p.section_mut(pos.section).push(m);
        }
        7 => {
            let r = Route {
                src: ModSource::Module(id.clone()),
                dst: ParamAddr::module(id, "level").unwrap(),
                depth: [250.0, f32::NAN, -0.0, 0.0][bits as usize % 4],
            };
            p.routes.push(r.clone());
            if bits.is_multiple_of(3) {
                p.routes.push(r);
            }
        }
        8 => {
            p.slots[usize::from(pick) % 64] = Some(
                ParamAddr::module(id, "seed").unwrap_or_else(|_| "master.level".parse().unwrap()),
            );
        }
        9 => p.macros[usize::from(pick) % 8].value = [2.0, -1.0, f32::NAN, -0.0][bits as usize % 4],
        10 => p.master[usize::from(pick) % p.master.len()] = f32::NAN,
        11 => p.voice.values[usize::from(pick) % p.voice.values.len()] = f32::INFINITY,
        12 => {
            let (pos, _) = p.find(&id).unwrap();
            p.section_mut(pos.section)[pos.index].enabled = false;
        }
        _ => {
            let (pos, _) = p.find(&id).unwrap();
            for _ in 0..40 {
                let m = p.section(pos.section)[pos.index].clone();
                p.section_mut(pos.section).push(m);
            }
        }
    }
}

proptest! {
    /// `validate` is the gatekeeper: whatever the patch looks like, it answers (no panic), every violation is
    /// typed, and the patch is valid iff there are no violations. The canonical form and the hashes are best
    /// effort on invalid patches but must not panic either.
    #[test]
    fn validate_survives_corrupted_patches(
        setup in raws(1..40), hits in prop::collection::vec((any::<u8>(), any::<u8>(), any::<u32>()), 1..4),
    ) {
        let mut p = build(&setup);
        for (k, pick, bits) in hits {
            corrupt(&mut p, k, pick, bits);
        }
        let vs = violations(&p);
        for v in &vs {
            assert_typed(&v.error);
        }
        match validate(&p) {
            Ok(()) => prop_assert!(vs.is_empty()),
            Err(inv) => {
                prop_assert_eq!(inv.0.len(), vs.len());
                typed(&inv)?;
            }
        }
        let _ = (exact_hash(&p), canonical_hash(&p), canonicalise(&p));
        // Writing an invalid patch must not panic either (it may write something the reader refuses).
        let text = p.to_json();
        let _ = load(&text);
    }

    /// Ops on a corrupted patch: answer or refuse, never panic. (The editor only ever holds valid patches; this is
    /// about not turning a bug elsewhere into a crash here.)
    #[test]
    fn ops_survive_corrupted_patches(
        setup in raws(1..40), hits in prop::collection::vec((any::<u8>(), any::<u8>(), any::<u32>()), 1..3),
        rs in raws(1..10),
    ) {
        let mut p = build(&setup);
        for (k, pick, bits) in hits {
            corrupt(&mut p, k, pick, bits);
        }
        let mut fresh = 500;
        for r in rs {
            let op = make_op(&p, r, &mut fresh);
            if let Err(e) = apply(&mut p, &op) {
                assert_typed(&e);
            }
        }
    }
}

#[test]
fn the_error_vocabulary_is_closed() {
    // Every code a refusal can have is listed, once, in snake case.
    let codes = enth_patch::error::CODES;
    let mut seen = std::collections::BTreeSet::new();
    for c in codes {
        assert!(seen.insert(*c), "{c} listed twice");
        assert!(
            c.chars().all(|ch| ch.is_ascii_lowercase() || ch == '_'),
            "{c}"
        );
    }
    let e = PatchError::MissingSchema;
    assert!(codes.contains(&e.code()));
}
