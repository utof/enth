"""A writer of our model file, ``ENTHMODL`` format 1 (lane ONX; the byte table is ``pack.rs``'s module docs).

Standard library only. The Rust reader (``enth_solver::propose::pack::ModelPack::parse``) is the judge of a file: this
writer lays out exactly what it reads (a 256-byte header, the five tables, zero padding to a multiple of 64, the
tensors at multiples of 64 in table order, the SHA-256 of the whole file with its own 32 bytes read as zeros), and the
Rust builder (``pack::ModelBuilder``) writes the same bytes for the same graph. What it does on top: every ``f32``
weight that is subnormal is flushed to 0 (the reader refuses subnormal weights), and ``f16`` tensors are rounded to
nearest, ties to even (``struct``'s ``e`` format), which widens back to ``f32`` exactly.

Value ids, as the reader numbers them: the inputs (``mel`` 0, ``context`` 1), then the tensors in the order they were
added, then each node's output in node order. :class:`ModelWriter` hands out symbolic values (:class:`Val`) and
resolves them when the file is written, so tensors and nodes may be added in any order.

Op set 1's codes and attribute counts are :data:`OPS` (``pack.rs``'s ``OPS``; ``tests/test_export_enthmodl.py``
checks the two tables agree).

Lane EXP added :func:`read`, a reader of the same bytes (the header, the five tables, the SHA-256), so an exporter can
read its own file back and compare every stored weight with the model it came from. It checks the structure, not the
budgets or the layout: the Rust reader (``ModelPack::parse``) is still the judge of a file.
"""

from __future__ import annotations

import dataclasses
import hashlib
import math
import pathlib
import struct
from typing import Iterable, Sequence

MAGIC = b"ENTHMODL"
FORMAT = 1
HEADER_LEN = 256
OP_SET = 1
TENSOR_ALIGN = 64
DATA_ALIGN = 64
SHA_AT = 224

#: Feature set 1's inputs, in order (``features::SPEC_V1``).
FEATURE_V1 = 1
INPUTS_V1: tuple[tuple[str, tuple[int, ...]], ...] = (("mel", (3, 128, 64)), ("context", (5,)))

#: Layout 1 (``propose-layout/1``) on generator 1 and template 0: its width and the file that pins its canonical text,
#: whose SHA-256 is the layout hash a header carries.
LAYOUT_V1 = 1
LAYOUT_V1_GENERATOR = 1
LAYOUT_V1_TEMPLATE = 0
LAYOUT_V1_WIDTH = 102
LAYOUT_V1_TEXT = pathlib.Path(__file__).resolve().parents[3] / "crates/enth-solver/src/propose/layout-v1.txt"

#: Op set 1: name -> (code, (fewest inputs, most inputs), attributes, whether the count is "at least").
OPS: dict[str, tuple[int, tuple[int, int], int, bool]] = {
    "Conv1d": (1, (2, 3), 5, False),
    "Conv2d": (2, (2, 3), 9, False),
    "Linear": (3, (2, 3), 0, False),
    "MatMul": (4, (2, 2), 0, False),
    "LayerNorm": (5, (3, 3), 1, False),
    "Relu": (6, (1, 1), 0, False),
    "Gelu": (7, (1, 1), 0, False),
    "Silu": (8, (1, 1), 0, False),
    "Tanh": (9, (1, 1), 0, False),
    "Sigmoid": (10, (1, 1), 0, False),
    "MaxPool1d": (11, (1, 1), 4, False),
    "AvgPool1d": (12, (1, 1), 4, False),
    "MaxPool2d": (13, (1, 1), 8, False),
    "AvgPool2d": (14, (1, 1), 8, False),
    "GlobalAvgPool": (15, (1, 1), 0, False),
    "GlobalMaxPool": (16, (1, 1), 0, False),
    "Softmax": (17, (1, 1), 1, False),
    "Add": (18, (2, 2), 0, False),
    "Mul": (19, (2, 2), 0, False),
    "Concat": (20, (2, 32), 1, False),
    "Reshape": (21, (1, 1), 1, True),
    "Transpose": (22, (1, 1), 1, True),
    "Sub": (23, (2, 2), 0, False),
    "Slice": (24, (1, 1), 4, False),
}

F32 = 1
F16 = 2
MIN_NORMAL_F32 = 2.0**-126


def layout_v1_sha256() -> bytes:
    """The layout hash of layout 1 as this checkout pins it (the SHA-256 of ``layout-v1.txt``)."""
    return hashlib.sha256(LAYOUT_V1_TEXT.read_bytes()).digest()


def f32_bits(v: float) -> int:
    """The bits of ``v`` rounded to ``f32``."""
    return struct.unpack("<I", struct.pack("<f", v))[0]


def eps_attr(eps: float) -> int:
    """A float attribute (``LayerNorm``'s epsilon) as the ``i32`` the file stores: its ``f32`` bit pattern."""
    bits = f32_bits(eps)
    return bits - (1 << 32) if bits >= 1 << 31 else bits


def flush_f32(v: float) -> float:
    """``v`` rounded to ``f32``, with a subnormal result flushed to 0 (as the reader requires)."""
    r = struct.unpack("<f", struct.pack("<f", v))[0]
    if r != 0.0 and abs(r) < MIN_NORMAL_F32:
        return 0.0
    return r


@dataclasses.dataclass(frozen=True)
class Val:
    """A value of the graph: ``kind`` is ``"input"``, ``"tensor"`` or ``"node"``, ``index`` its position."""

    kind: str
    index: int


MEL = Val("input", 0)
CONTEXT = Val("input", 1)


@dataclasses.dataclass
class _Tensor:
    name: str
    dtype: int
    dims: tuple[int, ...]
    data: bytes


class ModelWriter:
    """Builds one ``ENTHMODL`` file (see the module docstring)."""

    def __init__(
        self,
        model_id: str,
        hypotheses: int,
        *,
        engine: str = "",
        flops: int = 0,
        ds1: int = 0,
        manifest_sha256: bytes = bytes(32),
        layout_sha256: bytes | None = None,
    ) -> None:
        if not (1 <= len(model_id) <= 32) or not model_id.isascii():
            raise ValueError("the model id is 1 to 32 ASCII bytes, [a-z0-9][a-z0-9._-]*")
        if len(engine) > 16 or len(manifest_sha256) != 32:
            raise ValueError("the engine is at most 16 bytes and the manifest hash 32")
        self.model_id = model_id
        self.hypotheses = hypotheses
        self.engine = engine
        self.flops = flops
        self.ds1 = ds1
        self.manifest_sha256 = manifest_sha256
        self.layout_sha256 = layout_sha256 if layout_sha256 is not None else layout_v1_sha256()
        self.tensors: list[_Tensor] = []
        self.nodes: list[tuple[int, list[Val], list[int]]] = []
        self.outputs: list[tuple[str, Val, tuple[int, ...]]] = []
        self.info: list[tuple[str, str]] = []

    # -- the graph ---------------------------------------------------------------------------------------------------

    def tensor(self, name: str, dims: Sequence[int], values: Iterable[float], dtype: str = "f32") -> Val:
        """A stored tensor, row-major: ``f32`` (subnormals flushed to 0) or ``f16`` (rounded to nearest even)."""
        vals = [float(v) for v in values]
        if len(vals) != math.prod(dims):
            raise ValueError(f"tensor {name}: {len(vals)} values for dims {tuple(dims)}")
        if not all(math.isfinite(v) for v in vals):
            raise ValueError(f"tensor {name} holds a non-finite value")
        if dtype == "f32":
            data = struct.pack(f"<{len(vals)}f", *(flush_f32(v) for v in vals))
            code = F32
        elif dtype == "f16":
            data = struct.pack(f"<{len(vals)}e", *vals)
            code = F16
        else:
            raise ValueError(f"dtype {dtype}: f32 or f16")
        self.tensors.append(_Tensor(name, code, tuple(int(d) for d in dims), data))
        return Val("tensor", len(self.tensors) - 1)

    def node(self, op: str, inputs: Sequence[Val], attrs: Sequence[int] = ()) -> Val:
        """A node of op set 1 (by name, as :data:`OPS` lists them); returns its output."""
        code, (lo, hi), n_attr, at_least = OPS[op]
        if not lo <= len(inputs) <= hi:
            raise ValueError(f"{op} takes {lo} to {hi} inputs, not {len(inputs)}")
        if (len(attrs) < n_attr) if at_least else (len(attrs) != n_attr):
            raise ValueError(f"{op} takes {n_attr}{' or more' if at_least else ''} attributes, not {len(attrs)}")
        self.nodes.append((code, list(inputs), [int(a) for a in attrs]))
        return Val("node", len(self.nodes) - 1)

    def output(self, name: str, value: Val, dims: Sequence[int]) -> None:
        """A graph output (``hypotheses`` ``[K, H]`` and ``confidence`` ``[K]`` are required; others may be added)."""
        self.outputs.append((name, value, tuple(int(d) for d in dims)))

    def add_info(self, key: str, value: str) -> None:
        """An information-only entry (printable ASCII, at most 1,024 bytes; keys unique)."""
        self.info.append((key, value))

    def parameters(self) -> int:
        """Every tensor's elements, summed (the header's parameter count)."""
        return sum(math.prod(t.dims) for t in self.tensors)

    # -- the bytes ---------------------------------------------------------------------------------------------------

    def _id(self, v: Val) -> int:
        n_in, n_t = len(INPUTS_V1), len(self.tensors)
        return {"input": 0, "tensor": n_in, "node": n_in + n_t}[v.kind] + v.index

    def build(self) -> bytes:
        """The file's bytes."""

        def name(out: bytearray, s: str) -> None:
            b = s.encode("ascii")
            if not 1 <= len(b) <= 64:
                raise ValueError(f"name {s!r} is not 1 to 64 bytes")
            out += bytes([len(b)]) + b

        def dims(out: bytearray, d: Sequence[int]) -> None:
            out += bytes([len(d)]) + struct.pack(f"<{len(d)}I", *d)

        tables = bytearray()
        for n, d in INPUTS_V1:
            name(tables, n)
            tables += bytes([F32])
            dims(tables, d)
        data = bytearray()
        for t in self.tensors:
            at = -(-len(data) // TENSOR_ALIGN) * TENSOR_ALIGN
            data += bytes(at - len(data))
            data += t.data
            name(tables, t.name)
            tables += bytes([t.dtype])
            dims(tables, t.dims)
            tables += struct.pack("<Q", at)
        for code, ins, attrs in self.nodes:
            tables += struct.pack("<HBBHH", code, len(ins), 1, len(attrs), 0)
            tables += struct.pack(f"<{len(ins)}I", *(self._id(v) for v in ins))
            tables += struct.pack(f"<{len(attrs)}i", *attrs)
        for n, v, d in self.outputs:
            name(tables, n)
            tables += struct.pack("<I", self._id(v))
            dims(tables, d)
        for k, v in self.info:
            name(tables, k)
            b = v.encode("ascii")
            tables += struct.pack("<H", len(b)) + b
        data_offset = -(-(HEADER_LEN + len(tables)) // DATA_ALIGN) * DATA_ALIGN
        header = bytearray()
        header += MAGIC
        header += struct.pack("<HHHHHH", FORMAT, HEADER_LEN, OP_SET, FEATURE_V1, LAYOUT_V1, LAYOUT_V1_GENERATOR)
        header += struct.pack("<BBHII", LAYOUT_V1_TEMPLATE, 0, self.hypotheses, LAYOUT_V1_WIDTH, 0)
        header += self.model_id.encode("ascii").ljust(32, b"\0")
        header += self.engine.encode("ascii").ljust(16, b"\0")
        header += self.layout_sha256
        header += struct.pack("<QQ", self.parameters(), self.flops)
        header += struct.pack("<IIII", len(INPUTS_V1), len(self.tensors), len(self.nodes), len(self.outputs))
        header += struct.pack("<QQ", data_offset, len(data))
        header += struct.pack("<HHI", self.ds1, 0, len(self.info))
        header += self.manifest_sha256
        header += bytes(24 + 32)
        assert len(header) == HEADER_LEN
        out = bytearray(header) + tables
        out += bytes(data_offset - len(out))
        out += data
        out[SHA_AT : SHA_AT + 32] = hashlib.sha256(out).digest()
        return bytes(out)


# -- reading -----------------------------------------------------------------------------------------------------------


@dataclasses.dataclass
class ParsedModel:
    """What :func:`read` finds in a file: the header's fields by name, and the five tables as the file holds them."""

    header: dict
    inputs: list[tuple[str, int, tuple[int, ...]]]
    #: ``(name, dtype code, dims, offset into the data section)``
    tensors: list[tuple[str, int, tuple[int, ...], int]]
    #: ``(op name, input value ids, attributes)``
    nodes: list[tuple[str, tuple[int, ...], tuple[int, ...]]]
    #: ``(name, value id, dims)``
    outputs: list[tuple[str, int, tuple[int, ...]]]
    info: list[tuple[str, str]]
    data: bytes

    def tensor_values(self, index: int) -> list[float]:
        """Tensor ``index``'s values widened to Python floats (exact: ``f16`` and ``f32`` both fit a double)."""
        _, dtype, dims, at = self.tensors[index]
        n = math.prod(dims)
        return list(struct.unpack_from(f"<{n}{'f' if dtype == F32 else 'e'}", self.data, at))


def read(data: bytes) -> ParsedModel:
    """Parse an ``ENTHMODL`` format 1 file's structure (see the module docstring).

    Raises ``ValueError`` for a file that is not format 1, whose SHA-256 disagrees or whose tables run past the bytes."""
    if len(data) < HEADER_LEN or data[:8] != MAGIC:
        raise ValueError("not an ENTHMODL file")
    fmt, hlen, op_set, feature, layout, generator = struct.unpack_from("<HHHHHH", data, 8)
    if fmt != FORMAT or hlen != HEADER_LEN:
        raise ValueError(f"format {fmt}, header length {hlen}: this reader reads format {FORMAT}")
    template, _, k, width, _ = struct.unpack_from("<BBHII", data, 20)
    parameters, flops = struct.unpack_from("<QQ", data, 112)
    n_in, n_t, n_n, n_o = struct.unpack_from("<IIII", data, 128)
    data_offset, data_len = struct.unpack_from("<QQ", data, 144)
    ds1, _, n_info = struct.unpack_from("<HHI", data, 160)
    if data_offset + data_len != len(data):
        raise ValueError("the file is not exactly the header's offset plus length")
    zeroed = bytearray(data)
    zeroed[SHA_AT : SHA_AT + 32] = bytes(32)
    if hashlib.sha256(zeroed).digest() != data[SHA_AT : SHA_AT + 32]:
        raise ValueError("the file's SHA-256 does not match")

    def text(raw: bytes) -> str:
        return raw.split(b"\0", 1)[0].decode("ascii")

    header = {
        "format": fmt,
        "op_set": op_set,
        "feature_set": feature,
        "layout": layout,
        "generator": generator,
        "template": template,
        "hypotheses": k,
        "width": width,
        "model_id": text(data[32:64]),
        "engine": text(data[64:80]),
        "layout_sha256": bytes(data[80:112]),
        "parameters": parameters,
        "flops": flops,
        "ds1": ds1,
        "manifest_sha256": bytes(data[168:200]),
        "sha256": bytes(data[SHA_AT : SHA_AT + 32]),
    }
    at = HEADER_LEN

    def take(n: int) -> bytes:
        nonlocal at
        if at + n > data_offset:
            raise ValueError("a table runs past the data offset")
        b = data[at : at + n]
        at += n
        return b

    def name() -> str:
        return take(take(1)[0]).decode("ascii")

    def dims() -> tuple[int, ...]:
        rank = take(1)[0]
        return struct.unpack(f"<{rank}I", take(4 * rank))

    inputs = [(name(), take(1)[0], dims()) for _ in range(n_in)]
    tensor_table = []
    for _ in range(n_t):
        n, dtype, d = name(), take(1)[0], dims()
        tensor_table.append((n, dtype, d, struct.unpack("<Q", take(8))[0]))
    nodes = []
    for _ in range(n_n):
        code, n_i, n_out, n_a, _ = struct.unpack("<HBBHH", take(8))
        if n_out != 1:
            raise ValueError("a node with other than one output")
        ids = struct.unpack(f"<{n_i}I", take(4 * n_i))
        attrs = struct.unpack(f"<{n_a}i", take(4 * n_a))
        nodes.append(({c: n for n, (c, *_) in OPS.items()}[code], ids, attrs))
    outputs = [(name(), struct.unpack("<I", take(4))[0], dims()) for _ in range(n_o)]
    info = []
    for _ in range(n_info):
        key = name()
        (n,) = struct.unpack("<H", take(2))
        info.append((key, take(n).decode("ascii")))
    return ParsedModel(header, inputs, tensor_table, nodes, outputs, info, bytes(data[data_offset:]))
