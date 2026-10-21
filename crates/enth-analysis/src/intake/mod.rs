//! **Owner: lane FE** (the intake, freeze point F1s). Pre-declared by lane S0s (R3, freeze point F0s).
//!
//! The intake of "create from sound" (docs/r3.md, "The intake"): pure functions of samples that describe what an
//! uploaded recording still carries, so the search stops paying knobs to reproduce damage. Like the rest of this crate
//! they do no I/O, read no clocks, spawn no threads and use no global RNG, they build for wasm32, transcendentals go
//! through `libm`, and every decision runs on the scalar FFT path. The decoders are L8's (the plugin's `create`
//! feature and the CLI); the intake starts from decoded samples at the file's own rate.
//!
//! **Short sounds are never rejected, and nothing is filtered by duration.** There is **no resampler in the product
//! path**: render and analysis run at the file's own rate (8 to 192 kHz), as `enth match` already does.
//!
//! | File | Holds |
//! | --- | --- |
//! | [`trim`] | mono (the mean of the channels), the leading-silence strip with a few ms of pre-roll, the 4 to 6 s cap |
//! | [`floor`] | the noise floor, and the hum (50 and 60 Hz with harmonics) that a flat floor hides |
//! | [`bandwidth`] | the effective bandwidth: where the spectrum stops (a 64 kbps MP3 low-passes near 11 kHz) |
//! | [`clip`] | clipped or limited spans |
//! | [`perf`] | the inferred performance (`InferredPerf`): the note from YIN, the note-off from `envelope::infer_note_off`, the hold; a "this looks like a phrase" or chord warning |
//! | [`resample`] | **open check:** whether the intake needs a resampler at all (report 03: it does not); FE decides, and deletes the file if not |
//!
//! FE adds the types the other lanes build on (`Intake`, `Degradation`, `InferredPerf`) at F1s, in `intake`'s own files;
//! lane S0s declared the files and nothing else. The robust distance that uses what the intake reports is lane OBJ's
//! ([`robust`](crate::robust)).
//!
//! Acceptance (docs/phase-3.md): trim within 5 ms; bandwidth within 1/3 octave; the note right on at least 95 % of
//! pitched clips; a chord warning; short clips kept.

pub mod bandwidth;
pub mod clip;
pub mod floor;
pub mod perf;
pub mod resample;
pub mod trim;
