//! GR5's row "the receiver's score decides" (docs/duels.md, GR5; the F1e review's should-fix S8): for every golden
//! challenge, every pinned answer goes through the duel path, **answer → `AnswerCode` → its text → decode → `verify`**,
//! and the receiver's score is the golden's. It lives in `enth-golden`, so `golden.yml` runs it wherever the golden
//! challenges run: Linux and Windows (debug and release), macOS, and wasm32-wasip1 under wasmtime.
//!
//! Two comparisons per answer:
//!
//! - **bit for bit** with the harness's own score of the same answer on the same prepared round: the code, the
//!   rebuild and `verify` change no bit (the canonical answer is the answer: golden answers carry no stripped field and
//!   no added module);
//! - **with the pins** in `challenges/expected/`, within the harness's [`TOLERANCE`], under the scorer set this build
//!   re-scores with (`sound/1 structure/1 combined/1 time/1`).
//!
//! One round is prepared per case and both answers' checks run on it, as a duel does (`verify::prepare`, then
//! `verify_prepared`).
//!
//! The wasm run reads the cases at the path baked in at build time, so it preopens that directory, as `gc_challenges`:
//!
//! ```text
//! CARGO_TARGET_WASM32_WASIP1_RUNNER="/path/to/wasmtime run --dir /path/to/enth/tests/golden" \
//!   cargo test -p enth-golden --target wasm32-wasip1 --test gr5_receiver
//! ```

use std::collections::BTreeMap;

use enth_challenge::answer::AnswerCode;
use enth_challenge::result::ScorerSet;
use enth_challenge::score::round::score_round;
use enth_challenge::verify;
use enth_golden::challenges::case::Case;
use enth_golden::challenges::compute::{answers, subject};
use enth_golden::challenges::text::Doc;
use enth_golden::challenges::{self, TOLERANCE};

/// Every case name, with its parsed case file and its pins.
fn pinned() -> Vec<(String, Case, Doc)> {
    let dir = challenges::root();
    let mut names: Vec<String> = std::fs::read_dir(dir.join("cases"))
        .expect("cases/ exists")
        .filter_map(|e| {
            let path = e.expect("a directory entry").path();
            (path.extension()? == "toml").then(|| path.file_stem()?.to_str().map(str::to_owned))?
        })
        .collect();
    names.sort();
    names
        .into_iter()
        .map(|name| {
            let case = Case::parse(
                &std::fs::read_to_string(dir.join(format!("cases/{name}.toml"))).expect("a case"),
            )
            .expect("a case parses");
            let doc = Doc::parse(
                &std::fs::read_to_string(dir.join(format!("expected/{name}.txt")))
                    .expect("every case has its pins"),
            )
            .expect("pins parse");
            (name, case, doc)
        })
        .collect()
}

/// The pinned `report` line of `answer` under the scorer set `key`: sound %, structure %, combined %, passed.
fn pinned_report(doc: &Doc, key: &str, answer: &str) -> BTreeMap<String, String> {
    let block = doc
        .blocks
        .iter()
        .find(|b| b.key() == key)
        .unwrap_or_else(|| panic!("no block {key}"));
    let line = block
        .answers
        .iter()
        .find(|a| a.name == answer)
        .and_then(|a| a.lines.iter().find(|l| l.starts_with("report ")))
        .unwrap_or_else(|| panic!("no report of {answer} under {key}"));
    line.split_whitespace()
        .filter_map(|w| w.split_once('='))
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect()
}

#[test]
fn the_receivers_score_of_every_golden_answer_is_the_goldens() {
    let set = ScorerSet::current();
    assert_eq!(set, ScorerSet::V1, "the pins are under set 1's scorers");
    let ids = set.ids().expect("this build's set");
    let key = format!("{} {} {} {}", ids[0], ids[1], ids[2], ids[3]);
    let mut checked = 0;
    for (name, case, doc) in pinned() {
        let subject = subject(&case).unwrap_or_else(|e| panic!("{name}: {e}"));
        let mut round = None;
        for (answer, patch) in answers(&case, &subject).unwrap_or_else(|e| panic!("{name}: {e}")) {
            // The sender: the answer code, as text through the chat (a salt from the case's name, any 64 bits).
            let salt = name
                .bytes()
                .fold(0xC0FF_EE00_u64, |h, b| h.rotate_left(7) ^ u64::from(b));
            let text = AnswerCode::new(subject.code, salt, &patch)
                .unwrap_or_else(|e| panic!("{name}/{answer}: {e}"))
                .encode();
            // The receiver: decode (regenerates the target, rebuilds the answer, checks the tail), then verify.
            let code = AnswerCode::decode(&text).unwrap_or_else(|e| panic!("{name}/{answer}: {e}"));
            let round = round.get_or_insert_with(|| {
                verify::prepare(&code, set).unwrap_or_else(|e| panic!("{name}: {e}"))
            });
            let v = verify::verify_prepared(round, &code, None)
                .unwrap_or_else(|e| panic!("{name}/{answer}: {e}"));
            assert_eq!(v.scorers, set);
            // Bit for bit with the harness's score of the answer itself on the same round.
            let direct =
                score_round(round, &patch).unwrap_or_else(|e| panic!("{name}/{answer}: {e}"));
            for (what, got, want) in [
                ("sound", v.report.sound.percent, direct.sound.percent),
                (
                    "structure",
                    v.report.structure.percent,
                    direct.structure.percent,
                ),
                (
                    "combined",
                    v.report.combined.percent,
                    direct.combined.percent,
                ),
            ] {
                assert_eq!(
                    got.to_bits(),
                    want.to_bits(),
                    "{name}/{answer}: {what} {got} through the code, {want} directly"
                );
            }
            assert_eq!(v.report.passed, direct.passed, "{name}/{answer}");
            // The pins.
            let pins = pinned_report(&doc, &key, &answer);
            for (what, got) in [
                ("sound", v.report.sound.percent),
                ("structure", v.report.structure.percent),
                ("combined", v.report.combined.percent),
            ] {
                let want: f64 = pins[what].parse().unwrap();
                assert!(
                    (got - want).abs() <= TOLERANCE,
                    "{name}/{answer}: {what} {got}, pinned {want}"
                );
            }
            assert_eq!(
                v.report.passed.to_string(),
                pins["passed"],
                "{name}/{answer}"
            );
            checked += 1;
        }
    }
    assert!(checked >= 75, "{checked} answers checked");
}
