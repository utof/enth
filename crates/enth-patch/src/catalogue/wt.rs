//! `wt`: a wavetable oscillator on phase-read waves: generated tables, a phase warp, phase and amplitude modulation from
//! other generators through audio routes (`pm`, `am`; AM around zero is ring modulation), `feedback`, and unison on
//! `wide` lanes (lane UNI). Lane WT owns its DSP, `docs/dsp/wt.md` and its golden cases; lane UNI the unison
//! (docs/phase-s2.md).
//!
//! **A reserved S2a kind** (lane S0d, freeze F0m): its key, id prefix, section and discriminant (8) are fixed and
//! persisted; it is hidden ([`ModuleSpec::hidden`](super::ModuleSpec::hidden)) from the Add menu, `enth modules` and
//! the generator until lane INT-S2a clears the flag. **The parameter table below is lane CAT's F1m table**: after F1m
//! its keys, units, ranges, scales, defaults and flags change only through the coordinator (docs/dsp/catalogue.md,
//! "S2a decisions"). The unison parameters are in it from F1m and do nothing until lane UNI builds them.
//!
//! # The audio inputs (`pm`, `am`) and what their values mean
//!
//! An audio route (lane RTE's contract) delivers, per frame, the sum of its sources' **pre-level** outputs, each
//! scaled by its route depth (−100..100 %): `pm_in(t) = Σ depth·source(t)` into `pm`, `am_in(t)` likewise into `am`.
//! `pm` and `am` carry the `audio` flag, so no control-rate source reaches them; **the parameter's value is the input's
//! base amount**, the part a route does not supply:
//!
//! - **PM:** the phase offset, in radians, is `pm_index · (pm / 100) · pm_in(t)`. `pm` is a trim on everything routed
//!   in (100 % passes the route depths as set, 0 % turns PM off); `pm_index` is the modulatable amount, so an
//!   envelope, LFO or MSEG on `pm_index` is the growl (report 04, S4). With no route, `pm_in` is 0 and nothing
//!   happens, whatever the two values.
//! - **AM:** the output is multiplied by `am / 100 + am_in(t)`. At the default 100 % and no route the gain is 1;
//!   `am` above 0 % with a route is amplitude modulation around that gain (sidebands at `depth / (2·am)` of the
//!   carrier); **`am` at 0 % with a route is ring modulation** (the gain is the bipolar input itself, so the carrier
//!   disappears). Without a route, `am` scales the output like a second level.
//! - `feedback` is PM of the oscillator by its own output (a one-sample delay with two-sample averaging, report 05),
//!   a normal parameter, not a route.
//!
//! PM, warps and feedback act on the table's read phase, so they apply to phase-read waves only (report 06 §3.1, point
//! 5), which every `wt` table is.
//!
//! # Unison (lane UNI)
//!
//! `unison` copies of the oscillator per voice, detuned evenly across ±`detune`, the side copies at `blend` against the
//! centre, panned across `spread` through lane SVP's stereo voice path. `unison` is **structural**, so a plan knows
//! whether it needs the stereo voice path from the patch alone (the recommendation to UNI: stereo when an enabled
//! `wt` has `unison` ≥ 2; then `spread` can be modulated, and `spread` 0 equals mono on both channels). A copy's start
//! phase comes from `derive(voice_seed, k)` in `free` mode, from the module id and `k` (the same every note) in `reset`
//! mode, and from `seed` and `k` in `seeded` mode, never from a parameter value.
//!
//! At its defaults it is a plain sine at full level (the `basic` table at position 0), as harmless as a new `osc`.

use enth_params::{ParamSpec, Scale, Unit, enum_items, params};

use super::{PHASE_MODES, PhaseMode, SMOOTH};

enum_items! {
    /// The generated wavetables (formulas over harmonic amplitudes, no audio assets). The first frame of `basic` is a
    /// pure sine, the operator FM wants.
    pub enum WtTable in WT_TABLES {
        /// Sine → triangle → saw → square.
        Basic = ("basic", "Basic", "Morphs sine, triangle, saw and square; at position 0 it is a pure sine, the classic FM operator."),
        /// A sine phase-modulated by a sine at the same pitch, its index rising across the table.
        Fm = ("fm", "FM sweep", "Two-operator FM at a 1:1 ratio, the index rising with position: a growl you scan instead of route."),
        /// The same with the modulator an octave up (1:2): odd harmonics.
        FmOctave = ("fm_octave", "FM sweep, octave", "Two-operator FM with the modulator an octave up, the index rising with position: hollow, reedy tones."),
        /// A saw through one resonant peak that sweeps up the harmonics across the table.
        Formant = ("formant", "Formant sweep", "A saw with one vocal resonance that rises with position: scanning it says 'wow' and 'yoi'."),
        /// A few strong, widely spaced upper harmonics; position adds density.
        Metal = ("metal", "Sparse metal", "A few strong, widely spaced harmonics: clangy, robotic tones that grow denser with position."),
    }
}

enum_items! {
    /// How the read phase is bent before the table is read. Every warp is the identity at `warp_amount` 0.
    pub enum Warp in WARPS {
        /// No warp.
        Off = ("off", "Off", "The table is read straight."),
        /// The phase bends towards one end of the cycle (Massive's Bend, a Casio CZ-style phase distortion).
        Bend = ("bend", "Bend", "Bends the cycle so the wave's detail bunches up at the start (positive) or the end (negative): brighter, nasal tones."),
        /// Opposite bends in the two halves of the cycle.
        Asymmetric = ("asymmetric", "Asymmetric bend", "Bends the two halves of the cycle in opposite directions: a pulse-width-like change for any wave."),
        /// The cycle is squeezed into part of the period, the rest holding still.
        Squeeze = ("squeeze", "Squeeze", "Plays the whole cycle in a shorter part of the period and holds the rest: a thinner, pulse-like buzz."),
        /// The phase runs faster and wraps, like hard sync.
        Sync = ("sync", "Sync", "Reads the cycle several times per period and restarts it, like hard sync: tearing, vocal sweeps."),
        /// The phase moves in steps.
        Quantize = ("quantize", "Quantize", "Reads the cycle in steps: stepped, lo-fi versions of the wave."),
    }
}

params! {
    /// Parameters of the `wt` module, in spec order.
    pub enum WtParam in WT_PARAMS {
        Table = ParamSpec::enumeration("table", "Wavetable", &WT_TABLES, WtTable::Basic.index())
            .structural()
            .basic("Which generated wavetable to play: basic shapes, FM sweeps, a formant sweep or sparse metal; changing it rebuilds the voice plan.")
            .moves("spectrum.harmonics_db"),
        Position = ParamSpec::float("position", "Position", Unit::Percent, 0.0, 100.0, Scale::Linear, 0.0)
            .modulatable()
            .smoothing(SMOOTH)
            .basic("Where in the wavetable to play, from its first frame to its last; route an LFO or MSEG here for moving timbres.")
            .moves("spectrum.centroid_hz"),
        Warp = ParamSpec::enumeration("warp", "Warp", &WARPS, Warp::Off.index())
            .intermediate("How the wave's cycle is bent before it is read: bend, asymmetric bend, squeeze, sync or quantize.")
            .moves("spectrum.harmonics_db"),
        WarpAmount = ParamSpec::float("warp_amount", "Warp amount", Unit::Percent, -100.0, 100.0, Scale::Linear, 0.0)
            .modulatable()
            .smoothing(SMOOTH)
            .intermediate("How strongly the warp bends the cycle: 0 % leaves it straight; the sign flips the direction where the warp has one.")
            .moves("spectrum.centroid_hz"),
        Semi = ParamSpec::float("semi", "Semitones", Unit::Semitones, -48.0, 48.0, Scale::Linear, 0.0)
            .modulatable()
            .smoothing(SMOOTH)
            .basic("Transposition in semitones from the played note; as an FM modulator, +12st is a 1:2 ratio and +19.02st a 1:3.")
            .moves("pitch.f0_hz"),
        Fine = ParamSpec::float("fine", "Fine", Unit::Cents, -100.0, 100.0, Scale::Linear, 0.0)
            .modulatable()
            .smoothing(SMOOTH)
            .basic("Fine tuning in cents; a few cents off a 1:1 FM ratio makes the timbre slowly churn.")
            .moves("detune.beating_hz"),
        Level = ParamSpec::float("level", "Level", Unit::Percent, 0.0, 100.0, Scale::Linear, 100.0)
            .modulatable()
            .smoothing(SMOOTH)
            .basic("Output level into the voice, as a linear percentage; at 0 % an FM modulator is silent but still modulates.")
            .moves("loudness.lufs_i"),
        Pm = ParamSpec::float("pm", "PM input", Unit::Percent, 0.0, 100.0, Scale::Linear, 100.0)
            .audio()
            .advanced("Where audio routes from other generators land for phase modulation (FM); its value trims everything routed in: 100 % passes the route depths as set, 0 % turns PM off."),
        PmIndex = ParamSpec::float("pm_index", "PM index", Unit::None, 0.0, 10.0, Scale::Linear, 1.0)
            .modulatable()
            .smoothing(SMOOTH)
            .intermediate("How far a full-scale input into pm bends the phase, in radians: 1 is gentle, 5 a growl, 10 a harsh clang; route an envelope or MSEG here to make it talk.")
            .moves("spectrum.harmonics_db"),
        Am = ParamSpec::float("am", "AM input", Unit::Percent, 0.0, 100.0, Scale::Linear, 100.0)
            .audio()
            .intermediate("Where audio routes from other generators land for amplitude modulation; its value is the gain they swing around: 100 % is AM, 0 % is ring modulation (without a route it scales the output)."),
        Feedback = ParamSpec::float("feedback", "Feedback", Unit::Percent, 0.0, 100.0, Scale::Linear, 0.0)
            .modulatable()
            .smoothing(SMOOTH)
            .intermediate("Phase modulation of the oscillator by its own output: a sine turns towards a saw, then a rough, noisy edge.")
            .moves("spectrum.harmonics_db"),
        Unison = ParamSpec::int("unison", "Unison", Unit::None, 1, 8, 1)
            .structural()
            .intermediate("Copies of the oscillator per voice (1 is off), detuned against each other for width and thickness; changing it rebuilds the plan."),
        Detune = ParamSpec::float("detune", "Detune", Unit::Cents, 0.0, 100.0, Scale::Linear, 20.0)
            .modulatable()
            .smoothing(SMOOTH)
            .intermediate("How far the outermost unison copies sit from the center, in cents (±): small values thicken, large ones sound like a detuned chord.")
            .moves("detune.beating_hz"),
        Blend = ParamSpec::float("blend", "Blend", Unit::Percent, 0.0, 100.0, Scale::Linear, 100.0)
            .modulatable()
            .smoothing(SMOOTH)
            .advanced("Level of the side unison copies against the center one: 0 % is the center alone, 100 % every copy equal."),
        Spread = ParamSpec::float("spread", "Spread", Unit::Percent, 0.0, 100.0, Scale::Linear, 50.0)
            .modulatable()
            .smoothing(SMOOTH)
            .intermediate("How widely the unison copies are spread across the stereo field: 0 % is mono, 100 % fully left and right.")
            .moves("stereo.width"),
        PhaseMode = ParamSpec::enumeration("phase_mode", "Phase mode", &PHASE_MODES, PhaseMode::Reset.index())
            .advanced("How each note's start phase is chosen: reset (the phase knob, the same every note, as FM and warps want), free (random per render seed) or seeded (from seed)."),
        Phase = ParamSpec::float("phase", "Start phase", Unit::Degrees, 0.0, 360.0, Scale::Linear, 0.0)
            .periodic()
            .advanced("The start phase in reset mode; between a carrier and its modulator it changes the FM timbre."),
        Seed = ParamSpec::int("seed", "Seed", Unit::None, 0, 65_535, 0)
            .not_automatable()
            .advanced("The random seed used in seeded phase mode, for the start phase and the unison copies; part of the patch, unlike the render seed."),
    }
}
