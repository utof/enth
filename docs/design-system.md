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

**Type:** one regular-weight sans in 2 weights (e.g. Inter, OFL-1.1) plus a face with tabular digits for values. At most 3 font files in the atlas.

**Component sizes:** window 1200×800 (minimum about 1000×680) · header 40 · macro bar 56 · modulator strip 140 · status line 22 · generator card 96 (collapsed 24) · effect lane 220 wide · knob 36 with a 3 px track · lesson pane 320.

**Motion:** `repaint.animated_hz` 30, `repaint.scope_hz` 60.

## Code shape

```rust
// crates/enth-ui/src/theme/mod.rs
pub struct Theme {
    pub color: Colors,      // semantic colours
    pub space: Space,       // gap, pad, section
    pub radius: Radius,
    pub stroke: Strokes,
    pub text: TextStyles,   // label, value, title → egui::FontId
    pub size: Sizes,        // component tokens: knob, card, bars, panes
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

## Rules and enforcement

- **No literals in widget code:** no `Color32::from_rgb(…)`, `Color32::WHITE`, `FontId::new(…)`, `Stroke::new(1.0, …)`, `add_space(6.0)` or similar outside `theme/`. Need a new value? Add a token (semantic or component), with a comment saying what it's for.
- **Clippy:** when `enth-ui` is created, `disallowed-methods` entries for egui's raw constructors (`Color32::from_rgb`, `from_rgba_*`, `from_gray`, `from_hex`, `FontId::new`/`proportional`/`monospace`, `Stroke::new`, `CornerRadius::same`, `Margin::same`/`symmetric`) apply to the crate; only `theme/` opts out, with a reason. Clippy can't ban associated constants such as `Color32::WHITE`; review catches those, and an `ast-grep` rule gets added if they creep in.
- **Token gallery:** an `egui_kittest` snapshot test renders every colour, text style and core component into one PNG. Token changes show up as a reviewable image diff, and agents can look at it.
- Plain numbers in layout maths (e.g. `0.5` to centre something) are fine; values that set the look are tokens.

## Related

- Visual direction and component list: `_local/reference/ui/README.md` (local only; the screenshots are third-party).
- UI toolkit and repaint rules: [ADR-0002](adr/0002-ui-toolkit.md), [benchmarks.md](benchmarks.md).
