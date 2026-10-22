//! **Owner: lane F3C** (the model contract, freeze point F3s); lane ONX runs what it reads. Pre-declared by lane S0s
//! (R3).
//!
//! The model file's reader (docs/r3.md, "The model contract (F3s)"): [`ModelPack::parse`] takes the bytes of
//! `enth-model-<id>.bin` (the shell reads the file; the solver does no I/O), checks the header, the SHA-256, the
//! tables and the budgets, and checks the header against **this build**: the output layout's hash
//! ([`decode::OutputLayout`](super::decode::OutputLayout)), the feature set's inputs
//! ([`features::SPEC_V1`]) and the output shapes. Any refusal means "Create from sound"
//! runs R3a (no proposals) and says why, with the error's [`code`](PackError::code) and [`hint`](PackError::hint).
//!
//! # The file (`ENTHMODL`, format 1)
//!
//! Little-endian throughout. A 256-byte header, five tables, zero padding to a multiple of 64, the data section,
//! and nothing after it:
//!
//! | Offset | Field | Type | Rule |
//! | --- | --- | --- | --- |
//! | 0 | magic | `[u8; 8]` | `ENTHMODL` |
//! | 8 | format | `u16` | 1; a newer one is refused (`model_too_new`) before anything else is read |
//! | 10 | header length | `u16` | 256 |
//! | 12 | op set | `u16` | 1 to [`OP_SET`] (the kernels' op list, below) |
//! | 14 | feature set | `u16` | 1 to [`LATEST_FEATURE`] |
//! | 16 | output layout | `u16` | 1 to [`LATEST_LAYOUT`] |
//! | 18 | generator | `u16` | the layout's generator (1) |
//! | 20 | template | `u8` | the layout's template (0) |
//! | 21 | reserved | `u8` | 0 |
//! | 22 | hypotheses K | `u16` | 1 to [`MAX_HYPOTHESES`] (8 for propose v0; the model's K, not create's K branches) |
//! | 24 | width H | `u32` | the layout's [`width`](super::decode::OutputLayout::width) |
//! | 28 | reserved | `u32` | 0 |
//! | 32 | model id | `[u8; 32]` | ASCII `[a-z0-9][a-z0-9._-]*`, NUL-padded |
//! | 64 | engine | `[u8; 16]` | the engine version that rendered the training data, ASCII, NUL-padded (information only) |
//! | 80 | layout SHA-256 | `[u8; 32]` | the layout's [`sha256`](super::decode::OutputLayout::sha256) (the "catalogue hash") |
//! | 112 | parameters | `u64` | the stored tensors' elements, summed; at most [`MAX_PARAMETERS`] |
//! | 120 | FLOPs | `u64` | per inference, counted as below (information only: lane ONX recounts from the shapes at load and refuses a graph over [`MAX_FLOPS`]) |
//! | 128 | inputs, tensors, nodes, outputs | `u32` × 4 | the tables' lengths |
//! | 144 | data offset | `u64` | a multiple of 64, at or after the tables' end |
//! | 152 | data length | `u64` | the file is exactly offset + length bytes |
//! | 160 | `ds1` version | `u16` | the training data's sampler version (0: none, as in a test fixture; information only) |
//! | 162 | reserved | `u16` | 0 |
//! | 164 | info entries | `u32` | the info table's length |
//! | 168 | training manifest SHA-256 | `[u8; 32]` | the training run's manifest (zeros: none; information only) |
//! | 200 | reserved | `[u8; 24]` | 0 |
//! | 224 | SHA-256 | `[u8; 32]` | of the whole file with these 32 bytes read as zeros |
//!
//! Then, at byte 256, in order (a name is a `u8` length, 1 to 64, then that many printable ASCII bytes; dims are
//! `u32`, each at least 1, at most 8 of them):
//!
//! - **inputs**: name, dtype `u8`, rank `u8`, dims. They must be the feature set's, in its order (`mel`
//!   `[3, 128, 64]`, `context` `[5]`, `f32`);
//! - **tensors** (the weights): name, dtype `u8` (1 `f32`, 2 `f16`), rank `u8`, dims, offset `u64` into the data
//!   section, a multiple of [`TENSOR_ALIGN`] (64); stored in table order without overlap; each at most
//!   [`MAX_PARAMETERS`] elements; every value finite, and no `f32` value subnormal (the exporter flushes them to 0,
//!   so no kernel reads a value DAZ would change);
//! - **nodes**: op `u16`, inputs `u8`, outputs `u8`, attributes `u16`, reserved `u16` (0), then the input value
//!   ids (`u32` each) and the attributes (`i32` each, checked as the op table says);
//! - **outputs**: name, value id `u32`, rank `u8`, dims (`f32`, at most [`MAX_OUTPUT_ELEMENTS`] elements).
//!   `hypotheses` `[K, H]` and `confidence` `[K]` must be there; others (an embedding for retrieval) may be added;
//! - **info** (information only, covered by the SHA-256): a key (a name), then a value: a `u16` length, 0 to
//!   [`MAX_INFO_VALUE`], then that many printable ASCII bytes. Keys are unique; a reader ignores the keys it does
//!   not know, so new metadata (a training commit, data licences) never needs a format bump.
//!
//! Every count is checked against the bytes before the data section (each entry's smallest size) before anything is
//! allocated, and every product of dims is checked, so a damaged or hostile file is refused, never a panic or an
//! abort (a file with nodes = `u32::MAX` and a valid SHA-256 used to abort the process).
//!
//! **Value ids** are numbered in one sequence: the inputs, then the tensors, then each node's outputs in node
//! order. A node may only read ids numbered before it (the graph is in topological order, each value written once).
//! `f16` weights widen to `f32` exactly ([`f16_to_f32`]); every kernel computes in `f32`.
//!
//! # Op set 1
//!
//! The ops the kernels run ([`OpCode`]; lane ONX's `ops.rs`), in `f32`, **without the batch axis**: shapes below
//! are written without it (a feature map is `[C, L]` or `[C, H, W]`), and the exporter remaps every axis. Axes are
//! never negative. Padding is explicit (no `auto_pad`), pooling and convolution use **floor** mode only. Attributes
//! are `i32`s in the order given; a float attribute is its `f32` bit pattern. Where ONNX has an operator, this is its
//! meaning; where the table pins a choice, the table wins (`PyTorch`'s defaults differ in places, marked).
//!
//! | Code | Op | Inputs → output | Attributes | Meaning |
//! | --- | --- | --- | --- | --- |
//! | 1 | `Conv1d` | x `[Ci, L]`, w `[Co, Ci/g, k]`, b `[Co]`? → `[Co, Lo]` | stride ≥ 1, pad begin ≥ 0, pad end ≥ 0, dilation ≥ 1, groups ≥ 1 | ONNX `Conv` (cross-correlation, zero padding); `Lo = ⌊(L + pb + pe − d·(k − 1) − 1) / s⌋ + 1` |
//! | 2 | `Conv2d` | x `[Ci, H, W]`, w `[Co, Ci/g, kh, kw]`, b `[Co]`? → `[Co, Ho, Wo]` | stride h, stride w, pad top, pad left, pad bottom, pad right, dilation h, dilation w, groups (as `Conv1d`'s) | ONNX `Conv`, per axis as `Conv1d` |
//! | 3 | `Linear` | x `[…, n]`, w `[m, n]`, b `[m]`? → `[…, m]` | — | `x · wᵀ + b` over the last axis (ONNX `Gemm` on a 2-D x, else `MatMul` + `Add`) |
//! | 4 | `MatMul` | a `[…, p, q]`, b `[…, q, r]` → `[…, p, r]` | — | ONNX `MatMul` with both ranks ≥ 2; the leading axes broadcast as numpy's |
//! | 5 | `LayerNorm` | x `[…, n]`, γ `[n]`, β `[n]` → `[…, n]` | ε (`f32` bits, finite, > 0) | over the last axis: `μ = Σx / n`, `σ² = Σ(x − μ)² / n` (biased), `y = (x − μ) / √(σ² + ε) · γ + β` (ε inside the root) |
//! | 6 | `Relu` | x → same | — | `max(x, 0)` |
//! | 7 | `Gelu` | x → same | — | the erf form, `0.5 · x · (1 + erf(x / √2))` (ONNX `Gelu` at opset 20, `approximate = none`; not `PyTorch`'s `tanh` form) |
//! | 8 | `Silu` | x → same | — | `x · sigmoid(x)` (no ONNX op: `Mul` + `Sigmoid` there) |
//! | 9 | `Tanh` | x → same | — | `tanh(x)` |
//! | 10 | `Sigmoid` | x → same | — | `1 / (1 + exp(−x))` |
//! | 11, 12 | `MaxPool1d`, `AvgPool1d` | x `[C, L]` → `[C, Lo]` | kernel ≥ 1, stride ≥ 1, pad begin, pad end (each 0 to kernel − 1) | max over the window, padding never chosen; the **mean over the window's in-bounds elements**, padding not counted (ONNX `count_include_pad = 0`; `PyTorch`'s default `count_include_pad = True` differs) |
//! | 13, 14 | `MaxPool2d`, `AvgPool2d` | x `[C, H, W]` → `[C, Ho, Wo]` | kernel h, kernel w, stride h, stride w, pad top, pad left, pad bottom, pad right (as 1-D) | as 1-D, per axis |
//! | 15, 16 | `GlobalAvgPool`, `GlobalMaxPool` | x `[C, …]` → `[C, 1, …]` | — | the mean or maximum over every axis after the first, which stay as 1s (ONNX `GlobalAveragePool`, `GlobalMaxPool`) |
//! | 17 | `Softmax` | x → same | axis ≥ 0 | `exp(x − max) / Σ exp(x − max)` along the axis (the maximum subtracted first) |
//! | 18, 19 | `Add`, `Mul` | a, b → the broadcast shape | — | element-wise, broadcasting as numpy's (right-aligned) |
//! | 20 | `Concat` | 2 to 32 tensors → joined | axis ≥ 0 | ONNX `Concat`; every other axis equal |
//! | 21 | `Reshape` | x → the shape | the shape: 1 to 8 dims, each ≥ 1 or one −1 (inferred) | ONNX `Reshape` without its 0 (which would copy a dim): 0 is refused |
//! | 22 | `Transpose` | x → permuted | the permutation of 0..rank | output axis i is input axis `perm[i]` |
//! | 23 | `Sub` | a, b → the broadcast shape | — | `a − b`, broadcasting as `Add` |
//! | 24 | `Slice` | x → sliced | axis ≥ 0, start ≥ 0, end > start, step ≥ 1 | elements `start, start + step, …` below `min(end, dim)` along the axis (ONNX `Slice` with one axis and non-negative indices) |
//!
//! `BatchNorm` is folded into the preceding convolution at export. Multi-head attention is `Linear` + `Reshape` +
//! `Transpose` + `MatMul` + `Softmax`; pooling over one axis is `Transpose` + `GlobalAvgPool` or an `AvgPool2d` with
//! a full-length kernel; squeeze-excitation is `Mul` with broadcasting; a class token is `Concat` and `Slice`. A new
//! op is a new op set (codes are only appended); a build runs every op set up to its own.
//!
//! **FLOPs** (the header's count, and ONX's recount): `2 ·` the multiply-adds of `Conv1d`, `Conv2d`, `Linear` and
//! `MatMul` (a convolution: `2 · Co · Lo · (Ci / g) · k`, per output position), plus 1 per output element of every
//! other op (an activation, a pool's output, a normalisation, a softmax, an element-wise op; `Reshape`,
//! `Transpose`, `Concat` and `Slice` count 0). Information only in the header; ONX refuses on its own count.
//!
//! **Determinism** (docs/r3.md's determinism contract, point 1): each kernel sums in a fixed order of ONX's, the
//! same on every platform, with plain `+` and `×` (no `mul_add`) and transcendentals (`exp`, `erf`, `tanh`) through
//! the pinned `libm`. **No kernel depends on FTZ or DAZ**: weights are never subnormal (checked here), inputs never
//! are (feature set 1), and a kernel whose result can underflow (`exp` in `Softmax`, `Sigmoid`, `Silu`, `Gelu`'s
//! `erf` tail) flushes a subnormal result to 0 in code; ONX runs its output hashes with FTZ off, FTZ only and
//! FTZ+DAZ, as renders are (`tests/propose_ftz.rs` does it for the features and decoding). A later build may change
//! a summation order, which moves a model's output hashes but never a replay (results record their starts as
//! patches).

use core::fmt;

use enth_challenge::spec::{LATEST_GENERATOR, LATEST_TEMPLATE};
use sha2::{Digest, Sha256};

use super::decode::{LATEST_LAYOUT, LayoutError, OutputLayout};
use super::features::{self, LATEST_FEATURE};

/// The file's magic bytes.
pub const MAGIC: [u8; 8] = *b"ENTHMODL";

/// The file format this build reads. A newer one is refused ([`PackError::TooNew`]) before anything else is read.
pub const FORMAT: u16 = 1;

/// Bytes of the fixed header.
pub const HEADER_LEN: usize = 256;

/// The newest op set this build's kernels run.
pub const OP_SET: u16 = 1;

/// The most stored parameters (elements of every tensor): about 4 M (2²²), docs/r3.md's budget.
pub const MAX_PARAMETERS: u64 = 1 << 22;

/// The most floating-point operations per inference: about 2 G (2³¹), docs/r3.md's budget, counted as the
/// [module docs](self) say. Lane ONX's net recounts from the shapes at load and refuses a graph over it
/// ([`Budget::Flops`]); the header's figure is information only.
pub const MAX_FLOPS: u64 = 1 << 31;

/// The largest file read: every parameter in `f32` (16 MiB) plus 1 MiB of tables. Shipped models store their
/// weights in `f16` (at most 8 MB).
pub const MAX_FILE_BYTES: usize = 17 << 20;

/// The most hypotheses a model may propose.
pub const MAX_HYPOTHESES: u16 = 64;

/// The most axes of a tensor.
pub const MAX_RANK: usize = 8;

/// The data section starts at a multiple of this.
pub const DATA_ALIGN: u64 = 64;

/// Every tensor starts at a multiple of this in the data section (so a reader may map the weights in place).
pub const TENSOR_ALIGN: u64 = 64;

/// The most elements an output may declare (`hypotheses` is K · H, at most 64 · 102).
pub const MAX_OUTPUT_ELEMENTS: u64 = 1 << 20;

/// The longest value in the info table (bytes).
pub const MAX_INFO_VALUE: usize = 1024;

/// The output holding the K hypothesis rows, `[K, H]`.
pub const HYPOTHESES_OUTPUT: &str = "hypotheses";

/// The output holding the K confidence logits, `[K]`: the hypotheses are proposed in descending order (ties: the
/// lower index).
pub const CONFIDENCE_OUTPUT: &str = "confidence";

/// Where the SHA-256 field sits in the header.
const SHA_AT: usize = 224;

/// The longest name in a table.
const MAX_NAME: usize = 64;

/// The element type of a stored tensor or an input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum DType {
    /// IEEE 754 binary32.
    F32 = 1,
    /// IEEE 754 binary16, widened to `f32` exactly when read.
    F16 = 2,
}

impl DType {
    /// The type with this code, if format 1 has it.
    #[must_use]
    pub fn from_code(code: u8) -> Option<DType> {
        match code {
            1 => Some(DType::F32),
            2 => Some(DType::F16),
            _ => None,
        }
    }

    /// Bytes per element.
    #[must_use]
    pub fn size(self) -> usize {
        match self {
            DType::F32 => 4,
            DType::F16 => 2,
        }
    }
}

/// How many attributes an op takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Arity {
    /// Exactly this many.
    Exactly(u16),
    /// At least this many (a shape, a permutation).
    AtLeast(u16),
}

/// An op of the kernels' op set (see the [module docs](self)).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u16)]
#[non_exhaustive]
pub enum OpCode {
    /// 1-D convolution.
    Conv1d = 1,
    /// 2-D convolution.
    Conv2d = 2,
    /// `x · wᵀ + b` over the last axis.
    Linear = 3,
    /// Matrix product over the last two axes.
    MatMul = 4,
    /// Layer normalisation over the last axis.
    LayerNorm = 5,
    /// `max(x, 0)`.
    Relu = 6,
    /// The erf form of GELU.
    Gelu = 7,
    /// `x · sigmoid(x)`.
    Silu = 8,
    /// Hyperbolic tangent.
    Tanh = 9,
    /// Logistic sigmoid.
    Sigmoid = 10,
    /// 1-D max pooling.
    MaxPool1d = 11,
    /// 1-D average pooling (padding not counted).
    AvgPool1d = 12,
    /// 2-D max pooling.
    MaxPool2d = 13,
    /// 2-D average pooling (padding not counted).
    AvgPool2d = 14,
    /// Mean over every axis after the first.
    GlobalAvgPool = 15,
    /// Maximum over every axis after the first.
    GlobalMaxPool = 16,
    /// Softmax along an axis.
    Softmax = 17,
    /// Element-wise sum, broadcasting.
    Add = 18,
    /// Element-wise product, broadcasting.
    Mul = 19,
    /// Concatenation along an axis.
    Concat = 20,
    /// A new shape over the same elements.
    Reshape = 21,
    /// A permutation of the axes.
    Transpose = 22,
    /// Element-wise difference, broadcasting.
    Sub = 23,
    /// A strided range along one axis.
    Slice = 24,
}

/// What a node of an op must look like: its inputs, outputs and attributes, and the op set that introduced it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OpInfo {
    /// The op.
    pub op: OpCode,
    /// Its name in docs and tools.
    pub name: &'static str,
    /// The first op set that has it.
    pub since: u16,
    /// The fewest and the most inputs.
    pub inputs: (u8, u8),
    /// Its outputs.
    pub outputs: u8,
    /// Its attributes.
    pub attrs: Arity,
}

const fn info(op: OpCode, name: &'static str, inputs: (u8, u8), attrs: Arity) -> OpInfo {
    OpInfo {
        op,
        name,
        since: 1,
        inputs,
        outputs: 1,
        attrs,
    }
}

/// Op set 1, by code (index = code − 1).
pub const OPS: [OpInfo; 24] = [
    info(OpCode::Conv1d, "Conv1d", (2, 3), Arity::Exactly(5)),
    info(OpCode::Conv2d, "Conv2d", (2, 3), Arity::Exactly(9)),
    info(OpCode::Linear, "Linear", (2, 3), Arity::Exactly(0)),
    info(OpCode::MatMul, "MatMul", (2, 2), Arity::Exactly(0)),
    info(OpCode::LayerNorm, "LayerNorm", (3, 3), Arity::Exactly(1)),
    info(OpCode::Relu, "Relu", (1, 1), Arity::Exactly(0)),
    info(OpCode::Gelu, "Gelu", (1, 1), Arity::Exactly(0)),
    info(OpCode::Silu, "Silu", (1, 1), Arity::Exactly(0)),
    info(OpCode::Tanh, "Tanh", (1, 1), Arity::Exactly(0)),
    info(OpCode::Sigmoid, "Sigmoid", (1, 1), Arity::Exactly(0)),
    info(OpCode::MaxPool1d, "MaxPool1d", (1, 1), Arity::Exactly(4)),
    info(OpCode::AvgPool1d, "AvgPool1d", (1, 1), Arity::Exactly(4)),
    info(OpCode::MaxPool2d, "MaxPool2d", (1, 1), Arity::Exactly(8)),
    info(OpCode::AvgPool2d, "AvgPool2d", (1, 1), Arity::Exactly(8)),
    info(
        OpCode::GlobalAvgPool,
        "GlobalAvgPool",
        (1, 1),
        Arity::Exactly(0),
    ),
    info(
        OpCode::GlobalMaxPool,
        "GlobalMaxPool",
        (1, 1),
        Arity::Exactly(0),
    ),
    info(OpCode::Softmax, "Softmax", (1, 1), Arity::Exactly(1)),
    info(OpCode::Add, "Add", (2, 2), Arity::Exactly(0)),
    info(OpCode::Mul, "Mul", (2, 2), Arity::Exactly(0)),
    info(OpCode::Concat, "Concat", (2, 32), Arity::Exactly(1)),
    info(OpCode::Reshape, "Reshape", (1, 1), Arity::AtLeast(1)),
    info(OpCode::Transpose, "Transpose", (1, 1), Arity::AtLeast(1)),
    info(OpCode::Sub, "Sub", (2, 2), Arity::Exactly(0)),
    info(OpCode::Slice, "Slice", (1, 1), Arity::Exactly(4)),
];

impl OpCode {
    /// The op with this code, if any op set this build runs has it.
    #[must_use]
    pub fn from_code(code: u16) -> Option<OpCode> {
        let i = usize::from(code).checked_sub(1)?;
        OPS.get(i).map(|o| o.op)
    }

    /// Its row of the op table.
    #[must_use]
    pub fn info(self) -> &'static OpInfo {
        &OPS[self as usize - 1]
    }
}

/// The fixed header of a model file (see the [module docs](self)).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelHeader {
    /// The file format ([`FORMAT`]).
    pub format: u16,
    /// The op set the graph needs.
    pub op_set: u16,
    /// The feature set of its inputs.
    pub feature: u16,
    /// The output layout of its hypotheses.
    pub layout: u16,
    /// The generator whose draw set the layout is.
    pub generator: u16,
    /// The template the layout decodes onto.
    pub template: u8,
    /// K, the hypotheses per inference.
    pub hypotheses: u16,
    /// H, the values per hypothesis.
    pub width: u32,
    /// The model's id (what a result's provenance names).
    pub model_id: String,
    /// The engine version that rendered its training data (information only: never refused on).
    pub engine: String,
    /// The layout's SHA-256 it was trained against.
    pub layout_sha256: [u8; 32],
    /// The stored parameters.
    pub parameters: u64,
    /// FLOPs per inference, as the exporter counted them.
    pub flops: u64,
    /// The input table's length.
    pub inputs: u32,
    /// The tensor table's length.
    pub tensors: u32,
    /// The node list's length.
    pub nodes: u32,
    /// The output table's length.
    pub outputs: u32,
    /// Where the data section starts.
    pub data_offset: u64,
    /// The data section's length.
    pub data_len: u64,
    /// The `ds1` version of the training data (0: none; information only).
    pub ds1: u16,
    /// The info table's length.
    pub info: u32,
    /// The training run's manifest SHA-256 (zeros: none; information only).
    pub manifest_sha256: [u8; 32],
    /// The file's SHA-256 (with its own field read as zeros).
    pub sha256: [u8; 32],
}

/// A part of the file a newer build may introduce.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    /// The file format.
    Format,
    /// The kernels' op set.
    OpSet,
    /// The feature set.
    Feature,
    /// The output layout.
    Layout,
    /// The generator.
    Generator,
    /// The template.
    Template,
}

impl Part {
    /// A lowercase name.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Part::Format => "format",
            Part::OpSet => "op set",
            Part::Feature => "feature set",
            Part::Layout => "output layout",
            Part::Generator => "generator",
            Part::Template => "template",
        }
    }
}

/// A budget a model must fit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Budget {
    /// [`MAX_FILE_BYTES`].
    File,
    /// [`MAX_PARAMETERS`].
    Parameters,
    /// [`MAX_FLOPS`].
    Flops,
}

/// What of the header does not match this build.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mismatch {
    /// The layout's SHA-256 is not this build's: the catalogue or the template moved since the model was trained.
    LayoutHash,
    /// This build's catalogue or template cannot build the layout at all (a renamed key, a removed module).
    Catalogue,
    /// The width is not the layout's.
    Width,
    /// The inputs are not the feature set's.
    Inputs,
    /// The `hypotheses` or `confidence` output is missing or not `[K, H]` and `[K]`.
    Outputs,
}

/// Why a model file is refused. **Every refusal means create runs without the model (R3a) and says so.**
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PackError {
    /// Shorter than a header, or not the magic: not a model file.
    NotAModel,
    /// Made by a newer enth: a format, op set, feature set, layout, generator or template this build lacks.
    TooNew {
        /// What is newer.
        part: Part,
        /// The file's.
        found: u16,
        /// This build's newest.
        supported: u16,
    },
    /// Shorter than its header says.
    Truncated {
        /// Bytes the header says.
        expected: u64,
        /// Bytes there are.
        got: u64,
    },
    /// The SHA-256 does not match: damaged.
    Hash,
    /// A field or table no writer produces (named).
    Malformed(String),
    /// Over a budget.
    TooLarge {
        /// Which budget.
        what: Budget,
        /// The file's.
        found: u64,
        /// The budget.
        limit: u64,
    },
    /// Well formed, but made for another catalogue, template, feature set or output shape than this build's.
    Mismatch {
        /// What does not match.
        what: Mismatch,
        /// The detail.
        detail: String,
    },
}

impl PackError {
    /// A stable snake-case code. A newer format and a newer generator are both `model_too_new`, as a newer code
    /// version and generator are both `code_too_new`.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            PackError::NotAModel => "not_a_model",
            PackError::TooNew { .. } => "model_too_new",
            PackError::Truncated { .. } => "model_truncated",
            PackError::Hash => "model_hash",
            PackError::Malformed(_) => "model_malformed",
            PackError::TooLarge { .. } => "model_too_large",
            PackError::Mismatch { .. } => "model_mismatch",
        }
    }

    /// What to do about it, in one line.
    #[must_use]
    pub fn hint(&self) -> String {
        match self {
            PackError::NotAModel => {
                "this is not an enth model file (enth-model-<id>.bin); create runs without a model".to_owned()
            }
            PackError::TooNew { .. } => {
                "the model was made for a newer enth: update enth to use it; create runs without it meanwhile"
                    .to_owned()
            }
            PackError::Truncated { .. } | PackError::Hash | PackError::Malformed(_) => {
                "the model file is damaged or incomplete: download it again; create runs without it meanwhile"
                    .to_owned()
            }
            PackError::TooLarge { .. } => {
                "the model is over this build's size budget; create runs without it".to_owned()
            }
            PackError::Mismatch { .. } => {
                "the model was made for another enth catalogue: use the model released with this build; create runs without it meanwhile"
                    .to_owned()
            }
        }
    }
}

impl fmt::Display for PackError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PackError::NotAModel => f.write_str("not an enth model file (bad magic or cut short)"),
            PackError::TooNew {
                part,
                found,
                supported,
            } => write!(
                f,
                "model {} {found} is newer than this build reads ({supported})",
                part.name()
            ),
            PackError::Truncated { expected, got } => {
                write!(
                    f,
                    "the model file is {got} bytes; its header says {expected}"
                )
            }
            PackError::Hash => f.write_str("the model file does not match its SHA-256"),
            PackError::Malformed(why) => write!(f, "the model file is malformed: {why}"),
            PackError::TooLarge { what, found, limit } => {
                let name = match what {
                    Budget::File => "bytes",
                    Budget::Parameters => "parameters",
                    Budget::Flops => "FLOPs per inference",
                };
                write!(f, "the model has {found} {name}; the budget is {limit}")
            }
            PackError::Mismatch { what, detail } => {
                let name = match what {
                    Mismatch::LayoutHash => "its output layout's hash is not this build's",
                    Mismatch::Catalogue => "this build cannot build its output layout",
                    Mismatch::Width => "its hypothesis width is not the layout's",
                    Mismatch::Inputs => "its inputs are not the feature set's",
                    Mismatch::Outputs => {
                        "its outputs are not [K, H] hypotheses and [K] confidences"
                    }
                };
                write!(f, "the model does not match this build: {name} ({detail})")
            }
        }
    }
}

impl std::error::Error for PackError {}

fn malformed(why: impl Into<String>) -> PackError {
    PackError::Malformed(why.into())
}

/// An input of the graph (a feature set's tensor).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValueSpec {
    /// Its name.
    pub name: String,
    /// Its element type (inputs are `f32`).
    pub dtype: DType,
    /// Its shape.
    pub dims: Vec<u32>,
}

/// A stored tensor (weights).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TensorEntry {
    /// Its name (the exporter's, for tools and parity reports).
    pub name: String,
    /// Its element type in the file.
    pub dtype: DType,
    /// Its shape.
    pub dims: Vec<u32>,
    /// Where its bytes start in the data section.
    pub offset: u64,
}

impl TensorEntry {
    /// Its elements (saturating: a parsed tensor holds at most [`MAX_PARAMETERS`]).
    #[must_use]
    pub fn elements(&self) -> u64 {
        elements(&self.dims).unwrap_or(u64::MAX)
    }

    /// Its bytes in the file (saturating).
    #[must_use]
    pub fn byte_len(&self) -> u64 {
        self.elements().saturating_mul(self.dtype.size() as u64)
    }
}

/// The product of `dims`, or `None` when it overflows `u64`.
fn elements(dims: &[u32]) -> Option<u64> {
    dims.iter()
        .try_fold(1u64, |acc, d| acc.checked_mul(u64::from(*d)))
}

/// One node of the graph: an op, the value ids it reads, how many values it writes (the next ids), its attributes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node {
    /// The op.
    pub op: OpCode,
    /// The value ids it reads, in the op's order.
    pub inputs: Vec<u32>,
    /// The first value id it writes.
    pub first_output: u32,
    /// The values it writes.
    pub outputs: u8,
    /// Its attributes, in the op's order.
    pub attrs: Vec<i32>,
}

/// An output of the graph.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutputSpec {
    /// Its name ([`HYPOTHESES_OUTPUT`], [`CONFIDENCE_OUTPUT`], or an additional one).
    pub name: String,
    /// The value id it reads.
    pub value: u32,
    /// Its shape (`f32`).
    pub dims: Vec<u32>,
}

/// A model file, read and checked against this build (see the [module docs](self)).
#[derive(Clone, Debug, PartialEq)]
pub struct ModelPack {
    header: ModelHeader,
    inputs: Vec<ValueSpec>,
    tensors: Vec<TensorEntry>,
    nodes: Vec<Node>,
    outputs: Vec<OutputSpec>,
    info: Vec<(String, String)>,
    data: Vec<u8>,
    layout: OutputLayout,
}

/// A bounds-checked little-endian cursor over the tables.
struct Reader<'a> {
    b: &'a [u8],
    at: usize,
    end: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], PackError> {
        let stop = self
            .at
            .checked_add(n)
            .filter(|s| *s <= self.end)
            .ok_or_else(|| malformed("the tables run past the data section"))?;
        let out = &self.b[self.at..stop];
        self.at = stop;
        Ok(out)
    }
    fn u8(&mut self) -> Result<u8, PackError> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, PackError> {
        Ok(u16::from_le_bytes(self.array()?))
    }
    fn u32(&mut self) -> Result<u32, PackError> {
        Ok(u32::from_le_bytes(self.array()?))
    }
    fn u64(&mut self) -> Result<u64, PackError> {
        Ok(u64::from_le_bytes(self.array()?))
    }
    fn i32(&mut self) -> Result<i32, PackError> {
        Ok(i32::from_le_bytes(self.array()?))
    }
    fn array<const N: usize>(&mut self) -> Result<[u8; N], PackError> {
        let mut out = [0u8; N];
        out.copy_from_slice(self.take(N)?);
        Ok(out)
    }
    fn name(&mut self) -> Result<String, PackError> {
        let n = usize::from(self.u8()?);
        let bytes = self.take(n)?;
        if n == 0 || n > MAX_NAME || !bytes.iter().all(|c| (0x21..=0x7e).contains(c)) {
            return Err(malformed("a name is not 1 to 64 printable ASCII bytes"));
        }
        Ok(bytes.iter().map(|c| char::from(*c)).collect())
    }
    fn dims(&mut self) -> Result<Vec<u32>, PackError> {
        let rank = usize::from(self.u8()?);
        if rank == 0 || rank > MAX_RANK {
            return Err(malformed(format!("a rank of {rank}")));
        }
        let dims = (0..rank)
            .map(|_| self.u32())
            .collect::<Result<Vec<_>, _>>()?;
        if dims.contains(&0) {
            return Err(malformed("a dimension of 0"));
        }
        Ok(dims)
    }
}

/// The smallest input entry: a 1-byte name (2), dtype, rank, one dim.
const MIN_INPUT_BYTES: u64 = 2 + 1 + 1 + 4;
/// The smallest tensor entry: a 1-byte name (2), dtype, rank, one dim, offset.
const MIN_TENSOR_BYTES: u64 = 2 + 1 + 1 + 4 + 8;
/// The smallest node: op, counts, reserved, one input id.
const MIN_NODE_BYTES: u64 = 2 + 1 + 1 + 2 + 2 + 4;
/// The smallest output entry: a 1-byte name (2), value id, rank, one dim.
const MIN_OUTPUT_BYTES: u64 = 2 + 4 + 1 + 4;
/// The smallest info entry: a 1-byte key (2) and an empty value (2).
const MIN_INFO_BYTES: u64 = 2 + 2;

/// A node's attributes against its op's rules (the [module docs](self)' table): what needs no shapes is checked
/// here, the rest by ONX's net when it infers shapes.
fn check_attrs(op: OpCode, a: &[i32]) -> Result<(), PackError> {
    let pos = |xs: &[i32]| xs.iter().all(|v| *v >= 1);
    let nonneg = |xs: &[i32]| xs.iter().all(|v| *v >= 0);
    let (ok, why) = match op {
        // stride, pad begin, pad end, dilation, groups
        OpCode::Conv1d => (
            pos(&a[0..1]) && nonneg(&a[1..3]) && pos(&a[3..5]),
            "stride, dilation and groups are at least 1, padding at least 0",
        ),
        // strides, pads (top, left, bottom, right), dilations, groups
        OpCode::Conv2d => (
            pos(&a[0..2]) && nonneg(&a[2..6]) && pos(&a[6..9]),
            "strides, dilations and groups are at least 1, padding at least 0",
        ),
        OpCode::LayerNorm => {
            let eps = f32::from_bits(a[0] as u32);
            (
                eps.is_finite() && eps > 0.0,
                "epsilon is a finite value above 0",
            )
        }
        // kernel, stride, pad begin, pad end
        OpCode::MaxPool1d | OpCode::AvgPool1d => (
            pos(&a[0..2]) && nonneg(&a[2..4]) && a[2] < a[0] && a[3] < a[0],
            "kernel and stride are at least 1, each pad 0 to kernel - 1",
        ),
        // kernel h, kernel w, stride h, stride w, pad top, left, bottom, right
        OpCode::MaxPool2d | OpCode::AvgPool2d => (
            pos(&a[0..4])
                && nonneg(&a[4..8])
                && a[4] < a[0]
                && a[6] < a[0]
                && a[5] < a[1]
                && a[7] < a[1],
            "kernels and strides are at least 1, each pad 0 to its kernel - 1",
        ),
        OpCode::Softmax | OpCode::Concat => (a[0] >= 0, "the axis is not negative"),
        OpCode::Reshape => (
            a.iter().filter(|v| **v == -1).count() <= 1 && a.iter().all(|v| *v == -1 || *v >= 1),
            "each dim is at least 1, or one is -1 (0 is refused)",
        ),
        OpCode::Transpose => {
            let mut seen = [false; MAX_RANK];
            let perm = a.iter().all(|v| match usize::try_from(*v) {
                Ok(i) if i < a.len() && !seen[i] => {
                    seen[i] = true;
                    true
                }
                _ => false,
            });
            (perm, "the attributes are a permutation of 0..rank")
        }
        // axis, start, end, step
        OpCode::Slice => (
            a[0] >= 0 && a[1] >= 0 && a[2] > a[1] && a[3] >= 1,
            "axis and start are at least 0, end above start, step at least 1",
        ),
        _ => (true, ""),
    };
    if ok {
        Ok(())
    } else {
        Err(malformed(format!("a {} node: {why}", op.info().name)))
    }
}

fn u16_at(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}
fn u32_at(b: &[u8], at: usize) -> u32 {
    let mut a = [0u8; 4];
    a.copy_from_slice(&b[at..at + 4]);
    u32::from_le_bytes(a)
}
fn u64_at(b: &[u8], at: usize) -> u64 {
    let mut a = [0u8; 8];
    a.copy_from_slice(&b[at..at + 8]);
    u64::from_le_bytes(a)
}
fn array_at<const N: usize>(b: &[u8], at: usize) -> [u8; N] {
    let mut a = [0u8; N];
    a.copy_from_slice(&b[at..at + N]);
    a
}

/// A NUL-padded ASCII field: the text before the first NUL, and whether the rest is NUL and the text printable.
fn padded_text(field: &[u8]) -> Option<String> {
    let n = field.iter().position(|c| *c == 0).unwrap_or(field.len());
    let (text, pad) = field.split_at(n);
    (pad.iter().all(|c| *c == 0) && text.iter().all(|c| (0x20..=0x7e).contains(c)))
        .then(|| text.iter().map(|c| char::from(*c)).collect())
}

fn valid_model_id(id: &str) -> bool {
    let b = id.as_bytes();
    !b.is_empty()
        && (b[0].is_ascii_lowercase() || b[0].is_ascii_digit())
        && b.iter().all(|c| {
            c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, b'.' | b'_' | b'-')
        })
}

/// The SHA-256 of a model file's bytes with the header's SHA-256 field read as zeros.
///
/// # Panics
/// If `bytes` is shorter than [`HEADER_LEN`].
#[must_use]
pub fn file_sha256(bytes: &[u8]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(&bytes[..SHA_AT]);
    h.update([0u8; 32]);
    h.update(&bytes[SHA_AT + 32..]);
    h.finalize().into()
}

/// An IEEE 754 binary16 value as `f32`, exactly (every binary16 value is a binary32 value).
#[must_use]
pub fn f16_to_f32(h: u16) -> f32 {
    let sign = u32::from(h >> 15) << 31;
    let exp = u32::from((h >> 10) & 0x1f);
    let man = u32::from(h & 0x3ff);
    let bits = match exp {
        0 if man == 0 => sign,
        0 => {
            // Subnormal: man · 2⁻²⁴, normalised.
            let (mut m, mut e) = (man, 113u32);
            while m & 0x400 == 0 {
                m <<= 1;
                e -= 1;
            }
            sign | (e << 23) | ((m & 0x3ff) << 13)
        }
        0x1f => sign | 0x7f80_0000 | (man << 13),
        _ => sign | ((exp + 112) << 23) | (man << 13),
    };
    f32::from_bits(bits)
}

fn too_new(part: Part, found: u16, supported: u16) -> Result<(), PackError> {
    if found == 0 {
        return Err(malformed(format!("{} 0", part.name())));
    }
    if found > supported {
        return Err(PackError::TooNew {
            part,
            found,
            supported,
        });
    }
    Ok(())
}

impl ModelHeader {
    /// Read the fixed header: the magic, the format (a newer one is refused here), and the fields as they are. No
    /// hash or table is checked (tools print a header with this; [`ModelPack::parse`] checks everything).
    ///
    /// # Errors
    /// [`PackError::NotAModel`], [`PackError::TooNew`] for a newer format, [`PackError::Malformed`] for format 0
    /// or another header length.
    pub fn peek(bytes: &[u8]) -> Result<ModelHeader, PackError> {
        if bytes.len() < HEADER_LEN || bytes[..8] != MAGIC {
            return Err(PackError::NotAModel);
        }
        let format = u16_at(bytes, 8);
        too_new(Part::Format, format, FORMAT)?;
        if usize::from(u16_at(bytes, 10)) != HEADER_LEN {
            return Err(malformed("the header is not 256 bytes"));
        }
        let text = |at: usize, n: usize, what: &str| {
            padded_text(&bytes[at..at + n])
                .ok_or_else(|| malformed(format!("the {what} is not NUL-padded ASCII")))
        };
        Ok(ModelHeader {
            format,
            op_set: u16_at(bytes, 12),
            feature: u16_at(bytes, 14),
            layout: u16_at(bytes, 16),
            generator: u16_at(bytes, 18),
            template: bytes[20],
            hypotheses: u16_at(bytes, 22),
            width: u32_at(bytes, 24),
            model_id: text(32, 32, "model id")?,
            engine: text(64, 16, "engine version")?,
            layout_sha256: array_at(bytes, 80),
            parameters: u64_at(bytes, 112),
            flops: u64_at(bytes, 120),
            inputs: u32_at(bytes, 128),
            tensors: u32_at(bytes, 132),
            nodes: u32_at(bytes, 136),
            outputs: u32_at(bytes, 140),
            data_offset: u64_at(bytes, 144),
            data_len: u64_at(bytes, 152),
            ds1: u16_at(bytes, 160),
            info: u32_at(bytes, 164),
            manifest_sha256: array_at(bytes, 168),
            sha256: array_at(bytes, SHA_AT),
        })
    }
}

impl ModelPack {
    /// Read a model file and check it against this build, in this order: the magic; the format; the file's size
    /// ([`MAX_FILE_BYTES`]); the length the header gives; the SHA-256; the reserved fields; the op set, feature set,
    /// layout, generator and template (newer ones are `model_too_new`); the model id, K and the parameter budget;
    /// the tables (their counts against the bytes they need first), each node's attributes, the data section and
    /// every weight; then this build's layout hash, width, inputs and outputs (`model_mismatch`). Never a panic or
    /// an abort, whatever the bytes (a property test feeds it mutated files with valid hashes).
    ///
    /// # Errors
    /// See [`PackError`]: every refusal means create runs R3a and says why.
    pub fn parse(bytes: &[u8]) -> Result<ModelPack, PackError> {
        let header = ModelHeader::peek(bytes)?;
        if bytes.len() > MAX_FILE_BYTES {
            return Err(PackError::TooLarge {
                what: Budget::File,
                found: bytes.len() as u64,
                limit: MAX_FILE_BYTES as u64,
            });
        }
        let expected = header
            .data_offset
            .checked_add(header.data_len)
            .ok_or_else(|| malformed("the data section's end overflows"))?;
        let got = bytes.len() as u64;
        if got < expected {
            return Err(PackError::Truncated { expected, got });
        }
        if got > expected {
            return Err(malformed(format!(
                "{} bytes after the data section",
                got - expected
            )));
        }
        if file_sha256(bytes) != header.sha256 {
            return Err(PackError::Hash);
        }
        if bytes[21] != 0
            || u32_at(bytes, 28) != 0
            || u16_at(bytes, 162) != 0
            || bytes[200..SHA_AT].iter().any(|c| *c != 0)
        {
            return Err(malformed("a reserved field is not zero"));
        }
        too_new(Part::OpSet, header.op_set, OP_SET)?;
        too_new(Part::Feature, header.feature, LATEST_FEATURE)?;
        too_new(Part::Layout, header.layout, LATEST_LAYOUT)?;
        too_new(Part::Generator, header.generator, LATEST_GENERATOR)?;
        if header.template > LATEST_TEMPLATE {
            return Err(PackError::TooNew {
                part: Part::Template,
                found: u16::from(header.template),
                supported: u16::from(LATEST_TEMPLATE),
            });
        }
        if !valid_model_id(&header.model_id) {
            return Err(malformed("the model id is not [a-z0-9][a-z0-9._-]*"));
        }
        if header.hypotheses == 0 || header.hypotheses > MAX_HYPOTHESES {
            return Err(malformed(format!("{} hypotheses", header.hypotheses)));
        }
        if header.parameters > MAX_PARAMETERS {
            return Err(PackError::TooLarge {
                what: Budget::Parameters,
                found: header.parameters,
                limit: MAX_PARAMETERS,
            });
        }
        if !header.data_offset.is_multiple_of(DATA_ALIGN) || header.data_offset < HEADER_LEN as u64
        {
            return Err(malformed(
                "the data section is not at a multiple of 64 after the header",
            ));
        }
        // Every count against the bytes its entries need at least, before anything is allocated or read.
        let table_bytes = header.data_offset - HEADER_LEN as u64;
        let least = [
            (header.inputs, MIN_INPUT_BYTES),
            (header.tensors, MIN_TENSOR_BYTES),
            (header.nodes, MIN_NODE_BYTES),
            (header.outputs, MIN_OUTPUT_BYTES),
            (header.info, MIN_INFO_BYTES),
        ]
        .iter()
        .try_fold(0u64, |acc, (n, size)| {
            acc.checked_add(u64::from(*n).checked_mul(*size)?)
        });
        if least.is_none_or(|need| need > table_bytes) {
            return Err(malformed(format!(
                "{} inputs, {} tensors, {} nodes, {} outputs and {} info entries cannot fit in {table_bytes} bytes of tables",
                header.inputs, header.tensors, header.nodes, header.outputs, header.info
            )));
        }
        let data_start = header.data_offset as usize;
        let mut r = Reader {
            b: bytes,
            at: HEADER_LEN,
            end: data_start,
        };
        let inputs = (0..header.inputs)
            .map(|_| {
                let name = r.name()?;
                let dtype = DType::from_code(r.u8()?)
                    .filter(|d| *d == DType::F32)
                    .ok_or_else(|| malformed("an input is not f32"))?;
                Ok(ValueSpec {
                    name,
                    dtype,
                    dims: r.dims()?,
                })
            })
            .collect::<Result<Vec<_>, PackError>>()?;
        let tensors = (0..header.tensors)
            .map(|_| {
                let name = r.name()?;
                let dtype = DType::from_code(r.u8()?)
                    .ok_or_else(|| malformed("a tensor's dtype is not f32 or f16"))?;
                let dims = r.dims()?;
                if elements(&dims).is_none_or(|n| n > MAX_PARAMETERS) {
                    return Err(PackError::TooLarge {
                        what: Budget::Parameters,
                        found: elements(&dims).unwrap_or(u64::MAX),
                        limit: MAX_PARAMETERS,
                    });
                }
                let offset = r.u64()?;
                if !offset.is_multiple_of(TENSOR_ALIGN) {
                    return Err(malformed(format!(
                        "tensor {name} is not at a multiple of {TENSOR_ALIGN} bytes"
                    )));
                }
                Ok(TensorEntry {
                    name,
                    dtype,
                    dims,
                    offset,
                })
            })
            .collect::<Result<Vec<_>, PackError>>()?;
        let mut next = u64::from(header.inputs) + u64::from(header.tensors);
        let mut nodes = Vec::new();
        for _ in 0..header.nodes {
            let code = r.u16()?;
            let (n_in, n_out, n_attr, reserved) = (r.u8()?, r.u8()?, r.u16()?, r.u16()?);
            let op = OpCode::from_code(code)
                .filter(|op| op.info().since <= header.op_set)
                .ok_or_else(|| {
                    malformed(format!("op {code} is not in op set {}", header.op_set))
                })?;
            let info = op.info();
            let attrs_ok = match info.attrs {
                Arity::Exactly(n) => n_attr == n,
                Arity::AtLeast(n) => n_attr >= n && usize::from(n_attr) <= MAX_RANK,
            };
            if reserved != 0
                || n_in < info.inputs.0
                || n_in > info.inputs.1
                || n_out != info.outputs
                || !attrs_ok
            {
                return Err(malformed(format!(
                    "a {} node with {n_in} inputs, {n_out} outputs and {n_attr} attributes",
                    info.name
                )));
            }
            let ins = (0..n_in).map(|_| r.u32()).collect::<Result<Vec<_>, _>>()?;
            if let Some(v) = ins.iter().find(|v| u64::from(**v) >= next) {
                return Err(malformed(format!(
                    "a {} node reads value {v} before it is written",
                    info.name
                )));
            }
            let attrs = (0..n_attr)
                .map(|_| r.i32())
                .collect::<Result<Vec<_>, _>>()?;
            check_attrs(op, &attrs)?;
            let first_output = u32::try_from(next).map_err(|_| malformed("too many values"))?;
            next += u64::from(n_out);
            nodes.push(Node {
                op,
                inputs: ins,
                first_output,
                outputs: n_out,
                attrs,
            });
        }
        let outputs = (0..header.outputs)
            .map(|_| {
                let name = r.name()?;
                let value = r.u32()?;
                if u64::from(value) >= next {
                    return Err(malformed(format!("output {name} reads no value")));
                }
                let dims = r.dims()?;
                if elements(&dims).is_none_or(|n| n > MAX_OUTPUT_ELEMENTS) {
                    return Err(malformed(format!(
                        "output {name} declares more than {MAX_OUTPUT_ELEMENTS} elements"
                    )));
                }
                Ok(OutputSpec { name, value, dims })
            })
            .collect::<Result<Vec<_>, PackError>>()?;
        let info = (0..header.info)
            .map(|_| {
                let key = r.name()?;
                let n = usize::from(r.u16()?);
                let value = r.take(n)?;
                if n > MAX_INFO_VALUE || !value.iter().all(|c| (0x20..=0x7e).contains(c)) {
                    return Err(malformed(format!(
                        "info {key} is not 0 to {MAX_INFO_VALUE} printable ASCII bytes"
                    )));
                }
                Ok((key, value.iter().map(|c| char::from(*c)).collect()))
            })
            .collect::<Result<Vec<(String, String)>, PackError>>()?;
        for names in [
            inputs.iter().map(|i| i.name.as_str()).collect::<Vec<_>>(),
            outputs.iter().map(|o| o.name.as_str()).collect::<Vec<_>>(),
            info.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>(),
        ] {
            if names
                .iter()
                .enumerate()
                .any(|(i, n)| names[..i].contains(n))
            {
                return Err(malformed(
                    "two inputs, two outputs or two info entries share a name",
                ));
            }
        }
        if bytes[r.at..data_start].iter().any(|c| *c != 0) {
            return Err(malformed("the padding before the data section is not zero"));
        }
        let data = &bytes[data_start..];
        let mut end = 0u64;
        let mut parameters = 0u64;
        for t in &tensors {
            let len = t.byte_len();
            if t.offset < end
                || t.offset
                    .checked_add(len)
                    .is_none_or(|e| e > header.data_len)
            {
                return Err(malformed(format!(
                    "tensor {} overlaps another or runs past the data section",
                    t.name
                )));
            }
            end = t.offset + len;
            parameters = parameters.saturating_add(t.elements());
            if parameters > MAX_PARAMETERS {
                return Err(PackError::TooLarge {
                    what: Budget::Parameters,
                    found: parameters,
                    limit: MAX_PARAMETERS,
                });
            }
            let slice = &data[t.offset as usize..end as usize];
            let ok = match t.dtype {
                DType::F32 => slice.as_chunks::<4>().0.iter().all(|c| {
                    let v = f32::from_le_bytes(*c);
                    v.is_finite() && !v.is_subnormal()
                }),
                DType::F16 => slice
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .all(|c| u16::from_le_bytes(*c) & 0x7c00 != 0x7c00),
            };
            if !ok {
                return Err(malformed(format!(
                    "tensor {} holds a non-finite or subnormal value",
                    t.name
                )));
            }
        }
        if parameters != header.parameters {
            return Err(malformed(format!(
                "the tensors hold {parameters} parameters; the header says {}",
                header.parameters
            )));
        }
        let layout = check_build(&header, &inputs, &outputs)?;
        Ok(ModelPack {
            header,
            inputs,
            tensors,
            nodes,
            outputs,
            info,
            data: data.to_vec(),
            layout,
        })
    }

    /// The header.
    #[must_use]
    pub fn header(&self) -> &ModelHeader {
        &self.header
    }

    /// The graph's inputs, in value-id order (the feature set's).
    #[must_use]
    pub fn inputs(&self) -> &[ValueSpec] {
        &self.inputs
    }

    /// The stored tensors, in value-id order after the inputs.
    #[must_use]
    pub fn tensors(&self) -> &[TensorEntry] {
        &self.tensors
    }

    /// The nodes, in topological order.
    #[must_use]
    pub fn nodes(&self) -> &[Node] {
        &self.nodes
    }

    /// The graph's outputs.
    #[must_use]
    pub fn outputs(&self) -> &[OutputSpec] {
        &self.outputs
    }

    /// The output with this name.
    #[must_use]
    pub fn output(&self, name: &str) -> Option<&OutputSpec> {
        self.outputs.iter().find(|o| o.name == name)
    }

    /// The info table (information only), in file order.
    #[must_use]
    pub fn info(&self) -> &[(String, String)] {
        &self.info
    }

    /// The info value under `key`, if the file has it.
    #[must_use]
    pub fn info_value(&self, key: &str) -> Option<&str> {
        self.info
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    /// The file's SHA-256 (what a result records beside the model id).
    #[must_use]
    pub fn sha256(&self) -> [u8; 32] {
        self.header.sha256
    }

    /// This build's output layout, which the model's header matched: what decodes its hypotheses.
    #[must_use]
    pub fn layout(&self) -> &OutputLayout {
        &self.layout
    }

    /// Tensor `index`'s values as `f32`, in row-major order (`f16` widened exactly).
    ///
    /// # Panics
    /// If `index` is out of range.
    #[must_use]
    pub fn tensor_f32(&self, index: usize) -> Vec<f32> {
        let t = &self.tensors[index];
        let bytes = &self.data[t.offset as usize..(t.offset + t.byte_len()) as usize];
        match t.dtype {
            DType::F32 => bytes
                .as_chunks::<4>()
                .0
                .iter()
                .map(|c| f32::from_le_bytes(*c))
                .collect(),
            DType::F16 => bytes
                .as_chunks::<2>()
                .0
                .iter()
                .map(|c| f16_to_f32(u16::from_le_bytes(*c)))
                .collect(),
        }
    }
}

/// The header against this build: the layout it names must build here with the same hash and width, the inputs
/// must be the feature set's, and the outputs must hold `[K, H]` hypotheses and `[K]` confidences.
fn check_build(
    header: &ModelHeader,
    inputs: &[ValueSpec],
    outputs: &[OutputSpec],
) -> Result<OutputLayout, PackError> {
    let mismatch = |what, detail: String| PackError::Mismatch { what, detail };
    let layout = OutputLayout::build(header.layout, header.generator, header.template).map_err(
        |e| match e {
            LayoutError::Missing { .. } => mismatch(Mismatch::Catalogue, e.to_string()),
            // A layout named with another generator or template than its own: no writer makes it.
            _ => malformed(e.to_string()),
        },
    )?;
    if layout.sha256() != header.layout_sha256 {
        return Err(mismatch(
            Mismatch::LayoutHash,
            format!("layout {} as this build builds it", header.layout),
        ));
    }
    if header.width as usize != layout.width() {
        return Err(mismatch(
            Mismatch::Width,
            format!("{} against {}", header.width, layout.width()),
        ));
    }
    let spec = features::spec(header.feature).ok_or_else(|| {
        mismatch(
            Mismatch::Inputs,
            format!("feature set {} is not computed here", header.feature),
        )
    })?;
    let same = inputs.len() == spec.inputs.len()
        && inputs
            .iter()
            .zip(spec.inputs)
            .all(|(i, s)| i.name == s.name && i.dims == s.dims);
    if !same {
        return Err(mismatch(
            Mismatch::Inputs,
            format!("feature set {} is {}", spec.id, spec.name),
        ));
    }
    let k = u32::from(header.hypotheses);
    let has = |name: &str, dims: &[u32]| {
        outputs
            .iter()
            .any(|o| o.name == name && o.dims.as_slice() == dims)
    };
    if !has(HYPOTHESES_OUTPUT, &[k, header.width]) || !has(CONFIDENCE_OUTPUT, &[k]) {
        return Err(mismatch(
            Mismatch::Outputs,
            format!("K {k}, H {}", header.width),
        ));
    }
    Ok(layout)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The format-1 fixture's length and SHA-256 (`the_format_1_fixture_is_pinned`).
    const FIXTURE_LEN: usize = 8864;
    const FIXTURE_SHA256: &str = "641b6454a0c41573df5a2f92689028d844ac14cdb83a5b79d39fed746d9b15b7";

    /// A model file written field by field from the format's table, independently of the reader: two `Linear`
    /// heads on `context` (an `f16` weight for the hypotheses, reshaped to `[K, H]`, and an `f32` one for the
    /// confidences).
    #[derive(Clone)]
    struct Fixture {
        format: u16,
        op_set: u16,
        feature: u16,
        layout: u16,
        generator: u16,
        template: u8,
        k: u16,
        width: u32,
        model_id: &'static str,
        layout_sha256: [u8; 32],
        parameters: Option<u64>,
        flops: u64,
        mel_dims: Vec<u32>,
        reserved: u8,
        /// Node edits: the op code of the first node, the first node's first input.
        first_op: u16,
        first_input: u32,
        weight: u16,
        /// The f32 confidence weight's first value.
        conf0: f32,
        /// The second node's op and attributes (a Reshape to `[K, H]` by default).
        second_op: u16,
        second_attrs: Option<Vec<i32>>,
        /// The `ds1` version and the info table.
        ds1: u16,
        info: Vec<(&'static str, &'static str)>,
        /// The second tensor's offset (None: the next multiple of 64 after the first).
        w2_at: Option<u64>,
        /// Write the SHA-256 (false leaves zeros).
        hash: bool,
    }

    impl Default for Fixture {
        fn default() -> Self {
            let l = OutputLayout::v1();
            Fixture {
                format: FORMAT,
                op_set: 1,
                feature: 1,
                layout: 1,
                generator: 1,
                template: 0,
                k: 8,
                width: l.width() as u32,
                model_id: "propose-v0-test",
                layout_sha256: l.sha256(),
                parameters: None,
                flops: 10_000,
                mel_dims: vec![3, 128, 64],
                reserved: 0,
                first_op: 3,
                first_input: 1,
                weight: 0x3c00,
                conf0: 0.0,
                second_op: 21,
                second_attrs: None,
                ds1: 1,
                info: vec![("commit", "0123abc"), ("licence", "renders only")],
                w2_at: None,
                hash: true,
            }
        }
    }

    fn name(out: &mut Vec<u8>, s: &str) {
        out.push(s.len() as u8);
        out.extend_from_slice(s.as_bytes());
    }

    fn dims(out: &mut Vec<u8>, d: &[u32]) {
        out.push(d.len() as u8);
        for x in d {
            out.extend_from_slice(&x.to_le_bytes());
        }
    }

    fn bytes(f: &Fixture) -> Vec<u8> {
        let (k, h) = (u32::from(f.k), f.width);
        let w1 = k * h * 5; // f16 [K·H, 5]
        let w2 = k * 5; // f32 [K, 5]
        let mut tables = Vec::new();
        // inputs: mel, context
        name(&mut tables, "mel");
        tables.push(1);
        dims(&mut tables, &f.mel_dims);
        name(&mut tables, "context");
        tables.push(1);
        dims(&mut tables, &[5]);
        // tensors: w1 (f16) at 0, w2 (f32) after it, at the next multiple of 64
        let w2_at = f
            .w2_at
            .unwrap_or(u64::from(w1 * 2).next_multiple_of(TENSOR_ALIGN));
        name(&mut tables, "head.hyp.weight");
        tables.push(2);
        dims(&mut tables, &[k * h, 5]);
        tables.extend_from_slice(&0u64.to_le_bytes());
        name(&mut tables, "head.conf.weight");
        tables.push(1);
        dims(&mut tables, &[k, 5]);
        tables.extend_from_slice(&w2_at.to_le_bytes());
        // nodes: values 0 mel, 1 context, 2 w1, 3 w2; 4 = Linear(1, 2); 5 = Reshape(4); 6 = Linear(1, 3)
        let node = |out: &mut Vec<u8>, op: u16, ins: &[u32], attrs: &[i32]| {
            out.extend_from_slice(&op.to_le_bytes());
            out.push(ins.len() as u8);
            out.push(1);
            out.extend_from_slice(&(attrs.len() as u16).to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            for i in ins {
                out.extend_from_slice(&i.to_le_bytes());
            }
            for a in attrs {
                out.extend_from_slice(&a.to_le_bytes());
            }
        };
        node(&mut tables, f.first_op, &[f.first_input, 2], &[]);
        let reshape = vec![k as i32, h as i32];
        node(
            &mut tables,
            f.second_op,
            &[4],
            f.second_attrs.as_ref().unwrap_or(&reshape),
        );
        node(&mut tables, 3, &[1, 3], &[]);
        // outputs
        name(&mut tables, "hypotheses");
        tables.extend_from_slice(&5u32.to_le_bytes());
        dims(&mut tables, &[k, h]);
        name(&mut tables, "confidence");
        tables.extend_from_slice(&6u32.to_le_bytes());
        dims(&mut tables, &[k]);
        // info
        for (key, value) in &f.info {
            name(&mut tables, key);
            tables.extend_from_slice(&(value.len() as u16).to_le_bytes());
            tables.extend_from_slice(value.as_bytes());
        }
        let data_offset = (HEADER_LEN + tables.len()).next_multiple_of(64);
        let mut data = Vec::new();
        for _ in 0..w1 {
            data.extend_from_slice(&f.weight.to_le_bytes());
        }
        data.resize(w2_at as usize, 0);
        for i in 0..w2 {
            let v = if i == 0 { f.conf0 } else { i as f32 * 0.25 };
            data.extend_from_slice(&v.to_le_bytes());
        }
        let parameters = f.parameters.unwrap_or(u64::from(w1 + w2));
        let mut out = Vec::new();
        out.extend_from_slice(b"ENTHMODL");
        out.extend_from_slice(&f.format.to_le_bytes());
        out.extend_from_slice(&256u16.to_le_bytes());
        out.extend_from_slice(&f.op_set.to_le_bytes());
        out.extend_from_slice(&f.feature.to_le_bytes());
        out.extend_from_slice(&f.layout.to_le_bytes());
        out.extend_from_slice(&f.generator.to_le_bytes());
        out.push(f.template);
        out.push(f.reserved);
        out.extend_from_slice(&f.k.to_le_bytes());
        out.extend_from_slice(&f.width.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        let mut id = [0u8; 32];
        id[..f.model_id.len()].copy_from_slice(f.model_id.as_bytes());
        out.extend_from_slice(&id);
        let mut engine = [0u8; 16];
        engine[..5].copy_from_slice(b"0.2.0");
        out.extend_from_slice(&engine);
        out.extend_from_slice(&f.layout_sha256);
        out.extend_from_slice(&parameters.to_le_bytes());
        out.extend_from_slice(&f.flops.to_le_bytes());
        for n in [2u32, 2, 3, 2] {
            out.extend_from_slice(&n.to_le_bytes());
        }
        out.extend_from_slice(&(data_offset as u64).to_le_bytes());
        out.extend_from_slice(&(data.len() as u64).to_le_bytes());
        out.extend_from_slice(&f.ds1.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&(f.info.len() as u32).to_le_bytes());
        out.extend_from_slice(&[0x5a; 32]); // the training manifest's SHA-256
        out.extend_from_slice(&[0u8; 24]);
        out.extend_from_slice(&[0u8; 32]);
        assert_eq!(out.len(), HEADER_LEN);
        out.extend_from_slice(&tables);
        out.resize(data_offset, 0);
        out.extend_from_slice(&data);
        if f.hash {
            let sha: [u8; 32] = Sha256::digest(&out).into();
            out[224..256].copy_from_slice(&sha);
        }
        out
    }

    fn parse(f: &Fixture) -> Result<ModelPack, PackError> {
        ModelPack::parse(&bytes(f))
    }

    fn rehash(mut b: Vec<u8>) -> Vec<u8> {
        b[224..256].fill(0);
        let sha: [u8; 32] = Sha256::digest(&b).into();
        b[224..256].copy_from_slice(&sha);
        b
    }

    #[test]
    fn a_valid_file_reads_back() {
        let b = bytes(&Fixture::default());
        let m = ModelPack::parse(&b).unwrap();
        let h = m.header();
        assert_eq!(
            (
                h.format,
                h.op_set,
                h.feature,
                h.layout,
                h.generator,
                h.template
            ),
            (1, 1, 1, 1, 1, 0)
        );
        assert_eq!((h.hypotheses, h.width), (8, 102));
        assert_eq!(h.model_id, "propose-v0-test");
        assert_eq!(h.engine, "0.2.0");
        assert_eq!(h.parameters, 8 * 102 * 5 + 8 * 5);
        assert_eq!(m.sha256(), file_sha256(&b));
        assert_eq!(m.inputs()[0].dims, [3, 128, 64]);
        assert_eq!(m.tensors()[0].dtype, DType::F16);
        assert_eq!(m.nodes().len(), 3);
        assert_eq!(m.nodes()[1].op, OpCode::Reshape);
        assert_eq!(m.nodes()[2].first_output, 6);
        assert_eq!(m.output("hypotheses").unwrap().value, 5);
        assert!(m.tensor_f32(0).iter().all(|v| *v == 1.0));
        assert_eq!(m.tensor_f32(1)[3], 0.75);
        assert_eq!(m.layout().width(), 102);
        assert_eq!(ModelHeader::peek(&b).unwrap(), *h);
        assert_eq!((h.ds1, h.info, h.manifest_sha256), (1, 2, [0x5a; 32]));
        assert_eq!(m.info_value("commit"), Some("0123abc"));
        assert_eq!(m.info().len(), 2);
        assert!(m.tensors().iter().all(|t| t.offset % TENSOR_ALIGN == 0));
    }

    /// The format-1 fixture's bytes are pinned, so the byte layout cannot move together with the reader in one
    /// commit (a writer, TR's exporter, must follow it).
    #[test]
    fn the_format_1_fixture_is_pinned() {
        let b = bytes(&Fixture::default());
        let sha: [u8; 32] = Sha256::digest(&b).into();
        let hex = sha.iter().fold(String::new(), |mut h, x| {
            use core::fmt::Write as _;
            let _ = write!(h, "{x:02x}");
            h
        });
        assert_eq!(
            (b.len(), hex.as_str()),
            (FIXTURE_LEN, FIXTURE_SHA256),
            "the format-1 fixture moved: that is a format change (format 2), not an edit"
        );
    }

    /// The judge's probes: a node count no file can hold (with a valid SHA-256) and dims whose product overflows
    /// are refused, not an abort or a panic.
    #[test]
    fn hostile_counts_and_dims_are_refused() {
        let mut b = bytes(&Fixture::default());
        b[136..140].copy_from_slice(&u32::MAX.to_le_bytes());
        let e = ModelPack::parse(&rehash(b)).unwrap_err();
        assert_eq!(e.code(), "model_malformed", "{e}");
        for at in [128, 132, 140, 164] {
            let mut b = bytes(&Fixture::default());
            b[at..at + 4].copy_from_slice(&(1u32 << 28).to_le_bytes());
            assert_eq!(
                ModelPack::parse(&rehash(b)).unwrap_err().code(),
                "model_malformed",
                "count at {at}"
            );
        }
        // The first tensor's dims (after the two inputs' entries) set to u32::MAX twice: the product overflows.
        let b = bytes(&Fixture::default());
        let first_tensor = 256 + (1 + 3 + 1 + 1 + 12) + (1 + 7 + 1 + 1 + 4);
        assert_eq!(&b[first_tensor + 1..first_tensor + 16], b"head.hyp.weight");
        let dims_at = first_tensor + 16 + 2;
        let mut d = b.clone();
        d[dims_at..dims_at + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        d[dims_at + 4..dims_at + 8].copy_from_slice(&u32::MAX.to_le_bytes());
        let e = ModelPack::parse(&rehash(d)).unwrap_err();
        assert_eq!(e.code(), "model_too_large", "{e}");
        assert_eq!(
            TensorEntry {
                name: "w".to_owned(),
                dtype: DType::F32,
                dims: vec![u32::MAX; 8],
                offset: 0,
            }
            .byte_len(),
            u64::MAX
        );
    }

    proptest::proptest! {
        /// Whatever the bytes, `parse` returns: a mutated fixture with its SHA-256 recomputed (so the checks after
        /// the hash run) is read or refused, never a panic.
        #[test]
        fn parse_never_panics_on_mutated_files(
            edits in proptest::collection::vec((0usize..4096, proptest::prelude::any::<u8>()), 0..12),
            fields in proptest::collection::vec(
                (
                    0usize..28,
                    proptest::prop_oneof![
                        proptest::strategy::Just(u32::MAX),
                        proptest::strategy::Just(0u32),
                        0u32..400,
                        proptest::prelude::any::<u32>(),
                    ],
                ),
                0..4,
            ),
            cut in proptest::option::of(0usize..9000),
        ) {
            let mut b = bytes(&Fixture::default());
            for (slot, v) in fields {
                let at = 112 + 4 * slot;
                b[at..at + 4].copy_from_slice(&v.to_le_bytes());
            }
            for (at, v) in edits {
                let i = at % b.len();
                b[i] = v;
            }
            if let Some(c) = cut {
                b.truncate(c.min(b.len()));
            }
            if b.len() >= HEADER_LEN {
                b = rehash(b);
            }
            let _ = ModelPack::parse(&b);
        }
    }

    #[test]
    fn the_info_table_is_information_only_but_well_formed() {
        // Unknown keys are kept and ignored; none is required.
        let m = parse(&Fixture {
            info: vec![("zz-unknown-key", "")],
            ..Fixture::default()
        })
        .unwrap();
        assert_eq!(m.info_value("zz-unknown-key"), Some(""));
        assert!(
            parse(&Fixture {
                info: Vec::new(),
                ds1: 0,
                ..Fixture::default()
            })
            .is_ok()
        );
        for info in [
            vec![("commit", "a"), ("commit", "b")],
            vec![("commit", "tab\there")],
        ] {
            assert_eq!(
                parse(&Fixture {
                    info,
                    ..Fixture::default()
                })
                .unwrap_err()
                .code(),
                "model_malformed"
            );
        }
    }

    #[test]
    fn attributes_no_writer_makes_are_refused() {
        let ok = [
            (21, vec![-1, 102]),
            (22, vec![1, 0]),
            (17, vec![1]),
            (11, vec![3, 1, 2, 2]),
            (24, vec![0, 0, 4, 1]),
        ];
        for (op, attrs) in ok {
            assert!(
                parse(&Fixture {
                    second_op: op,
                    second_attrs: Some(attrs.clone()),
                    ..Fixture::default()
                })
                .is_ok(),
                "op {op} {attrs:?}"
            );
        }
        let bad = [
            (21, vec![0, 102]),     // Reshape's 0 (ONNX would copy the dim)
            (21, vec![-1, -1]),     // two inferred dims
            (21, vec![1; 9]),       // past the rank
            (22, vec![0, 0]),       // not a permutation
            (22, vec![2, 0]),       // out of range
            (17, vec![-1]),         // a negative axis
            (11, vec![2, 1, 2, 0]), // a pad as large as the kernel
            (11, vec![2, 0, 0, 0]), // stride 0
            (24, vec![0, 3, 3, 1]), // an empty slice
            (24, vec![0, 0, 4, 0]), // step 0
        ];
        for (op, attrs) in bad {
            assert_eq!(
                parse(&Fixture {
                    second_op: op,
                    second_attrs: Some(attrs.clone()),
                    ..Fixture::default()
                })
                .unwrap_err()
                .code(),
                "model_malformed",
                "op {op} {attrs:?}"
            );
        }
    }

    #[test]
    fn truncated_files_are_refused() {
        let b = bytes(&Fixture::default());
        for cut in [0, 7, 100, 255] {
            assert_eq!(
                ModelPack::parse(&b[..cut]).unwrap_err(),
                PackError::NotAModel,
                "{cut}"
            );
        }
        for cut in [256, 300, b.len() - 1] {
            assert_eq!(
                ModelPack::parse(&b[..cut]).unwrap_err().code(),
                "model_truncated",
                "{cut}"
            );
        }
        let mut long = b.clone();
        long.push(0);
        assert_eq!(
            ModelPack::parse(&long).unwrap_err().code(),
            "model_malformed"
        );
    }

    #[test]
    fn a_bad_magic_is_not_a_model() {
        let mut b = bytes(&Fixture::default());
        b[0] = b'X';
        assert_eq!(ModelPack::parse(&b).unwrap_err().code(), "not_a_model");
        let bank = *b"ENTHBANK";
        b[..8].copy_from_slice(&bank);
        assert_eq!(ModelPack::parse(&b).unwrap_err().code(), "not_a_model");
    }

    #[test]
    fn a_newer_format_is_refused_before_anything_else() {
        // A newer format may lay out everything else differently: nothing after the format is read, the hash
        // included.
        let mut b = bytes(&Fixture {
            format: 2,
            hash: false,
            ..Fixture::default()
        });
        b.truncate(300);
        let e = ModelPack::parse(&b).unwrap_err();
        assert_eq!(
            e,
            PackError::TooNew {
                part: Part::Format,
                found: 2,
                supported: 1
            }
        );
        assert_eq!(e.code(), "model_too_new");
        assert!(e.hint().contains("update enth"));
        assert_eq!(
            parse(&Fixture {
                format: 0,
                ..Fixture::default()
            })
            .unwrap_err()
            .code(),
            "model_malformed"
        );
    }

    #[test]
    fn newer_parts_are_too_new() {
        let cases = [
            (
                Fixture {
                    op_set: 2,
                    ..Fixture::default()
                },
                Part::OpSet,
            ),
            (
                Fixture {
                    feature: 2,
                    ..Fixture::default()
                },
                Part::Feature,
            ),
            (
                Fixture {
                    layout: 2,
                    ..Fixture::default()
                },
                Part::Layout,
            ),
            (
                Fixture {
                    generator: 2,
                    ..Fixture::default()
                },
                Part::Generator,
            ),
            (
                Fixture {
                    template: 1,
                    ..Fixture::default()
                },
                Part::Template,
            ),
        ];
        for (f, part) in cases {
            match parse(&f).unwrap_err() {
                PackError::TooNew { part: p, .. } => assert_eq!(p, part),
                e => panic!("{part:?}: {e}"),
            }
        }
    }

    #[test]
    fn a_catalogue_mismatch_runs_without_the_model() {
        let mut other = OutputLayout::v1().sha256();
        other[0] ^= 1;
        let e = parse(&Fixture {
            layout_sha256: other,
            ..Fixture::default()
        })
        .unwrap_err();
        assert!(
            matches!(
                e,
                PackError::Mismatch {
                    what: Mismatch::LayoutHash,
                    ..
                }
            ),
            "{e}"
        );
        assert_eq!(e.code(), "model_mismatch");
        assert!(e.hint().contains("create runs without it"));
        let e = parse(&Fixture {
            width: 101,
            ..Fixture::default()
        })
        .unwrap_err();
        assert!(
            matches!(
                e,
                PackError::Mismatch {
                    what: Mismatch::Width,
                    ..
                }
            ),
            "{e}"
        );
        let e = parse(&Fixture {
            mel_dims: vec![3, 64, 64],
            ..Fixture::default()
        })
        .unwrap_err();
        assert!(
            matches!(
                e,
                PackError::Mismatch {
                    what: Mismatch::Inputs,
                    ..
                }
            ),
            "{e}"
        );
    }

    #[test]
    fn a_bad_hash_is_refused() {
        let mut b = bytes(&Fixture::default());
        let last = b.len() - 1;
        b[last] ^= 0x40;
        assert_eq!(ModelPack::parse(&b).unwrap_err(), PackError::Hash);
        let mut b = bytes(&Fixture::default());
        b[66] = b'9'; // the engine string: information only, but covered by the hash
        assert_eq!(ModelPack::parse(&b).unwrap_err(), PackError::Hash);
        assert_eq!(
            parse(&Fixture {
                hash: false,
                ..Fixture::default()
            })
            .unwrap_err(),
            PackError::Hash
        );
    }

    #[test]
    fn budgets_are_enforced() {
        // The header's FLOP count is information only: lane ONX recounts from the shapes and refuses on its count.
        let m = parse(&Fixture {
            flops: MAX_FLOPS + 1,
            ..Fixture::default()
        })
        .unwrap();
        assert_eq!(m.header().flops, MAX_FLOPS + 1);
        let e = parse(&Fixture {
            parameters: Some(MAX_PARAMETERS + 1),
            ..Fixture::default()
        })
        .unwrap_err();
        assert!(
            matches!(
                e,
                PackError::TooLarge {
                    what: Budget::Parameters,
                    ..
                }
            ),
            "{e}"
        );
        let e = parse(&Fixture {
            parameters: Some(7),
            ..Fixture::default()
        })
        .unwrap_err();
        assert_eq!(e.code(), "model_malformed");
        let big = vec![0u8; MAX_FILE_BYTES + 1];
        let mut big = big;
        big[..256].copy_from_slice(&bytes(&Fixture::default())[..256]);
        assert!(matches!(
            ModelPack::parse(&big).unwrap_err(),
            PackError::TooLarge {
                what: Budget::File,
                ..
            }
        ));
    }

    #[test]
    fn malformed_graphs_and_weights_are_refused() {
        let cases = [
            Fixture {
                first_op: 99,
                ..Fixture::default()
            }, // no such op
            Fixture {
                first_op: 6,
                ..Fixture::default()
            }, // Relu with two inputs
            Fixture {
                first_input: 4,
                ..Fixture::default()
            }, // reads its own output
            Fixture {
                weight: 0x7c00,
                ..Fixture::default()
            }, // an f16 infinity
            Fixture {
                weight: 0x7e00,
                ..Fixture::default()
            }, // an f16 NaN
            Fixture {
                reserved: 1,
                ..Fixture::default()
            },
            Fixture {
                model_id: "Propose",
                ..Fixture::default()
            },
            Fixture {
                k: 0,
                ..Fixture::default()
            },
            Fixture {
                w2_at: Some(8_160 + 4),
                ..Fixture::default()
            }, // a tensor off the 64-byte grid
            Fixture {
                conf0: 1e-40,
                ..Fixture::default()
            }, // a subnormal f32 weight
        ];
        for (i, f) in cases.iter().enumerate() {
            assert_eq!(parse(f).unwrap_err().code(), "model_malformed", "case {i}");
        }
        // Padding before the data section that is not zero.
        let b = bytes(&Fixture::default());
        let data_offset = u64_at(&b, 144) as usize;
        let mut p = b.clone();
        p[data_offset - 1] = 1;
        assert_eq!(
            ModelPack::parse(&rehash(p)).unwrap_err().code(),
            "model_malformed"
        );
    }

    #[test]
    fn f16_widens_exactly() {
        assert_eq!(f16_to_f32(0x0000), 0.0);
        assert!(f16_to_f32(0x8000).is_sign_negative() && f16_to_f32(0x8000) == 0.0);
        assert_eq!(f16_to_f32(0x3c00), 1.0);
        assert_eq!(f16_to_f32(0xc000), -2.0);
        assert_eq!(f16_to_f32(0x3555), 0.333_251_95);
        assert_eq!(f16_to_f32(0x7bff), 65_504.0);
        let pow2 = |e: i32| f32::from_bits(((127 + e) as u32) << 23);
        assert_eq!(f16_to_f32(0x0400), pow2(-14));
        assert_eq!(f16_to_f32(0x0001), pow2(-24));
        assert_eq!(f16_to_f32(0x03ff), 1023.0 * pow2(-24));
        assert_eq!(f16_to_f32(0x7c00), f32::INFINITY);
        assert!(f16_to_f32(0x7e00).is_nan());
        // Monotonic over every positive finite value, so no value is skipped or doubled.
        let mut last = -1.0f32;
        for h in 0u16..0x7c00 {
            let v = f16_to_f32(h);
            assert!(v > last, "{h:#06x}");
            last = v;
        }
    }

    #[test]
    fn the_op_table_is_in_code_order() {
        for (i, o) in OPS.iter().enumerate() {
            assert_eq!(o.op as usize, i + 1);
            assert_eq!(OpCode::from_code(i as u16 + 1), Some(o.op));
            assert!(o.since <= OP_SET);
        }
        assert_eq!(OpCode::from_code(0), None);
        assert_eq!(OpCode::from_code(OPS.len() as u16 + 1), None);
    }
}
