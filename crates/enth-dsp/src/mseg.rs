//! MSEG core: a multi-segment envelope with up to 16 points, a curve per segment, and loop, one-shot and sustain-loop
//! modes (S2a lane MSEG). Notes and measurements: `docs/dsp/mseg.md`.
//!
//! The shape is a function of a **position** `x` in `[0, 1)`, a share of the shape's length, kept as a 64-bit integer
//! ([`Cursor`]: `2⁶⁴` is the whole length, as the LFO's phase). The length itself (a note division at the tempo, or
//! milliseconds) only sets how far the position moves per control step ([`synced_increment`], [`length_increment`]),
//! so the shape does not know about time and a tempo change cannot move a point. Both increments are computed
//! **exactly**: the ratio is a quotient of integers (every `f64` and `f32` is one), done in `u128` and rounded up, so
//! a length that is a whole number of control steps (a quarter note at 120 BPM and 48 kHz is 750) ends on exactly
//! that step. Rounding to nearest would end it a step late whenever the quotient fell short by half a count.
//!
//! # The shape
//!
//! Points have a time, a level (both `0..=1`) and a curve (`-1..=1`) for the segment that leaves them. A point's
//! effective time is its own or the previous point's, whichever is later, so points never cross and equal times make
//! a vertical edge. With `x` the position and `i` the last point whose time is at or before `x`:
//!
//! - before the first point's time the output holds that point's level;
//! - between point `i` and `i + 1`: `level[i] + (level[i+1] − level[i]) · f(u)`, `u = (x − t[i]) / (t[i+1] − t[i])`;
//! - after the last point it holds the last level, except in [`Mode::Loop`], where the last segment heads back to
//!   point 1's level and ends there at `x = 1`, so the loop joins (`u = (x − t[n]) / (1 − t[n])`, the curve of point
//!   `n`). When the last point sits at `1` there is no room and the loop jumps at the wrap.
//!
//! # The curve
//!
//! Report 05's rational curve, `f(u) = u / (u + k(1 − u))`, with `k` from the curve `c ∈ (−1, 1)` as
//! `k = (1 − c) / (1 + c)`: `c = 0` gives `k = 1`, a straight line; `c > 0` gives `k < 1`, early (fast, then slow);
//! `c < 0` gives `k > 1`, late. The curve for `−c` is the one for `c` turned half a revolution
//! (`f₋c(u) = 1 − f_c(1 − u)`). The ends are exact steps: `c = +1` jumps to the next level at once (`f = 1` for
//! `u > 0`, the point's own level at `u = 0` exactly) and `c = −1` holds the level and jumps at the next point
//! (`f = 0` for `u < 1`), so 16 points with `−1` curves are a 16-step pattern. One division, no `pow`.
//!
//! Everything is `f64` with `+ − × ÷` only, so the output is the same on every platform; the module casts the result
//! to `f32` once.

/// The most points a shape has.
pub const POINTS: usize = 16;

/// 2⁶⁴ as an `f64`.
const TWO_POW_64: f64 = 4_294_967_296.0 * 4_294_967_296.0;

/// 2⁻⁵³.
const INV_2_53: f64 = 1.0 / 9_007_199_254_740_992.0;

/// How the shape plays.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Repeats the whole length.
    Loop,
    /// Runs once from the start position to the end, then holds the last level.
    OneShot,
    /// Runs to the loop end, loops between the loop points while the key is held, then goes on to the end and holds.
    SustainLoop,
}

/// One point as the core takes it: shares of the length and the level (`0..=1`) and the curve (`-1..=1`) of the
/// segment that leaves it. Out-of-range values are clamped by [`Shape::new`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point {
    /// Where the point sits, as a share of the length.
    pub time: f64,
    /// The level at the point.
    pub level: f64,
    /// The bend of the segment leaving the point: 0 straight, positive early, negative late, ±1 the two steps.
    pub curve: f64,
}

/// How one segment bends (`k` of the rational curve, or one of its two limits).
#[derive(Clone, Copy, Debug, PartialEq)]
enum Bend {
    /// `k = 1`: `f(u) = u`.
    Linear,
    /// `0 < k < ∞`: `f(u) = u / (u + k(1 − u))`.
    Rational(f64),
    /// `k = 0`: the next level at once.
    Jump,
    /// `k = ∞`: the same level until the next point.
    Hold,
}

impl Bend {
    fn new(curve: f64) -> Bend {
        let c = if curve.is_nan() {
            0.0
        } else {
            curve.clamp(-1.0, 1.0)
        };
        if c >= 1.0 {
            Bend::Jump
        } else if c <= -1.0 {
            Bend::Hold
        } else if c == 0.0 {
            Bend::Linear
        } else {
            Bend::Rational((1.0 - c) / (1.0 + c))
        }
    }

    /// The share of the way from one level to the next at `u ∈ [0, 1)` along the segment.
    #[inline]
    fn at(self, u: f64) -> f64 {
        match self {
            Bend::Linear => u,
            Bend::Rational(k) => u / (u + k * (1.0 - u)),
            Bend::Jump => {
                if u > 0.0 {
                    1.0
                } else {
                    0.0
                }
            }
            Bend::Hold => 0.0,
        }
    }
}

/// A shape, compiled from its points: effective times, levels, the bend of each segment, the loop positions and how
/// a level becomes the output (polarity and the velocity amount). `Copy`, so a module can keep the shape before an
/// edit for the glide.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shape {
    n: usize,
    mode: Mode,
    bipolar: bool,
    velocity: f64,
    time: [f64; POINTS],
    level: [f64; POINTS],
    bend: [Bend; POINTS],
    loop_start: u64,
    loop_end: u64,
}

/// A share of the length as a position: `round-down(t · 2⁶⁴)`, saturating at the last position for `t = 1`.
fn position_of(t: f64) -> u64 {
    // `as` saturates, and the product of a share in [0, 1] is exact apart from the f64 rounding of `t`.
    (t * TWO_POW_64) as u64
}

/// A position as a share of the length: its top 53 bits, exact in `f64`, in `[0, 1)`.
#[inline]
fn share_of(pos: u64) -> f64 {
    (pos >> 11) as f64 * INV_2_53
}

impl Shape {
    /// A shape from its used points (1 to [`POINTS`]; more are ignored, none is a flat line at level 0), unipolar,
    /// with no velocity scaling and a sustain loop at the last point. Times are made non-decreasing, and levels,
    /// times and curves clamped.
    #[must_use]
    pub fn new(mode: Mode, points: &[Point]) -> Shape {
        let used = &points[..points.len().min(POINTS)];
        let mut shape = Shape {
            n: used.len().max(1),
            mode,
            bipolar: false,
            velocity: 0.0,
            time: [1.0; POINTS],
            level: [0.0; POINTS],
            bend: [Bend::Linear; POINTS],
            loop_start: 0,
            loop_end: 0,
        };
        let mut previous = 0.0f64;
        for (i, p) in used.iter().enumerate() {
            let t = if p.time.is_nan() {
                0.0
            } else {
                p.time.clamp(0.0, 1.0)
            };
            previous = previous.max(t);
            shape.time[i] = previous;
            shape.level[i] = if p.level.is_nan() {
                0.0
            } else {
                p.level.clamp(0.0, 1.0)
            };
            shape.bend[i] = Bend::new(p.curve);
        }
        shape.with_loop(shape.n, shape.n)
    }

    /// Set the sustain loop: the 1-based points it runs between (`start` to `end`). Values above the used points count
    /// as the last point, below 1 as the first, and a start after the end counts as the end.
    #[must_use]
    pub fn with_loop(mut self, start: usize, end: usize) -> Shape {
        let end = end.clamp(1, self.n);
        let start = start.clamp(1, self.n).min(end);
        self.loop_start = position_of(self.time[start - 1]);
        self.loop_end = position_of(self.time[end - 1]);
        self
    }

    /// Set how a level becomes the output: `bipolar` maps `0..=1` to `-1..=1`, and `velocity` (`0..=1`) is how much
    /// the note's velocity scales it ([`Shape::output`]).
    #[must_use]
    pub fn with_output(mut self, bipolar: bool, velocity: f64) -> Shape {
        self.bipolar = bipolar;
        self.velocity = if velocity.is_nan() {
            0.0
        } else {
            velocity.clamp(0.0, 1.0)
        };
        self
    }

    /// The mode.
    #[must_use]
    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// The number of points used.
    #[must_use]
    pub fn points(&self) -> usize {
        self.n
    }

    /// Point `i`'s effective time (0-based), a share of the length.
    #[must_use]
    pub fn time(&self, i: usize) -> f64 {
        self.time[i]
    }

    /// The level held after the last point: the end of a one-shot, or of the release of a sustain loop.
    #[must_use]
    pub fn final_level(&self) -> f64 {
        self.level[self.n - 1]
    }

    /// `from` to `to` along segment `i` at `u`: written so that `f = 0` gives `from` and `f = 1` gives `to` exactly.
    fn segment(&self, i: usize, from: f64, to: f64, u: f64) -> f64 {
        let f = self.bend[i].at(u);
        from * (1.0 - f) + to * f
    }

    /// The level (`0..=1`) at position `pos`. See the module docs for the shape.
    #[must_use]
    pub fn value(&self, pos: u64) -> f64 {
        let x = share_of(pos);
        let n = self.n;
        if x < self.time[0] {
            return self.level[0];
        }
        let mut i = 0;
        while i + 1 < n && self.time[i + 1] <= x {
            i += 1;
        }
        if i + 1 < n {
            // `time[i + 1]` is after `x`, which is not before `time[i]`: a segment of positive width.
            let (t0, t1) = (self.time[i], self.time[i + 1]);
            self.segment(i, self.level[i], self.level[i + 1], (x - t0) / (t1 - t0))
        } else if self.mode == Mode::Loop {
            // `x < 1` and `x >= time[n - 1]`, so the tail has positive width.
            let t0 = self.time[n - 1];
            self.segment(i, self.level[i], self.level[0], (x - t0) / (1.0 - t0))
        } else {
            self.level[n - 1]
        }
    }

    /// A level as the module's output: `level` (unipolar) or `2·level − 1` (bipolar), times the velocity scale
    /// `1 − a·(1 − v)` with `a` the velocity amount and `v` the note's velocity (`0..=1`; 1 for a global MSEG). The
    /// same factor as `adsr`'s `1 − a + a·v`.
    #[inline]
    #[must_use]
    pub fn output(&self, level: f64, velocity: f64) -> f64 {
        let y = if self.bipolar {
            2.0 * level - 1.0
        } else {
            level
        };
        y * (1.0 - self.velocity * (1.0 - velocity))
    }
}

/// A shape's position for one voice (or the one global MSEG), and whether a one-shot has run its course.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cursor {
    pos: u64,
    ended: bool,
}

impl Cursor {
    /// A cursor at position `pos` (see [`start_position`]).
    #[must_use]
    pub const fn new(pos: u64) -> Cursor {
        Cursor { pos, ended: false }
    }

    /// Begin again at `pos`, as a note-on does.
    pub fn restart(&mut self, pos: u64) {
        *self = Cursor::new(pos);
    }

    /// The position, `2⁶⁴` being the whole length (test support).
    #[must_use]
    pub fn position(&self) -> u64 {
        self.pos
    }

    /// Whether the shape has played to its end and holds its last level (a one-shot, or a sustain loop after its
    /// release; never in [`Mode::Loop`]). It turns true in the control step in which the travelled position reaches
    /// the end of the length, so with an exact increment it is the exact step.
    #[must_use]
    pub fn is_ended(&self, shape: &Shape) -> bool {
        self.ended && shape.mode != Mode::Loop
    }

    /// Move one control step of `inc` position counts. `held` is the key (for a sustain loop: while it is down the
    /// position loops between the loop points, and an equal start and end hold their level).
    #[inline]
    pub fn advance(&mut self, shape: &Shape, inc: u64, held: bool) {
        let (next, wrapped) = self.pos.overflowing_add(inc);
        match shape.mode {
            Mode::Loop => {
                self.ended = false;
                self.pos = next;
            }
            Mode::OneShot => self.run_to_end(next, wrapped),
            Mode::SustainLoop => {
                if held && !self.ended && (wrapped || next >= shape.loop_end) {
                    let span = shape.loop_end - shape.loop_start;
                    // `next − loop_end` modulo 2⁶⁴ is the distance past the loop end, also when `next` wrapped.
                    self.pos = if span == 0 {
                        shape.loop_end
                    } else {
                        shape.loop_start + next.wrapping_sub(shape.loop_end) % span
                    };
                } else {
                    self.run_to_end(next, wrapped);
                }
            }
        }
    }

    fn run_to_end(&mut self, next: u64, wrapped: bool) {
        if self.ended {
            return;
        }
        if wrapped {
            self.pos = u64::MAX;
            self.ended = true;
        } else {
            self.pos = next;
        }
    }

    /// The level (`0..=1`) now: the shape at the position, or its last level once the end is reached.
    #[inline]
    #[must_use]
    pub fn level(&self, shape: &Shape) -> f64 {
        if self.is_ended(shape) {
            shape.final_level()
        } else {
            shape.value(self.pos)
        }
    }
}

/// The position a note starts at for a start phase of `degrees` (`0..=360`; 360 is the start).
#[must_use]
pub fn start_position(degrees: f32) -> u64 {
    let turns = f64::from(degrees) / 360.0;
    if turns > 0.0 && turns < 1.0 {
        position_of(turns)
    } else {
        0
    }
}

/// The sign-free parts of a positive finite `f64`: `value = mantissa · 2^exponent`.
fn parts_f64(x: f64) -> (u128, i32) {
    let bits = x.to_bits();
    let exponent = ((bits >> 52) & 0x7ff) as i32;
    let fraction = u128::from(bits & ((1 << 52) - 1));
    if exponent == 0 {
        (fraction, -1074)
    } else {
        (fraction | (1 << 52), exponent - 1075)
    }
}

/// The same for an `f32`.
fn parts_f32(x: f32) -> (u128, i32) {
    let bits = x.to_bits();
    let exponent = ((bits >> 23) & 0xff) as i32;
    let fraction = u128::from(bits & ((1 << 23) - 1));
    if exponent == 0 {
        (fraction, -149)
    } else {
        (fraction | (1 << 23), exponent - 150)
    }
}

/// `ceil(num · 2^(64 + num_exp − den_exp) / den)`, saturating at `u64::MAX`; 0 when `num` is 0. `den` is below 2⁶⁴.
fn ceil_scaled(num: u128, num_exp: i32, den: u128, den_exp: i32) -> u64 {
    if num == 0 || den == 0 {
        return 0;
    }
    let shift = 64 + num_exp - den_exp;
    let (n, d) = if shift >= 0 {
        let shift = shift as u32;
        if shift > num.leading_zeros() {
            // The numerator needs 2¹²⁸ or more and `den` is below 2⁶⁴: the quotient exceeds a `u64`.
            return u64::MAX;
        }
        (num << shift, den)
    } else {
        let shift = shift.unsigned_abs();
        if shift > den.leading_zeros() {
            // The denominator is 2¹²⁸ or more, the numerator below: a positive quotient under 1 rounds up to 1.
            return 1;
        }
        (num, den << shift)
    };
    let q = n / d + u128::from(!n.is_multiple_of(d));
    u64::try_from(q).unwrap_or(u64::MAX)
}

/// The position increment of one control step of `step` frames when the shape's length is `num / den` beats at `bpm`
/// beats per minute (a quarter note is `(1, 1)`, a dotted eighth `(3, 4)`): `ceil(2⁶⁴ · bpm · den · step / (60 · num ·
/// fs))`, **exact** (an integer division in `u128` of the exact binary values of `bpm` and `fs`), rounded up. A tempo
/// or sample rate that is not finite and positive stops the shape (0), as does a zero-beat length.
///
/// Rounding up is what makes a length that is a whole number of steps end on exactly that step (`k · inc ≥ 2⁶⁴` at the
/// `k`-th step) and no step before: the excess is under one count per step, `5e-17` of the length.
#[must_use]
pub fn synced_increment(bpm: f64, cycle_beats: (u32, u32), sample_rate: f32, step: usize) -> u64 {
    let (num, den) = cycle_beats;
    if num == 0
        || step == 0
        || !(bpm.is_finite() && bpm > 0.0)
        || !(sample_rate.is_finite() && sample_rate > 0.0)
    {
        return 0;
    }
    let (bm, be) = parts_f64(bpm);
    let (fm, fe) = parts_f32(sample_rate);
    ceil_scaled(
        bm * u128::from(den) * step as u128,
        be,
        60 * u128::from(num) * fm,
        fe,
    )
}

/// The position increment of one control step of `step` frames when the shape's length is `length_ms` milliseconds:
/// `ceil(2⁶⁴ · 1000 · step / (length_ms · fs))`, exact and rounded up as [`synced_increment`]. A length or sample rate
/// that is not finite and positive stops the shape (0).
#[must_use]
pub fn length_increment(length_ms: f32, sample_rate: f32, step: usize) -> u64 {
    if step == 0
        || !(length_ms.is_finite() && length_ms > 0.0)
        || !(sample_rate.is_finite() && sample_rate > 0.0)
    {
        return 0;
    }
    let (lm, le) = parts_f32(length_ms);
    let (fm, fe) = parts_f32(sample_rate);
    ceil_scaled(1000 * step as u128, 0, lm * fm, le + fe)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A triangle 0 → 1 → 0 over the length.
    fn triangle(mode: Mode) -> Shape {
        Shape::new(
            mode,
            &[
                Point {
                    time: 0.0,
                    level: 0.0,
                    curve: 0.0,
                },
                Point {
                    time: 0.5,
                    level: 1.0,
                    curve: 0.0,
                },
                Point {
                    time: 1.0,
                    level: 0.0,
                    curve: 0.0,
                },
            ],
        )
    }

    fn at(shape: &Shape, x: f64) -> f64 {
        shape.value(position_of(x))
    }

    #[test]
    fn a_straight_triangle_is_linear_up_and_down() {
        let s = triangle(Mode::Loop);
        for k in 0..100 {
            let x = f64::from(k) / 100.0;
            let want = if x < 0.5 { 2.0 * x } else { 2.0 - 2.0 * x };
            assert!((at(&s, x) - want).abs() < 1e-12, "x = {x}");
        }
    }

    #[test]
    fn the_curve_is_the_rational_formula_and_symmetric() {
        // Two points, 0 → 1, with a curve c on the segment leaving the first.
        for c in [-0.9f64, -0.5, -0.1, 0.1, 0.5, 0.9] {
            let s = Shape::new(
                Mode::OneShot,
                &[
                    Point {
                        time: 0.0,
                        level: 0.0,
                        curve: c,
                    },
                    Point {
                        time: 1.0,
                        level: 1.0,
                        curve: 0.0,
                    },
                ],
            );
            let k = (1.0 - c) / (1.0 + c);
            for j in 1..20 {
                let u = f64::from(j) / 20.0;
                let want = u / (u + k * (1.0 - u));
                assert!((at(&s, u) - want).abs() < 1e-12, "c = {c}, u = {u}");
                // The curve for −c is this one turned half a revolution.
                let mirror = Shape::new(
                    Mode::OneShot,
                    &[
                        Point {
                            time: 0.0,
                            level: 0.0,
                            curve: -c,
                        },
                        Point {
                            time: 1.0,
                            level: 1.0,
                            curve: 0.0,
                        },
                    ],
                );
                assert!(
                    (at(&mirror, 1.0 - u) - (1.0 - at(&s, u))).abs() < 1e-12,
                    "c = {c}, u = {u}"
                );
            }
        }
    }

    #[test]
    fn plus_and_minus_one_are_the_two_steps() {
        let step = |c: f64| {
            Shape::new(
                Mode::OneShot,
                &[
                    Point {
                        time: 0.25,
                        level: 0.2,
                        curve: c,
                    },
                    Point {
                        time: 0.75,
                        level: 0.9,
                        curve: 0.0,
                    },
                ],
            )
        };
        let (jump, hold) = (step(1.0), step(-1.0));
        assert_eq!(
            at(&jump, 0.1),
            0.2,
            "before the first point it holds its level"
        );
        assert_eq!(
            at(&jump, 0.25),
            0.2,
            "at the point itself it has not jumped"
        );
        assert_eq!(at(&jump, 0.2500001), 0.9, "just after it, the next level");
        assert_eq!(at(&jump, 0.74), 0.9);
        assert_eq!(at(&hold, 0.25), 0.2);
        assert_eq!(at(&hold, 0.74), 0.2, "it holds until the next point");
        assert_eq!(at(&hold, 0.75), 0.9, "and jumps there");
        // Close to a step but not one: 99.9 % is steep, not a jump.
        let near = step(0.999);
        let early = at(&near, 0.2501);
        assert!(early > 0.2 && early < 0.9, "{early}");
        assert!(at(&near, 0.26) > 0.88);
    }

    #[test]
    fn points_never_cross_and_equal_times_make_an_edge() {
        // Point 3 asks for a time before point 2: it sits at point 2's time, a vertical edge up to its level.
        let s = Shape::new(
            Mode::OneShot,
            &[
                Point {
                    time: 0.0,
                    level: 0.0,
                    curve: 0.0,
                },
                Point {
                    time: 0.5,
                    level: 0.0,
                    curve: 0.0,
                },
                Point {
                    time: 0.2,
                    level: 1.0,
                    curve: 0.0,
                },
            ],
        );
        assert_eq!(s.time(2), 0.5);
        assert_eq!(at(&s, 0.4999), 0.0);
        assert_eq!(
            at(&s, 0.5),
            1.0,
            "from the edge on, the later point's level"
        );
        assert_eq!(at(&s, 0.9), 1.0);
    }

    #[test]
    fn a_loop_heads_back_to_the_first_level_and_a_one_shot_holds_the_last() {
        let pts = [
            Point {
                time: 0.0,
                level: 0.2,
                curve: 0.0,
            },
            Point {
                time: 0.5,
                level: 1.0,
                curve: 0.0,
            },
        ];
        let looped = Shape::new(Mode::Loop, &pts);
        let once = Shape::new(Mode::OneShot, &pts);
        // Loop: 0.2 → 1 over the first half, then back to 0.2 over the second.
        assert!((at(&looped, 0.75) - 0.6).abs() < 1e-12);
        assert!(at(&looped, 0.999999) < 0.2001);
        assert_eq!(at(&once, 0.75), 1.0);
        assert_eq!(once.final_level(), 1.0);
        // Before the first point's time the first level is held.
        let late = Shape::new(
            Mode::OneShot,
            &[
                Point {
                    time: 0.4,
                    level: 0.7,
                    curve: 0.0,
                },
                Point {
                    time: 1.0,
                    level: 0.0,
                    curve: 0.0,
                },
            ],
        );
        assert_eq!(at(&late, 0.1), 0.7);
    }

    #[test]
    fn the_output_maps_polarity_and_scales_by_velocity() {
        let s = triangle(Mode::Loop).with_output(true, 0.5);
        assert_eq!(s.output(0.0, 1.0), -1.0);
        assert_eq!(s.output(1.0, 1.0), 1.0);
        assert_eq!(s.output(0.5, 1.0), 0.0);
        // Velocity 0.2 at amount 0.5: 1 − 0.5·0.8 = 0.6.
        assert!((s.output(1.0, 0.2) - 0.6).abs() < 1e-15);
        let flat = triangle(Mode::Loop).with_output(false, 0.0);
        assert_eq!(
            flat.output(0.75, 0.1),
            0.75,
            "an amount of 0 ignores velocity"
        );
    }

    /// The oracle: `ceil(2⁶⁴ · bpm · den · step / (60 · num · fs))` in plain integers (an integer tempo and rate).
    fn integer_oracle(bpm: u128, beats: (u128, u128), fs: u128, step: u128) -> u64 {
        let n = (bpm * beats.1 * step) << 64;
        let d = 60 * beats.0 * fs;
        // More than a whole length per step saturates.
        u64::try_from(n / d + u128::from(!n.is_multiple_of(d))).unwrap_or(u64::MAX)
    }

    #[test]
    fn the_synced_increment_is_the_exact_quotient_rounded_up() {
        let lengths = [
            (1u32, 16u32),
            (1, 12),
            (1, 8),
            (1, 4),
            (3, 4),
            (1, 1),
            (3, 2),
            (2, 1),
            (4, 1),
            (8, 1),
            (16, 1),
            (64, 1),
            (32, 3),
        ];
        for bpm in [20u32, 63, 90, 120, 128, 140, 174, 333, 999] {
            for beats in lengths {
                for fs in [8_000u32, 44_100, 48_000, 96_000, 384_000] {
                    let got = synced_increment(f64::from(bpm), beats, fs as f32, 32);
                    let want = integer_oracle(
                        u128::from(bpm),
                        (u128::from(beats.0), u128::from(beats.1)),
                        u128::from(fs),
                        32,
                    );
                    assert_eq!(got, want, "{bpm} BPM, {beats:?}, {fs} Hz");
                    // Under one length per step for every case but 1/64 at the very fastest tempos.
                    assert!(got < u64::MAX || bpm > 300, "{bpm} BPM, {beats:?}, {fs} Hz");
                }
            }
        }
    }

    #[test]
    fn a_length_of_a_whole_number_of_steps_ends_on_exactly_that_step() {
        // 120 BPM, a quarter note, 48 kHz: 0.5 s = 24,000 frames = 750 steps. The 750th step reaches 2⁶⁴, the 749th
        // does not (rounding to nearest would have ended it on step 751).
        let inc = synced_increment(120.0, (1, 1), 48_000.0, 32);
        assert!(u128::from(inc) * 750 >= 1u128 << 64);
        assert!(u128::from(inc) * 749 < 1u128 << 64);
        let shape = triangle(Mode::OneShot);
        let mut cursor = Cursor::new(0);
        let mut ended_at = 0;
        for step in 1..=800 {
            cursor.advance(&shape, inc, true);
            if cursor.is_ended(&shape) && ended_at == 0 {
                ended_at = step;
            }
        }
        assert_eq!(ended_at, 750);
        // Every whole-step case at once: integer tempo, rate and length with fs·60·num divisible by bpm·den·step.
        for (bpm, beats, fs) in [
            (120u32, (1u32, 1u32), 48_000u32),
            (120, (4, 1), 48_000),
            (150, (1, 1), 48_000),
            (60, (1, 2), 48_000),
            (96, (3, 2), 48_000),
            (240, (2, 1), 96_000),
        ] {
            let steps_num = u128::from(beats.0) * 60 * u128::from(fs);
            let steps_den = u128::from(bpm) * u128::from(beats.1) * 32;
            if steps_num % steps_den != 0 {
                continue;
            }
            let want = steps_num / steps_den;
            let inc = synced_increment(f64::from(bpm), beats, fs as f32, 32);
            let mut cursor = Cursor::new(0);
            let mut steps = 0u128;
            while !cursor.is_ended(&shape) {
                cursor.advance(&shape, inc, true);
                steps += 1;
            }
            assert_eq!(steps, want, "{bpm} BPM, {beats:?}, {fs} Hz");
        }
    }

    #[test]
    fn the_length_increment_is_the_exact_quotient_rounded_up() {
        // 2⁶⁴ · 1000 · 32 / (ms · fs) with ms and fs whole numbers.
        for ms in [10u32, 100, 250, 1_000, 4_000, 17_000, 60_000] {
            for fs in [8_000u32, 44_100, 48_000, 96_000] {
                let n = (1000u128 * 32) << 64;
                let d = u128::from(ms) * u128::from(fs);
                let want = u64::try_from(n / d + u128::from(!n.is_multiple_of(d))).unwrap();
                assert_eq!(
                    length_increment(ms as f32, fs as f32, 32),
                    want,
                    "{ms} ms at {fs}"
                );
            }
        }
        // 60 s at 48 kHz is 90,000 steps exactly.
        let inc = length_increment(60_000.0, 48_000.0, 32);
        assert!(u128::from(inc) * 90_000 >= 1u128 << 64);
        assert!(u128::from(inc) * 89_999 < 1u128 << 64);
    }

    #[test]
    fn nonsense_stops_the_shape_instead_of_panicking() {
        assert_eq!(synced_increment(f64::NAN, (1, 1), 48_000.0, 32), 0);
        assert_eq!(synced_increment(-3.0, (1, 1), 48_000.0, 32), 0);
        assert_eq!(synced_increment(120.0, (0, 1), 48_000.0, 32), 0);
        assert_eq!(synced_increment(120.0, (1, 1), 0.0, 32), 0);
        assert_eq!(synced_increment(120.0, (1, 1), 48_000.0, 0), 0);
        assert_eq!(length_increment(0.0, 48_000.0, 32), 0);
        assert_eq!(length_increment(f32::INFINITY, 48_000.0, 32), 0);
        assert_eq!(length_increment(-1.0, 48_000.0, 32), 0);
        // Absurdly fast saturates instead of overflowing.
        assert_eq!(length_increment(1e-30, 48_000.0, 32), u64::MAX);
    }

    #[test]
    fn a_loop_wraps_and_never_ends() {
        let shape = triangle(Mode::Loop);
        let inc = synced_increment(120.0, (1, 1), 48_000.0, 32);
        let mut cursor = Cursor::new(0);
        for _ in 0..5_000 {
            cursor.advance(&shape, inc, true);
            assert!(!cursor.is_ended(&shape));
        }
        // 5,000 steps is 6 cycles and 500 steps: the position is the fraction.
        let want = (5_000.0 / 750.0f64).fract();
        assert!((share_of(cursor.position()) - want).abs() < 1e-12);
    }

    #[test]
    fn a_one_shot_from_a_start_phase_runs_the_rest_of_the_shape() {
        let shape = triangle(Mode::OneShot);
        let inc = synced_increment(120.0, (1, 1), 48_000.0, 32);
        // Starting at 180° is the half-way point: 375 steps to go.
        let mut cursor = Cursor::new(start_position(180.0));
        let mut steps = 0;
        while !cursor.is_ended(&shape) {
            cursor.advance(&shape, inc, true);
            steps += 1;
        }
        assert_eq!(steps, 375);
        assert_eq!(cursor.level(&shape), 0.0);
        assert_eq!(start_position(0.0), 0);
        assert_eq!(start_position(360.0), 0);
        assert_eq!(start_position(90.0), 1 << 62);
    }

    /// 0 → 1 at 0.25, then 0.5 at 0.5, 1 at 0.75 and back to 0 at 1: a sustain loop over points 2 to 3.
    fn sustain() -> Shape {
        let p = |time, level| Point {
            time,
            level,
            curve: 0.0,
        };
        Shape::new(
            Mode::SustainLoop,
            &[
                p(0.0, 0.0),
                p(0.25, 1.0),
                p(0.5, 0.5),
                p(0.75, 1.0),
                p(1.0, 0.0),
            ],
        )
        .with_loop(2, 4)
    }

    #[test]
    fn a_sustain_loop_loops_while_held_and_plays_on_when_released() {
        let shape = sustain();
        // A quarter of the length per step: 4 steps to the end.
        let inc = 1u64 << 62;
        let mut cursor = Cursor::new(0);
        let mut held = Vec::new();
        for _ in 0..9 {
            cursor.advance(&shape, inc, true);
            held.push(share_of(cursor.position()));
        }
        // 0.25, 0.5, then at 0.75 it reaches the loop end and goes back to 0.25 (the loop is 0.25..0.75).
        assert_eq!(held, [0.25, 0.5, 0.25, 0.5, 0.25, 0.5, 0.25, 0.5, 0.25]);
        // Released at 0.5: 0.75, then 1.0 ends it.
        cursor.advance(&shape, inc, true);
        assert_eq!(share_of(cursor.position()), 0.5);
        cursor.advance(&shape, inc, false);
        assert_eq!(share_of(cursor.position()), 0.75);
        assert!(!cursor.is_ended(&shape));
        cursor.advance(&shape, inc, false);
        assert!(cursor.is_ended(&shape));
        assert_eq!(cursor.level(&shape), 0.0);
        // Once ended it stays: a key coming back does not restart a finished shape.
        cursor.advance(&shape, inc, true);
        assert!(cursor.is_ended(&shape));
    }

    #[test]
    fn equal_loop_points_hold_a_level_until_the_release() {
        let p = |time, level| Point {
            time,
            level,
            curve: 0.0,
        };
        let shape =
            Shape::new(Mode::SustainLoop, &[p(0.0, 0.0), p(0.5, 1.0), p(1.0, 0.0)]).with_loop(2, 2);
        let inc = 1u64 << 61; // an eighth per step
        let mut cursor = Cursor::new(0);
        for _ in 0..4 {
            cursor.advance(&shape, inc, true);
        }
        assert_eq!(share_of(cursor.position()), 0.5);
        assert_eq!(cursor.level(&shape), 1.0);
        for _ in 0..40 {
            cursor.advance(&shape, inc, true);
            assert_eq!(
                share_of(cursor.position()),
                0.5,
                "held at the sustain point"
            );
        }
        // Released: the second half plays out, 4 steps.
        for _ in 0..3 {
            cursor.advance(&shape, inc, false);
            assert!(!cursor.is_ended(&shape));
        }
        cursor.advance(&shape, inc, false);
        assert!(cursor.is_ended(&shape));
    }

    #[test]
    fn a_note_released_before_the_loop_skips_it() {
        let shape = sustain();
        let inc = 1u64 << 60; // a sixteenth per step
        let mut cursor = Cursor::new(0);
        cursor.advance(&shape, inc, true);
        // Released at 1/16, before the loop starts at 0.25: it plays straight through the loop region to the end.
        let mut steps = 1;
        while !cursor.is_ended(&shape) {
            cursor.advance(&shape, inc, false);
            steps += 1;
        }
        assert_eq!(steps, 16);
    }

    #[test]
    fn loop_points_are_clamped_to_the_used_points() {
        let p = |time, level| Point {
            time,
            level,
            curve: 0.0,
        };
        let pts = [p(0.0, 0.0), p(0.5, 1.0), p(1.0, 0.0)];
        // End above the points counts as the last, a start after the end as the end.
        let a = Shape::new(Mode::SustainLoop, &pts).with_loop(2, 16);
        assert_eq!(
            (a.loop_start, a.loop_end),
            (position_of(0.5), position_of(1.0))
        );
        let b = Shape::new(Mode::SustainLoop, &pts).with_loop(3, 2);
        assert_eq!(b.loop_start, b.loop_end);
        assert_eq!(b.loop_end, position_of(0.5));
        let c = Shape::new(Mode::SustainLoop, &pts).with_loop(0, 0);
        assert_eq!((c.loop_start, c.loop_end), (0, 0));
    }
}
