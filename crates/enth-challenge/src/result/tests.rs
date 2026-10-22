//! Lane RC's tests of the frame, the scorecard and the ear result: round trips (proptest), every single substitution
//! and adjacent swap caught, truncation caught, every kind through code v1's decoder, the refusals, the float coding.

use proptest::prelude::*;

use super::*;
use crate::code::{SHORT_SEED_BITS, SHORT_SYMBOLS};
use crate::model::DEFAULT_PASS_MARK;
use crate::score::{Combined, SoundScore, StructureScore};
use crate::spec::{MAX_SEED, PERCENT_MAX, PRESETS};

// ---------------------------------------------------------------------------------------------------------------
// Strategies
// ---------------------------------------------------------------------------------------------------------------

pub(crate) fn any_short_challenge() -> impl Strategy<Value = ChallengeCode> {
    (0..PRESETS.len(), 0u64..(1 << SHORT_SEED_BITS))
        .prop_map(|(p, seed)| ChallengeCode::short(PRESETS[p].spec(1, seed)).unwrap())
}

fn any_engine() -> impl Strategy<Value = EngineVersion> {
    (any::<u8>(), any::<u8>(), any::<u8>()).prop_map(|(major, minor, patch)| EngineVersion {
        major,
        minor,
        patch,
    })
}

pub(crate) fn any_long_challenge() -> impl Strategy<Value = ChallengeCode> {
    (
        0..=MAX_SEED,
        0..=PERCENT_MAX,
        proptest::array::uniform6(proptest::option::of(0..=PERCENT_MAX)),
        0..=PERCENT_MAX,
        any_engine(),
        any::<u32>(),
    )
        .prop_map(|(seed, difficulty, amounts, musical, engine, prefix)| {
            let spec = RandomSpec {
                generator: 1,
                seed,
                difficulty,
                amounts,
                musical,
                template: 0,
            };
            ChallengeCode::long(spec, engine, TargetPrefix(prefix)).unwrap()
        })
}

fn any_challenge() -> impl Strategy<Value = ChallengeCode> {
    prop_oneof![3 => any_short_challenge(), 1 => any_long_challenge()]
}

fn any_assists() -> impl Strategy<Value = CardAssists> {
    (
        0..=HINTS_MAX,
        any::<bool>(),
        any::<bool>(),
        any::<bool>(),
        any::<bool>(),
    )
        .prop_map(
            |(hints, live_meter, dim_unused, closest, revealed)| CardAssists {
                hints,
                live_meter,
                dim_unused,
                closest,
                revealed,
            },
        )
}

pub(crate) fn any_scorecard() -> impl Strategy<Value = Scorecard> {
    (
        any_challenge(),
        any_engine(),
        0..=TENTHS_MAX,
        0..=TENTHS_MAX,
        any::<bool>(),
        1..=SUBMITS_MAX,
        any::<u16>(),
        any_assists(),
        0u64..(1 << Commitment::BITS),
    )
        .prop_map(
            |(challenge, engine, sound, combined, passed, submits, time_ds, assists, c)| {
                let mut assists = assists;
                assists.revealed &= passed;
                Scorecard::from_fields(ScorecardFields {
                    challenge,
                    engine,
                    scorers: ScorerSet::V1,
                    sound_tenths: sound,
                    combined_tenths: combined,
                    passed,
                    submits,
                    time_ds,
                    assists,
                    commitment: Commitment::from_bits(c).unwrap(),
                })
                .unwrap()
            },
        )
}

fn any_ear_answer() -> impl Strategy<Value = Option<EarAnswer>> {
    prop_oneof![
        Just(None),
        (0..EAR_CHOICES).prop_map(|i| Some(EarAnswer::Choice(i))),
        any::<f32>()
            .prop_filter("finite", |v| v.is_finite())
            .prop_map(|v| Some(EarAnswer::Value(v))),
        proptest::collection::vec(0..=EAR_TAP_MS_MAX, 0..=12).prop_map(|mut t| {
            t.sort_unstable();
            Some(EarAnswer::Taps(t))
        }),
    ]
}

pub(crate) fn any_ear_result() -> impl Strategy<Value = EarResult> {
    (
        any::<u8>(),
        any::<u8>(),
        any::<u8>(),
        0u64..(1 << EAR_SEED_BITS),
        proptest::collection::vec(any_ear_answer(), 0..=20),
        0u64..10_000_000,
        0..=HINTS_MAX,
        any::<bool>(),
    )
        .prop_map(
            |(game, generator, level, seed, answers, time_ms, hints, revealed)| {
                let challenge = EarChallengeRef {
                    game,
                    generator,
                    level,
                    seed,
                    count: answers.len() as u8,
                };
                EarResult::new(challenge, answers, time_ms, hints, revealed).unwrap()
            },
        )
}

// ---------------------------------------------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------------------------------------------

/// The symbols of a canonical code, hyphens dropped.
pub(crate) fn symbols_of(text: &str) -> Vec<u8> {
    text.bytes().filter(|&b| b != b'-').collect()
}

pub(crate) fn as_text(symbols: &[u8]) -> String {
    symbols.iter().map(|&b| char::from(b)).collect()
}

/// Every single substitution and every adjacent swap of `text`'s symbols, each refused by `decode`.
pub(crate) fn assert_damage_caught<T, E>(text: &str, decode: impl Fn(&str) -> Result<T, E>)
where
    T: fmt::Debug,
{
    let syms = symbols_of(text);
    for i in 0..syms.len() {
        for &b in code::ALPHABET {
            if b == syms[i] {
                continue;
            }
            let mut bad = syms.clone();
            bad[i] = b;
            assert!(
                decode(&as_text(&bad)).is_err(),
                "substitution at {i} -> {} not caught in {text}",
                char::from(b)
            );
        }
    }
    for i in 0..syms.len() - 1 {
        if syms[i] == syms[i + 1] {
            continue;
        }
        let mut bad = syms.clone();
        bad.swap(i, i + 1);
        assert!(
            decode(&as_text(&bad)).is_err(),
            "swap at {i} not caught in {text}"
        );
    }
}

/// Every truncation of `text` (dropping 1 to 8 symbols from the end, or one from anywhere) is refused.
pub(crate) fn assert_truncation_caught<T, E>(text: &str, decode: impl Fn(&str) -> Result<T, E>)
where
    T: fmt::Debug,
{
    let syms = symbols_of(text);
    for k in 1..=8.min(syms.len() - 1) {
        assert!(
            decode(&as_text(&syms[..syms.len() - k])).is_err(),
            "dropping the last {k} not caught"
        );
    }
    for i in 0..syms.len() {
        let mut bad = syms.clone();
        bad.remove(i);
        assert!(
            decode(&as_text(&bad)).is_err(),
            "dropping symbol {i} not caught"
        );
    }
}

/// What code v1's decoder (an R1-R3 build's) says: `code_too_new` at 64 symbols or fewer, `bad_length` above.
pub(crate) fn assert_old_builds_say_update(text: &str) {
    let n = symbols_of(text).len();
    let e = ChallengeCode::decode(text).unwrap_err();
    if n <= 64 {
        assert_eq!(e.code(), "code_too_new", "{n} symbols: {text}");
        assert!(e.hint().contains("update enth"), "{}", e.hint());
    } else {
        assert_eq!(e.code(), "bad_length", "{n} symbols: {text}");
    }
}

/// A version-15 code with the given header and fields and a valid 30-bit check and GF(32) symbol.
fn craft(kind: u8, layout: u8, fields: impl FnOnce(&mut Bits)) -> String {
    let mut w = Bits::header(kind, layout);
    fields(&mut w);
    text(with_check30(w.into_symbols()))
}

/// A scorecard's fields after its header, with a short challenge; `set` and `submits` as given.
fn card_fields(w: &mut Bits, generator: u16, set: u8, submits: u8) {
    w.put(0, 1);
    w.put(2, 8);
    w.put(u64::from(generator), 12);
    w.put(42, 40);
    put_engine(w, EngineVersion::CURRENT);
    w.put(u64::from(set), 8);
    w.put(934, 10);
    w.put(901, 10);
    w.put_bool(true);
    w.put(u64::from(submits), 7);
    w.put(1610, 16);
    w.put(0, 8);
    w.put(0xABC, 60);
}

fn sample_card() -> Scorecard {
    Scorecard::from_fields(ScorecardFields {
        challenge: ChallengeCode::short(SpecPreset::by_id(2).unwrap().spec(1, 42)).unwrap(),
        engine: EngineVersion::CURRENT,
        scorers: ScorerSet::V1,
        sound_tenths: 934,
        combined_tenths: 901,
        passed: true,
        submits: 4,
        time_ds: 1610,
        assists: CardAssists::default(),
        commitment: Commitment::from_bits(0x0012_3456_789A_BCDE).unwrap(),
    })
    .unwrap()
}

// ---------------------------------------------------------------------------------------------------------------
// The kinds
// ---------------------------------------------------------------------------------------------------------------

#[test]
fn the_reserved_kinds_are_the_documented_numbers() {
    assert_eq!(
        [
            KIND_SCORECARD,
            KIND_ANSWER,
            KIND_EAR_CHALLENGE,
            KIND_EAR_RESULT,
            KIND_LESSON_SIBLING
        ],
        [1, 2, 3, 4, 5]
    );
}

#[test]
fn kinds_are_distinct_nonzero_and_fit_five_bits() {
    for (i, kind) in KINDS.iter().enumerate() {
        assert!(
            (1..32).contains(kind),
            "kind {kind} must be 1..=31 (0 is never written)"
        );
        assert!(!KINDS[..i].contains(kind), "kind {kind} is reserved twice");
    }
}

#[test]
fn every_version_15_code_starts_with_f() {
    assert_eq!(CODE_VERSION_ESCAPE, 15);
    assert!(sample_card().encode().starts_with('F'));
    let ear = EarResult::new(
        EarChallengeRef {
            game: 1,
            generator: 1,
            level: 1,
            seed: 5,
            count: 0,
        },
        vec![],
        0,
        0,
        false,
    )
    .unwrap();
    assert!(ear.encode().starts_with('F'));
}

// ---------------------------------------------------------------------------------------------------------------
// The scorecard
// ---------------------------------------------------------------------------------------------------------------

proptest! {
    #[test]
    fn scorecards_round_trip(card in any_scorecard()) {
        let text = card.encode();
        prop_assert_eq!(Scorecard::decode(&text), Ok(card.clone()));
        let loose = text.replace('-', " ").to_ascii_lowercase();
        prop_assert_eq!(Scorecard::decode(&loose), Ok(card.clone()));
        let n = symbols_of(&text).len();
        match card.challenge().form() {
            CodeForm::Short => prop_assert_eq!(n, SCORECARD_SYMBOLS),
            CodeForm::Long => prop_assert!((72..=81).contains(&n), "{}", n),
        }
        prop_assert!(!text.contains("--"));
    }

    #[test]
    fn every_single_substitution_and_adjacent_swap_in_a_scorecard_is_caught(card in any_scorecard()) {
        assert_damage_caught(&card.encode(), Scorecard::decode);
    }

    #[test]
    fn a_truncated_scorecard_is_caught(card in any_scorecard()) {
        assert_truncation_caught(&card.encode(), Scorecard::decode);
    }

    #[test]
    fn old_builds_say_update_for_any_scorecard(card in any_scorecard()) {
        assert_old_builds_say_update(&card.encode());
    }

    #[test]
    fn ear_results_round_trip(ear in any_ear_result()) {
        let text = ear.encode();
        let back = EarResult::decode(&text).unwrap();
        prop_assert_eq!(back.challenge(), ear.challenge());
        prop_assert_eq!(back.time_ds(), ear.time_ds());
        prop_assert_eq!(back.hints(), ear.hints());
        prop_assert_eq!(back.revealed(), ear.revealed());
        prop_assert_eq!(back.answers().len(), ear.answers().len());
        for (a, b) in back.answers().iter().zip(ear.answers()) {
            if let (Some(EarAnswer::Value(x)), Some(EarAnswer::Value(y))) = (a, b) {
                prop_assert_eq!(x.to_bits(), if *y == 0.0 { 0 } else { y.to_bits() });
            } else {
                prop_assert_eq!(a, b);
            }
        }
        prop_assert_eq!(back.encode(), text);
    }

    #[test]
    fn every_single_substitution_and_adjacent_swap_in_an_ear_result_is_caught(ear in any_ear_result()) {
        assert_damage_caught(&ear.encode(), EarResult::decode);
    }

    #[test]
    fn a_truncated_ear_result_is_caught(ear in any_ear_result()) {
        assert_truncation_caught(&ear.encode(), EarResult::decode);
    }

    #[test]
    fn old_builds_say_update_for_any_ear_result(ear in any_ear_result()) {
        assert_old_builds_say_update(&ear.encode());
    }

    #[test]
    fn the_float_coding_is_exact(bits in any::<u32>()) {
        let v = f32::from_bits(bits);
        let mut w = Bits::default();
        put_f32(&mut w, v);
        let n = w.len();
        let syms = w.into_symbols();
        let back = take_f32(&mut Reader::at(&syms, 0)).unwrap();
        let want = if v == 0.0 { 0 } else { bits };
        prop_assert_eq!(back.to_bits(), want);
        prop_assert!(n == 16 || n == 33);
    }

    #[test]
    fn three_significant_digits_take_16_bits(x in 1e-5f32..1e6, negative in any::<bool>()) {
        // As generator v1 quantizes a value (`sig3`), which is also how a person types one.
        let v = crate::random::sig3(if negative { -x } else { x });
        let mut w = Bits::default();
        put_f32(&mut w, v);
        prop_assert_eq!(w.len(), 16, "{}", v);
    }

    #[test]
    fn garbage_never_panics(s in "\\PC{0,120}") {
        let _ = Scorecard::decode(&s);
        let _ = EarResult::decode(&s);
    }

    #[test]
    fn version_15_symbol_soup_never_panics(s in "F[0-9A-Z-]{0,90}") {
        let _ = Scorecard::decode(&s);
        let _ = EarResult::decode(&s);
    }
}

#[test]
fn a_scorecard_with_a_short_challenge_is_51_symbols_and_reads_update_on_old_builds() {
    let card = sample_card();
    let text = card.encode();
    assert_eq!(symbols_of(&text).len(), SCORECARD_SYMBOLS);
    assert_eq!(
        ChallengeCode::decode(&text).unwrap_err(),
        CodeError::TooNew {
            version: 15,
            supported: 1
        }
    );
    // The challenge code inside is the one the share line shows.
    assert_eq!(card.challenge().encode().len(), SHORT_SYMBOLS + 3);
}

#[test]
fn scorecard_refusals() {
    // Every field fine: decodes.
    let ok = craft(KIND_SCORECARD, 1, |w| card_fields(w, 1, 1, 4));
    assert!(Scorecard::decode(&ok).is_ok(), "{ok}");
    let cases: [(String, &str); 9] = [
        (
            craft(KIND_SCORECARD, 2, |w| card_fields(w, 1, 1, 4)),
            "code_too_new",
        ),
        (
            craft(KIND_SCORECARD, 0, |w| card_fields(w, 1, 1, 4)),
            "bad_field",
        ),
        (craft(0, 1, |w| card_fields(w, 1, 1, 4)), "bad_field"),
        (
            craft(KIND_ANSWER, 1, |w| card_fields(w, 1, 1, 4)),
            "wrong_kind",
        ),
        (craft(9, 1, |w| card_fields(w, 1, 1, 4)), "code_too_new"),
        (
            craft(KIND_SCORECARD, 1, |w| card_fields(w, 1, 2, 4)),
            "scorer_too_new",
        ),
        (
            craft(KIND_SCORECARD, 1, |w| card_fields(w, 1, 0, 4)),
            "bad_field",
        ),
        (
            craft(KIND_SCORECARD, 1, |w| card_fields(w, 2, 1, 4)),
            "code_too_new",
        ),
        (
            craft(KIND_SCORECARD, 1, |w| card_fields(w, 1, 1, 0)),
            "bad_field",
        ),
    ];
    for (text, want) in cases {
        let e = Scorecard::decode(&text).unwrap_err();
        assert_eq!(e.code(), want, "{text}: {e}");
        assert!(!e.hint().is_empty() && !e.to_string().is_empty());
    }
    // A newer generator and a newer scorer set say "update enth".
    for text in [
        craft(KIND_SCORECARD, 1, |w| card_fields(w, 2, 1, 4)),
        craft(KIND_SCORECARD, 1, |w| card_fields(w, 1, 2, 4)),
        craft(KIND_SCORECARD, 2, |w| card_fields(w, 1, 1, 4)),
    ] {
        assert!(
            Scorecard::decode(&text)
                .unwrap_err()
                .hint()
                .contains("update enth")
        );
    }
    // The form bit of the first symbol ('Z' instead of 'F') is reserved.
    let mut syms: Vec<u8> = symbols_of(&ok)
        .iter()
        .map(|&c| code::symbol_value(char::from(c)).unwrap())
        .collect();
    syms.pop();
    syms[0] |= 0b1_0000;
    let z = text(syms);
    assert!(z.starts_with('Z'));
    assert_eq!(
        Scorecard::decode(&z).unwrap_err(),
        ResultError::TooNew {
            what: "form",
            value: 1
        }
    );
    // A challenge code is not a scorecard.
    let challenge = ChallengeCode::short(SpecPreset::by_id(1).unwrap().spec(1, 1)).unwrap();
    assert_eq!(
        Scorecard::decode(&challenge.encode()).unwrap_err(),
        ResultError::WrongKind {
            found: None,
            expected: KIND_SCORECARD
        }
    );
    assert_eq!(Scorecard::decode(" - ").unwrap_err().code(), "empty_code");
    assert_eq!(Scorecard::decode("F0U0").unwrap_err().code(), "bad_symbol");
    assert_eq!(
        Scorecard::decode(&"0".repeat(MAX_SYMBOLS + 1))
            .unwrap_err()
            .code(),
        "bad_length"
    );
}

#[test]
fn a_reveal_before_a_pass_gives_no_scorecard() {
    let mut f = sample_card().fields().clone();
    f.passed = false;
    f.assists.revealed = true;
    assert_eq!(
        Scorecard::from_fields(f.clone()).unwrap_err().code(),
        "unencodable"
    );
    f.passed = true;
    assert!(
        Scorecard::from_fields(f).is_ok(),
        "a reveal after the pass is labelled"
    );
    let bad = craft(KIND_SCORECARD, 1, |w| {
        card_fields(w, 1, 1, 4);
    });
    // Flip passed off and revealed on in the crafted fields: refused as bad_field.
    let refused = craft(KIND_SCORECARD, 1, |w| {
        w.put(0, 1);
        w.put(2, 8);
        w.put(1, 12);
        w.put(42, 40);
        put_engine(w, EngineVersion::CURRENT);
        w.put(1, 8);
        w.put(500, 10);
        w.put(500, 10);
        w.put_bool(false);
        w.put(3, 7);
        w.put(100, 16);
        w.put(0b0000_0001, 8);
        w.put(1, 60);
    });
    assert!(Scorecard::decode(&bad).is_ok());
    assert_eq!(
        Scorecard::decode(&refused).unwrap_err(),
        ResultError::BadField {
            field: "revealed",
            value: 1
        }
    );
}

fn report(sound: f64, combined: f64, difficulty: u8) -> (ScoreReport, ScoringSpec) {
    let scoring = ScoringSpec::recreate(difficulty);
    let report = ScoreReport::new(
        &scoring,
        SoundScore {
            scorer: scoring.sound.clone(),
            percent: sound,
            distance: 0.0,
            floor: 0.0,
            scale: 1.0,
            jnd: 1.0,
            components: Vec::new(),
        },
        StructureScore {
            scorer: scoring.structure.clone(),
            percent: 50.0,
            params: Vec::new(),
            modules: Vec::new(),
        },
        Combined {
            scorer: scoring.combined.clone(),
            percent: combined,
            sound_weight: scoring.sound_weight,
        },
    );
    (report, scoring)
}

#[test]
fn a_scorecard_from_a_round_rounds_and_saturates() {
    let challenge = ChallengeCode::short(SpecPreset::by_id(3).unwrap().spec(1, 9)).unwrap();
    let (r, scoring) = report(93.449, 90.05, 50);
    let used = AssistsUsed {
        hints: 40,
        live_meter: true,
        ..AssistsUsed::default()
    };
    let c = Commitment::from_bits(7).unwrap();
    let card = Scorecard::new(challenge, &r, &scoring, 300, 161_049, &used, c).unwrap();
    let f = card.fields();
    assert_eq!((f.sound_tenths, f.combined_tenths), (934, 901));
    assert_eq!(f.passed, r.passed);
    assert_eq!(f.submits, SUBMITS_MAX);
    assert_eq!(f.time_ds, 1610);
    assert_eq!(f.assists.hints, HINTS_MAX);
    assert!(f.assists.live_meter);
    assert_eq!(f.scorers, ScorerSet::V1);
    assert_eq!(f.engine, EngineVersion::CURRENT);
    let long = Scorecard::new(challenge, &r, &scoring, 1, u64::MAX, &used, c).unwrap();
    assert_eq!(long.fields().time_ds, TIME_DS_MAX);
    // No submit, scorers outside the frozen table, a report another scoring made: refused.
    assert!(Scorecard::new(challenge, &r, &scoring, 0, 1, &used, c).is_err());
    let mut other = scoring.clone();
    other.pass_mark = DEFAULT_PASS_MARK - 1;
    assert_eq!(
        Scorecard::new(challenge, &r, &other, 1, 1, &used, c)
            .unwrap_err()
            .code(),
        "unencodable"
    );
    let mut stub = scoring.clone();
    stub.sound = ScorerId::SOUND_STUB;
    assert!(Scorecard::new(challenge, &r, &stub, 1, 1, &used, c).is_err());
}

#[test]
fn the_share_lines_follow_duels_md() {
    let card = sample_card();
    let [first, second] = card.share_lines();
    assert_eq!(first, "enth duel · Level 2 · 1080-1000-0001-AB");
    assert_eq!(second, "93.4 % in 2:41 · 4 submits · 0 hints");
    let mut f = card.fields().clone();
    f.passed = false;
    f.submits = 1;
    f.time_ds = 36_000;
    f.assists.hints = 1;
    f.assists.closest = true;
    let [_, second] = Scorecard::from_fields(f).unwrap().share_lines();
    assert_eq!(
        second,
        "93.4 %, not passed, 1:00:00 · 1 submit · 1 hint · how close"
    );
}

#[test]
fn scorer_set_1_is_this_builds_recreate_scoring() {
    assert_eq!(ScorerSet::current(), ScorerSet::V1);
    assert_eq!(ScorerSet::V1.ids(), RECREATE_SCORERS);
    for d in 0..=100 {
        assert_eq!(
            ScorerSet::V1.scoring(d),
            ScoringSpec::recreate(d),
            "difficulty {d}"
        );
        assert_eq!(
            ScorerSet::of(&ScoringSpec::recreate(d), d),
            Some(ScorerSet::V1)
        );
    }
    assert_eq!(ScorerSet::by_index(0), None);
    assert_eq!(ScorerSet::by_index(2), None);
    assert_eq!(
        ScorerSet::V1.to_string(),
        "sound/1 + structure/1 + combined/1 + time/1"
    );
}

#[test]
fn ear_result_refusals() {
    let ear = |count: u8| EarChallengeRef {
        game: 1,
        generator: 1,
        level: 2,
        seed: 3,
        count,
    };
    assert!(EarResult::new(ear(2), vec![None], 0, 0, false).is_err());
    assert!(EarResult::new(ear(1), vec![Some(EarAnswer::Choice(16))], 0, 0, false).is_err());
    assert!(EarResult::new(ear(1), vec![Some(EarAnswer::Value(f32::NAN))], 0, 0, false).is_err());
    assert!(EarResult::new(ear(1), vec![Some(EarAnswer::Taps(vec![5, 3]))], 0, 0, false).is_err());
    assert!(
        EarResult::new(
            ear(1),
            vec![Some(EarAnswer::Taps(vec![20_000]))],
            0,
            0,
            false
        )
        .is_err()
    );
    let mut seed = ear(0);
    seed.seed = 1 << 40;
    assert!(EarResult::new(seed, vec![], 0, 0, false).is_err());
    let fields = |w: &mut Bits, reserved: u64| {
        w.put(1, 8);
        w.put(1, 8);
        w.put(2, 8);
        w.put(3, 40);
        w.put(1, 8);
        w.put(2, 2);
        w.put(1, 1);
        w.put(u64::from(f32::INFINITY.to_bits()), 32);
        w.put(0, 16);
        w.put(0, 5);
        w.put(reserved, 3);
    };
    let inf = craft(KIND_EAR_RESULT, 1, |w| fields(w, 0));
    assert_eq!(EarResult::decode(&inf).unwrap_err().code(), "bad_field");
    let reserved = craft(KIND_EAR_RESULT, 1, |w| {
        w.put(1, 8);
        w.put(1, 8);
        w.put(2, 8);
        w.put(3, 40);
        w.put(0, 8);
        w.put(0, 16);
        w.put(0, 5);
        w.put(0b101, 3);
    });
    assert_eq!(
        EarResult::decode(&reserved).unwrap_err(),
        ResultError::BadField {
            field: "reserved",
            value: 5
        }
    );
    assert_eq!(
        EarResult::decode(&craft(KIND_EAR_RESULT, 2, |_| {}))
            .unwrap_err()
            .code(),
        "code_too_new"
    );
}

/// Sixteen choice answers, as a Spot-the-change round: about 49 symbols, which an old build reads as "update enth".
#[test]
fn a_sixteen_item_ear_result_is_about_49_symbols() {
    let answers = (0..16).map(|i| Some(EarAnswer::Choice(i % 4))).collect();
    let ear = EarResult::new(
        EarChallengeRef {
            game: 1,
            generator: 1,
            level: 3,
            seed: (1 << 40) - 1,
            count: 16,
        },
        answers,
        95_000,
        0,
        false,
    )
    .unwrap();
    let n = symbols_of(&ear.encode()).len();
    assert_eq!(n, 49);
    assert_old_builds_say_update(&ear.encode());
}

#[test]
fn a_long_ear_result_reads_bad_length_on_old_builds() {
    let answers = (0..40)
        .map(|i| Some(EarAnswer::Value(i as f32 * 0.123_457)))
        .collect();
    let ear = EarResult::new(
        EarChallengeRef {
            game: 2,
            generator: 1,
            level: 1,
            seed: 1,
            count: 40,
        },
        answers,
        1,
        0,
        false,
    )
    .unwrap();
    assert!(symbols_of(&ear.encode()).len() > 64);
    assert_old_builds_say_update(&ear.encode());
}
