# Taste check: two lessons of the basics pack (lane C1)

Before the rest of the pack is written, taste two lessons (about 20 to 30 minutes; docs/phase-3.md, "Your part"):

| Lesson | Directory | Why these two |
| --- | --- | --- |
| 1.1 Waveforms | [`basics/lessons/waveforms/`](basics/lessons/waveforms/) | An enum control (the wave), drawn from a set |
| 2.1 Low-pass cutoff | [`basics/lessons/cutoff/`](basics/lessons/cutoff/) | A continuous control, with a sound check and a drawn range |

This note sits beside the pack, not in it: a pack directory holds only what its manifest lists (`pack.toml`,
`CREDITS.md`, `lessons/`, `patches/`).

## Reading them as markdown

Open the lesson's `lesson.toml`: its steps are in playing order, and the comment above each one says what it is and
what was measured. Each step names its text, a markdown file in the same directory (`hear.md`, `name.md`, …), which is
what the player reads. A lesson runs:

1. **Hear**: two `listen` steps (the labels are the buttons; the patches are in
   [`basics/patches/`](basics/patches/)), then a `choose` step, the first ear question.
2. **Name**: `name.md`, at most about 80 words.
3. **Try**: a `do` step with a goal checked live (`[steps.check]`), and `hints` shown on request.
4. **Do**: three `recreate` steps, real game rounds on a hidden target drawn from the values in
   `[steps.challenge.target]` (the start patch is written inline below it).
5. **Check**: two more `choose` steps.

## Reading them through `enth lesson show`

Lane SKL is adding `enth lesson show` and `enth lesson check`, which read the pack in this layout. Once they land,
`enth lesson show --help` gives the arguments; point it at `content/lessons/basics` and the lesson ids `waveforms` and
`cutoff`.

## What to judge

Report 01's rubric (§3, docs/learn.md "The lesson shape"):

- Is the contrast heard before it is explained, and is each Hear pair about the new control only?
- Does the Name text use words for the sound (darker, buzzy, hollow) rather than for the circuit?
- Is the Try goal clear from its text alone?
- Are the rounds' targets far enough apart to hear, and are the hints listening cues rather than answers?
- Would you learn something, or is it too easy or too slow?
