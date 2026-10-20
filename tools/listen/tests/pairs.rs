//! The real-engine pairs tool. Nothing here plays audio: sessions use a recording mock, and `--dry-run` never calls
//! a player.
//!
//! - The plan (every pair and trial, drawn from the seed without audio) is pinned by its hash.
//! - A 12-pair design (two per distance bin) is made once and copied per test: its distances cover the bins, its
//!   clips are level-matched, and a session resumes after `q`. The full 40-pair design is
//!   `the_full_design_covers_the_planned_bins` (ignored: about 1.5 min on 3 threads; run it with `--ignored`).
//! - The report is checked on hand-made results with known answers, without audio.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use enth_challenge::calibrate::Curve;
use enth_challenge::loudness::integrated_lufs;
use enth_challenge::{ChallengeCode, RandomSpec, SpecPreset};
use enth_listen::files::{read_json, write_json};
use enth_listen::pairs::plan::{GENERATOR, PlannedPair, PlannedTrial, bin_counts};
use enth_listen::pairs::{
    BIN_LOWER, BIN_WEIGHTS, Config, DEFAULT_PAIRS, DEFAULT_SEED, FitData, Manifest, Measured,
    MoveKind, N_DIFFERENT, N_SAME, Pair, PairAnswer, PairRole, PairsAction, PairsArgs, ResultsFile,
    SCHEMA_FIT, SCHEMA_MANIFEST, SCHEMA_RESULTS, Side, analyze, bin_of, clips_dir, fit_rows,
    make_plan, manifest_path, rating_model, results_path, run,
};
use enth_listen::session::{MockPlayer, Scripted};
use enth_listen::wav::decode;

/// The hash of the default plan (seed 20261021, 40 test pairs). It moves only when the plan's rules, the generator v1
/// targets or the presets change, never with the engine's audio.
const PLAN_HASH_40: &str = "232f245171b51b20";

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("enth-listen-pairs-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn copy_dir(src: &Path, dst: &Path) {
    std::fs::create_dir_all(dst).unwrap();
    for e in std::fs::read_dir(src).unwrap() {
        let e = e.unwrap();
        let to = dst.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            copy_dir(&e.path(), &to);
        } else {
            std::fs::copy(e.path(), to).unwrap();
        }
    }
}

fn args(dir: &Path, pairs: usize) -> PairsArgs {
    PairsArgs {
        pairs,
        workers: 3,
        bootstrap: 200,
        player: "true".into(),
        ..PairsArgs::new(dir)
    }
}

/// The shared 12-pair design (made by the first test that needs it).
fn designed() -> &'static Path {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let d = scratch("design");
        let mut out = Vec::new();
        run(&args(&d, 12), &mut out, &mut Scripted::default(), None).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(
            text.contains("designing 12 test pairs and 4 checks")
                && text.contains("session 1: 0/16"),
            "{text}"
        );
        d
    })
}

fn manifest(dir: &Path) -> Manifest {
    let m: Manifest = read_json(&manifest_path(dir)).unwrap();
    assert_eq!(m.schema, SCHEMA_MANIFEST);
    m
}

#[test]
fn the_plan_is_deterministic_and_pinned() {
    let plan = make_plan(DEFAULT_SEED, DEFAULT_PAIRS).unwrap();
    assert_eq!(plan, make_plan(DEFAULT_SEED, DEFAULT_PAIRS).unwrap());
    assert_eq!(plan.hash(), PLAN_HASH_40, "the default plan moved");
    assert_ne!(
        make_plan(DEFAULT_SEED + 1, DEFAULT_PAIRS).unwrap().hash(),
        plan.hash()
    );
    assert!(make_plan(1, 0).is_err());

    let n = DEFAULT_PAIRS + N_SAME + N_DIFFERENT;
    assert_eq!(plan.pairs.len(), n);
    let test: Vec<&PlannedPair> = plan
        .pairs
        .iter()
        .filter(|p| p.role == PairRole::Test)
        .collect();
    assert_eq!(test.len(), DEFAULT_PAIRS);
    // planned bins as weighted, 8 targets per level, every move used
    for (b, w) in BIN_WEIGHTS.iter().enumerate() {
        assert_eq!(
            test.iter().filter(|p| p.planned_bin == Some(b)).count(),
            *w,
            "bin {b}"
        );
    }
    assert_eq!(bin_counts(DEFAULT_PAIRS), BIN_WEIGHTS.to_vec());
    for level in 1..=5u8 {
        assert_eq!(
            test.iter().filter(|p| p.level == level).count(),
            8,
            "level {level}"
        );
    }
    for kind in [MoveKind::All, MoveKind::One, MoveKind::Pitch] {
        let k = test.iter().filter(|p| p.kind == Some(kind)).count();
        assert!(k >= 10, "{kind:?}: {k}");
    }
    // targets: generator v1 at the level's preset, a 40-bit seed, so a short code
    for p in &plan.pairs {
        let preset = SpecPreset::by_id(p.level).unwrap();
        assert_eq!(
            p.target,
            preset.spec(GENERATOR, p.target.seed),
            "{}",
            p.index
        );
        assert!(ChallengeCode::short(p.target).is_some(), "{}", p.index);
        match p.role {
            PairRole::Test => assert_ne!(p.target_clip, p.answer_clip),
            PairRole::Same => assert_eq!(p.target_clip, p.answer_clip),
            PairRole::Different => {
                let other: RandomSpec = p.other.unwrap();
                assert_eq!(
                    other,
                    SpecPreset::by_id(5).unwrap().spec(GENERATOR, other.seed)
                );
            }
        }
        if matches!(p.kind, Some(MoveKind::One | MoveKind::Pitch)) {
            assert!(p.param.is_some(), "{}", p.index);
        }
    }
    let seeds: HashSet<u64> = test.iter().map(|p| p.target.seed).collect();
    assert_eq!(seeds.len(), DEFAULT_PAIRS, "distinct targets");
    // trials: each pair once per session, balanced A/B order and X, unique ids
    let mut ids = HashSet::new();
    for s in 1..=2u32 {
        let t: Vec<&PlannedTrial> = plan.trials.iter().filter(|t| t.session == s).collect();
        assert_eq!(t.len(), n);
        let pairs: HashSet<usize> = t.iter().map(|t| t.pair).collect();
        assert_eq!(pairs.len(), n);
        assert_eq!(t.iter().filter(|t| t.target_first).count(), n.div_ceil(2));
        assert_eq!(
            t.iter().filter(|t| t.x_is == Side::A).count(),
            n.div_ceil(2)
        );
        assert!(t.iter().enumerate().all(|(i, t)| t.n == i + 1));
        for t in t {
            assert!(ids.insert(t.id.clone()), "duplicate id {}", t.id);
        }
    }
    let orders: Vec<Vec<usize>> = (1..=2u32)
        .map(|s| {
            plan.trials
                .iter()
                .filter(|t| t.session == s)
                .map(|t| t.pair)
                .collect()
        })
        .collect();
    assert_ne!(orders[0], orders[1], "session 2 is reshuffled");
}

fn check_measured(m: &Manifest) {
    let curve = Curve::SOUND_V1;
    for p in &m.pairs {
        let s = &p.measured;
        assert!(
            (s.jnd - (s.z_metric + s.pitch_term + s.wave_term)).abs() <= 1e-9 * s.jnd.max(1.0),
            "pair {}: {s:?}",
            p.plan.index
        );
        assert!((s.percent - curve.percent(s.jnd)).abs() < 1e-9);
        assert_eq!(s.passes, s.percent >= 90.0);
        assert_eq!(p.bin, bin_of(s.jnd));
        match p.plan.role {
            PairRole::Same => assert!(s.jnd == 0.0 && s.percent == 100.0, "{s:?}"),
            // built different: another wave, every oscillator an octave apart, level 1 against level 5
            PairRole::Different => assert!(s.jnd >= 8.0, "pair {}: {s:?}", p.plan.index),
            PairRole::Test => {
                assert!(p.steps.is_some() && p.kind.is_some());
                assert!(p.ladder_jnd.len() >= 17);
                assert!(p.code.is_some());
            }
        }
    }
}

#[test]
fn distances_cover_the_planned_bins() {
    let m = manifest(designed());
    assert_eq!(m.config.pairs, 12);
    assert_eq!(m.scorer, "sound/1");
    assert_eq!(m.plan_hash, make_plan(DEFAULT_SEED, 12).unwrap().hash());
    check_measured(&m);
    let test: Vec<&Pair> = m
        .pairs
        .iter()
        .filter(|p| p.plan.role == PairRole::Test)
        .collect();
    for b in 0..BIN_LOWER.len() {
        assert!(test.iter().any(|p| p.bin == b), "no pair in bin {b}");
    }
    let landed = test
        .iter()
        .filter(|p| Some(p.bin) == p.plan.planned_bin)
        .count();
    assert!(landed >= 9, "{landed} of 12 landed in their planned bin");
    // both sides of the pass mark (0.99 JND)
    assert!(test.iter().filter(|p| p.measured.passes).count() >= 2);
    assert!(test.iter().filter(|p| !p.measured.passes).count() >= 6);
}

#[test]
#[ignore = "the full 40-pair design: about 1.5 min on 3 threads"]
fn the_full_design_covers_the_planned_bins() {
    let d = scratch("full");
    run(
        &args(&d, DEFAULT_PAIRS),
        &mut Vec::new(),
        &mut Scripted::default(),
        None,
    )
    .unwrap();
    let m = manifest(&d);
    assert_eq!(m.plan_hash, PLAN_HASH_40);
    check_measured(&m);
    let test: Vec<&Pair> = m
        .pairs
        .iter()
        .filter(|p| p.plan.role == PairRole::Test)
        .collect();
    for (b, w) in BIN_WEIGHTS.iter().enumerate() {
        let n = test.iter().filter(|p| p.bin == b).count();
        assert!(n >= w.div_ceil(2), "bin {b}: {n} pairs, planned {w}");
    }
    let landed = test
        .iter()
        .filter(|p| Some(p.bin) == p.plan.planned_bin)
        .count();
    assert!(landed >= 32, "{landed} of 40 landed in their planned bin");
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn clips_are_level_matched_stereo_and_blind() {
    let dir = designed();
    let m = manifest(dir);
    let names: HashSet<String> = std::fs::read_dir(clips_dir(dir))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names.len(), m.clips.len());
    for p in &m.pairs {
        let ids = [&p.plan.target_clip, &p.plan.answer_clip];
        let lufs: Vec<f64> = ids
            .iter()
            .map(|id| {
                assert!(id.starts_with('c') && id.len() == 7, "random stem {id}");
                let w = decode(&std::fs::read(clips_dir(dir).join(format!("{id}.wav"))).unwrap())
                    .unwrap();
                assert_eq!((w.sample_rate, w.channels.len()), (48_000, 2));
                assert!(w.peak() <= 0.981);
                let mid: Vec<f32> = w.channels[0]
                    .iter()
                    .zip(&w.channels[1])
                    .map(|(l, r)| 0.5 * (l + r))
                    .collect();
                integrated_lufs(&mid, 48_000).unwrap()
            })
            .collect();
        assert!(
            (lufs[0] - lufs[1]).abs() < 0.1,
            "pair {}: {lufs:?}",
            p.plan.index
        );
        let limited = m
            .clips
            .iter()
            .filter(|c| c.pair == p.plan.index)
            .any(|c| c.peak_dbfs > -0.2);
        if !limited {
            assert!(
                (lufs[0] + 23.0).abs() < 0.1,
                "pair {}: {lufs:?}",
                p.plan.index
            );
        }
    }
}

#[test]
fn a_session_resumes_after_q_and_records_the_distances() {
    let dir = scratch("resume");
    copy_dir(designed(), &dir);
    let m = manifest(&dir);
    let trials: Vec<&PlannedTrial> = m.session_trials(1);
    let mut a = PairsArgs {
        session: Some(1),
        ..args(&dir, 12)
    };
    // three trials (with a replay, a bad key and a replay at the rating prompt), then quit at the rating prompt
    let mut lines = Scripted::new(&["rx", "a", "3", "b", "r", "huh", "5", "a", "1", "b", "q"]);
    let mut player = MockPlayer::default();
    let mut out = Vec::new();
    run(&a, &mut out, &mut lines, Some(&mut player)).unwrap();
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("saved 3 answers"), "{text}");
    // 4 trials started: 4 full plays, plus the two replays (one clip, then all three)
    assert_eq!(player.played.len(), 6);
    let f: ResultsFile = read_json(&results_path(&dir, 1)).unwrap();
    assert_eq!(
        (f.schema.as_str(), f.session, f.synthetic),
        (SCHEMA_RESULTS, 1, false)
    );
    assert_eq!(f.plan_hash, m.plan_hash);
    assert_eq!(f.answers.len(), 3);
    let said = [("a", 3u8), ("b", 5), ("a", 1)];
    for (ans, (t, (x, r))) in f.answers.iter().zip(trials.iter().zip(said)) {
        assert_eq!((ans.id.as_str(), ans.n), (t.id.as_str(), t.n));
        assert_eq!((ans.x.as_str(), ans.rating), (x, r));
        let p = &m.pairs[t.pair];
        assert_eq!(ans.pair, t.pair);
        let right = (p.plan.role != PairRole::Same).then(|| x == t.x_is.key());
        assert_eq!(ans.x_correct, right);
        assert_eq!(
            (ans.distance, ans.jnd, ans.percent),
            (p.measured.distance, p.measured.jnd, p.measured.percent)
        );
    }
    assert_eq!(f.answers[0].replays, 1);
    assert_eq!(f.answers[1].replays, 1);

    // resume: the fourth trial comes next, and the session finishes
    let mut rest = Vec::new();
    for _ in 3..trials.len() {
        rest.extend(["b", "4"]);
    }
    a.session = Some(1);
    let mut out = Vec::new();
    run(
        &a,
        &mut out,
        &mut Scripted::new(&rest),
        Some(&mut MockPlayer::default()),
    )
    .unwrap();
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("Resuming after 3 answered trials"), "{text}");
    assert!(text.contains("Session 1 done: 16 answers"), "{text}");
    let f: ResultsFile = read_json(&results_path(&dir, 1)).unwrap();
    let ids: Vec<&str> = f.answers.iter().map(|a| a.id.as_str()).collect();
    let want: Vec<&str> = trials.iter().map(|t| t.id.as_str()).collect();
    assert_eq!(ids, want, "every trial once, in order");

    // the report reads one session
    let mut out = Vec::new();
    let r = PairsArgs {
        action: Some(PairsAction::Report),
        ..args(&dir, 12)
    };
    run(&r, &mut out, &mut Scripted::default(), None).unwrap();
    let text = String::from_utf8(out).unwrap();
    assert!(
        text.contains("12 rated test pairs; session 1 16/16"),
        "{text}"
    );
    assert!(!text.contains("SYNTHETIC"), "{text}");
    let fit: FitData = read_json(&dir.join("pairs-fit.json")).unwrap();
    assert_eq!(fit.schema, SCHEMA_FIT);
    assert_eq!(fit.rows.len(), m.pairs.len());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_dry_run_answers_both_sessions_and_says_so() {
    let dir = scratch("dry");
    copy_dir(designed(), &dir.join("dry-run"));
    let mut out = Vec::new();
    let a = PairsArgs {
        dry_run: true,
        player: "definitely-not-a-player-xyz".into(),
        ..args(&dir, 12)
    };
    run(&a, &mut out, &mut Scripted::default(), None).unwrap();
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("using the pairs designed earlier"), "{text}");
    assert!(
        text.contains("SYNTHETIC") && text.contains("test-retest"),
        "{text}"
    );
    for s in 1..=2 {
        let f: ResultsFile = read_json(&results_path(&dir.join("dry-run"), s)).unwrap();
        assert!(f.synthetic && f.answers.len() == 16);
    }
    let fit: FitData = read_json(&dir.join("dry-run").join("pairs-fit.json")).unwrap();
    assert!(fit.synthetic);
    // a session refuses a synthetic results file
    let s = PairsArgs {
        session: Some(1),
        dir: dir.join("dry-run"),
        ..args(&dir, 12)
    };
    let e = run(
        &s,
        &mut Vec::new(),
        &mut Scripted::default(),
        Some(&mut MockPlayer::default()),
    );
    assert!(e.unwrap_err().to_string().contains("synthetic"));
    std::fs::remove_dir_all(&dir).unwrap();
}

// ---------------------------------------------------------------------------------------------------------------
// The report on hand-made results (no audio)
// ---------------------------------------------------------------------------------------------------------------

fn measured(jnd: f64) -> Measured {
    let percent = Curve::SOUND_V1.percent(jnd);
    Measured {
        distance: 0.1 * jnd,
        floor: 0.0,
        scale: 0.1,
        z_metric: jnd,
        pitch_term: 0.0,
        wave_term: 0.0,
        jnd,
        percent,
        passes: percent >= 90.0,
        pitch_delta_ct: None,
        pitch_floor_ct: 0.0,
        odd_delta: None,
        odd_floor: 0.0,
    }
}

fn hand_pair(index: usize, role: PairRole, jnd: f64) -> Pair {
    let spec = SpecPreset::by_id(1)
        .unwrap()
        .spec(GENERATOR, index as u64 + 1);
    Pair {
        plan: PlannedPair {
            index,
            role,
            level: 1,
            target: spec,
            other: None,
            kind: (role == PairRole::Test).then_some(MoveKind::All),
            param: None,
            sign_seed: 0,
            planned_bin: (role == PairRole::Test).then(|| bin_of(jnd)),
            target_clip: format!("c{index:06}"),
            answer_clip: format!("d{index:06}"),
        },
        code: None,
        other_code: None,
        kind: (role == PairRole::Test).then_some(MoveKind::All),
        steps: (role == PairRole::Test).then_some(1.0),
        ladder_jnd: Vec::new(),
        measured: measured(jnd),
        bin: bin_of(jnd),
    }
}

/// Pairs at the given distances (test), then one identical and one obviously different pair; one session.
fn hand_manifest(jnds: &[f64]) -> Manifest {
    let mut pairs: Vec<Pair> = jnds
        .iter()
        .enumerate()
        .map(|(i, z)| hand_pair(i, PairRole::Test, *z))
        .collect();
    pairs.push(hand_pair(pairs.len(), PairRole::Same, 0.0));
    pairs.push(hand_pair(pairs.len(), PairRole::Different, 40.0));
    let trials = (0..pairs.len())
        .map(|i| PlannedTrial {
            session: 1,
            n: i + 1,
            id: format!("t{i:06}"),
            pair: i,
            target_first: i % 2 == 0,
            x_is: if i % 2 == 0 { Side::A } else { Side::B },
        })
        .collect();
    let c = Curve::SOUND_V1;
    Manifest {
        schema: SCHEMA_MANIFEST.into(),
        config: Config {
            seed: 1,
            pairs: jnds.len(),
            workers: 1,
        },
        plan_hash: "hand".into(),
        engine: "0.0.0".into(),
        scorer: "sound/1".into(),
        curve_z50: c.z50,
        curve_k: c.k,
        pass_mark: 90.0,
        host_threads: 1,
        design_wall_s: 0.0,
        pairs,
        clips: Vec::new(),
        trials,
    }
}

/// Answers: `ratings[i]` for pair i, X right where `right[i]` (identical pair: `a`).
/// A bin's pairs, mean rating, ear same, sound/1 pass, agree, ABX correct and ABX trials.
type BinCounts = (usize, Option<f64>, usize, usize, usize, usize, usize);

fn hand_results(m: &Manifest, session: u32, ratings: &[u8], right: &[bool]) -> ResultsFile {
    let mut f = ResultsFile::new(m, session, false);
    for (t, (r, ok)) in m
        .session_trials(session)
        .iter()
        .zip(ratings.iter().zip(right))
    {
        let p = &m.pairs[t.pair];
        let x = if *ok == (t.x_is == Side::A) { "a" } else { "b" };
        f.answers.push(PairAnswer {
            n: t.n,
            id: t.id.clone(),
            rating: *r,
            x: x.into(),
            x_correct: (p.plan.role != PairRole::Same).then_some(*ok),
            replays: 0,
            ms: 0,
            pair: t.pair,
            role: p.plan.role,
            level: 1,
            kind: p.kind,
            distance: p.measured.distance,
            jnd: p.measured.jnd,
            percent: p.measured.percent,
        });
    }
    f
}

#[test]
fn the_report_counts_each_bin_from_a_synthetic_answers_file() {
    // two pairs per bin; bin by bin: ratings and X answers chosen by hand
    let jnds = [0.2, 0.4, 0.6, 0.9, 1.2, 1.8, 2.5, 3.5, 5.0, 7.0, 10.0, 20.0];
    let m = hand_manifest(&jnds);
    let ratings = [5, 5, 5, 4, 5, 4, 3, 3, 2, 2, 1, 1, 5, 1];
    let right = [
        false, true, true, false, true, true, true, true, true, true, true, true, true, true,
    ];
    let f = hand_results(&m, 1, &ratings, &right);
    let r = analyze(&m, std::slice::from_ref(&f), 200, 3);
    assert_eq!(r.test_pairs, 12);
    assert!(!r.synthetic);
    let rows: Vec<BinCounts> = r
        .bins
        .iter()
        .map(|b| {
            (
                b.pairs,
                b.mean_rating,
                b.ear_same,
                b.sound_pass,
                b.agree,
                b.abx_correct,
                b.abx_trials,
            )
        })
        .collect();
    assert_eq!(
        rows,
        vec![
            (2, Some(5.0), 2, 2, 2, 1, 2),
            (2, Some(4.5), 1, 2, 1, 1, 2),
            (2, Some(4.5), 1, 0, 1, 2, 2),
            (2, Some(3.0), 0, 0, 2, 2, 2),
            (2, Some(2.0), 0, 0, 2, 2, 2),
            (2, Some(1.0), 0, 0, 2, 2, 2),
        ]
    );
    let a = r.agreement;
    assert_eq!((a.both, a.ear_only, a.sound_only, a.neither), (3, 1, 1, 7));
    assert!((a.share() - 10.0 / 12.0).abs() < 1e-12);
    assert!(
        r.anchors.ok && r.anchors.same_mean == Some(5.0) && r.anchors.different_mean == Some(1.0)
    );
    assert_eq!(r.anchors.different_abx, (1, 1));
    assert!(r.spearman.unwrap().estimate > 0.9 && r.retest.is_none());
    let fit = r.fit.unwrap();
    assert!(fit.z_pass > 0.4 && fit.z_pass < 2.0, "{fit:?}");
    let text = r.to_text();
    assert!(
        text.contains("0.5-1") && text.contains("8+") && text.contains("pass agreement"),
        "{text}"
    );
    // the fit rows hold every pair with its rating
    let d = fit_rows(&m, &[f], 50, 3);
    assert_eq!(d.rows.len(), 14);
    assert_eq!(d.rows[3].ratings, vec![Some(4)]);
    assert_eq!(d.rows[12].role, PairRole::Same);
    assert_eq!(d.rows[0].x_correct, vec![Some(false)]);
}

#[test]
fn the_report_recovers_the_curve_that_made_the_ratings() {
    // ratings drawn noise-free from a curve whose pass point is at 1.5 JND, rounded as a listener would
    let (c, q) = (5.5, 1.5);
    let jnds: Vec<f64> = (0..60)
        .map(|i| 0.1 * libm::pow(1.09, f64::from(i)))
        .collect();
    let m = hand_manifest(&jnds);
    let ratings: Vec<u8> = jnds
        .iter()
        .map(|z| rating_model(*z, c, q).round() as u8)
        .chain([5, 1])
        .collect();
    let f1 = hand_results(&m, 1, &ratings, &[true; 62]);
    let r = analyze(&m, &[f1], 200, 1);
    let fit = r.fit.unwrap();
    let want = c * libm::pow(7.0, -1.0 / q);
    assert!((want - 1.5).abs() < 0.01);
    assert!(
        (fit.z_pass - want).abs() / want < 0.15,
        "{} against {want}",
        fit.z_pass
    );
    assert!((fit.z50 - c).abs() / c < 0.15, "{fit:?}");
    assert!(r.reading.contains("stricter"), "{}", r.reading);
}

#[test]
fn the_report_command_writes_both_files_and_needs_results() {
    let dir = scratch("report");
    let m = hand_manifest(&[0.3, 0.8, 1.5, 3.0, 6.0, 12.0]);
    write_json(&manifest_path(&dir), &m).unwrap();
    let r = PairsArgs {
        action: Some(PairsAction::Report),
        ..args(&dir, 6)
    };
    let e = run(&r, &mut Vec::new(), &mut Scripted::default(), None).unwrap_err();
    assert!(e.to_string().contains("no results"), "{e}");
    let f1 = hand_results(&m, 1, &[5, 5, 4, 3, 2, 1, 5, 1], &[true; 8]);
    write_json(&results_path(&dir, 1), &f1).unwrap();
    let mut out = Vec::new();
    run(&r, &mut out, &mut Scripted::default(), None).unwrap();
    let text = String::from_utf8(out).unwrap();
    assert!(
        text.contains("6 rated test pairs") && text.contains("wrote"),
        "{text}"
    );
    assert!(dir.join("pairs-report.json").exists() && dir.join("pairs-fit.json").exists());
    // a results file from another design is refused
    let mut other = f1;
    other.plan_hash = "elsewhere".into();
    write_json(&results_path(&dir, 1), &other).unwrap();
    let e = run(&r, &mut Vec::new(), &mut Scripted::default(), None).unwrap_err();
    assert!(e.to_string().contains("another design"), "{e}");
    // no design at all
    let empty = scratch("report-empty");
    let e = run(
        &PairsArgs {
            action: Some(PairsAction::Report),
            ..args(&empty, 6)
        },
        &mut Vec::new(),
        &mut Scripted::default(),
        None,
    )
    .unwrap_err();
    assert!(e.to_string().contains("no design"), "{e}");
    std::fs::remove_dir_all(&dir).unwrap();
    std::fs::remove_dir_all(&empty).unwrap();
}
