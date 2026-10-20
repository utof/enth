# Running GR1 (for the owner)

The gate: you and your friends play 20 rounds each, mostly agree with the score and want more, and the agent's solve rate is recorded as the baseline ([phase-2.md](phase-2.md)). This page is the checklist; [gr1.md](gr1.md) says what each number means and what the files hold.

## 1. Before anyone plays

1. Give each friend the friend build ([friends-install.md](friends-install.md)) and the sheet [gr1-friends.md](gr1-friends.md). Tell them their id (`p2`, `p3`, ...; yours is `p1`). Ids are anonymous on purpose.
2. Optional, better: a fixed list of 20 codes, 4 per level, so everyone and the agent play the same targets. Make it with `enth challenge new --level N --seed S` (four seeds per level; the command prints the code) and send the list with the sheet. Without a list the report still works; step 4 takes the codes from the logs.
3. Play your own 20 rounds the same way. Your own log is the owner-only fallback if nobody else can.

## 2. Collect the logs

Ask each friend for `round-log.jsonl` (the sheet says where it is). Save each one in a folder outside git, named by the id:

```
_local/gr1/logs/p1.jsonl
_local/gr1/logs/p2.jsonl
_local/gr1/logs/p3.jsonl
```

(`_local/` is ignored by git.) The name is the player's id in the report. If one person sends two files, use `p2=first.jsonl p2=second.jsonl` on the command line instead and the report merges them. A round that appears twice, or a file sent twice, counts once.

Type their end-of-session answers into `_local/gr1/survey.jsonl`, one line each:

```
{"schema":"enth.gr1-survey/1","player":"p2","want_more":"yes","os":"windows","daw":"bitwig","note":"..."}
```

## 3. Check the files

```
cargo xtask gr1-report _local/gr1/logs --validate
```

Fix what it names. Errors skip a line; warnings drop a field; notes (a newer build's extra field) are fine. It also looks for an e-mail address or a home-folder path and exits 1 if it finds one: read the place it names, remove the text from the file, and tell the friend. Do not commit friends' logs.

## 4. Get the agent baseline

This is the only step that costs money, and nothing in the report runs it.

```
cargo xtask gr1-report _local/gr1/logs --codes
```

lists the codes the humans played (code, level, how many players). Run the agent three times on each, and the two floors once ([gr1.md](gr1.md#making-the-baseline-lines) has the recipe and the line format), and write one `enth.gr1-baseline/1` line per attempt into `_local/gr1/agent.jsonl` and `_local/gr1/floors.jsonl`. The report shows the humans on all their rounds and on the baseline's codes only, so a partial run is still comparable.

## 5. Read the report

```
cargo xtask gr1-report _local/gr1/logs --baseline _local/gr1/agent.jsonl --baseline _local/gr1/floors.jsonl --survey _local/gr1/survey.jsonl --strict
```

Add `--json` for the machine-readable form. Read it in this order:

| Look at | Decide |
| --- | --- |
| Gate indicators | Which of the five lines say `NOT MET`, and why (the `logs:` line names the player or number). |
| Players | Who is short of 20 played rounds. Ask them for another set rather than reading around it. |
| Agreement | The share of "about right", the response rate, and any rater under 50 %. |
| Pass rate by level | A level nobody passes, or everybody passes, is a calibration problem, not a verdict on the players. |
| Disputes | Read every note. Two people disputing the same code is a scorer bug until shown otherwise. |
| Blind rating | The rho and its interval. |
| Agent baseline | The agent beside the humans and the two floors. |
| Checks and Problems | Mixed scorers or engines, a code with two targets: the data is not comparable until explained. |

The thresholds are a proposal (70 % "about right" overall and no rater under 50 %, "another round?" on 60 % of rounds). The gate indicators show which side of them the data falls on; whether that is GR1 is your call, and the disputes are as important as the numbers.

## 6. Keep the record

Write down the date, the build ([friends-install.md](friends-install.md) says how a friend finds theirs), the report text and the JSON in `_local/gr1/`, and your decision. A report is repeatable: the same logs, baseline and survey always print the same numbers.
