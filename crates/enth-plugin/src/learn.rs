//! **Owner: lane SKL** (the Learn skeleton, freeze point F2l; feature `learn`). Pre-declared by lane S0l (R2).
//!
//! The plugin's Learn I/O (docs/learn.md, "Packs" and "Progress and the saved state"; docs/plugin.md, "Learn"). Shells
//! translate and hold no product rules: the pack format, the player, the checks and the progress merge are
//! `enth-learn`'s, the live binding is `enth-session`'s `learn`, and this file is the one place that touches the disk
//! for lessons:
//!
//! | Part | What |
//! | --- | --- |
//! | [`BUNDLED`], [`FsPacks`] | the packs: the bundled ones (built `.enthpack` files embedded with `include_bytes!`, read through the same reader as any other) and the Packs folder ([`packs_dir`]: `ENTH_PACKS_DIR`, else `<Documents>/enth/Packs`), an `.enthpack` file or a folder with the same layout each. No network code in R2 |
//! | [`ProgressFile`] | `<Documents>/enth/Learn/progress.json` (`ENTH_LEARN_DIR` overrides the folder): a write takes a lock file, re-reads, merges with `Progress::record`, writes a temporary file and renames it, so two instances (in one host or two) never lose each other's progress; a file that is not a progress file is set aside, never overwritten |
//! | [`Lessons`] | the library (`enth_learn::store::load_library` over [`FsPacks`], cached, rescanned when the editor opens), the progress file, and the answers to `Open` jobs: `Player::resume` at the saved step, then `Player::restore` from the progress file, so a step passed before a reload stays passed (phase-3's open check) |
//! | [`run`] | the Learn jobs on the game worker (`game.rs` calls it): `Open`, `Check` (lane PLY's `enth_learn::check::evaluate_sounds`: the step's sound checks rendered and judged), `Hint` (`enth_solver::hint` through fu1-hint's parallel evaluator), `Examples` (each example rendered, measured, and handed to the audio thread as one loop of a set) |
//! | [`act`] | after unlocking: the session's `LearnRequest`s: jobs to the worker, cancels, progress records to the file, the Listen state to `process()`'s atomics, dirty marks |
//!
//! `enth-learn` and the session's `learn` always compile, so a saved lesson restores in any build; without this
//! feature the plugin drains the session's Learn requests and drops them (a saved lesson stays in the project, its
//! stash kept, untouched).

use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use enth_analysis::expect::{Expect, parse_item};
use enth_challenge::model::{SCORING_SAMPLE_RATE, SCORING_SEED};
use enth_challenge::phrases;
use enth_learn::check::evaluate_sounds;
use enth_learn::lesson::ControlCurve;
use enth_learn::pack::{LessonId, PackId};
use enth_learn::player::{Audition, HintRequest, MeasuredHint, StepRecord};
use enth_learn::store::{Library, Origin, PackKey, PackSource, PackStore, load_library};
use enth_learn::{LearnError, LessonBundle, LessonRef, Player, Progress, Selector};
use enth_render::{Control, Controller, Performance, RenderOutput, RenderSpec, render};
use enth_session::learn::{LearnJob, LearnJobKind, LearnRequest, LearnResult};
use enth_solver::Evaluator;
use enth_solver::hint::{HintCfg, hint};
use parking_lot::Mutex;

use crate::presets::{Environment, Os, SystemEnv, documents_dir};

/// The packs built into this binary: `(file name, the built .enthpack)`. The example pack (one lesson, `cutoff`) is
/// the bundled lesson of freeze point F2l until lane C1's `basics` pack replaces it; `packs/` holds the built
/// archives, and `tests::the_bundled_packs_are_the_content_built` rebuilds each from `content/lessons/` and compares
/// the bytes (`ENTH_UPDATE_PACKS=1` rewrites them; the plan's `cargo xtask pack` is lane LCK's).
pub const BUNDLED: &[(&str, &[u8])] = &[(
    "example.enthpack",
    include_bytes!("../packs/example.enthpack"),
)];

/// The progress file's name, in [`learn_dir`].
pub const PROGRESS_FILE: &str = "progress.json";

/// The largest pack file the Packs folder is read for (the reader refuses more anyway; this keeps a stray huge file
/// out of memory).
const MAX_PACK_FILE: u64 = enth_learn::reader::MAX_ARCHIVE_BYTES as u64;

/// The most files a folder pack is read with (the reader's entry limit).
const MAX_FOLDER_FILES: usize = enth_learn::reader::MAX_ENTRIES as usize;

/// A control curve is sent as points this far apart, in seconds (linear between the authored points).
const CURVE_STEP_S: f64 = 0.01;

// ---------------------------------------------------------------------------------------------------------------
// Folders
// ---------------------------------------------------------------------------------------------------------------

/// A variable that is set and not empty, as a path.
fn var_dir(env: &dyn Environment, name: &str) -> Option<PathBuf> {
    env.var(name).filter(|v| !v.is_empty()).map(PathBuf::from)
}

/// The Packs folder: `$ENTH_PACKS_DIR` if set and not empty, else `<Documents>/enth/Packs` (beside Presets and
/// Crashes).
pub fn packs_dir(env: &dyn Environment, os: Os) -> Option<PathBuf> {
    var_dir(env, "ENTH_PACKS_DIR")
        .or_else(|| documents_dir(env, os).map(|d| d.join("enth").join("Packs")))
}

/// The Learn folder, which holds the progress file: `$ENTH_LEARN_DIR` if set and not empty, else
/// `<Documents>/enth/Learn`.
pub fn learn_dir(env: &dyn Environment, os: Os) -> Option<PathBuf> {
    var_dir(env, "ENTH_LEARN_DIR")
        .or_else(|| documents_dir(env, os).map(|d| d.join("enth").join("Learn")))
}

/// Write `bytes` to `target` through a temporary dot file in the same folder and a rename, so a crash mid-write
/// leaves the old file or none. The temporary name is unique within the process (two instances in one host).
fn write_atomic(target: &Path, bytes: &[u8]) -> io::Result<()> {
    static SERIAL: AtomicU64 = AtomicU64::new(0);
    let parent = target.parent().unwrap_or_else(|| Path::new("."));
    let name = target
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("file");
    let temp = parent.join(format!(
        ".{name}.{}.{}.tmp",
        std::process::id(),
        SERIAL.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temp, target)
    })();
    if result.is_err() {
        // The temporary file is ours; do not leave it behind.
        let _ = fs::remove_file(&temp);
    }
    result
}

// ---------------------------------------------------------------------------------------------------------------
// The packs
// ---------------------------------------------------------------------------------------------------------------

/// The plugin's [`PackStore`]: the bundled packs, then the Packs folder (each `*.enthpack` file and each folder
/// holding a `pack.json`, in name order; hidden entries skipped). Reading verifies nothing: `load_library` reads every
/// pack through `enth-learn`'s reader.
#[derive(Clone, Debug)]
pub struct FsPacks {
    dir: Option<PathBuf>,
    bundled: &'static [(&'static str, &'static [u8])],
}

impl FsPacks {
    /// The bundled packs and the Packs folder `dir` (none: bundled only).
    pub fn new(dir: Option<PathBuf>) -> FsPacks {
        FsPacks {
            dir,
            bundled: BUNDLED,
        }
    }

    /// Another set of bundled packs (tests).
    #[must_use]
    pub fn with_bundled(mut self, bundled: &'static [(&'static str, &'static [u8])]) -> FsPacks {
        self.bundled = bundled;
        self
    }

    fn store_error(what: &str, path: &Path, e: &io::Error) -> LearnError {
        LearnError::Store {
            message: format!("{what} {}: {e}", path.display()),
        }
    }
}

/// Whether a file or folder name is hidden (`.DS_Store`, our temporary files).
fn hidden(name: &str) -> bool {
    name.starts_with('.')
}

/// A folder pack's files by pack path (`/`-separated), hidden entries skipped.
fn read_folder(root: &Path) -> Result<BTreeMap<String, Vec<u8>>, LearnError> {
    fn walk(
        dir: &Path,
        prefix: &str,
        out: &mut BTreeMap<String, Vec<u8>>,
        total: &mut u64,
    ) -> Result<(), LearnError> {
        let entries =
            fs::read_dir(dir).map_err(|e| FsPacks::store_error("cannot list", dir, &e))?;
        let mut names: Vec<(String, PathBuf)> = entries
            .filter_map(Result::ok)
            .filter_map(|e| Some((e.file_name().to_str()?.to_owned(), e.path())))
            .filter(|(n, _)| !hidden(n))
            .collect();
        names.sort();
        for (name, path) in names {
            let rel = if prefix.is_empty() {
                name.clone()
            } else {
                format!("{prefix}/{name}")
            };
            if path.is_dir() {
                walk(&path, &rel, out, total)?;
                continue;
            }
            if out.len() >= MAX_FOLDER_FILES {
                return Err(LearnError::TooManyEntries {
                    found: out.len() as u64 + 1,
                    max: MAX_FOLDER_FILES as u64,
                });
            }
            let size = fs::metadata(&path).map_or(0, |m| m.len());
            *total = total.saturating_add(size);
            if size > enth_learn::pack::MAX_FILE_BYTES || *total > enth_learn::pack::MAX_PACK_BYTES
            {
                return Err(LearnError::TooLarge {
                    path: rel,
                    size,
                    max: enth_learn::pack::MAX_FILE_BYTES,
                });
            }
            let bytes =
                fs::read(&path).map_err(|e| FsPacks::store_error("cannot read", &path, &e))?;
            out.insert(rel, bytes);
        }
        Ok(())
    }
    let mut out = BTreeMap::new();
    let mut total = 0;
    walk(root, "", &mut out, &mut total)?;
    Ok(out)
}

impl PackStore for FsPacks {
    fn scan(&mut self) -> Vec<PackKey> {
        let mut keys: Vec<PackKey> = self
            .bundled
            .iter()
            .map(|(name, _)| PackKey {
                origin: Origin::Bundled,
                name: (*name).to_owned(),
            })
            .collect();
        let Some(dir) = &self.dir else {
            return keys;
        };
        let Ok(entries) = fs::read_dir(dir) else {
            return keys;
        };
        let mut user: Vec<String> = entries
            .filter_map(Result::ok)
            .filter_map(|e| {
                let name = e.file_name().to_str()?.to_owned();
                let path = e.path();
                let pack = if path.is_dir() {
                    path.join(enth_learn::pack::MANIFEST_PATH).is_file()
                } else {
                    Path::new(&name)
                        .extension()
                        .is_some_and(|x| x.eq_ignore_ascii_case("enthpack"))
                };
                (pack && !hidden(&name)).then_some(name)
            })
            .collect();
        user.sort();
        keys.extend(user.into_iter().map(|name| PackKey {
            origin: Origin::User,
            name,
        }));
        keys
    }

    fn read(&self, key: &PackKey) -> Result<PackSource, LearnError> {
        let missing = || LearnError::NotFound {
            key: key.to_string(),
        };
        match key.origin {
            Origin::Bundled => self
                .bundled
                .iter()
                .find(|(name, _)| *name == key.name)
                .map(|(_, bytes)| PackSource::Archive(bytes.to_vec()))
                .ok_or_else(missing),
            Origin::User => {
                let dir = self.dir.as_ref().ok_or_else(missing)?;
                let path = dir.join(&key.name);
                if path.is_dir() {
                    return read_folder(&path).map(PackSource::Folder);
                }
                let meta = fs::metadata(&path).map_err(|_| missing())?;
                if meta.len() > MAX_PACK_FILE {
                    return Err(LearnError::TooLarge {
                        path: key.name.clone(),
                        size: meta.len(),
                        max: MAX_PACK_FILE,
                    });
                }
                fs::read(&path)
                    .map(PackSource::Archive)
                    .map_err(|e| FsPacks::store_error("cannot read", &path, &e))
            }
        }
    }

    fn write_user(&mut self, file_name: &str, bytes: &[u8]) -> Result<PackKey, LearnError> {
        let dir = self.dir.as_ref().ok_or_else(|| LearnError::Store {
            message: "no Packs folder: set ENTH_PACKS_DIR or HOME".to_owned(),
        })?;
        if hidden(file_name) || file_name.contains(['/', '\\']) {
            return Err(LearnError::Store {
                message: format!("{file_name:?} is not a pack file name"),
            });
        }
        fs::create_dir_all(dir).map_err(|e| FsPacks::store_error("cannot create", dir, &e))?;
        let path = dir.join(file_name);
        write_atomic(&path, bytes).map_err(|e| FsPacks::store_error("cannot write", &path, &e))?;
        Ok(PackKey {
            origin: Origin::User,
            name: file_name.to_owned(),
        })
    }
}

// ---------------------------------------------------------------------------------------------------------------
// The progress file
// ---------------------------------------------------------------------------------------------------------------

/// The per-user progress file (see the [module docs](self)). `None` when there is no Learn folder (no home): records
/// are dropped and nothing is read.
#[derive(Clone, Debug)]
pub struct ProgressFile {
    path: Option<PathBuf>,
}

/// Serialises writers within the process; the lock file serialises processes.
static WRITERS: Mutex<()> = Mutex::new(());

impl ProgressFile {
    /// The file at `path`.
    pub fn new(path: Option<PathBuf>) -> ProgressFile {
        ProgressFile { path }
    }

    /// Where it is.
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// The progress it holds: empty when it is missing, and when it is not a progress file (which is then set aside
    /// as `progress.json.unreadable-<seconds>`, never overwritten).
    pub fn load(&self) -> Progress {
        self.path
            .as_deref()
            .map_or_else(Progress::new, read_progress)
    }

    /// Merge `record` into the file: under the lock file, re-read, `Progress::record`, a temporary file and a rename.
    ///
    /// # Errors
    /// The folder or the file cannot be written (the record is lost; the step stays passed in the lesson).
    pub fn record(&self, record: &StepRecord) -> io::Result<()> {
        let Some(path) = self.path.as_deref() else {
            return Ok(());
        };
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let _process = WRITERS.lock();
        let lock = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(path.with_extension("json.lock"))?;
        lock.lock()?;
        let mut progress = read_progress(path);
        progress.record(record);
        let result = write_atomic(path, progress.to_json_pretty().as_bytes());
        let _ = lock.unlock();
        result
    }
}

/// Read the progress file at `path` (see [`ProgressFile::load`]).
fn read_progress(path: &Path) -> Progress {
    let Ok(text) = fs::read_to_string(path) else {
        return Progress::new();
    };
    if let Ok(p) = Progress::from_json(&text) {
        return p;
    }
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let aside = path.with_extension(format!("json.unreadable-{secs}"));
    let _ = fs::rename(path, aside);
    Progress::new()
}

// ---------------------------------------------------------------------------------------------------------------
// The library and the progress, shared by the shell's threads
// ---------------------------------------------------------------------------------------------------------------

/// The installed lessons and the progress file, shared by the GUI thread (the editor's lesson list), the worker
/// (`Open`) and whoever drains the session (progress records). The library is read once and cached; the editor
/// rescans when it opens. Its lock is never held while the session's is taken.
#[derive(Debug)]
pub struct Lessons {
    packs: Option<PathBuf>,
    bundled: &'static [(&'static str, &'static [u8])],
    progress: ProgressFile,
    library: Mutex<Option<Arc<Library>>>,
}

impl Lessons {
    /// The folders the environment names ([`packs_dir`], [`learn_dir`]).
    pub fn from_env() -> Lessons {
        let os = Os::current();
        Lessons::new(
            packs_dir(&SystemEnv, os),
            learn_dir(&SystemEnv, os).map(|d| d.join(PROGRESS_FILE)),
        )
    }

    /// The bundled packs, the Packs folder `packs` and the progress file `progress`.
    pub fn new(packs: Option<PathBuf>, progress: Option<PathBuf>) -> Lessons {
        Lessons {
            packs,
            bundled: BUNDLED,
            progress: ProgressFile::new(progress),
            library: Mutex::new(None),
        }
    }

    /// Read every pack again (the editor opened).
    pub fn rescan(&self) -> Arc<Library> {
        let mut store = FsPacks::new(self.packs.clone()).with_bundled(self.bundled);
        let library = Arc::new(load_library(&mut store));
        *self.library.lock() = Some(Arc::clone(&library));
        library
    }

    /// The library, read on first use.
    pub fn library(&self) -> Arc<Library> {
        if let Some(l) = self.library.lock().clone() {
            return l;
        }
        self.rescan()
    }

    /// One lesson ready to play, rescanning once if it is not in the cached library (installed since).
    pub fn bundle(&self, pack: &PackId, lesson: &LessonId) -> Option<LessonBundle> {
        let find = |lib: &Library| lib.pack(pack).and_then(|p| p.bundle(lesson));
        find(&self.library()).or_else(|| find(&self.rescan()))
    }

    /// Every lesson of every installed pack, in pack and lesson order, for the editor's list.
    pub fn catalogue(&self) -> Vec<LessonBundle> {
        let library = self.rescan();
        library
            .packs
            .iter()
            .flat_map(|e| {
                e.pack
                    .manifest()
                    .lessons
                    .iter()
                    .filter_map(|id| e.pack.bundle(id))
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    /// The packs the library refused, each as one line for the editor (`user:bad.enthpack: …`).
    pub fn problems(&self) -> Vec<String> {
        self.library()
            .problems
            .iter()
            .map(|p| format!("{}: {} ({})", p.key, p.error, p.error.hint()))
            .collect()
    }

    /// The progress file.
    pub fn progress(&self) -> &ProgressFile {
        &self.progress
    }

    /// Answer an `Open` job: the saved lesson's player resumed at its step, with what the progress file knows
    /// restored (a step passed before the reload stays passed and never records again). A step the lesson no longer
    /// has (an update with fewer steps) gives a player at step 0 and no effects: its other content id makes the
    /// session start it again. A pack that is gone is `Failed`: the lesson breaks, and Restore still brings the
    /// player's patch back.
    pub fn open(&self, saved: &LessonRef) -> LearnResult {
        let Some(bundle) = self.bundle(&saved.pack, &saved.lesson) else {
            return LearnResult::Failed {
                message: format!(
                    "the lesson {}/{} is not installed (its pack is missing)",
                    saved.pack, saved.lesson
                ),
            };
        };
        let same = bundle.content == saved.content;
        let key = bundle.key();
        match Player::resume(bundle.clone(), saved.step) {
            Ok((mut player, effects)) => {
                if same && let Some(p) = self.progress.load().lesson(&key) {
                    player.restore(p);
                }
                LearnResult::Opened {
                    player: Box::new(player),
                    effects,
                }
            }
            Err(_) => LearnResult::Opened {
                player: Box::new(Player::new(bundle)),
                effects: Vec::new(),
            },
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Jobs
// ---------------------------------------------------------------------------------------------------------------

/// A Listen or Choose step's examples, rendered: each one's loudness at the scored rate, and the renders at the
/// host's rate the audio thread plays as a set of loops.
#[derive(Debug)]
pub struct RenderedExamples {
    /// One per example: its integrated loudness (LUFS) at the scored rate, `None` if silent.
    pub lufs: Vec<Option<f32>>,
    /// The renders at the host's rate (the scored rate when the host's is unknown), in order.
    pub outs: Vec<RenderOutput>,
}

/// Run one Learn job (the game worker; see [`crate::game`]). `eval` is the game worker's hint evaluator; `host_rate`
/// is the rate the example loops are rendered at (0: the scored rate). An `Examples` job also returns its renders,
/// which the worker hands to the audio thread.
pub fn run(
    job: LearnJob,
    lessons: &Lessons,
    eval: &mut dyn Evaluator,
    host_rate: u32,
) -> (LearnResult, Option<RenderedExamples>) {
    match job.kind {
        LearnJobKind::Open { lesson } => (lessons.open(&lesson), None),
        // A check that cannot run answers a failed verdict, so the step goes back to work instead of showing
        // "checking" until the next edit (PLY's guidance).
        LearnJobKind::Check(request) => (
            LearnResult::Checked(
                evaluate_sounds(&request.sounds, &request.patch).unwrap_or_default(),
            ),
            None,
        ),
        LearnJobKind::Hint(request) => (measured_hint(&request, eval), None),
        LearnJobKind::Examples { examples } => match render_examples(&examples, host_rate) {
            Ok(rendered) => (
                LearnResult::Examples {
                    lufs: rendered.lufs.clone(),
                },
                Some(rendered),
            ),
            Err(message) => (LearnResult::Failed { message }, None),
        },
        _ => (
            LearnResult::Failed {
                message: "this build's worker does not know the learn job".to_owned(),
            },
            None,
        ),
    }
}

/// A measured hint for an expectation step: `enth_solver::hint` from the step's patch over its items, within the
/// step's free parameters, through the game worker's evaluator. The text names the largest move's parameter and
/// its direction (never a value), and the hint highlights every moved parameter; a blocked item says why.
fn measured_hint(request: &HintRequest, eval: &mut dyn Evaluator) -> LearnResult {
    let Some(phrase) = phrases::phrase(request.phrase) else {
        return LearnResult::Failed {
            message: format!("phrase {} is not in this build", request.phrase.0),
        };
    };
    let items: Result<Vec<Expect>, _> = request.expect.iter().map(|i| parse_item(i)).collect();
    let items = match items {
        Ok(items) => items,
        Err(e) => {
            return LearnResult::Failed { message: e.message };
        }
    };
    let cfg = HintCfg {
        only: (!request.free.is_empty()).then(|| request.free.clone()),
        ..HintCfg::default()
    };
    let spec = RenderSpec::new(SCORING_SAMPLE_RATE, SCORING_SEED);
    match hint(
        &request.patch,
        &phrase.performance(),
        &spec,
        &items,
        eval,
        &cfg,
    ) {
        Ok(result) => LearnResult::Hinted(hint_text(&result)),
        Err(e) => LearnResult::Failed {
            message: format!("the hint: {e}"),
        },
    }
}

/// The Learn view's words for a hint result.
fn hint_text(result: &enth_solver::hint::HintResult) -> MeasuredHint {
    if let Some(fix) = &result.fix {
        let mut moves: Vec<_> = fix.moves.iter().collect();
        moves.sort_by(|a, b| b.normalised().abs().total_cmp(&a.normalised().abs()));
        if let Some(m) = moves.first() {
            let way = if m.to < m.from { "down" } else { "up" };
            let highlight = moves
                .iter()
                .filter_map(|m| m.addr.to_string().parse::<Selector>().ok())
                .collect();
            return MeasuredHint {
                text: format!("Try turning {} ({}) {way}.", m.spec.name, m.addr),
                highlight,
            };
        }
    }
    if let Some(b) = result.blocked.first() {
        return MeasuredHint {
            text: b.why.clone(),
            highlight: b
                .at_bound
                .iter()
                .filter_map(|a| a.to_string().parse::<Selector>().ok())
                .collect(),
        };
    }
    MeasuredHint {
        text: "It already sounds right: hold still a moment for the check.".to_owned(),
        highlight: Vec::new(),
    }
}

/// What an example plays: its phrase, transposed, with its control curve.
fn performance(a: &Audition) -> Option<Performance> {
    let mut perf = phrases::phrase(a.phrase)?.performance();
    for n in &mut perf.notes {
        n.key = u8::try_from((i16::from(n.key) + i16::from(a.transpose)).clamp(0, 127))
            .unwrap_or(n.key);
    }
    if let Some(curve) = &a.curve {
        perf.controls = curve_controls(curve, perf.duration_s);
    }
    Some(perf)
}

/// A control curve as controller moves every [`CURVE_STEP_S`], linear between its points and held after the last.
fn curve_controls(curve: &ControlCurve, duration_s: f64) -> Vec<Control> {
    let Some(controller) = Controller::from_source(&curve.source) else {
        return Vec::new();
    };
    let at = |t_s: f64| -> f32 {
        let ms = t_s * 1000.0;
        let pts = &curve.points;
        let pct = match pts.iter().position(|p| f64::from(p.ms) > ms) {
            None => pts.last().map_or(0.0, |p| f64::from(p.pct)),
            Some(0) => f64::from(pts[0].pct),
            Some(i) => {
                let (a, b) = (pts[i - 1], pts[i]);
                let span = f64::from(b.ms - a.ms).max(1.0);
                let x = (ms - f64::from(a.ms)) / span;
                f64::from(a.pct) + (f64::from(b.pct) - f64::from(a.pct)) * x
            }
        };
        (pct / 100.0) as f32
    };
    let end = curve
        .points
        .last()
        .map_or(0.0, |p| f64::from(p.ms) / 1000.0)
        .min(duration_s);
    let steps = (end / CURVE_STEP_S).ceil() as usize;
    (0..=steps)
        .map(|i| {
            let t = (i as f64 * CURVE_STEP_S).min(end);
            Control {
                time_s: t,
                controller,
                value: at(t),
            }
        })
        .collect()
}

/// Render each example (its seed, or the scored seed) at the scored rate for its loudness, and again at `host_rate`
/// when that differs, for the loops.
///
/// # Errors
/// An example whose phrase this build lacks, or a render that fails.
pub fn render_examples(examples: &[Audition], host_rate: u32) -> Result<RenderedExamples, String> {
    let rate = if host_rate == 0 {
        SCORING_SAMPLE_RATE
    } else {
        host_rate
    };
    let mut lufs = Vec::with_capacity(examples.len());
    let mut outs = Vec::with_capacity(examples.len());
    for a in examples {
        let perf = performance(a)
            .ok_or_else(|| format!("example {:?}: its phrase is not in this build", a.label))?;
        let seed = a.seed.map_or(SCORING_SEED, u64::from);
        let scored = render(&a.patch, &perf, &RenderSpec::new(SCORING_SAMPLE_RATE, seed))
            .map_err(|e| format!("example {:?}: {e}", a.label))?;
        lufs.push(crate::game::integrated_lufs(&scored));
        let out = if rate == SCORING_SAMPLE_RATE {
            scored
        } else {
            render(&a.patch, &perf, &RenderSpec::new(rate, seed))
                .map_err(|e| format!("example {:?}: {e}", a.label))?
        };
        outs.push(out);
    }
    Ok(RenderedExamples { lufs, outs })
}

// ---------------------------------------------------------------------------------------------------------------
// The shell's side
// ---------------------------------------------------------------------------------------------------------------

/// After unlocking: act on the Learn requests a drain took, in order. Jobs go to the game worker, cancels remove
/// queued jobs, progress records are merged into the file, the Listen state goes to `process()`'s atomics (and the
/// example loops are dropped when it is no longer ready). Returns whether the project must be marked dirty
/// (coalesced with `ShellRequest::PatchEdited` and `GameRequest::Dirty` by the caller: one mark per drain).
pub fn act(game: &crate::game::Game, requests: Vec<LearnRequest>) -> bool {
    let mut dirty = false;
    for request in requests {
        match request {
            LearnRequest::Run(job) => game.run_learn(job),
            LearnRequest::Cancel(id) => game.cancel_learn(id),
            LearnRequest::Record { record, .. } => {
                // A record that cannot be written is lost; the step stays passed in the lesson itself.
                let _ = game.lessons().progress().record(&record);
            }
            LearnRequest::Listen(state) => game.listen(&state),
            LearnRequest::Dirty => dirty = true,
            _ => {}
        }
    }
    dirty
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::disallowed_methods,
        reason = "the tests read content/lessons and write temporary folders"
    )]

    use super::*;
    use enth_learn::pack::{MANIFEST_PATH, lesson_path};
    use enth_learn::progress::StepProgress;
    use enth_learn::reader::write_archive;
    use enth_learn::{Lesson, Manifest};

    /// A fresh temporary folder for one test.
    fn temp(name: &str) -> PathBuf {
        static N: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "enth-learn-{name}-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// `content/lessons/<pack>/` built as `cargo xtask pack` will (lane LCK): TOML through the same serde types,
    /// texts, patches and credits copied, the manifest sealed, the canonical archive written.
    fn build(pack: &str) -> Vec<u8> {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../content/lessons")
            .join(pack);
        let manifest =
            Manifest::from_toml(&fs::read_to_string(dir.join("pack.toml")).unwrap()).unwrap();
        let mut files = BTreeMap::new();
        files.insert(
            "CREDITS.md".to_owned(),
            fs::read(dir.join("CREDITS.md")).unwrap(),
        );
        for entry in fs::read_dir(dir.join("patches")).unwrap() {
            let path = entry.unwrap().path();
            let name = path.file_name().unwrap().to_str().unwrap().to_owned();
            files.insert(format!("patches/{name}"), fs::read(&path).unwrap());
        }
        for id in &manifest.lessons {
            for entry in fs::read_dir(dir.join("lessons").join(id.as_str())).unwrap() {
                let path = entry.unwrap().path();
                let name = path.file_name().unwrap().to_str().unwrap().to_owned();
                if name == "lesson.toml" {
                    let lesson = Lesson::from_toml(&fs::read_to_string(&path).unwrap()).unwrap();
                    files.insert(lesson_path(id), lesson.to_json().into_bytes());
                } else {
                    files.insert(format!("lessons/{id}/{name}"), fs::read(&path).unwrap());
                }
            }
        }
        let sealed = manifest.seal(&files).unwrap();
        files.insert(MANIFEST_PATH.to_owned(), sealed.to_json().into_bytes());
        write_archive(&files)
    }

    #[test]
    fn the_bundled_packs_are_the_content_built() {
        for (name, bytes) in BUNDLED {
            let pack = name.trim_end_matches(".enthpack");
            let built = build(pack);
            if std::env::var_os("ENTH_UPDATE_PACKS").is_some() {
                let path = Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("packs")
                    .join(name);
                fs::write(&path, &built).unwrap();
                continue;
            }
            assert!(
                built == *bytes,
                "packs/{name} is stale: rebuild it with `ENTH_UPDATE_PACKS=1 cargo test -p enth-plugin --features learn \
                 -- the_bundled_packs_are_the_content_built` (and rebuild the plugin)"
            );
        }
    }

    #[test]
    fn the_bundled_example_pack_reads_and_plays_its_first_step() {
        let lessons = Lessons::new(None, None);
        let lib = lessons.library();
        assert!(lib.problems.is_empty(), "{:?}", lessons.problems());
        let catalogue = lessons.catalogue();
        assert_eq!(catalogue.len(), 1);
        let bundle = &catalogue[0];
        assert_eq!(bundle.key().to_string(), "example/cutoff");
        let mut player = Player::new(bundle.clone());
        let effects = player.start().unwrap();
        assert!(
            effects
                .iter()
                .any(|e| matches!(e, enth_learn::Effect::Examples(x) if x.len() == 2)),
            "the listen step's two examples: {effects:?}"
        );
    }

    #[test]
    fn user_packs_are_scanned_beside_the_bundled_ones_and_a_newer_one_wins() {
        let dir = temp("packs");
        // A newer copy of the example pack, as an archive, and a damaged one, as a folder.
        let mut files = enth_learn::reader::extract(BUNDLED[0].1).unwrap();
        let mut manifest =
            Manifest::from_json(std::str::from_utf8(&files[MANIFEST_PATH]).unwrap()).unwrap();
        manifest.version = "0.2.0".parse().unwrap();
        files.remove(MANIFEST_PATH);
        let sealed = manifest.seal(&files).unwrap();
        files.insert(MANIFEST_PATH.to_owned(), sealed.to_json().into_bytes());
        let mut store = FsPacks::new(Some(dir.clone()));
        let key = store
            .write_user("example.enthpack", &write_archive(&files))
            .unwrap();
        assert_eq!(key.origin, Origin::User);
        fs::create_dir_all(dir.join("broken")).unwrap();
        fs::write(dir.join("broken").join(MANIFEST_PATH), b"{}").unwrap();
        fs::write(dir.join(".hidden.enthpack"), b"x").unwrap();
        fs::write(dir.join("notes.txt"), b"x").unwrap();
        let keys = store.scan();
        let names: Vec<String> = keys.iter().map(ToString::to_string).collect();
        assert_eq!(
            names,
            [
                "bundled:example.enthpack",
                "user:broken",
                "user:example.enthpack"
            ]
        );
        let lib = load_library(&mut store);
        assert_eq!(lib.packs.len(), 1);
        assert_eq!(
            lib.packs[0].key.origin,
            Origin::User,
            "the newer version is kept"
        );
        assert_eq!(
            lib.problems.len(),
            1,
            "the broken folder is listed, not hidden"
        );
        assert_eq!(lib.replaced.len(), 1);
        let _ = fs::remove_dir_all(&dir);
    }

    fn record(lesson: &str, step: &str, clean: bool) -> StepRecord {
        StepRecord {
            lesson: lesson.parse().unwrap(),
            step: step.parse().unwrap(),
            progress: StepProgress {
                done: true,
                clean,
                best: None,
                extra: BTreeMap::new(),
            },
        }
    }

    #[test]
    fn two_writers_never_lose_each_others_progress() {
        let dir = temp("progress");
        let path = dir.join(PROGRESS_FILE);
        let a = ProgressFile::new(Some(path.clone()));
        let b = ProgressFile::new(Some(path.clone()));
        let threads: Vec<_> = (0..8)
            .map(|i| {
                let f = if i % 2 == 0 { a.clone() } else { b.clone() };
                std::thread::spawn(move || {
                    for j in 0..5 {
                        f.record(&record("example/cutoff", &format!("s{i}-{j}"), j % 2 == 0))
                            .unwrap();
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        let progress = a.load();
        let lesson = progress.lesson(&"example/cutoff".parse().unwrap()).unwrap();
        assert_eq!(
            lesson.steps.len(),
            40,
            "every writer's every record is kept"
        );
        // Never shrinks: a later record that is not clean keeps the clean one.
        a.record(&record("example/cutoff", "s0-0", false)).unwrap();
        let again = b.load();
        assert!(
            again
                .step(&"example/cutoff".parse().unwrap(), &"s0-0".parse().unwrap())
                .unwrap()
                .clean
        );
        let leftovers: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .filter_map(Result::ok)
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| {
                Path::new(n)
                    .extension()
                    .is_some_and(|x| x.eq_ignore_ascii_case("tmp"))
            })
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_that_is_not_progress_is_set_aside_not_overwritten() {
        let dir = temp("aside");
        let path = dir.join(PROGRESS_FILE);
        fs::write(&path, b"my notes, not progress").unwrap();
        let file = ProgressFile::new(Some(path.clone()));
        file.record(&record("example/cutoff", "try", true)).unwrap();
        let aside: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().contains("unreadable"))
            .collect();
        assert_eq!(aside.len(), 1);
        assert_eq!(
            fs::read(aside[0].path()).unwrap(),
            b"my notes, not progress"
        );
        assert!(
            file.load()
                .lesson(&"example/cutoff".parse().unwrap())
                .is_some()
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn open_resumes_at_the_saved_step_with_the_steps_passed_before() {
        let dir = temp("open");
        let lessons = Lessons::new(None, Some(dir.join(PROGRESS_FILE)));
        let bundle = lessons.catalogue().remove(0);
        lessons
            .progress()
            .record(&record("example/cutoff", "hear", true))
            .unwrap();
        lessons
            .progress()
            .record(&record("example/cutoff", "try", true))
            .unwrap();
        let LearnResult::Opened { player, effects } = lessons.open(&bundle.lesson_ref(1)) else {
            panic!("opened");
        };
        assert_eq!(player.lesson_ref().step, 1);
        assert_eq!(
            player.phase(),
            enth_learn::StepPhase::Passed,
            "try passed before the reload"
        );
        assert!(
            effects
                .iter()
                .all(|e| !matches!(e, enth_learn::Effect::LoadPatch(_))),
            "a resume loads no patch"
        );
        // A pack that is gone.
        let mut gone = bundle.lesson_ref(1);
        gone.pack = "nothere".parse().unwrap();
        assert!(matches!(lessons.open(&gone), LearnResult::Failed { .. }));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_example_steps_check_renders_and_judges_the_start_and_the_witness() {
        let bundle = Lessons::new(None, None).catalogue().remove(0);
        let enth_learn::StepKind::Do { check, .. } = &bundle.lesson.steps[1].kind else {
            panic!("step 2 is a do step");
        };
        let sounds = check.sounds();
        let patch = |name: &str| {
            let r = enth_learn::pack::PatchRef::new(name).unwrap();
            (**bundle.patch(&r).unwrap()).clone()
        };
        let start = evaluate_sounds(&sounds, &patch("patches/saw.patch.json")).unwrap();
        let witness = evaluate_sounds(&sounds, &patch("patches/dark.patch.json")).unwrap();
        assert!(!start.passed && witness.passed, "{start:?} {witness:?}");
        assert_eq!(witness.items.len(), 1);
    }

    /// PLY's `evaluate_sounds` repeats a few lines of `enth_solver::measure` (a core crate may not call its sibling):
    /// both must hear the same thing, so the judged values equal `measure`'s descriptors judged by `enth_analysis`.
    #[test]
    fn the_sound_check_measures_what_the_solver_measures() {
        use enth_challenge::model::{SCORING_SAMPLE_RATE, SCORING_SEED};
        let bundle = Lessons::new(None, None).catalogue().remove(0);
        let enth_learn::StepKind::Do { check, .. } = &bundle.lesson.steps[1].kind else {
            panic!("step 2 is a do step");
        };
        let sounds = check.sounds();
        for name in ["patches/saw.patch.json", "patches/dark.patch.json"] {
            let r = enth_learn::pack::PatchRef::new(name).unwrap();
            let patch = (**bundle.patch(&r).unwrap()).clone();
            let ours =
                enth_learn::check::evaluate_sounds_report(&sounds, &patch, Default::default())
                    .unwrap();
            for (sound, report) in sounds.iter().zip(&ours) {
                let items: Vec<Expect> = sound
                    .expect
                    .iter()
                    .map(|i| parse_item(i).unwrap())
                    .collect();
                let perf = phrases::phrase(sound.phrase).unwrap().performance();
                let features: Vec<_> = items.iter().map(|e| e.feature).collect();
                let job = enth_solver::Job {
                    patch: sound.pinned(&patch),
                    perf: &perf,
                    spec: RenderSpec::new(SCORING_SAMPLE_RATE, SCORING_SEED),
                    want: enth_analysis::descriptors::Want::for_features(&features),
                };
                let d = enth_solver::measure(&job, &mut None).unwrap();
                let solver = enth_analysis::check(&items, &d, &[]);
                assert_eq!(report.report.results, solver.results, "{name}");
            }
        }
    }

    #[test]
    fn examples_render_at_the_host_rate_and_measure_at_the_scored_rate() {
        let bundle = Lessons::new(None, None).catalogue().remove(0);
        let mut player = Player::new(bundle);
        let effects = player.start().unwrap();
        let Some(enth_learn::Effect::Examples(examples)) = effects
            .into_iter()
            .find(|e| matches!(e, enth_learn::Effect::Examples(_)))
        else {
            panic!("examples");
        };
        let r = render_examples(&examples, 44_100).unwrap();
        assert_eq!(r.lufs.len(), 2);
        assert!(r.lufs.iter().all(Option::is_some));
        assert!(r.outs.iter().all(|o| o.sample_rate == 44_100));
        assert!(
            r.lufs[0] > r.lufs[1],
            "the open sound is louder than the closed one"
        );
    }

    #[test]
    fn a_curve_is_linear_between_its_points_and_held_after() {
        let curve: ControlCurve = serde_json::from_str(
            r#"{"source":"modwheel","points":[{"ms":0,"pct":0},{"ms":100,"pct":100}]}"#,
        )
        .unwrap();
        let c = curve_controls(&curve, 1.0);
        assert_eq!(c.first().unwrap().value, 0.0);
        assert!((c[5].value - 0.5).abs() < 1e-3);
        assert_eq!(c.last().unwrap().value, 1.0);
    }

    #[test]
    fn the_folders_follow_the_environment() {
        struct Env(Vec<(&'static str, &'static str)>);
        impl Environment for Env {
            fn var(&self, name: &str) -> Option<std::ffi::OsString> {
                self.0
                    .iter()
                    .find(|(k, _)| *k == name)
                    .map(|(_, v)| (*v).into())
            }
            fn read_text(&self, _: &Path) -> Option<String> {
                None
            }
        }
        let env = Env(vec![("HOME", "/home/u"), ("ENTH_LEARN_DIR", "/tmp/l")]);
        assert_eq!(
            packs_dir(&env, Os::MacOs),
            Some(PathBuf::from("/home/u/Documents/enth/Packs"))
        );
        assert_eq!(learn_dir(&env, Os::MacOs), Some(PathBuf::from("/tmp/l")));
    }
}
