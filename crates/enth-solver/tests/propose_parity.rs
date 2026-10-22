//! Parity with ONNX Runtime (lane ONX; phase-3's ONX acceptance: "parity with ONNX Runtime within 1e-5 on exported
//! fixtures"). Each fixture under `tests/fixtures/propose/` is an ONNX graph that `ml/enth_ml/export/parity.py` built
//! with `onnx.helper`, ran through ONNX Runtime (one thread, no graph optimisation) and converted to `ENTHMODL`:
//! `<name>.bin` is the model (its info table names the input seed) and `<name>.expected` ONNX Runtime's outputs
//! (`ENTHTNS1`). Our kernels must agree within [`TOLERANCE`] on every output, and their own output hash is pinned, on
//! every CI leg (the fixtures are compiled in, so wasm32 needs no file access).
//!
//! Regenerate (only when a fixture changes on purpose): `cd ml && uv run --group parity python -m
//! enth_ml.export.parity --out ../crates/enth-solver/tests/fixtures/propose`.

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
