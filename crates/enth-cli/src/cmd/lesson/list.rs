//! **Owner: lane EV3.** `enth lesson list <PATH>...`: the packs at the given paths and the lessons in them, each lesson
//! with its steps in short (number, id, kind, authored hints). `enth.lesson-list/1`.
//!
//! A path is a pack (a folder holding `pack.toml` or `pack.json`, or an `.enthpack` file), or a folder of packs
//! (`content/lessons`, a Packs folder): one level down, every pack folder and every `.enthpack` is listed. A pack named
//! outright that this build refuses is an error (exit 3, the reader's code and hint, as for `show`); one found while
//! scanning a folder is listed under `refused` with the same reason, so a newer pack is named and never hidden (the
//! plugin's browser does the same).

use std::path::{Path, PathBuf};

use enth_learn::Pack;
use enth_learn::lesson::Tier;
use enth_learn::pack::MANIFEST_PATH;

use super::load_pack;
use crate::cmd::{Reply, Res};
use crate::error::{self, CliError};
use crate::patchio;
use crate::schema::{
    LessonListEntryOut, LessonListOut, LessonListPackOut, LessonRefusedOut, LessonStepRefOut,
    LessonTrackOut, ids,
};

/// Whether `dir` holds a pack in the authoring layout or a built one.
fn is_pack_dir(dir: &Path) -> bool {
    dir.join("pack.toml").is_file() || dir.join(MANIFEST_PATH).is_file()
}

/// A tier as the lesson file spells it.
pub(super) fn tier_name(t: Tier) -> String {
    serde_json::to_value(t)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default()
}

/// The packs one path stands for: itself, or the packs one level inside it.
fn candidates(path: &Path) -> Result<(Vec<PathBuf>, bool), CliError> {
    if path.is_file() || is_pack_dir(path) {
        return Ok((vec![path.to_owned()], false));
    }
    if !path.is_dir() {
        // Not there: the reader's own message (exit 5).
        patchio::read_bytes(path)?;
        return Ok((vec![path.to_owned()], false));
    }
    let entries =
        std::fs::read_dir(path).map_err(|e| error::not_found(&patchio::show(path), &e))?;
    let mut found: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            let hidden = p
                .file_name()
                .and_then(|n| n.to_str())
                .is_none_or(|n| n.starts_with('.'));
            !hidden
                && (is_pack_dir(p) || p.extension().is_some_and(|x| x == "enthpack") && p.is_file())
        })
        .collect();
    found.sort();
    if found.is_empty() {
        return Err(error::invalid(
            "not_a_pack",
            format!(
                "{} holds neither pack.toml nor pack.json, and no pack folder or .enthpack file inside it",
                patchio::show(path)
            ),
            "give a pack (content/lessons/<pack> or a built .enthpack) or a folder of packs (content/lessons)",
        ));
    }
    Ok((found, true))
}

fn pack_out(path: &Path, pack: &Pack) -> LessonListPackOut {
    let m = pack.manifest();
    LessonListPackOut {
        path: patchio::show(path),
        id: m.id.to_string(),
        version: m.version.to_string(),
        title: m.title.clone(),
        engine: m.engine.clone(),
        covers: m.covers.iter().map(|k| k.0.key().to_owned()).collect(),
        tracks: m
            .tracks
            .iter()
            .map(|t| LessonTrackOut {
                id: t.id.to_string(),
                title: t.title.clone(),
                lessons: t.lessons.iter().map(ToString::to_string).collect(),
            })
            .collect(),
        lessons: m
            .lessons
            .iter()
            .filter_map(|id| pack.bundle(id))
            .map(|b| {
                let l = &b.lesson;
                LessonListEntryOut {
                    key: b.key().to_string(),
                    id: l.id.to_string(),
                    title: l.title.clone(),
                    tier: tier_name(l.tier),
                    objective: l.objective.clone(),
                    teaches: l.teaches.iter().map(|t| t.as_str().to_owned()).collect(),
                    steps: l
                        .steps
                        .iter()
                        .enumerate()
                        .map(|(i, s)| LessonStepRefOut {
                            number: u16::try_from(i + 1).unwrap_or(u16::MAX),
                            id: s.id.to_string(),
                            kind: s.kind.name().to_owned(),
                            hints: u16::try_from(s.hints.len()).unwrap_or(u16::MAX),
                        })
                        .collect(),
                }
            })
            .collect(),
    }
}

pub(super) fn list(paths: &[PathBuf]) -> Res {
    let mut packs = Vec::new();
    let mut refused = Vec::new();
    for path in paths {
        let (found, scanned) = candidates(path)?;
        for p in found {
            match load_pack(&p) {
                Ok(pack) => packs.push(pack_out(&p, &pack)),
                Err(e) if scanned => refused.push(LessonRefusedOut {
                    path: patchio::show(&p),
                    code: e.0.error,
                    message: e.0.message,
                    hint: e.0.hint,
                }),
                Err(e) => return Err(e),
            }
        }
    }
    let lessons: usize = packs.iter().map(|p| p.lessons.len()).sum();
    let next = match packs
        .first()
        .and_then(|p| p.lessons.first().map(|l| (p, l)))
    {
        Some((p, l)) => format!(
            "`enth lesson show {} --lesson {}` reads a lesson ({lessons} lessons in {} packs)",
            p.path,
            l.id,
            packs.len()
        ),
        None => "no pack could be read; `refused` says why".to_owned(),
    };
    Ok(Reply::new(&LessonListOut {
        schema: ids::LESSON_LIST.to_owned(),
        packs,
        refused,
        next,
    }))
}
