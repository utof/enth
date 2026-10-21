//! Lane PLY: the player's properties under random intents (docs/learn.md, "Tests"; phase-3 acceptance). A harness
//! plays the session's part (it applies `LoadPatch` and `Apply`, refuses edits that touch a lock, counts revisions)
//! and drives a lesson of every step kind with random edits, ticks, results (right, stale, failed), hints, Show,
//! picks, round ends, Next and Back, ends and resumes. After every call:
//!
//! 1. **no panic**, and a refused call leaves the player usable;
//! 2. **no pass without a passing result for the current revision**: a Do step with a sound check passes only on
//!    `check_done` with a passing verdict for the check out for the latest revision; a stale or unasked result is
//!    refused (`stale_result`); a patch-only Do step passes only on an edit whose patch satisfies every predicate
//!    (and does pass on one); a Choose step on the right pick or Show; a Recreate step on a passing round; a Read or
//!    Listen step on Next; nothing else passes;
//! 3. **progress never shrinks**: each `Record` comes with its `Passed`, merging it never lowers a step's `done`,
//!    `clean` or `best`, and a passed step stays passed;
//! 4. **locks hold**: Show's op never touches a control the step's effective view locks, a step load sends at most
//!    one plan, and a Recreate step's view carries its challenge's locks.
//!
//! Fixed seed and 256 cases in the gate (`PROPTEST_RNG_SEED`), 10,000 nightly.

mod common;

use enth_learn::check::{Check, Knob, TouchLog};
use enth_learn::lesson::touches;
use enth_learn::player::{CheckVerdict, MeasuredHint};
use enth_learn::{
    EditSource, Effect, EndChoice, LessonBundle, Pack, Player, Progress, RoundEnd, StepKind,
    StepPhase, StepView,
};
use enth_patch::{Op, ParamAddr, Patch};
use proptest::prelude::*;

const DRAW: &str = r#"{"schema":"enth.challenge/2","target":{"kind":"draw","seed":7,"items":[{"param":"filter1.cutoff","set":[300.0,1200.0,"4kHz"]}]},"answer":"patch","phrase":1,"start":{"kind":"template","template":0},"constraints":{"editable":["filter1.cutoff"],"locked":["osc1.*","filter1.resonance"]},"assists":{"live_meter":false,"dim_unused":false,"hint_budget":null},"scoring":{"sound":"sound/1","structure":"structure/1","combined":"combined/1","time":"time/1","sound_weight":50,"pass_mark":90},"meta":{"title":"","tags":[]}}"#;

/// Every kind of step, and every kind of check (patch, touched, sound, all).
fn lesson() -> String {
    format!(
        r#"{{"schema":"enth.lesson/1","id":"props","title":"Props","tier":"basic","objective":"Every step kind.","teaches":["svf.cutoff"],
        "steps":[
          {{"id":"intro","kind":"read","text":"text.md"}},
          {{"id":"hear","kind":"listen","examples":[{{"label":"Open","patch":"patches/saw.patch.json"}},{{"label":"Closed","patch":"patches/dark.patch.json"}}]}},
          {{"id":"value","kind":"do","hints":["Left."],"view":{{"start":"patches/saw.patch.json","editable":["filter1.cutoff"],"locked":["osc1.*"]}},
            "check":{{"kind":"patch","predicates":["filter1.cutoff < 1kHz","master.level <= 0dB"]}},"witness":"patches/dark.patch.json"}},
          {{"id":"turn","kind":"do","view":{{"start":"patches/saw.patch.json","locked":["osc1.*","master.*"]}},
            "check":{{"kind":"patch","predicates":["touched filter1.resonance >= 20%"]}},"witness":"patches/dark.patch.json"}},
          {{"id":"sound","kind":"do","hints":["Darker."],"view":{{"start":"patches/saw.patch.json","locked":["osc1.*"]}},
            "check":{{"kind":"sound","phrase":1,"expect":["brightness<500Hz"]}},"witness":"patches/dark.patch.json"}},
          {{"id":"both","kind":"do","view":{{"locked":["osc1.wave"]}},
            "check":{{"kind":"all","checks":[{{"kind":"patch","predicates":["filter1.cutoff < 1kHz"]}},{{"kind":"sound","phrase":1,"expect":["brightness<500Hz"]}}]}},
            "witness":"patches/dark.patch.json"}},
          {{"id":"which","kind":"choose","options":[{{"label":"A","patch":"patches/saw.patch.json"}},{{"label":"B","patch":"patches/dark.patch.json"}}],"answer":1}},
          {{"id":"round","kind":"recreate","challenge":{DRAW}}}
        ]}}"#
    )
}

fn bundle() -> LessonBundle {
    let files = common::test_files(&[("props", &lesson())]);
    Pack::from_files(&files)
        .unwrap()
        .bundle(&"props".parse().unwrap())
        .unwrap()
}

/// The controls random edits turn: two editable ones, a locked oscillator, the master level and a macro.
const KNOBS: [&str; 6] = [
    "filter1.cutoff",
    "filter1.resonance",
    "osc1.semi",
    "osc1.wave",
    "master.level",
    "macro1",
];

#[derive(Clone, Debug)]
enum Rev {
    Current,
    Older,
    Other(u64),
}

#[derive(Clone, Debug)]
enum Intent {
    Edit {
        source: u8,
        edits: Vec<(usize, f32)>,
    },
    Tick(u64),
    CheckDone(Rev, bool),
    Hint,
    HintDone(Rev),
    Show,
    Choose(u8),
    RoundEnded(bool, bool, Option<u8>),
    Next,
    Back,
    End(bool),
    Resume,
}

fn rev() -> impl Strategy<Value = Rev> {
    prop_oneof![
        3 => Just(Rev::Current),
        1 => Just(Rev::Older),
        1 => (0u64..50).prop_map(Rev::Other),
    ]
}

fn intent() -> impl Strategy<Value = Intent> {
    prop_oneof![
        6 => (0u8..3, prop::collection::vec((0usize..KNOBS.len(), 0.0f32..=1.0), 1..3))
            .prop_map(|(source, edits)| Intent::Edit { source, edits }),
        5 => (0u64..800).prop_map(Intent::Tick),
        4 => (rev(), any::<bool>()).prop_map(|(r, p)| Intent::CheckDone(r, p)),
        1 => Just(Intent::Hint),
        1 => rev().prop_map(Intent::HintDone),
        2 => Just(Intent::Show),
        2 => (0u8..4).prop_map(Intent::Choose),
        1 => (any::<bool>(), any::<bool>(), prop::option::of(0u8..=100))
            .prop_map(|(p, a, b)| Intent::RoundEnded(p, a, b)),
        3 => Just(Intent::Next),
        1 => Just(Intent::Back),
        1 => any::<bool>().prop_map(Intent::End),
        1 => Just(Intent::Resume),
    ]
}

/// The session's side, as much of it as the properties need.
struct Harness {
    bundle: LessonBundle,
    player: Player,
    patch: Patch,
    revision: u64,
    now: u64,
    view: StepView,
    /// The revision of the sound check now out (the player's `Checking`), as the harness saw it asked.
    check_out: Option<u64>,
    hint_out: Option<u64>,
    touched: TouchLog,
    progress: Progress,
    ended: bool,
}

impl Harness {
    fn new() -> (Harness, Vec<Effect>) {
        let bundle = bundle();
        let mut player = Player::new(bundle.clone());
        let fx = player.start().unwrap();
        let mut h = Harness {
            bundle,
            player,
            patch: Patch::init(),
            revision: 0,
            now: 0,
            view: StepView::default(),
            check_out: None,
            hint_out: None,
            touched: TouchLog::default(),
            progress: Progress::new(),
            ended: false,
        };
        h.absorb(&fx, "start", None).unwrap();
        (h, fx)
    }

    fn kind(&self) -> StepKind {
        self.player.current().kind.clone()
    }

    /// A patch edit delivered to the player (the session's revision rises by one).
    fn deliver(&mut self, ops: &[Op], source: EditSource) -> Result<Vec<Effect>, TestCaseError> {
        self.revision += 1;
        self.now += 1;
        let fx = self
            .player
            .edited(self.now, self.revision, &self.patch, ops, source)
            .map_err(|e| TestCaseError::fail(format!("edited refused: {e}")))?;
        self.check_out = None;
        if source == EditSource::Player {
            for op in ops {
                let knob = match op {
                    Op::SetParam { addr, .. } => Knob::Param(addr.clone()),
                    Op::SetMacro { index, .. } => Knob::Macro(*index),
                    _ => continue,
                };
                if let (Some(spec), Some(v)) = (knob.spec(&self.patch), knob.value(&self.patch)) {
                    self.touched.note(knob, spec, v);
                }
            }
        }
        // Property 2 for patch-only Do steps: a pass exactly when the predicates hold (and it had not passed).
        if let StepKind::Do { check, .. } = self.kind()
            && check.sounds().is_empty()
        {
            let holds = check.patch_holds(&self.patch, &self.touched);
            let passed_now = fx.iter().any(|e| matches!(e, Effect::Passed { .. }));
            prop_assert!(
                !passed_now || holds,
                "a patch-only step passed on a patch that fails it"
            );
            let was_passed = self.player.phase() == StepPhase::Passed && !passed_now;
            prop_assert!(
                !holds || passed_now || was_passed,
                "a satisfied patch-only step did not pass"
            );
        } else {
            prop_assert!(
                !fx.iter().any(|e| matches!(e, Effect::Passed { .. })),
                "an edit passed a step that is not a patch-only Do step"
            );
        }
        self.absorb(&fx, "edited", None)?;
        Ok(fx)
    }

    /// Check the effects of a call and act on them as the session does.
    fn absorb(
        &mut self,
        fx: &[Effect],
        call: &str,
        step_before: Option<&(usize, StepKind)>,
    ) -> Result<(), TestCaseError> {
        let loads = fx
            .iter()
            .filter(|e| matches!(e, Effect::LoadPatch(_) | Effect::StartRound(_)))
            .count();
        prop_assert!(loads <= 1, "{call}: a step load sends at most one plan");
        let passes: Vec<u16> = fx
            .iter()
            .filter_map(|e| match e {
                Effect::Passed { step, .. } => Some(*step),
                _ => None,
            })
            .collect();
        let records = fx.iter().filter(|e| matches!(e, Effect::Record(_))).count();
        prop_assert_eq!(
            passes.len(),
            records,
            "{}: every Record comes with its Passed",
            call
        );
        prop_assert!(passes.len() <= 1, "{call}: at most one pass per call");
        if let (Some(step), Some((i, kind))) = (passes.first(), step_before) {
            prop_assert_eq!(
                usize::from(*step),
                *i,
                "{}: the pass is the current step's",
                call
            );
            let allowed = matches!(
                (call, kind),
                ("next", StepKind::Read | StepKind::Listen { .. })
                    | ("choose" | "show", StepKind::Choose { .. })
                    | ("round_ended", StepKind::Recreate { .. })
                    | ("check_done", StepKind::Do { .. })
            );
            prop_assert!(allowed, "{call} passed a {:?} step", kind);
        }
        // Progress never shrinks.
        let before = self.progress.clone();
        for e in fx {
            if let Effect::Record(r) = e {
                prop_assert!(r.progress.done, "a record is a done step");
                self.progress.record(r);
            }
        }
        for (key, lesson) in &before.lessons {
            for (id, s) in &lesson.steps {
                let now = self.progress.step(key, id).cloned().unwrap_or_default();
                prop_assert!(
                    now.done >= s.done && now.clean >= s.clean && now.best >= s.best,
                    "progress shrank"
                );
            }
        }
        for e in fx {
            match e {
                Effect::View(v) => {
                    self.view = v.clone();
                    if let StepKind::Recreate { challenge, .. } = self.kind() {
                        prop_assert_eq!(
                            &v.locked,
                            &challenge.constraints.locked,
                            "a round's view carries its locks"
                        );
                    }
                }
                Effect::Check(req) => {
                    prop_assert_eq!(
                        req.revision,
                        self.revision,
                        "a check is for the latest revision"
                    );
                    prop_assert_eq!(req.patch.as_ref(), &self.patch);
                    let StepKind::Do { check, .. } = self.kind() else {
                        return Err(TestCaseError::fail(
                            "a check asked on a step that is not a Do step",
                        ));
                    };
                    prop_assert!(
                        check.patch_holds(&req.patch, &self.touched),
                        "a sound check asked while the step's patch part fails"
                    );
                    prop_assert_eq!(&req.sounds, &check.sounds());
                    self.check_out = Some(req.revision);
                }
                Effect::HintJob(req) => self.hint_out = Some(req.revision),
                Effect::Apply(op) => {
                    // Locks hold: Show never asks for an op the session would refuse.
                    let t = touches(&self.patch, op).unwrap_or_default();
                    prop_assert!(
                        !t.iter().any(|t| self.view.locks(t)),
                        "Show's op {op:?} touches a lock"
                    );
                }
                Effect::End(_) => self.ended = true,
                _ => {}
            }
        }
        // The session acts on the plan and Show's op after checking: a load, then the op as one undo step.
        for e in fx {
            match e {
                Effect::LoadPatch(p) => {
                    self.patch = Patch::clone(p);
                    self.touched.clear();
                    self.deliver(&[], EditSource::Load)?;
                }
                Effect::StartRound(c) => {
                    self.patch = c.start_patch().unwrap_or_else(|_| Patch::init());
                    self.touched.clear();
                    self.deliver(&[], EditSource::Load)?;
                }
                Effect::Apply(op) if op.apply(&mut self.patch).is_ok() => {
                    self.deliver(std::slice::from_ref(op), EditSource::Player)?;
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn rev_of(&self, r: &Rev) -> u64 {
        match r {
            Rev::Current => self.revision,
            Rev::Older => self.revision.saturating_sub(1),
            Rev::Other(n) => *n,
        }
    }

    fn step(&mut self, intent: &Intent) -> Result<(), TestCaseError> {
        let before = (usize::from(self.player.step()), self.kind());
        let passed_before: Vec<bool> = (0..self.player.steps().len())
            .map(|i| self.player.passed(i))
            .collect();
        let (call, result) = match intent {
            Intent::Edit { source, edits } => {
                let source =
                    [EditSource::Player, EditSource::Host, EditSource::Load][usize::from(*source)];
                let mut ops = Vec::new();
                for (k, n) in edits {
                    let knob: Knob = KNOBS[*k].parse().unwrap();
                    let op = match &knob {
                        Knob::Param(a) => {
                            let Some(spec) = self.patch.spec(a) else {
                                continue;
                            };
                            Op::SetParam {
                                addr: a.clone(),
                                value: spec.from_norm(*n),
                            }
                        }
                        Knob::Macro(i) => Op::SetMacro {
                            index: *i,
                            value: *n,
                        },
                    };
                    // The session refuses a player's edit that touches a lock, before anything changes.
                    let t = touches(&self.patch, &op).unwrap_or_default();
                    if source == EditSource::Player && t.iter().any(|t| self.view.locks(t)) {
                        continue;
                    }
                    let mut p = self.patch.clone();
                    if op.apply(&mut p).is_ok() {
                        self.patch = p;
                        ops.push(op);
                    }
                }
                if ops.is_empty() || self.ended {
                    return Ok(());
                }
                self.deliver(&ops, source)?;
                return self.after(&before, &passed_before, "edited");
            }
            Intent::Tick(dt) => {
                self.now += dt;
                ("tick", self.player.tick(self.now))
            }
            Intent::CheckDone(r, pass) => {
                let revision = self.rev_of(r);
                let verdict = CheckVerdict {
                    passed: *pass,
                    items: Vec::new(),
                };
                let expected_ok = !self.ended && self.check_out == Some(revision);
                let result = self.player.check_done(revision, &verdict);
                prop_assert_eq!(
                    result.is_ok(),
                    expected_ok,
                    "check_done({}) with {:?} out",
                    revision,
                    self.check_out
                );
                if let Ok(fx) = &result {
                    let passed_now = fx.iter().any(|e| matches!(e, Effect::Passed { .. }));
                    prop_assert_eq!(
                        passed_now,
                        *pass,
                        "a check passes exactly on a passing verdict"
                    );
                    prop_assert_eq!(
                        revision,
                        self.revision,
                        "only the current revision's result counts"
                    );
                    self.check_out = None;
                } else if let Err(e) = &result
                    && !self.ended
                {
                    prop_assert_eq!(e.code(), "stale_result");
                }
                ("check_done", result)
            }
            Intent::Hint => ("hint", self.player.hint()),
            Intent::HintDone(r) => {
                let revision = self.rev_of(r);
                let hint = MeasuredHint {
                    text: "lower".to_owned(),
                    highlight: Vec::new(),
                };
                let expected_ok =
                    !self.ended && self.hint_out == Some(revision) && revision == self.revision;
                let result = self.player.hint_done(revision, hint);
                prop_assert_eq!(result.is_ok(), expected_ok, "hint_done({})", revision);
                if result.is_ok() {
                    self.hint_out = None;
                }
                ("hint_done", result)
            }
            Intent::Show => {
                let patch = self.patch.clone();
                ("show", self.player.show(&patch))
            }
            Intent::Choose(i) => ("choose", self.player.choose(*i)),
            Intent::RoundEnded(passed, assisted, best) => (
                "round_ended",
                self.player.round_ended(RoundEnd {
                    passed: *passed,
                    assisted: *assisted,
                    best_sound_pct: *best,
                }),
            ),
            Intent::Next => ("next", self.player.next()),
            Intent::Back => ("back", self.player.back()),
            Intent::End(restore) => (
                "end",
                self.player.end(if *restore {
                    EndChoice::Restore
                } else {
                    EndChoice::Keep
                }),
            ),
            Intent::Resume => {
                if self.ended {
                    return Ok(());
                }
                // A save and a reload mid-lesson: the session keeps the patch and the round, the player is new.
                let (player, fx) = Player::resume(self.bundle.clone(), self.player.step())
                    .map_err(|e| TestCaseError::fail(format!("resume refused: {e}")))?;
                prop_assert!(
                    !fx.iter()
                        .any(|e| matches!(e, Effect::LoadPatch(_) | Effect::StartRound(_)))
                );
                self.player = player;
                self.check_out = None;
                self.hint_out = None;
                self.touched.clear();
                self.absorb(&fx, "resume", Some(&before))?;
                return Ok(());
            }
        };
        match result {
            Ok(fx) => {
                if self.player.step() as usize != before.0 || call == "back" {
                    self.check_out = None;
                    self.hint_out = None;
                    self.touched.clear();
                }
                self.absorb(&fx, call, Some(&before))?;
                self.after(&before, &passed_before, call)
            }
            Err(e) => {
                // A refusal changes nothing a property watches; after the end, every call is refused.
                prop_assert!(!e.code().is_empty() && !e.hint().is_empty());
                if self.ended {
                    prop_assert_eq!(e.code(), "lesson_ended");
                }
                Ok(())
            }
        }
    }

    /// After a call on the same player: a passed step stays passed.
    fn after(
        &self,
        _before: &(usize, StepKind),
        passed_before: &[bool],
        call: &str,
    ) -> Result<(), TestCaseError> {
        for (i, was) in passed_before.iter().enumerate() {
            prop_assert!(!was || self.player.passed(i), "{call}: step {i} un-passed");
        }
        Ok(())
    }
}

proptest! {
    #[test]
    fn the_player_keeps_its_properties(intents in prop::collection::vec(intent(), 1..60)) {
        let (mut h, _) = Harness::new();
        for intent in &intents {
            h.step(intent)?;
        }
    }

    /// The same, starting at a random step (as a resumed lesson does), so the later steps get as many intents.
    #[test]
    fn from_any_step(start in 0u16..8, intents in prop::collection::vec(intent(), 1..40)) {
        let (mut h, _) = Harness::new();
        while h.player.step() < start && !h.player.is_finished() {
            h.step(&Intent::Next)?;
        }
        for intent in &intents {
            h.step(intent)?;
        }
    }
}

/// A check that names both a patch and a sound part is asked only while its patch part holds, so a sound verdict
/// can never pass a step whose patch predicates fail (property 2 for `all`).
#[test]
fn an_all_check_asks_for_sound_only_while_its_patch_part_holds() {
    let (mut h, _) = Harness::new();
    while h.player.current().id.as_str() != "both" {
        h.step(&Intent::Next).unwrap();
    }
    let StepKind::Do {
        check: Check::All { .. },
        ..
    } = h.kind()
    else {
        panic!("the all step")
    };
    // The carried-over patch has a 2 kHz cutoff: the patch part fails, so no check is asked.
    h.step(&Intent::Edit {
        source: 0,
        edits: vec![(4, 0.5)],
    })
    .unwrap();
    h.step(&Intent::Tick(1000)).unwrap();
    assert_eq!(h.check_out, None);
    let addr: ParamAddr = "filter1.cutoff".parse().unwrap();
    let low = h.patch.spec(&addr).unwrap().to_norm(500.0);
    h.step(&Intent::Edit {
        source: 0,
        edits: vec![(0, low)],
    })
    .unwrap();
    h.step(&Intent::Tick(1000)).unwrap();
    assert_eq!(h.check_out, Some(h.revision));
}
