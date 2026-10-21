//! `cargo xtask agent-eval`: the agent-eval harness (P1a lane E; protocol in docs/agent-cli.md and docs/p1a-g1.md).
//!
//! An agent (headless Claude Code) designs sounds with the `enth` CLI, one empty temp dir per trial, with only a
//! budget-counting `enth` wrapper as its tool. A judge then forces the goal's performance and its expectation list on
//! the final patch. Two tracks: the designer's list is given, or the agent writes its own from the brief. Three
//! trials per goal; the final patches are rendered to WAV for a blind owner check. Never a merge gate: a real run
//! needs the owner's approval of the spend, and `--dry-run` replaces the model with a scripted fake agent.
//!
//! Layout: `goal` (the set on disk), `wrap` (the wrapper), `stream` (Claude Code transcripts), `fake` (the dry-run
//! agents), `run` (trials), `report` (summaries), `blind` (the owner's blind check), `audit` (the set's honesty
//! checks), `util`.

mod audit;
mod blind;
mod fake;
mod goal;
mod lesson_fake;
mod lesson_track;
mod report;
mod run;
mod stream;
mod util;
mod wrap;

use std::path::{Path, PathBuf};

use run::{Auth, Opts, Track};

const USAGE: &str = "usage: cargo xtask agent-eval [run] [options]
  --dry-run               scripted fake agent, no model call (the harness end to end; default agent: witness)
  --agent witness|follower  the fake agent (witness: hand-picked solutions; follower: applies check's fix.ops)
  --track given|self|both   (default both)           --goals id,id,...   (default all)
  --trials N                (default 3)              --model ID          (default claude-sonnet-5-5)
  --max-usd N               (default 20: stop starting trials once the estimated cost reaches it)
  --jobs N                  (default 1)              --timeout-min N     (default 15, per trial)
  --auth auto|api-key|subscription                   (auto: api-key with --bare if ANTHROPIC_API_KEY is set)
  --enth PATH               (default: cargo build --release -p enth-cli)
  --out DIR                 (default target/agent-eval/<stamp>)
  --confirm-spend           required for a real run: the owner approved the spend
  --keep-tmp --no-blind --no-save-summary --seed N
  agent-eval report <dir>   rebuild summary.json and the tables from a run directory
  agent-eval listen <dir> [--player CMD] [--dry-run]
                            the owner's blind check (plays audio only when started by the owner; --dry-run plays nothing)
  agent-eval goals          list the goals and check their witnesses exist
  agent-eval lesson [--dry-run] [--agent witness|follower] [...]
                            the agent lesson track (lane EV3): whole lessons of the basics pack; `lesson --help`
  agent-eval audit [--goals id,...] [--cross] [--enth PATH]
                            honesty checks, no model: each witness passes both lists with no thin item, and the
                            untouched starting template fails the designer list (--cross: lists on other witnesses)";

/// The eval set under the workspace root.
pub fn load_set(root: &Path) -> Result<goal::EvalSet, String> {
    goal::load(&root.join("content/agent-evals"))
}

pub fn agent_eval(args: &[String]) -> Result<(), String> {
    match args.first().map(String::as_str) {
        Some("wrap") => {
            let code = wrap::main(&args[1..])?;
            #[allow(
                clippy::exit,
                reason = "the wrapper must exit with the wrapped enth call's own code"
            )]
            std::process::exit(code)
        }
        Some("report") => {
            let dir = args.get(1).ok_or(USAGE)?;
            let meta = super::metadata()?;
            report::report(Path::new(dir), &load_set(&meta.workspace_root)?)
        }
        Some("listen") => listen(&args[1..]),
        Some("goals") => goals(),
        Some("audit") => audit::audit(&args[1..]),
        Some("lesson") => lesson_track::lesson(&args[1..]),
        Some("--help" | "-h" | "help") => {
            println!("{USAGE}");
            Ok(())
        }
        Some("run") => run::run(&parse(&args[1..])?),
        _ => run::run(&parse(args)?),
    }
}

fn parse(args: &[String]) -> Result<Opts, String> {
    let mut o = Opts::default();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut value = |name: &str| -> Result<String, String> {
            it.next()
                .cloned()
                .ok_or_else(|| format!("{name} needs a value\n{USAGE}"))
        };
        match a.as_str() {
            "--dry-run" => o.dry_run = true,
            "--agent" => o.fake = fake::Kind::parse(&value("--agent")?)?,
            "--runner" => {
                let r = value("--runner")?;
                if r != "claude-code" {
                    return Err(format!(
                        "--runner `{r}`: only claude-code exists (a direct Messages API runner is later)"
                    ));
                }
            }
            "--track" => {
                o.tracks = match value("--track")?.as_str() {
                    "given" => vec![Track::Given],
                    "self" => vec![Track::SelfWritten],
                    "both" => vec![Track::Given, Track::SelfWritten],
                    other => {
                        return Err(format!(
                            "--track must be given, self or both, not `{other}`"
                        ));
                    }
                };
            }
            "--goals" => {
                o.goals = Some(
                    value("--goals")?
                        .split(',')
                        .map(|s| s.trim().to_owned())
                        .filter(|s| !s.is_empty())
                        .collect(),
                );
            }
            "--trials" => o.trials = number(&value("--trials")?, "--trials")?,
            "--model" => o.model = value("--model")?,
            "--max-usd" => o.max_usd = number(&value("--max-usd")?, "--max-usd")?,
            "--jobs" => o.jobs = number(&value("--jobs")?, "--jobs")?,
            "--timeout-min" => o.timeout_min = number(&value("--timeout-min")?, "--timeout-min")?,
            "--seed" => o.seed = number(&value("--seed")?, "--seed")?,
            "--enth" => o.enth = Some(PathBuf::from(value("--enth")?)),
            "--out" => o.out = Some(PathBuf::from(value("--out")?)),
            "--auth" => {
                o.auth = match value("--auth")?.as_str() {
                    "auto" => Auth::Auto,
                    "api-key" => Auth::ApiKey,
                    "subscription" => Auth::Subscription,
                    other => {
                        return Err(format!(
                            "--auth must be auto, api-key or subscription, not `{other}`"
                        ));
                    }
                };
            }
            "--confirm-spend" => o.confirm_spend = true,
            "--keep-tmp" => o.keep_tmp = true,
            "--no-blind" => o.blind = false,
            "--no-save-summary" => o.save_summary = false,
            "--save-floor" => o.save_floor = true,
            _ => return Err(format!("unknown argument `{a}`\n{USAGE}")),
        }
    }
    if o.trials == 0 {
        return Err("--trials must be at least 1".to_owned());
    }
    Ok(o)
}

fn number<T: std::str::FromStr>(text: &str, name: &str) -> Result<T, String> {
    text.parse()
        .map_err(|_| format!("{name}: `{text}` is not a number"))
}

fn listen(args: &[String]) -> Result<(), String> {
    let dir = args.first().ok_or(USAGE)?;
    let (mut player, mut dry_run) = ("pw-play --media-role=Production".to_owned(), false);
    let mut it = args[1..].iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--player" => player = it.next().cloned().ok_or(USAGE)?,
            "--dry-run" => dry_run = true,
            _ => return Err(format!("unknown argument `{a}`\n{USAGE}")),
        }
    }
    let meta = super::metadata()?;
    blind::listen(Path::new(dir), &meta.workspace_root, &player, dry_run)
}

fn goals() -> Result<(), String> {
    let meta = super::metadata()?;
    let set = load_set(&meta.workspace_root)?;
    println!(
        "eval set version {} ({} goals)",
        set.meta.eval_version,
        set.goals.len()
    );
    println!(
        "{:<22} {:>1}  {:<26} {:<14} {:>5} {:>8}  list",
        "id", "d", "brief", "template", "items", "witness"
    );
    let mut missing = 0;
    for g in &set.goals {
        let witness = set.witness(&g.id);
        if witness.is_err() {
            missing += 1;
        }
        println!(
            "{:<22} {:>1}  {:<26} {:<14} {:>5} {:>8}  {}",
            g.id,
            g.difficulty,
            g.brief,
            g.template,
            g.expect.split(',').count(),
            if witness.is_ok() { "yes" } else { "MISSING" },
            g.perf.short().unwrap_or_else(|| g.perf.json()),
        );
    }
    if missing > 0 {
        return Err(format!("{missing} goal(s) have no usable witness"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set() -> goal::EvalSet {
        goal::load(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../content/agent-evals"))
            .expect("the shipped eval set loads")
    }

    fn strings(a: &[&str]) -> Vec<String> {
        a.iter().map(|&s| s.to_owned()).collect()
    }

    #[test]
    fn the_shipped_set_has_thirty_goals_covering_the_brief() {
        // Eval version 3 (R1/S1 lane EV2): 30 goals, from P1a's 10, across the sound families.
        let set = set();
        assert_eq!(set.goals.len(), 30);
        let tags: Vec<&str> = set
            .goals
            .iter()
            .flat_map(|g| g.tags.iter().map(String::as_str))
            .collect();
        for want in [
            "pluck",
            "pad",
            "bass",
            "lead",
            "bell",
            "brass",
            "sweep",
            "percussion",
            "kick",
            "hat",
            "snare",
            "tom",
            "keys",
            "organ",
            "fx",
            "texture",
            "reverb",
            "tremolo",
            "vibrato",
        ] {
            assert!(tags.contains(&want), "no goal is tagged `{want}`");
        }
        assert_eq!(set.goals.first().map(|g| g.difficulty), Some(1));
        assert_eq!(set.goals.last().map(|g| g.difficulty), Some(5));
        assert!(
            set.goals
                .windows(2)
                .all(|w| w[0].difficulty <= w[1].difficulty)
        );
        for g in &set.goals {
            assert_eq!(
                g.budget.renders, 20,
                "{}: the protocol's render budget is 20",
                g.id
            );
            assert!(
                g.expect.split(',').count() >= 5,
                "{}: a designer's list has several items",
                g.id
            );
        }
    }

    #[test]
    fn percussion_is_covered_by_short_sounds_with_measurable_holds() {
        // Never filter by duration: short sounds are goals, held long enough (0.25 s or more) that the
        // noise, decay and sustain readings exist.
        let set = set();
        let perc: Vec<_> = set
            .goals
            .iter()
            .filter(|g| g.tags.iter().any(|t| t == "percussion"))
            .collect();
        assert!(perc.len() >= 3);
        for g in perc {
            assert!(
                g.perf.notes[0].off_s >= 0.25,
                "{}: hold the hit long enough to measure it",
                g.id
            );
            assert!(g.perf.dur_s <= 1.5, "{}: a drum hit is a short clip", g.id);
        }
    }

    #[test]
    fn every_goal_has_a_witness_that_forces_the_performance_and_the_list() {
        let set = set();
        for g in &set.goals {
            let w = set.witness(&g.id).unwrap_or_else(|e| panic!("{e}"));
            let last = &w.step.last().expect("steps").args;
            assert!(
                last.iter().any(|a| a == "{perf}") && last.iter().any(|a| a == "{expect}"),
                "{}",
                g.id
            );
            assert_eq!(
                w.step[0].args[0], "new",
                "{}: a witness starts from a template",
                g.id
            );
        }
    }

    #[test]
    fn prompts_fill_completely_for_every_goal() {
        let set = set();
        for g in &set.goals {
            for name in ["system", "given", "self"] {
                let text = run::render_prompt(&set.prompt(name).unwrap(), g);
                for key in [
                    "brief",
                    "perf_json",
                    "perf_short",
                    "expect",
                    "template",
                    "renders",
                    "patch",
                ] {
                    let placeholder = format!("{{{key}}}");
                    assert!(
                        !text.contains(&placeholder),
                        "{} {name}: {placeholder} left unfilled",
                        g.id
                    );
                }
                assert!(text.contains(&g.brief) || name == "system");
            }
            let given = run::render_prompt(&set.prompt("given").unwrap(), g);
            assert!(
                given.contains(&g.expect),
                "{}: the given prompt shows the list",
                g.id
            );
            let own = run::render_prompt(&set.prompt("self").unwrap(), g);
            assert!(
                !own.contains(&g.expect),
                "{}: the self prompt must not leak the designer's list",
                g.id
            );
            assert!(!own.contains(&g.rationale));
        }
    }

    #[test]
    fn flags_parse_and_reject_typos() {
        let o = parse(&strings(&[
            "--dry-run",
            "--track",
            "given",
            "--goals",
            "a, b",
            "--trials",
            "2",
            "--jobs",
            "3",
        ]))
        .unwrap();
        assert!(o.dry_run);
        assert_eq!(o.tracks, vec![Track::Given]);
        assert_eq!(o.goals, Some(strings(&["a", "b"])));
        assert_eq!((o.trials, o.jobs), (2, 3));
        assert_eq!(parse(&[]).unwrap().tracks.len(), 2);
        assert!(parse(&strings(&["--trials", "0"])).is_err());
        assert!(parse(&strings(&["--tracks", "given"])).is_err());
        assert!(parse(&strings(&["--runner", "api"])).is_err());
    }

    #[test]
    fn a_real_run_needs_the_spend_to_be_confirmed() {
        let err = run::run(&parse(&[]).unwrap())
            .expect_err("a real run without --confirm-spend is refused");
        assert!(err.contains("--confirm-spend"));
    }
}
