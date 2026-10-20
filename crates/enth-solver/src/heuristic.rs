//! **Owner: lane MT** (match). A heuristic start for a match, from the target's descriptors.
//!
//! Ported in spirit from the Phase 0 spike (`spikes/spike-metric/src/heuristic.rs`: beat rate gives detune, a crest
//! envelope gives A/D/S/R, harmonic roll-off against the guessed waveform's ideal spectrum gives the cutoff) and
//! adapted from the spike's ten-parameter toy synth to the real catalogue. The spike read the target's audio; this
//! reads its [`Descriptors`] (`enth describe`'s numbers), so it works for any target `describe` can measure and
//! needs no audio of its own. The spike's start put the first guess within 0.15 of the range on the envelope and
//! 0.13 on the cutoff and recovered 17 of 20 targets from it against 13 from a random start
//! (`docs/spikes/metric.md`); the numbers for this port are in `docs/dsp/match.md`.
//!
//! What it estimates, each only when the parameter is free ([`FreeSet`]) and the module is there:
//!
//! | Descriptor | Parameter | How |
//! | --- | --- | --- |
//! | `envelope.attack_ms`, `decay_ms`, `sustain_db`, `release_ms` | the amp envelope's `attack`, `decay`, `sustain`, `release` | the descriptors are thresholds on the rendered level, not the knobs: the ratios below were measured on the engine (attack: the 1 dB point is 0.75–0.8 of the knob; release: the −40 dB point is 0.5; decay: 0.45 down to 0.1 as the sustain rises) |
//! | `pitch.f0_hz` | the first oscillator's `semi` and `fine` | the interval between the played note and the measured pitch, split into whole semitones and cents when both are free |
//! | `spectrum.centroid_hz` | the filter's `cutoff` | a harmonic model of the first oscillator's wave through the filter's response (lp, hp, bp, with the patch's resonance), its centroid matched on a 1/12-octave grid; the model follows the engine's centroids within 1–13 % |
//! | `detune.beating_hz` | the second oscillator's `fine` | `beating_hz` over the pitch, in cents (the spike's rule) |
//!
//! It does **not** guess categorical parameters (a waveform is not in the descriptors; [`search`](crate::search)
//! probes the neighbours and keeps the best), resonance, drive, key tracking, filter envelopes, LFOs or effects.
//! The cutoff depends on the waveform and the filter mode, so [`Heuristic::suggest`] takes the patch *with the
//! categorical choice already applied*; the search calls it once per assignment.
//!
//! Pure: no renders, no clocks, no I/O; every transcendental is `libm`.

use core::f64::consts::PI;

use enth_analysis::Descriptors;
use enth_patch::catalogue::{AdsrParam, FilterMode, OscParam, SvfParam, Wave};
use enth_patch::{ModuleInstance, ModuleKind, ParamAddr, Patch};
use enth_render::Performance;

use crate::search::FreeSet;

/// The ratio of the attack descriptor (onset to within 1 dB of the peak) to the `attack` knob, above 15 ms.
const ATTACK_RATIO: f64 = 0.77;
/// Below this descriptor (ms) the attack reads the measurement floor, not the knob: take it as it is.
const ATTACK_FLOOR_MS: f64 = 15.0;
/// The ratio of the release descriptor (note-off to −40 dB) to the `release` knob.
const RELEASE_RATIO: f64 = 0.5;
/// The sustain descriptor sits this much above the knob's level (the peak is a little below full scale).
const SUSTAIN_BIAS: f64 = 1.15;
/// The decay descriptor over the `decay` knob at a sustain level (fraction 0–1), measured on the engine: a deeper
/// decay (low sustain) reads a larger share of the knob.
const DECAY_RATIO: [(f64, f64); 7] = [
    (0.0, 0.45),
    (0.1, 0.35),
    (0.3, 0.25),
    (0.5, 0.2),
    (0.7, 0.11),
    (0.9, 0.03),
    (1.0, 0.01),
];
/// The highest harmonic the centroid model counts (Hz): the engine's oscillators are band-limited below Nyquist.
const MODEL_TOP_HZ: f64 = 20_000.0;
/// Cutoff grid: 1/12 octave from 20 Hz to 20 kHz.
const GRID_STEPS: usize = 120;

/// How far the first CMA-ES step may stray from a suggestion, relative to the search's default step, per kind of
/// suggestion: the measured pitch is good to a few cents (a default step on the ±48 semitone axis would be 14
/// semitones), the cutoff model to about 0.15 octave (the default step is 1.5 octaves), the envelope ratios to about
/// 20 %. Measured on the randomizer's level-1 and level-2 targets (`docs/dsp/match.md`).
const SPREAD_PITCH: f64 = 0.02;
const SPREAD_CUTOFF: f64 = 0.3;
const SPREAD_ENVELOPE: f64 = 0.5;
const SPREAD_DETUNE: f64 = 0.5;

/// A suggestion at least this precise (a [`Suggestion::spread`] below it) is kept by random restarts: only the
/// parameters the descriptors do not pin down are re-randomised.
pub const PRECISE: f64 = 0.1;

/// One suggested value.
#[derive(Clone, Debug, PartialEq)]
pub struct Suggestion {
    /// The parameter.
    pub addr: ParamAddr,
    /// The plain value, already clamped to the parameter's range.
    pub value: f32,
    /// Which descriptor it comes from (`envelope.attack_ms`).
    pub from: &'static str,
    /// How wide the first search step around the value should be, relative to the search's default step (1 =
    /// default; the pitch is far tighter than the cutoff). Below [`PRECISE`] a random restart keeps the value.
    pub spread: f64,
}

/// Suggests parameter values from a target's descriptors. See the [module docs](self).
#[derive(Clone, Debug)]
pub struct Heuristic {
    desc: Descriptors,
    note_hz: f64,
}

/// The frequency of the first note the performance plays (earliest on, ties in performance order).
fn first_note_hz(perf: &Performance) -> f64 {
    let first = perf
        .notes
        .iter()
        .enumerate()
        .min_by(|(i, a), (j, b)| a.on_s.total_cmp(&b.on_s).then(i.cmp(j)))
        .map(|(_, n)| n.key);
    440.0 * libm::exp2((f64::from(first.unwrap_or(69)) - 69.0) / 12.0)
}

fn decay_ratio(sustain: f64) -> f64 {
    let s = sustain.clamp(0.0, 1.0);
    for w in DECAY_RATIO.windows(2) {
        let ((s0, r0), (s1, r1)) = (w[0], w[1]);
        if s <= s1 {
            return r0 + (r1 - r0) * (s - s0) / (s1 - s0);
        }
    }
    DECAY_RATIO[DECAY_RATIO.len() - 1].1
}

/// The relative amplitude of harmonic `k` of a wave (the engine's oscillators are band-limited, so the ideal series
/// is the model).
fn harmonic(wave: Wave, pw: f64, k: usize) -> f64 {
    let kf = k as f64;
    match wave {
        Wave::Sine => {
            if k == 1 {
                1.0
            } else {
                0.0
            }
        }
        Wave::Triangle => {
            if k % 2 == 1 {
                1.0 / (kf * kf)
            } else {
                0.0
            }
        }
        // The wavetable's content is not known here: a saw is the brightest common case.
        Wave::Saw | Wave::Table => 1.0 / kf,
        Wave::Pulse => libm::fabs(libm::sin(PI * kf * pw)) / kf,
    }
}

/// Magnitude of the filter's 2-pole response at `f` for a cutoff `fc` and quality `q`.
fn filter_gain(mode: FilterMode, f: f64, fc: f64, q: f64) -> f64 {
    let r = f / fc;
    let den = libm::sqrt((1.0 - r * r) * (1.0 - r * r) + (r / q) * (r / q));
    let lp = 1.0 / den;
    match mode {
        FilterMode::Lp => lp,
        FilterMode::Hp => r * r * lp,
        FilterMode::Bp => (r / q) * lp,
        FilterMode::Notch => libm::fabs(1.0 - r * r) * lp,
    }
}

/// The modelled magnitude-weighted centroid (Hz) of `wave` at `f0` through the filter at `fc`.
fn model_centroid(wave: Wave, pw: f64, f0: f64, filter: Option<(FilterMode, f64, f64)>) -> f64 {
    let kmax = (MODEL_TOP_HZ / f0) as usize;
    let (mut num, mut den) = (0.0, 0.0);
    for k in 1..=kmax.max(1) {
        let f = k as f64 * f0;
        let mut m = harmonic(wave, pw, k);
        if let Some((mode, fc, q)) = filter {
            m *= filter_gain(mode, f, fc, q);
        }
        num += f * m;
        den += m;
    }
    if den > 0.0 { num / den } else { f0 }
}

fn first_enabled(patch: &Patch, kind: ModuleKind) -> Option<&ModuleInstance> {
    patch
        .modules()
        .map(|(_, m)| m)
        .find(|m| m.enabled && m.kind == kind)
}

fn addr_of(m: &ModuleInstance, key: &str) -> Option<ParamAddr> {
    ParamAddr::module(m.id.clone(), key).ok()
}

impl Heuristic {
    /// A heuristic for a target measured as `desc`, played as `perf`.
    #[must_use]
    pub fn new(desc: &Descriptors, perf: &Performance) -> Heuristic {
        Heuristic {
            desc: desc.clone(),
            note_hz: first_note_hz(perf),
        }
    }

    /// Values for the free parameters of `patch` (which carries the categorical choice to suggest for), in a fixed
    /// order: the amp envelope, the pitch, the cutoff, the detune. Parameters the descriptors say nothing about are
    /// left out, so the caller keeps the patch's value for them.
    #[must_use]
    pub fn suggest(&self, patch: &Patch, free: &FreeSet) -> Vec<Suggestion> {
        let mut out: Vec<Suggestion> = Vec::new();
        let mut push = |patch: &Patch,
                        addr: Option<ParamAddr>,
                        value: f64,
                        from: &'static str,
                        spread: f64| {
            let Some(addr) = addr.filter(|a| free.contains(a)) else {
                return;
            };
            if let Some(spec) = patch.spec(&addr) {
                out.push(Suggestion {
                    value: spec.clamp(value as f32),
                    addr,
                    from,
                    spread,
                });
            }
        };

        // --- the amplitude envelope ---
        if let (Some(env), Some((_, amp))) = (
            self.desc.envelope,
            patch.find(&patch.voice.amp_env).filter(|(_, m)| m.enabled),
        ) {
            let key = |p: AdsrParam| addr_of(amp, p.key());
            let attack = if env.attack_ms >= ATTACK_FLOOR_MS {
                env.attack_ms / ATTACK_RATIO
            } else {
                env.attack_ms
            };
            push(
                patch,
                key(AdsrParam::Attack),
                attack,
                "envelope.attack_ms",
                SPREAD_ENVELOPE,
            );
            let sustain_frac = match env.sustain_db {
                Some(db) => {
                    let pct = if db > -0.5 {
                        100.0
                    } else {
                        100.0 * libm::pow(10.0, db / 20.0) / SUSTAIN_BIAS
                    };
                    push(
                        patch,
                        key(AdsrParam::Sustain),
                        pct,
                        "envelope.sustain_db",
                        SPREAD_ENVELOPE,
                    );
                    Some((pct / 100.0).clamp(0.0, 1.0))
                }
                None => amp
                    .get(AdsrParam::Sustain.key())
                    .map(|v| f64::from(v) / 100.0),
            };
            if let (Some(decay), Some(s)) = (env.decay_ms, sustain_frac)
                && decay > 0.0
            {
                push(
                    patch,
                    key(AdsrParam::Decay),
                    decay / decay_ratio(s),
                    "envelope.decay_ms",
                    SPREAD_ENVELOPE,
                );
            }
            if let Some(release) = env.release_ms {
                push(
                    patch,
                    key(AdsrParam::Release),
                    release / RELEASE_RATIO,
                    "envelope.release_ms",
                    SPREAD_ENVELOPE,
                );
            }
        }

        // --- the pitch ---
        let osc = first_enabled(patch, ModuleKind::Osc);
        let f0 = self
            .desc
            .pitch
            .as_ref()
            .map(|p| p.f0_hz)
            .filter(|f| *f > 0.0);
        if let (Some(osc), Some(f0)) = (osc, f0) {
            let semitones = 12.0 * libm::log2(f0 / self.note_hz);
            let semi_addr = addr_of(osc, OscParam::Semi.key()).filter(|a| free.contains(a));
            let fine_addr = addr_of(osc, OscParam::Fine.key()).filter(|a| free.contains(a));
            let fine_now = f64::from(osc.get(OscParam::Fine.key()).unwrap_or(0.0));
            let semi_now = f64::from(osc.get(OscParam::Semi.key()).unwrap_or(0.0));
            match (semi_addr.is_some(), fine_addr.is_some()) {
                (true, true) => {
                    let semi = libm::round(semitones);
                    push(patch, semi_addr, semi, "pitch.f0_hz", SPREAD_PITCH);
                    push(
                        patch,
                        fine_addr,
                        (semitones - semi) * 100.0,
                        "pitch.f0_hz",
                        SPREAD_PITCH,
                    );
                }
                (true, false) => push(
                    patch,
                    semi_addr,
                    semitones - fine_now / 100.0,
                    "pitch.f0_hz",
                    SPREAD_PITCH,
                ),
                (false, true) => push(
                    patch,
                    fine_addr,
                    (semitones - semi_now) * 100.0,
                    "pitch.f0_hz",
                    SPREAD_PITCH,
                ),
                (false, false) => {}
            }
        }

        // --- the cutoff ---
        if let (Some(osc), Some(filter), Some(spec)) = (
            osc,
            first_enabled(patch, ModuleKind::Svf),
            self.desc.spectrum,
        ) {
            let cutoff = addr_of(filter, SvfParam::Cutoff.key());
            let wave = osc
                .get(OscParam::Wave.key())
                .and_then(Wave::from_value)
                .unwrap_or(Wave::Saw);
            let mode = filter
                .get(SvfParam::Mode.key())
                .and_then(FilterMode::from_value)
                .unwrap_or(FilterMode::Lp);
            if wave != Wave::Sine && spec.centroid_hz > 0.0 {
                let pw = f64::from(osc.get(OscParam::Pw.key()).unwrap_or(50.0)) / 100.0;
                let f0 = f0.unwrap_or(self.note_hz);
                let res = f64::from(filter.get(SvfParam::Resonance.key()).unwrap_or(10.0));
                let q = 0.5 * libm::pow(50.0, res / 100.0);
                let want = libm::log(spec.centroid_hz);
                let (mut best, mut best_err) = (f64::from(filter.values[0]), f64::INFINITY);
                let (lo, hi) = (libm::log(20.0), libm::log(20_000.0));
                for i in 0..=GRID_STEPS {
                    let fc = libm::exp(lo + (hi - lo) * i as f64 / GRID_STEPS as f64);
                    let c = model_centroid(wave, pw, f0, Some((mode, fc, q)));
                    let err = libm::fabs(libm::log(c) - want);
                    if err < best_err {
                        best = fc;
                        best_err = err;
                    }
                }
                if best_err.is_finite() {
                    push(patch, cutoff, best, "spectrum.centroid_hz", SPREAD_CUTOFF);
                }
            }
        }

        // --- the detune between two oscillators ---
        if let Some(beat) = self.desc.detune {
            let mut oscs = patch
                .modules()
                .map(|(_, m)| m)
                .filter(|m| m.enabled && m.kind == ModuleKind::Osc);
            let _first = oscs.next();
            if let Some(second) = oscs.next() {
                let pitch = f0.unwrap_or(self.note_hz);
                let cents = 1200.0 * libm::log2(1.0 + beat.beating_hz / pitch);
                push(
                    patch,
                    addr_of(second, OscParam::Fine.key()),
                    cents,
                    "detune.beating_hz",
                    SPREAD_DETUNE,
                );
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use enth_analysis::Describer;
    use enth_analysis::descriptors::Want;
    use enth_patch::{Op, Position, Section, apply};
    use enth_render::{RenderSpec, render};

    fn set(p: &mut Patch, a: &str, v: f32) {
        apply(
            p,
            Op::SetParam {
                addr: a.parse().unwrap(),
                value: v,
            },
        )
        .unwrap();
    }

    fn with_filter(p: &mut Patch) {
        let m = ModuleInstance::new("filter1".parse().unwrap(), ModuleKind::Svf);
        apply(
            p,
            Op::AddModule {
                at: Position {
                    section: Section::VoiceFx,
                    index: 0,
                },
                module: m,
            },
        )
        .unwrap();
    }

    fn describe(p: &Patch, perf: &Performance) -> Descriptors {
        let out = render(p, perf, &RenderSpec::default()).unwrap();
        Describer::new(48_000.0)
            .describe_some(&out.mid(), crate::note_timing(perf), Want::ALL)
            .unwrap()
    }

    fn free_all(p: &Patch, addrs: &[&str]) -> FreeSet {
        let a: Vec<ParamAddr> = addrs.iter().map(|s| s.parse().unwrap()).collect();
        FreeSet::new(p, &a).unwrap()
    }

    fn value(s: &[Suggestion], addr: &str) -> f64 {
        let a: ParamAddr = addr.parse().unwrap();
        f64::from(
            s.iter()
                .find(|s| s.addr == a)
                .unwrap_or_else(|| panic!("no suggestion for {addr}: {s:?}"))
                .value,
        )
    }

    fn perf() -> Performance {
        enth_challenge::phrases::phrase(enth_challenge::phrases::HELD)
            .unwrap()
            .performance()
    }

    #[test]
    fn decay_ratio_interpolates_the_measured_table() {
        assert_eq!(decay_ratio(0.0), 0.45);
        assert!((decay_ratio(1.0) - 0.01).abs() < 1e-12);
        assert!((decay_ratio(0.2) - 0.30).abs() < 1e-12);
        assert!(decay_ratio(0.4) > decay_ratio(0.6));
    }

    #[test]
    fn the_model_centroid_follows_the_engine() {
        // Measured on the engine (C3, resonance 10 %, lp): saw 773.8 Hz at 1.6 kHz, 1814 at 6.4 kHz; square 659.9 and
        // 1599.9; triangle 224.2 at 1.6 kHz.
        let f0 = 130.8128;
        let lp = |fc| Some((FilterMode::Lp, fc, 0.5 * libm::pow(50.0, 0.1)));
        for (wave, fc, measured) in [
            (Wave::Saw, 1600.0, 773.8),
            (Wave::Saw, 6400.0, 1814.0),
            (Wave::Pulse, 1600.0, 659.9),
            (Wave::Pulse, 6400.0, 1599.9),
            (Wave::Triangle, 1600.0, 224.2),
        ] {
            let m = model_centroid(wave, 0.5, f0, lp(fc));
            assert!(
                (m / measured - 1.0).abs() < 0.15,
                "{wave:?} at {fc}: model {m}, engine {measured}"
            );
        }
    }

    #[test]
    fn recovers_the_envelope_pitch_and_cutoff_of_a_rendered_target() {
        let mut target = Patch::init();
        with_filter(&mut target);
        set(&mut target, "osc1.wave", 2.0); // saw
        set(&mut target, "osc1.semi", 7.0);
        set(&mut target, "filter1.cutoff", 1200.0);
        set(&mut target, "env1.attack", 200.0);
        set(&mut target, "env1.decay", 600.0);
        set(&mut target, "env1.sustain", 40.0);
        set(&mut target, "env1.release", 500.0);
        let perf = perf();
        let desc = describe(&target, &perf);

        let mut start = Patch::init();
        with_filter(&mut start);
        let free = free_all(
            &start,
            &[
                "osc1.semi",
                "osc1.fine",
                "filter1.cutoff",
                "env1.attack",
                "env1.decay",
                "env1.sustain",
                "env1.release",
                "osc1.level",
            ],
        );
        let s = Heuristic::new(&desc, &perf).suggest(&start, &free);

        let within = |got: f64, want: f64, ratio: f64| got / want < ratio && want / got < ratio;
        assert!(within(value(&s, "env1.attack"), 200.0, 1.3));
        assert!(within(value(&s, "env1.release"), 500.0, 1.3));
        assert!(within(value(&s, "env1.decay"), 600.0, 2.0));
        assert!((value(&s, "env1.sustain") - 40.0).abs() < 8.0);
        assert_eq!(value(&s, "osc1.semi"), 7.0);
        assert!(value(&s, "osc1.fine").abs() < 1.0);
        let fc = value(&s, "filter1.cutoff");
        assert!(within(fc, 1200.0, 1.6), "cutoff {fc}");
        // Nothing for a parameter the descriptors do not speak of.
        assert!(!s.iter().any(|s| s.addr.to_string() == "osc1.level"));
    }

    #[test]
    fn only_free_parameters_and_present_modules_are_suggested() {
        let target = Patch::init();
        let perf = perf();
        let desc = describe(&target, &perf);
        let start = Patch::init(); // no filter module
        let free = free_all(&start, &["env1.attack", "osc1.wave"]);
        let s = Heuristic::new(&desc, &perf).suggest(&start, &free);
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].addr.to_string(), "env1.attack");
        assert_eq!(s[0].from, "envelope.attack_ms");
    }

    #[test]
    fn a_sine_gives_no_cutoff_guess_and_the_pitch_follows_the_note() {
        let mut target = Patch::init();
        with_filter(&mut target);
        set(&mut target, "osc1.wave", 0.0);
        set(&mut target, "osc1.semi", -12.0);
        let perf = perf();
        let desc = describe(&target, &perf);
        let mut start = Patch::init();
        with_filter(&mut start);
        let free = free_all(&start, &["osc1.semi", "filter1.cutoff"]);
        // For the start's own wave (a saw) there is a cutoff to guess ...
        let s = Heuristic::new(&desc, &perf).suggest(&start, &free);
        assert!((value(&s, "osc1.semi") + 12.0).abs() < 0.05);
        assert!(s.iter().any(|s| s.addr.to_string() == "filter1.cutoff"));
        // ... but a sine has one harmonic: the filter cannot be read off its centroid.
        set(&mut start, "osc1.wave", 0.0);
        let s = Heuristic::new(&desc, &perf).suggest(&start, &free);
        assert!(!s.iter().any(|s| s.addr.to_string() == "filter1.cutoff"));
    }
}
