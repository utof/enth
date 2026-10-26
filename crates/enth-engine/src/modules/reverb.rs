//! The `reverb` module: the engine wrapper around `enth_dsp::reverb::Reverb` (P1a lane FXW; the core is lane FX1's
//! port of Signalsmith Basics' FDN reverb, `docs/dsp/reverb.md`).
//!
//! A global processor on the stereo bus: zero latency, `Reverb::prepare` in [`Reverb::new`] is the only allocation
//! (1,638,400 bytes at 44.1 and 48 kHz, sized from the sample rate alone), and one call per 32-frame segment,
//! which is the block length the core must see: its parameter fade and filter design run once per call, so the
//! segment is what makes a render independent of the host block size.
//!
//! # Parameters
//!
//! Each segment, [`core_params`] turns the catalogue's values (`GlobalParams::cur`) into the core's units and
//! `set_params` hands them over (cheap and idempotent: nothing happens unless a value changed):
//!
//! | Catalogue | Core | Conversion |
//! | --- | --- | --- |
//! | `dry`, `wet` (dB) | `dry_pct`, `wet_pct` | `100 · 10^(dB / 20)`; **−60 dB, the minimum, is off** (0 %) |
//! | `decay` (RT60, ms) | `rt20_s` | `decay / 3000`: RT20 in seconds, a third of the RT60 |
//! | `room`, `early`, `detune`, `low_cut`, `high_cut`, `low_damp`, `high_damp` | the same fields | as they are |
//!
//! Who smooths what: the core cross-fades `dry`, `wet`, `room`, `decay`, `early` and `detune` itself over 20 ms (the
//! catalogue gives them no engine smoothing; a second ramp would only add lag), and `room` and `decay` are not
//! modulatable because a value that changes every segment restarts that fade every 20 ms. The core does **not** fade
//! its cut and damping filters, so the engine smooths `low_cut`, `high_cut`, `low_damp` and `high_damp` (20 ms
//! linear, the catalogue's `Smoothing`) and this wrapper just passes each segment's `cur`.
//!
//! # Real-time safety
//!
//! - **NaN and infinity never reach the network.** The core does not sanitise its input, and one such sample would
//!   poison the feedback until `reset()`. Each segment is checked first (about 8 instructions per frame): NaN
//!   becomes 0 and anything beyond ±1e6 (+120 dBFS, far above any signal) is clamped.
//! - **Idle detection** (see [`GlobalProcessor::is_silent`] below): once the input and the output have been exactly
//!   zero for a while, the wrapper scans the network once, and if it holds only zeros it stops running the core.
//!   `process` of an idle wrapper with silent input returns at once, so skipping the call (as the engine does)
//!   gives the same bits as making it. The first non-silent input wakes the core with
//!   [`restart_from_silence`](enth_dsp::reverb::Reverb::restart_from_silence): pending parameters apply at once,
//!   and the detune LFO restarts at phase 0 (otherwise its phase would depend on how many segments were skipped).
//!
//! # Flags for the global FX bypass (lane S2b)
//!
//! The reverb is `time_based` in the catalogue (`ModuleKind::Reverb.spec().time_based`; the compressor is not). A
//! bypassed module is faded out by the engine, then not called; `reset()` it before fading it back in (the
//! `GlobalProcessor::reset` below clears the tail and re-reads the parameters).

use enth_dsp::reverb::{Reverb as Core, ReverbParams};
use enth_dsp::util::math;
use enth_patch::catalogue::ReverbParam;

use crate::module::{Block, GlobalCx, GlobalProcessor, GlobalStart, PrepareCfg, SEG};

/// The catalogue's `dry`/`wet` minimum, in dB: the fader at the bottom, mapped to gain 0.
pub const OFF_DB: f32 = -60.0;

/// `decay` (the RT60 in ms) divided by this is the core's RT20 in seconds: `ms / 1000` seconds, and the fall to
/// −20 dB takes a third of the fall to −60 dB.
pub const RT60_MS_PER_RT20_S: f64 = 3_000.0;

/// Input beyond this magnitude (+120 dBFS) is clamped before it reaches the network; NaN becomes 0.
const INPUT_LIMIT: f32 = 1.0e6;

/// Zero frames (input and output) before the network is first scanned for idleness: 0.68 s at 48 kHz. The last
/// state outlives the last nonzero output sample by up to a loop or two (measured at the default settings: 0.2 to
/// 0.7 s), so an earlier scan mostly fails; a scan costs about 40 µs (the whole 1.6 MB network).
const SCAN_FIRST: u32 = 32_768;
/// The longest wait between two scans of a network that still holds a little state (22 s at 48 kHz).
const SCAN_MAX: u32 = 1 << 20;

/// A gain in dB as the core's percentage of unity gain: `100 · 10^(dB / 20)`, exactly 0 at [`OFF_DB`] and below.
/// 0 dB is exactly 100.
#[must_use]
pub fn db_to_percent(db: f32) -> f64 {
    if db <= OFF_DB {
        0.0
    } else {
        100.0 * math::pow(10.0, f64::from(db) / 20.0)
    }
}

/// The core's parameters for the catalogue values `read` returns (in the catalogue's units). The core clamps them
/// into its own ranges, which contain the catalogue's.
pub fn core_params(read: impl Fn(ReverbParam) -> f32) -> ReverbParams {
    let f = |p| f64::from(read(p));
    ReverbParams {
        dry_pct: db_to_percent(read(ReverbParam::Dry)),
        wet_pct: db_to_percent(read(ReverbParam::Wet)),
        room_ms: f(ReverbParam::Room),
        rt20_s: f(ReverbParam::Decay) / RT60_MS_PER_RT20_S,
        early_pct: f(ReverbParam::Early),
        detune: f(ReverbParam::Detune),
        low_cut_hz: f(ReverbParam::LowCut),
        high_cut_hz: f(ReverbParam::HighCut),
        low_damp_rate: f(ReverbParam::LowDamp),
        high_damp_rate: f(ReverbParam::HighDamp),
    }
}

/// The reverb.
#[derive(Debug)]
pub struct Reverb {
    core: Core,
    /// The network holds only zeros and has been put to rest: silent input gives silent output and changes nothing,
    /// so `process` returns at once. Set by [`GlobalProcessor::reset`] and by a successful scan.
    idle: bool,
    /// Frames in a row for which both the input and the output of the core were exactly zero.
    quiet: u32,
    /// The `quiet` count at which the network is scanned next.
    scan_at: u32,
}

impl Reverb {
    /// Prepare a reverb: allocates the delay lines (sized from `cfg.sample_rate`) and applies the patch `values`
    /// at once (no fade at the start). Starts idle: a fresh network holds only zeros.
    #[must_use]
    pub fn new(values: &[f32], cfg: &PrepareCfg) -> Self {
        let mut core = Core::prepare(cfg.sample_rate);
        core.set_params(&core_params(|p| value(values, p)));
        Self {
            core,
            idle: true,
            quiet: 0,
            scan_at: SCAN_FIRST,
        }
    }

    /// The core, for tests and tools (`allocated_bytes`, `tail_samples`).
    #[must_use]
    pub fn core(&self) -> &Core {
        &self.core
    }
}

/// One of the patch `values`, or the catalogue default if the slice is too short (`values` is valid by contract;
/// this keeps a short slice from panicking in `new`).
fn value(values: &[f32], p: ReverbParam) -> f32 {
    values.get(p.index()).copied().unwrap_or(p.spec().default)
}

/// Replace NaN by 0 and clamp the rest to ±[`INPUT_LIMIT`]; a block that is already fine is left untouched (the
/// comparison is false for NaN, so NaN always takes the slow path).
#[inline]
fn sanitize(block: &mut Block) {
    if block.iter().all(|x| x.abs() <= INPUT_LIMIT) {
        return;
    }
    for x in block.iter_mut() {
        *x = if x.is_nan() {
            0.0
        } else {
            x.clamp(-INPUT_LIMIT, INPUT_LIMIT)
        };
    }
}

/// Whether every sample is exactly zero (`-0.0` counts).
#[inline]
fn is_zero(block: &Block) -> bool {
    block.iter().all(|x| *x == 0.0)
}

impl GlobalProcessor for Reverb {
    /// Silence the tail and read the patch values (`base`): a fresh start, or an un-bypass. The next block applies
    /// the parameters with no fade. Does not allocate (a 1.6 MB clear).
    fn reset(&mut self, start: &GlobalStart<'_>) {
        self.core.reset();
        self.core.set_params(&core_params(|p| start.params.base(p)));
        self.idle = true;
        self.quiet = 0;
        self.scan_at = SCAN_FIRST;
    }

    fn process(&mut self, cx: &mut GlobalCx<'_>) {
        sanitize(cx.left);
        sanitize(cx.right);
        let input_silent = is_zero(cx.left) && is_zero(cx.right);
        if self.idle {
            if input_silent {
                return;
            }
            self.core.restart_from_silence();
            self.idle = false;
        }
        self.core.set_params(&core_params(|p| cx.params.cur(p)));
        self.core.process_in_place(cx.left, cx.right);

        if input_silent && is_zero(cx.left) && is_zero(cx.right) {
            self.quiet = self.quiet.saturating_add(SEG as u32);
            if self.quiet >= self.scan_at {
                if self.core.is_state_zero() {
                    self.idle = true;
                } else {
                    self.scan_at = self.scan_at.saturating_mul(2).min(SCAN_MAX);
                }
            }
        } else {
            self.quiet = 0;
            self.scan_at = SCAN_FIRST;
        }
    }

    /// `true` once the network provably holds only zeros (after `reset`, or when a tail has died out exactly: the
    /// core flushes below 1e-15). Then `process` with silent input outputs silence and changes nothing, whether
    /// the engine calls it or skips it, so the skip is bit-identical to running.
    fn is_silent(&self) -> bool {
        self.idle
    }

    fn copy_state_from(&mut self, other: &Self) {
        if self.core.copy_state_from(&other.core) {
            self.idle = other.idle;
            self.quiet = other.quiet;
            self.scan_at = other.scan_at;
        }
    }

    /// An instant plan swap (P1b lane E+): exchange the whole core, an O(1) move of the 1.6 MB of delay lines
    /// instead of `copy_state_from`'s copy (research report 03: a swap block went from up to 997 µs to 149 µs).
    /// Every field the copy leaves alone (the ring sizes and masks, the detune depth, the mixer and interpolator
    /// tables) is a constant of the sample rate, so for the same sample rate the exchange is exactly the copy.
    /// `other` is left with this module's old core, which the main thread drops with the old plan.
    fn take_state_from(&mut self, other: &mut Self) {
        if self.core.sample_rate().to_bits() == other.core.sample_rate().to_bits() {
            core::mem::swap(&mut self.core, &mut other.core);
            self.idle = other.idle;
            self.quiet = other.quiet;
            self.scan_at = other.scan_at;
        }
    }

    fn subnormals(&self) -> usize {
        self.core.count_subnormals()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::module::GlobalParams;
    use enth_patch::ModuleKind;

    fn defaults() -> Vec<f32> {
        ModuleKind::Reverb.defaults()
    }

    #[test]
    fn minus_60_db_is_off_and_0_db_is_exactly_unity() {
        assert_eq!(db_to_percent(-60.0).to_bits(), 0.0f64.to_bits());
        assert_eq!(db_to_percent(-1000.0).to_bits(), 0.0f64.to_bits());
        assert_eq!(db_to_percent(0.0).to_bits(), 100.0f64.to_bits());
        // Just above the bottom is a real (tiny) gain, not zero: 0.1 %.
        let g = db_to_percent(-59.9);
        assert!((0.1..0.1012).contains(&g), "{g}");
        // The catalogue's other landmarks.
        assert!((db_to_percent(-6.0) - 50.118_723).abs() < 1e-5);
        assert!((db_to_percent(6.0) - 199.526_231).abs() < 1e-5);
        assert!((db_to_percent(-20.0) - 10.0).abs() < 1e-12);
        assert!((db_to_percent(20.0) - 1000.0).abs() < 1e-9);
    }

    #[test]
    fn the_off_level_is_the_catalogues_minimum() {
        for p in [ReverbParam::Dry, ReverbParam::Wet] {
            assert_eq!(
                p.spec().min().to_bits(),
                OFF_DB.to_bits(),
                "{}",
                p.spec().key
            );
        }
    }

    #[test]
    fn decay_is_the_rt60_in_ms_and_the_core_wants_the_rt20_in_seconds() {
        let at = |decay: f32| {
            let mut v = defaults();
            v[ReverbParam::Decay.index()] = decay;
            core_params(|p| v[p.index()]).rt20_s
        };
        assert_eq!(at(3_000.0), 1.0); // the catalogue's default is the core's default
        assert_eq!(at(30_000.0), 10.0);
        assert!((at(100.0) - 1.0 / 30.0).abs() < 1e-15);
        // The catalogue's range 100 ms .. 30 s is inside the core's RT20 range 0.01 .. 30 s.
        let spec = ReverbParam::Decay.spec();
        assert!(at(spec.min()) >= ReverbParams::MIN.rt20_s);
        assert!(at(spec.max()) <= ReverbParams::MAX.rt20_s);
    }

    #[test]
    fn every_catalogue_value_lands_in_its_core_field() {
        // A distinct value per parameter, so a crossed wire shows.
        let mut v = defaults();
        for (p, x) in [
            (ReverbParam::Dry, -20.0),
            (ReverbParam::Wet, -12.0),
            (ReverbParam::Room, 123.0),
            (ReverbParam::Decay, 4_500.0),
            (ReverbParam::Early, 77.0),
            (ReverbParam::Detune, 9.0),
            (ReverbParam::LowCut, 222.0),
            (ReverbParam::HighCut, 5_555.0),
            (ReverbParam::LowDamp, 3.5),
            (ReverbParam::HighDamp, 6.5),
        ] {
            v[p.index()] = x;
        }
        let c = core_params(|p| v[p.index()]);
        assert!((c.dry_pct - 10.0).abs() < 1e-12);
        assert!((c.wet_pct - 25.118_864).abs() < 1e-5);
        assert_eq!(c.room_ms, 123.0);
        assert_eq!(c.rt20_s, 1.5);
        assert_eq!(c.early_pct, 77.0);
        assert_eq!(c.detune, 9.0);
        assert_eq!(c.low_cut_hz, 222.0);
        assert_eq!(c.high_cut_hz, 5_555.0);
        assert_eq!(c.low_damp_rate, 3.5);
        assert_eq!(c.high_damp_rate, 6.5);
    }

    #[test]
    fn the_catalogue_defaults_are_the_cores_defaults_except_the_wet_level() {
        let c = core_params(|p| defaults()[p.index()]);
        let d = ReverbParams::default();
        assert_eq!(c.dry_pct, d.dry_pct); // 0 dB = 100 %
        assert!((c.wet_pct - 50.118_723).abs() < 1e-5); // -6 dB against the core's 50 %
        assert_eq!(
            (c.room_ms, c.rt20_s, c.early_pct, c.detune),
            (d.room_ms, d.rt20_s, d.early_pct, d.detune)
        );
        assert_eq!(
            (
                c.low_cut_hz,
                c.high_cut_hz,
                c.low_damp_rate,
                c.high_damp_rate
            ),
            (
                d.low_cut_hz,
                d.high_cut_hz,
                d.low_damp_rate,
                d.high_damp_rate
            )
        );
    }

    #[test]
    fn the_whole_catalogue_range_is_inside_the_cores_ranges() {
        // The core clamps; a catalogue value must never be clamped.
        for &p in ReverbParam::ALL {
            let spec = p.spec();
            for x in [spec.min(), spec.default, spec.max()] {
                let mut v = defaults();
                v[p.index()] = x;
                let c = core_params(|q| v[q.index()]);
                assert_eq!(c, c.clamped(), "{} = {x}", spec.key);
            }
        }
    }

    #[test]
    fn nan_and_out_of_range_catalogue_values_do_not_panic() {
        for x in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -1e30, 1e30] {
            let v = [x; 10];
            let _ = core_params(|p| v[p.index()]).clamped();
        }
    }

    #[test]
    fn sanitize_zeroes_nan_and_clamps_infinity_and_leaves_the_rest_alone() {
        let mut b: Block = [0.25; SEG];
        let before = b;
        sanitize(&mut b);
        assert_eq!(b, before);
        b[3] = f32::NAN;
        b[4] = f32::INFINITY;
        b[5] = f32::NEG_INFINITY;
        b[6] = 2.0e6;
        b[7] = -0.0;
        sanitize(&mut b);
        assert_eq!(b[3], 0.0);
        assert_eq!(b[4].to_bits(), INPUT_LIMIT.to_bits());
        assert_eq!(b[5].to_bits(), (-INPUT_LIMIT).to_bits());
        assert_eq!(b[6].to_bits(), INPUT_LIMIT.to_bits());
        assert_eq!(b[0], 0.25);
        assert_eq!(b[7].to_bits(), (-0.0f32).to_bits());
    }

    #[test]
    fn a_short_values_slice_falls_back_to_the_defaults() {
        let cfg = PrepareCfg::new(48_000.0);
        let r = Reverb::new(&[], &cfg);
        assert!(r.is_silent());
        assert_eq!(r.core().allocated_bytes(), 1_638_400);
    }

    fn views(vals: &[f32]) -> GlobalParams<'_> {
        GlobalParams::from_slices(vals, vals, vals)
    }

    #[test]
    fn a_fresh_reverb_is_idle_and_skipping_silence_changes_nothing() {
        let cfg = PrepareCfg::new(48_000.0);
        let vals = defaults();
        let mut r = Reverb::new(&vals, &cfg);
        assert!(r.is_silent());
        let (mut l, mut rr) = ([0.0; SEG], [0.0; SEG]);
        for _ in 0..10 {
            r.process(&mut GlobalCx {
                tempo_bpm: crate::tempo::DEFAULT_TEMPO_BPM,
                params: views(&vals),
                left: &mut l,
                right: &mut rr,
            });
        }
        assert!(r.is_silent());
        assert!(l.iter().chain(&rr).all(|x| *x == 0.0));
        // A click wakes it and the tail follows.
        l[0] = 1.0;
        r.process(&mut GlobalCx {
            tempo_bpm: crate::tempo::DEFAULT_TEMPO_BPM,
            params: views(&vals),
            left: &mut l,
            right: &mut rr,
        });
        assert!(!r.is_silent());
    }
}
