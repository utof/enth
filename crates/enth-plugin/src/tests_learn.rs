//! A lesson through the real plugin (lane SKL, F2l): [`Enth`] driven through fake nice-plug contexts, with the real
//! game worker running the real Learn jobs over the bundled example pack (`cutoff`). The session calls are the ones
//! the editor glue makes each frame ([`crate::editor::Glue::round_frame`] without the drawing): a tick, the worker's
//! Learn results handed back, then the lesson's intents; `process()` plays the examples.
//!
//! What is checked: start → the examples rendered on the worker and played from the target slot, exactly the
//! example's render at its level-match gain → Next → a locked edit refused → the cutoff turned down → the sound check
//! rendered on the worker passes → the pass merged into the progress file → the project saved mid-lesson → a fresh
//! instance loads it, reopens the lesson on the worker at the saved step with the step still passed (the progress
//! file read back) and its locks back → the lesson finishes → End restores the player's patch.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use enth_learn::{Player, StepPhase};
use enth_patch::{Op, ParamAddr};
use enth_session::Session;
use enth_session::game::{EditorView, EndChoice};
use enth_session::learn::LearnEdit;
use nice_plug::context::process::SendEventError;
use nice_plug::midi::NoteEvent;
use nice_plug::prelude::*;

use crate::learn::{Lessons, PROGRESS_FILE};
use crate::{Enth, Task, game};

type Ev = NoteEvent<()>;

const SR: u32 = 48_000;

#[derive(Default)]
struct Ctx {
    input: VecDeque<Ev>,
    transport: Option<Transport>,
}

impl ProcessContext<Enth> for Ctx {
    fn plugin_api(&self) -> PluginApi {
        PluginApi::Clap
    }

    fn execute_background(&self, _task: Task) {}

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
            "enth-skl-{label}-{}-{}",
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
    now: u64,
}

impl Host {
    fn new(lessons: &Arc<Lessons>) -> Host {
        let mut plugin = Enth::default();
        plugin.shared().game.set_lessons(Arc::clone(lessons));
        let requests = Arc::new(Requests::default());
        plugin.host_requests(HostRequests::new(requests.clone()));
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
                ..Ctx::default()
            },
            requests,
            now: 1_000,
        }
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

    /// One "editor frame" under the lock: the clock, the worker's Learn results handed back, then `intent`.
    fn frame<R>(&mut self, intent: impl FnOnce(&mut Session) -> R) -> R {
        self.now += 16;
        let now = self.now;
        let shared = Arc::clone(self.plugin.shared());
        let (r, writes) = shared.with_session(|s| {
            s.game_tick(now, EditorView::Visible);
            for (id, result) in shared.game.take_learn_results() {
                let _ = s.learn_job_done(id, result);
            }
            intent(s)
        });
        assert!(writes.is_empty(), "lessons make no host writes");
        r
    }

    /// Frames until `done` holds (the worker runs in the background), at most 60 s.
    fn until(&mut self, what: &str, mut done: impl FnMut(&mut Session) -> bool) {
        let t0 = Instant::now();
        while !self.frame(&mut done) {
            assert!(t0.elapsed() < Duration::from_secs(60), "{what}: timed out");
            std::thread::sleep(Duration::from_millis(2));
        }
    }
}

fn cutoff() -> ParamAddr {
    "filter1.cutoff".parse().unwrap()
}

fn rms(x: &[f32]) -> f64 {
    (x.iter().map(|v| f64::from(*v) * f64::from(*v)).sum::<f64>() / x.len().max(1) as f64).sqrt()
}

#[test]
fn a_lesson_through_the_real_plugin_with_the_worker_saved_and_resumed_mid_lesson() {
    let dir = TempDir::new("learn");
    let lessons = Arc::new(Lessons::new(None, Some(dir.0.join(PROGRESS_FILE))));
    let bundle = lessons.catalogue().remove(0);
    let witness = bundle
        .patch(&enth_learn::pack::PatchRef::new("patches/dark.patch.json").unwrap())
        .unwrap()
        .value(&cutoff())
        .unwrap();
    let mut host = Host::new(&lessons);
    let own = host.frame(|s| s.patch().clone());

    // Start: step 0 (Listen); its examples rendered on the worker.
    let player = Player::new(bundle.clone());
    host.frame(|s| {
        s.game_tick(1_000, EditorView::Opened);
        s.learn_start(Box::new(player)).unwrap();
    });
    host.until("the examples", |s| {
        LearnEdit::lesson(s).is_some_and(|l| l.listen.ready)
    });
    let view = host.frame(|s| LearnEdit::lesson(s).unwrap());
    let gain = enth_challenge::loudness::db_to_amp(f64::from(view.listen.gains_db[0])) as f32;

    // Listen to example 0: after the crossfade, exactly the example's render at its gain.
    host.frame(|s| s.learn_listen(Some(0)).unwrap());
    let examples = {
        let mut p = Player::new(bundle.clone());
        p.start()
            .unwrap()
            .into_iter()
            .find_map(|e| match e {
                enth_learn::Effect::Examples(x) => Some(x),
                _ => None,
            })
            .unwrap()
    };
    let rendered = crate::learn::render_examples(&examples, SR).unwrap();
    let fade = (SR as f32 * game::AB_FADE_MS / 1000.0) as usize;
    let n = SR as usize / 4;
    let (l, _) = host.render(n, 256);
    let want: Vec<f32> = rendered.outs[0].left[fade..n]
        .iter()
        .map(|v| v * gain)
        .collect();
    assert_eq!(&l[fade..], &want[..], "example 0, level-matched");
    assert!(rms(&l) > 0.01, "audible: {}", rms(&l));
    host.frame(|s| s.learn_listen(None).unwrap());
    let _ = host.render(SR as usize / 10, 256);

    // Next: the Do step, its start loaded, osc 1 locked.
    host.frame(|s| s.learn_next().unwrap());
    let refused = host.frame(|s| {
        s.edit(
            vec![Op::SetParam {
                addr: "osc1.level".parse().unwrap(),
                value: 50.0,
            }],
            None,
        )
    });
    assert!(refused.is_err(), "osc1 is locked");
    host.frame(|s| {
        s.edit(
            vec![Op::SetParam {
                addr: cutoff(),
                value: witness,
            }],
            None,
        )
        .unwrap();
    });
    // The settle moment, then the sound check rendered on the worker.
    host.until("the step passes", |s| {
        LearnEdit::lesson(s).is_some_and(|l| l.phase == StepPhase::Passed)
    });
    let progress = lessons.progress().load();
    let step = progress
        .step(&"example/cutoff".parse().unwrap(), &"try".parse().unwrap())
        .expect("merged into the progress file at once");
    assert!(step.done && step.clean);

    // Save the project mid-lesson, then a fresh instance loads it: the lesson reopens on the worker at the saved step,
    // the step still passed, its locks back, the patch the saved one.
    let fields = host.plugin.host_params().serialize_fields();
    let saved_patch = host.frame(|s| s.patch().clone());
    drop(host);
    let mut again = Host::new(&lessons);
    again.plugin.host_params().deserialize_fields(&fields);
    again.until("the lesson reopens", |s| {
        LearnEdit::lesson(s).is_some_and(|l| !l.pending.opening && l.steps > 0)
    });
    again.frame(|s| {
        let view = LearnEdit::lesson(s).unwrap();
        assert_eq!(view.lesson.as_ref().unwrap().step, 1);
        assert_eq!(view.phase, StepPhase::Passed, "the progress file read back");
        assert!(!view.broken);
        assert!(!s.locks().is_empty(), "the step's locks are back");
        assert_eq!(s.patch().value(&cutoff()), saved_patch.value(&cutoff()));
    });

    // Finish, then End with Restore: the player's own patch from before the lesson.
    again.frame(|s| s.learn_next().unwrap());
    again.frame(|s| s.learn_choose(1).unwrap());
    again.frame(|s| s.learn_next().unwrap());
    again.frame(|s| {
        assert!(LearnEdit::lesson(s).unwrap().finished);
        s.learn_end(EndChoice::Restore).unwrap();
        assert!(LearnEdit::lesson(s).is_none());
        assert_eq!(
            s.patch().generators,
            own.generators,
            "Restore brings the patch back"
        );
    });
    let progress = lessons.progress().load();
    let lesson = progress.lesson(&"example/cutoff".parse().unwrap()).unwrap();
    assert!(lesson.completed(&bundle.lesson), "{lesson:?}");
    assert!(
        again.requests.dirty.load(Ordering::SeqCst) > 0,
        "a lesson marks the project dirty"
    );
}
