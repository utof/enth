//! Minimal mono WAV (RIFF, 16-bit PCM) encoder and decoder, for the listening-test stimuli and their tests.

/// Encode mono 16-bit PCM. Samples clip at full scale.
#[must_use]
pub fn encode_wav_i16(samples: &[f32], sample_rate: u32) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + samples.len() * 2);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&(sample_rate * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        let v = (s.clamp(-1.0, 1.0) * 32767.0).round() as i16;
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

/// Decode what [`encode_wav_i16`] writes: `(sample_rate, samples in -1..1)`. `None` for anything else.
#[must_use]
pub fn decode_wav_i16(bytes: &[u8]) -> Option<(u32, Vec<f32>)> {
    if bytes.len() < 44
        || &bytes[0..4] != b"RIFF"
        || &bytes[8..16] != b"WAVEfmt "
        || &bytes[36..40] != b"data"
    {
        return None;
    }
    let rate = u32::from_le_bytes(bytes[24..28].try_into().ok()?);
    let len = u32::from_le_bytes(bytes[40..44].try_into().ok()?) as usize;
    let data = bytes.get(44..44 + len)?;
    let s = data
        .as_chunks::<2>()
        .0
        .iter()
        .map(|c| f32::from(i16::from_le_bytes(*c)) / 32767.0)
        .collect();
    Some((rate, s))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_header() {
        let x: Vec<f32> = (0..100).map(|i| (i as f32 / 50.0) - 1.0).collect();
        let bytes = encode_wav_i16(&x, 48_000);
        assert_eq!(bytes.len(), 44 + 200);
        let (rate, y) = decode_wav_i16(&bytes).expect("decodes");
        assert_eq!(rate, 48_000);
        assert_eq!(y.len(), 100);
        assert!(x.iter().zip(&y).all(|(a, b)| (a - b).abs() < 1e-4));
        assert!(decode_wav_i16(&bytes[..40]).is_none());
    }
}
