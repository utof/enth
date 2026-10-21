"""The proposal network (lane TR; pre-declared empty by lane S0s, R3).

A small 2D CNN of about 1 to 4 M parameters, K = 8 winner-takes-all hypothesis head sets and a confidence head
(docs/r3.md, "The model: propose v0"), built only from the op set of ``enth-solver::propose`` (lane ONX) so that every
layer runs on our own scalar kernels. The model contract is frozen at F3s by a judge before any long training run.
"""
