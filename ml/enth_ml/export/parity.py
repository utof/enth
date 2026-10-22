"""The ONNX Runtime parity fixtures (lane ONX; phase-3's ONX acceptance: "parity with ONNX Runtime within 1e-5 on
exported fixtures").

Builds small ONNX graphs with ``onnx.helper`` (every op of op set 1, and a CNN with an attention block), runs ONNX
Runtime on a synthetic input (:mod:`.synthetic`), converts each graph to ``ENTHMODL`` (:mod:`.onnx_import`) and writes,
per fixture, ``<name>.bin`` (the model; its info table names the input seed, ``parity.seed``) and ``<name>.expected``
(ONNX Runtime's outputs without the batch axis, :mod:`.tensors`). ``enth-solver``'s ``tests/propose_parity.rs`` runs
our kernels on every fixture on every CI leg and checks the largest difference is at most 1e-5;
``tools/model-check run <name>.bin --expected <name>.expected`` does the same by hand.

Run from ``ml/`` (it needs the ``parity`` group)::

    uv run --group parity python -m enth_ml.export.parity --out ../crates/enth-solver/tests/fixtures/propose

ONNX Runtime runs on one thread with every graph optimisation off, so it computes each node as ONNX defines it. Its
own kernels sum in their own order (and may fuse multiply-adds), so its outputs match ours within a tolerance, not bit
for bit; our bits are pinned by the Rust test instead. Weights come from the same deterministic generator as the
inputs, so a rerun writes the same ``.bin`` files byte for byte; ``.expected`` may move in the last bits with ONNX
Runtime's version or CPU (the tests compare a rerun within 1e-6).
"""

from __future__ import annotations

import argparse
import hashlib
import pathlib
import sys
from typing import Callable

import numpy as np
import onnx
import onnxruntime as ort
from onnx import TensorProto, helper, numpy_helper

from . import synthetic, tensors
from .onnx_import import convert

OPSET = 20  # Gelu is in ONNX from opset 20
IR_VERSION = 10


class Graph:
    """A small ONNX graph under construction, with feature set 1's inputs and the required heads."""

    def __init__(self, name: str, seed: int) -> None:
        self.name = name
        self.seed = seed
        self.nodes: list[onnx.NodeProto] = []
        self.inits: list[onnx.TensorProto] = []
        self.outputs: list[onnx.ValueInfoProto] = []
        self.f16: set[str] = set()
        self._n = 0
        self._w = seed * 1000

    def fresh(self, stem: str) -> str:
        self._n += 1
        return f"{stem}_{self._n}"

    def const(self, stem: str, array: np.ndarray, f16: bool = False) -> str:
        name = self.fresh(stem)
        a = np.asarray(array)
        if a.dtype.kind == "f":
            a = a.astype(np.float32)
            if f16:
                a = a.astype(np.float16).astype(np.float32)  # what the f16 file holds, widened exactly
                self.f16.add(name)
        self.inits.append(numpy_helper.from_array(a, name))
        return name

    def weight(self, stem: str, shape: tuple[int, ...], scale: float, f16: bool = False) -> str:
        self._w += 1
        values = synthetic.weights(self._w, int(np.prod(shape)), scale)
        return self.const(stem, np.array(values, dtype=np.float32).reshape(shape), f16)

    def ints(self, stem: str, values: list[int]) -> str:
        return self.const(stem, np.array(values, dtype=np.int64))

    def op(self, op: str, inputs: list[str], **attrs) -> str:
        out = self.fresh(op.lower())
        self.nodes.append(helper.make_node(op, inputs, [out], name=out, **attrs))
        return out

    def output(self, name: str, value: str, dims: list[int]) -> None:
        self.nodes.append(helper.make_node("Identity", [value], [name], name=f"out_{name}"))
        self.outputs.append(helper.make_tensor_value_info(name, TensorProto.FLOAT, [1, *dims]))

    def heads(self, k: int, feat: str | None = None, n: int = 0, f16: bool = False) -> None:
        """``hypotheses`` ``[K, 102]`` and ``confidence`` ``[K]`` from ``context`` (and a ``[1, n]`` feature)."""
        x, width = ("context", 5) if feat is None else (self.op("Concat", [feat, "context"], axis=1), n + 5)
        wh = self.weight("head_hyp_w", (k * 102, width), 0.25, f16)
        bh = self.weight("head_hyp_b", (k * 102,), 0.5)
        h = self.op("Gemm", [x, wh, bh], transB=1)
        self.output("hypotheses", self.op("Reshape", [h, self.ints("shape", [1, k, 102])]), [k, 102])
        wc = self.weight("head_conf_w", (width, k), 0.25)  # transB = 0: the converter transposes it
        self.output("confidence", self.op("Gemm", [x, wc]), [k])

    def model(self) -> onnx.ModelProto:
        inputs = [
            helper.make_tensor_value_info("mel", TensorProto.FLOAT, [1, 3, 128, 64]),
            helper.make_tensor_value_info("context", TensorProto.FLOAT, [1, 5]),
        ]
        graph = helper.make_graph(self.nodes, self.name, inputs, self.outputs, self.inits)
        m = helper.make_model(graph, opset_imports=[helper.make_opsetid("", OPSET)], producer_name="enth-ml-parity")
        m.ir_version = IR_VERSION
        onnx.checker.check_model(m, full_check=True)
        return m


def conv(g: Graph) -> None:
    """``Conv`` 2-D (stride, padding, dilation, groups, bias, an ``f16`` weight), ``Relu``, both 2-D pools, both
    global pools, and a ``BatchNormalization`` folded into its convolution."""
    s = g.op("Slice", ["mel", g.ints("s", [0]), g.ints("e", [32]), g.ints("a", [2])])  # [1, 3, 32, 64]
    c1 = g.op("Conv", [s, g.weight("c1_w", (8, 3, 3, 3), 0.25), g.weight("c1_b", (8,), 0.125)],
              strides=[2, 1], pads=[1, 1, 1, 1])  # [1, 8, 16, 64]
    r = g.op("Relu", [c1])
    c2 = g.op("Conv", [r, g.weight("c2_w", (8, 2, 3, 3), 0.5, f16=True)],
              strides=[1, 2], pads=[0, 1, 2, 1], dilations=[2, 1], group=4)  # [1, 8, 14, 32]
    m = g.op("MaxPool", [c2], kernel_shape=[2, 3], strides=[2, 2], pads=[0, 1, 1, 1])  # [1, 8, 7, 16]
    a = g.op("AveragePool", [m], kernel_shape=[3, 3], strides=[1, 2], pads=[1, 1, 1, 0])  # [1, 8, 7, 8]
    g.output("a", a, [8, 7, 8])
    g.output("g", g.op("GlobalAveragePool", [a]), [8, 1, 1])
    g.output("gm", g.op("GlobalMaxPool", [c2]), [8, 1, 1])
    c3 = g.op("Conv", [a, g.weight("c3_w", (4, 8, 1, 1), 0.5), g.weight("c3_b", (4,), 0.25)])
    bn = g.op("BatchNormalization", [c3, g.weight("bn_s", (4,), 1.0), g.weight("bn_b", (4,), 0.5),
                                     g.weight("bn_m", (4,), 0.25), g.const("bn_v", np.array([0.5, 1.0, 2.0, 0.25]))],
              epsilon=1e-5)
    g.output("bn", bn, [4, 7, 8])
    g.heads(1)


def conv1d(g: Graph) -> None:
    """``Conv`` 1-D (stride, padding, dilation, groups), ``Gelu``, both 1-D pools."""
    s = g.op("Slice", ["mel", g.ints("s", [1]), g.ints("e", [2]), g.ints("a", [1])])  # [1, 1, 128, 64]
    x = g.op("Reshape", [s, g.ints("shape", [1, 16, 512])])
    c = g.op("Conv", [x, g.weight("c_w", (12, 4, 5), 0.25), g.weight("c_b", (12,), 0.25)],
             strides=[3], pads=[2, 4], dilations=[2], group=4)  # [1, 12, 170]
    ge = g.op("Gelu", [c])
    mp = g.op("MaxPool", [ge], kernel_shape=[4], strides=[3], pads=[1, 2])  # [1, 12, 57]
    ap = g.op("AveragePool", [mp], kernel_shape=[5], strides=[2], pads=[4, 0])  # [1, 12, 29]
    g.output("ap", ap, [12, 29])
    g.heads(1)


def dense(g: Graph) -> None:
    """``LayerNormalization``, ``MatMul`` (stored and activation operands), ``Add``/``Mul``/``Sub`` broadcasting,
    ``Silu`` (as ``Mul(x, Sigmoid(x))``), ``Tanh``, ``Sigmoid``, ``Softmax`` on two axes, ``Transpose``, ``Concat``, a
    strided ``Slice``, ``Flatten``."""
    t = g.op("Slice", ["mel", g.ints("s", [5]), g.ints("e", [128]), g.ints("a", [2]), g.ints("st", [37])])
    x = g.op("Reshape", [t, g.ints("shape", [1, 12, 64])])
    ln = g.op("LayerNormalization", [x, g.weight("ln_g", (64,), 2.0), g.weight("ln_b", (64,), 0.5)],
              axis=-1, epsilon=1e-5)
    lin = g.op("Add", [g.op("MatMul", [ln, g.weight("lin_w", (64, 32), 0.25)]), g.weight("lin_b", (32,), 0.25)])
    si = g.op("Mul", [lin, g.op("Sigmoid", [lin])])
    th = g.op("Tanh", [lin])
    sg = g.op("Sigmoid", [lin])
    ad = g.op("Add", [si, g.weight("b2", (32,), 1.0)])
    sb = g.op("Sub", [ad, g.op("Mul", [th, sg])])
    sm0 = g.op("Softmax", [sb], axis=1)
    sm1 = g.op("Softmax", [sb], axis=-1)
    mm = g.op("MatMul", [sm0, g.op("Transpose", [sm1], perm=[0, 2, 1])])  # [1, 12, 12]
    mm2 = g.op("MatMul", [x, g.weight("mm_w", (64, 12), 0.125)])
    cc = g.op("Concat", [mm, mm2], axis=2)  # [1, 12, 24]
    g.output("cc", cc, [12, 24])
    b3 = g.op("Mul", [g.op("Reshape", [mm, g.ints("shape", [1, 3, 4, 12])]), g.weight("col", (4, 1), 1.0)])
    g.output("b3", b3, [3, 4, 12])
    g.output("flat", g.op("Flatten", [g.op("Slice", [b3, g.ints("s", [1]), g.ints("e", [2]), g.ints("a", [1])])],
                          axis=1), [48])
    g.heads(1)


def attention(g: Graph) -> None:
    """A small CNN with one four-head attention block, as propose v0 is shaped: a strided stem with a folded
    ``BatchNormalization``, time tokens, ``LayerNormalization``, attention as ``MatMul`` + ``Reshape`` + ``Transpose`` +
    ``MatMul`` + ``Softmax``, a ``Gelu`` MLP, residuals, mean pooling, and K = 2 heads; the matrices are ``f16``."""
    c = g.op("Conv", ["mel", g.weight("stem_w", (8, 3, 3, 3), 0.25, f16=True)], strides=[4, 4], pads=[1, 1, 1, 1])
    bn = g.op("BatchNormalization", [c, g.weight("bn_s", (8,), 1.0), g.weight("bn_b", (8,), 0.25),
                                     g.weight("bn_m", (8,), 0.25), g.const("bn_v", np.full(8, 1.5))])
    r = g.op("Relu", [bn])  # [1, 8, 32, 16]
    tok = g.op("Reshape", [g.op("Transpose", [r], perm=[0, 2, 1, 3]), g.ints("shape", [1, 32, 128])])
    x = g.op("MatMul", [tok, g.weight("proj_w", (128, 16), 0.125, f16=True)])  # [1, 32, 16]

    def ln(v: str) -> str:
        return g.op("LayerNormalization", [v, g.weight("ln_g", (16,), 2.0), g.weight("ln_b", (16,), 0.25)])

    def lin(v: str, n: int, m: int) -> str:
        return g.op("Add", [g.op("MatMul", [v, g.weight("w", (n, m), 0.25, f16=True)]), g.weight("b", (m,), 0.125)])

    def split(v: str) -> str:
        return g.op("Transpose", [g.op("Reshape", [v, g.ints("shape", [1, 32, 4, 4])])], perm=[0, 2, 1, 3])

    h = ln(x)
    q, k, v = (split(lin(h, 16, 16)) for _ in range(3))
    scores = g.op("MatMul", [q, g.op("Transpose", [k], perm=[0, 1, 3, 2])])  # [1, 4, 32, 32]
    att = g.op("Softmax", [g.op("Mul", [scores, g.const("scale", np.array([0.5]))])], axis=-1)
    ctx = g.op("Reshape", [g.op("Transpose", [g.op("MatMul", [att, v])], perm=[0, 2, 1, 3]),
                           g.ints("shape", [1, 32, 16])])
    x = g.op("Add", [x, lin(ctx, 16, 16)])
    x = g.op("Add", [x, lin(g.op("Gelu", [lin(ln(x), 16, 32)]), 32, 16)])
    pooled = g.op("GlobalAveragePool", [g.op("Transpose", [x], perm=[0, 2, 1])])  # [1, 16, 1]
    emb = g.op("Reshape", [pooled, g.ints("shape", [1, 16])])
    g.output("embedding", emb, [16])
    g.heads(2, emb, 16, f16=True)


#: name -> (graph, K, input seed)
FIXTURES: dict[str, tuple[Callable[[Graph], None], int, int]] = {
    "onnx_conv": (conv, 1, 11),
    "onnx_conv1d": (conv1d, 1, 12),
    "onnx_dense": (dense, 1, 13),
    "onnx_attention": (attention, 2, 14),
}


def build(name: str) -> tuple[onnx.ModelProto, Graph]:
    fn, _, seed = FIXTURES[name]
    g = Graph(name, seed)
    fn(g)
    return g.model(), g


def run_ort(model: onnx.ModelProto, seed: int) -> list[tuple[str, tuple[int, ...], list[float]]]:
    """ONNX Runtime's outputs on ``synthetic_input(seed)``, without the batch axis."""
    opts = ort.SessionOptions()
    opts.intra_op_num_threads = 1
    opts.inter_op_num_threads = 1
    opts.graph_optimization_level = ort.GraphOptimizationLevel.ORT_DISABLE_ALL
    sess = ort.InferenceSession(model.SerializeToString(), opts, providers=["CPUExecutionProvider"])
    mel, context = synthetic.synthetic_input(seed)
    feeds = {
        "mel": np.array(mel, dtype=np.float32).reshape(1, 3, 128, 64),
        "context": np.array(context, dtype=np.float32).reshape(1, 5),
    }
    names = [o.name for o in sess.get_outputs()]
    out = sess.run(names, feeds)
    return [(n, tuple(v.shape[1:]), v.reshape(-1).astype(np.float32).tolist()) for n, v in zip(names, out)]


def fixture(name: str) -> tuple[bytes, bytes]:
    """The ``.bin`` and ``.expected`` bytes of one fixture."""
    model, g = build(name)
    _, k, seed = FIXTURES[name]
    onnx_sha = hashlib.sha256(model.SerializeToString(deterministic=True)).hexdigest()
    info = [
        ("parity.seed", str(seed)),
        ("parity.onnx-sha256", onnx_sha),
        ("parity.source", f"onnx {onnx.__version__} opset {OPSET}; onnxruntime {ort.__version__}, 1 thread, no graph optimisation"),
    ]
    writer = convert(model, name.replace("_", "-"), k, f16=g.f16, info=info)
    expected = tensors.write(run_ort(model, seed))
    return writer.build(), expected


def main(argv: list[str] | None = None) -> int:
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument("--out", type=pathlib.Path, required=True, help="the fixture directory")
    p.add_argument("--only", nargs="*", default=None, help="fixture names (default: all)")
    args = p.parse_args(argv)
    args.out.mkdir(parents=True, exist_ok=True)
    for name in args.only or FIXTURES:
        model_bytes, expected = fixture(name)
        (args.out / f"{name}.bin").write_bytes(model_bytes)
        (args.out / f"{name}.expected").write_bytes(expected)
        print(f"{name}: {len(model_bytes)} + {len(expected)} bytes, sha256 {hashlib.sha256(model_bytes).hexdigest()}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
