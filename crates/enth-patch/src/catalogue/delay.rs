//! `delay`: a stereo delay after the voices, its time in ms or synced in beats to the render's tempo, with ping-pong
//! and filters in the feedback loop; `mix`. Lane DLY owns its DSP, `docs/dsp/delay.md` and its golden cases
//! (docs/phase-s2.md).
//!
//! **A reserved S2a kind** (lane S0d, freeze F0m): its key, id prefix, section and discriminant (16) are fixed and
//! persisted; it is hidden ([`ModuleSpec::hidden`](super::ModuleSpec::hidden)) from the Add menu, `enth modules` and
//! the generator until lane INT-S2a clears the flag. **The parameter table below is lane CAT's F1m table**: after F1m
//! its keys, units, ranges, scales, defaults and flags change only through the coordinator (docs/dsp/catalogue.md,
//! "S2a decisions").
//!
//! What the table's meanings assume (lane DLY implements it):
//!
//! - The delay time is `beats` of the render's tempo when `sync` is on (exact in beats, at segment boundaries across a
//!   tempo change), `time` when it is off; **never longer than 2 s** (`time`'s maximum, which sizes the rings at
//!   `prepare`), so a synced time past 2 s (a bar below 120 BPM) plays at 2 s. `beats` offers the shared list up to
//!   one bar ([`DELAY_BEATS`]): the same keys and indices as every synced parameter, without lengths a 2 s ring
//!   cannot hold.
//! - Each repeat passes `low_cut` and `high_cut` (inside the loop, so repeats thin out and darken) and returns at
//!   `feedback`; `ping_pong` crosses the repeats between the channels. A soft clip in the loop keeps 100 % feedback
//!   bounded (it repeats for ever).
//! - **The effect's own output is the dry signal plus the echoes**, so `mix` scales the echoes against an unchanged
//!   dry signal (100 %: the first echo as loud as the dry sound, filtered); `mix` 0 % returns the input bit for bit.
//! - A global, time-based effect (the plugin's FX bypass may mute it); only global sources modulate it. Time changes
//!   glide or crossfade (DLY's choice), never click.
//!
//! At its defaults it is **a gentle useful setting**: a dotted-eighth echo, synced, three or four audible repeats,
//! kept off the sub (low cut 150 Hz) and darkened (high cut 6 kHz).

use enth_params::{EnumItem, ParamSpec, Scale, Unit, params};

use super::{BEATS, Beats, SMOOTH};

/// The synced delay times: the shared [`BEATS`] list from its first item to one bar (`bar`), so every item fits the
/// 2 s ring at 120 BPM. Its indices are `Beats`' (decode a value with [`Beats::from_value`]).
pub const DELAY_BEATS: &[EnumItem] = BEATS.as_slice().split_at(Beats::Bar.index() + 1).0;

params! {
    /// Parameters of the `delay` module, in spec order.
    pub enum DelayParam in DELAY_PARAMS {
        Sync = ParamSpec::toggle("sync", "Tempo sync", true)
            .basic("On: the delay time is beats of the song's tempo. Off: it is time, in milliseconds."),
        Time = ParamSpec::float("time", "Time", Unit::Ms, 1.0, 2_000.0, Scale::Log, 300.0)
            .intermediate("The delay time when tempo sync is off: under 30 ms thickens, 80-150 ms is a slapback, longer is an echo."),
        Beats = ParamSpec::enumeration("beats", "Beats", DELAY_BEATS, Beats::EighthDotted.index())
            .basic("The delay time when tempo sync is on, as a note division up to a bar: 1/8D is the classic bouncing echo."),
        Feedback = ParamSpec::float("feedback", "Feedback", Unit::Percent, 0.0, 100.0, Scale::Linear, 35.0)
            .modulatable()
            .smoothing(SMOOTH)
            .basic("How much of each echo returns for another: more is more repeats; 100 % repeats for ever."),
        PingPong = ParamSpec::toggle("ping_pong", "Ping-pong", false)
            .intermediate("Bounces the echoes between the left and right channels."),
        LowCut = ParamSpec::float("low_cut", "Low cut", Unit::Hz, 20.0, 2_000.0, Scale::Log, 150.0)
            .modulatable()
            .smoothing(SMOOTH)
            .intermediate("Removes lows from every repeat, so echoes stay off the bass; each repeat gets thinner."),
        HighCut = ParamSpec::float("high_cut", "High cut", Unit::Hz, 1_000.0, 20_000.0, Scale::Log, 6_000.0)
            .modulatable()
            .smoothing(SMOOTH)
            .intermediate("Removes highs from every repeat: each echo gets darker, like tape.")
            .moves("spectrum.centroid_hz"),
        Mix = ParamSpec::float("mix", "Mix", Unit::Percent, 0.0, 100.0, Scale::Linear, 100.0)
            .modulatable()
            .smoothing(SMOOTH)
            .intermediate("How loud the echoes are against the dry sound, which always passes unchanged: 100 % makes the first echo as loud as the dry; 0 % is no echo at all."),
    }
}
