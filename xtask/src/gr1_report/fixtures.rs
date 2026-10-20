//! The fixture logs: three synthetic players' round logs, an agent run, the two floors and a survey, **made up** by a
//! seeded generator (no real data, no real person). `xtask/fixtures/gr1/` holds what [`generate`] writes; a test
//! checks the committed files are exactly that, and the tests below read them back through the report.
//!
//! `cargo xtask gr1-report xtask/fixtures/gr1/logs --baseline xtask/fixtures/gr1/agent.jsonl --baseline
//! xtask/fixtures/gr1/floors.jsonl --survey xtask/fixtures/gr1/survey.jsonl` prints `report.txt` (and `--json`
//! prints `report.json`).
//!
//! To change a fixture or the generator:
//! `cargo test -p xtask gr1_report::fixtures::regenerate -- --ignored`, read the diff, and commit it.
//!
//! The players are written to look like what the plugin writes (`RoundLogEntry`, field order and all), with these
//! things on purpose:
//!
//! | Player | Build | What is in the log |
//! | --- | --- | --- |
//! | `p1` | current | 20 rounds in four sittings of 5; a score dispute (a `dispute` line, then the `end` line carrying it); hints and one reveal; every round rated and voted |
//! | `p2` | newer | 20 rounds in two sittings, then a 21st left without a try (skipped); one round the host restored (two `end` lines, the later one counts); unknown fields and values from a newer build (`outcome: timed_out`, `agreement: unsure`) |
//! | `p3` | older | 17 rounds (three short of 20); no `rating`, `fingerprint`, `engine` or `edit_log` fields; a dispute |
//!
//! Every player plays the same 20 codes (4 per level) in the same order, level by level in sets of 5; the numbers
//! are made up (they are not a forecast of GR1).

use std::fmt::Write as _;
use std::path::PathBuf;

use serde::Serialize;

use super::code::encode_short;

/// Where the committed fixtures live.
pub fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/gr1")
}

/// Seeded `SplitMix64`, arithmetic only (no transcendental), so every platform writes the same bytes.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn uniform(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }

    /// Roughly normal, mean 0, spread about 0.58: four uniforms minus their mean.
    fn wobble(&mut self) -> f64 {
        (0..4).map(|_| self.uniform()).sum::<f64>() - 2.0
    }
}

// ---- A tiny ordered JSON writer: the plugin writes fields in struct order, serde_json's Map would sort them. ----

fn j(v: impl Serialize) -> String {
    serde_json::to_string(&v).expect("serializes")
}

fn obj(fields: Vec<(&str, String)>) -> String {
    let body: Vec<String> = fields
        .into_iter()
        .map(|(k, v)| format!("\"{k}\":{v}"))
        .collect();
    format!("{{{}}}", body.join(","))
}

fn arr(items: &[String]) -> String {
    format!("[{}]", items.join(","))
}

fn opt<T: Serialize>(v: Option<T>) -> String {
    v.map_or("null".to_owned(), j)
}

fn fingerprint(code: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut out = String::new();
    for round in 0..4u8 {
        for b in code.bytes().chain([round]) {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
        let _ = write!(out, "{h:016x}");
    }
    out
}

/// The starter patch with a few knobs moved, as patch-file text (the fixtures' `patch` field).
fn patch_text(rng: &mut Rng) -> String {
    let mut v: serde_json::Value =
        serde_json::from_str(include_str!("../preset/starter.patch.json"))
            .expect("the starter patch");
    v["meta"]["description"] = serde_json::Value::String(String::new());
    v["meta"]["name"] = serde_json::Value::String("Init".to_owned());
    let semi = (rng.below(25) as f64) - 12.0;
    let cutoff = 200.0 + (rng.below(7800) as f64);
    v["generators"][0]["params"]["semi"] = serde_json::json!(semi);
    v["generators"][0]["params"]["wave"] =
        serde_json::json!(["saw", "sine", "triangle", "pulse"][rng.below(4) as usize]);
    v["voice_fx"][0]["params"]["cutoff"] = serde_json::json!(cutoff);
    v["voice_fx"][0]["params"]["resonance"] = serde_json::json!(rng.below(60) as f64);
    j(v)
}

/// A report of `sound`, `structure` and `combined` in hundredths of a percent, as `enth.score-report/1`.
fn report(rng: &mut Rng, sound: u16, structure: u16, combined: u16, level: u8) -> String {
    let pct = |h: u16| f64::from(h) / 100.0;
    let kinds = [
        "pitch",
        "brightness",
        "harmonics",
        "attack",
        "envelope",
        "sustain",
    ];
    let dirs = ["same", "higher", "lower"];
    let closeness = ["close", "a_bit", "far"];
    let first = rng.below(4) as usize;
    let components: Vec<String> = (0..3)
        .map(|k| {
            let far = if sound >= 9000 {
                0
            } else {
                1 + rng.below(2) as usize
            };
            obj(vec![
                ("kind", j(kinds[first + k])),
                (
                    "direction",
                    j(if far == 0 {
                        dirs[0]
                    } else {
                        dirs[1 + rng.below(2) as usize]
                    }),
                ),
                ("closeness", j(closeness[far])),
            ])
        })
        .collect();
    let addrs = [
        "osc1.wave",
        "filter1.cutoff",
        "env1.release",
        "osc1.semi",
        "filter1.resonance",
    ];
    let params: Vec<String> = (0..3)
        .map(|k| {
            let score = f64::from(structure) / 10_000.0;
            let verdict = if score > 0.8 {
                "good"
            } else if score > 0.5 {
                "near"
            } else {
                "bad"
            };
            obj(vec![
                ("addr", j(addrs[(level as usize + k) % addrs.len()])),
                ("answer_addr", j(addrs[(level as usize + k) % addrs.len()])),
                ("score", j(score)),
                ("verdict", j(verdict)),
                ("direction", j("same")),
                ("randomized", j(true)),
            ])
        })
        .collect();
    obj(vec![
        ("schema", j("enth.score-report/1")),
        (
            "sound",
            obj(vec![
                ("scorer", j("sound/1")),
                ("percent", j(pct(sound))),
                ("distance", j(f64::from(10_000 - sound) / 10_000.0 * 2.0)),
                ("floor", j(0.0625)),
                ("scale", j(0.5)),
                ("jnd", j(f64::from(10_000 - sound) / 10_000.0 * 4.0)),
                ("components", arr(&components)),
            ]),
        ),
        (
            "structure",
            obj(vec![
                ("scorer", j("structure/1")),
                ("percent", j(pct(structure))),
                ("params", arr(&params)),
                ("modules", "[]".to_owned()),
            ]),
        ),
        (
            "combined",
            obj(vec![
                ("scorer", j("combined/1")),
                ("percent", j(pct(combined))),
                ("sound_weight", j(65)),
            ]),
        ),
        ("pass_mark", j(90)),
        ("passed", j(sound >= 9000)),
        ("engine", j("0.2.0")),
    ])
}

/// What a player is like.
struct Player {
    id: &'static str,
    seed: u64,
    skill: f64,
    /// Sitting lengths, in rounds; their sum is the rounds played.
    sittings: &'static [usize],
    /// The chance "about right" is the vote.
    about_right: f64,
    /// How far the blind rating strays from the score.
    rating_noise: f64,
    /// 0 current, 1 newer than this reader, 2 older.
    build: u8,
    /// The round (0-based) that gets a score dispute.
    dispute_at: Option<usize>,
}

const PLAYERS: [Player; 3] = [
    Player {
        id: "p1",
        seed: 11,
        skill: 0.95,
        sittings: &[5, 5, 5, 5],
        about_right: 0.86,
        rating_noise: 1.3,
        build: 0,
        dispute_at: Some(12),
    },
    Player {
        id: "p2",
        seed: 22,
        skill: 0.84,
        sittings: &[10, 11],
        about_right: 0.74,
        rating_noise: 1.7,
        build: 1,
        dispute_at: None,
    },
    Player {
        id: "p3",
        seed: 33,
        skill: 0.77,
        sittings: &[3, 2, 5, 5, 2],
        about_right: 0.58,
        rating_noise: 1.6,
        build: 2,
        dispute_at: Some(14),
    },
];

/// The 20 codes every player plays: four seeds for each level.
pub fn codes() -> Vec<String> {
    let mut rng = Rng(0xC0DE_5EED);
    let seeds: Vec<u64> = (0..20).map(|_| rng.next() & ((1 << 40) - 1)).collect();
    // Level by level in sets of 5: round r is level r % 5 + 1.
    (0..20)
        .map(|r| encode_short((r % 5 + 1) as u8, 1, seeds[r]))
        .collect()
}

/// The milliseconds a sitting starts at: one every other day from a Monday in October 2026.
const FIRST_SITTING_MS: u64 = 1_791_936_000_000;

struct Round {
    index: usize,
    code: String,
    level: u8,
    serial: u64,
    nonce: u64,
    ended_by: &'static str,
    outcome: &'static str,
    submits: Vec<(u64, u16, u16, u16)>,
    time_to_pass_ms: Option<u64>,
    active_ms: u64,
    hints: u8,
    revealed: bool,
    agreement: Option<&'static str>,
    rating: Option<u8>,
    dispute: Option<&'static str>,
    edits: Vec<(u64, String)>,
    patch: Option<String>,
}

fn play(p: &Player, codes: &[String]) -> Vec<Round> {
    let mut rng = Rng(p.seed.wrapping_mul(0x1234_5678_9ABC_DEF1) ^ 0xFEED);
    let mut rounds = Vec::new();
    let mut r = 0usize;
    for (s, len) in p.sittings.iter().enumerate() {
        let mut clock = FIRST_SITTING_MS + (s as u64) * 2 * 86_400_000 + rng.below(3_600_000);
        for k in 0..*len {
            let level = (r % 5) as u8 + 1;
            let l = f64::from(level);
            // p2's 21st round repeats the first code and is left without a try: no submit, no hint, no reveal.
            let skipped = p.id == "p2" && r == 20;
            let hinted = rng.uniform() < 0.08 + 0.07 * (l - 1.0);
            let hints = if hinted && !skipped {
                1 + rng.below(3) as u8
            } else {
                0
            };
            let p_pass =
                (p.skill - 0.12 * (l - 1.0) + if hinted { 0.1 } else { 0.0 }).clamp(0.05, 0.99);
            // p2's round 12 ends with an outcome only a newer build writes (`timed_out`): a try that did not pass.
            let timed_out = p.id == "p2" && r == 12;
            let passes = rng.uniform() < p_pass && !skipped && !timed_out;
            let reveals = !passes && !skipped && !timed_out && level >= 3 && rng.uniform() < 0.3;
            let (outcome, mut submits): (&'static str, Vec<(u64, u16, u16, u16)>) = (
                if passes {
                    "passed"
                } else if reveals {
                    "gave_up"
                } else {
                    "not_passed"
                },
                Vec::new(),
            );
            let n_submits = if skipped || (reveals && rng.uniform() < 0.3) {
                0
            } else if passes {
                if rng.uniform() < 0.45 {
                    1
                } else {
                    2 + rng.below(3) as usize
                }
            } else {
                1 + rng.below(3) as usize
            };
            let base_ms = 40_000.0 + 22_000.0 * (l - 1.0);
            let mut at = (base_ms * (0.4 + 0.5 * rng.uniform())) as u64;
            let mut sound = if passes && n_submits == 1 {
                9050 + rng.below(800) as u16
            } else {
                2000 + rng.below(6500) as u16
            };
            let mut time_to_pass_ms = None;
            for i in 0..n_submits {
                let last = i + 1 == n_submits;
                let s_now = if passes && last {
                    sound.max(9050 + rng.below(700) as u16)
                } else {
                    sound.min(8900)
                };
                let structure = ((f64::from(s_now) * 0.8) as u16)
                    .saturating_sub(rng.below(1500) as u16)
                    .min(10_000);
                let combined =
                    ((0.65 * f64::from(s_now) + 0.35 * f64::from(structure)).round()) as u16;
                submits.push((at, s_now, structure, combined));
                if passes && last {
                    time_to_pass_ms = Some(at);
                }
                at += 20_000 + rng.below(70_000);
                sound = (s_now + 600 + rng.below(1800) as u16).min(9800);
            }
            let active_ms = if skipped {
                6_000 + rng.below(9_000)
            } else {
                at.max(time_to_pass_ms.unwrap_or(0)) + rng.below(30_000)
            };
            // The blind rating of the first submit, and the vote on its score.
            let first = submits.first().map(|s| f64::from(s.1) / 10_000.0);
            let implied = first.map(|f| 1.0 + 4.0 * f * f.sqrt());
            let rating = implied.and_then(|e| {
                (rng.uniform() < 0.93)
                    .then(|| (e + rng.wobble() * p.rating_noise).round().clamp(1.0, 5.0) as u8)
            });
            let dispute = (p.dispute_at == Some(r)).then_some(match p.id {
                "p1" => "Sounded the same to me, and it scored far lower than I expected.",
                _ => "The cutoff and the wave matched by ear but structure says bad.",
            });
            let agreement = if submits.is_empty() {
                None
            } else if dispute.is_some() {
                Some("too_low")
            } else if rng.uniform() < 0.08 {
                None
            } else if rng.uniform() < p.about_right {
                Some("about_right")
            } else {
                let diff = f64::from(rating.unwrap_or(3)) - implied.unwrap_or(3.0);
                Some(if diff >= 0.0 { "too_low" } else { "too_high" })
            };
            let n_edits = 6 + 3 * u64::from(level) + rng.below(8);
            let mut edits = Vec::new();
            let mut t = 2_000 + rng.below(8_000);
            let addrs = [
                "filter1.cutoff",
                "osc1.semi",
                "env1.release",
                "filter1.resonance",
                "osc1.wave",
            ];
            for _ in 0..n_edits {
                let a = addrs[rng.below(addrs.len() as u64) as usize];
                let to = if a == "osc1.wave" {
                    j(["saw", "sine", "triangle"][rng.below(3) as usize])
                } else {
                    j((rng.below(10_000) as f64) / 10.0)
                };
                edits.push((t, obj(vec![("set", j(a)), ("to", to)])));
                t += 700 + rng.below(6_000);
            }
            let last_of_sitting = k + 1 == *len;
            rounds.push(Round {
                index: r,
                code: codes[r % codes.len()].clone(),
                level,
                serial: k as u64 + 1,
                nonce: clock,
                ended_by: if !last_of_sitting {
                    "new_round"
                } else if s % 2 == 0 {
                    "keep"
                } else {
                    "restore"
                },
                outcome,
                time_to_pass_ms,
                active_ms,
                hints,
                revealed: reveals,
                agreement,
                rating: if p.build == 2 { None } else { rating },
                dispute,
                edits,
                patch: (!submits.is_empty()).then(|| patch_text(&mut rng)),
                submits,
            });
            clock += active_ms + 15_000 + rng.below(60_000);
            r += 1;
        }
    }
    rounds
}

/// The result object (`enth.round-result/1`) of a round, as its `end` line carries it.
fn result_json(p: &Player, rd: &Round, rng: &mut Rng, in_progress: bool) -> String {
    let first = rd
        .submits
        .first()
        .map(|&(_, s, st, c)| report(rng, s, st, c, rd.level));
    let mut assists = vec![
        ("hints", j(rd.hints)),
        ("live_meter", j(false)),
        ("revealed", j(rd.revealed)),
    ];
    if p.build == 1 && rd.level >= 3 {
        assists.push(("coach", j(rd.hints > 0)));
    }
    // A `dispute` line is written mid-round: the round has not ended, so it has not given up and has not run out of
    // time, but a pass that already happened stands.
    let outcome = if in_progress && rd.outcome != "passed" {
        "not_passed"
    } else {
        rd.outcome
    };
    let outcome = if p.id == "p2" && rd.index == 12 && !in_progress {
        "timed_out"
    } else {
        outcome
    };
    let agreement = match (p.id, rd.agreement) {
        ("p2", Some("too_high")) => Some("unsure"),
        (_, a) => a,
    };
    let mut fields = vec![
        ("schema", j("enth.round-result/1")),
        ("code", j(&rd.code)),
        ("outcome", j(outcome)),
        ("first_submit", first.unwrap_or_else(|| "null".to_owned())),
        ("submits", j(rd.submits.len())),
        ("time_to_pass_ms", opt(rd.time_to_pass_ms)),
        ("active_ms", j(rd.active_ms)),
        ("assists", obj(assists)),
        ("agreement", opt(agreement)),
    ];
    if p.build != 2 {
        fields.push(("rating", opt(rd.rating)));
    }
    obj(fields)
}

fn line(p: &Player, rd: &Round, event: &str, rng: &mut Rng) -> String {
    let is_end = event == "end";
    let submits: Vec<String> = rd
        .submits
        .iter()
        .map(|&(at, s, st, c)| {
            obj(vec![
                ("at_ms", j(at)),
                ("passed", j(s >= 9000)),
                ("sound", j(s)),
                ("structure", j(st)),
                ("combined", j(c)),
            ])
        })
        .collect();
    let mut fields = vec![
        ("schema", j("enth.round-log/1")),
        ("event", j(event)),
        (
            "round",
            obj(vec![("serial", j(rd.serial)), ("nonce", j(rd.nonce))]),
        ),
        ("code", j(&rd.code)),
        (
            "ended_by",
            if is_end {
                j(rd.ended_by)
            } else {
                "null".to_owned()
            },
        ),
        ("broken", j(false)),
        ("result", result_json(p, rd, rng, !is_end)),
        ("submits", arr(&submits)),
        ("dispute", opt(rd.dispute)),
        ("edits", j(rd.edits.len())),
        ("patch", opt(rd.patch.as_deref())),
        (
            "report",
            match (p.build, rd.submits.as_slice()) {
                (0, [(_, s, st, c)]) => report(rng, *s, *st, *c, rd.level),
                _ => "null".to_owned(),
            },
        ),
    ];
    if p.build != 2 {
        fields.push((
            "fingerprint",
            j(fingerprint(&super::code::normalize(&rd.code))),
        ));
        fields.push(("engine", j("0.2.0")));
        let log: Vec<String> = if is_end {
            rd.edits
                .iter()
                .map(|(t, ops)| {
                    obj(vec![
                        ("t_ms", j(t)),
                        ("source", j("editor")),
                        ("ops", format!("[{ops}]")),
                    ])
                })
                .collect()
        } else {
            Vec::new()
        };
        fields.push(("edit_log", arr(&log)));
    }
    if p.build == 1 {
        fields.push(("input", j("mouse")));
    }
    obj(fields)
}

/// One player's log, as the plugin appends it.
fn log_for(p: &Player, codes: &[String]) -> String {
    let rounds = play(p, codes);
    let mut rng = Rng(p.seed ^ 0xA11CE);
    let mut lines: Vec<String> = Vec::new();
    let mut restored: Option<String> = None;
    for (i, rd) in rounds.iter().enumerate() {
        if rd.dispute.is_some() {
            lines.push(line(p, rd, "dispute", &mut rng));
        }
        if p.id == "p2" && i == 7 {
            // The host loaded another state in the middle of the round (REAPER's own Undo): the round logged an
            // `end` line with `ended_by: host`, then the project restored it and the player finished it.
            let mut early = Round {
                ended_by: "host",
                outcome: "not_passed",
                time_to_pass_ms: None,
                agreement: None,
                rating: None,
                ..clone_round(rd)
            };
            early.submits.truncate(1);
            early.submits.iter_mut().for_each(|s| {
                s.1 = s.1.min(6_000);
                s.2 = s.2.min(5_000);
                s.3 = s.3.min(5_500);
            });
            early.active_ms = early.submits.first().map_or(10_000, |s| s.0 + 5_000);
            lines.push(line(p, &early, "end", &mut rng));
            restored = Some(line(p, rd, "end", &mut rng));
            continue;
        }
        lines.push(line(p, rd, "end", &mut rng));
        if let Some(again) = restored.take() {
            lines.push(again);
        }
    }
    let mut text = lines.join("\n");
    text.push('\n');
    text
}

fn clone_round(rd: &Round) -> Round {
    Round {
        index: rd.index,
        code: rd.code.clone(),
        level: rd.level,
        serial: rd.serial,
        nonce: rd.nonce,
        ended_by: rd.ended_by,
        outcome: rd.outcome,
        submits: rd.submits.clone(),
        time_to_pass_ms: rd.time_to_pass_ms,
        active_ms: rd.active_ms,
        hints: rd.hints,
        revealed: rd.revealed,
        agreement: rd.agreement,
        rating: rd.rating,
        dispute: rd.dispute,
        edits: rd.edits.clone(),
        patch: rd.patch.clone(),
    }
}

/// The agent's three attempts on every code, and the two floors, as `enth.gr1-baseline/1`.
fn baselines(codes: &[String]) -> (String, String) {
    let mut rng = Rng(0xA6E27);
    let (mut agent, mut floors) = (Vec::new(), Vec::new());
    let rates = [0.96, 0.82, 0.6, 0.36, 0.2];
    let cma = [0.9, 0.7, 0.5, 0.25, 0.1];
    for (r, code) in codes.iter().enumerate() {
        let level = r % 5;
        for attempt in 1..=3u32 {
            let passed = rng.uniform() < rates[level];
            let sound = if passed {
                9050 + rng.below(850)
            } else {
                2500 + rng.below(6000)
            };
            let renders = 18 + rng.below(100) + 12 * level as u64;
            let tokens = 60_000 + 25_000 * level as u64 + rng.below(40_000);
            agent.push(obj(vec![
                ("schema", j("enth.gr1-baseline/1")),
                ("arm", j("agent")),
                ("code", j(code)),
                ("attempt", j(attempt)),
                ("passed", j(passed)),
                ("sound", j(sound as f64 / 100.0)),
                ("renders", j(renders)),
                ("tokens", j(tokens)),
                ("cost_usd", j(tokens as f64 * 1.5e-6)),
                ("model", j("agent-under-test")),
            ]));
        }
        floors.push(obj(vec![
            ("schema", j("enth.gr1-baseline/1")),
            ("arm", j("noop")),
            ("code", j(code)),
            ("attempt", j(1)),
            ("passed", j(false)),
            ("sound", j((200 + rng.below(1800)) as f64 / 100.0)),
            ("renders", j(1)),
        ]));
        let passed = rng.uniform() < cma[level];
        floors.push(obj(vec![
            ("schema", j("enth.gr1-baseline/1")),
            ("arm", j("cmaes")),
            ("code", j(code)),
            ("attempt", j(1)),
            ("passed", j(passed)),
            (
                "sound",
                j(if passed {
                    9050 + rng.below(700)
                } else {
                    3000 + rng.below(5800)
                } as f64
                    / 100.0),
            ),
            ("renders", j(1500)),
        ]));
    }
    (agent.join("\n") + "\n", floors.join("\n") + "\n")
}

fn survey() -> String {
    let rows = [
        obj(vec![
            ("schema", j("enth.gr1-survey/1")),
            ("player", j("p1")),
            ("want_more", j("yes")),
            ("os", j("linux")),
            ("daw", j("reaper")),
            ("note", j("More levels, and a way to keep the target.")),
        ]),
        obj(vec![
            ("schema", j("enth.gr1-survey/1")),
            ("player", j("p2")),
            ("want_more", j("yes")),
            ("os", j("windows")),
            ("daw", j("bitwig")),
        ]),
        obj(vec![
            ("schema", j("enth.gr1-survey/1")),
            ("player", j("p3")),
            ("want_more", j("maybe")),
            ("os", j("macos")),
            ("daw", j("reaper")),
            ("note", j("Level 5 felt unfair.")),
        ]),
    ];
    rows.join("\n") + "\n"
}

/// Every fixture file: name and text, in a fixed order. The report's expected outputs are made by [`expected`].
pub fn generate() -> Vec<(&'static str, String)> {
    let codes = codes();
    let (agent, floors) = baselines(&codes);
    vec![
        ("logs/p1.jsonl", log_for(&PLAYERS[0], &codes)),
        ("logs/p2.jsonl", log_for(&PLAYERS[1], &codes)),
        ("logs/p3.jsonl", log_for(&PLAYERS[2], &codes)),
        ("agent.jsonl", agent),
        ("floors.jsonl", floors),
        ("survey.jsonl", survey()),
    ]
}

// ---- Reading the fixtures back through the report ---------------------------------------------------------------

use super::baseline;
use super::logs::{self, Source};
use super::numbers::{self, Extras, Report, Thresholds};
use super::render;
use super::survey as survey_mod;

/// A report over texts: `(player id, file label, text)` logs, baseline texts and a survey.
pub fn report_of(
    logs_in: &[(&str, &str, &str)],
    baselines_in: &[(&str, &str)],
    survey_in: Option<&str>,
) -> Report {
    let sources: Vec<Source> = logs_in
        .iter()
        .map(|(player, label, text)| Source {
            player: (*player).to_owned(),
            label: (*label).to_owned(),
            text: (*text).to_owned(),
        })
        .collect();
    let mut extras = Extras::default();
    for (label, text) in baselines_in {
        let (attempts, problems) = baseline::read(label, text);
        extras.attempts.extend(attempts);
        extras.problems.extend(problems);
        extras.baseline_files += 1;
    }
    if let Some(text) = survey_in {
        let (answers, problems) = survey_mod::read("survey.jsonl", text);
        extras.survey = answers;
        extras.problems.extend(problems);
        extras.survey_given = true;
    }
    numbers::build(&logs::load(&sources), &extras, &Thresholds::default())
}

/// The report of the generated fixtures, as `cargo xtask gr1-report xtask/fixtures/gr1 --baseline ... --survey ...`
/// prints it.
pub fn fixture_report(files: &[(&'static str, String)]) -> Report {
    let text = |name: &str| {
        files
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, t)| t.as_str())
            .expect("fixture file")
    };
    report_of(
        &[
            ("p1", "p1.jsonl", text("logs/p1.jsonl")),
            ("p2", "p2.jsonl", text("logs/p2.jsonl")),
            ("p3", "p3.jsonl", text("logs/p3.jsonl")),
        ],
        &[
            ("agent.jsonl", text("agent.jsonl")),
            ("floors.jsonl", text("floors.jsonl")),
        ],
        Some(text("survey.jsonl")),
    )
}

/// The expected outputs, derived from the fixtures: the report as JSON and as text.
pub fn expected(files: &[(&'static str, String)]) -> Vec<(&'static str, String)> {
    let report = fixture_report(files);
    vec![
        (
            "report.json",
            serde_json::to_string_pretty(&report).expect("serializes") + "\n",
        ),
        ("report.txt", render::text(&report)),
    ]
}

#[test]
#[ignore = "writes xtask/fixtures/gr1: run it on purpose, read the diff"]
fn regenerate() {
    let dir = dir();
    std::fs::create_dir_all(dir.join("logs")).unwrap();
    let files = generate();
    let outputs = expected(&files);
    for (name, text) in files.iter().chain(outputs.iter()) {
        std::fs::write(dir.join(name), text).unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gr1_report::numbers::Reading;
    use crate::gr1_report::schema::{Agreement, Outcome, Severity};
    use crate::gr1_report::{privacy, read_extras, read_sources};

    fn report() -> Report {
        fixture_report(&generate())
    }

    #[test]
    fn the_committed_fixtures_are_what_the_generator_writes() {
        let files = generate();
        let outputs = expected(&files);
        for (name, text) in files.iter().chain(outputs.iter()) {
            let committed = std::fs::read_to_string(dir().join(name)).unwrap_or_else(|e| {
                panic!("{name}: {e}; run the `regenerate` test (see the module docs)")
            });
            assert!(
                &committed == text,
                "{name} differs from what the generator writes; if the change is on purpose, run the `regenerate` test and commit the diff"
            );
        }
    }

    #[test]
    fn the_fixtures_are_made_up_and_hold_no_personal_data() {
        for (name, text) in generate() {
            for line in text.lines() {
                assert!(privacy::scan(line).is_empty(), "{name}: {line:.60}");
            }
        }
        // 20 codes, 4 per level, every one a real code with its check symbol.
        let codes = codes();
        assert_eq!(codes.len(), 20);
        for level in 1..=5u8 {
            let at: Vec<_> = codes
                .iter()
                .filter(|c| {
                    crate::gr1_report::code::decode(c)
                        .is_some_and(|i| i.check_ok && i.level == Some(level))
                })
                .collect();
            assert_eq!(at.len(), 4, "level {level}");
        }
        // Every patch in the logs is a patch this build reads.
        for (name, text) in generate().iter().filter(|(n, _)| n.starts_with("logs/")) {
            for line in text.lines() {
                let v: serde_json::Value = serde_json::from_str(line).unwrap();
                if let Some(patch) = v["patch"].as_str() {
                    enth_patch::file::load(patch).unwrap_or_else(|e| panic!("{name}: patch: {e}"));
                }
            }
        }
    }

    #[test]
    fn the_fixture_logs_validate_against_schema_v1_with_notes_only_from_the_newer_build() {
        let r = report();
        assert_eq!(
            (r.input.errors, r.input.warnings),
            (0, 0),
            "{:?}",
            r.input.problems
        );
        assert_eq!(
            (r.input.skipped_unusable, r.input.skipped_other_schema),
            (0, 0)
        );
        assert_eq!(r.input.privacy_findings, 0);
        assert!(r.input.notes > 0);
        assert!(
            r.input
                .problems
                .iter()
                .all(|p| p.severity == "note" && p.first_at.starts_with("p2.jsonl")),
            "{:?}",
            r.input.problems
        );
    }

    #[test]
    fn the_dedupe_of_a_restored_round_keeps_the_last_end_line() {
        let files = generate();
        let text = |n: &str| files.iter().find(|(name, _)| *name == n).unwrap().1.clone();
        let p2 = text("logs/p2.jsonl");
        // 22 lines, 21 rounds: one round has two `end` lines.
        assert_eq!(p2.lines().count(), 22);
        let loaded = logs::load(&[Source {
            player: "p2".into(),
            label: "p2.jsonl".into(),
            text: p2.clone(),
        }]);
        assert_eq!(loaded.rounds.len(), 21);
        let restored: Vec<_> = loaded.rounds.iter().filter(|r| r.end_lines > 1).collect();
        assert_eq!(restored.len(), 1);
        let round = restored[0];
        assert_eq!(round.end_lines, 2);
        // The first line is `ended_by: host` with one early, failing submit; the line that counts is the later one.
        let ends: Vec<serde_json::Value> = p2
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .filter(|v: &serde_json::Value| {
                v["round"]["serial"] == round.key.serial && v["round"]["nonce"] == round.key.nonce
            })
            .collect();
        assert_eq!(ends.len(), 2);
        assert_eq!(ends[0]["ended_by"], "host");
        assert_ne!(ends[1]["ended_by"], "host");
        assert_eq!(
            round
                .ended_by
                .as_ref()
                .map(super::super::schema::EndedBy::as_str),
            ends[1]["ended_by"].as_str()
        );
        assert_eq!(
            round.result.as_ref().unwrap().submits,
            ends[1]["result"]["submits"].as_u64().unwrap()
        );
        assert_ne!(
            round.result.as_ref().unwrap().submits,
            ends[0]["result"]["submits"].as_u64().unwrap()
        );
        // And the report counts it once.
        let p2_row = report().players.into_iter().find(|p| p.id == "p2").unwrap();
        assert_eq!((p2_row.rounds, p2_row.restored), (21, 1));
        // Counting every `end` line would have given 22 rounds.
        assert_ne!(p2_row.rounds, 22);
    }

    #[test]
    fn tolerant_reading_of_a_newer_and_an_older_build() {
        let files = generate();
        let text = |n: &str| files.iter().find(|(name, _)| *name == n).unwrap().1.clone();
        // p2 is a newer build: an unknown field everywhere, an unknown outcome and an unknown vote.
        let p2 = logs::load(&[Source {
            player: "p2".into(),
            label: "p2.jsonl".into(),
            text: text("logs/p2.jsonl"),
        }]);
        assert!(p2.rounds.iter().any(|r| {
            r.result
                .as_ref()
                .is_some_and(|x| matches!(x.outcome, Outcome::Unknown(_)))
        }));
        let unknown_votes = p2
            .rounds
            .iter()
            .filter(|r| r.vote().is_some_and(|v| matches!(v, Agreement::Unknown(_))))
            .count();
        assert_eq!(unknown_votes, 4);
        assert!(p2.problems.iter().all(|p| p.severity == Severity::Note));
        // p3 is an older build: no rating, fingerprint, engine or edit log, and still a full citizen.
        let p3 = logs::load(&[Source {
            player: "p3".into(),
            label: "p3.jsonl".into(),
            text: text("logs/p3.jsonl"),
        }]);
        assert_eq!(p3.rounds.len(), 17);
        assert!(
            p3.rounds
                .iter()
                .all(|r| r.rating().is_none() && r.fingerprint.is_none() && r.engine.is_none())
        );
        assert!(
            p3.problems.iter().all(|p| p.severity == Severity::Note),
            "{:?}",
            p3.problems
        );
        // The report counts the unknown vote apart from the three it understands.
        let a = report().agreement;
        assert_eq!(
            (a.too_low, a.about_right, a.too_high, a.unknown),
            (5, 34, 6, 4)
        );
        assert_eq!(a.votes, 45, "an unknown vote is not a vote");
    }

    #[test]
    fn the_gr1_numbers_of_the_fixtures() {
        let r = report();
        let col = |f: fn(&numbers::PlayerRow) -> usize| -> Vec<usize> {
            r.players.iter().map(f).collect()
        };
        assert_eq!(
            r.players.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(),
            ["p1", "p2", "p3"]
        );
        // Rounds played per player.
        assert_eq!(col(|p| p.rounds), [20, 21, 17]);
        assert_eq!(col(|p| p.played), [20, 20, 17]);
        assert_eq!(col(|p| p.skipped), [0, 1, 0]);
        assert_eq!(col(|p| p.restored), [0, 1, 0]);
        assert_eq!(col(|p| p.passed), [13, 10, 13]);
        assert_eq!(col(|p| p.assisted), [8, 8, 3]);
        assert_eq!(col(|p| p.votes), [18, 13, 14]);
        assert_eq!(col(|p| p.about_right), [14, 12, 8]);
        assert_eq!(col(|p| p.rated), [17, 18, 0]);
        assert_eq!(col(|p| p.disputes), [1, 0, 1]);
        assert_eq!(col(|p| p.wanted_more), [16, 19, 12]);
        // Pass rate by level.
        let levels: Vec<(Option<u8>, usize, usize)> = r
            .levels
            .iter()
            .map(|l| (l.level, l.played, l.passed))
            .collect();
        assert_eq!(
            levels,
            [
                (Some(1), 12, 11),
                (Some(2), 12, 12),
                (Some(3), 11, 6),
                (Some(4), 11, 3),
                (Some(5), 11, 4)
            ]
        );
        let unassisted: Vec<(usize, usize)> = r
            .levels
            .iter()
            .map(|l| (l.unassisted_passed, l.unassisted_played))
            .collect();
        assert_eq!(unassisted, [(10, 11), (7, 7), (4, 8), (1, 7), (3, 5)]);
        // Median time to pass (ms): the mean of the middle two of 36 passes.
        assert_eq!(
            (r.time_to_pass.n, r.time_to_pass.median_ms),
            (36, Some(86_120.5))
        );
        assert_eq!(
            (
                r.time_to_pass.unassisted_n,
                r.time_to_pass.unassisted_median_ms
            ),
            (25, Some(85_870.0))
        );
        // Assisted share.
        let a = &r.assists;
        assert_eq!((a.played, a.assisted, a.share), (57, 19, Some(0.3333)));
        assert_eq!(
            (
                a.with_hint,
                a.hints,
                a.revealed,
                a.live_meter,
                a.assisted_passes
            ),
            (17, 40, 6, 0, 11)
        );
        // Agreement distribution: too low / about right / too high.
        let g = &r.agreement;
        assert_eq!(
            (g.too_low, g.about_right, g.too_high, g.unknown, g.no_vote),
            (5, 34, 6, 4, 6)
        );
        assert_eq!((g.scored_rounds, g.votes), (55, 45));
        assert_eq!(g.about_right_share, Some(0.7556));
        assert_eq!(g.median_sound_too_low, Some(81.64));
        assert_eq!(g.median_sound_too_high, Some(54.53));
        // The blind rating against the score.
        let b = &r.rating;
        assert_eq!((b.n, b.mean_rating), (35, Some(3.17)));
        let rho = b.sound.rho.unwrap();
        assert!((rho - 0.8181).abs() < 1e-3, "{rho}");
        let (lo, hi) = (b.sound.ci_lo.unwrap(), b.sound.ci_hi.unwrap());
        assert!(
            lo < rho && rho < hi && lo > 0.4 && hi < 0.95,
            "{lo} {rho} {hi}"
        );
        let by: Vec<(u8, usize)> = b.by_rating.iter().map(|x| (x.rating, x.n)).collect();
        assert_eq!(by, [(1, 3), (2, 12), (3, 5), (4, 6), (5, 9)]);
        assert_eq!(
            b.players.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(),
            ["p1", "p2"],
            "p3 rated nothing"
        );
        // Want more.
        let w = &r.want_more;
        assert_eq!((w.rounds.k, w.rounds.n), (47, 58));
        assert_eq!(w.survey.len(), 3);
        assert!(w.unanswered.is_empty(), "{:?}", w.unanswered);
    }

    #[test]
    fn the_disputes_are_listed_with_their_round() {
        let r = report();
        assert_eq!(
            r.disputes.len(),
            2,
            "each dispute is on a `dispute` line and on its `end` line: listed once"
        );
        let d: Vec<(&str, Option<u8>, Option<&str>, bool)> = r
            .disputes
            .iter()
            .map(|d| (d.player.as_str(), d.level, d.outcome.as_deref(), d.has_end))
            .collect();
        assert_eq!(
            d,
            [
                ("p1", Some(3), Some("passed"), true),
                ("p3", Some(5), Some("gave_up"), true)
            ]
        );
        assert!(r.disputes[0].note.starts_with("Sounded the same"));
        assert_eq!(r.disputes[0].scores, Some([62.42, 47.88, 57.33]));
        assert_eq!(r.disputes[0].vote.as_deref(), Some("too_low"));
    }

    #[test]
    fn assisted_rounds_are_counted_by_hint_and_by_reveal() {
        let r = report();
        // p1: hints in some rounds, and the reveal; each assisted round counts once.
        let p1 = &r.players[0];
        assert_eq!(p1.assisted, 8);
        assert_eq!(p1.gave_up, 2, "a reveal before a pass is a gave-up round");
        assert_eq!(r.assists.hints, 40);
        // Gave-up rounds never count as passed.
        assert!(r.players.iter().all(|p| p.passed + p.gave_up <= p.played));
    }

    #[test]
    fn the_gate_indicators_of_the_fixtures() {
        let r = report();
        let status: Vec<(&str, Reading)> = r.gate.iter().map(|g| (g.id, g.status)).collect();
        assert_eq!(
            status,
            [
                ("players", Reading::Met),
                ("rounds", Reading::NotMet),
                ("agree", Reading::Met),
                ("want_more", Reading::Met),
                ("agent", Reading::Met),
            ]
        );
        assert_eq!(r.gate[1].value, "short: p3 17/20");
    }

    #[test]
    fn the_baseline_columns_sit_beside_the_humans() {
        let b = report().baseline.expect("baseline files were given");
        assert_eq!((b.codes, b.codes_played_by_humans), (20, 20));
        let labels: Vec<&str> = b.arms.iter().map(|g| g.label.as_str()).collect();
        assert_eq!(labels, ["agent", "no-op floor", "CMA-ES floor"]);
        let overall: Vec<(usize, usize)> = b
            .arms
            .iter()
            .map(|g| (g.overall.passes, g.overall.n))
            .collect();
        assert_eq!(overall, [(36, 60), (0, 20), (10, 20)]);
        let agent = &b.arms[0];
        assert_eq!(agent.median_renders, Some(96.0));
        assert_eq!(agent.total_cost_usd, Some(11.739));
        let per_level: Vec<usize> = agent.by_level.iter().map(|c| c.cell.passes).collect();
        assert_eq!(per_level, [12, 9, 6, 5, 4]);
        // Every human round is on one of the baseline's codes, so the two human rows agree.
        let same = b.humans_same_codes.as_ref().unwrap();
        assert_eq!(same.overall, b.humans_all.overall);
        assert_eq!(
            (b.humans_all.overall.passes, b.humans_all.overall.n),
            (36, 57)
        );
    }

    #[test]
    fn the_same_logs_give_the_same_report() {
        assert_eq!(report(), report());
        let json = serde_json::to_value(report()).unwrap();
        assert_eq!(json["schema"], "enth.gr1-report/1");
    }

    #[test]
    fn the_command_line_reads_the_fixture_folder_like_the_tests_do() {
        let root = dir();
        let to_args = |list: Vec<String>| list;
        let args = to_args(vec![
            root.join("logs").to_string_lossy().into_owned(),
            "--baseline".to_owned(),
            root.join("agent.jsonl").to_string_lossy().into_owned(),
            "--baseline".to_owned(),
            root.join("floors.jsonl").to_string_lossy().into_owned(),
            "--survey".to_owned(),
            root.join("survey.jsonl").to_string_lossy().into_owned(),
        ]);
        let opts = crate::gr1_report::parse_args(&args).unwrap();
        let sources = read_sources(&opts).unwrap();
        let players: Vec<&str> = sources.iter().map(|s| s.player.as_str()).collect();
        assert_eq!(
            players,
            ["p1", "p2", "p3"],
            "one player per file, named by the file"
        );
        assert!(
            sources.iter().all(|s| !s.label.contains('/')),
            "labels never carry a path"
        );
        let extras = read_extras(&opts).unwrap();
        let loaded = logs::load(&sources);
        let report = numbers::build(&loaded, &extras, &opts.thresholds);
        assert_eq!(
            render::text(&report),
            std::fs::read_to_string(root.join("report.txt")).unwrap()
        );
        // `--codes`: the 20 codes the agent run must play, 4 per level, 17 of them played by all three players.
        let codes = loaded.played_codes();
        let levels: Vec<Option<u8>> = codes.iter().map(|c| c.level).collect();
        assert_eq!(codes.len(), 20);
        assert!(levels.windows(2).all(|w| w[0] <= w[1]), "sorted by level");
        for level in 1..=5u8 {
            assert_eq!(levels.iter().filter(|l| **l == Some(level)).count(), 4);
        }
        assert_eq!(codes.iter().filter(|c| c.players == 3).count(), 17);
    }
}
