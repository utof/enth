//! **Owner: lane ONX** (freeze point F3s). Pre-declared by lane S0s (R3).
//!
//! Our scalar kernels (docs/r3.md, "Our own scalar kernels"): every op of op set 1, exactly as
//! [`pack`](super::pack)'s op table defines it, **without the batch axis** (a feature map is `[C, L]` or
//! `[C, H, W]`), in `f32`. [`net`](super::net) runs them over a model's graph; this file holds the arithmetic and
//! nothing else (shapes are checked by the net before a kernel runs; a kernel indexes slices, so a wrong shape is a
//! panic, never a wrong read).
//!
//! # The determinism rules
//!
//! The same bits on every platform (x86-64, aarch64, wasm32), at every optimisation level and under any FTZ/DAZ
//! setting (docs/r3.md, the determinism contract, point 1):
//!
//! - **plain `+` and `×`, in a fixed order per output** (below), never `mul_add` (clippy bans it; Rust never
//!   contracts `a * b + c` on its own) and no reassociation. A loop may compute several outputs at once (the
//!   compiler vectorises across outputs), but each output's own sum runs in the order written here;
//! - **transcendentals through the pinned `libm`**: `expf` (sigmoid, `SiLU`, softmax), `erff` (GELU) and `tanhf`;
//!   `sqrt` and `÷` are correctly rounded in IEEE 754 and stay as they are;
//! - **every result is flushed in code** ([`ftz`]): a value at or below `f32::MIN_POSITIVE` in magnitude (a
//!   subnormal, the smallest normal, or a zero of either sign) becomes `+0.0`, after every product, every partial
//!   sum and every transcendental. So no operation ever reads a subnormal (DAZ has nothing to change) and none
//!   keeps one (FTZ has nothing to change), whichever way the hardware detects tininess (x86 after rounding, Arm
//!   before: flushing the smallest normal too covers both). Weights are never subnormal (the reader refuses them)
//!   and inputs never are (feature set 1; [`Net::run`](super::net::Net::run) flushes its inputs anyway);
//! - **comparisons are written out** (`>` with the first element winning), never `f32::max`, whose result on
//!   `max(-0.0, 0.0)` is not specified. NaN is a bug: it propagates through the arithmetic (a flush keeps it).
//!
//! # Summation orders
//!
//! | Op | Each output is |
//! | --- | --- |
//! | `Conv1d`, `Conv2d` | 0, then `+ w · x` over the group's input channels ascending, then kernel rows, then kernel columns (padding skipped: it adds nothing), then `+ b` |
//! | `Linear` | 0, then `+ x[i] · w[j, i]` for `i` ascending, then `+ b[j]` |
//! | `MatMul` | 0, then `+ a[p, q] · b[q, r]` for `q` ascending |
//! | `LayerNorm` | `μ = (Σ x, ascending) / n`; `d = x − μ`; `σ² = (Σ d · d, ascending) / n`; `y = (d / √(σ² + ε)) · γ + β` |
//! | `AvgPool`, `GlobalAvgPool` | the window's in-bounds elements summed row-major, then `÷` their count |
//! | `MaxPool`, `GlobalMaxPool` | the first in-bounds element, replaced by each later one that is `>` it, row-major |
//! | `Softmax` | `m` = the maximum as above; `e = exp(x − m)`; `s = Σ e` ascending; `y = e / s` |
//! | `Gelu` | `0.5 · x · (1 + erf(x · (1/√2)))`, evaluated as `(0.5 · x) · (1 + erf(x · FRAC_1_SQRT_2))` |
//! | `Silu`, `Sigmoid` | `x · s` and `s`, with `s = 1 / (1 + exp(−x))` |
//!
//! A later build may change an order (a blocked kernel that is 2–3× faster): that moves a model's output hashes,
//! never a replay (results record their starts as patches).

/// `v`, or `+0.0` when `|v|` is at or below `f32::MIN_POSITIVE` (see the [module docs](self)). NaN is kept.
#[inline(always)]
#[must_use]
pub fn ftz(v: f32) -> f32 {
    if v.abs() <= f32::MIN_POSITIVE { 0.0 } else { v }
}

/// `max(x, 0)`; `-0.0` gives `+0.0`, NaN stays NaN.
#[inline(always)]
#[must_use]
pub fn relu(x: f32) -> f32 {
    if x <= 0.0 { 0.0 } else { x }
}

/// `1 / (1 + exp(−x))`, flushed.
#[inline(always)]
#[must_use]
pub fn sigmoid(x: f32) -> f32 {
    ftz(1.0 / ftz(1.0 + ftz(libm::expf(-x))))
}

/// `x · sigmoid(x)`, flushed.
#[inline(always)]
#[must_use]
pub fn silu(x: f32) -> f32 {
    ftz(x * sigmoid(x))
}

/// `tanh(x)` through `libm`, flushed.
#[inline(always)]
#[must_use]
pub fn tanh(x: f32) -> f32 {
    ftz(libm::tanhf(x))
}

/// The erf form of GELU: `(0.5 · x) · (1 + erf(x · FRAC_1_SQRT_2))`, flushed at every step.
#[inline(always)]
#[must_use]
pub fn gelu(x: f32) -> f32 {
    let e = ftz(libm::erff(ftz(x * core::f32::consts::FRAC_1_SQRT_2)));
    ftz(ftz(0.5 * x) * ftz(1.0 + e))
}

/// An element-wise op of op set 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unary {
    /// [`relu`].
    Relu,
    /// [`gelu`].
    Gelu,
    /// [`silu`].
    Silu,
    /// [`tanh`].
    Tanh,
    /// [`sigmoid`].
    Sigmoid,
}

impl Unary {
    /// The op on one value.
    #[inline(always)]
    #[must_use]
    pub fn apply(self, x: f32) -> f32 {
        match self {
            Unary::Relu => relu(x),
            Unary::Gelu => gelu(x),
            Unary::Silu => silu(x),
            Unary::Tanh => tanh(x),
            Unary::Sigmoid => sigmoid(x),
        }
    }

    /// The op over `x` into `out` (the same length).
    pub fn run(self, x: &[f32], out: &mut [f32]) {
        assert_eq!(x.len(), out.len());
        match self {
            Unary::Relu => map(x, out, relu),
            Unary::Gelu => map(x, out, gelu),
            Unary::Silu => map(x, out, silu),
            Unary::Tanh => map(x, out, tanh),
            Unary::Sigmoid => map(x, out, sigmoid),
        }
    }
}

fn map(x: &[f32], out: &mut [f32], f: impl Fn(f32) -> f32) {
    for (o, v) in out.iter_mut().zip(x) {
        *o = f(*v);
    }
}

/// An element-wise op of two values, broadcasting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Binary {
    /// `a + b`.
    Add,
    /// `a × b`.
    Mul,
    /// `a − b`.
    Sub,
}

impl Binary {
    /// The op on one pair, flushed.
    #[inline(always)]
    #[must_use]
    pub fn apply(self, a: f32, b: f32) -> f32 {
        ftz(match self {
            Binary::Add => a + b,
            Binary::Mul => a * b,
            Binary::Sub => a - b,
        })
    }
}

/// The output length along one axis of a convolution: `⌊(len + pad_begin + pad_end − dilation · (k − 1) − 1) /
/// stride⌋ + 1`, or `None` when the dilated kernel does not fit in the padded input (or a factor is 0).
#[must_use]
pub fn conv_len(
    len: u64,
    k: u64,
    stride: u64,
    pad_begin: u64,
    pad_end: u64,
    dilation: u64,
) -> Option<u64> {
    if stride == 0 || k == 0 || dilation == 0 {
        return None;
    }
    let padded = len.checked_add(pad_begin)?.checked_add(pad_end)?;
    let span = dilation.checked_mul(k - 1)?.checked_add(1)?;
    Some(padded.checked_sub(span)? / stride + 1)
}

/// A 2-D convolution's geometry (ONNX `Conv` on `[Ci, H, W]`, zero padding, floor mode). A 1-D convolution is the
/// same with `H = 1`, a kernel height of 1, no vertical padding and unit vertical stride and dilation, which gives
/// each output the same summation order as a 1-D loop would.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Conv2d {
    /// Input channels, height, width.
    pub input: [usize; 3],
    /// Output channels.
    pub out_channels: usize,
    /// Kernel height and width.
    pub kernel: [usize; 2],
    /// Stride: vertical, horizontal.
    pub stride: [usize; 2],
    /// Padding: top, left, bottom, right.
    pub pad: [usize; 4],
    /// Dilation: vertical, horizontal.
    pub dilation: [usize; 2],
    /// Groups: input and output channels split evenly among them.
    pub groups: usize,
}

impl Conv2d {
    /// The output's `[Co, Ho, Wo]`, or `None` when the kernel does not fit or the channels do not split into the
    /// groups.
    #[must_use]
    pub fn output(&self) -> Option<[usize; 3]> {
        let [ci, h, w] = self.input;
        let g = self.groups;
        if g == 0
            || !ci.is_multiple_of(g)
            || !self.out_channels.is_multiple_of(g)
            || self.out_channels == 0
        {
            return None;
        }
        let u = |v: usize| v as u64;
        let ho = conv_len(
            u(h),
            u(self.kernel[0]),
            u(self.stride[0]),
            u(self.pad[0]),
            u(self.pad[2]),
            u(self.dilation[0]),
        )?;
        let wo = conv_len(
            u(w),
            u(self.kernel[1]),
            u(self.stride[1]),
            u(self.pad[1]),
            u(self.pad[3]),
            u(self.dilation[1]),
        )?;
        Some([
            self.out_channels,
            usize::try_from(ho).ok()?,
            usize::try_from(wo).ok()?,
        ])
    }

    /// The weight's shape, `[Co, Ci / groups, kh, kw]`.
    #[must_use]
    pub fn weight(&self) -> [usize; 4] {
        [
            self.out_channels,
            self.input[0] / self.groups.max(1),
            self.kernel[0],
            self.kernel[1],
        ]
    }

    /// FLOPs as the op table counts them: `2 · Co · Ho · Wo · (Ci / g) · kh · kw`.
    #[must_use]
    pub fn flops(&self) -> Option<u64> {
        let [co, ho, wo] = self.output()?;
        let [_, cig, kh, kw] = self.weight();
        [co, ho, wo, cig, kh, kw]
            .iter()
            .try_fold(2u64, |acc, v| acc.checked_mul(*v as u64))
    }

    /// The convolution of `x` (`[Ci, H, W]`) with `w` (`[Co, Ci / g, kh, kw]`) plus `bias` (`[Co]`) into `out`
    /// (`[Co, Ho, Wo]`, overwritten). Each output: 0, then `+ w · x` over its group's input channels, kernel rows
    /// and kernel columns in ascending order (padding skipped), then `+ b`.
    ///
    /// # Panics
    /// If a slice's length is not its shape's, or [`output`](Self::output) is `None`.
    pub fn run(&self, x: &[f32], w: &[f32], bias: Option<&[f32]>, out: &mut [f32]) {
        let [ci, h, wd] = self.input;
        let [co, ho, wo] = self.output().expect("a convolution that fits");
        let [_, cig, kh, kw] = self.weight();
        let [sh, sw] = self.stride;
        let [pt, pl, _, _] = self.pad;
        let [dh, dw] = self.dilation;
        assert_eq!(x.len(), ci * h * wd);
        assert_eq!(w.len(), co * cig * kh * kw);
        assert_eq!(out.len(), co * ho * wo);
        let cog = co / self.groups;
        let plane = ho * wo;
        // The output rows (or columns) a kernel tap at offset `off` reads in bounds: `o · s + off − pad ∈ [0, len)`.
        let valid = |len: usize, outs: usize, s: usize, off: usize, pad: usize| -> (usize, usize) {
            let lo = if pad > off {
                (pad - off).div_ceil(s)
            } else {
                0
            };
            let hi = if len + pad > off {
                ((len - 1 + pad - off) / s + 1).min(outs)
            } else {
                0
            };
            (lo, hi.max(lo))
        };
        for (o, acc) in out.chunks_exact_mut(plane).enumerate() {
            acc.fill(0.0);
            let first_in = (o / cog) * cig;
            for c in 0..cig {
                let xc = &x[(first_in + c) * h * wd..(first_in + c + 1) * h * wd];
                for i in 0..kh {
                    let (oh0, oh1) = valid(h, ho, sh, i * dh, pt);
                    for j in 0..kw {
                        let wv = w[((o * cig + c) * kh + i) * kw + j];
                        let (ow0, ow1) = valid(wd, wo, sw, j * dw, pl);
                        if ow1 == ow0 {
                            continue;
                        }
                        for oh in oh0..oh1 {
                            let ih = oh * sh + i * dh - pt;
                            let iw0 = ow0 * sw + j * dw - pl;
                            let row = &xc[ih * wd..(ih + 1) * wd];
                            let dst = &mut acc[oh * wo + ow0..oh * wo + ow1];
                            if sw == 1 {
                                for (a, v) in dst.iter_mut().zip(&row[iw0..iw0 + (ow1 - ow0)]) {
                                    *a = ftz(*a + ftz(wv * *v));
                                }
                            } else {
                                for (a, v) in dst.iter_mut().zip(row[iw0..].iter().step_by(sw)) {
                                    *a = ftz(*a + ftz(wv * *v));
                                }
                            }
                        }
                    }
                }
            }
            if let Some(b) = bias {
                let bv = b[o];
                for a in acc.iter_mut() {
                    *a = ftz(*a + bv);
                }
            }
        }
    }
}

/// `Linear` with the weight transposed (`wt` is `[n, m]`): `out` (`[rows, m]`) = `x` (`[rows, n]`) `· wt + b`.
/// Each output: 0, then `+ x[i] · w[j, i]` for `i` ascending, then `+ b[j]`, the same order as [`linear`], so the
/// two give the same bits (this one runs the outputs of a row side by side).
///
/// # Panics
/// If a slice's length is not its shape's.
pub fn linear_t(x: &[f32], n: usize, wt: &[f32], m: usize, bias: Option<&[f32]>, out: &mut [f32]) {
    assert_eq!(wt.len(), n * m);
    assert_eq!(x.len() / n.max(1), out.len() / m.max(1));
    for (xr, acc) in x.chunks_exact(n).zip(out.chunks_exact_mut(m)) {
        acc.fill(0.0);
        for (xv, wr) in xr.iter().zip(wt.chunks_exact(m)) {
            for (a, wv) in acc.iter_mut().zip(wr) {
                *a = ftz(*a + ftz(*xv * *wv));
            }
        }
        if let Some(b) = bias {
            for (a, bv) in acc.iter_mut().zip(b) {
                *a = ftz(*a + *bv);
            }
        }
    }
}

/// `Linear`: `out` (`[rows, m]`) = `x` (`[rows, n]`) `· wᵀ + b`, with `w` as stored (`[m, n]`). The same order and
/// bits as [`linear_t`] (one dot product at a time; used when the weight is computed, not stored).
///
/// # Panics
/// If a slice's length is not its shape's.
pub fn linear(x: &[f32], n: usize, w: &[f32], m: usize, bias: Option<&[f32]>, out: &mut [f32]) {
    assert_eq!(w.len(), n * m);
    assert_eq!(x.len() / n.max(1), out.len() / m.max(1));
    for (xr, yr) in x.chunks_exact(n).zip(out.chunks_exact_mut(m)) {
        for (j, y) in yr.iter_mut().enumerate() {
            let mut acc = 0.0f32;
            for (xv, wv) in xr.iter().zip(&w[j * n..(j + 1) * n]) {
                acc = ftz(acc + ftz(*xv * *wv));
            }
            if let Some(b) = bias {
                acc = ftz(acc + b[j]);
            }
            *y = acc;
        }
    }
}

/// One matrix product: `out` (`[p, r]`) = `a` (`[p, q]`) `· b` (`[q, r]`). Each output: 0, then
/// `+ a[p, q] · b[q, r]` for `q` ascending.
///
/// # Panics
/// If a slice's length is not its shape's.
pub fn matmul(a: &[f32], b: &[f32], [p, q, r]: [usize; 3], out: &mut [f32]) {
    assert_eq!((a.len(), b.len(), out.len()), (p * q, q * r, p * r));
    for (ar, acc) in a.chunks_exact(q).zip(out.chunks_exact_mut(r)) {
        acc.fill(0.0);
        for (av, br) in ar.iter().zip(b.chunks_exact(r)) {
            for (o, bv) in acc.iter_mut().zip(br) {
                *o = ftz(*o + ftz(*av * *bv));
            }
        }
    }
}

/// `LayerNorm` over rows of `n`: `μ = Σx / n`, `σ² = Σ(x − μ)² / n`, `y = ((x − μ) / √(σ² + ε)) · γ + β`.
///
/// # Panics
/// If a slice's length is not its shape's.
pub fn layer_norm(x: &[f32], gamma: &[f32], beta: &[f32], eps: f32, out: &mut [f32]) {
    let n = gamma.len();
    assert!(n > 0 && beta.len() == n && x.len() == out.len() && x.len().is_multiple_of(n));
    let nf = n as f32;
    for (xr, yr) in x.chunks_exact(n).zip(out.chunks_exact_mut(n)) {
        let mut sum = 0.0f32;
        for v in xr {
            sum = ftz(sum + *v);
        }
        let mean = ftz(sum / nf);
        let mut sq = 0.0f32;
        for (y, v) in yr.iter_mut().zip(xr) {
            let d = ftz(*v - mean);
            *y = d;
            sq = ftz(sq + ftz(d * d));
        }
        let denom = ftz(ftz(sq / nf) + eps).sqrt();
        for ((y, g), b) in yr.iter_mut().zip(gamma).zip(beta) {
            *y = ftz(ftz(ftz(*y / denom) * *g) + *b);
        }
    }
}

/// The output length along one axis of a pool: `⌊(len + pad_begin + pad_end − k) / stride⌋ + 1`, or `None`.
#[must_use]
pub fn pool_len(len: u64, k: u64, stride: u64, pad_begin: u64, pad_end: u64) -> Option<u64> {
    conv_len(len, k, stride, pad_begin, pad_end, 1)
}

/// A 2-D pool's geometry on `[C, H, W]` (floor mode; each pad below its kernel, so every window holds at least one
/// in-bounds element). A 1-D pool is the same with `H = 1`, a kernel height of 1 and no vertical padding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pool2d {
    /// Channels, height, width.
    pub input: [usize; 3],
    /// Kernel height and width.
    pub kernel: [usize; 2],
    /// Stride: vertical, horizontal.
    pub stride: [usize; 2],
    /// Padding: top, left, bottom, right.
    pub pad: [usize; 4],
}

impl Pool2d {
    /// The output's `[C, Ho, Wo]`, or `None` when the kernel does not fit or a pad is not below its kernel.
    #[must_use]
    pub fn output(&self) -> Option<[usize; 3]> {
        let [c, h, w] = self.input;
        let [kh, kw] = self.kernel;
        let [pt, pl, pb, pr] = self.pad;
        if pt >= kh || pb >= kh || pl >= kw || pr >= kw {
            return None;
        }
        let u = |v: usize| v as u64;
        let ho = pool_len(u(h), u(kh), u(self.stride[0]), u(pt), u(pb))?;
        let wo = pool_len(u(w), u(kw), u(self.stride[1]), u(pl), u(pr))?;
        Some([c, usize::try_from(ho).ok()?, usize::try_from(wo).ok()?])
    }

    /// The in-bounds rows and columns of output `(oh, ow)`'s window.
    fn window(&self, oh: usize, ow: usize) -> (core::ops::Range<usize>, core::ops::Range<usize>) {
        let [_, h, w] = self.input;
        let span = |o: usize, s: usize, pad: usize, k: usize, len: usize| {
            let start = o * s; // in padded coordinates
            let lo = start.max(pad) - pad;
            let hi = (start + k).min(len + pad) - pad;
            lo..hi
        };
        (
            span(oh, self.stride[0], self.pad[0], self.kernel[0], h),
            span(ow, self.stride[1], self.pad[1], self.kernel[1], w),
        )
    }

    /// Max pooling of `x` into `out`: per window, the first in-bounds element, replaced by each later one that is
    /// `>` it (row-major); padding is never chosen.
    ///
    /// # Panics
    /// If a slice's length is not its shape's, or [`output`](Self::output) is `None`.
    pub fn max(&self, x: &[f32], out: &mut [f32]) {
        self.each(x, out, |xc, rows, cols, w| {
            let mut m = xc[rows.start * w + cols.start];
            for r in rows {
                for v in &xc[r * w + cols.start..r * w + cols.end] {
                    if *v > m {
                        m = *v;
                    }
                }
            }
            m
        });
    }

    /// Average pooling of `x` into `out`: per window, the in-bounds elements summed row-major, then divided by
    /// their count (padding not counted: ONNX `count_include_pad = 0`).
    ///
    /// # Panics
    /// If a slice's length is not its shape's, or [`output`](Self::output) is `None`.
    pub fn avg(&self, x: &[f32], out: &mut [f32]) {
        self.each(x, out, |xc, rows, cols, w| {
            let count = (rows.len() * cols.len()) as f32;
            let mut s = 0.0f32;
            for r in rows {
                for v in &xc[r * w + cols.start..r * w + cols.end] {
                    s = ftz(s + *v);
                }
            }
            ftz(s / count)
        });
    }

    fn each(
        &self,
        x: &[f32],
        out: &mut [f32],
        f: impl Fn(&[f32], core::ops::Range<usize>, core::ops::Range<usize>, usize) -> f32,
    ) {
        let [c, h, w] = self.input;
        let [_, ho, wo] = self.output().expect("a pool that fits");
        assert_eq!((x.len(), out.len()), (c * h * w, c * ho * wo));
        for (xc, oc) in x.chunks_exact(h * w).zip(out.chunks_exact_mut(ho * wo)) {
            for oh in 0..ho {
                for ow in 0..wo {
                    let (rows, cols) = self.window(oh, ow);
                    oc[oh * wo + ow] = f(xc, rows, cols, w);
                }
            }
        }
    }
}

/// `GlobalAvgPool` over `x` (`[C, spatial…]`, `spatial` elements per channel): each channel's elements summed in
/// order, then divided by their count.
///
/// # Panics
/// If `x` is not `out.len() · spatial` long.
pub fn global_avg_pool(x: &[f32], spatial: usize, out: &mut [f32]) {
    assert_eq!(x.len(), out.len() * spatial);
    let count = spatial as f32;
    for (xc, o) in x.chunks_exact(spatial).zip(out.iter_mut()) {
        let mut s = 0.0f32;
        for v in xc {
            s = ftz(s + *v);
        }
        *o = ftz(s / count);
    }
}

/// `GlobalMaxPool` over `x` (`[C, spatial…]`): each channel's first element, replaced by each later one that is
/// `>` it.
///
/// # Panics
/// If `x` is not `out.len() · spatial` long, or `spatial` is 0.
pub fn global_max_pool(x: &[f32], spatial: usize, out: &mut [f32]) {
    assert_eq!(x.len(), out.len() * spatial);
    for (xc, o) in x.chunks_exact(spatial).zip(out.iter_mut()) {
        let mut m = xc[0];
        for v in &xc[1..] {
            if *v > m {
                m = *v;
            }
        }
        *o = m;
    }
}

/// `Softmax` along an axis of length `len`, with `inner` elements after it per step (the product of the later
/// axes): per line, `m` = the maximum (the first wins), `e = exp(x − m)`, `s = Σ e` in order, `y = e / s`.
///
/// # Panics
/// If `x` and `out` differ in length, or the length is not a multiple of `len · inner`.
pub fn softmax(x: &[f32], len: usize, inner: usize, out: &mut [f32]) {
    assert!(x.len() == out.len() && len > 0 && inner > 0 && x.len().is_multiple_of(len * inner));
    for (xb, ob) in x
        .chunks_exact(len * inner)
        .zip(out.chunks_exact_mut(len * inner))
    {
        for k in 0..inner {
            let mut m = xb[k];
            for i in 1..len {
                let v = xb[i * inner + k];
                if v > m {
                    m = v;
                }
            }
            let mut s = 0.0f32;
            for i in 0..len {
                let e = ftz(libm::expf(ftz(xb[i * inner + k] - m)));
                ob[i * inner + k] = e;
                s = ftz(s + e);
            }
            for i in 0..len {
                let y = &mut ob[i * inner + k];
                *y = ftz(*y / s);
            }
        }
    }
}

/// Row-major strides of `dims`, with 0 for an axis of length 1 when `broadcast` (so a broadcast axis re-reads).
fn strides(dims: &[usize], rank: usize, broadcast: bool) -> [usize; super::pack::MAX_RANK] {
    let mut out = [0usize; super::pack::MAX_RANK];
    let lead = rank - dims.len();
    let mut s = 1;
    for (i, d) in dims.iter().enumerate().rev() {
        out[lead + i] = if broadcast && *d == 1 { 0 } else { s };
        s *= d;
    }
    out
}

/// The numpy broadcast of two shapes (right-aligned; each pair of axes equal or one of them 1), or `None`.
#[must_use]
pub fn broadcast_shape(a: &[usize], b: &[usize]) -> Option<Vec<usize>> {
    let rank = a.len().max(b.len());
    let at = |d: &[usize], i: usize| {
        let lead = rank - d.len();
        if i < lead { 1 } else { d[i - lead] }
    };
    (0..rank)
        .map(|i| match (at(a, i), at(b, i)) {
            (x, y) if x == y => Some(x),
            (1, y) => Some(y),
            (x, 1) => Some(x),
            _ => None,
        })
        .collect()
}

/// Calls `f(out_index, a_index, b_index)` for every element of the broadcast of `a_dims` and `b_dims` (whose shape
/// is `out_dims`), in row-major order.
fn for_each_broadcast(
    a_dims: &[usize],
    b_dims: &[usize],
    out_dims: &[usize],
    mut f: impl FnMut(usize, usize, usize),
) {
    let rank = out_dims.len();
    let (sa, sb) = (strides(a_dims, rank, true), strides(b_dims, rank, true));
    let total: usize = out_dims.iter().product();
    let mut idx = [0usize; super::pack::MAX_RANK];
    let (mut ia, mut ib) = (0usize, 0usize);
    for o in 0..total {
        f(o, ia, ib);
        // Advance the multi-index, last axis fastest.
        for ax in (0..rank).rev() {
            idx[ax] += 1;
            ia += sa[ax];
            ib += sb[ax];
            if idx[ax] < out_dims[ax] {
                break;
            }
            ia -= sa[ax] * idx[ax];
            ib -= sb[ax] * idx[ax];
            idx[ax] = 0;
        }
    }
}

/// `Add`, `Mul` or `Sub` of `a` (`a_dims`) and `b` (`b_dims`) into `out` (their numpy broadcast, `out_dims`).
///
/// # Panics
/// If a slice's length is not its shape's.
pub fn binary(
    op: Binary,
    a: &[f32],
    a_dims: &[usize],
    b: &[f32],
    b_dims: &[usize],
    out_dims: &[usize],
    out: &mut [f32],
) {
    assert_eq!(a.len(), a_dims.iter().product::<usize>());
    assert_eq!(b.len(), b_dims.iter().product::<usize>());
    assert_eq!(out.len(), out_dims.iter().product::<usize>());
    if a_dims == b_dims {
        for ((o, x), y) in out.iter_mut().zip(a).zip(b) {
            *o = op.apply(*x, *y);
        }
        return;
    }
    for_each_broadcast(a_dims, b_dims, out_dims, |o, ia, ib| {
        out[o] = op.apply(a[ia], b[ib]);
    });
}

/// `MatMul` with numpy broadcasting of the leading axes: `a` (`[…, p, q]`) times `b` (`[…, q, r]`) into `out`
/// (`[…, p, r]`, the leading axes broadcast). Each output as [`matmul`].
///
/// # Panics
/// If a slice's length is not its shape's, or a rank is below 2.
pub fn batched_matmul(
    a: &[f32],
    a_dims: &[usize],
    b: &[f32],
    b_dims: &[usize],
    out_dims: &[usize],
    out: &mut [f32],
) {
    let (ra, rb, ro) = (a_dims.len(), b_dims.len(), out_dims.len());
    assert!(ra >= 2 && rb >= 2 && ro >= 2);
    let (p, q, r) = (a_dims[ra - 2], a_dims[ra - 1], b_dims[rb - 1]);
    assert_eq!(b_dims[rb - 2], q);
    let (ma, mb, mo) = (p * q, q * r, p * r);
    let lead = &out_dims[..ro - 2];
    for_each_broadcast(&a_dims[..ra - 2], &b_dims[..rb - 2], lead, |o, ia, ib| {
        matmul(
            &a[ia * ma..(ia + 1) * ma],
            &b[ib * mb..(ib + 1) * mb],
            [p, q, r],
            &mut out[o * mo..(o + 1) * mo],
        );
    });
}

/// `Concat` along `axis`: `parts` are `(values, dims)` with every other axis equal; `out` holds the join.
///
/// # Panics
/// If the lengths do not add up.
pub fn concat(parts: &[(&[f32], &[usize])], axis: usize, out: &mut [f32]) {
    let outer: usize = parts[0].1[..axis].iter().product();
    let mut at = 0;
    for o in 0..outer {
        for (x, dims) in parts {
            let chunk: usize = dims[axis..].iter().product();
            out[at..at + chunk].copy_from_slice(&x[o * chunk..(o + 1) * chunk]);
            at += chunk;
        }
    }
    assert_eq!(at, out.len());
}

/// `Transpose`: output axis `i` is input axis `perm[i]`.
///
/// # Panics
/// If a slice's length is not its shape's, or `perm` is not a permutation of `0..dims.len()`.
pub fn transpose(x: &[f32], dims: &[usize], perm: &[usize], out: &mut [f32]) {
    let rank = dims.len();
    assert_eq!(
        (perm.len(), x.len(), out.len()),
        (rank, dims.iter().product(), x.len())
    );
    let s = strides(dims, rank, false);
    let mut out_dims = [0usize; super::pack::MAX_RANK];
    let mut src = [0usize; super::pack::MAX_RANK];
    for (i, p) in perm.iter().enumerate() {
        out_dims[i] = dims[*p];
        src[i] = s[*p];
    }
    let mut idx = [0usize; super::pack::MAX_RANK];
    let mut at = 0usize;
    for o in out.iter_mut() {
        *o = x[at];
        for ax in (0..rank).rev() {
            idx[ax] += 1;
            at += src[ax];
            if idx[ax] < out_dims[ax] {
                break;
            }
            at -= src[ax] * idx[ax];
            idx[ax] = 0;
        }
    }
}

/// `Slice` along `axis`: elements `start, start + step, …` (`len` of them) of `x` (`dims`) into `out`.
///
/// # Panics
/// If a slice's length is not its shape's, or the range runs past the axis.
pub fn slice(
    x: &[f32],
    dims: &[usize],
    axis: usize,
    start: usize,
    step: usize,
    len: usize,
    out: &mut [f32],
) {
    let outer: usize = dims[..axis].iter().product();
    let inner: usize = dims[axis + 1..].iter().product();
    let d = dims[axis];
    assert!(start + (len - 1) * step < d);
    assert_eq!(out.len(), outer * len * inner);
    let mut at = 0;
    for o in 0..outer {
        for i in 0..len {
            let from = (o * d + start + i * step) * inner;
            out[at..at + inner].copy_from_slice(&x[from..from + inner]);
            at += inner;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ftz_flushes_tiny_values_and_signed_zeros_only() {
        assert_eq!(ftz(1e-40).to_bits(), 0);
        assert_eq!(ftz(-1e-40).to_bits(), 0);
        assert_eq!(ftz(-0.0).to_bits(), 0);
        assert_eq!(ftz(f32::MIN_POSITIVE).to_bits(), 0);
        let next = f32::from_bits(f32::MIN_POSITIVE.to_bits() + 1);
        assert_eq!(ftz(next), next);
        assert_eq!(ftz(-1.5), -1.5);
        assert!(ftz(f32::NAN).is_nan());
        assert_eq!(ftz(f32::INFINITY), f32::INFINITY);
    }

    #[test]
    fn activations_follow_the_op_table() {
        assert_eq!(relu(-0.0).to_bits(), 0);
        assert_eq!(relu(-3.0), 0.0);
        assert_eq!(relu(2.5), 2.5);
        assert!(relu(f32::NAN).is_nan());
        assert_eq!(sigmoid(0.0), 0.5);
        assert_eq!(sigmoid(200.0), 1.0);
        // exp(100) is finite, 1 / (1 + e^100) is subnormal-free after the flush; exp(200) is infinite: 1/inf = 0.
        assert_eq!(sigmoid(-200.0).to_bits(), 0);
        assert_eq!(sigmoid(-100.0).to_bits(), 0); // 3.7e-44 is subnormal: flushed
        assert!(sigmoid(-80.0) > 0.0);
        assert_eq!(silu(0.0), 0.0);
        assert!((silu(1.0) - 0.731_058_6).abs() < 1e-6);
        assert!((gelu(1.0) - 0.841_344_7).abs() < 1e-6);
        assert!((gelu(-1.0) + 0.158_655_3).abs() < 1e-6);
        assert_eq!(gelu(-10.0).to_bits(), 0);
        assert_eq!(gelu(10.0), 10.0);
        assert!((tanh(0.5) - 0.462_117_16).abs() < 1e-6);
        assert_eq!(tanh(1e-39), 0.0);
        // A tiny normal input: GELU is 0.5·x exactly (1 + erf(tiny) rounds to 1 however erf's tail is computed).
        let tiny = 3.0e-38f32;
        assert_eq!(gelu(tiny), 0.5 * tiny);
    }

    #[test]
    fn conv1d_matches_a_direct_loop_with_padding_stride_dilation_and_groups() {
        // [Ci = 4, L = 9], groups 2, Co 4, k 3, stride 2, pads (2, 1), dilation 2.
        let x: Vec<f32> = (0..36).map(|i| libm::sinf(i as f32 * 0.37)).collect();
        let w: Vec<f32> = (0..24).map(|i| i as f32 * 0.11 - 1.0).collect();
        let b = [0.5, -0.25, 1.0, 0.0];
        let c = Conv2d {
            input: [4, 1, 9],
            out_channels: 4,
            kernel: [1, 3],
            stride: [1, 2],
            pad: [0, 2, 0, 1],
            dilation: [1, 2],
            groups: 2,
        };
        let [_, _, lo] = c.output().unwrap();
        assert_eq!(lo, (9 + 2 + 1 - 2 * 2 - 1) / 2 + 1);
        let mut out = vec![0.0; 4 * lo];
        c.run(&x, &w, Some(&b), &mut out);
        for o in 0..4 {
            let g = o / 2;
            for l in 0..lo {
                let mut acc = 0.0f32;
                for ci in 0..2 {
                    for k in 0..3 {
                        let pos = (l * 2 + k * 2) as i64 - 2;
                        if (0..9).contains(&pos) {
                            acc = ftz(acc
                                + ftz(w[(o * 2 + ci) * 3 + k] * x[(g * 2 + ci) * 9 + pos as usize]));
                        }
                    }
                }
                acc = ftz(acc + b[o]);
                assert_eq!(out[o * lo + l].to_bits(), acc.to_bits(), "o {o} l {l}");
            }
        }
        assert_eq!(c.flops(), Some(2 * 4 * lo as u64 * 2 * 3));
    }

    #[test]
    fn conv2d_matches_a_direct_loop() {
        let c = Conv2d {
            input: [3, 7, 6],
            out_channels: 2,
            kernel: [3, 2],
            stride: [2, 1],
            pad: [1, 0, 2, 1],
            dilation: [1, 2],
            groups: 1,
        };
        let x: Vec<f32> = (0..3 * 7 * 6)
            .map(|i| libm::sinf(i as f32 * 0.21))
            .collect();
        let w: Vec<f32> = (0..2 * 3 * 3 * 2)
            .map(|i| libm::sinf(i as f32 * 0.07))
            .collect();
        let [co, ho, wo] = c.output().unwrap();
        let mut out = vec![0.0; co * ho * wo];
        c.run(&x, &w, None, &mut out);
        for o in 0..co {
            for oh in 0..ho {
                for ow in 0..wo {
                    let mut acc = 0.0f32;
                    for ci in 0..3 {
                        for i in 0..3 {
                            for j in 0..2 {
                                let ih = (oh * 2 + i) as i64 - 1;
                                let iw = (ow + j * 2) as i64;
                                if (0..7).contains(&ih) && (0..6).contains(&iw) {
                                    let xv = x[(ci * 7 + ih as usize) * 6 + iw as usize];
                                    acc = ftz(acc + ftz(w[((o * 3 + ci) * 3 + i) * 2 + j] * xv));
                                }
                            }
                        }
                    }
                    assert_eq!(out[(o * ho + oh) * wo + ow].to_bits(), acc.to_bits());
                }
            }
        }
    }

    #[test]
    fn both_linear_forms_give_the_same_bits() {
        let (rows, n, m) = (3, 7, 5);
        let x: Vec<f32> = (0..rows * n).map(|i| libm::sinf(i as f32 * 0.3)).collect();
        let w: Vec<f32> = (0..m * n).map(|i| libm::sinf(i as f32 * 0.17)).collect();
        let b: Vec<f32> = (0..m).map(|i| i as f32 * 0.1).collect();
        let mut wt = vec![0.0; n * m];
        transpose(&w, &[m, n], &[1, 0], &mut wt);
        let (mut a, mut c) = (vec![0.0; rows * m], vec![0.0; rows * m]);
        linear(&x, n, &w, m, Some(&b), &mut a);
        linear_t(&x, n, &wt, m, Some(&b), &mut c);
        assert_eq!(
            a.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
            c.iter().map(|v| v.to_bits()).collect::<Vec<_>>()
        );
    }

    #[test]
    fn pools_skip_padding() {
        // One channel [1, 1, 4]: kernel 3, stride 1, pads (1, 1): windows [-1..1], [0..2], [1..3], [2..4].
        let p = Pool2d {
            input: [1, 1, 4],
            kernel: [1, 3],
            stride: [1, 1],
            pad: [0, 1, 0, 1],
        };
        let x = [1.0, -2.0, 3.0, -4.0];
        let mut out = [0.0; 4];
        p.avg(&x, &mut out);
        assert_eq!(out, [-0.5, 2.0 / 3.0, -1.0, -0.5]);
        p.max(&x, &mut out);
        assert_eq!(out, [1.0, 3.0, 3.0, 3.0]);
        let negative = [-1.0, -2.0, -3.0, -4.0];
        p.max(&negative, &mut out);
        assert_eq!(
            out,
            [-1.0, -1.0, -2.0, -3.0],
            "padding is never the maximum"
        );
    }

    #[test]
    fn softmax_subtracts_the_maximum_and_sums_to_one() {
        let x = [1000.0, 1001.0, 1002.0, -5.0, 0.0, 5.0];
        let mut out = [0.0; 6];
        softmax(&x, 3, 1, &mut out);
        assert!(out.iter().all(|v| v.is_finite()));
        assert!((out[..3].iter().sum::<f32>() - 1.0).abs() < 1e-6);
        // Along axis 0 of [2, 3].
        softmax(&x, 2, 3, &mut out);
        assert_eq!(out[3], 0.0, "exp(-1005) underflows and is flushed");
        assert_eq!(out[0], 1.0);
    }

    #[test]
    fn broadcasting_is_numpys() {
        assert_eq!(broadcast_shape(&[2, 1, 3], &[4, 1]), Some(vec![2, 4, 3]));
        assert_eq!(broadcast_shape(&[3], &[2, 3]), Some(vec![2, 3]));
        assert_eq!(broadcast_shape(&[2], &[3]), None);
        let a = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0]; // [2, 3]
        let b = [10.0, 20.0]; // [2, 1]
        let mut out = [0.0; 6];
        binary(Binary::Sub, &a, &[2, 3], &b, &[2, 1], &[2, 3], &mut out);
        assert_eq!(out, [-9.0, -8.0, -7.0, -16.0, -15.0, -14.0]);
        let c = [1.0, 0.0, -1.0]; // [3]
        binary(Binary::Mul, &a, &[2, 3], &c, &[3], &[2, 3], &mut out);
        assert_eq!(
            out.map(f32::to_bits),
            [1.0, 0.0, -3.0, 4.0, 0.0, -6.0].map(f32::to_bits)
        );
    }

    #[test]
    fn matmul_broadcasts_leading_axes() {
        // a [2, 2, 3] times b [3, 1]: b is shared by both batches.
        let a: Vec<f32> = (0..12).map(|i| i as f32).collect();
        let b = [1.0, 0.5, -1.0];
        let mut out = [0.0; 4];
        batched_matmul(&a, &[2, 2, 3], &b, &[3, 1], &[2, 2, 1], &mut out);
        assert_eq!(out, [-1.5, 0.0, 1.5, 3.0]);
    }

    #[test]
    fn layout_ops_move_values_only() {
        let x: Vec<f32> = (0..24).map(|i| i as f32).collect(); // [2, 3, 4]
        let mut t = vec![0.0; 24];
        transpose(&x, &[2, 3, 4], &[2, 0, 1], &mut t); // [4, 2, 3]
        assert_eq!(t[..6], [0.0, 4.0, 8.0, 12.0, 16.0, 20.0]);
        assert_eq!(t[6], 1.0);
        let mut s = vec![0.0; 2 * 2 * 4];
        slice(&x, &[2, 3, 4], 1, 0, 2, 2, &mut s); // rows 0 and 2
        assert_eq!(s[..8], [0.0, 1.0, 2.0, 3.0, 8.0, 9.0, 10.0, 11.0]);
        let mut c = vec![0.0; 2 * 3 * 5];
        let one: Vec<f32> = vec![-1.0; 6];
        concat(&[(&x, &[2, 3, 4]), (&one, &[2, 3, 1])], 2, &mut c);
        assert_eq!(c[..6], [0.0, 1.0, 2.0, 3.0, -1.0, 4.0]);
    }

    #[test]
    fn layer_norm_is_biased_with_epsilon_inside_the_root() {
        let x = [1.0, 2.0, 3.0, 4.0];
        let mut out = [0.0; 4];
        layer_norm(&x, &[1.0; 4], &[0.0; 4], 1e-5, &mut out);
        let var = 1.25f32; // biased: (2.25 + 0.25 + 0.25 + 2.25) / 4
        assert!((out[0] - (-1.5 / (var + 1e-5).sqrt())).abs() < 1e-7);
        assert!((out.iter().sum::<f32>()).abs() < 1e-6);
    }
}
