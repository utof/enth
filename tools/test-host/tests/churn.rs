//! `churn` counts the threads, descriptors and memory of the whole process, so this file holds one test that
//! runs its scenarios one after another: nothing else may start threads or open files meanwhile.

mod common;

use common::{entry, state_for};
use enth_test_host::churn::{self, Inject, Options};
use enth_test_host::procfs;
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

    // The measure itself (lane CHURN): a thread that has been joined is never counted, though the kernel may list it for
    // a while longer. `pthread_join` returns when the kernel clears the thread's tid word, early in `do_exit`; the
    // thread leaves `/proc/self/task` only when it is reaped, later, and at load 40 that is often a few milliseconds.
    // Every audio thread of the cycles below is such a thread: the plain count read one too many (or, at the first
    // sample, one too few) in 15 runs of 80 at load 41, as 30 or 32 where 31 is expected and as a failed thread check
    // of a plugin that leaks only memory. A listed thread that is exiting (`PF_EXITING`) has run its last instruction,
    // so the count leaves it out; a leak is parked and always counted (below, exactly).
    let base = procfs::threads().live;
    for _ in 0..2000 {
        std::thread::spawn(|| {}).join().unwrap();
        assert_eq!(
            procfs::threads().live,
            base,
            "a joined thread was counted as live"
        );
    }

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

    // A plugin that leaves a thread per instance (thread stacks also show in memory, so only threads are asserted). The
    // growth is exact: one parked thread for each of the 31 samples after the first one past the warm-up (40 cycles, 8
    // of warm-up), the audio threads not counted whether or not the kernel still lists them.
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

    // A leak of 2 KB per cycle fails as well: the trim before each sample gives back free pages, never live ones (lane
    // LK; with the real plugin and no trim, a 2 KB leak hid in the pages freed instances left resident).
    let mut small = options(None);
    small.inject = Some(Inject::LeakKb(2));
    let report = churn::run(&entry, &id, &small).unwrap();
    assert_eq!(report["trim_before_sample"], true);
    assert!(!checks(&report).0, "2 KB per cycle passed: {report}");

    // And a threshold set above the fault lets it through, so the limits are what decide.
    let mut loose = options(Some(r#"{"leak":"mem:32"}"#));
    loose.max_kb_per_cycle = 100.0;
    assert_eq!(churn::run(&entry, &id, &loose).unwrap()["ok"], true);
}
