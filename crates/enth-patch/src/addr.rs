//! Ids and addresses, and their text forms: module ids (`filter1`), parameter addresses (`filter1.cutoff`,
//! `voice.glide`, `master.level`), modulation sources (`lfo1`, `velocity`, `key`, `modwheel`, `macro3`), routes
//! (`lfo1 -> filter1.cutoff`) and positions (`voice_fx[1]`).
//!
//! Every type here parses with `FromStr`, prints with `Display` (`parse(display(x)) == x`), and serialises as that
//! string. Parsing checks what can be checked without a patch: id grammar, reserved words, and that a parameter key
//! exists for the target (a built-in's own table, or *some* module kind for a module target). Whether `filter1`
//! exists and is a kind with `cutoff` is checked when the address is resolved against a patch (`validate`, ops).
//!
//! Macros and slots are 1-based in text (`macro1`…`macro8`, slots 1–64) and 0-based in memory ([`MacroIndex`],
//! [`SlotIndex`]).

use core::fmt;
use core::str::FromStr;

use crate::catalogue::{Builtin, MasterParam, VoiceParam, module_param_key};
use crate::model::{MACROS, SLOTS, Section};

/// Words that cannot be module ids: the built-ins, the built-in sources, `macro<digits>`, the names `enth get`
/// uses for patch parts (including the four section keys), and the names of **modulation sources that do not exist
/// yet** (`aftertouch`, `pressure`, `bend`, `random`, `gate`, `timbre`). Those are reserved now so that a source
/// added later (a new [`ModSource`] variant) cannot collide with a module somebody already gave that id in a patch
/// that already exists.
pub const RESERVED_IDS: &[&str] = &[
    "voice",
    "master",
    "velocity",
    "key",
    "modwheel",
    "meta",
    "routes",
    "macros",
    "slots",
    "generators",
    "voice_fx",
    "fx",
    "modulators",
    "aftertouch",
    "pressure",
    "bend",
    "random",
    "gate",
    "timbre",
];

/// Whether `id` is a reserved word ([`RESERVED_IDS`] or `macro` followed only by digits).
pub const fn is_reserved_id(id: &str) -> bool {
    let mut i = 0;
    while i < RESERVED_IDS.len() {
        if enth_params::spec::str_eq(id, RESERVED_IDS[i]) {
            return true;
        }
        i += 1;
    }
    let b = id.as_bytes();
    if b.len() > 5 && b[0] == b'm' && b[1] == b'a' && b[2] == b'c' && b[3] == b'r' && b[4] == b'o' {
        let mut j = 5;
        while j < b.len() {
            if !b[j].is_ascii_digit() {
                return false;
            }
            j += 1;
        }
        return true;
    }
    false
}

/// Why a text is not a valid id, address, source, route or position. Each variant has a stable snake-case
/// [`code`](Self::code) for CLI JSON (exit 3) and a [`hint`](Self::hint).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AddrError {
    /// A module id that does not match `[a-z][a-z0-9_]{0,15}`. Code `bad_id`.
    BadId {
        /// The offending text.
        text: String,
    },
    /// A module id that is a reserved word. Code `reserved_id`.
    ReservedId {
        /// The offending text.
        text: String,
    },
    /// Text that does not have the expected shape. Code `bad_address`.
    BadAddress {
        /// The offending text.
        text: String,
        /// The shape expected, e.g. `"<module>.<param>"`.
        expected: &'static str,
    },
    /// A parameter key that the target cannot have. Code `unknown_param`.
    UnknownParam {
        /// The address as written.
        text: String,
    },
    /// A macro, slot or section index out of range. Code `bad_index`.
    BadIndex {
        /// The offending text.
        text: String,
        /// The largest valid index as written (1-based for macros and slots).
        max: usize,
    },
    /// A section name that does not exist. Code `bad_section`.
    BadSection {
        /// The offending text.
        text: String,
    },
}

impl AddrError {
    /// Stable code: `bad_id`, `reserved_id`, `bad_address`, `unknown_param`, `bad_index`, `bad_section`.
    pub const fn code(&self) -> &'static str {
        match self {
            AddrError::BadId { .. } => "bad_id",
            AddrError::ReservedId { .. } => "reserved_id",
            AddrError::BadAddress { .. } => "bad_address",
            AddrError::UnknownParam { .. } => "unknown_param",
            AddrError::BadIndex { .. } => "bad_index",
            AddrError::BadSection { .. } => "bad_section",
        }
    }

    /// What to do about it, in one line.
    pub fn hint(&self) -> String {
        match self {
            AddrError::BadId { .. } => "ids are lowercase letters, digits and _, start with a letter, at most 16 characters (osc1, filter2)".to_owned(),
            AddrError::ReservedId { .. } => format!("pick another id; reserved: {}, macro1..macro8", RESERVED_IDS.join(", ")),
            AddrError::BadAddress { expected, .. } => format!("write {expected}"),
            AddrError::UnknownParam { .. } => "list the keys with `enth params <kind>`".to_owned(),
            // Macros and slots count from 1; a position (`generators[9]`) counts from 0.
            AddrError::BadIndex { text, max } if text.contains('[') => {
                format!("the first position is 0 and the last is {max}")
            }
            AddrError::BadIndex { max, .. } => format!("use 1..={max}"),
            AddrError::BadSection { .. } => {
                let keys: Vec<&str> = Section::ALL.iter().map(|s| s.key()).collect();
                format!("sections are {}", keys.join(", "))
            }
        }
    }
}

impl fmt::Display for AddrError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (what, text) = match self {
            AddrError::BadId { text } => ("is not a valid module id", text),
            AddrError::ReservedId { text } => ("is a reserved word", text),
            AddrError::BadAddress { text, .. } => ("is not a valid address", text),
            AddrError::UnknownParam { text } => ("names no parameter", text),
            AddrError::BadIndex { text, .. } => ("is out of range", text),
            AddrError::BadSection { text } => ("is not a section", text),
        };
        write!(f, "'{text}' {what}; {}", self.hint())
    }
}

impl std::error::Error for AddrError {}

// ---------------------------------------------------------------------------------------------------------------
// ModuleId
// ---------------------------------------------------------------------------------------------------------------

/// A module's stable id: `[a-z][a-z0-9_]{0,15}`, not a reserved word ([`is_reserved_id`]). Unique within a patch
/// (checked by `validate`). Seeds follow ids (`fnv1a64(id)`), so renaming a module changes its random streams.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ModuleId(String);

impl ModuleId {
    /// The longest id.
    pub const MAX_LEN: usize = 16;

    /// A checked id.
    pub fn new(id: &str) -> Result<ModuleId, AddrError> {
        if id.len() > Self::MAX_LEN || !enth_params::is_valid_key(id) {
            return Err(AddrError::BadId {
                text: id.to_owned(),
            });
        }
        if is_reserved_id(id) {
            return Err(AddrError::ReservedId {
                text: id.to_owned(),
            });
        }
        Ok(ModuleId(id.to_owned()))
    }

    /// `<prefix><n>` (`filter2`): how `add` names a module without an id (the lowest free `n` from 1), and how
    /// `canonical_hash` relabels.
    pub fn numbered(prefix: &str, n: u32) -> Result<ModuleId, AddrError> {
        ModuleId::new(&format!("{prefix}{n}"))
    }

    /// The id as text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ModuleId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for ModuleId {
    type Err = AddrError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        ModuleId::new(s)
    }
}

impl AsRef<str> for ModuleId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl core::borrow::Borrow<str> for ModuleId {
    fn borrow(&self) -> &str {
        &self.0
    }
}

// ---------------------------------------------------------------------------------------------------------------
// ParamAddr
// ---------------------------------------------------------------------------------------------------------------

/// What a parameter address points into: a module, or one of the built-ins.
///
/// The derived order (modules, then `voice`, then `master`) is part of how `canonical_hash` sorts routes, so new
/// variants go at the end.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Target {
    /// A module instance (`filter1`).
    Module(ModuleId),
    /// The built-in `voice`.
    Voice,
    /// The built-in `master`.
    Master,
}

impl fmt::Display for Target {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Target::Module(id) => f.write_str(id.as_str()),
            Target::Voice => f.write_str(Builtin::Voice.key()),
            Target::Master => f.write_str(Builtin::Master.key()),
        }
    }
}

/// A parameter address: `filter1.cutoff`, `voice.glide`, `master.level`.
///
/// The key is the catalogue's own `&'static str`, so an address can only name a key that exists: in the
/// built-in's table for `voice`/`master`, in some module kind for a module target. Fields are private to keep that
/// invariant; build one with [`ParamAddr::new`], [`ParamAddr::voice`], [`ParamAddr::master`] or by parsing.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ParamAddr {
    target: Target,
    param: &'static str,
}

impl ParamAddr {
    /// A checked address. Fails with `unknown_param` if the target cannot have `param`.
    pub fn new(target: Target, param: &str) -> Result<ParamAddr, AddrError> {
        let key = match &target {
            Target::Voice => Builtin::Voice
                .param_index(param)
                .map(|i| Builtin::Voice.params()[i].key),
            Target::Master => Builtin::Master
                .param_index(param)
                .map(|i| Builtin::Master.params()[i].key),
            Target::Module(_) => module_param_key(param),
        };
        match key {
            Some(param) => Ok(ParamAddr { target, param }),
            None => Err(AddrError::UnknownParam {
                text: format!("{target}.{param}"),
            }),
        }
    }

    /// `voice.<param>`.
    pub fn voice(param: VoiceParam) -> ParamAddr {
        ParamAddr {
            target: Target::Voice,
            param: param.key(),
        }
    }

    /// `master.<param>`.
    pub fn master(param: MasterParam) -> ParamAddr {
        ParamAddr {
            target: Target::Master,
            param: param.key(),
        }
    }

    /// `<id>.<param>` for a module, with the key checked against every module kind.
    pub fn module(id: ModuleId, param: &str) -> Result<ParamAddr, AddrError> {
        ParamAddr::new(Target::Module(id), param)
    }

    /// The target.
    pub fn target(&self) -> &Target {
        &self.target
    }

    /// The parameter key (the catalogue's static string).
    pub fn param(&self) -> &'static str {
        self.param
    }

    /// The module id, if the target is a module.
    pub fn module_id(&self) -> Option<&ModuleId> {
        match &self.target {
            Target::Module(id) => Some(id),
            _ => None,
        }
    }
}

impl fmt::Display for ParamAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}", self.target, self.param)
    }
}

impl FromStr for ParamAddr {
    type Err = AddrError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let t = s.trim();
        let (target, param) = t.split_once('.').ok_or_else(|| AddrError::BadAddress {
            text: t.to_owned(),
            expected: "<module>.<param>, e.g. filter1.cutoff",
        })?;
        let target = match target {
            "voice" => Target::Voice,
            "master" => Target::Master,
            id => Target::Module(ModuleId::new(id)?),
        };
        ParamAddr::new(target, param)
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Indices
// ---------------------------------------------------------------------------------------------------------------

/// A macro index, 0-based (`0..8`); `macro1`…`macro8` in text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MacroIndex(u8);

impl MacroIndex {
    /// A 0-based index, if `< 8`.
    pub const fn new(index: usize) -> Option<MacroIndex> {
        if index < MACROS {
            Some(MacroIndex(index as u8))
        } else {
            None
        }
    }

    /// From the 1-based number used in text (`1..=8`).
    pub const fn from_one_based(n: usize) -> Option<MacroIndex> {
        if n >= 1 { MacroIndex::new(n - 1) } else { None }
    }

    /// The 0-based index.
    pub const fn get(self) -> usize {
        self.0 as usize
    }

    /// The 1-based number used in text.
    pub const fn one_based(self) -> usize {
        self.0 as usize + 1
    }
}

/// A host automation slot index, 0-based (`0..64`); 1–64 in text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SlotIndex(u8);

impl SlotIndex {
    /// A 0-based index, if `< 64`.
    pub const fn new(index: usize) -> Option<SlotIndex> {
        if index < SLOTS {
            Some(SlotIndex(index as u8))
        } else {
            None
        }
    }

    /// From the 1-based number used in text (`1..=64`).
    pub const fn from_one_based(n: usize) -> Option<SlotIndex> {
        if n >= 1 { SlotIndex::new(n - 1) } else { None }
    }

    /// The 0-based index.
    pub const fn get(self) -> usize {
        self.0 as usize
    }

    /// The 1-based number used in text.
    pub const fn one_based(self) -> usize {
        self.0 as usize + 1
    }
}

// ---------------------------------------------------------------------------------------------------------------
// ModSource
// ---------------------------------------------------------------------------------------------------------------

/// A modulation source. Polarity belongs to the source (design.md §2): envelopes and velocity run 0–1, an LFO is
/// bipolar or unipolar (its `polarity`), `key` is `(note − 60) / 120`, the mod wheel and macros run 0–1.
///
/// Velocity and key are per voice; the mod wheel and macros are global; a module source follows its rate. Macros
/// are global in P1a; with CLAP polyphonic modulation (P1b) they gain a per-voice offset.
///
/// The derived order is part of how `canonical_hash` sorts routes, so new variants go at the end.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ModSource {
    /// A modulator module (`env1`, `lfo1`).
    Module(ModuleId),
    /// Note velocity, 0–1 (`velocity`).
    Velocity,
    /// Note number, `(note − 60) / 120` (`key`).
    Key,
    /// The mod wheel, 0–1 (`modwheel`).
    ModWheel,
    /// A macro knob, 0–1 (`macro1`…`macro8`).
    Macro(MacroIndex),
}

impl fmt::Display for ModSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ModSource::Module(id) => f.write_str(id.as_str()),
            ModSource::Velocity => f.write_str("velocity"),
            ModSource::Key => f.write_str("key"),
            ModSource::ModWheel => f.write_str("modwheel"),
            ModSource::Macro(i) => write!(f, "macro{}", i.one_based()),
        }
    }
}

impl FromStr for ModSource {
    type Err = AddrError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let t = s.trim();
        Ok(match t {
            "velocity" => ModSource::Velocity,
            "key" => ModSource::Key,
            "modwheel" => ModSource::ModWheel,
            _ => match t.strip_prefix("macro") {
                Some(n) if !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) => {
                    let index = n.parse::<usize>().ok().and_then(MacroIndex::from_one_based);
                    ModSource::Macro(index.ok_or_else(|| AddrError::BadIndex {
                        text: t.to_owned(),
                        max: MACROS,
                    })?)
                }
                _ => ModSource::Module(ModuleId::new(t)?),
            },
        })
    }
}

// ---------------------------------------------------------------------------------------------------------------
// RouteAddr
// ---------------------------------------------------------------------------------------------------------------

/// A route's identity without its depth: `lfo1 -> filter1.cutoff`. At most one route per (src, dst) exists in a
/// valid patch, so this names a route (for `unroute`, and as the key ops use).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RouteAddr {
    /// The source.
    pub src: ModSource,
    /// The destination.
    pub dst: ParamAddr,
}

impl fmt::Display for RouteAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} -> {}", self.src, self.dst)
    }
}

impl FromStr for RouteAddr {
    type Err = AddrError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let t = s.trim();
        let (src, dst) = t.split_once("->").ok_or_else(|| AddrError::BadAddress {
            text: t.to_owned(),
            expected: "<source> -> <module>.<param>, e.g. lfo1 -> filter1.cutoff",
        })?;
        Ok(RouteAddr {
            src: src.parse()?,
            dst: dst.parse()?,
        })
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Position
// ---------------------------------------------------------------------------------------------------------------

/// A place in a section: `voice_fx[1]` (0-based). Where `AddModule` inserts and `MoveModule` moves to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Position {
    /// The section.
    pub section: Section,
    /// The index within the section.
    pub index: usize,
}

impl fmt::Display for Position {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}[{}]", self.section, self.index)
    }
}

impl FromStr for Position {
    type Err = AddrError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let t = s.trim();
        let bad = || AddrError::BadAddress {
            text: t.to_owned(),
            expected: "<section>[<index>], e.g. voice_fx[0]",
        };
        let (section, rest) = t.split_once('[').ok_or_else(bad)?;
        let index = rest.strip_suffix(']').ok_or_else(bad)?;
        let section = Section::from_key(section.trim()).ok_or_else(|| AddrError::BadSection {
            text: section.trim().to_owned(),
        })?;
        if index.is_empty() || !index.bytes().all(|b| b.is_ascii_digit()) {
            return Err(bad());
        }
        let index = index
            .parse::<usize>()
            .ok()
            .filter(|&i| i < section.max_len());
        let index = index.ok_or_else(|| AddrError::BadIndex {
            text: t.to_owned(),
            max: section.max_len() - 1,
        })?;
        Ok(Position { section, index })
    }
}

// ---------------------------------------------------------------------------------------------------------------
// serde: every address type is its text form
// ---------------------------------------------------------------------------------------------------------------

macro_rules! serde_as_text {
    ($($t:ty),+) => {$(
        impl serde::Serialize for $t {
            fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                s.collect_str(self)
            }
        }

        impl<'de> serde::Deserialize<'de> for $t {
            fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                let s = <std::borrow::Cow<'de, str>>::deserialize(d)?;
                s.parse().map_err(serde::de::Error::custom)
            }
        }
    )+};
}

serde_as_text!(ModuleId, ParamAddr, ModSource, RouteAddr, Position);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids() {
        assert!(ModuleId::new("osc1").is_ok());
        assert!(ModuleId::new("my_lfo_2").is_ok());
        assert_eq!(ModuleId::new("Osc1").unwrap_err().code(), "bad_id");
        assert_eq!(ModuleId::new("1osc").unwrap_err().code(), "bad_id");
        assert_eq!(
            ModuleId::new("abcdefghijklmnopq").unwrap_err().code(),
            "bad_id"
        );
        assert_eq!(ModuleId::new("voice").unwrap_err().code(), "reserved_id");
        assert_eq!(ModuleId::new("macro3").unwrap_err().code(), "reserved_id");
        assert_eq!(ModuleId::new("macro12").unwrap_err().code(), "reserved_id");
        assert_eq!(ModuleId::new("fx").unwrap_err().code(), "reserved_id");
        // Sources that do not exist yet are reserved already (S1b); their neighbours are free.
        for id in ["aftertouch", "pressure", "bend", "random", "gate", "timbre"] {
            assert_eq!(ModuleId::new(id).unwrap_err().code(), "reserved_id", "{id}");
        }
        for id in [
            "random1",
            "gate_2",
            "bend2",
            "pressure_",
            "aftertouch1",
            "timbres",
        ] {
            assert!(ModuleId::new(id).is_ok(), "{id}");
        }
        assert!(ModuleId::new("fx1").is_ok());
        assert!(ModuleId::new("macro").is_ok());
        assert!(ModuleId::new("macro_a").is_ok());
        assert_eq!(ModuleId::numbered("filter", 2).unwrap().as_str(), "filter2");
    }

    #[test]
    fn addresses_round_trip() {
        for text in [
            "filter1.cutoff",
            "voice.glide",
            "master.level",
            "osc2.phase_mode",
        ] {
            let a: ParamAddr = text.parse().unwrap();
            assert_eq!(a.to_string(), text);
        }
        assert_eq!(
            "voice.polyphony".parse::<ParamAddr>().unwrap(),
            ParamAddr::voice(VoiceParam::Polyphony)
        );
        assert_eq!(
            " master.level ".parse::<ParamAddr>().unwrap(),
            ParamAddr::master(MasterParam::Level)
        );
        assert_eq!(
            "filter1.cutof".parse::<ParamAddr>().unwrap_err().code(),
            "unknown_param"
        );
        assert_eq!(
            "voice.cutoff".parse::<ParamAddr>().unwrap_err().code(),
            "unknown_param"
        );
        assert_eq!(
            "master.glide".parse::<ParamAddr>().unwrap_err().code(),
            "unknown_param"
        );
        assert_eq!(
            "filter1".parse::<ParamAddr>().unwrap_err().code(),
            "bad_address"
        );
        assert_eq!(
            "Filter1.cutoff".parse::<ParamAddr>().unwrap_err().code(),
            "bad_id"
        );
        assert_eq!(
            "velocity.cutoff".parse::<ParamAddr>().unwrap_err().code(),
            "reserved_id"
        );
    }

    #[test]
    fn sources_routes_positions() {
        for text in ["lfo1", "velocity", "key", "modwheel", "macro1", "macro8"] {
            assert_eq!(text.parse::<ModSource>().unwrap().to_string(), text);
        }
        assert_eq!(
            "macro3".parse::<ModSource>().unwrap(),
            ModSource::Macro(MacroIndex::new(2).unwrap())
        );
        assert_eq!(
            "macro9".parse::<ModSource>().unwrap_err().code(),
            "bad_index"
        );
        assert_eq!(
            "macro0".parse::<ModSource>().unwrap_err().code(),
            "bad_index"
        );
        let r: RouteAddr = "lfo1->filter1.cutoff".parse().unwrap();
        assert_eq!(r.to_string(), "lfo1 -> filter1.cutoff");
        assert_eq!(
            "lfo1 filter1.cutoff"
                .parse::<RouteAddr>()
                .unwrap_err()
                .code(),
            "bad_address"
        );
        let p: Position = "voice_fx[1]".parse().unwrap();
        assert_eq!((p.section, p.index), (Section::VoiceFx, 1));
        assert_eq!(p.to_string(), "voice_fx[1]");
        assert_eq!(
            "effects[0]".parse::<Position>().unwrap_err().code(),
            "bad_section"
        );
        assert_eq!("fx[4]".parse::<Position>().unwrap_err().code(), "bad_index");
        assert_eq!(
            "fx[-1]".parse::<Position>().unwrap_err().code(),
            "bad_address"
        );
        assert_eq!("fx".parse::<Position>().unwrap_err().code(), "bad_address");
    }

    #[test]
    fn indices() {
        assert_eq!(MacroIndex::from_one_based(8).map(MacroIndex::get), Some(7));
        assert_eq!(MacroIndex::from_one_based(0), None);
        assert_eq!(MacroIndex::new(8), None);
        assert_eq!(SlotIndex::from_one_based(64).map(SlotIndex::get), Some(63));
        assert_eq!(SlotIndex::new(64), None);
    }

    #[test]
    fn serde_uses_text() {
        let a: ParamAddr = "filter1.cutoff".parse().unwrap();
        assert_eq!(serde_json::to_string(&a).unwrap(), "\"filter1.cutoff\"");
        assert_eq!(
            serde_json::from_str::<ParamAddr>("\"voice.glide\"").unwrap(),
            ParamAddr::voice(VoiceParam::Glide)
        );
        assert!(serde_json::from_str::<ParamAddr>("\"voice.nope\"").is_err());
        let r: RouteAddr = serde_json::from_str("\"macro2 -> osc1.semi\"").unwrap();
        assert_eq!(r.src, ModSource::Macro(MacroIndex::new(1).unwrap()));
    }
}
