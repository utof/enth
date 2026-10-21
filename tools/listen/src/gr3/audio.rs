//! Reading the candidates' WAV files, and the level rule of a trial.
//!
//! **Reading.** `enth render` writes 32-bit float WAVs (with an extra chunk), real references are usually 16- or 24-bit
//! PCM, so [`decode_any`] reads PCM at 16, 24 and 32 bits, float at 32 and 64 bits, and the extensible header, mono or
//! stereo, and skips every chunk it does not need. A mono file becomes stereo by copying the channel (a lossless
//! change that the loudness meter then sees as it will be heard). No resampling: the three clips of a trial must share
//! one sample rate (render and analysis run at the file's own rate, docs/r3.md), and a trial that does not is an error
//! that names the files.
//!
//! **Levels.** [`TrialLevels::plan`] takes the loudness and true peak of the trial's three clips (R, A and B, each
//! measured once per file) and returns one gain per clip: every clip to [`TARGET_LUFS`] (-23 LUFS, BS.1770 through
//! the same `ebur128` pin as `enth-analysis`), then, if any clip's true peak would pass [`TRUE_PEAK_CEILING_DBTP`]
//! (-1 dBTP), all three lowered by the same amount, so the trial's levels stay matched and only its overall level drops.

use ebur128::{EbuR128, Mode};

use crate::loudness::{StereoMeter, TARGET_LUFS};
use crate::wav::Wav;

/// No clip of a trial may pass this true peak (dBTP) after matching; the trial is lowered as a whole instead.
pub const TRUE_PEAK_CEILING_DBTP: f64 = -1.0;

/// Decode PCM (16, 24 or 32 bit) or IEEE float (32 or 64 bit) WAV, mono or stereo, with the extensible header allowed.
/// A mono file is returned as stereo.
///
/// # Errors
/// Anything else: not a RIFF/WAVE file, a codec or channel count this reader does not handle, or a truncated file.
pub fn decode_any(bytes: &[u8]) -> Result<Wav, String> {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err("not a RIFF/WAVE file".into());
    }
    let u16_at = |b: &[u8], o: usize| u16::from_le_bytes([b[o], b[o + 1]]);
    let u32_at = |b: &[u8], o: usize| u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]);
    let mut pos = 12;
    let mut fmt: Option<(usize, u32, usize)> = None; // channels, sample rate, bits per sample
    let mut float = false;
    while pos + 8 <= bytes.len() {
        let id = &bytes[pos..pos + 4];
        let len = u32_at(bytes, pos + 4) as usize;
        let end = (pos + 8).saturating_add(len);
        // a data chunk written before its length was patched (a streaming writer) may claim more than the file holds
        let body = &bytes[pos + 8..end.min(bytes.len())];
        if id == b"fmt " {
            if body.len() < 16 {
                return Err("the fmt chunk is too short".into());
            }
            let mut tag = u16_at(body, 0);
            if tag == 0xFFFE {
                if body.len() < 26 {
                    return Err("the extensible fmt chunk is too short".into());
                }
                tag = u16_at(body, 24);
            }
            float = match tag {
                1 => false,
                3 => true,
                other => return Err(format!("WAV format tag {other} (only PCM and IEEE float)")),
            };
            fmt = Some((
                usize::from(u16_at(body, 2)),
                u32_at(body, 4),
                usize::from(u16_at(body, 14)),
            ));
        } else if id == b"data" {
            let (ch, rate, bits) = fmt.ok_or("the data chunk comes before the fmt chunk")?;
            if !(1..=2).contains(&ch) {
                return Err(format!("{ch} channels (only mono and stereo)"));
            }
            let bytes_per = match (float, bits) {
                (false, 16) => 2,
                (false, 24) => 3,
                (false | true, 32) => 4,
                (true, 64) => 8,
                _ => {
                    return Err(format!(
                        "{bits}-bit {} samples (supported: PCM 16, 24 and 32, float 32 and 64)",
                        if float { "float" } else { "PCM" }
                    ));
                }
            };
            let frames = body.len() / (bytes_per * ch);
            let mut chans = vec![Vec::with_capacity(frames); ch];
            for f in 0..frames {
                for (c, chan) in chans.iter_mut().enumerate() {
                    let o = (f * ch + c) * bytes_per;
                    let s = &body[o..o + bytes_per];
                    let v = match (float, bytes_per) {
                        (false, 2) => f32::from(i16::from_le_bytes([s[0], s[1]])) / 32_768.0,
                        (false, 3) => {
                            (f64::from(i32::from_le_bytes([0, s[0], s[1], s[2]]) >> 8)
                                / 8_388_608.0) as f32
                        }
                        (false, _) => {
                            (f64::from(i32::from_le_bytes([s[0], s[1], s[2], s[3]]))
                                / 2_147_483_648.0) as f32
                        }
                        (true, 4) => f32::from_le_bytes([s[0], s[1], s[2], s[3]]),
                        (true, _) => {
                            f64::from_le_bytes([s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7]])
                                as f32
                        }
                    };
                    chan.push(v);
                }
            }
            if frames == 0 {
                return Err("the file holds no samples".into());
            }
            let mut it = chans.into_iter();
            let left = it.next().expect("at least one channel");
            let right = it.next().unwrap_or_else(|| left.clone());
            if left.iter().chain(&right).any(|v| !v.is_finite()) {
                return Err("the file holds NaN or infinite samples".into());
            }
            return Ok(Wav::stereo(rate, left, right));
        }
        pos = end.saturating_add(len & 1);
    }
    Err("no data chunk".into())
}

/// Loudness and true peak of one clip, measured on the stereo signal as it will be heard.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ClipLevel {
    /// Integrated loudness (LUFS, BS.1770 with the ungated fallback for very short sounds).
    pub lufs: f64,
    /// True peak (dBTP, 4x oversampled below 96 kHz).
    pub true_peak_dbtp: f64,
}

/// The true peak of a stereo clip in dBTP (-150 for silence).
///
/// # Errors
/// `ebur128` rejects the sample rate.
pub fn true_peak_dbtp(w: &Wav) -> Result<f64, String> {
    let mut e = EbuR128::new(2, w.sample_rate, Mode::TRUE_PEAK)
        .map_err(|err| format!("{} Hz: {err}", w.sample_rate))?;
    e.add_frames_planar_f32(&[&w.channels[0], &w.channels[1]])
        .map_err(|err| err.to_string())?;
    let tp = (0..2).fold(0.0f64, |m, c| m.max(e.true_peak(c).unwrap_or(0.0)));
    Ok(if tp > 0.0 {
        20.0 * libm::log10(tp)
    } else {
        -150.0
    })
}

/// Measure a clip.
///
/// # Errors
/// The clip is silent (below -70 LUFS), so it cannot be matched, or the sample rate is not accepted.
pub fn measure(w: &Wav) -> Result<ClipLevel, String> {
    let lufs = StereoMeter::new(w.sample_rate)
        .integrated(&w.channels[0], &w.channels[1])
        .ok_or("the clip is silent (below -70 LUFS), so it cannot be level-matched")?;
    Ok(ClipLevel {
        lufs,
        true_peak_dbtp: true_peak_dbtp(w)?,
    })
}

/// The gains of one trial's three clips (reference, A, B), in dB.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TrialLevels {
    /// Gain applied to each clip, dB: `TARGET_LUFS - lufs - lowered_db`.
    pub gains_db: [f64; 3],
    /// How far the whole trial was lowered to keep every clip under the true-peak ceiling (0 when none was over).
    pub lowered_db: f64,
    /// True peak of each clip after the gains, dBTP.
    pub true_peak_out_dbtp: [f64; 3],
}

impl TrialLevels {
    /// Plan the gains of a trial from its clips' measured levels.
    #[must_use]
    pub fn plan(levels: [ClipLevel; 3]) -> Self {
        let matched = levels.map(|l| TARGET_LUFS - l.lufs);
        let worst = (0..3)
            .map(|i| levels[i].true_peak_dbtp + matched[i])
            .fold(f64::NEG_INFINITY, f64::max);
        let lowered = (worst - TRUE_PEAK_CEILING_DBTP).max(0.0);
        let gains_db = matched.map(|g| g - lowered);
        let true_peak_out_dbtp = [0, 1, 2].map(|i| levels[i].true_peak_dbtp + gains_db[i]);
        Self {
            gains_db,
            lowered_db: lowered,
            true_peak_out_dbtp,
        }
    }
}

/// `w` scaled by `gain_db`.
#[must_use]
pub fn scaled(w: &Wav, gain_db: f64) -> Wav {
    let mut out = w.clone();
    out.scale(crate::loudness::db_to_gain(gain_db) as f32);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wav::encode;
    use enth_analysis::test_signals::{SR, sine};

    fn riff(
        fmt_tag: u16,
        ch: u16,
        rate: u32,
        bits: u16,
        data: &[u8],
        extra_chunk: bool,
    ) -> Vec<u8> {
        let mut fmt = Vec::new();
        fmt.extend_from_slice(&fmt_tag.to_le_bytes());
        fmt.extend_from_slice(&ch.to_le_bytes());
        fmt.extend_from_slice(&rate.to_le_bytes());
        fmt.extend_from_slice(&(rate * u32::from(ch) * u32::from(bits / 8)).to_le_bytes());
        fmt.extend_from_slice(&(ch * bits / 8).to_le_bytes());
        fmt.extend_from_slice(&bits.to_le_bytes());
        let mut body = Vec::new();
        body.extend_from_slice(b"fmt ");
        body.extend_from_slice(&(fmt.len() as u32).to_le_bytes());
        body.extend_from_slice(&fmt);
        if extra_chunk {
            body.extend_from_slice(b"enth");
            body.extend_from_slice(&3u32.to_le_bytes());
            body.extend_from_slice(b"abc\0"); // 3 bytes and a pad byte
        }
        body.extend_from_slice(b"data");
        body.extend_from_slice(&(data.len() as u32).to_le_bytes());
        body.extend_from_slice(data);
        let mut out = b"RIFF".to_vec();
        out.extend_from_slice(&(4 + body.len() as u32).to_le_bytes());
        out.extend_from_slice(b"WAVE");
        out.extend_from_slice(&body);
        out
    }

    #[test]
    fn reads_float32_with_an_extra_chunk_and_makes_mono_stereo() {
        let samples = [0.5f32, -0.25, 1.5, 0.0];
        let data: Vec<u8> = samples.iter().flat_map(|v| v.to_le_bytes()).collect();
        let w = decode_any(&riff(3, 1, 44_100, 32, &data, true)).unwrap();
        assert_eq!(w.sample_rate, 44_100);
        assert_eq!(w.channels.len(), 2);
        assert_eq!(w.channels[0], samples);
        assert_eq!(w.channels[1], samples);
    }

    #[test]
    fn reads_pcm16_pcm24_and_pcm32() {
        let d16: Vec<u8> = [16_384i16, -16_384]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        let w = decode_any(&riff(1, 2, 48_000, 16, &d16, false)).unwrap();
        assert_eq!((w.channels[0][0], w.channels[1][0]), (0.5, -0.5));
        // the 24-bit reader agrees with this crate's own writer
        let own = Wav::stereo(48_000, vec![0.25, -0.5], vec![0.125, 0.75]);
        let back = decode_any(&encode(&own)).unwrap();
        for c in 0..2 {
            for i in 0..2 {
                assert!((back.channels[c][i] - own.channels[c][i]).abs() < 2e-7);
            }
        }
        let d32: Vec<u8> = [1i32 << 30, -(1 << 30)]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        let w = decode_any(&riff(1, 2, 48_000, 32, &d32, false)).unwrap();
        assert_eq!((w.channels[0][0], w.channels[1][0]), (0.5, -0.5));
    }

    #[test]
    fn refuses_what_it_cannot_read_with_a_reason() {
        assert!(decode_any(b"not a wav").unwrap_err().contains("RIFF"));
        let e = decode_any(&riff(1, 1, 48_000, 8, &[0, 0], false)).unwrap_err();
        assert!(e.contains("8-bit"), "{e}");
        let e = decode_any(&riff(2, 1, 48_000, 4, &[0, 0], false)).unwrap_err();
        assert!(e.contains("format tag 2"), "{e}");
        let e = decode_any(&riff(1, 6, 48_000, 16, &[0; 12], false)).unwrap_err();
        assert!(e.contains("6 channels"), "{e}");
        let nan: Vec<u8> = [f32::NAN, 0.0]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        assert!(
            decode_any(&riff(3, 1, 48_000, 32, &nan, false))
                .unwrap_err()
                .contains("NaN")
        );
    }

    #[test]
    fn true_peak_sees_the_peak_between_samples() {
        // A sine at a quarter of the sample rate, 45 degrees off the sample grid: every sample is 0.7071 of the peak,
        // the waveform between them reaches 1.0.
        let n = 4_800;
        let x: Vec<f32> = (0..n)
            .map(|i| {
                let ph = core::f64::consts::FRAC_PI_2 * f64::from(i) + core::f64::consts::FRAC_PI_4;
                libm::sin(ph) as f32
            })
            .collect();
        let w = Wav::stereo(48_000, x.clone(), x);
        let sample_peak = 20.0 * libm::log10(f64::from(w.peak()));
        let tp = true_peak_dbtp(&w).unwrap();
        assert!(
            (sample_peak + 3.01).abs() < 0.05,
            "sample peak {sample_peak}"
        );
        assert!(tp > -0.2 && tp < 0.3, "true peak {tp}");
    }

    fn level(lufs: f64, tp: f64) -> ClipLevel {
        ClipLevel {
            lufs,
            true_peak_dbtp: tp,
        }
    }

    #[test]
    fn every_clip_is_matched_to_the_target() {
        let t = TrialLevels::plan([level(-30.0, -12.0), level(-20.0, -10.0), level(-23.0, -9.0)]);
        assert_eq!(t.lowered_db, 0.0);
        assert!((t.gains_db[0] - 7.0).abs() < 1e-12);
        assert!((t.gains_db[1] + 3.0).abs() < 1e-12);
        assert!(t.gains_db[2].abs() < 1e-12);
        assert!((t.true_peak_out_dbtp[0] + 5.0).abs() < 1e-12);
    }

    #[test]
    fn a_clip_over_the_ceiling_lowers_the_whole_trial() {
        // clip A would end at +1.5 dBTP after matching: all three drop by 2.5 dB, so the ceiling is met exactly
        let t = TrialLevels::plan([level(-23.0, -10.0), level(-23.0, 1.5), level(-23.0, -6.0)]);
        assert!((t.lowered_db - 2.5).abs() < 1e-12, "{t:?}");
        assert!((t.true_peak_out_dbtp[1] + 1.0).abs() < 1e-12);
        // the matched levels stay equal to each other
        assert!((t.gains_db[0] - t.gains_db[1]).abs() < 1e-12);
        assert!((t.gains_db[1] - t.gains_db[2]).abs() < 1e-12);
        // exactly at the ceiling is not over it
        let ok = TrialLevels::plan([level(-23.0, -1.0), level(-23.0, -3.0), level(-23.0, -3.0)]);
        assert_eq!(ok.lowered_db, 0.0);
    }

    #[test]
    fn measure_and_scale_round_trip_to_the_target() {
        let x = sine(SR, 997.0, 0.1, 3.0);
        let w = Wav::stereo(48_000, x.clone(), x);
        let l = measure(&w).unwrap();
        let t = TrialLevels::plan([l, l, l]);
        let out = scaled(&w, t.gains_db[0]);
        let after = measure(&out).unwrap();
        assert!((after.lufs - TARGET_LUFS).abs() < 0.01, "{after:?}");
        assert!(measure(&Wav::stereo(48_000, vec![0.0; 4_800], vec![0.0; 4_800])).is_err());
    }
}
