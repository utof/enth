//! `random` never reaches `enth-render` (lane RG's acceptance, docs/phase-2.md): target generation works in
//! parameter space only, so a seed gives the same target on every machine whatever the renderer does.
//!
//! The crate depends on `enth-render` (for `model::Phrase::performance` and the scorers), so the dependency graph
//! cannot show it. This test reads the generator's sources instead (`random`, its mask, the priors and the phrase
//! table, each up to its test module): outside comments they name no crate but `enth_dsp` (the keyed draws),
//! `enth_params` and `enth_patch`, reach no module of this crate but `phrases`, `lesson_phrases`, `priors`,
//! `random`, `spec`, `fingerprint`, `code` and `model`, take nothing from `model` but the `Phrase`, `PhraseId`,
//! `PhraseNote`, `PhraseError` and `PhraseView` (a run-time phrase's borrowed notes, lane F3S) data types, and never
//! ask a phrase for its `Performance` (the one place `model` touches `enth-render`). The target fingerprint (`fingerprint`, F1r review M1) and the code module it names its
//! prefix from are read too, and the lesson-only phrase table (lane CV2), which the phrase lookup reaches.

const SOURCES: [(&str, &str); 7] = [
    ("src/random.rs", include_str!("../src/random.rs")),
    ("src/random/mask.rs", include_str!("../src/random/mask.rs")),
    ("src/priors.rs", include_str!("../src/priors.rs")),
    ("src/phrases.rs", include_str!("../src/phrases.rs")),
    // The lesson-only phrases (lane CV2, R2): `phrases::entry` looks in their table too.
    (
        "src/lesson_phrases.rs",
        include_str!("../src/lesson_phrases.rs"),
    ),
    ("src/fingerprint.rs", include_str!("../src/fingerprint.rs")),
    ("src/code.rs", include_str!("../src/code.rs")),
];

/// The code of a source file up to its test module: every line with its `//` comment (doc comments included) cut
/// off.
fn code(text: &str) -> Vec<(usize, &str)> {
    text.lines()
        .take_while(|l| l.trim() != "#[cfg(test)]")
        .enumerate()
        .map(|(i, l)| (i + 1, l.split("//").next().unwrap_or("")))
        .filter(|(_, l)| !l.trim().is_empty())
        .collect()
}

/// Every identifier in `line` that starts with `prefix` (`enth_render`, `crate::scorer`), up to the next
/// non-identifier character.
fn idents<'a>(line: &'a str, prefix: &str) -> Vec<&'a str> {
    let mut out = Vec::new();
    let mut rest = line;
    while let Some(at) = rest.find(prefix) {
        let tail = &rest[at + prefix.len()..];
        let end = tail
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .unwrap_or(tail.len());
        out.push(&tail[..end]);
        rest = &tail[end..];
    }
    out
}

#[test]
fn the_generator_never_reaches_the_renderer() {
    let mut checked = 0;
    for (file, text) in SOURCES {
        for (n, line) in code(text) {
            for krate in idents(line, "enth_") {
                assert!(
                    ["dsp", "params", "patch"].contains(&krate),
                    "{file}:{n}: the generator uses enth_{krate}: {line}"
                );
            }
            for module in idents(line, "crate::") {
                assert!(
                    [
                        "phrases",
                        "lesson_phrases",
                        "priors",
                        "random",
                        "spec",
                        "fingerprint",
                        "code",
                        "model"
                    ]
                    .contains(&module),
                    "{file}:{n}: the generator reaches crate::{module}: {line}"
                );
                if module == "model" {
                    let names = line.split("crate::model::").nth(1).unwrap_or("");
                    let names: Vec<&str> = names
                        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                        .filter(|w| !w.is_empty())
                        .collect();
                    assert!(
                        names.iter().all(|w| [
                            "Phrase",
                            "PhraseId",
                            "PhraseNote",
                            "PhraseError",
                            "PhraseView"
                        ]
                        .contains(w)),
                        "{file}:{n}: the generator takes more than phrase data from model: {line}"
                    );
                }
            }
            for banned in [".performance(", "Performance", "render(", "Render"] {
                assert!(
                    !line.contains(banned),
                    "{file}:{n}: {banned} in the generator: {line}"
                );
            }
            checked += 1;
        }
    }
    assert!(
        checked > 500,
        "only {checked} code lines read: did the files move?"
    );
}

/// The check itself bites: the same scan finds the renderer in `model.rs`, which builds the `Performance`.
#[test]
fn the_scan_finds_the_renderer_where_it_is() {
    let model = include_str!("../src/model.rs");
    let found = code(model)
        .iter()
        .any(|(_, l)| idents(l, "enth_").contains(&"render"));
    assert!(found);
    assert_eq!(
        idents("use enth_render::{Note}; enth_patch::Op", "enth_"),
        ["render", "patch"]
    );
}
