# ADR-0003: Agent interface — CLI first

- Status: **Accepted**
- Date: 2026-10-18

## Context

- We want LLM agents to build patches on their own: change parameters, render, get feedback, iterate. Humans, scripts and other programs should be able to use the same interface.
- Text LLMs can't reliably hear fine timbral differences, and audio-input models score near chance on basic attributes ([research.md §5](../research.md#5-can-llms-hear)). They do well with structured numbers and clear instructions.
- Evidence on CLI vs MCP is mixed: one benchmark shows large token savings for a CLI, another finds parity for a well-designed tool ([research.md §6](../research.md#6-agents-cli-vs-mcp)).

## Decision

Ship an `enth` CLI ([agent-cli.md](../agent-cli.md)) with: JSON output, real units, stateless patch files, ops as the only edit mechanism, perceptual descriptors, pass/fail expectation checks with measured hints, reference comparison, sensitivity-based suggestions, an optimiser restricted to agent-chosen parameters, and a one-page `enth guide`. Evaluate it with an agent benchmark in `content/agent-evals`.

An MCP server, if ever needed, is a thin wrapper that shells out to the CLI.

## Consequences

- One interface for agents, humans, CI and future tools; easy to test with snapshots.
- Output formats become a public contract: they must be versioned.
- Agents need a shell; chat-only clients wait for the MCP shim.

## Alternatives considered

- **MCP server first** — natural for chat clients; heavier context per call; locks the interface to one protocol.
- **Feeding audio to audio-native LLMs** — current models can't judge pitch, brightness or timbre reliably.
- **Python API only** — good for ML; poor for agents and humans without Python.

## How we'd know we're wrong

The agent eval solve rate stays low after two iterations on output design, or users mainly ask for an MCP integration.
