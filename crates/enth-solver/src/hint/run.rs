//! The search behind [`hint`](super::hint): probing, model steps verified by renders, the presence search for
//! absent descriptors, finalising, and the blocked verdicts. Private: the public types live in the parent module.

use enth_analysis::descriptors::{Descriptors, Feature, Want};
use enth_analysis::expect::{Expect, ItemResult, Test};
use enth_params::Unit;
use enth_patch::{Op, ParamAddr, Patch, apply};
use enth_render::{Performance, RenderSpec};

use super::model::{self, Aim, Matrix, Problem, Role, Row};
use super::space::{self, FreeParam};
use super::{BlockReason, Blocked, Fix, HintCfg, HintError, HintResult, HintStats, Move};
use crate::eval::{EvalError, Evaluator, Job, note_timing};

/// Descriptors that a short clip measures as well as the full performance.
fn spectral(f: Feature) -> bool {
    matches!(
        f,
        Feature::Pitch
            | Feature::PitchCents
            | Feature::PitchStability
            | Feature::Brightness
            | Feature::Noisiness
    )
}

/// A probe that moves an item by less than this many aim margins counts as no effect: descriptor quantisation
/// (1 ms envelope frames, YIN's resolution) would otherwise give every parameter a small, meaningless slope that
/// a far-off item then amplifies into large moves.
const DEAD_ZONE: f64 = 0.1;
/// A model prediction within this many aim margins of its band counts as reaching it.
const REACH_TOL: f64 = 0.05;
/// The envelope's frame (its 1 ms hop): time descriptors move in steps of this size.
const ENVELOPE_FRAME_MS: f64 = 1.0;
/// A parameter counts as helping an item when a full-range move shifts it by at least this many aim margins.
const HELPS: f64 = 0.5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Base,
    Probe,
    Absent,
    Verify,
    Final,
}

/// Pass/fail of every item on one render.
#[derive(Clone, Debug, PartialEq)]
struct Judged {
    pass: Vec<bool>,
    fails: usize,
    shortfall: f64,
}

impl Judged {
    /// `self` is acceptable over `best`: no item that passes in `best` fails here, and it fails fewer items or as
    /// many with a smaller total shortfall.
    fn improves_on(&self, best: &Judged) -> bool {
        !self.regresses(best)
            && (self.fails < best.fails
                || (self.fails == best.fails && self.shortfall < best.shortfall - 1e-9))
    }

    /// Some item that passes in `other` fails here.
    fn regresses(&self, other: &Judged) -> bool {
        other.pass.iter().zip(&self.pass).any(|(o, s)| *o && !*s)
    }
}

/// A measured point of the search.
#[derive(Clone, Debug)]
struct Point {
    patch: Patch,
    /// Normalised positions of the probed parameters.
    u: Vec<f64>,
    d: Descriptors,
    judged: Judged,
}

pub(super) struct Run<'a> {
    patch: &'a Patch,
    perf: &'a Performance,
    short: Option<Performance>,
    spec: RenderSpec,
    expects: &'a [Expect],
    roles: Vec<Role>,
    /// The aim of each item the model steers (None: null tests and performance-limited items).
    aims: Vec<Option<Aim>>,
    /// Items the performance makes impossible, with the reason.
    perf_limit: Vec<Option<String>>,
    want: Want,
    eval: &'a mut dyn Evaluator,
    cfg: &'a HintCfg,
    stats: HintStats,
    trace: Vec<String>,
}

impl<'a> Run<'a> {
    pub(super) fn new(
        patch: &'a Patch,
        perf: &'a Performance,
        spec: &RenderSpec,
        expects: &'a [Expect],
        eval: &'a mut dyn Evaluator,
        cfg: &'a HintCfg,
    ) -> Self {
        let roles: Vec<Role> = expects.iter().map(Role::of).collect();
        let timing = note_timing(perf);
        let no_release = timing.off_s.is_none_or(|off| off >= perf.duration_s);
        let perf_limit: Vec<Option<String>> = expects
            .iter()
            .map(|e| {
                (e.feature == Feature::Release && no_release).then(|| {
                    "the note is never released inside the clip, so there is no release to measure; lengthen the \
                     performance or shorten the hold"
                        .to_string()
                })
            })
            .collect();
        let aims = roles
            .iter()
            .zip(&perf_limit)
            .map(|(r, l)| if l.is_some() { None } else { r.aim().copied() })
            .collect();
        let features: Vec<Feature> = expects.iter().map(|e| e.feature).collect();
        let short = cfg
            .short_clip_s
            .filter(|s| *s > 0.0 && *s < perf.duration_s)
            .filter(|_| features.iter().all(|f| spectral(*f)))
            .map(|s| Performance {
                notes: perf.notes.clone(),
                automation: perf.automation.clone(),
                duration_s: s,
            });
        Self {
            patch,
            perf,
            short,
            spec: *spec,
            expects,
            roles,
            aims,
            perf_limit,
            want: Want::for_features(&features),
            eval,
            cfg,
            stats: HintStats::default(),
            trace: Vec::new(),
        }
    }

    fn note(&mut self, text: impl FnOnce() -> String) {
        if self.cfg.trace {
            self.trace.push(text());
        }
    }

    fn left(&self) -> usize {
        self.cfg.max_renders.saturating_sub(self.stats.renders)
    }

    /// Renders kept back after the probe: the final render and at least `min(3, max_steps)` verified steps.
    fn reserve(&self) -> usize {
        1 + self.cfg.max_steps.min(3)
    }

    /// Evaluate `(patch, on the short clip)` jobs in one batch, counting them under `phase`.
    fn evaluate(
        &mut self,
        jobs: Vec<(Patch, bool)>,
        phase: Phase,
    ) -> Result<Vec<Result<Descriptors, EvalError>>, HintError> {
        if jobs.is_empty() {
            return Ok(Vec::new());
        }
        let n = jobs.len();
        let shorts = jobs.iter().filter(|(_, s)| *s).count();
        let out = {
            let short = self.short.as_ref();
            let batch: Vec<Job<'_>> = jobs
                .into_iter()
                .map(|(patch, is_short)| Job {
                    patch,
                    perf: if is_short {
                        short.unwrap_or(self.perf)
                    } else {
                        self.perf
                    },
                    spec: self.spec,
                    want: self.want,
                })
                .collect();
            self.eval.evaluate(&batch)
        };
        if out.len() != n {
            return Err(HintError::Contract {
                jobs: n,
                results: out.len(),
            });
        }
        let s = &mut self.stats;
        s.renders += n;
        s.batches += 1;
        s.short_renders += shorts;
        s.failed += out.iter().filter(|r| r.is_err()).count();
        match phase {
            Phase::Base => s.base_renders += n,
            Phase::Probe => s.probe_renders += n,
            Phase::Absent => s.absent_renders += n,
            Phase::Verify => s.verify_renders += n,
            Phase::Final => s.final_renders += n,
        }
        Ok(out)
    }

    fn judge(&self, d: &Descriptors) -> Judged {
        let pass: Vec<bool> = self
            .expects
            .iter()
            .map(|e| e.evaluate(d, &[]).pass)
            .collect();
        let shortfall = self
            .expects
            .iter()
            .zip(&self.roles)
            .zip(&pass)
            .map(|((e, r), p)| model::shortfall(e, r, d, *p))
            .sum();
        Judged {
            fails: pass.iter().filter(|p| !**p).count(),
            pass,
            shortfall,
        }
    }

    /// Each item's value in its search domain (None: absent, or not steered).
    fn values(&self, d: &Descriptors) -> Vec<Option<f64>> {
        self.expects
            .iter()
            .zip(&self.aims)
            .map(|(e, a)| a.as_ref().and_then(|a| a.value(e, d)))
            .collect()
    }

    /// The model rows at a point, leaving out `skip`ped items.
    fn rows(&self, point: &Point, skip: &[bool]) -> Vec<Option<Row>> {
        let g = self.values(&point.d);
        (0..self.expects.len())
            .map(|i| {
                if skip[i] {
                    return None;
                }
                Some(self.aims[i]?.row(g[i]?, point.judged.pass[i]))
            })
            .collect()
    }

    /// The patch with every probed parameter at `u` (parameters at their original position are untouched), and
    /// the positions it actually realises.
    fn at(&self, free: &[FreeParam], u: &[f64]) -> (Patch, Vec<f64>) {
        let values: Vec<f32> = free
            .iter()
            .zip(u)
            .map(|(f, &x)| f.spec.from_norm(x.clamp(0.0, 1.0) as f32))
            .collect();
        let p = self.with_values(free, &values);
        let real = free
            .iter()
            .map(|f| f64::from(f.spec.to_norm(p.value(&f.addr).unwrap_or(f.plain))))
            .collect();
        (p, real)
    }

    /// The patch with plain values (one per probed parameter) set through ops.
    fn with_values(&self, free: &[FreeParam], values: &[f32]) -> Patch {
        let mut p = self.patch.clone();
        for (f, &v) in free.iter().zip(values) {
            if v.to_bits() != f.plain.to_bits() {
                // A rejected value leaves the parameter where it was (the op checks first, then mutates).
                let _ = apply(
                    &mut p,
                    &Op::SetParam {
                        addr: f.addr.clone(),
                        value: v,
                    },
                );
            }
        }
        p
    }

    /// Item `i` asks for its descriptor to be present (`!= null`, or an interval on it).
    fn wants_presence(&self, i: usize) -> bool {
        matches!(self.expects[i].test, Test::NotNull | Test::Within(_))
    }

    /// The item fails because its descriptor is absent, and presence could fix that.
    fn absent(&self, i: usize, point: &Point) -> bool {
        !point.judged.pass[i]
            && self.perf_limit[i].is_none()
            && self.wants_presence(i)
            && self.expects[i].compared(&point.d).is_none()
    }

    pub(super) fn go(mut self) -> Result<HintResult, HintError> {
        let m = self.expects.len();
        // 1. Base.
        let base_d = self
            .evaluate(vec![(self.patch.clone(), false)], Phase::Base)?
            .pop()
            .ok_or(HintError::Contract {
                jobs: 1,
                results: 0,
            })?
            .map_err(HintError::Base)?;
        let before: Vec<ItemResult> = self
            .expects
            .iter()
            .map(|e| e.evaluate(&base_d, &[]))
            .collect();
        let base_judged = self.judge(&base_d);
        let all_free = space::free_params(
            self.patch,
            self.perf,
            self.cfg.only.as_deref(),
            &self.cfg.frozen,
            self.cfg.transpose,
        );
        self.stats.free_params = all_free.len();
        let mut result = HintResult {
            before,
            fix: None,
            blocked: Vec::new(),
            probed: Vec::new(),
            stats: HintStats::default(),
            trace: Vec::new(),
        };
        let mut base = Point {
            patch: self.patch.clone(),
            u: Vec::new(),
            d: base_d,
            judged: base_judged,
        };
        if base.judged.fails == 0 {
            result.stats = self.stats;
            return Ok(result);
        }
        let absent_at_base = (0..m).any(|i| self.absent(i, &base));
        let actionable = (0..m).any(|i| {
            !base.judged.pass[i]
                && self.perf_limit[i].is_none()
                && (self.aims[i].is_some() || self.absent(i, &base))
        });

        // 2. Choose the parameters to probe: related ones first, unrelated ones only with room to spare.
        let failing: Vec<Feature> = (0..m)
            .filter(|&i| !base.judged.pass[i])
            .map(|i| self.expects[i].feature)
            .collect();
        let all: Vec<Feature> = self.expects.iter().map(|e| e.feature).collect();
        let mut order: Vec<(u8, usize)> = all_free
            .iter()
            .enumerate()
            .map(|(i, p)| (space::priority(p, &all_free, &failing, &all), i))
            .collect();
        order.sort_unstable();
        let absent_reserve = if absent_at_base {
            self.cfg.absent_probe.min(4)
        } else {
            0
        };
        let short_base = usize::from(self.short.is_some());
        let keep = absent_reserve + short_base;
        let room_min = self.left().saturating_sub(self.reserve() + keep);
        let room_full = self.left().saturating_sub(1 + self.cfg.max_steps + keep);
        let related = order.iter().filter(|(p, _)| *p <= 2).count();
        let n_main = if actionable { related.min(room_min) } else { 0 };
        let n_extra = if actionable {
            (order.len() - related).min(room_full.saturating_sub(n_main))
        } else {
            0
        };
        let mut chosen: Vec<usize> = order[..n_main]
            .iter()
            .chain(&order[related..related + n_extra])
            .map(|(_, i)| *i)
            .collect();
        chosen.sort_unstable();
        let free: Vec<FreeParam> = chosen.iter().map(|&i| all_free[i].clone()).collect();
        self.stats.probed_params = free.len();
        result.probed = free.iter().map(|f| f.addr.clone()).collect();
        base.u = free.iter().map(|f| f.u).collect();

        // 3. Probe: the finite-difference Jacobian.
        let mut jac = Matrix::zeros(m, free.len());
        let probe_points = self.probe(&free, &base, &mut jac)?;

        // 4-6. Model steps, verified by renders. Items the model cannot bring into their band even alone are left out
        // of the objective (they would drag every parameter to a bound for nothing) and reported as blocked. A
        // descriptor that is absent has no slope: when it appears along the way its row is measured there; when the
        // steps stall without it, look for it further out (and then step again).
        let mut best = base.clone();
        let mut cur = base.clone();
        let mut absent_tried: Vec<ParamAddr> = Vec::new();
        let g0 = self.values(&base.d);
        let mut row_known: Vec<bool> = g0.iter().map(Option::is_some).collect();
        let w_max = self
            .aims
            .iter()
            .flatten()
            .map(|a| 1.0 / (a.scale * a.scale))
            .fold(0.0, f64::max)
            .max(1.0);
        let mut lambda = 1e-3 * w_max;
        let mut steps = 0;
        let mut searched = false;
        let mut long_shot = false;
        loop {
            let mut unreachable = self.unreachable(&cur, &jac);
            self.note(|| format!("unreachable items: {unreachable:?}"));
            while !free.is_empty() && best.judged.fails > 0 && steps < self.cfg.max_steps {
                if self.left() <= 1 {
                    self.stats.budget_exhausted = true;
                    break;
                }
                let rows = self.rows(&cur, &unreachable);
                if !(0..m).any(|i| !cur.judged.pass[i] && rows[i].is_some()) {
                    break;
                }
                steps += 1;
                let du = model::step(&Problem {
                    rows: &rows,
                    j: &jac,
                    u: &cur.u,
                    lambda,
                });
                if du.iter().all(|x| x.abs() < 1e-4) {
                    break;
                }
                let target: Vec<f64> = cur.u.iter().zip(&du).map(|(u, d)| u + d).collect();
                let (p, real) = self.at(&free, &target);
                let r = self
                    .evaluate(vec![(p.clone(), false)], Phase::Verify)?
                    .pop();
                let Some(Ok(d)) = r else {
                    lambda *= 4.0;
                    continue;
                };
                let real_du: Vec<f64> = real.iter().zip(&cur.u).map(|(a, b)| a - b).collect();
                self.learn(&mut jac, &cur, &d, &real_du, None);
                let judged = self.judge(&d);
                let accepted = judged.improves_on(&best.judged);
                self.note(|| {
                    format!(
                        "step {steps}: lambda {lambda:.3e}, du {du:.3?}, fails {} shortfall {:.3}, accepted {accepted}",
                        judged.fails, judged.shortfall
                    )
                });
                if !accepted {
                    lambda *= 4.0;
                    continue;
                }
                let point = Point {
                    patch: p,
                    u: real,
                    d,
                    judged,
                };
                best = point.clone();
                cur = point;
                lambda = (lambda * 0.5).max(1e-6);
                self.stats.accepted_steps += 1;
                // An item that just appeared gets its row measured here.
                let g = self.values(&cur.d);
                let appeared: Vec<usize> = (0..m)
                    .filter(|&i| !row_known[i] && g[i].is_some())
                    .collect();
                for i in appeared {
                    self.measure_row(i, &free, &cur, &mut jac)?;
                    row_known[i] = true;
                    unreachable = self.unreachable(&cur, &jac);
                }
            }
            if free.is_empty() || best.judged.fails == 0 {
                break;
            }
            // A long shot for the items the linear model calls unreachable (a dB reading driven by a linear-%
            // parameter falls much faster near zero than its slope says): one step that includes them, kept only if
            // it improves on the best point (fewer failing items, or as many with a smaller shortfall), and the
            // loop then goes on from there, where the slope is steeper. A truly hopeless item costs one render.
            if !long_shot
                && self.left() > 1
                && (0..m).any(|i| unreachable[i] && !best.judged.pass[i])
            {
                long_shot = true;
                let rows = self.rows(&best, &vec![false; m]);
                let du = model::step(&Problem {
                    rows: &rows,
                    j: &jac,
                    u: &best.u,
                    lambda: 1e-6,
                });
                if du.iter().any(|x| x.abs() >= 1e-4) {
                    let target: Vec<f64> = best.u.iter().zip(&du).map(|(u, d)| u + d).collect();
                    let (p, real) = self.at(&free, &target);
                    let r = self
                        .evaluate(vec![(p.clone(), false)], Phase::Verify)?
                        .pop();
                    if let Some(Ok(d)) = r {
                        let real_du: Vec<f64> =
                            real.iter().zip(&best.u).map(|(a, b)| a - b).collect();
                        self.learn(&mut jac, &best, &d, &real_du, None);
                        let judged = self.judge(&d);
                        let accepted = judged.improves_on(&best.judged);
                        self.note(|| {
                            format!(
                                "long shot: du {du:.3?}, fails {} shortfall {:.3}, accepted {accepted}",
                                judged.fails, judged.shortfall
                            )
                        });
                        if accepted {
                            best = Point {
                                patch: p,
                                u: real,
                                d,
                                judged,
                            };
                            cur = best.clone();
                            self.stats.accepted_steps += 1;
                            continue;
                        }
                    }
                }
            }
            if searched || steps >= self.cfg.max_steps {
                break;
            }
            searched = true;
            let Some(i) = (0..m).find(|&i| self.absent(i, &best)) else {
                break;
            };
            let known: &[(usize, f64, Descriptors)] = if best.patch == base.patch {
                &probe_points
            } else {
                &[]
            };
            let Some(q) = self.search_presence(i, &free, &best, known, &mut absent_tried)? else {
                break;
            };
            self.note(|| format!("absent item {i} appears at {:?}", q.u));
            let jump: Vec<f64> = q.u.iter().zip(&best.u).map(|(a, b)| a - b).collect();
            self.learn(&mut jac, &best, &q.d, &jump, Some(i));
            self.measure_row(i, &free, &q, &mut jac)?;
            row_known[i] = true;
            if q.judged.improves_on(&best.judged) {
                best = q.clone();
                self.stats.accepted_steps += 1;
            }
            cur = q;
        }

        // 7. Finalise: sparsify and round, then measure what will be printed.
        let final_point = if best.patch == *self.patch {
            None
        } else {
            Some(self.finalise(&free, &base, &best)?)
        };

        // 8. Blocked verdicts at the final point.
        let at = final_point.as_ref().map_or(&best, |(p, _)| p);
        result.blocked = self.blocked(at, &free, &jac, &absent_tried);
        if let Some((p, rounded)) = final_point {
            let moves: Vec<Move> = free
                .iter()
                .filter_map(|f| {
                    let to = p.patch.value(&f.addr)?;
                    (to.to_bits() != f.plain.to_bits()).then(|| Move {
                        addr: f.addr.clone(),
                        spec: f.spec,
                        from: f.plain,
                        to,
                    })
                })
                .collect();
            let after: Vec<ItemResult> =
                self.expects.iter().map(|e| e.evaluate(&p.d, &[])).collect();
            let pass_all = after.iter().all(|r| r.pass);
            result.fix = Some(Fix {
                single_param_enough: pass_all && moves.len() == 1,
                moves,
                patch: p.patch,
                after,
                descriptors: p.d,
                pass_all,
                rounded,
            });
        }
        if self.left() == 0 && best.judged.fails > 0 {
            self.stats.budget_exhausted = true;
        }
        result.stats = self.stats;
        result.trace = self.trace;
        Ok(result)
    }

    /// One render per probed parameter at `u ± probe_step` (toward the middle), in one batch, filling `jac`.
    /// Returns the probe points measured with the full performance (`(parameter, step, descriptors)`).
    fn probe(
        &mut self,
        free: &[FreeParam],
        base: &Point,
        jac: &mut Matrix,
    ) -> Result<Vec<(usize, f64, Descriptors)>, HintError> {
        let mut points = Vec::new();
        if free.is_empty() {
            return Ok(points);
        }
        let m = self.expects.len();
        let short = self.short.is_some();
        let mut jobs = Vec::new();
        if short {
            jobs.push((self.patch.clone(), true));
        }
        let mut steps = Vec::new();
        for (k, f) in free.iter().enumerate() {
            let mut u = base.u.clone();
            u[k] = self.probe_target(f, f.u);
            let (p, real) = self.at(free, &u);
            let du = real[k] - f.u;
            if du.abs() > 1e-9 {
                steps.push((k, du));
                jobs.push((p, short));
            }
        }
        let mut out = self.evaluate(jobs, Phase::Probe)?.into_iter();
        let reference = if short {
            match out.next() {
                Some(Ok(d)) => Some(d),
                _ => None,
            }
        } else {
            Some(base.d.clone())
        };
        for ((k, du), r) in steps.into_iter().zip(out) {
            let Ok(d) = r else { continue };
            if let Some(reference) = &reference {
                for i in 0..m {
                    if let Some(change) = self.change(i, reference, &d) {
                        jac.set(i, k, change / du);
                    }
                }
            }
            if !short {
                points.push((k, du, d));
            }
        }
        if self.cfg.trace {
            for i in 0..m {
                let row = jac.row(i).to_vec();
                self.note(|| format!("J[{i}] = {row:.3?}"));
            }
        }
        Ok(points)
    }

    /// Failing items the model cannot bring into their band even when every probed parameter serves them alone.
    fn unreachable(&self, at: &Point, jac: &Matrix) -> Vec<bool> {
        let m = self.expects.len();
        let all = self.rows(at, &vec![false; m]);
        (0..m)
            .map(|i| {
                let Some(row) = all[i] else { return false };
                if at.judged.pass[i] || at.u.is_empty() {
                    return false;
                }
                !self.solo(i, row, at, jac).1
            })
            .collect()
    }

    /// The model step for item `i` alone from `at`, and whether it reaches the band.
    fn solo(&self, i: usize, row: Row, at: &Point, jac: &Matrix) -> (Vec<f64>, bool) {
        let mut rows = vec![None; self.expects.len()];
        rows[i] = Some(row);
        let du = model::step(&Problem {
            rows: &rows,
            j: jac,
            u: &at.u,
            lambda: 1e-9,
        });
        let scale = self.aims[i].map_or(1.0, |a| a.scale);
        let reach = row.dist(row.g + jac.dot_row(i, &du)).abs() <= REACH_TOL * scale;
        (du, reach)
    }

    /// The parameters to look along for item `i`'s descriptor: those whose `ParamSpec::perceptual` is in its
    /// family, or every probed parameter when none is.
    fn movers(&self, i: usize, free: &[FreeParam]) -> Vec<usize> {
        let fam = space::family(self.expects[i].feature);
        let cands: Vec<usize> = (0..free.len())
            .filter(|&k| space::perceptual(free[k].spec).is_some_and(|f| space::family(f) == fam))
            .collect();
        if cands.is_empty() {
            (0..free.len()).collect()
        } else {
            cands
        }
    }

    /// Renders the presence search and the row probe may use: the absent-probe cap, keeping back the final render
    /// and one verified step.
    fn absent_budget(&self) -> usize {
        self.cfg
            .absent_probe
            .min(self.left().saturating_sub(1 + self.cfg.max_steps.min(1)))
    }

    /// Look along the parameters that move item `i`'s descriptor, `u ± 0.3` then `± 0.6` from `from`, for a point
    /// where it appears. `known` are already measured points around `from` (`(parameter, step, descriptors)`).
    /// Returns the point closest to passing, or `None`.
    fn search_presence(
        &mut self,
        i: usize,
        free: &[FreeParam],
        from: &Point,
        known: &[(usize, f64, Descriptors)],
        tried: &mut Vec<ParamAddr>,
    ) -> Result<Option<Point>, HintError> {
        let cands = self.movers(i, free);
        let mut budget = self.absent_budget();
        let present = |run: &Self, d: &Descriptors| run.expects[i].compared(d).is_some();
        let mut found: Vec<Point> = Vec::new();
        for (k, du, d) in known {
            if present(self, d) {
                let mut u = from.u.clone();
                u[*k] += du;
                let (patch, real) = self.at(free, &u);
                found.push(Point {
                    patch,
                    u: real,
                    judged: self.judge(d),
                    d: d.clone(),
                });
            }
        }
        // One offset per parameter per round, until it appears or the budget runs out.
        for round in 0..4 {
            if !found.is_empty() || budget == 0 {
                break;
            }
            let mut jobs = Vec::new();
            let mut where_ = Vec::new();
            for &k in &cands {
                if jobs.len() == budget {
                    break;
                }
                let offsets: Vec<f64> = [0.3, -0.3, 0.6, -0.6]
                    .into_iter()
                    .filter(|o| (0.0..=1.0).contains(&(from.u[k] + o)))
                    .collect();
                let Some(off) = offsets.get(round) else {
                    continue;
                };
                let mut u = from.u.clone();
                u[k] += off;
                let (p, real) = self.at(free, &u);
                if !tried.contains(&free[k].addr) {
                    tried.push(free[k].addr.clone());
                }
                where_.push(real);
                jobs.push((p, false));
            }
            if jobs.is_empty() {
                continue;
            }
            budget -= jobs.len();
            let patches: Vec<Patch> = jobs.iter().map(|(p, _)| p.clone()).collect();
            let out = self.evaluate(jobs, Phase::Absent)?;
            for ((patch, u), r) in patches.into_iter().zip(where_).zip(out) {
                if let Ok(d) = r
                    && present(self, &d)
                {
                    found.push(Point {
                        patch,
                        u,
                        judged: self.judge(&d),
                        d,
                    });
                }
            }
        }
        // The point closest to passing: fewest failures, then smallest shortfall, then first found.
        Ok(found.into_iter().reduce(|a, b| {
            let better = b.judged.fails < a.judged.fails
                || (b.judged.fails == a.judged.fails && b.judged.shortfall < a.judged.shortfall);
            if better { b } else { a }
        }))
    }

    /// Measure item `i`'s row of the Jacobian at `at` along the parameters that move its descriptor.
    fn measure_row(
        &mut self,
        i: usize,
        free: &[FreeParam],
        at: &Point,
        jac: &mut Matrix,
    ) -> Result<(), HintError> {
        let cands = self.movers(i, free);
        let budget = self.absent_budget();
        let mut jobs = Vec::new();
        let mut steps = Vec::new();
        for &k in cands.iter().take(budget) {
            let mut u = at.u.clone();
            u[k] = self.probe_target(&free[k], at.u[k]);
            let (p, real) = self.at(free, &u);
            let du = real[k] - at.u[k];
            if du.abs() > 1e-9 {
                steps.push((k, du));
                jobs.push((p, false));
            }
        }
        let out = self.evaluate(jobs, Phase::Absent)?;
        let mut row = vec![0.0f64; free.len()];
        for ((k, du), r) in steps.into_iter().zip(out) {
            if let Ok(d) = r
                && let Some(change) = self.change(i, &at.d, &d)
            {
                row[k] = change / du;
            }
        }
        for (k, v) in row.into_iter().enumerate() {
            jac.set(i, k, v);
        }
        Ok(())
    }

    /// Broyden-update `jac` with a measured move `du` from `from` to descriptors `to` (every row with a reading at
    /// both ends, except `skip`).
    fn learn(
        &self,
        jac: &mut Matrix,
        from: &Point,
        to: &Descriptors,
        du: &[f64],
        skip: Option<usize>,
    ) {
        let a = self.values(&from.d);
        let b = self.values(to);
        let dg: Vec<Option<f64>> = a
            .iter()
            .zip(&b)
            .enumerate()
            .map(|(r, (x, y))| {
                if Some(r) == skip {
                    None
                } else {
                    Some((*y)? - (*x)?)
                }
            })
            .collect();
        model::broyden(jac, du, &dg);
    }

    /// The probe step for one parameter on its normalised axis. Transposition is probed by one semitone: a 10 %
    /// step of ±48 st is 9.6 st, which changes which pitch the analysis locks onto rather than measuring a slope.
    fn probe_du(&self, f: &FreeParam) -> f64 {
        let h = self.cfg.probe_step.clamp(1e-4, 0.5);
        if f.spec.unit == Unit::Semitones {
            let range = f64::from(f.spec.max() - f.spec.min());
            h.min(1.0 / range.max(1.0))
        } else {
            h
        }
    }

    /// Where to probe a parameter at `u`: one probe step toward the middle of its range.
    fn probe_target(&self, f: &FreeParam, u: f64) -> f64 {
        let h = self.probe_du(f);
        if u <= 0.5 { u + h } else { u - h }
    }

    /// The change of item `i` between two renders in its search domain, with measurement noise removed: a
    /// change under [`DEAD_ZONE`] aim margins, or under one envelope frame for time features, counts as 0.
    fn change(&self, i: usize, from: &Descriptors, to: &Descriptors) -> Option<f64> {
        let aim = self.aims[i]?;
        let e = &self.expects[i];
        let (a, b) = (aim.value(e, from)?, aim.value(e, to)?);
        let frames = matches!(
            e.feature,
            Feature::Attack | Feature::Decay | Feature::Release | Feature::Duration
        );
        if frames
            && let (Some(pa), Some(pb)) = (e.compared(from), e.compared(to))
            && (pb - pa).abs() <= ENVELOPE_FRAME_MS
        {
            return Some(0.0);
        }
        if (b - a).abs() < DEAD_ZONE * aim.scale {
            return Some(0.0);
        }
        Some(b - a)
    }

    /// Drop small moves and round, render, and keep the result if it holds; else try rounding alone; else keep
    /// the exact best. Returns the point and whether its values are rounded.
    fn finalise(
        &mut self,
        free: &[FreeParam],
        base: &Point,
        best: &Point,
    ) -> Result<(Point, bool), HintError> {
        let best_vals: Vec<f32> = free
            .iter()
            .map(|f| best.patch.value(&f.addr).unwrap_or(f.plain))
            .collect();
        let round = |f: &FreeParam, v: f32| -> f32 {
            let Some(digits) = self.cfg.round_digits else {
                return v;
            };
            if v.to_bits() == f.plain.to_bits() {
                return v;
            }
            let text = f.spec.display_rounded(v, digits).to_string();
            match f.spec.parse(&text) {
                Ok(r) if f.spec.is_valid(r) => r,
                _ => v,
            }
        };
        let sparse: Vec<f32> = free
            .iter()
            .zip(&best_vals)
            .zip(best.u.iter().zip(&base.u))
            .map(|((f, &v), (&ub, &u0))| {
                if (ub - u0).abs() < self.cfg.sparsify_below {
                    f.plain
                } else {
                    round(f, v)
                }
            })
            .collect();
        let rounded: Vec<f32> = free
            .iter()
            .zip(&best_vals)
            .map(|(f, &v)| round(f, v))
            .collect();
        let mut tried: Vec<Vec<f32>> = Vec::new();
        for cand in [sparse, rounded] {
            let same_as_best = cand
                .iter()
                .zip(&best_vals)
                .all(|(a, b)| a.to_bits() == b.to_bits());
            if same_as_best {
                return Ok((best.clone(), self.cfg.round_digits.is_some()));
            }
            let all_base = cand
                .iter()
                .zip(free)
                .all(|(v, f)| v.to_bits() == f.plain.to_bits());
            if all_base || tried.contains(&cand) || self.left() == 0 {
                continue;
            }
            tried.push(cand.clone());
            let patch = self.with_values(free, &cand);
            let r = self
                .evaluate(vec![(patch.clone(), false)], Phase::Final)?
                .pop();
            if let Some(Ok(d)) = r {
                let judged = self.judge(&d);
                let holds = !judged.regresses(&best.judged);
                self.note(|| format!("final candidate holds: {holds}"));
                if holds {
                    let u = free
                        .iter()
                        .zip(&cand)
                        .map(|(f, v)| f64::from(f.spec.to_norm(*v)))
                        .collect();
                    return Ok((
                        Point {
                            patch,
                            u,
                            d,
                            judged,
                        },
                        true,
                    ));
                }
            }
        }
        Ok((best.clone(), false))
    }

    /// Verdicts for the items still failing at `at`, from the local model there.
    fn blocked(
        &self,
        at: &Point,
        free: &[FreeParam],
        jac: &Matrix,
        absent_tried: &[ParamAddr],
    ) -> Vec<Blocked> {
        let m = self.expects.len();
        let rows = self.rows(at, &vec![false; m]);
        let hold = {
            let t = note_timing(self.perf);
            t.off_s.map(|off| off - t.on_s)
        };
        let joint = (!free.is_empty()).then(|| {
            model::step(&Problem {
                rows: &rows,
                j: jac,
                u: &at.u,
                lambda: 1e-9,
            })
        });
        let mut out = Vec::new();
        for i in 0..m {
            if at.judged.pass[i] {
                continue;
            }
            let e = &self.expects[i];
            let mut b = Blocked {
                item: i,
                expect: e.canonical.clone(),
                reason: BlockReason::AtBound,
                at_bound: Vec::new(),
                conflicts_with: Vec::new(),
                why: String::new(),
            };
            if let Some(why) = &self.perf_limit[i] {
                b.reason = BlockReason::Performance;
                b.why.clone_from(why);
                out.push(b);
                continue;
            }
            if at.d.is_lower_bound(e.feature) {
                b.reason = BlockReason::Performance;
                b.why = format!(
                    "the sound is still audible when the clip ends, so {} is only a lower bound; lengthen the performance",
                    e.feature.path()
                );
                out.push(b);
                continue;
            }
            if self.absent(i, at) {
                if absent_tried.is_empty() && self.stats.free_params > 0 {
                    // Not looked for (the budget ran out first): unfinished, not blocked.
                    continue;
                }
                b.reason = BlockReason::Absent;
                let tried: Vec<String> = absent_tried.iter().map(ToString::to_string).collect();
                b.why = if tried.is_empty() {
                    format!(
                        "{} is absent ({}), and the patch has no free parameter to move; the patch structure is the limit",
                        e.feature.path(),
                        e.feature.missing_reason()
                    )
                } else {
                    format!(
                        "{} is absent ({}); moving {} did not make it appear, so the patch structure is the limit",
                        e.feature.path(),
                        e.feature.missing_reason(),
                        tried.join(", ")
                    )
                };
                out.push(b);
                continue;
            }
            let (Some(aim), Some(row)) = (self.aims[i], rows[i]) else {
                b.reason = BlockReason::Unmodelled;
                b.why = format!(
                    "the solver judges '{}' but has no continuous measure to aim it with; change the structure (add or remove the module behind {})",
                    e.canonical,
                    e.feature.path()
                );
                out.push(b);
                continue;
            };
            let Some(joint) = &joint else {
                continue;
            };
            let dist_now = row.dist(row.g);
            if dist_now == 0.0 {
                // Inside its band yet failing (a lower-bound reading): nothing to aim at.
                continue;
            }
            if row.dist(row.g + jac.dot_row(i, joint)).abs() <= REACH_TOL * aim.scale {
                // The model thinks a joint move exists: not blocked, only unfinished.
                continue;
            }
            let need = -dist_now.signum();
            let helpful: Vec<usize> = (0..free.len())
                .filter(|&k| jac.at(i, k).abs() >= HELPS * aim.scale)
                .collect();
            if helpful.is_empty() {
                b.reason = BlockReason::NoEffect;
                b.why = format!(
                    "no free parameter moves {} (probed: {}); the patch structure is the limit",
                    e.feature.path(),
                    free.iter()
                        .map(|f| f.addr.to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                );
                out.push(b);
                continue;
            }
            let (solo, reach) = self.solo(i, row, at, jac);
            b.at_bound = helpful
                .iter()
                .filter(|&&k| {
                    let x = at.u[k] + solo[k];
                    let up = need * jac.at(i, k) > 0.0;
                    (up && x >= 1.0 - 1e-6) || (!up && x <= 1e-6)
                })
                .map(|&k| free[k].addr.clone())
                .collect();
            if reach {
                b.reason = BlockReason::Conflict;
                b.conflicts_with = (0..m)
                    .filter(|&k| k != i)
                    .filter(|&k| {
                        let (Some(rk), Some(ak)) = (rows[k], self.aims[k]) else {
                            return false;
                        };
                        let now = rk.dist(rk.g).abs();
                        let then = rk.dist(rk.g + jac.dot_row(k, &solo)).abs();
                        then > now + REACH_TOL * ak.scale
                    })
                    .collect();
                if b.conflicts_with.is_empty() {
                    continue;
                }
                let names: Vec<&str> = b
                    .conflicts_with
                    .iter()
                    .map(|&k| self.expects[k].canonical.as_str())
                    .collect();
                b.why = format!(
                    "the move that would satisfy it pushes {} out; no move of the free parameters satisfies both",
                    names.join(" and ")
                );
                out.push(b);
                continue;
            }
            let names: Vec<String> = b.at_bound.iter().map(ToString::to_string).collect();
            b.why = match names.len() {
                0 => format!(
                    "no move within the parameters' ranges brings {} into its window",
                    e.feature.path()
                ),
                1 => format!("{} would have to go past its limit", names[0]),
                _ => format!("{} would have to go past their limits", names.join(" and ")),
            };
            if e.feature == Feature::Duration
                && let (Some(hold), Test::Within(iv)) = (hold, e.test)
                && iv.hi * 1e-3 < hold
            {
                b.reason = BlockReason::Performance;
                b.why = format!(
                    "a held note sounds for at least its hold ({hold} s) unless its sustain is near silence; {}",
                    b.why
                );
            }
            out.push(b);
        }
        out
    }
}
