//! **Owner: lane ONX** (inference, wave 6, freeze point F3s). Pre-declared by lane S0s (R3, freeze point F0s); empty
//! until ONX writes it. R3b only: nothing in R3a calls it.
//!
//! The model's side of "create from sound" (docs/r3.md, "Our own scalar kernels"): `enth-solver::propose` runs the
//! small CNN of R3b (K = 8 winner-takes-all hypotheses and a confidence head) on **our own scalar kernels** (not
//! `tract`), pure like the rest of the crate: **no I/O, no threads, no clocks, and it builds for wasm32**. The weights
//! come in as bytes, which the shell reads from `enth-model-<id>.bin` beside the plugin (or `--model`). Inference is
//! bit-exact on every platform: plain `+` and `x` in a fixed order, no `mul_add`, transcendentals through the pinned
//! `libm`; each model file has exact output hashes, checked on Linux, Windows, macOS arm64 and wasm32 (ONX's
//! acceptance), and a header that does not match this build is refused like `code_too_new`.
//!
//! | File | Holds |
//! | --- | --- |
//! | [`pack`] | `ModelPack::parse(&[u8])`: the versioned header, sha256, fixture tests |
//! | [`ops`] | `Conv1d` and `Conv2d`, `Linear`, `LayerNorm`, the activations, pooling, softmax, residual add and concat, attention as Linear + matmul + softmax: a fixed summation order |
//! | [`net`] | `Net::run`: the layers listed in the pack |
//! | [`features`] | the model's input from `enth-analysis`'s log-mel on the scalar FFT path |
//! | [`decode`] | outputs to patches: normalised to plain through `ParamSpec`, onto the template's free set |
//!
//! Budgets: at most about 4 M parameters (f16 in the file, at most 8 MB), at most about 2 GFLOP per inference, the
//! encoder run once. The model contract (input features, outputs, the header) is frozen at F3s by a judge before any
//! long training run; after it the model file changes only with a version bump. The model's K hypotheses join the start
//! portfolio in [`create`](crate::create) (lane MS).

pub mod decode;
pub mod features;
pub mod net;
pub mod ops;
pub mod pack;
