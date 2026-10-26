//! Patch validation with typed errors that carry a path and a hint (P1a lane S1b).
//!
//! [`validate`] checks every invariant listed in [`model`](crate::model) and returns **all** violations, in document
//! order, each with a path (`generators[1].params.cutoff`, `routes[0]`, `voice.amp_env`) and a [`PatchError`] (code,
//! hint, `did_you_mean`). It runs at every boundary: after reading a file, before compiling a plan, in tests after
//! every op. The invariants, with their codes:
//!
//! 1. **Ids** are unique across sections (`duplicate_id`); their grammar and reserved words are the type's.
//! 2. **Placement:** each module sits in its kind's section (`wrong_section`), and a section holds at most its
//!    limit (`too_many`).
//! 3. **Values:** a module has one value per parameter of its kind (`wrong_value_count`); every value is finite,
//!    in range and integral for discrete kinds (`non_finite`, `out_of_range`, `not_integral`), and **stored
//!    canonically** (`not_canonical`): never `-0.0`, and never the top of a periodic range, because **a phase of 360
//!    is stored as 0** (decided in S1b: one spelling per sound keeps the file round trip and both hashes exact).
//!    The same holds for `voice`, `master`, macros and route depths.
//! 4. **Routes:** the source is a modulator that exists, a built-in or a generator (`unknown_module`,
//!    `not_a_source`); the destination exists and is `modulatable` (`unknown_module`, `unknown_param`,
//!    `not_modulatable`); `|depth| <= 100`; no (src, dst) twice (`duplicate_route`); at most 64 (`too_many`); a
//!    per-voice source never drives a global destination (`route_rate_mismatch`).
//!    **Audio routes** (S2a lane RTE, [`RouteKind::Audio`]): a generator is a source only for an `audio` parameter
//!    **of a generator** (the destination of a generator route that is not one is `not_audio_dest`, F1m S4: the same
//!    test `plan::compile` makes), an `audio` parameter takes only a generator (`not_audio_source`; it is never
//!    `modulatable`, so no control route reaches it), the audio routes together form no loop, a generator driving
//!    itself included (`audio_cycle`, reported on the route that closes it), and none makes a generator after the
//!    hard-sync master feed it (`audio_into_sync_master`, F1m S4: the rule `plan::compile` applies to the enabled
//!    generators' routes of nonzero depth, through the same [`sync_master_blocked`]; unreachable while no `osc`
//!    parameter is an audio input). The depth of an audio route is the same −100..100 %.
//! 5. **The amplitude envelope** `voice.amp_env` names an `adsr` in `modulators` (`unknown_module`, `wrong_kind`)
//!    that is **enabled** (`in_use_as_amp_env`): a disabled amp envelope would leave voices that never end.
//! 6. **Slots** point at existing `automatable` parameters (`unknown_module`, `unknown_param`, `not_automatable`);
//!    macro values are in 0–1.
//!
//! `validate` is stricter than [`ParamSpec::is_valid`](enth_params::ParamSpec::is_valid) in one way: that accepts a
//! phase of 360, this reports it `not_canonical` (ops, `load` and `wire` store it as 0, so a patch made through
//! them never trips it).

use std::collections::BTreeSet;

use enth_params::{DEPTH, ParamSpec};

use crate::addr::{ModSource, ModuleId, RouteAddr, SlotIndex, Target};
use crate::catalogue::{MACRO_VALUE, MASTER_PARAMS, ModuleKind, Rate, VOICE_PARAMS};
use crate::error::{Invalid, PatchError, Violation};
use crate::model::{MAX_ROUTES, Patch, Route, Section};
use crate::ops::{canonical_value, locate, resolve};

/// Check a patch against every invariant. `Ok` means it can be compiled, hashed and written.
pub fn validate(patch: &Patch) -> Result<(), Invalid> {
    let v = violations(patch);
    if v.is_empty() {
        Ok(())
    } else {
        Err(Invalid(v))
    }
}

fn check_value(path: &str, label: &str, spec: &ParamSpec, value: f32, out: &mut Vec<Violation>) {
    match canonical_value(spec, value) {
        Err(e) => out.push(Violation::new(path, e)),
        Ok(c) if c.to_bits() != value.to_bits() => out.push(Violation::new(
            path,
            PatchError::NotCanonical {
                param: label.to_owned(),
                value,
                canonical: c,
            },
        )),
        Ok(_) => {}
    }
}

/// Every violation of the invariants, in the order a patch file lists them (empty for a valid patch).
pub fn violations(p: &Patch) -> Vec<Violation> {
    violations_with(p, SYNC_MASTER_KIND)
}

/// [`violations`], with the kind of generator that is the hard-sync master as a parameter, so the unit tests can
/// exercise the `audio_into_sync_master` path with a kind that has an audio input (none of `osc`'s parameters has).
fn violations_with(p: &Patch, master_kind: ModuleKind) -> Vec<Violation> {
    let mut out = Vec::new();
    let mut seen: BTreeSet<&str> = BTreeSet::new();

    // The order of a file: voice (its values, then its amplitude envelope), master, the sections, routes, macros,
    // slots.
    for (spec, &value) in VOICE_PARAMS.iter().zip(&p.voice.values) {
        let label = format!("voice.{}", spec.key);
        check_value(&label, &label, spec, value, &mut out);
    }
    check_amp_env(p, &mut out);
    for (spec, &value) in MASTER_PARAMS.iter().zip(&p.master) {
        let label = format!("master.{}", spec.key);
        check_value(&label, &label, spec, value, &mut out);
    }

    for section in Section::ALL {
        let modules = p.section(section);
        if modules.len() > section.max_len() {
            out.push(Violation::new(
                section.key(),
                PatchError::TooMany {
                    what: section.key(),
                    max: section.max_len(),
                },
            ));
        }
        for (i, m) in modules.iter().enumerate() {
            let at = format!("{section}[{i}]");
            if !seen.insert(m.id.as_str()) {
                out.push(Violation::new(
                    format!("{at}.id"),
                    PatchError::DuplicateId {
                        id: m.id.to_string(),
                    },
                ));
            }
            if m.kind.section() != section {
                out.push(Violation::new(
                    &at,
                    PatchError::WrongSection {
                        id: m.id.to_string(),
                        kind: m.kind,
                        expected: m.kind.section(),
                        got: section,
                    },
                ));
            }
            let specs = m.kind.params();
            if m.values.len() != specs.len() {
                out.push(Violation::new(
                    &at,
                    PatchError::WrongValueCount {
                        id: m.id.to_string(),
                        expected: specs.len(),
                        found: m.values.len(),
                    },
                ));
                continue;
            }
            for (spec, &value) in specs.iter().zip(&m.values) {
                check_value(
                    &format!("{at}.params.{}", spec.key),
                    &format!("{}.{}", m.id, spec.key),
                    spec,
                    value,
                    &mut out,
                );
            }
        }
    }

    if p.routes.len() > MAX_ROUTES {
        out.push(Violation::new(
            "routes",
            PatchError::TooMany {
                what: "routes",
                max: MAX_ROUTES,
            },
        ));
    }
    let mut routes: BTreeSet<RouteAddr> = BTreeSet::new();
    // The audio routes accepted so far, as (source, destination) module ids: each new one is checked against these
    // for a loop, so a file with several loops reports each route that closes one.
    let mut audio: Vec<(&ModuleId, &ModuleId)> = Vec::new();
    let mut sync = SyncRule::new(p, master_kind);
    for (i, r) in p.routes.iter().enumerate() {
        let at = format!("routes[{i}]");
        let check = check_route_ends(p, r);
        let mut ok = check.src_error.is_none() && check.dst_error.is_none();
        if let Some(e) = check.src_error {
            out.push(Violation::new(format!("{at}.src"), e));
        }
        match check.dst_error {
            // The module's own value count is reported where the module is; not again for every route to it.
            Some(PatchError::WrongValueCount { .. }) | None => {}
            Some(e) => out.push(Violation::new(format!("{at}.dst"), e)),
        }
        if ok
            && check.kind == Some(RouteKind::Audio)
            && let (ModSource::Module(src), Some(dst)) = (&r.src, r.dst.module_id())
        {
            if reaches(&audio, dst, src) {
                ok = false;
                out.push(Violation::new(
                    &at,
                    PatchError::AudioCycle {
                        route: route_label(r),
                    },
                ));
            } else if sync.blocks(r) {
                ok = false;
                out.push(Violation::new(
                    &at,
                    PatchError::AudioIntoSyncMaster {
                        route: route_label(r),
                    },
                ));
            } else {
                audio.push((src, dst));
                sync.accept(r);
            }
        }
        check_value(
            &format!("{at}.depth"),
            "route depth",
            &DEPTH,
            r.depth,
            &mut out,
        );
        let key = RouteAddr {
            src: r.src.clone(),
            dst: r.dst.clone(),
        };
        if !routes.insert(key.clone()) {
            out.push(Violation::new(
                &at,
                PatchError::DuplicateRoute {
                    route: key.to_string(),
                },
            ));
        }
        if ok
            && let (Some(Rate::Voice), Some(Rate::Global)) =
                (p.source_rate(&r.src), p.target_rate(r.dst.target()))
        {
            out.push(Violation::new(
                &at,
                PatchError::RouteRateMismatch {
                    route: key.to_string(),
                    src: Rate::Voice,
                    dst: Rate::Global,
                },
            ));
        }
    }

    for (i, m) in p.macros.iter().enumerate() {
        let label = format!("macro{}", i + 1);
        check_value(
            &format!("macros[{i}].value"),
            &label,
            &MACRO_VALUE,
            m.value,
            &mut out,
        );
    }

    for (n, slot) in p.slots.iter().enumerate() {
        let Some(addr) = slot else { continue };
        let at = format!(
            "slots.{}",
            SlotIndex::new(n).map_or(n + 1, SlotIndex::one_based)
        );
        match resolve(p, addr) {
            Err(PatchError::WrongValueCount { .. }) => {}
            Err(e) => out.push(Violation::new(at, e)),
            Ok((_, spec)) if !spec.flags.automatable => out.push(Violation::new(
                at,
                PatchError::NotAutomatable {
                    addr: addr.to_string(),
                },
            )),
            Ok(_) => {}
        }
    }
    out
}

/// What a route carries (S2a, lane RTE).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RouteKind {
    /// Control rate: a modulator, velocity, key, the mod wheel or a macro moves a `modulatable` parameter, through
    /// the modulation matrix (once per segment).
    Control,
    /// Audio rate: a generator's pre-level output feeds an `audio` parameter of a generator (FM, PM, AM, ring
    /// modulation), per frame, scaled by the route's depth. Never in the modulation matrix.
    Audio,
}

/// Whether `src` is a generator of the patch (an audio source).
pub fn is_generator_source(p: &Patch, src: &ModSource) -> bool {
    matches!(src, ModSource::Module(id)
        if p.find(id).is_some_and(|(pos, _)| pos.section == Section::Generators))
}

/// Whether `dst` names a parameter flagged `audio` (an audio input).
pub fn is_audio_dest(p: &Patch, dst: &crate::addr::ParamAddr) -> bool {
    p.spec(dst).is_some_and(|spec| spec.flags.audio)
}

/// Whether `dst` names a parameter of a generator (F1m S4). An audio route's destination is an `audio` parameter *of
/// a generator*: `plan::compile` classifies routes with this same function, so the file layer and the compiler agree
/// even for a kind in another section that one day has an audio input.
pub fn is_generator_dest(p: &Patch, dst: &crate::addr::ParamAddr) -> bool {
    matches!(dst.target(), Target::Module(id)
        if p.find(id).is_some_and(|(pos, _)| pos.section == Section::Generators))
}

/// Whether `r` is an audio route as `plan::compile` sees it: a generator into an `audio` parameter of a generator.
pub fn is_audio_route(p: &Patch, r: &Route) -> bool {
    is_generator_source(p, &r.src) && is_audio_dest(p, &r.dst) && is_generator_dest(p, &r.dst)
}

/// The verdict of [`check_route_ends`].
#[derive(Debug)]
pub struct RouteEnds {
    /// `None` when an end is wrong (or does not resolve).
    pub kind: Option<RouteKind>,
    /// What is wrong with the source, reported at `routes[i].src`.
    pub src_error: Option<PatchError>,
    /// What is wrong with the destination, reported at `routes[i].dst`. It includes `wrong_value_count` of the
    /// destination module, which `validate` reports where the module is and not again for each route.
    pub dst_error: Option<PatchError>,
}

/// The two ends of a route against the routing contract: the source exists and is a modulator, a built-in or a
/// generator (`unknown_module`, `not_a_source`); the destination exists (`unknown_module`, `unknown_param`); and then
/// either a control route (a non-generator source into a `modulatable` parameter: `not_modulatable`) or an audio
/// route (a generator into an `audio` parameter), with the two mixed up refused (`not_audio_dest`: a generator into
/// a parameter that is not an audio input; `not_audio_source`: a control source into one that is). `validate` and
/// the `InsertRoute` op share it, so the file and the op agree on every code. It says nothing about depth, rate,
/// duplicates or loops.
pub fn check_route_ends(p: &Patch, r: &Route) -> RouteEnds {
    let mut src_error = None;
    let mut generator = false;
    if let ModSource::Module(id) = &r.src {
        match locate(p, id) {
            Err(e) => src_error = Some(e),
            Ok((pos, m)) => match pos.section {
                Section::Modulators => {}
                Section::Generators => generator = true,
                Section::VoiceFx | Section::Fx => {
                    src_error = Some(PatchError::NotASource {
                        id: id.to_string(),
                        kind: m.kind,
                    });
                }
            },
        }
    }
    let spec = match resolve(p, &r.dst) {
        Ok((_, spec)) => spec,
        Err(e) => {
            return RouteEnds {
                kind: None,
                src_error,
                dst_error: Some(e),
            };
        }
    };
    let (kind, src_error, dst_error) = route_verdict(
        r,
        src_error,
        generator,
        Dest {
            audio: spec.flags.audio,
            modulatable: spec.flags.modulatable,
            generator: is_generator_dest(p, &r.dst),
        },
    );
    RouteEnds {
        kind,
        src_error,
        dst_error,
    }
}

/// What the destination of a route is, for [`route_verdict`].
#[derive(Clone, Copy)]
struct Dest {
    /// The parameter is flagged `audio`.
    audio: bool,
    /// The parameter is flagged `modulatable`.
    modulatable: bool,
    /// The parameter belongs to a generator.
    generator: bool,
}

/// The decision of [`check_route_ends`], on facts alone: `src_error` is what is wrong with the source already,
/// `generator` says the source is a generator.
fn route_verdict(
    r: &Route,
    src_error: Option<PatchError>,
    generator: bool,
    dest: Dest,
) -> (Option<RouteKind>, Option<PatchError>, Option<PatchError>) {
    match (src_error, generator, dest.audio) {
        // The source is wrong already; say nothing more about the destination unless it is plainly not a target.
        (Some(e), _, audio) => (
            None,
            Some(e),
            (!audio && !dest.modulatable).then(|| not_modulatable(r)),
        ),
        (None, true, true) if dest.generator => (Some(RouteKind::Audio), None, None),
        // A generator into anything but an audio input of a generator (an audio input on another kind of module would
        // land here too: compile refuses it the same way).
        (None, true, _) => (
            None,
            None,
            Some(PatchError::NotAudioDest {
                src: r.src.to_string(),
                addr: r.dst.to_string(),
            }),
        ),
        (None, false, true) => (
            None,
            Some(PatchError::NotAudioSource {
                src: r.src.to_string(),
                addr: r.dst.to_string(),
            }),
            None,
        ),
        (None, false, false) if !dest.modulatable => (None, None, Some(not_modulatable(r))),
        (None, false, false) => (Some(RouteKind::Control), None, None),
    }
}

fn not_modulatable(r: &Route) -> PatchError {
    PatchError::NotModulatable {
        addr: r.dst.to_string(),
    }
}

/// `src -> dst`, as the errors write a route.
fn route_label(r: &Route) -> String {
    format!("{} -> {}", r.src, r.dst)
}

/// The audio routes of a patch as (source, destination) module ids: every route from a generator to an `audio`
/// parameter of a generator. Routes with an end that does not exist are not listed.
fn audio_edges(p: &Patch) -> Vec<(&ModuleId, &ModuleId)> {
    p.routes
        .iter()
        .filter(|r| is_audio_route(p, r))
        .filter_map(|r| match (&r.src, r.dst.target()) {
            (ModSource::Module(src), Target::Module(dst)) => Some((src, dst)),
            _ => None,
        })
        .collect()
}

/// Whether `to` can be reached from `from` along `edges` (a module reaches itself).
fn reaches(edges: &[(&ModuleId, &ModuleId)], from: &ModuleId, to: &ModuleId) -> bool {
    let mut seen: Vec<&ModuleId> = vec![from];
    let mut next = 0;
    while let Some(&at) = seen.get(next) {
        if at == to {
            return true;
        }
        next += 1;
        for &(src, dst) in edges {
            if src == at && !seen.contains(&dst) {
                seen.push(dst);
            }
        }
    }
    false
}

/// Whether adding the audio route `route` to `p` would close a loop between generators: the route's destination
/// already reaches its source through the audio routes `p` has, or the two are one module (a self-route). `false`
/// for a route that is not an audio route. The `InsertRoute` op calls it.
pub fn closes_audio_cycle(p: &Patch, route: &Route) -> bool {
    match (&route.src, route.dst.target()) {
        (ModSource::Module(src), Target::Module(dst)) if is_audio_route(p, route) => {
            reaches(&audio_edges(p), dst, src)
        }
        _ => false,
    }
}

/// The kind of generator that is the hard-sync master: the first enabled generator of this kind in patch order
/// (`plan::compile` picks it the same way). Every generator after it takes its sync.
pub const SYNC_MASTER_KIND: ModuleKind = ModuleKind::Osc;

/// The hard-sync rule on a graph of `n` generators (patch order, enabled ones) with `edges` of `(source, carrier)`
/// audio routes and the master at index `master`: whether some generator **after** the master reaches it, so that the
/// run order the routes force would run that generator first, and the master could not hand its sync to it. Kahn's
/// algorithm with the lowest patch index first runs the master before every later generator exactly when this is
/// `false` (the engine's tests check the two against each other for every acyclic graph on four generators).
/// `plan::compile` and `validate` both decide with it.
pub fn sync_master_blocked(n: usize, master: usize, edges: &[(usize, usize)]) -> bool {
    // Everything that can reach the master, walking the edges backwards.
    let mut feeds = vec![false; n];
    let mut stack = vec![master];
    while let Some(at) = stack.pop() {
        for &(src, dst) in edges {
            if dst == at && !core::mem::replace(&mut feeds[src], true) {
                stack.push(src);
            }
        }
    }
    (master + 1..n).any(|g| feeds[g])
}

/// Whether adding the audio route `route` to `p` would break the hard-sync rule ([`sync_master_blocked`]), the
/// enabled generators' routes of nonzero depth being the ones that count, as at compile. `false` for a route that is
/// not an audio route. The `InsertRoute` op calls it.
pub fn audio_into_sync_master(p: &Patch, route: &Route) -> bool {
    blocks_sync_master(p, route, SYNC_MASTER_KIND)
}

fn blocks_sync_master(p: &Patch, route: &Route, master_kind: ModuleKind) -> bool {
    if !is_audio_route(p, route) {
        return false;
    }
    let mut rule = SyncRule::new(p, master_kind);
    for r in p.routes.iter().filter(|r| is_audio_route(p, r)) {
        rule.accept(r);
    }
    rule.blocks(route)
}

/// The live audio routes accepted so far, as indices into the enabled generators, and the master among them.
struct SyncRule<'a> {
    generators: Vec<&'a ModuleId>,
    master: Option<usize>,
    edges: Vec<(usize, usize)>,
}

impl<'a> SyncRule<'a> {
    fn new(p: &'a Patch, master_kind: ModuleKind) -> Self {
        let enabled: Vec<_> = p.generators.iter().filter(|m| m.enabled).collect();
        Self {
            master: enabled.iter().position(|m| m.kind == master_kind),
            generators: enabled.iter().map(|m| &m.id).collect(),
            edges: Vec::new(),
        }
    }

    /// The route as an edge between enabled generators, if it counts at compile: nonzero depth, both ends enabled.
    fn edge(&self, r: &Route) -> Option<(usize, usize)> {
        if r.depth == 0.0 {
            return None;
        }
        let (ModSource::Module(src), Target::Module(dst)) = (&r.src, r.dst.target()) else {
            return None;
        };
        let at = |id: &ModuleId| self.generators.iter().position(|g| *g == id);
        Some((at(src)?, at(dst)?))
    }

    fn accept(&mut self, r: &Route) {
        if let Some(e) = self.edge(r) {
            self.edges.push(e);
        }
    }

    /// Whether the edges accepted so far plus `r` break the rule.
    fn blocks(&self, r: &Route) -> bool {
        let (Some(master), Some(e)) = (self.master, self.edge(r)) else {
            return false;
        };
        let mut edges = self.edges.clone();
        edges.push(e);
        sync_master_blocked(self.generators.len(), master, &edges)
    }
}

fn check_amp_env(p: &Patch, out: &mut Vec<Violation>) {
    let id = &p.voice.amp_env;
    match locate(p, id) {
        Err(e) => out.push(Violation::new("voice.amp_env", e)),
        Ok((_, m)) if m.kind != ModuleKind::Adsr => out.push(Violation::new(
            "voice.amp_env",
            PatchError::WrongKind {
                id: id.to_string(),
                expected: ModuleKind::Adsr,
                found: m.kind,
            },
        )),
        Ok((pos, _)) if pos.section != Section::Modulators => out.push(Violation::new(
            "voice.amp_env",
            PatchError::WrongSection {
                id: id.to_string(),
                kind: ModuleKind::Adsr,
                expected: Section::Modulators,
                got: pos.section,
            },
        )),
        Ok((_, m)) if !m.enabled => out.push(Violation::new(
            "voice.amp_env",
            PatchError::InUseAsAmpEnv {
                id: id.to_string(),
                action: "disabled",
            },
        )),
        Ok(_) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ModuleInstance;

    fn id(s: &str) -> ModuleId {
        ModuleId::new(s).unwrap()
    }

    fn route(src: &str, dst: &str, depth: f32) -> Route {
        Route {
            src: src.parse().unwrap(),
            dst: dst.parse().unwrap(),
            depth,
        }
    }

    /// Generators `names` in that patch order (`wt…` a `wt`, `osc…` an `osc`) with `routes`.
    fn generators(names: &[&str], routes: &[Route]) -> Patch {
        let mut p = Patch::init();
        p.generators.clear();
        for name in names {
            let kind = if name.starts_with("wt") {
                ModuleKind::Wt
            } else {
                ModuleKind::Osc
            };
            p.generators.push(ModuleInstance::new(id(name), kind));
        }
        p.routes = routes.to_vec();
        p
    }

    fn assert_clean(v: &[Violation]) {
        assert_eq!(codes(v), Vec::<(String, &'static str)>::new());
    }

    fn codes(v: &[Violation]) -> Vec<(String, &'static str)> {
        v.iter().map(|v| (v.path.clone(), v.error.code())).collect()
    }

    #[test]
    fn a_generator_into_an_audio_input_of_a_module_that_is_not_a_generator_is_not_an_audio_dest() {
        let r = route("osc1", "wt1.pm", 10.0);
        let wt = |generator| Dest {
            audio: true,
            modulatable: false,
            generator,
        };
        // The verdict, on facts: the same audio input on a generator is an audio route, on any other module it is
        // refused as `not_audio_dest` (the catalogue has no such parameter yet; `plan::compile` refuses it too).
        let (kind, src, dst) = route_verdict(&r, None, true, wt(true));
        assert_eq!((kind, src, dst), (Some(RouteKind::Audio), None, None));
        let (kind, src, dst) = route_verdict(&r, None, true, wt(false));
        assert_eq!(kind, None);
        assert_eq!(src, None);
        assert_eq!(dst.map(|e| e.code()), Some("not_audio_dest"));
        // And the two ends of the real catalogue come out as the compiler classifies them.
        let p = generators(&["osc1", "wt1"], &[]);
        assert!(is_generator_dest(&p, &"wt1.pm".parse().unwrap()));
        assert!(!is_generator_dest(&p, &"filter1.cutoff".parse().unwrap()));
        assert!(!is_generator_dest(&p, &"voice.glide".parse().unwrap()));
        assert!(is_audio_route(&p, &r));
        assert!(!is_audio_route(&p, &route("osc1", "wt1.level", 10.0)));
        assert!(!is_audio_route(&p, &route("lfo1", "wt1.pm", 10.0)));
    }

    #[test]
    fn the_sync_master_rule_is_reachability_from_a_later_generator() {
        // Master at 1 among 4 generators. 2 and 3 are after it; 0 is before it.
        assert!(!sync_master_blocked(4, 1, &[]));
        assert!(
            !sync_master_blocked(4, 1, &[(0, 1)]),
            "an earlier one may feed it"
        );
        assert!(
            !sync_master_blocked(4, 1, &[(1, 2), (1, 3), (2, 3)]),
            "it may feed the later ones"
        );
        assert!(sync_master_blocked(4, 1, &[(2, 1)]), "a later one feeds it");
        assert!(
            sync_master_blocked(4, 1, &[(3, 2), (2, 0), (0, 1)]),
            "through others"
        );
        assert!(
            !sync_master_blocked(4, 1, &[(1, 0), (0, 2), (2, 3)]),
            "downstream is not upstream"
        );
        assert!(!sync_master_blocked(1, 0, &[]));
    }

    #[test]
    fn validate_and_the_op_refuse_a_generator_after_the_master_that_feeds_it() {
        // Make `wt` the master kind (no `osc` parameter is an audio input, so the real kind cannot be tested with a
        // real route): wt1 is first in patch order, so it is the master; osc1 comes after it.
        let wt = ModuleKind::Wt;
        let p = generators(&["wt1", "osc1"], &[route("osc1", "wt1.pm", 50.0)]);
        assert_eq!(
            codes(&violations_with(&p, wt)),
            [("routes[0]".to_owned(), "audio_into_sync_master")]
        );
        // The real kind: the same patch is fine (osc1 is the master and nothing feeds it).
        assert_clean(&violations(&p));
        assert!(validate(&p).is_ok());
        // The op's check agrees, route by route, with the file's.
        let bare = generators(&["wt1", "osc1"], &[]);
        assert!(blocks_sync_master(
            &bare,
            &route("osc1", "wt1.pm", 50.0),
            wt
        ));
        assert!(!blocks_sync_master(
            &bare,
            &route("osc1", "wt1.pm", 50.0),
            SYNC_MASTER_KIND
        ));
        assert!(!audio_into_sync_master(
            &bare,
            &route("osc1", "wt1.pm", 50.0)
        ));
        // A depth of 0 leaves the route out at compile, so it breaks nothing; so does a disabled end.
        let zero = generators(&["wt1", "osc1"], &[route("osc1", "wt1.pm", 0.0)]);
        assert_clean(&violations_with(&zero, wt));
        let mut off = generators(&["wt1", "osc1"], &[route("osc1", "wt1.pm", 50.0)]);
        off.generators[1].enabled = false;
        assert_clean(&violations_with(&off, wt));
        // The master is the first *enabled* generator of its kind: with wt1 disabled, wt2 (after osc1) is it, and
        // osc1 feeding wt2 is the order the rule wants.
        let mut moved = generators(&["wt1", "osc1", "wt2"], &[route("osc1", "wt2.pm", 50.0)]);
        moved.generators[0].enabled = false;
        assert_clean(&violations_with(&moved, wt));
        // The patch order decides: osc1 first, the master after it.
        let ordered = generators(&["osc1", "wt1"], &[route("osc1", "wt1.pm", 50.0)]);
        assert_clean(&violations_with(&ordered, wt));
        // A chain through a generator before the master: wt1 (master), wt2, osc1; osc1 -> wt2 is fine,
        // wt2 -> wt1 then lets a later generator reach the master and is the route reported.
        let chain = generators(
            &["wt1", "wt2", "osc1"],
            &[route("osc1", "wt2.pm", 50.0), route("wt2", "wt1.am", 50.0)],
        );
        assert_eq!(
            codes(&violations_with(&chain, wt)),
            [("routes[1]".to_owned(), "audio_into_sync_master")]
        );
        // With the real master (osc1, last) the same patch is fine.
        assert_clean(&violations(&chain));
    }
}
