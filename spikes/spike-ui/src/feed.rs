//! The audio → UI feed: a lock-free single-producer ring of recent output samples, the LFO's
//! phase, and the handshake that lets the UI sleep while nothing changes.
//!
//! The producer (the plugin's audio thread, or the dev window's fake one) never blocks, locks or
//! allocates: every operation is an atomic load or store. The UI never waits on the producer; it
//! may see a frame that straddles two blocks, which a scope can't show.
//!
//! Repaint handshake: the UI calls [`AudioFeed::sleep`] when it stops scheduling frames. The next
//! publish that changes something returns `true` from [`FeedWriter::finish`], and the producer
//! then wakes the UI (`RepaintNotifier::request_repaint` in the plugin, an atomic store).

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};

/// Ring capacity in samples (a power of two, more than the scope's 2048).
pub const RING_SIZE: usize = 4096;

/// Peak below which a block counts as silent.
const SILENCE: f32 = 1e-5;

/// Sample rate of the demo signal.
pub const DEMO_SAMPLE_RATE: f32 = 48_000.0;

/// The demo signal of the dev window, the benchmark and the tests: an A major triad of naive
/// saws (a scope wants harmonics), phase-continuous for any length.
#[derive(Clone, Debug, Default)]
pub struct DemoChord {
    phases: [f32; 3],
}

impl DemoChord {
    const HZ: [f32; 3] = [220.0, 277.18, 329.63];
    const LEVEL: f32 = 0.25;

    /// The next sample at [`DEMO_SAMPLE_RATE`].
    pub fn next_sample(&mut self) -> f32 {
        let mut sum = 0.0;
        for (phase, hz) in self.phases.iter_mut().zip(Self::HZ) {
            sum += (*phase * 2.0 - 1.0) * Self::LEVEL;
            *phase = (*phase + hz / DEMO_SAMPLE_RATE).fract();
        }
        sum
    }
}

pub struct AudioFeed {
    samples: Box<[AtomicU32]>,
    /// Total samples published (monotonic); the newest sample is at `written - 1`.
    written: AtomicUsize,
    lfo_phase: AtomicU32,
    /// Something is sounding, so modulation is running.
    active: AtomicBool,
    /// Bumped by every publish that changed what the UI shows.
    generation: AtomicU64,
    /// Producer-only memory of the previous block (atomic only so the feed is `Sync`).
    last_silent: AtomicBool,
    ui_sleeping: AtomicBool,
}

impl Default for AudioFeed {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioFeed {
    pub fn new() -> Self {
        Self {
            samples: (0..RING_SIZE).map(|_| AtomicU32::new(0)).collect(),
            written: AtomicUsize::new(0),
            lfo_phase: AtomicU32::new(0),
            active: AtomicBool::new(false),
            generation: AtomicU64::new(0),
            last_silent: AtomicBool::new(true),
            ui_sleeping: AtomicBool::new(false),
        }
    }

    /// Starts a block (producer side).
    pub fn writer(&self) -> FeedWriter<'_> {
        FeedWriter {
            feed: self,
            position: self.written.load(Ordering::Relaxed),
            peak: 0.0,
        }
    }

    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }

    /// The LFO's phase (0..1) while something sounds, else `None`.
    pub fn lfo(&self) -> Option<f32> {
        self.active
            .load(Ordering::Relaxed)
            .then(|| f32::from_bits(self.lfo_phase.load(Ordering::Relaxed)))
    }

    /// Copies the newest `out.len()` samples (oldest first); zeros before the first publish.
    pub fn copy_latest(&self, out: &mut [f32]) {
        let end = self.written.load(Ordering::Acquire);
        let n = out.len().min(RING_SIZE);
        for (i, slot) in out.iter_mut().enumerate() {
            let back = n - i;
            *slot = if back <= end {
                f32::from_bits(self.samples[(end - back) % RING_SIZE].load(Ordering::Relaxed))
            } else {
                0.0
            };
        }
    }

    /// The UI stops scheduling frames. Returns `true` if a publish slipped in since the UI last
    /// read `seen` (then it should repaint once more instead of sleeping).
    pub fn sleep(&self, seen: u64) -> bool {
        self.ui_sleeping.store(true, Ordering::SeqCst);
        if self.generation() == seen {
            false
        } else {
            self.ui_sleeping.store(false, Ordering::SeqCst);
            true
        }
    }

    /// The UI is scheduling frames again (it will poll the feed itself).
    pub fn awake(&self) {
        self.ui_sleeping.store(false, Ordering::SeqCst);
    }
}

/// One block of samples being published. Real-time safe.
pub struct FeedWriter<'a> {
    feed: &'a AudioFeed,
    position: usize,
    peak: f32,
}

impl FeedWriter<'_> {
    pub fn push(&mut self, sample: f32) {
        self.feed.samples[self.position % RING_SIZE].store(sample.to_bits(), Ordering::Relaxed);
        self.position = self.position.wrapping_add(1);
        self.peak = self.peak.max(sample.abs());
    }

    /// Publishes the block with the LFO state. Returns `true` when the UI is asleep and should be
    /// woken. A silent block after a silent block publishes nothing, so an idle synth costs the
    /// UI nothing.
    pub fn finish(self, lfo_phase: f32, active: bool) -> bool {
        let feed = self.feed;
        let silent = self.peak < SILENCE;
        let was_silent = feed.last_silent.swap(silent, Ordering::Relaxed);
        let was_active = feed.active.swap(active, Ordering::Relaxed);
        feed.lfo_phase.store(lfo_phase.to_bits(), Ordering::Relaxed);
        let changed = !(silent && was_silent) || active || was_active;
        if !changed {
            return false;
        }
        if !(silent && was_silent) {
            feed.written.store(self.position, Ordering::Release);
        }
        feed.generation.fetch_add(1, Ordering::SeqCst);
        feed.ui_sleeping.swap(false, Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn publish(feed: &AudioFeed, samples: impl IntoIterator<Item = f32>, active: bool) -> bool {
        let mut writer = feed.writer();
        for s in samples {
            writer.push(s);
        }
        writer.finish(0.25, active)
    }

    #[test]
    fn copies_the_newest_samples_across_the_wrap() {
        let feed = AudioFeed::new();
        publish(&feed, (0..RING_SIZE + 100).map(|i| i as f32), false);
        let mut out = [0.0; 4];
        feed.copy_latest(&mut out);
        let last = (RING_SIZE + 99) as f32;
        assert_eq!(out, [last - 3.0, last - 2.0, last - 1.0, last]);
    }

    #[test]
    fn zeros_before_anything_was_written() {
        let feed = AudioFeed::new();
        publish(&feed, [1.0, 2.0], false);
        let mut out = [9.0; 4];
        feed.copy_latest(&mut out);
        assert_eq!(out, [0.0, 0.0, 1.0, 2.0]);
    }

    #[test]
    fn silence_after_silence_changes_nothing() {
        let feed = AudioFeed::new();
        let start = feed.generation();
        publish(&feed, [0.0; 64], false);
        assert_eq!(feed.generation(), start, "idle synth: no new data");
        publish(&feed, [0.5; 64], false);
        publish(&feed, [0.0; 64], false); // the first silent block clears the scope
        let after = feed.generation();
        assert_eq!(after, start + 2);
        publish(&feed, [0.0; 64], false);
        assert_eq!(feed.generation(), after);
    }

    #[test]
    fn a_sleeping_ui_is_woken_once() {
        let feed = AudioFeed::new();
        let seen = feed.generation();
        assert!(!feed.sleep(seen));
        assert!(publish(&feed, [0.5; 8], true), "first change wakes the UI");
        assert!(!publish(&feed, [0.5; 8], true), "already awake");
        assert!(
            feed.sleep(seen),
            "a publish slipped in: repaint instead of sleeping"
        );
    }

    #[test]
    fn lfo_is_visible_only_while_active() {
        let feed = AudioFeed::new();
        publish(&feed, [0.5; 8], true);
        assert_eq!(feed.lfo(), Some(0.25));
        publish(&feed, [0.0; 8], false);
        assert_eq!(feed.lfo(), None);
    }
}
