//! Lane SVP (S2a): the stereo voice path against the goldens; lane UNI added the cases that run it.
//!
//! Every patch case in `cases/` (the harness's own sine aside) renders through `enth_render` as `cargo xtask golden`
//! does, and with the engine's stereo flag forced on.
//!
//! - **A case without a stereo generator** (every case but `wt-unison-*`) runs the mono path under `Auto` (the flag is
//!   clear), and **both channels of the forced render must equal it bit for bit**: stereo with zero spread is mono.
//!   The hashes in `expected/` are the mono path's, so this also ties the stereo path to them on all six targets,
//!   without a second set of files.
//! - **A case with a unison `wt`** (flag set by the patch alone) runs the stereo path under `Auto`: forcing the flag
//!   changes nothing, and `StereoMode::Off` (the mono reference: the copies' mono mix) must give two equal channels
//!   (when the voice pan is centred). `wt-unison-reese` has spread 0 throughout, so its stereo render equals the mono
//!   reference on both channels bit for bit; the cases with a spread differ between the channels.

use enth_golden::patch::patch_from_file;
use enth_golden::{Case, Source, root};
use enth_patch::catalogue::VoiceParam;
use enth_render::{
    Automation, Control, Controller, DEFAULT_BLOCK, Note, Performance, RenderSpec, StereoMode,
    render_with_stereo,
};

fn bits(x: &[f32]) -> Vec<u32> {
    x.iter().map(|v| v.to_bits()).collect()
}

#[test]
fn every_golden_patch_runs_mono_or_the_unison_stereo_path_and_forcing_the_flag_changes_no_bit() {
    let dir = root().join("cases");
    let mut names: Vec<_> = std::fs::read_dir(&dir)
        .expect("cases/")
        .map(|e| e.expect("entry").path())
        .filter(|p| p.extension().is_some_and(|e| e == "toml"))
        .collect();
    names.sort();
    let (mut patches, mut cases, mut flagged) = (0, 0, 0);
    for path in names {
        cases += 1;
        let name = path.file_stem().unwrap().to_string_lossy().into_owned();
        let case: Case = toml::from_str(&std::fs::read_to_string(&path).unwrap())
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        let Source::Patch {
            seed,
            notes,
            automation,
            controls,
            patch,
        } = &case.source
        else {
            continue; // the harness's own sine: no engine
        };
        patches += 1;
        let patch = patch_from_file(patch).unwrap_or_else(|e| panic!("{name}: {e}"));
        let perf = Performance {
            notes: notes
                .iter()
                .map(|n| Note::new(n.key, n.velocity, n.on_s, n.off_s))
                .collect(),
            automation: automation
                .iter()
                .map(|a| Automation {
                    time_s: a.time_s,
                    addr: a.addr.parse().unwrap(),
                    value: a.value,
                })
                .collect(),
            controls: controls
                .iter()
                .map(|c| Control {
                    time_s: c.time_s,
                    controller: c.source.parse::<Controller>().unwrap(),
                    value: c.value,
                })
                .collect(),
            duration_s: case.frames() as f64 / f64::from(case.sample_rate),
            ..Performance::default()
        };
        let spec = RenderSpec::new(case.sample_rate, seed.unwrap_or(enth_render::DEFAULT_SEED));
        let auto = render_with_stereo(&patch, &perf, &spec, DEFAULT_BLOCK, StereoMode::Auto)
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        let forced = render_with_stereo(&patch, &perf, &spec, DEFAULT_BLOCK, StereoMode::Forced)
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(forced.stats.stereo, "{name}: the flag was forced on");
        assert_eq!(auto.frames(), case.frames(), "{name}");
        assert_eq!(
            auto.stats.stereo,
            name.starts_with("wt-unison-"),
            "{name}: the flag comes from the patch (a unison wt) and from nothing else"
        );
        if auto.stats.stereo {
            flagged += 1;
            // The flag was already set: forcing it changes nothing.
            assert_eq!(bits(&forced.left), bits(&auto.left), "{name}: left");
            assert_eq!(bits(&forced.right), bits(&auto.right), "{name}: right");
            // The mono reference: the same patch on the mono path. Its channels are one signal.
            let off = render_with_stereo(&patch, &perf, &spec, DEFAULT_BLOCK, StereoMode::Off)
                .unwrap_or_else(|e| panic!("{name}: {e}"));
            assert!(!off.stats.stereo, "{name}: Off runs the mono path");
            if patch.voice.values[VoiceParam::Pan.index()] == 0.0 {
                assert_eq!(bits(&off.left), bits(&off.right), "{name}: mono reference");
            }
            if name == "wt-unison-reese" {
                // Spread 0 throughout: stereo equals mono on both channels.
                assert_eq!(bits(&auto.left), bits(&off.left), "{name}: left = mono");
                assert_eq!(bits(&auto.right), bits(&off.left), "{name}: right = mono");
            } else {
                assert_ne!(
                    bits(&auto.left),
                    bits(&auto.right),
                    "{name}: a spread widens"
                );
            }
        } else {
            assert_eq!(bits(&forced.left), bits(&auto.left), "{name}: left");
            assert_eq!(bits(&forced.right), bits(&auto.right), "{name}: right");
        }
    }
    assert!(cases >= 73, "{cases} case files");
    assert!(patches >= 70, "{patches} patch cases");
    assert_eq!(flagged, 7, "the wt-unison-* cases");
}
