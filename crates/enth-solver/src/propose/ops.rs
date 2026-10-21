//! **Owner: lane ONX** (freeze point F3s). Pre-declared by lane S0s (R3); empty until ONX writes it.
//!
//! Our scalar kernels (docs/r3.md, "Our own scalar kernels"): `Conv1d` and `Conv2d` (stride, padding, dilation,
//! groups), `Linear`, `LayerNorm` (`BatchNorm` is folded at export), `ReLU`, `GELU`, `SiLU`, tanh, sigmoid, max,
//! average and global pooling, softmax, residual add and concat, and multi-head attention as `Linear` + matmul +
//! softmax. A fixed summation order, no `mul_add` (clippy bans it), transcendentals through the pinned `libm`: 512 of
//! 512 outputs identical on x86-64, `target-cpu=native` and wasm32 in report 03's measurement. f16 weights widen to
//! f32 exactly.
