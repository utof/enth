//! `enth duel result|verify` (lane SKD, F2d): each output is deserialized back into its published type (the typed round
//! trip), and the snapshots pin the codes, which depend on the frozen generator, the frozen `result-params-v1` table and
//! the salt only: the answers here are the target itself (100 % on every engine), and the salt is given (`--salt`), so
//! nothing in them depends on the clock or the machine.
#![allow(
    clippy::disallowed_methods,
    reason = "tests read and write scratch files"
)]

mod common;

use common::{Scratch, record, round_trip, run, run_stdin};
use enth_challenge::answer::AnswerCode;
use enth_challenge::result::{SCORECARD_SYMBOLS, Scorecard};
use enth_challenge::spec::{LATEST_GENERATOR, SpecPreset};
use enth_challenge::{ChallengeCode, prepare_round};
use enth_cli::schema::{DuelResultOut, DuelVerifyOut, ErrorOut};
use insta::assert_snapshot;

const SALT: &str = "0123456789abcdef";

fn level2() -> ChallengeCode {
    ChallengeCode::short(SpecPreset::by_id(2).unwrap().spec(LATEST_GENERATOR, 77)).unwrap()
}

/// The challenge's target, written as a patch file with a name in its meta (which the answer code strips).
fn target_patch(s: &Scratch, code: &ChallengeCode) -> String {
    let round = prepare_round(code.spec()).unwrap();
    let mut target = round.target().clone();
    "Somebody Real".clone_into(&mut target.meta.author);
    enth_patch::file::ENGINE_VERSION.clone_into(&mut target.engine);
    s.write("target.patch.json", &(target.to_json_pretty() + "\n"))
}

fn result(s: &Scratch, code: &ChallengeCode) -> DuelResultOut {
    let patch = target_patch(s, code);
    let o = run(&[
        "duel",
        "result",
        &patch,
        "--code",
        &code.to_string(),
        "--submits",
        "3",
        "--seconds",
        "161",
        "--hints",
        "1",
        "--salt",
        SALT,
    ]);
    assert_eq!(o.code, 0, "{}", o.stdout);
    round_trip(&o)
}

#[test]
fn duel_result_makes_the_scorecard_and_the_answer_code() {
    let s = Scratch::new("duel_result");
    let code = level2();
    let patch = target_patch(&s, &code);
    let o = run(&[
        "duel",
        "result",
        &patch,
        "--code",
        &code.to_string(),
        "--submits",
        "3",
        "--seconds",
        "161",
        "--hints",
        "1",
        "--salt",
        SALT,
    ]);
    assert_eq!(o.code, 0, "{}", o.stdout);
    let out: DuelResultOut = round_trip(&o);
    assert_eq!(out.code, code.to_string());
    assert!(out.score.passed);
    assert_eq!(out.symbols.scorecard, SCORECARD_SYMBOLS);
    let card = Scorecard::decode(&out.scorecard).unwrap();
    let answer = AnswerCode::decode(&out.answer).unwrap();
    assert_eq!(answer.salt(), 0x0123_4567_89AB_CDEF);
    assert_eq!(
        card.fields().commitment,
        answer.commitment(),
        "the scorecard fixes this answer"
    );
    assert_eq!(answer.answer().meta.author, "", "no name survives the code");
    assert_eq!(out.symbols.answer, answer.symbols());
    assert_eq!(out.message[2], out.scorecard);
    assert_eq!(
        (
            out.reported.submits,
            out.reported.seconds,
            out.reported.hints
        ),
        (3, 161.0, 1)
    );
    s.settings()
        .bind(|| assert_snapshot!("duel_result", record(&s, &o, false)));
    // The same salt makes the same codes; a fresh one (OS randomness) another answer code, with its own commitment.
    assert_eq!(result(&s, &code).answer, out.answer);
    let fresh: DuelResultOut = round_trip(&run(&[
        "duel",
        "result",
        &patch,
        "--code",
        &code.to_string(),
    ]));
    assert_ne!(fresh.salt, out.salt);
    assert_ne!(fresh.answer, out.answer);
    assert_ne!(fresh.scorecard, out.scorecard);
}

#[test]
fn duel_verify_checks_a_whole_pasted_message_on_this_computer() {
    let s = Scratch::new("duel_verify");
    let code = level2();
    let out = result(&s, &code);
    // A whole chat message, the way a phone mangles it: a dash, non-breaking spaces, a code split by a line break.
    let answer = out.answer.replacen('-', "\n", 1);
    let message = format!(
        "gg \u{2014} mine:\n{}\nand my answer: {answer}",
        out.message.join("\n").replace(' ', "\u{00A0}")
    );
    let o = run_stdin(&["duel", "verify", "-"], &message);
    assert_eq!(o.code, 0, "{}", o.stdout);
    let v: DuelVerifyOut = round_trip(&o);
    assert_eq!(v.cards.len(), 1);
    assert_eq!(v.checks.len(), 1);
    let check = &v.checks[0];
    assert_eq!(check.challenge, code.to_string());
    assert!(check.theirs.passed);
    let claim = check
        .claim
        .as_ref()
        .expect("their scorecard was in the text");
    assert!(claim.as_reported, "{claim:?}");
    assert!(claim.commitment_matches);
    assert!(!claim.engine_differs && !claim.scorers_differ);
    s.settings()
        .bind(|| assert_snapshot!("duel_verify", record(&s, &o, false)));
    // --mine scores your patch beside theirs on the same round; --out writes their answer's patch.
    let start = prepare_round(code.spec()).unwrap().start().clone();
    let mine = s.write("mine.patch.json", &(start.to_json_pretty() + "\n"));
    let theirs = s.path("theirs.patch.json");
    let o = run(&[
        "duel",
        "verify",
        &out.answer,
        "--mine",
        &mine,
        "--out",
        &theirs,
    ]);
    assert_eq!(o.code, 0, "{}", o.stdout);
    let v: DuelVerifyOut = round_trip(&o);
    let mine = v.checks[0].mine.as_ref().unwrap();
    assert!(
        mine.sound < v.checks[0].theirs.sound,
        "the start is further away than the target"
    );
    assert!(v.checks[0].claim.is_none(), "no scorecard in this text");
    assert!(
        std::fs::read_to_string(&theirs)
            .unwrap()
            .contains("\"schema\"")
    );
}

#[test]
fn a_scorecard_alone_is_shown_as_reported_and_asks_for_the_answer() {
    let s = Scratch::new("duel_card_alone");
    let out = result(&s, &level2());
    let o = run(&["duel", "verify", &out.scorecard]);
    assert_eq!(o.code, 0, "{}", o.stdout);
    let v: DuelVerifyOut = round_trip(&o);
    assert_eq!((v.cards.len(), v.checks.len()), (1, 0));
    assert_eq!(v.cards[0].reported.seconds, 161.0);
    assert!(v.next.contains("swap answer codes"), "{}", v.next);
}

#[test]
fn duel_errors_say_what_to_do() {
    let s = Scratch::new("duel_errors");
    let code = level2();
    let patch = target_patch(&s, &code);
    let cases: [(&str, Vec<&str>, i32, &str); 5] = [
        (
            "err_duel_bad_salt",
            vec!["duel", "result", &patch, "--code", "X", "--salt", "12"],
            2,
            "bad_salt",
        ),
        (
            "err_duel_bad_submits",
            vec!["duel", "result", &patch, "--code", "X", "--submits", "0"],
            2,
            "bad_submits",
        ),
        (
            "err_duel_no_code",
            vec!["duel", "verify", "hello there"],
            3,
            "no_code",
        ),
        (
            "err_duel_typo",
            vec![
                "duel",
                "verify",
                "F120-8010-0000-01A0-0100-0F9Q-1C40-S500-0938-NKRK-AYDW-N1XB-RBS",
            ],
            3,
            "bad_check",
        ),
        (
            "err_duel_one_answer",
            vec![
                "duel",
                "verify",
                "F120-8010-0000-01A0-0100-0F9Q-1C40-S500-0938-NKRK-AYDW-N1XB-RBR",
                "--out",
                "x.patch.json",
            ],
            2,
            "one_answer",
        ),
    ];
    for (name, args, exit, error) in cases {
        let o = run(&args);
        let e: ErrorOut = round_trip(&o);
        assert_eq!(
            (o.code, e.error.as_str()),
            (exit, error),
            "{name}: {}",
            o.stdout
        );
        assert_ne!(e.hint, "", "{name}");
        s.settings()
            .bind(|| assert_snapshot!(name, record(&s, &o, false)));
    }
}

#[test]
fn duel_is_listed_in_help_and_its_schemas_in_enth_schema() {
    let help = run(&["--help"]).stdout;
    assert!(help.contains("  duel "), "{help}");
    for name in ["duel-result", "duel-verify"] {
        let o = run(&["schema", name]);
        assert_eq!(o.code, 0, "{name}: {}", o.stdout);
        assert!(o.stdout.contains(&format!("enth.{name}/1")), "{}", o.stdout);
    }
}

/// A small deterministic generator for the measurement (`SplitMix64`, Vigna's public-domain constants).
struct Mix(u64);

impl Mix {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A uniform in `[-1, 1)`.
    fn signed(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 23) as f32 - 1.0
    }
}

/// `addr` moved by `d` of its knob's travel (the normalised domain), as a drag leaves it: an exact `f32` with no
/// three-significant-digit form. `None` for a value that is not a float or that the op refuses.
fn dragged(
    patch: &enth_patch::Patch,
    addr: &enth_patch::ParamAddr,
    from: f32,
    d: f32,
) -> Option<enth_patch::Patch> {
    let spec = patch.spec(addr)?;
    if !matches!(spec.kind, enth_params::Kind::Float { .. }) {
        return None;
    }
    let value = spec.from_norm((spec.to_norm(from) + d).clamp(0.0, 1.0));
    let mut out = patch.clone();
    enth_patch::apply(
        &mut out,
        enth_patch::Op::SetParam {
            addr: addr.clone(),
            value,
        },
    )
    .ok()?;
    Some(out)
}

fn median_max(v: &mut [usize]) -> (usize, usize) {
    v.sort_unstable();
    (
        v.get(v.len() / 2).copied().unwrap_or(0),
        v.last().copied().unwrap_or(0),
    )
}

/// Real answer sizes on **passing** answers (lane SKD, F2d; docs/duels.md, "Sizes", which measured targets as
/// stand-ins). Per level, 20 seeds of the level's preset:
///
/// - **target**: the target itself (what rebuilding it by edits gives, as the REAPER `duel` scenario does);
/// - **played**: every randomized float dragged off the target by up to 2 % of its knob's travel (halved until the
///   patch passes `sound/1`), as a player who matched it by ear and mouse leaves it;
/// - **played + touched**: that, plus three other knobs nudged by up to 0.5 % of their travel and left there (each kept
///   only if the patch still passes).
///
/// Run: `cargo test -p enth-cli --release --test duel -- --ignored --nocapture measure_passing_answer_sizes`.
#[test]
#[ignore = "a measurement for docs/duels.md: run it in release with --nocapture"]
fn measure_passing_answer_sizes() {
    let mut mix = Mix(0x5EED_D0E1);
    println!(
        "level | n | target median, max | played median, max | played+touched median, max | passed"
    );
    for level in 1..=5u8 {
        let preset = SpecPreset::by_id(level).unwrap();
        let (mut target, mut played, mut touched) = (Vec::new(), Vec::new(), Vec::new());
        let mut n = 0;
        for seed in 1..=20u64 {
            let code = ChallengeCode::short(preset.spec(LATEST_GENERATOR, seed * 7919)).unwrap();
            let Ok(round) = prepare_round(code.spec()) else {
                continue;
            };
            n += 1;
            let size = |p: &enth_patch::Patch| {
                AnswerCode::new(code, mix_salt(seed), p).map(|a| a.symbols())
            };
            let passes = |p: &enth_patch::Patch| {
                enth_challenge::score_round(&round, p).is_ok_and(|r| r.passed)
            };
            target.push(size(round.target()).unwrap());
            // Played: the randomized floats dragged near the target, closer until it passes.
            let params = round.generated().params.clone();
            let mut spread = 0.02f32;
            let mut answer = None;
            while spread > 1e-4 {
                let mut p = round.target().clone();
                for addr in &params {
                    let from = round.target().value(addr).unwrap_or(0.0);
                    if let Some(moved) = dragged(&p, addr, from, mix.signed() * spread) {
                        p = moved;
                    }
                }
                if passes(&p) {
                    answer = Some(p);
                    break;
                }
                spread /= 2.0;
            }
            let Some(p) = answer else {
                continue;
            };
            played.push(size(&p).unwrap());
            // Touched: three other floats of enabled modules nudged and left there, while it still passes.
            let others: Vec<enth_patch::ParamAddr> = p
                .modules()
                .filter(|(_, m)| m.enabled)
                .flat_map(|(_, m)| {
                    m.spec()
                        .params
                        .iter()
                        .filter_map(|s| enth_patch::ParamAddr::module(m.id.clone(), s.key).ok())
                        .collect::<Vec<_>>()
                })
                .filter(|a| !params.contains(a))
                .collect();
            let mut q = p.clone();
            let mut moved = 0;
            for _ in 0..12 {
                if moved == 3 || others.is_empty() {
                    break;
                }
                let addr = &others[(mix.next() % others.len() as u64) as usize];
                let from = q.value(addr).unwrap_or(0.0);
                if let Some(r) = dragged(&q, addr, from, mix.signed() * 0.005)
                    && passes(&r)
                {
                    q = r;
                    moved += 1;
                }
            }
            touched.push(size(&q).unwrap());
        }
        let (t, p, q) = (
            median_max(&mut target),
            median_max(&mut played),
            median_max(&mut touched),
        );
        println!(
            "{level} | {n} | {}, {} | {}, {} | {}, {} | {} of {n}",
            t.0,
            t.1,
            p.0,
            p.1,
            q.0,
            q.1,
            played.len()
        );
    }
}

/// A salt for the measurement (the size does not depend on it).
fn mix_salt(seed: u64) -> u64 {
    seed.wrapping_mul(0x9E37_79B9_7F4A_7C15)
}
