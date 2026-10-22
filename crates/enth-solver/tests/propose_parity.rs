//! Parity with ONNX Runtime (lane ONX; phase-3's ONX acceptance: "parity with ONNX Runtime within 1e-5 on exported
//! fixtures") and, from lane EXP, with PyTorch (phase-3's TR acceptance: "parity between PyTorch and our kernels on
//! fixtures"; the second test below). Each ONNX fixture under `tests/fixtures/propose/` is an ONNX graph that `ml/enth_ml/export/parity.py` built
//! with `onnx.helper`, ran through ONNX Runtime (one thread, no graph optimisation) and converted to `ENTHMODL`:
//! `<name>.bin` is the model (its info table names the input seed) and `<name>.expected` ONNX Runtime's outputs
//! (`ENTHTNS1`). Our kernels must agree within [`TOLERANCE`] on every output, and their own output hash is pinned, on
//! every CI leg (the fixtures are compiled in, so wasm32 needs no file access).
//!
//! Regenerate (only when a fixture changes on purpose): `cd ml && uv run --group parity python -m
//! enth_ml.export.parity --out ../crates/enth-solver/tests/fixtures/propose`; the PyTorch fixture needs torch
//! (`uv run --group train python -m enth_ml.export.propose fixture --out ../crates/enth-solver/tests/fixtures/propose`).

mod propose_support;

use enth_solver::propose::net::{Net, synthetic_input};
use enth_solver::propose::pack::ModelPack;
use propose_support::{hex, read_tensors};

/// The largest absolute difference allowed between our kernels and ONNX Runtime.
const TOLERANCE: f32 = 1e-5;

/// Each fixture: its name, its model, ONNX Runtime's outputs, and our pinned output SHA-256.
const FIXTURES: [(&str, &[u8], &[u8], &str); 4] = [
    (
        "onnx_conv",
        include_bytes!("fixtures/propose/onnx_conv.bin"),
        include_bytes!("fixtures/propose/onnx_conv.expected"),
        "202512685112c7803db50485b413b05586e3ba2e560f737709f9cdc53d03566a",
    ),
    (
        "onnx_conv1d",
        include_bytes!("fixtures/propose/onnx_conv1d.bin"),
        include_bytes!("fixtures/propose/onnx_conv1d.expected"),
        "8338c469a55854aaaf899f70ee1e5150ed7f79e92c838f6b33323ffd3efe2db2",
    ),
    (
        "onnx_dense",
        include_bytes!("fixtures/propose/onnx_dense.bin"),
        include_bytes!("fixtures/propose/onnx_dense.expected"),
        "99ce9212e47c12e20e697569c814cbc5fbf634b51ce3ea4516539701d2ce1570",
    ),
    (
        "onnx_attention",
        include_bytes!("fixtures/propose/onnx_attention.bin"),
        include_bytes!("fixtures/propose/onnx_attention.expected"),
        "dc3592e6a550e6799b69fa7fa18e7ea209702a9a5203c59d3d4f00605da01303",
    ),
];

/// Our outputs against ONNX Runtime's on every fixture, and our output hash against its pin; every result at once.
#[test]
fn our_kernels_match_onnx_runtime_within_1e_5() {
    use core::fmt::Write as _;
    let mut report = String::new();
    let mut bad = false;
    for (name, model, expected, pin) in FIXTURES {
        let pack = ModelPack::parse(model).unwrap_or_else(|e| panic!("{name}: {e}"));
        let net = Net::build(&pack).unwrap_or_else(|e| panic!("{name}: {e}"));
        let seed: u64 = pack
            .info_value("parity.seed")
            .and_then(|s| s.parse().ok())
            .unwrap_or_else(|| panic!("{name} names no parity.seed"));
        let out = net.run(&synthetic_input(seed)).unwrap();
        let mut worst = 0.0f32;
        for t in read_tensors(expected) {
            let ours = out
                .get(&t.name)
                .unwrap_or_else(|| panic!("{name}: no output {}", t.name));
            assert_eq!(ours.dims, t.dims, "{name}: {}", t.name);
            let mut w = 0.0f32;
            let mut mag = 0.0f32;
            for (a, b) in ours.values.iter().zip(&t.values) {
                let d = (a - b).abs();
                assert!(d.is_finite(), "{name}: {} has a non-finite value", t.name);
                w = w.max(d);
                mag = mag.max(b.abs());
            }
            println!(
                "  {name}.{}: max diff {w:.3e}, max |value| {mag:.3}",
                t.name
            );
            worst = worst.max(w);
        }
        let h = hex(&out.sha256());
        bad |= worst > TOLERANCE || h != pin;
        let _ = writeln!(
            report,
            "{name}: max |ours - onnxruntime| = {worst:.3e}; output sha256 {h} ({} FLOPs)",
            net.flops()
        );
    }
    println!("{report}");
    assert!(!bad, "parity or a pinned hash failed:\n{report}");
}

/// Lane EXP's PyTorch fixture: a randomly initialised network of propose v0's shape (about 10 k parameters), exported by
/// `enth_ml.export.propose` the way the pilot model is (`BatchNorm` folded, matrices in `f16`, the standardisation in
/// the graph), and PyTorch's outputs on the synthetic inputs of its `parity.seeds`. Our kernels must agree with them
/// within [`PYTORCH_TOLERANCE`], and `OutputLayout::decode` must read the same patch from every row of both: the same
/// modules, categoricals and routes, and values within [`DECODE_TOLERANCE`] in the normalised domain.
const PYTORCH_TOLERANCE: f32 = 1e-5;
const DECODE_TOLERANCE: f32 = 1e-4;
const PYTORCH_MODEL: &[u8] = include_bytes!("fixtures/propose/exp_tiny.bin");
const PYTORCH_EXPECTED: &[u8] = include_bytes!("fixtures/propose/exp_tiny.expected");
/// SHA-256 over the per-sample output hashes of the seeds, in order (ours; bit-exact on every platform).
const PYTORCH_PIN: &str = "64d32d6af67449642cae678db2dbd6619951f13490ea47d78ddfac4e2949eafd";

#[test]
fn our_kernels_match_pytorch_and_decode_the_same_patches() {
    use enth_solver::create::TargetPerf;
    use enth_solver::propose::decode::{Hearing, OutputLayout};
    use enth_solver::propose::features::Cue;
    use sha2::{Digest, Sha256};

    let pack = ModelPack::parse(PYTORCH_MODEL).unwrap();
    let net = Net::build(&pack).unwrap();
    let h = pack.header();
    assert_eq!(h.flops, net.flops(), "the header's FLOPs are what the shapes give");
    let seeds: Vec<u64> = pack
        .info_value("parity.seeds")
        .expect("the fixture names its seeds")
        .split(',')
        .map(|s| s.parse().unwrap())
        .collect();
    let runs: Vec<_> = seeds
        .iter()
        .map(|s| (synthetic_input(*s), net.run(&synthetic_input(*s)).unwrap()))
        .collect();
    let mut pin = Sha256::new();
    for (_, out) in &runs {
        pin.update(out.sha256());
    }
    assert_eq!(hex(&pin.finalize()), PYTORCH_PIN, "our output hashes moved");

    let expected = read_tensors(PYTORCH_EXPECTED);
    let find = |name: &str| expected.iter().find(|t| t.name == name).unwrap();
    let (k, w) = (h.hypotheses as usize, h.width as usize);
    assert_eq!(find("hypotheses").dims, [seeds.len() as u32, k as u32, w as u32]);
    assert_eq!(find("confidence").dims, [seeds.len() as u32, k as u32]);
    let mut worst = 0.0f32;
    for (n, (_, out)) in runs.iter().enumerate() {
        for (name, ours, len) in [
            ("hypotheses", out.hypotheses(), k * w),
            ("confidence", out.confidence(), k),
        ] {
            let theirs = &find(name).values[n * len..(n + 1) * len];
            for (a, b) in ours.iter().zip(theirs) {
                let d = (a - b).abs();
                assert!(d.is_finite(), "{name} of seed {} is not finite", seeds[n]);
                worst = worst.max(d);
            }
        }
    }
    println!("  exp_tiny: max |ours - pytorch| = {worst:.3e}");
    assert!(worst <= PYTORCH_TOLERANCE, "{worst:.3e} against PyTorch");

    // The decoded hypotheses: each sample is heard as a create job would hear it (the context's key and pitch, one note,
    // a 3 s window), as told and with the key moved by 3.
    let layout = OutputLayout::build(h.layout, h.generator, h.template).unwrap();
    let mut rows = 0;
    for (n, (input, out)) in runs.iter().enumerate() {
        let c = input.context;
        let key = (c[0].round() as i32 + 60).clamp(0, 127);
        let f0_hz = (c[2] > 0.5)
            .then(|| 440.0 * ((f64::from(key) - 69.0) / 12.0 + f64::from(c[1]) / 12.0).exp2());
        let hold_s = f64::from(c[3]).clamp(0.08, 4.0);
        let cue = Cue { key: key as u8, f0_hz, on_s: 0.007, hold_s, bandwidth_hz: None };
        for shift in [0, 3] {
            let perf = TargetPerf {
                key: (key + shift) as u8,
                velocity: 100.0 / 127.0,
                on_s: 0.007,
                hold_s,
                duration_s: 3.0,
                f0_hz,
            };
            let hearing = Hearing::job(&cue, &perf).unwrap();
            for r in 0..k {
                let at = (n * k + r) * w;
                let a = layout.decode(out.hypothesis(r), &hearing);
                let b = layout.decode(&find("hypotheses").values[at..at + w], &hearing);
                let (a, b) = match (a, b) {
                    (Ok(a), Ok(b)) => (a, b),
                    (Err(a), Err(b)) => {
                        assert_eq!(a.code(), b.code(), "sample {n} row {r}");
                        continue;
                    }
                    (a, b) => panic!("sample {n} row {r}: ours {:?}, pytorch's {:?}", a.is_ok(), b.is_ok()),
                };
                rows += 1;
                let on = |p: &enth_patch::Patch| -> Vec<_> {
                    p.modules().map(|(_, m)| (m.id.clone(), m.enabled)).collect()
                };
                assert_eq!(on(&a), on(&b), "sample {n} row {r}: modules");
                for e in layout.categorical() {
                    assert_eq!(a.value(&e.addr), b.value(&e.addr), "sample {n} row {r}: {}", e.addr);
                }
                assert_eq!(a.routes.len(), b.routes.len(), "sample {n} row {r}: routes");
                for (x, y) in a.routes.iter().zip(&b.routes) {
                    assert!(x.src == y.src && x.dst == y.dst, "sample {n} row {r}: a route differs");
                    assert!((x.depth - y.depth).abs() / 100.0 <= DECODE_TOLERANCE);
                }
                for e in layout.continuous() {
                    let (x, y) = (a.value(&e.addr).unwrap(), b.value(&e.addr).unwrap());
                    let d = (e.spec.to_norm(x) - e.spec.to_norm(y)).abs();
                    assert!(d <= DECODE_TOLERANCE, "sample {n} row {r}: {} is {d:.3e} apart", e.addr);
                }
            }
        }
    }
    assert!(rows > 0, "some rows decode");
    println!("  exp_tiny: {rows} decoded rows agree with PyTorch's");
}
