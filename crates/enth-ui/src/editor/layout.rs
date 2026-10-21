//! The layout seam (lane ED1, docs/editor.md): a layout is a value, not code.
//!
//! A [`LayoutSpec`] is a tree of rows and columns whose leaves are named [`Region`]s, sized by theme tokens or by
//! shares of what the tokens leave, plus a placement table that says which part of the patch ([`Part`]) fills which
//! region. It is JSON (schema [`LAYOUT_SCHEMA`]), the same form for the built-in layouts (`layout/<name>.json`, listed
//! in [`BUILTIN`]) and for a file named by `ENTH_LAYOUT`, so a layout switches without a rebuild. "Lanes" is layout 1.
//!
//! A layout holds no pixels: a size is a token path into the theme (`"size.bar.header"`, `"size.card.generator_height"`
//! times a count), or a share. A variant that needs a new region size adds a token to `theme/sizes/editor.rs`.
//!
//! [`Layout::new`] checks a spec against the rules below and resolves its tokens once; [`Layout::regions`] then cuts
//! a rectangle into region rectangles every frame. Components never know where the others are, and every control's
//! id comes from what it controls (`cards::ids`), so a re-layout keeps drags, hovers and egui's widget state.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use egui::{Rect, pos2};
use serde::{Deserialize, Serialize};

use super::regions::Arrangement;
use crate::Theme;

/// The schema string every layout file carries.
pub const LAYOUT_SCHEMA: &str = "enth.layout/1";

/// The built-in layouts, by name, as JSON. Layout 1, "lanes", comes first and is the default.
#[cfg(not(feature = "learn"))]
pub const BUILTIN: &[(&str, &str)] = &[("lanes", include_str!("layout/lanes.json"))];

/// The built-in layouts, by name, as JSON. Layout 1, "lanes", comes first and is the default. With the `learn`
/// feature, `learn` (lane LUI) places the `lesson` region: the editor draws it while the lesson pane shows.
#[cfg(feature = "learn")]
pub const BUILTIN: &[(&str, &str)] = &[
    ("lanes", include_str!("layout/lanes.json")),
    ("learn", include_str!("layout/learn.json")),
];

/// The name of the default layout.
pub const DEFAULT: &str = "lanes";

/// A named rectangle of the editor, drawn by one component.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Region {
    /// Wordmark, patch name, Undo and Redo; built-in parts placed here draw as inline knobs (layout 1: master level).
    Header,
    /// The eight macro knobs.
    Macros,
    /// A card region for the sound sources (layout 1: the generators as strips).
    Sources,
    /// A card region for the per-voice effects (layout 1: the filters as a lane).
    VoiceFx,
    /// A card region for the modulators (layout 1: envelopes and LFOs as a band).
    Modulators,
    /// A card region for the global effects (layout 1: voice settings, then the effects, as a lane).
    Effects,
    /// The modulation overview: the source tiles with their ⊕ handles, and the matrix.
    Overview,
    /// The status and help line.
    Status,
    /// The output: a scope (or a spectrum) and the stereo meter, from the engine's telemetry (lane ED2). Optional.
    Output,
    /// Reserved for Learn: the lesson pane (no presenter in P1b).
    #[cfg(feature = "learn")]
    Lesson,
    /// Reserved for Recreate: the challenge bar (no presenter in P1b).
    #[cfg(feature = "recreate")]
    ChallengeBar,
    /// Reserved for Recreate: the results drawer (no presenter in P1b).
    #[cfg(feature = "recreate")]
    Results,
}

impl Region {
    /// The regions that hold cards.
    pub fn holds_cards(self) -> bool {
        matches!(
            self,
            Region::Sources | Region::VoiceFx | Region::Modulators | Region::Effects
        )
    }

    /// How a card region arranges its cards unless the layout says otherwise.
    pub fn default_arrangement(self) -> Arrangement {
        match self {
            Region::Sources => Arrangement::Strips,
            Region::Modulators => Arrangement::Band,
            _ => Arrangement::Lane,
        }
    }

    /// The region's name in layout files (and its egui scope).
    pub fn name(self) -> &'static str {
        match self {
            Region::Header => "header",
            Region::Macros => "macros",
            Region::Sources => "sources",
            Region::VoiceFx => "voice_fx",
            Region::Modulators => "modulators",
            Region::Effects => "effects",
            Region::Overview => "overview",
            Region::Status => "status",
            Region::Output => "output",
            #[cfg(feature = "learn")]
            Region::Lesson => "lesson",
            #[cfg(feature = "recreate")]
            Region::ChallengeBar => "challenge_bar",
            #[cfg(feature = "recreate")]
            Region::Results => "results",
        }
    }
}

/// A part of the patch that the placement table puts into a region.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Part {
    /// The generators section (oscillators, noise).
    Generators,
    /// The per-voice effects section (filters).
    VoiceFx,
    /// The built-in voice settings (mode, polyphony, glide, pan).
    Voice,
    /// The modulators section (envelopes, LFOs).
    Modulators,
    /// The global effects section (compressor, reverb).
    Fx,
    /// The built-in master section (level).
    Master,
}

impl Part {
    /// Every part, in signal order.
    pub const ALL: [Part; 6] = [
        Part::Generators,
        Part::VoiceFx,
        Part::Voice,
        Part::Modulators,
        Part::Fx,
        Part::Master,
    ];

    /// The built-ins, which a header can show as inline knobs.
    pub fn is_builtin(self) -> bool {
        matches!(self, Part::Voice | Part::Master)
    }
}

/// One placement: `part` fills `region` (several parts in one card region follow the table's order).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Placement {
    pub part: Part,
    pub region: Region,
}

/// How big a row or column is along its parent's axis.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "RawSize", into = "RawSize")]
pub enum Size {
    /// `count` times a theme token (a path such as `"size.bar.header"` or `"space.gap"`), with the layout's gap
    /// between the copies: three generator strips are `{"token": "size.card.generator_height", "count": 3}`.
    Token { token: String, count: u16 },
    /// A share of the space the tokens leave, split among the shares of one row or column.
    Share(u16),
}

impl Size {
    /// One copy of a token.
    pub fn token(path: &str) -> Size {
        Size::Token {
            token: path.to_owned(),
            count: 1,
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSize {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    count: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    share: Option<u16>,
}

impl TryFrom<RawSize> for Size {
    type Error = String;

    fn try_from(raw: RawSize) -> Result<Self, Self::Error> {
        match raw {
            RawSize {
                token: Some(token),
                count,
                share: None,
            } => match count.unwrap_or(1) {
                0 => Err(format!("token {token:?} has count 0; use 1 or more")),
                count => Ok(Size::Token { token, count }),
            },
            RawSize {
                token: None,
                count: None,
                share: Some(0),
            } => Err("a share must be 1 or more".to_owned()),
            RawSize {
                token: None,
                count: None,
                share: Some(share),
            } => Ok(Size::Share(share)),
            _ => Err(
                "a size is {\"token\": \"size.…\"} (with an optional \"count\") or {\"share\": n}"
                    .to_owned(),
            ),
        }
    }
}

impl From<Size> for RawSize {
    fn from(size: Size) -> Self {
        match size {
            Size::Token { token, count } => RawSize {
                token: Some(token),
                count: (count != 1).then_some(count),
                share: None,
            },
            Size::Share(share) => RawSize {
                token: None,
                count: None,
                share: Some(share),
            },
        }
    }
}

/// What a node of the tree is.
#[derive(Clone, Debug, PartialEq)]
pub enum NodeKind {
    /// Children top to bottom.
    Rows(Vec<Node>),
    /// Children left to right.
    Cols(Vec<Node>),
    /// A region, with how it arranges its cards (card regions only; `None` takes the region's default).
    Region(Region, Option<Arrangement>),
}

/// A node of the layout tree: its size along the parent's axis (the root has none), the gap between its children
/// (the layout's gap unless given), and what it is.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "RawNode", into = "RawNode")]
pub struct Node {
    pub size: Option<Size>,
    pub gap: Option<Size>,
    pub kind: NodeKind,
}

impl Node {
    /// A row of `children` (top to bottom).
    pub fn rows(size: Option<Size>, children: Vec<Node>) -> Node {
        Node {
            size,
            gap: None,
            kind: NodeKind::Rows(children),
        }
    }

    /// A column split of `children` (left to right).
    pub fn cols(size: Option<Size>, children: Vec<Node>) -> Node {
        Node {
            size,
            gap: None,
            kind: NodeKind::Cols(children),
        }
    }

    /// A region leaf.
    pub fn region(size: Size, region: Region, arrange: Option<Arrangement>) -> Node {
        Node {
            size: Some(size),
            gap: None,
            kind: NodeKind::Region(region, arrange),
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawNode {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    size: Option<Size>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    gap: Option<Size>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    rows: Option<Vec<Node>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cols: Option<Vec<Node>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    region: Option<Region>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    arrange: Option<Arrangement>,
}

impl TryFrom<RawNode> for Node {
    type Error = String;

    fn try_from(raw: RawNode) -> Result<Self, Self::Error> {
        let RawNode {
            size,
            gap,
            rows,
            cols,
            region,
            arrange,
        } = raw;
        let kind = match (rows, cols, region) {
            (Some(rows), None, None) => NodeKind::Rows(rows),
            (None, Some(cols), None) => NodeKind::Cols(cols),
            (None, None, Some(region)) => NodeKind::Region(region, arrange),
            _ => {
                return Err("a node has exactly one of \"rows\", \"cols\" or \"region\"".to_owned());
            }
        };
        if arrange.is_some() && !matches!(kind, NodeKind::Region(..)) {
            return Err("\"arrange\" belongs to a region node".to_owned());
        }
        Ok(Node { size, gap, kind })
    }
}

impl From<Node> for RawNode {
    fn from(node: Node) -> Self {
        let mut raw = RawNode {
            size: node.size,
            gap: node.gap,
            rows: None,
            cols: None,
            region: None,
            arrange: None,
        };
        match node.kind {
            NodeKind::Rows(rows) => raw.rows = Some(rows),
            NodeKind::Cols(cols) => raw.cols = Some(cols),
            NodeKind::Region(region, arrange) => {
                raw.region = Some(region);
                raw.arrange = arrange;
            }
        }
        raw
    }
}

/// A layout: the tree of regions and the placement table. See the [module docs](self) and docs/editor.md.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LayoutSpec {
    /// [`LAYOUT_SCHEMA`].
    pub schema: String,
    /// The layout's name (`"lanes"`); the plugin saves it in its `enth` field.
    pub name: String,
    /// One line for people choosing between layouts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Space around the editor (default `space.gap`).
    #[serde(default = "space_gap")]
    pub margin: Size,
    /// Space between siblings, unless a node gives its own (default `space.gap`).
    #[serde(default = "space_gap")]
    pub gap: Size,
    /// The tree.
    pub root: Node,
    /// Which part of the patch fills which region.
    pub place: Vec<Placement>,
}

fn space_gap() -> Size {
    Size::token("space.gap")
}

impl LayoutSpec {
    /// A layout from its JSON text (a file named by `ENTH_LAYOUT`, or a built-in). Only the syntax is checked here;
    /// [`Layout::new`] checks the rules.
    pub fn from_json(text: &str) -> Result<LayoutSpec, LayoutError> {
        serde_json::from_str(text).map_err(|e| LayoutError(format!("not a layout file: {e}")))
    }

    /// The JSON text, pretty-printed (what `dev_window --dump-layout` prints).
    pub fn to_json(&self) -> String {
        // Strings, integers and enums only: serialising cannot fail.
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    /// A built-in layout by name.
    pub fn builtin(name: &str) -> Option<LayoutSpec> {
        BUILTIN
            .iter()
            .find(|(n, _)| *n == name)
            .and_then(|(_, text)| LayoutSpec::from_json(text).ok())
    }

    /// Layout 1, "lanes".
    pub fn lanes() -> LayoutSpec {
        // The built-in files are checked by a test, so this never falls back in practice.
        LayoutSpec::builtin(DEFAULT).unwrap_or_else(LayoutSpec::fallback)
    }

    /// A minimal valid layout, used only if a built-in file failed to parse (a test keeps that from happening).
    fn fallback() -> LayoutSpec {
        let share = || Size::Share(1);
        LayoutSpec {
            schema: LAYOUT_SCHEMA.to_owned(),
            name: DEFAULT.to_owned(),
            description: None,
            margin: space_gap(),
            gap: space_gap(),
            root: Node::rows(
                None,
                vec![
                    Node::region(Size::token("size.bar.header"), Region::Header, None),
                    Node::region(Size::token("size.bar.macros"), Region::Macros, None),
                    Node::cols(
                        Some(share()),
                        vec![
                            Node::region(share(), Region::Sources, None),
                            Node::region(share(), Region::Effects, None),
                        ],
                    ),
                    Node::region(Size::token("size.bar.status"), Region::Status, None),
                ],
            ),
            place: vec![
                Placement {
                    part: Part::Generators,
                    region: Region::Sources,
                },
                Placement {
                    part: Part::VoiceFx,
                    region: Region::Sources,
                },
                Placement {
                    part: Part::Modulators,
                    region: Region::Sources,
                },
                Placement {
                    part: Part::Voice,
                    region: Region::Effects,
                },
                Placement {
                    part: Part::Fx,
                    region: Region::Effects,
                },
                Placement {
                    part: Part::Master,
                    region: Region::Effects,
                },
            ],
        }
    }
}

/// Why a layout was refused: one line, naming where.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LayoutError(pub String);

impl fmt::Display for LayoutError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for LayoutError {}

/// A region's rectangle for one frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placed {
    pub region: Region,
    pub rect: Rect,
    /// How a card region arranges its cards.
    pub arrange: Arrangement,
}

/// A checked layout with its tokens resolved against a theme.
#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    spec: LayoutSpec,
    /// Every token path the spec uses, in logical px.
    tokens: BTreeMap<String, f32>,
}

/// The regions a layout must have: Undo and Redo are only in the header (no keyboard undo on Linux, ADR-0002), the
/// macros are host parameters, and the status line is the editor's help.
const REQUIRED: [Region; 3] = [Region::Header, Region::Macros, Region::Status];

impl Layout {
    /// Checks `spec` and resolves its tokens against `theme`. The rules: the schema is [`LAYOUT_SCHEMA`]; every
    /// child of a row or column has a size; a region appears at most once; the header, the macros and the status
    /// line are there; every part is placed exactly once, in a card region that is in the tree (the voice and
    /// master built-ins may also go in the header); `arrange` only on card regions; every token names a number in
    /// the theme.
    pub fn new(spec: LayoutSpec, theme: &Theme) -> Result<Layout, LayoutError> {
        let err = |m: String| Err(LayoutError(format!("layout {:?}: {m}", spec.name)));
        if spec.schema != LAYOUT_SCHEMA {
            return err(format!(
                "schema is {:?}, this build reads {LAYOUT_SCHEMA:?}",
                spec.schema
            ));
        }
        let mut seen = BTreeSet::new();
        let mut sizes = vec![&spec.margin, &spec.gap];
        if let Err(m) = walk(&spec.root, "root", true, &mut seen, &mut sizes) {
            return err(m);
        }
        for region in REQUIRED {
            if !seen.contains(&region) {
                return err(format!("the {} region is missing", region.name()));
            }
        }
        let mut placed = BTreeSet::new();
        for p in &spec.place {
            if !placed.insert(p.part) {
                return err(format!("part {:?} is placed twice", p.part));
            }
            if !seen.contains(&p.region) {
                return err(format!(
                    "part {:?} goes in region {}, which the tree does not have",
                    p.part,
                    p.region.name()
                ));
            }
            let fits =
                p.region.holds_cards() || (p.region == Region::Header && p.part.is_builtin());
            if !fits {
                return err(format!(
                    "part {:?} cannot go in region {} (cards go in sources, voice_fx, modulators or effects; \
                     voice and master may also go in the header)",
                    p.part,
                    p.region.name()
                ));
            }
        }
        for part in Part::ALL {
            if !placed.contains(&part) {
                return err(format!(
                    "part {part:?} is not placed, so its controls would be unreachable"
                ));
            }
        }
        let tree = match serde_json::to_value(theme) {
            Ok(tree) => tree,
            Err(e) => return err(format!("the theme does not serialise: {e}")),
        };
        let mut tokens = BTreeMap::new();
        for size in sizes {
            if let Size::Token { token, .. } = size {
                let value = token
                    .split('.')
                    .try_fold(&tree, |node, key| node.get(key))
                    .and_then(serde_json::Value::as_f64);
                match value {
                    Some(v) => {
                        tokens.insert(token.clone(), v as f32);
                    }
                    None => {
                        return err(format!(
                            "token {token:?} is not a number in the theme (paths look like \"size.bar.header\" or \
                             \"space.gap\")"
                        ));
                    }
                }
            }
        }
        Ok(Layout { spec, tokens })
    }

    /// The spec it was built from.
    pub fn spec(&self) -> &LayoutSpec {
        &self.spec
    }

    /// The layout's name.
    pub fn name(&self) -> &str {
        &self.spec.name
    }

    /// The parts placed in `region`, in the table's order.
    pub fn parts_in(&self, region: Region) -> impl Iterator<Item = Part> + '_ {
        self.spec
            .place
            .iter()
            .filter(move |p| p.region == region)
            .map(|p| p.part)
    }

    /// The region rectangles inside `rect`, in tree order (top to bottom, left to right). Token sizes are laid out
    /// first, each clamped to what is left, so a small window cuts the last fixed children first; shares split the
    /// rest, in whole pixels. A region that gets no space is left out.
    pub fn regions(&self, rect: Rect) -> Vec<Placed> {
        let margin = self.px(&self.spec.margin, 0.0);
        let mut out = Vec::new();
        self.place(&self.spec.root, rect.shrink(margin), &mut out);
        out
    }

    /// The rectangle of `region` inside `rect`, if the layout gives it space.
    pub fn rect_of(&self, rect: Rect, region: Region) -> Option<Rect> {
        self.regions(rect)
            .into_iter()
            .find(|p| p.region == region)
            .map(|p| p.rect)
    }

    fn gap(&self, node: &Node) -> f32 {
        let gap = self.px(&self.spec.gap, 0.0);
        node.gap.as_ref().map_or(gap, |g| self.px(g, gap))
    }

    /// A token size in px (`count` copies with `gap` between them); 0 for a share.
    fn px(&self, size: &Size, gap: f32) -> f32 {
        match size {
            Size::Token { token, count } => {
                let one = self.tokens.get(token).copied().unwrap_or(0.0);
                let n = f32::from(*count);
                one * n + gap * (n - 1.0)
            }
            Size::Share(_) => 0.0,
        }
    }

    fn place(&self, node: &Node, rect: Rect, out: &mut Vec<Placed>) {
        if rect.width() <= 0.0 || rect.height() <= 0.0 {
            return;
        }
        let (children, vertical) = match &node.kind {
            NodeKind::Region(region, arrange) => {
                out.push(Placed {
                    region: *region,
                    rect,
                    arrange: arrange.unwrap_or_else(|| region.default_arrangement()),
                });
                return;
            }
            NodeKind::Rows(children) => (children, true),
            NodeKind::Cols(children) => (children, false),
        };
        let gap = self.gap(node);
        let layout_gap = self.px(&self.spec.gap, 0.0);
        let total = if vertical {
            rect.height()
        } else {
            rect.width()
        };
        let gaps = gap * children.len().saturating_sub(1) as f32;
        let mut left = (total - gaps).max(0.0);
        let mut extents = vec![0.0; children.len()];
        for (extent, child) in extents.iter_mut().zip(children) {
            if let Some(size @ Size::Token { .. }) = &child.size {
                *extent = self.px(size, layout_gap).min(left);
                left -= *extent;
            }
        }
        let shares: u32 = children
            .iter()
            .filter_map(|c| match c.size {
                Some(Size::Share(s)) => Some(u32::from(s)),
                _ => None,
            })
            .sum();
        if shares > 0 {
            let unit = left / shares as f32;
            let last = children
                .iter()
                .rposition(|c| matches!(c.size, Some(Size::Share(_))));
            let mut given = 0.0;
            for (i, (extent, child)) in extents.iter_mut().zip(children).enumerate() {
                if let Some(Size::Share(s)) = child.size {
                    *extent = if Some(i) == last {
                        left - given
                    } else {
                        (unit * f32::from(s)).floor()
                    };
                    given += *extent;
                }
            }
        }
        let mut at = if vertical { rect.top() } else { rect.left() };
        for (extent, child) in extents.into_iter().zip(children) {
            let child_rect = if vertical {
                Rect::from_min_max(pos2(rect.left(), at), pos2(rect.right(), at + extent))
            } else {
                Rect::from_min_max(pos2(at, rect.top()), pos2(at + extent, rect.bottom()))
            };
            if extent > 0.0 {
                self.place(child, child_rect, out);
            }
            at += extent + gap;
        }
    }
}

/// Checks the tree: sizes on children, regions once, `arrange` only on card regions. Collects every size.
fn walk<'a>(
    node: &'a Node,
    path: &str,
    root: bool,
    seen: &mut BTreeSet<Region>,
    sizes: &mut Vec<&'a Size>,
) -> Result<(), String> {
    match &node.size {
        Some(size) => sizes.push(size),
        None if !root => return Err(format!("{path} has no size")),
        None => {}
    }
    if let Some(gap) = &node.gap {
        if matches!(gap, Size::Share(_)) {
            return Err(format!("{path}: a gap is a token, not a share"));
        }
        sizes.push(gap);
    }
    let (children, axis) = match &node.kind {
        NodeKind::Region(region, arrange) => {
            if !seen.insert(*region) {
                return Err(format!("{path}: region {} appears twice", region.name()));
            }
            if arrange.is_some() && !region.holds_cards() {
                return Err(format!(
                    "{path}: region {} holds no cards, so it takes no \"arrange\"",
                    region.name()
                ));
            }
            return Ok(());
        }
        NodeKind::Rows(children) => (children, "rows"),
        NodeKind::Cols(children) => (children, "cols"),
    };
    if children.is_empty() {
        return Err(format!("{path}.{axis} is empty"));
    }
    for (i, child) in children.iter().enumerate() {
        walk(child, &format!("{path}.{axis}[{i}]"), false, seen, sizes)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_builtin_parses_checks_and_round_trips() {
        let theme = Theme::dark();
        assert_eq!(BUILTIN[0].0, DEFAULT, "layout 1 comes first");
        for (name, text) in BUILTIN {
            let spec = LayoutSpec::from_json(text).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(spec.name, *name);
            let back = LayoutSpec::from_json(&spec.to_json()).unwrap();
            assert_eq!(back, spec, "{name} survives a dump and a reload");
            Layout::new(spec, &theme).unwrap_or_else(|e| panic!("{e}"));
        }
        assert_eq!(LayoutSpec::lanes(), LayoutSpec::builtin("lanes").unwrap());
        Layout::new(LayoutSpec::fallback(), &theme).expect("the fallback is valid");
    }

    fn refused(json: &str) -> String {
        let spec = match LayoutSpec::from_json(json) {
            Ok(spec) => spec,
            Err(e) => return e.0,
        };
        Layout::new(spec, &Theme::dark()).expect_err("refused").0
    }

    #[test]
    fn bad_layouts_are_refused_with_a_reason() {
        let mut spec: serde_json::Value = serde_json::from_str(BUILTIN[0].1).unwrap();
        let text = |v: &serde_json::Value| v.to_string();
        let mut wrong_schema = spec.clone();
        wrong_schema["schema"] = "enth.layout/9".into();
        assert!(refused(&text(&wrong_schema)).contains("schema"));
        let mut unplaced = spec.clone();
        unplaced["place"].as_array_mut().unwrap().pop();
        assert!(refused(&text(&unplaced)).contains("not placed"));
        let mut bad_token = spec.clone();
        bad_token["margin"] = serde_json::json!({"token": "size.bar.nope"});
        assert!(refused(&text(&bad_token)).contains("size.bar.nope"));
        let mut both = spec.clone();
        both["root"]["region"] = "header".into();
        assert!(refused(&text(&both)).contains("exactly one"));
        let mut unknown_region = spec.clone();
        unknown_region["place"][0]["region"] = "nowhere".into();
        assert!(refused(&text(&unknown_region)).contains("unknown variant"));
        spec["place"][0]["region"] = "status".into();
        assert!(refused(&text(&spec)).contains("cannot go in region status"));
    }
}
