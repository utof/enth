//! Motion tokens (component tier): repaint rates and transition time (docs/design-system.md, "Motion").

use serde::{Deserialize, Serialize};

use super::primitives as p;

/// Animation rates. Nothing animates unless it is visible and its data changed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Motion {
    /// Live rings and source tiles (Hz).
    pub animated_hz: f32,
    /// A visible scope with fresh data (Hz).
    pub scope_hz: f32,
    /// egui's own transitions (collapsing, scrolling), in seconds.
    pub transition: f32,
}

impl Motion {
    /// The dark theme's rates.
    pub(super) fn dark() -> Self {
        Self {
            animated_hz: p::RATE_ANIMATED,
            scope_hz: p::RATE_SCOPE,
            transition: 0.1,
        }
    }
}

impl Motion {
    /// Delay between frames while something animated is visible.
    pub fn animated_interval(&self) -> std::time::Duration {
        std::time::Duration::from_secs_f32(1.0 / self.animated_hz)
    }

    /// Delay between frames while a scope is visible and fed.
    pub fn scope_interval(&self) -> std::time::Duration {
        std::time::Duration::from_secs_f32(1.0 / self.scope_hz)
    }
}
