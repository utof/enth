# P1b owner sheet: your two sittings

- Date: 2026-10-20 (P1b lane INT). Every P1b lane is in, lane RT's real-time measurement included, and every automated check is green ([p1b-g1.md](p1b-g1.md)); what is left needs your eyes, ears and hands.
- Two sittings: **owner build 1** (the real display, about 45 min) and **owner build 2** (G1: four sounds, 60–90 min, plus the real-audio check, 30 min). Do them in either order, in as many sittings as you like.
- This page replaces sections C1, C3, C4, C5 and C6 of the Phase 0 [owner checklist](spikes/owner-checklist.md) (they used the spike plugins, which are retired). Its listening sections (A) and the dev window (B) still stand.

## Before each sitting

- **Save your work and close other projects.** The NVIDIA 595 X driver crashed Xorg on 2026-10-19 while plugin windows opened and closed. A crash ends the desktop session and every open application.
- **Wait 10 s between opening windows**, and do one step at a time.
- If Xorg crashes: log back in, write down the step and the time, and stop there.
- Keep a note as you go: `~/code/enth-scratch/owner/p1b-notes.md`, one line per check (the template is [at the end](#how-to-report)). Give the file to an agent; it fills in the G1 table and the docs.

## Install (about 10 minutes, no windows)

```bash
cd ~/code/enth
cargo xtask install --package enth-plugin --release    # builds, then copies enth-plugin.clap to ~/.clap and enth-plugin.vst3 to ~/.vst3
```

Start REAPER (`~/apps/reaper-7.82/reaper_linux_x86_64/REAPER/reaper`). Once: *Options → Preferences → Plug-ins → CLAP*, check that `~/.clap` is scanned, press *Re-scan*; the same for VST3 with `~/.vst3`. The plugin is **"CLAPi: enth (enth)"** and **"VST3i: enth (enth)"**. Spike plugins from Phase 0 ("enth spike-ui", "enth spike-shell") may still be listed: ignore them.

For live notes: arm the track, set its input to your MIDI keyboard or REAPER's virtual keyboard (*View → Virtual MIDI keyboard*, Alt+B), and turn monitoring on.

A fresh enth now plays "Init" (every P1 module; osc 2, noise, compressor and reverb switched off), and the editor can add and remove modules ("+ Add" in the header, "…" on each card; 2026-10-20, lane AM). The starter project below is still the quickest way to get four named tracks for checks 1.8, 1.10 and 1.11 (it refuses to overwrite a file):

```bash
cargo xtask starter-project --out ~/enth-g1/build1.rpp
```

## Owner build 1: the real display (about 45 minutes)

Do each check in CLAP first, then VST3 where the row says so.

| # | Check | What to do | Pass looks like | Time |
| --- | --- | --- | --- | --- |
| 1.1 | First open, ×10 per format | Insert "CLAPi: enth (enth)" on a new track (the editor opens). Close the FX window, wait 10 s, insert again. 10 times; then 10 times with the VST3 | Every open draws the whole editor at once: never a blank, grey or black window, no crash. Write down how many of the 10 drew, per format | 10 min |
| 1.2 | Idle CPU | One editor open, nothing playing, 60 s; then the same with the editor closed. In a terminal: `pidstat -t -p "$(pgrep -x reaper)" 1 60 \| grep Average` and `nvidia-smi pmon -s um -d 1 -c 60` | Open and idle costs about what closed costs (the editor draws nothing while nothing changes): under 1 % of a core for the editor's thread, GPU SM near 0 | 4 min |
| 1.3 | Play and watch | Play notes for 30 s with the editor open, then stop. Run `pidstat` again while playing | The knob rings, tiles, scope and meters move smoothly and stop when the notes end. Say how smooth the scope looks: the editor asks for 60 frames a second while data changes, which baseview delivers as about 30 (26–28 measured under Xvfb, 2026-10-20). Under 5 % of a core and under 5 % GPU SM while playing | 3 min |
| 1.4 | Resize | Drag the FX window's corner bigger and smaller | The editor stays drawn and usable (it may keep its own size); no garbage, no crash. Note what it does | 2 min |
| 1.5 | Space after clicking the editor | Click an empty part of the editor, press Space. Click a knob, press Space. Right-click a knob, *Type value…*, type `a b`, then Escape. Click REAPER's arrange view, press Space | Space starts and stops REAPER's transport after each click in the editor. While typing, `a b` appears in the field and the transport does not move. (This decides whether the keyboard-focus patch works in REAPER's windows) | 3 min |
| 1.6 | HiDPI | `echo "Xft.dpi: 192" \| xrdb -merge`, restart REAPER, open enth. Afterwards `echo "Xft.dpi: 96" \| xrdb -merge` and restart REAPER again | The editor is sharp and whole: REAPER does not clip it to its top-left quarter (baseview #359 under Xvfb) | 6 min |
| 1.7 | Save prompt after an editor edit, both formats | New project, insert enth, save the project. Turn one knob in the editor (not a macro: osc 1's Level). Then *File → Close project* | REAPER asks whether to save (the edit marked the project dirty). Then the same with the VST3 | 3 min |
| 1.8 | Renamed slot names, both formats | In `~/enth-g1/build1.rpp` (*File → Open project*), on the `bass` track: right-click the filter's Cutoff, *Automate*. Open the FX's parameter list (the *Param* button in the FX window, or the track's envelope button) | Slot 1 reads "Filter 1 · Cutoff" (or the card's label), not "Slot 1": CLAP after a moment (under a second), VST3 at once. Save, close and reopen the project: the names are still there. For the VST3, insert "VST3i: enth (enth)" on a new track and automate osc 1's Level the same way | 4 min |
| 1.9 | Hidden MIDI CC entries (VST3) | In the VST3's parameter list and its envelope menu, scroll past the 72 parameters | Information, not pass or fail: do "MIDI Ch. 1 CC 1" … "MIDI Ch. 16 CC 1" (16 entries) show? Note where. Say if they bother you | 2 min |
| 1.10 | Mod wheel from your controller, both formats | In `build1.rpp`, drag the *Wheel* tile's ⊕ onto the Cutoff (this makes a route), then play and move the hardware mod wheel; for the VST3, a new track with the VST3 and osc 1's Level as the target | The sound follows the wheel in CLAP and in VST3. Skip if you have no controller with a wheel | 3 min |
| 1.11 | Drag-to-modulate | In `build1.rpp`: drag *LFO 1*'s ⊕ onto the Cutoff and drop it; drag the orange dot on the knob's ring up and down; click *Undo*. Then drag *Env 1*'s ⊕ onto a reverb knob (the reverb card starts switched off; the refusal shows either way) | A drop makes an orange arc and a route in the overview; the dot sets the depth; Undo removes the route. The envelope on the reverb is refused, with the reason on the status line. Everything stays smooth | 4 min |
| 1.12 | CLAP offline bounce of short notes | A MIDI item with 16 short notes (1/16 notes at 120 BPM) on the CLAP. *File → Render*, full-speed offline, to a WAV; import the WAV | All 16 notes are in the WAV (count the blobs in the waveform). Stock nice-plug restarted the plugin about 30 ms into a CLAP render and lost a note; the fork's patch 9 stops that | 4 min |

## Owner build 2: G1, four sounds you like (60–90 minutes, one or two sittings)

The last row of G1: **"you can make a bass, pad, pluck and lead you like"** through the editor.

1. **Start from the starter project** (a fresh instance plays the same "Init" patch, and you can add or remove modules, but the project gives you the four named tracks). The starter project has four tracks, `bass`, `pad`, `pluck` and `lead`, each with enth (CLAP) holding every P1 module: osc 1 into the filter with env 1 on the amp; osc 2, the noise, the compressor and the reverb switched off (switch a card on with the toggle in its title); env 2 and both LFOs do nothing until you drag them onto a knob. Make it once (it refuses to overwrite a file):

   ```bash
   cd ~/code/enth && cargo xtask starter-project --out ~/enth-g1/g1-sounds.rpp
   ```

   Then open `~/enth-g1/g1-sounds.rpp` in REAPER.
2. Make each sound in the editor until you like it. The editor has Undo and Redo in its header, right-click → *Type value…* (units like `1.2kHz`, `A4`, `-12 st`), drag-to-modulate from the source tiles, the eight macros, and the status line, which names what you hover.
3. Save the project often (Ctrl+S).
4. For each sound, note: do you like it (yes or no), one line on what it is, and what got in your way (missing modules or controls, confusing layout). The last part feeds the layout pick and S1.
5. Give an agent the project's path. It runs `cargo xtask extract-presets ~/enth-g1/g1-sounds.rpp`, which writes `content/presets/{bass,pad,pluck,lead}.patch.json` as the plugin saved them, each named and tagged after its track ("Bass", tag `bass`; not "Init"), checks them and commits them ([content/presets/README.md](../content/presets/README.md)).

G1's row passes when you like all four. If one is not there yet, say what is missing: that is a finding, not a failure of yours.

## Real audio and xruns (about 30 minutes, with owner build 2)

The G0 xrun row: 8 instances at a 64-sample buffer, editors closed against editors open. Your PipeWire forces a minimum quantum of 1024, so force 64 for the test and put it back afterwards.

```bash
pw-metadata -n settings 0 clock.force-quantum 64     # 64-sample buffer
pw-top                                                # QUANT should read 64; if not: pw-metadata -n settings 0 clock.min-quantum 64
pw-jack ~/apps/reaper-7.82/reaper_linux_x86_64/REAPER/reaper
```

1. *Preferences → Audio → Device*: JACK.
2. enth on 8 tracks, each holding a chord (a looped MIDI item), editors closed, for 10 minutes. Note REAPER's xrun count (*View → Performance Meter*) and the ERR column in `pw-top`.
3. Open the editors (as many of the 8 as fit) for another 10 minutes. Note the same numbers. Listen for clicks.
4. Pass: no extra xruns with the editors open.
5. Afterwards: `pw-metadata -n settings 0 clock.force-quantum 0` (and `pw-metadata -n settings 0 clock.min-quantum 1024` if you changed it).

Lane RT measured `process()` itself in the test host under real-time priority (p99 about 116 µs of the 1,333 µs deadline, [benchmarks.md](benchmarks.md#lane-rt-the-plugin-measured-2026-10-20)); this check is the real device and the real desktop.

## Optional

| What | Time | Default if you skip it |
| --- | --- | --- |
| Bitwig poly-mod: start the 30-day trial (it needs an account), per-voice modulation of Macro 1, save and reload | 45 min | Deferred to S1 |
| Layout pick: read the Claude Design variants when they arrive; pick one, or "keep Lanes" | 20–30 min | Lanes |
| Upstream issues: post, rewrite or drop the drafted reports in `_local/drafts/` (RustAudio wants them in your own words) | 10 min | Not posted |

## How to report

One line per check in `~/code/enth-scratch/owner/p1b-notes.md`, then hand the file to an agent:

```text
date: 2026-10-__   REAPER 7.82   Xft.dpi 96   prime-select nvidia   load (uptime): __
1.1  CLAP 10/10 drew, VST3 10/10 drew            pass
1.2  open idle __ % (enth thread), closed __ %, SM __ %
1.3  smooth? __   playing __ % CPU, __ % SM
1.4  resize: __
1.5  space after click: yes/no; typing a b: yes/no; arrange view: yes/no
1.6  192 dpi: sharp/blurry, whole/clipped
1.7  save prompt: CLAP yes/no, VST3 yes/no
1.8  names: CLAP yes/no (delay __), VST3 yes/no; after reopen yes/no
1.9  MIDI CC entries shown: yes/no, where: __
1.10 wheel: CLAP yes/no, VST3 yes/no (or skipped)
1.11 drag-to-modulate: ok / what went wrong
1.12 bounce: __ of 16 notes
G1   bass yes/no __ | pad yes/no __ | pluck yes/no __ | lead yes/no __ | project: ~/enth-g1/...
C3   closed: __ xruns, ERR __ | open (__ editors): __ xruns, ERR __ | clicks? __
crashes: none / step and time
```

The agent updates the owner rows of [p1b-g1.md](p1b-g1.md), the G0 rows in [benchmarks.md](benchmarks.md#ui-thresholds) and [roadmap.md](roadmap.md), the real-display notes in [ADR-0002](adr/0002-ui-toolkit.md), and the open checks in [phase-1b.md](phase-1b.md#open-checks).
