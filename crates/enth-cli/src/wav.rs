//! WAV files: the writer behind `enth render` and the reader behind `describe` and `check` on a WAV.
//!
//! `enth render` writes a stereo 32-bit IEEE-float WAV (format 3, an 18-byte `fmt ` chunk and a `fact` chunk) with
//! an `enth` chunk before `data` holding `enth.wavmeta/1` JSON: the performance (`enth.perf/1`), the render seed, the
//! engine version and the patch hash. So `describe` can read decay, sustain and release from a WAV without being
//! told the note-off. The chunk's JSON is padded with spaces to an even length, so no pad byte follows it (hound
//! 3.5.1 skips unknown chunks but not their pad byte). `--bits 16|24` writes integer PCM, with the same chunk.
//!
//! Reading uses hound for the samples (any integer or float format it supports) and a small RIFF scan for the
//! `enth` chunk.

use std::io::Cursor;

use crate::schema::WavMeta;

/// Sample formats `enth render` writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum Bits {
    /// 32-bit IEEE float (the default): the engine's samples exactly, peaks above 0 dBFS kept.
    F32,
    /// 16-bit integer PCM, clipped at full scale.
    #[value(name = "16")]
    Pcm16,
    /// 24-bit integer PCM, clipped at full scale.
    #[value(name = "24")]
    Pcm24,
}

impl Bits {
    /// The name printed in `enth.render/1`'s `format`.
    pub const fn name(self) -> &'static str {
        match self {
            Bits::F32 => "f32",
            Bits::Pcm16 => "pcm16",
            Bits::Pcm24 => "pcm24",
        }
    }

    const fn bits(self) -> u16 {
        match self {
            Bits::F32 => 32,
            Bits::Pcm16 => 16,
            Bits::Pcm24 => 24,
        }
    }
}

fn chunk(out: &mut Vec<u8>, id: [u8; 4], body: &[u8]) {
    out.extend_from_slice(&id);
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(body);
    if body.len() % 2 == 1 {
        out.push(0);
    }
}

/// The bytes of a stereo WAV: `left` and `right` (same length) at `sample_rate`, with `meta` in an `enth` chunk.
pub fn encode(
    left: &[f32],
    right: &[f32],
    sample_rate: u32,
    bits: Bits,
    meta: &WavMeta,
) -> Vec<u8> {
    let channels: u16 = 2;
    let bytes_per_sample = bits.bits() / 8;
    let block_align = channels * bytes_per_sample;
    let frames = left.len().min(right.len());

    let mut fmt = Vec::with_capacity(18);
    let format_tag: u16 = if bits == Bits::F32 { 3 } else { 1 };
    fmt.extend_from_slice(&format_tag.to_le_bytes());
    fmt.extend_from_slice(&channels.to_le_bytes());
    fmt.extend_from_slice(&sample_rate.to_le_bytes());
    fmt.extend_from_slice(&(sample_rate * u32::from(block_align)).to_le_bytes());
    fmt.extend_from_slice(&block_align.to_le_bytes());
    fmt.extend_from_slice(&bits.bits().to_le_bytes());
    if bits == Bits::F32 {
        fmt.extend_from_slice(&0u16.to_le_bytes()); // cbSize: WAVEFORMATEX with no extra bytes
    }

    let mut json = serde_json::to_string(meta).unwrap_or_else(|e| unreachable!("{e}"));
    if json.len() % 2 == 1 {
        json.push(' ');
    }

    let mut data = Vec::with_capacity(frames * usize::from(block_align));
    for (l, r) in left.iter().zip(right).take(frames) {
        for s in [*l, *r] {
            match bits {
                Bits::F32 => data.extend_from_slice(&s.to_le_bytes()),
                Bits::Pcm16 => {
                    let v = (f64::from(s).clamp(-1.0, 1.0) * 32_767.0).round() as i16;
                    data.extend_from_slice(&v.to_le_bytes());
                }
                Bits::Pcm24 => {
                    let v = (f64::from(s).clamp(-1.0, 1.0) * 8_388_607.0).round() as i32;
                    data.extend_from_slice(&v.to_le_bytes()[..3]);
                }
            }
        }
    }

    let mut body = Vec::with_capacity(64 + json.len() + data.len());
    body.extend_from_slice(b"WAVE");
    chunk(&mut body, *b"fmt ", &fmt);
    if bits == Bits::F32 {
        chunk(&mut body, *b"fact", &(frames as u32).to_le_bytes());
    }
    chunk(&mut body, *b"enth", json.as_bytes());
    chunk(&mut body, *b"data", &data);

    let mut out = Vec::with_capacity(8 + body.len());
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(&body);
    out
}

/// A WAV read back: the mid signal `(L + R) / 2` (or the mean of all channels), the rate and the metadata.
#[derive(Clone, Debug, PartialEq)]
pub struct Decoded {
    /// Frames per second.
    pub sample_rate: u32,
    /// Channels in the file.
    pub channels: u16,
    /// The mean of the channels, frame by frame.
    pub mono: Vec<f32>,
    /// The `enth` chunk, if the file has one that parses.
    pub meta: Option<WavMeta>,
}

/// The JSON of the `enth` chunk, if any.
fn find_enth_chunk(bytes: &[u8]) -> Option<&[u8]> {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return None;
    }
    let mut pos = 12usize;
    while pos + 8 <= bytes.len() {
        let id = &bytes[pos..pos + 4];
        let len = u32::from_le_bytes(bytes[pos + 4..pos + 8].try_into().ok()?) as usize;
        let start = pos + 8;
        let end = start.checked_add(len)?;
        if end > bytes.len() {
            return None;
        }
        if id == b"enth" {
            return Some(&bytes[start..end]);
        }
        pos = end + (len % 2);
    }
    None
}

/// Read a WAV. The error text says what was wrong.
pub fn decode(bytes: &[u8]) -> Result<Decoded, String> {
    let reader = hound::WavReader::new(Cursor::new(bytes)).map_err(|e| e.to_string())?;
    let spec = reader.spec();
    let channels = spec.channels.max(1);
    let interleaved: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader
            .into_samples::<f32>()
            .collect::<Result<_, _>>()
            .map_err(|e| e.to_string())?,
        hound::SampleFormat::Int => {
            let scale = 1.0 / f64::from(1u32 << (spec.bits_per_sample.clamp(1, 32) - 1));
            reader
                .into_samples::<i32>()
                .map(|s| s.map(|v| (f64::from(v) * scale) as f32))
                .collect::<Result<_, _>>()
                .map_err(|e| e.to_string())?
        }
    };
    let n = usize::from(channels);
    let mono = interleaved
        .chunks_exact(n)
        .map(|frame| frame.iter().sum::<f32>() / f32::from(channels))
        .collect();
    let meta = find_enth_chunk(bytes)
        .and_then(|j| serde_json::from_slice::<WavMeta>(j.trim_ascii_end()).ok());
    Ok(Decoded {
        sample_rate: spec.sample_rate,
        channels,
        mono,
        meta,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::perf::Perf;
    use crate::schema::ids;

    fn meta() -> WavMeta {
        WavMeta {
            schema: ids::WAVMETA.to_owned(),
            engine: "0.1.0".to_owned(),
            sample_rate: 48_000,
            seed: 7,
            patch_hash: "ab".repeat(32),
            perf: Perf::default_perf().to_file(),
        }
    }

    #[test]
    fn float_wav_round_trips_bit_exact_with_its_chunk() {
        let left: Vec<f32> = (0..1000).map(|i| (i as f32 * 0.01).sin() * 1.5).collect();
        let right: Vec<f32> = left.iter().map(|x| -x).collect();
        let bytes = encode(&left, &right, 48_000, Bits::F32, &meta());
        let d = decode(&bytes).unwrap();
        assert_eq!((d.sample_rate, d.channels), (48_000, 2));
        assert_eq!(d.meta, Some(meta()));
        let mid: Vec<f32> = left
            .iter()
            .zip(&right)
            .map(|(l, r)| (l + r) / 2.0)
            .collect();
        assert_eq!(d.mono, mid);
        let same: Vec<f32> = left.clone();
        let d2 = decode(&encode(&left, &same, 48_000, Bits::F32, &meta())).unwrap();
        assert!(
            d2.mono
                .iter()
                .zip(&left)
                .all(|(a, b)| a.to_bits() == b.to_bits())
        );
    }

    #[test]
    fn pcm_wavs_read_back_close() {
        let x: Vec<f32> = (0..500).map(|i| ((i as f32) * 0.05).sin() * 0.5).collect();
        for (bits, tol) in [(Bits::Pcm16, 1e-4), (Bits::Pcm24, 1e-6)] {
            let d = decode(&encode(&x, &x, 44_100, bits, &meta())).unwrap();
            assert_eq!(d.sample_rate, 44_100);
            assert!(d.meta.is_some());
            for (a, b) in d.mono.iter().zip(&x) {
                assert!((a - b).abs() < tol, "{bits:?}: {a} vs {b}");
            }
        }
    }

    #[test]
    fn a_wav_without_the_chunk_has_no_meta() {
        assert!(find_enth_chunk(b"RIFF\x04\x00\x00\x00WAVE").is_none());
        assert!(decode(b"not a wav").is_err());
    }
}
