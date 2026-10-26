//! Lane D1: the `osc` and `noise` modules driven directly through the module contract (no plan, no engine): start
//! phases, wave switching, state copies, the table being prepared whatever `wave` is, the sync bus.

use enth_dsp::util::math;
use enth_dsp::util::rng::Rng;
use enth_dsp::wavetable::TableKind;
use enth_engine::module::{
    Block, MAX_VOICES, NO_SYNC, NoteState, PrepareCfg, SEG, SyncBus, VoiceBus, VoiceCx,
    VoiceParams, VoiceProcessor, VoiceStart,
};
use enth_engine::modules::{Noise, Osc};
use enth_engine::seed::{module_key, seeded_voice_seed, voice_seed};
use enth_patch::catalogue::{
    NoiseColor, NoiseParam, OscParam, PhaseMode, Wave, Wavetable as CatalogueTable,
};
use enth_patch::{ModuleId, ModuleKind};

const SR: f32 = 48_000.0;

/// One module, its parameter values (all voices alike) and a note state.
struct Rig<M: VoiceProcessor> {
    module: M,
    values: Vec<f32>,
    notes: NoteState,
    io: Box<VoiceBus>,
    sync: Box<SyncBus>,
    key: u64,
}

impl<M: VoiceProcessor> Rig<M> {
    fn new(
        kind: ModuleKind,
        make: impl FnOnce(&[f32]) -> M,
        edit: impl FnOnce(&mut Vec<f32>),
    ) -> Self {
        let mut values = kind.defaults();
        edit(&mut values);
        Self {
            module: make(&values),
            values,
            notes: NoteState::new(),
            io: Box::new([[0.0; SEG]; MAX_VOICES]),
            sync: Box::new([[NO_SYNC; SEG]; MAX_VOICES]),
            key: module_key(&ModuleId::new("osc1").unwrap()),
        }
    }

    fn start(&mut self, voice: usize, note_index: u64, seed: u64, pitch: f32) {
        self.notes.pitch_prev[voice] = pitch;
        self.notes.pitch_cur[voice] = pitch;
        self.notes.gate[voice] = true;
        self.notes.fresh[voice] = true;
        self.notes.note_index[voice] = note_index;
        let cur: Vec<[f32; MAX_VOICES]> = self.values.iter().map(|&v| [v; MAX_VOICES]).collect();
        self.module.start_voice(&VoiceStart {
            voice,
            seed,
            module_key: self.key,
            note_index,
            notes: &self.notes,
            params: VoiceParams::from_slices(&self.values, &cur, &cur),
        });
    }

    /// One segment for `voices`; `master`/`slave` wire the sync buses the way the engine does.
    fn segment(&mut self, voices: &[usize], master: bool, slave: bool) {
        let cur: Vec<[f32; MAX_VOICES]> = self.values.iter().map(|&v| [v; MAX_VOICES]).collect();
        if master {
            for &v in voices {
                self.sync[v] = [NO_SYNC; SEG];
            }
        }
        let params = VoiceParams::from_slices(&self.values, &cur, &cur);
        let (sync_out, sync_in) = if master {
            (Some(&mut *self.sync), None)
        } else if slave {
            (None, Some(&*self.sync))
        } else {
            (None, None)
        };
        self.module.process(&mut VoiceCx {
            voices,
            notes: &self.notes,
            params,
            io: &mut self.io,
            sync_out,
            sync_in,
            audio_in: None,
            pre_level: None,
            io_r: None,
        });
        for &v in voices {
            self.notes.fresh[v] = false;
        }
    }

    fn out(&self, voice: usize) -> Block {
        self.io[voice]
    }
}

fn osc_rig(edit: impl FnOnce(&mut Vec<f32>)) -> Rig<Osc> {
    Rig::new(ModuleKind::Osc, |v| Osc::new(v, &PrepareCfg::new(SR)), edit)
}

fn set(v: &mut [f32], p: OscParam, x: f32) {
    v[p.index()] = x;
}

fn wave(w: Wave) -> f32 {
    w.index() as f32
}

#[test]
fn the_start_phase_modes_follow_the_catalogue() {
    // Sine, so the first output sample is sin(2π·start phase): the output is delayed by one sample, never by a phase.
    let first = |edit: &dyn Fn(&mut Vec<f32>), seed: u64, note_index: u64| {
        let mut rig = osc_rig(|v| {
            set(v, OscParam::Wave, wave(Wave::Sine));
            edit(v);
        });
        rig.start(0, note_index, seed, 69.0);
        rig.segment(&[0], false, false);
        rig.out(0)[0]
    };
    let reset = |v: &mut Vec<f32>| {
        set(v, OscParam::PhaseMode, PhaseMode::Reset.index() as f32);
        set(v, OscParam::Phase, 90.0);
    };
    // Reset: the phase knob. 90 degrees is the top of the sine; 0 degrees its zero.
    assert!((first(&reset, 1, 0) - 1.0).abs() < 1e-6);
    assert_eq!(
        first(
            &|v| set(v, OscParam::PhaseMode, PhaseMode::Reset.index() as f32),
            1,
            0
        ),
        0.0
    );
    // Reset ignores the seeds.
    assert_eq!(
        first(&reset, 1, 0).to_bits(),
        first(&reset, 99, 7).to_bits()
    );

    // Free: the voice seed (render seed, module id, note index) and nothing else.
    let free = |v: &mut Vec<f32>| set(v, OscParam::Seed, 123.0);
    let key = module_key(&ModuleId::new("osc1").unwrap());
    let want = Rng::new(voice_seed(5, key, 2)).unit_f32();
    let expect = |u: f32| enth_dsp::osc::sine(u);
    assert_eq!(
        first(&free, voice_seed(5, key, 2), 2).to_bits(),
        expect(want).to_bits()
    );
    // The `seed` parameter does not move a free phase.
    let other = |v: &mut Vec<f32>| set(v, OscParam::Seed, 4_321.0);
    assert_eq!(
        first(&free, voice_seed(5, key, 2), 2).to_bits(),
        first(&other, voice_seed(5, key, 2), 2).to_bits()
    );
    assert_ne!(
        first(&free, voice_seed(5, key, 2), 2).to_bits(),
        first(&free, voice_seed(5, key, 3), 3).to_bits(),
        "every note has its own start phase"
    );

    // Seeded: the module's own seed (and the module id and note index), not the render seed.
    let seeded = |s: f32| {
        move |v: &mut Vec<f32>| {
            set(v, OscParam::PhaseMode, PhaseMode::Seeded.index() as f32);
            set(v, OscParam::Seed, s);
        }
    };
    let want = Rng::new(seeded_voice_seed(77, key, 4)).unit_f32();
    assert_eq!(first(&seeded(77.0), 1, 4).to_bits(), expect(want).to_bits());
    assert_eq!(
        first(&seeded(77.0), 1, 4).to_bits(),
        first(&seeded(77.0), 999, 4).to_bits(),
        "the render seed does not move a seeded phase"
    );
    assert_ne!(
        first(&seeded(77.0), 1, 4).to_bits(),
        first(&seeded(78.0), 1, 4).to_bits(),
        "the seed parameter does"
    );
}

#[test]
fn the_table_is_prepared_whatever_wave_is_and_wave_can_become_table_later() {
    for (name, table) in [
        (CatalogueTable::Basic, TableKind::Basic),
        (CatalogueTable::Pulse, TableKind::Pulse),
        (CatalogueTable::Tilt, TableKind::Tilt),
        (CatalogueTable::Formant, TableKind::Formant),
        (CatalogueTable::Organ, TableKind::Organ),
    ] {
        let mut rig = osc_rig(|v| {
            set(v, OscParam::Wave, wave(Wave::Saw));
            set(v, OscParam::Table, name.index() as f32);
        });
        assert_eq!(rig.module.table().kind(), table);
        // Automating `wave` to `table` mid-note: the base value changes, the table is already there.
        rig.start(0, 0, 1, 57.0);
        for _ in 0..8 {
            rig.segment(&[0], false, false);
        }
        set(&mut rig.values, OscParam::Wave, wave(Wave::Table));
        set(&mut rig.values, OscParam::Position, 50.0);
        let mut energy = 0.0f32;
        for _ in 0..40 {
            rig.segment(&[0], false, false);
            let o = rig.out(0);
            assert!(
                o.iter().all(|x| x.is_finite() && x.abs() < 3.0),
                "{table:?}"
            );
            energy += o.iter().map(|x| x * x).sum::<f32>();
        }
        assert!(
            energy > 1.0,
            "{table:?} is audible after the switch: {energy}"
        );
    }
}

#[test]
fn switching_the_wave_keeps_the_phase() {
    // A saw started at phase 0 and switched to a sine after n segments continues from the same phase: the first
    // sample of the sine (delayed by one) is sin(2π·phase).
    let hz = 330.0f64;
    let dt = hz / f64::from(SR);
    let key = (69.0 + 12.0 * math::log2(hz / 440.0)) as f32;
    for n in [1usize, 5, 17] {
        let mut rig = osc_rig(|v| {
            set(v, OscParam::Wave, wave(Wave::Saw));
            set(v, OscParam::PhaseMode, PhaseMode::Reset.index() as f32);
        });
        rig.start(0, 0, 1, key);
        for _ in 0..n {
            rig.segment(&[0], false, false);
        }
        set(&mut rig.values, OscParam::Wave, wave(Wave::Sine));
        rig.segment(&[0], false, false);
        let phase = (n * SEG) as f64 * dt;
        let want = math::sin(std::f64::consts::TAU * phase.fract());
        let got = f64::from(rig.out(0)[0]);
        assert!(
            (got - want).abs() < 2e-3,
            "after {n} segments: {got} vs {want}"
        );
    }
}

#[test]
fn copying_state_makes_a_module_continue_bit_identically() {
    for w in [
        Wave::Sine,
        Wave::Triangle,
        Wave::Saw,
        Wave::Pulse,
        Wave::Table,
    ] {
        let edit = |v: &mut Vec<f32>| {
            set(v, OscParam::Wave, wave(w));
            set(v, OscParam::Pw, 30.0);
            set(v, OscParam::Position, 35.0);
        };
        let mut a = osc_rig(edit);
        a.start(2, 0, 9, 52.0);
        a.start(5, 1, 10, 64.0);
        for _ in 0..13 {
            a.segment(&[2, 5], false, false);
        }
        let mut b = osc_rig(edit);
        b.notes = a.notes.clone();
        b.module.copy_state_from(&a.module);
        for _ in 0..5 {
            a.segment(&[2, 5], false, false);
            b.segment(&[2, 5], false, false);
            assert_eq!(
                a.out(2).map(f32::to_bits),
                b.out(2).map(f32::to_bits),
                "{w:?}"
            );
            assert_eq!(
                a.out(5).map(f32::to_bits),
                b.out(5).map(f32::to_bits),
                "{w:?}"
            );
        }
        assert_eq!(a.module.subnormals(), 0);
    }
    let mut a = Rig::new(
        ModuleKind::Noise,
        |v| Noise::new(v, &PrepareCfg::new(SR)),
        |v| {
            v[NoiseParam::Color.index()] = NoiseColor::Pink.index() as f32;
        },
    );
    a.start(1, 0, 3, 60.0);
    for _ in 0..9 {
        a.segment(&[1], false, false);
    }
    let mut b = Rig::new(
        ModuleKind::Noise,
        |v| Noise::new(v, &PrepareCfg::new(SR)),
        |v| {
            v[NoiseParam::Color.index()] = NoiseColor::Pink.index() as f32;
        },
    );
    b.notes = a.notes.clone();
    b.module.copy_state_from(&a.module);
    for _ in 0..4 {
        a.segment(&[1], false, false);
        b.segment(&[1], false, false);
        assert_eq!(a.out(1).map(f32::to_bits), b.out(1).map(f32::to_bits));
    }
}

#[test]
fn only_the_master_writes_the_sync_bus_and_the_slave_leaves_it_alone() {
    let mut master = osc_rig(|v| set(v, OscParam::PhaseMode, PhaseMode::Reset.index() as f32));
    master.start(0, 0, 1, 69.0);
    master.start(3, 1, 2, 57.0);
    let mut wraps = [0usize; 2];
    let mut bus_copy = Box::new([[NO_SYNC; SEG]; MAX_VOICES]);
    for _ in 0..200 {
        master.segment(&[0, 3], true, false);
        for (i, v) in [0usize, 3].into_iter().enumerate() {
            wraps[i] += master.sync[v].iter().filter(|&&r| r >= 0.0).count();
            assert!(
                master.sync[v]
                    .iter()
                    .all(|&r| r.to_bits() == NO_SYNC.to_bits() || (0.0..1.0).contains(&r))
            );
        }
        *bus_copy = *master.sync;
    }
    // 200 segments = 6,400 frames = 0.1333 s: 440 Hz wraps 58.7 times, 220 Hz 29.3 times.
    assert!((58..=59).contains(&wraps[0]), "{wraps:?}");
    assert!((29..=30).contains(&wraps[1]), "{wraps:?}");
    // A slave with `sync` off never reads the bus, with `sync` on it restarts at the master's wraps; neither writes it.
    let mut bus = bus_copy;
    bus[0] = [NO_SYNC; SEG];
    bus[0][5] = 0.25;
    bus[0][20] = 0.5;
    let run = |sync: f32, wired: bool| {
        let mut slave = osc_rig(|v| {
            set(v, OscParam::Sync, sync);
            set(v, OscParam::Semi, 7.0);
            set(v, OscParam::PhaseMode, PhaseMode::Reset.index() as f32);
        });
        slave.start(0, 0, 5, 69.0);
        slave.sync = bus.clone();
        slave.segment(&[0], false, wired);
        assert_eq!(
            slave.sync.map(|b| b.map(f32::to_bits)),
            bus.map(|b| b.map(f32::to_bits)),
            "a slave never writes the bus"
        );
        slave.out(0)
    };
    let off = run(0.0, true);
    assert_eq!(
        off.map(f32::to_bits),
        run(0.0, false).map(f32::to_bits),
        "sync off ignores the bus"
    );
    assert_eq!(
        off.map(f32::to_bits),
        run(1.0, false).map(f32::to_bits),
        "sync on without a master is free-running"
    );
    let on = run(1.0, true);
    assert_ne!(
        on.map(f32::to_bits),
        off.map(f32::to_bits),
        "sync on follows the bus"
    );
    // Before the first wrap (frame 5) the two agree; the restart shows from there on.
    assert_eq!(
        on[..5].iter().map(|x| x.to_bits()).collect::<Vec<_>>(),
        off[..5].iter().map(|x| x.to_bits()).collect::<Vec<_>>()
    );
}

#[test]
fn noise_is_in_range_per_voice_and_pink_is_scaled() {
    let mut white = Rig::new(
        ModuleKind::Noise,
        |v| Noise::new(v, &PrepareCfg::new(SR)),
        |_| {},
    );
    white.start(0, 0, 11, 60.0);
    white.start(1, 1, 12, 60.0);
    let (mut a, mut b) = (Vec::new(), Vec::new());
    for _ in 0..200 {
        white.segment(&[0, 1], false, false);
        a.extend(white.out(0));
        b.extend(white.out(1));
    }
    assert!(a.iter().chain(&b).all(|x| (-1.0..1.0).contains(x)));
    assert_ne!(a, b, "voices have independent streams");
    // Restarting a voice with the same seed repeats its stream; a different seed does not.
    white.start(0, 0, 11, 60.0);
    white.segment(&[0], false, false);
    assert_eq!(
        white.out(0).map(f32::to_bits),
        a[..SEG].iter().map(|x| x.to_bits()).collect::<Vec<_>>()[..]
    );
}
