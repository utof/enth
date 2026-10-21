//! **Owner: lane INT2** (R2 integration). Pre-declared by lane S0l (R2) as a driver of `enth-lesson-tool gr2`; INT2
//! read the files without the tool's dependencies (as `gr1-report` reads round logs), so the work is here and the
//! tool's `gr2` is gone.
//!
//! `cargo xtask gr2-report <dir>...`: the GR2 numbers (docs/learn.md, "GR2") from the files each person sends back,
//! per person: the pre-, mid- and post-test items (challenge codes played as Recreate rounds, read from their round
//! log), each family better, unchanged or worse, the lessons completed (their progress file), the per-lesson votes and
//! the "what did you learn?" notes with their coding (their answers file), and the proposed criteria. Local files
//! only: no network code, no telemetry. docs/r2-owner.md is the protocol and the reader's guide; the tables of record
//! are docs/r2-gates.md.
//!
//! | Module | Does |
//! | --- | --- |
//! | `input` | reads the forms (`enth.gr2-forms/1`), an answers file (`enth.gr2-answers/1`), a progress file, `lesson.toml` and `pack.toml` from text |
//! | `numbers` | the items, the families' changes, the lessons, the criteria: the `enth.gr2-report/1` value |
//! | `render` | the readable tables, and `--notes` for the coder |
//!
//! Round logs go through `gr1-report`'s reader (`gr1_report::logs`), so restored rounds, newer and older builds and the
//! privacy lint behave as they do for GR1.

mod input;
mod numbers;
mod render;

#[cfg(test)]
mod fixtures;

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use crate::gr1_report::logs::{self, Source};
use crate::gr1_report::privacy;
use crate::gr1_report::schema::Severity;
use input::{Answers, Forms, LessonInfo, PackInfo};
use numbers::{Inputs, Person, Thresholds};

const USAGE: &str = "usage: cargo xtask gr2-report [options] <dir>...

  Reads what each GR2 participant sent back and prints the GR2 numbers per person: the pre-, mid- and post-test
  items, each family better, unchanged or worse, tour lessons completed, the per-lesson votes and the \"what did you
  learn?\" notes, and the proposed criteria. Local files only; nothing is sent anywhere. docs/r2-owner.md explains it.

  <dir>   a folder of files named by the person's anonymous id (p1 is you by default):
            <id>.gr2.toml       their answers (enth.gr2-answers/1; template content/gr2/answers-template.toml)
            <id>.progress.json  their Documents/enth/Learn/progress.json
            <id>.rounds.jsonl   their Documents/enth/Rounds/round-log.jsonl (the test codes are read from it)
          A person needs an answers file; the other two are optional (a tour-only person sends no round log).

options:
  --forms FILE            the test forms (default content/gr2/forms.toml)
  --lessons DIR           the lesson packs, for titles, objectives, steps and the tour (default content/lessons)
  --owner ID              your id (default p1)
  --lessons-named N       criterion 1: lessons with a named new thing per person (default 3)
  --families-improved N   criterion 2: families the median participant improves on (default 2 of the 6)
  --json                  print the report as JSON (enth.gr2-report/1)
  --notes                 print every \"what did you learn?\" note beside its lesson's objective, for coding
  --validate              only check the files; exit 1 on an error or on possible personal data
  --strict                print the report, then exit 1 on an error or on possible personal data
  -h, --help              this text";

/// What the command line asked for.
#[derive(Debug, PartialEq)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "independent command-line flags, not a state machine"
)]
struct Options {
    dirs: Vec<PathBuf>,
    forms: PathBuf,
    lessons: PathBuf,
    owner: String,
    thresholds: Thresholds,
    json: bool,
    notes: bool,
    validate: bool,
    strict: bool,
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn parse(args: &[String]) -> Result<Option<Options>, String> {
    let mut o = Options {
        dirs: Vec::new(),
        forms: repo_root().join("content/gr2/forms.toml"),
        lessons: repo_root().join("content/lessons"),
        owner: "p1".to_owned(),
        thresholds: Thresholds::default(),
        json: false,
        notes: false,
        validate: false,
        strict: false,
    };
    let mut it = args.iter();
    let number = |v: Option<&String>, flag: &str| -> Result<usize, String> {
        v.ok_or(format!("{flag} needs a number"))?
            .parse()
            .map_err(|_| format!("{flag} needs a whole number"))
    };
    while let Some(a) = it.next() {
        match a.as_str() {
            "-h" | "--help" => return Ok(None),
            "--forms" => o.forms = PathBuf::from(it.next().ok_or("--forms needs a file")?),
            "--lessons" => o.lessons = PathBuf::from(it.next().ok_or("--lessons needs a folder")?),
            "--owner" => {
                o.owner.clone_from(it.next().ok_or("--owner needs an id")?);
                if !input::valid_id(&o.owner) {
                    return Err(format!("--owner {}: not a short anonymous id", o.owner));
                }
            }
            "--lessons-named" => o.thresholds.lessons_named = number(it.next(), a)?,
            "--families-improved" => o.thresholds.families_improved = number(it.next(), a)?,
            "--json" => o.json = true,
            "--notes" => o.notes = true,
            "--validate" => o.validate = true,
            "--strict" => o.strict = true,
            flag if flag.starts_with('-') => {
                return Err(format!("gr2-report: unknown option {flag}\n{USAGE}"));
            }
            dir => o.dirs.push(PathBuf::from(dir)),
        }
    }
    if o.dirs.is_empty() {
        return Err(format!("gr2-report: no folder given\n{USAGE}"));
    }
    Ok(Some(o))
}

/// A finding while reading: where (a file's base name, never its path) and what.
struct Finding {
    severity: Severity,
    place: String,
    message: String,
}

/// Everything read from disk.
struct Loaded {
    forms: Forms,
    lessons: BTreeMap<String, LessonInfo>,
    tour: Vec<String>,
    people: Vec<Person>,
    rounds: logs::Loaded,
    findings: Vec<Finding>,
    privacy: Vec<Finding>,
    files: usize,
}

fn base_name(path: &Path) -> String {
    path.file_name()
        .map_or_else(|| "?".to_owned(), |n| n.to_string_lossy().into_owned())
}

/// The lessons under `dir` (`<pack>/pack.toml`, `<pack>/lessons/<id>/lesson.toml`) and the tour track.
fn load_lessons(
    dir: &Path,
    findings: &mut Vec<Finding>,
) -> Result<(BTreeMap<String, LessonInfo>, Vec<String>), String> {
    let mut packs: Vec<PathBuf> = fs::read_dir(dir)
        .map_err(|e| format!("cannot list the lessons folder {}: {e}", dir.display()))?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.join("pack.toml").is_file())
        .collect();
    packs.sort();
    let mut lessons = BTreeMap::new();
    let mut tour: Option<Vec<String>> = None;
    for pack_dir in packs {
        let place = format!("{}/pack.toml", base_name(&pack_dir));
        let text =
            fs::read_to_string(pack_dir.join("pack.toml")).map_err(|e| format!("{place}: {e}"))?;
        let pack = PackInfo::from_toml(&text).map_err(|e| format!("{place}: {e}"))?;
        if let Some(t) = pack.tracks.get("tour") {
            if tour.is_some() {
                findings.push(Finding {
                    severity: Severity::Warn,
                    place: place.clone(),
                    message: "a second `tour` track; the first one read is used".to_owned(),
                });
            } else {
                tour = Some(t.clone());
            }
        }
        for id in &pack.lessons {
            let path = pack_dir.join("lessons").join(id).join("lesson.toml");
            let lplace = format!("{}/lessons/{id}/lesson.toml", pack.id);
            match fs::read_to_string(&path)
                .map_err(|e| e.to_string())
                .and_then(|t| LessonInfo::from_toml(&t))
            {
                Ok(info) => {
                    lessons.insert(format!("{}/{id}", pack.id), info);
                }
                Err(e) => findings.push(Finding {
                    severity: Severity::Warn,
                    place: lplace,
                    message: e,
                }),
            }
        }
    }
    let tour = tour.ok_or_else(|| format!("no pack under {} has a `tour` track", dir.display()))?;
    Ok((lessons, tour))
}

/// One person's three files, by kind.
#[derive(Default)]
struct Files {
    answers: Option<PathBuf>,
    progress: Option<PathBuf>,
    rounds: Option<PathBuf>,
}

fn load(o: &Options) -> Result<Loaded, String> {
    let mut findings = Vec::new();
    let mut privacy_found = Vec::new();
    let forms_text = fs::read_to_string(&o.forms)
        .map_err(|e| format!("cannot read the forms {}: {e}", o.forms.display()))?;
    let forms = Forms::from_toml(&forms_text)?;
    let (lessons, tour) = load_lessons(&o.lessons, &mut findings)?;

    let mut by_id: BTreeMap<String, Files> = BTreeMap::new();
    let mut files = 0;
    for dir in &o.dirs {
        let mut entries: Vec<PathBuf> = fs::read_dir(dir)
            .map_err(|e| format!("cannot list {}: {e}", dir.display()))?
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.is_file())
            .collect();
        entries.sort();
        for path in entries {
            let name = base_name(&path);
            let Some((id, kind)) = name.split_once('.') else {
                continue;
            };
            let slot: fn(&mut Files) -> &mut Option<PathBuf> = match kind {
                "gr2.toml" => |f| &mut f.answers,
                "progress.json" => |f| &mut f.progress,
                "rounds.jsonl" => |f| &mut f.rounds,
                _ => {
                    findings.push(Finding {
                        severity: Severity::Note,
                        place: name.clone(),
                        message:
                            "not <id>.gr2.toml, <id>.progress.json or <id>.rounds.jsonl: ignored"
                                .to_owned(),
                    });
                    continue;
                }
            };
            if !input::valid_id(id) {
                findings.push(Finding {
                    severity: Severity::Error,
                    place: name.clone(),
                    message: "the file name does not start with a short anonymous id (p2, p3, …): skipped".to_owned(),
                });
                continue;
            }
            let entry = by_id.entry(id.to_owned()).or_default();
            let target = slot(entry);
            if target.is_some() {
                findings.push(Finding {
                    severity: Severity::Error,
                    place: name.clone(),
                    message: format!("a second file of this kind for {id} (two folders?): skipped"),
                });
                continue;
            }
            *target = Some(path);
            files += 1;
        }
    }

    let mut people = Vec::new();
    let mut sources = Vec::new();
    for (id, f) in &by_id {
        if let Some(path) = &f.rounds {
            let text = fs::read_to_string(path).map_err(|e| format!("{}: {e}", base_name(path)))?;
            sources.push(Source {
                player: id.clone(),
                label: base_name(path),
                text,
            });
        }
        let Some(apath) = &f.answers else {
            findings.push(Finding {
                severity: Severity::Warn,
                place: id.clone(),
                message: "no answers file (<id>.gr2.toml): this person is not counted".to_owned(),
            });
            continue;
        };
        let aname = base_name(apath);
        let text = fs::read_to_string(apath).map_err(|e| format!("{aname}: {e}"))?;
        for (i, line) in text.lines().enumerate() {
            for kind in privacy::scan(line) {
                privacy_found.push(Finding {
                    severity: Severity::Warn,
                    place: format!("{aname}:{}", i + 1),
                    message: format!("looks like {kind}"),
                });
            }
        }
        let answers = match Answers::from_toml(&text) {
            Ok(a) if a.id == *id => a,
            Ok(a) => {
                findings.push(Finding {
                    severity: Severity::Error,
                    place: aname,
                    message: format!("its id is `{}`, its file name says `{id}`: skipped", a.id),
                });
                continue;
            }
            Err(e) => {
                findings.push(Finding {
                    severity: Severity::Error,
                    place: aname,
                    message: format!("{e}: skipped"),
                });
                continue;
            }
        };
        let progress = match &f.progress {
            None => None,
            Some(path) => {
                let pname = base_name(path);
                let text = fs::read_to_string(path).map_err(|e| format!("{pname}: {e}"))?;
                match input::progress_from_json(&text) {
                    Ok(p) => Some(p),
                    Err(e) => {
                        findings.push(Finding {
                            severity: Severity::Error,
                            place: pname,
                            message: format!("{e}: not read"),
                        });
                        None
                    }
                }
            }
        };
        people.push(Person { answers, progress });
    }
    let rounds = logs::load(&sources);
    for p in &rounds.problems {
        if p.severity >= Severity::Warn {
            findings.push(Finding {
                severity: p.severity,
                place: p.place.clone(),
                message: p.message.clone(),
            });
        }
    }
    for p in &rounds.privacy {
        privacy_found.push(Finding {
            severity: p.severity,
            place: p.place.clone(),
            message: p.message.clone(),
        });
    }
    Ok(Loaded {
        forms,
        lessons,
        tour,
        people,
        rounds,
        findings,
        privacy: privacy_found,
        files,
    })
}

fn header(l: &Loaded) -> String {
    let mut out = String::new();
    let count = |s: Severity| l.findings.iter().filter(|f| f.severity == s).count();
    let _ = writeln!(
        out,
        "Read {} file(s): {} people with answers, {} progress file(s), {} round log(s) ({} lines); {} error(s), {} \
         warning(s), {} note(s).",
        l.files,
        l.people.len(),
        l.people.iter().filter(|p| p.progress.is_some()).count(),
        l.rounds.stats.files,
        l.rounds.stats.lines,
        count(Severity::Error),
        count(Severity::Warn),
        count(Severity::Note)
            + l.rounds
                .problems
                .iter()
                .filter(|p| p.severity == Severity::Note)
                .count(),
    );
    if l.privacy.is_empty() {
        out.push_str("Privacy lint: clean (no email address or home-folder path found)\n");
    } else {
        let _ = writeln!(
            out,
            "Privacy lint: {} finding(s); check these places before sharing anything:",
            l.privacy.len()
        );
        for p in &l.privacy {
            let _ = writeln!(out, "  {}: {}", p.place, p.message);
        }
    }
    for f in l.findings.iter().filter(|f| f.severity >= Severity::Warn) {
        let _ = writeln!(out, "  {} {}: {}", f.severity.name(), f.place, f.message);
    }
    out
}

/// `cargo xtask gr2-report`.
#[allow(clippy::print_stdout, reason = "the report goes to stdout")]
pub fn gr2_report(args: &[String]) -> Result<(), String> {
    let Some(o) = parse(args)? else {
        println!("{USAGE}");
        return Ok(());
    };
    let l = load(&o)?;
    let errors = l.findings.iter().any(|f| f.severity == Severity::Error);
    let bad = errors || !l.privacy.is_empty();
    if o.validate {
        print!("{}", header(&l));
        return if bad {
            Err("gr2-report --validate: fix the errors or the personal data named above".to_owned())
        } else {
            println!("gr2-report --validate: the files read cleanly");
            Ok(())
        };
    }
    let report = numbers::compute(&Inputs {
        forms: &l.forms,
        lessons: &l.lessons,
        tour: &l.tour,
        people: &l.people,
        rounds: &l.rounds,
        owner: &o.owner,
        thresholds: o.thresholds,
    });
    if o.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?
        );
    } else if o.notes {
        print!("{}", render::render_notes(&report));
    } else {
        print!("{}", render::render(&report, &header(&l)));
    }
    if o.strict && bad {
        return Err("gr2-report --strict: errors or possible personal data (above)".to_owned());
    }
    // The verdicts are printed, never an exit status: GR2 is the owner's call.
    Ok(())
}
