//! **Owner: lane LCK.** Pre-declared by lane S0l (R2).
//!
//! `coverage [--all] [PACK...]` (`cargo xtask lesson-coverage`): GR2's mechanical half (docs/learn.md, "GR2"). Every P1
//! module (the ten parameter tables: the seven module kinds, `voice`, `master` and the macro knob) has a lesson and a
//! non-patch check (a sound check, a Recreate round, or an ear question `sound/1` can measure: Q6), and every
//! Basic-tier parameter is taught. **It fails on a hole.** "E only" (the ear question is the only non-patch check) and
//! "taught, not checked" (Q9: no non-patch check reaches a taught parameter; ear questions `sound/1` cannot compare
//! count as none) are listed apart, for the owner to accept at GR2. The computation is `enth_learn::coverage`; this
//! is the command around it.
//!
//! The packs are every pack in `content/lessons/` but `example` (a layout example, not shipped), or the packs named;
//! `--all` includes `example`.

use std::collections::BTreeSet;

use enth_learn::Pack;
use enth_learn::coverage::{Coverage, letters};

use crate::source;

/// Packs left out unless named or `--all`: layout examples, never shipped.
const NOT_CURRICULUM: [&str; 1] = ["example"];

/// `coverage`.
pub fn run(args: &[String]) -> Result<(), String> {
    let mut all = false;
    let mut names = Vec::new();
    for a in args {
        match a.as_str() {
            "--all" => all = true,
            flag if flag.starts_with("--") => {
                return Err(format!(
                    "coverage: unknown option {flag}\nusage: coverage [--all] [<pack>...]"
                ));
            }
            _ => names.push(a.clone()),
        }
    }
    let root = source::repo_root();
    let mut paths = source::resolve(&root, &names)?;
    if names.is_empty() && !all {
        paths.retain(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_none_or(|n| !NOT_CURRICULUM.contains(&n))
        });
    }
    let mut packs: Vec<Pack> = Vec::new();
    let mut unreadable = Vec::new();
    for path in &paths {
        let read = source::read(path)
            .and_then(|s| Pack::from_files(&s.files).map_err(|e| vec![source::learn_error(&e)]));
        match read {
            Ok(p) => packs.push(p),
            Err(problems) => {
                unreadable.push(format!("{}: {}", path.display(), problems.join("; ")));
            }
        }
    }
    if !unreadable.is_empty() {
        return Err(format!(
            "lesson-coverage: {} pack(s) do not read (run `cargo xtask lesson-check`):\n  {}",
            unreadable.len(),
            unreadable.join("\n  ")
        ));
    }
    let refs: Vec<&Pack> = packs.iter().collect();
    let cov = Coverage::of(&refs);
    print(&cov, &packs);
    let holes = cov.holes();
    if holes.is_empty() {
        println!("lesson-coverage: no hole");
        Ok(())
    } else {
        println!("holes ({}):", holes.len());
        for h in &holes {
            println!("  {h}");
        }
        Err(format!("lesson-coverage: {} hole(s)", holes.len()))
    }
}

fn print(cov: &Coverage, packs: &[Pack]) {
    let ids: Vec<String> = packs.iter().map(|p| p.id().to_string()).collect();
    println!(
        "lesson-coverage: {} pack(s) ({}), {} lesson(s)",
        packs.len(),
        if ids.is_empty() {
            "none".to_owned()
        } else {
            ids.join(", ")
        },
        cov.lessons
    );
    println!(
        "{:<8} {:>7}  {:<9} taught (Basic)",
        "table", "lessons", "checks"
    );
    for g in &cov.groups {
        let taught = g.params.iter().filter(|p| !p.taught_in.is_empty()).count();
        let basic: Vec<_> = g
            .params
            .iter()
            .filter(|p| p.tier == enth_params::Tier::Basic)
            .collect();
        let basic_taught = basic.iter().filter(|p| !p.taught_in.is_empty()).count();
        println!(
            "{:<8} {:>7}  {:<9} {taught}/{} ({basic_taught}/{})",
            g.group.key(),
            g.lessons.len(),
            letters(&g.kinds),
            g.params.len(),
            basic.len()
        );
    }
    println!();
    println!(
        "{:<22} {:<12} {:<9} taught in",
        "parameter", "tier", "checks"
    );
    for p in cov.params() {
        let lessons: Vec<String> = p.taught_in.iter().map(ToString::to_string).collect();
        println!(
            "{:<22} {:<12} {:<9} {}",
            p.key,
            p.tier.name(),
            letters(&p.kinds),
            if lessons.is_empty() {
                "-".to_owned()
            } else {
                lessons.join(", ")
            }
        );
    }
    println!();
    let list = |label: &str, items: Vec<String>| {
        println!(
            "{label} ({}): {}",
            items.len(),
            if items.is_empty() {
                "none".to_owned()
            } else {
                items.join(", ")
            }
        );
    };
    list(
        "E only, tables",
        cov.e_only_groups()
            .iter()
            .map(ToString::to_string)
            .collect(),
    );
    list(
        "E only, parameters",
        cov.e_only_params().iter().map(|p| p.key.clone()).collect(),
    );
    list(
        "taught, not checked (Q9: for the owner at GR2)",
        cov.taught_not_checked()
            .iter()
            .map(|p| p.key.clone())
            .collect(),
    );
    let basic_holes: BTreeSet<String> = cov
        .untaught()
        .iter()
        .filter(|p| p.tier == enth_params::Tier::Basic)
        .map(|p| p.key.clone())
        .collect();
    list(
        "not taught, Intermediate and Advanced",
        cov.untaught()
            .iter()
            .filter(|p| !basic_holes.contains(&p.key))
            .map(|p| p.key.clone())
            .collect(),
    );
    list(
        "ear questions sound/1 cannot compare (counted as no check; a listening review decides)",
        cov.unmeasured.iter().map(ToString::to_string).collect(),
    );
}
