//! Lane PLY's checks on the example pack, as `lesson-check` (lane LCK) will run them: every step's text lints clean
//! and its `enth:` links resolve, the Do step's sound check fails at the start and passes at the witness with a margin,
//! and the ear question's options pass the audibility gate. Also: the markdown parse never panics (proptest).

mod common;

use enth_learn::check::{
    MIN_JND, SoundOptions, auditions, choice_audibility, evaluate_sounds, evaluate_sounds_report,
};
use enth_learn::markdown::{self, STEP_WORD_BUDGET};
use enth_learn::{Pack, StepKind};
use proptest::prelude::*;

fn example() -> enth_learn::LessonBundle {
    let pack = Pack::from_files(&common::example_files()).unwrap();
    pack.bundle(&"cutoff".parse().unwrap()).unwrap()
}

#[test]
fn every_text_of_the_example_lints_clean_and_resolves() {
    let bundle = example();
    let mut start = None;
    for step in &bundle.lesson.steps {
        if let Some(r) = &step.view.start {
            start = bundle.patch(r).cloned();
        }
        let Some(text) = step.text.as_ref().and_then(|t| bundle.text(t)) else {
            continue;
        };
        let issues = markdown::lint(text, STEP_WORD_BUDGET);
        assert!(issues.is_empty(), "{}: {issues:?}", step.id);
        let examples = match &step.kind {
            StepKind::Listen { examples }
            | StepKind::Choose {
                options: examples, ..
            } => examples.len(),
            _ => 0,
        };
        let unresolved = markdown::parse(text).resolve(start.as_deref(), examples);
        assert!(unresolved.is_empty(), "{}: {unresolved:?}", step.id);
    }
}

#[test]
fn the_sound_check_fails_at_the_start_and_passes_at_the_witness() {
    let bundle = example();
    let step = bundle
        .lesson
        .steps
        .iter()
        .find(|s| s.id.as_str() == "try")
        .unwrap();
    let StepKind::Do { check, witness } = &step.kind else {
        panic!("a do step")
    };
    let sounds = check.sounds();
    let start = bundle.patch(step.view.start.as_ref().unwrap()).unwrap();
    let witness = bundle.patch(witness).unwrap();
    let before = evaluate_sounds(&sounds, start).unwrap();
    let after = evaluate_sounds(&sounds, witness).unwrap();
    assert!(!before.passed, "{before:?}");
    assert!(after.passed, "{after:?}");
    // lesson-check's margin rule: at least 1e-3 either side, and no thin item.
    assert!(
        before
            .items
            .iter()
            .all(|i| i.margin.is_some_and(|m| m <= -1e-3))
    );
    assert!(
        after
            .items
            .iter()
            .all(|i| i.margin.is_some_and(|m| m >= 1e-3))
    );
    let report = evaluate_sounds_report(&sounds, witness, SoundOptions { nuisance: true }).unwrap();
    assert!(report.iter().all(|r| r.report.thin == 0), "{report:?}");
}

#[test]
fn the_ear_question_passes_the_audibility_gate() {
    let bundle = example();
    let step = bundle
        .lesson
        .steps
        .iter()
        .find(|s| s.id.as_str() == "which")
        .unwrap();
    let StepKind::Choose { options, .. } = &step.kind else {
        panic!("a choose step")
    };
    let audibility = choice_audibility(&auditions(&bundle, options)).unwrap();
    assert!(audibility.passes(MIN_JND), "{audibility:?}");
}

proptest! {
    /// Any markdown-ish text: an AST and findings, never a panic.
    #[test]
    fn markdown_never_panics(text in "[-#*_`\\[\\]()<>!:/.a-z0-9 \n>|~&;\\\\]{0,120}") {
        let parsed = markdown::parse(&text);
        let _ = parsed.resolve(None, 2);
        let _ = markdown::lint(&text, STEP_WORD_BUDGET);
        prop_assert!(parsed.doc.words() <= text.split_whitespace().count() + 1);
    }

    /// Fenced blocks and links built from the grammar's own pieces: never a panic.
    #[test]
    fn grammar_shaped_markdown_never_panics(
        target in prop::sample::select(vec!["enth:filter1.cutoff", "enth:play/2", "enth:play/0", "enth:lfo1->osc1.semi", "enth:", "https://", "https://x.org", "http://x", "mailto:a@b"]),
        widget in prop::sample::select(vec!["play 1", "play", "control filter1.*", "scope", "scope now", "shape env2", "shape", "knob"]),
        fence in prop::sample::select(vec!["```enth", "```", "~~~enth", "    "]),
    ) {
        let text = format!("# T\n\n[x]({target}) and ![i]({target})\n\n{fence}\n{widget}\n```\n");
        let parsed = markdown::parse(&text);
        let _ = parsed.resolve(Some(&enth_patch::Patch::init()), 1);
    }
}
