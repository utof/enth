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

// ---------------------------------------------------------------------------------------------------------------
// Patch targets (Challenge v2, lane CV2)
// ---------------------------------------------------------------------------------------------------------------

/// The magic of the patch-target preimage (Challenge v2, lane CV2, R2): the fingerprint of a target that no generator
/// version made, a `Target::Patch` or the patch a `Target::Draw` drew over its start.
pub const MAGIC_PATCH_V1: &[u8] = b"enth-target-patch-v1";

/// One value of a patch-target entry.
enum PatchValue {
    /// `p` (a parameter or a macro knob) or `r` (a route depth): an f32's bits.
    Bits(u8, f32),
    /// `e`: a module's enable flag.
    Flag(bool),
    /// `t`: text (a module's kind key, its place `voice_fx[0]`, the amplitude envelope's id).
    Text(String),
}

/// The fingerprint of a target given as a patch, heard through `phrase` (Challenge v2's `Target::Patch`, and the
/// target a `Target::Draw` draws). No generator decided it, so it hashes what makes the patch sound as it does, by
/// name, under its own magic ([`MAGIC_PATCH_V1`]):
///
/// ```text
/// SHA-256( "enth-target-patch-v1" | phrase u16 | entry count u32 | each entry in byte order of its key:
///          key length u16 | key | tag | value )
/// p  <address> or macro<N>   an f32's bits: every parameter and macro knob NOT at its catalogue default
/// r  <src>-><dst>            an f32's bits: every route with a non-zero depth (depth 0 is no route)
/// e  <id>.enabled            0 or 1: every module
/// t  <id>.kind, <id>.at, voice.amp_env   u16 length and UTF-8: the module's kind key, its place
///                                         (`voice_fx[0]`), the amplitude envelope's id
/// ```
///
/// Parameters at their default are left out, so a catalogue change that adds a parameter (at a default that keeps
/// the sound) keeps every fingerprint, as generator v1's does; a schema migration that keeps the values keeps it too.
/// Labels, metadata and automation slots never change the sound, so they are not in it.
pub fn of_patch(patch: &enth_patch::Patch, phrase: PhraseId) -> TargetFingerprint {
    TargetFingerprint(Sha256::digest(patch_preimage(patch, phrase)).into())
}

/// The bytes [`of_patch`] hashes.
pub fn patch_preimage(patch: &enth_patch::Patch, phrase: PhraseId) -> Vec<u8> {
    use enth_patch::catalogue::{Builtin, MACRO_VALUE};
    let mut entries: Vec<(String, PatchValue)> = Vec::new();
    let mut params = |owner: &str, specs: &[enth_params::ParamSpec], values: &[f32]| {
        for (spec, v) in specs.iter().zip(values) {
            if v.to_bits() != spec.default.to_bits() {
                entries.push((format!("{owner}.{}", spec.key), PatchValue::Bits(b'p', *v)));
            }
        }
    };
    params(
        Builtin::Voice.key(),
        Builtin::Voice.params(),
        &patch.voice.values,
    );
    params(
        Builtin::Master.key(),
        Builtin::Master.params(),
        &patch.master,
    );
    for (_, m) in patch.modules() {
        params(m.id.as_str(), m.spec().params, &m.values);
    }
    for (i, m) in patch.macros.iter().enumerate() {
        if m.value.to_bits() != MACRO_VALUE.default.to_bits() {
            entries.push((format!("macro{}", i + 1), PatchValue::Bits(b'p', m.value)));
        }
    }
    for r in &patch.routes {
        if r.depth != 0.0 {
            entries.push((
                format!("{}->{}", r.src, r.dst),
                PatchValue::Bits(b'r', r.depth),
            ));
        }
    }
    for (at, m) in patch.modules() {
        let id = m.id.as_str();
        entries.push((format!("{id}.enabled"), PatchValue::Flag(m.enabled)));
        entries.push((
            format!("{id}.kind"),
            PatchValue::Text(m.spec().key.to_owned()),
        ));
        entries.push((format!("{id}.at"), PatchValue::Text(at.to_string())));
    }
    entries.push((
        "voice.amp_env".to_owned(),
        PatchValue::Text(patch.voice.amp_env.as_str().to_owned()),
    ));
    entries.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
    let mut out = Vec::with_capacity(MAGIC_PATCH_V1.len() + 6 + 24 * entries.len());
    out.extend_from_slice(MAGIC_PATCH_V1);
    out.extend_from_slice(&phrase.0.to_be_bytes());
    out.extend_from_slice(&(entries.len() as u32).to_be_bytes());
    let text = |out: &mut Vec<u8>, t: &[u8]| {
        // Keys and texts are ids, addresses and kind keys: far below 64 KiB.
        let n = t.len().min(usize::from(u16::MAX));
        out.extend_from_slice(&(n as u16).to_be_bytes());
        out.extend_from_slice(&t[..n]);
    };
    for (key, value) in &entries {
        text(&mut out, key.as_bytes());
        match value {
            PatchValue::Bits(tag, v) => {
                out.push(*tag);
                out.extend_from_slice(&bits(*v).to_be_bytes());
            }
            PatchValue::Flag(on) => {
                out.push(b'e');
                out.push(u8::from(*on));
            }
            PatchValue::Text(t) => {
                out.push(b't');
                text(&mut out, t.as_bytes());
            }
        }
    }
    out
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

    /// The patch-target preimage is a persisted contract too (Challenge v2): its bytes are pinned on `Patch::init()`
    /// (every value at its default: only the modules and the amplitude envelope enter), and its hash with them.
    #[test]
    fn the_patch_preimage_is_pinned() {
        let init = enth_patch::Patch::init();
        let mut want = Vec::new();
        want.extend_from_slice(b"enth-target-patch-v1");
        want.extend_from_slice(&[0, 1, 0, 0, 0, 7]);
        for (key, rest) in [
            ("env1.at", &b"t\0\x0dmodulators[0]"[..]),
            ("env1.enabled", b"e\x01"),
            ("env1.kind", b"t\0\x04adsr"),
            ("osc1.at", b"t\0\x0dgenerators[0]"),
            ("osc1.enabled", b"e\x01"),
            ("osc1.kind", b"t\0\x03osc"),
            ("voice.amp_env", b"t\0\x04env1"),
        ] {
            want.extend_from_slice(&(key.len() as u16).to_be_bytes());
            want.extend_from_slice(key.as_bytes());
            want.extend_from_slice(rest);
        }
        assert_eq!(patch_preimage(&init, PhraseId(1)), want);
        assert_eq!(
            of_patch(&init, PhraseId(1)).to_hex(),
            "fc33c9f63f7340878eb873174ec22699ef331ab647a937e62537369738e7762a",
            "update this literal only together with a new magic"
        );
    }

    /// What enters a patch target's fingerprint: moved values, routes and macros by name; not a depth-0 route,
    /// labels, metadata or slots.
    #[test]
    fn a_patch_fingerprint_holds_what_sounds() {
        use enth_patch::{ModSource, Op, ParamAddr, Route, apply};
        let mut p = enth_patch::Patch::init();
        let init = of_patch(&p, PhraseId(1));
        let semi: ParamAddr = "osc1.semi".parse().unwrap();
        apply(
            &mut p,
            Op::SetParam {
                addr: semi,
                value: 7.0,
            },
        )
        .unwrap();
        let moved = of_patch(&p, PhraseId(1));
        assert_ne!(moved, init, "a moved value");
        assert_ne!(
            of_patch(&p, PhraseId(2)),
            moved,
            "the phrase is part of the target"
        );
        let mut q = p.clone();
        q.routes.push(Route {
            src: ModSource::Velocity,
            dst: "osc1.level".parse().unwrap(),
            depth: 0.0,
        });
        assert_eq!(
            of_patch(&q, PhraseId(1)),
            moved,
            "a depth-0 route is no route"
        );
        q.routes[0].depth = 10.0;
        assert_ne!(of_patch(&q, PhraseId(1)), moved, "a route");
        let mut m = p.clone();
        m.macros[2].value = 0.5;
        assert_ne!(of_patch(&m, PhraseId(1)), moved, "a macro knob");
        let mut off = p.clone();
        off.generators[0].enabled = false;
        assert_ne!(of_patch(&off, PhraseId(1)), moved, "an enable flag");
        let mut labelled = p.clone();
        labelled.generators[0].label = Some("Lead".to_owned());
        labelled.meta.name = "renamed".to_owned();
        assert_eq!(
            of_patch(&labelled, PhraseId(1)),
            moved,
            "labels and meta are not sound"
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
