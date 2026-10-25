//! **Owner: lane ECK.** The measuring: what a question's clips sound like under `sound/1`, in just-noticeable units.
//!
//! `sound/1` compares two patches under **one** table phrase; a music question's options differ in their notes, so
//! `ear-check` renders each clip itself and applies `sound/1`'s own analysis to it
//! ([`enth_challenge::score::sound::analyse_render`], [`clip_distance`](sound::clip_distance) and the guard: the same
//! bits the scorer takes) with the calibration `SoundV1::prepare` makes: the nuisance floor from two renders at other
//! seeds, the JND scale from the 16 jittered copies ([`calibrate::jitter_set`]), the pitch and waveform floors.
//! [`calibrate`] is that function, replayed over any clip; the anchor test (`tests.rs`) holds it to
//! `score_round`'s `jnd` on patches under a table phrase, so a change to `sound/1` that this does not follow fails there.
//!
//! **Units.** `jnd(target, heard) = max(0, d - floor) / scale + guard`, the number `sound/1` returns for an answer
//! `heard` against a target. An item's options are measured against one calibration: the right answer's clip for a
//! music item (its floor and scale are those of the sound the player hears), and both directions' smaller for a
//! parameter game's clips, as `choice_audibility` measures lesson options.
//!
//! **Phrases.** A music clip's notes are its phrase; the guard's readings and the jitter set want a [`Phrase`], whose
//! notes are `'static`, so each music clip's notes are leaked once. A tool run measures a few thousand items.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use enth_challenge::calibrate;
use enth_challenge::ear::question::Clip as Played;
use enth_challenge::model::{Phrase, PhraseNote, SCORING_SAMPLE_RATE, SCORING_SEED};
use enth_challenge::phrases;
use enth_challenge::score::sound;
use enth_music::ear::{EqBand, EqKind};
use enth_patch::Patch;
use enth_patch::canon::{PatchHash, exact_hash};
use enth_render::{Performance, RenderSpec, render};

/// What a clip plays through: its patch, the performance, the phrase `sound/1` reads its readings from, and the band
/// applied after the engine (Frequency finder's peak or shelf).
#[derive(Clone)]
pub struct Playable {
    /// The patch, as the clip plays it.
    pub patch: Patch,
    /// The performance of the notes.
    pub perf: Performance,
    /// The notes as a phrase.
    pub phrase: &'static Phrase,
    /// The band applied after the render, if the clip has one.
    pub post: Option<EqBand>,
}

impl Playable {
    /// A clip as it plays.
    ///
    /// # Errors
    /// The clip is not playable (a timbre or phrase this build lacks).
    pub fn of(clip: &Played) -> Result<Playable, String> {
        match clip {
            Played::Patch { patch, phrase } => {
                let ph = phrases::phrase(*phrase)
                    .ok_or_else(|| format!("phrase {} is not in this build's tables", phrase.0))?;
                Ok(Playable {
                    patch: (**patch).clone(),
                    perf: ph.performance(),
                    phrase: ph,
                    post: None,
                })
            }
            Played::Stimulus(s) => {
                let patch = s.patch().map_err(|e| e.to_string())?;
                let notes: &'static [PhraseNote] = Box::leak(s.notes.clone().into_boxed_slice());
                let phrase: &'static Phrase = Box::leak(Box::new(Phrase {
                    notes,
                    length_ms: s.length_ms,
                }));
                Ok(Playable {
                    patch,
                    perf: s.performance(),
                    phrase,
                    post: s.post(),
                })
            }
        }
    }

    /// The mid channel of `patch` playing this clip's performance at `seed`, with the clip's band applied.
    ///
    /// # Errors
    /// The render failed.
    pub fn mid_of(&self, patch: &Patch, seed: u64) -> Result<Vec<f32>, String> {
        let mut out = render(
            patch,
            &self.perf,
            &RenderSpec::new(SCORING_SAMPLE_RATE, seed),
        )
        .map_err(|e| e.to_string())?;
        if let Some(band) = self.post
            && let Some(coeffs) = post_coeffs(band)
        {
            for channel in [&mut out.left, &mut out.right] {
                enth_dsp::biquad::Biquad::new(coeffs).process(channel);
            }
        }
        Ok(out.mid())
    }

    /// The clip rendered at `seed`.
    ///
    /// # Errors
    /// The render failed.
    pub fn mid(&self, seed: u64) -> Result<Vec<f32>, String> {
        self.mid_of(&self.patch, seed)
    }
}

/// The biquad section of a peak or shelf band (`None` for a band-pass band, which is in the timbre's filter): the
/// integers become `f64` by one division each, as `enth-challenge`'s renderer does it.
fn post_coeffs(band: EqBand) -> Option<enth_dsp::biquad::Coeffs> {
    let kind = match band.kind {
        EqKind::Peak => enth_dsp::biquad::Kind::Peak,
        EqKind::LowShelf => enth_dsp::biquad::Kind::LowShelf,
        EqKind::HighShelf => enth_dsp::biquad::Kind::HighShelf,
        EqKind::BandPass => return None,
    };
    Some(enth_dsp::biquad::Coeffs::design(
        kind,
        f64::from(band.freq_hz),
        f64::from(band.gain_cdb) / 100.0,
        f64::from(band.q_milli) / 1000.0,
        f64::from(SCORING_SAMPLE_RATE),
    ))
}

/// `sound/1`'s analysis of a render.
fn analyse(mid: &[f32], phrase: &Phrase, readings: bool) -> Result<sound::Clip, String> {
    sound::analyse_render(mid, phrase, readings).map_err(|e| e.to_string())
}

/// A clip, analysed at the scoring seed, readings included: what a player's answer is measured from.
///
/// # Errors
/// The render or its analysis failed.
pub fn hear(p: &Playable) -> Result<sound::Clip, String> {
    analyse(&p.mid(SCORING_SEED)?, p.phrase, true)
}

/// A clip's integrated loudness before `sound/1`'s gain (`None` when silent): how loud the option is as rendered.
pub fn lufs(clip: &sound::Clip) -> Option<f64> {
    clip.lufs
}

/// What `sound/1` keeps of a target besides its clip: its nuisance floor, its JND scale and the guard's floors.
#[derive(Clone)]
pub struct Calib {
    /// The nuisance floor (metric v1.1).
    pub floor: f64,
    /// The JND scale (metric v1.1).
    pub scale: f64,
    /// The target's own pitch floor, in cents.
    pub pitch_floor: f64,
    /// The target's own odd-share floor.
    pub wave_floor: f64,
}

/// `SoundV1::prepare` over a clip: the target's analysed clip, two floor renders at other seeds, the 16 jittered copies.
///
/// # Errors
/// A render or its analysis failed.
pub fn calibrate(p: &Playable) -> Result<Calib, String> {
    let clip = hear(p)?;
    if clip.pooled.silent {
        return Ok(Calib {
            floor: 0.0,
            scale: calibrate::SCALE_MIN,
            pitch_floor: 0.0,
            wave_floor: 0.0,
        });
    }
    let spread = |t: Option<f64>, o: Option<f64>| match (t, o) {
        (Some(t), Some(o)) => (o - t).abs(),
        _ => 0.0,
    };
    let (mut floor, mut pitch_floor, mut wave_floor) = (0.0f64, 0.0f64, 0.0f64);
    for k in 1..=sound::FLOOR_RENDERS {
        let other = analyse(&p.mid(sound::floor_seed(k))?, p.phrase, true)?;
        floor = floor.max(sound::clip_distance(&clip, &other));
        pitch_floor = pitch_floor.max(spread(clip.readings.pitch, other.readings.pitch));
        wave_floor = wave_floor.max(spread(clip.readings.harmonics, other.readings.harmonics));
    }
    let mut jitter = Vec::with_capacity(calibrate::JITTERS);
    for copy in calibrate::jitter_set(&p.patch, p.phrase) {
        let c = analyse(&p.mid_of(&copy, SCORING_SEED)?, p.phrase, false)?;
        jitter.push(sound::clip_distance(&clip, &c));
    }
    Ok(Calib {
        floor,
        scale: calibrate::scale(&jitter),
        pitch_floor,
        wave_floor,
    })
}

/// How far two clips are from each other in the units of `calib` (the item's calibration): the distance net of the
/// floor over the scale, plus the guard over the clips' own readings.
pub fn jnd_between(calib: &Calib, a: &sound::Clip, b: &sound::Clip) -> f64 {
    let d = sound::clip_distance(a, b);
    calibrate::jnd(d, calib.floor, calib.scale)
        + sound::guard(
            &a.readings,
            &b.readings,
            calib.pitch_floor,
            calib.wave_floor,
        )
}

/// A cache of calibrations of the parameter games' clips, which share a base: the same patch under the same phrase is
/// prepared once. Music clips are one-off and are not cached.
#[derive(Default)]
pub struct Calibrations(Mutex<HashMap<(PatchHash, u16), Arc<Calib>>>);

impl Calibrations {
    /// The calibration of `clip` (prepared at most a few times by racing threads: the results are identical).
    ///
    /// # Errors
    /// A render failed.
    pub fn get(&self, clip: &Played, p: &Playable) -> Result<Arc<Calib>, String> {
        let key = match clip {
            Played::Patch { patch, phrase } => Some((exact_hash(patch), phrase.0)),
            Played::Stimulus(_) => None,
        };
        if let Some(k) = key
            && let Some(hit) = self.0.lock().map_err(|e| e.to_string())?.get(&k)
        {
            return Ok(Arc::clone(hit));
        }
        let made = Arc::new(calibrate(p)?);
        if let Some(k) = key {
            self.0
                .lock()
                .map_err(|e| e.to_string())?
                .insert(k, Arc::clone(&made));
        }
        Ok(made)
    }
}
