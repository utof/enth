# Load-bearing designs

Nine designs are expensive to change later. Get them right in Phases 0–1 and the rest of the roadmap is additive. Code here sketches shape, not final API.

## 1. One parameter schema, eight consumers

Every module parameter is declared once. That declaration drives UI knobs, host automation, patch validation, CLI parsing and display, ML encoding, challenge constraints, the randomiser and knob-distance scoring.

```rust
pub struct ParamSpec {
    pub key: &'static str,        // stable within a module type: "cutoff"
    pub name: &'static str,       // "Cutoff"
    pub unit: Unit,               // Hz | Ms | Cents | Semitones | Octaves | Db | Percent | Ratio | Degrees | None
    pub kind: Kind,               // Float{min,max,scale} | Int{min,max} | Enum(&[EnumItem]) | Toggle — min/max in plain units: 20.0..=20_000.0
    pub default: f32,             // plain units
    pub smoothing: Smoothing,     // None | Linear{ms} | OnePole{ms}
    pub flags: Flags,             // modulatable, automatable, structural, periodic
    pub learn: LearnMeta,         // tier: Basic | Intermediate | Advanced; tags; one-line plain description
    pub perceptual: Option<&'static str>, // which descriptor it mainly moves: "spectrum.centroid_hz"
}
pub enum Scale { Linear, Log, Power(f32) } // plain ⇄ normalised; Log needs min > 0; Power covers 0-based times such as attack 0–10,000 ms
```

2026-10-19 (P1a research): the first draft had `range` and `mapping` fields and a `modulatable` bool. `Stepped` and `Enum` are value kinds, not scales, so they moved into `kind`.

- **Keys are short and unit-free** (`cutoff`, `attack`, `fine`); the unit lives in the spec, so a patch stores `"cutoff": 900` meaning 900 Hz. `enth params <module>` lists every key with its unit.
- **Plain values** (Hz, ms, dB…) are what patches store, what the CLI reads and writes, and what humans and LLMs see.
- **Normalised 0–1 values** exist only at the host-automation, UI-gesture and ML boundaries.
- **Flags.** `modulatable` and `automatable` say who may drive the parameter. `structural` marks parameters that change the render plan (`voice.mode`, polyphony, an LFO's voice/global scope), so an op that sets one recompiles instead of sending a value event. `periodic` makes phase knobs wrap instead of clamp (the CLAP flag of the same name).
- **The scale is part of the persisted contract.** Route depth is a percentage of the destination's range in the normalised domain (§2), and ML encoding and the randomiser use that domain too. So changing a scale, range, unit or default needs a patch migration. An insta snapshot of the whole catalogue makes such a change show up in review. All scale maths (`to_norm`, `from_norm`) goes through the pinned `libm`, never `std`, because the engine uses it for modulation (§5).
- **Declared as `const` tables** in plain Rust, with a small `macro_rules!` that emits the index enum and the table from one list, so their order cannot drift. A `const fn` check makes a bad spec a compile error (default out of range, `min >= max`, a Log scale with `min <= 0`, duplicate keys); measured on 1.99.1, where an out-of-range default stops the build with `error[E0080]`. No proc macro, no `build.rs`; docs and JSON Schemas are generated from the tables at runtime (`enth params`, `enth schema`).
- Parsing accepts units and converts them to the spec's unit: `800Hz`, `1.2kHz`, `40ms`, `0.5s`, `12ct`, `+2st`, `+1oct`, `-6dB`, `30%`, `4:1`, `90deg`, and names for enums (`saw`, `lp`, case-insensitive). Hz parameters also take note names (`A4`, `C#3`; A4 = 440). A bare number for a percent parameter is a percent (`30`, never `0.3`). An error names the parameter, the unit it expects and the suffixes it accepts. The unit tables and parser live in `enth-params`.

## 2. Patch model, ops and versioning

A patch is plain data with stable IDs. It never contains engine state.

2026-10-19 (P1a research): Patch v1 has two forms. On disk, `PatchFile` is JSON: sparse input is accepted, output is always dense (every parameter, in spec order, so a later default change cannot alter an old file), enums are names, and the schema number lives here. In memory, `Patch` is validated, with one dense `Vec<f32>` per module in spec order.

```rust
pub struct Patch {
    pub engine: Version,                      // engine the patch was authored on (the schema number lives in PatchFile)
    pub meta: PatchMeta,                      // name, author, tags, description; not hashed
    pub voice: VoiceSettings,                 // built-in `voice`: mode (poly/mono/legato), polyphony ≤ 16, glide, glide_mode, pan; amp_env names an ADSR in `modulators`
    pub master: [f32; MasterParam::COUNT],    // built-in `master`, a fixed table: level (dB), applied after the effects
    pub generators: Vec<ModuleInstance>,      // per voice, parallel, summed in this order
    pub voice_fx: Vec<ModuleInstance>,        // per voice, serial, after the generator sum and before the amp stage
    pub fx: Vec<ModuleInstance>,              // global, serial, after the voice sum
    pub modulators: Vec<ModuleInstance>,      // evaluated in this order
    pub routes: Vec<Route>,                   // source → (module, param), depth; the order is the summation order
    pub macros: [Macro; 8],
    pub slots: [Option<ParamAddr>; 64],       // host-exposed automation slots (§4)
}
pub struct ModuleInstance { pub id: ModuleId, pub kind: ModuleKind, pub label: Option<String> /* UI only, not hashed */,
                            pub enabled: bool, pub values: Vec<f32> /* dense, in the kind's ParamSpec order; never NaN, never -0.0 */ }
pub struct Route { pub src: ModSource, pub dst: ParamAddr,
                   pub depth: f32 /* percent of the destination's range, -100..=100 */ }
```

- **Sections, not lanes.** The first draft's `lanes: Vec<Vec<ModuleInstance>>` was ambiguous: a filter must run per voice (its envelope is per note) and a reverb must be global. P1 has two explicit serial chains, `voice_fx` and `fx`. A later `lanes: Vec<Lane { poly, modules }>` is a pure migration from them, in the way Phase Plant's per-lane Poly switch works.
- **`voice` and `master` are built-in pseudo-modules**: fixed parts of every patch, with no id, that cannot be added, removed or disabled. So `voice.glide`, `voice.pan` and `master.level` are ordinary addresses that `set` and the solver can reach. `master.level` (dB) is the way back from clipping (§6).
- **Oscillator phase mode.** Each oscillator has a start-phase parameter (free, reset or seeded). At 0 ct a phase *difference* between two oscillators is a timbre change, so it must be explicit, and the descriptors' noise estimate (§6) needs a nuisance seed that is separate from the patch.
- **Files.** Parameters are parsed from the JSON value against their `ParamSpec`, so errors are typed (`out_of_range`, `wrong_unit`, `not_integral`), not serde's "did not match any variant". Enum values are written as item names (`"saw"`, `"lp"`) and toggles as `true`/`false`; a number for an enum is rejected. `serde_json` runs with `float_roundtrip`; non-finite numbers are rejected (serde turns `1e39` into `inf`, and NaN and infinity serialise as `null`); `-0.0` is canonicalised to `+0.0`.
- **Migrations** are pure `serde_json::Value → Value` steps, one per schema version, each with fixtures; no versioning crate fits (they keep one Rust struct per version, which suits neither catalogue-driven parameters nor agent-written files). A catalogue change is a migration: a renamed key, a rescaled range (which also rescales the depths of routes that target it), a split module, a changed default (the step writes the old default where a file omits the key). DSP changes bump the engine version and the goldens, not the schema.
- **The `engine` stamp** says which engine build a patch was authored on; it is a record, not an input. A patch with no stamp (`Patch::init()`, a hand-written file) is written with the current one, a patch that has one keeps it through load and save (an older file, `"engine": "0.1.0"` included, loads without a note and re-saves byte for byte), and the sound hashes and `enth diff` leave it out, so it never changes a render. Nothing is keyed on it yet; compatibility modes for old patches are later work (§5). The version lives twice because of the layers: `enth_engine::ENGINE_VERSION` (L2) is the one DSP changes bump, and `enth_patch::file::ENGINE_VERSION` (L1, which cannot depend on the engine) is what the file layer stamps. `crates/enth-engine/tests/engine_version.rs` fails when they differ, so bump both in the same commit.

**Modulation depth** is relative by nature, so it is stored as a percentage of the destination's range, measured in the destination's normalised (scale) domain:

```
n   = clamp(to_norm(base) + Σ_routes (depth / 100) · source, 0, 1)
out = from_norm(n)
```

So `+1oct` on a Log cutoff of 20 Hz–20 kHz is exactly 10.03 %. The CLI accepts musical units where they make sense (`+1oct` on a cutoff, `+12st` on pitch) and converts them at the current base value, saying so where the scale is not Log. **Polarity belongs to the source**, not the route: an LFO is bipolar or unipolar, envelopes and velocity run 0–1, and the key source is `(note − 60) / 120`. A route then carries only a depth; a curve is deferred (an additive field later). A voice-rate source routed to a global destination is a validation error (`route_rate_mismatch`).

Every mutation is an **op**:

```rust
pub enum Op {
    SetParam     { addr: ParamAddr, value: f32 },                // plain units; also voice.* and master.*
    AddModule    { at: Position, module: ModuleInstance },       // the full instance, so an inverse can re-add it
    RemoveModule { id: ModuleId },
    MoveModule   { id: ModuleId, to: Position },
    SetRoute     { src: ModSource, dst: ParamAddr, depth: f32 }, // append, or update the depth (percent)
    Unroute      { src: ModSource, dst: ParamAddr },
    InsertRoute  { index: usize, route: Route },                 // the inverse of Unroute; keeps the summation order
    SetEnabled   { id: ModuleId, enabled: bool },
    SetMacro     { index: MacroIndex, value: f32 },
    AssignSlot   { slot: SlotIndex, addr: Option<ParamAddr> },
    SetAmpEnv    { id: ModuleId },
    SetLabel     { id: ModuleId, label: Option<String> },
    SetMeta      { meta: PatchMeta },
}
impl Op { fn apply(&self, p: &mut Patch) -> Result<Vec<Op> /* the inverse, in order */, OpError>; }
fn apply_all(p: &mut Patch, ops: &[Op]) -> Result<Vec<Op>, (usize, OpError)>; // atomic: a failed transaction leaves p bit-identical
```

- Addresses are human-readable: `osc1.fine`, `filter1.cutoff`, `voice.glide`, `master.level`, `lfo1 -> filter1.cutoff`.
- **An op returns its inverse as a list.** `RemoveModule`'s inverse re-adds the module and restores its routes (at their original indices) and its slot assignments. `apply_all` replays the collected inverses on the first error, so transactions are atomic. Errors have stable snake-case codes (`unknown_param`, `out_of_range`, `duplicate_id`, `route_rate_mismatch`, `in_use_as_amp_env`, …) that the CLI maps to exit 3, each with a hint.
- **Undo merges only edits to the same address.** During a knob drag, "keep the first inverse, take the last forward" is correct for `SetParam`s on one address. Across addresses it loses undo data (the history property test caught exactly that bug in the first draft), so edits to other addresses in the same gesture are concatenated. A gesture that ends where it started is dropped.
- **Host automation applies `SetParam` without a history entry**, and so do migration on load and the solver's moves inside a search. The "every mutation is an op" rule still holds; only the undo stack skips them.
- **Property suite** (measured in prototype: 4 properties × 2,000 cases in about 0.1 s, with negative controls that fail as they should): apply then inverse is the identity; transactions are atomic; the JSON round trip is exact; undo/redo history matches a snapshot model.
- **Two hashes.** `exact_hash` covers module IDs and order and keys render caches and goldens. `canonical_hash` identifies a patch up to symmetry: it drops disabled modules and depth-0 routes, sorts only the commutative sections (generators and modulators; the serial chains keep their order), relabels IDs and ignores meta, labels and slots. It keys dedup, leaderboard entries and the `structure` score. **A canonical-equal patch is not promised to render bit-identically**: f32 addition of three or more terms is not associative, and seeds follow module IDs.

## 3. Engine and real-time safety

- **The processing quantum is a 32-frame segment.** The engine renders one segment at a time into a one-segment FIFO and the host adapter drains it, so a host may call `process` with any block size (§5). An event takes effect at the first segment boundary at or after its frame (0.67 ms at 48 kHz).
- **Two kinds of change.** Value changes are smoothed parameter events. Structural changes compile a new `RenderPlan` and swap it in at a segment boundary; the old plan goes back to be dropped. In P1 the plan is compiled on the main thread (P1 patches are small). 2026-10-19 (P1b research): measured, the G1 patch compiles in about 0.9 ms (p50, mostly the reverb's preparation), which is fine off the audio thread.
- **The plan swap carries state.** The compiled plan names its modules by ID. At the swap the audio thread carries the state of every module whose ID and kind survive into the new plan, so a held note or a reverb tail survives an unrelated edit such as adding an LFO. Only added or removed modules start fresh or fade out; both plans run side by side for 256 frames with an equal-power ramp. fundsp and Glicol keep the state of surviving nodes the same way. 2026-10-19 (P1b research): **at an instant swap (nothing added or removed) effect state is moved, not copied.** The old plan never runs again, so the new plan takes over its state; the reverb exchanges its delay-line buffers in O(1) instead of copying 1.6 MB, and renders stay bit-identical. Measured paced at 64 frames under real-time priority, the swap block was p99 797 µs (max 997 µs, 75 % of the 1,333 µs deadline) with the copy, against p99 139 µs (max 149 µs) with the move. A swap that adds or removes a module still copies, because both plans run during the crossfade. Blending the two voice sums before one effect chain would move state there too, but it changes the crossfade definition, so it is not adopted; revisit it only if measurements need it ([phase-1b.md](phase-1b.md#open-checks)).
- **One command ring and one return ring** (`rtrb`, wait-free SPSC). The command ring carries plans and epoch-tagged value events in order, because with two rings an event could arrive before the plan that defines it; events with a stale epoch are dropped. 2026-10-19 (P1b research): that covered the session's value events only. Host automation arrives through `process()` as `EventKind::Param`, which carried no epoch, so a plan swap in the middle of a block could send an id from the old plan to another parameter of the new one (a bug found in the code). P1b fixes it in two ways. Host automation becomes a **slot event** (`EventKind::Slot { index, norm }`) resolved through a slot table compiled into each plan; after a swap the engine re-applies the stored host slot values against the new table, so automation survives an edit. And parameter events carry an epoch too, with stale ones ignored. The return ring carries old plans back and has a pending slot: if it is full, the engine keeps the plan and retries next segment, so nothing is ever freed on the audio thread. No `std::sync::mpsc` and no unbounded channels in `enth-engine` (a clippy ban: they allocate a block every 31 messages); `triple_buffer` (MPL-2.0), `basedrop` and `arc-swap` were rejected.
- **Voices as arrays.** Per-voice state in structure-of-arrays form so 4–8 voices can be processed in SIMD lanes later without changing module APIs. Measured in Phase 0 ([spikes/engine.md](spikes/engine.md)): `wide` f32x4/f32x8 lanes give the scalar hash bit for bit and cut instructions 1.8–1.95× at 16 voices, but lose at 1 voice (1,993× → 713× realtime), so pick the lane width from the number of active voices.
- **Control rate by default.** Modulation runs at control rate, once per 32-frame segment, with interpolation; audio rate only where needed (FM/PM/AM between generators, not in P1). Where a stepped coefficient is audible the module interpolates per sample: the SVF interpolates its coefficients `a1..a3` linearly across the segment, which measured 13–23 dB closer to a per-sample reference than stepping them.
- **Silence skipping.** Released voices and bypassed modules cost nothing.
- **Voice manager** (deterministic: no randomness, no clock, no dependence on the host's block size). 32 physical slots, polyphony ≤ 16, so steals have spare slots. A note takes the lowest free slot. When polyphony is full it steals the oldest released voice, else the oldest held voice, with a 64-frame fade. Mono and legato use a 16-key last-note stack; glide is linear in semitones at constant time. A voice ends when its amp envelope, past the attack, is below −100 dB. Notes are addressed by CLAP's (port, channel, key, note_id), so P1b's polyphonic modulation needs no change to note addressing (it adds per-voice macro offsets, below), and the engine reports each voice end for CLAP `NOTE_END`. Smoothers reset on a retrigger, and parameter changes reach held voices.
- **Seams for the plugin and the editor (2026-10-19, P1b research; lane E+, [phase-1b.md](phase-1b.md)).** The engine gains these, all additive; every golden stays unchanged.
  - **Depth events.** A route's depth is a value, so a depth drag (the matrix, a knob's amount handle) sends a depth event instead of recompiling the plan on every frame. Going from 0 to non-zero, or back, stays structural, because the compiler leaves depth-0 routes out of the plan.
  - **A read-only telemetry tap.** A set of atomics the audio thread writes at the end of `process()`, only while an editor is open and shown: the modulated values of routed parameters for the newest held voice (for the knob rings), modulator outputs, the active-voice count, engine stats, block peaks and a scope ring, with a generation counter and the spike's sleep/wake handshake. The UI wants the latest value, not every value, so these are atomics and not a ring. The tap only reads, so renders and goldens do not change, and `enth-render` never attaches one.
  - **Per-voice macro offsets** for CLAP polyphonic modulation: added to the macro source for voice-rate destinations only, zero at voice start.
  - `reset()` (equal to a fresh engine), `position()`, an audition-note command for the editor's keyboard, the 20 ms monitor-bypass fade that [ADR-0006](adr/0006-effects-strategy.md) specifies, and macro and mod-wheel events in `Performance`, so the CLI can replay what the plugin plays.

Modules are enum-dispatched by role, not boxed trait objects. The four roles are voice processor, voice modulator, global modulator and global processor:

```rust
pub const SEG: usize = 32;                                        // frames per segment = processing quantum
pub struct PrepareCfg { pub sample_rate: f32 }                    // the only input to allocation sizes, never parameter values
pub trait VoiceProcessor {                                        // VoiceModulator, GlobalModulator and GlobalProcessor are alike
    fn start_voice(&mut self, v: VoiceIdx, seed: u64);
    fn process(&mut self, cx: &mut VoiceCx<'_>);                  // never allocates, locks or blocks
    fn copy_state_from(&mut self, old: &Self);                    // for the plan swap, no allocation
}
pub enum ModuleImpl { Osc(Osc), Noise(Noise), Svf(Svf), Adsr(Adsr), Lfo(Lfo), Reverb(Reverb), Comp(Comp) }
impl ModuleImpl { pub fn new(kind: ModuleKind, cfg: &PrepareCfg) -> Self { /* an exhaustive match */ } }
```

`prepare` is the constructor (`ModuleImpl::new`): it may allocate and is sized only from `PrepareCfg`. The exhaustive `match` on `ModuleKind` turns "every descriptor has an implementation" into a compile error instead of a test. Dispatch happens once per module per segment, so cost is not the reason for the enum; exhaustiveness is.

Every segment runs in the same fixed order, which is where the determinism lives:

1. poll the command ring (plan swaps, value events);
2. apply the note events due at this boundary, in arrival order;
3. advance the base-value smoothers (in the normalised domain; a settled smoother delivers the exact plain value);
4. global modulators, in patch order;
5. per active voice in ascending index, voice modulators in patch order (a route from a modulator later in the order reads its previous-segment value: a one-segment delay that is deterministic and makes cycles harmless);
6. modulated parameter values, `n = clamp(base_norm + Σ depth · source)` in patch route order (§2);
7. voices: generators sum into the voice bus, then `voice_fx`, then the amp stage ramped from the amp envelope's previous to current value, then `voice.pan` (a constant-power law: −3 dB in each channel at centre) into the stereo sum in ascending voice index;
8. `fx` in order (a processor whose tail is silent and whose input is zero is skipped);
9. flush state below about 1e-15, end finished voices;
10. write the FIFO.

## 4. Host parameters: macros + automation slots

The modular parameter set changes whenever the patch changes, so it is never exposed directly to the DAW. Copy Phase Plant: **8 macros + a fixed pool of 64 automation slots**, each pointing at any `ParamAddr`. The host sees a fixed list of 72 parameters. Slot names are refreshed to "Osc 1 · Cutoff" where the host allows it; otherwise they read "Slot 7". Macros also receive CLAP polyphonic modulation.

Measured in Phase 0 (2026-10-19, [spikes/shell.md](spikes/shell.md)):

- **CLAP:** a rescan (`RESCAN_INFO`) renames a slot in REAPER within 50 ms. Stock nice-plug 0.4.2 cannot do it; a 123-line add-only patch can. We carry it in a privately vendored fork (decided 2026-10-19, P1b research; [ADR-0001](adr/0001-plugin-shell.md)).
- **VST3:** hosts may reject the title-change message (`kParamTitlesChanged`). REAPER 7.82 does, but it reads VST3 parameter titles live, so with the fork's display-name override VST3 in REAPER shows the new names at once (measured by lane PL, 2026-10-20, [plugin.md](plugin.md#parameter-names); the Phase 0 spike had no override, so it showed "Slot N"). Carla's VST3 host accepts the message.
- An automation lane that already exists keeps its old label but stays bound to the parameter by ID.
- Names are derived from the patch on the main thread only, and a rescan is requested after a slot assignment and after a state load. With the fork, the session asks for a rename with a `ShellRequest::RenameParams` (only when the host would see other names or kinds, [session.md](session.md#parameter-names-and-rescans)), and the shell sets the display names and requests the rescan (`request_param_info_rescan`) on the main thread, never from `process()`: the rescan is GUI work, and GUI work scheduled from `process()` may run inline there.

2026-10-19 (P1b research): **host values and the patch.**

- **The patch wins at load.** nice-plug restores the 72 host values from the saved state, but they are never pulled into the patch during the load. The patch itself is persisted as the exact `Patch::to_json()` text, so a plugin state and a `.patch.json` are interchangeable. A blob that does not load leaves an init patch and a typed error with a hint; it never panics.
- **A host value is written back only when it truly differs.** Plain → normalised → plain is not a round trip: `from_norm(to_norm(v)) != v` for 33–82 % of values on Log, Power and signed parameters (measured, 100,001 values per parameter). So for a host value `n` and a patch value `p`, the effective value is `p` if `n` equals `to_norm(p)` bit for bit, and `from_norm(n)` otherwise. The session applies a `SetParam` (no history entry) only in the second case, initial host values are never echoed back, and the engine's slot table uses the same rule. Without it, a saved and reloaded project drifts by an ulp, renders stop matching the CLI and the hashes change.
- **One owner for the values.** The session folds host values into the patch before a save and on each editor frame. A slot reassignment sets at most one host value (many VST3 hosts crash when a plugin sets about 150 parameters at once, nih-plug #92), and patches load through state, never through host automation. Saved state is byte-deterministic (no hash-map iteration order in persisted types), which clap-validator's `state-reproducibility-binary` test requires.
- **The patch wins in the engine too (F1b review).** The engine remembers the last host value it saw for each slot, and nice-plug 0.4.2 keeps the engine across a state load (it does not re-run `initialize`), so without more the old patch's host values would keep playing over a loaded patch, or over an edited value, until the host echoed new ones. Each plan therefore carries a mask of the slots whose remembered host value it forgets when it swaps in (`Plan::forget_host_slots`): every slot after a load, and the slots a recompiling edit, undo or redo writes. Setting the 72 host parameters from the patch instead was rejected (the nih-plug #92 crash above), and so was loading presets only through host state (`set_state` is banned). The mechanism is in [session.md](session.md#the-host-bridge-the-patch-wins) and the residual case in its [F1b review decisions](session.md#f1b-review-decisions-2026-10-19).
- **A slot reassignment's host write goes out only once its plan is sent.** The slot table is compiled into each plan, so a write that reached `process()` before the new plan would go through the old table to the slot's previous target. The session pushes the write of the new target's value only if the plan that carries the new table went out in the same call; while the compile is pending the host keeps showing the old value, which is harmless because a stale host value never enters the patch (rule 2 of the bridge in [session.md](session.md#the-host-bridge-the-patch-wins)).
- An editor edit of a slotted address reaches the host as begin, set and end gestures, so automation records in write or touch mode. Silent updates (`RESCAN_VALUES`) would need one more nice-plug patch, and are deferred.

## 5. Determinism

`render()` is a pure function of (patch, performance, sample rate, seed, engine version). The host's block size is not an input (see below). That one property buys reproducible ML datasets, trustworthy scores, DSP regression tests and lesson targets that need no audio files.

- All randomness is seeded, **and never from the patch.** No system time, no global RNG; ported DSP has its upstream RNG replaced, or a constant table where upstream's seeds are constants. The engine spike's mechanism carries over: an integer PRNG (SplitMix64) with `derive(seed, stream)`; its only float step is an exact integer-to-f32 conversion. The rule (2026-10-19, P1a research):

  ```
  module_key = fnv1a64(module_id bytes)                       // stable when other modules come and go
  per voice:         seed = derive(derive(render_seed, module_key), note_index)
  per global module: seed = derive(render_seed, module_key)
  ```

  `render_seed` comes from the render spec or the challenge (the CLI has a default constant and `--seed`); `note_index` counts note-ons since engine reset. **Seeds never depend on parameter values, and the patch hash only keys caches.** The first draft seeded voices and effects from the patch hash, so any knob move reseeded every voice's random phase and detune. Measured on the spike engine (8-voice held chord, 2 s): moving the cutoff by 0.1 % changed the 10 ms RMS envelope by up to 6.6 dB with hash seeds, against 0.008 dB with a fixed render seed. Finite-difference hints, `suggest` and CMA-ES would mostly have measured that noise. A test guards it: moving any continuous parameter by 1e-3 relative changes the RMS envelope by under 0.05 dB. A patch that wants a different random character gets an explicit `seed` parameter on the module, stored in the patch.
- **The host's block size is not an input of the render.** In the engine spike, 32- and 64-sample blocks gave the same hash and 48 or 50 did not, because the 32-sample control grid and the LFO restarted at each call. So the engine keeps a free-running 32-frame grid: it renders one segment at a time into a one-segment FIFO, and the host adapter drains it (zero latency, because the event quantum equals the render unit). An event takes effect at `ceil(frame / 32) × 32` (0.67 ms at 48 kHz); same-frame events apply in arrival order; blocks are never split at event times (nih-plug's example does, which makes the grid depend on when events arrive). Measured on a prototype: 1,200 of 1,200 (event list, host block size from 1 to 4,096) pairs were bit-identical to the offline render, while a spike-style adapter (grid restarting per call) differed across block sizes in 1,150 of 1,200 pairs. Surge XT works the same way. The price is events quantised to the grid, not sample-accurate; no listening test of grid jitter exists yet. One canonical scoring rate (48 kHz).
- **Renders are bit-exact across platforms**, wasm32 engines and build flags, and independent of the host's FTZ setting. Golden hashes in CI enforce it. A feature summary with tolerances is only the diagnostic when a hash breaks. Measured in Phase 0 ([spikes/engine.md](spikes/engine.md)): 1,010 hashes (10 renders and 1,000 seed → patch) are identical on native x86-64 Linux, on Windows x86-64 in CI, and in `wasm32-wasip1` under wasmtime with and without `simd128`, across debug/release, `opt-level` s and 1, fat LTO, `target-cpu=native`, `+fma,+avx2`, scalar vs `wide` f32x4/f32x8 lanes, and FTZ off, FTZ only and FTZ+DAZ. Research 03 also ran V8 on an earlier engine. aarch64 runs only by manual dispatch: the private repo has no free arm runners.
- **The four DSP rules** that make this hold (measured traps: `std` and `libm` results differ on up to 14% of inputs, and `std` maths differs between glibc and wasm; LLVM rewrote `powf(2,x)` into `exp2`; `wide::mul_add` changes its output once the build enables FMA):
  1. Transcendentals only through the `libm` crate, pinned with `=` (bumping it is an engine-version change), or our own approximations built from `+ − × ÷ sqrt`.
  2. No `mul_add` and no `algebraic_*` operations.
  3. A fixed voice order and a fixed summation order. No runtime CPU dispatch between numerically different kernels.
  4. Denormals are flushed in code: recursive state below about 1e-15 is set to zero (at control rate, or per write in feedback paths), and **voices end below −100 dB**. Never depend on, or set, FTZ/DAZ: hosts may set it themselves, and setting MXCSR/FPCR from Rust is documented undefined behaviour. (2026-10-19, P1b research: this text used to say nice-plug sets FTZ around `process()`. It does so only behind its `unsafe_flush_denormals` feature, which enth leaves off, so it never touches our renders.) A flushed engine gives the same hash with FTZ off, FTZ only and FTZ+DAZ; a naive one does not, and over a 40 s release tail its render takes 36× longer.
  NaN is a bug.
  Rules 1 and 2 are enforced by clippy `disallowed-methods`, not by review: `std`'s `f32`/`f64` transcendentals, `mul_add`, `algebraic_*` and `wide`'s `mul_add`/`mul_sub`/`mul_neg_*` are banned outside the one module that wraps `libm` (a probe with five violations got five errors). Rule 4 has two tests: a 40 s release tail must hold zero subnormals in the output and in the engine state, and the render hash must be the same with FTZ off, FTZ only and FTZ+DAZ; the unflushed negative control fails both (203,962 subnormal samples, voices that never end).
- **Scores across platforms.** Analysis is not covered by the renders' guarantee: `rustfft` picks its code path by CPU (AVX, SSE or scalar), and the three give different bits, so native and wasm distances differ by up to 2.7e-6 relative. For identical pass/fail decisions in duels, scoring uses a **scalar FFT path** (about 5× slower than AVX), or a tolerance ≥ 1e-5 with a deterministic tie rule. Phase 0 did not measure the scalar path's cost: lane C kept `rustfft`, and `ebur128` uses `std` maths, so loudness is equal only within a tolerance too. P1a measures it before a score decides anything across machines.
- **Golden tests:** one exact SHA-256 per reference render, over the raw f32 bits (never quantised samples) with the sample rate, channel count and frame count included; the engine version in the file header; the same file on every platform; plus a file of 1,000 seed → patch hashes. Plugin-level goldens go through the test host (`cargo xtask host-render`), never a REAPER offline render: REAPER deactivates, reactivates and resets CLAP plugins about 30 ms into an offline render and drops held notes ([spikes/test-host.md](spikes/test-host.md)). 2026-10-19 (P1b research): they reuse the engine goldens' hashes. A plugin-level case is an engine golden case played through the plugin, so a DSP change never churns plugin goldens; only plugin-only behaviour (slot automation, `reset()`, the monitor bypass) gets cases of its own.
- **The plugin renders like the CLI under stated conditions** (2026-10-19, P1b research). Measured: `enth-engine` in a nice-plug CLAP shell, driven by the test host, matched `enth-render` bit for bit in 20 of 20 runs (44.1 and 48 kHz × host blocks of 1, 37, 64, 256 and 4096 × processing on the caller or on a separate thread), including a slot automation point.
  - **The conditions:** the same engine build; the fixed default render seed (`DEFAULT_SEED`), the same value the CLI gets; the same sample rate (a 44.1 kHz host against a 48 kHz CLI differs from frame 0); events at the same frames counted from engine creation or `reset()`; no plan swap, monitor bypass or audition note during the render; and only events a `Performance` can express (notes and slot automation, plus macros and the mod wheel once lane E+ adds them).
  - **The shell guarantees:** `SAMPLE_ACCURATE_AUTOMATION = true` (with `false`, block 4096 differed from the start of the block that contains the change), no echo of initial host values, the exact-plain rule of §4, a `reset()` equal to a new engine, and a patch field that round-trips exactly.
  - **Outside the contract:** host restarts in the middle of a render (REAPER restarts CLAP plugins about 30 ms into an offline render), poly-mod, and same-frame zero-length notes (the CLI orders a note-on before its note-off).
- **Fallback:** Windows CI hashes are green since 2026-10-19. Until aarch64 (and later macOS) hashes have run, those platforms keep the older contract: bit-exact on the same build and platform; across platforms, equal within a tolerance on a feature summary.
- **Lessons and the engine version** (2026-10-21, R2 research, provisional; this replaces "lessons pin an engine version"). A content pack *records* the engine it was checked on, but loads on any build that has its schemas, generator and scorers: checks lean on structure, sound checks keep a margin of at least 1e-3, and `lessons.yml` re-runs `lesson-check` on every bundled pack after any DSP change, so a change that breaks a lesson fails CI instead of shipping ([learn.md](learn.md#packs)). When DSP must change, keep a compatibility mode for old patches.
- **Inference and proposals** (2026-10-21, R3 research, provisional; the full contract is [r3.md](r3.md#the-determinism-contract)).
  - **The model's inference is bit-exact on every platform.** Our own scalar kernels in `enth-solver::propose` follow the four rules below (pinned `libm`, no `mul_add`, a fixed summation order, no runtime CPU dispatch), and each model file has exact output hashes, checked on every CI platform. Measured (R3 report 03): `tract` on native x86-64 and on wasm32 agreed on only 37–48 of 512 outputs, and `rten`'s outputs change with its thread count; our naive kernels agreed on 512 of 512 across x86-64, `target-cpu=native` and wasm32.
  - **Search is deterministic given its starts**, its settings and its render budget, for any worker count. The wall-clock cap only truncates: a result records the renders it used, so a render-budget replay reproduces it anywhere.
  - **Every "create from sound" result records its K starts as patches**, the input features' hash and the decoded audio's hash (`enth.create/1`), so a run replays from its starts even where a lossy decoder differs in the last bits across CPUs.
  - Training is not reproducible bit for bit (GPU kernels); the model file and its output golden are.
- Only effects enth owns sit in the scored path. The global FX bypass is per-instance monitoring state outside the `Patch`, a mask over the effects flagged `time_based` (the reverb in P1); `enth-render` has no way to set it, so scoring and CLI renders ignore it by construction ([ADR-0006](adr/0006-effects-strategy.md)).

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
- Envelope features are beat-aware, because detuned oscillators make raw RMS beat. This holds for the CLI's descriptors too, not only the scorer: in the agent-loop probe the first beat peak of a detuned patch set the "peak", so the measured attack came out near 1/(4 × beat rate) whatever the attack was, beat nulls read as decay, and the sustain moved by several dB with the hold time ([spikes/agent-probe.md](spikes/agent-probe.md)).
- **Beat removal is a fitted periodic pattern, not smoothing** (2026-10-19, P1a research; this replaces the "crest envelope" of the first draft). Measure the beat rate first. Then fit the log envelope's periodic part at that rate, plus a polynomial trend, over the held part, and divide the pattern out of the whole envelope, bridging deep nulls. Measured on 300 random patches against each patch's detune-0 twin: all four envelope descriptors (attack, decay, sustain, release) agree within tolerance on 64 % of patches, against 19 % for the raw RMS envelope and 23 % for a one-beat boxcar (the "low-passed over a beat" idea, which smears every transient by a whole beat period). A crest (max-hold) envelope cannot fix attack: it moves rising edges earlier by half its window. Beats slower than one period in the held part (5 ct below about 250 Hz) are flagged (`beat_corrected: false`), not corrected. A multi-harmonic beating estimator reads the rate (harmonic *h* of two detuned oscillators beats at *h* × the difference) and tells detune from a tremolo LFO. `decay_ms` becomes a threshold time in dB, so its numbers differ from the spike's.
- **Re-judged with these descriptors, the probe's 15 of 15 became 13 of 15**, and 3 of those 13 pass within noise: the old descriptor misread a 6 ms attack as 38 ms, and agents had passed goals partly *because of* the beat bug.
- **Descriptors are taken on the float render, not the 16-bit file.** Clipping corrupts brightness, not only level: a patch that peaked at +6.7 dBFS (2.2 % of samples over full scale) read a centroid of 3,712 Hz against 2,995 Hz with the master 12 dB down. So `peak_dbfs` can exceed 0, and `true_peak_dbtp` and `clipped_pct` are reported with it. A `master.level` parameter (dB, §2) is the way back, and the fix for clipping is an exact gain move, because gain is linear.
- **Every check result carries a signed `margin` and a `noise`.** `noise` is the largest change of the reading over two nuisance re-renders: a common phase shift of the oscillators, plus a different inter-oscillator phase only when they are detuned (at 0 ct that difference is a timbre change, not a nuisance). The nuisance seed is separate from the patch. A result is `thin` when |margin| ≤ 2 × noise. An old-style margin would have looked comfortable: 38 − 8 = +30 ms on an attack that really read 6 ms.
- **Metric v1** (provisional, [ADR-0007](adr/0007-metric-v1.md)): both clips scaled to −23 LUFS (an ungated whole-clip fallback for very short sounds; below −70 LUFS counts as silent), then distance A, the mean absolute difference of log-mel spectra at FFT 2048, 1024 and 512 (hop n/2, 64 bands from 30 Hz to 16 kHz), averaged over the three. Crest envelope, centroid track, MFCC, f0 in cents and the waveform mismatch are reported beside it, not added into it. One distance costs about 0.05 ms on cached features.
- **Calibration against your ears** ([spikes/metric.md](spikes/metric.md#listening-test-not-yet-run)): two sessions of 34 trials (30 pairs from near-identical to very different, plus 2 identical and 2 obviously different anchors), loudness-matched and reshuffled between sessions; Kendall τ-b and Spearman ρ with bootstrap confidence intervals; the second session is the test–retest ceiling. Until those sessions pass the G0 metric gate, keep the `sound` weight in `combined` modest and lean on `structure` at the lowest difficulties.
- Raw parameter distance punishes correct answers: solves that met the strict sound threshold still had a median parameter error of about 0.15 of the range. That is why `structure` is symmetry- and sensitivity-aware (§7).

Each score is shown 0–100. Calibrate per challenge (*2026-10-20: for sound %, the ADR-0007 note replaces "the starting patch scores 0, the pass threshold scores 100" with distance in per-challenge JND units mapped by a curve fitted to ratings*). Always report the components ("brighter than target", "attack too fast") — they double as coaching text and as the agent's `compare` output. Trust comes from: loudness normalisation (no "louder sounds closer"), shown components, versioned scorers, and periodic checks against your own blind ratings.

`Target::Expectations` is how text goals work: "a pad" becomes `envelope.attack_ms > 200, spectrum.centroid_hz < 1500, stereo.width > 0.5`. A check reports each item's margin and noise as well as pass or fail, and its hints solve all failing items at once instead of one parameter at a time, because a one-parameter hint can break an item that already passes (measured in the agent-loop probe). The hint search is `enth-solver::hint` (§7). See [agent-cli.md](agent-cli.md).

## 7. "How far were your knobs" when many answers are right

Three layers, cheapest first:

1. **Symmetry-aware parameter distance**, when the target is a patch in the same template. Canonicalise both, compare only editable and perceptually critical parameters, weight each by its audio sensitivity at that point (Δθᵀ JᵀJ Δθ, with the Jacobian J from the same finite-difference renders as layer 2).
2. **Sensitivity hints.** For each editable parameter, render a small move both ways and measure how descriptors and score change. The move that helps most becomes the hint ("lower `filter1.cutoff` toward 1.2 kHz"). The same Jacobian powers the CLI's `suggest`.
3. **Nearest solution.** Run CMA-ES starting *from the player's patch*, penalising distance from it, until it finds the closest patch that passes. Show its per-parameter deltas. Different valid approaches get different, both-correct feedback. 2026-10-21 (R2 research): R2 builds this as `enth-solver::closest`, a "How close am I?" button with progress and cancel over a lesson step's editable parameters, never live (a scoring render costs 15–30 ms, and level-2 `match` runs took a median of 423 renders); `nearest.rs` is R3's k-NN start ([learn.md](learn.md#knob-feedback)).

**The `check` hint solver** (2026-10-19, P1a research; `enth-solver::hint`, L5, pure, with the CLI supplying the evaluator). One hint covers every failing item, as a multi-parameter move:

- A bounded Gauss-Newton step on a squared-hinge objective over all failing items, with the passing items kept in the sum so a move that pushes one out is penalised. The Jacobian is the finite-difference one `check` already measures, verified by real renders (a Broyden rank-1 update after each accepted step), then sparsified: moves under 3 % of their range are dropped if the result still holds.
- When no move of the free parameters satisfies everything, it says so: a `blocked` verdict names the item and the parameter sitting at its bound (a held note lasts at least `perf.hold`, so a `duration < 500ms` goal against a 1.5 s hold is blocked).
- Measured on the toy synth: a mechanical follower that applies one such hint per step passed 5 of 5 probe goals in a single hint step each (11 to 41 renders per hint, 0.26 to 1.9 s under load). The one-parameter follower with the corrected descriptors passed 4 of 5.
- Designed, not built: hints that choose among waveforms or filter modes (categorical moves), and a probe ordered by `ParamSpec.perceptual` to cut the wide probe that makes an absent descriptor such as `beating` appear (26 to 30 of the 41 renders in the two hardest cases).

Budget check (measured in Phase 0, 2026-10-19, on a loaded laptop, so upper bounds): budget per **evaluation**, meaning render plus analysis. On the toy 10-parameter template ([spikes/metric.md](spikes/metric.md)) a 2 s render takes 2.1–2.2 ms and the features, including loudness normalisation (about 1.1 ms), 4.9–5.2 ms at hop n/2 (8.8–9.4 ms at hop n/4), so one evaluation costs 7.4–7.8 ms on one thread. Eight workers speed a search up 4.3–4.7× on this 6-core laptop, not 8×, with bit-identical results for any worker count. A search is capped at **5,000 evaluations**: with distance A and the heuristic start the median solve took 3.5 s (worst 10.9 s) on 8 workers, and two searches side by side took at worst 21.6 s. At the G0 floor of 100× realtime a 2 s render alone would take ~20 ms; the engine spike renders its 1-voice template in about 1 ms and its 8-voice P1-like patch in about 13 ms, so size the cap against the real engine once it exists. The solver stays single-threaded and pure; the caller (CLI, plugin worker) supplies the parallel evaluator, and that evaluator must be order-stable (a work queue writing results by index) so results are identical for any worker count. Hints and `suggest` use shorter clips (~0.5 s) to stay interactive, but only for spectral items (2026-10-19): sustain, release, decay and slow beats cannot be measured on 0.5 s, so a probe uses the goal's performance whenever the list has an envelope or beating item.

## 8. Solver pipeline (sound → patch)

1. **Heuristic start:** pitch → tuning; spectral centroid and tilt → filter; amplitude envelope → ADSR; noisiness → noise level. Measured on the toy synth: +4 to +6 of 20 targets over a random start, with 30–45 % of the evaluations; it guessed the waveform first on 16 of 20, but it does not recover resonance or the filter envelope's decay from the audio.
2. **Search:** CMA-ES over a template's parameters with the composite score. Our own ask–tell implementation (no crate fits: `cmaes` panics on wasm32, `basin` has no ask–tell), Hansen 2016 defaults, IPOP restarts, bounds by clamping plus a penalty.
   - **Portfolio:** a heuristic start and random restarts side by side. Research 04 measured 18/20 solved this way against 15 (heuristic alone) and 14 (random alone); lane C's port did not reproduce the gain: 17/20 side by side, 17 for the heuristic alone and 13 for random starts, with search noise of about ±2 targets over three seeds. So the portfolio is optional until a real template shows it helps; the heuristic start is not.
   - **Population:** keep λ a multiple of the worker count (16 for 8 workers) so the work queue stays full. Lane C found no solve-rate difference between λ = 10 and 16, against the research's earlier result.
   - **Restarts:** IPOP. BIPOP-style restarts added nothing within 5,000 evaluations.
   - **Categorical parameters** (the waveform) cycle through their values at restarts; CatCMAwM later.
2026-10-21 (R3 research, `_local/sessions/2026-10-21-r3-sound-to-patch/`; [r3.md](r3.md) is the design of record; provisional until you steer, and work proceeds on these defaults): sound → patch ships in two steps, R3a with search alone and R3b with a model, and the first draft's later steps ("a model maps audio to K candidate patches … load in `enth-solver` with `tract`") are replaced by steps 3–5 below. Measured first (report 04 §2, one target, scored against the clean target): **today's search copies the damage** in an upload. `enth match` on a clean copy scored 99 %, on MP3 92 %, with 20 dB of noise 10 %, with reverb 25 %, clipped 64 %, because CMA-ES turns knobs to reproduce noise, reverb and clipping; trimming the noisy tail alone raised 10 % to 56 %.

3. **R3a, create from sound with search alone.**
   - **A robust intake** in `enth-analysis` (pure, wasm32): trim, the noise floor, the bandwidth, clipped spans, and the inferred note and hold, with a warning for a phrase; short sounds are never rejected. Render and analysis run at the file's own rate, with no resampler.
   - **A degradation-aware objective under a new metric id**, so the search stops paying knobs for damage; `sound/1` is unchanged. For room reverb and compression, which a patch can also make, the candidates include a dry patch and a wet or compressed one (your call; that is the default).
   - **A portfolio of starts:** the heuristic (always kept), the templates, and k-NN retrieval over a pre-rendered patch bank (`nearest.rs`), which needs no training and is the baseline every learned model must beat.
   - **K candidate branches**, each its own `Search`, whose `ask()` batches merge into one pool batch and split back by index, so the result does not depend on the worker count; refined by deterministic successive halving (about 10 % of the evaluations to all K, the next 30 % to the best half, the rest to the best two), on a per-job pool the shell can cancel within one batch. Shown as branches, Synplant-style: the start and the refined patch both stay selectable.
4. **R3b, learned proposals.** A small CNN on `enth-analysis` log-mel features (about 1–4 M parameters) with **K = 8 winner-takes-all hypotheses** and a confidence head. Its categorical heads (waveform, filter mode, LFO shape, enable flags, routes as a fixed source × destination grid) matter most, because that is where CMA-ES fails (GR1's level 5: 0 of 4 codes). The loss is masked by `observable()` (§9), in JND units, with canonical slots. Every hypothesis is rendered, ranked and refined beside R3a's starts, and the heuristic start is always kept, so the model can only add candidates. It must beat k-NN retrieval and search alone; flow matching, discrete diffusion and render-reward fine-tuning come later, if it fails GR3 in-domain.
5. **Structure (later):** pick a template by embedding retrieval, then evolutionary topology search for hard cases, after GR3.

**Inference** runs on our own small scalar kernels in `enth-solver::propose` (L5, pure, wasm32), with weights in a versioned model file beside the plugin (`enth-model-<id>.bin`: f16, at most about 4 M parameters and 2 GFLOP, a fixed op set, the encoder run once). ONNX is the export and parity format, not a runtime. Measured (report 03): any runtime is fast enough (18–160 ms for a 0.8 M-parameter CNN against a 60 s budget), but `tract` adds about 15.5 MB to a 16 MB plugin, needs `cc`, `tract-onnx` does not build for wasm32, and it is not bit-exact across platforms (§5); our naive kernels were bit-identical everywhere for about 0.1 MB. The fallback, with your OK, is `tract-nnef` behind a feature. A missing or mismatched model file means R3a runs and says so.

**Training data** comes from our engine, but not from generator v1 as it stands (level 1 has about 12 sounds, every phrase starts on C3, values lean towards the start): a versioned sampler, `ds1`, sits beside it (v1's keyed streams and priors at `amounts` 100, a phrase sampler, disjoint `r3/train`, `r3/dev` and `r3/test` streams), with half the preset candidates as anchors and the other half held out for GR3, so no prior is fitted on a test preset. Renders are augmented in `ml/` (codecs, rooms, device EQ, noise, our compressor, band limits; LGPL and GPL tools only as subprocesses), trained with PyTorch, exported to ONNX and converted to our model file. `enth-analysis` computes the features for training and inference alike. Unlabelled real recordings are a hypothesis: their first use is search-distilled pseudo-labels, capped at about 30 %.

Budget: everything runs locally; a "create from sound" result must arrive in **under 5 minutes** on the developer's laptop, aiming for about 60 seconds, with the first candidate within 15 s. The solver reads no clock, so the cap lives in the shell's `search_with` callback. Real-time generative models (e.g. Google's Magenta line) are on the watch list as possible future modules, not dependencies.

## 9. Randomizer and difficulty

The Recreate game's targets come from a randomizer the player controls on a settings page. All features stay available at every level — difficulty changes what the *challenge* randomizes; it never locks parts of the synth.

2026-10-20 (R1 + S1 research, `_local/sessions/2026-10-20-r1-s1-recreate-and-instrument/`: reports 01 and 05 and the summary): the generator below replaces the first draft's float `RandomSpec`, and adds keyed streams, an observability mask, a minimum step decided on values, reachable quantization and versioned codes. A 250-line prototype measured the rules (report 01). Lanes CT, RG and SKR build it ([phase-2.md](phase-2.md)).

```rust
pub struct RandomSpec {
    pub generator: u16,               // frozen generator version (priors and rules); any change is a new version
    pub seed: u64,
    pub difficulty: u8,               // 0..=100 %: picks one of five levels; the auto staircase moves it
    pub amounts: [Option<u8>; 6],     // % per Category; overrides the level when set
    pub musical: u8,                  // %: 100 = priors only, 0 = uniform
    pub template: u8,                 // archetype: start patch and prior set (pluck, pad, bass, lead, keys)
}
pub enum Category { Oscillators, Filter, Envelopes, Modulation, Effects, Structure }
```

**Integers only.** A float that goes through text and back can flip a draw, so only integer fields let a code, a JSON file and the settings page give the same target. Lane CT freezes the exact fields at F1r.

| Level | What gets randomized | Observable parameters |
| --- | --- | --- |
| 1 (lowest) | Oscillator 1's waveform and octave | 1–2 |
| 2 | + filter mode, cutoff, resonance | 3–5 |
| 3 | + amp and filter envelopes, the filter envelope's depth | 6–10 |
| 4 | + 1–2 LFO routes and their rates | 9–14 |
| 5 (highest) | + osc 2, noise and effects with their settings, more routes, and structure (which modules are on) | 12–20 |

- One more category per level, Basic-tier parameters first in each (`ParamSpec.learn.tier`); `amounts` set how many parameters a category draws and how wide. The first draft's lowest level also randomized the oscillator's level, but a lone source's level is a gauge (below).
- **Calibrated by pass rates, not by distance.** The median distance from the start stops growing after the filter level (0.36, 0.89, 1.00, 0.99 and 1.06 for levels 1–5 in the prototype), so a level is defined by what it randomizes, and calibrated by the solver's evaluations to pass, the agent's solve rate and GR1's pass rates. The levels are labelled beta for GR1.
- An optional **auto difficulty** is a staircase: two passes up, one fail down (it converges on about 71 % passes; Levitt 1971).
- Level 1 is a tutorial level of about 12 distinct sounds (4 waves × 3 octaves), so a session keeps a seed history.

**Target generation works in parameter space only.** A seed must give the same target on every machine, and a render-dependent accept/reject step (is it audible, does it differ from the start) can flip on a one-bit difference. The generator also uses the pinned maths of §5: in the engine spike, `std` maths instead of `libm` turned 13 of 1,000 seeds into a different patch on glibc (and changed every render). So every rule below is a test on parameter values, never on a render. If a render-based check is ever added, the challenge code carries the accepted attempt index, so every machine regenerates the same target without repeating the check.

**Keyed streams, never a sequence** (2026-10-20). Every value is drawn from its own stream, `derive(derive(seed, fnv1a64(address)), attempt)`, with the pinned `const` `derive` and `fnv1a64` of `enth-dsp`'s `rng` and all maths through the pinned `libm`. Adding a parameter, a module or a category never moves the other draws: a higher difficulty keeps the lower categories' values, and a new catalogue entry never silently changes every shared code. It is §5's voice-seed rule applied to the generator. Measured in the prototype: oscillator values identical at two difficulties for 1,000 of 1,000 seeds; 3 × 1,000 target hashes identical on native x86-64 and `wasm32-wasip1` (`libm`'s `pow`, `log10` and `round` included); 328 µs per target, mostly parsing the starter JSON.

**Never randomize what the phrase can't reveal** (2026-10-20). `observable(&Patch, &Performance) -> Mask` is pure and works in parameter space; the same mask feeds structure % and the score rings on the editor's overlay ([ADR-0007](adr/0007-metric-v1.md)). It leaves out:

- **gauges**, which never vary alone: the only source's level, `master.level`, `comp.makeup` (levels vary only as ratios between sources, wet only against dry);
- **dead parameters**: `pw` unless the wave is pulse, `position` unless it is a table, `sync` on the first oscillator, anything in a disabled module, the depths of absent routes; keytrack without two notes an octave apart, velocity amounts without two velocities, glide without overlapping mono or legato notes, an attack longer than the hold, a release longer than the tail;
- **nuisances**: phase mode, phase, seed;
- **degenerate pairs** (keytrack and cutoff with one note): at most one of them is randomized.

Measured with metric v1: `osc1.level` 100 → 50 %, `master.level` −6 dB and a 300 ms glide on one note each give a distance of 0.0000, and keytrack at 100 % on C3 equals a cutoff move from 2 to 1 kHz (0.4487 both).

**A minimum audible step from the start, decided on values** (2026-10-20). Each parameter has a hand-written just-noticeable step (cutoff 2 semitones, times × 1.5, levels 3 dB; a wave change always counts). The target must exceed it on k parameters, with k set by the level; where it doesn't, the failing keys' `attempt` goes up by one and they redraw, so every machine recomputes the same attempt. Without it, 55 of 200 lowest-level targets were effectively the start.

**Musical / safe** means the generator only accepts targets that:

- sample parameter values from per-parameter priors, not uniform noise. 2026-10-20: the priors are hand-written and relative to the phrase's note (an LP cutoff at least 1.5 × f0; resonance and drive skewed low; semitones on intervals; envelopes as a pluck/pad mixture; wave weights favouring saw and pulse), versioned as code (`priors::v1`), and fitted to enth's own presets later as a new generator version. `musical` blends towards uniform: below 100 a value comes from the prior with probability `musical`, otherwise uniformly. The same knob is R3's dataset knob. Measured over 200 targets per level: musical targets were silent 0–1 times against 5–8 for uniform ones, and quieter than −40 LUFS 0–12 times against 43–60;
- stay audible and unclipped by construction (level and gain ranges), with scoring loudness-normalised. 11–15 % of musical targets peaked above 0 dBFS in float, so playback is loudness-matched, and the generator may set `master.level` for headroom (a gauge, so outside structure);
- differ from the starting patch by the minimum step above, and are **reachable**: quantized to values a player can hit (3 significant digits, route depths in 5 % steps; "unreachable value" is one of the three complaints on Syntorial's help page), within the challenge's constraints;
- avoid extreme ranges (ear-piercing resonance, sub-audible pitch) unless `musical` is low: the safety bounds (a resonance cap, audible pitch, cutoff relative to f0, total level) stay on above 20.

**Structure over the starter superset** (2026-10-20; owner, default yes). At the highest level R1 randomizes enable flags and routes over the starter superset patch (`xtask/src/preset/starter.patch.json`: every P1 module, with osc 2, the noise, the compressor and the reverb off, and env 2 and both LFOs unrouted). Today's editor reaches such a target by enabling modules and dragging routes, and `canonical_hash` already drops disabled modules and depth-0 routes. Adding and removing modules comes with S1 (lane AM).

**Phrases** (2026-10-20). The challenge's performance is a phrase id from a versioned table in `enth-challenge`: one held note for levels 1–3, two notes an octave apart at two velocities when keytrack or velocity is in play, a legato pair for mono or glide. The generator picks the phrase, so observability is checked before the round starts.

**Codes and generator versions** (2026-10-20).

- A challenge code is a versioned binary in Crockford base32 with a check symbol (no I, L, O or U; hyphens are ignored): about 13 characters for a preset spec, about 32 for custom settings.
- It carries the generator and engine versions and a prefix of the target's canonical hash. Decoding regenerates the target and compares the prefix: a mismatch is refused with a hint, never scored against a different target (Factorio's lesson: checksum the state, and find the first divergence). Open for CT at F1r: the bit budget, since a 32-bit prefix and an engine version do not fit in 13 characters, and whether an engine-version mismatch refuses or only warns ([phase-2.md](phase-2.md#open-checks)).
- The target patch is never persisted: a round stores the code and regenerates the target.
- Old generator versions stay frozen. Each emits a patch in its own schema version, which then goes through `enth_patch::migrate`, so catalogue changes keep old codes valid; each has a 1,000-entry spec → target-hash golden on Linux, Windows and wasm (macOS weekly). Any change to priors or rules is a new version.

The same `RandomSpec` drives solo Recreate rounds, agent evals, ML dataset sampling and duel codes.

### The round: A/B and feedback (2026-10-20)

- **Target ▶** plays a pre-rendered, loudness-matched loop of the scored phrase, which is exactly the audio the scorer hears. **Mine ▶** plays the same phrase live through the player's engine, locked to the loop's position, so an edit is heard at once (the keyboard plays Mine). A/B flips at the same position with a short crossfade, and the louder side is attenuated, never the quieter one boosted (that can clip). The level match and the scorer share one gain function.
- **No second engine.** One engine runs, plus a buffer read: two engines would double the real-time cost against about 2.5× headroom, need a second plan pipeline, and play a target that is not the scored one. No engine work is needed. Reports 01 and 02 agree on Target; 01's live Mine was chosen over 02's second pre-rendered loop, so the player hears edits at once.
- **Score on submit.** No live score by default. Submit shows sound, structure and combined % with their components (§6, [ADR-0007](adr/0007-metric-v1.md)), the time, and Good/Near/Bad rings on the randomized knobs through the editor's `Overlay`; the rings then update live while the player keeps tweaking, and the first submit stays the recorded score. Hints and Reveal mark the round as assisted. GR1 tries a live meter as an assist and decides.
- **Time** is the first passing submit, from timestamps the shell supplies (core crates read no clocks); it pauses while the editor is closed.
- **Disputes.** A three-way vote ("felt too low", "about right", "too high") and a Disagree button write the code, the player's patch and the score report to a local round log. Nothing leaves the machine unless the player sends the file.
