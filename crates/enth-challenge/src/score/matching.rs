//! **Owner: lane ST** (module matching). Pre-declared by lane F1R from the F1r review (S5), so lane SS can call the
//! matcher before lane ST fills it: SS maps the player's modules onto the target's before the scoring render
//! (ADR-0007's 2026-10-20 note, fix 2), and ST's structure score and the rings use the same pairing.
//!
//! The contract: [`match_modules`]`(target, answer)` pairs each module of the answer with at most one of the target,
//! of the same kind, and lists what is left over on each side. [`ModuleMatching::answer_addr`] turns a target
//! address into the player's (where a ring goes: `ParamScore::answer_addr`). Pure and deterministic: no render.
//!
//! **Placeholder until lane ST:** modules pair by id and kind, which is exact while players cannot add or remove
//! modules (S1's lane AM changes that). ST replaces the body (canonicalising, per-kind matching, equivalences) and
//! keeps the signatures; SS keeps calling them.

use enth_patch::addr::Target;
use enth_patch::{ModuleId, ParamAddr, Patch};

/// How the answer's modules pair with the target's.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ModuleMatching {
    /// (target id, answer id), in the target's module order.
    pairs: Vec<(ModuleId, ModuleId)>,
    /// Target modules with no partner, in the target's order.
    missing: Vec<ModuleId>,
    /// Answer modules with no partner, in the answer's order.
    extra: Vec<ModuleId>,
}

impl ModuleMatching {
    /// The answer's module paired with the target's `id`.
    pub fn answer_of(&self, id: &ModuleId) -> Option<&ModuleId> {
        self.pairs.iter().find(|(t, _)| t == id).map(|(_, a)| a)
    }

    /// The target's module paired with the answer's `id`.
    pub fn target_of(&self, id: &ModuleId) -> Option<&ModuleId> {
        self.pairs.iter().find(|(_, a)| a == id).map(|(t, _)| t)
    }

    /// Every pair, (target id, answer id).
    pub fn pairs(&self) -> &[(ModuleId, ModuleId)] {
        &self.pairs
    }

    /// Target modules the answer lacks.
    pub fn missing(&self) -> &[ModuleId] {
        &self.missing
    }

    /// Answer modules the target lacks.
    pub fn extra(&self) -> &[ModuleId] {
        &self.extra
    }

    /// The answer's address for a target address: the same parameter on the paired module (voice and master
    /// addresses map to themselves); `None` when the module has no partner.
    pub fn answer_addr(&self, target: &ParamAddr) -> Option<ParamAddr> {
        match target.target() {
            Target::Module(id) => {
                ParamAddr::module(self.answer_of(id)?.clone(), target.param()).ok()
            }
            _ => Some(target.clone()),
        }
    }
}

/// Pair the answer's modules with the target's (placeholder: by id and kind; see the [module docs](self)). Disabled
/// modules take part like enabled ones; lane ST decides how "off" and "absent" compare.
pub fn match_modules(target: &Patch, answer: &Patch) -> ModuleMatching {
    let mut m = ModuleMatching::default();
    for (_, t) in target.modules() {
        match answer.find(&t.id) {
            Some((_, a)) if a.kind == t.kind => m.pairs.push((t.id.clone(), a.id.clone())),
            _ => m.missing.push(t.id.clone()),
        }
    }
    for (_, a) in answer.modules() {
        if m.target_of(&a.id).is_none() {
            m.extra.push(a.id.clone());
        }
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_patch_matches_itself_and_addresses_carry_over() {
        let p = Patch::init();
        let m = match_modules(&p, &p);
        assert_eq!((m.missing().len(), m.extra().len()), (0, 0));
        assert_eq!(m.pairs().len(), p.modules().count());
        let addr: ParamAddr = "env1.attack".parse().unwrap();
        assert_eq!(m.answer_addr(&addr), Some(addr));
        let voice: ParamAddr = "voice.glide".parse().unwrap();
        assert_eq!(m.answer_addr(&voice), Some(voice));
        let empty = Patch {
            generators: Vec::new(),
            ..Patch::init()
        };
        let m = match_modules(&p, &empty);
        assert_ne!(m.missing().len(), 0);
        let osc: ParamAddr = "osc1.semi".parse().unwrap();
        assert_eq!(m.answer_addr(&osc), None);
    }
}
