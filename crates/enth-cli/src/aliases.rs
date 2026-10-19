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
