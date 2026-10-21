//! **Owner: lane SK3** (the Create skeleton, freeze point F2s; feature `create`). Pre-declared by lane S0s (R3).
//!
//! The plugin's side of "create from sound" (docs/r3.md, "The cancellable pool" and "File input"; docs/plugin.md,
//! "Create"). Shells translate and hold no product rules: the intake is `enth-analysis`'s, the portfolio and the K
//! searches are `enth-solver`'s `create`, the state machine is `enth-session`'s `create`, and this file is the one
//! place that decodes files, owns threads and touches the disk for it:
//!
//! - **An `enth-create` orchestrator thread** ([`Create`], in [`crate::Shared`]), spawned at the first job and joined
//!   when `Shared` drops, like `enth-game` but never the same thread: the game worker is joined on drop ("a few
//!   hundred ms at most"), so a 60 s job there would hang an unload. It decodes the file ([`decode`]), runs the
//!   intake, builds the portfolio, then drives a [`CreateJob`] round by round. It never runs on the audio thread and
//!   never takes the session lock: its posts go to a queue the GUI thread hands to `Session::create_post` (each post
//!   asks the open editor for a repaint, as the game worker's do).
//! - **A per-job `rayon` pool** (never the global one), its threads named `enth-search-N`, built with
//!   `ThreadPoolBuilder::build_scoped`, so its threads are **joined** before the job returns and no thread exists
//!   between jobs (a plain `ThreadPool`'s drop does not wait for its threads, and a thread left running in an unloaded
//!   plugin crashes the host); [`search_workers`] = `min(available_parallelism / 2, 8)`, at normal priority, never
//!   real-time. Each worker keeps its own metric worker (FFT plans, buffers); a batch's distances come back by index,
//!   so the result is the same for any worker count.
//! - **Cancel within one batch:** a job's `AtomicBool` is checked before every render in the pool and after each round.
//!   A cancelled batch is dropped and never told (`CreateJob::cancel`), so a cancelled run is a prefix of the full one.
//!   The latency is one render per worker (tens of ms with effects on a 3 s clip).
//! - **The wall-clock cap** (the job's `seconds`, at most 300) is checked the same way: past it the batch is dropped
//!   and the job stops as `cancelled`, with a note saying it was the cap. The solver reads no clock.
//! - **Unload never hangs:** [`Create`]'s `Drop` cancels the running job, wakes the thread and joins it; the decoder
//!   polls the same flag between chunks, and the pool stops within one render.
//! - **The k-NN bank** (`enth-bank-v1.bin`) is looked for beside the plugin's binary, then in
//!   `$ENTH_MODELS_DIR` or `<Documents>/enth/Models/` ([`bank_paths`]); a missing or refused bank means the job runs on
//!   the heuristic and the templates, and its notes say so.
//! - **Audition:** a candidate rendered with the job's performance at the host's rate, handed to the audio thread
//!   through a lock-free slot ([`AuditionSlot`]) and played once by [`AuditionPlayer`] in `process()` (one atomic
//!   load per block while nothing plays; it never allocates or frees there).
//!
//! The gate under load is `enth-test-host process-stats --gate g1` with a create job running (the `create-harness`
//! feature, [`harness`]), as lane SKR gated the A/B. Off by default: the plugin's `create` feature gates this module;
//! the session's `create` always compiles.

pub mod decode;

use std::collections::VecDeque;
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::ptr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU32, AtomicUsize, Ordering};
use std::thread::JoinHandle;
use std::time::Instant;

use enth_analysis::intake::{Intake, IntakeCfg};
use enth_patch::{Patch, file};
use enth_render::{RenderSpec, render};
use enth_session::create::{
    CreateIntake, CreateJobId, CreatePost, CreateProgress, CreateRequest, CreateRun,
};
use enth_solver::create::{
    Candidate, CreateCfg, CreateJob, CreateStop, Inputs, Portfolio, Target, Template,
};
use enth_solver::nearest::Bank;
use enth_solver::search::{MetricTarget, MetricWorker};
use parking_lot::{Condvar, Mutex};
use rayon::prelude::*;

use crate::presets::{Environment, Os, SystemEnv, documents_dir};

/// The k-NN bank's file name (`cargo xtask bank` writes it; docs/r3.md, "The bank").
pub const BANK_FILE: &str = "enth-bank-v1.bin";
/// The objective's metric id (metric v1, the solver's `MetricWorker`), recorded with every job.
pub const OBJECTIVE: &str = "logmel-l1/1";

/// The templates every job proposes (the heuristic start's base first), embedded from `content/templates/` as the
/// CLI's are.
const TEMPLATES: [(&str, &str); 4] = [
    (
        "init",
        include_str!("../../../content/templates/init.patch.json"),
    ),
    (
        "pluck_1osc",
        include_str!("../../../content/templates/pluck_1osc.patch.json"),
    ),
    (
        "pad_2osc",
        include_str!("../../../content/templates/pad_2osc.patch.json"),
    ),
    (
        "bass_sub",
        include_str!("../../../content/templates/bass_sub.patch.json"),
    ),
];

/// The init template (the heuristic start's base) and the other templates.
pub fn templates() -> (Patch, Vec<Template>) {
    let mut all = TEMPLATES.iter().filter_map(|(name, text)| {
        file::load(text).ok().map(|loaded| Template {
            name: (*name).to_owned(),
            patch: loaded.patch,
        })
    });
    let init = all.next().map_or_else(Patch::init, |t| t.patch);
    (init, all.collect())
}

/// The workers of a job's pool: half the logical CPUs (about the physical cores, so the host's audio, GUI and main
/// threads keep the SMT siblings), at least 1 and at most 8.
pub fn search_workers() -> usize {
    let cpus = std::thread::available_parallelism().map_or(2, std::num::NonZero::get);
    (cpus / 2).clamp(1, 8)
}

/// `<Documents>/enth/Models/`, or `$ENTH_MODELS_DIR` when set: where the bank (and R3b's model file) live when they
/// are not beside the plugin.
pub fn models_dir(env: &dyn Environment, os: Os) -> Option<PathBuf> {
    env.var("ENTH_MODELS_DIR")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| documents_dir(env, os).map(|d| d.join("enth").join("Models")))
}

/// Where the bank is looked for, in order: beside the plugin's binary (the CLAP file, or the `.so`/`.dll`/Mach-O
/// inside a VST3 bundle), then [`models_dir`].
pub fn bank_paths(env: &dyn Environment, os: Os) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(dir) = plugin_binary().as_deref().and_then(Path::parent) {
        paths.push(dir.join(BANK_FILE));
    }
    if let Some(dir) = models_dir(env, os) {
        paths.push(dir.join(BANK_FILE));
    }
    paths
}

/// The bank at the first of `paths` that holds a file, with notes on what was found or refused.
pub fn load_bank(paths: &[PathBuf]) -> (Option<(Bank, PathBuf)>, Vec<String>) {
    let mut notes = Vec::new();
    for path in paths {
        let Ok(bytes) = std::fs::read(path) else {
            continue;
        };
        match Bank::parse(&bytes) {
            Ok(bank) => return (Some((bank, path.clone())), notes),
            Err(e) => notes.push(format!(
                "the k-NN bank {} was refused ({e}): running on the heuristic and the templates",
                path.display()
            )),
        }
    }
    if notes.is_empty() {
        let looked: Vec<String> = paths.iter().map(|p| p.display().to_string()).collect();
        notes.push(format!(
            "no k-NN bank ({BANK_FILE}) found (looked in {}): running on the heuristic and the templates",
            if looked.is_empty() {
                "no folder".to_owned()
            } else {
                looked.join(", ")
            }
        ));
    }
    (None, notes)
}

/// The path of the shared library this code is in (the plugin's binary), through `dladdr`.
#[cfg(unix)]
#[allow(
    unsafe_code,
    reason = "dladdr: the only way to learn the loaded plugin's own path on Linux and macOS"
)]
fn plugin_binary() -> Option<PathBuf> {
    use std::ffi::{CStr, OsStr, c_char, c_int, c_void};
    use std::os::unix::ffi::OsStrExt;

    #[repr(C)]
    #[allow(
        clippy::struct_field_names,
        reason = "the field names of the C struct `Dl_info`"
    )]
    struct DlInfo {
        dli_fname: *const c_char,
        dli_fbase: *mut c_void,
        dli_sname: *const c_char,
        dli_saddr: *mut c_void,
    }
    unsafe extern "C" {
        fn dladdr(addr: *const c_void, info: *mut DlInfo) -> c_int;
    }
    let mut info = DlInfo {
        dli_fname: ptr::null(),
        dli_fbase: ptr::null_mut(),
        dli_sname: ptr::null(),
        dli_saddr: ptr::null_mut(),
    };
    let here = plugin_binary as fn() -> Option<PathBuf> as *const c_void;
    // SAFETY: `here` is an address inside this library; `info` is a valid, writable `Dl_info` (the same layout on
    // glibc, musl and macOS).
    let found = unsafe { dladdr(here, &raw mut info) };
    if found == 0 || info.dli_fname.is_null() {
        return None;
    }
    // SAFETY: dladdr filled `dli_fname` with a NUL-terminated string that lives as long as the library is loaded.
    let name = unsafe { CStr::from_ptr(info.dli_fname) };
    let path = PathBuf::from(OsStr::from_bytes(name.to_bytes()));
    path.is_absolute().then_some(path)
}

/// The path of the DLL this code is in (the plugin's binary), through `GetModuleHandleExW` and `GetModuleFileNameW`.
#[cfg(windows)]
#[allow(
    unsafe_code,
    reason = "kernel32: the only way to learn the loaded plugin's own path on Windows"
)]
fn plugin_binary() -> Option<PathBuf> {
    use std::ffi::{OsString, c_void};
    use std::os::windows::ffi::OsStringExt;

    const FROM_ADDRESS: u32 = 0x4;
    const UNCHANGED_REFCOUNT: u32 = 0x2;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetModuleHandleExW(flags: u32, name: *const u16, module: *mut *mut c_void) -> i32;
        fn GetModuleFileNameW(module: *mut c_void, name: *mut u16, size: u32) -> u32;
    }
    let mut module: *mut c_void = ptr::null_mut();
    let here = plugin_binary as fn() -> Option<PathBuf> as *const u16;
    // SAFETY: with FROM_ADDRESS the name is an address inside this module; `module` is writable.
    let ok = unsafe { GetModuleHandleExW(FROM_ADDRESS | UNCHANGED_REFCOUNT, here, &raw mut module) };
    if ok == 0 {
        return None;
    }
    let mut buf = vec![0u16; 32_768];
    // SAFETY: `buf` holds `buf.len()` u16s; the call writes at most that many.
    let n = unsafe { GetModuleFileNameW(module, buf.as_mut_ptr(), buf.len() as u32) } as usize;
    (n > 0 && n < buf.len()).then(|| PathBuf::from(OsString::from_wide(&buf[..n])))
}

#[cfg(not(any(unix, windows)))]
fn plugin_binary() -> Option<PathBuf> {
    None
}

// ---------------------------------------------------------------------------------------------------------------
// Audition: a one-shot clip handed to the audio thread
// ---------------------------------------------------------------------------------------------------------------

/// A rendered candidate for the audio thread to play once. An empty clip stops what plays.
#[derive(Debug, Default)]
pub struct AuditionClip {
    left: Box<[f32]>,
    right: Box<[f32]>,
}

impl AuditionClip {
    /// A clip from a render's two channels (the shorter length).
    pub fn new(mut left: Vec<f32>, mut right: Vec<f32>) -> Self {
        let n = left.len().min(right.len());
        left.truncate(n);
        right.truncate(n);
        AuditionClip {
            left: left.into_boxed_slice(),
            right: right.into_boxed_slice(),
        }
    }

    /// Frames in the clip.
    pub fn frames(&self) -> usize {
        self.left.len()
    }
}

/// The lock-free hand-off of [`AuditionClip`]s: an incoming slot (to the audio thread) and a retired one (back), each
/// an `AtomicPtr` to a boxed clip or null, as `game::Mailbox` hands over a round's loop. The audio thread only swaps
/// pointers; every allocation and free happens on the other side.
#[derive(Debug, Default)]
pub struct AuditionSlot {
    incoming: AtomicPtr<AuditionClip>,
    retired: AtomicPtr<AuditionClip>,
}

/// The box behind a pointer taken out of a slot (`None` for null).
///
/// # Safety
/// `p` is null or came from `Box::into_raw` and has no other owner.
#[allow(unsafe_code, reason = "the lock-free hand-off owns raw boxes")]
unsafe fn take_box(p: *mut AuditionClip) -> Option<Box<AuditionClip>> {
    // SAFETY: the caller's contract.
    (!p.is_null()).then(|| unsafe { Box::from_raw(p) })
}

#[allow(unsafe_code, reason = "the lock-free hand-off owns raw boxes")]
impl AuditionSlot {
    /// Not the audio thread: free the clip the audio thread handed back, if any.
    pub fn collect(&self) {
        let p = self.retired.swap(ptr::null_mut(), Ordering::AcqRel);
        // SAFETY: a non-null pointer in a slot came from `Box::into_raw` and the swap made this its only owner.
        drop(unsafe { take_box(p) });
    }

    /// Not the audio thread: hand `clip` over (one the audio thread has not taken yet is freed here).
    pub fn publish(&self, clip: Box<AuditionClip>) {
        self.collect();
        let old = self.incoming.swap(Box::into_raw(clip), Ordering::AcqRel);
        // SAFETY: as in `collect`.
        drop(unsafe { take_box(old) });
    }

    /// The audio thread: the newest clip, if one waits. A load first, so an idle block writes nothing.
    fn take(&self) -> Option<Box<AuditionClip>> {
        if self.incoming.load(Ordering::Acquire).is_null() {
            return None;
        }
        let p = self.incoming.swap(ptr::null_mut(), Ordering::AcqRel);
        // SAFETY: as in `collect`.
        unsafe { take_box(p) }
    }

    /// The audio thread: hand `old` back, or get it back while the retired slot is still full. Never frees.
    fn retire(&self, old: Box<AuditionClip>) -> Result<(), Box<AuditionClip>> {
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

#[allow(unsafe_code, reason = "the lock-free hand-off owns raw boxes")]
impl Drop for AuditionSlot {
    fn drop(&mut self) {
        for slot in [&self.incoming, &self.retired] {
            let p = slot.swap(ptr::null_mut(), Ordering::AcqRel);
            // SAFETY: as in `collect`; nothing else reaches the slot while it drops.
            drop(unsafe { take_box(p) });
        }
    }
}

/// The audio thread's side of an audition: the clip that plays, added to the block once, from its start. While
/// nothing plays a block costs one atomic load. It never allocates or frees: a clip it is done with goes back
/// through the slot (kept in a pending place while the retired slot is still full).
#[derive(Debug)]
pub struct AuditionPlayer {
    slot: Arc<AuditionSlot>,
    clip: Option<Box<AuditionClip>>,
    pos: usize,
    pending: Option<Box<AuditionClip>>,
}

impl AuditionPlayer {
    /// A player on `slot`.
    pub fn new(slot: Arc<AuditionSlot>) -> Self {
        AuditionPlayer {
            slot,
            clip: None,
            pos: 0,
            pending: None,
        }
    }

    /// Hand `old` back, or keep it pending while the retired slot is full (the caller made sure nothing else is).
    fn give_back(&mut self, old: Box<AuditionClip>) {
        debug_assert!(self.pending.is_none());
        if let Err(old) = self.slot.retire(old) {
            self.pending = Some(old);
        }
    }

    /// Add the playing clip into `left` and `right` (one block), from where it is.
    pub fn mix(&mut self, left: &mut [f32], right: &mut [f32]) {
        if let Some(p) = self.pending.take()
            && let Err(p) = self.slot.retire(p)
        {
            self.pending = Some(p);
        }
        // A new clip is taken only when the one it replaces has somewhere to go: else it waits in the slot.
        if (self.pending.is_none() || self.clip.is_none())
            && let Some(next) = self.slot.take()
        {
            if let Some(old) = self.clip.replace(next) {
                self.give_back(old);
            }
            self.pos = 0;
        }
        let Some(clip) = self.clip.as_ref() else {
            return;
        };
        let n = left.len().min(right.len());
        let k = n.min(clip.frames().saturating_sub(self.pos));
        if k > 0 {
            let (l, r) = (
                &clip.left[self.pos..self.pos + k],
                &clip.right[self.pos..self.pos + k],
            );
            for (o, s) in left[..k].iter_mut().zip(l) {
                *o += *s;
            }
            for (o, s) in right[..k].iter_mut().zip(r) {
                *o += *s;
            }
            self.pos += k;
        }
        // Done: hand it back, or keep it (silent, at its end) until the retired slot has room.
        if self.pos >= clip.frames()
            && self.pending.is_none()
            && let Some(done) = self.clip.take()
        {
            self.give_back(done);
        }
    }

    /// Whether a clip plays.
    pub fn playing(&self) -> bool {
        self.clip.is_some()
    }
}

// ---------------------------------------------------------------------------------------------------------------
// The orchestrator
// ---------------------------------------------------------------------------------------------------------------

#[derive(Debug)]
enum Msg {
    Run(CreateRun),
    Audition {
        id: CreateJobId,
        patch: Box<Patch>,
    },
    StopAudition,
}

/// The running job: its id and its cancel flag.
#[derive(Debug)]
struct Running {
    id: CreateJobId,
    cancel: Arc<AtomicBool>,
}

struct Inner {
    queue: Mutex<VecDeque<Msg>>,
    wake: Condvar,
    quit: AtomicBool,
    running: Mutex<Option<Running>>,
    posts: Mutex<Vec<(CreateJobId, CreatePost)>>,
    repaint: Mutex<Option<egui::Context>>,
    host_rate: AtomicU32,
    /// Where the bank is looked for (`None`: [`bank_paths`] of the real environment).
    bank_paths: Mutex<Option<Vec<PathBuf>>>,
    /// The pool's worker count (0: [`search_workers`]).
    workers: AtomicUsize,
    audition: Arc<AuditionSlot>,
    /// The last job's target (its performance and rate), for auditions after it ends.
    target: Mutex<Option<(CreateJobId, Target)>>,
    /// Jobs started and finished (tests and the harness).
    started: AtomicUsize,
    finished: AtomicUsize,
    /// Rounds posted, and the renders of every job that ended (the harness's report).
    rounds: AtomicUsize,
    renders: AtomicUsize,
    /// Keep posts for the session (false in the harness, where no editor takes them: they are only counted).
    keep_posts: AtomicBool,
    /// Run this job again, under a new id, each time one ends (the harness).
    repeat: Mutex<Option<CreateRun>>,
}

impl Inner {
    fn post(&self, id: CreateJobId, post: CreatePost) {
        match &post {
            CreatePost::Round { .. } => {
                self.rounds.fetch_add(1, Ordering::AcqRel);
            }
            CreatePost::Done { progress, .. } => {
                self.renders.fetch_add(progress.renders, Ordering::AcqRel);
            }
            _ => {}
        }
        if !self.keep_posts.load(Ordering::Acquire) {
            return;
        }
        self.posts.lock().push((id, post));
        if let Some(ctx) = self.repaint.lock().as_ref() {
            ctx.request_repaint();
        }
    }

    /// Serve the auditions queued meanwhile (between rounds of a job, or when idle), leaving everything else queued.
    fn serve_auditions(&self) {
        let msgs: Vec<Msg> = {
            let mut queue = self.queue.lock();
            let (take, keep): (VecDeque<Msg>, VecDeque<Msg>) = queue
                .drain(..)
                .partition(|m| matches!(m, Msg::Audition { .. } | Msg::StopAudition));
            *queue = keep;
            take.into_iter().collect()
        };
        for msg in msgs {
            self.audition(msg);
        }
    }

    fn audition(&self, msg: Msg) {
        match msg {
            Msg::Audition { id, patch } => {
                let rate = self.host_rate.load(Ordering::Acquire);
                let target = self.target.lock().as_ref().filter(|(t, _)| *t == id).map(|(_, t)| t.clone());
                let (Some(target), true) = (target, (8_000..=384_000).contains(&rate)) else {
                    return;
                };
                let rendered = panic::catch_unwind(AssertUnwindSafe(|| {
                    render(
                        &patch,
                        &target.performance(),
                        &RenderSpec::new(rate, target.render_seed),
                    )
                }));
                if let Ok(Ok(out)) = rendered {
                    self.audition
                        .publish(Box::new(AuditionClip::new(out.left, out.right)));
                }
            }
            Msg::StopAudition => self.audition.publish(Box::default()),
            Msg::Run(_) => {}
        }
    }
}

/// The shell's create side, in [`crate::Shared`] (see the [module docs](self)).
pub struct Create {
    inner: Arc<Inner>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl std::fmt::Debug for Create {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Create")
            .field("thread", &self.thread.lock().is_some())
            .finish_non_exhaustive()
    }
}

impl Default for Create {
    fn default() -> Self {
        Create::new()
    }
}

impl Create {
    /// No thread yet, no job.
    pub fn new() -> Create {
        Create {
            inner: Arc::new(Inner {
                queue: Mutex::new(VecDeque::new()),
                wake: Condvar::new(),
                quit: AtomicBool::new(false),
                running: Mutex::new(None),
                posts: Mutex::new(Vec::new()),
                repaint: Mutex::new(None),
                host_rate: AtomicU32::new(0),
                bank_paths: Mutex::new(None),
                workers: AtomicUsize::new(0),
                audition: Arc::new(AuditionSlot::default()),
                target: Mutex::new(None),
                started: AtomicUsize::new(0),
                finished: AtomicUsize::new(0),
                rounds: AtomicUsize::new(0),
                renders: AtomicUsize::new(0),
                keep_posts: AtomicBool::new(true),
                repeat: Mutex::new(None),
            }),
            thread: Mutex::new(None),
        }
    }

    /// The audition hand-off, for the audio thread's [`AuditionPlayer`].
    pub fn audition_slot(&self) -> Arc<AuditionSlot> {
        Arc::clone(&self.inner.audition)
    }

    /// The host's rate (at every activation): auditions render at it.
    pub fn set_host_rate(&self, sample_rate: f32) {
        self.inner
            .host_rate
            .store(sample_rate.round() as u32, Ordering::Release);
    }

    /// The open editor's context (at `build()`), or none (at close): every post asks it for a repaint.
    pub fn set_repaint(&self, ctx: Option<egui::Context>) {
        *self.inner.repaint.lock() = ctx;
    }

    /// Look for the bank in `paths` instead of the environment's places (tests; `None`: the environment's).
    pub fn set_bank_paths(&self, paths: Option<Vec<PathBuf>>) {
        *self.inner.bank_paths.lock() = paths;
    }

    /// The pool's worker count (tests; 0: [`search_workers`]).
    pub fn set_workers(&self, n: usize) {
        self.inner.workers.store(n, Ordering::Release);
    }

    /// After unlocking: act on the create requests a drain took, in order. Runs are queued for the thread, a cancel
    /// drops a queued run (posting its end at once) or raises the running job's flag, auditions go to the thread.
    pub fn act(&self, requests: Vec<CreateRequest>) {
        for request in requests {
            match request {
                CreateRequest::Run(run) => self.send(Msg::Run(run)),
                CreateRequest::Cancel(id) => self.cancel(id),
                CreateRequest::Audition { id, patch, .. } => self.send(Msg::Audition { id, patch }),
                CreateRequest::StopAudition => self.send(Msg::StopAudition),
                _ => {}
            }
        }
    }

    /// The posts the thread made, oldest first, for `Session::create_post` (the GUI thread, at the top of a frame).
    /// Also frees clips the audio thread handed back.
    pub fn take_posts(&self) -> Vec<(CreateJobId, CreatePost)> {
        self.inner.audition.collect();
        std::mem::take(&mut *self.inner.posts.lock())
    }

    /// Whether nothing is queued or running.
    pub fn idle(&self) -> bool {
        self.inner.queue.lock().is_empty() && self.inner.running.lock().is_none()
    }

    /// Jobs the thread started and finished so far.
    pub fn jobs(&self) -> (usize, usize) {
        (
            self.inner.started.load(Ordering::Acquire),
            self.inner.finished.load(Ordering::Acquire),
        )
    }

    /// Whether the thread exists (spawned at the first request).
    pub fn thread_started(&self) -> bool {
        self.thread.lock().is_some()
    }

    fn send(&self, msg: Msg) {
        self.inner.queue.lock().push_back(msg);
        self.inner.wake.notify_one();
        let mut thread = self.thread.lock();
        if thread.is_none() {
            let inner = Arc::clone(&self.inner);
            // A thread that cannot start leaves the job queued: the view keeps showing it as starting rather than
            // the host crashing.
            *thread = std::thread::Builder::new()
                .name("enth-create".to_owned())
                .spawn(move || work(&inner))
                .ok();
        }
    }

    fn cancel(&self, id: CreateJobId) {
        let queued = {
            let mut queue = self.inner.queue.lock();
            let before = queue.len();
            queue.retain(|m| !matches!(m, Msg::Run(run) if run.id == id));
            before != queue.len()
        };
        if queued {
            self.inner.post(
                id,
                CreatePost::Done {
                    progress: CreateProgress::new(0, 0, 0, 0.0, 0.0),
                    stop: CreateStop::Cancelled,
                    candidates: Vec::new(),
                    notes: vec!["cancelled before it started".to_owned()],
                },
            );
        }
        if let Some(running) = self.inner.running.lock().as_ref()
            && running.id == id
        {
            running.cancel.store(true, Ordering::Release);
        }
    }
}

impl Drop for Create {
    /// Cancel the running job (the pool stops within one render, the decoder between two chunks), wake the thread
    /// and join it: an unload never waits for a search.
    fn drop(&mut self) {
        self.inner.quit.store(true, Ordering::Release);
        if let Some(running) = self.inner.running.lock().as_ref() {
            running.cancel.store(true, Ordering::Release);
        }
        {
            let _queue = self.inner.queue.lock();
            self.inner.wake.notify_all();
        }
        if let Some(thread) = self.thread.get_mut().take() {
            let _ = thread.join();
        }
        self.inner.audition.collect();
    }
}

/// The thread: one message at a time until [`Create`] drops. A panicking job is posted as failed (the crash hook
/// writes its file) and the thread goes on.
fn work(inner: &Inner) {
    loop {
        let msg = {
            let mut queue = inner.queue.lock();
            loop {
                if inner.quit.load(Ordering::Acquire) {
                    return;
                }
                if let Some(msg) = queue.pop_front() {
                    break msg;
                }
                inner.wake.wait(&mut queue);
            }
        };
        match msg {
            Msg::Run(run) => {
                let id = run.id;
                let cancel = Arc::new(AtomicBool::new(false));
                *inner.running.lock() = Some(Running {
                    id,
                    cancel: Arc::clone(&cancel),
                });
                inner.started.fetch_add(1, Ordering::AcqRel);
                let ran = panic::catch_unwind(AssertUnwindSafe(|| run_job(inner, &run, &cancel)));
                if ran.is_err() {
                    inner.post(
                        id,
                        CreatePost::Failed {
                            code: "create_panicked".to_owned(),
                            message: "the create job panicked (see the crash folder)".to_owned(),
                            hint: "try another file; the crash file says what happened".to_owned(),
                        },
                    );
                }
                *inner.running.lock() = None;
                inner.finished.fetch_add(1, Ordering::AcqRel);
                if let Some(again) = inner.repeat.lock().as_ref()
                    && !inner.quit.load(Ordering::Acquire)
                {
                    let next = CreateRun {
                        id: CreateJobId(id.0 + 1),
                        ..again.clone()
                    };
                    inner.queue.lock().push_back(Msg::Run(next));
                }
            }
            other => inner.audition(other),
        }
    }
}

fn failed(code: &str, message: impl Into<String>, hint: impl Into<String>) -> CreatePost {
    CreatePost::Failed {
        code: code.to_owned(),
        message: message.into(),
        hint: hint.into(),
    }
}

/// One job, from the file to its last post.
fn run_job(inner: &Inner, run: &CreateRun, cancel: &AtomicBool) {
    let t0 = Instant::now();
    let cap = run.settings.seconds.clamp(0.001, enth_session::create::MAX_SECONDS);
    let id = run.id;
    let stopped = || cancel.load(Ordering::Acquire) || inner.quit.load(Ordering::Acquire);
    let late = || t0.elapsed().as_secs_f64() >= cap;
    let ended = |notes: Vec<String>| CreatePost::Done {
        progress: CreateProgress::new(0, 0, 0, t0.elapsed().as_secs_f64(), cap),
        stop: CreateStop::Cancelled,
        candidates: Vec::new(),
        notes,
    };

    // 1. Decode (I/O), polling the cancel flag between chunks.
    let decoded = match decode::decode_path(&run.source.path, &|| stopped() || late()) {
        Ok(d) => d,
        Err(e) if e.code == "cancelled" => {
            inner.post(id, ended(vec![stop_note(stopped())]));
            return;
        }
        Err(e) => {
            inner.post(id, failed(e.code, e.message, e.hint));
            return;
        }
    };
    // 2. The intake, with the decoder's codec hint.
    let cfg = IntakeCfg {
        codec: Some(decoded.codec),
        ..IntakeCfg::default()
    };
    let intake = match Intake::with_cfg(&decoded.interleaved, decoded.channels, decoded.sample_rate, &cfg) {
        Ok(i) => i,
        Err(e) => {
            inner.post(
                id,
                failed(
                    "intake",
                    e.to_string(),
                    "check the file holds a sound (it is silent, or not finite, or at a rate the analysis refuses)",
                ),
            );
            return;
        }
    };
    let (described, mut notes) = (decoded.describe(), decoded.notes());
    drop(decoded);
    // 3. The target, the starts and the bank.
    let target = Target::from_intake(&intake, enth_session::RENDER_SEED);
    *inner.target.lock() = Some((id, target.clone()));
    let paths = inner
        .bank_paths
        .lock()
        .clone()
        .unwrap_or_else(|| bank_paths(&SystemEnv, Os::current()));
    let (bank, bank_notes) = load_bank(&paths);
    notes.extend(bank_notes);
    if let Some((_, path)) = &bank {
        notes.push(format!("k-NN bank: {}", path.display()));
    }
    inner.post(
        id,
        CreatePost::Intake(Box::new(CreateIntake::new(
            described,
            intake.report.clone(),
            notes,
        ))),
    );
    let analysis = match target.analyse() {
        Ok(a) => a,
        Err(e) => {
            inner.post(id, failed(e.code(), e.to_string(), e.hint()));
            return;
        }
    };
    let (init, templates) = templates();
    let inputs = Inputs {
        init: &init,
        templates: &templates,
        bank: bank.as_ref().map(|(b, _)| b),
        proposals: &[],
    };
    let mut ccfg = CreateCfg {
        k: run.settings.candidates,
        seed: run.settings.seed,
        ..CreateCfg::default()
    };
    if let Some(budget) = run.settings.budget {
        ccfg.budget = budget;
    }
    let portfolio = Portfolio::build(&target, &analysis, &inputs, &ccfg);
    let mut job = match CreateJob::new(&target, &analysis, portfolio, &ccfg) {
        Ok(j) => j,
        Err(e) => {
            inner.post(id, failed(e.code(), e.to_string(), e.hint()));
            return;
        }
    };
    // 4. The search, on a pool of its own whose threads are joined before this returns.
    let metric = target.metric_target();
    let workers = match inner.workers.load(Ordering::Acquire) {
        0 => search_workers(),
        n => n,
    };
    let progress = |job: &CreateJob| {
        let p = job.progress();
        CreateProgress::new(p.rounds, p.renders, p.budget, t0.elapsed().as_secs_f64(), cap)
    };
    let searched = rayon::ThreadPoolBuilder::new()
        .num_threads(workers)
        .thread_name(|i| format!("enth-search-{i}"))
        .build_scoped(rayon::ThreadBuilder::run, |pool| {
            search(inner, id, &mut job, &metric, pool, &|| stopped() || late(), &progress)
        });
    let mut end_notes = Vec::new();
    match searched {
        Ok(Ok(())) => {}
        Ok(Err(e)) => {
            inner.post(id, failed(e.code(), e.to_string(), e.hint()));
            return;
        }
        Err(e) => {
            inner.post(
                id,
                failed(
                    "pool_failed",
                    format!("cannot start {workers} search threads: {e}"),
                    "close other programs and try again",
                ),
            );
            return;
        }
    }
    if job.result().stop == CreateStop::Cancelled {
        end_notes.push(stop_note(stopped()));
    }
    let result = job.result();
    inner.post(
        id,
        CreatePost::Done {
            progress: progress(&job),
            stop: result.stop,
            candidates: result.candidates,
            notes: end_notes.into_iter().chain(result.notes).collect(),
        },
    );
}

fn stop_note(cancelled: bool) -> String {
    if cancelled {
        "cancelled".to_owned()
    } else {
        "stopped at the time cap".to_owned()
    }
}

/// The rounds of `job` on `pool`: each batch's distances by index (a render skipped once `stop` says so), a dropped
/// batch on a stop, a post after every round. Serves auditions between rounds.
fn search(
    inner: &Inner,
    id: CreateJobId,
    job: &mut CreateJob,
    metric: &MetricTarget,
    pool: &rayon::ThreadPool,
    stop: &(dyn Fn() -> bool + Sync),
    progress: &dyn Fn(&CreateJob) -> CreateProgress,
) -> Result<(), enth_solver::create::CreateError> {
    let workers: Vec<Mutex<MetricWorker<'_>>> = (0..pool.current_num_threads().max(1))
        .map(|_| Mutex::new(metric.worker()))
        .collect();
    loop {
        if stop() {
            job.cancel();
            return Ok(());
        }
        let batch = job.ask();
        if batch.is_empty() {
            return Ok(());
        }
        let distances: Vec<Option<f64>> = pool.install(|| {
            batch
                .par_iter()
                .map(|patch| {
                    if stop() {
                        return None;
                    }
                    let i = rayon::current_thread_index().unwrap_or(0) % workers.len();
                    Some(workers[i].lock().distance(patch))
                })
                .collect()
        });
        if distances.iter().any(Option::is_none) {
            // Stopped inside the batch: drop it, never tell it (a cancelled run stays a prefix of the full one).
            job.cancel();
            return Ok(());
        }
        let distances: Vec<f64> = distances.into_iter().flatten().collect();
        job.tell(&distances)?;
        let candidates: Vec<Candidate> = job.candidates();
        inner.post(
            id,
            CreatePost::Round {
                progress: progress(job),
                candidates,
            },
        );
        inner.serve_auditions();
    }
}

/// The `process-stats` harness (feature `create-harness`, off by default; never in a shipped build): with
/// `ENTH_CREATE_HARNESS` naming a sound file, activation starts create jobs on it one after the other (each capped
/// at `ENTH_CREATE_SECONDS`, 60 by default) until deactivation, which stops them and appends one
/// `enth.plugin-create-stats/1` line to `ENTH_CREATE_REPORT`: jobs started and finished, rounds, renders, workers.
/// The posts are only counted (no editor takes them).
#[cfg(feature = "create-harness")]
pub mod harness {
    use std::io::Write as _;
    use std::sync::atomic::Ordering;

    use enth_session::create::{CreateRequest, CreateRun, CreateSettings, CreateSource};

    use super::{Create, CreateJobId};

    /// At activation: start the jobs when `ENTH_CREATE_HARNESS` names a file (once per instance).
    pub fn start(create: &Create) {
        let Some(file) = std::env::var_os("ENTH_CREATE_HARNESS") else {
            return;
        };
        if create.inner.repeat.lock().is_some() {
            return;
        }
        let seconds = std::env::var("ENTH_CREATE_SECONDS")
            .ok()
            .and_then(|s| s.parse::<f64>().ok())
            .unwrap_or(60.0);
        let run = CreateRun {
            id: CreateJobId(1),
            source: CreateSource::file(file),
            settings: CreateSettings {
                seconds,
                ..CreateSettings::default()
            },
        };
        create.inner.keep_posts.store(false, Ordering::Release);
        *create.inner.repeat.lock() = Some(run.clone());
        create.act(vec![CreateRequest::Run(run)]);
    }

    /// At deactivation: stop the jobs and write the report line.
    pub fn report(create: &Create) {
        if create.inner.repeat.lock().take().is_none() {
            return;
        }
        if let Some(running) = create.inner.running.lock().as_ref() {
            running.cancel.store(true, Ordering::Release);
        }
        let (started, finished) = create.jobs();
        let Some(path) = std::env::var_os("ENTH_CREATE_REPORT") else {
            return;
        };
        let line = format!(
            "{{\"schema\":\"enth.plugin-create-stats/1\",\"jobs_started\":{started},\"jobs_finished\":{finished},\"rounds\":{},\"renders_of_finished_jobs\":{},\"workers\":{}}}\n",
            create.inner.rounds.load(Ordering::Acquire),
            create.inner.renders.load(Ordering::Acquire),
            super::search_workers()
        );
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            let _ = f.write_all(line.as_bytes());
        }
    }
}
