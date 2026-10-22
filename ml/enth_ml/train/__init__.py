"""Training (lane TR; pre-declared empty by lane S0s, R3).

PyTorch, on the laptop's GPU overnight with FP16 autocast under a memory cap, attended the first time and never beside
heavy builds (docs/r3.md, "Training"). Every run has a manifest (commit, engine version, ``ds1`` version and stream
keys, phrase and augmentation configs, per-sample seeds, ``ffmpeg -version``, cache and ``uv.lock`` hashes). Training is
not bit-reproducible; the artefact that must be is the hashed model file. Logs stay local.

* ``shards``: reads lane DG's ``ds1`` shard files and run manifests (numpy only);
* ``data``: the training stream, the dev set in RAM, the input standardisation;
* ``loss``: masked JND-unit Huber and cross-entropy terms, the LFO swap, annealed and relaxed winner-takes-all, the
  confidence's cross-entropy on the winner;
* ``metrics``: dev errors in JND units, accuracies, WTA statistics, the confidence's ranking;
* ``config``, ``loop``, ``__main__``: ``python -m enth_ml.train run --config <file>`` (``uv sync --group train`` first).

The first measured run (2026-10-22, lane TR) trained on the desktop's GTX 1660 Ti; its numbers are in docs/r3.md's TR
model card.
"""
