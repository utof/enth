"""A raw tensor file, ``ENTHTNS1`` (lane ONX): the expected outputs of a parity fixture, and inputs for
``tools/model-check --inputs``.

Standard library only. Little-endian: the magic ``ENTHTNS1``, a ``u32`` count, then per tensor its name (a ``u8``
length and ASCII bytes), its rank (``u8``), its dims (``u32`` each) and its values (``f32``, row-major). The Rust side
reads it in ``tools/model-check`` and in ``enth-solver``'s ``tests/propose_support``.
"""

from __future__ import annotations

import math
import struct
from typing import Iterable, Sequence

MAGIC = b"ENTHTNS1"


def write(tensors: Sequence[tuple[str, Sequence[int], Iterable[float]]]) -> bytes:
    """The bytes of a tensor file holding ``(name, dims, values)`` entries, in order."""
    out = bytearray(MAGIC) + struct.pack("<I", len(tensors))
    for name, dims, values in tensors:
        vals = [float(v) for v in values]
        if len(vals) != math.prod(dims):
            raise ValueError(f"{name}: {len(vals)} values for dims {tuple(dims)}")
        b = name.encode("ascii")
        out += bytes([len(b)]) + b + bytes([len(dims)]) + struct.pack(f"<{len(dims)}I", *dims)
        out += struct.pack(f"<{len(vals)}f", *vals)
    return bytes(out)


def read(data: bytes) -> list[tuple[str, tuple[int, ...], list[float]]]:
    """The ``(name, dims, values)`` entries of a tensor file."""
    if data[:8] != MAGIC:
        raise ValueError("not an ENTHTNS1 file")
    (count,) = struct.unpack_from("<I", data, 8)
    at = 12
    out = []
    for _ in range(count):
        n = data[at]
        name = data[at + 1 : at + 1 + n].decode("ascii")
        at += 1 + n
        rank = data[at]
        dims = struct.unpack_from(f"<{rank}I", data, at + 1)
        at += 1 + 4 * rank
        size = math.prod(dims)
        values = list(struct.unpack_from(f"<{size}f", data, at))
        at += 4 * size
        out.append((name, tuple(dims), values))
    if at != len(data):
        raise ValueError(f"{len(data) - at} bytes after the last tensor")
    return out
