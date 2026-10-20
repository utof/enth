# Owner checklist: what Phase 0 left for your eyes, ears and hands

- **2026-10-20 (P1b lane INT): the REAPER checks moved to the [P1b owner sheet](../p1b-owner.md),** on the product plugin: C1, C3, C4, C5 and C6 are its owner build 1 and its real-audio check, and owner build 2 is G1's "bass, pad, pluck and lead". The shell and UI spikes left the workspace and no longer build, so this page keeps only what does not need them: the listening (A), the dev window and the GPU probes (B), the scope rate (C2, now in the dev window) and section D.
- Date: 2026-10-19 (lane Z). Updated the same day after P1b lane FK: decisions 1 and 4 below are settled, and 5 is now the optional upstream reports. One ordered list that replaces the separate lists in [shell.md](shell.md), [ui.md](ui.md), [metric.md](metric.md), [agent-probe.md](agent-probe.md), [test-host.md](test-host.md) and the research reports 01-final and 02-final (`_local/sessions/2026-10-19-phase-0-spikes/`).
- Nothing in P1a waits for these. They close the G0 rows marked *pending owner* in [roadmap.md](../roadmap.md#p0--spikes-and-skeleton-done-2026-10-19-except-owner-checks) and [benchmarks.md](../benchmarks.md#ui-thresholds), and they decide whether ADR-0002 stays accepted and whether ADR-0007 can be accepted.
- Total: about 2½ hours of checks plus the two 15-minute listening sessions, in as many sittings as you like. Sections A and B are safe; risk rises from C to D.

## Safety first (read once)

- **Save your work and close other projects before section B.** On 2026-10-19 the NVIDIA 595 X driver crashed Xorg while GL windows were opened and closed (`nvidia_drv.so +0x3fc5a`, NVIDIA bug 6205955). A crash ends the desktop session and every open application with it.
- **Leave at least 10 s between opening windows**, and do one step at a time.
- These steps run on the real display, so they are yours alone: agents never run them ([AGENTS.md](../../AGENTS.md)).
- If Xorg does crash: log back in, note the step and the time, and stop. Step D4 (`prime-select on-demand`) moves the desktop to the Intel GPU if it keeps happening.
- Write what you see into a note (for example `~/code/enth-scratch/owner/notes.md`) and give it to an agent; the agent updates ADR-0002, ADR-0007, benchmarks.md and the roadmap.

## Build first (about 10 minutes, no windows)

From the repo root (`~/code/enth`), after merging lane Z:

```bash
cargo build --release -p spike-metric                                  # the listening tool
cargo build --release -p enth-ui --example dev_window --features dev-window   # the editor's dev window (replaces spike-ui-dev, 2026-10-20)
```

The plugin is installed by the [P1b owner sheet](../p1b-owner.md#install-about-10-minutes-no-windows) (`cargo xtask install --package enth-plugin --release`); `spike-ui` and `spike-ui-plugin` no longer build.

## A. Listening (no windows, headphones)

### A1. Metric listening, session 1 (about 15 minutes)

Decides the G0 metric gate and ADR-0007. Full description in [metric.md](metric.md#how-to-run-it-about-2--15-minutes).

```bash
./target/release/spike-metric listen --dir ~/enth-listen --session 1
```

- The first call designs the 30 pairs plus 4 anchors (about 3 s) and writes about 29 MB of WAVs.
- Set the headphone volume once and don't touch it again in this session or the next. Type `1`–`5` and Enter (1 = very different, 5 = indistinguishable), `r` to replay, `q` to quit; the same command resumes.
- Don't open `~/enth-listen/manifest.json`: it holds the distances and would unblind you.

### A2. Metric listening, session 2 (about 15 minutes, ideally another day)

```bash
./target/release/spike-metric listen --dir ~/enth-listen --session 2
./target/release/spike-metric listen --dir ~/enth-listen --analyze     # writes ~/enth-listen/listen-report.json
```

The gate: Spearman ρ ≥ 0.7 for the best distance, its lower 95 % bound ≥ 0.4, τ-b within 0.15 of your own test–retest τ-b, and the anchors right (identical pairs ≥ 4.5, obviously different ≤ 1.5).

### A3. Blind check of the agent trial (about 10 minutes)

Three agents built 15 sounds for five text goals ([agent-probe.md](agent-probe.md#trial-2026-10-19)); nobody has listened. Judge each sound against its text only, without looking at the patches or the numbers. Start with the volume low: some renders reach 0 dBFS.

| File prefix | Text the agent was given |
| --- | --- |
| `g1-pluck` | a short bright pluck |
| `g2-pad` | a slow dark pad |
| `g3-buzzy-bass` | a buzzy bass |
| `g4-bell` | a soft bell-like tone |
| `g5-supersaw-lead` | a detuned supersaw-ish lead |

Paste into `bash` (it plays the 15 files in random order and records y / p / n per file):

```bash
cd ~/enth-listen/agent-trial
for f in $(ls *.wav | shuf); do
  echo "== $f"; pw-play "$f"
  read -r -p "matches its text? y = yes, p = partly, n = no: " a
  echo "$f $a" >> ~/enth-listen/agent-trial-ratings.txt
done
```

This is the dry run of the G1 check ("at least 7 of 10 final sounds match their text").

## B. Standalone windows on the real display (save your work first)

### B1. Dev window: look, feel and cost (about 15 minutes)

2026-10-20: the spike's dev window is retired; the real editor's dev window takes its place (the real editor over a real session, an engine rendering into buffers nobody hears; [editor.md](../editor.md#the-dev-window)). It has no gallery and no "Audio on" button: F3 holds a note (middle C) instead, and the process is `dev_window`.

```bash
./target/release/examples/dev_window --patch content/templates/pad_2osc.patch.json
```

- Is the first frame drawn, or does the window stay blank (the researcher saw blank standalone windows on this GPU)?
- Do type, contrast and spacing read well at 100 %? Drag knobs (and with Shift), use the wheel, double-click to reset, drag a lane card by its header, scroll the generator column and the lesson.
- Hold F3 (a note plays into the engine; the rings, tiles, scope and meters move), then in a second terminal:

  ```bash
  pidstat -t -p "$(pgrep -f examples/dev_window)" 1 20
  nvidia-smi pmon -s um -c 20
  ```

  The CPU side was 3.5–4.2 % of a core at 30 fps headless; the gate is < 5 % per editor and < 5 % SM.
- With audio on and `pidstat` running, cover the window completely with another window for 10 s, then minimise it for 10 s. The GUI thread should drop to about the idle value (gate: occluded ≤ idle).
- Release F3 and let the note end: the CPU should fall to about zero (the editor draws nothing while nothing changes).

### B2. glow vs wgpu on the real GPU (about 5 minutes)

From research 02 (four 15 s runs; wait 10 s between them). Prebuilt probes, no build needed:

```bash
cd ~/code/enth-scratch/2026-10-19-phase-0-spikes/02
export ENTH_OWNER_PRESENT=1
ENTH_UI_MODE=idle  python3 -I scripts/measure-real.py --log runs/real-glow-idle.log  --secs 15 --label real-glow-idle  -- target-probe/release/enth_ui_probe --backend dummy
ENTH_UI_MODE=fps30 python3 -I scripts/measure-real.py --log runs/real-glow-fps30.log --secs 15 --label real-glow-fps30 -- target-probe/release/enth_ui_probe --backend dummy
ENTH_UI_MODE=idle  python3 -I scripts/measure-real.py --log runs/real-wgpu-idle.log  --secs 15 --label real-wgpu-idle  -- target-probe-wgpu/release/enth_ui_probe --backend dummy
ENTH_UI_MODE=fps30 python3 -I scripts/measure-real.py --log runs/real-wgpu-fps30.log --secs 15 --label real-wgpu-fps30 -- target-probe-wgpu/release/enth_ui_probe --backend dummy
```

Per run, note whether the window drew. The JSON gives launch → first frame, fps, GUI-thread CPU, wake-ups, PSS and `nvidia-smi` SM % and memory. glow stays unless wgpu is clearly better on open time, fps and VRAM ([benchmarks.md](../benchmarks.md#decision-rules-for-the-ui-toolkit-gate-g0), rule 2).

## C. REAPER on the real display (save your work first)

Start your own REAPER with `~/apps/reaper-7.82/reaper_linux_x86_64/REAPER/reaper`. Once: *Options → Preferences → Plug-ins → CLAP*, make sure `~/.clap` is scanned and press *Re-scan*; the same for VST3 with `~/.vst3`. Quitting REAPER between steps is fine.

**2026-10-20: C1, C3, C4, C5 and C6 are replaced by the [P1b owner sheet](../p1b-owner.md) on `enth-plugin`; the spike steps below are kept as the record and no longer run (the spikes do not build).**

### C1. First open, idle cost, close and reopen (about 20 minutes)

The plugin is "enth spike-ui": the shell from [shell.md](shell.md) with the full reference panel, so this covers shell item (i) too.

- Insert the CLAP version on a fresh track 10 times (close the FX window, wait 10 s, insert again). Is the editor ever blank or grey on first open? Then the same 10 times with the VST3 version.
- With one editor open and nothing playing, for 60 s each, then the same with the editor closed:

  ```bash
  pidstat -t -p "$(pgrep -x reaper)" 1 60 | grep -E "enth-gui|Average"
  nvidia-smi pmon -s um -d 1 -c 60
  ```

- Play notes (macro 1 is gain, slot 1 is cutoff). Do the rings, tiles and scopes move smoothly at about 30 fps, and stop when the notes end? Does the status line follow the hovered knob?
- Resize the FX window. Close and reopen the editor 20 times, 10 s apart.

### C2. Scope rate (about 5 minutes)

2026-10-20: `enth-plugin` does not read `ENTH_THEME`; the dev window does. Make a theme with the scope token at 30 Hz and compare it with the default:

```bash
./target/release/examples/dev_window --dump-theme > /tmp/theme-scope30.json   # set motion.scope_hz to 30 in it
./target/release/examples/dev_window --theme /tmp/theme-scope30.json --patch content/templates/pad_2osc.patch.json
```

Hold F3. Under baseview 0.4.1 a 30 Hz request gives about 20 fps (a 60 Hz request, the default, about 30). Do 20 fps scopes look smooth enough? See decision 3.

### C3. Real audio and xruns (about 30 minutes)

```bash
pw-metadata -n settings 0 clock.force-quantum 64      # 64-sample buffer
pw-top                                                 # check QUANT shows 64 for the graph; if not, also: pw-metadata -n settings 0 clock.min-quantum 64
pw-jack ~/apps/reaper-7.82/reaper_linux_x86_64/REAPER/reaper
```

- *Preferences → Audio → Device*: JACK. Put "enth spike-ui" on 8 tracks, each holding a chord (a looped MIDI item), editors closed, for 10 minutes; note REAPER's xrun count (*View → Performance Meter*) and the ERR column in `pw-top`. Then open the editors (as many of the 8 as fit) for another 10 minutes and compare. Listen for clicks.
- Gate: no extra xruns with the editors open.
- Afterwards: `pw-metadata -n settings 0 clock.force-quantum 0` (and `clock.min-quantum 1024` if you changed it).

### C4. Keyboard and focus (about 5 minutes)

```bash
cp ~/code/enth-scratch/2026-10-19-phase-0-spikes/02/clap/enth_ui_probe.clap ~/.clap/
```

Rescan, insert "enth UI probe", open it:

1. Click an empty editor area and press Space. Does the transport toggle? (Under Xvfb it did not: baseview never forwards unhandled keys.)
2. Click "Preset name" and type `a b`. Does the text appear, with the transport untouched? (It did under Xvfb.)
3. Click REAPER's arrange view and press Space. Does the transport toggle again? (Unknown: Xvfb has no window manager.)

Repeat 1 and 3 with "enth spike-ui".

### C5. HiDPI (about 10 minutes, two REAPER restarts)

```bash
echo "Xft.dpi: 192" | xrdb -merge          # then restart REAPER and open "enth spike-ui"
echo "Xft.dpi: 96"  | xrdb -merge          # afterwards; restart REAPER again
```

Is the editor sharp, and does REAPER clip it to its top-left quarter (seen under Xvfb, baseview #359)? Applications started while 192 is set also pick it up.

### C6. Slot rename in the UI (about 5 minutes)

```bash
cp ~/code/enth-scratch/2026-10-19-phase-0-spikes/01/bundles/spike_np_rename.clap ~/.clap/
```

Rescan and load "enth spike-np-rename" (CLAP; it carries the researcher's 123-line nice-plug patch). Create an automation lane for Slot 1, then set Slot 64 to its maximum. Do the FX parameter list and a new automation lane show "Osc 1 · Cutoff"? Does the lane created earlier keep its old label? Decision 1 is settled (the fork carries this patch: lane FK vendored it unchanged as patch 1 of 5 and checked the rename with a scratch CLAP host, `vendor/tests/rename-check/`, not with REAPER), so this step is only the REAPER confirmation.

## D. Optional, and the riskiest

### D1. Bitwig trial: poly-mod (about 45 minutes including the install)

Installing the trial and accepting its licence is your call. Then `cargo xtask install --package enth-plugin --release` (2026-10-20; it was the shell spike), load "enth" (CLAP), add a per-voice modulator on Macro 1 and check that held voices modulate independently; save and reload the project. If Ardour is around, do the plain load, play, automate and save checks with the VST3. This settles shell items (d) and (h); the real poly-mod check is in S1 anyway.

### D2. iced busy loop (2 minutes, low value)

iced is already out ([ADR-0002](../adr/0002-ui-toolkit.md)); this only checks whether research 02's "one core at 100 % while idle" was an Xvfb artefact:

```bash
cd ~/code/enth-scratch/2026-10-19-phase-0-spikes/02
ICED_BACKEND=tiny-skia target-np/release/gain_iced --backend dummy &
top -H -p $!          # 10 s, then quit top and: kill %1
```

### D3. 50 editor open/close cycles on the real display (about 3 minutes; highest crash risk)

Everything saved, other applications closed. This is the open/close churn that crashed Xorg; the test host's `editor-cycles` refuses to run outside Xvfb on purpose, so this uses research 02's scratch REAPER copy:

```bash
cd ~/code/enth-scratch/2026-10-19-phase-0-spikes/02
ENTH_REAPER_LOG=$PWD/runs/real-cycles.log ENTH_UI_LOG=$PWD/runs/real-cycles-probe.log ENTH_TEST=cycles ENTH_CYCLES=50 ENTH_PERIOD=1.0 reaper/REAPER/reaper -nosplash &
python3 -I scripts/sampler.py $! 70 1 runs/real-cycles-samples.jsonl
```

Did every open draw? Did Xorg survive? The samples give PSS and threads per cycle (under Xvfb: about 77 KB per cycle, threads constant).

### D4. Intel desktop (needs sudo and a reboot)

Only if Xorg crashes again or first opens stay blank: `sudo prime-select on-demand`, reboot, repeat B2 and C1. It tells whether the crash and the blank first frame belong to the NVIDIA X driver. `sudo prime-select nvidia` and a reboot undo it.

## Not ready yet

- **ABX "sounds the same" on the solver's results** (the G0 search gate: at least 14 of 20 judged the same). No ABX tool exists; an agent has to build one first ([metric.md](metric.md#not-done)).

## Decisions only you can make

1. **Slot names: settled (2026-10-19).** A vendored fork of nice-plug and baseview under `vendor/` with five patches (slot rename, `editor_closed`, baseview's xkb unref, keyboard focus only while typing, and a lost note-on in the CLAP wrapper), behind `[patch.crates-io]` and built by lane FK ([ADR-0001](../adr/0001-plugin-shell.md), `vendor/README.md`). CLAP names follow the patch in REAPER, and so do VST3 names: REAPER ignores the title-change message but reads VST3 titles live (lane PL measured both, 2026-10-20). Nothing to decide; check 1.8 of the [P1b owner sheet](../p1b-owner.md) is the real-display confirmation.
2. **The look:** palette, density, knob size (36 px in a 56 px cell), Inter ([design-system.md](../design-system.md)). Anything to change becomes a token edit plus a new gallery snapshot. Input: B1.
3. **Scope rate:** keep `motion.scope_hz` at 60 (about 30 fps under baseview 0.4.1) or drop it to 30 (about 20 fps, a third less drawing). Input: C2.
4. **BSL-1.0: accepted (2026-10-19)** for the Windows-only `clipboard-win` and `error-code` (scoped exceptions in `deny.toml`) and for ported source (2026-10-19 notes in [ADR-0004](../adr/0004-licensing.md)). Nothing to do unless you want to revisit it.
5. **Upstream reports (optional).** RustAudio's AI policy rules out agent-written contributions, so anything filed has to be in your own words. Candidates, none filed (the vendored fork carries every patch either way; upstream status checked 2026-10-19, [ADR-0001](../adr/0001-plugin-shell.md) lane FK note): the baseview xkb keymap leak (the draft is in `_local/drafts/baseview-xkb-keymap-leak.md`, with the valgrind trace), `NiceEguiApp::editor_closed()` never called by nice-plug-egui, the lost note-on when `SAMPLE_ACCURATE_AUTOMATION` is on (patch 5), the warnings that pristine nice-plug 0.4.2 emits in release builds, and runtime parameter names (patch 1). Drafts for the others may live in `_local/drafts/` too (local, never committed). Post, rewrite or drop each one; the default is not to post.
