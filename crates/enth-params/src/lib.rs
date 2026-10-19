//! enth-params (L0): the parameter schema shared by every consumer (docs/design.md §1, docs/architecture.md).
//!
//! - [`ParamSpec`]: key, name, [`Unit`], [`Kind`] (`Float{min,max,scale}`, `Int`, `Enum`, `Toggle`), default,
//!   [`Smoothing`], [`Flags`] (`modulatable`, `automatable`, `structural`, `periodic`), [`LearnMeta`] and the
//!   descriptor it mainly moves. Declared in `const` tables with [`params!`], checked at compile time.
//! - [`Scale`] (`Linear`, `Log`, `Power(k)`): plain ⇄ normalised, through the pinned `libm`. The scale is part of
//!   the persisted contract (route depth, ML encoding and the randomiser live in the normalised domain).
//! - Units: [`Unit::parse`] / [`ParamSpec::parse`] read `800Hz`, `1.2kHz`, `A4`, `40ms`, `0.5s`, `-6dB`, `+2st`,
//!   `12ct`, `+1oct`, `30%`, `4:1`, `90deg`, enum keys and `on`/`off`; a bare number is in the spec's unit. The
//!   typographic minus U+2212 that the editor displays (`−12 st`) reads as `-`.
//!   [`ParamSpec::display`] is the exact inverse; [`ParamSpec::display_rounded`] is for humans.
//! - Route depths: [`DEPTH`] and [`ParamSpec::depth_from_text`] (`+1oct` on 20 Hz–20 kHz Log is 10.034 %).
//!
//! Plain values in real units are what patches store; normalised values exist only at the host-automation, UI
//! gesture, modulation and ML boundaries (AGENTS.md).
//!
//! This crate is core (L0): no I/O, no clocks, no threads, builds for `wasm32-unknown-unknown`, and uses no `std`
//! transcendentals (the root `clippy.toml` bans them).

pub mod display;
mod macros;
pub mod parse;
pub mod scale;
pub mod spec;
pub mod units;

pub use display::{Formatted, UnitValue};
pub use parse::{DEPTH, Expected, ValueError};
pub use scale::Scale;
pub use spec::{
    EnumItem, Flags, Kind, LearnMeta, ParamSpec, Smoothing, Tier, check_items, check_spec,
    check_table, is_valid_key,
};
pub use units::{Unit, UnitError};
