# Owner sheet 5: R3a (Create from sound)

- Date: 2026-10-22 (R3 lane INTa). Every R3a lane is merged (FE, OBJ, TP, F1S, EVK, FK3, SK3, CLC, PUI) and every automated R3a check is in [r3-gates.md](r3-gates.md). What is left needs you: owner build 5 with Create on, **the 5-sound check** in the plugin, a yes or no on Create in the friend build, and GR3's two optional data items (your ten picks, the phone re-recordings).
- **Order:** top to bottom. Steps 1 and 2 are the R3a check (about 45 minutes with the build). Steps 4 and 5 feed R3b's GR3 and can wait for another day.
- Windows open on your real display in step 2 and 3. Before the sitting read P1b's [Before each sitting](p1b-owner.md#before-each-sitting): save your work, wait 10 s between windows, and stop and note the step if Xorg crashes.
- Keep notes in `~/code/enth-scratch/owner/r3-notes.md` (template [at the end](#how-to-report)), then give the file to an agent.

| Step | What | Time | Unblocks |
| --- | --- | --- | --- |
| 1 | [Install owner build 5 and the bank](#1-install-owner-build-5-and-the-bank-20-min-no-windows) | 20 min, no windows (most of it is the build) | step 2 |
| 2 | [The 5-sound check](#2-the-5-sound-check-2030-min) | 20–30 min | **R3a ships** |
| 3 | [Three quick checks while REAPER is open](#3-three-quick-checks-while-reaper-is-open-10-min) | 10 min | two open checks |
| 4 | [Create in the friend build: yes or not yet](#4-create-in-the-friend-build-yes-or-not-yet-2-min-plus-runner-time) | 2 min, plus runner time | friends get Create |
| 5 | [GR3: your ten picks](#5-gr3-your-ten-picks-3045-min-optional) | 30–45 min, optional | GR3's owner stratum (R3b) |
| 6 | [GR3: about 30 phone re-recordings](#6-gr3-about-30-phone-re-recordings-30-min-optional) | 30 min, optional | GR3's degraded set version 2 (R3b) |

**Still pending from earlier sheets** (not repeated here): [owner sheet 4](r2-owner.md) (R2), [owner sheet 3](r1s1-owner.md) (R1 and S1) and P1b's [owner sheet](p1b-owner.md). None of them blocks this one.

## 1. Install owner build 5 and the bank (20 min, no windows)

Owner build 5 is the plugin with the `create` feature on (Recreate and Learn stay on), plus the k-NN bank file that gives Create its best starting patches.

```bash
cd ~/code/enth && git pull
cargo xtask install --package enth-plugin --release --features create   # ~/.clap and ~/.vst3 (about 7 min)
cargo xtask bank                                                        # the k-NN bank, about 8 min: target/bank/enth-bank-v1.bin
sha256sum target/bank/enth-bank-v1.bin                                  # must start with 66d5f3808b5b9d6f
mkdir -p ~/Documents/enth/Models
cp target/bank/enth-bank-v1.bin ~/Documents/enth/Models/
```

- The bank is rebuilt bit for bit from the code (it is not in git): if the SHA-256 differs, stop and give the output to an agent.
- The plugin looks for the bank beside `~/.clap/enth-plugin.clap`, then in `~/Documents/enth/Models/`. Without it Create still runs (on the heuristic and four templates) and says so in its notes.
- Optional, for `enth create` on the command line: `cargo build --release -p enth-cli`, then `./target/release/enth create --target <file> --bank target/bank/enth-bank-v1.bin --out best.patch.json`.

## 2. The 5-sound check (20–30 min)

**Pick five sounds that are not among your ten GR3 picks** (step 5), so GR3's blind test stays blind. One note each, a few seconds long; any of WAV, AIFF, FLAC, Ogg Vorbis or MP3. A good spread:

| # | Kind | Why |
| --- | --- | --- |
| A | a synth bass or lead note from another synth | the case Create is built for |
| B | a pluck, keys or bell note | a fast attack and a decay |
| C | a pad or a long held note (2 s or more) | the sustain and the release (the intake can read a slow pad's note-off early, [r3-gates.md](r3-gates.md#open): listen whether the candidates' release sounds right) |
| D | a short drum hit (a kick, a snare or a hat; under 0.3 s is fine) | short sounds are never refused |
| E | a "damaged" copy: an MP3, a phone recording, or a sound with noise or room in it | the robust intake and objective |

In REAPER, insert "CLAPi: enth (enth)" on a new track and arm it for the virtual keyboard ([Install](p1b-owner.md#install-about-10-minutes-no-windows)). Open the plugin window and click **Create** in the header: a column opens at the right edge.

For each sound:

1. **Drop the file** on the window (or **Browse…** to it and click it). The job starts at once.
2. **Read the findings** under the drop zone: how it decoded, the note it heard and the hold ("Note A3 (220.0 Hz) • held 0.30 s of 0.60 s"), what it trimmed or found damaged, and any warning (a chord, a phrase, several notes). Write down whether the note is right.
3. **Time the first tiles**: from the drop until the eight tiles show distances (the progress line reads "… of 4,000 renders • … of 60 s"). It should be **at most 15 s** (INTa measured 0.1–0.7 s with `enth create`, the same code). The job stops by itself at 60 s; **Cancel** stops it sooner.
4. **Listen to the top three** (rank 1, 2, 3 on the tiles): select a tile, **▶ Candidate**, **▶ Upload**, and **A/B** a few times (both sides are level-matched; nothing is amplified). Also try **▶ Start** on one tile to hear what the search started from.
5. **Rate the best of the three**, 1–5: 1 nothing alike, 3 a usable start (the right kind of sound, you would tweak it), 5 you would use it as it is.
6. **Use this** on the best one, play a few keys on the virtual keyboard, then the header's **Undo**: your patch before comes back.

While the searches run, watch REAPER's performance meter: **no dropouts or crackles** on a playing track, and the window stays responsive.

**Pass (proposed; yours to change):** for at least 3 of the 5 sounds the best of the top three rates 3 or more; the first tiles come within 15 s every time; no dropout, crash or hang. Phase-3's default if you skip this step: R3a ships on the kit's numbers ([r3-gates.md](r3-gates.md)).

## 3. Three quick checks while REAPER is open (10 min)

| # | Check | What to do | Note down |
| --- | --- | --- | --- |
| 3.1 | **Drag from REAPER** (an open check: does REAPER drag media items as files?) | Drag one of your sounds onto the arrange view, then drag that media item from the track onto the Create column | Whether the drop zone lights up and the job starts. If not, the folder browser is the way in (nothing to fix now) |
| 3.2 | **Close and unload while searching** | Start a job; close the plugin window and reopen it (the job goes on); start another; then remove the plugin from the track (or close the project) while it searches | REAPER never freezes; the plugin goes away at once |
| 3.3 | **Dropping a second file** | While a job runs, drop another file | It is only chosen ("Start it once … is done"); Start runs it after |

## 4. Create in the friend build: yes or not yet (2 min, plus runner time)

**Default: not yet.** The friend build keeps Create off until your 5-sound check passes, so friends do not meet it first. When it has passed and you want friends to have it, run the release with the `create` input on: every zip then holds the plugin with Create and `enth-bank-v1.bin` beside the bundles (each leg rebuilds the bank and checks its SHA-256), and [friends-install.md](friends-install.md) says where the file goes.

```bash
cd ~/code/enth
gh workflow run release.yml --ref main -f create=true -f macos=false   # drop -f macos=false if a friend has a Mac
gh run list --workflow release.yml --limit 1                          # its id and state
gh run watch <id>                                                     # red: give the id to an agent
gh run download <id> --dir ~/enth-friend-build-create
```

A run with Create takes a few minutes longer per leg (the bank). Without `-f create=true` the build is the R2 friend build ([owner sheet 4, step 5](r2-owner.md#5-decide-gr2s-criteria-and-the-friend-round-cut-the-friend-build-20-min-plus-runner-time)).

## 5. GR3: your ten picks (30–45 min, optional)

GR3's listening test (clause 2, in R3b) has a stratum of ten sounds you care about. Each slot you leave empty uses an agent's CC0 pick (a supersaw, a Reese bass, an 808 kick, a hoover, a superwave lead, a wobble sub bass, a Rhodes, a saw bass, a piano-bell and a digital pad; [content/gr3/README.md](../content/gr3/README.md#your-part-the-owner)).

- Save them as `~/code/enth-scratch/owner/r3-picks/owner-01.<ext>` … `owner-10.<ext>` (any format enth reads), one note each, a few seconds.
- Beside them, `~/code/enth-scratch/owner/r3-picks/sources.txt`: one line each, `owner-NN: where it is from; licence (CC0, mine, or other)`.
- **Do not run Create on them** and do not listen to them through enth: GR3 plays them to you blind later. Your files stay local unless they are CC0 or yours.
- Then tell an agent "GR3 amendment 1: the owner picks are in r3-picks"; it fills the slots in `content/gr3/real.toml` (hashes, licences) under the freeze's one allowed amendment.

## 6. GR3: about 30 phone re-recordings (30 min, optional)

GR3's degraded references stand in for a phone recording with a synthetic chain (`phone-recording`). Real ones are better evidence: you play GR3's 30 clean agent-picked references through the laptop's speakers and record them with your phone. They become a new set version (they never replace items in this one) before R3b's first training run.

1. Put the phone about 1 m from the laptop in a normal room, start a voice-memo recording, and keep the room quiet.
2. Paste this (it plays a short beep, the sound's name in the terminal, the sound, then 2 s of silence, for all 30; about 4 minutes; volume at a normal listening level):

   ```bash
   cd ~/code/enth
   python3 -c 'import tomllib
   for x in tomllib.load(open("content/gr3/real.toml", "rb"))["reference"]:
       if x.get("role") == "test" and x.get("source") == "freesound": print(x["id"], x["freesound_id"])' > /tmp/r3-phone.txt
   while read -r id fid; do
     echo "$id"
     ffplay -nodisp -autoexit -loglevel error -f lavfi "sine=frequency=1000:duration=0.3"
     sleep 1
     ffplay -nodisp -autoexit -loglevel error ~/code/enth-scratch/r3-data/freesound/"$fid"_*-hq.ogg
     sleep 2
   done < /tmp/r3-phone.txt
   ```

3. Stop the recording, copy the phone's file to `~/code/enth-scratch/owner/r3-phone/take-1.<ext>` (with the phone's model and the room in `~/code/enth-scratch/owner/r3-phone/notes.txt`), and tell an agent; it cuts the take at the beeps into 30 clips for set version 2. A second take in another room is welcome but optional.

## How to report

`~/code/enth-scratch/owner/r3-notes.md`, one line per item; an agent fills the owner rows of [r3-gates.md](r3-gates.md):

```text
date: 2026-10-__   commit: _______   REAPER 7.82   bank sha256 starts: ________
1    installed: yes; bank in ~/Documents/enth/Models: yes
2A   file: ______ (format)  note heard: right | wrong (heard __, is __)  warnings: __  first tiles: __ s  best of top 3: _/5  use+undo: ok | …
2B   …
2C   …
2D   …
2E   …
2    dropouts while searching: none | when; responsive: yes | no; pass: yes | no | changed criteria: __
3.1  REAPER media item drag: starts a job | nothing happens | other: __
3.2  close/reopen/unload while searching: ok | froze for __ s | crashed
3.3  second file while running: waits | other: __
4    Create in the friend build: not yet | yes, run id ______ result: green | red at step __
5    picks: __ of 10 in r3-picks (sources.txt written: yes | no)
6    phone take: done (phone ______, room ______) | skipped
crashes: none | step and time
```
