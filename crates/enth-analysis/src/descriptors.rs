//! The descriptor vocabulary behind `enth describe` and `enth check`, and the [`Describer`] that measures it.
//!
//! Every descriptor is a plain, documented measurement of the float render (never of a clamped file and never read
//! off the patch). Each [`Feature`] has a canonical path (`envelope.attack_ms`), a short alias (`attack`), a unit
//! and a one-line meaning. A descriptor that cannot be measured is `None` with a reason
//! ([`Feature::missing_reason`]), never a made-up number.
//!
//! What is measured, in order (`docs/dsp/analysis.md` has the details and the measured accuracy):
//!
//! | descriptor | how |
//! | --- | --- |
//! | level | `peak_dbfs`, `true_peak_dbtp`, `clipped_pct`, `clipping`, LUFS ([`crate::loudness`]) |
//! | pitch | YIN, frame 2048, hop 512, median over voiced frames within 40 dB of the envelope peak |
//! | beating, tremolo | the multi-harmonic estimator ([`crate::beat`]) over the held part |
//! | envelope | 8 ms RMS at 1 ms with the beat fitted out ([`crate::envelope`]) |
//! | brightness | magnitude-weighted spectral centroid, STFT 2048 / hop 512, frames within 50 dB of the loudest |
//! | noisiness | 8192-point Welch spectrum of the held part; share of 40 Hz–16 kHz power outside spectral peaks |

use crate::beat::{self, BeatCfg, Beating, Tremolo};
use crate::envelope::{self, Envelope};
use crate::fft::{FftPath, FftPlanner, hann};
use crate::linalg::median;
use crate::loudness::{Level, LoudnessMeter};
use crate::pitch::{self, Pitch, Yin};
use crate::units::Unit;
use realfft::num_complex::Complex32;
use serde::{Deserialize, Serialize};

const STFT_N: usize = 2048;
const STFT_HOP: usize = 512;
const WELCH_N: usize = 8192;
const YIN_FRAME: usize = 2048;
const YIN_HOP: usize = 512;
/// Envelope level that counts as audible relative to the peak (amplitude), 40 dB.
const AUDIBLE: f64 = 0.01;
/// Frames this far below the loudest are ignored in the centroid (power), 50 dB.
const SPECTRUM_FLOOR: f64 = 1e-5;
/// The beating stretch ends where the level stays this far below the peak (amplitude), 30 dB.
const BEAT_FLOOR: f64 = 0.031_62;
/// Silence gate: sample peak below −80 dBFS.
const SILENT_PEAK: f32 = 1e-4;
/// Shortest signal `describe` accepts (ms).
pub const MIN_DURATION_MS: f64 = 20.0;

/// When the note was played, in seconds from the start of the clip.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct NoteTiming {
    pub on_s: f64,
    /// `None` when unknown (a WAV without its performance): decay, sustain and release are then `None`.
    pub off_s: Option<f64>,
}

impl NoteTiming {
    /// A note from time 0, held for `hold_s`.
    #[must_use]
    pub fn held(hold_s: f64) -> Self {
        Self {
            on_s: 0.0,
            off_s: Some(hold_s),
        }
    }

    /// A note from `on_s`, held for `hold_s`.
    #[must_use]
    pub fn with_hold(on_s: f64, hold_s: f64) -> Self {
        Self {
            on_s,
            off_s: Some(on_s + hold_s),
        }
    }

    /// A note from time 0 whose note-off is unknown.
    #[must_use]
    pub fn unknown_off() -> Self {
        Self {
            on_s: 0.0,
            off_s: None,
        }
    }
}

/// Which optional groups to compute. Pitch, beating and the envelope are always measured (the envelope needs the
/// beat rate, the beat estimator needs the pitch); spectrum, noisiness and loudness can be skipped to save time in a
/// hint search. Without `loudness`, `lufs_i` is `None` and `true_peak_dbtp` equals the sample peak; `peak_dbfs`,
/// `clipped_pct` and `clipping` are always exact.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Want {
    pub spectrum: bool,
    pub noise: bool,
    pub loudness: bool,
}

impl Want {
    pub const ALL: Want = Want {
        spectrum: true,
        noise: true,
        loudness: true,
    };
    pub const CORE: Want = Want {
        spectrum: false,
        noise: false,
        loudness: false,
    };

    /// Groups a set of features needs.
    #[must_use]
    pub fn for_features(features: &[Feature]) -> Want {
        let mut w = Want::CORE;
        for f in features {
            match f {
                Feature::Brightness => w.spectrum = true,
                Feature::Noisiness => w.noise = true,
                Feature::Loudness | Feature::TruePeak => w.loudness = true,
                _ => {}
            }
        }
        w
    }
}

/// Brightness.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Spectrum {
    pub centroid_hz: f64,
    /// Centroid over the first, middle and last third of the sound (reveals filter envelopes).
    pub centroid_track_hz: Option<[f64; 3]>,
}

/// Amplitude modulation that is not detune beating.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ModulationDesc {
    /// A tremolo: every harmonic's level moves at the same rate.
    pub amp_lfo: Option<Tremolo>,
}

/// All descriptors of one render.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Descriptors {
    pub dur_s: f64,
    /// Sample peak below −80 dBFS: only `loudness` is filled.
    pub silent: bool,
    pub pitch: Option<Pitch>,
    pub envelope: Option<Envelope>,
    pub spectrum: Option<Spectrum>,
    pub noise_ratio: Option<f64>,
    /// Detune beating (`detune.beating_hz`, `detune.depth_db`); `None` when absent or outside 0.15–25 Hz.
    pub detune: Option<Beating>,
    pub modulation: ModulationDesc,
    pub loudness: Level,
}

/// Why `describe` refused a signal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AnalysisError {
    Empty,
    /// Shorter than [`MIN_DURATION_MS`]: too short for an envelope.
    TooShort,
    /// The first non-finite sample.
    NonFinite {
        index: usize,
    },
    BadTiming(&'static str),
}

impl std::fmt::Display for AnalysisError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AnalysisError::Empty => write!(f, "the signal is empty"),
            AnalysisError::TooShort => write!(f, "the signal is shorter than {MIN_DURATION_MS} ms"),
            AnalysisError::NonFinite { index } => write!(f, "sample {index} is not finite"),
            AnalysisError::BadTiming(why) => write!(f, "bad note timing: {why}"),
        }
    }
}

impl std::error::Error for AnalysisError {}

/// Measures [`Descriptors`]. Holds FFT plans, windows and a loudness meter; one per worker thread.
pub struct Describer {
    sr: f64,
    planner: FftPlanner,
    yin: Yin,
    meter: LoudnessMeter,
    win_stft: Vec<f32>,
    win_welch: Vec<f32>,
    beat_cfg: BeatCfg,
}

impl std::fmt::Debug for Describer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Describer")
            .field("sr", &self.sr)
            .field("fft", &self.planner.path())
            .finish_non_exhaustive()
    }
}

impl Describer {
    /// A describer on the scalar FFT path (identical results on every platform).
    ///
    /// # Panics
    /// If `sample_rate` is not a positive whole number of Hz below 192 kHz.
    #[must_use]
    pub fn new(sample_rate: f64) -> Self {
        Self::with_path(sample_rate, FftPath::Scalar)
    }

    /// # Panics
    /// If `sample_rate` is not a positive whole number of Hz below 192 kHz.
    #[must_use]
    pub fn with_path(sample_rate: f64, path: FftPath) -> Self {
        assert!(
            (8000.0..192_000.0).contains(&sample_rate),
            "sample rate {sample_rate}"
        );
        let mut planner = FftPlanner::new(path);
        let yin = Yin::new(&mut planner, sample_rate, YIN_FRAME, 50.0, 2000.0);
        Self {
            sr: sample_rate,
            planner,
            yin,
            meter: LoudnessMeter::new(sample_rate as u32),
            win_stft: hann(STFT_N),
            win_welch: hann(WELCH_N),
            beat_cfg: BeatCfg::default(),
        }
    }

    #[must_use]
    pub fn sample_rate(&self) -> f64 {
        self.sr
    }

    #[must_use]
    pub fn path(&self) -> FftPath {
        self.planner.path()
    }

    /// Measure everything.
    pub fn describe(
        &mut self,
        x: &[f32],
        timing: NoteTiming,
    ) -> Result<Descriptors, AnalysisError> {
        self.describe_some(x, timing, Want::ALL)
    }

    /// Measure the core descriptors plus the groups in `want`.
    pub fn describe_some(
        &mut self,
        x: &[f32],
        timing: NoteTiming,
        want: Want,
    ) -> Result<Descriptors, AnalysisError> {
        if x.is_empty() {
            return Err(AnalysisError::Empty);
        }
        if let Some(index) = x.iter().position(|s| !s.is_finite()) {
            return Err(AnalysisError::NonFinite { index });
        }
        if (x.len() as f64) < self.sr * MIN_DURATION_MS / 1000.0 {
            return Err(AnalysisError::TooShort);
        }
        if !timing.on_s.is_finite() || timing.on_s < 0.0 {
            return Err(AnalysisError::BadTiming(
                "note-on must be a finite time at or after 0",
            ));
        }
        if let Some(off) = timing.off_s
            && (!off.is_finite() || off < timing.on_s)
        {
            return Err(AnalysisError::BadTiming(
                "note-off must be finite and not before note-on",
            ));
        }
        let sr = self.sr;
        let mut loudness = if want.loudness {
            self.meter.level(x)
        } else {
            let peak = x.iter().fold(0.0f32, |m, s| m.max(s.abs()));
            let over = x.iter().filter(|s| s.abs() >= 1.0).count();
            Level {
                lufs_i: None,
                peak_dbfs: crate::loudness::db20(f64::from(peak)),
                true_peak_dbtp: crate::loudness::db20(f64::from(peak)),
                clipped_pct: 100.0 * over as f64 / x.len() as f64,
                clipping: over > 0,
            }
        };
        let mut d = Descriptors {
            dur_s: x.len() as f64 / sr,
            silent: false,
            pitch: None,
            envelope: None,
            spectrum: None,
            noise_ratio: None,
            detune: None,
            modulation: ModulationDesc::default(),
            loudness,
        };
        let peak_sample = x.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        let (env, hop) = envelope::rms_envelope(x, sr);
        if peak_sample < SILENT_PEAK || env.len() < 2 {
            d.silent = true;
            loudness.lufs_i = None;
            d.loudness = loudness;
            return Ok(d);
        }
        let hop_ms = hop as f64 / sr * 1000.0;
        let to_frame = |s: f64| (libm::round(s * 1000.0 / hop_ms) as usize).min(env.len() - 1);
        let on = to_frame(timing.on_s);
        // A note-off past the clip end stays past it (release is then unknown).
        let off = timing
            .off_s
            .map(|s| libm::round(s * 1000.0 / hop_ms) as usize);
        let peak = env.iter().copied().fold(0.0, f64::max);

        // Pitch first: the beat estimator needs it.
        d.pitch = self.pitch(x, &env, peak, hop);

        // The held part for the beat estimator: from -6 dB re peak to note-off, or to where the level stays more
        // than 30 dB down (a 400 ms gap, not a single beat null, ends it).
        let top = env.iter().position(|&e| e >= 0.5 * peak).unwrap_or(0);
        let held_end_frame = off.unwrap_or(env.len()).min(env.len());
        let mut quiet = top;
        for (i, &e) in env.iter().enumerate().take(held_end_frame).skip(top) {
            if e >= BEAT_FLOOR * peak {
                quiet = i + 1;
            } else if i - quiet > (400.0 / hop_ms) as usize {
                break;
            }
        }
        if let Some(p) = &d.pitch {
            let m = beat::estimate(
                &mut self.planner,
                x,
                sr,
                p.f0_hz,
                top * hop,
                quiet * hop,
                &self.beat_cfg,
            );
            d.detune = m.beating;
            d.modulation.amp_lfo = m.tremolo;
        }
        let e = envelope::measure(&env, hop_ms, on, off, d.detune.map(|b| b.beating_hz));
        d.envelope = Some(e);

        // Spectrum over the audible span, noisiness over the held part.
        let onset = on
            + env[on..]
                .iter()
                .position(|&v| v >= AUDIBLE * peak)
                .unwrap_or(0);
        let last = env
            .iter()
            .rposition(|&v| v >= AUDIBLE * peak)
            .unwrap_or(onset)
            .max(onset);
        if want.spectrum {
            d.spectrum = self.spectrum(x, (onset * hop, ((last + 1) * hop).min(x.len())));
        }
        if want.noise {
            let t_top = onset + (e.attack_ms / hop_ms) as usize;
            let held_end = off.map_or((last + 1) * hop, |o| o * hop).min(x.len());
            d.noise_ratio = self.noise_ratio(x, (t_top * hop, held_end));
        }
        Ok(d)
    }

    fn pitch(&mut self, x: &[f32], env: &[f64], peak: f64, hop: usize) -> Option<Pitch> {
        if x.len() < YIN_FRAME {
            return None;
        }
        let track = self.yin.track(x, YIN_HOP);
        let loud: Vec<bool> = (0..track.len())
            .map(|i| {
                let centre = (i * YIN_HOP + YIN_FRAME / 2) / hop;
                env.get(centre).is_some_and(|&e| e >= AUDIBLE * peak)
            })
            .collect();
        pitch::summarise(&track, &loud)
    }

    fn spectrum(&mut self, x: &[f32], span: (usize, usize)) -> Option<Spectrum> {
        if x.len() < STFT_N {
            return None;
        }
        let fft = self.planner.forward(STFT_N);
        let mut input = vec![0f32; STFT_N];
        let mut out = vec![Complex32::new(0.0, 0.0); fft.bins()];
        let mut scratch = fft.make_scratch();
        let frames = (x.len() - STFT_N) / STFT_HOP + 1;
        // per frame: sum of magnitudes, sum of bin × magnitude, power
        let mut stats = Vec::with_capacity(frames);
        for fi in 0..frames {
            let seg = &x[fi * STFT_HOP..fi * STFT_HOP + STFT_N];
            for ((o, s), w) in input.iter_mut().zip(seg).zip(&self.win_stft) {
                *o = s * w;
            }
            fft.forward(&mut input, &mut out, &mut scratch);
            let (mut sum, mut moment, mut power) = (0.0f64, 0.0f64, 0.0f64);
            for (k, c) in out.iter().enumerate().skip(1) {
                let p = f64::from(c.norm_sqr());
                let m = p.sqrt();
                sum += m;
                moment += k as f64 * m;
                power += p;
            }
            stats.push((sum, moment, power));
        }
        let loudest = stats.iter().map(|s| s.2).fold(0.0, f64::max);
        if loudest <= 0.0 {
            return None;
        }
        let bin_hz = self.sr / STFT_N as f64;
        let centroid = |lo: usize, hi: usize| -> Option<f64> {
            let (mut sum, mut moment) = (0.0, 0.0);
            for (fi, s) in stats.iter().enumerate() {
                let centre = fi * STFT_HOP + STFT_N / 2;
                if centre >= lo && centre < hi && s.2 >= SPECTRUM_FLOOR * loudest {
                    sum += s.0;
                    moment += s.1;
                }
            }
            (sum > 0.0).then(|| moment / sum * bin_hz)
        };
        let centroid_hz = centroid(0, usize::MAX)?;
        let third = span.1.saturating_sub(span.0) / 3;
        let centroid_track_hz = (third > STFT_HOP)
            .then(|| {
                match [
                    centroid(span.0, span.0 + third),
                    centroid(span.0 + third, span.0 + 2 * third),
                    centroid(span.0 + 2 * third, span.1),
                ] {
                    [Some(a), Some(b), Some(c)] => Some([a, b, c]),
                    _ => None,
                }
            })
            .flatten();
        Some(Spectrum {
            centroid_hz,
            centroid_track_hz,
        })
    }

    /// Share of power outside spectral peaks. A peak is a local maximum 15 dB above the median of its ±64 bins; the
    /// peak and 3 bins either side count as tonal.
    fn noise_ratio(&mut self, x: &[f32], seg: (usize, usize)) -> Option<f64> {
        let end = seg.1.min(x.len());
        if end < WELCH_N {
            return None;
        }
        let start = seg.0.min(end - WELCH_N);
        let fft = self.planner.forward(WELCH_N);
        let mut input = vec![0f32; WELCH_N];
        let mut out = vec![Complex32::new(0.0, 0.0); fft.bins()];
        let mut scratch = fft.make_scratch();
        let nb = WELCH_N / 2 + 1;
        let mut pw = vec![0.0f64; nb];
        let mut pos = start;
        while pos + WELCH_N <= end {
            for ((o, s), w) in input
                .iter_mut()
                .zip(&x[pos..pos + WELCH_N])
                .zip(&self.win_welch)
            {
                *o = s * w;
            }
            fft.forward(&mut input, &mut out, &mut scratch);
            for (p, c) in pw.iter_mut().zip(&out) {
                *p += f64::from(c.norm_sqr());
            }
            pos += WELCH_N / 2;
        }
        let bin_hz = self.sr / WELCH_N as f64;
        let k_lo = (libm::ceil(40.0 / bin_hz) as usize).max(4);
        let k_hi = ((16_000.0f64.min(0.45 * self.sr) / bin_hz) as usize).min(nb - 5);
        let total: f64 = pw[k_lo..=k_hi].iter().sum();
        if total <= 0.0 {
            return None;
        }
        let pmax = pw[k_lo..=k_hi].iter().copied().fold(0.0, f64::max);
        let mut tonal = vec![false; nb];
        for k in k_lo..=k_hi {
            if pw[k] > pw[k - 1] && pw[k] >= pw[k + 1] && pw[k] > 1e-9 * pmax {
                let around = &pw[k.saturating_sub(64)..=(k + 64).min(nb - 1)];
                if pw[k] >= 31.6 * median(around) {
                    for t in &mut tonal[k - 3..=k + 3] {
                        *t = true;
                    }
                }
            }
        }
        let noise: f64 = (k_lo..=k_hi).filter(|&k| !tonal[k]).map(|k| pw[k]).sum();
        Some((noise / total).clamp(0.0, 1.0))
    }
}

/// Is a feature measured on a log scale (Hz, ms: it moves by ratios), or on a linear one (dB, cents, ratios)?
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scale {
    Log,
    Linear,
}

/// One descriptor of the vocabulary. Expectations, hints and `describe` all use these names.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Feature {
    Pitch,
    PitchCents,
    PitchStability,
    Attack,
    Decay,
    Sustain,
    Release,
    Duration,
    Ripple,
    Brightness,
    Noisiness,
    Beating,
    BeatDepth,
    Tremolo,
    TremoloDepth,
    Loudness,
    Peak,
    TruePeak,
    ClippedPct,
    Clipping,
}

impl Feature {
    pub const ALL: [Feature; 20] = [
        Feature::Pitch,
        Feature::PitchCents,
        Feature::PitchStability,
        Feature::Attack,
        Feature::Decay,
        Feature::Sustain,
        Feature::Release,
        Feature::Duration,
        Feature::Ripple,
        Feature::Brightness,
        Feature::Noisiness,
        Feature::Beating,
        Feature::BeatDepth,
        Feature::Tremolo,
        Feature::TremoloDepth,
        Feature::Loudness,
        Feature::Peak,
        Feature::TruePeak,
        Feature::ClippedPct,
        Feature::Clipping,
    ];

    /// Canonical path, as in `describe` output.
    #[must_use]
    pub fn path(self) -> &'static str {
        match self {
            Feature::Pitch => "pitch.f0_hz",
            Feature::PitchCents => "pitch.cents_dev",
            Feature::PitchStability => "pitch.stability",
            Feature::Attack => "envelope.attack_ms",
            Feature::Decay => "envelope.decay_ms",
            Feature::Sustain => "envelope.sustain_db",
            Feature::Release => "envelope.release_ms",
            Feature::Duration => "envelope.duration_ms",
            Feature::Ripple => "envelope.ripple_db",
            Feature::Brightness => "spectrum.centroid_hz",
            Feature::Noisiness => "noise_ratio",
            Feature::Beating => "detune.beating_hz",
            Feature::BeatDepth => "detune.depth_db",
            Feature::Tremolo => "modulation.amp_lfo.hz",
            Feature::TremoloDepth => "modulation.amp_lfo.depth_db",
            Feature::Loudness => "loudness.lufs_i",
            Feature::Peak => "loudness.peak_dbfs",
            Feature::TruePeak => "loudness.true_peak_dbtp",
            Feature::ClippedPct => "loudness.clipped_pct",
            Feature::Clipping => "loudness.clipping",
        }
    }

    /// Short names accepted in expectations, the first one preferred.
    #[must_use]
    pub fn aliases(self) -> &'static [&'static str] {
        match self {
            Feature::Pitch => &["pitch", "f0", "pitch.note", "note"],
            Feature::PitchCents => &["cents", "tuning"],
            Feature::PitchStability => &["stability"],
            Feature::Attack => &["attack"],
            Feature::Decay => &["decay"],
            Feature::Sustain => &["sustain"],
            Feature::Release => &["release"],
            Feature::Duration => &["duration"],
            Feature::Ripple => &["ripple"],
            Feature::Brightness => &["brightness", "centroid"],
            Feature::Noisiness => &["noisiness", "noise"],
            Feature::Beating => &["beating", "beat", "detune.beating"],
            Feature::BeatDepth => &["beat_depth"],
            Feature::Tremolo => &["tremolo", "amp_lfo", "modulation.amp_lfo"],
            Feature::TremoloDepth => &["tremolo_depth"],
            Feature::Loudness => &["loudness", "lufs"],
            Feature::Peak => &["peak"],
            Feature::TruePeak => &["true_peak"],
            Feature::ClippedPct => &["clipped", "clipped_pct"],
            Feature::Clipping => &["clipping", "clips"],
        }
    }

    /// Look a feature up by canonical path or alias (case-insensitive).
    #[must_use]
    pub fn parse(name: &str) -> Option<Feature> {
        let t = name.trim().to_ascii_lowercase();
        Feature::ALL
            .into_iter()
            .find(|f| f.path() == t || f.aliases().contains(&t.as_str()))
    }

    #[must_use]
    pub fn unit(self) -> Unit {
        match self {
            Feature::Pitch | Feature::Brightness | Feature::Beating | Feature::Tremolo => Unit::Hz,
            Feature::PitchCents => Unit::Cents,
            Feature::PitchStability | Feature::Noisiness => Unit::Ratio,
            Feature::Attack | Feature::Decay | Feature::Release | Feature::Duration => Unit::Ms,
            Feature::Sustain | Feature::Ripple | Feature::BeatDepth | Feature::TremoloDepth => {
                Unit::Db
            }
            Feature::Loudness => Unit::Lufs,
            Feature::Peak => Unit::Dbfs,
            Feature::TruePeak => Unit::Dbtp,
            Feature::ClippedPct => Unit::Percent,
            Feature::Clipping => Unit::Bool,
        }
    }

    /// Scale quantities (Hz, ms) move by ratios: searches and margins in percent use their logarithm.
    #[must_use]
    pub fn scale(self) -> Scale {
        match self.unit() {
            Unit::Hz | Unit::Ms => Scale::Log,
            _ => Scale::Linear,
        }
    }

    /// How far past a threshold a hint should aim, in the feature's unit (a ratio for log-scale features).
    #[must_use]
    pub fn hint_margin(self) -> f64 {
        match self {
            f if f.scale() == Scale::Log => 1.1,
            Feature::Sustain
            | Feature::Peak
            | Feature::TruePeak
            | Feature::Ripple
            | Feature::BeatDepth
            | Feature::TremoloDepth => 1.5,
            Feature::Loudness => 1.0,
            Feature::Noisiness | Feature::PitchStability => 0.03,
            Feature::ClippedPct => 0.05,
            _ => 3.0,
        }
    }

    #[must_use]
    pub fn meaning(self) -> &'static str {
        match self {
            Feature::Pitch => {
                "Fundamental frequency measured from the audio (YIN, median over the loud frames); the nearest note and the offset in cents come with it"
            }
            Feature::PitchCents => {
                "Offset of the measured pitch from the nearest equal-tempered note (A4 = 440 Hz), in cents"
            }
            Feature::PitchStability => {
                "How steady the pitch is: 1 is steady, 0 means it wanders by 50 cents or more"
            }
            Feature::Attack => {
                "Time for the level to rise from near-silence (-40 dB re peak) to within 1 dB of its peak, with detune beating removed. Under 10 ms is a click; over 200 ms is a swell"
            }
            Feature::Decay => {
                "Time from the peak until the level settles near the held level (within max(1.5 dB, 20% of the sustain in dB)); 0 means no decay stage. Not the ADSR parameter"
            }
            Feature::Sustain => {
                "Level held just before note-off, in dB below the peak, with beating removed: 0 is as loud as the peak, -80 means silent"
            }
            Feature::Release => {
                "Time after note-off for the level to fall 40 dB; 0 if the sound had already gone. Extrapolated when the clip ends first"
            }
            Feature::Duration => {
                "How long the sound stays within 40 dB of its peak, from its onset. A lower bound if still audible at the clip end"
            }
            Feature::Ripple => {
                "Residual level wobble over the held part after beat removal (dB, p95 - p5); a measurement-noise hint"
            }
            Feature::Brightness => {
                "Average frequency of the sound's energy, weighted by magnitude (spectral centroid). Higher is brighter; a pure sine reads its own pitch"
            }
            Feature::Noisiness => {
                "Share of spectral energy not in steady peaks: 0 is a clean tone, 1 is noise"
            }
            Feature::Beating => {
                "Rate of the slow level wobble from detuned oscillators (multi-harmonic estimate, 0.15 to 25 Hz); absent when there is none, it is too slow for the clip, or it is a tremolo"
            }
            Feature::BeatDepth => {
                "Peak-to-trough depth of the detune beating on its deepest harmonic (dB)"
            }
            Feature::Tremolo => {
                "Rate of an amplitude LFO: a level wobble that moves every harmonic at the same rate (unlike detune beating)"
            }
            Feature::TremoloDepth => "Peak-to-trough depth of the tremolo (dB)",
            Feature::Loudness => {
                "Integrated loudness (EBU R128) of the float render. About -23 LUFS is a quiet broadcast level, -14 is loud"
            }
            Feature::Peak => {
                "Highest sample level of the float render in dBFS; above 0 means it clips"
            }
            Feature::TruePeak => {
                "Highest level between samples (4x oversampled) in dBTP; above 0 clips a converter"
            }
            Feature::ClippedPct => "Share of samples at or beyond full scale, in percent",
            Feature::Clipping => {
                "True when any sample reaches full scale. Clipping bends brightness and levels too; lower master.level first"
            }
        }
    }

    /// Why a measurement can be missing, for hint text.
    #[must_use]
    pub fn missing_reason(self) -> &'static str {
        match self {
            Feature::Sustain | Feature::Decay => {
                "it needs a known note-off, with the note held past the end of the attack plus 80 ms"
            }
            Feature::Release => "it needs a known note-off inside the clip",
            Feature::Beating | Feature::BeatDepth => {
                "no detune beating was found: it needs detuned oscillators beating between 0.15 and 25 Hz, and at least 0.5 s of steady sound within 30 dB of the peak"
            }
            Feature::Tremolo | Feature::TremoloDepth => "no amplitude LFO was found",
            Feature::Pitch | Feature::PitchCents | Feature::PitchStability => {
                "no steady pitch was found"
            }
            Feature::Brightness => "the sound is too short (under 43 ms) or silent",
            Feature::Noisiness => "the held part is too short (under 171 ms) or silent",
            Feature::Loudness => "the sound is silent (below -70 LUFS)",
            _ => "the sound is silent",
        }
    }
}

impl Descriptors {
    /// The value of `feature`, `None` when it was not measured or is absent. Booleans read 0 or 1.
    #[must_use]
    pub fn get(&self, feature: Feature) -> Option<f64> {
        let e = self.envelope.as_ref();
        let lv = &self.loudness;
        match feature {
            Feature::Pitch => self.pitch.as_ref().map(|p| p.f0_hz),
            Feature::PitchCents => self.pitch.as_ref().map(|p| p.cents_dev),
            Feature::PitchStability => self.pitch.as_ref().map(|p| p.stability),
            Feature::Attack => e.map(|e| e.attack_ms),
            Feature::Decay => e.and_then(|e| e.decay_ms),
            Feature::Sustain => e.and_then(|e| e.sustain_db),
            Feature::Release => e.and_then(|e| e.release_ms),
            Feature::Duration => e.map(|e| e.duration_ms),
            Feature::Ripple => e.map(|e| e.ripple_db),
            Feature::Brightness => self.spectrum.map(|s| s.centroid_hz),
            Feature::Noisiness => self.noise_ratio,
            Feature::Beating => self.detune.map(|b| b.beating_hz),
            Feature::BeatDepth => self.detune.map(|b| b.depth_db),
            Feature::Tremolo => self.modulation.amp_lfo.map(|t| t.hz),
            Feature::TremoloDepth => self.modulation.amp_lfo.map(|t| t.depth_db),
            Feature::Loudness => lv.lufs_i,
            Feature::Peak => Some(lv.peak_dbfs),
            Feature::TruePeak => Some(lv.true_peak_dbtp),
            Feature::ClippedPct => Some(lv.clipped_pct),
            Feature::Clipping => Some(if lv.clipping { 1.0 } else { 0.0 }),
        }
    }

    /// True when the reading of `feature` is a lower bound (the clip ended first).
    #[must_use]
    pub fn is_lower_bound(&self, feature: Feature) -> bool {
        feature == Feature::Duration && self.envelope.is_some_and(|e| e.duration_truncated)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_signals::{SR, sine, white_noise};

    #[test]
    fn every_feature_has_a_unique_name_and_a_meaning() {
        let mut names = std::collections::BTreeSet::new();
        for f in Feature::ALL {
            assert!(names.insert(f.path()), "{f:?}");
            for a in f.aliases() {
                assert!(names.insert(a), "alias {a} of {f:?} is taken");
            }
            assert_eq!(Feature::parse(f.path()), Some(f));
            assert_eq!(
                Feature::parse(&f.aliases()[0].to_ascii_uppercase()),
                Some(f)
            );
            assert!(f.meaning().len() > 20 && !f.meaning().contains('\n'));
            assert!(f.missing_reason().len() > 10);
        }
    }

    #[test]
    fn refuses_bad_input() {
        let mut d = Describer::new(SR);
        assert_eq!(
            d.describe(&[], NoteTiming::held(1.0)),
            Err(AnalysisError::Empty)
        );
        assert_eq!(
            d.describe(&[0.5; 900], NoteTiming::held(1.0)),
            Err(AnalysisError::TooShort)
        );
        assert_eq!(
            d.describe(&[0.0, f32::NAN], NoteTiming::held(1.0)),
            Err(AnalysisError::NonFinite { index: 1 })
        );
        let x = sine(SR, 440.0, 0.5, 0.2);
        assert!(matches!(
            d.describe(
                &x,
                NoteTiming {
                    on_s: 0.5,
                    off_s: Some(0.1)
                }
            ),
            Err(AnalysisError::BadTiming(_))
        ));
    }

    #[test]
    fn silence_is_silent() {
        let d = Describer::new(SR)
            .describe(&vec![0.0; 48_000], NoteTiming::held(0.5))
            .unwrap();
        assert!(d.silent && d.pitch.is_none() && d.envelope.is_none());
        assert_eq!(d.get(Feature::Attack), None);
        assert_eq!(d.get(Feature::Clipping), Some(0.0));
    }

    #[test]
    fn a_sine_reads_its_pitch_brightness_and_level() {
        let d = Describer::new(SR)
            .describe(&sine(SR, 440.0, 0.5, 1.0), NoteTiming::unknown_off())
            .unwrap();
        let p = d.pitch.as_ref().unwrap();
        assert!(
            p.cents_dev.abs() < 3.0 && p.note == "A4" && p.stability > 0.98,
            "{p:?}"
        );
        assert!(
            (d.get(Feature::Brightness).unwrap() - 440.0).abs() < 25.0,
            "{d:?}"
        );
        assert!(d.noise_ratio.unwrap() < 0.01, "{d:?}");
        assert!((d.loudness.peak_dbfs + 6.02).abs() < 0.01);
        assert!(d.detune.is_none() && d.modulation.amp_lfo.is_none());
        assert_eq!(d.get(Feature::Sustain), None);
    }

    #[test]
    fn centroid_of_two_known_sines() {
        // magnitude-weighted: equal amplitudes centre at 2 kHz, 1 : 0.5 at 1667 Hz
        let mut d = Describer::new(SR);
        for (b, want) in [(0.25, 2000.0), (0.125, 1666.7)] {
            let x =
                crate::test_signals::mix(&sine(SR, 1000.0, 0.25, 0.5), &sine(SR, 3000.0, b, 0.5));
            let c = d
                .describe(&x, NoteTiming::unknown_off())
                .unwrap()
                .get(Feature::Brightness)
                .unwrap();
            assert!((c - want).abs() < 30.0, "{b}: {c}");
        }
    }

    #[test]
    fn noise_is_noisy() {
        let d = Describer::new(SR)
            .describe(&white_noise(SR, 0.3, 1.0, 9), NoteTiming::unknown_off())
            .unwrap();
        assert!(d.noise_ratio.unwrap() > 0.9, "{d:?}");
        let c = d.get(Feature::Brightness).unwrap();
        assert!((9_000.0..15_000.0).contains(&c), "{c}");
    }
}
