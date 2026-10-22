//! `enth-datagen ds1`, `shards` and `exclude` as a user runs them (lane DG): the binary on small runs against a small
//! GR3 directory (the real lists' shape, no targets to render), so each run takes seconds.
//!
//! The shard reader below shares no code with the tool: it is what `ml/` does with the standard library.
#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "the tests run the tool's own binary in temporary directories"
)]

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;

const BLOCK: &str =
    "seed-range 770000000000..770999999999\nstream r3/test\nstream gr3/*\npath content/gr3/**\n";

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_enth-datagen"))
        .args(args)
        .output()
        .expect("the tool runs")
}

fn ok(args: &[&str]) -> String {
    let out = run(args);
    assert!(
        out.status.success(),
        "{args:?}: {}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn fails(args: &[&str]) -> String {
    let out = run(args);
    assert_eq!(out.status.code(), Some(1), "{args:?}: {out:?}");
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// A scratch directory of this test binary's process.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("enth-datagen-ds1-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// A GR3 directory with `synth` as its synth list (plus GR3's block) and `degraded` as its degraded list; no targets
/// file entries, so building the references renders nothing.
fn gr3(dir: &Path, synth: &str, degraded: &str) -> String {
    let g = dir.join("gr3");
    fs::create_dir_all(&g).unwrap();
    fs::write(g.join("exclude-synth.txt"), format!("{BLOCK}{synth}")).unwrap();
    fs::write(g.join("exclude-real.txt"), "").unwrap();
    fs::write(g.join("exclude-degraded.txt"), degraded).unwrap();
    fs::write(g.join("synth.toml"), "[[stream]]\nkey = \"r3/test\"\n").unwrap();
    g.to_string_lossy().into_owned()
}

/// A shard, read with nothing but its header: `(header, arrays by name)`.
fn read_shard(path: &Path) -> (Value, BTreeMap<String, Vec<u8>>) {
    let raw = fs::read(path).unwrap();
    assert_eq!(&raw[0..8], b"ENTHSHRD");
    assert_eq!(u32::from_le_bytes(raw[8..12].try_into().unwrap()), 1);
    let hlen = u32::from_le_bytes(raw[12..16].try_into().unwrap()) as usize;
    let header: Value = serde_json::from_slice(&raw[16..16 + hlen]).unwrap();
    let mut arrays = BTreeMap::new();
    for a in header["arrays"].as_array().unwrap() {
        let (off, len) = (
            a["offset"].as_u64().unwrap() as usize,
            a["bytes"].as_u64().unwrap() as usize,
        );
        assert_eq!(off % 64, 0);
        arrays.insert(
            a["name"].as_str().unwrap().to_owned(),
            raw[off..off + len].to_vec(),
        );
    }
    (header, arrays)
}

fn f32s(b: &[u8]) -> Vec<f32> {
    b.as_chunks::<4>()
        .0
        .iter()
        .map(|c| f32::from_le_bytes(*c))
        .collect()
}

fn u64s(b: &[u8]) -> Vec<u64> {
    b.as_chunks::<8>()
        .0
        .iter()
        .map(|c| u64::from_le_bytes(*c))
        .collect()
}

fn shard_files(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "bin"))
        .collect();
    v.sort();
    v
}

/// Every sample of a run: index -> (mel, context, label, mask, rate).
type Row = (Vec<f32>, Vec<f32>, Vec<f32>, Vec<f32>, u64);

fn rows(dir: &Path) -> BTreeMap<u64, Row> {
    let mut out = BTreeMap::new();
    for f in shard_files(dir) {
        let (h, a) = read_shard(&f);
        let n = h["n"].as_u64().unwrap() as usize;
        let (mel, ctx, label, mask) = (
            f32s(&a["mel"]),
            f32s(&a["context"]),
            f32s(&a["label"]),
            f32s(&a["mask"]),
        );
        let (index, rate) = (
            u64s(&a["index"]),
            a["rate"]
                .as_chunks::<4>()
                .0
                .iter()
                .map(|c| u64::from(u32::from_le_bytes(*c)))
                .collect::<Vec<_>>(),
        );
        assert_eq!(
            (mel.len(), ctx.len(), label.len(), mask.len(), index.len()),
            (n * 24_576, n * 5, n * 102, n * 102, n)
        );
        for i in 0..n {
            out.insert(
                index[i],
                (
                    mel[i * 24_576..(i + 1) * 24_576].to_vec(),
                    ctx[i * 5..(i + 1) * 5].to_vec(),
                    label[i * 102..(i + 1) * 102].to_vec(),
                    mask[i * 102..(i + 1) * 102].to_vec(),
                    rate[i],
                ),
            );
        }
    }
    out
}

fn manifest(dir: &Path) -> Value {
    serde_json::from_slice(&fs::read(dir.join("manifest.json")).unwrap()).unwrap()
}

#[test]
fn one_worker_and_many_write_the_same_bytes_and_the_files_read_back() {
    let d = scratch("workers");
    let g = gr3(&d, "", "");
    let (a, b) = (d.join("a"), d.join("b"));
    for (out, workers) in [(&a, "1"), (&b, "3")] {
        ok(&[
            "ds1",
            "gen",
            "--count",
            "24",
            "--seed",
            "9",
            "--out",
            out.to_str().unwrap(),
            "--workers",
            workers,
            "--shard-size",
            "8",
            "--gr3",
            &g,
        ]);
    }
    let (fa, fb) = (shard_files(&a), shard_files(&b));
    assert_eq!(fa.len(), 3);
    for (x, y) in fa.iter().zip(&fb) {
        assert_eq!(
            fs::read(x).unwrap(),
            fs::read(y).unwrap(),
            "{}",
            x.display()
        );
    }
    assert_eq!(
        fs::read(a.join("manifest.json")).unwrap(),
        fs::read(b.join("manifest.json")).unwrap()
    );

    // The manifest says what a model's header copies, and the totals add up.
    let m = manifest(&a);
    assert_eq!(m["schema"], "enth.ds1-shards/1");
    assert_eq!(
        (m["ds1"].as_u64(), m["kind"].as_str(), m["role"].as_str()),
        (Some(1), Some("clean"), Some("train"))
    );
    assert_eq!(m["feature_set"]["id"], 1);
    assert_eq!(m["layout"]["id"], 1);
    assert_eq!(m["layout"]["width"], 102);
    assert_eq!(m["layout"]["sha256"].as_str().unwrap().len(), 64);
    assert_eq!(m["layout"]["entries"].as_array().unwrap().len(), 102);
    assert_eq!(m["jnd"]["scale"].as_array().unwrap().len(), 102);
    assert_eq!(m["jnd"]["table"].as_array().unwrap().len(), 33);
    assert_eq!(m["exclusions"]["sha256"].as_str().unwrap().len(), 64);
    let kept = m["kept"].as_u64().unwrap();
    let skipped: u64 = m["skips"]
        .as_object()
        .unwrap()
        .values()
        .map(|v| v.as_u64().unwrap())
        .sum();
    assert_eq!(kept + skipped, 24);
    assert!(kept >= 18, "{m}");
    let all = rows(&a);
    assert_eq!(all.len() as u64, kept);
    for (index, (mel, ctx, label, mask, rate)) in &all {
        assert!(
            index < &24 && [48_000, 44_100, 22_050, 16_000].contains(rate),
            "{index} {rate}"
        );
        assert!(mel.iter().chain(ctx).chain(label).all(|v| v.is_finite()));
        assert!(mask.iter().all(|v| *v == 0.0 || *v == 1.0));
        assert!(
            label
                .iter()
                .zip(mask)
                .all(|(l, m)| *m == 0.0 || (-0.001..=1.001).contains(l)),
            "{index}"
        );
        assert!(mask.iter().sum::<f32>() > 40.0);
    }
    // verify re-hashes everything; a flipped byte is caught.
    assert!(ok(&["shards", "verify", a.to_str().unwrap()]).starts_with("ok: 3 shards"));
    let mut bytes = fs::read(&fa[1]).unwrap();
    bytes[2_000] ^= 1;
    fs::write(&fa[1], bytes).unwrap();
    assert!(fails(&["shards", "verify", a.to_str().unwrap()]).contains("not the manifest's"));
    fs::remove_dir_all(&d).unwrap();
}

#[test]
fn a_rerun_resumes_and_repairs_shards_to_the_same_bytes() {
    let d = scratch("resume");
    let g = gr3(&d, "", "");
    let out = d.join("run");
    let args = [
        "ds1",
        "gen",
        "--count",
        "16",
        "--seed",
        "4",
        "--out",
        out.to_str().unwrap(),
        "--shard-size",
        "8",
        "--gr3",
        &g,
    ];
    ok(&args);
    let before: Vec<Vec<u8>> = shard_files(&out)
        .iter()
        .map(|f| fs::read(f).unwrap())
        .collect();
    let manifest_before = fs::read(out.join("manifest.json")).unwrap();
    // Nothing to do: both shards resume (the tool says so on stderr and computes nothing).
    let again = run(&args);
    assert!(again.status.success());
    let stderr = String::from_utf8_lossy(&again.stderr);
    assert_eq!(stderr.matches("(resumed)").count(), 2, "{stderr}");
    assert!(!String::from_utf8_lossy(&again.stdout).contains("computed"));
    // A damaged shard is recomputed, to the same bytes; an intact one is left alone.
    fs::write(shard_files(&out)[0].clone(), b"junk").unwrap();
    let repaired = run(&args);
    let stderr = String::from_utf8_lossy(&repaired.stderr);
    assert_eq!(stderr.matches("(resumed)").count(), 1, "{stderr}");
    let after: Vec<Vec<u8>> = shard_files(&out)
        .iter()
        .map(|f| fs::read(f).unwrap())
        .collect();
    assert_eq!(before, after);
    assert_eq!(
        manifest_before,
        fs::read(out.join("manifest.json")).unwrap()
    );
    // Another seed is another run: the same directory is rewritten, not resumed.
    let other = run(&[
        "ds1",
        "gen",
        "--count",
        "16",
        "--seed",
        "5",
        "--out",
        out.to_str().unwrap(),
        "--shard-size",
        "8",
        "--gr3",
        &g,
    ]);
    assert_eq!(
        String::from_utf8_lossy(&other.stderr)
            .matches("(resumed)")
            .count(),
        0
    );
    assert_ne!(
        before,
        shard_files(&out)
            .iter()
            .map(|f| fs::read(f).unwrap())
            .collect::<Vec<_>>()
    );
    fs::remove_dir_all(&d).unwrap();
}

/// `ds1 show`'s first JSON object.
fn show(index: u64, seed: u64, g: &str) -> Value {
    let text = ok(&[
        "ds1",
        "show",
        "--index",
        &index.to_string(),
        "--seed",
        &seed.to_string(),
        "--gr3",
        g,
    ]);
    let end = text.find("\n}\n").expect("a JSON object") + 2;
    serde_json::from_str(&text[..end]).unwrap()
}

#[test]
fn an_excluded_item_cannot_enter_a_shard() {
    let d = scratch("excluded");
    let open = gr3(&d, "", "");
    let base = d.join("base");
    ok(&[
        "ds1",
        "gen",
        "--count",
        "40",
        "--seed",
        "11",
        "--out",
        base.to_str().unwrap(),
        "--shard-size",
        "20",
        "--gr3",
        &open,
    ]);
    let kept = rows(&base);
    // Three kept generator-v1 samples: list the first's seed, the second's code and the third's fingerprint.
    let v1: Vec<(u64, Value)> = kept
        .keys()
        .map(|i| (*i, show(*i, 11, &open)))
        .filter(|(_, v)| !v["generator"].is_null())
        .take(3)
        .collect();
    assert_eq!(v1.len(), 3);
    let (seed, code, fingerprint) = (
        v1[0].1["generator"]["seed"].as_u64().unwrap(),
        v1[1].1["generator"]["code"].as_str().unwrap().to_owned(),
        v1[2].1["generator"]["fingerprint"]
            .as_str()
            .unwrap()
            .to_owned(),
    );
    let listed = gr3(
        &d.join("lists"),
        &format!("seed-range {seed}..{seed}\ncode {code}\nfingerprint {fingerprint}\n"),
        "",
    );
    let run1 = d.join("listed");
    ok(&[
        "ds1",
        "gen",
        "--count",
        "40",
        "--seed",
        "11",
        "--out",
        run1.to_str().unwrap(),
        "--shard-size",
        "20",
        "--gr3",
        &listed,
    ]);
    let after = rows(&run1);
    for (index, _) in &v1 {
        assert!(
            kept.contains_key(index) && !after.contains_key(index),
            "sample {index} entered a shard"
        );
    }
    // Everything else is the same sample as before.
    for (index, row) in &after {
        assert_eq!(kept.get(index), Some(row), "sample {index}");
    }
    let m = manifest(&run1);
    for code in ["excluded_seed", "excluded_code", "excluded_fingerprint"] {
        assert!(m["skips"][code].as_u64().unwrap() >= 1, "{code}: {m}");
    }
    // The check passes on the run made against these lists, and refuses the run made against others.
    assert!(
        ok(&["exclude", "check", run1.to_str().unwrap(), "--gr3", &listed]).starts_with("clean: ")
    );
    assert!(
        fails(&["exclude", "check", base.to_str().unwrap(), "--gr3", &listed])
            .contains("other exclusion lists")
    );
    // A listed stream is not drawn, except the test stream, which is the one the lists reserve.
    assert!(
        fails(&[
            "ds1",
            "gen",
            "--count",
            "4",
            "--seed",
            "1",
            "--out",
            d.join("x").to_str().unwrap(),
            "--stream",
            "gr3/dev",
            "--gr3",
            &open
        ])
        .contains("exclusion lists")
    );
    let test = d.join("test");
    ok(&[
        "ds1",
        "gen",
        "--count",
        "8",
        "--seed",
        "1",
        "--out",
        test.to_str().unwrap(),
        "--stream",
        "r3/test",
        "--gr3",
        &open,
    ]);
    assert_eq!(manifest(&test)["role"], "test");
    // Train against test: no label of the training run is within a step of the test run's (the dedup the README asks for).
    let train = d.join("train");
    ok(&[
        "ds1",
        "gen",
        "--count",
        "8",
        "--seed",
        "2",
        "--out",
        train.to_str().unwrap(),
        "--gr3",
        &open,
    ]);
    assert!(
        ok(&[
            "exclude",
            "check",
            train.to_str().unwrap(),
            "--gr3",
            &open,
            "--against",
            test.to_str().unwrap()
        ])
        .contains("1 other run")
    );
    // The same stream and seed is not "another run".
    assert!(
        fails(&[
            "exclude",
            "check",
            train.to_str().unwrap(),
            "--gr3",
            &open,
            "--against",
            train.to_str().unwrap()
        ])
        .contains("same stream and seed")
    );
    fs::remove_dir_all(&d).unwrap();
}

/// A 16-bit mono WAV of `x` at `rate`.
fn pcm16(rate: u32, x: &[i16]) -> Vec<u8> {
    let mut b = b"RIFF".to_vec();
    b.extend_from_slice(&(36 + 2 * x.len() as u32).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    for v in [16u32, 0x0001_0001, rate, rate * 2, 0x0010_0002] {
        b.extend_from_slice(&v.to_le_bytes());
    }
    b.extend_from_slice(b"data");
    b.extend_from_slice(&(2 * x.len() as u32).to_le_bytes());
    for s in x {
        b.extend_from_slice(&s.to_le_bytes());
    }
    b
}

#[test]
fn the_bridge_reproduces_the_clean_rows_and_reads_degraded_files() {
    let d = scratch("bridge");
    let g = gr3(&d, "", "degrade-pool heldout\n");
    let clean = d.join("clean");
    ok(&[
        "ds1",
        "gen",
        "--count",
        "6",
        "--seed",
        "21",
        "--out",
        clean.to_str().unwrap(),
        "--shard-size",
        "6",
        "--gr3",
        &g,
    ]);
    let r = d.join("render");
    let said = ok(&[
        "ds1",
        "render",
        "--count",
        "6",
        "--seed",
        "21",
        "--out",
        r.to_str().unwrap(),
        "--workers",
        "2",
        "--gr3",
        &g,
    ]);
    assert!(said.starts_with("wrote "), "{said}");
    let samples = r.join("samples.jsonl");
    let lines = fs::read_to_string(&samples).unwrap();
    assert!(lines.lines().count() >= 5, "{lines}");
    let feat = d.join("features");
    let said = ok(&[
        "shards",
        "features",
        "--samples",
        samples.to_str().unwrap(),
        "--clips",
        r.to_str().unwrap(),
        "--out",
        feat.to_str().unwrap(),
        "--shard-size",
        "4",
        "--gr3",
        &g,
    ]);
    assert!(said.contains("manifest sha256"), "{said}");
    assert_eq!(manifest(&feat)["kind"], "clips");
    assert!(ok(&["shards", "verify", feat.to_str().unwrap()]).starts_with("ok: "));
    // The undegraded clips give the rows `gen` wrote: the same mel, context, label and mask, bit for bit.
    let (a, b) = (rows(&clean), rows(&feat));
    assert!(!b.is_empty());
    for (index, row) in &b {
        assert_eq!(a.get(index), Some(row), "sample {index}");
    }
    assert_eq!(a.len(), b.len());

    // A degraded copy: 6 dB quieter, mono, 16-bit. It is read, heard by the intake at its own level, and labelled; the
    // clip's rate is the file's.
    let (index, wav) = lines
        .lines()
        .map(|l| serde_json::from_str::<Value>(l).unwrap())
        .find(|l| a.contains_key(&l["index"].as_u64().unwrap()))
        .map(|l| {
            (
                l["index"].as_u64().unwrap(),
                fs::read(r.join(l["clip"].as_str().unwrap())).unwrap(),
            )
        })
        .unwrap();
    let rate = u32::from_le_bytes(wav[24..28].try_into().unwrap());
    let st = f32s(&wav[44..]);
    let mono: Vec<i16> = st
        .as_chunks::<2>()
        .0
        .iter()
        .map(|f| (0.5 * f32::midpoint(f[0], f[1]) * 32_767.0) as i16)
        .collect();
    let deg = d.join("deg");
    fs::create_dir_all(&deg).unwrap();
    fs::write(deg.join("one.wav"), pcm16(rate, &mono)).unwrap();
    let line = serde_json::json!({"stream": "r3/train", "run_seed": 21, "index": index, "clip": "one.wav", "degrade": {"pool": "train", "profile": "gain"}});
    fs::write(deg.join("samples.jsonl"), format!("{line}\n")).unwrap();
    let out = d.join("deg-features");
    ok(&[
        "shards",
        "features",
        "--samples",
        deg.join("samples.jsonl").to_str().unwrap(),
        "--clips",
        deg.to_str().unwrap(),
        "--out",
        out.to_str().unwrap(),
        "--gr3",
        &g,
    ]);
    let got = rows(&out);
    assert_eq!(got.len(), 1, "{}", manifest(&out));
    let (mel, _, label, mask, got_rate) = &got[&index];
    assert_eq!(*got_rate, u64::from(rate));
    assert!(
        mel.iter().all(|v| v.is_finite()) && label.len() == 102 && mask.iter().sum::<f32>() > 40.0
    );

    // A record from the held-out pool stops the run (a leak is an error, not a skip), and so does a path under GR3.
    let bad = serde_json::json!({"stream": "r3/train", "run_seed": 21, "index": index, "clip": "one.wav", "degrade": {"pool": "heldout"}});
    fs::write(deg.join("bad.jsonl"), format!("{bad}\n")).unwrap();
    let stderr = fails(&[
        "shards",
        "features",
        "--samples",
        deg.join("bad.jsonl").to_str().unwrap(),
        "--clips",
        deg.to_str().unwrap(),
        "--out",
        d.join("never").to_str().unwrap(),
        "--gr3",
        &g,
    ]);
    assert!(stderr.contains("degrade-pool"), "{stderr}");
    let mixed = format!(
        "{line}\n{}\n",
        serde_json::json!({"stream": "r3/dev", "run_seed": 21, "index": 1, "clip": "one.wav"})
    );
    fs::write(deg.join("mixed.jsonl"), mixed).unwrap();
    let stderr = fails(&[
        "shards",
        "features",
        "--samples",
        deg.join("mixed.jsonl").to_str().unwrap(),
        "--clips",
        deg.to_str().unwrap(),
        "--out",
        d.join("never").to_str().unwrap(),
        "--gr3",
        &g,
    ]);
    assert!(stderr.contains("one stream and one run seed"), "{stderr}");
    let escape = serde_json::json!({"stream": "r3/train", "run_seed": 21, "index": index, "clip": "../one.wav"});
    fs::write(deg.join("escape.jsonl"), format!("{escape}\n")).unwrap();
    let stderr = fails(&[
        "shards",
        "features",
        "--samples",
        deg.join("escape.jsonl").to_str().unwrap(),
        "--clips",
        deg.to_str().unwrap(),
        "--out",
        d.join("never").to_str().unwrap(),
        "--gr3",
        &g,
    ]);
    assert!(stderr.contains("below --clips"), "{stderr}");
    fs::remove_dir_all(&d).unwrap();
}
