# Design system

**Every colour, size, spacing, radius, stroke, font and animation rate in the editor comes from one place: `enth-ui`'s `theme` module.** Widget code never contains raw values. Change a token and the whole editor follows. The look is minimal, flat and dark (direction "Lanes"; the analysis of 50 reference screenshots is in `_local/reference/ui/README.md`, kept local).

## Principles

1. **Minimal palette:** neutral greys, one accent for values (blue) and one for modulation (orange). Category hues appear only as thin stripes and title tints. Score colours always come with a glyph (✓ ~ ✗), never colour alone.
2. **Small scales:** spacing on a 4 px grid, 3 text sizes, 2 radii, 2 stroke widths. A value off the scale needs a new token, not a literal.
3. **Flat:** no gradients, shadows, glow, textures or 3D. They cost CPU/VRAM in immediate mode and date quickly.
4. **Legible:** labels ≥ 4.5:1 contrast on their surface, values ≥ 7:1. Title case at 11–12 px; caps only for at most 3 section headers.
5. **Calm motion:** animate only what's visible and changing, at 30 Hz by default (a scope being watched may use 60 Hz).

## Token tiers

Tokens live in `crates/enth-ui/src/theme/`, in three tiers. Widgets read only the semantic and component tiers.

| Tier | What | Example | Visible to widgets? |
| --- | --- | --- | --- |
| Primitives | Raw palette and scales | `gray_900 = #1b1d20`, `space_4 = 16.0` | No (private to `theme/`) |
| Semantic | Meaning | `color.surface.card`, `color.accent.modulation`, `space.gap`, `text.value` | Yes |
| Component | Per-widget dimensions | `knob.diameter`, `card.generator_height`, `bar.macro_height` | Yes |

All sizes are logical px at 100 % UI scale. The UI scale (75–200 %) is applied once through egui's zoom factor, never by multiplying in widgets.

## Initial values (dark theme)

Starting proposals from the UI reference analysis; the G0 UI spike checks them on screen.

**Colour (semantic):**

| Token | Value | Use |
| --- | --- | --- |
| `surface.background` | `#1b1d20` | Window |
| `surface.card` | `#26292d` | Module cards, bars |
| `surface.raised` | `#30343a` | Hover, pressed, popups |
| `surface.recessed` | `#141618` | Displays (waveform, scope) |
| `line.grid` | `#2c3036` | Display grid, dividers |
| `text.primary` | `#e8eaed` | Values, titles |
| `text.label` | `#a9aeb5` | Labels (about 6:1 on card) |
| `text.dimmed` | `#5e636b` | Disabled, de-emphasised |
| `accent.value` | `#6cb4fc` | Value arcs, selection, focus |
| `accent.modulation` | `#f0904f` | Modulation arcs, wires, handles |
| `category.generator` / `filter` / `modulator` / `effect` | `#4fb3a9` / `#d9b44a` / `#8a93a0` / `#a98bd9` | Card stripes only |
| `score.good` / `near` / `bad` | `#4cc38a` / `#e9c46a` / `#e5534b` | Score rings, with ✓ ~ ✗ |

**Scales:** space 2, 4, 8, 12, 16, 24, 32 · radius `small` 2, `medium` 4 · stroke `thin` 1, `thick` 3 · text `label` 11, `value` 12 (tabular digits), `title` 14 semibold.

**Type:** one regular-weight sans in 2 weights (e.g. Inter, OFL-1.1) plus a face with tabular digits for values. At most 3 font files in the atlas. *Built:* Inter 4.1 Regular and SemiBold plus Inter Tabular, a 10 KB subset of Inter Regular with `tnum` frozen in (egui applies no OpenType features); text styles `label` 11 regular, `body` 12 regular, `value` 12 tabular, `title` 14 semibold, `strong` 12 semibold.

**Component sizes:** window 1200×800 (minimum about 1000×680) · header 40 · macro bar 56 · modulator strip 140 · status line 22 · generator card 96 (collapsed 24) · effect lane 220 wide · knob 36 with a 3 px track · lesson pane 320. *Added by the G0 spike:* knob cell 56 wide (values in units need about 52 px), control height 24, generator display slot 128 wide, lane display 40 tall, enable toggle 12, source tile 72×56 with a 14 px badge, matrix rows 16 with a 88 px target column and 36 px source columns, display grid 4×2 with a 3 px phase marker, knob drag travel 200 px (Shift ×0.1) over a 270° sweep.

**Motion:** `repaint.animated_hz` 30, `repaint.scope_hz` 60 (built as `motion.animated_hz`, `motion.scope_hz`, plus `motion.transition` 0.1 s for egui's own animations). baseview 0.4.1 caps plugin editors near 30 fps, so a 60 Hz scope gets about 30 in a host.

## Code shape

```rust
// crates/enth-ui/src/theme/mod.rs
pub struct Theme {
    pub color: Colors,      // semantic colours
    pub space: Space,       // gap, pad, section
    pub radius: Radius,
    pub stroke: Strokes,
    pub text: TextStyles,   // label, body, value, title, strong → egui::FontId
    pub size: Sizes,        // component tokens: knob, card, bars, panes
    pub state: States,      // how a state reads: disabled opacity
    pub motion: Motion,     // repaint rates
}

impl Theme {
    pub fn dark() -> Self { /* built from private primitives */ }
    /// Called once when the editor opens: sets egui::Style, Visuals and fonts from the tokens.
    pub fn apply(&self, ctx: &egui::Context) { /* … */ }
}
```

- Widgets take `&Theme` (or read it from the egui context) and use, e.g., `theme.color.accent.modulation` and `theme.size.knob.diameter`.
- `Theme` derives `serde::Serialize`/`Deserialize`. That keeps a light theme or a dev-time override file (for live tweaking in the standalone dev window) possible without changing widgets.
- Components (`Knob`, `ModuleCard`, `SourceTile`, `MatrixGrid`, …) live in `widgets/` and never define their own colours or sizes.

### Changed by the G0 UI spike (2026-10-19)

The spike ([docs/spikes/ui.md](spikes/ui.md)) kept every proposed value above and added what the widgets needed:

- **Colour:** `accent.value_wash` and `accent.modulation_wash`, the accents at alpha 40, for selection and the area under a curve (flat translucent fills, not gradients). Washes serialise as `#rrggbbaa`; egui stores colours premultiplied, so their hex digits may read back one step off while the colour itself round-trips.
- **Type:** the `body` and `strong` styles (running text and its bold words); still three sizes. Tabular digits come from a derived Inter subset, since egui can't switch `tnum` on.
- **Sizes:** the component tokens listed under *Component sizes*, most importantly a 56 px knob cell.
- **Code:** `Theme::apply` sets the same style in egui's dark and light slots and selects dark; `Strokes::thin(color)` / `thick(color)` build strokes, so `Stroke::new` stays inside `theme/`.

### Added after the owner's first look (2026-10-19, u3)

- **State:** `state.disabled_opacity` 0.4. A disabled module card draws everything inside it (stripe, title, display, knobs, values) through one child `Ui` at this opacity, so no widget has a disabled look of its own; the card fill and the enable toggle stay at full strength, and the controls stay editable. `Theme::apply` gives egui's own disabled widgets the same value (`Visuals::disabled_alpha`).
- **Sizes:** `size.handle.radius` 4 and `size.handle.reach` 8, for draggable points on a curve display (envelope corners): the circle shown while the pointer is over the display, and the grab distance, which is also the radius of the accent wash behind a hovered or dragged handle. Handle drags reuse `knob.fine_ratio` with Shift.
- Both new groups deserialise with the dark theme's values when missing, so theme files dumped before them still load.

## Rules and enforcement

- **No literals in widget code:** no `Color32::from_rgb(…)`, `Color32::WHITE`, `FontId::new(…)`, `Stroke::new(1.0, …)`, `add_space(6.0)` or similar outside `theme/`. Need a new value? Add a token (semantic or component), with a comment saying what it's for.
- **Clippy:** `crates/enth-ui/clippy.toml` (and `spikes/spike-ui/clippy.toml`) bans egui's raw constructors: `Color32::from_*`, `gamma_multiply*`, `linear_multiply`, `Rgba::from_*`, `Hsva::new`, `FontId::new`/`proportional`/`monospace`, `RichText::size`, `Stroke::new`, `CornerRadius::same`, `Margin::same`/`symmetric`. Only files in `theme/` opt out, with a reason. Clippy can't ban associated constants such as `Color32::WHITE`; `crates/enth-ui/tests/no_raw_values.rs` scans the widget source for those, for hand-built `Shadow`s, `add_space(<literal>)` and float literals other than 0, ½, 1, 2 and 100.
- **Token gallery:** an `egui_kittest` snapshot test renders every colour, text style and core component into one PNG. Token changes show up as a reviewable image diff, and agents can look at it.
- Plain numbers in layout maths (e.g. `0.5` to centre something) are fine; values that set the look are tokens.

## Related

- Visual direction and component list: `_local/reference/ui/README.md` (local only; the screenshots are third-party).
- UI toolkit and repaint rules: [ADR-0002](adr/0002-ui-toolkit.md), [benchmarks.md](benchmarks.md).
