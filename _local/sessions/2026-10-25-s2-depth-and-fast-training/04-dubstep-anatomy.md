# 04: What makes a dubstep sound, and the smallest set of modules that covers most of them

*Sub-researcher 04, session 2026-10-25. Running notes: `~/code/enth-scratch/2026-10-25-s2ft/04/notes.md`.*

**Labels.** Every claim carries one of these:

- **[doc]**: documented in an encyclopaedia, a manual, source code, or a magazine tutorial with exact settings;
- **[anec]**: anecdotal (producer lore, tutorials, forums);
- **[judg]**: my judgment or arithmetic.

How a named artist made a named sound is almost always **[anec]**. The tutorials are recreations by other people.

**enth units** (catalogue, read 2026-10-25):

- `svf.resonance`: `Q = 0.5·50^(r/100)`, so Q 0.707 = 8.9 %, Q 1 = 17.7 %, Q 2 = 35.4 %, Q 3 = 45.8 %.
- Route depths: cutoff 10.034 %/oct; `osc.semi` 1.042 %/st; `lfo.rate` 8.14 %/oct.
- `lfo.rate` is in Hz only; `adsr` times stop at 10 s.

**Synced rates at 140 BPM** (×1.071 for 150) **[judg]**:

| 1 bar | 1/2 | 1/4 | 3/16 | 1/4T | 1/8 | 1/8T | 1/16 | 1/16T | 1/32 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 0.583 Hz | 1.167 | 2.333 | 3.111 | 3.5 | 4.667 | 7.0 | 9.333 | 14.0 | 18.67 |

## TL;DR

1. **A dubstep sound is a mid-range bass whose timbre moves in tempo-locked patterns, over a separate, clean, mono sine sub.** It runs at about 140 BPM (132–142; riddim 140–150), half time, and brostep moved the emphasis from the sub to the mid register **[doc]**.
2. **The timbre moves in three ways, usually stacked** **[anec]**, consistent across about 12 tutorials:
   - **filter motion**: the 2008–11 wobble;
   - **source motion**: FM/PM index, wavetable position and warp, sync;
   - **serial distortion, then OTT** (multiband upward and downward compression), which makes any motion dense and "talking".

   Formant, comb and notch filters add the vowels and metal.
3. **The motion is a tempo-synced, note-retriggered, often hand-drawn LFO shape**, with rates changed per note (1 bar, 1/4, 1/8, 3/16, 1/8T, 1/16), plus hand-drawn bends of ±12–24 st. **[doc]** for Attack's recipes; **[anec]** for the general practice.
4. **What enth can make today:** a sub, a basic wobble (Hz typed for one BPM), a two-saw reese, sync screeches, sub drops. **What it lacks:** **FM/PM, real distortion and OTT**, plus a pitch-bend source (`bend` is reserved, not implemented) **[judg]**.
5. **The smallest additions, in order** **[judg]**:
   1. LFO tempo sync plus drawn or step shapes (MSEG), a one-shot mode, `bend`, and envelopes longer than 10 s;
   2. distortion (soft, hard, fold, asymmetric, bitcrush, downsample; oversampled; with `mix`);
   3. PM between oscillators, with feedback, AM and ring modes;
   4. OTT;
   5. a keytracked comb;
   6. a formant filter;
   7. a sub lane;
   8. unison plus one modulation effect.

   Estimated coverage of the 14 families: today ~20 %, after 1–4 ~68 %, after 1–6 ~80 %, after 1–8 ~90 %.
6. **"Resampling" is not a module.** In a synth it means serial chains of repeated distortion and filtering with dry and wet blends. So **every new effect needs `mix`**, and `voice_fx` must accept several instances of one kind **[judg]**.
7. **Integer-ratio PM and single-operator feedback are periodic.** So a *generated wavetable* scanned by position (position = modulation index) can stand in for static "FM from B" before runtime FM exists. It cannot do detuned or audio-rate cross-modulation **[judg]**.
8. **Tempo** **[judg]**:
   - BPM and song position in the deterministic render context;
   - dotted and triplet divisions, stepped under modulation;
   - per-note, legato and bar-locked retrigger.

   An 8-bar riser at 140 BPM is 13.7 s, longer than the 10 s ADSR limit. Mono, legato and glide already exist.

## 1. The sound families

**Context** **[doc]** (Wikipedia *Dubstep*, *Riddim*):

- The wobble is a long bass note made rhythmic by an LFO on volume, distortion or cutoff.
- Brostep (Skrillex, Rusko) is described as robotic and metal-like in its aggression.
- Riddim (named about 2012, popular about 2015) uses square waves, an LFO, flanger and chorus.
- Briddim is riddim's drums plus brostep sound design.
- Colour bass (Chime's FAQ, about 2016) adds melody to that weight. Its "chord bass" turns an atonal growl into a chord through vocoders, resonators or Pitchmap (Futureproof **[anec]**).
- "Tearout" is used inconsistently **[anec]**.

The numbers below are the matrix columns in §3.

1. **Sub.**
   - **Sound:** a felt sine at 30–60 Hz, with no motion. Common keys E, F and G put it at 41–49 Hz; MusicTech (2014 **[doc]**) puts the low-end roll-off of most club systems at about 40 Hz.
   - **Mono:** below about 100–120 Hz **[anec]**.
   - **Separation:** on its own track, clear of the distortion (see §2).
   - **Riddim variant:** a 1/4 volume LFO (EDMProd 2025 **[anec]**).
2. **Wobble (2007–11).** A saw or square whose brightness opens and closes on the beat.
   - **Oscillators:** two on the same wave, one an octave down, with a 0.10–0.15 st pitch LFO (Synthtopia 2009 **[anec]**).
   - **Filter:** a low-pass with resonance set just short of harsh; the LFO at 1/4, 1/8, 1/16 or triplets, its rate changed per note (Violet 2026 **[anec]**).
   - **Attack's "warping bass"** **[doc]**: a 3/16 sine, retriggered, with a set phase, into a serial comb; multiband distortion on the mids and highs only.
3. **Reese.** Sustained, beating, phasey.
   - **Origin:** Saunderson's "Just Want Another Chance" (1988), made on a Casio CZ-5000, a phase-distortion synth (Attack interview 2014 **[doc]**).
   - **Recipes:** ±27 c, mono (Attack 2014 **[doc]**). Or two CZ lines at 60 c with ring mod, legato, 260 ms glide, 2 unison voices, bend ±12, chorus 50 %, and the phase-distortion depth (DCW) automated as the "filter" (Attack 2015 **[doc]**).
4. **Growl.** A vocal "rawr" or "wow" in about 300 Hz–3 kHz that changes vowel within each note.
   - **Source:** a wavetable with an LFO on position and warp, and/or "FM from B" (B silent, the FM amount on an LFO).
   - **Then:** distortion, notch, band-pass or formant filters, flanger and phaser, OTT, EQ.
   - **Recipes** **[anec]**:
     - **WA 2018:** FM from B, a drawn LFO into a high-pass, softclip at 77 %, a multiband compressor;
     - **BassGorilla 2024:** bend-warped tables, a 1/2 custom LFO on position, warp and a notch at 141 Hz; a formant filter at 363 Hz, Diode distortion, flanger, phaser, a vocoder, OTT, EQ with a low cut at 153 Hz;
     - **Virtual Riot (via Slam Academy):** a saw, a band-pass at ~1 kHz, OTT ×3, a frequency shifter, Erosion.
   - **FM version** (MusicTech 2014 **[doc]**): a sine carrier at E1 with a slightly detuned triangle modulator at the same pitch; sweeping the amount gives a basic vowel.
5. **Metallic FM.** Clangy, robotic, often 1/16.
   - **The artist:** Skrillex named Massive, Reaktor and FM8 as his tools, and took up FM8 because Noisia used it (MusicRadar, about 2011 **[doc]**).
   - **Ratios:** non-integer or high ratios sound metallic; feedback sounds rough **[anec]**.
   - **Attack 2014** **[doc]**: a looping envelope on the FM level wobbles *by adding harmonics*; bit reduction adds grit.
6. **Riddim.** A short, repetitive, "wonky" square or saw bark, chopped, often in triplets.
   - **Signature effect:** comb, flanger or allpass (WA course outline **[anec]**).
   - **EDMProd 2025:** resampled chops, Roar in noise mode, OTT **[anec]**.
7. **Yoi and vowel bass.** A vowel path within one note ("yoi": i → o → i, so F2 swings high → low → high).
   - **Formant route:** a formant filter on an LFO. Kilohearts' is two peaks from a vowel pad **[doc]**.
   - **"Yoy" route:** a resonant low-pass, then downsample distortion with an LFO on drive (Futureproof, snippet only; Computer Music 2020 gives 300 Hz, resonance 50 %, downsample 50 % wet **[anec]**).
   - **Targets** (Catford 2001 via Wikipedia, male speech **[doc]**), F1/F2 in Hz:

     | i | e | a | ɑ | o | u |
     | --- | --- | --- | --- | --- | --- |
     | 240/2400 | 390/2300 | 850/1610 | 750/940 | 360/640 | 250/595 |

   - **What enth has:** its `formant` table is a crude version.
8. **Screech and laser.**
   - **Screech:** hard sync with an envelope or LFO on the slave's pitch, a fast "drop" pitch envelope, vibrato, mono legato **[anec]**.
   - **Laser:** a pitch falling over 50–150 ms **[judg]**.
9. **Foghorn** (DnB, also bass music). A huge, wide "blaaat": FM with the modulator two octaves up, a 24 dB low-pass at 200 Hz, a long filter envelope, noise unison, multiband distortion, heavy compression, width, reverb, a separate sub (Attack 2021 **[doc]**; see S9).
10. **Neuro.** A churning, phasey, metallic mid bass with moving notches and bends.
    - **Attack 2015** **[doc]**: bend-warped tables 10 c and 24 st apart; noise at 33 %; a narrow band-reject; mono legato; bend ±24; a 1-bar LFO on the positions (in opposite directions) and the cutoff; tube distortion; a frequency shifter; a 1/16 LFO on a high shelf.
    - **nick-thompson/neuro** (MIT 2015 **[doc]**): three resampling passes of saws ~45 c apart, waveshapers, random-stepped parallel band-pass and notch filters (80 Hz–18 kHz), chorus, a 3:1 compressor and EQ.
11. **Colour and chord bass.** A growl that sounds like a chord: a vocoder, resonators or Pitchmap, or supersaw chords **[anec]**. In enth, **a keytracked comb per voice in poly mode** is the cheap route **[judg]**.
12. **Tearout and briddim.** Dense, noisy, "blown-out" 1/16–1/32 rhythms from stacked distortion, OTT, FM and chops **[anec]**.
13. **FX.**
    - **Sub drop:** a sine with a falling pitch envelope (IRPR **[anec]**).
    - **Risers:** noise and synth with a rising filter and pitch, an accelerating LFO, a growing reverb, often reversed **[anec]**.
    - **Impacts:** noise, a sub drop and reverb.
14. **Melodic.** Supersaws (unison 7 or more), plucks, chorus, delay, reverb, kick ducking **[anec]**.

## 2. Producer techniques (all [anec] unless marked)

- **Resampling.** Design → bounce → process → bounce → chop and pitch (EDMProd, DnB Forum). Each bounce freezes a modulated state into a new source; the advice is many light distortion stages. Thompson's code is a working example **[doc]**. The synth equivalent is serial chains, `mix` blends, and tables generated from processed spectra **[judg]**.
- **"FM from B".** The modulator is muted. Its pitch (0, ±12 st) and the FM amount set the timbre; an LFO or envelope on the amount makes the growl (WA; MusicTech **[doc]**).
- **OTT on everything.** An Ableton Multiband Dynamics *user preset* that Xfer turned into a free plug-in (BPB 2023).
  - Blogs give: crossovers 88.3 Hz and about 2.5 kHz; upward and downward compression per band; the high band at ∞:1, the low and mid at "1:66.7"; +5.2 dB input per band.
  - It is used one to three times per growl (Virtual Riot via Slam Academy). Zomboy uses it (Equipboard).
- **Sub and mid split.** A mono sine sub on its own track, clear of the distortion (Attack 2021 **[doc]**; Virtual Riot, BassGorilla). The mid bass is high-passed at about 100–150 Hz, and **multiband distortion leaves the lows clean** (Attack 2015, 2021 **[doc]**).
- **Drop motion.**
  - The rate changes per note or phrase: 1/4 → 1/8 → 1/16, triplets for wonk. Synced rate controls *step* through divisions (Trilian manual **[doc]**).
  - Drop forms: A–B–A–C then A–B–A–D (Virtual Riot); 8-bar drops after 4-bar false drops (EDMProd).
- **Pitch.** Hand-drawn bends of ±12 or ±24 st; mono legato glide; oscillator phase restart on each note for a consistent attack (Attack 2015 **[doc]**).
- **Texture.** White noise at about 33 % for grit and top end (Attack **[doc]**). Amplitude modulation above 20 Hz acts as a soft distortion (MusicTech **[doc]**). Drawn LFOs and looping envelopes appear everywhere (WA, BassGorilla, Attack 2014).

## 3. Feature → sound coverage matrix [judg]

Key: **E** essential, **h** helps, **-** not needed, **(have)** enth has it today.

Columns are the families in §1, by number:

- 1 Sub · 2 Wobble · 3 Reese · 4 Growl · 5 Metallic FM · 6 Riddim · 7 Yoi/vowel
- 8 Screech/laser · 9 Foghorn · 10 Neuro · 11 Colour/chord · 12 Tearout/briddim · 13 FX · 14 Melodic

| Feature | 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9 | 10 | 11 | 12 | 13 | 14 |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| FM/PM between oscs + feedback | - | h | h | E | E | h | h | h | E | h | h | E | h | h |
| AM / ring mod | - | h | h | h | h | h | - | h | - | h | - | h | h | - |
| Wavetable morph (have) + warps (bend/PD) | - | h | h | E | h | h | h | h | h | E | h | E | h | h |
| Hard sync (have) | - | - | - | h | h | - | - | E | - | h | - | h | h | h |
| Unison + detune (+spread) | - | h | E | h | - | h | - | h | h | h | E | h | h | E |
| Saturation / soft clip (svf drive is mild) | h | E | E | E | E | E | h | E | E | E | E | E | h | h |
| Hard clip / fold / asym | - | h | h | E | h | E | h | E | h | E | h | E | h | - |
| Bitcrush / downsample | - | - | - | h | h | h | E | h | - | h | - | h | h | - |
| Multiband up/down comp (OTT) | - | h | h | E | E | E | E | E | E | E | E | E | h | h |
| Comb filter (keytracked, ±fb) | - | h | h | h | h | E | - | h | - | E | E | h | h | h |
| Formant / vowel filter | - | h | - | E | h | h | E | h | - | h | h | h | - | - |
| Notch / band-pass (have) | - | h | - | E | h | h | h | h | - | E | h | h | h | - |
| Phaser | - | - | h | h | h | h | - | - | - | E | h | h | h | h |
| Flanger | - | - | h | h | h | E | - | h | - | h | h | h | h | h |
| Chorus / dimension | - | h | h | h | - | h | - | h | E | h | E | h | h | E |
| Delay | - | - | - | - | - | h | - | h | - | - | h | - | E | E |
| EQ (bells, shelves, notch) | h | h | h | E | h | h | h | h | h | E | h | h | h | h |
| Tempo sync + drawn LFO (MSEG) | h | E | h | E | E | E | E | h | h | E | E | E | E | h |
| S&H / smooth random (have) | - | - | - | h | - | h | - | h | - | E | - | h | h | - |
| Pitch env (have) + bend source | h | h | E | E | h | h | E | E | h | E | h | E | E | h |
| Envelope follower | - | - | - | h | - | - | - | - | - | - | - | h | h | - |
| Frequency shifter | - | - | - | h | h | - | - | h | - | E | - | h | h | - |
| Pitch shifter / granular | - | - | - | h | - | - | - | - | - | - | h | h | h | h |
| Resampling / sampler generator | - | h | h | h | h | E | h | - | - | E | h | E | h | - |
| Sub lane / multiband split | E | h | E | E | E | E | E | - | E | E | E | E | - | - |
| Mono / legato / glide (have) | E | h | E | h | h | h | h | E | h | E | - | h | - | h |

How to read it:

- **Essential for six or more families:** sync with drawn LFOs, saturation and clipping, OTT, FM/PM, the sub lane.
- **Narrow but cheap and reusable:** the comb (riddim, neuro, colour; also Karplus-Strong plucks and flanging) and the formant filter (growl, yoi).
- **Low value for the cost:** granular, pitch shifting, resampling.

## 4. The smallest set of new modules, in priority order [judg]

Coverage asks whether a recognisable member of each of the 14 families is reachable; partial credit means a cartoon. Check the estimates by matching the §5 sketches after each step lands.

| # | Addition | Where | Unlocks | Cumulative |
|---|---|---|---|---|
| 0 | Today | — | sub, basic wobble, two-saw reese, sync screech, sub drop | ≈3/14 (20 %) |
| 1 | **LFO sync (divisions, dotted, triplet; stepped), MSEG (8–16 points, curves, one-shot / loop / sustain-loop), `bend` source, envelope times above 10 s** | modulators | drawn per-note wobbles, rate steps, risers, bends | ≈3.5 (25 %) |
| 2 | **Distortion**: tanh, hard, sine fold, triangle fold, asym/rectify, bitcrush, downsample. Drive 0–48 dB, bias, tone, `mix`; 2–4× oversampling or ADAA; several instances | `voice_fx` (bass voices are mono, so the cost is 1–2 voices) | real wobble, reese grit, screech, riddim start, downsample vowels | ≈6.5 (45 %) |
| 3 | **PM into an oscillator**: a structural source, index 0 to ~8 rad (modulatable), feedback, modes PM / AM / ring | `osc` (or a new generator; report 06) | growl, metallic, foghorn, tearout; also e-piano and bells | ≈8.5 (60 %) |
| 4 | **OTT**: 3 bands (~88 Hz, ~2.5 kHz), upward and downward per band, depth, time, gains | `fx` (instrument-level) | the dense modern finish | ≈9.5 (68 %) |
| 5 | **Comb**: Hz or keytracked (semitone offset), feedback −99…+99 %, damping, `mix` | `voice_fx` | riddim flange filters, neuro, colour resonators, Karplus-Strong plucks, flanger (with an LFO) | ≈10.5 (75 %) |
| 6 | **Formant**: 2–3 parallel band-pass peaks, continuous `vowel` (A-E-I-O-U path or X/Y), `shift`, Q, `mix` | `voice_fx` | yoi and talking bass, articulate growls | ≈11.2 (80 %) |
| 7 | **Sub lane**: per-generator "bypass voice_fx", or 2 lanes. Fallback: a "keep lows below X Hz" crossover in the distortion | routing (report 06) | a clean sub under any growl; matching references at mix level | quality |
| 8 | **Unison** (1–8 voices, detune, mono first, stereo later) **+ one modulation effect** (chorus, flanger and phaser modes) | `osc`, `fx` | full reese, supersaws, width, neuro phasing | ≈12.5 (90 %) |
| 9 | EQ (bells, shelves, notch), synced delay, frequency shifter | `fx` / `voice_fx` | polish, neuro shifter, throws | ≈13 |

Why this order:

- **Sync and MSEG first.** No new audio DSP; they help every family and teach "1/8 wobble" instead of "4.667 Hz".
- **Distortion before FM.** It lifts the most families per unit of work; the svf's drive is a mild sigmoid capped at 24 dB.
- **FM before OTT.** OTT on a weak source only makes it louder.
- **Comb before formant.** One delay line, and it is reused for plucks and flanging.
- **Two cheap side wins:**
  - a `bend`/phase-distortion warp on `osc` (CZ, and Massive's "Bend ±", used in three of the Attack recipes);
  - a few generated "digital" tables: FM index sweeps, formant-swept saws, sparse "metal" spectra.

**Patch-model notes:**

- **`voice_fx` is serial after the generator sum.** That suits wobbles and breaks growls, whose band-pass, notch and distortion destroy the sub. Hence step 7.
- **Only global LFOs reach `fx`.** So put rhythm-critical effects (distortion, comb, formant) in `voice_fx`, and OTT and the modulation effects in `fx`.

## 5. Patch sketches (140 BPM, notes about F1–F2 = 43.7–87.3 Hz)

- `[NEW]` marks an addition from §4.
- Starting values come from the cited recipes, translated by me **[judg]**.
- The **target** line is what a test should measure.

**S1 Clean sub**
```
voice mode=legato glide=40ms polyphony=1;  osc1 sine phase_mode=reset
filter1 lp 120Hz res 8.9% drive 4dB;  env1 A2ms D0 S100% R40ms;  no reverb
riddim pump: lfo1 saw_down [NEW sync 1/4] -> osc1.level −40%
```
**Target:** f0 41–49 Hz, H2 −15 to −25 dB, nothing above 300 Hz louder than −40 dB, steady RMS. *(MusicTech 2014, Violet, EDMProd)*

**S2 Classic 1/8 wobble**
```
voice legato glide=30ms;  osc1 saw;  osc2 pulse 50% semi −12 fine +7c;  lfo2 sine 0.15Hz -> osc1.fine ±6c
filter1 lp 150Hz res 45.8% (Q3) drive 12dB keytrack 50%
lfo1 sine unipolar reset phase 270° 4.667Hz [NEW sync 1/8] -> filter1.cutoff +50% (150Hz→4.8kHz)
[NEW] dist soft 12dB (after the filter);  [NEW] ott depth 30%
macro1 -> lfo1.division [NEW stepped]: 1/4 → 1/8 → 1/8T → 1/16 per note
```
**Target:** the centroid oscillates at exactly 4.667 Hz between about 200 Hz and 2.5 kHz, in phase with note-on. *(Synthtopia 2009, Violet, Attack 2015)*

**S3 Reese**
```
voice legato glide=260ms [NEW] bend ±12;  osc1 saw fine −14c;  osc2 saw fine +14c  (beat ≈0.016·f0)
  CZ variant: osc2 +60c with [NEW] ring from osc1 25%;  or [NEW] unison 4 detune 25c
filter1 lp 700Hz res 17.7% drive 6dB;  lfo1 triangle 0.583Hz [NEW 1 bar] -> cutoff ±10% (±1 oct)
[NEW] dist soft 8dB;  [NEW] phaser 6 stages 0.2Hz fb 50% mix 40% (or chorus 50%)
[NEW] sub lane: osc3 sine;  mid path high-passed at 120 Hz
```
**Target:** amplitude and spectral beating at 1–2 Hz; a sustained, phasey low-mid band. *(Attack 2014 ±27 c; Attack 2015 CZ; Thompson ≈45 c)*

**S4 FM growl (+ metallic variant)**
```
voice legato glide=25ms [NEW] bend ±12
osc1 table basic position 60%;  osc2 sine semi 0 (1:1; variant +12 = 1:2) level 0
[NEW] osc1.pm_from=osc2 index 1.0rad
[NEW] mseg1 1/4 synced, retrigger: 0 → 1 (log, 60%) → 0.4 → 0.8
  -> osc1.pm_index +4rad, osc1.position ±25%, osc2.semi +7st (7.3%)
[NEW] dist1 soft/tube 18dB → filter1 bp 900Hz res 35.4% (mseg1 -> cutoff ±15%)
  → [NEW] formant a→o (mseg1 60%, mix 40%) → [NEW] dist2 sine-fold 8dB mix 40%
  → [NEW] ott 60% → [NEW] eq low cut 150Hz, bell −6dB @400Hz;  sub lane sine
metallic: osc2 semi +6 (ratio ≈1.414) or +19.02 (3:1), feedback 40%;
  adsr2 A0 D120ms S30% -> pm_index;  mseg 1/16;  [NEW] bitcrush 8 bit mix 30%;  [NEW] comb keytrack +19st fb +60%
```
**Target:** most energy in 300 Hz–3 kHz; a formant-like peak that moves once per 428 ms; harmonic (1:1) or inharmonic (√2) partials. *(WA 2018, BassGorilla 2024, MusicTech 2014, Attack 2014)*

**S5 Riddim square** (riddim usually runs at 145–150 BPM)
```
voice mono;  osc1 pulse 50% semi +12;  osc2 pulse 50% semi +12 fine +8c
filter1 lp 900Hz res 28% drive 6dB;  [NEW] comb keytrack +12st fb +70% damp 6kHz mix 50%
lfo1 square unipolar [NEW 1/8T] (7.0Hz) -> osc levels −100% (triplet chops)
lfo2 triangle [NEW 1/4] -> comb.freq ±3st
[NEW] dist hard 9dB;  [NEW] ott 100% time 30%
sub lane: osc3 sine at the root, lfo3 saw_down [NEW 1/4] -> level −40%
```
**Target:** triplet gating at 7.0 Hz (7.5 Hz at 150 BPM); comb peaks at multiples of 2·f0; a flat, dense mid spectrum. *(Wikipedia riddim, WA outline, EDMProd)*

**S6 Yoi**
```
voice legato glide=40ms;  osc1 saw;  osc2 table formant position 40%
A) [NEW] formant: F1/F2 path i(240/2400) → o(360/640) → i(240/2400)
   [NEW] mseg1 one-shot 1/4 (428ms): 0 → 1 at 45% → 0 -> vowel;  adsr2 A0 D90ms S0 -> osc1.semi +2st
B) filter1 lp 300Hz res 50%;  [NEW] dist downsample mix 50%;  mseg1 -> dist.drive 0 → 70%
[NEW] ott 70%;  sub lane sine
```
**Target:** F2 2.4 kHz → 0.64 kHz → 2.4 kHz within 428 ms; F1 240–360 Hz. *(Catford; Kilohearts; Futureproof snippet; MusicRadar 2020)*

**S7 Screech / laser**
```
screech: voice legato glide=60ms;  osc1 saw level 0 (sync master);  osc2 saw sync on semi +12
  adsr2 A0 D250ms S30% R100 -> osc2.semi +24st (25%);  lfo1 sine [NEW 1/16] -> osc2.semi ±1.6%
  filter1 hp 300Hz;  [NEW] dist hard 12dB;  [NEW] ott 70%;  reverb wet −14dB decay 1.2s
laser: osc1 sine;  adsr2 A0 D80ms S0 -> semi +36st (37.5%);  optional [NEW] pm ratio 2, index env D60ms
```
**Target:** the sync peak sweeps 2 octaves down over about 250 ms; the laser's f0 falls 3 octaves in 80 ms. *(Screech Machine, ADSR, ask.audio)*

**S8 Neuro** (Attack 2015 mapped, plus Thompson)
```
voice legato glide=150ms [NEW] bend ±24;  osc phase_mode=reset
osc1 table position 45% semi −12 fine +10c [NEW warp bend 0%]
osc2 table position 40% semi −24 [NEW warp bend 50%];  noise1 white 33%
filter1 notch 600Hz res 45.8% (Q3);  filter2 bp 1kHz res 12% (Q 0.8, Thompson)
lfo1 sine [NEW 1 bar] -> osc1.position +33%, osc2.position −33%, filter1.cutoff +25%
lfo2 sample_hold [NEW 1/8] -> filter2.cutoff ±30% (±3 oct)
[NEW] dist tube 20dB mix 60%;  [NEW] chorus mix 33%;  [NEW] freqshift +40Hz mix 50%
[NEW] eq high shelf 6kHz <- lfo3 sine 1/16 +4dB;  sub lane sine
```
**Target:** a notch that travels across a bar; random band steps on the 1/8 grid; 10 c beating. *(Attack 2015; nick-thompson/neuro)*

**S9 Foghorn** (Attack 2021 mapped)
```
voice legato polyphony=1;  osc1 table basic position 50% [NEW] pm_from=osc2 index ≈2rad
osc2 sine semi +24 (ratio 4) level 0
filter1 + filter2 lp 200Hz res 23% (≈24 dB/oct)
env1 amp A0 D17s [NEW >10 s] S0 R850ms
env2 A23ms D4.2s S0 -> both cutoffs +30% (3 oct), osc1.position +15%
[NEW] unison 3 (noisy detune 30%);  [NEW] dist keep-lows<200Hz, mids hard 18dB
comp −28.6dB 4.85:1;  [NEW] eq +3dB @4.85kHz, high shelf −6dB;  reverb wet −12dB low_cut 300Hz;  sub lane sine
```
**Target:** the cutoff opens over 23 ms and closes over 4.2 s; FM sidebands at multiples of f0; wide. *(Attack 2021)*

**S10 FX: sub drop and riser**
```
sub drop: osc1 sine C2 (65.4Hz);  adsr2 A0 D1500ms S0 -> semi +24st (261.6 → 65.4 Hz, RC curve)
  [NEW] dist soft 6dB;  noise1 white 60% with env D120ms;  reverb wet −6dB decay 2.5s
riser (4 bars = 6.86s): noise 70% + osc1 saw [NEW unison 7 ±25c]
  env2 A6860ms S100% -> hp cutoff 200Hz → 6.4kHz (+50%), osc1.semi +12st (12.5%)
  lfo1 [NEW 1/4] -> filter2.cutoff ±1 oct;  env2 -> lfo1.rate +24.4% (→ 18.67Hz = 1/32)
  an 8-bar riser (13.7 s) needs envelopes above 10 s [NEW];  reverb swell needs global mods on fx
```
**Target:** monotonic rises in centroid and pitch; amplitude modulation accelerating from 2.3 to 18.7 Hz. *(IRPR, SOS)*

## 6. Beyond dubstep

The same set still leaves these uncovered **[judg]**:

- **Pads:** stereo voices (unison spread), a lush modulated reverb (the roadmap's CloudSeed port), LFO fade-in and delay, multi-stage envelopes, an ensemble chorus.
- **Keys and bells:**
  - velocity routes to cutoff and modulation depth (not only envelope scaling);
  - FM with more operators: an electric piano or bell wants 3–4 operators with per-operator envelopes and ratios such as 1:1 and 14:1. Two-operator PM with an envelope on the index gets a basic DX-style e-piano.
- **Plucks:** the comb filter doubles as a Karplus-Strong engine. Delay and EQ help.
- **Brass and leads:** delayed vibrato, velocity-tracked filter envelopes, aftertouch or MPE (roadmap).
- **Realism** (piano, acoustic drums, voice): a sampler or granular generator and physical models, outside this set.

So the plan generalises if steps 8–9 are kept, and if **velocity, aftertouch and LFO fade-in** ship alongside MSEG.

## 7. Tempo, mono and glide

**Facts** **[doc]**:

- Tempos are in §1.
- Tutorial divisions are 1 bar to 1/16, plus 3/16 and triplets.
- Phase Plant's LFO has a synced mode and a trigger menu of Auto, Never, Always and Legato (Kilohearts docs).
- Serum's LFOs are drawn on a grid, run up to 100 Hz and sync as slow as 32 bars (SOS 2015).

**What enth needs** **[judg]**:

1. **BPM and song position in the render context.** It must be deterministic like the render seed: in the render spec, challenge codes and training data, default 140. Typing Hz works at a fixed tempo, so sync blocks presets, lessons and labels, not single-sound matching.
2. **`sync` + `division` (8 bars … 1/64) + `feel` (straight, dotted, triplet)**, stepped under modulation like synced rate knobs (Trilian). A continuous multiplier can come later.
3. **Trigger modes:**
   - per note (have);
   - legato (no retrigger on overlapping notes, for `voice.mode=legato`);
   - global, with phase **locked to bar position**.
4. **One-shot (envelope) mode** for LFO and MSEG (Serum's ENV, Vital's envelope mode), for yoi vowels, pitch dives and foghorn sweeps.
5. **MSEG.** 8–16 points, a curve per segment, a 1/16 grid, length synced (1/4–4 bars) or in ms; loop, one-shot or sustain-loop. It is the biggest single "dubstep" modulator.
6. **Envelope times above 10 s, or in beats.** An 8-bar riser is 13.7 s; the foghorn's decay is 17 s.
7. **Pitch.**
   - Implement `bend` (±2/12/24 st).
   - Glide exists (mono, legato, always or legato-only, 0–10 s); bass glides run 25–260 ms. A constant-time vs constant-rate option could follow.
   - `phase_mode=reset` already gives the tutorials' "restart via gate".
8. **Kick ducking.** Inside one instrument, a synced volume MSEG covers it.

## 8. Gaps

- **Not read:** videos and paid courses (Virtual Riot, SeamlessR, Seed to Stage, Kilohearts' YouTube, Zomboy, Excision, the EDMProd and WA courses). Claims attributed to them are secondary **[anec]**.
- **Failed or partial:**
  - MusicRadar's OTT page and ADSR's growl page were truncated (I curl'd the Skrillex and 2020 texts);
  - PressReader returned 403;
  - `blog.dubspot.com` URLs now serve unrelated content, so I did not use them;
  - the Futureproof "yoy" URL redirects (snippet only);
  - the Serum manual loads dynamically, so Serum is cited via Sound On Sound.
- **OTT preset values** come from one or two blogs. Read the Ableton preset or measure Xfer OTT before fixing defaults.
- **No measurement of records.** Downloading audio is not allowed, so the §5 targets come from recipes. Next step: `enth describe` on owner clips.
- **Definitions.** "Yoi", "tearout" and "briddim" have no authoritative definitions. The family boundaries and coverage numbers are mine. Validate them by matching the §5 sketches after each step lands.
- **Licences.** Vital and Surge XT are GPL: ideas only. Algorithms are report 05's job.

## 9. Sources

All accessed 2026-10-25. **[doc]** unless marked **[anec]**.

- Wikipedia: [Dubstep](https://en.wikipedia.org/wiki/Dubstep), [Riddim](https://en.wikipedia.org/wiki/Riddim_(genre)), [Formant](https://en.wikipedia.org/wiki/Formant) (Catford 2001)
- [Chime FAQ](https://chimetunes.net/faq); [Futureproof: colour bass](https://genreexplorer.futureproofmusicschool.com/genre/colour-bass) **[anec]**
- Attack Magazine:
  - [Foghorn Bass](https://www.attackmagazine.com/?p=99684) (Douglas, 2021-10-13);
  - [Twisted Neuro Bass](https://www.attackmagazine.com/technique/synth-secrets/twisted-neuro-bass/) (2015-08-19);
  - [Dirty FM Bass](https://www.attackmagazine.com/technique/tutorials/dirty-fm-bass/) (2014-09-19);
  - [Reese with Virtual CZ](https://www.attackmagazine.com/technique/synth-secrets/reese-bass-with-virtual-cz/) (2015-04-30);
  - [Reese Bass Redux](https://www.attackmagazine.com/technique/tutorials/reese-bass-redux/) (2014-09-04);
  - [Just Want Another Chance](https://www.attackmagazine.com/technique/deconstructed/reese-just-want-another-chance/) (2020-08-24);
  - [Warping Bass](https://www.attackmagazine.com/technique/synth-secrets/warping-bass-in-ni-massive/) (2015-09-30)
- [MusicTech, Dirty Dubstep pt 1](https://musictech.com/guides/essential-guide/dirty-dubstep-1/) (2014-10-02)
- MusicRadar: [Skrillex interview](https://www.musicradar.com/news/tech/interview-skrillex-talks-production-plug-ins-and-power-edits-545529) (about 2011); [Serum effects bass](https://musicradar.com/how-to/how-to-use-serums-effects-to-boost-a-bass-sound) (2020-06-25) **[anec]**
- [nick-thompson/neuro](https://github.com/nick-thompson/neuro) (MIT 2015, code)
- [Sound On Sound, Serum review](https://www.soundonsound.com/reviews/xfer-records-serum) (Nagle, 2015-07)
- Kilohearts: [modulation docs](https://kilohearts.com/docs/modulation), [Formant Filter](https://kilohearts.com/products/formant_filter), [Comb Filter](https://kilohearts.com/products/comb_filter)
- [Spectrasonics Trilian, LFO rate](https://support.spectrasonics.net/manual/Trilian/1.5/en/topic/lfo-rate); [Vital forum](https://forum.vital.audio/t/lfo-rate-automation-between-straight-and-triplet/7509) **[anec]**
- **[anec]**:
  - [BassGorilla growl](https://bassgorilla.com/serum-tutorial-super-growl-bass-in-ableton-live/) (2024-08-29);
  - [WA Jauz wobble](https://blog.waproduction.com/how-to-create-jauz-wobble-bass) (2018-02-13);
  - [WA riddim course](https://www.waproduction.com/videos/view/riddim-bass);
  - [ADSR Skrillex growl](https://www.adsrsounds.com/serum-tutorials/making-a-skrillex-growl-in-serum/);
  - [EDMProd riddim](https://www.edmprod.com/how-to-make-riddim/) (2025-05-23);
  - [EDMProd OTT](https://edmprod.com/ott-plugin);
  - [Violet wobble](https://violetrecording.com/how-to-make-a-wobble-bass/) (2026-06-04);
  - [Synthtopia wobble](https://www.synthtopia.com/content/2009/01/09/tutorial-1-dubstep-wobble-bass-with-massive/) (2009);
  - [Futureproof yoy](https://futureproofmusicschool.com/blog/how-to-make-yoy-bass-in-serum) (now redirects);
  - [Slam Academy, Virtual Riot](https://slamacademy.com/blog/virtual-riot-masterclass);
  - [BPB, OTT](https://bedroomproducersblog.com/2023/07/31/xfer-records-ott-2/);
  - [Mixelite, OTT](https://mixelite.com/blog/ott-compression/);
  - [Ali Jamieson, reese](https://alijamieson.co.uk/2021/08/14/reeses-pieces-how-to-create-kevin-saundersons-legendary-bass-patch/);
  - [Armada, mono < 120 Hz](https://www.armadamusic.com/university/music-production-articles/how-to-mix-your-kick-and-bass-5-must-know-methods);
  - [Screech Machine](https://www.pluginboutique.com/products/18010-Screech-Machine);
  - [ADSR screech](https://www.adsrsounds.com/serum-tutorials/hardstyle-screech-sound-design-with-serum/);
  - [IRPR sub drop](https://sounddesign.irpr.agency/glossary/sub-drop/);
  - [Equipboard Zomboy](https://equipboard.com/zomboy);
  - [DnB Forum](https://dnbforum.com/threads/resampling-sound-manipulation.158835/)
- enth: `crates/enth-patch/src/catalogue/*.rs`, `docs/dsp/svf.md`, `crates/enth-patch/src/addr.rs` (reserved `bend`)
