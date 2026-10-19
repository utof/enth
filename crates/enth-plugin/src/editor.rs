//! The `NiceEguiApp` that hosts the `enth-ui` editor in the plugin window: a translation, with no product rules
//! (lane SK for the first card; lane ED1 replaces the body with the full editor and its layout seam).
//!
//! F2b's editor is one real card: the first sound source's (`osc1` in the init patch), built by UI-C's
//! [`Card`] from the catalogue's `ParamSpec`s and bound to the session through [`SessionView`] and
//! [`SessionEdit`] only ([`Glue`]), so the same code runs in a test with a plain `Session`.
//!
//! Per frame, on baseview's GUI thread (docs/session.md, "Who calls what"): lock the session, `pump`, `fold_host`
//! with the host's values, draw from `SessionView::patch`, apply the frame's intents through `SessionEdit`, drain
//! the requests, **unlock**, then forward the host writes to nice-plug's `ParamSetter` (begin, set, end), so the host
//! records automation and never sees a call made under the lock. Every `build()` is a fresh open: it applies the
//! theme and ends any gesture the last window left open; `editor_closed` (called by lane FK's vendored
//! nice-plug-egui) ends it too and releases audition notes. Never `GuiContext::set_state`: an editor preset load
//! calls `Session::load_patch` under the lock.

use std::collections::BTreeSet;
use std::sync::Arc;

use enth_patch::history::GestureId;
use enth_session::{GestureStep, HostWrite, SessionEdit, ShellRequest};
use enth_ui::Theme;
use enth_ui::editor::cards::{Card, CardKey, CardView, Intent, Intents};
use nice_plug::context::gui::GuiContext;
use nice_plug_egui::baseview::HandlerError;
use nice_plug_egui::{
    EguiEditor, EguiEditorState, EguiNiceSettings, Frame, NiceEguiApp, RepaintNotifier,
    create_egui_editor,
};

use crate::Shared;
use crate::params::EnthParams;

/// The editor nice-plug spawns (the window's size comes from the theme's `size.window.default` token).
pub fn create(
    shared: Arc<Shared>,
    params: Arc<EnthParams>,
    state: Arc<EguiEditorState>,
    repaint: RepaintNotifier,
) -> Option<EguiEditor<EditorApp>> {
    create_egui_editor(
        state,
        repaint,
        EguiNiceSettings::new(),
        EditorApp {
            shared,
            params,
            theme: Theme::dark(),
            gui: None,
            glue: Glue::default(),
        },
    )
}

/// The editor's view state and gesture bookkeeping, and the code that turns intents into session calls. It knows
/// the session only through [`SessionEdit`].
#[derive(Debug, Default)]
pub struct Glue {
    /// The gesture a drag opened, until its `End` (or the pointer's release anywhere).
    gesture: Option<GestureId>,
    /// Cards whose "More" disclosure is open (editor view state, not in the patch).
    more: BTreeSet<CardKey>,
}

impl Glue {
    /// One frame: draw the first sound source's card from `session`'s patch, then apply its intents.
    /// `pointer_down`: whether any pointer button is held this frame (a drag released anywhere ends its gesture).
    pub fn frame(
        &mut self,
        ui: &mut egui::Ui,
        theme: &Theme,
        session: &mut dyn SessionEdit,
        pointer_down: bool,
    ) {
        let view = session
            .patch()
            .generators
            .first()
            .map(|m| CardView::module(session.patch(), m));
        let mut intents = Intents::default();
        if let Some(view) = view {
            let open = self.more.contains(&view.key);
            let view = view.with_more(open);
            Card::new(theme, &view).strip().show(ui, &mut intents);
        }
        self.apply(session, intents.take(), pointer_down);
    }

    /// Apply a frame's intents in order: `Begin` opens a gesture, edits inside it merge into one undo step and one
    /// host gesture per parameter, `End` closes it; clicks, toggles and menu items are one edit each. An edit the
    /// session refuses changes nothing (its error is the session's `last_error` for the status line).
    pub fn apply(
        &mut self,
        session: &mut dyn SessionEdit,
        intents: Vec<Intent>,
        pointer_down: bool,
    ) {
        for intent in intents {
            match &intent {
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
                        let _ = session.edit(ops, self.gesture);
                    }
                }
                _ => {
                    let ops = intent.ops(session.patch());
                    if !ops.is_empty() {
                        let _ = session.edit(ops, None);
                    }
                }
            }
        }
        if self.gesture.is_some() && !pointer_down {
            // The control that opened the drag vanished or the pointer left it: close the gesture anyway.
            session.end_gesture();
            self.gesture = None;
        }
    }

    /// Close any drag's gesture: at every `build()` (a fresh open) and when the window closes.
    pub fn end_drag(&mut self, session: &mut dyn SessionEdit) {
        session.end_gesture();
        self.gesture = None;
    }

    /// Whether a drag's gesture is open.
    pub fn gesture_open(&self) -> bool {
        self.gesture.is_some()
    }
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
        self.theme.apply(&egui_ctx);
        let glue = &mut self.glue;
        let ((), writes) = self.shared.with_session(|s| glue.end_drag(s));
        forward(&nice_gui_ctx, &self.params, writes);
        self.gui = Some(nice_gui_ctx);
        Ok(())
    }

    /// The window closed (called by the vendored nice-plug-egui, lane FK; `process()` also watches
    /// `EguiEditorState::is_open()`): end a drag, release audition notes, forward the `End`s, drop the context.
    fn editor_closed(&mut self) {
        let glue = &mut self.glue;
        let ((), writes) = self.shared.with_session(|s| {
            glue.end_drag(s);
            let _ = s.audition_all_off();
        });
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
        let ((), writes) = shared.with_session(|s| {
            s.pump();
            s.fold_host(&host);
            egui::CentralPanel::default().show(ui, |ui| {
                glue.frame(ui, theme, s, pointer_down);
            });
        });
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
        assert!(glue.more.contains(&key));
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
