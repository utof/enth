//! Tests the P1b modes (`--state`, `cli-equal`, `bursts`, `note-end`, `latency`, `process-stats`) against the
//! in-process stub plugin of `tests/common`, with a negative control for each: a stub (or a host fault) that
//! misbehaves in the way the check exists to catch must make it fail, or the check proves nothing.
//! (`churn` has its own file: it counts threads and descriptors of the whole process.)

mod common;

use std::path::{Path, PathBuf};

use common::{GAIN, INERT, entry, state_for};
use enth_test_host::cli_equal::{self, Inject as EqualInject, Profile, Status};
use enth_test_host::events::{
    BurstKind, BurstOptions, Inject, NoteEndOptions, SameFrame, run_bursts, run_note_end,
};
use enth_test_host::latency;
use enth_test_host::render::{ProcessThread, RenderConfig, RenderRequest, render_once};
use enth_test_host::state;
use enth_test_host::stats;
use serde_json::{Value, json};

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("enth-th-tests-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

// ---------------------------------------------------------------------------------------------------------------
// --state

fn render_request(script: &str, state: Option<Vec<u8>>) -> RenderRequest {
    RenderRequest {
        script: script.to_owned(),
        params: Vec::new(),
        state,
        config: RenderConfig::new(48_000.0, 64, 0.3).unwrap(),
    }
}

#[test]
fn a_patch_document_loaded_as_state_reaches_the_plugin_and_comes_back() {
    let (entry, id) = entry();
    let patch = r#"{"mode":"ramp","note":"from a .patch.json"}"#;
    let outcome = render_once(
        &entry,
        &id,
        &render_request("C4@0+0.1", Some(state_for(patch))),
        ProcessThread::Dedicated,
        true,
    )
    .unwrap();
    // The ramp mode is in the patch, so the plugin read it: a synth render would start silent at frame 0.
    let first: Vec<f32> = outcome.rendered.samples.iter().take(6).copied().collect();
    assert_eq!(first[0], 0.0);
    assert!(first[2] > 0.0, "{first:?}: a ramp rises from frame 1");
    // And the plugin saves the same patch text in the same field.
    let saved = outcome.state.expect("the stub has the state extension");
    assert_eq!(state::fields_of(&saved).unwrap()["patch"], patch);

    // Negative control: a patch wrapped into a field the plugin does not know leaves it at its default patch.
    let wrong = state::wrap_patch(patch, "not-the-field", "stub-1").unwrap();
    let outcome = render_once(
        &entry,
        &id,
        &render_request("C4@0+0.1", Some(wrong)),
        ProcessThread::Dedicated,
        true,
    )
    .unwrap();
    let saved = outcome.state.unwrap();
    assert_eq!(state::fields_of(&saved).unwrap()["patch"], "{}");
    assert!(
        outcome.rendered.stats.peak > 0.0 && outcome.rendered.samples[0] != 0.0,
        "a synth, not the ramp"
    );
}

#[test]
fn the_state_formats_are_told_apart_and_a_refused_blob_is_an_error() {
    let (entry, id) = entry();
    let patch = r#"{"mode":"ramp"}"#;
    let blob = state_for(patch);
    let envelope = r#"{"version":"stub-1","params":{"gain":{"f32":0.9}},"fields":{"patch":"{\"mode\":\"ramp\"}"}}"#;
    for (bytes, format, kind) in [
        (
            patch.as_bytes().to_vec(),
            state::Format::Auto,
            state::Kind::Patch,
        ),
        (
            envelope.as_bytes().to_vec(),
            state::Format::Auto,
            state::Kind::Envelope,
        ),
        (blob.clone(), state::Format::Auto, state::Kind::Blob),
        (
            patch.as_bytes().to_vec(),
            state::Format::Patch,
            state::Kind::Patch,
        ),
    ] {
        let prepared = state::prepare(bytes, format, "patch", "stub-1").unwrap();
        assert_eq!(prepared.kind, kind);
        let outcome = render_once(
            &entry,
            &id,
            &render_request("C4@0+0.1", Some(prepared.bytes)),
            ProcessThread::Dedicated,
            false,
        )
        .unwrap();
        assert_eq!(outcome.rendered.samples[0], 0.0);
        assert!(
            outcome.rendered.samples[2] > 0.0,
            "{kind:?} reached the plugin"
        );
    }
    // A blob the plugin refuses: bad length prefix.
    let mut bad = blob;
    bad[0] ^= 0xFF;
    assert!(
        render_once(
            &entry,
            &id,
            &render_request("C4@0+0.1", Some(bad)),
            ProcessThread::Dedicated,
            false,
        )
        .is_err()
    );
}

// ---------------------------------------------------------------------------------------------------------------
// cli-equal

/// Writes one case file and its expected file: a `ramp` plugin plays a signal the test can compute itself.
fn golden_dir(name: &str, hash_of_wrong: bool) -> (PathBuf, PathBuf, usize) {
    let dir = scratch(name);
    let (cases, expected) = (dir.join("cases"), dir.join("expected"));
    std::fs::create_dir_all(&cases).unwrap();
    std::fs::create_dir_all(&expected).unwrap();
    let frames = 14_400usize; // 0.3 s at 48 kHz
    std::fs::write(
        cases.join("ramp.toml"),
        "description = \"a plugin that plays a ramp\"\nsample_rate = 48000\nseconds = 0.3\n\n[source]\nkind = \"patch\"\nnotes = [ { key = 60, velocity = 0.8, on_s = 0.0, off_s = 0.1 } ]\n\n[source.patch]\nmode = \"ramp\"\n",
    )
    .unwrap();
    let mut samples = Vec::with_capacity(frames * 2);
    for i in 0..frames {
        let v = 0.5 * ((i as u64 % 1000) as f32 / 1000.0);
        samples.extend([v, v]);
    }
    if hash_of_wrong {
        samples[1] += 1.0e-6;
    }
    let hash = cli_equal::engine_hash(48_000, 2, &samples);
    std::fs::write(
        expected.join("ramp.toml"),
        format!("sample_rate = 48000\nframes = {frames}\nhash = \"{hash}\"\n"),
    )
    .unwrap();
    (cases, expected, frames)
}

fn equal_options(cases: &Path, expected: Option<&Path>) -> cli_equal::Options {
    cli_equal::Options {
        cases: vec![cases.to_owned()],
        expected_dir: expected.map(Path::to_owned),
        only: Vec::new(),
        blocks: cli_equal::DEFAULT_BLOCKS.to_vec(),
        profile: Profile::default(),
        state_field: "patch".to_owned(),
        min_compared: 1,
        write_expected: None,
        inject: EqualInject::None,
    }
}

#[test]
fn cli_equal_agrees_with_an_independently_computed_engine_hash() {
    let (entry, id) = entry();
    let (cases, expected, _) = golden_dir("equal", false);
    let report = cli_equal::run(&entry, &id, &equal_options(&cases, Some(&expected))).unwrap();
    assert_eq!(report.cases.len(), 1);
    let case = &report.cases[0];
    assert_eq!(case.status, Status::Equal, "{:?}", case.reason);
    assert_eq!(case.renders.len(), 4, "every block size was rendered");
    assert!(
        case.renders
            .iter()
            .all(|r| Some(&r.sha256) == case.expected.as_ref())
    );
    assert!(report.ok());
}

#[test]
fn cli_equal_fails_for_a_wrong_hash_a_flipped_bit_and_a_block_dependent_plugin() {
    let (entry, id) = entry();
    // The engine's hash is of a different signal.
    let (cases, expected, _) = golden_dir("wrong", true);
    let report = cli_equal::run(&entry, &id, &equal_options(&cases, Some(&expected))).unwrap();
    assert_eq!(report.cases[0].status, Status::Differs);
    assert!(!report.ok());

    // The plugin is right but the host flips one bit before hashing.
    let (cases, expected, _) = golden_dir("flip", false);
    let mut options = equal_options(&cases, Some(&expected));
    options.inject = EqualInject::FlipBit;
    let report = cli_equal::run(&entry, &id, &options).unwrap();
    assert_eq!(report.cases[0].status, Status::Differs);

    // A plugin whose sound depends on the host's block size, whatever the expected file says.
    let (cases, _, _) = golden_dir("blockdep", false);
    let text = std::fs::read_to_string(cases.join("ramp.toml")).unwrap();
    std::fs::write(
        cases.join("ramp.toml"),
        text.replace("\"ramp\"", "\"ramp-per-block\""),
    )
    .unwrap();
    let report = cli_equal::run(&entry, &id, &equal_options(&cases, None)).unwrap();
    assert_eq!(report.cases[0].status, Status::BlockDependent);
    assert!(!report.ok());
}

#[test]
fn cli_equal_without_expected_hashes_checks_block_independence_and_can_write_them() {
    let (entry, id) = entry();
    let (cases, _, _) = golden_dir("noexpected", false);
    let report = cli_equal::run(&entry, &id, &equal_options(&cases, None)).unwrap();
    assert_eq!(report.cases[0].status, Status::Equal);
    // Writing expected files from the plugin's own render, then comparing against them.
    let out = scratch("written");
    let mut options = equal_options(&cases, None);
    options.write_expected = Some(out.clone());
    cli_equal::run(&entry, &id, &options).unwrap();
    let written = std::fs::read_to_string(out.join("ramp.toml")).unwrap();
    assert!(written.contains("hash = \""), "{written}");
    let report = cli_equal::run(&entry, &id, &equal_options(&cases, Some(&out))).unwrap();
    assert_eq!(report.cases[0].status, Status::Equal);
}

#[test]
fn cli_equal_skips_what_a_plugin_cannot_play_and_does_not_pass_when_nothing_was_compared() {
    let (entry, id) = entry();
    let (cases, expected, _) = golden_dir("skip", false);
    let text = std::fs::read_to_string(cases.join("ramp.toml")).unwrap();
    // A seed the plugin cannot take.
    std::fs::write(
        cases.join("ramp.toml"),
        text.replace("[source]\n", "[source]\nseed = 7\n"),
    )
    .unwrap();
    let report = cli_equal::run(&entry, &id, &equal_options(&cases, Some(&expected))).unwrap();
    assert_eq!(report.cases[0].status, Status::Skipped);
    assert_eq!(report.compared(), 0);
    assert!(!report.ok(), "all skipped is not a pass");
}

// ---------------------------------------------------------------------------------------------------------------
// bursts

fn burst_options(counts: &[usize], state: Option<Vec<u8>>) -> BurstOptions {
    BurstOptions {
        counts: counts.to_vec(),
        kind: BurstKind::Mixed,
        block: 64,
        sample_rate: 48_000.0,
        capacity: None,
        state,
        warm_blocks: 4,
        after_blocks: 4,
        tail_s: 0.05,
        allow_unended: false,
        inject: Inject::default(),
        orders: vec![SameFrame::NotesFirst, SameFrame::ParamsFirst],
    }
}

fn failures_of(report: &Value) -> Vec<String> {
    report["trials"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|t| t["failures"].as_array().unwrap().iter())
        .map(|f| f.as_str().unwrap().to_owned())
        .collect()
}

fn says(report: &Value, needle: &str) -> bool {
    failures_of(report).iter().any(|f| f.contains(needle))
}

#[test]
fn a_plugin_that_handles_every_event_passes_bursts_at_every_size_and_order() {
    let (entry, id) = entry();
    let report = run_bursts(&entry, &id, &burst_options(&[16, 256, 1024], None)).unwrap();
    assert_eq!(report["ok"], true, "{:?}", failures_of(&report));
    let trials = report["trials"].as_array().unwrap();
    assert_eq!(trials.len(), 6, "three counts in two orders");
    assert!(
        trials
            .iter()
            .all(|t| t["applied_all"] == true && t["recovered"] == true)
    );
    assert_eq!(report["applied_all_up_to_events"]["notes-first"], 1024);
    assert_eq!(report["applied_all_up_to_events"]["params-first"], 1024);
}

#[test]
fn bursts_report_a_plugin_that_drops_events_past_its_capacity() {
    let (entry, id) = entry();
    // A documented capacity of 48 and a plugin that really keeps 48: not a failure, and the report says where it ends.
    let state = Some(state_for(r#"{"cap":48,"policy":"drop"}"#));
    let mut options = burst_options(&[16, 256], state.clone());
    options.capacity = Some(48);
    options.allow_unended = true; // a dropped note-on has no NOTE_END; the capacity policy is documented
    let report = run_bursts(&entry, &id, &options).unwrap();
    assert_eq!(report["ok"], true, "{:?}", failures_of(&report));
    assert_eq!(report["applied_all_up_to_events"]["params-first"], 16);
    let big = &report["trials"].as_array().unwrap()[3];
    assert_eq!(big["events"], 256);
    assert_eq!(big["applied_all"], false, "256 events do not fit 48");

    // Negative control: the same plugin held to a capacity of 256 fails the burst of 256.
    let mut options = burst_options(&[16, 256], state);
    options.capacity = Some(256);
    options.allow_unended = true;
    let report = run_bursts(&entry, &id, &options).unwrap();
    assert_eq!(report["ok"], false);
    assert!(
        says(&report, "within the declared capacity 256"),
        "{:?}",
        failures_of(&report)
    );
}

#[test]
fn bursts_catch_a_wedged_event_path_nan_and_nondeterminism() {
    let (entry, id) = entry();
    let run = |patch: &str| {
        let mut options = burst_options(&[256], Some(state_for(patch)));
        options.orders = vec![SameFrame::ParamsFirst];
        options.allow_unended = true;
        run_bursts(&entry, &id, &options).unwrap()
    };
    // After an overflow the plugin ignores every later note: the probe note after the burst stays silent.
    let wedged = run(r#"{"cap":8,"policy":"wedge"}"#);
    assert!(
        wedged["ok"] == false && says(&wedged, "probe note"),
        "{:?}",
        failures_of(&wedged)
    );
    let nan = run(r#"{"cap":8,"policy":"nan"}"#);
    assert!(
        nan["ok"] == false && says(&nan, "non-finite"),
        "{:?}",
        failures_of(&nan)
    );
    let shared = run(r#"{"nondeterministic":true}"#);
    assert!(
        shared["ok"] == false && says(&shared, "different samples"),
        "{:?}",
        failures_of(&shared)
    );
    // And the control for all three: the same run on a well-behaved plugin passes.
    let fine = run("{}");
    assert_eq!(fine["ok"], true, "{:?}", failures_of(&fine));
}

#[test]
fn the_host_can_deliver_fewer_events_to_show_the_burst_check_fails() {
    let (entry, id) = entry();
    let mut options = burst_options(&[64], None);
    options.orders = vec![SameFrame::ParamsFirst];
    options.capacity = Some(1024);
    options.inject = Inject::parse("drop-after=10").unwrap();
    let report = run_bursts(&entry, &id, &options).unwrap();
    assert_eq!(report["ok"], false);
    assert!(
        says(&report, "not all applied"),
        "{:?}",
        failures_of(&report)
    );
    options.inject = Inject::default();
    assert_eq!(run_bursts(&entry, &id, &options).unwrap()["ok"], true);
}

#[test]
fn bursts_see_note_end_faults() {
    let (entry, id) = entry();
    let expect = |patch: &str, needle: &str| {
        let mut options = burst_options(&[64], Some(state_for(patch)));
        options.orders = vec![SameFrame::ParamsFirst];
        let report = run_bursts(&entry, &id, &options).unwrap();
        assert!(
            report["ok"] == false && says(&report, needle),
            "{patch}: {:?}",
            failures_of(&report)
        );
    };
    expect(r#"{"note_end":"missing"}"#, "never got a NOTE_END");
    expect(r#"{"note_end":"dup"}"#, "ended more than once");
    expect(r#"{"note_end":"wrong_key"}"#, "different key");
}

// ---------------------------------------------------------------------------------------------------------------
// note-end

fn note_end_options(state: Option<Vec<u8>>) -> NoteEndOptions {
    NoteEndOptions {
        script: "C4@0+0.1, E4@0.01+0.05, G4@0.02+0.2, B4@0.1+0.00002, C5@0.1+0.1".to_owned(),
        blocks: vec![1, 64, 100, 512],
        sample_rate: 48_000.0,
        tail_s: 0.05,
        state,
        allow_unended: false,
        inject: Inject::default(),
    }
}

#[test]
fn note_end_passes_a_correct_plugin_at_every_block_size() {
    let (entry, id) = entry();
    let report = run_note_end(&entry, &id, &note_end_options(None)).unwrap();
    assert_eq!(report["ok"], true, "{report}");
    assert_eq!(report["same_notes_end_at_every_block_size"], true);
    for block in report["blocks"].as_array().unwrap() {
        assert_eq!(block["note_end"]["ended_once"], 5, "{block}");
    }
}

#[test]
fn note_end_fails_for_each_way_a_plugin_can_report_wrongly() {
    let (entry, id) = entry();
    for (patch, needle) in [
        (r#"{"note_end":"missing"}"#, "never got a NOTE_END"),
        (r#"{"note_end":"dup"}"#, "ended more than once"),
        (r#"{"note_end":"wrong_key"}"#, "different key"),
        (r#"{"note_end":"outside"}"#, "past the end of their block"),
    ] {
        let report = run_note_end(&entry, &id, &note_end_options(Some(state_for(patch)))).unwrap();
        assert_eq!(report["ok"], false, "{patch} passed: {report}");
        let text = report.to_string();
        assert!(
            text.contains(needle),
            "{patch}: expected {needle:?} in {text}"
        );
    }
    // A NOTE_END reported before its note started: the short note begins at frame 20 of a block and ends in it.
    let mut options = note_end_options(Some(state_for(r#"{"note_end":"early"}"#)));
    options.script = "C4@0.0004167+0.0002".to_owned(); // frames 20 to 30
    options.blocks = vec![64, 512];
    let report = run_note_end(&entry, &id, &options).unwrap();
    assert!(
        report["ok"] == false && report.to_string().contains("before their note-on"),
        "{report}"
    );
}

#[test]
fn note_end_host_faults_make_a_correct_plugin_fail() {
    let (entry, id) = entry();
    let mut options = note_end_options(None);
    options.inject = Inject::parse("drop-end=2").unwrap();
    assert_eq!(run_note_end(&entry, &id, &options).unwrap()["ok"], false);
    options.inject = Inject::parse("late-end=70").unwrap();
    assert_eq!(run_note_end(&entry, &id, &options).unwrap()["ok"], false);
    options.inject = Inject::default();
    assert_eq!(run_note_end(&entry, &id, &options).unwrap()["ok"], true);
}

// ---------------------------------------------------------------------------------------------------------------
// latency

fn latency_options(param: &str, state: Option<Vec<u8>>) -> latency::Options {
    latency::Options {
        param: param.to_owned(),
        value: 0.1,
        from: Some(1.0),
        notes: "C4@0+60:0.8, G4@0+60:0.8".to_owned(),
        sample_rate: 48_000.0,
        block: 64,
        at_s: 0.25,
        offsets: vec![0, 17, 63],
        after_s: 0.2,
        threshold_db: -60.0,
        state,
        inject_delay_blocks: 0,
        max_onset_ms: None,
        max_audible_ms: None,
    }
}

fn onsets(report: &Value) -> Vec<Option<u64>> {
    report["offsets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["onset_frames"].as_u64())
        .collect()
}

#[test]
fn latency_of_a_sample_accurate_plugin_is_zero_at_every_offset() {
    let (entry, id) = entry();
    let report = latency::run(&entry, &id, &latency_options("Gain", None)).unwrap();
    assert_eq!(report["ok"], true, "{report}");
    assert_eq!(onsets(&report), [Some(0), Some(0), Some(0)]);
    assert_eq!(report["summary"]["sample_accurate"], true);
    assert_eq!(report["summary"]["events_land_on_a_grid"], false);
}

#[test]
fn latency_sees_events_applied_at_the_next_block_start() {
    let (entry, id) = entry();
    let state = Some(state_for(r#"{"param_mode":"block"}"#));
    let report = latency::run(&entry, &id, &latency_options("Gain", state)).unwrap();
    // The event at offset 0, 17 or 63 is applied at the start of the next block: 64, 47 or 1 frames later (give or
    // take a frame where the held notes' sum happens to be zero).
    let got = onsets(&report);
    for (got, want) in got.iter().zip([64u64, 47, 1]) {
        let got = got.expect("the change is heard");
        assert!(
            (want..=want + 3).contains(&got),
            "onset {got}, expected about {want}"
        );
    }
    assert_eq!(report["summary"]["sample_accurate"], false);
    assert_eq!(report["summary"]["events_land_on_a_grid"], true);
}

#[test]
fn latency_budgets_and_delays_fail_a_slow_plugin() {
    let (entry, id) = entry();
    let state = Some(state_for(r#"{"param_mode":"delay:96"}"#));
    let mut options = latency_options("Gain", state);
    options.max_onset_ms = Some(1.0); // 96 frames are 2 ms
    let report = latency::run(&entry, &id, &options).unwrap();
    assert_eq!(report["ok"], false);
    assert!(
        report.to_string().contains("exceeds the budget"),
        "{report}"
    );
    options.max_onset_ms = Some(3.0);
    assert_eq!(latency::run(&entry, &id, &options).unwrap()["ok"], true);

    // The host delivering the event three blocks late is the same fault from outside.
    let mut options = latency_options("Gain", None);
    options.inject_delay_blocks = 3;
    options.max_onset_ms = Some(1.0);
    let report = latency::run(&entry, &id, &options).unwrap();
    assert_eq!(report["ok"], false);
    assert_eq!(onsets(&report), [Some(192), Some(192), Some(192)]);
}

#[test]
fn latency_measures_a_smoother_in_the_settling_time() {
    let (entry, id) = entry();
    let state = Some(state_for(r#"{"smooth_ms":10}"#));
    let report = latency::run(&entry, &id, &latency_options("Gain", state)).unwrap();
    assert_eq!(report["ok"], true, "{report}");
    for offset in report["offsets"].as_array().unwrap() {
        let settled = offset["settled_ms"].as_f64().unwrap();
        // 90 % of a one-pole step is 2.3 time constants.
        assert!((12.0..=32.0).contains(&settled), "settled in {settled} ms");
        assert!(
            offset["audible_ms"].as_f64().unwrap() < 2.0,
            "audible at once: {offset}"
        );
        assert_eq!(offset["onset_frames"], 0);
    }
}

#[test]
fn latency_fails_for_a_parameter_that_does_nothing_and_a_plugin_that_is_not_deterministic() {
    let (entry, id) = entry();
    let report = latency::run(&entry, &id, &latency_options("Inert", None)).unwrap();
    assert_eq!(report["ok"], false);
    assert!(report.to_string().contains("no audible change"), "{report}");
    assert_eq!(onsets(&report), [None, None, None]);
    // Parameters can be named by number too.
    let by_id = latency::run(&entry, &id, &latency_options(&GAIN.to_string(), None)).unwrap();
    assert_eq!(by_id["ok"], true);
    assert_ne!(GAIN, INERT);

    let state = Some(state_for(r#"{"nondeterministic":true}"#));
    let report = latency::run(&entry, &id, &latency_options("Gain", state)).unwrap();
    assert_eq!(report["ok"], false);
    assert!(report.to_string().contains("not deterministic"), "{report}");
}

// ---------------------------------------------------------------------------------------------------------------
// process-stats

fn stats_options(state: Option<Vec<u8>>) -> stats::Options {
    stats::Options {
        secs: 0.5,
        warmup_secs: 0.1,
        rt: None,
        notes: "C4@0+1000:0.8, G4@0+1000:0.8".to_owned(),
        state,
        ..stats::Options::default()
    }
}

fn p(report: &Value, key: &str) -> f64 {
    report["plugin_process"][key].as_f64().unwrap()
}

#[test]
fn process_stats_measures_what_process_costs() {
    let (entry, id) = entry();
    // 300 us spent inside every process() call shows in the distribution, from p50 up.
    let slow = Some(state_for(r#"{"busy_us":300}"#));
    let (slow_report, ok) = stats::run(&entry, &id, &stats_options(slow)).unwrap();
    assert!(ok, "{slow_report}");
    assert!(
        p(&slow_report, "p50_us") >= 295.0,
        "p50 {}",
        p(&slow_report, "p50_us")
    );
    assert!(p(&slow_report, "max_us") >= p(&slow_report, "p99_us"));
    let n = slow_report["plugin_process"]["n"].as_u64().unwrap();
    assert!(n >= 300, "{n} blocks measured");
    assert_eq!(slow_report["real_time"], false);
    assert_eq!(slow_report["warnings"].as_array().unwrap().len(), 1, "{slow_report}");
    assert!(
        slow_report["environment"]["uptime"].is_string()
            || slow_report["environment"]["uptime"].is_null()
    );
    assert!(slow_report["environment"]["uptime_seconds"].is_string());

    // And the cheap plugin is cheaper: the measurement can tell them apart.
    let (cheap, _) = stats::run(&entry, &id, &stats_options(None)).unwrap();
    assert!(p(&cheap, "p50_us") < p(&slow_report, "p50_us"));
    let counts: u64 = cheap["plugin_process"]["histogram_us"]["counts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c.as_u64().unwrap())
        .sum();
    assert_eq!(counts, cheap["plugin_process"]["n"].as_u64().unwrap());
}

#[test]
fn process_stats_budgets_fail_a_plugin_over_them_and_pass_one_under() {
    let (entry, id) = entry();
    // A tail spike: 2 ms every 50 blocks is rare enough for p99 = p50 but not for the max.
    let spiky = Some(state_for(r#"{"busy_us":2000,"busy_every":50}"#));
    let mut options = stats_options(spiky);
    options.max_p999_us = Some(500.0);
    let (report, ok) = stats::run(&entry, &id, &options).unwrap();
    assert!(!ok, "p99.9 {} us", p(&report, "p999_us"));
    assert_eq!(report["ok"], false);
    assert!(
        report["failures"][0].as_str().unwrap().contains("p99.9"),
        "{report}"
    );
    // The control: the same budgets on the quiet plugin pass.
    let mut options = stats_options(None);
    options.max_p99_us = Some(50_000.0);
    options.max_p999_us = Some(50_000.0);
    let (report, ok) = stats::run(&entry, &id, &options).unwrap();
    assert!(ok, "{report}");
    // A host-injected slowdown inside the timed window has the same effect on a quiet plugin.
    let mut options = stats_options(None);
    options.inject = Some(enth_test_host::render::BusyFault::parse("busy-us=400,every=5").unwrap());
    options.max_p99_us = Some(200.0);
    let (_, ok) = stats::run(&entry, &id, &options).unwrap();
    assert!(!ok);
}

#[test]
fn process_stats_runs_instances_on_several_threads_and_requires_the_priority_it_is_asked_for() {
    let (entry, id) = entry();
    let mut options = stats_options(None);
    options.instances = 3;
    options.threads = 2;
    let (report, ok) = stats::run(&entry, &id, &options).unwrap();
    assert!(ok, "{report}");
    assert_eq!(report["threads"].as_array().unwrap().len(), 2);
    assert_eq!(report["instances"].as_array().unwrap().len(), 3);
    // Priority 99 is above anything rtkit grants (and rtkit may be absent altogether): the thread stays at normal
    // priority and --require-rt turns that into a failure.
    let mut options = stats_options(None);
    options.rt = Some(99);
    options.require_rt = true;
    let (report, ok) = stats::run(&entry, &id, &options).unwrap();
    assert!(!ok, "{report}");
    assert_eq!(report["real_time"], false);
    assert!(
        report["failures"].to_string().contains("real-time"),
        "{report}"
    );
    assert_eq!(report["threads"][0]["scheduling"]["real_time"], false);
}

#[test]
fn bad_stats_options_are_errors() {
    let (entry, id) = entry();
    let mut options = stats_options(None);
    options.instances = 0;
    assert!(stats::run(&entry, &id, &options).is_err());
    let mut options = stats_options(None);
    options.secs = 0.0;
    assert!(stats::run(&entry, &id, &options).is_err());
    let _ = json!({});
}
