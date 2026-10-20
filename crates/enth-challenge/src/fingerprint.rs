//! **Owner: lane CT** (the target fingerprint, F1r review M1), with lane RG for what each generator version feeds it.
//! The reference is docs/challenge.md, "The target fingerprint".
//!
//! A [`TargetFingerprint`] says which target a generator made, in a form that never depends on the patch schema.
//! `enth_patch::canon::canonical_hash` cannot: its bytes hold the build's patch `SCHEMA` and every voice, master and
//! module value by position, so the first schema bump or new parameter would change every target hash, refuse every
//! long code (`target_mismatch`), break every round saved mid-play (`target_drift`) and move generator v1's golden.
//!
//! The fingerprint hashes only what the generator decided, by name:
//!
//! ```text
//! SHA-256( "enth-target-v1"                     magic, 14 bytes
//!        | generator                            u16, big-endian
//!        | phrase                               u16, big-endian
//!        | entry count                          u32, big-endian
//!        | each entry, in byte order of its key:
//!            key length                         u16, big-endian
//!            key                                UTF-8
//!            tag                                b'p' a parameter value, b'r' a route depth, b'e' an enable flag
//!            value                              p, r: the f32's bits, big-endian (-0.0 written as 0.0); e: 0 or 1 )
//! ```
//!
//! Keys are text: a parameter's address (`filter1.cutoff`, `voice.glide`, `master.level`), a route's
//! `<source>-><destination>` (`lfo1->filter1.cutoff`, no spaces) and a module's `<id>.enabled`. Each key appears once.
//! A generator version decides its entries (generator v1: every draw it made, at its final value, and `master.level`
//! when its headroom rule lowered it), and **freezes them with its golden**. The encoding is frozen with the magic: a
//! generator that needs another kind of entry uses a new magic.
//!
//! What this buys: a catalogue change that adds a parameter, or a schema bump that renames none of v1's keys, keeps
//! every fingerprint, because no entry holds a position, a default or the schema. What it keeps: a long code's
//! 32-bit prefix ([`TargetPrefix`]) and a saved round's full fingerprint still catch a target that drifted.

use core::fmt;

use sha2::{Digest, Sha256};

use crate::code::TargetPrefix;
use crate::model::PhraseId;

/// The magic of the first preimage format.
pub const MAGIC_V1: &[u8] = b"enth-target-v1";

/// A target's fingerprint: SHA-256 over what its generator decided (see the [module docs](self)). Shown as 64
/// lowercase hex digits.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TargetFingerprint(pub [u8; 32]);

impl TargetFingerprint {
    /// 64 lowercase hex digits.
    pub fn to_hex(&self) -> String {
        let mut out = String::with_capacity(64);
        for b in self.0 {
            out.push(char::from(HEX[usize::from(b >> 4)]));
            out.push(char::from(HEX[usize::from(b & 0xf)]));
        }
        out
    }

    /// Read 64 hex digits (either case); `None` for anything else.
    pub fn from_hex(text: &str) -> Option<TargetFingerprint> {
        let b = text.as_bytes();
        if b.len() != 64 {
            return None;
        }
        let digit = |c: u8| char::from(c).to_digit(16).map(|d| d as u8);
        let mut out = [0u8; 32];
        for (i, pair) in b.chunks(2).enumerate() {
            out[i] = digit(pair[0])? << 4 | digit(pair[1])?;
        }
        Some(TargetFingerprint(out))
    }

    /// The first 32 bits, big-endian: what a long code carries.
    pub fn prefix(&self) -> TargetPrefix {
        TargetPrefix::of(self)
    }
}

const HEX: &[u8; 16] = b"0123456789abcdef";

impl fmt::Display for TargetFingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

impl fmt::Debug for TargetFingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "TargetFingerprint({})", self.to_hex())
    }
}

/// The value of one fingerprint entry.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum EntryValue {
    /// A parameter's plain value (`p`).
    Param(f32),
    /// A route's depth, in percent (`r`).
    Depth(f32),
    /// A module's enable flag (`e`).
    Flag(bool),
}

/// The preimage of a fingerprint in the v1 format: the generator, the phrase and the entries. Build it, then
/// [`finish`](Preimage::finish) it.
#[derive(Clone, Debug, PartialEq)]
pub struct Preimage {
    generator: u16,
    phrase: PhraseId,
    entries: Vec<(String, EntryValue)>,
}

impl Preimage {
    /// An empty preimage for `generator`'s target heard through `phrase`.
    pub fn new(generator: u16, phrase: PhraseId) -> Preimage {
        Preimage {
            generator,
            phrase,
            entries: Vec::new(),
        }
    }

    /// Add an entry. A key added twice keeps its last value (a value drawn again is the same draw).
    pub fn push(&mut self, key: impl Into<String>, value: EntryValue) {
        let key = key.into();
        if let Some(e) = self.entries.iter_mut().find(|(k, _)| *k == key) {
            e.1 = value;
        } else {
            self.entries.push((key, value));
        }
    }

    /// The bytes that are hashed (see the [module docs](self)).
    pub fn bytes(&self) -> Vec<u8> {
        let mut entries: Vec<&(String, EntryValue)> = self.entries.iter().collect();
        entries.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
        let mut out = Vec::with_capacity(MAGIC_V1.len() + 8 + 24 * entries.len());
        out.extend_from_slice(MAGIC_V1);
        out.extend_from_slice(&self.generator.to_be_bytes());
        out.extend_from_slice(&self.phrase.0.to_be_bytes());
        out.extend_from_slice(&(entries.len() as u32).to_be_bytes());
        for (key, value) in entries {
            let k = key.as_bytes();
            // Keys are addresses: far below 64 KiB.
            out.extend_from_slice(&(k.len().min(usize::from(u16::MAX)) as u16).to_be_bytes());
            out.extend_from_slice(&k[..k.len().min(usize::from(u16::MAX))]);
            match *value {
                EntryValue::Param(v) => {
                    out.push(b'p');
                    out.extend_from_slice(&bits(v).to_be_bytes());
                }
                EntryValue::Depth(v) => {
                    out.push(b'r');
                    out.extend_from_slice(&bits(v).to_be_bytes());
                }
                EntryValue::Flag(on) => {
                    out.push(b'e');
                    out.push(u8::from(on));
                }
            }
        }
        out
    }

    /// The fingerprint: SHA-256 of [`bytes`](Preimage::bytes).
    pub fn finish(&self) -> TargetFingerprint {
        TargetFingerprint(Sha256::digest(self.bytes()).into())
    }
}

/// An f32's bits, with -0.0 written as 0.0 (the patch ops store it so too).
fn bits(v: f32) -> u32 {
    if v == 0.0 { 0 } else { v.to_bits() }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The preimage format is a persisted contract: these bytes never change (a change is a new magic). It holds no
    /// patch schema, no position and no default: only names and values.
    #[test]
    fn the_v1_preimage_is_pinned() {
        let mut p = Preimage::new(1, PhraseId(2));
        p.push("osc1.wave", EntryValue::Param(2.0));
        p.push("lfo1->filter1.cutoff", EntryValue::Depth(-15.0));
        p.push("osc2.enabled", EntryValue::Flag(true));
        p.push("filter1.cutoff", EntryValue::Param(-0.0));
        let mut want = Vec::new();
        want.extend_from_slice(b"enth-target-v1");
        want.extend_from_slice(&[0, 1, 0, 2, 0, 0, 0, 4]);
        want.extend_from_slice(&[0, 14]);
        want.extend_from_slice(b"filter1.cutoff");
        want.extend_from_slice(b"p\0\0\0\0");
        want.extend_from_slice(&[0, 20]);
        want.extend_from_slice(b"lfo1->filter1.cutoff");
        want.push(b'r');
        want.extend_from_slice(&(-15.0f32).to_bits().to_be_bytes());
        want.extend_from_slice(&[0, 9]);
        want.extend_from_slice(b"osc1.wave");
        want.push(b'p');
        want.extend_from_slice(&2.0f32.to_bits().to_be_bytes());
        want.extend_from_slice(&[0, 12]);
        want.extend_from_slice(b"osc2.enabled");
        want.extend_from_slice(b"e\x01");
        assert_eq!(p.bytes(), want);
        assert_eq!(
            p.finish().to_hex(),
            "afa957d31294f4233584bc91bf6a92d9e457973f27168258923a34b2c92dfb96",
            "update this literal only together with a new magic"
        );
    }

    #[test]
    fn order_and_repeats_do_not_matter() {
        let mut a = Preimage::new(1, PhraseId(1));
        a.push("b", EntryValue::Param(1.0));
        a.push("a", EntryValue::Flag(false));
        let mut b = Preimage::new(1, PhraseId(1));
        b.push("a", EntryValue::Flag(true));
        b.push("b", EntryValue::Param(1.0));
        b.push("a", EntryValue::Flag(false));
        assert_eq!(a.finish(), b.finish());
        let mut c = b.clone();
        c.push("b", EntryValue::Param(1.000_000_1));
        assert_ne!(c.finish(), b.finish(), "a one-ulp change is another target");
        assert_ne!(
            Preimage::new(1, PhraseId(1)).finish(),
            Preimage::new(1, PhraseId(2)).finish(),
            "the phrase is part of the target"
        );
    }

    #[test]
    fn hex_round_trips() {
        let f = TargetFingerprint([0xA5; 32]);
        assert_eq!(f.to_hex(), "a5".repeat(32));
        assert_eq!(TargetFingerprint::from_hex(&f.to_hex()), Some(f));
        assert_eq!(TargetFingerprint::from_hex(&"A5".repeat(32)), Some(f));
        assert_eq!(TargetFingerprint::from_hex("a5"), None);
        assert_eq!(TargetFingerprint::from_hex(&"g5".repeat(32)), None);
        assert_eq!(f.prefix().to_string(), "a5a5a5a5");
    }
}
