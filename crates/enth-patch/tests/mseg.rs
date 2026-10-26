//! Editing an `mseg` through ops (S2a lane MSEG): every one of its 59 parameters, the 48 point parameters included,
//! applies and inverts bit for bit, alone and in transactions; a point edit is a smoothed value event and only `scope`
//! recompiles the plan; and an MSEG as a modulation source has the span its polarity says.

mod common;

use common::*;
use enth_patch::addr::{ModuleId, ParamAddr, Position, RouteAddr};
use enth_patch::catalogue::{
    MSEG_POINT_BASE, MSEG_POINTS, ModuleKind, MsegParam, Polarity, SvfParam,
};
use enth_patch::model::{ModuleInstance, Patch, Route};
use enth_patch::modulation::modulation_range;
use enth_patch::ops::{Effect, Op, apply, apply_all, effect};
use enth_patch::validate::validate;
use proptest::prelude::*;

fn id(s: &str) -> ModuleId {
    ModuleId::new(s).unwrap()
}

fn with_mseg() -> Patch {
    let mut p = base_patch();
    for (name, kind) in [("filter1", ModuleKind::Svf), ("mseg1", ModuleKind::Mseg)] {
        let section = kind.section();
        let at = Position {
            section,
            index: p.section(section).len(),
        };
        let module = ModuleInstance::new(id(name), kind);
        apply(&mut p, Op::AddModule { at, module }).unwrap();
    }
    p
}

fn addr(key: &str) -> ParamAddr {
    ParamAddr::module(id("mseg1"), key).unwrap()
}

/// The op for parameter `index` of the mseg at the normalised position `n` of its range.
fn set(index: usize, n: f32) -> Op {
    let spec = &ModuleKind::Mseg.params()[index];
    Op::SetParam {
        addr: addr(spec.key),
        value: spec.from_norm(n),
    }
}

#[test]
fn the_point_parameters_are_where_the_catalogue_says() {
    assert_eq!(MsegParam::COUNT, MSEG_POINT_BASE + 3 * MSEG_POINTS);
    for i in 0..MSEG_POINTS {
        for (j, name) in ["time", "level", "curve"].into_iter().enumerate() {
            let spec = &ModuleKind::Mseg.params()[MSEG_POINT_BASE + 3 * i + j];
            assert_eq!(spec.key, format!("{name}{}", i + 1));
        }
    }
}

proptest! {
    /// Every parameter, at any position of its range: the op applies, the patch stays valid and the inverse restores
    /// the original bit for bit.
    #[test]
    fn any_parameter_edit_applies_and_inverts(index in 0..MsegParam::COUNT, n in 0.0f32..=1.0) {
        let mut p = with_mseg();
        let before = p.clone();
        let op = set(index, n);
        let inv = apply(&mut p, &op).map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert!(validate(&p).is_ok());
        for o in &inv {
            apply(&mut p, o).map_err(|e| TestCaseError::fail(e.to_string()))?;
        }
        prop_assert!(identical(&p, &before), "{op:?}");
    }

    /// A run of edits as one transaction (a drag across several points): atomic, and its inverse restores the patch.
    #[test]
    fn a_run_of_point_edits_is_one_transaction_that_inverts(
        edits in proptest::collection::vec((MSEG_POINT_BASE..MsegParam::COUNT, 0.0f32..=1.0), 1..24)
    ) {
        let mut p = with_mseg();
        let before = p.clone();
        let ops: Vec<Op> = edits.iter().map(|&(i, n)| set(i, n)).collect();
        let inv = apply_all(&mut p, &ops).map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert!(validate(&p).is_ok());
        let after = p.clone();
        apply_all(&mut p, &inv).map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert!(identical(&p, &before));
        // And redoing the ops reproduces the result.
        apply_all(&mut p, &ops).map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert!(identical(&p, &after));
    }
}

#[test]
fn only_scope_recompiles_the_plan() {
    let p = with_mseg();
    for (i, spec) in ModuleKind::Mseg.params().iter().enumerate() {
        let op = set(i, 0.5);
        let got = effect(&p, &op);
        if spec.key == "scope" {
            assert_eq!(got, Effect::Structural, "{}", spec.key);
        } else {
            assert_eq!(got, Effect::Value(addr(spec.key)), "{}", spec.key);
        }
    }
}

#[test]
fn an_mseg_source_spans_its_polarity() {
    let mut p = with_mseg();
    let cutoff: ParamAddr = "filter1.cutoff".parse().unwrap();
    let route: RouteAddr = "mseg1 -> filter1.cutoff".parse().unwrap();
    apply(
        &mut p,
        Op::InsertRoute {
            index: 0,
            route: Route {
                src: route.src,
                dst: route.dst,
                depth: 40.0,
            },
        },
    )
    .unwrap();
    let base = p.value(&cutoff).unwrap();
    let norm = ModuleKind::Svf.params()[SvfParam::Cutoff.index()].to_norm(base);
    // Unipolar (the default): 0 to +40 % of the range above the base.
    let r = modulation_range(&p, &cutoff).unwrap();
    assert_eq!(r.base, norm);
    assert!((r.lo - norm).abs() < 1e-6);
    assert!((r.hi - (norm + 0.4).min(1.0)).abs() < 1e-6);
    // Bipolar: 40 % either way.
    apply(
        &mut p,
        Op::SetParam {
            addr: addr("polarity"),
            value: Polarity::Bipolar.index() as f32,
        },
    )
    .unwrap();
    let r = modulation_range(&p, &cutoff).unwrap();
    assert!((r.lo - (norm - 0.4).max(0.0)).abs() < 1e-6);
    assert!((r.hi - (norm + 0.4).min(1.0)).abs() < 1e-6);
}
