//! **Owner: lane CV2** (Challenge v2, freeze point F1l). Pre-declared by lane S0l (R2). The reference is
//! docs/challenge.md, "Challenge v2" ("Rounds on patch and draw targets").
//!
//! [`prepare_patch_round`]: the "hand-made round on a patch target" that lived in `enth-cli`'s `cmd/compare.rs`
//! (`moved_params` and the context it built), moved down so the session (a lesson's Recreate step), xtask
//! (`lesson-check`) and the CLI share it. The CLI keeps a thin call to it, and `enth compare --target`'s output is
//! byte-identical (CV2's acceptance; `enth-cli`'s compare snapshots pin it).
//!
//! A patch target was not made by a generator, so the round has to say what it compares (`TargetContext::
//! randomized`, the rings, structure %). [`Compared`] names the two rules:
//!
//! | Rule | Compares | Used by |
//! | --- | --- | --- |
//! | [`Compared::FromStart`] | the parameters the target changes from the round's start, heard through the phrase | a challenge's `Target::Patch` ([`crate::prepare_challenge`]): the start is the lesson's own, so this is what the player has to change |
//! | [`Compared::OffDefaults`] | the parameters the target moved off their catalogue defaults, heard through the phrase | `enth compare --target`: a reference patch has no start of its own (the starter stands in), so this is what its author set |
//!
//! Routes and enable flags are compared as the scorers' hand-made context reads them (`score::matching::Draws::of`):
//! what the target changes from the start. A draw target ([`prepare_draw_round`]) compares exactly what it drew.

use enth_patch::{ModuleId, ParamAddr, Patch, RouteAddr};

use crate::model::{Challenge, Phrase, Target};
use crate::random::{Drawn, Visibility, observable};
use crate::score::ScoreError;
use crate::score::round::{PreparedRound, RoundError, prepare_parts};
use crate::v2::{DrawAt, hand_made};

/// Which parameters a patch target's round compares (see the [module docs](self)).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Compared {
    /// What the target changes from the round's start: a challenge's patch target.
    FromStart,
    /// What the target moved off the catalogue's defaults: `enth compare --target`, whose reference has no start.
    OffDefaults,
}

/// The reference's observable parameters that are not at their catalogue default: what its author set
/// ([`Compared::OffDefaults`]; `enth compare --target`'s rule, moved here unchanged).
pub fn moved_params(reference: &Patch, phrase: &Phrase) -> Vec<ParamAddr> {
    let mask = observable(reference, phrase);
    let mut out: Vec<ParamAddr> = mask
        .observable_params()
        .filter(|addr| {
            matches!(
                (reference.spec(addr), reference.value(addr)),
                (Some(spec), Some(v)) if v.to_bits() != spec.default.to_bits()
            )
        })
        .cloned()
        .collect();
    out.sort();
    out.dedup();
    out
}

/// The target's observable parameters whose value differs from the start's (or that the start lacks): what the player
/// has to change ([`Compared::FromStart`]).
pub fn changed_params(target: &Patch, start: &Patch, phrase: &Phrase) -> Vec<ParamAddr> {
    let mask = observable(target, phrase);
    let mut out: Vec<ParamAddr> = mask
        .observable_params()
        .filter(|addr| match (target.value(addr), start.value(addr)) {
            (Some(t), Some(s)) => t.to_bits() != s.to_bits(),
            (Some(_), None) => true,
            (None, _) => false,
        })
        .cloned()
        .collect();
    out.sort();
    out.dedup();
    out
}

/// The heard routes the target changes from the start, and the modules whose enable flag it changes: what the
/// scorers' hand-made context compares besides parameters, in the same order (`score::matching::Draws::of`).
fn changed_routes_and_enables(target: &Patch, start: &Patch, phrase: &Phrase) -> Vec<Drawn> {
    let mask = observable(target, phrase);
    let mut routes: Vec<RouteAddr> = target
        .routes
        .iter()
        .filter(|r| {
            !start
                .routes
                .iter()
                .any(|s| s.src == r.src && s.dst == r.dst && s.depth == r.depth)
        })
        .map(|r| RouteAddr {
            src: r.src.clone(),
            dst: r.dst.clone(),
        })
        .filter(|r| mask.route(r).is_some_and(Visibility::is_observable))
        .collect();
    routes.sort();
    routes.dedup();
    let mut enables: Vec<ModuleId> = target
        .modules()
        .filter(|(_, m)| {
            start
                .find(&m.id)
                .is_some_and(|(_, s)| s.kind == m.kind && s.enabled != m.enabled)
        })
        .map(|(_, m)| m.id.clone())
        .collect();
    enables.sort();
    enables.dedup();
    routes
        .into_iter()
        .map(Drawn::Route)
        .chain(enables.into_iter().map(Drawn::Enable))
        .collect()
}

/// The phrase a challenge plays, from either table.
fn phrase_of(challenge: &Challenge) -> Result<&'static Phrase, RoundError> {
    crate::phrases::phrase(challenge.phrase).ok_or(RoundError::Challenge(
        crate::model::ChallengeError::UnknownPhrase {
            phrase: challenge.phrase.0,
        },
    ))
}

/// Prepare the round of a challenge whose target is a patch (`Target::Patch`): validate it, take its start, compare
/// what `compared` says, and prepare the scorers it names. The round's fingerprint is the patch target's
/// ([`crate::fingerprint::of_patch`]); its [`generated`](PreparedRound::generated) is the same shape a generator
/// fills, with generator [`NOT_GENERATED`](crate::v2::NOT_GENERATED). A silent target is scored as given, as
/// [`crate::prepare_challenge`] does.
///
/// `enth compare --target REF --phrase N` is `prepare_patch_round(Challenge::new(Target::Patch(REF), template 0, N,
/// ScoringSpec::recreate(0)), Compared::OffDefaults)` and [`crate::score_round`].
pub fn prepare_patch_round(
    challenge: Challenge,
    compared: Compared,
) -> Result<PreparedRound, RoundError> {
    challenge.validate().map_err(RoundError::Challenge)?;
    let Target::Patch(target) = &challenge.target else {
        return Err(RoundError::Score(ScoreError::Unscorable {
            reason: "prepare_patch_round takes a challenge with a patch target".to_owned(),
        }));
    };
    let target: Patch = target.patch().clone();
    let start = challenge.start_patch().map_err(RoundError::Challenge)?;
    let phrase = phrase_of(&challenge)?;
    let params = match compared {
        Compared::FromStart => changed_params(&target, &start, phrase),
        Compared::OffDefaults => moved_params(&target, phrase),
    };
    let drawn: Vec<Drawn> = params
        .iter()
        .cloned()
        .map(Drawn::Param)
        .chain(changed_routes_and_enables(&target, &start, phrase))
        .collect();
    let generated = hand_made(target, challenge.phrase, drawn, params);
    prepare_parts(challenge, start, generated)
}

/// Prepare the round of a challenge whose target is a draw (`Target::Draw`): draw it over the start through the
/// challenge's phrase ([`crate::v2::Draw::draw`]), compare the drawn parameters and routes, and prepare the scorers
/// it names. The fingerprint is the drawn patch's ([`crate::fingerprint::of_patch`]).
pub fn prepare_draw_round(challenge: Challenge) -> Result<PreparedRound, RoundError> {
    challenge.validate().map_err(RoundError::Challenge)?;
    let Some(draw) = challenge.draw() else {
        return Err(RoundError::Score(ScoreError::Unscorable {
            reason: "prepare_draw_round takes a challenge with a draw target".to_owned(),
        }));
    };
    let start = challenge.start_patch().map_err(RoundError::Challenge)?;
    let phrase = phrase_of(&challenge)?;
    let drawing = draw.draw(&start, phrase).map_err(RoundError::Draw)?;
    let params = drawing.params();
    let drawn: Vec<Drawn> = drawing
        .values
        .iter()
        .map(|(at, _)| match at {
            DrawAt::Param(a) => Drawn::Param(a.clone()),
            DrawAt::Route(r) => Drawn::Route(r.clone()),
        })
        .collect();
    let generated = hand_made(drawing.patch, challenge.phrase, drawn, params);
    prepare_parts(challenge, start, generated)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lesson_phrases::CHORD;
    use crate::model::{ScoringSpec, Start};
    use crate::phrases::{self, HELD};
    use crate::score::combined::CombinedV1;
    use crate::score::sound::SoundV1;
    use crate::score::structure::StructureV1;
    use crate::score::{Combiner, ScoreReport, Scorer, ScorerId, TargetContext, Verdict};
    use crate::spec::{LATEST_GENERATOR, SpecPreset};
    use crate::v2::{Draw, DrawItem, DrawValue, DrawValues, InlinePatch, NOT_GENERATED};
    use crate::{prepare_challenge, prepare_round, random, score_round};
    use enth_patch::{Op, Route, apply};

    /// The hand-made round `enth compare --target` built before the move (`enth-cli`'s `compare.rs` at F0l,
    /// `4a76bc9`), kept here verbatim as the reference: a pseudo-seed challenge at level 1, the starter as the start,
    /// the reference's moved parameters, and the three v1 scorers composed by hand.
    fn compare_at_f0l(
        reference: &Patch,
        phrase: crate::model::PhraseId,
        answer: &Patch,
    ) -> ScoreReport {
        let entry = phrases::entry(phrase).unwrap();
        let start = random::start(0).unwrap();
        let randomized = moved_params(reference, &entry.phrase);
        let spec = SpecPreset::by_id(1).unwrap().spec(LATEST_GENERATOR, 0);
        let mut challenge = Challenge::recreate(spec, entry.id);
        challenge.scoring = ScoringSpec::recreate(0);
        let ctx = TargetContext {
            challenge: &challenge,
            target: reference,
            start: &start,
            phrase: &entry.phrase,
            randomized: &randomized,
        };
        let scoring = &ctx.challenge.scoring;
        let sound = SoundV1
            .score(&SoundV1.prepare(&ctx).unwrap(), answer)
            .unwrap();
        let structure = StructureV1
            .score(&StructureV1.prepare(&ctx).unwrap(), answer)
            .unwrap();
        let combined = CombinedV1.combine(scoring, &sound, &structure);
        ScoreReport::new(scoring, sound, structure, combined)
    }

    fn compare_now(
        reference: &Patch,
        phrase: crate::model::PhraseId,
        answer: &Patch,
    ) -> ScoreReport {
        let challenge = Challenge::new(
            Target::Patch(InlinePatch::new(reference.clone())),
            Start::Template { template: 0 },
            phrase,
            ScoringSpec::recreate(0),
        );
        let round = prepare_patch_round(challenge, Compared::OffDefaults).unwrap();
        score_round(&round, answer).unwrap()
    }

    /// CV2's acceptance: the moved-down round gives `enth compare --target` the same report, bit for bit (the JSON
    /// bytes, which print every score with shortest round-trip text). Cases: a level-3 and a level-5 target (routes
    /// and enable flags), the level-1 seed-0 target the old pseudo-seed challenge regenerated, and the octave pair.
    #[test]
    fn the_patch_round_gives_compare_the_report_it_gave_before() {
        let start = random::start(0).unwrap();
        let target = |level: u8, seed: u64| {
            prepare_round(&SpecPreset::by_id(level).unwrap().spec(1, seed))
                .unwrap()
                .target()
                .clone()
        };
        // Three patches per case: kept on the heap (a patch array would be a large stack value).
        let mut cases = Vec::new();
        cases.push((target(3, 7), HELD, start.clone()));
        cases.push((target(5, 2), phrases::OCTAVE_PAIR, target(2, 2)));
        cases.push((target(1, 0), HELD, start.clone()));
        for (i, (reference, phrase, answer)) in cases.iter().enumerate() {
            let before =
                serde_json::to_string(&compare_at_f0l(reference, *phrase, answer)).unwrap();
            let now = serde_json::to_string(&compare_now(reference, *phrase, answer)).unwrap();
            assert_eq!(now, before, "case {i}");
            let exact = compare_now(reference, *phrase, reference);
            assert_eq!(
                exact.sound.percent, 100.0,
                "case {i}: the reference scores 100"
            );
        }
    }

    #[test]
    fn a_challenges_patch_target_compares_what_it_changes_from_its_start() {
        let mut start = random::start(0).unwrap();
        apply(
            &mut start,
            Op::SetParam {
                addr: "filter1.cutoff".parse().unwrap(),
                value: 900.0,
            },
        )
        .unwrap();
        let mut target = start.clone();
        apply(
            &mut target,
            Op::SetParam {
                addr: "osc1.semi".parse().unwrap(),
                value: 7.0,
            },
        )
        .unwrap();
        apply(
            &mut target,
            Op::InsertRoute {
                index: 0,
                route: Route {
                    src: "env2".parse().unwrap(),
                    dst: "filter1.cutoff".parse().unwrap(),
                    depth: 40.0,
                },
            },
        )
        .unwrap();
        let challenge = Challenge::new(
            Target::Patch(InlinePatch::new(target.clone())),
            Start::Patch {
                patch: InlinePatch::new(start.clone()),
            },
            HELD,
            ScoringSpec::with_scorers(
                0,
                ScorerId::SOUND_V1,
                ScorerId::STRUCTURE_V1,
                ScorerId::COMBINED_V1,
                ScorerId::TIME_V1,
            ),
        );
        let round = prepare_challenge(challenge.clone()).unwrap();
        assert_eq!(round.challenge(), &challenge, "the challenge as given");
        assert_eq!(round.params(), ["osc1.semi".parse::<ParamAddr>().unwrap()]);
        assert_eq!(round.start(), &start);
        assert_eq!(round.target(), &target);
        assert_eq!(round.generated().generator, NOT_GENERATED);
        assert_eq!(
            round.fingerprint(),
            crate::fingerprint::of_patch(&target, HELD)
        );
        let exact = score_round(&round, &target).unwrap();
        assert_eq!(exact.sound.percent, 100.0);
        assert!(exact.passed);
        let from_start = score_round(&round, &start).unwrap();
        assert!(!from_start.passed, "{}", from_start.sound.percent);
        let rows: Vec<&str> = from_start
            .structure
            .params
            .iter()
            .filter(|p| p.randomized)
            .map(|p| p.addr.as_str())
            .collect();
        assert_eq!(rows, ["osc1.semi", "env2 -> filter1.cutoff"]);
        // The same target under the compare rule compares what it moved off the defaults.
        let off = prepare_patch_round(challenge, Compared::OffDefaults).unwrap();
        assert!(off.params().len() > 1);
    }

    fn cutoff_draw(seed: u64, phrase: crate::model::PhraseId) -> Challenge {
        Challenge::new(
            Target::Draw(Draw {
                seed,
                items: vec![
                    DrawItem {
                        at: crate::v2::DrawAt::Param("filter1.cutoff".parse().unwrap()),
                        values: DrawValues::Set(
                            [300.0, 600.0, 4000.0, 8000.0]
                                .into_iter()
                                .map(DrawValue::Number)
                                .collect(),
                        ),
                    },
                    DrawItem {
                        at: crate::v2::DrawAt::Route("env2 -> filter1.cutoff".parse().unwrap()),
                        values: DrawValues::Set(
                            [-40.0, 40.0].into_iter().map(DrawValue::Number).collect(),
                        ),
                    },
                ],
            }),
            Start::Template { template: 0 },
            phrase,
            ScoringSpec::with_scorers(
                0,
                ScorerId::SOUND_V1,
                ScorerId::STRUCTURE_V1,
                ScorerId::COMBINED_V1,
                ScorerId::TIME_V1,
            ),
        )
    }

    #[test]
    fn a_draw_round_scores_its_target_and_rings_what_it_drew() {
        let challenge = cutoff_draw(5, HELD);
        let round = prepare_challenge(challenge.clone()).unwrap();
        assert_eq!(round.challenge(), &challenge);
        let draw = challenge.draw().unwrap();
        let drawing = draw
            .draw(&random::start(0).unwrap(), phrases::phrase(HELD).unwrap())
            .unwrap();
        assert_eq!(round.target(), &drawing.patch);
        assert_eq!(
            round.params(),
            ["filter1.cutoff".parse::<ParamAddr>().unwrap()]
        );
        assert_eq!(
            round.fingerprint(),
            crate::fingerprint::of_patch(&drawing.patch, HELD)
        );
        assert_eq!(round.generated().randomized.len(), 2);
        let exact = score_round(&round, round.target()).unwrap();
        assert_eq!(exact.sound.percent, 100.0);
        assert_eq!(exact.structure.percent, 100.0);
        let start = score_round(&round, round.start()).unwrap();
        let rows: Vec<(&str, Verdict)> = start
            .structure
            .params
            .iter()
            .filter(|p| p.randomized)
            .map(|p| (p.addr.as_str(), p.verdict))
            .collect();
        assert_eq!(rows.len(), 2, "{rows:?}");
        assert_eq!(rows[0].0, "filter1.cutoff");
        assert_eq!(rows[1], ("env2 -> filter1.cutoff", Verdict::Bad));
        // Another seed is a sibling: same items, its own target, and it prepares again identically.
        let again = prepare_challenge(challenge.clone()).unwrap();
        assert_eq!(again.fingerprint(), round.fingerprint());
        let siblings: std::collections::BTreeSet<_> = (0..6)
            .map(|s| {
                prepare_challenge(challenge.with_seed(s).unwrap())
                    .unwrap()
                    .fingerprint()
            })
            .collect();
        assert!(siblings.len() > 2, "{}", siblings.len());
    }

    /// A route a draw removes (depth 0 over a start that has it) is still compared: as absent.
    #[test]
    fn a_route_drawn_away_is_rung_as_absent() {
        let mut start = random::start(0).unwrap();
        let route: enth_patch::RouteAddr = "env2 -> filter1.cutoff".parse().unwrap();
        apply(
            &mut start,
            Op::InsertRoute {
                index: 0,
                route: Route {
                    src: route.src.clone(),
                    dst: route.dst.clone(),
                    depth: 50.0,
                },
            },
        )
        .unwrap();
        let mut challenge = cutoff_draw(1, HELD);
        challenge.target = Target::Draw(Draw {
            seed: 1,
            items: vec![DrawItem {
                at: crate::v2::DrawAt::Route(route.clone()),
                values: DrawValues::Set(vec![DrawValue::Number(0.0)]),
            }],
        });
        challenge.start = Start::Patch {
            patch: InlinePatch::new(start.clone()),
        };
        let round = prepare_challenge(challenge).unwrap();
        assert_eq!(round.target().routes, []);
        let report = score_round(&round, &start).unwrap();
        let row = report
            .structure
            .params
            .iter()
            .find(|p| p.addr == route.to_string())
            .expect("the removed route has a row");
        assert!(row.randomized);
        assert_eq!(row.verdict, Verdict::Bad);
        assert!(score_round(&round, round.target()).unwrap().passed);
    }

    /// A lesson-only phrase plays a lesson round (the chord: four notes at once).
    #[test]
    fn a_lesson_phrase_plays_a_round() {
        let round = prepare_challenge(cutoff_draw(2, CHORD)).unwrap();
        assert_eq!(round.phrase(), phrases::phrase(CHORD).unwrap());
        assert!(score_round(&round, round.target()).unwrap().passed);
    }

    #[test]
    fn the_wrong_target_kind_is_refused() {
        let seed = Challenge::recreate(SpecPreset::by_id(1).unwrap().spec(1, 1), HELD);
        assert_eq!(
            prepare_patch_round(seed.clone(), Compared::FromStart)
                .unwrap_err()
                .code(),
            "unscorable"
        );
        assert_eq!(prepare_draw_round(seed).unwrap_err().code(), "unscorable");
    }
}
