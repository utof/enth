//! The design: which trials the owner and each friend hear, in which order, with which clips, at which gains. The key.
//!
//! **Plan** ([`plan`], from the seed alone, no audio):
//!
//! - *Main trials:* one per reference. The owner hears every reference. With `friends` kits the references are dealt
//!   to the kits round-robin inside each (stratum, degraded) group, so each kit has the same mix; a kit's reference is
//!   also in the owner's session, so it is judged twice.
//! - *Counterbalancing:* which arm plays first is dealt per reference, alternating inside each (stratum, degraded)
//!   group, so half of every group has arm B first. A friend hears each of their references in the order opposite to
//!   the owner's: judged by two listeners, every reference is heard in both orders.
//! - *Catch trials,* seven per listener: two **identical pairs** (A and B are the same candidate, to measure a bias towards
//!   a position or the second sound) and five **attention** trials (one arm's top candidate against the obviously wrong
//!   one, the Init patch; the wrong one is first in two or three of them, each arm is the real candidate in two or three). Attention items are clean items that have a `wrong`
//!   candidate. A kit's catch trials use references outside that kit when there are enough, so a friend does not hear a
//!   reference's candidate in a catch trial and again in a main trial.
//! - Each listener's trials are shuffled together.
//!
//! **Finish** ([`finish`]): every clip is measured once; each trial's gains come from [`TrialLevels::plan`]; the key
//! records them. The owner's clips are written when the design is made; a kit's when it is exported.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::audio::{ClipLevel, TrialLevels, decode_any, measure, scaled, to_out_rate};
use super::boot::SplitMix64;
use super::set::{Item, Set};
use crate::Res;
use crate::files::write_json;
use crate::wav::{Wav, encode};

/// The key's schema id.
pub const SCHEMA_MANIFEST: &str = "enth.listen.gr3-manifest/1";
/// The owner's answers file schema id.
pub const SCHEMA_ANSWERS: &str = "enth.listen.gr3-answers/1";
/// The default design seed (a directory keeps the seed it was designed with).
pub const DEFAULT_SEED: u64 = 20_261_021;
/// Identical pairs per listener.
pub const N_IDENTICAL: usize = 2;
/// Attention trials per listener.
pub const N_ATTENTION: usize = 5;
/// A listener below this share of attention trials right is excluded.
pub const ATTENTION_MIN: f64 = 0.8;
/// Silence between the reference, A and B.
pub const GAP_S: f32 = 0.8;
/// The owner's audience name.
pub const OWNER: &str = "owner";

/// What kind of trial.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// A real judgement: arm A's candidate against arm B's.
    Main,
    /// The same candidate twice: no right answer; shows a bias towards a position.
    Identical,
    /// One arm's candidate against the obviously wrong one: the right answer is the real candidate.
    Attention,
}

/// Where a clip came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    /// Arm A's top candidate.
    ArmA,
    /// Arm B's top candidate.
    ArmB,
    /// The obviously wrong candidate.
    Wrong,
}

impl Source {
    fn path(self, item: &Item) -> Option<&Path> {
        match self {
            Self::ArmA => Some(&item.a),
            Self::ArmB => Some(&item.b),
            Self::Wrong => item.wrong.as_deref(),
        }
    }
}

/// A trial before any audio is measured.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanTrial {
    /// `owner` or `kit<N>`.
    pub audience: String,
    /// 1-based position in that audience's sequence.
    pub n: usize,
    /// Kind.
    pub kind: Kind,
    /// Index into the set's items.
    pub item: usize,
    /// What plays in position A.
    pub pos_a: Source,
    /// What plays in position B.
    pub pos_b: Source,
}

/// The plan: every trial of every audience.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    /// Trials, audience by audience (the owner first), in play order.
    pub trials: Vec<PlanTrial>,
    /// The items each kit hears (indices), for `kit1`, `kit2`, ...
    pub kits: Vec<Vec<usize>>,
}

fn shuffle<T>(rng: &mut SplitMix64, v: &mut [T]) {
    for i in (1..v.len()).rev() {
        let j = rng.below(i + 1);
        v.swap(i, j);
    }
}

/// The name of kit `i` (0-based).
#[must_use]
pub fn kit_name(i: usize) -> String {
    format!("kit{}", i + 1)
}

/// Plan every trial.
///
/// # Errors
/// The set is too small for the catch trials, or has fewer references than `friends`.
pub fn plan(set: &Set, seed: u64, friends: usize) -> Res<Plan> {
    let n = set.items.len();
    let eligible: Vec<usize> = (0..n)
        .filter(|&i| !set.items[i].degraded && set.items[i].wrong.is_some())
        .collect();
    if eligible.len() < N_ATTENTION {
        return Err(format!(
            "the set needs at least {N_ATTENTION} clean items with a `wrong` candidate for the attention trials; it has {}",
            eligible.len()
        )
        .into());
    }
    if n < N_ATTENTION + N_IDENTICAL {
        return Err(format!(
            "the set has {n} items; the catch trials need at least {}",
            N_ATTENTION + N_IDENTICAL
        )
        .into());
    }
    if friends > n {
        return Err(format!("{friends} kits for {n} references").into());
    }
    let mut rng = SplitMix64::new(seed);
    // groups of (stratum, degraded), each dealt in a shuffled order
    let mut groups: BTreeMap<(&str, bool), Vec<usize>> = BTreeMap::new();
    for (i, it) in set.items.iter().enumerate() {
        groups
            .entry((it.stratum.as_str(), it.degraded))
            .or_default()
            .push(i);
    }
    let mut kits: Vec<Vec<usize>> = vec![Vec::new(); friends];
    let mut owner_b_first = vec![false; n];
    let (mut deal, mut flip) = (0usize, 0usize);
    for members in groups.values_mut() {
        shuffle(&mut rng, members);
        for &i in members.iter() {
            if friends > 0 {
                kits[deal % friends].push(i);
                deal += 1;
            }
            owner_b_first[i] = flip % 2 == 0;
            flip += 1;
        }
    }
    let mut trials = Vec::new();
    let audiences: Vec<(String, Vec<usize>, bool)> =
        std::iter::once((OWNER.to_string(), (0..n).collect(), false))
            .chain(
                kits.iter()
                    .enumerate()
                    .map(|(k, items)| (kit_name(k), items.clone(), true)),
            )
            .collect();
    for (name, items, reversed) in audiences {
        let mut mine: Vec<PlanTrial> = items
            .iter()
            .map(|&i| {
                let b_first = owner_b_first[i] != reversed;
                let (pos_a, pos_b) = if b_first {
                    (Source::ArmB, Source::ArmA)
                } else {
                    (Source::ArmA, Source::ArmB)
                };
                PlanTrial {
                    audience: name.clone(),
                    n: 0,
                    kind: Kind::Main,
                    item: i,
                    pos_a,
                    pos_b,
                }
            })
            .collect();
        // catch trials: references outside the kit when there are enough of them
        let mine_set: BTreeSet<usize> = items.iter().copied().collect();
        let outside = |pool: &[usize], need: usize| -> Vec<usize> {
            let out: Vec<usize> = pool
                .iter()
                .copied()
                .filter(|i| !mine_set.contains(i))
                .collect();
            if reversed && out.len() >= need {
                out
            } else {
                pool.to_vec()
            }
        };
        let mut att_pool = outside(&eligible, N_ATTENTION);
        shuffle(&mut rng, &mut att_pool);
        let att_items: Vec<usize> = att_pool[..N_ATTENTION].to_vec();
        // as balanced as an odd count allows: the arm and the side of the extra one are drawn
        let (extra_arm, extra_side) = (rng.below(2) == 0, rng.below(2) == 0);
        let mut arms: Vec<Source> = (0..N_ATTENTION)
            .map(|k| {
                if (k % 2 == 0) == extra_arm {
                    Source::ArmA
                } else {
                    Source::ArmB
                }
            })
            .collect();
        shuffle(&mut rng, &mut arms);
        let mut wrong_first: Vec<bool> = (0..N_ATTENTION)
            .map(|k| (k % 2 == 0) == extra_side)
            .collect();
        shuffle(&mut rng, &mut wrong_first);
        for k in 0..N_ATTENTION {
            let (pos_a, pos_b) = if wrong_first[k] {
                (Source::Wrong, arms[k])
            } else {
                (arms[k], Source::Wrong)
            };
            mine.push(PlanTrial {
                audience: name.clone(),
                n: 0,
                kind: Kind::Attention,
                item: att_items[k],
                pos_a,
                pos_b,
            });
        }
        let rest: Vec<usize> = (0..n).filter(|i| !att_items.contains(i)).collect();
        let mut id_pool = outside(&rest, N_IDENTICAL);
        shuffle(&mut rng, &mut id_pool);
        let mut id_arms = [Source::ArmA, Source::ArmB];
        shuffle(&mut rng, &mut id_arms);
        for k in 0..N_IDENTICAL {
            mine.push(PlanTrial {
                audience: name.clone(),
                n: 0,
                kind: Kind::Identical,
                item: id_pool[k],
                pos_a: id_arms[k],
                pos_b: id_arms[k],
            });
        }
        shuffle(&mut rng, &mut mine);
        for (k, t) in mine.iter_mut().enumerate() {
            t.n = k + 1;
        }
        trials.extend(mine);
    }
    Ok(Plan { trials, kits })
}

/// One trial in the key.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Trial {
    /// Random id (the owner's answers file refers to it).
    pub id: String,
    /// `owner` or `kit<N>`.
    pub audience: String,
    /// 1-based position in that audience's sequence.
    pub n: usize,
    /// Kind.
    pub kind: Kind,
    /// The reference id.
    pub item: String,
    /// What plays in position A.
    pub pos_a: Source,
    /// What plays in position B.
    pub pos_b: Source,
    /// File stems of the reference, A and B (the owner's under `clips/`, a kit's in its folder).
    pub clips: [String; 3],
    /// Gains and the true-peak result.
    pub levels: TrialLevels,
    /// Integrated loudness of the three source clips, LUFS.
    pub lufs_in: [f64; 3],
    /// Length of the three clips, seconds.
    pub seconds: [f64; 3],
    /// Sample rate of the three source files, Hz (every clip is played at 48 kHz).
    pub rates_in: [u32; 3],
}

impl Trial {
    /// The position of the right answer in an attention trial (`a` or `b`); `None` for other kinds.
    #[must_use]
    pub fn right_position(&self) -> Option<char> {
        (self.kind == Kind::Attention).then(|| {
            if self.pos_a == Source::Wrong {
                'b'
            } else {
                'a'
            }
        })
    }

    /// The source in position `a` or `b`.
    #[must_use]
    pub fn source_at(&self, pos: char) -> Source {
        if pos == 'a' { self.pos_a } else { self.pos_b }
    }
}

/// A reference in the key.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ItemInfo {
    /// Id.
    pub id: String,
    /// Stratum.
    pub stratum: String,
    /// Degraded.
    pub degraded: bool,
    /// Bootstrap cluster.
    pub cluster: String,
}

/// The key: everything the analysis needs and nothing the listener may see.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    /// Schema id.
    pub schema: String,
    /// The design seed.
    pub seed: u64,
    /// The set file the design was made from (absolute), for exporting kits.
    pub set_path: String,
    /// SHA-256 of the set file.
    pub set_sha256: String,
    /// The set's name.
    pub name: String,
    /// Label of arm A.
    pub arm_a: String,
    /// Label of arm B.
    pub arm_b: String,
    /// SHA-256 of every input file, `<item>/<field>`.
    pub inputs: BTreeMap<String, String>,
    /// The references.
    pub items: Vec<ItemInfo>,
    /// Kits designed (0 for an owner-only test).
    pub kits: usize,
    /// Every trial of every audience.
    pub trials: Vec<Trial>,
}

impl Manifest {
    /// The trials of one audience, in play order.
    #[must_use]
    pub fn trials_of(&self, audience: &str) -> Vec<&Trial> {
        self.trials
            .iter()
            .filter(|t| t.audience == audience)
            .collect()
    }

    /// The reference with this id.
    #[must_use]
    pub fn item(&self, id: &str) -> Option<&ItemInfo> {
        self.items.iter().find(|i| i.id == id)
    }

    /// The audiences in order: the owner, then each kit.
    #[must_use]
    pub fn audiences(&self) -> Vec<String> {
        std::iter::once(OWNER.to_string())
            .chain((0..self.kits).map(kit_name))
            .collect()
    }
}

/// The key's path in `dir`.
#[must_use]
pub fn manifest_path(dir: &Path) -> PathBuf {
    dir.join("manifest.json")
}

/// The owner's clips folder.
#[must_use]
pub fn clips_dir(dir: &Path) -> PathBuf {
    dir.join("clips")
}

/// What one input file contributes to a trial: its own rate, its length at 48 kHz and its levels.
#[derive(Clone, Copy, Debug)]
struct FileInfo {
    rate: u32,
    seconds: f64,
    level: ClipLevel,
}

/// Decode a candidate or reference file and bring it to 48 kHz stereo; returns the audio and the file's own rate.
///
/// # Errors
/// The file cannot be read or decoded.
pub fn load_audio(path: &Path) -> Res<(Wav, u32)> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let w = decode_any(&bytes).map_err(|e| format!("{}: {e}", path.display()))?;
    let rate = w.sample_rate;
    Ok((
        to_out_rate(w).map_err(|e| format!("{}: {e}", path.display()))?,
        rate,
    ))
}

/// The path a trial's clip `k` (0 = reference, 1 = A, 2 = B) is read from.
fn clip_source(item: &Item, pos_a: Source, pos_b: Source, k: usize) -> Res<&Path> {
    let src = match k {
        0 => return Ok(item.judged()),
        1 => pos_a,
        _ => pos_b,
    };
    src.path(item)
        .ok_or_else(|| format!("item {:?} has no {src:?} file", item.id).into())
}

/// Measure every file the plan uses (each once), build each trial's gains, and assemble the key. Names are random
/// from the seed.
///
/// # Errors
/// A file that cannot be decoded or is silent, or a trial whose clips differ in sample rate.
pub fn finish(set: &Set, plan: &Plan, seed: u64) -> Res<Manifest> {
    let mut info: HashMap<PathBuf, FileInfo> = HashMap::new();
    let mut rng = SplitMix64::new(seed ^ 0x6e61_6d65_7300_0001);
    let mut used: BTreeSet<String> = BTreeSet::new();
    let mut unique = |prefix: &str| loop {
        let id = format!("{prefix}{:06x}", rng.next_u64() & 0xff_ffff);
        if used.insert(id.clone()) {
            return id;
        }
    };
    let mut trials = Vec::new();
    for t in &plan.trials {
        let item = &set.items[t.item];
        let mut levels = [ClipLevel {
            lufs: 0.0,
            true_peak_dbtp: 0.0,
        }; 3];
        let mut rates = [0u32; 3];
        let mut seconds = [0.0; 3];
        for k in 0..3 {
            let p = clip_source(item, t.pos_a, t.pos_b, k)?;
            let fi = if let Some(fi) = info.get(p) {
                *fi
            } else {
                let (w, rate) = load_audio(p)?;
                let level = measure(&w).map_err(|e| format!("{}: {e}", p.display()))?;
                let fi = FileInfo {
                    rate,
                    seconds: w.seconds(),
                    level,
                };
                info.insert(p.to_path_buf(), fi);
                fi
            };
            levels[k] = fi.level;
            rates[k] = fi.rate;
            seconds[k] = fi.seconds;
        }
        let clips = if t.audience == OWNER {
            [unique("c"), unique("c"), unique("c")]
        } else {
            [1, 2, 3].map(|k| format!("{:02}-{k}-{}", t.n, unique("")))
        };
        trials.push(Trial {
            id: unique("t"),
            audience: t.audience.clone(),
            n: t.n,
            kind: t.kind,
            item: item.id.clone(),
            pos_a: t.pos_a,
            pos_b: t.pos_b,
            clips,
            levels: TrialLevels::plan(levels),
            lufs_in: levels.map(|l| l.lufs),
            seconds,
            rates_in: rates,
        });
    }
    Ok(Manifest {
        schema: SCHEMA_MANIFEST.into(),
        seed,
        set_path: std::fs::canonicalize(&set.path)
            .unwrap_or_else(|_| set.path.clone())
            .display()
            .to_string(),
        set_sha256: set.sha256.clone(),
        name: set.name.clone(),
        arm_a: set.arm_a.clone(),
        arm_b: set.arm_b.clone(),
        inputs: set.inputs.clone(),
        items: set
            .items
            .iter()
            .map(|i| ItemInfo {
                id: i.id.clone(),
                stratum: i.stratum.clone(),
                degraded: i.degraded,
                cluster: i.cluster.clone(),
            })
            .collect(),
        kits: plan.kits.len(),
        trials,
    })
}

/// Write one trial's three clips (reference, A, B) as 24-bit WAVs named `<stem>.wav` in `dest`, at the key's gains.
///
/// # Errors
/// An input that cannot be read, or an output that cannot be written.
pub fn write_trial_clips(set: &Set, t: &Trial, dest: &Path) -> Res<()> {
    let item = set
        .item(&t.item)
        .ok_or_else(|| format!("the set has no item {:?}", t.item))?;
    for k in 0..3 {
        let w = scaled(
            &load_audio(clip_source(item, t.pos_a, t.pos_b, k)?)?.0,
            t.levels.gains_db[k],
        );
        let p = dest.join(format!("{}.wav", t.clips[k]));
        std::fs::write(&p, encode(&w)).map_err(|e| format!("{}: {e}", p.display()))?;
    }
    Ok(())
}

/// Check that the inputs of `set` are the ones the key was designed from.
///
/// # Errors
/// A message naming the first files that changed.
pub fn check_inputs(m: &Manifest, set: &Set) -> Res<()> {
    let changed: Vec<&String> = m
        .inputs
        .keys()
        .chain(set.inputs.keys())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter(|k| m.inputs.get(*k) != set.inputs.get(*k))
        .collect();
    if changed.is_empty() {
        return Ok(());
    }
    let shown: Vec<&str> = changed.iter().take(5).map(|s| s.as_str()).collect();
    Err(format!(
        "{} input(s) differ from the ones this folder was designed from (first: {}); use a new --dir for a new set",
        changed.len(),
        shown.join(", ")
    )
    .into())
}

/// Design into `dir`: plan, measure, write the owner's clips, then the key.
///
/// # Errors
/// See [`plan`], [`finish`] and the writes.
pub fn design(set: &Set, seed: u64, friends: usize, dir: &Path) -> Res<Manifest> {
    let p = plan(set, seed, friends)?;
    let m = finish(set, &p, seed)?;
    let cd = clips_dir(dir);
    std::fs::create_dir_all(&cd)?;
    for t in m.trials_of(OWNER) {
        write_trial_clips(set, t, &cd)?;
    }
    write_json(&manifest_path(dir), &m)?;
    Ok(m)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gr3::dry::demo_set;

    fn set(dir: &str, refs: usize) -> (PathBuf, Set) {
        let d = std::env::temp_dir().join(format!(
            "enth-listen-gr3design-{dir}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&d);
        let p = demo_set(&d, refs).unwrap();
        (d, Set::load(&p).unwrap())
    }

    #[test]
    fn the_plan_has_the_trials_the_protocol_asks_for() {
        let (d, s) = set("plan", 60);
        let p = plan(&s, DEFAULT_SEED, 2).unwrap();
        let of = |a: &str| {
            p.trials
                .iter()
                .filter(|t| t.audience == a)
                .collect::<Vec<_>>()
        };
        let owner = of(OWNER);
        assert_eq!(owner.len(), 67);
        assert_eq!(owner.iter().filter(|t| t.kind == Kind::Main).count(), 60);
        assert_eq!(
            owner.iter().filter(|t| t.kind == Kind::Identical).count(),
            2
        );
        assert_eq!(
            owner.iter().filter(|t| t.kind == Kind::Attention).count(),
            5
        );
        assert_eq!(
            p.kits.iter().map(Vec::len).collect::<Vec<_>>(),
            vec![30, 30]
        );
        for k in ["kit1", "kit2"] {
            assert_eq!(of(k).len(), 37);
        }
        // numbering is 1..=len in play order
        assert!(owner.iter().enumerate().all(|(i, t)| t.n == i + 1));
        // every kit reference is in exactly one kit
        let all: BTreeSet<usize> = p.kits.iter().flatten().copied().collect();
        assert_eq!(all.len(), 60);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn arm_order_is_counterbalanced_per_group_and_across_listeners() {
        let (d, s) = set("order", 60);
        let p = plan(&s, DEFAULT_SEED, 2).unwrap();
        let mains = |a: &str| -> BTreeMap<usize, bool> {
            p.trials
                .iter()
                .filter(|t| t.audience == a && t.kind == Kind::Main)
                .map(|t| (t.item, t.pos_a == Source::ArmB))
                .collect()
        };
        let owner = mains(OWNER);
        // inside each (stratum, degraded) group the owner hears arm B first in half the references, give or take one
        let mut groups: BTreeMap<(String, bool), (usize, usize)> = BTreeMap::new();
        for (&i, &b_first) in &owner {
            let g = groups
                .entry((s.items[i].stratum.clone(), s.items[i].degraded))
                .or_default();
            g.0 += 1;
            g.1 += usize::from(b_first);
        }
        for (g, (n, b_first)) in &groups {
            assert!((2 * b_first).abs_diff(*n) <= 1, "{g:?}: {b_first} of {n}");
        }
        // a friend hears each of their references in the order opposite to the owner's
        for k in ["kit1", "kit2"] {
            for (i, b_first) in mains(k) {
                assert_ne!(owner[&i], b_first, "item {i} in {k}");
            }
        }
        // every main trial has both arms, once each
        assert!(
            p.trials
                .iter()
                .filter(|t| t.kind == Kind::Main)
                .all(|t| t.pos_a != t.pos_b)
        );
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn catch_trials_are_built_as_the_protocol_says() {
        let (d, s) = set("catch", 60);
        let p = plan(&s, DEFAULT_SEED, 2).unwrap();
        for a in [OWNER, "kit1", "kit2"] {
            let mine: Vec<&PlanTrial> = p.trials.iter().filter(|t| t.audience == a).collect();
            let att: Vec<&PlanTrial> = mine
                .iter()
                .copied()
                .filter(|t| t.kind == Kind::Attention)
                .collect();
            assert_eq!(att.len(), 5);
            // clean items with a wrong candidate; the wrong one first in two or three (never one side more than
            // one ahead), and each arm used two or three times
            assert!(
                att.iter()
                    .all(|t| !s.items[t.item].degraded && s.items[t.item].wrong.is_some())
            );
            let first = att.iter().filter(|t| t.pos_a == Source::Wrong).count();
            assert!((2..=3).contains(&first), "{a}: wrong first {first} of 5");
            assert_eq!(
                att.iter().filter(|t| t.pos_b == Source::Wrong).count(),
                5 - first
            );
            let real = |t: &&PlanTrial| {
                if t.pos_a == Source::Wrong {
                    t.pos_b
                } else {
                    t.pos_a
                }
            };
            let arm_a = att.iter().filter(|t| real(t) == Source::ArmA).count();
            assert!((2..=3).contains(&arm_a), "{a}: arm A {arm_a} of 5");
            let ident: Vec<&PlanTrial> = mine
                .iter()
                .copied()
                .filter(|t| t.kind == Kind::Identical)
                .collect();
            assert_eq!(ident.len(), 2);
            assert!(
                ident
                    .iter()
                    .all(|t| t.pos_a == t.pos_b && t.pos_a != Source::Wrong)
            );
            assert_ne!(ident[0].pos_a, ident[1].pos_a);
            // a kit's catch trials use references outside the kit
            if a != OWNER {
                let kit = &p.kits[a[3..].parse::<usize>().unwrap() - 1];
                assert!(
                    mine.iter()
                        .filter(|t| t.kind != Kind::Main)
                        .all(|t| !kit.contains(&t.item)),
                    "{a}"
                );
            }
        }
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn the_plan_depends_on_the_seed_alone_and_the_set_must_be_big_enough() {
        let (d, s) = set("seed", 12);
        let a = plan(&s, 7, 2).unwrap();
        assert_eq!(a, plan(&s, 7, 2).unwrap());
        assert_ne!(a, plan(&s, 8, 2).unwrap());
        assert_eq!(plan(&s, 7, 0).unwrap().kits.len(), 0);
        assert!(plan(&s, 7, 13).is_err());
        std::fs::remove_dir_all(&d).unwrap();
        let (d, s) = set("tiny", 5);
        assert!(plan(&s, 7, 0).is_err());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn the_key_has_gains_names_and_no_audio_leaks() {
        let (d, s) = set("key", 12);
        let out = d.join("out");
        let m = design(&s, DEFAULT_SEED, 2, &out).unwrap();
        assert_eq!(m.trials_of(OWNER).len(), 12 + 7);
        assert_eq!(m.trials.len(), 19 + 2 * (6 + 7));
        // owner clips exist under random names
        for t in m.trials_of(OWNER) {
            for c in &t.clips {
                assert!(clips_dir(&out).join(format!("{c}.wav")).exists());
                assert!(c.starts_with('c') && c.len() == 7);
            }
            // matched to -23 LUFS unless lowered: every clip's loudness after the gain is the same
            let after: Vec<f64> = (0..3)
                .map(|k| t.lufs_in[k] + t.levels.gains_db[k])
                .collect();
            assert!((after[0] - after[1]).abs() < 1e-9 && (after[0] - after[2]).abs() < 1e-9);
            assert!((after[0] + 23.0 + t.levels.lowered_db).abs() < 1e-9);
        }
        // kit files are numbered in play order and named at random
        let k1 = m.trials_of("kit1");
        assert!(k1[0].clips[0].starts_with("01-1-") && k1[0].clips[2].starts_with("01-3-"));
        let names: BTreeSet<&String> = m.trials.iter().flat_map(|t| t.clips.iter()).collect();
        assert_eq!(names.len(), m.trials.len() * 3, "names are unique");
        // the same seed gives the same key
        let m2 = design(&s, DEFAULT_SEED, 2, &d.join("out2")).unwrap();
        assert_eq!(m, m2);
        // changed inputs are caught
        check_inputs(&m, &s).unwrap();
        let mut moved = s.clone();
        moved.inputs.insert("ref-00/a".into(), "0".repeat(64));
        assert!(
            check_inputs(&m, &moved)
                .unwrap_err()
                .to_string()
                .contains("ref-00/a")
        );
        std::fs::remove_dir_all(&d).unwrap();
    }
}
