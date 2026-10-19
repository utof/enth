//! Small helpers for the agent-eval harness: UTC time without a date crate, FNV hashing, a seeded generator, medians,
//! shell quoting and a minimal WAV reader and writer (for the blind-listen set).

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

/// Seconds since the Unix epoch.
pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Civil date and time (UTC) of `secs` since the epoch, by Howard Hinnant's days-from-civil inverse.
pub fn civil(secs: u64) -> (i64, u32, u32, u32, u32, u32) {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = if month <= 2 { year + 1 } else { year };
    (
        year,
        month,
        day,
        (rem / 3600) as u32,
        (rem % 3600 / 60) as u32,
        (rem % 60) as u32,
    )
}

/// `2026-10-19T17:04:05Z`.
pub fn iso8601(secs: u64) -> String {
    let (y, mo, d, h, mi, s) = civil(secs);
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z")
}

/// `20261019T170405Z`, for directory names.
pub fn stamp(secs: u64) -> String {
    let (y, mo, d, h, mi, s) = civil(secs);
    format!("{y:04}{mo:02}{d:02}T{h:02}{mi:02}{s:02}Z")
}

/// `2026-10-19`.
pub fn date(secs: u64) -> String {
    let (y, mo, d, ..) = civil(secs);
    format!("{y:04}-{mo:02}-{d:02}")
}

/// FNV-1a, 64 bit: a content fingerprint for prompt files (not a security hash).
pub fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325_u64;
    for &b in bytes {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

/// `SplitMix64`: a seeded generator for the blind-listen picks and shuffle.
#[derive(Clone, Debug)]
pub struct Rng(pub u64);

impl Rng {
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// A value in `0..n` (`n > 0`).
    pub fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }

    /// Fisher-Yates.
    pub fn shuffle<T>(&mut self, items: &mut [T]) {
        for i in (1..items.len()).rev() {
            items.swap(i, self.below(i + 1));
        }
    }
}

/// The median of `values` (the mean of the two middle ones for an even count); `None` when empty.
pub fn median(values: &[f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let mut v = values.to_vec();
    v.sort_by(f64::total_cmp);
    let n = v.len();
    Some(if n % 2 == 1 {
        v[n / 2]
    } else {
        f64::midpoint(v[n / 2 - 1], v[n / 2])
    })
}

/// Round to 4 significant digits, like the CLI's own numbers (keeps result files diff-friendly).
pub fn round4(x: f64) -> f64 {
    if x == 0.0 || !x.is_finite() {
        return x;
    }
    let scale = 10f64.powi(3 - x.abs().log10().floor() as i32);
    (x * scale).round() / scale
}

/// A string safe inside single quotes in `sh`.
pub fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// A decoded WAV: interleaved samples as f32.
#[derive(Clone, Debug)]
pub struct Wav {
    pub sample_rate: u32,
    pub channels: u16,
    pub samples: Vec<f32>,
}

fn le16(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

fn le32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

/// The `fmt ` chunk: format tag, channels, sample rate, bits per sample.
type Fmt = (u16, u16, u32, u16);

/// Decode a RIFF/WAVE file: 32-bit float or 16/24-bit integer PCM. Other chunks (the `enth` chunk) are skipped.
pub fn parse_wav(bytes: &[u8]) -> Result<Wav, String> {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err("not a RIFF/WAVE file".to_owned());
    }
    let (mut fmt, mut data): (Option<Fmt>, Option<&[u8]>) = (None, None);
    let mut at = 12;
    while at + 8 <= bytes.len() {
        let id = &bytes[at..at + 4];
        let len = le32(bytes, at + 4) as usize;
        let body = at + 8;
        let end = (body + len).min(bytes.len());
        match id {
            b"fmt " if end - body >= 16 => {
                fmt = Some((
                    le16(bytes, body),
                    le16(bytes, body + 2),
                    le32(bytes, body + 4),
                    le16(bytes, body + 14),
                ));
            }
            b"data" => data = Some(&bytes[body..end]),
            _ => {}
        }
        at = body + len + (len & 1);
    }
    let (format, channels, sample_rate, bits) = fmt.ok_or("no fmt chunk")?;
    let data = data.ok_or("no data chunk")?;
    if channels == 0 {
        return Err("zero channels".to_owned());
    }
    let samples: Vec<f32> = match (format, bits) {
        (3, 32) => data
            .as_chunks::<4>()
            .0
            .iter()
            .map(|c| f32::from_le_bytes(*c))
            .collect(),
        (1, 16) => data
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| f32::from(i16::from_le_bytes(*c)) / 32768.0)
            .collect(),
        (1, 24) => data
            .as_chunks::<3>()
            .0
            .iter()
            .map(|c| (i32::from_le_bytes([0, c[0], c[1], c[2]]) >> 8) as f32 / 8_388_608.0)
            .collect(),
        _ => return Err(format!("unsupported WAV format {format} with {bits} bits")),
    };
    Ok(Wav {
        sample_rate,
        channels,
        samples,
    })
}

/// A 32-bit float WAV, no extra chunks.
pub fn wav_bytes(wav: &Wav) -> Vec<u8> {
    let data_len = (wav.samples.len() * 4) as u32;
    let block = u32::from(wav.channels) * 4;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&3u16.to_le_bytes());
    out.extend_from_slice(&wav.channels.to_le_bytes());
    out.extend_from_slice(&wav.sample_rate.to_le_bytes());
    out.extend_from_slice(&(wav.sample_rate * block).to_le_bytes());
    out.extend_from_slice(&(block as u16).to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for s in &wav.samples {
        out.extend_from_slice(&s.to_le_bytes());
    }
    out
}

/// Whether the file starts with `RIFF` (a WAV, so analysing it is not a render).
pub fn is_riff(path: &Path) -> bool {
    use std::io::Read;
    let mut magic = [0u8; 4];
    std::fs::File::open(path)
        .and_then(|mut f| f.read_exact(&mut magic))
        .is_ok()
        && &magic == b"RIFF"
}

/// Write `value` as pretty JSON with a trailing newline.
pub fn write_json(path: &Path, value: &serde_json::Value) -> Result<(), String> {
    let mut text = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    text.push('\n');
    std::fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_dates_match_known_epochs() {
        assert_eq!(iso8601(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso8601(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(iso8601(1_792_393_200), "2026-10-19T07:00:00Z");
        assert_eq!(date(1_792_393_200), "2026-10-19");
        assert_eq!(stamp(1_792_393_200), "20261019T070000Z");
    }

    #[test]
    fn median_handles_even_odd_and_empty() {
        assert_eq!(median(&[]), None);
        assert_eq!(median(&[3.0, 1.0, 2.0]), Some(2.0));
        assert_eq!(median(&[4.0, 1.0, 2.0, 3.0]), Some(2.5));
    }

    #[test]
    fn rounding_keeps_four_significant_digits() {
        assert_eq!(round4(0.123_456), 0.1235);
        assert_eq!(round4(12_345.6), 12_350.0);
        assert_eq!(round4(0.0), 0.0);
    }

    #[test]
    fn rng_is_seeded_and_shuffle_keeps_items() {
        let mut a = Rng(7);
        let mut b = Rng(7);
        assert_eq!(a.next_u64(), b.next_u64());
        let mut v: Vec<u32> = (0..10).collect();
        Rng(1).shuffle(&mut v);
        let mut sorted = v.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, (0..10).collect::<Vec<_>>());
    }

    #[test]
    fn wav_round_trips_and_skips_unknown_chunks() {
        let wav = Wav {
            sample_rate: 48_000,
            channels: 2,
            samples: vec![0.0, 0.5, -0.25, 1.0],
        };
        let mut bytes = wav_bytes(&wav);
        // splice an unknown chunk between fmt and data
        let extra = [b'e', b'n', b't', b'h', 3, 0, 0, 0, 1, 2, 3, 0];
        bytes.splice(36..36, extra);
        let back = parse_wav(&bytes).expect("parses");
        assert_eq!(back.sample_rate, 48_000);
        assert_eq!(back.channels, 2);
        assert_eq!(back.samples, wav.samples);
        assert!(parse_wav(b"nope").is_err());
    }

    #[test]
    fn shell_quoting_survives_single_quotes() {
        assert_eq!(sh_quote("a'b"), r"'a'\''b'");
    }
}
