//! **Owner: lane LCK.** `check`'s file rules (the [parent](super)'s step 1, after `Pack::from_files` verified the
//! manifest, `requires`, every file's size and hash, and decoded the lessons and patches): the archive and its pins,
//! patch bytes, licences, `covers`, tracks and the markdown lint.

use std::collections::BTreeSet;
use std::path::Path;

use enth_learn::Pack;
use enth_learn::markdown::{STEP_WORD_BUDGET, lint};
use enth_patch::ModuleKind;

use super::Log;
use crate::pack;
use crate::source::Source;

/// The licence ids a pack may name (docs/learn.md, "Content licence"; `content/lessons/LICENSE`).
const LICENCES: [&str; 2] = ["CC-BY-4.0", "CC0-1.0"];

/// Everything but the tracks, which need every pack ([`tracks`]).
pub(super) fn check(root: &Path, source: &Source, pack: &Pack, log: &mut Log) {
    let at = pack.id().to_string();
    let m = pack.manifest();
    let before = log.failures();

    // The archive, read back through the real reader, and its pins.
    match pack::build(source) {
        Ok(built) => {
            log.line(format!(
                "  {} {}: {} files, archive {} bytes, SHA-256 {}",
                built.id,
                built.version,
                source.files.len(),
                built.archive.len(),
                built.sha256
            ));
            for pin in pack::pins(root, &built, None) {
                match pin.mismatch {
                    None => log.line(format!("    pin {}: matches", pin.pin)),
                    Some(why) => log.fail(&at, format!("pin {}: {why}", pin.pin)),
                }
            }
        }
        Err(e) => log.fail(&at, e),
    }

    // Patches are exactly `Patch::to_json()`, at this build's schema.
    for (path, bytes) in source
        .files
        .iter()
        .filter(|(p, _)| p.starts_with("patches/"))
    {
        let Ok(text) = std::str::from_utf8(bytes) else {
            log.fail(&at, format!("{path}: not UTF-8"));
            continue;
        };
        match enth_patch::file::load(text) {
            Ok(loaded) if loaded.from_schema != enth_patch::file::SCHEMA => log.fail(
                &at,
                format!(
                    "{path}: patch schema {} migrates on load; write it at schema {} (Patch::to_json())",
                    loaded.from_schema,
                    enth_patch::file::SCHEMA
                ),
            ),
            Ok(loaded) => {
                let canonical = loaded.patch.to_json();
                if canonical.as_bytes() != bytes.as_slice() {
                    let at_byte = canonical
                        .bytes()
                        .zip(bytes.iter())
                        .position(|(a, b)| a != *b)
                        .unwrap_or_else(|| canonical.len().min(bytes.len()));
                    log.fail(
                        &at,
                        format!(
                            "{path}: not byte for byte Patch::to_json() (first difference at byte {at_byte}; \
                             {} bytes, canonical {}): write it with `enth new`/`enth apply`",
                            bytes.len(),
                            canonical.len()
                        ),
                    );
                }
            }
            Err(e) => log.fail(&at, format!("{path}: {e}")),
        }
    }

    // A scaffold's placeholder left in (titles, objectives, hints, labels, texts, credits, patch names).
    for (path, bytes) in &source.files {
        if bytes
            .windows(crate::new::TODO.len())
            .any(|w| w == crate::new::TODO.as_bytes())
        {
            log.fail(
                &at,
                format!(
                    "{path}: a {} is left (`lesson new`'s placeholder): write it",
                    crate::new::TODO
                ),
            );
        }
    }

    // Licences.
    for (field, id) in [("text", &m.license.text), ("patches", &m.license.patches)] {
        if !LICENCES.contains(&id.as_str()) {
            log.fail(
                &at,
                format!(
                    "license.{field} is {id:?}; a pack's content is {} (content/lessons/LICENSE)",
                    LICENCES.join(" or ")
                ),
            );
        }
    }

    // `covers` names the module kinds the lessons teach, no more and no fewer.
    let taught: BTreeSet<ModuleKind> = pack
        .lessons()
        .flat_map(|l| l.teaches.iter())
        .filter_map(|t| t.as_str().split_once('.'))
        .filter_map(|(kind, _)| ModuleKind::from_key(kind))
        .collect();
    let covers: BTreeSet<ModuleKind> = m.covers.iter().map(|k| k.0).collect();
    for k in taught.difference(&covers) {
        log.fail(
            &at,
            format!("covers lacks {}, which a lesson teaches", k.key()),
        );
    }
    for k in covers.difference(&taught) {
        log.fail(
            &at,
            format!("covers names {}, which no lesson teaches", k.key()),
        );
    }

    // The markdown lint, once per text (resolution against each step's start is the playthrough's).
    for (path, bytes) in source.files.iter().filter(|(p, _)| {
        p.starts_with("lessons/") && Path::new(p.as_str()).extension().is_some_and(|e| e == "md")
    }) {
        let text = String::from_utf8_lossy(bytes);
        for issue in lint(&text, STEP_WORD_BUDGET) {
            log.fail(
                &at,
                format!(
                    "{path}:{}: {} ({})",
                    issue.line(&text),
                    issue.message,
                    issue.code
                ),
            );
        }
    }
    if log.failures() == before {
        log.line("    files: ok (layout, schemas, sizes and hashes, patch bytes, licences, covers, markdown lint)");
    }
}

/// Track entries name lessons of the packs checked; an entry whose pack is not among them is listed, not failed
/// (that pack is written later, and the app shows its lessons as missing).
pub(super) fn tracks(packs: &[Pack], log: &mut Log) {
    let mut absent: BTreeSet<String> = BTreeSet::new();
    for pack in packs {
        for track in &pack.manifest().tracks {
            for entry in &track.lessons {
                let at = format!("{}: track {}", pack.id(), track.id);
                match packs.iter().find(|p| p.id() == &entry.pack) {
                    Some(other) if other.lesson(&entry.lesson).is_none() => {
                        log.fail(&at, format!("{entry} is not a lesson of {}", entry.pack));
                    }
                    Some(_) => {}
                    None => {
                        absent.insert(format!("{entry} ({at})"));
                    }
                }
            }
        }
    }
    if !absent.is_empty() {
        log.line("tracks:");
        log.note(format!(
            "{} track entr{} name packs not checked here: {}",
            absent.len(),
            if absent.len() == 1 { "y" } else { "ies" },
            absent.into_iter().collect::<Vec<_>>().join(", ")
        ));
    }
}
