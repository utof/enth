//! **Owner: lane OBJ** (the robust objective, R3a wave 2, after FE). Pre-declared by lane S0s.
//!
//! **The degradation-aware distance** (docs/r3.md, "The robust objective"), metric id [`METRIC_ROBUST`]: metric v1.1
//! ([`pooled`](crate::pooled), `logmel-pool85-l1/1`) compared only where the intake ([`intake`](crate::intake)) says
//! the upload still carries information, so a create search stops paying knobs to reproduce noise, a codec's low-pass
//! or clipping. [`distance`](crate::distance) (v1), [`pooled`](crate::pooled) (v1.1) and `sound/1` are untouched, and
//! every score already pinned stays valid. It is what "create from sound" minimises on an upload; `sound/1` stays the
//! judge.
//!
//! **Definition** (the [`Settings::default`] of this id). Both clips are metric v1.1's: loudness-normalised to
//! −23 LUFS, the mel magnitudes pooled over ~85 ms blocks at the three resolutions, then the natural log. Given the
//! upload's [`Damage`] (from FE's report, [`Damage::from_report`]):
//!
//! 1. **The band.** With a detected `bandwidth_hz`, only the mel bands whose upper edge lies at or below
//!    `min(bandwidth_hz, sample_rate / 2)` are compared, and the candidate is level-aligned to the target over them
//!    (half the log ratio of their summed squared magnitudes, per resolution), so the energy a codec removed above the
//!    cut neither pulls the candidate's level in the bands it kept nor rewards a candidate for imitating the cut.
//! 2. **The floor** (a *confirmed* floor only, FE decision 7: a held noise-like sound reads as an unconfirmed floor).
//!    Per resolution and band, the noise is read off the target itself (minimum statistics: noise is stationary, so a
//!    band's quietest pooled block is noise wherever the clip holds a noise-only stretch, and the sound's quietest part
//!    where it does not), and the spread of a stationary noise's pooled band level is known ([`NOISE_SPREAD`]):
//!    - the candidate is compared **as if recorded in the same noise**: the noise's mean level (the quietest block
//!      plus [`Settings::noise_sigmas`] spreads) is added to it in power, so where sound and noise overlap the upload's
//!      sum is matched by the sound alone, and a candidate that adds noise of its own (a noise module, a long release,
//!      a reverb tail) is charged twice for it;
//!    - both are then **clamped** from below at the floor (the quietest block plus [`Settings::floor_sigmas`]
//!      spreads, above almost every block of the noise), so what lies under the noise is neither paid nor charged.
//!
//!    Hum is stationary too, so its harmonics fall in the floor and are handled the same way.
//! 3. **Clipped spans** (hard clipping only, FE decision 7: soft clipping and limiting leave no plateau and are never
//!    inferred from `clipping`): the pooled blocks that overlap a clipped span weigh [`Settings::clip_weight`] of the
//!    others.
//! 4. **The silence after the sound.** When the clip ends because the sound fell into the floor (not at the length
//!    cap, the file's end or the scored window), the comparison runs [`Settings::tail_s`] past the clip's end, within
//!    the scored window: the candidate is rendered that much longer, and its extra blocks are compared, clamped, with
//!    what the upload holds there: the confirmed floor, or silence (60 dB under the clip's loudest cell, the intake's
//!    own silence threshold, or a band's quietest block when that is lower). A candidate that rings on after the upload
//!    went quiet is charged for it, where v1.1 stopped looking 100 ms after the signal's end (the intake's tail).
//! 5. **A late slow attack** (a confirmed floor only): a slow attack under noise is detected only as it clears the
//!    noise, 5–35 ms late (FE's measurement, F1s should-fix 6). [`attack_lead`] reads how long before the note-on the
//!    rise began, and the candidate's note starts that much earlier ([`RobustTarget::lead_s`]), so the search does not
//!    shorten the attack to catch up.
//!
//! The distance is the mean over the resolutions of the weighted mean absolute difference over the compared cells.
//! With no damage and no silence after the sound it equals metric v1.1 exactly (a test pins it). [`SILENT`] when
//! either clip is silent. Renders (5 and 4's longer render) are the caller's: `enth-solver`'s `robust` module renders
//! and scores a candidate, and is the create objective a shell passes.
//!
//! **Not treated as damage:** room reverb and compression, which the patch can make itself (the degradation
//! policy: the K candidates include a dry patch and a wet or compressed one, and the user picks by ear), soft
//! clipping and limiting (no plateau to find), and device EQ (no detector yet). The intake's `tail_t60_s` is not a
//! reverb detector and is not used.
//!
//! Pure, `libm` only (`exp`, `log`, `log10`, `pow`, `sqrt`), sums in a fixed order: the same bits on every platform
//! for the same [`Features`](crate::features::Features).

use crate::distance::SILENT;
use crate::features::{BANDS, FFT_SIZES, FeatureCfg, FeatureExtractor, MEL_RANGE_HZ};
use crate::intake::IntakeReport;
use crate::pooled::{self, POOL_FRAMES, Pooled};

/// Stable id of the robust distance. A change to its definition (or to [`Settings::default`]) gets a new id.
pub const METRIC_ROBUST: &str = "logmel-pool85-robust/1";

/// What the upload no longer carries faithfully, in the scored clip's time (seconds from its first sample), as the
/// intake found it. [`Damage::default`] is a clean upload, compared as metric v1.1.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Damage {
    /// The cut-off of a steep low-pass (Hz), or `None` for the full band.
    pub bandwidth_hz: Option<f64>,
    /// The noise floor (RMS dBFS of the clip as delivered), **confirmed only**; `None` otherwise.
    pub noise_floor_dbfs: Option<f64>,
    /// Hard-clipped spans `(start_s, end_s)`.
    pub clipped_s: Vec<(f64, f64)>,
    /// The clip ends because the sound fell into the floor (the intake's tail was kept, nothing capped it): what
    /// would follow is floor.
    pub ends_in_floor: bool,
    /// The note-on in clip time (the intake's `perf.on_s`), where the attack was detected; `None` when unknown.
    pub onset_s: Option<f64>,
}

impl Damage {
    /// The damage the intake reported (FE's [`IntakeReport`]): the bandwidth, the floor only when
    /// `floor_confirmed`, the hard-clipped spans, and whether the clip ends in the floor. The scored window is the
    /// caller's: a window that cuts the clip short (`create::Target::from_intake_scored`) must clear
    /// [`ends_in_floor`](Self::ends_in_floor) (see [`Damage::windowed`]).
    #[must_use]
    pub fn from_report(report: &IntakeReport) -> Damage {
        let d = &report.degradation;
        let s = &report.span;
        // The intake keeps `tail_ms` after the signal's end unless the file ends first or the cap cuts it; a clip
        // padded to the minimum length ends in silence.
        let tail_kept = s.end_s - s.signal_end_s >= 0.5 * report.cfg.tail_ms / 1000.0;
        Damage {
            bandwidth_hz: d.bandwidth_hz.filter(|b| b.is_finite() && *b > 0.0),
            noise_floor_dbfs: d
                .noise_floor_dbfs
                .filter(|f| d.floor_confirmed && f.is_finite()),
            clipped_s: d
                .clipping
                .as_ref()
                .map(|c| c.spans.iter().map(|sp| (sp.start_s, sp.end_s)).collect())
                .unwrap_or_default(),
            ends_in_floor: !s.capped && (tail_kept || s.tail_pad_s > 0.0),
            onset_s: Some(report.perf.on_s).filter(|t| t.is_finite() && *t >= 0.0),
        }
    }

    /// The same damage seen through a scored window of `window_s` of a clip `clip_s` long: when the window cuts the
    /// clip, the sound has not ended inside it.
    #[must_use]
    pub fn windowed(mut self, window_s: f64, clip_s: f64) -> Damage {
        if window_s + 0.5e-3 < clip_s {
            self.ends_in_floor = false;
        }
        self
    }
}

/// How the distance treats each kind of damage. [`Settings::default`] is [`METRIC_ROBUST`]'s definition; any other
/// value is an experiment with no id of its own (lane OBJ's dev harness compares them).
#[derive(Clone, Copy, Debug, PartialEq)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "each flag switches one kind of damage on or off, independently, for the dev comparisons"
)]
pub struct Settings {
    /// Compare only the bands under a detected cut-off.
    pub band_mask: bool,
    /// Level-align the candidate over the compared bands when some are masked.
    pub level_align: bool,
    /// Clamp both clips at a confirmed noise floor.
    pub floor_clamp: bool,
    /// How many standard deviations of a stationary noise's pooled band level ([`NOISE_SPREAD`]) the floor sits
    /// above the target's quietest block: the quietest of a clip's blocks lies about two under the noise's mean, so
    /// four put the floor about two above it, where almost no block of the noise reaches.
    pub floor_sigmas: f64,
    /// Nepers added to the floor besides (0 by default).
    pub floor_offset: f64,
    /// Add the noise's estimated mean band level to the candidate (in power) before the clamp, so a candidate is
    /// compared with the upload as if it had been recorded in the same noise: where the sound and the noise overlap,
    /// the upload reads their sum, which a clean candidate alone would fall short of.
    pub noise_add: bool,
    /// Where that mean sits above the quietest block, in the same standard deviations as `floor_sigmas` (the
    /// quietest of 20 to 40 blocks lies about two under the mean).
    pub noise_sigmas: f64,
    /// The weight of a pooled block that overlaps a hard-clipped span (1 = no change).
    pub clip_weight: f64,
    /// Seconds compared past the clip's end when it ends in the floor (0 = off), at most up to `max_end_s`.
    pub tail_s: f64,
    /// Start the candidate's note earlier when a slow attack surfaced late from a confirmed floor
    /// ([`RobustTarget::lead_s`]).
    pub attack_lead: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            band_mask: true,
            level_align: true,
            floor_clamp: true,
            floor_sigmas: 4.0,
            floor_offset: 0.0,
            noise_add: true,
            noise_sigmas: 2.0,
            clip_weight: 0.25,
            tail_s: 0.5,
            attack_lead: true,
        }
    }
}

impl Settings {
    /// Every kind of damage ignored: metric v1.1, for comparisons.
    #[must_use]
    pub fn off() -> Settings {
        Settings {
            band_mask: false,
            level_align: false,
            floor_clamp: false,
            floor_sigmas: 0.0,
            floor_offset: 0.0,
            noise_add: false,
            noise_sigmas: 0.0,
            clip_weight: 1.0,
            tail_s: 0.0,
            attack_lead: false,
        }
    }

    /// Settings from `key=value` pairs separated by commas, over [`Settings::default`] (`off` first starts from
    /// [`Settings::off`]): `mask`, `align`, `floor`, `nadd`, `lead` (0 or 1), `fsig`, `foff`, `nsig`, `clipw`, `tail` (numbers). For lane
    /// OBJ's dev harness; `None` for an unknown key or a bad value.
    #[doc(hidden)]
    #[must_use]
    pub fn parse(text: &str) -> Option<Settings> {
        let mut s = Settings::default();
        for part in text.split(',').map(str::trim).filter(|p| !p.is_empty()) {
            if part == "off" {
                s = Settings::off();
                continue;
            }
            let (k, v) = part.split_once('=')?;
            let x: f64 = v.parse().ok()?;
            let on = x != 0.0;
            match k {
                "mask" => s.band_mask = on,
                "align" => s.level_align = on,
                "floor" => s.floor_clamp = on,
                "foff" => s.floor_offset = x,
                "fsig" => s.floor_sigmas = x,
                "nadd" => s.noise_add = on,
                "nsig" => s.noise_sigmas = x,
                "clipw" => s.clip_weight = x,
                "tail" => s.tail_s = x,
                "lead" => s.attack_lead = on,
                _ => return None,
            }
        }
        Some(s)
    }
}

/// The mel bands' upper edges (Hz), as [`crate::features`] builds its filter bank.
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

/// The block-to-block standard deviation (nepers) of a stationary noise's pooled log-mel is about this over
/// `sqrt(bins · frames)`, where `bins` is a band's effective number of FFT bins (`(Σw)² / Σw²` of its triangular
/// weights) and `frames` the frames a block pools ([`POOL_FRAMES`]). Measured on 20 s of white and pink noise at
/// 48 kHz: 0.67–0.73 by resolution (median over the bands), 0.5–0.78 over single bands; the larger value is kept.
pub const NOISE_SPREAD: f64 = 0.7;

/// Per resolution and band, the standard deviation of a stationary noise's pooled log-mel ([`NOISE_SPREAD`]) at
/// `sample_rate`, from the same triangular mel weights as [`crate::features`]' filter bank.
fn noise_sigmas(sample_rate: f64) -> Vec<[f64; BANDS]> {
    let mel = |f: f64| 2595.0 * libm::log10(1.0 + f / 700.0);
    let hz = |m: f64| 700.0 * (libm::pow(10.0, m / 2595.0) - 1.0);
    let (lo, hi) = (mel(MEL_RANGE_HZ.0), mel(MEL_RANGE_HZ.1));
    let pts: Vec<f64> = (0..BANDS + 2)
        .map(|i| hz(lo + (hi - lo) * i as f64 / (BANDS + 1) as f64))
        .collect();
    FFT_SIZES
        .iter()
        .zip(POOL_FRAMES)
        .map(|(&n_fft, frames)| {
            let bin_hz = sample_rate / n_fft as f64;
            let mut out = [0.0; BANDS];
            for (b, o) in out.iter_mut().enumerate() {
                let (l, c, r) = (pts[b], pts[b + 1], pts[b + 2]);
                let (mut s1, mut s2) = (0.0f64, 0.0f64);
                for k in 0..=n_fft / 2 {
                    let f = k as f64 * bin_hz;
                    let v = if f > l && f < c {
                        (f - l) / (c - l)
                    } else if f >= c && f < r {
                        (r - f) / (r - c)
                    } else {
                        0.0
                    };
                    s1 += v;
                    s2 += v * v;
                }
                let bins = if s2 > 0.0 {
                    (s1 * s1 / s2).max(1.0)
                } else {
                    1.0
                };
                *o = NOISE_SPREAD / libm::sqrt(bins * frames as f64);
            }
            out
        })
        .collect()
}

/// One pooled block's span in samples at resolution `r`: `[start, end)`.
fn block_span(r: usize, b: usize, hop_div: usize) -> (usize, usize) {
    let n = FFT_SIZES[r];
    let hop = (n / hop_div.max(1)).max(1);
    let w = POOL_FRAMES[r];
    (b * w * hop, (b * w + w - 1) * hop + n)
}

/// A target prepared for the robust distance: its pooled log-mel and what its [`Damage`] means per band and block.
/// Immutable and shareable between workers; compute the candidates' features with the same [`FeatureCfg`].
#[derive(Clone, Debug)]
pub struct RobustTarget {
    settings: Settings,
    pooled: Pooled,
    /// Per band: compared.
    keep: [bool; BANDS],
    masked: bool,
    /// Per resolution, `BANDS` floor values (natural log), when the floor clamps the whole comparison.
    floor: Option<Vec<Vec<f64>>>,
    /// Per resolution, `BANDS` doubled noise levels (`2 ln`, so `exp` of it is the power) added to the candidate.
    noise2: Option<Vec<Vec<f64>>>,
    /// Per resolution, `BANDS` values the target reads past its end (the confirmed floor, or silence).
    after: Vec<Vec<f64>>,
    /// Per resolution: whether each target block overlaps a clipped span.
    clipped: Vec<Vec<bool>>,
    /// Blocks compared per resolution past the target's end (0 unless the clip ends in the floor).
    extra_blocks: Vec<usize>,
    /// Samples the candidate must render past the clip for those blocks.
    extra_samples: usize,
    /// The clip's length in samples (the candidate's loudness is measured over it).
    clip_len: usize,
    /// How much earlier (s) the candidate's note starts than the clip's note-on.
    lead_s: f64,
}

impl RobustTarget {
    /// Prepare the target `audio` (the scored clip at `fx`'s rate) with its `damage` and [`METRIC_ROBUST`]'s
    /// definition. `max_end_s` bounds how far past the clip the silence after the sound is compared (the scored
    /// window, `create::SCORE_S`).
    pub fn new(
        fx: &mut FeatureExtractor,
        audio: &[f32],
        damage: &Damage,
        max_end_s: f64,
    ) -> RobustTarget {
        RobustTarget::with_settings(fx, audio, damage, max_end_s, &Settings::default())
    }

    /// [`RobustTarget::new`] with other [`Settings`] (experiments; no metric id).
    #[doc(hidden)]
    pub fn with_settings(
        fx: &mut FeatureExtractor,
        audio: &[f32],
        damage: &Damage,
        max_end_s: f64,
        settings: &Settings,
    ) -> RobustTarget {
        let cfg = *fx.cfg();
        let features = fx.extract(audio);
        let pooled = pooled::pool(&features);
        let sr = cfg.sample_rate;
        let nyquist = 0.5 * sr;
        let mut keep = [true; BANDS];
        let mut masked = false;
        if settings.band_mask
            && let Some(bw) = damage.bandwidth_hz
        {
            let top = bw.min(nyquist);
            for (k, e) in band_upper_edges().iter().enumerate() {
                if *e > top {
                    keep[k] = false;
                    masked = true;
                }
            }
            if keep.iter().all(|k| !k) {
                // a cut under the lowest band's top: compare the lowest band rather than nothing
                keep[0] = true;
            }
        }
        // Per resolution and band, the quietest block: the noise floor when the clip holds noise-only blocks.
        let quietest: Vec<Vec<f64>> = pooled
            .logmel
            .iter()
            .map(|lm| {
                let blocks = lm.len() / BANDS;
                (0..BANDS)
                    .map(|k| {
                        (0..blocks)
                            .map(|b| lm[b * BANDS + k])
                            .fold(f64::INFINITY, f64::min)
                    })
                    .map(|v| if v.is_finite() { v } else { LOG_FLOOR })
                    .collect()
            })
            .collect();
        let (floor, noise2) = match damage.noise_floor_dbfs {
            // the floor's level in dBFS is not needed: the noise is read off the clip itself
            Some(_) if settings.floor_clamp && !pooled.silent => {
                let sigmas = noise_sigmas(sr);
                let level = |sig_count: f64, extra: f64| -> Vec<Vec<f64>> {
                    quietest
                        .iter()
                        .zip(&sigmas)
                        .map(|(r, sig)| {
                            r.iter()
                                .zip(sig)
                                .map(|(v, s)| v + sig_count * s + extra)
                                .collect()
                        })
                        .collect()
                };
                let floor = level(settings.floor_sigmas, settings.floor_offset);
                let noise2 = settings.noise_add.then(|| {
                    level(settings.noise_sigmas, 0.0)
                        .into_iter()
                        .map(|r| r.into_iter().map(|v| 2.0 * v).collect())
                        .collect()
                });
                (Some(floor), noise2)
            }
            _ => (None, None),
        };
        // Past the clip: the confirmed floor, or silence as the intake judged it (60 dB under the clip's loudest
        // cell, or the band's quietest block when that is lower: a clip of two or three blocks may not reach it).
        let after = floor.clone().unwrap_or_else(|| {
            pooled
                .logmel
                .iter()
                .zip(&quietest)
                .map(|(lm, q)| {
                    let top = lm.iter().copied().fold(LOG_FLOOR, f64::max) - SILENCE_NEPERS;
                    q.iter().map(|v| v.min(top).max(LOG_FLOOR)).collect()
                })
                .collect()
        });
        let clipped = pooled
            .logmel
            .iter()
            .enumerate()
            .map(|(r, lm)| {
                (0..lm.len() / BANDS)
                    .map(|b| {
                        let (a, e) = block_span(r, b, cfg.hop_div);
                        let (a, e) = (a as f64 / sr, e as f64 / sr);
                        damage.clipped_s.iter().any(|(s0, s1)| *s0 < e && *s1 > a)
                    })
                    .collect()
            })
            .collect();
        let clip_len = audio.len();
        let mut extra_blocks = vec![0; pooled.logmel.len()];
        let mut extra_samples = 0;
        if damage.ends_in_floor && settings.tail_s > 0.0 && !pooled.silent {
            let room = (max_end_s * sr - clip_len as f64).max(0.0);
            let want = libm::round(settings.tail_s.min(room / sr) * sr).max(0.0) as usize;
            for (r, lm) in pooled.logmel.iter().enumerate() {
                // The blocks of a render `want` samples longer that lie past the target's last block.
                let blocks_now = lm.len() / BANDS;
                let n = FFT_SIZES[r];
                let hop = (n / cfg.hop_div.max(1)).max(1);
                let len = clip_len + want;
                let frames = if len >= n { (len - n) / hop + 1 } else { 0 };
                let blocks = frames / POOL_FRAMES[r];
                extra_blocks[r] = blocks.saturating_sub(blocks_now);
            }
            if extra_blocks.iter().any(|b| *b > 0) {
                extra_samples = want;
            } else {
                extra_blocks.fill(0);
            }
        }
        let lead_s = match (damage.noise_floor_dbfs, damage.onset_s) {
            (Some(fl), Some(on)) if settings.attack_lead && !pooled.silent => {
                attack_lead(audio, sr, on, fl)
            }
            _ => 0.0,
        };
        RobustTarget {
            settings: *settings,
            pooled,
            keep,
            masked,
            floor,
            noise2,
            after,
            clipped,
            extra_blocks,
            extra_samples,
            clip_len,
            lead_s,
        }
    }

    /// How much earlier (s) than the clip's note-on a candidate's note should start: a slow attack under a confirmed
    /// floor is detected only as it clears the noise, 5–35 ms late on attacks of 130 ms or more (FE's measurement,
    /// F1s should-fix 6), so a candidate started at the note-on would lag the upload's rise and the search would
    /// shorten its attack to catch up. 0 without a confirmed floor, for a fast attack, or when the rise cannot be read
    /// (see [`attack_lead`]).
    #[must_use]
    pub fn lead_s(&self) -> f64 {
        self.lead_s
    }

    /// How many samples past the clip a candidate must be rendered ([`Settings::tail_s`], when the clip ends in the
    /// floor; else 0). Render the candidate's performance with this much more length, then [`RobustTarget::candidate`].
    #[must_use]
    pub fn extra_samples(&self) -> usize {
        self.extra_samples
    }

    /// The target's pooled log-mel.
    #[must_use]
    pub fn pooled(&self) -> &Pooled {
        &self.pooled
    }

    /// A candidate's pooled log-mel from its render (`audio`, the clip's length plus
    /// [`extra_samples`](Self::extra_samples) or shorter), normalised by the loudness of its first clip-length samples only, so the
    /// blocks inside the clip are those of metric v1.1 and the extra ones only add the silence after the sound.
    pub fn candidate(&self, fx: &mut FeatureExtractor, audio: &[f32]) -> Pooled {
        if self.extra_samples == 0 || audio.len() <= self.clip_len {
            return pooled::pool(&fx.extract(audio));
        }
        let target_lufs = fx.cfg().target_lufs;
        let Some(lufs) = fx.meter().integrated(&audio[..self.clip_len]) else {
            return pooled::pool(&fx.extract(&audio[..self.clip_len]));
        };
        let gain = libm::pow(10.0, (target_lufs - lufs) / 20.0) as f32;
        let scaled: Vec<f32> = audio.iter().map(|v| v * gain).collect();
        let mut f = fx.extract_unnormalised(&scaled);
        f.lufs = lufs;
        pooled::pool(&f)
    }

    /// The robust distance from the target to a candidate's pooled log-mel ([`RobustTarget::candidate`]); 0 for the
    /// target itself, [`SILENT`] when either is silent.
    #[must_use]
    pub fn distance(&self, candidate: &Pooled) -> f64 {
        if self.pooled.silent || candidate.silent {
            return SILENT;
        }
        let res = self.pooled.logmel.len();
        let mut s = 0.0;
        for r in 0..res {
            let (mut acc, mut wsum) = (0.0f64, 0.0f64);
            self.cells(candidate, r, |_, _, w, d| {
                // v1.1's order and arithmetic when nothing is damaged (w = 1, offset 0, no clamp)
                acc += w * d;
                wsum += w;
            });
            s += acc / wsum.max(1.0);
        }
        s / res.max(1) as f64
    }

    /// Every compared cell of resolution `r`: `f(block, band, weight, |target − candidate|)` (diagnostics).
    #[doc(hidden)]
    pub fn cells(&self, candidate: &Pooled, r: usize, mut f: impl FnMut(usize, usize, f64, f64)) {
        let t = &self.pooled.logmel[r];
        let Some(c) = candidate.logmel.get(r) else {
            return;
        };
        let (nt, nc) = (t.len() / BANDS, c.len() / BANDS);
        let common = nt.min(nc);
        let blocks = if self.extra_blocks[r] > 0 {
            nc.min(nt + self.extra_blocks[r]).max(common)
        } else {
            common
        };
        let offset = if self.masked && self.settings.level_align {
            let (mut et, mut ec) = (0.0f64, 0.0f64);
            for b in 0..common {
                for k in 0..BANDS {
                    if self.keep[k] {
                        et += libm::exp(2.0 * t[b * BANDS + k]);
                        ec += libm::exp(2.0 * c[b * BANDS + k]);
                    }
                }
            }
            if et > 0.0 && ec > 0.0 {
                0.5 * (libm::log(et) - libm::log(ec))
            } else {
                0.0
            }
        } else {
            0.0
        };
        let floor = self.floor.as_ref().map(|f| &f[r]);
        let noise2 = self.noise2.as_ref().map(|n| &n[r]);
        let after = &self.after[r];
        for b in 0..blocks {
            let w = if b < nt && self.clipped[r][b] {
                self.settings.clip_weight
            } else {
                1.0
            };
            for k in 0..BANDS {
                if !self.keep[k] {
                    continue;
                }
                let mut cv = c[b * BANDS + k] + offset;
                if let Some(n2) = noise2 {
                    cv = 0.5 * libm::log(libm::exp(2.0 * cv) + libm::exp(n2[k]));
                }
                let (tv, cv) = if b < nt {
                    let tv = t[b * BANDS + k];
                    match floor {
                        Some(fl) => (tv.max(fl[k]), cv.max(fl[k])),
                        None => (tv, cv),
                    }
                } else {
                    (after[k], cv.max(after[k]))
                };
                f(b, k, w, (tv - cv).abs());
            }
        }
    }

    /// The floor per resolution and band, when a confirmed floor clamps the comparison (diagnostics).
    #[doc(hidden)]
    #[must_use]
    pub fn floor(&self) -> Option<&Vec<Vec<f64>>> {
        self.floor.as_ref()
    }
}

/// The longest lead [`attack_lead`] reports (s): FE measured lags of 5–35 ms.
pub const MAX_LEAD_S: f64 = 0.05;

/// How long before `on_s` (clip time) the upload's attack began, for a sound that rose out of a noise floor at
/// `floor_dbfs` (RMS dBFS of `audio`). The signal amplitude (the floor's power subtracted) of 10 ms frames every 5 ms
/// from `on_s`, while it stays 6 dB under the attack's peak (the first 0.5 s) and at most 60 ms on, is fitted by a
/// line in time (least squares); where the line meets zero is where a rise linear in amplitude began. The engine's
/// attack is near linear over its first quarter (an RC curve with a 0.3 overshoot, docs/dsp/envelope.md). 0 when the
/// rise is not there (a fast attack, which the intake places exactly: fewer than four frames under the peak), the fit
/// is poor, or the lead is under 3 ms; at most [`MAX_LEAD_S`].
#[must_use]
pub fn attack_lead(audio: &[f32], sr: f64, on_s: f64, floor_dbfs: f64) -> f64 {
    let frame = (libm::round(0.010 * sr) as usize).max(1);
    let noise = libm::pow(10.0, floor_dbfs / 10.0);
    let amp = |t: f64| -> Option<f64> {
        let a = libm::round(t * sr) as usize;
        let e = a.checked_add(frame)?;
        if e > audio.len() {
            return None;
        }
        let mut p = 0.0f64;
        for v in &audio[a..e] {
            p += f64::from(*v) * f64::from(*v);
        }
        Some(libm::sqrt((p / frame as f64 - noise).max(0.0)))
    };
    let mut peak = 0.0f64;
    let mut t = on_s;
    while t < on_s + 0.5 {
        match amp(t) {
            Some(a) => peak = peak.max(a),
            None => break,
        }
        t += 0.005;
    }
    // the frames of the rise: from the onset while under half the peak's amplitude, at most 60 ms on
    let (mut st, mut sa, mut stt, mut sta, mut n) = (0.0f64, 0.0f64, 0.0f64, 0.0f64, 0usize);
    let mut pts = Vec::new();
    for i in 0..12u32 {
        let t0 = on_s + 0.005 * f64::from(i);
        let Some(a) = amp(t0) else { break };
        if a >= 0.5 * peak {
            break;
        }
        // time of the frame's centre, from the onset
        let c = 0.005 * f64::from(i) + 0.005;
        st += c;
        sa += a;
        stt += c * c;
        sta += c * a;
        n += 1;
        pts.push((c, a));
    }
    if n < 4 {
        return 0.0;
    }
    let nf = n as f64;
    let den = nf * stt - st * st;
    let slope = (nf * sta - st * sa) / den;
    let icpt = (sa - slope * st) / nf;
    if !(slope > 0.0 && den > 0.0) {
        return 0.0;
    }
    // the fit must explain the rise (R² of at least 0.9)
    let mean = sa / nf;
    let (mut ss_res, mut ss_tot) = (0.0f64, 0.0f64);
    for (c, a) in &pts {
        let e = a - (icpt + slope * c);
        ss_res += e * e;
        ss_tot += (a - mean) * (a - mean);
    }
    if ss_tot <= 0.0 || ss_res > 0.1 * ss_tot {
        return 0.0;
    }
    let lead = icpt / slope;
    if lead.is_finite() && lead >= 0.003 {
        lead.min(MAX_LEAD_S)
    } else {
        0.0
    }
}

/// 60 dB in nepers (`ln` of a magnitude ratio): the intake's silence, `IntakeCfg::silence_db` under the loudest frame.
const SILENCE_NEPERS: f64 = 60.0 * core::f64::consts::LN_10 / 20.0;

/// `ln(1e-5)`: the log-mel of digital silence (metric v1's magnitude floor).
const LOG_FLOOR: f64 = -11.512_925_464_970_229;

/// The feature settings the robust distance uses at `sample_rate`: metric v1's (loudness-normalised, scalar FFT path).
#[must_use]
pub fn feature_cfg(sample_rate: f64) -> FeatureCfg {
    FeatureCfg {
        sample_rate,
        ..FeatureCfg::default()
    }
}

#[cfg(test)]
mod tests;
