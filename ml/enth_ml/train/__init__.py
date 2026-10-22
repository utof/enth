"""Training (lane TR; pre-declared empty by lane S0s, R3).

PyTorch, on a local GPU overnight, attended the first time and never beside heavy builds (docs/r3.md, "Training"); FP16
autocast where it pays (on the GTX 1660 Ti it does not: FP32 measured 2.4x faster, ``config.py``). Every run has a
manifest (commit, ``uv.lock`` hash, environment, the data runs' manifest hashes with their ``ds1`` version, engine,
stream keys and seeds, the config; an augmented run adds its configs, per-sample seeds and ``ffmpeg -version``). Training
is not bit-reproducible; the artefact that must be is the hashed model file. Logs stay local.

* ``shards``: reads lane DG's ``ds1`` shard files and run manifests (numpy only);
* ``data``: the training stream, the dev set in RAM, the input standardisation;
* ``loss``: masked JND-unit Huber and cross-entropy terms, the LFO swap, annealed and relaxed winner-takes-all, the
  confidence's cross-entropy on the winner;
* ``metrics``: dev errors in JND units, accuracies, WTA statistics, the confidence's ranking;
* ``config``, ``loop``, ``__main__``: ``python -m enth_ml.train run --config <file>`` (``uv sync --group train`` first).

The first measured run (2026-10-22, lane TR) trained on the desktop's GTX 1660 Ti; its numbers are in docs/r3.md's TR
model card.
"""
