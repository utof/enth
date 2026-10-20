//! How audible generator v1's targets are (lane RG; a dev measurement, not a gate). Report 01 measured its prototype
//! this way: render each target's phrase, then count the silent ones (no integrated loudness, below −70 LUFS), the
//! too-quiet ones (below −40 LUFS) and the float peaks above 0 dBFS, per level, for musical = 100 and 0.
//!
//! This renders, so it is ignored in the gate, and it is the only RG file that touches `enth-render` (the generator
//! does not: `rg_render_free.rs`). Run it by hand, in release:
//!
//! ```text
//! cargo test --release -p enth-challenge --test rg_levels -- --ignored --nocapture
//! ```
//!
//! `RG_LEVELS_N` sets the targets per cell (default 200); `RG_LEVELS_SHOW=1` lists every silent or too-quiet target
//! with what was drawn.

#![allow(
    clippy::print_stdout,
    clippy::disallowed_methods,
    reason = "a hand-run measurement: reads its size from the environment and prints a table"
)]

use enth_analysis::loudness::LoudnessMeter;
use enth_challenge::random;
use enth_challenge::{Category, RandomSpec};
use enth_dsp::util::rng::derive;
use enth_render::{RenderSpec, render};

#[test]
#[ignore = "renders a few thousand targets; run by hand in release"]
fn audibility_by_level() {
    let n: u64 = std::env::var("RG_LEVELS_N")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(200);
    let mut meter = LoudnessMeter::new(48_000);
    println!("musical level n silent quiet(<-40LUFS) peak>0dBFS lufs_p50 forced_steps");
    for musical in [100u8, 0] {
        for (level, difficulty) in [(1, 0u8), (2, 25), (3, 50), (4, 75), (5, 100)] {
            let (mut silent, mut quiet, mut hot, mut forced) = (0, 0, 0, 0);
            let mut lufs = Vec::new();
            for i in 0..n {
                let spec = RandomSpec {
                    generator: random::GENERATOR_V1,
                    seed: derive(0x4C45_5645_4C53, i),
                    difficulty,
                    amounts: [None; Category::COUNT],
                    musical,
                    template: random::TEMPLATE_STARTER,
                };
                let g = random::target(&spec).unwrap();
                forced += g.steps.iter().filter(|s| s.forced).count();
                let out =
                    render(&g.patch, &g.phrase().performance(), &RenderSpec::default()).unwrap();
                let lvl = meter.level(&out.mid());
                if std::env::var("RG_LEVELS_SHOW").is_ok() && lvl.lufs_i.is_none_or(|l| l < -40.0) {
                    println!(
                        "  {} quiet: seed {:x} d {difficulty} m {musical} phrase {:?} lufs {:?}",
                        i, spec.seed, g.phrase, lvl.lufs_i
                    );
                    for r in &g.randomized {
                        if let random::Drawn::Param(a) = &r.drawn {
                            println!("    {a} = {:?}", g.patch.value(a));
                        } else {
                            println!("    {:?}", r.drawn);
                        }
                    }
                }
                match lvl.lufs_i {
                    None => silent += 1,
                    Some(l) => {
                        if l < -40.0 {
                            quiet += 1;
                        }
                        lufs.push(l);
                    }
                }
                if lvl.peak_dbfs > 0.0 {
                    hot += 1;
                }
            }
            lufs.sort_by(f64::total_cmp);
            let p50 = lufs.get(lufs.len() / 2).copied().unwrap_or(f64::NAN);
            println!("{musical} {level} {n} {silent} {quiet} {hot} {p50:.1} {forced}");
        }
    }
}
