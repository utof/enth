//! Minimal WAV (RIFF, 24-bit PCM, mono or stereo) writer and reader for the stimuli.
//!
//! 24 bits, not 16: reverb tails fall 60 dB and more, and a 16-bit file would need dither to keep them clean.
//! `pw-play` (libsndfile) reads 24-bit PCM.

/// Audio in memory: planar channels of equal length.
#[derive(Clone, Debug, PartialEq)]
pub struct Wav {
    /// Frames per second.
    pub sample_rate: u32,
    /// One vector per channel (1 or 2), all the same length.
    pub channels: Vec<Vec<f32>>,
}

impl Wav {
    /// A mono clip.
    #[must_use]
    pub fn mono(sample_rate: u32, samples: Vec<f32>) -> Self {
        Self {
            sample_rate,
            channels: vec![samples],
        }
    }

    /// A stereo clip (`left` and `right` have the same length).
    #[must_use]
    pub fn stereo(sample_rate: u32, left: Vec<f32>, right: Vec<f32>) -> Self {
        assert_eq!(left.len(), right.len(), "channels have the same length");
        Self {
            sample_rate,
            channels: vec![left, right],
        }
    }

    /// Frames per channel.
    #[must_use]
    pub fn frames(&self) -> usize {
        self.channels.first().map_or(0, Vec::len)
    }

    /// Length in seconds.
    #[must_use]
    pub fn seconds(&self) -> f64 {
        self.frames() as f64 / f64::from(self.sample_rate)
    }

    /// The largest absolute sample.
    #[must_use]
    pub fn peak(&self) -> f32 {
        self.channels
            .iter()
            .flatten()
            .fold(0.0f32, |m, v| m.max(v.abs()))
    }

    /// Multiply every sample by `gain`.
    pub fn scale(&mut self, gain: f32) {
        for v in self.channels.iter_mut().flatten() {
            *v *= gain;
        }
    }
}

/// Encode as 24-bit PCM. Samples beyond full scale clip.
#[must_use]
pub fn encode(w: &Wav) -> Vec<u8> {
    let ch = w.channels.len() as u16;
    let frames = w.frames();
    let data_len = (frames * usize::from(ch) * 3) as u32;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&ch.to_le_bytes());
    out.extend_from_slice(&w.sample_rate.to_le_bytes());
    out.extend_from_slice(&(w.sample_rate * u32::from(ch) * 3).to_le_bytes());
    out.extend_from_slice(&(ch * 3).to_le_bytes());
    out.extend_from_slice(&24u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for i in 0..frames {
        for c in &w.channels {
            let v = (f64::from(c[i]).clamp(-1.0, 1.0) * 8_388_607.0).round() as i32;
            out.extend_from_slice(&v.to_le_bytes()[..3]);
        }
    }
    out
}

/// Decode what [`encode`] writes (24-bit PCM, 1 or 2 channels). `None` for anything else.
#[must_use]
pub fn decode(bytes: &[u8]) -> Option<Wav> {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return None;
    }
    let mut pos = 12;
    let mut fmt: Option<(u16, u32)> = None;
    while pos + 8 <= bytes.len() {
        let id = &bytes[pos..pos + 4];
        let len = u32::from_le_bytes(bytes[pos + 4..pos + 8].try_into().ok()?) as usize;
        let body = bytes.get(pos + 8..pos + 8 + len)?;
        match id {
            b"fmt " if len >= 16 => {
                let tag = u16::from_le_bytes(body[0..2].try_into().ok()?);
                let ch = u16::from_le_bytes(body[2..4].try_into().ok()?);
                let rate = u32::from_le_bytes(body[4..8].try_into().ok()?);
                let bits = u16::from_le_bytes(body[14..16].try_into().ok()?);
                if tag != 1 || bits != 24 || !(1..=2).contains(&ch) {
                    return None;
                }
                fmt = Some((ch, rate));
            }
            b"data" => {
                let (ch, rate) = fmt?;
                let ch = usize::from(ch);
                let frames = body.len() / (3 * ch);
                let mut channels = vec![Vec::with_capacity(frames); ch];
                for f in 0..frames {
                    for (c, chan) in channels.iter_mut().enumerate() {
                        let o = (f * ch + c) * 3;
                        let v = i32::from_le_bytes([0, body[o], body[o + 1], body[o + 2]]) >> 8;
                        chan.push((f64::from(v) / 8_388_607.0) as f32);
                    }
                }
                return Some(Wav {
                    sample_rate: rate,
                    channels,
                });
            }
            _ => {}
        }
        pos += 8 + len + (len & 1);
    }
    None
}

/// Concatenate clips with `gap_s` seconds of silence between them (what a trial plays: A, B and X, or A and B).
///
/// # Panics
/// If the clips differ in channel count or sample rate (the tools never build such a trial).
#[must_use]
pub fn assemble(parts: &[&Wav], gap_s: f32) -> Wav {
    let first = parts.first().expect("at least one clip");
    let gap = (f64::from(gap_s) * f64::from(first.sample_rate)).round() as usize;
    let mut out = Wav {
        sample_rate: first.sample_rate,
        channels: vec![Vec::new(); first.channels.len()],
    };
    for (k, p) in parts.iter().enumerate() {
        assert_eq!(
            p.sample_rate, first.sample_rate,
            "one sample rate per trial"
        );
        assert_eq!(p.channels.len(), first.channels.len(), "one channel count");
        if k > 0 {
            for c in &mut out.channels {
                c.resize(c.len() + gap, 0.0);
            }
        }
        for (dst, src) in out.channels.iter_mut().zip(&p.channels) {
            dst.extend_from_slice(src);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_mono_and_stereo() {
        let l: Vec<f32> = (0..200).map(|i| (i as f32 / 100.0) - 1.0).collect();
        let r: Vec<f32> = l.iter().map(|v| -v * 0.5).collect();
        let st = Wav::stereo(48_000, l.clone(), r);
        let bytes = encode(&st);
        assert_eq!(bytes.len(), 44 + 200 * 2 * 3);
        let back = decode(&bytes).expect("decodes");
        assert_eq!(back.sample_rate, 48_000);
        assert_eq!(back.channels.len(), 2);
        let worst = st
            .channels
            .iter()
            .flatten()
            .zip(back.channels.iter().flatten())
            .fold(0.0f32, |m, (a, b)| m.max((a - b).abs()));
        assert!(worst < 2e-7, "24-bit error {worst}");
        let mono = Wav::mono(44_100, l);
        assert_eq!(decode(&encode(&mono)).unwrap().channels.len(), 1);
        assert!(decode(&bytes[..40]).is_none());
        assert!(decode(b"not a wav file at all").is_none());
    }

    #[test]
    fn clips_beyond_full_scale_clip() {
        let w = Wav::mono(8_000, vec![2.0, -2.0, 0.0]);
        let back = decode(&encode(&w)).unwrap();
        assert!(
            (back.channels[0][0] - 1.0).abs() < 1e-6 && (back.channels[0][1] + 1.0).abs() < 1e-6
        );
    }

    #[test]
    fn assemble_puts_the_gap_between_clips() {
        let a = Wav::mono(1_000, vec![1.0; 10]);
        let b = Wav::mono(1_000, vec![0.5; 20]);
        let t = assemble(&[&a, &b, &a], 0.1);
        assert_eq!(t.frames(), 10 + 100 + 20 + 100 + 10);
        assert!(t.channels[0][10..110].iter().all(|v| *v == 0.0));
        assert_eq!(t.channels[0][110], 0.5);
        assert_eq!(assemble(&[&a], 0.5).frames(), 10);
    }
}
