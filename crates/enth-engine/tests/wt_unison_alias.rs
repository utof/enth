//! Aliasing of `wt` unison (S2a lane UNI): the copies alias no worse than a single `wt`, measured with
//! `enth_dsp::alias` (report 02's measure: the power outside the harmonics over the power on them, 32,768 samples).
//!
//! Two measurements, because a detuned unison has no single fundamental:
//!
//! - **Detune 0**: every copy plays the pitch of the key, only their start phases differ, so the sum is periodic at the
//!   key's frequency and the standard measure applies to the whole render. Compared with a single `wt`'s, cell by cell,
//!   across the keyboard and the tables, and held to the bound `tests/wt.rs` records for one (-90 dB).
//! - **Detune 100 cents**: the copies sit at the pitches of the key ± 100 cents (the outermost, the worst case for the
//!   mip-level choice: the highest copy has the fewest partials under the limit and the most aliasing risk). Each copy
//!   is a plain voice at its own pitch to the last bit (the lanes' tests in `enth-dsp`), so its aliasing is that of a
//!   single `wt` at that pitch; a single `wt` at the key ± 100 cents is measured against the same bound. The sum of
//!   copies at different frequencies has its aliasing powers add and its signal powers add, so the ratio of the sum
//!   lies between the copies' ratios and cannot exceed the worst of them.
//!
//! The tables print with `cargo test -p enth-engine --test wt_unison_alias -- --nocapture`; `docs/dsp/wt.md` records
//! them.
#![allow(
    clippy::print_stderr,
    reason = "the sweeps print the tables docs/dsp/wt.md records"
)]

mod common;

use std::fmt::Write as _;

use common::{SEED, SR, drive, engine, events, id, note};
use enth_dsp::alias::{FFT_LEN, Measure, alias_db, midi_hz};
use enth_patch::catalogue::{AdsrParam, WtParam, WtTable};
use enth_patch::{ModuleInstance, ModuleKind, Patch};

const SRATE: f64 = SR as f64;
const FRAMES: usize = FFT_LEN + 8_192;

/// The bound `tests/wt.rs` records for every table at every key (`BOUND_TABLES_DB`).
const BOUND_DB: f64 = -90.0;

fn lone(set: &[(WtParam, f32)]) -> Patch {
    let mut p = Patch::init();
    p.generators.clear();
    let env = &mut p.modulators[0].values;
    env[AdsrParam::Attack.index()] = 0.0;
    env[AdsrParam::Decay.index()] = 0.0;
    env[AdsrParam::Sustain.index()] = 100.0;
    env[AdsrParam::Release.index()] = 0.0;
    let mut m = ModuleInstance::new(id("wt1"), ModuleKind::Wt);
    for &(param, value) in set {
        m.values[param.index()] = value;
    }
    p.generators.push(m);
    p
}

fn alias_of(patch: &Patch, key: u8, f0: f64) -> f64 {
    let mut e = engine(patch, SEED);
    let (left, _) = drive(
        &mut e,
        &events(&[note(key, 0, FRAMES as u64)]),
        FRAMES,
        512,
        |_, _, _| {},
    );
    alias_db(&left, f0, SRATE, &Measure::REPORT_02)
}

const KEYS: [u8; 6] = [24, 48, 72, 84, 96, 105];

/// The (table, position) pairs measured: the saw-like `basic` position, a square, and one position of each other
/// table.
fn tables() -> Vec<(&'static str, WtTable, f32)> {
    vec![
        ("basic saw-like", WtTable::Basic, 66.0),
        ("basic square", WtTable::Basic, 100.0),
        ("fm", WtTable::Fm, 50.0),
        ("fm_octave", WtTable::FmOctave, 50.0),
        ("formant", WtTable::Formant, 50.0),
        ("metal", WtTable::Metal, 50.0),
    ]
}

fn header(keys: &[u8]) -> String {
    let mut s = format!("  {:<28}", "");
    for k in keys {
        write!(s, "{:>8}", format!("key {k}")).unwrap();
    }
    s.push('\n');
    s
}

#[test]
fn copies_at_one_pitch_alias_no_worse_than_a_single_wt() {
    let mut out = format!(
        "alias dB, detune 0: single wt, then unison 2 and 7\n{}",
        header(&KEYS)
    );
    let (mut worst_single, mut worst_unison) = (f64::NEG_INFINITY, f64::NEG_INFINITY);
    for (name, table, pos) in tables() {
        let base = [
            (WtParam::Table, table.index() as f32),
            (WtParam::Position, pos),
        ];
        let mut rows: Vec<(String, Vec<f64>)> = Vec::new();
        for count in [1u32, 2, 7] {
            let mut set = base.to_vec();
            set.push((WtParam::Unison, count as f32));
            set.push((WtParam::Detune, 0.0));
            let p = lone(&set);
            let values: Vec<f64> = KEYS
                .iter()
                .map(|&k| alias_of(&p, k, midi_hz(f64::from(k))))
                .collect();
            rows.push((
                if count == 1 {
                    format!("{name}, single")
                } else {
                    format!("{name}, unison {count}")
                },
                values,
            ));
        }
        for (label, values) in &rows {
            write!(out, "  {label:<28}").unwrap();
            for v in values {
                write!(out, "{v:>8.1}").unwrap();
            }
            out.push('\n');
        }
        let single = &rows[0].1;
        for (label, values) in &rows[1..] {
            for ((&k, &v), &s) in KEYS.iter().zip(values).zip(single) {
                worst_unison = worst_unison.max(v);
                assert!(
                    v < BOUND_DB,
                    "{label} at key {k}: {v:.1} dB is above the single wt's bound {BOUND_DB}\n{out}"
                );
                // Both are at the floor of what the measure resolves; "no worse" means within 3 dB of the single's
                // cell or under its bound by a clear margin.
                assert!(
                    v < s + 3.0 || v < BOUND_DB - 10.0,
                    "{label} at key {k}: {v:.1} dB against the single wt's {s:.1}\n{out}"
                );
            }
        }
        worst_single = worst_single.max(single.iter().copied().fold(f64::NEG_INFINITY, f64::max));
    }
    writeln!(
        out,
        "  worst: single {worst_single:.1} dB, unison {worst_unison:.1} dB (bound {BOUND_DB})"
    )
    .unwrap();
    eprintln!("{out}");
}

#[test]
fn the_outermost_detuned_copies_alias_inside_the_single_wts_bound() {
    // A single wt at the key's pitch ± 100 cents is what the outermost copies are, bit for bit in the lanes.
    let mut out = format!(
        "alias dB, single wt at the key -100 / 0 / +100 cents\n{}",
        header(&KEYS)
    );
    let mut worst = f64::NEG_INFINITY;
    for (name, table, pos) in tables() {
        for cents in [-100.0f32, 0.0, 100.0] {
            let p = lone(&[
                (WtParam::Table, table.index() as f32),
                (WtParam::Position, pos),
                (WtParam::Fine, cents),
            ]);
            write!(out, "  {:<28}", format!("{name} {cents:+.0}")).unwrap();
            for &k in &KEYS {
                let f0 = midi_hz(f64::from(k) + f64::from(cents) / 100.0);
                let v = alias_of(&p, k, f0);
                worst = worst.max(v);
                write!(out, "{v:>8.1}").unwrap();
                assert!(
                    v < BOUND_DB,
                    "{name} {cents:+} cents at key {k}: {v:.1} dB\n{out}"
                );
            }
            out.push('\n');
        }
    }
    writeln!(out, "  worst: {worst:.1} dB (bound {BOUND_DB})").unwrap();
    eprintln!("{out}");
}
