//! A 32-bit float WAV writer (RIFF, format tag 3, with the `fact` chunk the spec asks for).

use std::io::Write;
use std::path::Path;

/// Encodes interleaved `f32` samples as a WAV file in memory.
pub fn encode_f32(sample_rate: u32, channels: u16, interleaved: &[f32]) -> Vec<u8> {
    let data_len = (interleaved.len() * 4) as u32;
    let frames = interleaved.len() / usize::from(channels.max(1));
    let block_align = channels * 4;
    let mut out = Vec::with_capacity(48 + interleaved.len() * 4);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(4 + 24 + 12 + 8 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&3u16.to_le_bytes()); // IEEE float
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&(sample_rate * u32::from(block_align)).to_le_bytes());
    out.extend_from_slice(&block_align.to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
    out.extend_from_slice(b"fact");
    out.extend_from_slice(&4u32.to_le_bytes());
    out.extend_from_slice(&(frames as u32).to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for s in interleaved {
        out.extend_from_slice(&s.to_le_bytes());
    }
    out
}

pub fn write_f32(
    path: &Path,
    sample_rate: u32,
    channels: u16,
    interleaved: &[f32],
) -> Result<(), String> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let mut file = std::fs::File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
    file.write_all(&encode_f32(sample_rate, channels, interleaved))
        .map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_and_data_are_consistent() {
        let wav = encode_f32(48_000, 2, &[0.0, 0.5, -0.5, 1.0]);
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        let riff_len = u32::from_le_bytes(wav[4..8].try_into().unwrap()) as usize;
        assert_eq!(riff_len + 8, wav.len());
        assert_eq!(u16::from_le_bytes(wav[20..22].try_into().unwrap()), 3);
        assert_eq!(u16::from_le_bytes(wav[22..24].try_into().unwrap()), 2);
        assert_eq!(u32::from_le_bytes(wav[24..28].try_into().unwrap()), 48_000);
        assert_eq!(&wav[36..40], b"fact");
        assert_eq!(u32::from_le_bytes(wav[44..48].try_into().unwrap()), 2); // 2 frames
        assert_eq!(&wav[48..52], b"data");
        assert_eq!(u32::from_le_bytes(wav[52..56].try_into().unwrap()), 16);
        assert_eq!(f32::from_le_bytes(wav[60..64].try_into().unwrap()), 0.5);
    }
}
