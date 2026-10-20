# content/presets-candidates

**Drafts, not factory content.** Twenty-two candidate presets that agents made through the `enth` CLI for gate GS1 ("ten presets you'd actually use": your four G1 sounds plus the six you keep, [docs/phase-2.md](../../docs/phase-2.md)). Nothing here ships: only `content/presets/` is embedded in the plugin. You audition these, record your pick in [PICKS.md](PICKS.md), and one command promotes the kept ones (below).

## How they were made

- Each one is a list of ops applied with `enth apply` to the starter patch (`xtask/src/preset/starter.patch.json`: every P1 module, the extras switched off), so the whole instrument is there to tweak. Then `enth render` and `enth describe` checked it, and `cargo xtask preset-add` wrote it here in the exact preset form (`Patch::to_json()`, a name, tags from the factory vocabulary, the engine stamp). `enth diff xtask/src/preset/starter.patch.json <file>` lists the ops behind any of them.
- Folders are the type tag: `bass`, `lead`, `pad`, `pluck`, `keys`, `arp`, `fx`, `texture`.
- **Macros** follow the G1 naming: 1 **Bright**, 2 **Pluck**, 3 **Space**, 4 **Drive**. They sit at 0 in the saved sound and add as you turn them up. Pluck makes the sound shorter and snappier (less sustain, a shorter decay or swell). Space raises the reverb's wet level; the basses have no Space. Drive raises the filter's drive *and* the compressor's makeup, with the compressor on at 1:1 as a clean gain stage (Poly Stab uses it for real): the filter's saturator loses about 1 dB of level per dB of drive, so drive alone would mostly make the sound quieter. Macro 5 is **Rate** on the three modulation showcases (Wobble Machine, Gate Keeper, Octave Arp).
- **Headroom.** Every candidate peaks at or below −1.5 dBFS on a single note and on a four-note chord (C–E–G–B) at velocity 127, from C1 to C6, over several render seeds. Poly sounds are gain-staged for chords, so their single notes are quieter (about −10 dBFS). The kick, snare and riser are mono (a new note retriggers), like drum voices.
- Short and percussive sounds are kept as they are; nothing was filtered by duration.

## The candidates

| Name | Folder | Tags | Sound | Macros |
| --- | --- | --- | --- | --- |
| Sub Floor | bass | bass, clean, dark, mono | Round sine sub, mono, short legato glide | Bright: adds a triangle an octave up · Pluck: decays · Drive: saturates |
| Acid Snap | bass | bass, gritty, hard, mono | Resonant saw, the filter snaps shut on each note; slides | Bright: opens the filter · Pluck: shorter snap · Drive: grit |
| Warm Reese | bass | bass, warm, smooth, mono | Two detuned saws, slow beating, warm low-pass | Bright: opens the filter · Pluck: decays · Drive: thickens |
| Wobble Machine | bass | bass, gritty, rhythmic, mono | Saw + sub square, LFO wobbling a resonant low-pass at 2 Hz | Bright: raises the filter · Pluck: decays · Drive: grit · Rate (5): faster wobble |
| Solar Lead | lead | lead, bright, mono | Two bright detuned saws, light vibrato, legato glide | Bright: opens fully · Pluck: decays · Space: hall · Drive: bite |
| Pixel Square | lead | lead, hollow, hard, vintage | Chiptune square, hard on and off, no movement | Bright: narrows the pulse · Pluck: decays · Drive: harder |
| Sync Tear | lead | lead, bright, gritty, metallic | Hard-synced saw, pitch sweeping down from two octaves up | Bright: higher sync pitch · Pluck: shorter sweep · Space: room · Drive: grit |
| Breath Flute | lead | lead, soft, smooth | Triangle with breath noise, soft swell, vibrato | Bright: brighter, more breath · Pluck: decays, faster onset · Space: hall · Drive: rougher |
| Velvet Pad | pad | pad, warm, smooth, wide | Detuned saws, warm low-pass, slow swell, hall, spread by pitch | Bright: opens the filter · Pluck: faster, less sustain · Space: more hall · Drive: thicker |
| Aurora | pad | pad, bright, evolving, wide | Lush detuned pad, filter breathing at 0.8 Hz, big hall | Bright: opens the filter · Pluck: faster, less sustain · Space: more hall · Drive: thicker |
| Night Drift | pad | pad, dark, evolving, soft | Dark drone: triangle + sub saw + pink breath, wandering filter | Bright: lifts the filter · Pluck: faster, less sustain · Space: more hall · Drive: thicker |
| Gate Keeper | pad | pad, rhythmic, hard | Trance gate: saw and octave chopped at 8 Hz by a square LFO | Bright: opens the filter · Pluck: shorter notes · Space: room · Drive: grit · Rate (5): chop speed |
| Glass Pluck | pluck | pluck, bright, clean | Short bright pluck: saw + pulse, fast filter envelope | Bright: opens the filter · Pluck: shorter · Space: room · Drive: bite |
| Poly Stab | pluck | pluck, hard, wide | Chord stab: detuned saws, fast filter envelope, compressor glue | Bright: opens the filter · Pluck: shorter · Space: room · Drive: grit |
| Drawbar Organ | keys | keys, clean, vintage, warm | Organ wavetable, instant on/off, slow rotary pan | Bright: more drawbars · Pluck: percussive · Space: room · Drive: overdrive |
| Tremolo Keys | keys | keys, soft, warm, vintage | E-piano-like: sine + fading bell partial, 5 Hz tremolo | Bright: more bell · Pluck: shorter · Space: room · Drive: warmer |
| Soft Bell | keys | keys, metallic, soft | Shimmering sine pair + inharmonic partial, long ring | Bright: louder partial · Pluck: shorter ring · Space: more hall · Drive: grit |
| Octave Arp | arp | arp, rhythmic | Hold a note: root/octave arpeggio, each step plucked | Bright: opens the filter · Pluck: snappier steps · Space: room · Drive: bite · Rate (5): speed |
| Thump Kick | fx | fx, hard, dark | Kick: sine with a fast pitch drop and a click; mono | Bright: more click · Pluck: shorter · Space: room · Drive: saturates |
| Snare Crack | fx | fx, noisy, hard, bright | Snare: white-noise burst over a short tonal body; mono | Bright: more sizzle · Pluck: tighter · Space: room · Drive: saturates |
| Noise Riser | fx | fx, noisy, evolving, bright | White-noise riser: swells and brightens over the note; mono | Bright: higher end point · Pluck: rises faster · Space: more hall · Drive: rougher |
| Wind Tunnel | texture | texture, noisy, evolving, soft | Wind: noise through a band-pass swept slowly by an LFO | Bright: higher band · Pluck: gusty, less sustain · Space: more hall · Drive: rougher |

### What `enth describe` reads

Rendered at the note and hold/clip lengths shown (velocity 100 for the readings; the peaks are at velocity 127). Envelope readings are `enth describe`'s, not knob values: attack to within 1 dB of the peak, sustain relative to the peak, release to 40 dB down, duration within 40 dB of the peak. "Peak" is single note / four-note chord, in dBFS; "macros at 1" is the chord's peak with every macro turned fully up.

| Name | Played | Pitch | Brightness | Attack / sustain / release / duration | Movement | Peak | Macros at 1 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| Sub Floor | C2, 2/3 s | C2 | 67 Hz | 0 ms / 0 dB / 60 ms / 2059 ms | steady | -3.1 / -3.1 | -5.3 |
| Acid Snap | C2, 2/3 s | C2 | 167 Hz | 30 ms / -3 dB / 41 ms / 2041 ms | brightness down 1.4 oct in 117 ms | -5.3 / -5.3 | -7.2 |
| Warm Reese | C2, 2/3 s | C2 | 234 Hz | 15 ms / -3 dB / 83 ms / 2072 ms | beating 0.98 Hz | -3.0 / -3.8 | -10.2 |
| Wobble Machine | C2, 2/3 s | C1 | 315 Hz | 455 ms / -8 dB / 62 ms / 2059 ms | brightness LFO 2.01 Hz, 3.5 oct; noisiness 0.40 | -2.0 / -2.1 | -5.8 |
| Solar Lead | C4, 1.5/3 s | C4 | 2.62 kHz | 7 ms / -1 dB / 421 ms / 1898 ms | beating 1.06 Hz | -3.1 / -3.2 | -9.8 |
| Pixel Square | C4, 1.5/3 s | C4 | 2.69 kHz | 0 ms / 0 dB / 10 ms / 1510 ms | steady | -5.5 / -5.5 | -7.3 |
| Sync Tear | C4, 1.5/3 s | C4 | 2.86 kHz | 4 ms / -1 dB / 363 ms / 1854 ms | steady | -3.0 / -3.2 | -3.0 |
| Breath Flute | C4, 1.5/3 s | C4 | 2.56 kHz | 116 ms / -1 dB / 435 ms / 1927 ms | noisiness 0.06 | -2.8 / -2.0 | -7.2 |
| Velvet Pad | C3, 5/8 s | C3 | 566 Hz | 561 ms / -2 dB / 1331 ms / 5944 ms | beating 1.21 Hz | -11.0 / -2.1 | -7.7 |
| Aurora | C3, 5/8 s | C3 | 1.68 kHz | 698 ms / -2 dB / 2093 ms / 7042 ms | beating 1.51 Hz; brightness LFO 0.80 Hz, 1.4 oct | -11.3 / -2.1 | -8.0 |
| Night Drift | C3, 5/8 s | C2 | 265 Hz | 989 ms / -2 dB / 1978 ms / 6811 ms | steady | -11.9 / -2.4 | -4.7 |
| Gate Keeper | C3, 5/8 s | C3 | 1.22 kHz | 64 ms / -12 dB / 445 ms / 5368 ms | tremolo 8.0 Hz, 32 dB; noisiness 0.10 | -12.3 / -1.7 | -6.3 |
| Glass Pluck | C4, 1/2.5 s | C4 | 2.14 kHz | 0 ms / -55 dB / 0 ms / 534 ms | brightness down 0.6 oct in 149 ms | -11.0 / -3.1 | -4.7 |
| Poly Stab | C4, 1/2.5 s | C4 | 1.64 kHz | 0 ms / -74 dB / 0 ms / 366 ms | noisiness 0.01 | -8.3 / -3.5 | -4.3 |
| Drawbar Organ | C4, 1.5/3 s | C4 | 589 Hz | 4 ms / -1 dB / 22 ms / 1522 ms | steady | -14.1 / -2.6 | -8.0 |
| Tremolo Keys | C4, 1.5/3 s | C4 | 489 Hz | 3 ms / -10 dB / 379 ms / 1698 ms | tremolo 5.0 Hz, 5 dB | -10.2 / -1.3 | -1.2 |
| Soft Bell | C5, 3/5 s | C5 | 816 Hz | 15 ms / -78 dB / 0 ms / 1709 ms | tremolo 2.7 Hz, 9 dB | -9.4 / -1.4 | -4.9 |
| Octave Arp | C4, 2/3 s | C4 | 1.27 kHz | 50 ms / -3 dB / 423 ms / 2413 ms | brightness LFO 8.01 Hz, 1.0 oct; noisiness 0.04 | -11.0 / -2.4 | -3.1 |
| Thump Kick | A1, 0.3/1 s | A1 | 120 Hz | 0 ms / -50 dB / 0 ms / 231 ms | brightness up 0.7 oct in 192 ms | -4.9 / -5.2 | -7.0 |
| Snare Crack | A3, 0.3/1 s | tonal A3 | 11.39 kHz | 0 ms / -52 dB / 0 ms / 181 ms | noisiness 0.49 | -4.1 / -2.7 | -4.5 |
| Noise Riser | C3, 3/4.5 s | – | 6.06 kHz | 2217 ms / -1 dB / 1354 ms / 4220 ms | brightness up 3.7 oct in 2037 ms; noisiness 1.00 | -4.4 / -4.4 | -2.9 |
| Wind Tunnel | C3, 5/7 s | – | 3.95 kHz | 1535 ms / -4 dB / 2370 ms / 6989 ms | brightness LFO 0.69 Hz, 0.7 oct; noisiness 1.00 | -10.2 / -4.1 | -4.3 |

Reading notes ([docs/dsp/analysis.md](../../docs/dsp/analysis.md) has the limits):

- Wobble Machine and Night Drift read an octave low: a sub oscillator an octave down sets the waveform's period. The snare's pitch is under its noise (`tonal_pitch`).
- Soft Bell's "tremolo" is the beat between its two detuned sines. Night Drift's random filter wander is not periodic, so describe reports no movement.
- Acid Snap's 30 ms attack and Wobble Machine's 455 ms are the filter, not the amplitude envelope (both envelopes start in 2–3 ms): the resonance crosses the low harmonics, and the LFO starts with the filter closed.
- A resonant filter sweep reads as noisiness (Wobble Machine 0.40); there is no noise source in it.
- The kick and the snare are tagged `fx`: the vocabulary has no percussion type.

## Auditioning

These are patch files, so any of the usual ways works:

- **In the plugin's browser:** copy the folder into your user presets, then they show under User → Drafts with their tags:
  `cp -r content/presets-candidates ~/Documents/enth/Presets/Drafts` (or start the host with `ENTH_PRESETS_DIR` pointing here).
- **As files:** `enth render content/presets-candidates/pad/aurora.patch.json --perf note=C3,vel=100,hold=5s,dur=8s --out aurora.wav`.

## Recording the pick and promoting it

1. In [PICKS.md](PICKS.md), write `yes` in the Keep column of each candidate you keep (and a new name in Factory name if you want one).
2. An agent runs, from the repo root:

   ```sh
   cargo xtask preset-add --picks content/presets-candidates/PICKS.md --dry-run   # what it would write
   cargo xtask preset-add --picks content/presets-candidates/PICKS.md             # into content/presets/<type>/
   cargo test -p enth-plugin --test presets                                       # the factory rules
   ```

   One at a time: `cargo xtask preset-add --promote "Velvet Pad"` (a name, ignoring case, or a file stem; `--name` renames it on the way). Promotion copies the file byte for byte into `content/presets/<type>/`, refuses a name already used there (ignoring case), and keeps the candidate here unless `--move` is given.
3. **A tweaked version:** save it in the editor as a user preset (or save the REAPER project and `cargo xtask extract-presets`), then `cargo xtask preset-add <saved file> --out content/presets --name "…" --tags type,word,…` normalises it into `content/presets/<type>/`. The same command, with `--move`, renames a file in place: the owner's G1 sounds extracted from the starter project carry the starter's name ("Starter") and its `starter` tag, which the factory test refuses.

`cargo xtask preset-add --help` lists every option. The xtask's tests promote every candidate into a temporary folder and check it against the factory test's rules (`xtask/src/preset_add.rs`).
