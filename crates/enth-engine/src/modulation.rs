//! The modulation matrix: compiled routes, their sources (modulators, velocity, key, macros, the mod wheel) and the
//! per-segment evaluation of modulated parameters (P1a lane S2b).
//!
//! A route moves its destination by `depth` percent of the destination's range in its normalised domain
//! (design.md §2):
//!
//! ```text
//! n   = clamp(smoothed_norm + Σ_routes (depth / 100) · source, 0, 1)      summed in patch route order
//! cur = from_norm(n)
//! ```
//!
//! `smoothed_norm` is the smoother's position ([`crate::params::Smoother::norm`]), so `to_norm` is not called per
//! segment. **Polarity belongs to the source**: envelopes, velocity, macros and the mod wheel run 0–1, an LFO is
//! bipolar (−1..1) or unipolar (0..1) by its own `polarity`, and `key` is `(pitch − 60) / 120` with the voice's
//! current, glided pitch (so `+100 %` of a Log cutoff's range per 120 semitones). A route carries only a depth.
//!
//! **When each destination is evaluated** (segment steps 4–6): a global modulator's parameters just before it ticks
//! (step 4), a voice modulator's just before it ticks (step 5), everything else (generators, `voice_fx`, `fx`,
//! `voice.*`, `master.*`) in step 6. A route from a modulator that ticks later reads its previous segment's value:
//! a deterministic one-segment delay that makes cycles harmless. Voice-rate destinations are evaluated for every
//! active voice in ascending order; a destination whose routes are all global is computed once and copied to every
//! voice (the same bits). Voice-rate sources never reach global destinations (the compiler refuses the route).
//!
//! A parameter with at least one route always takes `from_norm(n)`; one without keeps the exact plain value when
//! its smoother is settled ([`crate::params`]).

use std::collections::BTreeMap;

use crate::module::{MAX_VOICES, NoteState, VoiceIdx};
use crate::params::{GlobalTable, VoiceTable};

/// Macros (`macro1`…`macro8`) and the mod wheel: global sources held in the global table as nine parameters with
/// the `MACRO_VALUE` spec, after `master.*`. Index 8 is the mod wheel.
pub const MACRO_SOURCES: usize = 9;

/// The index of the mod wheel among [`MACRO_SOURCES`].
pub const MOD_WHEEL: usize = 8;

/// A route source, resolved against a plan.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Src {
    /// Output of `Plan::voice_mods[i]`.
    VoiceMod(u16),
    /// Output of `Plan::global_mods[i]`.
    GlobalMod(u16),
    /// Note velocity, 0–1.
    Velocity,
    /// `(pitch − 60) / 120`.
    Key,
    /// The mod wheel, 0–1.
    ModWheel,
    /// A macro, 0–1 (0-based).
    Macro(u8),
}

impl Src {
    /// Whether the source has one value per voice.
    pub fn is_voice_rate(self) -> bool {
        matches!(self, Src::VoiceMod(_) | Src::Velocity | Src::Key)
    }
}

/// One route into a destination.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Term {
    pub src: Src,
    /// `depth / 100`.
    pub depth: f32,
}

/// When a destination is evaluated.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Stage {
    /// Just before `global_mods[i]` ticks (its own parameters).
    GlobalMod(u16),
    /// Just before `voice_mods[i]` ticks.
    VoiceMod(u16),
    /// Step 6: processors and the built-ins.
    Processors,
}

/// One modulated parameter and its routes (`terms[start..start + len]`, in patch route order).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Dest {
    param: usize,
    start: usize,
    len: usize,
    voice_rate: bool,
}

/// A range of `Matrix::voice` or `Matrix::global`.
type Run = (usize, usize);

/// The compiled routes of a plan. Built off the audio thread; evaluated without allocating.
#[derive(Debug, Default)]
pub(crate) struct Matrix {
    terms: Vec<Term>,
    /// Voice-table destinations, by stage.
    voice: Vec<Dest>,
    /// Global-table destinations, by stage.
    global: Vec<Dest>,
    /// Per voice modulator: its destinations in `voice`.
    voice_by_vmod: Vec<Run>,
    /// Per global modulator: its destinations in `global`.
    global_by_gmod: Vec<Run>,
    voice_final: Run,
    global_final: Run,
}

/// Collects routes in patch order (compile time).
#[derive(Debug, Default)]
pub(crate) struct MatrixBuilder {
    /// (stage, in the voice table, parameter index) → terms in patch order.
    dests: BTreeMap<(Stage, bool, usize), Vec<Term>>,
}

impl MatrixBuilder {
    /// Add a route into parameter `param` of the voice table (`voice`) or the global table.
    pub fn add(&mut self, stage: Stage, voice: bool, param: usize, term: Term) {
        self.dests
            .entry((stage, voice, param))
            .or_default()
            .push(term);
    }

    /// The matrix for a plan with `n_vmods` voice and `n_gmods` global modulators; marks the routed parameters in
    /// the tables.
    pub fn build(
        self,
        n_vmods: usize,
        n_gmods: usize,
        voice_table: &mut VoiceTable,
        global_table: &mut GlobalTable,
    ) -> Matrix {
        let mut m = Matrix {
            voice_by_vmod: vec![(0, 0); n_vmods],
            global_by_gmod: vec![(0, 0); n_gmods],
            ..Matrix::default()
        };
        for ((stage, voice, param), terms) in self.dests {
            let dest = Dest {
                param,
                start: m.terms.len(),
                len: terms.len(),
                voice_rate: terms.iter().any(|t| t.src.is_voice_rate()),
            };
            m.terms.extend(terms);
            let (list, run) = if voice {
                voice_table.modulated[param] = true;
                let run = match stage {
                    Stage::VoiceMod(i) => &mut m.voice_by_vmod[usize::from(i)],
                    Stage::GlobalMod(_) | Stage::Processors => &mut m.voice_final,
                };
                (&mut m.voice, run)
            } else {
                global_table.modulated[param] = true;
                let run = match stage {
                    Stage::GlobalMod(i) => &mut m.global_by_gmod[usize::from(i)],
                    Stage::VoiceMod(_) | Stage::Processors => &mut m.global_final,
                };
                (&mut m.global, run)
            };
            // The BTreeMap iterates by stage, so each stage's destinations are contiguous.
            if run.1 == 0 {
                run.0 = list.len();
            }
            run.1 += 1;
            list.push(dest);
        }
        m
    }
}

/// What the sources read in one segment.
pub(crate) struct Sources<'a> {
    pub notes: &'a NoteState,
    /// `Plan::vmod_cur`.
    pub vmod: &'a [[f32; MAX_VOICES]],
    /// `Plan::gmod_cur`.
    pub gmod: &'a [f32],
    /// The macros and the mod wheel, this segment's smoothed values.
    pub macros: [f32; MACRO_SOURCES],
}

impl Sources<'_> {
    #[inline(always)]
    fn global(&self, src: Src) -> f32 {
        match src {
            Src::GlobalMod(i) => self.gmod[usize::from(i)],
            Src::ModWheel => self.macros[MOD_WHEEL],
            Src::Macro(i) => self.macros[usize::from(i)],
            // Refused by the compiler for global destinations.
            Src::VoiceMod(_) | Src::Velocity | Src::Key => 0.0,
        }
    }

    #[inline(always)]
    fn voice(&self, src: Src, v: VoiceIdx) -> f32 {
        match src {
            Src::VoiceMod(i) => self.vmod[usize::from(i)][v],
            Src::Velocity => self.notes.velocity[v],
            Src::Key => (self.notes.pitch_cur[v] - 60.0) / 120.0,
            Src::GlobalMod(_) | Src::ModWheel | Src::Macro(_) => self.global(src),
        }
    }
}

/// `clamp(n, 0, 1)`, falling back to `n0` if a source was NaN (a bug upstream, never a NaN parameter).
#[inline(always)]
fn settle(n: f32, n0: f32) -> f32 {
    if n.is_nan() { n0 } else { n.clamp(0.0, 1.0) }
}

impl Matrix {
    /// Whether the plan has no routes.
    pub fn is_empty(&self) -> bool {
        self.terms.is_empty()
    }

    /// The global destinations evaluated just before global modulator `i` ticks.
    pub fn eval_global_mod(&self, i: usize, table: &mut GlobalTable, src: &Sources<'_>) {
        if let Some(&run) = self.global_by_gmod.get(i) {
            self.eval_global(run, table, src);
        }
    }

    /// The voice destinations evaluated just before voice modulator `i` ticks.
    pub fn eval_voice_mod(
        &self,
        i: usize,
        table: &mut VoiceTable,
        src: &Sources<'_>,
        voices: &[VoiceIdx],
        reset: &[bool; MAX_VOICES],
    ) {
        if let Some(&run) = self.voice_by_vmod.get(i) {
            self.eval_voice(run, table, src, voices, reset);
        }
    }

    /// Step 6: every remaining destination.
    pub fn eval_processors(
        &self,
        voice_table: &mut VoiceTable,
        global_table: &mut GlobalTable,
        src: &Sources<'_>,
        voices: &[VoiceIdx],
        reset: &[bool; MAX_VOICES],
    ) {
        self.eval_voice(self.voice_final, voice_table, src, voices, reset);
        self.eval_global(self.global_final, global_table, src);
    }

    fn eval_global(&self, (start, len): Run, table: &mut GlobalTable, src: &Sources<'_>) {
        for d in &self.global[start..start + len] {
            let n0 = table.smooth[d.param].norm();
            let mut n = n0;
            for t in &self.terms[d.start..d.start + d.len] {
                n += t.depth * src.global(t.src);
            }
            table.cur[d.param] = table.specs[d.param].from_norm(settle(n, n0));
        }
    }

    fn eval_voice(
        &self,
        (start, len): Run,
        table: &mut VoiceTable,
        src: &Sources<'_>,
        voices: &[VoiceIdx],
        reset: &[bool; MAX_VOICES],
    ) {
        for d in &self.voice[start..start + len] {
            let spec = table.specs[d.param];
            let n0 = table.smooth[d.param].norm();
            let terms = &self.terms[d.start..d.start + d.len];
            let (cur, prev) = (&mut table.cur[d.param], &mut table.prev[d.param]);
            if d.voice_rate {
                for &v in voices {
                    let mut n = n0;
                    for t in terms {
                        n += t.depth * src.voice(t.src, v);
                    }
                    cur[v] = spec.from_norm(settle(n, n0));
                    if reset[v] {
                        prev[v] = cur[v];
                    }
                }
            } else {
                let mut n = n0;
                for t in terms {
                    n += t.depth * src.global(t.src);
                }
                let value = spec.from_norm(settle(n, n0));
                for &v in voices {
                    cur[v] = value;
                    if reset[v] {
                        prev[v] = value;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use enth_patch::ModuleKind;
    use enth_patch::catalogue::{AdsrParam, OscParam, SvfParam};

    const SR: f32 = 48_000.0;

    #[test]
    fn depth_is_a_share_of_the_normalised_range_summed_in_route_order() {
        let mut vt = VoiceTable::new(SR);
        let mut gt = GlobalTable::new(SR);
        let svf = vt.push(ModuleKind::Svf.params(), &ModuleKind::Svf.defaults());
        let osc = vt.push(ModuleKind::Osc.params(), &ModuleKind::Osc.defaults());
        let cutoff = svf.start + SvfParam::Cutoff.index();
        let level = osc.start + OscParam::Level.index();
        let mut b = MatrixBuilder::default();
        // +1 octave of a 20 Hz–20 kHz Log cutoff is 10.034 % of its range: velocity 1 at 10.034 %.
        b.add(
            Stage::Processors,
            true,
            cutoff,
            Term {
                src: Src::Velocity,
                depth: 0.100_343_37,
            },
        );
        b.add(
            Stage::Processors,
            true,
            level,
            Term {
                src: Src::Macro(0),
                depth: -0.5,
            },
        );
        b.add(
            Stage::Processors,
            true,
            level,
            Term {
                src: Src::ModWheel,
                depth: 0.25,
            },
        );
        let m = b.build(0, 0, &mut vt, &mut gt);
        assert!(vt.modulated[cutoff] && vt.modulated[level]);
        assert!(!vt.modulated[svf.start + SvfParam::Resonance.index()]);
        let mut notes = NoteState::new();
        notes.velocity[3] = 1.0;
        notes.velocity[5] = 0.0;
        let mut macros = [0.0; MACRO_SOURCES];
        macros[0] = 1.0;
        macros[MOD_WHEEL] = 0.4;
        let src = Sources {
            notes: &notes,
            vmod: &[],
            gmod: &[],
            macros,
        };
        let reset = [true; MAX_VOICES];
        vt.advance(&[3, 5], &reset);
        m.eval_processors(&mut vt, &mut gt, &src, &[3, 5], &reset);
        let c3 = vt.cur[cutoff][3];
        assert!((c3 / 4000.0 - 1.0).abs() < 1e-4, "one octave up: {c3}");
        assert_eq!(vt.prev[cutoff][3], c3, "a reset voice has no ramp");
        // velocity 0: from_norm(to_norm(2000)), within one step of the base (a routed parameter round-trips).
        assert!((vt.cur[cutoff][5] - 2000.0).abs() < 0.01);
        // level: 100 % − 50 % · 1 + 25 % · 0.4 = 60 %.
        assert!(
            (vt.cur[level][3] - 60.0).abs() < 1e-4,
            "{}",
            vt.cur[level][3]
        );
        assert_eq!(
            vt.cur[level][3], vt.cur[level][5],
            "global sources: one value"
        );
    }

    #[test]
    fn modulation_clamps_to_the_range_and_survives_nan_sources() {
        let mut vt = VoiceTable::new(SR);
        let mut gt = GlobalTable::new(SR);
        let env = vt.push(ModuleKind::Adsr.params(), &ModuleKind::Adsr.defaults());
        let sustain = env.start + AdsrParam::Sustain.index();
        let mut b = MatrixBuilder::default();
        b.add(
            Stage::VoiceMod(1),
            true,
            sustain,
            Term {
                src: Src::VoiceMod(0),
                depth: 1.0,
            },
        );
        let m = b.build(2, 0, &mut vt, &mut gt);
        let notes = NoteState::new();
        let mut out = [[0.0f32; MAX_VOICES]; 2];
        out[0][0] = 5.0;
        out[0][1] = -5.0;
        out[0][2] = f32::NAN;
        let src = Sources {
            notes: &notes,
            vmod: &out,
            gmod: &[],
            macros: [0.0; MACRO_SOURCES],
        };
        let reset = [false; MAX_VOICES];
        vt.advance(&[0, 1, 2], &reset);
        m.eval_voice_mod(0, &mut vt, &src, &[0, 1, 2], &reset);
        assert_eq!(vt.cur[sustain][0], 70.0, "not this modulator's stage");
        m.eval_voice_mod(1, &mut vt, &src, &[0, 1, 2], &reset);
        assert_eq!(vt.cur[sustain][0], 100.0);
        assert_eq!(vt.cur[sustain][1], 0.0);
        assert!((vt.cur[sustain][2] - 70.0).abs() < 1e-4);
    }
}
