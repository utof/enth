//! A Recreate round in the plugin (lane SKR, freeze point F2r; docs/session.md "The Recreate round", design.md §9
//! "The round: A/B and feedback", docs/plugin.md "The Recreate round"). The shell's side of the session's pure
//! `game` module: it runs the round's jobs, plays the A/B and writes the round log. No product rules: what a round
//! is, when a job runs and what the gains are stay the session's and `enth-challenge`'s.
//!
//! | Part | Thread | What |
//! | --- | --- | --- |
//! | [`Game`] | host main, GUI, background pump (whoever drains the session) | in [`crate::Shared`]: acts on the session's [`GameRequest`]s after unlocking (jobs to the worker, the A/B atomics, round-log lines; [`Game::act`] returns whether to mark the project dirty), hands results back ([`Game::take_results`]) |
//! | the game worker (`enth-game`) | its own thread, spawned at the first job | runs each [`Job`] from its payload ([`Runner`]): `prepare_round`, the target's loop rendered offline, `score_round`, `score_structure`, Mine's loudness. It never locks the session; results go to a queue the GUI thread drains at the top of a frame, and each post asks the editor for a repaint |
//! | [`Player`] | audio (`process`) | reads the A/B atomics ([`AbShared`]), takes the target's loop from a lock-free [`Mailbox`], plays the phrase live through the engine (Mine ▶, on [`GAME_PORT`]) locked to the loop's position, and mixes Target ▶ and Mine ▶ with a [`AB_FADE_MS`] crossfade and the loudness-match gains. Never allocates, frees, locks or logs |
//!
//! **Neither side blocks on a full queue** (docs/session.md, "What lane SKR must wire" 3): the job queue and the
//! result queue are unbounded `VecDeque`/`Vec` behind short `parking_lot` locks (never the session's), and the
//! worker waits on a condition variable only for work.
//!
//! **The loop's hand-off** works like a plan's (a slot and a return slot): the worker publishes a boxed
//! [`TargetLoop`] into [`Mailbox`]'s incoming slot (an `AtomicPtr`; a loop the audio thread has not taken yet is
//! dropped by the worker), the audio thread swaps it out and hands the loop it replaced back through the retired
//! slot, keeping it in a pending slot while the retired one is still full, so the audio thread never frees. The
//! worker, the GUI frame and [`Drop`] collect retired loops. Only the worker publishes, so a clear (an empty loop,
//! sent when `AbState::target_ready` turns false) and the next round's loop arrive in order.
//!
//! **Determinism.** With no round the player does nothing to the block: it reads four atomics and one pointer and
//! returns before touching a sample, so a plugin render still equals `enth render` and `host-render` is unchanged.
//! Target ▶ plays exactly the scored audio when the host runs at the scored rate (48 kHz,
//! `enth_challenge::model::SCORING_SAMPLE_RATE`); at another rate the worker renders the same patch and phrase at the
//! host's rate (the engine's output differs only by the rate), and re-renders when the rate changes mid-round.
//!
//! **Loudness** (`Prepared::target_lufs`, Mine's `Loudness` job): lane SS's in-house BS.1770 loudness
//! (`enth_challenge::loudness::integrated_lufs`, `libm` only) of the phrase rendered at the scored rate, on the mid
//! channel ([`integrated_lufs`]); the session turns both readings into the A/B gains with the scorer's own
//! `gain_db` (P11: the level match and the scorer share one gain function).
//!
//! **The round log** is JSONL, one [`RoundLogEntry`] per line (`enth.round-log/1`, lane GK's exporter reads it),
//! appended after unlocking on the thread that drained it (never the audio thread): `$ENTH_ROUNDS_DIR/round-log.jsonl`
//! when set, else `<Documents>/enth/Rounds/round-log.jsonl`, beside the crash folder ([`round_log_path`]).

use std::collections::VecDeque;
use std::fs;
use std::io::{self, Write as _};
use std::panic::{self, AssertUnwindSafe};
use std::path::PathBuf;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU8, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::thread::JoinHandle;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use enth_challenge::model::{Phrase, SCORING_SAMPLE_RATE, SCORING_SEED};
use enth_challenge::{
    PreparedRound, RoundError, phrases, prepare_challenge, prepare_round, score_round,
    score_structure,
};
use enth_engine::{Event, EventKind, NoteKey};
use enth_patch::Patch;
use enth_render::{RenderOutput, RenderSpec, render};
use enth_session::game::{
    AbState, CoachHint, GameRequest, Job, JobId, JobKind, JobResult, Listen, Prepared, RoundId,
    RoundLogEntry,
};
use enth_solver::coach::{CoachCfg, CoachPlan, CoachTarget};
use parking_lot::{Condvar, Mutex};

use crate::presets::{Environment, Os, SystemEnv, documents_dir};

/// The note port of Mine ▶'s phrase notes (beside the session's audition port `0x7F00`): host notes are on port 0,
/// so a host note-off never releases a phrase note, and the `NOTE_END` ledger never reports one.
pub const GAME_PORT: i16 = 0x7F01;

/// The A/B crossfade, and every gain ramp of the player, in milliseconds (design.md §9: "a short crossfade").
pub const AB_FADE_MS: f32 = 10.0;

/// The most notes of a phrase Mine ▶ plays (every phrase of generator v1 has one or two; more are dropped).
pub const MAX_PHRASE_NOTES: usize = 8;

/// The round log's file name, in [`round_log_path`]'s folder.
pub const ROUND_LOG_FILE: &str = "round-log.jsonl";

// ---------------------------------------------------------------------------------------------------------------
// Time and seeds (the shell supplies both; core crates read no clock)
// ---------------------------------------------------------------------------------------------------------------

/// The shell's monotonic clock for `Session::game_tick`, in milliseconds since the first call in this process.
pub fn now_ms() -> u64 {
    static EPOCH: OnceLock<Instant> = OnceLock::new();
    let epoch = *EPOCH.get_or_init(Instant::now);
    u64::try_from(epoch.elapsed().as_millis()).unwrap_or(u64::MAX)
}

/// A fresh 40-bit seed for a round from settings (a short code carries 40 bits; docs/session.md, "What lane SKR
/// must wire" 4): the wall clock and a per-process counter through `SplitMix64` (Vigna's public-domain constants).
pub fn fresh_seed() -> u64 {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos() as u64);
    let mut z = nanos.wrapping_add(
        COUNTER
            .fetch_add(1, Ordering::Relaxed)
            .wrapping_mul(0x9E37_79B9_7F4A_7C15),
    );
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    (z ^ (z >> 31)) & ((1 << 40) - 1)
}

// ---------------------------------------------------------------------------------------------------------------
// The target's loop and its hand-off
// ---------------------------------------------------------------------------------------------------------------

/// One phrase note of a loop, in loop frames at the loop's rate.
#[derive(Clone, Copy, Debug, PartialEq)]
struct LoopNote {
    key: i16,
    velocity: f32,
    on: usize,
    off: usize,
}

/// The target's loop of the scored phrase, rendered offline by the worker, with the phrase's notes at the loop's
/// rate (so Mine ▶ plays them at the same positions). Built off the audio thread; the audio thread only reads it and
/// hands it back. An empty loop (no frames) clears the player.
#[derive(Debug)]
pub struct TargetLoop {
    /// The round it belongs to (`None` for the empty loop that clears).
    round: Option<RoundId>,
    sample_rate: u32,
    left: Box<[f32]>,
    right: Box<[f32]>,
    notes: Box<[LoopNote]>,
}

impl TargetLoop {
    /// The loop of `round`'s target: `out` (a render of `phrase` at `out.sample_rate`) and the phrase's notes in
    /// frames at that rate (rounded to the nearest frame, the note-off at most the last frame).
    fn new(round: RoundId, out: RenderOutput, phrase: &Phrase) -> TargetLoop {
        let len = out.left.len().min(out.right.len());
        let sr = u64::from(out.sample_rate);
        let frame = |ms: u32| {
            let f = (u64::from(ms) * sr + 500) / 1000;
            (f as usize).min(len.saturating_sub(1))
        };
        let notes = phrase
            .notes
            .iter()
            .take(MAX_PHRASE_NOTES)
            .map(|n| LoopNote {
                key: i16::from(n.key),
                velocity: f32::from(n.velocity) / 127.0,
                on: frame(n.start_ms),
                off: frame(n.start_ms.saturating_add(n.hold_ms)),
            })
            .collect();
        let mut left = out.left;
        let mut right = out.right;
        left.truncate(len);
        right.truncate(len);
        TargetLoop {
            round: Some(round),
            sample_rate: out.sample_rate,
            left: left.into_boxed_slice(),
            right: right.into_boxed_slice(),
            notes,
        }
    }

    /// The empty loop: handed over when the round's target goes, so the audio thread drops (hands back) its loop.
    fn empty() -> TargetLoop {
        TargetLoop {
            round: None,
            sample_rate: 0,
            left: Box::default(),
            right: Box::default(),
            notes: Box::default(),
        }
    }

    /// Frames in the loop (0 for the empty loop).
    pub fn frames(&self) -> usize {
        self.left.len()
    }

    /// The loop's sample rate.
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// The round it belongs to.
    pub fn round(&self) -> Option<RoundId> {
        self.round
    }

    /// The left and right channels.
    pub fn channels(&self) -> (&[f32], &[f32]) {
        (&self.left, &self.right)
    }
}

/// The lock-free hand-off of [`TargetLoop`]s to the audio thread: an incoming slot (non-audio side → audio thread)
/// and a retired slot (audio thread → non-audio side), each an `AtomicPtr` holding a boxed loop or null. The audio
/// thread only swaps pointers ([`Mailbox::take`], [`Mailbox::retire`]); every allocation and free happens on the
/// other side ([`Mailbox::publish`], [`Mailbox::collect`], [`Drop`]).
#[derive(Debug)]
pub struct Mailbox {
    incoming: AtomicPtr<TargetLoop>,
    retired: AtomicPtr<TargetLoop>,
}

impl Default for Mailbox {
    fn default() -> Self {
        Mailbox::new()
    }
}

impl Mailbox {
    /// Both slots empty.
    pub fn new() -> Mailbox {
        Mailbox {
            incoming: AtomicPtr::new(ptr::null_mut()),
            retired: AtomicPtr::new(ptr::null_mut()),
        }
    }

    /// Not the audio thread: free the loop the audio thread handed back, if any.
    pub fn collect(&self) {
        let p = self.retired.swap(ptr::null_mut(), Ordering::AcqRel);
        // SAFETY: a non-null pointer in a slot always came from `Box::into_raw` and is owned by the slot; the swap
        // took it out, so this is its only owner.
        drop(unsafe { take_box(p) });
    }

    /// Not the audio thread: hand `next` to the audio thread. A loop it has not taken yet is freed here.
    pub fn publish(&self, next: Box<TargetLoop>) {
        self.collect();
        let old = self.incoming.swap(Box::into_raw(next), Ordering::AcqRel);
        // SAFETY: as in `collect`: the swap made this the pointer's only owner.
        drop(unsafe { take_box(old) });
    }

    /// The audio thread: the newest loop, if one is waiting. Swaps a pointer; never allocates.
    fn take(&self) -> Option<Box<TargetLoop>> {
        // A load first, so an idle block reads the slot without writing its cache line.
        if self.incoming.load(Ordering::Acquire).is_null() {
            return None;
        }
        let p = self.incoming.swap(ptr::null_mut(), Ordering::AcqRel);
        // SAFETY: as in `collect`.
        unsafe { take_box(p) }
    }

    /// The audio thread: hand `old` back for the other side to free. Gives it back while the retired slot is still
    /// full (the caller keeps it and tries again next block). Never frees.
    fn retire(&self, old: Box<TargetLoop>) -> Result<(), Box<TargetLoop>> {
        let p = Box::into_raw(old);
        match self
            .retired
            .compare_exchange(ptr::null_mut(), p, Ordering::AcqRel, Ordering::Acquire)
        {
            Ok(_) => Ok(()),
            // SAFETY: the exchange failed, so the slot did not take `p`: it is still only ours.
            Err(_) => Err(unsafe { Box::from_raw(p) }),
        }
    }
}

impl Drop for Mailbox {
    fn drop(&mut self) {
        for slot in [&self.incoming, &self.retired] {
            let p = slot.swap(ptr::null_mut(), Ordering::AcqRel);
            // SAFETY: as in `collect`; nothing else can reach the mailbox while it drops.
            drop(unsafe { take_box(p) });
        }
    }
}

/// The box behind a pointer taken out of a [`Mailbox`] slot (`None` for null).
///
/// # Safety
/// `p` is null or came from `Box::into_raw` and has no other owner.
unsafe fn take_box(p: *mut TargetLoop) -> Option<Box<TargetLoop>> {
    // SAFETY: the caller's contract.
    (!p.is_null()).then(|| unsafe { Box::from_raw(p) })
}

// ---------------------------------------------------------------------------------------------------------------
// The A/B atomics
// ---------------------------------------------------------------------------------------------------------------

/// The A/B state `process()` follows, as atomics: the switch, whether the target is ready, and the loudness-match
/// gains as linear factors (the session's dB values converted on the thread that drained them, never in `process()`).
#[derive(Debug)]
pub struct AbShared {
    listen: AtomicU8,
    ready: AtomicBool,
    target_gain: AtomicU32,
    mine_gain: AtomicU32,
}

/// One block's reading of [`AbShared`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AbFrame {
    /// The switch.
    pub listen: Listen,
    /// The target's loop is the session's prepared target.
    pub ready: bool,
    /// Target ▶'s gain (linear, at most 1).
    pub target_gain: f32,
    /// Mine ▶'s gain (linear, at most 1).
    pub mine_gain: f32,
}

impl Default for AbShared {
    fn default() -> Self {
        AbShared {
            listen: AtomicU8::new(0),
            ready: AtomicBool::new(false),
            target_gain: AtomicU32::new(1.0f32.to_bits()),
            mine_gain: AtomicU32::new(1.0f32.to_bits()),
        }
    }
}

/// dB (≤ 0; the session never boosts) to a linear factor in 0..=1 (`enth_challenge::loudness::db_to_amp`, the
/// scorer's own conversion); a value that is not finite is 0 dB.
fn gain(db: f32) -> f32 {
    if db.is_finite() {
        enth_challenge::loudness::db_to_amp(f64::from(db.min(0.0))) as f32
    } else {
        1.0
    }
}

impl AbShared {
    /// Store the session's A/B state (the gains first, so a block that sees the new switch sees its gains too).
    pub fn store(&self, state: &AbState) {
        self.target_gain
            .store(gain(state.target_gain_db).to_bits(), Ordering::Relaxed);
        self.mine_gain
            .store(gain(state.mine_gain_db).to_bits(), Ordering::Relaxed);
        let listen = match state.listen {
            Listen::Off => 0,
            Listen::Target => 1,
            Listen::Mine => 2,
        };
        self.ready.store(state.target_ready, Ordering::Release);
        self.listen.store(listen, Ordering::Release);
    }

    /// This block's reading (the audio thread: four atomic loads).
    pub fn load(&self) -> AbFrame {
        let listen = match self.listen.load(Ordering::Acquire) {
            1 => Listen::Target,
            2 => Listen::Mine,
            _ => Listen::Off,
        };
        AbFrame {
            listen,
            ready: self.ready.load(Ordering::Acquire),
            target_gain: f32::from_bits(self.target_gain.load(Ordering::Relaxed)),
            mine_gain: f32::from_bits(self.mine_gain.load(Ordering::Relaxed)),
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Jobs
// ---------------------------------------------------------------------------------------------------------------

/// Runs the round's jobs, each a pure function of its payload, and keeps the round's [`PreparedRound`] for its
/// score and rings jobs. The game worker owns one; tests and the `ab-toggle` harness may run one in place.
#[derive(Default)]
pub struct Runner {
    round: Option<(RoundId, Arc<PreparedRound>)>,
    /// Lane SN's coach for the round: its target measured once (`CoachTarget::for_round`), and the last patch's plan,
    /// so levels 2–4 of an unchanged patch cost nothing.
    coach: Option<Coach>,
    /// The coach's evaluator (lane HINT, FU1): [`crate::hint_eval::Workers`], half the logical CPUs within 1 to 4.
    eval: crate::hint_eval::Workers,
}

/// The coach's state for one round.
struct Coach {
    round: RoundId,
    target: CoachTarget,
    /// The last patch planned, and its four hints.
    plan: Option<(Patch, CoachPlan)>,
}

impl std::fmt::Debug for Runner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Runner")
            .field("round", &self.round.as_ref().map(|r| r.0))
            .field("coach", &self.coach.as_ref().map(|c| c.round))
            .field("hint_workers", &self.eval.count())
            .finish()
    }
}

/// A failed job's answer.
fn failed(what: &str, e: impl std::fmt::Display) -> JobResult {
    JobResult::Failed {
        message: format!("{what}: {e}"),
    }
}

impl Runner {
    /// A runner whose hints use `n` worker threads (tests and the latency measurement; the shell's default is
    /// [`crate::hint_eval::hint_workers`] for the machine).
    pub fn with_hint_workers(n: usize) -> Runner {
        Runner {
            eval: crate::hint_eval::Workers::new(n),
            ..Runner::default()
        }
    }

    /// Run `job`. A `Prepare` also returns the target's loop, rendered at `host_rate` (the scored rate when 0).
    pub fn run(&mut self, job: Job, host_rate: u32) -> (JobResult, Option<Box<TargetLoop>>) {
        match job.kind {
            JobKind::Prepare { spec } => match self.prepare(job.round, &spec, host_rate) {
                Ok((result, l)) => (result, Some(l)),
                Err(result) => (result, None),
            },
            JobKind::Score {
                challenge, patch, ..
            } => {
                let result = match self.prepared(job.round, &challenge) {
                    Ok(round) => match score_round(&round, &patch) {
                        Ok(report) => JobResult::Scored(Box::new(report)),
                        Err(e) => failed("scoring", e),
                    },
                    Err(e) => e,
                };
                (result, None)
            }
            JobKind::Rings {
                challenge, patch, ..
            } => {
                let result = match self.prepared(job.round, &challenge) {
                    Ok(round) => match score_structure(&round, &patch) {
                        Ok(rings) => JobResult::Rings(Box::new(rings)),
                        Err(e) => failed("rings", e),
                    },
                    Err(e) => e,
                };
                (result, None)
            }
            JobKind::Loudness { challenge, patch } => {
                let result = match phrases::phrase(challenge.phrase) {
                    Some(phrase) => match render_phrase(&patch, phrase, SCORING_SAMPLE_RATE) {
                        Ok(out) => JobResult::Loudness {
                            lufs: integrated_lufs(&out),
                        },
                        Err(e) => failed("rendering Mine's phrase", e),
                    },
                    None => failed("loudness", "the challenge's phrase is not in this build"),
                };
                (result, None)
            }
            JobKind::Hint {
                challenge,
                patch,
                level,
                ..
            } => (self.hint(job.round, &challenge, &patch, level), None),
            _ => (
                JobResult::Failed {
                    message: "this build's game worker does not know the job".to_owned(),
                },
                None,
            ),
        }
    }

    /// `prepare_round`, the scored render and its loudness, and the loop at the host's rate. A silent target is
    /// [`JobResult::SilentTarget`] (R1/S1 lane INT), which the editor answers with another seed.
    fn prepare(
        &mut self,
        id: RoundId,
        spec: &enth_challenge::RandomSpec,
        host_rate: u32,
    ) -> Result<(JobResult, Box<TargetLoop>), JobResult> {
        let round = prepare_round(spec).map_err(|e| match e {
            RoundError::SilentTarget => JobResult::SilentTarget,
            e => failed("preparing the target", e),
        })?;
        let scored = render_phrase(round.target(), round.phrase(), SCORING_SAMPLE_RATE)
            .map_err(|e| failed("rendering the target", e))?;
        let lufs = integrated_lufs(&scored);
        let rate = if host_rate == 0 {
            SCORING_SAMPLE_RATE
        } else {
            host_rate
        };
        let out = if rate == SCORING_SAMPLE_RATE {
            scored
        } else {
            render_phrase(round.target(), round.phrase(), rate)
                .map_err(|e| failed("rendering the target's loop", e))?
        };
        let l = Box::new(TargetLoop::new(id, out, round.phrase()));
        let result = JobResult::Prepared(Box::new(Prepared::from_round(&round, lufs)));
        self.round = Some((id, Arc::new(round)));
        Ok((result, l))
    }

    /// The round's prepared state: the one kept from its `Prepare`, else rebuilt from the job's challenge (the
    /// worker restarted, or the round was restored by another path).
    fn prepared(
        &mut self,
        id: RoundId,
        challenge: &enth_challenge::Challenge,
    ) -> Result<Arc<PreparedRound>, JobResult> {
        if let Some((rid, round)) = &self.round
            && *rid == id
            && round.challenge() == challenge
        {
            return Ok(Arc::clone(round));
        }
        let round = Arc::new(
            prepare_challenge(challenge.clone()).map_err(|e| failed("preparing the target", e))?,
        );
        self.round = Some((id, Arc::clone(&round)));
        Ok(round)
    }

    /// The kept round's loop rendered again at `rate` (the host's rate changed mid-round).
    pub fn rerender(&self, rate: u32) -> Option<Box<TargetLoop>> {
        let (id, round) = self.round.as_ref()?;
        let out = render_phrase(round.target(), round.phrase(), rate).ok()?;
        Some(Box::new(TargetLoop::new(*id, out, round.phrase())))
    }

    /// Forget the round (its target went).
    pub fn clear(&mut self) {
        self.round = None;
        self.coach = None;
    }

    /// Lane SN's coach at `level` for `patch` (docs/dsp/hints.md): the target measured once per round, one plan per
    /// patch, then that plan's hint at the level. Only the hint's text reaches the session (`CoachHint`); the plan's
    /// verified fix holds exact values and is never shown. The renders go through [`crate::hint_eval::Workers`] (lane
    /// HINT, FU1): parallel, results by job index, so the hint is the same for any worker count; the job runs on the
    /// game worker either way, so the editor never waits for it.
    fn hint(
        &mut self,
        id: RoundId,
        challenge: &enth_challenge::Challenge,
        patch: &Patch,
        level: u8,
    ) -> JobResult {
        let round = match self.prepared(id, challenge) {
            Ok(round) => round,
            Err(e) => return e,
        };
        let cfg = CoachCfg::default();
        let eval = &mut self.eval;
        if self.coach.as_ref().is_none_or(|c| c.round != id) {
            match CoachTarget::for_round(&round, eval, &cfg) {
                Ok(target) => {
                    self.coach = Some(Coach {
                        round: id,
                        target,
                        plan: None,
                    });
                }
                Err(e) => return failed("measuring the target for hints", e),
            }
        }
        let Some(coach) = self.coach.as_mut() else {
            return failed("hints", "no coach");
        };
        if coach.plan.as_ref().is_none_or(|(p, _)| p != patch) {
            match coach.target.plan(patch, eval, &cfg) {
                Ok(plan) => coach.plan = Some((patch.clone(), plan)),
                Err(e) => return failed("the hint", e),
            }
        }
        let Some((_, plan)) = coach.plan.as_ref() else {
            return failed("hints", "no plan");
        };
        let h = plan.hint(level);
        JobResult::Hint(CoachHint::new(h.level, h.text.clone()))
    }
}

/// `patch` playing `phrase` at `sample_rate` with the scored seed: what the scorer hears at the scored rate.
fn render_phrase(
    patch: &Patch,
    phrase: &Phrase,
    sample_rate: u32,
) -> Result<RenderOutput, enth_render::RenderError> {
    render(
        patch,
        &phrase.performance(),
        &RenderSpec::new(sample_rate, SCORING_SEED),
    )
}

/// The integrated loudness (LUFS) of a render, the way the sound scorer measures it (P11): lane SS's in-house
/// BS.1770 loudness, `enth_challenge::loudness::integrated_lufs`, on the mid channel `(L + R) / 2`. Both sides of the
/// A/B are 48 kHz renders (`SCORING_SAMPLE_RATE`), and the session turns the two readings into gains with the shared
/// `enth_challenge::loudness::gain_db` (`game::ab_gains`). `None` for silence.
pub fn integrated_lufs(out: &RenderOutput) -> Option<f32> {
    enth_challenge::loudness::integrated_lufs(&out.mid(), out.sample_rate).map(|l| l as f32)
}

// ---------------------------------------------------------------------------------------------------------------
// The worker and the shell's side
// ---------------------------------------------------------------------------------------------------------------

/// What the GUI (or whoever drained the session) sends the worker.
#[derive(Debug)]
enum Msg {
    /// A session job.
    Run(Job),
    /// The host's rate changed while a target is ready: render the kept round's loop at this rate.
    Rerender(u32),
    /// The target went (`AbState::target_ready` turned false): forget the round and clear the player.
    Clear,
}

/// What the worker shares with the shell's side.
struct Inner {
    queue: Mutex<VecDeque<Msg>>,
    wake: Condvar,
    quit: AtomicBool,
    /// Results waiting for the GUI thread's `game_job_done`, oldest first.
    results: Mutex<Vec<(JobId, JobResult)>>,
    mailbox: Arc<Mailbox>,
    /// The open editor's context, for a lock-free repaint request on every post.
    repaint: Mutex<Option<egui::Context>>,
    /// The host's sample rate (0 until the first activation).
    host_rate: AtomicU32,
    /// The rate of the newest loop published (0: none, or cleared).
    loop_rate: AtomicU32,
    /// Messages taken but not finished (tests wait for 0 with an empty queue).
    busy: AtomicU32,
}

impl Inner {
    fn publish(&self, l: Box<TargetLoop>) {
        self.loop_rate.store(l.sample_rate, Ordering::Release);
        self.mailbox.publish(l);
    }

    fn post(&self, id: JobId, result: JobResult) {
        self.results.lock().push((id, result));
        if let Some(ctx) = self.repaint.lock().as_ref() {
            ctx.request_repaint();
        }
    }
}

/// The worker thread: one message at a time, a panicking job answered `Failed` (the panic hook writes the crash
/// file), until [`Game`] drops.
fn work(inner: &Inner) {
    let mut runner = Runner::default();
    loop {
        let msg = {
            let mut queue = inner.queue.lock();
            loop {
                if inner.quit.load(Ordering::Acquire) {
                    return;
                }
                if let Some(msg) = queue.pop_front() {
                    inner.busy.fetch_add(1, Ordering::AcqRel);
                    break msg;
                }
                inner.wake.wait(&mut queue);
            }
        };
        match msg {
            Msg::Run(job) => {
                let id = job.id;
                let rate = inner.host_rate.load(Ordering::Acquire);
                let ran = panic::catch_unwind(AssertUnwindSafe(|| runner.run(job, rate)));
                let (result, l) = ran.unwrap_or_else(|_| {
                    runner.clear();
                    (
                        JobResult::Failed {
                            message: "the game worker panicked (see the crash folder)".to_owned(),
                        },
                        None,
                    )
                });
                if let Some(l) = l {
                    inner.publish(l);
                }
                inner.post(id, result);
            }
            Msg::Rerender(rate) => {
                if let Ok(Some(l)) = panic::catch_unwind(AssertUnwindSafe(|| runner.rerender(rate)))
                {
                    inner.publish(l);
                }
            }
            Msg::Clear => {
                runner.clear();
                inner.publish(Box::new(TargetLoop::empty()));
            }
        }
        inner.busy.fetch_sub(1, Ordering::AcqRel);
    }
}

/// The shell's side of a round, in [`crate::Shared`]: the worker (spawned at the first job, joined on drop), the A/B
/// atomics `process()` reads, the loop hand-off and the round log.
pub struct Game {
    inner: Arc<Inner>,
    thread: Mutex<Option<JoinHandle<()>>>,
    /// The A/B atomics (the audio thread reads them every block).
    pub ab: AbShared,
    /// The last A/B state stored said the target is ready.
    ready: AtomicBool,
    /// The host rate a re-render was last asked for (once per change).
    rerender_asked: AtomicU32,
    log: RoundLog,
}

impl std::fmt::Debug for Game {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Game")
            .field("worker", &self.thread.lock().is_some())
            .field("ab", &self.ab.load())
            .finish_non_exhaustive()
    }
}

impl Default for Game {
    fn default() -> Self {
        Game::new()
    }
}

impl Game {
    /// No worker yet, no round; the log goes where the environment says ([`round_log_path`]).
    pub fn new() -> Game {
        Game {
            inner: Arc::new(Inner {
                queue: Mutex::new(VecDeque::new()),
                wake: Condvar::new(),
                quit: AtomicBool::new(false),
                results: Mutex::new(Vec::new()),
                mailbox: Arc::new(Mailbox::new()),
                repaint: Mutex::new(None),
                host_rate: AtomicU32::new(0),
                loop_rate: AtomicU32::new(0),
                busy: AtomicU32::new(0),
            }),
            thread: Mutex::new(None),
            ab: AbShared::default(),
            ready: AtomicBool::new(false),
            rerender_asked: AtomicU32::new(0),
            log: RoundLog::default(),
        }
    }

    /// The loop hand-off, for the audio thread's [`Player`].
    pub fn mailbox(&self) -> Arc<Mailbox> {
        Arc::clone(&self.inner.mailbox)
    }

    /// The host's sample rate (at every activation): the next loop is rendered at it.
    pub fn set_host_rate(&self, sample_rate: f32) {
        self.inner
            .host_rate
            .store(sample_rate.round() as u32, Ordering::Release);
    }

    /// The open editor's context (at `build()`), or none (at close): the worker asks it for a repaint on every post.
    pub fn set_repaint(&self, ctx: Option<egui::Context>) {
        *self.inner.repaint.lock() = ctx;
    }

    /// Write the round log to `path` instead of the environment's place (tests; `None`: the environment's).
    pub fn set_log_path(&self, path: Option<PathBuf>) {
        *self.log.path.lock() =
            path.map_or(LogPath::FromEnvironment, |p| LogPath::Resolved(Some(p)));
    }

    /// After unlocking: act on the game requests a drain took, in order, the A/B state first (it is the state
    /// after every drained request, and a clear must reach the worker before the next round's `Prepare`). Jobs go
    /// to the worker, cancels remove queued jobs, the A/B state goes to the atomics (and a clear to the worker when
    /// the target went), log lines to the round log. Returns whether the project must be marked dirty (coalesced
    /// with `ShellRequest::PatchEdited` by the caller: one mark per drain).
    pub fn act(&self, requests: Vec<GameRequest>) -> bool {
        let mut dirty = false;
        for request in &requests {
            if let GameRequest::Ab(state) = request {
                self.ab.store(state);
                let was = self.ready.swap(state.target_ready, Ordering::AcqRel);
                if was && !state.target_ready {
                    self.send(Msg::Clear);
                }
            }
        }
        for request in requests {
            match request {
                GameRequest::Run(job) => self.send(Msg::Run(job)),
                GameRequest::Cancel(id) => self.cancel(id),
                GameRequest::Log(entry) => {
                    // A log that cannot be written must not stop the round (the record is in the saved state).
                    let _ = self.log.append(&entry);
                }
                GameRequest::Dirty => dirty = true,
                _ => {}
            }
        }
        dirty
    }

    /// The results the worker posted, oldest first, for `Session::game_job_done` (the GUI thread, at the top of a
    /// frame). Also frees loops the audio thread handed back.
    pub fn take_results(&self) -> Vec<(JobId, JobResult)> {
        self.inner.mailbox.collect();
        std::mem::take(&mut *self.inner.results.lock())
    }

    /// The GUI frame: while a target is ready and the newest loop has another rate than the host's (a
    /// re-activation at another rate), ask the worker once for a loop at the host's rate.
    pub fn follow_rate(&self) {
        let host = self.inner.host_rate.load(Ordering::Acquire);
        let current = self.inner.loop_rate.load(Ordering::Acquire);
        if self.ready.load(Ordering::Acquire)
            && host != 0
            && current != 0
            && current != host
            && self.rerender_asked.swap(host, Ordering::AcqRel) != host
        {
            self.send(Msg::Rerender(host));
        }
    }

    /// Whether the worker has nothing queued or running (tests and the `ab-toggle` harness).
    pub fn idle(&self) -> bool {
        self.inner.queue.lock().is_empty() && self.inner.busy.load(Ordering::Acquire) == 0
    }

    /// Whether the worker thread exists (spawned at the first job).
    pub fn worker_started(&self) -> bool {
        self.thread.lock().is_some()
    }

    fn send(&self, msg: Msg) {
        self.inner.queue.lock().push_back(msg);
        self.inner.wake.notify_one();
        let mut thread = self.thread.lock();
        if thread.is_none() {
            let inner = Arc::clone(&self.inner);
            // A worker that cannot start leaves the jobs queued: the round waits (the status shows "preparing")
            // rather than the host crashing.
            *thread = std::thread::Builder::new()
                .name("enth-game".to_owned())
                .spawn(move || work(&inner))
                .ok();
        }
    }

    fn cancel(&self, id: JobId) {
        self.inner
            .queue
            .lock()
            .retain(|m| !matches!(m, Msg::Run(job) if job.id == id));
    }
}

impl Drop for Game {
    /// Stop the worker (a job in progress finishes first; a few hundred milliseconds at most) and join it.
    fn drop(&mut self) {
        self.inner.quit.store(true, Ordering::Release);
        {
            // Under the queue's lock, so the worker is either waiting (and woken) or will see the flag.
            let _queue = self.inner.queue.lock();
            self.inner.wake.notify_all();
        }
        if let Some(thread) = self.thread.get_mut().take() {
            let _ = thread.join();
        }
        self.inner.mailbox.collect();
    }
}

// ---------------------------------------------------------------------------------------------------------------
// The round log
// ---------------------------------------------------------------------------------------------------------------

/// The round log's file for an environment: `$ENTH_ROUNDS_DIR/round-log.jsonl` if set and not empty, else
/// `<Documents>/enth/Rounds/round-log.jsonl` (beside `<Documents>/enth/Crashes`, the crash folder).
pub fn round_log_path(env: &dyn Environment, os: Os) -> Option<PathBuf> {
    env.var("ENTH_ROUNDS_DIR")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| documents_dir(env, os).map(|d| d.join("enth").join("Rounds")))
        .map(|d| d.join(ROUND_LOG_FILE))
}

/// Where the round log goes.
#[derive(Debug, Default)]
enum LogPath {
    /// Not looked up yet: [`round_log_path`] at the first line.
    #[default]
    FromEnvironment,
    /// Looked up (or set by a test); `None`: no folder (no home folder), nothing is written.
    Resolved(Option<PathBuf>),
}

/// The local round log: JSONL, appended one line per [`RoundLogEntry`].
#[derive(Debug, Default)]
struct RoundLog {
    path: Mutex<LogPath>,
}

impl RoundLog {
    fn append(&self, entry: &RoundLogEntry) -> io::Result<()> {
        let path = {
            let mut place = self.path.lock();
            if matches!(*place, LogPath::FromEnvironment) {
                *place = LogPath::Resolved(round_log_path(&SystemEnv, Os::current()));
            }
            match &*place {
                LogPath::Resolved(Some(path)) => path.clone(),
                _ => {
                    return Err(io::Error::new(
                        io::ErrorKind::NotFound,
                        "no folder for the round log",
                    ));
                }
            }
        };
        let mut line = serde_json::to_string(entry).map_err(io::Error::other)?;
        line.push('\n');
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        // One write per line, in append mode: lines from two instances never interleave mid-line.
        fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?
            .write_all(line.as_bytes())
    }
}

// ---------------------------------------------------------------------------------------------------------------
// The audio thread's player
// ---------------------------------------------------------------------------------------------------------------

/// The audio thread's side of the A/B (see the module docs). Allocated at construction; `process()` only reuses it.
#[derive(Debug)]
pub struct Player {
    mailbox: Arc<Mailbox>,
    current: Option<Box<TargetLoop>>,
    /// A loop waiting for the retired slot to empty.
    pending: Option<Box<TargetLoop>>,
    sample_rate: u32,
    /// One ramp step per frame (`1 / (AB_FADE_MS` in frames)).
    step: f32,
    /// The shared position in the loop, in frames.
    pos: usize,
    /// Mine's phrase is playing (Target ▶ or Mine ▶ is on).
    running: bool,
    /// Target ▶'s weight (0 = Mine or off, 1 = Target), the engine's gain and the loop's gain, ramped per frame.
    w: f32,
    engine_gain: f32,
    target_gain: f32,
    /// What the ramps head for this block.
    want: (f32, f32, f32),
    /// The note id each phrase note plays under, −1 when not held.
    held: [i32; MAX_PHRASE_NOTES],
    next_id: i32,
    /// The `ab-toggle` measurement harness.
    #[cfg(feature = "ab-toggle")]
    pub harness: harness::Toggle,
}

impl Player {
    /// A player on `mailbox` (the shared [`Game::mailbox`]).
    pub fn new(mailbox: Arc<Mailbox>) -> Player {
        Player {
            mailbox,
            current: None,
            pending: None,
            sample_rate: 0,
            step: 1.0,
            pos: 0,
            running: false,
            w: 0.0,
            engine_gain: 1.0,
            target_gain: 1.0,
            want: (0.0, 1.0, 1.0),
            held: [-1; MAX_PHRASE_NOTES],
            next_id: 0,
            #[cfg(feature = "ab-toggle")]
            harness: harness::Toggle::default(),
        }
    }

    /// At activate (main thread, before the engine plays): a fresh engine holds none of Mine's notes; the ramps
    /// follow the rate.
    pub fn activate(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate.round() as u32;
        self.step = 1.0 / (sample_rate * AB_FADE_MS / 1000.0).max(1.0);
        self.pos = 0;
        self.running = false;
        self.w = 0.0;
        self.engine_gain = 1.0;
        self.target_gain = 1.0;
        self.want = (0.0, 1.0, 1.0);
        self.held = [-1; MAX_PHRASE_NOTES];
    }

    /// At a CLAP reset (audio thread): the engine freed every voice, Mine's notes included.
    pub fn reset(&mut self) {
        self.held = [-1; MAX_PHRASE_NOTES];
    }

    /// Whether the player leaves the block untouched: off, with every ramp settled.
    pub fn idle(&self) -> bool {
        // The loop's gain does not matter while the loop is neither heard nor wanted.
        !self.running
            && self.w == 0.0
            && self.engine_gain == 1.0
            && self.want.0 == 0.0
            && self.want.1 == 1.0
    }

    /// The loop the player plays (tests).
    pub fn current(&self) -> Option<&TargetLoop> {
        self.current.as_deref()
    }

    /// Hand back what is pending, then take a new loop if one waits (only once nothing is pending, so a second
    /// loop never needs freeing here).
    fn poll(&mut self) {
        if let Some(old) = self.pending.take()
            && let Err(old) = self.mailbox.retire(old)
        {
            self.pending = Some(old);
            return;
        }
        if let Some(next) = self.mailbox.take() {
            if let Some(old) = self.current.replace(next)
                && let Err(old) = self.mailbox.retire(old)
            {
                self.pending = Some(old);
            }
            if let Some(l) = self.current.as_deref()
                && self.pos >= l.frames().max(1)
            {
                self.pos = 0;
            }
        }
    }

    /// The loop's length, when it can play here (not empty, and at the host's rate).
    fn usable(&self) -> usize {
        self.current
            .as_deref()
            .filter(|l| l.frames() > 0 && l.sample_rate == self.sample_rate)
            .map_or(0, TargetLoop::frames)
    }

    /// Before the engine renders a block of `frames`: take a new loop, follow the switch, and put Mine's phrase
    /// notes for this block into `events` (offsets from the block's start). Never allocates: an event that would
    /// not fit the list's capacity is dropped (the list holds every host event plus 72; the phrase adds at most a
    /// few per block).
    pub fn begin(&mut self, ab: AbFrame, frames: usize, events: &mut Vec<Event>) {
        self.poll();
        #[cfg(feature = "ab-toggle")]
        let ab = self.harness.apply(ab, frames);
        let len = self.usable();
        let active = ab.listen != Listen::Off && len > 0;
        let target = active && ab.listen == Listen::Target && ab.ready;
        self.want = (
            if target { 1.0 } else { 0.0 },
            if active { ab.mine_gain.min(1.0) } else { 1.0 },
            ab.target_gain.min(1.0),
        );
        if self.w == 0.0 {
            // The loop is not heard: its gain may jump.
            self.target_gain = self.want.2;
        }
        if active && !self.running {
            self.running = true;
            self.pos = 0;
        } else if !active && self.running {
            self.running = false;
            release_all(&mut self.held, events, 0);
        }
        if self.running {
            self.schedule(len, frames, events);
        }
    }

    /// Mine's phrase notes in `[pos, pos + frames)`, wrapping at the loop's end (where any note still held is
    /// released).
    fn schedule(&mut self, len: usize, frames: usize, events: &mut Vec<Event>) {
        let Some(l) = self.current.as_deref() else {
            return;
        };
        let mut p = self.pos % len;
        let mut done = 0;
        while done < frames {
            let n = (len - p).min(frames - done);
            for (i, note) in l.notes.iter().enumerate() {
                if (p..p + n).contains(&note.on) {
                    let offset = (done + note.on - p) as u32;
                    if self.held[i] >= 0 {
                        push(events, offset, note_off(self.held[i]));
                    }
                    self.next_id = self.next_id.wrapping_add(1) & i32::MAX;
                    self.held[i] = self.next_id;
                    push(
                        events,
                        offset,
                        EventKind::NoteOn {
                            key: NoteKey {
                                port: GAME_PORT,
                                channel: 0,
                                key: note.key,
                                note_id: self.next_id,
                            },
                            velocity: note.velocity,
                        },
                    );
                }
                if (p..p + n).contains(&note.off) && note.off > note.on && self.held[i] >= 0 {
                    push(events, (done + note.off - p) as u32, note_off(self.held[i]));
                    self.held[i] = -1;
                }
            }
            done += n;
            p += n;
            if p >= len {
                p = 0;
                release_all(&mut self.held, events, done.min(frames - 1) as u32);
            }
        }
    }

    /// After the engine rendered the block into `left` and `right`: the A/B mix,
    /// `out = engine · gain · (1 − w) + loop · gain · w`, every factor ramped over [`AB_FADE_MS`], and the shared
    /// position moved on. Returns at once (the block untouched) while [`Player::idle`].
    pub fn mix(&mut self, left: &mut [f32], right: &mut [f32]) {
        if self.idle() {
            return;
        }
        let n = left.len().min(right.len());
        let len = self.usable();
        let (tl, tr): (&[f32], &[f32]) = match self.current.as_deref() {
            Some(l) if len > 0 => (&l.left, &l.right),
            _ => (&[], &[]),
        };
        let (want_w, want_engine, want_target) = self.want;
        let mut p = if len > 0 { self.pos % len } else { 0 };
        for i in 0..n {
            self.w = approach(self.w, want_w, self.step);
            self.engine_gain = approach(self.engine_gain, want_engine, self.step);
            self.target_gain = approach(self.target_gain, want_target, self.step);
            let e = self.engine_gain * (1.0 - self.w);
            let t = self.target_gain * self.w;
            let (a, b) = if len > 0 { (tl[p], tr[p]) } else { (0.0, 0.0) };
            left[i] = left[i] * e + a * t;
            right[i] = right[i] * e + b * t;
            if len > 0 {
                p += 1;
                if p == len {
                    p = 0;
                }
            }
        }
        self.pos = p;
    }
}

/// `x` one ramp step towards `to`, landing on it exactly.
fn approach(x: f32, to: f32, step: f32) -> f32 {
    let d = to - x;
    if d.abs() <= step {
        to
    } else if d > 0.0 {
        x + step
    } else {
        x - step
    }
}

/// A note-off for Mine's note `id` (any key, on [`GAME_PORT`]).
fn note_off(id: i32) -> EventKind {
    EventKind::NoteOff {
        key: NoteKey {
            port: GAME_PORT,
            channel: -1,
            key: -1,
            note_id: id,
        },
        velocity: 0.0,
    }
}

/// Release every held phrase note at `offset`.
fn release_all(held: &mut [i32; MAX_PHRASE_NOTES], events: &mut Vec<Event>, offset: u32) {
    for id in held.iter_mut() {
        if *id >= 0 {
            push(events, offset, note_off(*id));
            *id = -1;
        }
    }
}

/// Push an event if the list has room (never grows it on the audio thread).
fn push(events: &mut Vec<Event>, offset: u32, kind: EventKind) {
    if events.len() < events.capacity() {
        events.push(Event { offset, kind });
    }
}

// ---------------------------------------------------------------------------------------------------------------
// The `ab-toggle` measurement harness
// ---------------------------------------------------------------------------------------------------------------

/// The `ab-toggle` cargo feature: a measurement harness for `enth-test-host process-stats --ab` (docs/plugin.md, "The
/// Recreate round"). With `ENTH_AB_TOGGLE_MS` set, activation starts a round (`ENTH_AB_CODE`, else level 1 with seed
/// 1), waits for its target, turns Target ▶ on, and `process()` flips Target ▶ and Mine ▶ itself every that many
/// milliseconds of audio, so a host that knows nothing of rounds measures `process()` with the A/B running: the loop
/// playing, the phrase live in the engine and a crossfade at every flip. At deactivation one JSON line
/// (`enth.plugin-ab-stats/1`) is appended to the file `ENTH_AB_REPORT` names. Off by default; never in a shipped build.
#[cfg(feature = "ab-toggle")]
pub mod harness {
    use std::io::Write as _;
    use std::time::{Duration, Instant};

    use enth_challenge::ChallengeCode;
    use enth_challenge::spec::{LATEST_GENERATOR, SpecPreset};
    use enth_session::game::{GameEdit, Listen, RoundSource};

    use super::{AbFrame, Player};
    use crate::Shared;

    /// The audio side: when to flip, and what happened.
    #[derive(Clone, Debug, Default)]
    pub struct Toggle {
        /// Frames between flips (0: the harness is off).
        pub every: u64,
        since: u64,
        target: bool,
        /// Flips made.
        pub flips: u64,
        /// Blocks with Target ▶ on, and with Mine ▶ on.
        pub target_blocks: u64,
        /// Blocks with Mine ▶ on.
        pub mine_blocks: u64,
        /// Whether the round's target was ready.
        pub ready: bool,
        /// The loop's frames.
        pub loop_frames: u64,
    }

    impl Toggle {
        /// This block's switch: the harness's, once the target is ready.
        pub(super) fn apply(&mut self, mut ab: AbFrame, frames: usize) -> AbFrame {
            if self.every == 0 || !ab.ready {
                return ab;
            }
            self.since += frames as u64;
            if self.since >= self.every {
                self.since = 0;
                self.target = !self.target;
                self.flips += 1;
            }
            ab.listen = if self.target {
                self.target_blocks += 1;
                Listen::Target
            } else {
                self.mine_blocks += 1;
                Listen::Mine
            };
            ab
        }
    }

    /// At activation (main thread): start the round and wait (at most 30 s) for its target, if `ENTH_AB_TOGGLE_MS`
    /// is set.
    pub fn start(shared: &Shared, player: &mut Player, sample_rate: f32) {
        let Some(ms) = std::env::var("ENTH_AB_TOGGLE_MS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
        else {
            return;
        };
        let code = std::env::var("ENTH_AB_CODE")
            .ok()
            .and_then(|c| ChallengeCode::decode(&c).ok())
            .or_else(|| {
                SpecPreset::by_id(1).and_then(|p| ChallengeCode::short(p.spec(LATEST_GENERATOR, 1)))
            });
        let Some(code) = code else {
            return;
        };
        let _ = shared.with_session(|s| s.game_start(RoundSource::Code(code)));
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut ready = false;
        while !ready && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
            let results = shared.game.take_results();
            if results.is_empty() {
                continue;
            }
            let (done, _) = shared.with_session(|s| {
                for (id, result) in results {
                    let _ = s.game_job_done(id, result);
                }
                s.game_ab().target_ready && s.game_listen(Listen::Target).is_ok()
            });
            ready = done;
        }
        player.harness = Toggle {
            every: (ms * sample_rate.round() as u64 / 1000).max(1),
            target: true,
            ready,
            ..Toggle::default()
        };
    }

    /// At deactivation: the report line, if `ENTH_AB_REPORT` names a file.
    pub fn report(player: &Player) {
        let Some(path) = std::env::var_os("ENTH_AB_REPORT") else {
            return;
        };
        let t = &player.harness;
        let line = serde_json::json!({
            "schema": "enth.plugin-ab-stats/1",
            "every_frames": t.every,
            "ready": t.ready,
            "flips": t.flips,
            "target_blocks": t.target_blocks,
            "mine_blocks": t.mine_blocks,
            "loop_frames": player.current().map_or(0, super::TargetLoop::frames),
        });
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            let _ = writeln!(f, "{line}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use enth_challenge::spec::{LATEST_GENERATOR, SpecPreset};
    use enth_session::game::GameEdit;

    fn round_id(serial: u32) -> RoundId {
        RoundId { serial, nonce: 7 }
    }

    /// A loop of `frames` frames whose left sample `i` is `i + 1` and right `-(i + 1)`, with one note.
    fn ramp_loop(round: u32, frames: usize, sample_rate: u32) -> TargetLoop {
        TargetLoop {
            round: Some(round_id(round)),
            sample_rate,
            left: (0..frames).map(|i| (i + 1) as f32).collect(),
            right: (0..frames).map(|i| -((i + 1) as f32)).collect(),
            notes: vec![LoopNote {
                key: 48,
                velocity: 100.0 / 127.0,
                on: 10,
                off: 60,
            }]
            .into_boxed_slice(),
        }
    }

    fn ab(listen: Listen) -> AbFrame {
        AbFrame {
            listen,
            ready: true,
            target_gain: 1.0,
            mine_gain: 1.0,
        }
    }

    #[test]
    fn the_mailbox_hands_loops_over_and_back_without_the_audio_side_freeing() {
        let mailbox = Arc::new(Mailbox::new());
        let mut player = Player::new(Arc::clone(&mailbox));
        player.activate(48_000.0);
        mailbox.publish(Box::new(ramp_loop(1, 100, 48_000)));
        mailbox.publish(Box::new(ramp_loop(2, 100, 48_000)));
        let mut events = Vec::with_capacity(16);
        player.begin(ab(Listen::Off), 32, &mut events);
        assert_eq!(
            player.current().and_then(TargetLoop::round),
            Some(round_id(2)),
            "the newest loop wins; the one never taken was freed by the publisher"
        );
        // A third loop: the player hands the second back through the retired slot.
        mailbox.publish(Box::new(ramp_loop(3, 100, 48_000)));
        player.begin(ab(Listen::Off), 32, &mut events);
        assert_eq!(
            player.current().and_then(TargetLoop::round),
            Some(round_id(3))
        );
        assert!(!mailbox.retired.load(Ordering::Acquire).is_null());
        // `publish` collects before it hands over, so fill the retired slot behind its back to see the pending slot
        // work: the loop the player replaces waits until the slot is collected.
        let blocker = Box::into_raw(Box::new(ramp_loop(9, 10, 48_000)));
        mailbox.collect();
        mailbox.retired.store(blocker, Ordering::Release);
        mailbox.incoming.store(
            Box::into_raw(Box::new(ramp_loop(4, 100, 48_000))),
            Ordering::Release,
        );
        player.begin(ab(Listen::Off), 32, &mut events);
        assert_eq!(
            player.current().and_then(TargetLoop::round),
            Some(round_id(4))
        );
        assert!(player.pending.is_some(), "round 3's loop waits");
        player.begin(ab(Listen::Off), 32, &mut events);
        assert!(player.pending.is_some(), "still full: it keeps waiting");
        mailbox.collect();
        player.begin(ab(Listen::Off), 32, &mut events);
        assert!(
            player.pending.is_none(),
            "handed back once the slot emptied"
        );
        // The empty loop clears.
        mailbox.publish(Box::new(TargetLoop::empty()));
        player.begin(ab(Listen::Off), 32, &mut events);
        assert_eq!(player.usable(), 0);
        assert!(events.is_empty(), "off: no phrase notes");
    }

    #[test]
    fn off_and_settled_leaves_the_block_bit_for_bit_untouched() {
        let mailbox = Arc::new(Mailbox::new());
        let mut player = Player::new(Arc::clone(&mailbox));
        player.activate(48_000.0);
        mailbox.publish(Box::new(ramp_loop(1, 1000, 48_000)));
        let mut events = Vec::with_capacity(16);
        let mut left: Vec<f32> = (0..64).map(|i| i as f32 * 0.01 - 0.3).collect();
        left[3] = -0.0;
        left[4] = f32::MIN_POSITIVE / 4.0;
        let mut right = left.clone();
        let before: Vec<u32> = left.iter().map(|x| x.to_bits()).collect();
        // A live round's level-match gains do not matter while the A/B is off.
        let off = AbFrame {
            target_gain: 0.5,
            mine_gain: 0.7,
            ..ab(Listen::Off)
        };
        player.begin(off, 64, &mut events);
        player.mix(&mut left, &mut right);
        assert!(player.idle());
        assert_eq!(left.iter().map(|x| x.to_bits()).collect::<Vec<_>>(), before);
        assert_eq!(
            right.iter().map(|x| x.to_bits()).collect::<Vec<_>>(),
            before
        );
    }

    #[test]
    fn target_plays_the_loop_after_the_crossfade_and_mine_plays_the_engine() {
        let mailbox = Arc::new(Mailbox::new());
        let mut player = Player::new(Arc::clone(&mailbox));
        player.activate(48_000.0);
        mailbox.publish(Box::new(ramp_loop(1, 4800, 48_000)));
        let fade = (48_000.0 * AB_FADE_MS / 1000.0) as usize;
        let mut events = Vec::with_capacity(16);
        // Target from off: the loop from position 0, the engine (all 0.5 here) faded out over the crossfade.
        let mut left = vec![0.5f32; 1024];
        let mut right = vec![0.5f32; 1024];
        player.begin(ab(Listen::Target), 1024, &mut events);
        player.mix(&mut left, &mut right);
        assert_eq!(
            left[fade + 10],
            (fade + 11) as f32,
            "the loop, at its own position"
        );
        assert_eq!(right[fade + 10], -((fade + 11) as f32));
        assert!(
            left[0] > 0.4 && left[0] < 0.6,
            "the first frame is mostly the engine"
        );
        // Mine's phrase note starts at loop frame 10 and ends at 60, even while Target plays.
        assert_eq!(events.len(), 2, "{events:?}");
        assert!(
            matches!(events[0], Event { offset: 10, kind: EventKind::NoteOn { key, .. } } if key.port == GAME_PORT && key.key == 48)
        );
        assert!(matches!(
            events[1],
            Event {
                offset: 60,
                kind: EventKind::NoteOff { .. }
            }
        ));
        // Flip to Mine at the same position: after the crossfade the block is the engine's.
        events.clear();
        let mut left = vec![0.25f32; 1024];
        let mut right = vec![0.25f32; 1024];
        player.begin(ab(Listen::Mine), 1024, &mut events);
        player.mix(&mut left, &mut right);
        assert_eq!(left[fade + 1], 0.25);
        assert!(left[0] > 1.0, "the first frames still hold the loop");
        assert_eq!(player.pos, 2048, "one shared position");
        // Off: the phrase stops (nothing held now) and the gains settle back to unity, then the player is idle.
        events.clear();
        let mut left = vec![0.25f32; 1024];
        let mut right = vec![0.25f32; 1024];
        player.begin(ab(Listen::Off), 1024, &mut events);
        player.mix(&mut left, &mut right);
        assert!(player.idle());
        assert!(left.iter().all(|&x| x == 0.25));
    }

    #[test]
    fn mine_is_attenuated_and_the_phrase_wraps_with_its_notes_released() {
        let mailbox = Arc::new(Mailbox::new());
        let mut player = Player::new(Arc::clone(&mailbox));
        player.activate(48_000.0);
        let mut l = Box::new(ramp_loop(1, 100, 48_000));
        l.notes = vec![LoopNote {
            key: 60,
            velocity: 1.0,
            on: 90,
            off: 99,
        }]
        .into_boxed_slice();
        // A note that would hang over the loop's end is released at the wrap.
        l.notes[0].off = 99;
        mailbox.publish(l);
        let mut events = Vec::with_capacity(16);
        let mine = AbFrame {
            mine_gain: 0.5,
            ..ab(Listen::Mine)
        };
        player.begin(mine, 150, &mut events);
        let kinds: Vec<(u32, bool)> = events
            .iter()
            .map(|e| (e.offset, matches!(e.kind, EventKind::NoteOn { .. })))
            .collect();
        // On at 90, off at 99, then the wrap at 100 (nothing held), and no second on before 190.
        assert_eq!(kinds, vec![(90, true), (99, false)]);
        let mut left = vec![1.0f32; 2000];
        let mut right = vec![1.0f32; 2000];
        player.begin(mine, 2000, &mut events);
        player.mix(&mut left, &mut right);
        assert_eq!(left[1999], 0.5, "Mine at its loudness-match gain");
        // A loop at another rate than the host's never plays.
        mailbox.publish(Box::new(ramp_loop(2, 100, 44_100)));
        player.begin(mine, 32, &mut events);
        assert_eq!(player.usable(), 0);
        assert!(!player.running);
    }

    #[allow(
        clippy::disallowed_methods,
        reason = "a calibration signal for the loudness test, not a render: std maths is fine here"
    )]
    fn sine(phase: f64) -> f64 {
        phase.sin()
    }

    #[test]
    fn the_loudness_is_the_scorers_on_the_mid_channel() {
        // BS.1770's calibration: a 0 dBFS sine at 997 Hz reads -3.01 LUFS; here it is in both channels, so the mid
        // channel holds it at full scale.
        let n = 48_000 * 3;
        let sine: Vec<f32> = (0..n)
            .map(|i| sine(2.0 * std::f64::consts::PI * 997.0 * i as f64 / 48_000.0) as f32)
            .collect();
        let out = RenderOutput {
            sample_rate: 48_000,
            left: sine.clone(),
            right: sine,
            stats: enth_render::RenderStats::default(),
        };
        let lufs = integrated_lufs(&out).unwrap();
        assert!((lufs + 3.01).abs() < 0.02, "{lufs}");
        assert_eq!(
            Some(lufs),
            enth_challenge::loudness::integrated_lufs(&out.mid(), 48_000).map(|l| l as f32)
        );
        let silent = RenderOutput {
            left: vec![0.0; n],
            right: vec![0.0; n],
            ..out
        };
        assert_eq!(integrated_lufs(&silent), None);
    }

    fn level1(seed: u64) -> enth_challenge::RandomSpec {
        SpecPreset::by_id(1).unwrap().spec(LATEST_GENERATOR, seed)
    }

    #[test]
    fn the_runner_prepares_the_scored_loop_and_scores_with_the_round_it_kept() {
        let mut runner = Runner::default();
        let id = round_id(1);
        let job = |kind| Job {
            id: JobId(1),
            round: id,
            kind,
        };
        let (result, l) = runner.run(job(JobKind::Prepare { spec: level1(42) }), 48_000);
        let JobResult::Prepared(prepared) = result else {
            panic!("{result:?}");
        };
        let l = l.unwrap();
        let round = prepare_round(&level1(42)).unwrap();
        let scored = render_phrase(round.target(), round.phrase(), SCORING_SAMPLE_RATE).unwrap();
        assert_eq!(
            l.channels(),
            (&scored.left[..], &scored.right[..]),
            "Target ▶ is exactly the scored audio at 48 kHz"
        );
        assert_eq!(
            l.frames(),
            (round.phrase().length_ms as usize) * 48,
            "the phrase's length"
        );
        assert_eq!(prepared.fingerprint, round.fingerprint());
        assert!(prepared.target_lufs.is_some_and(|v| v > -40.0 && v < 0.0));
        let challenge = Arc::new(prepared.challenge.clone());
        let target = Arc::new(prepared.target.clone());
        let (scored, _) = runner.run(
            job(JobKind::Score {
                challenge: Arc::clone(&challenge),
                target: Arc::clone(&target),
                patch: Box::new(prepared.target.clone()),
                submit: 1,
            }),
            48_000,
        );
        let JobResult::Scored(report) = scored else {
            panic!("{scored:?}")
        };
        assert!(report.passed && report.sound.percent == 100.0);
        let (rings, _) = runner.run(
            job(JobKind::Rings {
                challenge: Arc::clone(&challenge),
                target,
                patch: Box::new(prepared.start.clone()),
            }),
            48_000,
        );
        assert!(matches!(rings, JobResult::Rings(_)), "{rings:?}");
        let (loud, _) = runner.run(
            job(JobKind::Loudness {
                challenge: Arc::clone(&challenge),
                patch: Box::new(prepared.target.clone()),
            }),
            48_000,
        );
        assert_eq!(
            loud,
            JobResult::Loudness {
                lufs: prepared.target_lufs
            },
            "Mine's loudness of the target equals the target's: one measurement"
        );
        // At 44.1 kHz the loop is rendered at the host's rate; the loudness stays the scored render's.
        let (result, l) = Runner::default().run(job(JobKind::Prepare { spec: level1(42) }), 44_100);
        assert!(
            matches!(&result, JobResult::Prepared(p) if p.target_lufs == prepared.target_lufs),
            "{result:?}"
        );
        assert_eq!(l.unwrap().sample_rate(), 44_100);
        // A silent target (lane GC's `silent-l2-s23`, custom settings) is answered as such, with no loop and no
        // round kept (R1/S1 lane INT).
        let silent =
            enth_challenge::ChallengeCode::decode("H000-G000-0000-0000-Q680-0000-804X-2T92-M0S")
                .unwrap();
        let mut quiet = Runner::default();
        let (result, l) = quiet.run(
            job(JobKind::Prepare {
                spec: *silent.spec(),
            }),
            48_000,
        );
        assert_eq!(result, JobResult::SilentTarget);
        assert!(l.is_none() && quiet.round.is_none());
        // A runner that lost the round rebuilds it from the job's challenge.
        let (scored, _) = Runner::default().run(
            job(JobKind::Score {
                challenge,
                target: Arc::new(prepared.target.clone()),
                patch: Box::new(prepared.start.clone()),
                submit: 2,
            }),
            48_000,
        );
        assert!(matches!(scored, JobResult::Scored(_)), "{scored:?}");
    }

    #[test]
    fn hints_come_from_the_coach_one_plan_per_patch() {
        let mut runner = Runner::default();
        let id = round_id(3);
        let (result, _) = runner.run(
            Job {
                id: JobId(1),
                round: id,
                kind: JobKind::Prepare { spec: level1(9) },
            },
            48_000,
        );
        let JobResult::Prepared(prepared) = result else {
            panic!("{result:?}")
        };
        let challenge = Arc::new(prepared.challenge.clone());
        let hint = |runner: &mut Runner, level| {
            runner
                .run(
                    Job {
                        id: JobId(2),
                        round: id,
                        kind: JobKind::Hint {
                            challenge: Arc::clone(&challenge),
                            target: Arc::new(prepared.target.clone()),
                            patch: Box::new(prepared.start.clone()),
                            level,
                        },
                    },
                    48_000,
                )
                .0
        };
        let JobResult::Hint(first) = hint(&mut runner, 1) else {
            panic!("no hint")
        };
        assert_eq!(first.level, 1);
        assert_ne!(first.text, "");
        assert!(
            !first.text.chars().any(|c| c.is_ascii_digit()),
            "level 1 carries no number: {}",
            first.text
        );
        let planned = runner
            .coach
            .as_ref()
            .and_then(|c| c.plan.as_ref())
            .map(|p| p.1.clone());
        let JobResult::Hint(fourth) = hint(&mut runner, 4) else {
            panic!("no hint")
        };
        assert_eq!(fourth.level, 4);
        assert_eq!(
            runner
                .coach
                .as_ref()
                .and_then(|c| c.plan.as_ref())
                .map(|p| p.1.clone()),
            planned,
            "the same patch reuses its plan"
        );
    }

    /// Lane HINT (FU1): the hint job's parallel evaluator gives the same hints, every level, for one worker and for
    /// several, on the start patch and on a patch the player moved (the coach's sensitivity batch and the solver's
    /// probe batch both run in parallel).
    #[test]
    fn the_worker_count_never_changes_the_hint() {
        let spec = SpecPreset::by_id(2).unwrap().spec(LATEST_GENERATOR, 11);
        let id = round_id(5);
        let hints = |workers: usize| {
            let mut runner = Runner::with_hint_workers(workers);
            let (result, _) = runner.run(
                Job {
                    id: JobId(1),
                    round: id,
                    kind: JobKind::Prepare { spec },
                },
                48_000,
            );
            let JobResult::Prepared(prepared) = result else {
                panic!("{result:?}")
            };
            let challenge = Arc::new(prepared.challenge.clone());
            let mut moved = prepared.start.clone();
            if let Some(addr) = prepared.params.first() {
                let spec = moved.spec(addr).unwrap();
                let value = spec.from_norm((spec.to_norm(moved.value(addr).unwrap()) + 0.3) % 1.0);
                enth_patch::apply(
                    &mut moved,
                    &enth_patch::Op::SetParam {
                        addr: addr.clone(),
                        value,
                    },
                )
                .unwrap();
            }
            let mut out = Vec::new();
            for patch in [&prepared.start, &moved] {
                for level in 1..=4 {
                    let (r, _) = runner.run(
                        Job {
                            id: JobId(2),
                            round: id,
                            kind: JobKind::Hint {
                                challenge: Arc::clone(&challenge),
                                target: Arc::new(prepared.target.clone()),
                                patch: Box::new(patch.clone()),
                                level,
                            },
                        },
                        48_000,
                    );
                    let JobResult::Hint(h) = r else {
                        panic!("{r:?}")
                    };
                    out.push(h);
                }
            }
            out
        };
        let serial = hints(1);
        assert_eq!(serial.len(), 8);
        assert_eq!(hints(3), serial, "3 workers");
    }

    #[test]
    fn the_round_log_path_follows_the_environment() {
        struct Env(Vec<(&'static str, &'static str)>);
        impl Environment for Env {
            fn var(&self, name: &str) -> Option<std::ffi::OsString> {
                self.0
                    .iter()
                    .find(|(k, _)| *k == name)
                    .map(|(_, v)| v.into())
            }
            fn read_text(&self, _path: &std::path::Path) -> Option<String> {
                None
            }
        }
        assert_eq!(
            round_log_path(&Env(vec![("ENTH_ROUNDS_DIR", "/tmp/r")]), Os::Linux),
            Some(PathBuf::from("/tmp/r/round-log.jsonl"))
        );
        assert_eq!(
            round_log_path(&Env(vec![("HOME", "/home/u")]), Os::MacOs),
            Some(PathBuf::from(
                "/home/u/Documents/enth/Rounds/round-log.jsonl"
            ))
        );
        assert_eq!(round_log_path(&Env(vec![]), Os::Linux), None);
    }

    #[test]
    fn a_fresh_seed_fits_a_short_code() {
        for _ in 0..100 {
            let seed = fresh_seed();
            assert!(seed < 1 << 40);
            assert!(enth_challenge::ChallengeCode::short(level1(seed)).is_some());
        }
    }

    #[test]
    fn the_game_spawns_its_worker_at_the_first_job_and_joins_it_on_drop() {
        let game = Game::new();
        assert!(
            !game.worker_started(),
            "no thread until a round asks for one"
        );
        let mut session = enth_session::Session::new(Patch::init());
        session.game_tick(0, enth_session::game::EditorView::Opened);
        session
            .game_start(enth_session::game::RoundSource::Settings(level1(5)))
            .unwrap();
        let requests: Vec<GameRequest> = session.drain_game_requests().collect();
        let _ = game.act(requests);
        assert!(game.worker_started());
        let t0 = Instant::now();
        let results = loop {
            let r = game.take_results();
            if !r.is_empty() {
                break r;
            }
            assert!(
                t0.elapsed().as_secs() < 60,
                "the prepare job never finished"
            );
            std::thread::sleep(std::time::Duration::from_millis(2));
        };
        for (id, result) in results {
            session.game_job_done(id, result).unwrap();
        }
        assert!(session.game_ab().target_ready);
        drop(game);
    }
}
