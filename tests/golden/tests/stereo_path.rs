//! Lane SVP (S2a): the stereo voice path against the goldens.
//!
//! Every patch case in `cases/` (73 files, the harness's own sine aside) renders twice through `enth_render`: as
//! `cargo xtask golden` does, and with the engine's stereo flag forced on. The first must run the mono path (the flag
//! is clear for every golden patch), the second the stereo one, and **both channels of the second must equal the
//! first's bit for bit**: stereo with zero spread is mono. The hashes in `expected/` are the mono path's, so this
//! also ties the stereo path to them on all six targets, without a second set of files.

use enth_golden::patch::patch_from_file;
use enth_golden::{Case, Source, root};
use enth_render::{
    Automation, Control, Controller, DEFAULT_BLOCK, Note, Performance, RenderSpec, StereoMode,
    render_with_stereo,
};

fn bits(x: &[f32]) -> Vec<u32> {
    x.iter().map(|v| v.to_bits()).collect()
}

#[test]
fn every_golden_patch_runs_mono_and_equals_the_stereo_path_on_both_channels() {
    let dir = root().join("cases");
    let mut names: Vec<_> = std::fs::read_dir(&dir)
        .expect("cases/")
        .map(|e| e.expect("entry").path())
        .filter(|p| p.extension().is_some_and(|e| e == "toml"))
        .collect();
    names.sort();
    let (mut patches, mut cases) = (0, 0);
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
        let mono = render_with_stereo(&patch, &perf, &spec, DEFAULT_BLOCK, StereoMode::Auto)
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        let stereo = render_with_stereo(&patch, &perf, &spec, DEFAULT_BLOCK, StereoMode::Forced)
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(
            !mono.stats.stereo,
            "{name}: the flag is clear for a golden patch"
        );
        assert!(stereo.stats.stereo, "{name}: the flag was forced on");
        assert_eq!(mono.frames(), case.frames(), "{name}");
        assert_eq!(bits(&stereo.left), bits(&mono.left), "{name}: left");
        assert_eq!(bits(&stereo.right), bits(&mono.right), "{name}: right");
    }
    assert!(cases >= 73, "{cases} case files");
    assert!(patches >= 70, "{patches} patch cases");
}
