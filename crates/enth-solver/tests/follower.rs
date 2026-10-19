//! The mechanical follower of report 03 (`#[ignore]`: a sweep with recorded results, not a gate).
//!
//! Start from a neutral template with each probe goal's performance, apply one hint per step, re-check, and stop
//! at a pass or after 12 steps. On the toy synth of P1a research every goal passed in one hint step. Results on
//! the real engine are recorded in `docs/dsp/hints.md`; rerun after the module lanes land:
//!
//! ```text
//! cargo test --release -p enth-solver --test follower -- --ignored --nocapture
//! ```

mod common;

use common::{items, two_osc, with_filter};
use enth_render::{Performance, RenderSpec};
use enth_solver::{HintCfg, Serial, hint};

struct Goal {
    id: &'static str,
    key: u8,
    hold_s: f64,
    dur_s: f64,
    expect: &'static str,
}

const GOALS: [Goal; 5] = [
    Goal {
        id: "g1-pluck",
        key: 57,
        hold_s: 1.5,
        dur_s: 2.0,
        expect: "attack<10ms, brightness>2kHz, sustain<-30dB, duration<800ms, pitch=A3±30ct",
    },
    Goal {
        id: "g2-pad",
        key: 48,
        hold_s: 3.5,
        dur_s: 6.0,
        expect: "attack>400ms, brightness<700Hz, sustain>-4dB, release>800ms, duration>4000ms, pitch=C3±30ct",
    },
    Goal {
        id: "g3-buzzy-bass",
        key: 36,
        hold_s: 1.5,
        dur_s: 2.0,
        expect: "pitch=C2±30ct, attack<15ms, brightness>1.2kHz, brightness<2.5kHz, sustain>-6dB, duration>1200ms, \
                 release<300ms",
    },
    Goal {
        id: "g4-bell",
        key: 72,
        hold_s: 3.0,
        dur_s: 5.0,
        expect: "pitch=C5±40ct, attack>8ms, attack<40ms, brightness>800Hz, brightness<3kHz, decay>600ms, \
                 sustain<-35dB, duration>2000ms, beating>2Hz, beating<8Hz",
    },
    Goal {
        id: "g5-supersaw-lead",
        key: 69,
        hold_s: 2.0,
        dur_s: 3.0,
        expect: "pitch=A4±40ct, attack<20ms, brightness>3kHz, sustain>-4dB, beating>3Hz, beating<9Hz, release<400ms, \
                 duration>2000ms",
    },
];

#[test]
#[ignore = "sweep: results are recorded in docs/dsp/hints.md"]
#[allow(clippy::print_stdout, reason = "a sweep prints its table")]
fn follower_on_the_probe_goals() {
    let cfg = HintCfg::default();
    for g in &GOALS {
        let perf = Performance::note(g.key, 0.8, g.hold_s, g.dur_s);
        let list = items(g.expect);
        let mut patch = with_filter(two_osc(0.0, 0.0), 20_000.0);
        let (mut renders, mut steps, mut pass) = (0, 0, false);
        let mut last = String::new();
        for step in 0..12 {
            let r = hint(
                &patch,
                &perf,
                &RenderSpec::default(),
                &list,
                &mut Serial::new(),
                &cfg,
            )
            .unwrap();
            renders += r.stats.renders;
            if r.before.iter().all(|x| x.pass) {
                pass = true;
                steps = step;
                break;
            }
            let blocked: Vec<String> = r
                .blocked
                .iter()
                .map(|b| {
                    let at: Vec<String> = b.at_bound.iter().map(ToString::to_string).collect();
                    format!("{} ({} {at:?})", b.expect, b.reason.code())
                })
                .collect();
            last = format!("blocked {blocked:?}");
            let Some(fix) = r.fix else {
                steps = step;
                break;
            };
            let moves: Vec<String> = fix
                .moves
                .iter()
                .map(|m| format!("{}={}", m.addr, m.to_text()))
                .collect();
            last = format!("{last}; last fix {moves:?}");
            patch = fix.patch;
            steps = step + 1;
            if fix.pass_all {
                pass = true;
                break;
            }
        }
        println!(
            "{:18} {} after {steps} hint steps, {renders} renders | {last}",
            g.id,
            if pass { "PASS" } else { "fail" }
        );
    }
}
