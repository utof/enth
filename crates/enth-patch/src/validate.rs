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
//! 4. **Routes:** the source is a modulator that exists or a built-in (`unknown_module`, `not_a_source`); the
//!    destination exists and is `modulatable` (`unknown_module`, `unknown_param`, `not_modulatable`); `|depth| <=
//!    100`; no (src, dst) twice (`duplicate_route`); at most 64 (`too_many`); a per-voice source never drives a
//!    global destination (`route_rate_mismatch`).
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

use crate::addr::{ModSource, RouteAddr, SlotIndex};
use crate::catalogue::{MACRO_VALUE, MASTER_PARAMS, ModuleKind, Rate, VOICE_PARAMS};
use crate::error::{Invalid, PatchError, Violation};
use crate::model::{MAX_ROUTES, Patch, Section};
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

/// Every violation of the invariants, in document order (empty for a valid patch).
pub fn violations(p: &Patch) -> Vec<Violation> {
    let mut out = Vec::new();
    let mut seen: BTreeSet<&str> = BTreeSet::new();

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

    for (spec, &value) in VOICE_PARAMS.iter().zip(&p.voice.values) {
        let label = format!("voice.{}", spec.key);
        check_value(&label, &label, spec, value, &mut out);
    }
    for (spec, &value) in MASTER_PARAMS.iter().zip(&p.master) {
        let label = format!("master.{}", spec.key);
        check_value(&label, &label, spec, value, &mut out);
    }

    check_amp_env(p, &mut out);

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
    for (i, r) in p.routes.iter().enumerate() {
        let at = format!("routes[{i}]");
        let mut ok = true;
        if let ModSource::Module(id) = &r.src {
            match locate(p, id) {
                Err(e) => {
                    ok = false;
                    out.push(Violation::new(format!("{at}.src"), e));
                }
                Ok((pos, m)) if pos.section != Section::Modulators => {
                    ok = false;
                    out.push(Violation::new(
                        format!("{at}.src"),
                        PatchError::NotASource {
                            id: id.to_string(),
                            kind: m.kind,
                        },
                    ));
                }
                Ok(_) => {}
            }
        }
        match resolve(p, &r.dst) {
            Err(e) => {
                ok = false;
                out.push(Violation::new(format!("{at}.dst"), e));
            }
            Ok((_, spec)) if !spec.flags.modulatable => {
                ok = false;
                out.push(Violation::new(
                    format!("{at}.dst"),
                    PatchError::NotModulatable {
                        addr: r.dst.to_string(),
                    },
                ));
            }
            Ok(_) => {}
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
