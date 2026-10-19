//! fundsp 0.23's `reverb2_stereo` and `reverb4_stereo`, ADR-0006's fallback, as wet-only stereo processors.
//!
//! Both are the library's own constructions with its documented example settings (`reverb2_stereo(.., 1.0 modulation,
//! lowpole_hz(8000))`); only the room size and the reverberation time are ours. Room size is in metres: our `room`
//! parameter is a delay time where 1 ms is about a foot, so the catalogue default of 80 ms is about 24 m.

use fundsp::prelude::{AudioUnit, lowpole_hz, reverb2_stereo, reverb4_stereo};

/// Room size handed to fundsp, in metres (our default 80 ms room, at 1 ms per foot).
pub const ROOM_M: f64 = 24.0;
/// `reverb2_stereo`'s diffusion (the library's example value).
pub const DIFFUSION: f64 = 0.5;
/// `reverb2_stereo`'s modulation speed (the library's example value).
pub const MODULATION: f64 = 1.0;
/// `reverb2_stereo`'s loop low-pass, Hz (the library's example value).
pub const LOOP_LOWPASS_HZ: f32 = 8_000.0;

/// Which fundsp reverb.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// `reverb2_stereo`: a 32-line hybrid FDN with Schroeder allpass diffusers and modulation.
    Reverb2,
    /// `reverb4_stereo`: a 32-line FDN with a slow fade-in envelope.
    Reverb4,
}

fn build(kind: Kind, time_s: f64, sample_rate: u32) -> Box<dyn AudioUnit> {
    let mut node: Box<dyn AudioUnit> = match kind {
        Kind::Reverb2 => Box::new(reverb2_stereo(
            ROOM_M,
            time_s,
            DIFFUSION,
            MODULATION,
            lowpole_hz::<f32>(LOOP_LOWPASS_HZ),
        )),
        Kind::Reverb4 => Box::new(reverb4_stereo(ROOM_M, time_s)),
    };
    node.set_sample_rate(f64::from(sample_rate));
    node.reset();
    node.allocate();
    node
}

/// Run `dry` (stereo) through the reverb and return only the reverb's output (no dry signal), `frames` long. The
/// input is zero-padded or cut to `frames`.
#[must_use]
pub fn wet(
    kind: Kind,
    time_s: f64,
    sample_rate: u32,
    dry: [&[f32]; 2],
    frames: usize,
) -> [Vec<f32>; 2] {
    let mut node = build(kind, time_s, sample_rate);
    let mut left = Vec::with_capacity(frames);
    let mut right = Vec::with_capacity(frames);
    let mut out = [0.0f32; 2];
    for i in 0..frames {
        let input = [
            dry[0].get(i).copied().unwrap_or(0.0),
            dry[1].get(i).copied().unwrap_or(0.0),
        ];
        node.tick(&input, &mut out);
        left.push(out[0]);
        right.push(out[1]);
    }
    [left, right]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn impulse(frames: usize) -> Vec<f32> {
        let mut v = vec![0.0; frames];
        v[0] = 1.0;
        v
    }

    #[test]
    fn both_reverbs_make_a_stereo_tail_from_an_impulse() {
        for kind in [Kind::Reverb2, Kind::Reverb4] {
            let imp = impulse(1);
            let [l, r] = wet(kind, 1.0, 48_000, [&imp, &imp], 24_000);
            assert!(l.iter().chain(&r).all(|v| v.is_finite()));
            let energy = |x: &[f32]| x.iter().map(|v| f64::from(*v).powi(2)).sum::<f64>();
            assert!(
                energy(&l) > 1e-8 && energy(&r) > 1e-8,
                "{kind:?}: {} {}",
                energy(&l),
                energy(&r)
            );
            // stereo: the two channels differ
            assert!(
                l.iter().zip(&r).any(|(a, b)| (a - b).abs() > 1e-9),
                "{kind:?} is mono"
            );
        }
    }

    #[test]
    fn the_output_is_deterministic() {
        let imp = impulse(1);
        let a = wet(Kind::Reverb2, 1.0, 48_000, [&imp, &imp], 4_800);
        let b = wet(Kind::Reverb2, 1.0, 48_000, [&imp, &imp], 4_800);
        assert_eq!(a, b);
    }
}
