//! `enth guide`: the one-page agent workflow guide (at most about 6 KB) and its topic pages. The descriptor page is
//! generated from `enth-analysis`'s vocabulary, so it cannot drift from what `describe` and `check` measure.

use std::fmt::Write as _;

use enth_analysis::Feature;

/// The topics `--topic` takes.
pub const TOPICS: &[&str] = &["main", "expect", "ops", "descriptors"];

const MAIN: &str = include_str!("guide/main.md");
const EXPECT: &str = include_str!("guide/expect.md");
const OPS: &str = include_str!("guide/ops.md");

/// The page for a topic, or `None` for an unknown topic.
pub fn page(topic: &str) -> Option<String> {
    match topic {
        "main" => Some(MAIN.to_owned()),
        "expect" => Some(EXPECT.to_owned()),
        "ops" => Some(OPS.to_owned()),
        "descriptors" => Some(descriptors()),
        _ => None,
    }
}

fn descriptors() -> String {
    let mut s = String::from(
        "# enth guide: descriptors\n\n\
         Measured on the float render (never read off the patch). Null means not measurable, and `describe` says why\n\
         in `missing`. Short names work in expectations; the output always uses the path.\n\n\
         | path | short names | unit | meaning |\n|---|---|---|---|\n",
    );
    for f in Feature::ALL {
        let unit = match f.unit().label() {
            "" if f.unit() == enth_analysis::units::Unit::Bool => "true/false",
            "" => "0..1",
            u => u,
        };
        let _ = writeln!(
            s,
            "| `{}` | {} | {} | {} |",
            f.path(),
            f.aliases().join(", "),
            unit,
            f.meaning()
        );
    }
    s.push_str(
        "\nAlso in `describe`: `pitch.note` and `pitch.cents_dev`, `spectrum.centroid_track_hz` (start, middle, end:\n\
         a falling track is a closing filter), `envelope.beat_corrected` (false: a beat too slow to remove, sustain\n\
         uncertain), `envelope.release_extrapolated`, `envelope.duration_truncated`, `perf.source` (`flag`,\n\
         `default`, `wav`, `inferred`, `none`).\n",
    );
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_main_page_fits_in_6_kb_and_the_topics_in_8() {
        assert!(MAIN.len() <= 6 * 1024, "{}", MAIN.len());
        for t in TOPICS {
            let p = page(t).unwrap();
            assert!(p.len() <= 8 * 1024, "{t}: {}", p.len());
            assert!(p.is_ascii(), "{t} is not ASCII");
        }
        assert!(page("nope").is_none());
    }

    /// The starting points in the vocabulary table must parse as expectation lists.
    #[test]
    fn the_vocabulary_table_parses() {
        let table = MAIN
            .split("## Writing your own expectations")
            .nth(1)
            .unwrap();
        let mut rows = 0;
        for line in table
            .lines()
            .filter(|l| l.starts_with("| ") && !l.starts_with("| word"))
        {
            let cell = line.split('|').nth(2).unwrap().trim();
            for list in cell.split(';') {
                enth_analysis::parse_list(list).unwrap_or_else(|e| panic!("{list}: {e}"));
            }
            rows += 1;
        }
        assert!(rows >= 10, "{rows}");
    }

    /// Every short name the main page's table uses is a real descriptor with that path.
    #[test]
    fn the_descriptor_table_matches_the_vocabulary() {
        let table = MAIN.split("## Expectations").nth(1).unwrap();
        let table = table.split("## Reading check").next().unwrap();
        for line in table
            .lines()
            .filter(|l| l.starts_with("| ") && !l.starts_with("| name"))
        {
            let cells: Vec<&str> = line.split('|').map(str::trim).collect();
            let f = Feature::parse(cells[1]).unwrap_or_else(|| panic!("{}", cells[1]));
            assert_eq!(f.path(), cells[2]);
        }
    }
}
