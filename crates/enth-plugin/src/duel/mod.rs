//! **Owner: lane SKD** (the Duel skeleton, freeze point F2d; feature `duel`), then **DUI** (the history file,
//! [`history`], and its wiring here, additively). Pre-declared by lane S0e (R4/R5, freeze point F0e).
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
//!   or a seed** (the answer code carries it in the clear): no OS randomness, no codes. **It is written to the duel
//!   history with the codes, on the worker, before the codes reach the card** (lane DUI), so Copy is never offered for
//!   a salt that could be lost; when the history cannot be written (no home folder, a full disk) the codes are still
//!   offered and the card says they last only until the plugin closes.
//! - **The duel history** ([`history`], lane DUI): `<Documents>/enth/Duels/duels.json`, one entry per duel: the salt,
//!   your result and codes, their codes, the last check, a name you typed. Kept on Share (the worker, above), on every
//!   Read that imports their codes, on every check and on a name typed (after unlocking, in [`Duel::act`]). **A duel
//!   survives closing the editor, the plugin and the project:** the card's history lists the duels with **Open**, which
//!   restores one (your result through `DuelEdit::duel_set_own`, its salt and codes, their codes, and a new check if it
//!   had one); and a duel that opens on a code you played before (their message pasted the next day) resumes the
//!   newest unfinished entry on that code by itself: the first played attempt at a code stands.
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

pub mod history;

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
    DuelVerify, DuelView, FriendAnswer, FriendScorecard, Rescored, VerifyState,
};
use enth_ui::editor::Editor;
use enth_ui::editor::duel::{HistoryRow, ImportKind, ShareCodes, ShellAsks, ShellDuel};
use parking_lot::Mutex;

use self::history::{Checked, Entry, HistoryFile, Own, next_id, today};

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
    /// The codes (or why not), and why they could not be kept in the history.
    Shared(DuelId, Result<ShareCodes, String>, Option<String>),
}

/// How many history rows the card is handed (it lists the first few; the rest stay in the file).
const ROWS: usize = 50;

/// The duel history as this instance knows it: the file's entries (as last read or written), and which of the
/// session's duels is kept in which entry.
#[derive(Debug, Default)]
struct Store {
    file: HistoryFile,
    entries: Vec<Entry>,
    loaded: bool,
    /// The session's duels and the entries they are kept in.
    bound: Vec<(DuelId, u64)>,
    /// The session's duels whose own result came from another entry than theirs (copied into a new entry).
    seeded: Vec<(DuelId, u64)>,
    /// The session's duels already looked up in the history (a resume happens once, when the duel opens).
    seen: Vec<DuelId>,
}

impl Store {
    fn bound_of(&self, duel: DuelId) -> Option<u64> {
        self.bound.iter().find(|(d, _)| *d == duel).map(|(_, e)| *e)
    }

    fn bind(&mut self, duel: DuelId, entry: u64) {
        self.bound.retain(|(d, _)| *d != duel);
        self.bound.push((duel, entry));
    }

    fn entry(&self, id: u64) -> Option<&Entry> {
        self.entries.iter().find(|e| e.id == id)
    }

    /// The newest entry on `code` that holds your result (the first played attempt at a code stands), leaving out the
    /// sender's own entry when the codes they sent are among its codes (two instances sharing one history file, as in
    /// the REAPER `duel` scenario: you cannot duel yourself).
    fn own_on(&self, code: &str, sent: &[&str]) -> Option<&Entry> {
        self.entries
            .iter()
            .rev()
            .find(|e| e.code == code && e.own.is_some() && !senders(e, sent))
    }

    fn rows(&self) -> Vec<HistoryRow> {
        self.entries
            .iter()
            .rev()
            .take(ROWS)
            .map(Entry::row)
            .collect()
    }
}

/// Whether `entry` is the sender's own: its share holds one of the codes `sent` (their scorecard or answer).
fn senders(entry: &Entry, sent: &[&str]) -> bool {
    entry.own.as_ref().is_some_and(|o| {
        sent.iter()
            .any(|t| !t.is_empty() && (o.scorecard == *t || o.answer == *t))
    })
}

/// What the worker's tasks share with the shell.
#[derive(Debug, Default)]
struct Inner {
    posts: Mutex<Vec<Post>>,
    /// The verify job the session waits for (0: none): a cancelled job that has not run yet is skipped.
    verify_wanted: AtomicU64,
    /// The duel history (lane DUI). Short locks; never the session's.
    store: Mutex<Store>,
    /// Held across a history file read or write, so this instance's writes keep their order.
    io: Mutex<()>,
}

impl Inner {
    fn post(&self, post: Post) {
        self.posts.lock().push(post);
    }

    /// Read the history file into the store.
    fn load(&self) {
        let _io = self.io.lock();
        let file = self.store.lock().file.clone();
        let entries = file.load();
        let mut store = self.store.lock();
        store.entries = entries;
        store.loaded = true;
    }

    /// Change the history entry `duel` (on `code`) is kept in, made when it has none (with your result and salt
    /// copied from the entry it was resumed from, if any): under the file's lock, then the store follows the file.
    fn keep(
        &self,
        duel: DuelId,
        code: &str,
        change: impl FnOnce(&mut Entry),
    ) -> Result<u64, String> {
        let _io = self.io.lock();
        let (file, bound, seed) = {
            let store = self.store.lock();
            let seed = store
                .seeded
                .iter()
                .find(|(d, _)| *d == duel)
                .map(|(_, e)| *e);
            (store.file.clone(), store.bound_of(duel), seed)
        };
        let (id, entries) = file
            .update(|entries| {
                let at =
                    bound.and_then(|id| entries.iter().position(|e| e.id == id && e.code == code));
                let at = at.unwrap_or_else(|| {
                    let mut entry = Entry {
                        id: next_id(entries),
                        code: code.to_owned(),
                        day: today(),
                        ..Entry::default()
                    };
                    if let Some(from) = seed.and_then(|s| entries.iter().find(|e| e.id == s)) {
                        entry.salt.clone_from(&from.salt);
                        entry.own.clone_from(&from.own);
                        entry.day = from.day;
                    }
                    entries.push(entry);
                    entries.len() - 1
                });
                change(&mut entries[at]);
                entries[at].id
            })
            .map_err(|e| format!("this duel could not be kept on this computer ({e})"))?;
        let mut store = self.store.lock();
        store.entries = entries;
        store.loaded = true;
        store.bind(duel, id);
        Ok(id)
    }
}

/// A change to the open duel's history entry, made after unlocking ([`Duel::act`]).
#[derive(Debug)]
enum Keep {
    /// Their scorecard, as read.
    Card(String),
    /// Their answer code, as read.
    Answer(String),
    /// What a check measured.
    Checked(Checked),
    /// Their name, as typed.
    Name(String),
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
    /// History changes queued under the session lock, written after unlocking ([`Duel::act`]).
    keeps: Mutex<Vec<(DuelId, String, Keep)>>,
    /// The shell could not play an audition: the next frame stops it in the session.
    stop_audition: AtomicBool,
    log: Mutex<Option<PathBuf>>,
    /// The last state line written, so only changes are logged.
    logged: Mutex<String>,
}

impl Default for Duel {
    fn default() -> Self {
        let inner = Inner::default();
        // The history file where the environment puts it; none in unit tests, which set one with
        // `Duel::set_history_path` (the user's Documents are never touched by a test).
        if cfg!(not(test)) {
            inner.store.lock().file =
                HistoryFile::from_env(&crate::presets::SystemEnv, crate::presets::Os::current());
        }
        Duel {
            inner: Arc::new(inner),
            salted: Mutex::new(Vec::new()),
            making: Mutex::new(None),
            said: Mutex::new(None),
            pending: Mutex::new(Vec::new()),
            keeps: Mutex::new(Vec::new()),
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

    /// Keep the duel history in the file at `path` instead of the environment's (tests); it is read again.
    pub fn set_history_path(&self, path: Option<PathBuf>) {
        let mut store = self.inner.store.lock();
        store.file = HistoryFile::new(path);
        store.loaded = false;
    }

    /// The duel history as this instance last read or wrote it, oldest first (tests).
    pub fn history(&self) -> Vec<Entry> {
        self.inner.store.lock().entries.clone()
    }

    /// The history entry `duel` is kept in, if any.
    pub fn entry_of(&self, duel: DuelId) -> Option<u64> {
        self.inner.store.lock().bound_of(duel)
    }

    /// Keep `salt` and `codes` for `duel` on `code` (a duel restored from the history).
    fn restore_codes(
        &self,
        duel: DuelId,
        code: &str,
        salt: Option<u64>,
        codes: Option<ShareCodes>,
    ) {
        let Some(salt) = salt else { return };
        let mut salted = self.salted.lock();
        salted.retain(|s| s.duel != duel);
        salted.push(Salted {
            duel,
            code: code.to_owned(),
            salt,
            codes,
        });
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
        let code = match ChallengeCode::decode(&own.code) {
            Ok(c) => c,
            Err(e) => return self.say(Some(format!("{e}: {}", e.hint()))),
        };
        let text = code.to_string();
        // Codes made once (or restored from the history) stand: a second Share gives the same codes.
        let made = |duel: DuelId| {
            self.salted
                .lock()
                .iter()
                .any(|s| s.duel == duel && s.code == text && s.codes.is_some())
        };
        if session.duel().is_some_and(|v| v.code == text && made(v.id)) {
            return;
        }
        let Some(patch) = result.answer else {
            return self.say(Some(
                "Your answer's patch is not here yet (the target is being prepared again): try again in a moment"
                    .to_owned(),
            ));
        };
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
                // The salt and the codes go to the history before the card can offer Copy.
                let unkept = match &made {
                    Ok(codes) => inner
                        .keep(duel, &codes.code, |e| {
                            e.set_salt(job.salt);
                            e.own = Some(Own::of(&job.own, codes));
                            e.day = today();
                        })
                        .err()
                        .map(|e| format!("{e}: your codes last until the plugin closes")),
                    Err(_) => None,
                };
                inner.post(Post::Shared(duel, made, unkept));
                None
            })));
    }

    /// After unlocking (every drain of the session, `lib.rs`): the tasks queued under the lock go to the worker, then
    /// the session's duel requests are acted on: a verify job to the worker, a cancel skips it if it has not run, an
    /// audition rendered on the worker and played through the Learn examples slot (not while a round's target holds
    /// that slot), a stop clears it.
    pub fn act(&self, game: &Game, requests: Vec<DuelRequest>) {
        for (duel, code, keep) in std::mem::take(&mut *self.keeps.lock()) {
            let kept = self.inner.keep(duel, &code, |e| match keep {
                Keep::Card(text) => e.card = Some(text),
                Keep::Answer(text) => e.answer = Some(text),
                Keep::Checked(c) => e.checked = Some(c),
                Keep::Name(name) => e.set_name(&name),
            });
            if let Err(e) = kept {
                self.say(Some(e));
            }
        }
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

    /// What the card shows of the shell for the open duel, and the history.
    fn shell_view(&self, view: Option<&DuelView>) -> ShellDuel {
        let id = view.map(|v| v.id);
        let store = self.inner.store.lock();
        let entry = id.and_then(|id| store.bound_of(id));
        ShellDuel {
            codes: id.and_then(|id| self.codes_of(id)),
            making: self.making.lock().is_some(),
            said: self.said.lock().clone(),
            history: store.rows(),
            entry,
            name: entry
                .and_then(|e| store.entry(e))
                .map(|e| e.name.clone())
                .unwrap_or_default(),
        }
    }

    /// Queue a change to the history entry of the open duel, written after unlocking.
    fn queue_keep(&self, view: &DuelView, keep: Keep) {
        self.keeps.lock().push((view.id, view.code.clone(), keep));
    }

    /// Restore `entry` into the session's duel (its view `view`): your result (from this entry, else from the newest
    /// entry on its code that has one; over a live round's on the same code only when `first_stands`, as **Open**
    /// does), its salt and codes, and their codes the session does not hold yet.
    fn restore(
        &self,
        session: &mut Session,
        view: &DuelView,
        entry: &Entry,
        own_from: Option<&Entry>,
        first_stands: bool,
    ) {
        let own_from = own_from.filter(|_| first_stands || view.own.is_none());
        if let Some(from) = own_from
            && let Some(own) = &from.own
        {
            let _ = session.duel_set_own(own.result(&view.code));
            self.restore_codes(
                view.id,
                &view.code,
                from.salt(),
                Some(own.codes(&view.code)),
            );
        }
        if view.card.is_none()
            && let Some(text) = &entry.card
            && let Ok(card) = enth_challenge::result::Scorecard::decode(text)
        {
            let _ = session.duel_import_scorecard(FriendScorecard::new(card, text.clone()));
        }
        if view.answer == AnswerState::None
            && let Some(text) = &entry.answer
        {
            let _ = session.duel_import_answer(FriendAnswer::new(view.code.clone(), text.clone()));
        }
    }

    /// A duel that just opened, looked up once: if you have not played it here, the newest entry on its code with your
    /// result restores it (the first played attempt stands), and the newest unfinished entry on its code whose
    /// scorecard does not differ from the duel's keeps it.
    fn resume(&self, session: &mut Session, imported: &[enth_ui::editor::duel::Imported]) {
        let Some(view) = DuelEdit::duel(session) else {
            return;
        };
        let load = {
            let mut store = self.inner.store.lock();
            if store.seen.contains(&view.id) {
                return;
            }
            store.seen.push(view.id);
            if store.bound_of(view.id).is_some() || view.own.is_some() {
                return;
            }
            !store.loaded
        };
        // The file as it is now (another instance may have played this code since).
        if load {
            self.inner.load();
        }
        let (entry, own_from) = {
            let mut store = self.inner.store.lock();
            let card = view.card.as_ref().map(|c| c.text.as_str());
            let sent: Vec<&str> = card
                .into_iter()
                .chain(imported.iter().map(|i| i.text.as_str()))
                .collect();
            let entry = store
                .entries
                .iter()
                .rev()
                .find(|e| {
                    e.code == view.code
                        && !e.finished()
                        && !senders(e, &sent)
                        && (card.is_none() || e.card.is_none() || e.card.as_deref() == card)
                })
                .cloned();
            let own_from = store.own_on(&view.code, &sent).cloned();
            match (&entry, &own_from) {
                (Some(e), _) => store.bind(view.id, e.id),
                (None, Some(from)) => store.seeded.push((view.id, from.id)),
                (None, None) => return,
            }
            (entry, own_from)
        };
        let empty = Entry::default();
        let entry = entry.as_ref().unwrap_or(&empty);
        let own_from = entry.own.is_some().then_some(entry).or(own_from.as_ref());
        self.restore(session, &view, entry, own_from, false);
    }

    /// **Open** on a history row: a duel opens on its code (the open one closes), kept in that entry, restored, and
    /// checked again if it was checked before.
    fn open(&self, session: &mut Session, id: u64) {
        if !self.inner.store.lock().loaded {
            self.inner.load();
        }
        let (entry, own_from) = {
            let store = self.inner.store.lock();
            let Some(entry) = store.entry(id).cloned() else {
                drop(store);
                return self.say(Some("that duel is no longer in the history".to_owned()));
            };
            let sent: Vec<&str> = [entry.card.as_deref(), entry.answer.as_deref()]
                .into_iter()
                .flatten()
                .collect();
            let own_from = store.own_on(&entry.code, &sent).cloned();
            (entry, own_from)
        };
        let duel = match session.duel_open(&entry.code) {
            Ok(d) => d,
            Err(e) => return self.say(Some(format!("{e}: {}", e.hint()))),
        };
        {
            let mut store = self.inner.store.lock();
            store.bind(duel, entry.id);
            store.seen.push(duel);
        }
        let Some(view) = DuelEdit::duel(session) else {
            return;
        };
        let own_from = if entry.own.is_some() {
            Some(&entry)
        } else {
            own_from.as_ref()
        };
        self.restore(session, &view, &entry, own_from, true);
        if entry.checked.is_some() {
            let _ = session.duel_verify();
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
            // A result the duel no longer waits for is refused as stale, harmlessly. A check kept in the history.
            Post::Verified(job, result) => {
                if session.duel_job_done(job, result).is_ok()
                    && let Some(view) = DuelEdit::duel(session)
                    && let Some(c) = &view.checked
                {
                    duel.queue_keep(&view, Keep::Checked(Checked::of(c)));
                }
            }
            Post::Shared(id, made, unkept) => {
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
                        duel.say(unkept);
                    }
                    Err(e) => duel.say(Some(e)),
                }
            }
        }
    }
    // The history: read when the card shows and it has not been read (another instance may have written since).
    let card_open = editor.duel().card_open();
    let read = {
        let store = duel.inner.store.lock();
        card_open && !store.loaded
    };
    if read {
        duel.inner.load();
    }
    if !card_open {
        duel.inner.store.lock().loaded = false;
    }
    duel.resume(session, &[]);
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
    let duel = &game.duel;
    editor.apply_duel(session);
    let ShellAsks {
        share,
        open,
        imported,
        name,
    } = editor.take_duel_asks();
    if let Some(id) = open {
        duel.open(session, id);
    }
    // A duel a Read opened resumes from the history before what it imported is kept.
    duel.resume(session, &imported);
    if let Some(view) = DuelEdit::duel(session) {
        for i in imported {
            match i.kind {
                ImportKind::Scorecard => duel.queue_keep(&view, Keep::Card(i.text)),
                ImportKind::Answer => duel.queue_keep(&view, Keep::Answer(i.text)),
                // A challenge alone keeps nothing: the duel is kept once something of it is.
                ImportKind::Challenge => {}
            }
        }
        if let Some(name) = name {
            duel.queue_keep(&view, Keep::Name(name));
        }
    }
    if share {
        duel.share(session);
    }
    duel.log_state(DuelEdit::duel(session).as_ref());
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
