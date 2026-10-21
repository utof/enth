//! **Owner: lane PLY** (player and checks). Pre-declared by lane S0l (R2). The reference is docs/learn.md, "The
//! markdown block AST (lane PLY)".
//!
//! **A lesson step's text as a small block AST**, parsed with `pulldown-cmark` (MIT, `CommonMark`, no extensions, no
//! HTML output). The editor's `LessonPane` (lane LUI) draws a [`Doc`] with design tokens only; it never sees
//! markdown. The shape is deliberately small:
//!
//! | Markdown | AST |
//! | --- | --- |
//! | `#`, `##`, `###` headings | [`Block::Heading`] (levels 1–3; deeper ones are drawn as 3 and linted) |
//! | paragraphs | [`Block::Paragraph`] |
//! | `-` / `1.` lists, nested | [`Block::List`] (each item a list of blocks) |
//! | `*emphasis*`, `**strong**`, `` `code` `` | [`Inline::Emphasis`], [`Inline::Strong`], [`Inline::Code`] |
//! | `[Cutoff](enth:filter1.cutoff)` | [`Inline::Link`] to [`LinkTarget::Control`]: highlights the control on hover (any `Selector`: `filter1.*`, `macro3`, `lfo1->filter1.cutoff`) |
//! | `[hear it](enth:play/2)` | [`Inline::Link`] to [`LinkTarget::Play`]: plays the step's example or option 2 (1-based) |
//! | `[more](https://…)` | [`Inline::Link`] to [`LinkTarget::Web`]: opens the browser |
//! | a fenced block with the info string `enth` | [`Block::Widgets`], one [`Widget`] per line: `play 2`, `control filter1.cutoff`, `scope`, `shape env2` |
//!
//! **The lint** (`lesson-check`'s text step, docs/learn.md): no HTML (`html`), no images (`image`), no code blocks
//! other than `enth` widget blocks (`code_block`), no block quotes or rules (`quote`, `rule`), links only to `https:`
//! or `enth:` (`link_scheme`), `enth:` links and widgets that read (`bad_enth_link`, `bad_widget`) and resolve
//! against the step (`unresolved`: a control the step's start lacks, an example it does not have), headings at most
//! level 3 (`heading_level`), and at most [`STEP_WORD_BUDGET`] words (`too_long`, [`lint`]). The parse is lenient,
//! so the pane always draws something: refused content is dropped (HTML, code, images, rules) or kept as plain
//! content (a link to another scheme keeps its text, a quote its blocks), and reported.

use core::ops::Range;

use enth_challenge::v2::Selector;
use enth_patch::{ModuleId, Patch};
use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, LinkType, Options, Parser, Tag, TagEnd};

/// The most words a step's text may hold: about 80 for a Name step's explanation (docs/learn.md, "The lesson
/// shape"), plus room for a cue and a link. `lint` takes the budget, so `lesson-check` may tighten it per step.
pub const STEP_WORD_BUDGET: usize = 120;

/// A step's text, parsed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Doc {
    /// Its blocks, in order.
    pub blocks: Vec<Block>,
}

/// One block.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Block {
    /// A heading, level 1–3.
    Heading {
        /// 1, 2 or 3.
        level: u8,
        /// Its text.
        inlines: Vec<Inline>,
    },
    /// A paragraph.
    Paragraph(Vec<Inline>),
    /// A list.
    List {
        /// The first number of an ordered list; `None` for a bulleted one.
        start: Option<u64>,
        /// Its items, each a list of blocks (a paragraph, a nested list).
        items: Vec<Vec<Block>>,
    },
    /// A fenced `enth` block: widgets drawn in the text's flow.
    Widgets(Vec<Widget>),
}

/// Text inside a block.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Inline {
    /// Plain text (soft line breaks read as a space).
    Text(String),
    /// A hard line break.
    LineBreak,
    /// A code span (`filter1.cutoff`), drawn as a monospaced chip.
    Code(String),
    /// Emphasis.
    Emphasis(Vec<Inline>),
    /// Strong emphasis.
    Strong(Vec<Inline>),
    /// A link.
    Link {
        /// Where it goes.
        target: LinkTarget,
        /// Its text.
        inlines: Vec<Inline>,
    },
}

/// Where a link goes.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum LinkTarget {
    /// `enth:<selector>`: highlight a control (a parameter, a module, a route or a macro).
    Control(Selector),
    /// `enth:play/<n>`: play the step's example (Listen) or option (Choose) `n`, 1-based.
    Play(u8),
    /// An `https:` address.
    Web(String),
}

/// A widget in an `enth` block, one per line.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Widget {
    /// `play <n>`: a play button for the step's example or option `n` (1-based).
    Play(u8),
    /// `control <selector>`: the control itself, live, in the text (a knob, a switch, a route's depth).
    Control(Selector),
    /// `scope`: the live scope.
    Scope,
    /// `shape <module>`: a modulator's shape (an envelope's stages, an LFO's wave), drawn from the live patch.
    Shape(ModuleId),
}

/// One lint finding: a stable code, what is wrong, and where in the text (bytes).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Issue {
    /// `html`, `image`, `code_block`, `quote`, `rule`, `link_scheme`, `bad_enth_link`, `bad_widget`, `unresolved`,
    /// `heading_level`, `too_long`.
    pub code: &'static str,
    /// What is wrong.
    pub message: String,
    /// Where, in bytes of the text.
    pub range: Range<usize>,
}

impl Issue {
    /// The 1-based line of `range.start` in `text`.
    pub fn line(&self, text: &str) -> usize {
        text.get(..self.range.start)
            .map_or(1, |s| s.bytes().filter(|b| *b == b'\n').count() + 1)
    }
}

/// A step's text parsed: the AST, what the lint found while parsing, and every reference to resolve.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Parsed {
    /// The AST.
    pub doc: Doc,
    /// Findings of the parse (not the budget, not resolution).
    pub issues: Vec<Issue>,
    refs: Vec<(Range<usize>, Ref)>,
}

/// Something in the text that must exist in the step.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Ref {
    Control(Selector),
    Play(u8),
    Module(ModuleId),
}

impl Parsed {
    /// Check every `enth:` link and widget against the step: controls, routes, macros and modules against its start
    /// patch (when it has one of its own; `None` skips them), and example numbers against its examples or options.
    pub fn resolve(&self, start: Option<&Patch>, examples: usize) -> Vec<Issue> {
        let mut out = Vec::new();
        for (range, r) in &self.refs {
            let message = match r {
                Ref::Control(sel) => {
                    start.and_then(|p| sel.check_against(p).err().map(|e| e.to_string()))
                }
                Ref::Module(id) => start
                    .filter(|p| p.find(id).is_none())
                    .map(|_| format!("the step's start has no module {id}")),
                Ref::Play(n) => (usize::from(*n) > examples)
                    .then(|| format!("example {n} does not exist: the step has {examples}")),
            };
            if let Some(message) = message {
                out.push(Issue {
                    code: "unresolved",
                    message,
                    range: range.clone(),
                });
            }
        }
        out
    }
}

impl Doc {
    /// How many words its text holds: the whitespace-separated runs of [`Doc::plain`] with a letter or digit in
    /// them (widgets count none).
    pub fn words(&self) -> usize {
        self.plain()
            .split_whitespace()
            .filter(|w| w.chars().any(char::is_alphanumeric))
            .count()
    }

    /// Its visible text, one line per block (list items included), without markup: for word counts, search and
    /// agents reading a lesson through the CLI.
    pub fn plain(&self) -> String {
        fn inlines(list: &[Inline], out: &mut String) {
            for i in list {
                match i {
                    Inline::Text(t) | Inline::Code(t) => out.push_str(t),
                    Inline::LineBreak => out.push('\n'),
                    Inline::Emphasis(l) | Inline::Strong(l) | Inline::Link { inlines: l, .. } => {
                        inlines(l, out);
                    }
                }
            }
        }
        fn blocks(list: &[Block], out: &mut String) {
            for b in list {
                match b {
                    Block::Heading { inlines: l, .. } | Block::Paragraph(l) => {
                        inlines(l, out);
                        out.push('\n');
                    }
                    Block::List { items, .. } => {
                        for item in items {
                            blocks(item, out);
                        }
                    }
                    Block::Widgets(_) => {}
                }
            }
        }
        let mut out = String::new();
        blocks(&self.blocks, &mut out);
        out
    }
}

/// Parse a step's text (see the [module docs](self)). Never fails: what the lint refuses is reported and dropped or
/// kept as plain content.
pub fn parse(text: &str) -> Parsed {
    let mut b = Builder {
        events: Parser::new_ext(text, Options::empty())
            .into_offset_iter()
            .peekable(),
        out: Parsed::default(),
    };
    let blocks = b.blocks(None);
    b.out.doc = Doc { blocks };
    b.out
}

/// Lint a step's text: the parse's findings and the word budget (`budget` words, [`STEP_WORD_BUDGET`] by default).
/// Resolution against the step is [`Parsed::resolve`].
pub fn lint(text: &str, budget: usize) -> Vec<Issue> {
    let parsed = parse(text);
    let mut issues = parsed.issues;
    let words = parsed.doc.words();
    if words > budget {
        issues.push(Issue {
            code: "too_long",
            message: format!("{words} words; a step holds at most {budget}"),
            range: 0..text.len(),
        });
    }
    issues
}

type Events<'a> =
    core::iter::Peekable<pulldown_cmark::OffsetIter<'a, pulldown_cmark::DefaultBrokenLinkCallback>>;

struct Builder<'a> {
    events: Events<'a>,
    out: Parsed,
}

impl Builder<'_> {
    fn issue(&mut self, code: &'static str, message: impl Into<String>, range: Range<usize>) {
        self.out.issues.push(Issue {
            code,
            message: message.into(),
            range,
        });
    }

    /// Skip events up to and including the end of the element just started.
    fn skip(&mut self) {
        let mut depth = 1usize;
        for (event, _) in self.events.by_ref() {
            match event {
                Event::Start(_) => depth += 1,
                Event::End(_) => {
                    depth -= 1;
                    if depth == 0 {
                        return;
                    }
                }
                _ => {}
            }
        }
    }

    /// Blocks up to `end` (consumed), or to the end of the text. Inline content outside a paragraph (a tight list
    /// item's text) becomes a paragraph.
    fn blocks(&mut self, end: Option<TagEnd>) -> Vec<Block> {
        let mut out = Vec::new();
        let mut loose: Vec<Inline> = Vec::new();
        while let Some((event, range)) = self.events.next() {
            let block = match event {
                Event::End(e) if Some(e) == end => break,
                Event::End(_) => continue,
                Event::Start(Tag::Paragraph) => {
                    Some(Block::Paragraph(self.inlines(TagEnd::Paragraph)))
                }
                Event::Start(Tag::Heading { level, .. }) => {
                    let n = heading_level(level);
                    if n > 3 {
                        self.issue(
                            "heading_level",
                            format!("a level-{n} heading; steps use levels 1 to 3"),
                            range,
                        );
                    }
                    Some(Block::Heading {
                        level: n.min(3),
                        inlines: self.inlines(TagEnd::Heading(level)),
                    })
                }
                Event::Start(Tag::List(start)) => {
                    let ordered = start.is_some();
                    let mut items = Vec::new();
                    while let Some((e, _)) = self.events.next() {
                        match e {
                            Event::Start(Tag::Item) => items.push(self.blocks(Some(TagEnd::Item))),
                            Event::End(TagEnd::List(o)) if o == ordered => break,
                            _ => {}
                        }
                    }
                    Some(Block::List { start, items })
                }
                Event::Start(Tag::CodeBlock(kind)) => {
                    let info = match &kind {
                        CodeBlockKind::Fenced(info) => info.trim().to_owned(),
                        CodeBlockKind::Indented => String::new(),
                    };
                    if info == "enth" {
                        Some(Block::Widgets(self.widgets(range)))
                    } else {
                        self.issue(
                            "code_block",
                            "code blocks are not drawn; a fenced block may only be an `enth` widget block",
                            range,
                        );
                        self.skip();
                        None
                    }
                }
                Event::Start(Tag::HtmlBlock) => {
                    self.issue("html", "HTML is not drawn", range);
                    self.skip();
                    None
                }
                Event::Start(Tag::BlockQuote(_)) => {
                    self.issue(
                        "quote",
                        "block quotes are not drawn; their text is kept as plain blocks",
                        range,
                    );
                    let inner = self.blocks(Some(TagEnd::BlockQuote(None)));
                    out.extend(flush(&mut loose));
                    out.extend(inner);
                    None
                }
                Event::Rule => {
                    self.issue("rule", "rules are not drawn", range);
                    None
                }
                Event::Start(Tag::Item) => {
                    // Only inside a list, handled above.
                    self.skip();
                    None
                }
                other => {
                    // Inline content outside a paragraph: a tight list item's text.
                    self.inline_event(other, range, &mut loose);
                    continue;
                }
            };
            out.extend(flush(&mut loose));
            out.extend(block);
        }
        out.extend(flush(&mut loose));
        out
    }

    /// Inlines up to `end` (consumed).
    fn inlines(&mut self, end: TagEnd) -> Vec<Inline> {
        let mut out = Vec::new();
        while let Some((event, range)) = self.events.next() {
            match event {
                Event::End(e) if e == end => break,
                other => self.inline_event(other, range, &mut out),
            }
        }
        merge_text(out)
    }

    fn inline_event(&mut self, event: Event<'_>, range: Range<usize>, out: &mut Vec<Inline>) {
        match event {
            Event::Text(t) => out.push(Inline::Text(t.into_string())),
            Event::Code(t) => out.push(Inline::Code(t.into_string())),
            Event::SoftBreak => out.push(Inline::Text(" ".to_owned())),
            Event::HardBreak => out.push(Inline::LineBreak),
            Event::Html(_) | Event::InlineHtml(_) => self.issue("html", "HTML is not drawn", range),
            Event::Start(Tag::Emphasis) => {
                out.push(Inline::Emphasis(self.inlines(TagEnd::Emphasis)));
            }
            Event::Start(Tag::Strong) => out.push(Inline::Strong(self.inlines(TagEnd::Strong))),
            Event::Start(Tag::Image { .. }) => {
                self.issue(
                    "image",
                    "images are not drawn (no media in v1 packs)",
                    range,
                );
                self.skip();
            }
            Event::Start(Tag::Link {
                link_type,
                dest_url,
                ..
            }) => {
                let inlines = self.inlines(TagEnd::Link);
                match self.link_target(link_type, &dest_url, range) {
                    Some(target) => out.push(Inline::Link { target, inlines }),
                    None => out.extend(inlines),
                }
            }
            Event::Start(_) => {
                self.issue("html", "this markdown is not drawn", range);
                self.skip();
            }
            _ => {}
        }
    }

    fn link_target(
        &mut self,
        link_type: LinkType,
        dest: &str,
        range: Range<usize>,
    ) -> Option<LinkTarget> {
        if let Some(rest) = dest.strip_prefix("enth:") {
            return match enth_target(rest) {
                Ok(target) => {
                    let r = match &target {
                        LinkTarget::Control(sel) => Ref::Control(sel.clone()),
                        LinkTarget::Play(n) => Ref::Play(*n),
                        LinkTarget::Web(_) => unreachable!("enth_target returns no web link"),
                    };
                    self.out.refs.push((range, r));
                    Some(target)
                }
                Err(message) => {
                    self.issue("bad_enth_link", message, range);
                    None
                }
            };
        }
        if link_type == LinkType::Email {
            self.issue(
                "link_scheme",
                "e-mail links are not allowed; use an https: link",
                range,
            );
            return None;
        }
        if dest.starts_with("https://") && dest.len() > "https://".len() {
            return Some(LinkTarget::Web(dest.to_owned()));
        }
        self.issue(
            "link_scheme",
            format!("'{dest}': links go to https: addresses or enth: controls"),
            range,
        );
        None
    }

    /// The widgets of an `enth` block, one per non-empty line, up to the block's end.
    fn widgets(&mut self, range: Range<usize>) -> Vec<Widget> {
        let mut body = String::new();
        for (event, _) in self.events.by_ref() {
            match event {
                Event::Text(t) => body.push_str(&t),
                Event::End(TagEnd::CodeBlock) => break,
                _ => {}
            }
        }
        let mut out = Vec::new();
        for line in body.lines().map(str::trim).filter(|l| !l.is_empty()) {
            match widget(line) {
                Ok((w, r)) => {
                    if let Some(r) = r {
                        self.out.refs.push((range.clone(), r));
                    }
                    out.push(w);
                }
                Err(message) => self.issue("bad_widget", message, range.clone()),
            }
        }
        out
    }
}

/// What an `enth:` link points at: `play/<n>` or a selector (a compact route `lfo1->filter1.cutoff` reads as the
/// canonical `lfo1 -> filter1.cutoff`).
fn enth_target(rest: &str) -> Result<LinkTarget, String> {
    if let Some(n) = rest.strip_prefix("play/") {
        return example_number(n).map(LinkTarget::Play);
    }
    selector(rest).map(LinkTarget::Control)
}

/// A selector in a link or widget; a compact route (`lfo1->filter1.cutoff`) is accepted.
fn selector(text: &str) -> Result<Selector, String> {
    let t = text.trim();
    let canonical = match t.split_once("->") {
        Some((a, b)) => format!("{} -> {}", a.trim(), b.trim()),
        None => t.to_owned(),
    };
    canonical
        .parse::<Selector>()
        .map_err(|e| format!("'{t}' is not a control: {e}"))
}

/// An example number, 1–8 (a Listen step holds at most 8 examples, a Choose step 6 options).
fn example_number(text: &str) -> Result<u8, String> {
    text.trim()
        .parse::<u8>()
        .ok()
        .filter(|n| (1..=8).contains(n))
        .ok_or_else(|| format!("'{text}' is not an example number (1 to 8)"))
}

/// One widget line.
fn widget(line: &str) -> Result<(Widget, Option<Ref>), String> {
    let (name, arg) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
    let arg = arg.trim();
    match (name, arg.is_empty()) {
        ("scope", true) => Ok((Widget::Scope, None)),
        ("play", false) => {
            let n = example_number(arg)?;
            Ok((Widget::Play(n), Some(Ref::Play(n))))
        }
        ("control", false) => {
            let s = selector(arg)?;
            Ok((Widget::Control(s.clone()), Some(Ref::Control(s))))
        }
        ("shape", false) => {
            let id = ModuleId::new(arg).map_err(|e| e.to_string())?;
            Ok((Widget::Shape(id.clone()), Some(Ref::Module(id))))
        }
        _ => Err(format!(
            "'{line}' is not a widget: play <n>, control <selector>, scope or shape <module>"
        )),
    }
}

fn heading_level(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

/// The loose inlines so far as a paragraph, if any.
fn flush(loose: &mut Vec<Inline>) -> Option<Block> {
    if loose.is_empty() {
        return None;
    }
    Some(Block::Paragraph(merge_text(core::mem::take(loose))))
}

/// Adjacent text runs joined (pulldown-cmark splits text at escapes and soft breaks).
fn merge_text(list: Vec<Inline>) -> Vec<Inline> {
    let mut out: Vec<Inline> = Vec::with_capacity(list.len());
    for i in list {
        match (out.last_mut(), i) {
            (Some(Inline::Text(a)), Inline::Text(b)) => a.push_str(&b),
            (_, i) => out.push(i),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(s: &str) -> Inline {
        Inline::Text(s.to_owned())
    }

    #[test]
    fn the_supported_shape() {
        let src = "# Cutoff\n\nTurn the [cutoff](enth:filter1.cutoff) *down*: **darker**.\n\n- one\n- two `x`\n  1. nested\n\n```enth\nplay 2\ncontrol filter1.cutoff\nscope\nshape env2\n```\n\nSee [why](https://example.org/a) and [hear](enth:play/1).\n";
        let p = parse(src);
        assert!(p.issues.is_empty(), "{:?}", p.issues);
        let b = &p.doc.blocks;
        assert_eq!(
            b[0],
            Block::Heading {
                level: 1,
                inlines: vec![text("Cutoff")]
            }
        );
        assert_eq!(
            b[1],
            Block::Paragraph(vec![
                text("Turn the "),
                Inline::Link {
                    target: LinkTarget::Control("filter1.cutoff".parse().unwrap()),
                    inlines: vec![text("cutoff")]
                },
                text(" "),
                Inline::Emphasis(vec![text("down")]),
                text(": "),
                Inline::Strong(vec![text("darker")]),
                text("."),
            ])
        );
        let Block::List { start: None, items } = &b[2] else {
            panic!("{:?}", b[2])
        };
        assert_eq!(items.len(), 2);
        assert_eq!(items[0], vec![Block::Paragraph(vec![text("one")])]);
        assert_eq!(
            items[1][0],
            Block::Paragraph(vec![text("two "), Inline::Code("x".to_owned())])
        );
        assert!(matches!(&items[1][1], Block::List { start: Some(1), .. }));
        assert_eq!(
            b[3],
            Block::Widgets(vec![
                Widget::Play(2),
                Widget::Control("filter1.cutoff".parse().unwrap()),
                Widget::Scope,
                Widget::Shape(ModuleId::new("env2").unwrap()),
            ])
        );
        let Block::Paragraph(last) = &b[4] else {
            panic!()
        };
        assert!(last.contains(&Inline::Link {
            target: LinkTarget::Web("https://example.org/a".to_owned()),
            inlines: vec![text("why")]
        }));
        assert!(last.contains(&Inline::Link {
            target: LinkTarget::Play(1),
            inlines: vec![text("hear")]
        }));
        assert_eq!(p.doc.words(), 14, "{}", p.doc.plain());
    }

    #[test]
    fn the_lint_names_what_it_refuses() {
        for (src, code) in [
            ("a <b>bold</b> word", "html"),
            ("<div>\nblock\n</div>\n", "html"),
            ("![alt](https://example.org/x.png)", "image"),
            ("```rust\nfn main() {}\n```\n", "code_block"),
            ("    indented code\n", "code_block"),
            ("> quoted\n", "quote"),
            ("a\n\n---\n\nb", "rule"),
            ("[x](http://example.org)", "link_scheme"),
            ("[x](notes.md)", "link_scheme"),
            ("<someone@example.org>", "link_scheme"),
            ("[x](enth:filter1.cutof)", "bad_enth_link"),
            ("[x](enth:play/9)", "bad_enth_link"),
            ("```enth\nknob filter1.cutoff\n```\n", "bad_widget"),
            ("#### deep\n", "heading_level"),
        ] {
            let issues = lint(src, STEP_WORD_BUDGET);
            assert!(issues.iter().any(|i| i.code == code), "{src:?}: {issues:?}");
        }
        let long = "word ".repeat(STEP_WORD_BUDGET + 1);
        assert_eq!(lint(&long, STEP_WORD_BUDGET)[0].code, "too_long");
        assert_eq!(lint(&long, STEP_WORD_BUDGET + 1), []);
    }

    #[test]
    fn refused_content_is_dropped_or_kept_plain() {
        let p = parse("[keep](http://x.org) <b>t</b>\n\n```py\nx\n```\n\n> quote\n");
        assert_eq!(
            p.doc.blocks,
            vec![
                Block::Paragraph(vec![text("keep t")]),
                Block::Paragraph(vec![text("quote")]),
            ]
        );
        assert_eq!(
            p.issues.iter().map(|i| i.code).collect::<Vec<_>>(),
            ["link_scheme", "html", "html", "code_block", "quote"]
        );
    }

    #[test]
    fn links_and_widgets_resolve_against_the_step() {
        let src = "Turn [it](enth:filter1.cutoff), [the route](enth:lfo1->filter1.cutoff), [hear](enth:play/3).\n\n```enth\nshape env2\n```\n";
        let p = parse(src);
        assert!(p.issues.is_empty(), "{:?}", p.issues);
        let init = Patch::init();
        let issues = p.resolve(Some(&init), 2);
        let codes: Vec<_> = issues.iter().map(|i| i.code).collect();
        assert_eq!(codes, ["unresolved"; 4], "{issues:?}");
        assert_eq!(issues[0].line(src), 1);
        assert_eq!(issues[3].line(src), 3);
        assert!(
            p.resolve(None, 3).is_empty(),
            "no start: only example numbers are checked"
        );
    }

    #[test]
    fn any_text_parses() {
        for src in [
            "",
            "\n\n",
            "*",
            "[",
            "](",
            "```enth",
            "- \n  - \n",
            "# ",
            "<",
            "&amp;",
            "\\*x\\*",
        ] {
            let _ = lint(src, STEP_WORD_BUDGET);
        }
        assert_eq!(
            parse("\\*x\\*").doc.blocks,
            vec![Block::Paragraph(vec![text("*x*")])]
        );
    }
}
