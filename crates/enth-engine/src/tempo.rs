//! **Tempo: a deterministic render input** (S2a decision D5, lane TMP).
//!
//! The host's or the CLI's tempo reaches the engine as an [`EventKind::Tempo`](crate::EventKind::Tempo) event, like
//! a note or a parameter value, so it takes effect at the first segment boundary at or after its frame (design.md
//! §5) and the host's block size cannot change a render: a tempo that changes in the middle of a host block applies
//! at the next 32-frame boundary, the same boundary whatever the block size. The engine keeps one tempo, shared by
//! both plans during a crossfade and carried over every plan swap (it is not plan state), and hands it to the modules
//! that read it as `tempo_bpm` in their contexts ([`VoiceModCx`](crate::module::VoiceModCx),
//! [`GlobalModCx`](crate::module::GlobalModCx), [`GlobalCx`](crate::module::GlobalCx)). A module that syncs to it (the
//! `lfo` with `sync` on, later the `mseg` and the `delay`) turns beats into frames with the tempo of the segment it is
//! rendering, so a synced phase is exact in beats under a piecewise-constant tempo.
//!
//! - **Range.** [`MIN_TEMPO_BPM`] to [`MAX_TEMPO_BPM`], 20 to 999 beats per minute (the span hosts offer; slower
//!   than 20 or faster than 999 BPM is not music). The engine clamps a request into it and ignores NaN, as it does
//!   every event; `enth-render` and the CLI refuse a performance or flag outside it instead of clamping silently.
//! - **Default.** [`DEFAULT_TEMPO_BPM`], 140, for an engine nobody told: a patch with no synced module does not read
//!   it, so it changes no existing render (S0d's test renders an unsynced patch at 90, 140 and 174 BPM to the same
//!   bits).
//! - **A beat is a quarter note**, and a bar is four beats, whatever the host's time signature.
//! - **`Engine::reset`** returns the tempo to the default, like every other event-driven state, so what follows a
//!   reset renders like a new engine; a host adapter sends the host's tempo again (the plugin does).
//! - The tempo is an `f64`: [`EventKind::Tempo`](crate::EventKind::Tempo) carries the host's own number, and the
//!   synced LFO divides it exactly once, in `f64`.

/// The slowest tempo the engine accepts, in beats per minute.
pub const MIN_TEMPO_BPM: f64 = 20.0;

/// The fastest tempo the engine accepts, in beats per minute.
pub const MAX_TEMPO_BPM: f64 = 999.0;

/// The tempo of an engine nobody told, in beats per minute (S2a decision D5: bass music sits near 140).
pub const DEFAULT_TEMPO_BPM: f64 = 140.0;

/// Whether `bpm` is a tempo the engine takes as it is: finite and within [`MIN_TEMPO_BPM`]`..=`[`MAX_TEMPO_BPM`].
#[must_use]
pub fn is_valid(bpm: f64) -> bool {
    (MIN_TEMPO_BPM..=MAX_TEMPO_BPM).contains(&bpm)
}

/// The tempo the engine uses for a request: `bpm` clamped into the range, or `None` for NaN (ignored, like a NaN
/// parameter value). Infinities clamp to the ends.
#[must_use]
pub fn sanitize(bpm: f64) -> Option<f64> {
    if bpm.is_nan() {
        None
    } else {
        Some(bpm.clamp(MIN_TEMPO_BPM, MAX_TEMPO_BPM))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_is_inside_the_range_and_the_range_is_documented() {
        assert!(is_valid(DEFAULT_TEMPO_BPM));
        assert_eq!(DEFAULT_TEMPO_BPM, 140.0);
        assert_eq!((MIN_TEMPO_BPM, MAX_TEMPO_BPM), (20.0, 999.0));
        assert!(is_valid(20.0) && is_valid(999.0));
        assert!(!is_valid(19.999) && !is_valid(999.001));
        assert!(
            !is_valid(f64::NAN) && !is_valid(f64::INFINITY) && !is_valid(0.0) && !is_valid(-120.0)
        );
    }

    #[test]
    fn a_request_is_clamped_and_nan_is_ignored() {
        assert_eq!(sanitize(128.0), Some(128.0));
        assert_eq!(sanitize(10.0), Some(MIN_TEMPO_BPM));
        assert_eq!(sanitize(-5.0), Some(MIN_TEMPO_BPM));
        assert_eq!(sanitize(0.0), Some(MIN_TEMPO_BPM));
        assert_eq!(sanitize(5000.0), Some(MAX_TEMPO_BPM));
        assert_eq!(sanitize(f64::INFINITY), Some(MAX_TEMPO_BPM));
        assert_eq!(sanitize(f64::NEG_INFINITY), Some(MIN_TEMPO_BPM));
        assert_eq!(sanitize(f64::NAN), None);
    }
}
