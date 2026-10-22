"""ONNX to ``ENTHMODL`` (lane ONX): the converter behind the parity fixtures, and a fallback exporter for any graph
that stays inside op set 1.

Needs ``onnx`` and ``numpy`` (the ``parity`` dependency group: ``uv run --group parity ...``). The ONNX graph has a
batch axis of 1 on every activation (inputs ``mel`` ``[1, 3, 128, 64]`` and ``context`` ``[1, 5]``, in that order);
our op set has none, so the converter strips it and remaps every axis (docs/r3.md: "the exporter remaps every
axis"). It maps, node by node:

========================================  ==============================================================
ONNX                                      op set 1
========================================  ==============================================================
``Conv`` (rank 3 / 4, explicit pads)      ``Conv1d`` / ``Conv2d``; a following ``BatchNormalization``
                                          that only it feeds is folded into its weight and bias
``Gemm`` (2-D, alpha = beta = 1)          ``Linear`` (``transB = 0`` transposes the weight)
``MatMul``                                ``Linear`` when the right operand is a stored 2-D weight
                                          (transposed), else ``MatMul``
``LayerNormalization`` (last axis)        ``LayerNorm`` (a missing bias is zeros)
``Relu``, ``Tanh``, ``Sigmoid``           the same; ``Mul(x, Sigmoid(x))`` is ``Silu``
``Gelu`` (``approximate = none``)         ``Gelu``
``MaxPool``, ``AveragePool``              ``MaxPool1d/2d``, ``AvgPool1d/2d`` (floor mode, no dilation,
                                          ``count_include_pad = 0``)
``GlobalAveragePool``, ``GlobalMaxPool``  the same
``Softmax``, ``Concat``                   the same, axis − 1
``Add``, ``Mul``, ``Sub``                 the same (a constant of full rank loses its leading 1)
``Reshape``, ``Flatten``                  ``Reshape`` to the inferred shape without the batch axis
``Transpose`` (axis 0 stays first)        ``Transpose``
``Slice`` (one axis)                      ``Slice`` with clamped, non-negative indices
``Identity``                              nothing
========================================  ==============================================================

Anything else is refused with the node's name. ``f32`` initializers are written ``f32`` unless named in ``f16``
(rounded to nearest even; for exact parity the ONNX graph should already hold the ``f16``-rounded values).
"""

from __future__ import annotations

from typing import Iterable

import numpy as np
import onnx
from onnx import numpy_helper

from .enthmodl import CONTEXT, INPUTS_V1, MEL, ModelWriter, Val, eps_attr

I32_MAX = 2**31 - 1


class Unsupported(ValueError):
    """A node or graph shape the converter cannot express in op set 1."""


def _attrs(node: onnx.NodeProto) -> dict:
    return {a.name: onnx.helper.get_attribute_value(a) for a in node.attribute}


def convert(
    model: onnx.ModelProto,
    model_id: str,
    hypotheses: int,
    *,
    f16: Iterable[str] = (),
    info: Iterable[tuple[str, str]] = (),
    flops: int = 0,
) -> ModelWriter:
    """A :class:`ModelWriter` holding ``model``'s graph in op set 1 (see the module docstring)."""
    model = onnx.shape_inference.infer_shapes(model, strict_mode=True)
    g = model.graph
    f16 = set(f16)
    shapes: dict[str, tuple[int, ...]] = {}
    for vi in list(g.input) + list(g.value_info) + list(g.output):
        dims = tuple(d.dim_value for d in vi.type.tensor_type.shape.dim)
        shapes[vi.name] = dims
    consts = {t.name: numpy_helper.to_array(t) for t in g.initializer}
    w = ModelWriter(model_id, hypotheses, flops=flops)
    for k, v in info:
        w.add_info(k, v)

    inputs = [i for i in g.input if i.name not in consts]
    want = [(n, (1, *d)) for n, d in INPUTS_V1]
    got = [(i.name, shapes[i.name]) for i in inputs]
    if got != want:
        raise Unsupported(f"the graph's inputs are {got}; feature set 1 is {want}")
    env: dict[str, Val] = {"mel": MEL, "context": CONTEXT}
    tensor_of: dict[tuple[str, str], Val] = {}

    nodes = list(g.node)  # one list, so a node's identity is stable across the passes below
    consumers: dict[str, list[onnx.NodeProto]] = {}
    for n in nodes:
        for i in n.input:
            consumers.setdefault(i, []).append(n)
    outputs = {o.name for o in g.output}

    def only_feeds(name: str, op: str) -> onnx.NodeProto | None:
        c = consumers.get(name, [])
        if len(c) == 1 and c[0].op_type == op and name not in outputs:
            return c[0]
        return None

    def full_rank(name: str) -> int:
        return len(shapes[name])

    def stored(name: str, array: np.ndarray, tag: str = "", half: bool | None = None) -> Val:
        """A constant as a tensor (``f16`` when named in ``f16``, unless ``half`` says otherwise)."""
        key = (name, tag)
        if key not in tensor_of:
            a = np.asarray(array, dtype=np.float32)
            dims = list(a.shape) if a.ndim else [1]
            label = "".join(c if "!" <= c <= "~" else "_" for c in name + tag)[-64:]
            half_ = name in f16 if half is None else half
            tensor_of[key] = w.tensor(label, dims, a.reshape(-1).tolist(), "f16" if half_ else "f32")
        return tensor_of[key]

    def operand(name: str, ref_rank: int) -> Val:
        """An activation, or a constant as a tensor right-aligned against batch-stripped activations."""
        if name in env:
            return env[name]
        if name not in consts:
            raise Unsupported(f"value {name} is neither an activation nor an initializer")
        a = consts[name]
        if a.ndim == ref_rank:
            if a.shape[0] != 1:
                raise Unsupported(f"constant {name} {a.shape} broadcasts over the batch axis")
            return stored(name, a[0], ":nobatch")
        return stored(name, a)

    def act(name: str) -> Val:
        if name not in env:
            raise Unsupported(f"value {name} must be an activation")
        return env[name]

    def axis_of(node: onnx.NodeProto, axis: int, rank: int) -> int:
        a = axis + rank if axis < 0 else axis
        if not 1 <= a < rank:
            raise Unsupported(f"{node.name or node.op_type}: axis {axis} is the batch axis or out of range")
        return a - 1

    skip_ids: set[int] = set()  # nodes folded into an earlier one (BatchNormalization, the Mul of a SiLU)
    for n in nodes:
        if id(n) in skip_ids:
            continue
        op, a = n.op_type, _attrs(n)
        label = n.name or op
        out = n.output[0]
        if op == "Identity":
            env[out] = act(n.input[0])
        elif op == "Conv":
            if a.get("auto_pad", b"NOTSET") not in (b"NOTSET", "NOTSET"):
                raise Unsupported(f"{label}: auto_pad (pads must be explicit)")
            rank = full_rank(n.input[0])
            weight = consts[n.input[1]].astype(np.float64)
            bias = consts[n.input[2]].astype(np.float64) if len(n.input) > 2 and n.input[2] else None
            bn = only_feeds(out, "BatchNormalization")
            if bn is not None:
                scale, b0, mean, var = (consts[i].astype(np.float64) for i in bn.input[1:5])
                eps = _attrs(bn).get("epsilon", 1e-5)
                f = scale / np.sqrt(var + eps)
                weight = weight * f.reshape(-1, *([1] * (weight.ndim - 1)))
                bias = ((bias if bias is not None else 0.0) - mean) * f + b0
                skip_ids.add(id(bn))
                out = bn.output[0]
            k = len(weight.shape) - 2
            strides = a.get("strides", [1] * k)
            pads = a.get("pads", [0] * (2 * k))
            dil = a.get("dilations", [1] * k)
            groups = a.get("group", 1)
            # A folded weight holds new values: it stays f32 (rounding it to f16 again would lose the fold's precision).
            wv = stored(n.input[1], weight, ":bn", half=False) if bn is not None else stored(n.input[1], weight)
            ins = [act(n.input[0]), wv]
            if bias is not None:
                ins.append(stored((n.input[2] if len(n.input) > 2 and n.input[2] else n.input[1] + ".bias"), bias,
                                  ":bn" if bn is not None else ""))
            if rank == 3:
                env[out] = w.node("Conv1d", ins, [strides[0], pads[0], pads[1], dil[0], groups])
            elif rank == 4:
                env[out] = w.node(
                    "Conv2d", ins, [strides[0], strides[1], pads[0], pads[1], pads[2], pads[3], dil[0], dil[1], groups]
                )
            else:
                raise Unsupported(f"{label}: a rank-{rank} convolution")
        elif op == "Gemm":
            if a.get("transA", 0) or a.get("alpha", 1.0) != 1.0 or a.get("beta", 1.0) != 1.0:
                raise Unsupported(f"{label}: Gemm with transA or alpha/beta other than 1")
            if full_rank(n.input[0]) != 2 or n.input[1] not in consts:
                raise Unsupported(f"{label}: Gemm on a [1, n] activation with a stored weight only")
            weight = consts[n.input[1]]
            weight = weight if a.get("transB", 0) else weight.T
            ins = [act(n.input[0]), stored(n.input[1], weight, "" if a.get("transB", 0) else ":T")]
            if len(n.input) > 2 and n.input[2]:
                ins.append(stored(n.input[2], consts[n.input[2]].reshape(-1)))
            env[out] = w.node("Linear", ins)
        elif op == "MatMul":
            x, y = n.input
            if y in consts and consts[y].ndim == 2 and x in env:
                env[out] = w.node("Linear", [act(x), stored(y, consts[y].T, ":T")])
            else:
                r = full_rank(out)
                if r < 3:
                    raise Unsupported(f"{label}: MatMul needs rank 2 or more without the batch axis")
                env[out] = w.node("MatMul", [operand(x, r), operand(y, r)])
        elif op == "LayerNormalization":
            rank = full_rank(n.input[0])
            if a.get("axis", -1) not in (-1, rank - 1):
                raise Unsupported(f"{label}: LayerNormalization over other than the last axis")
            gamma = consts[n.input[1]]
            beta = consts[n.input[2]] if len(n.input) > 2 and n.input[2] else np.zeros_like(gamma)
            ins = [act(n.input[0]), stored(n.input[1], gamma), stored(n.input[2] if len(n.input) > 2 and n.input[2]
                                                                       else n.input[1] + ".zeros", beta)]
            env[out] = w.node("LayerNorm", ins, [eps_attr(a.get("epsilon", 1e-5))])
        elif op in ("Relu", "Tanh"):
            env[out] = w.node(op, [act(n.input[0])])
        elif op == "Sigmoid":
            mul = only_feeds(out, "Mul")
            if mul is not None and n.input[0] in mul.input:
                env[mul.output[0]] = w.node("Silu", [act(n.input[0])])
                skip_ids.add(id(mul))
            else:
                env[out] = w.node("Sigmoid", [act(n.input[0])])
        elif op == "Gelu":
            if a.get("approximate", b"none") not in (b"none", "none"):
                raise Unsupported(f"{label}: the tanh form of GELU (op set 1 pins the erf form)")
            env[out] = w.node("Gelu", [act(n.input[0])])
        elif op in ("MaxPool", "AveragePool"):
            rank = full_rank(n.input[0])
            k = a["kernel_shape"]
            s = a.get("strides", [1] * len(k))
            p = a.get("pads", [0] * (2 * len(k)))
            if a.get("ceil_mode", 0) or any(d != 1 for d in a.get("dilations", [1] * len(k))):
                raise Unsupported(f"{label}: ceil mode or dilation")
            if a.get("auto_pad", b"NOTSET") not in (b"NOTSET", "NOTSET") or a.get("storage_order", 0):
                raise Unsupported(f"{label}: auto_pad or storage_order")
            if op == "AveragePool" and a.get("count_include_pad", 0):
                raise Unsupported(f"{label}: count_include_pad = 1 (op set 1 never counts padding)")
            kind = "Max" if op == "MaxPool" else "Avg"
            if rank == 3:
                env[out] = w.node(f"{kind}Pool1d", [act(n.input[0])], [k[0], s[0], p[0], p[1]])
            elif rank == 4:
                env[out] = w.node(f"{kind}Pool2d", [act(n.input[0])], [k[0], k[1], s[0], s[1], p[0], p[1], p[2], p[3]])
            else:
                raise Unsupported(f"{label}: a rank-{rank} pool")
        elif op in ("GlobalAveragePool", "GlobalMaxPool"):
            env[out] = w.node("GlobalAvgPool" if op == "GlobalAveragePool" else "GlobalMaxPool", [act(n.input[0])])
        elif op == "Softmax":
            rank = full_rank(n.input[0])
            env[out] = w.node("Softmax", [act(n.input[0])], [axis_of(n, a.get("axis", -1), rank)])
        elif op in ("Add", "Mul", "Sub"):
            r = full_rank(out)
            x, y = n.input
            if x not in env and y not in env:
                raise Unsupported(f"{label}: both operands are constants (fold them first)")
            env[out] = w.node(op, [operand(x, r), operand(y, r)])
        elif op == "Concat":
            r = full_rank(out)
            env[out] = w.node("Concat", [operand(i, r) for i in n.input], [axis_of(n, a["axis"], r)])
        elif op in ("Reshape", "Flatten"):
            dims = shapes[out]
            if not dims or dims[0] != 1 or len(dims) < 2:
                raise Unsupported(f"{label}: the reshape must keep a leading batch axis of 1")
            env[out] = w.node("Reshape", [act(n.input[0])], list(dims[1:]))
        elif op == "Transpose":
            perm = list(a["perm"])
            if perm[0] != 0:
                raise Unsupported(f"{label}: the batch axis must stay first")
            env[out] = w.node("Transpose", [act(n.input[0])], [p - 1 for p in perm[1:]])
        elif op == "Slice":
            starts, ends = (consts[i].reshape(-1).tolist() for i in n.input[1:3])
            axes = consts[n.input[3]].reshape(-1).tolist() if len(n.input) > 3 and n.input[3] else list(range(len(starts)))
            steps = consts[n.input[4]].reshape(-1).tolist() if len(n.input) > 4 and n.input[4] else [1] * len(starts)
            if len(starts) != 1 or steps[0] < 1:
                raise Unsupported(f"{label}: Slice over one axis with a positive step only")
            x = shapes[n.input[0]]
            ax = axes[0] + len(x) if axes[0] < 0 else axes[0]
            dim = x[ax]
            start = min(max(starts[0] + dim if starts[0] < 0 else starts[0], 0), dim)
            end = min(max(ends[0] + dim if ends[0] < 0 else ends[0], 0), dim, I32_MAX)
            env[out] = w.node("Slice", [act(n.input[0])], [axis_of(n, ax, len(x)), start, end, steps[0]])
        else:
            raise Unsupported(f"{label}: ONNX {op} is not in op set 1")

    for o in g.output:
        dims = shapes[o.name]
        if not dims or dims[0] != 1:
            raise Unsupported(f"output {o.name} {dims} has no leading batch axis of 1")
        w.output(o.name, act(o.name), dims[1:])
    return w

