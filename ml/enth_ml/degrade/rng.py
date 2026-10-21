"""Seeded randomness that does not depend on the Python version or on the machine.

``random.Random`` only promises ``random()`` across versions, and ``gauss`` and friends may change, so the chain draws
from its own small generator: SHA-256 turns ``(seed, pool, label)`` into a 64-bit state, SplitMix64 turns a state into
floats. Every stage draws from its own stream, so adding a stage never moves another stage's numbers.
"""

from __future__ import annotations

import hashlib
import math
from collections.abc import Sequence
from typing import TypeVar

T = TypeVar("T")

_MASK = (1 << 64) - 1
# Instance seeds (noise sources, impulse responses, dither) stay below 2**31: ffmpeg's anoisesrc takes its `seed` as a
# double limited to [-1, 2**32), and a per-channel offset is added to it.
_INSTANCE_BITS = 31


def derive(*parts: object) -> int:
    """A 64-bit seed from any parts (``str()`` of each, joined by a separator no part contains)."""
    data = "\x1f".join(str(p) for p in parts).encode("utf-8")
    return int.from_bytes(hashlib.sha256(data).digest()[:8], "big")


def instance_seed(seed: int, pool: str, parity: int, *labels: object) -> int:
    """A seed for one noise source, impulse response or dither stream.

    The lowest bit is the pool's ``instance_parity`` (0 for ``train``, 1 for ``heldout``), so the two pools can never
    draw the same instance whatever the master seed: they are disjoint by construction, not just unlikely to meet.
    """
    if parity not in (0, 1):
        raise ValueError("instance parity is 0 or 1")
    h = derive("instance", seed, pool, *labels) >> (64 - _INSTANCE_BITS)
    return (h & ~1) | parity


def sig6(x: float) -> float:
    """Round to 6 significant digits, so a recorded parameter is exactly the one applied (and prints short)."""
    return float(f"{x:.6g}")


class Rng:
    """SplitMix64 with the few draws the chain needs."""

    def __init__(self, seed: int) -> None:
        self._state = seed & _MASK

    def _next(self) -> int:
        self._state = (self._state + 0x9E3779B97F4A7C15) & _MASK
        z = self._state
        z = ((z ^ (z >> 30)) * 0xBF58476D1CE4E5B9) & _MASK
        z = ((z ^ (z >> 27)) * 0x94D049BB133111EB) & _MASK
        return z ^ (z >> 31)

    def random(self) -> float:
        """Uniform in [0, 1)."""
        return (self._next() >> 11) * (1.0 / (1 << 53))

    def uniform(self, lo: float, hi: float) -> float:
        return lo + (hi - lo) * self.random()

    def log_uniform(self, lo: float, hi: float) -> float:
        if lo <= 0 or hi <= 0:
            raise ValueError("log-uniform needs positive bounds")
        return math.exp(self.uniform(math.log(lo), math.log(hi)))

    def randint(self, lo: int, hi: int) -> int:
        """Uniform integer in [lo, hi], both included."""
        return lo + int(self.random() * (hi - lo + 1))

    def chance(self, p: float) -> bool:
        return self.random() < p

    def choice(self, items: Sequence[T]) -> T:
        return items[int(self.random() * len(items))]

    def weighted(self, items: Sequence[T], weights: Sequence[float]) -> T:
        total = sum(weights)
        x = self.random() * total
        acc = 0.0
        for item, w in zip(items, weights, strict=True):
            acc += w
            if x < acc:
                return item
        return items[-1]
