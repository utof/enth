"""Training (lane TR; pre-declared empty by lane S0s, R3).

PyTorch, on the laptop's GPU overnight with FP16 autocast under a memory cap, attended the first time and never beside
heavy builds (docs/r3.md, "Training"). Every run has a manifest (commit, engine version, ``ds1`` version and stream
keys, phrase and augmentation configs, per-sample seeds, ``ffmpeg -version``, cache and ``uv.lock`` hashes). Training is
not bit-reproducible; the artefact that must be is the hashed model file. Logs stay local.
"""
