//! `cargo xtask affected [--base <rev>]`: the workspace crates a change touches, plus every crate that depends on
//! one of them (normal, dev and build dependencies), one package name per line on stdout. `scripts/gate.sh lane`
//! tests exactly these.
//!
//! The change is the working tree against the merge base of `HEAD` and the base revision (default `main`, else
//! `origin/main`): committed work, uncommitted edits and untracked files all count. A file inside a crate's
//! directory touches that crate. A change to the workspace's build configuration (the root manifest, the lock
//! file, the toolchain, the lint and tool configs) or to a file outside every crate that is not known to be
//! documentation or tooling (for example `content/`) touches every crate: unknown means everything.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::process::Command;

/// A workspace member as the closure sees it.
#[derive(Debug)]
struct Member {
    name: String,
    /// Directory relative to the workspace root, with `/` separators; empty for a crate at the root.
    dir: String,
    /// Names of the workspace members this one depends on.
    deps: Vec<String>,
}

/// Paths (files or directory prefixes) that cannot change what a Rust crate builds or tests.
const INERT: &[&str] = &[
    "docs/",
    "_local/",
    ".github/",
    "scripts/",
    "AGENTS.md",
    "CLAUDE.md",
    "README.md",
    "LICENSE",
    "THIRD_PARTY_NOTICES.md",
    ".gitignore",
    ".gitattributes",
];

pub fn affected(args: &[String]) -> Result<(), String> {
    let base = match args {
        [] => None,
        [flag, rev] if flag == "--base" => Some(rev.clone()),
        _ => return Err("usage: cargo xtask affected [--base <rev>]".to_owned()),
    };
    let meta = crate::metadata()?;
    let names: BTreeSet<&str> = meta
        .packages
        .iter()
        .filter(|p| meta.workspace_members.contains(&p.id))
        .map(|p| p.name.as_str())
        .collect();
    let members: Vec<Member> = meta
        .packages
        .iter()
        .filter(|p| meta.workspace_members.contains(&p.id))
        .map(|p| Member {
            name: p.name.clone(),
            dir: p
                .manifest_path
                .parent()
                .and_then(|dir| dir.strip_prefix(&meta.workspace_root).ok())
                .map(|rel| rel.to_string_lossy().replace('\\', "/"))
                .unwrap_or_default(),
            deps: p
                .dependencies
                .iter()
                .filter(|d| d.path.is_some() && names.contains(d.name.as_str()))
                .map(|d| d.name.clone())
                .collect(),
        })
        .collect();
    let changed = changed_files(&meta.workspace_root, base.as_deref())?;
    let result = compute(&changed, &members);
    eprintln!(
        "affected: {} changed file(s) touch {} of {} crates",
        changed.len(),
        result.len(),
        members.len()
    );
    for name in result {
        println!("{name}");
    }
    Ok(())
}

/// The files that differ from the merge base, relative to the workspace root.
fn changed_files(root: &Path, base: Option<&str>) -> Result<Vec<String>, String> {
    let merge_base = merge_base(root, base)?;
    // `--no-renames`: a moved file counts at both its old and its new path.
    let tracked = git(root, &["diff", "--name-only", "--no-renames", &merge_base])?;
    let untracked = git(root, &["ls-files", "--others", "--exclude-standard"])?;
    let mut files: Vec<String> = tracked
        .lines()
        .chain(untracked.lines())
        .map(str::to_owned)
        .collect();
    files.sort();
    files.dedup();
    Ok(files)
}

fn merge_base(root: &Path, base: Option<&str>) -> Result<String, String> {
    let candidates: Vec<&str> = match base {
        Some(rev) => vec![rev],
        None => vec!["main", "origin/main"],
    };
    for rev in &candidates {
        if let Ok(out) = git(root, &["merge-base", "HEAD", rev]) {
            return Ok(out.trim().to_owned());
        }
    }
    Err(format!(
        "no merge base of HEAD with {}: pass --base <rev>",
        candidates.join(" or ")
    ))
}

fn git(root: &Path, args: &[&str]) -> Result<String, String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["-c", "core.quotepath=false"])
        .args(args)
        .output()
        .map_err(|e| format!("cannot run git: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(format!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

/// The crates touched by `changed` and everything that depends on them, sorted.
fn compute(changed: &[String], members: &[Member]) -> Vec<String> {
    let mut touched: BTreeSet<&str> = BTreeSet::new();
    for path in changed {
        if INERT
            .iter()
            .any(|p| path == p || (p.ends_with('/') && path.starts_with(p)))
        {
            continue;
        }
        // The crate with the longest directory prefix owns the file.
        let owner = members
            .iter()
            .filter(|m| !m.dir.is_empty() && path.starts_with(&format!("{}/", m.dir)))
            .max_by_key(|m| m.dir.len());
        match owner {
            Some(m) => {
                touched.insert(m.name.as_str());
            }
            None => {
                return members
                    .iter()
                    .map(|m| m.name.clone())
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect();
            }
        }
    }
    // Reverse-dependency closure.
    let mut dependants: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for m in members {
        for dep in &m.deps {
            dependants
                .entry(dep.as_str())
                .or_default()
                .push(m.name.as_str());
        }
    }
    let mut result: BTreeSet<&str> = BTreeSet::new();
    let mut queue: Vec<&str> = touched.into_iter().collect();
    while let Some(name) = queue.pop() {
        if result.insert(name) {
            queue.extend(dependants.get(name).into_iter().flatten());
        }
    }
    result.into_iter().map(str::to_owned).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn member(name: &str, dir: &str, deps: &[&str]) -> Member {
        Member {
            name: name.to_owned(),
            dir: dir.to_owned(),
            deps: deps.iter().map(|d| (*d).to_owned()).collect(),
        }
    }

    fn workspace() -> Vec<Member> {
        vec![
            member("dsp", "crates/dsp", &[]),
            member("engine", "crates/engine", &["dsp"]),
            member("session", "crates/session", &["engine"]),
            member("ui", "crates/ui", &["session"]),
            member("plugin", "crates/plugin", &["session", "ui"]),
            member("tool", "tools/tool", &[]),
            member("xtask", "xtask", &[]),
        ]
    }

    fn changed(paths: &[&str]) -> Vec<String> {
        compute(
            &paths.iter().map(|p| (*p).to_owned()).collect::<Vec<_>>(),
            &workspace(),
        )
    }

    #[test]
    fn a_change_touches_its_crate_and_every_dependant() {
        assert_eq!(
            changed(&["crates/session/src/lib.rs"]),
            ["plugin", "session", "ui"]
        );
        assert_eq!(
            changed(&["crates/dsp/Cargo.toml"]),
            ["dsp", "engine", "plugin", "session", "ui"]
        );
        assert_eq!(changed(&["crates/plugin/src/lib.rs"]), ["plugin"]);
    }

    #[test]
    fn leaf_tools_touch_only_themselves() {
        assert_eq!(
            changed(&["tools/tool/src/main.rs", "xtask/layers.toml"]),
            ["tool", "xtask"]
        );
    }

    #[test]
    fn documentation_and_scripts_touch_nothing() {
        assert_eq!(
            changed(&[
                "docs/design.md",
                "scripts/gate.sh",
                ".github/workflows/ci.yml",
                "AGENTS.md"
            ]),
            Vec::<String>::new()
        );
        assert_eq!(changed(&[]), Vec::<String>::new());
    }

    #[test]
    fn build_configuration_and_unknown_files_touch_everything() {
        for path in [
            "Cargo.toml",
            "Cargo.lock",
            "rust-toolchain.toml",
            "clippy.toml",
            ".cargo/config.toml",
            "content/presets/a.json",
        ] {
            assert_eq!(changed(&[path]).len(), 7, "{path}");
        }
    }

    #[test]
    fn a_nested_crate_owns_its_files() {
        let mut members = workspace();
        members.push(member("golden", "crates/engine/golden", &["engine"]));
        let got = compute(&["crates/engine/golden/src/lib.rs".to_owned()], &members);
        assert_eq!(got, ["golden"]);
    }
}
