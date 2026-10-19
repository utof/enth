# enth guide: expectations

An expectation list is comma- or semicolon-separated items (commas inside `[...]` belong to the item):

```text
item    := feature op value [tolerance] | feature in [low, high] | feature == null | feature != null
feature := a canonical path (spectrum.centroid_hz) or a short name (brightness), any case
op      := <  <=  >  >=  =  ==  ~        (!= only with null or a true/false feature)
value   := a number with an optional unit (2kHz, 10ms, 0.5s, -6dB, 30%, 1oct), a note for pitch (C3, A#4), true/false
tolerance := +- value  or  +- n%        (also written with the plus-minus sign; only after =, == or ~)
```

Examples: `brightness > 2kHz`, `attack < 10ms`, `sustain >= -6dB`, `release in [300ms, 1s]`,
`pitch = C3 +- 20ct`, `pitch.note ~ A3`, `tonal_pitch ~ A3`, `beating in [3Hz, 9Hz]`, `tremolo == null`,
`filter_lfo in [1Hz, 4Hz]`, `filter_sweep > 1oct`, `clipping = false`, `noisiness < 10%`.

## Rules
- A bare number is in the feature's own unit (`attack < 10` is 10 ms). A unit that does not fit is an error that
  lists the accepted ones.
- `=` and `~` without a tolerance use a default: 25 ct for `pitch` and `tonal_pitch`, 50 ct for `pitch.note` and
  `tonal.note` (the same note), 10 % for Hz and ms features, 2 dB for dB features, 1.5 LU for loudness.
- Pitch compared with `=` or `~` is judged in cents from the target note: `margin` is in cents
  (`margin_unit: "ct"`), and `note` shows the nearest note (`C3+9ct`).
- `pitch` is the period of the waveform (YIN, 25 Hz to 2 kHz): a sub oscillator an octave down makes it read an
  octave low, and noise above about 0.25 noisiness hides it. `tonal_pitch` reads the pitch of the steady spectral
  peaks instead, so a snare-like goal can ask for `noisiness in [0.4, 0.85], tonal_pitch ~ A3`.
- `filter_lfo` is the rate of a cyclic brightness swing (two cycles or more, at least 0.2 octave), `filter_sweep`
  the size in octaves of that swing or of a one-way filter envelope (at least 0.5 octave); `sweep_ms` the time of a
  one-way sweep. A tremolo does not move them (it changes the level, not the balance).
- `== null` passes when the feature is absent (no beating, no tremolo); `!= null` when it is present.
- Hz and ms items also report `margin_pct`, the margin relative to the threshold.
- A reading that is a lower bound (`duration_ms` when the sound outlasts the clip) can only pass `>` items.
- A true/false item (`clipping = false`) has margin 0; it is `thin` only when the re-renders disagree.
- The output echoes each item in canonical form with the canonical path: `envelope.attack_ms < 10`.

## Margins, noise and thin
`check` renders the patch three times: once with the judged render seed, and twice more with other seeds, which
only move the oscillators' start phases (oscillators in the default `free` phase mode). `noise` is the largest
change of the reading across them. A result with |margin| <= 2 x noise is `thin`: an inaudible change could flip
it. For a WAV, the noise comes from re-analysing the signal shifted by a few samples instead.

## Writing good lists
- Bracket what matters with two items (`brightness > 1.2kHz, brightness < 2.5kHz`) rather than `=`.
- Leave room: an item that passes by 1 % will flip with the next small edit.
- Envelope items need the right performance: sustain, decay and release are read around the note-off, so hold
  the note long enough and make the clip cover the release (`--perf note=C3,hold=1.5s,dur=3s`).
- Check a list once on the starting patch: items that are already met with a wide margin cost nothing.
