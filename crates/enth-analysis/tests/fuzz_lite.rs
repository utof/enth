//! Fuzz-lite (proptest, fixed seed in the gate): the expectation parser and `describe` never panic, errors are
//! typed, and every parsed item round-trips through its canonical text.

use enth_analysis::descriptors::{AnalysisError, Describer, Feature, NoteTiming};
use enth_analysis::expect::{Test, parse_item, parse_list};
use enth_analysis::test_signals::SR;
use proptest::prelude::*;

fn grammar_like() -> impl Strategy<Value = String> {
    prop_oneof![
        any::<String>(),
        "[a-zA-Z_.]{0,14} ?(<|<=|>|>=|=|==|~|!=| in )? ?\\[?[-+0-9.eE]{0,8}(kHz|Hz|ms|s|dB|LUFS|ct|st|%|true|null|C3|A#4)? ?,? ?[-+0-9.]{0,5}\\]?( ?(±|\\+-) ?[0-9.]{0,4}(ct|%|Hz)?)?",
        "([a-z_.]{1,12}[<>=~]{1,2}[-0-9.]{1,6}[a-zA-Z%]{0,4}[,;] ?){1,5}",
    ]
}

proptest! {
    #[test]
    fn the_parser_never_panics(s in grammar_like()) {
        match parse_list(&s) {
            Ok(items) => prop_assert!(!items.is_empty()),
            Err(e) => {
                prop_assert!(e.code == "bad_expect" || e.code == "unknown_feature", "{e:?}");
                prop_assert!(!e.hint.is_empty() && !e.message.is_empty());
            }
        }
    }

    #[test]
    fn items_round_trip_through_their_canonical_text(
        fi in 0usize..Feature::ALL.len(),
        op in 0usize..7,
        v in -1000.0f64..20_000.0,
        w in 0.1f64..500.0,
    ) {
        let f = Feature::ALL[fi];
        let unit = f.unit().label();
        let text = match (f, op) {
            (Feature::Clipping, _) => format!("{} = {}", f.path(), if v > 0.0 { "true" } else { "false" }),
            (Feature::Pitch, 4) => format!("pitch = A3 ± {:.1}ct", w.min(100.0)),
            (Feature::Pitch, _) => format!("{} > {:.3}Hz", f.path(), v.abs() + 20.0),
            (_, 0) => format!("{} < {v:.4}{unit}", f.path()),
            (_, 1) => format!("{} >= {v:.4}{unit}", f.path()),
            (_, 2) => format!("{} in [{v:.3}, {:.3}]", f.path(), v + w),
            (_, 3) => format!("{} == null", f.aliases()[0]),
            (_, 4) => format!("{} = {v:.3} ± {w:.3}", f.aliases()[0]),
            (_, 5) => format!("{}~{v:.2}", f.aliases()[0]),
            _ => format!("{} != null", f.path()),
        };
        let a = parse_item(&text).map_err(|e| TestCaseError::fail(format!("{text}: {e:?}")))?;
        let b = parse_item(&a.canonical).map_err(|e| TestCaseError::fail(format!("{}: {e:?}", a.canonical)))?;
        prop_assert_eq!(a.feature, b.feature);
        prop_assert_eq!(a.canonical.clone(), b.canonical.clone());
        let close = |x: f64, y: f64| x == y || (x - y).abs() <= 1e-8 * x.abs().max(y.abs()).max(1.0);
        match (a.test, b.test) {
            (Test::Within(p), Test::Within(q)) => {
                prop_assert!(close(p.lo, q.lo) && close(p.hi, q.hi), "{} -> {}: {p:?} vs {q:?}", text, a.canonical);
                prop_assert_eq!((p.lo_inclusive, p.hi_inclusive), (q.lo_inclusive, q.hi_inclusive));
            }
            (x, y) => prop_assert_eq!(x, y),
        }
        prop_assert_eq!(a.reference_hz.map(f64::to_bits), b.reference_hz.map(f64::to_bits));
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    #[test]
    fn describe_never_panics(
        x in prop::collection::vec(-4.0f32..4.0, 0..6000),
        on in 0.0f64..0.2,
        hold in prop::option::of(0.0f64..0.3),
        scale in prop_oneof![Just(1.0f32), Just(1e-6f32), Just(0.0f32)],
    ) {
        let x: Vec<f32> = x.iter().map(|v| v * scale).collect();
        let timing = NoteTiming { on_s: on, off_s: hold.map(|h| on + h) };
        match Describer::new(SR).describe(&x, timing) {
            Ok(d) => {
                prop_assert!(d.loudness.peak_dbfs.is_finite());
                for f in Feature::ALL {
                    prop_assert!(d.get(f).is_none_or(f64::is_finite), "{f:?} = {:?}", d.get(f));
                }
            }
            Err(e) => prop_assert!(matches!(e, AnalysisError::Empty | AnalysisError::TooShort), "{e:?}"),
        }
    }
}
