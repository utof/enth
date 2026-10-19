//! `check` hints point the right way, and what they promise is true: a hint with `reaches: true` was measured by
//! rendering that single change, so applying it must make the expectation pass.

use serde_json::Value;
use spike_agent::commands::{check_patch, describe_patch};
use spike_agent::descriptors::Want;
use spike_agent::patch::Patch;
use spike_agent::units::{Unit, parse_value};

fn patch(perf: &str) -> Patch {
    let mut p = Patch::default();
    for item in perf.split(',') {
        let (k, v) = item.split_once('=').unwrap();
        p.set(&format!("perf.{k}"), v).unwrap();
    }
    p
}

/// A short clip keeps debug-build tests quick; the code path is the same.
fn short() -> Patch {
    patch("note=A3,dur=1s,hold=0.7s")
}

fn result(p: &Patch, expect: &str) -> Value {
    let (v, _) = check_patch(p, expect).unwrap();
    v["results"][0].clone()
}

fn hint(p: &Patch, expect: &str) -> Value {
    let r = result(p, expect);
    assert_eq!(
        r["pass"],
        Value::Bool(false),
        "{expect} should fail on this patch: {r}"
    );
    r["hint"].clone()
}

fn num(text: &Value, unit: Unit) -> f64 {
    parse_value(text.as_str().unwrap(), unit).unwrap()
}

#[test]
fn brightness_hints_move_the_cutoff_the_right_way() {
    let p = short();
    let down = hint(&p, "brightness<500Hz");
    assert_eq!(down["param"], "filter.cutoff");
    assert_eq!(down["dir"], "lower");
    assert!(
        num(&down["try"], Unit::Hz) < num(&down["now"], Unit::Hz),
        "{down}"
    );
    assert_eq!(down["measured"], Value::Bool(true));
    let up = hint(&p, "brightness>3kHz");
    assert_eq!(up["dir"], "raise", "{up}");
    assert!(
        num(&up["try"], Unit::Hz) > num(&up["now"], Unit::Hz) || up["param"] != "filter.cutoff",
        "{up}"
    );
}

#[test]
fn envelope_hints_pick_the_envelope_parameters() {
    let p = short();
    let h = hint(&p, "attack>150ms");
    assert_eq!(
        (h["param"].as_str(), h["dir"].as_str()),
        (Some("amp.attack"), Some("raise")),
        "{h}"
    );
    assert!(num(&h["try"], Unit::Ms) > 150.0);
    let h = hint(&p, "sustain<-30dB");
    assert_eq!(
        (h["param"].as_str(), h["dir"].as_str()),
        (Some("amp.sustain"), Some("lower")),
        "{h}"
    );
    assert!(num(&h["try"], Unit::Db) < -30.0);
    let h = hint(&p, "attack<2ms");
    assert_eq!(
        (h["param"].as_str(), h["dir"].as_str()),
        (Some("amp.attack"), Some("lower")),
        "{h}"
    );
}

#[test]
fn beating_hint_adds_detune() {
    let p = patch("note=A3,dur=1.5s,hold=1.2s");
    let r = result(&p, "beating>2Hz");
    assert!(r["got"].is_null(), "an undetuned note does not beat: {r}");
    assert!(
        r["why_missing"].as_str().unwrap().contains("detuned"),
        "{r}"
    );
    let h = &r["hint"];
    assert_eq!(
        (h["param"].as_str(), h["dir"].as_str()),
        (Some("osc.detune"), Some("raise")),
        "{h}"
    );
    assert!(num(&h["try"], Unit::Cents) > 5.0, "{h}");
}

#[test]
fn pitch_hint_points_at_the_played_note() {
    let h = hint(&short(), "pitch=C3±20ct");
    assert_eq!(h["param"], "perf.note");
    assert_eq!(h["try"], "C3");
    assert_eq!(h["measured"], Value::Bool(false));
}

#[test]
fn a_clip_that_ends_before_the_tail_says_so() {
    // 0.3 s after note-off is enough for the default 200 ms release, not for a 600 ms one
    let (p, _) = short()
        .apply_sets(&["amp.release=600ms".to_string()])
        .unwrap();
    let r = result(&p, "release<2000ms");
    assert_eq!(r["lower_bound"], Value::Bool(true), "{r}");
    assert_eq!(
        r["pass"],
        Value::Bool(false),
        "a lower bound cannot prove a maximum: {r}"
    );
    assert_eq!(r["hint"]["param"], "perf.dur", "{r}");
    // "longer than" can be decided from the lower bound alone
    let r = result(&p, "release>250ms");
    assert_eq!(r["pass"], Value::Bool(true), "{r}");
    // and the hint for a tail the default clip cannot hold names the clip length, not just the release
    let r = result(&short(), "release>500ms");
    assert_eq!(r["pass"], Value::Bool(false));
    assert_eq!(r["hint"]["param"], "perf.dur", "{r}");
    assert_eq!(r["hint"]["also"]["param"], "amp.release", "{r}");
}

#[test]
fn what_cannot_be_raised_is_said_plainly() {
    let h = hint(&short(), "noisiness>20%");
    assert_eq!(h["measured"], Value::Bool(false));
    assert!(
        h["text"].as_str().unwrap().contains("no noise source"),
        "{h}"
    );
}

#[test]
fn passing_expectations_get_no_hint() {
    let p = short();
    let (v, pass) = check_patch(&p, "attack<10ms, sustain>-8dB").unwrap();
    assert!(pass, "{v}");
    for r in v["results"].as_array().unwrap() {
        assert!(r.get("hint").is_none(), "{r}");
    }
}

#[test]
fn a_hint_that_reaches_really_reaches() {
    let base = short();
    let cases = [
        "brightness<600Hz",
        "brightness>2.5kHz",
        "attack>80ms",
        "sustain<-25dB",
        "sustain>-1dB",
        "decay>150ms",
        "duration<500ms",
        "loudness<-24LUFS",
        "peak<-10dB",
    ];
    let mut reached = 0;
    for expect in cases {
        let r = result(&base, expect);
        if r["pass"] == Value::Bool(true) {
            continue;
        }
        let h = &r["hint"];
        if h["reaches"] != Value::Bool(true) {
            continue;
        }
        let (param, to) = (h["param"].as_str().unwrap(), h["try"].as_str().unwrap());
        let (changed, _) = base.apply_sets(&[format!("{param}={to}")]).unwrap();
        let again = result(&changed, expect);
        assert_eq!(
            again["pass"],
            Value::Bool(true),
            "{expect}: applying {param}={to} did not pass: {again}"
        );
        reached += 1;
    }
    assert!(
        reached >= 7,
        "most single-parameter hints should reach their expectation: {reached}"
    );
}

#[test]
fn descriptors_move_with_the_parameters_the_hints_blame() {
    let get = |sets: &[&str], f: fn(&spike_agent::descriptors::Descriptors) -> Option<f64>| {
        let (p, _) = patch("note=A3,dur=1.5s,hold=1.2s")
            .apply_sets(&sets.iter().map(|s| (*s).to_string()).collect::<Vec<_>>())
            .unwrap();
        f(&describe_patch(&p, Want::ALL).unwrap()).unwrap_or(f64::NAN)
    };
    let c = |d: &spike_agent::descriptors::Descriptors| d.centroid_hz;
    assert!(get(&["filter.cutoff=4kHz"], c) > get(&["filter.cutoff=1kHz"], c));
    let a = |d: &spike_agent::descriptors::Descriptors| d.attack_ms;
    assert!(get(&["amp.attack=300ms"], a) > 5.0 * get(&["amp.attack=10ms"], a));
    let s = |d: &spike_agent::descriptors::Descriptors| d.sustain_db;
    assert!(get(&["amp.sustain=-3dB"], s) > get(&["amp.sustain=-20dB"], s) + 10.0);
    let r = |d: &spike_agent::descriptors::Descriptors| d.release_ms;
    assert!(
        get(&["amp.release=150ms"], r) > 100.0
            && get(&["amp.release=150ms"], r) < get(&["amp.release=250ms"], r)
    );
    let b = |d: &spike_agent::descriptors::Descriptors| d.beating_hz;
    let fast = get(&["osc.detune=50ct"], b);
    let slow = get(&["osc.detune=30ct"], b);
    assert!(fast > slow, "{fast} {slow}");
    // two oscillators 30 ct apart at 220 Hz beat at 220 * (2^(30/1200) - 1) = 3.85 Hz, 50 ct apart at 6.45 Hz
    assert!((slow - 3.85).abs() < 0.5, "{slow}");
    assert!((fast - 6.45).abs() < 0.6, "{fast}");
}
