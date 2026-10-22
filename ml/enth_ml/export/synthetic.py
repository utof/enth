"""Deterministic synthetic inputs and weights (lane ONX), bit for bit as ``enth_solver::propose::net::synthetic_input``.

Standard library only. One SplitMix64 stream started at ``seed``; each value is ``((z >> 40) - 2**23) / 2**22``, a
multiple of 2**-22 in [-2, 2): exact in ``f32`` and never subnormal. :func:`synthetic_input` draws every ``mel`` value
(``[3, 128, 64]``, row-major) and then the five ``context`` values, so a parity fixture records only its seed (in the
model's info table, ``parity.seed``) and needs no input file. :func:`weights` draws values for a fixture's tensors from
the same generator, scaled.
"""

from __future__ import annotations

MASK = (1 << 64) - 1
MEL_LEN = 3 * 128 * 64
CONTEXT_LEN = 5


def _stream(seed: int):
    state = seed & MASK
    while True:
        state = (state + 0x9E3779B97F4A7C15) & MASK
        z = state
        z = ((z ^ (z >> 30)) * 0xBF58476D1CE4E5B9) & MASK
        z = ((z ^ (z >> 27)) * 0x94D049BB133111EB) & MASK
        yield (z ^ (z >> 31)) & MASK


def values(seed: int, n: int, scale: float = 2.0) -> list[float]:
    """``n`` values of the stream from ``seed``, each in [-scale, scale) (exact in ``f32`` when ``scale`` is a power
    of two)."""
    s = _stream(seed)
    return [((next(s) >> 40) - (1 << 23)) / (1 << 22) * (scale / 2.0) for _ in range(n)]


def synthetic_input(seed: int) -> tuple[list[float], list[float]]:
    """``(mel, context)``: the values ``synthetic_input(seed)`` gives in Rust."""
    v = values(seed, MEL_LEN + CONTEXT_LEN)
    return v[:MEL_LEN], v[MEL_LEN:]


def weights(seed: int, n: int, scale: float) -> list[float]:
    """Fixture weights: ``n`` values in [-scale, scale) from the stream at ``seed ^ 0x5EED0000`` (``tests/
    propose_support``'s ``values``)."""
    return values(seed ^ 0x5EED0000, n, scale)
