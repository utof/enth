//! What each subcommand does and prints. Every command returns the JSON object to print and its exit code; the
//! binary only parses flags and prints. Files are read and written here (the spike's CLI layer), the maths is not.

use crate::descriptors::{Describer, Descriptors, Want};
use crate::error::{CliError, EXIT_CHECK_FAILED, EXIT_NOT_FOUND, EXIT_RENDER, Res, invalid, usage};
use crate::expect::{Eval, Expect, FEATURES, parse_list, want_for};
use crate::guide;
use crate::hints::{hint_for, probe};
use crate::patch::{Change, Patch};
use crate::synth::{SAMPLE_RATE, measure, render_patch, wav_bytes};
use crate::units::{Unit, r4};
use serde_json::{Map, Value, json};
use spike_metric::wav::decode_wav_i16;
use std::path::{Path, PathBuf};

/// What a command produced.
pub struct Out {
    pub json: Value,
    pub exit: i32,
}

fn done(json: Value) -> Out {
    Out { json, exit: 0 }
}

fn n(x: f64) -> Value {
    if x.is_finite() {
        json!(r4(x))
    } else {
        Value::Null
    }
}

fn on(x: Option<f64>) -> Value {
    x.map_or(Value::Null, n)
}

fn with_schema(name: &str, mut obj: Map<String, Value>) -> Value {
    obj.insert("schema".into(), json!(format!("enth.{name}/1")));
    Value::Object(obj)
}

fn path_text(p: &Path) -> String {
    p.display().to_string()
}

// ---- files ----------------------------------------------------------------------------------------------------

fn read_bytes(path: &Path) -> Res<Vec<u8>> {
    std::fs::read(path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            CliError::new(
                EXIT_NOT_FOUND,
                "file_not_found",
                format!("{} does not exist", path.display()),
            )
            .with("path", path_text(path))
            .hint("create a patch with: spike-agent new --out p.json")
        } else {
            CliError::new(
                EXIT_RENDER,
                "read_failed",
                format!("cannot read {}: {e}", path.display()),
            )
            .with("path", path_text(path))
        }
    })
}

fn write_bytes(path: &Path, bytes: &[u8]) -> Res<()> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| write_failed(path, &e))?;
    }
    std::fs::write(path, bytes).map_err(|e| write_failed(path, &e))
}

fn write_failed(path: &Path, e: &std::io::Error) -> CliError {
    CliError::new(
        EXIT_RENDER,
        "write_failed",
        format!("cannot write {}: {e}", path.display()),
    )
    .with("path", path_text(path))
}

fn write_patch(path: &Path, patch: &Patch) -> Res<()> {
    let mut text = serde_json::to_string_pretty(&patch.to_json()).expect("patch JSON serialises");
    text.push('\n');
    write_bytes(path, text.as_bytes())
}

/// Overrides from `--perf note=C3,dur=3s,hold=2s` (keys may carry a `perf.` prefix).
fn apply_perf(patch: &mut Patch, perf: &str, validate: bool) -> Res<()> {
    for item in perf.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        let Some((k, v)) = item.split_once('=') else {
            return Err(usage("bad_perf", format!("'{item}' is not key=value"))
                .hint("example: --perf note=C3,dur=3s,hold=2s"));
        };
        let k = k.trim().trim_start_matches("perf.");
        if !matches!(k, "note" | "dur" | "hold") {
            return Err(invalid(
                "unknown_param",
                format!("no performance setting named '{k}'"),
            )
            .with("did_you_mean", json!(["note", "dur", "hold"]))
            .hint("--perf takes note, dur and hold"));
        }
        patch.set(&format!("perf.{k}"), v)?;
    }
    if validate { patch.validate() } else { Ok(()) }
}

fn load_patch(path: &Path, perf: Option<&str>) -> Res<Patch> {
    let bytes = read_bytes(path)?;
    let doc: Value = serde_json::from_slice(&bytes).map_err(|e| {
        invalid(
            "bad_patch",
            format!("{} is not valid JSON: {e}", path.display()),
        )
        .with("path", path_text(path))
        .hint("create a valid file with: spike-agent new --out p.json")
    })?;
    let mut patch = Patch::from_json(&doc)?;
    if let Some(p) = perf {
        apply_perf(&mut patch, p, true)?;
    }
    Ok(patch)
}

/// What `describe` and `check` accept: a patch (rendered first) or a WAV file.
enum Input {
    Patch(Box<Patch>),
    Wav {
        samples: Vec<f32>,
        rate: u32,
        hold_s: Option<f64>,
    },
}

fn load_input(path: &Path, perf: Option<&str>) -> Res<Input> {
    let is_wav = path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("wav"));
    if !is_wav {
        return Ok(Input::Patch(Box::new(load_patch(path, perf)?)));
    }
    let bytes = read_bytes(path)?;
    let (rate, samples) = decode_wav_i16(&bytes).ok_or_else(|| {
        CliError::new(
            EXIT_RENDER,
            "unsupported_wav",
            format!("{} is not a 16-bit mono PCM WAV", path.display()),
        )
        .with("path", path_text(path))
        .hint("render one with: spike-agent render p.json --out p.wav")
    })?;
    let hold_s = match perf {
        Some(p) if p.contains("hold") => {
            let mut scratch = Patch::default();
            apply_perf(&mut scratch, p, false)?;
            Some(scratch.perf.hold_s)
        }
        _ => None,
    };
    Ok(Input::Wav {
        samples,
        rate,
        hold_s,
    })
}

fn default_wav_path(patch_path: &Path) -> PathBuf {
    patch_path.with_extension("wav")
}

// ---- JSON shapes ------------------------------------------------------------------------------------------------

#[must_use]
pub fn describe_json(d: &Descriptors, explain: bool) -> Value {
    let mut m = Map::new();
    m.insert("dur_s".into(), n(d.dur_s));
    if d.silent {
        m.insert("silent".into(), json!(true));
        m.insert(
            "loudness".into(),
            json!({"peak_dbfs": n(d.peak_dbfs), "clipping": false}),
        );
        return Value::Object(m);
    }
    m.insert(
        "pitch".into(),
        d.f0_hz.map_or(Value::Null, |f| {
            json!({"f0_hz": n(f), "note": d.note, "cents_dev": on(d.cents_dev), "stability": on(d.stability)})
        }),
    );
    let mut env = Map::new();
    env.insert("attack_ms".into(), on(d.attack_ms));
    env.insert("decay_ms".into(), on(d.decay_ms));
    env.insert("sustain_db".into(), on(d.sustain_db));
    env.insert("release_ms".into(), on(d.release_ms));
    env.insert("duration_ms".into(), on(d.duration_ms));
    if d.release_truncated {
        env.insert("release_truncated".into(), json!(true));
    }
    if d.duration_truncated {
        env.insert("duration_truncated".into(), json!(true));
    }
    m.insert("envelope".into(), Value::Object(env));
    m.insert(
        "spectrum".into(),
        json!({
            "centroid_hz": on(d.centroid_hz),
            "centroid_track_hz": d.centroid_track_hz.map_or(Value::Null, |t| json!([n(t[0]), n(t[1]), n(t[2])])),
        }),
    );
    m.insert("noise_ratio".into(), on(d.noise_ratio));
    m.insert(
        "detune".into(),
        json!({"beating_hz": on(d.beating_hz), "depth_db": on(d.beat_depth_db)}),
    );
    m.insert(
        "loudness".into(),
        json!({"lufs_i": on(d.lufs_i), "peak_dbfs": n(d.peak_dbfs), "clipping": d.clipping}),
    );
    if explain {
        let meaning: Map<String, Value> = FEATURES
            .iter()
            .map(|f| (f.name().to_string(), json!(f.meaning())))
            .collect();
        m.insert("meaning".into(), Value::Object(meaning));
    }
    Value::Object(m)
}

fn eval_json(e: &Expect, ev: &Eval, d: &Descriptors, hint: Option<Value>) -> Value {
    let mut m = Map::new();
    m.insert("expect".into(), json!(e.text));
    m.insert("feature".into(), json!(e.feature.name()));
    m.insert("got".into(), on(ev.got));
    let unit = e.feature.unit();
    if unit != Unit::Ratio {
        m.insert("unit".into(), json!(unit.label()));
    }
    m.insert("pass".into(), json!(ev.pass));
    if ev.lower_bound {
        m.insert("lower_bound".into(), json!(true));
    }
    if let Some(c) = ev.error_ct {
        m.insert("error_ct".into(), n(c));
    }
    if e.feature == crate::expect::Feature::Pitch
        && let (Some(note), Some(dev)) = (&d.note, d.cents_dev)
    {
        m.insert("note".into(), json!(format!("{note}{dev:+.0}ct")));
    }
    if ev.got.is_none() {
        m.insert("why_missing".into(), json!(e.feature.missing_reason()));
    }
    if let Some(h) = hint {
        m.insert("hint".into(), h);
    }
    Value::Object(m)
}

/// Judge `expects` against `d`; for a patch, add a measured hint to every failure.
fn run_checks(
    expects: &[Expect],
    d: &Descriptors,
    patch: Option<&Patch>,
    describer: &mut Describer,
) -> Res<(Value, bool)> {
    let evals: Vec<Eval> = expects.iter().map(|e| e.eval(d)).collect();
    let failing: Vec<usize> = (0..expects.len()).filter(|&i| !evals[i].pass).collect();
    let mut hints: Vec<Option<Value>> = vec![None; expects.len()];
    if let (Some(patch), false) = (patch, failing.is_empty()) {
        let failing_exp: Vec<Expect> = failing.iter().map(|&i| expects[i].clone()).collect();
        let want = want_for(&failing_exp);
        let probes = probe(patch, d, describer, want)?;
        for &i in &failing {
            hints[i] = Some(hint_for(&expects[i], d, &probes, describer, want)?);
        }
    }
    let results: Vec<Value> = expects
        .iter()
        .zip(&evals)
        .zip(hints)
        .map(|((e, ev), h)| eval_json(e, ev, d, h))
        .collect();
    let passed = evals.iter().filter(|e| e.pass).count();
    let pass = passed == expects.len();
    Ok((
        json!({"pass": pass, "passed": passed, "failed": expects.len() - passed, "results": results}),
        pass,
    ))
}

// ---- commands ---------------------------------------------------------------------------------------------------

#[must_use]
pub fn cmd_guide() -> Out {
    done(guide::guide())
}

#[must_use]
pub fn cmd_params() -> Out {
    done(guide::params())
}

pub fn cmd_new(out: &Path, perf: Option<&str>) -> Res<Out> {
    let mut patch = Patch::default();
    if let Some(p) = perf {
        apply_perf(&mut patch, p, true)?;
    }
    write_patch(out, &patch)?;
    let mut m = Map::new();
    m.insert("wrote".into(), json!(path_text(out)));
    m.insert("perf".into(), guide::perf_echo(&patch));
    m.insert(
        "next".into(),
        json!("spike-agent step <patch> --set key=value --expect '<list>' --out <patch>"),
    );
    Ok(done(with_schema("new", m)))
}

pub fn cmd_apply(
    path: &Path,
    sets: &[String],
    out: Option<&Path>,
    dry_run: bool,
    perf: Option<&str>,
) -> Res<Out> {
    let base = load_patch(path, None)?;
    let (mut next, changes) = base.apply_sets(sets)?;
    if let Some(p) = perf {
        apply_perf(&mut next, p, true)?;
    }
    let target = out.unwrap_or(path);
    if !dry_run {
        write_patch(target, &next)?;
    }
    let mut m = Map::new();
    m.insert("changed".into(), changes_json(&changes));
    m.insert(
        "wrote".into(),
        if dry_run {
            Value::Null
        } else {
            json!(path_text(target))
        },
    );
    m.insert("dry_run".into(), json!(dry_run));
    Ok(done(with_schema("apply", m)))
}

fn changes_json(changes: &[Change]) -> Value {
    json!(
        changes
            .iter()
            .map(|c| json!({"param": c.param, "from": c.from, "to": c.to}))
            .collect::<Vec<_>>()
    )
}

pub fn cmd_render(path: &Path, out: Option<&Path>, perf: Option<&str>) -> Res<Out> {
    let patch = load_patch(path, perf)?;
    let audio = render_patch(&patch)?;
    let target = out.map_or_else(|| default_wav_path(path), Path::to_path_buf);
    let bytes = wav_bytes(&audio);
    write_bytes(&target, &bytes)?;
    let peak = audio.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    let mut m = Map::new();
    m.insert("patch".into(), json!(path_text(path)));
    m.insert("wav".into(), json!(path_text(&target)));
    m.insert("sample_rate".into(), json!(SAMPLE_RATE as u32));
    m.insert(
        "dur_s".into(),
        n(audio.len() as f64 / f64::from(SAMPLE_RATE)),
    );
    m.insert("bytes".into(), json!(bytes.len()));
    m.insert(
        "peak_dbfs".into(),
        n(20.0 * f64::from(peak).max(1e-6).log10()),
    );
    m.insert("clipping".into(), json!(peak >= 0.9999));
    Ok(done(with_schema("render", m)))
}

/// Samples, sample rate and note-off time of whatever `describe`/`check` was given.
fn audio_of(input: &Input) -> Res<(Vec<f32>, u32, Option<f64>)> {
    Ok(match input {
        Input::Patch(p) => (render_patch(p)?, SAMPLE_RATE as u32, Some(p.perf.hold_s)),
        Input::Wav {
            samples,
            rate,
            hold_s,
        } => (samples.clone(), *rate, *hold_s),
    })
}

pub fn cmd_describe(path: &Path, explain: bool, perf: Option<&str>) -> Res<Out> {
    let input = load_input(path, perf)?;
    let (audio, rate, hold) = audio_of(&input)?;
    let mut describer = Describer::new(rate as f32);
    let d = describer.describe(&audio, hold, Want::ALL);
    let Value::Object(mut m) = describe_json(&d, explain) else {
        unreachable!("describe_json returns an object")
    };
    m.insert("source".into(), json!(path_text(path)));
    Ok(done(with_schema("describe", m)))
}

pub fn cmd_check(path: &Path, expect: &str, perf: Option<&str>) -> Res<Out> {
    let expects = parse_list(expect)?;
    let input = load_input(path, perf)?;
    let (audio, rate, hold) = audio_of(&input)?;
    let mut describer = Describer::new(rate as f32);
    let d = describer.describe(&audio, hold, want_for(&expects));
    let patch = match &input {
        Input::Patch(p) => Some(p.as_ref()),
        Input::Wav { .. } => None,
    };
    let (Value::Object(mut m), pass) = run_checks(&expects, &d, patch, &mut describer)? else {
        unreachable!("run_checks returns an object")
    };
    m.insert("source".into(), json!(path_text(path)));
    Ok(Out {
        json: with_schema("check", m),
        exit: if pass { 0 } else { EXIT_CHECK_FAILED },
    })
}

pub fn cmd_step(
    path: &Path,
    sets: &[String],
    expect: Option<&str>,
    out: Option<&Path>,
    wav: Option<&Path>,
    perf: Option<&str>,
) -> Res<Out> {
    let base = load_patch(path, None)?;
    let (mut next, changes) = base.apply_sets(sets)?;
    if let Some(p) = perf {
        apply_perf(&mut next, p, true)?;
    }
    // Everything that can be wrong with the arguments is checked before any file is written.
    let expects = expect.map(parse_list).transpose()?;
    let target = out.unwrap_or(path);
    write_patch(target, &next)?;
    let audio = render_patch(&next)?;
    let wav_path = wav.map_or_else(|| default_wav_path(target), Path::to_path_buf);
    write_bytes(&wav_path, &wav_bytes(&audio))?;
    let mut describer = Describer::new(SAMPLE_RATE);
    let d = describer.describe(&audio, Some(next.perf.hold_s), Want::ALL);
    let mut m = Map::new();
    m.insert("patch".into(), json!(path_text(target)));
    m.insert("wav".into(), json!(path_text(&wav_path)));
    m.insert("applied".into(), changes_json(&changes));
    m.insert("describe".into(), describe_json(&d, false));
    let mut exit = 0;
    if let Some(expects) = expects {
        let (check, pass) = run_checks(&expects, &d, Some(&next), &mut describer)?;
        if !pass {
            exit = EXIT_CHECK_FAILED;
        }
        m.insert("check".into(), check);
    }
    Ok(Out {
        json: with_schema("step", m),
        exit,
    })
}

/// Check a patch against an expectation list, with hints (library entry point; the JSON is what `check` prints
/// minus its `schema` and `source`). Returns the object and whether everything passed.
pub fn check_patch(patch: &Patch, expect: &str) -> Res<(Value, bool)> {
    let expects = parse_list(expect)?;
    let mut describer = Describer::new(SAMPLE_RATE);
    let d = measure(patch, &mut describer, want_for(&expects))?;
    run_checks(&expects, &d, Some(patch), &mut describer)
}

/// Measure a patch without files (library entry point for tests and tools).
pub fn describe_patch(patch: &Patch, want: Want) -> Res<Descriptors> {
    let mut describer = Describer::new(SAMPLE_RATE);
    measure(patch, &mut describer, want)
}
