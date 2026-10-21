//! `--dry-run`: a demo set of synthetic clips and synthetic answers, so the whole pipeline (levels, trials, kits,
//! answers, report) runs without a set from lane EVK and without a sound. Nothing here is ever played.
//!
//! The demo set has the shape of the real one: `refs` references (two thirds clean in four strata, one third degraded
//! copies of clean ones that are judged against them), each with candidates for arm A and arm B, and a wrong candidate
//! for the clean ones. The clips are short tones and noise at random levels, a few with a high crest factor so the
//! true-peak rule has something to do; they are written as 16-bit PCM and as 32-bit float, so both readers run.
//!
//! The synthetic listeners prefer arm B with a probability of about 0.66 on clean references and 0.60 on degraded ones,
//! moved by a per-reference effect that all listeners share (so the judgements of one reference agree more than
//! chance, as real ones do); they answer identical pairs at random and attention trials right, except `kit2`, who
//! misses the first two (3 of 5 is under 80 %; one miss would be kept), so the exclusion path shows in the report.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use super::answers::Responses;
use super::boot::SplitMix64;
use super::design::{Kind, Manifest, Source};
use super::set::SCHEMA_SET;
use crate::Res;

const SR: u32 = 48_000;
/// The strata of the demo set.
const STRATA: [&str; 4] = [
    "subtractive",
    "fm-wavetable-digital",
    "acoustic-percussive",
    "owner-picks",
];

fn uniform(rng: &mut SplitMix64) -> f64 {
    (rng.next_u64() >> 11) as f64 / (1u64 << 53) as f64
}

fn normal(rng: &mut SplitMix64) -> f64 {
    let (u1, u2) = (uniform(rng).max(1e-12), uniform(rng));
    libm::sqrt(-2.0 * libm::log(u1)) * libm::cos(2.0 * core::f64::consts::PI * u2)
}

/// A short stereo clip: a decaying tone with `harmonics` partials plus noise; `spiky` adds isolated near-full-scale
/// clicks over a quiet body (a high crest factor, so its true peak is over -1 dBTP once it is level-matched).
fn clip(
    rng: &mut SplitMix64,
    f0: f64,
    harmonics: usize,
    noise: f64,
    level: f64,
    secs: f64,
    spiky: bool,
) -> (Vec<f32>, Vec<f32>) {
    let n = (secs * f64::from(SR)) as usize;
    let mut chans = [Vec::with_capacity(n), Vec::with_capacity(n)];
    for (c, ch) in chans.iter_mut().enumerate() {
        for i in 0..n {
            let t = i as f64 / f64::from(SR);
            let env = libm::exp(-4.0 * t / secs);
            let mut v = 0.0;
            for h in 1..=harmonics {
                v += libm::sin(2.0 * core::f64::consts::PI * f0 * h as f64 * t + c as f64 * 0.3)
                    / h as f64;
            }
            v = v * env + noise * normal(rng) * env;
            if spiky && i % 9_600 == 100 {
                v += 40.0;
            }
            ch.push((v * level) as f32);
        }
    }
    let [l, r] = chans;
    (l, r)
}

fn write_pcm16(path: &Path, l: &[f32], r: &[f32]) -> Res<()> {
    let mut data = Vec::with_capacity(l.len() * 4);
    for (a, b) in l.iter().zip(r) {
        for v in [a, b] {
            data.extend_from_slice(
                &((f64::from(*v).clamp(-1.0, 1.0) * 32_767.0).round() as i16).to_le_bytes(),
            );
        }
    }
    write_riff(path, 1, 16, &data)
}

fn write_f32(path: &Path, l: &[f32], r: &[f32]) -> Res<()> {
    let mut data = Vec::with_capacity(l.len() * 8);
    for (a, b) in l.iter().zip(r) {
        data.extend_from_slice(&a.to_le_bytes());
        data.extend_from_slice(&b.to_le_bytes());
    }
    write_riff(path, 3, 32, &data)
}

fn write_riff(path: &Path, tag: u16, bits: u16, data: &[u8]) -> Res<()> {
    let mut out = Vec::with_capacity(44 + data.len());
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&tag.to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&SR.to_le_bytes());
    out.extend_from_slice(&(SR * 2 * u32::from(bits / 8)).to_le_bytes());
    out.extend_from_slice(&(2 * bits / 8).to_le_bytes());
    out.extend_from_slice(&bits.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(data);
    std::fs::write(path, out).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(())
}

/// Write a demo set of `refs` references (at least 6) into `dir` and return the path of its `set.toml`.
///
/// # Errors
/// The folder cannot be written.
pub fn demo_set(dir: &Path, refs: usize) -> Res<PathBuf> {
    let audio = dir.join("audio");
    std::fs::create_dir_all(&audio)?;
    let degraded = refs / 3;
    let clean = refs - degraded;
    let mut rng = SplitMix64::new(0x6465_6d6f_0000_0001);
    let mut toml = format!(
        "schema = \"{SCHEMA_SET}\"\nname = \"gr3 demo set (synthetic clips; nothing here is real)\"\narm_a = \"demo arm A\"\narm_b = \"demo arm B\"\n"
    );
    for i in 0..refs {
        let source = if i < clean { i } else { i - clean };
        let stratum = STRATA[source * STRATA.len() / clean.max(1)];
        let id = if i < clean {
            format!("ref-{i:02}")
        } else {
            format!("deg-{:02}", i - clean)
        };
        let f0 = 110.0 * libm::pow(2.0, (source % 24) as f64 / 12.0);
        let level = 0.02 + 0.2 * uniform(&mut rng);
        if i < clean {
            let (l, r) = clip(&mut rng, f0, 5, 0.01, level, 0.4, false);
            write_pcm16(&audio.join(format!("{id}.wav")), &l, &r)?;
        } else {
            let (l, r) = clip(&mut rng, f0, 5, 0.05, level, 0.4, false);
            write_pcm16(&audio.join(format!("{id}-upload.wav")), &l, &r)?;
        }
        // the candidates: arm B nearer the reference's timbre than arm A, both at their own levels
        let spiky = i % 17 == 3;
        let (a_level, b_level) = (
            0.02 + 0.3 * uniform(&mut rng),
            0.02 + 0.3 * uniform(&mut rng),
        );
        let (al, ar) = clip(&mut rng, f0 * 1.02, 3, 0.02, a_level, 0.4, false);
        let (bl, br) = clip(&mut rng, f0 * 1.005, 5, 0.01, b_level, 0.4, spiky);
        write_f32(&audio.join(format!("{id}-a.wav")), &al, &ar)?;
        write_f32(&audio.join(format!("{id}-b.wav")), &bl, &br)?;
        let _ = write!(
            toml,
            "\n[[item]]\nid = \"{id}\"\nstratum = \"{stratum}\"\ndegraded = {}\n",
            i >= clean
        );
        if i < clean {
            let _ = writeln!(toml, "reference = \"audio/{id}.wav\"");
            let (wl, wr) = clip(&mut rng, 220.0, 1, 0.0, 0.1, 0.4, false);
            write_f32(&audio.join(format!("{id}-init.wav")), &wl, &wr)?;
            let _ = writeln!(toml, "wrong = \"audio/{id}-init.wav\"");
        } else {
            let _ = write!(
                toml,
                "reference = \"audio/{id}-upload.wav\"\nclean_reference = \"audio/ref-{source:02}.wav\"\n"
            );
        }
        let _ = write!(toml, "a = \"audio/{id}-a.wav\"\nb = \"audio/{id}-b.wav\"\n");
    }
    let p = dir.join("set.toml");
    std::fs::write(&p, toml)?;
    Ok(p)
}

/// FNV-1a over a string (a stable per-reference seed).
fn fnv(s: &str) -> u64 {
    s.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

/// A synthetic listener's choices for one audience: trial number to `a` or `b`.
#[must_use]
pub fn synthetic_choices(m: &Manifest, audience: &str, seed: u64) -> BTreeMap<usize, char> {
    let mut rng = SplitMix64::new(seed ^ fnv(audience));
    let mut misses = if audience == "kit2" { 2 } else { 0 };
    let mut out = BTreeMap::new();
    for t in m.trials_of(audience) {
        let pos_of = |s: Source| if t.pos_a == s { 'a' } else { 'b' };
        let c = match t.kind {
            Kind::Main => {
                let item = m.item(&t.item).expect("the key lists its trials' items");
                let effect = 0.7 * normal(&mut SplitMix64::new(fnv(&item.cluster) ^ seed));
                let base = if item.degraded { 0.60f64 } else { 0.66 };
                let logit = libm::log(base / (1.0 - base)) + effect;
                let p_b = 1.0 / (1.0 + libm::exp(-logit));
                if uniform(&mut rng) < p_b {
                    pos_of(Source::ArmB)
                } else {
                    pos_of(Source::ArmA)
                }
            }
            Kind::Identical => {
                if uniform(&mut rng) < 0.5 {
                    'a'
                } else {
                    'b'
                }
            }
            Kind::Attention => {
                let right = t
                    .right_position()
                    .expect("attention trials have a right answer");
                if misses > 0 {
                    misses -= 1;
                    if right == 'a' { 'b' } else { 'a' }
                } else {
                    right
                }
            }
        };
        out.insert(t.n, c);
    }
    out
}

/// Responses of the synthetic kit listener.
#[must_use]
pub fn synthetic_responses(m: &Manifest, audience: &str, listener: &str, seed: u64) -> Responses {
    Responses {
        listener: listener.into(),
        audience: audience.into(),
        synthetic: true,
        choices: synthetic_choices(m, audience, seed),
    }
}
