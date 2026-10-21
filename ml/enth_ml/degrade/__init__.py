"""The degradation chain (lane DEG; pre-declared empty by lane S0s, R3).

A seeded chain that follows a real signal path (docs/r3.md, "Augmentation and licences"): the same seed gives the same
bytes, and the held-out pools (the codecs, impulse responses and noises GR3 uses) are disjoint from the training pools,
which a test asserts. LGPL and GPL encoders (LAME, shine, ffmpeg) run as subprocesses only. Degraded synth inputs are
scored against the clean render.
"""
