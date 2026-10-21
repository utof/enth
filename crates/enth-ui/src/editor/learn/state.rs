//! **Owner: lane LUI** (the Learn view, feature `learn`). Pre-declared by lane S0l (R2); lane SKL wrote the minimal
//! seam for freeze point F2l, which LUI grew into the Learn view's state.
//!
//! The editor's Learn state and the seam to the shell (docs/learn.md, "The Learn view"; docs/editor.md, "The Learn
//! view"): [`LearnIntent`] (what the view asks for), `Editor::set_lesson` (the session's `LearnEdit::lesson()` read
//! model, handed over before each frame, as `set_round` hands over a round), `Editor::set_library` (the shell's
//! installed packs and the progress file, handed over when the window opens; SKL's `set_lessons` still works) and
//! `Editor::apply_learn` (the intents applied to the session's `LearnEdit` after the frame). It mirrors
//! `recreate/state.rs`. The view never reads a file: the shell lists the lessons ([`LessonBundle`]s, each with its
//! texts and patches), and a start builds `enth_learn::Player` from one.
//!
//! [`LessonOverlay`] lays the step's view over the cards: what it highlights is outlined
//! ([`Emphasis::Highlighted`]), what it locks is dimmed with a lock glyph (the session refuses edits to it; hovering
//! says why), what it hides is not drawn, and a **focus** step (`layout = "focus"`) hides every module it neither
//! highlights nor leaves editable, so a preset-sized patch fits beside the pane. Hovering an `enth:` link in the
//! step's text outlines its control too ([`Learn::set_pointing`]). In a lesson's Recreate step the round's overlay
//! (rings, its hint, the dim assist) is laid over this one ([`crate::editor::overlay::Layered`]).
//!
//! **Progress.** The shell hands over the progress file when the window opens; while it is open, the view marks a
//! step done when it sees it pass (or Next leave a Read or Listen step), so the browser's counts follow the lesson.
//! The file stays the shell's: the session's `LearnRequest::Record` is what writes it.
//!
//! **Review** (lane REV). With the day from the shell (`Editor::set_review_day`), the view keeps
//! `enth_learn::review::queue` of its progress and lessons ([`Learn::review`]); the browser's Review shows how many
//! skills are due and plays the queue's head item through `LearnEdit::review_start`. The shell writes the round's
//! rings into the progress file and hands the file back (`Editor::set_progress`), so the next press plays the next
//! item.

use enth_learn::lesson::Layout as StepLayout;
use enth_learn::pack::LessonKey;
use enth_learn::player::StepRecord;
use enth_learn::progress::Progress;
use enth_learn::review::{self, Day, Queue};
use enth_learn::store::Library;
use enth_learn::{
    LessonBundle, PackId, Player, Selector, Step, StepKind, StepPhase, StepProgress, StepView,
    Touch,
};
use enth_patch::{ModSource, ModuleId, ParamAddr, Target};
use enth_session::game::EndChoice;
use enth_session::learn::{LearnEdit, LearnView};

use crate::editor::overlay::{Emphasis, Overlay, ParamOverlay};

use super::HeaderLearn;

/// What the Learn view asks for. The editor queues them while drawing and applies them after the frame
/// ([`Learn::apply`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LearnIntent {
    /// Show or hide the lesson pane (the header's **Learn**, the pane's **Hide**): hidden, a live lesson keeps going
    /// on the full editor, and the header's entry shows its step.
    Toggle,
    /// Show the lesson browser over a live lesson (`true`), or go back to the step (`false`).
    Browse(bool),
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
    /// A/B: play the other of the last two examples played, at the same position (they share the Target ▶ slot).
    Flip,
    /// Play the Review queue's next item (lane REV): the weakest due skill's seeded sibling round
    /// (`LearnEdit::review_start`). The queue follows the progress file, so the next press plays the next item.
    Review,
    /// The lesson browser's shelf: a track, or every pack.
    Shelf(Shelf),
    /// End the lesson.
    End(EndChoice),
}

/// The lesson browser's tabs: a track (the tour first), or every pack's lessons.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shelf {
    /// The track at this index of [`Learn::tracks`].
    Track(usize),
    /// Every installed pack, its lessons in order.
    Packs,
}

/// An installed pack, as the browser lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackInfo {
    /// Its id (`basics`).
    pub id: PackId,
    /// Its title ("Synth basics"); the id when the shell gave only lessons.
    pub title: String,
    /// Its version (`0.1.0`); empty when unknown.
    pub version: String,
    /// Its lessons, as indices into [`Learn::lessons`], in the pack's order.
    pub lessons: Vec<usize>,
}

/// One lesson of a track: its key, and where it is in [`Learn::lessons`] (`None`: its pack is not installed).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrackEntry {
    /// `pack/lesson`.
    pub key: LessonKey,
    /// The lesson, when installed.
    pub lesson: Option<usize>,
}

/// An ordered list of lessons across packs (the tour), as the browser lists it: tracks come before the packs, the
/// tour first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrackInfo {
    /// Its id (`tour`).
    pub id: String,
    /// Its title ("The tour").
    pub title: String,
    /// Its lessons, in order.
    pub entries: Vec<TrackEntry>,
}

/// How far one lesson got, from the progress the view holds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LessonStanding {
    /// Steps done.
    pub done: usize,
    /// Steps in the lesson.
    pub steps: usize,
    /// Every step done.
    pub completed: bool,
    /// Every step done, and every Recreate and Choose step clean.
    pub mastered: bool,
}

/// The editor's Learn state (see the [module docs](self)).
#[derive(Debug, Default)]
pub struct Learn {
    lessons: Vec<LessonBundle>,
    packs: Vec<PackInfo>,
    tracks: Vec<TrackInfo>,
    problems: Vec<String>,
    progress: Progress,
    /// The day the shell says it is (lane REV; `None`: no Review), and the Review queue for it, recomputed whenever
    /// the progress, the lessons or the day change.
    review_day: Option<Day>,
    review: Queue,
    open: bool,
    browsing: bool,
    view: Option<LearnView>,
    overlay: Option<LessonOverlay>,
    queued: Vec<LearnIntent>,
    /// The last refusal, for the pane.
    said: Option<String>,
    /// The last two examples played, newest first (A/B flips between them).
    ab: [Option<u8>; 2],
    /// The browser's tab, once the player picked one.
    shelf: Option<Shelf>,
}

/// The packs `lessons` come from, in order of first appearance, titled by their ids.
fn packs_of(lessons: &[LessonBundle]) -> Vec<PackInfo> {
    let mut packs: Vec<PackInfo> = Vec::new();
    for (i, b) in lessons.iter().enumerate() {
        match packs.iter_mut().find(|p| p.id == b.pack) {
            Some(p) => p.lessons.push(i),
            None => packs.push(PackInfo {
                id: b.pack.clone(),
                title: b.pack.as_str().to_owned(),
                version: b.version.to_string(),
                lessons: vec![i],
            }),
        }
    }
    packs
}

impl Learn {
    /// The lessons the shell found (every lesson of every installed pack, in order) and the packs it refused, one
    /// line each (lane SKL's seam; [`Learn::set_library`] also gives pack titles, tracks and progress).
    pub fn set_lessons(&mut self, lessons: Vec<LessonBundle>, problems: Vec<String>) {
        self.packs = packs_of(&lessons);
        self.tracks.clear();
        self.lessons = lessons;
        self.problems = problems;
        self.refresh_review();
    }

    /// When the window opens: the shell's library (every installed pack, the refused ones as problems) and the
    /// progress file. Tracks come from every pack's manifest, the tour first; a track's lesson whose pack is missing
    /// is listed as missing.
    pub fn set_library(&mut self, library: &Library, progress: Progress) {
        let mut lessons = Vec::new();
        let mut packs = Vec::new();
        for entry in &library.packs {
            let manifest = entry.pack.manifest();
            let mut indices = Vec::new();
            for id in &manifest.lessons {
                if let Some(bundle) = entry.pack.bundle(id) {
                    indices.push(lessons.len());
                    lessons.push(bundle);
                }
            }
            packs.push(PackInfo {
                id: manifest.id.clone(),
                title: manifest.title.clone(),
                version: manifest.version.to_string(),
                lessons: indices,
            });
        }
        let mut tracks: Vec<TrackInfo> = library
            .packs
            .iter()
            .flat_map(|e| e.pack.manifest().tracks.iter())
            .map(|track| TrackInfo {
                id: track.id.to_string(),
                title: track.title.clone(),
                entries: track
                    .lessons
                    .iter()
                    .map(|key| TrackEntry {
                        key: key.clone(),
                        lesson: lessons
                            .iter()
                            .position(|b| b.pack == key.pack && b.lesson.id == key.lesson),
                    })
                    .collect(),
            })
            .collect();
        tracks.sort_by_key(|t| t.id != "tour");
        self.problems = library
            .problems
            .iter()
            .map(|p| format!("{}: {} ({})", p.key, p.error, p.error.hint()))
            .collect();
        self.lessons = lessons;
        self.packs = packs;
        self.tracks = tracks;
        self.progress = progress;
        self.refresh_review();
    }

    /// The progress file's contents (the shell reads it when the window opens, and again after it writes a round's
    /// rings into the review boxes).
    pub fn set_progress(&mut self, progress: Progress) {
        self.progress = progress;
        self.refresh_review();
    }

    /// The day it is, as the shell counts it (days since 1970-01-01, UTC; lane REV): the Review queue's day. The view
    /// reads no clock.
    pub fn set_review_day(&mut self, day: Day) {
        if self.review_day != Some(day) {
            self.review_day = Some(day);
            self.refresh_review();
        }
    }

    /// The Review queue (lane REV): empty until the shell gives a day and a lesson with a Recreate round is passed.
    pub fn review(&self) -> &Queue {
        &self.review
    }

    /// Recompute the Review queue (`enth_learn::review::queue`: pure, a few microseconds a step).
    fn refresh_review(&mut self) {
        self.review = match self.review_day {
            Some(day) => review::queue(&self.progress, &self.lessons, day),
            None => Queue::default(),
        };
    }

    /// The progress the view holds: the file as handed over, plus the steps seen passing since.
    pub fn progress(&self) -> &Progress {
        &self.progress
    }

    /// How far lesson `i` of [`Learn::lessons`] got.
    pub fn standing(&self, i: usize) -> LessonStanding {
        let Some(bundle) = self.lessons.get(i) else {
            return LessonStanding::default();
        };
        let lesson = &bundle.lesson;
        let steps = lesson.steps.len();
        match self.progress.lesson(&bundle.key()) {
            Some(p) => LessonStanding {
                done: lesson
                    .steps
                    .iter()
                    .filter(|s| p.steps.get(&s.id).is_some_and(|s| s.done))
                    .count(),
                steps,
                completed: p.completed(lesson),
                mastered: p.mastered(lesson),
            },
            None => LessonStanding {
                steps,
                ..LessonStanding::default()
            },
        }
    }

    /// The session's lesson, before each frame. A lesson that appears (started, or resumed after a reload) opens
    /// the pane; a step change resets A/B; a step seen passing is marked done in the view's progress.
    pub fn set_lesson(&mut self, view: Option<LearnView>) {
        if view.is_some() && self.view.is_none() {
            self.open = true;
            self.browsing = false;
        }
        self.note_progress(view.as_ref());
        let step = |v: Option<&LearnView>| v.and_then(|v| v.lesson.as_ref()).map(|r| r.step);
        if step(view.as_ref()) != step(self.view.as_ref()) {
            self.ab = [None, None];
        }
        if let Some(e) = view.as_ref().and_then(|v| v.listen.example)
            && self.ab[0] != Some(e)
        {
            self.ab = [Some(e), self.ab[0]];
        }
        let pointing = self.overlay.as_mut().and_then(|o| o.pointing.take());
        self.overlay = view.as_ref().map(|v| LessonOverlay {
            view: v.view.clone(),
            pointing: pointing.clone(),
            beside_pane: self.open,
        });
        self.view = view;
    }

    /// Marks a step done in the view's progress when `next` shows it passed, or moved on from it (a Read or Listen
    /// step passes on Next).
    fn note_progress(&mut self, next: Option<&LearnView>) {
        let (Some(old), Some(new)) = (self.view.as_ref(), next) else {
            return;
        };
        let (Some(a), Some(b)) = (old.lesson.as_ref(), new.lesson.as_ref()) else {
            return;
        };
        if a.pack != b.pack || a.lesson != b.lesson {
            return;
        }
        let Some(bundle) = self
            .lessons
            .iter()
            .find(|l| l.pack == b.pack && l.lesson.id == b.lesson)
        else {
            return;
        };
        let passes_on_next = |i: u16| {
            bundle
                .lesson
                .steps
                .get(usize::from(i))
                .is_some_and(|s| matches!(s.kind, StepKind::Read | StepKind::Listen { .. }))
        };
        let mut done = Vec::new();
        if new.phase == StepPhase::Passed && a.step == b.step && old.phase != StepPhase::Passed {
            done.push(b.step);
        }
        if b.step == a.step + 1 && passes_on_next(a.step) {
            done.push(a.step);
        }
        if new.finished && !old.finished && passes_on_next(b.step) {
            done.push(b.step);
        }
        let key = bundle.key();
        let records: Vec<StepRecord> = done
            .into_iter()
            .filter_map(|i| bundle.lesson.steps.get(usize::from(i)))
            .map(|s| StepRecord {
                lesson: key.clone(),
                step: s.id.clone(),
                progress: StepProgress {
                    done: true,
                    ..StepProgress::default()
                },
            })
            .collect();
        for r in &records {
            self.progress.record(r);
        }
        if !records.is_empty() {
            self.refresh_review(); // a lesson passed here may join the Review queue (lane REV)
        }
    }

    /// What the header's hook shows.
    pub fn header(&self) -> HeaderLearn {
        let lesson = self.view.as_ref().map(|v| {
            (
                v.lesson.as_ref().map_or(0, |r| r.step.saturating_add(1)),
                v.steps,
            )
        });
        HeaderLearn {
            available: !self.lessons.is_empty() || self.view.is_some(),
            lesson,
            open: self.open,
        }
    }

    /// Whether the lesson pane shows this frame (the editor switches to the `learn` layout for it, unless its own
    /// layout places the `lesson` region).
    pub fn panel_open(&self) -> bool {
        self.open
    }

    /// Whether the pane shows the lesson browser: no lesson is live, or the player asked for it over one.
    pub fn browsing(&self) -> bool {
        self.browsing || self.view.is_none()
    }

    /// The lesson's read model, if one is live.
    pub fn view(&self) -> Option<&LearnView> {
        self.view.as_ref()
    }

    /// The shell's lessons, every pack's in order.
    pub fn lessons(&self) -> &[LessonBundle] {
        &self.lessons
    }

    /// The installed packs.
    pub fn packs(&self) -> &[PackInfo] {
        &self.packs
    }

    /// The tracks across packs, the tour first.
    pub fn tracks(&self) -> &[TrackInfo] {
        &self.tracks
    }

    /// The browser's tab: the one picked, else the first track (the tour), else every pack.
    pub fn shelf(&self) -> Shelf {
        match self.shelf {
            Some(Shelf::Track(i)) if i < self.tracks.len() => Shelf::Track(i),
            Some(Shelf::Packs) => Shelf::Packs,
            _ if !self.tracks.is_empty() => Shelf::Track(0),
            _ => Shelf::Packs,
        }
    }

    /// The packs the shell refused.
    pub fn problems(&self) -> &[String] {
        &self.problems
    }

    /// The last refusal, if the latest intent was refused.
    pub fn said(&self) -> Option<&str> {
        self.said.as_deref()
    }

    /// The other example A/B would play: the one played before the one playing now.
    pub fn flip_to(&self) -> Option<u8> {
        let playing = self.view.as_ref()?.listen.example?;
        match self.ab {
            [Some(a), Some(b)] if a == playing => Some(b),
            _ => None,
        }
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

    /// The step's overlay, while a lesson is live (in its Recreate step, the round's overlay is laid over it).
    pub fn overlay(&self) -> Option<&LessonOverlay> {
        self.overlay.as_ref()
    }

    /// The `enth:` link or `control` widget under the pointer this frame: its control is outlined while hovered
    /// (the pane is drawn before the cards, so no extra frame is needed).
    pub fn set_pointing(&mut self, pointing: Option<Selector>) {
        if let Some(o) = self.overlay.as_mut() {
            o.pointing = pointing;
        }
    }

    /// Queue an intent for [`Learn::apply`].
    pub fn push(&mut self, intent: LearnIntent) {
        self.queued.push(intent);
    }

    /// Whether intents wait for [`Learn::apply`] (the editor asks for one more frame to show their effect).
    pub fn has_queued(&self) -> bool {
        !self.queued.is_empty()
    }

    /// After the frame: the queued intents, in order, on the session. A refusal is kept for the pane (the session
    /// also keeps its last error in the read model).
    pub fn apply(&mut self, session: &mut dyn LearnEdit) {
        for intent in std::mem::take(&mut self.queued) {
            let result = match intent {
                LearnIntent::Toggle => {
                    self.open = !self.open;
                    if let Some(o) = self.overlay.as_mut() {
                        o.beside_pane = self.open;
                    }
                    Ok(())
                }
                LearnIntent::Browse(on) => {
                    self.browsing = on;
                    Ok(())
                }
                LearnIntent::Review => match self.review.items.first() {
                    Some(item) => session.review_start(item.clone()),
                    None => Ok(()),
                },
                LearnIntent::Shelf(shelf) => {
                    self.shelf = Some(shelf);
                    Ok(())
                }
                LearnIntent::Start(i) => match self.lessons.get(i) {
                    Some(bundle) => {
                        let started = session.learn_start(Box::new(Player::new(bundle.clone())));
                        if started.is_ok() {
                            self.browsing = false;
                            self.open = true;
                            if let Some(o) = self.overlay.as_mut() {
                                o.beside_pane = true;
                            }
                        }
                        started
                    }
                    None => Ok(()),
                },
                LearnIntent::Next => session.learn_next(),
                LearnIntent::Back => session.learn_back(),
                LearnIntent::Hint => session.learn_hint(),
                LearnIntent::Show => session.learn_show(),
                LearnIntent::Choose(o) => session.learn_choose(o),
                LearnIntent::Listen(e) => session.learn_listen(e),
                LearnIntent::Flip => match self.flip_to() {
                    Some(e) => session.learn_listen(Some(e)),
                    None => Ok(()),
                },
                LearnIntent::End(choice) => session.learn_end(choice),
            };
            self.said = result.err().map(|e| format!("{e}: {}", e.hint()));
        }
    }
}

/// Whether `selector` names something in module `id`: the module, one of its parameters, or a route into it or out of
/// it.
fn mentions(selector: &Selector, id: &ModuleId) -> bool {
    let module = Target::Module(id.clone());
    match selector {
        Selector::Module(t) => *t == module,
        Selector::Param(a) => *a.target() == module,
        Selector::Route(r) => {
            *r.dst.target() == module || matches!(&r.src, ModSource::Module(src) if src == id)
        }
        Selector::Macro(_) => false,
    }
}

/// A lesson step's view laid over the cards (see the [module docs](self)).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LessonOverlay {
    view: StepView,
    /// The `enth:` link under the pointer.
    pointing: Option<Selector>,
    /// The cards share the window with the lesson pane: a focus step then shows only its own modules. With the pane
    /// hidden the player asked for the whole editor, so focus gives way (hides, highlights and locks stay).
    beside_pane: bool,
}

impl LessonOverlay {
    /// The overlay of a step's view, beside the lesson pane.
    pub fn new(view: StepView) -> Self {
        LessonOverlay {
            view,
            pointing: None,
            beside_pane: true,
        }
    }

    /// The overlay with the pane shown (`true`) or hidden.
    #[must_use]
    pub fn beside_pane(mut self, beside: bool) -> Self {
        self.beside_pane = beside;
        self
    }

    /// The overlay with `selector` hovered in the step's text.
    #[must_use]
    pub fn pointing(mut self, selector: Option<Selector>) -> Self {
        self.pointing = selector;
        self
    }

    /// What it outlines: the step's highlights and the hovered link.
    fn outlined(&self) -> impl Iterator<Item = &Selector> {
        self.view.highlight.iter().chain(self.pointing.as_ref())
    }

    /// A focus step beside the pane shows only the modules it highlights or leaves editable.
    fn out_of_focus(&self, id: &ModuleId) -> bool {
        self.beside_pane
            && self.view.layout == StepLayout::Focus
            && !self
                .view
                .highlight
                .iter()
                .chain(&self.view.editable)
                .any(|s| mentions(s, id))
    }
}

impl Overlay for LessonOverlay {
    fn param(&self, addr: &ParamAddr) -> ParamOverlay {
        let touch = Touch::Param(addr.clone());
        let outlined = self.outlined().any(|s| match s {
            Selector::Param(a) => a == addr,
            Selector::Route(r) => r.dst == *addr,
            Selector::Module(_) | Selector::Macro(_) => false,
        });
        let emphasis = if self.view.hide.iter().any(|s| s.covers(&touch)) {
            Emphasis::Hidden
        } else if self.view.locks(&touch) {
            Emphasis::Dimmed
        } else if outlined {
            Emphasis::Highlighted
        } else {
            Emphasis::Normal
        };
        ParamOverlay {
            emphasis,
            score: None,
            hint: None,
        }
    }

    fn module(&self, id: &ModuleId) -> Emphasis {
        let touch = Touch::Module(Target::Module(id.clone()));
        let outlined = self.outlined().any(|s| match s {
            Selector::Module(Target::Module(m)) => m == id,
            Selector::Route(r) => matches!(&r.src, ModSource::Module(src) if src == id),
            _ => false,
        });
        if self.view.hide.iter().any(|s| s.covers(&touch)) || self.out_of_focus(id) {
            Emphasis::Hidden
        } else if outlined {
            Emphasis::Highlighted
        } else {
            Emphasis::Normal
        }
    }

    fn locked(&self, addr: &ParamAddr) -> bool {
        self.view.locks(&Touch::Param(addr.clone()))
    }

    fn module_locked(&self, id: &ModuleId) -> bool {
        self.view.locks(&Touch::Module(Target::Module(id.clone())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sel(s: &str) -> Selector {
        s.parse().unwrap()
    }

    fn addr(s: &str) -> ParamAddr {
        s.parse().unwrap()
    }

    fn id(s: &str) -> ModuleId {
        ModuleId::new(s).unwrap()
    }

    #[test]
    fn a_step_view_outlines_locks_hides_and_focuses() {
        let view = StepView {
            highlight: vec![sel("filter1.cutoff")],
            hide: vec![sel("noise1.*")],
            editable: vec![sel("filter1.cutoff")],
            locked: vec![sel("osc1.*"), sel("filter1.drive")],
            ..StepView::default()
        };
        let o = LessonOverlay::new(view.clone());
        assert_eq!(
            o.param(&addr("filter1.cutoff")).emphasis,
            Emphasis::Highlighted
        );
        assert_eq!(o.param(&addr("filter1.drive")).emphasis, Emphasis::Dimmed);
        assert!(o.locked(&addr("filter1.drive")));
        assert!(o.locked(&addr("osc1.level")) && o.module_locked(&id("osc1")));
        assert!(!o.module_locked(&id("filter1")));
        assert_eq!(o.module(&id("noise1")), Emphasis::Hidden);
        assert_eq!(
            o.module(&id("env1")),
            Emphasis::Normal,
            "a full step shows the rest"
        );
        let focus = LessonOverlay::new(StepView {
            layout: StepLayout::Focus,
            ..view
        });
        assert_eq!(focus.module(&id("env1")), Emphasis::Hidden, "out of focus");
        assert_eq!(focus.module(&id("filter1")), Emphasis::Normal);
        assert_eq!(
            focus.module(&id("osc1")),
            Emphasis::Hidden,
            "locked context is out of focus too"
        );
        let whole = focus.beside_pane(false);
        assert_eq!(
            whole.module(&id("env1")),
            Emphasis::Normal,
            "with the pane hidden, focus gives way"
        );
        assert_eq!(whole.module(&id("noise1")), Emphasis::Hidden, "hides stay");
        assert!(whole.module_locked(&id("osc1")), "locks stay");
    }

    #[test]
    fn a_hovered_link_outlines_its_control_or_card() {
        let o = LessonOverlay::new(StepView::default());
        assert_eq!(o.param(&addr("env1.attack")).emphasis, Emphasis::Normal);
        let o = o.pointing(Some(sel("env1.attack")));
        assert_eq!(
            o.param(&addr("env1.attack")).emphasis,
            Emphasis::Highlighted
        );
        let o = o.pointing(Some(sel("lfo1 -> filter1.cutoff")));
        assert_eq!(
            o.param(&addr("filter1.cutoff")).emphasis,
            Emphasis::Highlighted,
            "a route's destination"
        );
        assert_eq!(
            o.module(&id("lfo1")),
            Emphasis::Highlighted,
            "and its source's card"
        );
        let o = o.pointing(Some(sel("osc2.*")));
        assert_eq!(o.module(&id("osc2")), Emphasis::Highlighted);
        assert_eq!(o.param(&addr("osc2.level")).emphasis, Emphasis::Normal);
    }

    #[test]
    fn intents_that_only_change_the_view_need_no_session() {
        struct Never;
        impl LearnEdit for Never {
            fn lesson(&self) -> Option<LearnView> {
                None
            }
            fn learn_start(
                &mut self,
                _: Box<dyn enth_session::learn::LessonPlayer>,
            ) -> Result<(), enth_session::learn::LearnError> {
                unreachable!()
            }
            fn learn_next(&mut self) -> Result<(), enth_session::learn::LearnError> {
                unreachable!()
            }
            fn learn_back(&mut self) -> Result<(), enth_session::learn::LearnError> {
                unreachable!()
            }
            fn learn_hint(&mut self) -> Result<(), enth_session::learn::LearnError> {
                unreachable!()
            }
            fn learn_show(&mut self) -> Result<(), enth_session::learn::LearnError> {
                unreachable!()
            }
            fn learn_choose(&mut self, _: u8) -> Result<(), enth_session::learn::LearnError> {
                unreachable!()
            }
            fn learn_listen(
                &mut self,
                _: Option<u8>,
            ) -> Result<(), enth_session::learn::LearnError> {
                unreachable!()
            }
            fn learn_end(&mut self, _: EndChoice) -> Result<(), enth_session::learn::LearnError> {
                unreachable!()
            }
        }
        let mut learn = Learn::default();
        assert!(!learn.panel_open() && learn.browsing());
        for intent in [
            LearnIntent::Toggle,
            LearnIntent::Browse(true),
            LearnIntent::Review,
            LearnIntent::Flip,
            LearnIntent::Start(3),
        ] {
            learn.push(intent);
        }
        learn.apply(&mut Never);
        assert!(learn.panel_open() && learn.said().is_none());
    }
}
