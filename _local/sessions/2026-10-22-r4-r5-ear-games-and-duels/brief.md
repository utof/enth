# Research brief: R4 Ear-training games and R5 Duels with friends

- **Date:** 2026-10-22
- **Milestones:** R4 Ear-training games and R5 Duels with friends ([docs/roadmap.md](../../../docs/roadmap.md#r4--ear-training-games-later)).
  - **R4:** SoundGym-style games built on the same Challenge object: intervals, perfect-pitch drills, chord progressions, rhythm and syncopation, EQ A/B.
  - **R5:** no server-side compute. A challenge is a seed plus settings, so both players build the identical target and score locally. First async duels over any chat: share a challenge code, send back a result code carrying the patch and the time, which the friend's copy re-renders to verify; no networking code. Live duels (peer to peer, LAN first) only later.
- **Asked by:** the owner's standing request to keep advancing milestones in parallel. Docs are suggestions; only Rust and a native, non-WebView UI are firm. The owner reads the summary at their own pace and steers.

## Where things stand (2026-10-22)

On main:
- **P1, R1 and S1:**
  - **The engine:** osc, noise, svf with drive, adsr, lfo, reverb, comp, voices, modulation, macros; deterministic on Linux, Windows, macOS arm64 and wasm32, with goldens on six targets.
  - **The session** (`enth-session`): the patch, undo, the host bridge, the Recreate round, presets.
  - **The plugin and editor:** CLAP and VST3 on a nice-plug fork; `enth-ui`, egui, layouts as data.
- **Recreate:**
  - Challenge codes (`enth-challenge`, with Challenge v2: inline patches, draws, selectors and locks, lesson phrases) and the frozen generator v1.
  - The scorers `sound/1` and `structure/1`; the coach; match by CMA-ES.
  - The CLI (`enth suggest | match | explain | challenge | compare | lesson | create`).
- **R2 Learn, built:**
  - `enth-learn` (packs, lessons, progress, player, predicates, sound and choice checks).
  - Lesson rounds and a Review queue (Leitner boxes per skill, fed by Recreate rings).
  - The Learn view, five content packs, `lesson-check`, and GR2's test forms and `gr2-report`.
- **R3a Sound → patch, nearly done:** intake, create by search, the Create view. R3b (a small proposal model on our own kernels) has its contract written.
- **Known limits that matter here:**
  - `sound/1` has deaf spots: slow beating, sustain level, attacks under about 60 ms, pluck resonance, and movement inside a note.
  - Create and the generator play one note, and two-note phrases fail.
  - Lewton (Ogg) and nanomp3 cross-platform bit-exactness is unproven.

## Constraints

- Rust and a native non-WebView UI.
- Permissive licences: MIT, Apache, BSD, ISC, Zlib, BSL-1.0; MPL-2.0 needs the owner's OK.
- Real-time safety: nothing touches the audio thread except through the existing audition and voice paths.
- Determinism: a game round is a code, and every machine builds the same target bit for bit.
- Design tokens only in UI code; shells translate, never decide; layers enforced (`cargo xtask check-layers`); wasm32 for core crates.
- Linux and REAPER first; light builds without a feature are a cargo flag.
- **No telemetry, no accounts and no server:** R5 async duels have no networking code at all, and a download happens only on the user's click.
- Codes carry no personal data.

## Sub-researchers (run in parallel)

| # | Axis | Report |
| --- | --- | --- |
| 1 | **Ear-training pedagogy and game design.** Prior art and evidence:<br>• Products: SoundGym, Quiztones, TrainYourEars, EarMaster, Teoria, musictheory.net, Functional Ear Trainer, Tonedear, Melodics, Complete Ear Trainer, Syntorial's ear parts.<br>• Research: interval and chord recognition (relative pitch), what adult absolute-pitch training can and cannot achieve (be honest; cite studies), rhythm and syncopation training, and EQ and timbre training (frequency-band identification, "golden ears" studies).<br>• Design: adaptive difficulty and staircases (e.g. 2-down/1-up, QUEST), spaced repetition across games, and session length and streaks without dark patterns.<br>• Which games teach sound design (the synth's own parameters: cutoff, envelope times, detune, reverb size) versus music theory. What people say (forums, reviews), and which games people abandon and why.<br>• The output: a ranked list of games for R4 with each game's round shape, difficulty ladder, scoring, how many rounds make a session, and how each reuses Challenge v2, the generator, the scorers, Learn's skills and the Review queue. | `01-ear-games-pedagogy.md` |
| 2 | **Architecture, code structure, risks and the lane plan.**<br>• Where R4's games live in the layer stack (`enth-challenge` game kinds vs a new crate, `enth-learn` skills, the session's round, the editor's game view). Round types (multiple choice, slider estimate, A/B, tap rhythm) and their deterministic targets; how intervals, chords and rhythms are rendered by our engine (polyphony, phrases: today the generator plays one note).<br>• EQ A/B (an EQ module does not exist yet: S2 lists one); timing input for rhythm games in a plugin (host transport vs the editor clock, latency).<br>• R5 result codes: format, size, the existing challenge-code encoding, what they carry (patch, time, score, versions), verification by re-render, and what cheating determinism can and cannot stop (honest limits: a time is self-reported; a patch can be found by `enth match`). Cross-version rules (`code_too_new`).<br>• Tests (kittest, scripted players, agent evals), risks and unknowns, and a lane plan in waves (at most two heavy plugin builds at once) with freezes and acceptance per lane, plus the owner's sittings. | `02-architecture-plan.md` |
| 3 | **Duels and sharing: prior art, trust and later live play.**<br>• Async competitive formats with no server: Wordle-style shared results, daily seeded challenges, ghost races, correspondence chess, speedrun verification by replay, "patch battle" and sound-design challenge communities (Reddit, KVR, Discord, YouTube "sound design battles"): what made them work and what killed them.<br>• Result-code UX: copy-paste in chat apps, length limits (Discord, Telegram, WhatsApp, iMessage, SMS), QR codes, URL-safe alphabets, emoji summaries.<br>• Fairness and cheating without a server: what verification by re-render really proves; time claims; honest UI wording.<br>• Privacy: nothing personal in a code; names optional and local.<br>• For later live duels only, a short survey of LAN-first peer-to-peer in Rust (mDNS discovery, QUIC via quinn, libp2p, iroh): versions, licences and repo health checked today, binary size in a plugin, firewall and NAT realities. | `03-duels-sharing.md` |

## Rules

[docs/research-process.md](../../../docs/research-process.md) applies:
- Date every claim and link it; verify licences from the actual file.
- Mark claims *measured*, *documented* or *anecdotal*; say plainly when evidence is missing.
- Reports are at most about 25 KB.

AGENTS.md also applies:
- Xvfb only; never play audio.
- Persistent scratch with `notes.md` (`~/code/enth-scratch/2026-10-22-r4r5/NN/`); `flock -s /tmp/enth-quiet.lock` for builds.
- **No personal details in any web or API request** (User-Agent `enth-research/0.1`).
- **Web etiquette:** read a site's `robots.txt` and terms before any scripted fetch, never fetch disallowed paths, no scraping; ordinary web search and reading individual pages is fine.

Reports are saved verbatim and never edited.
