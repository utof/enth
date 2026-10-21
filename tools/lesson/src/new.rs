//! **Owner: lane LCK.** Pre-declared by lane S0l (R2).
//!
//! `new [--in DIR] <pack> <id>` (`cargo xtask lesson new`): scaffolds a lesson in the authoring layout of
//! `content/lessons/<pack>/`, the first step of the authoring pipeline (docs/learn.md): `lessons/<id>/lesson.toml` with
//! the lesson shape (Hear, Name, Try, an ear question), one markdown file per step, a start patch and a witness in
//! `patches/` (the starter, and the starter with the cutoff at 400 Hz), and the id appended to `pack.toml`'s `lessons`. A pack that
//! does not exist yet gets a `pack.toml` and a `CREDITS.md`. The scaffold reads and plays (its Try step is the example
//! pack's cutoff check), but its titles and texts say TODO, and `lesson-check` refuses a TODO, so it cannot ship
//! before an author drafts it from the plan and the catalogue (`enth params`, `enth explain`) and `check` proves it
//! honest.

use std::fs;
use std::path::{Path, PathBuf};

use enth_learn::Manifest;
use enth_learn::pack::{LessonId, PackId};
use enth_patch::Op;

use crate::source::{self, LESSON_TOML, LESSONS_DIR, PACK_TOML};

/// The marker a scaffold leaves in every text an author must write; `lesson-check` refuses it.
pub(crate) const TODO: &str = "TODO";

/// `new`.
pub fn run(args: &[String]) -> Result<(), String> {
    const USAGE: &str =
        "usage: new [--in DIR] <pack> <id>   (DIR: where packs live, default content/lessons)";
    let (lessons_dir, rest) = match args {
        [flag, dir, rest @ ..] if flag == "--in" => (PathBuf::from(dir), rest),
        rest => (source::repo_root().join(LESSONS_DIR), rest),
    };
    let [pack, id] = rest else {
        return Err(USAGE.to_owned());
    };
    PackId::new(pack).map_err(|e| format!("pack id {pack:?}: {e}"))?;
    LessonId::new(id).map_err(|e| format!("lesson id {id:?}: {e}"))?;
    let dir = lessons_dir.join(pack);
    scaffold(&dir, pack, id)?;
    println!(
        "scaffolded {pack}/{id} in {}: lessons/{id}/ (lesson.toml and four texts), patches/{id}-start.patch.json and \
         patches/{id}-witness.patch.json, and `{id}` in {PACK_TOML}'s lessons",
        dir.display()
    );
    println!(
        "next: replace every {TODO}, build the start and witness with `enth new | apply | check | compare`, then run \
         `cargo xtask lesson-check --cross --solve {pack}`"
    );
    Ok(())
}

/// Write the scaffold into `dir` (the pack directory).
pub(crate) fn scaffold(dir: &Path, pack: &str, id: &str) -> Result<(), String> {
    let lesson_dir = dir.join("lessons").join(id);
    let start_name = format!("{id}-start.patch.json");
    let witness_name = format!("{id}-witness.patch.json");
    let patches = dir.join("patches");
    for taken in [
        lesson_dir.clone(),
        patches.join(&start_name),
        patches.join(&witness_name),
    ] {
        if taken.exists() {
            return Err(format!(
                "{} exists already: pick another id",
                taken.display()
            ));
        }
    }
    let manifest_path = dir.join(PACK_TOML);
    let manifest_text = if manifest_path.is_file() {
        let text = fs::read_to_string(&manifest_path)
            .map_err(|e| format!("cannot read {}: {e}", manifest_path.display()))?;
        let m = Manifest::from_toml(&text)
            .map_err(|e| format!("{PACK_TOML}: {}", source::learn_error(&e)))?;
        if m.lessons.iter().any(|l| l.as_str() == id) {
            return Err(format!("{PACK_TOML} lists {id} already"));
        }
        let edited = add_lesson(&text, id).ok_or_else(|| {
            format!("cannot find `lessons = [...]` in {PACK_TOML}: add \"{id}\" to it by hand, then run this again")
        })?;
        let m = Manifest::from_toml(&edited).map_err(|e| {
            format!(
                "adding {id} to {PACK_TOML} broke it ({}): add it by hand",
                source::learn_error(&e)
            )
        })?;
        if !m.lessons.iter().any(|l| l.as_str() == id) {
            return Err(format!(
                "adding {id} to {PACK_TOML} did not take: add it by hand"
            ));
        }
        edited
    } else {
        fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
        fs::write(dir.join(enth_learn::pack::CREDITS_PATH), credits(pack))
            .map_err(|e| format!("cannot write CREDITS.md: {e}"))?;
        new_manifest(pack, id)
    };

    // The starter (template 0): what the editor's Init gives and the lessons' instance ids name (PLAN.md §1.1).
    let start = enth_challenge::random::start(0).map_err(|e| format!("the starter patch: {e}"))?;
    let mut witness = start.clone();
    enth_patch::apply(
        &mut witness,
        &Op::SetParam {
            addr: "filter1.cutoff".parse().map_err(|e| format!("{e}"))?,
            value: 400.0,
        },
    )
    .map_err(|e| format!("the witness: {e}"))?;

    let write = |path: &Path, text: &str| -> Result<(), String> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
        }
        fs::write(path, text).map_err(|e| format!("cannot write {}: {e}", path.display()))
    };
    write(&patches.join(&start_name), &start.to_json())?;
    write(&patches.join(&witness_name), &witness.to_json())?;
    write(
        &lesson_dir.join(LESSON_TOML),
        &lesson_toml(id, &start_name, &witness_name),
    )?;
    for (name, heading, body) in [
        (
            "hear.md",
            "what the player hears",
            "what to listen for, before anything is named",
        ),
        (
            "name.md",
            "the control's name",
            "the idea in at most 80 words, in perceptual terms",
        ),
        (
            "try.md",
            "the goal",
            "what to do, and how to hear that it is done",
        ),
        (
            "which.md",
            "the question",
            "the ear question; the answer is one of the options",
        ),
    ] {
        write(
            &lesson_dir.join(name),
            &format!("# {TODO}: {heading}\n\n{TODO}: {body}.\n"),
        )?;
    }
    write(&manifest_path, &manifest_text)?;
    Ok(())
}

/// `text` with `id` appended to its top-level `lessons = [...]` array, or `None` when there is no such array.
fn add_lesson(text: &str, id: &str) -> Option<String> {
    let mut offset = 0;
    let mut found = None;
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim_start();
        if trimmed.starts_with('[') {
            break; // a table: the top-level keys are behind us
        }
        if let Some(rest) = trimmed.strip_prefix("lessons")
            && rest.trim_start().starts_with('=')
        {
            found = Some(offset + (line.len() - trimmed.len()));
            break;
        }
        offset += line.len();
    }
    let start = found?;
    let open = start + text[start..].find('[')?;
    let close = open + text[open..].find(']')?;
    let before = text[open + 1..close].trim_end();
    let insert = if before.is_empty() {
        format!("\"{id}\"")
    } else if before.ends_with(',') {
        format!(" \"{id}\"")
    } else {
        format!(", \"{id}\"")
    };
    let at = open + 1 + before.len();
    Some(format!("{}{insert}{}", &text[..at], &text[at..]))
}

fn credits(pack: &str) -> String {
    format!(
        "# Credits: the {pack} pack\n\n{TODO}: who wrote it. The lesson text is licensed CC BY 4.0 and the patches CC0 \
         1.0, under the terms in [`../LICENSE`](../LICENSE).\n"
    )
}

fn new_manifest(pack: &str, id: &str) -> String {
    format!(
        r#"# The {pack} pack (docs/learn.md, "Packs"; `cargo xtask pack` builds it into {pack}.enthpack).
schema = "enth.pack/1"
id = "{pack}"
version = "0.1.0"
title = "{TODO}: the pack's title"
authors = ["enth contributors"]
covers = ["svf"]
lessons = ["{id}"]

[license]
text = "CC-BY-4.0"
patches = "CC0-1.0"

[requires]
lesson = 1
patch = 1
"#
    )
}

fn lesson_toml(id: &str, start: &str, witness: &str) -> String {
    format!(
        r#"# Scaffolded by `cargo xtask lesson new`. The shape is Hear (listen), Name (read), Try (do: a goal checked live)
# and an ear question (choose); content/lessons/PLAN.md has the card to draft from. Replace every {TODO}, and the
# patches with the lesson's own (exactly `Patch::to_json()`), then run `cargo xtask lesson-check --cross --solve`.

schema = "enth.lesson/1"
id = "{id}"
title = "{TODO}: the title"
tier = "basic"
objective = "{TODO}: what the player can do after it, in one sentence."
teaches = ["svf.cutoff"]

# Hear: examples that differ only in the new control, before it is named.
[[steps]]
id = "hear"
kind = "listen"
text = "hear.md"

[[steps.examples]]
label = "A"
patch = "patches/{start}"

[[steps.examples]]
label = "B"
patch = "patches/{witness}"

# Name: the idea in perceptual terms.
[[steps]]
id = "name"
kind = "read"
text = "name.md"

[steps.view]
highlight = ["filter1.cutoff"]

# Try: a goal checked live. The start must fail the check and the witness pass it by at least 1e-3 (lesson-check).
[[steps]]
id = "try"
kind = "do"
text = "try.md"
hints = ["{TODO}: a first hint."]
witness = "patches/{witness}"

[steps.view]
start = "patches/{start}"
highlight = ["filter1.cutoff"]
editable = ["filter1.cutoff"]

[steps.check]
kind = "sound"
phrase = 1
expect = ["brightness<500Hz"]

# An ear question: the options at least 2 JND apart under sound/1 (lesson-check --solve).
[[steps]]
id = "which"
kind = "choose"
text = "which.md"
answer = 1

[[steps.options]]
label = "A"
patch = "patches/{start}"

[[steps.options]]
label = "B"
patch = "patches/{witness}"
"#
    )
}

#[cfg(test)]
mod tests {
    use super::add_lesson;

    #[test]
    fn a_lesson_joins_the_lessons_array_in_any_layout() {
        assert_eq!(
            add_lesson("id = \"x\"\nlessons = [\"a\"]\n", "b").unwrap(),
            "id = \"x\"\nlessons = [\"a\", \"b\"]\n"
        );
        assert_eq!(
            add_lesson("lessons = []\n", "b").unwrap(),
            "lessons = [\"b\"]\n"
        );
        assert_eq!(
            add_lesson("lessons = [\n  \"a\",\n]\n", "b").unwrap(),
            "lessons = [\n  \"a\", \"b\"\n]\n"
        );
        assert_eq!(
            add_lesson("lessons = [\n  \"a\"\n]\n", "b").unwrap(),
            "lessons = [\n  \"a\", \"b\"\n]\n"
        );
        // Only the top-level key, never a track's `lessons`.
        assert_eq!(add_lesson("[[tracks]]\nlessons = [\"a\"]\n", "b"), None);
        assert_eq!(add_lesson("title = \"t\"\n", "b"), None);
    }
}
