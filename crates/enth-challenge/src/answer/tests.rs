//! Lane RC's tests of the answer code: `result-params-v1` pinned against the catalogue, round trips over mutated
//! answers (proptest), damage and truncation caught, the privacy proof, the refusals, the size budget per level, and
//! every answer through code v1's decoder.

use core::fmt::Write as _;

use enth_params::Kind;
use enth_patch::canon::{canonical_hash, exact_hash};
use enth_patch::catalogue::Builtin;
use enth_patch::{PatchMeta, SlotIndex, apply};
use proptest::prelude::*;
use sha2::{Digest, Sha256};

use super::*;
use crate::code::{CodeForm, TargetPrefix};
use crate::result::tests::{
    any_long_challenge, any_short_challenge, assert_damage_caught, assert_old_builds_say_update,
    assert_truncation_caught, symbols_of,
};
use crate::spec::SpecPreset;

// ---------------------------------------------------------------------------------------------------------------
// result-params-v1
// ---------------------------------------------------------------------------------------------------------------

/// Lowercase hex.
fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

/// The table as text: one line per entry, `owner index key coding default-bits`.
fn table_text() -> String {
    let mut out = String::new();
    for (owner, row) in RESULT_PARAMS_V1 {
        for (i, e) in row.iter().enumerate() {
            let coding = match e.coding {
                Coding::F32 => "f32".to_owned(),
                Coding::Int { min, bits } => format!("int {min} {bits}"),
                Coding::Enum { bits } => format!("enum {bits}"),
                Coding::Toggle => "toggle".to_owned(),
            };
            let _ = writeln!(
                out,
                "{owner} {i} {} {coding} {:08x}",
                e.key,
                e.default.to_bits()
            );
        }
    }
    out
}

/// `result-params-v1` is frozen: these bytes never change (a new parameter is appended, which changes this hash on
/// purpose and keeps every old line; a changed line is a new layout). Lane CAT appended S2a's kinds 7–17 after
/// `voice` and `master` at F1m, so this hash moved and [`f1e_entries_are_unchanged`] pins what F1e froze.
#[test]
fn result_params_v1_is_pinned() {
    let hex: String = hex(&Sha256::digest(table_text().as_bytes()));
    assert_eq!(
        hex,
        "c067471ff736aaba0533b2274dd3504937bd2d714ef9b281016527961d6ac733",
        "{}",
        table_text()
    );
}

/// F1e's entries, the only ones an answer code could carry before F1m, are byte for byte what F1e froze: the lines
/// of the table text whose owner and index F1e's table had hash to F1e's pin. Appended entries (S2a's kinds, `lfo`'s
/// tempo parameters) are skipped, so this holds whatever is appended.
#[test]
fn f1e_entries_are_unchanged() {
    const F1E_ROWS: [(&str, usize); 9] = [
        ("osc", 11),
        ("noise", 2),
        ("svf", 5),
        ("adsr", 5),
        ("lfo", 7),
        ("reverb", 10),
        ("comp", 7),
        ("voice", 5),
        ("master", 1),
    ];
    let mut old = String::new();
    for (owner, len) in F1E_ROWS {
        for line in table_text().lines() {
            let mut fields = line.split(' ');
            let (o, i) = (fields.next().unwrap(), fields.next().unwrap());
            if o == owner && i.parse::<usize>().unwrap() < len {
                old.push_str(line);
                old.push('\n');
            }
        }
    }
    assert_eq!(
        hex(&Sha256::digest(old.as_bytes())),
        "9f73b92ffa6c7f393d241226b8a299c0c1cfc3e5b2c5c8720b415842a9b6a9f1",
        "{old}"
    );
}

/// Every parameter of the live catalogue is in the table, in spec order, with a coding that holds every valid value
/// and the catalogue's default: a catalogue change that breaks this needs a new layout, never an edit of v1.
#[test]
fn result_params_v1_covers_the_catalogue() {
    let check = |owner: &str, row: &[ResultParam], specs: &[enth_params::ParamSpec]| {
        assert_eq!(
            row.len(),
            specs.len(),
            "{owner}: a parameter is missing from the table"
        );
        assert!(
            params_fit(row.len()),
            "{owner}: index {RUN_END} ends a run, so {} parameters do not fit",
            row.len()
        );
        for (e, s) in row.iter().zip(specs) {
            assert_eq!(e.key, s.key, "{owner}: order");
            assert_eq!(
                e.default.to_bits(),
                s.default.to_bits(),
                "{owner}.{}: default",
                e.key
            );
            match (e.coding, s.kind) {
                (Coding::F32, Kind::Float { .. }) | (Coding::Toggle, Kind::Toggle) => {}
                (Coding::Int { min: m, bits }, Kind::Int { min, max }) => {
                    assert!(
                        m <= min && i64::from(max) - i64::from(m) < 1 << bits,
                        "{owner}.{}",
                        e.key
                    );
                }
                (Coding::Enum { bits }, Kind::Enum(items)) => {
                    assert!(
                        items.len() < 1 << bits,
                        "{owner}.{}: room for one more item",
                        e.key
                    );
                }
                (c, k) => panic!("{owner}.{}: coding {c:?} for {k:?}", e.key),
            }
        }
    };
    // Every kind, the hidden S2a ones too (lane CAT added their rows at F1m), through the kind-to-row map: rows 7 and
    // 8 are voice and master, so kind k >= 7 is row k + 2. A listed kind must have a row; a later hidden kind may join
    // without one until its row is appended.
    for &kind in ModuleKind::ALL {
        let Some(row) = kind_row(kind) else {
            assert!(kind.is_hidden(), "{kind}: a listed kind needs a row");
            continue;
        };
        let (owner, entries) = RESULT_PARAMS_V1[row];
        assert_eq!(owner, kind.key(), "row {row}");
        check(owner, entries, kind.params());
    }
    assert_eq!(RESULT_PARAMS_V1[KINDS_F1E].0, "voice");
    assert_eq!(RESULT_PARAMS_V1[KINDS_F1E + 1].0, "master");
    check("voice", VOICE_V1, Builtin::Voice.params());
    check("master", MASTER_V1, Builtin::Master.params());
}

// ---------------------------------------------------------------------------------------------------------------
// Widths (D3, lane ANS): the fields against the patch model and the catalogue
// ---------------------------------------------------------------------------------------------------------------

/// The same relations as the compile-time asserts below the constants, with the failing section, kind or row named, so
/// a catalogue lane that adds the 64th parameter, the 65th kind or the 17th module of a section sees which one and by
/// how much. (The asserts stop the build first; this keeps the headroom written where a lane looks.)
#[test]
fn the_widths_cover_the_patch_model_and_the_catalogue() {
    let room = |bits: u32| 1usize << bits;
    // Kinds: every ModuleKind, the hidden reserved ones counted.
    assert!(
        kinds_fit(ModuleKind::ALL.len()),
        "{} kinds against {} (KIND_BITS {KIND_BITS})",
        ModuleKind::ALL.len(),
        room(KIND_BITS)
    );
    assert!(ModuleKind::ALL.iter().any(|k| k.is_hidden()));
    for (i, kind) in ModuleKind::ALL.iter().enumerate() {
        assert_eq!(
            *kind as usize, i,
            "{kind}: the kind field is the discriminant"
        );
    }
    // Places: every section cap.
    for section in enth_patch::Section::ALL {
        assert!(
            places_fit(section.max_len()),
            "{section}: {} modules against {} (PLACE_BITS {PLACE_BITS})",
            section.max_len(),
            room(PLACE_BITS)
        );
    }
    // Targets and id numbers: voice, master and a full patch.
    let total: usize = enth_patch::Section::ALL.iter().map(|s| s.max_len()).sum();
    assert_eq!(total, MODULE_CAP, "the section caps sum to the module cap");
    assert!(
        targets_fit(total),
        "voice, master and {total} modules against {} (TARGET_BITS {TARGET_BITS})",
        room(TARGET_BITS)
    );
    assert!(
        ids_fit(total),
        "{total} modules against id numbers 1..{} (ID_BITS {ID_BITS})",
        room(ID_BITS) - 1
    );
    // Routes and macros.
    assert!(
        routes_fit(enth_patch::model::MAX_ROUTES),
        "routes against ROUTE_BITS {ROUTE_BITS}"
    );
    assert!(
        enth_patch::model::MACROS <= room(MACRO_BITS),
        "macros against MACRO_BITS {MACRO_BITS}"
    );
    // Parameters: the last index stays below RUN_END, which ends a run.
    assert_eq!(RUN_END as usize, room(PARAM_BITS) - 1);
    for (owner, row) in RESULT_PARAMS_V1 {
        assert!(
            params_fit(row.len()),
            "{owner}: {} parameters against indices 0..{RUN_END}",
            row.len()
        );
    }
    for kind in ModuleKind::ALL {
        assert!(
            params_fit(kind.params().len()),
            "{kind}: {} catalogue parameters against indices 0..{RUN_END} (PARAM_BITS {PARAM_BITS})",
            kind.params().len()
        );
    }
    for builtin in Builtin::ALL {
        assert!(
            params_fit(builtin.params().len()),
            "{}: {} parameters",
            builtin.key(),
            builtin.params().len()
        );
    }
}

/// The guards' arithmetic at their edges, so an off-by-one cannot hide: each fits the largest value its field holds and
/// refuses the next one. A lane that raised `MAX_FX` to 17, added a 65th kind or gave a kind a 64th parameter would
/// fail the compile-time asserts (and the test above) through exactly these functions.
#[test]
fn each_guard_holds_the_last_value_and_refuses_the_next() {
    // Kinds 0 to 63.
    assert!(kinds_fit(64) && !kinds_fit(65));
    // Places 0 to 15: a section of 16, not 17 (MAX_FX 17 would fail).
    assert!(places_fit(16) && !places_fit(17));
    // Targets: 0 voice, 1 master, 2 + i for i up to 61, so 62 modules, not 63.
    assert!(targets_fit(62) && !targets_fit(63));
    // Route indices 0 to 63.
    assert!(routes_fit(64) && !routes_fit(65));
    // Parameter indices 0 to 62, 63 ending a run: 63 parameters, not 64 (an MSEG of 63 parameters still fits).
    assert!(params_fit(63) && !params_fit(64));
    assert!(params_fit(RUN_END as usize) && !params_fit(RUN_END as usize + 1));
    // Id numbers 1 to 63 cover 63 modules, not 64.
    assert!(ids_fit(63) && !ids_fit(64));
}

/// An answer code made on the F1e build, before the widening (D3), is not this build's: it has the right header and
/// check symbol, but its fields are narrower, so it must be refused cleanly (never read as another answer, never a
/// panic). Layout 1 had not reached a friend build, which is why it was widened in place. The four F1e fixture answers,
/// each pinned to the code this build gives it: the narrower fields read as an op, kind or parameter this build lacks
/// (`answer_too_new`) or as a field no encoder writes (`bad_field`).
#[test]
fn a_code_made_before_the_widening_is_refused() {
    let before = [
        (
            "F23G-7ZXR-014D-2PF0-0802-0000-002M-20RG-F1T1-B1ZR-0CK7-304S-TY",
            "answer_too_new",
        ),
        (
            "F23G-7ZXR-014D-2PF0-0R02-0000-002M-20RG-F1T1-B1ZH-824J-BG8A-DFWG-0Z8E-4YXG-8TPE-CB9H-WJ4Y-6E83-MDY8-0298-GJSW-0G39-6N7G-7K",
            "answer_too_new",
        ),
        (
            "F23G-7ZXR-014D-2PF0-1802-0000-002M-20RG-F1T1-B1ZG-S0NX-SJED-ZRHR-G243-AFY4-J508-J9E1-19NS-HYHZ-HRMG-SS0H-61H0-2FRK-S01Y-GW9X-V0HN-CWRP-K10M-JZS4-9WCW-G78V-S88H-6VR4-2Z8V-WG0J-H7T3-9089-522B-7J10-M4CC-GF0Z-DJD3-JSG6",
            "answer_too_new",
        ),
        (
            "F220-0000-0000-0004-0R02-0000-000F-C9AJ-8092-CVY2-5RR2-1M8J-RP3V-W9P8-02S0-HBSW-G646-93XF-JHEW-P0VD-9JV1-B1C",
            "bad_field",
        ),
    ];
    for (old, code) in before {
        let e = AnswerCode::decode(old).unwrap_err();
        assert_eq!(e.code(), code, "{} symbols: {e}", symbols_of(old).len());
    }
}

/// The widths the task needed: 62 modules, 63 parameters per owner (indices 0 to 62), 16 per section and 64 kinds,
/// where layout 1 at F1e held 30, 15, 8 and 16.
#[test]
fn the_widths_are_the_widened_ones() {
    assert_eq!(
        (TARGET_BITS, PARAM_BITS, RUN_END, PLACE_BITS, KIND_BITS),
        (6, 6, 63, 4, 6)
    );
    // Unchanged.
    assert_eq!(
        (OP_BITS, ROUTE_BITS, MACRO_BITS, ID_BITS, SOURCE_BITS),
        (4, 6, 3, 6, 3)
    );
}

/// The new fields at the bit level, with the writer and the reader the codec uses and no table row: a kind of 17
/// (`ladder`; 4 bits stop at 15), a parameter index of 40 (4 bits stop at 14, 15 ending a run), a place of 12 (3 bits
/// stop at 7) and a target of 40 (5 bits stop at 31). Each value would not fit the width layout 1 had at F1e.
#[test]
fn the_wider_fields_round_trip_at_the_bit_level() {
    const OLD: (u32, u32, u32, u32) = (5, 4, 3, 4); // target, parameter, place, kind
    let fields = [
        ("kind", 17u64, KIND_BITS, OLD.3),
        ("parameter", 40, PARAM_BITS, OLD.1),
        ("place", 12, PLACE_BITS, OLD.2),
        ("target", 40, TARGET_BITS, OLD.0),
    ];
    for (name, value, bits, old) in fields {
        assert!(
            value >= 1 << old,
            "{name}: {value} fits the old {old} bits, so it proves nothing"
        );
        assert!(value < 1 << bits, "{name}: {value} must fit {bits} bits");
    }
    // The AddModule order (kind, place), a run's (target, parameter), and the run's end, back to back, at odd offsets.
    for pad in 0..5 {
        let mut w = Bits::default();
        w.put(0, pad);
        w.put(17, KIND_BITS);
        w.put(12, PLACE_BITS);
        w.put(40, TARGET_BITS);
        w.put(40, PARAM_BITS);
        w.put(RUN_END, PARAM_BITS);
        assert_eq!(
            w.len(),
            (pad + KIND_BITS + PLACE_BITS + TARGET_BITS + 2 * PARAM_BITS) as usize
        );
        let syms = w.into_symbols();
        let mut r = Reader::at(&syms, pad as usize);
        assert_eq!(r.take(KIND_BITS as usize).unwrap(), 17);
        assert_eq!(r.take(PLACE_BITS as usize).unwrap(), 12);
        assert_eq!(r.take(TARGET_BITS as usize).unwrap(), 40);
        assert_eq!(r.take(PARAM_BITS as usize).unwrap(), 40);
        assert_eq!(r.take(PARAM_BITS as usize).unwrap(), RUN_END);
    }
    // The largest values each field holds, and the one a run reserves.
    let mut w = Bits::default();
    w.put((1 << KIND_BITS) - 1, KIND_BITS);
    w.put((1 << PLACE_BITS) - 1, PLACE_BITS);
    w.put((1 << TARGET_BITS) - 1, TARGET_BITS);
    w.put(RUN_END - 1, PARAM_BITS);
    let syms = w.into_symbols();
    let mut r = Reader::at(&syms, 0);
    assert_eq!(r.take(KIND_BITS as usize).unwrap(), 63);
    assert_eq!(r.take(PLACE_BITS as usize).unwrap(), 15);
    assert_eq!(r.take(TARGET_BITS as usize).unwrap(), 63);
    assert_eq!(r.take(PARAM_BITS as usize).unwrap(), 62);
}

/// A kind past the catalogue (18 to 63, the last `KIND_BITS` holds) has no row: a code that names one is
/// `answer_too_new` ("update enth").
#[test]
fn a_kind_past_the_catalogue_has_no_row() {
    let c = code(1, 5);
    for kind in ModuleKind::ALL.len() as u64..1 << KIND_BITS {
        let text = craft(&c, |w| {
            put_op_code(w, OP_ADD_MODULE);
            w.put(kind, KIND_BITS);
        });
        assert_eq!(
            AnswerCode::decode(&text).unwrap_err(),
            ResultError::AnswerTooNew {
                what: "module kind",
                value: kind
            },
            "kind {kind}"
        );
    }
}

/// A value of `spec` other than its default, reachable on its knob.
fn off_default(spec: &enth_params::ParamSpec, salt: u32) -> f32 {
    [0.37, 0.81, 0.13, 0.62, 1.0, 0.0]
        .into_iter()
        .map(|u: f32| value(spec, (u + salt as f32 * 0.071) % 1.0))
        .chain([spec.max(), spec.min()])
        .find(|v| v.to_bits() != spec.default.to_bits())
        .unwrap_or_else(|| panic!("{}: no value off its default", spec.key))
}

/// Lane CAT (F1m): every parameter of S2a's kinds 7–17 round-trips in an answer code. Per kind, two answers: one adds
/// a module with **every** parameter off its default, the other adds a default module and sets every parameter by
/// `SetParam` to other values (`ops_between` folds both into the `AddModule`'s (parameter, value) pairs, which use the
/// same indices as a `SetParam` run, since the start never holds a reserved kind); each also routes a macro to up to
/// eight modulatable parameters (`InsertRoute` writes the parameter index too). The decoded answer is the stripped
/// answer exactly, ids included. The kinds stay hidden from the Add menu and `enth modules`, but a patch may name them
/// by key.
#[test]
fn every_s2a_parameter_round_trips() {
    let challenge = code(3, 77);
    let target = target_of(&challenge);
    let mut checked = 0;
    for &kind in ModuleKind::ALL.iter().filter(|k| k.is_hidden()) {
        for edit in [false, true] {
            let mut answer = target.clone();
            let section = kind.section();
            // Room in the section: drop modules from its end until one more fits (never the amp envelope).
            while answer.section(section).len() >= section.max_len() {
                let id = answer.section(section).last().unwrap().id.clone();
                apply(&mut answer, &Op::RemoveModule { id }).unwrap();
            }
            let id = answer.free_id(kind);
            let mut module = ModuleInstance::new(id.clone(), kind);
            if !edit {
                for (i, spec) in kind.params().iter().enumerate() {
                    module.values[i] = off_default(spec, i as u32);
                }
            }
            let at = Position {
                section,
                index: answer.section(section).len(),
            };
            apply(&mut answer, &Op::AddModule { at, module }).unwrap();
            if edit {
                for (i, spec) in kind.params().iter().enumerate() {
                    let addr = ParamAddr::new(Target::Module(id.clone()), spec.key).unwrap();
                    let value = off_default(spec, 7 + i as u32);
                    apply(&mut answer, &Op::SetParam { addr, value }).unwrap();
                }
            }
            for (i, spec) in kind
                .params()
                .iter()
                .filter(|s| s.flags.modulatable)
                .enumerate()
                .take(8)
            {
                let route = Route {
                    src: ModSource::Macro(MacroIndex::new(i).unwrap()),
                    dst: ParamAddr::new(Target::Module(id.clone()), spec.key).unwrap(),
                    depth: 25.0 + i as f32,
                };
                let index = answer.routes.len();
                apply(&mut answer, &Op::InsertRoute { index, route }).unwrap();
            }
            let code = AnswerCode::new(challenge, 9, &answer)
                .unwrap_or_else(|e| panic!("{kind} (edit {edit}): {e}"));
            let back = AnswerCode::decode(&code.encode())
                .unwrap_or_else(|e| panic!("{kind} (edit {edit}): {e}"));
            assert_eq!(back, code, "{kind}");
            assert_eq!(exact_hash(back.answer()), exact_hash(&answer), "{kind}");
            assert!(
                enth_patch::canon::identical(back.answer(), &stripped_of(&answer)),
                "{kind} (edit {edit})"
            );
            let m = back.answer().find(&id).map(|(_, m)| m.clone()).unwrap();
            for (i, spec) in kind.params().iter().enumerate() {
                if !edit {
                    assert_ne!(
                        m.values[i].to_bits(),
                        spec.default.to_bits(),
                        "{kind}.{}",
                        spec.key
                    );
                }
            }
            checked += kind.params().len();
        }
    }
    // dist 9, wt 18, ott 12, comb 5, formant 5, mseg 59, chorus 9, phaser 9, eq 13, delay 8, ladder 6, twice.
    assert_eq!(checked, 2 * 153);
}

/// An MSEG is a route source like any modulator (a voice source to a voice destination), and its last point's
/// parameters (index 56 to 58, the highest any row uses) are written and read back.
#[test]
fn an_mseg_routes_and_its_last_point_round_trips() {
    let challenge = code(3, 78);
    let mut answer = target_of(&challenge);
    let id = answer.free_id(ModuleKind::Mseg);
    let at = Position {
        section: Section::Modulators,
        index: answer.modulators.len(),
    };
    apply(
        &mut answer,
        &Op::AddModule {
            at,
            module: ModuleInstance::new(id.clone(), ModuleKind::Mseg),
        },
    )
    .unwrap();
    let osc = answer.generators[0].id.clone();
    let route = Route {
        src: ModSource::Module(id.clone()),
        dst: ParamAddr::new(Target::Module(osc), "semi").unwrap(),
        depth: 12.5,
    };
    apply(&mut answer, &Op::InsertRoute { index: 0, route }).unwrap();
    for (key, value) in [
        ("points", 16.0),
        ("time16", 93.75),
        ("level16", 40.0),
        ("curve16", -100.0),
    ] {
        let addr = ParamAddr::new(Target::Module(id.clone()), key).unwrap();
        apply(&mut answer, &Op::SetParam { addr, value }).unwrap();
    }
    let code = AnswerCode::new(challenge, 3, &answer).unwrap();
    let back = AnswerCode::decode(&code.encode()).unwrap();
    assert_eq!(exact_hash(back.answer()), exact_hash(&answer));
    assert_eq!(
        kind_row(ModuleKind::Mseg).map(|r| RESULT_PARAMS_V1[r].1.len()),
        Some(59)
    );
}

// ---------------------------------------------------------------------------------------------------------------
// Rounds and answers
// ---------------------------------------------------------------------------------------------------------------

fn code(level: u8, seed: u64) -> ChallengeCode {
    ChallengeCode::short(SpecPreset::by_id(level).unwrap().spec(1, seed)).unwrap()
}

fn start() -> Patch {
    random::start(0).unwrap()
}

fn target_of(c: &ChallengeCode) -> Patch {
    random::target(c.spec()).unwrap().patch
}

/// What the code keeps of `answer`: the start's meta, engine stamp, macro names and slots, and no labels.
fn stripped_of(answer: &Patch) -> Patch {
    let s = start();
    let mut p = answer.clone();
    p.engine.clone_from(&s.engine);
    p.meta = s.meta.clone();
    p.slots = s.slots.clone();
    for (m, sm) in p.macros.iter_mut().zip(&s.macros) {
        m.name.clone_from(&sm.name);
    }
    for section in enth_patch::Section::ALL {
        for m in p.section_mut(section) {
            m.label = None;
        }
    }
    p
}

/// A value for `spec` from a uniform `u` in [0, 1]: any f32 the knob can reach.
fn value(spec: &enth_params::ParamSpec, u: f32) -> f32 {
    spec.check(spec.from_norm(u)).unwrap_or(spec.default)
}

/// One random edit, applied when it is valid (an invalid one is skipped): what a player, the CLI or an agent can do.
fn mutate(p: &mut Patch, kind: u8, a: u32, u: f32) {
    let n = p.modules().count();
    let pick = |p: &Patch, i: u32| p.modules().nth(i as usize % n).map(|(_, m)| m.clone());
    let op = match kind % 12 {
        0..=3 => {
            // A value of a module, voice or master.
            let which = a % (n as u32 + 2);
            let (target, specs) = match which {
                0 => (Target::Voice, Builtin::Voice.params()),
                1 => (Target::Master, Builtin::Master.params()),
                i => {
                    let m = pick(p, i - 2).unwrap();
                    (Target::Module(m.id.clone()), m.kind.params())
                }
            };
            let spec = &specs[(a / 97) as usize % specs.len()];
            let addr = ParamAddr::new(target, spec.key).unwrap();
            let v = if kind % 12 == 3 {
                crate::random::sig3(value(spec, u)).clamp(spec.min(), spec.max())
            } else {
                value(spec, u)
            };
            let v = spec.check(v).unwrap_or(spec.default);
            Op::SetParam { addr, value: v }
        }
        4 => {
            let m = pick(p, a).unwrap();
            Op::SetEnabled {
                id: m.id,
                enabled: !m.enabled,
            }
        }
        5 => {
            // A route from a source to a modulatable parameter.
            let src = match a % 5 {
                0 => ModSource::Velocity,
                1 => ModSource::Key,
                2 => ModSource::ModWheel,
                3 => ModSource::Macro(MacroIndex::new(a as usize / 5 % 8).unwrap()),
                _ => match pick(p, a / 5) {
                    Some(m) if m.kind == ModuleKind::Adsr || m.kind == ModuleKind::Lfo => {
                        ModSource::Module(m.id)
                    }
                    _ => ModSource::Velocity,
                },
            };
            let m = pick(p, a / 7).unwrap();
            let specs: Vec<_> = m
                .kind
                .params()
                .iter()
                .filter(|s| s.flags.modulatable)
                .collect();
            if specs.is_empty() {
                return;
            }
            let spec = specs[(a / 11) as usize % specs.len()];
            let depth = ((u * 200.0) - 100.0).clamp(-100.0, 100.0);
            Op::InsertRoute {
                index: (a as usize / 13) % (p.routes.len() + 1),
                route: Route {
                    src,
                    dst: ParamAddr::module(m.id, spec.key).unwrap(),
                    depth: if depth == 0.0 { 0.0 } else { depth },
                },
            }
        }
        6 => match p.routes.get(a as usize % (p.routes.len().max(1))) {
            Some(r) if a.is_multiple_of(2) => Op::RemoveRoute {
                route: RouteAddr {
                    src: r.src.clone(),
                    dst: r.dst.clone(),
                },
            },
            Some(r) => Op::SetDepth {
                route: RouteAddr {
                    src: r.src.clone(),
                    dst: r.dst.clone(),
                },
                depth: (u * 100.0).round() - 50.0,
            },
            None => return,
        },
        7 => Op::SetMacro {
            index: MacroIndex::new(a as usize % 8).unwrap(),
            value: u,
        },
        8 => {
            let listed: Vec<ModuleKind> = ModuleKind::listed().collect();
            let kind = listed[a as usize % listed.len()];
            let mut m = ModuleInstance::new(p.free_id(kind), kind);
            let i = (a / 7) as usize % m.values.len();
            m.values[i] = value(&kind.params()[i], u);
            Op::AddModule {
                at: Position {
                    section: kind.section(),
                    index: p.section(kind.section()).len(),
                },
                module: m,
            }
        }
        9 => Op::RemoveModule {
            id: pick(p, a).unwrap().id,
        },
        10 => {
            let m = pick(p, a).unwrap();
            let (at, _) = p.find(&m.id).unwrap();
            Op::Move {
                id: m.id,
                to: Position {
                    section: at.section,
                    index: (a / 3) as usize % p.section(at.section).len(),
                },
            }
        }
        _ => match pick(p, a) {
            Some(m) if m.kind == ModuleKind::Adsr => Op::SetAmpEnv { id: m.id },
            _ => return,
        },
    };
    let _ = apply(p, &op);
}

/// Personal and cosmetic fields: a name in the meta, labels, macro names, slots. The code must drop every one. (An
/// edit whose module an earlier edit removed is skipped.)
fn personalize(p: &mut Patch) {
    let ops = [
        Op::SetMeta {
            meta: PatchMeta {
                name: "Sam's bass".to_owned(),
                author: "Samantha Example".to_owned(),
                description: "made at 3 am on 2026-10-22".to_owned(),
                tags: vec!["mine".to_owned()],
            },
        },
        Op::SetLabel {
            id: ModuleId::new("filter1").unwrap(),
            label: Some("Sam's filter".to_owned()),
        },
        Op::SetMacroName {
            index: MacroIndex::new(0).unwrap(),
            name: Some("Sam".to_owned()),
        },
        Op::SetSlot {
            slot: SlotIndex::new(3).unwrap(),
            addr: Some("filter1.cutoff".parse().unwrap()),
        },
    ];
    for op in ops {
        let _ = apply(p, &op);
    }
}

fn any_edits() -> impl Strategy<Value = Vec<(u8, u32, f32)>> {
    proptest::collection::vec((any::<u8>(), any::<u32>(), 0.0f32..=1.0), 0..24)
}

proptest! {
    /// Round trip: a mutated target (values, enables, routes, macros, added, removed and moved modules, the amp
    /// envelope) encodes, decodes to the same code, and rebuilds the same sound: exactly (ids included) when no module
    /// was added, up to the added modules' ids otherwise.
    #[test]
    fn answers_round_trip(
        challenge in prop_oneof![3 => any_short_challenge(), 1 => any_long_challenge()],
        salt in any::<u64>(),
        edits in any_edits(),
        personal in any::<bool>(),
    ) {
        // A long challenge's prefix must be the regenerated target's for the code to be made.
        let generated = random::target(challenge.spec()).unwrap();
        let challenge = if challenge.form() == CodeForm::Long {
            ChallengeCode::long(*challenge.spec(), challenge.engine().unwrap(), TargetPrefix::of(&generated.fingerprint)).unwrap()
        } else {
            challenge
        };
        let mut answer = generated.patch.clone();
        for (kind, a, u) in edits {
            mutate(&mut answer, kind, a, u);
        }
        if personal {
            personalize(&mut answer);
        }
        let code = AnswerCode::new(challenge, salt, &answer).unwrap();
        let text = code.encode();
        let back = AnswerCode::decode(&text).unwrap();
        prop_assert_eq!(&back, &code);
        prop_assert_eq!(back.encode(), text.clone());
        prop_assert!(enth_patch::canon::identical(back.answer(), code.answer()));
        prop_assert_eq!(canonical_hash(back.answer()), canonical_hash(&answer));
        // Exactly, ids included, added modules too: an app-given id keeps its number (should-fix S1).
        prop_assert_eq!(exact_hash(back.answer()), exact_hash(&answer));
        prop_assert!(enth_patch::canon::identical(back.answer(), &stripped_of(&answer)));
        prop_assert_eq!(back.commitment(), code.commitment());
        prop_assert_eq!(back.salt(), salt);
        prop_assert!(!text.contains("--"));
        assert_old_builds_say_update(&text);
    }

    #[test]
    fn every_single_substitution_and_adjacent_swap_in_an_answer_is_caught(
        challenge in any_short_challenge(),
        edits in any_edits(),
    ) {
        let mut answer = target_of(&challenge);
        for (kind, a, u) in edits {
            mutate(&mut answer, kind, a, u);
        }
        let text = AnswerCode::new(challenge, 99, &answer).unwrap().encode();
        assert_damage_caught(&text, AnswerCode::decode);
    }

    #[test]
    fn a_truncated_answer_is_caught(challenge in any_short_challenge(), edits in any_edits()) {
        let mut answer = target_of(&challenge);
        for (kind, a, u) in edits {
            mutate(&mut answer, kind, a, u);
        }
        let text = AnswerCode::new(challenge, 7, &answer).unwrap().encode();
        assert_truncation_caught(&text, AnswerCode::decode);
    }

    #[test]
    fn garbage_never_panics(s in "F[0-9A-Z-]{0,200}") {
        let _ = AnswerCode::decode(&s);
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Privacy (GR5: no name, meta, label, macro name, slot or absolute time survives)
// ---------------------------------------------------------------------------------------------------------------

/// The proof: a patch with every personal field set gives **the same symbols** as the patch without them, so nothing
/// of them can be in the code; and the rebuilt answer holds none of them.
#[test]
fn no_name_meta_label_macro_name_slot_or_custom_id_survives() {
    let challenge = code(5, 1234);
    let target = target_of(&challenge);
    let mut personal = target.clone();
    personalize(&mut personal);
    // An added module with a name as its id and a label.
    let mut m = ModuleInstance::new(ModuleId::new("sam").unwrap(), ModuleKind::Lfo);
    m.label = Some("Samantha".to_owned());
    m.values[1] = 0.5;
    let at = Position {
        section: ModuleKind::Lfo.section(),
        index: personal.modulators.len(),
    };
    apply(
        &mut personal,
        &Op::AddModule {
            at,
            module: m.clone(),
        },
    )
    .unwrap();
    apply(
        &mut personal,
        &Op::SetMacroName {
            index: MacroIndex::new(7).unwrap(),
            name: Some("Sam".to_owned()),
        },
    )
    .unwrap();
    let mut plain = target.clone();
    m.id = plain.free_id(ModuleKind::Lfo);
    m.label = None;
    apply(&mut plain, &Op::AddModule { at, module: m }).unwrap();
    let a = AnswerCode::new(challenge, 42, &personal).unwrap();
    let b = AnswerCode::new(challenge, 42, &plain).unwrap();
    assert_eq!(a.encode(), b.encode(), "personal fields change no symbol");
    let back = AnswerCode::decode(&a.encode()).unwrap();
    let p = back.answer();
    assert_eq!(p.meta, start().meta, "the start's meta, never the player's");
    assert_eq!(p.meta.author, "");
    assert!(p.modules().all(|(_, m)| m.label.is_none()));
    assert!(p.macros.iter().all(|m| m.name.is_none()));
    assert!(p.slots.iter().all(Option::is_none));
    assert!(p.find(&ModuleId::new("sam").unwrap()).is_none());
    assert!(p.find(&ModuleId::new("lfo3").unwrap()).is_some());
    // No absolute time anywhere: the answer code has no time field at all (the scorecard has a duration).
    let bytes = answer_bytes(p, back.phrase());
    for needle in ["Sam", "2026", "mine"] {
        assert!(
            !bytes.windows(needle.len()).any(|w| w == needle.as_bytes()),
            "{needle}"
        );
    }
}

/// Should-fix S1: an added module keeps an id the app gave (its number in 6 bits), so the canonical answer renders
/// what the player heard (the engine seeds a module from its id); the lowest free id and an id the player typed both
/// write 0 and decode to the lowest free id, so no typed name survives.
#[test]
fn an_added_module_keeps_an_app_given_id() {
    let challenge = code(1, 42);
    let target = target_of(&challenge);
    let add = |p: &mut Patch, id: &str| {
        let mut m = ModuleInstance::new(ModuleId::new(id).unwrap(), ModuleKind::Osc);
        m.values[0] = 2.0;
        let at = Position {
            section: ModuleKind::Osc.section(),
            index: p.generators.len(),
        };
        apply(p, &Op::AddModule { at, module: m }).unwrap();
    };
    let free = target.free_id(ModuleKind::Osc);
    let n: u32 = free.as_str().trim_start_matches("osc").parse().unwrap();
    let rebuilt = |answer: &Patch| {
        let code = AnswerCode::new(challenge, 5, answer).unwrap();
        AnswerCode::decode(&code.encode()).unwrap().answer().clone()
    };
    // A number above the lowest free one (the player added two and removed the first) is kept.
    let kept = format!("osc{}", n + 4);
    let mut answer = target.clone();
    add(&mut answer, &kept);
    let back = rebuilt(&answer);
    assert!(
        back.find(&ModuleId::new(&kept).unwrap()).is_some(),
        "{kept} kept"
    );
    assert_eq!(exact_hash(&back), exact_hash(&answer));
    // The lowest free id, a typed name and a number past 6 bits all decode to the lowest free id.
    for id in [
        free.as_str().to_owned(),
        "lead".to_owned(),
        "osc64".to_owned(),
    ] {
        let mut answer = target.clone();
        add(&mut answer, &id);
        let back = rebuilt(&answer);
        assert!(back.find(&free).is_some(), "{id} becomes {free}");
        assert_eq!(canonical_hash(&back), canonical_hash(&answer));
    }
    // A taken id number is a field no encoder writes.
    let mut w = Bits::header(KIND_ANSWER, LAYOUT_V1);
    w.put(1, SALT_BITS);
    put_challenge(&mut w, &challenge);
    put_op_code(&mut w, OP_ADD_MODULE);
    w.put(ModuleKind::Osc as u64, KIND_BITS);
    w.put(0, PLACE_BITS);
    w.put_bool(true);
    w.put(1, ID_BITS);
    w.put(RUN_END, PARAM_BITS);
    end(&mut w);
    let mut w = Bits::continuing(w.into_symbols());
    w.put(0, TAIL_BITS);
    assert_eq!(
        AnswerCode::decode(&text(w.into_symbols())).unwrap_err(),
        ResultError::BadField {
            field: "module id",
            value: 1
        }
    );
}

/// Should-fix S10: the encoder checks its rebuild against the stripped answer, module ids aside, so a value that
/// overflowed its bits would be refused (`unencodable`) rather than share another patch. The check itself, on the
/// differences it must see and the ones it must not.
#[test]
fn the_rebuild_check_sees_every_difference_but_ids_and_stripped_fields() {
    let challenge = code(5, 1234);
    let answer = target_of(&challenge);
    assert!(same_ids_aside(&answer, &answer));
    let mut personal = answer.clone();
    personalize(&mut personal);
    assert!(same_ids_aside(&answer, &personal), "stripped fields aside");
    // A typed id, renumbered (with every route and the amp envelope following it).
    let mut renamed = answer.clone();
    let mut m = ModuleInstance::new(ModuleId::new("wob").unwrap(), ModuleKind::Lfo);
    m.values[1] = 3.0;
    let at = Position {
        section: ModuleKind::Lfo.section(),
        index: renamed.modulators.len(),
    };
    apply(
        &mut renamed,
        &Op::AddModule {
            at,
            module: m.clone(),
        },
    )
    .unwrap();
    apply(
        &mut renamed,
        &Op::InsertRoute {
            index: 0,
            route: Route {
                src: ModSource::Module(ModuleId::new("wob").unwrap()),
                dst: "filter1.cutoff".parse().unwrap(),
                depth: 20.0,
            },
        },
    )
    .unwrap();
    let code = AnswerCode::new(challenge, 1, &renamed).unwrap();
    assert!(same_ids_aside(code.answer(), &renamed));
    assert!(code.answer().find(&ModuleId::new("wob").unwrap()).is_none());
    // What must be seen: a value, a route's depth, a module's enabled flag, a macro.
    let mut value = answer.clone();
    value.master[0] += 1.0;
    let mut depth = renamed.clone();
    depth.routes[0].depth = 21.0;
    let mut enabled = answer.clone();
    enabled.generators[0].enabled = !enabled.generators[0].enabled;
    let mut macro_value = answer.clone();
    macro_value.macros[1].value = if macro_value.macros[1].value == 0.25 {
        0.75
    } else {
        0.25
    };
    assert!(!same_ids_aside(&answer, &value));
    assert!(!same_ids_aside(code.answer(), &depth));
    assert!(!same_ids_aside(&answer, &enabled));
    assert!(!same_ids_aside(&answer, &macro_value));
}

// ---------------------------------------------------------------------------------------------------------------
// Refusals
// ---------------------------------------------------------------------------------------------------------------

/// An answer code over `challenge` with hand-written op bits, a dummy tail and a valid GF(32) symbol: the decoder
/// refuses an op before it reaches the tail.
fn craft(challenge: &ChallengeCode, ops: impl FnOnce(&mut Bits)) -> String {
    let mut w = Bits::header(KIND_ANSWER, LAYOUT_V1);
    w.put(1, SALT_BITS);
    put_challenge(&mut w, challenge);
    ops(&mut w);
    let mut w = Bits::continuing(w.into_symbols());
    w.put(0, TAIL_BITS);
    text(w.into_symbols())
}

fn end(w: &mut Bits) {
    put_op_code(w, OP_END);
}

/// An `AddModule` of `kind` (default values, the lowest free id) at `place`.
fn add_module(w: &mut Bits, kind: ModuleKind, place: u64) {
    put_op_code(w, OP_ADD_MODULE);
    w.put(kind as u64, KIND_BITS);
    w.put(place, PLACE_BITS);
    w.put_bool(true);
    w.put(0, ID_BITS);
    w.put(RUN_END, PARAM_BITS);
}

/// Valid `AddModule`s that fill the starter's effects section to its cap.
fn fill_fx(w: &mut Bits) {
    let len = start().fx.len();
    for place in len..Section::Fx.max_len() {
        add_module(w, ModuleKind::Comp, place as u64);
    }
}

#[test]
fn answer_refusals() {
    let c = code(1, 5);
    let cases: Vec<(String, &str)> = vec![
        // Reserved op codes.
        (craft(&c, |w| put_op_code(w, 10)), "answer_too_new"),
        (craft(&c, |w| put_op_code(w, 15)), "answer_too_new"),
        // A module kind this build lacks: the first past the catalogue (18, after `ladder`) and the last the field
        // holds (63).
        (
            craft(&c, |w| {
                put_op_code(w, OP_ADD_MODULE);
                w.put(ModuleKind::ALL.len() as u64, KIND_BITS);
            }),
            "answer_too_new",
        ),
        (
            craft(&c, |w| {
                put_op_code(w, OP_ADD_MODULE);
                w.put((1 << KIND_BITS) - 1, KIND_BITS);
            }),
            "answer_too_new",
        ),
        // A parameter index past master's row: the next one, 15 (which ended a run before the widening), 40 and the
        // last the field holds below the run's end (62).
        (
            craft(&c, |w| {
                w.put(0, 1);
                w.put(1, TARGET_BITS);
                w.put(3, PARAM_BITS);
            }),
            "answer_too_new",
        ),
        (
            craft(&c, |w| {
                w.put(0, 1);
                w.put(1, TARGET_BITS);
                w.put(15, PARAM_BITS);
            }),
            "answer_too_new",
        ),
        (
            craft(&c, |w| {
                w.put(0, 1);
                w.put(1, TARGET_BITS);
                w.put(40, PARAM_BITS);
            }),
            "answer_too_new",
        ),
        (
            craft(&c, |w| {
                w.put(0, 1);
                w.put(1, TARGET_BITS);
                w.put(RUN_END - 1, PARAM_BITS);
            }),
            "answer_too_new",
        ),
        // A module added to a full section (a fifth effect: a build with a higher `MAX_FX` writes it), and a place at
        // or past a section's cap (12): a newer enth, not damage.
        (
            craft(&c, |w| {
                fill_fx(w);
                add_module(w, ModuleKind::Comp, Section::Fx.max_len() as u64);
            }),
            "answer_too_new",
        ),
        (
            craft(&c, |w| add_module(w, ModuleKind::Reverb, 12)),
            "answer_too_new",
        ),
        // A source tag this build lacks.
        (
            craft(&c, |w| {
                put_op_code(w, OP_INSERT_ROUTE);
                w.put(0, ROUTE_BITS);
                w.put(6, SOURCE_BITS);
            }),
            "answer_too_new",
        ),
        // A target past the patch's modules (31 and 63, the last the field holds); an empty run; voice where a module
        // is needed.
        (
            craft(&c, |w| {
                w.put(0, 1);
                w.put(31, TARGET_BITS);
            }),
            "bad_field",
        ),
        (
            craft(&c, |w| {
                w.put(0, 1);
                w.put((1 << TARGET_BITS) - 1, TARGET_BITS);
            }),
            "bad_field",
        ),
        (
            craft(&c, |w| {
                w.put(0, 1);
                w.put(2, TARGET_BITS);
                w.put(RUN_END, PARAM_BITS);
                end(w);
            }),
            "bad_field",
        ),
        (
            craft(&c, |w| {
                put_op_code(w, OP_REMOVE_MODULE);
                w.put(0, TARGET_BITS);
                end(w);
            }),
            "bad_field",
        ),
        // An op that does not apply: removing the amplitude envelope (env1 is the 7th module of the starter).
        (
            craft(&c, |w| {
                put_op_code(w, OP_REMOVE_MODULE);
                w.put(2 + 6, TARGET_BITS);
                end(w);
            }),
            "answer_mismatch",
        ),
        // Valid ops, a wrong tail.
        (craft(&c, end), "answer_mismatch"),
        // Bits after END.
        (
            craft(&c, |w| {
                end(w);
                w.put(0, 5);
            }),
            "bad_length",
        ),
    ];
    for (text, want) in cases {
        let e = AnswerCode::decode(&text).unwrap_err();
        assert_eq!(e.code(), want, "{text}: {e}");
        if want == "answer_too_new" {
            assert!(e.hint().contains("update enth"), "{}", e.hint());
        }
    }
    // A generator this build lacks, and a long challenge whose target this build regenerates differently.
    let mut w = Bits::header(KIND_ANSWER, LAYOUT_V1);
    w.put(1, SALT_BITS);
    w.put(0, 1);
    w.put(1, 8);
    w.put(2, 12);
    w.put(5, 40);
    end(&mut w);
    let mut w = Bits::continuing(w.into_symbols());
    w.put(0, TAIL_BITS);
    assert_eq!(
        AnswerCode::decode(&text(w.into_symbols()))
            .unwrap_err()
            .code(),
        "code_too_new"
    );
    let long =
        ChallengeCode::long(*c.spec(), EngineVersion::CURRENT, TargetPrefix(0xDEAD_BEEF)).unwrap();
    assert_eq!(
        AnswerCode::decode(&craft(&long, end)).unwrap_err().code(),
        "target_mismatch"
    );
    assert_eq!(
        AnswerCode::new(long, 1, &start()).unwrap_err().code(),
        "target_mismatch"
    );
    // Another kind.
    assert_eq!(
        AnswerCode::decode(&c.encode()).unwrap_err(),
        ResultError::WrongKind {
            found: None,
            expected: KIND_ANSWER
        }
    );
}

/// A code from a build with longer sections (a higher `MAX_FX`, say) says "update enth", not "damaged": an add to a
/// section already at its cap, and a place at or past the cap, are `answer_too_new` (with what and how much), while the
/// cap's own adds apply and a place inside the cap but past the section's length is no build's (`answer_mismatch`).
#[test]
fn a_full_section_or_a_place_past_its_cap_reads_update_enth() {
    let c = code(1, 5);
    let s = start();
    let (cap, len) = (Section::Fx.max_len(), s.fx.len());
    assert!(len + 2 <= cap, "the starter leaves room in its effects");
    let refused = |ops: &dyn Fn(&mut Bits)| AnswerCode::decode(&craft(&c, |w| ops(w))).unwrap_err();
    // A fifth effect: the section is full, so the count it would reach is the cap + 1.
    assert_eq!(
        refused(&|w| {
            fill_fx(w);
            add_module(w, ModuleKind::Reverb, cap as u64);
        }),
        ResultError::AnswerTooNew {
            what: "module count",
            value: cap as u64 + 1
        }
    );
    // The same adds up to the cap are fine: the code only fails at its (dummy) tail.
    assert_eq!(
        refused(&|w| {
            fill_fx(w);
            end(w);
        }),
        ResultError::AnswerMismatch
    );
    // A place of 12, on an add to a section with room and on a move.
    let too_new_place = ResultError::AnswerTooNew {
        what: "module place",
        value: 12,
    };
    assert_eq!(
        refused(&|w| add_module(w, ModuleKind::Reverb, 12)),
        too_new_place
    );
    let fx0 = target_number(&s, &Target::Module(s.fx[0].id.clone())).unwrap();
    assert_eq!(
        refused(&|w| {
            put_op_code(w, OP_MOVE);
            w.put(fx0, TARGET_BITS);
            w.put(12, PLACE_BITS);
        }),
        too_new_place
    );
    // Inside the cap but past the section's length: no build writes it, so it is damage.
    assert_eq!(
        refused(&|w| add_module(w, ModuleKind::Reverb, len as u64 + 1)),
        ResultError::AnswerMismatch
    );
    // And the hint says update.
    let e = refused(&|w| add_module(w, ModuleKind::Reverb, 12));
    assert!(e.hint().contains("update enth"), "{}", e.hint());
}

#[test]
fn the_start_itself_is_an_empty_answer() {
    let c = code(3, 77);
    let a = AnswerCode::new(c, 0, &start()).unwrap();
    assert_eq!(a.ops(), 0);
    assert!(enth_patch::canon::identical(a.answer(), &start()));
    // header 14 + salt 64 + challenge 61 + END 5 = 144 bits: 29 symbols, then the tail and code v1's symbol.
    assert_eq!(a.symbols(), 29 + 8 + 1);
}

#[test]
fn the_commitment_depends_on_the_salt_the_answer_and_the_challenge() {
    let c = code(2, 3);
    let t = target_of(&c);
    let a = AnswerCode::new(c, 1, &t).unwrap().commitment();
    assert_eq!(AnswerCode::new(c, 1, &t).unwrap().commitment(), a);
    assert_ne!(AnswerCode::new(c, 2, &t).unwrap().commitment(), a);
    assert_ne!(AnswerCode::new(c, 1, &start()).unwrap().commitment(), a);
    let other = code(2, 4);
    assert_ne!(AnswerCode::new(other, 1, &t).unwrap().commitment(), a);
    assert!(a.bits() < 1 << 60);
}

/// The answer bytes are a frozen preimage: pinned on the starter.
#[test]
fn the_answer_bytes_are_pinned() {
    let hex: String = hex(&Sha256::digest(answer_bytes(&start(), PhraseId(1))));
    assert_eq!(
        hex,
        "cb0097c6b1571ec0d720ed16da6d5c5ea7060e08a5e663de9580ebd66700b8b9"
    );
}

// ---------------------------------------------------------------------------------------------------------------
// Sizes
// ---------------------------------------------------------------------------------------------------------------

/// Symbols of the answer codes of `seeds` at `level`, with `answer` making the answer from the target.
fn sizes(
    level: u8,
    seeds: core::ops::Range<u64>,
    answer: impl Fn(&Patch, u64) -> Patch,
) -> Vec<usize> {
    let mut out: Vec<usize> = seeds
        .map(|seed| {
            let c = code(level, seed);
            let t = target_of(&c);
            AnswerCode::new(c, seed, &answer(&t, seed))
                .unwrap()
                .symbols()
        })
        .collect();
    out.sort_unstable();
    out
}

/// Every value the target moved from the start, nudged to a nearby value that a knob drag gives (not three
/// significant digits): the worst case for the float coding.
fn dragged(t: &Patch, seed: u64) -> Patch {
    let s = start();
    let mut p = t.clone();
    let ops = ops_between(&s, t).unwrap();
    for op in ops {
        if let Op::SetParam { addr, value } = op {
            let spec = p.spec(&addr).unwrap();
            if let Kind::Float { .. } = spec.kind {
                let n = spec.to_norm(value);
                let u = (n + 0.000_123_4 * (1.0 + (seed % 7) as f32)).min(1.0);
                let v = spec.check(spec.from_norm(u)).unwrap_or(value);
                let _ = apply(&mut p, &Op::SetParam { addr, value: v });
            }
        }
    }
    p
}

/// The target plus three extra knobs a player touched and left off their start (dragged values).
fn touched(t: &Patch, seed: u64) -> Patch {
    let mut p = dragged(t, seed);
    for (i, addr) in ["filter1.drive", "env1.velocity", "osc1.pw"]
        .iter()
        .enumerate()
    {
        let addr: ParamAddr = addr.parse().unwrap();
        let spec = p.spec(&addr).unwrap();
        let v = spec
            .check(spec.from_norm(0.123_456 + 0.1 * i as f32))
            .unwrap();
        let _ = apply(&mut p, &Op::SetParam { addr, value: v });
    }
    p
}

/// The size budget per level, on 40 seeds: targets as stand-ins for passing answers, the same with every moved value
/// dragged off three significant digits (the float coding's worst case), and dragged with three extra knobs touched.
/// Measured at F1e (median and max symbols, code v1's symbol included):
///
/// | Level | Target | Dragged | Touched | Report 02's target stand-ins |
/// | --- | --- | --- | --- | --- |
/// | 1 | 46, 50 | 49, 56 | 74, 75 | 51, 62 |
/// | 2 | 56, 61 | 66, 75 | 89, 91 | 72, 81 |
/// | 3 | 92, 98 | 123, 132 | 144, 147 | 141, 154 |
/// | 4 | 110, 130 | 144, 171 | 165, 186 | 165, 186 |
/// | 5 | 152, 188 | 212, 263 | 230, 271 | 254, 309 |
///
/// The budgets below were those with about 10 % of margin; a change that grows a code past them is a format change for
/// the F1e judge, not a budget bump. Lane SKD measures real passing answers at F2d.
///
/// **Widened on 2026-10-26** (owner decision D3, lane ANS: target 5 to 6 bits, parameter 4 to 6, place 3 to 4, kind 4
/// to 6; layout 1 stays layout 1, since no friend build has carried it). That is a format change the owner approved, so
/// each budget below grew by exactly its cell's measured growth, median and max each, and keeps its F1e margin.
/// Measured after, same 40 seeds (median and max symbols, code v1's symbol included; the F1e figures above are the
/// before):
///
/// | Level | Target | Dragged | Touched |
/// | --- | --- | --- | --- |
/// | 1 | 47, 51 | 50, 58 | 77, 79 |
/// | 2 | 58, 65 | 69, 78 | 93, 96 |
/// | 3 | 100, 106 | 130, 140 | 152, 155 |
/// | 4 | 120, 143 | 154, 183 | 175, 199 |
/// | 5 | 168, 208 | 227, 283 | 246, 290 |
///
/// Each moved knob costs 2 bits more (its parameter index), each run of knobs on one target 3 more (the target and the
/// run's end), each added module a few: about 1 to 4 symbols at level 1, about 16 to 20 at level 5.
#[test]
fn the_size_budget_per_level() {
    // (level, [(median, max) for target, dragged, touched])
    const BUDGET: [(u8, [(usize, usize); 3]); 5] = [
        (1, [(51, 57), (57, 66), (83, 88)]),
        (2, [(64, 72), (75, 87), (100, 105)]),
        (3, [(108, 116), (142, 153), (163, 170)]),
        (4, [(130, 158), (168, 200), (190, 218)]),
        (5, [(184, 227), (248, 310), (269, 317)]),
    ];
    // Every cell is measured and printed before any is asserted, so a format change shows its whole growth at once.
    let mut over = Vec::new();
    for (level, budget) in BUDGET {
        let rows = [
            sizes(level, 0..40, |t, _| t.clone()),
            sizes(level, 0..40, dragged),
            sizes(level, 0..40, touched),
        ];
        for (name, (sizes, (median, max))) in ["target", "dragged", "touched"]
            .iter()
            .zip(rows.iter().zip(budget))
        {
            let (m, x) = (sizes[sizes.len() / 2], sizes[sizes.len() - 1]);
            println!(
                "level {level} {name}: median {m}, max {x}, min {}",
                sizes[0]
            );
            if m > median || x > max {
                over.push(format!(
                    "level {level} {name}: median {m} (budget {median}), max {x} (budget {max})"
                ));
            }
        }
    }
    assert!(over.is_empty(), "{}", over.join("\n"));
}

/// A level-1 answer fits code v1's 64 symbols, so an old build says "update enth"; a level-5 answer does not, so it
/// says "bad length" (challenge.md, "What older builds print").
#[test]
fn every_answer_goes_through_code_v1s_decoder() {
    let l1 = AnswerCode::new(code(1, 42), 1, &target_of(&code(1, 42)))
        .unwrap()
        .encode();
    assert!(symbols_of(&l1).len() <= 64);
    assert_eq!(
        ChallengeCode::decode(&l1).unwrap_err().code(),
        "code_too_new"
    );
    let l5 = AnswerCode::new(code(5, 42), 1, &target_of(&code(5, 42)))
        .unwrap()
        .encode();
    assert!(symbols_of(&l5).len() > 64);
    assert_eq!(ChallengeCode::decode(&l5).unwrap_err().code(), "bad_length");
}

/// The longest answer a recreate round meets: every value of the starter moved by a knob drag (exact, non-round
/// floats), every macro moved and every module switched. It stays far below [`MAX_SYMBOLS`](crate::result::MAX_SYMBOLS).
#[test]
fn an_answer_that_moves_everything_fits() {
    let c = code(5, 1);
    let mut p = start();
    let mut targets: Vec<Target> = vec![Target::Voice, Target::Master];
    targets.extend(p.modules().map(|(_, m)| Target::Module(m.id.clone())));
    for target in targets {
        let specs = match &target {
            Target::Voice => Builtin::Voice.params(),
            Target::Master => Builtin::Master.params(),
            Target::Module(id) => p.find(id).unwrap().1.kind.params(),
        };
        for spec in specs {
            let addr = ParamAddr::new(target.clone(), spec.key).unwrap();
            let have = p.value(&addr).unwrap();
            let u = (spec.to_norm(have) + 0.371_234_5).fract();
            let v = value(spec, u);
            let _ = apply(&mut p, &Op::SetParam { addr, value: v });
        }
    }
    for i in 0..8 {
        let index = MacroIndex::new(i).unwrap();
        apply(
            &mut p,
            &Op::SetMacro {
                index,
                value: 0.123_457 * (i as f32 + 1.0),
            },
        )
        .unwrap();
    }
    for id in ["osc2", "noise1", "comp1", "reverb1"] {
        let id = ModuleId::new(id).unwrap();
        apply(&mut p, &Op::SetEnabled { id, enabled: true }).unwrap();
    }
    let a = AnswerCode::new(c, 3, &p).unwrap();
    println!(
        "every value moved: {} ops, {} symbols",
        a.ops(),
        a.symbols()
    );
    assert!(a.ops() > 70, "{}", a.ops());
    assert!(a.symbols() < 600, "{} symbols", a.symbols());
    assert_eq!(AnswerCode::decode(&a.encode()).unwrap(), a);
}
