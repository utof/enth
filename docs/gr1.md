# GR1: the kit that turns played rounds into numbers

- Date: 2026-10-20 (R1/S1 lane GK). Round-log schema `enth.round-log/1`, report schema `enth.gr1-report/1`.
- What it is: the protocol for gate GR1 (the owner and friends play 20 rounds each, mostly agree with the score and want more; the agent's solve rate is the baseline), the round-log format the report reads, and how to read what `cargo xtask gr1-report` prints. The gate is in [phase-2.md](phase-2.md); the game is in [session.md](session.md) and [challenge.md](challenge.md).
- People: the friends follow [gr1-friends.md](gr1-friends.md) (installing is [friends-install.md](friends-install.md)); the owner follows [gr1-owner.md](gr1-owner.md).
- Code: `xtask/src/gr1_report.rs` and its submodules; fixtures in `xtask/fixtures/gr1/`.
- Not here: the writer (`enth-session`'s `RoundLogEntry`, lane SG, and the file path, lane SKR in [plugin.md](plugin.md#the-recreate-round-lane-skr)), the agent eval itself (`cargo xtask agent-eval`, [agent-cli.md](agent-cli.md#evaluating-the-cli-like-a-product)), and the decision. The report prints what the logs say against a **proposed** reading; the owner decides GR1.

Everything is local. The report reads files you give it, opens no network connection and runs no paid eval.

## The protocol

| Part | What happens |
| --- | --- |
| Who | The owner and at least two friends. One player is the fallback when no friend can play, and the report says so. |
| How many | 20 rounds each: 4 sets of 5. A set is one round per level, 1 to 5, in any order, so four sets give 4 rounds per level. A round is 2 to 5 minutes, so a set is 10 to 25 minutes and a sitting is one or two sets. |
| Which rounds | Preferably **the same 20 codes for everyone**: the owner makes the list once (`enth challenge new --level N --seed S`, 4 seeds per level) and gives it out, so the humans, the agent and the floors all play the same targets. Without a list, each player plays what the build deals at each level, and the owner takes the codes from the logs (`gr1-report --codes`) for the agent run. Either way the report shows the humans twice, on all their rounds and on the baseline's codes only, so a partial agent run is still comparable. |
| What counts as a round | A round the player tried: at least one scored submit, or a reveal. A round ended with no submit is "skipped" and does not count towards the 20. |
| After each score | The player votes: the score felt too low, about right or too high. Before the first score shows, the player rates their own attempt 1 to 5, blind. |
| After each round | Ending it by starting another round is the "another round?" yes. Ending it any other way is a no. |
| Disputes | A score that is plainly wrong gets a dispute with a line of text. The report lists every one. |
| End of the last sitting | An optional "want more?" (yes, maybe, no), typed by the owner into the survey file. |
| The agent | The agent plays the humans' codes three times each (report 02, section 2.10). Its results go in a baseline file next to two floors: the untouched start patch (no-op) and a CMA-ES search over every parameter (`enth match`, [dsp/match.md](dsp/match.md)). |
| The decision | The owner reads the report against the proposed reading below, and the disputes in full. |

## The round log, schema v1

The plugin appends one JSON object per line to `round-log.jsonl` as rounds end. The path is `$ENTH_ROUNDS_DIR/round-log.jsonl` when that variable is set, else `<Documents>/enth/Rounds/round-log.jsonl` ([plugin.md](plugin.md#the-recreate-round-lane-skr) is the authority). A round writes an `end` line when it ends and a `dispute` line when the player disputes its score; nothing is rewritten, and a file keeps growing across sittings.

### Top-level fields

The writer is `RoundLogEntry` (`crates/enth-session/src/game.rs`). "Needed" means the reader skips the line without it.

| Field | Type | Meaning | Needed |
| --- | --- | --- | --- |
| `schema` | string | `enth.round-log/1`. Another version is skipped with a warning. | yes |
| `event` | string | `end` or `dispute`. Another value is kept and counts for nothing. | yes |
| `round` | object | `{serial, nonce}`: rounds counted in this instance, and the shell's time at the start. Lines of one round share it. | yes |
| `code` | string | The challenge code the round started from, as typed or dealt. The level is read from it. | yes |
| `ended_by` | string or null | `restore` (End, own patch back), `keep` (End, round patch kept), `new_round` ("another round?" yes), `host` (the host loaded another state). Null on a dispute line. | no |
| `broken` | bool | The round could not continue. Such a round does not count. | no |
| `result` | object or null | The round's outcome, below. Null only when the code does not decode. | for `end` |
| `submits` | array | Every scored submit: `{at_ms, passed, sound, structure, combined}`, the three scores in hundredths of a percent (9125 is 91.25 %). | no |
| `dispute` | string or null | The player's note. | for `dispute` |
| `edits` | integer | Edits made in the round. | no |
| `patch` | string or null | The last scored submit's patch, as patch-file text. | no |
| `report` | object or null | The last scored submit's full `enth.score-report/1`, while the session still held it. | no |
| `fingerprint` | string | The target's fingerprint, 64 hex digits (added at the F1r review). | no |
| `engine` | string | The build's engine version, `x.y.z` (added at the F1r review). | no |
| `edit_log` | array | On `end` lines: `{t_ms, source, ops}` for every change to the round's patch, with its active time (added at the F1r review). | no |

### The result

`result` is `RoundResult` (`enth.round-result/1`, [challenge.md](challenge.md)).

| Field | Type | Meaning |
| --- | --- | --- |
| `schema`, `code` | string | `enth.round-result/1`; the code again (a mismatch with the top-level `code` is a warning). |
| `outcome` | string | `passed`, `not_passed` or `gave_up` (the target was revealed before a pass). |
| `first_submit` | object or null | The **recorded score**: the first submit's report (`sound`, `structure`, `combined`, each `{scorer, percent, ...}`; `pass_mark`, `passed`, `engine`). Null when no submit was made. |
| `submits` | integer | How many times the player submitted. |
| `time_to_pass_ms` | integer or null | Active (editor-visible) time from the start to the first passing submit; null without a pass. |
| `active_ms` | integer | Active time to the end of the round. |
| `assists` | object | The help used, below. A round is **assisted** when any of it was used. |
| `agreement` | string or null | The vote: `too_low`, `about_right`, `too_high`. |
| `rating` | integer or null | The blind rating, 1 ("nothing like it") to 5 ("can't tell them apart"), given before the first score showed (added at the F1r review). |

| `assists` field | Type | Meaning |
| --- | --- | --- |
| `hints` | integer | Hints asked for. |
| `live_meter` | bool | The live meter was on. |
| `revealed` | bool | The target was revealed (`gave_up` when that came before a pass). |
| `dim_unused` | bool | The `dim_unused` assist (the modules the target leaves alone dimmed) was on while the round could be played (lane HINT, FU1). Written only when true: a line without it reads as false. |

### How the reader treats a line

The reader works on plain JSON, never on a typed struct, so a line from a newer or an older build does not fail the report and every difference is named.

| Severity | When | What happens |
| --- | --- | --- |
| error | not JSON, not an object, or no `schema`, `event`, `round` or `code` | the line is skipped and counted |
| warning | a field of the wrong type or out of range (a rating of 9, a percentage over 100), two fields that disagree (a `passed` outcome without a time to pass), a `dispute` line without a note, a schema version this reader does not know | the line is read and the bad field dropped; a line of another schema version is skipped |
| note | an unknown field or an unknown enum value (an `agreement` this build does not know), a field an older build did not write (`rating`, `fingerprint`, `engine`, `edit_log`) | the line is read; an unknown value counts as "unknown", never as a guess |

`cargo xtask gr1-report --validate <logs>` prints only these findings. It exits 1 on an error or on possible personal data.

### Restored rounds

When the host loads a state while a round is live, the round ends with `ended_by: host`; the host can then restore the saved round, and when it ends again it writes a second `end` line with the same `round`. Both lines are in the file. **The report keeps the last `end` line of each round** per player, and says how many rounds were restored. The same rule makes a file that was sent twice, or two files that overlap, harmless. Lines of different players never merge.

### Privacy

The log holds the code, the patch the player built, the scores, times, the vote, the rating and any dispute note. It holds no name, e-mail address or file path, and the plugin sends nothing anywhere. The report still scans every line for an e-mail address or a home-folder path, prints only where it found one (never the text), and `--strict` or `--validate` fails on a finding. A dispute note is free text the player typed, so the friends' sheet says to leave names out.

## The baseline file

One JSON object per attempt, schema `enth.gr1-baseline/1`. The report reads these files (`--baseline FILE`, repeatable); it never runs an agent.

| Field | Type | Meaning | Needed |
| --- | --- | --- | --- |
| `schema` | string | `enth.gr1-baseline/1` | yes |
| `arm` | string | `noop` and `cmaes` are the two floors; any other label (`agent`, `agent:sonnet-5-5`) is an agent and is printed as written. | yes |
| `code` | string | The challenge code played; the level is read from it. | yes |
| `attempt` | integer | 1, 2, 3 for the agent's three tries. | no |
| `passed` | bool | The final patch passed. | `passed`, or `sound` with `pass_mark` |
| `sound`, `structure`, `combined` | number | The final patch's scores, percent. `sound` is what the pass is judged on. | no |
| `pass_mark` | number | The pass mark in percent (90 by default in the game). | with `sound` |
| `renders`, `tokens`, `cost_usd`, `wall_s` | number | What the attempt spent. `cost_usd` is an estimate. | no |
| `model`, `note` | string | Free text. | no |

Rates are pooled single attempts (pass@1): passes over attempts. The humans' column is rounds passed over rounds played; its median sound % is each round's best submit, and an arm's is its final patch's.

## The survey file

One line per player, schema `enth.gr1-survey/1`, typed by the owner when the logs arrive (`--survey FILE`). `player` is the anonymous id the report uses for that player's log.

| Field | Type | Meaning | Needed |
| --- | --- | --- | --- |
| `schema` | string | `enth.gr1-survey/1` | yes |
| `player` | string | The anonymous id | yes |
| `want_more` | string | `yes`, `maybe` or `no` | yes |
| `os`, `daw`, `note` | string | Context. Keep the note short and free of names. | no |

## The report

`cargo xtask gr1-report <logs...> [--baseline FILE]... [--survey FILE] [--json]`. A log argument is a file, `id=file` to name its player, or a folder (every `*.jsonl` in it). A player is the file name without `.jsonl` unless named, so name files `p1.jsonl`, `p2.jsonl` before collecting them. `--player-id ID` merges every unnamed log into one player. `--codes` lists the codes the players played (code, level, how many players) for the agent run. `--rounds N`, `--bootstrap N` and `--seed N` change the defaults below. The same logs always give the same numbers: the bootstrap is seeded.

The top of a report, from the fixtures (`xtask/fixtures/gr1/report.txt`; a test keeps this block equal to the file):

```text
GR1 report (enth.gr1-report/1)
Read 3 round log file(s): 61 lines, 61 read into rounds (0 unusable, 0 from another schema version); 0 error(s), 0 warning(s), 40 note(s).
Privacy lint: clean (no email address or home-folder path found)

Gate indicators (the proposed reading, docs/gr1.md; you decide GR1)
  [  met   ] You and your friends play
             reading: at least 2 players (1 is the owner-only fallback)
             logs:    3 player(s)
  [NOT MET ] 20 rounds each, logged
             reading: every player has at least 20 played rounds
             logs:    short: p3 17/20
  [  met   ] Mostly agree with the score
             reading: at least 70 % "about right" overall and no rater under 50 % (a rater has 10 votes or more)
             logs:    76 % about right over 45 votes
  [  met   ] Want more
             reading: "another round?" answered yes on at least 60 % of rounds
             logs:    81 % (47 of 58 rounds); end-of-session: 2 of 3 said yes
  [  met   ] The agent's solve rate is recorded as a baseline
             reading: an agent arm beside the no-op and CMA-ES floors
             logs:    agent: pass@1 60 % over 60 attempts on 20 codes; floors present: 2 of 2
```

### What each section answers

| Section | Question | How it is computed |
| --- | --- | --- |
| Players | How many rounds did each person play, and how did it go? | `rounds` is distinct rounds after the dedupe; `played` excludes skipped and broken rounds; `restored` counts rounds with a second `end` line; `passed` is an outcome of `passed`; `pass` is passed over played; `assisted` is the share of played rounds with a hint, the live meter, a reveal or the dim assist. |
| Pass rate by level | Is each level winnable? | Level comes from the code (preset 1 to 5). The 95 % Wilson interval treats rounds as independent trials, so it is a rough guide. Also shown: unassisted passes over unassisted rounds, median time to pass, median sound % of the recorded first submit and of each round's best submit. Per player and level below it. |
| Time to pass and assists | How long, and how much help? | Median active time to the first passing submit (m:ss), over passes, with and without assists; hints, reveals, live-meter rounds, and how many passes were assisted. |
| Agreement | Do they agree with the score? | The vote after each scored round: counts and shares, response rate (votes over scored rounds), median sound % per vote, and "about right" per player. A missing vote is not disagreement; unknown values are counted apart. |
| Blind rating | Does the score follow what ears hear? | Spearman rank correlation (ties share their average rank) between the rating and the recorded first submit's sound, structure and combined %, with a 95 % percentile bootstrap over rounds; the rounds per rating; one row per player with at least 8 rated rounds. No estimate when a variable is constant or there are fewer than 3 pairs. |
| Want more | Do they want another round? | Share of ended rounds with `ended_by: new_round`; by player; the survey answers beside it. |
| Disputes | What did they say was wrong? | Every `dispute` line, once, with the code, level, the round's first scores, the outcome, the vote and the note. Read all of them. |
| Agent baseline | Where does the agent stand? | Per arm and level: pass@1 with Wilson interval, median sound %, renders, tokens and cost, beside the humans on all rounds and on the baseline's codes. |
| Checks | Is the data comparable? | Distinct codes and how many everyone played; scorer ids and engine versions seen (mixed ones flagged); a code with two target fingerprints; hand-set (custom) codes; rounds whose level could not be read. |
| Problems | What did the reader notice? | The findings above, grouped, with the first place of each (`file:line`, never a path). |

### The proposed reading

Both thresholds come from research report 05 and [phase-2.md](phase-2.md) and are a **proposal until the data settle them**. The report prints them and the numbers; it never says "GR1 passed". They are options (`Thresholds` in `numbers.rs`), not constants in the text.

| Gate | Reading | Why this way |
| --- | --- | --- |
| Play | 20 played rounds for each of at least 2 players | "You and your friends play 20 rounds each" |
| Mostly agree | "About right" on at least 70 % of the votes overall, and no rater (10 votes or more) under 50 % | One rater's strong disagreement is not averaged away. Skipped votes are not counted either way, so the response rate is printed beside it. |
| Want more | "Another round?" yes on at least 60 % of rounds | The count is the rounds ended by starting another one. |
| Agent | An agent arm with both floors present | GR1 records the agent as a baseline; it does not set a bar. |

Caveats to keep in mind:

- **Want more reads low.** The last round of every sitting ends by closing or ending, not by starting another, so each sitting costs about one "yes". The survey answer is the plain check.
- **Agreement and difficulty.** A friend who loses a level 5 round may vote "too high" because it was hard. Read the agreement shares beside the pass rates and the per-rater rows.
- **The blind rating has a few dozen points.** Read the interval, not the estimate. A rho near zero with a wide interval says nothing; a rho above about 0.6 with an interval clear of 0.3 says the score tracks what players hear.
- **Rounds are not independent**: the same person and the same codes repeat, so every interval is a rough guide, as research report 02 notes for 20 codes (10 of 20 is [30 %, 70 %]).

## Making the baseline lines

Nothing here runs by itself and nothing in the report spends money. The owner makes the files once the humans' codes are known (`gr1-report --codes <logs>`, or the list the owner gave out).

| Arm | How | Line |
| --- | --- | --- |
| `agent` | Play each code three times with the agent's `enth` wrapper (`cargo xtask agent-eval` once it has the target track, [agent-evals.md](agent-evals.md#reference-targets-not-built); or any script that plays the codes through the CLI). For each attempt, score the final patch with `enth compare <patch> --code <code>`. | `{"schema":"enth.gr1-baseline/1","arm":"agent","code":"...","attempt":1,"sound":91.4,"pass_mark":90,"renders":57,"tokens":60901,"cost_usd":0.09}` |
| `noop` | Score the round's untouched start patch against each code (`enth compare <start patch> --code <code>`). One attempt per code. | `{"schema":"enth.gr1-baseline/1","arm":"noop","code":"...","attempt":1,"sound":12.2,"pass_mark":90}` |
| `cmaes` | `enth match` over every parameter on each code with the same render budget as the agent, then `enth compare` the result. One attempt per code (or three with different seeds). | `{"schema":"enth.gr1-baseline/1","arm":"cmaes","code":"...","attempt":1,"sound":93.9,"pass_mark":90,"renders":1500}` |

An agent that played the same codes through the game and left a round log is turned into baseline lines by hand or by a short script (`arm`, `code`, `passed`, the best `sound` from `submits`); the report does not read it as a human log. The commands above are lane CL's ([agent-cli.md](agent-cli.md)); check them there before a run.

## Fixtures and tests

`xtask/fixtures/gr1/` holds made-up data (no real person): three players' logs (`logs/p1.jsonl` to `p3.jsonl`), an agent run, the two floors, a survey, and the report they give as text and JSON. The logs are written by a seeded generator in `xtask/src/gr1_report/fixtures.rs` (arithmetic only, so every platform writes the same bytes); the tests fail when the committed files differ from the generator, and `cargo test -p xtask gr1_report::fixtures::regenerate -- --ignored` rewrites them on purpose.

| What the fixtures contain | Why |
| --- | --- |
| `p1`: a current build, 20 rounds, a dispute | the normal case |
| `p2`: a newer build with unknown fields and values, a round restored by the host (two `end` lines) | tolerant reading, the dedupe |
| `p3`: an older build with no rating, fingerprint, engine or edit log; 17 rounds; a dispute | older logs, a player who is short |
| 20 codes, 4 per level; 8 rounds assisted for `p1` and `p2` | assisted share, the gate |

The tests pin the numbers (rounds per player 20, 21 and 17; 36 passes; 45 votes of which 34 "about right"; rho 0.82 for the rating against sound) and cover the dedupe, the tolerant reader, the privacy lint, the statistics (against hand values and an independent Python check), the gate statuses and the command line.

## Open

- The thresholds above are a proposal (phase-2.md says the same).
- The labels of the vote, dispute and "another round?" controls belong to lane RUI; the sheets describe what each does, not the button text.
- The baseline lines depend on lane CL's `enth challenge` and `enth compare --code`, and on `agent-eval` growing a target track (lane EV2); until then the owner makes the lines by script.
