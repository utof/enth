//! Challenge v2's draw golden (lane F1L, F1l review should-fix 5; docs/challenge.md, "Challenge v2"): 1,000 (draw,
//! seed → drawn values) lines, the same on every platform, as generator v1's 1,000-spec golden
//! (`rg_generator.rs`) pins the generator. It lives in `enth-golden` so that `golden.yml` runs it on Linux and
//! Windows (debug and release) and under wasmtime on every push, and on macOS weekly, with no job of its own.
//!
//! Five draws over the frozen starter (template 0), each drawn for 200 seeds: a set of numbers and unit text, an enum
//! set by key, a continuous range, a range on a grid, a route's signed depth (numbers and interval text), and one
//! draw of several items at once. Each line holds the case, the index, the seed, the attempt that gave the target,
//! whether the step was forced, and every drawn value (`at=value`, the shortest `f32` text, which reads back to the
//! same bits); the last line is the SHA-256 of every line before it.
//!
//! **Why the values, not a patch fingerprint.** A draw round's fingerprint (`enth-target-patch-v1`) holds the whole
//! target patch, so a catalogue default change moves it with no draw changing; the values move only when a draw
//! does.
//!
//! **What a change means.** A saved lesson round re-prepares its draw after a reload and checks the target's
//! fingerprint, so a draw that changes breaks every saved lesson round on it (`target_drift`), and a lesson's
//! siblings change under its players. A change here is therefore a change of the draw's meaning: it needs a draw
//! version in the challenge (an additive field, Challenge v3), never an update of this file, unless a draw broke its
//! own documented rules (say so in the commit).
//!
//! Regenerate (only for the reasons above, and review the diff):
//! `cargo test -p enth-golden --test cv2_draw -- --ignored regenerate`.

#![allow(
    clippy::print_stdout,
    reason = "prints the summary hash for the native-vs-wasm comparison"
)]

use std::fmt::Write as _;

use enth_challenge::Phrase;
use enth_challenge::lesson_phrases::LONG_HOLD;
use enth_challenge::phrases::{self, HELD};
use enth_challenge::random;
use enth_challenge::spec::MAX_SEED;
use enth_challenge::v2::{Draw, DrawAt, DrawItem, DrawValue, DrawValues};
use sha2::{Digest, Sha256};

const EXPECTED: &str = include_str!("../draw/v1.txt");

/// The seed every golden draw's seed derives from (`CV2dr` in ASCII).
const ROOT: u64 = 0x0043_5632_6472;

/// Seeds per case.
const SEEDS: u64 = 200;

/// The `i`-th golden seed: one `SplitMix64` output (Vigna's public-domain constants) over `ROOT + i·γ`, kept to its
/// top 53 bits (a draw's seed is at most `MAX_SEED`).
const fn seed_of(i: u64) -> u64 {
    let mut z = ROOT.wrapping_add(i.wrapping_mul(0x9E37_79B9_7F4A_7C15));
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    (z ^ (z >> 31)) >> 11
}

const _: () = assert!(seed_of(0) <= MAX_SEED && seed_of(5 * SEEDS) <= MAX_SEED);

fn num(x: f32) -> DrawValue {
    DrawValue::Number(x)
}

fn text(t: &str) -> DrawValue {
    DrawValue::Text(t.to_owned())
}

fn set(at: &str, values: Vec<DrawValue>) -> DrawItem {
    DrawItem {
        at: if at.contains("->") {
            DrawAt::Route(at.parse().unwrap())
        } else {
            DrawAt::Param(at.parse().unwrap())
        },
        values: DrawValues::Set(values),
    }
}

fn range(at: &str, low: f32, high: f32, step: Option<f32>) -> DrawItem {
    DrawItem {
        at: DrawAt::Param(at.parse().unwrap()),
        values: DrawValues::Range {
            low: num(low),
            high: num(high),
            step: step.map(num),
        },
    }
}

/// The golden's cases: a name, the items, the phrase they are heard through.
fn cases() -> Vec<(&'static str, Vec<DrawItem>, &'static Phrase)> {
    let held = phrases::phrase(HELD).unwrap();
    let long = phrases::phrase(LONG_HOLD).unwrap();
    vec![
        (
            "cutoff-set",
            vec![set(
                "filter1.cutoff",
                vec![
                    num(300.0),
                    num(600.0),
                    num(1200.0),
                    text("4kHz"),
                    num(8000.0),
                ],
            )],
            held,
        ),
        (
            "mode-and-wave",
            vec![
                set("filter1.mode", vec![text("lp"), text("hp"), text("bp")]),
                set(
                    "osc1.wave",
                    vec![text("sine"), text("triangle"), text("pulse")],
                ),
            ],
            held,
        ),
        (
            "ranges",
            vec![
                range("filter1.resonance", 0.0, 80.0, None),
                range("osc1.semi", -24.0, 24.0, Some(1.0)),
            ],
            held,
        ),
        (
            "route-depth",
            vec![set(
                "env2 -> filter1.cutoff",
                vec![num(-40.0), text("+1oct"), num(60.0)],
            )],
            long,
        ),
        (
            "lesson-2.1",
            vec![
                set(
                    "filter1.cutoff",
                    vec![num(300.0), num(600.0), num(1200.0), text("4kHz")],
                ),
                set("filter1.mode", vec![text("lp"), text("bp")]),
                range("filter1.resonance", 10.0, 70.0, Some(10.0)),
                set(
                    "env1.attack",
                    vec![num(2.0), num(25.0), num(200.0), num(900.0)],
                ),
                set(
                    "env2 -> filter1.cutoff",
                    vec![num(-30.0), num(30.0), text("+1oct")],
                ),
            ],
            held,
        ),
    ]
}

/// The golden file's text, computed.
fn compute() -> String {
    let start = random::start(random::TEMPLATE_STARTER).unwrap();
    let mut out = String::from(
        "# enth-challenge Challenge v2 draws: 1,000 (draw, seed) -> drawn values golden (tests/golden/tests/cv2_draw.rs).\n\
         # case index seed a=attempt f=forced at=value...; last line: sha256 of every line above it\n",
    );
    let mut all = Sha256::new();
    let mut i = 0_u64;
    for (name, items, phrase) in cases() {
        for _ in 0..SEEDS {
            let draw = Draw {
                seed: seed_of(i),
                items: items.clone(),
            };
            let drawing = draw.draw(&start, phrase).unwrap();
            let mut line = format!(
                "{name} {i:04} {:014x} a={} f={}",
                draw.seed,
                drawing.attempt,
                u8::from(drawing.forced)
            );
            for (at, value) in &drawing.values {
                let _ = write!(line, " {at}={value}");
            }
            all.update(line.as_bytes());
            all.update(b"\n");
            out.push_str(&line);
            out.push('\n');
            i += 1;
        }
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
fn challenge_v2_draws_match_their_golden() {
    let got = compute();
    println!("cv2 draws: {}", got.lines().last().unwrap());
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
        "{} of {} golden lines differ (a draw changed: that needs a draw version, see the file's docs):\n{}",
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

/// The golden covers what it claims: the values of the sets are drawn, the routes' signs both ways, siblings
/// differ, a retry (a later attempt) is pinned too, and nothing needs the forced step on these draws.
#[test]
fn the_golden_draws_cover_their_values() {
    let got = compute();
    let lines: Vec<&str> = got.lines().filter(|l| !l.starts_with('#')).collect();
    assert_eq!(lines.len(), 5 * SEEDS as usize + 1);
    assert!(lines.iter().all(|l| !l.contains(" f=1 ")), "a forced step");
    assert!(
        lines
            .iter()
            .any(|l| !l.contains(" a=0 ") && l.contains(" a=")),
        "no retry pinned"
    );
    for value in [
        "filter1.cutoff=300 ",
        "filter1.cutoff=4000",
        "filter1.cutoff=8000",
        "filter1.mode=0",
        "filter1.mode=2",
        "env2 -> filter1.cutoff=-40",
        "env2 -> filter1.cutoff=60",
    ] {
        assert!(
            lines
                .iter()
                .any(|l| format!("{l} ").contains(&format!("{value} ")) || l.contains(value)),
            "{value} never drawn"
        );
    }
    let distinct: std::collections::BTreeSet<String> = lines
        .iter()
        .filter(|l| l.starts_with("lesson-2.1"))
        .map(|l| l.split(" a=").nth(1).unwrap_or_default().to_owned())
        .collect();
    assert!(distinct.len() > 100, "{} distinct siblings", distinct.len());
}

#[test]
#[ignore = "rewrites tests/golden/draw/v1.txt; run by hand, for the reasons in the file's docs"]
#[allow(
    clippy::disallowed_methods,
    reason = "the regenerate test writes the golden file; the library does no I/O"
)]
fn regenerate() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("draw");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("v1.txt"), compute()).unwrap();
}
