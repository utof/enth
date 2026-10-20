//! The `NiceEguiApp` that hosts the `enth-ui` editor in the plugin window: a translation, with no product rules
//! (lane SK for the walking skeleton; lane ED1 for the full editor and its layout seam, docs/editor.md).
//!
//! The editor itself is [`enth_ui::editor::Editor`]: every region of a layout, drawn from the session through
//! [`SessionView`](enth_session::SessionView) and edited through [`SessionEdit`] only, so the same code runs here, in
//! the dev window and in kittest with a plain `Session`. [`Glue`] is the plugin's handle on it.
//!
//! Per frame, on baseview's GUI thread (docs/session.md, "Who calls what"): read the host's values, lock the
//! session, `pump`, `fold_host`, draw and apply the frame's intents ([`Glue::frame`]), drain the requests, **unlock**,
//! then forward the host writes to nice-plug's `ParamSetter` (begin, set, end), so the host records automation and
//! never sees a call made under the lock. Every `build()` is a fresh open: it applies the theme, picks the layout
//! (`ENTH_LAYOUT`, else the one saved in the `enth` field, else layout 1) and ends any gesture the last window left
//! open; `editor_closed` (called by lane FK's vendored nice-plug-egui) ends it too and releases audition notes. Every
//! `build()` also opens the editor's live feedback (a telemetry reader over the session's tap, lane ED2), and
//! `editor_closed` stops it; the editor asks egui for frames only while visible live data changes, and `process()`
//! turns the tap's wake flag into a repaint.
//! Never `GuiContext::set_state`: an editor preset load calls `Session::load_patch` under the lock.
//!
//! **A Recreate round** (lane SKR, docs/session.md "What lane SKR must wire" 1–2): every `build()` ticks the round's
//! clock with `EditorView::Opened` and hands the worker the window's egui context (it asks for a repaint each time it
//! posts a result); every frame, under the same lock and after `pump` and `fold_host`, ticks it with `Visible`, hands
//! the worker's results back (`Session::game_job_done`, a score held back while the blind rating is asked), gives
//! the editor the round's view, draws, and applies the Recreate bar's intents to the session's `GameEdit`; after the
//! frame it asks egui for a repaint at the round's `wake_at_ms`. Closing ticks it with `Hidden`, so the round's
//! time pauses while the window is closed.

use std::sync::Arc;

use enth_session::game::{EditorView, GameEdit};
use enth_session::{GestureStep, HostWrite, SessionEdit, SessionView, ShellRequest};
use enth_ui::Theme;
use enth_ui::editor::Editor;
use enth_ui::editor::cards::{CardKey, Intent};
use enth_ui::editor::layout::LayoutSpec;
use nice_plug::context::gui::GuiContext;
use nice_plug_egui::baseview::HandlerError;
use nice_plug_egui::{
    EguiEditor, EguiEditorState, EguiNiceSettings, Frame, NiceEguiApp, RepaintNotifier,
    create_egui_editor,
};

use crate::Shared;
use crate::game;
use crate::params::EnthParams;

/// The editor nice-plug spawns (the window's size comes from the theme's `size.window.default` token).
pub fn create(
    shared: Arc<Shared>,
    params: Arc<EnthParams>,
    state: Arc<EguiEditorState>,
    repaint: RepaintNotifier,
) -> Option<EguiEditor<EditorApp>> {
    let theme = Theme::dark();
    create_egui_editor(
        state,
        repaint,
        EguiNiceSettings::new(),
        EditorApp {
            shared,
            params,
            glue: Glue::new(&theme),
            theme,
            gui: None,
        },
    )
}

/// The plugin's handle on the editor: its view state (the open drag's gesture, "More" disclosures, the layout) and
/// the code that turns a frame's intents into session calls. It knows the session only through [`SessionEdit`].
#[derive(Debug, Default)]
pub struct Glue {
    editor: Editor,
}

impl Glue {
    /// The editor with layout 1 for `theme`.
    pub fn new(theme: &Theme) -> Self {
        Glue {
            editor: Editor::new(theme),
        }
    }

    /// One frame: every region of the layout drawn from `session`'s patch into `ui`'s rectangle, then the intents
    /// applied. `pointer_down`: whether any pointer button is held this frame (a drag released anywhere ends its
    /// gesture).
    pub fn frame(
        &mut self,
        ui: &mut egui::Ui,
        theme: &Theme,
        session: &mut dyn SessionEdit,
        pointer_down: bool,
    ) {
        self.editor.frame(ui, theme, session, pointer_down);
    }

    /// Apply a frame's intents in order (see [`Editor::apply`]).
    pub fn apply(
        &mut self,
        session: &mut dyn SessionEdit,
        intents: Vec<Intent>,
        pointer_down: bool,
    ) {
        self.editor.apply(session, intents, pointer_down);
    }

    /// Close any drag's gesture: at every `build()` (a fresh open) and when the window closes.
    pub fn end_drag(&mut self, session: &mut dyn SessionEdit) {
        self.editor.end_drag(session);
    }

    /// The window opened (every `build()`): live feedback from the session's telemetry tap starts (lane ED2).
    pub fn open(&mut self, session: &dyn SessionView, theme: &Theme) {
        self.editor.open(session, theme);
    }

    /// The window closed: the tap stops publishing and the editor asks for no more frames (lane ED2).
    pub fn close(&mut self) {
        self.editor.close();
    }

    /// Whether a drag's gesture is open.
    pub fn gesture_open(&self) -> bool {
        self.editor.gesture_open()
    }

    /// Whether `card`'s "More" disclosure is open.
    pub fn more_open(&self, card: &CardKey) -> bool {
        self.editor.more_open(card)
    }

    /// Pick the layout for a fresh open: `env` (what `ENTH_LAYOUT` named, already read), else the built-in `saved`
    /// in the `enth` field, else layout 1. A refused `ENTH_LAYOUT` is named on the status line.
    pub fn choose_layout(
        &mut self,
        theme: &Theme,
        env: Option<Result<LayoutSpec, String>>,
        saved: &str,
    ) {
        let saved = LayoutSpec::builtin(saved).unwrap_or_else(LayoutSpec::lanes);
        let (spec, mut notice) = match env {
            Some(Ok(spec)) => (spec, String::new()),
            Some(Err(e)) => (saved, format!("{e}; using the saved layout")),
            None => (saved, String::new()),
        };
        let name = spec.name.clone();
        if let Err(e) = self.editor.set_layout(spec, theme) {
            notice = format!("ENTH_LAYOUT {name}: {e}; using lanes");
            let _ = self.editor.set_layout(LayoutSpec::lanes(), theme);
        }
        self.editor.set_notice(notice);
    }

    /// The editor, for tests.
    pub fn editor(&self) -> &Editor {
        &self.editor
    }

    /// Before a frame (lane SKR): the session's round, for the Recreate bar.
    #[cfg(feature = "recreate")]
    pub fn set_round(&mut self, round: Option<enth_session::game::RoundView>) {
        self.editor.set_round(round);
    }

    /// After a frame (lane SKR): the Recreate bar's intents, applied to the session's round.
    #[cfg(feature = "recreate")]
    pub fn apply_game(&mut self, game: &mut dyn GameEdit) {
        self.editor.apply_game(game, &mut game::fresh_seed);
    }

    /// The worker's results to hand back now (a score waits while the bar asks the blind rating).
    #[cfg(feature = "recreate")]
    pub fn hold_scores(
        &mut self,
        results: Vec<(enth_session::game::JobId, enth_session::game::JobResult)>,
    ) -> Vec<(enth_session::game::JobId, enth_session::game::JobResult)> {
        self.editor.hold_scores(results)
    }

    /// Without the Recreate bar every result goes back at once.
    #[cfg(not(feature = "recreate"))]
    pub fn hold_scores(
        &mut self,
        results: Vec<(enth_session::game::JobId, enth_session::game::JobResult)>,
    ) -> Vec<(enth_session::game::JobId, enth_session::game::JobResult)> {
        results
    }

    /// One editor frame of a round's glue, under the session lock (lane SKR): the clock, the worker's results, the
    /// round view, the editor's frame, the bar's intents. Returns when the round next needs a frame (`wake_at_ms`).
    pub fn round_frame(
        &mut self,
        ui: &mut egui::Ui,
        theme: &Theme,
        session: &mut enth_session::Session,
        game: &game::Game,
        now_ms: u64,
        pointer_down: bool,
    ) -> Option<u64> {
        session.game_tick(now_ms, EditorView::Visible);
        for (id, result) in self.hold_scores(game.take_results()) {
            // A result the round no longer waits for is refused as stale, harmlessly.
            let _ = session.game_job_done(id, result);
        }
        game.follow_rate();
        #[cfg(feature = "recreate")]
        self.set_round(session.round());
        self.frame(ui, theme, session, pointer_down);
        #[cfg(feature = "recreate")]
        self.apply_game(session);
        session.round().and_then(|r| r.wake_at_ms)
    }
}

/// The layout `ENTH_LAYOUT` names, if it is set: a built-in's name ("lanes") or a layout file (JSON, schema
/// `enth.layout/1`). Read at every open of the window, so the owner can compare layouts in a host without a rebuild.
fn layout_from_env() -> Option<Result<LayoutSpec, String>> {
    let value = std::env::var_os("ENTH_LAYOUT")?;
    let shown = value.to_string_lossy().into_owned();
    if let Some(spec) = LayoutSpec::builtin(&shown) {
        return Some(Ok(spec));
    }
    Some(
        std::fs::read_to_string(&value)
            .map_err(|e| format!("ENTH_LAYOUT: cannot read {shown}: {e}"))
            .and_then(|text| {
                LayoutSpec::from_json(&text).map_err(|e| format!("ENTH_LAYOUT {shown}: {e}"))
            }),
    )
}

/// The plugin window's app.
pub struct EditorApp {
    shared: Arc<Shared>,
    params: Arc<EnthParams>,
    theme: Theme,
    gui: Option<GuiContext>,
    glue: Glue,
}

/// Forward host writes to nice-plug, **with the session unlocked**: `ParamSetter` may call into the host, which
/// may call back into the plugin on this thread.
fn forward(gui: &GuiContext, params: &EnthParams, writes: Vec<ShellRequest>) {
    let setter = gui.param_setter();
    for request in writes {
        if let ShellRequest::Host(HostWrite { param, step }) = request {
            let p = params.param(param);
            match step {
                GestureStep::Begin => setter.begin_set_parameter(p),
                GestureStep::Set(norm) => setter.set_parameter_normalized(p, norm),
                GestureStep::End => setter.end_set_parameter(p),
            }
        }
    }
}

impl NiceEguiApp for EditorApp {
    fn build(
        &mut self,
        egui_ctx: egui::Context,
        nice_gui_ctx: GuiContext,
        _frame: &mut Frame,
    ) -> Result<(), HandlerError> {
        #[cfg(feature = "editor-counts")]
        self.shared.editor_counts.built();
        self.theme.apply(&egui_ctx);
        // The round's worker asks this window for a repaint whenever it posts a result (lane SKR).
        self.shared.game.set_repaint(Some(egui_ctx.clone()));
        let env = layout_from_env();
        let glue = &mut self.glue;
        let theme = &self.theme;
        let (saved, writes) = self.shared.with_session(|s| {
            // Every build() is a fresh open: the time since the last tick never counts.
            s.game_tick(game::now_ms(), EditorView::Opened);
            glue.end_drag(s);
            glue.open(s, theme);
            s.instance().ui.layout.clone()
        });
        self.glue.choose_layout(&self.theme, env, &saved);
        forward(&nice_gui_ctx, &self.params, writes);
        self.gui = Some(nice_gui_ctx);
        Ok(())
    }

    /// The window closed (called by the vendored nice-plug-egui, lane FK; `process()` also watches
    /// `EguiEditorState::is_open()`): end a drag, release audition notes, forward the `End`s, drop the context.
    fn editor_closed(&mut self) {
        #[cfg(feature = "editor-counts")]
        self.shared.editor_counts.closed();
        let glue = &mut self.glue;
        let ((), writes) = self.shared.with_session(|s| {
            glue.end_drag(s);
            let _ = s.audition_all_off();
            // The round's time pauses while the window is closed (lane SKR).
            s.game_tick(game::now_ms(), EditorView::Hidden);
        });
        self.shared.game.set_repaint(None);
        glue.close();
        if let Some(gui) = self.gui.take() {
            forward(&gui, &self.params, writes);
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut Frame) {
        let host = self.params.host_values();
        let pointer_down = ui.input(|i| i.pointer.any_down());
        let EditorApp {
            shared,
            theme,
            glue,
            ..
        } = self;
        let now = game::now_ms();
        let (wake, writes) = shared.with_session(|s| {
            s.pump();
            s.fold_host(&host);
            // The round's clock and results, then the editor's frame (the root Ui covers the whole window; the editor
            // paints its own background) and the Recreate bar's intents (lane SKR).
            glue.round_frame(ui, theme, s, &shared.game, now, pointer_down)
        });
        // An idle editor draws nothing, so the round asks for its next frame itself: settled jobs, the timer's next
        // second (F1r review S11). The worker's posts request their own repaints.
        if let Some(at) = wake {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(at.saturating_sub(now)));
        }
        // Lane AM: count the drawn frame, and whether it asked for the next (live data), for owner check 1.3.
        #[cfg(feature = "editor-counts")]
        self.shared
            .editor_counts
            .frame(self.glue.editor().report().repaint.is_some());
        if let Some(gui) = &self.gui {
            forward(gui, &self.params, writes);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use enth_patch::{ParamAddr, Patch};
    use enth_session::Session;
    use enth_ui::editor::cards::{Grip, ids};

    fn level() -> ParamAddr {
        ParamAddr::module(Patch::init().generators[0].id.clone(), "level").unwrap()
    }

    #[test]
    fn a_drag_is_one_gesture_one_undo_step() {
        let mut session = Session::new(Patch::init());
        let mut glue = Glue::default();
        let addr = level();
        let mut intents = vec![Intent::Begin(Grip::Param(addr.clone()))];
        for v in [10.0, 20.0, 30.0] {
            intents.push(Intent::Set {
                addr: addr.clone(),
                value: v,
            });
        }
        glue.apply(&mut session, intents, true);
        assert!(glue.gesture_open());
        assert!(session.status().gesture_open);
        glue.apply(
            &mut session,
            vec![Intent::End(Grip::Param(addr.clone()))],
            false,
        );
        assert!(!glue.gesture_open());
        assert_eq!(session.patch().value(&addr), Some(30.0));
        let step = session.history().next_undo().unwrap();
        assert_eq!(step.forward.len(), 1, "one merged step: {step:?}");
    }

    #[test]
    fn a_release_anywhere_ends_the_gesture_and_clicks_are_single_edits() {
        let mut session = Session::new(Patch::init());
        let mut glue = Glue::default();
        let addr = level();
        glue.apply(
            &mut session,
            vec![
                Intent::Begin(Grip::Param(addr.clone())),
                Intent::Set {
                    addr: addr.clone(),
                    value: 55.0,
                },
            ],
            false,
        );
        assert!(!glue.gesture_open());
        assert!(!session.status().gesture_open);
        let wave = ParamAddr::module(Patch::init().generators[0].id.clone(), "wave").unwrap();
        glue.apply(
            &mut session,
            vec![Intent::Select {
                addr: wave.clone(),
                index: 2,
            }],
            false,
        );
        assert_eq!(session.patch().value(&wave), Some(2.0));
        assert_eq!(session.history().next_undo().unwrap().gesture, None);
    }

    #[test]
    fn show_more_is_view_state_not_an_edit() {
        let mut session = Session::new(Patch::init());
        let mut glue = Glue::default();
        let key = CardKey::Module(Patch::init().generators[0].id.clone());
        glue.apply(
            &mut session,
            vec![Intent::ShowMore {
                card: key.clone(),
                open: true,
            }],
            false,
        );
        assert!(glue.more_open(&key));
        assert!(!session.history().can_undo());
    }

    /// Runs one headless egui frame of the card over `session`.
    fn frame(
        ctx: &egui::Context,
        glue: &mut Glue,
        session: &mut Session,
        events: Vec<egui::Event>,
    ) {
        let theme = Theme::dark();
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                theme.size.window.default,
            )),
            events,
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            let down = ui.input(|i| i.pointer.any_down());
            glue.frame(ui, &theme, session, down);
        });
        // Headless: nothing uploads textures.
        output.textures_delta.clear();
    }

    #[test]
    fn opening_the_window_enables_the_tap_and_closing_it_stops_it() {
        let theme = Theme::dark();
        let mut session = Session::new(Patch::init());
        let engine = session
            .activate(enth_session::Activation {
                sample_rate: 48_000.0,
                offline: false,
            })
            .unwrap();
        let tap = engine.tap().unwrap();
        assert!(!tap.is_enabled(), "closed: the tap does not publish");
        let mut glue = Glue::new(&theme);
        glue.open(&session, &theme);
        assert!(tap.is_enabled(), "build() opens live feedback");
        glue.close();
        assert!(!tap.is_enabled(), "editor_closed stops it");
        // A window opened while the plugin is inactive picks the tap up when the session has one.
        let mut detached = Session::new(Patch::init());
        glue.open(&detached, &theme);
        let engine = detached
            .activate(enth_session::Activation {
                sample_rate: 48_000.0,
                offline: false,
            })
            .unwrap();
        let ctx = egui::Context::default();
        theme.apply(&ctx);
        frame(&ctx, &mut glue, &mut detached, Vec::new());
        assert!(engine.tap().unwrap().is_enabled());
    }

    #[test]
    fn the_layout_comes_from_enth_layout_then_the_saved_name_then_lanes() {
        let theme = Theme::dark();
        let mut glue = Glue::new(&theme);
        glue.choose_layout(&theme, None, "lanes");
        assert_eq!(glue.editor().layout().name(), "lanes");
        // A saved name this build does not know falls back to layout 1.
        glue.choose_layout(&theme, None, "from-a-newer-build");
        assert_eq!(glue.editor().layout().name(), "lanes");
        // A file that names a layout wins.
        let mut spec = LayoutSpec::lanes();
        "mine".clone_into(&mut spec.name);
        glue.choose_layout(&theme, Some(Ok(spec)), "lanes");
        assert_eq!(glue.editor().layout().name(), "mine");
        // A file that does not check leaves layout 1 and says why on the status line.
        let mut broken = LayoutSpec::lanes();
        broken.place.clear();
        glue.choose_layout(&theme, Some(Ok(broken)), "lanes");
        assert_eq!(glue.editor().layout().name(), "lanes");
        glue.choose_layout(
            &theme,
            Some(Err("ENTH_LAYOUT: cannot read x".into())),
            "lanes",
        );
        assert_eq!(glue.editor().layout().name(), "lanes");
    }

    #[test]
    fn dragging_the_cards_level_knob_edits_the_patch_through_the_session() {
        let ctx = egui::Context::default();
        Theme::dark().apply(&ctx);
        let mut session = Session::new(Patch::init());
        let mut glue = Glue::default();
        let addr = level();
        let before = session.patch().value(&addr).unwrap();
        frame(&ctx, &mut glue, &mut session, Vec::new());
        frame(&ctx, &mut glue, &mut session, Vec::new());
        let knob = ctx
            .read_response(ids::param(&addr))
            .expect("the level knob was drawn under its stable id");
        let at = knob.rect.center();
        let press = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame(
            &ctx,
            &mut glue,
            &mut session,
            vec![egui::Event::PointerMoved(at), press(true)],
        );
        for dy in 1..=8 {
            let to = at + egui::vec2(0.0, 6.0 * dy as f32);
            frame(
                &ctx,
                &mut glue,
                &mut session,
                vec![egui::Event::PointerMoved(to)],
            );
        }
        assert!(glue.gesture_open(), "the drag holds one gesture");
        frame(
            &ctx,
            &mut glue,
            &mut session,
            vec![egui::Event::PointerButton {
                pos: at + egui::vec2(0.0, 48.0),
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        frame(&ctx, &mut glue, &mut session, Vec::new());
        assert!(!glue.gesture_open());
        let after = session.patch().value(&addr).unwrap();
        // The level starts at its maximum (100 %): dragging down lowers it.
        assert!(
            after < before,
            "dragging down lowers the level: {before} -> {after}"
        );
        let step = session.history().next_undo().unwrap();
        assert_eq!(
            step.forward.len(),
            1,
            "one undo step for the drag: {step:?}"
        );
    }
}
