//! `enth render --bpm` (S2a, lane TMP): the tempo is a render input. A patch whose LFO is synced to the tempo
//! renders another WAV at another tempo, `--bpm 140` is the default, a patch with nothing synced does not care, and
//! a tempo outside 20 to 999 is a usage error. The flag's help is the one new snapshot.
#![allow(
    clippy::disallowed_methods,
    reason = "tests read and write scratch files"
)]

mod common;

use common::{Scratch, round_trip, run};
use enth_cli::schema::{ErrorOut, RenderOut};
use enth_patch::catalogue::{Beats, LfoParam};
use enth_patch::{ModuleId, ModuleInstance, ModuleKind, Patch, Route};
use insta::assert_snapshot;

/// The init saw with an LFO on its pitch; `sync` makes the LFO follow the tempo.
fn lfo_patch(sync: bool) -> Patch {
    let mut p = Patch::init();
    let mut lfo = ModuleInstance::new(ModuleId::new("lfo1").unwrap(), ModuleKind::Lfo);
    lfo.values[LfoParam::Sync.index()] = f32::from(u8::from(sync));
    lfo.values[LfoParam::Beats.index()] = Beats::EighthDotted.index() as f32;
    lfo.values[LfoParam::Rate.index()] = 6.0;
    p.modulators.push(lfo);
    p.routes.push(Route {
        src: "lfo1".parse().unwrap(),
        dst: "osc1.fine".parse().unwrap(),
        depth: 80.0,
    });
    p
}

/// Render `patch` with `extra` flags and return the WAV's bytes.
fn wav(s: &Scratch, name: &str, patch: &Patch, extra: &[&str]) -> Vec<u8> {
    let p = s.write(&format!("{name}.patch.json"), &patch.to_json());
    let out = s.path(&format!("{name}.wav"));
    let mut args = vec![
        "render",
        &p,
        "--perf",
        "note=A3,vel=90,hold=1s,dur=1.5s",
        "--out",
        &out,
    ];
    args.extend_from_slice(extra);
    let o = run(&args);
    assert_eq!(o.code, 0, "{}", o.stdout);
    let _: RenderOut = round_trip(&o);
    std::fs::read(&out).unwrap()
}

#[test]
fn a_synced_patch_follows_bpm_and_140_is_the_default() {
    let s = Scratch::new("render_bpm");
    let synced = lfo_patch(true);
    let default = wav(&s, "a", &synced, &[]);
    assert_eq!(wav(&s, "b", &synced, &["--bpm", "140"]), default);
    assert_eq!(wav(&s, "c", &synced, &["--bpm", "140bpm"]), default);
    let fast = wav(&s, "d", &synced, &["--bpm", "174"]);
    let slow = wav(&s, "e", &synced, &["--bpm", "90"]);
    assert_ne!(fast, default);
    assert_ne!(slow, default);
    assert_ne!(fast, slow);
    assert_eq!(
        wav(&s, "f", &synced, &["--bpm", "174"]),
        fast,
        "deterministic"
    );
}

#[test]
fn an_unsynced_patch_renders_the_same_bits_at_any_tempo() {
    let s = Scratch::new("render_bpm_unsynced");
    let unsynced = lfo_patch(false);
    let base = wav(&s, "a", &unsynced, &[]);
    for bpm in ["20", "90", "174", "999"] {
        assert_eq!(wav(&s, "b", &unsynced, &["--bpm", bpm]), base, "{bpm}");
    }
}

#[test]
fn a_tempo_outside_20_to_999_is_a_usage_error() {
    let s = Scratch::new("render_bpm_errors");
    let p = s.new_patch("init", "p.patch.json");
    for bpm in ["19.9", "1000", "0", "-120", "abc", "nan", "inf", ""] {
        // `--bpm=-120`: clap reads a bare `-120` as a flag.
        let o = run(&["render", &p, &format!("--bpm={bpm}")]);
        let e: ErrorOut = round_trip(&o);
        assert_eq!(
            (o.code, e.exit, e.error.as_str()),
            (2, 2, "bad_bpm"),
            "--bpm {bpm:?}: {}",
            o.stdout
        );
        assert!(e.hint.contains("20 to 999"), "{}", e.hint);
    }
    // No WAV was written for a refused tempo.
    assert!(!std::path::Path::new(&s.path("p.wav")).exists());
}

#[test]
fn the_flag_is_in_render_help() {
    let help = run(&["render", "--help"]).stdout;
    assert!(help.contains("--bpm <BPM>"), "{help}");
    assert!(help.contains("[default: 140]"), "{help}");
    // From the usage line on: the one-line description above it is `enth --help`'s too.
    let flags = help.split_once("Usage:").map_or(help.as_str(), |(_, f)| f);
    Scratch::new("render_help")
        .settings()
        .bind(|| assert_snapshot!("render_help", flags));
}
