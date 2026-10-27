//! A harness for the paired kernels of the per-voice effects (lane PRF): `VoiceProcessor::process_pair` on a module
//! and its right twin must give, bit for bit, what `process` on each gives (outputs and the whole state). The
//! harness runs four instances of one kind, two of them paired and two plain, over several voices and segments of
//! different signals, with the parameters edited by the caller before every segment.

use core::fmt::Debug;

use enth_dsp::util::rng::Rng;
use enth_patch::ModuleKind;

use crate::module::{
    MAX_VOICES, NoteState, SEG, VoiceBus, VoiceCx, VoiceParams, VoiceProcessor, VoiceStart,
};

/// One instance's parameter tables: base values, this segment's and the previous segment's per-voice values.
pub(crate) struct Tables {
    pub values: Vec<f32>,
    pub cur: Vec<[f32; MAX_VOICES]>,
    pub prev: Vec<[f32; MAX_VOICES]>,
}

impl Tables {
    fn new(kind: ModuleKind) -> Self {
        let values = kind.defaults();
        let cur: Vec<[f32; MAX_VOICES]> = values.iter().map(|&v| [v; MAX_VOICES]).collect();
        Self {
            prev: cur.clone(),
            values,
            cur,
        }
    }

    /// Set parameter `index` to `f(voice)` for this segment (and the base value to voice 0's).
    pub fn set(&mut self, index: usize, f: impl Fn(usize) -> f32) {
        for v in 0..MAX_VOICES {
            self.cur[index][v] = f(v);
        }
        self.values[index] = self.cur[index][0];
    }

    /// Set parameter `index` to `value` for every voice.
    pub fn all(&mut self, index: usize, value: f32) {
        self.set(index, |_| value);
    }
}

/// One of the four instances.
struct Inst<M> {
    module: M,
    tables: Tables,
    io: Box<VoiceBus>,
}

/// Run `segments` segments of a left and a right instance of `kind`, paired and plain, on `voices`, and compare
/// every output sample and each instance's `Debug` text (all of its state: `Debug` prints the sign of a zero) after
/// every segment. `edit(segment, right, tables)` sets the parameters of an instance before a segment (`right` tells
/// the twin, whose parameters a stereo plan keeps equal to the left's; a test may make them differ to cover the
/// fallback). `restart(segment)` lists the `(voice, right)` pairs to restart before a segment.
pub(crate) fn check<M: VoiceProcessor + Debug>(
    kind: ModuleKind,
    new: impl Fn(&[f32]) -> M,
    voices: &[usize],
    segments: usize,
    edit: impl Fn(usize, bool, &mut Tables),
    restart: impl Fn(usize) -> Vec<(usize, bool)>,
) {
    let make = || {
        let tables = Tables::new(kind);
        Inst {
            module: new(&tables.values),
            tables,
            io: Box::new([[0.0; SEG]; MAX_VOICES]),
        }
    };
    // Paired left and right, plain left and right.
    let (mut pl, mut pr, mut ql, mut qr) = (make(), make(), make(), make());
    let mut notes = NoteState::new();
    for &v in voices {
        notes.gate[v] = true;
        notes.pitch_cur[v] = 40.0 + 3.0 * v as f32;
        notes.pitch_prev[v] = notes.pitch_cur[v];
        notes.velocity[v] = 0.8;
    }
    let start = |inst: &mut Inst<M>, v: usize| {
        let params =
            VoiceParams::from_slices(&inst.tables.values, &inst.tables.cur, &inst.tables.prev);
        inst.module.start_voice(&VoiceStart {
            voice: v,
            seed: 7 + v as u64,
            module_key: 1,
            note_index: 0,
            notes: &notes,
            params,
        });
    };
    for &v in voices {
        for inst in [&mut pl, &mut pr, &mut ql, &mut qr] {
            start(inst, v);
        }
    }
    let mut rng = Rng::new(0x9A12_0001);
    for seg in 0..segments {
        for (v, right) in restart(seg) {
            let (a, b) = if right {
                (&mut pr, &mut qr)
            } else {
                (&mut pl, &mut ql)
            };
            start(a, v);
            start(b, v);
        }
        for (inst, right) in [
            (&mut pl, false),
            (&mut pr, true),
            (&mut ql, false),
            (&mut qr, true),
        ] {
            inst.tables.prev.clone_from(&inst.tables.cur);
            edit(seg, right, &mut inst.tables);
        }
        // Signals: a different one per voice and channel, loud enough in some segments to drive nonlinearities.
        let amp = [0.2f32, 1.0, 3.0, 0.0, 0.6][seg % 5];
        for &v in voices {
            let l: [f32; SEG] = core::array::from_fn(|_| amp * rng.bipolar_f32());
            let r: [f32; SEG] = core::array::from_fn(|_| amp * rng.bipolar_f32());
            pl.io[v] = l;
            ql.io[v] = l;
            pr.io[v] = r;
            qr.io[v] = r;
        }
        let params_l = VoiceParams::from_slices(&pl.tables.values, &pl.tables.cur, &pl.tables.prev);
        let params_r = VoiceParams::from_slices(&pr.tables.values, &pr.tables.cur, &pr.tables.prev);
        let mut lcx = VoiceCx {
            voices,
            notes: &notes,
            params: params_l,
            io: &mut pl.io,
            sync_out: None,
            sync_in: None,
            audio_in: None,
            pre_level: None,
            io_r: None,
        };
        let mut rcx = VoiceCx {
            voices,
            notes: &notes,
            params: params_r,
            io: &mut pr.io,
            sync_out: None,
            sync_in: None,
            audio_in: None,
            pre_level: None,
            io_r: None,
        };
        pl.module.process_pair(&mut pr.module, &mut lcx, &mut rcx);
        for inst in [&mut ql, &mut qr] {
            let params =
                VoiceParams::from_slices(&inst.tables.values, &inst.tables.cur, &inst.tables.prev);
            inst.module.process(&mut VoiceCx {
                voices,
                notes: &notes,
                params,
                io: &mut inst.io,
                sync_out: None,
                sync_in: None,
                audio_in: None,
                pre_level: None,
                io_r: None,
            });
        }
        for &v in voices {
            for (name, a, b) in [("left", &pl, &ql), ("right", &pr, &qr)] {
                let (x, y) = (a.io[v].map(f32::to_bits), b.io[v].map(f32::to_bits));
                assert_eq!(x, y, "{name} output, voice {v}, segment {seg}");
            }
        }
        assert_eq!(
            format!("{:?}", pl.module),
            format!("{:?}", ql.module),
            "left state, segment {seg}"
        );
        assert_eq!(
            format!("{:?}", pr.module),
            format!("{:?}", qr.module),
            "right state, segment {seg}"
        );
    }
}
