//! Descriptor audit: sweeps one parameter at a time on the toy synth and prints what the descriptors read next to
//! what the parameter value predicts. Run on request:
//! `cargo test --release -p spike-agent --test audit -- --ignored --nocapture`
//! The table in docs/spikes/agent-probe.md comes from this.

#![allow(clippy::print_stdout, reason = "the audit prints its table")]

use spike_agent::commands::describe_patch;
use spike_agent::descriptors::{Descriptors, Want};
use spike_agent::patch::Patch;
use std::fmt::Write as _;

fn measure(perf: &str, sets: &[&str]) -> Descriptors {
    let mut p = Patch::default();
    for item in perf.split(',') {
        let (k, v) = item.split_once('=').unwrap();
        p.set(&format!("perf.{k}"), v).unwrap();
    }
    let sets: Vec<String> = sets.iter().map(|s| (*s).to_string()).collect();
    let (p, _) = p.apply_sets(&sets).unwrap();
    describe_patch(&p, Want::ALL).unwrap()
}

fn fmt(x: Option<f64>) -> String {
    x.map_or("-".into(), |v| format!("{v:.1}"))
}

#[test]
#[ignore = "prints the audit table; run on request"]
fn audit() {
    let long = "note=A3,dur=6s,hold=2s";
    println!("\nattack: amp.attack (ms) -> attack_ms   [ramp theory: 0.88 x]");
    for a in [1, 2, 5, 10, 20, 50, 100, 200, 500, 1000] {
        let d = measure(long, &[&format!("amp.attack={a}ms")]);
        println!("  {a:>5} -> {}", fmt(d.attack_ms));
    }
    println!("\nsustain: amp.sustain (dB) -> sustain_db   (amp.decay 100ms, hold 2s)");
    for s in [0, -3, -6, -12, -20, -30, -45, -60] {
        let d = measure(long, &[&format!("amp.sustain={s}dB"), "amp.decay=100ms"]);
        println!("  {s:>5} -> {}", fmt(d.sustain_db));
    }
    println!("\ndecay: amp.decay (ms) -> decay_ms   (sustain off; theory 0.58 x)");
    for dcy in [20, 50, 100, 200, 500, 1000, 2000] {
        let d = measure(long, &[&format!("amp.decay={dcy}ms"), "amp.sustain=-60dB"]);
        println!("  {dcy:>5} -> {}", fmt(d.decay_ms));
    }
    println!("\nrelease: amp.release (ms) -> release_ms   (theory 1.15 x)");
    for r in [10, 50, 100, 200, 500, 1000, 2000] {
        let d = measure(long, &[&format!("amp.release={r}ms")]);
        println!(
            "  {r:>5} -> {}{}",
            fmt(d.release_ms),
            if d.release_truncated {
                " (lower bound)"
            } else {
                ""
            }
        );
    }
    println!("\nbrightness: filter.cutoff (Hz) -> centroid_hz, saw A3, no resonance/env");
    for c in [100, 200, 400, 800, 1600, 3200, 6400, 12800] {
        let d = measure(long, &[&format!("filter.cutoff={c}Hz")]);
        println!("  {c:>5} -> {}", fmt(d.centroid_hz));
    }
    println!("\nbrightness by wave at cutoff 4 kHz (A3): saw / square / triangle");
    for w in ["saw", "square", "triangle"] {
        let d = measure(long, &["filter.cutoff=4kHz", &format!("osc.wave={w}")]);
        println!("  {w:>8} -> {}", fmt(d.centroid_hz));
    }
    println!("\nbrightness: filter.resonance (Q) at cutoff 1 kHz -> centroid_hz");
    for q in ["0.5", "0.7", "2", "5", "10"] {
        let d = measure(
            long,
            &["filter.cutoff=1kHz", &format!("filter.resonance={q}")],
        );
        println!("  {q:>5} -> {}", fmt(d.centroid_hz));
    }
    println!("\npitch: note -> f0_hz / error in cents, by wave (detune 0)");
    for note in ["C2", "A2", "C3", "A3", "C4", "A4", "C5", "A5", "C6"] {
        let mut row = format!("  {note:>3}");
        for w in ["saw", "square", "triangle"] {
            let d = measure(
                &format!("note={note},dur=2s,hold=1.5s"),
                &[&format!("osc.wave={w}")],
            );
            let target = spike_agent::patch::note_or_hz(note).unwrap();
            let err = d.f0_hz.map(|f| 1200.0 * (f / target).log2());
            write!(row, "   {w}: {} ({} ct)", fmt(d.f0_hz), fmt(err)).unwrap();
        }
        println!("{row}");
    }
    println!("\npitch with detune (A3 saw): osc.detune (ct) -> error in cents");
    for det in [0, 5, 10, 20, 30, 40, 50] {
        let d = measure(
            "note=A3,dur=3s,hold=2.5s",
            &[&format!("osc.detune={det}ct"), "amp.sustain=0dB"],
        );
        let err = d.f0_hz.map(|f| 1200.0 * (f / 220.0).log2());
        println!("  {det:>3} -> {} ct", fmt(err));
    }
    println!(
        "\nbeating: osc.detune (ct) -> beating_hz / depth_db, [theory f0 x (2^(d/1200) - 1)], sustained, 3 s hold"
    );
    for note in ["C3", "A3", "A4"] {
        let hz = spike_agent::patch::note_or_hz(note).unwrap();
        for det in [5, 10, 20, 30, 50] {
            let perf = format!("note={note},dur=4s,hold=3s");
            let d = measure(
                &perf,
                &[
                    &format!("osc.detune={det}ct"),
                    "amp.sustain=0dB",
                    "filter.cutoff=4kHz",
                ],
            );
            let theory = hz * (f64::from(det) / 1200.0).exp2() - hz;
            println!(
                "  {note} {det:>3} ct -> {} Hz (depth {} dB)  theory {theory:.2}",
                fmt(d.beating_hz),
                fmt(d.beat_depth_db)
            );
        }
    }
    println!("\nwobble: sustain reading vs osc.detune with amp.sustain = -6 dB (A3 saw, 3 s hold)");
    for det in [0, 10, 25, 50] {
        let d = measure(
            "note=A3,dur=4s,hold=3s",
            &[
                &format!("osc.detune={det}ct"),
                "amp.sustain=-6dB",
                "amp.decay=100ms",
                "filter.cutoff=4kHz",
            ],
        );
        println!(
            "  {det:>3} ct -> sustain {} dB, attack {} ms",
            fmt(d.sustain_db),
            fmt(d.attack_ms)
        );
    }
    println!("\nnoise_ratio and loudness of the neutral patch, and of an unfiltered saw with Q 10");
    for sets in [&[][..], &["filter.cutoff=16kHz", "filter.resonance=10"][..]] {
        let d = measure("note=A3,dur=2s,hold=1.5s", sets);
        println!(
            "  {sets:?} -> noise {:?} lufs {} peak {} clipping {}",
            d.noise_ratio.map(|v| format!("{v:.5}")),
            fmt(d.lufs_i),
            fmt(Some(d.peak_dbfs)),
            d.clipping
        );
    }
}
