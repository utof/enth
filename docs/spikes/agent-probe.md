# Spike: agent-loop probe, part 1 (Phase 0, lane P)

- Date: 2026-10-19
- Code: `spikes/spike-agent` (package and binary `spike-agent`), branch `lane-p/agent-probe`.
- Question it feeds: can an LLM agent reach a text sound goal using only numeric descriptors and pass/fail checks, before the real `enth` CLI (lane P1) exists? ([agent-cli.md](../agent-cli.md), research question Q5.)
- Status: **part 1 only, the instrument.** This crate is the thing to be tested. It does not say whether an agent can use it; a separate, fresh agent runs the five goals, and its transcripts are part 2.

## Spoiler warning

This file and `spikes/spike-agent/tests/e2e.rs` contain a hand-picked solution to goal g1 (and `tests/e2e.rs` can print mechanical solutions for the other goals). Do not point the agent under test at either. Give it `spike-agent guide`, the goal text and its expectation list, and nothing else from the repository. `goals.toml` holds the goals and their expectation lists but no solutions.

## Why a sibling crate

`spikes/spike-metric` is merged and its binary (`solve`, `bench`, `listen`, `heurcheck`) is the metric spike's deliverable; lane C owns it. Adding a second CLI to it would mix two experiments and two sets of tests. `spike-agent` depends on `spike-metric` as a library for what it already does well and must not duplicate: the toy renderer (`render`), the WAV codec, EBU R128 loudness (`features::Analyzer::loudness`) and YIN (`yin`). Tooling crates named `spike-*` are exempt from the layer rules, so the dependency is allowed. Nothing in `spike-metric` changed.

## What exists

| Piece | Where | Notes |
| --- | --- | --- |
| Patch file | `patch.rs` | JSON, `"schema": 1`, `"engine": "toy10"`. The 10 parameters of the toy synth in real units: `osc.wave` (saw, square, triangle), `osc.detune` ct, `filter.cutoff` Hz, `filter.resonance` Q, `filter.env_amount` oct, `filter.env_decay` ms, `amp.attack` ms, `amp.decay` ms, `amp.sustain` dB (−60 means off), `amp.release` ms. A `perf` block holds the performance: `note`, `dur`, `hold`. Normalised 0..1 values never appear. Ranges are those of the metric spike's search box. |
| Units | `units.rs` | `800Hz`, `1.2kHz`, `40ms`, `0.5s`, `-6dB`, `12ct`, `3oct`, `30%`, note names (`C#4`, `Db2`). A bare number is read in the unit `params` shows. A wrong unit is an error that lists the accepted ones. |
| Renderer | `synth.rs` | Calls `spike_metric::render`, quantises to 16 bit, writes the WAV. Descriptors are measured on the quantised samples, i.e. on what the WAV holds, never read off the patch. |
| Descriptors | `descriptors.rs` | See "Descriptors". |
| Expectations | `expect.rs` | `brightness>2kHz`, `attack<10ms`, `sustain<-20dB`, `pitch=C3±20ct`, comma-separated. Short names or canonical paths (`spectrum.centroid_hz`). Ops `<`, `<=`, `>`, `>=`, `=` (with a tolerance). |
| Hints | `hints.rs` | See "Hints". |
| Commands | `commands.rs`, `main.rs`, `guide.rs` | `guide`, `params`, `new`, `apply`, `render`, `describe`, `check`, `step`. One JSON object on stdout per call, sorted keys, numbers rounded to 4 significant digits, `"schema": "enth.<command>/1"`. |
| Errors | `error.rs` | JSON on stdout too (`enth.error/1`): `code`, `message`, a hint (e.g. "did you mean filter.cutoff?"). Exit codes follow agent-cli.md and are listed in `guide`. |

Exit codes: 0 ok (for `check`, all passed), 1 `check` ran but an expectation failed, 2 usage error, 3 validation error (unknown parameter, bad unit, out of range), 4 render or analysis failure, 5 file not found.

### Commands an agent uses

```bash
spike-agent guide                               # once: workflow, commands, descriptors, exit codes
spike-agent params                              # once: every key, unit, range, default, perceptual effect
spike-agent new --out p.json --perf "note=A3,dur=2s,hold=1.5s"
spike-agent apply p.json --set "filter.cutoff=800Hz,amp.attack=40ms" [--dry-run] [--out p2.json]
spike-agent render p.json --out p.wav
spike-agent describe p.json --explain           # descriptors, each with a one-line meaning
spike-agent check p.json --expect "brightness>2kHz, attack<10ms"
spike-agent step p.json --set "..." --expect "..." --out p.json   # apply + render + describe + check
```

`describe` and `check` also accept a WAV (the check then has no hints, because there is no patch to move). `--perf` on `check`, `describe` and `step` overrides the patch's performance for that call; the judge uses it so that changing the performance (a different note, a shorter clip) cannot pass a goal.

### Descriptors

| Name | Path | How it is measured |
| --- | --- | --- |
| brightness | `spectrum.centroid_hz` | Magnitude-weighted spectral centroid, STFT 2048 / hop 512, over frames within 50 dB of the loudest. `centroid_track_hz` is the start, middle and end thirds. |
| attack | `envelope.attack_ms` | 8 ms RMS envelope at a 1 ms hop; time from 1 % of the peak to −1 dB of it. |
| decay | `envelope.decay_ms` | Peak to 90 % of the way (in amplitude) to the sustain level; 0 when there is no decay stage. |
| sustain | `envelope.sustain_db` | Median level over the 70 ms before note-off, in dB re the peak; −80 means silent. |
| release | `envelope.release_ms` | Note-off to −40 dB re the peak; 0 if already gone. A lower bound (`release_truncated`) if the clip ends first. |
| duration | `envelope.duration_ms` | How long the level stays within 40 dB of the peak, from the onset. A lower bound if still audible at the end. |
| pitch | `pitch.f0_hz`, `note`, `cents_dev`, `stability` | YIN (hop 512, frames within 40 dB of the peak), median over the loud frames. |
| noisiness | `noise_ratio` | Welch spectrum; share of energy not in steady spectral peaks. |
| beating | `detune.beating_hz`, `depth_db` | Quadrature demodulation of the fundamental, cubic log detrend (so an ADSR is not mistaken for a wobble), spectral peak of the level modulation; reported only if ≥ 9.5 dB deep, 1 to 25 Hz, and ≥ 6× the surrounding floor. Otherwise `null`: absent, or too slow to measure in the clip. |
| loudness, peak | `loudness.lufs_i`, `peak_dbfs`, `clipping` | `spike-metric`'s EBU R128 integrated loudness; sample peak. |

Descriptors are numbers about this render. They approximate what a listener hears; they are not hearing.

### Hints

A failed expectation on a patch carries `hint {param, now, try, expected, measured, reaches, others, dir}`.

1. Probe: move each of the 9 continuous parameters by ±10 % of its range (log domain for log parameters) and each other waveform, rendering each; this gives finite-difference sensitivities of the failing descriptor.
2. Rank candidates by how strongly and in which direction they move the descriptor (log domain for Hz and ms features).
3. Refine the best two candidates with regula falsi / secant steps, at most 4 renders each, stopping at the mildest value that passes the item. `expected` is the descriptor value measured by rendering exactly that single change, so it is true, not extrapolated.
4. `reaches: false` means no value of that parameter alone gets there (the range is exhausted); `others` lists parameters that push the same way. If the descriptor is absent at the base patch (no pitch, no beating), a wide probe is tried; if that fails the hint is structural text (`measured: false`), e.g. "no steady pitch: lower filter.resonance" or "noise_ratio is high".
5. Truncated tails (a release or duration cut by the clip) get a `perf.dur` hint.

Hints move one parameter at a time. After combining several, the agent must re-check everything; this is stated in `guide`.

## Descriptor audit

Does each descriptor move as the numbers say it should? `tests/audit.rs` (ignored; `cargo test --release -p spike-agent --test audit -- --ignored --nocapture`) sweeps one parameter at a time and prints the reading. Excerpts, saw at A3 unless stated:

| Sweep | Reading | Theory |
| --- | --- | --- |
| `amp.attack` 10, 100, 500, 1000 ms | attack 11, 88, 441, 884 ms | 0.88 × the ramp (1 % to −1 dB); the 8 ms window rounds short ramps up (1 ms reads 2 ms) |
| `amp.sustain` 0, −6, −20, −45 dB | −0.4, −5.7, −19.3, −44.2 dB | identity; −60 dB reads −80 (silent) |
| `amp.decay` 100, 500, 2000 ms | decay 56, 279, 1024 ms | 0.58 × tau (90 % of the way down) |
| `amp.release` 100, 500, 2000 ms | release 116, 571, 2280 ms | 1.15 × tau (−40 dB) |
| `filter.cutoff` 200, 800, 3200, 12800 Hz | centroid 375, 655, 1471, 3057 Hz | monotone once the cutoff is above the fundamental's first harmonics |
| wave at 4 kHz cutoff | saw 1672, square 1466, triangle 501 Hz | saw > square > triangle |
| `filter.resonance` 0.5 to 10 at 1 kHz | centroid 726 to 856 Hz | monotone, small |
| note C2 to C6, any wave | f0 within 0.4 ct | |
| `osc.detune` 20, 30, 50 ct at C3 | beating 1.5, 2.3, 3.8 Hz | 1.52, 2.29, 3.83 Hz (f0 × (2^(d/1200) − 1)) |
| `osc.detune` 10, 20, 30, 50 ct at A4 | beating 2.5, 5.1, 7.6, 12.7 Hz | 2.55, 5.11, 7.69, 12.89 Hz |
| `osc.detune` 5 ct at C3 and A3 | absent (`null`) | beat period (1.6 to 2.6 s) is not measurable in a clip this short; the descriptor says nothing rather than a wrong number |

Beating either matches theory within about 2 % or is absent. An earlier version reported 1.1 Hz for 5 ct and 0.8 Hz for 10 ct at C3 (theory 0.4 and 0.8 Hz), and a decay into a sustain level was read as slow beating; the prominence floor and the log detrend fixed both, and `tests/descriptors.rs` pins the no-false-positive cases.

Known blur, not fixed: with detune of about 15 ct or more the two oscillators beat, and the level wobbles by a few dB. Sustain then reads within about ±5 dB of the programmed level depending on where in the beat the 70 ms window falls (programmed −6 dB, detune 25 ct: reads −1.4 dB). Attack is blurred too: with sustain near 0 dB the loudest moment can fall a beat after the onset, and the attack then reads tens of ms late (g5 below: programmed 5 ms, read 35 ms at 31 ct). A goal that needs a tight attack or sustain together with a clear beat is therefore hard in this instrument. `guide` says so in general terms.

## Goals

`spikes/spike-agent/goals.toml`: five text goals of increasing difficulty. Each has `id`, `difficulty` (1 to 5), `text` (what the agent is told), `perf`, `expect` (the designer-style expectation list) and `rationale`. The judge is `spike-agent check <patch> --perf "<perf>" --expect "<expect>"`; exit 0 means the goal is met.

| Id | Difficulty | Text | Expectation list |
| --- | --- | --- | --- |
| g1-pluck | 1 | a short bright pluck | `attack<10ms, brightness>2kHz, sustain<-30dB, duration<800ms, pitch=A3±30ct` |
| g2-pad | 2 | a slow dark pad | `attack>400ms, brightness<700Hz, sustain>-4dB, release>800ms, duration>4000ms, pitch=C3±30ct` |
| g3-buzzy-bass | 3 | a buzzy bass | `pitch=C2±30ct, attack<15ms, brightness>1.2kHz, brightness<2.5kHz, sustain>-6dB, duration>1200ms, release<300ms` |
| g4-bell | 4 | a soft bell-like tone | `pitch=C5±40ct, attack>8ms, attack<40ms, brightness>800Hz, brightness<3kHz, decay>600ms, sustain<-35dB, duration>2000ms, beating>2Hz, beating<8Hz` |
| g5-supersaw-lead | 5 | a detuned supersaw-ish lead | `pitch=A4±40ct, attack<20ms, brightness>3kHz, sustain>-4dB, beating>3Hz, beating<9Hz, release<400ms, duration>2000ms` |

Difficulty comes from the number of items, items that pull against each other (g3 brackets brightness and wants a sustained body; g5 wants beating and a fast attack, and the beat delays the loudest moment, so the two items fight over where the peak is), and items that sit on a different parameter than intuition suggests (beating comes from detune, not from any filter setting).

### Reachability

Each goal was shown reachable in the toy synth with random search (500 draws, log-uniform for the log parameters) followed by hill climbing on the number of failing items, using only the public `check_patch`. `cargo test --release -p spike-agent --test goals -- --ignored` repeats it (under a minute in release); `AGENT_PROBE_WITNESS_DIR=<dir>` writes the witness patches there, outside the repository.

| Goal | All-pass draws of 500 random | Per-item pass rate among random draws | Reachable |
| --- | --- | --- | --- |
| g1-pluck | 2 | attack 25 %, brightness 22 %, sustain 46 %, duration 20 %, pitch 96 % | yes |
| g2-pad | 0 | attack 12 %, brightness 60 %, sustain 9 %, release 7 %, duration 8 %, pitch 95 % | yes |
| g3-buzzy-bass | 0 | attack 37 %, brightness (both) 28 % and 90 %, sustain 12 %, duration 80 %, release 74 % | yes |
| g4-bell | 0 | decay>600ms 1 %, beating<8Hz 27 %, sustain 46 %, others 50 to 98 % | yes |
| g5-supersaw-lead | 0 | brightness 17 %, sustain 6 %, beating 46 % and 32 %, others 56 to 98 % | yes |

So no goal is met by luck, and none is infeasible.

## Scripted end-to-end run

`tests/e2e.rs` runs this with the real binary and reads goal g1 from `goals.toml`. Values hand-picked as a sound designer would for a short bright pluck (instant attack, no sustain, a fast-closing filter envelope on a saw), in `~/code/enth-scratch/lane-p/play`:

```
$ spike-agent new --out g1.json --perf "note=A3,dur=2s,hold=1.5s"
{"next":"spike-agent step <patch> --set key=value --expect '<list>' --out <patch>","perf":{"dur":"2s","hold":"1.5s","note":"A3"},"schema":"enth.new/1","wrote":"g1.json"}                      exit 0

$ spike-agent check g1.json --expect "attack<10ms, brightness>2kHz, sustain<-30dB, duration<800ms, pitch=A3±30ct"
{"failed":3,"pass":false,"passed":2,"results":[
 {"expect":"attack<10ms","got":6.0,"pass":true,...},
 {"expect":"brightness>2kHz","got":1117.0,"pass":false,"hint":{"dir":"raise","expected":"brightness 2.158kHz","measured":true,"now":"2kHz","param":"filter.cutoff","reaches":true,"try":"6.377kHz"}},
 {"expect":"sustain<-30dB","got":-6.072,"pass":false,"hint":{"dir":"lower","expected":"sustain -32.93dB","measured":true,"now":"-6dB","param":"amp.sustain","reaches":true,"try":"-33.04dB"}},
 {"expect":"duration<800ms","got":1698.0,"pass":false,"hint":{"dir":"lower","expected":"duration 482ms","measured":true,"now":"-6dB","param":"amp.sustain","reaches":true,"try":"-42.21dB"}},
 {"expect":"pitch=A3±30ct","got":220.0,"note":"A3+0ct","pass":true,...}],"schema":"enth.check/1","source":"g1.json"}      exit 1

$ spike-agent step g1.json --set "amp.attack=2ms,amp.decay=120ms,amp.sustain=-60dB,amp.release=80ms,filter.cutoff=6kHz,filter.resonance=1.5,filter.env_amount=2oct,filter.env_decay=150ms" --expect "<same list>" --out g1.json
{"applied":[8 from/to pairs],"check":{"failed":0,"pass":true,"passed":5,"results":[
  attack 1.0 ms, brightness 3490 Hz, sustain -80 dB, duration 144 ms, pitch 220.0 Hz (A3-0ct)]},
 "describe":{...,"loudness":{"clipping":false,"lufs_i":-24.03,"peak_dbfs":-4.133},"spectrum":{"centroid_hz":3490.0,"centroid_track_hz":[3773.0,2988.0,2692.0]}},
 "patch":"g1.json","schema":"enth.step/1","wav":"g1.wav"}      exit 0
```

(Output abridged with `...`; the full objects are what the tool prints.) The track [3773, 2988, 2692] Hz is the filter envelope closing; a listener would call it a short, bright pluck.

### What a mechanical follower does

`cargo test --release -p spike-agent --test e2e -- --ignored --nocapture` runs a baseline with no judgement: start from the neutral patch, apply the first measured hint of every failing item, re-check, repeat up to 12 steps.

| Goal | Result |
| --- | --- |
| g1-pluck | passes after 1 step |
| g2-pad | passes after 2 steps |
| g3-buzzy-bass | passes after 1 step |
| g4-bell | passes after 2 steps |
| g5-supersaw-lead | does not pass in 12 steps. It ends with `attack<20ms` failing: at 31 ct detune the beat puts the loudest moment about 30 ms after the onset, so a programmed 5 ms attack reads 35 ms. The hint for that item is `amp.sustain=-12.82dB` (measured: attack 19 ms), which breaks `sustain>-4dB`, so the follower does not converge |

Read this as a property of the instrument. Where the hints are measured and the goal is a list of independent one-parameter thresholds, the loop closes without any understanding of sound, so g1 to g4 can show only that an agent *uses* the hints, not that it designs. The interesting readings in part 2 are therefore: render count, whether the agent leaves the hint path, whether it recovers on g5, and whether the result sounds like the goal (see the next section).

## Limits

- **Toy synth.** Ten parameters, one voice (two oscillators), one note, mono, 48 kHz, no modulation sources, no effects, no noise oscillator. Goals that need an LFO, a stereo image, unison of more than two voices or a noise component cannot be expressed. `modulation.*`, `stereo.*`, `envelope.shape` and `words` of agent-cli.md are not implemented.
- **Descriptors are proxies, and the checks are the goal.** An agent that satisfies every expectation has produced a sound with those measurements, which is not necessarily the sound the text names. Part 2 should have a person listen to what the agent built (Goodhart; research question Q5). A pass here means "met the designer's list", nothing more.
- **Hints encode the answer's direction.** They are measured, not guessed, and they are the product feature being tested; but on this small parameter space they are strong (see the follower baseline). Do not extrapolate success rates to a synth with hundreds of parameters.
- **One performance per patch.** `perf.note`, `perf.dur` and `perf.hold` are part of the patch, and sustain, decay and release are read around `perf.hold`. The judge fixes the performance with `--perf`.
- **Beat-induced wobble** blurs sustain by a few dB and can delay the measured attack by tens of ms at detune of 15 ct or more (see the audit and g5).
- **Beating below about 1 Hz** is reported as absent, not measured: the clip is too short for a sound beat period.
- **Determinism is for this build.** The renderer uses `libm`, so renders are bit-identical across platforms; descriptors use `realfft` and are deterministic on one machine and build, and were not compared across platforms.
- **Measurement cost.** A `check` with a hint takes about 0.1 s in release (a 2 s render is a few ms; the hint is the probe plus up to 8 refinement renders); a `step` takes well under a second.

## Deviations from agent-cli.md

| agent-cli.md | here | Why |
| --- | --- | --- |
| `enth` binary, `modules`, `schema`, `compare`, `suggest`, `match` | `spike-agent` with `guide`, `params`, `new`, `apply`, `render`, `describe`, `check`, `step` | Probe scope; the real CLI is P1. |
| Expectations `<feature> <op> <value>` with `~`, `in [a,b]`, `!= null` | Short names or paths, `< <= > >= =`, `±` tolerance, no `~`, `in`, `null` tests | Fewer forms to get wrong; the toy has no optional modulation. |
| `descriptor` list incl. shape, rolloff, flatness, tilt, harmonics, modulation, stereo, words | 11 descriptors (above) | What the toy can show and the five goals need. |
| `sustain_db` is a parameter `env.sustain` | `amp.sustain` is in dB (−60 = off) | So a unit string like `-6dB` is real and matches the descriptor. |
| Patch file `schema: 1`, modules | `schema: 1`, `engine: toy10`, flat `params`, `perf` | One engine. |

## Reproduce

```bash
unset DISPLAY WAYLAND_DISPLAY
export CARGO_BUILD_JOBS=3
flock -s /tmp/enth-quiet.lock cargo build --release -p spike-agent --locked
target/release/spike-agent guide
flock -s /tmp/enth-quiet.lock cargo test --release -p spike-agent --locked                 # unit, descriptors, hints, cli, goals, e2e
flock -s /tmp/enth-quiet.lock cargo test --release -p spike-agent --locked -- --ignored --nocapture   # reachability, audit, follower baseline
```

No audio is played anywhere; the tool writes WAV files and reads numbers.
