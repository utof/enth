//! A small feedback delay network (4 lines, Hadamard mix, one-pole damping) standing in for the reverb.
//!
//! Cost shape matches a real reverb tail: a handful of delay-line reads and writes, a mixing matrix and a
//! damping filter per sample, with state that decays towards the subnormal range unless flushed. All
//! scale factors are powers of two or computed once in `new` with the pinned `libm`.

use crate::SAMPLE_RATE;
use crate::math::{self, flush};
use crate::patch::Reverb;
use crate::prng::{self, Rng};

/// Stream id for the per-effect seed: `derive(seed, MODULE_REVERB)` (design.md §5: per effect from the patch hash and module id).
pub const MODULE_REVERB: u64 = 0x5245_5642;

const BASE_LENGTHS: [usize; 4] = [1031, 1327, 1523, 1801];
const JITTER: u64 = 32;
const LN_1000: f32 = 6.907_755_4;

pub struct Fdn {
    lines: [Vec<f32>; 4],
    pos: [usize; 4],
    gain: [f32; 4],
    damp_state: [f32; 4],
    damp_coef: f32,
    wet: f32,
    dry: f32,
    flush: bool,
}

impl Fdn {
    /// Allocates the delay lines. Not real-time safe; call from `prepare`.
    #[must_use]
    pub fn new(p: &Reverb, seed: u64, flush: bool) -> Self {
        let mut rng = Rng::new(prng::derive(seed, MODULE_REVERB));
        let len: [usize; 4] = std::array::from_fn(|i| BASE_LENGTHS[i] + rng.below(JITTER) as usize);
        // RT60: each pass through a line of n samples loses 60 dB * n / (rt60 * sr).
        let gain: [f32; 4] =
            std::array::from_fn(|i| math::exp(-LN_1000 * len[i] as f32 / (p.rt60_s * SAMPLE_RATE)));
        let mix = p.mix_pct * 0.01;
        Self {
            lines: len.map(|n| vec![0.0; n]),
            pos: [0; 4],
            gain,
            damp_state: [0.0; 4],
            damp_coef: 1.0 - p.damping_pct * 0.01 * 0.9,
            wet: mix,
            dry: 1.0 - mix,
            flush,
        }
    }

    /// Process one block in place. Never allocates.
    #[allow(
        clippy::manual_midpoint,
        reason = "`(a + b) * 0.5` is the specified formula; f32::midpoint is not documented to be bit-identical"
    )]
    pub fn process(&mut self, block: &mut [f32]) {
        for x in block.iter_mut() {
            let input = *x * 0.25;
            let r: [f32; 4] = std::array::from_fn(|i| self.lines[i][self.pos[i]]);
            let mut d = [0.0f32; 4];
            for i in 0..4 {
                let mut lp = self.damp_state[i] + self.damp_coef * (r[i] - self.damp_state[i]);
                if self.flush {
                    lp = flush(lp);
                }
                self.damp_state[i] = lp;
                d[i] = lp * self.gain[i];
            }
            // Hadamard / 2 (orthogonal; scaling by 0.5 is exact).
            let s0 = d[0] + d[1];
            let s1 = d[0] - d[1];
            let s2 = d[2] + d[3];
            let s3 = d[2] - d[3];
            let m = [
                (s0 + s2) * 0.5,
                (s1 + s3) * 0.5,
                (s0 - s2) * 0.5,
                (s1 - s3) * 0.5,
            ];
            for i in 0..4 {
                let mut w = input + m[i];
                if self.flush {
                    w = flush(w);
                }
                let line = &mut self.lines[i];
                line[self.pos[i]] = w;
                self.pos[i] += 1;
                if self.pos[i] == line.len() {
                    self.pos[i] = 0;
                }
            }
            // Fixed tap order and signs.
            let wet = ((r[0] - r[1]) + (r[2] - r[3])) * 0.5;
            *x = *x * self.dry + wet * self.wet;
        }
    }

    /// Number of subnormal values in the network's state (delay lines and damping filters).
    #[must_use]
    pub fn count_subnormals(&self) -> usize {
        let lines: usize = self
            .lines
            .iter()
            .map(|l| l.iter().filter(|x| x.is_subnormal()).count())
            .sum();
        lines + self.damp_state.iter().filter(|x| x.is_subnormal()).count()
    }
}
