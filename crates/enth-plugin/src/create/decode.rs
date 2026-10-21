//! **Owner: lane SK3.** The audio decoders of "create from sound" (docs/r3.md, "File input"), shared by the plugin's
//! `create` feature and `enth create`: the CLI includes this file by path (`#[path]`), so both shells decode a file to
//! the same bits. L8 only: core crates do no I/O.
//!
//! | Format | Crate | Exact? |
//! | --- | --- | --- |
//! | WAV (8–32-bit integer, 32-bit float; `RIFF`/`WAVE`) | `hound` | yes: integers scaled by a power of two |
//! | FLAC | `claxon` | yes: integers scaled by a power of two |
//! | Ogg Vorbis | `lewton` | the same bits on one machine; across platforms unverified (its window and twiddle tables use std's `sin`/`cos`/`exp`) |
//! | MP3 (and MP2, MP1) | `nanomp3`, a port of the CC0 minimp3 | the same bits on one machine; across platforms unverified (a fixed evaluation order and tables, no std transcendentals) |
//!
//! `symphonia` (M4A/AAC, ALAC) is MPL-2.0 and waits for the owner's OK; AIFF has no reader yet. A result records the
//! decoded clip's hash (`enth.create/1`'s `input.audio_sha256`), so a replay can check its clip either way.
//!
//! - **Bounded:** at most [`MAX_DECODE_S`] seconds are read (the intake looks at 30 s from a second before the first
//!   sound), and `stop` is polled between chunks, so a cancel or an unload never waits on a long file.
//! - **192 kHz and above:** the analysis runs at 8,000 to 191,999 Hz (`enth_analysis::intake::SAMPLE_RATES`). A file
//!   at 192 kHz or more is **decimated exactly 2:1** (again while it is still 192 kHz or more): a fixed 63-tap
//!   half-band FIR (flat to 40 kHz at 192 kHz, −61 dB at 56 kHz, below −100 dB from 60 kHz), summed in `f64` in a
//!   fixed order, so the same file gives the same bits everywhere. Other rates outside the range are refused with a
//!   hint that says so.
//! - **The codec hint:** each decode reports a [`CodecHint`] (lossy or not, and the bitrate when the stream says),
//!   which the shell passes to the intake (`IntakeCfg::codec`): a lossy file's cut-off is found on sparser spectra.

use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;

use enth_analysis::intake::{Codec, CodecHint, SAMPLE_RATES};

/// The most seconds of a file that are decoded.
pub const MAX_DECODE_S: f64 = 60.0;
/// How many interleaved samples are decoded between two polls of `stop`.
const POLL: usize = 1 << 15;

/// A file's container and codec.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    /// RIFF WAVE.
    Wav,
    /// FLAC.
    Flac,
    /// Ogg Vorbis.
    Vorbis,
    /// MPEG audio.
    Mp3,
}

impl Format {
    /// Its name, for the view and the record.
    pub fn name(self) -> &'static str {
        match self {
            Format::Wav => "WAV",
            Format::Flac => "FLAC",
            Format::Vorbis => "Ogg Vorbis",
            Format::Mp3 => "MP3",
        }
    }
}

/// A decoded file: interleaved samples in −1..1 at `sample_rate` (after any decimation).
#[derive(Clone, Debug, PartialEq)]
pub struct Decoded {
    /// `channels` samples per frame.
    pub interleaved: Vec<f32>,
    /// Channels in the file.
    pub channels: u16,
    /// The rate of `interleaved` (the file's, halved once per 2:1 decimation).
    pub sample_rate: u32,
    /// The file's own rate.
    pub file_rate: u32,
    /// What it was.
    pub format: Format,
    /// Bits per sample, for PCM and FLAC.
    pub bits: Option<u16>,
    /// The hint for the intake.
    pub codec: CodecHint,
    /// Only the first [`MAX_DECODE_S`] seconds were read.
    pub truncated: bool,
    /// The decimation factor (1: none; 2 for 192 kHz; 4 for 384 kHz).
    pub decimation: u32,
}

impl Decoded {
    /// How it decoded, in a line: "MP3, 128 kbps, 44100 Hz, 2 channels".
    pub fn describe(&self) -> String {
        let mut parts = vec![self.format.name().to_owned()];
        if let Some(bits) = self.bits {
            parts.push(format!("{bits}-bit"));
        }
        if let Some(kbps) = self.codec.bitrate_kbps {
            parts.push(format!("{kbps} kbps"));
        }
        parts.push(format!("{} Hz", self.file_rate));
        parts.push(match self.channels {
            1 => "mono".to_owned(),
            2 => "stereo".to_owned(),
            n => format!("{n} channels"),
        });
        parts.join(", ")
    }

    /// What the user should know about the decoding.
    pub fn notes(&self) -> Vec<String> {
        let mut notes = Vec::new();
        if self.decimation > 1 {
            notes.push(format!(
                "decimated {}:1 from {} Hz to {} Hz (the analysis runs at {} to {} Hz)",
                self.decimation,
                self.file_rate,
                self.sample_rate,
                SAMPLE_RATES.start,
                SAMPLE_RATES.end - 1
            ));
        }
        if self.truncated {
            notes.push(format!(
                "only the first {MAX_DECODE_S:.0} s of the file were read"
            ));
        }
        notes
    }

    /// Frames decoded.
    pub fn frames(&self) -> usize {
        self.interleaved.len() / usize::from(self.channels.max(1))
    }
}

/// Why a file could not be decoded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodeError {
    /// A stable snake-case code: `io`, `unknown_format`, `bad_file`, `sample_rate`, `no_audio`, `cancelled`.
    pub code: &'static str,
    /// What happened.
    pub message: String,
    /// What to do about it.
    pub hint: String,
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for DecodeError {}

const FORMATS_HINT: &str =
    "use a WAV, FLAC, Ogg Vorbis or MP3 file (M4A/AAC and AIFF are not read yet)";

fn bad(format: Format, e: impl std::fmt::Display) -> DecodeError {
    DecodeError {
        code: "bad_file",
        message: format!("the {} file does not decode: {e}", format.name()),
        hint: "check the file plays elsewhere, or export it again as WAV".to_owned(),
    }
}

fn cancelled() -> DecodeError {
    DecodeError {
        code: "cancelled",
        message: "decoding was cancelled".to_owned(),
        hint: "start the job again".to_owned(),
    }
}

/// The hint of a rate outside the analysis's range.
pub fn rate_hint() -> String {
    format!(
        "the analysis runs at {} to {} Hz; files at 192 kHz and above are decimated exactly 2:1 until they are under \
         192 kHz (192 kHz to 96 kHz, 352.8 kHz to 176.4 kHz), other rates are refused: resample the file to 44.1 or 48 kHz",
        SAMPLE_RATES.start,
        SAMPLE_RATES.end - 1
    )
}

/// Decode the file at `path`, polling `stop` between chunks (it returns [`DecodeError`] `cancelled` once `stop` says
/// so).
///
/// # Errors
/// A file that cannot be opened (`io`), is none of the formats (`unknown_format`), does not decode (`bad_file`),
/// holds no audio (`no_audio`), has a rate the analysis cannot take even after decimation (`sample_rate`), or a stop.
pub fn decode_path(path: &Path, stop: &dyn Fn() -> bool) -> Result<Decoded, DecodeError> {
    let file = File::open(path).map_err(|e| DecodeError {
        code: "io",
        message: format!("cannot open {}: {e}", path.display()),
        hint: "check the path and that the file can be read".to_owned(),
    })?;
    let size = file.metadata().map_or(0, |m| m.len());
    let mut reader = BufReader::new(file);
    let mut head = vec![0u8; 64 * 1024];
    let mut filled = 0usize;
    while filled < head.len() {
        match reader.read(&mut head[filled..]) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(e) => {
                return Err(DecodeError {
                    code: "io",
                    message: format!("cannot read {}: {e}", path.display()),
                    hint: "check that the file can be read".to_owned(),
                });
            }
        }
    }
    head.truncate(filled);
    let format = sniff(&head, path).ok_or_else(|| DecodeError {
        code: "unknown_format",
        message: format!("{} is not a sound file this build reads", path.display()),
        hint: FORMATS_HINT.to_owned(),
    })?;
    reader.seek(SeekFrom::Start(0)).map_err(|e| DecodeError {
        code: "io",
        message: format!("cannot read {}: {e}", path.display()),
        hint: "check that the file can be read".to_owned(),
    })?;
    let raw = match format {
        Format::Wav => wav(reader, stop)?,
        Format::Flac => flac(reader, stop)?,
        Format::Vorbis => vorbis(reader, stop)?,
        Format::Mp3 => mp3(reader, size, stop)?,
    };
    finish(raw, format)
}

/// The format from the first bytes, else from the extension.
fn sniff(head: &[u8], path: &Path) -> Option<Format> {
    if head.len() >= 12
        && matches!(&head[..4], b"RIFF" | b"RIFX" | b"RF64")
        && &head[8..12] == b"WAVE"
    {
        return Some(Format::Wav);
    }
    if head.starts_with(b"fLaC") {
        return Some(Format::Flac);
    }
    if head.starts_with(b"OggS") {
        return Some(Format::Vorbis);
    }
    if nanomp3::detect(head) {
        return Some(Format::Mp3);
    }
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase());
    match ext.as_deref() {
        Some("mp3" | "mp2" | "mp1") => Some(Format::Mp3),
        _ => None,
    }
}

/// What a decoder produced, before decimation.
struct Raw {
    interleaved: Vec<f32>,
    channels: u16,
    rate: u32,
    bits: Option<u16>,
    codec: CodecHint,
    truncated: bool,
}

/// The most interleaved samples read for `channels` at `rate`.
fn limit(channels: u16, rate: u32) -> usize {
    (MAX_DECODE_S * f64::from(rate)) as usize * usize::from(channels.max(1))
}

fn wav(reader: impl Read, stop: &dyn Fn() -> bool) -> Result<Raw, DecodeError> {
    let mut r = hound::WavReader::new(reader).map_err(|e| bad(Format::Wav, e))?;
    let spec = r.spec();
    let max = limit(spec.channels, spec.sample_rate);
    let mut out = Vec::with_capacity(max.min(r.len() as usize));
    let mut truncated = false;
    match spec.sample_format {
        hound::SampleFormat::Float => {
            for (i, s) in r.samples::<f32>().enumerate() {
                if i >= max {
                    truncated = true;
                    break;
                }
                if i % POLL == 0 && stop() {
                    return Err(cancelled());
                }
                out.push(s.map_err(|e| bad(Format::Wav, e))?);
            }
        }
        hound::SampleFormat::Int => {
            let scale = 1.0 / f64::from(1u32 << (spec.bits_per_sample.clamp(1, 32) - 1));
            for (i, s) in r.samples::<i32>().enumerate() {
                if i >= max {
                    truncated = true;
                    break;
                }
                if i % POLL == 0 && stop() {
                    return Err(cancelled());
                }
                let v = s.map_err(|e| bad(Format::Wav, e))?;
                out.push((f64::from(v) * scale) as f32);
            }
        }
    }
    Ok(Raw {
        interleaved: out,
        channels: spec.channels,
        rate: spec.sample_rate,
        bits: Some(spec.bits_per_sample),
        codec: CodecHint::lossless(Codec::Pcm),
        truncated,
    })
}

fn flac(reader: impl Read, stop: &dyn Fn() -> bool) -> Result<Raw, DecodeError> {
    let mut r = claxon::FlacReader::new(reader).map_err(|e| bad(Format::Flac, e))?;
    let info = r.streaminfo();
    let channels = u16::try_from(info.channels).unwrap_or(u16::MAX);
    let max = limit(channels, info.sample_rate);
    let scale = 1.0 / f64::from(1u32 << (info.bits_per_sample.clamp(1, 32) - 1));
    let mut out = Vec::with_capacity(
        max.min(
            info.samples
                .map_or(0, |s| s as usize * usize::from(channels)),
        ),
    );
    let mut truncated = false;
    for (i, s) in r.samples().enumerate() {
        if i >= max {
            truncated = true;
            break;
        }
        if i % POLL == 0 && stop() {
            return Err(cancelled());
        }
        let v = s.map_err(|e| bad(Format::Flac, e))?;
        out.push((f64::from(v) * scale) as f32);
    }
    Ok(Raw {
        interleaved: out,
        channels,
        rate: info.sample_rate,
        bits: u16::try_from(info.bits_per_sample).ok(),
        codec: CodecHint::lossless(Codec::Flac),
        truncated,
    })
}

fn vorbis(reader: impl Read + Seek, stop: &dyn Fn() -> bool) -> Result<Raw, DecodeError> {
    use lewton::inside_ogg::OggStreamReader;
    use lewton::samples::InterleavedSamples;
    let mut r = OggStreamReader::new(reader).map_err(|e| DecodeError {
        hint: "only Ogg Vorbis is read (not Ogg Opus or FLAC in Ogg): export the file as WAV"
            .to_owned(),
        ..bad(Format::Vorbis, e)
    })?;
    let channels = u16::from(r.ident_hdr.audio_channels);
    let rate = r.ident_hdr.audio_sample_rate;
    let nominal = r.ident_hdr.bitrate_nominal;
    let max = limit(channels, rate);
    let mut out = Vec::new();
    let mut truncated = false;
    loop {
        if stop() {
            return Err(cancelled());
        }
        let packet = r
            .read_dec_packet_generic::<InterleavedSamples<f32>>()
            .map_err(|e| bad(Format::Vorbis, e))?;
        let Some(p) = packet else { break };
        out.extend_from_slice(&p.samples);
        if out.len() >= max {
            out.truncate(max);
            truncated = true;
            break;
        }
    }
    let kbps = u32::try_from(nominal)
        .ok()
        .filter(|&b| b > 0)
        .map(|b| (b + 500) / 1000);
    Ok(Raw {
        interleaved: out,
        channels,
        rate,
        bits: None,
        codec: CodecHint::lossy(Codec::Vorbis, kbps),
        truncated,
    })
}

fn mp3(reader: impl Read + Seek, size: u64, stop: &dyn Fn() -> bool) -> Result<Raw, DecodeError> {
    let mut r = nanomp3::Reader::<_, f32>::new(reader).map_err(|e| bad(Format::Mp3, e))?;
    let rate = r.sample_rate();
    let Some(ch) = r.channels() else {
        return Err(DecodeError {
            code: "no_audio",
            message: "the MP3 file holds no audio frames".to_owned(),
            hint: "check the file plays elsewhere".to_owned(),
        });
    };
    let channels = u16::from(ch.num());
    // The average bitrate over the stream, from its length: a hint, tags included.
    let kbps = r.total_samples().filter(|&n| n > 0 && rate > 0).map(|n| {
        let seconds = n as f64 / f64::from(rate);
        (size as f64 * 8.0 / seconds / 1000.0).round() as u32
    });
    let max = limit(channels, rate);
    let mut out = Vec::new();
    let mut buf = vec![0f32; POLL];
    let mut truncated = false;
    loop {
        if stop() {
            return Err(cancelled());
        }
        let n = r.read(&mut buf).map_err(|e| bad(Format::Mp3, e))?;
        if n == 0 {
            break;
        }
        out.extend_from_slice(&buf[..n]);
        if out.len() >= max {
            out.truncate(max);
            truncated = true;
            break;
        }
    }
    Ok(Raw {
        interleaved: out,
        channels,
        rate,
        bits: None,
        codec: CodecHint::lossy(Codec::Mp3, kbps),
        truncated,
    })
}

/// Check what a decoder gave and decimate to the analysis's range.
fn finish(raw: Raw, format: Format) -> Result<Decoded, DecodeError> {
    if raw.channels == 0 || raw.interleaved.is_empty() {
        return Err(DecodeError {
            code: "no_audio",
            message: format!("the {} file holds no audio", format.name()),
            hint: "check the file plays elsewhere".to_owned(),
        });
    }
    let channels = usize::from(raw.channels);
    let whole = raw.interleaved.len() / channels * channels;
    let mut interleaved = raw.interleaved;
    interleaved.truncate(whole);
    let mut rate = raw.rate;
    let mut decimation = 1u32;
    while rate >= SAMPLE_RATES.end && rate.is_multiple_of(2) {
        interleaved = decimate2(&interleaved, channels);
        rate /= 2;
        decimation *= 2;
    }
    if !SAMPLE_RATES.contains(&rate) {
        return Err(DecodeError {
            code: "sample_rate",
            message: format!("the file is at {} Hz", raw.rate),
            hint: rate_hint(),
        });
    }
    Ok(Decoded {
        interleaved,
        channels: raw.channels,
        sample_rate: rate,
        file_rate: raw.rate,
        format,
        bits: raw.bits,
        codec: raw.codec,
        truncated: raw.truncated,
        decimation,
    })
}

/// The half-band low-pass's odd taps, ±1, ±3, … ±31 (the even ones are 0, the centre is [`HALF_BAND_CENTRE`]): a
/// Kaiser-windowed (β 9) sinc of 63 taps, normalised to unit gain at DC (`scripts/halfband.py` in lane SK3's notes).
const HALF_BAND: [f64; 16] = [
    0.316_906_897_985_930_7,
    -0.101_961_488_818_660_64,
    0.056_971_316_252_498_114,
    -0.036_531_174_570_313_05,
    0.024_554_242_336_781_777,
    -0.016_680_226_481_742_062,
    0.011_228_384_473_108_13,
    -0.007_390_471_621_495_213,
    0.004_703_943_371_279_738,
    -0.002_863_517_431_348_899_6,
    0.001_645_846_551_686_075_8,
    -0.000_877_870_920_956_918_4,
    0.000_423_228_669_016_464_6,
    -0.000_175_972_117_976_898_9,
    5.675_717_154_619_472e-5,
    -9.389_310_007_412_442e-6,
];
const HALF_BAND_CENTRE: f64 = 0.499_998_988_921_308;

/// Decimate interleaved `x` (`channels` per frame) exactly 2:1: output frame `j` is the half-band filter centred on
/// input frame `2j`, zeros outside the file, summed in `f64` in a fixed order.
pub fn decimate2(x: &[f32], channels: usize) -> Vec<f32> {
    let frames = x.len() / channels;
    let out_frames = frames.div_ceil(2);
    let mut out = Vec::with_capacity(out_frames * channels);
    let at = |f: isize, c: usize| -> f64 {
        if f < 0 || f as usize >= frames {
            0.0
        } else {
            f64::from(x[f as usize * channels + c])
        }
    };
    for j in 0..out_frames {
        let centre = (2 * j) as isize;
        for c in 0..channels {
            let mut acc = HALF_BAND_CENTRE * at(centre, c);
            for (i, h) in HALF_BAND.iter().enumerate() {
                let k = (2 * i + 1) as isize;
                acc += h * (at(centre - k, c) + at(centre + k, c));
            }
            out.push(acc as f32);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_half_band_has_unit_gain_at_dc() {
        let sum = HALF_BAND_CENTRE + 2.0 * HALF_BAND.iter().sum::<f64>();
        assert!((sum - 1.0).abs() < 1e-12, "{sum}");
        let dc = vec![0.25f32; 2 * 400];
        let y = decimate2(&dc, 2);
        assert_eq!(y.len(), 400);
        // Away from the ends the gain is one.
        for v in &y[40..360] {
            assert!((v - 0.25).abs() < 1e-6, "{v}");
        }
    }

    #[test]
    #[allow(
        clippy::disallowed_methods,
        reason = "a test tone: its bits do not matter, only its level after the filter"
    )]
    fn decimation_keeps_the_audio_band_and_removes_what_would_alias() {
        let sr = 192_000.0;
        let tone = |f: f64| -> Vec<f32> {
            (0..19_200)
                .map(|i| (2.0 * std::f64::consts::PI * f * f64::from(i) / sr).sin() as f32 * 0.5)
                .collect()
        };
        let rms = |y: &[f32]| -> f64 {
            let mid = &y[200..y.len() - 200];
            (mid.iter()
                .map(|v| f64::from(*v) * f64::from(*v))
                .sum::<f64>()
                / mid.len() as f64)
                .sqrt()
        };
        let pass = rms(&decimate2(&tone(10_000.0), 1));
        assert!(
            (pass - 0.5 / 2f64.sqrt()).abs() < 1e-3,
            "10 kHz kept: {pass}"
        );
        let stop = rms(&decimate2(&tone(80_000.0), 1));
        assert!(
            stop < 1e-4,
            "80 kHz (it would alias to 16 kHz) removed: {stop}"
        );
    }
}
