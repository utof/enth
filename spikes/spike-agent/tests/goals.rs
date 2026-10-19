//! `goals.toml`: well-formed (always) and reachable (on request: `cargo test --release -p spike-agent --test goals
//! -- --ignored --nocapture`). Reachable means a patch exists in the toy synth whose descriptors pass the whole
//! expectation list, found by random search plus hill climbing; it says nothing about how an agent would find one.

#![allow(
    clippy::print_stdout,
    clippy::needless_range_loop,
    reason = "the reachability search reports with println and indexes several arrays by one counter"
)]

use serde::Deserialize;
use spike_agent::commands::describe_patch;
use spike_agent::descriptors::{Describer, Want};
use spike_agent::expect::{Expect, parse_list, want_for};
use spike_agent::patch::{CONT, N_CONT, Patch, Wave};
use spike_agent::synth::{SAMPLE_RATE, measure};

#[derive(Deserialize)]
struct Goals {
    goal: Vec<Goal>,
}

#[derive(Deserialize)]
struct Goal {
    id: String,
    difficulty: u32,
    text: String,
    perf: String,
    expect: String,
    rationale: String,
}

fn goals() -> Vec<Goal> {
    let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/goals.toml"))
        .expect("goals.toml exists");
    toml::from_str::<Goals>(&text)
        .expect("goals.toml parses")
        .goal
}

fn start_patch(goal: &Goal) -> Patch {
    let mut p = Patch::default();
    for item in goal.perf.split(',') {
        let (k, v) = item.split_once('=').expect("perf is key=value");
        p.set(&format!("perf.{}", k.trim()), v.trim())
            .expect("perf value is valid");
    }
    p.validate().expect("perf is consistent");
    p
}

#[test]
fn goals_file_is_well_formed() {
    let goals = goals();
    assert_eq!(goals.len(), 5, "five starter goals");
    let mut ids: Vec<&str> = goals.iter().map(|g| g.id.as_str()).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), 5, "ids are unique");
    for (i, g) in goals.iter().enumerate() {
        assert_eq!(
            g.difficulty as usize,
            i + 1,
            "{}: difficulty rises with position",
            g.id
        );
        assert!(!g.text.trim().is_empty() && !g.rationale.trim().is_empty());
        let expects = parse_list(&g.expect).unwrap_or_else(|e| panic!("{}: {e}", g.id));
        assert!(
            expects.len() >= 4,
            "{}: a designer's list has several items",
            g.id
        );
        let p = start_patch(g);
        // the clip must be long enough to measure the tail the list asks about
        assert!(p.perf.dur_s >= p.perf.hold_s);
    }
}

#[test]
fn a_goal_is_not_met_by_the_neutral_patch() {
    for g in goals() {
        let expects = parse_list(&g.expect).unwrap();
        let d = describe_patch(&start_patch(&g), want_for(&expects)).unwrap();
        let passed = expects.iter().filter(|e| e.eval(&d).pass).count();
        assert!(
            passed < expects.len(),
            "{}: the starting patch already satisfies the goal",
            g.id
        );
    }
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> f64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 11) as f64 / (1u64 << 53) as f64
    }

    fn normal(&mut self) -> f64 {
        let (a, b) = (self.next().max(1e-12), self.next());
        (-2.0 * a.ln()).sqrt() * (2.0 * std::f64::consts::PI * b).cos()
    }
}

fn score(p: &Patch, expects: &[Expect], describer: &mut Describer, want: Want) -> (f64, Vec<bool>) {
    let d = measure(p, describer, want).unwrap();
    let mut total = 0.0;
    let mut passes = Vec::new();
    for e in expects {
        let ev = e.eval(&d);
        passes.push(ev.pass);
        total += if ev.pass {
            0.0
        } else {
            e.shortfall(&d).min(20.0) + 0.05
        };
    }
    (total, passes)
}

fn random_patch(rng: &mut Rng, start: &Patch) -> Patch {
    let mut p = start.clone();
    for i in 0..N_CONT {
        p.v[i] = CONT[i].from_u(rng.next());
    }
    p.wave = Wave::ALL[(rng.next() * 3.0) as usize % 3];
    p
}

#[test]
#[ignore = "slow search (minutes in release); run on request"]
fn every_goal_is_reachable() {
    let witness_dir = std::env::var("AGENT_PROBE_WITNESS_DIR").ok();
    let mut describer = Describer::new(SAMPLE_RATE);
    for g in goals() {
        let expects = parse_list(&g.expect).unwrap();
        let want = want_for(&expects);
        let start = start_patch(&g);
        let mut rng = Rng(0x9e37_79b9_7f4a_7c15 ^ u64::from(g.difficulty));
        let n_random = 500;
        let mut pool: Vec<(f64, Patch)> = Vec::new();
        let mut item_pass = vec![0usize; expects.len()];
        let mut all_pass = 0;
        for _ in 0..n_random {
            let p = random_patch(&mut rng, &start);
            let (s, passes) = score(&p, &expects, &mut describer, want);
            for (c, ok) in item_pass.iter_mut().zip(&passes) {
                *c += usize::from(*ok);
            }
            all_pass += usize::from(s == 0.0);
            pool.push((s, p));
        }
        pool.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut best = pool[0].clone();
        for (s0, p0) in pool.iter().take(6) {
            let (mut s, mut p) = (*s0, p0.clone());
            let mut sigma = 0.15;
            for it in 0..120 {
                if s == 0.0 {
                    break;
                }
                let mut q = p.clone();
                for i in 0..N_CONT {
                    if rng.next() < 0.5 {
                        q = q.with_u(i, CONT[i].to_u(q.v[i]) + sigma * rng.normal());
                    }
                }
                if rng.next() < 0.05 {
                    q.wave = Wave::ALL[(rng.next() * 3.0) as usize % 3];
                }
                let (sq, _) = score(&q, &expects, &mut describer, want);
                if sq <= s {
                    (s, p) = (sq, q);
                }
                if it % 40 == 39 {
                    sigma *= 0.5;
                }
            }
            if s < best.0 {
                best = (s, p);
            }
            if best.0 == 0.0 {
                break;
            }
        }
        let rates: Vec<String> = expects
            .iter()
            .zip(&item_pass)
            .map(|(e, c)| format!("{} {:.0}%", e.text, 100.0 * *c as f64 / f64::from(n_random)))
            .collect();
        println!(
            "{}: random all-pass {}/{}; best score {:.3}; item pass rates among random: {}",
            g.id,
            all_pass,
            n_random,
            best.0,
            rates.join("; ")
        );
        if let Some(dir) = &witness_dir {
            std::fs::create_dir_all(dir).unwrap();
            let path = format!("{dir}/{}.json", g.id);
            std::fs::write(
                &path,
                serde_json::to_string_pretty(&best.1.to_json()).unwrap(),
            )
            .unwrap();
        }
        assert_eq!(
            best.0, 0.0,
            "{}: no patch found that passes the list: {}",
            g.id, g.text
        );
    }
}
