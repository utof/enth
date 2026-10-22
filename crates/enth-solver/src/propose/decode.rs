//! **Owner: lane F3C** (the model contract, freeze point F3s); lane ONX builds inference on it. Pre-declared by lane
//! S0s (R3).
//!
//! The model's **output layout** and the mapping between its vectors and patches (docs/r3.md, "The model contract
//! (F3s)"). Patches store plain values in real units; the normalised 0–1 domain of each [`ParamSpec`] exists only
//! at this boundary (AGENTS.md).
//!
//! # Layout 1 (`propose-layout/1`)
//!
//! What the model predicts is **generator v1's draw set on template 0** (`enth_challenge::priors::v1`'s table over
//! the frozen starter superset), the label space of `ds1`, which reuses v1's draws: the parameters v1 can draw, the
//! four modules v1 can switch on (structure), and the routes v1 can add, as a fixed source × destination grid (no
//! slot symmetry). Every hypothesis is one row of [`OutputLayout::width`] `f32`s, in five segments:
//!
//! | Segment | Entries | Each is | Decoded as |
//! | --- | --- | --- | --- |
//! | continuous | the `Float` parameters | the [`ParamSpec`] normalised value, 0–1 | clamped to 0–1, then [`ParamSpec::from_norm`] |
//! | categorical | one group of logits per `Enum` parameter, one logit per item | a logit | the arg-max item (ties: the lower index) |
//! | structure | one per module the template has switched off | a logit | on when above 0 |
//! | route presence | one per grid cell, source-major | a logit | a route when above 0 and its depth is not 0 |
//! | route depth | one per grid cell, the same order | `(depth / 100 + 1) / 2`, 0–1 | `depth = (2n − 1) · 100` % of the destination's range |
//!
//! **The order is built from the template and the catalogue** ([`OutputLayout::build`]): the built-in `voice`
//! first, then the template's modules in file order (generators, voice effects, effects, modulators), each in its
//! kind's spec order; route sources are the template's modules in that order, then the built-in sources (velocity,
//! key, mod wheel, macros). The layout's sets are the lists below; adding a parameter elsewhere in the catalogue
//! moves nothing.
//!
//! # The layout hash (the "catalogue hash")
//!
//! [`OutputLayout::canonical_text`] writes one line per entry with **exactly what decoding reads**: the address,
//! the kind, range, scale, unit and items of its spec, the template's value, the offsets; plus the destination
//! spec of every route cell. Its SHA-256 ([`OutputLayout::sha256`]) is what a model file's header carries and what
//! [`pack`](super::pack) checks against this build. So a catalogue edit to a field a layout entry reads, a renamed
//! key, or a changed template value changes the hash and refuses old models; a new parameter, module kind or enum
//! item elsewhere does not. `layout-v1.txt` pins the text (`the_v1_layout_is_pinned`).
//!
//! # Decoding and labels
//!
//! Both read a [`Hearing`]: the key the model is told (the intake's, never a `--perf` override), its pitch offset,
//! and the phrase the patch is heard through, built at run time (the F3s judge's M1–M3, S8).
//!
//! - **Labels** ([`OutputLayout::encode`], for `tools/datagen`, lane DG) canonicalise under **the sample's phrase**
//!   ([`Hearing::sample`]): a route the phrase cannot reveal is removed, then every layout parameter it cannot reveal
//!   goes back to the template's value. Then **the semis are re-keyed to the told key**: `osc1.semi` and `osc2.semi`
//!   move by (the phrase's first key − the told key), so two sounds the model cannot tell apart (a semi of +12 at C3,
//!   a semi of 0 at C4) get one label; a re-key that leaves ±48 semitones, or meets a heard keytrack (which follows
//!   the played key, not the semis), is refused ([`LayoutError::Rekey`]). The loss mask has no loss on what the
//!   phrase does not reveal and, when the first note-off falls at or after the model's 2 s window
//!   ([`WINDOW_S`](super::features::WINDOW_S)), none on the envelopes' releases, which the model cannot see.
//! - **Decoding** ([`OutputLayout::decode`], lane MS) writes a hypothesis onto the template (structure, categoricals,
//!   values, routes), canonicalises it under **the job's performance** ([`Hearing::job`]: the key the job plays, its
//!   hold and its scored window, so a long attack the job reveals is kept), re-keys the semis from the told key to
//!   the job's key (they differ only under a `--perf` override), and sets `osc1.fine` from the pitch offset:
//!   `100 · context[1]` cents, within ±50 (layout 1's **context entry**), so a proposal starts in tune.
//!
//! So a label is exactly what decoding gives back when the job's performance is the sample's phrase (structure,
//! categoricals and routes exact; values within the `f32` rounding of the normalised domain; tested under the held
//! phrase, a 2.8 s pad and a 0.3 s note). Symmetric slots (the two LFOs with their route rows) are left to the loss
//! (lane TR: the minimum over the two permutations). Under one note, 14 of the 102 values never reach a decoded
//! patch (`voice.mode`, `voice.glide`, `filter1.keytrack`, `env1.velocity` and the velocity row of the route grid):
//! they stay in the layout for phrases that reveal them.
//!
//! **Render what you label.** Canonicalising resets what the phrase cannot reveal, which is not always silent (a
//! keytrack under one note is a cutoff move; a velocity amount under one velocity scales a depth): `ds1` renders the
//! patch [`OutputLayout::canonicalise`] leaves under the sample's phrase (generator v1 already draws only what the
//! phrase reveals; projected presets need it).

use core::fmt;

use enth_challenge::model::{Phrase, PhraseError, PhraseNote, PhraseView};
use enth_challenge::random::{self, Visibility, observable_view};
use enth_params::{Kind, ParamSpec};
use enth_patch::addr::Target;
use enth_patch::{
    Builtin, ModSource, ModuleId, ModuleKind, Op, ParamAddr, Patch, Route, RouteAddr, apply,
};
use sha2::{Digest, Sha256};

use super::features::{Cue, WINDOW_S};
use crate::create::TargetPerf;

/// Layout 1: generator v1's draw set on template 0 (see the [module docs](self)).
pub const LAYOUT_V1: u16 = 1;

/// The newest layout this build knows. A model naming a newer one is refused as too new; layouts are only added.
pub const LATEST_LAYOUT: u16 = 1;

/// Layout 1's stable name, the first line of its canonical text.
pub const LAYOUT_V1_NAME: &str = "propose-layout/1";

/// The generator whose draw set layout 1 is.
pub const LAYOUT_V1_GENERATOR: u16 = 1;

/// The generator template layout 1 decodes onto (the frozen starter superset).
pub const LAYOUT_V1_TEMPLATE: u8 = 0;

/// The parameters of layout 1: every address generator v1's table draws (`priors::v1`'s `Param` slots). Their
/// order here is not significant: [`OutputLayout::build`] orders them by the template and the catalogue.
const PARAMS_V1: [&str; 40] = [
    "voice.mode",
    "voice.glide",
    "osc1.wave",
    "osc1.semi",
    "osc1.pw",
    "osc1.position",
    "osc2.wave",
    "osc2.semi",
    "osc2.fine",
    "osc2.level",
    "osc2.pw",
    "osc2.position",
    "noise1.color",
    "noise1.level",
    "filter1.mode",
    "filter1.cutoff",
    "filter1.resonance",
    "filter1.drive",
    "filter1.keytrack",
    "env1.attack",
    "env1.decay",
    "env1.sustain",
    "env1.release",
    "env1.velocity",
    "env2.attack",
    "env2.decay",
    "env2.sustain",
    "env2.release",
    "lfo1.rate",
    "lfo1.shape",
    "lfo2.rate",
    "lfo2.shape",
    "reverb1.wet",
    "reverb1.decay",
    "reverb1.room",
    "reverb1.high_cut",
    "comp1.threshold",
    "comp1.ratio",
    "comp1.attack",
    "comp1.release",
];

/// The modules generator v1 may switch on (`Enable` slots): the structure heads.
const ENABLES_V1: [&str; 4] = ["osc2", "noise1", "reverb1", "comp1"];

/// The route sources generator v1 draws (`Route` slots).
const ROUTE_SOURCES_V1: [&str; 4] = ["velocity", "env2", "lfo1", "lfo2"];

/// Every destination a v1 route can reach (velocity and env 2 reach the cutoff; the LFOs one of these four).
const ROUTE_DESTINATIONS_V1: [&str; 4] = ["filter1.cutoff", "osc1.fine", "osc1.level", "osc1.pw"];

/// The values relative to the told key: labels add (the phrase's first key − the told key), decoding subtracts it.
const REKEYED_V1: [&str; 2] = ["osc1.semi", "osc2.semi"];

/// The parameter that follows the played key, not the semis: a re-key that meets it heard is refused.
const KEYTRACK_V1: &str = "filter1.keytrack";

/// Layout 1's context entry: decoded from `context[1]`, never predicted (the F3s judge's S7).
const CONTEXT_FINE_V1: &str = "osc1.fine";

/// The context entry's range (cents): `100 · context[1]` is within it whenever the key is the nearest note.
pub const FINE_LIMIT_CENTS: f32 = 50.0;

/// One parameter of the layout: a continuous value or a categorical group.
#[derive(Clone, Debug, PartialEq)]
pub struct ParamEntry {
    /// The parameter.
    pub addr: ParamAddr,
    /// Its spec in this build's catalogue (what normalises it).
    pub spec: &'static ParamSpec,
    /// The template's value (plain units): what an entry the phrase does not reveal is reset to.
    pub template: f32,
    /// Its index in a hypothesis row (a categorical group's first logit).
    pub offset: usize,
}

impl ParamEntry {
    /// The number of `f32`s it takes: 1 for a continuous value, one logit per item for an `Enum`, 2 for a
    /// `Toggle`.
    #[must_use]
    pub fn len(&self) -> usize {
        match self.spec.kind {
            Kind::Enum(items) => items.len(),
            Kind::Toggle => 2,
            _ => 1,
        }
    }

    /// Always `false`: every entry takes at least one value (for clippy's `len_without_is_empty`).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        false
    }
}

/// One structure head: a module the template has switched off.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnableEntry {
    /// The module.
    pub id: ModuleId,
    /// Its index in a hypothesis row.
    pub offset: usize,
}

/// One cell of the route grid.
#[derive(Clone, Debug, PartialEq)]
pub struct RouteCell {
    /// The source.
    pub src: ModSource,
    /// The destination.
    pub dst: ParamAddr,
    /// The destination's spec (a depth is a percentage of its normalised range).
    pub dst_spec: &'static ParamSpec,
    /// The presence logit's index in a hypothesis row.
    pub presence: usize,
    /// The normalised depth's index in a hypothesis row.
    pub depth: usize,
}

/// A layout built against this build's catalogue and template (see the [module docs](self)).
#[derive(Clone, Debug, PartialEq)]
pub struct OutputLayout {
    id: u16,
    generator: u16,
    template_id: u8,
    template: Patch,
    continuous: Vec<ParamEntry>,
    categorical: Vec<ParamEntry>,
    enables: Vec<EnableEntry>,
    routes: Vec<RouteCell>,
    /// The continuous entries re-keyed to the told key (layout order).
    rekeyed: Vec<ParamEntry>,
    /// The keytrack a re-key may not meet heard.
    keytrack: ParamAddr,
    /// The context entry (`osc1.fine`): its spec and the template's value.
    fine: (ParamAddr, &'static ParamSpec, f32),
    /// The envelopes' releases (layout order): no loss when the model's window holds no note-off.
    releases: Vec<ParamEntry>,
    width: usize,
    text: String,
    sha256: [u8; 32],
}

/// How a hypothesis is decoded or a label is read (see the [module docs](self)): the key the model is told, its
/// pitch offset, and the phrase the patch is heard through, built at run time.
#[derive(Clone, Debug, PartialEq)]
pub struct Hearing {
    told_key: u8,
    pitch_offset: Option<f64>,
    notes: Vec<PhraseNote>,
    length_ms: u32,
    on_s: f64,
}

/// Why a [`Hearing`] cannot be built.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum HearingError {
    /// The phrase is not valid (no notes, a key above 127, a velocity of 0 or above 127, a note outside it).
    Phrase(PhraseError),
    /// The told key is above 127.
    Key(u8),
    /// A note-on, hold or length that is negative or not finite, or a note-on at or after the end.
    Times,
}

impl HearingError {
    /// A stable snake-case code.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            HearingError::Phrase(_) => "hearing_phrase",
            HearingError::Key(_) => "hearing_key",
            HearingError::Times => "hearing_times",
        }
    }
}

impl fmt::Display for HearingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HearingError::Phrase(e) => write!(f, "the phrase is not valid: {e:?}"),
            HearingError::Key(k) => write!(f, "the told key {k} is above 127"),
            HearingError::Times => f.write_str(
                "a note-on, hold or length is negative or not finite, or the note starts after the end",
            ),
        }
    }
}

impl std::error::Error for HearingError {}

fn ms(seconds: f64) -> u32 {
    libm::round(seconds * 1000.0).clamp(0.0, f64::from(u32::MAX)) as u32
}

impl Hearing {
    /// A `ds1` sample (lane DG): the patch was rendered through `notes` over `length_ms` (ms from the phrase's
    /// start, which is the clip's onset), and the model is told `cue` (the intake's, on the degraded clip). The
    /// onset in the clip is `cue.on_s`.
    ///
    /// # Errors
    /// [`HearingError`] for an invalid phrase, a cue key above 127 or a bad note-on.
    pub fn sample(
        cue: &Cue,
        notes: &[PhraseNote],
        length_ms: u32,
    ) -> Result<Hearing, HearingError> {
        let view = PhraseView { notes, length_ms };
        view.validate().map_err(HearingError::Phrase)?;
        Hearing::new(cue, notes.to_vec(), length_ms, cue.on_s)
    }

    /// A create job (lane MS): the model was told `cue` (the intake's), and the job renders `perf` (the intake's,
    /// or a `--perf` override): one note on `perf.key` at `perf.velocity`, held `perf.hold_s` from `perf.on_s`, in
    /// a render of `perf.duration_s` (the scored window). Times round to whole milliseconds.
    ///
    /// # Errors
    /// [`HearingError`] for a key above 127, a negative or non-finite time, or a note-on at or after the end.
    pub fn job(cue: &Cue, perf: &TargetPerf) -> Result<Hearing, HearingError> {
        let times_ok = perf.on_s.is_finite()
            && perf.on_s >= 0.0
            && perf.hold_s.is_finite()
            && perf.hold_s >= 0.0
            && perf.duration_s.is_finite()
            && perf.duration_s > perf.on_s;
        if !times_ok {
            return Err(HearingError::Times);
        }
        let velocity = if perf.velocity.is_finite() {
            libm::roundf(perf.velocity * 127.0).clamp(1.0, 127.0) as u8
        } else {
            100
        };
        let notes = vec![PhraseNote {
            key: perf.key,
            velocity,
            start_ms: 0,
            hold_ms: ms(perf.hold_s),
        }];
        let length_ms = ms(perf.duration_s - perf.on_s).max(1);
        PhraseView {
            notes: &notes,
            length_ms,
        }
        .validate()
        .map_err(HearingError::Phrase)?;
        Hearing::new(cue, notes, length_ms, perf.on_s)
    }

    /// A table phrase heard as itself: the told key is its first key (no re-key), no pitch offset, the onset at
    /// the clip's start. For tests and tools.
    #[must_use]
    pub fn of_phrase(phrase: &Phrase) -> Hearing {
        Hearing {
            told_key: phrase.notes.first().map_or(48, |n| n.key),
            pitch_offset: None,
            notes: phrase.notes.to_vec(),
            length_ms: phrase.length_ms,
            on_s: 0.0,
        }
    }

    fn new(
        cue: &Cue,
        notes: Vec<PhraseNote>,
        length_ms: u32,
        on_s: f64,
    ) -> Result<Hearing, HearingError> {
        if cue.key > 127 {
            return Err(HearingError::Key(cue.key));
        }
        if !(on_s.is_finite() && on_s >= 0.0) {
            return Err(HearingError::Times);
        }
        Ok(Hearing {
            told_key: cue.key,
            pitch_offset: cue.pitch_offset(),
            notes,
            length_ms,
            on_s,
        })
    }

    /// The key the model is told (`context[0] + 60`).
    #[must_use]
    pub fn told_key(&self) -> u8 {
        self.told_key
    }

    /// `context[1]` (semitones), when the clip is pitched.
    #[must_use]
    pub fn pitch_offset(&self) -> Option<f64> {
        self.pitch_offset
    }

    /// The phrase the patch is heard through.
    #[must_use]
    pub fn phrase(&self) -> PhraseView<'_> {
        PhraseView {
            notes: &self.notes,
            length_ms: self.length_ms,
        }
    }

    /// The phrase's first key − the told key (semitones): what labels add to the semis and decoding subtracts.
    #[must_use]
    pub fn shift(&self) -> i32 {
        i32::from(self.notes.first().map_or(self.told_key, |n| n.key)) - i32::from(self.told_key)
    }

    /// The first note-off in clip time (s): the onset plus the earliest note-off of the phrase.
    #[must_use]
    pub fn first_note_off_s(&self) -> f64 {
        let off = self
            .notes
            .iter()
            .map(|n| n.start_ms.saturating_add(n.hold_ms))
            .min()
            .unwrap_or(0);
        self.on_s + f64::from(off) / 1000.0
    }

    /// Whether the model's window ([`WINDOW_S`]) holds a note-off, so a release can be seen in it.
    #[must_use]
    pub fn release_in_window(&self) -> bool {
        self.first_note_off_s() < WINDOW_S
    }
}

/// Why a layout cannot be built, or a patch is not on it.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum LayoutError {
    /// Layout 0 (no writer makes it), or a layout this build does not know: newer than [`LATEST_LAYOUT`].
    Layout {
        /// The layout asked for.
        layout: u16,
    },
    /// A layout asked for with a generator or template that is not its own.
    Pairing {
        /// The layout.
        layout: u16,
        /// The generator asked for.
        generator: u16,
        /// The template asked for.
        template: u8,
    },
    /// The catalogue or the template no longer has an address the layout lists (a renamed key, a removed module):
    /// this build cannot decode the layout, so its models are refused.
    Missing {
        /// The address.
        addr: String,
    },
    /// The patch is not on the layout: its modules differ from the template's, a parameter outside the layout
    /// differs from the template's value, or a route is off the grid (`ds1` projects its patches first).
    OffLayout {
        /// What differs.
        what: String,
    },
    /// Re-keying the label to the told key fails: a semi leaves ±48 semitones, or a heard keytrack would follow
    /// the played key, not the semis (the sample is skipped and counted).
    Rekey {
        /// Why.
        what: String,
    },
}

impl LayoutError {
    /// A stable snake-case code.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            LayoutError::Layout { .. } => "unknown_layout",
            LayoutError::Pairing { .. } => "layout_pairing",
            LayoutError::Missing { .. } => "layout_missing",
            LayoutError::OffLayout { .. } => "off_layout",
            LayoutError::Rekey { .. } => "rekey",
        }
    }
}

impl fmt::Display for LayoutError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LayoutError::Layout { layout } => write!(
                f,
                "output layout {layout} is not one this build knows (1..={LATEST_LAYOUT})"
            ),
            LayoutError::Pairing {
                layout,
                generator,
                template,
            } => write!(
                f,
                "output layout {layout} is not defined for generator {generator}, template {template}"
            ),
            LayoutError::Missing { addr } => write!(
                f,
                "this build's catalogue or template has no {addr}, which the layout lists"
            ),
            LayoutError::OffLayout { what } => write!(f, "the patch is not on the layout: {what}"),
            LayoutError::Rekey { what } => {
                write!(f, "the label cannot be re-keyed to the told key: {what}")
            }
        }
    }
}

impl std::error::Error for LayoutError {}

/// Why a hypothesis row does not decode.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum DecodeError {
    /// The row is not [`OutputLayout::width`] long.
    Width {
        /// The layout's width.
        expected: usize,
        /// The row's length.
        got: usize,
    },
    /// A value is NaN or infinite: the hypothesis is dropped, never guessed.
    NonFinite {
        /// Its index in the row.
        index: usize,
    },
    /// Re-keying from the told key to the job's key moves a semi past ±48 semitones (only under a `--perf`
    /// override far from the intake's key): the hypothesis is dropped.
    Rekey {
        /// The parameter.
        addr: String,
        /// The semitones it moves by (the told key − the job's key).
        by: i32,
    },
    /// An op on the template failed (a layout this build built cannot produce this; reported, not hidden).
    Op(String),
}

impl DecodeError {
    /// A stable snake-case code.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            DecodeError::Width { .. } => "hypothesis_width",
            DecodeError::NonFinite { .. } => "hypothesis_non_finite",
            DecodeError::Rekey { .. } => "hypothesis_rekey",
            DecodeError::Op(_) => "hypothesis_op",
        }
    }
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DecodeError::Width { expected, got } => {
                write!(
                    f,
                    "a hypothesis has {got} values; the layout has {expected}"
                )
            }
            DecodeError::NonFinite { index } => {
                write!(f, "hypothesis value {index} is not finite")
            }
            DecodeError::Rekey { addr, by } => write!(
                f,
                "re-keyed to the job's key ({by:+} semitones), {addr} leaves -48..48"
            ),
            DecodeError::Op(why) => write!(f, "decoding a hypothesis failed: {why}"),
        }
    }
}

impl std::error::Error for DecodeError {}

/// A training label: one row in the layout, and which of its values carry a loss.
#[derive(Clone, Debug, PartialEq)]
pub struct Labels {
    /// The target row: normalised values, one-hot categorical groups, 0 or 1 for structure and route presence.
    pub values: Vec<f32>,
    /// 1 where the value carries a loss: a parameter the phrase reveals (an envelope's release only when the model's
    /// window holds a note-off), every structure head and route presence, a route depth where the route is present;
    /// else 0.
    pub mask: Vec<u8>,
}

fn missing(addr: &str) -> LayoutError {
    LayoutError::Missing {
        addr: addr.to_owned(),
    }
}

fn op_error(e: impl fmt::Display) -> DecodeError {
    DecodeError::Op(e.to_string())
}

/// The rank of a route source: the template's modules in file order, then the built-in sources.
fn source_rank(template: &Patch, src: &ModSource) -> Option<usize> {
    let modules = template.modules().count();
    match src {
        ModSource::Module(id) => template.modules().position(|(_, m)| &m.id == id),
        ModSource::Velocity => Some(modules),
        ModSource::Key => Some(modules + 1),
        ModSource::ModWheel => Some(modules + 2),
        ModSource::Macro(i) => Some(modules + 3 + i.one_based()),
    }
}

/// Every parameter address of `template` in layout order: `voice`, then the modules in file order, each in spec
/// order (`master` last).
fn template_order(template: &Patch) -> Vec<ParamAddr> {
    let mut out = Vec::new();
    for spec in Builtin::Voice.params() {
        if let Ok(a) = ParamAddr::new(Target::Voice, spec.key) {
            out.push(a);
        }
    }
    for (_, m) in template.modules() {
        for spec in m.kind.params() {
            if let Ok(a) = ParamAddr::module(m.id.clone(), spec.key) {
                out.push(a);
            }
        }
    }
    for spec in Builtin::Master.params() {
        if let Ok(a) = ParamAddr::new(Target::Master, spec.key) {
            out.push(a);
        }
    }
    out
}

fn spec_line(spec: &ParamSpec) -> String {
    let kind = match spec.kind {
        Kind::Float { min, max, scale } => format!("float {min:?}..{max:?} {scale:?}"),
        Kind::Int { min, max } => format!("int {min}..{max}"),
        Kind::Enum(items) => format!(
            "enum [{}]",
            items.iter().map(|i| i.key).collect::<Vec<_>>().join(",")
        ),
        Kind::Toggle => "toggle".to_owned(),
    };
    let norm = [0.0f32, 0.25, 0.5, 0.75, 1.0].map(|n| spec.from_norm(n));
    format!("{kind} unit={:?} norm={norm:?}", spec.unit)
}

impl OutputLayout {
    /// The layout this build decodes for models of `layout`, `generator` and `template`, built from the catalogue
    /// and the template (see the [module docs](self)).
    ///
    /// # Errors
    /// [`LayoutError::Layout`] for a layout this build does not know, [`LayoutError::Pairing`] when the generator
    /// or template is not the layout's, [`LayoutError::Missing`] when the catalogue or template lost an address.
    pub fn build(layout: u16, generator: u16, template: u8) -> Result<OutputLayout, LayoutError> {
        if layout != LAYOUT_V1 {
            return Err(LayoutError::Layout { layout });
        }
        if generator != LAYOUT_V1_GENERATOR || template != LAYOUT_V1_TEMPLATE {
            return Err(LayoutError::Pairing {
                layout,
                generator,
                template,
            });
        }
        let start = random::start(template).map_err(|_| missing("the template"))?;
        OutputLayout::from_lists(
            layout,
            generator,
            template,
            start,
            &PARAMS_V1,
            &ENABLES_V1,
            &ROUTE_SOURCES_V1,
            &ROUTE_DESTINATIONS_V1,
        )
    }

    /// Layout 1 as this build decodes it.
    ///
    /// # Panics
    /// If this build's catalogue cannot build layout 1 (`the_v1_layout_is_pinned` fails first).
    #[must_use]
    pub fn v1() -> OutputLayout {
        OutputLayout::build(LAYOUT_V1, LAYOUT_V1_GENERATOR, LAYOUT_V1_TEMPLATE)
            .expect("this build's catalogue builds layout 1 (tested)")
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "one argument per list of the layout's definition"
    )]
    fn from_lists(
        id: u16,
        generator: u16,
        template_id: u8,
        template: Patch,
        params: &[&str],
        enables: &[&str],
        sources: &[&str],
        destinations: &[&str],
    ) -> Result<OutputLayout, LayoutError> {
        let wanted = params
            .iter()
            .map(|a| a.parse::<ParamAddr>().map_err(|_| missing(a)))
            .collect::<Result<Vec<_>, _>>()?;
        let order = template_order(&template);
        if let Some(a) = wanted.iter().find(|a| !order.contains(a)) {
            return Err(missing(&a.to_string()));
        }
        let ordered: Vec<ParamAddr> = order.into_iter().filter(|a| wanted.contains(a)).collect();
        let mut offset = 0;
        let mut continuous = Vec::new();
        for addr in &ordered {
            let spec = template
                .spec(addr)
                .ok_or_else(|| missing(&addr.to_string()))?;
            if matches!(spec.kind, Kind::Float { .. } | Kind::Int { .. }) {
                let value = template
                    .value(addr)
                    .ok_or_else(|| missing(&addr.to_string()))?;
                continuous.push(ParamEntry {
                    addr: addr.clone(),
                    spec,
                    template: value,
                    offset,
                });
                offset += 1;
            }
        }
        let mut categorical = Vec::new();
        for addr in &ordered {
            let spec = template
                .spec(addr)
                .ok_or_else(|| missing(&addr.to_string()))?;
            if matches!(spec.kind, Kind::Enum(_) | Kind::Toggle) {
                let value = template
                    .value(addr)
                    .ok_or_else(|| missing(&addr.to_string()))?;
                let entry = ParamEntry {
                    addr: addr.clone(),
                    spec,
                    template: value,
                    offset,
                };
                offset += entry.len();
                categorical.push(entry);
            }
        }
        let enable_ids = enables
            .iter()
            .map(|id| ModuleId::new(id).map_err(|_| missing(id)))
            .collect::<Result<Vec<_>, _>>()?;
        let mut enable_entries = Vec::new();
        for (_, m) in template.modules() {
            if enable_ids.contains(&m.id) {
                if m.enabled {
                    return Err(LayoutError::Missing {
                        addr: format!("{} switched off in the template", m.id.as_str()),
                    });
                }
                enable_entries.push(EnableEntry {
                    id: m.id.clone(),
                    offset,
                });
                offset += 1;
            }
        }
        if enable_entries.len() != enable_ids.len() {
            return Err(missing("a structure module"));
        }
        let mut srcs = sources
            .iter()
            .map(|s| {
                let src = s.parse::<ModSource>().map_err(|_| missing(s))?;
                let rank = source_rank(&template, &src).ok_or_else(|| missing(s))?;
                Ok((rank, src))
            })
            .collect::<Result<Vec<_>, LayoutError>>()?;
        srcs.sort_by_key(|(rank, _)| *rank);
        let dst_order = template_order(&template);
        let mut dsts = destinations
            .iter()
            .map(|d| {
                let addr = d.parse::<ParamAddr>().map_err(|_| missing(d))?;
                let rank = dst_order
                    .iter()
                    .position(|a| a == &addr)
                    .ok_or_else(|| missing(d))?;
                let spec = template.spec(&addr).ok_or_else(|| missing(d))?;
                Ok((rank, addr, spec))
            })
            .collect::<Result<Vec<_>, LayoutError>>()?;
        dsts.sort_by_key(|(rank, _, _)| *rank);
        let cells = srcs.len() * dsts.len();
        let mut routes = Vec::with_capacity(cells);
        for (_, src) in &srcs {
            for (_, dst, spec) in &dsts {
                let k = routes.len();
                routes.push(RouteCell {
                    src: src.clone(),
                    dst: dst.clone(),
                    dst_spec: spec,
                    presence: offset + k,
                    depth: offset + cells + k,
                });
            }
        }
        let width = offset + 2 * cells;
        let entry = |addr: &str| {
            continuous
                .iter()
                .find(|e| e.addr.to_string() == addr)
                .cloned()
                .ok_or_else(|| missing(addr))
        };
        let rekeyed = REKEYED_V1
            .iter()
            .map(|a| entry(a))
            .collect::<Result<Vec<_>, _>>()?;
        let keytrack = entry(KEYTRACK_V1)?.addr;
        let fine_addr = CONTEXT_FINE_V1
            .parse::<ParamAddr>()
            .map_err(|_| missing(CONTEXT_FINE_V1))?;
        let fine_spec = template
            .spec(&fine_addr)
            .ok_or_else(|| missing(CONTEXT_FINE_V1))?;
        let fine_template = template
            .value(&fine_addr)
            .ok_or_else(|| missing(CONTEXT_FINE_V1))?;
        let releases: Vec<ParamEntry> = continuous
            .iter()
            .filter(|e| {
                e.addr.param() == "release"
                    && e.addr.module_id().is_some_and(|id| {
                        template
                            .find(id)
                            .is_some_and(|(_, m)| m.kind == ModuleKind::Adsr)
                    })
            })
            .cloned()
            .collect();
        let mut layout = OutputLayout {
            id,
            generator,
            template_id,
            template,
            continuous,
            categorical,
            enables: enable_entries,
            routes,
            rekeyed,
            keytrack,
            fine: (fine_addr, fine_spec, fine_template),
            releases,
            width,
            text: String::new(),
            sha256: [0; 32],
        };
        layout.text = layout.write_text();
        layout.sha256 = Sha256::digest(layout.text.as_bytes()).into();
        Ok(layout)
    }

    fn write_text(&self) -> String {
        use core::fmt::Write as _;
        let mut t = String::new();
        let _ = writeln!(t, "{LAYOUT_V1_NAME}");
        let _ = writeln!(
            t,
            "generator {} template {} width {}",
            self.generator, self.template_id, self.width
        );
        let _ = writeln!(
            t,
            "row: a non-finite value drops the hypothesis; a subnormal value reads as 0"
        );
        let _ = writeln!(
            t,
            "continuous {}: clamped to 0..1, then from_norm",
            self.continuous.len()
        );
        for e in &self.continuous {
            let _ = writeln!(
                t,
                "{} {} {} template={:?}",
                e.offset,
                e.addr,
                spec_line(e.spec),
                e.template
            );
        }
        let _ = writeln!(
            t,
            "categorical {}: the arg-max logit, ties to the lower item",
            self.categorical.len()
        );
        for e in &self.categorical {
            let _ = writeln!(
                t,
                "{}..{} {} {} template={:?}",
                e.offset,
                e.offset + e.len(),
                e.addr,
                spec_line(e.spec),
                e.template
            );
        }
        let _ = writeln!(
            t,
            "structure {}: on when the logit is above 0",
            self.enables.len()
        );
        for e in &self.enables {
            let kind = self.template.find(&e.id).map_or("?", |(_, m)| m.kind.key());
            let _ = writeln!(t, "{} {} {kind} template=off", e.offset, e.id.as_str());
        }
        let _ = writeln!(
            t,
            "routes {}: present when the logit is above 0 and the depth is not 0; depth = (2n - 1) * 100 % of the destination's range",
            self.routes.len()
        );
        for c in &self.routes {
            let _ = writeln!(
                t,
                "{} {} {}->{} depth % of {}",
                c.presence,
                c.depth,
                c.src,
                c.dst,
                spec_line(c.dst_spec)
            );
        }
        let names = |es: &[ParamEntry]| {
            es.iter()
                .map(|e| e.addr.to_string())
                .collect::<Vec<_>>()
                .join(" ")
        };
        let (fine, fine_spec, fine_template) = &self.fine;
        let _ = writeln!(
            t,
            "context {fine} = 100 * context[1] clamped to {:?}..{FINE_LIMIT_CENTS:?} when pitched, else the template's: {} template={fine_template:?}",
            -FINE_LIMIT_CENTS,
            spec_line(fine_spec)
        );
        let _ = writeln!(
            t,
            "told {} relative to the told key: labels add the first key - the told key, decoding subtracts the job's key - the told key; refused past the range or with {} heard",
            names(&self.rekeyed),
            self.keytrack
        );
        let _ = writeln!(
            t,
            "window {WINDOW_S:?} s: no loss on {} when the first note-off is at or after it",
            names(&self.releases)
        );
        let _ = writeln!(
            t,
            "canonical labels under the sample's phrase, decoding under the job's performance"
        );
        t
    }

    /// The layout's id ([`LAYOUT_V1`]).
    #[must_use]
    pub fn id(&self) -> u16 {
        self.id
    }

    /// The generator whose draw set it is.
    #[must_use]
    pub fn generator(&self) -> u16 {
        self.generator
    }

    /// The template it decodes onto (its id).
    #[must_use]
    pub fn template_id(&self) -> u8 {
        self.template_id
    }

    /// The template patch it decodes onto.
    #[must_use]
    pub fn template(&self) -> &Patch {
        &self.template
    }

    /// The continuous entries, in row order.
    #[must_use]
    pub fn continuous(&self) -> &[ParamEntry] {
        &self.continuous
    }

    /// The categorical groups, in row order.
    #[must_use]
    pub fn categorical(&self) -> &[ParamEntry] {
        &self.categorical
    }

    /// The structure heads, in row order.
    #[must_use]
    pub fn enables(&self) -> &[EnableEntry] {
        &self.enables
    }

    /// The route grid's cells, source-major.
    #[must_use]
    pub fn routes(&self) -> &[RouteCell] {
        &self.routes
    }

    /// The `f32`s in one hypothesis row.
    #[must_use]
    pub fn width(&self) -> usize {
        self.width
    }

    /// The canonical text (see the [module docs](self)).
    #[must_use]
    pub fn canonical_text(&self) -> &str {
        &self.text
    }

    /// SHA-256 of [`canonical_text`](Self::canonical_text): the hash a model file's header carries.
    #[must_use]
    pub fn sha256(&self) -> [u8; 32] {
        self.sha256
    }

    /// One hypothesis row as a patch on the template, decoded under `hearing` ([`Hearing::job`]: the job's
    /// performance and the intake's cue; see the [module docs](self)): structure, categoricals, values and routes;
    /// canonicalised under the job's phrase; the semis re-keyed from the told key to the job's key; `osc1.fine` from
    /// the pitch offset.
    ///
    /// A subnormal value reads as 0, so the result is the same with FTZ or DAZ on.
    ///
    /// # Errors
    /// [`DecodeError::Width`], [`DecodeError::NonFinite`] and [`DecodeError::Rekey`] (the hypothesis is dropped),
    /// [`DecodeError::Op`].
    pub fn decode(&self, row: &[f32], hearing: &Hearing) -> Result<Patch, DecodeError> {
        if row.len() != self.width {
            return Err(DecodeError::Width {
                expected: self.width,
                got: row.len(),
            });
        }
        if let Some(index) = row.iter().position(|v| !v.is_finite()) {
            return Err(DecodeError::NonFinite { index });
        }
        // A subnormal reads as 0 (a bit test, not a float operation), so no comparison below depends on DAZ.
        let row: Vec<f32> = row
            .iter()
            .map(|v| if v.is_subnormal() { 0.0 } else { *v })
            .collect();
        let mut p = self.template.clone();
        for e in &self.enables {
            if row[e.offset] > 0.0 {
                let op = Op::SetEnabled {
                    id: e.id.clone(),
                    enabled: true,
                };
                apply(&mut p, op).map_err(op_error)?;
            }
        }
        for e in &self.categorical {
            let logits = &row[e.offset..e.offset + e.len()];
            let mut best = 0;
            for (i, v) in logits.iter().enumerate() {
                if *v > logits[best] {
                    best = i;
                }
            }
            let op = Op::SetParam {
                addr: e.addr.clone(),
                value: best as f32,
            };
            apply(&mut p, op).map_err(op_error)?;
        }
        for e in &self.continuous {
            let op = Op::SetParam {
                addr: e.addr.clone(),
                value: e.spec.from_norm(row[e.offset].clamp(0.0, 1.0)),
            };
            apply(&mut p, op).map_err(op_error)?;
        }
        for c in &self.routes {
            let depth = (2.0 * row[c.depth].clamp(0.0, 1.0) - 1.0) * 100.0;
            if row[c.presence] > 0.0 && depth != 0.0 {
                let op = Op::InsertRoute {
                    index: p.routes.len(),
                    route: Route {
                        src: c.src.clone(),
                        dst: c.dst.clone(),
                        depth,
                    },
                };
                apply(&mut p, op).map_err(op_error)?;
            }
        }
        let phrase = hearing.phrase();
        self.canonicalise(&mut p, phrase).map_err(op_error)?;
        let shift = hearing.shift();
        if shift != 0 {
            let mask = observable_view(&p, phrase);
            for e in self.rekeyed.iter().filter(|e| mask.is_observable(&e.addr)) {
                let value = p.value(&e.addr).unwrap_or(e.template) - shift as f32;
                if !(e.spec.min()..=e.spec.max()).contains(&value) {
                    return Err(DecodeError::Rekey {
                        addr: e.addr.to_string(),
                        by: -shift,
                    });
                }
                let op = Op::SetParam {
                    addr: e.addr.clone(),
                    value,
                };
                apply(&mut p, op).map_err(op_error)?;
            }
        }
        let (fine, _, fine_template) = &self.fine;
        let cents = hearing.pitch_offset().map_or(*fine_template, |o| {
            ((100.0 * o) as f32).clamp(-FINE_LIMIT_CENTS, FINE_LIMIT_CENTS)
        });
        if p.value(fine) != Some(cents) {
            let op = Op::SetParam {
                addr: fine.clone(),
                value: cents,
            };
            apply(&mut p, op).map_err(op_error)?;
        }
        Ok(p)
    }

    /// Reset what `phrase` cannot reveal, in two passes: every route whose depth the phrase does not reveal is
    /// removed, then every layout parameter it does not reveal (in a switched-off module, dead, a gauge) is set to
    /// the template's value. Decoding does this under the job's phrase, labels under the sample's ([`Hearing`]);
    /// `ds1` renders what it leaves under the sample's phrase (render what you label).
    ///
    /// # Errors
    /// An op that fails on `patch` (it is not on the template).
    pub fn canonicalise(
        &self,
        patch: &mut Patch,
        phrase: PhraseView<'_>,
    ) -> Result<(), enth_patch::OpError> {
        let mask = observable_view(patch, phrase);
        let unheard: Vec<RouteAddr> = patch
            .routes
            .iter()
            .map(|r| RouteAddr {
                src: r.src.clone(),
                dst: r.dst.clone(),
            })
            .filter(|r| !mask.route(r).is_some_and(Visibility::is_observable))
            .collect();
        for route in unheard {
            apply(&mut *patch, Op::RemoveRoute { route })?;
        }
        let mask = observable_view(patch, phrase);
        for e in self.continuous.iter().chain(&self.categorical) {
            if !mask.is_observable(&e.addr) && patch.value(&e.addr) != Some(e.template) {
                apply(
                    &mut *patch,
                    Op::SetParam {
                        addr: e.addr.clone(),
                        value: e.template,
                    },
                )?;
            }
        }
        Ok(())
    }

    /// The training label of `patch` heard as `hearing` says ([`Hearing::sample`]): canonicalised under the sample's
    /// phrase (as [`canonicalise`](Self::canonicalise)), the semis re-keyed to the told key, then read into a row,
    /// with the loss mask (see [`Labels`]).
    ///
    /// # Errors
    /// [`LayoutError::OffLayout`] when the patch is not on the layout: other modules than the template's, a module
    /// the template has on switched off, or, among what the phrase reveals, a parameter outside the layout away
    /// from the template's value (`osc1.fine` may be anywhere within ±50 cents: decoding sets it from the pitch
    /// offset) or a route off the grid. [`LayoutError::Rekey`] when a re-keyed semi leaves ±48 semitones or a
    /// heard keytrack is not 0. What the phrase cannot reveal (a gauge such as `master.level`, which generator v1's
    /// headroom rule lowers; a nuisance; a dead or switched-off parameter; an unheard route) is not checked:
    /// canonicalising drops it.
    pub fn encode(&self, patch: &Patch, hearing: &Hearing) -> Result<Labels, LayoutError> {
        self.check_modules(patch)?;
        let phrase = hearing.phrase();
        let mut p = patch.clone();
        self.canonicalise(&mut p, phrase)
            .map_err(|e| LayoutError::OffLayout {
                what: e.to_string(),
            })?;
        let mask = observable_view(&p, phrase);
        self.check_heard(&p, &mask)?;
        let shift = hearing.shift();
        if shift != 0 {
            let rekey = |what: String| LayoutError::Rekey { what };
            if mask.is_observable(&self.keytrack) && p.value(&self.keytrack) != Some(0.0) {
                return Err(rekey(format!(
                    "{} is heard and follows the played key, not the semis",
                    self.keytrack
                )));
            }
            for e in self.rekeyed.iter().filter(|e| mask.is_observable(&e.addr)) {
                let value = p.value(&e.addr).unwrap_or(e.template) + shift as f32;
                if !(e.spec.min()..=e.spec.max()).contains(&value) {
                    return Err(rekey(format!(
                        "{} would be {value} semitones, outside -48..48",
                        e.addr
                    )));
                }
                apply(
                    &mut p,
                    Op::SetParam {
                        addr: e.addr.clone(),
                        value,
                    },
                )
                .map_err(|e| rekey(e.to_string()))?;
            }
        }
        let mut values = vec![0.0f32; self.width];
        let mut loss = vec![0u8; self.width];
        for e in &self.continuous {
            let v = p.value(&e.addr).unwrap_or(e.template);
            values[e.offset] = e.spec.to_norm(v);
            loss[e.offset] = u8::from(mask.is_observable(&e.addr));
        }
        if !hearing.release_in_window() {
            for e in &self.releases {
                loss[e.offset] = 0;
            }
        }
        for e in &self.categorical {
            let v = p.value(&e.addr).unwrap_or(e.template);
            let item = (v.max(0.0) as usize).min(e.len() - 1);
            values[e.offset + item] = 1.0;
            let heard = u8::from(mask.is_observable(&e.addr));
            loss[e.offset..e.offset + e.len()].fill(heard);
        }
        for e in &self.enables {
            let on = p.find(&e.id).is_some_and(|(_, m)| m.enabled);
            values[e.offset] = if on { 1.0 } else { 0.0 };
            loss[e.offset] = 1;
        }
        for c in &self.routes {
            let route = p.routes.iter().find(|r| r.src == c.src && r.dst == c.dst);
            loss[c.presence] = 1;
            if let Some(r) = route {
                values[c.presence] = 1.0;
                values[c.depth] = f32::midpoint(r.depth / 100.0, 1.0);
                loss[c.depth] = 1;
            } else {
                values[c.depth] = 0.5;
            }
        }
        Ok(Labels { values, mask: loss })
    }

    /// `patch` holds the template's modules: ids, kinds, sections, and on wherever the template is on (bar the
    /// structure heads).
    fn check_modules(&self, patch: &Patch) -> Result<(), LayoutError> {
        let off = |what: String| LayoutError::OffLayout { what };
        let mine: Vec<_> = patch.modules().map(|(p, m)| (p, &m.id, m.kind)).collect();
        let theirs: Vec<_> = self
            .template
            .modules()
            .map(|(p, m)| (p, &m.id, m.kind))
            .collect();
        if mine != theirs {
            return Err(off("its modules are not the template's".to_owned()));
        }
        for (_, m) in self.template.modules() {
            let switchable = self.enables.iter().any(|e| e.id == m.id);
            let on = patch.find(&m.id).is_some_and(|(_, p)| p.enabled);
            if m.enabled && !on && !switchable {
                return Err(off(format!("{} is switched off", m.id.as_str())));
            }
        }
        Ok(())
    }

    /// Among what `mask` says the phrase reveals, every parameter outside the layout is at the template's value, and
    /// every route is on the grid (a canonical patch keeps only routes the phrase reveals).
    fn check_heard(
        &self,
        patch: &Patch,
        mask: &enth_challenge::random::Mask,
    ) -> Result<(), LayoutError> {
        let off = |what: String| LayoutError::OffLayout { what };
        let layout_params: Vec<&ParamAddr> = self
            .continuous
            .iter()
            .chain(&self.categorical)
            .map(|e| &e.addr)
            .collect();
        let (fine, _, _) = &self.fine;
        for addr in template_order(&self.template) {
            if layout_params.contains(&&addr) || !mask.is_observable(&addr) {
                continue;
            }
            let value = patch.value(&addr);
            if &addr == fine {
                if value.is_none_or(|v| v.abs() > FINE_LIMIT_CENTS) {
                    return Err(off(format!(
                        "{addr} is beyond {FINE_LIMIT_CENTS} cents (decoding sets it from the pitch offset)"
                    )));
                }
            } else if value != self.template.value(&addr) {
                return Err(off(format!("{addr} is not the template's value")));
            }
        }
        if let Some(r) = patch
            .routes
            .iter()
            .find(|r| !self.routes.iter().any(|c| c.src == r.src && c.dst == r.dst))
        {
            return Err(off(format!(
                "the route {}->{} is off the grid",
                r.src, r.dst
            )));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use enth_challenge::phrases::{self, HELD, OCTAVE_PAIR, PhraseFacts};
    use enth_challenge::spec::{Category, RandomSpec};

    /// The held phrase heard as itself (no re-key, no pitch offset).
    fn held() -> Hearing {
        Hearing::of_phrase(phrases::phrase(HELD).unwrap())
    }

    fn cue(key: u8) -> Cue {
        Cue {
            key,
            f0_hz: None,
            on_s: 0.0,
            hold_s: 1.5,
            bandwidth_hz: None,
        }
    }

    fn note(key: u8, hold_ms: u32) -> PhraseNote {
        PhraseNote {
            key,
            velocity: 100,
            start_ms: 0,
            hold_ms,
        }
    }

    /// A job playing what `notes` (one note) and `length_ms` describe, told `cue`.
    fn job_like(cue: &Cue, notes: &[PhraseNote], length_ms: u32) -> Hearing {
        let n = notes[0];
        let perf = TargetPerf {
            key: n.key,
            velocity: f32::from(n.velocity) / 127.0,
            on_s: cue.on_s,
            hold_s: f64::from(n.hold_ms) / 1000.0,
            duration_s: cue.on_s + f64::from(length_ms) / 1000.0,
            f0_hz: None,
        };
        Hearing::job(cue, &perf).unwrap()
    }

    fn set(p: &mut Patch, addr: &str, value: f32) {
        apply(
            p,
            Op::SetParam {
                addr: addr.parse().unwrap(),
                value,
            },
        )
        .unwrap();
    }

    fn entry<'a>(l: &'a OutputLayout, addr: &str) -> &'a ParamEntry {
        l.continuous()
            .iter()
            .chain(l.categorical())
            .find(|e| e.addr.to_string() == addr)
            .unwrap()
    }

    fn v1_target(seed: u64) -> Patch {
        let spec = RandomSpec {
            generator: 1,
            seed,
            difficulty: 100,
            amounts: [None; Category::COUNT],
            musical: [100, 50, 0][seed as usize % 3],
            template: 0,
        };
        random::target(&spec).unwrap().patch
    }

    /// The pinned SHA-256 of layout 1's canonical text (`layout-v1.txt`), as docs/r3.md quotes it.
    const LAYOUT_V1_SHA256: &str =
        "2463ccea7ea768f368a80a7634369b93b00891f148560ad84989a15c8e3224a8";

    fn hex(b: &[u8]) -> String {
        use core::fmt::Write as _;
        b.iter().fold(String::new(), |mut s, x| {
            let _ = write!(s, "{x:02x}");
            s
        })
    }

    #[test]
    fn the_v1_layout_is_pinned() {
        let l = OutputLayout::v1();
        let want = include_str!("layout-v1.txt");
        assert!(
            l.canonical_text() == want,
            "layout 1 changed: a catalogue field it reads moved, so every model of layout 1 is refused. Freeze the \
             old values or ship layout 2. Now:\n<<<\n{}>>>",
            l.canonical_text()
        );
        assert_eq!(hex(&l.sha256()), LAYOUT_V1_SHA256);
        assert_eq!(l.width(), 102);
        assert_eq!(
            (
                l.continuous().len(),
                l.categorical().len(),
                l.enables().len(),
                l.routes().len()
            ),
            (33, 7, 4, 16)
        );
    }

    #[test]
    fn the_layout_is_built_only_for_its_generator_and_template() {
        assert_eq!(
            OutputLayout::build(2, 1, 0).unwrap_err().code(),
            "unknown_layout"
        );
        assert_eq!(
            OutputLayout::build(0, 1, 0).unwrap_err().code(),
            "unknown_layout"
        );
        assert_eq!(
            OutputLayout::build(1, 2, 0).unwrap_err().code(),
            "layout_pairing"
        );
        assert_eq!(
            OutputLayout::build(1, 1, 1).unwrap_err().code(),
            "layout_pairing"
        );
    }

    #[test]
    fn a_renamed_address_is_missing_not_guessed() {
        let start = random::start(0).unwrap();
        let mut params = PARAMS_V1.to_vec();
        params[0] = "voice.moode";
        let e = OutputLayout::from_lists(
            1,
            1,
            0,
            start,
            &params,
            &ENABLES_V1,
            &ROUTE_SOURCES_V1,
            &ROUTE_DESTINATIONS_V1,
        )
        .unwrap_err();
        assert_eq!(e.code(), "layout_missing");
    }

    #[test]
    fn the_rows_segments_tile_the_width() {
        let l = OutputLayout::v1();
        let mut seen = vec![0u32; l.width()];
        for e in l.continuous().iter().chain(l.categorical()) {
            for s in &mut seen[e.offset..e.offset + e.len()] {
                *s += 1;
            }
        }
        for e in l.enables() {
            seen[e.offset] += 1;
        }
        for c in l.routes() {
            seen[c.presence] += 1;
            seen[c.depth] += 1;
        }
        assert!(seen.iter().all(|n| *n == 1), "{seen:?}");
    }

    /// Structure, categoricals and routes are the same; continuous values within the `f32` rounding of the
    /// normalised domain (plain -> normalised -> plain is not an exact round trip).
    fn assert_same_on_layout(l: &OutputLayout, decoded: &Patch, canon: &Patch, what: &str) {
        assert_eq!(decoded.routes.len(), canon.routes.len(), "{what}");
        for (a, b) in decoded.routes.iter().zip(&canon.routes) {
            assert_eq!((&a.src, &a.dst), (&b.src, &b.dst), "{what}");
            assert!(
                (a.depth - b.depth).abs() < 1e-3,
                "{what}: {} vs {}",
                a.depth,
                b.depth
            );
        }
        for e in l.enables() {
            assert_eq!(
                decoded.find(&e.id).unwrap().1.enabled,
                canon.find(&e.id).unwrap().1.enabled,
                "{what}"
            );
        }
        for e in l.categorical() {
            assert_eq!(
                decoded.value(&e.addr),
                canon.value(&e.addr),
                "{what}: {}",
                e.addr
            );
        }
        for e in l.continuous() {
            let (x, y) = (
                decoded.value(&e.addr).unwrap(),
                canon.value(&e.addr).unwrap(),
            );
            let (nx, ny) = (e.spec.to_norm(x), e.spec.to_norm(y));
            assert!((nx - ny).abs() <= 2e-7, "{what}: {}: {x} vs {y}", e.addr);
        }
    }

    #[test]
    fn the_template_row_decodes_to_the_template() {
        let l = OutputLayout::v1();
        let labels = l.encode(l.template(), &held()).unwrap();
        let p = l.decode(&labels.values, &held()).unwrap();
        assert_same_on_layout(&l, &p, l.template(), "the template");
        assert_eq!(p.routes, []);
        assert_eq!(p.modules().count(), l.template().modules().count());
    }

    #[test]
    fn generator_targets_round_trip_through_their_labels() {
        let l = OutputLayout::v1();
        let h = held();
        let mut routed = 0;
        let mut switched = 0;
        for seed in 0..60u64 {
            let target = v1_target(9_100_000 + seed);
            let labels = l.encode(&target, &h).unwrap();
            let decoded = l.decode(&labels.values, &h).unwrap();
            let mut canon = target.clone();
            l.canonicalise(&mut canon, h.phrase()).unwrap();
            routed += usize::from(!canon.routes.is_empty());
            switched += l
                .enables()
                .iter()
                .filter(|e| canon.find(&e.id).unwrap().1.enabled)
                .count();
            assert_same_on_layout(&l, &decoded, &canon, &format!("seed {seed}"));
            assert!(labels.mask.contains(&0) && labels.mask.contains(&1));
        }
        assert!(
            routed > 10 && switched > 20,
            "{routed} routed, {switched} switched on"
        );
    }

    #[test]
    fn decoding_reads_logits_and_clamps() {
        let l = OutputLayout::v1();
        let h = held();
        let mut row = l.encode(l.template(), &h).unwrap().values;
        // osc 2 on, filter 1 high-pass by its logit, cutoff past the top clamps to the range's top.
        let osc2 = l
            .enables()
            .iter()
            .find(|e| e.id.as_str() == "osc2")
            .unwrap();
        row[osc2.offset] = 3.5;
        let mode = l
            .categorical()
            .iter()
            .find(|e| e.addr.to_string() == "filter1.mode")
            .unwrap();
        row[mode.offset..mode.offset + 4].copy_from_slice(&[0.1, -2.0, 7.0, 7.0]);
        let cutoff = l
            .continuous()
            .iter()
            .find(|e| e.addr.to_string() == "filter1.cutoff")
            .unwrap();
        row[cutoff.offset] = 1.7;
        let p = l.decode(&row, &h).unwrap();
        assert!(p.find(&ModuleId::new("osc2").unwrap()).unwrap().1.enabled);
        assert_eq!(
            p.value(&mode.addr),
            Some(2.0),
            "a tie goes to the lower index"
        );
        assert_eq!(p.value(&cutoff.addr), Some(20_000.0));
        row[cutoff.offset] = f32::NAN;
        assert_eq!(
            l.decode(&row, &h).unwrap_err(),
            DecodeError::NonFinite {
                index: cutoff.offset
            }
        );
        assert_eq!(
            l.decode(&row[1..], &h).unwrap_err().code(),
            "hypothesis_width"
        );
    }

    #[test]
    fn a_patch_off_the_layout_is_refused() {
        let l = OutputLayout::v1();
        let mut p = l.template().clone();
        set(&mut p, "osc1.fine", 60.0);
        assert_eq!(l.encode(&p, &held()).unwrap_err().code(), "off_layout");
        // Within +-50 cents osc1.fine is layout 1's context entry: accepted, and never a label.
        set(&mut p, "osc1.fine", -50.0);
        assert_eq!(
            l.encode(&p, &held()).unwrap(),
            l.encode(l.template(), &held()).unwrap()
        );
        let mut p = l.template().clone();
        apply(
            &mut p,
            Op::InsertRoute {
                index: 0,
                route: Route {
                    src: "lfo1".parse().unwrap(),
                    dst: "filter1.resonance".parse().unwrap(),
                    depth: 20.0,
                },
            },
        )
        .unwrap();
        assert_eq!(l.encode(&p, &held()).unwrap_err().code(), "off_layout");
    }

    /// M1: the same sound, told the same key, gets one label. The template with `osc1.semi` +12 played at C3 and
    /// with 0 played at C4, both read as C4 by the intake (the judge's probe pair: one model input, two labels before
    /// the fix).
    #[test]
    fn labels_are_relative_to_the_told_key() {
        let l = OutputLayout::v1();
        let told = cue(60);
        let mut up = l.template().clone();
        set(&mut up, "osc1.semi", 12.0);
        let a = l
            .encode(
                &up,
                &Hearing::sample(&told, &[note(48, 1500)], 3000).unwrap(),
            )
            .unwrap();
        let b = l
            .encode(
                l.template(),
                &Hearing::sample(&told, &[note(60, 1500)], 3000).unwrap(),
            )
            .unwrap();
        assert_eq!(a, b);
        let semi = entry(&l, "osc1.semi");
        assert_eq!(semi.spec.from_norm(a.values[semi.offset]), 0.0);
        // osc 2 moves with osc 1, when it is heard.
        let mut two = up.clone();
        apply(
            &mut two,
            Op::SetEnabled {
                id: ModuleId::new("osc2").unwrap(),
                enabled: true,
            },
        )
        .unwrap();
        set(&mut two, "osc2.semi", 7.0);
        let c = l
            .encode(
                &two,
                &Hearing::sample(&told, &[note(48, 1500)], 3000).unwrap(),
            )
            .unwrap();
        let semi2 = entry(&l, "osc2.semi");
        assert!((semi2.spec.from_norm(c.values[semi2.offset]) + 5.0).abs() < 1e-4);
        assert_eq!(c.mask[semi2.offset], 1);
    }

    #[test]
    fn a_rekey_past_the_range_or_onto_a_heard_keytrack_is_refused() {
        let l = OutputLayout::v1();
        let mut p = l.template().clone();
        set(&mut p, "osc1.semi", 40.0);
        let h = Hearing::sample(&cue(36), &[note(48, 1500)], 3000).unwrap();
        assert_eq!(h.shift(), 12);
        assert_eq!(l.encode(&p, &h).unwrap_err().code(), "rekey");
        // Under the octave pair a keytrack is heard: re-keying would move the cutoff's tracking, so it is refused;
        // told the played key, nothing moves.
        let pair = phrases::phrase(OCTAVE_PAIR).unwrap();
        let mut k = l.template().clone();
        set(&mut k, "filter1.keytrack", 50.0);
        let off = Hearing::sample(&cue(50), pair.notes, pair.length_ms).unwrap();
        assert_eq!(l.encode(&k, &off).unwrap_err().code(), "rekey");
        let on = Hearing::sample(&cue(48), pair.notes, pair.length_ms).unwrap();
        assert!(l.encode(&k, &on).is_ok());
        // Under one note the keytrack is dead (reset to 0 by canonicalising), so a re-key is fine.
        let one = Hearing::sample(&cue(50), &[note(48, 1500)], 3000).unwrap();
        assert!(l.encode(&k, &one).is_ok());
    }

    /// M2: no loss on the envelopes' releases when the first note-off falls at or after the model's 2 s window.
    #[test]
    fn releases_the_window_cannot_see_carry_no_loss() {
        let l = OutputLayout::v1();
        let mut p = l.template().clone();
        set(&mut p, "env1.release", 900.0);
        let rel = entry(&l, "env1.release");
        let c = Cue {
            on_s: 0.006,
            ..cue(48)
        };
        let early = Hearing::sample(&c, &[note(48, 1990)], 4000).unwrap();
        assert!(early.release_in_window());
        assert_eq!(l.encode(&p, &early).unwrap().mask[rel.offset], 1);
        let late = Hearing::sample(&c, &[note(48, 1995)], 4000).unwrap();
        assert!(!late.release_in_window(), "{}", late.first_note_off_s());
        let labels = l.encode(&p, &late).unwrap();
        assert_eq!(labels.mask[rel.offset], 0);
        assert_eq!(labels.mask[entry(&l, "env1.attack").offset], 1);
        // The comp's release is not an envelope's: it keeps its loss whenever it is heard.
        assert!(
            l.releases
                .iter()
                .all(|e| e.addr.to_string().starts_with("env"))
        );
        assert_eq!(l.releases.len(), 2);
    }

    /// M3: a label is what decoding gives back when the job plays the sample's phrase: under the held phrase, a pad
    /// held 2.8 s (a 2 s attack is kept, not reset as under the held phrase) and a 0.3 s note (a 2 s release heard
    /// over its tail is kept).
    #[test]
    fn labels_round_trip_under_the_jobs_performance() {
        let l = OutputLayout::v1();
        let mut pad = l.template().clone();
        set(&mut pad, "env1.attack", 2000.0);
        let mut short = l.template().clone();
        set(&mut short, "env1.release", 2000.0);
        let c = cue(48);
        let cases: [(&str, Patch, Vec<PhraseNote>, u32); 3] = [
            ("held", v1_target(9_150_001), vec![note(48, 1500)], 3000),
            ("pad", pad, vec![note(48, 2800)], 4000),
            ("short", short, vec![note(48, 300)], 3000),
        ];
        for (name, patch, notes, length) in &cases {
            let sample = Hearing::sample(&c, notes, *length).unwrap();
            let job = job_like(&c, notes, *length);
            assert_eq!(job.phrase(), sample.phrase(), "{name}");
            let labels = l.encode(patch, &sample).unwrap();
            let decoded = l.decode(&labels.values, &job).unwrap();
            let mut canon = patch.clone();
            l.canonicalise(&mut canon, sample.phrase()).unwrap();
            assert_same_on_layout(&l, &decoded, &canon, name);
        }
        let attack = entry(&l, "env1.attack");
        let back = l
            .decode(
                &l.encode(
                    &cases[1].1,
                    &Hearing::sample(&c, &cases[1].2, 4000).unwrap(),
                )
                .unwrap()
                .values,
                &job_like(&c, &cases[1].2, 4000),
            )
            .unwrap();
        assert!((back.value(&attack.addr).unwrap() - 2000.0).abs() < 0.5);
        // The same row decoded under the held phrase loses the attack (the judge's measurement: 2,000 ms -> 5 ms).
        let row = l
            .encode(
                &cases[1].1,
                &Hearing::sample(&c, &cases[1].2, 4000).unwrap(),
            )
            .unwrap()
            .values;
        assert_eq!(
            l.decode(&row, &held()).unwrap().value(&attack.addr),
            Some(5.0)
        );
        // Generator targets round-trip under holds from 80 ms to 3.9 s.
        for seed in 0..20u64 {
            let t = v1_target(9_160_000 + seed);
            let h = Hearing::sample(&c, &[note(48, 80 + 200 * seed as u32)], 4000).unwrap();
            let labels = l.encode(&t, &h).unwrap();
            let decoded = l
                .decode(&labels.values, &job_like(&c, &h.notes, 4000))
                .unwrap();
            let mut canon = t.clone();
            l.canonicalise(&mut canon, h.phrase()).unwrap();
            assert_same_on_layout(&l, &decoded, &canon, &format!("seed {seed}"));
        }
    }

    /// Decoding re-keys onto the job's key (a `--perf` override) and sets `osc1.fine` from the pitch offset (S7).
    #[test]
    fn decoding_rekeys_to_the_jobs_key_and_tunes_osc1() {
        let l = OutputLayout::v1();
        let semi = entry(&l, "osc1.semi");
        let fine: ParamAddr = "osc1.fine".parse().unwrap();
        let mut row = l.encode(l.template(), &held()).unwrap().values;
        row[semi.offset] = semi.spec.to_norm(5.0);
        // Told C3 (+20 cents sharp), the job plays C3: semi 5, fine +20.
        let told = Cue {
            f0_hz: Some(130.812_78 * libm::pow(2.0, 0.2 / 12.0)),
            ..cue(48)
        };
        let perf = |key: u8| TargetPerf {
            key,
            velocity: 100.0 / 127.0,
            on_s: 0.005,
            hold_s: 1.0,
            duration_s: 3.0,
            f0_hz: told.f0_hz,
        };
        let p = l
            .decode(&row, &Hearing::job(&told, &perf(48)).unwrap())
            .unwrap();
        let five = p.value(&semi.addr).unwrap();
        assert!((five - 5.0).abs() < 1e-4, "{five}");
        assert!(
            (p.value(&fine).unwrap() - 20.0).abs() < 0.01,
            "{:?}",
            p.value(&fine)
        );
        // The job overridden to C4: the patch must still sound at the told key + 5, so the semi drops by 12.
        let p = l
            .decode(&row, &Hearing::job(&told, &perf(60)).unwrap())
            .unwrap();
        assert_eq!(p.value(&semi.addr), Some(five - 12.0));
        // Past the range: the hypothesis is dropped, never clamped to the wrong pitch.
        row[semi.offset] = semi.spec.to_norm(-40.0);
        assert_eq!(
            l.decode(&row, &Hearing::job(&told, &perf(60)).unwrap())
                .unwrap_err()
                .code(),
            "hypothesis_rekey"
        );
        // Unpitched: osc1.fine stays the template's; an offset past 50 cents is clamped.
        let flat = Cue {
            f0_hz: None,
            ..told
        };
        let p = l
            .decode(
                &l.encode(l.template(), &held()).unwrap().values,
                &Hearing::job(&flat, &perf(48)).unwrap(),
            )
            .unwrap();
        assert_eq!(p.value(&fine), Some(0.0));
        let wide = Cue {
            f0_hz: Some(130.812_78 * libm::pow(2.0, 0.7 / 12.0)),
            ..told
        };
        let p = l
            .decode(
                &l.encode(l.template(), &held()).unwrap().values,
                &Hearing::job(&wide, &perf(48)).unwrap(),
            )
            .unwrap();
        assert_eq!(p.value(&fine), Some(50.0));
    }

    #[test]
    fn hearings_refuse_what_no_job_or_sample_makes() {
        assert_eq!(
            Hearing::sample(&cue(48), &[], 3000).unwrap_err().code(),
            "hearing_phrase"
        );
        assert_eq!(
            Hearing::sample(&cue(200), &[note(48, 100)], 3000)
                .unwrap_err()
                .code(),
            "hearing_key"
        );
        let bad = Cue {
            on_s: f64::NAN,
            ..cue(48)
        };
        assert_eq!(
            Hearing::sample(&bad, &[note(48, 100)], 3000)
                .unwrap_err()
                .code(),
            "hearing_times"
        );
        let perf = TargetPerf {
            key: 48,
            velocity: 0.8,
            on_s: 3.0,
            hold_s: 1.0,
            duration_s: 3.0,
            f0_hz: None,
        };
        assert_eq!(
            Hearing::job(&cue(48), &perf).unwrap_err().code(),
            "hearing_times"
        );
        // A note held past the end is held throughout: valid, with no tail.
        let held_through = TargetPerf {
            on_s: 0.005,
            hold_s: 9.0,
            ..perf
        };
        let h = Hearing::job(&cue(48), &held_through).unwrap();
        assert_eq!(h.phrase().tail_ms(), 0);
        assert!(!h.release_in_window());
    }
}
