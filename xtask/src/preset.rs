//! `cargo xtask extract-presets`: turn sounds saved in REAPER into preset files (P1b lane INT; owner build 2).
//!
//! The owner makes a sound in the editor and saves the REAPER project. Every enth instance in a project file (`.rpp`,
//! or an FX chain `.RfxChain`, which uses the same blocks) carries the plugin's saved state: base64 around nice-plug's
//! JSON (`{"version","params","fields"}`), whose `patch` field is exactly `Patch::to_json()` (docs/plugin.md,
//! "State"). This command finds those instances (CLAP id `dev.enth.synth`, VST3 class id `55555EAD…`), decodes the
//! state with the REAPER harness's chunk code ([`crate::reaper`]'s `chunk.rs`) and writes each `patch` field to
//! `<out>/<name>.patch.json`, as `Patch::to_json()` with only its metadata changed ([`preset_text`]). Such a file is a
//! patch document like any other: `enth describe`, `enth render` and the test host's `--state` read it, and the plugin
//! loads it and saves it back unchanged.
//!
//! The name is the FX's own name if the owner renamed the instance in REAPER, else the track's name, else the
//! project file's name with the instance's number; the file name is it lower-cased to `[a-z0-9-]`, and the preset's
//! `meta.name` is it as written, with a capital first letter (track `bass` gives `bass.patch.json` named "Bass").
//! `--name` names a single instance. A sound made from Init (the patch a fresh instance and the starter project
//! load) would otherwise be named "Init" and tagged `init`, so four G1 sounds would collide with each other and with
//! the factory Init (R1/S1 lane INT): the `init` tag and Init's description are dropped, and when the name is a type
//! of the tag vocabulary (`bass`, `pad`, `pluck`, `lead`, …) and the patch has no type tag, that type is added.
//! Every other byte is the plugin's. Host parameter values are not in a preset: the plugin folds them into the patch
//! when it saves (docs/session.md), so the `patch` field already holds what was heard.
//!
//! ```text
//! cargo xtask extract-presets ~/enth-g1/sounds.rpp                     # writes content/presets/<track>.patch.json
//! cargo xtask extract-presets a.rpp b.rpp --out DIR --dry-run         # lists what it would write
//! ```
//!
//! **`cargo xtask starter-project --out FILE`** writes the other end: a REAPER project with four tracks named `bass`,
//! `pad`, `pluck` and `lead`, each holding enth (CLAP) loaded with the [starter patch](STARTER_PATCH), which is Init,
//! the patch a fresh instance plays (lane AM, R1/S1): every P1 module, the extra ones switched off, gain-staged. Owner
//! build 2 starts from this project for its four named tracks. `--tracks a,b,c` and `--patch FILE` change the tracks
//! and the patch.

use std::path::{Path, PathBuf};

use crate::reaper as chunk;

const USAGE: &str = "usage: cargo xtask extract-presets <project.rpp|chain.RfxChain>... [--out DIR] [--name NAME] \
                     [--force] [--dry-run]";

/// enth's permanent CLAP id (docs/naming.md).
const CLAP_ID: &str = "dev.enth.synth";
/// enth's permanent VST3 class id, as REAPER writes it in a `<VST` line (hex, no dashes).
const VST3_CLASS_HEX: &str = "55555EADAA77C9FAF878DDDDB0B9D9BD";

/// Where presets live in the repository.
pub const PRESET_DIR: &str = "content/presets";

/// The patch a starter project loads on every track, in the plugin's own serialisation (it loads and saves back byte
/// for byte): Init, the factory preset a fresh instance plays (lane AM): every P1 module at its catalogue defaults,
/// `osc1` into `filter1` with `env1` on the amp; `osc2`, `noise1`, `comp1` and `reverb1` switched off; `env2`, `lfo1`
/// and `lfo2` unrouted; no routes, no slots, macros at 0; the master at −12 dB, so a four-note chord peaks below
/// −1 dBFS. (`preset/starter.patch.json`, the same modules with the master at 0 dB, stays as the randomizer's
/// "starter superset" reference, docs/design.md §9.)
pub const STARTER_PATCH: &str = include_str!("../../content/presets/init.patch.json");
/// The starter project's tracks: the four G1 sounds.
pub const STARTER_TRACKS: [&str; 4] = ["bass", "pad", "pluck", "lead"];

/// The plugin format of an instance.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Clap,
    Vst3,
}

impl Kind {
    fn tag(self) -> &'static str {
        match self {
            Kind::Clap => "CLAP",
            Kind::Vst3 => "VST3",
        }
    }
}

/// One enth instance found in a project file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Instance {
    pub kind: Kind,
    /// The name of the track that holds it (empty when REAPER's track has none, or outside a track).
    pub track: String,
    /// The FX's own name, when the owner renamed the instance in REAPER (else empty).
    pub fx_name: String,
    /// The `patch` field: exactly `Patch::to_json()`.
    pub patch: String,
}

/// REAPER's tokens on a project line: words, or strings in `"`, `'` or `` ` `` (REAPER picks a quote character the
/// string does not contain).
fn tokens(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = line.trim().chars().peekable();
    while let Some(&c) = chars.peek() {
        if c.is_whitespace() {
            chars.next();
            continue;
        }
        let mut token = String::new();
        if matches!(c, '"' | '\'' | '`') {
            chars.next();
            for d in chars.by_ref() {
                if d == c {
                    break;
                }
                token.push(d);
            }
        } else {
            while let Some(&d) = chars.peek() {
                if d.is_whitespace() {
                    break;
                }
                token.push(d);
                chars.next();
            }
        }
        out.push(token);
    }
    out
}

/// What an opened block is.
enum Block {
    Track,
    /// An enth instance: its format, its own name and the decoded bytes of its base64 lines so far.
    Enth(Kind, String, Vec<u8>),
    /// A block inside an enth CLAP instance (`<STATE`), whose lines are the state.
    State,
    Other,
}

/// Every enth instance in a REAPER project or FX chain, in file order.
pub fn instances(text: &str) -> Result<Vec<Instance>, String> {
    let mut stack: Vec<Block> = Vec::new();
    let mut track_names: Vec<String> = Vec::new();
    let mut found = Vec::new();
    for (number, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if let Some(rest) = line.strip_prefix('<') {
            let words = tokens(rest);
            let head = words.first().map_or("", String::as_str);
            let block = match head {
                "TRACK" => {
                    track_names.push(String::new());
                    Block::Track
                }
                // <CLAP "CLAPi: enth (enth)" dev.enth.synth "own name"
                "CLAP" if words.get(2).is_some_and(|id| id == CLAP_ID) => Block::Enth(
                    Kind::Clap,
                    words.get(3).cloned().unwrap_or_default(),
                    Vec::new(),
                ),
                // <VST "VST3i: enth (enth)" enth-plugin.vst3 0 "own name" 717012819{55555EAD…} ""
                "VST"
                    if words
                        .iter()
                        .any(|w| w.to_ascii_uppercase().contains(VST3_CLASS_HEX)) =>
                {
                    Block::Enth(
                        Kind::Vst3,
                        words.get(4).cloned().unwrap_or_default(),
                        Vec::new(),
                    )
                }
                "STATE" if matches!(stack.last(), Some(Block::Enth(Kind::Clap, ..))) => {
                    Block::State
                }
                _ => Block::Other,
            };
            stack.push(block);
            continue;
        }
        if line == ">" {
            match stack.pop() {
                Some(Block::Track) => {
                    track_names.pop();
                }
                Some(Block::Enth(kind, fx_name, bytes)) => {
                    let state = chunk::nice_plug_state(&bytes).map_err(|e| {
                        format!(
                            "line {}: an enth {} instance whose state cannot be read: {e}",
                            number + 1,
                            kind.tag()
                        )
                    })?;
                    let patch = state["fields"]["patch"]
                        .as_str()
                        .ok_or_else(|| {
                            format!(
                                "line {}: an enth {} instance with no `patch` field in its state",
                                number + 1,
                                kind.tag()
                            )
                        })?
                        .to_owned();
                    found.push(Instance {
                        kind,
                        track: track_names.last().cloned().unwrap_or_default(),
                        fx_name,
                        patch,
                    });
                }
                Some(_) => {}
                None => return Err(format!("line {}: a `>` closes no block", number + 1)),
            }
            continue;
        }
        match stack.last_mut() {
            Some(Block::Track) => {
                let words = tokens(line);
                if words.first().is_some_and(|w| w == "NAME")
                    && let Some(name) = track_names.last_mut()
                {
                    *name = words.get(1).cloned().unwrap_or_default();
                }
            }
            // A VST3 instance's lines are base64 directly in its block (REAPER's header line first, then the state,
            // then the controller's); a CLAP instance's are inside its <STATE> block. Each line is a whole number of
            // base64 groups, so they decode one by one.
            Some(Block::Enth(Kind::Vst3, _, bytes)) => {
                bytes.extend(chunk::base64_decode(line));
            }
            Some(Block::State) => {
                let len = stack.len();
                if let Some(Block::Enth(_, _, bytes)) = stack.get_mut(len - 2) {
                    bytes.extend(chunk::base64_decode(line));
                }
            }
            _ => {}
        }
    }
    Ok(found)
}

/// A file name from a free text: lower case, ASCII letters and digits, other runs as one `-`.
pub fn slug(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_matches('-').to_owned()
}

/// The file name of an instance's preset (without the suffix): [`preset_title`] as a [`slug`].
pub fn preset_name(instance: &Instance, stem: &str, index: usize) -> String {
    slug(&preset_title(instance, stem, index))
}

/// The preset's own name (`meta.name`) for an instance: its FX name, else its track's, else `<stem> <n>` (n from 1),
/// trimmed, with a capital first letter. A name with no letter or digit in it is passed over.
pub fn preset_title(instance: &Instance, stem: &str, index: usize) -> String {
    let text = [&instance.fx_name, &instance.track]
        .into_iter()
        .map(|s| s.trim())
        .find(|s| !slug(s).is_empty())
        .map_or_else(
            || {
                let stem = stem.trim();
                let stem = if slug(stem).is_empty() {
                    "preset"
                } else {
                    stem
                };
                format!("{stem} {}", index + 1)
            },
            str::to_owned,
        );
    title(&text)
}

/// `text` trimmed, with its first character in upper case ("bass" gives "Bass").
pub fn title(text: &str) -> String {
    let text = text.trim();
    let mut chars = text.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(chars).collect()
    })
}

/// Init's tag: a sound made from Init must not keep it.
const INIT_TAG: &str = "init";

/// The preset file of an extracted sound: the saved `patch` (a patch document, migrated if it is older) named `name`,
/// without the `init` tag or Init's description, and with the type tag `name` spells when it has none
/// ([module docs](self)); everything else as saved. Returns the text, exactly `Patch::to_json()`, and what would still
/// keep it out of the factory folder (the factory test's rules, in words; empty when it passes them).
pub fn preset_text(patch: &str, name: &str) -> Result<(String, Vec<String>), String> {
    use crate::preset_add::vocab;
    let loaded = enth_patch::file::load(patch).map_err(|e| format!("not a valid patch: {e}"))?;
    let mut patch = loaded.patch;
    let init = enth_patch::file::load(STARTER_PATCH)
        .map_err(|e| format!("Init does not load: {e}"))?
        .patch;
    let meta = &mut patch.meta;
    meta.name = title(name);
    meta.tags.retain(|t| !t.eq_ignore_ascii_case(INIT_TAG));
    if meta.description == init.meta.description {
        meta.description.clear();
    }
    let has_type = meta.tags.iter().any(|t| vocab::TYPES.contains(&t.as_str()));
    if !has_type {
        let slugged = slug(name);
        let mut spelled = slugged
            .split('-')
            .filter(|w| *w != INIT_TAG && vocab::TYPES.contains(w));
        if let (Some(kind), None) = (spelled.next(), spelled.next()) {
            meta.tags.insert(0, kind.to_owned());
        }
    }
    let problems = vocab::check(&patch.meta);
    Ok((patch.to_json(), problems))
}

struct Options {
    files: Vec<PathBuf>,
    out: Option<PathBuf>,
    name: Option<String>,
    force: bool,
    dry_run: bool,
}

fn parse_options(args: &[String]) -> Result<Options, String> {
    let mut options = Options {
        files: Vec::new(),
        out: None,
        name: None,
        force: false,
        dry_run: false,
    };
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--out" => {
                options.out = Some(PathBuf::from(iter.next().ok_or("--out needs a directory")?));
            }
            "--name" => options.name = Some(iter.next().ok_or("--name needs a name")?.clone()),
            "--force" => options.force = true,
            "--dry-run" => options.dry_run = true,
            other if other.starts_with('-') => {
                return Err(format!("unknown option {other}; {USAGE}"));
            }
            file => options.files.push(PathBuf::from(file)),
        }
    }
    if options.files.is_empty() {
        return Err(USAGE.to_owned());
    }
    Ok(options)
}

pub fn extract_presets(args: &[String]) -> Result<(), String> {
    let options = parse_options(args)?;
    let out = match &options.out {
        Some(dir) => dir.clone(),
        None => crate::metadata()?.workspace_root.join(PRESET_DIR),
    };
    // (file name, preset name, project, instance)
    let mut planned: Vec<(String, String, PathBuf, Instance)> = Vec::new();
    for file in &options.files {
        let text = std::fs::read_to_string(file).map_err(|e| format!("{}: {e}", file.display()))?;
        let found = instances(&text).map_err(|e| format!("{}: {e}", file.display()))?;
        if found.is_empty() {
            return Err(format!(
                "{}: no enth instance (CLAP {CLAP_ID} or VST3 {VST3_CLASS_HEX}); was the project saved after the \
                 plugin was added?",
                file.display()
            ));
        }
        let stem = file
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("preset");
        for (i, instance) in found.into_iter().enumerate() {
            let name = preset_name(&instance, stem, i);
            let title = preset_title(&instance, stem, i);
            planned.push((name, title, file.clone(), instance));
        }
    }
    if let Some(name) = &options.name {
        if planned.len() != 1 {
            return Err(format!(
                "--name names one preset, but the files hold {} enth instances",
                planned.len()
            ));
        }
        let file_name = slug(name);
        if file_name.is_empty() {
            return Err("--name needs at least one letter or digit".to_owned());
        }
        planned[0].0 = file_name;
        planned[0].1 = title(name);
    }
    let mut names: Vec<&str> = planned.iter().map(|(n, ..)| n.as_str()).collect();
    names.sort_unstable();
    if let Some([twice, _]) = names.windows(2).find(|w| w[0] == w[1]) {
        return Err(format!(
            "two instances would both be {twice:?}: name the tracks (or the FX) apart"
        ));
    }
    let mut texts = Vec::with_capacity(planned.len());
    for (name, title, _, instance) in &planned {
        let (text, problems) = preset_text(&instance.patch, title).map_err(|e| {
            format!("{name}: the saved `patch` field is not a patch document ({e})")
        })?;
        texts.push((text, problems));
    }
    let paths: Vec<PathBuf> = planned
        .iter()
        .map(|(name, ..)| out.join(format!("{name}.patch.json")))
        .collect();
    check_names_free(&out, &planned, &paths)?;
    if !options.dry_run {
        std::fs::create_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;
    }
    for (((name, title, file, instance), (text, problems)), path) in
        planned.iter().zip(&texts).zip(&paths)
    {
        let state = match std::fs::read_to_string(path) {
            Ok(old) if old == *text => "unchanged",
            Ok(_) if !options.force => {
                return Err(format!(
                    "{} exists with other content (--force replaces it)",
                    path.display()
                ));
            }
            Ok(_) => "replaced",
            Err(_) => "new",
        };
        if !options.dry_run && state != "unchanged" {
            std::fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))?;
        }
        println!(
            "{}{}: {} named {title:?} ({state}, {} bytes; {} instance, track {:?}{} in {})",
            if options.dry_run { "(dry run) " } else { "" },
            name,
            display(path),
            text.len(),
            instance.kind.tag(),
            instance.track,
            if instance.fx_name.is_empty() {
                String::new()
            } else {
                format!(", FX {:?}", instance.fx_name)
            },
            file.display()
        );
        if !problems.is_empty() {
            println!(
                "  note: not yet a factory preset: {}. Fix with `cargo xtask preset-add {} --tags TYPE,... --out {} \
                 --folder . --force` (docs/presets.md, the tag vocabulary)",
                problems.join("; "),
                display(path),
                display(&out)
            );
        }
    }
    Ok(())
}

/// Refuses a preset name another preset under `out` already has, ignoring case (the factory test's rule: no two
/// factory presets share a name). The files this run replaces do not count, and a file that does not load is skipped.
fn check_names_free(
    out: &Path,
    planned: &[(String, String, PathBuf, Instance)],
    paths: &[PathBuf],
) -> Result<(), String> {
    let mut taken = std::collections::BTreeMap::new();
    for path in crate::preset_add::preset_files(out)? {
        if paths.contains(&path) {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        if let Ok(loaded) = enth_patch::file::load(&text) {
            taken.insert(loaded.patch.meta.name.to_lowercase(), path);
        }
    }
    for (name, title, ..) in planned {
        if let Some(other) = taken.get(&title.to_lowercase()) {
            return Err(format!(
                "{name}: the name {title:?} is taken by {} (presets need names of their own, ignoring case): rename \
                 the track or the FX in REAPER, or use --name",
                display(other)
            ));
        }
    }
    Ok(())
}

/// A REAPER project text: one track per name, each holding enth's CLAP with `patch` as its state. The blocks are the
/// ones REAPER 7.82 saves (a `<CLAP "CLAPi: enth (enth)" dev.enth.synth "">` block whose `<STATE>` is base64 of the
/// plugin's stream, a `u64` length and nice-plug's state JSON, in lines of 128 characters); REAPER fills in everything
/// else a project leaves out. The state holds no host parameter values (they start at their defaults) and no `enth`
/// field (the instance settings start at theirs).
pub fn starter_project(tracks: &[&str], patch: &str) -> String {
    use std::fmt::Write as _;
    let json = format!(
        r#"{{"version":"0.0.0","params":{{}},"fields":{{"patch":{}}}}}"#,
        serde_json::Value::String(patch.to_owned())
    );
    let mut bytes = (json.len() as u64).to_le_bytes().to_vec();
    bytes.extend_from_slice(json.as_bytes());
    let b64 = chunk::base64_encode(&bytes);
    let mut text = String::from("<REAPER_PROJECT 0.1 \"7.82/linux-x86_64\" 0\n  TEMPO 120 4 4\n");
    for name in tracks {
        let quoted = if name.contains('"') {
            format!("'{name}'")
        } else {
            format!("\"{name}\"")
        };
        let _ = write!(
            text,
            "  <TRACK\n    NAME {quoted}\n    <FXCHAIN\n      SHOW 0\n      LASTSEL 0\n      DOCKED 0\n      BYPASS 0 0 0\n      \
             <CLAP \"CLAPi: enth (enth)\" {CLAP_ID} \"\"\n        CFG 4 0 0 \"\"\n        <STATE\n"
        );
        for line in b64.as_bytes().chunks(128) {
            let _ = writeln!(text, "          {}", String::from_utf8_lossy(line));
        }
        text.push_str("        >\n      >\n      FLOATPOS 0 0 0 0\n      WAK 0 0\n    >\n  >\n");
    }
    text.push_str(">\n");
    text
}

/// `cargo xtask starter-project --out FILE [--tracks a,b,…] [--patch FILE] [--force]`.
pub fn write_starter_project(args: &[String]) -> Result<(), String> {
    const USAGE: &str = "usage: cargo xtask starter-project --out FILE.rpp [--tracks bass,pad,pluck,lead] [--patch FILE] [--force]";
    let (mut out, mut tracks, mut patch_file, mut force) = (None, None, None, false);
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--out" => out = Some(PathBuf::from(iter.next().ok_or(USAGE)?)),
            "--tracks" => tracks = Some(iter.next().ok_or(USAGE)?.clone()),
            "--patch" => patch_file = Some(PathBuf::from(iter.next().ok_or(USAGE)?)),
            "--force" => force = true,
            other => return Err(format!("unknown option {other}; {USAGE}")),
        }
    }
    let out = out.ok_or(USAGE)?;
    let names: Vec<String> = match &tracks {
        Some(list) => list
            .split(',')
            .map(|t| t.trim().to_owned())
            .filter(|t| !t.is_empty())
            .collect(),
        None => STARTER_TRACKS.iter().map(|t| (*t).to_owned()).collect(),
    };
    if names.is_empty() {
        return Err(format!("--tracks names no track; {USAGE}"));
    }
    let patch = match &patch_file {
        Some(path) => {
            std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?
        }
        None => STARTER_PATCH.to_owned(),
    };
    if serde_json::from_str::<serde_json::Value>(&patch)
        .ok()
        .and_then(|v| v.get("schema").cloned())
        .is_none()
    {
        return Err("the patch is not a patch document (no `schema`)".to_owned());
    }
    if out.exists() && !force {
        return Err(format!(
            "{} exists: it may hold your sounds (--force replaces it)",
            out.display()
        ));
    }
    if let Some(dir) = out.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    std::fs::write(&out, starter_project(&refs, patch.trim_end()))
        .map_err(|e| format!("{}: {e}", out.display()))?;
    println!(
        "{}: {} tracks ({}), each with enth (CLAP) on {} ({} bytes)",
        display(&out),
        names.len(),
        names.join(", "),
        patch_file.as_ref().map_or_else(
            || "the starter patch".to_owned(),
            |p| p.display().to_string()
        ),
        patch.trim_end().len()
    );
    Ok(())
}

fn display(path: &Path) -> String {
    std::env::current_dir()
        .ok()
        .and_then(|cwd| path.strip_prefix(cwd).ok().map(Path::to_path_buf))
        .unwrap_or_else(|| path.to_path_buf())
        .display()
        .to_string()
}

#[cfg(test)]
mod tests {
    use std::fmt::Write as _;

    use super::*;

    /// The REAPER harness's fixture: the patch `patch-roundtrip` loads, which REAPER saves back byte for byte.
    const ROUNDTRIP_PATCH: &str = include_str!("../reaper/enth-roundtrip.patch.json");
    /// Projects REAPER 7.82 saved in `reaper-test`'s `patch-roundtrip` (lane INT's run, the `-a` save), one per
    /// format, unedited.
    const SAVED_CLAP: &str = include_str!("preset/roundtrip-clap.rpp");
    const SAVED_VST3: &str = include_str!("preset/roundtrip-vst3.rpp");
    /// The project `cargo xtask starter-project` writes, as REAPER 7.82 saved it in `reaper-test`'s `starter-project`
    /// (R1/S1 lane INT's run, under Xvfb), unedited: four tracks holding Init.
    const SAVED_STARTER: &str = include_str!("preset/starter-clap-saved.rpp");

    #[test]
    fn reaper_projects_give_back_the_patch_byte_for_byte() {
        for (text, kind) in [(SAVED_CLAP, Kind::Clap), (SAVED_VST3, Kind::Vst3)] {
            let found = instances(text).unwrap();
            assert_eq!(found.len(), 1, "{kind:?}");
            assert_eq!(found[0].kind, kind);
            assert_eq!(found[0].patch, ROUNDTRIP_PATCH.trim_end(), "{kind:?}");
            assert_eq!(found[0].track, "");
            assert_eq!(preset_name(&found[0], "roundtrip-a", 0), "roundtrip-a-1");
        }
    }

    /// A project holding `blocks` on tracks named `names`, in REAPER's layout, with a state chunk built the way the
    /// harness builds them.
    fn project(tracks: &[(&str, &str, &str)]) -> String {
        let mut text = String::from("<REAPER_PROJECT 0.1 \"7.82/linux-x86_64\" 0 0\n");
        for (track, fx, patch) in tracks {
            // nice-plug writes `version` first, and the decoder looks for it.
            let json = format!(
                r#"{{"version":"0.0.0","params":{{}},"fields":{{"patch":{}}}}}"#,
                serde_json::to_string(patch).unwrap()
            )
            .into_bytes();
            let mut bytes = (json.len() as u64).to_le_bytes().to_vec();
            bytes.extend_from_slice(&json);
            let b64 = chunk::base64_encode(&bytes);
            let lines: Vec<&str> = b64
                .as_bytes()
                .chunks(128)
                .map(|c| std::str::from_utf8(c).unwrap())
                .collect();
            let _ = write!(
                text,
                "  <TRACK {{0}}\n    NAME {track}\n    <FXCHAIN\n      <CLAP \"CLAPi: enth (enth)\" dev.enth.synth {fx}\n        CFG 4 0 0 \"\"\n        <STATE\n"
            );
            for l in lines {
                let _ = writeln!(text, "          {l}");
            }
            text.push_str("        >\n      >\n      <CLAP \"CLAPi: other\" com.other.synth \"\"\n        <STATE\n          AAAA\n        >\n      >\n    >\n  >\n");
        }
        text.push_str(">\n");
        text
    }

    #[test]
    fn names_come_from_the_fx_then_the_track() {
        let text = project(&[
            ("\"Fat Bass\"", "\"\"", r#"{"schema":1,"a":1}"#),
            ("Pad", "\"My Pad 2\"", r#"{"schema":1,"b":2}"#),
            ("\"\"", "\"\"", r#"{"schema":1,"c":3}"#),
        ]);
        let found = instances(&text).unwrap();
        assert_eq!(found.len(), 3, "the other plugin is skipped: {found:?}");
        let names: Vec<String> = found
            .iter()
            .enumerate()
            .map(|(i, f)| preset_name(f, "G1 sounds", i))
            .collect();
        assert_eq!(names, ["fat-bass", "my-pad-2", "g1-sounds-3"]);
        assert_eq!(found[1].patch, r#"{"schema":1,"b":2}"#);
        assert_eq!(found[0].track, "Fat Bass");
    }

    #[test]
    fn reaper_tokens_and_slugs() {
        assert_eq!(
            tokens(r#"CLAP "CLAPi: enth (enth)" dev.enth.synth 'say "hi"'"#),
            ["CLAP", "CLAPi: enth (enth)", "dev.enth.synth", "say \"hi\""]
        );
        assert_eq!(slug("  Bass — Sub (v2)! "), "bass-sub-v2");
        assert_eq!(slug("—"), "");
    }

    #[test]
    fn a_starter_project_gives_back_its_patch_on_every_track() {
        let text = starter_project(&STARTER_TRACKS, STARTER_PATCH);
        let found = instances(&text).unwrap();
        let names: Vec<String> = found
            .iter()
            .enumerate()
            .map(|(i, f)| preset_name(f, "x", i))
            .collect();
        assert_eq!(names, STARTER_TRACKS);
        assert!(
            found
                .iter()
                .all(|f| f.patch == STARTER_PATCH && f.kind == Kind::Clap)
        );
        let starter: serde_json::Value = serde_json::from_str(STARTER_PATCH).unwrap();
        assert_eq!(starter["meta"]["name"], "Init");
        assert_eq!(starter["generators"].as_array().map(Vec::len), Some(3));
        // The randomizer's starter superset holds the same modules; Init only lowers the master.
        let superset: serde_json::Value =
            serde_json::from_str(include_str!("preset/starter.patch.json")).unwrap();
        for section in ["generators", "voice_fx", "fx", "modulators", "routes"] {
            assert_eq!(starter[section], superset[section], "{section}");
        }
    }

    /// A fresh, empty temporary folder, removed when dropped.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(what: &str) -> Scratch {
            use std::sync::atomic::{AtomicUsize, Ordering};
            static N: AtomicUsize = AtomicUsize::new(0);
            let dir = std::env::temp_dir().join(format!(
                "enth-xtask-extract-{what}-{}-{}",
                std::process::id(),
                N.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Scratch(dir)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A patch document without its `meta`, for "the same sound".
    fn without_meta(text: &str) -> serde_json::Value {
        let mut value: serde_json::Value = serde_json::from_str(text).unwrap();
        value.as_object_mut().unwrap().remove("meta");
        value
    }

    fn meta(text: &str) -> enth_patch::PatchMeta {
        enth_patch::file::load(text).unwrap().patch.meta
    }

    #[test]
    fn a_sound_made_from_init_is_named_after_its_track_not_init() {
        let (text, problems) = preset_text(STARTER_PATCH, "bass").unwrap();
        let got = meta(&text);
        assert_eq!(got.name, "Bass");
        assert_eq!(
            got.tags,
            ["bass"],
            "the init tag goes, the track's type comes"
        );
        assert_eq!(got.description, "", "Init's description is not the sound's");
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(without_meta(&text), without_meta(STARTER_PATCH));
        crate::preset_add::check_text(&text).unwrap();
        // A name that is no type: named, untagged, and the factory rules say what is missing.
        let (text, problems) = preset_text(STARTER_PATCH, "  wobble thing ").unwrap();
        let got = meta(&text);
        assert_eq!((got.name.as_str(), got.tags.len()), ("Wobble thing", 0));
        assert!(
            problems.iter().any(|p| p.contains("type tag")),
            "{problems:?}"
        );
        // Two types in the name: no guess.
        let (text, _) = preset_text(STARTER_PATCH, "pad pluck").unwrap();
        assert_eq!(meta(&text).tags, Vec::<String>::new());
    }

    #[test]
    fn a_sound_with_its_own_meta_keeps_its_tags_and_description() {
        let mut patch = enth_patch::file::load(STARTER_PATCH).unwrap().patch;
        patch.meta.tags = vec!["lead".to_owned(), "bright".to_owned(), "init".to_owned()];
        "A bright lead.".clone_into(&mut patch.meta.description);
        "Mine".clone_into(&mut patch.meta.author);
        let (text, problems) = preset_text(&patch.to_json(), "bass").unwrap();
        let got = meta(&text);
        assert_eq!(got.name, "Bass");
        assert_eq!(got.tags, ["lead", "bright"], "a type tag of its own wins");
        assert_eq!(
            (got.description.as_str(), got.author.as_str()),
            ("A bright lead.", "Mine")
        );
        assert!(problems.is_empty(), "{problems:?}");
    }

    #[test]
    fn titles_follow_the_file_names() {
        let instance = |track: &str, fx: &str| Instance {
            kind: Kind::Clap,
            track: track.to_owned(),
            fx_name: fx.to_owned(),
            patch: String::new(),
        };
        assert_eq!(preset_title(&instance("bass", ""), "x", 0), "Bass");
        assert_eq!(
            preset_title(&instance("bass", "  my Pad 2 "), "x", 0),
            "My Pad 2"
        );
        assert_eq!(
            preset_title(&instance("—", ""), "G1 sounds", 2),
            "G1 sounds 3"
        );
        assert_eq!(preset_title(&instance("", ""), "—", 0), "Preset 1");
        assert_eq!(preset_name(&instance("", ""), "—", 0), "preset-1");
        assert_eq!(title("élan"), "Élan");
    }

    /// What owner build 2 extracts: REAPER's own save of a starter project gives four presets named and typed after
    /// their tracks, each Init's sound with none of Init's metadata, all ready for the factory folder.
    #[test]
    fn a_reaper_saved_starter_project_gives_four_sounds_of_their_own() {
        let found = instances(SAVED_STARTER).unwrap();
        assert_eq!(found.len(), 4);
        let mut presets = Vec::new();
        for (i, instance) in found.iter().enumerate() {
            assert_eq!(
                instance.patch, STARTER_PATCH,
                "REAPER kept Init byte for byte"
            );
            let (text, problems) =
                preset_text(&instance.patch, &preset_title(instance, "g1", i)).unwrap();
            assert!(problems.is_empty(), "{problems:?}");
            assert_eq!(without_meta(&text), without_meta(STARTER_PATCH));
            let got = meta(&text);
            assert_eq!(got.description, "");
            presets.push((preset_name(instance, "g1", i), got.name, got.tags));
        }
        let want: Vec<(String, String, Vec<String>)> = STARTER_TRACKS
            .iter()
            .map(|t| ((*t).to_owned(), title(t), vec![(*t).to_owned()]))
            .collect();
        assert_eq!(presets, want);
    }

    /// The owner's case: the four tracks of a starter project, extracted beside the factory Init, give four presets of
    /// their own (before lane INT's fix every one was "Init", which the factory test refuses).
    #[test]
    fn a_starter_project_extracts_beside_init_without_a_collision() {
        let dir = Scratch::new("starter");
        let presets = dir.0.join("presets");
        std::fs::create_dir_all(&presets).unwrap();
        std::fs::write(presets.join("init.patch.json"), STARTER_PATCH).unwrap();
        let project = dir.0.join("g1 sounds.rpp");
        std::fs::write(&project, starter_project(&STARTER_TRACKS, STARTER_PATCH)).unwrap();
        let args = |extra: &[&str]| -> Vec<String> {
            [
                project.to_str().unwrap(),
                "--out",
                presets.to_str().unwrap(),
            ]
            .iter()
            .chain(extra)
            .map(|s| (*s).to_owned())
            .collect()
        };
        extract_presets(&args(&[])).unwrap();
        let mut names = Vec::new();
        for track in STARTER_TRACKS {
            let text =
                std::fs::read_to_string(presets.join(format!("{track}.patch.json"))).unwrap();
            let got = meta(&text);
            assert_eq!(got.tags, [track]);
            assert_eq!(without_meta(&text), without_meta(STARTER_PATCH));
            crate::preset_add::check_text(&text).unwrap();
            names.push(got.name);
        }
        assert_eq!(names, ["Bass", "Pad", "Pluck", "Lead"]);
        // Again: unchanged, no error.
        extract_presets(&args(&[])).unwrap();
        // A preset elsewhere in the folder already named "Pad" stops it, before anything is written.
        std::fs::create_dir_all(presets.join("pad")).unwrap();
        let (taken, _) = preset_text(STARTER_PATCH, "Pad").unwrap();
        std::fs::write(presets.join("pad/other.patch.json"), taken).unwrap();
        let err = extract_presets(&args(&[])).unwrap_err();
        assert!(err.contains("\"Pad\" is taken"), "{err}");
    }

    #[test]
    fn a_broken_state_is_an_error_not_a_skip() {
        let text =
            "<TRACK\n<CLAP \"CLAPi: enth (enth)\" dev.enth.synth \"\"\n<STATE\nAAAA\n>\n>\n>\n";
        assert!(instances(text).is_err());
    }
}
