//! `guide` and `params`: the self-describing output, so the agent needs no outside documentation.

use crate::expect::FEATURES;
use crate::patch::{
    CONT, DUR_KEY, DUR_MAX_S, DUR_MIN_S, HOLD_KEY, HOLD_MIN_S, NOTE_KEY, Patch, WAVE_EFFECT,
    WAVE_KEY,
};
use crate::units::{Unit, fmt_value, r4};
use serde_json::{Value, json};

/// `params`: names, units, ranges, defaults and the perceptual effect of every key.
#[must_use]
pub fn params() -> Value {
    let def = Patch::default();
    let mut list = vec![json!({
        "key": WAVE_KEY, "kind": "enum", "values": ["saw", "square", "triangle"],
        "default": def.wave.name(), "effect": WAVE_EFFECT,
    })];
    for (spec, v) in CONT.iter().zip(&def.v) {
        list.push(json!({
            "key": spec.key, "kind": "number", "unit": spec.unit.label(),
            "min": r4(spec.min), "max": r4(spec.max), "default": r4(*v),
            "scale": if spec.log { "log" } else { "linear" }, "effect": spec.effect,
        }));
    }
    json!({
        "schema": "enth.params/1",
        "engine": "toy10: two detuned oscillators -> lowpass filter (with envelope) -> amp ADSR. One voice, one note, mono, 48 kHz",
        "params": list,
        "performance": [
            {"key": NOTE_KEY, "kind": "note", "default": "A3", "min": "50Hz", "max": "2kHz",
             "effect": "The note played: a name (A3, C#4) or Hz. Sets the pitch; the sound design only detunes it"},
            {"key": DUR_KEY, "kind": "number", "unit": "s", "min": r4(DUR_MIN_S), "max": r4(DUR_MAX_S), "default": 2,
             "effect": "Clip length. It must cover the tail you want measured (hold plus about 1.2 times amp.release)"},
            {"key": HOLD_KEY, "kind": "number", "unit": "s", "min": r4(HOLD_MIN_S), "max": r4(DUR_MAX_S), "default": 1.5,
             "effect": "How long the note is held before release; at most perf.dur. Sustain, decay and release are read around this moment"},
        ],
        "set_syntax": "--set key=value[,key=value...]; values take units (800Hz, 1.2kHz, 40ms, 0.5s, -6dB, 12ct, 3oct) or a bare number in the unit shown above",
    })
}

/// `guide`: the one-page workflow.
#[must_use]
pub fn guide() -> Value {
    let descriptors: Vec<Value> = FEATURES
        .iter()
        .map(|f| {
            json!({
                "name": f.alias(), "path": f.name(),
                "unit": if f.unit() == Unit::Ratio { "0..1 or %" } else { f.unit().label() },
                "meaning": f.meaning(),
            })
        })
        .collect();
    json!({
        "schema": "enth.guide/1",
        "tool": "spike-agent: a CLI over a toy synth (two oscillators, lowpass filter with envelope, amp ADSR; one voice, one note). It renders and MEASURES; you cannot hear it, so work from the numbers",
        "goal": "Reach a text sound goal (e.g. 'short bright pluck') by editing the patch until every expectation passes",
        "division_of_labour": "You choose parameters and write expectations in real units. The tool renders deterministically, measures descriptors, says pass/fail per expectation, and for failures gives a measured hint",
        "workflow": [
            "1. spike-agent params   (once: keys, units, ranges, what each does)",
            "2. Turn the goal into an expectation list in descriptor terms (or use the one you were given)",
            "3. spike-agent new --out p.json [--perf note=C3,dur=3s,hold=2s]",
            "4. spike-agent step p.json --set key=value,key=value --expect '<list>' --out p.json   (repeat). Exit code 0 = all pass",
            "5. On failure read check.results[i].hint: param, now, try, expected (what that one change measured). Apply, step again",
            "6. When it passes (or your render budget is spent) say what you built and which values mattered"
        ],
        "commands": [
            {"cmd": "guide", "does": "this page"},
            {"cmd": "params", "does": "every key with unit, range, default, perceptual effect"},
            {"cmd": "new --out p.json [--perf note=A3,dur=2s,hold=1.5s]", "does": "write a neutral starting patch (saw, 2 kHz lowpass, 5 ms attack, 300 ms decay, -6 dB sustain, 200 ms release)"},
            {"cmd": "apply p.json --set k=v[,k=v] [--out p2.json] [--dry-run]", "does": "edit parameters (in place without --out); prints a from/to diff"},
            {"cmd": "render p.json [--out p.wav]", "does": "write a 16-bit mono 48 kHz WAV"},
            {"cmd": "describe <p.json|x.wav> [--explain]", "does": "all descriptors; --explain adds a one-line meaning for each"},
            {"cmd": "check <p.json|x.wav> --expect '<list>'", "does": "pass/fail per expectation; failures on a patch carry a measured hint"},
            {"cmd": "step p.json [--set ...] [--expect ...] [--out p2.json]", "does": "apply + render + describe + check in one call (writes the patch and a WAV beside it)"}
        ],
        "expectations": {
            "syntax": "comma-separated items: <descriptor><op><value><unit>. Ops: < <= > >= =. '=' takes a tolerance: pitch=C3±20ct, brightness=1500Hz±200Hz (default: pitch ±25ct, other scales ±10%)",
            "examples": ["brightness>2kHz", "attack<10ms", "sustain<-20dB", "pitch=C3±20ct", "release>500ms", "beating>2Hz"],
            "descriptors": descriptors,
            "also_in_describe": ["pitch.note and pitch.cents_dev", "pitch.stability (1 = steady)", "spectrum.centroid_track_hz [start, middle, end]", "detune.depth_db (how deep the beating is)"],
        },
        "hints": "A failed expectation on a patch carries hint {param, now, try, expected, reaches, others}. 'expected' was measured by rendering that single change. reaches=false means that parameter alone cannot get there; 'others' lists parameters that push the same way. Hints are one parameter at a time: after combining several, re-check everything",
        "exit_codes": {"0": "ok (check: all passed)", "1": "check ran but some expectation failed", "2": "usage error", "3": "validation error (unknown param, bad unit, out of range)", "4": "render or analysis failure", "5": "file not found"},
        "tips": [
            "Output is one JSON object per call; errors are JSON too, with a hint.",
            "Change one to three parameters per step. Filter cutoff mostly sets brightness, amp.attack the attack, amp.sustain the held level.",
            "perf.note sets the pitch, perf.dur the clip length, perf.hold when the note ends. sustain, decay and release are read around perf.hold, and a release longer than the rest of the clip is reported as a lower bound.",
            "Descriptors are measurements of this render; they approximate what a listener hears. Strong detune makes the level wobble, which blurs attack and sustain readings by a few dB."
        ],
    })
}

/// Text for `new`'s `perf` echo.
#[must_use]
pub fn perf_echo(p: &Patch) -> Value {
    json!({
        "note": p.show(crate::patch::Key::Note),
        "dur": fmt_value(p.perf.dur_s, Unit::Seconds),
        "hold": fmt_value(p.perf.hold_s, Unit::Seconds),
    })
}
