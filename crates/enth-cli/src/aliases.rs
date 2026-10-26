//! The alias table: names an agent is likely to guess, mapped to the catalogue's keys. It only feeds
//! `did_you_mean` (docs/agent-cli.md, design rule 6); patches store catalogue keys, which have no aliases. An alias
//! is offered only where its target is a key of the module in question, so `detune` suggests `fine` on an
//! oscillator but stays a real key on the reverb.

use enth_patch::catalogue::{Builtin, CATALOGUE, ModuleKind};
use enth_patch::{ModuleId, Patch};

/// Guessed parameter names and the keys they may mean.
const PARAM_ALIASES: &[(&str, &[&str])] = &[
    ("detune", &["fine"]),
    ("tune", &["fine", "semi"]),
    ("tuning", &["fine"]),
    ("finetune", &["fine"]),
    ("fine_tune", &["fine"]),
    ("cents", &["fine"]),
    ("transpose", &["semi"]),
    ("semitones", &["semi"]),
    ("semitone", &["semi"]),
    ("coarse", &["semi"]),
    ("octave", &["semi"]),
    ("pitch", &["semi"]),
    ("res", &["resonance"]),
    ("reso", &["resonance"]),
    ("q", &["resonance"]),
    ("emphasis", &["resonance"]),
    ("peak", &["resonance"]),
    ("freq", &["cutoff", "rate"]),
    ("frequency", &["cutoff", "rate"]),
    ("cut", &["cutoff"]),
    ("cutoff_hz", &["cutoff"]),
    ("fc", &["cutoff"]),
    ("volume", &["level"]),
    ("vol", &["level"]),
    ("gain", &["level", "makeup"]),
    ("amp", &["level"]),
    ("amplitude", &["level"]),
    ("output", &["level"]),
    ("waveform", &["wave"]),
    ("shape", &["wave"]),
    ("type", &["mode", "wave", "color", "shape"]),
    ("filter_type", &["mode"]),
    ("atk", &["attack"]),
    ("a", &["attack"]),
    ("dec", &["decay"]),
    ("d", &["decay"]),
    ("sus", &["sustain"]),
    ("s", &["sustain"]),
    ("rel", &["release"]),
    ("r", &["release"]),
    ("pulse_width", &["pw"]),
    ("pulsewidth", &["pw"]),
    ("width", &["pw"]),
    ("pwm", &["pw"]),
    ("duty", &["pw"]),
    ("speed", &["rate"]),
    ("lfo_rate", &["rate"]),
    ("hz", &["rate", "cutoff"]),
    ("size", &["room"]),
    ("room_size", &["room"]),
    ("time", &["decay"]),
    ("tail", &["decay"]),
    ("mix", &["wet"]),
    ("amount", &["wet", "mix"]),
    ("key_tracking", &["keytrack"]),
    ("keyfollow", &["keytrack"]),
    ("key_follow", &["keytrack"]),
    ("tracking", &["keytrack"]),
    ("saturation", &["drive"]),
    ("distortion", &["drive"]),
    ("overdrive", &["drive"]),
    ("colour", &["color"]),
    ("noise_color", &["color"]),
    ("portamento", &["glide"]),
    ("porta", &["glide"]),
    ("voices", &["polyphony"]),
    ("poly", &["polyphony", "mode"]),
    ("thresh", &["threshold"]),
    ("makeup_gain", &["makeup"]),
    ("vel", &["velocity"]),
    ("velocity_amount", &["velocity"]),
    ("start_phase", &["phase"]),
    // S2a's kinds (lane MTC): the names an agent guesses for their controls. Every name here is new to the table, so
    // no suggestion for a v1 module changes.
    ("wavetable", &["table"]),
    ("wave_table", &["table"]),
    ("morph", &["position"]),
    ("scan", &["position"]),
    ("pos", &["position"]),
    ("warp_type", &["warp"]),
    ("bend", &["warp_amount"]),
    ("fm", &["pm", "pm_index"]),
    ("fm_amount", &["pm_index"]),
    ("phase_mod", &["pm", "pm_index"]),
    ("phase_modulation", &["pm", "pm_index"]),
    ("pm_amount", &["pm_index"]),
    ("index", &["pm_index"]),
    ("ring", &["am"]),
    ("ring_mod", &["am"]),
    ("amplitude_modulation", &["am"]),
    ("unison_voices", &["unison"]),
    ("copies", &["unison"]),
    ("stereo", &["spread", "width"]),
    ("stereo_width", &["spread", "width"]),
    ("fb", &["feedback"]),
    ("regen", &["feedback"]),
    ("regeneration", &["feedback"]),
    ("repeats", &["feedback"]),
    ("echoes", &["feedback"]),
    ("bit_depth", &["bits"]),
    ("bitdepth", &["bits"]),
    ("sample_rate", &["rate"]),
    ("samplerate", &["rate"]),
    ("bias_amount", &["bias"]),
    ("asymmetry", &["bias"]),
    ("curve_type", &["mode"]),
    ("algorithm", &["mode"]),
    ("algo", &["mode"]),
    ("oversampling", &["oversample"]),
    ("keep_low", &["keep_lows"]),
    ("protect_lows", &["keep_lows"]),
    ("slope", &["mode"]),
    ("poles", &["stages", "mode"]),
    ("stage", &["stages"]),
    ("notches", &["stages"]),
    ("vowel", &["f1", "shift"]),
    ("vowel_shift", &["shift"]),
    ("formant1", &["f1"]),
    ("formant_1", &["f1"]),
    ("formant2", &["f2"]),
    ("formant_2", &["f2"]),
    ("damp", &["damping"]),
    ("dampening", &["damping"]),
    ("center_freq", &["center"]),
    ("centre", &["center"]),
    ("center_frequency", &["center"]),
    ("delay_time", &["time", "delay"]),
    ("echo_time", &["time"]),
    ("pingpong", &["ping_pong"]),
    ("tempo_sync", &["sync"]),
    ("bpm_sync", &["sync"]),
    ("synced", &["sync"]),
    ("division", &["beats"]),
    ("subdivision", &["beats"]),
    ("note_value", &["beats"]),
    ("note_length", &["beats"]),
    ("bass", &["low_gain"]),
    ("lows", &["low_gain"]),
    ("treble", &["high_gain"]),
    ("highs", &["high_gain"]),
    ("highpass", &["low_cut"]),
    ("high_pass", &["low_cut"]),
    ("lowcut", &["low_cut"]),
    ("lowpass", &["high_cut"]),
    ("low_pass", &["high_cut"]),
    ("highcut", &["high_cut"]),
    ("duration", &["length"]),
    ("loop_length", &["length"]),
    ("num_points", &["points"]),
    ("nodes", &["points"]),
    ("bipolar", &["polarity"]),
    ("unipolar", &["polarity"]),
    ("loop_mode", &["mode"]),
    ("upward_compression", &["upward"]),
    ("downward_compression", &["downward"]),
    ("bass_gain", &["low_gain"]),
    ("treble_gain", &["high_gain"]),
    ("resonance", &["feedback"]),
];

/// Guessed module names and what they stand for.
#[derive(Clone, Copy)]
enum ModuleAlias {
    AmpEnv,
    Kind(ModuleKind),
}

const MODULE_ALIASES: &[(&str, ModuleAlias)] = &[
    ("amp", ModuleAlias::AmpEnv),
    ("vca", ModuleAlias::AmpEnv),
    ("amp_env", ModuleAlias::AmpEnv),
    ("ampenv", ModuleAlias::AmpEnv),
    ("filter", ModuleAlias::Kind(ModuleKind::Svf)),
    ("filt", ModuleAlias::Kind(ModuleKind::Svf)),
    ("vcf", ModuleAlias::Kind(ModuleKind::Svf)),
    ("svf", ModuleAlias::Kind(ModuleKind::Svf)),
    ("lpf", ModuleAlias::Kind(ModuleKind::Svf)),
    ("lowpass", ModuleAlias::Kind(ModuleKind::Svf)),
    ("osc", ModuleAlias::Kind(ModuleKind::Osc)),
    ("oscillator", ModuleAlias::Kind(ModuleKind::Osc)),
    ("vco", ModuleAlias::Kind(ModuleKind::Osc)),
    ("env", ModuleAlias::Kind(ModuleKind::Adsr)),
    ("envelope", ModuleAlias::Kind(ModuleKind::Adsr)),
    ("adsr", ModuleAlias::Kind(ModuleKind::Adsr)),
    ("eg", ModuleAlias::Kind(ModuleKind::Adsr)),
    ("lfo", ModuleAlias::Kind(ModuleKind::Lfo)),
    ("noise", ModuleAlias::Kind(ModuleKind::Noise)),
    ("reverb", ModuleAlias::Kind(ModuleKind::Reverb)),
    ("verb", ModuleAlias::Kind(ModuleKind::Reverb)),
    ("rev", ModuleAlias::Kind(ModuleKind::Reverb)),
    ("comp", ModuleAlias::Kind(ModuleKind::Comp)),
    ("compressor", ModuleAlias::Kind(ModuleKind::Comp)),
    // S2a's kinds (lane MTC): the kind keys too, because `module_ids` reads only this table.
    ("wt", ModuleAlias::Kind(ModuleKind::Wt)),
    ("wavetable", ModuleAlias::Kind(ModuleKind::Wt)),
    ("wavetable_osc", ModuleAlias::Kind(ModuleKind::Wt)),
    ("wtosc", ModuleAlias::Kind(ModuleKind::Wt)),
    ("dist", ModuleAlias::Kind(ModuleKind::Dist)),
    ("distortion", ModuleAlias::Kind(ModuleKind::Dist)),
    ("overdrive", ModuleAlias::Kind(ModuleKind::Dist)),
    ("saturator", ModuleAlias::Kind(ModuleKind::Dist)),
    ("saturation", ModuleAlias::Kind(ModuleKind::Dist)),
    ("fuzz", ModuleAlias::Kind(ModuleKind::Dist)),
    ("waveshaper", ModuleAlias::Kind(ModuleKind::Dist)),
    ("ott", ModuleAlias::Kind(ModuleKind::Ott)),
    ("multiband", ModuleAlias::Kind(ModuleKind::Ott)),
    ("multiband_comp", ModuleAlias::Kind(ModuleKind::Ott)),
    ("comb", ModuleAlias::Kind(ModuleKind::Comb)),
    ("comb_filter", ModuleAlias::Kind(ModuleKind::Comb)),
    ("resonator", ModuleAlias::Kind(ModuleKind::Comb)),
    ("formant", ModuleAlias::Kind(ModuleKind::Formant)),
    ("formant_filter", ModuleAlias::Kind(ModuleKind::Formant)),
    ("vowel", ModuleAlias::Kind(ModuleKind::Formant)),
    ("mseg", ModuleAlias::Kind(ModuleKind::Mseg)),
    ("multisegment", ModuleAlias::Kind(ModuleKind::Mseg)),
    ("breakpoint", ModuleAlias::Kind(ModuleKind::Mseg)),
    ("chorus", ModuleAlias::Kind(ModuleKind::Chorus)),
    ("ensemble", ModuleAlias::Kind(ModuleKind::Chorus)),
    ("phaser", ModuleAlias::Kind(ModuleKind::Phaser)),
    ("phase_shifter", ModuleAlias::Kind(ModuleKind::Phaser)),
    ("eq", ModuleAlias::Kind(ModuleKind::Eq)),
    ("equalizer", ModuleAlias::Kind(ModuleKind::Eq)),
    ("equaliser", ModuleAlias::Kind(ModuleKind::Eq)),
    ("delay", ModuleAlias::Kind(ModuleKind::Delay)),
    ("echo", ModuleAlias::Kind(ModuleKind::Delay)),
    ("ladder", ModuleAlias::Kind(ModuleKind::Ladder)),
    ("ladder_filter", ModuleAlias::Kind(ModuleKind::Ladder)),
    ("moog", ModuleAlias::Kind(ModuleKind::Ladder)),
];

/// The keys among `keys` that the guessed parameter name `param` may mean.
pub fn param_keys(param: &str, keys: &[&'static str]) -> Vec<&'static str> {
    let p = param.trim().to_ascii_lowercase();
    PARAM_ALIASES
        .iter()
        .filter(|(alias, _)| *alias == p)
        .flat_map(|(_, targets)| targets.iter())
        .filter_map(|t| keys.iter().copied().find(|k| k == t))
        .collect()
}

/// The parameter keys of `target` (a module id in `patch`, or `voice` / `master`); every key in the catalogue when
/// the target is unknown.
pub fn target_keys(patch: Option<&Patch>, target: &str) -> Vec<&'static str> {
    let of = |specs: &'static [enth_params::ParamSpec]| specs.iter().map(|s| s.key).collect();
    match target {
        "voice" => return of(Builtin::Voice.params()),
        "master" => return of(Builtin::Master.params()),
        _ => {}
    }
    if let (Some(p), Ok(id)) = (patch, ModuleId::new(target))
        && let Some((_, m)) = p.find(&id)
    {
        return of(m.kind.params());
    }
    if let Some(kind) = kind_for_name(target) {
        return of(kind.params());
    }
    let mut keys: Vec<&'static str> = CATALOGUE
        .iter()
        .filter(|m| !m.hidden)
        .flat_map(|m| m.params.iter().map(|p| p.key))
        .chain(Builtin::Voice.params().iter().map(|p| p.key))
        .chain(Builtin::Master.params().iter().map(|p| p.key))
        .collect();
    keys.sort_unstable();
    keys.dedup();
    keys
}

/// The module kind a name stands for: a kind key (`svf`), an id prefix (`filter`), an id (`filter1`) or an alias
/// (`vcf`).
pub fn kind_for_name(name: &str) -> Option<ModuleKind> {
    let n = name.trim().to_ascii_lowercase();
    let base = n.trim_end_matches(|c: char| c.is_ascii_digit());
    if let Some(k) = ModuleKind::from_key(&n).or_else(|| ModuleKind::from_key(base)) {
        return Some(k);
    }
    if let Some(spec) = CATALOGUE.iter().find(|s| s.id_prefix == base) {
        return Some(spec.kind);
    }
    MODULE_ALIASES
        .iter()
        .find(|(alias, _)| *alias == base)
        .map(|(_, a)| match a {
            ModuleAlias::Kind(k) => *k,
            ModuleAlias::AmpEnv => ModuleKind::Adsr,
        })
}

/// Module ids in `patch` that a guessed module name may mean (`amp` is the amplitude envelope, `filter` the
/// filters, `osc` the oscillators), at most three.
pub fn module_ids(patch: &Patch, written: &str) -> Vec<String> {
    let n = written.trim().to_ascii_lowercase();
    let base = n.trim_end_matches(|c: char| c.is_ascii_digit());
    let Some((_, alias)) = MODULE_ALIASES.iter().find(|(a, _)| *a == base) else {
        return Vec::new();
    };
    match alias {
        ModuleAlias::AmpEnv => vec![patch.voice.amp_env.to_string()],
        ModuleAlias::Kind(kind) => patch
            .modules()
            .filter(|(_, m)| m.kind == *kind)
            .map(|(_, m)| m.id.to_string())
            .take(3)
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn alias_names_are_unique_within_each_table() {
        let mut seen = BTreeSet::new();
        for (alias, _) in PARAM_ALIASES {
            assert!(
                seen.insert(*alias),
                "parameter alias {alias} is listed twice"
            );
        }
        let mut seen = BTreeSet::new();
        for (alias, _) in MODULE_ALIASES {
            assert!(seen.insert(*alias), "module alias {alias} is listed twice");
        }
    }

    /// Every target of a parameter alias is a key of some kind, so no alias points at nothing.
    #[test]
    fn parameter_aliases_name_real_keys() {
        let keys: BTreeSet<&str> = CATALOGUE
            .iter()
            .flat_map(|m| m.params.iter().map(|p| p.key))
            .chain(Builtin::Voice.params().iter().map(|p| p.key))
            .chain(Builtin::Master.params().iter().map(|p| p.key))
            .collect();
        for (alias, targets) in PARAM_ALIASES {
            for t in *targets {
                assert!(keys.contains(t), "{alias} -> {t} is no key");
            }
        }
    }

    /// A module alias never steals a kind key or an id prefix from another kind (`kind_for_name` reads the kind
    /// keys first, then the prefixes, then this table), and every kind key is in the table.
    #[test]
    fn module_aliases_do_not_collide_with_kinds() {
        for (alias, target) in MODULE_ALIASES {
            let ModuleAlias::Kind(kind) = target else {
                continue;
            };
            if let Some(real) = ModuleKind::from_key(alias) {
                assert_eq!(real, *kind, "{alias} names two kinds");
            }
            if let Some(spec) = CATALOGUE.iter().find(|s| s.id_prefix == *alias) {
                assert_eq!(spec.kind, *kind, "{alias} is a prefix of another kind");
            }
        }
        for &kind in ModuleKind::ALL {
            assert!(
                MODULE_ALIASES
                    .iter()
                    .any(|(a, t)| *a == kind.key()
                        && matches!(t, ModuleAlias::Kind(k) if *k == kind)),
                "{kind} is not in the module alias table, so module_ids never finds it"
            );
        }
    }

    #[test]
    fn s2a_names_resolve_to_their_kinds_and_keys() {
        for (name, kind) in [
            ("wavetable", ModuleKind::Wt),
            ("overdrive", ModuleKind::Dist),
            ("moog", ModuleKind::Ladder),
            ("echo", ModuleKind::Delay),
            ("equalizer", ModuleKind::Eq),
            ("ensemble", ModuleKind::Chorus),
            ("vowel", ModuleKind::Formant),
            ("resonator", ModuleKind::Comb),
            ("multiband", ModuleKind::Ott),
            ("breakpoint", ModuleKind::Mseg),
            ("phase_shifter", ModuleKind::Phaser),
            ("delay2", ModuleKind::Delay),
        ] {
            assert_eq!(kind_for_name(name), Some(kind), "{name}");
        }
        // The v1 names still mean what they meant.
        assert_eq!(kind_for_name("vcf"), Some(ModuleKind::Svf));
        assert_eq!(kind_for_name("filter1"), Some(ModuleKind::Svf));
        // A guessed parameter is offered only where the module has the key.
        let keys = |kind: ModuleKind| -> Vec<&'static str> {
            kind.params().iter().map(|p| p.key).collect()
        };
        assert_eq!(param_keys("fm", &keys(ModuleKind::Wt)), ["pm", "pm_index"]);
        assert_eq!(param_keys("bit_depth", &keys(ModuleKind::Dist)), ["bits"]);
        assert_eq!(
            param_keys("bit_depth", &keys(ModuleKind::Svf)),
            Vec::<&str>::new()
        );
        assert_eq!(param_keys("echoes", &keys(ModuleKind::Delay)), ["feedback"]);
        assert_eq!(param_keys("treble", &keys(ModuleKind::Eq)), ["high_gain"]);
        assert_eq!(
            param_keys("vowel", &keys(ModuleKind::Formant)),
            ["f1", "shift"]
        );
        assert_eq!(param_keys("morph", &keys(ModuleKind::Wt)), ["position"]);
        // v1 suggestions are untouched: these alias names existed before.
        assert_eq!(param_keys("detune", &keys(ModuleKind::Osc)), ["fine"]);
        assert_eq!(param_keys("res", &keys(ModuleKind::Svf)), ["resonance"]);
    }

    /// `module_ids` finds a patch's S2a modules by their guessed names (only modules the patch has).
    #[test]
    fn module_ids_find_s2a_modules_in_a_patch() {
        use enth_patch::addr::Position;
        use enth_patch::{ModuleInstance, Op};
        let mut p = Patch::init();
        for (kind, id) in [(ModuleKind::Delay, "delay1"), (ModuleKind::Wt, "wt1")] {
            let at = Position {
                section: kind.section(),
                index: p.section(kind.section()).len(),
            };
            enth_patch::apply(
                &mut p,
                &Op::AddModule {
                    at,
                    module: ModuleInstance::new(ModuleId::new(id).unwrap(), kind),
                },
            )
            .unwrap();
        }
        assert_eq!(module_ids(&p, "echo"), ["delay1"]);
        assert_eq!(module_ids(&p, "wavetable"), ["wt1"]);
        assert!(module_ids(&p, "ladder").is_empty(), "the patch has none");
    }

    /// The hide flag: a guess about an unknown module draws its keys from the listed kinds only, so no key of a
    /// reserved kind that no listed kind shares is ever suggested.
    #[test]
    fn the_unknown_target_pool_holds_no_reserved_only_key() {
        let pool: BTreeSet<&str> = target_keys(None, "nothing_like_a_module")
            .into_iter()
            .collect();
        let listed: BTreeSet<&str> = CATALOGUE
            .iter()
            .filter(|m| !m.hidden)
            .flat_map(|m| m.params.iter().map(|p| p.key))
            .chain(Builtin::Voice.params().iter().map(|p| p.key))
            .chain(Builtin::Master.params().iter().map(|p| p.key))
            .collect();
        assert_eq!(pool, listed);
        for hidden in CATALOGUE.iter().filter(|m| m.hidden) {
            for p in hidden.params {
                if !listed.contains(p.key) {
                    assert!(!pool.contains(p.key), "{} is suggested", p.key);
                }
            }
        }
    }
}
