//! `ParamSpec` and its parts (`Kind`, `Flags`, `Smoothing`, `EnumItem`, `LearnMeta`), the `const` builders the
//! catalogue is written with, and the compile-time checks.
//!
//! A `ParamSpec` is declared once, in a `const` table, and drives every consumer: UI knobs, host automation, patch
//! validation, CLI parsing and display, ML encoding, challenge constraints, the randomiser and knob-distance
//! scoring (design.md §1). Tables are declared with [`params!`](crate::params), which runs [`check_table`] in a
//! `const` item, so an inconsistent spec (a default out of range, `min >= max`, a Log scale with `min <= 0`, a
//! duplicate key, …) stops the build with `error[E0080]` naming the table and the key.

use crate::scale::Scale;
use crate::units::Unit;

/// The value kind of a parameter. Values are always stored as `f32` in plain units: `Int` and `Enum` values are
/// exact integers (an enum value is the item's index), a `Toggle` is `0.0` or `1.0`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    /// A continuous value in `[min, max]` (plain units), mapped to 0–1 by `scale`.
    Float {
        /// Lowest plain value.
        min: f32,
        /// Highest plain value.
        max: f32,
        /// Plain ⇄ normalised mapping; part of the persisted contract.
        scale: Scale,
    },
    /// An integer in `[min, max]`, both within ±2^24 so every value is exact in `f32`.
    Int {
        /// Lowest value.
        min: i32,
        /// Highest value.
        max: i32,
    },
    /// One of a fixed list of named items; the value is the item's index. Text uses the item's key.
    Enum(&'static [EnumItem]),
    /// Off (`0.0`) or on (`1.0`).
    Toggle,
}

/// One item of an [`Kind::Enum`] parameter. The key is what files and the CLI use (`"saw"`, `"lp"`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EnumItem {
    /// Stable key, `[a-z][a-z0-9_]*`, unique within the enum (case-insensitively). Renaming one is a migration.
    pub key: &'static str,
    /// Display name for the UI.
    pub name: &'static str,
    /// One plain sentence for learners and agents.
    pub desc: &'static str,
}

impl EnumItem {
    /// An item with its key, display name and one-sentence description.
    pub const fn new(key: &'static str, name: &'static str, desc: &'static str) -> Self {
        EnumItem { key, name, desc }
    }
}

/// Who may drive a parameter, and how it behaves.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "four independent flags, mirroring CLAP's parameter flags"
)]
pub struct Flags {
    /// Modulation routes may target it. Only `Float` parameters, never structural ones.
    pub modulatable: bool,
    /// An automation slot may expose it to the host. Never structural ones (each change would recompile the plan).
    pub automatable: bool,
    /// Changing it changes the render plan (`voice.mode`, `voice.polyphony`, an LFO's scope), so an op that sets
    /// it recompiles instead of sending a value event.
    pub structural: bool,
    /// The value wraps around instead of clamping (phase knobs; CLAP's `IS_PERIODIC`). Linear `Float` or `Int`
    /// only.
    pub periodic: bool,
}

impl Flags {
    /// Nothing may drive it except an op (and, for structural parameters, a recompile).
    pub const NONE: Flags = Flags {
        modulatable: false,
        automatable: false,
        structural: false,
        periodic: false,
    };
    /// Host-automatable, not modulatable. The builders' default.
    pub const AUTOMATABLE: Flags = Flags {
        modulatable: false,
        automatable: true,
        structural: false,
        periodic: false,
    };
    /// Modulatable and automatable.
    pub const MODULATABLE: Flags = Flags {
        modulatable: true,
        automatable: true,
        structural: false,
        periodic: false,
    };
    /// Structural: changing it recompiles the plan; neither modulatable nor automatable.
    pub const STRUCTURAL: Flags = Flags {
        modulatable: false,
        automatable: false,
        structural: true,
        periodic: false,
    };
}

/// How the engine smooths changes to a parameter's base value. Smoothing runs in the normalised domain, so a
/// cutoff ramp is linear in octaves, and a settled smoother delivers the exact plain value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Smoothing {
    /// Steps at the next segment boundary (enums, toggles, integers, times read at segment starts, and effects
    /// that fade internally).
    None,
    /// A linear ramp over `ms` milliseconds.
    Linear {
        /// Ramp length.
        ms: f32,
    },
    /// A one-pole low-pass with time constant `ms` milliseconds.
    OnePole {
        /// Time constant.
        ms: f32,
    },
}

/// How far into the synth a parameter sits, for the UI's progressive disclosure and for lessons.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Tier {
    /// The knobs a first patch needs.
    Basic,
    /// Sound-design knobs.
    Intermediate,
    /// Rarely touched, or only meaningful once the rest is understood.
    Advanced,
}

impl Tier {
    /// A stable lowercase name: `basic`, `intermediate`, `advanced`.
    pub const fn name(self) -> &'static str {
        match self {
            Tier::Basic => "basic",
            Tier::Intermediate => "intermediate",
            Tier::Advanced => "advanced",
        }
    }
}

/// Learning metadata: the tier and one plain sentence for learners, the UI and agents (`enth params`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LearnMeta {
    /// Progressive-disclosure tier.
    pub tier: Tier,
    /// One plain sentence; never empty in the catalogue (checked at compile time).
    pub desc: &'static str,
}

/// The declaration of one parameter: identity, unit, value kind and range, default, smoothing, flags, learning
/// metadata and the descriptor it mainly moves.
///
/// Invariants (checked at compile time by [`check_table`] for every catalogue table):
/// - `key` matches `[a-z][a-z0-9_]{0,23}` and is unique in its table; it is unit-free (`cutoff`, not `cutoff_hz`);
/// - `name` and `learn.desc` are not empty;
/// - `Float`: `min < max`, both finite; `Log` needs `min > 0`; `Power(k)` needs a finite `k > 0`, `k ≠ 1`;
/// - `Int`: `min < max`, both within ±2^24; `Enum`: at least two items with valid, unique keys; `Enum` and
///   `Toggle` have [`Unit::None`];
/// - `default` is valid: finite, in range, integral for `Int`/`Enum`/`Toggle`, never `-0.0`;
/// - `modulatable` only on `Float`; `structural` excludes `modulatable` and `automatable`; `periodic` only on a
///   Linear `Float` or an `Int`; smoothing only on `Float`, with a finite `ms > 0`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ParamSpec {
    /// Stable key within the module kind (`"cutoff"`). Renaming one is a migration.
    pub key: &'static str,
    /// Display name (`"Cutoff"`).
    pub name: &'static str,
    /// The stored unit: patches hold plain values in it.
    pub unit: Unit,
    /// Value kind and range.
    pub kind: Kind,
    /// Default in plain units. Files are written dense, so changing a default only affects new modules (and needs
    /// a migration step for files that omit the key, design.md §2).
    pub default: f32,
    /// How the engine smooths changes.
    pub smoothing: Smoothing,
    /// Who may drive it.
    pub flags: Flags,
    /// Learning metadata.
    pub learn: LearnMeta,
    /// The descriptor this parameter mainly moves (`"spectrum.centroid_hz"`), for hints and lessons.
    pub perceptual: Option<&'static str>,
}

// ---------------------------------------------------------------------------------------------------------------
// Builders (const, so tables stay plain data)
// ---------------------------------------------------------------------------------------------------------------

const UNDESCRIBED: LearnMeta = LearnMeta {
    tier: Tier::Advanced,
    desc: "",
};

impl ParamSpec {
    /// A continuous parameter. Automatable, not modulatable, no smoothing, until the builder methods say otherwise.
    #[allow(
        clippy::too_many_arguments,
        reason = "mirrors the table columns; every argument is required"
    )]
    pub const fn float(
        key: &'static str,
        name: &'static str,
        unit: Unit,
        min: f32,
        max: f32,
        scale: Scale,
        default: f32,
    ) -> Self {
        ParamSpec {
            key,
            name,
            unit,
            kind: Kind::Float { min, max, scale },
            default,
            smoothing: Smoothing::None,
            flags: Flags::AUTOMATABLE,
            learn: UNDESCRIBED,
            perceptual: None,
        }
    }

    /// An integer parameter (automatable).
    pub const fn int(
        key: &'static str,
        name: &'static str,
        unit: Unit,
        min: i32,
        max: i32,
        default: i32,
    ) -> Self {
        ParamSpec {
            key,
            name,
            unit,
            kind: Kind::Int { min, max },
            default: default as f32,
            smoothing: Smoothing::None,
            flags: Flags::AUTOMATABLE,
            learn: UNDESCRIBED,
            perceptual: None,
        }
    }

    /// An enum parameter (automatable); `default` is the index of the default item.
    pub const fn enumeration(
        key: &'static str,
        name: &'static str,
        items: &'static [EnumItem],
        default: usize,
    ) -> Self {
        ParamSpec {
            key,
            name,
            unit: Unit::None,
            kind: Kind::Enum(items),
            default: default as f32,
            smoothing: Smoothing::None,
            flags: Flags::AUTOMATABLE,
            learn: UNDESCRIBED,
            perceptual: None,
        }
    }

    /// An on/off parameter (automatable).
    pub const fn toggle(key: &'static str, name: &'static str, default: bool) -> Self {
        ParamSpec {
            key,
            name,
            unit: Unit::None,
            kind: Kind::Toggle,
            default: if default { 1.0 } else { 0.0 },
            smoothing: Smoothing::None,
            flags: Flags::AUTOMATABLE,
            learn: UNDESCRIBED,
            perceptual: None,
        }
    }

    /// Modulation routes may target it (and it stays automatable).
    #[must_use]
    pub const fn modulatable(mut self) -> Self {
        self.flags.modulatable = true;
        self
    }

    /// Not exposed to host automation.
    #[must_use]
    pub const fn not_automatable(mut self) -> Self {
        self.flags.automatable = false;
        self
    }

    /// Changing it recompiles the plan; this also clears `automatable` and `modulatable`.
    #[must_use]
    pub const fn structural(mut self) -> Self {
        self.flags = Flags::STRUCTURAL;
        self
    }

    /// The value wraps instead of clamping (phase).
    #[must_use]
    pub const fn periodic(mut self) -> Self {
        self.flags.periodic = true;
        self
    }

    /// How the engine smooths changes.
    #[must_use]
    pub const fn smoothing(mut self, smoothing: Smoothing) -> Self {
        self.smoothing = smoothing;
        self
    }

    /// Tier and one-sentence description.
    #[must_use]
    pub const fn learn(mut self, tier: Tier, desc: &'static str) -> Self {
        self.learn = LearnMeta { tier, desc };
        self
    }

    /// Shorthand for `.learn(Tier::Basic, desc)`.
    #[must_use]
    pub const fn basic(self, desc: &'static str) -> Self {
        self.learn(Tier::Basic, desc)
    }

    /// Shorthand for `.learn(Tier::Intermediate, desc)`.
    #[must_use]
    pub const fn intermediate(self, desc: &'static str) -> Self {
        self.learn(Tier::Intermediate, desc)
    }

    /// Shorthand for `.learn(Tier::Advanced, desc)`.
    #[must_use]
    pub const fn advanced(self, desc: &'static str) -> Self {
        self.learn(Tier::Advanced, desc)
    }

    /// The descriptor it mainly moves (`"spectrum.centroid_hz"`).
    #[must_use]
    pub const fn moves(mut self, descriptor: &'static str) -> Self {
        self.perceptual = Some(descriptor);
        self
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Queries and value helpers
// ---------------------------------------------------------------------------------------------------------------

impl ParamSpec {
    /// Lowest plain value: `min` for `Float`/`Int`, `0` for `Enum` and `Toggle`.
    pub const fn min(&self) -> f32 {
        match self.kind {
            Kind::Float { min, .. } => min,
            Kind::Int { min, .. } => min as f32,
            Kind::Enum(_) | Kind::Toggle => 0.0,
        }
    }

    /// Highest plain value: `max` for `Float`/`Int`, the last index for `Enum`, `1` for `Toggle`.
    pub const fn max(&self) -> f32 {
        match self.kind {
            Kind::Float { max, .. } => max,
            Kind::Int { max, .. } => max as f32,
            Kind::Enum(items) => (items.len() - 1) as f32,
            Kind::Toggle => 1.0,
        }
    }

    /// `true` for `Int`, `Enum` and `Toggle`: values are exact integers.
    pub const fn is_discrete(&self) -> bool {
        !matches!(self.kind, Kind::Float { .. })
    }

    /// The scale of a `Float` parameter; `None` for discrete kinds (they map linearly by index).
    pub const fn scale(&self) -> Option<Scale> {
        match self.kind {
            Kind::Float { scale, .. } => Some(scale),
            _ => None,
        }
    }

    /// The number of distinct values minus one for discrete kinds (CLAP's stepped parameters); `None` for `Float`.
    pub const fn step_count(&self) -> Option<u32> {
        if self.is_discrete() {
            Some((self.max() - self.min()) as u32)
        } else {
            None
        }
    }

    /// The items of an `Enum` parameter.
    pub const fn enum_items(&self) -> Option<&'static [EnumItem]> {
        match self.kind {
            Kind::Enum(items) => Some(items),
            _ => None,
        }
    }

    /// The index of the enum item whose key matches `key` case-insensitively.
    pub fn enum_index(&self, key: &str) -> Option<usize> {
        self.enum_items()?
            .iter()
            .position(|i| i.key.eq_ignore_ascii_case(key))
    }

    /// The enum item a stored value names, if the value is a valid index.
    pub fn enum_item(&self, value: f32) -> Option<&'static EnumItem> {
        let items = self.enum_items()?;
        if value >= 0.0 && value.fract() == 0.0 {
            items.get(value as usize)
        } else {
            None
        }
    }

    /// Whether `value` may be stored: finite, within range, integral for discrete kinds, and not `-0.0`.
    pub fn is_valid(&self, value: f32) -> bool {
        value.is_finite()
            && value >= self.min()
            && value <= self.max()
            && (!self.is_discrete() || value.fract() == 0.0)
            && !(value == 0.0 && value.is_sign_negative())
    }

    /// Check a plain value against this spec. Returns it with `-0.0` turned into `+0.0`, or a typed error
    /// (`non_finite`, `not_integral`, `out_of_range`).
    pub fn check(&self, value: f32) -> Result<f32, crate::ValueError> {
        use crate::ValueError;
        if !value.is_finite() {
            return Err(ValueError::NonFinite { param: self.key });
        }
        if self.is_discrete() && value.fract() != 0.0 {
            return Err(ValueError::NotIntegral {
                param: self.key,
                value,
            });
        }
        if value < self.min() || value > self.max() {
            return Err(ValueError::OutOfRange {
                param: self.key,
                value,
                min: self.min(),
                max: self.max(),
                unit: self.unit,
            });
        }
        Ok(crate::units::canonical_zero(value))
    }

    /// The nearest valid value: clamped to the range, rounded for discrete kinds (or wrapped into range for
    /// periodic parameters), `-0.0` → `+0.0`. `NaN` stays `NaN` (it is a bug upstream).
    pub fn clamp(&self, value: f32) -> f32 {
        let (min, max) = (self.min(), self.max());
        let mut v = if self.flags.periodic && value.is_finite() {
            let span = f64::from(max) - f64::from(min);
            (f64::from(min) + (f64::from(value) - f64::from(min)).rem_euclid(span)) as f32
        } else {
            value
        };
        if self.is_discrete() {
            v = v.round();
        }
        crate::units::canonical_zero(v.clamp(min, max))
    }

    /// Plain → normalised 0–1 (see [`Scale`]). Discrete kinds map their values linearly by index.
    pub fn to_norm(&self, value: f32) -> f32 {
        match self.kind {
            Kind::Float { min, max, scale } => scale.to_norm(min, max, value),
            _ => Scale::Linear.to_norm(self.min(), self.max(), value),
        }
    }

    /// Normalised 0–1 → plain (see [`Scale`]). Discrete kinds round to the nearest value (a toggle is on from 0.5).
    pub fn from_norm(&self, norm: f32) -> f32 {
        if let Kind::Float { min, max, scale } = self.kind {
            return scale.from_norm(min, max, norm);
        }
        let (min, max) = (f64::from(self.min()), f64::from(self.max()));
        let n = f64::from(norm).clamp(0.0, 1.0);
        crate::units::canonical_zero((min + n * (max - min)).round() as f32)
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Compile-time checks
// ---------------------------------------------------------------------------------------------------------------

/// Panic (a compile error when called from a `const` item) with a message built from `parts`.
const fn fail(parts: &[&str]) -> ! {
    let mut buf = [0u8; 256];
    let mut n = 0;
    let mut p = 0;
    while p < parts.len() {
        let b = parts[p].as_bytes();
        let mut i = 0;
        while i < b.len() && n < buf.len() {
            buf[n] = b[i];
            n += 1;
            i += 1;
        }
        p += 1;
    }
    let (msg, _) = buf.split_at(n);
    match core::str::from_utf8(msg) {
        Ok(s) => panic!("{}", s),
        Err(_) => panic!("invalid spec"),
    }
}

/// `a == b` for `&str` in `const` code.
pub const fn str_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}

/// `a.eq_ignore_ascii_case(b)` in `const` code.
pub const fn str_eq_ignore_case(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if !a[i].eq_ignore_ascii_case(&b[i]) {
            return false;
        }
        i += 1;
    }
    true
}

/// Whether `key` is a valid parameter, item or module key: `[a-z][a-z0-9_]{0,23}`.
pub const fn is_valid_key(key: &str) -> bool {
    let b = key.as_bytes();
    if b.is_empty() || b.len() > 24 || !b[0].is_ascii_lowercase() {
        return false;
    }
    let mut i = 1;
    while i < b.len() {
        if !(b[i].is_ascii_lowercase() || b[i].is_ascii_digit() || b[i] == b'_') {
            return false;
        }
        i += 1;
    }
    true
}

/// Whether a `CamelCase` Rust variant name spells a `snake_case` key: letters and digits compared
/// case-insensitively, underscores in the key ignored (`PhaseMode` ↔ `phase_mode`, `SawUp` ↔ `saw_up`).
pub const fn variant_matches_key(variant: &str, key: &str) -> bool {
    let (v, k) = (variant.as_bytes(), key.as_bytes());
    let (mut i, mut j) = (0, 0);
    loop {
        while j < k.len() && k[j] == b'_' {
            j += 1;
        }
        if i == v.len() || j == k.len() {
            return i == v.len() && j == k.len();
        }
        if !v[i].eq_ignore_ascii_case(&k[j]) {
            return false;
        }
        i += 1;
        j += 1;
    }
}

const fn is_finite(x: f32) -> bool {
    x.is_finite()
}

const fn is_integral(x: f32) -> bool {
    is_finite(x) && x >= -16_777_216.0 && x <= 16_777_216.0 && (x as i64) as f32 == x
}

/// Check one spec; `table` names the table in the error (`"OSC_PARAMS"`). Panics, which is a compile error when
/// evaluated in a `const` item. See [`ParamSpec`] for the rules.
pub const fn check_spec(table: &str, s: &ParamSpec) {
    let k = s.key;
    if !is_valid_key(k) {
        fail(&[table, ".", k, ": key must match [a-z][a-z0-9_]{0,23}"]);
    }
    if s.name.is_empty() {
        fail(&[table, ".", k, ": name is empty"]);
    }
    if s.learn.desc.is_empty() {
        fail(&[
            table,
            ".",
            k,
            ": learn.desc is empty (use .basic/.intermediate/.advanced)",
        ]);
    }
    if let Some(p) = s.perceptual
        && p.is_empty()
    {
        fail(&[table, ".", k, ": perceptual descriptor is empty"]);
    }
    let d = s.default;
    if !is_finite(d) {
        fail(&[table, ".", k, ": default is not finite"]);
    }
    if d == 0.0 && d.is_sign_negative() {
        fail(&[table, ".", k, ": default is -0.0 (write 0.0)"]);
    }
    match s.kind {
        Kind::Float { min, max, scale } => {
            if !is_finite(min) || !is_finite(max) {
                fail(&[table, ".", k, ": min and max must be finite"]);
            }
            if min >= max {
                fail(&[table, ".", k, ": min must be < max"]);
            }
            match scale {
                Scale::Linear => {}
                Scale::Log => {
                    if min <= 0.0 {
                        fail(&[table, ".", k, ": a Log scale needs min > 0"]);
                    }
                }
                Scale::Power(e) => {
                    if !is_finite(e) || e <= 0.0 || e == 1.0 {
                        fail(&[
                            table,
                            ".",
                            k,
                            ": Power(k) needs a finite k > 0 and k != 1 (k = 1 is Linear)",
                        ]);
                    }
                }
            }
            if d < min || d > max {
                fail(&[table, ".", k, ": default out of range"]);
            }
            if s.flags.periodic && !matches!(scale, Scale::Linear) {
                fail(&[table, ".", k, ": periodic needs a Linear scale"]);
            }
        }
        Kind::Int { min, max } => {
            if min >= max {
                fail(&[table, ".", k, ": min must be < max"]);
            }
            if min < -16_777_216 || max > 16_777_216 {
                fail(&[
                    table,
                    ".",
                    k,
                    ": Int range must lie within +-2^24 (exact in f32)",
                ]);
            }
            if !is_integral(d) || d < min as f32 || d > max as f32 {
                fail(&[table, ".", k, ": default out of range or not an integer"]);
            }
        }
        Kind::Enum(items) => {
            if items.len() < 2 {
                fail(&[table, ".", k, ": an enum needs at least two items"]);
            }
            check_item_keys(table, k, items);
            if !is_integral(d) || d < 0.0 || d >= items.len() as f32 {
                fail(&[table, ".", k, ": default is not an item index"]);
            }
        }
        Kind::Toggle => {
            if !(d == 0.0 || d == 1.0) {
                fail(&[table, ".", k, ": a toggle default is 0 or 1"]);
            }
        }
    }
    let float = matches!(s.kind, Kind::Float { .. });
    if matches!(s.kind, Kind::Enum(_) | Kind::Toggle) && !matches!(s.unit, Unit::None) {
        fail(&[
            table,
            ".",
            k,
            ": enum and toggle parameters have Unit::None",
        ]);
    }
    if s.flags.modulatable && !float {
        fail(&[table, ".", k, ": only Float parameters may be modulatable"]);
    }
    if s.flags.structural && (s.flags.modulatable || s.flags.automatable) {
        fail(&[
            table,
            ".",
            k,
            ": a structural parameter is neither modulatable nor automatable",
        ]);
    }
    if s.flags.periodic && !matches!(s.kind, Kind::Float { .. } | Kind::Int { .. }) {
        fail(&[table, ".", k, ": periodic needs a Float or Int parameter"]);
    }
    match s.smoothing {
        Smoothing::None => {}
        Smoothing::Linear { ms } | Smoothing::OnePole { ms } => {
            if !float {
                fail(&[table, ".", k, ": only Float parameters are smoothed"]);
            }
            if !is_finite(ms) || ms <= 0.0 {
                fail(&[table, ".", k, ": smoothing needs a finite ms > 0"]);
            }
        }
    }
}

const fn check_item_keys(table: &str, param: &str, items: &[EnumItem]) {
    let mut i = 0;
    while i < items.len() {
        if !is_valid_key(items[i].key) {
            fail(&[
                table,
                ".",
                param,
                ": enum item key must match [a-z][a-z0-9_]{0,23}: ",
                items[i].key,
            ]);
        }
        if items[i].name.is_empty() || items[i].desc.is_empty() {
            fail(&[
                table,
                ".",
                param,
                ": enum item needs a name and a description: ",
                items[i].key,
            ]);
        }
        let mut j = i + 1;
        while j < items.len() {
            if str_eq_ignore_case(items[i].key, items[j].key) {
                fail(&[
                    table,
                    ".",
                    param,
                    ": duplicate enum item key: ",
                    items[i].key,
                ]);
            }
            j += 1;
        }
        i += 1;
    }
}

/// Check a whole table: every spec, unique keys, and (when `variants` is not empty) that the index enum's variant
/// names spell the keys in order. [`params!`](crate::params) calls this in a `const` item.
pub const fn check_table(table: &str, specs: &[ParamSpec], variants: &[&str]) {
    if !variants.is_empty() && variants.len() != specs.len() {
        fail(&[table, ": the index enum and the table differ in length"]);
    }
    let mut i = 0;
    while i < specs.len() {
        check_spec(table, &specs[i]);
        if !variants.is_empty() && !variant_matches_key(variants[i], specs[i].key) {
            fail(&[
                table,
                ": variant ",
                variants[i],
                " does not spell key ",
                specs[i].key,
            ]);
        }
        let mut j = i + 1;
        while j < specs.len() {
            if str_eq(specs[i].key, specs[j].key) {
                fail(&[table, ": duplicate key ", specs[i].key]);
            }
            j += 1;
        }
        i += 1;
    }
}

/// Check an enum item table and (when `variants` is not empty) that the Rust enum's variants spell the keys.
/// [`enum_items!`](crate::enum_items) calls this in a `const` item.
pub const fn check_items(table: &str, items: &[EnumItem], variants: &[&str]) {
    if items.len() < 2 {
        fail(&[table, ": an enum needs at least two items"]);
    }
    check_item_keys(table, "", items);
    if !variants.is_empty() && variants.len() != items.len() {
        fail(&[table, ": the Rust enum and the item table differ in length"]);
    }
    let mut i = 0;
    while i < variants.len() {
        if !variant_matches_key(variants[i], items[i].key) {
            fail(&[
                table,
                ": variant ",
                variants[i],
                " does not spell key ",
                items[i].key,
            ]);
        }
        i += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_grammar() {
        assert!(is_valid_key("cutoff"));
        assert!(is_valid_key("low_cut"));
        assert!(is_valid_key("macro8"));
        assert!(!is_valid_key(""));
        assert!(!is_valid_key("Cutoff"));
        assert!(!is_valid_key("1x"));
        assert!(!is_valid_key("cut-off"));
        assert!(!is_valid_key("abcdefghijklmnopqrstuvwxy"));
    }

    #[test]
    fn variant_names() {
        assert!(variant_matches_key("Cutoff", "cutoff"));
        assert!(variant_matches_key("PhaseMode", "phase_mode"));
        assert!(variant_matches_key("SawUp", "saw_up"));
        assert!(variant_matches_key("Lp", "lp"));
        assert!(!variant_matches_key("Cutoff", "resonance"));
        assert!(!variant_matches_key("Cut", "cutoff"));
        assert!(!variant_matches_key("Cutoffs", "cutoff"));
    }

    const WAVES: [EnumItem; 2] = [
        EnumItem::new("saw", "Saw", "Bright."),
        EnumItem::new("sine", "Sine", "Pure."),
    ];

    #[test]
    fn discrete_values_and_norm() {
        let e = ParamSpec::enumeration("wave", "Wave", &WAVES, 1).basic("x");
        assert_eq!((e.min(), e.max()), (0.0, 1.0));
        assert_eq!(e.enum_index("SINE"), Some(1));
        assert_eq!(e.enum_item(0.0).map(|i| i.key), Some("saw"));
        assert_eq!(e.enum_item(0.5), None);
        let i = ParamSpec::int("poly", "Polyphony", Unit::None, 1, 16, 8).basic("x");
        assert_eq!(i.to_norm(1.0), 0.0);
        assert_eq!(i.to_norm(16.0), 1.0);
        assert_eq!(i.from_norm(0.5), 9.0); // 8.5 rounds away from zero
        assert_eq!(i.step_count(), Some(15));
        assert_eq!(i.clamp(3.4), 3.0);
        assert_eq!(i.clamp(40.0), 16.0);
        let t = ParamSpec::toggle("sync", "Sync", false).basic("x");
        assert_eq!(t.from_norm(0.49), 0.0);
        assert_eq!(t.from_norm(0.5), 1.0);
    }

    #[test]
    fn validity() {
        let f = ParamSpec::float("x", "X", Unit::Db, -60.0, 6.0, Scale::Linear, 0.0).basic("x");
        assert!(f.is_valid(0.0));
        assert!(!f.is_valid(-0.0));
        assert!(!f.is_valid(f32::NAN));
        assert!(!f.is_valid(7.0));
        assert_eq!(f.check(-0.0).unwrap().to_bits(), 0.0f32.to_bits());
        assert_eq!(f.check(7.0).unwrap_err().code(), "out_of_range");
        assert_eq!(f.check(f32::INFINITY).unwrap_err().code(), "non_finite");
        let p = ParamSpec::float(
            "phase",
            "Phase",
            Unit::Degrees,
            0.0,
            360.0,
            Scale::Linear,
            0.0,
        )
        .periodic()
        .basic("x");
        assert_eq!(p.clamp(370.0), 10.0);
        assert_eq!(p.clamp(-90.0), 270.0);
    }
}
