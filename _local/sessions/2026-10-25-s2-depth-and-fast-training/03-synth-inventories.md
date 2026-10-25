# 03 — What the reference synths ship: one tiered union inventory

Sub-researcher 03, 2026-10-25. Legend: **[D]** documented (vendor manual or page), **[M]** measured by me from source or page text, **[A]** anecdotal (tutorial, review, forum), **[E]** my estimate. All version and date claims are as of 2026-10-25. Table abbreviations: PP Phase Plant, S2 Serum 2, V Vital, Pg Pigments 7, Sg Surge XT; Y has it, ~ partly or indirectly, - not found in the vendor docs, ? not verified.

## TL;DR

- **Versions today.** Phase Plant 2.4.6 (Kilohearts changelog, 2026-03-10; a free "v3" was teased in Dec 2025 but is on neither the changelog nor the blog today) [D]. Serum 2.0, manual dated 2025-03-17 [D]. Vital 1.5.5 is the latest binary I found (2024 reviews); its open repo's last commit is 2022-04-20, so source I read is a floor, not the shipping feature set [M]. Pigments 7 (2025-12-16; manual 7.0.0 dated 2026-07-28) [D]. Surge XT 1.3.x (changelog top entry 1.3.4) [D].
- **Owner's question: Phase Plant has no arpeggiator and no step sequencer.** Its manual lists neither. Nearest: the Trance Gate snapin (8 pattern slots), a stepped draw tool in its curve and LFO editors, Sample & Hold and Random modulators. Serum 2 (Arp + 12-slot Clip sequencer) and Pigments 7 (16-step, 6-track generative sequencer + arp) have both; Vital and Surge XT have neither (Surge has a 16-step LFO shape) [D].
- **Tier A core, shipped by all five:** classic + wavetable oscillators, noise, oscillator unison, hard sync, FM/PM and ring mod between oscillators, a resonant ladder-type filter and a comb, distortion with fold/clip/crush types, ADSR, LFO, drawable multi-point shapes, random/S&H, macros, MIDI sources, chorus, flanger, phaser, delay, reverb, compressor, EQ.
- **Phase Plant's edge:** any-generator-to-any-generator audio-rate modulation, up to 32 modulators of 18 kinds, per-voice effect lanes, 33 free snapins plus paid ones. **It lacks** oscillator warp menus, spectral warps, additive, modal, vocoder, arp and sequencer.
- **Dubstep (Tier B) is mostly Tier A modules used hard:** FM/PM + ring mod, diode/fold/crush distortion, formant and comb filters, multiband upward/downward compression (OTT), phase warps, tempo-synced and step-shaped LFOs, disperser. Nothing exotic is needed (anecdotal chains below; report 04 covers the anatomy).
- **Movement comes from five design choices, not slot counts:** per-voice vs global sources, retrigger and phase rules, drawn shapes with loops, tempo sync, and a second source scaling the first ("aux" in Serum, "sidechain" in Pigments, yellow mods in Phase Plant).
- **Cost anchors from our own budget** (docs/roadmap.md, measured 2026-10-19): a full voice is 229 instructions per sample, reverb 2,574 per frame, compressor ~160; the 16-voice prototype is 6,353 of the 12,000-instruction budget, leaving ~5,600 per frame. A 7-voice unison or any oversampled stage is a deliberate budget decision, not a free add [E].
- **Recommended order:** unison; FM/PM/RM; distortion; modulated-delay effects; EQ, limiter, stereo; LFO upgrade; ladder, comb, formant; phase warps; then OTT, follower, gate, disperser; Tier C last.
- **Evidence limit:** no public statistic on feature usage across preset libraries exists (searched). The ranking rests on convergence across five synths, library structure and tutorials; two tutorial sites blocked scripted fetches, so those are read only through search summaries.

## Per-synth inventories (compact)

### Kilohearts Phase Plant 2.4.6 [D]
Sources: [manual](https://kilohearts.com/docs/phase_plant), [modulation](https://kilohearts.com/docs/modulation), [snapins](https://kilohearts.com/docs/snapins), [changelog](https://kilohearts.com/changelog), [product page](https://kilohearts.com/products/phase_plant) ($199, free updates).

- **Generators (5):** Analog (saw, pulse, triangle, sine; sync; PW; unison); Wavetable (256 frames x 2048 samples; bandlimit; unison; editor with Selection, Morph, Pen, Brush, Wave, Harmonic Edit, Filter tools, 14 wavetable effects such as Self FM, Sine FM, Power Sync, Squarify, Disperse, sample-to-wavetable conversion); Sampler (loop modes, X-fade, unison); Granular (since 2.1.0, 2023-05-25); Noise (coloured, stepped and smooth key-tracked). All have Level, Pitch, Harmonic, Shift (Hz) and Phase (offset + random).
- **Generator-area modules:** Distortion (overdrive, saturate, foldback, sine, hard clip, quantize), Filter (LP, BP, HP, notch, shelves, peak; 1x or 2x slope), Non-Linear Filter, Mix and Aux routers, Envelope Output and Curve Output (send to a lane, master or the sideband sidechain bus), Groups. Up to 32 modules.
- **Audio-rate modulation, any module to any module:** Phase = classic FM, Harmonic or Shift = linear FM, Level = ring mod, Pitch = exponential FM. Other modulators update about every 64 samples.
- **Modulators (up to 32, 18 kinds):** Envelope, LFO, LFO Table (a 256-frame wavetable as a modulator), Curve (loop modes off, infinite, sustain, ping-pong, reverse), Random (jitter, smooth, chaos), Audio Follower, Pitch Tracker, Note, Pressure, Velocity, Pitch Wheel, Note Gate, MIDI CC, MPE Timbre, Remap, Lower/Upper Limit, Scale, Sample & Hold. 8 macros.
- **Effects:** three lanes (serial or parallel; per-lane Poly toggle runs the lane per voice). Essentials snapins in the manual (33; the shop text says 30): 3-Band EQ, Bitcrush, Channel Mixer, Chorus, Comb Filter, Compactor, Compressor, Delay, Distortion, Dynamics (upward and downward), Ensemble, Filter, Flanger (barber-pole), Formant Filter, Frequency Shifter, Gain, Gate, Haas, Ladder Filter (transistor or diode), Limiter, Nonlinear Filter, Phase Distortion, Phaser, Pitch Shifter, Resonator, Reverb, Reverser, Ring Mod, Stereo, Shaper, Tape Stop, Trance Gate, Transient Shaper. Free additions since: Clipper (2.4.0), Compactor (2.4.2). Paid (product pages, 2026-10-25): Disperser, Faturator ($59), Shaper Table ($39), Filter Table ($39, 2.3.0, 2024-11), Carve EQ ($79), Slice EQ ($79), Convolver, Multipass (5-band host), Snap Heap.
- **Voice:** glide (always or legato); polyphony (1 = mono, retrig or legato); oscillator unison and global unison (whole stack + poly lanes) with modes Hard, Smooth, Synthetic, Frequency Stack, Pitch Stack, Shepard, Chords; MPE since 2.0.0; oversampling is internal and automatic, not a user setting; microtuning only via per-note bend and a third-party tool.
- **Sequencer / arpeggiator: none documented.** The granular "chord picking pattern" is a strum.

### Xfer Serum 2 (2.0, 2025-03-18, $249) [D]
Sources: [What's New PDF](https://static.xferrecords.com/Serum%202%20What's%20New.pdf) (all 20 pages read), [product page](https://xferrecords.com/products/serum-2), [MusicTech](https://musictech.com/news/gear/serum-2-everything-you-need-to-know/).
- **Oscillators:** Sub, three main oscillators in one of five modes (Wavetable, Multisample/SFZ, Sample, Granular up to 256 grains, Spectral), Noise. **Two warp slots per oscillator**; the warp menu groups Sync, Alt Warp, Filter, Distortion, FM, PD, AM, RM (true FM and PD from filters and distortions are new; individual names are not in the public docs). Smooth wavetable interpolation, phase memory, unison.
- **Filters:** two, series or parallel, new types (MG, Acid and EMS Ladder, MG Dirty, PZ SVF, Diffusor, Comb 2, Wsp, DJ Mixer...), clean mode for Drive.
- **FX:** 13 effects (Bode frequency shifter, Chorus, Compressor with single or multiband mode, Convolve, Delay, Distortion, Equalizer, Filter, Flanger, Hyper/Dimension, Phaser, Reverb with Vintage/Nitrous/Basin, Utility) + 3 splitters (L/H, L/M/H, M/S); two FX buses plus main and direct; mixer page.
- **Modulation:** 4 envelopes, up to 10 LFOs (Lorenz and Rossler chaos modes, drawing tools, 1000 Hz max), 8 macros, **any oscillator or filter as a source**, matrix with source curves and aux sources with curves.
- **Performance:** Arpeggiator (12 slots, pattern editor, gate/chance/repeats), Clip sequencer (12 slots, piano roll, automation lanes, MIDI out), key/scale, swing.
- Unison is a stated CPU lever: above ~3-7 per oscillator is "a red flag", and one chorus on a bus is cheaper than stacked voices ([Xfer CPU guidance](https://support.xferrecords.com/article/51-serum2-sound-design-guidelines-for-optimizing-cpu-usage)) [D].

### Vital 1.5.5 (GPL-3, study only) [M from source, D from site]
Sources: [vital.audio](https://vital.audio/), [source](https://github.com/mtytel/vital) (read: `synth_constants.h`, `synth_oscillator.h`, `synth_lfo.h`, `random_lfo.h`, `compressor.h`, `modulation_connection_processor.h`, `synth_voice_handler.cpp`, wavetable component factory). Repo is stale (last commit 2022-04-20).
- 3 oscillators + a sample layer. Phase "distortion" types: None, Sync, Formant, Quantize, Bend, Squeeze, Pulse Width, FM from osc A/B/sample, RM from osc A/B/sample. Spectral morphs (12): Vocode, Form Scale, Harmonic Scale, Inharmonic Scale, Smear, Random Amplitudes, Low/High Pass, Phase Disperse, Shepard Tone, Skew. Unison up to 16 with 11 stack styles (octave, power chord, major, minor, harmonic series...). Wavetable creator with modifier components (Phase Shift, Wave Window, Frequency Filter, Slew Limiter, Wave Folder, Wave Warp). Text-to-wavetable web service (5 free requests per day) [D].
- 2 filters + filter FX; 8 models (Analog, Dirty, Ladder, Digital, Diode, Formant, Comb, Phaser). 9 FX in a reorderable chain: Chorus, Compressor, Delay, Distortion (soft and hard clip, linear and sine fold, bitcrush, downsample), EQ, Filter, Flanger, Phaser, Reverb. The compressor is **multiband with separate upper and lower thresholds per band** (OTT-like). Oversampling 1x to 8x.
- 8 LFOs (sync types Trigger, Sync, Envelope, Sustain Envelope, Loop Point, Loop Hold), 6 envelopes, 4 random LFOs (Perlin, Sample & Hold, Lorenz), 4 macros, note sources (note, note-in-octave, velocity, aftertouch, slide, lift, mod wheel, pitch wheel, random, stereo), up to 64 connections, 32 voices. No arp or sequencer in the module list.

### Arturia Pigments 7 (2025-12-16, $199) [D]
Sources: [overview](https://www.arturia.com/products/software-instruments/pigments/overview), [manual 7.0.0](https://downloads.arturia.net/products/pigments/manual/pigments_Manual_7_0_1_EN.pdf), [Sonic State](https://sonicstate.com/news/2025/12/16/arturia-pigments-7-out-today-free-upgrade/).
- **Two engine slots x five types:** Analog (3 oscillators, noise, unison Unison/Chord/Super up to 8), Wavetable (linear and exponential FM, ring mod, PM, Phase Transform = PD with Pulse Width, Skew, Round, Tri/Pulse, Octave Plus, Pseudo PW, Fractalize; wavefolding), Sample (+ granular), Harmonic (additive, up to 512 partials), Modal (physical modelling); plus a Utility engine (2 noise sources, sub oscillator, audio in). Engines cross-modulate.
- **Filters:** two, 16 types (Classic, Cluster, Phaser, Formant, Surgeon, Ripple, Reverb, Rage, LoFi, Comb, Mini, MS-20, Matrix 12, Jup-8, SEM, LowPass Gate), series/parallel/blend, filter FM.
- **FX (20):** Delay, Tape Echo, Pitch Shifting Delay, Reverb, Shimmer, Compressor, Multiband, Multi Filter, Param EQ, Vocoder, Distortion, Bitcrusher, Corroder, Super Unison, Chorus, Chorus JUN-6, Flanger, BL-20 Flanger, Phaser, Panner; two insert buses + one aux bus, 3 slots each.
- **Modulation:** 3 envelopes, 3 LFOs, 3 Functions (up to 72 points; loop, one-shot, envelope, envelope-loop), Random tab (Random, Voice Modulator, Sample & Hold, Turing, Binary), Combinate (Sum, Diff, Multiply, Divide, Crossfade, Lag, Threshold, Offset, Remap, Envelope Follower), 4 macros, modulation sidechain, keyboard curves.
- **Sequencer + arp:** 16 steps, 6 tracks, per-track length (polymetry), probability, generation, MIDI out. Voice modes Mono, Mono Legato, Reassign, Rotate; MPE; MTS-ESP.

### Surge XT 1.3 (GPL-3, study only) [D]
Sources: [manual](https://surge-synthesizer.github.io/manual-xt/) (read in full), [changelog](https://surge-synthesizer.github.io/changelog).
- 2 scenes x 3 oscillators; **12 oscillator algorithms**: Classic (unison up to 16, sub), Modern, Wavetable, Window, Sine, FM2, FM3, String, Twist, Alias, S&H Noise, Audio Input. The manual says Classic's unison is cheap and Wavetable's is not.
- 2 filters per scene in several configurations + waveshaper + scene high-pass; types include 12/24 dB (Standard, Driven, Clean), Legacy and Vintage Ladder, K35, Diode Ladder, OB-Xd, Cutoff and Resonance Warp, Tri-Pole, Allpass, Comb +/-, Sample & Hold.
- **16 FX units, 29 types:** EQ, Exciter, Graphic EQ, Resonator, Bonsai, CHOW, Distortion, Neuron, Tape, Waveshaper, Combulator, Frequency Shifter, Nimbus, Ring Modulator, Treemonster, Vocoder, Chorus, Ensemble, Flanger, Phaser, Rotary Speaker, Delay, Reverb 1, Reverb 2, Spring Reverb, Airwindows, Conditioner, Audio Input, Mid-Side Tool.
- 6 voice LFOs + 6 scene LFOs, each switchable between Sine, Triangle, Square, Saw, Noise, S&H, Envelope (DAHDSR), 16-step Step Seq (with envelope-retrigger lane), MSEG, Formula (Lua); Amp and Filter EG; Random, Alternate; 14 voice and note properties; 8 macros. No arp, no clip sequencer.

## The union table

Cost classes [E], anchored on enth's measured numbers (full voice 229 instructions per sample; SVF drive stage +1.06): per voice and sample, **cheap** up to ~15, **medium** 15-60, **heavy** over 60 or FFT or oversampling; global per stereo frame, cheap up to ~200 (compressor ~160), medium 200-1,500, heavy over 1,500 (reverb 2,574). Report 05 should replace these with measurements. Tier: **A** broad sound design, **B** modern bass music and dubstep, **C** nice-to-have; "(B)" means a Tier A feature that dubstep also leans on.

| Feature | PP | S2 | V | Pg | Sg | What it is for | Cost | Tier |
|---|---|---|---|---|---|---|---|---|
| Classic band-limited waves | Y | Y | ~ | Y | Y | subtractive core | cheap | A (have) |
| Wavetable osc, smooth frame interpolation | Y | Y | Y | Y | Y | evolving timbre | medium | A (have) |
| Sub oscillator or octave layer | ~ | Y | ~ | Y | Y | bass weight | cheap | A |
| Noise (white, pink, coloured, S&H) | Y | Y | Y | Y | Y | air, hits, grit | cheap | A (have) |
| Osc unison (detune, spread, blend) | Y | Y | Y | Y | Y | width, supersaw, reese | N x osc | A (cap ~5-7) |
| Unison stack styles (octave, chord, Super) | Y | ~ | Y | Y | ~ | instant chords | as unison | B |
| Start phase, phase random, sync, PW | Y | Y | Y | Y | Y | punch, sync leads | cheap | A (sync have) |
| Phase-domain warps (bend, skew, quantize, mirror, flip) | ~ | Y | Y | Y | ~ | one wave, many timbres | cheap-med | A (B) |
| Two warp slots per oscillator | - | Y | ~ | - | - | stacked warps | 2x warp | C |
| Spectral warps (vocode, smear, stretch, shepard) | - | ~ | Y | - | - | spectral morph | heavy | C |
| Audio-rate FM/PM osc to osc | Y | Y | ~ | Y | Y | bells, EP, metallic, screech | medium (+oversample) | A (B) |
| Ring or amplitude mod osc to osc | Y | Y | Y | Y | Y | clangor, sidebands | cheap | A (B) |
| Wavefold / foldback | Y | Y | ~ | Y | Y | harmonics from a sine | cheap-med | A (B) |
| Wavetable import, bake, morph | Y | Y | Y | ~ | ~ | content | offline | A import, C editor |
| Sample osc, loops, slices | Y | Y | ~ | Y | - | hits, textures | medium | C |
| Multisample / SFZ | - | Y | - | Y | - | real instruments | medium + assets | C |
| Granular osc | Y | Y | - | Y | - | clouds, freezes | heavy | C |
| Spectral resynthesis osc | - | Y | - | - | - | stretch, freeze | heavy | C |
| Additive / harmonic engine | - | - | - | Y | - | organs, bells | heavy | C |
| Modal / string / Plaits-style | - | - | - | Y | Y | plucks, strings | medium | C |
| SVF + drive | Y | Y | Y | Y | Y | all-round filter | cheap | A (have) |
| Ladder 4-pole, saturating | Y | Y | Y | Y | Y | classic bass, acid | medium | A |
| Other filter models (MS-20, SEM, diode, all-pass types) | ~ | Y | Y | Y | Y | colour | medium | C |
| Comb filter | Y | Y | Y | Y | Y | metal, pluck, flange | cheap | A (B) |
| Formant / vowel filter | Y | ~ | Y | Y | - | yoi, talking bass | medium | B |
| Dual filters, series/parallel | ~ | Y | Y | Y | Y | complex timbres | 2x filter | A |
| Filter FM (audio-rate cutoff) | Y | ? | ? | Y | ? | screech, metal | medium | C |
| ADSR/AHDSR (curve, hold, delay) | Y | Y | Y | Y | Y | amp, filter | cheap | A (have) |
| LFO: free/sync, retrigger, delay/fade | Y | Y | Y | Y | Y | movement | cheap | A (have) |
| Drawable LFO / MSEG / curve with loops | Y | Y | Y | Y | Y | wobble shapes | cheap | A (B) |
| Stepped modulator (16 steps, trigger lane) | ~ | ~ | ~ | Y | Y | riddim, gating | cheap | B |
| Random, S&H, smooth random | Y | Y | Y | Y | Y | life, humanise | cheap | A |
| Chaos (Lorenz, Rossler), Turing, mod math (lag, threshold) | ~ | Y | Y | Y | - | generative | cheap | C |
| Envelope / audio follower | Y | - | - | Y | - | ducking, pump | cheap | B |
| LFO Table, Pitch Tracker, Note Gate | Y | - | - | - | - | exotic sources | cheap-heavy | C |
| MIDI sources (vel, AT, wheel, key, MPE) | Y | Y | Y | Y | Y | expression | cheap | A |
| Macros (4 or 8) | 8 | 8 | 4 | 4 | 8 | performance | cheap | A (have) |
| Mod of mod / aux source / sidechain | Y | Y | ~ | Y | Y | depth control | cheap | B |
| Per-connection curve / remap | Y | Y | Y | ~ | - | response shape | cheap | B |
| Distortion (soft, hard, fold, sine, diode) | Y | Y | Y | Y | Y | bass music | cheap (heavy oversampled) | A (B) |
| Bitcrush / downsample | Y | Y | Y | Y | ~ | digital grit | cheap | B |
| User waveshaper (curve or table) | Y | ~ | - | - | Y | custom shapes | cheap | C |
| Chorus | Y | Y | Y | Y | Y | width | medium | A |
| Ensemble / Hyper-Dimension | Y | Y | - | Y | Y | lush width | medium | B |
| Flanger | Y | Y | Y | Y | Y | jet, comb sweep | medium | A |
| Phaser | Y | Y | Y | Y | Y | sweep | medium | A |
| Delay (sync, ping-pong, feedback filter) | Y | Y | Y | Y | Y | echo | cheap | A |
| Reverb | Y | Y | Y | Y | Y | space | heavy | A (have) |
| Compressor | Y | Y | Y | Y | ~ | dynamics | cheap | A (have) |
| Multiband upward/downward (OTT) | ~ | Y | Y | Y | - | loud glue | medium | B |
| Limiter / clipper | Y | ~ | ~ | ~ | Y | safety | cheap | A |
| EQ (parametric / graphic) | Y | Y | Y | Y | Y | tone | cheap | A |
| Stereo utility (width, M/S, Haas) | Y | Y | ~ | Y | Y | image | cheap | A |
| Frequency shifter, ring-mod FX | Y | Y/- | - | - | Y | inharmonic, bell tones | medium | C |
| Pitch shifter / harmoniser | Y | - | - | Y | - | octaves, detune | heavy | C |
| Vocoder | - | - | - | Y | Y | talking | heavy | C |
| Convolution | Y | Y | - | - | - | rooms, IRs | heavy | C |
| Gate / trance gate / transient shaper | Y | ~ | - | ~ | - | rhythm, punch | cheap | B |
| Disperser (all-pass dispersion) | Y | - | - | - | ~ | bass smear, zaps | medium | B |
| Tape, resonator, reverser | Y | - | - | Y | Y | character | medium | C |
| Serial FX chain, per-slot mix/bypass | Y | Y | Y | Y | Y | basic routing | cheap | A |
| Parallel buses / aux sends | Y | Y | ~ | Y | Y | send FX | cheap | B |
| Band splitter (L/H, L/M/H, M/S) | Y | Y | - | ~ | - | split-band distortion | medium | B |
| Per-voice FX lane | Y | - | - | - | - | per-note FX | heavy (x voices) | C |
| Free generator graph (any to any) | Y | ~ | - | ~ | ~ | patch freedom | n/a | C |
| Mono / legato / glide / priority | Y | Y | Y | Y | Y | leads, bass | cheap | A |
| MPE; microtuning | Y; ~ | ?; ? | Y; Y | Y; Y | Y; Y | expression; scales | cheap | C |
| Arpeggiator; clip or step sequencer | -; - | Y; Y | -; - | Y; Y | -; ~ | pattern play | cheap | C |
| Oversampling / quality control | Y (auto) | ? | Y (1x-8x) | - | ~ | alias control | heavy | A (infra) |

Cell notes: in paired rows ("A; B", "Y/-") the marks follow the order of the feature name. V mod-of-mod "~" (Vital modulates LFO and envelope parameters but has no per-connection aux source); S2 formant "~" (tutorial only); PP phase warps "~" (Phase Distortion snapin, sync, PW, wavetable-editor effects).

## Usage evidence

No public measurement of feature usage across preset libraries turned up (searches for Serum, Vital and Surge preset statistics found only datasets built for model training). Three weaker kinds of evidence, strongest first:

1. **Convergence [D, counted from the table].** Shipped by all five: unison, sync, FM/PM, ring mod, ladder filter, comb, distortion with fold and clip, chorus, flanger, phaser, delay, reverb, EQ, drawable LFO, random/S&H, macros. Four of five: multiband/OTT-capable dynamics, formant filter, ensemble, phase-domain warps. Two or fewer: arp and sequencer, convolution, vocoder, modal, additive, spectral oscillator, pitch tracker.
2. **Library structure [D].** Serum 2's factory browser has a Bass folder with sub-folders 808, Acid, Electric, Hard, Misc, Modulated, Reese, Retro Analog, Sub, Synth, and category tags Midrange, Modulated Bass, Arpeggio, Lead, Pad, Bell, Chord, Drum and others ([What's New PDF](https://static.xferrecords.com/Serum%202%20What's%20New.pdf), p.20). Serum 2's page features a "Drum & Bass, Drumstep" bass pack ([product page](https://xferrecords.com/products/serum-2)). Pigments 7's Explorations Vol. 5 has three 150-preset packs: Hazy, **Dubstep** and IDM ([overview](https://www.arturia.com/products/software-instruments/pigments/overview)). Reese and modulated bass are first-class categories in the flagship libraries.
3. **Tutorials [A].** The [CreativeLive "Xfer Serum" course](https://www.creativelive.com/classes/xfer-serum-synthesizer-sound-design-music-production-tomas-george) (read in full; 29 lessons, Serum 1 era) spends 13 of 25 content lessons on effects (Reverb, Compressor, Multiband Compressor, EQ, Delay, Chorus, Flanger, Phaser, Filter, Hyper-Dimension, Distortion, Serum as FX), 6 on oscillators, warp and wavetable editing, 4 on filter, envelopes, LFOs and voicing, 1 on the mod matrix. Dubstep chains, known only from search-engine summaries because [Bassgorilla](https://bassgorilla.com/serum-tutorial-super-growl-bass-in-ableton-live/) and [Output](https://output.com/blog/best-dubstep-plugins) block scripted fetches and the [ADSR](https://www.adsrsounds.com/serum-tutorials/making-a-skrillex-growl-in-serum/) page truncated: formant filter + Hyper/Dimension + Diode-2 distortion with an LFO on the formant; band-pass resonance as a formant stand-in, Diode II, an EQ cut near 300 Hz, then OTT; two FM-linked oscillators ([Bassgorilla](https://bassgorilla.com/serum-bass-tutorial-robotic-digital-dubstep-bass/)); Diode-2 + multiband + wide pitch bend ([Bassgorilla](https://bassgorilla.com/serum-tutorial-snails-bass-hybrid-trap-dubstep-bass/)); growl wavetables from FM and bend warps ([Sonic State](https://sonicstate.com/news/2020/04/06/100-free-serum-wavetables/)); OTT stacked with Disperser ([Slooply](https://slooply.com/blog/unlock-serums-secret-weapon-the-hidden-fx-module-presets/)); several mild distortion stages + multiband to keep the sub clean (Output).

**Macro features ranked for perceived versatility** [E, from the three kinds above]:

| Rank | Feature | Why | Cost in enth |
|---|---|---|---|
| 1 | Distortion family (clip, fold, crush, diode) | in every cited dubstep chain; 5 of 5 | cheap, medium with ADAA |
| 2 | Unison | 5 of 5; vendors name it the first CPU lever (Xfer, Surge docs) | linear in voices, the budget risk |
| 3 | Filter variety with movable resonance (ladder, comb, formant) | formant in the growl chains; 4 of 5 | medium |
| 4 | FM/PM/RM | 5 of 5; metallic basses, bells, EP | medium, alias care |
| 5 | Wavetable + phase warps | S2 and Vital headline it; 4 of 5 | cheap-medium |
| 6 | Chorus / ensemble, flanger, phaser | 5 of 5; the largest teaching block | medium, shared core |
| 7 | Drawn LFO / MSEG with sync | 5 of 5; wobble and riddim need it | cheap |
| 8 | OTT / multiband | 3-4 of 5; ends most cited chains; a mix tool | medium |

## Modulation system comparison

| | Phase Plant | Serum 2 | Vital | Pigments 7 | Surge XT |
|---|---|---|---|---|---|
| Assigning | click "+" on a source, drag the mod knob on any target | drag, or a matrix page (source, curve, amount, polarity, destination, aux source, aux curve, output) | drag a source onto a knob with preview; mod list | drag from the mod strip, or Mod Source and Mod Target views, quick-edit popup | click a source, click again for routing mode, drag the blue slider; or drag the source onto a parameter |
| Per connection | amount, curvature, bounds; yellow mods scale other mods | amount, polarity, source curve, aux source + curve, bypass | amount, power curve, bipolar, stereo (L/R split), bypass; max 64 [M] | amount, sidechain source, range display | amount (relative to base), polarity |
| Poly vs global | modulators are per voice with a global value for non-voice targets; Random has Unison or Independent voice mode | per-LFO MONO toggle visible in UI | LFOs plug into the voice handler and retrigger per note; poly path chosen by source [M] | voice-based sources; Voice Modulator advances per note | 6 voice LFOs and 6 scene LFOs; voice sources cannot drive scene parameters |
| Shape drawing | LFO editor, Curve with 5 loop modes, LFO Table | LFO drawing tools, independent grid, presets | LFO editor | Functions, up to 72 points, 4 play modes | MSEG, 16-step Step Seq, Lua Formula |
| Tempo sync | LFO, Random, Curve in note lengths | BPM with triplet, dotted, host anchor | tempo, triplet, dotted | straight, triplet, dotted | per-parameter menu |
| Retrigger | Auto, Never, Always, Legato, plus a modulatable trigger gate | FREE, RETRIG, ENVELOPE; invert legato on envelopes | Trigger, Sync, Envelope, Sustain Env, Loop Point, Loop Hold [M] | Retrig Source menu (keys, sequencer, grains) | Freerun, Keytrigger, Random |
| Follower / sidechain | Audio Follower on lane, master or sideband; Pitch Tracker | none documented | none | Envelope Follower combinator; modulation sidechain | none as a source |
| Counts | 32 modulators, any mix; 8 macros | 4 env, 10 LFO, 8 macros | 6 env, 8 LFO, 4 random, 4 macros | 3 env, 3 LFO, 3 Functions, random, combinators, 4 macros | 12 LFO, 2 EG, 8 macros |
| Audio rate | generator modules audio-rate, rest every ~64 samples | any oscillator or filter is a source (rate not stated) | audio-rate connection path exists [M] | engine cross-mod (FM/PM) | oscillator FM routing |

What this says for enth: the cheap, high-yield pieces are per-connection curve, a second-source scaler, retrigger modes (free, key, legato), tempo sync with triplet/dotted, drawn shapes with loop points, and a stepped shape. Audio-rate modulation should stay a fixed generator-to-generator feature (as in Phase Plant's generator area and Pigments' cross-mod), not a general matrix path, because of cost and determinism.

## Recommendations for enth: modules in priority order

Costs are [E]; report 05 should measure. SIMD voices (f32x4 halves the measured 16-voice render) change the per-voice numbers.

**Wave 1 — Tier A, cheap, reuses what exists**
1. **Oscillator unison** (detune, spread, blend; cap ~5-7 voices, "octave" stack later). Cost N x osc; the one A feature that can break the 400-instruction voice sub-budget (229 + 6 extra oscillators passes 400 if an oscillator costs more than ~30 instructions per sample). Decide the cap against a measurement.
2. **FM/PM and RM/AM between oscillators** as a fixed operator feature (B to A, sub or noise as modulator). Medium; decide oversampling or ADAA with report 05.
3. **Distortion module**: soft clip, hard clip, linear and sine fold, tube/diode, bitcrush + downsample, bias, drive, mix.
4. **Modulated-delay family on one core**: chorus, flanger, phaser, ensemble, plus delay. Global, ~100-300 per frame each.
5. **EQ** (4-band parametric), **limiter or clipper**, **stereo utility**.
6. **LFO and envelope upgrade**: tempo sync, retrigger modes, delay and fade, S&H and smooth random, a drawn multi-point shape with loop points, a 16-step shape; envelope hold, delay, curves.
7. **Filters**: nonlinear 4-pole ladder, comb, formant (vowel morph), dual series/parallel.
8. **Phase warps** on the oscillator: bend, quantize, mirror, flip, squeeze (sync and PW exist).

**Wave 2 — Tier B**
9. **Multiband OTT-style compressor** (Linkwitz-Riley split + three up/down compressors; reuse the compressor).
10. **Envelope follower** (global chain tap), random modulators, mod-of-mod aux source, per-connection curve.
11. **Step gate**, **disperser** (all-pass cascade), **frequency shifter**, **band splitter**, **parallel buses**.

**Wave 3 — Tier C:** sample and granular, additive, modal, spectral warps, vocoder, convolution, pitch shifter, per-voice FX, arp and sequencer (the DAW supplies MIDI), MPE, microtuning, formula modulators.

## Gaps

- **Serum 2 individual warp, distortion and unison-mode names**: not in public docs I could reach (the web manual exposes only getting-started pages; the PDF names categories). Check in the plugin or the installed User Guide PDF. S2 MPE, microtuning, oversampling and per-LFO voice scope not verified.
- **Vital**: the repo is stale (2022-04-20), so 1.5.x changes are not in the source I read; latest binary version and date unconfirmed; LFO poly/mono scope only partly checked.
- **Tutorial evidence is thin and second-hand**: Bassgorilla and Output returned 403, ADSR and MusicRadar truncated; chains come from search summaries. A measured corpus statistic could be built by counting module use in Vital's text presets or Surge's factory patches (statistics only, but a bulk download that needs an OK).
- **A robots.txt slip, disclosed:** before I had read github.com's robots.txt I fetched three `/tree/` directory-listing pages of the Vital repo through WebFetch; its generic group disallows `/*/tree/`. I stopped, did not use those listings as sources (every enum and constant above comes from raw.githubusercontent.com, which has no robots.txt, plus the repo's front page and one api.github.com commits call), and will not repeat it.
- **Cost classes are estimates**, anchored on docs/roadmap.md and docs/benchmarks.md; no per-oscillator instruction count was available to me.
- **Phase Plant 3** (free, teased Dec 2025 on KVR, wishlist only) is not released; it could add additive, vocoder or OTT and change the delta.

## Sources

Inline links above are the primary citations. Also read: Kilohearts [host plugins](https://kilohearts.com/docs/host_plugins), [curves](https://kilohearts.com/docs/curves_lfos_remaps), [wavetables](https://kilohearts.com/docs/wavetables), [basic usage](https://kilohearts.com/docs/basic_usage), [Multipass](https://kilohearts.com/docs/multipass), [blog](https://kilohearts.com/blog) and product pages (prices); [Serum 2 web manual](https://xferrecords.com/web-manual/serum-2/welcome); [KVR Phase Plant 3 teaser](https://www.kvraudio.com/forum/viewtopic.php?p=9175706); [Sound On Sound on Pigments 7](https://www.soundonsound.com/reviews/arturia-pigments-7) (paywalled); [Sonic Weaponry](https://sonic-weaponry.com/blogs/free-production-tutorials-and-resources/serum-2-released) (vendor copy, not used for claims). Internal: docs/roadmap.md (16-voice CPU budget), docs/benchmarks.md (voice instruction counts).

Method: robots.txt was checked for kilohearts.com (crawl-delay 1.5 s, kept to 2 s), xferrecords.com and its support and static hosts (comments only, no rules), arturia.com (sign-in paths only), vital.audio (none), surge-synthesizer.github.io (none) and raw.githubusercontent.com (none). Scripted fetches used User-Agent `enth-research/0.1`: about 35 pages of kilohearts.com at 2 s spacing or more, a handful per other site, no crawling. Third-party pages were read through the WebFetch tool.
