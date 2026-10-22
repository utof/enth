//! **Owner: lane DG.** From a drawn sample to a labelled one: render what you label, listen to it the way the product
//! does, and read the label back (docs/r3.md, "`ds1`" and "The model contract (F3s)").
//!
//! For one sample, in this order (the cheap tests first; the first that fails is the sample's skip, counted by code):
//!
//! | Step | Skip code | What |
//! | --- | --- | --- |
//! | draw | `v1_generate`, `v1_phrase_match`, `draw_ops` | the sampler ([`super::sampler`]) |
//! | lists | `excluded_seed`, `excluded_code`, `excluded_fingerprint` | the sample's seed, generator seed, code and fingerprint on `content/gr3/exclude-*.txt` |
//! | rule 3 | `near_gr3_generator` | a generator-v1 sample within one minimum step of a listed GR3 generator target, **at any phrase** (the README's rule 3, the way the bank applies it) |
//! | canonicalise | `canonicalise` | the patch under the sample's phrase: what the phrase cannot reveal goes back to the template ([`OutputLayout::canonicalise`]), so what is rendered is what is labelled |
//! | render | `render` | the engine at the sample's native rate, the sample's phrase and render seed |
//! | loudness | `quiet` | the mid channel's integrated loudness at or below -60 LUFS (clipped renders are kept; **no filter on duration**) |
//! | intake | `intake_*` | the product's intake on the clip, as a user's file would be conditioned: onset, hold, key, bandwidth |
//! | features | `features_*` | feature set 1 ([`ModelInput::from_intake`]): the log-mel and the context vector |
//! | hearing | `hearing_*` | the cue the intake gives, with the sample's notes: what the label is relative to |
//! | label | `off_layout`, `rekey`, ... | [`OutputLayout::encode`] under that hearing: values, and the loss mask |
//! | near | `near_gr3_target` | the label within one just-noticeable step of a GR3 generator target or test preset's, [`NearIndex`] |
//!
//! The README's rule 3 is a rule on generator parameters; a `preset` or `uniform` sample has no generator parameters,
//! so the label-space test is the one that applies to every source. Both run on every `v1` sample.
//!
//! [`label_clip`] is the whole of the render-to-label path and is shared with the references (GR3's targets are
//! labelled by the same function), with the bridge from degraded clips (`shards features`), and with `exclude check`.
#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "tooling: clocks for the stage timings, files for the references"
)]

use std::fmt;
use std::fs;
use std::path::Path;
use std::time::Instant;

use enth_analysis::intake::{Intake, IntakeError};
use enth_challenge::code::ChallengeCode;
use enth_challenge::loudness;
use enth_challenge::model::{PhraseNote, PhraseView, SCORING_SEED};
use enth_challenge::priors::{ROUTE_JND, jnd};
use enth_challenge::random::{self, Generated};
use enth_challenge::spec::RandomSpec;
use enth_patch::{Patch, file};
use enth_render::{RenderSpec, render};
use enth_solver::propose::decode::{Hearing, OutputLayout};
use enth_solver::propose::features::{CONTEXT, Cue, ModelInput};
use rayon::prelude::*;

use super::anchors;
use super::sampler::{Draw, Sampler, Source};
use crate::exclude::{Exclusions, NearIndex};

/// A render at or below this integrated loudness (LUFS) is dropped as silent.
pub const QUIET_LUFS: f64 = -60.0;
/// Two labels are near duplicates when every value both carry differs by at most this many just-noticeable steps.
pub const NEAR_RADIUS: f32 = 1.0;
/// The rate GR3's targets are rendered at.
pub const REFERENCE_RATE: u32 = 48_000;

/// Why a candidate did not become a sample.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Skip {
    /// A stable snake-case code (the manifest counts by it).
    pub code: &'static str,
    /// What happened, for a person.
    pub detail: String,
}

impl Skip {
    fn new(code: &'static str, detail: impl fmt::Display) -> Skip {
        Skip {
            code,
            detail: detail.to_string(),
        }
    }
}

impl fmt::Display for Skip {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.detail)
    }
}

/// Seconds spent in each stage (summed over samples).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Timing {
    /// Drawing the sample and screening it against the lists.
    pub draw: f64,
    /// The engine's render (and the loudness).
    pub render: f64,
    /// The product's intake.
    pub intake: f64,
    /// Feature set 1.
    pub features: f64,
    /// Encoding the label and the near-duplicate test.
    pub label: f64,
}

impl Timing {
    /// Add `other`'s times to these.
    pub fn add(&mut self, other: &Timing) {
        self.draw += other.draw;
        self.render += other.render;
        self.intake += other.intake;
        self.features += other.features;
        self.label += other.label;
    }

    /// The sum of the stages.
    pub fn total(&self) -> f64 {
        self.draw + self.render + self.intake + self.features + self.label
    }
}

/// A clip to label: a patch, the phrase it plays and how it is rendered.
#[derive(Clone, Copy, Debug)]
pub struct Clip<'a> {
    /// The patch (on layout 1; not necessarily canonical).
    pub patch: &'a Patch,
    /// The notes (the first at 0 ms).
    pub notes: &'a [PhraseNote],
    /// The render's length (ms).
    pub length_ms: u32,
    /// The render's sample rate (Hz).
    pub rate: u32,
    /// The engine's render seed.
    pub render_seed: u64,
}

/// A clip, rendered, heard by the intake, featurised and labelled.
#[derive(Clone, Debug)]
pub struct Labelled {
    /// Feature set 1's log-mel, `[3, 128, 64]` flattened.
    pub mel: Vec<f32>,
    /// Feature set 1's context vector.
    pub context: [f32; CONTEXT],
    /// Layout 1's row.
    pub label: Vec<f32>,
    /// 1.0 where the row carries a loss.
    pub mask: Vec<f32>,
    /// The render's integrated loudness (LUFS).
    pub lufs: f64,
    /// The render's sample peak.
    pub peak: f32,
    /// What the intake told the model.
    pub cue: Cue,
    /// Where the time went.
    pub timing: Timing,
}

fn intake_code(e: &IntakeError) -> &'static str {
    match e {
        IntakeError::Empty => "intake_empty",
        IntakeError::NoChannels => "intake_no_channels",
        IntakeError::SampleRate(_) => "intake_sample_rate",
        IntakeError::NonFinite { .. } => "intake_non_finite",
        IntakeError::Silent => "intake_silent",
        IntakeError::BadConfig(_) => "intake_config",
        _ => "intake_other",
    }
}

/// A clip, canonicalised under its phrase and rendered, passed the loudness filter.
#[derive(Clone, Debug)]
pub struct Rendered {
    /// The patch as rendered: canonical under the clip's phrase.
    pub patch: Patch,
    /// The engine's output.
    pub out: enth_render::RenderOutput,
    /// The mid channel's integrated loudness (LUFS).
    pub lufs: f64,
    /// Seconds in the render and the loudness.
    pub seconds: f64,
}

/// Canonicalise `clip` under its own phrase and render it (the first two steps of [`label_clip`], which the clean
/// clips of `ds1 render` stop at).
///
/// # Errors
/// `canonicalise`, `render` or `quiet`.
pub fn render_clip(layout: &OutputLayout, clip: &Clip<'_>) -> Result<Rendered, Skip> {
    let view = PhraseView {
        notes: clip.notes,
        length_ms: clip.length_ms,
    };
    let mut patch = clip.patch.clone();
    layout
        .canonicalise(&mut patch, view)
        .map_err(|e| Skip::new("canonicalise", e))?;
    let t = Instant::now();
    let out = render(
        &patch,
        &view.performance(),
        &RenderSpec::new(clip.rate, clip.render_seed),
    )
    .map_err(|e| Skip::new("render", e))?;
    let lufs = loudness::integrated_lufs(&out.mid(), clip.rate)
        .ok_or_else(|| Skip::new("quiet", "silence"))?;
    let seconds = t.elapsed().as_secs_f64();
    if lufs <= QUIET_LUFS {
        return Err(Skip::new("quiet", format!("{lufs:.1} LUFS")));
    }
    Ok(Rendered {
        patch,
        out,
        lufs,
        seconds,
    })
}

/// Audio of a sample, as a file would give it: interleaved, at its own rate.
#[derive(Clone, Copy, Debug)]
pub struct Audio<'a> {
    /// `channels` samples per frame.
    pub interleaved: &'a [f32],
    /// 1 or more.
    pub channels: u16,
    /// Hz.
    pub rate: u32,
}

/// The mono mix of interleaved audio: the mean of the channels of each frame.
pub fn mono(audio: &Audio<'_>) -> Vec<f32> {
    let c = usize::from(audio.channels.max(1));
    if c == 1 {
        return audio.interleaved.to_vec();
    }
    audio
        .interleaved
        .chunks_exact(c)
        .map(|f| f.iter().sum::<f32>() / c as f32)
        .collect()
}

/// What the product does with a clip, and the label read back: the intake, feature set 1, the hearing under the
/// intake's cue and [`OutputLayout::encode`] of the `canonical` patch. `lufs` is the clip's loudness, `render_s` the
/// seconds already spent rendering it (0 for a file).
///
/// # Errors
/// The [`Skip`] of the first step that refuses.
pub fn label_audio(
    layout: &OutputLayout,
    canonical: &Patch,
    notes: &[PhraseNote],
    length_ms: u32,
    audio: &Audio<'_>,
    lufs: f64,
    render_s: f64,
) -> Result<Labelled, Skip> {
    let mut timing = Timing {
        render: render_s,
        ..Timing::default()
    };
    let peak = audio
        .interleaved
        .iter()
        .fold(0.0f32, |m, s| if s.abs() > m { s.abs() } else { m });
    let t = Instant::now();
    let intake = Intake::new(audio.interleaved, audio.channels, audio.rate)
        .map_err(|e| Skip::new(intake_code(&e), &e))?;
    timing.intake = t.elapsed().as_secs_f64();

    let t = Instant::now();
    let input = ModelInput::from_intake(&intake).map_err(|e| Skip::new(e.code(), &e))?;
    timing.features = t.elapsed().as_secs_f64();

    let t = Instant::now();
    let cue = Cue::from_intake(&intake);
    let hearing = Hearing::sample(&cue, notes, length_ms).map_err(|e| Skip::new(e.code(), &e))?;
    let labels = layout
        .encode(canonical, &hearing)
        .map_err(|e| Skip::new(e.code(), &e))?;
    timing.label = t.elapsed().as_secs_f64();
    Ok(Labelled {
        mel: input.mel,
        context: input.context,
        label: labels.values,
        mask: labels.mask.iter().map(|m| f32::from(*m)).collect(),
        lufs,
        peak,
        cue,
        timing,
    })
}

/// Render `clip` (canonicalised under its own phrase first), then run it through the product's path and read the
/// label back (see the module docs).
///
/// # Errors
/// The [`Skip`] of the first step that refuses.
pub fn label_clip(layout: &OutputLayout, clip: &Clip<'_>) -> Result<Labelled, Skip> {
    let r = render_clip(layout, clip)?;
    let mid = r.out.mid();
    let audio = Audio {
        interleaved: &mid,
        channels: 1,
        rate: clip.rate,
    };
    label_audio(
        layout,
        &r.patch,
        clip.notes,
        clip.length_ms,
        &audio,
        r.lufs,
        r.seconds,
    )
}

/// DS's near-duplicate rule (content/gr3/synth.toml, selection rule 3), as the bank applies it (`bank.rs`): the same
/// enabled modules, the same routes with depths under one route step apart, and every value either target randomized
/// within its just-noticeable step. Across phrases and levels.
pub fn near_duplicate(a: &Generated, b: &Generated) -> bool {
    let enabled = |p: &Patch| -> Vec<(String, bool)> {
        p.modules()
            .map(|(_, m)| (m.id.as_str().to_owned(), m.enabled))
            .collect()
    };
    if enabled(&a.patch) != enabled(&b.patch) || a.patch.routes.len() != b.patch.routes.len() {
        return false;
    }
    for ra in &a.patch.routes {
        let Some(rb) = b
            .patch
            .routes
            .iter()
            .find(|r| r.src == ra.src && r.dst == ra.dst)
        else {
            return false;
        };
        if ROUTE_JND.exceeded(ra.depth, rb.depth) {
            return false;
        }
    }
    a.params.iter().chain(&b.params).all(|addr| {
        match (a.patch.spec(addr), a.patch.value(addr), b.patch.value(addr)) {
            (Some(spec), Some(va), Some(vb)) => !jnd(spec).exceeded(va, vb),
            _ => false,
        }
    })
}

// ---------------------------------------------------------------------------------------------------------------
// GR3's references
// ---------------------------------------------------------------------------------------------------------------

/// One `[[target]]` of `content/gr3/synth.toml`, the few fields the references need.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TargetLine {
    /// `id`.
    pub id: String,
    /// `kind`: `generator`, `preset` or `owner-g1`.
    pub kind: String,
    /// `file`, relative to the repository root.
    pub file: Option<String>,
    /// `perf`: key, velocity, hold (ms), length (ms).
    pub perf: Option<Perf>,
    /// `anchor = true`: a dev target that is one of ds1's own anchors.
    pub anchor: bool,
}

fn quoted(rest: &str) -> Option<String> {
    let rest = rest.trim().strip_prefix('"')?;
    rest.split_once('"').map(|(v, _)| v.to_owned())
}

fn field(perf: &str, name: &str) -> Option<u32> {
    let at = perf.find(&format!("{name} = "))? + name.len() + 3;
    let digits: String = perf[at..]
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    digits.parse().ok()
}

/// The targets of `synth.toml`'s text (line by line: the file's shape is fixed and frozen).
pub fn parse_targets(text: &str) -> Vec<TargetLine> {
    let mut out: Vec<TargetLine> = Vec::new();
    for raw in text.lines() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line == "[[target]]" {
            out.push(TargetLine::default());
            continue;
        }
        if line.starts_with('[') {
            out.push(TargetLine {
                kind: "other".to_owned(),
                ..TargetLine::default()
            });
            continue;
        }
        let Some(cur) = out.last_mut() else { continue };
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        match key.trim() {
            "id" => cur.id = quoted(value).unwrap_or_default(),
            "kind" => cur.kind = quoted(value).unwrap_or_default(),
            "file" => cur.file = quoted(value),
            "anchor" => cur.anchor = value.trim() == "true",
            "perf" => {
                cur.perf = (|| {
                    Some((
                        u8::try_from(field(value, "key")?).ok()?,
                        u8::try_from(field(value, "velocity")?).ok()?,
                        field(value, "hold_ms")?,
                        field(value, "length_ms")?,
                    ))
                })();
            }
            _ => {}
        }
    }
    out
}

/// A `perf` of `synth.toml`: key, velocity, hold (ms), length (ms).
type Perf = (u8, u8, u32, u32);

/// One reference to render and label.
enum Job<'a> {
    Gen(&'a str, &'a Generated),
    Preset(&'a str, &'a Patch, Perf),
}

/// What the references hold, for the manifest.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RefSummary {
    /// Listed generator targets (codes), regenerated.
    pub generator_targets: usize,
    /// Test presets (and owner sounds) with a file, rendered at their `perf`.
    pub preset_targets: usize,
    /// References in the label index.
    pub labelled: usize,
    /// References that could not be labelled, with why (they stay covered by rule 3 where they are generator targets).
    pub unlabelled: Vec<String>,
    /// Targets whose file is not written yet (the owner's placeholders).
    pub missing_files: Vec<String>,
}

/// A listed generator target: its code, the spec the code names, and the target it regenerates.
#[derive(Debug)]
pub struct GenRef {
    /// The code as listed.
    pub code: String,
    /// The spec it names.
    pub spec: RandomSpec,
    /// The regenerated target.
    pub generated: Generated,
}

/// GR3's held-out targets, ready to compare with: the generator targets for rule 3, and every target's label.
#[derive(Debug)]
pub struct References {
    /// The listed generator targets, regenerated from their codes.
    pub generator: Vec<GenRef>,
    /// Their labels and the test presets' (the product's convention).
    pub near: NearIndex,
    /// Counts for the manifest.
    pub summary: RefSummary,
}

impl References {
    /// No references: for tests of the rest of the pipeline. A run never uses it.
    pub fn none(layout: &OutputLayout) -> References {
        References {
            generator: Vec::new(),
            near: NearIndex::new(layout, NEAR_RADIUS),
            summary: RefSummary::default(),
        }
    }

    /// The listed generator targets, regenerated, with no labels: enough for the lists and rule 3 (`screen`), not for
    /// the label-space test. `ds1 render` uses it (a render needs no intake).
    ///
    /// # Errors
    /// A listed code that does not decode or generate.
    pub fn generators(ex: &Exclusions, layout: &OutputLayout) -> Result<References, String> {
        let generator = Self::regenerate(ex)?;
        Ok(References {
            summary: RefSummary {
                generator_targets: generator.len(),
                ..RefSummary::default()
            },
            generator,
            near: NearIndex::new(layout, NEAR_RADIUS),
        })
    }

    fn regenerate(ex: &Exclusions) -> Result<Vec<GenRef>, String> {
        ex.codes()
            .map(|c| {
                let code = ChallengeCode::decode(c).map_err(|e| format!("listed code {c}: {e}"))?;
                let generated =
                    random::target(code.spec()).map_err(|e| format!("listed code {c}: {e}"))?;
                Ok(GenRef {
                    code: c.clone(),
                    spec: *code.spec(),
                    generated,
                })
            })
            .collect()
    }

    /// Regenerate and label GR3's targets: every listed code at its generator phrase, and every non-anchor preset
    /// target of `synth.toml` projected onto the layout, at its `perf`, rendered at 48 kHz.
    ///
    /// # Errors
    /// A listed code that does not decode or generate, an unreadable `synth.toml`, or a preset that does not load.
    pub fn build(gr3: &Path, ex: &Exclusions, layout: &OutputLayout) -> Result<References, String> {
        let generator = Self::regenerate(ex)?;
        let toml = gr3.join("synth.toml");
        let text = fs::read_to_string(&toml)
            .map_err(|e| format!("cannot read {}: {e}", toml.display()))?;
        let mut summary = RefSummary {
            generator_targets: generator.len(),
            ..RefSummary::default()
        };
        let mut presets: Vec<(String, Patch, Perf)> = Vec::new();
        for t in parse_targets(&text) {
            if t.anchor || !matches!(t.kind.as_str(), "preset" | "owner-g1") {
                continue;
            }
            let (Some(file_name), Some(perf)) = (t.file, t.perf) else {
                summary.missing_files.push(t.id);
                continue;
            };
            let path = gr3.join("../..").join(&file_name);
            let Ok(body) = fs::read_to_string(&path) else {
                summary.missing_files.push(t.id);
                continue;
            };
            let loaded =
                file::load(&body).map_err(|e| format!("{file_name} does not load: {e}"))?;
            let projected =
                anchors::project(layout, &loaded.patch).map_err(|e| format!("{file_name}: {e}"))?;
            presets.push((t.id, projected, perf));
        }
        summary.preset_targets = presets.len();

        let jobs: Vec<Job<'_>> = generator
            .iter()
            .map(|r| Job::Gen(&r.code, &r.generated))
            .chain(
                presets
                    .iter()
                    .map(|(id, p, perf)| Job::Preset(id, p, *perf)),
            )
            .collect();
        let labelled: Vec<(String, Result<Labelled, Skip>)> = jobs
            .par_iter()
            .map(|job| match job {
                Job::Gen(code, g) => {
                    let phrase = g.phrase();
                    let clip = Clip {
                        patch: &g.patch,
                        notes: phrase.notes,
                        length_ms: phrase.length_ms,
                        rate: REFERENCE_RATE,
                        render_seed: SCORING_SEED,
                    };
                    ((*code).to_owned(), label_clip(layout, &clip))
                }
                Job::Preset(id, patch, (key, velocity, hold_ms, length_ms)) => {
                    let notes = [PhraseNote {
                        key: *key,
                        velocity: *velocity,
                        start_ms: 0,
                        hold_ms: *hold_ms,
                    }];
                    let clip = Clip {
                        patch,
                        notes: &notes,
                        length_ms: *length_ms,
                        rate: REFERENCE_RATE,
                        render_seed: SCORING_SEED,
                    };
                    ((*id).to_owned(), label_clip(layout, &clip))
                }
            })
            .collect();
        let mut near = NearIndex::new(layout, NEAR_RADIUS);
        for (id, result) in labelled {
            match result {
                Ok(l) => {
                    near.add(id, &l.label, &l.mask);
                    summary.labelled += 1;
                }
                Err(skip) => summary.unlabelled.push(format!("{id}: {skip}")),
            }
        }
        Ok(References {
            generator,
            near,
            summary,
        })
    }
}

// ---------------------------------------------------------------------------------------------------------------
// The pipeline
// ---------------------------------------------------------------------------------------------------------------

/// A sample that passed every step.
#[derive(Clone, Debug)]
pub struct Sample {
    /// The index in the stream.
    pub index: u64,
    /// The sample's seed.
    pub seed: u64,
    /// The generator-v1 seed of a `v1` sample, else 0.
    pub gen_seed: u64,
    /// The clip's sample rate (Hz).
    pub rate: u32,
    /// Where the patch came from.
    pub source: Source,
    /// The render, read and labelled.
    pub labelled: Labelled,
}

/// The sampler, the lists and the references: everything a candidate is judged by.
#[derive(Debug)]
pub struct Pipeline {
    /// The sampler.
    pub sampler: Sampler,
    /// The exclusion lists.
    pub ex: Exclusions,
    /// GR3's targets.
    pub refs: References,
    /// The stream key.
    pub stream: String,
    /// The run seed.
    pub run_seed: u64,
}

impl Pipeline {
    /// A pipeline for one stream and run seed.
    pub fn new(
        sampler: Sampler,
        ex: Exclusions,
        refs: References,
        stream: &str,
        run_seed: u64,
    ) -> Pipeline {
        Pipeline {
            sampler,
            ex,
            refs,
            stream: stream.to_owned(),
            run_seed,
        }
    }

    /// The lists and rule 3, on a drawn sample (before any rendering).
    ///
    /// # Errors
    /// The [`Skip`] of the first test it fails.
    pub fn screen(&self, d: &Draw) -> Result<(), Skip> {
        if let Some(hit) = self.ex.seed(d.seed) {
            return Err(Skip::new("excluded_seed", hit));
        }
        let Some(g) = &d.generator else {
            return Ok(());
        };
        if let Some(hit) = self.ex.seed(g.spec.seed) {
            return Err(Skip::new("excluded_seed", hit));
        }
        if let Some(hit) = g.code.as_deref().and_then(|c| self.ex.code(c)) {
            return Err(Skip::new("excluded_code", hit));
        }
        if let Some(hit) = self.ex.fingerprint(&g.fingerprint) {
            return Err(Skip::new("excluded_fingerprint", hit));
        }
        if let Some(r) = self.refs.generator.iter().find(|r| r.spec == g.spec) {
            return Err(Skip::new(
                "excluded_code",
                format!("the spec of listed code {}", r.code),
            ));
        }
        if let Some(r) = self
            .refs
            .generator
            .iter()
            .find(|r| near_duplicate(&g.generated, &r.generated))
        {
            return Err(Skip::new(
                "near_gr3_generator",
                format!("GR3 target {}", r.code),
            ));
        }
        Ok(())
    }

    /// The label-space near-duplicate test on a labelled sample.
    ///
    /// # Errors
    /// `near_gr3_target` when a GR3 reference is within the radius.
    pub fn near(&self, l: &Labelled) -> Result<(), Skip> {
        match self.refs.near.near(&l.label, &l.mask) {
            Some((id, d)) => Err(Skip::new(
                "near_gr3_target",
                format!("{id} within {d:.2} steps"),
            )),
            None => Ok(()),
        }
    }

    /// Candidate `index` drawn, screened and rendered, not yet read by the intake: what `ds1 render` writes as clean
    /// clips for the degradation chain.
    ///
    /// # Errors
    /// The [`Skip`] that ended it.
    pub fn render_candidate(&self, index: u64) -> Result<(Draw, Rendered), Skip> {
        let d = self
            .sampler
            .draw(&self.stream, self.run_seed, index)
            .map_err(|e| Skip::new(e.code(), e))?;
        self.screen(&d)?;
        let clip = Clip {
            patch: &d.patch,
            notes: &d.notes,
            length_ms: d.length_ms,
            rate: d.rate,
            render_seed: d.render_seed,
        };
        let r = render_clip(self.sampler.layout(), &clip)?;
        Ok((d, r))
    }

    /// A drawn sample heard through `audio`, a file that is (a degraded copy of) its render: the loudness filter, the
    /// intake, the features, the label under the cue the intake gives **this** clip, and the near-duplicate test.
    ///
    /// # Errors
    /// The [`Skip`] that ended it.
    pub fn label_file(&self, d: &Draw, audio: &Audio<'_>) -> Result<Labelled, Skip> {
        let layout = self.sampler.layout();
        let mut patch = d.patch.clone();
        layout
            .canonicalise(&mut patch, d.phrase())
            .map_err(|e| Skip::new("canonicalise", e))?;
        let lufs = loudness::integrated_lufs(&mono(audio), audio.rate)
            .ok_or_else(|| Skip::new("quiet", "silence"))?;
        if lufs <= QUIET_LUFS {
            return Err(Skip::new("quiet", format!("{lufs:.1} LUFS")));
        }
        let l = label_audio(layout, &patch, &d.notes, d.length_ms, audio, lufs, 0.0)?;
        self.near(&l)?;
        Ok(l)
    }

    /// Candidate `index` of the stream: drawn, screened, rendered, labelled, screened again. Also returns where the
    /// time went (for the stages reached).
    ///
    /// # Errors
    /// The [`Skip`] that ended it.
    pub fn sample(&self, index: u64) -> (Result<Sample, Skip>, Timing) {
        let t = Instant::now();
        let mut timing = Timing::default();
        let drawn = self
            .sampler
            .draw(&self.stream, self.run_seed, index)
            .map_err(|e| Skip::new(e.code(), e))
            .and_then(|d| self.screen(&d).map(|()| d));
        timing.draw = t.elapsed().as_secs_f64();
        let d = match drawn {
            Ok(d) => d,
            Err(skip) => return (Err(skip), timing),
        };
        let clip = Clip {
            patch: &d.patch,
            notes: &d.notes,
            length_ms: d.length_ms,
            rate: d.rate,
            render_seed: d.render_seed,
        };
        let labelled = match label_clip(self.sampler.layout(), &clip) {
            Ok(l) => l,
            Err(skip) => return (Err(skip), timing),
        };
        timing.add(&labelled.timing);
        let t = Instant::now();
        let near = self.near(&labelled);
        timing.label += t.elapsed().as_secs_f64();
        if let Err(skip) = near {
            return (Err(skip), timing);
        }
        let sample = Sample {
            index,
            seed: d.seed,
            gen_seed: d.generator.as_ref().map_or(0, |g| g.spec.seed),
            rate: d.rate,
            source: d.source,
            labelled,
        };
        (Ok(sample), timing)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ds1::sampler::STREAM_TRAIN;
    use crate::exclude::default_gr3;

    #[test]
    fn synth_toml_parses_into_targets() {
        let text = fs::read_to_string(default_gr3().join("synth.toml")).unwrap();
        let t = parse_targets(&text);
        let count = |kind: &str| t.iter().filter(|t| t.kind == kind).count();
        assert_eq!(count("generator"), 112);
        assert_eq!(count("preset"), 32);
        assert_eq!(count("owner-g1"), 8);
        let anchors = t.iter().filter(|t| t.anchor).count();
        assert_eq!(anchors, 10, "5 anchor-half presets x 2 notes");
        let a = t.iter().find(|t| t.id == "hand-t-octave-arp-a").unwrap();
        assert_eq!(a.perf, Some((60, 100, 2000, 3000)));
        assert_eq!(
            a.file.as_deref(),
            Some("content/gr3/presets/octave-arp.patch.json")
        );
        let b = t.iter().find(|t| t.id == "hand-t-octave-arp-b").unwrap();
        assert_eq!(b.perf, Some((67, 80, 2000, 3000)));
    }

    fn lists(synth: &str) -> Exclusions {
        Exclusions::parse(&[
            ("exclude-synth.txt", synth.as_bytes().to_vec()),
            ("exclude-real.txt", Vec::new()),
            ("exclude-degraded.txt", Vec::new()),
        ])
        .unwrap()
    }

    fn pipeline(ex: Exclusions, refs: Option<References>) -> Pipeline {
        let sampler = Sampler::new(Some(&ex)).unwrap();
        let refs = refs.unwrap_or_else(|| References::generators(&ex, sampler.layout()).unwrap());
        Pipeline::new(sampler, ex, refs, STREAM_TRAIN, 3)
    }

    /// The first generator-v1 draw of the stream that an unlisted pipeline keeps.
    fn a_kept_v1_draw() -> (u64, Draw) {
        let open = pipeline(lists(""), None);
        (0..400)
            .find_map(|i| {
                let d = open.sampler.draw(STREAM_TRAIN, 3, i).ok()?;
                (d.generator.is_some() && open.sample(i).0.is_ok()).then_some((i, d))
            })
            .unwrap()
    }

    #[test]
    fn a_listed_seed_code_or_fingerprint_never_gets_past_the_screen() {
        let (index, d) = a_kept_v1_draw();
        let g = d.generator.as_ref().unwrap();
        // The control: unlisted, the sample is kept; each entry below fails it for the list alone.
        assert!(pipeline(lists(""), None).sample(index).0.is_ok());
        let long = g.code.clone().unwrap();
        assert!(
            long.len() > 20,
            "a ds1 spec is no preset's, so its code is the long form: {long}"
        );
        for (entry, code) in [
            (format!("seed-range {0}..{0}", g.spec.seed), "excluded_seed"),
            (
                format!("seed-range {}..{}", g.spec.seed - 1, g.spec.seed + 1),
                "excluded_seed",
            ),
            (format!("seed-range {0}..{0}", d.seed), "excluded_seed"),
            (format!("code {long}"), "excluded_code"),
            (
                format!("fingerprint {}", g.fingerprint.to_uppercase()),
                "excluded_fingerprint",
            ),
        ] {
            let (r, _) = pipeline(lists(&entry), None).sample(index);
            assert_eq!(r.unwrap_err().code, code, "{entry}");
        }
        // A code from another engine (or target prefix) names the same spec: still the same sample.
        let other = ChallengeCode::long(
            g.spec,
            enth_challenge::code::EngineVersion::CURRENT,
            enth_challenge::code::TargetPrefix(0xdead_beef),
        )
        .unwrap()
        .encode();
        assert_ne!(other, long);
        let (r, _) = pipeline(lists(&format!("code {other}")), None).sample(index);
        assert_eq!(r.unwrap_err().code, "excluded_code");
    }

    #[test]
    fn gr3s_seed_block_is_never_drawn_into_a_sample() {
        // With the real lists, a generator seed anywhere in the block is skipped, whatever else is true, and the
        // block's edges are inclusive.
        let real = Exclusions::load(&default_gr3()).unwrap();
        let p = pipeline(real, None);
        let (_, d) = a_kept_v1_draw();
        assert!(p.screen(&d).is_ok());
        for (seed, listed) in [
            (769_999_999_999u64, false),
            (770_000_000_000, true),
            (770_000_500_000, true),
            (770_999_999_999, true),
            (771_000_000_000, false),
        ] {
            let mut moved = d.clone();
            moved.generator.as_mut().unwrap().spec.seed = seed;
            let hit = p.screen(&moved);
            assert_eq!(hit.is_err(), listed, "{seed}: {hit:?}");
            if listed {
                assert_eq!(hit.unwrap_err().code, "excluded_seed");
            }
        }
        // The sample's own seed is checked too.
        let mut own = d.clone();
        own.seed = 770_123_456_789;
        assert_eq!(p.screen(&own).unwrap_err().code, "excluded_seed");
    }

    #[test]
    fn a_near_duplicate_of_a_gr3_target_never_gets_past_the_pipeline() {
        let (index, d) = a_kept_v1_draw();
        // Rule 3: a generator target is a near duplicate of itself, and (almost surely) not of another.
        let g = &d.generator.as_ref().unwrap().generated;
        assert!(near_duplicate(g, g));
        let other = random::target(&enth_challenge::spec::RandomSpec {
            seed: d.generator.as_ref().unwrap().spec.seed ^ 0x5555,
            ..d.generator.as_ref().unwrap().spec
        })
        .unwrap();
        assert!(!near_duplicate(g, &other));
        // Label space: the sample's own label as a GR3 reference stops it, for every source.
        let sampler = Sampler::new(None).unwrap();
        let none = References::none(sampler.layout());
        let p = Pipeline::new(sampler, lists(""), none, STREAM_TRAIN, 3);
        let mut seen = std::collections::BTreeSet::new();
        for i in index..index + 400 {
            let (Ok(s), _) = p.sample(i) else { continue };
            if !seen.insert(s.source.name()) {
                continue;
            }
            let mut refs = References::none(p.sampler.layout());
            refs.near
                .add("gr3-target", &s.labelled.label, &s.labelled.mask);
            let q = pipeline(lists(""), Some(refs));
            let (r, _) = q.sample(i);
            assert_eq!(
                r.unwrap_err().code,
                "near_gr3_target",
                "{} sample {i}",
                s.source.name()
            );
            if seen.len() == 3 {
                break;
            }
        }
        assert_eq!(seen.len(), 3, "{seen:?}");
    }

    #[test]
    fn an_anchor_on_a_list_stops_the_sampler() {
        for entry in ["preset-stem warm-reese", "preset-name Warm Reese"] {
            let ex = lists(entry);
            assert!(
                Sampler::new(Some(&ex)).unwrap_err().contains("warm-reese"),
                "{entry}"
            );
        }
        let hash = anchors::load(&OutputLayout::v1()).unwrap()[0]
            .sha256
            .clone();
        let ex = lists(&format!("file-sha256 {}", hash.to_uppercase()));
        assert!(Sampler::new(Some(&ex)).is_err());
        // The test half of the presets is not an anchor.
        let real = Exclusions::load(&default_gr3()).unwrap();
        assert!(Sampler::new(Some(&real)).is_ok());
    }

    #[test]
    fn a_labelled_clip_is_a_pure_function_of_its_inputs() {
        let layout = OutputLayout::v1();
        let sampler = Sampler::new(None).unwrap();
        let mut done = 0;
        for index in 0..40 {
            let Ok(d) = sampler.draw(STREAM_TRAIN, 5, index) else {
                continue;
            };
            let clip = Clip {
                patch: &d.patch,
                notes: &d.notes,
                length_ms: d.length_ms,
                rate: d.rate,
                render_seed: d.render_seed,
            };
            let (Ok(a), Ok(b)) = (label_clip(&layout, &clip), label_clip(&layout, &clip)) else {
                continue;
            };
            assert_eq!(a.mel, b.mel);
            assert_eq!(a.label, b.label);
            assert_eq!(a.mask, b.mask);
            assert_eq!(a.mel.len(), 3 * 128 * 64);
            assert_eq!(a.label.len(), 102);
            assert!(a.mask.iter().all(|m| *m == 0.0 || *m == 1.0));
            assert!(a.lufs > QUIET_LUFS);
            done += 1;
            if done == 3 {
                break;
            }
        }
        assert!(done >= 1, "no sample of the first 40 labelled");
    }
}
