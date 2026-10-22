# Duels with friends (R5): codes, verification and later live play

R5 lets two friends play the same challenge and compare, with no server, no account and no networking code. Determinism gives the two things serverless contests lacked: one identical puzzle (a challenge code rebuilds the same target bit for bit on every machine) and a fair judge (each side re-scores both answers on its own build). This page is the design of record; the lane plan is [phase-4.md](phase-4.md), the code layer is [challenge.md](challenge.md#version-15-the-kinds-behind-the-escape-r4-r5-planned), and the evidence is the [R4/R5 research session](../_local/sessions/2026-10-22-r4-r5-ear-games-and-duels/summary.md) (report 03 on duels, sharing and live play; report 02 §5 on result codes; the summary wins where it corrects a report). Ear games are in [ear.md](ear.md).

**Status (2026-10-22): planned; nothing is built.** Every decision here is provisional until you steer; work proceeds on the defaults meanwhile. **R5a (duels on Recreate) ships first, before R4's games.** RC, SE, SKD and DUI add their sections as they land, and INT5 keeps this status line ([phase-4.md](phase-4.md#rules-for-every-lane)).

| Piece | Home | Lane | State |
| --- | --- | --- | --- |
| The scorecard, the answer code, the ear result, `result-params-v1`, paste-anything, the daily formula | `enth-challenge/src/{result,answer,paste,daily}.rs` (L4) | RC | Built, for the F1e judge (the ear result's replay waits for CV3's keys) |
| `verify` | `enth-challenge/src/verify.rs` (L4) | RC | Built, for the F1e judge |
| Duel state; the patch of the first passing submit | `enth-session/src/duel.rs`, additive in `game.rs` (L6) | SE | Built; awaiting the F1e judge |
| Duel jobs, the clipboard; `enth duel result\|verify`; the REAPER `duel` scenario | `enth-plugin/src/duel.rs`, `enth-cli/src/cmd/duel.rs` (L8) | SKD | Built (F2d): [below](#the-shell-lane-skd) |
| The duel card: share, paste, verify, side by side, the sealed answer, the history | `enth-ui/src/editor/duel/` (L7, feature `duel`) | SKD (the minimal card), then DUI | Minimal card built (F2d); DUI's card planned |
| GR5, the friend duel sheet, the friend build | `docs/r5-gates.md`, `docs/r5-owner.md`, `docs/duel-friends.md` (all planned) | INT5 | Planned |

## What makes serverless duels last

Report 03 §1 looked at Wordle, Heardle, seeded roguelikes, ghost races, correspondence chess, speedrun replays, Opus Magnum's records and sound-design contests.

- **What lasts:** one identical puzzle for everyone, a spoiler-free share line, plain text over chats people already use, and low stakes.
- **What killed formats:** a server its owner switched off (Heardle: bought, then shut, stats gone), an organiser who stopped (VCV's monthly patch challenge), and judging nobody could make fair (KVR's preset contest never ran: "no good way to fairly compare presets").
- **Verification by replay proves a result, not a person.** Trackmania's slowed-down runs gave valid replays; TASVideos and Doom's demo archives label tool-assisted runs rather than detect them; correspondence chess allows engines. enth ships `enth match` and `enth challenge show --reveal`, so the UI says so.
- **For enth:** determinism solves comparability, a daily formula needs no organiser, and no global boards or names keep the stakes low.

## The flow

1. A sends a challenge code (or both open the day's challenge), and both play.
2. On finishing, each copies a **share message**: a human line plus the **scorecard**. For example:

   ```text
   enth duel · Level 2 · 1080-1000-0001-AB
   93.4 % in 2:41 · 4 submits · 0 hints
   ```

   followed by the scorecard. Emoji, when added, always come with words (screen readers, and SMS, where one emoji cuts 160 characters to 70).
3. Pasting a friend's scorecard shows their result **as reported by their app**: "Swap answer codes to check it."
4. When both have played, they swap **answer codes**. A received answer stays sealed until you've submitted or given up.
5. Your build re-scores **both** answers: "Checked on your computer: their patch scores 93.4 %, as reported", or "… 92.9 % here (they reported 93.4 % on engine 0.3.0; you run 0.4.0)."
6. **Your machine's scores decide.** The time reads "reported by their app; enth can't check times".

## The two codes

Both take code v1's reserved version-15 escape. **Lane RC built them (2026-10-22); the F1e judge reviewed them and lane F1E closed the findings,** so from freeze F1e on they change only additively (a new layout, never an edit); the bit layouts, the op table, `result-params-v1`, the checks and what older builds print are in [challenge.md](challenge.md#version-15-the-kinds-behind-the-escape-r4-r5).

**Why two codes, not one** (the summary's correction of report 02): the scorecard spoils nothing, so you send it the moment you finish, and its salted commitment shows your answer was fixed before you saw your friend's. The answer code follows once both have played.

| | Scorecard (`result::Scorecard`) | Answer code (`answer::AnswerCode`) |
| --- | --- | --- |
| Carries | The challenge, the engine version, the scorer set, sound % and combined % in tenths, passed, submits, the time (to pass, or the round's active time without a pass), the assists, a 60-bit salted commitment to the answer | A 64-bit salt, the challenge, the patch as a diff from the round's start |
| Sent | On finishing | After both have played |
| Spoils | Nothing | The answer (so it stays sealed until you've played) |
| Size | **51 symbols, always** (72–81 with custom settings) | Targets as answers: medians 46, 92 and 152 symbols at levels 1, 3 and 5; solver answers about 48–176 ([below](#sizes)) |
| Checks | Its fixed length and 30 check bits, then code v1's GF(32) symbol | A 40-bit tail over the rebuilt answer, then code v1's GF(32) symbol |

- **Which submit a duel compares** (question 4): the **first passing submit** (its patch, the time to pass and the submit count); otherwise the best submit, marked "not passed". The session keeps only the latest submitted patch today, so lane SE adds the patch at the first pass (additive, within the round's saved budget: a named F1e check). `Scorecard::new` takes that submit's `ScoreReport`, the round's `ScoringSpec`, the submits, the time in milliseconds, the `AssistsUsed` and the answer code's commitment; a reveal before a pass gives no scorecard.
- **The answer** is `ops_between(start, answer)` in binary, in its order: a run of `SetParam`s per target (5 bits for the target, then 4 bits per parameter and its value), the other ops behind a 5-bit code. Targets are places in the patch, never ids; parameters are indices into **`result-params-v1`**, a frozen, append-only table of keys, value codings and defaults, as generator v1 freezes its catalogue snapshot. A float is 16 bits when it has three significant digits (a typed or quantized value) and its exact 33 bits otherwise. No deflated whole patch is needed: an added module carries only its values off the table's defaults.
- **Stripped before encoding:** `SetMeta` (`PatchMeta` has `author`, which can hold a real name), labels, macro names and slots; an added module keeps an id number the app gave (`osc3`, 6 bits; should-fix S1, so the receiver renders the phases the player heard) and any id the player typed becomes the lowest free `<prefix><n>`, so no typed name survives either. A test gives a patch with all of them set and the same patch without them, and gets the same symbols.
- **The salt** (the F1e review's M6): 64 bits of **OS randomness**, drawn by the shell once per duel and kept with the result; **never derived from a clock or a seed**, because the code carries it in the clear ([Privacy](#privacy)).
- **Decoding rebuilds the answer:** it regenerates the target, applies the ops to the start and checks the 40-bit tail, which hashes the salt, the target's fingerprint and the rebuilt answer. A damaged code or a build that regenerates another start or target is refused there (`answer_mismatch`), before anything is scored.
- **The commitment** is the first 60 bits of SHA-256 over the salt, the target's fingerprint and the rebuilt answer (under its own magic). It is salted because a level-1 answer space (waveforms × semitones) is only hundreds of patches, so a bare hash could be brute-forced. The shell draws the salt (OS randomness, once per duel) and keeps it with the result **before the scorecard reaches the clipboard**; it builds the answer code on finishing (`AnswerCode::new(challenge, salt, &patch)`), puts its commitment in the scorecard (`Scorecard::new(…, code.commitment())`, only when `can_share()`), and sends the answer code later. A salt lost before the answer is sent makes a rebuilt answer miss its commitment, which looks like cheating.

### Sizes

Symbols, code v1's included (40 seeds per level, 8 for the solver; reports 02 and 03 measured targets as stand-ins). "Dragged" moves every changed value off three significant digits, "touched" adds three more knobs, and the solver searched 1,500 renders over the drawn or every observable parameter, like `enth match`. Lane SKD measured real passing answers at F2d ([below the table](#passing-answers-lane-skd-f2d)).

| Level | Target (median, max) | Dragged | Touched | Solver, drawn | Solver, observable | Report 02 | Report 03 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | 46, 50 | 49, 56 | 74, 75 | 48, 56 | 107, 107 | 51, 62 | 49 |
| 3 | 92, 98 | 123, 132 | 144, 147 | 113, 117 | 100, 109 | 141, 154 | 133 |
| 5 | 152, 188 | 212, 263 | 230, 271 | 176, 181 | 110, 125 | 254, 309 | 259 |

#### Passing answers (lane SKD, F2d)

Measured on 2026-10-22 with answers that **pass** `sound/1` (`enth-cli`'s `measure_passing_answer_sizes`, 20 seeds per level, every answer passed): "played" drags every randomized float off the target by up to 2 % of its knob's travel (closer until it passes), as a player who matched it by ear and mouse leaves it; "touched" also leaves three other knobs nudged by up to 0.5 %, kept only while the answer passes. The REAPER `duel` scenario's real round (level 2, seed 77, the target rebuilt by edits, then shared from the plugin) gave an answer of **52 symbols** and a scorecard of 51.

| Level | Target (median, max) | Played, passing | Played and touched, passing |
| --- | --- | --- | --- |
| 1 | 46, 50 | 49, 56 | 68, 85 |
| 2 | 56, 60 | 66, 75 | 86, 99 |
| 3 | 92, 96 | 123, 132 | 139, 151 |
| 4 | 109, 123 | 144, 168 | 157, 183 |
| 5 | 152, 190 | 212, 269 | 228, 291 |

So real passing answers are about 1.1–1.4 times the target's size, at most about 290 symbols at level 5: inside Discord's 2,000 characters, not inside one SMS above level 1. **A level-1 answer with touched knobs is usually above 64 symbols**, so an R1–R3 build reads it "bad length", not "update enth" (the friend sheet should say both). The solver's answers were not measured again.

Each extra knob left off three significant digits costs about 8 symbols. The solver moves values only, so at levels 4 and 5 it rarely passes; over every observable parameter it moves about eleven knobs to exact values, about 105 symbols at any level, the likeliest shape of an agent's answer. Every answer of 64 symbols or fewer reads "update enth" on an R1–R3 build, a longer one "bad length".

## Verification

`verify::verify(answer, scorecard)` in `enth-challenge` (lane RC), run as a job on the plugin's game worker or by `enth duel verify`:

1. decode (`AnswerCode::decode`: the challenge regenerated, the ops applied to the start, the tail checked; `Scorecard::decode`);
2. check the pair: the same challenge (`not_the_same_challenge`), and whether the answer is the one the scorecard's commitment fixed (`ClaimCheck::commitment_matches`: reported, not refused);
3. prepare the round with the scorecard's scorer set when this build has it, otherwise (a newer build's set, or no scorecard) with this build's current set (`verify::scorer_set`; the F1e review's M4);
4. score the rebuilt answer (`score_round`);
5. compare with the claim (`ClaimCheck`: the claimed and the re-scored sound % and combined % in tenths, passed, both engines and both scorer sets; `as_reported()`, false when the sets differ; `engine_differs()`, `scorers_differ()`).

One prepare plus one scored render takes under about 1 s. A duel prepares once (`verify::prepare` with `verify::scorer_set(card)`) and verifies both answers on it (`verify_prepared` on theirs, and a re-score of your own **canonical** answer, `AnswerCode::answer()`), so both numbers come from one machine, one scorer set and the same patches the codes carry. **The receiver's own re-score of both answers decides;** a claimed number never does. An ear result needs no render: `verify::verify_ear` replays the answers against the generator's keys ([below](#ear-duels)).

### Versions and refusals

| Case | What happens |
| --- | --- |
| Another engine version | Re-score and **show both numbers** ("claimed 93.4 on 0.3.0, 92.9 here"): `ClaimCheck::engine_differs` |
| An unknown kind, layout or form, or a generator, preset or template this build lacks | `code_too_new` |
| An unknown scorer set (a newer build's, GR1's `sound/2` say) | **Read and shown as reported** (the F1e review's M4); re-scored with this build's current set and **both numbers shown**: `ClaimCheck::scorers_differ`. Both sides then score on the older set, since every build keeps its old sets, so a duel across the update works both ways |
| An answer op, module kind, parameter or source this build lacks | `answer_too_new` |
| A generator or hash mismatch | **Refused** at decode (`target_mismatch` for a long challenge's prefix, `answer_mismatch` for the tail): the round is never judged against a different target or a damaged answer |
| An answer that is not the committed one | Scored, and flagged: it is not the answer the scorecard fixed |
| An R1–R3 build | It cannot read either code: "code too new" for a code of 64 symbols or fewer (every scorecard with a short challenge, most level-1 answers), "bad length" above ([challenge.md](challenge.md#what-older-builds-print)); the friend sheet says "update enth" |

## What a code proves

| Claim | Proved by re-scoring? |
| --- | --- |
| "My patch scores 93.4 %" | **Yes,** on the same engine: scores are bit-exact on Linux, Windows, macOS and wasm32, and generator v1 is frozen |
| "My score beats yours" | **Yes,** when one machine scores both answers |
| "My answer was fixed before I saw yours" | **Yes,** through the salted commitment in a scorecard sent first; the chat shows the order |
| "I made it myself" | **No.** `enth match` passes 11–13 of 20 golden and GR1 codes in 6–12 s, and `enth challenge show --reveal` writes the target |
| "In 2:41", "first try", "no hints" | **No.** They come from the shell's clock and local counters: editable, practisable, pausable |

- **No cryptography bounds a time** without trusted clocks at both ends; timestamping services need a network. The chat is the referee: the gap between sending the challenge and receiving the result is an upper bound the friend sees.
- **Label, don't detect.** Assists print on the card, like Wordle's hard-mode asterisk; **a revealed target gives no scorecard**.
- **Copying:** scorecards first and answers later stops honest-but-tempted friends. Decoding a code with the CLI to cheat is out of scope, and the About line says so.

## Wording

- **"Checked on your computer"** for scores; **"reported"** for times and assists.
- **Never** "verified time", "verified player", "proof" or "anti-cheat".
- **The About line, said once:** "Duel codes are for friends. A code proves what a patch scores, not who made it or how long it took. enth's own tools can solve any challenge."

## Sharing and paste

- **Crockford base32 survives chats:** digits and capital letters dodge markdown (`_`, `*`, `~` and the backtick), which breaks base64url and base58.
- **Chat limits:** a 300-symbol code fits Discord (2,000 characters), Telegram (4,096), WhatsApp and Signal, but not one SMS (160 GSM-7 characters, 70 once an emoji appears). A scorecard (about 60 characters with hyphens) fits everywhere, one SMS included.
- **Paste-anything** (RC's `paste.rs`, built; the plugin and the CLI share it): `paste::paste` takes a whole message and returns every token that decodes (challenge, scorecard, answer, ear result), several per paste, each once, in order, and the code-looking tokens that do not decode with their reason ("a typo", "update enth"). It maps every dash-like character to a hyphen (phones' smart punctuation turns `--` into a dash, so codes never contain `--`), drops zero-width characters and soft hyphens, reads non-breaking spaces as spaces and full-width letters as ASCII, and joins capital-and-digit pieces that a line break or spaces split. A corpus test runs every kind through 35 manglings (dashes, zero-width characters, markdown, quotes, links, line wraps, full-width letters and their combinations).
- **The clipboard** is written on a click and read only on an explicit Paste, never on its own.
- **Untested:** phone data detectors underlining a code as a phone number (INT5 and your duel check).

## Spoilers, and the duel as a lesson

A friend's answer stays sealed until you've submitted or given up (question 6). It is a UI guard, not cryptography: the source is open and the code decodes. Afterwards you can audition their patch or load it as one undo step, so a duel becomes a lesson.

## Privacy

- **A code carries no name, machine id or absolute time** (durations only), and no patch `meta` or file path. RC's test asserts it on every kind.
- **Names are optional and local:** the receiver may label a pasted result ("from: Sam") in the local duel history. The chat already shows the sender.
- **Seeds keep today's 40-bit mask.** `fresh_seed` (`crates/enth-plugin/src/game.rs`) mixes wall-clock nanoseconds through a bijective finalizer; an unmasked seed would reveal its creation time to the nanosecond, while 40 bits leave about 80 candidate instants a day.
- **The answer code's salt is OS randomness, never a clock** (the F1e review's M6). The salt is 64 bits in the clear in every answer code, so it is drawn from the operating system (std's `RandomState` keys, or `getrandom`, which is already in `Cargo.lock`; a new direct dependency is the coordinator's call), **once per duel**, and kept with the result in the duel history. It is **never derived from a clock or a seed**: a salt from `fresh_seed` would carry the absolute time, to the nanosecond, in every answer code, which is exactly why seeds are masked. No test in `enth-challenge` can catch it; lane SKD follows the rule and DUI persists the salt.
- **No telemetry and no network** in async duels.

## The daily challenge

One challenge per level per day, from a frozen formula of the UTC day (question 6), so everyone who opens "today, level 3" plays the same target with no server and no organiser. Built by lane RC as `daily::v1(day, level)` with `daily::utc_day(unix_seconds)` ([challenge.md](challenge.md#the-daily-challenge-formula-r5)): integer arithmetic on the day number and the level, generator 1, the top 40 bits, so it is a short code; a golden from an independent implementation pins it (2026-10-22, level 3: `10C0-1FYS-GTCM-85`). The day turns at 00:00 UTC; the share line carries the code itself, so a friend in another time zone pastes it.

## Ear duels

After R4's games exist. An adaptive session would give two players different items, so **ear duels use a fixed ladder** (item *i*'s level depends only on *i*). An ear-challenge code (kind 3, 19 symbols) carries the `EarSpec`; paste-anything reads it (`Found::EarChallenge`, for the Ear view). An ear-result code (kind 4, 50 symbols for 16 choice answers) carries the spec, the **ear rules** it was judged by (should-fix S5), the answers (taps relative to the first tap, on any clock) and the time; `verify::verify_ear` replays it against the generator's keys with no render, with the result's rules when this build has them (else its own, flagged). **Anyone holding the code can compute the answer keys,** and the UI says so plainly.

## The duel history

Lane DUI: a local list of duels (the codes, both re-scored results, the date you played), with optional names the receiver types. It lives in a per-user file the shell owns, like the progress file, and leaves the machine only if you send it.

## The session (lane SE)

The pure duel state is built ([session.md](session.md#duel-lane-se-r5a) has the API, the lifecycle and the tests). What it settles, for the other lanes:

- **The player's result** is the first passing submit's scores, time to pass, submit count and assists, with its patch; otherwise the best submit, marked "not passed". It is kept when the round ends, so the answer can still be shared after Restore or Keep. The first played attempt at a duel's code stands.
- **The round keeps a pass as a diff from its start** (`RoundRecord::compared`, 64 KiB at most) inside the unchanged `ROUND_BYTES`; the best of a round that has not passed is held in memory only. No existing round fixture changed.
- **Their scorecard is shown at once, as reported. Their answer is sealed** until you have a scored submit or have revealed; ending a round with nothing played does not open it. It is a session guard, not cryptography.
- **The receiver's re-score decides.** The session posts a `Verify` job to the shell and stores what the worker measured: both re-scores, the commitment check, the claim check and their patch. A mismatch on the same engine is a flag, not a verdict.
- **Their patch loads as one undo step** with no host writes, never while a round is live; it can be auditioned once, rendered by the shell with the challenge's phrase.
- **The salt is the shell's:** the session has no randomness and builds no code; the shell draws the salt from OS randomness once per duel (never from a clock or a seed, [Privacy](#privacy)) and keeps it, with the result, in the duel history.
- **Not saved in the project.** The duel history is the shell's file (DUI).

## The shell (lane SKD)

Built at F2d ([plugin.md](plugin.md#duels-lane-skd-r5a), [agent-cli.md](agent-cli.md#duels-on-the-command-line-enth-duel)); the plugin's `duel` feature is **off by default** until DUI's card and INT5's friend build:

- **Share** (the card's click): the player's result is cloned on the click, a duel opens on its code, the duel's salt is drawn, and the game worker makes the codes: `AnswerCode::new(challenge, salt, &patch)`, the round prepared once with the round's own scorer set, the **canonical answer** scored, `Scorecard::new(…, code.commitment())`. The scorecard claims the canonical answer's score, which is what every receiver re-scores, so the same engine reads "as reported".
- **The salt:** `getrandom::u64` in the plugin, std's `RandomState` keys in the CLI; drawn once per duel at its first Share and kept with its codes (a second Share gives the same codes); never a clock or a seed. **DUI writes it to the history** with the codes before Copy is offered.
- **Verify** on the game worker in the F1e order: decode, prepare once with `verify::scorer_set(card)`, `verify_prepared` on theirs, your canonical answer re-scored on the same round, `DuelVerified…with_claim(v.claim)`; an answer for another challenge fails as `not_the_same_challenge`. **Audition** through the Learn examples slot (no new audio-thread code), never while a round's target holds it.
- **The clipboard** is written with egui's `copy_text` inside a click on Copy and **never read**: a paste is the platform's Paste into the card's paste box, and Read hands the text to `paste::paste`. `Found::EarChallenge` is named on the card for the Ear view (SKE routes it).
- **The minimal card** (`enth-ui`'s `editor/duel/`): a column at the window's right edge, opened from **Duel** in the overview's corner; its controls in fixed rows (Share result, Copy message, Copy answer; the paste box; Read, Verify, Use theirs, ▶ Theirs; Play, Close duel), then the text, worded as [Wording](#wording) says. DUI replaces it.
- **Checked:** the REAPER `duel` scenario (two instances, the X clipboard, a saved round passed and shared, the answer sealed, then checked as reported with the commitment matched: 15 of 15 checks); `enth duel result|verify` with insta snapshots; two real instances through the worker in `enth-plugin`'s `tests_duel.rs`.

## GR5

**Gate GR5** (proposed; the owner sets the criteria). The table of record will be `docs/r5-gates.md` (INT5, planned).

| Check | How | Who |
| --- | --- | --- |
| Three async duels finish with both codes verified both ways; any score difference is explained by a flagged engine difference | Real duels through your own chat; the default stand-in is a two-instance REAPER test | You and a friend; INT5 |
| Codes survive a paste through Discord, Telegram, WhatsApp, Signal, SMS and iMessage, smart punctuation included | A corpus test of mangled pastes (dashes, zero-width characters, quotes, line breaks) in CI, plus your duel check on real apps | RC, INT5; you |
| No name, `meta` or absolute time in any code | A test on every kind, with a patch whose `meta.author` and labels are set | RC |
| The receiver's score decides, on the 25 golden challenges | Target → result → verify gives the same score bits on Linux, Windows and wasm: `enth-golden`'s `gr5_receiver` test (lane F1E) sends every pinned answer through the answer code and `verify` on every `golden.yml` leg | RC, F1E, INT5 |

## Live play later

Not in R5a. **After GR1, in the standalone app first** (question 12), after a research session of its own (report 03 §5 is the survey):

- **Discovery:** mdns-sd plus a typed `IP:port`, which also covers the VPN overlays friends already run (Tailscale, ZeroTier), with no relay of ours.
- **Transport:** plain TCP, or quinn for encryption with a key pinned by a code, carrying the same codes plus start and progress messages. Threads stay off the audio thread and are joined on deactivate and unload.
- **Not libp2p or iroh** unless internet NAT traversal becomes a goal (iroh falls back to its company's relays by default), and then with relays off or self-hosted. Sizes over a 0.38 MB baseline: mdns-sd about +0.8 MB, quinn about +2.8 MB, libp2p about +9 MB, iroh about +15 MB.
- **The OS is the obstacle:** macOS's local-network permission attaches to the DAW, not the plugin (the host's `Info.plist` must carry the usage key, and the alert names REAPER); Windows' firewall rule names the DAW's exe; guest and dorm Wi-Fi isolate clients; hole punching fails about 30 % of the time without a relay. Hence the standalone app first, with its own bundle and firewall entry.
- **Privacy on a LAN:** mDNS instance names reach the whole network, and default hostnames often carry a person's name, so instance names are random.

## Deferred

QR result cards (encoders and the `rqrr` decoder were checked; QR is useless between two desktops), leaderboards, internet NAT traversal, op-log replays (long and forgeable), (time, score) ghosts, and MIDI answers until the owner OKs the audio-thread path.

## Risks

| Risk | Retirement |
| --- | --- |
| **Cheating is easy** | Honest wording, labelled assists, sealed answers, the social model |
| **Old builds say "bad length"** and can't be fixed | Every kind keeps code v1's GF(32) symbol, so short codes read "update enth"; the friend sheet |
| **Scorer drift** after GR1's `sound/2` | The scorer set in every code; old ids kept |
| **Format sprawl** (two result kinds, an ear result, the parameter table, the daily formula) | One judge at F1e; fixtures with pinned SHAs |
| **Real answers longer than measured** | Measured at F1e on dragged, touched and solver answers (up to about 270 symbols at level 5); SKD measures passing answers at F2d; a whole-patch layout if ever needed |
| **Paste mangling** in chat apps and phones | Paste-anything, a corpus test, your duel check |

## Decisions (provisional)

| Area | Decision (the default unless you say otherwise) | Evidence | Status |
| --- | --- | --- | --- |
| Result codes | A scorecard (about 51 symbols) and an answer code (the patch as a diff) on the version-15 escape | Reports 03 §2.4, 02 §5 | Needs you (Q3) |
| Answer form | `ops_between` with exact `f32` bits through the frozen `result-params-v1` table; `meta`, labels, names, slots and typed ids stripped | Reports 02 §5, 03 §2.4 | Built (RC), for the F1e judge |
| Code checks | Scorecard: a fixed length, 30 check bits; answer: a 40-bit tail over the rebuilt answer; every kind ends with code v1's GF(32) symbol | Report 03 §2.3, the summary, [challenge.md](challenge.md#what-older-builds-print) | Built (RC), for the F1e judge |
| Compared submit | The first passing submit; otherwise the best, marked "not passed" | Report 03 §7 | Needs you (Q4) |
| Verification | The receiver re-scores both answers; an engine mismatch shows both numbers; refuse only on a generator or hash mismatch | Reports 02 §5, 03 §3 | Proposed |
| Wording | "Checked on your computer" (scores), "reported" (time, assists); never "verified player", "proof", "anti-cheat" | Report 03 §3 | Proposed |
| Spoilers | A friend's answer sealed until you've played or given up | Reports 02 §5, 03 §2.4 | Needs you (Q6) |
| Paste and privacy | Paste-anything; the clipboard read only on Paste; no names, ids, absolute times or `meta` in codes; seeds keep the 40-bit mask | Report 03 §2.2, §4 | Proposed |
| Daily challenge | One per level, from a frozen formula of the UTC day | Reports 02 §5, 03 §1 | Needs you (Q6) |
| Ear duels | A fixed ladder, after R4's games | Report 01 §4 | Proposed |
| Order | R5a before R4 | Report 02 §8 | Needs you (Q5) |
| GR5 | Three duels verified both ways; chat-paste survival; no personal data in codes | Reports 02 §8, 03 §2 | Needs you (Q11) |
| Live duels | After GR1; the standalone app first; LAN with mdns-sd and TCP or quinn; no libp2p, iroh or relay | Report 03 §5 | Needs you (Q12) |
