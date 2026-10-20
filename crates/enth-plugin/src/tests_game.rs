//! A Recreate round through the real plugin (lane SKR, F2r): [`Enth`] driven through fake nice-plug contexts, with
//! the real game worker running the real jobs. The session calls are the ones the editor glue makes each frame
//! ([`crate::editor::Glue::round_frame`] without the drawing): a tick, the worker's results handed back, then the
//! round's intents; `process()` renders the A/B.
//!
//! What is checked: start → the target prepared on the worker → Target ▶ plays exactly the scored render of the
//! target (`enth render` of the target with its phrase at 48 kHz) → Mine ▶ plays the phrase live through the
//! player's patch at the same position → a scored submit that does not pass → the target rebuilt by edits → a
//! passing submit → End restores the player's patch, the round log has the round's line, and every drain marked the
//! project dirty at most once. A plugin that never starts a round renders as before (the in-process host tests,
//! `host-render` and `cli-equal` cover that bit for bit).

use std::cell::Cell;
use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use enth_challenge::model::{SCORING_SAMPLE_RATE, SCORING_SEED};
use enth_challenge::spec::{LATEST_GENERATOR, SpecPreset};
use enth_patch::{Patch, ops_between};
use enth_render::RenderSpec;
use enth_session::game::{EditorView, EndChoice, GameEdit, Listen, Phase, RoundSource};
use nice_plug::context::process::SendEventError;
use nice_plug::midi::NoteEvent;
use nice_plug::prelude::*;

use crate::{Enth, Task, game};

type Ev = NoteEvent<()>;

const SR: u32 = 48_000;

#[derive(Default)]
struct Ctx {
    input: VecDeque<Ev>,
    transport: Option<Transport>,
    background: Cell<u32>,
}

impl ProcessContext<Enth> for Ctx {
    fn plugin_api(&self) -> PluginApi {
        PluginApi::Clap
    }

    fn execute_background(&self, task: Task) {
        assert_eq!(task, Task::Pump);
        self.background.set(self.background.get() + 1);
    }

    fn execute_gui(&self, _task: Task) {
        panic!("process() must never schedule GUI work");
    }

    fn transport(&self) -> &Transport {
        self.transport.as_ref().expect("set by the host")
    }

    fn next_event(&mut self) -> Option<Ev> {
        self.input.pop_front()
    }

    fn try_send_event(&mut self, _event: Ev) -> Result<(), (Ev, SendEventError)> {
        Ok(())
    }

    fn set_latency_samples(&self, _samples: u32) {}

    fn request_restart(&self) {}

    fn request_param_info_rescan(&self) {
        panic!("process() must never request a rescan");
    }

    fn set_current_voice_capacity(&self, _capacity: u32) {}
}

#[derive(Default)]
struct ActCtx;

impl ActivateContext<Enth> for ActCtx {
    fn plugin_api(&self) -> PluginApi {
        PluginApi::Clap
    }

    fn execute(&self, _task: Task) {}

    fn set_latency_samples(&self, _samples: u32) {}

    fn set_current_voice_capacity(&self, _capacity: u32) {}
}

#[derive(Default)]
struct Requests {
    dirty: AtomicU32,
}

impl nice_plug::context::host::HostRequestsInner for Requests {
    fn mark_dirty(&self) {
        self.dirty.fetch_add(1, Ordering::SeqCst);
    }

    fn request_param_info_rescan(&self) {}
}

/// A fresh folder under the system's temporary folder, removed when dropped.
struct TempDir(std::path::PathBuf);

impl TempDir {
    fn new(label: &str) -> Self {
        static N: AtomicU32 = AtomicU32::new(0);
        let path = std::env::temp_dir().join(format!(
            "enth-skr-{label}-{}-{}",
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

struct Host {
    plugin: Enth,
    ctx: Ctx,
    requests: Arc<Requests>,
    /// The shell's clock for the round (milliseconds), moved by the test.
    now: u64,
}

impl Host {
    fn new(sample_rate: u32) -> Host {
        let mut plugin = Enth::default();
        let requests = Arc::new(Requests::default());
        plugin.host_requests(HostRequests::new(requests.clone()));
        let layout = <Enth as Plugin>::AUDIO_IO_LAYOUTS[0];
        let config = BufferConfig {
            sample_rate: sample_rate as f32,
            min_buffer_size: None,
            max_buffer_size: 4096,
            process_mode: ProcessMode::Realtime,
        };
        assert!(plugin.activate(&layout, &config, &mut ActCtx));
        Host {
            plugin,
            ctx: Ctx {
                transport: Some(Transport::new(sample_rate as f32)),
                ..Ctx::default()
            },
            requests,
            now: 1_000,
        }
    }

    fn dirty(&self) -> u32 {
        self.requests.dirty.load(Ordering::SeqCst)
    }

    /// Render `frames` frames in blocks of `block`.
    fn render(&mut self, frames: usize, block: usize) -> (Vec<f32>, Vec<f32>) {
        let mut left = vec![0.0f32; frames];
        let mut right = vec![0.0f32; frames];
        let mut b0 = 0;
        while b0 < frames {
            let b1 = (b0 + block).min(frames);
            let (l, r) = (&mut left[b0..b1], &mut right[b0..b1]);
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
            b0 = b1;
        }
        (left, right)
    }

    /// One "editor frame" under the lock: the clock, the worker's results handed back, then `intent`.
    fn frame<R>(&mut self, intent: impl FnOnce(&mut enth_session::Session) -> R) -> R {
        self.now += 16;
        let now = self.now;
        let shared = Arc::clone(self.plugin.shared());
        let (r, writes) = shared.with_session(|s| {
            s.game_tick(now, EditorView::Visible);
            for (id, result) in shared.game.take_results() {
                let _ = s.game_job_done(id, result);
            }
            intent(s)
        });
        assert!(writes.is_empty(), "rounds make no host writes");
        r
    }

    /// Frames until `done` holds (the worker runs in the background), at most 60 s.
    fn until(&mut self, what: &str, mut done: impl FnMut(&mut enth_session::Session) -> bool) {
        let t0 = Instant::now();
        while !self.frame(&mut done) {
            assert!(t0.elapsed() < Duration::from_secs(60), "{what}: timed out");
            std::thread::sleep(Duration::from_millis(2));
        }
    }
}

fn rms(x: &[f32]) -> f64 {
    (x.iter().map(|v| f64::from(*v) * f64::from(*v)).sum::<f64>() / x.len().max(1) as f64).sqrt()
}

#[test]
fn a_full_round_through_the_real_plugin_with_the_worker_and_real_jobs() {
    let logs = TempDir::new("round-log");
    let log_file = logs.0.join(game::ROUND_LOG_FILE);
    let mut host = Host::new(SR);
    host.plugin
        .shared()
        .game
        .set_log_path(Some(log_file.clone()));
    let own = host.frame(|s| s.patch().clone());
    let spec = SpecPreset::by_id(1).unwrap().spec(LATEST_GENERATOR, 42);
    let round = enth_challenge::prepare_round(&spec).unwrap();

    // Start: the session asks for the target; the worker prepares it off the GUI thread.
    host.frame(|s| {
        s.game_tick(host_now(), EditorView::Opened);
        s.game_start(RoundSource::Settings(spec)).unwrap();
    });
    assert!(host.plugin.shared().game.worker_started());
    host.until("the prepared target", |s| {
        s.round().is_some_and(|r| r.phase == Phase::Playing)
    });
    host.frame(|s| {
        assert_eq!(s.patch(), round.start(), "the start patch plays");
        assert!(s.game_ab().target_ready);
    });

    // Target ▶: the pre-rendered loop of the scored phrase, from its start, after the 10 ms crossfade exactly the
    // scored audio at the level-match gain (the engine's own output faded out).
    host.frame(|s| s.game_listen(Listen::Target).unwrap());
    let g = host.plugin.shared().game.ab.load().target_gain;
    assert!(g > 0.0 && g <= 1.0);
    let scored = enth_render::render(
        round.target(),
        &round.phrase().performance(),
        &RenderSpec::new(SCORING_SAMPLE_RATE, SCORING_SEED),
    )
    .unwrap();
    let fade = (SR as f32 * game::AB_FADE_MS / 1000.0) as usize;
    let (l, r) = host.render(SR as usize / 2, 256);
    let heard = |x: &[f32]| x.iter().map(|v| v * g).collect::<Vec<f32>>();
    assert_eq!(&l[fade..], &heard(&scored.left[fade..SR as usize / 2])[..]);
    assert_eq!(&r[fade..], &heard(&scored.right[fade..SR as usize / 2])[..]);
    assert!(rms(&l) > 0.01, "Target ▶ is audible: {}", rms(&l));

    // Mine ▶ at the same position: the phrase played live through the start patch (the engine's output, not the
    // loop's), audible.
    host.frame(|s| s.game_listen(Listen::Mine).unwrap());
    let (mine, _) = host.render(SR as usize / 2, 256);
    assert!(
        rms(&mine[fade..]) > 0.01,
        "Mine ▶ is audible: {}",
        rms(&mine)
    );
    assert_ne!(
        &mine[fade..],
        &scored.left[SR as usize / 2 + fade..SR as usize],
        "Mine ▶ is the start patch, not the loop"
    );
    // Off: back to plain playback; the phrase's notes released.
    host.frame(|s| s.game_listen(Listen::Off).unwrap());
    let _ = host.render(SR as usize, 512);

    // A scored submit of the start patch: recorded, not passed (level 1's start is audibly away).
    let dirty_before = host.dirty();
    host.frame(|s| s.game_submit().unwrap());
    host.until("the first score", |s| {
        s.round().is_some_and(|r| r.submits == 1)
    });
    let first = host.frame(|s| s.round().unwrap());
    assert_eq!(first.phase, Phase::Submitted, "{first:?}");
    let report = first.report.clone().unwrap();
    assert_eq!(
        report.sound.scorer,
        round.challenge().scoring.sound,
        "scored by the scorer the challenge names (sound/1 since lane SS)"
    );
    assert!(!report.passed);
    assert!(
        host.dirty() > dirty_before,
        "a scored submit marks the project dirty"
    );

    // Rebuild the target by edits, submit again: it passes with 100 %.
    host.frame(|s| {
        let ops = ops_between(s.patch(), round.target()).unwrap();
        s.edit(ops, None).unwrap();
        s.game_submit().unwrap();
    });
    host.until("the passing score", |s| {
        s.round().is_some_and(|r| r.phase == Phase::Passed)
    });
    let passed = host.frame(|s| s.round().unwrap());
    assert_eq!(passed.last.unwrap().score.sound, 10_000);
    assert!(passed.time_to_pass_ms.is_some());

    // End, restoring the player's own patch; the round's line is in the log, and the player stops.
    let dirty_before = host.dirty();
    host.frame(|s| s.game_end(EndChoice::Restore).unwrap());
    host.frame(|s| {
        assert!(s.round().is_none());
        // The stash comes back through the patch-file text (the session's rule), which stamps this build's engine.
        let mut back = s.patch().clone();
        back.engine.clone_from(&own.engine);
        assert_eq!(back, own, "the player's patch is back");
    });
    assert_eq!(
        host.dirty(),
        dirty_before + 1,
        "the end's dirty mark and its patch change are one mark"
    );
    let log = std::fs::read_to_string(&log_file).unwrap();
    let lines: Vec<serde_json::Value> = log
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(lines.len(), 1, "{log}");
    assert_eq!(lines[0]["schema"], "enth.round-log/1");
    assert_eq!(lines[0]["event"], "end");
    assert_eq!(lines[0]["ended_by"], "restore");
    assert_eq!(lines[0]["result"]["outcome"], "passed");
    assert_eq!(lines[0]["result"]["submits"], 2);
    assert_eq!(
        lines[0]["fingerprint"].as_str().unwrap(),
        round.fingerprint().to_hex()
    );
    // The clear reached the audio thread: the loop is handed back and the player is idle again.
    let t0 = Instant::now();
    loop {
        let _ = host.render(256, 256);
        host.plugin.shared().game.take_results();
        if host.plugin.player.current().is_none_or(|l| l.frames() == 0) && host.plugin.player.idle()
        {
            break;
        }
        assert!(
            t0.elapsed() < Duration::from_secs(30),
            "the loop was never cleared"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// Any timestamp: `Opened` restarts the clock.
fn host_now() -> u64 {
    1_000
}

#[test]
fn at_44_1_khz_target_plays_a_loop_rendered_at_the_hosts_rate() {
    let mut host = Host::new(44_100);
    let spec = SpecPreset::by_id(2).unwrap().spec(LATEST_GENERATOR, 7);
    let round = enth_challenge::prepare_round(&spec).unwrap();
    host.frame(|s| s.game_start(RoundSource::Settings(spec)).unwrap());
    host.until("the prepared target", |s| s.game_ab().target_ready);
    host.frame(|s| s.game_listen(Listen::Target).unwrap());
    let g = host.plugin.shared().game.ab.load().target_gain;
    let at_host = enth_render::render(
        round.target(),
        &round.phrase().performance(),
        &RenderSpec::new(44_100, SCORING_SEED),
    )
    .unwrap();
    let (l, _) = host.render(22_050, 100);
    let fade = (44_100.0 * game::AB_FADE_MS / 1000.0) as usize;
    let heard: Vec<f32> = at_host.left[fade + 1..22_050]
        .iter()
        .map(|v| v * g)
        .collect();
    assert_eq!(&l[fade + 1..], &heard[..]);
}

#[test]
fn no_round_no_worker_and_no_change_to_the_block() {
    let mut host = Host::new(SR);
    host.frame(|s| {
        let level =
            enth_patch::ParamAddr::module(Patch::init().generators[0].id.clone(), "level").unwrap();
        let _ = s.edit(
            vec![enth_patch::Op::SetParam {
                addr: level,
                value: 50.0,
            }],
            None,
        );
    });
    let _ = host.render(4_800, 64);
    assert!(!host.plugin.shared().game.worker_started());
    assert!(host.plugin.player.idle());
}
