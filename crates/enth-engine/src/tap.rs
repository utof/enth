//! The telemetry tap (P1b lane E+): a read-only, lock-free, allocation-free window from the audio thread into the
//! engine, for the editor's knob rings, modulator tiles, voice count, statistics, scope and spectrum.
//!
//! ```text
//! audio thread (end of Engine::process)              any other thread (the editor)
//! publish: seq odd, relaxed stores of f32 bits,
//!          seq even (Release) ───────────▶ Tap ◀── read(): copy a consistent frame (pre-sized, no allocation)
//!          the block's mono mix ────────▶ scope ring ◀── scope(out): the newest samples, oldest first
//!          generation += 1 (SeqCst) ─────▶            ◀── generation() / sleep(seen) / take_wake()
//! ```
//!
//! - **Atomics, not a queue**: the editor wants the latest value, not every value, so each value is one atomic
//!   holding `f32` bits, overwritten by every publish. The scope is a ring of atomic samples the writer overwrites: a
//!   reader that falls behind loses the oldest samples. Nothing blocks and nothing allocates after [`Tap::new`].
//! - **A seqlock around the frame** (F1b review, issue 1): a publish brackets its stores with a version counter that
//!   is odd while it writes, and [`Tap::read`] keeps a copy only if the counter was even and unchanged around it, so a
//!   reader never pairs one plan's epoch or lengths with another plan's values across a swap. The writer never waits.
//!   The reader retries a bounded number of times and, if the writer stays mid-publish (the audio thread was
//!   preempted), returns a frame with no routed values and the generation it had, never a torn one and never after
//!   waiting for the audio thread.
//! - **Overflow drops, never blocks.** Capacities are fixed at construction ([`TapCapacity`]). Routed parameters or
//!   modulators beyond them are not published and are counted in [`TapFrame::dropped`]; a block longer than the
//!   scope ring keeps its newest samples.
//! - **Read-only**: publishing reads engine state and writes only the tap, so renders are the same bits with or
//!   without a tap (tested). `enth-render` never attaches one.
//! - **Quiet costs almost nothing**: the engine publishes only while the tap is enabled ([`Tap::set_enabled`]: the
//!   shell enables it while the editor is open and shown). Once the scope ring holds a full ring of silence and no
//!   voice plays, a block publishes only if a value the editor shows differs from the one the tap holds (the quiet
//!   rule, P1b lane ED2): a new plan's epoch, a global modulator still running, a macro or a parameter moved in
//!   silence, a value that changed while the tap was disabled. Such a publish leaves the silent scope ring alone.
//!   Otherwise the generation stops and the editor can sleep; the check is one pass over the tap's own last stores.
//! - **The wake handshake** (the UI spike's): the editor calls [`Tap::sleep`] with the last generation it drew; the
//!   next publish that brings new data sets a wake flag, which the shell takes with [`Tap::take_wake`] after
//!   `process` and turns into a repaint request (the engine cannot name the plugin framework's types).
//!
//! What is published, per `process` call (the latest segment's values):
//! - the plan's epoch (the order of the routed values below follows that plan:
//!   [`Plan::tap_params`](crate::Plan::tap_params) and [`Plan::tap_modulators`](crate::Plan::tap_modulators); a
//!   reader ignores values whose epoch it has no map for);
//! - the routed parameters' modulated values (`cur`) in plain units: voice-table ones for the *display voice*, global
//!   ones as they are; the modulator outputs (voice modulators for the display voice, then global ones);
//! - the display voice: the newest held voice (highest note index), else the newest releasing one, else none (then a
//!   voice-table value is its base, and a voice modulator's output 0);
//! - the active and held voice counts, the eight macros and the mod wheel (smoothed), the block's peaks, the engine's
//!   counters, and the block's mono mix `(L + R) / 2` into the scope ring.

use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering, fence};
use std::sync::Arc;

use crate::engine::EngineStats;
use crate::modulation::MACRO_SOURCES;

/// Sizes of a [`Tap`], fixed at construction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TapCapacity {
    /// Routed parameters published (more are dropped and counted).
    pub params: usize,
    /// Modulator outputs published (more are dropped and counted).
    pub modulators: usize,
    /// Scope ring length in samples, rounded up to a power of two (at least 32).
    pub scope: usize,
}

impl Default for TapCapacity {
    /// 64 routed parameters (a patch's route limit), 16 modulators, 4,096 scope samples (85 ms at 48 kHz; the
    /// spectrum reads the newest 2,048).
    fn default() -> Self {
        Self {
            params: 64,
            modulators: 16,
            scope: 4096,
        }
    }
}

/// No display voice.
const NO_VOICE: u32 = u32::MAX;

/// The [`EngineStats`] fields, in declaration order.
const STATS: usize = 10;

/// How many times [`Tap::read`] tries for a consistent frame before it gives up and shows nothing routed. A publish
/// takes a few microseconds and a try about a tenth of that, so only a preempted audio thread exhausts it.
const READ_TRIES: usize = 32;

/// The shared telemetry block. Build it with [`Tap::new`] (allocates), attach it to the engine with
/// [`Engine::attach_tap`](crate::Engine::attach_tap) on the main thread, keep a clone for the reader.
#[derive(Debug)]
pub struct Tap {
    enabled: AtomicBool,
    /// The seqlock's version: odd while a publish writes the frame, even between publishes ([`Tap::begin`]).
    seq: AtomicU64,
    generation: AtomicU64,
    sleeping: AtomicBool,
    wake: AtomicBool,
    pub(crate) epoch: AtomicU64,
    pub(crate) voices: AtomicU32,
    pub(crate) held: AtomicU32,
    pub(crate) display_voice: AtomicU32,
    pub(crate) display_note: AtomicU64,
    pub(crate) params: Box<[AtomicU32]>,
    pub(crate) params_len: AtomicU32,
    pub(crate) modulators: Box<[AtomicU32]>,
    pub(crate) modulators_len: AtomicU32,
    pub(crate) macros: [AtomicU32; MACRO_SOURCES],
    pub(crate) peaks: [AtomicU32; 2],
    pub(crate) stats: [AtomicU64; STATS],
    pub(crate) dropped: AtomicU64,
    scope: Box<[AtomicU32]>,
    scope_written: AtomicU64,
}

/// One copy of the tap's values, for the reader. Build it with [`Tap::frame`] (sized to the tap; allocates once) and
/// refill it with [`Tap::read`] (never allocates).
///
/// `non_exhaustive` (F1b review): fields are added without breaking readers. A fake telemetry source outside the
/// crate builds one with `TapFrame::default()` and sets the fields it needs.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct TapFrame {
    /// The generation the values were read at (0: nothing published yet).
    pub generation: u64,
    /// The epoch of the plan whose order `params` and `modulators` follow.
    pub epoch: u64,
    /// Playing voices (held, releasing or fading).
    pub voices: u32,
    /// Voices with their key down.
    pub held: u32,
    /// The voice slot the per-voice values describe, if any.
    pub display_voice: Option<u32>,
    /// The note index of that voice's note.
    pub display_note: u64,
    /// Routed parameter values (plain units), in [`Plan::tap_params`](crate::Plan::tap_params) order; the first
    /// `params_len` are valid.
    pub params: Vec<f32>,
    /// How many of `params` are valid.
    pub params_len: usize,
    /// Modulator outputs, in [`Plan::tap_modulators`](crate::Plan::tap_modulators) order; the first
    /// `modulators_len` are valid.
    pub modulators: Vec<f32>,
    /// How many of `modulators` are valid.
    pub modulators_len: usize,
    /// The eight macros and the mod wheel (index 8), smoothed, 0–1.
    pub macros: [f32; MACRO_SOURCES],
    /// The last published block's peak per channel.
    pub peaks: [f32; 2],
    /// The engine's counters.
    pub stats: EngineStats,
    /// Values that did not fit the tap's capacities, summed over every publish.
    pub dropped: u64,
}

/// Store `x`'s bits (relaxed): every value the tap holds.
pub(crate) fn store_f32(a: &AtomicU32, x: f32) {
    a.store(x.to_bits(), Ordering::Relaxed);
}

fn load_f32(a: &AtomicU32) -> f32 {
    f32::from_bits(a.load(Ordering::Relaxed))
}

impl Tap {
    /// A disabled tap with these capacities (allocates; main thread).
    #[must_use]
    pub fn new(capacity: TapCapacity) -> Arc<Self> {
        let atoms = |n: usize| (0..n).map(|_| AtomicU32::new(0)).collect::<Box<[_]>>();
        let scope = capacity.scope.max(32).next_power_of_two();
        Arc::new(Self {
            enabled: AtomicBool::new(false),
            seq: AtomicU64::new(0),
            generation: AtomicU64::new(0),
            sleeping: AtomicBool::new(false),
            wake: AtomicBool::new(false),
            epoch: AtomicU64::new(0),
            voices: AtomicU32::new(0),
            held: AtomicU32::new(0),
            display_voice: AtomicU32::new(NO_VOICE),
            display_note: AtomicU64::new(0),
            params: atoms(capacity.params),
            params_len: AtomicU32::new(0),
            modulators: atoms(capacity.modulators),
            modulators_len: AtomicU32::new(0),
            macros: core::array::from_fn(|_| AtomicU32::new(0)),
            peaks: core::array::from_fn(|_| AtomicU32::new(0)),
            stats: core::array::from_fn(|_| AtomicU64::new(0)),
            dropped: AtomicU64::new(0),
            scope: atoms(scope),
            scope_written: AtomicU64::new(0),
        })
    }

    /// Publish or not: the shell enables the tap while the editor is open and shown, and disables it otherwise, so a
    /// closed editor costs the audio thread one relaxed load per `process` call.
    pub fn set_enabled(&self, on: bool) {
        self.enabled.store(on, Ordering::Relaxed);
    }

    /// Whether the engine publishes.
    #[must_use]
    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    /// Bumped by every publish that brought new data; 0 before the first.
    #[must_use]
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }

    /// The editor goes to sleep having drawn generation `seen`: returns `false` (stay awake, draw again) if newer data
    /// already arrived. Otherwise the next publish with new data sets the wake flag ([`Tap::take_wake`]).
    pub fn sleep(&self, seen: u64) -> bool {
        self.sleeping.store(true, Ordering::SeqCst);
        if self.generation.load(Ordering::SeqCst) != seen {
            self.sleeping.store(false, Ordering::SeqCst);
            return false;
        }
        true
    }

    /// Whether a publish found the editor asleep since the last call (and clear it): the shell calls this after
    /// `process` and requests a repaint when it is `true`. Real-time safe.
    pub fn take_wake(&self) -> bool {
        self.wake.swap(false, Ordering::AcqRel)
    }

    /// The capacities this tap was built with (the scope rounded up).
    #[must_use]
    pub fn capacity(&self) -> TapCapacity {
        TapCapacity {
            params: self.params.len(),
            modulators: self.modulators.len(),
            scope: self.scope.len(),
        }
    }

    /// A frame sized for this tap, for [`Tap::read`] (allocates once; reader thread).
    #[must_use]
    pub fn frame(&self) -> TapFrame {
        TapFrame {
            params: vec![0.0; self.params.len()],
            modulators: vec![0.0; self.modulators.len()],
            ..TapFrame::default()
        }
    }

    /// Copy the latest values into `out` (built by [`Tap::frame`]; a smaller one gets what fits). Never allocates and
    /// never waits for the audio thread. The frame is one publish's: the epoch, the lengths and the values belong
    /// together, also across a plan swap (a seqlock, see the module docs). If the writer stays mid-publish for every
    /// try (it was preempted), `out` keeps its generation and comes back with no routed values and no modulators, so
    /// the reader draws nothing from it and reads again next frame.
    pub fn read(&self, out: &mut TapFrame) {
        let seen = out.generation;
        for _ in 0..READ_TRIES {
            let before = self.seq.load(Ordering::Acquire);
            if before & 1 == 0 {
                self.copy_into(out);
                // Order the value loads before the version check (Boehm's seqlock reader).
                fence(Ordering::Acquire);
                if self.seq.load(Ordering::Relaxed) == before {
                    return;
                }
            }
            core::hint::spin_loop();
        }
        out.generation = seen;
        out.params_len = 0;
        out.modulators_len = 0;
    }

    /// One unchecked copy of the frame: [`Tap::read`] keeps it only if the version did not move around it.
    fn copy_into(&self, out: &mut TapFrame) {
        out.generation = self.generation();
        out.epoch = self.epoch.load(Ordering::Relaxed);
        out.voices = self.voices.load(Ordering::Relaxed);
        out.held = self.held.load(Ordering::Relaxed);
        let v = self.display_voice.load(Ordering::Relaxed);
        out.display_voice = (v != NO_VOICE).then_some(v);
        out.display_note = self.display_note.load(Ordering::Relaxed);
        out.params_len = (self.params_len.load(Ordering::Relaxed) as usize).min(out.params.len());
        for (o, a) in out.params.iter_mut().zip(self.params.iter()) {
            *o = load_f32(a);
        }
        out.modulators_len =
            (self.modulators_len.load(Ordering::Relaxed) as usize).min(out.modulators.len());
        for (o, a) in out.modulators.iter_mut().zip(self.modulators.iter()) {
            *o = load_f32(a);
        }
        for (o, a) in out.macros.iter_mut().zip(&self.macros) {
            *o = load_f32(a);
        }
        for (o, a) in out.peaks.iter_mut().zip(&self.peaks) {
            *o = load_f32(a);
        }
        let s: [u64; STATS] = core::array::from_fn(|i| self.stats[i].load(Ordering::Relaxed));
        out.stats = EngineStats {
            notes: s[0],
            dropped_notes: s[1],
            dropped_events: s[2],
            segments: s[3],
            stolen_voices: s[4],
            cut_voices: s[5],
            stale_events: s[6],
            swaps: s[7],
            crossfades: s[8],
            dropped_note_ends: s[9],
        };
        out.dropped = self.dropped.load(Ordering::Relaxed);
    }

    /// Fill `out` with the newest scope samples, oldest first, the newest last; samples older than the ring (or
    /// than the first publish) read 0. Returns the total number of samples ever written, so a reader can tell whether
    /// anything new arrived. Never allocates. Samples the writer overwrote during the copy may show the newer block:
    /// harmless for a display.
    pub fn scope(&self, out: &mut [f32]) -> u64 {
        let written = self.scope_written.load(Ordering::Acquire);
        let mask = self.scope.len() as u64 - 1;
        let available = written.min(self.scope.len() as u64);
        let len = out.len() as u64;
        for (k, o) in out.iter_mut().enumerate() {
            // Sample `written − len + k`, if it is still in the ring.
            let age = len - k as u64; // 1 for the newest
            *o = if age <= available {
                load_f32(&self.scope[((written - age) & mask) as usize])
            } else {
                0.0
            };
        }
        written
    }

    /// Writer: one block's mono mix into the scope ring (only the newest ring-length samples of a longer block).
    pub(crate) fn write_scope(&self, left: &[f32], right: &[f32]) {
        let n = left.len().min(right.len());
        let skip = n.saturating_sub(self.scope.len());
        let mask = self.scope.len() as u64 - 1;
        let mut w = self.scope_written.load(Ordering::Relaxed) + skip as u64;
        for (l, r) in left[skip..n].iter().zip(&right[skip..n]) {
            store_f32(&self.scope[(w & mask) as usize], 0.5 * (l + r));
            w += 1;
        }
        self.scope_written.store(w, Ordering::Release);
    }

    /// The scope ring's length.
    pub(crate) fn scope_len(&self) -> usize {
        self.scope.len()
    }

    pub(crate) fn store_stats(&self, s: &EngineStats) {
        let v = [
            s.notes,
            s.dropped_notes,
            s.dropped_events,
            s.segments,
            s.stolen_voices,
            s.cut_voices,
            s.stale_events,
            s.swaps,
            s.crossfades,
            s.dropped_note_ends,
        ];
        for (a, x) in self.stats.iter().zip(v) {
            a.store(x, Ordering::Relaxed);
        }
    }

    /// Writer (the audio thread, the only one): a publish of the frame starts. The version goes odd, and a reader that
    /// sees it odd, or changed after its copy, tries again. Call it before the first value store of a publish and
    /// [`Tap::finish`] after the last.
    pub(crate) fn begin(&self) {
        let v = self.seq.load(Ordering::Relaxed);
        self.seq.store(v.wrapping_add(1), Ordering::Relaxed);
        // The stores that follow must not move above the odd version (Boehm's seqlock writer).
        fence(Ordering::Release);
    }

    /// Writer: the publish is complete; the version goes even again, then bump the generation and set the wake flag
    /// if the reader sleeps.
    pub(crate) fn finish(&self) {
        let v = self.seq.load(Ordering::Relaxed);
        self.seq.store((v | 1).wrapping_add(1), Ordering::Release);
        self.generation.fetch_add(1, Ordering::SeqCst);
        if self.sleeping.swap(false, Ordering::SeqCst) {
            self.wake.store(true, Ordering::Release);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// How many routed values and modulators the synthetic plan with this epoch publishes.
    fn layout(epoch: u64) -> (usize, usize) {
        (1 + (epoch % 5) as usize, (epoch % 3) as usize)
    }

    /// The value stores of one synthetic publish, which come first in the engine too. Epoch `e` publishes
    /// `layout(e)` values, all `e`, and modulators, all `-e`.
    fn write_values(tap: &Tap, e: u64) {
        let (n, m) = layout(e);
        for a in tap.params.iter().take(n) {
            store_f32(a, e as f32);
        }
        for a in tap.modulators.iter().take(m) {
            store_f32(a, -(e as f32));
        }
    }

    /// The rest of a publish, with the epoch last: the worst case for a torn read.
    fn write_header(tap: &Tap, e: u64) {
        let (n, m) = layout(e);
        tap.params_len.store(n as u32, Ordering::Relaxed);
        tap.modulators_len.store(m as u32, Ordering::Relaxed);
        tap.voices.store(e as u32, Ordering::Relaxed);
        tap.epoch.store(e, Ordering::Relaxed);
    }

    fn publish(tap: &Tap, e: u64) {
        tap.begin();
        write_values(tap, e);
        write_header(tap, e);
        tap.finish();
    }

    /// Whether `frame` is exactly one publish of the synthetic plan.
    fn is_whole(frame: &TapFrame) -> bool {
        let e = frame.epoch;
        let (n, m) = layout(e);
        frame.params_len == n
            && frame.modulators_len == m
            && frame.voices == e as u32
            && frame.params[..n].iter().all(|&v| v == e as f32)
            && frame.modulators[..m].iter().all(|&v| v == -(e as f32))
    }

    fn tap() -> Arc<Tap> {
        Tap::new(TapCapacity {
            params: 8,
            modulators: 4,
            scope: 32,
        })
    }

    #[test]
    fn a_reader_refuses_a_frame_while_a_publish_is_open() {
        let tap = tap();
        let mut frame = tap.frame();
        publish(&tap, 1);
        tap.read(&mut frame);
        assert!(is_whole(&frame));
        assert_eq!((frame.generation, frame.epoch), (1, 1));

        // Mid-publish: epoch 4's values (5 routed, 1 modulator) are in, its epoch and lengths are not yet.
        tap.begin();
        write_values(&tap, 4);
        // A bare copy shows the tear the seqlock exists to hide (the negative control) ...
        let mut raw = tap.frame();
        tap.copy_into(&mut raw);
        assert_eq!(raw.epoch, 1);
        assert!(
            !is_whole(&raw),
            "the bare copy mixes epoch 1's header with epoch 4's values"
        );
        // ... and the reader comes back empty-handed, with its generation, instead of waiting or guessing.
        tap.read(&mut frame);
        assert_eq!((frame.params_len, frame.modulators_len), (0, 0));
        assert_eq!(frame.generation, 1, "no new data was claimed");
        write_header(&tap, 4);
        tap.read(&mut frame);
        assert_eq!(
            (frame.params_len, frame.modulators_len),
            (0, 0),
            "still open"
        );
        tap.finish();
        tap.read(&mut frame);
        assert!(is_whole(&frame));
        assert_eq!((frame.generation, frame.epoch), (2, 4));
    }

    #[test]
    fn a_copy_between_two_publishes_is_whole_and_the_version_stays_even() {
        let tap = tap();
        let mut frame = tap.frame();
        for e in 1..=50 {
            publish(&tap, e);
            assert_eq!(tap.seq.load(Ordering::Relaxed) & 1, 0);
            tap.read(&mut frame);
            assert!(is_whole(&frame), "epoch {e}");
            assert_eq!(frame.generation, e);
        }
        assert_eq!(tap.seq.load(Ordering::Relaxed), 100);
    }

    #[test]
    #[allow(
        clippy::disallowed_methods,
        reason = "a test of the concurrent contract: the engine publishes on the audio thread while the editor reads"
    )]
    fn a_reader_never_sees_a_torn_frame_while_the_writer_publishes() {
        const PUBLISHES: u64 = 200_000;
        let tap = tap();
        let done = AtomicBool::new(false);
        let mut seen = 0u64;
        std::thread::scope(|s| {
            s.spawn(|| {
                for e in 1..=PUBLISHES {
                    publish(&tap, e);
                }
                done.store(true, Ordering::Release);
            });
            let mut frame = tap.frame();
            let mut last = 0;
            while !done.load(Ordering::Acquire) {
                tap.read(&mut frame);
                if frame.generation == 0 || (frame.params_len == 0 && frame.modulators_len == 0) {
                    continue; // nothing yet, or the writer was busy for every try
                }
                assert!(is_whole(&frame), "torn: {frame:?}");
                assert!(frame.generation >= last, "the generation never goes back");
                last = frame.generation;
                seen += 1;
            }
        });
        let mut frame = tap.frame();
        tap.read(&mut frame);
        assert!(is_whole(&frame));
        assert_eq!(frame.epoch, PUBLISHES);
        assert_eq!(frame.generation, PUBLISHES);
        eprintln!("{seen} frames checked while the writer published");
    }
}
