//! Tempo as a render input (S2a, lane TMP): a tempo event applies at a segment boundary like every event, the host's
//! block size cannot change a render with a tempo change in it, and the `lfo` that syncs to the tempo is exact in
//! beats. A global LFO drives `comp1.mix` (50 % ± 50 % of the LFO), so the LFO's value after each segment is read
//! back with `Engine::value` and compared with a closed form computed in `f64` here, independently of the DSP.

mod common;

use common::{SEED, SR, bits, drive, engine, every_kind_patch, id, note, reset_saw_patch};
use enth_engine::{
    DEFAULT_TEMPO_BPM, Engine, Event, EventKind, LinkCapacity, MAX_TEMPO_BPM, MIN_TEMPO_BPM,
    NoteKey, ParamId, PrepareCfg, SEG, compile,
};
use enth_patch::catalogue::{Beats, CompParam, LfoParam, LfoScope, LfoShape};
use enth_patch::{ModuleInstance, ModuleKind, Patch, Route};

fn route(p: &mut Patch, src: &str, dst: &str, depth: f32) {
    p.routes.push(Route {
        src: src.parse().unwrap(),
        dst: dst.parse().unwrap(),
        depth,
    });
}

fn lfo(name: &str, scope: LfoScope, shape: LfoShape, beats: Beats) -> ModuleInstance {
    let mut m = ModuleInstance::new(id(name), ModuleKind::Lfo);
    m.values[LfoParam::Scope.index()] = scope.index() as f32;
    m.values[LfoParam::Shape.index()] = shape.index() as f32;
    m.values[LfoParam::Sync.index()] = 1.0;
    m.values[LfoParam::Beats.index()] = beats.index() as f32;
    m
}

/// A global synced LFO `lfo1` drives `comp1.mix`: mix = 50 + 50 · (the LFO's −1..1 value).
fn global_lfo_patch(shape: LfoShape, beats: Beats) -> Patch {
    let mut p = reset_saw_patch();
    let mut comp = ModuleInstance::new(id("comp1"), ModuleKind::Comp);
    comp.values[CompParam::Mix.index()] = 50.0;
    p.fx.push(comp);
    p.modulators
        .push(lfo("lfo1", LfoScope::Global, shape, beats));
    route(&mut p, "lfo1", "comp1.mix", 50.0);
    p
}

struct Rig {
    engine: Engine,
    mix: ParamId,
}

impl Rig {
    fn new(patch: &Patch) -> Rig {
        let (plan, map) = compile(patch, &PrepareCfg::new(SR)).expect("the patch compiles");
        let mix = map
            .get(&"comp1.mix".parse().unwrap())
            .expect("comp1.mix is a parameter");
        Rig {
            engine: Engine::new(plan, SEED),
            mix,
        }
    }

    /// Run one segment (one 32-frame call) with `events` at offset 0, then the global LFO's value (−1..1) in it.
    fn segment(&mut self, events: &[EventKind]) -> f64 {
        let ev: Vec<Event> = events
            .iter()
            .map(|&kind| Event { offset: 0, kind })
            .collect();
        let (mut l, mut r) = ([0.0f32; SEG], [0.0f32; SEG]);
        self.engine.process(&mut l, &mut r, &ev);
        (f64::from(self.engine.value(self.mix, 0).expect("a value")) - 50.0) / 50.0
    }

    /// The LFO's value after each of `segments` segments; `at(k)` gives the events at the start of segment `k`.
    fn trace(&mut self, segments: usize, mut at: impl FnMut(usize) -> Vec<EventKind>) -> Vec<f64> {
        (0..segments).map(|k| self.segment(&at(k))).collect()
    }
}

fn tempo(bpm: f64) -> EventKind {
    EventKind::Tempo { bpm }
}

/// Cycles per 32-frame segment of a cycle of `beats` at `bpm`, in `f64`.
fn cycles_per_segment(bpm: f64, beats: Beats) -> f64 {
    let (n, d) = beats.length();
    let cycle_seconds = f64::from(n) / f64::from(d) * 60.0 / bpm;
    SEG as f64 / (cycle_seconds * f64::from(SR))
}

fn sine(cycles: f64) -> f64 {
    (std::f64::consts::TAU * cycles.fract()).sin()
}

fn assert_close(got: f64, want: f64, tol: f64, what: &str) {
    assert!((got - want).abs() <= tol, "{what}: got {got}, want {want}");
}

#[test]
fn a_synced_lfo_runs_exactly_in_beats_at_two_tempos_and_four_divisions() {
    // Quarter, a triplet, a dotted note and a whole bar; 90 and 174 BPM; 30 s of the sine each.
    for bpm in [90.0, 174.0] {
        for beats in [
            Beats::Quarter,
            Beats::EighthTriplet,
            Beats::QuarterDotted,
            Beats::Bar,
        ] {
            let mut rig = Rig::new(&global_lfo_patch(LfoShape::Sine, beats));
            let cps = cycles_per_segment(bpm, beats);
            let trace = rig.trace(45_000, |k| if k == 0 { vec![tempo(bpm)] } else { vec![] });
            for (i, v) in trace.iter().enumerate() {
                // The LFO advances, then reads: the value after segment i is the sine at (i + 1) increments.
                assert_close(
                    *v,
                    sine((i as f64 + 1.0) * cps),
                    2e-5,
                    &format!("{beats:?} at {bpm} BPM, segment {i}"),
                );
            }
        }
    }
}

#[test]
fn a_tempo_change_keeps_the_phase_and_changes_only_the_speed() {
    let beats = Beats::EighthDotted;
    let changes = [(0usize, 90.0), (700, 174.0), (1500, 63.5), (1800, 140.0)];
    let mut rig = Rig::new(&global_lfo_patch(LfoShape::Sine, beats));
    let trace = rig.trace(4000, |k| {
        changes
            .iter()
            .filter(|(at, _)| *at == k)
            .map(|(_, bpm)| tempo(*bpm))
            .collect()
    });
    let mut phase = 0.0f64;
    for (i, v) in trace.iter().enumerate() {
        let bpm = changes.iter().rev().find(|(at, _)| *at <= i).unwrap().1;
        phase += cycles_per_segment(bpm, beats);
        assert_close(*v, sine(phase), 2e-5, &format!("segment {i}"));
    }
}

#[test]
fn a_tempo_event_applies_at_the_first_segment_boundary_at_or_after_its_frame() {
    let beats = Beats::Quarter;
    // The event goes in at absolute frame f; blocks are one segment, so f lands inside a call at offset f % 32.
    let run = |f: u64| -> Vec<u64> {
        let mut e = engine(&global_lfo_patch(LfoShape::Sine, beats), SEED);
        let (mut l, mut r) = ([0.0f32; SEG], [0.0f32; SEG]);
        let mut seen = Vec::new();
        for k in 0..6u64 {
            let ev: Vec<Event> = if f / SEG as u64 == k {
                vec![Event {
                    offset: (f % SEG as u64) as u32,
                    kind: tempo(60.0),
                }]
            } else {
                Vec::new()
            };
            e.process(&mut l, &mut r, &ev);
            seen.push(e.tempo_bpm().to_bits());
        }
        seen
    };
    let (old, new) = (DEFAULT_TEMPO_BPM.to_bits(), 60.0f64.to_bits());
    for (f, boundary) in [
        (0u64, 0usize),
        (1, 1),
        (31, 1),
        (32, 1),
        (33, 2),
        (64, 2),
        (65, 3),
    ] {
        let seen = run(f);
        for (k, bits) in seen.iter().enumerate() {
            // Segment k has run when seen[k] is read: the event is applied by then when its boundary <= k.
            assert_eq!(
                *bits == new,
                boundary <= k,
                "tempo event at frame {f} (boundary {boundary}) after call {k}"
            );
            assert!(*bits == new || *bits == old);
        }
    }
}

#[test]
fn the_engine_clamps_a_tempo_ignores_nan_and_a_reset_restores_the_default() {
    let mut e = engine(&reset_saw_patch(), SEED);
    assert_eq!(e.tempo_bpm(), DEFAULT_TEMPO_BPM);
    let set = |e: &mut Engine, bpm: f64| {
        let (mut l, mut r) = ([0.0f32; SEG], [0.0f32; SEG]);
        e.process(
            &mut l,
            &mut r,
            &[Event {
                offset: 0,
                kind: tempo(bpm),
            }],
        );
        e.tempo_bpm()
    };
    assert_eq!(set(&mut e, 128.0), 128.0);
    assert_eq!(set(&mut e, 5.0), MIN_TEMPO_BPM);
    assert_eq!(set(&mut e, 0.0), MIN_TEMPO_BPM);
    assert_eq!(set(&mut e, -90.0), MIN_TEMPO_BPM);
    assert_eq!(set(&mut e, 5000.0), MAX_TEMPO_BPM);
    assert_eq!(set(&mut e, f64::INFINITY), MAX_TEMPO_BPM);
    assert_eq!(set(&mut e, 100.0), 100.0);
    assert_eq!(set(&mut e, f64::NAN), 100.0, "NaN is ignored");
    assert_eq!(set(&mut e, f64::NEG_INFINITY), MIN_TEMPO_BPM);
    e.reset();
    assert_eq!(e.tempo_bpm(), DEFAULT_TEMPO_BPM);
}

#[test]
fn a_reset_engine_renders_like_a_new_one() {
    let patch = global_lfo_patch(LfoShape::Triangle, Beats::Eighth);
    let mut fresh = Rig::new(&patch);
    let reference = fresh.trace(600, |_| vec![]);
    let mut rig = Rig::new(&patch);
    rig.trace(333, |k| if k == 0 { vec![tempo(174.0)] } else { vec![] });
    rig.engine.reset();
    let again = rig.trace(600, |_| vec![]);
    assert_eq!(
        again.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
        reference.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
        "the tempo is back at the default after a reset"
    );
}

/// A voice LFO (`lfo1`, 100 · LFO cents on `osc1.fine`) and a global LFO (`lfo2` on `comp1.mix`), both synced, on
/// the every-kind patch, so every other module is in the render too.
fn synced_patch() -> Patch {
    let mut p = every_kind_patch(400.0);
    for m in &mut p.modulators {
        match m.id.as_str() {
            "lfo1" => {
                m.values[LfoParam::Sync.index()] = 1.0;
                m.values[LfoParam::Beats.index()] = Beats::EighthDotted.index() as f32;
            }
            "lfo2" => {
                m.values[LfoParam::Sync.index()] = 1.0;
                m.values[LfoParam::Beats.index()] = Beats::QuarterTriplet.index() as f32;
                m.values[LfoParam::Shape.index()] = LfoShape::Triangle.index() as f32;
            }
            _ => {}
        }
    }
    route(&mut p, "lfo1", "osc1.fine", 30.0);
    route(&mut p, "lfo2", "comp1.mix", 40.0);
    p
}

/// Notes and tempo events (absolute frames), notes first at equal frames.
fn performance() -> Vec<(u64, EventKind)> {
    let mut ev = common::events(&[
        note(60, 0, 9000),
        note(64, 1234, 20_000),
        note(67, 7777, 30_001),
    ]);
    for (f, bpm) in [
        (1u64, 120.0),
        (5000, 90.0),
        (17_777, 174.0),
        (17_800, f64::NAN),
        (20_000, 5000.0),
        (26_001, 33.3),
    ] {
        ev.push((f, tempo(bpm)));
    }
    ev.sort_by_key(|(f, _)| *f);
    ev
}

#[test]
fn the_host_block_size_does_not_change_a_render_with_a_mid_render_tempo_change() {
    let patch = synced_patch();
    let frames = 36_000;
    let ev = performance();
    let mut reference = engine(&patch, SEED);
    let (rl, rr) = drive(&mut reference, &ev, frames, SEG, |_, _, _| {});
    assert!(rl.iter().any(|x| *x != 0.0), "not silent");
    for block in [1, 2, 7, 31, 33, 64, 100, 441, 1000, 4096, frames] {
        let mut e = engine(&patch, SEED);
        let (l, r) = drive(&mut e, &ev, frames, block, |_, _, _| {});
        assert_eq!(bits(&l), bits(&rl), "left, block {block}");
        assert_eq!(bits(&r), bits(&rr), "right, block {block}");
        assert_eq!(e.tempo_bpm(), reference.tempo_bpm(), "block {block}");
    }
    // The tempo is a real input of a synced patch: without the tempo events the render is another one.
    let plain: Vec<_> = ev
        .iter()
        .copied()
        .filter(|(_, k)| !matches!(k, EventKind::Tempo { .. }))
        .collect();
    let mut e = engine(&patch, SEED);
    let (pl, _) = drive(&mut e, &plain, frames, SEG, |_, _, _| {});
    assert_ne!(bits(&pl), bits(&rl));
}

#[test]
fn an_unsynced_patch_ignores_the_tempo_bit_for_bit() {
    // The same patch with every lfo unsynced: tempo events change nothing (S0d's 90/140/174 test, at engine level).
    let mut patch = synced_patch();
    for m in &mut patch.modulators {
        if m.kind == ModuleKind::Lfo {
            m.values[LfoParam::Sync.index()] = 0.0;
        }
    }
    let frames = 36_000;
    let with = performance();
    let without: Vec<_> = with
        .iter()
        .copied()
        .filter(|(_, k)| !matches!(k, EventKind::Tempo { .. }))
        .collect();
    let render = |ev: &[(u64, EventKind)]| {
        let mut e = engine(&patch, SEED);
        drive(&mut e, ev, frames, 100, |_, _, _| {})
    };
    let (wl, wr) = render(&with);
    let (ol, or) = render(&without);
    assert!(ol.iter().any(|x| *x != 0.0));
    assert_eq!(bits(&wl), bits(&ol));
    assert_eq!(bits(&wr), bits(&or));
}

#[test]
fn the_beats_parameter_steps_through_the_divisions_and_never_between_them() {
    // macro1 moves lfo1.beats: base item 0, depth 100 %, so the item is round(24 · macro).
    let mut p = global_lfo_patch(LfoShape::Sine, Beats::SixtyFourth);
    route(&mut p, "macro1", "lfo1.beats", 100.0);
    let (plan, map) = compile(&p, &PrepareCfg::new(SR)).unwrap();
    let beats_id = map.get(&"lfo1.beats".parse().unwrap()).unwrap();
    let mut e = Engine::new(plan, SEED);
    let (mut l, mut r) = ([0.0f32; SEG], [0.0f32; SEG]);
    let mut seen = Vec::new();
    for k in 0..=1000 {
        let m = k as f32 / 1000.0;
        e.process(
            &mut l,
            &mut r,
            &[Event {
                offset: 0,
                kind: EventKind::Macro { index: 0, value: m },
            }],
        );
        seen.push(e.value(beats_id, 0).unwrap());
    }
    assert!(
        seen.iter().all(|v| v.fract() == 0.0),
        "every value is an item index"
    );
    assert!(
        seen.windows(2).all(|w| w[0] <= w[1]),
        "a rising knob only slows the LFO"
    );
    assert_eq!(seen[0], 0.0);
    assert_eq!(*seen.last().unwrap(), (Beats::COUNT - 1) as f32);
    for item in 0..Beats::COUNT {
        assert!(
            seen.contains(&(item as f32)),
            "item {item} is never reached"
        );
    }

    // Anywhere inside one item's span the LFO runs at that item's speed: macro 0.50 and 0.52 both round to item 12
    // (the half-note triplet) and render the same bits, and they are the closed form of 4/3 beats; 0.54 is item 13.
    let at = |m: f32| -> Vec<f64> {
        let mut p = p.clone();
        p.macros[0].value = m;
        Rig::new(&p).trace(3000, |_| vec![])
    };
    let (a, b, c) = (at(0.50), at(0.52), at(0.54));
    assert_eq!(
        a.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
        b.iter().map(|v| v.to_bits()).collect::<Vec<_>>()
    );
    let items = Beats::ALL;
    let cps = |item: usize| cycles_per_segment(DEFAULT_TEMPO_BPM, items[item]);
    assert_eq!(items[12], Beats::HalfTriplet);
    assert_eq!(items[13], Beats::QuarterDotted);
    for (i, (x, y)) in a.iter().zip(&c).enumerate() {
        assert_close(*x, sine((i as f64 + 1.0) * cps(12)), 2e-5, "item 12");
        assert_close(*y, sine((i as f64 + 1.0) * cps(13)), 2e-5, "item 13");
    }
}

#[test]
fn a_one_shot_runs_one_cycle_then_holds_and_a_voice_lfo_restarts_at_each_note() {
    // Global, saw up (rises from −1 to +1): one cycle of a quarter note at 120 BPM is 750 segments, then it holds.
    let mut p = global_lfo_patch(LfoShape::SawUp, Beats::Quarter);
    p.modulators[1].values[LfoParam::OneShot.index()] = 1.0;
    let mut rig = Rig::new(&p);
    let cycle = 750;
    let trace = rig.trace(cycle + 2000, |k| {
        if k == 0 { vec![tempo(120.0)] } else { vec![] }
    });
    let cps = cycles_per_segment(120.0, Beats::Quarter);
    assert_close(cps * cycle as f64, 1.0, 1e-12, "a cycle is 750 segments");
    for (i, v) in trace.iter().enumerate().take(cycle - 1) {
        assert_close(*v, 2.0 * ((i as f64 + 1.0) * cps) - 1.0, 2e-5, "rising");
    }
    let held = trace[cycle - 1];
    assert!(
        held > 0.999,
        "the cycle ends at the top of the ramp, not back at −1: {held}"
    );
    assert!(
        trace[cycle..].iter().all(|v| v.to_bits() == held.to_bits()),
        "it holds"
    );
    // A tempo change after the cycle does not wake it.
    let mut rig = Rig::new(&p);
    let trace = rig.trace(cycle + 2000, |k| match k {
        0 => vec![tempo(120.0)],
        1500 => vec![tempo(33.0)],
        _ => vec![],
    });
    assert!(trace[cycle..].iter().all(|v| v.to_bits() == held.to_bits()));

    // Per voice: lfo1 (one shot, saw up) on osc1.fine = 100 · value cents; each note starts its own cycle.
    let mut p = reset_saw_patch();
    let mut one = lfo("lfo1", LfoScope::Voice, LfoShape::SawUp, Beats::Quarter);
    one.values[LfoParam::OneShot.index()] = 1.0;
    p.modulators.push(one);
    route(&mut p, "lfo1", "osc1.fine", 50.0);
    let (plan, map) = compile(&p, &PrepareCfg::new(SR)).unwrap();
    let fine = map.get(&"osc1.fine".parse().unwrap()).unwrap();
    let mut e = Engine::new(plan, SEED);
    let (mut l, mut r) = ([0.0f32; SEG], [0.0f32; SEG]);
    let on = |id: i32| EventKind::NoteOn {
        key: NoteKey::new(60 + id as u8, id),
        velocity: 0.8,
    };
    let voice_of = |e: &Engine, id: i32| {
        *e.active_voices()
            .iter()
            .find(|&&v| e.voices().key(v).note_id == id)
            .expect("the note plays")
    };
    let second_note_at = cycle + 400;
    let mut values = |e: &mut Engine, k: usize| -> (Option<f32>, Option<f32>) {
        let ev: Vec<Event> = match k {
            0 => vec![
                Event {
                    offset: 0,
                    kind: tempo(120.0),
                },
                Event {
                    offset: 0,
                    kind: on(1),
                },
            ],
            k if k == second_note_at => vec![Event {
                offset: 0,
                kind: on(2),
            }],
            _ => Vec::new(),
        };
        e.process(&mut l, &mut r, &ev);
        let first = e.value(fine, voice_of(e, 1));
        let second = (k >= second_note_at).then(|| e.value(fine, voice_of(e, 2)).unwrap());
        (first, second)
    };
    let mut first_at_cycle_end = 0.0;
    for k in 0..second_note_at + 300 {
        let (first, second) = values(&mut e, k);
        let first = first.unwrap();
        if k == cycle - 1 {
            first_at_cycle_end = first;
            assert!(
                first > 99.9,
                "the first note's ramp ends at the top: {first}"
            );
        }
        if k >= cycle {
            assert_eq!(
                first, first_at_cycle_end,
                "the first note holds at segment {k}"
            );
        }
        if let Some(second) = second {
            // The second note starts its own ramp from the bottom while the first still holds the top.
            let n = k - second_note_at + 1;
            let want = 100.0 * (2.0 * (n as f64 * cps) - 1.0);
            assert_close(f64::from(second), want, 1e-2, "the second note's ramp");
        }
    }
}

#[test]
fn the_tempo_survives_a_plan_swap_and_a_synced_lfo_keeps_its_phase_across_it() {
    let patch = synced_patch();
    let frames_segments = 900;
    let run = |swap: bool| -> (Vec<f32>, Engine) {
        let (plan, _) = compile(&patch, &PrepareCfg::new(SR)).unwrap();
        let (mut e, mut link) = Engine::with_link(plan, SEED, LinkCapacity::default());
        let mut out = Vec::new();
        let (mut l, mut r) = ([0.0f32; SEG], [0.0f32; SEG]);
        for k in 0..frames_segments {
            if swap && k == 300 {
                let (plan, _) = compile(&patch, &PrepareCfg::new(SR)).unwrap();
                link.send_plan(plan).expect("room for a plan");
            }
            link.collect();
            let ev: Vec<Event> = match k {
                0 => vec![
                    Event {
                        offset: 0,
                        kind: tempo(174.0),
                    },
                    Event {
                        offset: 0,
                        kind: EventKind::NoteOn {
                            key: NoteKey::new(60, 0),
                            velocity: 0.8,
                        },
                    },
                ],
                _ => Vec::new(),
            };
            e.process(&mut l, &mut r, &ev);
            out.extend_from_slice(&l);
        }
        (out, e)
    };
    let (plain, _) = run(false);
    let (swapped, e) = run(true);
    assert_eq!(e.stats().swaps, 1);
    assert_eq!(
        e.tempo_bpm(),
        174.0,
        "the tempo is the engine's, not a plan's"
    );
    assert!(plain.iter().any(|x| *x != 0.0));
    assert_eq!(bits(&swapped), bits(&plain));
}
