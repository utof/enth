# ADSR envelope (`enth-dsp::envelope`, `enth-engine::modules::adsr`)

- Lane: P1a D2. Code: `crates/enth-dsp/src/envelope.rs` (the state machine and the maths, white-box tests), `crates/enth-engine/src/modules/adsr.rs` (the per-voice module). Catalogue entry: `crates/enth-patch/src/catalogue/adsr.rs`. Bench: `crates/enth-engine/benches/adsr.rs`. Goldens: `tests/golden/cases/adsr-*.toml`. The gate contract: `crates/enth-engine/tests/contract.rs`.
- Decisions behind it: research report 02 (RC curves, segment-duration semantics), Nigel Redmon's target-ratio idea (EarLevel Engineering, "ADSR envelope generator", 2013-06-03; the idea only, the code is ours).

An attack-decay-sustain-release envelope with RC (exponential) curves. Every patch has one as its amplitude envelope (`voice.amp_env`), and any `adsr` module can be a modulation source. It is stepped once per 32-frame segment per sounding voice; the engine ramps the amplitude linearly from the previous segment's value to this one's across the 32 frames, so the audible curve is the control-rate curve joined by straight lines.

## The curve

Each segment (attack, decay, release) moves the level towards an *overshoot target* beyond its end:

```text
target = end + (end − start)·r          level ← target + (level − target)·c          c = (r / (1 + r))^(1/N)
```

That is an analogue RC charge curve cut off where it crosses `end`. Because the overshoot is proportional to the segment's own span (`end − start`), the curve crosses `end` after exactly `N` steps from **any** start level; a retrigger from 0.4, a release from the middle of an attack and a decay to a sustain that moved all take the set time. On the segment's last step the level is set to `end` exactly (a step counter, not a comparison), so the stages hand over without rounding residue and a released voice reaches exactly 0.0.

| Segment | `r` | Shape |
| --- | --- | --- |
| attack | 0.3 | gently convex; 0.68 of full at half the time, from any start |
| decay, release | `10^(−dB/20)`, `dB = min(80, 8·(N − 1))` | an RC fall: 80 dB below the span for any segment of 11 steps (7 ms) or more; flatter for shorter ones (see below) |

### Segment-duration semantics

Attack, decay and release are *times*: how long that segment takes. `decay = 300 ms` reaches the sustain level at 300 ms (at the first control step at or after it: never early, at most one 32-frame step, 0.67 ms, late), not "a rate that would fall the whole range in 300 ms". `N = ms · fs / (1000 · 32)` is **fractional** (300 ms at 48 kHz is 450.0, 5 ms is 7.5); the curve is the continuous-time one sampled at whole steps, the segment lasts `ceil(N)` ticks, and the level snaps to the end on the last. Fractional `N` is deliberate: the first version rounded the length up to whole steps, and a 0.3 ms nudge of `release` (300 → 300.3 ms) flipped a whole step (450 → 451) and moved the RMS envelope by 0.057 dB, failing the seed-stability test (0.05 dB). With a continuous curve the same nudge moves it 0.025 dB, which also keeps the finite-difference parameter hints well behaved.

### Minimum ramp: 1 ms per segment

A segment lasts at least `MIN_RAMP_MS = 1.0` ms: `N ≥ 1.5` at 48 kHz, so 2 ticks. A time of 0 ms is therefore a ramp of about a millisecond, never a jump; "only the discontinuity goes": a 0 ms decay with sustain 0 is a tight percussive hit (a 1 ms fade-out, no click), and a 5 ms or 20 ms pluck is untouched.

- **Ticks at each sample rate** (`ceil(1 ms · fs / 32)`): 1 at 8 kHz and 32 kHz, 2 at 44.1 and 48 kHz, 3 at 96 kHz, 6 at 192 kHz, 12 at 384 kHz. `adsr::min_ramp_steps(fs)` returns it.
- **The curve flattens for short segments.** The 80 dB decay/release curve of a 1.5-step ramp would lose 40 dB in its first step, a click again. The overshoot therefore depends on `N` (`overshoot`): 4 dB at 1.5 steps, 8 dB at 2, 16 dB at 3, 80 dB from 11 steps. A 0 ms release from 1.0 at 48 kHz goes 1.0 → 0.235 → 0.0 (the amp stage's linear ramps make that two straight segments); a 2-step segment leaves 0.35 after its first step where an 80 dB curve would leave 0.01.
- **A segment with no span takes one step.** Attack from 1.0, decay to a sustain of 100 %, release from 0 finish on their first step instead of waiting out a segment that moves nothing.

### The gate envelope (attack 0, decay 0, sustain 100 %, release 0)

Module goldens play through this envelope so that they do not depend on the shape of the real one (`ref-saw-gate`, and the convention in engine.md). S2a's contract test required it to be exactly 1.0 at the end of the first segment and exactly 0.0 one segment after the note-off; with a 1 ms minimum ramp, 1.5 steps cannot do that at 48 kHz. The contract is now stated in terms of the minimum ramp, and `tests/contract.rs` was amended in the same commit as this lane (the file is S2a's, the change is the minimum necessary):

- while the key is down, the first `n − 1` segments are strictly between 0 and 1 and rising, then the output is **exactly 1.0**, and stays exactly 1.0;
- after the note-off, the first `n − 1` segments are strictly falling and the voice is not finished, then the output is **exactly 0.0** and `voice_finished` is true at segment `n`;

with `n = min_ramp_steps(fs)`: 2 at 44.1 and 48 kHz, 3 at 96 kHz, 12 at 384 kHz, 1 at 8 kHz (where the old statement holds literally). A gate envelope stays a gate: a `1 ms` attack and release, constant in between, exactly 0 afterwards. The goldens `ref-saw-gate`, `ref-saw-1v` and `engine-chord` moved when the envelope changed (the first about 0.03 dB at note-on and 0.8 dB in the release segment).

## The module

Per sounding voice and segment:

1. A note-on landing on the voice (`NoteState::trigger`), or a voice whose gate is up while the envelope is still idle (its first segment), calls `gate_on`: the attack starts **from the current level**, so a retrigger is continuous and a fresh voice starts at 0.
2. A fallen gate calls `gate_off`: the release starts from the current level. It does nothing unless the key was down.
3. The envelope steps once: `level = env.tick(timing, sustain/100, decay)`; the attack's end starts the decay (or goes straight to sustain when `sustain ≥ 100 %`); during the sustain the level *is* the sustain parameter, so it follows moves (smoothed by the engine); a decay that is running when sustain moves aims at the new value.
4. The output is `level · (1 − v + v·velocity)` with `v = velocity% / 100`: the catalogue's `velocity` percentage scales the output from "ignores velocity" (0 %, the default) to "follows it fully" (100 %).

The times are read from the parameters' `cur` values at the start of each segment (a segment in flight keeps its length).

**Voice end.** `voice_finished` is true when the release has finished (exactly 0), or when the envelope is past its attack and its output is below `VOICE_END_LEVEL` (1e-5, −100 dB). The release's 80 dB curve crosses −100 dB on its last step or two, so a voice ends within a step of its release time, and a decay to a sustain of 0 (a pluck) ends the voice while the key is still down. During the attack a voice never ends (an envelope at 0 on its first segment is not silence).

## Determinism

`exp2`, `ln` and `exp` run only when a segment starts, in `f64`, through `enth_dsp::util::math`. A step is one multiply and one add (written as a multiply and an add, never `mul_add`), and the level is flushed (`util::flush::flush`, 1e-15) every step. The tests assert that no state is ever subnormal or NaN, and that the goldens `adsr-*` are the same on wasm (`cargo xtask golden --wasm`).

## Tests

- `enth-dsp` (white-box): step counts and the minimum at several rates; a segment takes exactly `N` steps from any start level (attack, decay, release); the curve is the RC curve with the stated overshoot; decay and release hit their ends from any level; a 0 ms attack, decay and release each take an intermediate step (attack to 0.2 … 0.9, decay to 0.5 … 0.9 of the span, a release from 0.5 to 0.05 … 0.35, then exact ends), never a jump; retrigger continuity and zero-span segments; sustain jumps; the release passes −100 dB before it ends; levels vary continuously with the set time (300 vs 300.3 ms, under 0.1 dB); no subnormals.
- Module: the minimum ramp counts, the gate envelope exact at 8 kHz, 44.1 kHz, 48 kHz, 96 kHz and 384 kHz, the default envelope's times (8, 450 and 600 ticks), the −100 dB voice end, not finished during an attack or while held, a pluck ending while held, velocity, retrigger, times read at the start of a segment, per-voice independence, `copy_state_from`.
- Goldens: `adsr-pluck` (zero attack, 120 ms decay to sustain 0, the voice ends while held), `adsr-pad` (400 ms attack, 600 ms decay, 900 ms release), `adsr-zero-times` (all times 0, sustain 50 %, and a 10 ms note), `adsr-velocity` (three velocities), `adsr-long-release` (5 s release, a second note during the tail, silence after it), `adsr-retrigger` (polyphony 1, retriggers from non-zero levels).

## Cost

Callgrind, 1 s at 48 kHz, whole engine. The `engine` bench (one saw, the default patch's ADSR as the amplitude envelope) became the baseline for every module bench; the `adsr` bench adds a second ADSR.

| Case | Instructions | Budget (+10 %, rounded up to 1,000) | Marginal |
| --- | --- | --- | --- |
| `engine` `saw_1v` | 3,873,015 | 4,261,000 (was 4,030,000) | |
| `engine` `saw_16v` | 43,370,997 | 47,709,000 (was 44,400,000) | |
| `adsr` `held_1v` | 4,301,515 | 4,732,000 | |
| `adsr` `held_16v` | 47,071,463 | 51,779,000 | 4.8 per sample per voice |

Callgrind's self cost for an ADSR is about 1.5 instructions per sample per voice (roughly 48 per voice per segment); the rest of the marginal figure is the engine handling one more modulator output per voice per segment. The `engine` budgets are the first and second lines of the bench because every patch carries an ADSR: they were re-measured in the same commit that replaced the reference envelope. At S2a's own commit the reference envelope measured 3,984,758 and 44,866,745 on this machine (its recorded 3,660,346 and 40,367,014 and its 44.4 M budget no longer held here, a machine or toolchain difference, not a code change), so the real ADSR is a little cheaper than the reference one. Lane D1 re-measures both again with the real oscillator.

## Behaviour other lanes must know

- `adsr::min_ramp_steps(sample_rate)` is the length of the shortest segment in 32-frame segments; `tests/contract.rs` uses it.
- docs/dsp/engine.md states the gate-envelope contract in terms of `min_ramp_steps` (updated in the CH integration pass), and the module comment of `crates/enth-patch/src/catalogue/adsr.rs` says the shortest segment is 1 ms rounded up to whole steps.
- Modulating `attack`, `decay`, `release` or `sustain` works through `cur`: the times are read at the start of each segment, so a route changes the next segment, never one in flight.

## References

- Research report 02 (RC envelope shapes and the segment-duration semantics).
- N. Redmon, "ADSR envelope generator" (Earlevel Engineering, 2013-06-03): the target-ratio ("overshoot") idea for exponential segments.
