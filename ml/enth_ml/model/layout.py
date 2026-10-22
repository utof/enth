"""Layout 1 (``propose-layout/1``) as the network and its loss read it (docs/r3.md, "The output: layout 1").

One hypothesis is a row of 102 values. The offsets below are the model contract's (frozen at F3s); the shard
manifest's entry names are checked against them at load (``check_entry_names``), so a reader never trains on a
layout it misreads. Standard library only.
"""

from __future__ import annotations

WIDTH = 102
LAYOUT_ID = 1
LAYOUT_NAME = "propose-layout/1"
LAYOUT_SHA256 = "2463ccea7ea768f368a80a7634369b93b00891f148560ad84989a15c8e3224a8"
FEATURE_SET_ID = 1
FEATURE_SET_NAME = "propose-logmel/1"
MEL_SHAPE = (3, 128, 64)
CONTEXT_LEN = 5

# Segments, as half-open [start, end) ranges of the row.
CONTINUOUS = (0, 33)  # normalised values, 0-1 (a sigmoid in the graph)
CATEGORICAL = (33, 66)  # 7 groups of logits
STRUCTURE = (66, 70)  # osc2, noise1, comp1, reverb1: logits, on above 0
PRESENCE = (70, 86)  # 4 sources x 4 destinations, source-major: logits
DEPTH = (86, 102)  # the same cells: (depth / 100 + 1) / 2, 0-1 (a sigmoid in the graph)

# The categorical groups: (name, start, end).
GROUPS = (
    ("voice.mode", 33, 36),
    ("osc1.wave", 36, 41),
    ("osc2.wave", 41, 46),
    ("noise1.color", 46, 48),
    ("filter1.mode", 48, 52),
    ("lfo1.shape", 52, 59),
    ("lfo2.shape", 59, 66),
)

CONTINUOUS_NAMES = (
    "voice.glide",
    "osc1.semi",
    "osc1.pw",
    "osc1.position",
    "osc2.semi",
    "osc2.fine",
    "osc2.level",
    "osc2.pw",
    "osc2.position",
    "noise1.level",
    "filter1.cutoff",
    "filter1.resonance",
    "filter1.keytrack",
    "filter1.drive",
    "comp1.threshold",
    "comp1.ratio",
    "comp1.attack",
    "comp1.release",
    "reverb1.wet",
    "reverb1.room",
    "reverb1.decay",
    "reverb1.high_cut",
    "env1.attack",
    "env1.decay",
    "env1.sustain",
    "env1.release",
    "env1.velocity",
    "env2.attack",
    "env2.decay",
    "env2.sustain",
    "env2.release",
    "lfo1.rate",
    "lfo2.rate",
)
STRUCTURE_NAMES = ("osc2", "noise1", "comp1", "reverb1")
ROUTE_SOURCES = ("env2", "lfo1", "lfo2", "velocity")
ROUTE_DESTS = ("osc1.fine", "osc1.level", "osc1.pw", "filter1.cutoff")

# Which entries of the graph's output are values in 0-1 (a sigmoid), the rest being logits.
SIGMOID_RANGES = (CONTINUOUS, DEPTH)


def _swap_pairs() -> list[tuple[int, int]]:
    """The LFO swap of the model contract: continuous 31<->32, categorical 52-58<->59-65, presence 74-77<->78-81,
    depth 90-93<->94-97 (the lfo1 and lfo2 rows of the route grid)."""
    pairs = [(31, 32)]
    pairs += [(52 + i, 59 + i) for i in range(7)]
    pairs += [(74 + i, 78 + i) for i in range(4)]
    pairs += [(90 + i, 94 + i) for i in range(4)]
    return pairs


SWAP_PAIRS = tuple(_swap_pairs())


def swap_permutation() -> list[int]:
    """``perm`` such that ``row[perm]`` is the row with lfo1 and lfo2 exchanged (an involution)."""
    perm = list(range(WIDTH))
    for a, b in SWAP_PAIRS:
        perm[a], perm[b] = b, a
    return perm


def expected_entry_names() -> list[str]:
    """The 102 entry names as `tools/datagen` writes them in the shard manifest (``layout.entries``)."""
    names = list(CONTINUOUS_NAMES)
    items = {
        "voice.mode": ("poly", "mono", "legato"),
        "osc1.wave": ("sine", "triangle", "saw", "pulse", "table"),
        "osc2.wave": ("sine", "triangle", "saw", "pulse", "table"),
        "noise1.color": ("white", "pink"),
        "filter1.mode": ("lp", "bp", "hp", "notch"),
        "lfo1.shape": ("sine", "triangle", "saw_up", "saw_down", "square", "sample_hold", "smooth_random"),
        "lfo2.shape": ("sine", "triangle", "saw_up", "saw_down", "square", "sample_hold", "smooth_random"),
    }
    for name, start, end in GROUPS:
        assert len(items[name]) == end - start, name
        names += [f"{name}={item}" for item in items[name]]
    names += [f"{m}.enabled" for m in STRUCTURE_NAMES]
    names += [f"route {s}->{d} present" for s in ROUTE_SOURCES for d in ROUTE_DESTS]
    names += [f"route {s}->{d} depth" for s in ROUTE_SOURCES for d in ROUTE_DESTS]
    assert len(names) == WIDTH
    return names


class LayoutMismatch(ValueError):
    """The shards were written for another layout or feature set than this code reads."""


def check_layout_block(layout: dict, feature_set: dict) -> None:
    """Refuse a shard manifest whose layout or feature set is not the one this code was written for."""
    if layout.get("id") != LAYOUT_ID or layout.get("width") != WIDTH:
        raise LayoutMismatch(f"layout {layout.get('id')} width {layout.get('width')}; this reader reads layout 1 (102)")
    if layout.get("sha256") != LAYOUT_SHA256:
        raise LayoutMismatch(f"layout SHA-256 {layout.get('sha256')} is not layout 1's pinned {LAYOUT_SHA256}")
    names = layout.get("entries")
    if names != expected_entry_names():
        raise LayoutMismatch("the manifest's entry names are not layout 1's in its order")
    if feature_set.get("id") != FEATURE_SET_ID:
        raise LayoutMismatch(f"feature set {feature_set.get('id')}; this reader reads feature set 1")
    dims = {i["name"]: tuple(i["dims"]) for i in feature_set.get("inputs", [])}
    if dims != {"mel": MEL_SHAPE, "context": (CONTEXT_LEN,)}:
        raise LayoutMismatch(f"feature set inputs {dims}")
