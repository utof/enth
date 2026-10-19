# Vision

**One instrument you play, learn on and compete with — without ever graduating out of it.** Syntorial teaches on a small synth you leave behind; Phase Plant is deep but teaches nothing; Synplant matches sounds but only on its own small engine; SoundGym trains ears but isn't an instrument. `enth` puts the game, the learning and the matching on one open, modular, efficient synth.

Built for its author and friends first. If it's great for us, it can grow.

## The pillars

| Pillar | Inspired by | What it means here |
| --- | --- | --- |
| **Recreate** | Syntorial challenges, the Synth Basics match game | The synth makes a random patch at the difficulty you choose; you rebuild it and get a smart score. The headline feature |
| **Instrument** | Phase Plant | Structured modularity: generators, modulators, effects as needed; modulate almost anything; macros. Starts Syntorial-sized, grows in parallel. Light enough for many instances |
| **Learning** | Syntorial | Lessons from basics to advanced sound design, on the *same* synth. Any preset can become a lesson |
| **Sound → patch** | Synplant 2 (Genopatch) | Give a sound, get several candidate patches that are fully editable. Local, under 5 minutes |

Tying them together: **agents can drive it.** An LLM can build and tune patches through the `enth` CLI with structured feedback ([agent-cli.md](agent-cli.md)).

## Modes (working names)

- **Play** — the synth itself, in a DAW.
- **Recreate** — one button turns the plugin into the game: hear a target, rebuild it, get sound / structure / combined scores, time and hints. A settings page controls how much each part of the synth is randomized, how musical the results are, and an overall difficulty.
- **Learn** — lessons: short text + a challenge, loaded as content packs.
- **Create from sound** — drop a sample, get candidate patches.
- **Later:** ear-training games (intervals, chords, rhythm, EQ A/B) and duels with friends (shared seeds, local scoring, peer-to-peer).

## Principles (in priority order, for trade-offs)

1. **Sound and stability first.** No clicks, no dropouts, no crashes in the host. The audio thread is sacred.
2. **Light.** Low CPU per voice, near-zero cost when the editor is closed, small RAM per instance. This is why the UI is native, not a WebView ([ADR-0002](adr/0002-ui-toolkit.md)).
3. **One engine, one truth.** The same deterministic Rust engine renders in the plugin, the CLI, tests, ML training and duels. No second implementation.
4. **Local only.** All AI runs on your machine. No server-side compute, no accounts needed.
5. **Research before building.** Every milestone starts with a research session on current facts ([research-process.md](research-process.md)).
6. **Reuse before writing.** Permissively licensed DSP (reverb, compression…) is reused when it's good, wrapped behind our own modules.
7. **Everything is data.** Patches, lessons, challenges and duel codes are versioned files or short codes.
8. **Agent-friendly by design.** Every action in the UI exists as a CLI operation with JSON output and real units.
9. **Open.** MIT licence, no GPL code.

## Non-goals (for now)

- A DAW, sequencer or sampler workstation.
- A free cable-patching canvas (the engine is a graph internally, so a cable view can come later).
- Onboarding flows and marketing polish — the first users already know what it is.
- A website, accounts or servers. The core still builds for wasm32, so a website stays possible.

## What "v1" means

A CLAP (and VST3) synth on Linux first, then Windows and macOS, with: a Syntorial-sized engine (2–3 oscillators, noise, a multimode filter, envelopes, LFOs, a modulation matrix, macros, a reverb and a compressor), the Recreate mode with its randomizer settings and scores, a first lesson pack, and the `enth` CLI with `describe`, `check`, `compare` and `suggest`.
