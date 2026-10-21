//! **Owner: lane LUI** (the Learn view, feature `learn`). Pre-declared by lane S0l (R2); **lane SKL** wrote the
//! minimal seam for freeze point F2l, which LUI replaces or grows.
//!
//! The editor's Learn state and the seam to the shell (docs/learn.md, "The Learn view"): [`LearnIntent`] (what the
//! view asks for: open the panel, start a lesson, next, back, hint, show me, choose, listen, end),
//! `Editor::set_lesson` (the session's `LearnEdit::lesson()` read model, handed over before each frame, as
//! `set_round` hands over a round), `Editor::set_lessons` (the shell's installed lessons, handed over when the window
//! opens) and `Editor::apply_learn` (the intents applied to the session's `LearnEdit` after the frame). It mirrors
//! `recreate/state.rs`. The view never reads a file: the shell lists the lessons ([`LessonBundle`]s, each with its
//! texts and patches), and a start builds `enth_learn::Player` from one.
//!
//! [`LessonOverlay`] lays the step's view over the cards: what it highlights gets the hint outline (fu1-hint's
//! [`Pointer::Here`]), what it locks is dimmed (the session refuses edits to it; the status line says so), what it
//! hides is not drawn. Lane LUI adds the overlay's own `Highlighted` emphasis and lock glyph.

use enth_learn::{LessonBundle, Player, Step, StepView, Touch};
use enth_patch::{ModuleId, ParamAddr, Target};
use enth_session::game::EndChoice;
use enth_session::learn::{LearnEdit, LearnView};

use crate::editor::overlay::{Emphasis, Overlay, ParamOverlay, Pointer};

use super::HeaderLearn;

/// What the Learn view asks for. The editor queues them while drawing and applies them after the frame
/// ([`Learn::apply`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LearnIntent {
    /// Show or hide the Learn panel (the header's **Learn**).
    Toggle,
    /// Start the lesson at this index of the shell's list.
    Start(usize),
    /// The next step (or Finish).
    Next,
    /// The previous step.
    Back,
    /// The next hint.
    Hint,
    /// Show me.
    Show,
    /// Answer the ear question with this option.
    Choose(u8),
    /// Play an example (`None`: stop).
    Listen(Option<u8>),
    /// End the lesson.
    End(EndChoice),
}

/// The editor's Learn state (see the [module docs](self)).
#[derive(Debug, Default)]
pub struct Learn {
    lessons: Vec<LessonBundle>,
    problems: Vec<String>,
    open: bool,
    view: Option<LearnView>,
    overlay: Option<LessonOverlay>,
    queued: Vec<LearnIntent>,
    /// The last refusal, for the panel.
    said: Option<String>,
}

impl Learn {
    /// The lessons the shell found (every lesson of every installed pack, in order) and the packs it refused, one
    /// line each. Handed over when the window opens (the view rescans then).
    pub fn set_lessons(&mut self, lessons: Vec<LessonBundle>, problems: Vec<String>) {
        self.lessons = lessons;
        self.problems = problems;
    }

    /// The session's lesson, before each frame. A lesson that appears (started, or resumed after a reload) opens
    /// the panel.
    pub fn set_lesson(&mut self, view: Option<LearnView>) {
        if view.is_some() && self.view.is_none() {
            self.open = true;
        }
        self.overlay = view.as_ref().filter(|v| !v.round).map(|v| LessonOverlay {
            view: v.view.clone(),
        });
        self.view = view;
    }

    /// What the header's hook shows.
    pub fn header(&self) -> HeaderLearn {
        HeaderLearn {
            available: !self.lessons.is_empty() || self.view.is_some(),
        }
    }

    /// Whether the panel takes its slot this frame.
    pub fn panel_open(&self) -> bool {
        self.open
    }

    /// The lesson's read model, if one is live.
    pub fn view(&self) -> Option<&LearnView> {
        self.view.as_ref()
    }

    /// The shell's lessons.
    pub fn lessons(&self) -> &[LessonBundle] {
        &self.lessons
    }

    /// The packs the shell refused.
    pub fn problems(&self) -> &[String] {
        &self.problems
    }

    /// The last refusal, if the latest intent was refused.
    pub fn said(&self) -> Option<&str> {
        self.said.as_deref()
    }

    /// The live lesson's bundle and current step, when the shell's list has it.
    pub fn step(&self) -> Option<(&LessonBundle, usize, &Step)> {
        let r = self.view.as_ref()?.lesson.as_ref()?;
        let bundle = self
            .lessons
            .iter()
            .find(|b| b.pack == r.pack && b.lesson.id == r.lesson)?;
        let i = usize::from(r.step);
        Some((bundle, i, bundle.lesson.steps.get(i)?))
    }

    /// The step's overlay, while a lesson step (not its round) is live.
    pub fn overlay(&self) -> Option<&LessonOverlay> {
        self.overlay.as_ref()
    }

    /// Queue an intent for [`Learn::apply`].
    pub fn push(&mut self, intent: LearnIntent) {
        self.queued.push(intent);
    }

    /// Whether intents wait for [`Learn::apply`] (the editor asks for one more frame to show their effect).
    pub fn has_queued(&self) -> bool {
        !self.queued.is_empty()
    }

    /// After the frame: the queued intents, in order, on the session. A refusal is kept for the panel (the session
    /// also keeps its last error in the read model).
    pub fn apply(&mut self, session: &mut dyn LearnEdit) {
        for intent in std::mem::take(&mut self.queued) {
            let result = match intent {
                LearnIntent::Toggle => {
                    self.open = !self.open;
                    Ok(())
                }
                LearnIntent::Start(i) => match self.lessons.get(i) {
                    Some(bundle) => session.learn_start(Box::new(Player::new(bundle.clone()))),
                    None => Ok(()),
                },
                LearnIntent::Next => session.learn_next(),
                LearnIntent::Back => session.learn_back(),
                LearnIntent::Hint => session.learn_hint(),
                LearnIntent::Show => session.learn_show(),
                LearnIntent::Choose(o) => session.learn_choose(o),
                LearnIntent::Listen(e) => session.learn_listen(e),
                LearnIntent::End(choice) => session.learn_end(choice),
            };
            self.said = result.err().map(|e| format!("{e}: {}", e.hint()));
        }
    }
}

/// A lesson step's view laid over the cards (see the [module docs](self)).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LessonOverlay {
    view: StepView,
}

impl LessonOverlay {
    /// The overlay of a step's view.
    pub fn new(view: StepView) -> Self {
        LessonOverlay { view }
    }
}

impl Overlay for LessonOverlay {
    fn param(&self, addr: &ParamAddr) -> ParamOverlay {
        let touch = Touch::Param(addr.clone());
        let emphasis = if self.view.hide.iter().any(|s| s.covers(&touch)) {
            Emphasis::Hidden
        } else if self.view.locks(&touch) {
            Emphasis::Dimmed
        } else {
            Emphasis::Normal
        };
        ParamOverlay {
            emphasis,
            score: None,
            hint: self
                .view
                .highlight
                .iter()
                .any(|s| s.covers(&touch))
                .then_some(Pointer::Here),
        }
    }

    fn module(&self, id: &ModuleId) -> Emphasis {
        let touch = Touch::Module(Target::Module(id.clone()));
        if self.view.hide.iter().any(|s| s.covers(&touch)) {
            Emphasis::Hidden
        } else if self.view.locks(&touch) {
            Emphasis::Dimmed
        } else {
            Emphasis::Normal
        }
    }

    fn module_hint(&self, id: &ModuleId) -> bool {
        let touch = Touch::Module(Target::Module(id.clone()));
        self.view.highlight.iter().any(|s| s.covers(&touch))
    }
}
