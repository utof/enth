# R1 Recreate + S1 Instrument 0.1: research summary

- **Date:** 2026-10-20
- **Reports:**
  - `01-recreate-game.md`
  - `02-scoring-coaching.md`
  - `03-instrument-editor.md`
  - `04-platforms-distribution.md`
  - `05-risks-tests-plan.md`
- **Status:** the owner reads this at their own pace, and work proceeds on the assumption they agree. Items marked **owner** need a yes, or carry a stated default.

## Verdict

**R1 and S1 can run in parallel on what exists.** Most of the risk is in the game logic's determinism and in score fairness, not in the plugin.

1. **The hooks are already there.**
   - The patch model has every structural op, with an exact undo for each.
   - The editor has an `Overlay` with Good/Near/Bad rings, and reserved challenge-bar and results regions behind the `recreate` feature.
   - The engine plays the starter superset patch (every P1 module, extras off) at no cost for disabled modules.
   - The real-time headroom is about 2.5×.
2. **Targets can be deterministic across machines** if every random draw is keyed by its parameter address, never taken in sequence. A 250-line prototype gave identical target hashes on x86-64 and wasm for 3,000 seeds. Raising the difficulty kept the easier values unchanged (01, measured).
3. **The sound score needs three fixes to feel fair:**
   - **Equivalent patches score as different:** start phase and noise follow module ids, so re-adding an oscillator costs more than a third-of-an-octave cutoff error (02, measured).
   - **Short attacks barely register:** 5 → 20 ms moves the metric by at most 0.003 (01 and 02, measured).
   - **A single global pass threshold is unfair:** an octave of cutoff is 0.05 on a pluck and 0.54 on a pad (02, measured).
4. **The sound score is not yet bit-exact across platforms.** Its loudness gain comes from `ebur128`, which uses `std` maths (05, documented). That is harmless for solo rounds, but fragile for CI golden challenges and for R5 duel codes.
5. **The fork already compiles for Windows and macOS** (04, measured: check and clippy pass). Friends can run unsigned builds with one manual step on macOS. **CI minutes were the real limit:** 1,458 Linux and 418 Windows minutes in the repo's first ~1.1 days. Docs-only pushes no longer run workflows (coordinator commit, 2026-10-20).
6. **Score disputes are the top product risk.** Syntorial keeps a support path for "I disagree with my score" (05, documented). A three-way "felt too low / right / too high" vote and a local round log go into the first playable round. They also measure GR1's "mostly agree".

## Decisions

| Area | Decision | Evidence | Status |
| --- | --- | --- | --- |
| Target generation | Keyed streams: each value is drawn from `derive(derive(seed, fnv1a64(address)), attempt)`. A redraw `attempt` gives the minimum audible step from the start. Generation is parameter space only (design.md §9) | 01 (measured) | Adopted |
| Priors | Hand-written per-parameter priors relative to the phrase's note (e.g. LP cutoff ≥ 1.5 × f0); resonance and drive skewed low; envelopes as a pluck/pad mixture. The `musical` slider blends towards uniform (later the ML-sampling knob). Targets are quantized to reachable values: 3 significant digits, depths in 5 % steps | 01 (silent 0–1 in 200 vs 5–8 uniform) | Adopted |
| Observability mask | Never randomize what the phrase can't reveal: gauges, dead parameters, nuisances. The same mask feeds structure % and the overlay | 01 (measured 0.0000 distances) | Adopted |
| Structure in R1 | Randomize enable flags and routes over the starter superset; add/remove comes with S1 lane AM | 01 | **Owner** (default: yes) |
| Difficulty | Five levels, one category each, calibrated by solver and GR1 pass rates, not by distance (distance plateaus after the filter level). Optional auto staircase (two passes up, one fail down) | 01 | Adopted; GR1 tunes it |
| `RandomSpec` and codes | Integer-only fields plus `generator: u16`. Crockford base32 with a check symbol: about 13 characters short, about 32 custom. The code carries the generator and engine versions and a target-hash prefix, and a mismatch is refused. Old generator versions stay frozen, with hash goldens on Linux, Windows and wasm | 01, 05 | Adopted |
| A/B | **Target ▶** is a pre-rendered, loudness-matched loop of the scored phrase. **Mine ▶** plays the same phrase live through the player's engine, in sync; A/B flips at the same position, and the louder side is attenuated. No second engine | 01 and 02 agree on Target; 01's live Mine chosen over 02's two prerendered loops, so the player hears edits at once | Adopted |
| Sound % | Metric v1 plus fixes: (a) map the player's modules onto the target's ids before the scoring render, and subtract the target's own nuisance floor; (b) metric **v1.1**, pooling mel power over about 85 ms before the log (nuisance 2–20× lower, ρ unchanged at 0.81); (c) descriptor components beside the headline (attack, pitch in cents, brightness, envelope, noisiness, movement); (d) distance in per-challenge JND units, then a curve fitted to ratings | 02 (measured) | Adopted. A vs B and v1.1 wait for metric session 2 |
| Scored-path loudness | An in-house BS.1770 loudness on `libm` for scoring, so sound % is bit-exact across platforms | 05 | Adopted (lane SS) |
| Structure %, combined, time | Canonicalise, match modules per kind, compare only active parameters with sensitivity-based tolerances. Combined = w·sound + (1 − w)·structure, with w from 0.5 to 0.8 by difficulty. **Only sound decides pass.** Time = the first passing submit, from shell timestamps | 02 | Adopted |
| Feedback | No live score by default. Submit shows Good/Near/Bad rings, which then update live. Hints and reveal mark the round as assisted. GR1 tries a live meter as a variant | 01 | Adopted; GR1 decides |
| Coach and CLI | Target descriptors become an `Expect` list for `enth-solver::hint`, unchanged. `compare` and `suggest` use the game's scorer; `match` is the spike's CMA-ES ported into `enth-solver`; `explain` comes from the catalogue | 02 | Adopted |
| Code placement | `enth-challenge` (L4): generator, phrases, codes, scoring. `enth-session`: a pure `game` module, with time and jobs supplied by the shell. UI and plugin: views, the buffer player, a worker | 01, 05 | Adopted |
| GR1 data | A local JSONL round log (code, patch, scores, time, agreement vote), a Disagree button, `cargo xtask gr1-report`. Agent baseline on the humans' 20 codes, beside no-op and CMA-ES floors | 01, 02, 05 | Adopted |
| Add/remove modules | Card "⋯" menu, right-click on empty region space, a dashed "+ Add" slot. One op and one undo step each | 03 | Adopted (lane AM) |
| Init patch | A fresh instance plays the starter patch as "Init". **Gain-staged:** 04 measured the default peaking at +7.9 dBFS on a four-note chord, so Init must peak at or below −1 dBFS on a four-note chord. `Patch::init()` stays minimal for the CLI and tests | 03, 04 | Adopted |
| Presets | Exactly `Patch::to_json()` per `.patch.json`. Folders are directories; factory tags use a vocabulary. User presets in `Documents/enth/Presets` (`dirs` 7.0.0, MIT/Apache-2.0), overridable with `ENTH_PRESETS_DIR`. Header ◀ name ▶ / Browse / Save; the browse panel covers the body in any layout | 03 | Adopted |
| Preset loads | An editor preset load is one undo step (`ops_between`, moved down from the CLI's verified `diff_ops`). It is delivered like a load with **no host writes**, and it marks the project dirty | 03 (nih-plug #92: per-parameter writes crashed VST3 hosts) | Adopted |
| Factory presets | The owner's 4 G1 sounds. Agents draft about 20 more through the CLI, normalised by `xtask preset-add`, and the owner keeps 6 | 03 | Adopted; owner picks |
| Recreate and Learn in the layout | The challenge bar shares the header row; results take the overview slot. A Learn pane of 320 px works only when the lesson hides unused modules | 03 (measured at 1200 × 800) | Adopted |
| Minimum window | At 1000 × 680, lanes drops the overview, so the drag sources vanish. Raise the minimum, or make the variants handle it. Claude Design gets the `LayoutSpec` vocabulary and both sizes | 03 (measured) | Open: lane AM measures, then LV |
| Frame rate | Ask for 1/60 s while live data changes (baseview paces at the X Present `msc + 2`), and count frames under `editor-counts` | 03 (code read) | Adopted; owner check 1.3 confirms |
| Poly-mod | Already implemented and test-host-checked; only the owner's Bitwig check is left (about 20 min). **05's PM lane is dropped** | 03 | Adopted |
| Friend build | One `release.yml` on a tag or by hand, never on push: Linux, Windows MSVC `+crt-static`, a universal macOS bundle. clap-validator on each leg. About $1–1.5 per run. Plus a per-push Windows and macOS cross-target clippy guard on the Linux runner | 04 | Adopted |
| Standalone app | `enth-standalone` (L8) on nice-plug's `standalone` feature, for Logic users and friends with no DAW. It pulls `cpal`/`midir`, and `jack` on Linux, which AGENTS.md says needs the owner's OK | 04, 05 | **Owner** (default: plugin bundles first; standalone after OK) |
| Signing | R1: unsigned, ad-hoc signed macOS bundles with an `xattr -dr com.apple.quarantine` line, and no Windows signing (Smart App Control blocks unknown DLLs, which is rare). Public S1: Developer ID + notarization ($99/yr) and Windows signing | 04 | Adopted for R1; S1 later |
| AU | Deferred past S1 (clap-wrapper ≥ 0.16.0 via CMake if ever) | 04 | Deferred |
| Soak, RTSan, crashes | `cargo xtask reaper-soak --minutes 60`, local, notes from the virtual keyboard (REAPER's transport doesn't run under Dummy Audio). RTSan in the test host around `process()`. A local panic-hook crash file | 04 | Adopted |

## Corrections to the reports

| Where | Said | Correction |
| --- | --- | --- |
| 05 | A poly-mod proof lane (PM) | Poly-mod is implemented and test-host-checked (03, PL's tests). PM is dropped; the Bitwig check stays the owner's |
| 05 [A1] | The A/B mechanism might need engine work | No engine work: Target is a pre-rendered loop, Mine is the live engine (01, 02) |
| 03 | "Fix the engine-version mismatch first" | Fixed on main (lane VER, `166b389`) |
| 04 | Add Markdown `paths-ignore` | Done on main (coordinator, 2026-10-20) |
| 04 | REAPER soak probe: editor open/close grew memory by about 0.5–0.7 MB per cycle | Conflicts with the test host's 12.4 KB per cycle (leak-test). Not yet explained: lane SOAK must attribute it (REAPER's own windows, or ours) |
| 04 | `nightly.yml` has never run | Its schedule exists. Lane FB finds out why, and starts it once by hand |

## Open checks

| Check | Owner | When |
| --- | --- | --- |
| Metric session 2 (test–retest), then A vs B and v1.1 | Owner, ~15 min | Before lane SS fixes the curve |
| A real-engine pairs session (15–20 min); every rating so far is from the toy synth | Owner | After SS, before GR1 |
| Friend survey: OS and DAW (5 min) | Owner | First; it sets FB's targets |
| GitHub plan and minutes (the billing API needs the owner's `user` scope) | Owner | Soon |
| The REAPER editor-cycle memory growth | Lane SOAK | Wave 1 |
| Linking and running on real Windows and Mac machines | Friends | The first friend build |
| Live meter vs score-on-submit; difficulty steps | GR1 data | GR1 |

## Lane plan

The full table is in report 05 §1. Adjustments:
- PM is dropped;
- AM also owns the Init patch's gain staging and the minimum-window measurement;
- PBC owns undoable loads;
- SOAK attributes the REAPER memory growth;
- FB owns the cross-target guard, `release.yml`, `docs/distribution.md` and the nightly check. The standalone waits for the owner's OK.

| Wave | Heavy | Light | Starts when |
| --- | --- | --- | --- |
| 1 | S0r (workspace prep → F0r) | FB, SOAK, EV2 | Now |
| 2 | AM | CT, SG (→ F1r, judge-reviewed), RG, PBC | F0r |
| 3 | SKR (round skeleton → F2r), PBU | SS, ST, SN, MT, GC, PR | F1r (PBU after PBC) |
| 4 | RUI | CL, GK, EV2's recreate track | F2r, SS, ST |
| 5 | INT | — | Everything; GR1 and GS1 tables, friend release |
| Later | LV | — | The owner's layout pick |

- **Size:** about 22–42 agent-hours planned; expect 26–30, calibrated on P1b's 20 h measured against its 27–51 estimate.
- **Critical path:** S0r → CT + SG → SKR → RUI → INT, about 8–11 h of wall time with merges.
- **What really sets GR1's end date** is the friends' calendars, which is why FB starts in wave 1.

## Docs to change

| Doc | Change |
| --- | --- |
| `docs/phase-2.md` (new) | The R1 + S1 lane plan, freezes F0r/F1r/F2r, acceptance per lane (from 05, with the adjustments above), owner gates |
| `docs/roadmap.md` | R1 and S1 point to phase-2.md; a dated change entry; the friend build's form (bundles first, standalone after OK, unsigned for R1) |
| `docs/design.md` §9 | Keyed streams, observability mask, integer `RandomSpec`, codes and generator versions, reachable quantization, structure over the starter superset; the A/B mechanism; score-on-submit feedback |
| `docs/adr/0007-*` | A dated note: v1.1 pooling, mapping modules before scoring, descriptor components, per-challenge JND calibration, `libm` loudness in the scored path; A vs B waits for session 2 |
| `docs/architecture.md` | `enth-challenge` (L4) responsibilities; the session's `game` module; `enth-standalone` (L8) once approved |
| `docs/editor.md`, `docs/design-system.md` | Add/remove entry points; presets header and browse panel; Recreate in the header row and results in the overview slot; minimum window |
| `AGENTS.md` | Docs-only pushes run no CI; `release.yml` is tag or manual only |
