//! **Owner: lane LCK.** Pre-declared by lane S0l (R2).
//!
//! `check [--cross] [--solve] [--budget N] [PACK...]` (`cargo xtask lesson-check`): an honest lesson is one this
//! passes (docs/learn.md, "`lesson-check` and the authoring loop"). Over every pack in `content/lessons/` (or the packs
//! named: an id, a pack directory or an `.enthpack`), deterministically and with no spend:
//!
//! 1. **Files** ([`files`]): the authoring layout builds (lane C1's rule: a pack directory holds only what its manifest
//!    lists, and every file is used), `pack.toml` and every `lesson.toml` decode strictly (unknown fields refused),
//!    `requires` is met and covers what the lessons use, the built archive reads back through the real reader (every
//!    file listed, at its size and SHA-256, nothing unlisted), the pins match ([`crate::pack`]), every patch file is
//!    `Patch::to_json()` byte for byte, the licences are `CC-BY-4.0` or `CC0-1.0`, `covers` names exactly the module
//!    kinds the lessons teach, track entries name lessons that exist, and every text passes the markdown lint.
//! 2. **Playthrough** ([`play`]): each lesson is played through a headless `enth_session::Session` with the real
//!    `enth_learn::Player`, every job run inline between frames. From each step's start, every check **fails before**
//!    and **passes after** its reference solution: a Do step's witness, applied by Show me through the session's edit
//!    path (locks hold); a Recreate step's witness (or its target), submitted to the real round; an ear question's
//!    wrong and right picks. Sound checks print their margins, re-rendered at `enth check`'s two nuisance seeds: at the
//!    witness every item passes by at least [`MIN_MARGIN`] and none is `thin`, and at the start an item fails by at
//!    least as much. A witness that touches a locked control, or one the step does not list as editable, fails. Every
//!    `enth:` link and widget resolves against the step's start, and challenge codes decode back to their spec.
//! 3. **`--cross`** ([`cross`]): a Do step's check does not pass another Do step's witness (in its lesson and the
//!    lessons beside it) unless that step's start already passes it.
//! 4. **`--solve`** ([`solve`]): the hint follower (the player's measured hints, `enth_solver::hint`, applied) reaches
//!    a sound step's pass from its start within the editable set, or the witness shows structure the hints cannot
//!    move; `closest` passes every Recreate round from its start within `--budget` renders (300); every ear question's
//!    options are at least `MIN_JND` (2) apart under `sound/1` (an inaudible choice fails; options `sound/1` cannot
//!    compare are listed for a listening review, and `lesson-coverage` counts them as no check).
//!
//! Coverage is its own command (`lesson-coverage`). The exit status is the verdict: 0 only when nothing failed.

mod cross;
mod files;
mod ops;
mod play;
mod solve;

use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use enth_learn::Pack;

use crate::source;

/// The smallest margin a sound check's items keep at the witness (and its failing item at the start), in the
/// feature's unit, as the golden challenges do (design.md §5).
pub(crate) const MIN_MARGIN: f64 = 1e-3;

/// The options of one run.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Options {
    /// `--cross`.
    pub cross: bool,
    /// `--solve`.
    pub solve: bool,
    /// `closest`'s render budget for a Recreate round.
    pub budget: usize,
    /// How many lessons are checked at once (`--jobs`; the report's order and content do not depend on it).
    pub jobs: usize,
}

/// The report: lines on stdout (at once for the run's own log, in lesson order for a lesson's), and every failure
/// again at the end.
#[derive(Debug, Default)]
pub(crate) struct Log {
    /// Print each line at once; otherwise keep it for [`Log::absorb`].
    live: bool,
    lines: Vec<String>,
    failures: Vec<String>,
    notes: usize,
    steps: usize,
}

impl Log {
    /// The run's own log, which prints as it goes.
    fn live() -> Log {
        Log {
            live: true,
            ..Log::default()
        }
    }

    /// A line of the report.
    pub fn line(&mut self, text: impl Into<String>) {
        let text = text.into();
        if self.live {
            println!("{text}");
        } else {
            self.lines.push(text);
        }
    }

    /// A failure at `at` (a pack, `pack/lesson` or `pack/lesson#step`).
    pub fn fail(&mut self, at: &str, what: impl AsRef<str>) {
        let what = what.as_ref();
        self.line(format!("      FAIL {what}"));
        self.failures.push(format!("{at}: {what}"));
    }

    /// Something to look at that is not a failure.
    pub fn note(&mut self, what: impl AsRef<str>) {
        self.notes += 1;
        self.line(format!("      note {}", what.as_ref()));
    }

    /// Take in a lesson's log: its lines, failures, notes and steps.
    fn absorb(&mut self, other: Log) {
        for line in other.lines {
            self.line(line);
        }
        self.failures.extend(other.failures);
        self.notes += other.notes;
        self.steps += other.steps;
    }

    /// A step was checked.
    pub fn step(&mut self) {
        self.steps += 1;
    }

    /// How many failures so far.
    pub fn failures(&self) -> usize {
        self.failures.len()
    }
}

/// `check`.
pub fn run(args: &[String]) -> Result<(), String> {
    let mut opts = Options {
        cross: false,
        solve: false,
        budget: enth_solver::closest::DEFAULT_BUDGET,
        jobs: default_jobs(),
    };
    let mut names = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--cross" => opts.cross = true,
            "--solve" => opts.solve = true,
            "--jobs" => {
                opts.jobs = it
                    .next()
                    .and_then(|n| n.parse().ok())
                    .filter(|n| *n > 0)
                    .ok_or("--jobs needs a number above 0")?;
            }
            "--budget" => {
                opts.budget = it
                    .next()
                    .and_then(|n| n.parse().ok())
                    .filter(|n| *n > 0)
                    .ok_or("--budget needs a number of renders above 0")?;
            }
            flag if flag.starts_with("--") => {
                return Err(format!(
                    "check: unknown option {flag}\nusage: check [--cross] [--solve] [--budget N] [--jobs N] [<pack>...]"
                ));
            }
            name => names.push(name.to_owned()),
        }
    }
    let root = source::repo_root();
    let paths = source::resolve(&root, &names)?;
    let started = Instant::now();
    let mut log = Log::live();
    let mut packs: Vec<Pack> = Vec::new();
    let mut lessons = 0_usize;
    for path in &paths {
        if let Some(pack) = check_pack(&root, path, opts, &mut log) {
            lessons += pack.lessons().count();
            packs.push(pack);
        }
    }
    files::tracks(&packs, &mut log);
    let secs = started.elapsed().as_secs_f64();
    println!(
        "lesson-check{}{}: {} pack(s), {lessons} lesson(s), {} step(s), {} note(s), {} failure(s) in {secs:.1} s",
        if opts.cross { " --cross" } else { "" },
        if opts.solve { " --solve" } else { "" },
        paths.len(),
        log.steps,
        log.notes,
        log.failures.len(),
    );
    if log.failures.is_empty() {
        Ok(())
    } else {
        println!("failures:");
        for f in &log.failures {
            println!("  {f}");
        }
        Err(format!("lesson-check: {} failure(s)", log.failures.len()))
    }
}

/// Check one pack: files, then each lesson's playthrough, `--cross` and `--solve`. The verified pack, when it reads.
fn check_pack(root: &Path, path: &Path, opts: Options, log: &mut Log) -> Option<Pack> {
    let started = Instant::now();
    log.line(format!("pack {}", path.display()));
    let at = path.display().to_string();
    let source = match source::read(path) {
        Ok(s) => s,
        Err(problems) => {
            for p in problems {
                log.fail(&at, p);
            }
            return None;
        }
    };
    let pack = match Pack::from_files(&source.files) {
        Ok(p) => p,
        Err(e) => {
            log.fail(&at, source::learn_error(&e));
            return None;
        }
    };
    let at = pack.id().to_string();
    files::check(root, &source, &pack, log);

    let lessons: Vec<_> = pack
        .manifest()
        .lessons
        .iter()
        .filter_map(|id| pack.bundle(id))
        .collect();
    // Each lesson's playthrough and `--solve` on their own (a session, a player and renders per lesson), in parallel;
    // the report keeps the lessons' order.
    let done = AtomicUsize::new(0);
    let results = parallel(&lessons, opts.jobs, |bundle| {
        let mut l = Log::default();
        let played = play::lesson(bundle, &mut l);
        if opts.solve {
            solve::lesson(bundle, &played, opts, &mut l);
        }
        let n = done.fetch_add(1, Ordering::Relaxed) + 1;
        eprintln!("lesson-check: {n}/{} {}", lessons.len(), bundle.key());
        (l, played)
    });
    let mut played = Vec::with_capacity(lessons.len());
    for (l, p) in results {
        log.absorb(l);
        played.push(p);
    }
    if opts.cross {
        cross::run(&lessons, &played, log);
    }
    log.line(format!(
        "  {at}: {} lesson(s) in {:.1} s",
        lessons.len(),
        started.elapsed().as_secs_f64()
    ));
    Some(pack)
}

/// `--jobs` when not given: `ENTH_LESSON_JOBS`, else the machine's parallelism, at most 4 (CI's runners have 4).
fn default_jobs() -> usize {
    std::env::var("ENTH_LESSON_JOBS")
        .ok()
        .and_then(|j| j.parse().ok())
        .filter(|j| *j > 0)
        .unwrap_or_else(|| std::thread::available_parallelism().map_or(1, |n| n.get().min(4)))
}

/// `f` over `items` on up to `jobs` threads, the results in the items' order. Each item's work is independent and
/// deterministic, so the results do not depend on `jobs`.
fn parallel<T: Sync, R: Send>(items: &[T], jobs: usize, f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let next = AtomicUsize::new(0);
    let slots: Mutex<Vec<Option<R>>> = Mutex::new(items.iter().map(|_| None).collect());
    std::thread::scope(|s| {
        for _ in 0..jobs.clamp(1, items.len().max(1)) {
            s.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(item) = items.get(i) else {
                        break;
                    };
                    let r = f(item);
                    slots.lock().expect("no worker panicked holding the lock")[i] = Some(r);
                }
            });
        }
    });
    slots
        .into_inner()
        .expect("no worker panicked holding the lock")
        .into_iter()
        .map(|r| r.expect("every item was run"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::parallel;

    #[test]
    fn parallel_keeps_the_items_order_whatever_the_jobs() {
        let items: Vec<u64> = (0..37).collect();
        let want: Vec<u64> = items.iter().map(|i| i * i).collect();
        for jobs in [1, 2, 4, 8, 64] {
            assert_eq!(parallel(&items, jobs, |i| i * i), want, "{jobs} jobs");
        }
        assert_eq!(parallel(&[] as &[u64], 4, |i| *i), Vec::<u64>::new());
    }
}
