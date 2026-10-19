//! One in-process snapshot per command, each output deserialized back into its type (the typed round trip that
//! keeps the published schemas honest). Audio-dependent numbers are masked (`common::mask`): the engine's modules
//! are still being replaced by other lanes, so only the shape, the performance and the pass/fail of deliberately
//! robust expectations are pinned.
#![allow(
    clippy::disallowed_methods,
    reason = "tests read and write scratch files"
)]

mod common;

use common::{Scratch, json, record, round_trip, run, run_stdin, run_tty};
use enth_cli::schema::{
    ApplyOut, BenchOut, CheckOut, DescribeOut, DiffOut, GetOut, GuideOut, ModulesOut, NewOut,
    ParamsOut, RenderOut, SchemaListOut, StepOut, TemplatesOut,
};
use insta::assert_snapshot;

#[test]
fn schema_lists_every_schema() {
    let s = Scratch::new("schema_list");
    let o = run(&["schema"]);
    assert_eq!(o.code, 0);
    let out: SchemaListOut = round_trip(&o);
    assert!(out.schemas.iter().any(|e| e.id == "enth.check/1"));
    s.settings()
        .bind(|| assert_snapshot!("schema_list", record(&s, &o, false)));
}

#[test]
fn modules_lists_the_catalogue() {
    let s = Scratch::new("modules");
    let o = run(&["modules"]);
    let out: ModulesOut = round_trip(&o);
    assert_eq!(out.modules.len(), 7);
    s.settings()
        .bind(|| assert_snapshot!("modules", record(&s, &o, false)));
    let t = run_tty(&["modules"]);
    assert_eq!(t.code, 0);
    s.settings()
        .bind(|| assert_snapshot!("modules_table", s.clean(&t.stdout)));
}

#[test]
fn params_by_kind_builtin_and_id() {
    let s = Scratch::new("params");
    let svf = run(&["params", "svf"]);
    let out: ParamsOut = round_trip(&svf);
    assert_eq!(out.kind, "svf");
    let by_id = run(&["params", "filter1"]);
    assert_eq!(by_id.stdout, svf.stdout, "an id resolves to its kind");
    let p = s.new_patch("pad_2osc", "pad.patch.json");
    let by_patch = run(&["params", "osc2", "--patch", &p]);
    let osc: ParamsOut = round_trip(&by_patch);
    assert_eq!(osc.kind, "osc");
    let voice = run(&["params", "voice"]);
    let _: ParamsOut = round_trip(&voice);
    s.settings().bind(|| {
        assert_snapshot!("params_svf", record(&s, &svf, false));
        assert_snapshot!("params_voice", record(&s, &voice, false));
        assert_snapshot!("params_svf_table", run_tty(&["params", "svf"]).stdout);
    });
}

#[test]
fn guide_main_and_topics() {
    let s = Scratch::new("guide");
    let o = run(&["guide"]);
    let g: GuideOut = round_trip(&o);
    assert!(g.bytes <= 6 * 1024, "{}", g.bytes);
    assert_eq!(g.bytes, g.text.len());
    for topic in ["expect", "ops", "descriptors"] {
        let t = run(&["guide", "--topic", topic]);
        let g: GuideOut = round_trip(&t);
        assert_eq!(g.topic, topic);
    }
    // On a terminal the guide is the markdown itself.
    let tty = run_tty(&["guide"]);
    assert_eq!(tty.stdout, g_text(&o));
    s.settings().bind(|| {
        assert_snapshot!("guide_main", tty.stdout);
        assert_snapshot!(
            "guide_descriptors",
            run_tty(&["guide", "--topic", "descriptors"]).stdout
        );
    });
}

fn g_text(o: &enth_cli::Output) -> String {
    json(o)["text"].as_str().unwrap().to_owned()
}

#[test]
fn new_writes_a_stamped_dense_patch() {
    let s = Scratch::new("new");
    let out = s.path("p.patch.json");
    let o = run(&["new", "--template", "pluck_1osc", "--out", &out]);
    assert_eq!(o.code, 0);
    let n: NewOut = round_trip(&o);
    assert_eq!(n.modules, ["osc1", "filter1", "env1"]);
    let text = std::fs::read_to_string(&out).unwrap();
    assert!(text.contains("\"engine\": \"0.1.0\""), "{text}");
    assert!(text.contains("\"keytrack\""), "written dense: {text}");
    let list = run(&["new", "--list"]);
    let t: TemplatesOut = round_trip(&list);
    assert_eq!(t.templates.len(), 4);
    s.settings().bind(|| {
        assert_snapshot!("new", record(&s, &o, false));
        assert_snapshot!("new_list", record(&s, &list, false));
    });
}

#[test]
fn apply_set_ops_dry_run_and_inverse() {
    let s = Scratch::new("apply");
    let p = s.new_patch("init", "p.patch.json");
    let original = std::fs::read_to_string(&p).unwrap();
    let dry = run(&["apply", &p, "--set", "osc1.fine=7ct", "--dry-run"]);
    let d: ApplyOut = round_trip(&dry);
    assert!(d.dry_run && d.wrote.is_none());
    assert_eq!(std::fs::read_to_string(&p).unwrap(), original);

    let ops = r#"[{"add": "svf", "params": {"cutoff": "1.2kHz"}}, {"set": "env1.release", "to": "1.5s"}]"#;
    let o = run(&[
        "apply",
        &p,
        "--ops",
        ops,
        "--set",
        "osc1.wave=pulse,osc1.pw=30%",
    ]);
    assert_eq!(o.code, 0, "{}", o.stdout);
    let a: ApplyOut = round_trip(&o);
    assert_eq!(a.applied.len(), 4);
    assert_eq!(a.wrote.as_deref(), Some(p.as_str()));
    s.settings()
        .bind(|| assert_snapshot!("apply", record(&s, &o, false)));

    // The inverse undoes it exactly.
    let inverse = serde_json::to_string(&a.inverse).unwrap();
    let back = run(&["apply", &p, "--ops", &inverse]);
    assert_eq!(back.code, 0, "{}", back.stdout);
    assert_eq!(std::fs::read_to_string(&p).unwrap(), original);

    // --out leaves the input alone; ops from stdin.
    let out = s.path("q.patch.json");
    let piped = run_stdin(&["apply", &p, "--ops", "-", "--out", &out], ops);
    assert_eq!(piped.code, 0, "{}", piped.stdout);
    assert_eq!(std::fs::read_to_string(&p).unwrap(), original);
    assert!(std::fs::read_to_string(&out).unwrap().contains("filter1"));
    s.settings().bind(|| {
        assert_snapshot!(
            "apply_table",
            s.clean(&run_tty(&["apply", &p, "--set", "osc1.fine=7ct", "--dry-run"]).stdout)
        );
    });
}

#[test]
fn get_summary_addresses_and_plain() {
    let s = Scratch::new("get");
    let p = s.new_patch("pad_2osc", "p.patch.json");
    let summary = run(&["get", &p]);
    let g: GetOut = round_trip(&summary);
    assert_eq!(
        g.values["osc2.fine"],
        enth_cli::schema::Reading::Text("+8ct".into())
    );
    let some = run(&[
        "get",
        &p,
        "filter1.cutoff",
        "osc1",
        "master",
        "routes",
        "meta",
    ]);
    let _: GetOut = round_trip(&some);
    let plain = run(&["get", &p, "filter1.cutoff", "osc1.wave", "--plain"]);
    let gp: GetOut = round_trip(&plain);
    assert_eq!(
        gp.values["filter1.cutoff"],
        enth_cli::schema::Reading::Num(1500.0)
    );
    s.settings().bind(|| {
        assert_snapshot!("get_summary", record(&s, &summary, false));
        assert_snapshot!("get_addresses", record(&s, &some, false));
        assert_snapshot!("get_plain", record(&s, &plain, false));
    });
}

#[test]
fn diff_lists_changes_and_replayable_ops() {
    let s = Scratch::new("diff");
    let a = s.new_patch("pluck_1osc", "a.patch.json");
    let b = s.new_patch("pad_2osc", "b.patch.json");
    let same = run(&["diff", &a, &a]);
    let d: DiffOut = round_trip(&same);
    assert!(d.same && d.changes.is_empty());
    let o = run(&["diff", &a, &b]);
    let d: DiffOut = round_trip(&o);
    assert!(!d.same);
    // Replaying the ops on a gives b.
    let ops = serde_json::to_string(d.ops.as_ref().expect("verified ops")).unwrap();
    let c = s.path("c.patch.json");
    let applied = run(&["apply", &a, "--ops", &ops, "--out", &c]);
    assert_eq!(applied.code, 0, "{}", applied.stdout);
    let again: DiffOut = round_trip(&run(&["diff", &c, &b]));
    assert!(again.same, "{again:?}");
    s.settings()
        .bind(|| assert_snapshot!("diff", record(&s, &o, false)));
    let described = run(&["diff", &a, &b, "--describe"]);
    let dd: DiffOut = round_trip(&described);
    assert!(dd.describe.is_some());
    s.settings()
        .bind(|| assert_snapshot!("diff_describe", record(&s, &described, true)));
}

#[test]
fn render_writes_a_float_wav_with_its_performance() {
    let s = Scratch::new("render");
    let p = s.new_patch("pluck_1osc", "p.patch.json");
    let o = run(&["render", &p, "--perf", "note=A3,vel=90,hold=1s,dur=2s"]);
    assert_eq!(o.code, 0, "{}", o.stdout);
    let r: RenderOut = round_trip(&o);
    assert_eq!(r.wav, s.path("p.wav"));
    assert_eq!(
        (r.format.as_str(), r.channels, r.sample_rate),
        ("f32", 2, 48_000)
    );
    let bytes = std::fs::read(&r.wav).unwrap();
    let d = enth_cli::wav::decode(&bytes).unwrap();
    let meta = d.meta.expect("the enth chunk");
    assert_eq!(meta.perf.dur_s, 2.0);
    assert_eq!(d.mono.len(), 96_000);
    // hound (an outside reader) skips the chunk and reads the samples.
    let reader = hound::WavReader::open(&r.wav).unwrap();
    assert_eq!(reader.spec().sample_format, hound::SampleFormat::Float);
    assert_eq!(reader.len(), 2 * 96_000);
    s.settings()
        .bind(|| assert_snapshot!("render", record(&s, &o, true)));
    let pcm = run(&["render", &p, "--bits", "16", "--out", &s.path("p16.wav")]);
    let r16: RenderOut = round_trip(&pcm);
    assert_eq!(r16.format, "pcm16");
}

#[test]
fn describe_a_patch_and_a_wav() {
    let s = Scratch::new("describe");
    let p = s.new_patch("pluck_1osc", "p.patch.json");
    let o = run(&["describe", &p]);
    assert_eq!(o.code, 0, "{}", o.stdout);
    let d: DescribeOut = round_trip(&o);
    assert_eq!(d.perf.source, "default");
    assert_eq!(d.pitch.as_ref().map(|p| p.note.as_str()), Some("C3"));
    s.settings()
        .bind(|| assert_snapshot!("describe_patch", record(&s, &o, true)));

    // A WAV that enth rendered carries its performance.
    let wav = s.path("p.wav");
    assert_eq!(run(&["render", &p, "--out", &wav]).code, 0);
    let w = run(&["describe", &wav]);
    let dw: DescribeOut = round_trip(&w);
    assert_eq!(dw.perf.source, "wav");
    assert_eq!(dw.pitch, d.pitch, "the WAV reads like the patch");
    // --perf wins over the chunk.
    let f = run(&["describe", &wav, "--perf", "note=C3,hold=1s,dur=3s"]);
    let df: DescribeOut = round_trip(&f);
    assert_eq!(df.perf.source, "flag");

    // A WAV from elsewhere (no chunk): the note-off is inferred from the audio, or the envelope stages that need
    // it come back null with a reason.
    let held = s.new_patch("init", "held.patch.json");
    let src = s.path("held.wav");
    assert_eq!(run(&["render", &held, "--out", &src]).code, 0);
    let plain = s.path("plain.wav");
    let mut reader = hound::WavReader::open(&src).unwrap();
    let mut writer = hound::WavWriter::create(&plain, reader.spec()).unwrap();
    for x in reader.samples::<f32>() {
        writer.write_sample(x.unwrap()).unwrap();
    }
    writer.finalize().unwrap();
    let i = run(&["describe", &plain]);
    let di: DescribeOut = round_trip(&i);
    match di.perf.source.as_str() {
        "inferred" => {
            let off = di.perf.notes[0].off_s.unwrap();
            assert!((off - 1.5).abs() < 0.1, "inferred note-off {off}");
            assert!(di.envelope.as_ref().unwrap().sustain_db.is_some());
        }
        "none" => {
            assert!(di.perf.why_missing.is_some());
            assert!(di.missing.contains_key("envelope.sustain_db"));
        }
        other => panic!("perf.source {other}"),
    }
}

#[test]
fn check_a_patch_with_margins_and_noise() {
    let s = Scratch::new("check");
    let p = s.new_patch("pluck_1osc", "p.patch.json");
    let list =
        "pitch.note ~ C3, attack < 100ms, sustain < -20dB, beating == null, brightness > 15kHz";
    let o = run(&["check", &p, "--expect", list]);
    assert_eq!(o.code, 1, "one item fails on purpose: {}", o.stdout);
    let c: CheckOut = round_trip(&o);
    assert_eq!((c.passed, c.failed), (4, 1));
    // 15 kHz is out of reach of a saw through a low-pass: no measured move, and the item says why and what to try.
    assert!(c.fix.is_none(), "{:?}", c.fix);
    assert_eq!(c.blocked.len(), 1);
    assert_eq!(c.blocked[0].expect, "spectrum.centroid_hz > 15000");
    assert_eq!(c.blocked[0].reason, "at_bound");
    assert!(c.blocked[0].at_bound.contains(&"filter1.cutoff".to_owned()));
    let hint = c.results[4].hint.as_ref().expect("a structural hint");
    assert!(!hint.measured && hint.text.contains("osc.wave"), "{hint:?}");
    assert!(
        c.results
            .iter()
            .filter(|r| r.got.is_some())
            .all(|r| r.noise.is_some())
    );
    assert_eq!(c.results[0].margin_unit.as_deref(), Some("ct"));
    s.settings()
        .bind(|| assert_snapshot!("check_patch", record(&s, &o, true)));
    let pass = run(&["check", &p, "--expect", "pitch.note ~ C3", "--no-fix"]);
    assert_eq!(pass.code, 0);
    // A WAV gets margins and noise too (from re-analysis), never a fix.
    let wav = s.path("p.wav");
    assert_eq!(run(&["render", &p, "--out", &wav]).code, 0);
    let w = run(&["check", &wav, "--expect", list]);
    let cw: CheckOut = round_trip(&w);
    assert_eq!((cw.passed, cw.failed), (4, 1));
    assert_eq!(cw.perf.source, "wav");
}

/// A failing list the parameters can reach: one measured `fix` whose `after` is a real render, exit 1 still.
#[test]
fn check_offers_a_measured_fix() {
    let s = Scratch::new("check_fix");
    let p = s.new_patch("pluck_1osc", "p.patch.json");
    let list = "pitch.note ~ C3, attack < 100ms, brightness < 700Hz, duration > 1200ms";
    let o = run(&["check", &p, "--expect", list]);
    assert_eq!(o.code, 1, "{}", o.stdout);
    let c: CheckOut = round_trip(&o);
    assert_eq!((c.passed, c.failed), (2, 2));
    let fix = c.fix.as_ref().expect("a fix");
    assert!(fix.measured && fix.pass_all, "{fix:?}");
    assert!(fix.renders >= 2 && fix.renders <= 16, "{}", fix.renders);
    assert_ne!(fix.ops.len(), 0);
    for op in &fix.ops {
        assert!(op["set"].is_string() && op["to"].is_string(), "{op}");
    }
    assert!(fix.after.contains_key("spectrum.centroid_hz"));
    assert_eq!(c.blocked.len(), 0, "{:?}", c.blocked);
    s.settings()
        .bind(|| assert_snapshot!("check_fix", record(&s, &o, true)));

    // --no-fix and a budget of 0 search nothing; a budget of 1 can only measure the base.
    for flags in [&["--no-fix"][..], &["--fix-budget", "0"]] {
        let mut args = vec!["check", p.as_str(), "--expect", list];
        args.extend_from_slice(flags);
        let c: CheckOut = round_trip(&run(&args));
        assert!(c.fix.is_none() && c.blocked.is_empty(), "{flags:?}");
    }
    let one = run(&["check", &p, "--expect", list, "--fix-budget", "1"]);
    let c: CheckOut = round_trip(&one);
    assert!(c.fix.is_none(), "one render is the base: {:?}", c.fix);
}

/// Clipping is undone by `master.level` alone, by exactly the overshoot, in one render.
#[test]
fn clipping_is_fixed_with_the_master_level() {
    let s = Scratch::new("check_clip");
    let p = s.new_patch("init", "hot.patch.json");
    assert_eq!(run(&["apply", &p, "--set", "master.level=10dB"]).code, 0);
    let list = "clipping == false, pitch.note ~ C3";
    let o = run(&["check", &p, "--expect", list]);
    assert_eq!(o.code, 1, "{}", o.stdout);
    let c: CheckOut = round_trip(&o);
    assert_eq!((c.passed, c.failed), (1, 1));
    let fix = c.fix.as_ref().expect("a fix");
    assert!(fix.pass_all && fix.single_param_enough, "{fix:?}");
    assert_eq!(fix.renders, 1, "the base render is the only one");
    assert_eq!(fix.ops.len(), 1);
    assert_eq!(fix.ops[0]["set"], "master.level");
    s.settings()
        .bind(|| assert_snapshot!("check_clip", record(&s, &o, true)));
    // Applying it is what the fix measured.
    let ops = serde_json::to_string(&fix.ops).unwrap();
    let applied = run(&["step", &p, "--ops", &ops, "--expect", list]);
    assert_eq!(applied.code, 0, "{}", applied.stdout);
}

#[test]
fn step_applies_renders_and_checks() {
    let s = Scratch::new("step");
    let p = s.new_patch("pluck_1osc", "p.patch.json");
    let out = s.path("p2.patch.json");
    let o = run(&[
        "step",
        &p,
        "--set",
        "env1.decay=200ms,filter1.cutoff=3kHz",
        "--expect",
        "pitch.note ~ C3, attack < 100ms, sustain < -20dB",
        "--perf",
        "note=C3,hold=1s,dur=2s",
        "--out",
        &out,
    ]);
    assert_eq!(o.code, 0, "{}", o.stdout);
    let st: StepOut = round_trip(&o);
    assert_eq!(st.wav, s.path("p2.wav"));
    assert!(st.check.as_ref().is_some_and(|c| c.pass));
    assert!(st.describe.contains_key("envelope.sustain_db"));
    assert!(
        !st.describe.contains_key("noise_ratio"),
        "only headline and expected"
    );
    s.settings()
        .bind(|| assert_snapshot!("step", record(&s, &o, true)));
    // Without --expect: no check, exit 0; --full echoes everything.
    let full = run(&["step", &out, "--full"]);
    let sf: StepOut = round_trip(&full);
    assert!(sf.check.is_none() && sf.describe.contains_key("noise_ratio"));
}

#[test]
fn bench_reports_wall_clock_as_information() {
    let s = Scratch::new("bench");
    let o = run(&["bench", "--voices", "2", "--seconds", "0.25"]);
    assert_eq!(o.code, 0, "{}", o.stdout);
    let b: BenchOut = round_trip(&o);
    assert!(b.realtime_x_per_core > 0.0 && b.note.contains("information only"));
    let mut v = json(&o);
    v["build"]["profile"] = "<profile>".into();
    v["build"]["target"] = "<target>".into();
    let masked = common::mask(&v);
    s.settings()
        .bind(|| assert_snapshot!("bench", common::pretty(&masked)));
}
