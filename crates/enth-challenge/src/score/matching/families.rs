//! **Owner: lane MTC** (S2a wave 3). The kind families of `structure/1`'s module matching: which kinds may stand in
//! for each other, with what credit, and which parameters correspond. The pairing itself is
//! [`super`]; this file is the data and the rules it reads.
//!
//! # Why families
//!
//! The matcher pairs modules *per kind*. Before S2a every kind of the catalogue had its own pass, so a kind that
//! nobody listed was never paired at all: an S2a `dist` in both patches was a missing module and an extra one, and
//! the identical patch scored below 100. [`PASSES`] closes that hole (every kind has its pass; a test checks the
//! list against `ModuleKind::ALL`, so a kind added later cannot be forgotten silently). Families are the second
//! half: a *substitute* of a close kind should earn part of the credit instead of scoring 0 twice (once as a
//! missing module, once as an extra one).
//!
//! # The rule
//!
//! 1. **A module pairs with one of its own kind first**, exactly as before (the exact minimum-cost assignment of
//!    [`super`]'s module docs). Nothing in this file changes a pair of the same kind.
//! 2. **What is left over fills gaps by family**: a target module nobody of its kind paired with can take an
//!    answer module of a linked kind that nobody paired with. The passes run in a fixed order ([`links`]'s
//!    preference), each an exact assignment between two kinds, so the result is deterministic.
//! 3. **A substitute scores in two ways.** (a) Every parameter of the target module is read against the answer
//!    module's *corresponding* parameter ([`counterpart_key`]): the same key, only where the link lists it as
//!    meaning the same on both kinds. A parameter with no counterpart is infinitely far, like a missing module's.
//!    (b) The pair itself is one more item of the structure score, worth the link's [`Tier`] credit, because
//!    structure is "which modules exist": a comb for a filter is not the filter.
//! 4. **Families never cross a section**, and **no v1 kind is linked to another v1 kind**, so a pair of v1-only
//!    patches never meets a substitute: every `structure/1` score for such a pair is the one it always was
//!    (`tests/golden/tests/st_structure.rs` and `st_properties.rs` pin it).
//!
//! # The table
//!
//! | Link | Credit | Shared parameters | Why |
//! | --- | --- | --- | --- |
//! | `osc` ~ `wt` | related, 0.5 | `semi`, `fine`, `level`, `position`, `phase_mode`, `phase`, `seed` | Both are phase-read tone generators with the same tuning and level. `wave` and `table` are different lists (a table's `basic` and `formant` are not the oscillator's), so the choice of sound is not credited |
//! | `svf` ~ `ladder` | close, 0.75 | `mode`, `cutoff`, `resonance`, `keytrack`, `drive` | CAT made the ladder's keys mirror the filter's so a patch can swap one for the other. `mode` compares by item name, and the filter's `lp` (12 dB) equals the ladder's `lp12`; `lp24`, `notch` have no twin |
//! | `svf` ~ `formant` | loose, 0.25 | `resonance` | A filter slot with a vowel filter: a resonant band shaping, but the cutoff has no meaning on it |
//! | `ladder` ~ `formant` | loose, 0.25 | `resonance`, `mix` | As above |
//! | `svf` ~ `comb` | loose, 0.25 | none | A comb is a tuned resonator; none of the filter's controls means what the comb's do (`keytrack` moves a cutoff on one and the comb's pitch on the other, so it is not shared) |
//! | `ladder` ~ `comb` | loose, 0.25 | `mix` | As above |
//! | `comb` ~ `formant` | loose, 0.25 | `mix` | Two resonant colourings of the voice |
//! | `comp` ~ `ott` | related, 0.5 | `mix` | OTT is a three-band upward and downward compressor: the same job (dynamics) with other controls, so only the blend is shared |
//! | `chorus` ~ `phaser` | close, 0.75 | `rate`, `sync`, `beats`, `depth`, `feedback`, `width`, `mix` | One modulated-delay core (lane MOD) with a swept comb or notch: the rate, depth, feedback, width and blend mean the same |
//! | `chorus` ~ `delay`, `phaser` ~ `delay` | loose, 0.25 | `sync`, `beats`, `feedback`, `mix` | All are time-based effects (`ModuleSpec::time_based`) with a fed-back, optionally tempo-synced delay line; an echo is not a modulation, so only the shared timing and blend are credited |
//! | `lfo` ~ `mseg` | close, 0.75 | `polarity`, `scope`, `phase`, `sync`, `beats` | A looping MSEG is an LFO with a drawn shape; the shape and rate lists are not shared |
//! | `adsr` ~ `mseg` | related, 0.5 | `velocity` | A one-shot MSEG is an envelope (the long one: CAT's decision 2). The amplitude envelope stays an `adsr` (validation rule 5), and it is forced to pair with the answer's, so this link serves the other envelopes |
//!
//! **Alone**, linked to nothing: `noise` (a generator without pitch), `reverb`, `dist` (a curve, not a filter: a
//! filter's `drive` is a parameter, not a module, and one `dist` is not a substitute for it), and `eq` (six fixed
//! bands, nothing like one filter). A target `dist` the answer lacks is missing, as before.
//!
//! The credits are beta, like the tolerances: GR1's disputes tune them. They are ordered, not measured: a close
//! link keeps the role and most of the controls, a related one keeps the role, a loose one keeps only the slot.

use enth_patch::catalogue::ModuleKind;
use enth_patch::catalogue::ModuleKind::{
    Adsr, Chorus, Comb, Comp, Delay, Dist, Eq, Formant, Ladder, Lfo, Mseg, Noise, Osc, Ott, Phaser,
    Reverb, Svf, Wt,
};

/// How much of a module pair's credit a substitute earns: how close the answer's kind is to the target's.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Tier {
    /// The slot, a few controls: a filter that is not the same filter.
    Loose,
    /// The role, other controls.
    Related,
    /// The role and most of the controls.
    Close,
}

impl Tier {
    /// The item score of a substituted pair (an item of the structure score, like a ring: 1 is exact).
    pub const fn credit(self) -> f64 {
        match self {
            Tier::Loose => 0.25,
            Tier::Related => 0.5,
            Tier::Close => 0.75,
        }
    }
}

/// One link of the table: two kinds of a section that may stand in for each other.
#[derive(Clone, Copy, Debug)]
pub struct Link {
    /// One kind.
    pub a: ModuleKind,
    /// The other kind.
    pub b: ModuleKind,
    /// How much credit a substitution earns.
    pub tier: Tier,
    /// The keys that mean the same on both kinds (the same key on each).
    pub shared: &'static [&'static str],
}

/// The links, in preference order: when several substitutes are free for a target module, the earlier link is
/// tried first. Symmetric (a link serves both ways).
pub const LINKS: &[Link] = &[
    Link {
        a: Osc,
        b: Wt,
        tier: Tier::Related,
        shared: &[
            "semi",
            "fine",
            "level",
            "position",
            "phase_mode",
            "phase",
            "seed",
        ],
    },
    Link {
        a: Svf,
        b: Ladder,
        tier: Tier::Close,
        shared: &["mode", "cutoff", "resonance", "keytrack", "drive"],
    },
    Link {
        a: Svf,
        b: Formant,
        tier: Tier::Loose,
        shared: &["resonance"],
    },
    Link {
        a: Ladder,
        b: Formant,
        tier: Tier::Loose,
        shared: &["resonance", "mix"],
    },
    Link {
        a: Svf,
        b: Comb,
        tier: Tier::Loose,
        shared: &[],
    },
    Link {
        a: Ladder,
        b: Comb,
        tier: Tier::Loose,
        shared: &["mix"],
    },
    Link {
        a: Comb,
        b: Formant,
        tier: Tier::Loose,
        shared: &["mix"],
    },
    Link {
        a: Comp,
        b: Ott,
        tier: Tier::Related,
        shared: &["mix"],
    },
    Link {
        a: Chorus,
        b: Phaser,
        tier: Tier::Close,
        shared: &[
            "rate", "sync", "beats", "depth", "feedback", "width", "mix",
        ],
    },
    Link {
        a: Chorus,
        b: Delay,
        tier: Tier::Loose,
        shared: &["sync", "beats", "feedback", "mix"],
    },
    Link {
        a: Phaser,
        b: Delay,
        tier: Tier::Loose,
        shared: &["sync", "beats", "feedback", "mix"],
    },
    Link {
        a: Lfo,
        b: Mseg,
        tier: Tier::Close,
        shared: &["polarity", "scope", "phase", "sync", "beats"],
    },
    Link {
        a: Adsr,
        b: Mseg,
        tier: Tier::Related,
        shared: &["velocity"],
    },
];

/// The generators, in the order their passes run (the first two are generator v1's).
pub const GENERATORS: &[ModuleKind] = &[Osc, Noise, Wt];

/// The effects (voice effects and global ones), in the order their passes run (`svf`, `reverb`, `comp` are v1's).
pub const EFFECTS: &[ModuleKind] = &[
    Svf, Reverb, Comp, Dist, Ladder, Comb, Formant, Ott, Chorus, Phaser, Delay, Eq,
];

/// The modulators, in the order their passes run (`adsr` and `lfo` are v1's; the order among them matters, because
/// a route onto a modulator reads the pairs found so far).
pub const MODULATORS: &[ModuleKind] = &[Adsr, Lfo, Mseg];

/// The link between two kinds, either way round.
pub fn link(x: ModuleKind, y: ModuleKind) -> Option<&'static Link> {
    LINKS
        .iter()
        .find(|l| (l.a == x && l.b == y) || (l.a == y && l.b == x))
}

/// The links in the order their substitutions are tried: the closest tier first, the table's order within a tier.
pub fn ranked() -> Vec<&'static Link> {
    let mut links: Vec<&'static Link> = LINKS.iter().collect();
    links.sort_by_key(|l| std::cmp::Reverse(l.tier));
    links
}

/// The kinds that can stand in for `kind`, in the order their passes try them (the table's order).
pub fn substitutes(kind: ModuleKind) -> impl Iterator<Item = ModuleKind> {
    LINKS.iter().filter_map(move |l| {
        if l.a == kind {
            Some(l.b)
        } else if l.b == kind {
            Some(l.a)
        } else {
            None
        }
    })
}

/// The key of `answer`'s parameter that corresponds to the target kind's parameter `key`: the key itself, when the
/// link lists it as shared. `None` for a parameter nothing corresponds to, and for two kinds that are not linked.
/// For equal kinds, the parameter itself.
pub fn counterpart_key(target: ModuleKind, key: &str, answer: ModuleKind) -> Option<&'static str> {
    if target == answer {
        return target.param(key).map(|spec| spec.key);
    }
    let link = link(target, answer)?;
    link.shared
        .contains(&key)
        .then(|| answer.param(key).map(|spec| spec.key))?
}

/// Whether the target's enumeration item `t` is the answer's item `a`: the same key, or a 12 dB low-pass for the
/// ladder's 12 dB mode (`lp` ≡ `lp12`; `lp24` and the others have no twin).
pub fn same_item(t: &str, a: &str) -> bool {
    t == a || matches!((t, a), ("lp", "lp12") | ("lp12", "lp"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use enth_params::Kind;
    use std::collections::BTreeSet;

    /// Every kind has exactly one pass, in its own section's list: a kind added to the catalogue without a pass
    /// would never pair (the hole this module exists to close).
    #[test]
    fn every_kind_has_exactly_one_pass_in_its_section() {
        let mut seen: BTreeSet<ModuleKind> = BTreeSet::new();
        for (list, section) in [
            (GENERATORS, enth_patch::Section::Generators),
            (EFFECTS, enth_patch::Section::Fx),
            (MODULATORS, enth_patch::Section::Modulators),
        ] {
            for &kind in list {
                let ok = if section == enth_patch::Section::Fx {
                    matches!(
                        kind.section(),
                        enth_patch::Section::Fx | enth_patch::Section::VoiceFx
                    )
                } else {
                    kind.section() == section
                };
                assert!(ok, "{kind} is in the wrong pass list");
                assert!(seen.insert(kind), "{kind} has two passes");
            }
        }
        let all: BTreeSet<ModuleKind> = ModuleKind::ALL.iter().copied().collect();
        assert_eq!(seen, all, "a kind has no pass");
    }

    #[test]
    fn links_stay_inside_a_section_and_never_join_two_v1_kinds() {
        let v1 = [Osc, Noise, Svf, Adsr, Lfo, Reverb, Comp];
        let mut pairs = BTreeSet::new();
        for l in LINKS {
            assert_ne!(l.a, l.b);
            assert_eq!(l.a.section(), l.b.section(), "{}~{}", l.a, l.b);
            assert!(
                !(v1.contains(&l.a) && v1.contains(&l.b)),
                "{}~{} would change a v1 pairing",
                l.a,
                l.b
            );
            assert!(
                pairs.insert((l.a.min(l.b), l.a.max(l.b))),
                "{}~{} is listed twice",
                l.a,
                l.b
            );
        }
    }

    /// A shared key exists on both kinds and means the same kind of value in the same unit (the structure score
    /// reads a corresponding pair in the target's just-noticeable steps, which only makes sense then). An
    /// enumeration's items are compared by name, so both must be enumerations.
    #[test]
    fn shared_keys_exist_on_both_kinds_with_one_unit() {
        for l in LINKS {
            for key in l.shared {
                let (x, y) = (
                    l.a.param(key).unwrap_or_else(|| panic!("{} lacks {key}", l.a)),
                    l.b.param(key).unwrap_or_else(|| panic!("{} lacks {key}", l.b)),
                );
                assert_eq!(x.unit, y.unit, "{}~{} {key}: unit", l.a, l.b);
                assert_eq!(
                    std::mem::discriminant(&x.kind),
                    std::mem::discriminant(&y.kind),
                    "{}~{} {key}: kind of value",
                    l.a,
                    l.b
                );
                if let (Kind::Float { .. }, Kind::Float { .. }) = (x.kind, y.kind) {
                    assert_eq!(
                        x.flags.periodic, y.flags.periodic,
                        "{}~{} {key}: periodic",
                        l.a, l.b
                    );
                }
            }
        }
    }

    /// The correspondence is symmetric, so a ring on the player's control and the target's reading agree.
    #[test]
    fn the_correspondence_is_symmetric() {
        for l in LINKS {
            for spec in l.a.params() {
                let fwd = counterpart_key(l.a, spec.key, l.b);
                if let Some(k) = fwd {
                    assert_eq!(counterpart_key(l.b, k, l.a), Some(spec.key));
                }
            }
        }
        // And it is the identity for one kind.
        assert_eq!(counterpart_key(Svf, "cutoff", Svf), Some("cutoff"));
        assert_eq!(counterpart_key(Svf, "nope", Svf), None);
    }

    #[test]
    fn dist_eq_noise_and_reverb_stand_alone() {
        for kind in [Dist, Eq, Noise, Reverb] {
            assert_eq!(substitutes(kind).count(), 0, "{kind}");
        }
        assert_eq!(
            substitutes(Svf).collect::<Vec<_>>(),
            [Ladder, Formant, Comb],
            "the closest first"
        );
        assert_eq!(substitutes(Mseg).collect::<Vec<_>>(), [Lfo, Adsr]);
    }

    #[test]
    fn the_modes_of_the_filters_line_up_by_name() {
        assert!(same_item("lp", "lp12"));
        assert!(same_item("bp", "bp") && same_item("hp", "hp"));
        assert!(!same_item("lp", "lp24") && !same_item("notch", "hp"));
    }
}
