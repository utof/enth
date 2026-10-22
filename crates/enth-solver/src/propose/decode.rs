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
//! [`OutputLayout::decode`] turns one hypothesis into a patch on the template: structure, categoricals, continuous
//! values, routes, then [`OutputLayout::canonicalise`] under the held phrase (what a create job searches): a route
//! the phrase cannot reveal is removed, then every layout parameter the phrase cannot reveal is reset to the
//! template's value. [`OutputLayout::encode`] is the inverse for training labels (`tools/datagen`, lane DG): it
//! canonicalises the same way under the sample's phrase, so a label is exactly what decoding gives back, and returns
//! a loss mask (no loss on what the phrase does not reveal). Symmetric slots (the two LFOs with their route rows)
//! are left to the loss (lane TR: the minimum over the two permutations).

use core::fmt;

use enth_challenge::model::Phrase;
use enth_challenge::phrases::{self, HELD};
use enth_challenge::random::{self, Visibility, observable};
use enth_params::{Kind, ParamSpec};
use enth_patch::addr::Target;
use enth_patch::{Builtin, ModSource, ModuleId, Op, ParamAddr, Patch, Route, RouteAddr, apply};
use sha2::{Digest, Sha256};

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
    width: usize,
    text: String,
    sha256: [u8; 32],
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
    /// 1 where the value carries a loss: a parameter the phrase reveals, every structure head and route presence,
    /// a route depth where the route is present; else 0.
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

/// The held phrase (one note): what a create job's search frees, and what decoding canonicalises under.
fn held() -> &'static Phrase {
    phrases::phrase(HELD).unwrap_or_else(|| unreachable!("the held phrase is in the table"))
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
    format!("{kind} unit={:?}", spec.unit)
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
        let mut layout = OutputLayout {
            id,
            generator,
            template_id,
            template,
            continuous,
            categorical,
            enables: enable_entries,
            routes,
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
        let _ = writeln!(t, "continuous {}", self.continuous.len());
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
        let _ = writeln!(t, "categorical {}", self.categorical.len());
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
        let _ = writeln!(t, "structure {}", self.enables.len());
        for e in &self.enables {
            let kind = self.template.find(&e.id).map_or("?", |(_, m)| m.kind.key());
            let _ = writeln!(t, "{} {} {kind} template=off", e.offset, e.id.as_str());
        }
        let _ = writeln!(t, "routes {}", self.routes.len());
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

    /// One hypothesis row as a patch on the template, canonicalised under the held phrase (see the
    /// [module docs](self)).
    ///
    /// # Errors
    /// [`DecodeError::Width`], [`DecodeError::NonFinite`] (the hypothesis is dropped), [`DecodeError::Op`].
    pub fn decode(&self, row: &[f32]) -> Result<Patch, DecodeError> {
        if row.len() != self.width {
            return Err(DecodeError::Width {
                expected: self.width,
                got: row.len(),
            });
        }
        if let Some(index) = row.iter().position(|v| !v.is_finite()) {
            return Err(DecodeError::NonFinite { index });
        }
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
        self.canonicalise(&mut p, held()).map_err(op_error)?;
        Ok(p)
    }

    /// Reset what `phrase` cannot reveal, in two passes: every route whose depth the phrase does not reveal is
    /// removed, then every layout parameter it does not reveal (in a switched-off module, dead, a gauge) is set to
    /// the template's value. Decoding does this under the held phrase; labels under the sample's phrase.
    ///
    /// # Errors
    /// An op that fails on `patch` (it is not on the template).
    pub fn canonicalise(
        &self,
        patch: &mut Patch,
        phrase: &Phrase,
    ) -> Result<(), enth_patch::OpError> {
        let mask = observable(patch, phrase);
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
        let mask = observable(patch, phrase);
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

    /// The training label of `patch` heard through `phrase`: canonicalised (as [`canonicalise`](Self::canonicalise)),
    /// then read into a row, with the loss mask (see [`Labels`]).
    ///
    /// # Errors
    /// [`LayoutError::OffLayout`] when the patch is not on the layout: other modules than the template's, a module
    /// the template has on switched off, or, among what the phrase reveals, a parameter outside the layout away
    /// from the template's value or a route off the grid. What the phrase cannot reveal (a gauge such as
    /// `master.level`, which generator v1's headroom rule lowers; a nuisance; a dead or switched-off parameter; an
    /// unheard route) is not checked: canonicalising drops it.
    pub fn encode(&self, patch: &Patch, phrase: &Phrase) -> Result<Labels, LayoutError> {
        self.check_modules(patch)?;
        let mut p = patch.clone();
        self.canonicalise(&mut p, phrase)
            .map_err(|e| LayoutError::OffLayout {
                what: e.to_string(),
            })?;
        let mask = observable(&p, phrase);
        self.check_heard(&p, &mask)?;
        let mut values = vec![0.0f32; self.width];
        let mut loss = vec![0u8; self.width];
        for e in &self.continuous {
            let v = p.value(&e.addr).unwrap_or(e.template);
            values[e.offset] = e.spec.to_norm(v);
            loss[e.offset] = u8::from(mask.is_observable(&e.addr));
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
        for addr in template_order(&self.template) {
            if !layout_params.contains(&&addr)
                && mask.is_observable(&addr)
                && patch.value(&addr) != self.template.value(&addr)
            {
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
    use enth_challenge::spec::{Category, RandomSpec};

    /// The pinned SHA-256 of layout 1's canonical text (`layout-v1.txt`), as docs/r3.md quotes it.
    const LAYOUT_V1_SHA256: &str =
        "ccbe7858601bf29d978b06d99f470e8e319dcf4a907ad39659c53536c036e41e";

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
        let labels = l.encode(l.template(), held()).unwrap();
        let p = l.decode(&labels.values).unwrap();
        assert_same_on_layout(&l, &p, l.template(), "the template");
        assert_eq!(p.routes, []);
        assert_eq!(p.modules().count(), l.template().modules().count());
    }

    #[test]
    fn generator_targets_round_trip_through_their_labels() {
        let l = OutputLayout::v1();
        let phrase = held();
        let mut routed = 0;
        let mut switched = 0;
        for seed in 0..60u64 {
            let spec = RandomSpec {
                generator: 1,
                seed: 9_100_000 + seed,
                difficulty: 100,
                amounts: [None; Category::COUNT],
                musical: [100, 50, 0][seed as usize % 3],
                template: 0,
            };
            let target = random::target(&spec).unwrap().patch;
            let labels = l.encode(&target, phrase).unwrap();
            let decoded = l.decode(&labels.values).unwrap();
            let mut canon = target.clone();
            l.canonicalise(&mut canon, phrase).unwrap();
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
        let mut row = l.encode(l.template(), held()).unwrap().values;
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
        let p = l.decode(&row).unwrap();
        assert!(p.find(&ModuleId::new("osc2").unwrap()).unwrap().1.enabled);
        assert_eq!(
            p.value(&mode.addr),
            Some(2.0),
            "a tie goes to the lower index"
        );
        assert_eq!(p.value(&cutoff.addr), Some(20_000.0));
        row[cutoff.offset] = f32::NAN;
        assert_eq!(
            l.decode(&row).unwrap_err(),
            DecodeError::NonFinite {
                index: cutoff.offset
            }
        );
        assert_eq!(l.decode(&row[1..]).unwrap_err().code(), "hypothesis_width");
    }

    #[test]
    fn a_patch_off_the_layout_is_refused() {
        let l = OutputLayout::v1();
        let mut p = l.template().clone();
        apply(
            &mut p,
            Op::SetParam {
                addr: "osc1.fine".parse().unwrap(),
                value: 30.0,
            },
        )
        .unwrap();
        assert_eq!(l.encode(&p, held()).unwrap_err().code(), "off_layout");
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
        assert_eq!(l.encode(&p, held()).unwrap_err().code(), "off_layout");
    }
}
