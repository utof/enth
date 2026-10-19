//! Every object the CLI prints or reads, as a Rust type. These types are the schema: `enth schema <name>` prints
//! their JSON Schema (draft 2020-12, from `schemars`), and the tests deserialize every snapshotted output back into
//! its type (`deny_unknown_fields`), so the code and the published schema cannot drift. Any change to a shape or to
//! a field's meaning bumps that object's version in [`super::ids`] and updates the snapshots (AGENTS.md).

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[allow(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde's skip_serializing_if passes a reference"
)]
fn is_false(b: &bool) -> bool {
    !*b
}

/// A value that is a number, a text (a unit string such as `900Hz`, an enum key, a note name) or a boolean.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum Reading {
    /// A boolean (`loudness.clipping`, a toggle with `--plain`).
    Bool(bool),
    /// A number in the stated unit.
    Num(f64),
    /// Text: a unit string, an enum key or a note name.
    Text(String),
}

// ---------------------------------------------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------------------------------------------

/// `enth.error/1`: what went wrong and how to fix it. Printed on stdout instead of the command's object.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ErrorOut {
    /// `enth.error/1`.
    pub schema: String,
    /// A stable snake-case code (`unknown_param`, `out_of_range`, `bad_expect`, `file_not_found`, ...).
    pub error: String,
    /// What was wrong, in one line.
    pub message: String,
    /// The exit code: 2 usage, 3 validation, 4 render or analysis, 5 file not found.
    pub exit: i32,
    /// What to do about it: the fix or the next command to run.
    pub hint: String,
    /// Likely intended spellings (full addresses where the error is about one), nearest first; may be empty.
    pub did_you_mean: Vec<String>,
    /// Where: a parameter address (`filter1.cutof`), an op (`ops[2]`) or a path into a file
    /// (`generators[1].params.cutoff`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// The file the problem is in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    /// For `out_of_range`: the nearest valid value, as a unit string.
    #[serde(default, rename = "try", skip_serializing_if = "Option::is_none")]
    pub try_value: Option<String>,
    /// For `out_of_range`: the lowest valid value, in the parameter's unit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    /// For `out_of_range`: the highest valid value, in the parameter's unit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    /// The names that are valid here (templates, kinds, topics, schema names), when the list is short.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub known: Vec<String>,
    /// Every problem, in order, when there is more than one (the fields above describe the first).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<ErrorItem>,
}

/// One problem of several.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ErrorItem {
    /// Where.
    pub path: String,
    /// The stable code.
    pub error: String,
    /// What was wrong.
    pub message: String,
    /// What to do about it.
    pub hint: String,
    /// Likely intended spellings; may be empty.
    pub did_you_mean: Vec<String>,
}

// ---------------------------------------------------------------------------------------------------------------
// Performance and WAV metadata
// ---------------------------------------------------------------------------------------------------------------

/// `enth.perf/1`: what is played. Given to `--perf` as a file or inline JSON (`schema` may be left out), and stored
/// in the `enth` chunk of every WAV that `enth render` writes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PerfFile {
    /// `enth.perf/1`; optional on input.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    /// The notes, at least one.
    pub notes: Vec<PerfNote>,
    /// Length of the render in seconds; it should cover the release you want measured.
    pub dur_s: f64,
}

/// One note of a performance.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PerfNote {
    /// A note name (`C3`, `A#4`, `Db2`; A4 = 440 Hz) or a MIDI key number (60 = C4).
    pub note: NoteRef,
    /// MIDI velocity 1-127 (default 100).
    #[serde(default = "default_vel")]
    pub vel: u8,
    /// Note-on time in seconds (default 0).
    #[serde(default)]
    pub on_s: f64,
    /// Note-off time in seconds.
    pub off_s: f64,
}

fn default_vel() -> u8 {
    100
}

/// A note name or a MIDI key number.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum NoteRef {
    /// A MIDI key number, 0-127.
    Key(u8),
    /// A note name such as `C3`.
    Name(String),
}

/// `enth.wavmeta/1`: the JSON in the `enth` RIFF chunk of a WAV written by `enth render`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WavMeta {
    /// `enth.wavmeta/1`.
    pub schema: String,
    /// The engine version that rendered it.
    pub engine: String,
    /// Frames per second.
    pub sample_rate: u32,
    /// The render seed.
    pub seed: u64,
    /// The exact hash of the patch (64 hex digits).
    pub patch_hash: String,
    /// The performance that was rendered.
    pub perf: PerfFile,
}

/// The performance a command used, and where it came from.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PerfOut {
    /// `flag` (`--perf`, `--note`, `--hold`, `--dur`, `--vel`), `default` (the CLI's default performance), `wav` (the
    /// `enth` chunk of the WAV), `inferred` (a note-off found in the audio) or `none` (unknown: decay, sustain and
    /// release are null).
    pub source: String,
    /// Length of the clip in seconds.
    pub dur_s: f64,
    /// The notes (empty when unknown).
    pub notes: Vec<NoteOut>,
    /// Why the performance is unknown, when it is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub why_missing: Option<String>,
}

/// One note of a reported performance.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NoteOut {
    /// The note name; null when inferred from audio.
    pub note: Option<String>,
    /// MIDI velocity; null when inferred from audio.
    pub vel: Option<u8>,
    /// Note-on time in seconds.
    pub on_s: f64,
    /// Note-off time in seconds; null when unknown.
    pub off_s: Option<f64>,
}

// ---------------------------------------------------------------------------------------------------------------
// schema, modules, params, guide, new, templates
// ---------------------------------------------------------------------------------------------------------------

/// `enth.schema/1`: the schemas `enth schema <name>` prints.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SchemaListOut {
    /// `enth.schema/1`.
    pub schema: String,
    /// One entry per schema.
    pub schemas: Vec<SchemaRef>,
    /// How to print one.
    pub hint: String,
}

/// One printable schema.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SchemaRef {
    /// The name to pass to `enth schema`.
    pub name: String,
    /// The schema id (`enth.check/1`), also its `$id`.
    pub id: String,
    /// What it describes.
    pub describes: String,
}

/// `enth.modules/1`: the module kinds and the built-ins.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ModulesOut {
    /// `enth.modules/1`.
    pub schema: String,
    /// Every module kind, in catalogue order.
    pub modules: Vec<ModuleKindOut>,
    /// The built-in pseudo-modules `voice` and `master` (their parameters are ordinary addresses).
    pub builtins: Vec<BuiltinOut>,
    /// The next command.
    pub hint: String,
}

/// One module kind.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ModuleKindOut {
    /// The kind key used in ops and files (`svf`).
    pub kind: String,
    /// Display name (`Filter`).
    pub name: String,
    /// The patch section modules of this kind live in (`generators`, `voice_fx`, `fx`, `modulators`).
    pub section: String,
    /// The prefix of new ids (`filter` gives `filter1`).
    pub id_prefix: String,
    /// One plain sentence.
    pub summary: String,
    /// How many parameters it has.
    pub params: usize,
    /// `basic`, `intermediate` or `advanced`.
    pub tier: String,
}

/// A built-in pseudo-module.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BuiltinOut {
    /// `voice` or `master`.
    pub key: String,
    /// One plain sentence.
    pub summary: String,
    /// How many parameters it has.
    pub params: usize,
}

/// `enth.params/1`: the parameters of one module kind (or `voice`, `master`), with units and ranges.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ParamsOut {
    /// `enth.params/1`.
    pub schema: String,
    /// The kind key (`svf`), or `voice` / `master`.
    pub kind: String,
    /// Display name.
    pub name: String,
    /// The section modules of this kind live in; null for the built-ins.
    pub section: Option<String>,
    /// One plain sentence.
    pub summary: String,
    /// The parameters, in spec order.
    pub params: Vec<ParamOut>,
}

/// One parameter. Values are plain numbers in `unit`; ranges are hard limits (a value outside is an error that names
/// the nearest valid one).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "a flat record of independent parameter flags"
)]
pub struct ParamOut {
    /// The key used in addresses (`filter1.cutoff`).
    pub key: String,
    /// Display name.
    pub name: String,
    /// The unit suffix values are stored in (`Hz`, `ms`, `dB`, `%`, `ct`, `st`, `:1`, `deg`); empty for plain
    /// numbers, enums and toggles.
    pub unit: String,
    /// The lowest value (null for enums and toggles).
    pub min: Option<f64>,
    /// The highest value (null for enums and toggles).
    pub max: Option<f64>,
    /// The default: a number in `unit`, an enum key, or a boolean for a toggle.
    pub default: Reading,
    /// `linear`, `log`, `power`, `int`, `enum` or `toggle`: how the knob maps its range.
    pub scale: String,
    /// The enum keys, for enums.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub values: Vec<String>,
    /// Example spellings a value may take (`800Hz`, `1.2kHz`, `A4`).
    pub accepts: Vec<String>,
    /// Routes may modulate it.
    pub modulatable: bool,
    /// Host automation may drive it.
    pub automatable: bool,
    /// Changing it rebuilds the voice plan.
    pub structural: bool,
    /// `basic`, `intermediate` or `advanced`.
    pub tier: String,
    /// What it does, in one plain sentence.
    pub effect: String,
    /// The descriptor it mainly moves (`spectrum.centroid_hz`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub moves: Option<String>,
}

/// `enth.guide/1`: the agent workflow guide, as markdown.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GuideOut {
    /// `enth.guide/1`.
    pub schema: String,
    /// `main`, `expect`, `ops` or `descriptors`.
    pub topic: String,
    /// The page, as markdown.
    pub text: String,
    /// Its size in bytes.
    pub bytes: usize,
    /// The topics `--topic` takes.
    pub topics: Vec<String>,
}

/// `enth.new/1`: a patch written from a template.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NewOut {
    /// `enth.new/1`.
    pub schema: String,
    /// The path written.
    pub wrote: String,
    /// The template's name.
    pub template: String,
    /// The engine version stamped into the file.
    pub engine: String,
    /// The module ids in the patch, in section order.
    pub modules: Vec<String>,
    /// The default performance used when a command gets no `--perf`.
    pub perf: PerfOut,
    /// The usual next command.
    pub next: String,
}

/// `enth.templates/1`: the templates `enth new --template` takes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TemplatesOut {
    /// `enth.templates/1`.
    pub schema: String,
    /// The templates.
    pub templates: Vec<TemplateOut>,
}

/// One template.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TemplateOut {
    /// The name to pass to `--template`.
    pub name: String,
    /// What it is.
    pub summary: String,
    /// Its module ids, in section order.
    pub modules: Vec<String>,
}

// ---------------------------------------------------------------------------------------------------------------
// apply, get, diff
// ---------------------------------------------------------------------------------------------------------------

/// One applied change, in words and real units.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChangeOut {
    /// The verb (`set`, `add`, `remove`, `move`, `route`, `unroute`, `enable`, `amp_env`, `macro`, `slot`, `label`,
    /// `meta`).
    pub op: String,
    /// What it changed: an address, a module id, a route `src -> dst`.
    pub target: String,
    /// The value before, in real units; absent when there was none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    /// The value after, in real units; absent when there is none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
}

/// `enth.apply/1`: ops applied to a patch.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ApplyOut {
    /// `enth.apply/1`.
    pub schema: String,
    /// The path written; null with `--dry-run`.
    pub wrote: Option<String>,
    /// True when nothing was written.
    pub dry_run: bool,
    /// What changed, in order.
    pub applied: Vec<ChangeOut>,
    /// The applied ops in resolved wire form (plain numbers), exact.
    pub ops: Vec<serde_json::Value>,
    /// The ops that undo them, in order, exact: `enth apply <patch> --ops '<inverse>'`.
    pub inverse: Vec<serde_json::Value>,
}

/// `enth.get/1`: values read from a patch.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetOut {
    /// `enth.get/1`.
    pub schema: String,
    /// The patch file.
    pub source: String,
    /// Parameter values by address, as unit strings (numbers with `--plain`). Without addresses: only the values
    /// that differ from their defaults, plus `voice.amp_env`.
    pub values: BTreeMap<String, Reading>,
    /// The modules, when asked for (no addresses, or `modules`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modules: Option<Vec<ModuleOut>>,
    /// The routes in summation order, when asked for (no addresses, or `routes`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub routes: Option<Vec<RouteOut>>,
    /// The metadata, when asked for (`meta`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<MetaOut>,
}

/// One module of a patch.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ModuleOut {
    /// Its id.
    pub id: String,
    /// Its kind key.
    pub kind: String,
    /// Its section.
    pub section: String,
    /// False when bypassed.
    pub enabled: bool,
}

/// One route.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RouteOut {
    /// `src -> dst`.
    pub route: String,
    /// Depth in percent of the destination's range.
    pub depth: f64,
}

/// Patch metadata.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MetaOut {
    /// Name.
    pub name: String,
    /// Author.
    pub author: String,
    /// Description.
    pub description: String,
    /// Tags.
    pub tags: Vec<String>,
}

/// `enth.diff/1`: how patch `b` differs from patch `a`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DiffOut {
    /// `enth.diff/1`.
    pub schema: String,
    /// The first patch file.
    pub a: String,
    /// The second patch file.
    pub b: String,
    /// True when the patches are identical (the engine stamp aside).
    pub same: bool,
    /// The differences, as the changes that turn `a` into `b`.
    pub changes: Vec<ChangeOut>,
    /// Ops that turn `a` into `b` (checked by applying them), exact; null if no op list could be verified.
    pub ops: Option<Vec<serde_json::Value>>,
    /// With `--describe`: descriptors that differ between the two renders, as `{a, b}`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub describe: Option<BTreeMap<String, DiffReading>>,
}

/// One descriptor in two renders.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DiffReading {
    /// The reading for `a` (null when absent).
    pub a: Option<Reading>,
    /// The reading for `b` (null when absent).
    pub b: Option<Reading>,
}

// ---------------------------------------------------------------------------------------------------------------
// render, describe, check, step, bench
// ---------------------------------------------------------------------------------------------------------------

/// `enth.render/1`: a WAV written, with its levels.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RenderOut {
    /// `enth.render/1`.
    pub schema: String,
    /// The WAV written.
    pub wav: String,
    /// `f32` (IEEE float, the default), `pcm16` or `pcm24`.
    pub format: String,
    /// Frames per second.
    pub sample_rate: u32,
    /// 2 (stereo).
    pub channels: u16,
    /// Length in seconds.
    pub dur_s: f64,
    /// Highest sample of the float render on either channel, dBFS; above 0 means it clips.
    pub peak_dbfs: f64,
    /// Highest inter-sample peak (4x oversampled) on either channel, dBTP.
    pub true_peak_dbtp: f64,
    /// Share of samples at or beyond full scale, percent.
    pub clipped_pct: f64,
    /// True when any sample reaches full scale: lower `master.level`.
    pub clipping: bool,
    /// The performance rendered (it is also stored in the WAV's `enth` chunk).
    pub perf: PerfOut,
    /// The first 16 hex digits of the patch's exact hash.
    pub patch_hash: String,
    /// The render seed.
    pub seed: u64,
}

/// `enth.describe/1`: perceptual descriptors of one sound. Null means not measurable (see `missing`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DescribeOut {
    /// `enth.describe/1`.
    pub schema: String,
    /// The patch or WAV described.
    pub source: String,
    /// Length in seconds.
    pub dur_s: f64,
    /// Frames per second.
    pub sample_rate: u32,
    /// The sample peak is below -80 dBFS: only `loudness` is measured.
    pub silent: bool,
    /// The performance the envelope is read against.
    pub perf: PerfOut,
    /// Pitch.
    pub pitch: Option<PitchOut>,
    /// The amplitude envelope, beat-corrected.
    pub envelope: Option<EnvelopeOut>,
    /// Brightness.
    pub spectrum: Option<SpectrumOut>,
    /// Share of spectral energy not in steady peaks: 0 is a clean tone, 1 is noise.
    pub noise_ratio: Option<f64>,
    /// Detune beating; null when absent or outside 0.15-25 Hz.
    pub detune: Option<DetuneOut>,
    /// Amplitude modulation that is not detune beating.
    pub modulation: ModulationOut,
    /// Level, measured on the float render.
    pub loudness: LoudnessOut,
    /// Why each null descriptor is null, by path.
    pub missing: BTreeMap<String, String>,
}

/// Pitch descriptors.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PitchOut {
    /// Fundamental (Hz).
    pub f0_hz: f64,
    /// Nearest note (A4 = 440 Hz).
    pub note: String,
    /// Offset from that note (cents).
    pub cents_dev: f64,
    /// 1 is a steady pitch, 0 wanders by 50 cents or more.
    pub stability: f64,
}

/// Envelope descriptors, read on the beat-corrected curve. Times in ms, levels in dB re the peak.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "a flat record of independent honesty flags"
)]
pub struct EnvelopeOut {
    /// Onset (-40 dB re peak) to within 1 dB of the peak.
    pub attack_ms: f64,
    /// Peak until the level settles near the sustain (a dB-threshold time, not the ADSR parameter).
    pub decay_ms: Option<f64>,
    /// Level held just before note-off, dB re the peak (-80 is silent).
    pub sustain_db: Option<f64>,
    /// Note-off until the level is 40 dB down.
    pub release_ms: Option<f64>,
    /// Onset to the last time within 40 dB of the peak.
    pub duration_ms: f64,
    /// Residual wobble after beat removal (dB).
    pub ripple_db: f64,
    /// False when a beat was found but was too slow to remove: sustain may be off by a few dB.
    pub beat_corrected: bool,
    /// The clip ended before the release did: `release_ms` is extrapolated.
    pub release_extrapolated: bool,
    /// Still audible at the clip end: `duration_ms` is a lower bound.
    pub duration_truncated: bool,
}

/// Brightness.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SpectrumOut {
    /// Spectral centroid (Hz): higher is brighter.
    pub centroid_hz: f64,
    /// Centroid over the first, middle and last third (Hz): reveals filter envelopes.
    pub centroid_track_hz: Option<Vec<f64>>,
}

/// Detune beating.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DetuneOut {
    /// Beat rate (Hz).
    pub beating_hz: f64,
    /// Peak-to-trough depth on the deepest harmonic (dB).
    pub depth_db: f64,
}

/// Amplitude modulation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ModulationOut {
    /// A tremolo (every harmonic moves at one rate); null when none.
    pub amp_lfo: Option<TremoloOut>,
}

/// A tremolo.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TremoloOut {
    /// Rate (Hz).
    pub hz: f64,
    /// Peak-to-trough depth (dB).
    pub depth_db: f64,
}

/// Level descriptors.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LoudnessOut {
    /// Integrated loudness (LUFS); null when silent.
    pub lufs_i: Option<f64>,
    /// Sample peak (dBFS); above 0 clips.
    pub peak_dbfs: f64,
    /// Inter-sample peak (dBTP).
    pub true_peak_dbtp: f64,
    /// Share of samples at or beyond full scale (percent).
    pub clipped_pct: f64,
    /// True when any sample reaches full scale.
    pub clipping: bool,
}

/// `enth.check/1`: pass/fail per expectation, with margins and noise.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CheckOut {
    /// `enth.check/1`.
    pub schema: String,
    /// The patch or WAV judged.
    pub source: String,
    /// The performance it was judged with.
    pub perf: PerfOut,
    /// Every item passed.
    pub pass: bool,
    /// Items that passed.
    pub passed: usize,
    /// Items that failed.
    pub failed: usize,
    /// Items within measurement noise of their threshold.
    pub thin: usize,
    /// One verdict per item, in order.
    pub results: Vec<ItemOut>,
    /// One measured multi-parameter move for all failures; null when every item passes, with `--no-fix`, for a
    /// WAV, or when no fix was found.
    pub fix: Option<FixOut>,
    /// Items that no move of the patch's parameters can satisfy.
    pub blocked: Vec<BlockedOut>,
}

/// The verdict on one expectation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "a flat record of independent verdict flags"
)]
pub struct ItemOut {
    /// The item in canonical form (`envelope.attack_ms < 10`).
    pub expect: String,
    /// The reading in `unit`; null when absent or not measured (see `why`).
    pub got: Option<Reading>,
    /// The unit of `got` (and of `margin` and `noise`, unless `margin_unit` says otherwise).
    pub unit: String,
    /// The item passed.
    pub pass: bool,
    /// Signed distance to the nearest edge of the accepted range, positive inside; null for null tests and
    /// absent readings.
    pub margin: Option<f64>,
    /// The unit of `margin` and `noise` when it differs from `unit` (`ct` for a pitch judged against a note).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub margin_unit: Option<String>,
    /// `margin` relative to the edge, percent (Hz and ms items).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub margin_pct: Option<f64>,
    /// Largest change of the reading over the nuisance re-renders (or re-analyses, for a WAV).
    pub noise: Option<f64>,
    /// |margin| <= 2 x noise, or the reading comes and goes: an inaudible change could flip it. Aim further.
    pub thin: bool,
    /// For pitch items: the nearest note and its offset (`C3+9ct`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Why `got` is null.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
    /// The reading is a lower bound (the clip ended first): only `>` items can pass.
    #[serde(default, skip_serializing_if = "is_false")]
    pub lower_bound: bool,
    /// The reading is an estimate (an extrapolated release, or an uncorrected slow beat).
    #[serde(default, skip_serializing_if = "is_false")]
    pub estimated: bool,
    /// Present in some renders and absent in others.
    #[serde(default, skip_serializing_if = "is_false")]
    pub unstable: bool,
    /// A structural suggestion for this item (not a measurement).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint: Option<ItemHintOut>,
}

/// A per-item suggestion.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ItemHintOut {
    /// The suggestion.
    pub text: String,
    /// True when it was rendered and measured.
    pub measured: bool,
}

/// One move for the whole expectation list, rendered and measured.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FixOut {
    /// True: `after` holds real values from a render, not predictions.
    pub measured: bool,
    /// The move satisfies every item.
    pub pass_all: bool,
    /// Renders the search used.
    pub renders: u32,
    /// Apply these together: `enth step <patch> --ops '<ops>'`. Exact.
    pub ops: Vec<serde_json::Value>,
    /// The measured readings after the move, by descriptor path (booleans as booleans, like `got`).
    pub after: BTreeMap<String, Reading>,
    /// One parameter would have done.
    pub single_param_enough: bool,
    /// What to know about it.
    pub note: String,
}

/// An item no move of the patch's parameters can satisfy.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BlockedOut {
    /// The item, in canonical form.
    pub expect: String,
    /// Why, in one plain sentence: the structure or the performance is the limit.
    pub why: String,
    /// A stable code for the reason: `performance`, `at_bound`, `conflict`, `no_effect`, `absent` or
    /// `unmodelled`.
    pub reason: String,
    /// Parameters sitting at a bound.
    pub at_bound: Vec<String>,
    /// Indices (into `results`) of the items this one conflicts with: passing it would push them out.
    pub conflicts_with: Vec<usize>,
}

/// The `check` part of `enth.step/1` (the fields of `enth.check/1` without `schema`, `source` and `perf`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CheckBody {
    /// Every item passed.
    pub pass: bool,
    /// Items that passed.
    pub passed: usize,
    /// Items that failed.
    pub failed: usize,
    /// Items within measurement noise.
    pub thin: usize,
    /// One verdict per item.
    pub results: Vec<ItemOut>,
    /// One measured move for all failures, or null.
    pub fix: Option<FixOut>,
    /// Items no move can satisfy.
    pub blocked: Vec<BlockedOut>,
}

/// `enth.step/1`: apply, render, describe and check in one call.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StepOut {
    /// `enth.step/1`.
    pub schema: String,
    /// The patch written.
    pub patch: String,
    /// The WAV written.
    pub wav: String,
    /// What changed (empty when no ops were given).
    pub applied: Vec<ChangeOut>,
    /// The ops that undo this step, exact.
    pub inverse: Vec<serde_json::Value>,
    /// The performance rendered and judged.
    pub perf: PerfOut,
    /// Readings by descriptor path: the features named in `--expect` plus a headline (every feature with `--full`).
    /// Null means not measurable.
    pub describe: BTreeMap<String, Option<Reading>>,
    /// The verdict, when `--expect` was given.
    pub check: Option<CheckBody>,
}

/// `enth.bench/1`: engine speed. Wall-clock time, information only: the regression gate is instruction counts in
/// `cargo xtask bench`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BenchOut {
    /// `enth.bench/1`.
    pub schema: String,
    /// The patch rendered (a path, or `template:<name>`).
    pub patch: String,
    /// Voices held at once.
    pub voices: u32,
    /// Seconds of audio rendered.
    pub seconds: f64,
    /// Frames per second.
    pub sample_rate: u32,
    /// Wall-clock time of the render (ms).
    pub wall_ms: f64,
    /// Seconds of audio per second of wall-clock time, on one core.
    pub realtime_x_per_core: f64,
    /// Wall-clock nanoseconds per voice per frame.
    pub ns_per_voice_sample: f64,
    /// How the binary was built.
    pub build: BuildOut,
    /// `wall-clock, information only`.
    pub note: String,
}

/// The build that ran a benchmark.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BuildOut {
    /// `debug` or `release`.
    pub profile: String,
    /// `<arch>-<os>`.
    pub target: String,
}

// ---------------------------------------------------------------------------------------------------------------
// Input documents
// ---------------------------------------------------------------------------------------------------------------

/// A patch file (`*.patch.json`, schema 1). Reading accepts sparse parameters (defaults fill in) and any key
/// order; writing is dense, in spec order. Parameter values are plain numbers in each parameter's unit (`enth params
/// <kind>`), unit strings (`"1.2kHz"`), enum keys or booleans.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PatchDoc {
    /// The patch file schema version, 1.
    pub schema: u32,
    /// The engine version the patch was written by.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine: Option<String>,
    /// Name, author, description, tags.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<PatchMetaDoc>,
    /// The built-in voice: its parameters plus `amp_env`, the id of the amplitude envelope.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub voice: Option<BTreeMap<String, Reading>>,
    /// The built-in master: `level` (dB).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub master: Option<BTreeMap<String, Reading>>,
    /// Oscillators and noise, summed per voice.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generators: Option<Vec<ModuleDoc>>,
    /// Per-voice effects (filters), in series.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub voice_fx: Option<Vec<ModuleDoc>>,
    /// Global effects (reverb, compressor), in series.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fx: Option<Vec<ModuleDoc>>,
    /// Envelopes and LFOs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modulators: Option<Vec<ModuleDoc>>,
    /// Modulation routes, in summation order.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub routes: Option<Vec<RouteDoc>>,
    /// Macros 1-8.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub macros: Option<Vec<MacroDoc>>,
    /// Host automation slots: 1-based slot number to address.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slots: Option<BTreeMap<String, String>>,
}

/// Patch metadata in a file.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PatchMetaDoc {
    /// Name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Author.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    /// Free text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Tags.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
}

/// One module in a file.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ModuleDoc {
    /// Module id (`osc1`, `filter1`).
    pub id: String,
    /// Kind key (`osc`, `noise`, `svf`, `adsr`, `lfo`, `reverb`, `comp`).
    pub kind: String,
    /// False when bypassed; omitted when true.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    /// A display label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Parameter values by key (sparse on input).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<BTreeMap<String, Reading>>,
}

/// One route in a file.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RouteDoc {
    /// Source: a modulator id, `velocity`, `key`, `modwheel` or `macro1`..`macro8`.
    pub src: String,
    /// Destination address (`filter1.cutoff`).
    pub dst: String,
    /// Percent of the destination's range, -100..100.
    pub depth: f64,
}

/// One macro in a file.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MacroDoc {
    /// Knob position, 0-1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<f64>,
    /// Display name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// One op of the ops JSON (`--ops`). Values are plain numbers in the parameter's unit, unit strings (`"1.2kHz"`,
/// `"600ms"`, `"-6dB"`), enum keys or booleans. The verb is the key that names it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum OpDoc {
    /// `{"set": "filter1.cutoff", "to": "1.2kHz"}`: set a parameter (also `voice.*`, `master.*`).
    Set(SetOp),
    /// `{"add": "svf", "id": "filter2", "at": "voice_fx[1]", "params": {...}}`: add a module.
    Add(AddOp),
    /// `{"remove": "filter2"}`: remove a module with its routes and slots.
    Remove(RemoveOp),
    /// `{"move": "filter2", "to": "voice_fx[0]"}`: move a module within its section.
    Move(MoveOp),
    /// `{"route": "lfo1 -> filter1.cutoff", "depth": "30%"}`: add a route or change its depth.
    Route(RouteOp),
    /// `{"unroute": "lfo1 -> filter1.cutoff"}`: remove a route.
    Unroute(UnrouteOp),
    /// `{"enable": "reverb1", "to": false}`: bypass or restore a module.
    Enable(EnableOp),
    /// `{"macro": 1, "to": 0.5, "name": "Brightness"}`: set a macro knob or its name.
    Macro(MacroOp),
    /// `{"slot": 7, "to": "filter1.cutoff"}`: assign (or with null, clear) an automation slot.
    Slot(SlotOp),
    /// `{"amp_env": "env1"}`: name the amplitude envelope.
    AmpEnv(AmpEnvOp),
    /// `{"label": "osc1", "to": "Bass"}`: set or clear a module's label.
    Label(LabelOp),
    /// `{"meta": {"name": "Soft pluck"}}`: change metadata fields.
    Meta(MetaOp),
}

/// `set`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetOp {
    /// The address.
    pub set: String,
    /// The value.
    pub to: Reading,
}

/// `add`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AddOp {
    /// The kind key.
    pub add: String,
    /// The id (default: the lowest free `<prefix><n>`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// `section[index]` (default: the end of the kind's section).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<String>,
    /// Parameter values by key (sparse).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<BTreeMap<String, Reading>>,
    /// False to add it bypassed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    /// A display label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// `remove`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RemoveOp {
    /// The module id.
    pub remove: String,
}

/// `move`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MoveOp {
    /// The module id.
    #[serde(rename = "move")]
    pub move_id: String,
    /// `section[index]`.
    pub to: String,
}

/// `route`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RouteOp {
    /// `src -> dst`.
    pub route: String,
    /// Percent of the destination's range (a number or `30%`), or a musical interval converted at the
    /// destination's current value (`+1oct`, `+12st`).
    pub depth: Reading,
    /// Position in the summation order (new routes only; default: the end).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index: Option<u32>,
}

/// `unroute`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UnrouteOp {
    /// `src -> dst`.
    pub unroute: String,
}

/// `enable`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EnableOp {
    /// The module id.
    pub enable: String,
    /// Whether it runs (default true).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<bool>,
}

/// `macro`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MacroOp {
    /// The macro number, 1-8.
    #[serde(rename = "macro")]
    pub number: u8,
    /// Knob position, 0-1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<f64>,
    /// Display name (null clears it).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// `slot`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SlotOp {
    /// The slot number, 1-64.
    pub slot: u8,
    /// An automatable address, or null to clear.
    pub to: Option<String>,
}

/// `amp_env`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AmpEnvOp {
    /// An enabled `adsr` module id.
    pub amp_env: String,
}

/// `label`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LabelOp {
    /// The module id.
    pub label: String,
    /// The label, or null to clear.
    pub to: Option<String>,
}

/// `meta`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MetaOp {
    /// Any of `name`, `author`, `description`, `tags`; the others keep their value.
    pub meta: PatchMetaDoc,
}
