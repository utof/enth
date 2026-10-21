"""enth-ml: R3b's Python side (docs/r3.md, "R3b: the first model"). Never shipped; in no cargo or CI build.

Lane S0s (R3, freeze point F0s) declared the packages and nothing else:

* ``degrade``: the seeded degradation chain (lane DEG); held-out pools are disjoint from training's.
* ``augment``: augmentation and the data manifest (lane AUG); licences recorded per source.
* ``model``: the proposal network, K = 8 winner-takes-all hypotheses and a confidence head (lane TR).
* ``train``: training, with a manifest per run (lane TR).
* ``export``: ONNX and our model file, with a 20-input output golden (lane TR).

Features are computed in Rust by ``enth-analysis`` (through files ``tools/datagen`` writes, or ``enth-py`` if lane DG
decides so at F3s), never re-implemented here. See ``ml/README.md`` for the ML rules.
"""
