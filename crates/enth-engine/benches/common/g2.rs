//! The G2 reference patch (S2a lane G2B), shared by the `g2` bench and the `g2` tests
//! (`#[path = "common/g2.rs"] mod g2_patch;`): a dubstep growl voice that exercises what S2a added, the way G1 does
//! for P1a. The test host's fixtures `tools/test-host/golden/enth-plugin.g2*.patch.json` are `Patch::to_json()` of
//! [`g2_patch`], [`value_edit`] and [`module_added`] (and, as `enth-plugin.g2-stress*.patch.json`, of the same three
//! for [`G2::stress`]), and the golden case `tests/golden/cases/g2-growl.toml` embeds the reference patch (a test in
//! `tests/golden/tests/g2.rs` compares them); change them together.
//!
//! The sound is report 04's S4 "FM growl" (`_local/sessions/2026-10-25-s2-depth-and-fast-training/`), built from the
//! modules that exist:
//!
//! - **Source:** a `wt` on the `basic` table at 60 % (a saw-leaning frame) with **7-copy unison** (22 cents, 80 %
//!   blend, 60 % spread), phase-modulated by a second generator, a sine `osc` at 1:1 with its level at 0 (it is heard
//!   only through the audio route). An **MSEG** (one pass per eighth note, five bent points, per voice) pushes
//!   `pm_index` up by 4 radians and the table position by 25 % once per eighth: that is the growl.
//! - **Voice FX:** `dist` (asymmetric, 18 dB, the default 2× oversampling) into a `ladder` band-pass (the MSEG moves
//!   its cutoff) into a `formant` (a vowel between "o" and "i": a **synced LFO**, a dotted eighth against the MSEG's
//!   eighth, sweeps F2). Four voice-FX slots are the most; three are used.
//! - **FX:** `ott` (60 %, report 04's depth), `chorus` (default, 30 % mix) and a synced `delay` (dotted eighth, 30 %
//!   feedback, ping-pong, 25 % mix).
//! - **Voices:** `polyphony` **4**, the reference (the coordinator's decision of 2026-10-27: bass is played mono or
//!   with low polyphony, and four voices cover the overlap of releases). The bench holds the first four notes of the
//!   bass chord [`KEYS`]. **8 voices is the stress case** ([`G2::stress`]): the same patch with `polyphony` 8 and all
//!   eight notes held; it has its own bench, fixtures and hard limit and is not the budget (docs/benchmarks.md, "The
//!   G2 row").
//!
//! Because a unison `wt` makes the voice stereo, the plan runs the whole voice-FX chain twice (a right-channel twin of
//! every module): the chain's cost is doubled, which is the price of S2a's stereo voice path and the reason G2's
//! number is not the sum of the modules' own (docs/dsp/engine.md, "Stereo voice path").
#![allow(dead_code, reason = "each bench and test uses a different subset")]

use enth_patch::catalogue::{
    AdsrParam, Beats, ChorusParam, DistMode, DistParam, FormantParam, LadderMode, LadderParam,
    LfoParam, MSEG_POINT_BASE, MasterParam, MsegMode, MsegParam, OscParam, OttParam, Oversample,
    VoiceParam, Wave, WtParam, WtTable,
};
use enth_patch::{ModuleId, ModuleInstance, ModuleKind, Patch, Route};

/// The held chord of the benches and the real-time run: E1, B1, E2, G2, B2, E3, G3, B3 (an E minor stack across the
/// bass register). A variant with `voices` voices holds the first `voices` of them, so no voice is stolen: the
/// reference (4 voices) holds E1 B1 E2 G2, the stress case all eight. `enth-test-host`'s `--notes` scripts for the
/// same chords are [`NOTES`] and [`STRESS_NOTES`].
pub const KEYS: [u8; 8] = [28, 35, 40, 43, 47, 52, 55, 59];

/// The reference's chord (the first four of [`KEYS`]) as a test-host notes script (`key@start+duration:velocity`,
/// seconds): held for an hour.
pub const NOTES: &str = "E1@0+3600:0.8, B1@0+3600:0.8, E2@0+3600:0.8, G2@0+3600:0.8";

/// The stress case's chord (all of [`KEYS`]) as a test-host notes script.
pub const STRESS_NOTES: &str = "E1@0+3600:0.8, B1@0+3600:0.8, E2@0+3600:0.8, G2@0+3600:0.8, B2@0+3600:0.8, E3@0+3600:0.8, G3@0+3600:0.8, B3@0+3600:0.8";

fn id(s: &str) -> ModuleId {
    ModuleId::new(s).expect("valid id")
}

/// A module of `kind` with the given parameter values set.
pub fn module(name: &str, kind: ModuleKind, set: &[(usize, f32)]) -> ModuleInstance {
    let mut m = ModuleInstance::new(id(name), kind);
    for &(i, v) in set {
        m.values[i] = v;
    }
    m
}

/// The choices that make up a G2 variant. [`G2::default`] is the reference patch (4 voices); [`G2::stress`] is the
/// 8-voice stress case; the bench measures variants of the stress case to show what each choice costs
/// (docs/benchmarks.md, "The G2 row").
#[derive(Clone, Copy, Debug)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "each flag is one independent choice of the variant, named in the bench"
)]
pub struct G2 {
    /// `polyphony` (and how many notes of [`KEYS`] the bench holds).
    pub voices: u32,
    /// The `wt`'s unison copies (1 turns the unison and the stereo voice path off).
    pub copies: u32,
    /// The `dist`'s curve (the reference is the asymmetric one, report 04's "soft/tube").
    pub dist_mode: DistMode,
    /// The `dist`'s oversampling.
    pub oversample: Oversample,
    /// Whether the MSEG moves the table position (the reference does; PM and the filters stay modulated).
    pub position_mod: bool,
    /// Whether the `ladder` is in the voice-FX chain.
    pub ladder: bool,
    /// Whether the `formant` is in the voice-FX chain.
    pub formant: bool,
    /// Whether the `delay` is in the FX chain.
    pub delay: bool,
}

impl Default for G2 {
    fn default() -> Self {
        Self {
            voices: 4,
            copies: 7,
            dist_mode: DistMode::Asymmetric,
            oversample: Oversample::X2,
            position_mod: true,
            ladder: true,
            formant: true,
            delay: true,
        }
    }
}

impl G2 {
    /// The stress case: the reference with 8 voices (all of [`KEYS`] held). Not the budget.
    pub fn stress() -> Self {
        Self {
            voices: 8,
            ..Self::default()
        }
    }

    /// The variant's patch.
    pub fn patch(self) -> Patch {
        let mut p = Patch::init();
        p.voice.values[VoiceParam::Polyphony.index()] = self.voices as f32;
        p.master[MasterParam::Level.index()] = -9.0;

        p.generators = vec![
            // The PM source: a sine at the carrier's pitch, silent in the mix (level 0), heard through the route.
            module(
                "osc1",
                ModuleKind::Osc,
                &[
                    (OscParam::Wave.index(), Wave::Sine.index() as f32),
                    (OscParam::Level.index(), 0.0),
                ],
            ),
            module(
                "wt1",
                ModuleKind::Wt,
                &[
                    (WtParam::Table.index(), WtTable::Basic.index() as f32),
                    (WtParam::Position.index(), 60.0),
                    (WtParam::PmIndex.index(), 1.0),
                    (WtParam::Unison.index(), self.copies as f32),
                    (WtParam::Detune.index(), 22.0),
                    (WtParam::Blend.index(), 80.0),
                    (WtParam::Spread.index(), 60.0),
                    (WtParam::Level.index(), 80.0),
                ],
            ),
        ];

        let mut voice_fx = vec![module(
            "dist1",
            ModuleKind::Dist,
            &[
                (DistParam::Mode.index(), self.dist_mode.index() as f32),
                (DistParam::Drive.index(), 18.0),
                (
                    DistParam::Oversample.index(),
                    self.oversample.index() as f32,
                ),
            ],
        )];
        if self.ladder {
            voice_fx.push(module(
                "ladder1",
                ModuleKind::Ladder,
                &[
                    (LadderParam::Mode.index(), LadderMode::Bp.index() as f32),
                    (LadderParam::Cutoff.index(), 900.0),
                    (LadderParam::Resonance.index(), 35.0),
                ],
            ));
        }
        if self.formant {
            voice_fx.push(module(
                "formant1",
                ModuleKind::Formant,
                &[
                    (FormantParam::F1.index(), 520.0),
                    (FormantParam::F2.index(), 900.0),
                    (FormantParam::Resonance.index(), 55.0),
                    (FormantParam::Mix.index(), 60.0),
                ],
            ));
        }
        p.voice_fx = voice_fx;

        let mut mseg = module(
            "mseg1",
            ModuleKind::Mseg,
            &[
                (MsegParam::Mode.index(), MsegMode::Loop.index() as f32),
                (MsegParam::Beats.index(), Beats::Eighth.index() as f32),
                (MsegParam::Points.index(), 5.0),
            ],
        );
        // Five points over one eighth note: a quick rise, a bent fall and a second small bump, report 04's S4
        // (0 -> 1 -> 0.4 -> 0.8), as (time %, level %, curve %).
        for (i, (t, l, c)) in [
            (0.0, 5.0, 60.0),
            (25.0, 100.0, -40.0),
            (55.0, 40.0, 30.0),
            (75.0, 80.0, -30.0),
            (100.0, 5.0, 0.0),
        ]
        .into_iter()
        .enumerate()
        {
            let base = MSEG_POINT_BASE + 3 * i;
            mseg.values[base] = t;
            mseg.values[base + 1] = l;
            mseg.values[base + 2] = c;
        }
        p.modulators = vec![
            module(
                "env1",
                ModuleKind::Adsr,
                &[
                    (AdsrParam::Attack.index(), 2.0),
                    (AdsrParam::Decay.index(), 0.0),
                    (AdsrParam::Sustain.index(), 100.0),
                    (AdsrParam::Release.index(), 120.0),
                ],
            ),
            mseg,
            // A synced LFO: a dotted eighth, against the MSEG's eighth.
            module(
                "lfo1",
                ModuleKind::Lfo,
                &[
                    (LfoParam::Sync.index(), 1.0),
                    (LfoParam::Beats.index(), Beats::EighthDotted.index() as f32),
                ],
            ),
        ];

        p.fx = vec![
            module("ott1", ModuleKind::Ott, &[(OttParam::Depth.index(), 60.0)]),
            module(
                "chorus1",
                ModuleKind::Chorus,
                &[(ChorusParam::Mix.index(), 30.0)],
            ),
        ];
        if self.delay {
            p.fx.push(delay_module());
        }

        let mut routes = vec![
            // Audio: the sine phase-modulates the wavetable, at full depth; `pm_index` scales it.
            ("osc1", "wt1.pm", 100.0),
            // The growl: the MSEG moves the PM index (+4 rad of 0-10) and the table position (+25 %) once per eighth.
            ("mseg1", "wt1.pm_index", 40.0),
            // Harder notes are more distorted.
            ("velocity", "dist1.drive", 8.0),
        ];
        if self.position_mod {
            routes.insert(2, ("mseg1", "wt1.position", 25.0));
        }
        if self.ladder {
            routes.push(("mseg1", "ladder1.cutoff", 15.0));
        }
        if self.formant {
            // The vowel: the LFO swings F2 from the "o" towards the "i".
            routes.push(("lfo1", "formant1.f2", 30.0));
        }
        for (src, dst, depth) in routes {
            p.routes.push(Route {
                src: src.parse().expect("source"),
                dst: dst.parse().expect("destination"),
                depth,
            });
        }
        p
    }
}

/// The `delay`: a synced dotted eighth, ping-pong, 30 % feedback, 25 % mix.
fn delay_module() -> ModuleInstance {
    use enth_patch::catalogue::DelayParam;
    module(
        "delay1",
        ModuleKind::Delay,
        &[
            (DelayParam::Feedback.index(), 30.0),
            (DelayParam::PingPong.index(), 1.0),
            (DelayParam::Mix.index(), 25.0),
        ],
    )
}

/// The G2 reference patch (4 voices).
pub fn g2_patch() -> Patch {
    G2::default().patch()
}

/// The stress case's patch (8 voices).
pub fn stress_patch() -> Patch {
    G2::stress().patch()
}

impl G2 {
    /// The variant with `wt1`'s level at 90 %: the same modules, so the swap is instant.
    pub fn value_edit(self) -> Patch {
        let mut p = self.patch();
        p.generators[1].values[WtParam::Level.index()] = 90.0;
        p
    }

    /// The variant with an unrouted voice LFO added: a module added, so the swap crossfades (G1's `module_added`).
    pub fn module_added(self) -> Patch {
        let mut p = self.patch();
        p.modulators.push(module(
            "lfo2",
            ModuleKind::Lfo,
            &[(LfoParam::Rate.index(), 2.0)],
        ));
        p
    }
}

/// The reference with `wt1`'s level at 90 %: the same modules, so the swap is instant.
pub fn value_edit() -> Patch {
    G2::default().value_edit()
}

/// The reference with an unrouted voice LFO added: a module added, so the swap crossfades.
pub fn module_added() -> Patch {
    G2::default().module_added()
}
