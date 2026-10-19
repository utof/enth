//! The plan swap (lane S2b): state carried by module id and kind, crossfades only for added or removed modules,
//! epoch-tagged value events, the rings' overflow paths under `assert_no_alloc` (a full command ring, a swap while
//! the return ring is full), at most two plans in flight, and the monitor bypass.
#![cfg(debug_assertions)]

mod common;

use assert_no_alloc::{AllocDisabler, assert_no_alloc};
use common::{SEED, SR, bits, every_kind_patch, id, reset_saw_patch, saw_patch};
use enth_engine::module::{
    GlobalCx, GlobalModCx, GlobalParams, GlobalStart, MAX_VOICES, NoteState, PrepareCfg, SEG,
    VoiceBus, VoiceCx, VoiceModCx, VoiceParams, VoiceStart,
};
use enth_engine::modules::ModuleImpl;
use enth_engine::{
    Engine, Event, EventKind, Link, LinkCapacity, NoteKey, Plan, PlanMap, SendError, compile,
};
use enth_patch::catalogue::{LfoParam, LfoScope, OscParam};
use enth_patch::{ModuleInstance, ModuleKind, ParamAddr, Patch};

#[global_allocator]
static ALLOCATOR: AllocDisabler = AllocDisabler;

fn plan(patch: &Patch) -> (Plan, PlanMap) {
    compile(patch, &PrepareCfg::new(SR)).expect("the patch compiles")
}

fn linked(patch: &Patch, capacity: LinkCapacity) -> (Engine, Link) {
    Engine::with_link(plan(patch).0, SEED, capacity)
}

fn on(key: u8, id: i32) -> EventKind {
    EventKind::NoteOn {
        key: NoteKey::new(key, id),
        velocity: 0.8,
    }
}

fn off(key: u8, id: i32) -> EventKind {
    EventKind::NoteOff {
        key: NoteKey::new(key, id),
        velocity: 0.0,
    }
}

/// Run `segments` segments, one 32-frame call each, under `assert_no_alloc`. `before(k, engine, link)` runs before
/// segment `k` (it may send commands; it runs outside the guard) and returns that segment's events.
fn run(
    engine: &mut Engine,
    link: &mut Link,
    segments: usize,
    mut before: impl FnMut(usize, &Engine, &mut Link) -> Vec<EventKind>,
) -> (Vec<f32>, Vec<f32>) {
    let mut left = vec![0.0; segments * SEG];
    let mut right = vec![0.0; segments * SEG];
    for k in 0..segments {
        let events: Vec<Event> = before(k, engine, link)
            .into_iter()
            .map(|kind| Event { offset: 0, kind })
            .collect();
        let (l, r) = (
            &mut left[k * SEG..(k + 1) * SEG],
            &mut right[k * SEG..(k + 1) * SEG],
        );
        assert_no_alloc(|| engine.process(l, r, &events));
    }
    (left, right)
}

/// A chord held from segment 0, released at segment 400; `edit` is swapped in at segment 200 when given. Returns
/// the output and the engine.
fn held_chord_with_swap(base: &Patch, edit: Option<&Patch>) -> (Vec<f32>, Vec<f32>, Engine) {
    let (mut e, mut link) = linked(base, LinkCapacity::default());
    let edit = edit.map(|p| plan(p).0);
    let mut edit = edit;
    let (l, r) = run(&mut e, &mut link, 900, |k, _, link| {
        if k == 200
            && let Some(p) = edit.take()
        {
            link.send_plan(p).expect("room for a plan");
        }
        link.collect();
        match k {
            0 => vec![on(48, 0), on(55, 1), on(60, 2)],
            100 => vec![on(64, 3)],
            400 => vec![off(48, 0), off(55, 1), off(60, 2), off(64, 3)],
            _ => Vec::new(),
        }
    });
    (l, r, e)
}

#[test]
fn the_engine_and_its_link_move_to_their_threads() {
    fn send<T: Send>() {}
    send::<Engine>();
    send::<Link>();
    send::<Plan>();
}

#[test]
fn a_held_note_survives_recompiling_the_same_patch_bit_for_bit() {
    for base in [saw_patch(), every_kind_patch(800.0)] {
        let (l0, r0, _) = held_chord_with_swap(&base, None);
        let (l1, r1, e) = held_chord_with_swap(&base, Some(&base));
        assert!(l0.iter().any(|x| *x != 0.0));
        assert_eq!(e.stats().swaps, 1);
        assert_eq!(
            e.stats().crossfades,
            0,
            "nothing added or removed: no crossfade"
        );
        assert_eq!(e.epoch(), 1);
        assert_eq!(bits(&l1), bits(&l0));
        assert_eq!(bits(&r1), bits(&r0));
    }
}

#[test]
fn a_held_note_survives_an_unrelated_structural_edit() {
    let base = every_kind_patch(800.0);
    let (l0, r0, _) = held_chord_with_swap(&base, None);
    // Added: a voice LFO and an oscillator that is off (level 0). Removed: the global LFO.
    let mut added = base.clone();
    added
        .modulators
        .push(ModuleInstance::new(id("lfo3"), ModuleKind::Lfo));
    let mut removed = base.clone();
    removed.modulators.retain(|m| m.id.as_str() != "lfo2");
    let mut silent_osc = base.clone();
    let mut osc3 = ModuleInstance::new(id("osc3"), ModuleKind::Osc);
    osc3.values[OscParam::Level.index()] = 0.0;
    silent_osc.generators.push(osc3);
    for (name, edit) in [
        ("added lfo", added),
        ("removed lfo", removed),
        ("silent osc", silent_osc),
    ] {
        let (l1, r1, e) = held_chord_with_swap(&base, Some(&edit));
        assert_eq!(e.stats().crossfades, 1, "{name}: a module came or went");
        // Both plans give the same samples, and `old + (new − old)·w` is exact where they agree.
        assert_eq!(bits(&l1), bits(&l0), "{name}");
        assert_eq!(bits(&r1), bits(&r0), "{name}");
        assert!(!e.is_crossfading());
        assert_eq!(e.plans_held(), 1, "{name}: the old plan went back");
    }
}

#[test]
fn an_added_generator_fades_in_over_256_frames() {
    let base = reset_saw_patch();
    let mut edit = base.clone();
    let mut osc2 = ModuleInstance::new(id("osc2"), ModuleKind::Osc);
    osc2.values[OscParam::Semi.index()] = 7.0;
    edit.generators.push(osc2);
    let (l0, _, _) = held_chord_with_swap(&base, None);
    let (l1, _, e) = held_chord_with_swap(&base, Some(&edit));
    let swap = 200 * SEG;
    assert_eq!(
        bits(&l1[..swap]),
        bits(&l0[..swap]),
        "nothing changes before"
    );
    assert_ne!(bits(&l1[swap..swap + 256]), bits(&l0[swap..swap + 256]));
    assert_eq!(e.stats().crossfades, 1);
    // The fade starts at the old plan's output: the first frame differs from it by at most 1/256 of the new osc.
    assert!(
        (l1[swap] - l0[swap]).abs() < 0.02,
        "{} vs {}",
        l1[swap],
        l0[swap]
    );
    assert!(l1.iter().all(|x| x.is_finite()));
}

#[test]
fn stale_epoch_events_are_ignored() {
    let base = saw_patch();
    let level = |map: &PlanMap| {
        map.get(&"osc1.level".parse::<ParamAddr>().unwrap())
            .unwrap()
    };
    // A note held; plan 1 (the same patch) swaps in at segment 10; at segment 20 a value event arrives tagged with
    // the old epoch (stale) or the new one.
    let render = |stale: bool, fresh: bool| {
        let (p0, map0) = plan(&base);
        let (p1, map1) = plan(&base);
        let (mut e, mut link) = Engine::with_link(p0, SEED, LinkCapacity::default());
        let mut p1 = Some(p1);
        let mut epoch = 0;
        let (l, _) = run(&mut e, &mut link, 100, |k, _, link| {
            if k == 10 {
                epoch = link.send_plan(p1.take().unwrap()).unwrap();
            }
            if k == 20 && stale {
                link.send_param(0, level(&map0), 10.0).unwrap();
            }
            if k == 20 && fresh {
                link.send_param(epoch, level(&map1), 10.0).unwrap();
            }
            link.collect();
            if k == 0 { vec![on(60, 0)] } else { Vec::new() }
        });
        (l, e.stats())
    };
    let (plain, _) = render(false, false);
    let (stale, s) = render(true, false);
    assert_eq!(s.stale_events, 1);
    assert_eq!(
        bits(&stale),
        bits(&plain),
        "the stale event changed nothing"
    );
    let (fresh, s) = render(false, true);
    assert_eq!(s.stale_events, 0);
    assert_ne!(
        bits(&fresh),
        bits(&plain),
        "the current-epoch event applies"
    );
}

#[test]
fn a_full_command_ring_drains_without_allocating() {
    let base = every_kind_patch(400.0);
    let (_, map) = plan(&base);
    let cutoff = map.get(&"filter1.cutoff".parse().unwrap()).unwrap();
    let capacity = LinkCapacity {
        commands: 64,
        returns: 4,
    };
    let (mut e, mut link) = linked(&base, capacity);
    let mut next = Some(plan(&base).0);
    run(&mut e, &mut link, 50, |k, _, link| {
        if k == 5 {
            // Fill every slot: a plan in the middle, value events and macros around it.
            let mut sent = 0;
            loop {
                let r = if sent == 30 {
                    let p = next.take().unwrap();
                    link.send_plan(p).map(|_| ()).map_err(|e| match e {
                        SendError::Full(_) => SendError::Full(()),
                        SendError::InFlight(_) => SendError::InFlight(()),
                    })
                } else if sent % 3 == 0 {
                    link.send_macro(sent % 8, 0.5)
                } else {
                    let epoch = u64::from(sent > 30);
                    link.send_param(epoch, cutoff, 500.0 + sent as f32)
                };
                match r {
                    Ok(()) => sent += 1,
                    Err(SendError::Full(())) => break,
                    Err(SendError::InFlight(())) => panic!("one plan only"),
                }
            }
            assert_eq!(sent, 64);
            assert_eq!(link.free_slots(), 0);
            assert!(link.send_mod_wheel(1.0).is_err(), "full");
        }
        link.collect();
        if k == 0 {
            vec![on(60, 0), on(64, 1)]
        } else {
            Vec::new()
        }
    });
    assert_eq!(e.stats().swaps, 1);
    assert_eq!(e.stats().stale_events, 0);
    assert_eq!(link.free_slots(), 64, "drained in one segment");
}

#[test]
fn a_swap_while_the_return_ring_is_full_keeps_the_plan() {
    let base = every_kind_patch(400.0);
    let capacity = LinkCapacity {
        commands: 16,
        returns: 1,
    };
    let (mut e, mut link) = linked(&base, capacity);
    let mut plans: Vec<Plan> = (0..3).map(|_| plan(&base).0).collect();
    let mut collected = 0;
    let mut held = Vec::new();
    run(&mut e, &mut link, 60, |k, e, link| {
        held.push(e.plans_held());
        match k {
            // Swap 1 returns plan 0 into the ring (now full); swap 2 finds it full: plan 1 waits in the slot.
            10 | 11 => {
                link.send_plan(plans.pop().unwrap()).unwrap();
            }
            12 => {
                let third = plans.pop().unwrap();
                assert!(
                    matches!(link.send_plan(third), Err(SendError::InFlight(_))),
                    "at most two plans in flight"
                );
            }
            40 | 41 => collected += link.collect(),
            _ => {}
        }
        if k == 0 { vec![on(60, 0)] } else { Vec::new() }
    });
    assert_eq!(e.stats().swaps, 2);
    assert_eq!(held[12], 2, "the replaced plan is kept, not dropped");
    assert_eq!(held[40], 2, "still kept while the ring is full");
    assert_eq!(held[41], 1, "handed back once there was room");
    assert_eq!(
        collected, 2,
        "both replaced plans came back to the main thread"
    );
    assert_eq!(link.in_flight(), 0);
    assert!(e.active_voices().len() == 1, "the note kept playing");
}

#[test]
fn a_plan_that_arrives_during_a_crossfade_waits_for_it() {
    // A plan that arrives during a crossfade waits in the ring (with any command behind it) until the fade is over.
    let base = saw_patch();
    let mut edit = base.clone();
    edit.modulators
        .push(ModuleInstance::new(id("lfo3"), ModuleKind::Lfo));
    let (mut e, mut link) = linked(&base, LinkCapacity::default());
    let mut a = Some(plan(&edit).0);
    let mut b = Some(plan(&base).0);
    let mut epochs = Vec::new();
    run(&mut e, &mut link, 40, |k, e, link| {
        epochs.push(e.epoch());
        if k == 5 {
            link.send_plan(a.take().unwrap()).unwrap();
            link.send_plan(b.take().unwrap()).unwrap();
        }
        link.collect();
        Vec::new()
    });
    // Plan 1 (an added LFO) crossfades for 8 segments from segment 5; plan 2 (removes it) waits for that.
    assert_eq!(epochs[6], 1);
    assert_eq!(epochs[13], 1);
    assert_eq!(epochs[14], 2);
    assert_eq!(e.stats().crossfades, 2);
}

#[test]
fn the_monitor_bypass_mutes_time_based_effects_only() {
    let mut with_fx = saw_patch();
    with_fx
        .fx
        .push(ModuleInstance::new(id("comp1"), ModuleKind::Comp));
    with_fx
        .fx
        .push(ModuleInstance::new(id("reverb1"), ModuleKind::Reverb));
    let mut no_reverb = with_fx.clone();
    no_reverb.fx.pop();
    let render = |patch: &Patch, bypass: bool| {
        let (mut e, mut link) = linked(patch, LinkCapacity::default());
        e.set_monitor_bypass(bypass);
        assert_eq!(e.monitor_bypass(), bypass);
        run(&mut e, &mut link, 300, |k, _, _| match k {
            0 => vec![on(60, 0), on(67, 1)],
            100 => vec![off(60, 0), off(67, 1)],
            _ => Vec::new(),
        })
        .0
    };
    // Bypassed, the reverb is not there at all; the compressor (not time-based) still runs.
    assert_eq!(
        bits(&render(&with_fx, true)),
        bits(&render(&no_reverb, false))
    );
    assert_eq!(
        bits(&render(&no_reverb, true)),
        bits(&render(&no_reverb, false))
    );
}

// ---------------------------------------------------------------------------------------------------------------
// The monitor bypass fade (P1b lane E+, ADR-0006): 20 ms out, then the identity; 20 ms back in from silence.
// ---------------------------------------------------------------------------------------------------------------

/// The saw, a compressor, then the reverb (last, so bypassing it leaves everything before it alone).
fn fx_patches() -> (Patch, Patch) {
    let mut with_fx = saw_patch();
    with_fx
        .fx
        .push(ModuleInstance::new(id("comp1"), ModuleKind::Comp));
    with_fx
        .fx
        .push(ModuleInstance::new(id("reverb1"), ModuleKind::Reverb));
    let mut no_reverb = with_fx.clone();
    no_reverb.fx.pop();
    (with_fx, no_reverb)
}

/// Segments the bypass fade lasts at the test rate (30 at 48 kHz).
const FADE: usize = 30;

/// A held chord from segment 0 to 400, 500 segments; `before(k, engine, link)` runs before segment `k` and may flip
/// the bypass or send a plan. Returns the output and, per segment, whether the fade was running after it.
fn bypass_run(
    patch: &Patch,
    mut before: impl FnMut(usize, &mut Engine, &mut Link),
) -> (Vec<f32>, Vec<bool>) {
    let (mut e, mut link) = linked(patch, LinkCapacity::default());
    let mut left = vec![0.0; 500 * SEG];
    let mut fading = Vec::new();
    for k in 0..500 {
        before(k, &mut e, &mut link);
        let events: Vec<Event> = match k {
            0 => vec![on(57, 0), on(64, 1)],
            400 => vec![off(57, 0), off(64, 1)],
            _ => Vec::new(),
        }
        .into_iter()
        .map(|kind| Event { offset: 0, kind })
        .collect();
        let mut right = [0.0; SEG];
        let l = &mut left[k * SEG..(k + 1) * SEG];
        assert_no_alloc(|| e.process(l, &mut right, &events));
        fading.push(e.is_bypass_fading());
        link.collect();
    }
    (left, fading)
}

#[test]
fn engaging_the_monitor_bypass_fades_the_reverb_out_over_20_ms() {
    assert_eq!(
        (f64::from(enth_engine::MONITOR_BYPASS_FADE_MS) * f64::from(SR) / 1000.0 / SEG as f64)
            .round(),
        FADE as f64
    );
    let (with_fx, no_reverb) = fx_patches();
    let (plain, _) = bypass_run(&with_fx, |_, _, _| {});
    let (dry, _) = bypass_run(&no_reverb, |_, _, _| {});
    let (out, fading) = bypass_run(&with_fx, |k, e, _| {
        if k == 100 {
            e.set_monitor_bypass(true);
        }
    });
    let (at, settled) = (100 * SEG, (100 + FADE) * SEG);
    assert_eq!(bits(&out[..at]), bits(&plain[..at]), "untouched before");
    assert!(
        fading[100..100 + FADE - 1].iter().all(|f| *f),
        "fading for 20 ms"
    );
    assert!(!fading[100 + FADE - 1] && !fading[99]);
    // A fade, not a switch: in between the reverb and the dry signal.
    assert_ne!(bits(&out[at..settled]), bits(&plain[at..settled]));
    assert_ne!(bits(&out[at..settled]), bits(&dry[at..settled]));
    // ADR-0006's test: once the fade has settled, the same output as the patch without the reverb.
    assert_eq!(bits(&out[settled..]), bits(&dry[settled..]));
}

#[test]
fn releasing_the_monitor_bypass_fades_a_fresh_reverb_in_like_adding_it() {
    let (with_fx, no_reverb) = fx_patches();
    // Bypassed from the start (no fade before the first segment), released at segment 200.
    let (out, fading) = bypass_run(&with_fx, |k, e, _| match k {
        0 => e.set_monitor_bypass(true),
        200 => e.set_monitor_bypass(false),
        _ => {}
    });
    let (dry, _) = bypass_run(&no_reverb, |_, _, _| {});
    assert_eq!(
        bits(&out[..200 * SEG]),
        bits(&dry[..200 * SEG]),
        "no reverb while bypassed"
    );
    assert!(fading[200..200 + FADE - 1].iter().all(|f| *f));
    // The reference: the reverb added by a plan swap at segment 200 (it starts fresh too, and crossfades in).
    let mut added = Some(plan(&with_fx).0);
    let (swapped, _) = bypass_run(&no_reverb, |k, _, link| {
        if k == 200 {
            link.send_plan(added.take().unwrap()).unwrap();
        }
    });
    let settled = (200 + FADE) * SEG;
    assert_ne!(
        bits(&out[200 * SEG..settled]),
        bits(&swapped[200 * SEG..settled])
    );
    assert_eq!(bits(&out[settled..]), bits(&swapped[settled..]));
}

#[test]
fn reversing_the_monitor_bypass_mid_fade_turns_the_fade_around_without_a_reset() {
    let (with_fx, _) = fx_patches();
    let (plain, _) = bypass_run(&with_fx, |_, _, _| {});
    let (out, fading) = bypass_run(&with_fx, |k, e, _| match k {
        100 => e.set_monitor_bypass(true),
        110 => e.set_monitor_bypass(false),
        _ => {}
    });
    // Ten segments down, ten back up; the reverb kept running, so afterwards nothing remains of it.
    assert!(fading[100..119].iter().all(|f| *f));
    assert!(!fading[119]);
    assert_ne!(
        bits(&out[100 * SEG..120 * SEG]),
        bits(&plain[100 * SEG..120 * SEG])
    );
    assert_eq!(bits(&out[120 * SEG..]), bits(&plain[120 * SEG..]));
}

// ---------------------------------------------------------------------------------------------------------------
// For every ModuleKind: copy the state of A into a fresh B, then both give the same bits.
// ---------------------------------------------------------------------------------------------------------------

/// Every kind, both LFO scopes, with non-default values so the state is not trivial.
fn kinds() -> Vec<(ModuleKind, Vec<f32>)> {
    let mut out = Vec::new();
    for &kind in ModuleKind::ALL {
        let values = kind.defaults();
        if kind == ModuleKind::Lfo {
            let mut global = values.clone();
            global[LfoParam::Scope.index()] = LfoScope::Global.index() as f32;
            out.push((kind, global));
        }
        out.push((kind, values));
    }
    out
}

/// A deterministic test signal.
fn signal(k: usize, i: usize, v: usize) -> f32 {
    let t = (k * SEG + i) as f32;
    0.5 * enth_dsp::util::math::sinf(t * (0.01 + 0.003 * v as f32)) + 0.1 * ((k + v) % 3) as f32
}

/// One segment of a module; its output (every active voice's samples or modulator values, or the stereo bus).
fn step(m: &mut ModuleImpl, values: &[f32], k: usize, notes: &NoteState) -> Vec<f32> {
    const VOICES: [usize; 2] = [0, 5];
    let rows: Vec<[f32; MAX_VOICES]> = values.iter().map(|&x| [x; MAX_VOICES]).collect();
    let vp = VoiceParams::from_slices(values, &rows, &rows);
    let gp = GlobalParams::from_slices(values, values, values);
    match m {
        ModuleImpl::VoiceProcessor(p) => {
            let mut io: Box<VoiceBus> = Box::new([[0.0; SEG]; MAX_VOICES]);
            for &v in &VOICES {
                for i in 0..SEG {
                    io[v][i] = signal(k, i, v);
                }
            }
            p.process(&mut VoiceCx {
                voices: &VOICES,
                notes,
                params: vp,
                io: &mut io,
                sync_out: None,
                sync_in: None,
            });
            VOICES.iter().flat_map(|&v| io[v]).collect()
        }
        ModuleImpl::VoiceModulator(p) => {
            let mut out = [0.0; MAX_VOICES];
            p.tick(&mut VoiceModCx {
                voices: &VOICES,
                notes,
                params: vp,
                out: &mut out,
            });
            vec![out[0], out[5]]
        }
        ModuleImpl::GlobalModulator(p) => vec![p.tick(&mut GlobalModCx { params: gp })],
        ModuleImpl::GlobalProcessor(p) => {
            let mut left: [f32; SEG] = core::array::from_fn(|i| signal(k, i, 1));
            let mut right: [f32; SEG] = core::array::from_fn(|i| signal(k, i, 2));
            p.process(&mut GlobalCx {
                params: gp,
                left: &mut left,
                right: &mut right,
            });
            left.iter().chain(&right).copied().collect()
        }
    }
}

fn start(m: &mut ModuleImpl, values: &[f32], notes: &NoteState) {
    let rows: Vec<[f32; MAX_VOICES]> = values.iter().map(|&x| [x; MAX_VOICES]).collect();
    let vp = VoiceParams::from_slices(values, &rows, &rows);
    let gp = GlobalParams::from_slices(values, values, values);
    for v in [0, 5] {
        let s = VoiceStart {
            voice: v,
            seed: 0x1234 + v as u64,
            module_key: 99,
            note_index: v as u64,
            notes,
            params: vp,
        };
        match m {
            ModuleImpl::VoiceProcessor(p) => p.start_voice(&s),
            ModuleImpl::VoiceModulator(p) => p.start_voice(&s),
            ModuleImpl::GlobalModulator(_) | ModuleImpl::GlobalProcessor(_) => {}
        }
    }
    let g = GlobalStart {
        seed: 0x5678,
        module_key: 99,
        params: gp,
    };
    match m {
        ModuleImpl::GlobalModulator(p) => p.reset(&g),
        ModuleImpl::GlobalProcessor(p) => p.reset(&g),
        ModuleImpl::VoiceProcessor(_) | ModuleImpl::VoiceModulator(_) => {}
    }
}

#[test]
fn for_every_kind_copying_state_into_a_fresh_module_gives_identical_output() {
    every_kind_carries_state(false);
}

/// The instant swap's move (P1b lane E+): taking the state out of A (which never runs again) leaves B exactly where
/// a copy would. Compared against A's continued output rendered from a copy taken first.
#[test]
fn for_every_kind_taking_state_into_a_fresh_module_gives_identical_output() {
    every_kind_carries_state(true);
}

fn every_kind_carries_state(take: bool) {
    let cfg = PrepareCfg::new(SR);
    for (kind, values) in kinds() {
        let mut notes = NoteState::new();
        for (v, key) in [(0, 57.0), (5, 64.0)] {
            notes.pitch_prev[v] = key;
            notes.pitch_cur[v] = key;
            notes.velocity[v] = 0.7;
            notes.gate[v] = true;
            notes.trigger[v] = true;
            notes.fresh[v] = true;
        }
        let mut a = ModuleImpl::new(kind, &values, &cfg);
        start(&mut a, &values, &notes);
        for k in 0..40 {
            step(&mut a, &values, k, &notes);
            notes.trigger = [false; MAX_VOICES];
            notes.fresh = [false; MAX_VOICES];
        }
        let mut b = ModuleImpl::new(kind, &values, &cfg);
        if take {
            // A runs no more once its state is taken: the copy stands in for its continued output.
            let mut copy = ModuleImpl::new(kind, &values, &cfg);
            assert!(copy.copy_state_from(&a), "{kind}");
            assert!(b.take_state_from(&mut a), "{kind}");
            a = copy;
        } else {
            assert!(b.copy_state_from(&a), "{kind}");
        }
        for k in 40..400 {
            if k == 100 {
                notes.gate[0] = false;
                notes.release[0] = true;
            }
            let (ya, yb) = (
                step(&mut a, &values, k, &notes),
                step(&mut b, &values, k, &notes),
            );
            assert_eq!(bits(&ya), bits(&yb), "{kind}, segment {k}");
            notes.release = [false; MAX_VOICES];
        }
    }
}
