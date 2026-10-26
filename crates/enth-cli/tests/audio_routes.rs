//! A patch with a `wt` and an audio route in the CLI (S2a, lane SKS, freeze F2m): `params wt` marks the audio inputs,
//! `apply`, `diff` and `step` say what an audio route's depth does (a share of the source's output, not a place in
//! the input's range), `render`, `describe` and `check` work on the patch and hear the route, `explain` names the route
//! and no longer says "routes cannot", and a refused route says why. `wt` is hidden from `enth modules` until lane
//! INT-S2a, so these patches add it by key the way a file does.
#![allow(
    clippy::disallowed_methods,
    reason = "tests read and write scratch files"
)]

mod common;

use common::{Scratch, json, round_trip, run};
use enth_cli::schema::{ApplyOut, DescribeOut, ErrorOut, ExplainOut, ParamsOut};
use enth_patch::catalogue::ModuleKind;

/// The smallest FM patch: a sine `osc1` an octave up at level 0 (an operator, heard only through its route) into
/// `wt1`'s `pm` at 60 %, `pm_index` 3, under `init`'s amplitude envelope. Written to `fm.patch.json` in `s`.
fn fm_patch(s: &Scratch) -> String {
    let p = s.new_patch("init", "fm.patch.json");
    let ops = r#"[{"add":"wt","id":"wt1","params":{"pm_index":3}},
        {"set":"osc1.wave","to":"sine"},{"set":"osc1.semi","to":"+12st"},{"set":"osc1.level","to":"0%"},
        {"route":"osc1 -> wt1.pm","depth":"60%"}]"#;
    let o = run(&["apply", &p, "--ops", ops]);
    assert_eq!(o.code, 0, "{}", o.stdout);
    p
}

fn means(p: &str, ops: &str) -> Vec<String> {
    let o = run(&["apply", p, "--dry-run", "--ops", ops]);
    assert_eq!(o.code, 0, "{}", o.stdout);
    let a: ApplyOut = round_trip(&o);
    a.applied
        .iter()
        .filter(|c| c.op == "route")
        .map(|c| c.means.clone().expect("a route says what it means"))
        .collect()
}

fn describe(p: &str) -> DescribeOut {
    let o = run(&["describe", p]);
    assert_eq!(o.code, 0, "{}", o.stdout);
    round_trip(&o)
}

#[test]
fn params_marks_the_audio_inputs_and_only_them() {
    let wt: ParamsOut = round_trip(&run(&["params", "wt"]));
    let audio: Vec<&str> = wt
        .params
        .iter()
        .filter(|p| p.audio)
        .map(|p| p.key.as_str())
        .collect();
    assert_eq!(audio, ["pm", "am"]);
    for p in wt.params.iter().filter(|p| p.audio) {
        assert!(
            !p.modulatable,
            "{}: a control source cannot reach it",
            p.key
        );
    }
    // No other listed kind has one, and the flag is absent (not `false`) from the JSON of a parameter without it.
    for kind in ModuleKind::listed() {
        let out: ParamsOut = round_trip(&run(&["params", kind.key()]));
        assert!(out.params.iter().all(|p| !p.audio), "{}", kind.key());
    }
    let v = json(&run(&["params", "wt"]));
    let level = v["params"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["key"] == "level")
        .unwrap();
    assert!(level.get("audio").is_none(), "{level}");
}

#[test]
fn an_audio_routes_depth_is_said_as_a_share_of_the_source_not_a_range() {
    let s = Scratch::new("audio_means");
    let p = fm_patch(&s);
    let m = means(
        &p,
        r#"[{"route":"osc1 -> wt1.pm","depth":"30%"},{"route":"osc1 -> wt1.am","depth":"-25%"}]"#,
    );
    // pm: pm_index 3 x pm 100 % x 30 % = 0.9 rad at osc1's full swing.
    assert!(
        m[0].starts_with(
            "osc1 phase-modulates wt1: the phase moves by up to +-0.9 rad as osc1 swings -1 to 1"
        ),
        "{}",
        m[0]
    );
    assert!(
        m[0].contains("pm_index 3 x pm 100% x 30% of osc1's output")
            && m[0].contains("a share of osc1's output, not of wt1.pm's range"),
        "{}",
        m[0]
    );
    // am: the gain swings 100 % -/+ 25 % around 1.
    assert!(
        m[1].starts_with("osc1 amplitude-modulates wt1: its gain swings 0.75..1.25 around 1"),
        "{}",
        m[1]
    );
    assert!(m[1].contains("am 100% - 25% of osc1's output"), "{}", m[1]);
    // None of the modulation text: no "goes ... ->" at the source's full swing, no "linear range".
    for text in &m {
        assert!(
            !text.contains("linear range") && !text.contains(" goes "),
            "{text}"
        );
    }

    // am at 0 % with a route is ring modulation; pm with pm_index 0 says nothing is heard.
    let m = means(
        &p,
        r#"[{"set":"wt1.am","to":"0%"},{"route":"osc1 -> wt1.am","depth":"80%"},
            {"set":"wt1.pm_index","to":"0"},{"route":"osc1 -> wt1.pm","depth":"50%"}]"#,
    );
    assert!(
        m[0].starts_with(
            "osc1 ring-modulates wt1: its gain is 80% of osc1's output itself, -0.8 to +0.8"
        ),
        "{}",
        m[0]
    );
    assert!(
        m[1].contains("+-0 rad") && m[1].contains("nothing is heard"),
        "{}",
        m[1]
    );

    // A modulation route right beside it keeps its own text.
    let m = means(
        &p,
        r#"[{"add":"lfo","id":"lfo1"},{"route":"lfo1 -> wt1.position","depth":"20%"}]"#,
    );
    assert!(m[0].starts_with("wt1.position swings "), "{}", m[0]);

    // `diff` reads the same way.
    let b = s.path("fm2.patch.json");
    let o = run(&[
        "apply",
        &p,
        "--out",
        &b,
        "--ops",
        r#"[{"route":"osc1 -> wt1.pm","depth":"30%"}]"#,
    ]);
    assert_eq!(o.code, 0, "{}", o.stdout);
    let o = run(&["diff", &p, &b]);
    assert_eq!(o.code, 0, "{}", o.stdout);
    let v = json(&o);
    let text = v["changes"][0]["means"].as_str().unwrap();
    assert!(text.starts_with("osc1 phase-modulates wt1"), "{text}");
}

#[test]
fn a_wt_patch_with_an_audio_route_renders_describes_and_checks() {
    let s = Scratch::new("audio_render");
    let fm = fm_patch(&s);
    let plain = s.path("plain.patch.json");
    let o = run(&[
        "apply",
        &fm,
        "--out",
        &plain,
        "--ops",
        r#"[{"unroute":"osc1 -> wt1.pm"}]"#,
    ]);
    assert_eq!(o.code, 0, "{}", o.stdout);

    // Render: a WAV, the same bytes twice, and different bytes without the route.
    let (a, b, c) = (s.path("a.wav"), s.path("b.wav"), s.path("c.wav"));
    for out in [&a, &b] {
        let o = run(&["render", &fm, "--out", out]);
        assert_eq!(o.code, 0, "{}", o.stdout);
    }
    let o = run(&["render", &plain, "--out", &c]);
    assert_eq!(o.code, 0, "{}", o.stdout);
    let bytes = |p: &str| std::fs::read(p).unwrap();
    assert_eq!(
        bytes(&a),
        bytes(&b),
        "the same patch renders the same samples"
    );
    assert_ne!(bytes(&a), bytes(&c), "the route is heard");

    // Describe: sounding, on C3 either way, and the route brightens the plain sine (FM).
    let fm_d = describe(&fm);
    let plain_d = describe(&plain);
    assert!(!fm_d.silent && !plain_d.silent);
    assert_eq!(fm_d.pitch.as_ref().unwrap().note, "C3");
    assert!(fm_d.loudness.lufs_i.unwrap() > -30.0, "{:?}", fm_d.loudness);
    let (bright, dull) = (
        fm_d.spectrum.unwrap().centroid_hz,
        plain_d.spectrum.unwrap().centroid_hz,
    );
    assert!(dull < 300.0, "a sine's centroid is near its pitch: {dull}");
    assert!(
        bright > 2.0 * dull,
        "FM at 1.8 rad brightens the sine: {bright} vs {dull}"
    );

    // Check judges the two differently; `step` does the apply, render, describe and check in one call.
    let expect = "spectrum.centroid_hz > 250";
    let o = run(&["check", &fm, "--expect", expect]);
    assert_eq!(o.code, 0, "{}", o.stdout);
    let o = run(&["check", &plain, "--expect", expect]);
    assert_eq!(o.code, 1, "{}", o.stdout);
    let o = run(&[
        "step",
        &plain,
        "--ops",
        r#"[{"route":"osc1 -> wt1.pm","depth":"60%"}]"#,
        "--expect",
        expect,
    ]);
    assert_eq!(o.code, 0, "{}", o.stdout);
    let v = json(&o);
    let text = v["applied"][0]["means"].as_str().unwrap();
    assert!(text.starts_with("osc1 phase-modulates wt1"), "{text}");
}

#[test]
fn explain_names_the_audio_route_and_no_longer_says_routes_cannot() {
    let s = Scratch::new("audio_explain");
    let p = fm_patch(&s);
    let ex = |what: &str| -> ExplainOut {
        let o = run(&["explain", what, "--patch", &p]);
        assert_eq!(o.code, 0, "{what}: {}", o.stdout);
        round_trip(&o)
    };

    let pm = ex("wt1.pm");
    assert!(pm.param.as_ref().unwrap().audio);
    assert!(!pm.text.contains("routes cannot"), "{}", pm.text);
    assert!(
        pm.text.contains("Audio routes from generators land here"),
        "{}",
        pm.text
    );
    assert!(
        pm.text.contains("Audio routes into it in this patch:")
            && pm
                .text
                .contains("osc1 -> wt1.pm at 60%: osc1 phase-modulates wt1"),
        "{}",
        pm.text
    );

    // The module on either end lists the route; a module with none, and a parameter that is not an audio input,
    // read as they did.
    for module in ["wt1", "osc1"] {
        let e = ex(module);
        assert!(
            e.text
                .contains("Audio routes of this module in this patch:")
                && e.text.contains("osc1 -> wt1.pm at 60%"),
            "{module}: {}",
            e.text
        );
    }
    assert!(!ex("env1").text.contains("Audio routes"));
    let level = ex("wt1.level");
    assert!(!level.text.contains("Audio routes"), "{}", level.text);
    let facts = level.param.unwrap();
    assert!(facts.modulatable && !facts.audio);

    // An audio input nothing feeds says so and how to feed it; without a patch there is nothing to say about one.
    let bare = s.path("bare.patch.json");
    let o = run(&[
        "apply",
        &p,
        "--out",
        &bare,
        "--ops",
        r#"[{"unroute":"osc1 -> wt1.pm"}]"#,
    ]);
    assert_eq!(o.code, 0, "{}", o.stdout);
    let o = run(&["explain", "wt1.am", "--patch", &bare]);
    let e: ExplainOut = round_trip(&o);
    assert!(
        e.text
            .contains("No audio route lands in wt1.am in this patch"),
        "{}",
        e.text
    );
    let e: ExplainOut = round_trip(&run(&["explain", "wt.pm"]));
    assert!(!e.text.contains("in this patch"), "{}", e.text);
    assert!(e.param.unwrap().audio);
}

#[test]
fn a_refused_audio_route_says_why_and_how_to_fix_it() {
    let s = Scratch::new("audio_refused");
    let p = fm_patch(&s);
    let refuse = |ops: &str, code: &str| -> ErrorOut {
        let o = run(&["apply", &p, "--dry-run", "--ops", ops]);
        assert_eq!(o.code, 3, "{ops}: {}", o.stdout);
        let e: ErrorOut = round_trip(&o);
        assert_eq!(e.error, code, "{ops}: {}", e.message);
        e
    };
    let e = refuse(
        r#"[{"route":"env1 -> wt1.pm","depth":"30%"}]"#,
        "not_audio_source",
    );
    assert!(e.hint.contains("generator"), "{}", e.hint);
    let e = refuse(
        r#"[{"route":"osc1 -> wt1.pm_index","depth":"30%"}]"#,
        "not_audio_dest",
    );
    assert!(e.hint.contains("`enth params <kind>`"), "{}", e.hint);
    refuse(
        r#"[{"route":"wt1 -> wt1.pm","depth":"30%"}]"#,
        "audio_cycle",
    );
    refuse(
        r#"[{"add":"wt","id":"wt2"},{"route":"wt1 -> wt2.pm","depth":"30%"},{"route":"wt2 -> wt1.pm","depth":"30%"}]"#,
        "audio_cycle",
    );
}
