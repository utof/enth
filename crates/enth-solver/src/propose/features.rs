//! **Owner: lane F3C** (the model contract, freeze point F3s); lane ONX builds inference on it. Pre-declared by lane
//! S0s (R3).
//!
//! The model's **input** (docs/r3.md, "The model contract (F3s)"). **Features are computed here, in Rust, and never
//! re-implemented in Python** (the ML rules in docs/phase-3.md): `tools/datagen` (lane DG) calls
//! [`ModelInput::compute`] on every training clip and writes what it returns, and a create job calls the same
//! function on the upload, so training and inference cannot drift.
//!
//! # Feature set 1 (`propose-logmel/1`)
//!
//! Two inputs, `f32`, no batch axis:
//!
//! | Input | Shape | What |
//! | --- | --- | --- |
//! | `mel` | `[3, 128, 64]`: resolution, block, band | natural-log mel magnitudes on a fixed time grid |
//! | `context` | `[5]` | what the intake inferred, in fixed units |
//!
//! **`mel`**, step by step:
//!
//! 1. **The window:** the first [`WINDOW_S`] = 2 s of the intake's clip (`Intake::audio`: mono, the file's own rate,
//!    the onset at `perf.on_s`, about 5–9 ms in), from its first sample, zero-padded when the clip is shorter. This
//!    is the start of create's scored window ([`SCORE_S`](crate::create::SCORE_S), 3 s), so the model sees what the
//!    search scores. Short sounds are valid: a 0.1 s clip is 1.9 s of padding, never refused.
//! 2. **Level:** the window's audio (before the padding) is brought to −23 LUFS with the in-house BS.1770 meter
//!    (`enth_challenge::loudness`, bit-exact on every platform, as `sound/1` does; `ebur128` is not). A window at
//!    or below −70 LUFS is [`FeatureError::Silent`].
//! 3. **Log-mel:** `enth-analysis`'s `FeatureExtractor::extract_unnormalised` on the scalar FFT path at the file's
//!    own rate: 64 mel bands from 30 Hz to 16 kHz in Hz, at three resolutions **scaled to the rate**
//!    (`FeatureCfg::rate_scaled_fft`, the F3s judge's M4): `n = 2 · round(N · sample_rate / 96,000)` points for
//!    N = 2048, 1024 and 512 (exactly those at 48 kHz; 1882, 940 and 470 at 44.1 kHz; 4096, 2048 and 1024 at
//!    96 kHz), with a hop of `n / 2`. So the bins are about 23.4 Hz wide and the windows the same length in seconds
//!    at every rate, and a band's level does not follow the rate: with fixed sizes a noise band read about 0.5
//!    nepers louder at 16 kHz and 0.35 quieter at 96 kHz than at 48 (the judge's probe), and the model could not
//!    tell the rate.
//! 4. **The time grid, in seconds:** frames per second would still follow any rounding of the sizes, and the grid
//!    keeps the time axis exact. Each resolution's frames are read as a piecewise-constant track (frame m holds from
//!    half a hop before its centre to half a hop after; the first from the window's start, the last to its end), and
//!    each of [`FRAMES`] = 128 blocks of 1/64 s takes the **overlap-weighted mean of the mel magnitudes** over its
//!    span, then the natural log. Sums run in `f64`, in frame order, with `libm`'s `exp` and `log`: the same bits on
//!    every platform.
//! 5. **Empty bands:** a band whose upper edge lies above `min(sample_rate / 2, bandwidth_hz)` (the intake's
//!    detected low-pass, as OBJ's distance and the k-NN embedding mask it) is set to [`masked`] (`logf(1e-5)`, the
//!    analysis's floor of a silent band, bit for bit), so a low-rate or band-limited file has exactly empty top
//!    bands.
//!
//! **`context`:** `[key − 60, pitch offset, pitched, hold, band limit]`:
//!
//! | # | Value | Unit |
//! | --- | --- | --- |
//! | 0 | the intake's MIDI key − 60 (the key the model is told) | semitones |
//! | 1 | `12 · log2(f0 / f(key))` when the intake found a pitch, else 0 | semitones (about ±0.5) |
//! | 2 | 1 when the intake found a pitch, else 0 | — |
//! | 3 | the note's hold, capped at [`HOLD_CAP_S`] = 4 s | seconds |
//! | 4 | `log2(min(sample_rate / 2, bandwidth_hz, 16 kHz) / 16 kHz)` | octaves, at most 0 |
//!
//! **The cue is the intake's, never an override** ([`Cue::from_intake`]; the F3s judge's M1): the key, the pitch and
//! the hold the intake inferred. A `--perf` override changes how a create job renders, not what the model is told;
//! [`decode`](super::decode) re-keys the hypotheses onto the job's key. The model is pitch-relative, and so are
//! `ds1`'s labels: `osc1.semi` and `osc2.semi` against the key the intake reads ([`Cue::key`]), and `osc1.fine`
//! from [`Cue::pitch_offset`] (layout 1's context entry).
//!
//! **Determinism.** Every value is the same on every platform and under any FTZ/DAZ setting (`tests/propose_ftz.rs`):
//! the scalar FFT path, the in-house loudness, `libm`, and no value of `mel` or `context` is subnormal (a log of an
//! `f64` is 0 or at least about 1e-16 in magnitude). The scalar FFT's own twiddles come from `libm`, but `rustfft`'s
//! inner transforms take theirs from `std`'s `f64` `cos` and `sin` (as every analysis in `enth-analysis` does), so
//! `feature_set_1_is_pinned` runs on every CI leg (Linux, Windows, macOS arm64 and wasm32: lane ONX's hash
//! step).
//!
//! A new definition is a new feature id (and a new model): [`LATEST_FEATURE`] grows, old ids stay readable.

use core::fmt;

use enth_analysis::features::{BANDS, FFT_SIZES, FeatureCfg, FeatureExtractor, MEL_RANGE_HZ};
use enth_analysis::fft::FftPath;
use enth_analysis::intake::Intake;
use enth_challenge::loudness;
use sha2::{Digest, Sha256};

/// Feature set 1 (see the [module docs](self)).
pub const FEATURE_V1: u16 = 1;

/// The newest feature set this build computes. A model naming a newer one is refused as too new.
pub const LATEST_FEATURE: u16 = 1;

/// Feature set 1's stable name.
pub const FEATURE_V1_NAME: &str = "propose-logmel/1";

/// The window the model sees, from the clip's first sample (s).
pub const WINDOW_S: f64 = 2.0;

/// Blocks of the time grid over the window (1/64 s each).
pub const FRAMES: usize = 128;

/// Mel resolutions: `enth-analysis`'s [`FFT_SIZES`].
pub const RESOLUTIONS: usize = FFT_SIZES.len();

/// Mel bands per resolution: `enth-analysis`'s [`BANDS`].
pub const MEL_BANDS: usize = BANDS;

/// Values in `context`.
pub const CONTEXT: usize = 5;

/// The longest hold `context` reports (s): `ds1`'s phrase sampler holds 0.08–4 s.
pub const HOLD_CAP_S: f64 = 4.0;

/// The loudness the window is brought to (LUFS), `sound/1`'s.
pub const TARGET_LUFS: f64 = loudness::SCORING_LUFS;

/// The name of the log-mel input in a model file.
pub const MEL_INPUT: &str = "mel";

/// The name of the context input in a model file.
pub const CONTEXT_INPUT: &str = "context";

/// One input a feature set gives a model: its name and shape (`f32`, no batch axis).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InputSpec {
    /// The name a model file's input table uses.
    pub name: &'static str,
    /// The shape.
    pub dims: &'static [u32],
}

/// A feature set: what a model file's `feature` field names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FeatureSpec {
    /// The id ([`FEATURE_V1`]).
    pub id: u16,
    /// The stable name.
    pub name: &'static str,
    /// The inputs, in the order a model file lists them.
    pub inputs: &'static [InputSpec],
}

/// Feature set 1.
pub const SPEC_V1: FeatureSpec = FeatureSpec {
    id: FEATURE_V1,
    name: FEATURE_V1_NAME,
    inputs: &[
        InputSpec {
            name: MEL_INPUT,
            dims: &[RESOLUTIONS as u32, FRAMES as u32, MEL_BANDS as u32],
        },
        InputSpec {
            name: CONTEXT_INPUT,
            dims: &[CONTEXT as u32],
        },
    ],
};

/// The feature set with this id, if this build computes it.
#[must_use]
pub fn spec(id: u16) -> Option<&'static FeatureSpec> {
    (id == FEATURE_V1).then_some(&SPEC_V1)
}

/// The value of a masked (empty) band: `logf(1e-5)`, bit for bit the analysis's log-mel of a silent band
/// (`logf(0 + 1e-5)`), so padded silence and a masked band hold the same value.
#[must_use]
pub fn masked() -> f32 {
    libm::logf(1e-5)
}

/// What the model is told besides the audio: the intake's inferred performance and detected bandwidth (see the
/// [module docs](self)). Always the intake's ([`Cue::from_intake`]), never a `--perf` override: labels are relative
/// to this key (the F3s judge's M1).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cue {
    /// The MIDI key the intake reads (the nearest note to its pitch; 48 when unpitched): the key the model is told.
    pub key: u8,
    /// The intake's pitch reading, when the clip is pitched.
    pub f0_hz: Option<f64>,
    /// The note-on in the clip (s): the intake's onset, after its pre-roll.
    pub on_s: f64,
    /// How long the note is held (s).
    pub hold_s: f64,
    /// The intake's detected low-pass (Hz), when it found one.
    pub bandwidth_hz: Option<f64>,
}

impl Cue {
    /// The cue of an intake: its inferred key, pitch, note-on and hold, and its detected bandwidth.
    #[must_use]
    pub fn from_intake(intake: &Intake) -> Cue {
        let r = &intake.report;
        Cue {
            key: r.perf.key,
            f0_hz: r.perf.f0_hz,
            on_s: r.perf.on_s,
            hold_s: r.perf.hold_s,
            bandwidth_hz: r.degradation.bandwidth_hz,
        }
    }

    /// `context[1]`: `12 · log2(f0 / f(key))` in semitones when the intake found a pitch (about ±0.5), else `None`.
    /// Layout 1 decodes `osc1.fine` from it (its context entry).
    #[must_use]
    pub fn pitch_offset(&self) -> Option<f64> {
        let key_hz = 440.0 * libm::pow(2.0, (f64::from(self.key) - 69.0) / 12.0);
        self.f0_hz
            .filter(|f| f.is_finite() && *f > 0.0)
            .map(|f0| 12.0 * libm::log2(f0 / key_hz))
    }
}

/// Why the input could not be computed. A create job then runs without the model (R3a) and says why.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum FeatureError {
    /// A rate outside 8,000–191,999 Hz (the analysis's range).
    SampleRate(u32),
    /// A cue's key above 127.
    Key(u8),
    /// A sample is NaN or infinite.
    NonFinite,
    /// The window is at or below −70 LUFS.
    Silent,
}

impl FeatureError {
    /// A stable snake-case code.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            FeatureError::SampleRate(_) => "features_sample_rate",
            FeatureError::Key(_) => "features_key",
            FeatureError::NonFinite => "features_non_finite",
            FeatureError::Silent => "features_silent",
        }
    }
}

impl fmt::Display for FeatureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FeatureError::SampleRate(sr) => {
                write!(f, "{sr} Hz is outside the analysis's 8000 to 191999 Hz")
            }
            FeatureError::Key(k) => write!(f, "the cue's key {k} is above 127"),
            FeatureError::NonFinite => f.write_str("the clip has a non-finite sample"),
            FeatureError::Silent => {
                f.write_str("the model's window is silent (at or below -70 LUFS)")
            }
        }
    }
}

impl std::error::Error for FeatureError {}

/// The model's input for one clip (feature set 1).
#[derive(Clone, Debug, PartialEq)]
pub struct ModelInput {
    /// `[RESOLUTIONS, FRAMES, MEL_BANDS]`, row-major: `mel[(r · FRAMES + t) · MEL_BANDS + b]`.
    pub mel: Vec<f32>,
    /// `[CONTEXT]` (see the [module docs](self)).
    pub context: [f32; CONTEXT],
}

/// The upper edge (Hz) of each mel band, from the same formula as `enth-analysis`'s filter bank.
fn band_upper_edges() -> [f64; BANDS] {
    let mel = |f: f64| 2595.0 * libm::log10(1.0 + f / 700.0);
    let hz = |m: f64| 700.0 * (libm::pow(10.0, m / 2595.0) - 1.0);
    let (lo, hi) = (mel(MEL_RANGE_HZ.0), mel(MEL_RANGE_HZ.1));
    let mut out = [0.0; BANDS];
    for (b, e) in out.iter_mut().enumerate() {
        *e = hz(lo + (hi - lo) * (b + 2) as f64 / (BANDS + 1) as f64);
    }
    out
}

/// One resolution's frames (`frames × BANDS` natural logs, frame m starting at `m · hop`) re-gridded onto
/// [`FRAMES`] blocks over `window` samples: the overlap-weighted mean of the magnitudes, then the log.
fn regrid(logmel: &[f32], n_fft: usize, hop: usize, window: usize, out: &mut [f32]) {
    let frames = logmel.len() / BANDS;
    let floor = masked();
    if frames == 0 {
        out.fill(floor);
        return;
    }
    // Frame m holds [lo(m), lo(m + 1)): half a hop either side of its centre m·hop + n/2, the first from 0 and
    // the last to the window's end. Every bound is a whole number of samples, every block bound a multiple of
    // 1/FRAMES of the window (exact in f64), so every overlap is exact.
    let lo = |m: usize| -> f64 {
        if m == 0 {
            0.0
        } else if m >= frames {
            window as f64
        } else {
            (m * hop + n_fft / 2 - hop / 2) as f64
        }
    };
    let block = window as f64 / FRAMES as f64;
    let mut first = 0;
    for t in 0..FRAMES {
        let (b0, b1) = (t as f64 * block, (t + 1) as f64 * block);
        while first + 1 < frames && lo(first + 1) <= b0 {
            first += 1;
        }
        let row = &mut out[t * BANDS..(t + 1) * BANDS];
        for (band, v) in row.iter_mut().enumerate() {
            let mut acc = 0.0f64;
            let mut m = first;
            while m < frames && lo(m) < b1 {
                let overlap = lo(m + 1).min(b1) - lo(m).max(b0);
                if overlap > 0.0 {
                    acc += overlap * libm::exp(f64::from(logmel[m * BANDS + band]));
                }
                m += 1;
            }
            *v = libm::log(acc / (b1 - b0)) as f32;
        }
    }
}

impl ModelInput {
    /// The input for `clip` (the intake's clip: mono, at `sample_rate`, from its first sample) and `cue` (see the
    /// [module docs](self)). Pure and deterministic: the same bits on every platform.
    ///
    /// # Errors
    /// [`FeatureError::SampleRate`], [`FeatureError::NonFinite`], [`FeatureError::Silent`]. Never a short clip.
    pub fn compute(clip: &[f32], sample_rate: u32, cue: &Cue) -> Result<ModelInput, FeatureError> {
        if !(8_000..192_000).contains(&sample_rate) {
            return Err(FeatureError::SampleRate(sample_rate));
        }
        if cue.key > 127 {
            return Err(FeatureError::Key(cue.key));
        }
        let sr = f64::from(sample_rate);
        let window = libm::round(WINDOW_S * sr) as usize;
        let audio = &clip[..clip.len().min(window)];
        if audio.iter().any(|s| !s.is_finite()) {
            return Err(FeatureError::NonFinite);
        }
        let lufs = loudness::integrated_lufs(audio, sample_rate).ok_or(FeatureError::Silent)?;
        let gain = loudness::db_to_amp(loudness::gain_db(lufs, TARGET_LUFS)) as f32;
        let mut x = vec![0.0f32; window];
        for (o, s) in x.iter_mut().zip(audio) {
            *o = s * gain;
        }
        let cfg = FeatureCfg {
            sample_rate: sr,
            hop_div: 2,
            target_lufs: TARGET_LUFS,
            with_f0: false,
            fft: FftPath::Scalar,
            rate_scaled_fft: true,
            ..FeatureCfg::default()
        };
        let features = FeatureExtractor::new(cfg).extract_unnormalised(&x);
        if features.silent {
            return Err(FeatureError::Silent);
        }
        let mut mel = vec![masked(); RESOLUTIONS * FRAMES * BANDS];
        for (r, &n_fft) in cfg.fft_sizes().iter().enumerate() {
            let hop = (n_fft / 2).max(1);
            let out = &mut mel[r * FRAMES * BANDS..(r + 1) * FRAMES * BANDS];
            regrid(&features.logmel[r], n_fft, hop, window, out);
        }
        let nyquist = 0.5 * sr;
        let top = cue
            .bandwidth_hz
            .filter(|b| b.is_finite() && *b > 0.0)
            .map_or(nyquist, |b| b.min(nyquist));
        let floor = masked();
        for (b, edge) in band_upper_edges().iter().enumerate() {
            if *edge > top {
                for rt in 0..RESOLUTIONS * FRAMES {
                    mel[rt * BANDS + b] = floor;
                }
            }
        }
        let (offset, pitched) = match cue.pitch_offset() {
            Some(offset) => (offset, 1.0),
            None => (0.0, 0.0),
        };
        let hold = if cue.hold_s.is_finite() {
            cue.hold_s.clamp(0.0, HOLD_CAP_S)
        } else {
            HOLD_CAP_S
        };
        let limit = libm::log2(top.min(MEL_RANGE_HZ.1) / MEL_RANGE_HZ.1);
        let context = [
            (f64::from(cue.key) - 60.0) as f32,
            offset as f32,
            pitched as f32,
            hold as f32,
            limit as f32,
        ];
        Ok(ModelInput { mel, context })
    }

    /// The input for an intake: its clip, rate and [`Cue::from_intake`]. A create job computes it from the intake,
    /// before any `--perf` override (the override changes how candidates render, not what the model is told).
    ///
    /// # Errors
    /// As [`compute`](Self::compute).
    pub fn from_intake(intake: &Intake) -> Result<ModelInput, FeatureError> {
        ModelInput::compute(
            &intake.audio,
            intake.report.sample_rate,
            &Cue::from_intake(intake),
        )
    }

    /// One `mel` value: resolution `r`, block `t`, band `b`.
    #[must_use]
    pub fn mel_at(&self, r: usize, t: usize, b: usize) -> f32 {
        self.mel[(r * FRAMES + t) * MEL_BANDS + b]
    }

    /// SHA-256 of the feature id (`u16`, little-endian), then every `mel` value, then every `context` value (`f32`,
    /// little-endian, in order): the model input's hash a result records.
    #[must_use]
    pub fn sha256(&self) -> [u8; 32] {
        let mut h = Sha256::new();
        h.update(FEATURE_V1.to_le_bytes());
        for v in &self.mel {
            h.update(v.to_le_bytes());
        }
        for v in &self.context {
            h.update(v.to_le_bytes());
        }
        h.finalize().into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cue() -> Cue {
        Cue {
            key: 57,
            f0_hz: Some(220.0),
            on_s: 0.005,
            hold_s: 0.8,
            bandwidth_hz: None,
        }
    }

    /// Deterministic white noise (xorshift), `len_s` long at `sr`, after `on_s` of silence.
    fn noise(sr: u32, on_s: f64, len_s: f64) -> Vec<f32> {
        let mut x = 0x9e37_79b9_7f4a_7c15u64;
        let n = (len_s * f64::from(sr)) as usize;
        let on = (on_s * f64::from(sr)) as usize;
        (0..n)
            .map(|i| {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                if i < on {
                    0.0
                } else {
                    ((x >> 11) as f64 / (1u64 << 53) as f64 - 0.5) as f32 * 0.4
                }
            })
            .collect()
    }

    /// A plucked saw at `hz` starting at `on_s`, decaying, `len_s` long at `sr`.
    fn pluck(sr: u32, hz: f64, on_s: f64, len_s: f64) -> Vec<f32> {
        let n = (len_s * f64::from(sr)) as usize;
        (0..n)
            .map(|i| {
                let t = i as f64 / f64::from(sr) - on_s;
                if t < 0.0 {
                    0.0
                } else {
                    let phase = (t * hz).fract();
                    ((2.0 * phase - 1.0) * libm::exp(-3.0 * t)) as f32 * 0.5
                }
            })
            .collect()
    }

    fn hex(b: &[u8]) -> String {
        use core::fmt::Write as _;
        b.iter().fold(String::new(), |mut s, x| {
            let _ = write!(s, "{x:02x}");
            s
        })
    }

    #[test]
    fn the_shapes_are_feature_set_1s() {
        let x = pluck(48_000, 220.0, 0.005, 1.0);
        let m = ModelInput::compute(&x, 48_000, &cue()).unwrap();
        assert_eq!(m.mel.len(), 3 * 128 * 64);
        let dims: Vec<usize> = SPEC_V1.inputs[0].dims.iter().map(|d| *d as usize).collect();
        assert_eq!(dims.iter().product::<usize>(), m.mel.len());
        assert_eq!(SPEC_V1.inputs[1].dims, &[5]);
        assert!(m.mel.iter().all(|v| v.is_finite()));
        assert_eq!(spec(1), Some(&SPEC_V1));
        assert_eq!(spec(2), None);
    }

    /// Feature set 1 is frozen: a change to anything it reads (the analysis's log-mel, the FFT sizes, the loudness
    /// meter, the grid, the masking) moves one of these hashes, and needs feature set 2 and a new model, not an edit.
    /// One input per path that differs: a non-48 kHz rate (scaled FFT sizes), a band limit, 16 kHz (masked top bands),
    /// a clip shorter than the window, unpitched noise, 96 kHz. Run on every CI leg (lane ONX's hash step).
    #[test]
    fn feature_set_1_is_pinned() {
        use core::fmt::Write as _;
        let unpitched = Cue {
            key: 48,
            f0_hz: None,
            hold_s: 1.2,
            ..cue()
        };
        let banded = Cue {
            bandwidth_hz: Some(4_000.0),
            ..cue()
        };
        let cases: [(&str, Vec<f32>, u32, Cue, &str); 6] = [
            (
                "44.1 kHz pluck",
                pluck(44_100, 196.0, 0.006, 1.3),
                44_100,
                cue(),
                "2a740d072505bb39fd2aff679c53b41414940738195fa749399f1cf5bfdd6f83",
            ),
            (
                "48 kHz pluck, 4 kHz band limit",
                pluck(48_000, 220.0, 0.005, 1.5),
                48_000,
                banded,
                "1e1634f338a32dcfa5c41ee4bb5bd8894db8cd193cecca6056c185a4bf5d35ca",
            ),
            (
                "16 kHz pluck",
                pluck(16_000, 220.0, 0.005, 1.5),
                16_000,
                cue(),
                "b9cb9298f4a0ae27c69e5d6c82f0b0ab1a743082c600cdf8ad81fd02ee7d17e3",
            ),
            (
                "0.1 s clip",
                pluck(48_000, 440.0, 0.005, 0.1),
                48_000,
                cue(),
                "b9440d3581f6185abc6c55d4af4bddcf94b4d65d2784f6ffe7dc1fb3f89fc3bb",
            ),
            (
                "unpitched noise",
                noise(48_000, 0.007, 1.0),
                48_000,
                unpitched,
                "8c8848c5088ad3a74dd6107af4574b8ac58584e7828febf944ee9236fa42096e",
            ),
            (
                "96 kHz pluck",
                pluck(96_000, 220.0, 0.005, 1.5),
                96_000,
                cue(),
                "34ca0bad6772332fbb72d214a43ca76729e64073096f8b7ca80bec06ef31e7a2",
            ),
        ];
        let mut got = String::new();
        let mut bad = false;
        for (name, x, sr, c, want) in &cases {
            let m = ModelInput::compute(x, *sr, c).unwrap();
            assert_eq!(m, ModelInput::compute(x, *sr, c).unwrap(), "{name}");
            let h = hex(&m.sha256());
            bad |= h != *want;
            let _ = writeln!(got, "{name}: {h}");
        }
        assert!(!bad, "feature set 1 moved:\n{got}");
    }

    #[test]
    fn the_time_grid_is_in_seconds_at_every_rate() {
        // A pluck starting 0.5 s in: the first loud block is the same at every rate.
        let first_loud = |sr: u32| {
            let x = pluck(sr, 220.0, 0.5, 1.5);
            let m = ModelInput::compute(&x, sr, &cue()).unwrap();
            // The 512-point resolution, the band nearest 220 Hz's region: the first block 20 dB (2.3 nepers) above
            // the quiet start.
            let r = 2;
            let quiet = m.mel_at(r, 0, 8);
            (0..FRAMES)
                .find(|t| m.mel_at(r, *t, 8) > quiet + 2.3)
                .unwrap()
        };
        let at48 = first_loud(48_000);
        assert!((31..=33).contains(&at48), "{at48}");
        for sr in [44_100, 32_000, 22_050, 16_000] {
            let t = first_loud(sr);
            assert!(t.abs_diff(at48) <= 1, "{sr} Hz: block {t}, 48 kHz: {at48}");
        }
    }

    #[test]
    fn bands_above_the_rate_or_the_bandwidth_are_exactly_empty() {
        let edges = band_upper_edges();
        let x = pluck(16_000, 220.0, 0.005, 1.0);
        let m = ModelInput::compute(&x, 16_000, &cue()).unwrap();
        for (b, e) in edges.iter().enumerate() {
            let empty = (0..FRAMES).all(|t| m.mel_at(0, t, b) == masked());
            assert_eq!(empty, *e > 8_000.0, "band {b}, edge {e}");
        }
        assert!((m.context[4] - -1.0).abs() < 1e-6);
        let x = pluck(48_000, 220.0, 0.005, 1.0);
        let banded = Cue {
            bandwidth_hz: Some(4_000.0),
            ..cue()
        };
        let m = ModelInput::compute(&x, 48_000, &banded).unwrap();
        for (b, e) in edges.iter().enumerate() {
            assert_eq!(m.mel_at(1, 10, b) == masked(), *e > 4_000.0, "band {b}");
        }
        assert!((m.context[4] - -2.0).abs() < 1e-6);
    }

    #[test]
    fn short_clips_are_padded_and_silence_is_refused() {
        let x = pluck(48_000, 440.0, 0.005, 0.1);
        let m = ModelInput::compute(&x, 48_000, &cue()).unwrap();
        // Past the clip's end the blocks read the floor of silence.
        assert!(m.mel_at(1, 100, 20) < masked() + 0.01);
        assert!(m.mel_at(1, 2, 20) > masked() + 1.0);
        let silent = vec![0.0f32; 48_000];
        assert_eq!(
            ModelInput::compute(&silent, 48_000, &cue()).unwrap_err(),
            FeatureError::Silent
        );
        assert_eq!(
            ModelInput::compute(&x, 7_999, &cue()).unwrap_err().code(),
            "features_sample_rate"
        );
        let high = Cue { key: 128, ..cue() };
        assert_eq!(
            ModelInput::compute(&x, 48_000, &high).unwrap_err().code(),
            "features_key"
        );
        let mut bad = x.clone();
        bad[10] = f32::NAN;
        assert_eq!(
            ModelInput::compute(&bad, 48_000, &cue()).unwrap_err(),
            FeatureError::NonFinite
        );
    }

    #[test]
    fn the_level_is_normalised_and_the_context_reads_the_cue() {
        let x = pluck(48_000, 220.0, 0.005, 1.0);
        let quiet: Vec<f32> = x.iter().map(|v| v * 0.1).collect();
        let a = ModelInput::compute(&x, 48_000, &cue()).unwrap();
        let b = ModelInput::compute(&quiet, 48_000, &cue()).unwrap();
        let worst = a
            .mel
            .iter()
            .zip(&b.mel)
            .map(|(p, q)| (p - q).abs())
            .fold(0.0f32, f32::max);
        assert!(worst < 1e-3, "{worst}");
        assert_eq!(a.context[0], -3.0);
        assert!(a.context[1].abs() < 1e-5, "{}", a.context[1]);
        assert_eq!(a.context[2], 1.0);
        assert_eq!(a.context[3], 0.8);
        assert_eq!(a.context[4], 0.0);
        let unpitched = Cue {
            f0_hz: None,
            hold_s: 30.0,
            ..cue()
        };
        let c = ModelInput::compute(&x, 48_000, &unpitched).unwrap();
        assert_eq!(&c.context[1..4], &[0.0, 0.0, 4.0]);
        let sharp = Cue {
            f0_hz: Some(220.0 * libm::pow(2.0, 0.3 / 12.0)),
            ..cue()
        };
        let d = ModelInput::compute(&x, 48_000, &sharp).unwrap();
        assert!((d.context[1] - 0.3).abs() < 1e-5);
    }

    /// No input value is subnormal, so a kernel reading them behaves the same with FTZ or DAZ on; padded silence and
    /// masked bands hold the analysis's floor exactly.
    #[test]
    fn no_value_is_subnormal_and_the_floor_is_the_analysiss() {
        for (x, sr) in [
            (pluck(48_000, 220.0, 0.005, 0.1), 48_000),
            (pluck(16_000, 220.0, 0.005, 1.0), 16_000),
            (noise(44_100, 0.0, 2.5), 44_100),
        ] {
            let m = ModelInput::compute(&x, sr, &cue()).unwrap();
            assert!(m.mel.iter().chain(&m.context).all(|v| !v.is_subnormal()));
        }
        assert_eq!(masked(), libm::logf(0.0 + 1e-5));
        let m = ModelInput::compute(&pluck(48_000, 440.0, 0.005, 0.1), 48_000, &cue()).unwrap();
        assert_eq!(
            m.mel_at(0, 127, 30),
            masked(),
            "padded silence is the floor"
        );
    }
}
