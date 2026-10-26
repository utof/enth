//! The structure score's golden, `structure/1` (lane ST; docs/challenge.md "Structure, combined and time",
//! docs/phase-2.md "Scoring across platforms"): 300 generated targets, each scored against three answers, the same
//! bits on every platform. It lives in `enth-golden` so that `golden.yml` runs it on Linux and Windows (debug and
//! release) and under wasmtime on every push, and on macOS weekly, with no job of its own: the structure score feeds
//! the combined score and the rings, and must not differ between a player's machine and CI.
//!
//! Each line holds the spec, then per answer the structure % as the f64's bits and the first 16 hex digits of a
//! SHA-256 over every row and module term (addresses, the player's addresses, score bits, verdicts, directions,
//! `randomized`), and the combined % at a fixed sound % of 73.25. The answers: the start patch; the target with two
//! observable parameters moved (chosen by the index); and the target with osc 1 and osc 2 trading ids, osc 2 and the
//! reverb switched over, and the sections reversed. The last line is the SHA-256 of all lines.
//!
//! **Frozen with `structure/1`.** Scorers are additive (design.md §6): a change that moves any of these numbers is a
//! new scorer id (`structure/2`) with its own file, never an update of this one. It also moves when a generator v1
//! target or the observability mask changes, which is itself a new generator version.
//!
//! The file is compiled in, so the wasm run needs no directory:
//!
//! ```text
//! CARGO_TARGET_WASM32_WASIP1_RUNNER=/path/to/wasmtime \
//!   cargo test -p enth-golden --target wasm32-wasip1 --test st_structure -- --nocapture
//! ```
//!
//! Regenerate (only while `structure/1` is unreleased, and review the diff):
//! `cargo test -p enth-golden --test st_structure -- --ignored regenerate`.

#![allow(
    clippy::print_stdout,
    reason = "prints the summary hash for the native-vs-wasm comparison"
)]

use std::fmt::Write as _;

use enth_challenge::random;
use enth_challenge::score::ScorerId;
use enth_challenge::score::StructureScore;
use enth_challenge::score::combined::CombinedV1;
use enth_challenge::spec::{MAX_SEED, PRESETS};
use enth_challenge::{
    Category, Challenge, PreparedRound, RandomSpec, prepare_challenge, score_structure,
};
use enth_patch::{ModSource, ModuleId, Op, ParamAddr, Patch, Section};
use sha2::{Digest, Sha256};

const EXPECTED: &str = include_str!("../structure/v1.txt");

/// The seed every golden spec's seed derives from (`ST_v1` in ASCII).
const ROOT: u64 = 0x0053_545F_7631;

/// The fixed sound % the combined column uses.
const SOUND: f64 = 73.25;

/// The `i`-th golden seed: one `SplitMix64` output over `ROOT + i·γ`, kept to its top 53 bits.
const fn seed_of(i: u64) -> u64 {
    let mut z = ROOT.wrapping_add(i.wrapping_mul(0x9E37_79B9_7F4A_7C15));
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    (z ^ (z >> 31)) >> 11
}

const _: () = assert!(seed_of(0) <= MAX_SEED && seed_of(299) <= MAX_SEED);

/// The `i`-th spec: every fourth a code v1 preset, the rest spread over difficulty (the upper levels twice as often:
/// they randomize routes and enable flags), `musical` and an amount.
fn spec(i: u64) -> RandomSpec {
    if i.is_multiple_of(4) {
        return PRESETS[((i / 4) % 5) as usize].spec(random::GENERATOR_V1, seed_of(i) >> (53 - 40));
    }
    let mut amounts = [None; Category::COUNT];
    if i % 7 == 3 {
        amounts[((i / 7) % 6) as usize] = Some(((i * 41) % 101) as u8);
    }
    let difficulty = if i.is_multiple_of(3) {
        ((i * 37) % 101) as u8
    } else {
        61 + ((i * 13) % 40) as u8
    };
    RandomSpec {
        generator: random::GENERATOR_V1,
        seed: seed_of(i),
        difficulty,
        amounts,
        musical: [100, 0, 50, 100, 21][(i % 5) as usize],
        template: random::TEMPLATE_STARTER,
    }
}

/// The round of `spec` with the sound stub (only structure is read here, and `sound/1`'s `prepare` renders).
fn prepare_round(spec: &RandomSpec) -> Result<PreparedRound, enth_challenge::RoundError> {
    let generated = random::target(spec).unwrap();
    let mut challenge = Challenge::recreate(*spec, generated.phrase);
    challenge.scoring.sound = ScorerId::SOUND_STUB;
    prepare_challenge(challenge)
}

fn set(p: &mut Patch, addr: &ParamAddr, value: f32) {
    enth_patch::apply(
        p,
        &Op::SetParam {
            addr: addr.clone(),
            value,
        },
    )
    .unwrap();
}

/// Parameters appended to a kind after `structure/1` froze, as (module kind, parameter key). The `moved` answer
/// picks its parameters by index into the observable list, so a longer list would move every pick and the golden
/// with it, though the scorer did not change: the picks stay among the parameters that existed at the freeze. S2a
/// F1m (lane TMP): `lfo.sync`, `lfo.beats` and `lfo.one_shot`.
const APPENDED_AFTER_FREEZE: &[(&str, &str)] =
    &[("lfo", "sync"), ("lfo", "beats"), ("lfo", "one_shot")];

fn existed_at_the_freeze(target: &Patch, addr: &ParamAddr) -> bool {
    addr.module_id()
        .and_then(|id| target.find(id))
        .is_none_or(|(_, m)| !APPENDED_AFTER_FREEZE.contains(&(m.kind.key(), addr.param())))
}

/// The target with two observable parameters moved to values picked from the index (integers only, so exact).
fn moved(i: u64, target: &Patch, observable: &[ParamAddr]) -> Patch {
    let mut out = target.clone();
    for k in 0..2u64 {
        let h = seed_of(10_000 + 2 * i + k);
        let addr = &observable[(h % observable.len() as u64) as usize];
        let spec = target.spec(addr).unwrap();
        let (lo, hi) = (f64::from(spec.min()), f64::from(spec.max()));
        let u = ((h >> 20) % 1000) as f64 / 999.0;
        let v = lo + u * (hi - lo);
        set(
            &mut out,
            addr,
            if spec.is_discrete() { v.round() } else { v } as f32,
        );
    }
    out
}

/// The target with osc 1 and osc 2 trading ids, osc 2 and the reverb switched over, and every section reversed.
fn restructured(target: &Patch) -> Patch {
    let (a, b) = (
        ModuleId::new("osc1").unwrap(),
        ModuleId::new("osc2").unwrap(),
    );
    let swap = |id: &ModuleId| {
        if *id == a {
            b.clone()
        } else if *id == b {
            a.clone()
        } else {
            id.clone()
        }
    };
    let mut out = target.clone();
    for id in ["osc2", "reverb1"] {
        let id = ModuleId::new(id).unwrap();
        let on = out.find(&id).unwrap().1.enabled;
        enth_patch::apply(&mut out, &Op::SetEnabled { id, enabled: !on }).unwrap();
    }
    for section in Section::ALL {
        let modules = out.section_mut(section);
        for m in modules.iter_mut() {
            m.id = swap(&m.id);
        }
        if section != Section::Fx {
            modules.reverse();
        }
    }
    for r in &mut out.routes {
        if let ModSource::Module(id) = &r.src {
            r.src = ModSource::Module(swap(id));
        }
        if let Some(id) = r.dst.module_id() {
            r.dst = ParamAddr::module(swap(id), r.dst.param()).unwrap();
        }
    }
    out.routes.reverse();
    out
}

fn bits(x: f64) -> String {
    format!("{:016x}", x.to_bits())
}

/// The structure %, a hash of every row and module term, and the combined % at [`SOUND`].
fn column(s: &StructureScore, sound_weight: u8) -> String {
    let mut h = Sha256::new();
    for p in &s.params {
        h.update(
            format!(
                "{}|{:?}|{}|{:?}|{:?}|{}\n",
                p.addr,
                p.answer_addr,
                bits(p.score),
                p.verdict,
                p.direction,
                p.randomized
            )
            .as_bytes(),
        );
    }
    for m in &s.modules {
        h.update(format!("{}|{}|{:?}\n", m.id, m.kind, m.status).as_bytes());
    }
    let digest: [u8; 32] = h.finalize().into();
    let rows = digest[..8].iter().fold(String::new(), |mut x, b| {
        let _ = write!(x, "{b:02x}");
        x
    });
    format!(
        "{}:{rows}:{}",
        bits(s.percent),
        bits(CombinedV1::percent(sound_weight, SOUND, s.percent))
    )
}

/// The golden file's text, computed.
fn compute() -> String {
    let mut out = String::from(
        "# enth-challenge structure/1: 300 targets x 3 answers -> structure % golden (tests/golden/tests/st_structure.rs).\n\
         # index seed d=difficulty m=musical a=amounts; per answer (start, moved, restructured): \
         percent bits:rows sha256[..16]:combined bits at sound 73.25; last line: sha256 of all lines\n",
    );
    let mut all = Sha256::new();
    for i in 0..300 {
        let s = spec(i);
        let round = prepare_round(&s).unwrap();
        let target = round.target();
        let observable: Vec<ParamAddr> = round
            .generated()
            .mask()
            .observable_params()
            .filter(|a| existed_at_the_freeze(target, a))
            .cloned()
            .collect();
        let w = round.challenge().scoring.sound_weight;
        let amounts: Vec<String> = s
            .amounts
            .iter()
            .map(|a| a.map_or("-".to_owned(), |a| a.to_string()))
            .collect();
        let mut line = format!(
            "{i:03} {:014x} d={:03} m={:03} a={}",
            s.seed,
            s.difficulty,
            s.musical,
            amounts.join(",")
        );
        for answer in [
            round.start().clone(),
            moved(i, target, &observable),
            restructured(target),
        ] {
            let score = score_structure(&round, &answer).unwrap();
            let _ = write!(line, " {}", column(&score, w));
        }
        all.update(line.as_bytes());
        out.push_str(&line);
        out.push('\n');
    }
    let digest: [u8; 32] = all.finalize().into();
    let hex = digest.iter().fold(String::new(), |mut h, b| {
        let _ = write!(h, "{b:02x}");
        h
    });
    let _ = writeln!(out, "all {hex}");
    out
}

#[test]
fn structure_v1_matches_its_golden() {
    let got = compute();
    println!("st structure v1: {}", got.lines().last().unwrap());
    if got == EXPECTED {
        return;
    }
    let diffs: Vec<String> = got
        .lines()
        .zip(EXPECTED.lines())
        .filter(|(a, b)| a != b)
        .map(|(a, b)| format!("  want {b}\n  got  {a}"))
        .collect();
    panic!(
        "{} of {} golden lines differ (structure/1 moved: that is a new scorer id, see the file's docs):\n{}",
        diffs.len(),
        EXPECTED.lines().count(),
        diffs
            .iter()
            .take(20)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}

/// The golden covers what it claims: every level, structure % from 0-ish to 100, rows of every kind.
#[test]
fn the_golden_covers_the_scorer() {
    let mut levels = [0usize; 6];
    let (mut routes, mut enables, mut modules, mut perfect) = (0, 0, 0, 0);
    for i in 0..300 {
        let s = spec(i);
        levels[usize::from(random::level(s.difficulty))] += 1;
        let round = prepare_round(&s).unwrap();
        let score = score_structure(&round, &restructured(round.target())).unwrap();
        routes += usize::from(score.params.iter().any(|p| p.addr.contains("->")));
        enables += usize::from(score.params.iter().any(|p| p.addr.ends_with(".enabled")));
        modules += usize::from(!score.modules.is_empty());
        perfect += usize::from(score_structure(&round, round.target()).unwrap().percent == 100.0);
    }
    assert!(levels[1..].iter().all(|n| *n >= 15), "{levels:?}");
    assert!(
        routes > 60 && enables > 60 && modules > 30,
        "{routes} {enables} {modules}"
    );
    assert_eq!(perfect, 300);
}

#[test]
#[ignore = "rewrites tests/golden/structure/v1.txt; run by hand, for the reasons in the file's docs"]
#[allow(
    clippy::disallowed_methods,
    reason = "the regenerate test writes the golden file; the library does no I/O"
)]
fn regenerate() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("structure/v1.txt");
    std::fs::write(path, compute()).unwrap();
}
