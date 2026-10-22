//! **Owner: lane ONX** (freeze point F3s). Pre-declared by lane S0s (R3).
//!
//! `Net`: a [`ModelPack`]'s graph, checked and planned once ([`Net::build`]), then run over [`ops`] on one thread
//! ([`Net::run`]); the encoder runs once per create job. Its outputs are the Kₘ hypothesis rows and the Kₘ
//! confidence logits (docs/r3.md, "The model contract (F3s)") that [`decode`](super::decode) turns into create
//! starts, plus any extra output the model declares.
//!
//! # Building (what the reader leaves to the net)
//!
//! [`ModelPack::parse`] checks the bytes, the counts, the products of dims, each node's attributes where no shape
//! is needed, and the feature set's inputs and the layout's outputs by name and declared shape. [`Net::build`] does
//! the rest, before anything is allocated from a shape:
//!
//! - **shape inference**, op by op, from `pack.rs`'s op table: each input's rank and dims, the channels against the
//!   groups, the kernel against the padded input, broadcasting, a `Reshape`'s element count, a permutation's rank, a
//!   slice's range, strides, pads and dilations at most [`MAX_GEOMETRY`]. A graph no exporter makes is
//!   `model_malformed`, naming the node;
//! - **the declared outputs** against the inferred shapes (`model_malformed` when they disagree);
//! - **the FLOP recount** from the shapes, as the op table counts them (`2 ·` the multiply-adds of `Conv1d`,
//!   `Conv2d`, `Linear` and `MatMul`, 1 per output element of every other op, 0 for `Reshape`, `Transpose`, `Concat`
//!   and `Slice`), refused over [`MAX_FLOPS`] (`model_too_large`); the header's figure is information only;
//! - **memory, bounded up front**: every value at most [`MAX_VALUE_ELEMENTS`], and the values alive at once (the
//!   inputs and node outputs, each freed after its last reader; graph outputs kept) at most [`MAX_LIVE_ELEMENTS`]
//!   along the node order (`model_too_large`, [`Budget::Memory`]). The weights are the reader's budget (at most
//!   [`MAX_PARAMETERS`](super::pack::MAX_PARAMETERS)), widened to `f32` once here, and a stored `Linear` weight is
//!   also kept transposed (so a run sums each output in the same order, with its outputs side by side).
//!
//! # Running
//!
//! [`Net::run`] takes a [`ModelInput`] (value 0 is `mel`, 1 is `context`; then the tensors; then each node's output
//! in node order), flushes it (no input is subnormal in feature set 1, so this changes nothing there), and runs the
//! nodes in order. It is pure: no I/O, no threads, no clocks; it allocates each node's output once, within the
//! bound the build computed. The same bits on every platform and under any FTZ/DAZ setting ([`ops`]' rules);
//! [`Outputs::sha256`] is what the output hashes pin.

use core::fmt;

use sha2::{Digest, Sha256};

use super::features::{CONTEXT, FRAMES, MEL_BANDS, ModelInput, RESOLUTIONS};
use super::ops::{self, Binary, Conv2d, Pool2d, Unary};
use super::pack::{
    Budget, CONFIDENCE_OUTPUT, HYPOTHESES_OUTPUT, MAX_FLOPS, MAX_RANK, ModelPack, OpCode, PackError,
};

/// The most elements of any one value (16 Mi `f32`s, 64 MiB): a node output above it is refused at build.
pub const MAX_VALUE_ELEMENTS: u64 = 1 << 24;

/// The most elements of the values alive at once along the node order (32 Mi `f32`s, 128 MiB), weights aside.
pub const MAX_LIVE_ELEMENTS: u64 = 1 << 25;

/// The largest stride, padding or dilation a convolution or pool may have (real models use single digits): it keeps
/// every index of a run inside 32 bits, so a hostile file cannot overflow one on wasm32.
pub const MAX_GEOMETRY: i32 = 1 << 16;

/// What a node computes, with the shapes it needs resolved at build.
#[derive(Clone, Debug, PartialEq)]
enum Step {
    Conv(Conv2d),
    /// `n` inputs and `m` outputs per row; `t` indexes [`Net::transposed`] when the weight is stored.
    Linear {
        n: usize,
        m: usize,
        t: Option<usize>,
    },
    MatMul,
    LayerNorm(f32),
    Unary(Unary),
    MaxPool(Pool2d),
    AvgPool(Pool2d),
    GlobalAvg(usize),
    GlobalMax(usize),
    Softmax {
        len: usize,
        inner: usize,
    },
    Binary(Binary),
    Concat(usize),
    Copy,
    Transpose(Vec<usize>),
    Slice {
        axis: usize,
        start: usize,
        step: usize,
        len: usize,
    },
}

#[derive(Clone, Debug, PartialEq)]
struct Planned {
    step: Step,
    inputs: Vec<u32>,
    output: u32,
    /// Values whose last reader this node is (inputs and node outputs; never tensors or graph outputs).
    free: Vec<u32>,
}

/// A model's graph, checked and planned (see the [module docs](self)).
#[derive(Clone, Debug, PartialEq)]
pub struct Net {
    inputs: usize,
    weights: Vec<Vec<f32>>,
    transposed: Vec<Vec<f32>>,
    shapes: Vec<Vec<usize>>,
    plan: Vec<Planned>,
    outputs: Vec<(String, u32, Vec<u32>)>,
    flops: u64,
    peak: u64,
}

/// Why a run cannot start (a model input that is not the feature set's shape; [`ModelInput::compute`] never makes
/// one).
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum RunError {
    /// An input of the wrong length.
    Input {
        /// The input's name.
        name: &'static str,
        /// The feature set's length.
        expected: usize,
        /// The given length.
        got: usize,
    },
}

impl RunError {
    /// A stable snake-case code.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            RunError::Input { .. } => "model_input",
        }
    }
}

impl fmt::Display for RunError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RunError::Input {
                name,
                expected,
                got,
            } => write!(
                f,
                "model input {name} has {got} values; feature set 1 has {expected}"
            ),
        }
    }
}

impl std::error::Error for RunError {}

/// One output of a run.
#[derive(Clone, Debug, PartialEq)]
pub struct Output {
    /// Its name in the model file.
    pub name: String,
    /// Its shape.
    pub dims: Vec<u32>,
    /// Its values, row-major.
    pub values: Vec<f32>,
}

/// The outputs of a run, in the model file's output order.
#[derive(Clone, Debug, PartialEq)]
pub struct Outputs(Vec<Output>);

impl Outputs {
    /// Every output, in the model file's order.
    #[must_use]
    pub fn all(&self) -> &[Output] {
        &self.0
    }

    /// The output with this name.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&Output> {
        self.0.iter().find(|o| o.name == name)
    }

    /// The `[K, H]` hypothesis rows, row-major (every parsed model has them).
    ///
    /// # Panics
    /// Never for a net built from a parsed pack.
    #[must_use]
    pub fn hypotheses(&self) -> &[f32] {
        &self
            .get(HYPOTHESES_OUTPUT)
            .expect("a parsed model has hypotheses")
            .values
    }

    /// The `[K]` confidence logits (every parsed model has them).
    ///
    /// # Panics
    /// Never for a net built from a parsed pack.
    #[must_use]
    pub fn confidence(&self) -> &[f32] {
        &self
            .get(CONFIDENCE_OUTPUT)
            .expect("a parsed model has confidences")
            .values
    }

    /// Hypothesis row `k` (`H` values).
    ///
    /// # Panics
    /// If `k` is not below K.
    #[must_use]
    pub fn hypothesis(&self, k: usize) -> &[f32] {
        let o = self
            .get(HYPOTHESES_OUTPUT)
            .expect("a parsed model has hypotheses");
        let h = o.dims[1] as usize;
        &o.values[k * h..(k + 1) * h]
    }

    /// SHA-256 of every output in order: its name's length (`u8`) and bytes, its rank (`u8`) and dims (`u32`,
    /// little-endian), then its values (`f32`, little-endian). The output hash a model's golden pins.
    #[must_use]
    pub fn sha256(&self) -> [u8; 32] {
        let mut h = Sha256::new();
        for o in &self.0 {
            h.update([o.name.len() as u8]);
            h.update(o.name.as_bytes());
            h.update([o.dims.len() as u8]);
            for d in &o.dims {
                h.update(d.to_le_bytes());
            }
            for v in &o.values {
                h.update(v.to_le_bytes());
            }
        }
        h.finalize().into()
    }
}

fn malformed(node: usize, op: OpCode, why: impl fmt::Display) -> PackError {
    PackError::Malformed(format!("node {node} ({}): {why}", op.info().name))
}

fn elements(dims: &[usize]) -> Option<u64> {
    dims.iter()
        .try_fold(1u64, |acc, d| acc.checked_mul(*d as u64))
}

/// A shape inference's result: the step, the output's dims, the FLOPs.
type Inferred = (Step, Vec<usize>, u64);

/// One node's shape inference (see the [module docs](self)); `shapes` are its inputs'.
fn infer(op: OpCode, attrs: &[i32], shapes: &[&[usize]]) -> Result<Inferred, String> {
    let a = |i: usize| attrs[i] as usize;
    let rank = |i: usize, want: usize| {
        if shapes[i].len() == want {
            Ok(())
        } else {
            Err(format!(
                "input {i} has rank {}, not {want}",
                shapes[i].len()
            ))
        }
    };
    let bias = |co: usize| -> Result<(), String> {
        match shapes.get(2) {
            Some(b) if *b != [co] => Err(format!("the bias is {b:?}, not [{co}]")),
            _ => Ok(()),
        }
    };
    let out_elems =
        |dims: &[usize]| elements(dims).ok_or_else(|| "the output's size overflows".to_owned());
    // A convolution's attributes are strides, pads and dilations, then the groups; a pool's, the kernel sizes, then
    // strides and pads.
    let geometry = match op {
        OpCode::Conv1d | OpCode::Conv2d => Some(&attrs[..attrs.len() - 1]),
        OpCode::MaxPool1d | OpCode::AvgPool1d => Some(&attrs[1..]),
        OpCode::MaxPool2d | OpCode::AvgPool2d => Some(&attrs[2..]),
        _ => None,
    };
    if geometry.is_some_and(|g| g.iter().any(|v| *v > MAX_GEOMETRY)) {
        return Err(format!("a stride, pad or dilation above {MAX_GEOMETRY}"));
    }
    match op {
        OpCode::Conv1d | OpCode::Conv2d => {
            let two = op == OpCode::Conv2d;
            rank(0, if two { 3 } else { 2 })?;
            rank(1, if two { 4 } else { 3 })?;
            let (x, w) = (shapes[0], shapes[1]);
            let geo = if two {
                Conv2d {
                    input: [x[0], x[1], x[2]],
                    out_channels: w[0],
                    kernel: [w[2], w[3]],
                    stride: [a(0), a(1)],
                    pad: [a(2), a(3), a(4), a(5)],
                    dilation: [a(6), a(7)],
                    groups: a(8),
                }
            } else {
                Conv2d {
                    input: [x[0], 1, x[1]],
                    out_channels: w[0],
                    kernel: [1, w[2]],
                    stride: [1, a(0)],
                    pad: [0, a(1), 0, a(2)],
                    dilation: [1, a(3)],
                    groups: a(4),
                }
            };
            let [co, ho, wo] = geo
                .output()
                .ok_or("the channels do not split into the groups, or the kernel does not fit the padded input")?;
            if w[1] != geo.weight()[1] {
                return Err(format!(
                    "the weight is {w:?}: {} input channels per group",
                    geo.weight()[1]
                ));
            }
            bias(co)?;
            let flops = geo.flops().ok_or("the FLOP count overflows")?;
            let dims = if two { vec![co, ho, wo] } else { vec![co, wo] };
            Ok((Step::Conv(geo), dims, flops))
        }
        OpCode::Linear => {
            rank(1, 2)?;
            let (x, w) = (shapes[0], shapes[1]);
            let (m, n) = (w[0], w[1]);
            if x.last() != Some(&n) {
                return Err(format!("x is {x:?} and the weight {w:?}"));
            }
            bias(m)?;
            let mut dims = x.to_vec();
            if let Some(last) = dims.last_mut() {
                *last = m;
            }
            let rows = out_elems(&x[..x.len() - 1])?;
            let flops = [2, rows, m as u64, n as u64]
                .iter()
                .try_fold(1u64, |acc, v| acc.checked_mul(*v))
                .ok_or("the FLOP count overflows")?;
            Ok((Step::Linear { n, m, t: None }, dims, flops))
        }
        OpCode::MatMul => {
            let (x, y) = (shapes[0], shapes[1]);
            if x.len() < 2 || y.len() < 2 {
                return Err("both inputs need rank 2 or more".to_owned());
            }
            let (p, q, r) = (x[x.len() - 2], x[x.len() - 1], y[y.len() - 1]);
            if y[y.len() - 2] != q {
                return Err(format!("{x:?} times {y:?}"));
            }
            let mut dims = ops::broadcast_shape(&x[..x.len() - 2], &y[..y.len() - 2])
                .ok_or_else(|| format!("the leading axes of {x:?} and {y:?} do not broadcast"))?;
            let lead = out_elems(&dims)?;
            dims.extend([p, r]);
            if dims.len() > MAX_RANK {
                return Err("the output's rank is above 8".to_owned());
            }
            let flops = [2, lead, p as u64, q as u64, r as u64]
                .iter()
                .try_fold(1u64, |acc, v| acc.checked_mul(*v))
                .ok_or("the FLOP count overflows")?;
            Ok((Step::MatMul, dims, flops))
        }
        OpCode::LayerNorm => {
            let x = shapes[0];
            let n = *x.last().unwrap_or(&0);
            if shapes[1] != [n] || shapes[2] != [n] {
                return Err(format!("x is {x:?}, γ {:?}, β {:?}", shapes[1], shapes[2]));
            }
            let eps = f32::from_bits(attrs[0] as u32);
            Ok((Step::LayerNorm(eps), x.to_vec(), out_elems(x)?))
        }
        OpCode::Relu | OpCode::Gelu | OpCode::Silu | OpCode::Tanh | OpCode::Sigmoid => {
            let u = match op {
                OpCode::Relu => Unary::Relu,
                OpCode::Gelu => Unary::Gelu,
                OpCode::Silu => Unary::Silu,
                OpCode::Tanh => Unary::Tanh,
                _ => Unary::Sigmoid,
            };
            Ok((Step::Unary(u), shapes[0].to_vec(), out_elems(shapes[0])?))
        }
        OpCode::MaxPool1d | OpCode::AvgPool1d | OpCode::MaxPool2d | OpCode::AvgPool2d => {
            let two = matches!(op, OpCode::MaxPool2d | OpCode::AvgPool2d);
            rank(0, if two { 3 } else { 2 })?;
            let x = shapes[0];
            let geo = if two {
                Pool2d {
                    input: [x[0], x[1], x[2]],
                    kernel: [a(0), a(1)],
                    stride: [a(2), a(3)],
                    pad: [a(4), a(5), a(6), a(7)],
                }
            } else {
                Pool2d {
                    input: [x[0], 1, x[1]],
                    kernel: [1, a(0)],
                    stride: [1, a(1)],
                    pad: [0, a(2), 0, a(3)],
                }
            };
            let [c, ho, wo] = geo
                .output()
                .ok_or("the kernel does not fit the padded input")?;
            let dims = if two { vec![c, ho, wo] } else { vec![c, wo] };
            let flops = out_elems(&dims)?;
            let step = if matches!(op, OpCode::MaxPool1d | OpCode::MaxPool2d) {
                Step::MaxPool(geo)
            } else {
                Step::AvgPool(geo)
            };
            Ok((step, dims, flops))
        }
        OpCode::GlobalAvgPool | OpCode::GlobalMaxPool => {
            let x = shapes[0];
            if x.len() < 2 {
                return Err("x needs rank 2 or more".to_owned());
            }
            let spatial = out_elems(&x[1..])? as usize;
            let mut dims = vec![1; x.len()];
            dims[0] = x[0];
            let step = if op == OpCode::GlobalAvgPool {
                Step::GlobalAvg(spatial)
            } else {
                Step::GlobalMax(spatial)
            };
            Ok((step, dims, x[0] as u64))
        }
        OpCode::Softmax => {
            let x = shapes[0];
            let axis = a(0);
            if axis >= x.len() {
                return Err(format!("axis {axis} of a rank-{} input", x.len()));
            }
            let inner = out_elems(&x[axis + 1..])? as usize;
            Ok((
                Step::Softmax {
                    len: x[axis],
                    inner,
                },
                x.to_vec(),
                out_elems(x)?,
            ))
        }
        OpCode::Add | OpCode::Mul | OpCode::Sub => {
            let dims = ops::broadcast_shape(shapes[0], shapes[1])
                .ok_or_else(|| format!("{:?} and {:?} do not broadcast", shapes[0], shapes[1]))?;
            let b = match op {
                OpCode::Add => Binary::Add,
                OpCode::Mul => Binary::Mul,
                _ => Binary::Sub,
            };
            let flops = out_elems(&dims)?;
            Ok((Step::Binary(b), dims, flops))
        }
        OpCode::Concat => {
            let axis = a(0);
            let first = shapes[0];
            if axis >= first.len() {
                return Err(format!("axis {axis} of a rank-{} input", first.len()));
            }
            let mut dims = first.to_vec();
            dims[axis] = 0;
            for s in shapes {
                let same = s.len() == first.len()
                    && s.iter()
                        .zip(first)
                        .enumerate()
                        .all(|(i, (x, y))| i == axis || x == y);
                if !same {
                    return Err(format!("{s:?} does not join {first:?} along axis {axis}"));
                }
                dims[axis] += s[axis];
            }
            Ok((Step::Concat(axis), dims, 0))
        }
        OpCode::Reshape => {
            let have = out_elems(shapes[0])?;
            let known = attrs
                .iter()
                .filter(|v| **v != -1)
                .try_fold(1u64, |acc, v| acc.checked_mul(*v as u64))
                .ok_or("the shape's size overflows")?;
            let mut dims: Vec<usize> = attrs.iter().map(|v| *v as usize).collect();
            if let Some(i) = attrs.iter().position(|v| *v == -1) {
                if known == 0 || !have.is_multiple_of(known) {
                    return Err(format!("{have} elements do not fill {attrs:?}"));
                }
                dims[i] = usize::try_from(have / known).map_err(|_| "too large")?;
            } else if known != have {
                return Err(format!("{have} elements reshaped to {attrs:?}"));
            }
            Ok((Step::Copy, dims, 0))
        }
        OpCode::Transpose => {
            let x = shapes[0];
            if attrs.len() != x.len() {
                return Err(format!(
                    "a permutation of {} axes on a rank-{} input",
                    attrs.len(),
                    x.len()
                ));
            }
            let perm: Vec<usize> = attrs.iter().map(|v| *v as usize).collect();
            let dims = perm.iter().map(|p| x[*p]).collect();
            Ok((Step::Transpose(perm), dims, 0))
        }
        OpCode::Slice => {
            let x = shapes[0];
            let (axis, start, step) = (a(0), a(1), a(3));
            if axis >= x.len() {
                return Err(format!("axis {axis} of a rank-{} input", x.len()));
            }
            let end = a(2).min(x[axis]);
            if start >= end {
                return Err(format!(
                    "the slice starts at {start}, past axis {axis}'s {}",
                    x[axis]
                ));
            }
            let len = (end - start).div_ceil(step);
            let mut dims = x.to_vec();
            dims[axis] = len;
            Ok((
                Step::Slice {
                    axis,
                    start,
                    step,
                    len,
                },
                dims,
                0,
            ))
        }
    }
}

impl Net {
    /// Plan `pack`'s graph: infer every shape, check the declared outputs, recount the FLOPs and bound the memory
    /// (see the [module docs](self)).
    ///
    /// # Errors
    /// `model_malformed` for a shape no exporter makes or a declared output that disagrees; `model_too_large` over
    /// [`MAX_FLOPS`], [`MAX_VALUE_ELEMENTS`] or [`MAX_LIVE_ELEMENTS`]. Either way, create runs without the model.
    pub fn build(pack: &ModelPack) -> Result<Net, PackError> {
        let inputs = pack.inputs().len();
        let tensors = pack.tensors().len();
        let to_usize = |d: &[u32]| d.iter().map(|v| *v as usize).collect::<Vec<_>>();
        let mut shapes: Vec<Vec<usize>> = pack
            .inputs()
            .iter()
            .map(|i| to_usize(&i.dims))
            .chain(pack.tensors().iter().map(|t| to_usize(&t.dims)))
            .collect();
        let is_tensor = |v: u32| (inputs..inputs + tensors).contains(&(v as usize));
        let mut plan = Vec::with_capacity(pack.nodes().len());
        let mut flops = 0u64;
        let mut transposed_of: Vec<usize> = Vec::new();
        for (i, node) in pack.nodes().iter().enumerate() {
            let ins: Vec<&[usize]> = node
                .inputs
                .iter()
                .map(|v| shapes[*v as usize].as_slice())
                .collect();
            let (mut step, dims, f) =
                infer(node.op, &node.attrs, &ins).map_err(|why| malformed(i, node.op, why))?;
            let size = elements(&dims).unwrap_or(u64::MAX);
            if size > MAX_VALUE_ELEMENTS {
                return Err(PackError::TooLarge {
                    what: Budget::Memory,
                    found: size,
                    limit: MAX_VALUE_ELEMENTS,
                });
            }
            flops = flops.saturating_add(f);
            if flops > MAX_FLOPS {
                return Err(PackError::TooLarge {
                    what: Budget::Flops,
                    found: flops,
                    limit: MAX_FLOPS,
                });
            }
            if let Step::Linear { t, .. } = &mut step
                && is_tensor(node.inputs[1])
            {
                let tensor = node.inputs[1] as usize - inputs;
                let at = transposed_of
                    .iter()
                    .position(|x| *x == tensor)
                    .unwrap_or_else(|| {
                        transposed_of.push(tensor);
                        transposed_of.len() - 1
                    });
                *t = Some(at);
            }
            debug_assert_eq!(node.first_output as usize, shapes.len());
            shapes.push(dims);
            plan.push(Planned {
                step,
                inputs: node.inputs.clone(),
                output: node.first_output,
                free: Vec::new(),
            });
        }
        let outputs: Vec<(String, u32, Vec<u32>)> = pack
            .outputs()
            .iter()
            .map(|o| (o.name.clone(), o.value, o.dims.clone()))
            .collect();
        for (name, value, dims) in &outputs {
            let inferred = &shapes[*value as usize];
            if to_usize(dims) != *inferred {
                return Err(PackError::Malformed(format!(
                    "output {name} declares {dims:?}; the graph gives {inferred:?}"
                )));
            }
        }
        // Liveness: each input and node output is freed after its last reader (graph outputs are kept).
        let mut last: Vec<Option<usize>> = vec![None; shapes.len()];
        for (i, p) in plan.iter().enumerate() {
            for v in &p.inputs {
                last[*v as usize] = Some(i);
            }
        }
        let kept =
            |v: usize| is_tensor(v as u32) || outputs.iter().any(|(_, o, _)| *o as usize == v);
        for (v, last) in last.iter().enumerate() {
            if kept(v) {
                continue;
            }
            // A value nobody reads: an input stays, a node output is freed right after its own node.
            let at = last.or_else(|| v.checked_sub(inputs + tensors));
            if let Some(at) = at {
                plan[at].free.push(v as u32);
            }
        }
        let size = |v: u32| elements(&shapes[v as usize]).unwrap_or(u64::MAX);
        let mut live: u64 = (0..inputs as u32).map(size).sum();
        let mut peak = live;
        for p in &plan {
            live += size(p.output);
            peak = peak.max(live);
            live -= p.free.iter().map(|v| size(*v)).sum::<u64>();
        }
        if peak > MAX_LIVE_ELEMENTS {
            return Err(PackError::TooLarge {
                what: Budget::Memory,
                found: peak,
                limit: MAX_LIVE_ELEMENTS,
            });
        }
        let weights: Vec<Vec<f32>> = (0..tensors).map(|t| pack.tensor_f32(t)).collect();
        let transposed = transposed_of
            .iter()
            .map(|t| {
                let dims = &shapes[inputs + t];
                let mut out = vec![0.0; weights[*t].len()];
                ops::transpose(&weights[*t], dims, &[1, 0], &mut out);
                out
            })
            .collect();
        Ok(Net {
            inputs,
            weights,
            transposed,
            shapes,
            plan,
            outputs,
            flops,
            peak,
        })
    }

    /// FLOPs per run, recounted from the shapes.
    #[must_use]
    pub fn flops(&self) -> u64 {
        self.flops
    }

    /// The most `f32` elements alive at once during a run (inputs and node outputs; weights aside).
    #[must_use]
    pub fn peak_elements(&self) -> u64 {
        self.peak
    }

    /// The shape of value `id` (inputs, then tensors, then node outputs).
    #[must_use]
    pub fn shape(&self, id: u32) -> Option<&[usize]> {
        self.shapes.get(id as usize).map(Vec::as_slice)
    }

    /// Run the graph on one input (see the [module docs](self)).
    ///
    /// # Errors
    /// [`RunError::Input`] when `mel` or `context` is not the feature set's length.
    pub fn run(&self, input: &ModelInput) -> Result<Outputs, RunError> {
        const MEL: usize = RESOLUTIONS * FRAMES * MEL_BANDS;
        if input.mel.len() != MEL {
            return Err(RunError::Input {
                name: "mel",
                expected: MEL,
                got: input.mel.len(),
            });
        }
        debug_assert_eq!(input.context.len(), CONTEXT);
        let tensors = self.weights.len();
        let mut values: Vec<Option<Vec<f32>>> = vec![None; self.shapes.len()];
        values[0] = Some(input.mel.iter().map(|v| ops::ftz(*v)).collect());
        values[1] = Some(input.context.iter().map(|v| ops::ftz(*v)).collect());
        for p in &self.plan {
            let n = elements(&self.shapes[p.output as usize]).unwrap_or(0) as usize;
            let mut out = vec![0.0f32; n];
            {
                let get = |v: u32| -> &[f32] {
                    let v = v as usize;
                    if (self.inputs..self.inputs + tensors).contains(&v) {
                        &self.weights[v - self.inputs]
                    } else {
                        values[v]
                            .as_deref()
                            .expect("a value is written before it is read")
                    }
                };
                let shape = |v: u32| self.shapes[v as usize].as_slice();
                let x = get(p.inputs[0]);
                let opt = |i: usize| p.inputs.get(i).map(|v| get(*v));
                match &p.step {
                    Step::Conv(geo) => geo.run(x, get(p.inputs[1]), opt(2), &mut out),
                    Step::Linear { n, m, t } => match t {
                        Some(t) => ops::linear_t(x, *n, &self.transposed[*t], *m, opt(2), &mut out),
                        None => ops::linear(x, *n, get(p.inputs[1]), *m, opt(2), &mut out),
                    },
                    Step::MatMul => ops::batched_matmul(
                        x,
                        shape(p.inputs[0]),
                        get(p.inputs[1]),
                        shape(p.inputs[1]),
                        shape(p.output),
                        &mut out,
                    ),
                    Step::LayerNorm(eps) => {
                        ops::layer_norm(x, get(p.inputs[1]), get(p.inputs[2]), *eps, &mut out);
                    }
                    Step::Unary(u) => u.run(x, &mut out),
                    Step::MaxPool(geo) => geo.max(x, &mut out),
                    Step::AvgPool(geo) => geo.avg(x, &mut out),
                    Step::GlobalAvg(spatial) => ops::global_avg_pool(x, *spatial, &mut out),
                    Step::GlobalMax(spatial) => ops::global_max_pool(x, *spatial, &mut out),
                    Step::Softmax { len, inner } => ops::softmax(x, *len, *inner, &mut out),
                    Step::Binary(b) => ops::binary(
                        *b,
                        x,
                        shape(p.inputs[0]),
                        get(p.inputs[1]),
                        shape(p.inputs[1]),
                        shape(p.output),
                        &mut out,
                    ),
                    Step::Concat(axis) => {
                        let parts: Vec<(&[f32], &[usize])> =
                            p.inputs.iter().map(|v| (get(*v), shape(*v))).collect();
                        ops::concat(&parts, *axis, &mut out);
                    }
                    Step::Copy => out.copy_from_slice(x),
                    Step::Transpose(perm) => ops::transpose(x, shape(p.inputs[0]), perm, &mut out),
                    Step::Slice {
                        axis,
                        start,
                        step,
                        len,
                    } => ops::slice(x, shape(p.inputs[0]), *axis, *start, *step, *len, &mut out),
                }
            }
            values[p.output as usize] = Some(out);
            for v in &p.free {
                values[*v as usize] = None;
            }
        }
        let value = |v: u32| -> Vec<f32> {
            let v = v as usize;
            if (self.inputs..self.inputs + tensors).contains(&v) {
                self.weights[v - self.inputs].clone()
            } else {
                values[v].clone().unwrap_or_default()
            }
        };
        Ok(Outputs(
            self.outputs
                .iter()
                .map(|(name, v, dims)| Output {
                    name: name.clone(),
                    dims: dims.clone(),
                    values: value(*v),
                })
                .collect(),
        ))
    }
}

/// The next value of a `SplitMix64` stream.
fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// A deterministic input for fixtures and output hashes (not a sound): every `mel` value, then every `context`
/// value, from one `SplitMix64` stream started at `seed`, each `((z >> 40) − 2²³) / 2²²`, a multiple of 2⁻²² in
/// [−2, 2) (exact in `f32`, never subnormal). `ml/enth_ml/export/synthetic.py` computes the same values, so a parity
/// fixture needs no input file.
#[must_use]
pub fn synthetic_input(seed: u64) -> ModelInput {
    let mut state = seed;
    let mut next = || ((splitmix64(&mut state) >> 40) as i32 - (1 << 23)) as f32 / (1 << 22) as f32;
    let mel = (0..RESOLUTIONS * FRAMES * MEL_BANDS)
        .map(|_| next())
        .collect();
    let mut context = [0.0; CONTEXT];
    for c in &mut context {
        *c = next();
    }
    ModelInput { mel, context }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::propose::pack::{ModelBuilder, Val};

    /// Deterministic weights: multiples of 2⁻²² in [−scale, scale).
    fn values(seed: u64, n: usize, scale: f32) -> Vec<f32> {
        let mut s = seed;
        (0..n)
            .map(|_| {
                ((splitmix64(&mut s) >> 40) as i32 - (1 << 23)) as f32 / (1 << 23) as f32 * scale
            })
            .collect()
    }

    /// The two required heads on `context` (K = 1).
    fn heads(b: &mut ModelBuilder) {
        let w = b.tensor("hyp.w", &[102, 5], &values(1, 510, 0.5));
        let h = b.node(OpCode::Linear, &[Val::CONTEXT, w], &[]);
        let r = b.node(OpCode::Reshape, &[h], &[1, 102]);
        b.output("hypotheses", r, &[1, 102]);
        let w = b.tensor("conf.w", &[1, 5], &values(2, 5, 0.5));
        let c = b.node(OpCode::Linear, &[Val::CONTEXT, w], &[]);
        b.output("confidence", c, &[1]);
    }

    fn build(f: impl FnOnce(&mut ModelBuilder)) -> Result<Net, PackError> {
        let mut b = ModelBuilder::new("onx-test", 1);
        heads(&mut b);
        f(&mut b);
        let pack = ModelPack::parse(&b.build()).expect("the builder writes a valid file");
        Net::build(&pack)
    }

    fn refused(f: impl FnOnce(&mut ModelBuilder)) -> String {
        match build(f) {
            Err(e) => format!("{} {e}", e.code()),
            Ok(_) => "accepted".to_owned(),
        }
    }

    #[test]
    fn the_heads_alone_compute_x_times_w_transposed() {
        let net = build(|_| {}).unwrap();
        let input = synthetic_input(3);
        let out = net.run(&input).unwrap();
        let w = values(1, 510, 0.5);
        let mut want = vec![0.0; 102];
        ops::linear(&input.context, 5, &w, 102, None, &mut want);
        assert_eq!(out.hypotheses(), want.as_slice());
        assert_eq!(out.hypothesis(0), want.as_slice());
        assert_eq!(out.confidence().len(), 1);
        assert_eq!(net.flops(), 2 * 102 * 5 + 2 * 5);
        assert_eq!(out, net.run(&input).unwrap());
    }

    #[test]
    fn a_short_input_is_refused() {
        let net = build(|_| {}).unwrap();
        let mut input = synthetic_input(0);
        input.mel.pop();
        let e = net.run(&input).unwrap_err();
        assert_eq!(e.code(), "model_input");
        assert!(e.to_string().contains("mel"));
    }

    #[test]
    fn convolution_flops_are_recounted_from_the_shapes() {
        let net = build(|b| {
            let w = b.tensor("conv.w", &[4, 3, 3, 3], &values(3, 108, 0.3));
            let c = b.node(OpCode::Conv2d, &[Val::MEL, w], &[1, 1, 1, 1, 1, 1, 1, 1, 1]);
            b.output("probe", c, &[4, 128, 64]);
        })
        .unwrap();
        assert_eq!(net.flops(), 2 * 4 * 128 * 64 * 3 * 9 + 2 * 102 * 5 + 2 * 5);
        assert_eq!(
            net.shape(net.plan.last().unwrap().output),
            Some(&[4usize, 128, 64][..])
        );
    }

    #[test]
    fn shapes_no_exporter_makes_are_refused() {
        type Graph = Box<dyn FnOnce(&mut ModelBuilder)>;
        let cases: Vec<(&str, Graph)> = vec![
            (
                "conv weight channels",
                Box::new(|b: &mut ModelBuilder| {
                    let w = b.tensor("w", &[4, 2, 3, 3], &values(3, 72, 0.3));
                    b.node(OpCode::Conv2d, &[Val::MEL, w], &[1, 1, 0, 0, 0, 0, 1, 1, 1]);
                }),
            ),
            (
                "conv groups",
                Box::new(|b: &mut ModelBuilder| {
                    let w = b.tensor("w", &[4, 1, 3, 3], &values(3, 36, 0.3));
                    b.node(OpCode::Conv2d, &[Val::MEL, w], &[1, 1, 0, 0, 0, 0, 1, 1, 2]);
                }),
            ),
            (
                "conv kernel past the input",
                Box::new(|b: &mut ModelBuilder| {
                    let w = b.tensor("w", &[1, 3, 3, 3], &values(3, 27, 0.3));
                    b.node(
                        OpCode::Conv2d,
                        &[Val::MEL, w],
                        &[1, 1, 0, 0, 0, 0, 100, 1, 1],
                    );
                }),
            ),
            (
                "conv bias",
                Box::new(|b: &mut ModelBuilder| {
                    let w = b.tensor("w", &[2, 3, 1, 1], &values(3, 6, 0.3));
                    let bias = b.tensor("b", &[3], &values(4, 3, 0.3));
                    b.node(
                        OpCode::Conv2d,
                        &[Val::MEL, w, bias],
                        &[1, 1, 0, 0, 0, 0, 1, 1, 1],
                    );
                }),
            ),
            (
                "conv1d rank",
                Box::new(|b: &mut ModelBuilder| {
                    let w = b.tensor("w", &[2, 3, 3], &values(3, 18, 0.3));
                    b.node(OpCode::Conv1d, &[Val::MEL, w], &[1, 0, 0, 1, 1]);
                }),
            ),
            (
                "linear width",
                Box::new(|b: &mut ModelBuilder| {
                    let w = b.tensor("w", &[2, 6], &values(3, 12, 0.3));
                    b.node(OpCode::Linear, &[Val::CONTEXT, w], &[]);
                }),
            ),
            (
                "matmul inner",
                Box::new(|b: &mut ModelBuilder| {
                    let w = b.tensor("w", &[3, 2], &values(3, 6, 0.3));
                    b.node(OpCode::MatMul, &[Val::MEL, w], &[]);
                }),
            ),
            (
                "matmul rank 1",
                Box::new(|b: &mut ModelBuilder| {
                    let w = b.tensor("w", &[5, 2], &values(3, 10, 0.3));
                    b.node(OpCode::MatMul, &[Val::CONTEXT, w], &[]);
                }),
            ),
            (
                "layernorm gamma",
                Box::new(|b: &mut ModelBuilder| {
                    let g = b.tensor("g", &[4], &[1.0; 4]);
                    b.node(
                        OpCode::LayerNorm,
                        &[Val::CONTEXT, g, g],
                        &[1e-5f32.to_bits() as i32],
                    );
                }),
            ),
            (
                "pool past the input",
                Box::new(|b: &mut ModelBuilder| {
                    b.node(OpCode::MaxPool1d, &[Val::CONTEXT], &[3, 1, 0, 0]);
                }),
            ),
            (
                "global pool rank 1",
                Box::new(|b: &mut ModelBuilder| {
                    b.node(OpCode::GlobalAvgPool, &[Val::CONTEXT], &[]);
                }),
            ),
            (
                "softmax axis",
                Box::new(|b: &mut ModelBuilder| {
                    b.node(OpCode::Softmax, &[Val::CONTEXT], &[1]);
                }),
            ),
            (
                "add broadcast",
                Box::new(|b: &mut ModelBuilder| {
                    b.node(OpCode::Add, &[Val::MEL, Val::CONTEXT], &[]);
                }),
            ),
            (
                "concat dims",
                Box::new(|b: &mut ModelBuilder| {
                    b.node(OpCode::Concat, &[Val::MEL, Val::CONTEXT], &[0]);
                }),
            ),
            (
                "concat axis",
                Box::new(|b: &mut ModelBuilder| {
                    b.node(OpCode::Concat, &[Val::CONTEXT, Val::CONTEXT], &[1]);
                }),
            ),
            (
                "reshape count",
                Box::new(|b: &mut ModelBuilder| {
                    b.node(OpCode::Reshape, &[Val::CONTEXT], &[2, 3]);
                }),
            ),
            (
                "reshape inferred",
                Box::new(|b: &mut ModelBuilder| {
                    b.node(OpCode::Reshape, &[Val::CONTEXT], &[2, -1]);
                }),
            ),
            (
                "transpose rank",
                Box::new(|b: &mut ModelBuilder| {
                    b.node(OpCode::Transpose, &[Val::MEL], &[1, 0]);
                }),
            ),
            (
                "slice start",
                Box::new(|b: &mut ModelBuilder| {
                    b.node(OpCode::Slice, &[Val::CONTEXT], &[0, 5, 9, 1]);
                }),
            ),
        ];
        for (name, f) in cases {
            let got = refused(f);
            assert!(got.starts_with("model_malformed"), "{name}: {got}");
        }
        // A declared output that is not what the graph gives.
        let got = refused(|b| {
            let s = b.node(OpCode::Slice, &[Val::CONTEXT], &[0, 1, 4, 2]);
            b.output("probe", s, &[3]);
        });
        assert!(
            got.starts_with("model_malformed") && got.contains("probe"),
            "{got}"
        );
        // The same, declared right: elements 1 and 3.
        assert!(
            build(|b| {
                let s = b.node(OpCode::Slice, &[Val::CONTEXT], &[0, 1, 4, 2]);
                b.output("probe", s, &[2]);
            })
            .is_ok()
        );
    }

    #[test]
    fn outputs_may_name_inputs_and_tensors_directly() {
        let net = build(|b| {
            let t = b.tensor("t", &[3], &[1.0, -2.0, 0.5]);
            b.output("t", t, &[3]).output("ctx", Val::CONTEXT, &[5]);
        })
        .unwrap();
        let input = synthetic_input(9);
        let out = net.run(&input).unwrap();
        assert_eq!(out.get("t").unwrap().values, [1.0, -2.0, 0.5]);
        assert_eq!(out.get("ctx").unwrap().values, input.context);
    }

    #[test]
    fn hostile_geometry_is_refused() {
        let huge = MAX_GEOMETRY + 1;
        for attrs in [
            [huge, 1, 0, 0, 0, 0, 1, 1, 1],
            [1, 1, 0, huge, 0, 0, 1, 1, 1],
            [1, 1, 0, 0, 0, 0, 1, huge, 1],
        ] {
            let got = refused(|b| {
                let w = b.tensor("w", &[1, 3, 1, 1], &[0.5; 3]);
                b.node(OpCode::Conv2d, &[Val::MEL, w], &attrs);
            });
            assert!(
                got.starts_with("model_malformed") && got.contains("stride, pad or dilation"),
                "{got}"
            );
        }
        let got = refused(|b| {
            b.node(OpCode::MaxPool1d, &[Val::CONTEXT], &[1, huge, 0, 0]);
        });
        assert!(got.contains("stride, pad or dilation"), "{got}");
        // Groups and kernel sizes are not geometry: a 1×1 conv with groups 3 on 3 channels is fine.
        assert!(
            build(|b| {
                let w = b.tensor("w", &[3, 1, 1, 1], &[0.5; 3]);
                b.node(OpCode::Conv2d, &[Val::MEL, w], &[1, 1, 0, 0, 0, 0, 1, 1, 3]);
            })
            .is_ok()
        );
    }

    #[test]
    fn a_graph_over_the_flop_budget_is_refused_before_it_runs() {
        // Four 64-channel 3×3 convolutions over [64, 128, 64]: about 0.6 GFLOP each.
        let got = refused(|b| {
            let w = b.tensor("stem", &[64, 3, 3, 3], &values(5, 64 * 27, 0.1));
            let mut x = b.node(OpCode::Conv2d, &[Val::MEL, w], &[1, 1, 1, 1, 1, 1, 1, 1, 1]);
            for i in 0..4 {
                let w = b.tensor(
                    &format!("c{i}"),
                    &[64, 64, 3, 3],
                    &values(6 + i, 64 * 64 * 9, 0.05),
                );
                x = b.node(OpCode::Conv2d, &[x, w], &[1, 1, 1, 1, 1, 1, 1, 1, 1]);
            }
        });
        assert!(
            got.starts_with("model_too_large") && got.contains("FLOPs"),
            "{got}"
        );
    }

    #[test]
    fn memory_is_bounded_up_front() {
        // One value over MAX_VALUE_ELEMENTS, with no FLOPs at all (concatenation counts 0).
        let got = refused(|b| {
            let c1 = b.node(OpCode::Concat, &[Val::MEL; 32], &[0]); // [96, 128, 64]
            b.node(OpCode::Concat, &[c1; 32], &[0]); // 25 M elements
        });
        assert!(
            got.starts_with("model_too_large") && got.contains("in memory"),
            "{got}"
        );
        // Three values of 12.6 M alive at once: over MAX_LIVE_ELEMENTS, though each is under the per-value bound.
        let got = refused(|b| {
            let c1 = b.node(OpCode::Concat, &[Val::MEL; 32], &[0]);
            let a = b.node(OpCode::Concat, &[c1; 16], &[0]);
            let c = b.node(OpCode::Concat, &[c1; 16], &[0]);
            b.node(OpCode::Add, &[a, c], &[]);
        });
        assert!(got.starts_with("model_too_large"), "{got}");
        // The same with the first value freed before the second is made fits.
        let net = build(|b| {
            let c1 = b.node(OpCode::Concat, &[Val::MEL; 32], &[0]);
            let a = b.node(OpCode::Concat, &[c1; 16], &[0]);
            let s = b.node(OpCode::GlobalMaxPool, &[a], &[]);
            b.output("probe", s, &[1536, 1, 1]);
        })
        .unwrap();
        // At `a`: the two kept heads (102 + 1), c1 and a (mel is freed after c1, context after the heads).
        let peak = 102 + 1 + 786_432 + 12_582_912;
        assert_eq!(net.peak_elements(), peak);
    }

    /// A model with one node of most kinds, for the mutation property below.
    fn mixed_model() -> Vec<u8> {
        let mut b = ModelBuilder::new("onx-mixed", 1);
        heads(&mut b);
        let s = b.node(OpCode::Slice, &[Val::MEL], &[1, 0, 16, 2]); // [3, 8, 64]
        let w = b.tensor("w", &[4, 3, 3, 3], &values(7, 108, 0.25));
        let c = b.node(OpCode::Conv2d, &[s, w], &[1, 2, 1, 1, 1, 1, 1, 1, 1]);
        let p = b.node(OpCode::AvgPool2d, &[c], &[2, 2, 2, 2, 1, 1, 0, 0]);
        let r = b.node(OpCode::Reshape, &[p], &[4, -1]);
        let t = b.node(OpCode::Transpose, &[r], &[1, 0]);
        let m = b.node(OpCode::MatMul, &[r, t], &[]);
        let sm = b.node(OpCode::Softmax, &[m], &[1]);
        let cc = b.node(OpCode::Concat, &[sm, m], &[0]);
        let g = b.node(OpCode::GlobalMaxPool, &[cc], &[]);
        b.output("probe", g, &[8, 1]);
        b.build()
    }

    proptest::proptest! {
        /// Whatever a hostile file's graph says, once the reader accepts it the net refuses it or runs it: never a
        /// panic in `Net::build` or `Net::run` (attribute, dim and value-id bytes mutated, the SHA-256 recomputed).
        #[test]
        fn build_and_run_never_panic_on_mutated_graphs(
            edits in proptest::collection::vec((0usize..2048, proptest::prelude::any::<u8>()), 1..6),
        ) {
            let mut bytes = mixed_model();
            let tables = 256..usize::try_from(u64::from_le_bytes(bytes[144..152].try_into().unwrap())).unwrap();
            for (at, v) in edits {
                let i = tables.start + at % tables.len();
                bytes[i] = v;
            }
            let sha = crate::propose::pack::file_sha256(&bytes);
            bytes[224..256].copy_from_slice(&sha);
            if let Ok(pack) = ModelPack::parse(&bytes)
                && let Ok(net) = Net::build(&pack)
            {
                let _ = net.run(&synthetic_input(1));
            }
        }
    }

    #[test]
    fn the_mixed_model_runs() {
        let pack = ModelPack::parse(&mixed_model()).unwrap();
        let net = Net::build(&pack).unwrap();
        assert_eq!(
            net.run(&synthetic_input(1))
                .unwrap()
                .get("probe")
                .unwrap()
                .values
                .len(),
            8
        );
    }

    #[test]
    fn synthetic_inputs_are_exact_and_pinned() {
        let a = synthetic_input(0);
        assert_eq!(a, synthetic_input(0));
        assert_ne!(a, synthetic_input(1));
        let all = a.mel.iter().chain(&a.context);
        for v in all {
            assert!((-2.0..2.0).contains(v) && !v.is_subnormal());
            assert_eq!((v * (1 << 22) as f32).fract(), 0.0);
        }
        // The first values of seed 0 (ml/enth_ml/export/synthetic.py pins the same).
        assert_eq!(
            &a.mel[..4],
            &[1.533_243_2, -0.273_888_1, -1.894_264_9, 1.883_527_8]
        );
        assert_eq!(a.context[4], 0.750_684_74);
    }
}
