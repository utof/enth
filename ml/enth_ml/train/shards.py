"""Reading ``ds1`` shards (lane DG's files, ``tools/datagen/src/shards.rs``; docs/r3.md, "Who computes what").

A shard (format 1) is ``ENTHSHRD``, a ``u32`` format, a ``u32`` header length, the header JSON (``enth.ds1-shard/1``)
padded to byte 1024, then structure-of-arrays at multiples of 64: ``mel`` ``<f4 [n, 3, 128, 64]``, ``context``
``<f4 [n, 5]``, ``label`` ``<f4 [n, 102]``, ``mask`` ``<f4 [n, 102]`` (0 or 1), ``index``, ``seed``, ``gen_seed``
``<u8 [n]``, ``rate`` ``<u4 [n]``, ``source`` ``|u1 [n]``. A run directory has ``manifest.json``
(``enth.ds1-shards/1``): the ids a model's header copies, the per-entry JND scale and its table, and per shard the file,
counts and SHA-256.

numpy only. Python never computes a feature or a label (the ML rules); it reads them.
"""

from __future__ import annotations

import hashlib
import json
import pathlib
import struct
from dataclasses import dataclass, field

import numpy as np

from ..model import layout as L

MAGIC = b"ENTHSHRD"
FORMAT = 1
DATA_START = 1024
HEADER_SCHEMA = "enth.ds1-shard/1"
MANIFEST_SCHEMA = "enth.ds1-shards/1"
ARRAYS = (
    ("mel", "<f4", L.MEL_SHAPE),
    ("context", "<f4", (L.CONTEXT_LEN,)),
    ("label", "<f4", (L.WIDTH,)),
    ("mask", "<f4", (L.WIDTH,)),
    ("index", "<u8", ()),
    ("seed", "<u8", ()),
    ("gen_seed", "<u8", ()),
    ("rate", "<u4", ()),
    ("source", "|u1", ()),
)
SOURCES = ("v1", "preset", "uniform")


class ShardError(ValueError):
    """A file that is not a format-1 shard, or a run that disagrees with itself or with this reader."""


def read_header(path: pathlib.Path | str) -> dict:
    """The shard's header (its arrays' offsets included), from the first 1024 bytes."""
    with open(path, "rb") as fh:
        head = fh.read(DATA_START)
    if len(head) < DATA_START or head[:8] != MAGIC:
        raise ShardError(f"{path}: not a shard file")
    fmt, hlen = struct.unpack_from("<II", head, 8)
    if fmt != FORMAT:
        raise ShardError(f"{path}: shard format {fmt}; this reader reads {FORMAT}")
    if 16 + hlen > DATA_START:
        raise ShardError(f"{path}: the header is longer than its block")
    header = json.loads(head[16 : 16 + hlen])
    if header.get("schema") != HEADER_SCHEMA or header.get("format") != FORMAT:
        raise ShardError(f"{path}: unknown shard header {header.get('schema')}")
    n = int(header["n"])
    want = [(name, dtype, [n, *shape]) for name, dtype, shape in ARRAYS]
    got = [(a["name"], a["dtype"], list(a["shape"])) for a in header["arrays"]]
    if got != want:
        raise ShardError(f"{path}: the header's arrays are not format 1's")
    return header


def read_shard(path: pathlib.Path | str, names: tuple[str, ...] | None = None, mel_dtype: str = "f4") -> dict[str, np.ndarray]:
    """The shard's arrays (all, or ``names``), each read with one ``numpy.fromfile`` call.

    ``mel_dtype="f2"`` converts ``mel`` to ``float16`` after reading (half the memory; see the model card's note on
    storage)."""
    header = read_header(path)
    out: dict[str, np.ndarray] = {}
    for a in header["arrays"]:
        if names is not None and a["name"] not in names:
            continue
        count = int(np.prod(a["shape"]))
        arr = np.fromfile(path, dtype=a["dtype"], count=count, offset=int(a["offset"]))
        if arr.size != count:
            raise ShardError(f"{path}: {a['name']} is truncated")
        arr = arr.reshape(a["shape"])
        if a["name"] == "mel" and mel_dtype == "f2":
            arr = arr.astype(np.float16)
        out[a["name"]] = arr
    return out


def sha256_file(path: pathlib.Path | str, chunk: int = 1 << 22) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as fh:
        while True:
            b = fh.read(chunk)
            if not b:
                break
            h.update(b)
    return h.hexdigest()


@dataclass
class Run:
    """One run directory: its manifest and its shards."""

    path: pathlib.Path
    manifest: dict
    manifest_sha256: str

    @property
    def shards(self) -> list[tuple[pathlib.Path, int]]:
        return [(self.path / s["file"], int(s["kept"])) for s in self.manifest["shards"] if int(s["kept"]) > 0]

    @property
    def kept(self) -> int:
        return int(self.manifest["kept"])


def load_run(path: pathlib.Path | str) -> Run:
    """Read ``manifest.json`` and check it is a ds1 run on layout 1 and feature set 1."""
    path = pathlib.Path(path)
    mpath = path / "manifest.json"
    raw = mpath.read_bytes()
    m = json.loads(raw)
    if m.get("schema") != MANIFEST_SCHEMA:
        raise ShardError(f"{mpath}: schema {m.get('schema')}, not {MANIFEST_SCHEMA}")
    L.check_layout_block(m["layout"], m["feature_set"])
    jnd = m["jnd"]
    if len(jnd["scale"]) != L.WIDTH or len(jnd["table"]) != L.CONTINUOUS[1] or any(len(r) != 17 for r in jnd["table"]):
        raise ShardError(f"{mpath}: the JND block is not 102 scales and a 33 x 17 table")
    return Run(path=path, manifest=m, manifest_sha256=hashlib.sha256(raw).hexdigest())


@dataclass
class Dataset:
    """Several runs read as one set (a stream's shards, possibly from more than one run): they must agree on the ids
    and the JND scale."""

    runs: list[Run]
    role: str = ""
    shards: list[tuple[pathlib.Path, int]] = field(default_factory=list)

    def __post_init__(self) -> None:
        if not self.runs:
            raise ShardError("no run")
        first = self.runs[0].manifest
        for r in self.runs[1:]:
            for k in ("ds1", "layout", "feature_set", "jnd"):
                if r.manifest[k] != first[k]:
                    raise ShardError(f"{r.path}: its {k} differs from {self.runs[0].path}'s")
        self.shards = [s for r in self.runs for s in r.shards]

    @property
    def n(self) -> int:
        return sum(n for _, n in self.shards)

    @property
    def jnd_scale(self) -> np.ndarray:
        return np.asarray(self.runs[0].manifest["jnd"]["scale"], dtype=np.float32)

    @property
    def jnd_table(self) -> np.ndarray:
        return np.asarray(self.runs[0].manifest["jnd"]["table"], dtype=np.float32)

    def describe(self) -> dict:
        """What a training manifest records of the data (no local path beyond the run directories' names)."""
        return {
            "role": self.role,
            "samples": self.n,
            "shards": len(self.shards),
            "runs": [
                {
                    "dir": r.path.name,
                    "manifest_sha256": r.manifest_sha256,
                    "stream": r.manifest.get("stream"),
                    "run_seed": r.manifest.get("run_seed"),
                    "count": r.manifest.get("count"),
                    "kept": r.manifest.get("kept"),
                    "skips": r.manifest.get("skips"),
                    "sources": r.manifest.get("sources"),
                    "kind": r.manifest.get("kind"),
                    "engine": r.manifest.get("engine"),
                }
                for r in self.runs
            ],
            "ds1": self.runs[0].manifest.get("ds1"),
            "layout_sha256": self.runs[0].manifest["layout"]["sha256"],
            "feature_set": self.runs[0].manifest["feature_set"].get("id"),
        }


def open_dataset(dirs: list[pathlib.Path | str], role: str = "") -> Dataset:
    return Dataset(runs=[load_run(d) for d in dirs], role=role)


def streams_disjoint(a: Dataset, b: Dataset) -> bool:
    """Train and dev must come from different stream keys (docs/r3.md: splits by stream key, never index ranges)."""
    sa = {r.manifest.get("stream") for r in a.runs}
    sb = {r.manifest.get("stream") for r in b.runs}
    return not (sa & sb)


# -- writing (tests only: lane DG writes real shards) ------------------------------------------------------------------


def write_shard(path: pathlib.Path | str, arrays: dict[str, np.ndarray], meta: dict | None = None) -> int:
    """Write a format-1 shard with DG's layout (for tests and fixtures). Returns the file's size."""
    n = int(arrays["label"].shape[0])
    infos = []
    at = DATA_START
    for name, dtype, shape in ARRAYS:
        nbytes = int(np.prod([n, *shape])) * np.dtype(dtype).itemsize
        infos.append({"name": name, "dtype": dtype, "shape": [n, *shape], "offset": at, "bytes": nbytes})
        at = (at + nbytes + 63) // 64 * 64
    header = {"schema": HEADER_SCHEMA, "format": FORMAT, **(meta or {}), "n": n, "arrays": infos}
    js = json.dumps(header, separators=(",", ":")).encode()
    assert 16 + len(js) <= DATA_START
    end = infos[-1]["offset"] + infos[-1]["bytes"]
    buf = bytearray(b" " * DATA_START)
    buf[0:8] = MAGIC
    buf[8:16] = struct.pack("<II", FORMAT, len(js))
    buf[16 : 16 + len(js)] = js
    buf.extend(b"\0" * (end - DATA_START))
    for info, (name, dtype, shape) in zip(infos, ARRAYS):
        a = np.ascontiguousarray(arrays[name], dtype=dtype).reshape([n, *shape])
        buf[info["offset"] : info["offset"] + info["bytes"]] = a.tobytes()
    pathlib.Path(path).write_bytes(bytes(buf))
    return len(buf)
