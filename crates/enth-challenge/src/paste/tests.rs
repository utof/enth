//! Lane RC's corpus of mangled pastes (GR5's row "codes survive a paste through the chat apps, smart punctuation
//! included"): every code kind, through every damage a chat app or a phone does, alone and combined, found inside whole
//! messages.

use proptest::prelude::*;

use super::*;
use crate::result::{
    CardAssists, Commitment, EarAnswer, EarChallengeRef, ScorecardFields, ScorerSet,
};
use crate::spec::SpecPreset;

fn challenge(level: u8, seed: u64) -> ChallengeCode {
    ChallengeCode::short(SpecPreset::by_id(level).unwrap().spec(1, seed)).unwrap()
}

fn scorecard() -> Scorecard {
    Scorecard::from_fields(ScorecardFields {
        challenge: challenge(2, 42),
        engine: crate::code::EngineVersion::CURRENT,
        scorers: ScorerSet::V1,
        sound_tenths: 934,
        combined_tenths: 912,
        passed: true,
        submits: 4,
        time_ds: 1610,
        assists: CardAssists::default(),
        commitment: Commitment::from_bits(0x00FE_DCBA_9876_5432).unwrap(),
    })
    .unwrap()
}

fn answer(level: u8, seed: u64) -> AnswerCode {
    let c = challenge(level, seed);
    let target = crate::random::target(c.spec()).unwrap().patch;
    AnswerCode::new(c, 0xA5A5_5A5A, &target).unwrap()
}

fn ear() -> EarResult {
    EarResult::new(
        EarChallengeRef {
            game: 1,
            generator: 1,
            level: 2,
            seed: 77,
            count: 3,
        },
        vec![
            Some(EarAnswer::Choice(2)),
            None,
            Some(EarAnswer::Taps(vec![0, 250, 500])),
        ],
        42_000,
        0,
        false,
    )
    .unwrap()
}

/// One code of each kind, with its canonical text.
fn corpus_codes() -> Vec<String> {
    vec![
        challenge(2, 42).encode(),
        scorecard().encode(),
        answer(1, 5).encode(),
        answer(4, 9).encode(),
        ear().encode(),
    ]
}

/// What chat apps and phones do to a code (each a function of the canonical text).
/// A named damage.
type Mangle = (&'static str, fn(&str) -> String);

fn mangles() -> Vec<Mangle> {
    vec![
        ("as is", |c| c.to_owned()),
        ("in a sentence", |c| {
            format!("hey, try this one: {c} (took me ages)")
        }),
        ("lower case", |c| c.to_ascii_lowercase()),
        ("en dashes", |c| c.replace('-', "\u{2013}")),
        ("em dashes", |c| c.replace('-', "\u{2014}")),
        ("minus signs", |c| c.replace('-', "\u{2212}")),
        ("non-breaking hyphens", |c| c.replace('-', "\u{2011}")),
        ("full-width hyphens", |c| c.replace('-', "\u{FF0D}")),
        ("double hyphen made a dash", |c| {
            c.replace('-', "--").replace("--", "\u{2014}")
        }),
        ("zero-width spaces", |c| {
            c.chars().flat_map(|x| [x, '\u{200B}']).collect()
        }),
        ("zero-width joiners and a BOM", |c| {
            format!("\u{FEFF}{}", c.replace('-', "-\u{200D}\u{2060}"))
        }),
        ("soft hyphens", |c| c.replace('-', "\u{00AD}-")),
        ("no-break spaces", |c| c.replace('-', "\u{00A0}")),
        ("narrow no-break spaces", |c| c.replace('-', "\u{202F}")),
        ("spaces", |c| c.replace('-', " ")),
        ("no hyphens", |c| c.replace('-', "")),
        ("markdown bold", |c| format!("**{c}**")),
        ("markdown italic", |c| format!("_{c}_")),
        ("whatsapp bold", |c| format!("*{c}*")),
        ("strike", |c| format!("~{c}~")),
        ("inline code", |c| format!("`{c}`")),
        ("code block", |c| format!("```\n{c}\n```")),
        ("curly quotes", |c| format!("\u{201C}{c}\u{201D}")),
        ("single quotes", |c| format!("\u{2018}{c}\u{2019}")),
        ("guillemets", |c| format!("\u{00AB}{c}\u{00BB}")),
        ("a full stop", |c| format!("Here: {c}.")),
        ("brackets", |c| format!("[{c}]")),
        ("a link", |c| {
            format!("https://example.invalid/duel?c={c}&x=1")
        }),
        ("full-width letters", |c| {
            c.chars()
                .map(|x| {
                    if x.is_ascii_alphanumeric() {
                        char::from_u32(u32::from(x) + 0xFEE0).unwrap()
                    } else {
                        x
                    }
                })
                .collect()
        }),
        ("wrapped at every 5th hyphen", |c| wrap(c, "\n")),
        ("wrapped with CRLF", |c| wrap(c, "\r\n")),
        ("wrapped and indented", |c| wrap(c, "\n    ")),
        ("wrapped, the hyphen moved down", |c| {
            wrap(c, "\n").replace("-\n", "\n-")
        }),
        ("smart dashes, zero-width and wrapped", |c| {
            wrap(c, "\n").replace('-', "\u{2013}\u{200B}")
        }),
        ("an ideographic space before", |c| {
            format!("\u{3000}{c}\u{3000}")
        }),
    ]
}

/// A line break after every fifth hyphen (a chat app wrapping a long code).
fn wrap(c: &str, sep: &str) -> String {
    let mut out = String::new();
    let mut hyphens = 0;
    for ch in c.chars() {
        out.push(ch);
        if ch == '-' {
            hyphens += 1;
            if hyphens % 5 == 0 {
                out.push_str(sep);
            }
        }
    }
    out
}

#[test]
fn every_code_survives_every_mangle() {
    for code in corpus_codes() {
        for (name, mangle) in mangles() {
            let pasted = paste(&mangle(&code));
            let found: Vec<String> = pasted.found.iter().map(Found::encode).collect();
            assert_eq!(found, vec![code.clone()], "{name}: {:?}", mangle(&code));
            assert_eq!(pasted.refused.len(), 0, "{name}: {:?}", pasted.refused);
        }
    }
}

#[test]
fn a_whole_message_gives_every_code_in_order_each_once() {
    let card = scorecard();
    let [line1, line2] = card.share_lines();
    let a = answer(3, 11);
    let message = format!(
        "{line1}\n{line2}\n{card}\n\nand my answer (don't peek): {a}\nagain: {card}\n{}",
        ear()
    );
    let pasted = paste(&message);
    let found: Vec<String> = pasted.found.iter().map(Found::encode).collect();
    assert_eq!(
        found,
        vec![
            challenge(2, 42).encode(),
            card.encode(),
            a.encode(),
            ear().encode()
        ]
    );
    assert!(matches!(pasted.found[0], Found::Challenge(_)));
    assert!(matches!(pasted.found[1], Found::Scorecard(_)));
    assert!(matches!(pasted.found[2], Found::Answer(_)));
    assert!(matches!(pasted.found[3], Found::EarResult(_)));
    assert_eq!(pasted.refused.len(), 0, "{:?}", pasted.refused);
}

#[test]
fn two_codes_on_one_line_split_by_spaces_are_both_found() {
    let a = challenge(1, 1).encode();
    let b = challenge(5, 99).encode();
    let pasted = paste(&format!("{a} {b}"));
    let found: Vec<String> = pasted.found.iter().map(Found::encode).collect();
    assert_eq!(found, vec![a.clone(), b.clone()]);
    let pasted = paste(&format!("{} {}", a.replace('-', " "), b.replace('-', " ")));
    assert_eq!(pasted.found.len(), 2, "{pasted:?}");
}

#[test]
fn a_typo_or_a_newer_kind_is_refused_with_its_reason() {
    let card = scorecard().encode();
    let mut typo: Vec<char> = card.chars().collect();
    typo[7] = if typo[7] == 'X' { 'Y' } else { 'X' };
    let typo: String = typo.into_iter().collect();
    let pasted = paste(&format!("my result: {typo}"));
    assert_eq!(pasted.found.len(), 0);
    assert_eq!(pasted.refused.len(), 1, "{pasted:?}");
    assert_eq!(pasted.refused[0].error.code(), "bad_check");
    // A kind this build does not read (an ear challenge, until F1E wires CV3's decoder; or a newer kind).
    for kind in [3u8, 9] {
        let mut w = crate::result::Bits::header(kind, 1);
        w.put(12345, 40);
        let text = crate::result::text(w.into_symbols());
        let pasted = paste(&text);
        assert_eq!(pasted.refused.len(), 1, "{pasted:?}");
        assert_eq!(pasted.refused[0].error.code(), "code_too_new");
        assert!(pasted.refused[0].error.hint().contains("update enth"));
    }
}

#[test]
fn prose_gives_nothing() {
    for text in [
        "",
        "   ",
        "GG WP, that was a good one!",
        "ABCD EFGH JKMN PQRS TVWX YZ01 2345 6789",
        "call me at 555-0100-1234 tomorrow",
        "Level 2 · 93.4 % in 2:41 · 4 submits · 0 hints",
    ] {
        let pasted = paste(text);
        assert_eq!(pasted.found.len(), 0, "{text}: {pasted:?}");
    }
}

#[test]
fn normalize_maps_dashes_drops_zero_width_and_reads_full_width() {
    assert_eq!(normalize("a\u{2013}b\u{2014}c\u{2212}d"), "a-b-c-d");
    assert_eq!(normalize("a\u{200B}b\u{FEFF}c\u{00AD}d"), "abcd");
    assert_eq!(normalize("a\u{00A0}b\u{202F}c"), "a b c");
    assert_eq!(normalize("\u{FF21}\u{FF11}\u{FF0D}"), "A1-");
}

#[test]
fn decode_reads_every_kind() {
    assert!(matches!(
        decode(&challenge(1, 2).encode()),
        Ok(Found::Challenge(_))
    ));
    assert!(matches!(
        decode(&scorecard().encode()),
        Ok(Found::Scorecard(_))
    ));
    assert!(matches!(
        decode(&answer(1, 2).encode()),
        Ok(Found::Answer(_))
    ));
    assert!(matches!(decode(&ear().encode()), Ok(Found::EarResult(_))));
    assert_eq!(decode("").unwrap_err().code(), "empty_code");
}

proptest! {
    #[test]
    fn any_text_never_panics(s in "\\PC{0,300}") {
        let _ = paste(&s);
    }

    #[test]
    fn code_soup_never_panics(s in "[0-9A-Z \\-\n\u{2013}\u{200B}]{0,300}") {
        let _ = paste(&s);
    }

    #[test]
    fn a_challenge_code_in_noise_is_found(level in 1u8..=5, seed in 0u64..(1 << 40), before in "[a-z ,.!?]{0,40}", after in "[a-z ,.!?]{0,40}") {
        let code = challenge(level, seed).encode();
        let pasted = paste(&format!("{before} {code} {after}"));
        let found: Vec<String> = pasted.found.iter().map(Found::encode).collect();
        prop_assert_eq!(found, vec![code]);
    }
}
