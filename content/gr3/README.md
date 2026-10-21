# content/gr3: GR3's held-out sets

GR3 ([docs/r3.md](../../docs/r3.md#gr3-and-its-hygiene), [docs/phase-3.md](../../docs/phase-3.md)) asks three things of "create from sound":

1. model + search beats the best search alone on held-out synth sounds at equal time;
2. a blind listening test prefers it on real and degraded sounds;
3. every result arrives in under 5 minutes.

This folder holds the sets that clauses 1 and 2 are measured on, **frozen and hashed before any training**, plus the exclusion lists that keep training off them. Lane S0s created the folder; lane DS filled it on 2026-10-21 (freeze F0s). Nothing here is trained on.

## The files

| File | Holds |
| --- | --- |
| `synth.toml` | Clause 1's synth targets: generator-v1 codes from a reserved seed block, the test half of the preset candidates, the owner's four G1 sounds (placeholders), the dev split, and the `r3/test` stream's definition. Each target is a code or a patch file plus a performance, never audio |
| `presets/` | Byte copies of the 11 test-half candidate presets, so a later retune of `content/presets-candidates/` never moves a GR3 target |
| `real.toml` | Clause 2's references: 40 clean ones in four strata, 10 owner slots with their defaults, 8 reserves. Each has its URL, licence (read from the source page), attribution, duration and audio SHA-256. **No audio is committed** |
| `degraded.toml` | Every degraded item, synth and real: source, held-out profile, seed, what the plan drew, and the plan's hash. Lane DEG's chain makes them; they are judged against the **clean** source |
| `exclude-synth.txt`, `exclude-real.txt`, `exclude-degraded.txt` | What training must never contain: seed ranges, codes, fingerprints, preset stems and file hashes; Freesound ids, packs and audio hashes; the held-out pool, its variants and seeds |
| `hashes.txt` | The SHA-256 of every frozen file above (`sha256sum -c` format, paths relative to this folder) |
| `check.py` | The check: recomputes the hashes and re-derives every rule (below). Standard library only |
| `runs.jsonl` | Every test run, appended (build, date, load, the input and output hashes of degraded files), never dropped or rewritten: `enth.gr3-run/1` lines that `cargo xtask gr3-run` writes and `gr3-report` reads ([docs/gr3.md](../../docs/gr3.md)). Lane EVK created it on 2026-10-21 with the arm-A core baseline (a render-equivalent estimate: no G1 or owner item, so amendment 1 stays open); EVK and EVb append to it. Not in `hashes.txt`: it grows |

## The sets

| Set | Size | Where | Rule |
| --- | --- | --- | --- |
| Generator targets (test) | 92: levels 1–5 hold 12, 20, 20, 20, 20 | `synth.toml`, `kind = "generator"` | Seeds `770000000000 + 10000·level + k` in order, skipping silent targets, repeated fingerprints and targets within one minimum step of an earlier one. Level 1 has only 14 distinct targets in 500+ seeds (12 test, 2 dev), so its share went to levels 2–5 |
| Hand-made (test) | 22 renders: 11 presets × 2 notes | `kind = "preset"` | The test half of the 22 candidates. Note a is the README's note, hold and clip at velocity 100; note b is a fifth up at velocity 80 |
| Owner G1 (test) | 8 renders: 4 sounds × 2 notes (placeholders) | `kind = "owner-g1"` | `content/presets/{bass,pad,pluck,lead}.patch.json`, once extracted |
| **Clause 1 total** | **122** (114 until the G1 sounds exist) | | r3.md's "about 120" |
| Core | 40 of the 122 | `core = true` | Per generator level, the ⌈n/3⌉ ids with the smallest SHA-256 of `gr3/core/v1:<id>`, plus the 4 test presets picked the same way (both notes). The pre-declared 300 s tail, and the degraded core |
| Degraded synth | 488: the 122 × `mp3`, `noise-hum`, `reverb`, `clip-limit` (160 in the core) | `degraded.toml`, `[items] synth` | Seed `770100000000 + 10·t + p` |
| Dev | 30: 20 generator targets (2, 4, 5, 4, 5 per level) + 5 anchor-half presets × 2 notes | `split = "dev"` | Seeds `770000500000 + 10000·level + k`. The anchor presets are training anchors, so a model looks better on them: dev is for tuning, never a measure |
| `r3/test` stream | about 2,000 | `[[stream]]` | `ds1`'s own stream key (lane DG); development curves only |
| References (clean) | 40: subtractive 10, FM/wavetable/digital 10, acoustic and percussive 10, owner 10 | `real.toml`, `role = "test"` | Agent picks by description, blind to any result; see below |
| References (degraded) | 20: 5 per stratum | `degraded.toml`, `[items] real` | Per stratum, the 5 ids with the smallest SHA-256 of `gr3/real-degraded/v1:<id>` get, in that order, the chains `mp3`, `upload-noisy`, `upload-room`, `upload-loud`, `phone-recording`. Seed `770200000000 + k` |

**The preset split.** Sort the 22 files of `content/presets-candidates/` by folder, then by SHA-256 of `gr3/preset-split/v1:<stem>`. Even positions are the test half, odd positions the anchors:

- **Test:** Octave Arp, Acid Snap, Sub Floor, Drawbar Organ, Tremolo Keys, Breath Flute, Sync Tear, Gate Keeper, Aurora, Snare Crack, Poly Stab.
- **Anchors:** Wobble Machine, Warm Reese, Noise Riser, Soft Bell, Solar Lead, Pixel Square, Velvet Pad, Night Drift, Thump Kick, Glass Pluck, Wind Tunnel.

Each type folder splits evenly (bass 2/2, lead 2/2, pad 2/2, keys 2/1, perc and pluck 1/1). **No prior is ever fitted on the test half.** It is never a `ds1` perturbation anchor, template, start or k-NN bank entry, and nothing is tuned on it. If the owner promotes a test-half candidate to a factory preset, it stays a GR3 target and stays out of every bank. `exclude-synth.txt` lists its file hashes for that reason.

**The held-out degradations** are lane DEG's `heldout` pool (`ml/enth_ml/degrade/pools.toml` at DEG commit 4cd21c1, SHA-256 `da671fcb…`). It is disjoint from the `train` pool by construction, and DEG tests that.

| Profile | Draws | Training has instead |
| --- | --- | --- |
| `mp3` | libshine CBR 32–96 kbps; LAME VBR V5–V9 or ABR 32–96 kbps | LAME CBR only |
| `noise-hum` | blue, violet or velvet noise, or AM band-limited pink babble, at 5–20 dB SNR; plus a 6–12-harmonic buzz at 15–29 dB SNR | white, pink, brown at 15–50 dB; 2–4-harmonic hum at 30–60 dB |
| `reverb` | synthetic rooms at RT60 0.4–0.6, 1.1–1.4 or 2.0–2.5 s, or a flutter echo at 0.8–1.6 s | 0.15–0.35, 0.7–1.0, 1.5–1.9 s; no flutter |
| `clip-limit` | hard clip or limiter at +13 to +18 dB; atan or cubic soft clip at +6 to +18 dB | hard, tanh, limiter at +3 to +12 dB |
| `upload-noisy`, `upload-room`, `upload-loud` | `noise-hum`, `reverb` or `clip-limit`, followed by an `mp3` codec | — |
| `phone-recording` | a living room, phone-speaker EQ (high-pass 420–800 Hz, low-pass 2.4–3 kHz), a mono downmix, then AMR-NB or GSM at 8 kHz | — |

`phone-recording` stands in for the owner's phone re-recordings. If the owner makes those, they become a new set version; they never replace items in this one.

**The real set.**

- **Source.** Every agent pick is a Freesound sound whose page shows **CC0**, read on 2026-10-21 (48 of 48: 30 test, 10 owner defaults, 8 reserves).
- **How they were picked.** By stratum, title, description and tags only. Nothing was played or run through enth.
- **Diversity and provenance.** At most two sounds per uploader and one per pack. Sounds whose page says they were ripped from a record, a video or a commercial pack were skipped.
- **Disjoint from FSD50K.** No pick is an FSD50K clip: three that were (and one reserve) were swapped out after checking FSD50K v1.0's ground truth.
- **The audio.** Each reference is the sound's public HQ Ogg preview: Vorbis at about 192 kbps stereo or 110 kbps mono, at the original sample rate, with no account needed. It is stored in `~/code/enth-scratch/r3-data/freesound/` and hashed in `real.toml`.
- **The originals** need a Freesound account. Fetching them is optional, and they never replace a preview within this set version.
- **Short sounds are in.** Three references are under 200 ms (a closed hi-hat, a rack tom, a clave), and nothing was filtered by duration.

## The freeze

- **Frozen on 2026-10-21, before any training.** `hashes.txt` pins every frozen file. After this, an entry changes only through a new, named set version (`set_version = 2` in new files beside these), and the old version stays so earlier results stay checkable. Reserves replace a reference only if its source disappears before the first test run, and that replacement is a new version too.
- **One amendment is allowed, until the first test run that includes an item it fills.** It fills placeholders and nothing else:
  - the four G1 entries' `sha256` (and their `status`), with the extracted files' hashes added to `exclude-synth.txt`;
  - each owner slot `owner-NN`: its file's `sha256`, `licence` and source, or "uses owner-default-NN".

  Its commit changes only those fields and `hashes.txt`, says "GR3 amendment 1" in its message, and is recorded in a line at the end of this README.
- **A test run never includes an amendable item while its entry is unfilled** (a G1 target whose `sha256` is still `pending`, an owner slot not yet filled). `cargo xtask gr3-run` refuses one. A test run that leaves out every amendable item (the arm-A core baseline: 32 generator targets and 8 test-preset renders) does not close the amendment; the first test run that includes a filled amendable item does.
- *Ruled by the coordinator on 2026-10-21, before any test run, while `runs.jsonl` was empty* (lane EVK raised it). The rule first read "before the first test run in `runs.jsonl`", which the arm-A baseline would have closed before the owner's G1 files exist. Nothing a run leaving out the amendable items can show depends on them, so the amendment's content stays independent of every result.
- **Tuning happens on dev only.** No search setting, threshold or prior is tuned on a test item.
- **Every test run is appended** to `runs.jsonl`, whatever it showed. That includes runs on degraded items, with the ffmpeg version and the input and output SHA-256 from DEG's record, since a degraded file's bytes depend on the ffmpeg build.
- **The listening key stays closed** until every listener is done, and whoever prepares the stimuli does not audition them.

## Checking the freeze

```sh
python3 content/gr3/check.py          # offline, a second: exit 0 = every check passes
sha256sum -c content/gr3/hashes.txt   # the hashes alone (run inside content/gr3/)
```

`check.py` recomputes every hash and re-derives every rule from the files themselves:

- each code from its level and seed, with code v1's Crockford base32 and GF(32) check;
- each seed from its formula;
- the preset split from `content/presets-candidates/`, the core, both degraded rules and their seeds;
- the sizes; test and dev disjoint by id, seed, code and fingerprint;
- licences being CC0 or CC BY, one sound per pack, at most two per uploader;
- every id, hash, stem and seed present in the exclusion lists.

Options:

- `--enth <an enth build>` re-derives every generator code and fingerprint. It does no render; generator v1 is frozen, so any build that has it agrees.
- `--reselect` (with `--enth`) re-runs the whole seed selection: 6–10 minutes.
- `--deg ml` recomputes all 508 plan hashes with lane DEG's chain.
- `--audio ~/code/enth-scratch/r3-data/freesound` checks each reference file.

On 2026-10-21 all four passed: 112 targets, 112 seeds re-selected, 508 plans and 48 files.

## How a training pipeline asserts the exclusions

Lane DG implements this in `enth-datagen exclude`, and every pipeline that builds training data, a k-NN bank or a prior runs it.

1. **Read the three `exclude-*.txt` files.** Each non-empty line, minus anything after `#`, is `<type> <value>`. The value of `preset-name` is the rest of the line. Fail on an unknown type, so a newer list is never half-read. Record the SHA-256 of all three files in the dataset's manifest.
2. **Assert, for every training item, that it matches no entry.** Fail hard on the first match, naming the item and the entry.
   - `seed-range`: the item's generator or sampler seed lies outside the range, whatever its stream or spec.
   - `stream`: it was drawn from no listed stream (`*` is a prefix).
   - `code` and `fingerprint`: no target has them. Compute the fingerprint for every generator-v1 target the pipeline draws.
   - `preset-stem`, `preset-name`, `file-sha256`: no anchor, prior, template, start or bank entry is a test preset, or a byte copy of one.
   - `path`: nothing is read from under these paths.
   - `freesound-id`, `freesound-pack`, `audio-sha256`: no audio comes from these sounds or packs, or has these bytes, whether it comes from Freesound, FSD50K (`fname` is the Freesound id), a mirror or anything else.
   - `freesound-uploader`: drop these uploaders where the source records the uploader. This one is advisory: FSD50K's names can differ from Freesound's (sgossner's clips are "Samulis" there), so the hard guards are the ids and packs.
   - `degrade-pool`, `degrade-variant`, `degrade-profile`, `degrade-seed-range`: every degradation record in training says `pool = train`, and shows no held-out variant, profile or seed.
3. **Dedupe against the sets.** After every refresh, dedupe training against every test and dev target within about one just-noticeable step (r3.md, `ds1`'s pooled log-mel check). The lists catch exact leaks; this catches near ones.
4. **Bank and templates (lane TP, EVK).** R3a's start portfolio and k-NN bank never contain a test target, a test preset or its byte copy. Dev runs also use a bank without the dev targets.

## Your part (the owner)

- **Ten picks** (optional, phase-3 "your part"): sounds you care about, saved as `~/code/enth-scratch/owner/r3-picks/owner-01.<ext>` … `owner-10.<ext>` with a line each on source and licence. Empty slots use the agent defaults `owner-default-NN`, a set of familiar archetypes: supersaw, Reese bass, 808 kick, hoover, a superwave lead, a wobble sub bass, a Rhodes, a saw bass, a piano-bell and a digital pad. Your files stay local unless they are CC0 or yours.
- **Downloads: none needed.** The 48 previews are fetched (2.7 MB). The 48 originals (about 40 MB in total, sizes in `real.toml`) need your Freesound account and are optional.
- **Phone re-recordings:** optional (about 30 min); see the freeze rules above.

## Licence notes (phase-3's open check, read 2026-10-21)

| Item | Licence, as read at the source | Use here |
| --- | --- | --- |
| opencore-amr (AMR-NB encoder, ffmpeg's `libopencore_amrnb`) | Apache-2.0 (its `LICENSE`, SourceForge) | `phone-recording`, through DEG's ffmpeg subprocess, evaluation only |
| vo-amrwbenc (AMR-WB) | Apache-2.0 (its `COPYING`) | not used by these sets |
| libgsm (GSM 06.10) | permissive: "any use of this software is permitted provided that this notice is not removed" (TU Berlin, 1994, `COPYRIGHT`) | `phone-recording`, evaluation only |
| WHAM! noise | CC BY-NC 4.0 (wham.whisper.ai) | evaluation only; never for shipped weights |
| OpenSLR 28 (RIRS_NOISES) | the page says Apache 2.0. Its sources: AIR v1.4 is MIT (RWTH page); RWCP-SSD is "for research and development purposes only" (NII-SRC page); the REVERB 2014 site did not resolve | for lane AUG: shipped weights should skip the RWCP-derived files until that is settled |
| Freesound | each sound's own CC licence governs; the web terms have no clause on training. The API terms bar robots and scraping for API users, and robots.txt disallows `/search/` | bulk access for training goes through the API with your account, or through FSD50K |

## Rules

- **Licences are read from the source**, per clip, and recorded in the manifest. Audio is committed only when it is CC0 or the owner's; for now none is committed, and the root `.gitignore` excludes `*.wav`.
- **Short sounds are valid.** The sets include sounds under 200 ms; nothing is ever filtered by duration.
- **Privacy.** No personal detail goes into any manifest, request, header, URL or payload; where a service asks for contact information, use `enth-research/0.1`.
- **Byte for byte.** Files here are compared and hashed byte for byte, so `.gitattributes` keeps them LF on every platform (`content/gr3/** text=auto eol=lf`, audio and archives binary).

## Amendments

None yet.

Rule changes (not amendments: no set entry changed):

- 2026-10-21, ruled by the coordinator before any test run, `runs.jsonl` empty: amendment 1 stays allowed until the first test run that includes an item it fills, and no test run includes an unfilled amendable item ("The freeze" above).
