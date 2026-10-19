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

use egui::{Rect, Ui, Vec2};
use enth_patch::Section;
use enth_patch::history::GestureId;
use enth_session::SessionEdit;

use super::cards::view::{self, CardView};
use super::cards::{CardKey, Intent, Intents};
use super::describe;
use super::layout::{Layout, LayoutError, LayoutSpec, Part, Placed, Region};
use super::overlay::{self, NoOverlay, Overlay};
use super::regions::{
    CardRegion, Header, HeaderView, MacroView, Macros, MatrixView, Overview, SourceView, Status,
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
    report: FrameReport,
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
            report: FrameReport::default(),
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

    /// The cards of one part of `patch`, with "More" state and the overlay applied.
    fn cards(&self, patch: &enth_patch::Patch, part: Part) -> Vec<CardView> {
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
        }
        overlay::apply(self.overlay.as_ref(), &mut cards);
        cards
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
            theme.clone_into(&mut self.resolved_for);
        }
        let t = theme;
        let rect = ui.max_rect();
        ui.painter()
            .rect_filled(rect, 0.0, t.color.surface.background);
        let regions = self.layout.regions(rect);
        let mut out = Intents::default();
        let mut overflow = Vec::new();
        let mut status = None;
        {
            let patch = session.patch();
            for placed in &regions {
                let r = placed.rect;
                match placed.region {
                    Region::Header => {
                        let history = session.history();
                        let header = HeaderView {
                            patch_name: patch.meta.name.clone(),
                            can_undo: history.can_undo(),
                            can_redo: history.can_redo(),
                            undo: history.next_undo().map(|e| describe::entry(patch, e)),
                            redo: history.next_redo().map(|e| describe::entry(patch, e)),
                        };
                        let knobs: Vec<CardView> = self
                            .layout
                            .parts_in(Region::Header)
                            .flat_map(|part| self.cards(patch, part))
                            .collect();
                        Header::new(t, &header).knobs(&knobs).show(ui, r, &mut out);
                    }
                    Region::Macros => {
                        Macros::new(t, &MacroView::all(patch)).show(ui, r, &mut out);
                    }
                    Region::Sources | Region::VoiceFx | Region::Modulators | Region::Effects => {
                        let cards: Vec<CardView> = self
                            .layout
                            .parts_in(placed.region)
                            .flat_map(|part| self.cards(patch, part))
                            .collect();
                        let shown =
                            CardRegion::new(t, placed.region.name(), &cards, placed.arrange)
                                .show(ui, r, &mut out);
                        overflow.push((placed.region.name(), shown.overflow));
                    }
                    Region::Overview => {
                        let sources = SourceView::all(patch);
                        let matrix = MatrixView::of(patch);
                        let shown = Overview::new(t, &sources, &matrix).show(ui, r, &mut out);
                        overflow.push(("overview.tiles", shown.tiles_overflow));
                        overflow.push(("overview.matrix", shown.matrix_overflow));
                    }
                    // Drawn last: it shows what the other regions report as hovered.
                    Region::Status => status = Some(r),
                    #[cfg(feature = "learn")]
                    Region::Lesson => super::reserved::show(ui, t, r, placed.region),
                    #[cfg(feature = "recreate")]
                    Region::ChallengeBar | Region::Results => {
                        super::reserved::show(ui, t, r, placed.region);
                    }
                }
            }
        }
        if let Some(r) = status {
            let error = session.status().last_error.map(|e| e.hint());
            let right = error.as_deref().unwrap_or(&self.notice);
            Status::new(t, out.hovered.as_ref())
                .right(right)
                .show(ui, r);
        }
        overflow.retain(|(_, o)| o.x > 0.5 || o.y > 0.5);
        self.report = FrameReport { regions, overflow };
        let intents = out.take();
        self.apply(session, intents, pointer_down);
    }

    /// Apply a frame's intents in order: `Begin` opens a gesture, edits inside it merge into one undo step and one
    /// host gesture per parameter, `End` closes it; clicks, toggles, typed values and menu items are one edit each.
    /// An edit the session refuses changes nothing (its error is the session's `last_error`, shown on the status
    /// line). A gesture still open when no pointer button is held is closed: the control that opened it vanished.
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
            session.end_gesture();
            self.gesture = None;
        }
    }

    /// Close any drag's gesture: at every open of the window (`build()`) and when it closes.
    pub fn end_drag(&mut self, session: &mut dyn SessionEdit) {
        session.end_gesture();
        self.gesture = None;
    }
}
