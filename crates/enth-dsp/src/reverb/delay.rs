//! Multi-channel delay lines: the power-of-two ring buffer, the 7th-order Lagrange reader and upstream's
//! `MultiDelay` helper (eight channels, per-channel integer offsets, A/B cross-fade between two offset sets).
//!
//! Derived from Signalsmith DSP `delay.h` (MIT, Copyright (c) 2021 Geraint Luff / Signalsmith Audio Ltd.:
//! `Buffer`, `MultiBuffer`, `Reader::read`, `InterpolatorLagrangeN<7>`) and Signalsmith Basics `reverb.h`
//! (MIT, Copyright 2022+ Signalsmith Audio Ltd. / Geraint Luff: `MultiDelay`). See `THIRD_PARTY_NOTICES.md`.
//! The expressions keep upstream's evaluation order, including every `f32`/`f64` mix, so results are bit-exact.

use super::table::Draw;

/// Channels of the feedback delay network.
pub(super) const CHANNELS: usize = 8;

/// Eight channels in one power-of-two ring. Channel `c` starts `c * stride` samples after the head; reads and
/// writes wrap with a mask. Allocated once, in [`MultiBuffer::new`].
pub(super) struct MultiBuffer {
    buf: Box<[f32]>,
    mask: u32,
    index: u32,
    stride: i32,
}

impl MultiBuffer {
    /// `capacity` is the per-channel length in samples; the ring holds the next power of two of `channels * capacity`.
    fn new(capacity: i32) -> Self {
        let min = CHANNELS as i32 * capacity;
        let mut len = 1i32;
        while len < min {
            len *= 2;
        }
        Self {
            buf: vec![0.0; len as usize].into_boxed_slice(),
            mask: (len - 1) as u32,
            index: 0,
            stride: capacity,
        }
    }

    fn clear(&mut self) {
        self.buf.fill(0.0);
    }

    /// Sample of channel `ch` at `offset` from the head (offsets are zero or negative: older samples).
    #[inline(always)]
    fn at(&self, ch: usize, offset: i32) -> f32 {
        let p = self
            .index
            .wrapping_add((ch as i32 * self.stride) as u32)
            .wrapping_add(offset as u32)
            & self.mask;
        self.buf[p as usize]
    }

    #[inline(always)]
    fn set(&mut self, ch: usize, v: f32) {
        let p = self.index.wrapping_add((ch as i32 * self.stride) as u32) & self.mask;
        self.buf[p as usize] = v;
    }

    /// Floats allocated.
    fn len(&self) -> usize {
        self.buf.len()
    }
}

/// `InterpolatorLagrangeN<float, 7>` and its `Reader`: Franck's (2009) product-tree evaluation of Lagrange
/// interpolation over eight taps, in upstream's exact order of operations.
pub(super) struct Lagrange7 {
    inv: [f32; 8],
}

impl Lagrange7 {
    pub(super) fn new() -> Self {
        let mut inv = [0.0f32; 8];
        for j in 0..=7i32 {
            let mut d = 1.0f64;
            for k in 0..j {
                d *= f64::from(j - k);
            }
            for k in (j + 1)..=7 {
                d *= f64::from(j - k);
            }
            inv[j as usize] = (1.0 / d) as f32;
        }
        Self { inv }
    }

    /// `Reader::read(buffer[ch], delay)`: the sample `delay` samples behind the head of channel `ch`.
    #[inline(always)]
    fn read(&self, b: &MultiBuffer, ch: usize, delay: f32) -> f32 {
        let start = delay as i32;
        let rem = delay - start as f32;
        let data = |j: i32| b.at(ch, -start - j);
        let x = rem + 3.0; // latency (n - 1) / 2 = 3
        let d: [f32; 8] = core::array::from_fn(|k| x - k as f32);
        let inv = &self.inv;
        let t01 = d[0] * d[1];
        let t23 = d[2] * d[3];
        let t03 = t01 * t23;
        let t45 = d[4] * d[5];
        let t67 = d[6] * d[7];
        let t47 = t45 * t67;
        // left.calculateResult(right.total, ..) + right.calculateResult(left.total, ..)
        let e = t47;
        let e01 = e * t23;
        let e23 = e * t01;
        let l = (((e01 * d[1]) * data(0)) * inv[0] + ((e01 * d[0]) * data(1)) * inv[1])
            + (((e23 * d[3]) * data(2)) * inv[2] + ((e23 * d[2]) * data(3)) * inv[3]);
        let f = t03;
        let f45 = f * t67;
        let f67 = f * t45;
        let r = (((f45 * d[5]) * data(4)) * inv[4] + ((f45 * d[4]) * data(5)) * inv[5])
            + (((f67 * d[7]) * data(6)) * inv[6] + ((f67 * d[6]) * data(7)) * inv[7]);
        l + r
    }
}

pub(super) type Array = [f32; CHANNELS];

/// Upstream's `MultiDelay`: eight delay lines sharing one ring, each with an integer offset that is re-drawn when
/// the room size changes. `offsets` is the target set, `prev` the set the last change started from; reads with a
/// `fade` ratio blend the two (`from + (to - from) * fade`).
pub(super) struct MultiDelay {
    buffer: MultiBuffer,
    scale: f64,
    offsets: [i32; CHANNELS],
    prev: [i32; CHANNELS],
}

impl MultiDelay {
    /// `max_range` is the longest room in samples and `scale` this stage's fraction of it. Allocates.
    pub(super) fn new(max_range: f64, scale: f64) -> Self {
        let capacity = (libm::ceil(max_range * scale) + 1.0) as i32;
        Self {
            buffer: MultiBuffer::new(capacity),
            scale,
            offsets: [0; CHANNELS],
            prev: [0; CHANNELS],
        }
    }

    /// Clear the audio. The offsets stay: they are recomputed from the parameters at the next block after a reset.
    pub(super) fn reset(&mut self) {
        self.buffer.clear();
    }

    /// Floats allocated for this stage.
    pub(super) fn len(&self) -> usize {
        self.buffer.len()
    }

    /// `updateLengths`: offsets spread over `range` samples from the constant draws of one seed, optionally shifted
    /// so the shortest delay is zero (`minimise`).
    pub(super) fn update_lengths(&mut self, draws: &[Draw; CHANNELS], range: f64, minimise: bool) {
        let range = range * self.scale;
        self.prev = self.offsets;
        for (i, &(bits, swap)) in draws.iter().enumerate() {
            let unit = f32::from_bits(bits);
            let sum = unit + i as f32; // float + int is a float addition in C++
            self.offsets[i] = (-libm::floor(range * f64::from(sum) / 8.0)) as i32;
            self.offsets.swap(i, usize::from(swap));
        }
        if minimise {
            // Moves things along so the shortest delay is always 0.
            let max = self.offsets.iter().copied().fold(self.offsets[0], i32::max);
            for d in &mut self.offsets {
                *d -= max;
            }
        }
    }

    /// `updateLengthsExponential`: offsets at 2^(+-1/16, +-3/16, ...) of the range (the feedback delay).
    pub(super) fn update_lengths_exponential(&mut self, range: f64) {
        const RATIOS: [f64; CHANNELS] = [
            0.0625, -0.0625, 0.1875, -0.1875, 0.3125, -0.3125, 0.4375, -0.4375,
        ];
        let range = range * self.scale;
        self.prev = self.offsets;
        for (offset, ratio) in self.offsets.iter_mut().zip(RATIOS) {
            *offset = (-libm::floor(range * libm::pow(2.0, ratio))) as i32;
        }
    }

    /// Advance the head and write one sample per channel.
    #[inline(always)]
    pub(super) fn write(&mut self, a: &Array) {
        self.buffer.index = self.buffer.index.wrapping_add(1);
        for (i, &v) in a.iter().enumerate() {
            self.buffer.set(i, v);
        }
    }

    #[inline(always)]
    pub(super) fn read(&self) -> Array {
        core::array::from_fn(|i| self.buffer.at(i, self.offsets[i]))
    }

    /// Read while cross-fading from the previous offsets to the current ones.
    #[inline(always)]
    pub(super) fn read_fade(&self, fade: f32) -> Array {
        core::array::from_fn(|i| {
            let to = self.buffer.at(i, self.offsets[i]);
            let from = self.buffer.at(i, self.prev[i]);
            from + (to - from) * fade
        })
    }

    /// Read with the first three channels detuned by a modulated fractional delay (`lfo` is each channel's
    /// extra delay in samples), optionally cross-fading.
    #[inline(always)]
    pub(super) fn read_detuned(
        &self,
        lagrange: &Lagrange7,
        lfo: &[f32; 3],
        fade: Option<f32>,
    ) -> Array {
        let mut r = [0.0f32; CHANNELS];
        for i in 0..3 {
            let to = lagrange.read(&self.buffer, i, lfo[i] - self.offsets[i] as f32);
            r[i] = match fade {
                None => to,
                Some(f) => {
                    let from = lagrange.read(&self.buffer, i, lfo[i] - self.prev[i] as f32);
                    from + (to - from) * f
                }
            };
        }
        for (i, slot) in r.iter_mut().enumerate().skip(3) {
            let to = self.buffer.at(i, self.offsets[i]);
            *slot = match fade {
                None => to,
                Some(f) => {
                    let from = self.buffer.at(i, self.prev[i]);
                    from + (to - from) * f
                }
            };
        }
        r
    }

    /// Test access: the samples in the ring.
    #[cfg(test)]
    pub(super) fn samples(&self) -> &[f32] {
        &self.buffer.buf
    }

    /// Test access: the current offsets.
    #[cfg(test)]
    pub(super) fn offsets(&self) -> [i32; CHANNELS] {
        self.offsets
    }

    /// Test access: samples per channel (the stride).
    #[cfg(test)]
    pub(super) fn capacity(&self) -> i32 {
        self.buffer.stride
    }
}
