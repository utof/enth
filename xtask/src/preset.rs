//! `cargo xtask extract-presets`: turn sounds saved in REAPER into preset files (P1b lane INT; owner build 2).
//!
//! The owner makes a sound in the editor and saves the REAPER project. Every enth instance in a project file (`.rpp`,
//! or an FX chain `.RfxChain`, which uses the same blocks) carries the plugin's saved state: base64 around nice-plug's
//! JSON (`{"version","params","fields"}`), whose `patch` field is exactly `Patch::to_json()` (docs/plugin.md,
//! "State"). This command finds those instances (CLAP id `dev.enth.synth`, VST3 class id `55555EAD…`), decodes the
//! state with the REAPER harness's chunk code ([`crate::reaper`]'s `chunk.rs`) and writes each `patch` field, byte for
//! byte, to `<out>/<name>.patch.json`. Such a file is a patch document like any other: `enth describe`, `enth render`
//! and the test host's `--state` read it, and the plugin loads it and saves it back unchanged.
//!
//! The name is the FX's own name if the owner renamed the instance in REAPER, else the track's name, else the
//! project file's name with the instance's number; it is lower-cased to `[a-z0-9-]`. `--name` names a single
//! instance. Host parameter values are not in a preset: the plugin folds them into the patch when it saves
//! (docs/session.md), so the `patch` field already holds what was heard.
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

/// The preset name of an instance: its FX name, else its track's, else `<stem>-<n>` (n from 1).
pub fn preset_name(instance: &Instance, stem: &str, index: usize) -> String {
    [&instance.fx_name, &instance.track]
        .into_iter()
        .map(|s| slug(s))
        .find(|s| !s.is_empty())
        .unwrap_or_else(|| {
            let stem = slug(stem);
            let stem = if stem.is_empty() {
                "preset".to_owned()
            } else {
                stem
            };
            format!("{stem}-{}", index + 1)
        })
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
    let mut planned: Vec<(String, PathBuf, Instance)> = Vec::new();
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
            planned.push((name, file.clone(), instance));
        }
    }
    if let Some(name) = &options.name {
        if planned.len() != 1 {
            return Err(format!(
                "--name names one preset, but the files hold {} enth instances",
                planned.len()
            ));
        }
        let name = slug(name);
        if name.is_empty() {
            return Err("--name needs at least one letter or digit".to_owned());
        }
        planned[0].0 = name;
    }
    let mut names: Vec<&str> = planned.iter().map(|(n, ..)| n.as_str()).collect();
    names.sort_unstable();
    if let Some([twice, _]) = names.windows(2).find(|w| w[0] == w[1]) {
        return Err(format!(
            "two instances would both be {twice:?}: name the tracks (or the FX) apart"
        ));
    }
    for (name, _, instance) in &planned {
        if serde_json::from_str::<serde_json::Value>(&instance.patch)
            .ok()
            .and_then(|v| v.get("schema").cloned())
            .is_none()
        {
            return Err(format!(
                "{name}: the saved `patch` field is not a patch document"
            ));
        }
    }
    if !options.dry_run {
        std::fs::create_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;
    }
    for (name, file, instance) in &planned {
        let path = out.join(format!("{name}.patch.json"));
        let state = match std::fs::read_to_string(&path) {
            Ok(old) if old == instance.patch => "unchanged",
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
            std::fs::write(&path, &instance.patch)
                .map_err(|e| format!("{}: {e}", path.display()))?;
        }
        println!(
            "{}{}: {} ({state}, {} bytes; {} instance, track {:?}{} in {})",
            if options.dry_run { "(dry run) " } else { "" },
            name,
            display(&path),
            instance.patch.len(),
            instance.kind.tag(),
            instance.track,
            if instance.fx_name.is_empty() {
                String::new()
            } else {
                format!(", FX {:?}", instance.fx_name)
            },
            file.display()
        );
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

    #[test]
    fn a_broken_state_is_an_error_not_a_skip() {
        let text =
            "<TRACK\n<CLAP \"CLAPi: enth (enth)\" dev.enth.synth \"\"\n<STATE\nAAAA\n>\n>\n>\n";
        assert!(instances(text).is_err());
    }
}
