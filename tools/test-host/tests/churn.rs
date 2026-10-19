//! `churn` counts the threads, descriptors and memory of the whole process, so this file holds one test that
//! runs its scenarios one after another: nothing else may start threads or open files meanwhile.

mod common;

use common::{entry, state_for};
use enth_test_host::churn::{self, Inject, Options};
use serde_json::Value;

fn options(patch: Option<&str>) -> Options {
    Options {
        cycles: 40,
        warmup: 8,
        settle_ms: 5,
        state: patch.map(state_for),
        ..Options::default()
    }
}

fn checks(report: &Value) -> (bool, bool, bool) {
    (
        report["checks"]["pss"] == true,
        report["checks"]["threads"] == true,
        report["checks"]["fds"] == true,
    )
}

#[test]
fn churn_passes_a_clean_plugin_and_fails_each_kind_of_leak() {
    let (entry, id) = entry();

    // The control: a plugin that cleans up after itself passes all three checks.
    let clean = churn::run(&entry, &id, &options(None)).unwrap();
    assert_eq!(clean["ok"], true, "{clean}");
    assert_eq!(checks(&clean), (true, true, true));
    assert_eq!(clean["series"]["pss_kb"].as_array().unwrap().len(), 40);
    assert!(
        clean["summary"]["pss_kb_per_cycle_after_warmup"]
            .as_f64()
            .unwrap()
            < 1.0
    );

    // A plugin that leaks 32 KB per instance fails the memory check and only that.
    let leaky = churn::run(&entry, &id, &options(Some(r#"{"leak":"mem:32"}"#))).unwrap();
    assert_eq!(leaky["ok"], false);
    assert_eq!(checks(&leaky), (false, true, true), "{leaky}");
    assert!(
        leaky["summary"]["pss_kb_per_cycle_after_warmup"]
            .as_f64()
            .unwrap()
            > 20.0
    );

    // A plugin that keeps a file descriptor per instance.
    let fd = churn::run(&entry, &id, &options(Some(r#"{"leak":"fd"}"#))).unwrap();
    assert_eq!(fd["ok"], false);
    assert!(!checks(&fd).2, "{fd}");
    assert_eq!(fd["summary"]["fd_growth_after_warmup"], 31);

    // A plugin that leaves a thread per instance (thread stacks also show in memory, so only threads are asserted).
    let thread = churn::run(&entry, &id, &options(Some(r#"{"leak":"thread"}"#))).unwrap();
    assert_eq!(thread["ok"], false);
    assert!(!checks(&thread).1, "{thread}");
    assert_eq!(thread["summary"]["thread_growth_after_warmup"], 31);

    // The same faults made by the host itself.
    for (inject, which) in [("leak-kb=32", 0), ("fd", 2), ("thread", 1)] {
        let mut o = options(None);
        o.inject = Some(Inject::parse(inject).unwrap());
        let report = churn::run(&entry, &id, &o).unwrap();
        assert_eq!(report["ok"], false, "--inject {inject} passed: {report}");
        let c = checks(&report);
        let failed = [c.0, c.1, c.2];
        assert!(
            !failed[which],
            "--inject {inject} should fail check {which}: {report}"
        );
    }

    // And a threshold set above the fault lets it through, so the limits are what decide.
    let mut loose = options(Some(r#"{"leak":"mem:32"}"#));
    loose.max_kb_per_cycle = 100.0;
    assert_eq!(churn::run(&entry, &id, &loose).unwrap()["ok"], true);
}
