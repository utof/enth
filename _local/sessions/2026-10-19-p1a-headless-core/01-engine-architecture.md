# 01 — Engine architecture

- **Date:** 2026-10-19
- **Researcher:** sub-researcher 01 (engine architecture), P1a research session
- **Questions answered:** 1 ParamSpec catalogue and units · 2 Patch v1 model · 3 ops, inverses, undo · 4 migrations · 5 `Module` trait, plan compiler, lock-free swap · 6 voice manager · 7 determinism hooks
- **Scratch and prototypes:** `~/code/enth-scratch/2026-10-19-p1a/01/` (`notes.md`, `proto-rechunk`, `proto-seed`, `proto-ops`, `proto-const`, `proto-rtrb`). All prototypes ran on rustc 1.99.1 on this machine. They live outside the repo.
- **Evidence labels:** *measured* means I ran it here today. *Documented* means it comes from source code, docs or a manual, linked. *Anecdotal* means forum posts or tutorials.
- **Privacy note:** my first batch of crates.io API calls put the owner's e-mail in the HTTP User-Agent. I stopped that and re-ran the batch without it. No other data was sent.

## Answer in brief

- **Declare parameters once, as `const` static tables in plain Rust.** A small `macro_rules!` emits the typed index enum and the table together, so their order always matches. A `const fn` checks the tables at compile time (*measured*: an out-of-range default is a compile error on 1.99.1). There is no build-time codegen and no proc macro. `enth-params` (L0) owns `ParamSpec`, units, scales, parsing and display. `enth-patch` (L1) owns the module catalogue. All plain ⇄ normalised maths goes through pinned `libm`, because the engine uses it for modulation.
- **Treat the scale (Linear, Log, Power) as part of the persisted contract, not cosmetics.** Route depth is "% of range in the normalised domain", and ML encoding and the randomiser use the same domain. So changing a scale, range, unit or default needs a schema migration. An `insta` snapshot of the whole catalogue makes every such change visible.
- **Patch v1 keeps two forms.** On disk, `PatchFile` is JSON: serde-friendly, sparse input accepted, always written dense, enums as names. In memory, `Patch` is validated, with one dense `Vec<f32>` per module in spec order. The sections are explicit (`generators`, `voice_fx`, `fx`, `modulators`) instead of design.md's ambiguous `lanes`. `voice` and `master` are built-in pseudo-modules, so `voice.glide` and `master.level` are ordinary addresses.
- **Each op's `apply` returns its inverse as `Vec<Op>`, and transactions are atomic.** `AddModule` carries the full instance. `RemoveModule`'s inverse re-adds the module and restores its routes (at their indices) and its slot assignments. *Measured* in `proto-ops`: four properties × 2,000 proptest cases pass (apply then inverse is the identity, atomic transactions, JSON round trip, history matches a snapshot model). Two negative controls fail as they should.
- **Merging during knob drags has a trap.** "First inverse, last forward" is correct only for SetParams on the *same address*. Applying it across addresses loses undo data, and the history property caught exactly that bug in my first draft. The rule: same gesture and same address → replace forward; same gesture, other address → concatenate.
- **Migrations are pure `serde_json::Value → Value` steps, one per schema version, with fixtures.** No versioning crate fits (checked six). They all keep one Rust struct per version, which suits neither catalogue-driven parameters nor agent-written files.
- **Keep two hashes.** `exact_hash` covers IDs and order and keys render caches and goldens. `canonical_hash` covers identity up to symmetry and keys dedup, leaderboards and structure scores. A canonical form cannot render bit-identically to the original: IEEE addition is not associative for 3+ terms, and seeds follow IDs.
- **Make the processing quantum the 32-frame control segment, with a one-segment FIFO host adapter.** Events take effect at the first segment boundary ≥ their frame. *Measured* in `proto-rechunk`: 1,200 of 1,200 (event list, host block size 1…4,096) pairs are bit-identical to the offline render. With zero latency, the event quantum *must* equal the on-demand render unit, so "internal block 64" should become "segment 32". Surge XT does the same with 32-sample blocks (*documented*, GPL source read for ideas only).
- **Enum dispatch for modules (`ModuleImpl`), split by role.** Roles are voice processor, voice modulator, global modulator and global processor. An exhaustive `match` on `ModuleKind` makes "every descriptor has an implementation" a compile error instead of a test. `prepare` is the constructor, sized only from `PrepareCfg`. Each segment runs in the same order: commands → note events → smoothers → global mods → voice mods → modulated params → voices → FX.
- **Swap plans over one `rtrb` command ring plus a return ring.** One ring keeps plan swaps and value events in order. The return ring has a pending slot when full, so nothing is ever freed on the audio thread. *Measured*: `assert_no_alloc` passes, and the code builds for wasm32. During a swap both plans run for 256 frames with a crossfade. Matched modules (same `ModuleId` and kind) get their state copied into the new plan, and new modules start their voices fresh. `basedrop`, `triple_buffer` (MPL-2.0) and `arc-swap` are all rejected.
- **The voice manager is deterministic.** It takes the lowest free slot of 32 physical slots, with polyphony ≤ 16. It steals the oldest released voice, else the oldest held voice, with a 64-frame fade into a spare slot. Mono and legato use a 16-key last-note stack. Glide is linear in semitones at constant time. A voice ends when its amp envelope, past attack, is below −100 dB. Notes are addressed by CLAP's (port, channel, key, note_id) with wildcards.
- **Correction to design.md §5: never seed from the patch hash.** *Measured* in `proto-seed`: a 0.1% cutoff move moved the 10 ms RMS envelope by up to **6.6 dB** with hash seeds, against **0.008 dB** with a fixed render seed. That much noise breaks `suggest`, finite-difference hints and CMA-ES. Instead, seed = `derive(derive(render_seed, fnv1a64(module_id)), note_index)`.

## Findings

### 1. ParamSpec catalogue and units

**What is shared, and by whom.** The UI, CLI, solver, randomiser, ML, challenge constraints and the engine all need the same facts per parameter: key, unit, range, scale, default, enum names, smoothing, flags and learn metadata. The engine also needs `to_norm`/`from_norm`, because modulation depth lives in the normalised domain (see §2). Those functions therefore must be **deterministic** (pinned `libm`, four DSP rules), not the `std` `powf` that nice-plug uses.

**Prior art.**

- **nice-plug 0.4.2** ([docs.rs FloatRange](https://docs.rs/nice-plug/0.4.2/nice_plug/params/range/enum.FloatRange.html), [SmoothingStyle](https://docs.rs/nice-plug/0.4.2/nice_plug/params/smoothing/enum.SmoothingStyle.html), released 2026-09-14, ISC; *documented*):
  - `FloatRange::{Linear, Skewed{factor}, SymmetricalSkewed{factor, center}, Reversed}`;
  - `SmoothingStyle::{None, Linear(ms), Logarithmic(ms), Exponential(ms), OversamplingAware}`.

  `Exponential` never reaches its target and snaps at the end. `Logarithmic` cannot cross zero. Parameters are runtime objects built by `#[derive(Params)]` on structs, with atomics and smoothers inside. That fits a plugin's host surface (our 72 host params, P1b), not a catalogue that must exist without DSP and without an instance.
- **CLAP `params.h`** ([free-audio/clap](https://github.com/free-audio/clap/blob/main/include/clap/ext/params.h), MIT, tag 1.2.10; *documented*):
  - plain `min_value`/`max_value`/`default_value` doubles;
  - flags `IS_STEPPED`, `IS_PERIODIC`, `IS_ENUM` (requires STEPPED), `IS_AUTOMATABLE`, `IS_MODULATABLE(_PER_NOTE_ID…)`, `REQUIRES_PROCESS`.

  Two of these are worth mirroring: `periodic` (phase knobs wrap instead of clamping) and the stepped/enum distinction.
- **clack** has no parameter abstraction beyond the CLAP structs (it is a binding), so it adds nothing here.
- **Surge XT** (GPL-3.0; ideas only, [Parameter.h](https://github.com/surge-synthesizer/surge/blob/main/src/common/Parameter.h), last commit 2026-10-12; *documented*): one central enum of control types (`ct_percent`, `ct_decibel`, `ct_freq_audible`, …) carries unit, range and formatting, and modules pick from it. That keeps ranges consistent across modules. The cautionary part: the enum has grown to 173 variants with special cases such as `ct_freq_audible_deactivatable_hp`. So: a small set of composable named presets (Unit × Kind × range), not a closed enum of special cases.
- **Vital** (GPL; ideas only): couldn't find a public manual page on its parameter or modulation model today, so no claim is made about it.

**Static tables vs macros vs codegen.**

| Option | For | Against | Verdict |
| --- | --- | --- | --- |
| `const`/`static` tables in Rust + `const fn` checks | Plain data that agents can read and grep; no build step; checked at compile time (*measured*, `proto-const`: float comparisons, `&str` equality and enum patterns in a `const fn` over a `static [ParamSpec; N]` give `error[E0080]: evaluation panicked: default out of range`) | Index constants must stay in sync with table order | **Use**, with a `macro_rules!` that emits both from one list |
| Derive/proc macros on structs (nih-plug style) | Typed field access | Parameters become runtime objects; a proc-macro crate; the catalogue needs an instance; poor fit for "descriptors without DSP" (architecture rule 4) | No |
| `build.rs` codegen from TOML/JSON | One data file could also generate docs and schemas | A second language for agents to edit, worse IDE support, slower builds, errors reported in generated code | No; generate docs and JSON Schema *from* the Rust tables at runtime instead (`enth params`, `enth schema`) |

**Units, parsing and display** (extends the spike's `units.rs`, which agents used without trouble; [agent-probe.md](../../../docs/spikes/agent-probe.md)):

| Unit (stored) | Accepted on input | Display |
| --- | --- | --- |
| `Hz` | `800`, `800Hz`, `1.2kHz`, note names `A4`, `C#3`, `Db2` (A4 = 440) | `999Hz`, `1.235kHz` (4 significant digits) |
| `Ms` | `40`, `40ms`, `0.5s` | `40ms`, `1.5s` at ≥ 1,000 ms |
| `Cents` / `Semitones` / `Octaves` | `ct`, `st`, `oct`, converted between each other (1 oct = 12 st = 1,200 ct); a leading `+` is allowed | in the stored unit |
| `Db` | `-6`, `-6dB` | `-6dB` |
| `Percent` | `30`, `30%` (a bare number *is* a percent; never 0–1) | `30%` |
| `Ratio` | `4`, `4:1` | `4:1` |
| `None` | a bare number | bare |
| enum | key, case-insensitive (`saw`, `lp`) | key |
| toggle | `on`/`off`, `true`/`false`, `1`/`0` | `on`/`off` |

An error names the parameter, the unit it expects and the suffixes it accepts. This follows the spike's pattern, which agents learned (*documented*, agent-probe.md).

**Scales.** `Linear`, `Log` (needs min > 0; exact "octaves = % of range" conversion) and `Power(k)`. Power covers 0-based times such as attack 0–10,000 ms, where log is impossible; it is nice-plug's `Skewed` idea, implemented with `libm::powf`. `Stepped`/`Enum`/`Toggle` are value **kinds**, not scales. design.md's `Mapping::Exp(k)` becomes `Power(k)`, and `Stepped(n)` and `Enum` move to `Kind`.

**Smoothing.** `None | Linear{ms} | OnePole{ms}`, applied by the engine to the *base* value in the normalised domain (§5), so a cutoff ramp is linear in octaves. A settled smoother delivers the exact plain value (see the invariants).

### 2. Patch v1 model

**Sections.** design.md has `lanes: Vec<Vec<ModuleInstance>>` "effect lanes", while agent-cli.md puts the filter (a per-voice module, because filter envelopes are per note) inside `lanes[0]`. That is ambiguous: the reverb in a lane must be global, the filter must be per voice. Phase Plant answers it with a per-lane **Poly** toggle that "can only be enabled on the lanes in order from left to right" ([Phase Plant manual](https://kilohearts.com/docs/phase_plant), *documented*). For P1 the simplest equivalent is two explicit serial chains:

- `voice_fx` (per voice, after the generator sum, before the VCA);
- `fx` (global, after the voice sum).

A later `lanes: Vec<Lane { poly, modules }>` is a trivial pure migration from these two lists.

**Modulation depth.** Store `depth` as % of the destination's range, in the destination's **normalised (scale) domain**:

```
n   = clamp(to_norm(base) + Σ_routes (depth/100) · src, 0, 1)
out = from_norm(n)
```

This is the usual "% of range" behaviour in modern synths. Phase Plant lets you set the result bounds instead of the amount and warns that "the total of all modulations may lead to values outside the bounds", so it clamps ([Phase Plant modulation](https://kilohearts.com/docs/modulation), *documented*). On a Log cutoff of 20 Hz–20 kHz (log2 of 1,000 ≈ 9.966 octaves), `+1oct` = 10.03 %, exactly. For non-Log scales the CLI converts musical units at the current base value and says so.

**Polarity.** Phase Plant puts the output range on the **modulator**: unipolar, bipolar or inverted ([docs](https://kilohearts.com/docs/modulation), *documented*). Minimal Audio puts a bipolar/unipolar switch and a curve on each **route** ([Rift mod matrix](https://manual.minimal.audio/rift-manual/modulation/mod-matrix), *documented*). For P1, put polarity on the source:

- LFO `polarity: bipolar | unipolar`;
- envelopes 0–1;
- velocity 0–1;
- key `(note − 60)/120` (±1 over ±10 octaves, so depth 100 % ≈ 1:1 tracking on a 20 Hz–20 kHz log cutoff).

A route then carries only `depth`, so there are fewer degrees of freedom for the solver and for canonicalisation. Route `curve` is deferred (an additive field later, no migration needed with `#[serde(default)]`).

**Per-voice vs global sources.**

- Voice-rate: envelopes, voice LFOs, velocity, key.
- Global: global LFOs, macros, modwheel.

A voice-rate source routed to a global destination (an envelope to reverb mix, say) is a validation error, `route_rate_mismatch`. Macros are global in P1a. With CLAP polyphonic modulation in P1b they become "global value + per-voice offset" (design.md §4), so the source table already reserves a per-voice slot for them.

**Sketch** (`crates/enth-patch/src/model.rs`):

```rust
pub const SCHEMA: u32 = 1;
pub const MAX_GENERATORS: usize = 4; pub const MAX_VOICE_FX: usize = 4; pub const MAX_FX: usize = 4;
pub const MAX_MODULATORS: usize = 8; pub const MAX_ROUTES: usize = 64; pub const MACROS: usize = 8; pub const SLOTS: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ModuleId(String);            // ^[a-z][a-z0-9_]{0,15}$, unique; reserved: voice, master, velocity, key, modwheel, macro1..8
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ParamAddr { pub module: ModuleId, pub param: &'static str }  // "filter1.cutoff", "voice.glide", "master.level"
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ModSource { Module(ModuleId), Velocity, Key, ModWheel, Macro(u8) }   // text: "lfo1", "velocity", "macro3"

pub struct ModuleInstance {
    pub id: ModuleId,
    pub kind: ModuleKind,               // catalogue enum
    pub label: Option<String>,          // UI only; not hashed
    pub enabled: bool,
    pub values: Vec<f32>,               // dense, kind.spec().params order; Int/Enum as exact integer f32; never -0.0, never NaN
}
pub struct ModRoute { pub src: ModSource, pub dst: ParamAddr, pub depth: f32 }   // −100..=100, ≠ 0 after canonicalise
pub struct Macro { pub value: f32 /* 0..=1 */, pub name: Option<String> }
pub struct VoiceSettings { pub values: Vec<f32> /* VOICE_PARAMS: mode, polyphony, glide, glide_mode */, pub amp_env: ModuleId }
pub struct Patch {
    pub engine: String,                 // authoring engine version, e.g. "0.1.0" (schema lives in the file layer)
    pub meta: PatchMeta,                // name, author, tags, description — not hashed
    pub voice: VoiceSettings,
    pub master: Vec<f32>,               // MASTER_PARAMS: level (dB) — agent-cli.md's clipping fix
    pub generators: Vec<ModuleInstance>,  // per voice, parallel, summed in this order
    pub voice_fx: Vec<ModuleInstance>,    // per voice, serial
    pub fx: Vec<ModuleInstance>,          // global, serial
    pub modulators: Vec<ModuleInstance>,  // evaluated in this order (§5)
    pub routes: Vec<ModRoute>,            // order is summation order
    pub macros: [Macro; MACROS],
    pub slots: [Option<ParamAddr>; SLOTS],
}
```

**File layer** (`file.rs`):

- `PatchFile` uses `#[serde(deny_unknown_fields)]`. Params are a map of `key → RawValue`, where `#[serde(untagged)] enum RawValue { Num(f32), Text(String), Bool(bool) }`.
- `Text` may be an enum key or, on CLI input, a unit string. It is resolved against the spec in a second pass.
- Reading accepts sparse params (defaults fill in) and any key order.
- Writing is always dense, in spec order, through a custom `Serialize`. Writing dense makes files immune to later default changes.
- Slots are written as a sparse object `{"7": "filter1.cutoff"}`.
- *Measured*: `serde_json` 1.0.154 round-trips f32 bit-exactly with shortest text (`[900.0,0.1,1200.0,1e-7,-0.0,20000.0,0.33333334]`). It prints `900.0`, not `900`, and keeps `-0.0`, so normalise −0.0 to +0.0 on input. JSON `16777217` reads as `16777216.0` (f32).

**Validation (`validate.rs`), invariants a valid `Patch` always holds:**

1. IDs are unique, match the grammar and are not reserved.
2. Every kind sits in a section its role allows.
3. `values.len()` matches the spec. Every value is finite and in range, Int/Enum values are integral, and none is −0.0.
4. Routes:
   - the source exists (a modulator or built-in);
   - the destination exists and its param is `modulatable`;
   - `|depth| ≤ 100`;
   - no duplicate (src, dst);
   - the rate rule holds.
5. `voice.amp_env` names an ADSR in `modulators`.
6. Limits hold (the constants above, polyphony 1–16).
7. Slots point at existing `automatable` params.

Every op keeps these, which is a property test.

**Canonicalise and hash** (`canon.rs`):

1. Drop disabled modules and every route or slot that touches them. A disabled generator or processor is the same as an absent one.
2. Drop routes with depth 0.
3. Sort the *commutative* sections only. Generators are summed, and modulators are order-free semantically. `voice_fx` and `fx` are serial chains and keep their order.
   - Sort key: kind, then the params' f32 bits, then a route signature refined by colour refinement (1-WL), because modulators can modulate modulators and the destination labels are circular.
   - Iterate until labels are stable (≤ n rounds).
   - Ties left after refinement are true symmetries at P1 sizes; the final tie-break is the original order, which is deterministic.
4. Relabel IDs as `<prefix><n>` in canonical order (prefixes from `ModuleSpec::id_prefix`). Rewrite routes and `amp_env`.
5. Sort routes by (dst, src).
6. Hash nothing that leaves the sound unchanged: meta, labels, slots and macro names. Macro *values* are hashed.
7. Canonical bytes: magic `b"enth-patch-c1"`, the schema version, then sections in fixed order. Strings are length-prefixed, f32 values are written as little-endian bits, enums as indices. SHA-256 through `sha2` (0.11.1, released 2026-10-12, MIT/Apache, pure Rust; the spike already uses it).

`exact_hash()` uses the same encoder *without* steps 1–5 (IDs and order included). **Do not claim** `render(canonicalise(p)) == render(p)` bit for bit:

- swapping two of three summed generators changes IEEE rounding (addition is commutative but not associative);
- relabelling IDs changes the seeds (§7).

Canonical equality means "same sound up to rounding and random phase". The render cache key is `(exact_hash, performance, spec, seed, engine version)`. RFC 8785 JSON canonicalisation (`serde_jcs` 0.2.0) was considered and rejected: it widens f32 to f64 text (`0.10000000149011612`), and hashing typed bytes is simpler.

### 3. Ops, inverses, undo

**Op set** (`ops.rs`):

```rust
pub enum Op {
    SetParam    { addr: ParamAddr, value: f32 },                 // plain units; also voice.* and master.*
    AddModule   { at: Slot, module: ModuleInstance },            // full instance, so it can be an inverse
    RemoveModule{ id: ModuleId },
    MoveModule  { id: ModuleId, to: Slot },
    SetRoute    { src: ModSource, dst: ParamAddr, depth: f32 },  // insert at end or update depth
    Unroute     { src: ModSource, dst: ParamAddr },
    InsertRoute { index: usize, route: ModRoute },               // inverse of Unroute; keeps summation order
    SetEnabled  { id: ModuleId, enabled: bool },
    SetMacro    { index: u8, value: f32 },
    AssignSlot  { slot: u8, addr: Option<ParamAddr> },
    SetAmpEnv   { id: ModuleId },
    SetLabel    { id: ModuleId, label: Option<String> },
    SetMeta     { meta: PatchMeta },
}
pub struct Slot { pub section: Section, pub index: usize }
impl Op { pub fn apply(&self, p: &mut Patch) -> Result<Vec<Op>, OpError>; }        // Ok(inverse ops, in order)
pub fn apply_all(p: &mut Patch, ops: &[Op]) -> Result<Vec<Op>, (usize, OpError)>;  // atomic; Ok(inverse transaction)
pub fn effect(op: &Op, cat: &Catalogue) -> Effect;   // Value(ParamAddr) | Macro(u8) | Structural | NoAudio
```

- **Inverses:**
  - `SetParam` → the old value.
  - `AddModule` → `RemoveModule`.
  - `RemoveModule` → `[AddModule{at, full instance}, InsertRoute{original index, route}…, AssignSlot{…}…]`.
  - `Unroute` → `InsertRoute{index}`.
  - `MoveModule` → a move back.
  - Removing the amp envelope is an error (`in_use_as_amp_env`, hint: set `voice.amp_env` first).
- **Atomicity:** `apply_all` replays collected inverses on the first error, so a failed transaction leaves the patch bit-identical. *Measured* as a property.
- **`effect()`** is what `enth-session` uses to choose a value event or a recompile.
  - `ParamSpec::flags.structural` marks params that change the plan: `voice.mode`, `voice.polyphony`, LFO `mode` voice/global, later unison count.
  - Route add/remove, enable and move are structural.
  - A depth change on an existing route is a value event.
- **Errors (`OpError`, stable snake-case codes for CLI JSON, exit 3, each with a hint and `did_you_mean`):** `unknown_module`, `unknown_param`, `unknown_kind`, `bad_value`, `wrong_unit{accepted}`, `out_of_range{min,max,unit}`, `not_integral`, `duplicate_id`, `reserved_id`, `wrong_section`, `bad_index`, `no_such_route`, `route_rate_mismatch`, `not_modulatable`, `too_many{what,max}`, `in_use_as_amp_env`, `non_finite`.

**Undo and redo** (`history.rs`, pure; used by `enth-session`):

- An `Entry` is `{forward: Vec<Op>, inverse: Vec<Op>, gesture: Option<GestureId>}`. Two stacks. A new record clears redo. The cap is 1,000 entries, dropping the oldest. A "saved at" index drives the dirty flag.
- **Merging** follows the shape of the `undo` crate (0.52.0, 2025-03-08, MIT/Apache; [`Edit::merge → Merged::{Yes, No(E), Annul}`](https://docs.rs/undo/0.52.0/undo/enum.Merged.html), *documented*). We don't need the crate, but these are the semantics:
  - same gesture and same address → keep the first inverse, take the last forward;
  - same gesture, other address → concatenate (forward in order, inverse reversed);
  - at `end_gesture`, if the patch equals the pre-gesture state → **Annul** (drop the entry).
  - *Measured*: the naive same-gesture merge without the address check fails the history-vs-snapshot property.
- **Not recorded:** host automation (applied as `SetParam` with no history entry, so the "every mutation is an op" rule holds), migration on load, and parameter moves made by the solver inside a search.

**Wire format for `enth apply --ops`.** One format, two strictness levels. Values may be plain numbers or unit strings; the resolved form uses plain numbers only. It extends agent-cli.md's verbs so that every inverse is expressible, which makes undo logs and future duel messages serialisable:

```json
[
  {"set": "filter1.cutoff", "to": "1.2kHz"},
  {"set": "voice.mode", "to": "legato"},
  {"add": "svf", "id": "filter2", "at": "voice_fx[1]", "params": {"cutoff": "800Hz"}},
  {"remove": "filter2"},
  {"move": "filter2", "to": "voice_fx[0]"},
  {"route": "lfo1 -> filter1.cutoff", "depth": "+1oct"},
  {"route": "env2 -> filter1.cutoff", "depth": 30, "index": 2},
  {"unroute": "lfo1 -> filter1.cutoff"},
  {"enable": "fx1", "to": false},
  {"macro": 1, "to": 0.5},
  {"slot": 7, "to": "filter1.cutoff"},
  {"amp_env": "env1"}
]
```

- `add` without `id` takes the lowest free `<prefix><n>`.
- `at` defaults to the end of the kind's section.
- Macros and slots are 1-based in text and 0-based in `Op`.
- Parsing tries the verb keys in a fixed order and returns `unknown_op` listing the verbs. Deserialise into a `serde_json::Map` first, not `#[serde(untagged)]`, so errors stay specific.
- The CLI JSON Schema and `schemars` choices belong to report 03.

**Property tests** (all measured in prototype form in `proto-ops`, ~0.1 s for 4 × 2,000 cases in release):

1. For a random valid `p` and a random (possibly invalid) op: if `apply` succeeds, applying its inverse gives `p` back exactly; if it fails, `p` is unchanged.
2. Random transactions are atomic, invert to `p`, and redo reproduces the result.
3. `from_file(to_file(p)) == p`, plus byte-identical re-serialisation.
4. History with merges, undo and redo matches a model that stores snapshots.
5. To add in the product: `validate(apply(op, p))` is `Ok` whenever `apply` succeeded.

Negative controls, *measured*: removing route restoration from `RemoveModule`'s inverse fails properties 1, 2 and 4.

### 4. Migrations

**Prior art (crates.io API and GitHub, 2026-10-19; all *documented*):**

| Crate | Latest | Licence | Approach | Fit |
| --- | --- | --- | --- | --- |
| `obake` | 1.0.5, 2023-04-20 | MIT/Apache | proc macro, one struct per version | Old; typed structs per version |
| `serde-version` | 0.5.1, 2019-11-26 | MIT/Apache | — | Dead |
| `version-migrate` | 0.21.0, 2026-07-17 | MIT/Apache (crates.io) | typed versions plus storage/ACID features | 1 GitHub star, heavy scope |
| `magic_migrate` | 2.0.0, 2025-08-18 | crates.io says Apache-2.0, GitHub detects MIT | `TryFrom` chain of structs | Typed; licence metadata inconsistent |
| `versionize` | 0.2.0, 2024-01-02 | Apache-2.0 | binary (Firecracker) | Binary |
| `revision` | 0.30.0, 2026-07-02 | Apache-2.0 | binary (SurrealDB) | Binary |

They all solve "struct changed shape". Our migrations are mostly about *catalogue* changes inside a stable shape: a renamed key, a rescaled range, a split module, a new default. Those are data rewrites. A typed struct per version would freeze the whole catalogue per version.

**Design** (`migrate.rs`, `migrate/v1_to_v2.rs`, …):

```rust
pub type Step = fn(&mut serde_json::Value, &mut Vec<MigrationNote>) -> Result<(), MigrateError>;
pub static STEPS: &[Step] = &[/* STEPS[i] migrates schema i+1 → i+2 */];
pub fn migrate(v: Value) -> Result<(Value, Vec<MigrationNote>), MigrateError>;  // TooNew{found, supported}, Missing("schema"), Step{from, msg}
pub fn load(json: &str) -> Result<(Patch, Vec<MigrationNote>), LoadError>;     // parse → migrate → PatchFile (deny_unknown_fields) → resolve → validate
```

- Steps are pure and forward-only, and each sets `schema`. Notes are user-facing (`"osc1.detune clamped to 100ct"`); the CLI prints them.
- **The rules a catalogue change triggers:**
  - adding a param → its default must reproduce the old sound, otherwise a step writes the old-behaviour value;
  - renaming a key → a step;
  - a unit, range or scale change → a step rescales values *and the depths of routes targeting it*;
  - removal → a step drops it, with a note;
  - a default change → a step materialises the old default where a file omits the key.
- DSP changes bump `ENGINE_VERSION` (goldens), not `SCHEMA`. Compatibility modes keyed on `engine` are later work.
- **Tests:**
  - `tests/fixtures/patch/v{N}/*.patch.json` → `migrate` → equals `tests/fixtures/patch/expected/*.patch.json` (compared as serialised `PatchFile`);
  - every version 1…SCHEMA has at least one fixture;
  - `migrate(current)` is the identity;
  - a future version gives `TooNew`;
  - the catalogue snapshot (`insta`, 1.49.0, Apache-2.0) fails on any spec change, so the author must either bump the schema with a step or record in the snapshot why none is needed.

### 5. `Module` trait, plan compiler, swap

**Processing quantum and re-chunking.** The spike found that block size is part of the render function: 32 = 64, while 48 and 50 differ ([engine.md](../../../docs/spikes/engine.md), *measured* there). design.md §5 (updated today in commit 8986e83) says hosts re-chunk onto a fixed internal block. With zero latency, the **event quantum must equal the on-demand render unit**.

Suppose the engine renders a 64-frame block when the host position reaches 64k. An event at frame 70, due at the next 32-boundary (96), is unknown at that moment if the host block is shorter than 7 frames. It would land at 128 instead, so the output would depend on host block size.

So:

- the engine's quantum is the **32-frame control segment**, rendered on demand into a one-segment FIFO;
- an event takes effect at `ceil(frame / 32) × 32`;
- same-frame events apply in arrival order.

*Measured* (`proto-rechunk`, toy engine with control-rate LFO, one-pole and amp ramp): 50 random event lists × 24 host block sizes (1, 2, 3, 5, 7, 13, 31, 32, 33, 48, 63, 64, 65, 100, 127, 128, 256, 441, 480, 511, 512, 1000, 1024, 4096) gave **1,200 of 1,200 bit-identical** to the offline render. The spike-style adapter (render each host chunk, grid restarting per call) differs from itself across block sizes in 1,150 of 1,200 pairs; the 50 equal ones are block 32 itself.

Surge XT works the same way (*documented* from [SurgeSynthProcessor.cpp](https://github.com/surge-synthesizer/surge/blob/main/src/surge-xt/SurgeSynthProcessor.cpp), GPL-3.0, read for ideas only). Its `blockPos` persists across host calls, it calls `surge->process()` (one 32-sample block) when `blockPos == 0`, it applies MIDI before that render, and it outputs from the buffer, all with zero latency. A KVR thread confirms Surge's modulation runs every 32 samples ([KVR](https://www.kvraudio.com/forum/viewtopic.php?p=8808651), *anecdotal*).

The alternative is one block of reported latency, which buys sample-accurate events independent of host block size. It costs 1.33 ms at 48 kHz on live playing and an ADR (AGENTS.md: zero latency unless an ADR says otherwise). Not recommended for P1. nih-plug's ISC `poly_mod_synth` example splits blocks at event times (≤ 64), which is sample-accurate but host-block-dependent ([source](https://github.com/robbert-vdh/nih-plug/blob/master/plugins/examples/poly_mod_synth/src/lib.rs), *documented*).

**Roles and dispatch** (`module.rs`):

```rust
pub const SEG: usize = 32;            // frames per control segment = processing quantum
pub const MAX_VOICES: usize = 32;     // physical slots (polyphony ≤ 16, the rest are steal spares)
pub type VoiceIdx = u8;
pub struct PrepareCfg { pub sample_rate: f32 }    // the ONLY input to allocation sizes (never param values)

pub struct Voices<'a> { pub active: &'a [VoiceIdx] /* ascending: fixed summation order */, pub starting: &'a [VoiceIdx] }
pub struct Params<'a> { /* this module's slice of the plan's ParamTable */ }
impl Params<'_> {
    pub fn cur(&self, p: impl Into<usize>, v: VoiceIdx) -> f32;   // modulated plain value for this segment
    pub fn prev(&self, p: impl Into<usize>, v: VoiceIdx) -> f32;  // last segment's, for per-sample ramps
    pub fn global(&self, p: impl Into<usize>) -> f32;
}
pub struct VoiceCx<'a>  { pub voices: Voices<'a>, pub params: Params<'a>, pub notes: &'a NoteState, pub bus: &'a mut VoiceBus }
pub struct ModCx<'a>    { pub voices: Voices<'a>, pub params: Params<'a>, pub notes: &'a NoteState, pub out: &'a mut [f32; MAX_VOICES] }
pub struct GlobalCx<'a> { pub params: Params<'a>, pub bus: &'a mut StereoBus }
// NoteState per voice: pitch_st (prev, cur; glided), velocity, gate, released_now, note_index.

pub trait VoiceProcessor { fn start_voice(&mut self, v: VoiceIdx, seed: u64); fn process(&mut self, cx: &mut VoiceCx<'_>); fn copy_state_from(&mut self, o: &Self); }
pub trait VoiceModulator { fn start_voice(&mut self, v: VoiceIdx, seed: u64); fn tick(&mut self, cx: &mut ModCx<'_>); fn copy_state_from(&mut self, o: &Self); }
pub trait GlobalModulator { fn reset(&mut self, seed: u64); fn tick(&mut self, cx: &mut GlobalCx<'_>) -> f32; fn copy_state_from(&mut self, o: &Self); }
pub trait GlobalProcessor { fn reset(&mut self, seed: u64); fn process(&mut self, cx: &mut GlobalCx<'_>); fn copy_state_from(&mut self, o: &Self); fn tail_silent(&self) -> bool; }

pub enum ModuleImpl { Analog(Analog), Wavetable(Wavetable), Noise(Noise), Svf(Svf), Adsr(Adsr), Lfo(Lfo), Reverb(Reverb), Comp(Comp) }
impl ModuleImpl {
    pub fn new(kind: ModuleKind, cfg: &PrepareCfg) -> Self { match kind { /* exhaustive: a new ModuleKind without an impl does not compile */ } }
    pub fn copy_state_from(&mut self, other: &Self) -> bool { /* same variant → copy, no allocation; else false */ }
}
```

- design.md's `prepare` becomes the constructor (it may allocate). `reset_voice` becomes `start_voice(v, seed)`. `process` is per role.
- Envelopes see gate and release through `NoteState`, not through callbacks.
- One struct may implement two roles (LFO voice/global). The compiler picks the role from the structural `mode` param.
- Per-voice state is fixed arrays `[f32; MAX_VOICES]` (structure of arrays), so state copies are `memcpy`. Delay lines are `Box<[f32]>` sized in `new`, copied with `copy_from_slice`.
- Enum dispatch also removes per-module `Box`es and vtables. Dispatch happens once per module per segment, so cost is not the reason; exhaustiveness is.

**Voice bus.** The P1 voice bus is **mono**, with a per-voice constant-power pan into the stereo sum. Two reasons:

- the spike's P1-like voice costs 146.3 instructions per sample and voice, mono (*documented*, engine.md), against the proposed G1 budget of ≈ 220 (roadmap.md);
- a stereo voice path doubles filter cost.

Unison spread needs a stereo voice bus. That is an open decision for the summary, together with report 02's budget.

**The order of one segment** (`engine.rs`). Fixed; this is where the determinism lives.

1. Poll the command ring: plan swap, param and macro events with the plan epoch.
2. Apply note events due at this boundary, in arrival order (voice manager).
3. Advance base-value smoothers (normalised domain; libm coefficients). A smoother that reaches its target snaps to the exact plain value.
4. Global modulators, in patch order.
5. Per active voice, in ascending index: voice modulators in patch order. A route from a modulator *later* in the order reads its previous-segment value. That one-segment delay (0.67 ms) is deterministic and makes cycles harmless without cycle detection. Moving a modulator is structural, so its order is fixed per plan.
6. Modulated parameter values: for each destination with routes, `n = clamp(base_norm + Σ depth·src)` in patch route order, then `from_norm`. An unmodulated, settled param gives `base_plain` exactly.
7. Voices: generators in patch order sum into the voice bus → `voice_fx` chain → VCA ramping from the amp envelope's prev to cur value → pan → sum into the stereo bus in ascending voice index.
8. `fx` chain in order. A processor with `tail_silent()` (all state flushed to exact zero) and a zero input is skipped.
9. Flush state below 1e-15 (each module, per segment). End voices whose amp envelope is past attack and below 1e-5 (−100 dB), and emit `VoiceEnded`.
10. Write the FIFO.

Audio-rate routes (FM/PM between generators) are not in P1. The bus layout leaves room: a generator can read an earlier generator's per-voice buffer.

**Plan compiler** (`plan.rs`). A pure function, `compile(&Patch, &PrepareCfg, render_seed) -> Result<(RenderPlan, PlanMap), CompileError>`. P1's topology is fixed (four sections), so the "graph" is just ordered index lists: no general DAG scheduling.

```rust
pub struct RenderPlan {
    pub epoch: u32,
    modules: Vec<ModuleImpl>,                 // all instances, prepared
    keys: Vec<(u64 /* fnv1a64(id) */, ModuleKind)>,   // for state transfer and seeds
    order: Order { global_mods: Vec<u16>, voice_mods: Vec<u16>, generators: Vec<u16>, voice_fx: Vec<u16>, fx: Vec<u16> },
    params: ParamTable,                       // flat: spec, base_plain, base_norm, Smoother, has_routes, per-voice cur/prev
    routes: Vec<CompiledRoute>,               // (src: SrcRef, dst: u32, depth: f32 /* fraction */), sorted by dst, then patch order
    mods: ModValues,                          // [n_sources][MAX_VOICES] cur and prev
    buses: Buses,                             // voice [MAX_VOICES][SEG], stereo [2][SEG], crossfade scratch
}
pub struct PlanMap { pub params: BTreeMap<ParamAddr, u32> }   // stays on the main thread: Op → value event
```

**Lock-free swap** (`link.rs`):

- **One command ring**, `rtrb::RingBuffer<Cmd>`, with `enum Cmd { Plan(Box<RenderPlan>), Param{epoch, param: u32, plain: f32}, Macro{index, value} }`. A single ring is needed because two rings (plans and events) can let the consumer see an event with the new epoch before the plan that defines it. Value events with a stale epoch are dropped. That is safe because the plan was compiled from a patch that already contains the value, and events sent after the plan carry the new epoch. Notes come with `process()` from the host, not through the ring.
- **A return ring**, `rtrb::RingBuffer<Box<RenderPlan>>`, capacity 4, plus `pending_return: Option<Box<RenderPlan>>`. If the push fails, keep the plan, stop popping commands (back-pressure) and retry next segment.
- The session never has more than two plans in flight. Returned plans double as acks, and edits that arrive meanwhile are coalesced. fundsp does the same: the backend keeps only the latest pending version and sends superseded ones straight back ([realnet.rs](https://github.com/SamiPerttu/fundsp/blob/master/src/realnet.rs), *documented*).
- Firewheel's processor *ignores* a failed return push (`let _ = self.to_graph_tx.try_push(…DropSchedule…)` in [handle_messages.rs](https://github.com/BillyDM/firewheel/blob/main/crates/firewheel-graph/src/processor/handle_messages.rs), 0.14.0; *documented*). If the ring were ever full, the dropped `Box` would be freed on the audio thread. That is the bug the pending slot prevents.
- *Measured* (`proto-rtrb`): `rtrb =0.4.0` with `default-features = false` builds for `wasm32-unknown-unknown`. Swap, return and a forced ring-full retry over 20 plans pass `assert_no_alloc` on the audio side.

**Crossfade.** fundsp's `Slot` runs the old and new units side by side and mixes them with `smooth5` or `sine_ease` over the fade time. Its `Net` backend *moves* unchanged nodes, matched by ID, into the new network, so their state survives ([slot.rs](https://github.com/SamiPerttu/fundsp/blob/master/src/slot.rs), [net.rs `migrate`](https://github.com/SamiPerttu/fundsp/blob/master/src/net.rs); fundsp 0.23.0, MIT/Apache; *documented*). Firewheel keeps nodes in a persistent arena and sends only the new nodes and the IDs to remove (*documented*, same file).

For enth:

- At the swap, for each new-plan module whose (key, kind) matches an old one, call `copy_state_from`. Copying (not moving) lets the old plan keep running.
- Call `start_voice(v, seed)` on unmatched modules for every active voice.
- Run both plans for **256 frames** (8 segments, 5.3 ms) with an equal-power ramp computed once per plan through `libm`.
- Send the old plan back.

The voice manager lives in the engine, not in the plan, so both plans see the same notes. During those 8 segments CPU doubles. That fits under the G1 rule (p99.9 below 25 % of the deadline) only if the steady state stays under about 12 %, so P1b should measure it. Swaps never happen in an offline render, so goldens are unaffected.

**Rejected crates** (2026-10-19; crates.io API plus licence files read):

- **`triple_buffer` 9.0.0** is MPL-2.0, which `deny.toml` does not allow. Its LICENSE file reads "Mozilla Public License Version 2.0".
- **`basedrop` 0.1.3** (2025-10-29, MIT/Apache) has two releases in four years. A 2021 commit fixed use-after-frees and a 2025-10-28 commit fixed memory orderings ([commits](https://github.com/micahrj/basedrop/commits/master)). A return ring does the same job.
- **`arc-swap` 1.9.3** (2026-10-18) has lock-free reads that are mostly wait-free, but readers decrement the old value's count, so the drop can land on the audio thread. Its docs say writer contention "might be problematic for some hard-realtime cases" ([performance docs](https://docs.rs/arc-swap/latest/arc_swap/docs/performance/index.html), *documented*).
- **`rtrb` 0.4.0** (2026-08-17, MIT/Apache; LICENSE-MIT and LICENSE-APACHE read) is the choice: wait-free SPSC and `no_std` + alloc. 0.4.0 fixed a panic-safety double free in `ReadChunk::commit` ([release](https://github.com/mgeier/rtrb/releases/tag/0.4.0)). Pin it with `=`.

### 6. Voice manager

**Prior art:**

- **Phase Plant** (*documented*, [manual](https://kilohearts.com/docs/phase_plant)):
  - steals "the oldest and quietest voice";
  - polyphony 1 means mono;
  - retrig mode retriggers every note; legato does not retrigger when keys overlap;
  - glide mode is "always" or legato-only;
  - modulator triggers are Never, Always, Legato or Auto ([modulation](https://kilohearts.com/docs/modulation)).
- **nih-plug's ISC `poly_mod_synth`** (*documented*) takes the first free slot and otherwise hard-steals the voice with the smallest monotonic id, sending `VoiceTerminated`.
- **CLAP `events.h`** (MIT, tag 1.2.10, *documented*): notes are matched by (port, channel, key, note_id) with −1 wildcards; `NOTE_END` is sent by the plugin when a voice finishes; `NOTE_CHOKE` exists.

**Design** (`voice.rs`):

```rust
pub enum VoiceMode { Poly, Mono, Legato }           // voice.mode (enum, structural)
pub enum GlideMode { Always, Legato }               // voice.glide_mode; voice.glide in ms (0 = off)
pub struct NoteKey { pub port: i16, pub channel: i16, pub key: i16, pub note_id: i32 }   // −1 = wildcard in queries only
pub enum VoiceState { Free, Held, Released, Stolen { fade_left: u8 } }
pub struct Voice { pub state: VoiceState, pub key: NoteKey, pub note_index: u32, pub age: u64 /* start segment */,
                   pub velocity: f32, pub pitch_prev: f32, pub pitch_cur: f32, pub glide_step: f32, pub glide_left: u16 }
pub struct VoiceManager { slots: [Voice; MAX_VOICES], stack: NoteStack /* [NoteKey; 16], last-note priority */,
                          note_counter: u32, mode: VoiceMode, polyphony: u8, glide_segments: u16, glide_mode: GlideMode }
pub enum VoiceOut { Ended { key: NoteKey, frame: u64 } }   // into a fixed-capacity output queue → CLAP NOTE_END in P1b
```

- **Poly note-on:**
  - If fewer than `polyphony` voices are Held or Released, take the **lowest-index Free slot**.
  - Otherwise steal. The victim is the Released voice with the smallest `age`, else the Held voice with the smallest `age`; ties go to the lowest index. The victim becomes `Stolen{2 segments}` (a 64-frame fade to zero), and the new note takes the lowest Free slot, a spare.
  - If all 32 slots are busy, which needs many steals within 1.3 ms, hard-reuse the victim's slot.
  - A same-key note-on starts a new voice; the old one keeps its release (overlapping voices, as CLAP note ids allow).
  - "Quietest" is left out on purpose. It is deterministic too, but harder for users and agents to predict.
- **Note-off:** Held voices matching the PCKN wildcard become Released. **Choke** → `Stolen`.
- **Mono and legato:**
  - One slot, plus the 16-key stack.
  - New note while held: Mono retriggers the envelopes *from their current level* (no click); Legato changes pitch only. Glide applies if `glide > 0` and (`Always`, or the notes overlap).
  - Releasing the top key returns to the newest still-held key without retriggering.
  - An empty stack means release.
  - The voice keeps its first note's `note_index`, and therefore its seed.
- **Glide:** linear in semitones over `round(glide_ms · sr / 1000 / SEG)` segments, at constant time. The oscillator ramps `pitch_prev → pitch_cur` per sample. Glide is off in Poly mode for P1 (documented in `enth params voice`).
- **Ending:** at a segment end, a voice is freed when its amp envelope is past attack and below 1e-5 (−100 dB), or when a steal fade completes. Freed voices emit `Ended` and cost nothing afterwards: they are not in `active`. The spike's rule and its 40 s tail test carry over (*measured* there: 0 voices alive at the end, 0 subnormals).
- **Determinism:** there is no randomness, no clock and no host-block dependence. Events arrive on the segment grid in arrival order, so the same performance gives the same slot assignment, and the same slot order means the same summation order.

### 7. Determinism hooks

**The seed defect, measured.** design.md §5 says per-voice randomness is seeded "from the patch hash and note index". Then every parameter change reseeds random phases and detune.

`proto-seed` used the spike engine (per-voice random phase and ±10 ct random detune). It rendered each patch and the same patch with cutoff × 1.001:

| Case | Fixed render seed: RMS-env \|Δ\| mean / max | Seed = patch hash: mean / max |
| --- | --- | --- |
| P1-like, 8-voice held chord, 2 s | 0.0010 / 0.0079 dB | **1.58 / 6.61 dB** |
| simple, 8-voice held chord, 2 s | 0.0003 / 0.0013 dB | **1.68 / 6.29 dB** |
| P1-like, phrase, 4 s | 0.0014 / 0.0066 dB | **1.28 / 4.82 dB** |
| simple, phrase, 4 s | 0.0006 / 0.0022 dB | **1.00 / 4.01 dB** |

The relative L2 waveform difference is about 1e-3 with a fixed seed and about 1.4 with hash seeds. A finite-difference sensitivity (`suggest`, the multi-parameter hints, CMA-ES steps) would mostly measure reseeding noise, which is the same detune-beating sensitivity that broke the probe's descriptors (*measured*, `~/code/enth-scratch/2026-10-19-p1a/01/proto-seed`).

**Hooks:**

- **Seeds never depend on parameter values:**
  - `render_seed` comes from the render spec or challenge (CLI default constant; `--seed`);
  - `module_key = fnv1a64(module_id bytes)`, stable when other modules are added or removed;
  - per voice: `derive(derive(render_seed, module_key), note_index)`;
  - per global module: `derive(render_seed, module_key)`;
  - `note_index` counts note-ons since engine reset.

  The spike's SplitMix64 `derive` and the 24-bit `unit()` carry over to `enth-dsp::rng`, with `fnv1a64` next to them (10 lines, no dependency). Guard it with a test: moving any continuous parameter by 1e-3 relative changes the RMS envelope by less than 0.05 dB.
- **Fixed orders everywhere:** `Vec` in patch order; `BTreeMap` only in the file layer and `PlanMap`. `HashMap`/`HashSet` are already banned by the root `clippy.toml` (checked). Voice sums go in ascending slot order, generators in patch order, routes in patch order.
- **The four DSP rules:**
  - `enth-dsp::math` is the only door to `libm` (pinned `=0.2.16`). `enth-params` scale functions use it too.
  - Move the spike's numeric `disallowed-methods` into the root `clippy.toml`, as engine.md and AGENTS.md now say.
  - No `mul_add`, and no runtime-dispatched kernels. Lanes vs scalar may be chosen at runtime because they are bit-identical (*measured* in the spike).
  - Flush in every module at segment end; voices end below −100 dB.
- **Grid and events:** the 32-frame segment grid and the FIFO adapter. Note times in seconds become frames with `round(t · sr)` in f64, then take effect at the segment ceiling.
- **Exact values at rest:** smoothers start settled at load (no initial ramp), and unmodulated settled parameters pass the exact plain value. Canonicalise normalises −0.0 and rejects NaN. A non-finite output sample is a bug: `debug_assert` in the engine and a panic in the golden hasher.

## Recommended design

### Crates and files

| Crate (layer) | Files | Dependencies |
| --- | --- | --- |
| `enth-params` (L0) | `unit.rs` (Unit, suffix tables, notes), `scale.rs` (`to_norm`/`from_norm` via libm), `spec.rs` (ParamSpec, Kind, Flags, Smoothing, `const fn check`), `parse.rs`/`format.rs`, `depth.rs` (musical units → % of range), `presets.rs` (named kinds: `FREQ_AUDIBLE`, `ENV_TIME`, `LEVEL_DB`, `PERCENT`…), `macros.rs` (`params!`) | `libm =0.2.16` |
| `enth-dsp` (L0) | `math.rs` (the only libm door, `flush`, `tanh_approx`), `rng.rs` (SplitMix64, `derive`, `fnv1a64`) | `libm` |
| `enth-patch` (L1) | `catalogue/{mod,generators,voice_fx,fx,modulators,builtin}.rs`, `model.rs`, `addr.rs`, `validate.rs`, `ops.rs`, `wire.rs`, `history.rs`, `file.rs`, `schema.rs`, `migrate.rs` + `migrate/vN_to_vN+1.rs`, `canon.rs` | `serde`, `serde_json`, `sha2 =0.11.1`; dev: `proptest 1.12.0` (2026-10-10, MIT/Apache, licence files read), `insta` |
| `enth-engine` (L2) | `consts.rs`, `module.rs`, `params.rs` (ParamTable, Smoother, modulation), `plan.rs` (compile, PlanMap), `voice.rs`, `engine.rs` (segment loop, FIFO, crossfade), `link.rs` (Cmd, rings), `modules/{generators,modulators,effects}/` | `enth-dsp`, `enth-params`, `enth-patch`, `rtrb =0.4.0` (no default features), `wide` later; dev: `assert_no_alloc =1.1.2` |
| `enth-render` (L3) | `lib.rs`: `Performance` (notes in seconds or frames, velocity), `RenderSpec { sample_rate, seed, frames }`, `render()` | `enth-engine` |

Sketch of `ParamSpec`:

```rust
pub enum Unit { Hz, Ms, Cents, Semitones, Octaves, Db, Percent, Ratio, None }
pub enum Scale { Linear, Log, Power(f32) }
pub enum Kind { Float { min: f32, max: f32, scale: Scale }, Int { min: i32, max: i32 }, Enum(&'static [EnumItem]), Toggle }
pub struct EnumItem { pub key: &'static str, pub label: &'static str, pub learn: &'static str }
pub enum Smoothing { None, Linear { ms: f32 }, OnePole { ms: f32 } }
pub struct Flags { pub modulatable: bool, pub automatable: bool, pub structural: bool, pub periodic: bool }
pub struct ParamSpec { pub key: &'static str, pub name: &'static str, pub unit: Unit, pub kind: Kind, pub default: f32,
                       pub smoothing: Smoothing, pub flags: Flags, pub learn: LearnMeta, pub perceptual: Option<&'static str> }
pub struct ModuleSpec { pub kind: ModuleKind, pub key: &'static str, pub name: &'static str, pub id_prefix: &'static str,
                        pub role: Role /* Generator | VoiceFx | Fx | Modulator */, pub params: &'static [ParamSpec], pub learn: LearnMeta }
// params! { pub SvfParam / SVF_PARAMS { Mode = enum_param("mode", "Mode", &MODES, 0), Cutoff = freq("cutoff", "Cutoff", 1000.0), … } }
// → enum SvfParam { Mode, Cutoff, … } + static SVF_PARAMS: [ParamSpec; N] + const _: () = check_all(&SVF_PARAMS);
```

### Invariants (each with a test)

1. Every `ParamSpec` passes `check` at compile time: min < max, default in range, Log min > 0, enum range 0…n−1, keys unique per module.
2. `from_norm(to_norm(x))` is within 2 ulp-equivalent relative error and monotonic. `parse(format(v))` is within 4-significant-digit rounding. `+1oct` on a 20 Hz–20 kHz Log param equals 10.034 %.
3. Every successful op maps a valid patch to a valid patch. Op inverses are exact. Transactions are atomic.
4. `serialize(load(serialize(p))) == serialize(p)` byte for byte. `load` of every fixture × version reaches the current schema.
5. `canonicalise` is idempotent, invariant under generator/modulator permutation plus ID renaming, and sensitive to a one-ulp change of any live parameter. `exact_hash` and `canonical_hash` goldens: 1,000 seeded random patches, in the same file on every platform, like the spike's `patches.txt`.
6. The output is bit-identical for host block sizes 1…4,096 (port `proto-rechunk` to the real engine).
7. `process()`, including plan swap, crossfade, steal and voice end, never allocates (`assert_no_alloc`, debug) and never frees on the audio thread (a `Drop` counter in the test).
8. For every `ModuleKind`, run A for N segments, copy its state into B, then run both for M segments: the outputs are bit-identical. This is generic over `ModuleKind::ALL`.
9. Same performance, same slot assignment. Steal order, mono stack, legato (no retrigger), glide segment count, wildcard note-off and choke are each unit-tested.
10. A 40 s tail ends with 0 active voices and 0 subnormals. The hash is the same with FTZ off, FTZ only and FTZ+DAZ (the spike's tests carried over).
11. Seed stability: a 1e-3 relative move of any continuous parameter changes the 10 ms RMS envelope by < 0.05 dB. This is the `proto-seed` measurement turned into a guard.
12. Value events with a stale epoch are ignored. When the return ring is full, the plan is kept, not dropped.

### Order of work for P1a (dependencies only; report 04 owns the lane plan)

1. `enth-params`, then the `enth-patch` model, catalogue, ops, file layer and history (properties first).
2. `enth-dsp::{math, rng}`.
3. `enth-engine`: segment loop, FIFO adapter, voice manager and ParamTable, built on two spike modules (saw, SVF, ADSR) to prove invariants 6–12.
4. The rest of the modules (report 02).
5. Plan swap and crossfade (tested headless in P1a; wired to `enth-session` in P1b).
6. `canon.rs` and the migration scaffolding with a v1 fixture set (there are no v2 steps yet).

## Plan check

| Where | Now | Change |
| --- | --- | --- |
| design.md §1 | `range: Range` + `mapping: Linear\|Log\|Exp(k)\|Stepped(n)\|Enum` | `kind: Float{min,max,scale}\|Int\|Enum\|Toggle`, `Scale: Linear\|Log\|Power(k)`; add `flags {modulatable, automatable, structural, periodic}`; say the scale is part of the persisted contract (route depth, ML, randomiser), so changing it needs a migration; scale maths through pinned `libm`; note names accepted for Hz; a bare number for `%` params is a percent |
| design.md §2 | `lanes: Vec<Vec<ModuleInstance>>` | `voice_fx` (per voice) and `fx` (global) for P1; `lanes{poly}` later by migration. Add `master` (level, dB) and built-in `voice` params as addresses (`voice.glide`), `voice.amp_env` |
| design.md §2 | `params: BTreeMap<ParamKey, ParamValue>` (untagged) | In memory: a dense `Vec<f32>` in spec order. Untagged `RawValue` only in the file layer; files written dense |
| design.md §2 | `ModRoute` "depth, curve, polarity" | Depth only (% of range, normalised domain, clamped); polarity belongs to the source; curve deferred |
| design.md §2 | `apply -> Result<Op, OpError>`; `AddModule{slot, kind, id}` | `apply -> Result<Vec<Op>, OpError>`; `AddModule` carries the full instance; add `InsertRoute{index}`, `SetEnabled`, `SetAmpEnv`, `SetLabel`, `SetMeta`; `apply_all` is atomic |
| design.md §2 | `canonicalise()` "so patches that differ only by symmetry compare equal"; one `hash()` | Two hashes (exact, canonical). Canonical drops disabled modules and depth-0 routes, sorts only commutative sections (1-WL refinement), relabels IDs, excludes meta, labels and slots. Do not promise bit-identical renders for canonical-equal patches |
| design.md §3 | "Control rate … e.g. every 32 samples" | Exactly 32 frames: the segment is the processing quantum; one-segment FIFO host adapter; events at `ceil(frame/32)·32` (measured invariance). Segment order as in §5 of this report |
| design.md §3 | `trait Module { prepare, reset_voice, process }` | Four roles plus enum dispatch (`ModuleImpl`); `prepare` = constructor sized only from `PrepareCfg`; `copy_state_from` for crossfades |
| design.md §3 and architecture.md threads | "swap … with a short crossfade; old plan goes back"; "plans and events from rtrb ring buffers" | One command ring (plans + epoch-tagged events, ordered) plus a return ring with a pending slot; at most 2 plans in flight; 256-frame dual-run crossfade with state copied for matched (id, kind) modules |
| design.md §5 | "per voice from the patch hash and note index; per effect from the patch hash and module id" | `derive(derive(render_seed, fnv1a64(module_id)), note_index)`; never from parameter values (measured: 6.6 dB RMS-envelope jumps from a 0.1 % move) |
| design.md §5 | "re-chunks onto that fixed internal block" | Name it: the 32-frame segment, rendered on demand; with zero latency the event quantum must equal the render unit. The spike's 64 is just a render chunk size |
| brief.md and roadmap | "fixed internal block (64)" | "fixed 32-frame segment"; renders may call `process` with any chunk |
| architecture.md rule 4 | "A test asserts every descriptor has an implementation" | An exhaustive `match ModuleKind` in `ModuleImpl::new`: a compile error, not a test |
| architecture.md, enth-patch files | `src/{model,catalogue,ops,units,schema,migrate,canon}.rs` | Units live in `enth-params`; add `file.rs`, `validate.rs`, `addr.rs`, `wire.rs`, `history.rs` |
| architecture.md, enth-session | "compiles plans … background worker for big patches" | P1: compile on the main thread (measure; P1 patches are small); `PlanMap` stays on the main thread |
| agent-cli.md, ops | `set`, `add`, `route`, `remove` | Add `at`, `move`, `unroute`, `enable`, `macro`, `slot`, `amp_env`, route `index`; `set` covers `voice.*` and `master.*`; musical depth units (`+1oct`); 1-based macros and slots in text |
| agent-cli.md, patch example | `"cutoff": 900`, `lanes: [[filter1]]` | serde_json prints `900.0` unless a custom number formatter is added (decide with report 03); `voice_fx: [filter1]`, `master`, `amp_env` |
| AGENTS.md | "Every patch mutation is an `Op`" | Add: host automation applies `SetParam` without a history entry; seeds never depend on parameter values |

## Gaps

- **Stereo voice bus** (needed for unison spread) vs the proposed G1 budget: undecided. It needs report 02's per-module costs.
- **Crossfade cost** is unmeasured: the dual-run CPU spike against p99.9 < 25 % of the deadline. So is plan compile time on the main thread. Both are P1b test-host work.
- **1-WL canonicalisation** can, in theory, leave non-isomorphic patches tied. I have not tested whether that happens at P1 sizes (≤ 4 generators and ≤ 8 modulators); a property test with random permutations would find out.
- **Events are not sample-accurate** (0.67 ms grid at 48 kHz). The Surge precedent and the spike's quantised notes are the evidence. There is no listening test of grid jitter.
- **Glide "scaled" (constant rate) mode** is not designed. The only sources I found are anecdotal (a mono-glide course and a Serum glossary).
- **Not covered:** MPE and CLAP note expressions, sustain pedal, tempo sync, modulating a route's depth, and per-voice macro offsets (CLAP poly-mod, P1b). The types reserve room for each, but nothing is designed yet.
- **Vital's parameter and modulation model** was not checked: no official source found today.
- **RTSan** stays a manual nightly run (per the spike); nothing new here.
- **Prototype scope:** the prototypes are toy-sized (a 4-kind catalogue, a one-voice engine). They validate the shapes and the invariants, not the product code's performance.
