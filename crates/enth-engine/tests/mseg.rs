//! The `mseg` module through the engine (S2a lane MSEG): the three modes with a held and a released note, per voice
//! and global; the synced length exact at every host block size across tempo changes; the curves; a 60 s envelope at a
//! coarse resolution; the global restart rule (F1m S7); mono retrigger and legato; the edit glide; and the module
//! contract (separate state, host-block independence, a plan swap, no allocation, a 40 s tail with no subnormals,
//! FTZ-independent bits).
//!
//! The MSEG's value is read back through a route: a voice MSEG drives `osc1.fine` (`fine = 100 · value` cents, base 0,
//! depth 50 %) and a global one `comp1.mix` (`mix = 50 + 50 · value`), so `Engine::value` after each 32-frame call is
//! the module's value at the end of that segment, to about 1e-6. The expected values come from a model written here
//! from the documented shape, with no code shared with the DSP core. The module's own unit tests
//! (`src/modules/mseg.rs`) check the f32 output bit for bit.
#![allow(
    unsafe_code,
    reason = "the FTZ test emulates a host that sets FTZ/DAZ by writing MXCSR/FPCR directly"
)]

mod common;

use common::{SEED, SR, bits, drive, events, id, note, reset_saw_patch};
use enth_engine::module::count_subnormals;
use enth_engine::{Engine, Event, EventKind, NoteKey, ParamId, PlanMap, PrepareCfg, SEG, compile};
use enth_patch::catalogue::{
    AdsrParam, Beats, CompParam, MSEG_POINT_BASE, MsegMode, MsegParam, MsegScope, OscParam,
    Polarity, SvfParam, VoiceMode, VoiceParam,
};
use enth_patch::{ModuleInstance, ModuleKind, Patch, Route};

// ---- the model, from the documented shape ----

/// `(time %, level %, curve %)`.
type Pt = (f32, f32, f32);

fn bend(c: f64, u: f64) -> f64 {
    if c >= 1.0 {
        // Reaching a point's time passes it: a jump leaving the point has been made.
        return 1.0;
    }
    if c <= -1.0 {
        return 0.0;
    }
    let k = (1.0 - c) / (1.0 + c);
    u / (u + k * (1.0 - u))
}

/// The shape at the share `x` of the length: points never cross, before point 1 it holds point 1's level, after the
/// last it holds the last level (a loop's last segment heads back to point 1's level and ends there at `x = 1`).
fn model(points: &[Pt], looped: bool, x: f64) -> f64 {
    let n = points.len();
    let mut t = Vec::new();
    let mut latest = 0.0f64;
    for p in points {
        latest = latest.max(f64::from(p.0) / 100.0);
        t.push(latest);
    }
    let level = |i: usize| f64::from(points[i].1) / 100.0;
    let curve = |i: usize| f64::from(points[i].2) / 100.0;
    if x < t[0] {
        return level(0);
    }
    let i = (0..n).rev().find(|&i| t[i] <= x).unwrap();
    let mix = |from: f64, to: f64, f: f64| from * (1.0 - f) + to * f;
    if i + 1 < n {
        let u = (x - t[i]) / (t[i + 1] - t[i]);
        mix(level(i), level(i + 1), bend(curve(i), u))
    } else if looped {
        let u = (x - t[i]) / (1.0 - t[i]);
        mix(level(i), level(0), bend(curve(i), u))
    } else {
        level(i)
    }
}

// ---- patches ----

fn route(p: &mut Patch, src: &str, dst: &str, depth: f32) {
    p.routes.push(Route {
        src: src.parse().unwrap(),
        dst: dst.parse().unwrap(),
        depth,
    });
}

/// An MSEG: sync on at `beats`, the given mode, scope and points; `set` overrides any parameter after that.
fn mseg(
    name: &str,
    scope: MsegScope,
    mode: MsegMode,
    beats: Beats,
    points: &[Pt],
    set: &[(MsegParam, f32)],
) -> ModuleInstance {
    let mut m = ModuleInstance::new(id(name), ModuleKind::Mseg);
    let v = &mut m.values;
    v[MsegParam::Scope.index()] = scope.index() as f32;
    v[MsegParam::Mode.index()] = mode.index() as f32;
    v[MsegParam::Sync.index()] = 1.0;
    v[MsegParam::Beats.index()] = beats.index() as f32;
    v[MsegParam::Points.index()] = points.len() as f32;
    for (i, &(t, l, c)) in points.iter().enumerate() {
        let base = MSEG_POINT_BASE + 3 * i;
        v[base] = t;
        v[base + 1] = l;
        v[base + 2] = c;
    }
    for &(p, x) in set {
        v[p.index()] = x;
    }
    m
}

/// A voice MSEG drives `osc1.fine`; the amp envelope releases slowly so a released note outlives the traces.
fn voice_patch(m: ModuleInstance) -> Patch {
    let mut p = reset_saw_patch();
    p.modulators[0].values[AdsrParam::Release.index()] = 8_000.0;
    let name = m.id.as_str().to_string();
    p.modulators.push(m);
    route(&mut p, &name, "osc1.fine", 50.0);
    p
}

/// A global MSEG drives `comp1.mix`.
fn global_patch(m: ModuleInstance) -> Patch {
    let mut p = reset_saw_patch();
    let mut comp = ModuleInstance::new(id("comp1"), ModuleKind::Comp);
    comp.values[CompParam::Mix.index()] = 50.0;
    p.fx.push(comp);
    let name = m.id.as_str().to_string();
    p.modulators.push(m);
    route(&mut p, &name, "comp1.mix", 50.0);
    p
}

fn on(key: u8, id: i32, velocity: f32) -> EventKind {
    EventKind::NoteOn {
        key: NoteKey::new(key, id),
        velocity,
    }
}

fn off(key: u8, id: i32) -> EventKind {
    EventKind::NoteOff {
        key: NoteKey::new(key, id),
        velocity: 0.0,
    }
}

fn tempo(bpm: f64) -> EventKind {
    EventKind::Tempo { bpm }
}

fn compiled(patch: &Patch) -> (Engine, PlanMap) {
    let (plan, map) = compile(patch, &PrepareCfg::new(SR)).expect("the patch compiles");
    (Engine::new(plan, SEED), map)
}

fn param(map: &PlanMap, addr: &str) -> ParamId {
    map.get(&addr.parse().unwrap()).expect("a parameter")
}

/// One 32-frame call at a time, reading the MSEG's value after each.
struct Rig {
    engine: Engine,
    map: PlanMap,
    out: ParamId,
    global: bool,
}

impl Rig {
    fn new(patch: &Patch) -> Rig {
        let global = patch.fx.iter().any(|m| m.id.as_str() == "comp1");
        let (engine, map) = compiled(patch);
        let out = param(&map, if global { "comp1.mix" } else { "osc1.fine" });
        Rig {
            engine,
            map,
            out,
            global,
        }
    }

    /// Run one segment with `events` at offset 0; the value (-1..1 or 0..1) per note id (a global one is note 0).
    fn segment(&mut self, events: &[EventKind]) -> Vec<(i32, f64)> {
        let ev: Vec<Event> = events
            .iter()
            .map(|&kind| Event { offset: 0, kind })
            .collect();
        let (mut l, mut r) = ([0.0f32; SEG], [0.0f32; SEG]);
        self.engine.process(&mut l, &mut r, &ev);
        if self.global {
            let mix = f64::from(self.engine.value(self.out, 0).unwrap());
            return vec![(0, (mix - 50.0) / 50.0)];
        }
        self.engine
            .active_voices()
            .iter()
            .map(|&v| {
                let fine = f64::from(self.engine.value(self.out, v).unwrap());
                (self.engine.voices().key(v).note_id, fine / 100.0)
            })
            .collect()
    }

    /// `segments` segments; `at(k)` gives the events of segment `k`. Every segment's values.
    fn run(
        &mut self,
        segments: usize,
        mut at: impl FnMut(usize) -> Vec<EventKind>,
    ) -> Vec<Vec<(i32, f64)>> {
        (0..segments).map(|k| self.segment(&at(k))).collect()
    }

    /// The same for a patch with one voice (or the global MSEG): that value per segment, NaN before a note plays.
    fn one(&mut self, segments: usize, at: impl FnMut(usize) -> Vec<EventKind>) -> Vec<f64> {
        self.run(segments, at)
            .into_iter()
            .map(|v| v.first().map_or(f64::NAN, |x| x.1))
            .collect()
    }
}

fn close(got: f64, want: f64, tol: f64, what: &str) {
    assert!((got - want).abs() <= tol, "{what}: got {got}, want {want}");
}

/// A quarter note at 120 BPM: 750 segments.
const N: usize = 750;
const TOL: f64 = 3e-6;

fn at_start(k: usize) -> Vec<EventKind> {
    if k == 0 {
        vec![tempo(120.0), on(60, 1, 0.8)]
    } else {
        Vec::new()
    }
}

// ---- the three modes, with a held and a released note ----

#[test]
fn loop_mode_repeats_the_shape_with_the_note_held_or_released() {
    // The last point sits at 90 % with a +100 % curve: the tail jumps at once to point 1's level.
    let pts: [Pt; 5] = [
        (0.0, 0.0, 0.0),
        (10.0, 100.0, 50.0),
        (40.0, 30.0, -60.0),
        (70.0, 80.0, 0.0),
        (90.0, 10.0, 100.0),
    ];
    let mut traces = Vec::new();
    for release_at in [None, Some(1200usize)] {
        let patch = voice_patch(mseg(
            "mseg1",
            MsegScope::Voice,
            MsegMode::Loop,
            Beats::Quarter,
            &pts,
            &[],
        ));
        let trace = Rig::new(&patch).one(2400, |k| {
            let mut ev = at_start(k);
            if Some(k) == release_at {
                ev.push(off(60, 1));
            }
            ev
        });
        for (k, y) in trace.iter().enumerate() {
            let x = ((k + 1) % N) as f64 / N as f64;
            close(*y, model(&pts, true, x), TOL, &format!("segment {k}"));
        }
        traces.push(trace);
    }
    assert_eq!(
        traces[0].iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
        traces[1].iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
        "a loop does not care about the key"
    );
}

#[test]
fn one_shot_runs_once_and_holds_the_last_level_held_or_released() {
    let pts: [Pt; 4] = [
        (0.0, 0.0, 0.0),
        (20.0, 100.0, 0.0),
        (60.0, 40.0, -50.0),
        (100.0, 80.0, 0.0),
    ];
    let mut traces = Vec::new();
    for release_at in [None, Some(300usize), Some(900)] {
        let patch = voice_patch(mseg(
            "mseg1",
            MsegScope::Voice,
            MsegMode::OneShot,
            Beats::Quarter,
            &pts,
            &[],
        ));
        let trace = Rig::new(&patch).one(2000, |k| {
            let mut ev = at_start(k);
            if Some(k) == release_at {
                ev.push(off(60, 1));
            }
            ev
        });
        for (k, y) in trace.iter().enumerate() {
            let want = if k + 1 >= N {
                0.8
            } else {
                model(&pts, false, (k + 1) as f64 / N as f64)
            };
            close(*y, want, TOL, &format!("segment {k}"));
        }
        traces.push(trace);
    }
    for t in &traces[1..] {
        assert_eq!(
            t.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
            traces[0].iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
            "a one-shot does not care about the key"
        );
    }
}

/// The position (in segments of a 750-segment length) of a sustain loop after each segment: while `held` it wraps
/// from `end` back into `[start, end)`, keeping the overshoot; released it runs on to the end of the length.
fn sustain_positions(start: usize, end: usize, held_until: usize, segments: usize) -> Vec<usize> {
    let mut pos = 0usize;
    (0..segments)
        .map(|k| {
            let mut next = pos + 1;
            if k < held_until && pos < N && next >= end {
                next = start + (next - end) % (end - start);
            } else if next >= N {
                next = N;
            }
            pos = next;
            pos
        })
        .collect()
}

#[test]
fn sustain_loop_loops_while_held_and_plays_out_after_the_release() {
    // Points at 0, 10, 30, 50 and 100 %: loop between points 3 and 4 (segments 225 and 375).
    let pts: [Pt; 5] = [
        (0.0, 0.0, 0.0),
        (10.0, 100.0, 0.0),
        (30.0, 60.0, 0.0),
        (50.0, 80.0, 0.0),
        (100.0, 10.0, 0.0),
    ];
    // Held for ever; released inside the loop; released during the rise; released after a few turns.
    for release_at in [None, Some(1000usize), Some(40), Some(1700)] {
        let patch = voice_patch(mseg(
            "mseg1",
            MsegScope::Voice,
            MsegMode::SustainLoop,
            Beats::Quarter,
            &pts,
            &[(MsegParam::LoopStart, 3.0), (MsegParam::LoopEnd, 4.0)],
        ));
        let segments = 3000;
        let trace = Rig::new(&patch).one(segments, |k| {
            let mut ev = at_start(k);
            if Some(k) == release_at {
                ev.push(off(60, 1));
            }
            ev
        });
        let positions = sustain_positions(225, 375, release_at.unwrap_or(usize::MAX), segments);
        for (k, y) in trace.iter().enumerate() {
            let want = model(&pts, false, positions[k] as f64 / N as f64);
            close(
                *y,
                want,
                TOL,
                &format!("release {release_at:?}, segment {k}"),
            );
        }
        match release_at {
            None => assert!(positions.iter().all(|&p| p < N), "held: it loops for ever"),
            Some(_) => assert_eq!(positions[segments - 1], N, "released: it plays out"),
        }
    }
}

#[test]
fn a_sustain_loop_with_equal_loop_points_holds_that_level() {
    // Attack to 100 % at 10 %, the sustain is point 2 alone (loop 2..2), release down to 0 at 100 %.
    let pts: [Pt; 3] = [(0.0, 0.0, 0.0), (10.0, 100.0, 0.0), (100.0, 0.0, 0.0)];
    let patch = voice_patch(mseg(
        "mseg1",
        MsegScope::Voice,
        MsegMode::SustainLoop,
        Beats::Quarter,
        &pts,
        &[(MsegParam::LoopStart, 2.0), (MsegParam::LoopEnd, 2.0)],
    ));
    let trace = Rig::new(&patch).one(2200, |k| {
        let mut ev = at_start(k);
        if k == 1500 {
            ev.push(off(60, 1));
        }
        ev
    });
    for (k, y) in trace.iter().enumerate().take(1500) {
        let want = if k < 74 { (k + 1) as f64 / 75.0 } else { 1.0 };
        close(*y, want, TOL, &format!("held, segment {k}"));
    }
    // Released at 1500: down over the 675 segments left, from the hold position (the loop never moved on).
    let last = trace[1500..].iter().rposition(|y| *y > 1e-9).unwrap();
    assert!(
        last >= 600,
        "the release takes the rest of the length: {last}"
    );
    close(trace[2199], 0.0, TOL, "the end");
}

// ---- the synced length, exact across tempo changes and at every host block size ----

/// Runs of `(bpm, segments)` whose quarter-note progress `bpm / 90000` per segment adds up to exactly 1.
const RUNS: [(f64, usize); 5] = [
    (120.0, 150),
    (60.0, 300),
    (240.0, 100),
    (90.0, 200),
    (240.0, 50),
];
const RUNS_TOTAL: usize = 800;

/// The tempo events of [`RUNS`] at absolute frames: each lands at a segment boundary but 5 frames early, inside the
/// call before it.
fn tempo_events() -> Vec<(u64, EventKind)> {
    let mut ev = Vec::new();
    let mut start = 0usize;
    for (bpm, count) in RUNS {
        let frame = if start == 0 {
            0
        } else {
            (start * SEG - 5) as u64
        };
        ev.push((frame, tempo(bpm)));
        start += count;
    }
    assert_eq!(start, RUNS_TOTAL);
    ev
}

fn hold_then_jump(scope: MsegScope) -> ModuleInstance {
    mseg(
        "mseg1",
        scope,
        MsegMode::OneShot,
        Beats::Quarter,
        &[(0.0, 0.0, -100.0), (100.0, 100.0, 0.0)],
        &[],
    )
}

#[test]
fn a_synced_one_shot_ends_on_the_exact_segment_across_tempo_changes() {
    assert_eq!(
        RUNS.iter().map(|&(b, n)| b as usize * n).sum::<usize>(),
        90_000
    );
    for scope in [MsegScope::Global, MsegScope::Voice] {
        let patch = match scope {
            MsegScope::Global => global_patch(hold_then_jump(scope)),
            MsegScope::Voice => voice_patch(hold_then_jump(scope)),
        };
        let mut start = 0usize;
        let mut changes = Vec::new();
        for (bpm, count) in RUNS {
            changes.push((start, bpm));
            start += count;
        }
        let trace = Rig::new(&patch).one(RUNS_TOTAL + 20, |k| {
            let mut ev: Vec<EventKind> = changes
                .iter()
                .filter(|(at, _)| *at == k)
                .map(|(_, bpm)| tempo(*bpm))
                .collect();
            if k == 0 && scope == MsegScope::Voice {
                ev.push(on(60, 1, 0.8));
            }
            ev
        });
        for (k, y) in trace.iter().enumerate() {
            let want = if k + 1 >= RUNS_TOTAL { 1.0 } else { 0.0 };
            close(*y, want, TOL, &format!("{scope:?}, segment {k}"));
        }
    }
}

#[test]
fn a_synced_length_is_exact_at_host_blocks_1_64_100_and_4096_across_a_tempo_change() {
    // The jump of the hold-then-jump shape gates the oscillator's level: silence until segment 799, then sound. The
    // first sound is in segment 799's frames (one segment late would be 800's), whatever the host block.
    let mut p = reset_saw_patch();
    p.generators[0].values[OscParam::Level.index()] = 0.0;
    p.modulators.push(hold_then_jump(MsegScope::Voice));
    route(&mut p, "mseg1", "osc1.level", 100.0);
    let mut ev = tempo_events();
    ev.extend(events(&[note(69, 0, 1_000_000)]).into_iter().take(1));
    ev.sort_by_key(|(f, _)| *f);
    let frames = (RUNS_TOTAL + 40) * SEG;
    let mut reference: Option<Vec<u32>> = None;
    for block in [1usize, 7, 32, 64, 100, 4096, frames] {
        let (mut e, _) = compiled(&p);
        let (l, _) = drive(&mut e, &ev, frames, block, |_, _, _| {});
        let first = l.iter().position(|x| *x != 0.0).expect("sound");
        assert!(
            (799 * SEG..800 * SEG).contains(&first),
            "block {block}: the first sample is at frame {first}, want segment 799 ({}..{})",
            799 * SEG,
            800 * SEG
        );
        match &reference {
            None => reference = Some(bits(&l)),
            Some(r) => assert_eq!(&bits(&l), r, "block {block}"),
        }
    }
}

#[test]
fn a_ms_length_is_exact_too() {
    // 500 ms = 750 segments, 1 s = 1500: the jump lands on the last segment of the length.
    for (length, n) in [(500.0f32, 750usize), (1000.0, 1500), (10.0, 15)] {
        let m = mseg(
            "mseg1",
            MsegScope::Global,
            MsegMode::OneShot,
            Beats::Quarter,
            &[(0.0, 0.0, -100.0), (100.0, 100.0, 0.0)],
            &[(MsegParam::Sync, 0.0), (MsegParam::Length, length)],
        );
        let trace = Rig::new(&global_patch(m)).one(n + 10, |_| Vec::new());
        for (k, y) in trace.iter().enumerate() {
            close(
                *y,
                if k + 1 >= n { 1.0 } else { 0.0 },
                TOL,
                &format!("{length} ms, segment {k}"),
            );
        }
    }
}

// ---- curves ----

#[test]
fn the_curves_follow_the_formula_through_the_engine() {
    for c in [-100.0f32, -80.0, -30.0, 0.0, 30.0, 80.0, 100.0] {
        let pts: [Pt; 2] = [(0.0, 10.0, c), (100.0, 90.0, 0.0)];
        let m = mseg(
            "mseg1",
            MsegScope::Global,
            MsegMode::OneShot,
            Beats::Quarter,
            &pts,
            &[],
        );
        let trace = Rig::new(&global_patch(m)).one(N + 5, |k| {
            if k == 0 {
                vec![tempo(120.0)]
            } else {
                Vec::new()
            }
        });
        for (k, y) in trace.iter().enumerate() {
            let want = if k + 1 >= N {
                0.9
            } else {
                model(&pts, false, (k + 1) as f64 / N as f64)
            };
            close(*y, want, TOL, &format!("curve {c}, segment {k}"));
        }
    }
}

// ---- long times ----

#[test]
fn a_60_s_envelope_reaches_its_levels_at_the_right_segment() {
    // 60 s = 90,000 segments. Points at 10 %, 50 % and the end: 9,000, 45,000 and 90,000. Read every 15 segments.
    let pts: [Pt; 4] = [
        (0.0, 0.0, 0.0),
        (10.0, 100.0, 0.0),
        (50.0, 40.0, 0.0),
        (100.0, 0.0, 0.0),
    ];
    for scope in [MsegScope::Global, MsegScope::Voice] {
        let m = mseg(
            "mseg1",
            scope,
            MsegMode::OneShot,
            Beats::Quarter,
            &pts,
            &[(MsegParam::Sync, 0.0), (MsegParam::Length, 60_000.0)],
        );
        let patch = match scope {
            MsegScope::Global => global_patch(m),
            MsegScope::Voice => voice_patch(m),
        };
        let mut rig = Rig::new(&patch);
        let mut seen = Vec::new();
        for call in 0..=6000usize {
            // 15 segments per call: 480 frames.
            let ev = if call == 0 && scope == MsegScope::Voice {
                vec![Event {
                    offset: 0,
                    kind: on(60, 1, 0.8),
                }]
            } else {
                Vec::new()
            };
            let (mut l, mut r) = (vec![0.0f32; 15 * SEG], vec![0.0f32; 15 * SEG]);
            rig.engine.process(&mut l, &mut r, &ev);
            let y = if scope == MsegScope::Global {
                (f64::from(rig.engine.value(rig.out, 0).unwrap()) - 50.0) / 50.0
            } else {
                f64::from(rig.engine.value(rig.out, 0).unwrap()) / 100.0
            };
            seen.push(y);
        }
        for (call, y) in seen.iter().enumerate().take(6000) {
            // After call m the last segment is index 15 (m + 1) - 1: the position is 15 (m + 1) / 90000.
            let x = (15 * (call + 1)) as f64 / 90_000.0;
            let want = if x >= 1.0 { 0.0 } else { model(&pts, false, x) };
            close(*y, want, 5e-6, &format!("{scope:?}, call {call}"));
        }
        // The levels at the points, at the right call: 9,000 segments is call 599, 45,000 is call 2,999.
        close(seen[599], 1.0, 5e-6, "point 2");
        close(seen[2999], 0.4, 5e-6, "point 3");
        close(seen[5999], 0.0, 5e-6, "the end");
        // One call before the point it is still on the way (a segment late or early would show here).
        assert!(seen[598] < 1.0 - 1e-4 && seen[2998] > 0.4 + 1e-4);
    }
}

// ---- velocity and polarity ----

#[test]
fn velocity_scales_a_voice_mseg_and_a_global_one_ignores_it() {
    let pts: [Pt; 2] = [(0.0, 80.0, 0.0), (100.0, 80.0, 0.0)];
    let set = [(MsegParam::Velocity, 50.0)];
    for (polarity, level) in [(Polarity::Unipolar, 0.8), (Polarity::Bipolar, 0.6)] {
        let mut set = set.to_vec();
        set.push((MsegParam::Polarity, polarity.index() as f32));
        let voice = voice_patch(mseg(
            "mseg1",
            MsegScope::Voice,
            MsegMode::Loop,
            Beats::Quarter,
            &pts,
            &set,
        ));
        let trace = Rig::new(&voice).one(10, |k| {
            if k == 0 {
                vec![on(60, 1, 0.4)]
            } else {
                Vec::new()
            }
        });
        // 1 - 0.5 (1 - 0.4) = 0.7
        close(trace[5], level * 0.7, TOL, &format!("voice, {polarity:?}"));
        let global = global_patch(mseg(
            "mseg1",
            MsegScope::Global,
            MsegMode::Loop,
            Beats::Quarter,
            &pts,
            &set,
        ));
        let trace = Rig::new(&global).one(10, |k| {
            if k == 0 {
                vec![on(60, 1, 0.1)]
            } else {
                Vec::new()
            }
        });
        close(trace[5], level, TOL, &format!("global, {polarity:?}"));
    }
}

// ---- retrigger ----

fn saw_shape() -> [Pt; 2] {
    [(0.0, 0.0, 0.0), (100.0, 100.0, 0.0)]
}

fn one_shot_saw(mode: VoiceMode) -> Patch {
    let mut p = voice_patch(mseg(
        "mseg1",
        MsegScope::Voice,
        MsegMode::OneShot,
        Beats::Quarter,
        &saw_shape(),
        &[],
    ));
    p.voice.values[VoiceParam::Mode.index()] = mode.index() as f32;
    p
}

#[test]
fn mono_restarts_on_a_new_note_and_legato_goes_on() {
    let second = 300usize;
    let events = |k: usize| match k {
        0 => vec![tempo(120.0), on(60, 1, 0.8)],
        k if k == second => vec![on(64, 2, 0.8)],
        _ => Vec::new(),
    };
    // Mono: the second note (the first is still held) restarts the shape: the value after segment 300 is one step.
    let trace = Rig::new(&one_shot_saw(VoiceMode::Mono)).one(900, events);
    for (k, y) in trace.iter().enumerate() {
        let n = if k < second { k + 1 } else { k - second + 1 };
        close(
            *y,
            (n as f64 / N as f64).min(1.0),
            TOL,
            &format!("mono, segment {k}"),
        );
    }
    // Legato: the shape goes on.
    let trace = Rig::new(&one_shot_saw(VoiceMode::Legato)).one(900, events);
    for (k, y) in trace.iter().enumerate() {
        close(
            *y,
            ((k + 1) as f64 / N as f64).min(1.0),
            TOL,
            &format!("legato, segment {k}"),
        );
    }
    // Mono, the first note already released (the voice is in its release): a new note still restarts.
    let trace = Rig::new(&one_shot_saw(VoiceMode::Mono)).one(900, |k| match k {
        0 => vec![tempo(120.0), on(60, 1, 0.8)],
        200 => vec![off(60, 1)],
        k if k == second => vec![on(64, 2, 0.8)],
        _ => Vec::new(),
    });
    close(trace[299], 300.0 / N as f64, TOL, "before the second note");
    close(
        trace[300],
        1.0 / N as f64,
        TOL,
        "the second note restarts it",
    );
}

#[test]
fn poly_voices_keep_their_own_position_and_the_start_phase_applies_to_each() {
    let patch = voice_patch(mseg(
        "mseg1",
        MsegScope::Voice,
        MsegMode::OneShot,
        Beats::Quarter,
        &saw_shape(),
        &[(MsegParam::Phase, 90.0)],
    ));
    let mut rig = Rig::new(&patch);
    let frames = rig.run(1000, |k| match k {
        0 => vec![tempo(120.0), on(60, 1, 0.8)],
        200 => vec![on(64, 2, 0.8)],
        _ => Vec::new(),
    });
    // Each note starts a quarter of the way in (90 degrees) and has 562.5 segments to go.
    for (k, voices) in frames.iter().enumerate() {
        for &(note, y) in voices {
            let began = if note == 1 { 0 } else { 200 };
            let n = (k - began + 1) as f64;
            close(
                y,
                (0.25 + n / N as f64).min(1.0),
                TOL,
                &format!("note {note}, segment {k}"),
            );
        }
    }
    assert_eq!(frames[0].len(), 1);
    assert_eq!(frames[300].len(), 2);
}

// ---- global scope: the restart rule (F1m S7) ----

fn global_mseg(mode: MsegMode, pts: &[Pt], set: &[(MsegParam, f32)]) -> Patch {
    global_patch(mseg(
        "mseg1",
        MsegScope::Global,
        mode,
        Beats::Quarter,
        pts,
        set,
    ))
}

#[test]
fn a_global_loop_free_runs_and_ignores_the_notes() {
    let pts = saw_shape();
    let patch = global_mseg(MsegMode::Loop, &pts, &[]);
    let with_notes = Rig::new(&patch).one(2000, |k| match k {
        0 => vec![tempo(120.0)],
        100 => vec![on(60, 1, 0.8)],
        900 => vec![off(60, 1)],
        1500 => vec![on(64, 2, 0.8)],
        _ => Vec::new(),
    });
    let without = Rig::new(&patch).one(2000, |k| {
        if k == 0 {
            vec![tempo(120.0)]
        } else {
            Vec::new()
        }
    });
    assert_eq!(
        with_notes.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
        without.iter().map(|v| v.to_bits()).collect::<Vec<_>>()
    );
    for (k, y) in without.iter().enumerate() {
        close(
            *y,
            ((k + 1) % N) as f64 / N as f64,
            TOL,
            &format!("segment {k}"),
        );
    }
}

#[test]
fn a_global_one_shot_restarts_at_the_first_key_of_a_phrase_only() {
    let patch = global_mseg(MsegMode::OneShot, &saw_shape(), &[]);
    // Segment 1000: key A. 1100: key B on top of it (no restart). 1200: A up (B still down). 1300: B up.
    // 1900: key C. 2000: C up and key D in the same segment (the phrase ended, so D starts a new one).
    let restarts = [0usize, 1000, 1900, 2000];
    let trace = Rig::new(&patch).one(2600, |k| match k {
        0 => vec![tempo(120.0)],
        1000 => vec![on(60, 1, 0.8)],
        1100 => vec![on(64, 2, 0.8)],
        1200 => vec![off(60, 1)],
        1300 => vec![off(64, 2)],
        1900 => vec![on(67, 3, 0.8)],
        2000 => vec![off(67, 3), on(72, 4, 0.8)],
        _ => Vec::new(),
    });
    for (k, y) in trace.iter().enumerate() {
        let r = *restarts.iter().rev().find(|&&r| r <= k).unwrap();
        close(
            *y,
            ((k - r + 1) as f64 / N as f64).min(1.0),
            TOL,
            &format!("segment {k} (restart at {r})"),
        );
    }
    // From the engine's start it ran its length and held: before the first key the value is the end.
    close(trace[999], 1.0, TOL, "before any key");
}

#[test]
fn a_global_sustain_loop_is_gated_by_any_key_and_plays_out_when_the_last_is_up() {
    // 0 → 100 % at 20 %, flat to 40 %, down to 0 at 100 %; loop points 2 and 3 (segments 150 and 300).
    let pts: [Pt; 4] = [
        (0.0, 0.0, 0.0),
        (20.0, 100.0, 0.0),
        (40.0, 100.0, 0.0),
        (100.0, 0.0, 0.0),
    ];
    let patch = global_mseg(
        MsegMode::SustainLoop,
        &pts,
        &[(MsegParam::LoopStart, 2.0), (MsegParam::LoopEnd, 3.0)],
    );
    // Key A from 100 to 900, key B from 400 to 1200: the gate is down only at 1200.
    let trace = Rig::new(&patch).one(2200, |k| match k {
        0 => vec![tempo(120.0)],
        100 => vec![on(60, 1, 0.3)],
        400 => vec![on(64, 2, 0.3)],
        900 => vec![off(60, 1)],
        1200 => vec![off(64, 2)],
        _ => Vec::new(),
    });
    // The state machine in segments: free-running from 0 with no key (the first 100 segments), a restart at 100,
    // looping over 150..300 while a key is down, then on to 750.
    let mut pos = 0usize;
    for (k, y) in trace.iter().enumerate() {
        if k == 100 {
            pos = 0;
        }
        let held = (100..1200).contains(&k);
        let mut next = pos + 1;
        if held && pos < N && next >= 300 {
            next = 150 + (next - 300) % 150;
        } else if next >= N {
            next = N;
        }
        pos = next;
        close(
            *y,
            model(&pts, false, pos as f64 / N as f64),
            TOL,
            &format!("segment {k}"),
        );
    }
    assert!(trace[1199] > 0.99, "still looping on the flat part");
    close(trace[2199], 0.0, TOL, "played out");
}

#[test]
fn a_reset_restarts_a_global_one_shot() {
    let patch = global_mseg(MsegMode::OneShot, &saw_shape(), &[]);
    let mut rig = Rig::new(&patch);
    let first = rig.one(900, |k| {
        if k == 0 {
            vec![tempo(120.0)]
        } else {
            Vec::new()
        }
    });
    close(first[899], 1.0, TOL, "ended");
    rig.engine.reset();
    // A reset puts the tempo back to the default 140: 1.0 / 140 s per quarter: 642.857 segments.
    let again = rig.one(10, |_| Vec::new());
    let cps = 140.0 / 90_000.0;
    for (k, y) in again.iter().enumerate() {
        close(
            *y,
            (k + 1) as f64 * cps,
            TOL,
            &format!("segment {k} after the reset"),
        );
    }
}

// ---- edits do not click ----

#[test]
fn an_edit_in_the_middle_of_a_note_glides_over_eight_segments() {
    let pts: [Pt; 2] = [(0.0, 50.0, 0.0), (100.0, 50.0, 0.0)];
    let patch = voice_patch(mseg(
        "mseg1",
        MsegScope::Voice,
        MsegMode::SustainLoop,
        Beats::Quarter,
        &pts,
        &[(MsegParam::LoopStart, 1.0), (MsegParam::LoopEnd, 2.0)],
    ));
    let mut rig = Rig::new(&patch);
    let level1 = param(&rig.map, "mseg1.level1");
    let level2 = param(&rig.map, "mseg1.level2");
    let trace = rig.one(240, |k| match k {
        0 => vec![tempo(120.0), on(60, 1, 0.8)],
        200 => vec![
            EventKind::Param {
                id: level1,
                value: 100.0,
            },
            EventKind::Param {
                id: level2,
                value: 100.0,
            },
        ],
        _ => Vec::new(),
    });
    for (k, y) in trace.iter().enumerate() {
        let want = match k {
            0..=200 => 0.5,
            201..=207 => 0.5 + 0.5 * (k - 200) as f64 / 8.0,
            _ => 1.0,
        };
        close(*y, want, 2e-5, &format!("segment {k}"));
    }
}

/// A small deterministic generator (xorshift64*), for a scripted sequence of edits.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
}

#[test]
fn a_run_of_edits_never_steps_more_than_an_eighth_of_the_edit_plus_the_slope() {
    // A looping six-point shape with straight segments at least 10 % long: its own slope is under 1 / 75 per
    // segment. Every 40 segments one point's time or level changes, or the mode flips between loop and sustain loop;
    // the output may move by at most 1/8 (a whole-range edit's share of one glide step) plus the slope.
    let pts: [Pt; 6] = [
        (0.0, 0.0, 0.0),
        (15.0, 100.0, 0.0),
        (30.0, 20.0, 0.0),
        (50.0, 80.0, 0.0),
        (70.0, 10.0, 0.0),
        (85.0, 60.0, 0.0),
    ];
    let patch = voice_patch(mseg(
        "mseg1",
        MsegScope::Voice,
        MsegMode::Loop,
        Beats::Quarter,
        &pts,
        &[],
    ));
    let mut rig = Rig::new(&patch);
    let time_ids: Vec<ParamId> = (1..=6)
        .map(|i| param(&rig.map, &format!("mseg1.time{i}")))
        .collect();
    let level_ids: Vec<ParamId> = (1..=6)
        .map(|i| param(&rig.map, &format!("mseg1.level{i}")))
        .collect();
    let mode = param(&rig.map, "mseg1.mode");
    let mut rng = Lcg(0x9E37_79B9_7F4A_7C15);
    let mut times: Vec<f32> = pts.iter().map(|p| p.0).collect();
    let trace = rig.one(4000, |k| {
        if k == 0 {
            return vec![tempo(120.0), on(60, 1, 0.8)];
        }
        if k % 40 != 0 {
            return Vec::new();
        }
        let r = rng.next();
        let i = (r >> 8) as usize % 6;
        match r % 3 {
            0 => {
                // A level anywhere.
                let v = ((r >> 16) % 101) as f32;
                vec![EventKind::Param {
                    id: level_ids[i],
                    value: v,
                }]
            }
            1 if i > 0 => {
                // A time between its neighbours, 10 % apart at least.
                let lo = times[i - 1] + 10.0;
                let hi = if i == 5 { 95.0 } else { times[i + 1] - 10.0 };
                if hi < lo {
                    return Vec::new();
                }
                let v = lo + (hi - lo) * (((r >> 16) % 101) as f32 / 100.0);
                times[i] = v;
                vec![EventKind::Param {
                    id: time_ids[i],
                    value: v,
                }]
            }
            _ => {
                let v = if (r >> 20) & 1 == 0 {
                    MsegMode::Loop
                } else {
                    MsegMode::SustainLoop
                };
                vec![EventKind::Param {
                    id: mode,
                    value: v.index() as f32,
                }]
            }
        }
    });
    let slope = 1.0 / 75.0;
    let worst = trace
        .windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0f64, f64::max);
    assert!(
        worst <= 0.125 + slope + 1e-4,
        "the biggest step between two segments is {worst}"
    );
    assert!(trace.iter().all(|y| (-1e-6..=1.000001).contains(y)));
}

// ---- the module contract ----

fn busy() -> Patch {
    let mut p = reset_saw_patch();
    p.modulators[0].values[AdsrParam::Release.index()] = 3_000.0;
    p.voice_fx
        .push(ModuleInstance::new(id("filter1"), ModuleKind::Svf));
    p.voice_fx[0].values[SvfParam::Cutoff.index()] = 1500.0;
    let mut comp = ModuleInstance::new(id("comp1"), ModuleKind::Comp);
    comp.values[CompParam::Mix.index()] = 50.0;
    p.fx.push(comp);
    // A wobble on the cutoff, a sustain loop on the pitch, a global loop and a global one-shot on the mix.
    p.modulators.push(mseg(
        "mseg1",
        MsegScope::Voice,
        MsegMode::Loop,
        Beats::Eighth,
        &[
            (0.0, 100.0, -40.0),
            (30.0, 10.0, 60.0),
            (60.0, 70.0, 0.0),
            (100.0, 100.0, 20.0),
        ],
        &[],
    ));
    p.modulators.push(mseg(
        "mseg2",
        MsegScope::Voice,
        MsegMode::SustainLoop,
        Beats::Quarter,
        &[
            (0.0, 0.0, 0.0),
            (20.0, 100.0, 30.0),
            (50.0, 60.0, 0.0),
            (100.0, 0.0, -30.0),
        ],
        &[
            (MsegParam::LoopStart, 2.0),
            (MsegParam::LoopEnd, 3.0),
            (MsegParam::Polarity, 0.0),
            (MsegParam::Velocity, 60.0),
        ],
    ));
    p.modulators.push(mseg(
        "mseg3",
        MsegScope::Global,
        MsegMode::Loop,
        Beats::Quarter,
        &[(0.0, 0.0, 0.0), (50.0, 100.0, 0.0), (100.0, 0.0, 0.0)],
        &[],
    ));
    route(&mut p, "mseg1", "filter1.cutoff", 60.0);
    route(&mut p, "mseg2", "osc1.fine", 40.0);
    route(&mut p, "mseg3", "comp1.mix", 30.0);
    p
}

fn performance(map: &PlanMap) -> Vec<(u64, EventKind)> {
    let mut ev = events(&[
        note(48, 0, 9000),
        note(55, 1500, 17_000),
        note(64, 2001, 12_000),
    ]);
    for (f, bpm) in [(1u64, 120.0), (5000, 90.0), (13_777, 174.0), (20_000, 63.0)] {
        ev.push((f, tempo(bpm)));
    }
    // Edits while the notes play: a point's level and time, and the length switched to ms and back.
    for (f, addr, value) in [
        (3_001u64, "mseg1.level2", 55.0f32),
        (7_777, "mseg2.time3", 70.0),
        (11_111, "mseg1.curve1", 90.0),
        (15_000, "mseg2.mode", 1.0),
    ] {
        ev.push((
            f,
            EventKind::Param {
                id: param(map, addr),
                value,
            },
        ));
    }
    ev.sort_by_key(|(f, _)| *f);
    ev
}

#[test]
fn the_host_block_size_is_not_an_input() {
    let patch = busy();
    let frames = 36_000;
    let (mut reference, map) = compiled(&patch);
    let ev = performance(&map);
    let (rl, rr) = drive(&mut reference, &ev, frames, SEG, |_, _, _| {});
    assert!(rl.iter().any(|x| x.abs() > 0.05), "not silent");
    for block in [1usize, 2, 7, 31, 33, 64, 100, 441, 1000, 4096, frames] {
        let (mut e, _) = compiled(&patch);
        let (l, r) = drive(&mut e, &ev, frames, block, |_, _, _| {});
        assert_eq!(bits(&l), bits(&rl), "left, block {block}");
        assert_eq!(bits(&r), bits(&rr), "right, block {block}");
    }
    // The MSEGs are real inputs: with them off the render is another one.
    let mut quiet = patch.clone();
    quiet.routes.clear();
    let (mut e, _) = compiled(&quiet);
    let (ql, _) = drive(&mut e, &ev, frames, 100, |_, _, _| {});
    assert_ne!(bits(&ql), bits(&rl));
}

#[test]
fn two_instances_keep_separate_state() {
    // `mseg1` (a wobble) and `mseg2` (a one-shot rise) on the two oscillators' pitch, each alone and together: the
    // traces are the same bits.
    let wobble = mseg(
        "mseg1",
        MsegScope::Voice,
        MsegMode::Loop,
        Beats::Eighth,
        &[(0.0, 0.0, 0.0), (50.0, 100.0, 40.0), (100.0, 0.0, 0.0)],
        &[],
    );
    let rise = mseg(
        "mseg2",
        MsegScope::Voice,
        MsegMode::OneShot,
        Beats::Bar,
        &[(0.0, 0.0, -30.0), (100.0, 100.0, 0.0)],
        &[],
    );
    let build = |with1: bool, with2: bool| {
        let mut p = reset_saw_patch();
        p.generators
            .push(ModuleInstance::new(id("osc2"), ModuleKind::Osc));
        if with1 {
            p.modulators.push(wobble.clone());
            route(&mut p, "mseg1", "osc1.fine", 50.0);
        }
        if with2 {
            p.modulators.push(rise.clone());
            route(&mut p, "mseg2", "osc2.fine", 50.0);
        }
        p
    };
    let read = |p: &Patch| {
        let (mut e, map) = compiled(p);
        let (f1, f2) = (param(&map, "osc1.fine"), param(&map, "osc2.fine"));
        let (mut l, mut r) = ([0.0f32; SEG], [0.0f32; SEG]);
        let mut out = Vec::new();
        for k in 0..2000 {
            let ev: Vec<Event> = if k == 0 {
                vec![
                    Event {
                        offset: 0,
                        kind: tempo(120.0),
                    },
                    Event {
                        offset: 0,
                        kind: on(60, 1, 0.8),
                    },
                ]
            } else if k == 333 {
                vec![Event {
                    offset: 0,
                    kind: on(64, 2, 0.8),
                }]
            } else {
                Vec::new()
            };
            e.process(&mut l, &mut r, &ev);
            for &v in e.active_voices() {
                out.push((
                    e.value(f1, v).unwrap().to_bits(),
                    e.value(f2, v).unwrap().to_bits(),
                ));
            }
        }
        out
    };
    let both = read(&build(true, true));
    let only1 = read(&build(true, false));
    let only2 = read(&build(false, true));
    assert_eq!(both.len(), only1.len());
    for (i, ((b1, b2), ((a1, _), (_, c2)))) in both.iter().zip(only1.iter().zip(&only2)).enumerate()
    {
        assert_eq!(b1, a1, "mseg1 on its own, sample {i}");
        assert_eq!(b2, c2, "mseg2 on its own, sample {i}");
    }
    assert!(both.iter().any(|(a, b)| a != b));
}

#[test]
fn a_plan_swap_carries_the_msegs_state_and_an_unrelated_edit_changes_no_bit() {
    use enth_engine::{Link, LinkCapacity};
    let base = busy();
    let swapped = |edit: Option<&Patch>| {
        let (plan, _) = compile(&base, &PrepareCfg::new(SR)).unwrap();
        let (mut e, mut link): (Engine, Link) =
            Engine::with_link(plan, SEED, LinkCapacity::default());
        let mut edit = edit.map(|p| compile(p, &PrepareCfg::new(SR)).unwrap().0);
        let ev = [
            (0u64, tempo(120.0)),
            (0, on(48, 1, 0.8)),
            (0, on(55, 2, 0.8)),
            (60_000, off(48, 1)),
        ];
        let (mut l, mut r) = ([0.0f32; SEG], [0.0f32; SEG]);
        let mut out = Vec::new();
        for k in 0..2400usize {
            if k == 1000
                && let Some(p) = edit.take()
            {
                link.send_plan(p).expect("room for a plan");
            }
            link.collect();
            let chunk: Vec<Event> = ev
                .iter()
                .filter(|(f, _)| (*f / SEG as u64) as usize == k)
                .map(|&(f, kind)| Event {
                    offset: (f % SEG as u64) as u32,
                    kind,
                })
                .collect();
            e.process(&mut l, &mut r, &chunk);
            out.extend_from_slice(&l);
        }
        (bits(&out), e)
    };
    let (plain, _) = swapped(None);
    // The same patch recompiled, and one with an unrelated filter added: the MSEGs go on from where they were.
    let (same, e) = swapped(Some(&base));
    assert_eq!(e.stats().swaps, 1);
    assert_eq!(same, plain);
    let mut added = base.clone();
    added
        .modulators
        .push(ModuleInstance::new(id("lfo9"), ModuleKind::Lfo));
    let (with_lfo, e) = swapped(Some(&added));
    assert_eq!(e.stats().crossfades, 1);
    assert_eq!(with_lfo, plain);
}

fn tail_notes() -> Vec<common::Note> {
    vec![
        note(36, 0, SR as u64),
        note(48, 0, SR as u64),
        note(60, 960, SR as u64),
        note(84, 1920, SR as u64 + 4800),
    ]
}

#[test]
fn a_40_s_release_tail_has_no_subnormals_and_every_voice_ends() {
    let mut p = busy();
    p.modulators[0].values[AdsrParam::Release.index()] = 3_000.0;
    // A one-shot that runs the whole 40 s on a tiny level: its glide state must flush like the rest.
    p.modulators.push(mseg(
        "mseg4",
        MsegScope::Voice,
        MsegMode::OneShot,
        Beats::Bar,
        &[(0.0, 0.0, 0.0), (100.0, 0.001, 0.0)],
        &[],
    ));
    route(&mut p, "mseg4", "osc1.semi", 10.0);
    let (mut e, _) = compiled(&p);
    let (mut output, mut state, mut non_finite) = (0usize, 0usize, 0usize);
    let mut calls = 0usize;
    drive(
        &mut e,
        &events(&tail_notes()),
        40 * SR as usize,
        512,
        |e, l, r| {
            output += count_subnormals(l) + count_subnormals(r);
            non_finite += l.iter().chain(r).filter(|x| !x.is_finite()).count();
            if calls.is_multiple_of(16) {
                state = state.max(e.subnormals());
            }
            calls += 1;
        },
    );
    state = state.max(e.subnormals());
    assert_eq!(output, 0, "subnormal output samples");
    assert_eq!(state, 0, "subnormal values in the engine state");
    assert_eq!(non_finite, 0, "non-finite samples");
    assert!(
        e.active_voices().is_empty(),
        "every voice ends below -100 dB"
    );
}

#[cfg(target_arch = "x86_64")]
mod fp {
    use core::arch::asm;
    pub const FTZ: u64 = 1 << 15;
    pub const DAZ: u64 = 1 << 6;
    pub const MASK: u64 = FTZ | DAZ;
    pub fn get() -> u64 {
        let mut v: u32 = 0;
        // SAFETY: stmxcsr stores the 32-bit MXCSR to the address in the register; `v` is a valid u32.
        unsafe { asm!("stmxcsr [{}]", in(reg) &raw mut v, options(nostack, preserves_flags)) };
        u64::from(v)
    }
    pub fn set(v: u64) {
        let v = v as u32;
        // SAFETY: ldmxcsr loads MXCSR from a valid u32; only the FTZ/DAZ bits differ from the current value.
        unsafe { asm!("ldmxcsr [{}]", in(reg) &raw const v, options(nostack, preserves_flags)) };
    }
    pub const MODES: [(&str, u64); 3] = [("IEEE", 0), ("FTZ", FTZ), ("FTZ+DAZ", FTZ | DAZ)];
}

#[cfg(target_arch = "aarch64")]
mod fp {
    use core::arch::asm;
    pub const FZ: u64 = 1 << 24;
    pub const MASK: u64 = FZ;
    pub fn get() -> u64 {
        let v: u64;
        // SAFETY: reads the FPCR system register.
        unsafe { asm!("mrs {}, fpcr", out(reg) v, options(nostack, preserves_flags)) };
        v
    }
    pub fn set(v: u64) {
        // SAFETY: writes the FPCR system register; only FZ differs from the current value.
        unsafe { asm!("msr fpcr, {}", in(reg) v, options(nostack)) };
    }
    pub const MODES: [(&str, u64); 2] = [("IEEE", 0), ("FZ", FZ)];
}

#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
#[test]
fn the_render_is_the_same_under_every_ftz_mode() {
    struct Restore(u64);
    impl Drop for Restore {
        fn drop(&mut self) {
            fp::set(self.0);
        }
    }
    let run = || {
        let (mut e, map) = compiled(&busy());
        let mut ev = events(&tail_notes());
        ev.push((
            4_000,
            EventKind::Param {
                id: param(&map, "mseg1.level2"),
                value: 20.0,
            },
        ));
        ev.sort_by_key(|(f, _)| *f);
        let (l, r) = drive(&mut e, &ev, 6 * SR as usize, 512, |_, _, _| {});
        (bits(&l), bits(&r))
    };
    let with_mode = |mode: u64| {
        let old = fp::get();
        let _restore = Restore(old);
        fp::set((old & !fp::MASK) | mode);
        std::hint::black_box(run())
    };
    let reference = with_mode(0);
    for (name, mode) in fp::MODES {
        assert!(with_mode(mode) == reference, "the render under {name}");
    }
}

#[cfg(debug_assertions)]
mod noalloc {
    use super::*;
    use assert_no_alloc::{AllocDisabler, assert_no_alloc};

    #[global_allocator]
    static ALLOCATOR: AllocDisabler = AllocDisabler;

    /// Every host call inside the guard: notes, tempo changes, point and mode edits, all three modes, both scopes.
    #[test]
    fn process_does_not_allocate_with_msegs_edits_and_tempo_changes() {
        let (_, map) = compiled(&busy());
        let mut ev = performance(&map);
        for (f, addr, value) in [
            (2_500u64, "mseg3.mode", 1.0f32),
            (9_000, "mseg3.mode", 2.0),
            (9_500, "mseg2.loop_start", 1.0),
            (10_000, "mseg1.points", 16.0),
            (14_000, "mseg1.sync", 0.0),
            (14_001, "mseg1.length", 37.0),
            (16_000, "mseg1.polarity", 1.0),
        ] {
            ev.push((
                f,
                EventKind::Param {
                    id: param(&map, addr),
                    value,
                },
            ));
        }
        ev.sort_by_key(|(f, _)| *f);
        for block in [1usize, 64, 4096] {
            let mut e2 = compiled(&busy()).0;
            let frames = 30_000;
            let mut left = vec![0.0f32; frames];
            let mut right = vec![0.0f32; frames];
            let mut chunk: Vec<Event> = Vec::with_capacity(ev.len() + 1);
            let (mut start, mut next) = (0usize, 0usize);
            while start < frames {
                let end = (start + block).min(frames);
                chunk.clear();
                while let Some(&(f, kind)) = ev.get(next) {
                    if f >= end as u64 {
                        break;
                    }
                    chunk.push(Event {
                        offset: (f - start as u64) as u32,
                        kind,
                    });
                    next += 1;
                }
                let (l, r) = (&mut left[start..end], &mut right[start..end]);
                assert_no_alloc(|| e2.process(l, r, &chunk));
                start = end;
            }
            assert!(left.iter().any(|x| *x != 0.0), "not silent");
        }
    }
}
