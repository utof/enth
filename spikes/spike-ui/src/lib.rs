//! Phase 0 UI spike (docs/phase-0.md, Task 4): the reference panel from docs/benchmarks.md built
//! from enth-ui's widgets, with repaint gating and a lock-free audio feed. Throwaway code shared by
//! the dev window (`spike-ui-dev`), the snapshot test and the plugin (`spike-ui-plugin`).

pub mod feed;
pub mod layout;
pub mod lesson;
pub mod panel;
pub mod params;
pub mod shapes;
pub mod spectrum;

pub use feed::{AudioFeed, DEMO_SAMPLE_RATE, DemoChord};
pub use panel::{REFERENCE_SIZE, ReferencePanel, Repaint};
pub use params::{HostParams, LocalParams, ParamId};
