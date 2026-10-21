//! **`enth-listen gr3`** (lane LT, R3): the blind GR3 listening test of model + search against search alone on real and
//! degraded sounds (docs/r3.md, "GR3 and its hygiene", clause 2; docs/listening.md, "5. GR3").
//!
//! The trial: the reference R, then A, then B, "which is closer to R?", forced choice. A and B are two arms' **top-ranked**
//! candidates by their own objective (never a human pick), their order counterbalanced per reference. Every clip, R
//! included, is scaled to -23 LUFS (the in-house BS.1770), and a trial with a clip over -1 dBTP is lowered as a whole.
//! Seven catch trials per listener: two identical pairs (bias) and five against an obviously wrong candidate (attention);
//! a listener under 80 % on attention is excluded (one miss in five is kept, two are out). **Degraded references are judged against the clean original.**
//!
//! ```text
//! enth-listen gr3 --set set.toml --dir D       design once (the key, the owner's clips), then the owner's 67 trials
//! enth-listen gr3 --dir D --export-kit K       the friends' kits: K/kit1, K/kit2 (random-named WAVs, answers.txt)
//! enth-listen gr3 --dir D --analyze            per listener, strata, the decision rule fixed in advance
//! enth-listen gr3 --dry-run --dir D            all of it with a synthetic set and synthetic answers; plays nothing
//! ```
//!
//! The modules: [`set`] (the set manifest EVK and DS fill), [`design`] (trials, counterbalancing, levels, the key),
//! [`session`] (the owner's loop), [`answers`] (the friends' text files), [`analyze`] and [`boot`] (the rule and its
//! bootstrap, with a Python reference in `tests/gr3/`), [`audio`] (WAV reading, loudness, true peak), [`dry`] (the
//! synthetic set and listeners). **The listening key stays closed until every listener is done**, and whoever prepares
//! stimuli does not audition them; the tool never plays audio in a test (`--dry-run`, a recording mock player).

pub mod analyze;
pub mod answers;
pub mod audio;
pub mod boot;
pub mod design;
pub mod dry;
pub mod session;
pub mod set;

use std::io::Write;
use std::path::{Path, PathBuf};

use self::analyze::{Report, SCHEMA_REPORT, analyze};
use self::answers::{Responses, kit_answers_text, kit_readme, owner_responses, parse_kit_answers};
use self::boot::DEFAULT_REPLICATES;
use self::design::{
    DEFAULT_SEED, Manifest, OWNER, SCHEMA_ANSWERS, SCHEMA_MANIFEST, check_inputs, design, kit_name,
    manifest_path, write_trial_clips,
};
use self::dry::{demo_set, synthetic_choices};
use self::session::{SessionEnd, answers_path, estimate_minutes, run_session};
use self::set::Set;
use crate::Res;
use crate::files::{read_json, write_json};
use crate::player::{CommandPlayer, DEFAULT_PLAYER, Player};
use crate::session::{Answer, AnswerFile, LineSource, load_answers};

/// `enth-listen gr3`'s arguments.
#[derive(Clone, Debug, PartialEq, Eq, clap::Args)]
pub struct Gr3Args {
    /// Where the key (manifest.json), the owner's clips and answers, and the friends' returned files live.
    #[arg(long, default_value = "gr3-test")]
    pub dir: PathBuf,
    /// The set manifest (TOML, `enth.listen.gr3-set/1`): the references and each arm's candidates. Needed the first
    /// time; later runs use the set recorded in the key, and a --set given again must be the same set.
    #[arg(long)]
    pub set: Option<PathBuf>,
    /// Write the friends' kits into DIR/kit1, DIR/kit2, ...: random-named WAVs and an answers.txt for any OS and player.
    /// The key stays in --dir; never send manifest.json.
    #[arg(long, value_name = "DIR")]
    pub export_kit: Option<PathBuf>,
    /// Read the answers (answers.json and friends/*.txt) and apply the decision rule fixed in advance.
    #[arg(long)]
    pub analyze: bool,
    /// With --analyze: show a result although a listener has not finished (the key is meant to stay closed until
    /// everyone is done; use this when a friend will not answer).
    #[arg(long)]
    pub partial: bool,
    /// Kits for friends, decided when the folder is designed (default 2; 0 for an owner-only test).
    #[arg(long)]
    pub friends: Option<usize>,
    /// Design in DIR/dry-run (a synthetic set unless --set is given), answer everything with synthetic answers,
    /// export the kits and report. Plays nothing.
    #[arg(long)]
    pub dry_run: bool,
    /// Command that plays one WAV file (the path is appended).
    #[arg(long, default_value = DEFAULT_PLAYER)]
    pub player: String,
    /// Seeds the design. A folder keeps the seed it was designed with.
    #[arg(long)]
    pub seed: Option<u64>,
    /// Bootstrap replicates behind the report's intervals (the seed is fixed in advance and not a flag).
    #[arg(long, default_value_t = DEFAULT_REPLICATES)]
    pub bootstrap: usize,
}

impl Gr3Args {
    /// Defaults for `dir` (what clap gives with only `--dir`).
    #[must_use]
    pub fn new(dir: &Path) -> Self {
        Self {
            dir: dir.to_path_buf(),
            set: None,
            export_kit: None,
            analyze: false,
            partial: false,
            friends: None,
            dry_run: false,
            player: DEFAULT_PLAYER.to_string(),
            seed: None,
            bootstrap: DEFAULT_REPLICATES,
        }
    }
}

/// The friends' returned answer files go here (any file name; the listener is the name without `.txt`).
#[must_use]
pub fn friends_dir(dir: &Path) -> PathBuf {
    dir.join("friends")
}

/// The report's path.
#[must_use]
pub fn report_path(dir: &Path) -> PathBuf {
    dir.join("gr3-report.json")
}

fn load_manifest(dir: &Path) -> Res<Option<Manifest>> {
    let p = manifest_path(dir);
    if !p.exists() {
        return Ok(None);
    }
    let m: Manifest = read_json(&p)?;
    if m.schema != SCHEMA_MANIFEST {
        return Err(format!(
            "{} has schema {}, expected {SCHEMA_MANIFEST}",
            p.display(),
            m.schema
        )
        .into());
    }
    Ok(Some(m))
}

fn print_design(out: &mut dyn Write, m: &Manifest) -> std::io::Result<()> {
    let clean = m.items.iter().filter(|i| !i.degraded).count();
    let owner = m.trials_of(OWNER);
    let lowered = owner.iter().filter(|t| t.levels.lowered_db > 0.0).count();
    writeln!(
        out,
        "{} references ({} clean, {} degraded) in {} strata; the owner hears {} trials (about {:.0} min), {} kit(s) of {} trials",
        m.items.len(),
        clean,
        m.items.len() - clean,
        m.items
            .iter()
            .map(|i| i.stratum.as_str())
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        owner.len(),
        estimate_minutes(m, OWNER),
        m.kits,
        m.trials_of("kit1").len(),
    )?;
    writeln!(
        out,
        "every clip is at -23 LUFS; trials lowered as a whole to keep a clip under -1 dBTP: {lowered} of {} (the key lists them)",
        owner.len()
    )
}

/// Load the key in `dir`, or design it from `--set`.
fn prepare(args: &Gr3Args, dir: &Path, out: &mut dyn Write) -> Res<(Manifest, Option<Set>)> {
    std::fs::create_dir_all(dir)?;
    if let Some(m) = load_manifest(dir)? {
        writeln!(
            out,
            "using the design in {} ({} trials for the owner, {} kit(s), seed {})",
            dir.display(),
            m.trials_of(OWNER).len(),
            m.kits,
            m.seed
        )?;
        if let Some(f) = args.friends.filter(|f| *f != m.kits) {
            return Err(format!(
                "{} was designed with {} kit(s), not {f}; use a new --dir",
                dir.display(),
                m.kits
            )
            .into());
        }
        if let Some(seed) = args.seed.filter(|s| *s != m.seed) {
            return Err(format!(
                "{} was designed with seed {}, not {seed}; use a new --dir",
                dir.display(),
                m.seed
            )
            .into());
        }
        let set = match &args.set {
            Some(p) => {
                let s = Set::load(p)?;
                check_inputs(&m, &s)?;
                Some(s)
            }
            None => None,
        };
        return Ok((m, set));
    }
    let Some(p) = &args.set else {
        return Err(format!(
            "no design in {}; give the set manifest: `enth-listen gr3 --set <set.toml> --dir {}`",
            dir.display(),
            dir.display()
        )
        .into());
    };
    let set = Set::load(p)?;
    writeln!(
        out,
        "designing from {} ({} references) ...",
        p.display(),
        set.items.len()
    )?;
    let m = design(
        &set,
        args.seed.unwrap_or(DEFAULT_SEED),
        args.friends.unwrap_or(2),
        dir,
    )?;
    print_design(out, &m)?;
    Ok((m, Some(set)))
}

/// Write the kits `root/kit1`, `root/kit2`, ... : each trial's three clips, `answers.txt` and `README.txt`.
fn export_kits(m: &Manifest, set: Option<Set>, root: &Path, out: &mut dyn Write) -> Res<()> {
    if m.kits == 0 {
        return Err("this design has no kits (it was made with --friends 0)".into());
    }
    let set = if let Some(s) = set {
        s
    } else {
        let s = Set::load(Path::new(&m.set_path))?;
        check_inputs(m, &s)?;
        s
    };
    for k in 0..m.kits {
        let name = kit_name(k);
        let kd = root.join(&name);
        std::fs::create_dir_all(&kd)?;
        let trials = m.trials_of(&name);
        for t in &trials {
            write_trial_clips(&set, t, &kd)?;
        }
        std::fs::write(
            kd.join("answers.txt"),
            kit_answers_text(&name, &trials, None),
        )?;
        std::fs::write(kd.join("README.txt"), kit_readme(&name, &trials))?;
        writeln!(
            out,
            "wrote {}: {} trials ({} WAV files), about {:.0} minutes",
            kd.display(),
            trials.len(),
            trials.len() * 3,
            estimate_minutes(m, &name)
        )?;
    }
    writeln!(
        out,
        "Send each kit folder (zipped) to one friend, and nothing else: never manifest.json or the --dir folder.\nWhen a friend returns answers.txt, save it as <name>.txt in the friends folder of --dir; `--analyze` reads it."
    )?;
    Ok(())
}

/// The owner's answers and every friend's file in `dir`.
fn collect_responses(dir: &Path, m: &Manifest) -> Res<Vec<Responses>> {
    let mut v = Vec::new();
    if let Some(f) = load_answers(&answers_path(dir), SCHEMA_ANSWERS, 1)? {
        v.push(owner_responses(m, &f)?);
    }
    let fd = friends_dir(dir);
    if fd.is_dir() {
        let mut files: Vec<PathBuf> = std::fs::read_dir(&fd)?
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("txt")))
            .collect();
        files.sort();
        for p in files {
            let stem = p
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("friend")
                .to_string();
            let text = std::fs::read_to_string(&p).map_err(|e| format!("{}: {e}", p.display()))?;
            v.push(parse_kit_answers(&stem, &text, m)?);
        }
    }
    Ok(v)
}

/// Analyze `dir`'s answers, print the report and write `gr3-report.json`.
fn analyze_dir(args: &Gr3Args, dir: &Path, out: &mut dyn Write) -> Res<Report> {
    let m = load_manifest(dir)?.ok_or_else(|| {
        format!(
            "no design in {}; there is nothing to analyze yet",
            dir.display()
        )
    })?;
    let responses = collect_responses(dir, &m)?;
    if responses.is_empty() {
        return Err(format!(
            "no answers in {} yet: the owner's session writes answers.json, and each friend's file goes in {}",
            dir.display(),
            friends_dir(dir).display()
        )
        .into());
    }
    let report = analyze(&m, &responses, args.bootstrap);
    if !report.complete && !args.partial {
        write!(out, "{}", report.progress_text())?;
        return Err("the listening key stays closed until every listener is done; run again with --partial to look anyway (for example when a friend will not answer)".into());
    }
    write!(out, "{}", report.to_text())?;
    write_json(&report_path(dir), &report)?;
    writeln!(
        out,
        "wrote {} ({SCHEMA_REPORT})",
        report_path(dir).display()
    )?;
    Ok(report)
}

/// `--dry-run`: the whole pipeline, synthetic.
fn dry_run(args: &Gr3Args, dir: &Path, out: &mut dyn Write) -> Res<()> {
    std::fs::create_dir_all(dir)?;
    for sub in ["clips", "kits", "friends"] {
        let p = dir.join(sub);
        if p.is_dir() {
            std::fs::remove_dir_all(&p)?;
        }
    }
    let set = match &args.set {
        Some(p) => Set::load(p)?,
        None => Set::load(&demo_set(&dir.join("demo-set"), 60)?)?,
    };
    let seed = args.seed.unwrap_or(DEFAULT_SEED);
    writeln!(
        out,
        "dry run: nothing is played; the clips are measured and written, the answers are synthetic"
    )?;
    let m = design(&set, seed, args.friends.unwrap_or(2), dir)?;
    print_design(out, &m)?;
    let mut f = AnswerFile::new(SCHEMA_ANSWERS, 1, true);
    for (t, c) in m
        .trials_of(OWNER)
        .iter()
        .zip(synthetic_choices(&m, OWNER, seed).values())
    {
        f.answers.push(Answer {
            n: t.n,
            id: t.id.clone(),
            choice: c.to_string(),
            replays: 0,
            ms: 0,
        });
    }
    write_json(&answers_path(dir), &f)?;
    if m.kits > 0 {
        export_kits(&m, Some(set), &dir.join("kits"), out)?;
        std::fs::create_dir_all(friends_dir(dir))?;
        for k in 0..m.kits {
            let name = kit_name(k);
            let choices = synthetic_choices(&m, &name, seed);
            let text = kit_answers_text(&name, &m.trials_of(&name), Some(&choices));
            std::fs::write(friends_dir(dir).join(format!("{name}.txt")), text)?;
        }
    }
    writeln!(
        out,
        "\nSYNTHETIC answers: the owner and each kit are generated; kit2 misses two attention trials on purpose, to show the exclusion.\n"
    )?;
    analyze_dir(args, dir, out)?;
    Ok(())
}

/// Run `enth-listen gr3`. `player` replaces the command player (tests); `lines` is where answers are typed.
///
/// # Errors
/// A bad folder, set or answers file, a failing player, or a listener who has not finished (`--analyze`).
pub fn run(
    args: &Gr3Args,
    out: &mut dyn Write,
    lines: &mut dyn LineSource,
    player: Option<&mut dyn Player>,
) -> Res<()> {
    if args.dry_run {
        return dry_run(args, &args.dir.join("dry-run"), out);
    }
    let dir = &args.dir;
    if args.analyze {
        analyze_dir(args, dir, out)?;
        return Ok(());
    }
    let (m, set) = prepare(args, dir, out)?;
    if let Some(root) = &args.export_kit {
        return export_kits(&m, set, root, out);
    }
    let mut own;
    let player: &mut dyn Player = if let Some(p) = player {
        p
    } else {
        own = CommandPlayer::parse(&args.player)?;
        writeln!(out, "playing with: {}", own.command_line())?;
        &mut own
    };
    if run_session(dir, &m, player, lines, out)? == SessionEnd::Finished {
        writeln!(
            out,
            "Next: `enth-listen gr3 --dir {d} --export-kit <folder>` for the friends, and `enth-listen gr3 --dir {d} --analyze` when everyone is done.",
            d = dir.display()
        )?;
    }
    Ok(())
}
