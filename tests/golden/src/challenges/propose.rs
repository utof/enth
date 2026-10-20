//! The authoring helper: `enth-golden --challenges --propose <code or settings>`.
//!
//! It prints a challenge's code, its target's facts and every randomized parameter, then scores the target moved by a
//! few amounts, one parameter at a time, under the v1 scorers, as the recipes a case file would use. Pick a
//! near miss from it (one that is clearly a pass or clearly a fail: the margin rule refuses anything within 1e-3 of
//! the pass mark).
//!
//! Settings are `key=value` words: `level=1..5` or `difficulty=0..100`, `seed=N`, `musical=0..100`, `amounts=a,b,c,d,e,f`
//! (`-` for "follows the difficulty"), as in the expected files' `spec` line.

use std::fmt::Write as _;

use enth_challenge::model::ScoringSpec;
use enth_challenge::random::{self, Drawn};
use enth_challenge::spec::{LATEST_GENERATOR, SpecPreset};
use enth_challenge::{
    Category, Challenge, ChallengeCode, EngineVersion, RandomSpec, prepare_challenge, score_round,
};
use enth_patch::{ParamAddr, Patch};

use super::case::{Base, Recipe};
use super::compute::{Scoring, Subject, apply_recipe, num};

/// Read a code, or settings, into a spec.
///
/// # Errors
/// Neither a code nor settings.
pub fn parse_input(text: &str) -> Result<(RandomSpec, Option<ChallengeCode>), String> {
    if !text.contains('=') {
        let code = ChallengeCode::decode(text).map_err(|e| format!("{e}. {}", e.hint()))?;
        return Ok((*code.spec(), Some(code)));
    }
    let mut spec = SpecPreset::by_id(1)
        .ok_or("no preset 1")?
        .spec(LATEST_GENERATOR, 1);
    for word in text.split_whitespace() {
        let (key, value) = word
            .split_once('=')
            .ok_or_else(|| format!("`{word}`: expected key=value"))?;
        let int = |v: &str| {
            v.parse::<u64>()
                .map_err(|_| format!("`{word}`: not a number"))
        };
        match key {
            "level" => {
                let preset = SpecPreset::by_id(int(value)? as u8)
                    .ok_or_else(|| format!("`{word}`: levels are 1 to 5"))?;
                spec.difficulty = preset.difficulty;
            }
            "difficulty" => spec.difficulty = int(value)? as u8,
            "seed" => spec.seed = int(value)?,
            "musical" => spec.musical = int(value)? as u8,
            "generator" => spec.generator = int(value)? as u16,
            "template" => spec.template = int(value)? as u8,
            "amounts" => {
                let parts: Vec<&str> = value.split(',').collect();
                if parts.len() != Category::COUNT {
                    return Err(format!(
                        "`{word}`: {} amounts, - or 0..100",
                        Category::COUNT
                    ));
                }
                for (slot, part) in spec.amounts.iter_mut().zip(parts) {
                    *slot = if part == "-" {
                        None
                    } else {
                        Some(int(part)? as u8)
                    };
                }
            }
            _ => return Err(format!("`{word}`: unknown key")),
        }
    }
    Ok((spec, None))
}

/// The moves tried on a parameter, as recipes: `(label, recipe)`.
fn moves(patch: &Patch, addr: &ParamAddr) -> Vec<(String, Recipe)> {
    let Some(spec) = patch.spec(addr) else {
        return Vec::new();
    };
    let name = addr.to_string();
    let one = |kind: &str, x: f64| {
        let mut r = Recipe::default();
        match kind {
            "mul" => r.mul.insert(name.clone(), x),
            "add" => r.add.insert(name.clone(), x),
            _ => r.set.insert(name.clone(), x),
        };
        (format!("{kind} = {{ \"{name}\" = {x} }}"), r)
    };
    let mut out = Vec::new();
    if spec.is_discrete() {
        if let Some(v) = patch.value(addr) {
            let v = f64::from(v);
            for d in [-1.0, 1.0] {
                let x = v + d;
                if x >= f64::from(spec.min()) && x <= f64::from(spec.max()) {
                    out.push(one("set", x));
                }
            }
        }
        return out;
    }
    let (kind, amounts): (&str, &[f64]) = match format!("{:?}", spec.unit).as_str() {
        "Hz" | "Ms" | "Ratio" | "None" => ("mul", &[1.06, 1.12, 1.25, 1.6]),
        "Db" => ("add", &[1.5, 3.0, 6.0]),
        "Cents" => ("add", &[10.0, 25.0, 50.0]),
        "Semitones" => ("add", &[0.1, 0.25, 0.5, 1.0]),
        "Octaves" => ("add", &[0.02, 0.05, 0.1]),
        "Degrees" => ("add", &[20.0, 60.0]),
        _ => ("add", &[5.0, 10.0, 20.0]),
    };
    for &x in amounts {
        out.push(one(kind, x));
        out.push(one(kind, if kind == "mul" { 1.0 / x } else { -x }));
    }
    out
}

/// The helper's output.
///
/// # Errors
/// An input that is no code or settings, a target that does not generate or a round that does not prepare.
pub fn propose(input: &str) -> Result<String, String> {
    let (spec, code) = parse_input(input)?;
    let start = random::start(spec.template).map_err(|e| format!("{e}. {}", e.hint()))?;
    let generated = random::generate(&spec, &start).map_err(|e| format!("{e}. {}", e.hint()))?;
    let code = match code {
        Some(code) => code,
        None => ChallengeCode::short(spec).map_or_else(
            || {
                ChallengeCode::long(spec, EngineVersion::CURRENT, generated.fingerprint.prefix())
                    .map_err(|e| format!("{e}"))
            },
            Ok,
        )?,
    };
    let subject = Subject {
        code,
        spec,
        generated,
        start,
    };
    let g = &subject.generated;
    let mut out = String::new();
    let _ = writeln!(out, "code {}", code.encode());
    for line in super::compute::header(&subject)?.iter().skip(1) {
        let _ = writeln!(out, "{line}");
    }
    let _ = writeln!(out, "randomized parameters (address: target, start):");
    for addr in &g.params {
        let _ = writeln!(
            out,
            "  {addr}: {} (start {})",
            g.patch
                .value(addr)
                .map_or_else(|| "-".to_owned(), |v| v.to_string()),
            subject
                .start
                .value(addr)
                .map_or_else(|| "-".to_owned(), |v| v.to_string())
        );
    }
    for r in &g.randomized {
        match &r.drawn {
            Drawn::Enable(id) => {
                let on = g.patch.find(id).is_some_and(|(_, m)| m.enabled);
                let _ = writeln!(out, "  {id}.enabled: {}", if on { "on" } else { "off" });
            }
            Drawn::Route(route) => {
                let depth = g
                    .patch
                    .routes
                    .iter()
                    .find(|x| x.src == route.src && x.dst == route.dst)
                    .map_or_else(|| "-".to_owned(), |x| x.depth.to_string());
                let _ = writeln!(out, "  route {route}: depth {depth}");
            }
            Drawn::Param(_) => {}
        }
    }
    let scoring = Scoring::named("v1").ok_or("no v1 set")?;
    let mut challenge = Challenge::recreate(spec, g.phrase);
    challenge.scoring = ScoringSpec::with_scorers(
        spec.difficulty,
        scoring.sound,
        scoring.structure,
        scoring.combined,
        scoring.time,
    );
    let round = prepare_challenge(challenge).map_err(|e| format!("{e}. {}", e.hint()))?;
    let start_report = score_round(&round, round.start()).map_err(|e| e.to_string())?;
    let _ = writeln!(
        out,
        "start: sound {} structure {} passed {}",
        num(start_report.sound.percent),
        num(start_report.structure.percent),
        start_report.passed
    );
    let _ = writeln!(
        out,
        "moves of the target, one parameter at a time (`!` marks a score within 1e-3 of the pass mark):"
    );
    for addr in &g.params {
        for (label, mut recipe) in moves(&g.patch, addr) {
            recipe.base = Base::Target;
            let Ok(patch) = apply_recipe(&recipe, &subject) else {
                continue;
            };
            let Ok(report) = score_round(&round, &patch) else {
                continue;
            };
            let flag = if report.margin().abs() < super::MARGIN {
                '!'
            } else {
                ' '
            };
            let _ = writeln!(
                out,
                "{flag} sound {:>7.3} structure {:>7.3} {}  {label}",
                report.sound.percent,
                report.structure.percent,
                if report.passed { "pass" } else { "FAIL" },
            );
        }
    }
    Ok(out)
}
