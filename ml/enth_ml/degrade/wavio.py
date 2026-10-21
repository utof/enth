"""A small WAV reader and writer (standard library only) for the chain's working files and measurements.

ffmpeg writes every working file as 32-bit float, which the ``wave`` module cannot read, so this parses the RIFF
chunks itself (PCM 8, 16, 24 and 32 bit; float 32 and 64 bit; the extensible header ffmpeg writes for float).
"""

from __future__ import annotations

import array
import hashlib
import math
import struct
import sys
from collections.abc import Iterable, Sequence
from dataclasses import dataclass
from pathlib import Path


class WavError(ValueError):
    """The file is not a WAV this module reads."""


@dataclass(frozen=True)
class WavInfo:
    sample_rate: int
    channels: int
    frames: int

    @property
    def duration_s(self) -> float:
        return self.frames / self.sample_rate


def read_info(path: str | Path) -> WavInfo:
    """The format of a WAV without reading its samples (walks the chunk headers)."""
    with open(path, "rb") as f:
        head = f.read(12)
        if head[:4] not in (b"RIFF", b"RF64") or head[8:12] != b"WAVE":
            raise WavError(f"{path}: not a RIFF/WAVE file")
        fmt = None
        while True:
            hdr = f.read(8)
            if len(hdr) < 8:
                raise WavError(f"{path}: no data chunk")
            cid, size = hdr[:4], struct.unpack("<I", hdr[4:])[0]
            if cid == b"fmt ":
                tag, channels, rate, _, align, bits = struct.unpack("<HHIIHH", f.read(16))
                f.seek(size - 16 + (size & 1), 1)
                fmt = (channels, rate, align)
            elif cid == b"data":
                if fmt is None:
                    raise WavError(f"{path}: data before fmt")
                channels, rate, align = fmt
                if size == 0xFFFFFFFF or size == 0:
                    size = max(Path(path).stat().st_size - f.tell(), 0)
                return WavInfo(rate, channels, size // align)
            else:
                f.seek(size + (size & 1), 1)


def read_wav(path: str | Path) -> tuple[WavInfo, array.array]:
    """The format and the interleaved samples as ``array('f')`` scaled to [-1, 1] (float files are kept as is)."""
    data = Path(path).read_bytes()
    if data[:4] not in (b"RIFF", b"RF64") or data[8:12] != b"WAVE":
        raise WavError(f"{path}: not a RIFF/WAVE file")
    pos, fmt, body = 12, None, None
    while pos + 8 <= len(data):
        cid = data[pos : pos + 4]
        size = struct.unpack("<I", data[pos + 4 : pos + 8])[0]
        chunk = data[pos + 8 : pos + 8 + size]
        if cid == b"fmt ":
            tag, channels, rate, _, _, bits = struct.unpack("<HHIIHH", chunk[:16])
            if tag == 0xFFFE and len(chunk) >= 26:
                tag = struct.unpack("<H", chunk[24:26])[0]
            fmt = (tag, channels, rate, bits)
        elif cid == b"data":
            body = chunk
            break
        pos += 8 + size + (size & 1)
    if fmt is None or body is None:
        raise WavError(f"{path}: no fmt or data chunk")
    tag, channels, rate, bits = fmt
    if channels < 1 or rate < 1:
        raise WavError(f"{path}: bad format {fmt}")
    samples = _decode(body, tag, bits, path)
    if len(samples) % channels:
        samples = samples[: len(samples) - len(samples) % channels]
    return WavInfo(rate, channels, len(samples) // channels), samples


def _decode(body: bytes, tag: int, bits: int, path: str | Path) -> array.array:
    if tag == 3 and bits in (32, 64):
        raw = array.array("f" if bits == 32 else "d")
        raw.frombytes(body[: len(body) - len(body) % (bits // 8)])
        if sys.byteorder == "big":
            raw.byteswap()
        return raw if bits == 32 else array.array("f", raw)
    if tag == 1 and bits == 16:
        raw = array.array("h")
        raw.frombytes(body[: len(body) - len(body) % 2])
        if sys.byteorder == "big":
            raw.byteswap()
        return array.array("f", (x / 32768.0 for x in raw))
    if tag == 1 and bits == 8:
        return array.array("f", ((x - 128) / 128.0 for x in body))
    if tag == 1 and bits == 24:
        n = len(body) // 3
        return array.array(
            "f",
            (int.from_bytes(body[3 * i : 3 * i + 3], "little", signed=True) / 8388608.0 for i in range(n)),
        )
    if tag == 1 and bits == 32:
        raw = array.array("i")
        raw.frombytes(body[: len(body) - len(body) % 4])
        if sys.byteorder == "big":
            raw.byteswap()
        return array.array("f", (x / 2147483648.0 for x in raw))
    raise WavError(f"{path}: unsupported WAV format tag {tag}, {bits} bits")


def write_wav_f32(path: str | Path, sample_rate: int, channels: int, samples: Sequence[float]) -> None:
    """Write interleaved samples as a 32-bit float WAV (the impulse responses ffmpeg's ``afir`` reads)."""
    raw = array.array("f", samples)
    if sys.byteorder == "big":
        raw.byteswap()
    data = raw.tobytes()
    header = (
        b"RIFF"
        + struct.pack("<I", 36 + len(data))
        + b"WAVEfmt "
        + struct.pack("<IHHIIHH", 16, 3, channels, sample_rate, sample_rate * channels * 4, channels * 4, 32)
        + b"data"
        + struct.pack("<I", len(data))
    )
    Path(path).write_bytes(header + data)


def rms(samples: Iterable[float]) -> float:
    """Root mean square over all samples of all channels (0.0 for an empty signal)."""
    values = samples if isinstance(samples, array.array) else list(samples)
    if len(values) == 0:
        return 0.0
    return math.sqrt(math.fsum(x * x for x in values) / len(values))


def peak(samples: Sequence[float]) -> float:
    """Highest absolute sample (0.0 for an empty signal)."""
    if len(samples) == 0:
        return 0.0
    return max(max(samples), -min(samples))


def to_db(x: float, floor: float = -300.0) -> float:
    """Amplitude to dB, with ``floor`` for zero."""
    return 20.0 * math.log10(x) if x > 0 else floor


def sha256_file(path: str | Path) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for block in iter(lambda: f.read(1 << 20), b""):
            h.update(block)
    return h.hexdigest()
