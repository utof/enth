# Owner sheet 3: R1 and S1 (the Recreate game and Instrument 0.1)

- Date: 2026-10-20 (R1/S1 lane INT). Every R1/S1 lane is merged and every automated check passes ([r1s1-gates.md](r1s1-gates.md)). What is left needs you, your ears and your friends.
- **Order:** do the steps top to bottom. Each says how long it takes. Your new time is about **3½ to 4½ hours**, spread over a few days. Your friends' rounds set the end date, so step 1 comes first.
- Windows open on your real display in steps 3, 4 and 6. Before those, read P1b's [Before each sitting](p1b-owner.md#before-each-sitting): save your work, wait 10 s between windows, and stop and write down the step if Xorg crashes.
- Keep notes as you go in `~/code/enth-scratch/owner/r1s1-notes.md` (template [at the end](#how-to-report)), then give the file to an agent.

| Step | What | Time | Unblocks |
| --- | --- | --- | --- |
| 1 | [Friend survey](#1-friend-survey-5-min) | 5 min | which builds to make; GR1's players |
| 2 | [Listening: metric session 2 and the real-engine pairs](#2-listening-3540-min) | 35–40 min, in one or two sittings | the sound score's curve (`sound/2`) |
| 3 | [Owner build 3: the real display and five rounds](#3-owner-build-3-45-min) | 45 min | GS1's real-display row; a smoke test of the game |
| 4 | [Pick 6 of the 22 presets](#4-pick-6-presets-3060-min) | 30–60 min | GS1's ten presets (with your four G1 sounds) |
| 5 | [Cut the friend build and send it](#5-cut-the-friend-build-15-min-plus-runner-time) | 15 min, plus 20–55 min of runner time | GR1's first row; the friends' rounds |
| 6 | [Your own 20 rounds](#6-your-own-20-rounds-60100-min-in-34-sittings) | 60–100 min in 3–4 sittings | GR1 |
| 7 | [Collect the logs and read the report](#7-collect-the-logs-and-run-gr1-report-20-min) | 20 min | the GR1 decision |
| 8 | [Bitwig poly-mod](#8-bitwig-poly-mod-optional-20-min) (optional) | 20 min | GS1's poly-mod row in a real host |

**Still pending from earlier phases** (not repeated here; do them whenever you like): P1b's [owner sheet](p1b-owner.md): owner build 1 on the real display (45 min), owner build 2, where you make G1's four sounds (60–90 min, and also GS1's first four presets), and real audio (30 min); P1a's [reverb A/B and blind listen](p1a-g1.md) (35–50 min).

## 1. Friend survey (5 min)

Send each friend something like this (no build yet):

> I'm testing a synth game I'm building: it makes a hidden sound and you rebuild it by ear. Would you play 20 short rounds (about an hour and a half in all, in a few sittings) and send me a small log file? Which computer (Windows, Mac with Apple silicon or Intel, Linux) and which music program do you use?

Write down, per friend: an id (`p2`, `p3`, …; you are `p1`), OS, DAW, yes or no. Logic and GarageBand cannot load enth yet (they only take Audio Units), and Linux needs Ubuntu 24.04 or newer. The OSes decide which legs to build in step 5.

## 2. Listening (35–40 min)

**Metric session 2** (headphones, no windows): the test–retest of the pairs you rated in session 1 ([listening.md](listening.md#3-metric-pairs-spike-metric)).

```bash
cd ~/code/enth
cargo build --release -p spike-metric                                 # once, a few minutes
./target/release/spike-metric listen --dir ~/enth-listen --session 2  # about 15 min; q quits, the same command resumes
./target/release/spike-metric listen --dir ~/enth-listen --analyze
```

Send back the printed report. It tells whether your ratings repeat, which the sound score's curve is fitted to.

**The real-engine pairs session** (15–20 min, plus a 1–2 min design; headphones, no windows): 40 pairs rendered by the real engine at planned sound-score distances, plus four checks ([listening.md](listening.md#4-real-engine-pairs-how-similar-do-two-sounds-seem-at-a-given-sound1-distance)). Each trial plays A, B, then X: type `a` or `b` for which one X is, then rate how similar A and B sound, 1 (very different) to 5 (indistinguishable). Some trials play the same sound twice: guess X and rate what you hear.

```bash
cd ~/code/enth
cargo build --release -p enth-listen                                  # once, a few minutes
./target/release/enth-listen pairs --dir ~/enth-listen                # designs once (1-2 min, all cores; close other work), then the status
./target/release/enth-listen pairs --dir ~/enth-listen --session 1    # about 15-20 min; q quits, the same command resumes
./target/release/enth-listen pairs report --dir ~/enth-listen
```

Send back the printed report and `~/enth-listen/pairs-fit.json`: an agent fits `sound/2`'s curve to them. Until then the curve stays the one bridged from the toy synth, and GR1 can start without it. A retest on another day (`--session 2`, the same pairs reshuffled) is optional and makes the fit firmer.

## 3. Owner build 3 (45 min)

Install the current build (no windows; about 10 min):

```bash
cd ~/code/enth && git pull
cargo xtask install --package enth-plugin --release    # ~/.clap and ~/.vst3
```

In REAPER, insert "CLAPi: enth (enth)" on a new track and arm it for the virtual keyboard (P1b's [Install](p1b-owner.md#install-about-10-minutes-no-windows)).

| # | Check | What to do | Pass looks like | Time |
| --- | --- | --- | --- | --- |
| 3.1 | Add and remove | "+ Add" in the header: add an LFO; "…" on its card: remove it; Undo | Each is one step; Undo brings it back; nothing flickers or crashes | 3 min |
| 3.2 | The preset browser | Browse; type `pad`; click a preset, then ◀ ▶; hold ▶ on a row (audition); Save as "Test" | Clicks load at once, audition sounds with the transport stopped, Undo takes a load back, Save asks before replacing | 5 min |
| 3.3 | The Recreate bar | Recreate, Level 1, Start; Target ▶, Mine ▶, A/B; Submit; rate it; vote; End and restore my patch | The bar fits the header, A/B flips without a click or a level jump, the drawer shows the score, your own sound comes back | 7 min |
| 3.4 | Five low-difficulty rounds | Levels 1 and 2, up to 5 minutes each; vote on every score | Note per round: the score, your vote, and anything that felt unfair or confusing. Don't count these toward GR1's 20 | 25 min |
| 3.5 | Frame rate and focus | While a round's target plays, watch the meters; press Space after clicking in the editor | Smooth meters (about 30 fps); Space reaches REAPER's transport | 2 min |

## 4. Pick 6 presets (30–60 min)

Twenty-two candidates are waiting ([content/presets-candidates/README.md](../content/presets-candidates/README.md): what each is, its macros Bright, Pluck, Space and Drive). GS1's ten are your four G1 sounds (P1b owner build 2) plus the six you keep.

1. Make them browsable as user presets: `cp -r ~/code/enth/content/presets-candidates ~/Documents/enth/Presets/Drafts`.
2. In enth's browser: User tab, folder Drafts. Click through them (hold ▶ to audition), play each a little, turn the four macros.
3. In [`content/presets-candidates/PICKS.md`](../content/presets-candidates/PICKS.md), write `yes` in **Keep** for the six, a new name in **Factory name** if you like, and a note for anything you'd change. Fill "Picked on" and the commit (`git -C ~/code/enth rev-parse --short HEAD`).
4. An agent then runs `cargo xtask preset-add --picks content/presets-candidates/PICKS.md` (into `content/presets/<type>/`) and the factory test, and commits them. A tweaked version: save it in the editor under a name and tell the agent the name.

## 5. Cut the friend build (15 min, plus runner time)

The repository is public, so GitHub's runners cost nothing; a full run takes about 20 minutes warm and 55 cold. Build only the legs your friends need (step 1). Nothing has been linked or run on Windows or macOS yet, so this run is also the first test ([distribution.md](distribution.md#what-to-check-on-the-first-run)).

```bash
cd ~/code/enth
gh workflow run release.yml --ref main -f macos=false        # drop -f macos=false if a friend has a Mac; -f windows=false if none has Windows
gh run list --workflow release.yml --limit 1                 # its id and state
gh run watch <id>                                            # until it ends; red: give the id to an agent
gh run download <id> --dir ~/enth-friend-build               # one zip per OS: enth-dev-<commit>-<os>.zip
```

A tag instead names the build (`git tag v0.1.0-friend1 && git push origin v0.1.0-friend1` runs all three legs). Each zip holds both plugin formats, `INSTALL.md` (the install sheet), `gr1-friends.md` (how to play and send the log) and `BUILD.txt`.

**Forward** each friend the zip for their OS (chat or a cloud link; about 11 MB, or more for the Mac one) with three lines: their id, "install with INSTALL.md, play with gr1-friends.md", and the code list below. Ask **one friend per OS** for a smoke round first (one set of 5), and fix what breaks before the rest start.

**The 20 codes** ([content/gr1/codes.txt](../content/gr1/codes.txt); everyone plays the same ones, in sets of five, one per level):

| Set | Level 1 | Level 2 | Level 3 | Level 4 | Level 5 |
| --- | --- | --- | --- | --- | --- |
| 1 | `1040-1000-000Z-9V` | `1080-1000-000Z-9T` | `10C0-1000-000Z-96` | `10G0-1000-000Z-9R` | `10M0-1000-000Z-94` |
| 2 | `1040-1000-000Z-AX` | `1080-1000-000Z-AW` | `10C0-1000-000Z-A0` | `10G0-1000-000Z-AY` | `10M0-1000-000Z-A2` |
| 3 | `1040-1000-000Z-BZ` | `1080-1000-000Z-BY` | `10C0-1000-000Z-B2` | `10G0-1000-000Z-BW` | `10M0-1000-000Z-B0` |
| 4 | `1040-1000-000Z-CH` | `1080-1000-000Z-CG` | `10C0-1000-000Z-CC` | `10G0-1000-000Z-CJ` | `10M0-1000-000Z-CE` |

## 6. Your own 20 rounds (60–100 min in 3–4 sittings)

Play the 20 codes above the way your friends do ([gr1-friends.md](gr1-friends.md)): paste a code into **Paste a code**, Start, listen, Submit, rate before the score, **vote on every score**, and use **Disagree…** when a score is plainly wrong. "New round" or "Try again" straight away means "want more"; End otherwise. Your log is `~/Documents/enth/Rounds/round-log.jsonl`. If nobody else plays, your log alone is GR1's owner-only fallback.

## 7. Collect the logs and run gr1-report (20 min)

The full checklist is [gr1-owner.md](gr1-owner.md); in short:

```bash
cd ~/code/enth
mkdir -p _local/gr1/logs                                       # _local is not in git; never commit friends' logs
cp ~/Documents/enth/Rounds/round-log.jsonl _local/gr1/logs/p1.jsonl
# save each friend's file as _local/gr1/logs/p2.jsonl, p3.jsonl, … and type their survey lines into _local/gr1/survey.jsonl
cargo xtask gr1-report _local/gr1/logs --validate             # fix what it names; exit 1 on personal data
cargo xtask gr1-report _local/gr1/logs --baseline content/gr1/floors.jsonl --survey _local/gr1/survey.jsonl --strict
```

`content/gr1/floors.jsonl` already holds the no-op and CMA-ES floors on the 20 codes (the start passes 0 of 20, CMA-ES 13 of 20; re-made on engine 0.3.0). The agent arm is the one step that costs money (about $25 for three attempts on each code): say if you want it, and an agent runs it ([gr1.md](gr1.md#making-the-baseline-lines)). Read the report in [gr1-owner.md](gr1-owner.md#5-read-the-report)'s order: the gate lines, who is short of 20, agreement, pass rate by level, every dispute. The thresholds (70 % "about right", no rater under 50 %, 60 % want more) are proposals; GR1 is your call.

## 8. Bitwig poly-mod (optional, 20 min)

Poly-mod passes in the test host (P1b lane PL); this is the real-host check. In a Bitwig trial (it needs an account): insert enth (CLAP), put a per-voice modulator (Random or an envelope) on **Macro 1**, route Macro 1 to the filter's cutoff in enth's editor, play a chord. Pass: each voice sounds different, and on release no voice hangs. Save, reopen: still there.

## How to report

`~/code/enth-scratch/owner/r1s1-notes.md`, one line per item, then hand it to an agent (it fills the owner rows of [r1s1-gates.md](r1s1-gates.md) and the open checks in [phase-2.md](phase-2.md#open-checks)):

```text
date: 2026-10-__   commit: _______   REAPER 7.82
1    friends: p2 os/daw yes|no, p3 …
2    metric session 2: done (report attached) | skipped
2b   real-engine pairs: session 1 done (report + pairs-fit.json attached) | skipped; session 2: done | skipped
3.1  add/remove: ok | what went wrong
3.2  browser: ok | …
3.3  recreate bar: ok | …
3.4  rounds: L1 __ % vote __, L1 __ % vote __, L2 …   unfair or confusing: __
3.5  meters smooth: yes/no; space after click: yes/no
4    picks written in PICKS.md: yes/no
5    release run id: ______ legs: linux/windows/macos   result: green | red at step __
     smoke round: p_ (os) __ of 5 rounds, problems: __
6    my 20 rounds: done | __ of 20
7    report: attached; GR1: yes | no | not yet, why: __; agent run: yes/no
8    bitwig: pass | fail (what) | skipped
crashes: none | step and time
```
