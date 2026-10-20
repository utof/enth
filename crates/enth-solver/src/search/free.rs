//! The search space of a match: the free parameters, which axis each is searched on, and how a point of that space
//! becomes a patch.
//!
//! - A **continuous** axis is a `Float` parameter (or an `Int` with more than [`MAX_CATEGORIES`] values, rounded when
//!   it is applied). It is searched on its normalised axis `u ∈ [0, 1]` (`ParamSpec::to_norm` / `from_norm`), the
//!   domain the scale defines: an octave of cutoff is the same step everywhere and a time knob is as fine at 5 ms as
//!   at 5 s.
//! - A **categorical** axis is an `Enum`, a `Toggle` or a small `Int`: a waveform, a filter mode. CMA-ES cannot
//!   move on it, so the search fixes it per run and switches it between runs
//!   ([`neighbours`](FreeSet::neighbours)).
//!
//! A point becomes a patch only through ops (`Op::SetParam`, applied as one transaction), as AGENTS.md requires, so
//! a candidate is always a valid patch.

use enth_params::{Kind, ParamSpec};
use enth_patch::{Op, ParamAddr, Patch, apply_all};

use super::SearchError;

/// An `Int` parameter with more values than this is searched as a continuous axis (and rounded); an `Enum` or a
/// `Toggle` is always categorical.
pub const MAX_CATEGORIES: usize = 8;

/// How one free parameter is searched.
#[derive(Clone, Debug, PartialEq)]
pub enum AxisKind {
    /// On the normalised axis, 0–1.
    Continuous,
    /// One of these plain values (an enum's item indices, a toggle's 0 and 1, a small integer range).
    Categorical {
        /// The values, in ascending order.
        values: Vec<f32>,
    },
}

/// One free parameter.
#[derive(Clone, Debug, PartialEq)]
pub struct Axis {
    /// Where it lives.
    pub addr: ParamAddr,
    /// Its range, scale and kind (the bounds of the search come from here).
    pub spec: &'static ParamSpec,
    /// How it is searched.
    pub kind: AxisKind,
    /// Its plain value in the start patch.
    pub start: f32,
}

/// A point of the search space: the normalised position on every continuous axis and the chosen value index on
/// every categorical axis, each in [`FreeSet`] order.
#[derive(Clone, Debug, PartialEq)]
pub struct Point {
    /// Normalised positions, one per continuous axis.
    pub cont: Vec<f64>,
    /// Indices into [`AxisKind::Categorical::values`], one per categorical axis.
    pub cat: Vec<usize>,
}

/// The parameters a match may move, in the order they were given (duplicates dropped).
#[derive(Clone, Debug, PartialEq)]
pub struct FreeSet {
    axes: Vec<Axis>,
    cont: Vec<usize>,
    cat: Vec<usize>,
}

fn categorical_values(spec: &ParamSpec) -> Option<Vec<f32>> {
    match spec.kind {
        Kind::Float { .. } => None,
        Kind::Enum(items) => Some((0..items.len()).map(|i| i as f32).collect()),
        Kind::Toggle => Some(vec![0.0, 1.0]),
        Kind::Int { min, max } => {
            let n = (i64::from(max) - i64::from(min) + 1) as usize;
            (n <= MAX_CATEGORIES).then(|| (min..=max).map(|v| v as f32).collect())
        }
    }
}

impl FreeSet {
    /// The free set of `addrs` in `start`: each address must name a parameter of a module `start` has.
    ///
    /// # Errors
    /// [`SearchError::UnknownParam`] for an address `start` lacks, [`SearchError::NoFreeParameters`] for an empty
    /// list.
    pub fn new(start: &Patch, addrs: &[ParamAddr]) -> Result<FreeSet, SearchError> {
        let mut axes: Vec<Axis> = Vec::new();
        for addr in addrs {
            if axes.iter().any(|a| &a.addr == addr) {
                continue;
            }
            let (Some(spec), Some(value)) = (start.spec(addr), start.value(addr)) else {
                return Err(SearchError::UnknownParam {
                    addr: addr.to_string(),
                });
            };
            let kind = match categorical_values(spec) {
                Some(values) => AxisKind::Categorical { values },
                None => AxisKind::Continuous,
            };
            axes.push(Axis {
                addr: addr.clone(),
                spec,
                kind,
                start: value,
            });
        }
        if axes.is_empty() {
            return Err(SearchError::NoFreeParameters);
        }
        let cont = (0..axes.len())
            .filter(|&i| axes[i].kind == AxisKind::Continuous)
            .collect();
        let cat = (0..axes.len())
            .filter(|&i| axes[i].kind != AxisKind::Continuous)
            .collect();
        Ok(FreeSet { axes, cont, cat })
    }

    /// The observable parameters of `start` under a phrase's mask (`enth_challenge::random::observable`): gauges,
    /// nuisances, dead and disabled parameters stay out, as in the generator and the structure score.
    ///
    /// # Errors
    /// [`SearchError::NoFreeParameters`] when nothing is observable.
    pub fn from_mask(
        start: &Patch,
        mask: &enth_challenge::random::Mask,
    ) -> Result<FreeSet, SearchError> {
        let addrs: Vec<ParamAddr> = mask.observable_params().cloned().collect();
        FreeSet::new(start, &addrs)
    }

    /// Every axis, in order.
    #[must_use]
    pub fn axes(&self) -> &[Axis] {
        &self.axes
    }

    /// The addresses, in order.
    pub fn addrs(&self) -> impl Iterator<Item = &ParamAddr> {
        self.axes.iter().map(|a| &a.addr)
    }

    /// Whether `addr` is free.
    #[must_use]
    pub fn contains(&self, addr: &ParamAddr) -> bool {
        self.axes.iter().any(|a| &a.addr == addr)
    }

    /// How many axes.
    #[must_use]
    pub fn len(&self) -> usize {
        self.axes.len()
    }

    /// Always false: a free set has at least one axis.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.axes.is_empty()
    }

    /// The continuous axes (indices into [`axes`](Self::axes)), in order.
    #[must_use]
    pub fn continuous(&self) -> &[usize] {
        &self.cont
    }

    /// The categorical axes (indices into [`axes`](Self::axes)), in order.
    #[must_use]
    pub fn categorical(&self) -> &[usize] {
        &self.cat
    }

    /// The point of a patch: where its values sit on every axis (a categorical value not in the axis's list reads as
    /// the first one).
    #[must_use]
    pub fn point_of(&self, patch: &Patch) -> Point {
        let value = |a: &Axis| patch.value(&a.addr).unwrap_or(a.start);
        Point {
            cont: self
                .cont
                .iter()
                .map(|&i| f64::from(self.axes[i].spec.to_norm(value(&self.axes[i]))))
                .collect(),
            cat: self
                .cat
                .iter()
                .map(|&i| {
                    let a = &self.axes[i];
                    match &a.kind {
                        AxisKind::Categorical { values } => {
                            let v = value(a);
                            values.iter().position(|x| *x == v).unwrap_or(0)
                        }
                        AxisKind::Continuous => 0,
                    }
                })
                .collect(),
        }
    }

    /// The point of the start patch (what [`FreeSet::new`] read).
    #[must_use]
    pub fn start_point(&self) -> Point {
        let value = |i: usize| self.axes[i].start;
        Point {
            cont: self
                .cont
                .iter()
                .map(|&i| f64::from(self.axes[i].spec.to_norm(value(i))))
                .collect(),
            cat: self
                .cat
                .iter()
                .map(|&i| match &self.axes[i].kind {
                    AxisKind::Categorical { values } => {
                        values.iter().position(|x| *x == value(i)).unwrap_or(0)
                    }
                    AxisKind::Continuous => 0,
                })
                .collect(),
        }
    }

    /// The patch for a point: `base` with every free parameter set through one transaction of `SetParam` ops.
    /// `None` if the transaction fails (the point names a value the patch refuses; cannot happen for points made by
    /// this set from the patch it was built on).
    #[must_use]
    pub fn apply(&self, base: &Patch, point: &Point) -> Option<Patch> {
        let mut ops = Vec::with_capacity(self.axes.len());
        for (k, &i) in self.cont.iter().enumerate() {
            let a = &self.axes[i];
            let u = point.cont.get(k).copied().unwrap_or(0.5).clamp(0.0, 1.0) as f32;
            // A parameter still at its base position keeps its exact value: plain → normalised → plain is not an
            // exact round trip (AGENTS.md), and the start patch must stay itself.
            if base
                .value(&a.addr)
                .is_some_and(|cur| a.spec.to_norm(cur) == u)
            {
                continue;
            }
            ops.push(Op::SetParam {
                addr: a.addr.clone(),
                value: a.spec.from_norm(u),
            });
        }
        for (k, &i) in self.cat.iter().enumerate() {
            let a = &self.axes[i];
            if let AxisKind::Categorical { values } = &a.kind {
                let v = values.get(point.cat.get(k).copied().unwrap_or(0))?;
                ops.push(Op::SetParam {
                    addr: a.addr.clone(),
                    value: *v,
                });
            }
        }
        let mut patch = base.clone();
        apply_all(&mut patch, &ops).ok()?;
        Some(patch)
    }

    /// The assignments one flip away from `cat`: for each categorical axis in order, each other value in order.
    #[must_use]
    pub fn neighbours(&self, cat: &[usize]) -> Vec<Vec<usize>> {
        let mut out = Vec::new();
        for (k, &i) in self.cat.iter().enumerate() {
            if let AxisKind::Categorical { values } = &self.axes[i].kind {
                for v in 0..values.len() {
                    if cat.get(k) != Some(&v) {
                        let mut n = cat.to_vec();
                        n[k] = v;
                        out.push(n);
                    }
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(s: &str) -> ParamAddr {
        s.parse().unwrap()
    }

    #[test]
    fn axes_follow_the_param_spec() {
        let p = Patch::init();
        let free = FreeSet::new(
            &p,
            &[
                addr("osc1.wave"),
                addr("osc1.semi"),
                addr("osc1.sync"),
                addr("osc1.seed"),
                addr("voice.polyphony"),
                addr("osc1.semi"),
            ],
        )
        .unwrap();
        // The duplicate is dropped; wave and sync are categorical; semi, seed (65,536 values) and polyphony (16) are
        // continuous.
        assert_eq!(free.len(), 5);
        assert_eq!(free.categorical(), &[0, 2]);
        assert_eq!(free.continuous(), &[1, 3, 4]);
        match &free.axes()[0].kind {
            AxisKind::Categorical { values } => assert_eq!(values.len(), 5),
            AxisKind::Continuous => panic!("wave is categorical"),
        }
        let pt = free.start_point();
        assert_eq!(pt, free.point_of(&p));
        assert_eq!(pt.cat, vec![2, 0], "saw is item 2, sync off");
        assert!((pt.cont[0] - 0.5).abs() < 1e-6, "semi 0 sits mid-range");
    }

    #[test]
    fn unknown_and_empty_sets_are_errors() {
        let p = Patch::init();
        assert!(matches!(
            FreeSet::new(&p, &[addr("osc9.semi")]),
            Err(SearchError::UnknownParam { .. })
        ));
        assert!(matches!(
            FreeSet::new(&p, &[addr("osc1.cutoff")]),
            Err(SearchError::UnknownParam { .. })
        ));
        assert_eq!(
            FreeSet::new(&p, &[]).unwrap_err(),
            SearchError::NoFreeParameters
        );
    }

    #[test]
    fn apply_goes_through_ops_and_round_trips_the_start() {
        let p = Patch::init();
        let free = FreeSet::new(
            &p,
            &[addr("osc1.wave"), addr("osc1.semi"), addr("env1.attack")],
        )
        .unwrap();
        // The point of a patch applied to it gives the patch back exactly, though plain → normalised → plain is not
        // an exact round trip in general (AGENTS.md).
        let same = free.apply(&p, &free.point_of(&p)).unwrap();
        assert_eq!(same, p);
        let moved = free
            .apply(
                &p,
                &Point {
                    cont: vec![1.0, 0.25],
                    cat: vec![3],
                },
            )
            .unwrap();
        assert_eq!(moved.value(&addr("osc1.wave")), Some(3.0));
        assert_eq!(moved.value(&addr("osc1.semi")), Some(48.0));
        let attack = moved.value(&addr("env1.attack")).unwrap();
        // Power(4): 0.25 of the knob is 10,000 · 0.25⁴ = 39.06 ms.
        assert!((attack - 39.0625).abs() < 0.01, "{attack}");
        // Out-of-range normalised positions clamp instead of failing.
        let clamped = free
            .apply(
                &p,
                &Point {
                    cont: vec![7.0, -3.0],
                    cat: vec![0],
                },
            )
            .unwrap();
        assert_eq!(clamped.value(&addr("osc1.semi")), Some(48.0));
        assert_eq!(clamped.value(&addr("env1.attack")), Some(0.0));
    }

    #[test]
    fn neighbours_flip_one_axis_at_a_time() {
        let p = Patch::init();
        let free = FreeSet::new(
            &p,
            &[addr("osc1.wave"), addr("osc1.sync"), addr("osc1.level")],
        )
        .unwrap();
        let n = free.neighbours(&[2, 0]);
        // wave: 4 others; sync: 1 other.
        assert_eq!(n.len(), 5);
        assert_eq!(n[0], vec![0, 0]);
        assert_eq!(n[3], vec![4, 0]);
        assert_eq!(n[4], vec![2, 1]);
    }
}
