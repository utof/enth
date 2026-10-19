//! Contracts later lanes keep, tested through the public module API (the way a lane unit-tests its own module).

use enth_engine::module::{
    MAX_VOICES, NoteState, PrepareCfg, VoiceModCx, VoiceModulator, VoiceParams, VoiceStart,
};
use enth_engine::modules::Adsr;
use enth_engine::modules::adsr::min_ramp_steps;
use enth_patch::ModuleKind;
use enth_patch::catalogue::AdsrParam;

/// The golden convention (`docs/dsp/engine.md`): module goldens play through a *gate envelope* (attack 0, decay 0,
/// sustain 100 %, release 0, velocity 0), so they do not move when the envelope's curves change. That holds only if
/// a gate envelope is exact once its minimum ramp is over: lane D2's envelope never jumps (every segment lasts at
/// least 1 ms, rounded up to whole 32-frame segments: `min_ramp_steps(sample_rate)`, 2 at 48 kHz), so the output is
/// strictly between 0 and 1 and rising for the first `n - 1` segments, then **exactly 1.0 from the end of segment
/// `n` while the key is held**, and strictly falling for `n - 1` segments after the note-off, then **exactly 0.0
/// `n` segments after it**, at any sample rate (S2a's reference envelope had `n = 1`).
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
        let n = min_ramp_steps(sr) as usize;
        let mut last = 0.0f32;
        for segment in 0..100 {
            tick(&mut env, &notes, &mut out);
            if segment + 1 < n {
                assert!(
                    out[v] > last && out[v] < 1.0,
                    "{sr} Hz, ramp segment {segment}: {}",
                    out[v]
                );
            } else {
                assert_eq!(
                    out[v].to_bits(),
                    1.0f32.to_bits(),
                    "{sr} Hz, held segment {segment} (the ramp is {n} segments)"
                );
            }
            last = out[v];
            notes.trigger[v] = false;
            notes.fresh[v] = false;
        }
        notes.gate[v] = false;
        notes.release[v] = true;
        for segment in 0..n {
            tick(&mut env, &notes, &mut out);
            if segment + 1 < n {
                assert!(
                    out[v] > 0.0 && out[v] < last,
                    "{sr} Hz, release segment {segment}: {}",
                    out[v]
                );
                assert!(!env.voice_finished(v), "{sr} Hz: still releasing");
                last = out[v];
            } else {
                assert_eq!(
                    out[v].to_bits(),
                    0.0f32.to_bits(),
                    "{sr} Hz, {n} segments after the note-off"
                );
                assert!(env.voice_finished(v), "{sr} Hz: the voice ends");
            }
        }
        tick(&mut env, &notes, &mut out);
        assert_eq!(out[v].to_bits(), 0.0f32.to_bits());
        assert_eq!(VoiceModulator::subnormals(&env), 0);
    }
}
