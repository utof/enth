# ADR-0005: Patch format — JSON, real units, ops, versioned

- Status: **Accepted**
- Date: 2026-10-18 (refines the 2026-10-17 plan, which stored normalised 0–1 values)

## Context

- Patches are read and written by the plugin, the CLI, LLM agents, lessons, tests, ML pipelines and possibly a website.
- Humans and LLMs reason far better about "cutoff 800 Hz, attack 40 ms" than about "0.43".
- Knob curves (plain ⇄ normalised mappings) may change as the synth evolves; values stored in plain units survive such changes.

## Decision

- Patches are **JSON** files (`*.patch.json`), human-readable and diffable. Compact binary (`postcard`) only for ML datasets and network transport.
- Parameter values are stored as **plain values in real units**, keyed by short, stable, unit-free keys (`cutoff`, `attack`); each key's unit is defined once in its `ParamSpec` (`cutoff` is Hz, `attack` is ms) and listed by `enth params`. Enum values are stored as names (`"saw"`, `"lp"`). Modulation depth is stored as a percentage of the destination's range. Normalised values exist only at host automation, UI gesture and ML boundaries.
- Modules have stable, human-readable IDs (`osc1`, `filter1`); addresses look like `filter1.cutoff`.
- Every mutation is an **op** with an inverse; the UI, CLI, undo and any future network sync use the same ops.
- Every patch carries `schema` (format version) and `engine` (authoring engine version). Migrations are pure functions with fixture tests.
- `canonicalise()` and `hash()` define patch identity up to symmetries.

## Consequences

- Patches are easy to read, review and generate; lessons and agent evals are just files.
- Changing a parameter's range or unit needs a migration.
- Slightly larger files than binary; irrelevant at patch sizes.

## Alternatives considered

- **Normalised values** — compact, host-like; unreadable and fragile when curves change.
- **TOML** — pleasant for flat config; awkward for nested lists of modules and routes.
- **Binary only** — opaque to humans and agents.

## How we'd know we're wrong

Migrations become frequent and painful, or file size matters (e.g. URL-encoded sharing) — then add a compact encoding alongside, not instead.
