//! A small TOML reader for the golden case files, returning a `serde_json::Value`.
//!
//! `cli-equal` has to read `tests/golden/cases/*.toml` (and their `expected/` files), and `enth-test-host`
//! may not take the `toml` crate: a dependency added to this package changes `Cargo.lock`, which only the
//! coordinator edits during P1b. So this is the subset those files use, written to be strict about it:
//!
//! - `key = value`, `[table]`, `[a.b]` and `[[array.of.tables]]` headers, dotted and quoted keys;
//! - basic `"…"` strings (with `\" \\ \n \r \t \b \f \uXXXX \UXXXXXXXX` escapes) and literal `'…'` strings;
//! - integers (decimal, with `_`), floats (with `_`, an exponent), booleans;
//! - arrays and inline tables, which may span lines and carry a trailing comma;
//! - `#` comments.
//!
//! Multi-line strings, dates and times, hexadecimal integers and non-finite floats are rejected with an error
//! that names the line, so a case file that grows past the subset fails loudly instead of being half-read.
//! The tests below read every golden case in the repository and compare the result with the shape the
//! harness (`tests/golden/src/source.rs`) deserialises.

use serde_json::{Map, Number, Value};

/// Parses a TOML document.
pub fn parse(text: &str) -> Result<Value, String> {
    let mut parser = Parser {
        chars: text.chars().collect(),
        at: 0,
        line: 1,
    };
    parser.document()
}

struct Parser {
    chars: Vec<char>,
    at: usize,
    line: usize,
}

type Res<T> = Result<T, String>;

impl Parser {
    fn err<T>(&self, message: impl std::fmt::Display) -> Res<T> {
        Err(format!("TOML line {}: {message}", self.line))
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.at).copied()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.at += 1;
        if c == '\n' {
            self.line += 1;
        }
        Some(c)
    }

    fn skip_spaces(&mut self) {
        while matches!(self.peek(), Some(' ' | '\t')) {
            self.at += 1;
        }
    }

    fn skip_comment(&mut self) {
        if self.peek() == Some('#') {
            while !matches!(self.peek(), None | Some('\n')) {
                self.at += 1;
            }
        }
    }

    /// Spaces, comments and newlines: between statements and between the items of an array.
    fn skip_blank(&mut self) {
        loop {
            self.skip_spaces();
            self.skip_comment();
            match self.peek() {
                Some('\n' | '\r') => {
                    self.bump();
                }
                _ => break,
            }
        }
    }

    fn document(&mut self) -> Res<Value> {
        let mut root = Value::Object(Map::new());
        let mut current: Vec<String> = Vec::new();
        loop {
            self.skip_blank();
            match self.peek() {
                None => return Ok(root),
                Some('[') => {
                    self.bump();
                    let array = self.peek() == Some('[');
                    if array {
                        self.bump();
                    }
                    let path = self.key_path()?;
                    self.expect(']')?;
                    if array {
                        self.expect(']')?;
                    }
                    self.end_of_statement()?;
                    if array {
                        let (last, parents) = path.split_last().ok_or("empty table name")?;
                        let table = self.walk(&mut root, parents)?;
                        let object = as_object(table, self.line)?;
                        let entry = object
                            .entry(last.clone())
                            .or_insert_with(|| Value::Array(Vec::new()));
                        match entry {
                            Value::Array(items) => items.push(Value::Object(Map::new())),
                            _ => return self.err(format!("{last:?} is not an array of tables")),
                        }
                    } else {
                        // Creating the table now keeps empty tables.
                        self.walk(&mut root, &path)?;
                    }
                    current = path;
                }
                Some(_) => {
                    let path = self.key_path()?;
                    self.skip_spaces();
                    self.expect('=')?;
                    self.skip_spaces();
                    let value = self.value()?;
                    self.end_of_statement()?;
                    let (last, parents) = path.split_last().ok_or("empty key")?;
                    let mut full = current.clone();
                    full.extend(parents.iter().cloned());
                    let line = self.line;
                    let table = self.walk(&mut root, &full)?;
                    let object = as_object(table, line)?;
                    if object.insert(last.clone(), value).is_some() {
                        return self.err(format!("duplicate key {last:?}"));
                    }
                }
            }
        }
    }

    /// The table at `path`, created on the way. An array of tables on the path stands for its last element.
    fn walk<'a>(&self, root: &'a mut Value, path: &[String]) -> Res<&'a mut Value> {
        let mut node = root;
        for key in path {
            let object = as_object(node, self.line)?;
            node = object
                .entry(key.clone())
                .or_insert_with(|| Value::Object(Map::new()));
            if let Value::Array(items) = node {
                node = items
                    .last_mut()
                    .ok_or_else(|| format!("TOML line {}: empty array at {key:?}", self.line))?;
            }
        }
        Ok(node)
    }

    fn end_of_statement(&mut self) -> Res<()> {
        self.skip_spaces();
        self.skip_comment();
        match self.peek() {
            None => Ok(()),
            Some('\n') => {
                self.bump();
                Ok(())
            }
            Some('\r') => {
                self.bump();
                if self.peek() == Some('\n') {
                    self.bump();
                }
                Ok(())
            }
            Some(c) => self.err(format!("unexpected {c:?} after a value")),
        }
    }

    fn expect(&mut self, want: char) -> Res<()> {
        match self.bump() {
            Some(c) if c == want => Ok(()),
            Some(c) => self.err(format!("expected {want:?}, found {c:?}")),
            None => self.err(format!("expected {want:?}, found the end of the file")),
        }
    }

    fn key_path(&mut self) -> Res<Vec<String>> {
        let mut path = Vec::new();
        loop {
            self.skip_spaces();
            path.push(self.key()?);
            self.skip_spaces();
            if self.peek() == Some('.') {
                self.bump();
            } else {
                return Ok(path);
            }
        }
    }

    fn key(&mut self) -> Res<String> {
        match self.peek() {
            Some('"') => self.basic_string(),
            Some('\'') => self.literal_string(),
            Some(c) if c.is_ascii_alphanumeric() || c == '_' || c == '-' => {
                let mut key = String::new();
                while let Some(c) = self.peek() {
                    if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                        key.push(c);
                        self.at += 1;
                    } else {
                        break;
                    }
                }
                Ok(key)
            }
            Some(c) => self.err(format!("{c:?} cannot start a key")),
            None => self.err("a key was expected"),
        }
    }

    fn value(&mut self) -> Res<Value> {
        match self.peek() {
            Some('"') => {
                if self.chars.get(self.at + 1) == Some(&'"')
                    && self.chars.get(self.at + 2) == Some(&'"')
                {
                    return self.err("multi-line strings are not supported");
                }
                Ok(Value::String(self.basic_string()?))
            }
            Some('\'') => {
                if self.chars.get(self.at + 1) == Some(&'\'')
                    && self.chars.get(self.at + 2) == Some(&'\'')
                {
                    return self.err("multi-line strings are not supported");
                }
                Ok(Value::String(self.literal_string()?))
            }
            Some('[') => self.array(),
            Some('{') => self.inline_table(),
            Some(_) => self.scalar(),
            None => self.err("a value was expected"),
        }
    }

    fn basic_string(&mut self) -> Res<String> {
        self.expect('"')?;
        let mut out = String::new();
        loop {
            match self.bump() {
                None | Some('\n') => return self.err("unterminated string"),
                Some('"') => return Ok(out),
                Some('\\') => {
                    let escaped = self.bump().ok_or("unterminated escape")?;
                    match escaped {
                        '"' => out.push('"'),
                        '\\' => out.push('\\'),
                        'n' => out.push('\n'),
                        'r' => out.push('\r'),
                        't' => out.push('\t'),
                        'b' => out.push('\u{8}'),
                        'f' => out.push('\u{c}'),
                        'u' | 'U' => {
                            let digits = if escaped == 'u' { 4 } else { 8 };
                            let mut code = 0u32;
                            for _ in 0..digits {
                                let d =
                                    self.bump().and_then(|c| c.to_digit(16)).ok_or_else(|| {
                                        format!("TOML line {}: bad \\{escaped} escape", self.line)
                                    })?;
                                code = code * 16 + d;
                            }
                            out.push(char::from_u32(code).ok_or_else(|| {
                                format!(
                                    "TOML line {}: \\{escaped}{code:x} is not a character",
                                    self.line
                                )
                            })?);
                        }
                        other => return self.err(format!("unknown escape \\{other}")),
                    }
                }
                Some(c) => out.push(c),
            }
        }
    }

    fn literal_string(&mut self) -> Res<String> {
        self.expect('\'')?;
        let mut out = String::new();
        loop {
            match self.bump() {
                None | Some('\n') => return self.err("unterminated string"),
                Some('\'') => return Ok(out),
                Some(c) => out.push(c),
            }
        }
    }

    fn array(&mut self) -> Res<Value> {
        self.expect('[')?;
        let mut items = Vec::new();
        loop {
            self.skip_blank();
            if self.peek() == Some(']') {
                self.bump();
                return Ok(Value::Array(items));
            }
            items.push(self.value()?);
            self.skip_blank();
            match self.bump() {
                Some(',') => {}
                Some(']') => return Ok(Value::Array(items)),
                Some(c) => {
                    return self.err(format!("expected ',' or ']' in an array, found {c:?}"));
                }
                None => return self.err("unterminated array"),
            }
        }
    }

    fn inline_table(&mut self) -> Res<Value> {
        self.expect('{')?;
        let mut object = Map::new();
        loop {
            self.skip_blank();
            if self.peek() == Some('}') {
                self.bump();
                return Ok(Value::Object(object));
            }
            let path = self.key_path()?;
            self.skip_spaces();
            self.expect('=')?;
            self.skip_spaces();
            let value = self.value()?;
            let (last, parents) = path.split_last().ok_or("empty key")?;
            let mut table = &mut object;
            for key in parents {
                let entry = table
                    .entry(key.clone())
                    .or_insert_with(|| Value::Object(Map::new()));
                table = as_object(entry, self.line)?;
            }
            if table.insert(last.clone(), value).is_some() {
                return self.err(format!("duplicate key {last:?}"));
            }
            self.skip_blank();
            match self.bump() {
                Some(',') => {}
                Some('}') => return Ok(Value::Object(object)),
                Some(c) => {
                    return self.err(format!(
                        "expected ',' or '}}' in an inline table, found {c:?}"
                    ));
                }
                None => return self.err("unterminated inline table"),
            }
        }
    }

    fn scalar(&mut self) -> Res<Value> {
        let mut token = String::new();
        while let Some(c) = self.peek() {
            if c.is_whitespace() || matches!(c, ',' | ']' | '}' | '#') {
                break;
            }
            token.push(c);
            self.at += 1;
        }
        match token.as_str() {
            "true" => return Ok(Value::Bool(true)),
            "false" => return Ok(Value::Bool(false)),
            "" => return self.err("a value was expected"),
            _ => {}
        }
        let clean: String = token.chars().filter(|c| *c != '_').collect();
        let numeric_start = clean
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_digit() || c == '+' || c == '-');
        if !numeric_start {
            return self.err(format!("cannot read {token:?} as a value"));
        }
        let is_int = clean
            .strip_prefix(['+', '-'])
            .unwrap_or(&clean)
            .chars()
            .all(|c| c.is_ascii_digit());
        if is_int {
            return clean
                .parse::<i64>()
                .map(|n| Value::Number(n.into()))
                .or_else(|_| self.err(format!("integer {token:?} does not fit 64 bits")));
        }
        // Dates, times, hexadecimal and the like fall through to here and fail the float parse.
        match clean.parse::<f64>() {
            Ok(x) => Number::from_f64(x)
                .map(Value::Number)
                .map_or_else(|| self.err(format!("{token:?} is not a finite number")), Ok),
            Err(_) => self.err(format!("cannot read {token:?} as a number")),
        }
    }
}

fn as_object(value: &mut Value, line: usize) -> Res<&mut Map<String, Value>> {
    value.as_object_mut().ok_or_else(|| {
        format!("TOML line {line}: a key path runs through a value that is not a table")
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn scalars_tables_arrays_and_comments() {
        let doc = parse(
            r#"
# a comment
description = "it's \"quoted\"\tand\u00e9"  # trailing
sample_rate = 48_000
seconds = 2.0
ratio = -1.5e-3
flag = true
literal = 'C:\raw'

[source]
kind = "patch"
notes = [
    { key = 48, velocity = 0.9, on_s = 0.0, off_s = 1.0 },
    { key = 55, on_s = 0.0103, off_s = 1, },
]

[source.patch]
schema = 1
"a.b" = [[1, 2], [3]]
voice = { amp_env = "env1", polyphony = 4 }
"#,
        )
        .unwrap();
        assert_eq!(doc["description"], "it's \"quoted\"\tandé");
        assert_eq!(doc["sample_rate"], 48_000);
        assert_eq!(doc["seconds"], 2.0);
        assert!(doc["seconds"].is_f64());
        assert_eq!(doc["ratio"], -0.0015);
        assert_eq!(doc["flag"], true);
        assert_eq!(doc["literal"], r"C:\raw");
        assert_eq!(doc["source"]["notes"][0]["velocity"], 0.9);
        assert_eq!(doc["source"]["notes"][1]["off_s"], 1);
        assert!(doc["source"]["notes"][1]["off_s"].is_i64());
        assert_eq!(doc["source"]["patch"]["a.b"], json!([[1, 2], [3]]));
        assert_eq!(doc["source"]["patch"]["voice"]["polyphony"], 4);
    }

    #[test]
    fn arrays_of_tables_and_dotted_keys() {
        let doc =
            parse("[[case]]\nname = \"a\"\nx.y = 1\n[[case]]\nname = \"b\"\n[case.sub]\nz = 2\n")
                .unwrap();
        assert_eq!(doc["case"][0]["name"], "a");
        assert_eq!(doc["case"][0]["x"]["y"], 1);
        assert_eq!(doc["case"][1]["sub"]["z"], 2);
    }

    #[test]
    fn what_is_outside_the_subset_is_an_error_with_a_line() {
        for (text, needle) in [
            ("a = \"\"\"x\"\"\"", "multi-line"),
            ("a = 1979-05-27", "cannot read"),
            ("a = 0x10", "cannot read"),
            ("a = nan", "cannot read"),
            ("a = 1\na = 2", "duplicate"),
            ("a = \"unterminated", "unterminated"),
            ("a = [1, 2", "unterminated"),
            ("a = 1 2", "unexpected"),
            ("[t]\n[t.a]\nb = 1\n[t]\nb = 2\nb = 3", "duplicate"),
            ("a = 99999999999999999999", "64 bits"),
        ] {
            let error = parse(text).unwrap_err();
            assert!(error.contains(needle), "{text:?}: {error}");
            assert!(error.contains("line"), "{text:?}: {error}");
        }
    }

    /// Every golden case in the repository reads, and has the shape the harness expects.
    #[test]
    fn every_golden_case_and_expected_file_reads() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/golden");
        let mut cases = 0;
        for entry in std::fs::read_dir(root.join("cases")).unwrap().flatten() {
            let text = std::fs::read_to_string(entry.path()).unwrap();
            let doc = parse(&text).unwrap_or_else(|e| panic!("{}: {e}", entry.path().display()));
            assert!(doc["sample_rate"].is_u64(), "{}", entry.path().display());
            assert!(doc["seconds"].is_number());
            assert!(doc["source"]["kind"].is_string());
            cases += 1;
        }
        assert!(cases >= 70, "only {cases} cases found");
        for entry in std::fs::read_dir(root.join("expected")).unwrap().flatten() {
            let text = std::fs::read_to_string(entry.path()).unwrap();
            let doc = parse(&text).unwrap_or_else(|e| panic!("{}: {e}", entry.path().display()));
            assert_eq!(doc["hash"].as_str().map(str::len), Some(64));
            assert!(doc["frames"].is_u64());
        }
    }
}
