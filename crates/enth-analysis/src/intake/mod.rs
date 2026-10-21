//! **Owner: lane FE** (the intake, freeze point F1s). Pre-declared by lane S0s (R3, freeze point F0s).
//!
//! The intake of "create from sound" (docs/r3.md, "The intake"): pure functions of samples that describe what an
//! uploaded recording still carries, so the search stops paying knobs to reproduce damage. Like the rest of this crate
//! they do no I/O, read no clocks, spawn no threads and use no global RNG, they build for wasm32, transcendentals go
//! through `libm`, and every decision runs on the scalar FFT path. The decoders are L8's (the plugin's `create`
//! feature and the CLI); the intake starts from decoded samples at the file's own rate.
//!
//! **Short sounds are never rejected, and nothing is filtered by duration.** There is **no resampler in the product
//! path**: render and analysis run at the file's own rate (8 to 192 kHz), as `enth match` already does, so this
//! module has none (lane FE's open check, closed: `resample.rs` is deleted).
//!
//! | File | Holds |
//! | --- | --- |
//! | [`trim`] | mono (the mean of the channels), the onset, the leading-silence strip with a few ms of pre-roll, the tail cut where the signal falls into the floor, the length cap; [`Span`] |
//! | [`floor`] | the noise floor, and the hum (50 and 60 Hz with harmonics) that a flat floor hides; [`Hum`] |
//! | [`bandwidth`] | the effective bandwidth: where the spectrum stops (a 64 kbps MP3 low-passes near 11 kHz) |
//! | [`clip`] | hard-clipped spans; [`Clipping`] |
//! | [`perf`] | the inferred performance ([`InferredPerf`]): the note from YIN, the note-off from `envelope::infer_note_off`, the hold; the phrase, notes and chord warnings |
//!
//! [`Intake::new`] runs them in that order and returns the conditioned clip with an [`IntakeReport`]: the
//! [`Degradation`] (what OBJ's robust objective and TP's `create` consume), the [`InferredPerf`] (what every candidate
//! plays) and the [`Warning`]s (what the CLI and the plugin show). The report is serialisable and versioned
//! ([`INTAKE_VERSION`]); `enth.create/1` embeds it whole.
//!
//! **Time bases.** [`Span`] is in seconds of the decoded file; everything else is clip time, seconds from the
//! conditioned clip's first sample, where the onset sits at exactly [`InferredPerf::on_s`].
//!
//! Acceptance (docs/phase-3.md): trim within 5 ms; bandwidth within 1/3 octave; the note right on at least 95 % of
//! pitched clips; a chord warning; short clips kept (docs/r3.md, "The intake", has the measured numbers).

pub mod bandwidth;
pub mod clip;
pub mod floor;
pub mod perf;
pub mod trim;

#[cfg(test)]
mod tests;

use realfft::num_complex::Complex32;
use serde::{Deserialize, Serialize};

use crate::fft::{FftPath, FftPlanner, hann};

pub use clip::{ClipSpan, Clipping};
pub use floor::Hum;
pub use perf::InferredPerf;
pub use trim::Span;

/// The version of the intake's algorithm and report shape. A change to either that moves a number or a field bumps it.
pub const INTAKE_VERSION: u32 = 1;
/// Sample rates the intake takes (Hz): the analysis's own range (`Describer` takes 8 kHz up to, not including,
/// 192 kHz).
pub const SAMPLE_RATES: core::ops::Range<u32> = 8_000..192_000;
/// Below this sample peak (−100 dBFS) a file is silent: there is nothing to match.
pub const SILENT_PEAK: f32 = 1e-5;
/// The mono mix losing more than this (dB) against the channels raises [`Warning::ChannelsCancel`].
pub const CANCEL_DB: f64 = 6.0;
/// The intake looks at this many seconds of a file, from a second before its first sample that is not silent (the
/// clip is at most `max_len_s` from the onset).
pub const MAX_SOURCE_S: f64 = 30.0;

/// How the clip is conditioned. [`IntakeCfg::default`] is the product's setting.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct IntakeCfg {
    /// The length cap (s), pre-roll included: 5 by default (docs/r3.md: 4 to 6; evaluation cost grows with it).
    pub max_len_s: f64,
    /// Kept before the onset (ms), rounded up to whole 32-frame engine segments: 5 by default.
    pub preroll_ms: f64,
    /// Leading and trailing silence: this many dB below the loudest 20 ms frame (60 by default), or just above the
    /// noise floor when that is higher.
    pub silence_db: f64,
    /// Kept after the signal falls into the floor (ms): 100 by default, so a candidate that rings on is still heard
    /// to.
    pub tail_ms: f64,
    /// A clip shorter than this (s) is padded with silence, never rejected: 0.1 by default.
    pub min_len_s: f64,
}

impl Default for IntakeCfg {
    fn default() -> Self {
        Self {
            max_len_s: 5.0,
            preroll_ms: 5.0,
            silence_db: 60.0,
            tail_ms: 100.0,
            min_len_s: 0.1,
        }
    }
}

/// What the intake detected: what the upload no longer carries faithfully. Clip time.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Degradation {
    /// The noise floor, RMS dBFS; `None` when no noise-only stretch was found (clean, or sounding throughout).
    pub noise_floor_dbfs: Option<f64>,
    /// The loudest 20 ms frame above the floor (dB); `None` without a floor.
    pub snr_db: Option<f64>,
    /// Mains hum in the floor.
    pub hum: Option<Hum>,
    /// The cut-off of a steep low-pass (a codec, a resampled file), Hz; `None` for the full band up to the file's
    /// Nyquist frequency.
    pub bandwidth_hz: Option<f64>,
    /// Hard clipping.
    pub clipping: Option<Clipping>,
    /// The T60 of the final decay (s): a release and a room tail look alike here, so this is the decay of whatever
    /// is last, not a reverb detector. `None` without a measurable final decay.
    pub tail_t60_s: Option<f64>,
}

/// Something one note of a patch cannot reproduce, or that the user should know. The CLI and the plugin show them;
/// the CLI's `--perf` overrides the inferred performance.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Warning {
    /// Several onsets: this looks like a phrase.
    Phrase { onsets: usize },
    /// The pitch holds two or more different notes (MIDI keys, in order).
    Notes { keys: Vec<u8> },
    /// Several pitches at once: a chord, or an inharmonic sound (tones in Hz, ascending).
    Chord { tones_hz: Vec<f64> },
    /// No stable pitch: the key is a default.
    Unpitched,
    /// The sound runs past the length cap; the clip is its first `max_len_s` (the decoded length in seconds).
    Capped { source_s: f64 },
    /// The channels cancel in the mono mix (a polarity-flipped channel): the mono mix lost this many dB.
    ChannelsCancel { loss_db: f64 },
}

/// Everything the intake found, for `enth.create/1`, the plugin's Create view and lane OBJ's objective.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct IntakeReport {
    /// [`INTAKE_VERSION`].
    pub version: u32,
    /// The file's own rate (Hz): the clip's, and every candidate's render rate.
    pub sample_rate: u32,
    /// Channels in the file.
    pub channels: u16,
    /// The decoded file's length (s).
    pub source_s: f64,
    pub span: Span,
    pub degradation: Degradation,
    pub perf: InferredPerf,
    pub warnings: Vec<Warning>,
}

/// The conditioned clip and its report.
#[derive(Clone, Debug, PartialEq)]
pub struct Intake {
    /// Mono, at `report.sample_rate`, the onset at `report.perf.on_s`, at most `max_len_s` long and at least
    /// `min_len_s`. Every candidate renders `report.perf` for `report.perf.duration_s` and is compared with this.
    pub audio: Vec<f32>,
    pub report: IntakeReport,
}

/// Why the intake refused a file. A short sound is never a reason.
#[derive(Clone, Debug, PartialEq)]
pub enum IntakeError {
    /// No samples.
    Empty,
    /// The file declares no channels.
    NoChannels,
    /// The rate is outside [`SAMPLE_RATES`].
    SampleRate(u32),
    /// The first non-finite sample (index into the interleaved input).
    NonFinite { index: usize },
    /// The sample peak is below [`SILENT_PEAK`]: nothing to match.
    Silent,
    /// A field of [`IntakeCfg`] is out of range.
    BadConfig(&'static str),
}

impl std::fmt::Display for IntakeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IntakeError::Empty => write!(f, "the file holds no samples"),
            IntakeError::NoChannels => write!(f, "the file declares no channels"),
            IntakeError::SampleRate(sr) => write!(
                f,
                "the file is at {sr} Hz; the analysis runs at {} to {} Hz",
                SAMPLE_RATES.start,
                SAMPLE_RATES.end - 1
            ),
            IntakeError::NonFinite { index } => write!(f, "sample {index} is not finite"),
            IntakeError::Silent => write!(f, "the file is silent (its peak is below -100 dBFS)"),
            IntakeError::BadConfig(why) => write!(f, "bad intake settings: {why}"),
        }
    }
}

impl std::error::Error for IntakeError {}

impl Intake {
    /// The intake of a decoded file with the product's settings. `interleaved` holds `channels` samples per frame
    /// (1 for mono), at `sample_rate`.
    ///
    /// # Errors
    /// [`IntakeError`]: an empty, silent or non-finite file, no channels, or a rate outside [`SAMPLE_RATES`]. Never
    /// for being short.
    pub fn new(
        interleaved: &[f32],
        channels: u16,
        sample_rate: u32,
    ) -> Result<Intake, IntakeError> {
        Self::with_cfg(interleaved, channels, sample_rate, &IntakeCfg::default())
    }

    /// [`Intake::new`] with other settings.
    ///
    /// # Errors
    /// As [`Intake::new`], and [`IntakeError::BadConfig`] for a non-finite or negative setting, or a cap under
    /// 0.5 s.
    pub fn with_cfg(
        interleaved: &[f32],
        channels: u16,
        sample_rate: u32,
        cfg: &IntakeCfg,
    ) -> Result<Intake, IntakeError> {
        check(cfg)?;
        if channels == 0 {
            return Err(IntakeError::NoChannels);
        }
        if !SAMPLE_RATES.contains(&sample_rate) {
            return Err(IntakeError::SampleRate(sample_rate));
        }
        if let Some(index) = interleaved.iter().position(|s| !s.is_finite()) {
            return Err(IntakeError::NonFinite { index });
        }
        let mono = trim::mono(interleaved, channels);
        if mono.is_empty() {
            return Err(IntakeError::Empty);
        }
        let sr = f64::from(sample_rate);
        // `MAX_SOURCE_S` are looked at, from a second before the first sample that is not silent: the clip is at
        // most `max_len_s` from the onset, and work and memory stay bounded for any file.
        let Some(first) = mono.iter().position(|s| s.abs() >= SILENT_PEAK) else {
            return Err(IntakeError::Silent);
        };
        let skip = first.saturating_sub(sample_rate as usize);
        let src = &mono[skip..mono.len().min(skip + (MAX_SOURCE_S * sr) as usize)];
        let mut planner = FftPlanner::new(FftPath::Scalar);
        let mut warnings = Vec::new();
        if let Some(loss_db) = trim::mono_loss_db(interleaved, channels, &mono)
            && loss_db > CANCEL_DB
        {
            warnings.push(Warning::ChannelsCancel { loss_db });
        }

        // Floor first: the onset and the end sit just above it when it is above the silence threshold.
        let levels = trim::Levels::standard(src, sr);
        let peak = levels.peak_db();
        let fl = floor::estimate(&levels, src, sr, &mut planner);
        let quiet = peak - cfg.silence_db;
        // Above the floor by a margin that grows with its spread (brown noise wanders more than white), capped, and
        // never above 6 dB under the loudest frame, so the loudest frame always qualifies.
        let (thr_on, thr_end) = match fl {
            Some(f) => (
                quiet.max(f.level_db + (6.0 + 2.5 * f.spread_db).clamp(10.0, 20.0)),
                quiet.max(f.level_db + (3.0 + 2.0 * f.spread_db).clamp(6.0, 12.0)),
            ),
            None => (quiet, quiet),
        };
        let (thr_on, thr_end) = (thr_on.min(peak - 6.0), thr_end.min(peak - 6.0));
        let onset = trim::onset(src, sr, &levels, thr_on);
        let sig_end = trim::signal_end(&levels, src.len(), thr_end, onset);
        let frames = trim::CutFrames::new(
            sr,
            cfg.preroll_ms,
            cfg.tail_ms,
            cfg.max_len_s,
            cfg.min_len_s,
        );
        let (audio, mut span) = trim::cut(src, sr, onset, sig_end, frames);
        let shift = skip as f64 / sr;
        span.onset_s += shift;
        span.start_s += shift;
        span.end_s += shift;
        span.signal_end_s += shift;
        if span.capped {
            warnings.push(Warning::Capped {
                source_s: mono.len() as f64 / sr,
            });
        }

        // Clip coordinates: the onset after the pre-roll, the signal end where it fell (or the clip's end).
        let on = frames.preroll;
        let to_clip = |s: usize| (s + frames.preroll).saturating_sub(onset).min(audio.len());
        let end_c = to_clip(sig_end).max((on + 1).min(audio.len()));
        let hum = fl.and_then(|f| floor::hum(src, sr, f.run, &mut planner));
        let bandwidth_hz = bandwidth::estimate(&audio, sr, (on, end_c), &mut planner);
        let clipping = clip::detect(&audio, sr);
        let clip_levels = trim::Levels::standard(&audio, sr);
        let tail_t60_s = trim::tail_t60(
            &clip_levels,
            sr,
            end_c.div_ceil(clip_levels.hop),
            fl.map_or(clip_levels.peak_db() - cfg.silence_db, |f| f.level_db),
        );
        let (perf, found) =
            perf::infer(&audio, sr, on, end_c, fl.map(|f| f.level_db), &mut planner);
        if perf.onsets_s.len() > 1 {
            warnings.push(Warning::Phrase {
                onsets: perf.onsets_s.len(),
            });
        }
        if !found.notes.is_empty() {
            warnings.push(Warning::Notes { keys: found.notes });
        }
        if !found.chord.is_empty() {
            warnings.push(Warning::Chord {
                tones_hz: found.chord,
            });
        }
        if found.unpitched {
            warnings.push(Warning::Unpitched);
        }
        let report = IntakeReport {
            version: INTAKE_VERSION,
            sample_rate,
            channels,
            source_s: mono.len() as f64 / sr,
            span,
            degradation: Degradation {
                noise_floor_dbfs: fl.map(|f| f.level_db),
                snr_db: fl.map(|f| peak - f.level_db),
                hum,
                bandwidth_hz,
                clipping,
                tail_t60_s,
            },
            perf,
            warnings,
        };
        Ok(Intake { audio, report })
    }
}

fn check(cfg: &IntakeCfg) -> Result<(), IntakeError> {
    let all = [
        cfg.max_len_s,
        cfg.preroll_ms,
        cfg.silence_db,
        cfg.tail_ms,
        cfg.min_len_s,
    ];
    if all.iter().any(|v| !v.is_finite() || *v < 0.0) {
        return Err(IntakeError::BadConfig(
            "every setting is a finite number, at least 0",
        ));
    }
    if cfg.max_len_s < 0.5 {
        return Err(IntakeError::BadConfig("max_len_s is at least 0.5"));
    }
    if cfg.min_len_s > cfg.max_len_s {
        return Err(IntakeError::BadConfig("min_len_s is at most max_len_s"));
    }
    Ok(())
}

/// The power spectrum `|X_k|²` summed over Hann frames of `n` (even) every `n / 2` across `x[a..b]` on the scalar
/// path, and the frame count; one zero-padded frame when the range is shorter than `n`.
pub(crate) fn welch(
    planner: &mut FftPlanner,
    x: &[f32],
    a: usize,
    b: usize,
    n: usize,
) -> (Vec<f64>, usize) {
    let fft = planner.forward(n);
    let win = hann(n);
    let mut buf = vec![0.0f32; n];
    let mut spec = vec![Complex32::new(0.0, 0.0); fft.bins()];
    let mut scratch = fft.make_scratch();
    let mut acc = vec![0.0f64; fft.bins()];
    let (a, b) = (a.min(x.len()), b.min(x.len()));
    let mut frames = 0usize;
    let mut s = a;
    loop {
        let e = (s + n).min(b);
        buf.fill(0.0);
        for (i, v) in x[s..e].iter().enumerate() {
            buf[i] = v * win[i];
        }
        fft.forward(&mut buf, &mut spec, &mut scratch);
        for (acc_k, c) in acc.iter_mut().zip(&spec) {
            *acc_k += f64::from(c.re) * f64::from(c.re) + f64::from(c.im) * f64::from(c.im);
        }
        frames += 1;
        if s + n >= b {
            break;
        }
        s += n / 2;
    }
    (acc, frames)
}
