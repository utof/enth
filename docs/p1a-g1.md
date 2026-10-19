# P1a: the headless G1 table

- Date: 2026-10-19 (P1a lane E). Engine 0.2.0, main at `d58c5cb` plus lane E.
- Source of the gate: [roadmap.md](roadmap.md#p1--minimal-synth--cli-v0-46-weeks) (Gate G1) and [phase-1a.md](phase-1a.md#the-headless-g1-checks). The agent-eval protocol is in [agent-cli.md](agent-cli.md#evaluating-the-cli-like-a-product); this page records what was built and where each check stands.
- Status words: **passed** (measured or green in CI, with the source), **pending owner** (needs your ears), **pending coordinator** (needs the paid run or a CI trigger that costs money), **failed**. Nothing is failed.

## The table

| # | Check | Status | Source |
| --- | --- | --- | --- |
| 1 | 16 voices: at most 12,000 instructions per frame, and at least 10× realtime on one core | **passed**: 6,154 instructions per frame (590,803,521 per 2 s render), 23.5× realtime (median; 25–26× best run) | [`benches/g1.rs`](../crates/enth-engine/benches/g1.rs); [numbers below](#1-the-16-voice-cpu-budget). The `p99 process()` half of the criterion is P1b (it needs the test host) |
| 2 | Patches round-trip through JSON | **passed**: bit-exact for every finite f32, fixtures for every schema version, non-finite numbers refused, fuzz-lite never panics | S1b: [`props.rs`](../crates/enth-patch/tests/props.rs) (`json_round_trip_is_bit_exact`, `every_parameter_value_round_trips`, `wire_form_round_trips`), [`migrate.rs`](../crates/enth-patch/tests/migrate.rs), [`fuzz.rs`](../crates/enth-patch/tests/fuzz.rs); `cargo test -p enth-patch` green on 2026-10-19 |
| 3 | Goldens on Linux, Windows and wasm32 on every push, reverb and compressor included | **passed**: `golden` workflow run 39264457142 on `d58c5cb`: Linux x86_64, Windows x86_64 and wasm32-wasip1 under wasmtime all success. The 72 cases include `reverb-*` and `comp-*`. `engine-golden` run 39264457218 also success | `gh run list --workflow golden.yml`; [`.github/workflows/golden.yml`](../.github/workflows/golden.yml); `cargo xtask golden` is `ok` in the lane E worktree |
| 4 | Goldens on macOS, once before G1 | **passed**: `golden` run 39273104073 (`workflow_dispatch`, 2026-10-19) on macOS **aarch64**: all 72 cases identical to the x86_64 hashes. Renders are now bit-identical on Linux x86_64, Windows x86_64, macOS arm64 and wasm32. (That run's Windows leg failed to compile `xtask` on a dead-code warning, fixed in the same commit as this table update) | `gh run view 39273104073` |
| 5 | The reverb was chosen by a blind, level-matched A/B | **pending owner** (20–30 min; the FX1 wrapper has landed, so it can be done now): `cargo build --release -p enth-listen`, then `./target/release/enth-listen reverb-ab --dir ~/enth-reverb` and `... --analyze`. ADR-0006 is confirmed if ours is not clearly worse | [listening.md](listening.md#2-reverb-ab-our-signalsmith-port-against-fundsp) |
| 6 | An agent passes the text-goal evaluation: at least 5 of 10 goals within 20 renders (given lists), 3 runs each, both tracks, and a blind listen with at least 7 of 10 | **passed (agent part), pending owner (blind listen)**: Claude Sonnet 5.5 (`claude-sonnet-5-5`), 2026-10-19, run as fresh subagents with only the `enth` binary and a 20-render budget. **Given track: 30/30** (10/10 in each of 3 runs, 1–5 rendering calls per goal). **Self-written track: 15/30** against the designer lists (6, 4, 5 per run). Misses cluster on bell-soft, brass-ish, filter-wobble. Judged by the coordinator with `enth check --no-fix`, the goal's performance forced. Sounds for the blind listen: `~/enth-listen/g1-eval/{given,self}/<goal>.wav` | [section below](#6-the-agent-evaluation); raw: `~/code/enth-scratch/p1a-eval/` |
| 7 | "A bass, pad, pluck and lead you like" | **moved to P1b** (it needs the editor) | roadmap.md, 2026-10-19 (d) |

Checks 1–4 and the agent part of 6 are done. What is left of G1's headless part is the owner's: the reverb A/B (5) and the blind listen of the agent's sounds (6).

## 1. The 16-voice CPU budget

The reference patch is the one in [`benches/g1.rs`](../crates/enth-engine/benches/g1.rs): polyphony 16; two oscillators (saw, and a pulse an octave down, detuned) and noise into a resonant low-pass per voice; an amp and a filter envelope; a vibrato LFO per voice and a slow global LFO; compressor and reverb; eight routes (envelope, LFOs, velocity, key, a macro); a held 16-note chord, 2 s at 48 kHz in 512-frame host calls. It uses every P1a module kind, with all real modules (D1, D2, FXW).

| Measure | Value |
| --- | --- |
| Instructions, 2 s render (callgrind through gungraun 0.20.0, valgrind) | **590,803,521** (identical on two runs) |
| Per stereo frame | **6,154** (G1 cap: 12,000) |
| Hard limit now in CI | 650,000,000 = measured + 10 % (6,771 per frame); `const`-asserted to stay within the 1,152,000,000 cap |
| L1 hits / LL hits / RAM hits | 759,464,564 / 3,608,392 / 12,840 |
| Estimated cycles | 777,955,924 |
| Wall-clock, release build, 16 voices, 2.000 s of audio | median **85 ms** (min 76–79 ms, worst 104–116 ms over 30 runs, twice): **23.5× realtime** (25–26× best, 17–19× worst) |
| The same patch with 1 voice | median 44.6 ms: 44.8× realtime (the reverb, compressor and per-sample fixed work dominate) |
| Before (S2b, stub modules) | 122,452,870 (1,276 per frame) |
| The Phase 0 prototype | 6,353 per frame, 22.7× realtime |

- How: `GUNGRAUN_RUNNER=<gungraun-runner 0.20.0> flock -x /tmp/enth-quiet.lock cargo xtask bench --bench g1`. Without the runner, `cargo xtask bench` says what to install. CI runs it in the `budget` workflow (`cargo xtask bench`), where the new limit takes effect on merge.
- The limit is real: setting it to 500,000,000 makes the bench fail with `Instructions (590803521) exceeds limit ... (>500000000)` and exit 3.
- The wall-clock figures are information, not a gate (the project has no wall-clock assertions). They were taken on the dev laptop (Intel i7-9750H, 6 cores / 12 threads), `uptime` load average about 7 at the start (other lanes were active; the exclusive lock only stops other builds), so read the median and the best run, not the worst. Method: the same `g1_patch()` and `common::render` as the bench, compiled `--release` as a temporary example, 30 renders timed with `Instant`, nothing else in the process. At 23.5× the patch uses about 4.3 % of one core.
- Margin: the 10 % over the measurement is the project's rule (docs/benchmarks.md). The distance to the cap (12,000) is large (a 1.95× margin), so G1's number can absorb the P1b effects (the crossfade at a plan swap doubles the cost for 256 frames) without touching this limit.

## 2. JSON round trip

S1b's property suite is the evidence: `json_round_trip_is_bit_exact` over generated patches, `every_parameter_value_round_trips` over values drawn across the representable f32 numbers of every parameter of every module kind (subnormals and the neighbours of the bounds included), `wire_form_round_trips`, fixtures for every schema version (`migrate` is the identity on the current one, a future version gives `TooNew`), `negative.rs` (non-finite numbers give a typed error), and fuzz-lite (arbitrary and mutated JSON never panics and returns a typed error with a hint). All green in `cargo test -p enth-patch` on 2026-10-19. The agent CLI adds nothing here: `enth` reads and writes the same files.

## 3–4. Goldens

`golden.yml` runs the same 72 cases on Linux x86_64, Windows x86_64 and wasm32-wasip1 under wasmtime on every push, and `engine-golden.yml` runs the engine hashes on Linux, Windows and wasm, plus the callgrind budgets. Both were green on main `d58c5cb` (run ids in the table). The macOS leg (arm64 runner) is a manual or weekly job, and has not run: the first one shows whether aarch64 agrees with the x86_64 hashes, and it is the only way G1's "and on macOS, once" is met. Lane E's changes touch no golden hash (the `svf-*` descriptions changed, text only; `cargo xtask golden` is `ok`).

## 5. The reverb A/B

Nothing to add from lane E: the tool is `enth-listen reverb-ab` (lane T), and the sources were rendered through the placeholder oscillator and filter, so [listening.md](listening.md) suggests rendering again in a new folder now that D1 and D2 have landed. Answer files go under `~/enth-reverb` (not in git).

## 6. The agent evaluation

### What was built

- **Ten goals** (`content/agent-evals/goals/*.toml`), one per file, each with an id, a brief (the only text on the self-written track), the performance the judge forces, the designer's expectation list, a rationale (for maintainers; the agent never sees it) and a budget (20 renders, 60 turns, $1.00). Difficulty 1 to 5:

  | d | Goal | Brief | Notes (performance) |
  | --- | --- | --- | --- |
  | 1 | `pluck-bright-short` | a short bright pluck | A3, 1.5 s |
  | 2 | `kick-thump` | a short kick-like thump | A1, 0.3 s |
  | 2 | `pad-slow-dark` | a slow dark pad | C3, 3.5 s |
  | 3 | `bass-buzzy` | a buzzy bass | C2, 1.5 s |
  | 3 | `hat-closed-noise` | a short closed hi-hat made of noise | C3, 0.3 s |
  | 4 | `bell-soft` | a soft bell-like tone | C5, 3 s |
  | 4 | `brass-ish` | a brassy sustained note with a slow swell | E3, 2 s |
  | 4 | `filter-wobble` | a low saw with a slow wobbling filter sweep | A2, 4 s |
  | 5 | `lead-supersaw` | a detuned supersaw-ish lead | A4, 2 s |
  | 5 | `snare-crack` | a snare-like crack: a noise burst over a short tonal body | A3, 0.3 s |

  Every goal is reachable with the current engine: each has a hand-picked solution (`witness/<id>.toml`) that passes both its designer list and the witness's own list. The percussion goals (kick, hat, snare) are short sounds and are measured as such: no goal drops or pads a sound by its duration.
- **`cargo xtask agent-eval`** (`xtask/src/agent_eval.rs` and `xtask/src/agent_eval/`). Each goal runs on two tracks: *given* (the designer's list is in the prompt; gated) and *self* (the agent gets only the brief and writes its own list; the designer's list judges it; reported, not gated, with the agent's own-list pass rate as a second number). 3 trials per goal (so 60 trials), each in an **empty temp dir** whose only content is a wrapper named `enth` on `PATH`. The wrapper counts renders (`render`, `step`, and `describe` or `check` of a patch), **refuses the 21st with exit 9 (`budget_exhausted`)**, and logs every call as JSONL. The judge is `enth check --no-fix` on the agent's final patch with the goal's performance forced, so an agent cannot pass by judging a different note. A trial passes when the designer list passes; a goal passes when a majority of its trials do (pass@3 and pass^3 are also printed).
- **Records.** Per trial: `result.json` (model id and date, Claude Code version, tokens in four classes, estimated cost, renders, calls, invalid-command rate, pass, the margin of every list item and how many were thin, the final patch path), plus `stream.jsonl` (the raw transcript), `final.patch.json`, `final.wav`, the judge outputs and the call log. Per run: `header.json`, `summary.json` (medians per goal and track), the tables above, and a committed summary at `content/agent-evals/results/<date>-<model>.json` (summaries only; everything else stays in `target/agent-eval/`).
- **The blind check.** The final patches' WAVs are level-matched to −23 LUFS with a −1 dBFS peak ceiling and copied to `<run>/blind/NN.wav` under shuffled names; the key (`blind-key.json`) is separate and `agent-eval listen` shows only the brief, never the numbers.
- **`--dry-run`** uses a scripted fake agent and no model: the *witness* replays each goal's hand-picked solution, the *follower* applies `check`'s `fix.ops` until nothing more is offered. Both go through the same wrapper, budget counter, judge and report as a real run, and write a transcript in Claude Code's `stream-json` shape that the real parser reads. The costs and tokens in a dry run are synthetic (bytes over four; Sonnet 5.5 list prices) and the report says so.

### What the dry runs say

- **Witness (10 goals × 2 tracks × 3 trials):** 10 of 10 goals pass on both tracks, every trial, in one render. So each expectation list is reachable with the current engine and the judge, wrapper and report agree.
- **Follower (the mechanical baseline; no model):** 2 of 10 pass from the template the witness starts from (`pluck-bright-short`, `pad-slow-dark`). The other eight stop at a `blocked` verdict because they need structure the hint solver cannot add (a second oscillator, noise, an LFO and a route, a filter envelope). This is the floor a model has to beat on the given track, and it says what the hints do not yet do: they move parameters, and a person (or an agent) adds modules.
- **Real-runner path exercised with stand-ins, not a model:** a stand-in `claude` script (via `CLAUDE_BIN`) was parsed correctly (model id, version, tokens, cost, structured answer), a hung process hit the timeout and was recorded as `timeout`, and a script that attempted 25 renders was allowed exactly 20 and refused 5 with exit 9.

### Findings from designing the goals (for the descriptor and CLI owners)

These are properties of the current analysis, found while making every goal reachable. None blocks G1; they decide what a list may ask.

- `noise_ratio`-based items (`noisiness`) need about 171 ms of held sound; `decay` and `sustain` need the note held more than attack + 80 ms. So the percussion goals hold the key 0.3 s and make the sound short with an envelope sustain of 0.
- `pitch` is null for a 55 Hz sine that dies in under about 200 ms, for sounds with `noisiness` above about 0.44, and for `bass_sub` at C2 (its sub oscillator is an octave below). The kick therefore decays over at least 400 ms and the snare has no pitch item.
- The `attack` descriptor is inflated on low notes by the oscillator period (a C2 saw with a 3 ms attack knob reads 12 ms, and the margin is thin), so the bass asks for `attack < 40ms`.
- A filter-cutoff LFO reads as `tremolo` (2.01 Hz, 6 dB), but `attack` and `sustain` under tremolo are not corrected (attack 307 ms, sustain +4.9 dB), so `filter-wobble` carries no attack or sustain item.
- The `pad_2osc` template clips (+1.9 dBFS at C3), so the lists carry `peak < -1dB` and the witnesses lower the master level.
- In the given track the agent can see that the 20-render budget is generous: the witnesses use 1 render, so a real run's render count is the measure of the feedback, not the pass rate.

### The real run

The real run needs Claude Code (`claude`, 2.1.296 here) logged in, and it **costs money**, so `--confirm-spend` is required and nothing in lane E ran it. On a claude.ai subscription (no `ANTHROPIC_API_KEY`) the harness uses `--safe-mode` instead of `--bare`, which `--auth subscription` (or `--auth auto`) selects; with an API key it uses `--bare`. The cost figure is Claude Code's own client-side estimate (`total_cost_usd`), not a bill; against a subscription it is usage, not dollars.

```bash
# 1. Smoke test: one goal, one track, one trial (a few cents). Look at target/agent-eval/<stamp>/ first.
cd ~/code/enth && cargo xtask agent-eval --confirm-spend --goals pluck-bright-short --track given --trials 1 --model claude-sonnet-5-5

# 2. The full run: 10 goals x 3 trials x 2 tracks = 60 trials; the plan estimated $9. It stops starting trials once the estimated cost reaches --max-usd.
cd ~/code/enth && cargo xtask agent-eval --confirm-spend --track both --trials 3 --model claude-sonnet-5-5 --max-usd 20

# 3. The owner's blind listen, afterwards (plays audio, so run it yourself):
cd ~/code/enth && cargo xtask agent-eval listen target/agent-eval/<stamp>
```

- The harness builds `enth` (release) itself unless `--enth PATH` is given; the first build takes a few minutes. Builds are brief; the run itself is mostly waiting on the model, so it does not need the exclusive lock. `--jobs 1` is the default; each trial has a 15 minute timeout (`--timeout-min`).
- The full run writes `content/agent-evals/results/<date>-<model>.json`; commit it. G1 passes check 6 when the *given* track shows at least 5 of 10 goals passed and the owner's blind listen says at least 7 of the 10 final sounds match their text. The first run's cost is recorded against the $9 estimate in that summary and in `summary.json`.
- **Not verified without a model:** the exact flags and `stream-json` field names of a first real call. The parser is defensive (a missing field leaves a value empty and adds a warning to the trial's `result.json`; the raw `stream.jsonl` is always kept), and the flags (`-p`, `--output-format stream-json --verbose`, `--tools Bash`, `--allowedTools "Bash(enth *)"`, `--permission-mode dontAsk`, `--max-turns`, `--json-schema`, `--append-system-prompt-file`, `--no-session-persistence`) were checked against `claude --help`, parse probes and the headless documentation only. Run the smoke test first; if a flag is rejected, `claude` exits at once without a model call, the trial's status is `agent_error` and `claude.stderr` in the trial directory says why, so the smoke test costs next to nothing.
- Changing a prompt or a goal's list changes the eval version (`content/agent-evals/eval.toml`), which is recorded in every result.


### Eval findings to act on (2026-10-19 trial)

Agents reported, consistently across runs:
1. **Envelope readings vs knobs:** measured release and decay read about half the knob value (the descriptors measure to −40 dB, the knobs set the RC segment time). The guide or `params` should state the relation, or the knobs should be labelled by the time they produce.
2. **Route depth typed as `+12st`** becomes a percentage of the target's range (12.5 % of 96 st) silently. Show the conversion in `apply`/`step` output.
3. **`bass_sub`'s sub-octave makes pitch read as absent**, and `blocked` says "structure is the limit" without naming the module. Name the module, or fix the template.
4. **`pad_2osc` clips** (+1.9 dBFS) out of the box. Lower its master level.
5. **A bare `clipping = false` is always `thin`** (boolean margin 0). Treat booleans as not thin.
6. **The tremolo descriptor only fires on deep filter sweeps**, so a "filter wobble" is hard to express. Consider a `filter_sweep` / brightness-modulation descriptor (listed in agent-cli.md, not built).
7. **Noisy sounds lose pitch above noisiness ≈ 0.2–0.4.** Snare-like goals can't ask for both, so the pitch descriptor needs a "tonal component under noise" mode.
