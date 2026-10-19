//! The error type every command returns, and the stable exit codes (docs/agent-cli.md).
//!
//! Errors print as one JSON object on stdout (so a caller parses a single stream) and teach: `hint` says how to fix it.

use serde_json::{Map, Value, json};
use std::fmt;

/// `check` ran fine but at least one expectation failed.
pub const EXIT_CHECK_FAILED: i32 = 1;
/// Bad flags or unparsable arguments.
pub const EXIT_USAGE: i32 = 2;
/// Bad patch file, unknown parameter, value or unit out of range.
pub const EXIT_VALIDATION: i32 = 3;
/// Render or analysis failure (also: cannot write an output file).
pub const EXIT_RENDER: i32 = 4;
/// Input file not found.
pub const EXIT_NOT_FOUND: i32 = 5;

#[derive(Debug)]
pub struct CliError {
    pub exit: i32,
    pub code: &'static str,
    pub message: String,
    pub extra: Map<String, Value>,
}

pub type Res<T> = Result<T, CliError>;

impl CliError {
    #[must_use]
    pub fn new(exit: i32, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            exit,
            code,
            message: message.into(),
            extra: Map::new(),
        }
    }

    #[must_use]
    pub fn with(mut self, key: &str, value: impl Into<Value>) -> Self {
        self.extra.insert(key.to_string(), value.into());
        self
    }

    #[must_use]
    pub fn hint(self, text: impl Into<String>) -> Self {
        self.with("hint", text.into())
    }

    #[must_use]
    pub fn to_json(&self) -> Value {
        let mut m = self.extra.clone();
        m.insert("schema".into(), json!("enth.error/1"));
        m.insert("error".into(), json!(self.code));
        m.insert("message".into(), json!(self.message));
        m.insert("exit".into(), json!(self.exit));
        Value::Object(m)
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for CliError {}

/// Usage error for an argument that cannot be understood at all.
#[must_use]
pub fn usage(code: &'static str, message: impl Into<String>) -> CliError {
    CliError::new(EXIT_USAGE, code, message)
}

/// Validation error for a well-formed argument with a bad meaning.
#[must_use]
pub fn invalid(code: &'static str, message: impl Into<String>) -> CliError {
    CliError::new(EXIT_VALIDATION, code, message)
}
