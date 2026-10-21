# content/gr3 — GR3's held-out sets

GR3 ([docs/r3.md](../../docs/r3.md#gr3-and-its-hygiene), [docs/phase-3.md](../../docs/phase-3.md)) asks three things of "create from sound": that model + search beats the best search alone on held-out synth sounds at equal time; that a blind listening test prefers it on real and degraded sounds; that every result arrives in under 5 minutes. This folder holds the sets the first two are measured on, **frozen and hashed before any training**, and the log of every test run.

Lane S0s (R3, freeze point F0s) created this folder and the empty placeholders below. **Lane DS fills them** (wave 1, together with S0s at F0s); nobody creates the sets before DS, and nothing here is trained on.

| File | Holds | Filled by |
| --- | --- | --- |
| `synth.toml` | the held-out synth targets for clause 1: generator-v1 targets from a seed range training never draws (60 at 12 per level, more to keep about 120 targets), the hand-made renders (the test half of the preset candidates, the owner's four G1 sounds once extracted) | DS |
| `real.toml` | the 60 references for clause 2: 40 clean in four strata (other subtractive synths, FM or wavetable or digital, acoustic and percussive, the owner's picks) and the 20 degraded copies' sources, with a licence read from the source for every clip | DS |
| `degraded.toml` | the held-out degradation chain (seeded, parameters in the manifest; MP3 32 to 96 kbps, noise at 5 to 20 dB SNR plus hum, reverb RT60 0.4 to 2.5 s, clipping and limiting, phone-speaker EQ, stereo to mono) and the clean targets it is applied to; **degraded sounds are judged against the clean original** | DS (with DEG's chain) |
| `hashes.txt` | the SHA-256 of every set file and every audio file named in the manifests, so a change after the freeze is caught | DS |
| `exclude-synth.txt`, `exclude-real.txt`, `exclude-degraded.txt` | what training must never contain: seed ranges and preset names, Freesound ids and file ids, the impulse-response, noise and codec pools. `enth-datagen exclude` (lane DG) asserts that no training item is on them. DS may rename these files; the pattern is `exclude-*.txt` | DS |
| `runs.jsonl` | **not created yet.** Every test run, appended (build, date, load), never dropped or rewritten; created by lane EVK on the first run, then appended by EVK and EVb | EVK, EVb |

The three TOML placeholders hold one comment line naming their owner and the `.txt` ones are empty; lane DS replaces them with the content and its own header.

## Rules

- **Frozen before training.** The sets, their hashes and the exclusion lists freeze in wave 1. After that a set changes only by a new, named version; the old one stays so earlier results stay checkable.
- **Tuning happens on dev only.** Dev is 30 targets from disjoint streams and files; no search setting, threshold or prior is tuned on a test item, and no prior is fitted on a test preset (the preset candidates are split into two disjoint halves, anchors for training and tests for GR3).
- **Every test run is appended** to `runs.jsonl`, whatever it showed.
- **The listening key stays closed** until every listener is done, and whoever prepares the stimuli does not audition them.
- **Licences are read from the source**, per clip, and recorded in the manifest. Audio is committed only when it is CC0 or the owner's; the manifest of everything else lists where to get it (a dataset that needs an account, such as Freesound's originals, is fetched by the owner, not by an agent). The root `.gitignore` excludes `*.wav`, so a committed clip needs `git add -f` or a negation line DS adds.
- **Short sounds are valid.** The set includes sounds under 200 ms; nothing is ever filtered by duration.
- **Privacy.** No personal detail in any manifest, request, header, URL or payload; where a service asks for contact information use `enth-research/0.1`.

Files here are compared and hashed byte for byte, so `.gitattributes` keeps them LF on every platform (`content/gr3/** text=auto eol=lf`, audio and archives binary).
