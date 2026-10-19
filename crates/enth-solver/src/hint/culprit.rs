//! Which module of the patch is behind a blocked item, so a verdict names it instead of saying only that "the
//! patch structure is the limit" (eval finding 3, `docs/p1a-g1.md`: on `bass_sub`, a sub oscillator an octave
//! below the note made `pitch.note ~ C2` fail, and `blocked` named no module).
//!
//! Like the inert-parameter rules in `space.rs`, this is product knowledge about the catalogue kept in the solver
//! until the catalogue can say it. It never moves anything: the text goes into `Blocked::why`.

use enth_analysis::descriptors::{Descriptors, Feature};
use enth_analysis::expect::Expect;
use enth_analysis::units::{cents, nearest_note};
use enth_patch::Patch;
use enth_patch::addr::ModSource;
use enth_patch::catalogue::ModuleKind;

/// Enabled modules of `kind`, in patch order.
fn enabled(patch: &Patch, kind: ModuleKind) -> Vec<&enth_patch::ModuleInstance> {
    patch
        .modules()
        .filter(|(_, m)| m.kind == kind && m.enabled)
        .map(|(_, m)| m)
        .collect()
}

/// Ids joined for a sentence: `osc1`, `osc1 and osc2`, `osc1, osc2 and osc3`.
fn names(ids: &[String]) -> String {
    match ids {
        [] => String::new(),
        [one] => one.clone(),
        [init @ .., last] => format!("{} and {last}", init.join(", ")),
    }
}

/// A clause naming the module(s) that keep `e` from passing on `patch` (read at the final point `d`), or `None`
/// when no module is to blame (a parameter is at a bound, or the performance is the limit).
pub(super) fn culprit(e: &Expect, patch: &Patch, d: &Descriptors) -> Option<String> {
    match e.feature {
        Feature::Pitch | Feature::PitchCents | Feature::PitchStability | Feature::TonalPitch => {
            pitch(e, patch, d)
        }
        Feature::Beating | Feature::BeatDepth => {
            let osc = enabled(patch, ModuleKind::Osc);
            (osc.len() < 2).then(|| {
                format!(
                    "the patch has {} enabled oscillator{} ({}); beating needs two detuned against each other",
                    osc.len(),
                    if osc.len() == 1 { "" } else { "s" },
                    names(&osc.iter().map(|m| m.id.to_string()).collect::<Vec<_>>())
                )
            })
        }
        Feature::Noisiness => enabled(patch, ModuleKind::Noise)
            .is_empty()
            .then(|| "the patch has no noise module".to_owned()),
        Feature::Brightness => enabled(patch, ModuleKind::Svf)
            .is_empty()
            .then(|| "the patch has no filter (svf) to shape the brightness".to_owned()),
        Feature::Tremolo | Feature::TremoloDepth => {
            let to_level = patch
                .routes
                .iter()
                .any(|r| matches!(r.src, ModSource::Module(_)) && r.dst.param() == "level");
            (!to_level).then(|| "no LFO is routed to a level (osc.level, noise.level)".to_owned())
        }
        Feature::SweepRate | Feature::SweepDepth | Feature::SweepTime => {
            let filters = enabled(patch, ModuleKind::Svf);
            if filters.is_empty() {
                return Some("the patch has no filter (svf) whose cutoff could sweep".to_owned());
            }
            let routed: Vec<String> = patch
                .routes
                .iter()
                .filter(|r| r.dst.param() == "cutoff" && r.depth != 0.0)
                .map(|r| format!("{} -> {}", r.src, r.dst))
                .collect();
            if routed.is_empty() {
                Some(format!(
                    "nothing is routed to the cutoff of {} (route an LFO for a cyclic sweep, an envelope for a one-shot one)",
                    names(&filters.iter().map(|m| m.id.to_string()).collect::<Vec<_>>())
                ))
            } else {
                Some(format!(
                    "the brightness follows {}: its depth or the source's rate is the lever, and the fix does not move route depths",
                    names(&routed)
                ))
            }
        }
        _ => None,
    }
}

fn pitch(e: &Expect, patch: &Patch, d: &Descriptors) -> Option<String> {
    let osc = enabled(patch, ModuleKind::Osc);
    let noise = enabled(patch, ModuleKind::Noise);
    let noise_ids: Vec<String> = noise.iter().map(|m| m.id.to_string()).collect();
    if osc.is_empty() {
        return Some(if noise_ids.is_empty() {
            "the patch has no enabled oscillator".to_owned()
        } else {
            format!(
                "the patch has no enabled oscillator, only {}: noise has no pitch",
                names(&noise_ids)
            )
        });
    }
    let transposed: Vec<(String, f32)> = osc
        .iter()
        .filter_map(|m| {
            m.get("semi")
                .filter(|s| *s != 0.0)
                .map(|s| (m.id.to_string(), s))
        })
        .collect();
    let reading = d.get(e.feature).filter(|v| *v > 0.0);
    match (reading, e.reference_hz) {
        (Some(got), Some(target)) => {
            let off = cents(got, target);
            if off.abs() < 50.0 {
                return None;
            }
            let st = libm::round(off / 100.0);
            let (note, _) = nearest_note(got);
            let side = if off < 0.0 { "below" } else { "above" };
            // The oscillator whose transposition matches how far off the reading is.
            let culprit = transposed
                .iter()
                .find(|(_, s)| (f64::from(*s) - st).abs() < 0.5)
                .or_else(|| transposed.first());
            Some(match culprit {
                Some((id, s)) => format!(
                    "it reads {note}, {} st {side} the note: {id} is transposed {s:+} st ({id}.semi) and sets the \
                     period; the fix never moves semi, so set {id}.semi yourself (0st plays the note) or lower \
                     {id}.level",
                    libm::fabs(st)
                ),
                None => format!(
                    "it reads {note}, {} st {side} the note, and no oscillator is transposed: look for a source an \
                     octave away, or hard sync",
                    libm::fabs(st)
                ),
            })
        }
        (None, _) if !noise_ids.is_empty() && e.feature != Feature::TonalPitch => Some(format!(
            "{} cover{} the tone, so the waveform does not repeat: lower the noise level, or judge tonal_pitch, which \
             reads a tone under noise",
            names(&noise_ids),
            if noise_ids.len() == 1 { "s" } else { "" }
        )),
        (None, _) if !noise_ids.is_empty() => Some(format!(
            "the tone is under 1% of the power next to {}: raise an oscillator's level or lower the noise",
            names(&noise_ids)
        )),
        (None, _) if !transposed.is_empty() => Some(format!(
            "transposed oscillators: {}",
            transposed
                .iter()
                .map(|(id, s)| format!("{id} ({s:+} st)"))
                .collect::<Vec<_>>()
                .join(", ")
        )),
        _ => {
            // Silence the analysis can still see: an oscillator at level 0, or one routed out of range.
            let quiet: Vec<String> = osc
                .iter()
                .filter(|m| m.get("level").is_some_and(|l| l <= 0.0))
                .map(|m| m.id.to_string())
                .collect();
            (!quiet.is_empty()).then(|| format!("{} is at level 0", names(&quiet)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use enth_analysis::expect::parse_item;
    use enth_analysis::pitch::Pitch;
    use enth_patch::wire;

    fn patch(ops: &str) -> Patch {
        let mut p = Patch::init();
        let ops = wire::resolve_str(&p, ops).unwrap();
        enth_patch::apply_all(&mut p, &ops).unwrap();
        p
    }

    fn reading(hz: Option<f64>) -> Descriptors {
        Descriptors {
            dur_s: 2.0,
            silent: false,
            pitch: hz.map(|f| Pitch {
                f0_hz: f,
                note: nearest_note(f).0,
                cents_dev: 0.0,
                stability: 1.0,
                voiced: 1.0,
            }),
            tonal: None,
            envelope: None,
            spectrum: None,
            noise_ratio: None,
            detune: None,
            modulation: enth_analysis::descriptors::ModulationDesc::default(),
            loudness: enth_analysis::loudness::Level {
                lufs_i: None,
                peak_dbfs: -6.0,
                true_peak_dbtp: -6.0,
                clipped_pct: 0.0,
                clipping: false,
            },
        }
    }

    #[test]
    fn a_sub_octave_names_the_transposed_oscillator() {
        let p = patch(r#"[{"add":"osc","id":"osc2","params":{"wave":"sine","semi":"-12st"}}]"#);
        let e = parse_item("pitch.note ~ C2").unwrap();
        let why = culprit(&e, &p, &reading(Some(32.7))).unwrap();
        assert!(
            why.contains("osc2 is transposed -12 st (osc2.semi)"),
            "{why}"
        );
        assert!(why.contains("C1, 12 st below"), "{why}");
        // on pitch: nothing to blame
        assert!(culprit(&e, &p, &reading(Some(65.4))).is_none());
    }

    #[test]
    fn noise_and_missing_modules_are_named() {
        let p = patch(r#"[{"add":"noise","id":"noise1"}]"#);
        let why = culprit(&parse_item("pitch ~ A3").unwrap(), &p, &reading(None)).unwrap();
        assert!(
            why.contains("noise1 covers the tone") && why.contains("tonal_pitch"),
            "{why}"
        );
        let p = patch(r#"[{"remove":"osc1"},{"add":"noise","id":"noise1"}]"#);
        let why = culprit(&parse_item("pitch ~ A3").unwrap(), &p, &reading(None)).unwrap();
        assert!(why.contains("no enabled oscillator, only noise1"), "{why}");
        let init = Patch::init();
        let why = culprit(
            &parse_item("beating in [1Hz, 5Hz]").unwrap(),
            &init,
            &reading(None),
        )
        .unwrap();
        assert!(why.contains("1 enabled oscillator (osc1)"), "{why}");
        let why = culprit(
            &parse_item("filter_lfo in [1Hz, 3Hz]").unwrap(),
            &init,
            &reading(None),
        )
        .unwrap();
        assert!(why.contains("no filter"), "{why}");
        assert!(
            culprit(
                &parse_item("noisiness > 0.5").unwrap(),
                &init,
                &reading(None)
            )
            .is_some()
        );
    }
}
