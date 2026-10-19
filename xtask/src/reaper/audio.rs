//! Reading REAPER's offline renders and measuring them. The measures are directional (louder, brighter, unchanged),
//! because REAPER's offline render restarts the plugin shortly after it starts, so the exact samples are not a golden
//! (plugin goldens come from the test host, `AGENTS.md`).

pub struct Wav {
    pub rate: u32,
    pub left: Vec<f32>,
    pub right: Vec<f32>,
}

impl Wav {
    pub fn read(path: &std::path::Path) -> Result<Wav, String> {
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        read_wav(&bytes)
    }

    /// The left channel between two times in seconds.
    pub fn window(&self, from: f64, to: f64) -> &[f32] {
        let rate = f64::from(self.rate);
        let end = ((to * rate) as usize).min(self.left.len());
        let start = ((from * rate) as usize).min(end);
        &self.left[start..end]
    }

    pub fn seconds(&self) -> f64 {
        self.left.len() as f64 / f64::from(self.rate)
    }

    /// Every sample of both channels is finite.
    pub fn finite(&self) -> bool {
        self.left.iter().chain(&self.right).all(|s| s.is_finite())
    }
}

pub fn read_wav(bytes: &[u8]) -> Result<Wav, String> {
    if bytes.len() < 12 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err("not a WAV file".into());
    }
    let le16 = |b: &[u8]| usize::from(u16::from_le_bytes([b[0], b[1]]));
    let le32 = |b: &[u8]| u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
    let (mut format, mut channels, mut rate, mut bits) = (0, 0, 0, 0);
    let mut data: Option<&[u8]> = None;
    let mut at = 12;
    while at + 8 <= bytes.len() {
        let size = le32(&bytes[at + 4..]) as usize;
        let body = &bytes[at + 8..(at + 8 + size).min(bytes.len())];
        match &bytes[at..at + 4] {
            b"fmt " if body.len() >= 16 => {
                (format, channels, rate, bits) = (
                    le16(body),
                    le16(&body[2..]),
                    le32(&body[4..]),
                    le16(&body[14..]),
                );
            }
            b"data" => data = Some(body),
            _ => {}
        }
        at += 8 + size + (size & 1);
    }
    let data = data.ok_or("no data chunk")?;
    let step = bits / 8 * channels;
    if channels == 0 || step == 0 {
        return Err("no fmt chunk".into());
    }
    let sample = |b: &[u8]| -> Result<f32, String> {
        Ok(match (format, bits) {
            (1, 16) => f32::from(i16::from_le_bytes([b[0], b[1]])) / 32768.0,
            (1, 24) => (i32::from_le_bytes([0, b[0], b[1], b[2]]) >> 8) as f32 / 8_388_608.0,
            (1, 32) => i32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f32 / 2.0_f32.powi(31),
            (3, 32) => f32::from_le_bytes([b[0], b[1], b[2], b[3]]),
            _ => return Err(format!("unsupported WAV format {format} with {bits} bits")),
        })
    };
    let width = bits / 8;
    let (mut left, mut right) = (Vec::new(), Vec::new());
    for frame in data.chunks_exact(step) {
        left.push(sample(&frame[..width])?);
        right.push(sample(&frame[(channels - 1) * width..channels * width])?);
    }
    Ok(Wav { rate, left, right })
}

pub fn rms(x: &[f32]) -> f64 {
    (x.iter().map(|&s| f64::from(s) * f64::from(s)).sum::<f64>() / x.len().max(1) as f64).sqrt()
}

/// RMS of the first difference relative to the RMS: grows with high-frequency content.
pub fn roughness(x: &[f32]) -> f64 {
    let diff: Vec<f32> = x.windows(2).map(|w| w[1] - w[0]).collect();
    let level = rms(x);
    if level == 0.0 {
        0.0
    } else {
        rms(&diff) / level
    }
}

/// The measures of every note of a render, in order: the scenarios play notes of 0.11 s every 0.125 s from time 0, and a
/// note is measured over its body (0.01 to 0.10 s after its start: past the attack, before the release). Comparing
/// medians of a few notes of one pitch, not windows of seconds, keeps one note that REAPER's offline render dropped (it restarts the
/// plugin shortly after a render starts, and under load at other times too) from failing a scenario.
pub struct Notes {
    pub rms: Vec<f64>,
    pub roughness: Vec<f64>,
}

/// The note period and body of the scenarios' MIDI (see `render.lua`).
const NOTE_PERIOD: f64 = 0.125;
const BODY: (f64, f64) = (0.01, 0.10);

impl Notes {
    pub fn measure(wav: &Wav, count: usize) -> Notes {
        let body = |i: usize| {
            let start = NOTE_PERIOD * i as f64;
            wav.window(start + BODY.0, start + BODY.1)
        };
        Notes {
            rms: (0..count).map(|i| rms(body(i))).collect(),
            roughness: (0..count).map(|i| roughness(body(i))).collect(),
        }
    }
}

/// The median of the values at the indices `at` (the mean of the middle two for an even count).
pub fn median_at(values: &[f64], at: &[usize]) -> f64 {
    let mut part: Vec<f64> = at.iter().filter_map(|&i| values.get(i).copied()).collect();
    part.sort_by(f64::total_cmp);
    match part.len() {
        0 => 0.0,
        n if n % 2 == 1 => part[n / 2],
        n => f64::midpoint(part[n / 2 - 1], part[n / 2]),
    }
}

/// How the notes of two renders compare from note `from` on: the median relative difference of their levels and the
/// number of notes that differ by more than `tolerance` (a dropped note is one).
pub fn compare_levels(a: &Notes, b: &Notes, from: usize, tolerance: f64) -> (f64, usize) {
    let relative: Vec<f64> = a
        .rms
        .iter()
        .zip(&b.rms)
        .skip(from)
        .map(|(x, y)| (x - y).abs() / x.max(*y).max(1e-9))
        .collect();
    let outliers = relative.iter().filter(|r| **r > tolerance).count();
    let all: Vec<usize> = (0..relative.len()).collect();
    (median_at(&relative, &all), outliers)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wav16(frames: &[(i16, i16)], rate: u32) -> Vec<u8> {
        let data: Vec<u8> = frames
            .iter()
            .flat_map(|(l, r)| l.to_le_bytes().into_iter().chain(r.to_le_bytes()))
            .collect();
        let mut out = b"RIFF".to_vec();
        out.extend((36 + data.len() as u32).to_le_bytes());
        out.extend(b"WAVEfmt ");
        out.extend(16u32.to_le_bytes());
        out.extend(1u16.to_le_bytes()); // PCM
        out.extend(2u16.to_le_bytes());
        out.extend(rate.to_le_bytes());
        out.extend((rate * 4).to_le_bytes());
        out.extend(4u16.to_le_bytes());
        out.extend(16u16.to_le_bytes());
        out.extend(b"data");
        out.extend((data.len() as u32).to_le_bytes());
        out.extend(data);
        out
    }

    #[test]
    fn reads_both_channels_and_windows_clamp() {
        let wav = read_wav(&wav16(&[(16384, -16384), (0, 8192)], 4)).unwrap();
        assert_eq!(wav.rate, 4);
        assert_eq!(wav.left, vec![0.5, 0.0]);
        assert_eq!(wav.right, vec![-0.5, 0.25]);
        assert_eq!(wav.window(0.0, 100.0).len(), 2);
        assert_eq!(wav.window(50.0, 60.0).len(), 0);
        assert!((wav.seconds() - 0.5).abs() < 1e-12);
    }

    #[test]
    fn medians_ignore_one_outlier() {
        let values = [1.0, 1.1, 0.0, 1.05, 1.0];
        assert!((median_at(&values, &[0, 1, 2, 3, 4]) - 1.0).abs() < 1e-12);
        assert!((median_at(&values, &[1, 2]) - 0.55).abs() < 1e-12);
        assert_eq!(median_at(&values, &[]), 0.0);
        assert_eq!(median_at(&values, &[9]), 0.0);
    }

    #[test]
    fn notes_are_measured_over_their_bodies_and_compared_by_level() {
        // Two notes a period apart, the second twice as loud; each note is a square wave over its body.
        let rate = 48_000_usize;
        let mut left = vec![0.0_f32; rate / 4];
        for (i, level) in [(0usize, 0.25_f32), (1, 0.5)] {
            let start = (NOTE_PERIOD * i as f64 * rate as f64) as usize;
            for n in 0..(0.11 * rate as f64) as usize {
                left[start + n] = if n % 20 < 10 { level } else { -level };
            }
        }
        let wav = Wav {
            rate: rate as u32,
            right: left.clone(),
            left,
        };
        let notes = Notes::measure(&wav, 2);
        assert!((notes.rms[0] - 0.25).abs() < 0.01 && (notes.rms[1] - 0.5).abs() < 0.01);
        let (median_difference, outliers) = compare_levels(&notes, &notes, 0, 0.05);
        assert_eq!((median_difference, outliers), (0.0, 0));
        let quiet = Notes {
            rms: vec![0.25, 0.25],
            roughness: vec![0.0; 2],
        };
        assert_eq!(compare_levels(&notes, &quiet, 0, 0.05).1, 1);
    }

    #[test]
    fn roughness_grows_with_high_frequency_and_ignores_level() {
        let slow: Vec<f32> = (0..400).map(|i| (i as f32 * 0.05).sin()).collect();
        let fast: Vec<f32> = (0..400).map(|i| (i as f32 * 0.8).sin()).collect();
        let quiet: Vec<f32> = slow.iter().map(|s| s * 0.1).collect();
        assert!(roughness(&fast) > 5.0 * roughness(&slow));
        assert!((roughness(&quiet) - roughness(&slow)).abs() < 1e-6);
        assert_eq!(roughness(&[0.0; 8]), 0.0);
    }
}
