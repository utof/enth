//! Parsing, dispatch and printing. Each command returns a [`Reply`] (a typed output, normalised) or a
//! [`CliError`]; this module turns either into an [`Output`].

pub(crate) mod challenge;
mod common;
pub(crate) mod compare;
mod edit;
pub(crate) mod explain;
mod game;
mod info;
pub(crate) mod matching;
mod sound;
pub(crate) mod suggest;

use std::ffi::OsString;
use std::fmt::Write as _;

use clap::Parser;
use clap::error::{ContextKind, ContextValue, ErrorKind};
use serde::Serialize;
use serde_json::Value;

use crate::cli::{Cli, Cmd, Format};
use crate::error::{CliError, EXIT_OK, EXIT_USAGE};
use crate::{Env, Output, out, table};

/// A command's result.
pub struct Reply {
    /// The output object, normalised.
    pub value: Value,
    /// The exit code.
    pub code: i32,
    /// What a terminal shows instead of a table (the guide's markdown).
    pub text: Option<String>,
    /// Notes for humans.
    pub stderr: String,
    /// Columns tables should show first.
    pub columns: &'static [&'static str],
}

impl Reply {
    /// A reply with exit code 0.
    pub fn new<T: Serialize>(t: &T) -> Self {
        Reply {
            value: out::to_value(t),
            code: EXIT_OK,
            text: None,
            stderr: String::new(),
            columns: &[],
        }
    }

    /// Set the exit code.
    #[must_use]
    pub fn code(mut self, code: i32) -> Self {
        self.code = code;
        self
    }

    /// Set the terminal text.
    #[must_use]
    pub fn text(mut self, text: String) -> Self {
        self.text = Some(text);
        self
    }

    /// Add a note for humans.
    #[must_use]
    pub fn note(mut self, line: &str) -> Self {
        self.stderr.push_str(line);
        self.stderr.push('\n');
        self
    }

    /// Set the leading table columns.
    #[must_use]
    pub fn columns(mut self, cols: &'static [&'static str]) -> Self {
        self.columns = cols;
        self
    }
}

/// What every command returns.
pub type Res = Result<Reply, CliError>;

/// Parse, run and print one command.
pub fn dispatch(argv: &[OsString], stdin: &str, env: Env) -> Output {
    let table_flag = format_flag(argv);
    let cli = match Cli::try_parse_from(argv) {
        Ok(c) => c,
        Err(e) => return clap_error(&e, table_flag.unwrap_or(env.stdout_tty)),
    };
    let table = match cli.format {
        Some(Format::Table) => true,
        Some(Format::Json) => false,
        None => env.stdout_tty,
    };
    match run(cli.cmd, stdin) {
        Ok(r) => emit(r, table),
        Err(e) => emit_error(&e, table),
    }
}

fn run(cmd: Cmd, stdin: &str) -> Res {
    match cmd {
        Cmd::Guide { topic } => info::guide(topic.as_deref()),
        Cmd::Schema { name } => info::schema(name.as_deref()),
        Cmd::Modules => Ok(info::modules()),
        Cmd::Params { kind, patch } => info::params(&kind, patch.as_deref()),
        Cmd::New {
            template,
            out,
            list,
        } => edit::new(template.as_deref(), out.as_deref(), list),
        Cmd::Apply {
            patch,
            edits,
            out,
            dry_run,
        } => edit::apply(&patch, &edits, out.as_deref(), dry_run, stdin),
        Cmd::Get {
            patch,
            addrs,
            plain,
        } => edit::get(&patch, &addrs, plain),
        Cmd::Diff {
            a,
            b,
            describe,
            perf,
        } => edit::diff(&a, &b, describe, &perf, stdin),
        Cmd::Render {
            patch,
            perf,
            out,
            bits,
            seed,
        } => sound::render(&patch, &perf, out.as_deref(), bits, seed, stdin),
        Cmd::Describe { input, perf, seed } => sound::describe(&input, &perf, seed, stdin),
        Cmd::Check {
            input,
            expect,
            perf,
            fix,
            seed,
        } => sound::check(&input, &expect, &perf, &fix, seed, stdin),
        Cmd::Step {
            patch,
            edits,
            expect,
            perf,
            out,
            wav,
            full,
            fix,
            seed,
        } => sound::step(&sound::StepArgs {
            patch: &patch,
            edits: &edits,
            expect: expect.as_deref(),
            perf: &perf,
            out: out.as_deref(),
            wav: wav.as_deref(),
            full,
            fix: &fix,
            seed,
            stdin,
        }),
        Cmd::Bench {
            patch,
            voices,
            seconds,
        } => sound::bench(patch.as_deref(), voices, seconds),
        Cmd::Compare(args) => compare::run(&args, stdin),
        Cmd::Suggest(args) => suggest::run(&args),
        Cmd::Match(args) => matching::run(&args, stdin),
        Cmd::Explain(args) => explain::run(&args, stdin),
        Cmd::Challenge(args) => challenge::run(&args),
    }
}

/// `--format` read before clap runs, so clap's own errors honour it.
fn format_flag(argv: &[OsString]) -> Option<bool> {
    let args: Vec<String> = argv
        .iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    args.iter().enumerate().find_map(|(i, a)| {
        let v = a.strip_prefix("--format=").map(str::to_owned).or_else(|| {
            (a == "--format")
                .then(|| args.get(i + 1).cloned())
                .flatten()
        })?;
        match v.as_str() {
            "table" => Some(true),
            "json" => Some(false),
            _ => None,
        }
    })
}

fn emit(r: Reply, table: bool) -> Output {
    let stdout = if table {
        r.text
            .clone()
            .unwrap_or_else(|| table::render(&r.value, r.columns))
    } else {
        out::line(&r.value)
    };
    Output {
        code: r.code,
        stdout,
        stderr: r.stderr,
    }
}

fn emit_error(e: &CliError, table: bool) -> Output {
    if table {
        let mut s = format!(
            "error ({}): {}\nhint: {}\n",
            e.0.error, e.0.message, e.0.hint
        );
        if !e.0.did_you_mean.is_empty() {
            let _ = writeln!(s, "did you mean: {}", e.0.did_you_mean.join(", "));
        }
        for item in e.0.errors.iter().skip(1) {
            let _ = writeln!(s, "also at {}: {}", item.path, item.message);
        }
        return Output {
            code: e.exit(),
            stdout: String::new(),
            stderr: s,
        };
    }
    Output {
        code: e.exit(),
        stdout: out::line(&out::to_value(&*e.0)),
        stderr: String::new(),
    }
}

fn context_strings(e: &clap::Error, kind: ContextKind) -> Vec<String> {
    match e.get(kind) {
        Some(ContextValue::String(s)) => vec![s.clone()],
        Some(ContextValue::Strings(v)) => v.clone(),
        Some(ContextValue::StyledStr(s)) => vec![s.to_string()],
        Some(ContextValue::StyledStrs(v)) => v.iter().map(ToString::to_string).collect(),
        _ => Vec::new(),
    }
}

fn clap_error(e: &clap::Error, table: bool) -> Output {
    let text = e.render().to_string();
    if matches!(
        e.kind(),
        ErrorKind::DisplayHelp
            | ErrorKind::DisplayVersion
            | ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
    ) {
        let code = if e.kind() == ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand {
            EXIT_USAGE
        } else {
            EXIT_OK
        };
        return Output {
            code,
            stdout: text,
            stderr: String::new(),
        };
    }
    // The first line, plus the indented lines that continue it (clap lists missing arguments that way).
    let mut lines = text.lines().skip_while(|l| l.trim().is_empty());
    let mut message = lines
        .next()
        .unwrap_or("bad command line")
        .trim_start_matches("error: ")
        .trim_end_matches(':')
        .to_owned();
    let more: Vec<&str> = lines
        .take_while(|l| l.starts_with(' ') && !l.trim().is_empty())
        .map(str::trim)
        .collect();
    if !more.is_empty() {
        message = format!("{message}: {}", more.join(", "));
    }
    let usage = text
        .lines()
        .find_map(|l| l.trim().strip_prefix("Usage: "))
        .map(str::to_owned);
    let valid = context_strings(e, ContextKind::ValidValue);
    let hint = match usage {
        Some(u) => format!("usage: {u}; `--help` shows the flags, `enth guide` the workflow"),
        None if !valid.is_empty() => format!("use one of: {}", valid.join(", ")),
        None => "`enth --help` lists the commands; `enth guide` shows the workflow".to_owned(),
    };
    let mut suggestions = context_strings(e, ContextKind::SuggestedSubcommand);
    suggestions.extend(context_strings(e, ContextKind::SuggestedArg));
    suggestions.extend(context_strings(e, ContextKind::SuggestedValue));
    // Nearest first, by edit distance to what was typed.
    let typed = context_strings(e, ContextKind::InvalidSubcommand)
        .into_iter()
        .chain(context_strings(e, ContextKind::InvalidArg))
        .chain(context_strings(e, ContextKind::InvalidValue))
        .next()
        .unwrap_or_default();
    suggestions.sort_by_key(|s| strsim::damerau_levenshtein(&typed, s));
    suggestions.dedup();
    let mut err = crate::error::usage("usage", message, hint).suggest(suggestions);
    if !valid.is_empty() && valid.len() <= 12 {
        err = err.known(valid);
    }
    emit_error(&err, table)
}
