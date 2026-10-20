//! The plan: every pair and trial, drawn from the design seed alone (no audio), so it is the same on every machine
//! and under every engine version. Its hash ([`Plan::hash`]) is pinned by a test.
//!
//! - **Slots.** `n` test pairs are spread over the distance bins by [`super::BIN_WEIGHTS`] (largest remainder), in
//!   bin order. Slot `k` gets level `k mod 5 + 1` and move `[all, one, pitch][k mod 3]`, so every bin holds a mix of
//!   levels and moves, and every level a mix of bins.
//! - **Targets.** Slot `k`'s target is the level's preset (`SpecPreset` 1–5: musical 100, template 0, generator 1)
//!   with a 40-bit seed from the plan's generator, so it has a short code.
//! - **Moves.** `one` picks one continuous parameter the generator randomized for this target, other than a pitch
//!   (else any observable continuous one); `pitch` moves `osc1.semi`. A move with nothing to move falls back to
//!   `all`. Each pair has its own sign seed for the directions.
//! - **Checks.** Identical pairs at levels 2 and 4; obviously different pairs of a level-1 and a level-5 target whose
//!   osc 1 waves differ and whose osc 1 pitches are at least an octave apart (drawn again until they are).
//! - **Trials.** Each session plays every pair once, in a shuffled order, with the target first in half of the pairs
//!   and X a copy of A in half of the trials (both balanced per session). Trial ids and clip ids are random.

use std::collections::HashSet;

use enth_challenge::calibrate;
use enth_challenge::random;
use enth_challenge::{RandomSpec, SpecPreset};
use enth_patch::ParamAddr;
use serde::{Deserialize, Serialize};

use super::{BIN_WEIGHTS, MoveKind, N_DIFFERENT, N_SAME, PairRole, SESSIONS, Side};

/// The generator version every target uses (frozen v1).
pub const GENERATOR: u16 = 1;
/// The largest seed a short code holds.
pub const SEED_MASK: u64 = (1 << 40) - 1;
/// The parameter the `pitch` move changes.
pub const PITCH_PARAM: &str = "osc1.semi";

/// `SplitMix64`: a small, fixed generator for the plan (its output must never change, or the pinned hash moves).
#[derive(Clone, Debug)]
pub struct Rng(u64);

impl Rng {
    /// A generator from `seed`.
    #[must_use]
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    /// The next 64 random bits.
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        mix(self.0)
    }

    /// Uniform in `[0, 1)`.
    pub fn uniform(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Uniform in `0..n` (`n > 0`).
    pub fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }

    /// Fisher–Yates.
    pub fn shuffle<T>(&mut self, v: &mut [T]) {
        for i in (1..v.len()).rev() {
            let j = self.below(i + 1);
            v.swap(i, j);
        }
    }
}

/// `SplitMix64`'s finaliser.
#[must_use]
pub fn mix(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// FNV-1a, 64 bits.
#[must_use]
pub fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// One pair of the plan.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlannedPair {
    /// 0-based; test pairs first, then the checks.
    pub index: usize,
    /// Test, same or different.
    pub role: PairRole,
    /// The target's level (1–5).
    pub level: u8,
    /// The target's spec.
    pub target: RandomSpec,
    /// The second sound's spec (obviously different pairs).
    pub other: Option<RandomSpec>,
    /// How the answer is made (test pairs).
    pub kind: Option<MoveKind>,
    /// The parameter a `one` or `pitch` move changes.
    pub param: Option<String>,
    /// Keys the move's directions.
    pub sign_seed: u64,
    /// The distance bin the design aims at (test pairs).
    pub planned_bin: Option<usize>,
    /// The target's clip id.
    pub target_clip: String,
    /// The answer's clip id (the target's for an identical pair).
    pub answer_clip: String,
}

/// One trial of the plan.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlannedTrial {
    /// 1-based session.
    pub session: u32,
    /// 1-based position in the session.
    pub n: usize,
    /// Random id (the results files refer to it).
    pub id: String,
    /// The pair.
    pub pair: usize,
    /// The target is A (else B).
    pub target_first: bool,
    /// Which of A and B X is a copy of.
    pub x_is: Side,
}

/// The whole plan.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Plan {
    /// The design seed.
    pub seed: u64,
    /// Every pair: the test pairs, then the identical and the obviously different ones.
    pub pairs: Vec<PlannedPair>,
    /// Every trial of every session.
    pub trials: Vec<PlannedTrial>,
}

impl Plan {
    /// FNV-1a 64 of the plan's compact JSON, as 16 hex digits: what a test pins.
    #[must_use]
    pub fn hash(&self) -> String {
        let text = serde_json::to_string(self).unwrap_or_default();
        format!("{:016x}", fnv1a64(text.as_bytes()))
    }
}

/// Test pairs per bin for `n` pairs: [`BIN_WEIGHTS`] scaled to `n` by largest remainder (ties to the lower bin).
#[must_use]
pub fn bin_counts(n: usize) -> Vec<usize> {
    let total: usize = BIN_WEIGHTS.iter().sum();
    let mut counts: Vec<usize> = BIN_WEIGHTS.iter().map(|w| n * w / total).collect();
    let mut rest: Vec<(usize, usize)> = BIN_WEIGHTS
        .iter()
        .enumerate()
        .map(|(b, w)| (n * w % total, b))
        .collect();
    rest.sort_by(|x, y| y.0.cmp(&x.0).then(x.1.cmp(&y.1)));
    let short = n - counts.iter().sum::<usize>();
    for (_, b) in rest.iter().take(short) {
        counts[*b] += 1;
    }
    counts
}

fn spec(level: u8, seed: u64) -> Result<RandomSpec, String> {
    let preset = SpecPreset::by_id(level).ok_or_else(|| format!("no level {level}"))?;
    Ok(preset.spec(GENERATOR, seed & SEED_MASK))
}

fn is_pitch(addr: &ParamAddr) -> bool {
    let s = addr.to_string();
    matches!(s.rsplit('.').next(), Some("semi" | "fine"))
}

/// The parameter a `one` or `pitch` move changes on `target`, or `None` when it has none (the move becomes `all`).
fn pick_param(
    kind: MoveKind,
    target: &RandomSpec,
    rng: &mut Rng,
) -> Result<Option<String>, String> {
    let g = random::target(target).map_err(|e| format!("generator: {e}"))?;
    let (movable, _) = calibrate::jittered_params(&g.patch, g.phrase());
    Ok(match kind {
        MoveKind::All => None,
        MoveKind::Pitch => movable
            .iter()
            .find(|a| a.to_string() == PITCH_PARAM)
            .map(ToString::to_string),
        MoveKind::One => {
            let randomized: Vec<&ParamAddr> = movable
                .iter()
                .filter(|a| !is_pitch(a) && g.params.contains(a))
                .collect();
            let pool: Vec<&ParamAddr> = if randomized.is_empty() {
                movable.iter().filter(|a| !is_pitch(a)).collect()
            } else {
                randomized
            };
            (!pool.is_empty()).then(|| pool[rng.below(pool.len())].to_string())
        }
    })
}

/// osc 1's wave index and semitones of a generated target (for the obviously different pairs).
fn osc1_of(spec: &RandomSpec) -> Result<(f32, f32), String> {
    let g = random::target(spec).map_err(|e| format!("generator: {e}"))?;
    let read = |a: &str| {
        a.parse::<ParamAddr>()
            .ok()
            .and_then(|addr| g.patch.value(&addr))
            .unwrap_or(0.0)
    };
    Ok((read("osc1.wave"), read("osc1.semi")))
}

struct Ids {
    used: HashSet<String>,
}

impl Ids {
    fn fresh(&mut self, rng: &mut Rng, prefix: char) -> String {
        loop {
            let id = format!("{prefix}{:06x}", rng.next_u64() & 0xff_ffff);
            if self.used.insert(id.clone()) {
                return id;
            }
        }
    }
}

/// A balanced, shuffled list of `n` booleans (`n.div_ceil(2)` true).
fn balanced(rng: &mut Rng, n: usize) -> Vec<bool> {
    let mut v: Vec<bool> = (0..n).map(|i| i < n.div_ceil(2)).collect();
    rng.shuffle(&mut v);
    v
}

/// Make the plan for `n_pairs` test pairs from `seed` (see the [module docs](self)).
///
/// # Errors
/// `n_pairs` is 0, or the generator refuses a spec (it never does for the presets).
pub fn make_plan(seed: u64, n_pairs: usize) -> Result<Plan, String> {
    if n_pairs == 0 {
        return Err("--pairs must be at least 1".into());
    }
    let mut rng = Rng::new(mix(seed ^ 0x7061_6972_7331)); // "pairs1"
    let mut ids = Ids {
        used: HashSet::new(),
    };
    let kinds = [MoveKind::All, MoveKind::One, MoveKind::Pitch];
    let mut pairs = Vec::new();
    let mut k = 0usize;
    for (bin, count) in bin_counts(n_pairs).into_iter().enumerate() {
        for _ in 0..count {
            let level = (k % 5) as u8 + 1;
            let target = spec(level, rng.next_u64())?;
            let planned = kinds[k % 3];
            let param = pick_param(planned, &target, &mut rng)?;
            let kind = if planned != MoveKind::All && param.is_none() {
                MoveKind::All
            } else {
                planned
            };
            pairs.push(PlannedPair {
                index: pairs.len(),
                role: PairRole::Test,
                level,
                target,
                other: None,
                kind: Some(kind),
                param,
                sign_seed: rng.next_u64(),
                planned_bin: Some(bin),
                target_clip: ids.fresh(&mut rng, 'c'),
                answer_clip: ids.fresh(&mut rng, 'c'),
            });
            k += 1;
        }
    }
    for i in 0..N_SAME {
        let level = if i % 2 == 0 { 2 } else { 4 };
        let target = spec(level, rng.next_u64())?;
        let clip = ids.fresh(&mut rng, 'c');
        pairs.push(PlannedPair {
            index: pairs.len(),
            role: PairRole::Same,
            level,
            target,
            other: None,
            kind: None,
            param: None,
            sign_seed: 0,
            planned_bin: None,
            target_clip: clip.clone(),
            answer_clip: clip,
        });
    }
    for _ in 0..N_DIFFERENT {
        let (target, other) = loop {
            let a = spec(1, rng.next_u64())?;
            let b = spec(5, rng.next_u64())?;
            let ((wa, sa), (wb, sb)) = (osc1_of(&a)?, osc1_of(&b)?);
            if wa != wb && (sa - sb).abs() >= 12.0 {
                break (a, b);
            }
        };
        pairs.push(PlannedPair {
            index: pairs.len(),
            role: PairRole::Different,
            level: 1,
            target,
            other: Some(other),
            kind: None,
            param: None,
            sign_seed: 0,
            planned_bin: None,
            target_clip: ids.fresh(&mut rng, 'c'),
            answer_clip: ids.fresh(&mut rng, 'c'),
        });
    }
    let mut trials = Vec::new();
    for session in 1..=SESSIONS {
        let mut order: Vec<usize> = (0..pairs.len()).collect();
        rng.shuffle(&mut order);
        let first = balanced(&mut rng, pairs.len());
        let x_a = balanced(&mut rng, pairs.len());
        for (n, pair) in order.into_iter().enumerate() {
            trials.push(PlannedTrial {
                session,
                n: n + 1,
                id: ids.fresh(&mut rng, 't'),
                pair,
                target_first: first[n],
                x_is: if x_a[n] { Side::A } else { Side::B },
            });
        }
    }
    Ok(Plan {
        seed,
        pairs,
        trials,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bin_counts_scale_the_weights() {
        assert_eq!(bin_counts(40), BIN_WEIGHTS.to_vec());
        assert_eq!(bin_counts(6), vec![1, 1, 1, 1, 1, 1]);
        assert_eq!(bin_counts(1).iter().sum::<usize>(), 1);
        for n in 1..120 {
            assert_eq!(bin_counts(n).iter().sum::<usize>(), n, "{n}");
        }
    }

    #[test]
    fn the_generator_and_hash_are_fixed() {
        let mut r = Rng::new(1);
        assert_eq!(r.next_u64(), 0x910a_2dec_8902_5cc1);
        assert_eq!(fnv1a64(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a64(b"a"), 0xaf63_dc4c_8601_ec8c);
    }
}
