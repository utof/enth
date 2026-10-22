//! The biquad's golden (lane BQ; docs/dsp/biquad.md; docs/phase-4.md, Acceptance): for a grid of peak, low-shelf and
//! high-shelf sections, a digest of the **coefficients** (the five `f64` bit patterns, so a different `libm` result
//! shows here first) and a digest of the **output** over a fixed input (0.34 s of seeded noise, then the same length of
//! silence, so the decaying tail and its flush are in the hash), one line per case in `fixtures/biquad-golden.txt`,
//! the same on every platform. The file is compiled in, so the wasm run needs no directory:
//!
//! ```text
//! CARGO_TARGET_WASM32_WASIP1_RUNNER=/path/to/wasmtime \
//!   cargo test --release -p enth-dsp --target wasm32-wasip1 --test biquad_golden -- --nocapture
//! ```
//!
//! `golden.yml` runs it on Linux (x86-64 and aarch64), Windows, macOS (arm64), wasm32-wasip1 under wasmtime and the
//! same with wasm `simd128`: the six golden targets. It is a test of `enth-dsp` on purpose (the biquad is not an
//! engine module, so `tests/golden`'s engine cases do not reach it).
//!
//! The cases are the integers an EQ band carries (Hz, centi-dB, thousandths of Q) turned to `f64` the way
//! `enth-challenge` turns them (one IEEE division each), so a question's band and a case here design the same bits.
//!
//! Regenerate only on purpose (a deliberate change to the design or the processing, which is an engine-version
//! change for anything that renders through it), and review the diff:
//! `cargo test -p enth-dsp --test biquad_golden -- --ignored regenerate`.

#![allow(
    clippy::print_stdout,
    reason = "prints the summary hash for the native-vs-wasm comparison"
)]
#![allow(
    clippy::disallowed_methods,
    reason = "only the ignored `regenerate` test writes the golden file; a test, not library code"
)]

use std::fmt::Write as _;

use enth_dsp::biquad::{Biquad, Coeffs, Kind};

const EXPECTED: &str = include_str!("fixtures/biquad-golden.txt");

/// Samples of noise, and of silence after it, per case (the same at every sample rate).
const NOISE: usize = 16_384;

/// The input's seed.
const INPUT_SEED: u64 = 0x0000_4251_5F76_3100;

#[cfg(debug_assertions)]
#[global_allocator]
static ALLOCATOR: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;

/// `assert_no_alloc::assert_no_alloc` in debug builds (release builds lack the allocator hook and run the closure).
fn no_alloc<T>(f: impl FnOnce() -> T) -> T {
    assert_no_alloc::assert_no_alloc(f)
}

// ---------------------------------------------------------------------------------------------------------------
// SHA-256 (FIPS 180-4)

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
        for (slot, add) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
            *slot = slot.wrapping_add(add);
        }
    }
    let mut out = [0u8; 32];
    for (bytes, word) in out.as_chunks_mut::<4>().0.iter_mut().zip(h) {
        *bytes = word.to_be_bytes();
    }
    out
}

fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        write!(s, "{b:02x}").expect("writing to a String");
    }
    s
}

#[test]
fn sha256_matches_the_published_vectors() {
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
// The cases

#[derive(Clone, Copy)]
struct Case {
    kind: Kind,
    sample_rate: u32,
    freq_hz: u32,
    gain_cdb: i32,
    q_milli: u32,
}

impl Case {
    fn name(&self) -> String {
        let kind = match self.kind {
            Kind::Peak => "peak",
            Kind::LowShelf => "low-shelf",
            Kind::HighShelf => "high-shelf",
        };
        format!(
            "{kind}/{}/{}/{}/{}",
            self.sample_rate, self.freq_hz, self.gain_cdb, self.q_milli
        )
    }

    /// The band's integers as `f64`, the way `enth-challenge` converts them.
    fn coeffs(&self) -> Coeffs {
        Coeffs::design(
            self.kind,
            f64::from(self.freq_hz),
            f64::from(self.gain_cdb) / 100.0,
            f64::from(self.q_milli) / 1000.0,
            f64::from(self.sample_rate),
        )
    }
}

fn cases() -> Vec<Case> {
    let mut out = Vec::new();
    // (sample rate, centre or corner): low and high ends, an octave grid at 48 kHz, 44.1 kHz and 96 kHz.
    let places = [
        (48_000, 40),
        (48_000, 250),
        (48_000, 1_000),
        (48_000, 4_000),
        (48_000, 16_000),
        (44_100, 1_000),
        (44_100, 18_000),
        (96_000, 1_000),
        (96_000, 30_000),
    ];
    // (gain in centi-dB, Q in thousandths): boost and cut at Q 1, a narrow big boost, a wide small cut.
    let shapes = [(1_200, 1_000), (-1_200, 1_000), (2_400, 8_000), (-300, 300)];
    for kind in [Kind::Peak, Kind::LowShelf, Kind::HighShelf] {
        for (sample_rate, freq_hz) in places {
            for (gain_cdb, q_milli) in shapes {
                out.push(Case {
                    kind,
                    sample_rate,
                    freq_hz,
                    gain_cdb,
                    q_milli,
                });
            }
        }
        // The ends of the ranges, and a band with no gain (the identity).
        for (freq_hz, gain_cdb, q_milli) in [
            (20, 2_400, 25_000),
            (20_000, -2_400, 100),
            (1_000, 0, 1_000),
        ] {
            out.push(Case {
                kind,
                sample_rate: 48_000,
                freq_hz,
                gain_cdb,
                q_milli,
            });
        }
    }
    out
}

/// `NOISE` samples of seeded noise in `[-0.5, 0.5)` (24 random bits, exact in `f32`), then `NOISE` zeros.
fn input() -> Vec<f32> {
    let mut state = INPUT_SEED;
    let mut v = Vec::with_capacity(2 * NOISE);
    for _ in 0..NOISE {
        // SplitMix64 (Vigna's public-domain constants).
        state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        v.push((z >> 40) as f32 / 16_777_216.0 - 0.5);
    }
    v.resize(2 * NOISE, 0.0);
    v
}

/// The golden text for this build, and the summary hash over every digest.
fn render() -> (String, String) {
    let input = input();
    let mut text = String::new();
    let mut all = Vec::new();
    for case in cases() {
        let coeffs = case.coeffs();
        let mut coeff_bytes = Vec::with_capacity(40);
        for v in [coeffs.b0, coeffs.b1, coeffs.b2, coeffs.a1, coeffs.a2] {
            coeff_bytes.extend_from_slice(&v.to_bits().to_le_bytes());
        }
        let mut buf = input.clone();
        Biquad::new(coeffs).process(&mut buf);
        let mut out_bytes = Vec::with_capacity(buf.len() * 4);
        for v in &buf {
            out_bytes.extend_from_slice(&v.to_bits().to_le_bytes());
        }
        let (c, o) = (sha256(&coeff_bytes), sha256(&out_bytes));
        writeln!(text, "{} {} {}", case.name(), hex(&c[..8]), hex(&o[..8]))
            .expect("writing to a String");
        all.extend_from_slice(&c);
        all.extend_from_slice(&o);
    }
    let sum = hex(&sha256(&all));
    writeln!(text, "sum {sum}").expect("writing to a String");
    (text, sum)
}

#[test]
fn every_case_matches_the_golden() {
    let (text, sum) = render();
    println!("biquad golden: {} cases, sum {sum}", cases().len());
    let want: Vec<&str> = EXPECTED.lines().collect();
    let got: Vec<&str> = text.lines().collect();
    let mut bad = Vec::new();
    for i in 0..want.len().max(got.len()) {
        let (w, g) = (want.get(i).copied(), got.get(i).copied());
        if w != g {
            bad.push(format!("line {}: expected {w:?}, got {g:?}", i + 1));
        }
    }
    assert!(
        bad.is_empty(),
        "{} of {} lines differ:\n{}",
        bad.len(),
        got.len(),
        bad.join("\n")
    );
}

#[test]
fn the_golden_covers_every_kind_and_a_silent_identity() {
    let cases = cases();
    for kind in [Kind::Peak, Kind::LowShelf, Kind::HighShelf] {
        assert!(cases.iter().filter(|c| c.kind == kind).count() >= 30);
    }
    let input = input();
    let identity = cases
        .iter()
        .find(|c| c.gain_cdb == 0)
        .expect("an identity case");
    let mut buf = input.clone();
    Biquad::new(identity.coeffs()).process(&mut buf);
    assert!(
        buf.iter()
            .zip(&input)
            .all(|(a, b)| a.to_bits() == b.to_bits())
    );
    // A real band changes the noise and ends the tail in exact zeros.
    let band = cases.iter().find(|c| c.gain_cdb == 1_200).expect("a boost");
    let mut buf = input.clone();
    Biquad::new(band.coeffs()).process(&mut buf);
    assert!(buf[..NOISE].iter().zip(&input).any(|(a, b)| a != b));
    assert!(buf.iter().all(|v| v.is_finite()));
}

#[test]
fn process_allocates_nothing() {
    for kind in [Kind::Peak, Kind::LowShelf, Kind::HighShelf] {
        let coeffs = Coeffs::design(kind, 800.0, 9.0, 1.2, 48_000.0);
        let mut buf = input();
        let mut biquad = Biquad::new(coeffs);
        no_alloc(|| {
            biquad.process(&mut buf);
            biquad.reset();
            for v in buf.iter_mut().take(256) {
                *v = biquad.tick(*v);
            }
        });
    }
}

/// Rewrite the fixture. Ignored: run it by name, review the diff, and name the reason in the commit.
#[test]
#[ignore = "rewrites tests/fixtures/biquad-golden.txt"]
fn regenerate() {
    let (text, sum) = render();
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/biquad-golden.txt"
    );
    std::fs::write(path, text).expect("write the fixture");
    println!("wrote {path}: sum {sum}");
}
