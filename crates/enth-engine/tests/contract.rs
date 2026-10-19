//! Contracts later lanes keep, tested through the public module API (the way a lane unit-tests its own module).

use enth_engine::module::{
    MAX_VOICES, NoteState, PrepareCfg, VoiceModCx, VoiceModulator, VoiceParams, VoiceStart,
};
use enth_engine::modules::Adsr;
use enth_patch::ModuleKind;
use enth_patch::catalogue::AdsrParam;

/// The golden convention (`docs/dsp/engine.md`): module goldens play through a *gate envelope* (attack 0, decay 0,
/// sustain 100 %, release 0, velocity 0), so they do not move when lane D2 replaces the reference envelope. That
/// holds only if a gate envelope's output is exactly 1.0 from the end of a note's first segment while the key is held
/// and exactly 0.0 one segment after the note-off, at any sample rate. Lane D2's envelope keeps this test green.
#[test]
fn a_gate_envelope_is_exact() {
    let mut values = ModuleKind::Adsr.defaults();
    values[AdsrParam::Attack.index()] = 0.0;
    values[AdsrParam::Decay.index()] = 0.0;
    values[AdsrParam::Sustain.index()] = 100.0;
    values[AdsrParam::Release.index()] = 0.0;
    values[AdsrParam::Velocity.index()] = 0.0;
    let cur: Vec<[f32; MAX_VOICES]> = values.iter().map(|&v| [v; MAX_VOICES]).collect();
    for sr in [8_000.0, 44_100.0, 48_000.0, 96_000.0, 384_000.0] {
        let params = VoiceParams::from_slices(&values, &cur, &cur);
        let mut env = Adsr::new(&values, &PrepareCfg::new(sr));
        let mut notes = NoteState::new();
        let v = 3;
        notes.gate[v] = true;
        notes.trigger[v] = true;
        notes.fresh[v] = true;
        notes.velocity[v] = 0.3;
        notes.pitch_cur[v] = 60.0;
        notes.pitch_prev[v] = 60.0;
        env.start_voice(&VoiceStart {
            voice: v,
            seed: 1,
            module_key: 2,
            note_index: 0,
            notes: &notes,
            params,
        });
        let mut out = [0.5; MAX_VOICES];
        let tick = |env: &mut Adsr, notes: &NoteState, out: &mut [f32; MAX_VOICES]| {
            env.tick(&mut VoiceModCx {
                voices: &[v],
                notes,
                params,
                out,
            });
        };
        for segment in 0..100 {
            tick(&mut env, &notes, &mut out);
            assert_eq!(
                out[v].to_bits(),
                1.0f32.to_bits(),
                "{sr} Hz, held segment {segment}"
            );
            notes.trigger[v] = false;
            notes.fresh[v] = false;
        }
        notes.gate[v] = false;
        notes.release[v] = true;
        tick(&mut env, &notes, &mut out);
        assert_eq!(
            out[v].to_bits(),
            0.0f32.to_bits(),
            "{sr} Hz, one segment after the note-off"
        );
        assert!(env.voice_finished(v), "{sr} Hz: the voice ends");
        notes.release[v] = false;
        tick(&mut env, &notes, &mut out);
        assert_eq!(out[v].to_bits(), 0.0f32.to_bits());
        assert_eq!(VoiceModulator::subnormals(&env), 0);
    }
}
