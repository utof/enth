//! Design-token rule (docs/design-system.md), the part clippy can't check: no associated colour
//! constants (`Color32::WHITE`), no egui style structs built by hand, no `add_space(6.0)`, and no
//! float literal in widget or editor code other than 0, ½, 1, 2 (layout maths) and 100 (percent).
//! Only `src/theme/` may. The gallery and the cards' display shapes (`editor/cards/shapes.rs`) are
//! content (sample signals, models of a module's sound), so the float rule skips them.
//! Comments and string literals are ignored.
#![allow(
    clippy::disallowed_methods,
    reason = "the test reads the crate's own source files"
)]

use std::path::{Path, PathBuf};

const BANNED: &[&str] = &[
    "Color32::",
    "Rgba::",
    "Hsva::",
    "FontId::",
    "Stroke::new",
    "CornerRadius::same",
    "Margin::same",
    "Margin::symmetric",
    "Shadow {",
];

/// Floats that are layout maths or unit conversion (100 for percent), not looks.
const PLAIN_FLOATS: &[&str] = &["0.0", "0.5", "1.0", "2.0", "100.0"];

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// The source with comments and string/char literals blanked (newlines kept).
fn code_only(source: &str) -> String {
    let chars: Vec<char> = source.chars().collect();
    let mut out = String::with_capacity(source.len());
    let mut i = 0;
    while i < chars.len() {
        let (c, next) = (chars[i], chars.get(i + 1).copied());
        if c == '/' && next == Some('/') {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
        } else if c == '/' && next == Some('*') {
            while i + 1 < chars.len() && !(chars[i] == '*' && chars[i + 1] == '/') {
                out.push(if chars[i] == '\n' { '\n' } else { ' ' });
                i += 1;
            }
            i += 2;
        } else if c == '"' {
            i += 1;
            while i < chars.len() && chars[i] != '"' {
                if chars[i] == '\\' {
                    i += 1;
                }
                out.push(if chars.get(i) == Some(&'\n') {
                    '\n'
                } else {
                    ' '
                });
                i += 1;
            }
            i += 1;
            out.push_str("\"\"");
        } else if c == '\'' && chars.get(i + 2) == Some(&'\'') {
            out.push_str("' '");
            i += 3;
        } else {
            out.push(c);
            i += 1;
        }
    }
    out
}

/// Float literals (`12.0`, `0.25f32`) with their line numbers.
fn floats(code: &str) -> Vec<(usize, String)> {
    let chars: Vec<char> = code.chars().collect();
    let mut found = Vec::new();
    let mut line = 1;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '\n' {
            line += 1;
        }
        let starts_number = c.is_ascii_digit()
            && (i == 0
                || !(chars[i - 1].is_alphanumeric() || chars[i - 1] == '_' || chars[i - 1] == '.'));
        if !starts_number {
            i += 1;
            continue;
        }
        let start = i;
        while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '_') {
            i += 1;
        }
        if chars.get(i) == Some(&'.') && chars.get(i + 1).is_some_and(char::is_ascii_digit) {
            i += 1;
            while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '_') {
                i += 1;
            }
            let literal: String = chars[start..i].iter().collect();
            found.push((line, literal.trim_end_matches('_').to_owned()));
        }
    }
    found
}

#[test]
fn widgets_use_tokens_only() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    rust_files(&src, &mut files);
    files.sort();
    let mut problems = Vec::new();
    for path in &files {
        let relative = path.strip_prefix(&src).unwrap();
        if relative.starts_with("theme") {
            continue;
        }
        let code = code_only(&std::fs::read_to_string(path).unwrap());
        for (n, line) in code.lines().enumerate() {
            for banned in BANNED {
                if line.contains(banned) {
                    problems.push(format!("{}:{}: `{banned}`", relative.display(), n + 1));
                }
            }
            if line.contains("add_space(")
                && line
                    .split("add_space(")
                    .nth(1)
                    .is_some_and(|rest| rest.starts_with(|c: char| c.is_ascii_digit()))
            {
                problems.push(format!(
                    "{}:{}: add_space with a literal",
                    relative.display(),
                    n + 1
                ));
            }
        }
        // The gallery and the display shapes are content (sample signals, example values, models of a
        // sound); widgets and the editor get the float rule.
        let content = relative == Path::new("editor/cards/shapes.rs");
        if (relative.starts_with("widgets") || relative.starts_with("editor")) && !content {
            for (n, literal) in floats(&code) {
                if !PLAIN_FLOATS.contains(&literal.as_str()) {
                    problems.push(format!(
                        "{}:{n}: float literal {literal}",
                        relative.display()
                    ));
                }
            }
        }
    }
    assert!(
        problems.is_empty(),
        "raw visual values outside src/theme (add a token instead):\n  {}",
        problems.join("\n  ")
    );
}

#[test]
fn the_scanner_finds_what_it_should() {
    let code = code_only(
        "let a = 12.5; // 3.0\nlet s = \"Color32::RED 4.0\";\nui.add_space(6.0); Color32::WHITE",
    );
    assert_eq!(
        floats(&code),
        vec![(1, "12.5".to_owned()), (3, "6.0".to_owned())]
    );
    assert!(code.contains("Color32::WHITE") && !code.contains("RED"));
}
