//! Integration tests of the reverb core (P1a lane FX1): bit-exactness against the upstream C++, no allocation in
//! `process`, no clicks on a room-size change.
//!
//! This file is its own test binary on purpose: the `assert_no_alloc` allocator below is the global allocator of
//! every test in it, and a second `#[global_allocator]` anywhere else in `enth-dsp`'s tests would not link.
//!
//! The scenarios in `fixtures/reverb-scenarios.txt` are rendered here and, with the same text file as input, by the
//! C++ harness kept in the FX1 scratch directory (see `docs/dsp/reverb.md`). `fixtures/reverb-cxx-sha256.txt`
//! holds the SHA-256 of the C++ output. The same test binary runs natively and, built for `wasm32-wasip1`, under
//! wasmtime, so passing on both means native = wasm = C++.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test code: a failed parse or lookup is a failed test"
)]

use enth_dsp::reverb::{Reverb, ReverbParams};

#[cfg(debug_assertions)]
#[global_allocator]
static ALLOCATOR: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;

/// `assert_no_alloc::assert_no_alloc` in debug builds (release builds lack the allocator hook and run the closure).
fn no_alloc<T>(f: impl FnOnce() -> T) -> T {
    assert_no_alloc::assert_no_alloc(f)
}

// ---------------------------------------------------------------------------------------------------------------
// SHA-256 (FIPS 180-4), so the fixtures are plain `sha256sum` digests of the C++ output files.

fn sha256(data: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&((data.len() as u64) * 8).to_be_bytes());
    for chunk in msg.as_chunks::<64>().0 {
        let mut w = [0u32; 64];
        for (i, word) in chunk.as_chunks::<4>().0.iter().enumerate() {
            w[i] = u32::from_be_bytes(*word);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (slot, v) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
            *slot = slot.wrapping_add(v);
        }
    }
    let mut out = [0u8; 32];
    for (i, word) in h.iter().enumerate() {
        out[4 * i..4 * i + 4].copy_from_slice(&word.to_be_bytes());
    }
    out
}

fn hex(bytes: &[u8]) -> String {
    use core::fmt::Write;
    bytes.iter().fold(String::new(), |mut s, b| {
        write!(s, "{b:02x}").expect("writing to a String");
        s
    })
}

#[test]
fn sha256_known_answers() {
    assert_eq!(
        hex(&sha256(b"")),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    assert_eq!(
        hex(&sha256(b"abc")),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert_eq!(
        hex(&sha256(
            b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"
        )),
        "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
    );
}

// ---------------------------------------------------------------------------------------------------------------
// The scenario files.

struct Scenario {
    name: String,
    sample_rate: u32,
    block: usize,
    seconds: usize,
    init: Vec<(String, f64)>,
    changes: Vec<(u64, Vec<(String, f64)>)>,
}

fn parse_kv<'a>(tokens: impl Iterator<Item = &'a str>) -> Vec<(String, f64)> {
    tokens
        .map(|t| {
            let (k, v) = t.split_once('=').expect("key=value");
            (k.to_owned(), v.parse().expect("number"))
        })
        .collect()
}

fn parse_scenarios(text: &str) -> Vec<Scenario> {
    let mut all: Vec<Scenario> = Vec::new();
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("");
        let mut tokens = line.split_whitespace();
        match tokens.next() {
            None => {}
            Some("scenario") => all.push(Scenario {
                name: tokens.next().unwrap().to_owned(),
                sample_rate: tokens.next().unwrap().parse().unwrap(),
                block: tokens.next().unwrap().parse().unwrap(),
                seconds: tokens.next().unwrap().parse().unwrap(),
                init: Vec::new(),
                changes: Vec::new(),
            }),
            Some("set") => all.last_mut().unwrap().init = parse_kv(tokens),
            Some("change") => {
                let ms = tokens.next().unwrap().parse().unwrap();
                all.last_mut().unwrap().changes.push((ms, parse_kv(tokens)));
            }
            Some(other) => panic!("unknown directive {other}"),
        }
    }
    all
}

fn apply(p: &mut ReverbParams, key: &str, v: f64) {
    match key {
        "dry_pct" => p.dry_pct = v,
        "wet_pct" => p.wet_pct = v,
        "room_ms" => p.room_ms = v,
        "rt20_s" => p.rt20_s = v,
        "early_pct" => p.early_pct = v,
        "detune" => p.detune = v,
        "low_cut_hz" => p.low_cut_hz = v,
        "high_cut_hz" => p.high_cut_hz = v,
        "low_damp_rate" => p.low_damp_rate = v,
        "high_damp_rate" => p.high_damp_rate = v,
        other => panic!("unknown parameter {other}"),
    }
}

/// `SplitMix64`, the generator of the C++ harness.
struct SplitMix(u64);

impl SplitMix {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn unit(&mut self) -> f32 {
        (self.next() >> 40) as f32 / 16_777_216.0
    }
}

/// The harness input: a noise burst in the first 250 ms and an impulse (1.0, -0.5) at 1.5 s.
fn scenario_input(sample_rate: u32, frames: usize) -> (Vec<f32>, Vec<f32>) {
    let sr = sample_rate as usize;
    let mut rng = SplitMix(12345);
    let (mut left, mut right) = (vec![0.0f32; frames], vec![0.0f32; frames]);
    for i in 0..frames {
        let (mut l, mut r) = (0.0f32, 0.0f32);
        if i < sr / 4 {
            l = (rng.unit() - 0.5) * 0.5;
            r = (rng.unit() - 0.5) * 0.5;
        }
        if i == sr * 3 / 2 {
            l = 1.0;
            r = -0.5;
        }
        left[i] = l;
        right[i] = r;
    }
    (left, right)
}

/// Render a scenario exactly the way the C++ harness does: parameters before the first block, changes before the
/// first block starting at or after their frame, blocks of `block` frames with a short last one.
fn render_scenario(s: &Scenario) -> Vec<f32> {
    let sr = s.sample_rate as usize;
    let frames = sr * s.seconds;
    let mut params = ReverbParams::default();
    for (k, v) in &s.init {
        apply(&mut params, k, *v);
    }
    let mut reverb = Reverb::prepare(s.sample_rate as f32);
    reverb.set_params(&params);
    let (in_l, in_r) = scenario_input(s.sample_rate, frames);
    let mut out = vec![0.0f32; 2 * frames];
    let (mut out_l, mut out_r) = (vec![0.0f32; s.block], vec![0.0f32; s.block]);
    let mut done = vec![false; s.changes.len()];
    let mut pos = 0;
    while pos < frames {
        for (c, (ms, kv)) in s.changes.iter().enumerate() {
            let at = sr as u64 * ms / 1000;
            if !done[c] && pos as u64 >= at {
                for (k, v) in kv {
                    apply(&mut params, k, *v);
                }
                reverb.set_params(&params);
                done[c] = true;
            }
        }
        let n = s.block.min(frames - pos);
        reverb.process(
            &in_l[pos..pos + n],
            &in_r[pos..pos + n],
            &mut out_l[..n],
            &mut out_r[..n],
        );
        for i in 0..n {
            out[2 * (pos + i)] = out_l[i];
            out[2 * (pos + i) + 1] = out_r[i];
        }
        pos += n;
    }
    out
}

fn output_sha256(out: &[f32]) -> String {
    let bytes: Vec<u8> = out.iter().flat_map(|x| x.to_le_bytes()).collect();
    hex(&sha256(&bytes))
}

const SCENARIOS: &str = include_str!("fixtures/reverb-scenarios.txt");
const CXX_HASHES: &str = include_str!("fixtures/reverb-cxx-sha256.txt");

/// The tentpole: every scenario's output is bit-identical to upstream's C++ reverb (SHA-256 of the C++ output).
/// Running this same test built for wasm32-wasip1 under wasmtime proves native = wasm.
#[test]
fn bit_exact_against_the_upstream_cxx_reference() {
    let scenarios = parse_scenarios(SCENARIOS);
    let expected: Vec<(&str, &str)> = CXX_HASHES
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.split_once(' ').expect("name hash"))
        .collect();
    assert_eq!(
        scenarios
            .iter()
            .map(|s| s.name.as_str())
            .collect::<Vec<_>>(),
        expected.iter().map(|e| e.0).collect::<Vec<_>>(),
        "scenarios and fixture hashes must list the same names in the same order"
    );
    let mut failures = Vec::new();
    for (s, (name, want)) in scenarios.iter().zip(&expected) {
        let got = output_sha256(&render_scenario(s));
        println!("{name} {got}");
        if got != *want {
            failures.push(format!("{name}: rust {got}, C++ {want}"));
        }
    }
    assert!(
        failures.is_empty(),
        "differs from the C++ reference:\n{}",
        failures.join("\n")
    );
}

#[test]
fn the_scenarios_cover_the_interesting_paths() {
    let scenarios = parse_scenarios(SCENARIOS);
    assert!(scenarios.len() >= 5);
    // Mid-render changes in every scenario, a block that is not a power of two, a short last block, three rates.
    assert!(scenarios.iter().all(|s| !s.changes.is_empty()));
    assert!(
        scenarios.iter().any(|s| s.block == 32),
        "the engine's segment"
    );
    assert!(scenarios.iter().any(|s| s.block.count_ones() != 1));
    assert!(
        scenarios
            .iter()
            .any(|s| !(s.sample_rate as usize * s.seconds).is_multiple_of(s.block))
    );
    let mut rates: Vec<u32> = scenarios.iter().map(|s| s.sample_rate).collect();
    rates.sort_unstable();
    rates.dedup();
    assert!(rates.len() >= 3, "{rates:?}");
}

// ---------------------------------------------------------------------------------------------------------------
// In place, allocation, clicks.

fn noise(frames: usize, seed: u64, amp: f32) -> (Vec<f32>, Vec<f32>) {
    let mut rng = SplitMix(seed);
    let l = (0..frames)
        .map(|_| (rng.unit() - 0.5) * 2.0 * amp)
        .collect();
    let r = (0..frames)
        .map(|_| (rng.unit() - 0.5) * 2.0 * amp)
        .collect();
    (l, r)
}

#[test]
fn in_place_gives_the_same_bits_as_separate_buffers() {
    let params = ReverbParams {
        room_ms: 120.0,
        detune: 10.0,
        ..ReverbParams::default()
    };
    let (in_l, in_r) = noise(32 * 600, 7, 0.5);
    let mut a = Reverb::prepare(48_000.0);
    let mut b = Reverb::prepare(48_000.0);
    a.set_params(&params);
    b.set_params(&params);
    let (mut out_l, mut out_r) = (vec![0.0; in_l.len()], vec![0.0; in_l.len()]);
    let (mut p_l, mut p_r) = (in_l.clone(), in_r.clone());
    for k in 0..600 {
        let r = k * 32..(k + 1) * 32;
        if k == 300 {
            let changed = ReverbParams {
                room_ms: 40.0,
                wet_pct: 90.0,
                ..params
            };
            a.set_params(&changed);
            b.set_params(&changed);
        }
        a.process(
            &in_l[r.clone()],
            &in_r[r.clone()],
            &mut out_l[r.clone()],
            &mut out_r[r.clone()],
        );
        b.process_in_place(&mut p_l[r.clone()], &mut p_r[r]);
    }
    assert!(
        out_l
            .iter()
            .zip(&p_l)
            .all(|(x, y)| x.to_bits() == y.to_bits())
    );
    assert!(
        out_r
            .iter()
            .zip(&p_r)
            .all(|(x, y)| x.to_bits() == y.to_bits())
    );
    assert!(out_l.iter().any(|x| *x != 0.0));
}

/// `process`, `set_params` and `reset` allocate nothing, whatever the block length and however often the
/// parameters change (a fade start redraws 48 offsets and recomputes filters: all in place).
#[test]
fn no_allocation_in_process_set_params_or_reset() {
    let mut reverb = Reverb::prepare(48_000.0);
    let (in_l, in_r) = noise(4096, 3, 0.5);
    let (mut out_l, mut out_r) = (vec![0.0f32; 4096], vec![0.0f32; 4096]);
    let sets = [
        ReverbParams::default(),
        ReverbParams {
            room_ms: 190.0,
            rt20_s: 20.0,
            detune: 40.0,
            low_cut_hz: 400.0,
            ..ReverbParams::default()
        },
        ReverbParams {
            room_ms: 12.0,
            rt20_s: 0.02,
            high_cut_hz: 2_000.0,
            wet_pct: 300.0,
            ..ReverbParams::default()
        },
        ReverbParams {
            room_ms: f64::NAN, // clamped, not a panic
            rt20_s: f64::INFINITY,
            ..ReverbParams::default()
        },
    ];
    no_alloc(|| {
        for round in 0..4 {
            for (k, set) in sets.iter().enumerate() {
                reverb.set_params(set);
                for block in [1usize, 7, 32, 64, 100, 960, 1024] {
                    reverb.process(
                        &in_l[..block],
                        &in_r[..block],
                        &mut out_l[..block],
                        &mut out_r[..block],
                    );
                }
                reverb.process_in_place(&mut out_l[..32], &mut out_r[..32]);
                reverb.process(&in_l, &in_r, &mut out_l, &mut out_r);
                if (round + k) % 3 == 0 {
                    reverb.reset();
                }
            }
        }
    });
    assert!(out_l.iter().chain(&out_r).all(|x| x.is_finite()));
}

/// Largest absolute second difference of `y` at the samples `range` (`y[n - 1]`, `y[n]` and `y[n + 1]` are read).
fn max_second_difference(y: &[f32], range: core::ops::Range<usize>) -> f32 {
    range
        .map(|n| (y[n + 1] - 2.0 * y[n] + y[n - 1]).abs())
        .fold(0.0, f32::max)
}

/// The three windows of the click test around a parameter change at frame `change_at`.
struct ClickReport {
    /// The steady sine before the change.
    before: f32,
    /// From the change to 100 ms later, which contains the 20 ms cross-fade (and the junction, if the signal was
    /// spliced there).
    during: f32,
    /// Half a second of settled signal 1.5 s after the change.
    settled: f32,
}

impl ClickReport {
    fn of(y: &[f32], change_at: usize, sample_rate: usize) -> Self {
        Self {
            before: max_second_difference(y, change_at - sample_rate / 2..change_at - 2),
            during: max_second_difference(y, change_at - 1..change_at + sample_rate / 10),
            settled: max_second_difference(
                y,
                change_at + 3 * sample_rate / 2..change_at + 2 * sample_rate - 2,
            ),
        }
    }

    /// The change's worst second difference against the larger of the two steady states around it. A new room
    /// changes the tail's level, so the settled value may legitimately differ from the old one; a click would
    /// stand out against both.
    fn ratio(&self) -> f32 {
        self.during / self.before.max(self.settled)
    }
}

/// Feed a 440 Hz sine through the wet path only, change the room size (and in one case the decay too) while it
/// plays, and compare the largest second difference around the change with the steady states before and after: a
/// click would show as a spike many times larger. Upstream's cross-fade keeps the ratio near 1 to 1.5 (P1a report
/// 02); splicing the output of two rooms together without a fade is the negative control and must read far above
/// the bound. (Not covered, by design: the low-cut, high-cut and damping filters switch coefficients at once at the
/// next block, as upstream does, so a large jump of those steps the output; see `docs/dsp/reverb.md`.)
#[test]
fn room_size_change_has_no_clicks_and_allocates_nothing() {
    const SR: usize = 48_000;
    const BLOCK: usize = 32;
    const SECONDS: usize = 4;
    const BOUND: f32 = 2.0;
    let wet_only = ReverbParams {
        dry_pct: 0.0,
        wet_pct: 100.0,
        ..ReverbParams::default()
    };
    // A sine with a raised-cosine onset over 0.3 s, so the start-up transient (broadband, and still ringing a second
    // later) does not pollute the "steady" windows.
    let onset = SR * 3 / 10;
    let sine: Vec<f32> = (0..SR * SECONDS)
        .map(|n| {
            let ramp = if n < onset {
                0.5 - 0.5 * libm::cos(core::f64::consts::PI * n as f64 / onset as f64)
            } else {
                1.0
            };
            (0.5 * ramp * libm::sin(core::f64::consts::TAU * 440.0 * n as f64 / SR as f64)) as f32
        })
        .collect();
    let change_at = SR * 2;

    let render = |first: &ReverbParams, second: &ReverbParams| -> Vec<f32> {
        let mut reverb = Reverb::prepare(SR as f32);
        reverb.set_params(first);
        let mut out = vec![0.0f32; SR * SECONDS];
        let mut scratch = vec![0.0f32; BLOCK];
        for start in (0..SR * SECONDS).step_by(BLOCK) {
            let end = start + BLOCK;
            no_alloc(|| {
                if start == change_at {
                    reverb.set_params(second);
                }
                reverb.process(
                    &sine[start..end],
                    &sine[start..end],
                    &mut out[start..end],
                    &mut scratch,
                );
            });
        }
        out
    };

    let room = |ms: f64| ReverbParams {
        room_ms: ms,
        ..wet_only
    };
    let cases: [(&str, ReverbParams, ReverbParams); 5] = [
        ("room 80 -> 150", wet_only, room(150.0)),
        ("room 80 -> 20", wet_only, room(20.0)),
        ("room 10 -> 200", room(10.0), room(200.0)),
        ("room 200 -> 10", room(200.0), room(10.0)),
        (
            "room and decay together",
            wet_only,
            ReverbParams {
                room_ms: 140.0,
                rt20_s: 4.0,
                ..wet_only
            },
        ),
    ];
    for (name, first, second) in &cases {
        let report = ClickReport::of(&render(first, second), change_at, SR);
        println!(
            "{name}: before {:.5}, during {:.5}, settled {:.5}, ratio {:.2}",
            report.before,
            report.during,
            report.settled,
            report.ratio()
        );
        assert!(
            report.before > 1e-5,
            "{name}: the wet signal must be audible"
        );
        assert!(
            report.ratio() < BOUND,
            "{name}: the change reads {:.2} x the steady value",
            report.ratio()
        );
    }

    // Negative control: the two rooms' renders spliced at the change instant, with no cross-fade.
    let (a, b) = (
        render(&cases[0].1, &cases[0].1),
        render(&cases[0].2, &cases[0].2),
    );
    let mut spliced = a;
    spliced[change_at..].copy_from_slice(&b[change_at..]);
    let report = ClickReport::of(&spliced, change_at, SR);
    println!(
        "hard splice: before {:.5}, during {:.5}, settled {:.5}, ratio {:.2}",
        report.before,
        report.during,
        report.settled,
        report.ratio()
    );
    assert!(
        report.ratio() > 10.0 * BOUND,
        "the metric must be able to see a click"
    );
}
