# Owner sheet 4: R2 (Learn)

- Date: 2026-10-22 (R2 lane INT2). Every R2 lane is merged and every automated GR2 check is in [r2-gates.md](r2-gates.md). What is left needs you and your friends: your taste, owner build 4 on the real display, the friend round, and GR2's pre- and post-tests.
- **Order:** top to bottom. **Step 2, your pre-test, comes before you play any lesson**, so it measures you before the lessons do. Your new time is about **4½ to 5½ hours over one to two weeks**, most of it the tour itself (step 6), which you play like a friend would.
- Windows open on your real display from step 2 on. Before each sitting read P1b's [Before each sitting](p1b-owner.md#before-each-sitting): save your work, wait 10 s between windows, and stop and note the step if Xorg crashes.
- Keep notes in `~/code/enth-scratch/owner/r2-notes.md` (template [at the end](#how-to-report)), then give the file to an agent.

| Step | What | Time | Unblocks |
| --- | --- | --- | --- |
| 1 | [Install owner build 4 and the packs](#1-install-owner-build-4-and-the-packs-15-min-no-windows) | 15 min, no windows | everything below |
| 2 | [Your GR2 pre-test](#2-your-gr2-pre-test-25-min) | 25 min | GR2's "you pass an item you failed before" |
| 3 | [Taste check: lessons 1.1 and 2.1](#3-taste-check-lessons-11-and-21-2030-min) | 20–30 min | the content's tone, for every later edit |
| 4 | [Owner build 4: the Learn view](#4-owner-build-4-the-learn-view-4560-min) | 45–60 min | GR2's friend round; the real-display rows |
| 5 | [Decide GR2's criteria and the friend round; cut the friend build](#5-decide-gr2s-criteria-and-the-friend-round-cut-the-friend-build-20-min-plus-runner-time) | 20 min, plus 25–60 min of runner time | the friends' tours |
| 6 | [Your tour, mid-test, post-test and debrief](#6-your-tour-mid-test-post-test-and-debrief-253-h-in-45-sittings) | 2½–3 h in 4–5 sittings | GR2 |
| 7 | [Collect the files and run gr2-report](#7-collect-the-files-and-run-gr2-report-30-min) | 30 min | the GR2 decision |

**Still pending from earlier sheets** (not repeated here): [owner sheet 3](r1s1-owner.md) (R1 and S1: the friend survey, listening, owner build 3, the preset pick, the friend build, your 20 rounds, GR1), and P1b's [owner sheet](p1b-owner.md). If you have not cut a friend build yet, step 5 here does it once for both GR1 and GR2: the zip carries both friend sheets.

## 1. Install owner build 4 and the packs (15 min, no windows)

```bash
cd ~/code/enth && git pull
cargo xtask install --package enth-plugin --release        # ~/.clap and ~/.vst3; the basics pack is built in
cargo build --release -p enth-cli                          # for step 3's `enth lesson show`
cargo xtask pack --out ~/Documents/enth/Packs shaping space-and-dynamics studies
ls ~/Documents/enth/Packs                                  # shaping.enthpack  space-and-dynamics.enthpack  studies.enthpack
```

`pack` builds the three packs the plugin does not carry (about 0.1–0.2 MB each) into your Packs folder; the tour needs `shaping` and `space-and-dynamics`. Friends get the same three files in their zip (step 5).

## 2. Your GR2 pre-test (25 min)

Do this before any lesson. In REAPER, insert "CLAPi: enth (enth)" on a new track and arm it for the virtual keyboard ([Install](p1b-owner.md#install-about-10-minutes-no-windows)).

Your pre-test is **form A** (friends with an odd id take A, even ids B; you are `p1`). Play the six form A codes in [gr2-friends.md, "The tests"](gr2-friends.md#3-the-tests-only-if-you-picked-the-tour-with-tests) exactly as friends do: **Recreate**, **Paste a code**, **Start**; at most 3 minutes each by the round's timer; no Hint, no Reveal, no dimming; stop at a pass or at 3 minutes, then **End and restore my patch**. Each code counts once, the first time it is played: do not try them beforehand.

The rounds go to your round log (`~/Documents/enth/Rounds/round-log.jsonl`) like any game round, so nothing else is needed now. Start your answers file:

```bash
mkdir -p ~/code/enth/_local/gr2                             # _local is not in git; never commit anyone's files
cp ~/code/enth/content/gr2/answers-template.toml ~/code/enth/_local/gr2/p1.gr2.toml
# edit it: id = "p1", protocol = "full", pre_form = "A", and the [intake] block
```

## 3. Taste check: lessons 1.1 and 2.1 (20–30 min)

[content/lessons/TASTE-basics.md](../content/lessons/TASTE-basics.md) is the guide: Waveforms (an enum control drawn from a set) and Low-pass cutoff (a continuous control with a sound check). Read each as markdown (its `lesson.toml` lists the steps in order, each naming its `.md` text), or through the CLI:

```bash
cd ~/code/enth
for l in waveforms cutoff; do
  ./target/release/enth lesson show content/lessons/basics --lesson $l | python3 -c 'import json,sys
for s in json.load(sys.stdin)["lessons"][0]["steps"]: print("--- step", s["number"], s["kind"], "| hints:", " / ".join(s["hints"])); print(s["text"] or "")'
done | less                                                # every step's text and hints, in playing order
```

Judge them by report 01's rubric (TASTE-basics.md, "What to judge"): the contrast heard before it is explained, words for the sound rather than the circuit, a Try goal clear from its text alone, rounds far enough apart to hear, hints that are listening cues. Note what you would change; an agent applies it to the pack and re-runs `lesson-check --cross --solve`.

## 4. Owner build 4: the Learn view (45–60 min)

The plugin from step 1. Click **Learn** in the header.

| # | Check | What to do | Pass looks like | Time |
| --- | --- | --- | --- | --- |
| 4.1 | The browser | Look at **The tour** and **All lessons**; at the bottom, **Open packs folder** and **Get more lessons** | 13 tour lessons, none "not installed" (step 1's packs); four packs under All lessons; the folder opens in the file manager; the releases page opens in your browser (no download happens) | 5 min |
| 4.2 | Lesson 1, How a lesson works | **Start** it and play it through to **Finish** | Every step kind once: listen (▶ examples, A/B), read, try (a knob, checked live), a round (Target ▶, Mine ▶, Submit), a question; ✓ on each step; your own patch back after **End and restore my patch** | 7 min |
| 4.3 | Lesson 5, Low-pass cutoff: highlight and lock | Play it; on the Try step, try to turn a control the step locks (it shows a lock glyph) | The step's control is outlined; a locked control does not move and the status line says why; **Hint** names a direction, never a value; **Show me** turns the knob | 10 min |
| 4.4 | "How close am I?" | In a round of lesson 5, Submit a patch that does not pass, then **How close am I?** | A progress bar, then "N moves away" in words and rings on the cards; ■ Stop works; **Try a new one** deals a sibling round | 5 min |
| 4.5 | Hide the pane | **Hide** in a lesson's Listen or Try step, then the header's **Lesson n/m** | The full editor comes back with the lesson going on; the entry brings the pane back | 2 min |
| 4.6 | Save and reopen mid-lesson | In lesson 2 (Waveforms), stop after a few steps, save the project, close REAPER, reopen it, open enth | The lesson resumes at the step you left, the passed steps still ✓, your own patch is restored after **End** | 8 min |
| 4.7 | Review | After lesson 2 is complete, the browser's **Review** | "Review • N due"; a press plays one round from a lesson you passed, with no hint before your first Submit | 5 min |
| 4.8 | Feel | While examples and rounds play, watch the meters and the pane; leave the window idle a minute | Smooth meters, no stutter when steps change, no CPU use while idle (REAPER's performance meter) | 3 min |

Lessons 1, 2 and 5 here are part of your tour (step 6): their votes and notes go in your answers file now.

## 5. Decide GR2's criteria and the friend round; cut the friend build (20 min, plus runner time)

**The criteria** (report 01 §7, adapted; `gr2-report` prints each as met, not met or pending). Say yes or change them in your notes:

1. Every participant names a specific new thing in **at least 3 lessons** (their "what did you learn?" notes, coded by an agent against each lesson's objective).
2. The median participant improves on **at least 2 of the 6 families**, and **no family gets worse on average**. Report 01 said 3 of 10; the tests are the Recreate generator's six categories (below), so 2 of 6 is the same share. `--families-improved 3` gives the stricter reading.
3. **You** name something new and pass a post-test item you failed on the pre-test.

**What changed from report 01's protocol**, and why:

- **Six families, not ten**: the test items are long challenge codes that randomize one generator category each (oscillators; sources, which adds a second oscillator, noise or glide; filter; envelopes; modulation; effects), so they play as ordinary Recreate rounds, logged automatically. The macro knob has no generator category, so the Macros lesson has no test item.
- **No "what changed?" items**: the plugin plays ear questions only inside lessons, so a test of them would need a test pack (open). The tour's own ear questions are counted per lesson (the report's "ear 1st try").
- **Per-lesson votes are typed**, in the answers file: the plugin has no "new to me / knew it" tap yet (open).
- The mid-test is all six families after lesson 5 (the tour's first third, lesson 1 included): oscillators, sources and filter have begun by then, envelopes, modulation and effects not, which is the multiple baseline.

**The friend round** (phase-3's open check "GR2's friend time", resolved here with its default): report 01's full protocol takes about 3 hours per friend, the tour alone about 1½–2. So **each friend does the full protocol if they agree, and otherwise the tour with per-lesson votes and the debrief; the pre/post rows come from you and the full-protocol friends.** Say if you want otherwise.

Send each friend something like this:

> enth has lessons now: 13 short ones (about 7 minutes each) that teach sound design on the synth itself. Would you play them over a week or two and tell me after each whether it was new to you? About 1½–2 hours in all. If you have about an hour more, I would also like you to play three short tests of 6 sounds (before, in the middle, at the end), which show whether the lessons helped. Your id is p2; if you do the tests, your first test is form B.

Give ids `p2`, `p3`, …; odd ids take form A first, even ids form B (counterbalanced).

**Cut the friend build** (as owner sheet 3, step 5; the repository is public, so the runners cost nothing; a run takes about 25–60 minutes, a few more than before because each leg builds the three packs):

```bash
cd ~/code/enth
gh workflow run release.yml --ref main -f macos=false      # drop -f macos=false if a friend has a Mac; -f windows=false if none has Windows
gh run list --workflow release.yml --limit 1               # its id and state
gh run watch <id>                                          # red: give the id to an agent
gh run download <id> --dir ~/enth-friend-build
```

Each zip now holds `Packs/` (the three `.enthpack` files), `gr2-friends.md` (the tour, the tests, what to send back) and `gr2-answers.toml` (the answers template), beside the plugin, `INSTALL.md` and `gr1-friends.md`. Forward each friend their OS's zip with the message above.

## 6. Your tour, mid-test, post-test and debrief (2.5–3 h in 4–5 sittings)

Play the tour as [gr2-friends.md](gr2-friends.md) says, with your answers file open: after each lesson, its vote, pace and one line of what you learned. You may play more than the tour (the full basics path, the studies); the report counts every lesson you complete, and only the tour's lessons have test families.

| When | What | Time |
| --- | --- | --- |
| After lesson 5 (Low-pass cutoff) | the mid-test: the six form M codes, as in step 2 | 20 min |
| After lesson 13 (Macros) | the post-test: the six form **B** codes | 25 min |
| Then | the debrief in your answers file: up to three things you learned, and the pace | 5 min |
| A week later (optional) | the delayed test: the six form D codes | 20 min |

## 7. Collect the files and run gr2-report (30 min)

Put everyone's files in one folder, named by id (nobody's name in a file name):

```bash
cd ~/code/enth
cp ~/Documents/enth/Learn/progress.json _local/gr2/p1.progress.json
cp ~/Documents/enth/Rounds/round-log.jsonl _local/gr2/p1.rounds.jsonl
# your p1.gr2.toml is there already (step 2)
# each friend's three files: _local/gr2/p2.gr2.toml, p2.progress.json, p2.rounds.jsonl (a tour-only friend: no rounds)
# a friend who sent answers in a message: copy content/gr2/answers-template.toml to _local/gr2/pN.gr2.toml and type them in
cargo xtask gr2-report _local/gr2 --validate               # fix what it names; exit 1 on an error or personal data
cargo xtask gr2-report _local/gr2 --notes                  # every note beside its lesson's objective
```

**Coding the notes:** give an agent the `--notes` output and the answers files, and ask it to write `coded = "specific"`, `"vague"` or `"none"` beside each `learned` line (and for a debrief thing, turn it into `{ text = "…", lesson = "pack/lesson", coded = "…" }` when it names a lesson). "Specific" means the note names something the lesson's objective teaches. You can do it yourself instead; either way the coder does not see the test results first.

```bash
cargo xtask gr2-report _local/gr2 --strict                 # the report; --json for enth.gr2-report/1
```

Read it in this order: the three criteria (met, not met, or pending with what is missing), the people (who finished the tour, who is short of notes), the items per person (pre, mid, post per family), the families (better, same, worse; the mid-test's taught against not-yet-taught split), the lessons (votes, pace and ear questions per lesson: a lesson many call too hard or "knew it" is the next content fix). The sign test is printed to be described, not leaned on: with five people, all improving gives p ≈ 0.03. **GR2 is your call**; the criteria are proposals.

## The GR2 files

All local; participants send them by choice, and `gr2-report` reads them only from disk.

| File | Form | Who writes it |
| --- | --- | --- |
| [`content/gr2/forms.toml`](../content/gr2/forms.toml) | `enth.gr2-forms/1`: `time_limit_s` (180), `structure_step` (10 points), `mid_after` (5 tour lessons), the six `[[families]]` (id, title, the tour lessons that teach it) and the 24 `[[items]]` (family, form A, B, M or D, code) | INT2 ([content/gr2/README.md](../content/gr2/README.md) says how the codes were made and checked) |
| `<id>.gr2.toml` | `enth.gr2-answers/1` ([template](../content/gr2/answers-template.toml)): `id`, `protocol` (`full` or `tour`), `pre_form` (`A` or `B`, full only), `[intake]` (`years`, `synths`, `confidence` 1–5 per family, 0 = not answered), one `[[lessons]]` per lesson (`key`, `vote` = `new` or `knew`, `pace` = `too_easy`, `ok` or `too_hard`, `learned`, `coded`), `[debrief]` (`pace`, up to three `things`). Blank `""` is "not answered" | the participant; `coded` by the coder |
| `<id>.progress.json` | the plugin's `Documents/enth/Learn/progress.json` (`enth.progress/1`) | the plugin |
| `<id>.rounds.jsonl` | the plugin's `Documents/enth/Rounds/round-log.jsonl` (`enth.round-log/1`), read through `gr1-report`'s reader | the plugin |

How the numbers are made (`xtask/src/gr2/numbers.rs` is the reference; the fixtures in `xtask/fixtures/gr2/` show each case with its hand count):

- **An item** is a test code's first played round in the person's log. It is a **clean pass** when a submit passed within 180 s of active time and the round used no hint, reveal, live meter or dimming; its **structure %** is the best of the submits made within the 180 s (0 when none was).
- **A family is better** when the post-test passes cleanly and the pre-test did not, or, with the pass unchanged, when structure % rose by at least 10 points; **worse** the other way round; otherwise **the same**. The mid-test is compared with the pre-test, the delayed test with the post-test.
- **A named new thing** is a note about a lesson coded `specific`.
- The GR1 report counts these test rounds as hand-set codes (it lists them under "Checks"): give `gr1-report` a round log from before your pre-test, or leave the GR2 codes in and read past them.

## How to report

`~/code/enth-scratch/owner/r2-notes.md`, one line per item; an agent fills the owner rows of [r2-gates.md](r2-gates.md):

```text
date: 2026-10-__   commit: _______   REAPER 7.82
1    installed: yes; packs in ~/Documents/enth/Packs: 3
2    pre-test form A: done (6 codes) | __ of 6
3    taste 1.1: ok | change: __     taste 2.1: ok | change: __
4.1  browser: ok | …      4.2 lesson 1: ok | …      4.3 highlight/lock: ok | …      4.4 how close: ok | …
4.5  hide: ok | …         4.6 reopen: ok | …        4.7 review: ok | …              4.8 feel: ok | …
5    criteria: as proposed | changed: __; friend round: default | changed: __
     friends: p2 os/daw full|tour, p3 …; release run id: ______ result: green | red at step __
6    my tour: __ of 13 lessons; mid-test: done; post-test: done; debrief: done; delayed: done | skipped
7    files collected: p1 … ; report attached; GR2: yes | no | not yet, why: __
crashes: none | step and time
```
