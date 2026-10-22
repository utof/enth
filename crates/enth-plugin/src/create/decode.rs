//! **Owner: lane SK3.** The audio decoders of "create from sound" (docs/r3.md, "File input"), shared by the plugin's
//! `create` feature and `enth create`: the CLI includes this file by path (`#[path]`), so both shells decode a file to
//! the same bits. L8 only: core crates do no I/O.
//!
//! | Format | Crate | Exact? |
//! | --- | --- | --- |
//! | WAV (8–32-bit integer, 32-bit float; `RIFF`/`WAVE`) | `hound` | yes: integers scaled by a power of two |
//! | AIFF and AIFF-C (`FORM`/`AIFF`, `AIFC`: big-endian PCM 1–32 bits, `NONE`, `twos`, `sowt`, `fl32`, `fl64`) | ours (`aiff`, lane `INTa`; no dependency) | yes: integers scaled by a power of two |
//! | FLAC | `claxon` | yes: integers scaled by a power of two |
//! | Ogg Vorbis | `lewton` | the same bits on one machine; across platforms unverified (its window and twiddle tables use std's `sin`/`cos`/`exp`) |
//! | MP3 (and MP2, MP1) | `nanomp3`, a port of the CC0 minimp3 | the same bits on one machine; across platforms unverified (a fixed evaluation order and tables, no std transcendentals) |
//!
//! `symphonia` (M4A/AAC, ALAC) is MPL-2.0 and waits for the owner's OK. A result records the decoded clip's hash
//! (`enth.create/1`'s `input.audio_sha256`), so a replay can check its clip either way.
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
    /// AIFF or AIFF-C (uncompressed: `NONE`, `twos`, `sowt`, `fl32`, `fl64`).
    Aiff,
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
            Format::Aiff => "AIFF",
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
    /// Bits per sample, for PCM (WAV, AIFF) and FLAC.
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
    "use a WAV, AIFF, FLAC, Ogg Vorbis or MP3 file (M4A/AAC is not read yet)";

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
        Format::Aiff => aiff(reader, stop)?,
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
    if head.len() >= 12 && &head[..4] == b"FORM" && matches!(&head[8..12], b"AIFF" | b"AIFC") {
        return Some(Format::Aiff);
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
#[derive(Debug)]
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

/// How an AIFF file stores one sample (its `COMM` chunk's sample size and, in AIFF-C, compression type).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AiffEncoding {
    /// Two's-complement integers in `bytes` bytes, their bits left-justified as AIFF stores them: big-endian (AIFF,
    /// AIFF-C `NONE` and `twos`) or little-endian (AIFF-C `sowt`).
    Int { bytes: usize, little: bool },
    /// IEEE 754 binary32, big-endian (AIFF-C `fl32`, `FL32`).
    F32,
    /// IEEE 754 binary64, big-endian (AIFF-C `fl64`, `FL64`).
    F64,
}

impl AiffEncoding {
    /// Bytes per sample.
    fn bytes(self) -> usize {
        match self {
            AiffEncoding::Int { bytes, .. } => bytes,
            AiffEncoding::F32 => 4,
            AiffEncoding::F64 => 8,
        }
    }

    /// One sample in −1..1. An integer is scaled by a power of two of its container (exact, and the same bits as the
    /// same samples in a WAV of that depth: AIFF's left-justified bits make a 12-bit sample in two bytes read as the
    /// WAV reader's 12-bit value over 2¹¹); a float is taken as stored.
    fn sample(self, b: &[u8]) -> f32 {
        match self {
            AiffEncoding::Int { bytes, little } => {
                let push = |u: u32, byte: &u8| (u << 8) | u32::from(*byte);
                let u = if little {
                    b[..bytes].iter().rev().fold(0, push)
                } else {
                    b[..bytes].iter().fold(0, push)
                };
                let v = (u << (32 - 8 * bytes)) as i32;
                (f64::from(v) * (1.0 / 2_147_483_648.0)) as f32
            }
            AiffEncoding::F32 => f32::from_be_bytes([b[0], b[1], b[2], b[3]]),
            AiffEncoding::F64 => {
                f64::from_be_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]) as f32
            }
        }
    }
}

/// What an AIFF file's `COMM` chunk says.
#[derive(Clone, Copy, Debug, PartialEq)]
struct AiffComm {
    channels: u16,
    frames: u32,
    bits: u16,
    rate: u32,
    encoding: AiffEncoding,
}

/// The IEEE 754 80-bit extended number of a `COMM` chunk's sample rate (sign, a 15-bit exponent biased by 16,383, a
/// 64-bit mantissa with its integer bit explicit), as an `f64`: `None` for zero, a negative number, an infinity or a
/// NaN. Scaled by exact powers of two, so the same bytes give the same rate everywhere.
fn extended(b: [u8; 10]) -> Option<f64> {
    let sign_exp = u16::from_be_bytes([b[0], b[1]]);
    let mantissa = u64::from_be_bytes([b[2], b[3], b[4], b[5], b[6], b[7], b[8], b[9]]);
    let exp = i32::from(sign_exp & 0x7fff);
    if sign_exp & 0x8000 != 0 || exp == 0x7fff || mantissa == 0 {
        return None;
    }
    // value = mantissa · 2^(exp − 16383 − 63); a power of two 2^s for 0 ≤ s ≤ 1000 is an exact normal f64.
    let pow2 = |s: i32| f64::from_bits(((1023 + s) as u64) << 52);
    let mut v = mantissa as f64;
    let mut e = exp - 16_383 - 63;
    while e > 0 {
        let s = e.min(1000);
        v *= pow2(s);
        e -= s;
    }
    while e < 0 {
        let s = (-e).min(1000);
        v /= pow2(s);
        e += s;
    }
    (v.is_finite() && v > 0.0).then_some(v)
}

/// Read a `COMM` chunk's body (`aifc`: the AIFF-C form, which adds the compression type).
fn aiff_comm(body: &[u8], aifc: bool) -> Result<AiffComm, DecodeError> {
    if body.len() < 18 || (aifc && body.len() < 22) {
        return Err(bad(Format::Aiff, "its COMM chunk is too short"));
    }
    let channels = i16::from_be_bytes([body[0], body[1]]);
    let frames = u32::from_be_bytes([body[2], body[3], body[4], body[5]]);
    let bits = i16::from_be_bytes([body[6], body[7]]);
    let mut rate = [0u8; 10];
    rate.copy_from_slice(&body[8..18]);
    let kind: [u8; 4] = if aifc {
        [body[18], body[19], body[20], body[21]]
    } else {
        *b"NONE"
    };
    let channels = u16::try_from(channels)
        .map_err(|_| bad(Format::Aiff, format!("it says it has {channels} channels")))?;
    let encoding = match &kind {
        b"NONE" | b"twos" | b"sowt" => {
            if !(1..=32).contains(&bits) {
                return Err(bad(
                    Format::Aiff,
                    format!("its samples are {bits} bits (1 to 32 are read)"),
                ));
            }
            AiffEncoding::Int {
                bytes: (bits as usize).div_ceil(8),
                little: &kind == b"sowt",
            }
        }
        b"fl32" | b"FL32" => AiffEncoding::F32,
        b"fl64" | b"FL64" => AiffEncoding::F64,
        other => {
            return Err(DecodeError {
                code: "unknown_format",
                message: format!(
                    "the AIFF-C file is compressed ('{}'), which this build does not read",
                    other.escape_ascii()
                ),
                hint: "export it as uncompressed AIFF (or AIFF-C 'NONE', 'sowt', 'fl32') or as WAV"
                    .to_owned(),
            });
        }
    };
    let bits = match encoding {
        AiffEncoding::Int { .. } => bits as u16,
        AiffEncoding::F32 => 32,
        AiffEncoding::F64 => 64,
    };
    let hz = extended(rate)
        .map(f64::round)
        .filter(|&r| (1.0..=f64::from(u32::MAX)).contains(&r))
        .ok_or_else(|| DecodeError {
            code: "sample_rate",
            message: "the AIFF file's sample rate is not a positive number".to_owned(),
            hint: rate_hint(),
        })?;
    Ok(AiffComm {
        channels,
        frames,
        bits,
        rate: hz as u32,
        encoding,
    })
}

/// Fill `buf` as far as the reader goes; the count read (short only at the end).
fn read_full(r: &mut impl Read, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut filled = 0;
    while filled < buf.len() {
        match r.read(&mut buf[filled..]) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(filled)
}

/// **AIFF and AIFF-C, by hand** (lane `INTa`; no dependency): a `FORM` of chunks, each an id, a big-endian size and a
/// body padded to an even length, in any order. `COMM` gives the channels, the frames, the sample size, the rate (an
/// 80-bit extended number, [`extended`], rounded to the nearest hertz: a 22,254.545 Hz file plays 0.04 cents sharp)
/// and, in AIFF-C, the compression type; `SSND` gives an offset, a block size and the samples, interleaved. Read:
/// big-endian integers of 1 to 32 bits (AIFF; AIFF-C `NONE` and `twos`), little-endian ones (`sowt`), and big-endian
/// floats (`fl32`, `fl64`); any other compression is refused as `unknown_format` with a hint. Other chunks (markers,
/// instruments, comments, `FVER`) are skipped. A file that ends inside its `SSND` keeps the whole frames it holds,
/// and an `SSND` size a writer never filled in reads to the end.
fn aiff(mut reader: impl Read + Seek, stop: &dyn Fn() -> bool) -> Result<Raw, DecodeError> {
    let io = |e: std::io::Error| bad(Format::Aiff, e);
    let mut form = [0u8; 12];
    reader.read_exact(&mut form).map_err(io)?;
    let aifc = &form[8..12] == b"AIFC";
    let mut comm = None;
    let mut ssnd: Option<(u64, u64)> = None;
    let mut pos = 12u64;
    while comm.is_none() || ssnd.is_none() {
        let mut head = [0u8; 8];
        if read_full(&mut reader, &mut head).map_err(io)? < head.len() {
            break;
        }
        let size = u64::from(u32::from_be_bytes([head[4], head[5], head[6], head[7]]));
        let body = pos + 8;
        match &head[..4] {
            b"COMM" => {
                let mut b = vec![0u8; size.min(256) as usize];
                let n = read_full(&mut reader, &mut b).map_err(io)?;
                comm = Some(aiff_comm(&b[..n], aifc)?);
            }
            b"SSND" => ssnd = Some((body, size)),
            _ => {}
        }
        pos = body + size + (size & 1);
        reader.seek(SeekFrom::Start(pos)).map_err(io)?;
    }
    let comm = comm.ok_or_else(|| bad(Format::Aiff, "it has no COMM chunk"))?;
    let Some((body, size)) = ssnd else {
        if comm.frames == 0 {
            // A file with no frames may leave its SSND out: `finish` says it holds no audio.
            return Ok(Raw {
                interleaved: Vec::new(),
                channels: comm.channels,
                rate: comm.rate,
                bits: Some(comm.bits),
                codec: CodecHint::lossless(Codec::Pcm),
                truncated: false,
            });
        }
        return Err(bad(Format::Aiff, "it has no SSND chunk"));
    };
    reader.seek(SeekFrom::Start(body)).map_err(io)?;
    let mut offset = [0u8; 8];
    reader.read_exact(&mut offset).map_err(io)?;
    let offset = u64::from(u32::from_be_bytes([
        offset[0], offset[1], offset[2], offset[3],
    ]));
    reader
        .seek(SeekFrom::Start(body + 8 + offset))
        .map_err(io)?;
    let bps = comm.encoding.bytes();
    let channels = usize::from(comm.channels);
    let mut frames = u64::from(comm.frames);
    if let Some(len) = size.checked_sub(8 + offset).filter(|&l| l > 0) {
        frames = frames.min(len / (bps * channels.max(1)) as u64);
    }
    let total = usize::try_from(frames)
        .unwrap_or(usize::MAX)
        .saturating_mul(channels);
    let max = limit(comm.channels, comm.rate);
    let want = total.min(max);
    // Grown as read, never sized from the header alone: a damaged header cannot ask for gigabytes.
    let mut out = Vec::with_capacity(want.min(1 << 20));
    let mut buf = vec![0u8; POLL * bps];
    while out.len() < want {
        if stop() {
            return Err(cancelled());
        }
        let n = (want - out.len()).min(POLL) * bps;
        let got = read_full(&mut reader, &mut buf[..n]).map_err(io)?;
        out.extend(
            buf[..got - got % bps]
                .chunks_exact(bps)
                .map(|b| comm.encoding.sample(b)),
        );
        if got < n {
            break;
        }
    }
    Ok(Raw {
        interleaved: out,
        channels: comm.channels,
        rate: comm.rate,
        bits: Some(comm.bits),
        codec: CodecHint::lossless(Codec::Pcm),
        truncated: total > max,
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

    // ---- AIFF (lane `INTa`): files built in the test, so no binary fixture is committed ----

    /// The 80-bit extended number of a whole rate, as an AIFF writer stores it.
    fn ext(rate: u32) -> [u8; 10] {
        let lz = u64::from(rate).leading_zeros();
        let exp = (16_383 + 63 - lz) as u16;
        let mut b = [0u8; 10];
        b[..2].copy_from_slice(&exp.to_be_bytes());
        b[2..].copy_from_slice(&(u64::from(rate) << lz).to_be_bytes());
        b
    }

    /// A chunk: its id, its big-endian size, its body and a pad byte when the size is odd.
    #[allow(
        clippy::trivially_copy_pass_by_ref,
        reason = "the ids are byte-string literals, b\"COMM\""
    )]
    fn chunk(id: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut c = id.to_vec();
        c.extend_from_slice(&(body.len() as u32).to_be_bytes());
        c.extend_from_slice(body);
        if body.len() % 2 == 1 {
            c.push(0);
        }
        c
    }

    /// A `COMM` body: AIFF when `kind` is `None`, else AIFF-C with that compression type and a name.
    fn comm(
        kind: Option<&[u8; 4]>,
        channels: i16,
        frames: u32,
        bits: i16,
        rate: [u8; 10],
    ) -> Vec<u8> {
        let mut b = channels.to_be_bytes().to_vec();
        b.extend_from_slice(&frames.to_be_bytes());
        b.extend_from_slice(&bits.to_be_bytes());
        b.extend_from_slice(&rate);
        if let Some(k) = kind {
            b.extend_from_slice(k);
            b.extend_from_slice(b"\x0bnot named\x00"); // a pascal string, padded to an even length
        }
        b
    }

    /// An `SSND` body: `offset` junk bytes before the samples.
    fn ssnd(offset: u32, data: &[u8]) -> Vec<u8> {
        let mut b = offset.to_be_bytes().to_vec();
        b.extend_from_slice(&0u32.to_be_bytes());
        b.extend(std::iter::repeat_n(0xAA, offset as usize));
        b.extend_from_slice(data);
        b
    }

    /// A `FORM` of `chunks` (`AIFC` when `aifc`).
    fn form(aifc: bool, chunks: &[Vec<u8>]) -> Vec<u8> {
        let body: Vec<u8> = chunks.concat();
        let mut f = b"FORM".to_vec();
        f.extend_from_slice(&(body.len() as u32 + 4).to_be_bytes());
        f.extend_from_slice(if aifc { b"AIFC" } else { b"AIFF" });
        f.extend_from_slice(&body);
        f
    }

    /// Right-justified test values of `bits` bits (a fixed sweep over the whole range, extremes included).
    fn values(bits: u32, n: usize) -> Vec<i32> {
        let lo = -(1i64 << (bits - 1));
        let span = 1i64 << bits;
        (0..n)
            .map(|i| match i {
                0 => lo as i32,
                1 => (lo + span - 1) as i32,
                2 => 0,
                _ => (lo + (i as i64 * 7_919 + 13) % span) as i32,
            })
            .collect()
    }

    /// `values` stored as AIFF does: left-justified in whole bytes, big-endian (or little-endian for `sowt`).
    fn encode(v: &[i32], bits: u32, little: bool) -> Vec<u8> {
        let bytes = bits.div_ceil(8) as usize;
        let mut out = Vec::new();
        for &x in v {
            let left =
                ((x as u32) << (8 * bytes as u32 - bits)) & (u32::MAX >> (32 - 8 * bytes as u32));
            let be = left.to_be_bytes();
            let mut s = be[4 - bytes..].to_vec();
            if little {
                s.reverse();
            }
            out.extend_from_slice(&s);
        }
        out
    }

    fn read(bytes: &[u8]) -> Result<Raw, DecodeError> {
        aiff(std::io::Cursor::new(bytes.to_vec()), &|| false)
    }

    #[test]
    fn the_80_bit_rate_reads_exactly() {
        assert_eq!(ext(44_100), [0x40, 0x0E, 0xAC, 0x44, 0, 0, 0, 0, 0, 0]);
        assert_eq!(ext(48_000), [0x40, 0x0E, 0xBB, 0x80, 0, 0, 0, 0, 0, 0]);
        for r in [
            1,
            8_000,
            11_025,
            22_050,
            44_100,
            48_000,
            88_200,
            96_000,
            176_400,
            192_000,
            384_000,
            u32::MAX,
        ] {
            assert_eq!(extended(ext(r)), Some(f64::from(r)), "{r}");
        }
        // The classic Macintosh rate, 22,254.545… Hz: read as an f64, rounded to the nearest hertz by the reader.
        let mac = [0x40, 0x0D, 0xAD, 0xDD, 0x17, 0x45, 0xD1, 0x45, 0x82, 0x6B];
        let v = extended(mac).unwrap();
        assert!((v - 22_254.545_454_54).abs() < 1e-6, "{v}");
        let c = aiff_comm(&comm(None, 1, 0, 16, mac), false).unwrap();
        assert_eq!(c.rate, 22_255);
        // Zero, negative, infinite and NaN rates are refused.
        assert_eq!(extended([0; 10]), None);
        let mut neg = ext(44_100);
        neg[0] |= 0x80;
        assert_eq!(extended(neg), None);
        assert_eq!(extended([0x7F, 0xFF, 0x80, 0, 0, 0, 0, 0, 0, 0]), None);
        assert_eq!(extended([0x7F, 0xFF, 0xC0, 0, 0, 0, 0, 0, 0, 0]), None);
        for bad_rate in [[0u8; 10], neg] {
            let e = read(&form(
                false,
                &[
                    chunk(b"COMM", &comm(None, 1, 1, 16, bad_rate)),
                    chunk(b"SSND", &ssnd(0, &[0, 0])),
                ],
            ))
            .unwrap_err();
            assert_eq!(e.code, "sample_rate");
            assert!(e.hint.contains("decimated exactly 2:1"), "{}", e.hint);
        }
    }

    #[test]
    fn integer_aiff_and_aiff_c_read_like_the_wav_reader_at_every_depth() {
        for bits in [8u32, 12, 16, 20, 24, 32] {
            let v = values(bits, 300);
            // What the WAV reader gives for the same right-justified values.
            let scale = 1.0 / f64::from(1u32 << (bits - 1));
            let want: Vec<f32> = v.iter().map(|&x| (f64::from(x) * scale) as f32).collect();
            for (kind, aifc, little) in [
                (None, false, false),
                (Some(b"NONE"), true, false),
                (Some(b"twos"), true, false),
                (Some(b"sowt"), true, true),
            ] {
                let file = form(
                    aifc,
                    &[
                        chunk(b"COMM", &comm(kind, 2, 150, bits as i16, ext(48_000))),
                        chunk(b"SSND", &ssnd(0, &encode(&v, bits, little))),
                    ],
                );
                let raw = read(&file).unwrap();
                assert_eq!(raw.interleaved, want, "{bits} bits, {kind:?}");
                assert_eq!(
                    (raw.channels, raw.rate, raw.bits, raw.truncated),
                    (2, 48_000, Some(bits as u16), false)
                );
                assert_eq!(raw.codec, CodecHint::lossless(Codec::Pcm));
            }
        }
    }

    #[test]
    fn float_aiff_c_reads_as_stored() {
        let v: Vec<f64> = (0..64).map(|i| f64::from(i) / 64.0 - 0.5).collect();
        let f32s: Vec<u8> = v.iter().flat_map(|&x| (x as f32).to_be_bytes()).collect();
        let f64s: Vec<u8> = v.iter().flat_map(|&x| x.to_be_bytes()).collect();
        for (kind, data, bits) in [
            (b"fl32", &f32s, 32),
            (b"FL32", &f32s, 32),
            (b"fl64", &f64s, 64),
            (b"FL64", &f64s, 64),
        ] {
            let file = form(
                true,
                &[
                    chunk(b"COMM", &comm(Some(kind), 1, 64, bits, ext(44_100))),
                    chunk(b"SSND", &ssnd(0, data)),
                ],
            );
            let raw = read(&file).unwrap();
            let want: Vec<f32> = v.iter().map(|&x| x as f32).collect();
            assert_eq!(raw.interleaved, want, "{kind:?}");
            assert_eq!(raw.bits, Some(bits as u16));
        }
    }

    #[test]
    fn chunks_in_any_order_with_padding_offsets_and_extras_are_read() {
        let v = values(16, 40);
        let data = encode(&v, 16, false);
        let want: Vec<f32> = v
            .iter()
            .map(|&x| (f64::from(x) / 32_768.0) as f32)
            .collect();
        let file = form(
            true,
            &[
                chunk(b"FVER", &0xA280_5140u32.to_be_bytes()),
                chunk(b"ANNO", b"odd length"), // 10: no pad
                chunk(b"NAME", b"odd"),        // 3: padded
                chunk(b"SSND", &ssnd(6, &data)),
                chunk(b"MARK", &[0, 0]),
                chunk(b"COMM", &comm(Some(b"NONE"), 1, 40, 16, ext(96_000))),
            ],
        );
        let raw = read(&file).unwrap();
        assert_eq!(raw.interleaved, want);
        assert_eq!(raw.rate, 96_000);
        assert_eq!(sniff(&file, Path::new("no-extension")), Some(Format::Aiff));
        assert_eq!(
            sniff(&form(false, &[]), Path::new("x.wav")),
            Some(Format::Aiff)
        );
    }

    #[test]
    fn a_short_file_keeps_its_whole_frames_and_an_unfilled_size_reads_to_the_end() {
        let v = values(24, 2 * 100);
        let data = encode(&v, 24, false);
        let comm_c = chunk(b"COMM", &comm(None, 2, 100, 24, ext(48_000)));
        // Cut inside frame 61's second sample: 61 whole frames remain.
        let mut cut = form(false, &[comm_c.clone(), chunk(b"SSND", &ssnd(0, &data))]);
        cut.truncate(cut.len() - ((100 - 61) * 6 - 4));
        let raw = read(&cut).unwrap();
        let d = finish(raw, Format::Aiff).unwrap();
        assert_eq!(d.frames(), 61);
        // An SSND whose size was never filled in (0): the frames COMM names, read to the end.
        let mut unfilled = form(false, &[comm_c, chunk(b"SSND", &ssnd(0, &data))]);
        let at = unfilled.windows(4).position(|w| w == b"SSND").unwrap() + 4;
        unfilled[at..at + 4].copy_from_slice(&0u32.to_be_bytes());
        assert_eq!(read(&unfilled).unwrap().interleaved.len(), 200);
        // COMM's frame count bounds what is read.
        let fewer = form(
            false,
            &[
                chunk(b"COMM", &comm(None, 2, 10, 24, ext(48_000))),
                chunk(b"SSND", &ssnd(0, &data)),
            ],
        );
        assert_eq!(read(&fewer).unwrap().interleaved.len(), 20);
    }

    #[test]
    fn aiff_at_192_khz_is_decimated_and_long_files_stop_at_the_decode_limit() {
        let frames = 192_000 / 4;
        let data = vec![0u8; frames * 2];
        let file = form(
            false,
            &[
                chunk(b"COMM", &comm(None, 1, frames as u32, 16, ext(192_000))),
                chunk(b"SSND", &ssnd(0, &data)),
            ],
        );
        let d = finish(read(&file).unwrap(), Format::Aiff).unwrap();
        assert_eq!(
            (d.sample_rate, d.file_rate, d.decimation, d.frames()),
            (96_000, 192_000, 2, frames / 2)
        );
        assert_eq!(d.describe(), "AIFF, 16-bit, 192000 Hz, mono");
        // 8 kHz mono, 61 s: only MAX_DECODE_S are read.
        let frames = 8_000 * 61;
        let data = vec![0u8; frames];
        let file = form(
            false,
            &[
                chunk(b"COMM", &comm(None, 1, frames as u32, 8, ext(8_000))),
                chunk(b"SSND", &ssnd(0, &data)),
            ],
        );
        let raw = read(&file).unwrap();
        assert!(raw.truncated);
        assert_eq!(raw.interleaved.len(), 8_000 * 60);
    }

    #[test]
    fn compressed_or_damaged_aiff_is_refused_with_a_hint_and_a_stop_cancels() {
        let data = encode(&values(16, 8), 16, false);
        let with = |c: Vec<u8>, aifc: bool| {
            form(aifc, &[chunk(b"COMM", &c), chunk(b"SSND", &ssnd(0, &data))])
        };
        let e = read(&with(comm(Some(b"ulaw"), 1, 8, 16, ext(8_000)), true)).unwrap_err();
        assert_eq!(e.code, "unknown_format");
        assert!(e.message.contains("'ulaw'"), "{}", e.message);
        assert!(e.hint.contains("uncompressed AIFF"), "{}", e.hint);
        for bits in [0, 33, -16] {
            assert_eq!(
                read(&with(comm(None, 1, 8, bits, ext(8_000)), false))
                    .unwrap_err()
                    .code,
                "bad_file",
                "{bits}"
            );
        }
        assert_eq!(
            read(&with(comm(None, -1, 8, 16, ext(8_000)), false))
                .unwrap_err()
                .code,
            "bad_file"
        );
        // An AIFF-C COMM without its compression type is too short.
        assert_eq!(
            read(&with(comm(None, 1, 8, 16, ext(8_000)), true))
                .unwrap_err()
                .code,
            "bad_file"
        );
        assert_eq!(
            read(&form(false, &[chunk(b"SSND", &ssnd(0, &data))]))
                .unwrap_err()
                .code,
            "bad_file"
        );
        assert_eq!(
            read(&form(
                false,
                &[chunk(b"COMM", &comm(None, 1, 8, 16, ext(8_000)))]
            ))
            .unwrap_err()
            .code,
            "bad_file"
        );
        // No frames and no SSND: no audio.
        let empty = read(&form(
            false,
            &[chunk(b"COMM", &comm(None, 1, 0, 16, ext(8_000)))],
        ))
        .unwrap();
        assert_eq!(finish(empty, Format::Aiff).unwrap_err().code, "no_audio");
        // Zero channels: no audio.
        let none = read(&with(comm(None, 0, 8, 16, ext(8_000)), false)).unwrap();
        assert_eq!(finish(none, Format::Aiff).unwrap_err().code, "no_audio");
        assert_eq!(read(b"FORM\0\0").unwrap_err().code, "bad_file");
        let e = aiff(
            std::io::Cursor::new(with(comm(None, 1, 8, 16, ext(8_000)), false)),
            &|| true,
        )
        .unwrap_err();
        assert_eq!(e.code, "cancelled");
    }

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
