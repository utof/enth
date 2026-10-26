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
//!    (the destination of a generator route that is not one is `not_audio_dest`), an `audio` parameter takes only a
//!    generator (`not_audio_source`; it is never `modulatable`, so no control route reaches it), and the audio
//!    routes together form no loop, a generator driving itself included (`audio_cycle`, reported on the route that
//!    closes it). The depth of an audio route is the same −100..100 %.
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
            } else {
                audio.push((src, dst));
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
    let (kind, src_error, dst_error) = match (src_error, generator, spec.flags.audio) {
        // The source is wrong already; say nothing more about the destination unless it is plainly not a target.
        (Some(e), _, audio) => (
            None,
            Some(e),
            (!audio && !spec.flags.modulatable).then(|| not_modulatable(r)),
        ),
        (None, true, true) => (Some(RouteKind::Audio), None, None),
        (None, true, false) => (
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
        (None, false, false) if !spec.flags.modulatable => (None, None, Some(not_modulatable(r))),
        (None, false, false) => (Some(RouteKind::Control), None, None),
    };
    RouteEnds {
        kind,
        src_error,
        dst_error,
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

/// The audio routes of a patch as (source, destination) module ids: every route from a generator to a parameter of a
/// module. Routes with an end that does not exist are not listed.
fn audio_edges(p: &Patch) -> Vec<(&ModuleId, &ModuleId)> {
    p.routes
        .iter()
        .filter(|r| is_generator_source(p, &r.src) && is_audio_dest(p, &r.dst))
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
        (ModSource::Module(src), Target::Module(dst))
            if is_generator_source(p, &route.src) && is_audio_dest(p, &route.dst) =>
        {
            reaches(&audio_edges(p), dst, src)
        }
        _ => false,
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
