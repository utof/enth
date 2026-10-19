//! Zero subnormals reach the output or the state, even through a 40 s release tail, and the test can fail:
//! the same render with flushing disabled produces them. (Instruction counts cannot see this problem: a
//! naive engine costs 2x the instructions but 36x the wall time, so this test is half of the CI gate.)

use spike_engine::render::{fast_decay_patch, frames, tail_notes};
use spike_engine::{BLOCK, Engine, Flush, Note, Patch, Path};

struct Scan {
    output: usize,
    state_max: usize,
    active_at_end: usize,
}

fn scan(patch: &Patch, notes: &[Note], seconds: f32, path: Path, flush: Flush) -> Scan {
    let mut engine = Engine::new(patch, notes, 7, path, flush);
    let mut out = vec![0.0f32; frames(seconds)];
    let mut s = Scan {
        output: 0,
        state_max: 0,
        active_at_end: 0,
    };
    for (i, block) in out.chunks_mut(BLOCK).enumerate() {
        engine.process(block);
        s.output += block.iter().filter(|x| x.is_subnormal()).count();
        if i % 16 == 0 {
            s.state_max = s.state_max.max(engine.count_subnormals());
        }
    }
    s.state_max = s.state_max.max(engine.count_subnormals());
    s.active_at_end = engine.active_voices();
    s
}

#[test]
fn flushed_engine_has_no_subnormals_in_a_40s_tail() {
    let cases = [
        ("p1 tail, scalar", Patch::p1_like(), Path::Scalar),
        ("p1 tail, x8 lanes", Patch::p1_like(), Path::X8),
        ("fast-decay tail, scalar", fast_decay_patch(), Path::Scalar),
        ("simple template tail, x4 lanes", Patch::simple(), Path::X4),
    ];
    for (name, patch, path) in cases {
        let s = scan(&patch, &tail_notes(), 40.0, path, Flush::On);
        println!(
            "flush on, 40 s, {name}: subnormal output samples {}, max subnormal state values {}, voices left {}",
            s.output, s.state_max, s.active_at_end
        );
        assert_eq!(s.output, 0, "{name}: subnormal output samples");
        assert_eq!(s.state_max, 0, "{name}: subnormal values in engine state");
        assert_eq!(s.active_at_end, 0, "{name}: voices must end below -100 dB");
    }
}

#[test]
fn random_patches_stay_normal_through_a_long_tail() {
    for seed in 0..12u64 {
        let patch = Patch::from_seed(seed);
        let s = scan(&patch, &tail_notes(), 30.0, Path::Scalar, Flush::On);
        assert_eq!((s.output, s.state_max), (0, 0), "seed {seed}");
    }
}

/// The negative control: without flushing the same render hits the subnormal range, so the assertions above can fail.
#[test]
fn unflushed_engine_does_produce_subnormals() {
    let s = scan(
        &fast_decay_patch(),
        &tail_notes(),
        12.0,
        Path::Scalar,
        Flush::Off,
    );
    println!(
        "flush OFF, 12 s fast-decay tail: subnormal output samples {}, max subnormal state values {}, voices left {}",
        s.output, s.state_max, s.active_at_end
    );
    assert!(
        s.output > 1000,
        "expected many subnormal output samples, got {}",
        s.output
    );
    assert!(s.state_max > 0, "expected subnormal state");
    assert_eq!(
        s.active_at_end, 4,
        "without the -100 dB rule voices never end"
    );
}
