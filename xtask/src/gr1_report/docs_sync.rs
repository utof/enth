//! Keeps `docs/gr1.md` and its two sheets honest: the field tables name every field the readers know, the sample
//! report block is the head of the committed fixture report, and every relative link points at a file.

use std::fs;
use std::path::{Path, PathBuf};

use super::{baseline, schema, survey};

fn docs() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("docs")
}

fn read(name: &str) -> String {
    let path = docs().join(name);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The text of `doc` between the heading `from` and the next heading of the same or a higher level.
fn section<'a>(doc: &'a str, from: &str) -> &'a str {
    let start = doc
        .find(from)
        .unwrap_or_else(|| panic!("no heading {from:?}"));
    let level = from.chars().take_while(|c| *c == '#').count();
    let rest = &doc[start + from.len()..];
    let end = rest
        .match_indices("\n#")
        .find(|(i, _)| {
            let hashes = rest[i + 1..].chars().take_while(|c| *c == '#').count();
            hashes <= level
        })
        .map_or(rest.len(), |(i, _)| i);
    &rest[..end]
}

#[test]
fn the_field_tables_name_every_field_the_readers_know() {
    let doc = read("gr1.md");
    let cases: [(&str, &str, &[&str]); 5] = [
        ("### Top-level fields", "round log", schema::TOP_FIELDS),
        ("### The result", "result", schema::RESULT_FIELDS),
        ("### The result", "assists", schema::ASSIST_FIELDS),
        ("## The baseline file", "baseline", baseline::FIELDS),
        ("## The survey file", "survey", survey::FIELDS),
    ];
    for (heading, what, fields) in cases {
        let text = section(&doc, heading);
        for field in fields {
            // A table row starts with the field in backticks, alone or in a list (`a`, `b`).
            let named = text
                .lines()
                .filter(|l| l.starts_with('|'))
                .any(|l| l.contains(&format!("`{field}`")));
            assert!(named, "docs/gr1.md, {what}: no table row names `{field}`");
        }
    }
}

#[test]
fn the_sample_report_block_is_the_head_of_the_fixture_report() {
    let doc = read("gr1.md");
    let open = "```text\nGR1 report";
    let start = doc.find(open).expect("the sample block") + "```text\n".len();
    let block = &doc[start..];
    let block = &block[..block.find("\n```").expect("the block ends")];
    let report = fs::read_to_string(super::fixtures::dir().join("report.txt")).unwrap();
    assert!(
        report.starts_with(block),
        "the sample in docs/gr1.md differs from the head of xtask/fixtures/gr1/report.txt; paste the new head in"
    );
}

#[test]
fn every_relative_link_in_the_kit_docs_points_at_a_file() {
    for name in ["gr1.md", "gr1-friends.md", "gr1-owner.md"] {
        let doc = read(name);
        let mut rest = doc.as_str();
        while let Some(at) = rest.find("](") {
            rest = &rest[at + 2..];
            let Some(end) = rest.find(')') else { break };
            let target = &rest[..end];
            if target.starts_with("http") || target.starts_with('#') {
                continue;
            }
            let file = target.split('#').next().unwrap_or(target);
            assert!(
                docs().join(file).is_file(),
                "docs/{name} links to docs/{file}, which is not a file"
            );
        }
    }
}

#[test]
fn the_sheets_point_at_each_other_and_at_the_install_page() {
    assert!(read("gr1-friends.md").contains("](friends-install.md)"));
    assert!(read("gr1-owner.md").contains("](gr1-friends.md)"));
    let main = read("gr1.md");
    assert!(main.contains("](gr1-friends.md)") && main.contains("](gr1-owner.md)"));
}
