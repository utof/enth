# Architecture decision records

One file per decision. When a decision changes, add a new ADR that supersedes the old one (or update the status line and add a dated note) — never silently edit history.

| ADR | Decision | Status |
| --- | --- | --- |
| [0001](0001-plugin-shell.md) | Plugin shell: start on nice-plug, keep it thin; clack as escape hatch | Proposed — spike passed; accept at G0 |
| [0002](0002-ui-toolkit.md) | UI: native Rust, egui on glow, no WebView, no second toolkit; design tokens in enth-ui | Proposed — UI spike (first half) passed; accept at G0 |
| [0003](0003-agent-cli.md) | Agent interface: CLI first, JSON, real units; MCP shim later | Accepted |
| [0004](0004-licensing.md) | MIT; permissive dependencies only; no GPL code | Accepted |
| [0005](0005-patch-format.md) | Patch: JSON, plain values with units, ops, versioned | Accepted |
| [0006](0006-effects-strategy.md) | Effects: port permissive DSP into our own deterministic modules; global FX bypass; no plugin hosting | Proposed — confirm at G1 |
| [0007](0007-metric-v1.md) | Scoring metric v1 (provisional): multi-resolution log-mel L1 on loudness-normalised audio, components reported separately | Proposed — recalibrate after the listening sessions and in R1 |

Upcoming (written during Phase 0–1): determinism contract, lesson format.

## Template

```markdown
# ADR-NNNN: <title>

- Status: Proposed | Accepted | Superseded by ADR-XXXX
- Date: YYYY-MM-DD

## Context
What forces are at play; what we know and don't.

## Decision
What we will do.

## Consequences
What gets easier, what gets harder, what we must watch.

## Alternatives considered
Each with why it lost.

## How we'd know we're wrong
The measurement or event that would reopen this.
```
