# Duels with friends (R5): codes, verification and later live play

R5 lets two friends play the same challenge and compare, with no server, no account and no networking code. Determinism gives the two things serverless contests lacked: one identical puzzle (a challenge code rebuilds the same target bit for bit on every machine) and a fair judge (each side re-scores both answers on its own build). This page is the design of record; the lane plan is [phase-4.md](phase-4.md), the code layer is [challenge.md](challenge.md#version-15-the-kinds-behind-the-escape-r4-r5-planned), and the evidence is the [R4/R5 research session](../_local/sessions/2026-10-22-r4-r5-ear-games-and-duels/summary.md) (report 03 on duels, sharing and live play; report 02 §5 on result codes; the summary wins where it corrects a report). Ear games are in [ear.md](ear.md).

**Status (2026-10-22): planned; nothing is built.** Every decision here is provisional until you steer; work proceeds on the defaults meanwhile. **R5a (duels on Recreate) ships first, before R4's games.** RC, SE, SKD and DUI add their sections as they land, and INT5 keeps this status line ([phase-4.md](phase-4.md#rules-for-every-lane)).

| Piece | Home | Lane | State |
| --- | --- | --- | --- |
| The scorecard, the answer code, the ear result, `result-params-v1`, paste-anything, the daily formula | `enth-challenge/src/{result,answer,paste,daily}.rs` (L4) | RC | Planned (F1e) |
| `verify` | `enth-challenge/src/verify.rs` (L4) | RC | Planned (F1e) |
| Duel state; the patch of the first passing submit | `enth-session/src/duel.rs`, additive in `game.rs` (L6) | SE | Planned (F1e) |
| Duel jobs, the clipboard; `enth duel result\|verify`; the REAPER `duel` scenario | `enth-plugin/src/duel.rs`, `enth-cli/src/cmd/duel.rs` (L8) | SKD | Planned (F2d) |
| The duel card: share, paste, verify, side by side, the sealed answer, the history | `enth-ui/src/editor/duel/` (L7, feature `duel`) | DUI | Planned |
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

Both take code v1's reserved version-15 escape; their layouts, sizes, checks and what older builds print are in [challenge.md](challenge.md#version-15-the-kinds-behind-the-escape-r4-r5-planned). The F1e judge fixes the exact layout.

**Why two codes, not one** (the summary's correction of report 02): the scorecard spoils nothing, so you send it the moment you finish, and its salted commitment shows your answer was fixed before you saw your friend's. The answer code follows once both have played.

| | Scorecard | Answer code |
| --- | --- | --- |
| Carries | The challenge, the engine version, the scorer set, sound % and combined %, passed, submits, the time to pass, the assists, a salted commitment to the answer | The salt, the challenge, the patch as a diff from the round's start |
| Sent | On finishing | After both have played |
| Spoils | Nothing | The answer (so it stays sealed until you've played) |
| Size | About 51 symbols, fixed | About 50, 135 and 255 symbols at levels 1, 3 and 5 |
| Checks | About 30 check bits, then code v1's GF(32) symbol | A 40-bit hash tail, then code v1's GF(32) symbol |

- **Which submit a duel compares** (question 4): the **first passing submit** (its patch, the time to pass and the submit count); otherwise the best submit, marked "not passed". The session keeps only the latest submitted patch today, so lane SE adds the patch at the first pass (additive, within the round's saved budget: a named F1e check).
- **The answer** is `ops_between(start, answer)`, checked, in binary: each op a kind and a module reference; a `SetParam` adds a parameter index and a value (enums by item count, toggles one bit, floats as exact `f32` bits). Indices go through **`result-params-v1`**, a frozen, append-only table, as generator v1 freezes its catalogue snapshot. A whole deflated patch (about 920–1,120 symbols) is the fallback only when it is shorter (added modules).
- **Stripped before encoding:** `SetMeta` (`PatchMeta` has `author`, which can hold a real name), labels, macro names and slots.
- **The commitment** is a salted hash of the canonical answer, truncated to 60 bits. It is salted because a level-1 answer space (waveforms × semitones) is only hundreds of patches, so a bare hash could be brute-forced.
- **Sizes** were measured independently by reports 02 and 03 on targets standing in for passing answers; real answers touch more knobs and may be 1.2–2× longer (lane SKD measures real ones at F2d). Each extra parameter touched adds about 8.4 symbols.

| Level | Report 02 (median, max) | Report 03 | Deflated whole patch |
| --- | --- | --- | --- |
| 1 | 51, 62 | 49 | 924–946 |
| 3 | 141, 154 | 133 | 999–1,028 |
| 5 | 254, 309 | 259 | 1,055–1,117 |

## Verification

`verify(result, evaluator)` in `enth-challenge` (lane RC), run as a job on the plugin's game worker or by `enth duel verify`:

1. decode;
2. regenerate the challenge;
3. apply the answer's ops to the start and check the hash (and the scorecard's commitment, when both are at hand);
4. score with the named scorer set;
5. compare with the claim.

One prepare plus one scored render takes under about 1 s. **The receiver's own re-score of both answers decides;** a claimed number never does. An ear result needs no render: the answers are replayed against the key ([below](#ear-duels)).

### Versions and refusals

| Case | What happens |
| --- | --- |
| Another engine version | Re-score and **show both numbers** ("claimed 93.4 on 0.3.0, 92.9 here"), flagged |
| An unknown kind or layout, or an unknown generator | `code_too_new` |
| An unknown scorer set | `scorer_too_new` (GR1's `sound/2` will be a new set; old ids are kept) |
| An answer op on a module kind this build lacks | `answer_too_new` |
| A generator or hash mismatch | **Refused**: the round must never be judged against a different target or a damaged answer |
| An R1–R3 build | It cannot read either code: "code too new" for a short code, "bad length" for one over 64 symbols ([challenge.md](challenge.md#what-older-builds-print)); the friend sheet says "update enth" |

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
- **Paste-anything** (RC's `paste.rs`; the plugin and the CLI share it): the paste box takes a whole message and pulls out every token that decodes (challenge, scorecard, answer), several per paste. It maps every dash-like character to a hyphen (phones' smart punctuation turns `--` into a dash, so codes never contain `--`) and drops zero-width characters and non-breaking spaces.
- **The clipboard** is written on a click and read only on an explicit Paste, never on its own.
- **Untested:** phone data detectors underlining a code as a phone number (INT5 and your duel check).

## Spoilers, and the duel as a lesson

A friend's answer stays sealed until you've submitted or given up (question 6). It is a UI guard, not cryptography: the source is open and the code decodes. Afterwards you can audition their patch or load it as one undo step, so a duel becomes a lesson.

## Privacy

- **A code carries no name, machine id or absolute time** (durations only), and no patch `meta` or file path. RC's test asserts it on every kind.
- **Names are optional and local:** the receiver may label a pasted result ("from: Sam") in the local duel history. The chat already shows the sender.
- **Seeds keep today's 40-bit mask.** `fresh_seed` (`crates/enth-plugin/src/game.rs`) mixes wall-clock nanoseconds through a bijective finalizer; an unmasked seed would reveal its creation time to the nanosecond, while 40 bits leave about 80 candidate instants a day.
- **No telemetry and no network** in async duels.

## The daily challenge

One challenge per level per day, from a frozen formula of the UTC day (question 6), so everyone who opens "today, level 3" plays the same target with no server and no organiser. The formula is in [challenge.md](challenge.md#the-daily-challenge-formula-r5-planned): integer arithmetic on the day number and the level, generator 1, the top 40 bits, so it is a short code. The day turns at 00:00 UTC; the share line carries the code itself, so a friend in another time zone pastes it.

## Ear duels

After R4's games exist. An adaptive session would give two players different items, so **ear duels use a fixed ladder** (item *i*'s level depends only on *i*). An ear-challenge code (kind 3) carries the `EarSpec`; an ear-result code (kind 4, about 40–60 symbols) carries the answers, verified by replay against the key with no render. **Anyone holding the code can compute the answer keys,** and the UI says so plainly.

## The duel history

Lane DUI: a local list of duels (the codes, both re-scored results, the date you played), with optional names the receiver types. It lives in a per-user file the shell owns, like the progress file, and leaves the machine only if you send it.

## GR5

**Gate GR5** (proposed; the owner sets the criteria). The table of record will be `docs/r5-gates.md` (INT5, planned).

| Check | How | Who |
| --- | --- | --- |
| Three async duels finish with both codes verified both ways; any score difference is explained by a flagged engine difference | Real duels through your own chat; the default stand-in is a two-instance REAPER test | You and a friend; INT5 |
| Codes survive a paste through Discord, Telegram, WhatsApp, Signal, SMS and iMessage, smart punctuation included | A corpus test of mangled pastes (dashes, zero-width characters, quotes, line breaks) in CI, plus your duel check on real apps | RC, INT5; you |
| No name, `meta` or absolute time in any code | A test on every kind, with a patch whose `meta.author` and labels are set | RC |
| The receiver's score decides, on the 25 golden challenges | Target → result → verify gives the same score bits on Linux, Windows and wasm | RC, INT5 |

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
| **Real answers longer than measured** | SKD measures passing answers at F2d; the fallback form |
| **Paste mangling** in chat apps and phones | Paste-anything, a corpus test, your duel check |

## Decisions (provisional)

| Area | Decision (the default unless you say otherwise) | Evidence | Status |
| --- | --- | --- | --- |
| Result codes | A scorecard (about 51 symbols) and an answer code (the patch as a diff) on the version-15 escape | Reports 03 §2.4, 02 §5 | Needs you (Q3) |
| Answer form | `ops_between` with exact `f32` bits through the frozen `result-params-v1` table; `meta`, labels and names stripped | Reports 02 §5, 03 §2.4 | Proposed |
| Code checks | Scorecard: a fixed length, about 30 check bits; answer: a 40-bit hash tail; every kind ends with code v1's GF(32) symbol | Report 03 §2.3, the summary, [challenge.md](challenge.md#what-older-builds-print) | Proposed (F1e judge) |
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
