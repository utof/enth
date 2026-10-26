//! The search space: which parameters a hint may move, where they sit on their normalised axis, and in which order
//! to probe them when the render budget cannot cover them all.
//!
//! A parameter is **free** when it is a continuous (`Float`) parameter that is modulatable or automatable, not
//! structural and not periodic, of an enabled module, and not *inert* for this patch and performance. Enum, integer
//! and toggle parameters (waveforms, filter modes, voice mode, polyphony) are never moved in P1: categorical moves
//! are a follow-up (`docs/dsp/hints.md`). Periodic parameters (start phases) wrap around and are timbre-neutral
//! nuisances for the descriptors' noise estimate, so they are left alone as well.
//!
//! Transposition (parameters in semitones, `osc.semi`) stays fixed unless the caller allows it: it duplicates
//! `fine` as a pitch offset, intervals between oscillators are design, and on the normalised axis a cent of `semi`
//! (±48 st) is 48 times cheaper than a cent of `fine` (±100 ct), so a damped least-squares step would prefer
//! fractions of a semitone where an agent expects a detune in cents.
//!
//! **Inert** parameters cannot change this render, so probing them would waste a render. The rules are
//! conservative (when in doubt a parameter stays free):
//!
//! - every parameter of a disabled module;
//! - every parameter of a modulator that is neither the amplitude envelope nor the source of a route with a
//!   non-zero depth;
//! - `osc.pw` unless the wave is `pulse`, and `osc.position` unless it is `table` (only when the performance
//!   automates nothing, since automation could switch the wave);
//! - an `lfo`'s `rate` when `sync` is on (the speed then comes from `beats` and the tempo, and `rate` is ignored) and
//!   its `beats` when `sync` is off (F1m S8; only when the performance automates nothing, since automation could
//!   switch `sync`). `beats` is an enum, so it is never moved in P1 anyway; the rule keeps it out of the space when
//!   categorical moves arrive, and generator v2's mask draws the same rule;
//! - `voice.glide` when the performance has at most one note;
//! - an `adsr`'s `velocity` when every note has full velocity;
//! - an `adsr`'s `release` when no note-off happens inside the clip.

use enth_analysis::descriptors::Feature;
use enth_params::{Kind, ParamSpec, Unit};
use enth_patch::catalogue::{AdsrParam, LfoParam, OscParam, VoiceParam, Wave};
use enth_patch::{Builtin, ModSource, ModuleKind, ParamAddr, Patch, Target};
use enth_render::Performance;

/// One parameter the hint may move.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct FreeParam {
    pub addr: ParamAddr,
    pub spec: &'static ParamSpec,
    /// The patch's plain value.
    pub plain: f32,
    /// Its normalised position, 0–1.
    pub u: f64,
}

/// Whether the solver may ever move a parameter with this spec (before the per-patch inert rules).
pub(crate) fn movable(spec: &ParamSpec, transpose: bool) -> bool {
    matches!(spec.kind, Kind::Float { .. })
        && !spec.flags.structural
        && !spec.flags.periodic
        && (spec.flags.modulatable || spec.flags.automatable)
        && (transpose || spec.unit != Unit::Semitones)
}

/// Whether the `lfo` parameter at index `i` cannot change the render when `sync` is `synced` (F1m S8): `rate` is
/// ignored with sync on, and `beats` is read only with sync on.
pub(crate) fn lfo_param_inert(i: usize, synced: bool) -> bool {
    (i == LfoParam::Rate.index() && synced) || (i == LfoParam::Beats.index() && !synced)
}

/// Every free parameter of `patch` for `perf`, in patch order (modules by section and position, parameters in
/// spec order, then `voice`, then `master`), filtered by `only` and `frozen`; transposition only with `transpose`.
pub(crate) fn free_params(
    patch: &Patch,
    perf: &Performance,
    only: Option<&[ParamAddr]>,
    frozen: &[ParamAddr],
    transpose: bool,
) -> Vec<FreeParam> {
    let single_note = perf.notes.len() <= 1;
    let full_velocity = perf.notes.iter().all(|n| n.velocity >= 1.0);
    let no_note_off = perf.notes.iter().all(|n| n.off_s >= perf.duration_s);
    let automated = !perf.automation.is_empty();
    let mut out = Vec::new();
    let mut push = |addr: ParamAddr, spec: &'static ParamSpec, plain: f32| {
        if !movable(spec, transpose)
            || only.is_some_and(|o| !o.contains(&addr))
            || frozen.contains(&addr)
        {
            return;
        }
        out.push(FreeParam {
            u: f64::from(spec.to_norm(plain)),
            addr,
            spec,
            plain,
        });
    };
    for (_, m) in patch.modules() {
        if !m.enabled {
            continue;
        }
        if matches!(m.kind, ModuleKind::Adsr | ModuleKind::Lfo) {
            let src = ModSource::Module(m.id.clone());
            let drives = m.id == patch.voice.amp_env
                || patch.routes.iter().any(|r| r.src == src && r.depth != 0.0);
            if !drives {
                continue;
            }
        }
        let synced = (m.kind == ModuleKind::Lfo)
            .then(|| m.values.get(LfoParam::Sync.index()).copied())
            .flatten()
            .is_some_and(|v| v >= 0.5);
        let wave = (m.kind == ModuleKind::Osc)
            .then(|| m.values.get(OscParam::Wave.index()).copied())
            .flatten()
            .and_then(Wave::from_value);
        for (i, spec) in m.kind.params().iter().enumerate() {
            let inert = match m.kind {
                ModuleKind::Osc if !automated => {
                    (i == OscParam::Pw.index() && wave != Some(Wave::Pulse))
                        || (i == OscParam::Position.index() && wave != Some(Wave::Table))
                }
                ModuleKind::Lfo if !automated => lfo_param_inert(i, synced),
                ModuleKind::Adsr => {
                    (i == AdsrParam::Velocity.index() && full_velocity)
                        || (i == AdsrParam::Release.index() && no_note_off)
                }
                _ => false,
            };
            if inert {
                continue;
            }
            let Ok(addr) = ParamAddr::module(m.id.clone(), spec.key) else {
                continue;
            };
            push(addr, spec, m.values[i]);
        }
    }
    for (i, spec) in Builtin::Voice.params().iter().enumerate() {
        if i == VoiceParam::Glide.index() && single_note {
            continue;
        }
        if let Ok(addr) = ParamAddr::new(Target::Voice, spec.key) {
            push(addr, spec, patch.voice.values[i]);
        }
    }
    for (i, spec) in Builtin::Master.params().iter().enumerate() {
        if let Ok(addr) = ParamAddr::new(Target::Master, spec.key) {
            push(addr, spec, patch.master[i]);
        }
    }
    out
}

/// The family of a descriptor: features that the same parameters tend to move.
pub(crate) fn family(f: Feature) -> u8 {
    match f {
        Feature::Pitch | Feature::PitchCents | Feature::PitchStability | Feature::TonalPitch => 0,
        Feature::Attack
        | Feature::Decay
        | Feature::Sustain
        | Feature::Release
        | Feature::Duration
        | Feature::Ripple => 1,
        // A brightness sweep is moved by the filter (cutoff, resonance) and the source routed to it.
        Feature::Brightness | Feature::SweepRate | Feature::SweepDepth | Feature::SweepTime => 2,
        Feature::Noisiness => 3,
        Feature::Beating | Feature::BeatDepth => 4,
        Feature::Tremolo | Feature::TremoloDepth => 5,
        Feature::Loudness
        | Feature::Peak
        | Feature::TruePeak
        | Feature::ClippedPct
        | Feature::Clipping => 6,
    }
}

/// The descriptor a parameter mainly moves (`ParamSpec::perceptual`), when it is one of ours.
pub(crate) fn perceptual(spec: &ParamSpec) -> Option<Feature> {
    spec.perceptual.and_then(Feature::parse)
}

/// Probe priority (lower first): 0 when the parameter's own descriptor is in a failing item's family, 1 when a
/// sibling parameter of the same module is (a filter's resonance with its cutoff), 2 when it is in any item's
/// family, 3 otherwise. Ties keep patch order.
pub(crate) fn priority(
    p: &FreeParam,
    siblings: &[FreeParam],
    failing: &[Feature],
    all: &[Feature],
) -> u8 {
    let fam = |f: Feature| family(f);
    let hits = |set: &[Feature], spec: &ParamSpec| {
        perceptual(spec).is_some_and(|f| set.iter().any(|s| fam(*s) == fam(f)))
    };
    if hits(failing, p.spec) {
        return 0;
    }
    let module = p.addr.module_id();
    let sibling_hit = module.is_some()
        && siblings
            .iter()
            .filter(|s| s.addr.module_id() == module)
            .any(|s| hits(failing, s.spec));
    if sibling_hit {
        1
    } else if hits(all, p.spec) {
        2
    } else {
        3
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use enth_patch::{ModuleId, ModuleInstance, Op, Position, Section, apply};

    fn names(v: &[FreeParam]) -> Vec<String> {
        v.iter().map(|p| p.addr.to_string()).collect()
    }

    #[test]
    fn free_params_skip_discrete_structural_periodic_and_inert() {
        let p = Patch::init();
        let perf = Performance::note(60, 0.8, 1.0, 2.0);
        let f = free_params(&p, &perf, None, &[], false);
        assert_eq!(
            names(&f),
            [
                "osc1.fine",
                "osc1.level",
                "env1.attack",
                "env1.decay",
                "env1.sustain",
                "env1.release",
                "env1.velocity",
                "voice.pan",
                "master.level"
            ]
        );
        // Full velocity, no note-off in the clip: velocity and release are inert.
        let held = Performance::note(60, 1.0, 3.0, 2.0);
        let f = free_params(&p, &held, None, &[], false);
        assert!(!names(&f).contains(&"env1.release".to_string()));
        assert!(!names(&f).contains(&"env1.velocity".to_string()));
        // `only` and `frozen`.
        let only: Vec<ParamAddr> = vec!["osc1.fine".parse().unwrap(), "osc1.wave".parse().unwrap()];
        assert_eq!(
            names(&free_params(&p, &perf, Some(&only), &[], false)),
            ["osc1.fine"]
        );
        let semi = free_params(&p, &perf, None, &[], true);
        assert_eq!(names(&semi)[0], "osc1.semi");
        let frozen: Vec<ParamAddr> = vec!["master.level".parse().unwrap()];
        assert!(
            !names(&free_params(&p, &perf, None, &frozen, false))
                .contains(&"master.level".to_string())
        );
    }

    #[test]
    fn unrouted_modulators_and_disabled_modules_are_inert() {
        let mut p = Patch::init();
        let env2 = ModuleInstance::new(ModuleId::new("env2").unwrap(), ModuleKind::Adsr);
        let at = Position {
            section: Section::Modulators,
            index: 1,
        };
        apply(&mut p, &Op::AddModule { at, module: env2 }).unwrap();
        let osc2 = ModuleInstance::new(ModuleId::new("osc2").unwrap(), ModuleKind::Osc);
        let at = Position {
            section: Section::Generators,
            index: 1,
        };
        apply(&mut p, &Op::AddModule { at, module: osc2 }).unwrap();
        apply(
            &mut p,
            &Op::SetEnabled {
                id: ModuleId::new("osc2").unwrap(),
                enabled: false,
            },
        )
        .unwrap();
        let f = names(&free_params(
            &p,
            &Performance::note(60, 0.8, 1.0, 2.0),
            None,
            &[],
            false,
        ));
        assert!(
            f.iter()
                .all(|n| !n.starts_with("env2.") && !n.starts_with("osc2."))
        );
    }

    #[test]
    fn an_lfos_rate_is_inert_with_sync_on_and_its_beats_with_sync_off() {
        // The rule itself, for every parameter of the lfo.
        for (i, spec) in ModuleKind::Lfo.params().iter().enumerate() {
            let rate = spec.key == "rate";
            let beats = spec.key == "beats";
            assert_eq!(lfo_param_inert(i, true), rate, "{} with sync on", spec.key);
            assert_eq!(
                lfo_param_inert(i, false),
                beats,
                "{} with sync off",
                spec.key
            );
        }
        // In the space: a routed lfo offers its rate when free-running and not when synced; `beats` is an enum and
        // never free; and automation (which could flip `sync`) keeps the rate in either case.
        let mut p = Patch::init();
        let lfo = ModuleInstance::new(ModuleId::new("lfo1").unwrap(), ModuleKind::Lfo);
        let at = Position {
            section: Section::Modulators,
            index: 1,
        };
        apply(&mut p, &Op::AddModule { at, module: lfo }).unwrap();
        apply(
            &mut p,
            &Op::InsertRoute {
                index: 0,
                route: enth_patch::Route {
                    src: "lfo1".parse().unwrap(),
                    dst: "osc1.level".parse().unwrap(),
                    depth: 30.0,
                },
            },
        )
        .unwrap();
        let perf = Performance::note(60, 0.8, 1.0, 2.0);
        let free = |p: &Patch, perf: &Performance| names(&free_params(p, perf, None, &[], false));
        assert!(free(&p, &perf).contains(&"lfo1.rate".to_string()));
        assert!(!free(&p, &perf).contains(&"lfo1.beats".to_string()));
        p.modulators[1].values[LfoParam::Sync.index()] = 1.0;
        assert!(
            !free(&p, &perf).contains(&"lfo1.rate".to_string()),
            "sync on"
        );
        assert!(
            free(&p, &perf).iter().all(|n| !n.starts_with("lfo1.")),
            "nothing else of a synced lfo is a float the solver moves"
        );
        let mut automated = perf.clone();
        automated.automation.push(enth_render::Automation {
            time_s: 0.5,
            addr: "master.level".parse().unwrap(),
            value: -6.0,
        });
        assert!(
            free(&p, &automated).contains(&"lfo1.rate".to_string()),
            "automation could switch sync off, so the rate stays"
        );
    }

    #[test]
    fn probe_priority_follows_the_failing_families() {
        let p = Patch::init();
        let f = free_params(&p, &Performance::note(60, 0.8, 1.0, 2.0), None, &[], true);
        let by = |name: &str| f.iter().find(|x| x.addr.to_string() == name).unwrap();
        let failing = [Feature::Duration];
        let all = [Feature::Duration, Feature::Pitch];
        assert_eq!(priority(by("env1.decay"), &f, &failing, &all), 0);
        assert_eq!(priority(by("env1.velocity"), &f, &failing, &all), 1);
        assert_eq!(priority(by("osc1.semi"), &f, &failing, &all), 2);
        assert_eq!(priority(by("voice.pan"), &f, &failing, &all), 3);
    }
}
