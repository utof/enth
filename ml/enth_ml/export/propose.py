"""Export propose v0 to ``ENTHMODL`` and prove parity with PyTorch (lane EXP, R3b; docs/r3.md, "The model: propose v0").

Needs torch and numpy (``uv sync --group train``). It writes a trained checkpoint to ``enth-model-<id>.bin`` through
:class:`~enth_ml.export.enthmodl.ModelWriter` and gives the reference outputs a parity run compares against.

**The weights the file holds are the weights PyTorch runs.** :func:`as_stored` folds every ``BatchNorm`` into its
convolution (in ``float32``), then rounds each tensor to the dtype the file stores it in: ``f16`` for the convolution,
``Linear`` and hypothesis matrices (round to nearest even, which widens back to ``f32`` exactly), ``f32`` for biases and
the input standardisation, with an ``f32`` below the smallest normal flushed to 0 as the reader requires. The folded
weight is rounded **once**, after folding (ONX's warning). The exporter writes the graph from that model, and the
reference outputs are that model's ``forward``: same numbers in the file and in PyTorch, so what a parity run measures
is the kernels' arithmetic (summation order, ``libm``), not storage.

The graph is the module tree walked in ``forward``'s order, without the batch axis (op set 1 has none): ``Mul`` and
``Add`` for the standardisation, ``Conv2d`` + ``Silu`` (and ``Add`` for a residual), an ``AvgPool2d`` over the whole
frequency axis, ``Reshape``, ``Linear`` + ``Silu``, ``Concat`` of the context, then the heads: ``Linear``, ``Silu``,
``Reshape`` to ``[K, 1, H1]``, ``MatMul`` with the stored ``[K, H1, W]`` matrices, ``Add`` of the bias stored as
``[K, 1, W]`` (numpy broadcasting is right-aligned, so a ``[K, W]`` bias would not line up), ``Reshape`` to ``[K, W]``,
and ``Slice`` + ``Sigmoid`` on the continuous values and the route depths, ``Concat``. The graph outputs exactly what
decoding reads (``hypotheses`` ``[K, 102]`` and ``confidence`` ``[K]``).

Commands (``python -m enth_ml.export.propose``)::

    export     --ckpt C --run RUN.json --id ID --out FILE [--weights f16|f32]   # the model file
    inputs     --dev DIR [--dev DIR ...] --out FILE [--count N] [--part I/N]   # real dev inputs, as a tensor file
    reference  --ckpt C --inputs FILE --out FILE [--weights f16|f32] [--device cpu|cuda]   # PyTorch's outputs
    dev        --ckpt C [--weights f16|f32|none] [--dev DIR ...]                 # the dev loss of the stored weights
    diff       A B                                                              # the largest difference per tensor
    fixture    --out DIR                                                        # the committed tiny parity fixture
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
import pathlib
import sys

# torch 2.14 routes a few eager ops to Triton kernels it compiles at first use, which needs a C compiler (the training
# desktop has none). Set before torch is imported, as ``enth_ml.train`` does.
os.environ.setdefault("TORCH_DISABLE_NATIVE_JIT", "1")

import numpy as np  # noqa: E402
import torch  # noqa: E402
from torch import nn  # noqa: E402

from ..model import layout as L  # noqa: E402
from ..model.fold import fold_batchnorm  # noqa: E402
from ..model.net import ModelConfig, ProposeV0, audit_ops, count_flops, exported_parameter_count  # noqa: E402
from . import enthmodl as E  # noqa: E402
from . import synthetic, tensors  # noqa: E402

MIN_NORMAL_F32 = 2.0**-126

#: The tiny network of the committed fixture: propose v0's shape (stem, four stride-2 stages with a two-block third,
#: frequency pool, embedding, K = 8 heads and a confidence head) at a few thousand parameters.
TINY = ModelConfig(stem=4, widths=(4, 4, 6, 8), blocks=(1, 1, 2, 1), embed=16, hyp_hidden=4, conf_hidden=8, hypotheses=8)
TINY_SEED = 20261022
TINY_INPUT_SEEDS = (11, 12, 13)
TINY_ID = "exp-tiny"


# -- the weights as the file stores them -------------------------------------------------------------------------------


def storage_dtype(name: str, weights: str = "f16") -> str:
    """``f16`` for matrices (``*.weight`` and the hypothesis matrices ``hyp_w``) when ``weights`` is ``"f16"``; ``f32``
    for everything else (biases, the standardisation constants), whose few elements cost nothing and whose rounding
    would shift every output."""
    return "f16" if weights == "f16" and (name.endswith(".weight") or name == "hyp_w") else "f32"


@torch.no_grad()
def as_stored(model: ProposeV0, weights: str = "f16") -> ProposeV0:
    """An eval-mode copy of ``model`` with every ``BatchNorm`` folded into its convolution and every tensor rounded to
    what the file stores (module docstring). Raises ``ValueError`` for a model outside op set 1 (TR's audit) or a value
    ``f16`` cannot hold."""
    problems = audit_ops(model)
    if problems:
        raise ValueError("the model is not in op set 1: " + "; ".join(problems))
    m = fold_batchnorm(model.cpu().float())
    for name, t in list(m.named_parameters()) + list(m.named_buffers()):
        if not bool(torch.isfinite(t).all()):
            raise ValueError(f"{name} holds a non-finite value")
        if storage_dtype(name, weights) == "f16":
            r = t.to(torch.float16)
            if not bool(torch.isfinite(r).all()):
                raise ValueError(f"{name} does not fit binary16")
            t.copy_(r.to(torch.float32))
        else:
            t.copy_(torch.where(t.abs() < MIN_NORMAL_F32, torch.zeros_like(t), t))
    return m


def load_checkpoint(path: str | pathlib.Path) -> tuple[ProposeV0, dict]:
    """The model of a training checkpoint (``ckpt-best.pt``) in eval mode, and the checkpoint's dict (a file this
    project wrote: it holds numpy arrays and the config, so it loads with ``weights_only=False``)."""
    from ..train.config import TrainConfig

    state = torch.load(path, map_location="cpu", weights_only=False)
    cfg = TrainConfig.from_dict(state["config"])
    model = ProposeV0(cfg.model)
    model.load_state_dict(state["model"])
    return model.eval(), state


# -- the graph ---------------------------------------------------------------------------------------------------------


def build_writer(
    m: ProposeV0,
    model_id: str,
    *,
    weights: str = "f16",
    engine: str = "",
    ds1: int = 0,
    manifest_sha256: bytes = bytes(32),
    info: list[tuple[str, str]] = (),
) -> E.ModelWriter:
    """The ``ModelWriter`` holding ``m``'s graph; ``m`` is an :func:`as_stored` model (``BatchNorm`` folded)."""
    cfg = m.cfg
    k, h1, width = cfg.hypotheses, cfg.hyp_hidden, cfg.width
    w = E.ModelWriter(model_id, k, engine=engine, flops=count_flops(m), ds1=ds1, manifest_sha256=manifest_sha256)
    for key, value in info:
        w.add_info(key, value)

    def stored(name: str, p: torch.Tensor, dims: tuple[int, ...] | None = None) -> E.Val:
        return w.tensor(name, dims or tuple(p.shape), p.detach().cpu().reshape(-1).tolist(), storage_dtype(name, weights))

    def conv(x: E.Val, name: str, c: nn.Conv2d) -> E.Val:
        if c.bias is None or c.padding_mode != "zeros":
            raise ValueError(f"{name}: expected a folded convolution with a bias and zero padding")
        (sh, sw), (ph, pw), (dh, dw) = c.stride, c.padding, c.dilation
        return w.node("Conv2d", [x, stored(f"{name}.weight", c.weight), stored(f"{name}.bias", c.bias)], [sh, sw, ph, pw, ph, pw, dh, dw, c.groups])

    def linear(x: E.Val, name: str, lin: nn.Linear) -> E.Val:
        return w.node("Linear", [x, stored(f"{name}.weight", lin.weight), stored(f"{name}.bias", lin.bias)])

    def silu(x: E.Val) -> E.Val:
        return w.node("Silu", [x])

    def folded(name: str, cb) -> nn.Conv2d:
        if not isinstance(cb.bn, nn.Identity):
            raise ValueError(f"{name}: BatchNorm is not folded")
        return cb.conv

    # The standardisation: Mul and Add constants inside the graph (the model contract), batch axis dropped.
    res, _, f = cfg.mel_shape
    x = w.node("Mul", [E.MEL, stored("mel_scale", m.mel_scale, (res, 1, f))])
    x = w.node("Add", [x, stored("mel_shift", m.mel_shift, (res, 1, f))])
    c = w.node("Mul", [E.CONTEXT, stored("ctx_scale", m.ctx_scale, (cfg.context_len,))])
    c = w.node("Add", [c, stored("ctx_shift", m.ctx_shift, (cfg.context_len,))])

    x = silu(conv(x, "stem.conv", folded("stem", m.stem)))
    for si, stage in enumerate(m.stages):
        x = silu(conv(x, f"stages.{si}.down.conv", folded("down", stage.down)))
        for bi, blk in enumerate(stage.blocks):
            p = f"stages.{si}.blocks.{bi}"
            y = silu(conv(x, f"{p}.a.conv", folded(p, blk.a)))
            y = conv(y, f"{p}.b.conv", folded(p, blk.b))
            x = silu(w.node("Add", [x, y]))
    pool = m.freq_pool
    (kh, kw), (sh, sw) = _pair(pool.kernel_size), _pair(pool.stride)
    if _pair(pool.padding) != (0, 0) or pool.ceil_mode:
        raise ValueError("the frequency pool must have no padding and floor mode")
    x = w.node("AvgPool2d", [x], [kh, kw, sh, sw, 0, 0, 0, 0])
    x = w.node("Reshape", [x], [m.flat])
    e = silu(linear(x, "proj", m.proj))
    e = w.node("Concat", [e, c], [0])
    e = silu(linear(e, "mix", m.mix))

    h = silu(linear(e, "hyp_in", m.hyp_in))
    h = w.node("Reshape", [h], [k, 1, h1])
    raw = w.node("MatMul", [h, stored("hyp_w", m.hyp_w)])
    raw = w.node("Add", [raw, stored("hyp_b", m.hyp_b, (k, 1, width))])
    raw = w.node("Reshape", [raw], [k, width])
    c0, c1 = L.CONTINUOUS
    d0, d1 = L.DEPTH
    cont = w.node("Sigmoid", [w.node("Slice", [raw], [1, c0, c1, 1])])
    logits = w.node("Slice", [raw], [1, c1, d0, 1])
    depth = w.node("Sigmoid", [w.node("Slice", [raw], [1, d0, d1, 1])])
    hyp = w.node("Concat", [cont, logits, depth], [1])
    conf = linear(silu(linear(e, "conf_in", m.conf_in)), "conf_out", m.conf_out)
    w.output("hypotheses", hyp, [k, width])
    w.output("confidence", conf, [k])
    if w.parameters() != exported_parameter_count(m):
        raise ValueError(f"the file holds {w.parameters()} parameters, the model's exported count is {exported_parameter_count(m)}")
    return w


def _pair(v) -> tuple[int, int]:
    return (v, v) if isinstance(v, int) else (int(v[0]), int(v[1]))


def check_round_trip(data: bytes, m: ProposeV0, weights: str = "f16") -> int:
    """Read the file back and compare every stored tensor with the :func:`as_stored` model it came from, exactly.
    Returns the number of tensors compared."""
    parsed = E.read(data)
    have = {name: i for i, (name, *_rest) in enumerate(parsed.tensors)}
    n = 0
    for name, t in list(m.named_parameters()) + list(m.named_buffers()):
        i = have.get(name)
        if i is None:
            raise ValueError(f"the file has no tensor {name}")
        if parsed.tensors[i][1] != (E.F16 if storage_dtype(name, weights) == "f16" else E.F32):
            raise ValueError(f"{name} is stored in another dtype")
        if parsed.tensor_values(i) != t.detach().cpu().reshape(-1).tolist():
            raise ValueError(f"{name}: the file's values differ from the model's")
        n += 1
    return n


# -- provenance --------------------------------------------------------------------------------------------------------


def _ascii(value: object, limit: int = 1024) -> str:
    s = str(value)
    if not all(" " <= ch <= "~" for ch in s):
        raise ValueError(f"info value {s!r} is not printable ASCII")
    return s[:limit]


def provenance(state: dict, run_json: bytes, weights: str = "f16") -> tuple[int, str, bytes, list[tuple[str, str]]]:
    """From the checkpoint and the run manifest (``run.json``): the header's ``ds1`` version, engine and the manifest's
    SHA-256, and the info table. Refuses a run whose layout is not this checkout's layout 1."""
    run = json.loads(run_json)
    if run.get("schema") != "enth.train-run/1":
        raise ValueError(f"run manifest schema {run.get('schema')}")
    if run["layout"]["sha256"] != E.layout_v1_sha256().hex() or run["layout"]["width"] != L.WIDTH:
        raise ValueError("the run was trained on another layout than this checkout's layout 1")
    train, dev = run["data"]["train"], run["data"]["dev"]
    tr, dv = train["runs"][0], dev["runs"][0]
    cfg = run["config"]
    mc = cfg["model"]
    env = run["environment"]
    info = [
        ("model.name", "propose v0"),
        (
            "model.arch",
            f"cnn stem{mc['stem']} widths{','.join(map(str, mc['widths']))} blocks{','.join(map(str, mc['blocks']))} "
            f"embed{mc['embed']} hyp{mc['hyp_hidden']} conf{mc['conf_hidden']} K{mc['hypotheses']}",
        ),
        ("train.run", pathlib.PurePosixPath(str(cfg["out"]).replace("\\", "/")).name),
        ("train.checkpoint", f"best, epoch {int(state['epoch']) + 1}, step {int(state['step'])}"),
        ("train.dev_best_of_k", f"{float(state['best_dev']):.6f}"),
        ("train.seed", cfg["seed"]),
        ("train.commit", env["commit"] + (" (dirty)" if env.get("dirty") else "")),
        ("train.torch", f"{env['torch']} cuda {env.get('cuda')}"),
        ("train.uv_lock_sha256", env.get("uv_lock_sha256")),
        ("data.ds1", train["ds1"]),
        ("data.train", f"{tr['stream']} seed {tr['run_seed']}, {train['samples']} samples, manifest sha256 {tr['manifest_sha256']}"),
        ("data.dev", f"{dv['stream']} seed {dv['run_seed']}, {dev['samples']} samples, manifest sha256 {dv['manifest_sha256']}"),
        ("data.sources", ", ".join(f"{k} {v}" for k, v in sorted(tr["sources"].items()))),
        ("data.augmentation", run["augmentation"]),
        ("data.licence", "ds1 clean renders by enth's own engine from generated and perturbed factory-preset patches; no recorded or third-party audio"),
        ("export.weights", "f16: convolution, linear and hypothesis matrices; f32: biases and input standardisation" if weights == "f16" else "f32: every tensor"),
        ("export.fold", "BatchNorm folded in f32, then rounded once"),
        ("export.tool", "enth_ml.export.propose (lane EXP)"),
    ]
    info = [(k, _ascii(v)) for k, v in info if v is not None]
    return int(train["ds1"]), str(tr["engine"]), hashlib.sha256(run_json).digest(), info


def export_checkpoint(ckpt: str | pathlib.Path, run_json: str | pathlib.Path, model_id: str, weights: str = "f16") -> tuple[bytes, ProposeV0, dict]:
    """The model file for a checkpoint and its run manifest; also the :func:`as_stored` model and the checkpoint."""
    model, state = load_checkpoint(ckpt)
    raw = pathlib.Path(run_json).read_bytes()
    ds1, engine, manifest, info = provenance(state, raw, weights)
    stored_model = as_stored(model, weights)
    writer = build_writer(stored_model, model_id, weights=weights, engine=engine, ds1=ds1, manifest_sha256=manifest, info=info)
    data = writer.build()
    check_round_trip(data, stored_model, weights)
    return data, stored_model, state


# -- reference outputs -------------------------------------------------------------------------------------------------


@torch.no_grad()
def run_reference(m: ProposeV0, mel: np.ndarray, context: np.ndarray, batch: int = 64, device: str = "cpu") -> tuple[np.ndarray, np.ndarray]:
    """``(hypotheses [N, K, 102], confidence [N, K])`` of an eval-mode model on ``float32`` inputs."""
    dev = torch.device(device)
    m = m.to(dev).eval()
    hyp, conf = [], []
    for i in range(0, len(mel), batch):
        a = torch.from_numpy(np.ascontiguousarray(mel[i : i + batch])).to(dev).float()
        b = torch.from_numpy(np.ascontiguousarray(context[i : i + batch])).to(dev).float()
        h, c = m(a, b)
        hyp.append(h.cpu().numpy())
        conf.append(c.cpu().numpy())
    return np.concatenate(hyp).astype(np.float32), np.concatenate(conf).astype(np.float32)


def pick_dev_inputs(dev_dirs: list[str], count: int = 0, part: tuple[int, int] = (0, 1)) -> dict[str, np.ndarray]:
    """Real dev inputs: ``count`` samples spread evenly over the dev stream (0: all of them), then part ``i`` of ``n``
    (contiguous). ``index`` is each sample's position in the stream, in file order."""
    from ..train.shards import open_dataset, read_shard

    ds = open_dataset(dev_dirs, "dev")
    total = ds.n
    chosen = np.arange(total) if count <= 0 or count >= total else np.unique(np.round(np.linspace(0, total - 1, count)).astype(np.int64))
    i, n = part
    chosen = np.array_split(chosen, n)[i]
    mel = np.empty((len(chosen), *L.MEL_SHAPE), dtype=np.float32)
    ctx = np.empty((len(chosen), L.CONTEXT_LEN), dtype=np.float32)
    first = 0
    at = 0
    for path, kept in ds.shards:
        hit = chosen[(chosen >= first) & (chosen < first + kept)] - first
        if len(hit):
            a = read_shard(path, ("mel", "context"))
            mel[at : at + len(hit)] = a["mel"][hit]
            ctx[at : at + len(hit)] = a["context"][hit]
            at += len(hit)
        first += kept
    assert at == len(chosen)
    return {"mel": mel, "context": ctx, "index": chosen.astype(np.float32)}


# -- the committed fixture ---------------------------------------------------------------------------------------------


def tiny_model(seed: int = TINY_SEED) -> ProposeV0:
    """A randomly initialised network of the fixture's shape. Every value comes from ``synthetic.weights`` (SplitMix64),
    never from torch's generator, so the committed bytes do not depend on torch's version. ``BatchNorm`` gets
    non-trivial affine and running statistics, so the fold is exercised; the standardisation is not the identity."""
    m = ProposeV0(TINY).eval()
    sd = m.state_dict()
    h1 = TINY.hyp_hidden
    with torch.no_grad():
        for i, name in enumerate(sorted(sd)):
            t = sd[name]
            if t.dtype != torch.float32:
                continue  # BatchNorm's batch counters
            u = torch.tensor(synthetic.weights(seed + i, t.numel(), 1.0), dtype=torch.float32).reshape(t.shape)
            if name.endswith("bn.weight"):
                v = 1.0 + 0.25 * u
            elif name.endswith("bn.bias"):
                v = 0.1 * u
            elif name.endswith("running_mean"):
                v = 0.2 * u
            elif name.endswith("running_var"):
                v = 1.0 + 0.5 * u
            elif name == "hyp_w":
                v = u * (3.0 / h1) ** 0.5 * 2.0
            elif name == "hyp_b":
                v = 0.8 * u
            elif name in ("mel_scale", "ctx_scale"):
                v = 0.5 + 0.25 * u
            elif name in ("mel_shift", "ctx_shift"):
                v = 0.3 * u
            elif name.endswith(".bias"):
                v = 0.1 * u
            elif name.endswith(".weight"):
                v = u * (3.0 / math.prod(t.shape[1:])) ** 0.5
            else:
                raise ValueError(f"no rule for {name}")
            t.copy_(v)
    return m


def fixture() -> tuple[bytes, bytes]:
    """``(exp_tiny.bin, exp_tiny.expected)``: the tiny network exported as the pilot is, and PyTorch's outputs on the
    synthetic inputs of :data:`TINY_INPUT_SEEDS` (batched: ``hypotheses`` ``[3, 8, 102]``, ``confidence`` ``[3, 8]``),
    which a CI test reproduces without torch (``propose_parity.rs``)."""
    m = as_stored(tiny_model(), "f16")
    info = [
        ("fixture", "a randomly initialised propose-v0-shaped network exported by enth_ml.export.propose (lane EXP)"),
        ("parity.seeds", ",".join(map(str, TINY_INPUT_SEEDS))),
    ]
    w = build_writer(m, TINY_ID, weights="f16", engine="0.3.0", ds1=1, info=info)
    data = w.build()
    check_round_trip(data, m, "f16")
    mel = np.array([synthetic.synthetic_input(s)[0] for s in TINY_INPUT_SEEDS], dtype=np.float32).reshape(-1, *L.MEL_SHAPE)
    ctx = np.array([synthetic.synthetic_input(s)[1] for s in TINY_INPUT_SEEDS], dtype=np.float32)
    hyp, conf = run_reference(m, mel, ctx)
    return data, tensors.write_arrays([("hypotheses", hyp), ("confidence", conf)])


# -- the command line --------------------------------------------------------------------------------------------------


def _part(text: str) -> tuple[int, int]:
    i, _, n = text.partition("/")
    return int(i), int(n)


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="python -m enth_ml.export.propose", description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    e = sub.add_parser("export")
    e.add_argument("--ckpt", required=True)
    e.add_argument("--run", required=True, help="the run manifest (run.json); its SHA-256 goes in the header")
    e.add_argument("--id", required=True)
    e.add_argument("--out", required=True)
    e.add_argument("--weights", default="f16", choices=["f16", "f32"])
    i = sub.add_parser("inputs")
    i.add_argument("--dev", action="append", required=True)
    i.add_argument("--out", required=True)
    i.add_argument("--count", type=int, default=0)
    i.add_argument("--part", type=_part, default=(0, 1))
    r = sub.add_parser("reference")
    r.add_argument("--ckpt", required=True)
    r.add_argument("--inputs", required=True)
    r.add_argument("--out", required=True)
    r.add_argument("--weights", default="f16", choices=["f16", "f32"])
    r.add_argument("--device", default="cpu")
    r.add_argument("--batch", type=int, default=64)
    d = sub.add_parser("dev")
    d.add_argument("--ckpt", required=True)
    d.add_argument("--weights", default="f16", choices=["f16", "f32", "none"])
    d.add_argument("--dev", action="append", default=None)
    d.add_argument("--device", default=None)
    df = sub.add_parser("diff")
    df.add_argument("a")
    df.add_argument("b")
    f = sub.add_parser("fixture")
    f.add_argument("--out", required=True)
    args = ap.parse_args(argv)

    if args.cmd == "export":
        data, _, state = export_checkpoint(args.ckpt, args.run, args.id, args.weights)
        out = pathlib.Path(args.out)
        out.write_bytes(data)
        sha = hashlib.sha256(data).hexdigest()
        print(json.dumps({"file": out.name, "bytes": len(data), "sha256": sha, "epoch": int(state["epoch"]) + 1, "step": int(state["step"])}, indent=1))
        return 0
    if args.cmd == "inputs":
        arrays = pick_dev_inputs(args.dev, args.count, args.part)
        pathlib.Path(args.out).write_bytes(tensors.write_arrays(list(arrays.items())))
        print(f"{len(arrays['index'])} dev samples (stream positions {int(arrays['index'][0])}..{int(arrays['index'][-1])}) -> {args.out}")
        return 0
    if args.cmd == "reference":
        model, _ = load_checkpoint(args.ckpt)
        m = as_stored(model, args.weights)
        a = tensors.read_arrays(pathlib.Path(args.inputs).read_bytes())
        hyp, conf = run_reference(m, a["mel"], a["context"], args.batch, args.device)
        pathlib.Path(args.out).write_bytes(tensors.write_arrays([("hypotheses", hyp), ("confidence", conf)]))
        print(f"{len(hyp)} samples through the {args.weights} weights on {args.device} -> {args.out}")
        return 0
    if args.cmd == "dev":
        from ..train.config import TrainConfig
        from ..train.data import DevSet
        from ..train.loop import evaluate
        from ..train.loss import ProposeLoss
        from ..train.shards import open_dataset

        model, state = load_checkpoint(args.ckpt)
        cfg = TrainConfig.from_dict(state["config"])
        device = torch.device(args.device or ("cuda" if torch.cuda.is_available() else "cpu"))
        dev_ds = open_dataset(args.dev or cfg.dev_dirs, "dev")
        m = model if args.weights == "none" else as_stored(model, args.weights)
        loss_fn = ProposeLoss(dev_ds.jnd_table, dev_ds.jnd_scale, cfg.loss).to(device)
        res = evaluate(m.to(device), loss_fn, DevSet(dev_ds.shards, cfg.dev_limit), device, cfg, per_entry=False)
        print(json.dumps(res, indent=1))
        return 0
    if args.cmd == "diff":
        a = tensors.read_arrays(pathlib.Path(args.a).read_bytes())
        b = tensors.read_arrays(pathlib.Path(args.b).read_bytes())
        for name in sorted(set(a) & set(b)):
            dlt = np.abs(a[name].astype(np.float64) - b[name].astype(np.float64))
            print(f"{name} {tuple(a[name].shape)}: max |a - b| = {dlt.max():.3e}, mean {dlt.mean():.3e}")
        return 0
    if args.cmd == "fixture":
        out = pathlib.Path(args.out)
        out.mkdir(parents=True, exist_ok=True)
        data, expected = fixture()
        (out / f"{TINY_ID.replace('-', '_')}.bin").write_bytes(data)
        (out / f"{TINY_ID.replace('-', '_')}.expected").write_bytes(expected)
        print(f"{TINY_ID}: {len(data)} bytes, expected {len(expected)} bytes -> {out}")
        return 0
    return 2


if __name__ == "__main__":
    sys.exit(main())
