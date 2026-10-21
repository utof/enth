"""Export to ONNX and to our model file (lane TR; pre-declared empty by lane S0s, R3).

ONNX is the export and parity format (ONNX Runtime here, ``tools/model-check`` on the Rust side); the artefact is
``enth-model-<id>.bin`` (docs/r3.md, "The model file") plus a 20-input output golden. The parity of PyTorch and our
kernels on fixtures is TR's acceptance.
"""
