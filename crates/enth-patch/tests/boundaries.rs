//! Edges that a mutation run found unguarded (S1b): the exact size of the route table, an amplitude envelope in the
//! wrong section, and the route text inside route errors (the CLI prints it back to the user).

mod common;

use common::*;
use enth_patch::PatchError;
use enth_patch::addr::{MacroIndex, ModSource, ModuleId, Position};
use enth_patch::catalogue::{ModuleKind, Rate};
use enth_patch::model::{MAX_ROUTES, ModuleInstance, Route};
use enth_patch::ops::{Op, apply};
use enth_patch::validate::{validate, violations};

fn insert(index: usize, src: ModSource, dst: &str, depth: f32) -> Op {
    Op::InsertRoute {
        index,
        route: Route {
            src,
            dst: dst.parse().unwrap(),
            depth,
        },
    }
}

#[test]
fn the_route_table_holds_exactly_the_limit_and_not_one_more() {
    let mut p = base_patch();
    // Distinct (macro, destination) pairs: 8 macros x every modulatable parameter.
    let dsts = mod_targets(&p);
    let mut pairs = Vec::new();
    for dst in &dsts {
        for m in 0..8 {
            pairs.push((ModSource::Macro(MacroIndex::new(m).unwrap()), dst.clone()));
        }
    }
    assert!(pairs.len() > MAX_ROUTES, "{} pairs", pairs.len());

    for (i, (src, dst)) in pairs.iter().take(MAX_ROUTES).enumerate() {
        let route = Route {
            src: src.clone(),
            dst: dst.clone(),
            depth: 1.0,
        };
        apply(&mut p, Op::InsertRoute { index: i, route }).unwrap();
    }
    assert_eq!(p.routes.len(), MAX_ROUTES);
    validate(&p).expect("a full route table is valid");

    // One more through an op is refused, and the table is unchanged.
    let (src, dst) = pairs[MAX_ROUTES].clone();
    let extra = Route {
        src,
        dst,
        depth: 1.0,
    };
    let err = apply(
        &mut p,
        Op::InsertRoute {
            index: 0,
            route: extra.clone(),
        },
    )
    .unwrap_err();
    assert_eq!(
        err,
        PatchError::TooMany {
            what: "routes",
            max: MAX_ROUTES
        }
    );
    assert_eq!(p.routes.len(), MAX_ROUTES);

    // One more smuggled in by hand is reported by validate, once, at "routes".
    p.routes.push(extra);
    let found = violations(&p);
    let at_routes: Vec<_> = found.iter().filter(|v| v.path == "routes").collect();
    assert_eq!(at_routes.len(), 1, "{found:?}");
    assert_eq!(at_routes[0].error.code(), "too_many");
}

#[test]
fn an_amplitude_envelope_outside_the_modulators_is_misplaced() {
    let mut p = base_patch();
    validate(&p).unwrap();
    // The envelope module is an enabled adsr, but sits in the effects section.
    let env = p.modulators.remove(0);
    assert_eq!(env.kind, ModuleKind::Adsr);
    assert!(env.enabled);
    p.fx.push(env);
    let found = violations(&p);
    let amp: Vec<_> = found.iter().filter(|v| v.path == "voice.amp_env").collect();
    assert_eq!(amp.len(), 1, "{found:?}");
    assert_eq!(amp[0].error.code(), "wrong_section");
}

#[test]
fn route_errors_carry_the_route_as_text() {
    let mut p = base_patch();
    let at = Position {
        section: ModuleKind::Reverb.section(),
        index: 0,
    };
    let reverb = ModuleInstance::new(ModuleId::new("reverb1").unwrap(), ModuleKind::Reverb);
    apply(&mut p, Op::AddModule { at, module: reverb }).unwrap();

    let err = apply(&mut p, insert(0, ModSource::Velocity, "reverb1.wet", 10.0)).unwrap_err();
    assert_eq!(
        err,
        PatchError::RouteRateMismatch {
            route: "velocity -> reverb1.wet".to_owned(),
            src: Rate::Voice,
            dst: Rate::Global,
        }
    );
    assert!(err.to_string().contains("velocity -> reverb1.wet"), "{err}");

    apply(&mut p, insert(0, ModSource::Key, "voice.pan", 10.0)).unwrap();
    let err = apply(&mut p, insert(1, ModSource::Key, "voice.pan", 20.0)).unwrap_err();
    assert_eq!(
        err,
        PatchError::DuplicateRoute {
            route: "key -> voice.pan".to_owned(),
        }
    );
    assert!(err.to_string().contains("key -> voice.pan"), "{err}");
}
