# Load-bearing designs

Nine designs are expensive to change later. Get them right in Phases 0–1 and the rest of the roadmap is additive. Code here sketches shape, not final API.

## 1. One parameter schema, eight consumers

Every module parameter is declared once. That declaration drives UI knobs, host automation, patch validation, CLI parsing and display, ML encoding, challenge constraints, the randomiser and knob-distance scoring.

```rust
pub struct ParamSpec {
    pub key: &'static str,        // stable within a module type: "cutoff"
    pub name: &'static str,       // "Cutoff"
    pub unit: Unit,               // Hz | Ms | Cents | Semitones | Octaves | Db | Percent | Ratio | None
    pub range: Range,             // plain units: 20.0..=20_000.0
    pub mapping: Mapping,         // Linear | Log | Exp(k) | Stepped(n) | Enum(&[..]) — plain ⇄ normalised
    pub default: f32,             // plain units
    pub modulatable: bool,
    pub smoothing: Smoothing,     // None | Linear(ms) | OnePole(ms)
    pub learn: LearnMeta,         // tier: Basic | Intermediate | Advanced; tags; one-line plain description
    pub perceptual: Option<&'static str>, // which descriptor it mainly moves: "spectrum.centroid_hz"
}
```

- **Keys are short and unit-free** (`cutoff`, `attack`, `detune`); the unit lives in the spec, so a patch stores `"cutoff": 900` meaning 900 Hz. `enth params <module>` lists every key with its unit.
- **Plain values** (Hz, ms, dB…) are what patches store, what the CLI reads and writes, and what humans and LLMs see.
- **Normalised 0–1 values** exist only at the host-automation, UI-gesture and ML boundaries.
- Parsing accepts units and converts them to the spec's unit: `800Hz`, `1.2kHz`, `40ms`, `0.5s`, `12ct`, `+2st`, `+1oct`, `-6dB`, `30%`, and names for enums (`saw`, `lp`).

## 2. Patch model, ops and versioning

A patch is plain data with stable IDs. It never contains engine state.

```rust
pub struct Patch {
    pub schema: u32,                          // v1 → v2 migrations are pure functions with fixtures
    pub engine: Version,                      // engine the patch was authored on
    pub meta: PatchMeta,                      // name, author, tags, description
    pub voice: VoiceSettings,                 // poly/mono/legato, voices, glide, unison
    pub generators: Vec<ModuleInstance>,
    pub modulators: Vec<ModuleInstance>,
    pub lanes: Vec<Vec<ModuleInstance>>,      // effect lanes
    pub routes: Vec<ModRoute>,                // source → (module, param), depth, curve, polarity
    pub macros: [Macro; 8],
    pub automation_slots: Vec<Option<ParamAddr>>, // host-exposed, e.g. 64
}
pub struct ModuleInstance { pub id: ModuleId, pub kind: ModuleKind, pub label: Option<String>,
                            pub params: BTreeMap<ParamKey, ParamValue>, pub enabled: bool }
pub enum ParamValue { Num(f32) /* plain units */, Choice(EnumKey) /* "saw", "lp" */, Toggle(bool) } // serde: untagged
pub struct ModRoute { pub src: ModSource, pub dst: ParamAddr,
                      pub depth: f32 /* percent of the destination's range, -100..=100 */ }
```

Modulation depth is relative by nature, so it is stored as a percentage of the destination's range. The CLI also accepts musical units where they make sense (`+1oct` on a cutoff, `+12st` on pitch) and converts them to that percentage.

Every mutation is an **op**:

```rust
pub enum Op {
    SetParam   { addr: ParamAddr, value: ParamValue },
    AddModule  { slot: Slot, kind: ModuleKind, id: ModuleId },
    RemoveModule { id: ModuleId },
    MoveModule { id: ModuleId, to: Slot },
    Route      { src: ModSource, dst: ParamAddr, depth: f32 /* percent */ },
    Unroute    { src: ModSource, dst: ParamAddr },
    SetMacro   { index: u8, value: f32 },
    AssignSlot { slot: u8, addr: Option<ParamAddr> },
    // …
}
impl Op { fn apply(&self, p: &mut Patch) -> Result<Op /* inverse */, OpError>; }
```

- Addresses are human-readable: `osc1.detune`, `filter1.cutoff`, `lfo1 -> filter1.cutoff`.
- `canonicalise()` sorts interchangeable modules and drops no-op routes, so patches that differ only by symmetry compare equal. Its `hash()` keys dedup, render caches and leaderboard entries.

## 3. Engine and real-time safety

- **Two kinds of change.** Value changes are smoothed parameter events. Structural changes compile a new `RenderPlan` off the audio thread and swap it at a block boundary with a short crossfade; the old plan goes back to be dropped.
- **Voices as arrays.** Per-voice state in structure-of-arrays form so 4–8 voices can be processed in SIMD lanes later without changing module APIs. Measured in Phase 0 ([spikes/engine.md](spikes/engine.md)): `wide` f32x4/f32x8 lanes give the scalar hash bit for bit and cut instructions 1.8–1.95× at 16 voices, but lose at 1 voice (1,993× → 713× realtime), so pick the lane width from the number of active voices.
- **Control rate by default.** Modulation runs at control rate (e.g. every 32 samples) with interpolation; audio rate only where needed (FM/PM/AM between generators).
- **Silence skipping.** Released voices and bypassed modules cost nothing.

```rust
pub trait Module: Send {
    fn prepare(&mut self, cfg: &PrepareCfg);   // sample rate, max block, max voices; may allocate
    fn reset_voice(&mut self, voice: VoiceIdx);
    fn process(&mut self, cx: &mut ProcessCx); // never allocates, locks or blocks
}
```

## 4. Host parameters: macros + automation slots

The modular parameter set changes whenever the patch changes, so it is never exposed directly to the DAW. Copy Phase Plant: **8 macros + a fixed pool of 64 automation slots**, each pointing at any `ParamAddr`. The host sees a fixed list of 72 parameters. Slot names are refreshed to "Osc 1 · Cutoff" where the host allows it; otherwise they read "Slot 7". Macros also receive CLAP polyphonic modulation.

Measured in Phase 0 (2026-10-19, [spikes/shell.md](spikes/shell.md)):

- **CLAP:** a rescan (`RESCAN_INFO`) renames a slot in REAPER within 50 ms. Stock nice-plug 0.4.2 cannot do it; a 123-line add-only patch can. We carry it in a fork when P1 needs names.
- **VST3:** hosts may reject the title-change message (`kParamTitlesChanged`). REAPER 7.82 does, so VST3 in REAPER shows "Slot N". Carla's VST3 host accepts it.
- An automation lane that already exists keeps its old label but stays bound to the parameter by ID.
- Names are derived from the patch on the main thread only, and a rescan is requested after a slot assignment and after a state load.

## 5. Determinism

`render()` is a pure function of (patch, performance, sample rate, block size, seed, engine version). That one property buys reproducible ML datasets, trustworthy scores, DSP regression tests and lesson targets that need no audio files.

- All randomness is seeded: per voice from the patch hash and note index; per effect from the patch hash and module id. No system time, no global RNG; ported DSP has its upstream RNG replaced. The engine spike's mechanism: an integer PRNG (SplitMix64) with `derive(seed, stream)`, `derive(seed, voice)` per voice and `derive(seed, module_id)` per effect; its only float step is an exact integer-to-f32 conversion.
- **Block size is part of the render function.** In the engine spike, 32- and 64-sample blocks give the same hash and 48 or 50 do not, because the 32-sample control grid and the LFO restart at each block. So block size and the control-rate grid are inputs of `render()`: a host-driven engine keeps an internal 32-sample counter and re-chunks the host's buffers onto that fixed internal block, and a note's start and end quantise to the grid (0.67 ms at 48 kHz). One canonical scoring rate (48 kHz).
- **Renders are bit-exact across platforms**, wasm32 engines and build flags, and independent of the host's FTZ setting. Golden hashes in CI enforce it. A feature summary with tolerances is only the diagnostic when a hash breaks. Measured in Phase 0 ([spikes/engine.md](spikes/engine.md)): 1,010 hashes (10 renders and 1,000 seed → patch) are identical on native x86-64 Linux, on Windows x86-64 in CI, and in `wasm32-wasip1` under wasmtime with and without `simd128`, across debug/release, `opt-level` s and 1, fat LTO, `target-cpu=native`, `+fma,+avx2`, scalar vs `wide` f32x4/f32x8 lanes, and FTZ off, FTZ only and FTZ+DAZ. Research 03 also ran V8 on an earlier engine. aarch64 runs only by manual dispatch: the private repo has no free arm runners.
- **The four DSP rules** that make this hold (measured traps: `std` and `libm` results differ on up to 14% of inputs, and `std` maths differs between glibc and wasm; LLVM rewrote `powf(2,x)` into `exp2`; `wide::mul_add` changes its output once the build enables FMA):
  1. Transcendentals only through the `libm` crate, pinned with `=` (bumping it is an engine-version change), or our own approximations built from `+ − × ÷ sqrt`.
  2. No `mul_add` and no `algebraic_*` operations.
  3. A fixed voice order and a fixed summation order. No runtime CPU dispatch between numerically different kernels.
  4. Denormals are flushed in code: recursive state below about 1e-15 is set to zero (at control rate, or per write in feedback paths), and **voices end below −100 dB**. Never depend on, or set, FTZ/DAZ: nice-plug sets FTZ around `process()` anyway, and setting MXCSR/FPCR from Rust is documented undefined behaviour. A flushed engine gives the same hash with FTZ off, FTZ only and FTZ+DAZ; a naive one does not, and over a 40 s release tail its render takes 36× longer.
  NaN is a bug.
  Rules 1 and 2 are enforced by clippy `disallowed-methods`, not by review: `std`'s `f32`/`f64` transcendentals, `mul_add`, `algebraic_*` and `wide`'s `mul_add`/`mul_sub`/`mul_neg_*` are banned outside the one module that wraps `libm` (a probe with five violations got five errors). Rule 4 has two tests: a 40 s release tail must hold zero subnormals in the output and in the engine state, and the render hash must be the same with FTZ off, FTZ only and FTZ+DAZ; the unflushed negative control fails both (203,962 subnormal samples, voices that never end).
- **Scores across platforms.** Analysis is not covered by the renders' guarantee: `rustfft` picks its code path by CPU (AVX, SSE or scalar), and the three give different bits, so native and wasm distances differ by up to 2.7e-6 relative. For identical pass/fail decisions in duels, scoring uses a **scalar FFT path** (about 5× slower than AVX), or a tolerance ≥ 1e-5 with a deterministic tie rule. Phase 0 did not measure the scalar path's cost: lane C kept `rustfft`, and `ebur128` uses `std` maths, so loudness is equal only within a tolerance too. P1a measures it before a score decides anything across machines.
- **Golden tests:** one exact SHA-256 per reference render, over the raw f32 bits (never quantised samples) with the sample rate, channel count and frame count included; the engine version in the file header; the same file on every platform; plus a file of 1,000 seed → patch hashes. Plugin-level goldens go through the test host (`cargo xtask host-render`), never a REAPER offline render: REAPER deactivates, reactivates and resets CLAP plugins about 30 ms into an offline render and drops held notes ([spikes/test-host.md](spikes/test-host.md)).
- **Fallback:** Windows CI hashes are green since 2026-10-19. Until aarch64 (and later macOS) hashes have run, those platforms keep the older contract: bit-exact on the same build and platform; across platforms, equal within a tolerance on a feature summary.
- Lessons pin an engine version. When DSP must change, keep a compatibility mode for old patches.
- Only effects enth owns sit in the scored path. The global FX bypass is per-instance monitoring state outside the `Patch`; scoring and CLI renders ignore it ([ADR-0006](adr/0006-effects-strategy.md)).

## 6. Challenge and scoring

```rust
pub struct Challenge {
    pub id: ChallengeId,
    pub version: u32,
    pub target: Target,             // Patch(PatchRef) | Audio(AudioRef) | Expectations(Vec<Expect>) | Seed(RandomSpec)
    pub answer: AnswerKind,         // Patch (rebuild the sound) | Choice(options) | Value(range) — ear-training later
    pub performance: Performance,   // several notes and velocities, so keytracking and velocity count
    pub start: Patch,               // what the player begins from
    pub constraints: Constraints,   // editable params, allowed module kinds, max modules, locked parts
    pub scoring: ScoringSpec,       // which scorers (id + version), weights, pass threshold, hint policy
    pub meta: ChallengeMeta,        // title, difficulty, tags, markdown text
}
```

- `Target::Seed` is how the Recreate game and duels work: the target is *generated* from a seed and randomizer settings (§9), so it costs nothing to store or share and is identical on every machine.
- `AnswerKind` keeps SoundGym-style games (intervals, chords, rhythm, EQ A/B) possible later without changing the Challenge shape: those answer with a choice or a value instead of a patch.

### Scoring is additive

Scores are pluggable `Scorer`s, each with a stable id and version, so adding a new score never changes the old ones:

| Scorer | Measures | Notes |
| --- | --- | --- |
| `sound` | How close the audio is | Render both → loudness-normalise (EBU R128 / BS.1770) → features (multi-resolution log-mel, amplitude envelope, pitch, spectral centroid/tilt, modulation, stereo) → per-component distances → weighted sum |
| `structure` | How close the patch is | Symmetry-aware, sensitivity-weighted parameter distance to the target, or to the nearest passing solution (§7) — so a different valid route isn't punished |
| `combined` | One headline number | A documented weighted formula of `sound` and `structure`; default weights favour `sound` |
| `time` | Speed | Time to reach the pass threshold; used for duels and speed runs |

Details measured in Phase 0 (2026-10-19):

- Loudness uses EBU R128 / BS.1770 (`ebur128`), with short-term loudness over windows under about 1 s.
- A categorical mismatch (the oscillator waveform, say) is its own component, not folded into a distance.
- Envelope features are beat-aware (a crest envelope), because detuned oscillators make raw RMS beat. This holds for the CLI's descriptors too, not only the scorer: in the agent-loop probe the first beat peak of a detuned patch set the "peak", so the measured attack came out near 1/(4 × beat rate) whatever the attack was, beat nulls read as decay, and the sustain moved by several dB with the hold time ([spikes/agent-probe.md](spikes/agent-probe.md)). Attack, decay and sustain are measured on an envelope smoothed over at least one beat period.
- **Metric v1** (provisional, [ADR-0007](adr/0007-metric-v1.md)): both clips scaled to −23 LUFS (an ungated whole-clip fallback for very short sounds; below −70 LUFS counts as silent), then distance A, the mean absolute difference of log-mel spectra at FFT 2048, 1024 and 512 (hop n/2, 64 bands from 30 Hz to 16 kHz), averaged over the three. Crest envelope, centroid track, MFCC, f0 in cents and the waveform mismatch are reported beside it, not added into it. One distance costs about 0.05 ms on cached features.
- **Calibration against your ears** ([spikes/metric.md](spikes/metric.md#listening-test-not-yet-run)): two sessions of 34 trials (30 pairs from near-identical to very different, plus 2 identical and 2 obviously different anchors), loudness-matched and reshuffled between sessions; Kendall τ-b and Spearman ρ with bootstrap confidence intervals; the second session is the test–retest ceiling. Until those sessions pass the G0 metric gate, keep the `sound` weight in `combined` modest and lean on `structure` at the lowest difficulties.
- Raw parameter distance punishes correct answers: solves that met the strict sound threshold still had a median parameter error of about 0.15 of the range. That is why `structure` is symmetry- and sensitivity-aware (§7).

Each score is shown 0–100. Calibrate per challenge: the starting patch scores 0, the pass threshold scores 100. Always report the components ("brighter than target", "attack too fast") — they double as coaching text and as the agent's `compare` output. Trust comes from: loudness normalisation (no "louder sounds closer"), shown components, versioned scorers, and periodic checks against your own blind ratings.

`Target::Expectations` is how text goals work: "a pad" becomes `envelope.attack_ms > 200, spectrum.centroid_hz < 1500, stereo.width > 0.5`. A check reports each item's margin as well as pass or fail, and its hints solve all failing items at once instead of one parameter at a time, because a one-parameter hint can break an item that already passes (measured in the agent-loop probe). See [agent-cli.md](agent-cli.md).

## 7. "How far were your knobs" when many answers are right

Three layers, cheapest first:

1. **Symmetry-aware parameter distance**, when the target is a patch in the same template. Canonicalise both, compare only editable and perceptually critical parameters, weight each by its audio sensitivity at that point (Δθᵀ JᵀJ Δθ, with the Jacobian J from the same finite-difference renders as layer 2).
2. **Sensitivity hints.** For each editable parameter, render a small move both ways and measure how descriptors and score change. The move that helps most becomes the hint ("lower `filter1.cutoff` toward 1.2 kHz"). The same Jacobian powers the CLI's `suggest`.
3. **Nearest solution.** Run CMA-ES starting *from the player's patch*, penalising distance from it, until it finds the closest patch that passes. Show its per-parameter deltas. Different valid approaches get different, both-correct feedback.

Budget check (measured in Phase 0, 2026-10-19, on a loaded laptop, so upper bounds): budget per **evaluation**, meaning render plus analysis. On the toy 10-parameter template ([spikes/metric.md](spikes/metric.md)) a 2 s render takes 2.1–2.2 ms and the features, including loudness normalisation (about 1.1 ms), 4.9–5.2 ms at hop n/2 (8.8–9.4 ms at hop n/4), so one evaluation costs 7.4–7.8 ms on one thread. Eight workers speed a search up 4.3–4.7× on this 6-core laptop, not 8×, with bit-identical results for any worker count. A search is capped at **5,000 evaluations**: with distance A and the heuristic start the median solve took 3.5 s (worst 10.9 s) on 8 workers, and two searches side by side took at worst 21.6 s. At the G0 floor of 100× realtime a 2 s render alone would take ~20 ms; the engine spike renders its 1-voice template in about 1 ms and its 8-voice P1-like patch in about 13 ms, so size the cap against the real engine once it exists. The solver stays single-threaded and pure; the caller (CLI, plugin worker) supplies the parallel evaluator, and that evaluator must be order-stable (a work queue writing results by index) so results are identical for any worker count. Hints and `suggest` use shorter clips (~0.5 s) to stay interactive.

## 8. Solver pipeline (sound → patch)

1. **Heuristic start:** pitch → tuning; spectral centroid and tilt → filter; amplitude envelope → ADSR; noisiness → noise level. Measured on the toy synth: +4 to +6 of 20 targets over a random start, with 30–45 % of the evaluations; it guessed the waveform first on 16 of 20, but it does not recover resonance or the filter envelope's decay from the audio.
2. **Search:** CMA-ES over a template's parameters with the composite score. Our own ask–tell implementation (no crate fits: `cmaes` panics on wasm32, `basin` has no ask–tell), Hansen 2016 defaults, IPOP restarts, bounds by clamping plus a penalty.
   - **Portfolio:** a heuristic start and random restarts side by side. Research 04 measured 18/20 solved this way against 15 (heuristic alone) and 14 (random alone); lane C's port did not reproduce the gain: 17/20 side by side, 17 for the heuristic alone and 13 for random starts, with search noise of about ±2 targets over three seeds. So the portfolio is optional until a real template shows it helps; the heuristic start is not.
   - **Population:** keep λ a multiple of the worker count (16 for 8 workers) so the work queue stays full. Lane C found no solve-rate difference between λ = 10 and 16, against the research's earlier result.
   - **Restarts:** IPOP. BIPOP-style restarts added nothing within 5,000 evaluations.
   - **Categorical parameters** (the waveform) cycle through their values at restarts; CatCMAwM later.
3. **Learned proposals (later):** a model maps audio to K candidate patches (generative, symmetry-aware); each is refined by step 2 and shown as a branch, Synplant-style.
4. **Structure (later):** pick a template by embedding retrieval, then evolutionary topology search for hard cases.

Training data (later): sample patches from distributions fitted to real presets, render with `enth-py`, train in `ml/`, export ONNX, load in `enth-solver` with `tract`.

Budget: everything runs locally; a "create from sound" result must arrive in **under 5 minutes** on the developer's laptop, aiming for about 60 seconds. Real-time generative models (e.g. Google's Magenta line) are on the watch list as possible future modules, not dependencies.

## 9. Randomizer and difficulty

The Recreate game's targets come from a randomizer the player controls on a settings page. All features stay available at every level — difficulty changes what the *challenge* randomizes; it never locks parts of the synth.

```rust
pub struct RandomSpec {
    pub seed: u64,
    pub difficulty: f32,                       // 0..=1, one knob for quick use
    pub amounts: BTreeMap<Category, f32>,      // 0..=1 per category; overrides the knob when set
    pub musical: f32,                          // 0 = anything goes, 1 = safe, musical results only
    pub template: Option<TemplateId>,          // structure to start from (or random structure at high difficulty)
}
pub enum Category { Oscillators, Filter, Envelopes, Modulation, Effects, Structure }
```

| Difficulty | What gets randomized |
| --- | --- |
| Lowest | Oscillator waveform, tuning, level — 1–2 knobs |
| Low | + filter type, cutoff, resonance |
| Medium | + amp and filter envelopes |
| High | + LFOs and modulation routes |
| Highest | + effects and structure (which modules exist) |

**Target generation works in parameter space only.** A seed must give the same target on every machine, and a render-dependent accept/reject step (is it audible, does it differ from the start) can flip on a one-bit difference. The generator also uses the pinned maths of §5: in the engine spike, `std` maths instead of `libm` turned 13 of 1,000 seeds into a different patch on glibc (and changed every render). So every rule below is a test on parameter values, never on a render. If a render-based check is ever added, the challenge code carries the accepted attempt index, so every machine regenerates the same target without repeating the check.

**Musical / safe** means the generator only accepts targets that:

- sample parameter values from per-parameter priors, not uniform noise: hand-written ranges, log scales and forbidden combinations at first, fitted to enth's own presets once they exist;
- stay audible and unclipped by construction (level and gain ranges), with scoring loudness-normalised;
- differ from the starting patch by a minimum step in the editable parameters, and are reachable within the challenge's constraints;
- avoid extreme ranges (ear-piercing resonance, sub-audible pitch) unless `musical` is low.

The same `RandomSpec` drives solo Recreate rounds, agent evals, ML dataset sampling and duel codes.
