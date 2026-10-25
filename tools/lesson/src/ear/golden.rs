//! **Owner: lane ECK.** The determinism checks (docs/ear.md, "The ear generator" and "GR4"): the music generator's
//! golden and `ear-check`'s own digest of every game's items. Until INT4's friend build freezes the generator, a ladder
//! change moves them on purpose and the commit names the games.
//!
//! **1. Ear generator v1's golden, recomputed.** `crates/enth-music/tests/golden/ear-v1.txt` holds, per (game, level), 150
//! specs (a game, level, seed and count) and the first 16 hex digits of the SHA-256 of their questions' canonical bytes,
//! and a `sum` line over the block. `enth-music`'s own test checks it on the six golden targets (Linux x86-64 and
//! aarch64, Windows, macOS, wasm32-wasip1); this reads the file as data and recomputes it from the seeds and counts it
//! holds, with this crate's own hashing, so a generator that moved without the file does not pass `ear-check` either,
//! and it names the games. It also holds the file to **completeness**: a block for every (game, level) of this build,
//! none extra, in order.
//!
//! **2. The items digest.** `golden/items-v1.txt` holds one line per (game, level) of every shipped game, music and
//! parameter: the SHA-256 of 1,000 items (4 seeds of 250), each item being the question **and the clip of every option**
//! (the counterfactual builders, `game.rs`), so a change to a counterfactual moves it too. Integers, patch hashes and
//! float bits only, so it is the same on every platform; it runs here on one, and the music games' share of it is
//! carried to the six targets by (1). A new level or game appends a line; no existing line changes. Regenerate on purpose
//! with `cargo xtask ear-check --update-golden` (refused under CI) and review the diff.

use std::fmt::Write as _;

use enth_challenge::ear::question::Clip as Played;
use enth_dsp::util::rng::{derive, fnv1a64};
use enth_learn::pack::sha256;
use enth_music::ear::{EarError, Game, LATEST_GENERATOR, MAX_COUNT, MAX_SEED, Question};
use enth_patch::canon::exact_hash;

use super::game::{EarGame, Item, Layout, Shipped, shipped};

/// The committed items digest.
pub const ITEMS_GOLDEN: &str = include_str!("golden/items-v1.txt");

/// The music generator's golden, as `enth-music`'s test keeps it.
pub const MUSIC_GOLDEN: &str =
    include_str!("../../../../crates/enth-music/tests/golden/ear-v1.txt");

/// Seeds per (game, level) block of the items digest, and items per seed.
pub const ITEM_SEEDS: u64 = 4;
/// Items per seed.
pub const ITEMS_PER_SEED: u32 = 250;

/// Something that differs from the golden.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Finding {
    /// The block (`g=05 l=01`) or `file`.
    pub cell: String,
    /// What differs.
    pub detail: String,
}

fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        })
}

/// Recompute the music golden from `text`'s own specs with the generator `generate(game, level, seed, item)`.
///
/// # Errors
/// Never; every difference is a [`Finding`].
#[must_use]
pub fn check_music(
    text: &str,
    generate: &dyn Fn(Game, u8, u64, u32) -> Result<Question, EarError>,
) -> Vec<Finding> {
    let mut found = Vec::new();
    let mut blocks: Vec<(u8, u8)> = Vec::new();
    // The block being read: its key, the running sum of its digests, how many specs.
    let mut open: Option<((u8, u8), Vec<u8>)> = None;
    let key = |field: &str, word: &str| -> Option<u8> {
        word.strip_prefix(field).and_then(|v| v.parse().ok())
    };
    for (n, raw) in text.replace("\r\n", "\n").lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let words: Vec<&str> = line.split_whitespace().collect();
        let at = format!("line {}", n + 1);
        match words.first().copied() {
            Some("block") => {
                let (Some(g), Some(l)) = (
                    words.get(1).and_then(|w| key("g=", w)),
                    words.get(2).and_then(|w| key("l=", w)),
                ) else {
                    found.push(Finding {
                        cell: at,
                        detail: format!("a block line that does not read: {line:?}"),
                    });
                    continue;
                };
                if open.is_some() {
                    found.push(Finding {
                        cell: at.clone(),
                        detail: "a block starts before the last one's sum".to_owned(),
                    });
                }
                blocks.push((g, l));
                open = Some(((g, l), Vec::new()));
            }
            Some("sum") => {
                let Some(((g, l), digests)) = open.take() else {
                    found.push(Finding {
                        cell: at,
                        detail: "a sum line outside a block".to_owned(),
                    });
                    continue;
                };
                let want = words.get(3).copied().unwrap_or("");
                let got = hex(&sha256(&digests));
                if want != got {
                    found.push(Finding {
                        cell: format!("g={g:02} l={l:02}"),
                        detail: format!("the block's sum is {got}, the file has {want}"),
                    });
                }
            }
            Some(_) => {
                let Some(((g, l), digests)) = open.as_mut() else {
                    found.push(Finding {
                        cell: at,
                        detail: "a spec line outside a block".to_owned(),
                    });
                    continue;
                };
                let cell = format!("g={g:02} l={l:02}");
                let parsed = (|| {
                    let seed = u64::from_str_radix(words.get(1)?, 16).ok()?;
                    let count: u8 = words.get(2)?.strip_prefix("n=")?.parse().ok()?;
                    let d = words.get(3)?.strip_prefix("d=")?;
                    Some((seed, count, d))
                })();
                let Some((seed, count, want)) = parsed else {
                    found.push(Finding {
                        cell,
                        detail: format!("{at}: a spec line that does not read: {line:?}"),
                    });
                    continue;
                };
                let Some(game) = Game::from_id(*g) else {
                    found.push(Finding {
                        cell,
                        detail: format!("{at}: this build has no game {g}"),
                    });
                    continue;
                };
                if *l == 0
                    || *l > game.max_level()
                    || seed > MAX_SEED
                    || count == 0
                    || count > MAX_COUNT
                {
                    found.push(Finding {
                        cell,
                        detail: format!("{at}: the spec is not one this build can play"),
                    });
                    continue;
                }
                let mut bytes = Vec::new();
                let mut failed = None;
                for item in 0..u32::from(count) {
                    match generate(game, *l, seed, item) {
                        Ok(q) => q.write_canonical(&mut bytes),
                        Err(e) => {
                            failed = Some(e.to_string());
                            break;
                        }
                    }
                }
                if let Some(e) = failed {
                    found.push(Finding {
                        cell,
                        detail: format!("{at}: the generator refuses it: {e}"),
                    });
                    continue;
                }
                let digest = sha256(&bytes);
                digests.extend_from_slice(&digest);
                let got = &hex(&digest)[..16];
                if got != want {
                    found.push(Finding {
                        cell,
                        detail: format!("{at}: spec seed {seed:014x} n={count:02} digests to {got}, the file has {want}"),
                    });
                }
            }
            None => {}
        }
    }
    if let Some(((g, l), _)) = open {
        found.push(Finding {
            cell: format!("g={g:02} l={l:02}"),
            detail: "the last block has no sum line".to_owned(),
        });
    }
    // Completeness: a block for every (game, level) of this build, none extra, games in id order and levels ascending.
    let all: Vec<(u8, u8)> = Game::ALL
        .iter()
        .flat_map(|g| (1..=g.max_level()).map(move |l| (g.id(), l)))
        .collect();
    for pair in &all {
        if !blocks.contains(pair) {
            found.push(Finding {
                cell: format!("g={:02} l={:02}", pair.0, pair.1),
                detail: "this build has the level, the golden has no block for it (regenerate on purpose)".to_owned(),
            });
        }
    }
    for pair in &blocks {
        if !all.contains(pair) {
            found.push(Finding {
                cell: format!("g={:02} l={:02}", pair.0, pair.1),
                detail: "the golden has a block this build does not have".to_owned(),
            });
        }
    }
    if blocks
        .iter()
        .filter(|b| all.contains(b))
        .copied()
        .collect::<Vec<_>>()
        != all
            .iter()
            .filter(|a| blocks.contains(a))
            .copied()
            .collect::<Vec<_>>()
    {
        found.push(Finding {
            cell: "file".to_owned(),
            detail: "the blocks are not in order (games by id, levels ascending)".to_owned(),
        });
    }
    found
}

/// Recompute the music golden against the real generator.
#[must_use]
pub fn check_music_real(text: &str) -> Vec<Finding> {
    check_music(text, &enth_music::ear::question)
}

// ---------------------------------------------------------------------------------------------------------------
// The items digest

/// The seed `k` of the (game, level) block: a stream keyed by the pair, kept to 53 bits.
#[must_use]
pub fn seed_of(game: u8, level: u8, k: u64) -> u64 {
    derive(
        derive(
            0x0000_4543_4B5F_4954,
            fnv1a64(format!("{game}/{level}").as_bytes()),
        ),
        k,
    ) & MAX_SEED
}

fn put_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_be_bytes());
}

fn put_played(out: &mut Vec<u8>, p: &Played) {
    match p {
        Played::Patch { patch, phrase } => {
            out.push(0);
            out.extend_from_slice(&exact_hash(patch).0);
            out.extend_from_slice(&phrase.0.to_be_bytes());
        }
        Played::Stimulus(s) => {
            out.push(1);
            out.push(s.timbre);
            put_u32(out, s.notes.len() as u32);
            for n in &s.notes {
                out.extend_from_slice(&[n.key, n.velocity]);
                put_u32(out, n.start_ms);
                put_u32(out, n.hold_ms);
            }
            put_u32(out, s.length_ms);
            match &s.band {
                None => out.push(0),
                Some(b) => {
                    out.push(1);
                    out.push(b.kind as u8);
                    out.extend_from_slice(&b.freq_hz.to_be_bytes());
                    out.extend_from_slice(&b.gain_cdb.to_be_bytes());
                    out.extend_from_slice(&b.q_milli.to_be_bytes());
                }
            }
        }
    }
}

/// An item's canonical bytes: the question (its key and options) and every option's clip.
#[must_use]
pub fn item_bytes(item: &Item) -> Vec<u8> {
    use enth_challenge::ear::scorers::EarKey;
    let mut out = vec![item.game, item.level];
    put_u32(&mut out, item.index);
    put_u32(&mut out, item.options as u32);
    put_u32(&mut out, item.key as u32);
    out.push(match item.layout {
        Layout::PerOption => 0,
        Layout::Changed => 1,
        Layout::OddOne => 2,
    });
    match &item.ekey {
        EarKey::Choice { index, options } => out.extend_from_slice(&[0, *index, *options]),
        EarKey::Taps(t) => {
            out.push(1);
            put_u32(&mut out, t.onsets_ms.len() as u32);
            for o in &t.onsets_ms {
                put_u32(&mut out, *o);
            }
            out.extend_from_slice(&t.window_ms.to_be_bytes());
        }
        EarKey::Value(_) => out.push(2),
    }
    let skill = item.skill.as_deref().unwrap_or("");
    put_u32(&mut out, skill.len() as u32);
    out.extend_from_slice(skill.as_bytes());
    put_u32(&mut out, item.states.len() as u32);
    for s in &item.states {
        put_played(&mut out, s);
    }
    for c in &item.controls {
        put_u32(&mut out, c.len() as u32);
        out.extend_from_slice(c.as_bytes());
    }
    if let Some(steps) = item.steps {
        out.extend_from_slice(&steps.to_bits().to_be_bytes());
    }
    out
}

/// The digest of the (game, level) block: `ITEM_SEEDS` seeds of `ITEMS_PER_SEED` items.
///
/// # Errors
/// An item the game cannot draw (named).
pub fn items_digest(game: &dyn EarGame, level: u8) -> Result<String, String> {
    let per = ITEMS_PER_SEED.min(game.items_per_seed());
    let mut block = Vec::with_capacity((ITEM_SEEDS as usize) * per as usize * 32);
    for k in 0..ITEM_SEEDS {
        let seed = seed_of(game.id(), level, k);
        for index in 0..per {
            let item = game.item(level, seed, index)?;
            block.extend_from_slice(&sha256(&item_bytes(&item)));
        }
    }
    Ok(hex(&sha256(&block)))
}

/// Every (game, level) of the shipped games, in id order.
#[must_use]
pub fn cells() -> Vec<(Shipped, u8)> {
    shipped()
        .into_iter()
        .flat_map(|g| (1..=g.levels()).map(move |l| (g, l)))
        .collect()
}

/// The items digest file, computed: one line per (game, level).
///
/// # Errors
/// An item a game cannot draw.
pub fn items_file(games: &[(Shipped, u8)]) -> Result<String, String> {
    let mut out = String::from(
        "# enth-lesson-tool ear-check: digests of 1,000 items per (game, level), the question and the clip of every option\n\
         # (tools/lesson/src/ear/golden.rs). g=game l=level; 4 seeds of 250 items; a new level or game appends a line.\n",
    );
    for (g, l) in games {
        let _ = writeln!(
            out,
            "g={:02} l={l:02} sha256={}",
            g.id(),
            items_digest(g, *l)?
        );
    }
    Ok(out)
}

/// The (game, level, digest) lines of an items digest file (lines that do not read are skipped).
#[must_use]
pub fn parse_items(text: &str) -> Vec<(u8, u8, String)> {
    text.lines()
        .filter_map(|line| {
            let mut words = line.split_whitespace();
            let g = words.next()?.strip_prefix("g=")?.parse().ok()?;
            let l = words.next()?.strip_prefix("l=")?.parse().ok()?;
            let d = words.next()?.strip_prefix("sha256=")?;
            Some((g, l, d.to_owned()))
        })
        .collect()
}

/// Compare the committed items digest with `computed` (one line per (game, level)); completeness included.
#[must_use]
pub fn compare_items(committed: &str, computed: &[(u8, u8, String)]) -> Vec<Finding> {
    let mut found = Vec::new();
    let mut seen: Vec<(u8, u8)> = Vec::new();
    for raw in committed.replace("\r\n", "\n").lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let words: Vec<&str> = line.split_whitespace().collect();
        let parsed = (|| {
            let g: u8 = words.first()?.strip_prefix("g=")?.parse().ok()?;
            let l: u8 = words.get(1)?.strip_prefix("l=")?.parse().ok()?;
            let d = words.get(2)?.strip_prefix("sha256=")?;
            Some((g, l, d))
        })();
        let Some((g, l, want)) = parsed else {
            found.push(Finding {
                cell: "file".to_owned(),
                detail: format!("a line that does not read: {line:?}"),
            });
            continue;
        };
        seen.push((g, l));
        match computed.iter().find(|(cg, cl, _)| (*cg, *cl) == (g, l)) {
            Some((_, _, got)) if got == want => {}
            Some((_, _, got)) => found.push(Finding {
                cell: format!("g={g:02} l={l:02}"),
                detail: format!("1,000 items digest to {got}, the file has {want}"),
            }),
            None => found.push(Finding {
                cell: format!("g={g:02} l={l:02}"),
                detail: "the file has a block this build does not have".to_owned(),
            }),
        }
    }
    for (g, l, _) in computed {
        if !seen.contains(&(*g, *l)) {
            found.push(Finding {
                cell: format!("g={g:02} l={l:02}"),
                detail:
                    "this build has the level, the file has no line for it (regenerate on purpose)"
                        .to_owned(),
            });
        }
    }
    found
}

/// The latest music generator, for the report.
#[must_use]
pub fn generator() -> u8 {
    LATEST_GENERATOR
}
