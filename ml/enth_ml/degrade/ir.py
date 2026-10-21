"""Synthetic room impulse responses: seeded decaying noise, standard library only.

An impulse response is a direct impulse (amplitude 1 at sample 0) plus a wet part of unit energy scaled by ``wet_db``,
so ``wet_db`` is the reverberant-to-direct energy ratio. The wet part is early reflections plus a noise tail whose low
band decays with the room's RT60 and whose high band (above ``damp_hz``) decays ``hf_ratio`` times as fast to -60 dB
in RT60 * hf_ratio. Two generators: ``diffuse`` scatters the early reflections at random over ``early_ms``;
``flutter`` spaces them every ``flutter_ms`` (an echo between parallel walls) under a weak tail.

The noise comes from ``random.Random.random()``, the one method the standard library promises to keep identical across
Python versions for a given seed.
"""

from __future__ import annotations

import array
import hashlib
import math
import random
from typing import Any


def synthesize(sample_rate: int, channels: int, params: dict[str, Any], seed: int) -> tuple[array.array, dict[str, Any]]:
    """The interleaved impulse response (``array('f')``) and facts about it, for ``params`` and ``seed``.

    Each channel gets its own noise (``seed + 2 * channel``, which keeps the seed's pool-parity bit).
    """
    per_channel = [_one_channel(sample_rate, params, seed + 2 * c) for c in range(channels)]
    n = max(len(h) for h in per_channel)
    out = array.array("f", bytes(4 * n * channels))
    for c, h in enumerate(per_channel):
        out[c : len(h) * channels : channels] = array.array("f", h)
    energy_direct = 1.0
    energy_wet = math.fsum(x * x for x in per_channel[0][1:])
    meta = {
        "samples": n,
        "seconds": round(n / sample_rate, 6),
        "wet_db_actual": round(10.0 * math.log10(energy_wet / energy_direct), 3),
        "sha256": hashlib.sha256(out.tobytes()).hexdigest(),
    }
    return out, meta


def _unit_energy(x: list[float]) -> list[float]:
    e = math.fsum(v * v for v in x)
    if e <= 0.0:
        return x
    g = 1.0 / math.sqrt(e)
    return [v * g for v in x]


def _one_channel(sr: int, p: dict[str, Any], seed: int) -> list[float]:
    rng = random.Random(seed)
    rt60 = float(p["rt60_s"])
    hf_ratio = float(p["hf_ratio"])
    pre = int(round(float(p.get("predelay_ms", 0.0)) / 1000.0 * sr))
    n_early = int(p["n_early"])
    er_fraction = float(p["er_fraction"])
    wet_lin = 10.0 ** (float(p["wet_db"]) / 20.0)

    # Wet part, indexed from 1 + pre (sample 0 is the direct impulse).
    tail_len = int(rt60 * sr * 1.1)
    if p["kind"] == "flutter":
        spacing = float(p["flutter_ms"]) / 1000.0 * sr
        g = 10.0 ** (-3.0 * (spacing / sr) / rt60)
        er_len = int(spacing * (n_early + 1)) + 1
        er = [0.0] * er_len
        for k in range(1, n_early + 1):
            at = int(round(k * spacing * (1.0 + 0.04 * (rng.random() - 0.5))))
            er[min(at, er_len - 1)] += (g**k) * (1.0 if rng.random() < 0.5 else -1.0)
    else:
        er_len = int(float(p["early_ms"]) / 1000.0 * sr) + 1
        er = [0.0] * er_len
        for _ in range(n_early):
            at = 1 + int(rng.random() * (er_len - 1))
            amp = (0.5 + 0.5 * rng.random()) * 10.0 ** (-0.6 * at / er_len)
            er[at] += amp if rng.random() < 0.5 else -amp

    a = 1.0 - math.exp(-2.0 * math.pi * float(p["damp_hz"]) / sr)
    d_lo = 10.0 ** (-3.0 / (rt60 * sr))
    d_hi = 10.0 ** (-3.0 / (rt60 * hf_ratio * sr))
    env_lo = env_hi = 1.0
    y = 0.0
    tail = [0.0] * tail_len
    rand = rng.random
    for i in range(tail_len):
        x = 2.0 * rand() - 1.0
        y += a * (x - y)
        env_lo *= d_lo
        env_hi *= d_hi
        tail[i] = y * env_lo + (x - y) * env_hi
    tail = _unit_energy(tail)
    er = _unit_energy(er)

    g_er, g_tail = math.sqrt(er_fraction) * wet_lin, math.sqrt(1.0 - er_fraction) * wet_lin
    n = 1 + pre + max(tail_len, er_len)
    h = [0.0] * n
    h[0] = 1.0
    base = 1 + pre
    for i, v in enumerate(tail):
        h[base + i] += g_tail * v
    for i, v in enumerate(er):
        h[base + i] += g_er * v
    return h
