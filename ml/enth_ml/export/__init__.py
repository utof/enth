"""Export to ONNX and to our model file (lane TR; pre-declared empty by lane S0s, R3).

ONNX is the export and parity format (ONNX Runtime here, ``tools/model-check`` on the Rust side); the artefact is
``enth-model-<id>.bin`` (docs/r3.md, "The model file") plus a 20-input output golden. The parity of PyTorch and our
kernels on fixtures is TR's acceptance.

Lane ONX added (R3b, the kernels' parity with ONNX Runtime):

* ``enthmodl``: ``ModelWriter``, a writer of ``ENTHMODL`` format 1 (standard library only; it reproduces ``pack.rs``'s
  pinned fixture byte for byte), which TR's exporter may write through;
* ``tensors``: the raw tensor file (``ENTHTNS1``) for expected outputs and ``tools/model-check --inputs``;
* ``synthetic``: deterministic inputs and weights, bit for bit as ``propose::net::synthetic_input``;
* ``onnx_import``: ONNX (batch axis 1) to ``ENTHMODL``, for graphs inside op set 1 (needs the ``parity`` group);
* ``parity``: the ONNX Runtime fixtures under ``crates/enth-solver/tests/fixtures/propose/``.

Lane EXP added (R3b, the pilot model's export and its parity with our kernels):

* ``propose`` (needs torch): propose v0 to ``ENTHMODL``: BatchNorm folded and the weights rounded once to what the file
  stores, the graph written from that model, the header's ids and the info table from the run manifest, the reference
  outputs of a parity run, real dev inputs as a tensor file, and the committed tiny fixture (``exp_tiny``);
* ``enthmodl.read`` and ``tensors.write_arrays`` / ``read_arrays``: a reader of the file's structure, and numpy-array
  tensor files (a parity run over thousands of inputs).
"""
