//! From a case to the facts the expected file pins: the code's decoded settings, the target's fingerprint and
//! render, and for each scorer set the score reports of the case's answers.

use std::fmt::Write as _;

use enth_challenge::model::{SCORING_SAMPLE_RATE, SCORING_SEED, ScoringSpec};
use enth_challenge::random::{self, Drawn, Generated};
use enth_challenge::score::ScorerId;
use enth_challenge::{
    Challenge, ChallengeCode, EngineVersion, PreparedRound, RandomSpec, ScoreReport,
    prepare_challenge, score_round,
};
use enth_patch::addr::RouteAddr;
use enth_patch::{ModuleId, Op, ParamAddr, Patch};
use enth_render::{RenderSpec, render};
use serde::Serialize;
use sha2::{Digest, Sha256};

use super::case::{Base, Case, Recipe};
use super::text::{Answer, Block, Doc};

/// The four scorers a challenge names. A scorer set is one block of the expected file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scoring {
    /// The sound scorer.
    pub sound: ScorerId,
    /// The structure scorer.
    pub structure: ScorerId,
    /// The combiner.
    pub combined: ScorerId,
    /// The time rule.
    pub time: ScorerId,
}

impl Scoring {
    /// The named set a case lists in `scorings`: `v1` (`sound/1`, `structure/1`, `combined/1`, `time/1`: the
    /// released scorers, pinned explicitly so that a new default never changes a golden) or `stub` (the parameter-space
    /// stand-ins, the columns lane SKR's first five challenges pinned).
    pub fn named(name: &str) -> Option<Scoring> {
        let (sound, structure, combined) = match name {
            "v1" => (
                ScorerId::SOUND_V1,
                ScorerId::STRUCTURE_V1,
                ScorerId::COMBINED_V1,
            ),
            "stub" => (
                ScorerId::SOUND_STUB,
                ScorerId::STRUCTURE_STUB,
                ScorerId::COMBINED_STUB,
            ),
            _ => return None,
        };
        Some(Scoring {
            sound,
            structure,
            combined,
            time: ScorerId::TIME_V1,
        })
    }

    /// The set a block's key names (`sound/1 structure/1 combined/1 time/1`).
    pub fn from_key(key: &str) -> Option<Scoring> {
        let mut ids = key.split_whitespace().map(ScorerId::parse);
        let scoring = Scoring {
            sound: ids.next()??,
            structure: ids.next()??,
            combined: ids.next()??,
            time: ids.next()??,
        };
        ids.next().is_none().then_some(scoring)
    }

    /// The four ids, space separated: the block's key.
    pub fn key(&self) -> String {
        format!(
            "{} {} {} {}",
            self.sound, self.structure, self.combined, self.time
        )
    }
}

/// A decoded case: its code, the regenerated target and the template's start.
pub struct Subject {
    /// The code, as the case file wrote it.
    pub code: ChallengeCode,
    /// Its settings.
    pub spec: RandomSpec,
    /// The regenerated target.
    pub generated: Generated,
    /// The template's start patch.
    pub start: Patch,
}

/// Decode a case's code and regenerate its target.
///
/// # Errors
/// A code that does not decode, is not in canonical form, or does not regenerate the target it names (a long code's
/// fingerprint prefix), with the code's own hint.
pub fn subject(case: &Case) -> Result<Subject, String> {
    let code = ChallengeCode::decode(&case.code)
        .map_err(|e| format!("code {}: {e}. {}", case.code, e.hint()))?;
    if code.encode() != case.code {
        return Err(format!(
            "code {} is not in canonical form; write {}",
            case.code,
            code.encode()
        ));
    }
    let spec = *code.spec();
    let start = random::start(spec.template).map_err(|e| format!("{e}. {}", e.hint()))?;
    let generated = random::generate(&spec, &start).map_err(|e| format!("{e}. {}", e.hint()))?;
    code.verify(&generated.fingerprint, EngineVersion::CURRENT)
        .map_err(|e| format!("code {}: {e}. {}", case.code, e.hint()))?;
    Ok(Subject {
        code,
        spec,
        generated,
        start,
    })
}

/// The lowercase snake-case word serde writes for a plain enum (`a_bit`, `higher`).
fn tag<T: Serialize>(value: &T) -> String {
    match serde_json::to_value(value) {
        Ok(serde_json::Value::String(s)) => s,
        _ => "?".to_owned(),
    }
}

/// A pinned number: nine decimals, always with a point (so the comparison reads it as a number).
pub fn num(x: f64) -> String {
    format!("{x:.9}")
}

fn opt(x: Option<f64>) -> String {
    x.map_or_else(|| "-".to_owned(), num)
}

/// The header: what the code names and what the generator and the engine made of it.
///
/// # Errors
/// The target does not render.
pub fn header(subject: &Subject) -> Result<Vec<String>, String> {
    let (spec, g) = (&subject.spec, &subject.generated);
    let amounts: Vec<String> = spec
        .amounts
        .iter()
        .map(|a| a.map_or("-".to_owned(), |a| a.to_string()))
        .collect();
    let (mut params, mut routes, mut enables) = (0, 0, 0);
    for r in &g.randomized {
        match r.drawn {
            Drawn::Param(_) => params += 1,
            Drawn::Route(_) => routes += 1,
            Drawn::Enable(_) => enables += 1,
        }
    }
    let rendered = render(
        &g.patch,
        &g.phrase().performance(),
        &RenderSpec::new(SCORING_SAMPLE_RATE, SCORING_SEED),
    )
    .map_err(|e| format!("the target does not render: {e}"))?;
    let lufs = enth_challenge::loudness::integrated_lufs(&rendered.mid(), SCORING_SAMPLE_RATE);
    let form = match subject.code.engine() {
        None => "short".to_owned(),
        Some(engine) => format!("long engine={engine}"),
    };
    Ok(vec![
        format!("code {}", subject.code.encode()),
        format!("form {form}"),
        format!(
            "spec generator={} seed={} difficulty={} musical={} template={} amounts={}",
            spec.generator,
            spec.seed,
            spec.difficulty,
            spec.musical,
            spec.template,
            amounts.join(",")
        ),
        format!("level {}", g.level),
        format!("phrase {}", g.phrase.0),
        format!("fingerprint {}", g.fingerprint.to_hex()),
        format!("drawn params={params} routes={routes} enables={enables}"),
        format!(
            "target lufs={}",
            lufs.map_or_else(|| "silent".to_owned(), num)
        ),
        format!(
            "render rate={SCORING_SAMPLE_RATE} seed={SCORING_SEED} frames={} sha256={}",
            rendered.frames(),
            crate::hash_samples(SCORING_SAMPLE_RATE, 2, &rendered.interleaved())
        ),
    ])
}

/// The answers of a case, in pinned order: `exact`, `start`, then the case's own by name.
///
/// # Errors
/// A recipe that names an unknown module, parameter or route, or an edit the patch refuses.
pub fn answers(case: &Case, subject: &Subject) -> Result<Vec<(String, Patch)>, String> {
    let mut out = vec![
        ("exact".to_owned(), subject.generated.patch.clone()),
        ("start".to_owned(), subject.start.clone()),
    ];
    for (name, recipe) in &case.answers {
        let patch = apply_recipe(recipe, subject).map_err(|e| format!("answer `{name}`: {e}"))?;
        out.push((name.clone(), patch));
    }
    Ok(out)
}

/// The patch a recipe makes: its base, then the enable flags, `set`, `mul`, `add` and `depth` edits, each map in
/// address order.
///
/// # Errors
/// An unknown name, or an edit the patch refuses (a value out of range).
pub fn apply_recipe(recipe: &Recipe, subject: &Subject) -> Result<Patch, String> {
    let mut patch = match recipe.base {
        Base::Target => subject.generated.patch.clone(),
        Base::Start => subject.start.clone(),
    };
    for (id, enabled) in &recipe.enable {
        let id = ModuleId::new(id).map_err(|e| format!("module `{id}`: {e}"))?;
        apply(
            &mut patch,
            &Op::SetEnabled {
                id: id.clone(),
                enabled: *enabled,
            },
            &format!("{id}.enabled"),
        )?;
    }
    for (maps, how) in [
        (&recipe.set, Edit::Set),
        (&recipe.mul, Edit::Mul),
        (&recipe.add, Edit::Add),
    ] {
        for (addr, x) in maps {
            let param: ParamAddr = addr.parse().map_err(|e| format!("`{addr}`: {e}"))?;
            let current = f64::from(
                patch
                    .value(&param)
                    .ok_or_else(|| format!("`{addr}` is not a parameter of this patch"))?,
            );
            let value = match how {
                Edit::Set => *x,
                Edit::Mul => current * x,
                Edit::Add => current + x,
            };
            apply(
                &mut patch,
                &Op::SetParam {
                    addr: param,
                    value: value as f32,
                },
                addr,
            )?;
        }
    }
    for (route, depth) in &recipe.depth {
        let route: RouteAddr = route.parse().map_err(|e| format!("`{route}`: {e}"))?;
        let text = route.to_string();
        apply(
            &mut patch,
            &Op::SetDepth {
                route,
                depth: *depth as f32,
            },
            &text,
        )?;
    }
    Ok(patch)
}

#[derive(Clone, Copy)]
enum Edit {
    Set,
    Mul,
    Add,
}

fn apply(patch: &mut Patch, op: &Op, what: &str) -> Result<(), String> {
    enth_patch::apply(patch, op)
        .map(|_| ())
        .map_err(|e| format!("{what}: {e}"))
}

/// The result lines of one report: the scores, the sound readings, the components, the rings.
pub fn report_lines(r: &ScoreReport) -> Vec<String> {
    let mut out = vec![
        format!(
            "report sound={} structure={} combined={} passed={} margin={}",
            num(r.sound.percent),
            num(r.structure.percent),
            num(r.combined.percent),
            r.passed,
            num(r.margin())
        ),
        format!(
            "sound distance={} floor={} scale={} jnd={}",
            num(r.sound.distance),
            num(r.sound.floor),
            num(r.sound.scale),
            num(r.sound.jnd)
        ),
    ];
    for c in &r.sound.components {
        out.push(format!(
            "component {} {} {} target={} answer={}",
            tag(&c.kind),
            tag(&c.direction),
            tag(&c.closeness),
            opt(c.target),
            opt(c.answer)
        ));
    }
    for p in &r.structure.params {
        let row = if p.randomized { "ring" } else { "moved" };
        out.push(format!(
            "{row} {} {} {} {} answer={}",
            p.addr,
            tag(&p.verdict),
            tag(&p.direction),
            num(p.score),
            p.answer_addr.as_deref().unwrap_or("-")
        ));
    }
    for m in &r.structure.modules {
        out.push(format!("module {} {} {}", tag(&m.status), m.id, m.kind));
    }
    out
}

/// The round of a case under a scorer set, the scorers named explicitly (`ScoringSpec::with_scorers`, so this
/// build's defaults never change a golden), with the sound weight and the pass mark that follow.
///
/// # Errors
/// The round does not prepare.
pub fn round(subject: &Subject, scoring: &Scoring) -> Result<(PreparedRound, u8, u8), String> {
    let spec = subject.spec;
    let mut challenge = Challenge::recreate(spec, subject.generated.phrase);
    challenge.scoring = ScoringSpec::with_scorers(
        spec.difficulty,
        scoring.sound.clone(),
        scoring.structure.clone(),
        scoring.combined.clone(),
        scoring.time.clone(),
    );
    let (weight, mark) = (challenge.scoring.sound_weight, challenge.scoring.pass_mark);
    let round = prepare_challenge(challenge)
        .map_err(|e| format!("{}: {e}. {}", scoring.key(), e.hint()))?;
    Ok((round, weight, mark))
}

/// One scorer set's block: prepare the round with the set named explicitly, score every answer, and check each
/// pass decision against the margin.
///
/// # Errors
/// The round does not prepare, an answer does not score, or a pass decision is within the margin of the pass mark.
pub fn block(
    name: &str,
    subject: &Subject,
    scoring: &Scoring,
    answers: &[(String, Patch)],
    bits: &mut Sha256,
) -> Result<Block, String> {
    let (round, weight, mark) = round(subject, scoring)?;
    let mut heading = String::from("scoring ");
    let _ = write!(
        heading,
        "{} weight={weight} pass_mark={mark}",
        scoring.key()
    );
    let mut out = Block {
        heading,
        answers: Vec::new(),
    };
    for (answer, patch) in answers {
        let report = score_round(&round, patch)
            .map_err(|e| format!("{name} / {answer}: {e}. {}", e.hint()))?;
        super::margin::check(name, answer, &report)?;
        feed(bits, &report);
        out.answers.push(Answer {
            name: answer.clone(),
            lines: report_lines(&report),
        });
    }
    Ok(out)
}

/// What a case computes now: the document, and a digest of every number in its reports as raw bits (information, not
/// a pin: the pins are compared within [`super::TOLERANCE`]; this says whether two runs, say native and wasm, agree to
/// the last bit).
pub struct Computed {
    /// The pins.
    pub doc: Doc,
    /// SHA-256 over the bits of every score, reading and ring score, as hex.
    pub bits: String,
}

/// The document a case computes now, for these scorer sets.
///
/// # Errors
/// As [`header`], [`answers`] and [`block`].
pub fn document(
    name: &str,
    case: &Case,
    subject: &Subject,
    scorings: &[Scoring],
) -> Result<Computed, String> {
    let answers = answers(case, subject)?;
    let mut doc = Doc {
        header: header(subject)?,
        blocks: Vec::new(),
    };
    let mut bits = Sha256::new();
    for scoring in scorings {
        doc.blocks
            .push(block(name, subject, scoring, &answers, &mut bits)?);
    }
    Ok(Computed {
        doc,
        bits: hex(&bits.finalize()),
    })
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut out, b| {
        let _ = write!(out, "{b:02x}");
        out
    })
}

/// Feed every number of a report into the digest, as bits.
fn feed(h: &mut Sha256, r: &ScoreReport) {
    let mut put = |x: f64| h.update(x.to_bits().to_be_bytes());
    for x in [
        r.sound.percent,
        r.sound.distance,
        r.sound.floor,
        r.sound.scale,
        r.sound.jnd,
        r.structure.percent,
        r.combined.percent,
    ] {
        put(x);
    }
    for c in &r.sound.components {
        for x in [c.target, c.answer] {
            put(x.unwrap_or(f64::NAN));
        }
    }
    for p in &r.structure.params {
        put(p.score);
    }
}
