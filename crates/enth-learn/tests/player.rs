//! The player's basic body (lane LM): the lifecycle the session (lane SL) drives, step by step. PLY adds the
//! properties under random intents (`tests/player_props.rs`).

mod common;

use std::sync::Arc;

use enth_learn::player::{CheckVerdict, MeasuredHint, SETTLE_MS};
use enth_learn::{EditSource, Effect, EndChoice, Pack, Player, PlayerError, RoundEnd, StepPhase};
use enth_patch::{Op, ParamAddr, Patch};

const CHALLENGE: &str = r#"{"schema":"enth.challenge/1","target":{"kind":"seed","spec":{"generator":1,"seed":42,"difficulty":0,"amounts":[null,null,null,null,null,null],"musical":100,"template":0}},"answer":"patch","phrase":1,"start":{"kind":"template","template":0},"constraints":{},"assists":{"live_meter":false,"dim_unused":false,"hint_budget":null},"scoring":{"sound":"sound/1","structure":"structure/1","combined":"combined/1","time":"time/1","sound_weight":50,"pass_mark":90},"meta":{"title":"","tags":[]}}"#;

fn lesson() -> String {
    format!(
        r#"{{"schema":"enth.lesson/1","id":"play","title":"Play","tier":"basic","objective":"Play every kind of step.","teaches":["svf.cutoff"],
        "steps":[
          {{"id":"intro","kind":"read","text":"text.md"}},
          {{"id":"turn","kind":"do","text":"text.md","hints":["Turn it left.","Further."],
            "view":{{"start":"patches/saw.patch.json","editable":["filter1.cutoff"],"lock":["osc1.*"]}},
            "check":{{"kind":"patch","predicates":[{{"kind":"value","knob":"filter1.cutoff","cmp":"<","value":"1kHz"}}]}},
            "witness":"patches/dark.patch.json"}},
          {{"id":"darker","kind":"do","view":{{"start":"patches/saw.patch.json","editable":["filter1.cutoff"]}},
            "check":{{"kind":"sound","phrase":1,"expect":["brightness<500Hz"]}},"witness":"patches/dark.patch.json"}},
          {{"id":"which","kind":"choose","text":"text.md","options":[
            {{"label":"A","patch":"patches/saw.patch.json"}},{{"label":"B","patch":"patches/dark.patch.json","transpose":-12}}],"answer":1}},
          {{"id":"round","kind":"recreate","challenge":{CHALLENGE}}}
        ]}}"#
    )
}

fn player() -> Player {
    let files = common::test_files(&[("play", &lesson())]);
    let pack = Pack::from_files(&files).unwrap();
    Player::new(pack.bundle(&"play".parse().unwrap()).unwrap())
}

fn patch(name: &str) -> Patch {
    enth_patch::file::load(&common::example_patch(name))
        .unwrap()
        .patch
}

fn loads(effects: &[Effect]) -> usize {
    effects
        .iter()
        .filter(|e| matches!(e, Effect::LoadPatch(_)))
        .count()
}

fn passed(effects: &[Effect]) -> Option<bool> {
    effects.iter().find_map(|e| match e {
        Effect::Passed { assisted, .. } => Some(*assisted),
        _ => None,
    })
}

fn record(effects: &[Effect]) -> Option<&enth_learn::player::StepRecord> {
    effects.iter().find_map(|e| match e {
        Effect::Record(r) => Some(r),
        _ => None,
    })
}

fn cutoff(value: f32) -> Op {
    Op::SetParam {
        addr: "filter1.cutoff".parse::<ParamAddr>().unwrap(),
        value,
    }
}

/// Step through to `step`, leaving each step on the way with Next.
fn at(step: u16) -> Player {
    let mut p = player();
    p.start().unwrap();
    while p.step() < step {
        p.next().unwrap();
    }
    p
}

#[test]
fn a_read_step_loads_no_plan_and_passes_on_next() {
    let mut p = player();
    let fx = p.start().unwrap();
    assert_eq!(loads(&fx), 0, "no start on the read step");
    assert!(matches!(fx.last(), Some(Effect::View(_))));
    let fx = p.next().unwrap();
    assert_eq!(passed(&fx), Some(false));
    let r = record(&fx).unwrap();
    assert_eq!(
        (r.lesson.to_string(), r.step.as_str()),
        ("test/play".to_owned(), "intro")
    );
    assert!(r.progress.done && r.progress.clean);
    assert_eq!(loads(&fx), 1, "a step load sends exactly one plan");
    assert_eq!(p.step(), 1);
    let view = fx.iter().find_map(|e| match e {
        Effect::View(v) => Some(v),
        _ => None,
    });
    assert_eq!(view.unwrap().lock[0].to_string(), "osc1.*");
}

#[test]
fn a_patch_check_passes_on_the_edit_that_satisfies_it() {
    let mut p = at(1);
    let saw = patch("saw.patch.json");
    assert!(
        p.edited(0, 1, &saw, &[], EditSource::Load)
            .unwrap()
            .is_empty()
    );
    let mut darker = saw.clone();
    cutoff(1500.0).apply(&mut darker).unwrap();
    assert!(
        p.edited(10, 2, &darker, &[cutoff(1500.0)], EditSource::Player)
            .unwrap()
            .is_empty()
    );
    cutoff(800.0).apply(&mut darker).unwrap();
    let fx = p
        .edited(20, 3, &darker, &[cutoff(800.0)], EditSource::Player)
        .unwrap();
    assert_eq!(passed(&fx), Some(false));
    assert_eq!(p.phase(), StepPhase::Passed);
    // Passed is sticky: undoing the edit does not unpass the step.
    assert!(
        p.edited(30, 4, &saw, &[cutoff(2000.0)], EditSource::Player)
            .unwrap()
            .is_empty()
    );
    assert_eq!(p.phase(), StepPhase::Passed);
}

#[test]
fn hints_and_show_mark_the_step_assisted() {
    let mut p = at(1);
    let saw = patch("saw.patch.json");
    p.edited(0, 1, &saw, &[], EditSource::Load).unwrap();
    assert_eq!(
        p.hint().unwrap(),
        vec![Effect::Hint("Turn it left.".to_owned())]
    );
    assert!(p.assisted());
    assert_eq!(p.hint().unwrap(), vec![Effect::Hint("Further.".to_owned())]);
    assert!(
        p.hint().unwrap().is_empty(),
        "a patch-only check has no measured hint"
    );
    let fx = p.show(&saw).unwrap();
    assert_eq!(
        fx,
        vec![Effect::Apply(cutoff(400.0))],
        "the witness's op on the editable control"
    );
    let mut shown = saw.clone();
    cutoff(400.0).apply(&mut shown).unwrap();
    let fx = p
        .edited(5, 2, &shown, &[cutoff(400.0)], EditSource::Player)
        .unwrap();
    assert_eq!(passed(&fx), Some(true));
    assert!(!record(&fx).unwrap().progress.clean);
}

#[test]
fn a_sound_check_waits_for_the_patch_to_settle_and_refuses_stale_results() {
    let mut p = at(2);
    let saw = patch("saw.patch.json");
    let dark = patch("dark.patch.json");
    p.edited(1000, 7, &saw, &[], EditSource::Load).unwrap();
    assert!(p.tick(1000 + SETTLE_MS - 1).unwrap().is_empty());
    let fx = p.tick(1000 + SETTLE_MS).unwrap();
    let [Effect::Check(req)] = fx.as_slice() else {
        panic!("{fx:?}")
    };
    assert_eq!((req.revision, req.sounds.len()), (7, 1));
    assert_eq!(p.phase(), StepPhase::Checking { revision: 7 });
    assert!(p.tick(5000).unwrap().is_empty(), "one check per revision");
    // An edit while the check is out sends the step back to work; the old result is refused.
    p.edited(1400, 8, &dark, &[cutoff(400.0)], EditSource::Player)
        .unwrap();
    assert_eq!(p.phase(), StepPhase::Working);
    let pass = CheckVerdict {
        passed: true,
        items: Vec::new(),
    };
    assert_eq!(p.check_done(7, &pass).unwrap_err().code(), "stale_result");
    let fx = p.tick(1400 + SETTLE_MS).unwrap();
    assert!(matches!(fx.as_slice(), [Effect::Check(r)] if r.revision == 8));
    // A failing verdict waits for the next edit.
    assert!(
        p.check_done(8, &CheckVerdict::default())
            .unwrap()
            .is_empty()
    );
    assert!(p.tick(9999).unwrap().is_empty());
    p.edited(2000, 9, &dark, &[], EditSource::Host).unwrap();
    assert!(
        matches!(p.tick(2000 + SETTLE_MS).unwrap().as_slice(), [Effect::Check(r)] if r.revision == 9)
    );
    let fx = p.check_done(9, &pass).unwrap();
    assert_eq!(passed(&fx), Some(false));
    assert_eq!(
        p.check_done(9, &pass).unwrap_err().code(),
        "stale_result",
        "answered once"
    );
}

#[test]
fn a_measured_hint_follows_the_authored_ones_and_is_revision_guarded() {
    let mut p = at(2);
    let saw = patch("saw.patch.json");
    p.edited(0, 3, &saw, &[], EditSource::Load).unwrap();
    let fx = p.hint().unwrap();
    let [Effect::HintJob(req)] = fx.as_slice() else {
        panic!("{fx:?}")
    };
    assert_eq!(req.revision, 3);
    assert_eq!(req.expect, vec!["brightness<500Hz".to_owned()]);
    assert_eq!(
        req.free,
        vec!["filter1.cutoff".parse::<ParamAddr>().unwrap()]
    );
    assert!(
        p.hint().unwrap().is_empty(),
        "one measured hint out per revision"
    );
    let hint = MeasuredHint {
        text: "Darker: the filter's cutoff, lower.".to_owned(),
        highlight: vec!["filter1.cutoff".parse().unwrap()],
    };
    assert_eq!(
        p.hint_done(2, hint.clone()).unwrap_err().code(),
        "stale_result"
    );
    assert_eq!(
        p.hint_done(3, hint.clone()).unwrap(),
        vec![Effect::Measured(hint.clone())]
    );
    assert_eq!(p.hint_done(3, hint).unwrap_err().code(), "stale_result");
}

#[test]
fn an_ear_question_counts_wrong_picks() {
    let mut p = at(3);
    let fx = p.choose(0).unwrap();
    assert_eq!(
        fx,
        vec![Effect::Chose {
            option: 0,
            correct: false
        }]
    );
    let fx = p.choose(1).unwrap();
    assert_eq!(
        fx[0],
        Effect::Chose {
            option: 1,
            correct: true
        }
    );
    assert_eq!(
        passed(&fx),
        Some(true),
        "passed, but not clean after a wrong pick"
    );
    assert_eq!(p.choose(9).unwrap_err().code(), "out_of_range");
    let mut q = at(3);
    let fx = q.show(&Patch::init()).unwrap();
    assert_eq!(fx[0], Effect::Reveal { answer: 1 });
    assert_eq!(passed(&fx), Some(true));
}

#[test]
fn choose_and_listen_steps_carry_their_examples() {
    let mut p = at(2);
    let fx = p.next().unwrap();
    let examples = fx
        .iter()
        .find_map(|e| match e {
            Effect::Examples(x) => Some(Arc::clone(x)),
            _ => None,
        })
        .unwrap();
    assert_eq!(examples.len(), 2);
    assert_eq!(
        (examples[1].label.as_str(), examples[1].transpose),
        ("B", -12)
    );
    assert!(examples[0].level_match);
}

#[test]
fn a_recreate_step_starts_a_round_and_never_loads_a_plan() {
    let mut p = at(3);
    let fx = p.next().unwrap();
    assert_eq!(loads(&fx), 0);
    assert_eq!(
        fx.iter()
            .filter(|e| matches!(e, Effect::StartRound(_)))
            .count(),
        1
    );
    assert_eq!(
        p.hint().unwrap_err(),
        PlayerError::NotHere {
            call: "hint",
            kind: "recreate"
        }
    );
    assert!(
        p.round_ended(RoundEnd {
            passed: false,
            assisted: true,
            best_sound_pct: Some(40)
        })
        .unwrap()
        .is_empty()
    );
    let fx = p
        .round_ended(RoundEnd {
            passed: true,
            assisted: false,
            best_sound_pct: Some(93),
        })
        .unwrap();
    let r = record(&fx).unwrap();
    assert_eq!(
        (r.progress.best, r.progress.clean),
        (Some(93), false),
        "the reveal before still counts"
    );
    assert_eq!(p.next().unwrap(), vec![Effect::Finished]);
    assert!(p.is_finished());
    assert_eq!(
        p.end(EndChoice::Keep).unwrap(),
        vec![Effect::End(EndChoice::Keep)]
    );
    assert_eq!(p.next().unwrap_err(), PlayerError::Ended);
}

#[test]
fn leaving_an_unpassed_round_ends_it() {
    let mut p = at(4);
    let fx = p.back().unwrap();
    assert_eq!(fx[0], Effect::EndRound);
    assert_eq!(p.step(), 3);
    let mut q = at(4);
    assert_eq!(
        q.end(EndChoice::Restore).unwrap(),
        vec![Effect::EndRound, Effect::End(EndChoice::Restore)]
    );
}

#[test]
fn back_reloads_a_step_and_passed_stays_passed() {
    let mut p = at(1);
    let mut dark = patch("saw.patch.json");
    cutoff(400.0).apply(&mut dark).unwrap();
    p.edited(0, 1, &dark, &[cutoff(400.0)], EditSource::Player)
        .unwrap();
    p.next().unwrap();
    let fx = p.back().unwrap();
    assert_eq!(loads(&fx), 1);
    assert_eq!(p.phase(), StepPhase::Passed);
    assert!(p.passed(1));
    assert_eq!(at(0).back().unwrap_err().code(), "out_of_range");
}

#[test]
fn skipping_a_step_records_nothing() {
    let mut p = at(1);
    let fx = p.next().unwrap();
    assert!(record(&fx).is_none());
    assert!(!p.passed(1));
}

#[test]
fn resume_sends_the_view_without_a_plan() {
    let files = common::test_files(&[("play", &lesson())]);
    let pack = Pack::from_files(&files).unwrap();
    let bundle = pack.bundle(&"play".parse().unwrap()).unwrap();
    let (p, fx) = Player::resume(bundle.clone(), 4).unwrap();
    assert_eq!(fx.len(), 1);
    assert!(matches!(fx[0], Effect::View(_)));
    let r = p.lesson_ref();
    assert_eq!(
        (r.step, r.lesson.as_str(), r.version.to_string()),
        (4, "play", "1.0.0".to_owned())
    );
    assert_eq!(r.content, bundle.content);
    let (_, fx) = Player::resume(bundle.clone(), 1).unwrap();
    assert_eq!(loads(&fx), 0, "the session restored the patch");
    assert_eq!(
        Player::resume(bundle, 5).unwrap_err().code(),
        "out_of_range"
    );
}

#[test]
fn touched_counts_player_edits_only() {
    let json = lesson().replace(
        r#"{"kind":"value","knob":"filter1.cutoff","cmp":"<","value":"1kHz"}"#,
        r#"{"kind":"touched","knob":"filter1.cutoff","min_span_pct":20}"#,
    );
    let files = common::test_files(&[("play", &json)]);
    let pack = Pack::from_files(&files).unwrap();
    let mut p = Player::new(pack.bundle(&"play".parse().unwrap()).unwrap());
    p.start().unwrap();
    p.next().unwrap();
    let saw = patch("saw.patch.json");
    let mut moved = saw.clone();
    cutoff(200.0).apply(&mut moved).unwrap();
    assert!(
        p.edited(0, 1, &moved, &[cutoff(200.0)], EditSource::Host)
            .unwrap()
            .is_empty()
    );
    assert!(
        p.edited(0, 2, &saw, &[cutoff(2000.0)], EditSource::Player)
            .unwrap()
            .is_empty()
    );
    let fx = p
        .edited(0, 3, &moved, &[cutoff(200.0)], EditSource::Player)
        .unwrap();
    assert!(
        passed(&fx).is_some(),
        "2 kHz down to 200 Hz is well over 20 % of the cutoff's range"
    );
}
