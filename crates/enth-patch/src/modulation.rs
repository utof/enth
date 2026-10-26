//! Modulation ranges: how far a parameter's routes can move it (P1b lane ED2; docs/session.md, F1b review decision
//! 5). A pure function of the patch, shared by the editor's static depth arcs and the CLI's `describe`.
//!
//! The engine computes a routed parameter in its normalised domain (design.md §2, `enth-engine`'s `modulation`):
//!
//! ```text
//! n = clamp(to_norm(base) + Σ_routes (depth / 100) · source, 0, 1)
//! ```
//!
//! where each source runs over its own span, set by its polarity: envelopes, velocity, macros, the mod wheel and a
//! unipolar LFO 0..1, a bipolar LFO −1..1, and `key` `(note − 60) / 120` over MIDI notes 0..=127. Since the clamp is
//! monotonic, the span of `n` is the clamped sum of each route's lowest and highest contribution. Routes from or to a
//! disabled module have no term in the engine's plan, so they contribute nothing here either.

use crate::addr::{ModSource, ParamAddr, Target};
use crate::catalogue::{LfoParam, ModuleKind, MsegParam, Polarity};
use crate::model::Patch;

/// The depth a route made without one starts at, in percent of the destination's range: the editor's drag-to-modulate
/// drop (P1b lane ED2). Audible at once; the knob's depth handle then sets it.
pub const DEFAULT_ROUTE_DEPTH: f32 = 25.0;

/// The span of `key` as a source: `(note − 60) / 120` over MIDI notes 0..=127.
pub const KEY_SPAN: (f32, f32) = (-60.0 / 120.0, 67.0 / 120.0);

/// How far a parameter can be moved by its routes, normalised by its spec (0..1).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ModulationRange {
    /// The base value, `to_norm` of the patch's plain value.
    pub base: f32,
    /// The lowest value the routes can reach, clamped to 0..1.
    pub lo: f32,
    /// The highest value the routes can reach, clamped to 0..1.
    pub hi: f32,
}

impl ModulationRange {
    /// Whether the routes can move the parameter at all.
    pub fn is_modulated(&self) -> bool {
        self.lo != self.hi
    }
}

/// The span a source's output runs over, by its polarity (module docs). `None` for a module that does not exist or
/// is not a modulator.
pub fn source_span(patch: &Patch, src: &ModSource) -> Option<(f32, f32)> {
    match src {
        ModSource::Velocity | ModSource::ModWheel | ModSource::Macro(_) => Some((0.0, 1.0)),
        ModSource::Key => Some(KEY_SPAN),
        ModSource::Module(id) => {
            let (_, m) = patch.find(id)?;
            match m.kind {
                ModuleKind::Adsr => Some((0.0, 1.0)),
                ModuleKind::Lfo => match m
                    .values
                    .get(LfoParam::Polarity.index())
                    .copied()
                    .and_then(Polarity::from_value)
                {
                    Some(Polarity::Unipolar) => Some((0.0, 1.0)),
                    _ => Some((-1.0, 1.0)),
                },
                // Lane MSEG: 0..1 as drawn, or -1..1 bipolar; velocity only scales it down.
                ModuleKind::Mseg => match m
                    .values
                    .get(MsegParam::Polarity.index())
                    .copied()
                    .and_then(Polarity::from_value)
                {
                    Some(Polarity::Unipolar) => Some((0.0, 1.0)),
                    _ => Some((-1.0, 1.0)),
                },
                _ => None,
            }
        }
    }
}

/// Whether a module a route names is enabled (built-ins and non-module sources always are).
fn enabled(patch: &Patch, target: Option<&crate::addr::ModuleId>) -> bool {
    target.is_none_or(|id| patch.find(id).is_some_and(|(_, m)| m.enabled))
}

/// The normalised span `addr` can reach through its routes: `to_norm(base) + Σ depth · source span`, each source by
/// its polarity, clamped to 0..1 (module docs). `None` if the address does not resolve. With no route (or only routes
/// from or to disabled modules) `lo == hi == base`.
pub fn modulation_range(patch: &Patch, addr: &ParamAddr) -> Option<ModulationRange> {
    let spec = patch.spec(addr)?;
    let base = spec.to_norm(patch.value(addr)?);
    let dst_module = match addr.target() {
        Target::Module(id) => Some(id),
        Target::Voice | Target::Master => None,
    };
    let (mut lo, mut hi) = (base, base);
    if enabled(patch, dst_module) {
        for r in patch.routes.iter().filter(|r| &r.dst == addr) {
            let src_module = match &r.src {
                ModSource::Module(id) => Some(id),
                _ => None,
            };
            if !enabled(patch, src_module) {
                continue;
            }
            let Some((a, b)) = source_span(patch, &r.src) else {
                continue;
            };
            let d = r.depth / 100.0;
            let (x, y) = (d * a, d * b);
            lo += x.min(y);
            hi += x.max(y);
        }
    }
    Some(ModulationRange {
        base,
        lo: lo.clamp(0.0, 1.0),
        hi: hi.clamp(0.0, 1.0),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalogue::{LfoScope, SvfParam};
    use crate::model::{ModuleInstance, Route};
    use crate::{ModuleId, ModuleKind};

    fn patch() -> Patch {
        let mut p = Patch::init();
        let mut filter = ModuleInstance::new(ModuleId::new("filter1").unwrap(), ModuleKind::Svf);
        // A cutoff whose normalised value is easy to reason about is not needed: compare with to_norm.
        filter.values[SvfParam::Cutoff.index()] = 1000.0;
        p.voice_fx.push(filter);
        p.modulators.push(ModuleInstance::new(
            ModuleId::new("lfo1").unwrap(),
            ModuleKind::Lfo,
        ));
        p
    }

    fn route(p: &mut Patch, src: &str, dst: &str, depth: f32) {
        p.routes.push(Route {
            src: src.parse().unwrap(),
            dst: dst.parse().unwrap(),
            depth,
        });
    }

    fn cutoff() -> ParamAddr {
        "filter1.cutoff".parse().unwrap()
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-6
    }

    #[test]
    fn no_route_is_no_range_and_an_unknown_address_is_none() {
        let p = patch();
        let r = modulation_range(&p, &cutoff()).unwrap();
        assert_eq!((r.lo, r.hi), (r.base, r.base));
        assert!(!r.is_modulated());
        let base = p.spec(&cutoff()).unwrap().to_norm(1000.0);
        assert_eq!(r.base, base);
        assert!(modulation_range(&p, &"filter9.cutoff".parse().unwrap()).is_none());
    }

    #[test]
    fn each_source_moves_by_its_polarity_and_the_routes_sum() {
        let mut p = patch();
        let base = modulation_range(&p, &cutoff()).unwrap().base;
        // An envelope is unipolar: up only, by the depth.
        route(&mut p, "env1", "filter1.cutoff", 20.0);
        let r = modulation_range(&p, &cutoff()).unwrap();
        assert!(close(r.lo, base) && close(r.hi, base + 0.2), "{r:?}");
        // A bipolar LFO swings both ways; a negative depth is the same span.
        route(&mut p, "lfo1", "filter1.cutoff", -10.0);
        let r = modulation_range(&p, &cutoff()).unwrap();
        assert!(close(r.lo, base - 0.1) && close(r.hi, base + 0.3), "{r:?}");
        // A unipolar LFO with a negative depth only lowers it.
        p.modulators[1].values[LfoParam::Polarity.index()] = Polarity::Unipolar.index() as f32;
        let r = modulation_range(&p, &cutoff()).unwrap();
        assert!(close(r.lo, base - 0.1) && close(r.hi, base + 0.2), "{r:?}");
        // Key runs (note − 60) / 120 over notes 0..=127.
        route(&mut p, "key", "filter1.cutoff", 20.0);
        let r = modulation_range(&p, &cutoff()).unwrap();
        assert!(
            close(r.lo, base - 0.1 - 0.1) && close(r.hi, base + 0.2 + 0.2 * 67.0 / 120.0),
            "{r:?}"
        );
    }

    #[test]
    fn the_span_is_clamped_to_the_range() {
        let mut p = patch();
        route(&mut p, "velocity", "filter1.cutoff", 100.0);
        route(&mut p, "macro1", "filter1.cutoff", -100.0);
        let r = modulation_range(&p, &cutoff()).unwrap();
        assert_eq!((r.lo, r.hi), (0.0, 1.0));
    }

    #[test]
    fn routes_from_or_to_a_disabled_module_count_for_nothing() {
        let mut p = patch();
        route(&mut p, "lfo1", "filter1.cutoff", 30.0);
        p.modulators[1].enabled = false;
        let r = modulation_range(&p, &cutoff()).unwrap();
        assert!(!r.is_modulated(), "a disabled source: {r:?}");
        p.modulators[1].enabled = true;
        p.voice_fx[0].enabled = false;
        assert!(!modulation_range(&p, &cutoff()).unwrap().is_modulated());
        // A global LFO on a built-in.
        p.modulators[1].values[LfoParam::Scope.index()] = LfoScope::Global.index() as f32;
        route(&mut p, "lfo1", "master.level", 10.0);
        let r = modulation_range(&p, &"master.level".parse().unwrap()).unwrap();
        assert!(r.is_modulated());
    }

    #[test]
    fn source_spans_follow_polarity() {
        let p = patch();
        assert_eq!(source_span(&p, &"env1".parse().unwrap()), Some((0.0, 1.0)));
        assert_eq!(source_span(&p, &"lfo1".parse().unwrap()), Some((-1.0, 1.0)));
        assert_eq!(source_span(&p, &"key".parse().unwrap()), Some(KEY_SPAN));
        assert_eq!(
            source_span(&p, &"macro8".parse().unwrap()),
            Some((0.0, 1.0))
        );
        assert_eq!(source_span(&p, &"osc1".parse().unwrap()), None);
        assert_eq!(source_span(&p, &"nope".parse().unwrap()), None);
    }
}
