//! Property tests for units, display/parse round trips and scales. The gate runs them with the fixed
//! `PROPTEST_RNG_SEED` from `.cargo/config.toml` and the default 256 cases; failures are committed under
//! `proptest-regressions/`.

use enth_params::{ParamSpec, Scale, Unit};
use proptest::prelude::*;

const UNITS: [Unit; 10] = [
    Unit::Hz,
    Unit::Ms,
    Unit::Db,
    Unit::Cents,
    Unit::Semitones,
    Unit::Octaves,
    Unit::Percent,
    Unit::Ratio,
    Unit::Degrees,
    Unit::None,
];

fn finite_f32() -> impl Strategy<Value = f32> {
    prop_oneof![
        // Every finite f32, including subnormals and huge values.
        any::<u32>()
            .prop_map(f32::from_bits)
            .prop_filter("finite", |x| x.is_finite()),
        // Values people write.
        (-100_000i32..100_000).prop_map(|i| i as f32 / 100.0),
        (-20_000.0f32..20_000.0),
    ]
}

fn canonical(x: f32) -> f32 {
    if x == 0.0 { 0.0 } else { x }
}

/// A random but valid continuous range and scale.
fn scale_and_range() -> impl Strategy<Value = (Scale, f32, f32)> {
    let linear =
        (-1.0e6f32..1.0e6, 1.0e-3f32..1.0e6).prop_map(|(lo, w)| (Scale::Linear, lo, lo + w));
    let log = (1.0e-3f32..1.0e3, 1.001f32..1.0e5).prop_map(|(lo, r)| (Scale::Log, lo, lo * r));
    let power = (
        prop::sample::select(vec![0.5f32, 2.0, 3.0, 4.0]),
        -1.0e3f32..1.0e3,
        1.0e-2f32..1.0e5,
    )
        .prop_map(|(k, lo, w)| (Scale::Power(k), lo, lo + w));
    prop_oneof![linear, log, power].prop_filter("min < max after rounding", |(_, lo, hi)| lo < hi)
}

fn ulp(x: f32) -> f32 {
    let x = x.abs();
    f32::from_bits(x.to_bits() + 1) - x
}

proptest! {
    /// `parse(display(x)) == x`, bit for bit, for every unit and every finite value.
    #[test]
    fn exact_display_round_trips(unit in prop::sample::select(UNITS.to_vec()), x in finite_f32()) {
        let text = unit.display(x).to_string();
        let back = unit.parse(&text);
        prop_assert_eq!(back.map(f32::to_bits), Ok(canonical(x).to_bits()), "{} -> {:?}", x, text);
    }

    /// Rounded display is stable: re-parsing and re-displaying gives the same text.
    #[test]
    fn rounded_display_is_stable(unit in prop::sample::select(UNITS.to_vec()), x in finite_f32(), digits in 1u8..=7) {
        let text = unit.display_rounded(x, digits).to_string();
        let back = unit.parse(&text).unwrap_or_else(|e| panic!("{text}: {e}"));
        prop_assert_eq!(unit.display_rounded(back, digits).to_string(), text);
    }

    /// The parser never panics, and anything it accepts is finite and never -0.0.
    #[test]
    fn parser_never_panics(unit in prop::sample::select(UNITS.to_vec()), text in "\\PC{0,12}") {
        if let Ok(v) = unit.parse(&text) {
            prop_assert!(v.is_finite());
            prop_assert!(!(v == 0.0 && v.is_sign_negative()));
        }
    }

    /// Unit-shaped junk: numbers with random suffixes.
    #[test]
    fn parser_handles_suffix_soup(unit in prop::sample::select(UNITS.to_vec()), n in finite_f32(),
                                  suffix in "[ a-zA-Z%:°#0-9.+-]{0,6}") {
        let text = format!("{n}{suffix}");
        if let Ok(v) = unit.parse(&text) {
            prop_assert!(v.is_finite());
        }
    }

    /// `to_norm` stays in [0, 1] and is monotonic; `from_norm` is monotonic and stays in [min, max].
    #[test]
    fn scales_are_monotonic((scale, lo, hi) in scale_and_range(), a in 0.0f32..=1.0, b in 0.0f32..=1.0) {
        let (n1, n2) = if a <= b { (a, b) } else { (b, a) };
        let (x1, x2) = (scale.from_norm(lo, hi, n1), scale.from_norm(lo, hi, n2));
        prop_assert!(x1 <= x2, "from_norm not monotonic: {} -> {}, {} -> {}", n1, x1, n2, x2);
        prop_assert!(lo <= x1 && x2 <= hi);
        let (m1, m2) = (scale.to_norm(lo, hi, x1), scale.to_norm(lo, hi, x2));
        prop_assert!(m1 <= m2, "to_norm not monotonic");
        prop_assert!((0.0..=1.0).contains(&m1) && (0.0..=1.0).contains(&m2));
    }

    /// Plain → normalised → plain loses no more than the rounding of the normalised value to f32 implies.
    #[test]
    fn scales_round_trip((scale, lo, hi) in scale_and_range(), t in 0.0f32..=1.0) {
        let x = scale.from_norm(lo, hi, t);
        let back = scale.from_norm(lo, hi, scale.to_norm(lo, hi, x));
        let eps = f32::EPSILON;
        let tol = match scale {
            Scale::Linear => 2.0 * eps * (hi - lo) + 2.0 * ulp(x),
            Scale::Log => 2.0 * eps * libm::logf(hi / lo) * x.abs() + 2.0 * ulp(x),
            Scale::Power(k) => 2.0 * eps * k.max(1.0 / k) * (x - lo).abs() + 2.0 * ulp(x) + 2.0 * ulp(lo),
        };
        prop_assert!((back - x).abs() <= tol, "{:?} [{}, {}]: {} -> {} (tol {})", scale, lo, hi, x, back, tol);
    }

    /// Discrete kinds round-trip exactly through the normalised domain.
    #[test]
    fn discrete_round_trips(min in -1000i32..1000, width in 1i32..1000, pick in 0.0f64..1.0) {
        let spec = ParamSpec::int("n", "N", Unit::None, min, min + width, min).basic("x");
        let v = (f64::from(min) + (pick * f64::from(width)).floor()) as f32;
        prop_assert_eq!(spec.from_norm(spec.to_norm(v)), v);
    }
}

#[test]
fn accepted_examples_parse() {
    for unit in UNITS {
        for example in unit.accepted() {
            if *example == "a plain number" {
                continue;
            }
            assert!(
                unit.parse(example).is_ok(),
                "{unit:?} should accept {example}"
            );
        }
    }
}
