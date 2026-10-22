//! A duel through two real plugin instances (lane SKD, F2d): [`Enth`] driven through fake nice-plug contexts, with the
//! real game worker running the real duel jobs, and each instance's editor glue doing what it does every frame
//! ([`crate::duel::before_frame`], then the card's intents, then [`crate::duel::after_frame`]), without the drawing.
//!
//! What is checked: A passes a round and shares it (the salt drawn once from OS randomness and kept with the duel, the
//! codes made on the worker: a 51-symbol scorecard whose commitment is the answer code's, the share message's two
//! lines); a second Share gives the same codes; B reads A's whole message (the scorecard, held as reported, the duel
//! opened on its challenge) and A's answer (sealed until B has played); B gives up, checks A's answer on the worker
//! ("as reported", the commitment matched, the canonical answer's score), auditions it through the examples slot, then
//! loads it as one undo step with no host writes; the duel log records the share and the states; a refusal from the
//! worker (another challenge's answer) reaches the card as the check's failure.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use enth_challenge::answer::AnswerCode;
use enth_challenge::result::{SCORECARD_SYMBOLS, Scorecard};
use enth_challenge::spec::{LATEST_GENERATOR, SpecPreset};
use enth_challenge::{ChallengeCode, prepare_round};
use enth_patch::ops_between;
use enth_session::Session;
use enth_session::duel::{AnswerState, DuelEdit, VerifyState};
use enth_session::game::{EditorView, EndChoice, GameEdit, Phase, RoundSource};
use enth_ui::editor::Editor;
use enth_ui::editor::cards::Intent;
use enth_ui::editor::duel::DuelIntent;
use nice_plug::prelude::*;

use crate::{Enth, Task};

const SR: u32 = 48_000;

#[derive(Default)]
struct Ctx {
    transport: Option<Transport>,
}

impl ProcessContext<Enth> for Ctx {
    fn plugin_api(&self) -> PluginApi {
        PluginApi::Clap
    }

    fn execute_background(&self, task: Task) {
        assert_eq!(task, Task::Pump);
    }

    fn execute_gui(&self, _task: Task) {
        panic!("process() must never schedule GUI work");
    }

    fn transport(&self) -> &Transport {
        self.transport.as_ref().expect("set by the host")
    }

    fn next_event(&mut self) -> Option<NoteEvent<()>> {
        None
    }

    fn try_send_event(
        &mut self,
        _event: NoteEvent<()>,
    ) -> Result<(), (NoteEvent<()>, nice_plug::context::process::SendEventError)> {
        Ok(())
    }

    fn set_latency_samples(&self, _samples: u32) {}

    fn request_restart(&self) {}

    fn request_param_info_rescan(&self) {
        panic!("process() must never request a rescan");
    }

    fn set_current_voice_capacity(&self, _capacity: u32) {}
}

struct ActCtx;

impl ActivateContext<Enth> for ActCtx {
    fn plugin_api(&self) -> PluginApi {
        PluginApi::Clap
    }

    fn execute(&self, _task: Task) {}

    fn set_latency_samples(&self, _samples: u32) {}

    fn set_current_voice_capacity(&self, _capacity: u32) {}
}

/// A fresh folder under the system's temporary folder, removed when dropped.
struct TempDir(std::path::PathBuf);

impl TempDir {
    fn new(label: &str) -> Self {
        static N: AtomicU32 = AtomicU32::new(0);
        let path = std::env::temp_dir().join(format!(
            "enth-skd-{label}-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).unwrap();
        TempDir(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// One plugin instance and its editor (never drawn).
struct Host {
    plugin: Enth,
    ctx: Ctx,
    editor: Editor,
    now: u64,
}

impl Host {
    fn new() -> Host {
        let mut plugin = Enth::default();
        let layout = <Enth as Plugin>::AUDIO_IO_LAYOUTS[0];
        let config = BufferConfig {
            sample_rate: SR as f32,
            min_buffer_size: None,
            max_buffer_size: 4096,
            process_mode: ProcessMode::Realtime,
        };
        assert!(plugin.activate(&layout, &config, &mut ActCtx));
        Host {
            plugin,
            ctx: Ctx {
                transport: Some(Transport::new(SR as f32)),
            },
            editor: Editor::default(),
            now: 1_000,
        }
    }

    /// One editor frame under the lock, as the glue makes it: the clock and the round's results, the duel's posts and
    /// read model, then `f` (the session calls, and the card's intents through `card`), then the card's intents applied
    /// and Share taken. The duel requests are acted on after unlocking (`with_session`).
    fn frame<R>(&mut self, card: Vec<DuelIntent>, f: impl FnOnce(&mut Session) -> R) -> R {
        self.now += 16;
        let now = self.now;
        let shared = Arc::clone(self.plugin.shared());
        let editor = &mut self.editor;
        let (r, writes) = shared.with_session(|s| {
            s.game_tick(now, EditorView::Visible);
            for (id, result) in shared.game.take_results() {
                let _ = s.game_job_done(id, result);
            }
            crate::duel::before_frame(editor, s, &shared.game);
            let r = f(s);
            let intents = card.into_iter().map(Intent::Duel).collect();
            editor.apply(s, intents, false);
            crate::duel::after_frame(editor, s, &shared.game);
            r
        });
        assert!(writes.is_empty(), "duels make no host writes");
        r
    }

    fn session<R>(&mut self, f: impl FnOnce(&mut Session) -> R) -> R {
        self.frame(Vec::new(), f)
    }

    /// Frames until `done` holds (the worker runs in the background), at most 60 s.
    fn until(&mut self, what: &str, mut done: impl FnMut(&mut Session) -> bool) {
        let t0 = Instant::now();
        while !self.session(&mut done) {
            assert!(t0.elapsed() < Duration::from_secs(60), "{what}: timed out");
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    fn render(&mut self, frames: usize) -> Vec<f32> {
        let mut left = vec![0.0f32; frames];
        let mut right = vec![0.0f32; frames];
        for (l, r) in left.chunks_mut(256).zip(right.chunks_mut(256)) {
            let n = l.len();
            let mut buffer = Buffer::default();
            // SAFETY: both slices live until the end of this block and have `n` frames.
            unsafe {
                buffer.set_slices(n, |s| {
                    *s = vec![l, r];
                });
            }
            let mut aux = AuxiliaryBuffers {
                inputs: &mut [],
                outputs: &mut [],
            };
            let _ = self.plugin.process(&mut buffer, &mut aux, &mut self.ctx);
        }
        left
    }

    /// Start a round on `code` and wait until it plays.
    fn play(&mut self, code: &ChallengeCode) {
        self.session(|s| {
            s.game_tick(1_000, EditorView::Opened);
            s.game_start(RoundSource::Code(*code)).unwrap();
        });
        self.until("the prepared target", |s| {
            s.round().is_some_and(|r| r.phase == Phase::Playing)
        });
    }
}

fn rms(x: &[f32]) -> f64 {
    (x.iter().map(|v| f64::from(*v) * f64::from(*v)).sum::<f64>() / x.len().max(1) as f64).sqrt()
}

fn level2(seed: u64) -> ChallengeCode {
    ChallengeCode::short(SpecPreset::by_id(2).unwrap().spec(LATEST_GENERATOR, seed)).unwrap()
}

#[test]
fn a_duel_between_two_instances_shares_reads_checks_and_loads() {
    let logs = TempDir::new("duel-log");
    let log_a = logs.0.join("a.jsonl");
    let code = level2(77);
    let round = prepare_round(code.spec()).unwrap();

    // A passes: the target rebuilt by edits, submitted.
    let mut a = Host::new();
    a.plugin
        .shared()
        .game
        .duel
        .set_log_path(Some(log_a.clone()));
    a.play(&code);
    a.session(|s| {
        let ops = ops_between(s.patch(), round.target()).unwrap();
        s.edit(ops, None).unwrap();
        s.game_submit().unwrap();
    });
    a.until("the pass", |s| {
        s.round().is_some_and(|r| r.phase == Phase::Passed)
    });

    // Share: the duel opens on the round's code, the salt is drawn once, the codes are made on the worker.
    let shared_a = Arc::clone(a.plugin.shared());
    let shell = &shared_a.game.duel;
    a.frame(vec![DuelIntent::Share], |_| ());
    a.until("the codes", |s| {
        s.duel().is_some_and(|d| shell.codes_of(d.id).is_some())
    });
    let duel = a.session(|s| s.duel().unwrap());
    let salt = shell.salt_of(duel.id).expect("a salt kept with the duel");
    let codes = shell.codes_of(duel.id).unwrap();
    assert_eq!(codes.code, code.to_string());
    assert_eq!(codes.scorecard_symbols, SCORECARD_SYMBOLS);
    let card = Scorecard::decode(&codes.scorecard).unwrap();
    let answer = AnswerCode::decode(&codes.answer).unwrap();
    assert_eq!(answer.salt(), salt);
    assert_eq!(
        card.fields().commitment,
        answer.commitment(),
        "the scorecard fixes this answer"
    );
    assert!(card.fields().passed);
    assert_eq!(card.fields().submits, 1, "submits up to the compared one");
    let [first, second] = card.share_lines();
    assert_eq!(
        codes.message,
        format!("{first}\n{second}\n{}", codes.scorecard)
    );
    assert!(first.starts_with("enth duel · "), "{first}");
    // A second Share: the same salt, the same codes.
    a.frame(vec![DuelIntent::Share], |_| ());
    a.until("the codes again", |s| {
        s.duel().is_some_and(|d| shell.codes_of(d.id).is_some())
    });
    assert_eq!(shell.salt_of(duel.id), Some(salt));
    assert_eq!(shell.codes_of(duel.id).unwrap().answer, codes.answer);
    a.session(|s| s.game_end(EndChoice::Keep).unwrap());

    // B reads A's whole message: their scorecard as reported, a duel on its challenge.
    let mut b = Host::new();
    let message = format!("beat this \u{2014}\n{}\n", codes.message);
    b.frame(
        vec![DuelIntent::PasteText(message), DuelIntent::Read],
        |_| (),
    );
    let view = b.session(|s| s.duel().expect("a duel opened on their card"));
    assert_eq!(view.code, code.to_string());
    assert_eq!(
        view.card.as_ref().map(|c| c.card.clone()),
        Some(card.clone())
    );
    // A's answer: held, sealed.
    b.frame(
        vec![
            DuelIntent::PasteText(codes.answer.clone()),
            DuelIntent::Read,
        ],
        |_| (),
    );
    let view = b.session(|s| s.duel().unwrap());
    assert_eq!(view.answer, AnswerState::Sealed);
    assert_eq!(
        b.session(|s| s.duel_verify().map_err(|e| e.code())),
        Err("answer_sealed")
    );

    // B plays and gives up: the answer opens; the check runs on B's worker.
    b.play(&code);
    b.session(|s| s.game_reveal().unwrap());
    b.until("the open answer", |s| {
        s.duel().is_some_and(|d| d.answer == AnswerState::Open)
    });
    b.frame(vec![DuelIntent::Verify], |_| ());
    b.until("the check", |s| {
        s.duel().is_some_and(|d| d.verify == VerifyState::Done)
    });
    let checked = b.session(|s| s.duel().unwrap().checked.unwrap());
    assert_eq!(
        checked.theirs.sound, 10_000,
        "their passing answer is the target"
    );
    assert_eq!(checked.matches_claim(), Some(true), "checked as reported");
    assert_eq!(checked.commitment_ok(), Some(true));
    assert!(checked.own.is_none(), "B gave up with nothing scored");
    assert!(checked.has_patch);

    // Their patch, auditioned through the examples slot once B's round is over, then loaded as one undo step.
    b.session(|s| s.game_end(EndChoice::Restore).unwrap());
    b.frame(vec![DuelIntent::Audition], |_| ());
    let t0 = Instant::now();
    let heard = loop {
        b.session(|_| ());
        let out = b.render(SR as usize / 10);
        if rms(&out) > 0.01 {
            break true;
        }
        if t0.elapsed() > Duration::from_secs(30) {
            break false;
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    assert!(heard, "the audition plays their patch");
    b.frame(vec![DuelIntent::StopAudition], |_| ());
    b.frame(vec![DuelIntent::Use], |_| ());
    b.session(|s| {
        let d = s.duel().unwrap();
        assert!(d.used);
        let mut loaded = s.patch().clone();
        loaded.meta = answer.answer().meta.clone();
        loaded.engine.clone_from(&answer.answer().engine);
        assert_eq!(&loaded, answer.answer(), "their canonical answer");
        assert!(
            s.history()
                .next_undo()
                .is_some_and(|step| step.load.is_some())
        );
    });

    // The duel log saw the share and the states, with codes and no name.
    let log = std::fs::read_to_string(&log_a).unwrap();
    let lines: Vec<serde_json::Value> = log
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert!(lines.iter().all(|l| l["schema"] == crate::duel::LOG_SCHEMA));
    assert!(
        lines
            .iter()
            .any(|l| l["event"] == "share" && l["answer"] == codes.answer.as_str())
    );
    assert!(
        lines
            .iter()
            .any(|l| l["event"] == "state" && l["own"] == true)
    );
}

#[test]
fn salts_are_drawn_from_the_os_and_differ() {
    let draws: Vec<u64> = (0..8).map(|_| crate::duel::draw_salt().unwrap()).collect();
    let mut unique = draws.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(unique.len(), draws.len(), "{draws:?}");
}

/// The worker's refusal reaches the card: an answer code of another challenge, held for this duel (as a shell that
/// mislabelled it would), fails the check with `not_the_same_challenge` instead of being scored.
#[test]
fn an_answer_for_another_challenge_fails_the_check() {
    let code = level2(5);
    let other = level2(6);
    let round = prepare_round(other.spec()).unwrap();
    let theirs = AnswerCode::new(other, 9, round.target()).unwrap();
    let mut b = Host::new();
    b.session(|s| {
        s.duel_open(&code.to_string()).unwrap();
        s.duel_import_answer(enth_session::duel::FriendAnswer::new(
            code.to_string(),
            theirs.encode(),
        ))
        .unwrap();
    });
    b.play(&code);
    b.session(|s| s.game_reveal().unwrap());
    b.frame(vec![DuelIntent::Verify], |_| ());
    b.until("the refusal", |s| {
        s.duel()
            .is_some_and(|d| matches!(d.verify, VerifyState::Failed(_)))
    });
    let state = b.session(|s| s.duel().unwrap().verify);
    let VerifyState::Failed(f) = state else {
        panic!("{state:?}");
    };
    assert_eq!(f.code, "not_the_same_challenge");
    assert_ne!(f.hint, "");
}

/// The REAPER `duel` scenario's first instance starts from this state (`xtask/reaper/enth-duel-round.state.json`, the
/// nice-plug fields `patch` and `enth`): a round on [`fixture_code`], playing, whose patch is the target rebuilt by
/// edits, so its first Submit passes. REAPER loads it as the instance's state; the round restores and its target is
/// prepared again on the worker.
const REAPER_FIXTURE: &str = include_str!("../../../xtask/reaper/enth-duel-round.state.json");

/// The fixture's challenge: level 2, seed 77.
fn fixture_code() -> ChallengeCode {
    level2(77)
}

/// The fixture's fields, made by playing: start the round, rebuild the target by edits, save.
fn fixture_fields() -> BTreeMap<String, String> {
    let code = fixture_code();
    let round = prepare_round(code.spec()).unwrap();
    let mut host = Host::new();
    host.play(&code);
    host.session(|s| {
        let ops = ops_between(s.patch(), round.target()).unwrap();
        s.edit(ops, None).unwrap();
    });
    host.plugin.host_params().serialize_fields()
}

/// Regenerates the fixture. Run it (`cargo test -p enth-plugin --features duel -- --ignored write_the_reaper_duel_fixture`)
/// when `the_reaper_duel_fixture_resumes_a_round_whose_first_submit_passes` fails after a change to the saved state or
/// the engine; the commit says why.
#[test]
#[ignore = "writes xtask/reaper/enth-duel-round.state.json"]
fn write_the_reaper_duel_fixture() {
    let fields = fixture_fields();
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../xtask/reaper/enth-duel-round.state.json"
    );
    let text = serde_json::to_string_pretty(&fields).unwrap() + "\n";
    std::fs::write(path, text).unwrap();
}

#[test]
fn the_reaper_duel_fixture_resumes_a_round_whose_first_submit_passes() {
    let fields: BTreeMap<String, String> = serde_json::from_str(REAPER_FIXTURE).unwrap();
    assert_eq!(
        fields.keys().map(String::as_str).collect::<Vec<_>>(),
        ["enth", "patch"]
    );
    let mut host = Host::new();
    host.plugin.host_params().deserialize_fields(&fields);
    // The saved phase reads `playing` at once; the target is back once the worker has prepared it again.
    host.until("the restored round's target", |s| {
        s.round().is_some_and(|r| r.phase == Phase::Playing) && s.game_ab().target_ready
    });
    host.session(|s| {
        assert_eq!(s.round().unwrap().code, fixture_code().to_string());
        s.game_submit().unwrap();
    });
    host.until("the pass", |s| {
        s.round().is_some_and(|r| r.phase == Phase::Passed)
    });
    // And it is shareable at once.
    let own = host.session(|s| s.duel_own().unwrap());
    assert!(own.own.can_share_answer(), "{:?}", own.own);
}
