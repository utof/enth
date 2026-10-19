//! The commands that render and measure: `render`, `describe`, `check`, `step`, `bench`.

use std::collections::BTreeMap;
use std::path::Path;

use enth_analysis::descriptors::Want;
use enth_analysis::loudness::{Level, LoudnessMeter};
use enth_analysis::{CheckReport, Describer, Descriptors, Expect, Feature, ItemResult, parse_list};
use enth_patch::catalogue::VoiceParam;
use enth_patch::{Op, ParamAddr, Patch, canon};
use enth_render::{DEFAULT_SEED, RenderOutput};

use super::common::{self, HEADLINE, reading, readings, resolve_edits};
use super::{Reply, Res};
use crate::cli::{Edits, FixArgs};
use crate::error::{self, CliError, EXIT_CHECK_FAILED, EXIT_OK};
use crate::eval::Evaluator;
use crate::fix::{self, FixOutcome, FixRequest};
use crate::perf::{Perf, PerfArgs};
use crate::schema::{
    BenchOut, BlockedOut, BuildOut, CheckBody, CheckOut, FixOut, ItemHintOut, ItemOut, RenderOut,
    StepOut, WavMeta, ids,
};
use crate::wav::{self, Bits};
use crate::{patchio, templates};

/// Delays (in samples) of the two re-analyses that give the noise of a check on a WAV. Not multiples of any
/// analysis hop, so every frame grid moves.
const WAV_NUISANCE_SHIFTS: [usize; 2] = [137, 311];

fn wav_meta(patch: &Patch, perf: &Perf, seed: u64, sample_rate: u32) -> WavMeta {
    WavMeta {
        schema: ids::WAVMETA.to_owned(),
        engine: enth_patch::file::ENGINE_VERSION.to_owned(),
        sample_rate,
        seed,
        patch_hash: canon::exact_hash(patch).to_hex(),
        perf: perf.to_file(),
    }
}

fn write_wav(path: &Path, r: &RenderOutput, bits: Bits, meta: &WavMeta) -> Result<(), CliError> {
    let bytes = wav::encode(&r.left, &r.right, r.sample_rate, bits, meta);
    std::fs::write(path, bytes).map_err(|e| error::write_failed(&patchio::show(path), &e))
}

/// Levels over both channels: the larger peak and true peak, the clipped share of all samples.
fn stereo_level(r: &RenderOutput) -> Level {
    let mut m = LoudnessMeter::new(r.sample_rate);
    let l = m.level(&r.left);
    let rr = m.level(&r.right);
    Level {
        lufs_i: None,
        peak_dbfs: l.peak_dbfs.max(rr.peak_dbfs),
        true_peak_dbtp: l.true_peak_dbtp.max(rr.true_peak_dbtp),
        clipped_pct: f64::midpoint(l.clipped_pct, rr.clipped_pct),
        clipping: l.clipping || rr.clipping,
    }
}

pub fn render(
    path: &Path,
    perf: &PerfArgs,
    out: Option<&Path>,
    bits: Bits,
    seed: Option<u64>,
    stdin: &str,
) -> Res {
    let (patch, notes) = patchio::load(path)?;
    let perf = Perf::from_args(perf, stdin)?.unwrap_or_else(Perf::default_perf);
    let seed = seed.unwrap_or(DEFAULT_SEED);
    let ev = Evaluator::new(&perf, seed, Want::CORE);
    let r = ev.render(&patch, seed)?;
    let wav_path = out.map_or_else(|| patchio::with_wav_extension(path), Path::to_path_buf);
    write_wav(
        &wav_path,
        &r,
        bits,
        &wav_meta(&patch, &perf, seed, r.sample_rate),
    )?;
    let lv = stereo_level(&r);
    let res = RenderOut {
        schema: ids::RENDER.to_owned(),
        wav: patchio::show(&wav_path),
        format: bits.name().to_owned(),
        sample_rate: r.sample_rate,
        channels: 2,
        dur_s: r.frames() as f64 / f64::from(r.sample_rate),
        peak_dbfs: lv.peak_dbfs,
        true_peak_dbtp: lv.true_peak_dbtp,
        clipped_pct: lv.clipped_pct,
        clipping: lv.clipping,
        perf: perf.to_out(),
        patch_hash: canon::exact_hash(&patch).short(),
        seed,
    };
    let mut reply = Reply::new(&res);
    if r.stats.dropped_notes > 0 {
        reply = reply.note(&format!(
            "{} notes were dropped: polyphony was full",
            r.stats.dropped_notes
        ));
    }
    for n in notes {
        reply = reply.note(&n);
    }
    Ok(reply)
}

/// A WAV ready to analyse: the mono signal, its rate, and its performance (flag > chunk > inferred > unknown).
struct WavInput {
    mono: Vec<f32>,
    sample_rate: u32,
    perf: Perf,
}

fn load_wav(bytes: &[u8], name: &str, flag: Option<Perf>) -> Result<WavInput, CliError> {
    let d = wav::decode(bytes).map_err(|why| {
        error::failed(
            "unsupported_wav",
            format!("cannot read {name}: {why}"),
            "describe and check read 8-32 bit integer or 32-bit float WAVs at 8-192 kHz",
        )
        .in_file(name)
    })?;
    if !(8_000..192_000).contains(&d.sample_rate) {
        return Err(error::failed(
            "unsupported_wav",
            format!(
                "{name} is at {} Hz; the analysis runs at 8 to 192 kHz",
                d.sample_rate
            ),
            "resample the file",
        )
        .in_file(name));
    }
    let dur_s = d.mono.len() as f64 / f64::from(d.sample_rate);
    let mut perf = if let Some(p) = flag {
        p
    } else if let Some(meta) = &d.meta {
        Perf::from_file(&meta.perf, crate::perf::Source::Wav)?
    } else if let Some(off) =
        enth_analysis::envelope::infer_note_off(&d.mono, f64::from(d.sample_rate))
    {
        Perf::inferred(off, dur_s)
    } else {
        Perf::unknown(dur_s)
    };
    perf.dur_s = dur_s;
    Ok(WavInput {
        mono: d.mono,
        sample_rate: d.sample_rate,
        perf,
    })
}

pub fn describe(input: &Path, perf: &PerfArgs, seed: Option<u64>, stdin: &str) -> Res {
    let name = patchio::show(input);
    let bytes = patchio::read_bytes(input)?;
    let flag = Perf::from_args(perf, stdin)?;
    if patchio::is_wav(&bytes) {
        let w = load_wav(&bytes, &name, flag)?;
        let d = Describer::new(f64::from(w.sample_rate))
            .describe(&w.mono, w.perf.timing())
            .map_err(|e| error::from_analysis(&e))?;
        let out = common::describe_out(&name, w.sample_rate, w.perf.to_out(), &d);
        return Ok(Reply::new(&out));
    }
    let (patch, _) = patchio::parse(&bytes, &name)?;
    let perf = flag.unwrap_or_else(Perf::default_perf);
    let seed = seed.unwrap_or(DEFAULT_SEED);
    let ev = Evaluator::new(&perf, seed, Want::ALL);
    let d = one(ev.describe_jobs(&[(&patch, seed)]))?;
    let out = common::describe_out(&name, ev.sample_rate(), perf.to_out(), &d);
    Ok(Reply::new(&out))
}

fn one<T>(v: Vec<Result<T, CliError>>) -> Result<T, CliError> {
    v.into_iter()
        .next()
        .unwrap_or_else(|| unreachable!("one job, one result"))
}

// ---------------------------------------------------------------------------------------------------------------
// check
// ---------------------------------------------------------------------------------------------------------------

fn want_for(items: &[Expect]) -> Want {
    let features: Vec<Feature> = items.iter().map(|e| e.feature).collect();
    Want::for_features(&features)
}

fn item_out(r: &ItemResult, hint: Option<&fix::ItemHint>) -> ItemOut {
    let unit = r.feature.unit();
    ItemOut {
        expect: r.expect.clone(),
        got: r.got.map(|g| reading(r.feature, g)),
        unit: unit.label().to_owned(),
        pass: r.pass,
        margin: r.margin,
        margin_unit: (r.unit != unit).then(|| r.unit.label().to_owned()),
        margin_pct: r.margin_pct,
        noise: r.noise,
        thin: r.thin,
        note: match (r.feature, r.got) {
            (f, Some(hz)) if f.is_pitch() && hz > 0.0 => {
                let (name, cents) = enth_analysis::units::nearest_note(hz);
                Some(format!("{name}{:+}ct", cents.round() as i64))
            }
            _ => None,
        },
        why: (r.got.is_none() && !r.pass).then(|| r.feature.missing_reason().to_owned()),
        lower_bound: r.lower_bound,
        estimated: r.estimated,
        unstable: r.unstable,
        hint: hint.map(|h| ItemHintOut {
            text: h.text.clone(),
            measured: h.measured,
        }),
    }
}

fn check_body(
    report: &CheckReport,
    items: &[Expect],
    outcome: &FixOutcome,
    patch: Option<&Patch>,
) -> CheckBody {
    let results = report
        .results
        .iter()
        .enumerate()
        .map(|(i, r)| item_out(r, outcome.item_hints.get(i).and_then(Option::as_ref)))
        .collect();
    let fix = match (&outcome.fix, patch) {
        (Some(f), Some(p)) => Some(FixOut {
            measured: f.measured,
            pass_all: f.pass_all,
            renders: f.renders,
            ops: fix::ops_json(p, &f.ops),
            after: f
                .after
                .iter()
                .map(|(feat, v)| (feat.path().to_owned(), reading(*feat, *v)))
                .collect(),
            single_param_enough: f.single_param_enough,
            note: f.note.clone(),
        }),
        _ => None,
    };
    let blocked = outcome
        .blocked
        .iter()
        .map(|b| BlockedOut {
            expect: items
                .get(b.item)
                .map_or_else(String::new, |e| e.canonical.clone()),
            why: b.why.clone(),
            reason: b.reason.to_owned(),
            at_bound: b.at_bound.iter().map(ToString::to_string).collect(),
            conflicts_with: b.conflicts_with.clone(),
        })
        .collect();
    CheckBody {
        pass: report.pass,
        passed: report.passed,
        failed: report.failed,
        thin: report.thin,
        results,
        fix,
        blocked,
    }
}

/// Judge a patch: the judged render plus two nuisance re-renders, then (for failures) the fix seam. `main` is
/// the judged render's descriptors when the caller already has them.
fn judge_patch(
    patch: &Patch,
    items: &[Expect],
    ev: &Evaluator,
    main: Option<Descriptors>,
    fix_args: &FixArgs,
) -> Result<CheckBody, CliError> {
    let [n1, n2] = ev.nuisance_seeds();
    let mut jobs: Vec<(&Patch, u64)> = vec![(patch, n1), (patch, n2)];
    if main.is_none() {
        jobs.insert(0, (patch, ev.seed()));
    }
    let mut results = ev.describe_jobs(&jobs).into_iter();
    let main = match main {
        Some(d) => d,
        None => results
            .next()
            .unwrap_or_else(|| unreachable!("three jobs"))?,
    };
    let nuisance = results.collect::<Result<Vec<_>, _>>()?;
    let report = enth_analysis::check(items, &main, &nuisance);
    let outcome = if report.pass || fix_args.no_fix {
        FixOutcome::default()
    } else {
        fix::propose(&FixRequest {
            patch,
            items,
            report: &report,
            base: &main,
            evaluator: ev,
            budget: fix_args.fix_budget,
        })?
    };
    Ok(check_body(&report, items, &outcome, Some(patch)))
}

/// Judge a WAV: noise from re-analysing the signal delayed by a few samples (no patch, so no fix).
fn judge_wav(w: &WavInput, items: &[Expect]) -> Result<CheckBody, CliError> {
    let sr = f64::from(w.sample_rate);
    let want = want_for(items);
    let mut describer = Describer::new(sr);
    let timing = w.perf.timing();
    let main = describer
        .describe_some(&w.mono, timing, want)
        .map_err(|e| error::from_analysis(&e))?;
    let mut nuisance = Vec::new();
    for shift in WAV_NUISANCE_SHIFTS {
        let mut x = vec![0.0f32; shift];
        x.extend_from_slice(&w.mono);
        let dt = shift as f64 / sr;
        let t = enth_analysis::NoteTiming {
            on_s: timing.on_s + dt,
            off_s: timing.off_s.map(|o| o + dt),
        };
        nuisance.push(
            describer
                .describe_some(&x, t, want)
                .map_err(|e| error::from_analysis(&e))?,
        );
    }
    let report = enth_analysis::check(items, &main, &nuisance);
    Ok(check_body(&report, items, &FixOutcome::default(), None))
}

pub fn check(
    input: &Path,
    expect: &str,
    perf: &PerfArgs,
    fix_args: &FixArgs,
    seed: Option<u64>,
    stdin: &str,
) -> Res {
    let items = parse_list(expect).map_err(|e| error::from_expect(&e))?;
    let name = patchio::show(input);
    let bytes = patchio::read_bytes(input)?;
    let flag = Perf::from_args(perf, stdin)?;
    let (body, perf_out) = if patchio::is_wav(&bytes) {
        let w = load_wav(&bytes, &name, flag)?;
        (judge_wav(&w, &items)?, w.perf.to_out())
    } else {
        let (patch, _) = patchio::parse(&bytes, &name)?;
        let perf = flag.unwrap_or_else(Perf::default_perf);
        let ev = Evaluator::new(&perf, seed.unwrap_or(DEFAULT_SEED), want_for(&items));
        (
            judge_patch(&patch, &items, &ev, None, fix_args)?,
            perf.to_out(),
        )
    };
    let code = if body.pass {
        EXIT_OK
    } else {
        EXIT_CHECK_FAILED
    };
    let out = CheckOut {
        schema: ids::CHECK.to_owned(),
        source: name,
        perf: perf_out,
        pass: body.pass,
        passed: body.passed,
        failed: body.failed,
        thin: body.thin,
        results: body.results,
        fix: body.fix,
        blocked: body.blocked,
    };
    Ok(Reply::new(&out)
        .code(code)
        .columns(&["expect", "pass", "got", "unit", "margin", "noise", "thin"]))
}

// ---------------------------------------------------------------------------------------------------------------
// step
// ---------------------------------------------------------------------------------------------------------------

/// The arguments of `step`.
pub struct StepArgs<'a> {
    pub patch: &'a Path,
    pub edits: &'a Edits,
    pub expect: Option<&'a str>,
    pub perf: &'a PerfArgs,
    pub out: Option<&'a Path>,
    pub wav: Option<&'a Path>,
    pub full: bool,
    pub fix: &'a FixArgs,
    pub seed: Option<u64>,
    pub stdin: &'a str,
}

pub fn step(a: &StepArgs<'_>) -> Res {
    let items = a
        .expect
        .map(parse_list)
        .transpose()
        .map_err(|e| error::from_expect(&e))?;
    let (patch, notes) = patchio::load(a.patch)?;
    let perf = Perf::from_args(a.perf, a.stdin)?.unwrap_or_else(Perf::default_perf);
    let ed = resolve_edits(&patch, a.edits, a.stdin)?;
    let out_path = a.out.unwrap_or(a.patch);
    patchio::write(out_path, &ed.after)?;
    let wav_path = a
        .wav
        .map_or_else(|| patchio::with_wav_extension(out_path), Path::to_path_buf);

    let seed = a.seed.unwrap_or(DEFAULT_SEED);
    let ev = Evaluator::new(&perf, seed, items.as_deref().map_or(Want::CORE, want_for));
    let r = ev.render(&ed.after, seed)?;
    write_wav(
        &wav_path,
        &r,
        Bits::F32,
        &wav_meta(&ed.after, &perf, seed, r.sample_rate),
    )?;
    // The judged reading is the solver's `measure` (the render above is the same bits: it only feeds the WAV).
    let main = one(ev.describe_jobs_with(&[(&ed.after, seed)], Want::ALL))?;

    let check = match &items {
        Some(items) => Some(judge_patch(
            &ed.after,
            items,
            &ev,
            Some(main.clone()),
            a.fix,
        )?),
        None => None,
    };
    let wanted: Vec<&str> = items
        .iter()
        .flatten()
        .map(|e| e.feature.path())
        .chain(HEADLINE.iter().copied())
        .collect();
    let describe: BTreeMap<_, _> = readings(&main)
        .into_iter()
        .filter(|(k, _)| a.full || wanted.contains(&k.as_str()))
        .collect();
    let code = match &check {
        Some(c) if !c.pass => EXIT_CHECK_FAILED,
        _ => EXIT_OK,
    };
    let out = StepOut {
        schema: ids::STEP.to_owned(),
        patch: patchio::show(out_path),
        wav: patchio::show(&wav_path),
        applied: common::changes(&patch, &ed.ops),
        inverse: ed.inverse_json(),
        perf: perf.to_out(),
        describe,
        check,
    };
    let mut reply = Reply::new(&out).code(code).columns(&[
        "expect", "pass", "got", "unit", "margin", "noise", "thin", "op", "target",
    ]);
    for n in notes {
        reply = reply.note(&n);
    }
    Ok(reply)
}

// ---------------------------------------------------------------------------------------------------------------
// bench
// ---------------------------------------------------------------------------------------------------------------

pub fn bench(patch: Option<&Path>, voices: u32, seconds: f64) -> Res {
    if !(1..=16).contains(&voices) {
        return Err(error::usage(
            "usage",
            format!("--voices {voices} is outside 1..=16"),
            "the engine plays at most 16 voices",
        ));
    }
    if !(seconds.is_finite() && (0.05..=120.0).contains(&seconds)) {
        return Err(error::usage(
            "usage",
            format!("--seconds {seconds} is outside 0.05..=120"),
            "render between 0.05 and 120 seconds",
        ));
    }
    let (mut p, name) = match patch {
        Some(path) => (patchio::load(path)?.0, patchio::show(path)),
        None => (
            templates::find("pad_2osc")
                .unwrap_or_else(|| unreachable!("the pad template exists"))
                .patch(),
            "template:pad_2osc".to_owned(),
        ),
    };
    for (param, text) in [
        (VoiceParam::Mode, "poly".to_owned()),
        (VoiceParam::Polyphony, voices.to_string()),
    ] {
        let addr = ParamAddr::voice(param);
        let value = p
            .spec(&addr)
            .and_then(|s| s.parse(&text).ok())
            .unwrap_or_else(|| unreachable!("{text} is valid for {addr}"));
        enth_patch::apply(&mut p, Op::SetParam { addr, value })
            .map_err(|e| error::from_patch_error("voice", &e, error::Ctx::default()))?;
    }
    let notes = (0..voices)
        .map(|i| crate::perf::Note {
            key: Some(36 + (i * 5) as u8),
            vel: Some(100),
            on_s: 0.0,
            off_s: Some(seconds),
        })
        .collect();
    let perf = Perf {
        notes,
        dur_s: seconds,
        source: crate::perf::Source::Flag,
    };
    let ev = Evaluator::new(&perf, DEFAULT_SEED, Want::CORE);
    let t0 = std::time::Instant::now();
    let r = ev.render(&p, DEFAULT_SEED)?;
    let wall = t0.elapsed().as_secs_f64().max(1e-9);
    let frames = r.frames() as f64;
    let res = BenchOut {
        schema: ids::BENCH.to_owned(),
        patch: name,
        voices,
        seconds,
        sample_rate: r.sample_rate,
        wall_ms: wall * 1000.0,
        realtime_x_per_core: seconds / wall,
        ns_per_voice_sample: wall * 1e9 / (frames * f64::from(voices)),
        build: BuildOut {
            profile: if cfg!(debug_assertions) { "debug" } else { "release" }.to_owned(),
            target: format!("{}-{}", std::env::consts::ARCH, std::env::consts::OS),
        },
        note: "wall-clock, information only; the regression gate is instruction counts (cargo xtask bench)"
            .to_owned(),
    };
    Ok(Reply::new(&res))
}
