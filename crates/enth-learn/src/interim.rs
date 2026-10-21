//! **Owner: lane LM. Interim, deleted at the F1l merge.** Stand-ins for three names lane CV2 is landing in
//! `enth-challenge` on its own branch, so this branch builds before CV2 merges. The coordinator ruled (F1l seams)
//! that the lock selector is CV2's type and that this crate codes against these exact names; when CV2 is on main,
//! each item below becomes a one-line `pub use` of CV2's and this file goes:
//!
//! | Here | CV2's |
//! | --- | --- |
//! | [`Selector`] | `enth_challenge::v2::Selector` (same variants, same text form, so the lesson format does not move) |
//! | [`Touch`] and [`Selector::covers`] | `enth_challenge::v2::Touch`, `Selector::covers` |
//! | [`is_known_scorer`] | `enth_challenge::score::is_known` |
//! | [`phrase`] | `enth_challenge::phrases::phrase` once it also finds the lesson-only ids 4 to 7 |
//!
//! The text grammar is the one LM proposed and CV2 adopted: `filter1.cutoff`, `voice.glide`, `master.level` (a
//! parameter), `filter1.*`, `voice.*`, `master.*` (a whole module or built-in: its parameters, the module itself,
//! and routes from it or into it), `macro3` (a macro knob), `lfo1 -> filter1.cutoff` (a route). No kind-level
//! selectors in v2.

use core::fmt;
use core::str::FromStr;

use enth_challenge::{Phrase, PhraseId, ScorerId};
use enth_patch::{AddrError, MacroIndex, ModSource, ModuleId, ParamAddr, RouteAddr, Target};

/// What a lock, an editable set or a highlight names. See the [module docs](self) for the text form.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Selector {
    /// One parameter: `filter1.cutoff`.
    Param(ParamAddr),
    /// A whole module or built-in: `filter1.*`, `voice.*`.
    Module(Target),
    /// One route: `lfo1 -> filter1.cutoff`.
    Route(RouteAddr),
    /// A macro knob: `macro3`.
    Macro(MacroIndex),
}

/// What an edit touches, for [`Selector::covers`].
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Touch {
    /// A parameter's value.
    Param(ParamAddr),
    /// A route (insert, remove, depth).
    Route(RouteAddr),
    /// A module itself (add, remove, move, enable).
    Module(ModuleId),
    /// A macro knob.
    Macro(MacroIndex),
}

impl Selector {
    /// Whether this selector names what `touch` changes. `m.*` covers the module's parameters, the module itself and
    /// routes from it or into it.
    pub fn covers(&self, touch: &Touch) -> bool {
        match (self, touch) {
            (Selector::Param(a), Touch::Param(b)) => a == b,
            (Selector::Route(a), Touch::Route(b)) => a == b,
            (Selector::Macro(a), Touch::Macro(b)) => a == b,
            (Selector::Module(t), Touch::Param(p)) => p.target() == t,
            (Selector::Module(t), Touch::Module(id)) => matches!(t, Target::Module(m) if m == id),
            (Selector::Module(t), Touch::Route(r)) => {
                r.dst.target() == t
                    || matches!((t, &r.src), (Target::Module(m), ModSource::Module(s)) if m == s)
            }
            _ => false,
        }
    }
}

impl fmt::Display for Selector {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Selector::Param(a) => write!(f, "{a}"),
            Selector::Module(t) => write!(f, "{t}.*"),
            Selector::Route(r) => write!(f, "{r}"),
            Selector::Macro(i) => write!(f, "macro{}", i.one_based()),
        }
    }
}

impl FromStr for Selector {
    type Err = AddrError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let t = s.trim();
        if t.contains("->") {
            return t.parse().map(Selector::Route);
        }
        if let Some(target) = t.strip_suffix(".*") {
            return Ok(Selector::Module(match target {
                "voice" => Target::Voice,
                "master" => Target::Master,
                id => Target::Module(ModuleId::new(id)?),
            }));
        }
        if !t.contains('.')
            && let Ok(ModSource::Macro(i)) = t.parse::<ModSource>()
        {
            return Ok(Selector::Macro(i));
        }
        t.parse().map(Selector::Param)
    }
}

impl serde::Serialize for Selector {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> serde::Deserialize<'de> for Selector {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = <std::borrow::Cow<'de, str>>::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

/// Whether this build scores with `id` (CV2's `score::is_known`, pinned there against the dispatch). Until then:
/// this build's default scorers and the three stubs, which the dispatch has.
pub fn is_known_scorer(id: &ScorerId) -> bool {
    enth_challenge::model::RECREATE_SCORERS.contains(id)
        || [
            ScorerId::SOUND_STUB,
            ScorerId::STRUCTURE_STUB,
            ScorerId::COMBINED_STUB,
        ]
        .contains(id)
}

/// The phrase with this id, from the generator's table (CV2 extends the lookup to its lesson-only ids 4 to 7).
pub fn phrase(id: PhraseId) -> Option<&'static Phrase> {
    enth_challenge::phrases::phrase(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_text_form_round_trips() {
        for text in [
            "filter1.cutoff",
            "voice.glide",
            "master.level",
            "filter1.*",
            "voice.*",
            "master.*",
            "macro3",
            "lfo1 -> filter1.cutoff",
            "macro1 -> filter1.cutoff",
        ] {
            let s: Selector = text.parse().unwrap();
            assert_eq!(s.to_string(), text);
        }
        assert!("macro9".parse::<Selector>().is_err());
        assert!("filter1".parse::<Selector>().is_err());
        assert!("Filter1.cutoff".parse::<Selector>().is_err());
    }

    #[test]
    fn a_module_selector_covers_its_params_itself_and_its_routes() {
        let m: Selector = "lfo1.*".parse().unwrap();
        let route: RouteAddr = "lfo1 -> filter1.cutoff".parse().unwrap();
        assert!(m.covers(&Touch::Route(route.clone())));
        assert!(m.covers(&Touch::Param("lfo1.rate".parse().unwrap())));
        assert!(m.covers(&Touch::Module(ModuleId::new("lfo1").unwrap())));
        assert!(!m.covers(&Touch::Param("filter1.cutoff".parse().unwrap())));
        let f: Selector = "filter1.*".parse().unwrap();
        assert!(f.covers(&Touch::Route(route)));
    }
}
