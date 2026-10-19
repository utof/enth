//! The P1a G1 agent-eval findings (`docs/p1a-g1.md`, "Eval findings to act on"), each shown through the CLI on the
//! real engine, the way an agent meets it:
//!
//! 1. envelope knobs against their readings (`params` says how they read, and the engine holds to it);
//! 2. a route depth typed as `+12st` (the change says what the stored percentage does);
//! 3. `bass_sub`'s sub-octave (pitch reads the sub, and `blocked` names `osc2.semi`);
//! 4. `pad_2osc` clipped (the templates leave headroom);
//! 5. a bare `clipping = false` was always thin (now only when the re-renders disagree);
//! 6. a filter wobble was a tremolo or nothing (a filter LFO is a brightness sweep, an amplitude LFO is not);
//! 7. a tone under noise had no pitch (`tonal_pitch`).
#![allow(
    clippy::disallowed_methods,
    reason = "tests read and write scratch files"
)]

mod common;

use common::{Scratch, json, round_trip, run};
use enth_cli::schema::{ApplyOut, CheckOut, DescribeOut, ParamsOut};

fn describe(p: &str, perf: &str) -> DescribeOut {
    let o = run(&["describe", p, "--perf", perf]);
    assert_eq!(o.code, 0, "{}", o.stdout);
    round_trip(&o)
}

fn check(p: &str, perf: &str, list: &str) -> (i32, CheckOut) {
    let o = run(&["check", p, "--perf", perf, "--expect", list]);
    (o.code, round_trip(&o))
}

#[test]
fn finding_1_params_say_how_envelope_knobs_read() {
    let o = run(&["params", "adsr"]);
    let out: ParamsOut = round_trip(&o);
    for key in ["attack", "decay", "sustain", "release"] {
        let p = out.params.iter().find(|p| p.key == key).unwrap();
        let r = p
            .reads_as
            .as_deref()
            .unwrap_or_else(|| panic!("{key} has no reads_as"));
        assert!(r.starts_with("envelope."), "{key}: {r}");
    }
    // ... and the engine holds to what it says (knobs.rs): a 300 ms attack reads about 0.72 x, a 1 s release about
    // 0.5 x, a 1 s decay to 0 % about 0.4 x and to 50 % about 0.22 x, and 50 % sustain about -6 dB.
    let s = Scratch::new("findings_knobs");
    let base = s.new_patch("init", "k.patch.json");
    let read = |set: &str, perf: &str| {
        let p = s.path("k2.patch.json");
        let a = run(&["apply", &base, "--set", set, "--out", &p]);
        assert_eq!(a.code, 0, "{}", a.stdout);
        describe(&p, perf).envelope.unwrap()
    };
    let e = read(
        "env1.attack=300ms,env1.sustain=100%",
        "note=A3,hold=1s,dur=1.5s",
    );
    assert!((e.attack_ms / 300.0 - 0.72).abs() < 0.06, "{e:?}");
    let e = read(
        "env1.attack=2ms,env1.sustain=100%,env1.release=1s",
        "note=A3,hold=0.5s,dur=2s",
    );
    assert!((e.release_ms.unwrap() / 1000.0 - 0.5).abs() < 0.03, "{e:?}");
    let e = read(
        "env1.attack=2ms,env1.decay=1s,env1.sustain=0%",
        "note=A3,hold=1.5s,dur=2s",
    );
    assert!((e.decay_ms.unwrap() / 1000.0 - 0.4).abs() < 0.05, "{e:?}");
    let e = read(
        "env1.attack=2ms,env1.decay=1s,env1.sustain=50%",
        "note=A3,hold=2s,dur=2.5s",
    );
    assert!((e.decay_ms.unwrap() / 1000.0 - 0.22).abs() < 0.04, "{e:?}");
    assert!((e.sustain_db.unwrap() + 6.02).abs() < 0.5, "{e:?}");
}

#[test]
fn finding_2_a_route_says_what_its_depth_means() {
    let s = Scratch::new("findings_route");
    let p = s.new_patch("init", "r.patch.json");
    let ops = r#"[{"add":"lfo","id":"lfo1","params":{"rate":"5Hz"}},{"route":"lfo1 -> osc1.semi","depth":"+12st"},
        {"add":"svf","params":{"cutoff":"1kHz"}},{"route":"env1 -> filter1.cutoff","depth":"+2oct"}]"#;
    let o = run(&["apply", &p, "--dry-run", "--ops", ops]);
    let a: ApplyOut = round_trip(&o);
    let means: Vec<&str> = a
        .applied
        .iter()
        .filter(|c| c.op == "route")
        .map(|c| c.means.as_deref().unwrap())
        .collect();
    assert_eq!(a.applied[1].to.as_deref(), Some("12.5%"));
    assert!(
        means[0].starts_with("osc1.semi swings -12st..+12st around 0st")
            && means[0].contains("bipolar")
            && means[0].contains("12.5% of osc1.semi's -48st..+48st linear range"),
        "{}",
        means[0]
    );
    assert!(
        means[1].starts_with("filter1.cutoff goes 1kHz -> 4kHz (+2 oct) at full env1"),
        "{}",
        means[1]
    );
    // Non-route changes carry no `means`.
    assert!(a.applied[0].means.is_none());
}

#[test]
fn finding_3_a_sub_octave_reads_as_the_sub_and_blocked_names_it() {
    let s = Scratch::new("findings_sub");
    let p = s.new_patch("bass_sub", "bass.patch.json");
    let perf = "note=C2,vel=100,hold=1.5s,dur=2s";
    let (code, c) = check(&p, perf, "pitch.note ~ C2");
    assert_eq!(code, 1);
    let r = &c.results[0];
    assert_eq!(r.note.as_deref(), Some("C1+0ct"), "{r:?}");
    let b = &c.blocked[0];
    assert!(
        b.why.contains("osc2 is transposed -12 st (osc2.semi)"),
        "{}",
        b.why
    );
    let hint = &r.hint.as_ref().expect("a structural hint").text;
    let start = hint.find("--ops '").unwrap() + "--ops '".len();
    let end = start + hint[start..].find("]'").unwrap();
    let ops = &hint[start..=end];
    let o = run(&[
        "step",
        &p,
        "--ops",
        ops,
        "--perf",
        perf,
        "--expect",
        "pitch.note ~ C2",
    ]);
    assert_eq!(o.code, 0, "{}", o.stdout);
}

#[test]
fn finding_4_and_5_the_templates_leave_headroom_and_booleans_are_not_thin() {
    let s = Scratch::new("findings_headroom");
    for (t, note) in [("pad_2osc", "C3"), ("pad_2osc", "A3"), ("bass_sub", "C3")] {
        let p = s.new_patch(t, &format!("{t}.patch.json"));
        let perf = format!("note={note},vel=100,hold=1.5s,dur=3s");
        let o = run(&[
            "check",
            &p,
            "--perf",
            &perf,
            "--no-fix",
            "--expect",
            "peak < -1dB, clipping = false",
        ]);
        assert_eq!(o.code, 0, "{t} {note}: {}", o.stdout);
        let c: CheckOut = round_trip(&o);
        assert!(!c.results[1].thin, "{t} {note}: {:?}", c.results[1]);
    }
}

#[test]
fn finding_6_a_filter_lfo_is_a_sweep_and_an_amplitude_lfo_is_not() {
    let s = Scratch::new("findings_sweep");
    let perf = "note=A2,vel=100,hold=3s,dur=3.5s";
    let filter = s.new_patch("init", "filter.patch.json");
    let ops = r#"[{"add":"svf","at":"voice_fx[0]","params":{"mode":"lp","cutoff":"900Hz","resonance":"30%"}},
        {"add":"lfo","id":"lfo1","params":{"rate":"2Hz"}},{"route":"lfo1 -> filter1.cutoff","depth":"+1oct"},
        {"set":"env1.sustain","to":"100%"}]"#;
    assert_eq!(run(&["apply", &filter, "--ops", ops]).code, 0);
    let amp = s.new_patch("init", "amp.patch.json");
    let ops = r#"[{"add":"svf","at":"voice_fx[0]","params":{"mode":"lp","cutoff":"900Hz","resonance":"30%"}},
        {"add":"lfo","id":"lfo1","params":{"rate":"2Hz"}},{"route":"lfo1 -> osc1.level","depth":"30%"},
        {"set":"env1.sustain","to":"100%"}]"#;
    assert_eq!(run(&["apply", &amp, "--ops", ops]).code, 0);

    let d = describe(&filter, perf);
    let sweep = d.modulation.filter_sweep.expect("a filter sweep");
    assert_eq!(sweep.kind, "lfo");
    assert!((sweep.hz.unwrap() / 2.0 - 1.0).abs() < 0.03, "{sweep:?}");
    assert!(sweep.depth_oct > 0.5, "{sweep:?}");
    let d = describe(&amp, perf);
    assert!(d.modulation.filter_sweep.is_none(), "{:?}", d.modulation);
    let trem = d.modulation.amp_lfo.expect("a tremolo");
    assert!((trem.hz / 2.0 - 1.0).abs() < 0.03, "{trem:?}");
    // The goal's own words: a filter-wobble item passes on one and not the other.
    let list = "filter_lfo in [1.5Hz, 3.5Hz]";
    assert_eq!(check(&filter, perf, list).0, 0);
    let (code, c) = check(&amp, perf, list);
    assert_eq!(code, 1);
    assert!(c.results[0].got.is_none(), "{:?}", c.results[0]);
}

#[test]
fn finding_7_a_tone_under_noise_has_a_tonal_pitch() {
    let s = Scratch::new("findings_tonal");
    let p = s.new_patch("init", "snare.patch.json");
    // The snare-crack witness: a triangle body under white noise, then a short envelope.
    let ops = r#"[{"set":"osc1.wave","to":"triangle"},{"set":"osc1.level","to":"35%"},
        {"add":"noise","id":"noise1","params":{"level":"100%"}},
        {"add":"svf","at":"voice_fx[0]","params":{"mode":"lp","cutoff":"4kHz","resonance":"10%"}},
        {"set":"env1.attack","to":"0ms"},{"set":"env1.decay","to":"300ms"},{"set":"env1.sustain","to":"0%"},
        {"set":"env1.release","to":"50ms"}]"#;
    assert_eq!(run(&["apply", &p, "--ops", ops]).code, 0);
    let perf = "note=A3,vel=100,hold=0.3s,dur=1s";
    let (code, c) = check(
        &p,
        perf,
        "noisiness in [0.4, 0.85], tonal_pitch ~ A3, pitch == null",
    );
    assert_eq!(code, 0, "{c:?}");
    assert_eq!(c.results[1].margin_unit.as_deref(), Some("ct"));
    assert!(
        c.results[1]
            .note
            .as_deref()
            .is_some_and(|n| n.starts_with("A3"))
    );
    let v = json(&run(&["describe", &p, "--perf", perf]));
    assert_eq!(v["tonal"]["note"], "A3", "{v}");
}
