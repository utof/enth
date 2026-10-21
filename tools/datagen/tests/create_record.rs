//! Lane TP: `enth.create/1` (`enth_solver::create::record`) through JSON, here because this package has `serde_json`
//! and the solver must not add it (a core crate's lock entry). The record survives a round trip, reads leniently (a
//! shell's extra fields are ignored, an unknown enum value reads as `unknown`), names its provenance and stop in snake
//! case, and replays the run. Lane F1S: a run the shell's clock stopped replays from its JSON alone (the F1s judge's
//! M1), and the intake's report goes in whole and comes back (M3).
#![allow(
    clippy::disallowed_methods,
    reason = "a test signal: std's sin and exp make the target, never a score"
)]

use enth_analysis::intake::{FloorSource, Intake, Warning};
use enth_patch::{ParamAddr, Patch};
use enth_solver::create::record::RecordContext;
use enth_solver::create::{
    CandidateState, CreateCfg, CreateJob, CreateRecord, CreateStop, Inputs, Portfolio, Provenance,
    Target, TargetPerf, Template, run,
};

fn template(name: &str, text: &str) -> Template {
    Template {
        name: name.to_owned(),
        patch: enth_patch::file::load(text).unwrap().patch,
    }
}

/// A sine at A3 with a decay: no render needed for the target.
fn target() -> Target {
    let sr = 22_050u32;
    let n = (f64::from(sr) * 0.6) as usize;
    let audio = (0..n)
        .map(|i| {
            let t = i as f64 / f64::from(sr);
            (0.5 * (-4.0 * t).exp() * (2.0 * core::f64::consts::PI * 220.0 * t).sin()) as f32
        })
        .collect();
    Target {
        audio,
        sample_rate: sr,
        render_seed: 1,
        perf: TargetPerf {
            key: 57,
            velocity: 100.0 / 127.0,
            on_s: 0.0,
            hold_s: 0.3,
            duration_s: 0.6,
            f0_hz: Some(220.0),
        },
        bandwidth_hz: Some(9_000.0),
        noise_floor_dbfs: Some(-80.0),
    }
}

fn proxy(p: &Patch) -> f64 {
    let addr: ParamAddr = "env1.decay".parse().unwrap();
    match (p.spec(&addr), p.value(&addr)) {
        (Some(s), Some(v)) => f64::from((s.to_norm(v) - s.to_norm(250.0)).abs()),
        _ => 1.0,
    }
}

#[test]
fn the_record_round_trips_through_json_and_replays() {
    let t = target();
    let init = enth_patch::file::load(include_str!("../../../content/templates/init.patch.json"))
        .unwrap()
        .patch;
    let templates = vec![
        template(
            "pluck_1osc",
            include_str!("../../../content/templates/pluck_1osc.patch.json"),
        ),
        template(
            "bass_sub",
            include_str!("../../../content/templates/bass_sub.patch.json"),
        ),
    ];
    let inputs = Inputs {
        init: &init,
        templates: &templates,
        bank: None,
        proposals: &[],
    };
    let cfg = CreateCfg {
        k: 2,
        budget: 150,
        ..CreateCfg::default()
    };
    let analysis = t.analyse().unwrap();
    let mut job = CreateJob::new(
        &t,
        &analysis,
        Portfolio::build(&t, &analysis, &inputs, &cfg),
        &cfg,
    )
    .unwrap();
    run(&mut job, &mut proxy, &mut |_| true).unwrap();
    let result = job.result();
    let names: Vec<String> = templates.iter().map(|x| x.name.clone()).collect();
    let record = CreateRecord::new(
        &result,
        &RecordContext {
            target: &t,
            analysis: &analysis,
            intake: None,
            objective: "proxy/0",
            bank: None,
            templates: &names,
        },
    );

    let text = serde_json::to_string_pretty(&record).unwrap();
    let back: CreateRecord = serde_json::from_str(&text).unwrap();
    assert_eq!(back, record);

    let v: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(v["schema"], "enth.create/1");
    assert_eq!(v["starts"][0]["provenance"]["kind"], "heuristic");
    assert_eq!(v["starts"][1]["provenance"]["kind"], "template");
    assert_eq!(v["starts"][1]["provenance"]["name"], "pluck_1osc");
    assert!(
        v["stop"] == "budget" || v["stop"] == "exhausted",
        "{}",
        v["stop"]
    );
    assert_eq!(
        v["starts"][0]["patch"]["schema"], 1,
        "patches are patch files"
    );
    assert_eq!(v["input"]["bandwidth_hz"], 9_000.0);
    assert_eq!(v["input"]["perf"]["key"], 57);
    assert_eq!(v["settings"]["embedding"], "knn-logmel-rel/1");
    assert!(
        v["rounds"][0]["seconds"].is_null(),
        "the shell stamps seconds"
    );

    // Lenient: a shell's extra fields (the CLI's hint) are ignored when read back, and an intake report this build
    // cannot read (a newer intake's) reads as absent. (Edited as text: through `serde_json::Value` an f32 would be
    // rounded twice.)
    let extended = text.replacen(
        '{',
        r#"{"hint": "listen to candidate 0", "shell": {"version": 1},"#,
        1,
    );
    let read: CreateRecord = serde_json::from_str(&extended).unwrap();
    assert_eq!(read, record);
    let newer_intake = text.replacen(r#""intake": null"#, r#""intake": {"version": 9}"#, 1);
    assert_ne!(newer_intake, text);
    let read: CreateRecord = serde_json::from_str(&newer_intake).unwrap();
    assert_eq!(read, record);

    // Tolerant: an enum value a newer build wrote reads as `unknown` rather than refusing the record.
    let newer = text
        .replacen(r#""kind": "heuristic""#, r#""kind": "preset", "name": "x""#, 1)
        .replacen(r#""stop": ""#, r#""stop": "deadline", "was": ""#, 1);
    let read: CreateRecord = serde_json::from_str(&newer).unwrap();
    assert_eq!(read.starts[0].provenance, Provenance::Unknown);
    assert_eq!(read.stop, CreateStop::Unknown);
    let state = serde_json::from_str::<CandidateState>(r#""paused""#).unwrap();
    assert_eq!(state, CandidateState::Unknown);
    let w = serde_json::from_str::<Warning>(r#"{"kind": "reverb", "rt60_s": 1.2}"#).unwrap();
    assert_eq!(w, Warning::Unknown);
    let f = serde_json::from_str::<FloorSource>(r#""trailing""#).unwrap();
    assert_eq!(f, FloorSource::Unknown);

    // Replay from the JSON's starts and settings.
    let mut again = CreateJob::new(&t, &analysis, back.starts().unwrap(), &back.cfg()).unwrap();
    run(&mut again, &mut proxy, &mut |_| true).unwrap();
    let replay = again.result();
    assert_eq!(replay.renders, result.renders);
    assert_eq!(replay.rounds, result.rounds);
    for (a, b) in replay.candidates.iter().zip(&result.candidates) {
        assert_eq!(a.distance, b.distance);
        assert_eq!(a.patch.to_json(), b.patch.to_json());
    }
}

/// The F1s judge's M1 through JSON: a run the shell's clock stopped (the way time enters a job now that the budget is
/// only a cap) replays from its record alone, under the settings it ran under with its renders as the budget.
#[test]
fn a_run_stopped_by_the_clock_replays_from_its_json() {
    let t = target();
    let init = enth_patch::file::load(include_str!("../../../content/templates/init.patch.json"))
        .unwrap()
        .patch;
    let templates = vec![template(
        "pad_2osc",
        include_str!("../../../content/templates/pad_2osc.patch.json"),
    )];
    let inputs = Inputs {
        init: &init,
        templates: &templates,
        bank: None,
        proposals: &[],
    };
    let cfg = CreateCfg {
        k: 2,
        budget: 5_000,
        first_rung: 20,
        ..CreateCfg::default()
    };
    let analysis = t.analyse().unwrap();
    let mut job = CreateJob::new(
        &t,
        &analysis,
        Portfolio::build(&t, &analysis, &inputs, &cfg),
        &cfg,
    )
    .unwrap();
    let mut rounds = 0;
    run(&mut job, &mut proxy, &mut |_| {
        rounds += 1;
        rounds < 9
    })
    .unwrap();
    let result = job.result();
    let names = vec!["pad_2osc".to_owned()];
    let record = CreateRecord::new(
        &result,
        &RecordContext {
            target: &t,
            analysis: &analysis,
            intake: None,
            objective: "proxy/0",
            bank: None,
            templates: &names,
        },
    );
    let back: CreateRecord =
        serde_json::from_str(&serde_json::to_string(&record).unwrap()).unwrap();
    assert_eq!(back.cfg(), cfg);
    assert_eq!(back.stop, CreateStop::Cancelled);
    let mut again =
        CreateJob::new(&t, &analysis, back.starts().unwrap(), &back.replay_cfg()).unwrap();
    run(&mut again, &mut proxy, &mut |_| true).unwrap();
    let replay = again.result();
    assert_eq!(replay.renders, result.renders);
    assert_eq!(replay.rounds, result.rounds);
    for (a, b) in replay.candidates.iter().zip(&result.candidates) {
        assert_eq!(a.distance, b.distance);
        assert_eq!(a.trace, b.trace);
        assert_eq!(a.patch.to_json(), b.patch.to_json());
    }
}

/// The F1s judge's M3: the intake's report goes into the record whole, with the settings it ran under, and comes back
/// from JSON (`enth-analysis` has no JSON reader, so this is the report's read-back test); the performance played
/// sits beside the one the intake inferred.
#[test]
fn the_intake_report_goes_in_whole_and_reads_back() {
    let sr = 22_050u32;
    let mut file = vec![0.0f32; sr as usize / 10];
    file.extend(target().audio);
    let it = Intake::new(&file, 1, sr).unwrap();
    let mut t = Target::from_intake(&it, enth_render::DEFAULT_SEED);
    t.perf.key += 12; // the CLI's --perf override: what was played differs from what was inferred
    let init = enth_patch::file::load(include_str!("../../../content/templates/init.patch.json"))
        .unwrap()
        .patch;
    let inputs = Inputs {
        init: &init,
        templates: &[],
        bank: None,
        proposals: &[],
    };
    let cfg = CreateCfg {
        k: 1,
        budget: 40,
        ..CreateCfg::default()
    };
    let analysis = t.analyse().unwrap();
    let mut job = CreateJob::new(
        &t,
        &analysis,
        Portfolio::build(&t, &analysis, &inputs, &cfg),
        &cfg,
    )
    .unwrap();
    run(&mut job, &mut proxy, &mut |_| true).unwrap();
    let record = CreateRecord::new(
        &job.result(),
        &RecordContext {
            target: &t,
            analysis: &analysis,
            intake: Some(&it.report),
            objective: "proxy/0",
            bank: None,
            templates: &[],
        },
    );
    let text = serde_json::to_string_pretty(&record).unwrap();
    let back: CreateRecord = serde_json::from_str(&text).unwrap();
    assert_eq!(back.intake.as_ref(), Some(&it.report));
    let v: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(v["intake"]["version"], 1);
    assert_eq!(v["intake"]["cfg"]["max_len_s"], 5.0);
    assert_eq!(v["input"]["perf"]["key"], v["intake"]["perf"]["key"].as_u64().unwrap() + 12);
    // A record without an intake (a target built another way) reads with `intake` absent.
    let bare = text.replacen(r#""intake": {"#, r#""intake_was": {"#, 1);
    let read: CreateRecord = serde_json::from_str(&bare).unwrap();
    assert_eq!(read.intake, None);
}
