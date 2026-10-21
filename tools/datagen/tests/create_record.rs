//! Lane TP: `enth.create/1` (`enth_solver::create::record`) through JSON, here because this package has `serde_json`
//! and the solver must not add it (a core crate's lock entry). The record survives a round trip, reads leniently (a
//! shell's extra fields are ignored), names its provenance and stop in snake case, and replays the run.
#![allow(
    clippy::disallowed_methods,
    reason = "a test signal: std's sin and exp make the target, never a score"
)]

use enth_patch::{ParamAddr, Patch};
use enth_solver::create::record::RecordContext;
use enth_solver::create::{
    CreateCfg, CreateJob, CreateRecord, Inputs, Portfolio, Target, TargetPerf, Template, run,
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
        Portfolio::build(&analysis, &inputs, &cfg),
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
            cfg: &cfg,
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

    // Lenient: a shell's extra fields (the CLI's hint, the intake report) are ignored when read back. (Edited as
    // text: through `serde_json::Value` an f32 would be rounded twice.)
    let extended = text.replacen(
        '{',
        r#"{"hint": "listen to candidate 0", "intake": {"version": 1},"#,
        1,
    );
    let read: CreateRecord = serde_json::from_str(&extended).unwrap();
    assert_eq!(read, record);

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
