//! **Owner: lane SKD** (the Duel skeleton, freeze point F2d; feature `duel`), then **DUI** (the history file under
//! `duel/`, additively). Pre-declared by lane S0e (R4/R5, freeze point F0e).
//!
//! The plugin's side of duels (docs/duels.md, "The flow", "Verification" and "Privacy"; docs/plugin.md, "Duels"). Shells
//! translate and hold no product rules: the codes, `verify` and paste-anything are `enth-challenge`'s, the state machine
//! is `enth-session`'s `duel`, the card is `enth-ui`'s, and this file runs the jobs and draws the salt:
//!
//! - **Duel jobs run on the game worker** ([`crate::game::Game::run_task`]), never on the audio thread and never under
//!   the session lock: **the codes** (the answer code, then the round prepared once with the round's scorer set and
//!   the canonical answer scored, then the scorecard: about a second), **verify** (their answer decoded and checked,
//!   the round prepared once with `verify::scorer_set(card)`, theirs verified and your canonical answer re-scored on
//!   the same round, posted as `DuelVerified…with_claim(v.claim)`) and **an audition** (their patch rendered with the
//!   challenge's phrase at the host's rate, played through the Learn examples slot). Results come back to the GUI
//!   thread's next frame ([`before_frame`]); each post asks the editor for a repaint.
//! - **The salt** (the F1e review's M6): 64 bits of OS randomness (`getrandom::u64`), drawn **once per duel**, when the
//!   player first shares it, and kept with the duel's codes before anything can be copied. **Never derived from a clock
//!   or a seed** (the answer code carries it in the clear): no OS randomness, no codes. Lane DUI writes it to the duel
//!   history; until then it lives as long as the plugin instance.
//! - **The clipboard is written on a click and never read here.** The card writes it with egui's `copy_text` inside a
//!   click on Copy; a paste is the platform's Paste (Ctrl+V, ⌘V) into the card's paste box, which egui-baseview turns
//!   into text. This file has no clipboard access at all.
//! - **No networking code**: no sockets, no HTTP, no QR. A code is text the player moves.
//! - **The duel log** (`ENTH_DUEL_LOG`, a file path, unset by default): when set, one JSON line per share and per change
//!   of the duel's state (`enth.duel-log/1`: codes, states and scores, never a name or a time of day), so the REAPER
//!   `duel` scenario can see what the card shows. It is a test seam, not the history.
//!
//! The editor glue calls [`before_frame`] and [`after_frame`] around each frame (`editor.rs`'s `round_frame`), and every
//! drain of the session hands the duel requests to [`Duel::act`] after unlocking (`lib.rs`'s `with_session`).

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use enth_challenge::answer::AnswerCode;
use enth_challenge::result::{Scorecard, ScorerSet};
use enth_challenge::{ChallengeCode, score_round, verify};
use enth_patch::Patch;
use enth_session::Session;
use enth_session::duel::{
    AnswerState, DuelEdit, DuelId, DuelJobId, DuelJobResult, DuelOwn, DuelRequest, DuelVerified,
    DuelVerify, DuelView, Rescored, VerifyState,
};
use enth_ui::editor::Editor;
use enth_ui::editor::duel::{ShareCodes, ShellDuel};
use parking_lot::Mutex;

#[cfg(feature = "learn")]
use crate::game::TargetLoop;
use crate::game::{Game, WorkerTask};

/// The duel log's schema (the `ENTH_DUEL_LOG` test seam).
pub const LOG_SCHEMA: &str = "enth.duel-log/1";

/// A duel's salt: **64 bits of OS randomness**, never a clock or a seed (docs/duels.md, "Privacy"). An error when the
/// operating system gives none: then no codes are made.
///
/// # Errors
/// The OS randomness source failed.
pub fn draw_salt() -> Result<u64, String> {
    getrandom::u64().map_err(|e| {
        format!(
            "this computer gave no OS randomness for the duel's salt ({e}); the codes are not made"
        )
    })
}

/// What the worker posts back.
#[derive(Debug)]
enum Post {
    Verified(DuelJobId, DuelJobResult),
    Shared(DuelId, Result<ShareCodes, String>),
}

/// What the worker's tasks share with the shell.
#[derive(Debug, Default)]
struct Inner {
    posts: Mutex<Vec<Post>>,
    /// The verify job the session waits for (0: none): a cancelled job that has not run yet is skipped.
    verify_wanted: AtomicU64,
}

impl Inner {
    fn post(&self, post: Post) {
        self.posts.lock().push(post);
    }
}

/// A duel's salt and codes.
#[derive(Clone, Debug)]
struct Salted {
    duel: DuelId,
    code: String,
    salt: u64,
    codes: Option<ShareCodes>,
}

/// The duel shell, in [`crate::game::Game`]: the duels' salts and codes, the worker's posts, the duel log. Shared by the
/// threads that drain the session (interior mutability, short `parking_lot` locks, never the session's).
#[derive(Debug)]
pub struct Duel {
    inner: Arc<Inner>,
    salted: Mutex<Vec<Salted>>,
    /// The duel whose codes the worker is making.
    making: Mutex<Option<DuelId>>,
    /// What the shell last said, for the card.
    said: Mutex<Option<String>>,
    /// Tasks queued under the session lock, sent to the worker after unlocking ([`Duel::act`]).
    pending: Mutex<Vec<WorkerTask>>,
    /// The shell could not play an audition: the next frame stops it in the session.
    stop_audition: AtomicBool,
    log: Mutex<Option<PathBuf>>,
    /// The last state line written, so only changes are logged.
    logged: Mutex<String>,
}

impl Default for Duel {
    fn default() -> Self {
        Duel {
            inner: Arc::default(),
            salted: Mutex::new(Vec::new()),
            making: Mutex::new(None),
            said: Mutex::new(None),
            pending: Mutex::new(Vec::new()),
            stop_audition: AtomicBool::new(false),
            log: Mutex::new(
                std::env::var_os("ENTH_DUEL_LOG")
                    .filter(|p| !p.is_empty())
                    .map(PathBuf::from),
            ),
            logged: Mutex::new(String::new()),
        }
    }
}

impl Duel {
    /// The duel's salt if it has one (tests: drawn once per duel).
    pub fn salt_of(&self, duel: DuelId) -> Option<u64> {
        self.salted
            .lock()
            .iter()
            .find(|s| s.duel == duel)
            .map(|s| s.salt)
    }

    /// The codes made for `duel`, if any.
    pub fn codes_of(&self, duel: DuelId) -> Option<ShareCodes> {
        self.salted
            .lock()
            .iter()
            .find(|s| s.duel == duel)
            .and_then(|s| s.codes.clone())
    }

    /// Write the duel log to `path` instead of `ENTH_DUEL_LOG` (tests).
    pub fn set_log_path(&self, path: Option<PathBuf>) {
        *self.log.lock() = path;
    }

    /// The salt of `duel` on `code`: the one drawn for it, else a new draw kept with it.
    fn salt(&self, duel: DuelId, code: &str) -> Result<u64, String> {
        let mut salted = self.salted.lock();
        if let Some(s) = salted.iter().find(|s| s.duel == duel && s.code == code) {
            return Ok(s.salt);
        }
        let salt = draw_salt()?;
        salted.push(Salted {
            duel,
            code: code.to_owned(),
            salt,
            codes: None,
        });
        Ok(salt)
    }

    fn say(&self, text: Option<String>) {
        if let Some(t) = &text {
            self.log_line(&serde_json::json!({ "schema": LOG_SCHEMA, "event": "said", "text": t }));
        }
        *self.said.lock() = text;
    }

    /// After the card's **Share result**, under the lock: the player's result (cloned on this click), the duel opened
    /// on its code if none is, the duel's salt, then the codes queued for the worker.
    fn share(&self, session: &mut Session) {
        self.say(None);
        let Some(result) = session.duel_own() else {
            return self.say(Some(
                "Submit once (or give up) in a round before you share it".to_owned(),
            ));
        };
        let own = result.own.clone();
        if !own.can_share() {
            return self.say(Some(if own.assists.revealed {
                "A revealed target gives no scorecard".to_owned()
            } else {
                "Pass the round, or end it, before you share".to_owned()
            }));
        }
        let Some(patch) = result.answer else {
            return self.say(Some(
                "Your answer's patch is not here yet (the target is being prepared again): try again in a moment"
                    .to_owned(),
            ));
        };
        let code = match ChallengeCode::decode(&own.code) {
            Ok(c) => c,
            Err(e) => return self.say(Some(format!("{e}: {}", e.hint()))),
        };
        let text = code.to_string();
        let duel = match session.duel() {
            Some(v) if v.code == text => v.id,
            _ => match session.duel_open(&text) {
                Ok(id) => id,
                Err(e) => return self.say(Some(format!("{e}: {}", e.hint()))),
            },
        };
        let salt = match self.salt(duel, &text) {
            Ok(s) => s,
            Err(e) => return self.say(Some(e)),
        };
        *self.making.lock() = Some(duel);
        let inner = Arc::clone(&self.inner);
        let job = ShareJob {
            code,
            salt,
            patch: *patch,
            own,
        };
        self.pending
            .lock()
            .push(WorkerTask(Box::new(move |_runner, _rate| {
                let made =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| make_codes(&job)))
                        .unwrap_or_else(|_| {
                            Err("making the codes failed (see the crash folder)".to_owned())
                        });
                inner.post(Post::Shared(duel, made));
                None
            })));
    }

    /// After unlocking (every drain of the session, `lib.rs`): the tasks queued under the lock go to the worker, then
    /// the session's duel requests are acted on: a verify job to the worker, a cancel skips it if it has not run, an
    /// audition rendered on the worker and played through the Learn examples slot (not while a round's target holds
    /// that slot), a stop clears it.
    pub fn act(&self, game: &Game, requests: Vec<DuelRequest>) {
        for task in std::mem::take(&mut *self.pending.lock()) {
            game.run_task(task);
        }
        for request in requests {
            match request {
                DuelRequest::Verify(v) => {
                    self.inner.verify_wanted.store(v.job.0, Ordering::Release);
                    let inner = Arc::clone(&self.inner);
                    game.run_task(WorkerTask(Box::new(move |_runner, _rate| {
                        if inner.verify_wanted.load(Ordering::Acquire) != v.job.0 {
                            return None;
                        }
                        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            verify_job(&v)
                        }))
                        .unwrap_or_else(|_| DuelJobResult::Failed {
                            code: "worker_panic".to_owned(),
                            message: "the check failed on the game worker".to_owned(),
                            hint: "see the crash folder, and report it with the codes".to_owned(),
                        });
                        inner.post(Post::Verified(v.job, result));
                        None
                    })));
                }
                DuelRequest::Cancel(job) => {
                    let _ = self.inner.verify_wanted.compare_exchange(
                        job.0,
                        0,
                        Ordering::AcqRel,
                        Ordering::Acquire,
                    );
                }
                DuelRequest::Audition { code, patch, .. } => self.audition(game, code, *patch),
                DuelRequest::StopAudition => stop(game),
                _ => {}
            }
        }
    }

    /// Their patch with the challenge's phrase, at the host's rate, through the Learn examples slot (one example at
    /// 0 dB): never while a round's target holds the slot.
    #[cfg(feature = "learn")]
    fn audition(&self, game: &Game, code: String, patch: Patch) {
        if game.ab.load().ready {
            self.refuse_audition(
                "End your round first: its Target ▶ plays where an audition would",
            );
            return;
        }
        game.listen(&enth_session::learn::ListenState {
            example: Some(0),
            ready: true,
            gains_db: vec![0.0],
        });
        game.run_task(WorkerTask(Box::new(move |_runner, rate| {
            audition_loop(&code, &patch, rate)
        })));
    }

    /// Without the Learn examples slot there is nowhere to play it.
    #[cfg(not(feature = "learn"))]
    fn audition(&self, _game: &Game, _code: String, _patch: Patch) {
        self.refuse_audition("this build has no examples slot to play an audition in");
    }

    fn refuse_audition(&self, why: &str) {
        self.stop_audition.store(true, Ordering::Release);
        self.say(Some(why.to_owned()));
    }

    /// What the card shows of the shell for the open duel.
    fn shell_view(&self, view: Option<&DuelView>) -> ShellDuel {
        let id = view.map(|v| v.id);
        ShellDuel {
            codes: id.and_then(|id| self.codes_of(id)),
            making: self.making.lock().is_some(),
            said: self.said.lock().clone(),
        }
    }

    fn log_line(&self, line: &serde_json::Value) {
        use std::io::Write as _;
        let Some(path) = self.log.lock().clone() else {
            return;
        };
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
        {
            let _ = writeln!(f, "{line}");
        }
    }

    /// A state line when the duel's state changed since the last one.
    fn log_state(&self, view: Option<&DuelView>) {
        if self.log.lock().is_none() {
            return;
        }
        let line = state_line(view);
        let text = line.to_string();
        let mut last = self.logged.lock();
        if *last != text {
            text.clone_into(&mut last);
            drop(last);
            self.log_line(&line);
        }
    }
}

/// An audition stopped: the examples slot cleared.
#[cfg(feature = "learn")]
fn stop(game: &Game) {
    game.listen(&enth_session::learn::ListenState::default());
}

/// Without the examples slot nothing plays.
#[cfg(not(feature = "learn"))]
fn stop(_game: &Game) {}

/// The duel's state as the log's line.
fn state_line(view: Option<&DuelView>) -> serde_json::Value {
    let Some(v) = view else {
        return serde_json::json!({ "schema": LOG_SCHEMA, "event": "state", "duel": false });
    };
    let answer = match v.answer {
        AnswerState::None => "none",
        AnswerState::Sealed => "sealed",
        AnswerState::Open => "open",
        _ => "other",
    };
    let verify = match &v.verify {
        VerifyState::Idle => "idle".to_owned(),
        VerifyState::Running => "running".to_owned(),
        VerifyState::Done => "done".to_owned(),
        VerifyState::Failed(f) => format!("failed:{}", f.code),
        _ => "other".to_owned(),
    };
    let checked = v.checked.as_ref().map(|c| {
        serde_json::json!({
            "theirs_sound": c.theirs.sound,
            "theirs_passed": c.theirs.passed,
            "own_sound": c.own.map(|o| o.sound),
            "as_reported": c.matches_claim(),
            "commitment_ok": c.commitment_ok(),
            "has_patch": c.has_patch,
        })
    });
    serde_json::json!({
        "schema": LOG_SCHEMA,
        "event": "state",
        "duel": true,
        "code": v.code,
        "own": v.own.as_ref().map(DuelOwn::can_share),
        "card": v.card.is_some(),
        "answer": answer,
        "verify": verify,
        "checked": checked,
        "used": v.used,
        "auditioning": v.auditioning,
    })
}

/// Before a frame, under the session lock: the worker's posts handed back (a verify result to the session, codes kept
/// with their duel), a refused audition stopped, then the session's duel and the shell's codes to the editor.
pub fn before_frame(editor: &mut Editor, session: &mut Session, game: &Game) {
    let duel = &game.duel;
    let posts = std::mem::take(&mut *duel.inner.posts.lock());
    for post in posts {
        match post {
            // A result the duel no longer waits for is refused as stale, harmlessly.
            Post::Verified(job, result) => {
                let _ = session.duel_job_done(job, result);
            }
            Post::Shared(id, made) => {
                let mut making = duel.making.lock();
                if *making == Some(id) {
                    *making = None;
                }
                drop(making);
                match made {
                    Ok(codes) => {
                        duel.log_line(&serde_json::json!({
                            "schema": LOG_SCHEMA,
                            "event": "share",
                            "code": codes.code,
                            "scorecard": codes.scorecard,
                            "answer": codes.answer,
                            "scorecard_symbols": codes.scorecard_symbols,
                            "answer_symbols": codes.answer_symbols,
                        }));
                        if let Some(s) = duel.salted.lock().iter_mut().find(|s| s.duel == id) {
                            s.codes = Some(codes);
                        }
                    }
                    Err(e) => duel.say(Some(e)),
                }
            }
        }
    }
    if duel.stop_audition.swap(false, Ordering::AcqRel) {
        session.duel_stop_audition();
    }
    let view = DuelEdit::duel(session);
    duel.log_state(view.as_ref());
    let shell = duel.shell_view(view.as_ref());
    editor.set_duel(view, shell);
}

/// After a frame, under the session lock: the card's intents applied to the session's duel, then **Share** (the codes
/// queued for the worker, sent after unlocking).
pub fn after_frame(editor: &mut Editor, session: &mut Session, game: &Game) {
    editor.apply_duel(session);
    if editor.take_duel_share() {
        game.duel.share(session);
    }
    game.duel.log_state(DuelEdit::duel(session).as_ref());
}

/// What the codes are made from.
#[derive(Debug)]
struct ShareJob {
    code: ChallengeCode,
    salt: u64,
    patch: Patch,
    own: DuelOwn,
}

/// Symbols in a code's text (hyphens and spaces not counted; code v1's check symbol included).
fn symbols(text: &str) -> usize {
    text.chars().filter(char::is_ascii_alphanumeric).count()
}

/// The codes, on the worker: `AnswerCode::new(challenge, salt, &patch)`, the round prepared once with the round's own
/// scorer set, the **canonical answer** scored on it (what any receiver scores, so a check on the same engine reads
/// "as reported"), then `Scorecard::new(…, answer.commitment())` with the compared submit's number, the time (to the
/// pass, else the round's active time) and the assists.
fn make_codes(job: &ShareJob) -> Result<ShareCodes, String> {
    let said = |e: &dyn std::fmt::Display, hint: String| format!("{e}: {hint}");
    let answer = AnswerCode::new(job.code, job.salt, &job.patch).map_err(|e| said(&e, e.hint()))?;
    let scoring = job
        .own
        .scoring
        .as_ref()
        .ok_or("the round's scoring is not known yet: try again once its target is ready")?;
    let scorers = ScorerSet::of(scoring, job.code.spec().difficulty)
        .ok_or("this round's scorers are not a frozen scorer set, so it cannot be shared")?;
    let round = verify::prepare(&answer, scorers).map_err(|e| said(&e, e.hint()))?;
    let report = score_round(&round, answer.answer()).map_err(|e| said(&e, e.hint()))?;
    let submits = job.own.number.unwrap_or(job.own.submits).max(1);
    let time_ms = job.own.time_ms.unwrap_or(job.own.active_ms);
    let card = Scorecard::new(
        job.code,
        &report,
        &round.challenge().scoring,
        submits,
        time_ms,
        &job.own.assists,
        answer.commitment(),
    )
    .map_err(|e| said(&e, e.hint()))?;
    let [a, b] = card.share_lines();
    let scorecard = card.encode();
    Ok(ShareCodes {
        code: job.code.to_string(),
        message: format!("{a}\n{b}\n{scorecard}"),
        scorecard_symbols: symbols(&scorecard),
        scorecard,
        answer: answer.encode(),
        answer_symbols: answer.symbols(),
    })
}

/// A score in the session's hundredths.
fn rescored(report: &enth_challenge::ScoreReport) -> Rescored {
    let s = enth_session::game::summary(report);
    Rescored::new(s.sound, s.combined, s.passed)
}

fn failed(code: &str, message: impl Into<String>, hint: impl Into<String>) -> DuelJobResult {
    DuelJobResult::Failed {
        code: code.to_owned(),
        message: message.into(),
        hint: hint.into(),
    }
}

/// The verify job, on the worker (the F1e report's order): their answer decoded (the target regenerated, the ops
/// applied, the tail checked) and their scorecard; the round prepared **once** with `verify::scorer_set(card)`; theirs
/// verified on it (`verify_prepared`, the claim compared); your patch re-scored on the same round as its **canonical**
/// answer (the patch an answer code of it carries, so both machines score the same patches; the salt plays no part in
/// it and the code is never shown); posted as `DuelVerified…with_patch(their answer).with_claim(v.claim)`. A refusal
/// (`answer_too_new`, `bad_check`, `answer_mismatch`, `target_mismatch`, another challenge) is the job's failure.
pub fn verify_job(v: &DuelVerify) -> DuelJobResult {
    let answer = match AnswerCode::decode(&v.answer) {
        Ok(a) => a,
        Err(e) => return failed(e.code(), e.to_string(), e.hint()),
    };
    if answer.challenge().to_string() != v.code {
        return failed(
            "not_the_same_challenge",
            format!(
                "their answer is for {}, the duel is on {}",
                answer.challenge(),
                v.code
            ),
            "paste the answer code of this duel's challenge",
        );
    }
    let card = match v.card.as_deref().map(Scorecard::decode).transpose() {
        Ok(c) => c,
        Err(e) => return failed(e.code(), e.to_string(), e.hint()),
    };
    let round = match verify::prepare(&answer, verify::scorer_set(card.as_ref())) {
        Ok(r) => r,
        Err(e) => return failed(e.code(), e.to_string(), e.hint()),
    };
    let theirs = match verify::verify_prepared(&round, &answer, card.as_ref()) {
        Ok(t) => t,
        Err(e) => return failed(e.code(), e.to_string(), e.hint()),
    };
    let own = match &v.own {
        Some(patch) => {
            let canonical = AnswerCode::new(*answer.challenge(), 0, patch)
                .map_or_else(|_| Patch::clone(patch), |c| c.answer().clone());
            match score_round(&round, &canonical) {
                Ok(r) => Some(rescored(&r)),
                Err(e) => return failed(e.code(), e.to_string(), e.hint()),
            }
        }
        None => None,
    };
    DuelJobResult::Verified(Box::new(
        DuelVerified::new(own, rescored(&theirs.report))
            .with_patch(answer.answer().clone())
            .with_claim(theirs.claim),
    ))
}

/// Their patch playing the challenge's phrase at `rate` (the scored rate before the first activation) with the scored
/// seed, as a set of one example.
#[cfg(feature = "learn")]
fn audition_loop(code: &str, patch: &Patch, rate: u32) -> Option<Box<TargetLoop>> {
    let code = ChallengeCode::decode(code).ok()?;
    let generated = enth_challenge::random::target(code.spec()).ok()?;
    let rate = if rate == 0 {
        enth_challenge::model::SCORING_SAMPLE_RATE
    } else {
        rate
    };
    let out = enth_render::render(
        patch,
        &generated.phrase().performance(),
        &enth_render::RenderSpec::new(rate, enth_challenge::model::SCORING_SEED),
    )
    .ok()?;
    Some(Box::new(TargetLoop::examples(vec![out])))
}
