//! **Owner: lane GK** (the GR1 kit). `cargo xtask gr1-report <round logs>...`: read the friends' and the owner's round
//! logs (JSONL, `enth.round-log/1`: code, patch, scores, time, assists, the blind rating, the agreement vote,
//! disputes) and print the GR1 numbers: rounds played, pass rates by level, time to pass, the assisted share, the
//! agreement distribution, the blind rating against the score, the disputes and "want more", with the agent baseline
//! beside the no-op and CMA-ES floors (docs/phase-2.md). `docs/gr1.md` is the protocol and the reader's guide.
//!
//! Local files only: no network code, no telemetry, no paid run. The modules:
//!
//! | Module | Does |
//! | --- | --- |
//! | `schema` | reads one log line tolerantly and names every deviation |
//! | `code` | reads a challenge code far enough to give its level |
//! | `logs` | merges lines into rounds (the last `end` line of a restored round wins) and finds disputes |
//! | `numbers` | the GR1 numbers, as the `enth.gr1-report/1` value |
//! | `baseline`, `survey` | the agent and floor attempts, and the end-of-session answers |
//! | `render` | the readable tables |
//! | `stats`, `privacy` | medians, Spearman with a seeded bootstrap, Wilson; the personal-data lint |

// `code`, `logs`, `privacy` and `schema` are crate-visible: `gr2-report` (R2 lane INT2) reads round logs through them.
mod baseline;
pub(crate) mod code;
pub(crate) mod logs;
mod numbers;
pub(crate) mod privacy;
mod render;
pub(crate) mod schema;
mod stats;
mod survey;

#[cfg(test)]
mod docs_sync;
#[cfg(test)]
mod fixtures;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use logs::Source;
use numbers::{Extras, Report, Thresholds};

const USAGE: &str = "usage: cargo xtask gr1-report [options] <round log>...

  Reads round logs (JSONL, schema enth.round-log/1) and prints the GR1 numbers: rounds played, pass rate by level,
  median time to pass, the assisted share, the agreement distribution, the blind rating against the score (Spearman
  rho with a bootstrap interval), disputes, \"want more\", and the agent baseline beside the no-op and CMA-ES floors.
  Local files only; nothing is sent anywhere. docs/gr1.md explains every number.

  <round log>   a log file, `id=file` to name the player, or a directory (every *.jsonl in it, one player per file).
                A player is the file's name without .jsonl unless named; files of one player merge, and a round the
                host restored (logged twice) counts once, by its last `end` line. Two different files with the same
                name need `id=` to say whether they are one player or two.

options:
  --player-id ID     the id for every log given without `id=` (merges them as one player; use a short anonymous id)
  --baseline FILE    agent or floor attempts, JSONL enth.gr1-baseline/1 (repeatable); never run, only read
  --survey FILE      end-of-session answers, JSONL enth.gr1-survey/1 (\"want more?\" per player)
  --json             print the report as JSON (enth.gr1-report/1) instead of tables
  --validate         only check the logs against the schema and print what is wrong; exit 1 on an error
  --codes            only list the codes the players played (one per line: code, level, players), for the agent run
  --strict           print the report, then exit 1 if any line was an error or looks like personal data
  --rounds N         rounds each player should play (default 20)
  --bootstrap N      bootstrap resamples for rho's interval (default 2000)
  --seed N           the bootstrap's seed (default fixed, so the same logs give the same numbers)
  -h, --help         this text";

/// What the command line asked for.
#[derive(Debug, PartialEq)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "independent command-line flags, not a state machine"
)]
struct Options {
    logs: Vec<LogArg>,
    player_id: Option<String>,
    baselines: Vec<PathBuf>,
    survey: Option<PathBuf>,
    json: bool,
    validate: bool,
    codes: bool,
    strict: bool,
    thresholds: Thresholds,
}

/// One log argument: a file and, maybe, whose it is.
#[derive(Debug, PartialEq)]
struct LogArg {
    id: Option<String>,
    path: PathBuf,
}

/// `cargo xtask gr1-report`.
pub fn gr1_report(args: &[String]) -> Result<(), String> {
    if args.iter().any(|a| a == "-h" || a == "--help") {
        println!("{USAGE}");
        return Ok(());
    }
    let opts = parse_args(args)?;
    let sources = read_sources(&opts)?;
    let extras = read_extras(&opts)?;
    let loaded = logs::load(&sources);
    if opts.codes {
        return print_codes(&loaded, opts.json);
    }
    let report = numbers::build(&loaded, &extras, &opts.thresholds);
    if opts.validate {
        print_validation(&report, opts.json)?;
        return check_clean(&report, true);
    }
    if opts.json {
        let text = serde_json::to_string_pretty(&report)
            .map_err(|e| format!("cannot write the report: {e}"))?;
        println!("{text}");
    } else {
        print!("{}", render::text(&report));
    }
    check_clean(&report, opts.strict)
}

/// `--validate` and `--strict` fail on an unusable line or on possible personal data.
fn check_clean(report: &Report, enforce: bool) -> Result<(), String> {
    if !enforce {
        return Ok(());
    }
    let (errors, privacy) = (report.input.errors, report.input.privacy_findings);
    if errors > 0 || privacy > 0 {
        return Err(format!(
            "{errors} error(s) and {privacy} possible personal-data finding(s) in the logs (see the problems above)"
        ));
    }
    Ok(())
}

/// `--codes`: the distinct codes of played rounds, which the agent and the floors must play too.
fn print_codes(loaded: &logs::Loaded, json: bool) -> Result<(), String> {
    let rows = loaded.played_codes();
    if json {
        let text = serde_json::to_string_pretty(&rows).map_err(|e| e.to_string())?;
        println!("{text}");
        return Ok(());
    }
    for row in &rows {
        let level = row.level.map_or_else(|| "?".to_owned(), |l| l.to_string());
        println!("{}  level {level}  {} player(s)", row.code, row.players);
    }
    Ok(())
}

fn print_validation(report: &Report, json: bool) -> Result<(), String> {
    if json {
        let value = serde_json::json!({
            "schema": "enth.gr1-validate/1",
            "log_schema": schema::ROUND_LOG_SCHEMA,
            "input": report.input,
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&value).map_err(|e| e.to_string())?
        );
        return Ok(());
    }
    let i = &report.input;
    println!(
        "Checked {} line(s) in {} file(s) against {}: {} error(s), {} warning(s), {} note(s); {} line(s) skipped; {} personal-data finding(s).",
        i.lines,
        i.files,
        schema::ROUND_LOG_SCHEMA,
        i.errors,
        i.warnings,
        i.notes,
        i.skipped_unusable + i.skipped_other_schema,
        i.privacy_findings
    );
    for p in &i.problems {
        println!(
            "  {:<7} {} (x{}, first at {})",
            p.severity, p.message, p.count, p.first_at
        );
    }
    for p in &i.privacy {
        println!("  privacy {p}");
    }
    Ok(())
}

fn parse_args(args: &[String]) -> Result<Options, String> {
    let mut opts = Options {
        logs: Vec::new(),
        player_id: None,
        baselines: Vec::new(),
        survey: None,
        json: false,
        validate: false,
        codes: false,
        strict: false,
        thresholds: Thresholds::default(),
    };
    let mut positional: Vec<&str> = Vec::new();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        // `--flag=value` and `--flag value` both work.
        let (flag, inline) = match arg.split_once('=') {
            Some((f, v)) if f.starts_with("--") => (f, Some(v.to_owned())),
            _ => (arg.as_str(), None),
        };
        let mut value = |name: &str| -> Result<String, String> {
            inline
                .clone()
                .or_else(|| it.next().cloned())
                .ok_or_else(|| format!("{name} needs a value\n\n{USAGE}"))
        };
        match flag {
            "--json" => opts.json = true,
            "--validate" => opts.validate = true,
            "--codes" => opts.codes = true,
            "--strict" => opts.strict = true,
            "--player-id" => opts.player_id = Some(check_id(&value("--player-id")?)?),
            "--baseline" => opts.baselines.push(PathBuf::from(value("--baseline")?)),
            "--survey" => opts.survey = Some(PathBuf::from(value("--survey")?)),
            "--rounds" => opts.thresholds.rounds_each = number(&value("--rounds")?, "--rounds")?,
            "--bootstrap" => {
                opts.thresholds.bootstrap_reps = number(&value("--bootstrap")?, "--bootstrap")?;
            }
            "--seed" => opts.thresholds.bootstrap_seed = number(&value("--seed")?, "--seed")?,
            f if f.starts_with("--") => return Err(format!("unknown option {f}\n\n{USAGE}")),
            _ => positional.push(arg),
        }
    }
    if positional.is_empty() {
        return Err(format!("no round log given\n\n{USAGE}"));
    }
    for p in positional {
        opts.logs.push(log_arg(p)?);
    }
    Ok(opts)
}

fn number<T: std::str::FromStr>(text: &str, flag: &str) -> Result<T, String> {
    text.parse()
        .map_err(|_| format!("{flag} needs a number, not {text:?}"))
}

/// A player id is short and anonymous: letters, digits, `_`, `-` and `.`.
fn check_id(id: &str) -> Result<String, String> {
    let ok = !id.is_empty()
        && id.len() <= 40
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'));
    if ok {
        Ok(id.to_owned())
    } else {
        Err(format!(
            "player id {id:?}: use a short anonymous id of letters, digits, `_`, `-` or `.` (p1, friend2); never a name or an address"
        ))
    }
}

/// `id=path`, or a plain path. The part before the first `=` is an id only when it looks like one.
fn log_arg(text: &str) -> Result<LogArg, String> {
    if let Some((id, path)) = text.split_once('=')
        && !id.is_empty()
        && !path.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
    {
        return Ok(LogArg {
            id: Some(check_id(id)?),
            path: PathBuf::from(path),
        });
    }
    Ok(LogArg {
        id: None,
        path: PathBuf::from(text),
    })
}

fn file_label(path: &Path) -> String {
    path.file_name()
        .map_or_else(|| "log".to_owned(), |n| n.to_string_lossy().into_owned())
}

fn file_stem(path: &Path) -> String {
    path.file_stem()
        .map_or_else(|| "log".to_owned(), |n| n.to_string_lossy().into_owned())
}

/// Expand directories, read every file and decide whose it is.
fn read_sources(opts: &Options) -> Result<Vec<Source>, String> {
    // (explicit id, path)
    let mut files: Vec<(Option<String>, PathBuf)> = Vec::new();
    for arg in &opts.logs {
        if arg.path.is_dir() {
            let mut inside: Vec<PathBuf> = std::fs::read_dir(&arg.path)
                .map_err(|e| format!("{}: {e}", file_label(&arg.path)))?
                .filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|x| x == "jsonl"))
                .collect();
            inside.sort();
            if inside.is_empty() {
                return Err(format!(
                    "{}: no .jsonl files in this folder",
                    file_label(&arg.path)
                ));
            }
            files.extend(inside.into_iter().map(|p| (arg.id.clone(), p)));
        } else {
            files.push((arg.id.clone(), arg.path.clone()));
        }
    }
    // Ids for the files given without one: --player-id, or the file's name. Two different files with the same name
    // and no id could be one player's two files or two players' files: refuse to guess.
    let mut stems: BTreeMap<String, Vec<&Path>> = BTreeMap::new();
    if opts.player_id.is_none() {
        for (id, path) in &files {
            if id.is_none() {
                let paths = stems.entry(file_stem(path)).or_default();
                if !paths.contains(&path.as_path()) {
                    paths.push(path);
                }
            }
        }
        if let Some((stem, _)) = stems.iter().find(|(_, paths)| paths.len() > 1) {
            return Err(format!(
                "more than one file is called {stem}.jsonl: name each with id=file (one id for one player's files, two ids for two players), or give --player-id for a single player"
            ));
        }
    }
    files
        .into_iter()
        .map(|(id, path)| {
            let player = match id {
                Some(id) => id,
                None => match &opts.player_id {
                    Some(id) => id.clone(),
                    None => check_id(&file_stem(&path)).map_err(|_| {
                        format!(
                            "{}: cannot use the file name as a player id; name it with id=file or --player-id",
                            file_label(&path)
                        )
                    })?,
                },
            };
            let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", file_label(&path)))?;
            Ok(Source {
                player,
                label: file_label(&path),
                text: String::from_utf8_lossy(&bytes).into_owned(),
            })
        })
        .collect()
}

fn read_extras(opts: &Options) -> Result<Extras, String> {
    let mut extras = Extras::default();
    for path in &opts.baselines {
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", file_label(path)))?;
        let (attempts, problems) =
            baseline::read(&file_label(path), &String::from_utf8_lossy(&bytes));
        extras.attempts.extend(attempts);
        extras.problems.extend(problems);
        extras.baseline_files += 1;
    }
    if let Some(path) = &opts.survey {
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", file_label(path)))?;
        let (answers, problems) = survey::read(&file_label(path), &String::from_utf8_lossy(&bytes));
        extras.survey = answers;
        extras.problems.extend(problems);
        extras.survey_given = true;
    }
    Ok(extras)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn arguments_parse_flags_ids_and_both_value_forms() {
        let o = parse_args(&args(&[
            "--json",
            "--baseline",
            "agent.jsonl",
            "--baseline=floors.jsonl",
            "--survey",
            "s.jsonl",
            "--rounds=15",
            "p1=a.jsonl",
            "b.jsonl",
            "./odd=name.jsonl",
        ]))
        .unwrap();
        assert!(o.json && !o.validate && !o.strict && !o.codes);
        assert_eq!(o.baselines.len(), 2);
        assert_eq!(o.survey, Some(PathBuf::from("s.jsonl")));
        assert_eq!(o.thresholds.rounds_each, 15);
        assert_eq!(
            o.logs,
            vec![
                LogArg {
                    id: Some("p1".into()),
                    path: "a.jsonl".into()
                },
                LogArg {
                    id: None,
                    path: "b.jsonl".into()
                },
                LogArg {
                    id: None,
                    path: "./odd=name.jsonl".into()
                },
            ]
        );
    }

    #[test]
    fn codes_is_a_mode_of_its_own() {
        let o = parse_args(&args(&["--codes", "a.jsonl"])).unwrap();
        assert!(o.codes && !o.json);
    }

    #[test]
    fn bad_arguments_say_what_is_wrong() {
        for (list, want) in [
            (&["--json"][..], "no round log"),
            (&["--nope", "a"][..], "unknown option --nope"),
            (&["--rounds", "many", "a"][..], "needs a number"),
            (&["a", "--survey"][..], "needs a value"),
            (
                &["--player-id", "alice@example.com", "a"][..],
                "anonymous id",
            ),
            (&["a/b=c.jsonl"][..], ""),
        ] {
            let r = parse_args(&args(list));
            if want.is_empty() {
                assert!(r.is_ok(), "{list:?}");
            } else {
                let e = r.unwrap_err();
                assert!(e.contains(want), "{list:?}: {e}");
            }
        }
    }

    #[test]
    fn ids_are_short_and_anonymous() {
        assert!(check_id("p1").is_ok() && check_id("friend-2.a_b").is_ok());
        for bad in ["", "a b", "a@b", "a/b", &"x".repeat(41)] {
            assert!(check_id(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn help_prints_and_succeeds() {
        assert!(gr1_report(&args(&["--help"])).is_ok());
    }
}
