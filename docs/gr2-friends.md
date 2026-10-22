# Learning with enth: the tour (for friends)

Thank you for trying this. enth has lessons now: short steps where you hear a difference, read a few lines, turn a knob and then rebuild a sound by ear. I want to know whether you learn something from them. There are two ways to help; my message says which one you picked:

| Way | What you do | Time |
| --- | --- | --- |
| **The tour** | the 13 lessons of the tour (about 7 minutes each), a quick vote after each, three lines at the end | about 1½–2 hours, in sittings of two or three lessons over a week or two |
| **The tour with tests** | the same, plus three short tests of 6 sounds each: before the first lesson, after lesson 5, and after the last; a fourth a week later if you like | about 3 hours in all |

Install enth first: [friends-install.md](friends-install.md) (the zip has a copy as `INSTALL.md`). Start with your volume low.

## 1. Add the lesson packs (5 minutes)

The plugin comes with the first pack of lessons built in. The tour also uses two more packs, which are in the zip's `Packs` folder (`shaping.enthpack`, `space-and-dynamics.enthpack`, and `studies.enthpack`, six longer sound-design lessons you can try afterwards).

1. Open enth in your DAW and click **Learn** at the top of the window. The lesson list opens on **The tour**; lessons 8 to 13 say "not installed" for now.
2. At the bottom of the list, click **Open packs folder**. A folder opens (`Documents/enth/Packs`).
3. Copy the three `.enthpack` files from the zip's `Packs` folder into it. Do not unzip them.
4. Close the enth window and open it again. The tour now lists all 13 lessons, none "not installed".

## 2. Play the tour

Click **Learn**, then **Start** on the next lesson of **The tour** (start with 1, "How a lesson works"; it shows you around). A lesson goes step by step in the panel on the right: listen to the examples, read, try the knob it names, then rebuild a hidden sound by ear (the same **Target ▶**, **Mine ▶**, **A/B** and **Submit** as a Recreate round). **Next ▶** moves on; **Hint** and **Show me** help when you are stuck (that is fine; enth notes it). At the end, **Finish**, then **End and restore my patch** gives your own sound back.

**After each lesson**, fill its block in the answers file (below): was it new to you or did you know it, was it too easy, about right or too hard, and, if you can, one line on what you learned. A specific thing ("a saw is buzzier than a triangle") helps me more than "a lot".

## 3. The tests (only if you picked the tour with tests)

A test is 6 codes. Each code is one Recreate round, like the ones you may have played before:

1. Click **Recreate**, paste the code into **Paste a code**, then click **Start**.
2. Rebuild the sound by ear, **for at most 3 minutes** (the round's timer shows the time). **No Hint, no Reveal**, and leave "Dim the modules the target leaves alone" off. Submit as often as you like.
3. Stop when it passes or when the 3 minutes are up, whichever comes first: **End**, then **End and restore my patch**. If enth asks how close yours is, answer or skip; the vote after the score is optional here.

Play the 6 codes in the order given, one straight after the other (about 20–25 minutes). Use the form my message names: your **pre-test** is form A or B, your **post-test** is the other one, the **mid-test** (after lesson 5, Low-pass cutoff) is form M, and the **delayed test** (a week after the post-test, if you like) is form D. Do not practise on these codes; each code counts once, the first time you play it.

| When | Form | |
| --- | --- | --- |
| Before lesson 1 | A or B (my message says) | the pre-test |
| After lesson 5 (Low-pass cutoff) | M | the mid-test |
| After lesson 13 (Macros) | the other of A and B | the post-test |
| A week later (optional) | D | the delayed test |

<!-- gr2-codes:start -->
The codes (also in the enth repository, `content/gr2/forms.toml`). Copy each one whole; case, hyphens and spaces do not matter.

**Form A**

```text
1  H000-G000-0000-00MG-9068-0000-C020-B457-3ZJ0-0000-000H
2  H000-G000-0000-00MG-9S68-0000-C098-FC0H-3ZJ0-0000-1J0M
3  H000-G000-0000-00MG-96E8-0000-C01D-XYAX-3Y0C-G000-0005
4  H000-G000-0000-00MG-9CP8-0000-C02S-DFPB-QY00-2B00-0009
5  H000-G000-0000-00MG-DJY8-0000-C046-PB68-QY00-00S0-000M
6  H000-G000-0000-00MG-9S68-0000-C04N-8CTS-QY00-0006-800J
```

**Form B**

```text
1  H000-G000-0000-00NF-J068-0000-C00S-TRDT-QZJ0-0000-000Q
2  H000-G000-0000-00NF-HS68-0000-C024-6GWR-KZJ0-0000-1J0Q
3  H000-G000-0000-00NF-H6E8-0000-C095-QR38-VY0C-G000-000A
4  H000-G000-0000-00NF-HCP8-0000-C051-W115-BY00-2B00-0002
5  H000-G000-0000-00NF-QJY8-0000-C00P-WHXX-QY00-00S0-000A
6  H000-G000-0000-00NF-KS68-0000-C0AZ-D1YT-BY00-0006-8006
```

**Form M**

```text
1  H000-G000-0000-00PE-S068-0000-C020-B457-3ZJ0-0000-000T
2  H000-G000-0000-00PE-WS68-0000-C0F9-83GQ-ZZJ0-0000-1J0J
3  H000-G000-0000-00PE-V6E8-0000-C0C1-TM4Q-KY0C-G000-000T
4  H000-G000-0000-00PE-SCP8-0000-C06Q-8WPZ-BY00-2B00-0007
5  H000-G000-0000-00PF-1JY8-0000-C02W-N7MP-ZY00-00S0-000N
6  H000-G000-0000-00PF-2S68-0000-C02K-QZJA-VY00-0006-800M
```

**Form D**

```text
1  H000-G000-0000-00QE-2068-0000-C02T-31Q5-7ZJ0-0000-000F
2  H000-G000-0000-00QE-4S68-0000-C0CX-0K04-ZZJ0-0000-1J0D
3  H000-G000-0000-00QE-16E8-0000-C04E-EXXW-FY0C-G000-000B
4  H000-G000-0000-00QE-1CP8-0000-C04H-48S7-BY00-2B00-0007
5  H000-G000-0000-00QE-5JY8-0000-C035-TFMS-QY00-00S0-000B
6  H000-G000-0000-00QE-3S68-0000-C0EV-1E7X-QY00-0006-800S
```
<!-- gr2-codes:end -->

## The answers file

The zip holds `gr2-answers.toml`, a plain-text form with a block for each tour lesson. Rename it to your id from my message (`p2.gr2.toml`, `p3.gr2.toml`, …) and open it in a plain-text editor: Notepad on Windows; on a Mac, TextEdit after **Format > Make Plain Text**, with **Edit > Substitutions > Smart Quotes** off (curly quotes break the file). Write only between the quotes `""`, and leave what you skip as `""`. At the top, set `protocol` to `"full"` if you do the tests (and `pre_form` to `"A"` or `"B"`), else leave `"tour"`. Leave every `coded = ""` alone: that is where I mark your notes.

If editing the file is a bother, send me the answers in a message instead (per lesson: new or knew, too easy, OK or too hard, and a line on what you learned), and I will type them in.

## Sending it back

At the end (or whenever you want to stop), send me these files, by chat or e-mail as attachments:

| File | Where it is |
| --- | --- |
| your answers, `p2.gr2.toml` (your id) | wherever you saved it |
| `progress.json` (which steps you passed) | `Documents/enth/Learn/progress.json`; on Windows `%USERPROFILE%\Documents\enth\Learn\progress.json` (paste the folder into the Explorer address bar) |
| `round-log.jsonl` (the test rounds; only with the tests) | `Documents/enth/Rounds/round-log.jsonl`, the same file as for the 20 rounds, if you played them |

Also tell me, in a line each, which system and DAW you used, and anything that annoyed you or broke.

## Privacy

- Nothing is sent anywhere by enth. The files leave your computer only when you send them.
- `progress.json` holds lesson and step names, whether each was passed, and your best score in each lesson round. The round log holds the codes, the patches you built, your scores and times. The answers file holds what you write. None of them has your name, e-mail address or a folder path, unless you type one: please don't.
- All three are plain text; open them before you send if you want to see what is in them.
- I will call you "p2" or similar in my notes, not by name.
