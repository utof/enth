//! **The stereo voice path: lane SVP's seam file** (S2a; created empty by lane S0d at freeze F0m, built by lane SVP).
//!
//! Until S2a a voice was mono from the generator sum to the pan stage, so the per-voice effects ran once and
//! `voice.pan` placed one signal in the stereo sum. Unison spread (lane UNI) and a stereo voice effect need two
//! channels in between. This file is that bus. Its edits to the shared files (`plan.rs`, `engine.rs`) are call
//! sites, so lane RTE's audio routes ([`crate::routing`]) never meet it in the same lines (docs/phase-s2.md,
//! "Rules for every lane", the hotspot table). The architecture as built is in docs/dsp/engine.md, "Stereo voice
//! path".
//!
//! # The flag
//!
//! A plan carries one `stereo` flag, set when it is compiled ([`compile_with`](crate::plan::compile_with)):
//!
//! - **[`StereoMode::Auto`]** (what [`compile`](crate::plan::compile) uses): the flag is [`patch_needs_stereo`],
//!   which is `false` for every patch today, because no module yet makes a per-voice stereo signal. Lane UNI turns
//!   it on for a `wt` whose unison spread is above 0. While the flag is clear, **today's mono path runs untouched
//!   and bit for bit**: no right-channel instance exists, no bus is copied, and the engine takes one branch per
//!   active voice and segment to learn that.
//! - **[`StereoMode::Forced`]** sets it for any patch. It is the test and bench hook (no catalogue change, no
//!   parameter): the 73 goldens render through it in `tests/golden/tests/stereo_path.rs`.
//!
//! Flipping the flag is a recompile (a new plan), never an edit of a running one.
//!
//! # What runs when it is set
//!
//! 1. **The generators** sum into the left voice bus exactly as before. [`StereoPath::split`] then copies it to the
//!    right bus for every active voice: a mono source is "the same signal on both channels". When a stereo
//!    generator exists (lane UNI), it writes its right channel to that bus, and `split` is the only function that
//!    changes.
//! 2. **Every `voice_fx` module is instantiated twice.** The left instance is the plan's own, running on the left
//!    bus exactly as in the mono path. The right instance ([`StereoPath::process`]) is built in `compile` from the
//!    same patch values, reads **the same parameter span** (so one set of knobs, one set of smoothers, one set of
//!    modulation routes, and no new `ParamId`) and runs on the right bus. State is separate. Two modules of one
//!    kind keep separate state per channel as they do in the mono path (S2 rule 3).
//! 3. **Seeds are derived from the module id and the channel** ([`channel_key`]): the left instance keeps the mono
//!    key, `fnv1a64(id)`, so its seed is the one the mono path would give; the right one gets
//!    `derive(key, `[`RIGHT_STREAM`]`)` in place of the key, in `VoiceStart::seed` (the voice rule) and in
//!    `VoiceStart::module_key` (the `seeded` phase mode's rule). No voice effect reads its seed today, so the
//!    channels are bit-equal; a future one that does would decorrelate its right channel, which is what a stereo
//!    effect wants (a module that must match mono at spread 0 uses no seeded randomness in its audio path).
//! 4. **The amp-and-balance stage** ([`mix_voice`]) replaces the mono pan stage for the voice. See below.
//!
//! # The balance law
//!
//! `voice.pan` (−100…+100 %) acts on a stereo voice as **a constant-power balance**: each channel is scaled by the
//! mono pan law's gain for that side, `(gl, gr) = (cos θ, sin θ)` with `θ = (pan + 1)·π/4`
//! ([`constant_power_pan`](enth_dsp::util::math::constant_power_pan)), never mixed into the other channel:
//!
//! ```text
//! out_l = x_l · amp · gl        out_r = x_r · amp · gr
//! ```
//!
//! - **A mono source panned (`x_l = x_r = x`) is bit-identical to the mono path.** This is the requirement that
//!   fixes the law: stereo with zero spread must equal mono on both channels, so the gains must be the mono law's
//!   gains and the products must be formed the same way (the same ramps, in the same order; see [`mix_voice`]).
//!   That is also why the centre is −3 dB per channel and not unity as in a console's balance knob: a unity centre
//!   would make a stereo voice 3 dB louder than the same voice with the flag clear, and the flag would change the
//!   level of a patch.
//! - **Hard left (−100 %) passes the left channel at unity and silences the right; hard right the opposite.** This
//!   is a balance, not a pan: a wide voice is narrowed by pushing it to a side, and its far channel is dropped.
//! - **For two uncorrelated channels of equal power the total power is constant over the pan range**
//!   (`cos²θ + sin²θ = 1`), as the pan law gives a mono voice.
//! - The gains are the same cached, ramped pair the mono stage keeps (`plan::Output`); the swap carries them.
//!
//! # Swap carry
//!
//! [`StereoPath::carry_from`] runs inside `Plan::carry_from` (audio thread, never allocating):
//!
//! | old plan | new plan | what the right instances do | crossfade |
//! | --- | --- | --- | --- |
//! | stereo | stereo | each pairs with the old right instance of the same (id, kind) and copies its state; one with no partner starts every active voice (right seeds) | as for any edit: only if a module was added or removed |
//! | mono | stereo | each copies the **old left** instance's state: before the swap the signal was mono, so the right channel continues exactly as the left was | **yes** |
//! | stereo | mono | the right instances are dropped with the old plan | **yes** |
//! | mono | mono | nothing; the lists are empty | as before |
//!
//! A flag change counts as a structural change: the swap crossfades (256 frames, an equal-gain raised cosine), the
//! old plan keeps running its own path, and the output is `old + (new − old)·w`. So turning the flag on or off
//! cannot click: with mono generators the two plans give the same bits and the fade is the identity; when the
//! stereo content really differs (lane UNI) the fade carries the change over 256 frames instead of in one sample.
//! The right instances of the new plan are built in `compile` on the main thread, so a swap allocates nothing; the
//! old plan's go back with it on the return ring and are freed on the main thread.
//!
//! # Tests
//!
//! Here: the key rule, the balance arithmetic, the right chain's independence from the left, and the carry of right
//! state (stereo to stereo from the old right, mono to stereo from the old left). In `tests/stereo.rs`: stereo with
//! zero spread equals mono on both channels (G1, every voice mode, every voice effect kind, pans, steals, glide,
//! routes, at several host blocks), swap carry for an unchanged flag and for both toggles, and `assert_no_alloc`
//! over rendering and swapping. In `tests/golden/tests/stereo_path.rs`: the flag is clear for the 73 goldens, and
//! each of them renders the same bits with the flag forced on. `benches/stereo.rs`: the cost.

use enth_dsp::util::rng::derive;
use enth_patch::{ModuleInstance, Patch};

use crate::module::{Block, NoteState, PrepareCfg, RAMP, VoiceBus, VoiceCx, VoiceIdx, VoiceStart};
use crate::modules::{ModuleImpl, VoiceProcessorImpl};
use crate::params::VoiceTable;
use crate::plan::{Slot, pair};
use crate::seed::voice_seed;

/// The stream that turns a module key into its right-channel key: `derive(key, RIGHT_STREAM)`. A change moves the
/// seed of every right instance (the left ones are the mono seeds and never move).
pub const RIGHT_STREAM: u64 = 0x5354_5245_4F5F_5231; // "STREO_R1"

/// The channel of a voice-effect instance.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Channel {
    /// The left channel: the plan's own instance, running as it does on the mono path.
    Left,
    /// The right channel: the second instance a stereo plan builds.
    Right,
}

/// The key that seeds one channel of a voice effect: the module key (`fnv1a64(id)`, [`crate::seed::module_key`])
/// for the left channel, `derive(key, `[`RIGHT_STREAM`]`)` for the right. Feed it to
/// [`voice_seed`](crate::seed::voice_seed) in place of the module key.
#[must_use]
pub const fn channel_key(module_key: u64, channel: Channel) -> u64 {
    match channel {
        Channel::Left => module_key,
        Channel::Right => derive(module_key, RIGHT_STREAM),
    }
}

/// How [`compile_with`](crate::plan::compile_with) decides a plan's stereo flag.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StereoMode {
    /// The flag is [`patch_needs_stereo`]: clear for every patch until lane UNI.
    #[default]
    Auto,
    /// The flag is set whatever the patch holds (tests and benches: the stereo path with mono content).
    Forced,
}

/// Whether `patch` has a module that makes a per-voice stereo signal, so its plan needs the stereo voice path.
///
/// **Always `false` today**: no generator or voice effect produces a stereo voice signal, so every existing patch
/// takes the mono path unchanged. Lane UNI replaces the body: a `wt` whose unison spread is above 0 (a function of
/// the patch's values only, never of a running plan).
#[must_use]
pub fn patch_needs_stereo(_patch: &Patch) -> bool {
    false
}

/// A plan's stereo half: the flag and the right-channel instances of its `voice_fx` chain.
pub(crate) struct StereoPath {
    on: bool,
    /// The right instance of each `Plan::voice_fx` slot, in the same order, with the same id, key and parameter
    /// span. Empty unless `on`.
    fx_r: Vec<Slot<VoiceProcessorImpl>>,
}

impl StereoPath {
    /// The path for `patch` under `mode`. Allocates nothing yet: [`StereoPath::mirror`] fills it while the plan is
    /// compiled.
    pub(crate) fn new(mode: StereoMode, patch: &Patch) -> Self {
        let on = match mode {
            StereoMode::Auto => patch_needs_stereo(patch),
            StereoMode::Forced => true,
        };
        Self {
            on,
            fx_r: Vec::new(),
        }
    }

    /// Whether the plan runs the stereo voice path.
    #[inline(always)]
    pub(crate) fn is_on(&self) -> bool {
        self.on
    }

    /// While compiling: when the flag is set, build the right instance of the `voice_fx` module `left` (made from
    /// `m`). Allocates (the module's own buffers), which is why it happens in `compile`.
    pub(crate) fn mirror(
        &mut self,
        left: &Slot<VoiceProcessorImpl>,
        m: &ModuleInstance,
        cfg: PrepareCfg,
    ) {
        if !self.on {
            return;
        }
        // The kind builds a `VoiceProcessor` (it just did, for `left`), so the `else` cannot happen.
        if let ModuleImpl::VoiceProcessor(module) = ModuleImpl::new(m.kind, &m.values, &cfg) {
            self.fx_r.push(Slot {
                module,
                id: left.id.clone(),
                key: left.key,
                params: left.params,
            });
        }
    }

    /// Step 7, after the generator sum: the right bus starts as a copy of the left for every active voice. Only a
    /// stereo generator (lane UNI) will make the channels differ here.
    #[inline]
    pub(crate) fn split(active: &[VoiceIdx], left: &VoiceBus, right: &mut VoiceBus) {
        for &v in active {
            right[v] = left[v];
        }
    }

    /// Step 7, after the left `voice_fx` chain: the right chain, in the same order, on the right bus.
    pub(crate) fn process(
        &mut self,
        active: &[VoiceIdx],
        notes: &NoteState,
        table: &VoiceTable,
        io: &mut VoiceBus,
    ) {
        for slot in &mut self.fx_r {
            slot.module.process(&mut VoiceCx {
                voices: active,
                notes,
                params: table.view(slot.params),
                io: &mut *io,
                sync_out: None,
                sync_in: None,
            });
        }
    }

    /// A fresh voice in slot `v` (its notes already set): start it on every right instance with the right
    /// channel's seed.
    pub(crate) fn start_voice(
        &mut self,
        v: VoiceIdx,
        notes: &NoteState,
        table: &VoiceTable,
        render_seed: u64,
    ) {
        for slot in &mut self.fx_r {
            slot.module
                .start_voice(&right_start(slot, v, notes, table, render_seed));
        }
    }

    /// Swap carry (the module docs have the table): the new plan's right instances take their state from `old`.
    /// `left` and `old_left` are the new and the old plan's `voice_fx` lists; `start` is what a fresh voice needs.
    /// The parameters need no carry: the right instances share the left ones' span, which `Plan::carry_from`
    /// carries. No allocation.
    pub(crate) fn carry_from(
        &mut self,
        left: &[Slot<VoiceProcessorImpl>],
        old_left: &[Slot<VoiceProcessorImpl>],
        old: &StereoPath,
        start: &StartCx<'_>,
    ) {
        if self.fx_r.is_empty() {
            return;
        }
        let (pairs, _) = pair(left, old_left, VoiceProcessorImpl::kind);
        for (i, slot) in self.fx_r.iter_mut().enumerate() {
            match pairs[i] {
                // Both stereo: the old right instance of the same module; the old plan was mono: the signal was
                // the left one, so the right channel continues from the left instance's state.
                Some(j) if old.on => {
                    slot.module.copy_state(&old.fx_r[j].module);
                }
                Some(j) => {
                    slot.module.copy_state(&old_left[j].module);
                }
                None => {
                    for &v in start.active {
                        slot.module.start_voice(&right_start(
                            slot,
                            v,
                            start.notes,
                            start.table,
                            start.render_seed,
                        ));
                    }
                }
            }
        }
    }

    /// The number of subnormal values in the right instances' state (test support).
    pub(crate) fn subnormals(&self) -> usize {
        self.fx_r.iter().map(|s| s.module.subnormals()).sum()
    }
}

/// What starting a voice on a new plan's instance needs: the active voices, their notes, the new plan's voice table and
/// the render seed.
pub(crate) struct StartCx<'a> {
    pub active: &'a [VoiceIdx],
    pub notes: &'a NoteState,
    pub table: &'a VoiceTable,
    pub render_seed: u64,
}

fn right_start<'a>(
    slot: &Slot<VoiceProcessorImpl>,
    v: VoiceIdx,
    notes: &'a NoteState,
    table: &'a VoiceTable,
    render_seed: u64,
) -> VoiceStart<'a> {
    let key = channel_key(slot.key, Channel::Right);
    let note_index = notes.note_index[v];
    VoiceStart {
        voice: v,
        seed: voice_seed(render_seed, key, note_index),
        module_key: key,
        note_index,
        notes,
        params: table.view(slot.params),
    }
}

/// One voice's amp-and-balance gains for a segment, per channel: the value at the segment's start and the change
/// over it (`gain = g0 + dg·RAMP[i]`), exactly what the mono stage forms from the amp envelope and the pan gains.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Gains {
    pub l0: f32,
    pub dl: f32,
    pub r0: f32,
    pub dr: f32,
}

/// Step 7's amp stage and balance for one stereo voice: add `xl·gain_l` to `left` and `xr·gain_r` to `right`, with
/// the gains ramping per sample ([`Gains`]) and, for a stolen voice, times its linear fade `(f0, f1)` over the
/// segment. The arithmetic is the mono stage's, written once per channel in the same order, so `xl == xr` gives the
/// mono path's bits on both channels.
#[inline]
pub(crate) fn mix_voice(
    left: &mut Block,
    right: &mut Block,
    xl: &Block,
    xr: &Block,
    g: Gains,
    fade: Option<(f32, f32)>,
) {
    if let Some((f0, f1)) = fade {
        let df = f1 - f0;
        for ((((l, r), xl), xr), w) in left
            .iter_mut()
            .zip(right.iter_mut())
            .zip(xl)
            .zip(xr)
            .zip(RAMP)
        {
            let f = f0 + df * w;
            *l += xl * ((g.l0 + g.dl * w) * f);
            *r += xr * ((g.r0 + g.dr * w) * f);
        }
    } else {
        for ((((l, r), xl), xr), w) in left
            .iter_mut()
            .zip(right.iter_mut())
            .zip(xl)
            .zip(xr)
            .zip(RAMP)
        {
            *l += xl * (g.l0 + g.dl * w);
            *r += xr * (g.r0 + g.dr * w);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::module::{MAX_VOICES, SEG};
    use crate::plan::{Plan, compile, compile_with};
    use crate::seed::module_key;
    use enth_dsp::util::math::constant_power_pan;
    use enth_patch::catalogue::{FilterMode, SvfParam};
    use enth_patch::{ModuleId, ModuleKind};

    const SEED: u64 = 0x0E17_5EED;

    fn id(s: &str) -> ModuleId {
        ModuleId::new(s).unwrap()
    }

    /// `Patch::init()` plus a resonant low-pass and a high-pass in the voice chain.
    fn two_filters() -> Patch {
        let mut p = Patch::init();
        let mut f1 = ModuleInstance::new(id("filter1"), ModuleKind::Svf);
        f1.values[SvfParam::Mode.index()] = FilterMode::Lp.index() as f32;
        f1.values[SvfParam::Cutoff.index()] = 900.0;
        f1.values[SvfParam::Resonance.index()] = 70.0;
        let mut f2 = ModuleInstance::new(id("filter2"), ModuleKind::Svf);
        f2.values[SvfParam::Mode.index()] = FilterMode::Hp.index() as f32;
        f2.values[SvfParam::Cutoff.index()] = 200.0;
        p.voice_fx = vec![f1, f2];
        p
    }

    fn cfg() -> PrepareCfg {
        PrepareCfg::new(48_000.0)
    }

    fn notes() -> NoteState {
        let mut n = NoteState::new();
        n.pitch_prev = [60.0; MAX_VOICES];
        n.pitch_cur = [60.0; MAX_VOICES];
        n.velocity = [0.8; MAX_VOICES];
        n.gate = [true; MAX_VOICES];
        n.fresh[0] = true;
        n
    }

    /// A deterministic test signal in -1..1, different per `salt`.
    fn signal(k: usize, salt: usize) -> Block {
        core::array::from_fn(|i| {
            let x = (k * 131 + i * 17 + salt * 53) % 97;
            x as f32 / 48.0 - 1.0
        })
    }

    fn bits(b: &Block) -> [u32; SEG] {
        b.map(f32::to_bits)
    }

    /// Run plan `p`'s left `voice_fx` chain on `io` (voice 0).
    fn run_left(p: &mut Plan, n: &NoteState, io: &mut VoiceBus) {
        let Plan {
            voice_fx,
            voice_table,
            ..
        } = p;
        for slot in voice_fx.iter_mut() {
            slot.module.process(&mut VoiceCx {
                voices: &[0],
                notes: n,
                params: voice_table.view(slot.params),
                io: &mut *io,
                sync_out: None,
                sync_in: None,
            });
        }
    }

    fn run_right(p: &mut Plan, n: &NoteState, io: &mut VoiceBus) {
        let Plan {
            stereo,
            voice_table,
            ..
        } = p;
        stereo.process(&[0], n, voice_table, io);
    }

    fn bus(b: Block) -> Box<VoiceBus> {
        let mut bus = Box::new([[0.0; SEG]; MAX_VOICES]);
        bus[0] = b;
        bus
    }

    #[test]
    fn the_left_key_is_the_mono_key_and_the_right_one_is_derived() {
        let k = module_key(&id("filter1"));
        assert_eq!(channel_key(k, Channel::Left), k);
        let r = channel_key(k, Channel::Right);
        assert_ne!(r, k);
        assert_eq!(r, derive(k, RIGHT_STREAM));
        assert_ne!(r, channel_key(module_key(&id("filter2")), Channel::Right));
        // The rule is pinned: a change moves the seed of every right instance.
        assert_eq!(r, 0x3a9e_9a44_ce35_e25a);
        let n = 3;
        assert_ne!(voice_seed(SEED, r, n), voice_seed(SEED, k, n));
        assert_eq!(voice_seed(SEED, k, n), crate::seed::voice_seed(SEED, k, n));
    }

    #[test]
    fn a_right_instance_starts_a_voice_with_the_right_channels_key_and_seed() {
        let patch = two_filters();
        let (st, _) = compile_with(&patch, &cfg(), StereoMode::Forced).unwrap();
        let mut n = notes();
        n.note_index[0] = 7;
        for (l, r) in st.voice_fx.iter().zip(&st.stereo.fx_r) {
            let start = right_start(r, 0, &n, &st.voice_table, SEED);
            let key = channel_key(l.key, Channel::Right);
            assert_eq!(start.module_key, key);
            assert_eq!(start.seed, voice_seed(SEED, key, 7));
            assert_eq!(start.note_index, 7);
            // The left instance of the plan keeps the mono rule (`plan::voice_start`).
            assert_ne!(start.seed, voice_seed(SEED, l.key, 7));
            assert_ne!(start.module_key, l.key);
        }
        // Two effects of one kind get two different streams on each channel.
        let keys: Vec<u64> = st
            .stereo
            .fx_r
            .iter()
            .map(|r| channel_key(r.key, Channel::Right))
            .collect();
        assert_ne!(keys[0], keys[1]);
    }

    #[test]
    fn auto_leaves_the_flag_clear_and_forced_sets_it_with_one_right_instance_per_effect() {
        let p = two_filters();
        assert!(!patch_needs_stereo(&p));
        let (mono, _) = compile(&p, &cfg()).unwrap();
        assert!(!mono.is_stereo());
        assert!(
            mono.stereo.fx_r.is_empty(),
            "the mono path has no instances"
        );
        let (st, map) = compile_with(&p, &cfg(), StereoMode::Forced).unwrap();
        assert!(st.is_stereo());
        assert_eq!(st.stereo.fx_r.len(), st.voice_fx.len());
        for (l, r) in st.voice_fx.iter().zip(&st.stereo.fx_r) {
            assert_eq!((&l.id, l.key, l.params), (&r.id, r.key, r.params));
            assert_eq!(l.module.kind(), r.module.kind());
        }
        // One set of parameters: the map and the table are the mono plan's.
        let (_, mono_map) = compile(&p, &cfg()).unwrap();
        assert_eq!(map, mono_map);
        assert_eq!(st.voice_table.base.len(), mono.voice_table.base.len());
        assert_eq!(st.module_ids(), mono.module_ids());
    }

    #[test]
    fn the_right_chain_is_a_second_instance_on_its_own_bus() {
        let patch = two_filters();
        let n = notes();
        let (mut st, _) = compile_with(&patch, &cfg(), StereoMode::Forced).unwrap();
        let (mut ml, _) = compile(&patch, &cfg()).unwrap();
        let (mut mr, _) = compile(&patch, &cfg()).unwrap();
        for p in [&mut st, &mut ml, &mut mr] {
            p.start_voice(0, &n, SEED);
        }
        for k in 0..24 {
            let (xl, xr) = (signal(k, 1), signal(k, 2));
            let (mut bl, mut br) = (bus(xl), bus(xr));
            run_left(&mut st, &n, &mut bl);
            run_right(&mut st, &n, &mut br);
            let (mut rl, mut rr) = (bus(xl), bus(xr));
            run_left(&mut ml, &n, &mut rl);
            run_left(&mut mr, &n, &mut rr);
            assert_eq!(bits(&bl[0]), bits(&rl[0]), "left, segment {k}");
            assert_eq!(bits(&br[0]), bits(&rr[0]), "right, segment {k}");
            assert_ne!(
                bits(&bl[0]),
                bits(&br[0]),
                "different inputs, different outputs"
            );
        }
    }

    /// Run `k` more segments of different left and right inputs on `p`, to put different state in the channels.
    fn warm_up(p: &mut Plan, n: &NoteState, segments: usize) {
        for k in 0..segments {
            let (mut bl, mut br) = (bus(signal(k, 1)), bus(signal(k, 2)));
            run_left(p, n, &mut bl);
            if p.is_stereo() {
                run_right(p, n, &mut br);
            }
        }
    }

    #[test]
    fn a_stereo_to_stereo_swap_carries_each_channels_own_state() {
        let patch = two_filters();
        let n = notes();
        let (mut old, _) = compile_with(&patch, &cfg(), StereoMode::Forced).unwrap();
        old.start_voice(0, &n, SEED);
        warm_up(&mut old, &n, 12);
        let (mut new, _) = compile_with(&patch, &cfg(), StereoMode::Forced).unwrap();
        let changed = new.carry_from(&mut old, &[0], &n, SEED);
        assert!(!changed, "an unchanged flag and chain: no crossfade");
        let x = signal(99, 3);
        let (mut a_l, mut a_r) = (bus(x), bus(x));
        run_left(&mut old, &n, &mut a_l);
        run_right(&mut old, &n, &mut a_r);
        let (mut b_l, mut b_r) = (bus(x), bus(x));
        run_left(&mut new, &n, &mut b_l);
        run_right(&mut new, &n, &mut b_r);
        assert_eq!(bits(&b_l[0]), bits(&a_l[0]), "left continues");
        assert_eq!(bits(&b_r[0]), bits(&a_r[0]), "right continues");
        assert_ne!(
            bits(&a_l[0]),
            bits(&a_r[0]),
            "the channels held different state, so this is not vacuous"
        );
    }

    #[test]
    fn a_mono_to_stereo_swap_starts_the_right_channel_from_the_left_state() {
        let patch = two_filters();
        let n = notes();
        let (mut old, _) = compile(&patch, &cfg()).unwrap();
        old.start_voice(0, &n, SEED);
        warm_up(&mut old, &n, 12);
        let (mut new, _) = compile_with(&patch, &cfg(), StereoMode::Forced).unwrap();
        assert!(
            new.carry_from(&mut old, &[0], &n, SEED),
            "a flag change crossfades"
        );
        let x = signal(99, 3);
        let mut a = bus(x);
        run_left(&mut old, &n, &mut a);
        let (mut b_l, mut b_r) = (bus(x), bus(x));
        run_left(&mut new, &n, &mut b_l);
        run_right(&mut new, &n, &mut b_r);
        assert_eq!(bits(&b_l[0]), bits(&a[0]), "left continues");
        assert_eq!(
            bits(&b_r[0]),
            bits(&a[0]),
            "right continues as the left was"
        );
    }

    #[test]
    fn a_stereo_to_mono_swap_is_a_structural_change_and_a_new_effect_starts_both_channels() {
        let patch = two_filters();
        let n = notes();
        let (mut old, _) = compile_with(&patch, &cfg(), StereoMode::Forced).unwrap();
        old.start_voice(0, &n, SEED);
        let (mut mono, _) = compile(&patch, &cfg()).unwrap();
        assert!(mono.carry_from(&mut old, &[0], &n, SEED));
        // A chain with an effect the old plan lacked: the new right instance starts the active voices.
        let mut three = two_filters();
        three
            .voice_fx
            .push(ModuleInstance::new(id("filter3"), ModuleKind::Svf));
        let (mut new, _) = compile_with(&three, &cfg(), StereoMode::Forced).unwrap();
        let (mut old, _) = compile_with(&patch, &cfg(), StereoMode::Forced).unwrap();
        assert!(new.carry_from(&mut old, &[0], &n, SEED), "an added module");
        assert_eq!(new.stereo.fx_r.len(), 3);
        let x = signal(5, 4);
        let (mut b_l, mut b_r) = (bus(x), bus(x));
        run_left(&mut new, &n, &mut b_l);
        run_right(&mut new, &n, &mut b_r);
        assert_eq!(
            bits(&b_l[0]),
            bits(&b_r[0]),
            "fresh state on both channels, same input"
        );
    }

    /// The mono stage's arithmetic, as `render_plan` writes it.
    fn mono_stage(x: &Block, g: Gains, fade: Option<(f32, f32)>) -> (Block, Block) {
        let (mut l, mut r) = ([0.0; SEG], [0.0; SEG]);
        if let Some((f0, f1)) = fade {
            let df = f1 - f0;
            for (((l, r), x), w) in l.iter_mut().zip(r.iter_mut()).zip(x).zip(RAMP) {
                let f = f0 + df * w;
                *l += x * ((g.l0 + g.dl * w) * f);
                *r += x * ((g.r0 + g.dr * w) * f);
            }
        } else {
            for (((l, r), x), w) in l.iter_mut().zip(r.iter_mut()).zip(x).zip(RAMP) {
                *l += x * (g.l0 + g.dl * w);
                *r += x * (g.r0 + g.dr * w);
            }
        }
        (l, r)
    }

    fn gains(pan_from: f32, pan_to: f32, amp: (f32, f32)) -> Gains {
        let (ql, qr) = constant_power_pan(pan_from * 0.01);
        let (pl, pr) = constant_power_pan(pan_to * 0.01);
        let (l0, r0) = (amp.0 * ql, amp.0 * qr);
        Gains {
            l0,
            dl: amp.1 * pl - l0,
            r0,
            dr: amp.1 * pr - r0,
        }
    }

    #[test]
    fn equal_channels_through_the_balance_are_the_mono_pan_stage_bit_for_bit() {
        for (k, (from, to)) in [(0.0, 0.0), (-100.0, -100.0), (30.0, 65.0), (100.0, -20.0)]
            .into_iter()
            .enumerate()
        {
            for fade in [None, Some((1.0, 0.5)), Some((0.25, 0.0))] {
                let x = signal(k, 7);
                let g = gains(from, to, (0.3, 0.9));
                let (ml, mr) = mono_stage(&x, g, fade);
                let (mut l, mut r) = ([0.0; SEG], [0.0; SEG]);
                mix_voice(&mut l, &mut r, &x, &x, g, fade);
                assert_eq!(bits(&l), bits(&ml), "left, pan {from}..{to}, fade {fade:?}");
                assert_eq!(
                    bits(&r),
                    bits(&mr),
                    "right, pan {from}..{to}, fade {fade:?}"
                );
            }
        }
    }

    #[test]
    fn the_balance_scales_each_channel_and_never_mixes_them() {
        let (xl, xr) = (signal(1, 1), signal(1, 2));
        // Hard left passes the left channel at unity and silences the right; hard right the opposite.
        let hard_left = gains(-100.0, -100.0, (1.0, 1.0));
        let (mut l, mut r) = ([0.0; SEG], [0.0; SEG]);
        mix_voice(&mut l, &mut r, &xl, &xr, hard_left, None);
        assert_eq!(bits(&l), bits(&xl));
        assert!(r.iter().all(|x| *x == 0.0));
        let hard_right = gains(100.0, 100.0, (1.0, 1.0));
        let (mut l, mut r) = ([0.0; SEG], [0.0; SEG]);
        mix_voice(&mut l, &mut r, &xl, &xr, hard_right, None);
        assert!(l.iter().all(|x| *x == 0.0));
        assert_eq!(bits(&r), bits(&xr));
        // A silent right channel stays silent at every pan: nothing leaks across.
        let silent = [0.0; SEG];
        for pan in [-100.0, -40.0, 0.0, 25.0, 100.0] {
            let (mut l, mut r) = ([0.0; SEG], [0.0; SEG]);
            mix_voice(
                &mut l,
                &mut r,
                &xl,
                &silent,
                gains(pan, pan, (0.5, 0.5)),
                None,
            );
            assert!(r.iter().all(|x| *x == 0.0), "pan {pan}");
            // Hard right silences the left channel (a balance drops the far side).
            assert_eq!(l.iter().any(|x| *x != 0.0), pan < 100.0, "pan {pan}");
        }
        // At the centre each channel is the mono law's -3 dB.
        let centre = gains(0.0, 0.0, (1.0, 1.0));
        let (c, _) = constant_power_pan(0.0);
        assert_eq!(centre.l0.to_bits(), c.to_bits());
        assert_eq!(centre.r0.to_bits(), c.to_bits());
    }

    #[test]
    fn the_balance_keeps_the_power_of_uncorrelated_channels_constant_over_the_pan_range() {
        for pan in (-100..=100).step_by(10) {
            let (gl, gr) = constant_power_pan(pan as f32 * 0.01);
            let p = f64::from(gl) * f64::from(gl) + f64::from(gr) * f64::from(gr);
            assert!((p - 1.0).abs() < 1e-6, "pan {pan}: {p}");
        }
    }
}
