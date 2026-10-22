//! Our scalar kernels' output hashes (lane ONX; docs/r3.md, the determinism contract, point 1): hand-built model
//! files covering every op of op set 1, each run on a synthetic input, with the SHA-256 of their outputs pinned. The
//! same hashes on every platform (the golden workflow runs this file on Linux x86-64 and aarch64, Windows, macOS
//! arm64 and wasm32) and under every FTZ/DAZ setting the target has (FTZ off, FTZ only, FTZ+DAZ on x86-64; FZ off and
//! on on aarch64), as renders are. A hash that moves is a change to a kernel's arithmetic or summation order: a
//! deliberate one updates the pin and says so in the commit (old models' output goldens move, replays do not).
//!
//! The `underflow` model drives products, sums, sigmoids, softmaxes and GELUs into the subnormal range on purpose:
//! it is the test that the kernels flush in code rather than depend on the host's flags.

mod propose_support;

use enth_solver::propose::net::{Net, synthetic_input};
use enth_solver::propose::pack::{ModelBuilder, ModelPack, OpCode, Val};
use propose_support::{hex, modes, values, with_mode};

fn eps_bits(eps: f32) -> i32 {
    eps.to_bits() as i32
}

/// The two required heads on `context` and on an optional feature vector `feat` of `n` values: `hypotheses`
/// `[K, 102]` (an `f16` weight) and `confidence` `[K]`.
fn heads(b: &mut ModelBuilder, k: u32, feat: Option<(Val, u32)>) {
    let (x, n) = match feat {
        Some((v, n)) => (b.node(OpCode::Concat, &[v, Val::CONTEXT], &[0]), n + 5),
        None => (Val::CONTEXT, 5),
    };
    let w = b.tensor_f16(
        "head.hyp.w",
        &[k * 102, n],
        &values(900, (k * 102 * n) as usize, 0.25),
    );
    let bias = b.tensor(
        "head.hyp.b",
        &[k * 102],
        &values(901, (k * 102) as usize, 0.5),
    );
    let h = b.node(OpCode::Linear, &[x, w, bias], &[]);
    let r = b.node(OpCode::Reshape, &[h], &[k as i32, -1]);
    b.output("hypotheses", r, &[k, 102]);
    let w = b.tensor("head.conf.w", &[k, n], &values(902, (k * n) as usize, 0.25));
    let c = b.node(OpCode::Linear, &[x, w], &[]);
    b.output("confidence", c, &[k]);
}

/// The K = 8 heads alone.
fn heads_only(b: &mut ModelBuilder) {
    heads(b, 8, None);
}

/// Two 2-D convolutions (stride, padding, dilation, groups, bias), `ReLU`, both 2-D pools, both global pools.
fn conv_stack(b: &mut ModelBuilder) {
    let s = b.node(OpCode::Slice, &[Val::MEL], &[1, 0, 32, 1]); // [3, 32, 64]
    let w = b.tensor("c1.w", &[8, 3, 3, 3], &values(10, 216, 0.25));
    let bias = b.tensor("c1.b", &[8], &values(11, 8, 0.125));
    let c1 = b.node(OpCode::Conv2d, &[s, w, bias], &[2, 1, 1, 1, 1, 1, 1, 1, 1]); // [8, 16, 64]
    let r = b.node(OpCode::Relu, &[c1], &[]);
    let w = b.tensor_f16("c2.w", &[8, 2, 3, 3], &values(12, 144, 0.5));
    let c2 = b.node(OpCode::Conv2d, &[r, w], &[1, 2, 0, 1, 2, 1, 2, 1, 4]); // [8, 14, 32]
    let m = b.node(OpCode::MaxPool2d, &[c2], &[2, 3, 2, 2, 0, 1, 1, 1]); // [8, 7, 16]
    let a = b.node(OpCode::AvgPool2d, &[m], &[3, 3, 1, 2, 1, 1, 1, 0]); // [8, 7, 8]
    let g = b.node(OpCode::GlobalAvgPool, &[a], &[]);
    let gm = b.node(OpCode::GlobalMaxPool, &[c2], &[]);
    b.output("a", a, &[8, 7, 8])
        .output("g", g, &[8, 1, 1])
        .output("gm", gm, &[8, 1, 1]);
    let f = b.node(OpCode::Reshape, &[g], &[8]);
    heads(b, 2, Some((f, 8)));
}

/// A 1-D convolution (stride, padding, dilation, groups), GELU, both 1-D pools.
fn conv1d_stack(b: &mut ModelBuilder) {
    let s = b.node(OpCode::Slice, &[Val::MEL], &[0, 1, 2, 1]); // [1, 128, 64]
    let x = b.node(OpCode::Reshape, &[s], &[16, 512]);
    let w = b.tensor("c.w", &[12, 4, 5], &values(20, 240, 0.25));
    let bias = b.tensor("c.b", &[12], &values(21, 12, 0.25));
    let c = b.node(OpCode::Conv1d, &[x, w, bias], &[3, 2, 4, 2, 4]); // [12, 170]
    let g = b.node(OpCode::Gelu, &[c], &[]);
    let mp = b.node(OpCode::MaxPool1d, &[g], &[4, 3, 1, 2]); // [12, 57]
    let ap = b.node(OpCode::AvgPool1d, &[mp], &[5, 2, 4, 0]); // [12, 29]
    b.output("ap", ap, &[12, 29]);
    heads(b, 1, None);
}

/// `LayerNorm`, `Linear` (stored and computed weights), the activations, broadcasting `Add`, `Mul`, `Sub`, both
/// softmax axes, `Transpose`, `MatMul` (two activations), `Concat` and a strided `Slice`.
fn dense_mix(b: &mut ModelBuilder) {
    let t = b.node(OpCode::Slice, &[Val::MEL], &[1, 5, 128, 37]); // rows 5, 42, 79, 116: [3, 4, 64]
    let x = b.node(OpCode::Reshape, &[t], &[12, 64]);
    let g = b.tensor("ln.g", &[64], &values(30, 64, 2.0));
    let be = b.tensor("ln.b", &[64], &values(31, 64, 0.5));
    let ln = b.node(OpCode::LayerNorm, &[x, g, be], &[eps_bits(1e-5)]);
    let w = b.tensor("lin.w", &[32, 64], &values(32, 2048, 0.25));
    let bias = b.tensor("lin.b", &[32], &values(33, 32, 0.25));
    let lin = b.node(OpCode::Linear, &[ln, w, bias], &[]); // [12, 32]
    let si = b.node(OpCode::Silu, &[lin], &[]);
    let th = b.node(OpCode::Tanh, &[lin], &[]);
    let sg = b.node(OpCode::Sigmoid, &[lin], &[]);
    let b2 = b.tensor("b2", &[32], &values(34, 32, 1.0));
    let ad = b.node(OpCode::Add, &[si, b2], &[]);
    let mu = b.node(OpCode::Mul, &[th, sg], &[]);
    let sb = b.node(OpCode::Sub, &[ad, mu], &[]);
    let sm0 = b.node(OpCode::Softmax, &[sb], &[0]);
    let sm1 = b.node(OpCode::Softmax, &[sb], &[1]);
    let tr = b.node(OpCode::Transpose, &[sm1], &[1, 0]); // [32, 12]
    let mm = b.node(OpCode::MatMul, &[sm0, tr], &[]); // [12, 12]
    let wm = b.tensor("mm.w", &[64, 12], &values(35, 768, 0.125));
    let mm2 = b.node(OpCode::MatMul, &[x, wm], &[]); // [12, 12]
    let cc = b.node(OpCode::Concat, &[mm, mm2], &[1]); // [12, 24]
    let wc = b.node(OpCode::Slice, &[cc], &[0, 0, 6, 1]); // [6, 24], a computed weight
    let lin2 = b.node(OpCode::Linear, &[cc, wc], &[]); // [12, 6]
    let r3 = b.node(OpCode::Reshape, &[mm], &[3, 4, 12]);
    let col = b.tensor("col", &[4, 1], &values(36, 4, 1.0));
    let b3 = b.node(OpCode::Mul, &[r3, col], &[]); // [3, 4, 12]
    b.output("lin2", lin2, &[12, 6])
        .output("b3", b3, &[3, 4, 12]);
    heads(b, 1, None);
}

/// A small CNN encoder with one attention block, as propose v0 is shaped (docs/r3.md): a strided convolution stem,
/// time tokens, `LayerNorm`, four-head self-attention as `Linear` + `Reshape` + `Transpose` + `MatMul` + `Softmax`, a
/// GELU MLP, residuals, mean pooling over time, then the K = 8 heads on the embedding and the context.
fn cnn_attention(b: &mut ModelBuilder) {
    let conv =
        |b: &mut ModelBuilder, x: Val, name: &str, co: u32, ci: u32, seed: u64, attrs: &[i32]| {
            let w = b.tensor(
                name,
                &[co, ci, 3, 3],
                &values(seed, (co * ci * 9) as usize, 0.25),
            );
            let bias = b.tensor(
                &format!("{name}.b"),
                &[co],
                &values(seed + 1, co as usize, 0.125),
            );
            b.node(OpCode::Conv2d, &[x, w, bias], attrs)
        };
    let c1 = conv(b, Val::MEL, "stem", 16, 3, 40, &[2, 2, 1, 1, 1, 1, 1, 1, 1]); // [16, 64, 32]
    let r1 = b.node(OpCode::Relu, &[c1], &[]);
    let c2 = conv(b, r1, "c2", 32, 16, 42, &[2, 2, 1, 1, 1, 1, 1, 1, 1]); // [32, 32, 16]
    let s2 = b.node(OpCode::Silu, &[c2], &[]);
    let tok = b.node(OpCode::Transpose, &[s2], &[1, 0, 2]); // [T 32, C 32, F 16]
    let tok = b.node(OpCode::Reshape, &[tok], &[32, 512]);
    let w = b.tensor_f16("proj.w", &[64, 512], &values(44, 64 * 512, 0.0625));
    let x = b.node(OpCode::Linear, &[tok, w], &[]); // [32, 64]
    let ln = |b: &mut ModelBuilder, x: Val, name: &str, seed: u64| {
        let g = b.tensor(&format!("{name}.g"), &[64], &values(seed, 64, 2.0));
        let be = b.tensor(&format!("{name}.b"), &[64], &values(seed + 1, 64, 0.25));
        b.node(OpCode::LayerNorm, &[x, g, be], &[eps_bits(1e-5)])
    };
    let lin = |b: &mut ModelBuilder, x: Val, name: &str, m: u32, n: u32, seed: u64| {
        let w = b.tensor(
            &format!("{name}.w"),
            &[m, n],
            &values(seed, (m * n) as usize, 0.25),
        );
        let bias = b.tensor(
            &format!("{name}.b"),
            &[m],
            &values(seed + 1, m as usize, 0.125),
        );
        b.node(OpCode::Linear, &[x, w, bias], &[])
    };
    let h = ln(b, x, "ln1", 46);
    let heads_of = |b: &mut ModelBuilder, v: Val| {
        let r = b.node(OpCode::Reshape, &[v], &[32, 4, 16]);
        b.node(OpCode::Transpose, &[r], &[1, 0, 2]) // [4, 32, 16]
    };
    let q = lin(b, h, "q", 64, 64, 48);
    let k = lin(b, h, "k", 64, 64, 50);
    let v = lin(b, h, "v", 64, 64, 52);
    let (q, k, v) = (heads_of(b, q), heads_of(b, k), heads_of(b, v));
    let kt = b.node(OpCode::Transpose, &[k], &[0, 2, 1]); // [4, 16, 32]
    let scores = b.node(OpCode::MatMul, &[q, kt], &[]); // [4, 32, 32]
    let scale = b.tensor("scale", &[1], &[0.25]);
    let scores = b.node(OpCode::Mul, &[scores, scale], &[]);
    let att = b.node(OpCode::Softmax, &[scores], &[2]);
    let ctx = b.node(OpCode::MatMul, &[att, v], &[]); // [4, 32, 16]
    let ctx = b.node(OpCode::Transpose, &[ctx], &[1, 0, 2]);
    let ctx = b.node(OpCode::Reshape, &[ctx], &[32, 64]);
    let o = lin(b, ctx, "o", 64, 64, 54);
    let x = b.node(OpCode::Add, &[x, o], &[]);
    let h = ln(b, x, "ln2", 56);
    let m1 = lin(b, h, "mlp1", 128, 64, 58);
    let m1 = b.node(OpCode::Gelu, &[m1], &[]);
    let m2 = lin(b, m1, "mlp2", 64, 128, 60);
    let x = b.node(OpCode::Add, &[x, m2], &[]);
    let xt = b.node(OpCode::Transpose, &[x], &[1, 0]); // [64, 32]
    let pooled = b.node(OpCode::GlobalAvgPool, &[xt], &[]); // [64, 1]
    let emb = b.node(OpCode::Reshape, &[pooled], &[64]);
    b.output("embedding", emb, &[64]);
    heads(b, 8, Some((emb, 64)));
}

/// Values pushed into the subnormal range on purpose: tiny products and sums, sigmoids of large negatives, a
/// softmax with a wide spread, GELU and `LayerNorm` of tiny values. Their hash is the same in every FTZ/DAZ mode only
/// because every result is flushed in code.
fn underflow(b: &mut ModelBuilder) {
    // mel · 2⁻¹²⁶ (the smallest normal): every product below 1 in magnitude is subnormal, about half of them.
    let tiny = b.tensor("tiny", &[1], &[f32::MIN_POSITIVE]);
    let t1 = b.node(OpCode::Mul, &[Val::MEL, tiny], &[]);
    let t1m = b.node(OpCode::GlobalAvgPool, &[t1], &[]);
    // mel · 2⁻¹¹², then a Linear with weights near 2⁻¹⁰: products on both sides of 2⁻¹²⁶, summed.
    let small = b.tensor("small", &[1], &[f32::from_bits(15 << 23)]);
    let t = b.node(OpCode::Mul, &[Val::MEL, small], &[]);
    let x = b.node(OpCode::Reshape, &[t], &[384, 64]);
    let w = b.tensor("w", &[16, 64], &values(70, 1024, 1.0 / 1024.0));
    let lin = b.node(OpCode::Linear, &[x, w], &[]); // products near 2⁻¹³⁰
    let g = b.node(OpCode::Gelu, &[lin], &[]);
    let one = b.tensor("ln.g", &[64], &[1.0; 64]);
    let zero = b.tensor("ln.b", &[64], &[0.0; 64]);
    let ln = b.node(OpCode::LayerNorm, &[x, one, zero], &[eps_bits(1e-5)]);
    let big = b.tensor("big", &[1], &[48.0]);
    let wide = b.node(OpCode::Mul, &[Val::MEL, big], &[]); // [−96, 96]
    let sg = b.node(OpCode::Sigmoid, &[wide], &[]);
    let si = b.node(OpCode::Silu, &[wide], &[]);
    let sm = b.node(OpCode::Softmax, &[wide], &[2]);
    let gl = b.node(OpCode::GlobalAvgPool, &[g], &[]);
    let lsum = b.node(OpCode::GlobalAvgPool, &[ln], &[]);
    let sgm = b.node(OpCode::GlobalAvgPool, &[sg], &[]);
    let sim = b.node(OpCode::GlobalMaxPool, &[si], &[]);
    let smm = b.node(OpCode::GlobalAvgPool, &[sm], &[]);
    b.output("tiny", t1m, &[3, 1, 1])
        .output("gelu", gl, &[384, 1])
        .output("ln", lsum, &[384, 1])
        .output("sigmoid", sgm, &[3, 1, 1])
        .output("silu", sim, &[3, 1, 1])
        .output("softmax", smm, &[3, 1, 1]);
    let probe = b.node(OpCode::Slice, &[g], &[0, 0, 8, 1]);
    b.output("gelu_rows", probe, &[8, 16]);
    heads(b, 1, None);
}

/// Every fixture: its name, its K, its graph, its input seed, its FLOP recount and its pinned output SHA-256.
type Fixture = (
    &'static str,
    u16,
    fn(&mut ModelBuilder),
    u64,
    u64,
    &'static str,
);

const FIXTURES: [Fixture; 6] = [
    (
        "heads",
        8,
        heads_only,
        1,
        8_240,
        "23d048efa7c373c10175867c72809a02a6186ac225c16728727a2157bd01ccdc",
    ),
    (
        "conv_stack",
        2,
        conv_stack,
        2,
        586_300,
        "efdcd421f5d6d7181d9a5d4f0f7b68178efd0e44c7f1b2cd5d6363a760c4d34f",
    ),
    (
        "conv1d_stack",
        1,
        conv1d_stack,
        3,
        85_702,
        "b505caedad5109188c8f083bf84e233a875729a76aba27488c4611dfd04138fd",
    ),
    (
        "dense_mix",
        1,
        dense_mix,
        4,
        85_270,
        "5114880b47f4ad2c4a91f6246174b3743ec4ad51aef6cbbb481ff02a5b537b03",
    ),
    (
        "cnn_attention",
        8,
        cnn_attention,
        5,
        11_127_920,
        "988b952a990d82f7ca507e065ac131498d96072573617cbc4747c1dd182203f0",
    ),
    (
        "underflow",
        1,
        underflow,
        6,
        966_418,
        "cd68f34d875e7ebedf9e3eb2955b29136e6c670a5e57b837d92acd91e89c21ca",
    ),
];

fn net_of(name: &str, k: u16, graph: fn(&mut ModelBuilder)) -> Net {
    let mut b = ModelBuilder::new(&format!("onx-{}", name.replace('_', "-")), k);
    graph(&mut b);
    let pack = ModelPack::parse(&b.build()).unwrap_or_else(|e| panic!("{name}: {e}"));
    Net::build(&pack).unwrap_or_else(|e| panic!("{name}: {e}"))
}

fn run_hash(net: &Net, seed: u64) -> String {
    hex(&net.run(&synthetic_input(seed)).unwrap().sha256())
}

/// Every fixture's outputs, hashed, against its pin; all mismatches at once.
#[test]
fn kernel_output_hashes_are_pinned() {
    use core::fmt::Write as _;
    let mut got = String::new();
    let mut bad = false;
    for (name, k, graph, seed, flops, want) in FIXTURES {
        let net = net_of(name, k, graph);
        let h = run_hash(&net, seed);
        assert_eq!(h, run_hash(&net, seed), "{name} is not repeatable");
        bad |= h != want || net.flops() != flops;
        let _ = writeln!(
            got,
            "{name}: {h} ({} FLOPs, peak {} elements)",
            net.flops(),
            net.peak_elements()
        );
    }
    assert!(!bad, "a kernel output hash moved:\n{got}");
}

/// The same bits under every FTZ/DAZ setting this target has.
#[test]
fn kernel_outputs_do_not_depend_on_ftz_or_daz() {
    for (name, k, graph, seed, _, _) in FIXTURES {
        let net = net_of(name, k, graph);
        let reference = with_mode(0, || run_hash(&net, seed));
        for (mode, bits) in modes() {
            assert_eq!(
                with_mode(bits, || run_hash(&net, seed)),
                reference,
                "{name} under {mode}"
            );
        }
    }
}

/// The FTZ/DAZ switch really changes plain arithmetic here (so the test above is not vacuous).
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
#[test]
fn ftz_really_changes_float_behaviour_here() {
    let product =
        |a: f32, b: f32| std::hint::black_box(std::hint::black_box(a) * std::hint::black_box(b));
    assert!(with_mode(0, || product(1e-30, 1e-10)).is_subnormal());
    assert_eq!(with_mode(modes()[1].1, || product(1e-30, 1e-10)), 0.0);
}

/// The underflow model really reaches the subnormal range: computed without flushing, its tiny products are
/// subnormal (so its hash under FTZ would differ if the kernels did not flush).
#[test]
fn the_underflow_fixture_reaches_subnormals() {
    let mel = synthetic_input(6).mel;
    let tiny = f32::MIN_POSITIVE;
    let n = mel
        .iter()
        .filter(|v| (std::hint::black_box(**v) * tiny).is_subnormal())
        .count();
    assert!(n > 10_000, "{n} of {} products are subnormal", mel.len());
}
