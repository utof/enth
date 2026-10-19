//! One error vocabulary for everything that can go wrong with a patch: applying an op, validating, reading a file,
//! migrating, parsing the ops JSON. A code means one thing everywhere, so the CLI maps them all the same way (exit
//! 3, `{"code", "message", "hint", "did_you_mean"}`).
//!
//! [`PatchError`] is the variant list (alias [`OpError`] where an op is the cause). A list of problems with where
//! they are ([`Violation`], [`Invalid`]) is what `validate`, `load` and the wire parser return, so an agent sees
//! every mistake in a file at once.
//!
//! Codes (stable, snake case): the codes of [`AddrError`] (`bad_id`, `reserved_id`, `bad_address`, `unknown_param`,
//! `bad_index`, `bad_section`), of [`ValueError`] (`bad_value`, `wrong_unit`, `not_integral`, `out_of_range`,
//! `non_finite`), `unknown_kind`, and the rest listed in [`CODES`].

use core::fmt;

use enth_params::{Expected, ValueError};

use crate::addr::AddrError;
use crate::catalogue::{ModuleKind, Rate, UnknownKind};
use crate::model::Section;

/// Every code a [`PatchError`] can carry, in the order of the variants. A test keeps this list complete.
pub const CODES: &[&str] = &[
    "bad_id",
    "reserved_id",
    "bad_address",
    "unknown_param",
    "bad_index",
    "bad_section",
    "bad_value",
    "wrong_unit",
    "not_integral",
    "out_of_range",
    "non_finite",
    "unknown_kind",
    "unknown_module",
    "duplicate_id",
    "wrong_section",
    "wrong_kind",
    "not_a_source",
    "too_many",
    "wrong_value_count",
    "no_such_route",
    "duplicate_route",
    "not_modulatable",
    "not_automatable",
    "route_rate_mismatch",
    "in_use_as_amp_env",
    "module_disabled",
    "not_canonical",
    "missing_amp_env",
    "bad_json",
    "bad_file",
    "missing_schema",
    "bad_schema",
    "schema_too_new",
    "migration_failed",
    "unknown_op",
    "bad_op",
];

/// What went wrong. See the [module docs](self); every variant has a stable [`code`](Self::code), a one-line
/// [`hint`](Self::hint) and, where a near miss is likely, [`did_you_mean`](Self::did_you_mean).
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum PatchError {
    /// A malformed id, address, source, route or position text. Codes of [`AddrError`].
    Addr(AddrError),
    /// A value that does not fit its parameter. Codes of [`ValueError`].
    Value(ValueError),
    /// A module kind key that is not in the catalogue. Code `unknown_kind`.
    UnknownKind(UnknownKind),
    /// No module has this id. Code `unknown_module`.
    UnknownModule {
        /// The id looked for.
        id: String,
        /// The ids that exist.
        existing: Vec<String>,
    },
    /// The module (or `voice`, `master`) has no such parameter. Code `unknown_param`.
    UnknownParam {
        /// The address as written (`filter1.cutof`).
        addr: String,
        /// What the target is (`svf`, `voice`, `master`).
        owner: &'static str,
        /// The keys it does have.
        keys: Vec<&'static str>,
    },
    /// A second module with an id already in use. Code `duplicate_id`.
    DuplicateId {
        /// The id.
        id: String,
    },
    /// A module in the wrong section for its kind. Code `wrong_section`.
    WrongSection {
        /// The module.
        id: String,
        /// Its kind.
        kind: ModuleKind,
        /// The section its kind lives in.
        expected: Section,
        /// The section given.
        got: Section,
    },
    /// A module of the wrong kind for the job (the amplitude envelope must be an `adsr`). Code `wrong_kind`.
    WrongKind {
        /// The module.
        id: String,
        /// The kind needed.
        expected: ModuleKind,
        /// The kind it is.
        found: ModuleKind,
    },
    /// A route source that is not a modulator. Code `not_a_source`.
    NotASource {
        /// The module.
        id: String,
        /// Its kind.
        kind: ModuleKind,
    },
    /// An index past the end of what it indexes (a position in a section, a route index). Code `bad_index`.
    BadIndex {
        /// What is indexed (`"position"`, `"route index"`).
        what: &'static str,
        /// The index given.
        index: usize,
        /// The largest valid index.
        max: usize,
    },
    /// A limit reached (4 generators, 64 routes, …). Code `too_many`.
    TooMany {
        /// What there are too many of (`"generators"`, `"routes"`).
        what: &'static str,
        /// The limit.
        max: usize,
    },
    /// A module with the wrong number of parameter values for its kind. Code `wrong_value_count`.
    WrongValueCount {
        /// The module.
        id: String,
        /// The parameter count of its kind.
        expected: usize,
        /// The values given.
        found: usize,
    },
    /// No route with this source and destination. Code `no_such_route`.
    NoSuchRoute {
        /// `src -> dst`.
        route: String,
    },
    /// A route with this source and destination already exists. Code `duplicate_route`.
    DuplicateRoute {
        /// `src -> dst`.
        route: String,
    },
    /// A route to a parameter that modulation cannot drive. Code `not_modulatable`.
    NotModulatable {
        /// The destination.
        addr: String,
    },
    /// An automation slot on a parameter the host cannot automate. Code `not_automatable`.
    NotAutomatable {
        /// The parameter.
        addr: String,
    },
    /// A per-voice source driving a global destination. Code `route_rate_mismatch`.
    RouteRateMismatch {
        /// `src -> dst`.
        route: String,
        /// The source's rate.
        src: Rate,
        /// The destination's rate.
        dst: Rate,
    },
    /// The module is the amplitude envelope and cannot be removed or disabled. Code `in_use_as_amp_env`.
    InUseAsAmpEnv {
        /// The module.
        id: String,
        /// What was attempted: `"removed"` or `"disabled"`.
        action: &'static str,
    },
    /// A disabled module where an enabled one is needed (`set amp_env` to a disabled envelope). Code
    /// `module_disabled`.
    ModuleDisabled {
        /// The module.
        id: String,
    },
    /// A stored value that has a canonical spelling it is not written in: `-0.0` (store `0`) or the top of a
    /// periodic range (a phase of 360 is stored as 0). Code `not_canonical`.
    NotCanonical {
        /// The parameter (`osc1.phase`) or `route depth`.
        param: String,
        /// The stored value.
        value: f32,
        /// The value it should be stored as.
        canonical: f32,
    },
    /// The file names no amplitude envelope and has no `adsr` in `modulators` to take. Code `missing_amp_env`.
    MissingAmpEnv,
    /// Text that is not JSON. Code `bad_json`.
    BadJson {
        /// The parser's message, with line and column.
        message: String,
    },
    /// JSON of the wrong shape for a patch file (an unknown field, a wrong type). Code `bad_file`.
    BadFile {
        /// The decoder's message.
        message: String,
    },
    /// A file without a `schema` number. Code `missing_schema`.
    MissingSchema,
    /// A `schema` that is not a non-negative whole number. Code `bad_schema`.
    BadSchema {
        /// The value as written.
        found: String,
    },
    /// A file from a newer enth than this one. Code `schema_too_new`.
    SchemaTooNew {
        /// The file's schema.
        found: u32,
        /// The newest this build reads.
        supported: u32,
    },
    /// A migration step failed. Code `migration_failed`.
    MigrationFailed {
        /// The schema version the step migrates from.
        from: u32,
        /// Why.
        message: String,
    },
    /// An op JSON object with no known verb. Code `unknown_op`.
    UnknownOp {
        /// The keys it had.
        found: Vec<String>,
    },
    /// An op JSON object with a missing or ill-typed field. Code `bad_op`.
    BadOp {
        /// The verb.
        verb: &'static str,
        /// What is wrong.
        message: String,
    },
}

/// The error of applying an [`Op`](crate::ops::Op); the same type as every other patch error.
pub type OpError = PatchError;

impl From<AddrError> for PatchError {
    fn from(e: AddrError) -> Self {
        PatchError::Addr(e)
    }
}

impl From<ValueError> for PatchError {
    fn from(e: ValueError) -> Self {
        PatchError::Value(e)
    }
}

impl From<UnknownKind> for PatchError {
    fn from(e: UnknownKind) -> Self {
        PatchError::UnknownKind(e)
    }
}

impl PatchError {
    /// Stable snake-case code, one of [`CODES`].
    pub fn code(&self) -> &'static str {
        match self {
            PatchError::Addr(e) => e.code(),
            PatchError::Value(e) => e.code(),
            PatchError::UnknownKind(e) => e.code(),
            PatchError::UnknownModule { .. } => "unknown_module",
            PatchError::UnknownParam { .. } => "unknown_param",
            PatchError::DuplicateId { .. } => "duplicate_id",
            PatchError::WrongSection { .. } => "wrong_section",
            PatchError::WrongKind { .. } => "wrong_kind",
            PatchError::NotASource { .. } => "not_a_source",
            PatchError::BadIndex { .. } => "bad_index",
            PatchError::TooMany { .. } => "too_many",
            PatchError::WrongValueCount { .. } => "wrong_value_count",
            PatchError::NoSuchRoute { .. } => "no_such_route",
            PatchError::DuplicateRoute { .. } => "duplicate_route",
            PatchError::NotModulatable { .. } => "not_modulatable",
            PatchError::NotAutomatable { .. } => "not_automatable",
            PatchError::RouteRateMismatch { .. } => "route_rate_mismatch",
            PatchError::InUseAsAmpEnv { .. } => "in_use_as_amp_env",
            PatchError::ModuleDisabled { .. } => "module_disabled",
            PatchError::NotCanonical { .. } => "not_canonical",
            PatchError::MissingAmpEnv => "missing_amp_env",
            PatchError::BadJson { .. } => "bad_json",
            PatchError::BadFile { .. } => "bad_file",
            PatchError::MissingSchema => "missing_schema",
            PatchError::BadSchema { .. } => "bad_schema",
            PatchError::SchemaTooNew { .. } => "schema_too_new",
            PatchError::MigrationFailed { .. } => "migration_failed",
            PatchError::UnknownOp { .. } => "unknown_op",
            PatchError::BadOp { .. } => "bad_op",
        }
    }

    /// What went wrong, in one line, without the hint.
    pub fn message(&self) -> String {
        match self {
            PatchError::Addr(e) => e.to_string(),
            PatchError::Value(e) => e.to_string(),
            PatchError::UnknownKind(e) => e.to_string(),
            PatchError::UnknownModule { id, .. } => format!("no module '{id}'"),
            PatchError::UnknownParam { addr, owner, .. } => {
                format!("'{addr}' names no parameter of {owner}")
            }
            PatchError::DuplicateId { id } => format!("the id '{id}' is already in use"),
            PatchError::WrongSection {
                id,
                kind,
                expected,
                got,
            } => format!("'{id}' is an {kind}, which lives in {expected}, not {got}"),
            PatchError::WrongKind {
                id,
                expected,
                found,
            } => format!("'{id}' is an {found}, not an {expected}"),
            PatchError::NotASource { id, kind } => {
                format!("'{id}' is an {kind}; only modulators can be route sources")
            }
            PatchError::BadIndex { what, index, max } => {
                format!("{what} {index} is out of range (the largest is {max})")
            }
            PatchError::TooMany { what, max } => format!("at most {max} {what} are allowed"),
            PatchError::WrongValueCount {
                id,
                expected,
                found,
            } => format!("'{id}' has {found} parameter values, its kind has {expected}"),
            PatchError::NoSuchRoute { route } => format!("there is no route {route}"),
            PatchError::DuplicateRoute { route } => format!("the route {route} already exists"),
            PatchError::NotModulatable { addr } => format!("{addr} cannot be modulated"),
            PatchError::NotAutomatable { addr } => {
                format!("{addr} cannot be exposed to the host")
            }
            PatchError::RouteRateMismatch { route, src, dst } => format!(
                "the route {route} sends a {} source to a {} destination",
                rate_name(*src),
                rate_name(*dst)
            ),
            PatchError::InUseAsAmpEnv { id, action } => {
                format!("'{id}' is the amplitude envelope and cannot be {action}")
            }
            PatchError::ModuleDisabled { id } => format!("'{id}' is disabled"),
            PatchError::NotCanonical {
                param,
                value,
                canonical,
            } => format!("{param} is stored as {value:?}, it must be stored as {canonical:?}"),
            PatchError::MissingAmpEnv => {
                "the patch has no amplitude envelope (voice.amp_env)".to_owned()
            }
            PatchError::BadJson { message } => format!("not valid JSON: {message}"),
            PatchError::BadFile { message } => format!("not a patch file: {message}"),
            PatchError::MissingSchema => "the file has no \"schema\" number".to_owned(),
            PatchError::BadSchema { found } => {
                format!("\"schema\" must be a whole number, found {found}")
            }
            PatchError::SchemaTooNew { found, supported } => {
                format!("the file is schema {found}, this enth reads up to schema {supported}")
            }
            PatchError::MigrationFailed { from, message } => {
                format!("migrating from schema {from} failed: {message}")
            }
            PatchError::UnknownOp { found } => {
                if found.is_empty() {
                    "the op has no verb".to_owned()
                } else {
                    format!("no known verb among the keys {}", found.join(", "))
                }
            }
            PatchError::BadOp { verb, message } => format!("{verb}: {message}"),
        }
    }

    /// What to do about it, in one line.
    pub fn hint(&self) -> String {
        match self {
            PatchError::Addr(e) => e.hint(),
            PatchError::Value(e) => e.hint(),
            PatchError::UnknownKind(_) => format!("the kinds are {}", kind_keys().join(", ")),
            PatchError::UnknownModule { existing, .. } => {
                if existing.is_empty() {
                    "the patch has no modules".to_owned()
                } else {
                    format!("the modules are {}", existing.join(", "))
                }
            }
            PatchError::UnknownParam { owner, keys, .. } => {
                format!("{owner} has: {}", keys.join(", "))
            }
            PatchError::DuplicateId { .. } => {
                "pick another id, or leave it out to take the next free one".to_owned()
            }
            PatchError::WrongSection { expected, .. } => {
                format!("put it in {expected}")
            }
            PatchError::WrongKind { expected, .. } => format!("pick an {expected} module"),
            PatchError::NotASource { .. } => {
                "sources are adsr and lfo modules, velocity, key, modwheel and macro1..macro8"
                    .to_owned()
            }
            PatchError::BadIndex { .. } => "leave the index out to append".to_owned(),
            PatchError::TooMany { .. } => "remove one first".to_owned(),
            PatchError::WrongValueCount { .. } => {
                "write the parameters by key in \"params\"".to_owned()
            }
            PatchError::NoSuchRoute { .. } => "list the routes with `enth get routes`".to_owned(),
            PatchError::DuplicateRoute { .. } => {
                "change its depth instead: leave out the index".to_owned()
            }
            PatchError::NotModulatable { .. } => {
                "only continuous parameters flagged `mod` in `enth params <kind>` take routes"
                    .to_owned()
            }
            PatchError::NotAutomatable { .. } => {
                "structural parameters and seeds are not automatable".to_owned()
            }
            PatchError::RouteRateMismatch { .. } => {
                "a per-voice source (an envelope, a voice LFO, velocity, key) can only drive per-voice destinations; use a global LFO, a macro or the mod wheel for effects and master"
                    .to_owned()
            }
            PatchError::InUseAsAmpEnv { .. } => {
                "set voice.amp_env to another envelope first (`{\"amp_env\": \"env2\"}`)".to_owned()
            }
            PatchError::ModuleDisabled { .. } => "enable it first".to_owned(),
            PatchError::NotCanonical { .. } => {
                "store -0 as 0, and a phase of 360 as 0 (use `ParamSpec::clamp`)".to_owned()
            }
            PatchError::MissingAmpEnv => {
                "add an adsr module to modulators and name it in voice.amp_env".to_owned()
            }
            PatchError::BadJson { .. } => "check commas, quotes and brackets".to_owned(),
            PatchError::BadFile { .. } => "see `enth schema patch`".to_owned(),
            PatchError::MissingSchema => format!(
                "add \"schema\": {} (the current version)",
                crate::file::SCHEMA
            ),
            PatchError::BadSchema { .. } => {
                format!("write \"schema\": {}", crate::file::SCHEMA)
            }
            PatchError::SchemaTooNew { .. } => "upgrade enth to open this file".to_owned(),
            PatchError::MigrationFailed { .. } => {
                "the file does not look like its schema version; fix it by hand".to_owned()
            }
            PatchError::UnknownOp { .. } => format!("the verbs are {}", crate::wire::VERBS.join(", ")),
            PatchError::BadOp { verb, .. } => {
                format!("see `enth guide --topic ops` for the shape of {verb}")
            }
        }
    }

    /// Likely intended spellings of what was written: the nearest module ids, parameter keys, kinds or enum items.
    /// Empty when nothing is close.
    pub fn did_you_mean(&self) -> Vec<String> {
        match self {
            PatchError::UnknownModule { id, existing } => {
                closest(id, existing.iter().map(String::as_str))
            }
            PatchError::UnknownParam { addr, keys, .. } => {
                let param = addr.rsplit_once('.').map_or(addr.as_str(), |(_, p)| p);
                closest(param, keys.iter().copied())
            }
            // An address whose parameter key exists on no module kind (so no module was consulted): suggest from
            // every key there is.
            PatchError::Addr(AddrError::UnknownParam { text }) => {
                let param = text.rsplit_once('.').map_or(text.as_str(), |(_, p)| p);
                closest(param, all_param_keys())
            }
            PatchError::Addr(AddrError::BadSection { text }) => {
                closest(text, Section::ALL.iter().map(|s| s.key()))
            }
            PatchError::UnknownKind(UnknownKind(k)) => closest(k, kind_keys()),
            PatchError::Value(ValueError::BadValue {
                text,
                expected: Expected::Item(items),
                ..
            }) => closest(text, items.iter().map(|i| i.key)),
            PatchError::UnknownOp { found } => found
                .iter()
                .flat_map(|k| closest(k, crate::wire::VERBS.iter().copied()))
                .collect(),
            _ => Vec::new(),
        }
    }
}

fn rate_name(r: Rate) -> &'static str {
    match r {
        Rate::Voice => "per-voice",
        Rate::Global => "global",
    }
}

/// Every parameter key of every module kind, `voice` and `master`.
fn all_param_keys() -> Vec<&'static str> {
    use crate::catalogue::{CATALOGUE, MASTER_PARAMS, VOICE_PARAMS};
    let mut keys: Vec<&'static str> = CATALOGUE
        .iter()
        .flat_map(|m| m.params.iter().map(|p| p.key))
        .chain(VOICE_PARAMS.iter().map(|p| p.key))
        .chain(MASTER_PARAMS.iter().map(|p| p.key))
        .collect();
    keys.sort_unstable();
    keys.dedup();
    keys
}

fn kind_keys() -> Vec<&'static str> {
    ModuleKind::ALL.iter().map(|k| k.key()).collect()
}

impl fmt::Display for PatchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message())
    }
}

impl std::error::Error for PatchError {}

/// Up to three candidates close to `input`, nearest first (ties alphabetical). "Close" is an edit distance of one
/// for texts under six characters and at most a third of the longer text (at least 2) above that, or a prefix match.
pub fn closest<'a>(input: &str, pool: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let input_lc = input.to_ascii_lowercase();
    let mut scored: Vec<(usize, &str)> = pool
        .into_iter()
        .filter_map(|c| {
            let c_lc = c.to_ascii_lowercase();
            let d = edit_distance(&input_lc, &c_lc);
            // Short words (`set`, `wave`) tolerate one slip, longer ones a third of their length.
            let longest = input.chars().count().max(c.chars().count());
            let limit = if longest >= 6 {
                (longest / 3).max(2)
            } else {
                1
            };
            let related = !input_lc.is_empty()
                && (c_lc.starts_with(&input_lc) || input_lc.starts_with(&c_lc));
            (d <= limit || related).then_some((d, c))
        })
        .collect();
    scored.sort_unstable();
    scored.dedup();
    scored
        .into_iter()
        .take(3)
        .map(|(_, c)| c.to_owned())
        .collect()
}

/// Optimal string alignment distance (insert, delete, substitute, swap two neighbours).
fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev2: Vec<usize> = vec![0; b.len() + 1];
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur: Vec<usize> = vec![0; b.len() + 1];
    for i in 1..=a.len() {
        cur[0] = i;
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            let mut d = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                d = d.min(prev2[j - 2] + 1);
            }
            cur[j] = d;
        }
        core::mem::swap(&mut prev2, &mut prev);
        core::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

/// One problem and where it is: a path into the file or patch (`generators[1].params.cutoff`, `routes[0].dst`,
/// `voice.amp_env`, `ops[2]`), empty when the whole input is at fault.
#[derive(Clone, Debug, PartialEq)]
pub struct Violation {
    /// Where, as a path.
    pub path: String,
    /// What.
    pub error: PatchError,
}

impl Violation {
    /// A problem at `path`.
    pub fn new(path: impl Into<String>, error: impl Into<PatchError>) -> Self {
        Violation {
            path: path.into(),
            error: error.into(),
        }
    }
}

impl fmt::Display for Violation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.path.is_empty() {
            write!(f, "{}", self.error)
        } else {
            write!(f, "{}: {}", self.path, self.error)
        }
    }
}

/// A non-empty list of [`Violation`]s, in document order: what `validate`, `load` and the wire parser return.
#[derive(Clone, Debug, PartialEq)]
pub struct Invalid(pub Vec<Violation>);

impl Invalid {
    /// One problem.
    pub fn one(path: impl Into<String>, error: impl Into<PatchError>) -> Self {
        Invalid(vec![Violation::new(path, error)])
    }

    /// The codes of all problems, in order.
    pub fn codes(&self) -> Vec<&'static str> {
        self.0.iter().map(|v| v.error.code()).collect()
    }
}

impl fmt::Display for Invalid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, v) in self.0.iter().enumerate() {
            if i > 0 {
                f.write_str("\n")?;
            }
            write!(f, "{v}")?;
        }
        Ok(())
    }
}

impl std::error::Error for Invalid {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edit_distance_basics() {
        assert_eq!(edit_distance("", ""), 0);
        assert_eq!(edit_distance("cutoff", "cutoff"), 0);
        assert_eq!(edit_distance("cutof", "cutoff"), 1);
        assert_eq!(edit_distance("cutoff", "cuotff"), 1, "a swap is one edit");
        assert_eq!(edit_distance("abc", "xyz"), 3);
        assert_eq!(edit_distance("kitten", "sitting"), 3);
    }

    #[test]
    fn closest_picks_near_misses_only() {
        let keys = ["cutoff", "resonance", "keytrack", "drive", "mode"];
        assert_eq!(closest("cutof", keys.iter().copied()), ["cutoff"]);
        assert_eq!(closest("resonanse", keys.iter().copied()), ["resonance"]);
        assert_eq!(closest("CUTOFF", keys.iter().copied()), ["cutoff"]);
        assert_eq!(
            closest("zzzzzz", keys.iter().copied()),
            Vec::<String>::new()
        );
        let many = ["osc1", "osc2", "osc3", "osc4", "osc5"];
        assert_eq!(closest("osc", many.iter().copied()).len(), 3);
    }

    #[test]
    fn unknown_module_suggests_and_lists() {
        let e = PatchError::UnknownModule {
            id: "filt1".into(),
            existing: vec!["osc1".into(), "filter1".into(), "env1".into()],
        };
        assert_eq!(e.code(), "unknown_module");
        assert_eq!(e.did_you_mean(), ["filter1"]);
        assert!(e.hint().contains("osc1, filter1, env1"), "{}", e.hint());
    }
}
