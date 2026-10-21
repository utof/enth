//! The editor: one frame drawn from the session through a layout, and its intents applied at the end (lane ED1).
//!
//! [`Editor::frame`] runs inside the shell's lock on the session (the plugin's `Shared::with_session`, the dev
//! window's own session): it reads the patch through [`SessionView`], cuts the window into the layout's regions,
//! draws each region's component from fresh read models (nothing is cached across frames), then applies the frame's
//! intents in order through [`SessionEdit`] ([`Editor::apply`]). It never pumps, folds or drains: the shell does
//! those around it, and forwards host writes after unlocking (docs/plugin.md, docs/editor.md).
//!
//! Editor view state that is not in the patch lives here: the open drag's gesture, the cards whose "More" is open,
//! the layout, the overlay and a notice for the status line. egui's own state (scroll offsets, the "Type value…"
//! field) is keyed by stable ids, so it follows a control through a re-layout.

use std::collections::BTreeSet;
use std::time::Duration;

use egui::{Context, DragAndDrop, Rect, Ui, Vec2};
use enth_patch::history::GestureId;
use enth_patch::modulation::source_span;
use enth_patch::{ModSource, Patch, Section, modulation_range};
use enth_session::{SessionEdit, SessionView};

use super::add_module::{self, Room};
use super::browser::{self, Presets};
use super::cards::view::{self, CardView};
use super::cards::{CardKey, Intent, Intents, ids};
use super::describe;
use super::layout::{Layout, LayoutError, LayoutSpec, Part, Placed, Region};
use super::live::{Fingerprint, Live};
use super::modulate::{self, SourceDrag};
use super::overlay::{self, NoOverlay, Overlay};
use super::regions::{
    CardRegion, Header, HeaderView, MacroView, Macros, MatrixView, Output, OutputState, OutputView,
    Overview, SourceView, Status,
};
use crate::Theme;

/// What the last frame drew, for tests and the dev window: where the regions went and what did not fit.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FrameReport {
    /// The region rectangles, in tree order.
    pub regions: Vec<Placed>,
    /// Content that reached past its region, by part (`"sources"`, …, `"overview.tiles"`, `"overview.matrix"`), in
    /// px per axis. Only entries over half a pixel are listed: an empty list means nothing scrolls.
    pub overflow: Vec<(&'static str, Vec2)>,
    /// The next frame the editor asked egui for (lane ED2): `motion.scope_interval()` (1/60 s; lane AM, R1/S1) while
    /// visible live data changes, `None` when it sleeps on the tap (0 frames until input or new data).
    pub repaint: Option<Duration>,
}

impl FrameReport {
    /// The rectangle of `region`, if the layout gave it space.
    pub fn rect(&self, region: Region) -> Option<Rect> {
        self.regions
            .iter()
            .find(|p| p.region == region)
            .map(|p| p.rect)
    }
}

/// The editor. See the [module docs](self).
pub struct Editor {
    layout: Layout,
    /// The theme the layout's tokens were resolved against.
    resolved_for: Theme,
    /// The gesture a drag opened, until its `End` (or the pointer's release anywhere).
    gesture: Option<GestureId>,
    /// Cards whose "More" disclosure is open.
    more: BTreeSet<CardKey>,
    overlay: Box<dyn Overlay + Send + Sync>,
    /// Shown on the status line's right end (a refused `ENTH_LAYOUT`, the dev window's layout name).
    notice: String,
    /// What the last add or removal did ("Removed Filter 1 and 2 routes"; lane AM), shown on the status line's right
    /// end in the notice's place until the next edit.
    said: Option<String>,
    report: FrameReport,
    /// Telemetry and repaint gating (lane ED2).
    live: Live,
    /// The output region's mode, meters and spectrum.
    output: OutputState,
    /// The preset browser (lane PBU): its store, the sheet over the body, the crash offer.
    presets: Presets,
    /// egui's input time at the last frame (the meters fall by the time between frames).
    last_time: Option<f64>,
    /// The Recreate bar's state (lane SKR's skeleton): the round view the shell handed over, the round intents
    /// waiting for it, the blind-rating question.
    #[cfg(feature = "recreate")]
    recreate: super::recreate::bar::Recreate,
    /// The Learn state (lane SKL's minimal panel, F2l; lane LUI's view): the shell's lessons, the session's lesson
    /// read model, the pane and the intents waiting for the session.
    #[cfg(feature = "learn")]
    learn: super::learn::Learn,
    /// The built-in layout `learn` (lane LUI), drawn while the lesson pane shows unless `layout` places the `lesson`
    /// region itself.
    #[cfg(feature = "learn")]
    learn_layout: Layout,
}

impl std::fmt::Debug for Editor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Editor")
            .field("layout", &self.layout.name())
            .field("gesture", &self.gesture)
            .field("more", &self.more)
            .finish_non_exhaustive()
    }
}

impl Default for Editor {
    /// The editor with layout 1 ("lanes") for the dark theme.
    fn default() -> Self {
        Editor::new(&Theme::dark())
    }
}

impl Editor {
    /// An editor with layout 1, "lanes".
    pub fn new(theme: &Theme) -> Self {
        let layout = Layout::new(LayoutSpec::lanes(), theme)
            .or_else(|_| Layout::new(LayoutSpec::lanes(), &Theme::dark()))
            .unwrap_or_else(|e| unreachable!("the built-in layout checks: {e}"));
        Editor {
            layout,
            resolved_for: theme.clone(),
            gesture: None,
            more: BTreeSet::new(),
            overlay: Box::new(NoOverlay),
            notice: String::new(),
            said: None,
            report: FrameReport::default(),
            live: Live::default(),
            output: OutputState::default(),
            presets: Presets::default(),
            last_time: None,
            #[cfg(feature = "recreate")]
            recreate: super::recreate::bar::Recreate::default(),
            #[cfg(feature = "learn")]
            learn: super::learn::Learn::default(),
            #[cfg(feature = "learn")]
            learn_layout: learn_layout(theme),
        }
    }

    /// An editor with `spec`, if it checks.
    pub fn with_layout(spec: LayoutSpec, theme: &Theme) -> Result<Self, LayoutError> {
        let mut editor = Editor::new(theme);
        editor.set_layout(spec, theme)?;
        Ok(editor)
    }

    /// Switch to `spec` (between frames; a drag in flight continues, since every control keeps its id). On error the
    /// current layout stays.
    pub fn set_layout(&mut self, spec: LayoutSpec, theme: &Theme) -> Result<(), LayoutError> {
        self.layout = Layout::new(spec, theme)?;
        theme.clone_into(&mut self.resolved_for);
        Ok(())
    }

    /// The layout in use.
    pub fn layout(&self) -> &Layout {
        &self.layout
    }

    /// Lay `overlay` over the editor (Learn and Recreate; P1b keeps [`NoOverlay`]).
    pub fn set_overlay(&mut self, overlay: Box<dyn Overlay + Send + Sync>) {
        self.overlay = overlay;
    }

    /// Text for the status line's right end (`ENTH_LAYOUT` refused, the dev window's layout name); empty for none.
    pub fn set_notice(&mut self, notice: impl Into<String>) {
        self.notice = notice.into();
    }

    /// What the last frame drew.
    pub fn report(&self) -> &FrameReport {
        &self.report
    }

    /// Whether a drag's gesture is open.
    pub fn gesture_open(&self) -> bool {
        self.gesture.is_some()
    }

    /// Whether `card`'s "More" disclosure is open.
    pub fn more_open(&self, card: &CardKey) -> bool {
        self.more.contains(card)
    }

    /// The window opened (the shell's `build()`, every time): live feedback starts from the session's telemetry tap,
    /// now or as soon as the session has one (lane ED2). Until then the editor shows no telemetry and asks for no
    /// frames of its own.
    pub fn open(&mut self, session: &dyn SessionView, theme: &Theme) {
        self.live
            .open(session, usize::from(theme.size.output.scope_samples));
        self.last_time = None;
    }

    /// The window closed (or hid): the tap stops publishing and the editor asks for no more frames.
    pub fn close(&mut self) {
        self.live.close();
        self.presets.closed();
    }

    /// The live state (telemetry, for tests and the dev window).
    pub fn live(&self) -> &Live {
        &self.live
    }

    /// The preset browser (lane PBU): what covers the body, the list, the last message.
    pub fn presets(&self) -> &Presets {
        &self.presets
    }

    /// The preset browser, to hand it the shell's store ([`Presets::set_store`]) and crash reports
    /// ([`Presets::offer_crash`]).
    pub fn presets_mut(&mut self) -> &mut Presets {
        &mut self.presets
    }

    /// Before a frame (lane SKR, the Recreate hook): the session's round, from `GameEdit::round()`. A shell that
    /// plays rounds calls it every frame; until one does, the header shows no Recreate bar.
    #[cfg(feature = "recreate")]
    pub fn set_round(&mut self, round: Option<enth_session::game::RoundView>) {
        self.recreate.set_round(round);
    }

    /// After a frame (lane SKR): apply the bar's queued round intents to the shell's `game` (the session, which the
    /// editor otherwise sees only as a `SessionEdit`), with `seed` for a round from settings.
    #[cfg(feature = "recreate")]
    pub fn apply_game(
        &mut self,
        game: &mut dyn enth_session::game::GameEdit,
        seed: &mut dyn FnMut() -> u64,
    ) {
        self.recreate.apply(game, seed);
    }

    /// Before handing worker results back to the session (lane SKR): a score waits while the blind rating is asked.
    #[cfg(feature = "recreate")]
    pub fn hold_scores(
        &mut self,
        results: Vec<(enth_session::game::JobId, enth_session::game::JobResult)>,
    ) -> Vec<(enth_session::game::JobId, enth_session::game::JobResult)> {
        self.recreate.hold_scores(results)
    }

    /// The Recreate bar's state (tests).
    #[cfg(feature = "recreate")]
    pub fn recreate(&self) -> &super::recreate::bar::Recreate {
        &self.recreate
    }

    /// Before a frame (lane SKL, F2l): the session's lesson (`LearnEdit::lesson()`), as `set_round` hands over a
    /// round.
    #[cfg(feature = "learn")]
    pub fn set_lesson(&mut self, lesson: Option<enth_session::learn::LearnView>) {
        self.learn.set_lesson(lesson);
    }

    /// When the window opens (lane SKL): the shell's installed lessons and the packs it refused. Lane LUI's
    /// [`Editor::set_library`] also gives the pack titles, the tracks and the progress file.
    #[cfg(feature = "learn")]
    pub fn set_lessons(&mut self, lessons: Vec<enth_learn::LessonBundle>, problems: Vec<String>) {
        self.learn.set_lessons(lessons, problems);
    }

    /// When the window opens (lane LUI): the shell's library (every installed pack, the refused ones as problems)
    /// and the progress file, for the lesson browser.
    #[cfg(feature = "learn")]
    pub fn set_library(
        &mut self,
        library: &enth_learn::store::Library,
        progress: enth_learn::Progress,
    ) {
        self.learn.set_library(library, progress);
    }

    /// When the window opens (lane LFU): the Packs folder, for the lesson browser's "Open packs folder".
    #[cfg(feature = "learn")]
    pub fn set_packs_folder(&mut self, folder: Option<String>) {
        self.learn.set_packs_folder(folder);
    }

    /// After a frame (lane LFU): whether "Open packs folder" was clicked; the shell makes the folder if missing and
    /// opens it (egui's `OpenUrl`), since the editor does no I/O. Asking clears it.
    #[cfg(feature = "learn")]
    pub fn take_open_packs_folder(&mut self) -> bool {
        self.learn.take_open_packs_folder()
    }

    /// After a frame (lane SKL): the Learn view's intents, applied to the session's `LearnEdit`.
    #[cfg(feature = "learn")]
    pub fn apply_learn(&mut self, session: &mut dyn enth_session::learn::LearnEdit) {
        self.learn.apply(session);
    }

    /// The Learn state (tests and the dev window).
    #[cfg(feature = "learn")]
    pub fn learn(&self) -> &super::learn::Learn {
        &self.learn
    }

    /// The progress file again (lane REV): after the shell writes a round's rings into it, so the Review queue
    /// follows.
    #[cfg(feature = "learn")]
    pub fn set_progress(&mut self, progress: enth_learn::Progress) {
        self.learn.set_progress(progress);
    }

    /// The day it is, as the shell counts it (days since 1970-01-01, UTC; lane REV): the Review queue's day.
    #[cfg(feature = "learn")]
    pub fn set_review_day(&mut self, day: enth_learn::review::Day) {
        self.learn.set_review_day(day);
    }

    /// The cards of one part of `patch`, with "More" state, live values, modulation ranges, the drop check of a
    /// dragged source and the overlay applied.
    fn cards(&self, patch: &Patch, part: Part, drag: Option<&ModSource>) -> Vec<CardView> {
        let mut cards = match part {
            Part::Generators => view::section(patch, Section::Generators),
            Part::VoiceFx => view::section(patch, Section::VoiceFx),
            Part::Modulators => view::section(patch, Section::Modulators),
            Part::Fx => view::section(patch, Section::Fx),
            Part::Voice => vec![CardView::voice(patch)],
            Part::Master => vec![CardView::master(patch)],
        };
        for card in &mut cards {
            card.more_open = self.more.contains(&card.key);
            for p in &mut card.params {
                if !p.routes.is_empty() {
                    p.range = modulation_range(patch, &p.addr)
                        .filter(enth_patch::ModulationRange::is_modulated)
                        .map(|r| (r.lo, r.hi));
                    p.live = self.live.value(&p.addr).map(|v| p.spec.to_norm(v));
                }
                if let Some(src) = drag {
                    p.drop = Some(modulate::check(patch, src, &p.addr, p.spec));
                }
            }
        }
        self.with_overlay(|o| overlay::apply(o, &mut cards));
        cards
    }

    /// Runs `f` with the overlay laid over the cards: a live Recreate round's rings and dimming (lane RUI), a lesson
    /// step's highlights, locks and hidden modules (lane LUI), the round's over the step's in a lesson's Recreate
    /// step, else the one set with [`Editor::set_overlay`].
    fn with_overlay<R>(&self, f: impl FnOnce(&dyn Overlay) -> R) -> R {
        #[cfg(all(feature = "recreate", feature = "learn"))]
        if let (Some(round), Some(lesson)) = (self.recreate.overlay(), self.learn.overlay()) {
            return f(&overlay::Layered {
                top: round,
                base: lesson,
            });
        }
        #[cfg(feature = "recreate")]
        if let Some(round) = self.recreate.overlay() {
            return f(round);
        }
        #[cfg(feature = "learn")]
        if let Some(lesson) = self.learn.overlay() {
            return f(lesson);
        }
        f(self.overlay.as_ref())
    }

    /// The modulation sources, with their live levels in the tiles' 0..1. A module the overlay hides (a lesson step
    /// that does not use it, lane LUI) has no tile either.
    fn sources(&self, patch: &Patch) -> Vec<SourceView> {
        let mut sources = SourceView::all(patch);
        self.with_overlay(|o| {
            sources.retain(
                |s| !matches!(&s.src, ModSource::Module(id) if o.module(id) == overlay::Emphasis::Hidden),
            );
        });
        for s in &mut sources {
            let output = match &s.src {
                ModSource::Module(id) => self.live.modulator(id),
                ModSource::ModWheel => self.live.wheel(),
                _ => None,
            };
            s.level = output.zip(source_span(patch, &s.src)).map(|(v, (lo, hi))| {
                if hi > lo {
                    ((v - lo) / (hi - lo)).clamp(0.0, 1.0)
                } else {
                    0.0
                }
            });
        }
        sources
    }

    /// One frame: draw every region of the layout into `ui`'s rectangle from `session`, then apply the intents.
    /// `pointer_down`: whether any pointer button is held (a drag released anywhere ends its gesture).
    pub fn frame(
        &mut self,
        ui: &mut Ui,
        theme: &Theme,
        session: &mut dyn SessionEdit,
        pointer_down: bool,
    ) {
        if *theme != self.resolved_for {
            if let Ok(layout) = Layout::new(self.layout.spec().clone(), theme) {
                self.layout = layout;
            }
            #[cfg(feature = "learn")]
            {
                self.learn_layout = learn_layout(theme);
            }
            theme.clone_into(&mut self.resolved_for);
        }
        let t = theme;
        self.live.refresh(&*session);
        let ctx = ui.ctx().clone();
        let now = ctx.input(|i| i.time);
        let dt = self.last_time.map_or(0.0, |last| (now - last) as f32);
        self.last_time = Some(now);
        // A source's ⊕ being dragged (drag-to-modulate): every control checks itself against it.
        let drag = DragAndDrop::payload::<SourceDrag>(&ctx);
        let drag = drag.as_deref().map(|d| &d.src);
        let mut fingerprint = Fingerprint::default();
        let rect = ui.max_rect();
        ui.painter()
            .rect_filled(rect, 0.0, t.color.surface.background);
        // While the lesson pane shows, the built-in layout `learn` places it (lane LUI), unless the editor's own layout
        // places the `lesson` region; what no longer fits is in the overflow report.
        #[cfg(feature = "learn")]
        let (regions, layout) = {
            let own = self.layout.regions(rect);
            if self.learn.panel_open() && !own.iter().any(|p| p.region == Region::Lesson) {
                (self.learn_layout.regions(rect), &self.learn_layout)
            } else {
                (own, &self.layout)
            }
        };
        #[cfg(not(feature = "learn"))]
        let (regions, layout) = (self.layout.regions(rect), &self.layout);
        // A preset sheet (lane PBU) covers the body: the regions under it are not drawn at all.
        let body = browser::body(&regions);
        #[cfg(feature = "recreate")]
        let settings_page = self.recreate.covers_body();
        #[cfg(not(feature = "recreate"))]
        let settings_page = false;
        let sheet = body.filter(|_| self.presets.covers_body() || settings_page);
        let mut out = Intents::default();
        let mut overflow = Vec::new();
        let mut status = None;
        {
            let patch = session.patch();
            // The lesson pane first (lane LUI): the `enth:` link it finds under the pointer outlines its control on
            // the cards drawn after it, in the same frame.
            #[cfg(feature = "learn")]
            if let Some(p) = regions.iter().find(|p| p.region == Region::Lesson) {
                let shown = super::learn::pane::show(
                    &self.learn,
                    ui,
                    t,
                    p.rect,
                    patch,
                    &self.live,
                    &mut out,
                );
                overflow.push(("learn", shown.overflow));
                self.learn.set_pointing(shown.pointing);
            }
            for placed in &regions {
                if sheet.is_some_and(|b| browser::covered(placed, b)) {
                    continue;
                }
                let r = placed.rect;
                match placed.region {
                    Region::Header => {
                        let history = session.history();
                        let header = HeaderView {
                            presets: self.presets.header(&*session),
                            patch_name: patch.meta.name.clone(),
                            can_undo: history.can_undo(),
                            can_redo: history.can_redo(),
                            undo: history.next_undo().map(|e| describe::entry(patch, e)),
                            redo: history.next_redo().map(|e| describe::entry(patch, e)),
                            room: Room::all(patch),
                            #[cfg(feature = "recreate")]
                            recreate: self.recreate.view(),
                            // Lane S0l (R2): lessons are not available until lane LUI hands the editor a Learn state.
                            #[cfg(feature = "learn")]
                            learn: self.learn.header(),
                            // Lane S0s (R3): create is not available until lane PUI hands the editor a create state.
                            #[cfg(feature = "create")]
                            create: super::create::HeaderCreate::default(),
                        };
                        let knobs: Vec<CardView> = layout
                            .parts_in(Region::Header)
                            .flat_map(|part| self.cards(patch, part, drag))
                            .collect();
                        Header::new(t, &header).knobs(&knobs).show(ui, r, &mut out);
                        mark_cards(&ctx, &knobs, &mut fingerprint);
                    }
                    Region::Macros => {
                        Macros::new(t, &MacroView::all(patch)).show(ui, r, &mut out);
                    }
                    Region::Sources | Region::VoiceFx | Region::Modulators | Region::Effects => {
                        let cards: Vec<CardView> = layout
                            .parts_in(placed.region)
                            .flat_map(|part| self.cards(patch, part, drag))
                            .collect();
                        let room = Room::of_parts(patch, layout.parts_in(placed.region));
                        let shown =
                            CardRegion::new(t, placed.region.name(), &cards, placed.arrange)
                                .room(&room)
                                .show(ui, r, &mut out);
                        overflow.push((placed.region.name(), shown.overflow));
                        mark_cards(&ctx, &cards, &mut fingerprint);
                    }
                    // A Recreate round's results drawer takes the overview slot once it has something to show (lane RUI).
                    #[cfg(feature = "recreate")]
                    Region::Overview if self.recreate.drawer_open() => {
                        let shown =
                            super::recreate::results::show(&mut self.recreate, ui, t, r, &mut out);
                        overflow.push(("results", shown.overflow));
                    }
                    Region::Overview => {
                        let sources = self.sources(patch);
                        let matrix = self.with_overlay(|o| {
                            // The routes of a module the overlay hides (a lesson step, lane LUI) are not listed.
                            let hidden = |id: &enth_patch::ModuleId| o.module(id) == overlay::Emphasis::Hidden;
                            let gone = |r: &enth_patch::Route| {
                                matches!(&r.src, ModSource::Module(id) if hidden(id))
                                    || matches!(r.dst.target(), enth_patch::Target::Module(id) if hidden(id))
                            };
                            if patch.routes.iter().any(gone) {
                                let mut shown = patch.clone();
                                shown.routes.retain(|r| !gone(r));
                                MatrixView::of(&shown).scored(|route| o.route(route))
                            } else {
                                MatrixView::of(patch).scored(|route| o.route(route))
                            }
                        });
                        let shown = Overview::new(t, &sources, &matrix).show(ui, r, &mut out);
                        #[cfg(feature = "recreate")]
                        if self.recreate.has_results() {
                            super::recreate::results::reopen(ui, t, r, &mut out);
                        }
                        overflow.push(("overview.tiles", shown.tiles_overflow));
                        overflow.push(("overview.matrix", shown.matrix_overflow));
                        for s in &sources {
                            if let Some(level) = s.level {
                                fingerprint.visible(&ctx, ids::source(&s.src), &s.src, level);
                            }
                        }
                    }
                    Region::Output => {
                        let view = OutputView {
                            scope: self.live.scope(),
                            peaks: self.live.peaks(),
                            dt,
                        };
                        Output::new(t, view).show(
                            ui,
                            r,
                            &mut self.output,
                            &mut fingerprint,
                            &mut out,
                        );
                    }
                    // Drawn last: it shows what the other regions report as hovered.
                    Region::Status => status = Some(r),
                    // Drawn first, above (lane LUI).
                    #[cfg(feature = "learn")]
                    Region::Lesson => {}
                    #[cfg(feature = "recreate")]
                    Region::ChallengeBar | Region::Results => {
                        super::reserved::show(ui, t, r, placed.region);
                    }
                }
            }
            if let Some(b) = sheet {
                #[cfg(feature = "recreate")]
                if settings_page {
                    super::recreate::settings::show(&mut self.recreate, ui, t, b, &mut out);
                }
                if !settings_page {
                    self.presets.show(ui, t, b, &*session, &mut out);
                }
            }
        }
        if let Some(r) = status {
            let error = session.status().last_error.map(|e| e.hint());
            // A live Recreate round's readout (lane SKR; kept by lane RUI's bar) takes the notice's place.
            #[cfg(feature = "recreate")]
            let round = self.recreate.readout();
            #[cfg(not(feature = "recreate"))]
            let round: Option<String> = None;
            let right = error
                .as_deref()
                .or(self.said.as_deref())
                .or(round.as_deref())
                .unwrap_or(&self.notice);
            Status::new(t, out.hovered.as_ref())
                .right(right)
                .show(ui, r);
        }
        if let Some(b) = body {
            self.presets.show_notice(ui, t, b, &mut out);
        }
        overflow.retain(|(_, o)| o.x > 0.5 || o.y > 0.5);
        // 1/60 s, not the 30 Hz the design system animates at: baseview 0.4.1 paces redraws at the X Present `msc + 2`
        // (every other vblank), so a 1/30 s request draws about 20 frames a second and a 1/60 s one about 30
        // (Phase 0; docs/editor.md, lane AM).
        let repaint = self
            .live
            .schedule(&ctx, fingerprint.finish(), t.motion.scope_interval());
        self.report = FrameReport {
            regions,
            overflow,
            repaint,
        };
        let intents = out.take();
        self.apply(session, intents, pointer_down);
        // A sheet that opened or closed at the frame's end shows in the next pass.
        if self.presets.take_changed() {
            ctx.request_repaint();
        }
        // So does a change to the Recreate view (the mode, the drawer, the settings page; lane RUI).
        #[cfg(feature = "recreate")]
        if self.recreate.take_changed() {
            ctx.request_repaint();
        }
        // So do the Learn view's intents (lanes SKL, LUI): the shell applies them after this frame (`apply_learn`), and
        // the next pass shows what they did. Only after a click: never a timer.
        #[cfg(feature = "learn")]
        if self.learn.has_queued() {
            ctx.request_repaint();
        }
    }

    /// Apply a frame's intents in order: `Begin` opens a gesture, edits inside it merge into one undo step and one
    /// host gesture per parameter, `End` closes it; clicks, toggles, typed values and menu items are one edit each.
    /// An edit the session refuses changes nothing (its error is the session's `last_error`, shown on the status
    /// line). A gesture still open when no pointer button is held is closed: the control that opened it vanished.
    /// Adding, removing, moving, duplicating and renaming a module (lane AM) are one edit each, through the session's
    /// structural path (a recompiled plan); an add or a removal says what it did on the status line.
    pub fn apply(
        &mut self,
        session: &mut dyn SessionEdit,
        intents: Vec<Intent>,
        pointer_down: bool,
    ) {
        for intent in intents {
            match &intent {
                Intent::Preset(p) => self.presets.apply(session, p),
                #[cfg(feature = "recreate")]
                Intent::Game(g) => self.recreate.push(g.clone()),
                #[cfg(feature = "learn")]
                Intent::Learn(l) => self.learn.push(l.clone()),
                Intent::Begin(_) => self.gesture = Some(session.begin_gesture()),
                Intent::End(_) => {
                    session.end_gesture();
                    self.gesture = None;
                }
                Intent::ShowMore { card, open } => {
                    if *open {
                        self.more.insert(card.clone());
                    } else {
                        self.more.remove(card);
                    }
                }
                Intent::Undo => {
                    let _ = session.undo();
                }
                Intent::Redo => {
                    let _ = session.redo();
                }
                Intent::Set { .. } | Intent::SetMacro { .. } | Intent::SetDepth { .. } => {
                    let ops = intent.ops(session.patch());
                    if !ops.is_empty() {
                        self.said = None;
                        let _ = session.edit(ops, self.gesture);
                    }
                }
                _ => {
                    let ops = intent.ops(session.patch());
                    if !ops.is_empty() {
                        let note = add_module::note(session.patch(), &intent);
                        self.said = session.edit(ops, None).ok().and(note);
                    }
                }
            }
        }
        if self.gesture.is_some() && !pointer_down {
            session.end_gesture();
            self.gesture = None;
        }
        self.presets.settle(session, pointer_down);
    }

    /// Close any drag's gesture: at every open of the window (`build()`) and when it closes.
    pub fn end_drag(&mut self, session: &mut dyn SessionEdit) {
        session.end_gesture();
        self.gesture = None;
    }
}

/// Counts the live rings of `cards` that drew where they can be seen into this frame's fingerprint.
fn mark_cards(ctx: &Context, cards: &[CardView], fingerprint: &mut Fingerprint) {
    for card in cards {
        for p in card.drawn() {
            if let Some(live) = p.live {
                fingerprint.visible(ctx, ids::param(&p.addr), &p.addr, live);
            }
        }
    }
}

/// The built-in layout `learn` for `theme` (lane LUI), or layout 1 if it does not resolve (it always does: the
/// unit test in `layout.rs` checks every built-in).
#[cfg(feature = "learn")]
fn learn_layout(theme: &Theme) -> Layout {
    LayoutSpec::builtin("learn")
        .and_then(|spec| Layout::new(spec, theme).ok())
        .or_else(|| Layout::new(LayoutSpec::lanes(), theme).ok())
        .unwrap_or_else(|| unreachable!("the built-in layouts check"))
}
