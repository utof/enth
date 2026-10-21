//! Exit codes and the error object (`enth.error/1`), and the translation of every library error into it.
//!
//! | Exit | Meaning | Codes |
//! | --- | --- | --- |
//! | 0 | OK (`check`: every item passed) | |
//! | 1 | Ran fine, but `check` had failures | |
//! | 2 | Usage error | `usage`, `bad_expect`, `bad_perf`, `bad_ops_source`, `nothing_to_apply`; the game commands': `bad_level`, `bad_workers`, `bad_budget`, `bad_stop_at`, `bad_seconds`, `bad_free`, `bad_amount`, `perf_required`, `perf_fixed`, `phrase_fixed`, `unknown_phrase`; `not_built_yet` (an R2 command whose lane has not built it) |
//! | 3 | Validation error | every `enth-patch` code (`unknown_param`, `unknown_module`, `out_of_range`, `bad_json`, ...), `unknown_feature`, `unknown_kind`, `unknown_template`, `unknown_topic`, `unknown_schema`; the game commands': a challenge code's own (`bad_check`, `bad_symbol`, `empty_code`, `code_too_new`, `target_mismatch`, ...), `unknown_category`, `unknown_module`, `unknown_kind` |
//! | 4 | Render or analysis failure | `render_failed` (and the render's own codes), `analysis_failed`, `unsupported_wav`, `write_failed` |
//! | 5 | File not found | `file_not_found` |
//!
//! Every error carries a `hint` (the fix or the next command) and `did_you_mean` (possibly empty). For parameter
//! addresses, `did_you_mean` holds full addresses and also draws on the alias table in `aliases` (`osc1.detune`
//! suggests `osc1.fine`).

use enth_analysis::expect::ExpectError;
use enth_params::ValueError;
use enth_patch::addr::AddrError;
use enth_patch::error::closest;
use enth_patch::{Invalid, Patch, PatchError, Violation};

use crate::aliases;
use crate::schema::{ErrorItem, ErrorOut, ids};

/// OK (for `check`: every expectation passed).
pub const EXIT_OK: i32 = 0;
/// `check` ran fine, but at least one expectation failed.
pub const EXIT_CHECK_FAILED: i32 = 1;
/// Bad flags or arguments that cannot be understood.
pub const EXIT_USAGE: i32 = 2;
/// A bad patch, an unknown parameter or name, a value out of range.
pub const EXIT_VALIDATION: i32 = 3;
/// A render or analysis failure, or an output file that cannot be written.
pub const EXIT_RENDER: i32 = 4;
/// An input file that does not exist or cannot be read.
pub const EXIT_NOT_FOUND: i32 = 5;

/// An error on its way to stdout as `enth.error/1`.
#[derive(Clone, Debug, PartialEq)]
pub struct CliError(pub Box<ErrorOut>);

impl CliError {
    /// An error with an exit code, a stable code, a message and a hint.
    pub fn new(exit: i32, code: &str, message: impl Into<String>, hint: impl Into<String>) -> Self {
        CliError(Box::new(ErrorOut {
            schema: ids::ERROR.to_owned(),
            error: code.to_owned(),
            message: message.into(),
            exit,
            hint: hint.into(),
            did_you_mean: Vec::new(),
            path: None,
            file: None,
            try_value: None,
            min: None,
            max: None,
            known: Vec::new(),
            errors: Vec::new(),
        }))
    }

    /// The exit code.
    pub fn exit(&self) -> i32 {
        self.0.exit
    }

    /// The stable code.
    pub fn code(&self) -> &str {
        &self.0.error
    }

    /// Set `path`.
    #[must_use]
    pub fn at(mut self, path: impl Into<String>) -> Self {
        self.0.path = Some(path.into());
        self
    }

    /// Set `file`.
    #[must_use]
    pub fn in_file(mut self, file: impl Into<String>) -> Self {
        self.0.file = Some(file.into());
        self
    }

    /// Set `did_you_mean`.
    #[must_use]
    pub fn suggest(mut self, names: Vec<String>) -> Self {
        self.0.did_you_mean = names;
        self
    }

    /// Set `known`.
    #[must_use]
    pub fn known(mut self, names: Vec<String>) -> Self {
        self.0.known = names;
        self
    }
}

/// A usage error (exit 2).
pub fn usage(code: &str, message: impl Into<String>, hint: impl Into<String>) -> CliError {
    CliError::new(EXIT_USAGE, code, message, hint)
}

/// A validation error (exit 3).
pub fn invalid(code: &str, message: impl Into<String>, hint: impl Into<String>) -> CliError {
    CliError::new(EXIT_VALIDATION, code, message, hint)
}

/// A render, analysis or write failure (exit 4).
pub fn failed(code: &str, message: impl Into<String>, hint: impl Into<String>) -> CliError {
    CliError::new(EXIT_RENDER, code, message, hint)
}

/// An input file that cannot be read (exit 5).
pub fn not_found(path: &str, why: &std::io::Error) -> CliError {
    let missing = why.kind() == std::io::ErrorKind::NotFound;
    let message = if missing {
        format!("no such file: {path}")
    } else {
        format!("cannot read {path}: {why}")
    };
    CliError::new(
        EXIT_NOT_FOUND,
        "file_not_found",
        message,
        "check the path; start a patch with `enth new --template init --out p.patch.json`",
    )
    .in_file(path)
}

/// An output file that cannot be written (exit 4).
pub fn write_failed(path: &str, why: &std::io::Error) -> CliError {
    failed(
        "write_failed",
        format!("cannot write {path}: {why}"),
        "check that the directory exists and is writable",
    )
    .in_file(path)
}

/// Up to three names from `pool` close to `input` (the patch crate's rule: an edit distance of one for short words,
/// a third of the length for longer ones, or a prefix).
pub fn nearest<'a>(input: &str, pool: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    closest(input, pool)
}

// ---------------------------------------------------------------------------------------------------------------
// enth-patch errors
// ---------------------------------------------------------------------------------------------------------------

/// What the CLI knows around a patch error: the patch (for module ids and kinds), the file, and the `--set` pairs
/// (so `set[1]` reads as the address that pair named).
#[derive(Clone, Copy, Default)]
pub struct Ctx<'a> {
    /// The patch the error is about.
    pub patch: Option<&'a Patch>,
    /// The file it came from.
    pub file: Option<&'a str>,
    /// The `--set` pairs, as written.
    pub set_pairs: &'a [String],
    /// The address as written, when the error is about one (`enth get p filtr1.cutoff`).
    pub addr: Option<&'a str>,
}

/// An `Invalid` (one or more violations) as one error: the first violation on top, all of them in `errors` when
/// there are several. Exit 3.
pub fn from_invalid(inv: &Invalid, ctx: Ctx<'_>) -> CliError {
    let items: Vec<ErrorItem> = inv.0.iter().map(|v| item(v, ctx)).collect();
    let Some(first) = inv.0.first() else {
        return invalid(
            "bad_patch",
            "the patch is invalid",
            "see `enth schema patch`",
        );
    };
    let top = &items[0];
    let mut e = CliError::new(
        EXIT_VALIDATION,
        &top.error,
        top.message.clone(),
        top.hint.clone(),
    )
    .suggest(top.did_you_mean.clone());
    if !top.path.is_empty() {
        e = e.at(top.path.clone());
    }
    if let Some(f) = ctx.file {
        e = e.in_file(f);
    }
    if let PatchError::Value(ValueError::OutOfRange {
        value,
        min,
        max,
        unit,
        ..
    }) = &first.error
    {
        let nearest = value.clamp(*min, *max);
        e.0.try_value = Some(unit.display_rounded(nearest, 4).to_string());
        e.0.min = Some(f64::from(*min));
        e.0.max = Some(f64::from(*max));
    }
    if items.len() > 1 {
        e.0.errors = items;
    }
    e
}

/// One patch error with a path, without the list.
pub fn from_patch_error(path: &str, err: &PatchError, ctx: Ctx<'_>) -> CliError {
    from_invalid(&Invalid::one(path, err.clone()), ctx)
}

/// The address a `--set` pair names, for `set[i]` paths.
fn set_pair_addr<'a>(path: &str, pairs: &'a [String]) -> Option<&'a str> {
    let rest = path.strip_prefix("set[")?;
    let (idx, _) = rest.split_once(']')?;
    let i: usize = idx.parse().ok()?;
    // `--set` values may be given as several flags; each flag may hold several comma-separated pairs.
    let pair = pairs
        .iter()
        .flat_map(|p| p.split(','))
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .nth(i)?;
    Some(pair.split_once('=').map_or(pair, |(k, _)| k).trim())
}

fn item(v: &Violation, ctx: Ctx<'_>) -> ErrorItem {
    let written = set_pair_addr(&v.path, ctx.set_pairs).or(ctx.addr);
    let path = written.map_or_else(|| v.path.clone(), str::to_owned);
    let e = &v.error;
    let mut hint = e.hint();
    if let PatchError::UnknownParam { owner, .. } = e
        && *owner != "voice"
        && *owner != "master"
    {
        hint = format!("{hint}; units and ranges: `enth params {owner}`");
    }
    // A key that no module kind has (`filter1.res`): name the keys this module does have.
    if let PatchError::Addr(AddrError::UnknownParam { text }) = e
        && let Some((target, _)) = text.rsplit_once('.')
    {
        let keys = aliases::target_keys(ctx.patch, target);
        let owner = aliases::kind_for_name(target).map_or(target, |k| k.key());
        if keys.len() < 20 {
            hint = format!(
                "{target} has: {}; units and ranges: `enth params {owner}`",
                keys.join(", ")
            );
        }
    }
    ErrorItem {
        path,
        error: e.code().to_owned(),
        message: e.message(),
        hint,
        did_you_mean: suggestions(e, ctx.patch, written),
    }
}

/// `did_you_mean` for a patch error: the patch crate's near misses plus the alias table, as full addresses where
/// the error is about an address.
fn suggestions(e: &PatchError, patch: Option<&Patch>, written: Option<&str>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut push = |s: String| {
        if !out.contains(&s) {
            out.push(s);
        }
    };
    match e {
        PatchError::UnknownParam { addr, keys, .. } => {
            let (target, param) = addr.rsplit_once('.').unwrap_or(("", addr.as_str()));
            for k in aliases::param_keys(param, keys) {
                push(join(target, k));
            }
            for k in e.did_you_mean() {
                push(join(target, &k));
            }
        }
        PatchError::Addr(AddrError::UnknownParam { text }) => {
            let (target, param) = text.rsplit_once('.').unwrap_or(("", text.as_str()));
            let keys = aliases::target_keys(patch, target);
            for k in aliases::param_keys(param, &keys) {
                push(join(target, k));
            }
            for k in closest(param, keys.iter().copied()) {
                push(join(target, &k));
            }
        }
        PatchError::UnknownModule { id, existing } => {
            let mut ids: Vec<String> = Vec::new();
            if let Some(p) = patch {
                ids.extend(aliases::module_ids(p, id));
            }
            ids.extend(closest(id, existing.iter().map(String::as_str)));
            // An address like `filtr1.cutoff`: suggest the same parameter on the near ids.
            let rest = written
                .and_then(|w| w.strip_prefix(id.as_str()))
                .and_then(|r| r.strip_prefix('.'));
            for i in ids {
                match rest {
                    Some(r) => push(format!("{i}.{r}")),
                    None => push(i),
                }
            }
        }
        _ => {
            for s in e.did_you_mean() {
                push(s);
            }
        }
    }
    out.truncate(3);
    out
}

fn join(target: &str, key: &str) -> String {
    if target.is_empty() {
        key.to_owned()
    } else {
        format!("{target}.{key}")
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Other library errors
// ---------------------------------------------------------------------------------------------------------------

/// An expectation that cannot be parsed: `bad_expect` is a usage error (exit 2), `unknown_feature` a validation
/// error (exit 3).
pub fn from_expect(e: &ExpectError) -> CliError {
    let exit = if e.code == "unknown_feature" {
        EXIT_VALIDATION
    } else {
        EXIT_USAGE
    };
    CliError::new(exit, e.code, e.message.clone(), e.hint.clone())
        .at(e.item.clone())
        .suggest(e.did_you_mean.clone())
}

/// A render that could not run (exit 4).
pub fn from_render(e: &enth_render::RenderError) -> CliError {
    let hint = if e.code() == "not_yet" {
        "this engine build cannot render that yet: undo the edit (apply the `inverse` ops) or remove it"
    } else {
        "check the patch with `enth get <patch>` and the performance with `enth guide`"
    };
    failed(
        "render_failed",
        format!("the render failed ({}): {e}", e.code()),
        hint,
    )
}

/// An evaluation of the solver's `measure` that failed: a render or an analysis failure (exit 4).
pub fn from_eval(e: &enth_solver::EvalError) -> CliError {
    match e {
        enth_solver::EvalError::Render(r) => from_render(r),
        enth_solver::EvalError::Analysis(a) => from_analysis(a),
        other => failed(
            other.code(),
            other.to_string(),
            "the CLI renders at 48 kHz; report this as a bug",
        ),
    }
}

/// A signal the analysis refused (exit 4).
pub fn from_analysis(e: &enth_analysis::descriptors::AnalysisError) -> CliError {
    failed(
        "analysis_failed",
        format!("the analysis failed: {e}"),
        "render at least 20 ms of finite audio; check the performance (--perf)",
    )
}
