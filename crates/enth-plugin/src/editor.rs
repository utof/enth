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
//! open; `editor_closed` (called by lane FK's vendored nice-plug-egui) ends it too and releases audition notes.
//! Never `GuiContext::set_state`: an editor preset load calls `Session::load_patch` under the lock.

use std::sync::Arc;

use enth_session::{GestureStep, HostWrite, SessionEdit, ShellRequest};
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
        self.theme.apply(&egui_ctx);
        let env = layout_from_env();
        let glue = &mut self.glue;
        let (saved, writes) = self.shared.with_session(|s| {
            glue.end_drag(s);
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
            // The root Ui covers the whole window; the editor paints its own background.
            glue.frame(ui, theme, s, pointer_down);
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
