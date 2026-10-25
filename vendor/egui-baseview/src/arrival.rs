// enth fork (patch 12): when each input event arrived. Licensed like the rest of egui-baseview, MIT OR Apache-2.0.

//! When each input event arrived, for input whose timing matters more than the frame's (a tap pad).
//!
//! egui sees input once per frame: [`egui::InputState::time`] is the instant the frame took its input, and every event
//! of the frame shares it, so at 30 frames per second a key press can be up to about 33 ms older than its frame. The
//! window stamps each baseview event with a monotonic [`Instant`] the moment its handler receives it from the platform's
//! event loop (before any work on it), and gives every egui event that event produced the same stamp.
//!
//! The stamps of a frame are in the context's temporary data, under keys made of plain strings and holding plain
//! types, so code that only depends on egui (a widget crate) can read them without naming this crate:
//!
//! - `egui::Id::unique(STAMPS_KEY)`: an `Arc<[Instant]>`, one stamp per event of [`egui::InputState::raw`]'s `events`,
//!   index for index (not `InputState::events`, from which widgets may consume events). When egui runs a second pass
//!   in one frame (a discarded pass), that pass has no events, and its stamps do not apply: compare the lengths.
//! - `egui::Id::unique(ORIGIN_KEY)`: the [`Instant`] that `InputState::time` counts from, so that
//!   `origin + Duration::from_secs_f64(time)` is the frame's instant and a stamp converts into egui's time base.
//!
//! A frame without the backend (an `egui_kittest` harness) has neither, so a reader falls back to the frame's time.

use std::sync::Arc;
use std::time::Instant;

/// The key (through `egui::Id::unique`) of a frame's arrival stamps in the context's temporary data, an `Arc<[Instant]>`.
pub const STAMPS_KEY: &str = "egui_baseview::arrival::stamps";

/// The key (through `egui::Id::unique`) of the instant that `InputState::time` counts from, an [`Instant`].
pub const ORIGIN_KEY: &str = "egui_baseview::arrival::origin";

/// The arrival stamps of this pass's events, index for index with `InputState::raw.events`; `None` when the backend
/// gave none, or when they do not line up with this pass's events (a second pass in one frame has none).
pub fn stamps(ctx: &egui::Context) -> Option<Arc<[Instant]>> {
    let stamps = ctx.data(|data| data.get_temp::<Arc<[Instant]>>(egui::Id::unique(STAMPS_KEY)))?;
    let events = ctx.input(|input| input.raw.events.len());
    (stamps.len() == events).then_some(stamps)
}

/// The instant that `InputState::time` counts from (the window's creation).
pub fn origin(ctx: &egui::Context) -> Option<Instant> {
    ctx.data(|data| data.get_temp::<Instant>(egui::Id::unique(ORIGIN_KEY)))
}

/// This pass's events with the instant each arrived, in order; empty when there are no stamps.
pub fn events(ctx: &egui::Context) -> Vec<(egui::Event, Instant)> {
    let Some(stamps) = stamps(ctx) else {
        return Vec::new();
    };
    ctx.input(|input| input.raw.events.iter().cloned().zip(stamps.iter().copied()).collect())
}

/// An instant in egui's time base: seconds since [`origin`], comparable with `InputState::time`.
pub fn egui_time(ctx: &egui::Context, at: Instant) -> Option<f64> {
    origin(ctx).map(|origin| at.saturating_duration_since(origin).as_secs_f64())
}

/// Stores a frame's stamps and the time origin for [`stamps`] and [`origin`] (the window, before each frame).
pub(crate) fn store(ctx: &egui::Context, stamps: Arc<[Instant]>, origin: Instant) {
    ctx.data_mut(|data| {
        data.insert_temp(egui::Id::unique(STAMPS_KEY), stamps);
        data.insert_temp(egui::Id::unique(ORIGIN_KEY), origin);
    });
}
