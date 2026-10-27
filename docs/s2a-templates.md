# S2a create templates and report 04's ten sketches (lane TPL)

- **Status: built and measured (S2a wave 5, 2026-10-27).** The four templates, the loader that keeps them off while their kinds are hidden, the ten sketches as reference patches, and the measurements of create with and without the templates. INT-S2a switches the templates on by clearing the hidden flags; nothing else changes (see [Switching them on](#switching-them-on)).
- **In one line:** all four templates beat `init` on their genre's dev targets at 4,000 renders (wobble not at 250 and 500); create with the S2a templates, the bank and 4,000 renders covers **5 of the ten sketches** (S2, S3, S4, S8, S9) against 3 without them and 2 with `init` only; today's create is bit-identical with the templates off ([Results](#results)).
- Code: `crates/enth-solver/src/create/s2a.rs` (loader and tests), `content/templates/s2a/{growl,reese,wobble,fm_bass}.patch.json`, `content/templates/s2a/sketches/`. The GR3 kit gained a `templates` job option and moves `wt` with the register shift (`tools/datagen/src/gr3.rs`, `gr3/job.rs`, `gr3/target.rs`). Measurement tooling and every run's records stay in the lane's scratch folder, named below.
- Why templates: create searches parameter values only (docs/r3.md); a start's modules, routes and route depths are fixed by the start, so the templates decide which structures create can reach. Before TPL the four (`init`, `pluck_1osc`, `pad_2osc`, `bass_sub`) had no distortion, no phase modulation, no formant, no tempo-synced motion.

## The loader

| Function (`enth_solver::create::s2a`) | Returns | Used by |
| --- | --- | --- |
| `s2a_templates()` | all four, whatever the flags say | tests; the GR3 kit's `templates` job option; measurements |
| `enabled_s2a_templates()` | those whose every module kind is visible (`ModuleKind::is_hidden`) | `enth create` (`crates/enth-cli/src/cmd/create.rs`), the plugin's Create view (`crates/enth-plugin/src/create.rs`), the GR3 kit's default arm A (appended after `pluck_1osc`, `pad_2osc`, `bass_sub`) |

While any S2a kind is hidden `enabled_s2a_templates()` is empty, so create's starts, its screening round and every result are exactly what they were before this lane (checked bit for bit: [Today's create is unchanged](#todays-create-is-unchanged)). When INT-S2a clears the flags each template whose kinds are all visible joins the portfolio, in the order growl, reese, wobble, fm_bass, after the three older ones. `enth new --template` is untouched: these are create starts, not user templates.

## The four templates

All four are mono legato basses with a sparse patch file: values that differ from the defaults only. Each keeps the structure its genre needs and sets characteristic values, so the screening render already sounds like the genre; create searches the rest. The route depths are fixed (create never searches a route), so each is the genre's typical amount.

| Template | Structure | Routes (fixed depth) | Voice effects / effects | Free axes (continuous + categorical) |
| --- | --- | --- | --- | --- |
| `growl` | `wt1` (basic table, position 50 %, PM index 2) phase-modulated by a silent `osc2` sine at 1:1; a looping quarter-note `mseg1` of four points | `osc2 → wt1.pm` 100; `mseg1 →` `wt1.pm_index` +40, `wt1.position` +25, `osc2.semi` +5, `filter1.cutoff` +15, `formant1.f1` −30, `formant1.f2` −30 | `dist1` asymmetric 18 dB, `filter1` band-pass 1 kHz, `formant1` mix 40 %, `dist2` sine fold 8 dB mix 40 % (four voice effects, the cap); `ott1` 50 %, `eq1` low cut 150 Hz and a −6 dB bell at 400 Hz | 120 + 14 |
| `reese` | `osc1`, `osc2` saws +12 st, 28 ct apart; `osc3` sine sub on the key at 50 % | `lfo1` (bar-synced triangle) `→ filter1.cutoff` +10 (±1 octave) | `filter1` low-pass 700 Hz, resonance 18 %, drive 6 dB; `dist1` soft 8 dB with `keep_lows` 120 Hz; `phaser1` six stages, 0.2 Hz, feedback 50 %, mix 40 % | 30 + 15 |
| `wobble` | `osc1` saw, `osc2` pulse an octave down +7 ct | `lfo1` (eighth-synced sine, unipolar, start phase 270°) `→ filter1.cutoff` +50 (five octaves) | `filter1` low-pass 150 Hz, resonance 45 %, drive 12 dB; `dist1` soft 12 dB; `ott1` 30 % | 33 + 11 |
| `fm_bass` | `wt1` (basic, three-voice unison, detune 25 ct, PM index 2) phase-modulated by a silent `osc2` sine two octaves up; `osc3` sine sub on the key at 50 % | `osc2 → wt1.pm` 100; `env2` (23 ms attack, 4.2 s decay, no sustain) `→` `ladder1.cutoff` +30, `wt1.position` +15, `wt1.pm_index` +20 | `ladder1` 24 dB low-pass 300 Hz, resonance 25 %; `dist1` hard 18 dB with `keep_lows` 200 Hz; `comp1`, `eq1` (presence bell +3 dB at 4.85 kHz, high shelf −6 dB), `reverb1` −12 dB | 66 + 9 |

The counts are what a job searches under the dev-target performance (`cargo test -p enth-solver --lib s2a -- --nocapture`). Two limits to know:

- **MSEG's 59 parameters are all free** while the observability mask knows nothing about `points`: `growl` searches 59 MSEG axes for a four-point shape (12 live). A `lfo` has six. If create's growl results need it, the mask learning that a point beyond `points` is dead is a small rule in `crates/enth-challenge/src/random/mask.rs` (not changed here: the mask is a shared hotspot, and no generator-v1 patch has an MSEG).
- **Structure is the template's, values are create's.** A route absent from a template is out of reach, and a depth is as the template sets it. `wobble` therefore sweeps five octaves whatever the target's sweep.

`delay` (merged on 2026-10-27) is in no template and no sketch: none of report 04's ten sketches uses one, and a fifth echo structure would only add axes. `eq` is in `growl` and `fm_bass`, where S4 and S9 use it.

## Report 04's ten sketches

`content/templates/s2a/sketches/` holds S1 to S10 as sparse patch files, built from the merged modules and played at C3, velocity 100, held 1.5 s of 3 s (the dev-target performance; S10b is held 6.9 s of 8.5 s). Report 04 places the bass sketches on F1 to F2 (43.7 to 87.3 Hz), so each oscillator carries a `semi` offset (−19 st puts C3 on F1, −7 st on F2) and svf cutoffs under key tracking are the report's cutoff at the played note. S4m is the report's metallic variant exactly as written; it is refused (six voice effects), and S4m4 is that variant cut to the cap.

| Sketch | File | What it is | Measured against report 04's target line |
| --- | --- | --- | --- |
| S1 clean sub | `s01-clean-sub` | sine, asymmetric 2 dB, 120 Hz low-pass, legato glide, quarter pump on the level | f0 43.3 Hz (41 to 49); H2 −16.3 dB (−15 to −25); above 300 Hz −68 dB (< −40); the quarter pump the sketch lists moves the level 1.9 dB (p95/p5), where the target line says steady RMS |
| S2 1/8 wobble | `s02-wobble-eighth` | saw + pulse, 150 Hz low-pass, resonance 45.8 %, eighth-synced sine from its trough, soft 12 dB, OTT 30 % | filter sweep 4.662 Hz (4.667); centroid 5 to 95 %: 139 to 2492 Hz (about 200 Hz to 2.5 kHz) |
| S3 reese | `s03-reese` | two saws ±14 ct, sine sub, bar-synced triangle on the cutoff, soft 8 dB sparing the lows, phaser | amplitude beating 1.82 Hz (1 to 2) |
| S4 FM growl | `s04-fm-growl` | the `growl` structure at the report's values, plus the chain's EQ | 94 % of the energy in 300 Hz to 3 kHz; centroid moves at 2.21 Hz (quarter 2.33) |
| S5 riddim | `s05-riddim` | two pulses gated by an eighth-triplet square, comb, hard 9 dB, OTT 100 %, sine sub with a quarter pump | gating 6.92 Hz (7.0) |
| S6 yoi | `s06-yoi` | saw + formant-table oscillator, formant i-o-i from a one-shot quarter MSEG, downsampler on the MSEG, OTT 70 % | F2 region 2.06 kHz → 0.50 kHz → 2.41 kHz inside 0.45 s |
| S7 screech | `s07-screech` | hard-synced slave swept by an envelope, 1/16 vibrato, high-pass, hard 12 dB, OTT, reverb | the dominant peak falls from 621 Hz (first 20 ms) through 363 Hz (30 ms) to 263 Hz (70 ms) and stays: the target says two octaves over about 250 ms; the adsr is exponential (see gaps) |
| S7b laser | `s07b-laser` | wt sine dropping 36 st in 80 ms, PM at ratio 2 with a 60 ms index envelope | dominant peak 3.9 kHz in the first 10 ms, 0.75 kHz at 20 ms, 0.66 kHz from 50 ms (a PM sideband); f0 after the dive 130.8 Hz (`enth describe`) |
| S8 neuro | `s08-neuro` | two bent wavetables, noise, notch and band-pass, bar LFO on positions and notch, sample-and-hold band-pass, asymmetric 20 dB at 60 %, chorus, EQ high shelf from a global 1/16 LFO | structure only (no number in the report) |
| S9 foghorn | `s09-foghorn` | three-voice wt PM by a sine at +24 st, two low-passes in series, opened by a 23 ms/4.2 s envelope, hard 18 dB sparing the lows, 17 s MSEG decay on the level, comp, EQ, reverb | centroid peaks at 0.1 s and falls to 0.6 kHz by 0.8 s |
| S10 sub drop | `s10-sub-drop` | sine falling two octaves (RC curve), noise burst, soft 6 dB, reverb | pitch by 200 ms steps: 175, 87, 70, 67, 67, 64 Hz (the report's 261.6 → 65.4 Hz on an RC curve) |
| S10b riser | `s10b-riser` | noise + seven-voice saw, high-pass and pitch raised by a 4-bar one-shot MSEG, a band-pass LFO sped from 1/4 to 1/32 by the MSEG | the level and centroid modulation speeds 3.1, 4.6, 7.0, 12.3 and 12.8 Hz over the windows 0.3-1.5, 1.5-3, 3-4.5, 4.5-6 and 6-6.9 s (2.3 to 18.7 Hz in the report) |

**Gaps** (a sketch is matched as far as the modules allow; the rest is listed). The first six are in lane CAT's report; the last three were found here.

| Gap | Sketches | Effect on the sketch | Source |
| --- | --- | --- | --- |
| `bend` (pitch-bend range) is not a patch parameter | S3, S4, S8 | the +-12/24 st hand-drawn bends are absent; the glide stays | CAT |
| the sub lane: `voice_fx` runs serially after the generator sum | S3 to S9 | the sine sub shares the chain; `dist.keep_lows` spares it from the distortion only (the comb, formant, filters and OTT still touch it) | CAT |
| frequency shifter | S8 | absent | CAT |
| amplitude envelope above 10 s (`voice.amp_env` must be an `adsr`) | S9 | the 17 s decay is a one-shot MSEG on `wt1.level`; inside the 3 s clip it is a 9 % fall | CAT |
| stepping `lfo.beats` per note from a macro (an enum cannot be modulated) | S2 | unobservable in a one-note render, so S2 has no macro | CAT |
| the metallic S4 needs six voice effects against `MAX_VOICE_FX` 4 | S4 | S4m is refused; S4m4 is the same cut to four (see [Chain depth](#chain-depth-the-open-check)) | CAT |
| "noisy" unison detune | S9 | absent (no parameter) | here |
| the `adsr` is exponential (RC) | S7, S10 | report 04's "D250" sweep spends most of its fall in the first 60 ms (S7: two octaves over about 250 ms is not reached); an MSEG would draw a linear one | here |
| global one-shot MSEG on `reverb.wet` (only global LFOs reach `fx`; report 04 notes it) | S10b | the riser has no reverb swell | here |

### Chain depth (the open check)

`MAX_VOICE_FX` and `MAX_FX` stay 4; the coordinator decides. Evidence from the sketches: S4's metallic variant as report 04 writes it (asymmetric distortion, band-pass, formant, sine fold, bitcrusher, keytracked comb) needs **six** voice effects, and `validate` refuses it (`at most 4 voice_fx are allowed`; `s04m-fm-growl-metallic-6fx`, pinned by a test). Every other sketch fits: voice effects S1 2, S2 2, S3 2, S4 4, S5 3, S6 2, S7 2, S8 3, S9 3, S10 1, S10b 2; global effects at most 3 (S9: compressor, EQ, reverb) of 4; generators at most 3 of 4, modulators at most 4 of 8, routes at most 8 (S4m) of 64. `growl` is itself exactly at the voice-effect cap (S4's four). S4m4 (`s04m4-fm-growl-metallic-4fx`) is the variant with the formant and the sine fold dropped; its create result is in the tables below.

## Today's create is unchanged

With the S2a kinds hidden, `enabled_s2a_templates()` is empty: create's starts, screening round, searches and results are what they were before this lane. Checked bit for bit against main `f59fb60` (`git archive`, built in a separate target directory): `enth create --target <wav> --budget 800 --seed 1 --workers 2 --seconds 300 [--bank enth-bank-v1.bin] --out best.patch.json --record rec.json` on four of the sketch renders (S2, S3, S4, S9), each with and without the bank, run from separate directories with the same relative output names. All eight pairs are identical: the best patch byte for byte, the exit status, and the printed object and the `enth.create/1` record after dropping the two wall-clock fields (`seconds` per round and `elapsed_s`), which differ between any two runs of one binary. The test `no_s2a_template_is_enabled_while_its_kinds_are_hidden` keeps it so: it fails the day a template's kinds all turn visible, which is INT-S2a's cue (below).

## Switching them on

INT-S2a clears the `hidden` flags (never this lane). Nothing else is needed for create: a template joins the portfolio as soon as every kind it uses is visible, so `wobble` (hidden kinds `dist`, `ott`) and `reese` (`dist`, `phaser`) can come on before `growl` (`wt`, `dist`, `formant`, `mseg`, `ott`, `eq`) and `fm_bass` (`wt`, `ladder`, `dist`, `eq`). Then:

- change `no_s2a_template_is_enabled_while_its_kinds_are_hidden` (`crates/enth-solver/src/create/s2a.rs`) to expect the enabled names;
- create's portfolio grows by up to four starts, so the screening round renders four more patches and the `--candidates 8` shortlist chooses among more; the plugin's Create view lists the start names in its tiles (`crates/enth-plugin/src/create.rs` appends `enabled_s2a_templates()` to its template list; check the tile row's width with eight starts);
- the numbers in [Results](#results) are for the full set of four with the bank and the GR3 kit's default create settings.

## Results

All numbers are from the GR3 kit's render mode (machine-independent: a run is an exact prefix of a longer one, the same for any worker count), three seeds, scored the way the kit scores a preset target (`sound/1`, pass at 90 %, distances in JND; lower is better) after its register shift. The run records (`enth.gr3-run/1` JSONL, one per configuration and seed, with the best patch at every mark) and every script are in the lane's scratch folder `~/code/enth-scratch/s2/tpl/` (`runs/`, `jobs/`, `work/`, `notes.md`); they are not in the repository.

### Protocol (fixed before the first template result)

- **Kit and settings.** `enth-datagen gr3 run` (arm A: create itself), render budget 4,000, marks 250, 500, 1,000, 2,000 and 4,000, K = 8, first rung 50, eta 2, two finalists, 16 neighbours, scored window 3 s, three workers, seeds 1, 2 and 3. The bank for the "bank" configurations is the GR3 baseline's `enth-bank-v1.bin` (sha 66d5f380...). The binary was built from `36a84bf`; the templates and the measuring code have not changed since (only lints, tests and sketches).
- **Targets (23 presets).** The twelve sketches (S1 to S10, S7b laser, S10b riser) rendered at C3, velocity 100, held 1.5 s of 3 s (S10b: 6.9 s of 8.5 s); three kit anchors from `content/presets-candidates/bass/` (`wobble-machine` at C2 and at G2, `warm-reese` at C2); eight **hold-out** patches, two per genre, written before the first template run with other recipes (`h-growl-1/2`, `h-reese-1/2`, `h-wobble-1/2`, `h-fm-1/2`) and never run before the grid.
- **Genre dev sets, fixed at the start:** growl = S4, S6, S8; reese = S3, `warm-reese`; wobble = S1, S2, S5, `wobble-machine` (two keys); fm_bass = S4, S9, S7b. **One amendment**, after the first exploratory runs and before the grid: S7b laser left fm_bass's set (a three-octave pitch dive is not an FM-bass trait, and no fm_bass structure has a route that dives); it is still run and reported. S7 screech, S10 sub drop and S10b riser belong to no set.
- **Configurations.** I0N = `init` only (the heuristic start), no bank. GN, RN, WN, FN = `init` + one S2a template, no bank. P0N / P1N = today's three templates / + the four S2a templates, no bank. P0P / P1P = the same with the bank (the product's configuration). `init` is always kept by create, so "template alone" means the portfolio {init, template}.
- **"Beats init"** (fixed in advance): the template configuration beats I0N on the genre's dev targets when the mean `sound/1` percent is higher **and** more (seed, target) pairs end closer in JND than farther. Reported at 250, 500 and 4,000 renders and on the hold-out pairs. The objective (create's own distance; lower is better) is reported beside it.
- **Coverage count** (fixed in advance): create = P1P at 4,000 renders. A sketch is **covered** when create passes `sound/1` on it in at least two of the three seeds (S7 = the screech, S10 = the sub drop; S7b and S10b are listed apart, and a second count includes them). Also reported: pass in any seed; mean percent at or above 90.
- **What the numbers are not.** The templates were adjusted three times on the dev targets (v0 to v2: `wt` carrier and `eq` in growl, a sub on the key and saws an octave up in reese, the foghorn shape in fm_bass), so the dev numbers are optimistic; the hold-out patches were not used until the grid. The v0 runs also predate the kit fix below and count as exploratory only.
- **Kit fix (additive).** The kit judges a created patch as played at the key create inferred by moving every `osc` `semi` by `intake key - true key`; it left `wt` alone, so every `wt` carrier of an S2a template was judged off by the register shift. `transpose` in `tools/datagen/src/gr3/target.rs` now moves `wt` too (no existing target or bank entry has a `wt`, so no existing score moves; a test pins it).

### Each template against init

Template + init, no bank, against init only, no bank. "Pairs" are (seed, target); "Passed" counts pairs that pass.

| Template | Targets | Renders | Mean sound/1 % (init -> with template) | Mean JND | Better / worse by JND (pairs) | By the objective (pairs) | Passed (pairs) |
| --- | --- | --- | --- | --- | --- | --- | --- |
| growl | dev (3 targets, 9 pairs) | 250 | 83.8 -> 93.1 | 1.50 -> 0.68 | 8 / 1 | 9 / 0 | 3 -> 6 |
| reese | dev (2 targets, 6 pairs) | 250 | 42.3 -> 72.0 | 6.45 -> 9.36 | 5 / 1 | 6 / 0 | 0 -> 3 |
| wobble | dev (5 targets, 15 pairs) | 250 | 59.7 -> 57.6 | 3.88 -> 7.82 | 6 / 5 | 6 / 5 | 0 -> 3 |
| fm_bass | dev (2 targets, 6 pairs) | 250 | 76.4 -> 77.1 | 2.12 -> 2.32 | 4 / 2 | 6 / 0 | 0 -> 3 |
| growl | dev (3 targets, 9 pairs) | 500 | 84.1 -> 94.5 | 1.47 -> 0.56 | 8 / 1 | 9 / 0 | 3 -> 8 |
| reese | dev (2 targets, 6 pairs) | 500 | 42.3 -> 70.6 | 6.45 -> 3.35 | 4 / 2 | 6 / 0 | 0 -> 3 |
| wobble | dev (5 targets, 15 pairs) | 500 | 63.5 -> 58.9 | 3.51 -> 8.95 | 4 / 5 | 9 / 0 | 0 -> 3 |
| fm_bass | dev (2 targets, 6 pairs) | 500 | 76.8 -> 87.8 | 2.09 -> 1.17 | 6 / 0 | 6 / 0 | 0 -> 3 |
| growl | dev (3 targets, 9 pairs) | 4000 | 87.4 -> 95.0 | 1.19 -> 0.52 | 8 / 1 | 9 / 0 | 6 -> 8 |
| growl | hold-out (2 targets, 6 pairs) | 4000 | 53.8 -> 61.1 | 10.28 -> 6.78 | 5 / 1 | 6 / 0 | 3 -> 3 |
| reese | dev (2 targets, 6 pairs) | 4000 | 14.2 -> 87.0 | 24.53 -> 1.44 | 6 / 0 | 6 / 0 | 0 -> 4 |
| reese | hold-out (2 targets, 6 pairs) | 4000 | 51.6 -> 91.3 | 32.04 -> 0.80 | 3 / 0 | 6 / 0 | 3 -> 4 |
| wobble | dev (5 targets, 15 pairs) | 4000 | 48.9 -> 61.8 | 10.13 -> 12.50 | 12 / 3 | 14 / 1 | 0 -> 3 |
| wobble | hold-out (2 targets, 6 pairs) | 4000 | 85.7 -> 96.7 | 1.34 -> 0.41 | 3 / 3 | 6 / 0 | 3 -> 6 |
| fm_bass | dev (2 targets, 6 pairs) | 4000 | 41.9 -> 64.1 | 15.27 -> 23.65 | 4 / 2 | 6 / 0 | 0 -> 4 |
| fm_bass | hold-out (2 targets, 6 pairs) | 4000 | 52.4 -> 65.5 | 14.73 -> 5.28 | 3 / 3 | 6 / 0 | 3 -> 1 |

Per target at 4,000 renders (mean over the three seeds; `sound/1` percent, JND in brackets):

| Template | Target | Set | init only | with the template | Passed (init / template) |
| --- | --- | --- | --- | --- | --- |
| growl | s04-fm-growl | dev | 74.5 (2.30) | 95.6 (0.51) | 0.00 / 1.00 |
| growl | s06-yoi | dev | 91.1 (0.90) | 89.2 (1.05) | 1.00 / 0.67 |
| growl | s08-neuro | dev | 96.6 (0.38) | 100.0 (0.00) | 1.00 / 1.00 |
| growl | h-growl-1 | hold-out | 14.1 (19.85) | 24.2 (13.28) | 0.00 / 0.00 |
| growl | h-growl-2 | hold-out | 93.5 (0.70) | 98.0 (0.28) | 1.00 / 1.00 |
| reese | s03-reese | dev | 14.8 (18.87) | 100.0 (0.00) | 0.00 / 1.00 |
| reese | warm-reese-a | dev | 13.6 (30.18) | 73.9 (2.89) | 0.00 / 0.33 |
| reese | h-reese-1 | hold-out | 3.3 (64.08) | 82.6 (1.61) | 0.00 / 0.33 |
| reese | h-reese-2 | hold-out | 100.0 (0.00) | 100.0 (0.00) | 1.00 / 1.00 |
| wobble | s01-clean-sub | dev | 23.1 (12.56) | 48.3 (6.58) | 0.00 / 0.00 |
| wobble | s02-wobble-eighth | dev | 85.9 (1.32) | 99.5 (0.09) | 0.00 / 1.00 |
| wobble | s05-riddim | dev | 19.7 (28.70) | 4.3 (51.90) | 0.00 / 0.00 |
| wobble | wobble-machine-a | dev | 59.3 (3.87) | 78.9 (1.92) | 0.00 / 0.00 |
| wobble | wobble-machine-b | dev | 56.7 (4.19) | 78.1 (2.03) | 0.00 / 0.00 |
| wobble | h-wobble-1 | hold-out | 75.6 (2.19) | 98.1 (0.27) | 0.00 / 1.00 |
| wobble | h-wobble-2 | hold-out | 95.9 (0.48) | 95.3 (0.54) | 1.00 / 1.00 |
| fm_bass | s04-fm-growl | dev | 74.5 (2.30) | 94.4 (0.62) | 0.00 / 1.00 |
| fm_bass | s09-foghorn | dev | 9.3 (28.25) | 33.9 (46.67) | 0.00 / 0.33 |
| fm_bass | h-fm-1 | hold-out | 8.9 (28.96) | 64.3 (7.52) | 0.00 / 0.33 |
| fm_bass | h-fm-2 | hold-out | 95.8 (0.50) | 66.8 (3.04) | 1.00 / 0.00 |
| fm_bass | s07b-laser | extra | 92.5 (0.77) | 35.0 (44.89) | 0.67 / 0.33 |

Reading, at 4,000 renders (create's default budget):

- **growl** beats init on its dev set at every budget (mean 87.4 to 95.0 %, closer in JND on 8 of 9 pairs, lower objective on 9 of 9) and on the hold-out pairs (53.8 to 61.1 %, 5 of 6). The one loss is S6 yoi (91.1 to 89.2 %): init's heuristic start already passes it.
- **reese** beats init by the widest margin (14.2 to 87.0 % on dev; S3 14.8 to 100 %; hold-out 51.6 to 91.3 %). `warm-reese` reaches 73.9 %, below the pass mark; the sound is heavier than the template's.
- **wobble** beats init at 4,000 renders (48.9 to 61.8 %, 12 of 15 pairs closer, three pairs passing where init has none) and on the hold-out (85.7 to 96.7 %, passes 3 to 6 of 6), but **not at 250 or 500 renders** (59.7 to 57.6 %, 63.5 to 58.9 %). It makes S5 riddim worse (19.7 to 4.3 %: riddim gates the level at 7 Hz, and the template's only route is the cutoff), and it does not pass S1 (48.3 %), which is a sine sub. Its gains are S2 (85.9 to 99.5 %), the two `wobble-machine` anchors (59 and 57 to 79 and 78 %) and `h-wobble-1`.
- **fm_bass** beats init on its dev set at 500 and 4,000 renders (76.8 to 87.8 %; 41.9 to 64.1 %, 4 of 6 pairs closer, passing pairs 0 to 4) and marginally at 250. S4 goes from 74.5 to 94.4 %. S9 is bimodal: in seed 1 the template's own start scores 94.9 % and stays there; in seeds 2 and 3 the search leaves it (2.1 % and 4.8 %, see [the objective overshoots](#findings)). Hold-out: `h-fm-1` 8.9 to 64.3 %, but `h-fm-2` 95.8 to 66.8 % (init passes it), and the laser (S7b, outside the amended set) 92.5 to 35.0 %. fm_bass is a better bass than the others on FM bass sounds and a worse start for a pitch dive.

**Verdict:** each template beats init on its genre's dev targets at 4,000 renders under the pre-registered criterion; wobble does not at 250 and 500 renders; on the hold-out, growl and reese win clearly, wobble wins on mean and passes, fm_bass is mixed (one big win, one loss).

The kit's own `gr3-report` (McNemar on passes, Wilcoxon on JND) cannot confirm any single-seed comparison at these set sizes (4 to 7 pairs per template, 23 for the portfolio): its decision line is "no" everywhere (it needs McNemar and Wilcoxon both below 0.05). Wilcoxon alone reaches 0.0312 (growl, seed 1, five pairs) and 0.0195 and 0.0138 for init only against all four S2a templates in seeds 2 and 3 (23 pairs). The table above is the evidence; it is not a significance claim.

### Create with and without the S2a templates

All 23 targets, equal renders (`P0*` = today's three templates; `P1*` = + the four S2a templates). Wilcoxon on the pairs' percent differences.

| Comparison | Renders | Mean sound/1 % | Mean JND | Better / worse by % (pairs) | By JND | By the objective | Passed (pairs) | Wilcoxon p (pairs) |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| without the bank: today's -> + S2a | 250 | 63.3 -> 71.8 | 12.88 -> 7.99 | 41 / 23 | 41 / 23 | 14 / 50 | 13 -> 24 of 69 | 0.005 |
| with the bank: today's -> + S2a | 250 | 68.0 -> 69.6 | 5.12 -> 6.84 | 15 / 6 | 15 / 6 | 18 / 3 | 12 -> 24 of 69 | 0.168 |
| no bank: init only -> today's | 250 | 72.1 -> 63.3 | 2.76 -> 12.88 | 33 / 33 | 33 / 33 | 65 / 4 | 16 -> 13 of 69 | 0.270 |
| no bank: init only -> + S2a | 250 | 72.1 -> 71.8 | 2.76 -> 7.99 | 35 / 34 | 35 / 34 | 50 / 19 | 16 -> 24 of 69 | 0.560 |
| without the bank: today's -> + S2a | 500 | 69.2 -> 63.0 | 7.43 -> 12.50 | 25 / 36 | 25 / 36 | 27 / 38 | 22 -> 27 of 69 | 0.210 |
| with the bank: today's -> + S2a | 500 | 72.3 -> 76.5 | 4.40 -> 3.96 | 27 / 17 | 27 / 17 | 25 / 22 | 16 -> 29 of 69 | 0.071 |
| no bank: init only -> today's | 500 | 73.0 -> 69.2 | 2.67 -> 7.43 | 34 / 32 | 34 / 32 | 65 / 4 | 16 -> 22 of 69 | 0.648 |
| no bank: init only -> + S2a | 500 | 73.0 -> 63.0 | 2.67 -> 12.50 | 30 / 36 | 30 / 36 | 66 / 3 | 16 -> 27 of 69 | 0.063 |
| without the bank: today's -> + S2a | 4000 | 69.1 -> 72.0 | 9.88 -> 10.29 | 29 / 16 | 29 / 16 | 37 / 13 | 23 -> 30 of 69 | 0.045 |
| with the bank: today's -> + S2a | 4000 | 73.6 -> 74.3 | 7.89 -> 8.59 | 31 / 27 | 31 / 27 | 39 / 24 | 26 -> 32 of 69 | 0.479 |
| no bank: init only -> today's | 4000 | 54.3 -> 69.1 | 12.62 -> 9.88 | 47 / 19 | 47 / 19 | 66 / 3 | 20 -> 23 of 69 | 0.000 |
| no bank: init only -> + S2a | 4000 | 54.3 -> 72.0 | 12.62 -> 10.29 | 49 / 17 | 49 / 17 | 68 / 1 | 20 -> 30 of 69 | 0.000 |

The kit's report on the same pairs (per seed, 23 targets each, at 4,000 renders):

| Seed | Bank | Pairs | Pass today's | Pass + S2a | Today's only | + S2a only | McNemar p (+ S2a better) | Wilcoxon p (+ S2a closer) |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | no | 23 | 9 | 11 | 1 | 3 | 0.3125 | 0.4020 |
| 2 | no | 23 | 7 | 9 | 0 | 2 | 0.2500 | 0.1651 |
| 3 | no | 23 | 7 | 10 | 1 | 4 | 0.1875 | 0.1039 |
| 1 | yes | 23 | 8 | 12 | 0 | 4 | 0.0625 | 0.4609 |
| 2 | yes | 23 | 9 | 9 | 1 | 1 | 0.7500 | 0.1559 |
| 3 | yes | 23 | 9 | 11 | 1 | 3 | 0.3125 | 0.5235 |

Per target at 4,000 renders, the **product configuration** (bank; mean over seeds):

| Target | today's (P0P) | + S2a (P1P) | change |
| --- | --- | --- | --- |
| h-fm-1 | 2.7 (75.32) | 64.8 (11.95) | +62.1 |
| h-fm-2 | 75.6 (2.28) | 52.8 (13.24) | -22.8 |
| h-growl-1 | 68.2 (2.90) | 54.2 (16.67) | -14.0 |
| h-growl-2 | 98.0 (0.28) | 97.9 (0.29) | -0.1 |
| h-reese-1 | 61.2 (3.63) | 61.1 (3.64) | -0.1 |
| h-reese-2 | 100.0 (0.00) | 100.0 (0.00) | +0.0 |
| h-wobble-1 | 97.3 (0.35) | 97.4 (0.33) | +0.2 |
| h-wobble-2 | 95.8 (0.50) | 95.8 (0.50) | +0.0 |
| s01-clean-sub | 52.9 (4.68) | 53.4 (4.62) | +0.4 |
| s02-wobble-eighth | 78.0 (2.00) | 98.5 (0.20) | +20.5 |
| s03-reese | 93.5 (0.68) | 100.0 (0.00) | +6.5 |
| s04-fm-growl | 94.5 (0.61) | 95.6 (0.51) | +1.0 |
| s05-riddim | 55.1 (16.12) | 33.7 (29.36) | -21.4 |
| s06-yoi | 62.3 (10.12) | 82.8 (1.59) | +20.5 |
| s07-screech | 75.1 (2.28) | 24.4 (44.66) | -50.8 |
| s07b-laser | 98.5 (0.22) | 98.5 (0.23) | -0.1 |
| s08-neuro | 100.0 (0.00) | 98.9 (0.13) | -1.1 |
| s09-foghorn | 51.1 (30.61) | 63.7 (40.88) | +12.7 |
| s10-sub-drop | 71.7 (2.58) | 71.2 (2.61) | -0.5 |
| s10b-riser | 78.0 (1.99) | 76.6 (2.10) | -1.3 |
| warm-reese-a | 54.0 (17.74) | 57.0 (17.49) | +3.0 |
| wobble-machine-a | 60.3 (3.80) | 62.0 (3.69) | +1.7 |
| wobble-machine-b | 70.0 (2.85) | 68.9 (2.92) | -1.0 |

Without the bank:

| Target | today's (P0N) | + S2a (P1N) | change |
| --- | --- | --- | --- |
| h-fm-1 | 25.9 (61.27) | 20.0 (48.11) | -5.8 |
| h-fm-2 | 64.9 (12.10) | 61.0 (12.49) | -3.9 |
| h-growl-1 | 61.1 (3.80) | 54.2 (16.67) | -7.0 |
| h-growl-2 | 98.1 (0.27) | 99.1 (0.15) | +1.0 |
| h-reese-1 | 65.6 (3.18) | 66.2 (3.11) | +0.7 |
| h-reese-2 | 100.0 (0.00) | 100.0 (0.00) | +0.0 |
| h-wobble-1 | 98.2 (0.26) | 98.5 (0.23) | +0.3 |
| h-wobble-2 | 96.3 (0.45) | 96.3 (0.45) | +0.0 |
| s01-clean-sub | 65.0 (3.28) | 65.0 (3.28) | +0.0 |
| s02-wobble-eighth | 82.5 (1.61) | 98.5 (0.20) | +16.1 |
| s03-reese | 52.1 (17.52) | 100.0 (0.00) | +47.9 |
| s04-fm-growl | 95.0 (0.57) | 95.5 (0.52) | +0.5 |
| s05-riddim | 29.9 (32.99) | 29.4 (41.05) | -0.5 |
| s06-yoi | 88.7 (1.09) | 83.7 (1.51) | -5.0 |
| s07-screech | 37.7 (27.83) | 42.1 (27.57) | +4.4 |
| s07b-laser | 95.2 (0.52) | 95.0 (0.54) | -0.2 |
| s08-neuro | 100.0 (0.00) | 100.0 (0.00) | +0.0 |
| s09-foghorn | 66.9 (3.07) | 64.8 (16.70) | -2.1 |
| s10-sub-drop | 70.5 (2.67) | 27.1 (40.55) | -43.4 |
| s10b-riser | 74.1 (2.34) | 74.7 (2.29) | +0.6 |
| warm-reese-a | 54.0 (17.74) | 77.7 (2.03) | +23.7 |
| wobble-machine-a | 21.4 (27.29) | 44.9 (15.87) | +23.6 |
| wobble-machine-b | 46.0 (7.33) | 62.9 (3.45) | +16.9 |

Reading:

- With the bank, the S2a templates raise the number of passing pairs at 4,000 renders from 26 to 32 of 69 and at 500 renders from 16 to 29; the mean over the 23 targets moves little (73.6 to 74.3 % at 4,000, 72.3 to 76.5 % at 500; neither significant). Without the bank: 69.1 to 72.0 % at 4,000 (p = 0.045), 63.3 to 71.8 % at 250 (p = 0.005), but 69.2 to 63.0 % at 500.
- The gains sit where a template's structure is needed: S2 (+20.5 and +16.1), S3 without the bank (+47.9), S9 (+12.7), `h-fm-1` (+62.1), the two `wobble-machine` anchors and `warm-reese` without the bank (+17 to +24). The losses are S7 screech (-50.8), `h-fm-2` (-22.8), S5 riddim (-21.4), `h-growl-1` (-14.0), and S10 sub drop without the bank (-43.4).
- Where the winning patch has an S2a structure (it contains a `wt`, `dist`, `ott` or another hidden kind; 20 of 69 pairs at 4,000 renders, 14 at 500), P0P to P1P goes 68.1 to 70.4 % at 4,000 (15 better, 5 worse, passes 5 to 10) and **59.8 to 92.9 % at 500** (13 better, 1 worse, passes 0 to 13). Where it does not (49 and 55 pairs) the change is **75.9 to 75.9 %** (16 better, 22 worse) at 4,000 and 75.5 to 72.3 % at 500: that is run-to-run noise, because four more starts shift the branch indices and so each branch's random stream; single targets swing by 50 points (S7: 75.1 to 24.4 %, S6: 62.3 to 82.8 %). Three seeds do not resolve an effect below about 10 points on one target.
- Equal renders are not equal seconds. CPU time per render, measured in the same runs (a loaded machine, indicative): init only 14.7 ms, today's templates 33.4 ms, with the S2a templates 55.7 ms (no bank); 47.9 and 57.5 ms with the bank. A wall-clock-capped create (the product's 60 s default) will therefore get 1.2 to 1.7 times fewer renders with the S2a templates enabled.

### The ten-sketch coverage count

Passes of `sound/1` (at least 90 %) at 4,000 renders; each cell is mean percent, mean JND and passing seeds of three.

| Sketch | init only (I0N) | today's, bank (P0P) | + S2a, bank (P1P) | today's, no bank (P0N) | + S2a, no bank (P1N) |
| --- | --- | --- | --- | --- | --- |
| s01-clean-sub | 23.1% 12.56J 0/3 | 52.9% 4.68J 0/3 | 53.4% 4.62J 0/3 | 65.0% 3.28J 0/3 | 65.0% 3.28J 0/3 |
| s02-wobble-eighth | 85.9% 1.32J 0/3 | 78.0% 2.00J 0/3 | 98.5% 0.20J 3/3 | 82.5% 1.61J 0/3 | 98.5% 0.20J 3/3 |
| s03-reese | 14.8% 18.87J 0/3 | 93.5% 0.68J 3/3 | 100.0% 0.00J 3/3 | 52.1% 17.52J 0/3 | 100.0% 0.00J 3/3 |
| s04-fm-growl | 74.5% 2.30J 0/3 | 94.5% 0.61J 3/3 | 95.6% 0.51J 3/3 | 95.0% 0.57J 3/3 | 95.5% 0.52J 3/3 |
| s05-riddim | 19.7% 28.70J 0/3 | 55.1% 16.12J 0/3 | 33.7% 29.36J 0/3 | 29.9% 32.99J 0/3 | 29.4% 41.05J 0/3 |
| s06-yoi | 91.1% 0.90J 3/3 | 62.3% 10.12J 1/3 | 82.8% 1.59J 1/3 | 88.7% 1.09J 1/3 | 83.7% 1.51J 1/3 |
| s07-screech | 7.7% 33.21J 0/3 | 75.1% 2.28J 0/3 | 24.4% 44.66J 0/3 | 37.7% 27.83J 0/3 | 42.1% 27.57J 0/3 |
| s07b-laser | 92.5% 0.77J 2/3 | 98.5% 0.22J 3/3 | 98.5% 0.23J 3/3 | 95.2% 0.52J 2/3 | 95.0% 0.54J 2/3 |
| s08-neuro | 96.6% 0.38J 3/3 | 100.0% 0.00J 3/3 | 98.9% 0.13J 3/3 | 100.0% 0.00J 3/3 | 100.0% 0.00J 3/3 |
| s09-foghorn | 9.3% 28.25J 0/3 | 51.1% 30.61J 0/3 | 63.7% 40.88J 2/3 | 66.9% 3.07J 0/3 | 64.8% 16.70J 2/3 |
| s10-sub-drop | 55.2% 4.38J 0/3 | 71.7% 2.58J 0/3 | 71.2% 2.61J 0/3 | 70.5% 2.67J 0/3 | 27.1% 40.55J 0/3 |
| s10b-riser | 61.2% 3.63J 0/3 | 78.0% 1.99J 0/3 | 76.6% 2.10J 0/3 | 74.1% 2.34J 0/3 | 74.7% 2.29J 0/3 |

| Count | I0N | P0P | P1P | P0N | P1N |
| --- | --- | --- | --- | --- | --- |
| covered (pass in >= 2 of 3 seeds) | 2/10 | 3/10 | 5/10 | 2/10 | 5/10 |
| pass in any seed | 2/10 | 4/10 | 6/10 | 3/10 | 6/10 |
| mean sound/1 >= 90 % | 2/10 | 3/10 | 4/10 | 2/10 | 4/10 |
| 12 sketches (S7b, S10b included), >= 2 of 3 | 3/12 | 4/12 | 6/12 | 3/12 | 6/12 |

**The count: create with the S2a templates (P1P, bank, 4,000 renders) covers 5 of the 10 sketches: S2, S3, S4, S8 and S9; S6 passes in one seed (6 of 10 in any seed).** With S7b (3 of 3 seeds) and S10b (0 of 3) the count is 6 of 12. Without the S2a templates (P0P): 3 of 10 (S3, S4, S8), 4 of 12. Init alone: 2 of 10 (S6, S8). This count replaces report 04's judged percentages in GS2a's table, with these cautions:

- It counts `sound/1` passes, a perceptual closeness, not a structural reproduction: S3, S4 and S8 pass without any S2a structure (bank neighbours and today's templates reach them), and with the templates the winner has an S2a structure on S2, S3 (100 % against 93.5 %), S5 (which fails), S9 (two seeds) and S4 (one seed). Only S2 and S9 are covered *because of* the templates.
- At the small budgets the templates matter more: at 250 and 500 renders the count is 5 of 10 with the templates and 1 of 10 without (S8 only); the bank's neighbours catch up at 4,000.
- Uncovered either way: S1 (53 %), S5 (34 to 55 %), S6 (62 to 83 %; one seed passes), S7 (24 to 75 %), S10 (71 %), S10b (77 %): sub sines, riddim gating, vowels, screech and risers are no template's genre.

Counts by budget (covered, with "pass in any seed" in brackets):

| Renders | I0N | P0N | P1N | P0P | P1P |
| --- | --- | --- | --- | --- | --- |
| 250 | 1 (1) | 1 (1) | 5 (5) | 1 (1) | 5 (5) |
| 500 | 1 (1) | 2 (4) | 5 (6) | 1 (1) | 5 (5) |
| 1,000 | 1 (1) | 2 (4) | 5 (6) | 1 (1) | 5 (5) |
| 2,000 | 1 (1) | 3 (5) | 6 (6) | 2 (4) | 5 (5) |
| 4,000 | 2 (2) | 2 (3) | 5 (6) | 3 (4) | 5 (6) |

### S4m4: the metallic S4 cut to four voice effects

`s04m4-fm-growl-metallic-4fx` (dist, band-pass, bitcrusher, comb) is reachable. Two seeds at 4,000 renders: init only 79.5 %; today's templates with the bank 96.2 and 95.5 % (a neighbour); growl alone 99.8 and 97.3 %; all S2a templates with the bank 98.8 and 100 %. The six-effect S4m cannot be built (`validate` refuses it), so its cost against S4m4 is not measured; the evidence for the cap is that the one sketch of ten that needs more than four voice effects is the metallic variant, and that cut to four it is matched.

### Findings

1. **The objective overshoots.** Create keeps lowering its own objective after `sound/1` has peaked. For the pairs whose winner has an S2a structure at 500 renders (14 pairs), mean `sound/1` is 91.7, 92.9, 92.7, 85.5 and 81.7 % at 250, 500, 1,000, 2,000 and 4,000 renders while the mean objective falls from 0.664 to 0.528; for the 20 pairs with an S2a winner at 4,000 renders it is 66.2, 82.1, 75.3, 67.8 and 70.4 % (objective 0.909 to 0.682). S9 with fm_bass is the sharpest case: the template's own start scores 94.9 % at 500 renders in all three seeds; at 4,000 renders seeds 2 and 3 have a lower objective (0.470 and 0.451 against 0.478) and `sound/1` 2.1 % and 4.8 %: the search left an almost exact structure for a marginal logmel gain (the `wt` `semi` moved by +36.9 and +5.9 st). The same disagreement explains S10 sub drop without the bank (70.5 to 27.1 % on average): in seeds 1 and 2 the S2a winner ends at 4.3 % and 3.1 % with a lower objective (0.428 and 0.410) than today's templates (0.630 and 0.606) or init (0.517), and `enth compare`'s logmel distance of the seed-1 patch to the reference is 0.357 against 0.435 for init's and 0.530 for today's; in seed 3 the heuristic start wins (74.0 %). Template design cannot fix it; whoever owns the create objective or the stopping rule should know. A stop at about 500 renders would have kept the templates' early result.
2. **Free axes.** The templates leave 134 (growl), 45 (reese), 44 (wobble) and 75 (fm_bass) axes under the dev-target performance; 59 of growl's are the MSEG's (11 global and 3 per point for 16 points), which the observability mask counts all of (it does not know `points`); only the 12 of the four drawn points are live. A `lfo` has six. The mask rule is a small change in `crates/enth-challenge/src/random/mask.rs` (a shared hotspot, not touched).
3. **Intake keys.** The intake's key is erratic on these sounds (S2, S4 and S8 unpitched: key 48; S6: 53; S7: 60; S10: 37; S1, S3, S5, S9: 29), so the kit's register shift is large (-19 to +12 st) and the +-48 st clamp in `transpose` can wreck a patch that scored well before the shift. `enth_solver::nearest::transpose` (the k-NN neighbours' shift) and the heuristic (it tunes the first `osc` only) still know nothing about `wt`.
4. **Structure alone does not guarantee a pass.** The oracle arm (each sketch's own structure at default values, no bank) passes 4 of 15 dev targets at 4,000 renders; S9 reaches 93.7 % with its own structure and 94.9 % with fm_bass (at 500 renders).
5. **Seed variance is large.** S5 riddim with init only: 46.5 % in one seed, 7.7 % in another; S9 with fm_bass: 94.9 or 2 %. All counts above are three-seed counts and move by one sketch with a fourth seed.

### What other lanes need to know

- **INT-S2a:** clear the flags and the templates join create (see [Switching them on](#switching-them-on)); run the create goldens and the plugin's Create view with eight starts. The S2a templates make a render about 1.2 to 1.7 times dearer.
- **Whoever owns the create objective** (R3b or the coordinator): finding 1.
- **The caps** (coordinator): `MAX_VOICE_FX` 4 is met exactly by S4 and growl; only S4m (six) exceeds it; `MAX_FX` 4 is not met by any sketch (at most 3). The evidence is above; nothing is changed.
- **CAT's sketch gaps** are confirmed by building the sketches (the first six rows of the gap table); three more are listed there (noisy unison detune, the RC-exponential `adsr` that keeps S7 and S10 from taking report 04's "D250" literally, and no reverb swell for S10b).


### Re-running it

`enth-datagen gr3 run --job <job.json>` takes the new optional job field `templates` (`tools/datagen/src/gr3/job.rs`): absent = create's own templates plus the enabled S2a ones (today none), a list = exactly these (names of today's three or of the S2a four, or paths to patch files), an empty list = `init` only. A job pins each target file by sha256, so a changed sketch needs a new job. The lane's scratch folder holds the jobs (`jobs/g-<config>-s<seed>.json`, built by `work/mkgrid.sh` and `work/mkjob.py`), the run records (`runs/`), the analysis (`work/analyze.py`, `work/mdtables.py`, `work/kitreport.py`), the bit-identity check (`work/bitid.sh`) and the WAV exports (`wav/INDEX.txt`: each sketch's reference render, the four templates, and create's best match for every target at 4,000 renders for the three seeds with and without the S2a templates).
