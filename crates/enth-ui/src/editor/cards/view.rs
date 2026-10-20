//! The read model: what a card shows, built from the patch and the catalogue every frame (report 02 measured the
//! whole editor's at about 5 % of a frame, so nothing is cached). A card is a module instance or a built-in; a
//! parameter is a `ParamSpec` with its address and plain value. Nothing here is written per module kind.

use enth_params::{ParamSpec, Tier};
use enth_patch::catalogue::Builtin;
use enth_patch::{
    ModSource, ModuleId, ModuleInstance, ModuleKind, ParamAddr, Patch, Section, SlotIndex, Target,
};

use super::present;
use crate::editor::add_module::Place;
use crate::editor::modulate::DropCheck;
use crate::editor::overlay::Emphasis;
use crate::theme::{Category, Score};

/// Which card: a module by its id, or one of the built-ins. Stable across moves, relabels and layouts.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CardKey {
    Module(ModuleId),
    Voice,
    Master,
}

/// What a card presents.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CardKind {
    Module(ModuleKind),
    Voice,
    Master,
}

/// The tiers a card shows before its "More" disclosure is opened: basic and intermediate, so an oscillator shows
/// its eight sound-design controls and keeps the three phase controls behind "More" (the layout handoff's "~8
/// visible + 3 advanced").
pub const VISIBLE_TIER: Tier = Tier::Intermediate;

/// One route into a parameter, for the status line.
#[derive(Clone, Debug, PartialEq)]
pub struct RouteView {
    pub src: ModSource,
    /// The source's short name ("LFO 1", "Vel", "M1").
    pub name: String,
    /// Percent of the destination's range.
    pub depth: f32,
}

/// One parameter of a card.
#[derive(Clone, Debug, PartialEq)]
pub struct ParamView {
    pub addr: ParamAddr,
    pub spec: &'static ParamSpec,
    /// The plain value in the patch.
    pub value: f32,
    /// The modulated value the engine uses now, normalised, when it differs (the engine's telemetry, lane ED2):
    /// drawn as the knob's modulation arc.
    pub live: Option<f32>,
    /// The routes into this parameter, in summation order.
    pub routes: Vec<RouteView>,
    /// The span its routes can reach, normalised (`enth_patch::modulation_range`), when they move it at all: the
    /// knob's static modulation arc (lane ED2).
    pub range: Option<(f32, f32)>,
    /// While a source's ⊕ is dragged: what dropping it here would do (lane ED2, `editor::modulate`).
    pub drop: Option<DropCheck>,
    /// The host automation slots that drive it, in slot order (the "Automate" menu and the status line).
    pub slots: Vec<SlotIndex>,
    /// How strongly it is shown (the editor's [`Overlay`](crate::editor::overlay::Overlay); normal in P1b).
    pub emphasis: Emphasis,
    /// A score ring from a challenge's results (none in P1b).
    pub score: Option<Score>,
}

impl ParamView {
    /// The value normalised by the spec's own scale (power, log or linear).
    pub fn norm(&self) -> f32 {
        self.spec.to_norm(self.value)
    }
}

/// One card.
#[derive(Clone, Debug, PartialEq)]
pub struct CardView {
    pub key: CardKey,
    pub kind: CardKind,
    /// The module's label, else its short kind name and number ("Osc 2"), or "Voice" / "Master".
    pub title: String,
    /// The module's enable switch; `None` for the built-ins, which can't be bypassed.
    pub enabled: Option<bool>,
    /// This envelope is the voice's amplitude envelope.
    pub amp_env: bool,
    /// Every parameter, in spec order.
    pub params: Vec<ParamView>,
    /// Parameters up to this tier show; the rest wait behind "More".
    pub visible: Tier,
    /// "More" is open (editor view state the editor keeps, toggled by [`super::Intent::ShowMore`]).
    pub more_open: bool,
    /// Where a module sits in its section, for the card's "…" menu (lane AM); `None` for the built-ins, which have
    /// no menu.
    pub place: Option<Place>,
}

impl CardView {
    /// The card of a module in `patch`.
    pub fn module(patch: &Patch, module: &ModuleInstance) -> Self {
        let params = module
            .kind
            .params()
            .iter()
            .zip(&module.values)
            .filter_map(|(spec, &value)| {
                let addr = ParamAddr::module(module.id.clone(), spec.key).ok()?;
                Some(param_view(patch, addr, spec, value))
            })
            .collect();
        CardView {
            key: CardKey::Module(module.id.clone()),
            kind: CardKind::Module(module.kind),
            title: module
                .label
                .clone()
                .unwrap_or_else(|| title(module.kind, &module.id)),
            enabled: Some(module.enabled),
            amp_env: patch.voice.amp_env == module.id,
            params,
            visible: VISIBLE_TIER,
            more_open: false,
            place: Place::of(patch, &module.id),
        }
    }

    /// The built-in `voice` card.
    pub fn voice(patch: &Patch) -> Self {
        Self::builtin(patch, Builtin::Voice)
    }

    /// The built-in `master` card.
    pub fn master(patch: &Patch) -> Self {
        Self::builtin(patch, Builtin::Master)
    }

    fn builtin(patch: &Patch, builtin: Builtin) -> Self {
        let (key, kind, title, target) = match builtin {
            Builtin::Voice => (CardKey::Voice, CardKind::Voice, "Voice", Target::Voice),
            Builtin::Master => (CardKey::Master, CardKind::Master, "Master", Target::Master),
        };
        let params = builtin
            .params()
            .iter()
            .filter_map(|spec| {
                let addr = ParamAddr::new(target.clone(), spec.key).ok()?;
                let value = patch.value(&addr)?;
                Some(param_view(patch, addr, spec, value))
            })
            .collect();
        CardView {
            key,
            kind,
            title: title.to_owned(),
            enabled: None,
            amp_env: false,
            params,
            visible: VISIBLE_TIER,
            more_open: false,
            place: None,
        }
    }

    /// With "More" open or closed.
    #[must_use]
    pub fn with_more(mut self, open: bool) -> Self {
        self.more_open = open;
        self
    }

    /// The parameter with this key.
    pub fn param(&self, key: &str) -> Option<&ParamView> {
        self.params.iter().find(|p| p.spec.key == key)
    }

    /// The parameters shown before "More".
    pub fn shown(&self) -> impl Iterator<Item = &ParamView> {
        self.params
            .iter()
            .filter(|p| p.spec.learn.tier <= self.visible)
    }

    /// The parameters behind "More".
    pub fn hidden(&self) -> impl Iterator<Item = &ParamView> {
        self.params
            .iter()
            .filter(|p| p.spec.learn.tier > self.visible)
    }

    /// Whether the card has a "More" disclosure.
    pub fn has_more(&self) -> bool {
        self.hidden().next().is_some()
    }

    /// The parameters drawn now: the shown ones, and the hidden ones when "More" is open.
    pub fn drawn(&self) -> impl Iterator<Item = &ParamView> {
        self.shown().chain(self.hidden().filter(|_| self.more_open))
    }

    /// The category stripe.
    pub fn category(&self) -> Category {
        present::category(self.kind)
    }

    /// The text after the title: the header enum's item ("Saw", "Low-pass"), or "Amp env" for the amplitude
    /// envelope.
    pub fn kind_text(&self) -> Option<&'static str> {
        if self.amp_env {
            return Some("Amp env");
        }
        let key = present::header_enum(self.kind)?;
        let p = self.param(key)?;
        p.spec.enum_item(p.value).map(|i| i.name)
    }

    /// "Filter 1 › Cutoff".
    pub fn path(&self, spec: &ParamSpec) -> String {
        format!("{} {} {}", self.title, crate::theme::glyph::PATH, spec.name)
    }
}

fn param_view(patch: &Patch, addr: ParamAddr, spec: &'static ParamSpec, value: f32) -> ParamView {
    let routes = patch
        .routes
        .iter()
        .filter(|r| r.dst == addr)
        .map(|r| RouteView {
            name: source_name(patch, &r.src),
            src: r.src.clone(),
            depth: r.depth,
        })
        .collect();
    let slots = enth_session::bridge::slots_for(patch, &addr).collect();
    ParamView {
        addr,
        spec,
        value,
        live: None,
        routes,
        range: None,
        drop: None,
        slots,
        emphasis: Emphasis::Normal,
        score: None,
    }
}

/// "osc2" → "Osc 2". An id that isn't the kind's prefix and a number is shown as it is.
pub(crate) fn title(kind: ModuleKind, id: &ModuleId) -> String {
    let short = present::short_name(kind);
    match id.as_str().strip_prefix(kind.spec().id_prefix) {
        Some(n) if !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) => format!("{short} {n}"),
        _ => id.as_str().to_owned(),
    }
}

/// A modulation source's short name: the module's card title ("Env 2", "LFO 1"), "Vel", "Key", "Wheel", "M3".
pub fn source_name(patch: &Patch, src: &ModSource) -> String {
    match src {
        ModSource::Module(id) => match patch.find(id) {
            Some((_, m)) => m.label.clone().unwrap_or_else(|| title(m.kind, id)),
            None => id.as_str().to_owned(),
        },
        ModSource::Velocity => "Vel".to_owned(),
        ModSource::Key => "Key".to_owned(),
        ModSource::ModWheel => "Wheel".to_owned(),
        ModSource::Macro(i) => format!("M{}", i.one_based()),
    }
}

/// The cards of one section, in order.
pub fn section(patch: &Patch, section: Section) -> Vec<CardView> {
    patch
        .section(section)
        .iter()
        .map(|m| CardView::module(patch, m))
        .collect()
}

/// Every card: the four sections in file order, then `voice` and `master`.
pub fn all(patch: &Patch) -> Vec<CardView> {
    let mut cards: Vec<CardView> = Section::ALL
        .into_iter()
        .flat_map(|s| section(patch, s))
        .collect();
    cards.push(CardView::voice(patch));
    cards.push(CardView::master(patch));
    cards
}
