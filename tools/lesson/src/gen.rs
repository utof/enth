//! **Owner: lane DEC** (deconstruct and studies, wave 4). Pre-declared by lane S0l (R2).
//!
//! `gen deconstruct <preset>` (`cargo xtask lesson-gen deconstruct <preset>`): writes a "deconstruct this preset"
//! lesson by ablation (docs/learn.md, "Deconstruct lessons (lane DEC)"). It renders the preset without each element
//! and takes the `sound/1` distance and the coach's descriptor rows (`enth_learn::deconstruct::measure`), plans with
//! `enth_learn::deconstruct::plan` on those ranks, measures each step in its context, and writes the lesson in the
//! frozen schema in the authoring layout: `lessons/<id>/lesson.toml` (with what was measured, and the ablation cost
//! in renders, as comments), its texts and its patches. The wall time goes to stdout only, so a rerun writes the
//! same bytes.
//!
//! **Text is never overwritten**: an existing `.md` file is kept (`--force-text` rewrites them), so the human edits
//! of the factory studies survive a regeneration; a text the lesson no longer names is removed.
//!
//! `gen studies` regenerates the six factory studies of `content/lessons/studies/` (PLAN.md U9) from the preset
//! candidates, with their titles, names and hints from the table below, and removes the pack's patches no study names.
//! The plugin's worker runs the same generator on a user's presets (`enth_learn::deconstruct::auto_lesson`).
#![allow(
    clippy::disallowed_methods,
    reason = "a command-line tool: it reads presets, writes lessons and times the ablation"
)]

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use enth_learn::Lesson;
use enth_learn::deconstruct::build::{Generated, Options, generate};
use enth_learn::deconstruct::toml_out::lesson_toml;
use enth_learn::lesson::Tier;
use enth_learn::pack::LessonId;

const USAGE: &str = "usage: enth-lesson-tool gen deconstruct <preset.patch.json> [--pack DIR] [--id ID] [--title TITLE]
                          [--tag TAG] [--no-choice] [--no-variation] [--force-text] [--dry-run] [--report FILE]
       enth-lesson-tool gen studies [--force-text] [--only ID] [--report FILE]
Writes a deconstruct lesson by ablation into DIR/lessons/<id>/ (default: content/lessons/studies). Existing texts are
kept unless --force-text.";

/// One factory study (content/lessons/PLAN.md, U9): the candidate it takes apart, its names, and the hints and part
/// names a person wrote (the generator's own are the fallback).
struct Study {
    id: &'static str,
    preset: &'static str,
    title: &'static str,
    objective: &'static str,
    labels: &'static [(&'static str, &'static str)],
    hints: &'static [(&'static str, &'static [&'static str])],
}

/// The six studies, in PLAN.md's order.
const STUDIES: [Study; 6] = [
    Study {
        id: "bass",
        preset: "content/presets-candidates/bass/acid-snap.patch.json",
        title: "Bass: Acid Snap",
        objective: "Take a resonant acid bass apart and rebuild its filter, its snap and its slide by ear.",
        labels: &[
            ("filter1", "the filter"),
            ("filter1.keytrack", "key tracking"),
            ("filter1.drive", "the drive"),
            ("env1", "the amplitude envelope"),
            ("env2 -> filter1.cutoff", "the snap"),
            ("voice", "the slide"),
        ],
        hints: &[
            (
                "filter1",
                &[
                    "Listen to the brightness first, then to the ring at the cutoff: the target is dark and nasal.",
                    "The cutoff sets how dark it is; the resonance sets how much it rings.",
                ],
            ),
            (
                "filter1.keytrack",
                &[
                    "Two notes an octave apart: listen to whether the high note opens up.",
                    "Key tracking makes the cutoff follow the note you play.",
                ],
            ),
            (
                "filter1.drive",
                &[
                    "The drive is subtle: listen for a slightly thicker, rougher tone, not for brightness.",
                ],
            ),
            (
                "env1",
                &[
                    "Listen to the end of each note: does it stop short or hang on?",
                    "The release sets how fast a note dies once the key lets go.",
                ],
            ),
            (
                "env2 -> filter1.cutoff",
                &[
                    "Listen to the start of each note: a quick wah that closes at once.",
                    "Envelope 2 drives the cutoff; with its sustain at 0 the filter closes again after the snap.",
                ],
            ),
            (
                "voice",
                &[
                    "The second note starts before the first lets go: does it glide there, or start again?",
                    "Legato keeps one voice playing; the glide time sets how long the slide takes.",
                ],
            ),
        ],
    },
    Study {
        id: "plucks",
        preset: "content/presets-candidates/pluck/glass-pluck.patch.json",
        title: "Plucks: Glass Pluck",
        objective: "Take a bright pluck apart and rebuild its second oscillator, its filter envelope and its short decay by ear.",
        labels: &[
            ("osc2", "the octave pulse"),
            ("env1", "the short decay"),
            ("env2 -> filter1.cutoff", "the filter envelope"),
            ("reverb1", "the room"),
        ],
        hints: &[
            (
                "osc2",
                &[
                    "Listen above the saw: a thinner, hollow tone an octave higher, much quieter.",
                    "Switch the second oscillator on, pick the pulse wave, tune it up an octave and keep it quiet.",
                ],
            ),
            (
                "env1",
                &[
                    "Listen to how long each note rings before it is gone.",
                    "A sustain of 0 lets the note die away; the decay sets how fast.",
                ],
            ),
            (
                "env2 -> filter1.cutoff",
                &[
                    "Listen to the first instant of each note: it starts bright and dulls quickly.",
                    "Route envelope 2 to the cutoff, so each note opens the filter and lets it close.",
                ],
            ),
            (
                "reverb1",
                &[
                    "Listen after each note: a short, quiet room follows it.",
                    "Switch the reverb on and keep its wet level low.",
                ],
            ),
        ],
    },
    Study {
        id: "pads",
        preset: "content/presets-candidates/pad/velvet-pad.patch.json",
        title: "Pads: Velvet Pad",
        objective: "Take a warm pad apart and rebuild its detune, its slow swell and its hall by ear.",
        labels: &[
            ("osc2", "the second saw"),
            ("osc1", "the detune"),
            ("filter1", "the filter"),
            ("env1", "the slow swell"),
            ("reverb1", "the hall"),
        ],
        hints: &[
            (
                "osc2",
                &[
                    "Listen for a slow beat, like two strings slightly out of tune.",
                    "Switch the second oscillator on and tune it a few cents sharp.",
                ],
            ),
            (
                "osc1",
                &[
                    "The beat gets faster when the two saws move further apart.",
                    "Tune the first oscillator a few cents flat, as far as the second is sharp.",
                ],
            ),
            (
                "filter1",
                &[
                    "Listen to the brightness: the target is warm, with the top taken off.",
                    "Lower the cutoff until the saws lose their edge.",
                ],
            ),
            (
                "env1",
                &[
                    "Listen to how the note fades in, and how long it hangs on after the key lets go.",
                    "A long attack makes the swell; a long release makes the tail.",
                ],
            ),
            (
                "reverb1",
                &[
                    "Listen after the note: a long, soft hall carries it on.",
                    "Switch the reverb on with a long decay, and keep the wet level moderate.",
                ],
            ),
        ],
    },
    Study {
        id: "leads",
        preset: "content/presets-candidates/lead/breath-flute.patch.json",
        title: "Leads: Breath Flute",
        objective: "Take a flute-like lead apart and rebuild its breath, its swell, its vibrato and its slide by ear.",
        labels: &[
            ("noise1", "the breath"),
            ("osc1", "the triangle"),
            ("filter1", "the filter"),
            ("env1", "the swell"),
            ("lfo1 -> osc1.fine", "the vibrato"),
            ("voice", "the slide"),
            ("reverb1", "the hall"),
        ],
        hints: &[
            (
                "noise1",
                &[
                    "Listen for air around the tone, like breath across a flute.",
                    "Switch the noise on and mix it in fairly high: the filter keeps it soft.",
                ],
            ),
            (
                "osc1",
                &[
                    "Listen to the tone under the breath: is it buzzy, or soft and hollow?",
                    "A triangle wave has far fewer harmonics than a saw.",
                ],
            ),
            (
                "filter1",
                &[
                    "Listen to the top of the breath: how much hiss is left?",
                    "The cutoff trims the breath noise; keep the resonance low.",
                ],
            ),
            (
                "env1",
                &[
                    "Listen to the start of each note: does it speak at once, or swell in like a breath?",
                    "The attack sets the swell; the release how long the note lingers.",
                ],
            ),
            (
                "lfo1 -> osc1.fine",
                &[
                    "Listen to the pitch while the note holds: a gentle, regular waver.",
                    "Route an LFO at about five cycles a second to the oscillator's fine tune, and keep it shallow.",
                ],
            ),
            (
                "voice",
                &[
                    "The second note starts before the first lets go: does it slide there, or start again?",
                    "Legato keeps one voice playing; a short glide makes the slide.",
                ],
            ),
            (
                "reverb1",
                &[
                    "Listen after each note: a soft hall follows it.",
                    "Switch the reverb on and keep its wet level low.",
                ],
            ),
        ],
    },
    Study {
        id: "percussion",
        preset: "content/presets-candidates/perc/thump-kick.patch.json",
        title: "Percussion: Thump Kick",
        objective: "Take a synth kick apart and rebuild its pitch drop, its click and its short body by ear.",
        labels: &[
            ("osc1", "the sine"),
            ("env1", "the body"),
            ("env2 -> osc1.semi", "the pitch drop"),
            ("reverb1", "the room"),
        ],
        hints: &[
            (
                "osc1",
                &[
                    "Listen to the tone: a pure, round sine, not a buzzy saw.",
                    "Pick the sine wave, and make every note start at the same point of the wave.",
                ],
            ),
            (
                "env1",
                &[
                    "Listen to how long the thump lasts: a short body, then nothing.",
                    "With the sustain at 0 the decay alone sets how long it lasts.",
                ],
            ),
            (
                "env2 -> osc1.semi",
                &[
                    "Listen to the very start: the pitch falls from high to low in a flash.",
                    "Route envelope 2 to the oscillator's pitch with a large depth and a very short decay.",
                ],
            ),
            (
                "reverb1",
                &[
                    "Listen after the hit: a faint, short room.",
                    "Switch the reverb on and keep it very quiet.",
                ],
            ),
        ],
    },
    Study {
        id: "textures",
        preset: "content/presets-candidates/texture/wind-tunnel.patch.json",
        title: "Textures: Wind Tunnel",
        objective: "Take a wind texture apart and rebuild its band-pass, its slow sweep and its swell by ear.",
        labels: &[
            ("filter1", "the band-pass"),
            ("env1", "the swell"),
            ("lfo1 -> filter1.cutoff", "the sweep"),
            ("reverb1", "the hall"),
        ],
        hints: &[
            (
                "filter1",
                &[
                    "Listen to which part of the noise is left: a band in the middle, with a whistle in it.",
                    "Switch the filter to band-pass, set the band and add some resonance.",
                ],
            ),
            (
                "env1",
                &[
                    "Listen to how slowly the wind rises and how long it fades.",
                    "A long attack and a long release make the gust.",
                ],
            ),
            (
                "lfo1 -> filter1.cutoff",
                &[
                    "Listen to the band while it holds: does it drift up and down slowly?",
                    "Route an LFO slower than once a second to the cutoff.",
                ],
            ),
            (
                "reverb1",
                &[
                    "Listen after the note: a long hall carries the wind on.",
                    "Switch the reverb on with a long decay.",
                ],
            ),
        ],
    },
];

/// `gen`: `deconstruct` or `studies`.
pub fn run(args: &[String]) -> Result<(), String> {
    match args.first().map(String::as_str) {
        Some("deconstruct") => deconstruct(&args[1..]),
        Some("studies") => studies(&args[1..]),
        _ => Err(USAGE.to_owned()),
    }
}

/// The repository root: the nearest ancestor of the working directory with `xtask/layers.toml`.
fn repo_root() -> Result<PathBuf, String> {
    let mut dir =
        std::env::current_dir().map_err(|e| format!("cannot read the working directory: {e}"))?;
    loop {
        if dir.join("xtask/layers.toml").is_file() {
            return Ok(dir);
        }
        if !dir.pop() {
            return Err(
                "run it inside the enth repository (no xtask/layers.toml above here)".to_owned(),
            );
        }
    }
}

fn load_preset(path: &Path) -> Result<enth_patch::Patch, String> {
    let text = fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    enth_patch::file::load(&text)
        .map(|l| l.patch)
        .map_err(|e| format!("{}: not a patch: {e:?}", path.display()))
}

struct Flags {
    force_text: bool,
    dry_run: bool,
    report: Option<PathBuf>,
}

fn deconstruct(args: &[String]) -> Result<(), String> {
    let mut preset = None;
    let mut pack = None;
    let mut id = None;
    let mut title = None;
    let mut tag = None;
    let mut choice = true;
    let mut variation = true;
    let mut flags = Flags {
        force_text: false,
        dry_run: false,
        report: None,
    };
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut value = |name: &str| {
            it.next()
                .cloned()
                .ok_or_else(|| format!("{name} needs a value\n{USAGE}"))
        };
        match a.as_str() {
            "--pack" => pack = Some(PathBuf::from(value("--pack")?)),
            "--id" => id = Some(value("--id")?),
            "--title" => title = Some(value("--title")?),
            "--tag" => tag = Some(value("--tag")?),
            "--report" => flags.report = Some(PathBuf::from(value("--report")?)),
            "--no-choice" => choice = false,
            "--no-variation" => variation = false,
            "--force-text" => flags.force_text = true,
            "--dry-run" => flags.dry_run = true,
            s if s.starts_with("--") => return Err(format!("unknown option {s}\n{USAGE}")),
            s => preset = Some(PathBuf::from(s)),
        }
    }
    let preset_path = preset.ok_or_else(|| USAGE.to_owned())?;
    let patch = load_preset(&preset_path)?;
    let mut opts = Options::for_preset(&patch);
    if let Some(id) = id {
        opts.lesson = LessonId::new(&id)?;
        opts.prefix = id;
    }
    if let Some(t) = title {
        opts.title = t;
    }
    if let Some(t) = tag {
        opts.tag = t;
    }
    opts.choice = choice;
    opts.variation = variation;
    let pack = match pack {
        Some(p) => p,
        None => repo_root()?.join("content/lessons/studies"),
    };
    let reports = vec![one(
        &patch,
        &preset_path.display().to_string(),
        &opts,
        &pack,
        &flags,
    )?];
    write_report(&flags, &reports)
}

fn studies(args: &[String]) -> Result<(), String> {
    let mut flags = Flags {
        force_text: false,
        dry_run: false,
        report: None,
    };
    let mut only = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--force-text" => flags.force_text = true,
            "--dry-run" => flags.dry_run = true,
            "--only" => only = it.next().cloned(),
            "--report" => flags.report = it.next().map(PathBuf::from),
            s => return Err(format!("unknown argument {s}\n{USAGE}")),
        }
    }
    let root = repo_root()?;
    let pack = root.join("content/lessons/studies");
    let mut reports = Vec::new();
    for s in STUDIES
        .iter()
        .filter(|s| only.as_deref().is_none_or(|o| o == s.id))
    {
        let patch = load_preset(&root.join(s.preset))?;
        let mut opts = Options::for_preset(&patch);
        opts.lesson = LessonId::new(s.id)?;
        s.id.clone_into(&mut opts.prefix);
        s.title.clone_into(&mut opts.title);
        opts.objective = Some(s.objective.to_owned());
        opts.tier = Tier::Intermediate;
        "studies".clone_into(&mut opts.tag);
        opts.labels = s
            .labels
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        opts.hints = s
            .hints
            .iter()
            .map(|(k, v)| ((*k).to_owned(), v.iter().map(|h| (*h).to_owned()).collect()))
            .collect();
        reports.push(one(&patch, s.preset, &opts, &pack, &flags)?);
    }
    if only.is_none() && !flags.dry_run {
        prune_patches(&pack)?;
    }
    write_report(&flags, &reports)
}

/// lesson-check's `closest` budget for a Recreate round (`--budget`, default 300).
const CLOSEST_BUDGET: usize = 300;

/// The elements of the Recreate steps `closest` does not pass within [`CLOSEST_BUDGET`] renders from their start,
/// searched as `lesson-check --solve` searches (the challenge's editable parameters, unlocked; stop at the first
/// passing batch). A round no search reaches is a round lesson-check refuses, so it is taught as a Do step instead.
fn unreachable_rounds(g: &Generated) -> Result<Vec<String>, String> {
    use enth_challenge::v2::{Selector, Touch};
    use enth_learn::StepKind;
    use enth_patch::{Builtin, ParamAddr, Target};
    use enth_solver::closest::{ClosestCfg, ClosestJob, RoundSerial, closest_with};
    let mut out = Vec::new();
    for step in &g.lesson.steps {
        let StepKind::Recreate { challenge, .. } = &step.kind else {
            continue;
        };
        let round = enth_challenge::prepare_challenge(challenge.as_ref().clone())
            .map_err(|e| e.to_string())?;
        let c = &challenge.constraints;
        let params_of = |sel: &Selector| -> Vec<ParamAddr> {
            let of_target = |t: &Target| -> Vec<ParamAddr> {
                let specs = match t {
                    Target::Module(id) => round
                        .start()
                        .find(id)
                        .map_or(&[][..], |(_, m)| m.kind.params()),
                    Target::Voice => Builtin::Voice.params(),
                    Target::Master => Builtin::Master.params(),
                };
                specs
                    .iter()
                    .filter_map(|sp| ParamAddr::new(t.clone(), sp.key).ok())
                    .collect()
            };
            match sel {
                Selector::Param(a) => vec![a.clone()],
                Selector::Module(t) => of_target(t),
                _ => Vec::new(),
            }
        };
        let mut free: Vec<ParamAddr> = c.editable.iter().flat_map(params_of).collect();
        free.retain(|a| !c.is_locked(&Touch::Param(a.clone())));
        free.dedup();
        let job = ClosestJob {
            round: &round,
            patch: round.start(),
            free: &free,
            budget: CLOSEST_BUDGET,
        };
        let found = closest_with(
            &job,
            &mut RoundSerial::new(),
            &ClosestCfg::default(),
            &mut |p| p.passing.is_none(),
        )
        .map_err(|e| e.to_string())?;
        if !found.passed {
            println!(
                "  {}: closest reaches {:.1} % within {CLOSEST_BUDGET} renders over {} parameter(s): taught as a Do step",
                step.id,
                found.percent,
                free.len()
            );
            if let Some(r) = g.report.steps.iter().find(|r| r.id == step.id.as_str()) {
                out.extend(r.elements.iter().cloned());
            }
        }
    }
    Ok(out)
}

/// Generate, write and report one lesson; the JSON report with the wall time. A round `closest` cannot pass is
/// regenerated as a Do step (at most a few times: each pass only adds elements).
fn one(
    patch: &enth_patch::Patch,
    source: &str,
    opts: &Options,
    pack: &Path,
    flags: &Flags,
) -> Result<String, String> {
    let started = Instant::now();
    let mut opts = opts.clone();
    let g = loop {
        let g = generate(patch, &opts)
            .map_err(|e| format!("{source}: {e} ({}; {})", e.code(), e.hint()))?;
        let bad = unreachable_rounds(&g)?;
        if bad.is_empty() || bad.iter().all(|b| opts.no_round.contains(b)) {
            break g;
        }
        opts.no_round.extend(bad);
    };
    let opts = &opts;
    let secs = started.elapsed().as_secs_f64();
    for line in g.report.lines() {
        println!("{line}");
    }
    for s in &g.report.steps {
        println!(
            "  step {:<34} {:<6} phrase {}  start {:>5.1} % ({:>5.2} JND)  {}",
            s.id,
            s.teach,
            s.phrase,
            s.start_percent,
            s.start_jnd,
            s.items.join("; ")
        );
    }
    if let Some((opts_, answer, closest)) = &g.report.missing {
        println!(
            "  what disappeared: {} (answer {}), closest pair {closest:.2} JND",
            opts_.join(" | "),
            answer + 1
        );
    }
    if let Some(items) = &g.report.variation {
        println!("  variation: {}", items.join(", "));
    }
    if !g.report.merged.is_empty() {
        println!("  merged into a neighbour: {}", g.report.merged.join(", "));
    }
    println!(
        "  {}: {} steps, {} renders in {secs:.1} s ({:.1} ms a render, release build assumed)",
        opts.lesson,
        g.lesson.steps.len(),
        g.report.renders.total(),
        1000.0 * secs / f64::from(g.report.renders.total().max(1))
    );
    if !flags.dry_run {
        write_lesson(&g, source, opts, pack, flags.force_text)?;
    }
    Ok(format!(
        "{{\"lesson\":\"{}\",\"seconds\":{secs:.3},\"report\":{}}}",
        opts.lesson,
        g.report.to_json()
    ))
}

fn write_report(flags: &Flags, reports: &[String]) -> Result<(), String> {
    if let Some(path) = &flags.report {
        fs::write(path, format!("[{}]\n", reports.join(",\n")))
            .map_err(|e| format!("{}: {e}", path.display()))?;
    }
    Ok(())
}

/// Write `lessons/<id>/lesson.toml`, its texts (an existing one is kept unless `force_text`) and its patches, after
/// checking the TOML reads back as the generated lesson.
fn write_lesson(
    g: &Generated,
    source: &str,
    opts: &Options,
    pack: &Path,
    force_text: bool,
) -> Result<(), String> {
    let dir = pack.join("lessons").join(opts.lesson.as_str());
    fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    fs::create_dir_all(pack.join("patches")).map_err(|e| format!("{}: {e}", pack.display()))?;
    let mut header = vec![
        format!(
            "{} [{} · {}]: a deconstruct lesson generated by `cargo xtask lesson-gen` (lane DEC) from {source}, on engine {}.",
            opts.title,
            pack.file_name().and_then(|n| n.to_str()).unwrap_or("pack"),
            match opts.tier {
                Tier::Basic => "basic",
                Tier::Intermediate => "intermediate",
                Tier::Advanced => "advanced",
            },
            enth_patch::file::ENGINE_VERSION
        ),
        "The steps, their checks and the patches are generated: regenerate rather than edit them. The texts (*.md) are \
         edited by hand and kept by a regeneration."
            .to_owned(),
        String::new(),
    ];
    header.extend(g.report.lines());
    let toml = lesson_toml(&g.lesson, &header, &g.notes);
    let back = Lesson::from_toml(&toml)
        .map_err(|e| format!("the generated lesson.toml does not read back: {e}"))?;
    if back != g.lesson {
        return Err(
            "the generated lesson.toml reads back as another lesson (a bug in toml_out)".to_owned(),
        );
    }
    write_if_changed(&dir.join("lesson.toml"), toml.as_bytes())?;
    let mut kept = 0;
    for (name, text) in &g.texts {
        let path = dir.join(name);
        if path.exists() && !force_text {
            kept += 1;
            continue;
        }
        write_if_changed(&path, text.as_bytes())?;
    }
    let named: BTreeSet<&str> = g.texts.keys().map(String::as_str).collect();
    for entry in fs::read_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))? {
        let path = entry.map_err(|e| e.to_string())?.path();
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_owned();
        let is_text = Path::new(&name)
            .extension()
            .is_some_and(|x| x.eq_ignore_ascii_case("md"));
        if is_text && !named.contains(name.as_str()) {
            fs::remove_file(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            println!("  removed {} (no step names it now)", path.display());
        }
    }
    for (path, patch) in &g.patches {
        write_if_changed(&pack.join(path), patch.to_json().as_bytes())?;
    }
    println!(
        "  wrote {} ({} texts kept as edited, {} patches)",
        dir.join("lesson.toml").display(),
        kept,
        g.patches.len()
    );
    Ok(())
}

fn write_if_changed(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if fs::read(path).is_ok_and(|old| old == bytes) {
        return Ok(());
    }
    fs::write(path, bytes).map_err(|e| format!("{}: {e}", path.display()))
}

/// Remove the pack's patches that no lesson of the pack names (a regeneration that dropped a step).
fn prune_patches(pack: &Path) -> Result<(), String> {
    let mut named: BTreeSet<String> = BTreeSet::new();
    let lessons = pack.join("lessons");
    for entry in fs::read_dir(&lessons).map_err(|e| format!("{}: {e}", lessons.display()))? {
        let dir = entry.map_err(|e| e.to_string())?.path();
        let Ok(text) = fs::read_to_string(dir.join("lesson.toml")) else {
            continue;
        };
        let lesson = Lesson::from_toml(&text).map_err(|e| format!("{}: {e}", dir.display()))?;
        named.extend(lesson.patches().map(|p| p.as_str().to_owned()));
    }
    let dir = pack.join("patches");
    let mut removed: BTreeSet<String> = BTreeSet::new();
    for entry in fs::read_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))? {
        let path = entry.map_err(|e| e.to_string())?.path();
        let name = format!(
            "patches/{}",
            path.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default()
        );
        if !named.contains(&name) {
            fs::remove_file(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            removed.insert(name);
        }
    }
    if !removed.is_empty() {
        println!(
            "removed {} unused patches: {}",
            removed.len(),
            removed.iter().cloned().collect::<Vec<_>>().join(", ")
        );
    }
    Ok(())
}
