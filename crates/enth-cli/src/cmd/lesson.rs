//! **Owner: lane SKL** (`show` and `check`, freeze point F2l), then **EV3** (`list`, `hint` and `closest`, additive).
//! Lane S0l pre-declared the command; SKL filled [`Args`] and [`run`] here, un-hid it in `cli.rs`, added its schemas
//! (`schema/lesson.rs`) and snapshots, and documented it in docs/agent-cli.md.
//!
//! `enth lesson`: the agent evals' door to lessons (docs/learn.md, "Where Learn lives"). Stateless like `enth check`:
//! the lesson logic is `enth-learn`'s, and a pack is read from a path: a folder in the authoring layout
//! (`pack.toml`, read through the crate's `toml` feature and sealed as `cargo xtask pack` will), a built folder
//! (`pack.json`) or an `.enthpack` archive, every one through `enth-learn`'s verified reader.
//!
//! - `enth lesson show <PACK> [--lesson ID] [--reveal]`: the pack's lessons step by step: each step's kind, text,
//!   hints, view (start, highlight, hide, editable, locked), a do step's check, the examples' or options' labels, a
//!   recreate step's challenge (`enth.lesson-show/1`). No answers (a choose step's answer, a witness) unless
//!   `--reveal`.
//! - `enth lesson check <PACK> --lesson ID --step STEP <PATCH>`: one do step's check run against a patch file: its
//!   patch predicates, then its sound checks (lane PLY's `enth_learn::check::evaluate_sounds`: each rendered through
//!   its phrase at the scored rate and seed and judged; `enth.lesson-check/1`). Exit 1 when it fails. `STEP` is the step's id or its 1-based number.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use enth_learn::check::{TouchLog, evaluate_sounds};
use enth_learn::pack::{LessonId, MANIFEST_PATH, lesson_path};
use enth_learn::{
    Check, LearnError, Lesson, LessonBundle, Manifest, Pack, Predicate, Step, StepKind,
};

use super::{Reply, Res};
use crate::error::{self, CliError, EXIT_CHECK_FAILED, nearest};
use crate::patchio;
use crate::schema::{
    LessonCheckOut, LessonOut, LessonPackOut, LessonPredicateOut, LessonShowOut,
    LessonSoundItemOut, LessonStepOut, ids,
};

/// `enth lesson`'s arguments.
#[derive(Clone, Debug, PartialEq, Eq, clap::Args)]
pub struct Args {
    #[command(subcommand)]
    pub cmd: Sub,
}

/// `enth lesson`'s subcommands.
#[derive(Clone, Debug, PartialEq, Eq, clap::Subcommand)]
pub enum Sub {
    /// A pack's lessons, step by step (no answers unless --reveal).
    Show {
        /// The pack: a folder (`pack.toml` or `pack.json`) or an `.enthpack` file.
        pack: PathBuf,
        /// One lesson's id (default: every lesson of the pack).
        #[arg(long, value_name = "ID")]
        lesson: Option<String>,
        /// Also show a choose step's answer and a do step's witness (it gives the answers away).
        #[arg(long)]
        reveal: bool,
    },
    /// Run one do step's check against a patch file. Exit 1 if it fails.
    Check {
        /// The pack: a folder (`pack.toml` or `pack.json`) or an `.enthpack` file.
        pack: PathBuf,
        /// The lesson's id.
        #[arg(long, value_name = "ID")]
        lesson: String,
        /// The step: its id or its 1-based number.
        #[arg(long, value_name = "STEP")]
        step: String,
        /// The patch file to check.
        patch: PathBuf,
    },
}

/// Run `enth lesson`.
pub fn run(args: &Args) -> Res {
    match &args.cmd {
        Sub::Show {
            pack,
            lesson,
            reveal,
        } => show(pack, lesson.as_deref(), *reveal),
        Sub::Check {
            pack,
            lesson,
            step,
            patch,
        } => check(pack, lesson, step, patch),
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Reading a pack
// ---------------------------------------------------------------------------------------------------------------

/// A refused pack: `enth-learn`'s code and hint (exit 3).
fn refused(path: &Path, e: &LearnError) -> CliError {
    error::invalid(e.code(), format!("{}: {e}", patchio::show(path)), e.hint())
        .in_file(patchio::show(path))
}

/// Read a pack from `path` (see the module docs).
fn load_pack(path: &Path) -> Result<Pack, CliError> {
    if path.is_dir() {
        let files = if path.join(MANIFEST_PATH).is_file() {
            read_folder(path)?
        } else if path.join("pack.toml").is_file() {
            authoring(path)?
        } else {
            return Err(error::invalid(
                "not_a_pack",
                format!(
                    "{} holds neither pack.toml nor pack.json",
                    patchio::show(path)
                ),
                "give a pack folder (content/lessons/<pack>) or a built .enthpack file",
            ));
        };
        return Pack::from_files(&files).map_err(|e| refused(path, &e));
    }
    let bytes = patchio::read_bytes(path)?;
    enth_learn::reader::read_archive(&bytes).map_err(|e| refused(path, &e))
}

/// A built folder's files by pack path (hidden entries skipped, as the plugin's store skips them).
fn read_folder(root: &Path) -> Result<BTreeMap<String, Vec<u8>>, CliError> {
    fn walk(dir: &Path, prefix: &str, out: &mut BTreeMap<String, Vec<u8>>) -> Result<(), CliError> {
        let entries =
            std::fs::read_dir(dir).map_err(|e| error::not_found(&patchio::show(dir), &e))?;
        let mut paths: Vec<(String, PathBuf)> = entries
            .filter_map(Result::ok)
            .filter_map(|e| Some((e.file_name().to_str()?.to_owned(), e.path())))
            .filter(|(n, _)| !n.starts_with('.'))
            .collect();
        paths.sort();
        for (name, path) in paths {
            let rel = if prefix.is_empty() {
                name
            } else {
                format!("{prefix}/{name}")
            };
            if path.is_dir() {
                walk(&path, &rel, out)?;
            } else {
                out.insert(rel, patchio::read_bytes(&path)?);
            }
        }
        Ok(())
    }
    let mut out = BTreeMap::new();
    walk(root, "", &mut out)?;
    Ok(out)
}

/// The authoring layout built as `cargo xtask pack` will (lane LCK): each TOML file through the same serde types as
/// its JSON, the texts, patches and credits copied, the manifest sealed.
fn authoring(dir: &Path) -> Result<BTreeMap<String, Vec<u8>>, CliError> {
    let text = |p: &Path| -> Result<String, CliError> {
        String::from_utf8(patchio::read_bytes(p)?).map_err(|_| {
            error::invalid(
                "bad_content",
                format!("{} is not UTF-8 text", patchio::show(p)),
                "save it as UTF-8",
            )
        })
    };
    let manifest =
        Manifest::from_toml(&text(&dir.join("pack.toml"))?).map_err(|e| refused(dir, &e))?;
    let mut files = BTreeMap::new();
    let add_dir = |sub: &str, files: &mut BTreeMap<String, Vec<u8>>| -> Result<(), CliError> {
        let path = dir.join(sub);
        if !path.is_dir() {
            return Ok(());
        }
        for (rel, bytes) in read_folder(&path)? {
            files.insert(format!("{sub}/{rel}"), bytes);
        }
        Ok(())
    };
    add_dir("patches", &mut files)?;
    let credits = dir.join(enth_learn::pack::CREDITS_PATH);
    if credits.is_file() {
        files.insert(
            enth_learn::pack::CREDITS_PATH.to_owned(),
            patchio::read_bytes(&credits)?,
        );
    }
    for id in &manifest.lessons {
        let ldir = dir.join("lessons").join(id.as_str());
        for (name, bytes) in read_folder(&ldir)? {
            if name == "lesson.toml" {
                let path = ldir.join(&name);
                let source = String::from_utf8_lossy(&bytes).into_owned();
                let lesson = Lesson::from_toml(&source).map_err(|e| refused(&path, &e))?;
                files.insert(lesson_path(id), lesson.to_json().into_bytes());
            } else {
                files.insert(format!("lessons/{id}/{name}"), bytes);
            }
        }
    }
    let sealed = manifest.seal(&files).map_err(|e| refused(dir, &e))?;
    files.insert(MANIFEST_PATH.to_owned(), sealed.to_json().into_bytes());
    Ok(files)
}

/// The lesson `id` of `pack`, or an error naming the lessons it has.
fn lesson_of(pack: &Pack, id: &str) -> Result<LessonBundle, CliError> {
    let found = id
        .parse::<LessonId>()
        .ok()
        .and_then(|lid| pack.bundle(&lid));
    found.ok_or_else(|| {
        let names: Vec<String> = pack.lessons().map(|l| l.id.to_string()).collect();
        error::usage(
            "unknown_lesson",
            format!("pack {} has no lesson '{id}'", pack.id()),
            format!("its lessons are {}", names.join(", ")),
        )
        .suggest(nearest(id, names.iter().map(String::as_str)))
        .known(names)
    })
}

/// The step `which` (an id or a 1-based number) of `lesson`.
fn step_of<'a>(lesson: &'a Lesson, which: &str) -> Result<(usize, &'a Step), CliError> {
    let by_number = which
        .trim()
        .parse::<usize>()
        .ok()
        .filter(|n| (1..=lesson.steps.len()).contains(n))
        .map(|n| (n - 1, &lesson.steps[n - 1]));
    let by_id = || {
        lesson
            .steps
            .iter()
            .enumerate()
            .find(|(_, s)| s.id.as_str() == which.trim())
    };
    by_number.or_else(by_id).ok_or_else(|| {
        let names: Vec<String> = lesson.steps.iter().map(|s| s.id.to_string()).collect();
        error::usage(
            "unknown_step",
            format!("lesson {} has no step '{which}'", lesson.id),
            format!(
                "give a step's id ({}) or its number, 1 to {}",
                names.join(", "),
                lesson.steps.len()
            ),
        )
        .suggest(nearest(which, names.iter().map(String::as_str)))
        .known(names)
    })
}

// ---------------------------------------------------------------------------------------------------------------
// show
// ---------------------------------------------------------------------------------------------------------------

fn texts(list: &[enth_learn::Selector]) -> Vec<String> {
    list.iter().map(ToString::to_string).collect()
}

fn step_out(bundle: &LessonBundle, i: usize, s: &Step, reveal: bool) -> LessonStepOut {
    let (check, examples, challenge, answer, witness) = match &s.kind {
        StepKind::Do { check, witness } => (
            serde_json::to_value(check).ok(),
            Vec::new(),
            None,
            None,
            reveal.then(|| witness.to_string()),
        ),
        StepKind::Listen { examples } => (
            None,
            examples.iter().map(|e| e.label.clone()).collect(),
            None,
            None,
            None,
        ),
        StepKind::Choose { options, answer } => (
            None,
            options.iter().map(|e| e.label.clone()).collect(),
            None,
            reveal.then_some(*answer),
            None,
        ),
        StepKind::Recreate { challenge, witness } => (
            None,
            Vec::new(),
            serde_json::from_str(&challenge.to_json()).ok(),
            None,
            witness.as_ref().filter(|_| reveal).map(ToString::to_string),
        ),
        StepKind::Read => (None, Vec::new(), None, None, None),
    };
    LessonStepOut {
        number: u16::try_from(i + 1).unwrap_or(u16::MAX),
        id: s.id.to_string(),
        kind: s.kind.name().to_owned(),
        text: s
            .text
            .as_ref()
            .and_then(|t| bundle.text(t))
            .map(str::to_owned),
        hints: s.hints.clone(),
        start: s.view.start.as_ref().map(ToString::to_string),
        highlight: texts(&s.view.highlight),
        hide: texts(&s.view.hide),
        editable: texts(&s.view.editable),
        locked: texts(&s.view.locked),
        check,
        examples,
        challenge,
        answer,
        witness,
    }
}

fn lesson_out(bundle: &LessonBundle, reveal: bool) -> LessonOut {
    let l = &bundle.lesson;
    LessonOut {
        key: bundle.key().to_string(),
        id: l.id.to_string(),
        title: l.title.clone(),
        tier: serde_json::to_value(l.tier)
            .ok()
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_default(),
        objective: l.objective.clone(),
        teaches: l.teaches.iter().map(|t| t.as_str().to_owned()).collect(),
        steps: l
            .steps
            .iter()
            .enumerate()
            .map(|(i, s)| step_out(bundle, i, s, reveal))
            .collect(),
    }
}

fn show(path: &Path, lesson: Option<&str>, reveal: bool) -> Res {
    let pack = load_pack(path)?;
    let bundles: Vec<LessonBundle> = match lesson {
        Some(id) => vec![lesson_of(&pack, id)?],
        None => pack
            .manifest()
            .lessons
            .iter()
            .filter_map(|id| pack.bundle(id))
            .collect(),
    };
    let m = pack.manifest();
    let first = bundles.first().map(|b| b.lesson.id.to_string());
    let out = LessonShowOut {
        schema: ids::LESSON_SHOW.to_owned(),
        pack: LessonPackOut {
            id: m.id.to_string(),
            version: m.version.to_string(),
            title: m.title.clone(),
            engine: m.engine.clone(),
        },
        lessons: bundles.iter().map(|b| lesson_out(b, reveal)).collect(),
        next: match first {
            Some(id) => format!(
                "check a do step against a patch: `enth lesson check {} --lesson {id} --step <id|n> <patch>`",
                patchio::show(path)
            ),
            None => "the pack has no lessons".to_owned(),
        },
    };
    Ok(Reply::new(&out))
}

// ---------------------------------------------------------------------------------------------------------------
// check
// ---------------------------------------------------------------------------------------------------------------

/// The patch predicates of a check, in order.
fn predicates(check: &Check) -> Vec<&Predicate> {
    match check {
        Check::Patch { predicates } => predicates.iter().collect(),
        Check::All { checks } => checks.iter().flat_map(predicates).collect(),
        Check::Sound { .. } => Vec::new(),
    }
}

fn check(path: &Path, lesson: &str, which: &str, patch_path: &Path) -> Res {
    let pack = load_pack(path)?;
    let bundle = lesson_of(&pack, lesson)?;
    let (i, step) = step_of(&bundle.lesson, which)?;
    let StepKind::Do { check, .. } = &step.kind else {
        let how = match step.kind {
            StepKind::Choose { .. } => {
                "an ear question is answered by picking an option, not by a patch"
            }
            StepKind::Recreate { .. } => {
                "a recreate step is a round: score a patch with `enth compare --challenge` on its challenge"
            }
            _ => "a read or listen step passes when the player moves on",
        };
        return Err(error::usage(
            "not_checkable",
            format!(
                "step {} ({}) of {} is a {} step, which has no check",
                i + 1,
                step.id,
                bundle.key(),
                step.kind.name()
            ),
            format!("{how}; `enth lesson show` lists the do steps"),
        ));
    };
    let (patch, _notes) = patchio::load(patch_path)?;
    let touched = TouchLog::default();
    let preds: Vec<LessonPredicateOut> = predicates(check)
        .into_iter()
        .map(|p| LessonPredicateOut {
            predicate: serde_json::to_value(p).unwrap_or_default(),
            pass: p.holds(&patch, &touched),
        })
        .collect();
    let has_touched = predicates(check)
        .iter()
        .any(|p| matches!(p, Predicate::Touched { .. }));
    let sounds = check.sounds();
    let verdict = evaluate_sounds(&sounds, &patch).map_err(|e| {
        error::failed(
            e.code(),
            format!("the step's sound check could not run: {e}"),
            e.hint(),
        )
    })?;
    // Each sound check's items, in order, with its phrase.
    let mut items = verdict.items.iter();
    let mut sound_out = Vec::new();
    for s in &sounds {
        for _ in &s.expect {
            if let Some(v) = items.next() {
                sound_out.push(LessonSoundItemOut {
                    phrase: s.phrase.0,
                    item: v.item.clone(),
                    pass: v.passed,
                    margin: v.margin,
                });
            }
        }
    }
    let pass = preds.iter().all(|p| p.pass) && verdict.passed;
    let out = LessonCheckOut {
        schema: ids::LESSON_CHECK.to_owned(),
        lesson: bundle.key().to_string(),
        step: step.id.to_string(),
        number: u16::try_from(i + 1).unwrap_or(u16::MAX),
        patch: patchio::show(patch_path),
        pass,
        predicates: preds,
        sounds: sound_out,
        note: has_touched.then(|| {
            "a `touched` predicate reads the editor's edit log, which a patch file has not: it fails here".to_owned()
        }),
        next: if pass {
            "the step passes with this patch".to_owned()
        } else {
            "change the patch (`enth apply`) and check again; `enth check <patch> --expect <items>` measures each item"
                .to_owned()
        },
    };
    let code = if pass { 0 } else { EXIT_CHECK_FAILED };
    Ok(Reply::new(&out).code(code))
}
