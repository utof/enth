//! The hint seam, at the library level: the CLI's evaluator is the solver's `measure` (so a hint's `before` is
//! the CLI's own reading, to the bit), and a hint does not depend on the number of rayon workers.
#![allow(
    clippy::disallowed_methods,
    reason = "tests read and write scratch files and build thread pools"
)]

mod common;

use common::{Scratch, json, run};
use enth_analysis::descriptors::Want;
use enth_analysis::parse_list;
use enth_cli::eval::{Evaluator, Parallel};
use enth_cli::perf::Perf;
use enth_patch::file;
use enth_render::RenderSpec;
use enth_solver::{HintCfg, hint};

/// The pluck, in the loop's own form, and a list with two failing items.
fn pluck() -> (enth_patch::Patch, Perf) {
    let s = Scratch::new("hints_pluck");
    let p = s.new_patch("pluck_1osc", "p.patch.json");
    let text = std::fs::read_to_string(p).unwrap();
    let patch = file::load(&text).unwrap().patch;
    let perf = Perf::from_args(
        &enth_cli::perf::PerfArgs {
            perf: Some("note=A3,vel=100,hold=1s,dur=2s".to_owned()),
            ..Default::default()
        },
        "",
    )
    .unwrap()
    .unwrap();
    (patch, perf)
}

const LIST: &str = "pitch.note ~ A3, attack < 30ms, brightness < 900Hz, duration > 900ms";

#[test]
fn a_hints_before_is_what_check_reads() {
    let (patch, perf) = pluck();
    let items = parse_list(LIST).unwrap();
    let ev = Evaluator::new(&perf, enth_render::DEFAULT_SEED, Want::ALL);
    let main = ev
        .describe_jobs(&[(&patch, ev.seed())])
        .pop()
        .unwrap()
        .unwrap();
    let report = enth_analysis::check(&items, &main, &[]);
    let cfg = HintCfg {
        max_renders: 1,
        ..HintCfg::default()
    };
    let spec = RenderSpec::new(ev.sample_rate(), ev.seed());
    let res = hint(&patch, ev.performance(), &spec, &items, &mut Parallel, &cfg).unwrap();
    assert_eq!(res.before.len(), report.results.len());
    for (b, r) in res.before.iter().zip(&report.results) {
        assert_eq!(b, r, "{}", r.expect);
    }

    // And the command prints the same readings (to its four digits).
    let s = Scratch::new("hints_check");
    let p = s.new_patch("pluck_1osc", "p.patch.json");
    let o = run(&[
        "check",
        &p,
        "--expect",
        LIST,
        "--perf",
        "note=A3,vel=100,hold=1s,dur=2s",
        "--no-fix",
    ]);
    let v = json(&o);
    for (b, shown) in res.before.iter().zip(v["results"].as_array().unwrap()) {
        let got = shown["got"].as_f64().unwrap();
        let want = b.got.unwrap();
        assert!(
            (got - want).abs() <= want.abs() * 1e-3,
            "{}: {got} vs {want}",
            b.expect
        );
    }
}

#[test]
fn a_hint_is_the_same_for_any_number_of_workers() {
    let (patch, perf) = pluck();
    let items = parse_list(LIST).unwrap();
    let ev = Evaluator::new(&perf, enth_render::DEFAULT_SEED, Want::ALL);
    let spec = RenderSpec::new(ev.sample_rate(), ev.seed());
    let run_with = |threads: usize| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap()
            .install(|| {
                hint(
                    &patch,
                    ev.performance(),
                    &spec,
                    &items,
                    &mut Parallel,
                    &HintCfg::default(),
                )
                .unwrap()
            })
    };
    let one = run_with(1);
    let many = run_with(5);
    assert!(
        one.fix.is_some(),
        "the list is reachable: {:?}",
        one.blocked
    );
    assert_eq!(one, many);
}
