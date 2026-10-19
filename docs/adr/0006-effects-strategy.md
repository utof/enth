# ADR-0006: Effects — port permissive DSP into our own deterministic modules; no plugin hosting

- Status: **Proposed** — confirm at Gate G1, after a wasm32 build check and a blind A/B pick the reverb
- Date: 2026-10-19

## Context

- P1 needs a reverb and a compressor. The owner asked whether enth could plug in an external reverb instead of building one.
- `render()` must be a pure function of patch, performance, sample rate, block size, seed and engine version ([design.md §5](../design.md#5-determinism)). Scoring, lessons, the CLI, ML datasets and seed-based duels all re-render patches offline, and Recreate's top difficulty randomises effects, so effects sit inside the scored path.
- Core crates (L0–L5) must build for wasm32, do no I/O and use no global RNG; the audio path never allocates.
- The research session `_local/sessions/2026-10-18-effects-reuse-and-plugin-hosting/` (short version in [research.md §8](../research.md#8-reusing-effects-and-hosting-plugins)) found:
  - Signalsmith Basics `reverb.h` (MIT, ~409 lines, a diffuser feeding an FDN, real-unit parameters, explicitly seeded, last commit 2026-09-30) is the best permissive reverb to own. fundsp 0.23 (MIT OR Apache-2.0) has four deterministic reverbs and needs no port, but brings an 81-crate wasm32 tree and no compressor.
  - No maintained, permissively licensed Rust compressor exists.
  - The real reuse risk is hidden behaviour: CloudSeedCore, Airwindows and the `basic-reverb` crate read unseeded global RNGs; CloudSeedCore also allocates on parameter changes and has no denormal protection.
  - Mixed-licence repos hide copyleft: JUCE modules (AGPLv3 since JUCE 9), DaisySP's LGPL submodule, chowdsp's DSP modules, GPL-declared Faust functions.
  - Hosting a CLAP plugin inside enth is possible with `clack-host` (0.2.0, 2026-09-12), estimated at 4–8 weeks plus shell work. It breaks bit-exact renders, the CLI and preset sharing, and adds nothing over the same reverb placed after enth in the DAW. Users of other synths ask for a global FX bypass, not hosting.

## Decision

- **Effects are our own modules.** Each effect is ported from a permissively licensed source (or written from textbooks) into `enth-dsp`, wrapped behind our `Module` trait, and owned from then on. No crate that wraps C/C++ in L0–L5.
- **Reverb (P1):** port Signalsmith Basics `reverb.h` and its MIT helpers (`delay.h`, `mix.h`, `filters.h`). Replace `std::mt19937` with the engine's seeded PRNG (seeded from the patch hash and module id), or reproduce mt19937 exactly for the first golden test against the C++. That test can be bit-exact if neither side uses fast-math: a Rust port of Google's SynthMark voice matched its C++ `-O2` build bit for bit (a C++ `-Ofast` build differs by up to 9.5e-3). Allocate in `prepare`; flush denormals in code. **Fallback:** fundsp 0.23 `reverb2_stereo`/`reverb4_stereo` with `default-features = false, features = ["std"]` (2026-10-19: fundsp 0.23.0 and master fail to build without `std`), pinned to an exact version (`=0.23.0`) and wrapped behind `Module`.
- **Compressor (P1):** in-house, a few hundred lines: feed-forward stereo-linked detector, attack/release, soft knee, make-up gain. The algorithm follows Julius O. Smith's Faust `compressor_mono` (STK-4.3), hand-ported rather than shipped as Faust compiler output; web-audio-api-rs `dynamics_compressor.rs` (MIT) is the Rust cross-check.
- **Zero latency.** Effects have no look-ahead and report no latency unless a later ADR says otherwise, so P1 never triggers a CLAP latency restart.
- **Global FX bypass.** A persistent per-instance "dry out" switch, saved with the plugin's state but outside the `Patch`, so it survives preset changes. Scoring and CLI renders ignore it. The docs tell users to put their own reverb after enth in REAPER.
- **No third-party plugin hosting in the engine.** If a concrete need appears after GS1, the only shape considered is a CLAP-only, in-process post-FX slot in `enth-plugin`, after a dry scoring tap, flagged non-reproducible and bypassed by the CLI by default, preceded by a one-day spike. VST3/LV2 hosting, embedded third-party editors and out-of-process sandboxing need a new ADR.
- **Provenance.** Every port records upstream URL, commit, licence and copyright in `THIRD_PARTY_NOTICES.md` and in `docs/dsp/<module>.md`, along with its deviations from upstream (PRNG, denormals, sample-rate scaling). [ADR-0004](0004-licensing.md) sets which licences ported source may carry.

## Consequences

- Effects can sit inside the scored path: golden tests, scores, lessons and duels stay reproducible, and `enth-dsp` itself becomes the reusable asset (and the boundary for shipping modules as their own plugins in S3).
- We maintain ported code ourselves; upstream fixes are pulled by hand.
- A user's favourite reverb is never part of a patch, a score or a CLI render, and presets don't carry it.
- Each port costs days, not weeks (estimated 2–4 days for the reverb; unvalidated).
- Later effects follow the same path (S2): a CloudSeedCore port as a "lush" reverb (credit Ghost Note Audio, no "Cloud Seed" name, buffers sized in `prepare`), chorus/flanger/phaser from DaisySP core or Signalsmith, saturation from Faust `aanl.lib`.

## Alternatives considered

- **fundsp first** — no porting; but a heavy, often-breaking dependency for one reverb, no compressor, no commit since 2026-03-03, and reverb parameters fixed at construction. Kept as the fallback and the A/B reference.
- **CloudSeedCore first** — the best-reputed permissive reverb, but a stale upstream, C++14, process-global `rand()`, ~64 MB of fixed buffers per stereo instance and allocation on parameter changes. Planned as a second reverb in S2.
- **A hosting slot (CLAP via clack-host)** — 4–8 weeks plus shell work; breaks determinism, the CLI and preset sharing; the DAW's FX chain already gives latency compensation, automation, crash handling and a GUI.
- **Full hosting (CLAP + VST3 + LV2, embedded editors, sandboxing)** — months of work for a PatchWork-class product inside a synth.

## How we'd know we're wrong

- The Signalsmith port loses the blind, level-matched A/B to something we can't port.
- The port takes more than twice its estimate, or doesn't build for wasm32.
- Users concretely need third-party effects inside the voice or modulation path, not after the synth.
