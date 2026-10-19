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
- **Voices as arrays.** Per-voice state in structure-of-arrays form so 4–8 voices can be processed in SIMD lanes later without changing module APIs.
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

The modular parameter set changes whenever the patch changes, so it is never exposed directly to the DAW. Copy Phase Plant: **8 macros + a fixed pool of 64 automation slots**, each pointing at any `ParamAddr`. The host sees a fixed list of 72 parameters. Where the host supports it (CLAP parameter rescan), slot names are refreshed to "Osc 1 · Cutoff"; otherwise they read "Slot 7". Macros also receive CLAP polyphonic modulation.

## 5. Determinism

`render()` is a pure function of (patch, performance, sample rate, block size, seed, engine version). That one property buys reproducible ML datasets, trustworthy scores, DSP regression tests and lesson targets that need no audio files.

- All randomness is seeded: per voice from the patch hash and note index; per effect from the patch hash and module id. No system time, no global RNG; ported DSP has its upstream RNG replaced.
- Flush denormals; fixed block size for offline renders; one canonical scoring rate (48 kHz).
- Bit-exact on the same build and platform; across platforms and wasm32, equal within a tolerance on a feature summary.
- Golden tests render every reference patch and compare a hash (same platform) and a feature summary (across platforms).
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
| `sound` | How close the audio is | Render both → loudness-normalise → features (multi-resolution log-mel, amplitude envelope, pitch, spectral centroid/tilt, modulation, stereo) → per-component distances → weighted sum |
| `structure` | How close the patch is | Symmetry-aware, sensitivity-weighted parameter distance to the target, or to the nearest passing solution (§7) — so a different valid route isn't punished |
| `combined` | One headline number | A documented weighted formula of `sound` and `structure`; default weights favour `sound` |
| `time` | Speed | Time to reach the pass threshold; used for duels and speed runs |

Each score is shown 0–100. Calibrate per challenge: the starting patch scores 0, the pass threshold scores 100. Always report the components ("brighter than target", "attack too fast") — they double as coaching text and as the agent's `compare` output. Trust comes from: loudness normalisation (no "louder sounds closer"), shown components, versioned scorers, and periodic checks against your own blind ratings.

`Target::Expectations` is how text goals work: "a pad" becomes `envelope.attack_ms > 200, spectrum.centroid_hz < 1500, stereo.width > 0.5`. See [agent-cli.md](agent-cli.md).

## 7. "How far were your knobs" when many answers are right

Three layers, cheapest first:

1. **Symmetry-aware parameter distance**, when the target is a patch in the same template. Canonicalise both, compare only editable and perceptually critical parameters, weight each by its audio sensitivity at that point.
2. **Sensitivity hints.** For each editable parameter, render a small move both ways and measure how descriptors and score change. The move that helps most becomes the hint ("lower `filter1.cutoff` toward 1.2 kHz"). The same Jacobian powers the CLI's `suggest`.
3. **Nearest solution.** Run CMA-ES starting *from the player's patch*, penalising distance from it, until it finds the closest patch that passes. Show its per-parameter deltas. Different valid approaches get different, both-correct feedback.

Budget check (confirm in Phase 0): at the G0 minimum of 100× realtime per core, one 2 s render takes ~20 ms, so a 1,000-render search takes ~20 s on one core or ~3 s on 8 cores. The solver stays single-threaded and pure; the caller (CLI, plugin worker) supplies the parallel evaluator. Hints and `suggest` use shorter clips (~0.5 s) to stay interactive.

## 8. Solver pipeline (sound → patch)

1. **Heuristic start:** pitch → tuning; spectral centroid and tilt → filter; amplitude envelope → ADSR; noisiness → noise level.
2. **Search:** CMA-ES over a template's parameters with the composite score.
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

**Musical / safe** means the generator only accepts targets that:

- sample parameter values from distributions fitted to good presets, not uniform noise;
- are audible, not clipping, and loudness-normalised;
- differ audibly from the starting patch (a minimum `sound` distance), and are reachable within the challenge's constraints;
- avoid extreme ranges (ear-piercing resonance, sub-audible pitch) unless `musical` is low.

The same `RandomSpec` drives solo Recreate rounds, agent evals, ML dataset sampling and duel codes.
