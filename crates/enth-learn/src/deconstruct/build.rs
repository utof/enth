//! **Owner: lane DEC.** The lesson a plan becomes, with its patches and its automatic text (docs/learn.md,
//! "Deconstruct lessons (lane DEC)"; report 01 §6, steps 5 to 7).
//!
//! | Step | Kind | What |
//! | --- | --- | --- |
//! | `intro` | read | what the lesson does, the parts in order |
//! | `hear` | listen | the whole sound and where the lesson starts |
//! | `hear-<part>` | listen | the sound without and with the part, through the phrase that reveals it |
//! | `build-<part>` | recreate, or do | rebuild the part: a real round on a patch target with only the part editable; a Do step (patch predicates, and sound items when they verify) when `sound/1` scores the start above [`ROUND_MAX_START`] |
//! | `missing-hear`, `missing` | listen, choose | "What disappeared?": the whole sound and one ablation, then which part it lacks (1 of up to 4, pairwise at least two just-noticeable steps apart) |
//! | `vary` | do | a variation goal checked by expectations derived from the preset ("keep it plucky, make it brighter") |
//! | `recap` | read | what each part did, and the fine detail left in from the start |
//!
//! The automatic text is a template over each part's name, its controls (`learn.desc`) and the measured changes
//! (the coach's descriptor rows: "it darkens the sound and sharpens the attack"). Factory studies replace it with
//! human-edited text (`lesson-gen` never overwrites an existing text file); a user's presets keep it.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use enth_challenge::lesson_phrases::{CHORD, LONG_HOLD};
use enth_challenge::phrases::{HELD, LEGATO_PAIR, OCTAVE_PAIR};
use enth_challenge::v2::{InlinePatch, Selector, touches};
use enth_challenge::{Assists, Challenge, ChallengeMeta, Constraints, PhraseId, Start, Target};
use enth_patch::catalogue::ModuleKind;
use enth_patch::{ModSource, Op, ParamAddr, Patch, PatchMeta, apply, ops_between};

use super::DeconstructError;
use super::element::{Element, Part, slug};
use super::measure::{
    Renders, STEP_RENDERS, StepMeasure, describe, measure_step, pick_options, propose_items,
    scoring, verify_items,
};
use super::plan::{Aspect, Change, FineWhy, Plan, Rank, Way};
use crate::check::{Check, Cmp, Knob, Plain, Predicate, TouchLog};
use crate::lesson::{Example, Layout, Lesson, ParamKey, Step, StepKind, StepView, Tier};
use crate::pack::{LessonId, PatchRef, StepId, TextRef};

/// A step whose start `sound/1` already scores above this sound % is taught as a Do step, not a round: its round
/// would pass, or nearly, on the start (the pass mark is 90).
pub const ROUND_MAX_START: f64 = 85.0;

/// The most sound items a Do step or the variation goal checks.
const MAX_ITEMS: usize = 3;

/// The most options "What disappeared?" offers.
const MAX_OPTIONS: usize = 4;

/// The most candidates measured for it (pairwise, so the cost grows with the square).
const MAX_CANDIDATES: usize = 6;

/// How a lesson is generated: its names, and what to build besides the steps.
#[derive(Clone, Debug, PartialEq)]
pub struct Options {
    /// The lesson's id (its directory).
    pub lesson: LessonId,
    /// The prefix of its patch files (`patches/<prefix>-1.patch.json`): a slug, unique in the pack.
    pub prefix: String,
    /// The title.
    pub title: String,
    /// The objective; `None` writes "Hear what each part of <name> does, and rebuild it part by part, by ear."
    pub objective: Option<String>,
    /// The tier (hints by tier).
    pub tier: Tier,
    /// The tag every embedded challenge carries besides the prefix (`studies`, or `deconstruct`).
    pub tag: String,
    /// Names for elements by id ("the snap" for `env2 -> filter1.cutoff`), instead of the automatic ones.
    pub labels: BTreeMap<String, String>,
    /// Authored hints for an element's build step, by element id, instead of the automatic ones.
    pub hints: BTreeMap<String, Vec<String>>,
    /// Build "What disappeared?" (measures the candidates' audibility).
    pub choice: bool,
    /// Build the variation goal.
    pub variation: bool,
    /// Elements (by id) never taught as a round: `lesson-gen` adds the ones `closest` cannot pass within
    /// lesson-check's budget, and regenerates.
    pub no_round: BTreeSet<String>,
}

impl Options {
    /// The defaults for `preset`: the lesson id and patch prefix from its name, "Deconstruct: <name>", intermediate.
    pub fn for_preset(preset: &Patch) -> Options {
        let name = preset_name(preset);
        let id = {
            let s = slug(&name, 40);
            if s.is_empty() { "preset".to_owned() } else { s }
        };
        Options {
            lesson: LessonId::new(&id)
                .unwrap_or_else(|_| LessonId::new("preset").unwrap_or_else(|_| unreachable!())),
            prefix: id,
            title: format!("Deconstruct: {name}"),
            objective: None,
            tier: Tier::Intermediate,
            tag: "deconstruct".to_owned(),
            labels: BTreeMap::new(),
            hints: BTreeMap::new(),
            choice: true,
            variation: true,
            no_round: BTreeSet::new(),
        }
    }
}

/// The preset's name, or "your preset".
pub fn preset_name(preset: &Patch) -> String {
    let n = preset.meta.name.trim();
    if n.is_empty() {
        "your preset".to_owned()
    } else {
        n.to_owned()
    }
}

/// How a step is taught, decided from its measure.
#[derive(Clone, Debug, PartialEq)]
pub enum Teach {
    /// A real round on the step's patch target.
    Round,
    /// A Do step with this check (patch predicates, with sound items when they verified).
    Check(Check),
    /// Not heard in its context and nothing to check: ride along with the neighbouring step.
    Merge,
}

impl Teach {
    /// `round`, `check` or `merge`.
    pub fn name(&self) -> &'static str {
        match self {
            Teach::Round => "round",
            Teach::Check(_) => "check",
            Teach::Merge => "merge",
        }
    }
}

/// One element in the report.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct ElementReport {
    /// Its id.
    pub id: String,
    /// Its flow.
    pub flow: &'static str,
    /// Its name.
    pub label: String,
    /// The phrase it was measured through.
    pub phrase: Option<u16>,
    /// `sound/1` of the preset without it, in just-noticeable steps.
    pub jnd: f64,
    /// The same as a sound %.
    pub percent: f64,
    /// The descriptor rows it moves, in words ("brighter (far)").
    pub changes: Vec<String>,
    /// `step N`, or `fine: <why>`.
    pub fate: String,
}

/// One step in the report.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct StepReport {
    /// Its build step's id.
    pub id: String,
    /// The elements it adds.
    pub elements: Vec<String>,
    /// Its phrase.
    pub phrase: u16,
    /// `round` or `check`.
    pub teach: &'static str,
    /// `sound/1` of its start against its target, as a sound %.
    pub start_percent: f64,
    /// The same in just-noticeable steps.
    pub start_jnd: f64,
    /// The descriptor rows that move between start and target.
    pub changes: Vec<String>,
    /// A Do step's check items, as written.
    pub items: Vec<String>,
}

/// What a generation found and spent, for `lesson-gen`'s report and the lesson's comments.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct Report {
    /// The preset's name.
    pub preset: String,
    /// Every element, in patch order.
    pub elements: Vec<ElementReport>,
    /// The taught steps, in order.
    pub steps: Vec<StepReport>,
    /// "What disappeared?": the options' labels, the answer, the closest pair's distance; `None` when left out.
    pub missing: Option<(Vec<String>, usize, f64)>,
    /// The variation goal's items; `None` when left out.
    pub variation: Option<Vec<String>>,
    /// Steps merged into a neighbour because they were not heard in their context.
    pub merged: Vec<String>,
    /// Renders spent.
    pub renders: Renders,
}

impl Report {
    /// The report as compact JSON.
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    /// The report as lines of text (no clock: it is written into the lesson as comments).
    pub fn lines(&self) -> Vec<String> {
        let mut out = vec![format!(
            "Ablation of {}: {} elements, {} taught; {} renders (ablation {}, steps {}, choice {}, variation {}).",
            self.preset,
            self.elements.len(),
            self.steps.len(),
            self.renders.total(),
            self.renders.ablation,
            self.renders.steps,
            self.renders.choice,
            self.renders.variation
        )];
        for e in &self.elements {
            out.push(format!(
                "- {} ({}, phrase {}): {:.2} JND, {:.1} %; {}{}",
                e.id,
                e.flow,
                e.phrase.map_or("none".to_owned(), |p| p.to_string()),
                e.jnd,
                e.percent,
                e.fate,
                if e.changes.is_empty() {
                    String::new()
                } else {
                    format!("; {}", e.changes.join(", "))
                }
            ));
        }
        out
    }
}

/// A generated deconstruct lesson: the lesson, the files it names, and what was measured.
#[derive(Clone, Debug, PartialEq)]
pub struct Generated {
    /// The plan it was built from (after merges).
    pub plan: Plan,
    /// The lesson.
    pub lesson: Lesson,
    /// Its patches by pack path (`patches/acid-snap-1.patch.json`).
    pub patches: BTreeMap<String, Patch>,
    /// Its texts by file name (`intro.md`).
    pub texts: BTreeMap<String, String>,
    /// Comment lines per step id: what was measured for it.
    pub notes: BTreeMap<String, Vec<String>>,
    /// The report.
    pub report: Report,
}

// ---------------------------------------------------------------------------------------------------------------
// Words
// ---------------------------------------------------------------------------------------------------------------

/// `text` with its first letter upper-case.
fn cap(text: &str) -> String {
    let mut c = text.chars();
    c.next()
        .map_or_else(String::new, |f| f.to_uppercase().chain(c).collect())
}

/// A list in prose: "a", "a and b", "a, b and c".
fn and_list(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [init @ .., last] => format!("{} and {last}", init.join(", ")),
    }
}

/// A value as prose: the display with a space before its unit ("320 Hz", "55 %", "+12 st").
fn value_text(spec: &enth_params::ParamSpec, v: f32) -> String {
    let s = spec.display_rounded(v, 3).to_string();
    match spec.kind {
        enth_params::Kind::Enum(_) | enth_params::Kind::Toggle => s,
        _ => match s.find(|c: char| c.is_ascii_alphabetic() || c == '%') {
            Some(i) if i > 0 => format!("{} {}", &s[..i], &s[i..]),
            _ => s,
        },
    }
}

/// What a change says the element does, read as "the sound *without* it, against the sound with it".
fn effect(c: &Change) -> Option<&'static str> {
    use Aspect as A;
    use Way as W;
    Some(match (c.aspect, c.way) {
        (A::Brightness, W::Higher) => "darkens the sound",
        (A::Brightness, W::Lower) => "brightens the sound",
        (A::Harmonics, W::Higher) => "fills in the even harmonics",
        (A::Harmonics, W::Lower) => "makes it hollower",
        (A::Pitch, W::Higher) => "lowers the pitch",
        (A::Pitch, W::Lower) => "raises the pitch",
        (A::Pitch, W::Gone) => "gives it a clear pitch",
        (A::Pitch, W::New) => "blurs the pitch",
        (A::Attack, W::Higher) => "sharpens the attack",
        (A::Attack, W::Lower) => "softens the attack",
        (A::Decay, W::Higher) => "shortens the decay",
        (A::Decay, W::Lower | W::Gone) => "lengthens the decay",
        (A::Sustain, W::Higher) => "lets the held part fall away",
        (A::Sustain, W::Lower) => "holds the note up",
        (A::Release, W::Higher) => "shortens the tail",
        (A::Release, W::Lower) => "lengthens the tail",
        (A::Noisiness, W::Higher) => "cleans the sound up",
        (A::Noisiness, W::Lower | W::Gone) => "adds noise",
        (A::Beating, W::Gone) => "adds a slow beat",
        (A::Beating, W::Higher) => "slows the beat",
        (A::Beating, W::Lower) => "speeds up the beat",
        (A::Tremolo, W::Gone) => "adds a tremolo",
        (A::Tremolo, W::Higher) => "slows the tremolo",
        (A::Tremolo, W::Lower) => "speeds up the tremolo",
        (A::Sweep, W::Gone) => "adds a filter sweep",
        (A::Sweep, W::New) => "steadies the brightness",
        (A::Sweep, W::Higher) => "tames the filter sweep",
        (A::Sweep, W::Lower) => "deepens the filter sweep",
        (A::Movement, W::Gone) => "adds movement",
        (A::Beating | A::Tremolo | A::Movement, W::New) => "steadies the sound",
        (A::Movement, W::Higher) => "slows the movement",
        (A::Movement, W::Lower) => "speeds up the movement",
        _ => return None,
    })
}

/// A listening cue for a change.
fn cue(c: &Change) -> &'static str {
    use Aspect as A;
    match c.aspect {
        A::Brightness | A::Harmonics => {
            "Listen to the brightness: is the target duller or brighter than yours?"
        }
        A::Pitch => "Listen to the pitch, and to how one note moves to the next.",
        A::Attack => "Listen to the very start of each note: how fast does it speak?",
        A::Decay | A::Sustain => {
            "Listen to how the note falls after its start, and what level it holds."
        }
        A::Release => "Listen to the tail after the note lets go.",
        A::Noisiness => "Listen for hiss or breath under the tone.",
        A::Beating => "Listen for a slow beat, like two strings slightly out of tune.",
        A::Tremolo => "Listen for the level pulsing.",
        A::Sweep => "Listen to the brightness move during the note.",
        A::Movement => "Listen for movement: a wobble, a sweep or a slow beat.",
    }
}

/// "It darkens the sound and sharpens the attack." from the changes (at most two, the furthest first).
fn effect_sentence(changes: &[Change]) -> Option<String> {
    let mut words: Vec<String> = Vec::new();
    for c in changes {
        if let Some(e) = effect(c)
            && !words.iter().any(|w| w.ends_with(e))
        {
            let strength = if c.steps < 2.0 { "slightly " } else { "" };
            words.push(format!("{strength}{e}"));
        }
        if words.len() == 2 {
            break;
        }
    }
    (!words.is_empty()).then(|| format!("It {}.", and_list(&words)))
}

fn phrase_note(p: PhraseId) -> &'static str {
    match p {
        x if x == LEGATO_PAIR => {
            " Both play two overlapping notes, so you hear how one note moves to the next."
        }
        x if x == OCTAVE_PAIR => " Both play two notes an octave apart, the second softer.",
        x if x == LONG_HOLD => " Both play one long note, so slow movement has time to show.",
        x if x == CHORD => " Both play a chord.",
        _ => "",
    }
}

/// A selector's name in prose with its link: "the filter's [cutoff](enth:filter1.cutoff)", "envelope 2's
/// [attack](enth:env2.attack)", "[glide](enth:voice.glide)", "the [second oscillator](enth:osc2.*)".
fn control_link(patch: &Patch, sel: &Selector) -> String {
    match sel {
        Selector::Param(a) => {
            let name = patch
                .spec(a)
                .map_or_else(|| a.param().to_owned(), |s| s.name.to_lowercase());
            match a.target() {
                enth_patch::Target::Module(id) => {
                    match patch.find(id).map(|(_, m)| module_name(patch, m)) {
                        Some(o) if o.ends_with(|c: char| c.is_ascii_digit()) => {
                            format!("{o}'s [{name}](enth:{a})")
                        }
                        Some(o) => format!("the {o}'s [{name}](enth:{a})"),
                        None => format!("[{name}](enth:{a})"),
                    }
                }
                _ => format!("[{name}](enth:{a})"),
            }
        }
        Selector::Module(enth_patch::Target::Module(id)) => {
            let name = patch
                .find(id)
                .map_or_else(|| id.to_string(), |(_, m)| module_name(patch, m));
            if name.ends_with(|c: char| c.is_ascii_digit()) {
                format!("[{name}](enth:{id}.*)")
            } else {
                format!("the [{name}](enth:{id}.*)")
            }
        }
        Selector::Route(r) => {
            let dst = patch
                .spec(&r.dst)
                .map_or_else(|| r.dst.param().to_owned(), |s| s.name.to_lowercase());
            let src = match &r.src {
                ModSource::Module(id) => patch
                    .find(id)
                    .map_or_else(|| id.to_string(), |(_, m)| module_name(patch, m)),
                other => other.to_string(),
            };
            format!(
                "the [route from {src} to the {dst}](enth:{}->{})",
                r.src, r.dst
            )
        }
        other => format!("[{other}](enth:{other})"),
    }
}

/// Whether an element can be heard in an aspect at all (the coach's rule, `Aspect::involves`, by what the element
/// changes), so the text names what the element does and not a side effect of the measurement.
fn involves(patch: &Patch, e: &Element, aspect: Aspect) -> bool {
    use Aspect as A;
    let kind_of = |a: &ParamAddr| match a.target() {
        enth_patch::Target::Module(id) => patch.find(id).map(|(_, m)| m.kind),
        _ => None,
    };
    let route_dst = e.parts.iter().find_map(|p| match p {
        Part::Route(r) => Some(r.dst.clone()),
        _ => None,
    });
    let kind = match &route_dst {
        Some(dst) => kind_of(dst),
        None => e.main.as_ref().and_then(kind_of),
    };
    let param = route_dst
        .as_ref()
        .or(e.main.as_ref())
        .map(ParamAddr::param)
        .unwrap_or_default();
    match (e.flow, kind) {
        (super::element::Flow::Voice, _) => matches!(aspect, A::Pitch | A::Attack),
        (_, Some(ModuleKind::Osc)) if param == "semi" || param == "fine" => {
            matches!(
                aspect,
                A::Pitch | A::Beating | A::Sweep | A::Brightness | A::Movement
            )
        }
        (_, Some(ModuleKind::Osc)) => matches!(
            aspect,
            A::Brightness | A::Harmonics | A::Beating | A::Noisiness | A::Pitch
        ),
        (_, Some(ModuleKind::Noise)) => matches!(aspect, A::Noisiness | A::Brightness),
        (_, Some(ModuleKind::Svf)) => matches!(
            aspect,
            A::Brightness | A::Harmonics | A::Sweep | A::Movement
        ),
        (_, Some(ModuleKind::Adsr)) => {
            matches!(aspect, A::Attack | A::Decay | A::Sustain | A::Release)
        }
        (_, Some(ModuleKind::Reverb)) => matches!(aspect, A::Release | A::Sustain | A::Decay),
        _ => true,
    }
}

/// What an element is, in a sentence of sound and values (no links): "A low-pass filter at 320 Hz with 55 %
/// resonance.", "Envelope 2 opens the cutoff at the start of each note, then closes it: a filter envelope."
fn what_it_is(patch: &Patch, e: &Element) -> String {
    let v = |a: &str| -> Option<String> {
        let addr: ParamAddr = a.parse().ok()?;
        let spec = patch.spec(&addr)?;
        Some(value_text(spec, patch.value(&addr)?))
    };
    let id = |p: &str| {
        e.id.split('.')
            .next()
            .map(|m| format!("{m}.{p}"))
            .unwrap_or_default()
    };
    match e.flow {
        super::element::Flow::Source => {
            let module = e.id.as_str();
            let kind = enth_patch::ModuleId::new(module)
                .ok()
                .and_then(|m| patch.find(&m).map(|(_, x)| x.kind));
            if kind == Some(ModuleKind::Noise) {
                format!(
                    "Noise under the tone: {} noise at {}.",
                    v(&id("color")).unwrap_or_default(),
                    v(&id("level")).unwrap_or_default()
                )
            } else {
                {
                    let wave = v(&id("wave")).unwrap_or_default();
                    let semi = v(&id("semi")).filter(|s| s != "0 st" && s != "+0 st");
                    let fine = v(&id("fine")).filter(|s| s != "0 ct" && s != "+0 ct");
                    let mut bits = vec![format!("a {wave} wave")];
                    bits.extend(semi.map(|s| format!("tuned {s}")));
                    bits.extend(fine.map(|s| format!("detuned {s}")));
                    bits.extend(
                        v(&id("level"))
                            .filter(|l| l != "100 %")
                            .map(|l| format!("at {l}")),
                    );
                    format!("{}.", cap(&bits.join(", ")))
                }
            }
        }
        super::element::Flow::Filter if e.id.rsplit('.').next() == Some("keytrack") => format!(
            "The cutoff follows the note: {} key tracking, so higher notes come out brighter.",
            v(&id("keytrack")).unwrap_or_default()
        ),
        super::element::Flow::Filter if e.id.rsplit('.').next() == Some("drive") => {
            format!(
                "{} of drive into the filter: a little grit and weight.",
                v(&id("drive")).unwrap_or_default()
            )
        }
        super::element::Flow::Filter => format!(
            "A {} filter at {} with {} resonance.",
            match v(&id("mode")).as_deref() {
                Some("lp") => "low-pass",
                Some("hp") => "high-pass",
                Some("bp") => "band-pass",
                Some("notch") => "notch",
                _ => "",
            },
            v(&id("cutoff")).unwrap_or_default(),
            v(&id("resonance")).unwrap_or_default()
        ),
        super::element::Flow::Amp => format!(
            "Attack {}, decay {}, sustain {}, release {}.",
            v(&id("attack")).unwrap_or_default(),
            v(&id("decay")).unwrap_or_default(),
            v(&id("sustain")).unwrap_or_default(),
            v(&id("release")).unwrap_or_default()
        ),
        super::element::Flow::Voice if e.id == "voice.pan" => "Every voice is panned.".to_owned(),
        super::element::Flow::Voice => {
            let glide = v("voice.glide").filter(|g| g != "0 ms");
            match (v("voice.mode").as_deref(), glide) {
                (Some("legato"), Some(g)) => format!(
                    "One note at a time: a note played before the last one lets go slides there in {g}, without starting again."
                ),
                (Some("legato"), None) => "One note at a time: a note played before the last one lets go changes pitch without starting again.".to_owned(),
                (Some("mono"), Some(g)) => format!("One note at a time, each new one starting again and sliding in {g}."),
                (Some("mono"), None) => "One note at a time: each new note cuts the last one off.".to_owned(),
                (_, Some(g)) => format!("Each note slides in from the last pitch in {g}."),
                _ => "The voice settings.".to_owned(),
            }
        }
        super::element::Flow::Effects => {
            match e.id.trim_end_matches(|c: char| c.is_ascii_digit()) {
                "reverb" => format!(
                    "A reverb: {} wet, with a {} tail.",
                    v(&id("wet")).unwrap_or_default(),
                    v(&id("decay")).unwrap_or_default()
                ),
                _ => format!("{}.", cap(&e.label)),
            }
        }
        super::element::Flow::Modulation | super::element::Flow::Macros => {
            let Some(Part::Route(r)) = e.parts.iter().find(|p| matches!(p, Part::Route(_))) else {
                return format!("{}.", cap(&e.label));
            };
            let src = match &r.src {
                ModSource::Module(m) => patch
                    .find(m)
                    .map_or_else(|| m.to_string(), |(_, x)| cap(&module_name(patch, x))),
                ModSource::Key => "The note played".to_owned(),
                ModSource::Velocity => "How hard a note is played".to_owned(),
                ModSource::ModWheel => "The mod wheel".to_owned(),
                ModSource::Macro(i) => format!("Macro {}", i.one_based()),
            };
            let is_env = matches!(&r.src, ModSource::Module(m) if patch.find(m).is_some_and(|(_, x)| x.kind == ModuleKind::Adsr));
            let rate = match &r.src {
                ModSource::Module(m) => patch.find(m).and_then(|(_, x)| {
                    let spec = x.kind.param("rate")?;
                    Some(value_text(spec, x.get("rate")?))
                }),
                _ => None,
            };
            let at = rate.map(|r| format!(" at {r}")).unwrap_or_default();
            match (r.dst.param(), is_env) {
                ("cutoff", true) => format!(
                    "{src} opens the cutoff at the start of each note, then closes it: {}.",
                    e.label
                ),
                ("cutoff", false) => format!("{src} sweeps the cutoff up and down{at}."),
                ("semi", true) => format!(
                    "{src} throws the pitch up at the start of each note and drops it fast: {}.",
                    e.label
                ),
                ("semi" | "fine", false) => format!("{src} wobbles the pitch{at}: {}.", e.label),
                ("level", false) => format!("{src} pulses the level{at}: {}.", e.label),
                ("level", true) => format!("{src} shapes the level of each note: {}.", e.label),
                ("pan", _) => format!(
                    "{src} moves each voice across the stereo field: {}.",
                    e.label
                ),
                _ => format!("{src} moves the {}: {}.", r.dst, e.label),
            }
        }
    }
}

/// A module's name as a player reads it: "filter", "second oscillator", "envelope 2", "LFO 1", "reverb".
fn module_name(patch: &Patch, m: &enth_patch::ModuleInstance) -> String {
    let n =
        m.id.as_str()
            .trim_start_matches(|c: char| c.is_ascii_alphabetic());
    match m.kind {
        ModuleKind::Osc => {
            let i = patch
                .generators
                .iter()
                .filter(|g| g.kind == ModuleKind::Osc)
                .position(|g| g.id == m.id);
            match i {
                Some(0) => "first oscillator".to_owned(),
                Some(1) => "second oscillator".to_owned(),
                _ => format!("oscillator {n}"),
            }
        }
        ModuleKind::Noise => "noise".to_owned(),
        ModuleKind::Svf => "filter".to_owned(),
        ModuleKind::Adsr if m.id == patch.voice.amp_env => "amplitude envelope".to_owned(),
        ModuleKind::Adsr => format!("envelope {n}"),
        ModuleKind::Lfo => format!("LFO {n}"),
        ModuleKind::Reverb => "reverb".to_owned(),
        ModuleKind::Comp => "compressor".to_owned(),
    }
}

fn round_pct(d: f32) -> String {
    let s = format!("{d:.1}");
    s.trim_end_matches('0').trim_end_matches('.').to_owned()
}

// ---------------------------------------------------------------------------------------------------------------
// Patches
// ---------------------------------------------------------------------------------------------------------------

/// `patch` with its name and description set (an op: `SetMeta`), tags kept.
fn named(patch: &Patch, name: String, description: String) -> Patch {
    let mut p = patch.clone();
    let meta = PatchMeta {
        name,
        author: p.meta.author.clone(),
        description,
        tags: p.meta.tags.clone(),
    };
    let _ = apply(&mut p, Op::SetMeta { meta });
    p
}

fn patch_ref(path: &str) -> PatchRef {
    PatchRef::new(path).unwrap_or_else(|e| unreachable!("generated patch paths are valid: {e}"))
}

fn text_ref(name: &str) -> TextRef {
    TextRef::new(name).unwrap_or_else(|e| unreachable!("generated text names are valid: {e}"))
}

fn step_id(text: &str) -> StepId {
    StepId::new(text).unwrap_or_else(|e| unreachable!("generated step ids are valid: {e}"))
}

// ---------------------------------------------------------------------------------------------------------------
// Steps
// ---------------------------------------------------------------------------------------------------------------

/// The parts of step `k` that differ between its start and its target.
fn step_parts<'p>(plan: &'p Plan, k: usize, start: &Patch, target: &Patch) -> Vec<&'p Part> {
    let mut seen = BTreeSet::new();
    plan.steps[k]
        .elements
        .iter()
        .flat_map(|i| plan.elements[*i].parts.iter())
        .filter(|p| !p.same_in(start, target))
        .filter(|p| seen.insert(*p))
        .collect()
}

/// The selectors of the parts a step changes: a switched module as a whole, else each parameter and route (with the
/// route's destination).
fn editable(parts: &[&Part]) -> Vec<Selector> {
    let modules: BTreeSet<enth_patch::ModuleId> = parts
        .iter()
        .filter_map(|p| match p {
            Part::Enable(id) => Some(id.clone()),
            _ => None,
        })
        .collect();
    let mut out: Vec<Selector> = Vec::new();
    let push = |s: Selector, out: &mut Vec<Selector>| {
        if !out.contains(&s) {
            out.push(s);
        }
    };
    for p in parts {
        let covered =
            matches!(p, Part::Param(a) if a.module_id().is_some_and(|m| modules.contains(m)));
        if !covered {
            push(p.selector(), &mut out);
        }
        // A route edit also touches its destination (`v2::touches`), so the destination is editable too: the
        // witness touches only editable controls (lesson-check).
        if let Part::Route(r) = p
            && !r.dst.module_id().is_some_and(|m| modules.contains(m))
        {
            push(Selector::Param(r.dst.clone()), &mut out);
        }
    }
    out
}

/// What the step locks: every parameter and route the earlier steps taught, unless the step's own edits touch it
/// (a route into a locked cutoff would be refused).
fn locked(plan: &Plan, k: usize, start: &Patch, target: &Patch) -> Vec<Selector> {
    let ops = ops_between(start, target).unwrap_or_default();
    let mut scratch = start.clone();
    let mut touched = Vec::new();
    for op in &ops {
        touched.extend(touches(&scratch, op).unwrap_or_default());
        let _ = apply(&mut scratch, op);
    }
    let mut out: Vec<Selector> = Vec::new();
    for step in &plan.steps[..k] {
        for i in &step.elements {
            for p in &plan.elements[*i].parts {
                if matches!(p, Part::Enable(_)) {
                    continue;
                }
                let s = p.selector();
                if !touched.iter().any(|t| s.covers(t)) && !out.contains(&s) {
                    out.push(s);
                }
            }
        }
    }
    out
}

/// Modules switched off in the target that no part of the step touches: hidden.
fn hidden(target: &Patch, parts: &[&Part]) -> Vec<Selector> {
    target
        .modules()
        .filter(|(_, m)| !m.enabled)
        .filter(|(_, m)| {
            !parts.iter().any(|p| match p {
                Part::Enable(id) => id == &m.id,
                Part::Param(a) => a.module_id() == Some(&m.id),
                Part::Route(r) => r.dst.module_id() == Some(&m.id),
            })
        })
        .map(|(_, m)| Selector::Module(enth_patch::Target::Module(m.id.clone())))
        .collect()
}

/// How far either side of the target a loose predicate reaches, in the control's normalised travel (on its own
/// scale: a cutoff ×1.4, a 70 ms glide 39–116 ms, 5 points of a percentage).
const LOOSE_NORM: f32 = 0.05;

/// The step's patch predicates, loose: each changed part roughly as the target has it (a continuous control within
/// [`LOOSE_NORM`] of its travel, a semitone control within 1 st, a cents control within 10 ct, an enum, an integer or
/// a toggle exactly, a route at least half its depth with its sign, a switch), kept only when the start fails it and
/// the target passes. "Roughly right", because the step is rebuilt by ear.
fn predicates(parts: &[&Part], start: &Patch, target: &Patch, heard: &[Change]) -> Vec<Predicate> {
    let none = TouchLog::default();
    let keep = |p: &&&Part| match p {
        Part::Param(a) => target.spec(a).is_none_or(|spec| {
            !matches!(spec.kind, enth_params::Kind::Float { .. })
                || spec.perceptual.and_then(aspect_of).is_none_or(|asp| {
                    heard
                        .iter()
                        .any(|c| c.steps >= super::plan::MIN_CHANGE_STEPS && c.aspect == asp)
                })
        }),
        _ => true,
    };
    let heard_parts: Vec<&Part> = parts.iter().filter(keep).copied().collect();
    let all = loose(parts, start, target, &none);
    let some = loose(&heard_parts, start, target, &none);
    if some.is_empty() { all } else { some }
}

/// The aspect a parameter's main descriptor is heard as (`ParamSpec::perceptual`), when it has one.
fn aspect_of(perceptual: &str) -> Option<Aspect> {
    Some(match perceptual {
        "envelope.attack_ms" => Aspect::Attack,
        "envelope.decay_ms" => Aspect::Decay,
        "envelope.sustain_db" => Aspect::Sustain,
        "envelope.release_ms" => Aspect::Release,
        "pitch.f0_hz" => Aspect::Pitch,
        "detune.beating_hz" => Aspect::Beating,
        "spectrum.centroid_hz"
        | "spectrum.harmonics_db"
        | "spectrum.odd_even_ratio"
        | "spectrum.tilt_db_oct" => Aspect::Brightness,
        "noise_ratio" => Aspect::Noisiness,
        _ => return None,
    })
}

/// A loose bound as text: `x` rounded outward (down for a low bound, up for a high one) to two significant digits,
/// so the band still holds the target `v`, written as the display reads it back.
fn bound_text(spec: &enth_params::ParamSpec, x: f32, v: f32, up: bool) -> String {
    let r = spec.clamp(round_out(f64::from(x), 2, up) as f32);
    let r = if up { r.max(v) } else { r.min(v) };
    let shown = spec.display(r).to_string();
    if spec.parse(&shown).ok().map(f32::to_bits) == Some(r.to_bits()) {
        shown
    } else {
        format!("{r}")
    }
}

/// `x` rounded to `digits` significant digits, toward +∞ when `up`, else toward −∞.
fn round_out(x: f64, digits: i32, up: bool) -> f64 {
    if x == 0.0 || !x.is_finite() {
        return x;
    }
    let mag = enth_dsp::util::math::log10(x.abs()).floor() as i32;
    let scale = enth_dsp::util::math::pow(10.0, f64::from(digits - 1 - mag));
    let y = x * scale;
    (if up { y.ceil() } else { y.floor() }) / scale
}

fn loose(parts: &[&Part], start: &Patch, target: &Patch, none: &TouchLog) -> Vec<Predicate> {
    parts
        .iter()
        .filter_map(|p| loose_one(p, target))
        .filter(|p| p.holds(target, none) && !p.holds(start, none))
        .take(crate::check::MAX_ITEMS)
        .collect()
}

/// One part as `target` has it, loosely (see [`predicates`]); a cents or semitone control's band is at most half its
/// distance from 0, so a detune is told from none.
fn loose_one(p: &Part, target: &Patch) -> Option<Predicate> {
    match p {
        Part::Param(a) => {
            let spec = target.spec(a)?;
            let v = target.value(a)?;
            if let enth_params::Kind::Float { .. } = spec.kind {
                {
                    let (lo, hi) = match spec.unit {
                        enth_params::Unit::Semitones => {
                            (v - 1.0f32.min(0.5 * v.abs()), v + 1.0f32.min(0.5 * v.abs()))
                        }
                        enth_params::Unit::Cents => (
                            v - 10.0f32.min(0.5 * v.abs()),
                            v + 10.0f32.min(0.5 * v.abs()),
                        ),
                        _ => {
                            let n = spec.to_norm(v);
                            (
                                spec.from_norm((n - LOOSE_NORM).max(0.0)),
                                spec.from_norm((n + LOOSE_NORM).min(1.0)),
                            )
                        }
                    };
                    let (lo, hi) = (spec.clamp(lo.min(v)), spec.clamp(hi.max(v)));
                    Some(Predicate::Within {
                        knob: Knob::Param(a.clone()),
                        lo: Plain::new(bound_text(spec, lo, v, false)),
                        hi: Plain::new(bound_text(spec, hi, v, true)),
                    })
                }
            } else {
                {
                    let shown = spec.display(v).to_string();
                    let text = if spec.parse(&shown).ok().map(f32::to_bits) == Some(v.to_bits()) {
                        shown
                    } else {
                        format!("{v}")
                    };
                    Some(Predicate::Value {
                        knob: Knob::Param(a.clone()),
                        cmp: Cmp::Eq,
                        value: Plain::new(text),
                    })
                }
            }
        }
        Part::Route(r) => {
            let d = target
                .routes
                .iter()
                .find(|x| x.src == r.src && x.dst == r.dst)
                .map_or(0.0, |x| x.depth);
            Some(Predicate::Route {
                route: r.clone(),
                cmp: if d >= 0.0 { Cmp::Ge } else { Cmp::Le },
                depth: Plain::new(round_pct(0.5 * d)),
            })
        }
        Part::Enable(id) => Some(Predicate::Enabled {
            module: id.clone(),
            enabled: target.find(id).is_some_and(|(_, m)| m.enabled),
        }),
    }
}

/// The most context predicates a Do step carries.
const MAX_CONTEXT: usize = 4;

/// Context predicates: the parameters the earlier steps taught and this step locks, as the start has them. They hold
/// at the start and at the witness and cannot move (locked), so they change nothing for the player and no hint, but
/// they tell this step's goal apart from a neighbouring lesson's (`lesson-check --cross`): a filter envelope on Acid
/// Snap is not one on Glass Pluck.
fn context(plan: &Plan, k: usize, start: &Patch, target: &Patch) -> Vec<Predicate> {
    let none = TouchLog::default();
    locked(plan, k, start, target)
        .iter()
        .filter_map(|s| match s {
            Selector::Param(a) => loose_one(&Part::Param(a.clone()), start),
            _ => None,
        })
        .filter(|p| p.holds(start, &none) && p.holds(target, &none))
        .take(MAX_CONTEXT)
        .collect()
}

/// Decide how step `k` is taught from its measure (see [`ROUND_MAX_START`]); the renders a Do step's sound items
/// cost.
fn decide(
    plan: &Plan,
    k: usize,
    m: &StepMeasure,
    start: &Patch,
    target: &Patch,
    no_round: &BTreeSet<String>,
) -> Result<(Teach, u32), DeconstructError> {
    let parts = step_parts(plan, k, start, target);
    // A round only where closest can solve it: parameters, no switch and no route (closest moves parameters only),
    // and not one the caller found closest cannot pass.
    let structural = parts
        .iter()
        .any(|p| matches!(p, Part::Enable(_) | Part::Route(_)));
    let forced = plan.steps[k]
        .elements
        .iter()
        .any(|i| no_round.contains(&plan.elements[*i].id));
    if m.percent <= ROUND_MAX_START && !structural && !forced {
        return Ok((Teach::Round, 0));
    }
    let phrase = plan.steps[k].phrase;
    let (items, renders) = verify_items(
        &propose_items(&m.start, &m.target),
        phrase,
        start,
        target,
        MAX_ITEMS,
    )?;
    // What is heard is checked by ear, the sound items alone (a patch predicate beside them could still fail where
    // the measured hints stop: lesson-check's --solve); what no descriptor hears is checked as the parts, loosely.
    // Either way the context predicates tell the goal apart from a neighbour's.
    let ctx = context(plan, k, start, target);
    let teach = if items.is_empty() {
        let mut preds = predicates(&parts, start, target, &m.changes);
        if preds.is_empty() {
            return Ok((Teach::Merge, renders));
        }
        preds.extend(ctx);
        preds.truncate(crate::check::MAX_ITEMS);
        Teach::Check(Check::Patch { predicates: preds })
    } else {
        let sound = Check::Sound {
            phrase,
            expect: items,
            macros: Vec::new(),
        };
        // A step that switches a module on also checks its parts: no measured hint switches a module, so the hints
        // never reach the sound pass with them failing, and a neighbour's sound is told apart.
        let mut preds = if parts.iter().any(|p| matches!(p, Part::Enable(_))) {
            loose(&parts, start, target, &TouchLog::default())
        } else {
            Vec::new()
        };
        preds.extend(ctx);
        preds.truncate(crate::check::MAX_ITEMS);
        if preds.is_empty() {
            Teach::Check(sound)
        } else {
            Teach::Check(Check::All {
                checks: vec![Check::Patch { predicates: preds }, sound],
            })
        }
    };
    Ok((teach, renders))
}

/// The label of an element, the caller's when given.
fn label_of(opts: &Options, e: &Element) -> String {
    opts.labels
        .get(&e.id)
        .cloned()
        .unwrap_or_else(|| e.label.clone())
}

/// A step's label: its elements' labels in prose.
fn step_label(plan: &Plan, opts: &Options, k: usize) -> String {
    let labels: Vec<String> = plan.steps[k]
        .elements
        .iter()
        .map(|i| label_of(opts, &plan.elements[*i]))
        .collect();
    and_list(&labels)
}

/// The slug of step `k` for its ids and file names, unique in the lesson.
fn step_slug(plan: &Plan, k: usize, used: &mut BTreeSet<String>) -> String {
    let base = plan.elements[plan.steps[k].elements[0]].slug(24);
    let mut s = base.clone();
    let mut n = 2;
    while !used.insert(s.clone()) {
        s = format!("{base}-{n}");
        n += 1;
    }
    s
}

// ---------------------------------------------------------------------------------------------------------------
// Generation
// ---------------------------------------------------------------------------------------------------------------

/// The challenge of step `k`: a patch target, its start, only its parts editable, the earlier parts locked.
fn step_challenge(
    plan: &Plan,
    opts: &Options,
    k: usize,
    start: &Patch,
    target: &Patch,
) -> Challenge {
    let parts = step_parts(plan, k, start, target);
    let mut c = Challenge::new(
        Target::Patch(InlinePatch::new(target.clone())),
        Start::Patch {
            patch: InlinePatch::new(start.clone()),
        },
        plan.steps[k].phrase,
        scoring(),
    );
    c.constraints = Constraints {
        editable: editable(&parts),
        locked: locked(plan, k, start, target),
    };
    c.assists = Assists {
        live_meter: false,
        dim_unused: false,
        hint_budget: None,
    };
    c.meta = ChallengeMeta {
        title: format!("{}, step {}", opts.title, k + 1),
        tags: vec![opts.tag.clone(), opts.prefix.clone()],
    };
    c
}

/// Each step's measure and how it is taught, the merged steps' labels, and the renders spent.
type Settled = (Vec<(StepMeasure, Teach)>, Vec<String>, u32);

/// A variation goal: its witness, its sound items, its goal sentence, and the renders spent.
type Variation = (Patch, Vec<String>, String, u32);

/// Measure every step in its context and merge the ones not heard there, until none is left to merge. Returns each
/// step's measure and how it is taught, the merged steps' labels and the renders spent.
fn settle(plan: &mut Plan, opts: &Options) -> Result<Settled, DeconstructError> {
    type Key = (Vec<usize>, Vec<usize>);
    let mut cache: BTreeMap<Key, (StepMeasure, Teach)> = BTreeMap::new();
    let mut merged = Vec::new();
    let mut renders = 0;
    loop {
        let mut out = Vec::with_capacity(plan.steps.len());
        let mut merge_at = None;
        for k in 0..plan.steps.len() {
            let key: Key = (
                plan.included_before(k).into_iter().collect(),
                plan.steps[k].elements.clone(),
            );
            if !cache.contains_key(&key) {
                let (start, target) = (plan.step_start(k), plan.step_target(k));
                let c = step_challenge(plan, opts, k, &start, &target);
                let m = measure_step(&c, &start, &target)?;
                renders += STEP_RENDERS;
                let (teach, r) = decide(plan, k, &m, &start, &target, &opts.no_round)?;
                renders += r;
                cache.insert(key.clone(), (m, teach));
            }
            let entry = cache[&key].clone();
            if entry.1 == Teach::Merge && merge_at.is_none() && plan.steps.len() > 1 {
                merge_at = Some(k);
            }
            out.push(entry);
        }
        if let Some(k) = merge_at {
            merged.push(step_label(plan, opts, k));
            plan.merge(k);
        } else {
            // A lone step that is not heard in context is still taught, as a round.
            for e in &mut out {
                if e.1 == Teach::Merge {
                    e.1 = Teach::Round;
                }
            }
            return Ok((out, merged, renders));
        }
    }
}

/// The variation goal: a brighter version of the preset (its first filter an octave up, or down when it is already
/// open), checked by a brightness item and the items that keep its character, each verified against the preset and
/// the witness. `None` when the preset has no filter or nothing verifies.
fn variation(preset: &Patch) -> Result<Option<Variation>, DeconstructError> {
    let Some(f) = preset
        .voice_fx
        .iter()
        .find(|m| m.enabled && m.kind == ModuleKind::Svf)
    else {
        return Ok(None);
    };
    let Ok(addr) = ParamAddr::module(f.id.clone(), "cutoff") else {
        return Ok(None);
    };
    let (Some(spec), Some(c)) = (preset.spec(&addr), preset.value(&addr)) else {
        return Ok(None);
    };
    let brighter = c <= 5000.0;
    let v = spec.clamp(if brighter { c * 2.0 } else { c * 0.5 });
    let mut witness = preset.clone();
    let _ = apply(&mut witness, Op::SetParam { addr, value: v });
    let (from, to) = (describe(preset, HELD)?, describe(&witness, HELD)?);
    let mut renders = 2;
    let bright: Vec<String> = propose_items(&from, &to)
        .into_iter()
        .filter(|i| i.starts_with("brightness"))
        .collect();
    let (mut bright, r) = verify_items(&bright, HELD, preset, &witness, 1)?;
    renders += r;
    if bright.is_empty() {
        return Ok(None);
    }
    // And not too far: within half an octave past the witness, so "brighter" is this sound made brighter and not any
    // bright sound (a neighbouring lesson's witness, lesson-check's --cross).
    if let Some(b) = to.get(enth_analysis::Feature::Brightness) {
        let (op, edge) = if brighter {
            ('<', b * 1.41)
        } else {
            ('>', b / 1.41)
        };
        let item = format!("brightness{op}{}Hz", edge.round());
        if verify_both(&item, preset, &witness)? {
            bright.push(item);
        }
        renders += 6;
    }
    // What keeps its character: a plucky decay, a slow swell or a fast start, as the preset reads.
    let mut keep: Vec<(String, &'static str)> = Vec::new();
    if let Some(s) = from.get(enth_analysis::Feature::Sustain)
        && s <= -20.0
    {
        keep.push((format!("sustain<{}dB", (s + 8.0).round()), "plucky"));
    }
    if let Some(a) = from.get(enth_analysis::Feature::Attack) {
        if a >= 150.0 {
            keep.push((format!("attack>{}ms", (a / 2.5).round()), "slow to swell"));
        } else {
            keep.push((
                format!("attack<{}ms", (a * 2.0 + 30.0).round()),
                "quick to speak",
            ));
        }
    }
    let mut kept: Vec<(String, &'static str)> = Vec::new();
    for (item, word) in keep {
        // A keep item passes on both: check it as "the witness passes" against a start that passes too.
        let both = verify_both(&item, preset, &witness)?;
        renders += 6;
        if both {
            kept.push((item, word));
        }
    }
    let mut items = bright;
    items.extend(kept.iter().map(|(i, _)| i.clone()));
    let way = if brighter { "brighter" } else { "darker" };
    let goal = match kept.first() {
        Some((_, character)) => format!("Keep it {character}, and make it {way}."),
        None => format!("Make it {way}, and keep its envelope as it is."),
    };
    Ok(Some((witness, items, goal, renders)))
}

/// The variation goal's check: its sound items, and the amplitude envelope kept as the preset has it (loosely), so the
/// goal is this sound made brighter and not any bright sound with this brightness (lesson-check's --cross). The
/// envelope is never what the measured hints move toward a brightness item, so the predicates hold wherever they stop.
fn vary_check(preset: &Patch, witness: &Patch, items: Vec<String>) -> Check {
    let none = TouchLog::default();
    let amp = preset.voice.amp_env.clone();
    let keep: Vec<Predicate> = preset
        .find(&amp)
        .map(|(_, m)| {
            m.kind
                .params()
                .iter()
                .filter_map(|spec| ParamAddr::module(amp.clone(), spec.key).ok())
                .filter(|a| {
                    preset
                        .value(a)
                        .zip(preset.spec(a))
                        .is_some_and(|(v, s)| v.to_bits() != s.default.to_bits())
                })
                .filter_map(|a| loose_one(&Part::Param(a), preset))
                .filter(|p| p.holds(preset, &none) && p.holds(witness, &none))
                .take(MAX_CONTEXT)
                .collect()
        })
        .unwrap_or_default();
    let sound = Check::Sound {
        phrase: HELD,
        expect: items,
        macros: Vec::new(),
    };
    if keep.is_empty() {
        sound
    } else {
        Check::All {
            checks: vec![Check::Patch { predicates: keep }, sound],
        }
    }
}

/// Whether `item` passes on both patches, solidly (not thin, margin at least `MIN_MARGIN`).
fn verify_both(item: &str, a: &Patch, b: &Patch) -> Result<bool, DeconstructError> {
    let check = crate::check::SoundCheck {
        phrase: HELD,
        expect: vec![item.to_owned()],
        macros: Vec::new(),
    };
    let opts = crate::check::SoundOptions { nuisance: true };
    for p in [a, b] {
        let r = crate::check::evaluate_sounds_report(std::slice::from_ref(&check), p, opts)
            .map_err(DeconstructError::Sound)?;
        let res = &r[0].report.results[0];
        let solid = res.pass && !res.thin && !res.unstable;
        if !solid || res.margin.is_none_or(|m| m < super::measure::MIN_MARGIN) {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Generate the deconstruct lesson of `preset`: measure the ablations, plan, measure and settle the steps, then build
/// the lesson, its patches and its automatic text.
///
/// # Errors
/// [`DeconstructError`] when a render or a round fails, or when nothing in the preset is audible.
pub fn generate(preset: &Patch, opts: &Options) -> Result<Generated, DeconstructError> {
    let (ranks, ablation) = super::measure::measure_ablations(preset)?;
    let mut plan = super::plan::plan(preset, &ranks);
    if plan.steps.is_empty() {
        return Err(DeconstructError::Nothing {
            preset: preset_name(preset),
        });
    }
    let (teach, merged, steps_r) = settle(&mut plan, opts)?;
    let mut renders = Renders {
        ablation,
        steps: steps_r,
        ..Renders::default()
    };
    assemble(plan, opts, &teach, merged, &mut renders)
}

fn assemble(
    plan: Plan,
    opts: &Options,
    teach: &[(StepMeasure, Teach)],
    merged: Vec<String>,
    renders: &mut Renders,
) -> Result<Generated, DeconstructError> {
    let preset = &plan.preset;
    let name = preset_name(preset);
    let n = plan.steps.len();
    let mut patches: BTreeMap<String, Patch> = BTreeMap::new();
    let mut texts: BTreeMap<String, String> = BTreeMap::new();
    let mut notes: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut steps: Vec<Step> = Vec::new();
    let px = &opts.prefix;

    // Patch files: the preset, the start, each step's target (the last is the preset).
    let full_path = format!("patches/{px}.patch.json");
    patches.insert(
        full_path.clone(),
        named(preset, name.clone(), preset.meta.description.clone()),
    );
    let start_path = format!("patches/{px}-0.patch.json");
    let target_path = |k: usize| {
        if k + 1 == n {
            full_path.clone()
        } else {
            format!("patches/{px}-{}.patch.json", k + 1)
        }
    };
    let start_path_of = |k: usize| {
        if k == 0 {
            start_path.clone()
        } else {
            target_path(k - 1)
        }
    };
    let step_labels: Vec<String> = (0..n).map(|k| step_label(&plan, opts, k)).collect();
    patches.insert(
        start_path.clone(),
        named(
            &plan.start_patch(),
            format!("{name}: the start"),
            format!("Where the {name} lesson starts: {name} with every part it teaches taken out."),
        ),
    );
    for k in 0..n.saturating_sub(1) {
        patches.insert(
            target_path(k),
            named(
                &plan.step_target(k),
                format!("{name}: step {}", k + 1),
                format!(
                    "{name} rebuilt up to step {}: {}.",
                    k + 1,
                    and_list(&step_labels[..=k])
                ),
            ),
        );
    }

    // Intro.
    let numbered: Vec<String> = step_labels
        .iter()
        .enumerate()
        .map(|(i, l)| format!("{}. {}", i + 1, cap(l)))
        .collect();
    let intro = format!(
        "# Taking {name} apart\n\n{}\n\nThis lesson rebuilds it part by part, in the order the sound flows:\n\n{}\n\nEach step \
         plays the sound without and with one part, then you rebuild that part by ear.\n",
        if preset.meta.description.trim().is_empty() {
            format!("{name} is one of your sounds.")
        } else {
            preset.meta.description.trim().to_owned()
        },
        numbered.join("\n"),
    );
    texts.insert("intro.md".into(), intro);
    steps.push(Step {
        id: step_id("intro"),
        text: Some(text_ref("intro.md")),
        view: StepView::default(),
        hints: Vec::new(),
        kind: StepKind::Read,
    });
    texts.insert(
        "hear.md".into(),
        format!(
            "# The whole sound, and where we start\n\nFirst {name} as it is, then the plain sound this lesson starts from. \
             What does {name} have that the start lacks?\n"
        ),
    );
    steps.push(Step {
        id: step_id("hear"),
        text: Some(text_ref("hear.md")),
        view: StepView::default(),
        hints: Vec::new(),
        kind: StepKind::Listen {
            examples: vec![
                example("The whole sound", &full_path, HELD),
                example("Where we start", &start_path, HELD),
            ],
        },
    });

    // The steps.
    let mut used = BTreeSet::new();
    let mut step_reports = Vec::new();
    let mut recap_lines = Vec::new();
    for k in 0..n {
        let (m, how) = &teach[k];
        let (start, target) = (plan.step_start(k), plan.step_target(k));
        let parts = step_parts(&plan, k, &start, &target);
        let phrase = plan.steps[k].phrase;
        let s = step_slug(&plan, k, &mut used);
        let label = step_labels[k].clone();
        let first = &plan.elements[plan.steps[k].elements[0]];
        let ablation: Option<&Rank> = plan.ranks.get(&first.id);
        let mut changes: Vec<Change> = m.changes.clone();
        if changes.is_empty()
            && let Some(r) = ablation
        {
            changes.clone_from(&r.changes);
        }
        let involved: Vec<Change> = changes
            .iter()
            .filter(|c| {
                plan.steps[k]
                    .elements
                    .iter()
                    .any(|i| involves(&target, &plan.elements[*i], c.aspect))
            })
            .cloned()
            .collect();
        // The voice settings are told by their template: their descriptor changes (a median pitch, an attack read
        // across two notes) are the measurement's, not what a listener names.
        let voice = plan.steps[k]
            .elements
            .iter()
            .any(|i| plan.elements[*i].flow == super::element::Flow::Voice);
        let effect = if voice {
            None
        } else {
            effect_sentence(if involved.is_empty() {
                &changes
            } else {
                &involved
            })
        };
        let what: Vec<String> = plan.steps[k]
            .elements
            .iter()
            .map(|i| what_it_is(&target, &plan.elements[*i]))
            .collect();
        let hear_id = format!("hear-{s}");
        let build_id = format!("build-{s}");
        texts.insert(
            format!("{hear_id}.md"),
            format!(
                "# {}\n\n{} {}\n\nListen to **Without** and **With**.{}\n",
                cap(&label),
                what.join(" "),
                effect
                    .clone()
                    .unwrap_or_else(|| "The change is small: listen closely.".to_owned()),
                phrase_note(phrase)
            ),
        );
        steps.push(Step {
            id: step_id(&hear_id),
            text: Some(text_ref(&format!("{hear_id}.md"))),
            view: StepView::default(),
            hints: Vec::new(),
            kind: StepKind::Listen {
                examples: vec![
                    example("Without", &start_path_of(k), phrase),
                    example("With", &target_path(k), phrase),
                ],
            },
        });
        let ed = editable(&parts);
        let links: Vec<String> = ed.iter().map(|sel| control_link(&target, sel)).collect();
        let desc = first
            .main
            .as_ref()
            .filter(|_| !first.parts.iter().any(|p| matches!(p, Part::Route(_))))
            .and_then(|a| target.spec(a))
            .map(|s| cap(s.learn.desc))
            .filter(|d| !d.is_empty());
        let hints = opts.hints.get(&first.id).cloned().unwrap_or_else(|| {
            let mut h = Vec::new();
            if let Some(c) = changes.first() {
                h.push(cue(c).to_owned());
            }
            h.push(format!(
                "In this step you change only {}.",
                and_list(&links_plain(&target, &ed))
            ));
            h
        });
        let view_common = StepView {
            highlight: ed.clone(),
            hide: hidden(&target, &parts),
            ..StepView::default()
        };
        let (kind, view, body) = match how {
            Teach::Check(check) => (
                StepKind::Do {
                    check: check.clone(),
                    witness: patch_ref(&target_path(k)),
                },
                StepView {
                    layout: Layout::Full,
                    start: Some(patch_ref(&start_path_of(k))),
                    editable: ed.clone(),
                    locked: locked(&plan, k, &start, &target),
                    ..view_common
                },
                format!(
                    "Set {} so your sound matches **With**. The step passes once it does.",
                    and_list(&links)
                ),
            ),
            Teach::Round | Teach::Merge => (
                StepKind::Recreate {
                    challenge: Box::new(step_challenge(
                        &plan,
                        opts,
                        k,
                        &patches[&start_path_of(k)],
                        &patches[&target_path(k)],
                    )),
                    witness: None,
                },
                view_common,
                format!(
                    "Match the target with {}. Press **Target** and **Mine** to compare, then **Submit**.",
                    and_list(&links)
                ),
            ),
        };
        texts.insert(
            format!("{build_id}.md"),
            format!(
                "# Rebuild {label}\n\n{}{body}\n",
                desc.map_or_else(String::new, |d| format!("{d}\n\n"))
            ),
        );
        let items: Vec<String> = match how {
            Teach::Check(c) => {
                let mut v: Vec<String> = c.sounds().into_iter().flat_map(|s| s.expect).collect();
                v.extend(check_predicates(c).iter().map(ToString::to_string));
                v
            }
            _ => Vec::new(),
        };
        notes.insert(
            hear_id.clone(),
            vec![format!(
                "Step {} of {n}: {} ({}), phrase {}. sound/1 of the start against the target: {:.1} % ({:.2} JND); \
                 taught as a {}.",
                k + 1,
                label,
                plan.steps[k].elements.iter().map(|i| plan.elements[*i].id.as_str()).collect::<Vec<_>>().join(" + "),
                phrase.0,
                m.percent,
                m.jnd,
                how.name()
            ), format!(
                "Changes between start and target: {}.",
                if changes.is_empty() { "none read".to_owned() } else { changes.iter().map(Change::words).collect::<Vec<_>>().join(", ") }
            )],
        );
        step_reports.push(StepReport {
            id: build_id.clone(),
            elements: plan.steps[k]
                .elements
                .iter()
                .map(|i| plan.elements[*i].id.clone())
                .collect(),
            phrase: phrase.0,
            teach: how.name(),
            start_percent: m.percent,
            start_jnd: m.jnd,
            changes: changes.iter().map(Change::words).collect(),
            items,
        });
        steps.push(Step {
            id: step_id(&build_id),
            text: Some(text_ref(&format!("{build_id}.md"))),
            view,
            hints,
            kind,
        });
        recap_lines.push(format!(
            "{}. **{}.** {}",
            k + 1,
            cap(&label),
            what.join(" ")
        ));
    }

    // What disappeared?
    let mut missing_report = None;
    if opts.choice {
        let mut cands: Vec<usize> = plan
            .steps
            .iter()
            .flat_map(|s| s.elements.iter().copied())
            .filter(|i| {
                plan.ranks
                    .get(&plan.elements[*i].id)
                    .is_some_and(|r| r.phrase == Some(HELD) && r.audible())
            })
            .collect();
        cands.sort_by(|a, b| {
            let c = |i: &usize| {
                plan.ranks
                    .get(&plan.elements[*i].id)
                    .map_or(0.0, super::plan::contribution_of)
            };
            c(b).total_cmp(&c(a)).then(a.cmp(b))
        });
        cands.truncate(MAX_CANDIDATES);
        if cands.len() >= 2 {
            let list: Vec<(String, Patch)> = cands
                .iter()
                .map(|i| {
                    (
                        format!("Without {}", label_of(opts, &plan.elements[*i])),
                        plan.without(*i),
                    )
                })
                .collect();
            let (chosen, closest, r) = pick_options(&list, HELD, MAX_OPTIONS)?;
            renders.choice += r;
            if chosen.len() >= 2 {
                // In teaching order.
                let mut picked: Vec<usize> = chosen.iter().map(|c| cands[*c]).collect();
                let order: Vec<usize> = plan
                    .steps
                    .iter()
                    .flat_map(|s| s.elements.iter().copied())
                    .collect();
                picked.sort_by_key(|i| order.iter().position(|j| j == i));
                let answer =
                    (enth_dsp::util::rng::fnv1a64(name.as_bytes()) % picked.len() as u64) as usize;
                let mut options = Vec::new();
                for i in &picked {
                    let e = &plan.elements[*i];
                    let path = format!("patches/{px}-without-{}.patch.json", e.slug(24));
                    patches.insert(
                        path.clone(),
                        named(
                            &plan.without(*i),
                            format!("{name} without {}", label_of(opts, e)),
                            format!("{name} with {} taken out.", label_of(opts, e)),
                        ),
                    );
                    options.push(example(
                        &format!("Without {}", label_of(opts, e)),
                        &path,
                        HELD,
                    ));
                }
                let mystery = options[answer].patch.as_str().to_owned();
                texts.insert(
                    "missing-hear.md".into(),
                    format!("# What disappeared?\n\nThe whole sound, then {name} with one part taken out. Which part is missing?\n"),
                );
                steps.push(Step {
                    id: step_id("missing-hear"),
                    text: Some(text_ref("missing-hear.md")),
                    view: StepView::default(),
                    hints: Vec::new(),
                    kind: StepKind::Listen {
                        examples: vec![
                            example("The whole sound", &full_path, HELD),
                            example("Something missing", &mystery, HELD),
                        ],
                    },
                });
                texts.insert(
                    "missing.md".into(),
                    format!(
                        "# Which part was missing?\n\nEach option plays {name} without one part. Which one matches \
                         **Something missing**?\n"
                    ),
                );
                steps.push(Step {
                    id: step_id("missing"),
                    text: Some(text_ref("missing.md")),
                    view: StepView::default(),
                    hints: vec![
                        "Play **Something missing** again, then compare it with each option."
                            .to_owned(),
                    ],
                    kind: StepKind::Choose {
                        options: options.clone(),
                        answer: answer as u8,
                    },
                });
                notes.insert(
                    "missing-hear".into(),
                    vec![format!(
                        "What disappeared: {} options, pairwise at least {:.2} JND apart (sound/1, both directions); \
                         the answer is option {} ({}).",
                        options.len(),
                        closest,
                        answer + 1,
                        options[answer].label
                    )],
                );
                missing_report = Some((
                    options.iter().map(|o| o.label.clone()).collect(),
                    answer,
                    closest,
                ));
            }
        }
    }

    // The variation goal.
    let mut variation_report = None;
    if opts.variation
        && let Some((witness, items, goal, r)) = variation(preset)?
    {
        renders.variation += r;
        let path = format!("patches/{px}-variation.patch.json");
        patches.insert(
            path.clone(),
            named(
                &witness,
                format!("{name}, varied"),
                format!("{name} with its filter moved: one answer to the variation goal."),
            ),
        );
        let cutoff_link = preset
            .voice_fx
            .iter()
            .find(|m| m.enabled && m.kind == ModuleKind::Svf)
            .map(|m| format!("[cutoff](enth:{}.cutoff)", m.id));
        texts.insert(
            "vary.md".into(),
            format!(
                "# Make it your own\n\n{goal} You start from the whole sound, and every control is yours. The check \
                 listens to one held note.\n"
            ),
        );
        notes.insert(
            "vary".into(),
            vec![format!(
                "Variation: {} (verified: the preset fails the first item, the witness passes every item, no thin item, \
                 margins at least 1e-3).",
                items.join(", ")
            )],
        );
        steps.push(Step {
            id: step_id("vary"),
            text: Some(text_ref("vary.md")),
            view: StepView {
                start: Some(patch_ref(&full_path)),
                ..StepView::default()
            },
            hints: match cutoff_link {
                Some(_) => vec![
                    "The filter's cutoff moves the brightness most; leave the envelopes alone."
                        .to_owned(),
                ],
                None => Vec::new(),
            },
            kind: StepKind::Do {
                check: vary_check(preset, &witness, items.clone()),
                witness: patch_ref(&path),
            },
        });
        variation_report = Some(items);
    }

    // Recap.
    let fine: Vec<String> = plan
        .fine
        .iter()
        .filter(|(_, w)| {
            matches!(
                w,
                FineWhy::BelowJnd | FineWhy::OverCap | FineWhy::Unobservable
            )
        })
        .map(|(i, _)| label_of(opts, &plan.elements[*i]))
        .collect();
    let mut recap = format!("# What made {name}\n\n{}\n", recap_lines.join("\n"));
    if crate::markdown::parse(&recap)
        .doc
        .plain()
        .split_whitespace()
        .count()
        + 3 * fine.len()
        + 20
        > crate::markdown::STEP_WORD_BUDGET
    {
        // Too long for one step: the parts by name only.
        let short: Vec<String> = step_labels
            .iter()
            .enumerate()
            .map(|(i, l)| format!("{}. {}", i + 1, cap(l)))
            .collect();
        recap = format!("# What made {name}\n\n{}\n", short.join("\n"));
    }
    if !fine.is_empty() {
        let _ = write!(
            recap,
            "\nAlso there from the start, too small to hear on a held note: {}.\n",
            and_list(&fine)
        );
    }
    texts.insert("recap.md".into(), recap);
    steps.push(Step {
        id: step_id("recap"),
        text: Some(text_ref("recap.md")),
        view: StepView::default(),
        hints: Vec::new(),
        kind: StepKind::Read,
    });

    // The lesson.
    let mut teaches: Vec<ParamKey> = Vec::new();
    for step in &plan.steps {
        for i in &step.elements {
            for p in &plan.elements[*i].parts {
                let addr = match p {
                    Part::Param(a) => a,
                    Part::Route(r) => &r.dst,
                    Part::Enable(_) => continue,
                };
                let kind = match addr.target() {
                    enth_patch::Target::Module(id) => {
                        preset.find(id).map(|(_, m)| m.kind.key().to_owned())
                    }
                    enth_patch::Target::Voice => Some("voice".to_owned()),
                    enth_patch::Target::Master => Some("master".to_owned()),
                };
                if let Some(key) =
                    kind.and_then(|k| ParamKey::new(&format!("{k}.{}", addr.param())).ok())
                    && !teaches.contains(&key)
                {
                    teaches.push(key);
                }
            }
        }
    }
    let lesson = Lesson {
        id: opts.lesson.clone(),
        title: opts.title.clone(),
        tier: opts.tier,
        objective: opts.objective.clone().unwrap_or_else(|| {
            format!("Hear what each part of {name} does, and rebuild it part by part, by ear.")
        }),
        teaches,
        steps,
    };
    lesson.validate().map_err(DeconstructError::Lesson)?;

    let report = Report {
        preset: name,
        elements: plan
            .elements
            .iter()
            .enumerate()
            .map(|(i, e)| {
                let r = plan.ranks.get(&e.id);
                let fate = plan
                    .steps
                    .iter()
                    .position(|s| s.elements.contains(&i))
                    .map(|k| format!("step {}", k + 1))
                    .or_else(|| {
                        plan.fine
                            .iter()
                            .find(|(j, _)| *j == i)
                            .map(|(_, w)| format!("fine: {}", w.name()))
                    })
                    .unwrap_or_default();
                ElementReport {
                    id: e.id.clone(),
                    flow: e.flow.name(),
                    label: label_of(opts, e),
                    phrase: r.and_then(|r| r.phrase).map(|p| p.0),
                    jnd: r.map_or(0.0, |r| r.jnd),
                    percent: r.map_or(100.0, |r| r.percent),
                    changes: r
                        .map_or_else(Vec::new, |r| r.changes.iter().map(Change::words).collect()),
                    fate,
                }
            })
            .collect(),
        steps: step_reports,
        missing: missing_report,
        variation: variation_report,
        merged,
        renders: *renders,
    };
    Ok(Generated {
        plan,
        lesson,
        patches,
        texts,
        notes,
        report,
    })
}

fn links_plain(patch: &Patch, sels: &[Selector]) -> Vec<String> {
    sels.iter()
        .map(|s| {
            let l = control_link(patch, s);
            // "[name](enth:x)" → "name"
            let mut out = String::new();
            let mut skip = false;
            for c in l.chars() {
                match c {
                    '[' => {}
                    ']' => skip = true,
                    ')' if skip => skip = false,
                    _ if skip => {}
                    _ => out.push(c),
                }
            }
            out
        })
        .collect()
}

fn check_predicates(c: &Check) -> Vec<&Predicate> {
    match c {
        Check::Patch { predicates } => predicates.iter().collect(),
        Check::All { checks } => checks.iter().flat_map(check_predicates).collect(),
        Check::Sound { .. } => Vec::new(),
    }
}

fn example(label: &str, path: &str, phrase: PhraseId) -> Example {
    Example {
        label: label.to_owned(),
        patch: patch_ref(path),
        phrase,
        transpose: 0,
        level_match: true,
        seed: None,
        curve: None,
    }
}
