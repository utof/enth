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
/// purpose and keeps every old line; a changed line is a new layout).
#[test]
fn result_params_v1_is_pinned() {
    let hex: String = hex(&Sha256::digest(table_text().as_bytes()));
    assert_eq!(
        hex,
        "9f73b92ffa6c7f393d241226b8a299c0c1cfc3e5b2c5c8720b415842a9b6a9f1",
        "{}",
        table_text()
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
        assert!(row.len() <= 15, "{owner}: index 15 ends a run");
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
    for kind in ModuleKind::ALL {
        let (owner, row) = RESULT_PARAMS_V1[*kind as usize];
        assert_eq!(owner, kind.key());
        check(owner, row, kind.params());
    }
    assert_eq!(
        ModuleKind::ALL.len(),
        KINDS_V1,
        "a new kind is appended to the table"
    );
    check("voice", VOICE_V1, Builtin::Voice.params());
    check("master", MASTER_V1, Builtin::Master.params());
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
            let kind = ModuleKind::ALL[a as usize % ModuleKind::ALL.len()];
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
        let mut added = false;
        for (kind, a, u) in edits {
            added |= kind % 12 == 8;
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
        if !added {
            prop_assert_eq!(exact_hash(back.answer()), exact_hash(&answer));
            prop_assert!(enth_patch::canon::identical(back.answer(), &stripped_of(&answer)));
        }
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

#[test]
fn answer_refusals() {
    let c = code(1, 5);
    let cases: Vec<(String, &str)> = vec![
        // Reserved op codes.
        (craft(&c, |w| put_op_code(w, 10)), "answer_too_new"),
        (craft(&c, |w| put_op_code(w, 15)), "answer_too_new"),
        // A module kind this build lacks.
        (
            craft(&c, |w| {
                put_op_code(w, OP_ADD_MODULE);
                w.put(7, KIND_BITS);
            }),
            "answer_too_new",
        ),
        // A parameter index past master's row.
        (
            craft(&c, |w| {
                w.put(0, 1);
                w.put(1, TARGET_BITS);
                w.put(3, PARAM_BITS);
            }),
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
        // A target past the patch's modules; an empty run; voice where a module is needed.
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
/// The budgets below are those with about 10 % of margin; a change that grows a code past them is a format change for
/// the F1e judge, not a budget bump. Lane SKD measures real passing answers at F2d.
#[test]
fn the_size_budget_per_level() {
    // (level, [(median, max) for target, dragged, touched])
    const BUDGET: [(u8, [(usize, usize); 3]); 5] = [
        (1, [(50, 56), (56, 64), (80, 84)]),
        (2, [(62, 68), (72, 84), (96, 100)]),
        (3, [(100, 108), (135, 145), (155, 162)]),
        (4, [(120, 145), (158, 188), (180, 205)]),
        (5, [(168, 207), (233, 290), (253, 298)]),
    ];
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
            assert!(
                m <= median && x <= max,
                "level {level} {name}: median {m} (budget {median}), max {x} (budget {max})"
            );
        }
    }
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
