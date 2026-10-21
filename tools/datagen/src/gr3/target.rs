//! A GR3 synth target: its patch, the performance it is heard and scored through, its upload, its scorers and its
//! oracle template (arm D).

use std::fs;

use enth_analysis::distance::mfcc_mse;
use enth_analysis::features::{FeatureCfg, FeatureExtractor, Features};
use enth_challenge::code::ChallengeCode;
use enth_challenge::model::{Phrase, PhraseNote, SCORING_SAMPLE_RATE, SCORING_SEED};
use enth_challenge::patch_round::moved_params;
use enth_challenge::random::{self, observable};
use enth_challenge::score::Scorer;
use enth_challenge::score::sound::{SoundPrepared, SoundV1};
use enth_challenge::score::structure::{self, StructureV1};
use enth_challenge::v2::InlinePatch;
use enth_challenge::{
    Challenge, PhraseId, PreparedRound, ScoringSpec, Start, Target as ChallengeTarget,
    TargetContext, prepare_round, score_round,
};
use enth_patch::{ModuleKind, Op, ParamAddr, Patch, apply, apply_all};
use enth_render::{RenderSpec, render};
use serde::Serialize;
use sha2::{Digest, Sha256};

use super::job::{Kind, TargetJob};

/// The strata clause 1 reports apart (docs/gr3.md): `create` plays one note in every arm, so the generator's
/// two-note phrases (octave and legato pairs) are their own stratum (the F1s judge).
pub fn stratum(kind: Kind, phrase: Option<u16>) -> &'static str {
    match (kind, phrase) {
        (Kind::Preset, _) => "preset",
        (Kind::Generator, Some(1) | None) => "gen-1note",
        (Kind::Generator, Some(_)) => "gen-2note",
    }
}

/// The SHA-256 of a patch's compact JSON (`Patch::to_json`): what a record keeps instead of the patch.
pub fn patch_sha256(p: &Patch) -> String {
    hex(&Sha256::digest(p.to_json().as_bytes()))
}

fn hex(bytes: &[u8]) -> String {
    enth_solver::create::record::hex(bytes)
}

/// A target, built and checked.
pub struct Gr3Target {
    /// Its id.
    pub id: String,
    /// The target patch.
    pub patch: Patch,
    /// The performance it is heard through and scored through: the code's phrase, or the preset's one note.
    pub phrase: &'static Phrase,
    /// The generator spec (generator targets): the game's own round scores them.
    spec: Option<enth_challenge::RandomSpec>,
    /// Its stratum.
    pub stratum: &'static str,
}

/// A preset's performance as a one-note phrase. Phrases hold `'static` notes; a run builds a few dozen, so leaking
/// them is the simple way.
fn one_note(key: u8, velocity: u8, hold_ms: u32, length_ms: u32) -> &'static Phrase {
    let notes: &'static [PhraseNote] = Box::leak(Box::new([PhraseNote {
        key,
        velocity,
        start_ms: 0,
        hold_ms,
    }]));
    Box::leak(Box::new(Phrase { notes, length_ms }))
}

impl Gr3Target {
    /// Build `t`: regenerate a code and check its fingerprint and phrase, or read a preset file and check its
    /// SHA-256.
    pub fn build(t: &TargetJob) -> Result<Gr3Target, String> {
        match t.kind {
            Kind::Generator => {
                let code = t
                    .code
                    .as_deref()
                    .ok_or("a generator target needs its code")?;
                let c = ChallengeCode::decode(code).map_err(|e| format!("code {code}: {e}"))?;
                let spec = *c.spec();
                let g = random::target(&spec).map_err(|e| format!("code {code}: {e}"))?;
                let fp = g.fingerprint.to_hex();
                if let Some(want) = &t.fingerprint
                    && !want.eq_ignore_ascii_case(&fp)
                {
                    return Err(format!(
                        "code {code} regenerates to fingerprint {fp}, but the manifest lists {want}: refusing a moved target"
                    ));
                }
                if let Some(want) = t.phrase
                    && want != g.phrase.0
                {
                    return Err(format!(
                        "code {code} plays phrase {}, but the manifest lists {want}",
                        g.phrase.0
                    ));
                }
                Ok(Gr3Target {
                    id: t.id.clone(),
                    phrase: g.phrase(),
                    stratum: stratum(t.kind, Some(g.phrase.0)),
                    patch: g.patch,
                    spec: Some(spec),
                })
            }
            Kind::Preset => {
                let path = t.file.as_ref().ok_or("a preset target needs its file")?;
                let bytes =
                    fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
                let sha = hex(&Sha256::digest(&bytes));
                if let Some(want) = &t.sha256
                    && !want.eq_ignore_ascii_case(&sha)
                {
                    return Err(format!(
                        "{} has SHA-256 {sha}, but the manifest lists {want}: refusing a moved target",
                        path.display()
                    ));
                }
                let text = String::from_utf8(bytes)
                    .map_err(|_| format!("{} is not UTF-8", path.display()))?;
                let patch = enth_patch::file::load(&text)
                    .map_err(|e| format!("{} does not load: {e}", path.display()))?
                    .patch;
                let p = t.perf.ok_or("a preset target needs its performance")?;
                Ok(Gr3Target {
                    id: t.id.clone(),
                    patch,
                    phrase: one_note(p.key, p.velocity, p.hold_ms, p.length_ms),
                    spec: None,
                    stratum: stratum(t.kind, None),
                })
            }
        }
    }

    /// The key the target's performance plays first: what the intake's key is compared with (FE's key is "the first
    /// of several notes").
    pub fn true_key(&self) -> u8 {
        self.phrase.notes.first().map_or(48, |n| n.key)
    }

    /// The upload: the target patch through its performance at 48 kHz with the default render seed (the scoring
    /// seed), mono.
    pub fn upload(&self) -> Result<Vec<f32>, String> {
        render_mid(&self.patch, self.phrase)
    }

    /// The scorers, prepared once (a couple of dozen renders).
    pub fn scorer(&self) -> Result<Gr3Scorer, String> {
        let target_features = features(&self.upload()?);
        let kind = if let Some(spec) = &self.spec {
            ScorerKind::Round(Box::new(
                prepare_round(spec).map_err(|e| format!("{}: the round: {e}", self.id))?,
            ))
        } else {
            let start = random::start(0).map_err(|e| e.to_string())?;
            let randomized = moved_params(&self.patch, self.phrase);
            let challenge = Challenge::new(
                ChallengeTarget::Patch(InlinePatch::new(self.patch.clone())),
                Start::Template { template: 0 },
                PhraseId(1),
                ScoringSpec::recreate(0),
            );
            let ctx = TargetContext {
                challenge: &challenge,
                target: &self.patch,
                start: &start,
                phrase: self.phrase,
                randomized: &randomized,
            };
            let sound = SoundV1
                .prepare(&ctx)
                .map_err(|e| format!("{}: sound/1: {e}", self.id))?;
            let structure = StructureV1
                .prepare(&ctx)
                .map_err(|e| format!("{}: structure/1: {e}", self.id))?;
            ScorerKind::Hand(Box::new((sound, structure)))
        };
        Ok(Gr3Scorer {
            kind,
            phrase: self.phrase,
            target_features,
        })
    }

    /// Arm D's oracle template: the target's own structure (its modules, their enable flags, its routes and their
    /// depths) with every parameter its performance reveals set to the catalogue default, so the search starts from
    /// the right topology and none of the right values.
    pub fn oracle_template(&self) -> Result<Patch, String> {
        let mask = observable(&self.patch, self.phrase);
        let ops: Vec<Op> = mask
            .observable_params()
            .filter_map(|addr| {
                let spec = self.patch.spec(addr)?;
                Some(Op::SetParam {
                    addr: addr.clone(),
                    value: spec.default,
                })
            })
            .collect();
        let mut p = self.patch.clone();
        apply_all(&mut p, &ops).map_err(|e| format!("{}: the oracle template: {e}", self.id))?;
        Ok(p)
    }
}

/// `patch` with every enabled oscillator's `semi` moved by `semis` (clamped, through ops): the rule
/// `enth_solver::nearest` transposes a neighbour by. The kit uses it to judge a candidate **as played at the note
/// create inferred**: a candidate searched at key `k_inferred` and scored through the target's performance (which
/// starts on `k_true`) is first moved by `k_inferred − k_true`, because one note cannot tell the key from the
/// oscillators' tuning (a target whose oscillators sit an octave down sounds like the key an octave lower, and the
/// intake rightly reports that). Every arm is judged the same way; the raw score is recorded beside it.
pub fn transpose(patch: &Patch, semis: i32) -> Patch {
    let mut p = patch.clone();
    if semis == 0 {
        return p;
    }
    let addrs: Vec<ParamAddr> = p
        .generators
        .iter()
        .filter(|m| m.enabled && m.kind == ModuleKind::Osc)
        .filter_map(|m| ParamAddr::module(m.id.clone(), "semi").ok())
        .collect();
    for addr in addrs {
        let (Some(spec), Some(v)) = (p.spec(&addr), p.value(&addr)) else {
            continue;
        };
        let value = spec.clamp(v + semis as f32);
        let _ = apply(&mut p, Op::SetParam { addr, value });
    }
    p
}

fn render_mid(patch: &Patch, phrase: &Phrase) -> Result<Vec<f32>, String> {
    render(
        patch,
        &phrase.performance(),
        &RenderSpec::new(SCORING_SAMPLE_RATE, SCORING_SEED),
    )
    .map(|o| o.mid())
    .map_err(|e| format!("render: {e}"))
}

fn features(mid: &[f32]) -> Features {
    FeatureExtractor::new(FeatureCfg {
        sample_rate: f64::from(SCORING_SAMPLE_RATE),
        ..FeatureCfg::default()
    })
    .extract(mid)
}

enum ScorerKind {
    /// A generator target: the game's own round (`prepare_round`, `score_round`).
    Round(Box<PreparedRound>),
    /// A preset: `sound/1` and `structure/1` prepared on a hand-made context through its one-note performance
    /// (the parameters it moved off their defaults, as `enth compare --target` compares).
    Hand(Box<(SoundPrepared, structure::Prepared)>),
}

/// The scorers of one target.
pub struct Gr3Scorer {
    kind: ScorerKind,
    phrase: &'static Phrase,
    target_features: Features,
}

/// What a patch scores against the target.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct ScoreOut {
    /// `sound/1` %.
    pub percent: f64,
    /// `sound/1`'s distance in JND units.
    pub jnd: f64,
    /// `sound/1`'s raw metric v1.1 distance.
    pub distance: f64,
    /// `sound/1` % at or above 90.
    pub passed: bool,
    /// `structure/1` %.
    pub structure: f64,
    /// The independent metric no arm optimises: MFCC mean squared error between the two scoring renders.
    pub mfcc: f64,
}

/// The pass mark (`sound/1` %).
pub const PASS_MARK: f64 = 90.0;

impl Gr3Scorer {
    /// Score `answer`.
    pub fn score(&self, answer: &Patch) -> Result<ScoreOut, String> {
        let (percent, jnd, distance, structure) = match &self.kind {
            ScorerKind::Round(round) => {
                let r = score_round(round, answer).map_err(|e| format!("score: {e}"))?;
                (
                    r.sound.percent,
                    r.sound.jnd,
                    r.sound.distance,
                    r.structure.percent,
                )
            }
            ScorerKind::Hand(h) => {
                let s = SoundV1
                    .score(&h.0, answer)
                    .map_err(|e| format!("sound/1: {e}"))?;
                let st = StructureV1
                    .score(&h.1, answer)
                    .map_err(|e| format!("structure/1: {e}"))?;
                (s.percent, s.jnd, s.distance, st.percent)
            }
        };
        let mfcc = mfcc_mse(
            &self.target_features,
            &features(&render_mid(answer, self.phrase)?),
        );
        Ok(ScoreOut {
            percent,
            jnd,
            distance,
            passed: percent >= PASS_MARK,
            structure,
            mfcc,
        })
    }
}
