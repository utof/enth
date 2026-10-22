# content/gr2

Data for gate GR2's human half ([docs/learn.md](../../docs/learn.md#gr2), [owner sheet 4](../../docs/r2-owner.md)), made by R2 lane INT2 on 2026-10-22 with this build's generator v1 and scorers (`sound/1`, `structure/1`, `combined/1`), engine 0.3.0.

| File | What it is |
| --- | --- |
| `forms.toml` | The test forms (`enth.gr2-forms/1`): six families, one long challenge code per family on each of the forms A and B (the parallel pre- and post-test forms), M (the mid-test, after lesson 5 of the tour) and D (the optional delayed test); the 180 s limit, the 10-point structure step and the mid-test's place. `cargo xtask gr2-report` reads it |
| `answers-template.toml` | The answers file each participant fills (`enth.gr2-answers/1`): intake, a block per tour lesson (vote, pace, what they learned, the coder's verdict), the debrief. The friend zip carries it as `gr2-answers.toml` |

## The families

Each family is one category of the Recreate generator (docs/challenge.md, "Levels and what each randomizes"), switched on alone with a long code's amounts, so a test item is an ordinary Recreate round that the round log records. The curriculum follows the generator's order, so each family maps to tour lessons:

| Family | Settings (`enth challenge new --difficulty D --amount …`) | Tour lessons |
| --- | --- | --- |
| `oscillators` | difficulty 0; oscillators 100, the rest 0 | Waveforms |
| `sources` | difficulty 100; oscillators 100, structure 100, the rest 0 | Two oscillators, Noise, Glide |
| `filter` | difficulty 25; filter 100 (drive included), the rest 0 | Low-pass cutoff, Drive |
| `envelopes` | difficulty 50; envelopes 75 (the amplitude envelope, env 2 on the cutoff), the rest 0 | Decay and sustain, Routes and depth |
| `modulation` | difficulty 75; modulation 100, the rest 0 | Vibrato |
| `effects` | difficulty 100; effects 100, the rest 0 | Reverb space, Compressor level |

Structure alone draws nothing a player can hear (osc 2 and the noise switch on at their defaults), so `sources` adds the oscillators. The macro knob has no generator category, so the Macros lesson has no family; How a lesson works teaches no control.

## How the codes were made

Per family and form, seeds were tried in order from 21001 (A), 22001 (B), 23001 (M) and 24001 (D), and the first kept that met every rule:

1. `enth challenge new` makes it (the target is not silent);
2. for `sources`, osc 2 or the noise is among the drawn parameters, and at most 7 are drawn (a 3-minute item);
3. the untouched start does not pass and its sound score is at most 80 % (`enth match`'s `start_score`), so an item is never won by submitting the start;
4. the CMA-ES floor passes (`enth match --target <code> --free observable --budget 1500 --workers 3`, GR1's floor), so every item is reachable by changing parameters; when 8 seeds gave none, the one with the best floor was kept (marked **fails** below);
5. `modulation`'s mid-test item draws both LFOs, as its A, B and D items do (seeds from 23009, `repick.py`), so the mid-test is not easier than the pre-test on a family the tour has not taught yet (the multiple baseline);
6. `effects` M and D: rule 3 left none of the first 8 seeds (most starts score 81–99 % on sound), so seeds were screened by rule 3 alone with a 10-render `enth match` (its `start_score`), the first that met it was kept, and its floor measured after (`screen.py`; M took 10 seeds, D 3).

The scripts are lane INT2's (`gen_forms.py` and `repick.py` in its scratch folder); the result is deterministic for this build.

<!-- gr2-items:start -->
| Family | Form | Seed | Level | Drawn | Phrase | Start: sound / structure % | CMA-ES floor |
| --- | --- | --- | --- | --- | --- | --- | --- |
| oscillators | A | 21001 | 1 | `osc1.semi`, `osc1.wave` | held | 4.5 / 50.0 | passes (combined 98.0 %) |
| oscillators | B | 22002 | 1 | `osc1.semi`, `osc1.wave` | held | 4.4 / 50.0 | passes (combined 100.0 %) |
| oscillators | M | 23001 | 1 | `osc1.semi`, `osc1.wave` | held | 4.5 / 50.0 | passes (combined 98.0 %) |
| oscillators | D | 24002 | 1 | `osc1.semi`, `osc1.wave` | held | 3.6 / 0.0 | passes (combined 100.0 %) |
| sources | A | 21001 | 5 | `osc1.semi`, `osc1.wave`, `osc2.fine`, `osc2.level`, `osc2.semi`, `osc2.wave` | held | 4.4 / 22.2 | passes (combined 80.6 %) |
| sources | B | 22001 | 5 | `noise1.color`, `noise1.level`, `osc1.pw`, `osc1.semi`, `osc1.wave` | held | 4.5 / 28.6 | passes (combined 84.8 %) |
| sources | M | 23004 | 5 | `osc1.semi`, `osc1.wave`, `osc2.fine`, `osc2.level`, `osc2.semi`, `osc2.wave` | held | 4.5 / 22.2 | passes (combined 87.5 %) |
| sources | D | 24004 | 5 | `noise1.color`, `noise1.level`, `osc1.semi`, `osc1.wave` | held | 4.6 / 33.3 | passes (combined 86.3 %) |
| filter | A | 21001 | 2 | `filter1.cutoff`, `filter1.drive`, `filter1.mode`, `filter1.resonance` | held | 30.3 / 18.4 | passes (combined 100.0 %) |
| filter | B | 22001 | 2 | `filter1.cutoff`, `filter1.drive`, `filter1.mode`, `filter1.resonance` | held | 27.2 / 41.6 | passes (combined 100.0 %) |
| filter | M | 23003 | 2 | `filter1.cutoff`, `filter1.drive`, `filter1.mode`, `filter1.resonance` | held | 46.0 / 75.0 | passes (combined 99.9 %) |
| filter | D | 24001 | 2 | `filter1.cutoff`, `filter1.drive`, `filter1.mode`, `filter1.resonance` | held | 18.4 / 63.9 | passes (combined 100.0 %) |
| envelopes | A | 21001 | 3 | `env1.attack`, `env1.decay`, `env1.release`, `env1.sustain`, `env2.decay`, `env2.sustain` | held | 69.5 / 32.7 | passes (combined 82.0 %) |
| envelopes | B | 22001 | 3 | `env1.attack`, `env1.decay`, `env1.release`, `env1.sustain`, `env2.decay`, `env2.sustain` | held | 48.4 / 22.1 | passes (combined 82.0 %) |
| envelopes | M | 23001 | 3 | `env1.attack`, `env1.decay`, `env1.release`, `env1.sustain`, `env2.decay`, `env2.sustain` | held | 56.4 / 41.4 | passes (combined 76.1 %) |
| envelopes | D | 24001 | 3 | `env1.attack`, `env1.decay`, `env1.release`, `env1.sustain`, `env2.decay`, `env2.sustain` | held | 56.8 / 23.7 | passes (combined 80.6 %) |
| modulation | A | 21005 | 4 | `lfo1.rate`, `lfo1.shape`, `lfo2.rate`, `lfo2.shape` | held | 78.2 / 0.0 | **fails** (combined 57.8 %) |
| modulation | B | 22007 | 4 | `lfo1.rate`, `lfo1.shape`, `lfo2.rate`, `lfo2.shape` | held | 78.7 / 0.0 | passes (combined 68.5 %) |
| modulation | M | 23009 | 4 | `lfo1.rate`, `lfo1.shape`, `lfo2.rate`, `lfo2.shape` | held | 80.0 / 0.0 | **fails** (combined 59.7 %) |
| modulation | D | 24005 | 4 | `lfo1.rate`, `lfo1.shape`, `lfo2.rate`, `lfo2.shape` | held | 67.5 / 0.0 | **fails** (combined 54.1 %) |
| effects | A | 21001 | 5 | `reverb1.decay`, `reverb1.high_cut`, `reverb1.room`, `reverb1.wet` | held | 76.8 / 16.7 | **fails** (combined 70.2 %) |
| effects | B | 22003 | 5 | `reverb1.decay`, `reverb1.high_cut`, `reverb1.room`, `reverb1.wet` | held | 78.2 / 16.7 | **fails** (combined 70.4 %) |
| effects | M | 23010 | 5 | `reverb1.decay`, `reverb1.high_cut`, `reverb1.room`, `reverb1.wet` | held | 66.1 / 16.7 | **fails** (combined 65.9 %) |
| effects | D | 24003 | 5 | `reverb1.decay`, `reverb1.high_cut`, `reverb1.room`, `reverb1.wet` | held | 71.8 / 16.7 | **fails** (combined 69.9 %) |
<!-- gr2-items:end -->

**Reading it.** `modulation` and `effects` are the hard families: their starts already score 66–80 % on sound (an LFO or a reverb on a held note moves `sound/1` little), and the CMA-ES floor fails on 7 of their 8 items, so these items will mostly move on structure %, not on passes. The other four families' floors pass on every form. Every `effects` item happens to draw the reverb alone (the compressor is drawn 30 % of the time), and no `sources` item drew glide (it needs the legato phrase, 20 % of the time), so the Compressor level and Glide lessons have no item of their own. The mid-test items' starts differ from the pre-test's (`filter` M starts at 46 % sound against 27–30 % for A and B), so read the mid-test as taught against not-yet-taught families, not item by item. A and B are counterbalanced across people (odd ids take A first), so a harder form does not favour the pre- or the post-test on average.

Re-make the codes after a generator or scorer change (a long code carries the engine version and the target's fingerprint prefix; another engine plays it with a warning, a changed generator refuses it).
