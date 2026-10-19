//! Lane D1: `osc` and `noise` never allocate while rendering: every wave and table, wave and position automation,
//! hard sync, both noise colours, full polyphony. The wavetables are built (or fetched from the process-wide cache) in
//! `compile`, outside the guard. Debug builds only (`assert_no_alloc` drops its checks in release).
#![cfg(debug_assertions)]

mod common;

use assert_no_alloc::{AllocDisabler, assert_no_alloc};
use common::{SEED, SR, events, id, note};
use enth_engine::{Engine, Event, EventKind, PrepareCfg, compile};
use enth_patch::catalogue::{
    AdsrParam, NoiseColor, NoiseParam, OscParam, VoiceParam, Wave, Wavetable,
};
use enth_patch::{ModuleInstance, ModuleKind, Patch};

#[global_allocator]
static ALLOCATOR: AllocDisabler = AllocDisabler;

/// Run `ev` (absolute frames) through `engine` in `block`-frame calls, each inside the guard.
fn guarded(engine: &mut Engine, ev: &[(u64, EventKind)], frames: usize, block: usize) -> Vec<f32> {
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
        assert_no_alloc(|| engine.process(l, r, &chunk));
        start = end;
    }
    left
}

/// Every module of the lane: three oscillators (the first the table one, the third a synced pulse), and noise.
fn full_patch() -> Patch {
    let mut p = Patch::init();
    p.modulators[0].values[AdsrParam::Release.index()] = 100.0;
    p.voice.values[VoiceParam::Polyphony.index()] = 16.0;
    let o = &mut p.generators[0].values;
    o[OscParam::Wave.index()] = Wave::Table.index() as f32;
    o[OscParam::Table.index()] = Wavetable::Formant.index() as f32;
    o[OscParam::Position.index()] = 40.0;
    let mut o2 = ModuleInstance::new(id("osc2"), ModuleKind::Osc);
    o2.values[OscParam::Wave.index()] = Wave::Pulse.index() as f32;
    o2.values[OscParam::Sync.index()] = 1.0;
    o2.values[OscParam::Semi.index()] = 9.0;
    p.generators.push(o2);
    let mut o3 = ModuleInstance::new(id("osc3"), ModuleKind::Osc);
    o3.values[OscParam::Wave.index()] = Wave::Sine.index() as f32;
    o3.values[OscParam::Table.index()] = Wavetable::Organ.index() as f32;
    p.generators.push(o3);
    let mut n = ModuleInstance::new(id("noise1"), ModuleKind::Noise);
    n.values[NoiseParam::Color.index()] = NoiseColor::Pink.index() as f32;
    p.generators.push(n);
    p
}

fn set_events(patch: &Patch, sets: &[(u64, &str, f32)]) -> Vec<(u64, EventKind)> {
    let (_, map) = compile(patch, &PrepareCfg::new(SR)).unwrap();
    sets.iter()
        .map(|&(f, addr, value)| {
            (
                f,
                EventKind::Param {
                    id: map.get(&addr.parse().unwrap()).unwrap(),
                    value,
                },
            )
        })
        .collect()
}

#[test]
fn osc_and_noise_do_not_allocate_under_automation_sync_and_full_polyphony() {
    let patch = full_patch();
    for block in [1, 33, 512, 4096] {
        let notes: Vec<_> = (0..16)
            .map(|i| note(36 + 3 * i as u8, i * 200, 9_000 + i * 50))
            .collect();
        let mut ev = events(&notes);
        let mut sets = vec![
            (500, "osc1.wave", 0.0),
            (1_500, "osc1.wave", 1.0),
            (2_500, "osc1.wave", 2.0),
            (3_500, "osc1.wave", 3.0),
            (4_500, "osc1.wave", 4.0),
            (5_000, "osc1.position", 95.0),
            (5_500, "osc2.wave", 4.0),
            (6_000, "osc3.wave", 4.0),
            (6_500, "noise1.color", 0.0),
            (7_000, "noise1.level", 40.0),
            (7_500, "osc2.sync", 0.0),
            (8_000, "osc2.pw", 8.0),
            (8_500, "osc1.semi", 24.0),
            (9_000, "osc1.level", 0.0),
        ];
        sets.sort_by_key(|s| s.0);
        ev.extend(set_events(&patch, &sets));
        ev.sort_by_key(|(f, _)| *f);
        let mut e = Engine::new(compile(&patch, &PrepareCfg::new(SR)).unwrap().0, SEED);
        let out = guarded(&mut e, &ev, 24_000, block);
        assert!(out.iter().any(|x| *x != 0.0), "not silent");
        assert!(out.iter().all(|x| x.is_finite()));
        assert_eq!(e.subnormals(), 0);
    }
}

#[test]
fn every_table_renders_without_allocating() {
    for table in [
        Wavetable::Basic,
        Wavetable::Pulse,
        Wavetable::Tilt,
        Wavetable::Formant,
        Wavetable::Organ,
    ] {
        let mut p = Patch::init();
        p.generators[0].values[OscParam::Wave.index()] = Wave::Table.index() as f32;
        p.generators[0].values[OscParam::Table.index()] = table.index() as f32;
        p.generators[0].values[OscParam::Position.index()] = 62.0;
        let mut e = Engine::new(compile(&p, &PrepareCfg::new(SR)).unwrap().0, SEED);
        let out = guarded(
            &mut e,
            &events(&[note(21, 0, 6_000), note(108, 0, 6_000)]),
            8_000,
            256,
        );
        assert!(out.iter().any(|x| *x != 0.0), "{table:?}");
    }
}
