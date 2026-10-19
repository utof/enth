//! The dry sources, rendered by the real engine through `enth-render`, and our reverb as an engine module.
//!
//! The engine's oscillator is still the reference saw and its filter a pass-through (lanes D1 and D2), so every
//! source is saws through an amp envelope, and the `svf` low-pass in each patch does nothing yet: it will, when
//! lane D2 lands (render again in a fresh folder then). What matters here is that all three reverbs hear the same
//! dry signal, which they do whatever the oscillator is.

use enth_patch::Patch;
use enth_patch::file::load;
use enth_render::{Note, Performance, RenderOutput, RenderSpec, render};
use serde_json::{Value, json};

use crate::Res;

/// A dry source: a patch (no effects) and what is played.
#[derive(Clone, Debug)]
pub struct Source {
    /// `pad`, `pluck`, `transient`.
    pub name: &'static str,
    /// For the manifest.
    pub description: &'static str,
    /// The patch as JSON, without `fx`.
    pub patch: Value,
    /// The notes.
    pub notes: Vec<Note>,
    /// When the dry sound has ended (last note-off plus the envelope's release), seconds.
    pub dry_end_s: f64,
}

fn voice_patch(oscs: &[Value], cutoff: &str, env: &Value) -> Value {
    let generators: Vec<Value> = oscs
        .iter()
        .enumerate()
        .map(|(i, p)| json!({ "id": format!("osc{}", i + 1), "kind": "osc", "params": p }))
        .collect();
    json!({
        "schema": 1,
        "engine": "0.1.0",
        "voice": { "amp_env": "env1", "polyphony": 8 },
        "generators": generators,
        "voice_fx": [{ "id": "filter1", "kind": "svf", "params": { "cutoff": cutoff, "resonance": 10 } }],
        "modulators": [{ "id": "env1", "kind": "adsr", "params": env }],
    })
}

/// The three sources, in the order the report lists them. `quick` plays the same notes four times faster (all
/// times a quarter as long), so a debug-build test stays cheap.
#[must_use]
pub fn sources(quick: bool) -> Vec<Source> {
    let k = if quick { 0.25 } else { 1.0 };
    let ms = |v: f64| (v * k).round();
    let n = |key: u8, vel: f32, on: f64, off: f64| Note::new(key, vel, on * k, off * k);
    let pad = Source {
        name: "pad",
        description: "a slow A-minor chord of detuned saws (attack 0.6 s, held 1.6 s, release 0.9 s)",
        patch: voice_patch(
            &[
                json!({ "wave": "saw", "fine": -7 }),
                json!({ "wave": "saw", "fine": 7, "level": 80 }),
            ],
            "1.5kHz",
            &json!({ "attack": ms(600.0), "decay": 0, "sustain": 100, "release": ms(900.0) }),
        ),
        notes: [45u8, 52, 57, 60, 64]
            .iter()
            .map(|key| n(*key, 0.8, 0.0, 1.6))
            .collect(),
        dry_end_s: (1.6 + 0.9) * k,
    };
    let pluck = Source {
        name: "pluck",
        description: "four short plucked notes, 0.5 s apart (decay 0.18 s, sustain 0)",
        patch: voice_patch(
            &[json!({ "wave": "saw", "phase_mode": "reset" })],
            "3.5kHz",
            &json!({ "attack": 0, "decay": ms(180.0), "sustain": 0, "release": ms(60.0) }),
        ),
        notes: [(60u8, 0.0), (67, 0.5), (72, 1.0), (64, 1.5)]
            .iter()
            .map(|(key, t)| n(*key, 0.9, *t, *t + 0.25))
            .collect(),
        dry_end_s: (1.5 + 0.25 + 0.06) * k,
    };
    let transient = Source {
        name: "transient",
        description: "two short percussive hits, a low one and a bright one (35 ms decay)",
        patch: voice_patch(
            &[
                json!({ "wave": "saw", "phase_mode": "reset" }),
                json!({ "wave": "saw", "phase_mode": "reset", "semi": 12, "level": 60 }),
            ],
            "6kHz",
            &json!({ "attack": 0, "decay": ms(35.0), "sustain": 0, "release": ms(10.0) }),
        ),
        notes: vec![n(52, 1.0, 0.0, 0.04), n(76, 0.8, 1.6, 1.64)],
        dry_end_s: (1.6 + 0.04 + 0.01) * k,
    };
    vec![pad, pluck, transient]
}

impl Source {
    /// The first hit of the transient on its own: the impulse the decay calibration excites each reverb with.
    #[must_use]
    pub fn first_hit_only(&self) -> Source {
        let mut s = self.clone();
        s.notes.truncate(1);
        s.dry_end_s = s.notes[0].off_s + 0.01;
        s
    }
}

fn patch_with_fx(source: &Source, fx: Option<Value>) -> Res<Patch> {
    let mut v = source.patch.clone();
    if let Some(fx) = fx {
        v["fx"] = json!([fx]);
    }
    let loaded = load(&v.to_string()).map_err(|e| format!("source {}: {e}", source.name))?;
    Ok(loaded.patch)
}

fn performance(source: &Source, seconds: f64) -> Performance {
    Performance {
        notes: source.notes.clone(),
        duration_s: seconds,
        ..Performance::default()
    }
}

/// The dry signal: the engine's output with no effect at all.
///
/// # Errors
/// A patch or render error (a bug in the source definition).
pub fn render_dry(source: &Source, sample_rate: u32, seed: u64, seconds: f64) -> Res<RenderOutput> {
    let patch = patch_with_fx(source, None)?;
    Ok(render(
        &patch,
        &performance(source, seconds),
        &RenderSpec::new(sample_rate, seed),
    )?)
}

/// Our reverb's contribution alone: the same patch plus the engine's `reverb` module with `dry` off (-60 dB is
/// off), `wet` at `wet_db` and `decay` (RT60) at `decay_ms`; every other parameter at its catalogue default.
///
/// # Errors
/// A patch or render error.
pub fn render_ours_wet(
    source: &Source,
    sample_rate: u32,
    seed: u64,
    seconds: f64,
    wet_db: f64,
    decay_ms: f64,
) -> Res<RenderOutput> {
    let fx = json!({ "id": "reverb1", "kind": "reverb", "params": { "dry": -60, "wet": wet_db, "decay": decay_ms } });
    let patch = patch_with_fx(source, Some(fx))?;
    Ok(render(
        &patch,
        &performance(source, seconds),
        &RenderSpec::new(sample_rate, seed),
    )?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_source_loads_renders_and_makes_sound_with_and_without_the_reverb() {
        for quick in [true, false] {
            for s in sources(quick) {
                let dry = render_dry(&s, 16_000, 7, 2.0).unwrap();
                assert!(dry.peak() > 0.01, "{}: silent dry render", s.name);
                assert_eq!(dry.frames(), 32_000);
                let wet = render_ours_wet(&s, 16_000, 7, 2.0, -6.0, 1500.0).unwrap();
                assert!(wet.peak() > 1e-4, "{}: silent wet render", s.name);
                // the wet render holds no dry signal: it starts silent (the reverb's first reflections come later)
                let early = wet.left[..16]
                    .iter()
                    .chain(&wet.right[..16])
                    .fold(0.0f32, |m, v| m.max(v.abs()));
                assert!(
                    early < 0.05 * wet.peak(),
                    "{}: dry leaking into the wet render",
                    s.name
                );
            }
        }
    }

    #[test]
    fn renders_are_deterministic_and_seeded() {
        let s = &sources(true)[0];
        let a = render_dry(s, 16_000, 7, 1.0).unwrap();
        assert_eq!(a, render_dry(s, 16_000, 7, 1.0).unwrap());
        assert_ne!(
            a.left,
            render_dry(s, 16_000, 8, 1.0).unwrap().left,
            "the pad's free phases follow the seed"
        );
    }

    #[test]
    fn the_first_hit_is_one_note() {
        let t = &sources(false)[2];
        assert_eq!(t.first_hit_only().notes.len(), 1);
        assert!(t.first_hit_only().dry_end_s < 0.1);
    }
}
