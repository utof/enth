# 03: Duels and sharing: prior art, trust and later live play

- **Sub-researcher:** 03, 2026-10-22.
- **Scratch:** `~/code/enth-scratch/2026-10-22-r4r5/03/` (`notes.md`, `size_probe.py`, `qr_probe.py`, `probe/`).
- **Tags:** **[M]** measured here today, **[D]** documented by the source, **[A]** anecdotal. Pages were read on 2026-10-22 unless dated.

## TL;DR

1. **What makes serverless async competition last:** one identical puzzle for everyone, a spoiler-free share artefact, plain text over existing chats, and low stakes.
   - **What killed such formats:** a server its owner switched off (Heardle), an organiser who stopped (VCV's patch challenge), and judging nobody could make fair (KVR's preset contest, which never ran).
   - Determinism gives enth the identical target and the objective score for free.
2. **Re-rendering proves a patch's score, nothing more.** Trackmania's slowed-down runs gave valid replays. TASVideos and DSDA label tool-assisted runs rather than detect them, and ICCF allowed engines. enth ships `enth match` and `enth challenge show --reveal`, so say so in the UI. **The time is always self-reported.**
3. **Two codes on code v1's version-15 escape:**
   - **Scorecard:** about 50 symbols, spoiler-free. It holds the challenge, engine, scores, time, assists and a salted commitment to the answer.
   - **Answer code:** the patch as a diff from the start, plus the salt. Measured: **49 / 133 / 259 symbols** at levels 1 / 3 / 5.
   - **Flow:** send the scorecard on finishing, and swap answers after both have played. The receiver re-scores *both* answers on its own build, so claimed numbers never decide.
   - **Doc conflict this settles:** roadmap R5 says the result code "contains their patch", but challenge.md says result codes carry "the answer's fingerprint and the scores … never the patch itself". The pair satisfies both.
4. **Chat UX:**
   - Crockford base32 already dodges markdown (`_ * ~` and the backtick). Base64url's `_` gets eaten.
   - A 300-symbol code fits Discord (2,000), Telegram (4,096) and SMS (Twilio caps at 1,600).
   - **Paste-anything parsing** matters most: find codes inside a pasted message, tolerate dashes, zero-width characters and emoji, and never auto-read the clipboard.
   - Pair emoji with words, for screen readers and for SMS, where one emoji turns 160 characters into 70.
5. **Live duels later:**
   - Stripped size over a 0.38 MB std baseline: mdns-sd adds about 0.8 MB, quinn about 2.8 MB, libp2p about 9 MB and iroh 1.4.1 about 15 MB [M].
   - The real obstacle is the OS. On macOS 15+, the local-network permission attaches to the DAW, not to our plugin (TN3179), and Windows' firewall rule names the host exe.
   - Hole punching fails about 30 % of the time without a relay.
   - **Advice:** LAN with mdns-sd plus std TCP or quinn, in the standalone app first, plus typed IPs for VPN overlays. No relay of ours.

## 1. Prior art

| Case | Mechanism | What worked | What hurt or killed it |
| --- | --- | --- | --- |
| **Wordle (2021–22)** | Answer chosen from the date in the client JS ([Deno solver notes](https://deno.land/x/wordle_solver)) [D]; later moved server-side ([sbplat/wordle](https://github.com/sbplat/wordle), archived 2024-02-19) [D] | The emoji grid, added in Dec 2021 after New Zealand players made their own. Wardle: it shares how you did "without ruining the game for others" ([The Spinoff, 2022-01-13](https://thespinoff.co.nz/media/13-01-2022/those-coloured-boxes-on-twitter-are-new-zealands-fault)) [D]. Hard mode adds a self-declared asterisk ([NPR, 2022-01-24](https://www.npr.org/2022/01/24/1074532327/wordle-strategy)) [D] | Cheating was trivial (read the JS) and pointless. Grids read badly in VoiceOver ([MacStories](https://www.macstories.net/ios/wordlebot-1-1-now-fully-accessible-with-native-emoji-to-image-conversion/); [opensource.com, 2022-01](https://opensource.com/article/22/1/open-source-accessibility-wordle)) [D] |
| **Heardle** | Server-hosted daily song clip | Wordle's formula, for music | Spotify bought it (2022-07) and shut it on 2023-05-05, stats gone ([TechCrunch, 2023-04-14](https://techcrunch.com/2023/04/14/spotify-is-shutting-down-heardle-the-wordle-like-music-guessing-game-it-bought-last-year)) [D]. A server game dies with its owner's priorities; local codes don't |
| **Seeded roguelikes (Balatro)** | A short seed offered for copying after a loss | One code rebuilds the run | Seeded runs grant no unlocks, so stakes stay low ([Steam forum, 2024](https://steamcommunity.com/app/2379780/discussions/0/4338734550484225211)) [A] |
| **Ghost races** | An input log replayed on deterministic physics. Mario Kart Wii's `.rkg` holds the inputs plus lap metadata ([MKW TAS wiki](https://wiki.mkwtas.com/wiki/TT_Rec)) [D] | You race a real run, not a number | **Trackmania, 2021-05:** runs were driven at 40–80 % game speed and the replays were valid. Caught by input-rate forensics, not by validation ([Kotaku](https://kotaku.com/trackmania-cheating-scandal-is-utterly-fascinating-1846960428); [Liquipedia](https://liquipedia.net/trackmania/Riolu)) [D] |
| **Correspondence chess** | Post, then email, then a server; language-neutral ICCF numeric notation ([Wikipedia](https://en.wikipedia.org/wiki/Correspondence_chess)) [D] | Compact text that any channel carries | ICCF allows engines ("hybrid … guided by the accuracy of machines"). Draws are "very high" [D]; forum claims of 99–100 % at the top ([talkchess](https://talkchess.com/viewtopic.php?p=938058)) [A] |
| **Speedrun replays** | **Compet-n/DSDA:** one exact Doom exe (checked by file size), which "makes it possible to … detect cheating". Slow motion is banned and built demos are labelled TAS ([Doom Wiki](https://www.doomwiki.org/wiki/Compet-n)) [D]. **TASVideos:** judges re-run the input file until it is "sync-verified" ([TASVideos](https://tasvideos.org/SubmissionInstructions)) [D] | A fixed build plus deterministic replay | Assistance is labelled, never detected. With no replay, statistics stand in: Dream's 2020 luck was put at 1 in 7.5 trillion ([Defector](https://defector.com/lets-dive-into-this-minecraft-youtuber-speedrunning-drama)) [D/A] |
| **Opus Magnum records** | A community bot re-simulates uploaded solutions (omsim) and renders GIFs ([biggieblog, 2023-01-06](https://biggieblog.com/tracking-the-global-opus-magnum-records/)) [D] | The closest analogue to re-render verification. Records carry **no names**. The game shows the 1st-percentile score to dodge cheated tops [D] | Glitches had to be patched or checked for |
| **Sound-design contests** | **KVR One Synth Challenge:** a tune from one free synth a month, then a vote; 207th edition ([VCV forum, 2026-06-08](https://community.vcvrack.com/t/patch-challenges/25893)) [A]. Volunteer-run, with rule confusion over effects ([KVR, 2025-07](https://www.kvraudio.com/forum/viewtopic.php?p=9102460)) [A] | One instrument, a cadence, votes | **KVR "Sound Designer Challenge"** (2018): never ran, because there is "no good way to fairly compare presets between different synths" ([KVR](https://www.kvraudio.com/forum/viewtopic.php?p=7182643)) [A]. **VCV's monthly patch challenge** died over the organiser's holidays (VCV thread) [A]. "Recreate this sound" lives as help threads ([Image-Line](https://forum.image-line.com/viewtopic.php?p=846313)). No current YouTube "sound design battle" series found (evidence missing). **SoundGym** runs weekly 3-minute EQ contests on its servers ([Sonicstate, 2020-08-07](https://sonicstate.com/news/2020/08/07/online-ear-training-for-musicians/)) [D] |

**For enth:**

- **Comparability:** solved by determinism. The seed gives one target and the scorer one number, which is what KVR's proposal lacked.
- **Cadence without an organiser:** a daily seed by formula needs no exchange at all. challenge.md already reserves "a seed made by a formula … (a daily challenge's)", and Wordle got away with a client-side answer.
- **Low stakes:** no global boards and no names. Mark assisted runs the way Wordle marks hard mode.
- **Cheap ghosts:** a (time, score) pair per submit costs about 26 bits, so 10 submits are about 52 symbols. Enough to draw a friend's progress curve (unverified, like the time).

## 2. Result-code UX

### 2.1 Chat limits per message

| Channel | Limit | Status |
| --- | --- | --- |
| Discord | 2,000 characters (4,000 with Nitro) | [D] [support "Account Caps"](https://support.discord.com/hc/en-us/articles/33694251638295) (via search snippet; direct fetch gave 403) |
| Telegram | 1–4,096 characters after entities (Bot API `sendMessage`; Bot API 10.3, 2026-08-24) | [D] via the [aiogram mirror](https://docs.aiogram.dev/en/latest/api/methods/send_message.html) |
| WhatsApp | 65,536 characters | [A] secondary only ([ioBroker forum](https://forum.iobroker.net/topic/60568/whatsapp-l%C3%A4nge/3)) |
| Signal | The app removed its cap (Android 4.37/4.38 "send long messages"); signal-cli splits at 2,000 | [A] |
| SMS | 160 GSM-7 characters (153 per segment). **One emoji switches to UCS-2: 70 (67 per segment).** Twilio caps at 1,600 and advises under 320 | [D] [Twilio](https://www.twilio.com/docs/glossary/what-sms-character-limit) |
| iMessage | none documented | evidence missing |

A scorecard (about 60 characters with hyphens) fits everywhere, including one SMS without emoji. An answer code of 300 or fewer symbols fits every app. A deflated whole patch (952–1,096 base32 symbols [M]) fits everything but SMS comfortably.

### 2.2 What breaks codes

- **Markdown:** WhatsApp's `_italic_ *bold* ~strike~` ([How-To Geek](https://www.howtogeek.com/438504/how-to-format-your-whatsapp-messages/)) [D]. Discord underscores break text and URLs ([community post](https://support.discord.com/hc/nl/community/posts/360043014671)) [A]. Crockford base32 (digits plus uppercase letters) is immune; base64url and base58 are not.
- **Smart punctuation:** iOS turns `--` into an em dash and quotes into curly ones. It is on by default, and pasted text can carry it ([PSPDFKit](https://pspdfkit.com/blog/2018/ios-11-smart-punctuation)) [D]. So the decoder maps every dash-like character (U+2010–2015, U+2212) to a hyphen and drops zero-width characters and NBSP, and codes never contain `--`.
- **Selection:** BIP-173 kept its separator alphanumeric so that a double-click selects the whole string [D]; hyphens break that. **Paste-anything** fixes it: the paste box takes a whole message and pulls out every token that decodes (challenge, scorecard, answer). Several codes can arrive in one paste.
- **Clipboard:** iOS 14's banners exposed apps reading the clipboard on every keystroke ([MacTrast, 2020-06](https://www.mactrast.com/2020/06/tiktok-apps-reading-of-ios-clipboard-exposed-by-new-feature-in-ios-14/)) [D]. Read only on an explicit Paste.
- **Untested:** phone data detectors may underline `1080-1000-0001` as a phone number.

### 2.3 Alphabet, checks, QR

- **Keep Crockford base32.** BIP-173 wrote down the same reasons ([BIP-173](https://github.com/bitcoin/bips/blob/master/bip-0173.mediawiki)) [D]:
  - QR alphanumeric mode, "45 % more compact than the normal byte mode";
  - case-insensitivity;
  - a checksum that "guarantees detection of any error affecting at most 4 characters", designed for up to 89 characters ("other designs are preferable" beyond).
- **The single GF(32) check** suits a 14-symbol code. At 50–270 symbols, random damage slips through 1 time in 32. Truncation is the usual copy-paste failure, and a long code's length isn't fixed.
  - **Scorecard:** a 6-symbol BCH or CRC-30 check plus a fixed length.
  - **Answer code:** a 40-bit hash tail. Row 2 picks the exact code.
- **QR [M]** (`qr_probe.py`, alphanumeric mode):

  | Code | Symbols | QR version (error correction L) |
  | --- | --- | --- |
  | Challenge code | 14 | 1 (21×21) |
  | Scorecard | 49 | 3 |
  | Answer code, level 3 | 133 | 5 |
  | Answer code, level 5 | 259 | 8 (49×49) |
  | Full deflated patch | 1,100 | 19 |

  QR is useless between two desktops. Its later use is a result-card image whose screenshot can be dropped onto the plugin and verified.
  - Encoders: `qrcode` 0.14.1 (2024-07-05), MIT/Apache, or `fast_qr` 0.14.0 (2026-09-01), MIT.
  - Decoder: `rqrr` 0.11.0 (2026-09-02), (MIT OR Apache-2.0) AND ISC, from the quirc port.
  - Licence files checked. Defer past v1.

### 2.4 The recommended pair

**Scorecard** (about 211 bits, so 43 data symbols plus a 6-symbol check):

| Field | Bits | Note |
| --- | --- | --- |
| version 15 escape, kind, kind version | 5 + 5 + 4 | as challenge.md reserves |
| challenge (short spec; a flag selects a long spec) | 1 + 60 | self-contained |
| engine version | 24 | R5 must "refuse or rescore" on a mismatch (challenge.md) |
| answer commitment | 60 | SHA-256(`enth-answer-v1` ‖ salt ‖ canonical answer), truncated. **Salted**: a level-1 answer space (waveforms × semitones) is only hundreds, so a bare hash could be brute-forced |
| sound %, combined % (×10) | 10 + 10 | |
| passed, submits | 1 + 7 | |
| time to pass (deciseconds) | 16 | self-reported |
| assists (hints, live meter, dim-unused, revealed) | 4 | from `RoundResult.assists` |
| first attempt on this device | 1 | local start counter |

**Answer code:**

- **Contents:** the salt (64 bits) and the challenge (61). Then the diff from the round's start patch: a frozen, append-only address index plus the raw f32 bits (exact). Then structural entries (enable flags, routes), then a 40-bit hash tail.
- **Measured** [M] (`size_probe.py`, targets of seed 4,242, 160-bit header plus 42 bits per changed parameter):

  | Level | Changed parameters | Symbols |
  | --- | --- | --- |
  | 1 | 2 | 49 |
  | 3 | 12 | 133 |
  | 5 | 27 | 259 |

  Each extra parameter the player touched adds about 8.4 symbols.
- **Alternative:** a deflated whole patch, 595–685 bytes (952–1,096 base32 symbols) [M]. No new table, but 4–20× longer.
- **Strip patch `meta`:** `meta.author` can hold a real name.

**Flow and wording:**

1. A sends the challenge code, and both play.
2. Each copies a share message: a human line plus the scorecard. For example:
   ```
   enth duel · Level 3 · 1080-1000-0001-AQ
   ✅ 93.4 % in 2:41 · 4 submits · 0 hints
   ```
   followed by the scorecard. Optional closeness squares come from `components[].closeness`, always with words.
3. On paste: "Sam's result, *as reported by their app*. Swap answer codes to check it."
4. A received answer stays sealed until you've submitted or given up (a UI guard, not crypto).
5. Re-score both answers on your own build:
   - "Checked on your computer: their patch scores 93.4 %, as reported."
   - Or: "… 92.9 % here (they reported 93.4 % on engine 0.3.0, you run 0.4.0)."
6. Your machine's scores decide. The time reads "reported by their app; enth can't check times".

## 3. Fairness without a server

| Claim | Proved by re-rendering? |
| --- | --- |
| "My patch scores 93.4 %" | **Yes,** on the same engine. Scores are bit-exact native and wasm32 (`ss_determinism.rs`), and generator v1 is frozen |
| "My score beats yours" | **Yes,** when one machine scores both answers. Claimed numbers become irrelevant |
| "I made it myself" | **No.** `enth match` (CMA-ES) passes, and `enth challenge show --reveal --out` writes the target |
| "In 2:41" or "first try" | **No.** It comes from the shell's clock and local counters: editable, practisable, pausable |
| "My answer was fixed before I saw yours" | **Yes,** via the salted commitment in a scorecard sent first; the chat history shows the order |

- **No crypto bounds a time without trusted clocks at both ends.** RFC 3161 and OpenTimestamps need a network. A verifiable delay function proves a minimum elapsed time, not a maximum.
- **The chat is the referee:** the gap between sending the code and receiving the result is an upper bound the friend sees.
- **Race mode:** "Start together". The target is heard only after Start.
- **Label, don't detect** (TASVideos, DSDA, ICCF). Assists go on the card, and a revealed target gives no scorecard.
- **About text:** "Duel codes are for friends. A code proves what a patch scores, not who made it or how long it took. enth's own tools can solve any challenge."
- **Words to avoid:** "verified time", "proof", "anti-cheat". Use "checked on your computer" (score) and "reported" (time, assists).
- **Engine mismatch:** re-score and show both numbers. Refuse only on a generator or fingerprint mismatch (code v1's rule).
- **Copying:** scorecards first, answers later, stops honest-but-tempted friends. CLI-decoding cheats are out of scope; say so.

## 4. Privacy

- **Codes carry no names, machine ids or absolute timestamps** (durations only), and no patch `meta` or paths.
- **Names are optional and local:** the receiver may label a pasted result ("from: Sam") in the local duel history. The chat already shows the sender.
- **Seeds today** [M, code read]: `fresh_seed` in `crates/enth-plugin/src/game.rs` and the CLI's version mix wall-clock nanoseconds through the SplitMix64 finalizer, masked to 40 bits.
  - The finalizer is a bijection on u64, so an *unmasked* 64-bit clock seed would reveal its creation time to the nanosecond.
  - The 40-bit mask leaves about 80 candidate instants per day (2^46.3 / 2^40): no practical leak.
  - Keep the mask, or use `getrandom` for duel seeds.
- **Clipboard** write on click, read on an explicit paste. **No telemetry or network** in async duels.
- **Later LAN:** mDNS instance names reach the whole LAN, and default hostnames often hold the owner's name ([RFC 8117](https://www.rfc-editor.org/rfc/rfc8117.html), 2017-03: a fixed hostname is a name tag) [D]. Use random instance names, and switch off iroh's default `portmapper` (UPnP/NAT-PMP).

## 5. Later: LAN-first live duels in Rust

Versions come from the crates.io API and repo health from the GitHub API, both with User-Agent `enth-research/0.1`, on 2026-10-22. I checked licences in the repos' LICENSE files.

| Crate | Version (date) | Licence | Health | Notes |
| --- | --- | --- | --- | --- |
| **mdns-sd** | 0.21.6 (2026-10-19) | MIT OR Apache-2.0 | 6.5 M downloads, 227 stars, 19 open, pushed 2026-10-20; a single user account, very active | Pure Rust, its own thread, no async runtime |
| **quinn** | 0.11.12 (2026-09-14) | MIT OR Apache-2.0 | 368 M downloads, organisation, 5.3 k stars, last commit 2026-10-18 | QUIC streams; self-signed certificates pinned by a code |
| **libp2p** | 0.57.1 (2026-10-15) | MIT (Parity) | 19.7 M downloads, organisation, 5.6 k stars, 307 open | The full stack (mdns, quic, relay, DCUtR) |
| **iroh** | 1.4.1 (2026-10-21); 1.0 shipped 2026-06-15 ([blog](https://iroh.computer/blog/v1)) | MIT OR Apache-2.0 | 3.5 M downloads, organisation (n0), 12.8 k stars; Rust 1.91+ (we pin 1.99.1) | Dial by key. **n0's public relays are the default fallback** ([docs.rs](https://docs.rs/iroh/latest/iroh/)) [D]. Rate-limited, with old-version support time-limited. A company's servers, unless turned off or self-hosted |
| zeroconf | 0.18.0 (2026-04-07) | MIT (file; crates.io shows "non-standard") | 94 stars | Wraps Bonjour/Avahi; Windows needs Bonjour installed |
| libmdns / searchlight | 0.10.1 (2025-09) / 0.3.2 (2023) | MIT / MIT-Apache | responder only / stale | — |

**Size [M].** Tiny release binaries, x86-64 Linux, cargo's default release profile (the enth workspace sets none), rustc 1.95, `CARGO_BUILD_JOBS=2` under `flock`:

| Probe | Stripped | Over the baseline | Dependencies |
| --- | --- | --- | --- |
| std TCP+UDP | 0.38 MB | — | 1 |
| + mdns-sd | 1.16 MB | +0.8 MB | 16 |
| + quinn, rustls (ring), rcgen, tokio | 3.18 MB | +2.8 MB | 69 |
| + libp2p (mdns, quic, ping) | 9.44 MB | +9.1 MB | 241 |
| + iroh (defaults) | 15.6 MB | +15.2 MB | 311 |

For scale, today's release `libenth_plugin.so` is 19.7 MB unstripped.

**Network realities:**

- **macOS 15+** ([TN3179](https://developer.apple.com/documentation/technotes/tn3179-understanding-local-network-privacy), revised 2026-10-06) [D]:
  - Needs the permission: outgoing TCP to a LAN address, sending UDP, receiving multicast, and all Bonjour.
  - Does not: listening for TCP or receiving unicast UDP.
  - The **responsible code** is the app. For a plugin that is the DAW, whose `Info.plist` must carry `NSLocalNetworkUsageDescription`, and the alert names REAPER.
  - The multicast entitlement isn't needed on macOS. Users can reset a denial only from macOS 27.2 on.
  - **Unknown:** behaviour when the host lacks the key.
- **Windows:** inbound connections are blocked by default. The first listen alerts, and only administrators can allow it ([Microsoft, older docs](https://technet.microsoft.com/en-us/library/cc738649.aspx)) [D]. The rule names the DAW exe, and audio vendors tell users to allow each DAW by hand ([Neumann/Merging](https://help.neumann.com/hc/en-us/articles/36551155533586-Antivirus-and-Merging-Technologies-Softwares)) [D].
- **Wi-Fi client isolation** (guest, hotel and dorm networks) blocks client-to-client traffic ([Meraki](https://documentation.meraki.com/MR/Design_and_Configure/Firewall_and_Traffic_Shaping/Wireless_Client_Isolation)) [D]. Multicast pass-through varies by vendor ([OpenWrt forum](https://forum.openwrt.org/t/can-anyone-explain-how-isolate-clients-works/244879)) [A].
- **Internet:** DCUtR hole punching succeeds about 70 % ± 7.1 % across 4.4 M attempts, with TCP and QUIC alike; symmetric NAT is about 40 % on a small sample ([arXiv 2510.27500](https://arxiv.org/abs/2510.27500), IMC '26; [ProbeLab](https://probelab.io/blog/can-libp2p-punch-through-nats/)) [D]. The other ~30 % need a relay, which is a server. A claim of 90 % for iroh appears only on third-party pages [A].

**Advice (later):**

- **Home:** the standalone app (S4) first, with its own bundle, `Info.plist` and firewall entry. Then the plugin, once tested in hosts.
- **Discovery:** mdns-sd plus manual `IP:port`. The manual address also covers VPN overlays friends already run (Tailscale, ZeroTier) with no relay of ours.
- **Transport:** plain TCP, or quinn for encryption with a pinned key, carrying the same codes plus start and progress messages.
- **Threads:** off the audio thread, joined on deactivate and unload. Tokio in a `dlclose`-able plugin is untested; use the leak-test target.
- **Not libp2p or iroh** unless internet NAT traversal becomes a goal, and then with relays off or self-hosted.

## 6. Recommendations

**R5 v1 (async, no networking):**

1. Scorecard plus answer code on the version-15 escape, with stronger checks for long codes. Settle the roadmap/challenge.md conflict in the summary.
2. The answer is a diff from the start with exact f32 bits and a frozen address table, with `meta` stripped.
3. The receiver re-scores both answers, and that decides. Times and assists read "reported".
4. Paste-anything decoding: dashes, zero-width characters and emoji tolerated, several codes per paste, no clipboard auto-read.
5. A spoiler-free share line with optional emoji closeness squares and a plain-text option.
6. Sealed answers until you've played; "Start together" race wording; assisted runs marked like Wordle's asterisk.
7. A local duel history with optional receiver-side names.
8. Optional v1.1: a daily seed by a frozen formula, and a (time, score) ghost.

**Defer:** live duels (standalone first), QR cards, internet NAT traversal, leaderboards, and op-log replays (long and forgeable).

## 7. Risks and gaps

- **Which submit is compared:** `RoundResult.first_submit` (the recorded score), the first passing submit (time), or the best? The answer code must carry that submit's patch. This is row 2's call.
- **The frozen address table** is a new append-only table. The text-key alternative is about 2× longer (estimate).
- **Snapping values** to UI steps would shorten codes but must happen *before* scoring.
- **Gaps:**
  - Discord's support page and Gearspace returned 403.
  - No official WhatsApp or iMessage limits, and no ICCF draw statistics.
  - No current "sound design battle" series found.
  - Data detectors untested.
  - macOS hosts lacking the usage key untested.
  - Tokio-in-plugin unload untested.
  - Size probes were built with rustc 1.95 outside the plugin, so the deltas are approximate.
