//! Rendering: the patch goes through the metric spike's toy voice, then through 16-bit quantisation, so that what
//! the descriptors measure is exactly what the WAV file holds (`describe patch.json` equals `describe patch.wav`).

use crate::descriptors::{Describer, Descriptors, Want};
use crate::error::{CliError, EXIT_RENDER, Res};
use crate::patch::Patch;
use spike_metric::render;
use spike_metric::wav;

pub const SAMPLE_RATE: f32 = 48_000.0;

/// Round to what a 16-bit WAV can hold (the same arithmetic as `spike_metric::wav`).
pub fn quantise(x: &mut [f32]) {
    for s in x {
        *s = (s.clamp(-1.0, 1.0) * 32767.0).round() / 32767.0;
    }
}

/// Render the patch's note: mono, 48 kHz, 16-bit quantised.
pub fn render_patch(p: &Patch) -> Res<Vec<f32>> {
    let (cfg, toy) = p.to_toy();
    let mut out = Vec::new();
    render::render(&cfg, &toy, &mut out);
    if out.iter().any(|s| !s.is_finite()) {
        return Err(CliError::new(
            EXIT_RENDER,
            "render_failed",
            "the render produced non-finite samples",
        )
        .hint("lower filter.resonance or filter.cutoff and try again"));
    }
    quantise(&mut out);
    Ok(out)
}

/// Render a patch and measure it.
pub fn measure(p: &Patch, describer: &mut Describer, want: Want) -> Res<Descriptors> {
    let audio = render_patch(p)?;
    Ok(describer.describe(&audio, Some(p.perf.hold_s), want))
}

/// A 16-bit mono WAV file.
#[must_use]
pub fn wav_bytes(samples: &[f32]) -> Vec<u8> {
    wav::encode_wav_i16(samples, SAMPLE_RATE as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quantised_render_matches_the_wav_codec() {
        let mut p = Patch::default();
        p.perf.dur_s = 0.3;
        p.perf.hold_s = 0.2;
        let x = render_patch(&p).unwrap();
        let (rate, y) = wav::decode_wav_i16(&wav_bytes(&x)).unwrap();
        assert_eq!(rate, 48_000);
        assert_eq!(x, y);
    }

    #[test]
    fn render_is_deterministic_and_sounds() {
        let mut p = Patch::default();
        p.perf.dur_s = 0.3;
        p.perf.hold_s = 0.2;
        let a = render_patch(&p).unwrap();
        let b = render_patch(&p).unwrap();
        assert_eq!(a, b);
        assert!(a.iter().any(|s| s.abs() > 0.1));
    }
}
