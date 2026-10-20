# Playing 20 rounds of enth (for friends)

Thank you for trying this. You rebuild a sound by ear, enth scores how close you got, and then I ask what you thought of the score. 20 rounds take 40 to 100 minutes in all; stop whenever you like and come back.

Install enth first: [friends-install.md](friends-install.md). Start with your volume low.

## How to play 20 rounds

Play them as **4 sets of 5**. A set is one round at each level, 1 (easy) to 5 (hard), in any order. A round takes about 2 to 5 minutes, so a set is 10 to 25 minutes. One or two sets in a sitting is plenty. If I sent you a list of codes, start each round from the next code on it instead of choosing a level.

Each round:

1. Click **Recreate** at the top of the window. Pick the level with the five dots (1 to 5), or paste the code into **Paste a code**, then click **Start**. enth makes a hidden sound and gives you a patch to change.
2. **Listen**: **Target ▶** plays the hidden sound, **Mine ▶** plays yours, and **A/B** flips between them at the same place. Change your patch until they sound the same.
3. **Submit** when you think it is close. Before the score shows, enth asks **How close is yours to the target?**: click 1 ("Nothing like it") to 5 ("I can't tell them apart"), or **Skip**. Answer by ear, before you see the number.
4. **Vote** on the score, beside **Score felt**: **Too low**, **About right** or **Too high**. Please vote every time; this is the main thing I am measuring.
5. Keep going until it passes (you can **Submit** again), or use **Hint** or **Reveal** if you are stuck. Both are fine and enth notes that you used them. A round only counts if you submit at least once or reveal.
6. If you would like to play another, click **New round** under the score (or **Try again** for the same sound) without ending first: starting another round straight away is how you tell me you want more. Otherwise click **End**, then **End and restore my patch** (your own sound comes back) or **End and keep this patch**.

If a score is plainly wrong, click **Disagree…**, write one line about what you heard, and click **Send**. Leave out names and anything private.

## Where the log file is

enth writes every finished round to a small text file, **in your Documents folder**:

| System | File |
| --- | --- |
| Windows | `%USERPROFILE%\Documents\enth\Rounds\round-log.jsonl` (paste the folder into the Explorer address bar) |
| Mac | `~/Documents/enth/Rounds/round-log.jsonl` |
| Linux | `Documents/enth/Rounds/round-log.jsonl` in your home folder |

The file keeps growing across sittings, so you can send it once at the end. If a round shows up twice because you reopened your DAW project, that is fine.

## Sending it to me

Send me the one file `round-log.jsonl` by chat or e-mail, as an attachment (about 100 KB for 20 rounds). Also tell me, in a line each:

- which system and DAW you used;
- whether you would like to play more, yes, maybe or no, and anything that annoyed you.

## Privacy

- Nothing is sent anywhere by enth. The file leaves your computer only when you send it.
- The file holds the codes, the patches you built, your scores and times, your votes and ratings, and any dispute note you wrote. enth does not write your name, e-mail address or any folder path into it.
- It is plain text. Open it in any editor before you send it if you want to see what is in it. If a dispute note holds something you do not want to share, delete that line.
- I will call you "p2" or similar in my notes, not by name.
