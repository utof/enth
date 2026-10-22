"""Recorded and simulated room impulse responses, made ready for the convolution (standard library only).

DEG's synthetic rooms are built as a unit impulse (the direct sound) plus a wet part whose energy is ``wet_db`` below
the direct (``degrade.ir``), so ``wet_db`` is the reverberant-to-direct energy ratio. A recorded impulse response has
no such split and carries its own microphone and loudspeaker colour in the direct part, which would filter the clip
whatever ``wet_db`` says. This module therefore turns a recording into the same shape as DEG's:

1. the **onset** is the first sample reaching a quarter of the recording's peak;
2. the **direct window** (1 ms before the onset to 2 ms after it) is replaced by a unit impulse at sample 0, so the
   clip passes through unfiltered at unit gain;
3. the **tail** is what follows the window, cut where the Schroeder energy-decay curve falls below -45 dB (at most 4 s
   from the onset) with a short fade out, resampled to the clip's rate if it differs, and scaled to energy
   ``wet_db`` below the direct. It starts ``window + 1`` samples (of the recording's rate) after the impulse, so the early
   reflections keep their delay;
4. the **RT60** is measured from the decay curve (T20 extrapolated, or T10 where the tail is too short) and recorded;
   the convolution pads the clip by two thirds of it, as DEG does.

A tail recorded at 16 kHz carries nothing above 8 kHz: the wet part is band-limited, the direct part is not. The record
says what rate the recording had.

Everything here works on lists of floats and is deterministic.
"""

from __future__ import annotations

import array
import itertools
import math
from dataclasses import dataclass, field
from pathlib import Path

from ..degrade import wavio

ONSET_FRACTION = 0.25
WINDOW_MS = 2.0  # the direct window ends this long after the onset
MIN_TAIL_MS = 4.0
CUT_DB = -45.0
CAP_S = 4.0
FADE_FRACTION = 0.05
MIN_FADE_MS = 2.0


class IrError(ValueError):
    """The recording cannot be used as a room (silent, or no tail after the direct sound)."""


@dataclass
class Analysis:
    """A recording split into direct window and tail, per chosen channel, at the recording's own rate."""

    sr: int
    channels_picked: list[int]  # indices into the recording's channels, one per output channel
    tails: list[list[float]]  # unscaled, equal length (zero padded), fade applied
    offset_samples: int  # from the unit impulse to the first tail sample, at ``sr``
    onset_ms: list[float] = field(default_factory=list)
    rt60_s: float | None = None
    rt60_method: str | None = None
    rt60_r2: float | None = None
    tail_s: float = 0.0
    source_channels: int = 1
    source_frames: int = 0


def pick_channels(source_channels: int, out_channels: int, u: float) -> list[int]:
    """Which recorded channel(s) feed each output channel: one at ``u`` for mono, that one and the next for stereo."""
    first = min(int(u * source_channels), source_channels - 1)
    if out_channels == 1:
        return [first]
    if source_channels == 1:
        return [0, 0]  # a mono recording on both sides (the record says so)
    return [first, (first + 1) % source_channels]


def onset_index(x: array.array | list[float], fraction: float = ONSET_FRACTION) -> int:
    peak = max(max(x), -min(x)) if len(x) else 0.0
    if peak <= 0.0:
        raise IrError("the impulse response is silent")
    thr = fraction * peak
    for i, v in enumerate(x):
        if abs(v) >= thr:
            return i
    raise IrError("no onset found")  # unreachable: the peak itself reaches the threshold


def decay_curve_db(x: list[float]) -> list[float]:
    """Schroeder's backward-integrated energy decay curve of ``x``, in dB re its total energy."""
    sq = [v * v for v in x]
    back = list(itertools.accumulate(reversed(sq)))
    back.reverse()
    total = back[0]
    if total <= 0.0:
        raise IrError("the impulse response has no energy after its onset")
    return [10.0 * math.log10(max(b / total, 1e-30)) for b in back]


def _fit_slope(edc: list[float], sr: int, hi_db: float, lo_db: float) -> tuple[float, float] | None:
    """Least-squares (slope in dB per second, R squared) of the decay curve between ``hi_db`` and ``lo_db``; None if it
    never gets there."""
    i0 = next((i for i, v in enumerate(edc) if v <= hi_db), None)
    i1 = next((i for i, v in enumerate(edc) if v <= lo_db), None)
    if i0 is None or i1 is None or i1 - i0 < 8:
        return None
    n = i1 - i0 + 1
    xs = [i / sr for i in range(i0, i1 + 1)]
    ys = edc[i0 : i1 + 1]
    mx, my = math.fsum(xs) / n, math.fsum(ys) / n
    sxx = math.fsum((a - mx) ** 2 for a in xs)
    syy = math.fsum((b - my) ** 2 for b in ys)
    if sxx <= 0.0 or syy <= 0.0:
        return None
    sxy = math.fsum((a - mx) * (b - my) for a, b in zip(xs, ys, strict=True))
    return sxy / sxx, (sxy * sxy) / (sxx * syy)


def estimate_rt60(edc: list[float], sr: int) -> tuple[float | None, str | None, float | None]:
    """(RT60, method, R squared of the fit): T20 (-5 to -25 dB) extrapolated to -60 dB, else T10 (-5 to -15 dB), else Nones.

    A decay that is not a straight line on the dB scale (a quick early drop and a long weak tail) fits badly: the R squared
    says how far to trust the number, and it is recorded beside it.
    """
    for name, lo in (("T20", -25.0), ("T10", -15.0)):
        fit = _fit_slope(edc, sr, -5.0, lo)
        if fit is not None and fit[0] < 0.0:
            return -60.0 / fit[0], name, fit[1]
    return None, None, None


def analyse(path: str | Path, out_channels: int, u_chan: float) -> Analysis:
    """Read the recording at ``path`` and split the picked channel(s) into direct window and tail."""
    info, data = wavio.read_wav(path)
    sr, nch = info.sample_rate, info.channels
    picked = pick_channels(nch, out_channels, u_chan)
    win = max(1, int(round(WINDOW_MS / 1000.0 * sr)))
    tails: list[list[float]] = []
    onsets: list[float] = []
    rt60s: list[tuple[float, str, float]] = []
    cut_lens: list[int] = []
    for c in picked:
        x = data[c::nch]
        on = onset_index(x)
        seg = list(x[on:])
        edc = decay_curve_db(seg)
        end = len(seg)
        below = next((i for i, v in enumerate(edc) if v < CUT_DB), None)
        if below is not None:
            end = below
        end = min(end, int(CAP_S * sr))
        tail = seg[win + 1 : end]
        if len(tail) < max(8, int(MIN_TAIL_MS / 1000.0 * sr)):
            raise IrError(f"{Path(path).name}: no tail after the direct window ({len(tail)} samples)")
        fade = max(int(FADE_FRACTION * len(tail)), int(MIN_FADE_MS / 1000.0 * sr), 1)
        fade = min(fade, len(tail))
        for k in range(fade):
            tail[len(tail) - fade + k] *= 0.5 * (1.0 + math.cos(math.pi * (k + 1) / (fade + 1)))
        tails.append(tail)
        onsets.append(round(on / sr * 1000.0, 3))
        r, m, q = estimate_rt60(edc, sr)
        if r is not None:
            rt60s.append((r, m or "", q or 0.0))
        cut_lens.append(len(tail))
    n = max(cut_lens)
    tails = [t + [0.0] * (n - len(t)) for t in tails]
    rt, method, r2 = rt60s[0] if rt60s else (None, None, None)
    return Analysis(
        sr=sr,
        channels_picked=picked,
        tails=tails,
        offset_samples=win + 1,
        onset_ms=onsets,
        rt60_s=None if rt is None else round(rt, 4),
        rt60_method=method,
        rt60_r2=None if r2 is None else round(r2, 4),
        tail_s=round(n / sr, 4),
        source_channels=nch,
        source_frames=info.frames,
    )


def interleave(tails: list[list[float]]) -> array.array:
    """Channel lists to one interleaved ``array('f')`` (the shape ``wavio`` writes and reads)."""
    c, n = len(tails), len(tails[0])
    out = array.array("f", bytes(4 * n * c))
    for k, t in enumerate(tails):
        out[k::c] = array.array("f", t)
    return out


def deinterleave(data: array.array, channels: int) -> list[list[float]]:
    return [list(data[k::channels]) for k in range(channels)]


def assemble(tails: list[list[float]], sr: int, offset_s: float, wet_db: float) -> tuple[array.array, dict[str, float | int]]:
    """The convolution kernel: a unit impulse at 0 and each channel's tail at ``offset_s``, at energy ``wet_db`` below it.

    ``sr`` is the clip's rate, and ``tails`` are already at it. Returns the interleaved kernel and facts about it.
    """
    wet_lin = 10.0 ** (wet_db / 20.0)
    d = max(1, int(round(offset_s * sr)))
    n = d + max(len(t) for t in tails)
    scaled: list[list[float]] = []
    actual: list[float] = []
    for t in tails:
        e = math.fsum(v * v for v in t)
        if e <= 0.0:
            raise IrError("the resampled tail has no energy")
        g = wet_lin / math.sqrt(e)
        h = [0.0] * n
        h[0] = 1.0
        for i, v in enumerate(t):
            h[d + i] += g * v
        scaled.append(h)
        actual.append(10.0 * math.log10(math.fsum(v * v for v in h[1:])))
    return interleave(scaled), {"samples": n, "seconds": round(n / sr, 6), "wet_db_actual": round(actual[0], 3), "offset_samples": d}
